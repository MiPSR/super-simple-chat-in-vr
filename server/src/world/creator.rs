use mlua::{Lua, LuaSerdeExt, Table};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spawn {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub owner: Option<String>,
}
impl Default for Spawn {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            owner: None,
        }
    }
}
pub enum Command {
    Call(String, crate::realtime::CallAction),
    Radio(String, crate::realtime::RadioRequest),
    Sound(String, &'static str, f64),
    Media(super::media::Request),
    Spawn(String, String, Spawn),
    Destroy(String),
    Visible(String, bool),
    Active(String, bool),
    Move(String, [f32; 3]),
    Rotate(String, [f32; 4]),
    Attach(String, String, [f32; 3]),
    Detach(String),
    Velocity(String, [f32; 3]),
    Damage(String, f32, Option<String>),
    Score(String, i32),
    Lease(String, String),
}
impl Command {
    fn checked_bytes(&self) -> mlua::Result<usize> {
        let valid = |id: &str| !id.is_empty() && id.len() <= 64;
        let optional = |id: &Option<String>| id.as_deref().is_none_or(valid);
        let (valid_ids, bytes) = match self {
            Self::Call(actor, action) => match action {
                crate::realtime::CallAction::Invite { user } => {
                    (valid(actor) && valid(user), actor.len() + user.len())
                }
                crate::realtime::CallAction::Leave => (valid(actor), actor.len()),
                crate::realtime::CallAction::Accept { .. } => {
                    return Err(mlua::Error::RuntimeError(
                        "Creators cannot accept calls".into(),
                    ));
                }
            },
            Self::Radio(actor, request) => (valid(actor), actor.len() + request.payload_bytes()),
            Self::Sound(id, _, _)
            | Self::Destroy(id)
            | Self::Visible(id, _)
            | Self::Active(id, _)
            | Self::Move(id, _)
            | Self::Rotate(id, _)
            | Self::Detach(id)
            | Self::Velocity(id, _)
            | Self::Score(id, _) => (valid(id), id.len()),
            Self::Media(request) => {
                request.validate().map_err(mlua::Error::external)?;
                (
                    true,
                    request.target.len() + request.action.len() + request.source.len(),
                )
            }
            Self::Spawn(id, prefab, options) => (
                valid(id) && valid(prefab) && optional(&options.owner),
                id.len() + prefab.len() + options.owner.as_ref().map_or(0, String::len),
            ),
            Self::Damage(id, _, actor) => (
                valid(id) && optional(actor),
                id.len() + actor.as_ref().map_or(0, String::len),
            ),
            Self::Lease(id, actor) => (valid(id) && valid(actor), id.len() + actor.len()),
            Self::Attach(child, parent, _) => {
                (valid(child) && valid(parent), child.len() + parent.len())
            }
        };
        if !valid_ids {
            return Err(mlua::Error::RuntimeError(
                "Invalid creator command ID".into(),
            ));
        }
        // Charge fixed bookkeeping as well as every owned string's UTF-8 bytes.
        Ok(bytes + 128)
    }
}
#[derive(Default)]
pub struct Context {
    pub interaction_actor: Option<String>,
    pub objects: BTreeMap<String, Value>,
    pub commands: Vec<Command>,
    pub next_entity: u64,
    pub state_dirty: bool,
}
pub type Shared = Arc<Mutex<Context>>;
fn enqueue(context: &Shared, command: Command) -> mlua::Result<()> {
    let bytes = command.checked_bytes()?;
    let mut context = context.lock().unwrap();
    if context.commands.len() >= 64 {
        return Err(mlua::Error::RuntimeError(
            "Script command budget exceeded".into(),
        ));
    }
    let queued = context.commands.iter().try_fold(0usize, |total, command| {
        Ok::<_, mlua::Error>(total + command.checked_bytes()?)
    })?;
    if queued + bytes > 65536 {
        return Err(mlua::Error::RuntimeError(
            "Script command byte budget exceeded".into(),
        ));
    }
    context.commands.push(command);
    Ok(())
}
fn components<const N: usize>(value: mlua::Value) -> mlua::Result<[f32; N]> {
    let mlua::Value::Table(table) = value else {
        return Err(mlua::Error::RuntimeError(
            "Expected a flat numeric vector".into(),
        ));
    };
    if table.raw_len() != N {
        return Err(mlua::Error::RuntimeError(
            "Wrong vector component count".into(),
        ));
    }
    let mut result = [0.0; N];
    for (index, component) in result.iter_mut().enumerate() {
        let value = match table.raw_get(index + 1)? {
            mlua::Value::Number(value) => value,
            mlua::Value::Integer(value) => value as f64,
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "Expected finite numeric components".into(),
                ));
            }
        };
        if !value.is_finite() || value.abs() > 100000.0 {
            return Err(mlua::Error::RuntimeError(
                "Vector component out of bounds".into(),
            ));
        }
        *component = value as f32;
    }
    Ok(result)
}
fn vector(_lua: &Lua, value: mlua::Value) -> mlua::Result<[f32; 3]> {
    components(value)
}
pub fn install(
    lua: &Lua,
    world: &Table,
    context: Shared,
    capabilities: &[String],
) -> mlua::Result<()> {
    if capabilities.iter().any(|capability| capability == "voice") {
        let calls = context.clone();
        world.set(
            "call_invite",
            lua.create_function(move |_, user: String| {
                let actor = calls
                    .lock()
                    .unwrap()
                    .interaction_actor
                    .clone()
                    .ok_or_else(|| {
                        mlua::Error::RuntimeError(
                            "Call requests require an authenticated interaction".into(),
                        )
                    })?;
                enqueue(
                    &calls,
                    Command::Call(actor, crate::realtime::CallAction::Invite { user }),
                )
            })?,
        )?;
        let calls = context.clone();
        world.set(
            "call_leave",
            lua.create_function(move |_, (): ()| {
                let actor = calls
                    .lock()
                    .unwrap()
                    .interaction_actor
                    .clone()
                    .ok_or_else(|| {
                        mlua::Error::RuntimeError(
                            "Call requests require an authenticated interaction".into(),
                        )
                    })?;
                enqueue(
                    &calls,
                    Command::Call(actor, crate::realtime::CallAction::Leave),
                )
            })?,
        )?;
    }
    lua.set_named_registry_value("events", lua.create_table()?)?;
    world.set(
        "on",
        lua.create_function(|lua, (event, callback): (String, mlua::Function)| {
            if !matches!(
                event.as_str(),
                "join"
                    | "leave"
                    | "tick"
                    | "collision"
                    | "touch"
                    | "grab"
                    | "lease"
                    | "release"
                    | "damage"
                    | "destroy"
                    | "attach"
                    | "detach"
            ) {
                return Err(mlua::Error::RuntimeError("Unsupported world event".into()));
            }
            let events: Table = lua.named_registry_value("events")?;
            events.set(event, callback)
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "object",
        lua.create_function(move |lua, id: String| {
            if id.is_empty() || id.len() > 64 {
                return Err(mlua::Error::RuntimeError("Invalid object ID".into()));
            }
            lua.to_value_with(
                &scene
                    .lock()
                    .unwrap()
                    .objects
                    .get(&id)
                    .cloned()
                    .unwrap_or(Value::Null),
                mlua::SerializeOptions::new()
                    .serialize_unit_to_null(false)
                    .serialize_none_to_null(false),
            )
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "find_tag",
        lua.create_function(move |lua, (tag, after): (String, Option<String>)| {
            let context = scene.lock().unwrap();
            let (ids, next) = crate::creator_objects::find_tag(&context.objects, &tag, after)
                .map_err(|error| mlua::Error::RuntimeError(error.into()))?;
            Ok((lua.to_value(&ids)?, next))
        })?,
    )?;
    if capabilities.iter().any(|c| c == "media") {
        let media = context.clone();
        world.set(
            "media",
            lua.create_function(move |lua, value: mlua::Value| {
                let request: super::media::Request = lua.from_value(value)?;
                request.validate().map_err(mlua::Error::external)?;
                enqueue(&media, Command::Media(request))
            })?,
        )?;
    }
    if capabilities.iter().any(|c| c == "audio") {
        for action in ["play_sound", "stop_sound"] {
            let context = context.clone();
            world.set(
                action,
                lua.create_function(move |_, id: String| {
                    if id.is_empty() || id.len() > 64 {
                        return Err(mlua::Error::RuntimeError("Invalid sound ID".into()));
                    }
                    enqueue(&context, Command::Sound(id, action, 0.0))
                })?,
            )?;
        }
        let context = context.clone();
        world.set(
            "seek_sound",
            lua.create_function(move |_, (id, seconds): (String, f64)| {
                if id.is_empty()
                    || id.len() > 64
                    || !seconds.is_finite()
                    || !(0.0..=600.0).contains(&seconds)
                {
                    return Err(mlua::Error::RuntimeError("Invalid sound seek".into()));
                }
                enqueue(&context, Command::Sound(id, "seek_sound", seconds))
            })?,
        )?;
    }
    if capabilities.iter().any(|c| c == "radio") {
        for (name, mode) in [("radio_message", "ft8"), ("radio_image", "sstv")] {
            let context = context.clone();
            world.set(
                name,
                lua.create_function(move |_, (target, data): (String, String)| {
                    let request = crate::realtime::RadioRequest::digital(target, mode, data)
                        .map_err(mlua::Error::external)?;
                    let actor = context
                        .lock()
                        .unwrap()
                        .interaction_actor
                        .clone()
                        .ok_or_else(|| {
                            mlua::Error::RuntimeError(
                                "Radio requires an authenticated interaction callback".into(),
                            )
                        })?;
                    enqueue(&context, Command::Radio(actor, request))
                })?,
            )?;
        }
    }
    if !capabilities.iter().any(|c| c == "objects") {
        return Ok(());
    }
    let scene = context.clone();
    world.set(
        "attach",
        lua.create_function(
            move |_, (child, parent, offset): (String, String, mlua::Value)| {
                let offset = components::<3>(offset)?;
                if offset.iter().any(|v| v.abs() > 100.0) {
                    return Err(mlua::Error::RuntimeError(
                        "Attachment offset exceeds 100 metres".into(),
                    ));
                }
                enqueue(&scene, Command::Attach(child, parent, offset))
            },
        )?,
    )?;
    let scene = context.clone();
    world.set(
        "detach",
        lua.create_function(move |_, child: String| enqueue(&scene, Command::Detach(child)))?,
    )?;
    let scene = context.clone();
    world.set(
        "rotate",
        lua.create_function(move |_, (id, value): (String, mlua::Value)| {
            let mut rotation = components::<4>(value)?;
            let length = rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !(0.0001..=100.0).contains(&length) {
                return Err(mlua::Error::RuntimeError(
                    "Invalid entity quaternion".into(),
                ));
            }
            for component in &mut rotation {
                *component /= length;
            }
            enqueue(&scene, Command::Rotate(id, rotation))
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "spawn",
        lua.create_function(move |lua, (prefab, options): (String, mlua::Value)| {
            let options: Spawn = lua.from_value(options)?;
            let id = {
                let mut context = scene.lock().unwrap();
                context.next_entity += 1;
                format!("e-{}", context.next_entity)
            };
            enqueue(&scene, Command::Spawn(id.clone(), prefab, options))?;
            Ok(id)
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "destroy",
        lua.create_function(move |_, id: String| enqueue(&scene, Command::Destroy(id)))?,
    )?;
    let scene = context.clone();
    world.set(
        "set_visible",
        lua.create_function(move |_, (id, value): (String, bool)| {
            enqueue(&scene, Command::Visible(id, value))
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "set_active",
        lua.create_function(move |_, (id, value): (String, bool)| {
            enqueue(&scene, Command::Active(id, value))
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "move",
        lua.create_function(move |lua, (id, value): (String, mlua::Value)| {
            enqueue(&scene, Command::Move(id, vector(lua, value)?))
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "velocity",
        lua.create_function(move |lua, (id, value): (String, mlua::Value)| {
            enqueue(&scene, Command::Velocity(id, vector(lua, value)?))
        })?,
    )?;
    let scene = context.clone();
    world.set(
        "damage",
        lua.create_function(
            move |_, (id, amount, actor): (String, f32, Option<String>)| {
                if !amount.is_finite() || !(0.0..=100000.0).contains(&amount) {
                    return Err(mlua::Error::RuntimeError("Invalid damage amount".into()));
                }
                enqueue(&scene, Command::Damage(id, amount, actor))
            },
        )?,
    )?;
    let scene = context.clone();
    world.set(
        "add_score",
        lua.create_function(move |_, (id, amount): (String, i32)| {
            enqueue(&scene, Command::Score(id, amount))
        })?,
    )?;
    world.set(
        "lease",
        lua.create_function(move |_, (id, user): (String, String)| {
            enqueue(&context, Command::Lease(id, user))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_command_queue_bounds_ids_count_and_aggregate_owned_bytes() {
        let context: Shared = Default::default();
        assert!(enqueue(&context, Command::Destroy("x".repeat(65))).is_err());
        assert!(enqueue(&context, Command::Lease("prop".into(), String::new())).is_err());
        assert!(context.lock().unwrap().commands.is_empty());
        for _ in 0..64 {
            enqueue(&context, Command::Visible("prop".into(), false)).unwrap();
        }
        assert!(enqueue(&context, Command::Destroy("prop".into())).is_err());
        context.lock().unwrap().commands.clear();
        let mut accepted = 0;
        loop {
            let command = Command::Media(super::super::media::Request {
                target: "screen".into(),
                action: "load".into(),
                source: format!("https://example.com/{}", "x".repeat(4000)),
                value: 0.0,
            });
            if enqueue(&context, command).is_err() {
                break;
            }
            accepted += 1;
            assert!(accepted < 64);
        }
        let state = context.lock().unwrap();
        assert!(accepted > 0 && accepted < 20);
        assert!(
            state
                .commands
                .iter()
                .map(|command| command.checked_bytes().unwrap())
                .sum::<usize>()
                <= 65536
        );
    }
}
