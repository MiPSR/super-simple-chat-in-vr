//! Persist only operator policies, never credentials, paths or live game state.
use crate::config::Config;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const KEY: &str = "operator_policy_v1";

macro_rules! policies {
    ($($(#[$meta:meta])* $field:ident: $ty:ty),* $(,)?) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Settings { $($(#[$meta])* pub $field: $ty,)* }
        impl Settings {
            // The generated field list intentionally mixes Copy and owned types.
            #[allow(clippy::clone_on_copy)]
            pub fn from_config(config: &Config) -> Self {
                Self { $($field: config.$field.clone(),)* }
            }
            #[allow(clippy::clone_on_copy)]
            pub fn apply(&self, config: &mut Config) {
                $(config.$field = self.$field.clone();)*
            }
        }
    };
}
policies! {
    tps: u32,
    #[serde(default)]
    script_policy: crate::world::ScriptPolicy,
    network_rates: crate::realtime::rates::Policy,
    instance_limit: usize,
    instance_players: usize,
    default_world_slots: u32,
    default_avatar_slots: u32,
    default_world_bytes: u64,
    default_avatar_bytes: u64,
    voice_bitrate: u32,
    audio_effect_max: f32,
    radio_channels: std::collections::BTreeSet<u16>,
    radio_bitrate: u32,
    radio_presets: Vec<String>,
    radio_noise: f32,
    radio_interference: f32,
    radio_filter_hz: f32,
    radio_ft8_interval: u64,
    radio_sstv_interval: u64,
}

impl Settings {
    pub fn validate(&self, base: &Config) -> anyhow::Result<()> {
        let mut candidate = base.clone();
        self.apply(&mut candidate);
        candidate.validate()
    }
    pub fn read(db: &Connection) -> anyhow::Result<Option<Self>> {
        let json: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key=?", [KEY], |r| r.get(0))
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(Into::into)).transpose()
    }
    pub fn save(&self, db: &Connection) -> anyhow::Result<()> {
        db.execute(
            "INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![KEY, serde_json::to_string(self)?],
        )?;
        Ok(())
    }
}
