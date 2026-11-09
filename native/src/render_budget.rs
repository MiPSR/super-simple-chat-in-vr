//! Conservative retained-scene/workload admission, not a native/GPU allocator.
use godot::{classes::RefCounted, prelude::*};
use std::sync::Mutex;

#[derive(Clone, Copy, Default)]
struct Cost { nodes: u64, surfaces: u64, vertices: u64, textures: u64, bones: u64, lights: u64 }
static USAGE: Mutex<Cost> = Mutex::new(Cost { nodes: 0, surfaces: 0, vertices: 0, textures: 0, bones: 0, lights: 0 });
const LIMIT: Cost = Cost { nodes: 32768, surfaces: 16384, vertices: 4_000_000, textures: 512 * 1024 * 1024, bones: 4096, lights: 64 };

struct Reservation(Cost);
impl Reservation {
    fn new(cost: Cost) -> Result<Self, &'static str> {
        let mut usage = USAGE.lock().unwrap();
        if cost.nodes > LIMIT.nodes.saturating_sub(usage.nodes)
            || cost.surfaces > LIMIT.surfaces.saturating_sub(usage.surfaces)
            || cost.vertices > LIMIT.vertices.saturating_sub(usage.vertices)
            || cost.bones > LIMIT.bones.saturating_sub(usage.bones)
            || cost.lights > LIMIT.lights.saturating_sub(usage.lights)
            || cost.textures > LIMIT.textures.saturating_sub(usage.textures) {
            return Err("Client aggregate renderer budget exceeded");
        }
        usage.nodes += cost.nodes;
        usage.surfaces += cost.surfaces;
        usage.vertices += cost.vertices;
        usage.textures += cost.textures;
        usage.bones += cost.bones;
        usage.lights += cost.lights;
        Ok(Self(cost))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut usage = USAGE.lock().unwrap();
        usage.nodes -= self.0.nodes;
        usage.surfaces -= self.0.surfaces;
        usage.vertices -= self.0.vertices;
        usage.textures -= self.0.textures;
        usage.bones -= self.0.bones;
        usage.lights -= self.0.lights;
    }
}

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
struct RenderLease {
    reservation: Option<Reservation>,
    base: Base<RefCounted>,
}
#[godot_api]
impl RenderLease {
    #[func]
    fn reserve(&mut self, nodes: i64, surfaces: i64, vertices: i64, texture_bytes: i64, bones: i64, lights: i64) -> GString {
        if self.reservation.is_some() || [nodes,surfaces,vertices,texture_bytes,bones,lights].iter().any(|n| *n < 0) {
            return "Invalid or already-owned renderer reservation".into();
        }
        match Reservation::new(Cost { nodes: nodes as u64, surfaces: surfaces as u64,
            vertices: vertices as u64, textures: texture_bytes as u64, bones: bones as u64, lights: lights as u64 }) {
            Ok(reservation) => { self.reservation = Some(reservation); GString::new() },
            Err(error) => error.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_reservations_are_atomic_and_release_every_dimension() {
        for cost in [Cost { nodes: LIMIT.nodes, ..Cost::default() },
            Cost { surfaces: LIMIT.surfaces, ..Cost::default() },
            Cost { vertices: LIMIT.vertices, ..Cost::default() },
            Cost { textures: LIMIT.textures, ..Cost::default() },
            Cost { bones: LIMIT.bones, ..Cost::default() },
            Cost { lights: LIMIT.lights, ..Cost::default() }] {
            let lease = Reservation::new(cost).unwrap();
            assert!(Reservation::new(Cost { nodes: 1, surfaces: 1, vertices: 1, textures: 1, bones: 1, lights: 1 }).is_err());
            drop(lease);
        }
        assert!(Reservation::new(LIMIT).is_ok());
    }
}
