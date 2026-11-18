//! Glue between the runtime and `mw-ai`, the engine-free port of the original's AI racer driving (docs/decomp.md
//! "AI racers (ported)"): the racer state 8 (`FUN_0050da90`: steering, target speed, nitrous, emergency brake,
//! the pedal controller), the crash recovery (state 11, `FUN_004eeba0`), the racer interaction, `AIPlayer::FillInput`
//! and the rubber band's manager live there. This module only converts what the runtime knows into `mw-ai`'s
//! inputs: [`car_state`] (the AI vehicle state), [`path_of`] (the race route as the game's path nodes, 15 m
//! apart) and [`config_from_data`] (`AIConfig` from `GLOBALCONFIG.BNDL`); `ai.rs::ai_drive` runs
//! `mw_ai::Driver::tick` every fixed tick and applies its control record to the car's `CarControls`.
//!
//! What `mw-ai` does not cover yet and the runtime still supplies (named, not hand-written driving rules): the
//! path itself (the route's resampled centre line, [`crate::ai::AiRoute`]; the game's racing line `FUN_004e70e0`
//! and the lane planner `FUN_00502e20` are not ported, so the lane offset stays 0 and the nav-raycast clearances
//! are open), the obstacle sweep's world geometry (`FUN_004ff270`: `RacerInput::sweep` stays clear), the route
//! speed factor of the rubber band's expected time (`FUN_00661eb0`: no time correction) and the reset's placement.

use bevy::prelude::*;

use crate::ai::AiRoute;
use crate::drive::Car;

/// `AIConfig` (`GLOBALCONFIG.BNDL` 010000000007fb0e): the fields `mw-ai` reads, by hash; a field the data lacks
/// keeps the install's value.
pub fn config_from_data(store: &mw_assets::AssetStore) -> mw_ai::AiConfig {
    use mw_formats::genesys::Value;
    match store.genesys_object(mw_bundle::ResourceId(0x010000000007fb0e)) {
        Ok(obj) => mw_ai::AiConfig::from_fields(|hash| match obj.get(hash) {
            Some(Value::Float(v)) => Some(*v),
            _ => None,
        }),
        Err(e) => {
            warn!("AI: no AIConfig ({e}); using the defaults (the data's values)");
            mw_ai::AiConfig::default()
        }
    }
}

/// The race route as the game's path: the route's 15 m nodes (`FUN_004e95e0`) with the game's tangents and turns
/// (`mw_ai::Path::new`). Side clearances (the game's nav raycasts, `FUN_004e58e0`, not ported): the route's drivable
/// half-width at the node where the road graph gave one, else open (model decision, as before the port).
pub fn path_of(route: &AiRoute) -> mw_ai::Path {
    let points: Vec<mw_ai::V2> = route.nodes.iter().map(|n| mw_ai::V2::new(n.position.x, n.position.y)).collect();
    let mut path = mw_ai::Path::new(&points, route.closed);
    for (node, r) in path.nodes.iter_mut().zip(&route.nodes) {
        let half = route.half_width_at(r.s).min(mw_ai::OPEN_CLEARANCE);
        node.clearance = [half; 2];
    }
    path
}

/// The lateral acceleration of the obstacle sweep's turning circle (`FUN_00514ad0`): vehicle +0x550 × +0x5c4 ×
/// wheel 0's effective lateral surface grip × 9.81 (`.bss` 0x01106a10, live 9.81). +0x5c4 is the front axle's
/// `Pure_Lateral_Grip` peak coefficient (the +0x5c0 block is (gradient, peak, peak angle, ·): snapshot 2's
/// SL 65 AMG holds 0.27 / 2.737 / 25, its data's front curve). +0x550 = 1 in both snapshots (writer not traced:
/// model value).
pub fn turn_accel(car: &Car) -> f32 {
    const VEHICLE_550: f32 = 1.0;
    VEHICLE_550 * car.params.tyres[0].pure_lateral.peak * car.wheel_lateral_grip(0) * 9.81
}

/// The stopping distance from `speed` (`FUN_00518520`(speed, 0)): a = (μ × −(+0x7d8) + +0x7d4) / mass, b = μ ×
/// 9.81 with μ = vehicle +0x688 = the rear axle's `traction_pure_long` peak (snapshot 2's SL 65 AMG: 2.05, its
/// data's rear curve, the block at +0x680 being (max slip, max slip ratio, peak, peak ratio)). +0x7d8 = −1
/// (three cars in the snapshots) and +0x7d4 = 0.8 (the SL 65; 1.1 on the snapshot-1 car): writers not traced,
/// model values.
pub fn stop_distance(car: &Car, speed: f32) -> f32 {
    const VEHICLE_7D8: f32 = -1.0;
    const VEHICLE_7D4: f32 = 0.8;
    let mu = car.params.tyres[1].traction_pure_long.peak;
    let a = (mu * -VEHICLE_7D8 + VEHICLE_7D4) / car.params.mass.max(1.0);
    mw_ai::braking_distance(speed, 0.0, a, mu * 9.81)
}

/// A car as the AI code reads it (the 0x94-byte AI vehicle state): `id` is the player handle stand-in (the
/// entity's index), the radius the body's larger half extent (model decision: the writer of +0x50 is not traced).
pub fn car_state(id: i32, t: &Transform, velocity: Vec3, car: &Car) -> mw_ai::CarState {
    let fwd = t.rotation * Vec3::Z;
    let ext = car.params.body_half_extents;
    mw_ai::CarState {
        id,
        position: t.translation.to_array(),
        velocity: velocity.to_array(),
        forward: fwd.to_array(),
        heading: mw_ai::V2::new(fwd.x, fwd.z).normalize(),
        radius: ext.x.max(ext.z),
        half_length: ext.z,
        field_74: 0.0,
        nitrous_tank: car.nitrous.fill(),
        physical: true,
        nitrous_allowed: true,
        flag_5: false,
        contact: car.body_contact,
        all_wheels_off: car.wheels_down() == 0,
    }
}
