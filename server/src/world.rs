use crate::{assets::Manifest, db};
use mlua::{Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};
mod creator;
#[path = "../../shared/creator_timers.rs"]
mod creator_timers;
pub mod media;
mod physics;
#[path = "../../shared/script_budget.rs"]
mod script_budget;
use creator::{Command, Context, Shared, Spawn};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Join {
    #[serde(default)]
    pub world: String,
    #[serde(default)]
    pub instance: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub invitation: String,
    #[serde(default = "one")]
    pub send_divisor: u32,
    #[serde(default = "one")]
    pub receive_divisor: u32,
}
fn one() -> u32 {
    1
}
#[derive(Clone, Serialize)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub pose: Vec<f32>,
    pub avatar: Option<String>,
    pub avatar_version: Option<String>,
    pub avatar_actions: BTreeMap<String, f32>,
    pub tracking: BTreeMap<String, TrackedJoint>,
    pub sleeping: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedJoint {
    pub pose: [f32; 7],
    pub source: String,
}
pub(crate) fn deserialize_hand<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() && value.fract() == 0.0 && (0.0..=1.0).contains(&value) {
        Ok(value as u8)
    } else {
        Err(serde::de::Error::custom("Hand must be 0 or 1"))
    }
}
pub fn valid_tracking(joints: &BTreeMap<String, TrackedJoint>) -> bool {
    joints.len() <= 6
        && joints.iter().all(|(name, j)| {
            matches!(
                name.as_str(),
                "Hips" | "Chest" | "LeftLowerLeg" | "RightLowerLeg" | "LeftFoot" | "RightFoot"
            ) && matches!(j.source.as_str(), "tracked" | "estimated")
                && j.pose.iter().all(|v| v.is_finite() && v.abs() < 100000.0)
                && (j.pose[3..].iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 0.05
        })
}
#[derive(Serialize, Clone)]
pub struct Entity {
    pub id: String,
    pub component: String,
    pub tags: Vec<String>,
    pub position: [f32; 3],
    pub visible: bool,
    pub active: bool,
    pub controller: Option<String>,
    pub rotation: [f32; 4],
    pub size: [f32; 3],
    pub prefab: Option<String>,
    pub health: f32,
    pub owner: Option<String>,
    #[serde(skip)]
    spec: Option<crate::assets::Physics>,
    #[serde(skip)]
    lease_until: u64,
    #[serde(skip)]
    held_hand: Option<usize>,
    #[serde(skip)]
    controls: [f32; 2],
    #[serde(skip)]
    last_control: u64,
    #[serde(skip)]
    last_fire: Option<u64>,
    #[serde(skip)]
    expires: Option<u64>,
}
impl Entity {
    fn control_event(&self, event: &str, reason: &str, clock: u64) -> Option<Value> {
        self.controller.as_ref().map(|actor| {
            let mut event = json!({"event":event,"kind":"object","actor":actor,
                "target":self.id,"mode":if self.held_hand.is_some() {"grab"} else {"lease"},
                "reason":reason,"clock":clock});
            if let Some(hand) = self.held_hand {
                event["hand"] = json!(hand);
            }
            event
        })
    }
}
pub struct Instance {
    pub media: BTreeMap<String, media::Session>,
    pub id: String,
    pub world: String,
    pub version: String,
    pub owner: String,
    pub private: bool,
    pub capacity: usize,
    pub players: BTreeMap<String, Player>,
    pub entities: BTreeMap<String, Entity>,
    tag_bytes: usize,
    pub variables: Arc<Mutex<BTreeMap<String, Value>>>,
    pub revision: u64,
    pub invitations: HashMap<String, (String, i64)>,
    pub manifest: Manifest,
    scripts: Vec<Sandbox>,
    scene: physics::Scene,
    context: Shared,
    pub scores: BTreeMap<String, i32>,
    pub events: Vec<Value>,
    creator_events: VecDeque<Value>,
    creator_event_overflow: bool,
    pub radio_requests: Vec<(String, crate::realtime::RadioRequest)>,
    pub clock: u64,
    tps: u32,
    contacts: std::collections::BTreeSet<(String, String)>,
    pub interactions: BTreeMap<String, Interaction>,
}
#[derive(Clone, Serialize)]
pub struct Interaction {
    pub actor: String,
    pub target: String,
    pub bone: String,
    pub segment: String,
    pub hand: u8,
}
pub struct Sandbox {
    lua: Lua,
    budget: Arc<Mutex<script_budget::ScriptBudget>>,
    enabled: bool,
    context: Shared,
    timers: creator_timers::TimerQueue,
}
struct InteractionActor(Shared);
impl Drop for InteractionActor {
    fn drop(&mut self) {
        self.0.lock().unwrap().interaction_actor = None;
    }
}
impl Sandbox {
    #[cfg(test)]
    pub fn new(
        source: &str,
        variables: Arc<Mutex<BTreeMap<String, Value>>>,
        capabilities: &[String],
    ) -> anyhow::Result<Self> {
        Self::with_context(
            source,
            variables,
            capabilities,
            Arc::new(Mutex::new(Context::default())),
            Arc::new(BTreeMap::from([(
                "on".into(),
                crate::assets::StateVariable::Boolean { default: false },
            )])),
        )
    }
    fn with_context(
        source: &str,
        variables: Arc<Mutex<BTreeMap<String, Value>>>,
        capabilities: &[String],
        context: Shared,
        declarations: Arc<BTreeMap<String, crate::assets::StateVariable>>,
    ) -> anyhow::Result<Self> {
        let lua = Lua::new();
        lua.set_memory_limit(8 * 1024 * 1024)?;
        for name in ["os", "io", "require", "loadstring", "collectgarbage"] {
            lua.globals().set(name, mlua::Value::Nil)?;
        }
        lua.sandbox(true)?;
        let budget = Arc::new(Mutex::new(script_budget::ScriptBudget::new(5)));
        let b = budget.clone();
        lua.set_interrupt(move |_| {
            if b.lock().unwrap().expired() {
                Err(mlua::Error::RuntimeError(
                    "Script time budget exceeded".into(),
                ))
            } else {
                Ok(mlua::VmState::Continue)
            }
        });
        let world = lua.create_table()?;
        let timers = creator_timers::TimerQueue::install(&lua, &world)?;
        let handlers = lua.create_table()?;
        lua.set_named_registry_value("handlers", handlers)?;
        world.set(
            "on_interact",
            lua.create_function(|lua, (id, callback): (String, mlua::Function)| {
                if id.len() > 64 {
                    return Err(mlua::Error::RuntimeError("ID too long".into()));
                }
                let t: mlua::Table = lua.named_registry_value("handlers")?;
                t.set(id, callback)
            })?,
        )?;
        if capabilities.iter().any(|s| s == "state") {
            let state = lua.create_table()?;
            let vars = variables.clone();
            let readable = declarations.clone();
            state.set(
                "get",
                lua.create_function(move |lua, key: String| {
                    if !readable.contains_key(&key) {
                        return Err(mlua::Error::RuntimeError(
                            "Undeclared replicated variable".into(),
                        ));
                    }
                    lua.to_value(
                        &vars
                            .lock()
                            .unwrap()
                            .get(&key)
                            .cloned()
                            .unwrap_or(Value::Null),
                    )
                })?,
            )?;
            let changes = context.clone();
            state.set(
                "set",
                lua.create_function(move |lua, (key, value): (String, mlua::Value)| {
                    let value: Value = lua.from_value(value)?;
                    let value = declarations
                        .get(&key)
                        .and_then(|declaration| declaration.checked_value(value))
                        .ok_or_else(|| {
                            mlua::Error::RuntimeError(
                                "Undeclared variable or incompatible replicated value".into(),
                            )
                        })?;
                    let mut vars = variables.lock().unwrap();
                    if key.len() > 64
                        || (!vars.contains_key(&key) && vars.len() >= 128)
                        || value.to_string().len() > 8192
                        || vars
                            .iter()
                            .filter(|(name, _)| **name != key)
                            .map(|(name, existing)| name.len() + existing.to_string().len() + 4)
                            .sum::<usize>()
                            + key.len()
                            + value.to_string().len()
                            + 4
                            > crate::assets::REPLICATED_STATE_BYTES
                    {
                        return Err(mlua::Error::RuntimeError(
                            "Replicated state budget exceeded".into(),
                        ));
                    }
                    if vars.get(&key) != Some(&value) {
                        vars.insert(key, value);
                        changes.lock().unwrap().state_dirty = true;
                    }
                    Ok(())
                })?,
            )?;
            world.set("state", state)?;
        }
        creator::install(&lua, &world, context.clone(), capabilities)?;
        lua.globals().set("world", world)?;
        // No IO, package loader, OS or engine handles are exposed.
        budget.lock().unwrap().reset();
        lua.load(source).set_name("creator").exec()?;
        Ok(Self {
            lua,
            budget,
            enabled: true,
            context,
            timers,
        })
    }
    fn interact(&self, id: &str, player: &str) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        self.budget.lock().unwrap().reset();
        self.context.lock().unwrap().interaction_actor = Some(player.into());
        let _actor = InteractionActor(self.context.clone());
        let handlers: mlua::Table = self.lua.named_registry_value("handlers")?;
        if let Some(f) = handlers.get::<Option<mlua::Function>>(id)? {
            let user = self.lua.create_table()?;
            user.set("id", player)?;
            f.call::<()>(user)?;
        }
        Ok(())
    }
    fn has_event(&self, event: &str) -> bool {
        if event == "tick" && self.timers.pending() {
            return true;
        }
        self.lua
            .named_registry_value::<mlua::Table>("events")
            .ok()
            .and_then(|t| t.get::<Option<mlua::Function>>(event).ok().flatten())
            .is_some()
    }
    fn event(&self, event: &str, value: &Value) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        self.budget.lock().unwrap().reset();
        if event == "tick" {
            for callback in self
                .timers
                .advance(&self.lua, value["dt"].as_f64().unwrap_or(0.0))?
            {
                callback.call::<()>(())?;
            }
        }
        let events: mlua::Table = self.lua.named_registry_value("events")?;
        if let Some(f) = events.get::<Option<mlua::Function>>(event)? {
            f.call::<()>(self.lua.to_value(value)?)?;
        }
        Ok(())
    }
}
impl Instance {
    fn create(
        id: String,
        join: &Join,
        version: String,
        owner: String,
        manifest: Manifest,
        sources: Vec<String>,
        tps: u32,
    ) -> anyhow::Result<Self> {
        let mut instance = Self {
            id,
            world: join.world.clone(),
            version,
            owner,
            private: join.private,
            capacity: manifest.capacity,
            players: BTreeMap::new(),
            entities: BTreeMap::new(),
            tag_bytes: 0,
            variables: Arc::new(Mutex::new(
                manifest
                    .variables
                    .iter()
                    .map(|(key, declaration)| (key.clone(), declaration.default_value()))
                    .collect(),
            )),
            revision: 0,
            invitations: HashMap::new(),
            manifest,
            scripts: Vec::new(),
            scene: Default::default(),
            context: Arc::new(Mutex::new(Context::default())),
            scores: BTreeMap::new(),
            events: Vec::new(),
            creator_events: VecDeque::new(),
            creator_event_overflow: false,
            radio_requests: Vec::new(),
            clock: 0,
            tps,
            contacts: Default::default(),
            interactions: BTreeMap::new(),
            media: BTreeMap::new(),
        };
        for object in instance.manifest.objects.clone() {
            let mut tags = object.tags.clone();
            if let Some(prefab) = object
                .prefab
                .as_ref()
                .and_then(|id| instance.manifest.prefabs.get(id))
            {
                tags.extend(prefab.tags.iter().cloned());
            }
            tags.sort();
            tags.dedup();
            instance.tag_bytes += crate::assets::tag_bytes(&tags);
            anyhow::ensure!(
                instance.tag_bytes <= crate::assets::MAX_ENTITY_TAG_BYTES,
                "Live entity tags exceed 64 KiB"
            );
            let spec = object
                .physics
                .or_else(|| {
                    object
                        .prefab
                        .as_ref()
                        .and_then(|p| instance.manifest.prefabs.get(p))
                        .map(|p| p.physics.clone())
                })
                .or_else(|| {
                    if matches!(object.component.as_str(), "static" | "door")
                        && object.size.iter().all(|v| *v > 0.0)
                    {
                        Some(crate::assets::Physics {
                            dynamic: false,
                            size: object.size,
                            ..Default::default()
                        })
                    } else {
                        None
                    }
                });
            let size = spec.as_ref().map(|p| p.size).unwrap_or_else(|| {
                if object.size.iter().all(|s| *s > 0.0) {
                    object.size
                } else {
                    [0.25; 3]
                }
            });
            if let Some(p) = &spec {
                instance.scene.insert(
                    &object.id,
                    object.position,
                    p,
                    object.component == "vehicle",
                );
            }
            instance.entities.insert(
                object.id.clone(),
                Entity {
                    id: object.id,
                    component: object.component,
                    tags,
                    position: object.position,
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    size,
                    prefab: object.prefab,
                    health: spec.as_ref().map_or(0.0, |p| p.health),
                    visible: true,
                    active: false,
                    controller: None,
                    owner: None,
                    spec,
                    lease_until: 0,
                    held_hand: None,
                    controls: [0.0; 2],
                    last_control: 0,
                    last_fire: None,
                    expires: None,
                },
            );
        }
        instance.refresh_context();
        for (source, decl) in sources.iter().zip(
            instance
                .manifest
                .scripts
                .iter()
                .filter(|s| s.side == "server"),
        ) {
            match Sandbox::with_context(
                source,
                instance.variables.clone(),
                &decl.capabilities,
                instance.context.clone(),
                Arc::new(instance.manifest.variables.clone()),
            ) {
                Ok(script) => instance.scripts.push(script),
                Err(e) => eprintln!("Script initialization failed: {e}"),
            }
        }
        instance.apply_commands();
        Ok(instance)
    }
    fn refresh_context(&self) {
        self.context.lock().unwrap().objects = self
            .entities
            .iter()
            .map(|(id, e)| (id.clone(), serde_json::to_value(e).unwrap()))
            .collect();
    }
    pub(crate) fn dispatch(&mut self, event: &str, value: Value) {
        if !self.scripts.iter().any(|s| s.enabled && s.has_event(event)) {
            return;
        }
        self.refresh_context();
        let start = Instant::now();
        for script in &mut self.scripts {
            if start.elapsed().as_millis() >= 5 {
                break;
            }
            if let Err(e) = script.event(event, &value) {
                script.enabled = false;
                eprintln!("Script {} ({event}) disabled: {e}", self.id);
            }
        }
        self.apply_commands();
    }
    fn apply_commands(&mut self) {
        let commands = std::mem::take(&mut self.context.lock().unwrap().commands);
        for command in commands {
            let result: anyhow::Result<()> = (|| {
                match command {
                    Command::Radio(actor, request) => {
                        anyhow::ensure!(
                            self.radio_requests.len() < 64,
                            "Creator radio queue budget exceeded"
                        );
                        self.radio_requests.push((actor, request));
                    }
                    Command::Sound(id, action, seconds) => {
                        anyhow::ensure!(
                            self.manifest.sounds.iter().any(|sound| sound.id == id),
                            "Undeclared packaged sound"
                        );
                        self.events.push(json!({"event":"sound","action":action,"target":id,"value":seconds,"clock":self.clock}));
                    }
                    Command::Spawn(id, prefab, options) => {
                        self.spawn(id, &prefab, options)?;
                    }
                    Command::Media(request) => {
                        let owner = self.owner.clone();
                        let _ = self.media_command(&owner, request);
                    }
                    Command::Destroy(id) => {
                        self.destroy(&id);
                    }
                    Command::Visible(id, visible) => {
                        if let Some(e) = self.entities.get_mut(&id) {
                            e.visible = visible;
                            self.revision += 1;
                        }
                    }
                    Command::Active(id, active) => {
                        if let Some(e) = self.entities.get_mut(&id) {
                            e.active = active;
                            if e.component == "door"
                                && let Some(body) = self.scene.body_mut(&id)
                            {
                                body.set_enabled(!active);
                            }
                            self.revision += 1;
                        }
                    }
                    Command::Move(id, position) => {
                        if let Some(e) = self.entities.get_mut(&id) {
                            e.position = position;
                            if let Some(b) = self.scene.body_mut(&id) {
                                b.set_translation(position.into(), true);
                            }
                            self.revision += 1;
                        }
                    }
                    Command::Velocity(id, velocity) => {
                        anyhow::ensure!(
                            velocity.iter().all(|v| v.abs() <= 100.0),
                            "Velocity limit"
                        );
                        self.scene.velocity(&id, velocity);
                    }
                    Command::Damage(id, amount, actor) => {
                        self.damage(&id, amount, actor.as_deref());
                    }
                    Command::Score(user, amount) => {
                        if self.players.contains_key(&user) {
                            let score = self.scores.entry(user).or_default();
                            *score = score.saturating_add(amount);
                            self.revision += 1;
                        }
                    }
                    Command::Lease(id, user) => {
                        self.set_control(&id, &user, None, "script")?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("World {} command rejected: {e}", self.id);
            }
        }
        let mut context = self.context.lock().unwrap();
        if context.state_dirty {
            self.revision += 1;
            context.state_dirty = false;
        }
    }
    fn spawn(&mut self, id: String, prefab: &str, options: Spawn) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.entities.len() < 4096
                && self
                    .entities
                    .values()
                    .filter(|e| e.spec.as_ref().is_some_and(|s| s.dynamic))
                    .count()
                    < 512,
            "Entity/physics budget exhausted"
        );
        anyhow::ensure!(
            options
                .position
                .iter()
                .all(|x| x.is_finite() && x.abs() <= 100000.0)
                && options
                    .velocity
                    .iter()
                    .all(|x| x.is_finite() && x.abs() <= 100.0),
            "Invalid spawn transform/velocity"
        );
        anyhow::ensure!(!self.entities.contains_key(&id), "Entity ID already exists");
        anyhow::ensure!(
            options
                .owner
                .as_ref()
                .is_none_or(|u| self.players.contains_key(u)),
            "Unknown prefab owner"
        );
        let prefab_def = self
            .manifest
            .prefabs
            .get(prefab)
            .ok_or_else(|| anyhow::anyhow!("Prefab is not packaged"))?;
        let spec = prefab_def.physics.clone();
        let mut tags = prefab_def.tags.clone();
        tags.sort();
        let tag_bytes = crate::assets::tag_bytes(&tags);
        anyhow::ensure!(
            self.tag_bytes + tag_bytes <= crate::assets::MAX_ENTITY_TAG_BYTES,
            "Live entity tags exceed 64 KiB"
        );
        self.scene.insert(
            &id,
            options.position,
            &spec,
            prefab_def.component == "vehicle",
        );
        self.scene.velocity(&id, options.velocity);
        let expires = if spec.lifetime > 0.0 {
            Some(self.clock + (spec.lifetime * self.tps as f32).ceil() as u64)
        } else {
            None
        };
        self.entities.insert(
            id.clone(),
            Entity {
                id: id.clone(),
                component: prefab_def.component.clone(),
                tags,
                position: options.position,
                rotation: [0.0, 0.0, 0.0, 1.0],
                size: spec.size,
                prefab: Some(prefab.into()),
                health: spec.health,
                visible: true,
                active: false,
                controller: None,
                owner: options.owner,
                spec: Some(spec),
                lease_until: 0,
                held_hand: None,
                controls: [0.0; 2],
                last_control: 0,
                last_fire: None,
                expires,
            },
        );
        self.events
            .push(json!({"event":"spawn","entity":self.entities[&id],"clock":self.clock}));
        self.tag_bytes += tag_bytes;
        self.revision += 1;
        Ok(())
    }
    fn queue_creator_event(&mut self, event: Value) {
        let name = event["event"].as_str().unwrap_or("");
        if !self
            .scripts
            .iter()
            .any(|script| script.enabled && script.has_event(name))
        {
            return;
        }
        if self.creator_events.len() < 128 {
            self.creator_events.push_back(event);
        } else {
            self.creator_event_overflow = true;
        }
    }
    pub(crate) fn release_contacts(&mut self, ids: Vec<String>, reason: &str) {
        let mut changed = false;
        for id in ids {
            if let Some(grab) = self.interactions.remove(&id) {
                changed = true;
                self.queue_creator_event(
                    json!({"event":"release","action":"release","kind":"avatar",
                    "actor":grab.actor,"target":grab.target,"segment":grab.segment,
                    "bone":grab.bone,"hand":grab.hand,"reason":reason,"clock":self.clock}),
                );
            }
        }
        if changed {
            self.revision += 1;
        }
    }
    fn flush_creator_events(&mut self) {
        if std::mem::take(&mut self.creator_event_overflow) {
            eprintln!(
                "World {} creator event queue overflow; newest notifications dropped",
                self.id
            );
        }
        // Snapshot the batch length: handler-generated events cannot recursively
        // dispatch or consume another slot until the following simulation tick.
        let count = self.creator_events.len().min(8);
        let start = Instant::now();
        for index in 0..count {
            if index > 0 && start.elapsed().as_millis() >= 5 {
                break;
            }
            let event = self.creator_events.pop_front().unwrap();
            let name = event["event"].as_str().unwrap_or("").to_owned();
            self.dispatch(&name, event);
        }
    }
    fn destroy(&mut self, id: &str) {
        if let Some(entity) = self.entities.remove(id) {
            self.tag_bytes -= crate::assets::tag_bytes(&entity.tags);
            if let Some(event) = entity.control_event("release", "destroy", self.clock) {
                self.queue_creator_event(event);
            }
            self.scene.remove(id);
            self.revision += 1;
            let event = json!({"event":"destroy","target":id,"clock":self.clock});
            self.queue_creator_event(event.clone());
            self.events.push(event);
        }
    }
    fn damage(&mut self, id: &str, amount: f32, actor: Option<&str>) {
        let Some(entity) = self.entities.get_mut(id) else {
            return;
        };
        if entity.health <= 0.0 {
            return;
        }
        entity.health = (entity.health - amount).max(0.0);
        let dead = entity.health == 0.0;
        let event = json!({"event":"damage","target":id,"amount":amount,"health":entity.health,"actor":actor,"clock":self.clock});
        self.queue_creator_event(event.clone());
        self.events.push(event);
        self.revision += 1;
        if dead {
            if let Some(actor) = actor.filter(|u| self.players.contains_key(*u)) {
                let score = self.scores.entry(actor.into()).or_default();
                *score = score.saturating_add(1);
            }
            self.destroy(id);
        }
    }
    pub(crate) fn set_avatar(
        &mut self,
        user: &str,
        selected: Option<(String, String)>,
    ) -> anyhow::Result<()> {
        // The realtime handler performs the owner/enabled-version database check
        // before passing a selection here. Keep lifecycle cleanup in one place.
        let player = self
            .players
            .get_mut(user)
            .ok_or_else(|| anyhow::anyhow!("Not joined"))?;
        let (avatar, version) =
            selected.map_or((None, None), |(id, version)| (Some(id), Some(version)));
        let changed = player.avatar != avatar || player.avatar_version != version;
        player.avatar = avatar;
        player.avatar_version = version;
        player.avatar_actions.clear();
        self.revision += 1;
        if changed {
            let released = self
                .interactions
                .iter()
                .filter(|(_, grab)| grab.target == user)
                .map(|(id, _)| id.clone())
                .collect();
            self.release_contacts(released, "avatar_change");
        }
        Ok(())
    }
    fn reconnect(&mut self, user: &str, name: &str) {
        let controlled: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.controller.as_deref() == Some(user))
            .map(|(id, _)| id.clone())
            .collect();
        for id in controlled {
            self.release_control(&id, "reconnect");
        }
        let contacts = self
            .interactions
            .iter()
            .filter(|(_, grab)| grab.actor == user || grab.target == user)
            .map(|(id, _)| id.clone())
            .collect();
        self.release_contacts(contacts, "reconnect");
        if let Some(player) = self.players.get_mut(user)
            && player.name != name
        {
            player.name = name.into();
            self.revision += 1;
        }
    }
    fn release_control(&mut self, id: &str, reason: &str) {
        let Some(entity) = self.entities.get_mut(id) else {
            return;
        };
        let Some(event) = entity.control_event("release", reason, self.clock) else {
            return;
        };
        entity.controller = None;
        entity.held_hand = None;
        entity.controls = [0.0; 2];
        self.scene.release(id);
        self.revision += 1;
        self.queue_creator_event(event);
    }
    fn set_control(
        &mut self,
        id: &str,
        user: &str,
        hand: Option<usize>,
        reason: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.players.contains_key(user), "Unknown controller");
        let entity = self
            .entities
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown entity"))?;
        anyhow::ensure!(
            entity.spec.as_ref().is_some_and(|spec| spec.dynamic),
            "Entity cannot be controlled"
        );
        anyhow::ensure!(
            hand.is_none_or(|hand| hand < 2 && entity.component == "prop"),
            "Invalid prop/hand"
        );
        let renewed = entity.controller.as_deref() == Some(user) && entity.held_hand == hand;
        if !renewed {
            self.release_control(id, "handoff");
        }
        let entity = self.entities.get_mut(id).unwrap();
        entity.controller = Some(user.into());
        entity.held_hand = hand;
        entity.lease_until = self.clock + self.tps as u64 * 2;
        if !renewed {
            let event = entity
                .control_event(
                    if hand.is_some() { "grab" } else { "lease" },
                    reason,
                    self.clock,
                )
                .unwrap();
            self.revision += 1;
            self.queue_creator_event(event);
        }
        Ok(())
    }
    pub fn action(&mut self, user: &str, action: Action) -> anyhow::Result<()> {
        let (Action::Lease { target }
        | Action::Release { target }
        | Action::Grab { target, .. }
        | Action::Drive { target, .. }
        | Action::Fire { target }) = &action;
        let target = target.clone();
        let player = self
            .players
            .get(user)
            .ok_or_else(|| anyhow::anyhow!("Not joined"))?;
        let entity = self
            .entities
            .get_mut(&target)
            .ok_or_else(|| anyhow::anyhow!("Unknown entity"))?;
        anyhow::ensure!(
            entity.spec.as_ref().is_some_and(|p| p.dynamic),
            "Entity cannot be controlled"
        );
        match action {
            Action::Lease { .. } | Action::Grab { .. } => {
                anyhow::ensure!(
                    entity.controller.as_deref().is_none_or(|u| u == user),
                    "Entity already leased"
                );
                let distance: f32 = (0..3)
                    .map(|i| (player.pose[i] - entity.position[i]).powi(2))
                    .sum();
                anyhow::ensure!(distance <= 16.0, "Entity is out of reach");
                if let Action::Grab { hand, .. } = action {
                    anyhow::ensure!(hand < 2 && entity.component == "prop", "Invalid prop/hand");
                }
                let hand = if let Action::Grab { hand, .. } = action {
                    Some(hand as usize)
                } else {
                    None
                };
                self.set_control(&target, user, hand, "explicit")?;
            }
            Action::Release { .. } => {
                anyhow::ensure!(
                    entity.controller.as_deref() == Some(user),
                    "Not the controller"
                );
                self.release_control(&target, "explicit");
            }
            Action::Drive { throttle, turn, .. } => {
                anyhow::ensure!(
                    entity.controller.as_deref() == Some(user) && entity.component == "vehicle",
                    "No vehicle control lease"
                );
                anyhow::ensure!(
                    throttle.is_finite()
                        && turn.is_finite()
                        && throttle.abs() <= 1.0
                        && turn.abs() <= 1.0,
                    "Invalid vehicle input"
                );
                entity.controls = [throttle, turn];
                entity.last_control = self.clock;
                entity.lease_until = self.clock + self.tps as u64 * 2;
            }
            Action::Fire { .. } => {
                anyhow::ensure!(
                    entity.controller.as_deref() == Some(user),
                    "No weapon control lease"
                );
                let spec = entity.spec.as_ref().unwrap();
                anyhow::ensure!(
                    entity
                        .last_fire
                        .is_none_or(|last| self.clock.saturating_sub(last)
                            >= (spec.fire_interval * self.tps as f32).ceil() as u64),
                    "Weapon cooling down"
                );
                let prefab = spec
                    .projectile
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Entity has no weapon"))?;
                let definition = self
                    .manifest
                    .prefabs
                    .get(&prefab)
                    .ok_or_else(|| anyhow::anyhow!("Projectile not packaged"))?;
                let rotation = rapier3d::math::Rotation::from_xyzw(
                    entity.rotation[0],
                    entity.rotation[1],
                    entity.rotation[2],
                    entity.rotation[3],
                );
                let direction = rotation * rapier3d::math::Vector::new(0.0, 0.0, -1.0);
                let offset = (entity.size[2] + definition.physics.size[2]) / 2.0 + 0.1;
                let position = std::array::from_fn(|i| entity.position[i] + direction[i] * offset);
                let velocity = std::array::from_fn(|i| direction[i] * definition.physics.speed);
                entity.last_fire = Some(self.clock);
                entity.lease_until = self.clock + self.tps as u64 * 2;
                let id = {
                    let mut context = self.context.lock().unwrap();
                    context.next_entity += 1;
                    format!("e-{}", context.next_entity)
                };
                self.spawn(
                    id,
                    &prefab,
                    Spawn {
                        position,
                        velocity,
                        owner: Some(user.into()),
                    },
                )?;
            }
        }
        Ok(())
    }
    fn tick(&mut self) {
        self.clock += 1;
        let expired: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.controller.is_some() && entity.lease_until <= self.clock)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.release_control(&id, "lease_expired");
        }
        for (id, entity) in &mut self.entities {
            if let Some(hand) = entity.held_hand {
                if let Some(player) = entity.controller.as_ref().and_then(|u| self.players.get(u)) {
                    let i = 7 * (hand + 1);
                    self.scene
                        .hold(id, [player.pose[i], player.pose[i + 1], player.pose[i + 2]]);
                    entity.lease_until = self.clock + self.tps as u64 * 2;
                }
            } else if entity.component == "vehicle" {
                let inputs = if self.clock.saturating_sub(entity.last_control) > self.tps as u64 / 4
                {
                    [0.0; 2]
                } else {
                    entity.controls
                };
                if let Some(spec) = &entity.spec {
                    self.scene
                        .drive(id, inputs[0], inputs[1], spec.speed, spec.turn_speed);
                }
            }
        }
        let contacts = self.scene.step(1.0 / self.tps as f32);
        for (id, entity) in &mut self.entities {
            if let Some((position, rotation)) = self.scene.transform(id) {
                entity.position = position;
                entity.rotation = rotation;
            }
        }
        let mut hit = std::collections::BTreeSet::new();
        for (a, b) in &contacts {
            for (projectile, target) in [(a, b), (b, a)] {
                if hit.contains(projectile) {
                    continue;
                }
                if let Some(entity) = self
                    .entities
                    .get(projectile)
                    .filter(|e| e.component == "projectile")
                {
                    let amount = entity.spec.as_ref().map_or(0.0, |p| p.damage);
                    let owner = entity.owner.clone();
                    self.damage(target, amount, owner.as_deref());
                    hit.insert(projectile.clone());
                }
            }
        }
        let current: std::collections::BTreeSet<_> = contacts.into_iter().collect();
        let new_contacts: Vec<_> = current.difference(&self.contacts).cloned().collect();
        self.contacts = current;
        for id in hit {
            self.destroy(&id);
        }
        let expired: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, e)| {
                e.expires.is_some_and(|t| t <= self.clock)
                    || e.position
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > 100000.0)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.destroy(&id);
        }
        for (a, b) in new_contacts.into_iter().take(64) {
            self.dispatch("collision", json!({"a":a,"b":b,"clock":self.clock}));
        }
        self.flush_creator_events();
        self.dispatch("tick", json!({"dt":1.0/self.tps as f32,"clock":self.clock}));
    }
    pub fn snapshot(&self, tick: u64) -> Value {
        let media: BTreeMap<_, _> = self
            .media
            .iter()
            .map(|(id, session)| (id, session.snapshot()))
            .collect();
        json!({"instance":self.id,"world":self.world,"version":self.version,"revision":self.revision,"tick":tick,"players":self.players,"entities":self.entities,"variables":*self.variables.lock().unwrap(),"manifest":self.manifest,"scores":self.scores,"clock":self.clock,"interactions":self.interactions,"media":media})
    }
    pub fn media_command(&mut self, actor: &str, request: media::Request) -> anyhow::Result<()> {
        let surface = self
            .manifest
            .video_surfaces
            .iter()
            .find(|s| s.id == request.target)
            .ok_or_else(|| anyhow::anyhow!("Unknown media surface"))?;
        anyhow::ensure!(
            actor == self.owner || surface.public_controls,
            "Only the instance owner may control this player"
        );
        self.media
            .entry(request.target.clone())
            .or_default()
            .apply(&request)?;
        self.revision += 1;
        Ok(())
    }
    pub fn interact(&mut self, actor: &str, target: &str) -> anyhow::Result<()> {
        let player = self
            .players
            .get(actor)
            .ok_or_else(|| anyhow::anyhow!("Not joined"))?;
        let entity = self
            .entities
            .get_mut(target)
            .ok_or_else(|| anyhow::anyhow!("Unknown object"))?;
        if player.pose.len() < 3 {
            anyhow::bail!("No tracking sample");
        }
        let d: f32 = (0..3)
            .map(|i| (player.pose[i] - entity.position[i]).powi(2))
            .sum();
        anyhow::ensure!(d < 16.0, "Interaction out of reach");
        if matches!(entity.component.as_str(), "switch" | "door") {
            entity.active = !entity.active;
            if entity.component == "door"
                && let Some(body) = self.scene.body_mut(target)
            {
                body.set_enabled(!entity.active);
            }
        }
        self.refresh_context();
        for script in &mut self.scripts {
            if let Err(e) = script.interact(target, actor) {
                eprintln!("Script {}: {e}", self.id);
                script.enabled = false;
            }
        }
        self.apply_commands();
        self.revision += 1;
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Lease {
        target: String,
    },
    Release {
        target: String,
    },
    Grab {
        target: String,
        #[serde(deserialize_with = "deserialize_hand")]
        hand: u8,
    },
    Drive {
        target: String,
        throttle: f32,
        turn: f32,
    },
    Fire {
        target: String,
    },
}
pub struct Registry {
    pub instances: BTreeMap<String, Instance>,
    pub tick: u64,
    pub tps: u32,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            instances: BTreeMap::new(),
            tick: 0,
            tps: 60,
        }
    }
}
impl Registry {
    pub fn admit(
        &mut self,
        join: &Join,
        user: &str,
        name: &str,
        version: &str,
        manifest: Manifest,
        sources: Vec<String>,
    ) -> anyhow::Result<String> {
        anyhow::ensure!(
            [1, 2, 4].contains(&join.send_divisor) && [1, 2, 4].contains(&join.receive_divisor),
            "Invalid network divisors"
        );
        if !join.instance.is_empty() {
            let instance = self
                .instances
                .get(&join.instance)
                .ok_or_else(|| anyhow::anyhow!("Instance no longer exists"))?;
            if instance.players.contains_key(user) {
                let id = join.instance.clone();
                self.instances.get_mut(&id).unwrap().reconnect(user, name);
                return Ok(id);
            }
            if instance.private && instance.owner != user {
                anyhow::ensure!(
                    instance
                        .invitations
                        .get(&join.invitation)
                        .is_some_and(|(id, expires)| id == user && *expires > db::now()),
                    "Invitation required"
                );
            }
            anyhow::ensure!(instance.players.len() < instance.capacity, "Instance full");
        }
        let created = if join.instance.is_empty() {
            let vacated = self
                .instances
                .values()
                .filter(|instance| {
                    instance.players.len() == 1 && instance.players.contains_key(user)
                })
                .count();
            anyhow::ensure!(
                self.instances.len().saturating_sub(vacated) < 64,
                "Instance limit reached"
            );
            let id = loop {
                let candidate = db::token()[..16].to_string();
                if !self.instances.contains_key(&candidate) {
                    break candidate;
                }
            };
            let instance = Instance::create(
                id.clone(),
                join,
                version.into(),
                user.into(),
                manifest,
                sources,
                self.tps,
            )?;
            Some((id, instance))
        } else {
            None
        };
        // Admission checks and new-instance construction must succeed before
        // removing the user's existing membership or its ephemeral state.
        self.leave(user);
        let id = if let Some((id, instance)) = created {
            self.instances.insert(id.clone(), instance);
            id
        } else {
            join.instance.clone()
        };
        let instance = self
            .instances
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Instance emptied during transition"))?;
        instance.invitations.remove(&join.invitation);
        let p = instance.manifest.spawn;
        instance.players.insert(
            user.into(),
            Player {
                id: user.into(),
                name: name.into(),
                pose: vec![
                    p[0],
                    p[1] + 1.65,
                    p[2],
                    0.,
                    0.,
                    0.,
                    1.,
                    p[0] - 0.3,
                    p[1] + 1.,
                    p[2],
                    0.,
                    0.,
                    0.,
                    1.,
                    p[0] + 0.3,
                    p[1] + 1.,
                    p[2],
                    0.,
                    0.,
                    0.,
                    1.,
                ],
                avatar: None,
                avatar_version: None,
                avatar_actions: BTreeMap::new(),
                tracking: BTreeMap::new(),
                sleeping: false,
            },
        );
        instance.revision += 1;
        instance.dispatch("join", json!({"id":user}));
        Ok(id)
    }
    pub fn leave(&mut self, user: &str) {
        self.instances.retain(|_, i| {
            if i.players.remove(user).is_some() {
                i.revision += 1;
                let controlled: Vec<_> = i
                    .entities
                    .iter()
                    .filter(|(_, entity)| entity.controller.as_deref() == Some(user))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in controlled {
                    i.release_control(&id, "disconnect");
                }
                i.dispatch("leave", json!({"id":user}));
                let released = i
                    .interactions
                    .iter()
                    .filter(|(_, interaction)| {
                        interaction.actor == user || interaction.target == user
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                i.release_contacts(released, "disconnect");
            }
            !i.players.is_empty()
        });
    }
    pub fn step(&mut self) {
        self.tick += 1;
        for instance in self.instances.values_mut() {
            instance.tick();
        }
    }
    pub fn settings_changed(&mut self, user: &str) {
        for instance in self.instances.values_mut() {
            if instance.players.contains_key(user) {
                instance.revision += 1;
            }
        }
    }
    pub fn list(&self, user: &str) -> Value {
        json!(self.instances.values().filter(|i|!i.private || i.owner==user || i.players.contains_key(user)).map(|i|json!({"id":i.id,"world":i.world,"version":i.version,"players":i.players.len(),"capacity":i.capacity,"private":i.private})).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Manifest {
        serde_json::from_value(json!({"schema":1,"kind":"world","name":"Test","scene":"world.glb"}))
            .unwrap()
    }
    #[test]
    fn private_invitation_identity_expiry_consumption_and_visibility() {
        let mut registry = Registry::default();
        let base = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let mut definition = manifest();
        definition.capacity = 2;
        let private = registry
            .admit(
                &Join {
                    private: true,
                    ..base.clone()
                },
                "a",
                "A",
                "v1",
                definition.clone(),
                vec![],
            )
            .unwrap();
        let public = registry
            .admit(&base, "c", "C", "public", manifest(), vec![])
            .unwrap();
        assert!(
            !registry
                .list("b")
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == private)
        );
        assert!(
            registry
                .list("a")
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == private)
        );
        registry
            .instances
            .get_mut(&private)
            .unwrap()
            .invitations
            .insert("valid".into(), ("b".into(), db::now() + 300));
        registry
            .instances
            .get_mut(&private)
            .unwrap()
            .invitations
            .insert("expired".into(), ("b".into(), db::now() - 1));
        let request = Join {
            instance: private.clone(),
            invitation: "valid".into(),
            ..base.clone()
        };
        assert!(
            registry
                .admit(&request, "c", "C", "ignored", manifest(), vec![])
                .is_err()
        );
        assert!(registry.instances[&public].players.contains_key("c"));
        assert!(
            registry.instances[&private]
                .invitations
                .contains_key("valid")
        );
        assert!(
            registry
                .admit(
                    &Join {
                        invitation: "expired".into(),
                        ..request.clone()
                    },
                    "b",
                    "B",
                    "ignored",
                    manifest(),
                    vec![]
                )
                .is_err()
        );
        let mut changed = manifest();
        changed.name = "Changed publication".into();
        registry
            .admit(&request, "b", "B", "v2", changed, vec![])
            .unwrap();
        assert!(
            !registry.instances[&private]
                .invitations
                .contains_key("valid")
        );
        assert_eq!(registry.instances[&private].version, "v1");
        assert_eq!(registry.instances[&private].manifest.name, definition.name);
        assert!(
            registry
                .list("b")
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == private)
        );
        registry
            .instances
            .get_mut(&private)
            .unwrap()
            .invitations
            .insert("for-c".into(), ("c".into(), db::now() + 300));
        assert!(
            registry
                .admit(
                    &Join {
                        invitation: "for-c".into(),
                        ..request.clone()
                    },
                    "c",
                    "C",
                    "ignored",
                    manifest(),
                    vec![]
                )
                .is_err()
        );
        assert!(
            registry.instances[&private]
                .invitations
                .contains_key("for-c")
        );
        assert!(registry.instances[&public].players.contains_key("c"));
        registry.leave("a");
        registry
            .admit(
                &Join {
                    invitation: String::new(),
                    ..request.clone()
                },
                "a",
                "A",
                "ignored",
                manifest(),
                vec![],
            )
            .unwrap();
        registry
            .admit(
                &Join {
                    instance: public.clone(),
                    ..base
                },
                "b",
                "B",
                "ignored",
                manifest(),
                vec![],
            )
            .unwrap();
        assert!(
            registry
                .admit(&request, "b", "B", "ignored", manifest(), vec![])
                .is_err(),
            "Invitation reuse must fail after leaving"
        );
        assert!(registry.instances[&public].players.contains_key("b"));
        registry.leave("a");
        assert!(!registry.instances.contains_key(&private));
        assert!(
            registry
                .admit(
                    &Join {
                        invitation: "for-c".into(),
                        ..request
                    },
                    "c",
                    "C",
                    "ignored",
                    manifest(),
                    vec![]
                )
                .is_err()
        );
        assert!(registry.instances[&public].players.contains_key("c"));
    }
    #[test]
    fn failed_instance_transition_preserves_existing_membership_and_state() {
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let shared = registry
            .admit(&join, "a", "A", "v1", manifest(), vec![])
            .unwrap();
        registry
            .admit(
                &Join {
                    instance: shared.clone(),
                    ..join.clone()
                },
                "b",
                "B",
                "ignored",
                manifest(),
                vec![],
            )
            .unwrap();
        let mut solo = String::new();
        for index in 0..63 {
            let id = registry
                .admit(
                    &join,
                    &format!("solo{index}"),
                    "Solo",
                    "v1",
                    manifest(),
                    vec![],
                )
                .unwrap();
            if index == 0 {
                solo = id;
            }
        }
        let revision = registry.instances[&shared].revision;
        assert!(
            registry
                .admit(&join, "a", "A", "v2", manifest(), vec![])
                .is_err()
        );
        assert!(registry.instances[&shared].players.contains_key("a"));
        assert_eq!(registry.instances[&shared].revision, revision);
        let replacement = registry
            .admit(&join, "solo0", "Solo", "v2", manifest(), vec![])
            .unwrap();
        assert!(!registry.instances.contains_key(&solo));
        assert_eq!(registry.instances.len(), 64);
        assert_eq!(registry.instances[&replacement].version, "v2");
        registry.leave("solo1");
        let tags: Vec<_> = (0..16)
            .map(|i| format!("tag{i:02}{}", "x".repeat(27)))
            .collect();
        let invalid: Manifest = serde_json::from_value(json!({"schema":1,"kind":"world","name":"Oversized tags","scene":"world.glb",
            "objects":(0..200).map(|i| json!({"id":format!("object{i}"),"component":"static","tags":tags})).collect::<Vec<_>>() })).unwrap();
        assert!(
            registry
                .admit(&join, "a", "A", "invalid", invalid, vec![])
                .is_err()
        );
        assert!(registry.instances[&shared].players.contains_key("a"));
        assert_eq!(registry.instances[&shared].revision, revision);
    }
    #[test]
    fn live_reconnect_preserves_state_and_releases_old_constraints() {
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            private: true,
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let definition: Manifest = serde_json::from_value(json!({"schema":1,"kind":"world","name":"Reconnect","scene":"world.glb","capacity":2,
            "objects":[{"id":"prop","component":"prop","position":[-1,1,0],"physics":{"size":[0.2,0.2,0.2]}}]})).unwrap();
        let id = registry
            .admit(&join, "a", "A", "v1", definition.clone(), vec![])
            .unwrap();
        registry
            .instances
            .get_mut(&id)
            .unwrap()
            .invitations
            .insert("invite".into(), ("b".into(), db::now() + 300));
        registry
            .admit(
                &Join {
                    instance: id.clone(),
                    invitation: "invite".into(),
                    ..join.clone()
                },
                "b",
                "B",
                "ignored",
                definition,
                vec![],
            )
            .unwrap();
        let instance = registry.instances.get_mut(&id).unwrap();
        instance.scores.insert("a".into(), 37);
        instance
            .set_avatar("a", Some(("avatar".into(), "avatar-version".into())))
            .unwrap();
        instance
            .players
            .get_mut("a")
            .unwrap()
            .avatar_actions
            .insert("happy".into(), 1.0);
        instance
            .action(
                "a",
                Action::Grab {
                    target: "prop".into(),
                    hand: 0,
                },
            )
            .unwrap();
        instance.tick();
        instance.interactions.insert(
            "a:0".into(),
            Interaction {
                actor: "a".into(),
                target: "b".into(),
                bone: "Hair".into(),
                segment: "hair".into(),
                hand: 0,
            },
        );
        let pose = instance.players["a"].pose.clone();
        let resumed = registry
            .admit(
                &Join {
                    instance: id.clone(),
                    invitation: String::new(),
                    ..join.clone()
                },
                "a",
                "A renamed",
                "ignored",
                manifest(),
                vec![],
            )
            .unwrap();
        assert_eq!(resumed, id);
        let instance = registry.instances.get_mut(&id).unwrap();
        assert_eq!(instance.players.len(), 2);
        assert_eq!(instance.scores["a"], 37);
        assert_eq!(instance.players["a"].pose, pose);
        assert_eq!(
            instance.players["a"].avatar_version.as_deref(),
            Some("avatar-version")
        );
        assert_eq!(instance.players["a"].avatar_actions["happy"], 1.0);
        assert_eq!(instance.players["a"].name, "A renamed");
        assert!(instance.entities["prop"].controller.is_none() && instance.interactions.is_empty());
        assert_eq!(
            instance.scene.body_mut("prop").unwrap().body_type(),
            rapier3d::prelude::RigidBodyType::Dynamic
        );
        assert_eq!(
            registry
                .admit(
                    &Join {
                        instance: id.clone(),
                        invitation: String::new(),
                        ..join
                    },
                    "b",
                    "B",
                    "ignored",
                    manifest(),
                    vec![]
                )
                .unwrap(),
            id,
            "An existing invited member can reconnect to a full private instance without reusing its consumed invitation"
        );
    }
    #[test]
    fn tag_queries_are_paginated_stable_and_detached() {
        let mut context = Context::default();
        for index in 0..600 {
            let id = format!("obj{index:04}");
            context.objects.insert(
                id.clone(),
                json!({"id":id,"tags":["game.enemy"],"visible":true}),
            );
        }
        let source = r#"
            local page, cursor = 0, nil
            world.on('tick', function()
                page += 1
                if page <= 3 then
                    local ids, next = world.find_tag('game.enemy', cursor)
                    if page == 1 then assert(#ids == 256 and ids[1] == 'obj0000' and next == 'obj0255') end
                    if page == 2 then assert(#ids == 256 and ids[1] == 'obj0256' and next == 'obj0511') end
                    if page == 3 then assert(#ids == 88 and ids[88] == 'obj0599' and next == nil) end
                    cursor = next
                else
                    local missing, done = world.find_tag('missing')
                    assert(#missing == 0 and done == nil)
                    local object = world.object('obj0000'); object.visible = false; object.tags[1] = 'changed'
                    assert(world.object('obj0000').visible == true and world.object('obj0000').tags[1] == 'game.enemy')
                    assert(not pcall(function() world.find_tag('bad/tag') end))
                    assert(not pcall(function() world.find_tag('game.enemy', string.rep('x',65)) end))
                end
            end)
        "#;
        let script = Sandbox::with_context(
            source,
            Arc::new(Mutex::new(BTreeMap::new())),
            &[],
            Arc::new(Mutex::new(context)),
            Arc::new(BTreeMap::new()),
        )
        .unwrap();
        for _ in 0..4 {
            script.event("tick", &json!({"dt":1.0/60.0})).unwrap();
        }
    }
    #[test]
    fn entity_tags_inherit_from_packaged_prefabs_and_refresh_after_destroy() {
        let definition: Manifest = serde_json::from_value(json!({"schema":1,"kind":"world","name":"Tags","scene":"world.glb",
            "prefabs":{"crate":{"scene":"crate.glb","component":"prop","tags":["shared","base"],"physics":{"size":[1,1,1]}}},
            "objects":[{"id":"authored","component":"prop","prefab":"crate","tags":["shared","switch"]}]})).unwrap();
        let mut instance = Instance::create(
            "tags".into(),
            &Join::default(),
            "hash".into(),
            "owner".into(),
            definition,
            vec![],
            60,
        )
        .unwrap();
        assert_eq!(
            instance.entities["authored"].tags,
            ["base", "shared", "switch"]
        );
        instance
            .spawn("spawned".into(), "crate", Spawn::default())
            .unwrap();
        assert_eq!(instance.entities["spawned"].tags, ["base", "shared"]);
        instance.refresh_context();
        assert_eq!(
            instance.context.lock().unwrap().objects["spawned"]["tags"],
            json!(["base", "shared"])
        );
        instance.destroy("spawned");
        instance.refresh_context();
        assert!(
            !instance
                .context
                .lock()
                .unwrap()
                .objects
                .contains_key("spawned")
        );
    }
    #[test]
    fn spawning_cannot_expand_tags_past_the_live_budget() {
        let tags: Vec<_> = (0..16)
            .map(|i| format!("tag{i:02}{}", "x".repeat(27)))
            .collect();
        let definition: Manifest = serde_json::from_value(json!({"schema":1,"kind":"world","name":"Tags","scene":"world.glb",
            "prefabs":{"crate":{"scene":"crate.glb","component":"prop","tags":tags,"physics":{"dynamic":false,"size":[1,1,1]}}}})).unwrap();
        let cost = crate::assets::tag_bytes(&tags);
        let mut instance = Instance::create(
            "tags".into(),
            &Join::default(),
            "hash".into(),
            "owner".into(),
            definition,
            vec![],
            60,
        )
        .unwrap();
        let count = crate::assets::MAX_ENTITY_TAG_BYTES / cost;
        for i in 0..count {
            instance
                .spawn(format!("tag-{i}"), "crate", Spawn::default())
                .unwrap();
        }
        assert_eq!(instance.tag_bytes, count * cost);
        assert!(
            instance
                .spawn("overflow".into(), "crate", Spawn::default())
                .is_err()
        );
        assert!(!instance.entities.contains_key("overflow"));
        assert!(instance.scene.body_mut("overflow").is_none());
        instance.destroy("tag-0");
        assert_eq!(instance.tag_bytes, (count - 1) * cost);
        instance
            .spawn("recovered".into(), "crate", Spawn::default())
            .unwrap();
        assert_eq!(instance.tag_bytes, count * cost);
    }
    #[test]
    fn control_events_cover_renewal_handoff_expiry_destruction_and_disconnect() {
        let definition: Manifest = serde_json::from_value(json!({"schema":1,"kind":"world","name":"Control events","scene":"world.glb",
            "variables":{"history":{"type":"string","default":"","max_bytes":4096}},
            "objects":[{"id":"car","component":"vehicle","position":[1,0.5,-1],"physics":{"size":[1,1,1]}},
                {"id":"prop","component":"prop","position":[-1,1,0],"physics":{"size":[0.2,0.2,0.2]}},
                {"id":"fixed","component":"static","position":[0,-0.1,0],"size":[20,0.2,20]},
                {"id":"handoff","component":"switch","position":[0,1.2,-1]}],
            "scripts":[{"path":"events.luau","side":"server","attachment":"world","capabilities":["state","objects"]}]})).unwrap();
        let source = r#"
            for _, name in ipairs({'grab','lease','release'}) do
                world.on(name, function(e)
                    assert(e.kind == 'object' and e.event == name)
                    world.state.set('history', world.state.get('history') .. name .. ':' .. e.reason .. ':' .. e.actor .. ':' .. e.mode .. ';')
                end)
            end
            world.on_interact('handoff', function() world.lease('prop','b') end)
        "#;
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let id = registry
            .admit(
                &join,
                "a",
                "A",
                "hash",
                definition.clone(),
                vec![source.into()],
            )
            .unwrap();
        registry
            .admit(
                &Join {
                    instance: id.clone(),
                    ..join
                },
                "b",
                "B",
                "hash",
                definition,
                vec![],
            )
            .unwrap();
        let instance = registry.instances.get_mut(&id).unwrap();
        instance
            .action(
                "a",
                Action::Grab {
                    target: "prop".into(),
                    hand: 0,
                },
            )
            .unwrap();
        assert!(
            instance
                .action(
                    "b",
                    Action::Grab {
                        target: "prop".into(),
                        hand: 0
                    }
                )
                .is_err()
        );
        instance
            .action(
                "a",
                Action::Grab {
                    target: "prop".into(),
                    hand: 0,
                },
            )
            .unwrap();
        assert_eq!(
            instance.creator_events.len(),
            1,
            "Lease renewal must not duplicate acquisition events"
        );
        instance.tick();
        assert_eq!(
            instance.variables.lock().unwrap()["history"],
            "grab:explicit:a:grab;"
        );
        instance
            .action(
                "a",
                Action::Release {
                    target: "prop".into(),
                },
            )
            .unwrap();
        instance.tick();
        instance
            .action(
                "a",
                Action::Lease {
                    target: "car".into(),
                },
            )
            .unwrap();
        instance.tick();
        instance.entities.get_mut("car").unwrap().lease_until = instance.clock + 1;
        instance.tick();
        assert!(instance.entities["car"].controller.is_none());
        assert!(
            instance.variables.lock().unwrap()["history"]
                .as_str()
                .unwrap()
                .ends_with("release:lease_expired:a:lease;")
        );
        instance
            .action(
                "a",
                Action::Grab {
                    target: "prop".into(),
                    hand: 0,
                },
            )
            .unwrap();
        instance.interact("a", "handoff").unwrap();
        assert_eq!(instance.entities["prop"].controller.as_deref(), Some("b"));
        assert!(
            instance.entities["prop"].held_hand.is_none(),
            "Script handoff must release the previous hand constraint"
        );
        assert_eq!(
            instance.scene.body_mut("prop").unwrap().body_type(),
            rapier3d::prelude::RigidBodyType::Dynamic
        );
        instance.tick();
        assert!(
            instance.variables.lock().unwrap()["history"]
                .as_str()
                .unwrap()
                .ends_with("release:handoff:a:grab;lease:script:b:lease;")
        );
        assert!(instance.set_control("fixed", "a", None, "script").is_err());
        instance.destroy("prop");
        instance.tick();
        assert!(
            instance.variables.lock().unwrap()["history"]
                .as_str()
                .unwrap()
                .ends_with("release:destroy:b:lease;")
        );
        instance
            .action(
                "a",
                Action::Lease {
                    target: "car".into(),
                },
            )
            .unwrap();
        registry.leave("a");
        registry.step();
        let instance = &registry.instances[&id];
        assert!(instance.entities["car"].controller.is_none());
        assert!(
            instance.variables.lock().unwrap()["history"]
                .as_str()
                .unwrap()
                .ends_with("release:disconnect:a:lease;")
        );
        assert!(instance.context.lock().unwrap().interaction_actor.is_none());
    }
    #[test]
    fn damage_destroy_events_are_deferred_bounded_and_nonrecursive() {
        let definition: Manifest = serde_json::from_value(json!({
            "schema":1,"kind":"world","name":"Events","scene":"world.glb",
            "variables":{"hits":{"type":"integer","default":0,"min":0,"max":10000},"gone":{"type":"boolean","default":false}},
            "objects":[{"id":"prop","component":"prop","position":[0,1,0],"physics":{"size":[1,1,1],"health":100}}],
            "scripts":[{"path":"events.luau","side":"server","attachment":"world","capabilities":["state","objects"]}]
        })).unwrap();
        let script = "world.on('damage', function(e) assert(e.target == 'prop'); world.state.set('hits', world.state.get('hits') + 1); world.damage('prop', 0) end); world.on('destroy', function(e) assert(e.target == 'prop'); world.state.set('gone', true) end)";
        let mut instance = Instance::create(
            "events".into(),
            &Join::default(),
            "hash".into(),
            "owner".into(),
            definition,
            vec![script.into()],
            60,
        )
        .unwrap();
        instance.damage("prop", 1.0, None);
        assert_eq!(instance.variables.lock().unwrap()["hits"], 0);
        instance.tick();
        assert_eq!(instance.variables.lock().unwrap()["hits"], 1);
        assert_eq!(
            instance.creator_events.len(),
            1,
            "Handler-generated damage must wait for the next tick"
        );
        instance.tick();
        assert_eq!(instance.variables.lock().unwrap()["hits"], 2);
        instance.damage("prop", 1000.0, None);
        assert!(!instance.entities.contains_key("prop"));
        for _ in 0..4 {
            instance.tick();
        }
        assert_eq!(instance.variables.lock().unwrap()["gone"], true);
        assert!(instance.creator_events.is_empty());
        for _ in 0..140 {
            instance.queue_creator_event(json!({"event":"damage","target":"prop"}));
        }
        assert_eq!(instance.creator_events.len(), 128);
        assert!(instance.creator_event_overflow);
        instance.flush_creator_events();
        assert!((120..128).contains(&instance.creator_events.len()));
        assert!(!instance.creator_event_overflow);
    }
    #[test]
    fn empty_is_destroyed() {
        let mut r = Registry::default();
        let j = Join {
            world: "w".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let id = r.admit(&j, "a", "A", "hash", manifest(), vec![]).unwrap();
        let other = Join {
            instance: id.clone(),
            ..j.clone()
        };
        r.admit(&other, "b", "B", "hash", manifest(), vec![])
            .unwrap();
        r.leave("a");
        assert!(r.instances.contains_key(&id));
        r.leave("b");
        assert!(r.instances.is_empty());
        assert!(
            r.admit(&other, "a", "A", "hash", manifest(), vec![])
                .is_err()
        );
    }
    #[test]
    fn automatic_contact_release_notifies_scripts_after_disconnect() {
        let mut definition = manifest();
        definition.variables.insert(
            "released".into(),
            crate::assets::StateVariable::Boolean { default: false },
        );
        definition.scripts = serde_json::from_value(json!([{"path":"release.luau","side":"server","attachment":"world","capabilities":["state"]}])).unwrap();
        let source = "world.on('release', function(e) assert(e.actor == 'a' and e.target == 'b' and e.segment == 'hair' and e.hand == 0 and (e.reason == 'disconnect' or e.reason == 'policy' or e.reason == 'avatar_change')); world.state.set('released', true) end)";
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let id = registry
            .admit(
                &join,
                "a",
                "A",
                "hash",
                definition.clone(),
                vec![source.into()],
            )
            .unwrap();
        registry
            .admit(
                &Join {
                    instance: id.clone(),
                    ..join
                },
                "b",
                "B",
                "hash",
                definition,
                vec![],
            )
            .unwrap();
        registry
            .instances
            .get_mut(&id)
            .unwrap()
            .interactions
            .insert(
                "a:0".into(),
                Interaction {
                    actor: "a".into(),
                    target: "b".into(),
                    segment: "hair".into(),
                    bone: "Hair".into(),
                    hand: 0,
                },
            );
        registry.leave("a");
        let instance = &registry.instances[&id];
        assert!(instance.interactions.is_empty());
        assert_eq!(instance.creator_events[0]["reason"], "disconnect");
        assert_eq!(instance.variables.lock().unwrap()["released"], false);
        registry.step();
        assert_eq!(
            registry.instances[&id].variables.lock().unwrap()["released"],
            true
        );
        assert!(
            registry.instances[&id]
                .context
                .lock()
                .unwrap()
                .interaction_actor
                .is_none()
        );
        let instance = registry.instances.get_mut(&id).unwrap();
        instance
            .variables
            .lock()
            .unwrap()
            .insert("released".into(), json!(false));
        instance.interactions.insert(
            "a:0".into(),
            Interaction {
                actor: "a".into(),
                target: "b".into(),
                segment: "hair".into(),
                bone: "Hair".into(),
                hand: 0,
            },
        );
        instance.release_contacts(vec!["missing".into(), "a:0".into(), "a:0".into()], "policy");
        assert_eq!(instance.creator_events.len(), 1);
        assert_eq!(instance.creator_events[0]["reason"], "policy");
        instance.tick();
        assert_eq!(instance.variables.lock().unwrap()["released"], true);
        let grab = Interaction {
            actor: "a".into(),
            target: "b".into(),
            segment: "hair".into(),
            bone: "Hair".into(),
            hand: 0,
        };
        instance.interactions.insert("a:0".into(), grab.clone());
        instance
            .set_avatar("b", Some(("avatar".into(), "version".into())))
            .unwrap();
        assert!(instance.interactions.is_empty());
        assert_eq!(instance.creator_events[0]["reason"], "avatar_change");
        instance.tick();
        instance.interactions.insert("a:0".into(), grab);
        instance
            .set_avatar("b", Some(("avatar".into(), "version".into())))
            .unwrap();
        assert_eq!(
            instance.interactions.len(),
            1,
            "Reselecting the same immutable avatar must not release grabs"
        );
        assert!(instance.creator_events.is_empty());
        instance.set_avatar("b", None).unwrap();
        assert!(instance.interactions.is_empty());
    }
    #[test]
    fn sandbox_budget_and_state() {
        let vars = Arc::new(Mutex::new(BTreeMap::new()));
        assert!(Sandbox::new("while true do end", vars.clone(), &[]).is_err());
        let s=Sandbox::new("assert(io == nil and os == nil); world.on_interact('switch', function(p) world.state.set('on', true) end)",vars.clone(),&["state".into()]).unwrap();
        s.interact("switch", "a").unwrap();
        assert_eq!(vars.lock().unwrap()["on"], true);
        for source in [
            "world.state.set('on', 1)",
            "world.state.set('undeclared', true)",
            "world.state.get('undeclared')",
        ] {
            assert!(Sandbox::new(source, vars.clone(), &["state".into()]).is_err());
            assert_eq!(
                *vars.lock().unwrap(),
                BTreeMap::from([("on".into(), json!(true))])
            );
        }
        assert!(Sandbox::new("world.state.set('on', false)", vars.clone(), &[]).is_err());
    }
    #[test]
    fn declared_defaults_are_ephemeral_instance_state() {
        let mut definition = manifest();
        definition.variables.insert(
            "on".into(),
            crate::assets::StateVariable::Boolean { default: false },
        );
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Join::default()
        };
        let id = registry
            .admit(&join, "a", "A", "version", definition.clone(), vec![])
            .unwrap();
        assert_eq!(
            registry.instances[&id].variables.lock().unwrap()["on"],
            false
        );
        registry.instances[&id]
            .variables
            .lock()
            .unwrap()
            .insert("on".into(), json!(true));
        registry.leave("a");
        let fresh = registry
            .admit(&join, "a", "A", "version", definition, vec![])
            .unwrap();
        assert_eq!(
            registry.instances[&fresh].variables.lock().unwrap()["on"],
            false
        );
    }
    #[test]
    fn server_timers_do_not_capture_transmission_identity() {
        let vars = Arc::new(Mutex::new(BTreeMap::from([("on".into(), json!(false))])));
        let script = Sandbox::new(
            "world.after(0.1, function() world.state.set('on', true) end)",
            vars.clone(),
            &["state".into()],
        )
        .unwrap();
        script.event("tick", &json!({"dt":0.05})).unwrap();
        assert_eq!(vars.lock().unwrap()["on"], false);
        script.event("tick", &json!({"dt":0.05})).unwrap();
        assert_eq!(vars.lock().unwrap()["on"], true);
        let script = Sandbox::new("world.on_interact('send', function(player) world.after(0, function() world.radio_message('desk', player.id) end) end)", vars, &["radio".into()]).unwrap();
        script.interact("send", "authenticated").unwrap();
        assert!(script.event("tick", &json!({"dt":0.01})).is_err());
        assert!(script.context.lock().unwrap().commands.is_empty());
        assert!(script.context.lock().unwrap().interaction_actor.is_none());
        let mut definition = manifest();
        definition.variables.insert(
            "on".into(),
            crate::assets::StateVariable::Boolean { default: false },
        );
        definition.scripts = serde_json::from_value(json!([{"path":"script.luau","side":"server","attachment":"world","capabilities":["state"]}])).unwrap();
        let mut instance = Instance::create(
            "timer".into(),
            &Join {
                world: "world".into(),
                ..Join::default()
            },
            "version".into(),
            "owner".into(),
            definition,
            vec!["world.after(0.1, function() world.state.set('on', true) end)".into()],
            60,
        )
        .unwrap();
        instance.dispatch("tick", json!({"dt":0.1}));
        assert_eq!(instance.variables.lock().unwrap()["on"], true);
    }
    #[test]
    fn server_radio_uses_only_current_authenticated_interaction() {
        let vars = Arc::new(Mutex::new(BTreeMap::new()));
        assert!(
            Sandbox::new(
                "world.radio_message('desk','CQ')",
                vars.clone(),
                &["radio".into()]
            )
            .is_err()
        );
        assert!(Sandbox::new("world.radio_message('desk','CQ')", vars.clone(), &[]).is_err());
        let script = Sandbox::new("world.on_interact('send', function(player) player.id = 'forged'; world.radio_message('desk', 'CQ CREATOR') end); world.on_interact('fail', function() error('failure') end); world.on('tick', function() world.radio_message('desk', 'CQ TIMER') end)", vars, &["radio".into()]).unwrap();
        script.interact("send", "authenticated").unwrap();
        let mut context = script.context.lock().unwrap();
        assert!(context.interaction_actor.is_none());
        assert!(
            matches!(&context.commands[0], Command::Radio(actor, request) if actor == "authenticated" && !request.is_voice())
        );
        context.commands.clear();
        drop(context);
        assert!(script.interact("fail", "authenticated").is_err());
        assert!(script.context.lock().unwrap().interaction_actor.is_none());
        assert!(script.event("tick", &json!({})).is_err());
        assert!(script.context.lock().unwrap().commands.is_empty());
    }
    #[test]
    fn sound_commands_require_capability_and_packaged_declaration() {
        let vars = Arc::new(Mutex::new(BTreeMap::new()));
        assert!(Sandbox::new("world.play_sound('bell')", vars, &[]).is_err());
        let mut definition = manifest();
        definition.sounds =
            serde_json::from_value(json!([{"id":"bell","path":"audio/bell.opus"}])).unwrap();
        definition.scripts = serde_json::from_value(json!([{"path":"script.luau","side":"server","attachment":"world","capabilities":["audio"]}])).unwrap();
        let mut instance = Instance::create("test".into(), &Join {world:"world".into(), ..Join::default()}, "version".into(), "owner".into(), definition,
            vec!["world.play_sound('bell'); world.seek_sound('bell', 0.1); world.stop_sound('bell'); world.play_sound('missing')".into()], 60).unwrap();
        let actions: Vec<_> = instance
            .events
            .iter()
            .map(|event| event["action"].as_str().unwrap())
            .collect();
        assert_eq!(actions, ["play_sound", "seek_sound", "stop_sound"]);
        instance.events.clear();
    }
    #[test]
    fn vehicle_stale_input_and_prop_lease_cleanup() {
        let definition:Manifest=serde_json::from_value(json!({"schema":1,"kind":"world","name":"Physics","scene":"world.glb",
            "objects":[{"id":"ground","component":"static","position":[0,-0.1,0],"size":[20,0.2,20]},
                {"id":"car","component":"vehicle","position":[0,0.5,-1],"physics":{"size":[1,1,1]}},
                {"id":"prop","component":"prop","position":[-1,1,0],"physics":{"size":[0.2,0.2,0.2]}}]})).unwrap();
        let mut registry = Registry::default();
        let join = Join {
            world: "world".into(),
            send_divisor: 1,
            receive_divisor: 1,
            ..Default::default()
        };
        let id = registry
            .admit(&join, "a", "A", "hash", definition.clone(), vec![])
            .unwrap();
        registry
            .admit(
                &Join {
                    instance: id.clone(),
                    ..join
                },
                "b",
                "B",
                "hash",
                definition,
                vec![],
            )
            .unwrap();
        let instance = registry.instances.get_mut(&id).unwrap();
        assert!(
            instance
                .action(
                    "a",
                    Action::Grab {
                        target: "prop".into(),
                        hand: 9
                    }
                )
                .is_err()
        );
        assert!(instance.entities["prop"].controller.is_none());
        instance
            .action(
                "a",
                Action::Lease {
                    target: "car".into(),
                },
            )
            .unwrap();
        assert!(
            instance
                .action(
                    "b",
                    Action::Drive {
                        target: "car".into(),
                        throttle: 1.0,
                        turn: 0.0
                    }
                )
                .is_err()
        );
        instance
            .action(
                "a",
                Action::Drive {
                    target: "car".into(),
                    throttle: 1.0,
                    turn: 0.0,
                },
            )
            .unwrap();
        for _ in 0..30 {
            registry.step();
        }
        let position = registry.instances[&id].entities["car"].position;
        assert!(position[2] < -1.2);
        for _ in 0..30 {
            registry.step();
        }
        assert!((registry.instances[&id].entities["car"].position[2] - position[2]).abs() < 0.01);
        registry
            .instances
            .get_mut(&id)
            .unwrap()
            .action(
                "a",
                Action::Grab {
                    target: "prop".into(),
                    hand: 0,
                },
            )
            .unwrap();
        registry.leave("a");
        let instance = registry.instances.get_mut(&id).unwrap();
        assert!(instance.entities["car"].controller.is_none());
        assert!(instance.entities["prop"].controller.is_none());
        assert_eq!(
            instance.scene.body_mut("prop").unwrap().body_type(),
            rapier3d::prelude::RigidBodyType::Dynamic
        );
        instance
            .action(
                "b",
                Action::Lease {
                    target: "car".into(),
                },
            )
            .unwrap();
    }
}
