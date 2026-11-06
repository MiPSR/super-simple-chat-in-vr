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
    Radio(String, crate::realtime::RadioRequest),
    Sound(String, &'static str, f64),
    Media(super::media::Request),
    Spawn(String, String, Spawn),
    Destroy(String),
    Visible(String, bool),
    Active(String, bool),
    Move(String, [f32; 3]),
    Velocity(String, [f32; 3]),
    Damage(String, f32, Option<String>),
    Score(String, i32),
    Lease(String, String),
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
    let mut context = context.lock().unwrap();
    if context.commands.len() >= 64 {
        return Err(mlua::Error::RuntimeError(
            "Script command budget exceeded".into(),
        ));
    }
    context.commands.push(command);
    Ok(())
}
fn vector(lua: &Lua, value: mlua::Value) -> mlua::Result<[f32; 3]> {
    let values: Vec<f32> = lua.from_value(value)?;
    if values.len() != 3 || !values.iter().all(|v| v.is_finite() && v.abs() <= 100000.0) {
        return Err(mlua::Error::RuntimeError(
            "Expected a finite three-component vector".into(),
        ));
    }
    Ok([values[0], values[1], values[2]])
}
pub fn install(
    lua: &Lua,
    world: &Table,
    context: Shared,
    capabilities: &[String],
) -> mlua::Result<()> {
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
                    | "release"
                    | "damage"
                    | "destroy"
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
            lua.to_value(
                &scene
                    .lock()
                    .unwrap()
                    .objects
                    .get(&id)
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })?,
    )?;
    if capabilities.iter().any(|c| c == "media") {
        let media = context.clone();
        world.set(
            "media",
            lua.create_function(move |lua, value: mlua::Value| {
                let request: super::media::Request = lua.from_value(value)?;
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
