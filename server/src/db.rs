use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn open(path: &Path) -> anyhow::Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch(include_str!("../migrations/001.sql"))?;
    Ok(db)
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub can_touch_me: bool,
    pub can_touch_others: bool,
    pub quality: u8,
    pub send_divisor: u32,
    pub receive_divisor: u32,
    pub sleeping: bool,
    pub phone_hand: String,
    pub call_mode: String,
    pub input_device: String,
    pub output_device: String,
    pub voice_volume: f32,
    pub microphone_gain: f32,
    pub resolution_scale: f32,
    pub antialiasing: String,
    pub upscaler: String,
    pub texture_lod_bias: f32,
    pub texture_reduction: u8,
    pub radio_preset: String,
    pub radial_hold_seconds: f32,
    pub radial_actions: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            can_touch_me: false,
            can_touch_others: false,
            quality: 1,
            send_divisor: 1,
            receive_divisor: 1,
            sleeping: false,
            phone_hand: "left".into(),
            call_mode: "automatic".into(),
            input_device: "Default".into(),
            output_device: "Default".into(),
            voice_volume: 1.0,
            microphone_gain: 1.0,
            resolution_scale: 1.0,
            antialiasing: "msaa_2x".into(),
            upscaler: "bilinear".into(),
            texture_lod_bias: 0.0,
            texture_reduction: 0,
            radio_preset: "standard".into(),
            radial_hold_seconds: 0.45,
            radial_actions: [
                "microphone",
                "sleep",
                "phone_hand",
                "interact",
                "avatar_action",
                "video_pause",
                "phone",
                "leave",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.quality <= 2
            && [1, 2, 4].contains(&self.send_divisor)
            && [1, 2, 4].contains(&self.receive_divisor)
            && matches!(self.phone_hand.as_str(), "left" | "right")
            && matches!(self.call_mode.as_str(), "automatic" | "spatial" | "private")
            && [&self.input_device, &self.output_device]
                .iter()
                .all(|name| {
                    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
                })
            && self.voice_volume.is_finite()
            && (0.0..=1.0).contains(&self.voice_volume)
            && self.microphone_gain.is_finite()
            && (0.0..=2.0).contains(&self.microphone_gain)
            && self.resolution_scale.is_finite()
            && (0.5..=1.5).contains(&self.resolution_scale)
            && matches!(
                self.antialiasing.as_str(),
                "off" | "msaa_2x" | "msaa_4x" | "msaa_8x"
            )
            && matches!(self.upscaler.as_str(), "bilinear" | "fsr1")
            && self.texture_lod_bias.is_finite()
            && (0.0..=3.0).contains(&self.texture_lod_bias)
            && self.texture_reduction <= 3
            && matches!(self.radio_preset.as_str(), "standard" | "narrow" | "weak")
            && self.radial_hold_seconds.is_finite()
            && (0.15..=1.5).contains(&self.radial_hold_seconds)
            && self.radial_actions.len() == 8
            && self.radial_actions.iter().all(|action| {
                matches!(
                    action.as_str(),
                    "microphone"
                        | "sleep"
                        | "phone_hand"
                        | "interact"
                        | "avatar_action"
                        | "video_pause"
                        | "phone"
                        | "leave"
                        | "none"
                )
            })
    }
}
pub fn preferences(db: &Connection, id: &str) -> Preferences {
    db.query_row("SELECT preferences FROM users WHERE id=?", [id], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|s| serde_json::from_str(&s).ok())
    .unwrap_or_default()
}
pub fn allowed(db: &Connection, id: &str) -> bool {
    db.query_row("SELECT enabled FROM users WHERE id=?", [id], |r| {
        r.get::<_, bool>(0)
    })
    .unwrap_or(false)
}
pub fn blocked(db: &Connection, a: &str, b: &str, flag: u32) -> bool {
    db.query_row("SELECT EXISTS(SELECT 1 FROM blocks WHERE ((owner=?1 AND target=?2) OR (owner=?2 AND target=?1)) AND (flags & ?3)!=0)", params![a, b, flag | 16], |r| r.get(0)).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preference_defaults_migrate_and_new_controls_are_bounded() {
        let old: Preferences =
            serde_json::from_str(r#"{"can_touch_me":true,"quality":2}"#).unwrap();
        assert!(old.valid() && old.can_touch_me && old.quality == 2);
        assert_eq!(old.input_device, "Default");
        assert_eq!(old.texture_reduction, 0);
        assert_eq!(old.radial_actions.len(), 8);
        assert_eq!(old.radial_hold_seconds, 0.45);
        let defaults = serde_json::to_value(Preferences::default()).unwrap();
        for (key, value) in [
            ("resolution_scale", json!(0.49)),
            ("resolution_scale", json!(1.51)),
            ("voice_volume", json!(1.1)),
            ("microphone_gain", json!(-1)),
            ("texture_lod_bias", json!(4)),
            ("texture_reduction", json!(4)),
            ("radial_hold_seconds", json!(0.1)),
            ("radial_hold_seconds", json!(1.6)),
            ("radial_actions", json!(["none"])),
            ("radial_actions", json!(vec!["unknown"; 8])),
            ("radial_actions", json!(vec!["none"; 9])),
            ("antialiasing", json!("unknown")),
            ("upscaler", json!("unimplemented")),
            ("radio_preset", json!("ultra")),
            ("input_device", json!("bad\nname")),
            ("output_device", json!("x".repeat(257))),
        ] {
            let mut value_map = defaults.clone();
            value_map[key] = value;
            assert!(
                !serde_json::from_value::<Preferences>(value_map)
                    .unwrap()
                    .valid(),
                "{key}"
            );
        }
        let invalid = Preferences {
            voice_volume: f32::NAN,
            ..Preferences::default()
        };
        assert!(!invalid.valid());
    }
}
