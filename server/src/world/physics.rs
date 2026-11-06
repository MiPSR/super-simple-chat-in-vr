//! Rapier owns shared collisions. Player tracking is never predicted here.
use crate::assets::Physics;
use rapier3d::{math::Vector, prelude::*};
use std::collections::BTreeMap;

pub struct Scene {
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    pub bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse: ImpulseJointSet,
    multibody: MultibodyJointSet,
    soft: SoftBodySet,
    ccd: CCDSolver,
    handles: BTreeMap<String, RigidBodyHandle>,
    names: std::collections::HashMap<ColliderHandle, String>,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            handles: BTreeMap::new(),
            names: Default::default(),
        }
    }
}
impl Scene {
    pub fn insert(&mut self, id: &str, position: [f32; 3], spec: &Physics, vehicle: bool) {
        let builder = if spec.dynamic {
            RigidBodyBuilder::dynamic()
        } else {
            RigidBodyBuilder::fixed()
        };
        let mut builder = builder
            .translation(Vector::from(position))
            .gravity_scale(spec.gravity_scale)
            .ccd_enabled(true)
            .linear_damping(if vehicle { 2.0 } else { 0.1 });
        if vehicle {
            builder = builder.enabled_rotations(false, true, false);
        }
        let body = self.bodies.insert(builder.build());
        let collider = if spec.shape == "sphere" {
            ColliderBuilder::ball(spec.size[0] / 2.0)
        } else {
            ColliderBuilder::cuboid(spec.size[0] / 2.0, spec.size[1] / 2.0, spec.size[2] / 2.0)
        };
        let collider = self.colliders.insert_with_parent(
            collider.mass(spec.mass).friction(0.8).build(),
            body,
            &mut self.bodies,
        );
        self.handles.insert(id.into(), body);
        self.names.insert(collider, id.into());
    }
    pub fn body_mut(&mut self, id: &str) -> Option<&mut RigidBody> {
        self.bodies.get_mut(*self.handles.get(id)?)
    }
    pub fn transform(&self, id: &str) -> Option<([f32; 3], [f32; 4])> {
        let body = self.bodies.get(*self.handles.get(id)?)?;
        Some((body.translation().to_array(), body.rotation().to_array()))
    }
    pub fn velocity(&mut self, id: &str, velocity: [f32; 3]) {
        if let Some(body) = self.body_mut(id) {
            body.set_linvel(Vector::from(velocity), true);
        }
    }
    pub fn drive(&mut self, id: &str, throttle: f32, turn: f32, speed: f32, turn_speed: f32) {
        if let Some(body) = self.body_mut(id) {
            let forward = body.rotation() * Vector::new(0.0, 0.0, -1.0);
            body.set_linvel(
                Vector::new(
                    forward.x * throttle * speed,
                    body.linvel().y,
                    forward.z * throttle * speed,
                ),
                true,
            );
            body.set_angvel(Vector::new(0.0, -turn * turn_speed, 0.0), true);
        }
    }
    pub fn hold(&mut self, id: &str, position: [f32; 3]) {
        if let Some(body) = self.body_mut(id) {
            body.set_body_type(RigidBodyType::KinematicPositionBased, true);
            let mut pose = *body.position();
            pose.translation = Vector::from(position);
            body.set_next_kinematic_position(pose);
        }
    }
    pub fn release(&mut self, id: &str) {
        if let Some(body) = self.body_mut(id) {
            body.set_body_type(RigidBodyType::Dynamic, true);
        }
    }
    pub fn remove(&mut self, id: &str) {
        if let Some(handle) = self.handles.remove(id) {
            if let Some(body) = self.bodies.get(handle) {
                for c in body.colliders() {
                    self.names.remove(c);
                }
            }
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse,
                &mut self.multibody,
                &mut self.soft,
                true,
            );
        }
    }
    pub fn step(&mut self, dt: f32) -> Vec<(String, String)> {
        let params = IntegrationParameters {
            dt,
            ..Default::default()
        };
        self.pipeline.step(
            Vector::new(0.0, -9.81, 0.0),
            &params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse,
            &mut self.multibody,
            &mut self.soft,
            &mut self.ccd,
            &(),
            &(),
        );
        self.narrow
            .contact_pairs()
            .filter(|p| p.has_any_active_contact())
            .filter_map(|p| {
                Some((
                    self.names.get(&p.collider1)?.clone(),
                    self.names.get(&p.collider2)?.clone(),
                ))
            })
            .collect()
    }
}
