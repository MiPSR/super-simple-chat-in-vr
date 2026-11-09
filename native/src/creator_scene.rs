//! Trusted host snapshots, never Godot objects, are shared by visual Lua VMs.
use crate::creator_objects::{MAX_TAG_BYTES, tag_bytes, valid_tag};
use godot::{classes::RefCounted, prelude::*};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_OBJECTS: usize = 4096;
pub(crate) type SharedScene = Arc<Mutex<SceneState>>;

#[derive(Default)]
pub(crate) struct SceneState {
    pub objects: BTreeMap<String, Value>,
    pub variables: BTreeMap<String, Value>,
    state_tick: Option<u64>,
    graph_tick: Option<u64>,
    pose_ticks: BTreeMap<String, u64>,
}

fn vector(value: &Value, length: usize) -> bool {
    value.as_array().is_some_and(|values| {
        values.len() == length
            && values
                .iter()
                .all(|value| value.as_f64().is_some_and(f64::is_finite))
    })
}

fn parse_objects(bytes: &[u8]) -> Result<BTreeMap<String, Value>, &'static str> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err("Object snapshot exceeds 2 MiB");
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid object snapshot JSON")?;
    let Value::Object(entries) = root else {
        return Err("Expected an object snapshot map");
    };
    if entries.len() > MAX_OBJECTS {
        return Err("At most 4096 snapshot objects");
    }
    let mut objects = BTreeMap::new();
    let mut tag_total = 0;
    for (id, value) in entries {
        if id.is_empty() || id.len() > 64 {
            return Err("Invalid snapshot object ID");
        }
        let Value::Object(mut object) = value else {
            return Err("Invalid snapshot object");
        };
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "id" | "component"
                    | "name"
                    | "tags"
                    | "position"
                    | "rotation"
                    | "size"
                    | "health"
                    | "visible"
                    | "active"
                    | "controller"
                    | "owner"
                    | "prefab"
                    | "attached_to"
                    | "attachment_offset"
            )
        }) {
            return Err("Unsupported snapshot object property");
        }
        if object
            .get("id")
            .is_some_and(|value| value.as_str() != Some(id.as_str()))
        {
            return Err("Inconsistent snapshot object ID");
        }
        object.insert("id".into(), json!(id));
        if object
            .get("name")
            .is_some_and(|value| !value.as_str().is_some_and(|name| name.len() <= 256))
        {
            return Err("Invalid snapshot display name");
        }
        let mut tags: Vec<String> = match object.get("tags") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("Invalid snapshot tags")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|tag| valid_tag(tag))
                        .map(String::from)
                        .ok_or("Invalid snapshot tag")
                })
                .collect::<Result<_, _>>()?,
        };
        if tags.len() > 32 || tags.iter().collect::<HashSet<_>>().len() != tags.len() {
            return Err("At most 32 unique resolved object tags");
        }
        tags.sort();
        tag_total += tag_bytes(&tags);
        if tag_total > MAX_TAG_BYTES {
            return Err("Snapshot tags exceed 64 KiB");
        }
        object.insert("tags".into(), json!(tags));
        for field in ["position", "size", "attachment_offset"] {
            if object.get(field).is_some_and(|value| !vector(value, 3)) {
                return Err("Invalid snapshot vector");
            }
        }
        if object
            .get("rotation")
            .is_some_and(|value| !vector(value, 4))
        {
            return Err("Invalid snapshot rotation");
        }
        if object
            .get("health")
            .is_some_and(|value| !value.as_f64().is_some_and(f64::is_finite))
        {
            return Err("Invalid snapshot scalar");
        }
        for field in ["active", "visible"] {
            if object.get(field).is_some_and(|value| !value.is_boolean()) {
                return Err("Invalid snapshot boolean");
            }
        }
        for field in ["owner", "controller", "prefab", "component", "attached_to"] {
            if object.get(field).is_some_and(|value| {
                !value.is_null()
                    && !value
                        .as_str()
                        .is_some_and(|text| !text.is_empty() && text.len() <= 64)
            }) {
                return Err("Invalid snapshot reference/component");
            }
        }
        objects.insert(id, Value::Object(object));
    }
    Ok(objects)
}

impl SceneState {
    pub fn replace_state(&mut self, bytes: &[u8], tick: u64) -> Result<(), &'static str> {
        // Godot round-trips integer JSON numbers through floats. Leave bounded
        // formatting headroom above the server's 16-KiB logical state budget.
        if bytes.len() > 2 * crate::creator_state::REPLICATED_STATE_BYTES {
            return Err("State snapshot exceeds 32 KiB");
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| "Invalid state snapshot JSON")?;
        let values = value.as_object().ok_or("Expected state snapshot map")?;
        if values.len() > 128
            || values.iter().any(|(key, value)| {
                !crate::creator_state::valid_key(key)
                    || !(value.is_boolean()
                        || value.as_str().is_some_and(|text| text.len() <= 4096)
                        || value
                            .as_f64()
                            .is_some_and(|number| number.is_finite() && number.abs() <= 1e9)
                        || value.as_array().is_some_and(|items| {
                            items.len() == 3
                                && items.iter().all(|v| {
                                    v.as_f64().is_some_and(|n| n.is_finite() && n.abs() <= 1e9)
                                })
                        }))
            })
        {
            return Err("Invalid replicated state value/key");
        }
        if self.state_tick.is_some_and(|latest| tick < latest) {
            return Ok(());
        }
        self.variables = values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.state_tick = Some(tick);
        Ok(())
    }
    pub fn replace(&mut self, bytes: &[u8], tick: u64) -> Result<(), &'static str> {
        let mut objects = parse_objects(bytes)?;
        if self.graph_tick.is_some_and(|latest| tick < latest) {
            return Ok(());
        }
        let mut pose_ticks = BTreeMap::new();
        for (id, object) in &mut objects {
            let previous_tick = self.pose_ticks.get(id).copied().unwrap_or(0);
            if previous_tick > tick
                && let Some(previous) = self.objects.get(id)
            {
                for field in ["position", "rotation"] {
                    if let Some(value) = previous.get(field) {
                        object[field] = value.clone();
                    }
                }
                pose_ticks.insert(id.clone(), previous_tick);
            } else {
                pose_ticks.insert(id.clone(), tick);
            }
        }
        self.objects = objects;
        self.pose_ticks = pose_ticks;
        self.graph_tick = Some(tick);
        Ok(())
    }

    pub fn transforms(&mut self, bytes: &[u8], tick: u64) -> Result<(), &'static str> {
        if bytes.len() > MAX_JSON_BYTES {
            return Err("Object transforms exceed 2 MiB");
        }
        let root: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid transform JSON")?;
        let entries = root.as_array().ok_or("Expected transform array")?;
        if entries.len() > MAX_OBJECTS {
            return Err("At most 4096 object transforms");
        }
        let mut seen = HashSet::new();
        for entry in entries {
            let object = entry.as_object().ok_or("Invalid object transform")?;
            let id = entry["id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 64)
                .ok_or("Invalid transform ID")?;
            if !seen.insert(id)
                || object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "id" | "position" | "rotation"))
                || !vector(&entry["position"], 3)
                || !vector(&entry["rotation"], 4)
            {
                return Err("Invalid/duplicate object transform");
            }
        }
        for entry in entries {
            let id = entry["id"].as_str().unwrap();
            if self.pose_ticks.get(id).is_some_and(|latest| tick < *latest) {
                continue;
            }
            if let Some(object) = self.objects.get_mut(id) {
                object["position"] = entry["position"].clone();
                object["rotation"] = entry["rotation"].clone();
                self.pose_ticks.insert(id.into(), tick);
            }
        }
        Ok(())
    }
}

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
pub(crate) struct CreatorScene {
    pub data: SharedScene,
    base: Base<RefCounted>,
}
#[godot_api]
impl CreatorScene {
    #[func]
    fn update_state(&self, bytes: PackedByteArray, tick: i64) -> GString {
        if tick < 0 {
            return "Invalid state snapshot tick".into();
        }
        match self
            .data
            .lock()
            .unwrap()
            .replace_state(bytes.as_slice(), tick as u64)
        {
            Ok(()) => GString::new(),
            Err(error) => error.into(),
        }
    }
    #[func]
    fn update_objects(&self, bytes: PackedByteArray, tick: i64) -> GString {
        if tick < 0 {
            return "Invalid snapshot tick".into();
        }
        match self
            .data
            .lock()
            .unwrap()
            .replace(bytes.as_slice(), tick as u64)
        {
            Ok(()) => GString::new(),
            Err(error) => error.into(),
        }
    }
    #[func]
    fn update_transforms(&self, bytes: PackedByteArray, tick: i64) -> GString {
        if tick < 0 {
            return "Invalid snapshot tick".into();
        }
        match self
            .data
            .lock()
            .unwrap()
            .transforms(bytes.as_slice(), tick as u64)
        {
            Ok(()) => GString::new(),
            Err(error) => error.into(),
        }
    }
    #[func]
    fn clear(&self) {
        *self.data.lock().unwrap() = SceneState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_snapshots_are_bounded_atomic_and_do_not_regress() {
        let mut scene = SceneState::default();
        scene
            .replace_state(br#"{"ready":false,"count":1,"waypoint":[1,2,3]}"#, 10)
            .unwrap();
        let original = scene.variables.clone();
        scene.replace_state(br#"{"count":0}"#, 9).unwrap();
        assert_eq!(scene.variables, original);
        for value in [
            json!([]),
            json!({"bad key":1}),
            json!({"x":null}),
            json!({"x":{"nested":true}}),
            json!({"x":[1,2]}),
            json!({"x":1e10}),
            json!({"x":"x".repeat(4097)}),
        ] {
            assert!(
                scene
                    .replace_state(&serde_json::to_vec(&value).unwrap(), 11)
                    .is_err()
            );
            assert_eq!(scene.variables, original);
        }
        let too_many: BTreeMap<_, _> = (0..129).map(|i| (format!("k{i}"), json!(false))).collect();
        assert!(
            scene
                .replace_state(&serde_json::to_vec(&too_many).unwrap(), 11)
                .is_err()
        );
        assert!(
            scene
                .replace_state(
                    &vec![b' '; 2 * crate::creator_state::REPLICATED_STATE_BYTES + 1],
                    11
                )
                .is_err()
        );
        scene.replace_state(b"{}", 11).unwrap();
        assert!(scene.variables.is_empty());
    }
    #[test]
    fn snapshots_and_poses_do_not_regress_or_resurrect_objects() {
        let mut scene = SceneState::default();
        scene
            .replace(br#"{"prop":{"tags":["old"],"position":[0,0,0]}}"#, 10)
            .unwrap();
        scene
            .transforms(
                br#"[{"id":"prop","position":[2,0,0],"rotation":[0,0,0,1]}]"#,
                12,
            )
            .unwrap();
        scene
            .replace(br#"{"prop":{"tags":["new"],"position":[1,0,0]}}"#, 11)
            .unwrap();
        assert_eq!(scene.objects["prop"]["position"], json!([2, 0, 0]));
        assert_eq!(scene.objects["prop"]["tags"], json!(["new"]));
        scene
            .transforms(
                br#"[{"id":"prop","position":[0,0,0],"rotation":[0,0,0,1]}]"#,
                9,
            )
            .unwrap();
        assert_eq!(scene.objects["prop"]["position"], json!([2, 0, 0]));
        scene.replace(b"{}", 13).unwrap();
        scene
            .transforms(
                br#"[{"id":"prop","position":[3,0,0],"rotation":[0,0,0,1]}]"#,
                14,
            )
            .unwrap();
        scene.replace(br#"{"prop":{"tags":[]}}"#, 8).unwrap();
        assert!(scene.objects.is_empty());
    }
    #[test]
    fn malformed_or_oversized_updates_are_atomic() {
        let mut scene = SceneState::default();
        scene
            .replace(br#"{"prop":{"tags":["game.prop"]}}"#, 1)
            .unwrap();
        let original = scene.objects.clone();
        for invalid in [
            br#"[]"#.as_slice(),
            br#"{"prop":{"id":"other"}}"#,
            br#"{"prop":{"script":"x"}}"#,
            br#"{"prop":{"tags":["a","a"]}}"#,
            br#"{"prop":{"position":[0,0]}}"#,
            br#"{"prop":{"name":17}}"#,
            br#"{"prop":{"attached_to":17}}"#,
            br#"{"prop":{"attachment_offset":[0,0]}}"#,
        ] {
            assert!(scene.replace(invalid, 2).is_err());
            assert_eq!(scene.objects, original);
        }
        assert!(scene.replace(&vec![b' '; MAX_JSON_BYTES + 1], 2).is_err());
        assert!(
            scene
                .replace(
                    &serde_json::to_vec(&json!({"prop":{"name":"x".repeat(257)}})).unwrap(),
                    2
                )
                .is_err()
        );
        let entries: BTreeMap<_, _> = (0..4097).map(|i| (format!("id{i}"), json!({}))).collect();
        assert!(
            scene
                .replace(&serde_json::to_vec(&entries).unwrap(), 2)
                .is_err()
        );
        let tags: Vec<_> = (0..32)
            .map(|i| format!("tag{i:02}{}", "x".repeat(27)))
            .collect();
        let entries: BTreeMap<_, _> = (0..100)
            .map(|i| (format!("id{i}"), json!({"tags":tags})))
            .collect();
        assert!(
            scene
                .replace(&serde_json::to_vec(&entries).unwrap(), 2)
                .is_err()
        );
        assert!(scene.transforms(br#"[{"id":"prop","position":[1,0,0],"rotation":[0,0,0,1]},{"id":"prop","position":[2,0,0],"rotation":[0,0,0,1]}]"#,2).is_err());
        assert_eq!(scene.objects, original);
    }
}
