//! What the nitrous refill sees of the world (docs/decomp.md "Nitrous refill"). The tank itself is
//! `mw_handling::nitrous::Nitrous`, run per physics tick in `drive::car_physics`; this module fills the player
//! car's [`NitrousFeed`] just before it:
//! - the road position on the gameplay road network (`RoadData`, `FUN_00666200`: refreshed every 24th tick, one
//!   player slot per game-logic tick) and the oncoming-lane test (`FUN_0065eec0`) and shortcut flag;
//! - the near-miss query (traffic cars within `NEAR_MISS_RADIUS`, `traffic::TrafficSystem::near_miss_query`)
//!   and the cancel while touching a traffic car (`FUN_006564c0`);
//! - the slipstream amount behind another race car (`FUN_0063c790`);
//! - touching another race car (the HittingAnotherCompetitor refill, `FUN_00658e80`).
//!
//! Not fed (no such events in this runtime): takedowns (FatalHit), crash escapes (a police car crashing near
//! the player), gas stations (a full tank, `FUN_00748ff0`) and smashed gates (+10, `FUN_0074cf40`).
//!
//! `MW_NITROUS_LOG=1` prints the tank and each event's refill once a second, one-off awards at once.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_formats::{RoadData, RoadPosition};
use mw_handling::nitrous::{EVENTS, Lane, NEAR_MISS_RADIUS};

use crate::drive::{Car, Player};

pub struct NitrousPlugin;

impl Plugin for NitrousPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Roads>()
            .init_resource::<NitrousLog>()
            .add_systems(Startup, load_roads)
            .add_systems(FixedUpdate, feed_nitrous.before(crate::drive::car_physics))
            .add_systems(FixedUpdate, log_nitrous.after(crate::drive::car_physics));
    }
}

/// The gameplay road network (`HAWAII/GAMELOGIC/ROADDATA.BNDL`).
#[derive(Resource, Default)]
pub struct Roads(pub Option<Arc<RoadData>>);

fn load_roads(game: Res<crate::Game>, mut roads: ResMut<Roads>) {
    match game.store.road_data() {
        Ok(r) => {
            info!("road data: {} roads", r.roads.len());
            roads.0 = Some(Arc::new(r));
        }
        Err(e) => warn!("no road data: {e}"),
    }
}

/// `FUN_00668650` refreshes one of 24 player slots per game-logic tick.
const ROAD_REFRESH_TICKS: u32 = 24;

/// Slipstream (`FUN_0063c790`): both cars faster than 50 mph (`.rdata` 0x00fb6330 × `.bss` 0x0118a010), 1 to 80 m
/// apart (0x00fb6260, 0x00fb6340), the other ahead within acos 0.95 of our travel (0x00fb6350); the amount is
/// (the best alignment of the other's travel with the line to it − 0.95) / 0.05 (0x00fb6360), clamped to 0..1.
const SLIP_MIN_SPEED: f32 = 50.0 * 0.44704;
const SLIP_RANGE: (f32, f32) = (1.0, 80.0);
const SLIP_COS: f32 = 0.95;
const SLIP_SPAN: f32 = 0.05;
/// The line-of-sight ray between the two cars is cast this high above their origins (model decision: the game's
/// ray end points, from objects at vehicle +0xac, were not traced).
const SLIP_RAY_HEIGHT: f32 = 0.5;

/// The world as the player car's nitrous refill sees it this tick (`mw_handling::nitrous::World`'s outside part).
#[derive(Component, Debug, Clone, Default)]
pub struct NitrousFeed {
    /// The car's top speed (vehicle list +0x3C, mph).
    pub top_mph: f32,
    pub road: Option<RoadPosition>,
    road_tick: u32,
    pub lane: Lane,
    pub shortcut: bool,
    pub slipstream: f32,
    pub touching_competitor: bool,
    pub near_cars: Vec<(u32, bool)>,
    pub near_miss_cancel: bool,
}

impl NitrousFeed {
    pub fn new(top_mph: f32) -> Self {
        Self { top_mph, ..Default::default() }
    }
}

#[allow(clippy::type_complexity)]
fn feed_nitrous(
    roads: Res<Roads>,
    traffic: Option<Res<crate::traffic::TrafficSystem>>,
    collisions: Collisions,
    collider_of: Query<&ColliderOf>,
    traffic_parts: Query<(), Or<(With<crate::traffic::TrafficBody>, With<crate::traffic::TrafficCar>)>>,
    racers: Query<(), With<crate::ai::AiDriver>>,
    spatial: SpatialQuery,
    mut players: Query<(Entity, &Car, &Transform, &LinearVelocity, &mut NitrousFeed), With<Player>>,
    cars: Query<(Entity, &Transform, &LinearVelocity), With<Car>>,
) {
    for (entity, car, transform, velocity, mut feed) in &mut players {
        let pos = transform.translation;
        let v = velocity.0;
        let forward = transform.rotation * Vec3::Z;
        // Road position and the oncoming test.
        let feed = &mut *feed;
        if let Some(r) = &roads.0 {
            if feed.road_tick.is_multiple_of(ROAD_REFRESH_TICKS) {
                feed.road = r.locate(pos.to_array());
            }
            feed.lane = match feed.road {
                None => Lane::OffRoad,
                Some(at) => Lane::OnRoad { oncoming: r.oncoming(at, pos.to_array(), forward.to_array()) },
            };
            feed.shortcut =
                feed.road.and_then(|at| r.road(at)).is_some_and(|road| road.flags & mw_formats::Road::SHORTCUT != 0);
        }
        feed.road_tick = feed.road_tick.wrapping_add(1);

        // Contacts: traffic (cancels the near misses) and other race cars.
        let (mut traffic_touch, mut racer_touch) = (false, false);
        for collider in [car.body_collider, car.bumper_collider] {
            for pair in collisions.collisions_with(collider).filter(|p| p.is_touching()) {
                let other = if pair.collider1 == collider { pair.collider2 } else { pair.collider1 };
                let body = collider_of.get(other).map_or(other, |c| c.body);
                if body == entity {
                    continue;
                }
                traffic_touch |= traffic_parts.contains(other) || traffic_parts.contains(body);
                racer_touch |= racers.contains(body);
            }
        }
        feed.touching_competitor = racer_touch;
        feed.near_miss_cancel = traffic_touch;
        feed.near_cars.clear();
        if !traffic_touch && let Some(t) = &traffic {
            feed.near_cars
                .extend(t.near_miss_query(pos, NEAR_MISS_RADIUS).into_iter().map(|(id, f)| (id, f.dot(v) >= 0.0)));
        }

        // Slipstream behind another race car.
        let mut best = 0.0f32;
        if v.length() > SLIP_MIN_SPEED {
            let dir = v.normalize();
            for (other, t, ov) in &cars {
                if other == entity || ov.length() <= SLIP_MIN_SPEED {
                    continue;
                }
                let d = t.translation - pos;
                let len = d.length();
                if !(SLIP_RANGE.0 < len && len < SLIP_RANGE.1) {
                    continue;
                }
                let dhat = d / len;
                if dhat.dot(dir) <= SLIP_COS {
                    continue;
                }
                let up = Vec3::Y * SLIP_RAY_HEIGHT;
                let world = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
                if let Ok(ray) = Dir3::new(d) {
                    if spatial.cast_ray(pos + up, ray, len, true, &world).is_some() {
                        continue;
                    }
                }
                best = best.max(dhat.dot(ov.normalize()));
            }
        }
        feed.slipstream = ((best.min(1.0) - SLIP_COS) / SLIP_SPAN).clamp(0.0, 1.0);
    }
}

/// `MW_NITROUS_LOG`: per-second refill totals per event.
#[derive(Resource, Default)]
struct NitrousLog {
    on: Option<bool>,
    time: f32,
    next: f32,
    sums: [f32; 13],
}

fn log_nitrous(
    time: Res<Time>,
    mut log: ResMut<NitrousLog>,
    cars: Query<(&Car, &LinearVelocity, Option<&NitrousFeed>), With<Player>>,
) {
    let on = *log.on.get_or_insert_with(|| std::env::var_os("MW_NITROUS_LOG").is_some());
    if !on {
        return;
    }
    let Ok((car, velocity, feed)) = cars.single() else { return };
    log.time += time.delta_secs();
    let t = log.time;
    let n = &car.nitrous;
    for (k, &a) in n.added.iter().enumerate() {
        log.sums[k] += a;
        if a > 0.0 && k >= 8 {
            let extra = match k {
                8 => format!(" ({} cars)", n.near_same.len()),
                9 => format!(" ({} cars)", n.near_oncoming.len()),
                _ => String::new(),
            };
            info!("NITROUS {t:.2} s: {} +{a:.2}{extra}, tank {:.1}", EVENTS[k], n.tank);
        }
    }
    if t < log.next {
        return;
    }
    log.next = t + 1.0;
    let parts: Vec<String> = (0..8)
        .filter(|&k| log.sums[k] > 0.0 || n.active[k])
        .map(|k| format!("{} +{:.2}", EVENTS[k], log.sums[k]))
        .collect();
    let f = feed.cloned().unwrap_or_default();
    info!(
        "NITROUS {t:.2} s: {:.0} mph, tank {:.1}{} [{}] lane {:?} slip {:.2} near {}{}",
        velocity.length() / crate::drive::MPH,
        n.tank,
        if n.burning { " burning" } else { "" },
        parts.join(", "),
        f.lane,
        f.slipstream,
        f.near_cars.len(),
        if f.near_miss_cancel { " (touching traffic)" } else { "" },
    );
    log.sums = [0.0; 13];
}
