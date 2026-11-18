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
//!   engine (`ignition::EngineStart`).
//!
//! Fed each frame ([`Feed`]): the local player's vehicle record (`SimFields.world.vehicles`: position, direction,
//! velocity, speed in mph, flags), the player slot's loaded / alive / `VehicleList` id, the rollout record's selected
//! vehicle and colour (`CareerFields`), the camera's position and direction and the car's top speed
//! (`SimFields.queries`).
//!
//! Model decisions: the runtime has one car, the logic's local slot's vehicle id ([`LOCAL_VEHICLE`]) names it; a
//! request for another slot or vehicle does nothing; the award's slot multiplier (+0xc8) is read as 1 (offline).

use avian3d::prelude::{ColliderOf, Collisions, LinearVelocity};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mw_logic::natives::LogicState;
use mw_logic::natives::lua_requests::ContactBody;
use mw_logic::natives::lua_world::WorldVehicle;

use crate::drive::{Car, Player};

/// The vehicle id of the runtime's one car in the logic's tables (the original's vehicle slot of the local player).
pub const LOCAL_VEHICLE: i32 = 0;
/// m/s to mph.
const MPH: f32 = 2.236_94;
/// The tank's capacity (`.rdata` 0x00fb77e4).
const TANK: f32 = 100.0;

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
}

/// `SetInfiniteNitrous`: the local tank is held at 100 (nitrous state +0xc0).
#[derive(Resource, Default)]
pub struct InfiniteNitrous(pub bool);

#[derive(SystemParam)]
pub struct Effects<'w, 's> {
    commands: Commands<'w, 's>,
    markers: Query<'w, 's, Entity, With<crate::chevron::RouteMarker>>,
    gps: Option<ResMut<'w, crate::gps::Gps>>,
    destination: MessageWriter<'w, crate::gps::SetDestination>,
    cars: Query<
        'w,
        's,
        (
            &'static mut Car,
            Option<&'static mut crate::ignition::EngineStart>,
            Option<&'static mut crate::police_fx::FlashCar>,
        ),
        With<Player>,
    >,
    infinite: Option<ResMut<'w, InfiniteNitrous>>,
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
        }
    }
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
    camera: Query<'w, 's, &'static GlobalTransform, With<Camera3d>>,
    garage: Option<Res<'w, crate::garage::Garage>>,
    /// `HasInputBeenDetected`'s sources: the keyboard, the pads and the player's control block.
    keys: Res<'w, ButtonInput<KeyCode>>,
    pads: Query<'w, 's, &'static Gamepad>,
    input: Option<Res<'w, crate::drive::CarInput>>,
    /// `IsHittingRaceCar`: the player's body collider against the AI racers' bodies.
    collisions: Collisions<'w>,
    collider_of: Query<'w, 's, &'static ColliderOf>,
    racers: Query<'w, 's, &'static crate::race::Racer>,
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
}
