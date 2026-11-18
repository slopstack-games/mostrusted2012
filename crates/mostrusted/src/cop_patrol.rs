//! A patrolling cop (driver state 0xd Patrolling, update `FUN_005101e0`): ported in `crates/mw-police`
//! (`patrol.rs`; docs/decomp.md "How a cop patrols (2026-10-16)", "Police and pursuit (ported)"). The game drives
//! a patrolling cop with traffic: the cop asks the traffic system for a ghost and rides it. This module is the
//! runtime's side: the traffic system as the crate's `PatrolTraffic`.

use bevy::prelude::*;
use mw_police::patrol::{GhostPose as PatrolPose, PatrolTraffic};
pub use mw_police::patrol::{PatrolDrive, PatrolInput};

use crate::traffic::{GhostId, TrafficSystem};

/// The Patrolling state's mode, with the runtime's ghost handle.
pub type PatrolMode = mw_police::patrol::PatrolMode<GhostId>;

/// The traffic system for one cop (ghost requests are made for its entity).
pub struct Lanes<'a> {
    pub traffic: &'a mut TrafficSystem,
    pub cop: Entity,
}

fn pose(p: crate::traffic::GhostPose) -> PatrolPose {
    PatrolPose { position: p.position, forward: p.forward, up: p.up, speed: p.speed, cruise: p.cruise }
}

impl PatrolTraffic for Lanes<'_> {
    type Ghost = GhostId;
    fn nearest_lane(&self, point: Vec3, radius: f32, max_dy: Option<f32>, active_only: bool) -> Option<Vec3> {
        self.traffic.nearest_lane(point, radius, max_dy, active_only).map(|l| l.position)
    }
    fn add_ghost(&mut self, at: Vec3, _cop: u32, half_length: f32) -> Option<GhostId> {
        self.traffic.add_ghost(at, self.cop, half_length)
    }
    fn ghost(&mut self, ghost: GhostId) -> Option<PatrolPose> {
        self.traffic.ghost(ghost).map(pose)
    }
    fn set_ghost_override(&mut self, ghost: GhostId, value: u8) {
        self.traffic.set_ghost_override(ghost, value)
    }
    fn remove_ghost(&mut self, ghost: GhostId) {
        self.traffic.remove_ghost(ghost)
    }
    fn ghost_path(&self, ghost: GhostId, ahead: f32) -> Vec<Vec3> {
        self.traffic.ghost_path(ghost, ahead)
    }
}
