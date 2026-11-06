//! VM-local one-shot timers. Registry keys avoid Rust/Lua ownership cycles;
//! callbacks execute inside the caller's existing per-dispatch CPU budget.
use mlua::{Function, Lua, RegistryKey, Table};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

struct Timer {
    due: f64,
    callback: RegistryKey,
}
#[derive(Default)]
struct State {
    clock: f64,
    next: u32,
    timers: BTreeMap<u32, Timer>,
}
pub struct TimerQueue(Arc<Mutex<State>>);
impl TimerQueue {
    pub fn pending(&self) -> bool {
        !self.0.lock().unwrap().timers.is_empty()
    }
    pub fn install(lua: &Lua, api: &Table) -> mlua::Result<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let pending = state.clone();
        api.set(
            "after",
            lua.create_function(move |lua, (delay, callback): (f64, Function)| {
                if !delay.is_finite() || !(0.0..=600.0).contains(&delay) {
                    return Err(mlua::Error::RuntimeError(
                        "Timer delay must be 0..600 seconds".into(),
                    ));
                }
                let mut state = pending.lock().unwrap();
                if state.timers.len() >= 64 {
                    return Err(mlua::Error::RuntimeError(
                        "At most 64 pending timers per script".into(),
                    ));
                }
                let id = state.next.checked_add(1).ok_or_else(|| {
                    mlua::Error::RuntimeError("Timer identity budget exhausted".into())
                })?;
                let timer = Timer {
                    due: state.clock + delay,
                    callback: lua.create_registry_value(callback)?,
                };
                state.next = id;
                state.timers.insert(id, timer);
                Ok(id)
            })?,
        )?;
        let pending = state.clone();
        api.set(
            "cancel_timer",
            lua.create_function(move |lua, id: u32| {
                let timer = pending.lock().unwrap().timers.remove(&id);
                if let Some(timer) = timer {
                    lua.remove_registry_value(timer.callback)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            })?,
        )?;
        Ok(Self(state))
    }
    pub fn advance(&self, lua: &Lua, dt: f64) -> mlua::Result<Vec<Function>> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(mlua::Error::RuntimeError("Invalid timer step".into()));
        }
        let timers = {
            let mut state = self.0.lock().unwrap();
            // No unbounded catch-up after OS sleep/focus loss. A zero-delay
            // callback rescheduling itself can only run on the next tick.
            state.clock += dt.min(1.0);
            let mut due: Vec<_> = state
                .timers
                .iter()
                .filter(|(_, timer)| timer.due <= state.clock)
                .map(|(id, timer)| (*id, timer.due))
                .collect();
            due.sort_by(|(a, at), (b, bt)| at.total_cmp(bt).then(a.cmp(b)));
            due.into_iter()
                .map(|(id, _)| state.timers.remove(&id).unwrap())
                .collect::<Vec<_>>()
        };
        let mut callbacks = Vec::with_capacity(timers.len());
        for timer in timers {
            callbacks.push(lua.registry_value(&timer.callback)?);
            lua.remove_registry_value(timer.callback)?;
        }
        Ok(callbacks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rescheduling_yields_until_next_tick_and_sleep_does_not_burst() {
        let lua = Lua::new();
        let api = lua.create_table().unwrap();
        let queue = TimerQueue::install(&lua, &api).unwrap();
        lua.globals().set("world", api).unwrap();
        lua.load("count = 0; function again() count += 1; last = world.after(0, again) end; last = world.after(0, again)").exec().unwrap();
        for expected in 1..=3 {
            let due = queue.advance(&lua, 0.0).unwrap();
            assert_eq!(due.len(), 1);
            for callback in due {
                callback.call::<()>(()).unwrap();
            }
            assert_eq!(lua.globals().get::<u32>("count").unwrap(), expected);
        }
        lua.load("assert(world.cancel_timer(last)); world.after(2, function() count = 99 end)")
            .exec()
            .unwrap();
        assert!(queue.advance(&lua, 1000.0).unwrap().is_empty());
        for callback in queue.advance(&lua, 1000.0).unwrap() {
            callback.call::<()>(()).unwrap();
        }
        assert_eq!(lua.globals().get::<u32>("count").unwrap(), 99);
    }
    #[test]
    fn ordering_cancellation_limits_and_no_ownership_cycle() {
        let lua = Lua::new();
        let api = lua.create_table().unwrap();
        let queue = TimerQueue::install(&lua, &api).unwrap();
        lua.globals().set("world", api.clone()).unwrap();
        lua.load("order = ''; world.after(0.2, function() order ..= 'B' end); world.after(0.1, function() order ..= 'A' end); local id = world.after(0, function() error('cancelled') end); assert(world.cancel_timer(id)); assert(not world.cancel_timer(id))").exec().unwrap();
        for callback in queue.advance(&lua, 0.25).unwrap() {
            callback.call::<()>(()).unwrap();
        }
        assert_eq!(lua.globals().get::<String>("order").unwrap(), "AB");
        assert!(queue.advance(&lua, f64::NAN).is_err());
        assert!(queue.advance(&lua, -1.0).is_err());
        assert!(lua.load("world.after(-1, function() end)").exec().is_err());
        assert!(lua.load("world.after(601, function() end)").exec().is_err());
        lua.load("for i=1,64 do world.after(600, function() end) end")
            .exec()
            .unwrap();
        assert!(lua.load("world.after(0, function() end)").exec().is_err());
        let weak = Arc::downgrade(&queue.0);
        drop(queue);
        drop(api);
        drop(lua);
        assert!(
            weak.upgrade().is_none(),
            "Timer callbacks kept the VM alive"
        );
    }
}
