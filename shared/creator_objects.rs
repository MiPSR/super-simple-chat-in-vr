//! Read-only, JSON-only object queries shared by authoritative and visual hosts.
use serde_json::Value;
use std::{collections::BTreeMap, ops::Bound};

pub const MAX_TAG_BYTES: usize = 65536;
pub type TagPage = (Vec<String>, Option<String>);

pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 32
        && tag
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
}

pub fn tag_bytes(tags: &[String]) -> usize {
    tags.iter().map(|tag| tag.len() + 3).sum::<usize>() + 2
}

pub fn find_tag(
    objects: &BTreeMap<String, Value>,
    tag: &str,
    after: Option<String>,
) -> Result<TagPage, &'static str> {
    if !valid_tag(tag) || after.as_ref().is_some_and(|id| id.len() > 64) {
        return Err("Invalid object tag/query cursor");
    }
    let start = after.map_or(Bound::Unbounded, Bound::Excluded);
    let mut ids: Vec<String> = objects
        .range((start, Bound::Unbounded))
        .filter(|(_, object)| {
            object["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().any(|value| value.as_str() == Some(tag)))
        })
        .take(257)
        .map(|(id, _)| id.clone())
        .collect();
    let next = if ids.len() > 256 {
        ids.pop();
        ids.last().cloned()
    } else {
        None
    };
    Ok((ids, next))
}
