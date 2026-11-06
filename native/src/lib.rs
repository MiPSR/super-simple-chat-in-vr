//! Only missing engine facilities belong here: bounded Luau and libopus.
//! No Godot object ever enters a creator VM; the bridge accepts/returns JSON.
// GodotClass's generated constructors repeat field names.
#![allow(clippy::redundant_field_names)]
use godot::{classes::RefCounted, prelude::*};
use mlua::{Lua, LuaSerdeExt};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[path = "../../shared/creator_timers.rs"]
mod creator_timers;
mod media_proxy;
mod media_security;
mod media_source;
mod packaged_audio;
#[path = "../../shared/script_budget.rs"]
mod script_budget;
mod video;

struct Native;
static CLIENT_VMS: AtomicUsize = AtomicUsize::new(0);
struct VmSlot;
impl VmSlot {
    fn reserve() -> mlua::Result<Self> {
        if CLIENT_VMS.fetch_add(1, Ordering::AcqRel) >= 64 {
            CLIENT_VMS.fetch_sub(1, Ordering::AcqRel);
            return Err(mlua::Error::RuntimeError(
                "At most 64 active creator VMs per client".into(),
            ));
        }
        Ok(Self)
    }
}
impl Drop for VmSlot {
    fn drop(&mut self) {
        CLIENT_VMS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[gdextension]
unsafe impl ExtensionLibrary for Native {
    fn on_stage_deinit(stage: godot::init::InitStage) {
        if stage == godot::init::InitStage::Scene {
            video::shutdown();
        }
    }
}

struct Vm {
    lua: Lua,
    deadline: Arc<Mutex<script_budget::ScriptBudget>>,
    commands: Arc<Mutex<Vec<Value>>>,
    timers: creator_timers::TimerQueue,
    _slot: VmSlot,
}
impl Vm {
    fn new(source: &str, capabilities: &[String]) -> mlua::Result<Self> {
        let slot = VmSlot::reserve()?;
        let lua = Lua::new();
        lua.set_memory_limit(8 * 1024 * 1024)?;
        for name in ["os", "io", "require", "loadstring", "collectgarbage"] {
            lua.globals().set(name, mlua::Value::Nil)?;
        }
        let deadline = Arc::new(Mutex::new(script_budget::ScriptBudget::new(3)));
        let budget = deadline.clone();
        lua.set_interrupt(move |_| {
            if budget.lock().unwrap().expired() {
                Err(mlua::Error::RuntimeError(
                    "Client script time budget exceeded".into(),
                ))
            } else {
                Ok(mlua::VmState::Continue)
            }
        });
        let commands = Arc::new(Mutex::new(Vec::new()));
        let api = lua.create_table()?;
        let timers = creator_timers::TimerQueue::install(&lua, &api)?;
        let handlers = lua.create_table()?;
        lua.set_named_registry_value("handlers", handlers)?;
        api.set(
            "on",
            lua.create_function(|lua, (event, callback): (String, mlua::Function)| {
                if !matches!(
                    event.as_str(),
                    "tick" | "state" | "action" | "touch" | "grab" | "release"
                ) {
                    return Err(mlua::Error::RuntimeError("Unsupported visual event".into()));
                }
                let handlers: mlua::Table = lua.named_registry_value("handlers")?;
                handlers.set(event, callback)
            })?,
        )?;
        if capabilities.iter().any(|c| c == "visual") {
            let output = commands.clone();
            api.set(
                "set_visible",
                lua.create_function(move |_, (id, visible): (String, bool)| {
                    push(
                        &output,
                        json!({"action":"visible","target":id,"value":visible}),
                    )
                })?,
            )?;
            let output = commands.clone();
            api.set(
                "expression",
                lua.create_function(move |_, (id, weight): (String, f32)| {
                    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
                        return Err(mlua::Error::RuntimeError(
                            "Invalid expression weight".into(),
                        ));
                    }
                    push(
                        &output,
                        json!({"action":"expression","target":id,"value":weight}),
                    )
                })?,
            )?;
            let output = commands.clone();
            api.set(
                "animation",
                lua.create_function(move |_, name: String| {
                    push(&output, json!({"action":"animation","target":name}))
                })?,
            )?;
        }
        if capabilities.iter().any(|c| c == "audio") {
            for action in ["play_sound", "stop_sound"] {
                let output = commands.clone();
                api.set(
                    action,
                    lua.create_function(move |_, id: String| {
                        if id.is_empty() || id.len() > 64 {
                            return Err(mlua::Error::RuntimeError("Invalid sound ID".into()));
                        }
                        push(&output, json!({"action":action,"target":id}))
                    })?,
                )?;
            }
            let output = commands.clone();
            api.set(
                "seek_sound",
                lua.create_function(move |_, (id, seconds): (String, f64)| {
                    if id.is_empty()
                        || id.len() > 64
                        || !seconds.is_finite()
                        || !(0.0..=600.0).contains(&seconds)
                    {
                        return Err(mlua::Error::RuntimeError("Invalid sound seek".into()));
                    }
                    push(
                        &output,
                        json!({"action":"seek_sound","target":id,"value":seconds}),
                    )
                })?,
            )?;
        }
        if capabilities.iter().any(|c| c == "radio") {
            for (name, mode) in [("radio_message", "ft8"), ("radio_image", "sstv")] {
                let output = commands.clone();
                api.set(
                    name,
                    lua.create_function(move |_, (id, data): (String, String)| {
                        let valid = if mode == "ft8" {
                            !data.is_empty()
                                && data.len() <= 40
                                && data.bytes().all(|b| (32..=126).contains(&b))
                        } else {
                            data.len() == 6144 && data.bytes().all(|b| b.is_ascii_hexdigit())
                        };
                        if id.is_empty() || id.len() > 64 || !valid {
                            return Err(mlua::Error::RuntimeError(
                                "Invalid protected radio payload".into(),
                            ));
                        }
                        push(
                            &output,
                            json!({"action":"radio","target":id,"mode":mode,"data":data}),
                        )
                    })?,
                )?;
            }
        }
        lua.globals().set("world", api.clone())?;
        lua.globals().set("avatar", api)?;
        lua.sandbox(true)?;
        deadline.lock().unwrap().reset();
        lua.load(source).set_name("creator-client").exec()?;
        Ok(Self {
            lua,
            deadline,
            commands,
            timers,
            _slot: slot,
        })
    }
    fn dispatch(&self, event: &str, data: Value) -> mlua::Result<Vec<Value>> {
        self.deadline.lock().unwrap().reset();
        if event == "tick" && self.timers.pending() {
            for callback in self
                .timers
                .advance(&self.lua, data["dt"].as_f64().unwrap_or(0.0))?
            {
                callback.call::<()>(())?;
            }
        }
        let handlers: mlua::Table = self.lua.named_registry_value("handlers")?;
        if let Some(f) = handlers.get::<Option<mlua::Function>>(event)? {
            f.call::<()>(self.lua.to_value(&data)?)?;
        }
        Ok(std::mem::take(&mut self.commands.lock().unwrap()))
    }
}
fn push(commands: &Arc<Mutex<Vec<Value>>>, value: Value) -> mlua::Result<()> {
    if value["target"].as_str().is_none_or(|s| s.len() > 128) {
        return Err(mlua::Error::RuntimeError("Invalid visual target".into()));
    }
    let mut commands = commands.lock().unwrap();
    if commands.len() >= 64 {
        return Err(mlua::Error::RuntimeError(
            "Visual command budget exceeded".into(),
        ));
    }
    if commands
        .iter()
        .map(|command| command.to_string().len())
        .sum::<usize>()
        + value.to_string().len()
        > 65536
    {
        return Err(mlua::Error::RuntimeError(
            "Client command byte budget exceeded".into(),
        ));
    }
    commands.push(value);
    Ok(())
}

#[derive(GodotClass)]
#[class(base=RefCounted, init)]
struct CreatorVm {
    vm: Option<Vm>,
    base: Base<RefCounted>,
}
#[godot_api]
impl CreatorVm {
    #[func]
    fn load_script(&mut self, source: GString, capabilities: PackedStringArray) -> GString {
        self.vm = None;
        if source.len() > 65536 {
            return "Script exceeds 64 KiB".into();
        }
        match Vm::new(
            &source.to_string(),
            &capabilities
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        ) {
            Ok(vm) => {
                self.vm = Some(vm);
                GString::new()
            }
            Err(e) => e.to_string().as_str().into(),
        }
    }
    #[func]
    fn dispatch(&mut self, event: GString, data: GString) -> GString {
        if data.len() > 262144 {
            return r#"{"error":"Event payload budget exceeded"}"#.into();
        }
        let data = match serde_json::from_str(&data.to_string()) {
            Ok(data) => data,
            Err(_) => return r#"{"error":"Invalid event JSON"}"#.into(),
        };
        let Some(vm) = &self.vm else {
            return r#"{"commands":[]}"#.into();
        };
        match vm.dispatch(&event.to_string(), data) {
            Ok(commands) => json!({"commands":commands}).to_string().as_str().into(),
            Err(e) => {
                self.vm = None;
                json!({"error":e.to_string()}).to_string().as_str().into()
            }
        }
    }
    #[func]
    fn close(&mut self) {
        self.vm = None;
    }
}

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
struct OpusCodec {
    encoder: Option<opus::Encoder>,
    decoder: Option<opus::Decoder>,
    base: Base<RefCounted>,
}
#[godot_api]
impl OpusCodec {
    #[func]
    fn configure(&mut self, bitrate: i32) -> bool {
        if !(8000..=64000).contains(&bitrate) {
            return false;
        }
        self.encoder =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip).ok();
        self.decoder = opus::Decoder::new(48000, opus::Channels::Mono).ok();
        if let Some(encoder) = &mut self.encoder {
            let _ = encoder.set_bitrate(opus::Bitrate::Bits(bitrate));
            let _ = encoder.set_inband_fec(true);
        }
        self.encoder.is_some() && self.decoder.is_some()
    }
    #[func]
    fn encode(&mut self, pcm: PackedVector2Array) -> PackedByteArray {
        if pcm.len() != 960 {
            return PackedByteArray::new();
        }
        let Some(encoder) = &mut self.encoder else {
            return PackedByteArray::new();
        };
        let samples: Vec<f32> = pcm
            .as_slice()
            .iter()
            .map(|v| ((v.x + v.y) * 0.5).clamp(-1.0, 1.0))
            .collect();
        let mut packet = [0; 1275];
        match encoder.encode_float(&samples, &mut packet) {
            Ok(n) => PackedByteArray::from(&packet[..n]),
            Err(_) => PackedByteArray::new(),
        }
    }
    #[func]
    fn decode(&mut self, packet: PackedByteArray) -> PackedVector2Array {
        if packet.len() > 1275
            || (!packet.is_empty()
                && opus::packet::get_nb_samples(packet.as_slice(), 48000).ok() != Some(960))
        {
            return PackedVector2Array::new();
        }
        let Some(decoder) = &mut self.decoder else {
            return PackedVector2Array::new();
        };
        let mut samples = [0.0; 960];
        match decoder.decode_float(packet.as_slice(), &mut samples, false) {
            Ok(n) => PackedVector2Array::from(
                samples[..n]
                    .iter()
                    .map(|s| Vector2::new(*s, *s))
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            Err(_) => PackedVector2Array::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_visual_sandbox() {
        assert!(Vm::new("while true do end", &[]).is_err());
        assert!(
            Vm::new(
                "local a = {}; while true do table.insert(a, string.rep('a', 100000)) end",
                &[]
            )
            .is_err()
        );
        let vm=Vm::new("assert(os==nil and io==nil and require==nil); avatar.on('action', function(e) avatar.expression('happy', e.weight) end)",&["visual".into()]).unwrap();
        assert_eq!(
            vm.dispatch("action", json!({"weight":0.7})).unwrap()[0]["target"],
            "happy"
        );
        assert!(Vm::new("avatar.set_visible('other',false)", &[]).is_err());
    }
    #[test]
    fn client_timers_share_capabilities_and_cpu_watchdog() {
        let vm = Vm::new(
            "world.after(0.1, function() world.set_visible('lamp', true) end)",
            &["visual".into()],
        )
        .unwrap();
        assert!(vm.dispatch("tick", json!({"dt":0.05})).unwrap().is_empty());
        assert_eq!(
            vm.dispatch("tick", json!({"dt":0.05})).unwrap()[0]["target"],
            "lamp"
        );
        assert!(vm.dispatch("tick", json!({"dt":0.1})).unwrap().is_empty());
        let vm = Vm::new("world.after(0, function() while true do end end)", &[]).unwrap();
        assert!(vm.dispatch("tick", json!({"dt":0.01})).is_err());
    }
    #[test]
    fn client_vm_slots_are_bounded_and_released() {
        let output = std::process::Command::new("timeout")
            .arg("15")
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::client_slot_ceiling_child",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    #[ignore = "isolated process: exhausts the process-wide VM budget"]
    fn client_slot_ceiling_child() {
        let vms: Vec<_> = (0..64)
            .map(|_| Vm::new("world.after(600, function() end)", &[]).unwrap())
            .collect();
        assert!(Vm::new("", &[]).is_err());
        drop(vms);
        assert_eq!(CLIENT_VMS.load(Ordering::Acquire), 0);
        assert!(Vm::new("", &[]).is_ok());
        assert_eq!(CLIENT_VMS.load(Ordering::Acquire), 0);
    }
    #[test]
    fn bounded_radio_creator_requests() {
        assert!(Vm::new("world.radio_message('desk', 'CQ VR')", &[]).is_err());
        let vm = Vm::new("assert(world.radio_voice == nil); world.on('action', function() world.radio_message('desk', 'CQ VR'); world.radio_image('desk', string.rep('00', 3072)) end)", &["radio".into()]).unwrap();
        let commands = vm.dispatch("action", json!({})).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            json!({"action":"radio","target":"desk","mode":"ft8","data":"CQ VR"})
        );
        assert_eq!(commands[1]["data"].as_str().unwrap().len(), 6144);
        assert!(
            Vm::new(
                "for i=1,11 do world.radio_image('desk', string.rep('00', 3072)) end",
                &["radio".into()]
            )
            .is_err()
        );
        for source in [
            "world.radio_message('desk', string.rep('A', 41))",
            "world.radio_message('desk', '猫')",
            "world.radio_image('desk', string.rep('zz', 3072))",
            "world.radio_image('desk', '00')",
        ] {
            assert!(Vm::new(source, &["radio".into()]).is_err());
        }
    }
    #[test]
    fn opus_roundtrip() {
        let mut enc =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        let mut dec = opus::Decoder::new(48000, opus::Channels::Mono).unwrap();
        let input: Vec<f32> = (0..960)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.1)
            .collect();
        let mut packet = [0; 1275];
        let n = enc.encode_float(&input, &mut packet).unwrap();
        let mut pcm = [0.0; 960];
        assert_eq!(
            dec.decode_float(&packet[..n], &mut pcm, false).unwrap(),
            960
        );
        assert!(pcm.iter().all(|v| v.is_finite()));
        assert!(pcm.iter().any(|v| v.abs() > 0.01));
    }
}
