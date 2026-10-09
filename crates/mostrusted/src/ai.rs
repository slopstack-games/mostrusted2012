//! AI racers: opponent cars that run the player's physics (`drive.rs`, every `Car` steps from its own
//! [`CarControls`]) and are driven along a route by an AI driver.
//!
//! A race hands in a [`RaceSetup`] (the route as a polyline and the start-grid transforms); without one, `--ai`
//! builds a test route along the traffic lanes from the player's spawn point (`route_from_lanes`) and places the
//! opponents on it ahead of and beside the player. Opponent cars are loaded by the loader thread like the
//! player's (`CarRequest::opponent`) and built by [`crate::drive::build_car`].

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::drive::{Car, CarBuild, CarControls, CarParts, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests};

const MPH: f32 = 0.44704;

pub struct AiPlugin {
    /// Opponent car bundles (`--ai`).
    pub cars: Vec<String>,
    /// `--test ai`: drive the player car with the same driver, log both, exit after `seconds`.
    pub test: Option<f32>,
}

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Opponents {
            bundles: self.cars.clone(),
            requested: false,
            arrived: Vec::new(),
            spawned: vec![false; self.cars.len()],
        })
        .add_systems(Update, (setup_race, request_opponents, spawn_opponents, log_ai).chain())
        .add_systems(FixedUpdate, ai_drive.before(crate::drive::car_physics));
        if let Some(seconds) = self.test {
            app.insert_resource(AiTest { seconds, ..default() });
        }
    }
}

/// The route and start grid a race hands to its AI drivers. Insert it before the opponents arrive; without it
/// `--ai` builds a test route along the traffic lanes from the player's spawn.
#[derive(Resource, Clone)]
pub struct RaceSetup {
    pub route: Arc<AiRoute>,
    /// Start-grid transforms (road level, facing along the route), one per opponent in order. Opponents
    /// beyond the grid are placed behind the last slot along the route.
    pub grid: Vec<Transform>,
}

/// Opponent cars: requested from the loader, waiting to be placed, placed.
#[derive(Resource, Default)]
pub struct Opponents {
    bundles: Vec<String>,
    requested: bool,
    arrived: Vec<(usize, Box<CarSpec>, CarParts)>,
    spawned: Vec<bool>,
}

impl Opponents {
    /// Called by the loader-message pump when an opponent's car is ready.
    pub fn arrived(&mut self, slot: usize, spec: Box<CarSpec>, parts: CarParts) {
        self.arrived.push((slot, spec, parts));
    }
}

/// A route for AI drivers: a polyline at road level with the distance along it at every point.
#[derive(Debug, Clone)]
pub struct AiRoute {
    pub points: Vec<Vec3>,
    /// Distance from the first point (m).
    pub distance: Vec<f32>,
    /// The last point joins the first (a circuit).
    pub closed: bool,
}

impl AiRoute {
    pub fn new(mut points: Vec<Vec3>, closed: bool) -> Self {
        points.dedup_by(|a, b| a.distance(*b) < 0.05);
        let mut distance = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d += p.distance(points[i - 1]);
            }
            distance.push(d);
        }
        Self { points, distance, closed }
    }

    pub fn length(&self) -> f32 {
        self.distance.last().copied().unwrap_or(0.0)
            + if self.closed && self.points.len() > 1 {
                self.points[0].distance(*self.points.last().unwrap())
            } else {
                0.0
            }
    }

    fn segment_count(&self) -> usize {
        if self.closed { self.points.len() } else { self.points.len().saturating_sub(1) }
    }

    fn segment(&self, i: usize) -> (Vec3, Vec3, f32) {
        let a = self.points[i];
        let b = self.points[(i + 1) % self.points.len()];
        (a, b, self.distance[i])
    }

    /// Wrap (closed) or clamp (open) a distance onto the route.
    fn wrap(&self, s: f32) -> f32 {
        let len = self.length();
        if self.closed && len > 0.0 { s.rem_euclid(len) } else { s.clamp(0.0, len) }
    }

    /// The point and unit tangent at distance `s`.
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        let s = self.wrap(s);
        let n = self.segment_count();
        if n == 0 {
            return (self.points.first().copied().unwrap_or_default(), Vec3::Z);
        }
        // Binary search for the segment holding s.
        let i = match self.distance.binary_search_by(|d| d.total_cmp(&s)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
        .min(n - 1);
        let (a, b, d0) = self.segment(i);
        let len = a.distance(b).max(1e-4);
        let t = ((s - d0) / len).clamp(0.0, 1.0);
        (a.lerp(b, t), (b - a) / len)
    }

    /// Project `p` onto the route near the distance `hint` (searching `window` metres either side; the whole
    /// route when `hint` is `None`): (distance along, signed lateral offset (m, positive = left of travel)).
    pub fn project(&self, p: Vec3, hint: Option<f32>, window: f32) -> (f32, f32) {
        let n = self.segment_count();
        let mut best = (f32::MAX, 0.0, 0.0);
        for i in 0..n {
            let (a, b, d0) = self.segment(i);
            if let Some(h) = hint {
                let d1 = d0 + a.distance(b);
                let len = self.length();
                let near = |x: f32| {
                    let mut d = (x - h).abs();
                    if self.closed {
                        d = d.min(len - d);
                    }
                    d
                };
                if near(d0) > window && near(d1) > window && !(d0 <= h && h <= d1) {
                    continue;
                }
            }
            let ab = b - a;
            let flat = Vec3::new(ab.x, 0.0, ab.z);
            let len2 = flat.length_squared().max(1e-6);
            let ap = p - a;
            let t = (Vec3::new(ap.x, 0.0, ap.z).dot(flat) / len2).clamp(0.0, 1.0);
            let q = a + ab * t;
            let off = Vec3::new(p.x - q.x, 0.0, p.z - q.z);
            let d2 = off.length_squared() + 0.25 * (p.y - q.y).powi(2);
            if d2 < best.0 {
                // Left of travel: +Y × forward points left for a car facing +Z with +X to its left.
                let left = Vec3::Y.cross(flat).normalize_or_zero();
                best = (d2, d0 + ab.length() * t, off.dot(left));
            }
        }
        (best.1, best.2)
    }
}

/// Model decision (docs/decomp.md "AI racers"): a test route along the traffic lanes, used only when no race
/// hands one in. Start at the lane section nearest `start` running within 90° of `heading` (yaw, 0 = +Z), then
/// follow the forward links, at each fork the continuation that turns least, until `length` metres.
pub fn route_from_lanes(
    lanes: &mw_formats::TrafficLaneData,
    start: Vec3,
    heading: f32,
    length: f32,
) -> Option<AiRoute> {
    let dir = Vec3::new(heading.sin(), 0.0, heading.cos());
    let mid = |hull: usize, section: usize| -> Option<Vec<Vec3>> {
        let h = lanes.hulls.get(hull)?;
        let s = h.sections.get(section)?;
        let rungs = h.rungs.get(s.first_rung as usize..s.first_rung as usize + s.rung_count as usize)?;
        Some(rungs.iter().map(|[a, b]| (Vec3::from_slice(&a[..3]) + Vec3::from_slice(&b[..3])) * 0.5).collect())
    };
    // The starting section: nearest rung midpoint, lane running our way.
    let mut best: Option<(f32, usize, usize, usize)> = None;
    for lane in lanes.lanes_near(start.x, start.z, 60.0) {
        let pts: Vec<Vec3> = lane.points.iter().map(|p| Vec3::from(*p)).collect();
        for (i, w) in pts.windows(2).enumerate() {
            let along = (w[1] - w[0]).normalize_or_zero();
            if along.dot(dir) < 0.0 {
                continue;
            }
            let d = Vec3::new(w[0].x - start.x, 0.0, w[0].z - start.z).length();
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, lane.hull, lane.section, i));
            }
        }
    }
    let (_, mut hull, mut section, first) = best?;
    let mut points: Vec<Vec3> = mid(hull, section)?.into_iter().skip(first).collect();
    let mut total = 0.0;
    let mut visited = std::collections::HashSet::new();
    while total < length {
        let s = &lanes.hulls[hull].sections[section];
        let end_dir = match points.len() {
            n if n >= 2 => (points[n - 1] - points[n - 2]).normalize_or_zero(),
            _ => dir,
        };
        let mut next: Option<(f32, usize, usize, Vec<Vec3>)> = None;
        for &(h, sec) in &s.forward {
            let (h, sec) = (h as usize, sec as usize);
            if visited.contains(&(h, sec)) {
                continue;
            }
            let Some(pts) = mid(h, sec) else { continue };
            if pts.len() < 2 {
                continue;
            }
            let turn = (pts[1] - pts[0]).normalize_or_zero().dot(end_dir);
            if next.as_ref().is_none_or(|n| turn > n.0) {
                next = Some((turn, h, sec, pts));
            }
        }
        let Some((_, h, sec, pts)) = next else { break };
        visited.insert((h, sec));
        let before = points.len();
        points.extend(pts);
        total += points[before.saturating_sub(1)..].windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
        (hull, section) = (h, sec);
    }
    (points.len() >= 2).then(|| AiRoute::new(points, false))
}

/// Grid slots along `route` from distance `s0`: two abreast, `gap` metres between rows, `half_width` either side.
pub fn grid_along(route: &AiRoute, s0: f32, count: usize, gap: f32, half_width: f32) -> Vec<Transform> {
    (0..count)
        .map(|i| {
            let s = s0 - (i / 2) as f32 * gap;
            let (p, t) = route.at(s);
            let left = Vec3::Y.cross(Vec3::new(t.x, 0.0, t.z)).normalize_or_zero();
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            Transform::from_translation(p + left * side * half_width)
                .with_rotation(Quat::from_rotation_y(t.x.atan2(t.z)))
        })
        .collect()
}

/// The AI driver of one car (`ai_drive`).
#[derive(Component, Debug)]
pub struct AiDriver {
    pub route: Arc<AiRoute>,
    /// Distance along the route at the last tick, and the lateral offset from it (m, positive = left).
    pub progress: f32,
    pub lateral: f32,
    /// Target speed of the last tick (m/s).
    pub target_speed: f32,
    /// Seconds since the start, and when each 100 m mark was passed.
    pub time: f32,
    pub splits: Vec<f32>,
    pub finished: Option<f32>,
}

impl AiDriver {
    pub fn new(route: Arc<AiRoute>) -> Self {
        Self { route, progress: 0.0, lateral: 0.0, target_speed: 0.0, time: 0.0, splits: Vec::new(), finished: None }
    }
}

/// `--test ai` state.
#[derive(Resource, Default)]
struct AiTest {
    seconds: f32,
    started: bool,
}

/// Build the test race when there is none: once the player's car is on the road, a route along the lanes from
/// it and grid slots ahead of it.
fn setup_race(
    mut commands: Commands,
    race: Option<Res<RaceSetup>>,
    opponents: Res<Opponents>,
    game: Res<crate::Game>,
    test: Option<Res<AiTest>>,
    player: Query<(Entity, &Transform), (With<Player>, With<Car>)>,
    mut done: Local<bool>,
) {
    if *done || race.is_some() || (opponents.bundles.is_empty() && test.is_none()) {
        return;
    }
    let Ok((player_entity, t)) = player.single() else { return };
    *done = true;
    let fwd = t.rotation * Vec3::Z;
    let heading = fwd.x.atan2(fwd.z);
    let lanes = match game.store.traffic_lanes() {
        Ok(l) => l,
        Err(e) => {
            warn!("AI: no traffic lanes ({e}); no route");
            return;
        }
    };
    let Some(route) = route_from_lanes(&lanes, t.translation, heading, 3000.0) else {
        warn!("AI: no lane near {:?}; no route", t.translation);
        return;
    };
    let route = Arc::new(route);
    let (s_player, _) = route.project(t.translation, None, 0.0);
    info!(
        "AI route: {} points, {:.0} m along the traffic lanes from {:?} (player at {:.0} m)",
        route.points.len(),
        route.length(),
        route.points[0],
        s_player
    );
    // Opponents line up ahead of the player: the first row 20 m ahead.
    let grid = grid_along(
        &route,
        s_player + 20.0 + 10.0 * (opponents.bundles.len().saturating_sub(1) / 2) as f32,
        opponents.bundles.len(),
        10.0,
        2.0,
    );
    if test.is_some() {
        // The player's car follows the same route with the same driver: its pace is the reference.
        commands.entity(player_entity).insert((AiDriver::new(route.clone()), crate::drive::ExternalControls));
    }
    commands.insert_resource(RaceSetup { route, grid });
}

fn request_opponents(mut opponents: ResMut<Opponents>, requests: Option<Res<LoaderRequests>>) {
    if opponents.requested || opponents.bundles.is_empty() {
        return;
    }
    let Some(requests) = requests else { return };
    opponents.requested = true;
    for (slot, bundle) in opponents.bundles.clone().into_iter().enumerate() {
        // A paint per slot, so two of the same car differ.
        let req = CarRequest { bundle, paint_index: slot + 1, opponent: Some(slot) };
        if requests.0.send(req).is_err() {
            warn!("AI: the loader thread has stopped");
        }
    }
}

/// Place opponents that have arrived once the race (route and grid) is known and the ground under their slot
/// has streamed in.
fn spawn_opponents(
    mut commands: Commands,
    mut opponents: ResMut<Opponents>,
    race: Option<Res<RaceSetup>>,
    spatial: SpatialQuery,
) {
    let Some(race) = race else { return };
    let mut waiting = Vec::new();
    for (slot, spec, parts) in std::mem::take(&mut opponents.arrived) {
        let grid = race.grid.get(slot).copied().unwrap_or_else(|| {
            let last = race.grid.last().copied().unwrap_or_default();
            let (s, _) = race.route.project(last.translation, None, 0.0);
            grid_along(&race.route, s - 10.0 * (slot + 1 - race.grid.len()) as f32, 1, 0.0, 0.0)[0]
        });
        let origin = grid.translation + Vec3::Y * 5.0;
        let ground_only = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
        let Some(hit) = spatial.cast_ray(origin, Dir3::NEG_Y, 40.0, true, &ground_only) else {
            waiting.push((slot, spec, parts));
            continue;
        };
        let start = origin - Vec3::Y * hit.distance + Vec3::Y * 0.15;
        let fwd = grid.rotation * Vec3::Z;
        let car = crate::drive::build_car(
            &mut commands,
            &CarBuild {
                name: &format!("opponent {slot}"),
                spec: &spec,
                parts: &parts,
                handling: &spec.handling,
                nitrous_burn: None,
                start,
                heading: fwd.x.atan2(fwd.z),
                velocity: Vec3::ZERO,
                damage: Some(crate::damage::DamageSource(spec.damage.clone().map(|d| (d, spec.impact_graph)))),
            },
        );
        commands.entity(car).insert(AiDriver::new(race.route.clone()));
        opponents.spawned[slot] = true;
        info!("AI: opponent {slot} ({}) placed at {start:.1?}", spec.bundle);
    }
    opponents.arrived = waiting;
}

/// Every fixed tick, before the physics: each AI driver writes its car's controls.
fn ai_drive(time: Res<Time>, mut cars: Query<(&Car, &Transform, &LinearVelocity, &mut AiDriver, &mut CarControls)>) {
    let dt = time.delta_secs();
    for (car, t, v, mut ai, mut ctl) in &mut cars {
        let ai = &mut *ai;
        ai.time += dt;
        let hint = (ai.time > dt * 1.5).then_some(ai.progress);
        let (s, lateral) = ai.route.project(t.translation, hint, 30.0);
        ai.progress = s;
        ai.lateral = lateral;
        while (ai.splits.len() as f32 + 1.0) * 100.0 <= s {
            ai.splits.push(ai.time);
        }
        if ai.finished.is_none() && !ai.route.closed && s >= ai.route.length() - 1.0 {
            ai.finished = Some(ai.time);
        }
        *ctl = drive_controls(car, t, v.0, ai);
    }
}

/// The driving law (placeholder until the port lands).
fn drive_controls(car: &Car, t: &Transform, v: Vec3, ai: &mut AiDriver) -> CarControls {
    let speed = v.length();
    let look = 8.0 + 0.6 * speed;
    let (target, _) = ai.route.at(ai.progress + look);
    let local = t.rotation.inverse() * (target - t.translation);
    let angle = local.x.atan2(local.z);
    let max = car.params.steering.max_angle.to_radians().max(0.1);
    let steer = (angle / max).clamp(-1.0, 1.0);
    // Speed from the curvature ahead.
    let mut limit = 90.0f32;
    for k in 1..12 {
        let s = ai.progress + k as f32 * 10.0;
        let (_, t0) = ai.route.at(s - 5.0);
        let (_, t1) = ai.route.at(s + 5.0);
        let curv = t0.angle_between(t1) / 10.0;
        let vmax = (9.0 / curv.max(1e-4)).sqrt();
        let dist = k as f32 * 10.0;
        limit = limit.min((vmax * vmax + 2.0 * 7.0 * dist).sqrt());
    }
    ai.target_speed = limit;
    let (throttle, brake) = if speed < limit {
        (1.0, 0.0)
    } else if speed > limit + 2.0 {
        (0.0, 1.0)
    } else {
        (0.3, 0.0)
    };
    if ai.finished.is_some() || ai.route.length() - ai.progress < 20.0 {
        return CarControls { brake: 1.0, ..default() };
    }
    let _ = MPH;
    CarControls { throttle, brake, steer, ..default() }
}

/// Once a second: every AI car's position, speed, progress and lateral error; in `--test ai`, the result.
fn log_ai(
    time: Res<Time>,
    mut next: Local<f32>,
    cars: Query<(&Name, &Transform, &LinearVelocity, &AiDriver, Has<Player>)>,
    test: Option<ResMut<AiTest>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now.floor() + 1.0;
    for (name, t, v, ai, player) in &cars {
        info!(
            "AI {}{}: at {:.0} {:.1} {:.0}, {:.0} km/h (target {:.0}), {:.0} m along, lateral {:+.2} m",
            name,
            if player { " (player car)" } else { "" },
            t.translation.x,
            t.translation.y,
            t.translation.z,
            v.length() * 3.6,
            ai.target_speed * 3.6,
            ai.progress,
            ai.lateral
        );
    }
    let Some(mut test) = test else { return };
    let Some(longest) = cars.iter().map(|c| c.3.time).reduce(f32::max) else { return };
    test.started = true;
    let all_done = cars.iter().all(|c| c.3.finished.is_some());
    if longest >= test.seconds || all_done {
        for (name, _, _, ai, player) in &cars {
            let max_lat = ai.lateral.abs();
            info!(
                "TEST RESULT AI {}{}: {:.0} m in {:.1} s ({:.1} km/h mean), 100 m splits {:?}, finished {:?}, lateral now {:.2} m",
                name,
                if player { " (player car)" } else { "" },
                ai.progress,
                ai.time,
                ai.progress / ai.time.max(1e-3) * 3.6,
                ai.splits.iter().map(|s| (s * 10.0).round() / 10.0).collect::<Vec<_>>(),
                ai.finished,
                max_lat
            );
        }
        exit.write(AppExit::Success);
    }
}
