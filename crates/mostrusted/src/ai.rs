//! AI racers: opponent cars that run the player's physics (`drive.rs`, every `Car` steps from its own
//! [`CarControls`]) and are driven along a route by an AI driver.
//!
//! A race hands in a [`RaceSetup`] (the route as a polyline and the start-grid transforms); without one, `--ai`
//! builds a test route along the AI road graph (`AIData`) from the player's spawn point (`route_from_ai_graph`) and places the
//! opponents on it ahead of and beside the player. Opponent cars are loaded by the loader thread like the
//! player's (`CarRequest::opponent`) and built by [`crate::drive::build_car`].

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::ai_driver::RubberBand;
use crate::drive::{Car, CarBuild, CarControls, CarParts, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests};

pub struct AiPlugin {
    /// Opponent car bundles (`--ai`).
    pub cars: Vec<String>,
    /// `--test ai`: drive the player car with the same driver, log both, exit after `seconds`.
    pub test: Option<f32>,
    /// `--ai-balancing <RaceBalancingData>,<RaceBalancingProfile>`: give the test race the event's rubber band.
    pub balancing: Option<(mw_bundle::ResourceId, mw_bundle::ResourceId)>,
}

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Opponents {
            bundles: self.cars.clone(),
            requested: false,
            arrived: Vec::new(),
            spawned: vec![false; self.cars.len()],
        })
        .init_resource::<AiSettings>()
        .add_systems(Startup, |game: Res<crate::Game>, mut s: ResMut<AiSettings>| s.0 = load_ai_config(&game.store))
        .add_systems(Update, (setup_race, request_opponents, spawn_opponents, log_ai).chain())
        .add_systems(FixedUpdate, ai_drive.before(crate::drive::car_physics));
        if let Some(seconds) = self.test {
            app.insert_resource(AiTest { seconds, ..default() });
        }
        if let Some(b) = self.balancing {
            app.insert_resource(AiBalancingArg(Some(b)));
        }
    }
}

/// `--ai-balancing`: the `RaceBalancingData` and `RaceBalancingProfile` ids for the test race.
#[derive(Resource)]
struct AiBalancingArg(Option<(mw_bundle::ResourceId, mw_bundle::ResourceId)>);

/// The route and start grid a race hands to its AI drivers. Insert it before the opponents arrive; without it
/// `--ai` builds a test route along the traffic lanes from the player's spawn.
#[derive(Resource, Clone)]
pub struct RaceSetup {
    pub route: Arc<AiRoute>,
    /// Start-grid transforms (road level, facing along the route), one per opponent in order. Opponents
    /// beyond the grid are placed behind the last slot along the route.
    pub grid: Vec<Transform>,
    /// Each opponent's rubber band (by slot; `None` = no schedule, full speed). [`rubber_band_from_data`] reads
    /// the event's `RaceBalancingData` / `RaceBalancingProfile`.
    pub balancing: Vec<Option<RubberBand>>,
}

/// One opponent's rubber band from the event's `RaceBalancingData` (`OpponentData[k]`) and `RaceBalancingProfile`
/// GeneSys objects (`OfflineEvent` +0x94 / +0x98), with the event's pace multiplier for the player's car
/// (`AllowedVehicleList` entry `a17a5e72`, 1.0 by default).
pub fn rubber_band_from_data(
    store: &mw_assets::AssetStore,
    data: mw_bundle::ResourceId,
    profile: mw_bundle::ResourceId,
    k: usize,
    pace: f32,
) -> anyhow::Result<RubberBand> {
    use mw_formats::genesys::Value;
    let rbd = store.genesys_object(data)?;
    let prof = store.genesys_object(profile)?;
    let f = |o: &mw_formats::genesys::Object, h: u32, default: f32| match o.get(h) {
        Some(Value::Float(v)) => *v,
        _ => default,
    };
    let Some(Value::Array(opps)) = rbd.get(0x811f88a3) else { anyhow::bail!("{data}: no OpponentData") };
    let Some(Value::Struct(e)) = opps.get(k).or(opps.last()) else { anyhow::bail!("{data}: no opponent {k}") };
    Ok(RubberBand {
        max_speed: f(&prof, 0x02318b5f, 100.0) * pace * 0.44704,
        ahead: f(e, 0x01b78556, 100.0),
        behind: f(e, 0x849481e9, 100.0),
        late_x: f(e, 0xb11db112, 1.0),
        late: [f(e, 0x7c5d657c, 1.0), f(e, 0x8fd2c7ab, 1.0)],
        early_x: f(e, 0x871a887d, 0.0),
        early: [f(e, 0xa7662f40, 1.0), f(e, 0xab82db21, 1.0)],
    })
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
    /// Drivable half-width at each point (m; empty when unknown).
    pub half_width: Vec<f32>,
    /// The path nodes the driver reasons about (the game's 0x50-byte path elements: one per road-graph node):
    /// for a polyline handed in, its points.
    pub nodes: Vec<RouteNode>,
}

/// A path node: distance along the route, the signed heading change there (rad, positive = left; the game's node
/// +0x28) and the radius of the circle through it and its neighbours (m; the game's +0x2c, infinite when straight).
#[derive(Debug, Clone, Copy)]
pub struct RouteNode {
    pub s: f32,
    pub turn: f32,
    pub radius: f32,
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
        let mut route = Self { points, distance, closed, half_width: Vec::new(), nodes: Vec::new() };
        let pts = route.points.clone();
        route.set_nodes(&pts);
        route
    }

    /// Set the path nodes from their positions (in route order, on the route).
    pub fn set_nodes(&mut self, positions: &[Vec3]) {
        let mut hint = None;
        let s: Vec<f32> = positions
            .iter()
            .map(|p| {
                let (s, _) = self.project(*p, hint, 60.0);
                hint = Some(s);
                s
            })
            .collect();
        let flat = |v: Vec3| Vec2::new(v.x, v.z);
        self.nodes = (0..positions.len())
            .map(|k| {
                let (Some(prev), Some(next)) = (k.checked_sub(1).map(|j| positions[j]), positions.get(k + 1)) else {
                    return RouteNode { s: s[k], turn: 0.0, radius: f32::INFINITY };
                };
                let (a, b) = (flat(positions[k] - prev), flat(*next - positions[k]));
                // Signed angle from a to b; with +X to the left of +Z, a turn to the left is a positive perp-dot.
                let turn = (a.x * b.y - a.y * b.x).atan2(a.dot(b)) * -1.0;
                // The circle through three points `chord` apart turning by Δθ: R = (chord / 2) / sin(Δθ / 2)
                // (the game's (chord / 2) / cos((π − |Δθ|) / 2), FUN_004ea860).
                let chord = 0.5 * (a.length() + b.length());
                let half = (turn.abs() * 0.5).sin();
                let radius = if half > 1e-4 { chord * 0.5 / half } else { f32::INFINITY };
                RouteNode { s: s[k], turn, radius }
            })
            .collect();
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

    /// The drivable half-width at distance `s` (m; infinite when the route has no widths).
    pub fn half_width_at(&self, s: f32) -> f32 {
        if self.half_width.len() != self.points.len() {
            return f32::INFINITY;
        }
        let s = self.wrap(s);
        let i = match self.distance.binary_search_by(|d| d.total_cmp(&s)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        self.half_width[i.min(self.half_width.len() - 1)]
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

/// A path along the AI's road graph (`AIData`) from `start` heading `heading` (yaw, 0 = +Z), the way the game's path
/// builders walk it (`FUN_004ed1a0` / `FUN_004e8ac0`, docs/decomp.md "AI racers"): from the nearest non-junction
/// node (`FUN_004375b0`), each step takes the neighbour best aligned with the direction so far, main roads before
/// shortcut-flagged ones (`FUN_00436c00`); nodes of width 0 are not path points. The centre line is a
/// Catmull-Rom spline through the nodes (tangents 0.5 × (next − previous), `FUN_004e6160`), sampled every ~2 m,
/// with the node's drivable half-width (`width` × 0.5, `FUN_004f2310`). Picking the straightest exit at every
/// junction is ours (the game's junction choice, `FUN_004e90d0`, is random or angle-based and not ported): it is
/// only the test route used when no race hands one in.
pub fn route_from_ai_graph(ai: &mw_formats::AiData, start: Vec3, heading: f32, length: f32) -> Option<AiRoute> {
    let pos = |i: usize| Vec3::from(ai.nodes[i].position);
    let first = ai.nearest_node(start.to_array(), true)?;
    let mut dir = Vec2::new(heading.sin(), heading.cos());
    let mut nodes = vec![first];
    let mut total = 0.0;
    let mut prev = None;
    let mut cur = first;
    while total < length && nodes.len() < 20_000 {
        let Some(next) = ai.next_node(cur, dir.to_array(), prev) else { break };
        let d = pos(next) - pos(cur);
        total += d.length();
        dir = Vec2::new(d.x, d.z).normalize_or(dir);
        prev = Some(cur);
        cur = next;
        nodes.push(cur);
    }
    let nodes: Vec<usize> = nodes.into_iter().filter(|&i| ai.nodes[i].width > 0.0).collect();
    if nodes.len() < 2 {
        return None;
    }
    let (mut points, mut widths) = (Vec::new(), Vec::new());
    for k in 0..nodes.len() - 1 {
        let p = |j: isize| pos(nodes[j.clamp(0, nodes.len() as isize - 1) as usize]);
        let (p0, p1, p2, p3) = (p(k as isize - 1), p(k as isize), p(k as isize + 1), p(k as isize + 2));
        let (m1, m2) = ((p2 - p0) * 0.5, (p3 - p1) * 0.5);
        let steps = (p1.distance(p2) / 2.0).ceil().max(1.0) as usize;
        let (w1, w2) = (ai.nodes[nodes[k]].width * 0.5, ai.nodes[nodes[k + 1]].width * 0.5);
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let h = [2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, -2.0 * t3 + 3.0 * t2, t3 - t2];
            points.push(p1 * h[0] + m1 * h[1] + p2 * h[2] + m2 * h[3]);
            widths.push(w1 + (w2 - w1) * t);
        }
    }
    points.push(pos(*nodes.last().unwrap()));
    widths.push(ai.nodes[*nodes.last().unwrap()].width * 0.5);
    let mut route = AiRoute::new(points, false);
    route.half_width = widths;
    route.set_nodes(&nodes.iter().map(|&i| pos(i)).collect::<Vec<_>>());
    Some(route)
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
    pub state: crate::ai_driver::DriverState,
    /// The rubber band's schedule speed (m/s; `f32::MAX` without a race, as the game's schedule returns then).
    pub schedule_speed: f32,
    /// The race's rubber band, which sets `schedule_speed` every tick.
    pub rubber_band: Option<RubberBand>,
    /// Distance along the route at the last tick, and the lateral offset from it (m, positive = left).
    pub progress: f32,
    pub lateral: f32,
    /// Seconds since the start, and when each 100 m mark was passed.
    pub time: f32,
    pub splits: Vec<f32>,
    pub finished: Option<f32>,
    /// Test statistics: the largest lateral error so far (m), body hits on the world, resets.
    pub max_lateral: f32,
    pub hits: u32,
    pub resets: u32,
    touching: bool,
}

impl AiDriver {
    pub fn new(route: Arc<AiRoute>) -> Self {
        Self {
            route,
            state: Default::default(),
            schedule_speed: f32::MAX,
            rubber_band: None,
            progress: 0.0,
            lateral: 0.0,
            time: 0.0,
            splits: Vec::new(),
            finished: None,
            max_lateral: 0.0,
            hits: 0,
            resets: 0,
            touching: false,
        }
    }
}

/// The racer's `AIConfig` values (`ai_driver::AiConfig`), read from `GLOBALCONFIG.BNDL` when the race is set up.
#[derive(Resource, Default)]
pub struct AiSettings(pub crate::ai_driver::AiConfig);

/// Read `AIConfig` (010000000007fb0e) by field hash; a field the data lacks keeps the default (the data's value).
fn load_ai_config(store: &mw_assets::AssetStore) -> crate::ai_driver::AiConfig {
    use mw_formats::genesys::Value;
    let mut c = crate::ai_driver::AiConfig::default();
    let obj = match store.genesys_object(mw_bundle::ResourceId(0x010000000007fb0e)) {
        Ok(o) => o,
        Err(e) => {
            warn!("AI: no AIConfig ({e}); using the defaults (the data's values)");
            return c;
        }
    };
    let f = |hash: u32, slot: &mut f32| {
        if let Some(Value::Float(v)) = obj.get(hash) {
            *slot = *v;
        }
    };
    // Field hashes by instance offset (`mwtool genesys-types`).
    f(0x54d23bfb, &mut c.nitrous_refill); // 0x10
    f(0xbd6f7f45, &mut c.lane_rate); // 0x18
    f(0x6f76d23f, &mut c.contact_time); // 0x28
    f(0x492f7ddb, &mut c.corner_speed_scale); // 0xc8
    f(0x27d18b55, &mut c.nitrous_keep_turn); // 0x124
    f(0x63c7374a, &mut c.nitrous_start_turn); // 0x128
    f(0xc064967b, &mut c.nitrous_min_tank); // 0x140
    f(0xe9d45ba2, &mut c.min_look); // 0x144
    f(0xd695c3bb, &mut c.nitrous_margin_mph); // 0x14c
    f(0x619a7200, &mut c.stuck_end_radius); // 0x19c
    f(0x51c3d831, &mut c.reset_time); // 0x1a0
    f(0xae76a715, &mut c.reset_time_hidden); // 0x1a4
    f(0x072ea6da, &mut c.corner_clearance); // 0x1ec
    f(0x8744d6b3, &mut c.look_scan_time); // 0x1f0
    f(0xc932b6d4, &mut c.sharp_turn); // 0x1f4
    f(0xedfecd74, &mut c.min_look_time); // 0x1fc
    info!("AI: AIConfig {c:?}");
    c
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
    balancing: Option<Res<AiBalancingArg>>,
    mut done: Local<bool>,
) {
    if *done || race.is_some() || (opponents.bundles.is_empty() && test.is_none()) {
        return;
    }
    let Ok((player_entity, t)) = player.single() else { return };
    *done = true;
    let fwd = t.rotation * Vec3::Z;
    let heading = fwd.x.atan2(fwd.z);
    let graph = match game.store.ai_data() {
        Ok(g) => g,
        Err(e) => {
            warn!("AI: no road graph ({e}); no route");
            return;
        }
    };
    let Some(route) = route_from_ai_graph(&graph, t.translation, heading, 3000.0) else {
        warn!("AI: no lane near {:?}; no route", t.translation);
        return;
    };
    let route = Arc::new(route);
    let (s_player, _) = route.project(t.translation, None, 0.0);
    info!(
        "AI route: {} points, {:.0} m along the AI road graph from {:?} (player at {:.0} m)",
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
    let balancing = (0..opponents.bundles.len())
        .map(|k| {
            let (data, profile) = balancing.as_ref()?.0?;
            match rubber_band_from_data(&game.store, data, profile, k, 1.0) {
                Ok(rb) => {
                    info!("AI: opponent {k} rubber band {rb:?}");
                    Some(rb)
                }
                Err(e) => {
                    warn!("AI: no rubber band for opponent {k}: {e:#}");
                    None
                }
            }
        })
        .collect();
    commands.insert_resource(RaceSetup { route, grid, balancing });
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
        let mut driver = AiDriver::new(race.route.clone());
        driver.rubber_band = race.balancing.get(slot).copied().flatten();
        commands.entity(car).insert(driver);
        opponents.spawned[slot] = true;
        info!("AI: opponent {slot} ({}) placed at {start:.1?}", spec.bundle);
    }
    opponents.arrived = waiting;
}

/// Every fixed tick, before the physics: each AI driver writes its car's controls (the game runs the AI pass
/// before the player manager and the world step, `FUN_004766a0`).
#[allow(clippy::type_complexity)]
fn ai_drive(
    time: Res<Time>,
    settings: Res<AiSettings>,
    spatial: SpatialQuery,
    mut cars: Query<(
        Entity,
        &mut Car,
        &mut Transform,
        &mut LinearVelocity,
        &mut AngularVelocity,
        Option<&mut AiDriver>,
        &mut CarControls,
        Has<Player>,
    )>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // Every car for the emergency brake (circle radius from the body's half extents), and the player's position.
    let all: Vec<(Entity, crate::ai_driver::OtherCar)> = cars
        .iter()
        .map(|(e, car, t, v, ..)| {
            let fwd = t.rotation * Vec3::Z;
            let h = Vec2::new(fwd.x, fwd.z).normalize_or(Vec2::Y);
            let ext = car.params.body_half_extents;
            let other = crate::ai_driver::OtherCar {
                position: Vec2::new(t.translation.x, t.translation.z),
                velocity: h * v.0.length(),
                radius: ext.x.max(ext.z),
            };
            (e, other)
        })
        .collect();
    let player = cars.iter().find(|c| c.7).map(|c| (c.0, c.2.translation));
    // The player's progress along each route (the race tracker's remaining distance, `world+0x2f47c` +0xdf0).
    let player_progress = |route: &AiRoute| player.map(|(_, p)| route.project(p, None, 0.0).0);
    for (entity, mut car, mut t, mut v, mut w, ai, mut ctl, _) in &mut cars {
        let Some(mut ai) = ai else { continue };
        let ai = &mut *ai;
        ai.time += dt;
        let hint = (ai.time > dt * 1.5).then_some(ai.progress);
        let (s, lateral) = ai.route.project(t.translation, hint, 30.0);
        ai.progress = s;
        ai.lateral = lateral;
        if ai.time > 1.0 {
            ai.max_lateral = ai.max_lateral.max(lateral.abs());
        }
        if car.body_contact && !ai.touching {
            ai.hits += 1;
        }
        ai.touching = car.body_contact;
        while (ai.splits.len() as f32 + 1.0) * 100.0 <= s {
            ai.splits.push(ai.time);
        }
        if ai.finished.is_none() && !ai.route.closed && s >= ai.route.length() - 1.0 {
            ai.finished = Some(ai.time);
        }
        // The AI's tank refills at `AIConfig` 0x10 per second (`FUN_00667c10`, the AI branch).
        car.nitrous.add(settings.0.nitrous_refill * dt);

        let fwd = t.rotation * Vec3::Z;
        // Probe rays at ±25° (`.data` 0x010c4234 = 0.436 rad) in the direction of travel, for the recovery.
        let reverse = ai.state.recovering.is_some_and(|r| r.reverse);
        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider]);
        let probes = [0.436f32, -0.436].map(|a| {
            let dir = Quat::from_rotation_y(a) * if reverse { -fwd } else { fwd };
            Dir3::new(dir)
                .ok()
                .and_then(|d| spatial.cast_ray(t.translation + Vec3::Y * 0.5, d, PROBE_LENGTH, true, &filter))
                .is_some()
        });
        let others: Vec<_> = all.iter().filter(|o| o.0 != entity).map(|o| o.1).collect();
        let player_ahead = player.is_some_and(|(e, p)| e != entity && (p - t.translation).dot(fwd) > 0.0);
        let ext = car.params.body_half_extents;
        let input = crate::ai_driver::DriverInput {
            dt,
            route: &ai.route,
            progress: s,
            position: t.translation,
            forward: fwd,
            velocity: v.0,
            radius: ext.x.max(ext.z),
            steer_gains: car.params.ai_steer_pid,
            schedule_speed: match (ai.rubber_band, player_progress(&ai.route)) {
                (Some(rb), Some(ps)) if player.is_some_and(|(e, _)| e != entity) => {
                    let x = (s / ai.route.length().max(1.0)).clamp(0.0, 1.0);
                    let gap = player.map_or(0.0, |(_, p)| p.distance(t.translation));
                    ai.schedule_speed = rb.speed(x, s - ps, gap);
                    ai.schedule_speed
                }
                _ => ai.schedule_speed,
            },
            corner_speed: corner_speed(&ai.route, s, &car, &settings.0),
            all_wheels_off: car.wheels_down() == 0,
            contact: car.body_contact,
            nitrous_fill: car.nitrous.fill(),
            nitrous_burning: car.nitrous.burning,
            player_ahead,
            probes,
            others: &others,
        };
        let out = ai.state.tick(&settings.0, &input);
        *ctl = out.controls;
        if ai.finished.is_some() || (!ai.route.closed && ai.route.length() - s < 15.0) {
            // End of an open route: stop (ours; a race's finish logic will take over).
            *ctl = CarControls { brake: 1.0, ..default() };
        }
        if out.reset {
            // The game's reset request (`FUN_0083d4b0`): back on the path where the car got stuck. Model decision:
            // the placement (the route's point at the car's progress, facing along it, at rest).
            let (p, tangent) = ai.route.at(s);
            t.translation = p + Vec3::Y * 0.5;
            t.rotation = Quat::from_rotation_y(tangent.x.atan2(tangent.z));
            v.0 = Vec3::ZERO;
            w.0 = Vec3::ZERO;
            ai.resets += 1;
        }
    }
}

/// Length of the recovery's probe rays (m). Model decision: not read.
const PROBE_LENGTH: f32 = 6.0;

/// The corner speed limit at distance `s` (placeholder until its source is settled).
fn corner_speed(route: &AiRoute, s: f32, car: &Car, cfg: &crate::ai_driver::AiConfig) -> f32 {
    let mu = 1.0;
    let decel = 8.0;
    let mut limit = f32::MAX;
    for n in route.nodes.iter().filter(|n| n.s > s && n.s < s + 400.0) {
        let v = (cfg.corner_speed_scale * (9.81 * mu * n.radius).sqrt()).min(car.params.top_speed);
        limit = limit.min((v * v + 2.0 * decel * (n.s - s)).sqrt());
    }
    limit
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
            ai.state.target_speed.min(999.0) * 3.6,
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
            info!(
                "TEST RESULT AI {}{}: {:.0} m in {:.1} s ({:.1} km/h mean), finished {:?}, max lateral {:.2} m, {} world hits, 100 m splits {:?}",
                name,
                if player { " (player car)" } else { "" },
                ai.progress,
                ai.time,
                ai.progress / ai.time.max(1e-3) * 3.6,
                ai.finished,
                ai.max_lateral,
                ai.hits,
                ai.splits.iter().map(|s| (s * 10.0).round() / 10.0).collect::<Vec<_>>(),
            );
        }
        exit.write(AppExit::Success);
    }
}
