//! Bounded in-memory log ring for the administrator UI. stderr output stays
//! as-is; this ring additionally keeps the most recent entries inspectable
//! through the authenticated admin API without a log shipper.
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::Mutex;

const LIMIT: usize = 256;

static RING: Mutex<Option<VecDeque<(i64, String)>>> = Mutex::new(None);

pub fn record(message: impl AsRef<str>) {
    let line = message.as_ref();
    eprintln!("{line}");
    if let Ok(mut guard) = RING.lock() {
        let ring = guard.get_or_insert_with(VecDeque::new);
        if ring.len() >= LIMIT {
            ring.pop_front();
        }
        ring.push_back((crate::db::now(), line.chars().take(2048).collect()));
    }
}

pub fn snapshot() -> Value {
    match RING.lock() {
        Ok(guard) => json!(guard
            .as_ref()
            .map(|ring| ring
                .iter()
                .map(|(time, line)| json!({"time":time,"line":line}))
                .collect::<Vec<_>>())
            .unwrap_or_default()),
        Err(_) => json!([]),
    }
}
