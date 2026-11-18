//! How a cop drives in a pursuit: the game's cop driver states, ported in `crates/mw-police` (docs/decomp.md
//! "Police and pursuit (ported)"). `police.rs` gathers the cars' state every fixed tick, calls the driver and
//! hands the result to the AI racers' driver (`ai.rs`, `ai_driver.rs`).

use bevy::prelude::*;
pub use mw_police::driver::*;

/// The cop's or the player's pose for the driver.
pub fn car_state(t: &Transform, velocity: Vec3) -> CarState {
    CarState::from_pose(t.translation, t.rotation, velocity)
}

/// The driver's overrides as the racers' driver takes them.
pub fn overrides(o: ControlOverride) -> crate::ai::ControlOverride {
    crate::ai::ControlOverride {
        steer: o.steer,
        throttle: o.throttle,
        brake: o.brake,
        handbrake: o.handbrake,
        nitrous: o.nitrous,
    }
}
