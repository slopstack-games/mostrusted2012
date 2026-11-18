//! What the logic's engine requests do to the runtime's own systems, and the fields the runtime feeds back (the
//! `Request`s / fields documented in `mw_logic::natives`; logic.rs routes them here).
//!
//! Carried out ([`Effect`]):
//! - `ClearAllChevrons` (`FUN_006465a0`): the route markers of the race are dropped (`chevron::RouteMarker`);
//! - `ClearTargetRoute` / `SetRouteFromPlayerToEvent` (`FUN_0067fd00`): `gps::Gps` cleared / routed to the event's
//!   start (`gps::SetDestination`, as the map's SET DESTINATION);
//! - `NitrousAward` / `NitrousReset` / `NitrousInfinite` (`FUN_00658ad0` / `FUN_00658c90` / the byte +0xc0): the local
//!   car's tank (`mw_handling::nitrous::Nitrous`); infinite holds the tank at 100 (decomp.md, nitrous state +0xc0);
//! - `SirenState` / `EngineState` (`FUN_00844300` / `FUN_008444a0`): the local car's siren (`police_fx::FlashCar`) and
//!   engine (`ignition::EngineStart`);
//! - `PlaceVehicle` / `ResetAlongRoad` / `TeleportAlongRoute` / `SpawnVehicle` of the local car: a [`Placement`]
//!   carried out as `FUN_00647e00` does it ([`place_vehicles`]): mph capped at 100 (`FUN_0064b290`, `.rdata`
//!   0x00fb69e0), held at the target with the streaming focus there (message 4, `streaming::PlacementFocus`) until
//!   it is Resident within max(200 m, 3 s of travel) (decomp.md "Waiting for streaming"), then put on the ground
//!   moving at the speed along the direction (`Car::place_moving`, `FUN_008f8190`).
//!
//! Fed each frame ([`Feed`]): the local player's vehicle record (`SimFields.world.vehicles`: position, direction,
//! velocity, speed in mph, flags), the player slot's loaded / alive / `VehicleList` id, the rollout record's selected
//! vehicle and colour (`CareerFields`), the camera's position and direction and the car's top speed
//! (`SimFields.queries`), the road natives' state (`SimFields.road`, [`Feed::fill_road`]).
//!
//! Model decisions: the runtime has one car, the logic's local slot's vehicle id ([`LOCAL_VEHICLE`]) names it; a
//! request for another slot or vehicle does nothing; the award's slot multiplier (+0xc8) is read as 1 (offline).

use avian3d::prelude::{ColliderOf, Collisions, LinearVelocity};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::HashMap;

use avian3d::prelude::{AngularVelocity, SpatialQuery, SpatialQueryFilter};
use mw_logic::natives::LogicState;
use mw_logic::natives::lua_requests::ContactBody;
use mw_logic::natives::lua_road::Shared;
use mw_logic::natives::lua_world::WorldVehicle;

use crate::drive::{Car, Player};

/// The vehicle id of the runtime's one car in the logic's tables (the original's vehicle slot of the local player).
pub const LOCAL_VEHICLE: i32 = 0;
/// m/s to mph.
const MPH: f32 = 2.236_94;
/// The tank's capacity (`.rdata` 0x00fb77e4).
const TANK: f32 = 100.0;
/// mph to m/s (`.bss` 0x0118a010, `FUN_00596050`).
const MS_PER_MPH: f32 = 0.447_04;
/// `FUN_0064b290` caps a placement's speed at 100 mph (`.rdata` 0x00fb69e0).
const PLACE_MAX_MPH: f32 = 100.0;
/// `FUN_00647e00`: the placement waits for max(200 m, the distance of this many seconds at its speed).
const PLACE_LOOKAHEAD: f32 = 3.0;

/// A request the runtime carries out on its systems.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    ClearChevrons,
    ClearRoute,
    /// The event's start position.
    RouteToEvent {
        event: u32,
        position: Vec3,
    },
    NitrousAward {
        amount: f32,
    },
    NitrousInfinite(bool),
    NitrousReset,
    Siren(bool),
    Engine(bool),
    /// `FUN_00647e00` on the local car.
    Place(Placement),
    /// `TeleportToDistanceFromFinish`: `FUN_00665ae0`'s pose `distance` m from the start along the race route,
    /// then placed (mph 0, reason 2).
    PlaceAlongRoute {
        distance: f32,
    },
}

/// A placement of the local car (`FUN_00647e00`'s arguments the runtime uses).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: Vec3,
    pub direction: Vec3,
    pub mph: f32,
    /// G+0x25690 +0x920 + vehicle × 4 (3 `TeleportVehicle`, 4 `ResetOnRoad`, 2 `TeleportToNamedRoad` /
    /// `TeleportToDistanceFromFinish`, 8 `ResetAlongRoad`; 0 here for `SpawnVehicle`, which does not go through
    /// `FUN_00647e00`): logged.
    pub reason: i32,
}

/// The placement waiting for its target to be resident ([`place_vehicles`]); a new one replaces it, as the
/// reset service's record of the vehicle is overwritten.
#[derive(Resource, Default)]
pub struct PendingPlacement {
    placement: Option<Placement>,
    /// The streaming focus was moved to the target (message 4 sent: message 5 goes back).
    focused: bool,
}

/// `SetInfiniteNitrous`: the local tank is held at 100 (nitrous state +0xc0).
#[derive(Resource, Default)]
pub struct InfiniteNitrous(pub bool);

/// The player's car as the effects change it: its record, engine start and siren / flash state.
type PlayerCarMut = (
    &'static mut Car,
    Option<&'static mut crate::ignition::EngineStart>,
    Option<&'static mut crate::police_fx::FlashCar>,
);

#[derive(SystemParam)]
pub struct Effects<'w, 's> {
    commands: Commands<'w, 's>,
    markers: Query<'w, 's, Entity, With<crate::chevron::RouteMarker>>,
    gps: Option<ResMut<'w, crate::gps::Gps>>,
    destination: MessageWriter<'w, crate::gps::SetDestination>,
    cars: Query<'w, 's, PlayerCarMut, With<Player>>,
    infinite: Option<ResMut<'w, InfiniteNitrous>>,
    placement: Option<ResMut<'w, PendingPlacement>>,
    race: Option<Res<'w, crate::race::RaceState>>,
}

impl Effects<'_, '_> {
    pub fn apply(&mut self, e: &Effect) {
        match e {
            Effect::ClearChevrons => {
                for m in &self.markers {
                    self.commands.entity(m).despawn();
                }
            }
            Effect::ClearRoute => {
                if let Some(g) = self.gps.as_deref_mut() {
                    g.clear();
                }
            }
            Effect::RouteToEvent { event, position } => {
                self.destination.write(crate::gps::SetDestination { position: *position, event: Some(*event) });
            }
            Effect::NitrousInfinite(on) => {
                if let Some(i) = self.infinite.as_deref_mut() {
                    i.0 = *on;
                }
                // `FUN_00658c90` first: the record is reset.
                for (mut car, ..) in &mut self.cars {
                    car.nitrous = Default::default();
                }
            }
            Effect::NitrousReset => {
                for (mut car, ..) in &mut self.cars {
                    car.nitrous = Default::default();
                }
            }
            Effect::NitrousAward { amount } => {
                // `FUN_00658ad0`: amount x 0.01 x 100 x the slot's multiplier, clamped to 0..100 by `Nitrous::add`.
                for (mut car, ..) in &mut self.cars {
                    car.nitrous.add(*amount);
                }
            }
            Effect::Siren(on) => {
                for (.., flash) in &mut self.cars {
                    if let Some(mut f) = flash {
                        f.set_siren_state(*on);
                    }
                }
            }
            Effect::Engine(on) => {
                for (_, engine, _) in &mut self.cars {
                    if let Some(mut e) = engine {
                        e.set_running(*on);
                    }
                }
            }
            Effect::Place(p) => {
                if let Some(pending) = self.placement.as_deref_mut() {
                    pending.placement = Some(*p);
                }
            }
            Effect::PlaceAlongRoute { distance } => {
                let Some(race) = self.race.as_deref() else { return };
                let (position, direction) = route_pose(race, *distance);
                if let Some(pending) = self.placement.as_deref_mut() {
                    pending.placement = Some(Placement { position, direction, mph: 0.0, reason: 2 });
                }
            }
        }
    }
}

/// The pose `s` m from the start along the race route (the runtime's route polylines, `race::RouteSegment`).
/// Model decision: `FUN_00665ae0` / `FUN_00662670` (the original's pose and direction on the route) are not read;
/// the polyline's point and its piece's direction stand in.
fn route_pose(race: &crate::race::RaceState, s: f32) -> (Vec3, Vec3) {
    let k = race.entries.partition_point(|e| e.distance <= s).clamp(1, race.entries.len().max(2) - 1);
    match (race.entries.get(k - 1), race.segments.get(k)) {
        (Some(e), Some(seg)) => seg.at(s - e.distance),
        _ => (race.player_start.translation, race.player_start.rotation * Vec3::Z),
    }
}

/// `FUN_00647e00` → `FUN_0064b290` → `FUN_008f8190` for the local car: while the target is not Resident within
/// max(200 m, mph × 0.44704 × 3 s) (or the ground under it is not in) the car is held there at rest and the
/// streaming focus moves to it (message 4); then it is put on the ground facing the direction, moving at the
/// capped speed (`FUN_00596050`: velocity = mph × 0.44704 × direction), the wheels and gearbox placed for it
/// (`Car::place_moving`), and the focus goes back (message 5). On the ground as `cutscene.rs` places its
/// teleports (y = the ground: the car origin is at road level). Model decisions: the yaw of the direction only
/// (no pitch); `keep` (+0x78) and the flags (+0x84) are not modelled.
#[allow(clippy::type_complexity)]
pub fn place_vehicles(
    mut pending: ResMut<PendingPlacement>,
    residency: Option<Res<crate::frontend::Residency>>,
    mut focus: ResMut<crate::streaming::PlacementFocus>,
    spatial: SpatialQuery,
    mut cars: Query<(Entity, &mut Car, &mut Transform, &mut LinearVelocity, &mut AngularVelocity), With<Player>>,
) {
    let Some(p) = pending.placement else { return };
    let Ok((entity, mut car, mut t, mut lin, mut ang)) = cars.single_mut() else { return };
    let mph = p.mph.min(PLACE_MAX_MPH);
    let speed = mph * MS_PER_MPH;
    let radius = crate::frontend::PLACE_RADIUS.max(speed * PLACE_LOOKAHEAD);
    let from = p.position + Vec3::Y * 5.0;
    let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider])
        .with_mask(crate::props::GameLayer::World);
    let ground = spatial.cast_ray(from, Dir3::NEG_Y, 40.0, true, &filter).map(|h| from.y - h.distance);
    let resident = residency.is_none_or(|r| r.resident(p.position, radius));
    let yaw = Quat::from_rotation_y(p.direction.x.atan2(p.direction.z));
    let (Some(ground), true) = (ground, resident) else {
        if !pending.focused {
            info!("logic: placement at {:.1?} waits for the world to stream in", p.position);
        }
        pending.focused = true;
        focus.0 = Some(p.position);
        t.translation = p.position + Vec3::Y * 0.5;
        t.rotation = yaw;
        lin.0 = Vec3::ZERO;
        ang.0 = Vec3::ZERO;
        return;
    };
    if std::mem::take(&mut pending.focused) {
        focus.0 = None;
    }
    pending.placement = None;
    t.translation = p.position.with_y(ground + 0.02);
    t.rotation = yaw;
    lin.0 = yaw * Vec3::Z * speed;
    ang.0 = Vec3::ZERO;
    car.place_moving(speed);
    info!("logic: car placed at {:.1?} at {mph:.0} mph (reason {})", t.translation, p.reason);
}

/// The road natives' state that does not change while the world is loaded (`RoadFields`): the road network
/// (`nitrous::Roads`), the road names' texts, the loading-screen layouts.
#[derive(Resource, Default)]
pub struct RoadFeed {
    roads: Shared<mw_formats::RoadData>,
    /// The text database's strings for the roads' name ids (`TeleportToNamedRoad` reads only those).
    texts: HashMap<u32, String>,
    /// `UIConfig.eee932fa` (+0x80): the general loading layouts, the ring `FUN_00799a60` takes from.
    layouts: Vec<u32>,
}

/// Fills [`RoadFeed`] once the road network is loaded.
pub fn load_road_feed(
    game: Res<crate::Game>,
    roads: Option<Res<crate::nitrous::Roads>>,
    mut feed: ResMut<RoadFeed>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(roads) = roads.as_ref().and_then(|r| r.0.clone()) else { return };
    *done = true;
    let lang = game.store.language(mw_assets::DEFAULT_LANGUAGE).ok();
    feed.texts =
        roads.roads.iter().filter_map(|r| Some((r.name, lang.as_ref()?.strings.get(&r.name)?.clone()))).collect();
    feed.roads = Shared(Some(roads));
    feed.layouts = game.store.loading_config().map(|c| c.layouts).unwrap_or_default();
    info!(
        "logic: road network for the natives ({} road names, {} loading layouts)",
        feed.texts.len(),
        feed.layouts.len()
    );
}

/// The infinite tank is held full.
pub fn hold_infinite(infinite: Option<Res<InfiniteNitrous>>, mut cars: Query<&mut Car, With<Player>>) {
    if infinite.is_some_and(|i| i.0) {
        for mut c in &mut cars {
            c.nitrous.tank = TANK;
        }
    }
}

#[derive(SystemParam)]
pub struct Feed<'w, 's> {
    cars: Query<
        'w,
        's,
        (&'static Transform, &'static LinearVelocity, &'static Car, Option<&'static crate::nitrous::NitrousFeed>),
        With<Player>,
    >,
    camera: Query<'w, 's, &'static GlobalTransform, crate::MainCamera>,
    garage: Option<Res<'w, crate::garage::Garage>>,
    /// `HasInputBeenDetected`'s sources: the keyboard, the pads and the player's control block.
    keys: Res<'w, ButtonInput<KeyCode>>,
    pads: Query<'w, 's, &'static Gamepad>,
    input: Option<Res<'w, crate::drive::CarInput>>,
    /// `IsHittingRaceCar`: the player's body collider against the AI racers' bodies.
    collisions: Collisions<'w>,
    collider_of: Query<'w, 's, &'static ColliderOf>,
    racers: Query<'w, 's, &'static crate::race::Racer>,
    /// The road natives (`fill_road`).
    road: Option<Res<'w, RoadFeed>>,
    residency: Option<Res<'w, crate::frontend::Residency>>,
    race: Option<Res<'w, crate::race::RaceState>>,
    progress: Query<'w, 's, &'static crate::race::RacerProgress, With<Player>>,
    loaded: Res<'w, crate::Loaded>,
}

/// The logic's vehicle id of the AI racer in grid slot `slot` (model decision: the runtime has one logic vehicle,
/// [`LOCAL_VEHICLE`]; the racers follow it).
pub fn racer_vehicle(slot: usize) -> i32 {
    LOCAL_VEHICLE + 1 + slot as i32
}

impl Feed<'_, '_> {
    /// Fills the runtime-fed fields of the logic's state for this frame.
    pub fn fill(&self, st: &mut LogicState) {
        if let Ok(gt) = self.camera.single() {
            let q = &mut st.sim.queries;
            q.camera_position = gt.translation().to_array();
            q.camera_direction = gt.forward().as_vec3().to_array();
            q.gameplay_camera_position = q.camera_position;
        }
        let bundle = self.garage.as_ref().and_then(|g| g.current.as_ref());
        let vehicle_id = bundle.and_then(|c| crate::frontend::vehicle_id(&c.bundle)).unwrap_or(0);
        let colour = bundle.map_or(0, |c| c.paint_index as i32);
        self.fill_requests(st);
        self.fill_road(st);
        let Some(slot) = st.players.local else { return };
        let Ok((xf, vel, car, feed)) = self.cars.single() else {
            st.sim.world.vehicles.remove(&LOCAL_VEHICLE);
            if let Some(Some(p)) = st.players.slots.get_mut(slot) {
                p.loaded = false;
                p.vehicle_alive = false;
            }
            return;
        };
        let id = st.players.slots.get(slot).and_then(|p| p.as_ref()).map_or(LOCAL_VEHICLE, |p| p.vehicle);
        let forward = xf.rotation * Vec3::Z;
        st.sim.world.vehicles.insert(
            id,
            WorldVehicle {
                exists: true,
                loaded: true,
                physical: true,
                cop: false,
                car_id: vehicle_id as i32,
                position: xf.translation.to_array(),
                direction: forward.to_array(),
                velocity: vel.0.to_array(),
                forward: forward.to_array(),
                speed_mph: car.speed.abs() * MPH,
            },
        );
        if let Some(f) = feed {
            st.sim.queries.top_speed_mph.insert(id, f.top_mph);
        }
        if let Some(Some(p)) = st.players.slots.get_mut(slot) {
            p.loaded = true;
            p.vehicle_alive = true;
            p.vehicle_id = vehicle_id;
        }
        if let (Some(v), Some(c)) = (st.career.selected_vehicle.get_mut(slot), vehicle_id.try_into().ok()) {
            *v = c;
        }
        if let Some(c) = st.career.selected_colour.get_mut(slot) {
            *c = colour;
        }
    }

    /// `RequestFields` (`natives/lua_requests.rs`): the controller's detected byte (any key or pad button held), the
    /// local vehicle's input differing from a neutral one (`FUN_00670fc0`: throttle, brake, steering, handbrake or
    /// nitrous), and the player's contacts with AI racers' bodies (avian's contact graph; both sides of kind 7, a
    /// vehicle body). Body ids are the vehicle ids (model decision: the bodies' table is not modelled).
    fn fill_requests(&self, st: &mut LogicState) {
        let f = &mut st.sim.requests;
        f.controller_input =
            self.keys.get_pressed().next().is_some() || self.pads.iter().any(|p| p.get_pressed().next().is_some());
        f.vehicle_input = self
            .input
            .as_ref()
            .is_some_and(|i| i.throttle != 0.0 || i.brake != 0.0 || i.steer != 0.0 || i.handbrake || i.nitrous);
        f.vehicle_contacts.clear();
        f.vehicle_body_id.clear();
        let Ok((_, _, car, _)) = self.cars.single() else { return };
        f.vehicle_body_id.insert(LOCAL_VEHICLE, LOCAL_VEHICLE as i16);
        let body = |id: i32| ContactBody { id: id as i16, kind: 7 };
        let mut list = Vec::new();
        for pair in self.collisions.collisions_with(car.body_collider) {
            if !pair.is_touching() {
                continue;
            }
            let other = if pair.collider1 == car.body_collider { pair.collider2 } else { pair.collider1 };
            let Ok(c) = self.collider_of.get(other) else { continue };
            let Some(slot) = self.racers.get(c.body).ok().and_then(|r| r.grid_slot) else { continue };
            list.push([body(LOCAL_VEHICLE), body(racer_vehicle(slot))]);
        }
        f.vehicle_contacts.insert(LOCAL_VEHICLE, list);
        for racer in &self.racers {
            if let Some(slot) = racer.grid_slot {
                f.vehicle_body_id.insert(racer_vehicle(slot), racer_vehicle(slot) as i16);
            }
        }
    }

    /// `RoadFields` (`natives/lua_road.rs`) and the race manager's running state:
    /// - the road network and the road names' texts ([`RoadFeed`]);
    /// - the zones and the track units streamed in (`frontend::Residency`; once every unit was sent all count as
    ///   loaded, as `Residency::resident` has it), and the camera's world streamed (`FUN_005470e0`; model decision:
    ///   the loader's first world load is done, `Loaded::done`);
    /// - the local slot's route length (+0xdf0[slot], `RaceState::total_distance`) and the route's direction at its
    ///   car ([`route_pose`] at its distance from the start) while a race is loaded (`race_active`, the race
    ///   manager's vtable +0xc; model decision: any loaded race, every phase);
    /// - the loading-screen ring (`loading_layouts`; model decision: LSM +0x48 (rotate) set with the ring: its writer
    ///   was not found, only the constructor `FUN_00790a90` clearing it).
    ///
    /// The HUD's named message queues are never full (`message_queues` empty, `hud_queue_full` false) and no
    /// message serial is taken here: the runtime has no HUD message queue and posts no feedback messages.
    pub fn fill_road(&self, st: &mut LogicState) {
        st.sim.world.camera_world_streamed = self.loaded.done;
        let f = &mut st.sim.road;
        if let Some(feed) = &self.road {
            if f.roads != feed.roads {
                f.roads = feed.roads.clone();
                f.texts = feed.texts.clone();
            }
            if f.loading_layouts != feed.layouts {
                f.loading_layouts = feed.layouts.clone();
                f.loading_rotation = !f.loading_layouts.is_empty();
                f.loading_layout_index = 0;
            }
        }
        if let Some(r) = &self.residency {
            let zones = Shared(r.zones().cloned());
            if r.is_changed() || f.zones != zones {
                f.loaded_units = match (r.all_sent, zones.get()) {
                    (true, Some(z)) => z.zones.iter().map(|z| z.id).collect(),
                    _ => r.loaded_units().clone(),
                };
                f.zones = zones;
            }
        }
        f.route_length.clear();
        f.route_forward.clear();
        let race = self.race.as_deref().filter(|r| r.entries.len() > 1);
        st.sim.feedback.race_active = race.is_some();
        if let (Some(race), Some(slot)) = (race, st.players.local) {
            let slot = slot as i32;
            let total = race.total_distance();
            f.route_length.insert(slot, total);
            if let Ok(p) = self.progress.single() {
                f.route_forward.insert(slot, route_pose(race, total - p.to_finish).1.to_array());
            }
        }
    }
}
