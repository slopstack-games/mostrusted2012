//! Free-roam collectables, first slice: speed cameras (docs/decomp.md "Free-roam rules").
//!
//! The cameras are the TriggerData boxes of kind '4' (`FUN_006e4920`). Entering one records the car's speed as
//! `FUN_006ed0a0` does: |speed| in mph (vehicle +0x1620, the ground speed signed by the forward component, here
//! `Car::speed`), counted only at or above `CollectablesConfig` field `d89eeda4` (offset 0x64, read through
//! `GameLogicConfig` +0xc), kept as `(int)(100 * mph)`. Best per camera; logged, no HUD yet.
//! Model decisions (open in the research log): the car's origin is the point tested against the box, and the box
//! `size` is taken as full extents; a pass fires on entering the box.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use mw_formats::{Collectable, Trigger};

use crate::Game;
use crate::drive::Car;

/// The minimum speed in mph: `CollectablesConfig` field `d89eeda4` in GLOBALCONFIG.BNDL (`mwtool genesys all --type
/// CollectablesConfig`), 50.0 in the install. Copied here for now; to be read through the GeneSys reader.
const MIN_MPH: f32 = 50.0;

pub struct FreeroamPlugin;

impl Plugin for FreeroamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpeedCameras>().add_systems(Update, speed_cameras);
    }
}

#[derive(Resource, Default)]
pub struct SpeedCameras {
    loaded: bool,
    cameras: Vec<Trigger>,
    min_mph: f32,
    inside: HashSet<u32>,
    /// Best recorded speed per camera id, in hundredths of a mph (as the game stores it).
    pub best: HashMap<u32, i32>,
}

fn speed_cameras(
    game: Option<Res<Game>>,
    mut state: ResMut<SpeedCameras>,
    cars: Query<(&Car, &Transform), With<crate::drive::Player>>,
) {
    let Some(game) = game else { return };
    if !state.loaded {
        state.loaded = true;
        match game.store.trigger_data() {
            Ok(data) => state.cameras = data.collectables(Collectable::SpeedCamera).copied().collect(),
            Err(e) => warn!("speed cameras: TriggerData not loaded: {e}"),
        }
        state.min_mph = MIN_MPH;
        info!("speed cameras: {} (min {} mph)", state.cameras.len(), state.min_mph);
    }
    let Some((car, transform)) = cars.iter().next() else { return };
    let p = transform.translation.to_array();
    let mph = car.speed.abs() / 0.447_04;
    let state = &mut *state;
    for cam in &state.cameras {
        let inside = cam.contains(p);
        if !inside {
            state.inside.remove(&cam.id);
            continue;
        }
        if !state.inside.insert(cam.id) || mph < state.min_mph {
            continue;
        }
        let centi = (100.0 * mph) as i32;
        let best = state.best.entry(cam.id).or_insert(0);
        *best = (*best).max(centi);
        info!("speed camera {}: {:.2} mph (best {:.2})", cam.id, centi as f32 / 100.0, *best as f32 / 100.0);
    }
}
