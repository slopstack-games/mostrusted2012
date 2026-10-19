//! World streaming as the game does it (docs/decomp.md "World streaming"): the focus zone and its neighbours are
//! loaded, every other track unit is unloaded.
//!
//! - **Focus** (`FUN_00558740`, the stream update; in game mode 2 it passes the camera manager's position
//!   (+0x140) and its matrix row +0x130 to `FUN_005586e0`): the camera, not the car. The loader reads it from
//!   [`StreamFocus`], written each frame by [`send_focus`].
//! - **Stream list** (`FUN_00549fb0`, [`stream_list`]): the zone holding the focus (`FUN_00548ab0`; outside every
//!   zone the last one found, world +0xe0), priority [`CURRENT_PRIORITY`], then **every** neighbour of that zone
//!   with priority `(1500 − d) / 750 + 3 · max(0, dir · to) + (flag bit 2 ? 10 : 9.8)`, d and `to` the distance
//!   and direction (XZ) from the focus to the neighbour's polygon centroid (`FUN_0055f160`), `dir` the focus
//!   vector's XZ direction.
//! - **Streamer** (`FUN_005573f0` → `FUN_0053acf0`, a 32-slot resource streamer built by `FUN_005486c0`): the
//!   world's district and arena bundles (kinds 1 / 2, priorities 500 / 200 in `FUN_00546d70`) and the listed
//!   units are requested; every loaded slot not requested is unloaded first (`FUN_00537c80`), then one missing
//!   entry of highest priority is loaded at a time (`FUN_00533130` → `FUN_00546d70`). Nothing else is resident:
//!   the budget is the slot count (32), not a byte size. Here the loader thread does the same (`Worker::
//!   stream_step`), one unit between frames; the district / arena bundles hold shared resources that our asset
//!   store reaches through its cross-bundle index, so they are not units here.
//! - **Unload**: the loader forgets what only the unit used (models, textures, shared backdrop placements,
//!   groundcover collections) and sends [`crate::loader::Msg::UnitUnloaded`]; [`unloaded`] drops the handles
//!   (so the GPU buffers go) and queues the unit's entities, which [`despawn_streamed`] despawns a batch per frame:
//!   placements, merged statics, props (with the pieces of smashed compounds), colliders; its lights go from
//!   `world_lights::LightSources`.
//!
//! - **Placements** (`FUN_00647e00`): a placement whose target is not resident holds the car there and moves the
//!   focus to the target (message 4 → modes 1 / 3 of `FUN_00558740`, [`PlacementFocus`]) under the loading
//!   screen until it is; a runtime race start does this (`race::place_at_start`, the event card holds meanwhile).
//!
//! `--units N` and `--load-all` load a fixed set once (the behaviour before streaming). Jack spots' parked cars
//! are released by the loader as soon as they arrive (`loader::PARKED_OWNER`): the car holds its meshes and
//! materials and they go with it.
//!
//! Model decisions: until the camera has reached the player's car (the first frames, or a fixed `--camera` far
//! away) the car is the focus ([`CAMERA_REACH`]); a free camera without `--drive` loads every unit unless
//! `--stream` is given (the original has no free camera; its stream focus is the gameplay camera near the car,
//! and our aerial view of `zone_view.rs` needs every unit's main geometry: streamed, a view from 260 m shows bare
//! far water planes in the fog); the district / arena bundles are not loaded as units; a unit's entities are
//! despawned over a few frames ([`DESPAWN_PER_FRAME`]).

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use mw_bundle::ResourceId;

use crate::loader::{StreamFocus, Unloaded};

/// The focus zone's priority (`.data` 0x010c45a4).
pub const CURRENT_PRIORITY: f32 = 100.0;
/// A neighbour's base priority with neighbour flag bit 2 set (`.data` 0x010c45a8) or clear (0x010c45ac).
pub const NEIGHBOUR_PRIORITY_FLAGGED: f32 = 10.0;
pub const NEIGHBOUR_PRIORITY: f32 = 9.8;
/// The weight of the focus direction towards a neighbour (`.data` 0x010c45b0).
pub const FACING_WEIGHT: f32 = 3.0;
/// The distance term `(DISTANCE_REF − d) × DISTANCE_SCALE` (`.rdata` 0x00fab750 = 1500, 0x00fab74c = 1 / 750).
pub const DISTANCE_REF: f32 = 1500.0;
pub const DISTANCE_SCALE: f32 = 1.0 / 750.0;

/// The camera is the focus once it is this close to the player's car (model decision, see the module doc).
pub const CAMERA_REACH: f32 = 150.0;
/// Entities despawned per frame (model decision: spreads a unit's few thousand roots over frames).
pub const DESPAWN_PER_FRAME: usize = 1500;

/// The stream list for a focus ([`stream_list`]).
#[derive(Debug, Clone)]
pub struct StreamList {
    /// Index of the focus zone in `ZoneList::zones`.
    pub zone: usize,
    /// (track unit, priority): the focus zone first.
    pub units: Vec<(u32, f32)>,
}

/// The zone the game streams for `pos` (`FUN_00549fb0`: the zone holding it, else the zone of the last position
/// that had one). Before any (`last` = `None`) the nearest zone (model decision: the game's +0xe0 starts at the
/// position it loads at).
pub fn focus_zone(zones: &mw_formats::ZoneList, pos: Vec3, last: Option<usize>) -> Option<usize> {
    zones.zones.iter().position(|z| z.contains(pos.x, pos.z)).or(last).or_else(|| {
        zones
            .zones
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance(pos.x, pos.z).total_cmp(&b.1.distance(pos.x, pos.z)))
            .map(|(i, _)| i)
    })
}

/// A zone polygon's centroid on the ground plane (`FUN_0055f160`: the mean of its points).
fn centroid(z: &mw_formats::Zone) -> Vec2 {
    let n = z.points.len().max(1) as f32;
    z.points.iter().fold(Vec2::ZERO, |s, p| s + Vec2::from(*p)) / n
}

/// `FUN_00549fb0`: the focus zone, then all of its neighbours with their load priorities.
pub fn stream_list(zones: &mw_formats::ZoneList, pos: Vec3, dir: Vec3, last: Option<usize>) -> Option<StreamList> {
    let zi = focus_zone(zones, pos, last)?;
    let zone = &zones.zones[zi];
    let at = Vec2::new(pos.x, pos.z);
    let facing = Vec2::new(dir.x, dir.z).normalize_or_zero();
    let mut units = vec![(zone.id, CURRENT_PRIORITY)];
    for &(ni, flags) in &zone.neighbours {
        let Some(n) = zones.zones.get(ni) else { continue };
        let to = centroid(n) - at;
        let base = if flags & 2 != 0 { NEIGHBOUR_PRIORITY_FLAGGED } else { NEIGHBOUR_PRIORITY };
        let ahead = to.normalize_or_zero().dot(facing).max(0.0);
        units.push((n.id, (DISTANCE_REF - to.length()) * DISTANCE_SCALE + ahead * FACING_WEIGHT + base));
    }
    Some(StreamList { zone: zi, units })
}

/// Which entities belong to which track unit, shared backdrop placement or groundcover collection, so that an
/// unloaded unit's go with it.
#[derive(Resource, Default)]
pub struct StreamRegistry {
    /// The world is streamed (`Msg::Streaming`).
    pub streaming: bool,
    /// The unit being received (`Msg::UnitBegin` .. `Msg::UnitLoaded`).
    unit: Option<u32>,
    /// The shared backdrop placement being received (`Msg::StreamKey`).
    key: Option<u64>,
    by_unit: HashMap<u32, Vec<Entity>>,
    by_key: HashMap<u64, Vec<Entity>>,
    by_collection: HashMap<ResourceId, Vec<Entity>>,
    /// Entities to despawn ([`despawn_streamed`]).
    pending: VecDeque<Entity>,
    /// Units unloaded / models and textures dropped so far (log).
    pub unloads: usize,
}

impl StreamRegistry {
    pub fn begin_unit(&mut self, unit: Option<u32>) {
        self.unit = unit;
    }

    /// The unit being received.
    pub fn unit(&self) -> Option<u32> {
        self.unit
    }

    pub fn set_key(&mut self, key: Option<u64>) {
        self.key = key;
    }

    /// A placement root, prop or collider spawned for the unit (or shared placement) being received.
    pub fn register(&mut self, e: Entity) {
        if let Some(k) = self.key {
            self.by_key.entry(k).or_default().push(e);
        } else if let Some(u) = self.unit {
            self.by_unit.entry(u).or_default().push(e);
        }
    }

    pub fn register_groundcover(&mut self, collection: ResourceId, e: Entity) {
        self.by_collection.entry(collection).or_default().push(e);
    }
}

/// `Msg::UnitUnloaded`: queue the unit's entities and drop the models, textures and materials the loader no
/// longer uses.
pub fn unloaded(u: Unloaded, reg: &mut StreamRegistry, loaded: &mut crate::Loaded) {
    let mut gone: Vec<Entity> = reg.by_unit.remove(&u.unit).unwrap_or_default();
    for k in &u.backdrops {
        gone.extend(reg.by_key.remove(k).unwrap_or_default());
    }
    for c in &u.groundcover {
        gone.extend(reg.by_collection.remove(c).unwrap_or_default());
    }
    let entities = gone.len();
    reg.pending.extend(gone);
    let mut materials = HashSet::new();
    for m in &u.models {
        if let Some(parts) = loaded.models.remove(m) {
            materials.extend(parts.iter().map(|(_, mat)| mat.id()));
        }
        loaded.shadow_ranges.remove(m);
        loaded.pass_masks.remove(m);
    }
    for t in &u.textures {
        loaded.textures.remove(t);
    }
    // Materials no remaining model uses (a material is cached per look and shared).
    if !materials.is_empty() {
        for parts in loaded.models.values() {
            for (_, mat) in parts {
                materials.remove(&mat.id());
            }
        }
        loaded.materials.retain(|_, h| !materials.contains(&h.id()));
        loaded.mask_materials.retain(|id| !materials.contains(id));
    }
    reg.unloads += 1;
    info!(
        "TRK_UNIT{} gone: {entities} entities queued, {} materials dropped; {} models, {} textures, {} materials kept",
        u.unit,
        materials.len(),
        loaded.models.len(),
        loaded.textures.len(),
        loaded.materials.len()
    );
}

/// Despawn queued entities, [`DESPAWN_PER_FRAME`] a frame (with the pieces of smashed compounds).
pub fn despawn_streamed(
    mut commands: Commands,
    mut reg: ResMut<StreamRegistry>,
    compounds: Query<&crate::props::Compound>,
) {
    let n = reg.pending.len().min(DESPAWN_PER_FRAME);
    for e in reg.pending.drain(..n) {
        if let Ok(c) = compounds.get(e) {
            for p in c.piece_entities() {
                commands.entity(p).try_despawn();
            }
        }
        commands.entity(e).try_despawn();
    }
}

/// The streaming stall's radius (`DAT_00faba94`, decomp.md "Streaming stall").
pub const STALL_RADIUS: f32 = 25.0;

/// The streaming stall (`FUN_006a2200` from `FUN_004766a0`): the simulation pauses (pause reason 3) while the
/// player's car's surroundings are not Resident(car, 25 m), e.g. when it outruns the loader. Virtual time is paused
/// only when nothing else paused it, and resumed only by this system.
pub fn streaming_stall(
    reg: Res<StreamRegistry>,
    loaded: Res<crate::Loaded>,
    residency: Res<crate::frontend::Residency>,
    player: Query<&Transform, (With<crate::drive::Player>, With<crate::drive::Car>)>,
    mut time: ResMut<Time<Virtual>>,
    mut stalled: Local<bool>,
) {
    let car = player.iter().next().map(|t| t.translation);
    let stall = reg.streaming && loaded.done && car.is_some_and(|c| !residency.resident(c, STALL_RADIUS));
    if stall && !*stalled && !time.is_paused() {
        time.pause();
        *stalled = true;
        info!("streaming stall: the car's surroundings are not resident, simulation paused");
    } else if !stall && *stalled {
        time.unpause();
        *stalled = false;
        info!("streaming stall over");
    }
}

/// A placement's target while it waits to be resident (`FUN_00647e00` sends message 4: the world's stream update
/// `FUN_00558740` then streams around a stored position, modes 1 / 3, instead of the camera; message 5 goes back).
#[derive(Resource, Default)]
pub struct PlacementFocus(pub Option<Vec3>);

/// Hand the loader the focus: the camera's position and view direction ([`CAMERA_REACH`] for the car), or a
/// waiting placement's target ([`PlacementFocus`]).
pub fn send_focus(
    focus: Option<Res<StreamFocus>>,
    reg: Res<StreamRegistry>,
    placement: Res<PlacementFocus>,
    game: Res<crate::Game>,
    camera: Query<&Transform, crate::MainCamera>,
    player: Query<&Transform, (With<crate::drive::Player>, With<crate::drive::Car>)>,
) {
    let Some(focus) = focus else { return };
    if !reg.streaming {
        return;
    }
    if let Some(p) = placement.0 {
        focus.set(p, Vec3::ZERO);
        return;
    }
    let Some(cam) = camera.iter().next() else { return };
    let car = player.iter().next().map(|t| t.translation);
    let at = match car {
        Some(c) if cam.translation.distance(c) > CAMERA_REACH => c,
        Some(_) => cam.translation,
        // In a drive the camera is not on the car yet: keep the spawn point.
        None if matches!(game.request, crate::loader::Request::Drive { .. }) => return,
        None => cam.translation,
    };
    focus.set(at, *cam.forward());
}
