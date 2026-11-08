//! Bounds shared by replicated-state publication and client snapshot queries.
pub const REPLICATED_STATE_BYTES: usize = 16 * 1024;

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
