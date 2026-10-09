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
        .add_systems(Update, (race_from_event, setup_race, request_opponents, spawn_opponents, log_ai).chain())
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
/// `--ai` builds a test route along the AI road graph from the player's spawn.
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
    /// The path nodes the driver reasons about (the game's 0x50-byte path elements), every 15 m along the route.
    pub nodes: Vec<RouteNode>,
}

/// Path node spacing (m): the game resamples a race path every 15 m (`FUN_004e95e0`).
const NODE_SPACING: f32 = 15.0;

/// A path node: distance along the route, ground-plane position, and the signed heading change there (rad,
/// positive = left; the game's node +0x28 = wrap(heading of the next segment − heading of the previous one),
/// `FUN_004e6a10`, whose sign is the other way round).
#[derive(Debug, Clone, Copy)]
pub struct RouteNode {
    pub s: f32,
    pub position: Vec2,
    pub turn: f32,
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
        // The game resamples a race path every 15 m (`FUN_004e95e0`).
        let count = (route.length() / NODE_SPACING).floor() as usize + 1;
        let flat = |v: Vec3| Vec2::new(v.x, v.z);
        let pts: Vec<(f32, Vec2)> =
            (0..count).map(|k| k as f32 * NODE_SPACING).map(|s| (s, flat(route.at(s).0))).collect();
        route.nodes = (0..pts.len())
            .map(|k| {
                let (s, position) = pts[k];
                let turn = match (k.checked_sub(1).map(|j| pts[j].1), pts.get(k + 1).map(|p| p.1)) {
                    (Some(prev), Some(next)) => {
                        let (a, b) = (position - prev, next - position);
                        // With +X to the left of +Z, a turn to the left has a negative perp-dot.
                        -(a.x * b.y - a.y * b.x).atan2(a.dot(b))
                    }
                    _ => 0.0,
                };
                RouteNode { s, position, turn }
            })
            .collect();
        route
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
    spline_through(ai, &nodes)
}

/// A race's path along the road graph through `waypoints` (the start, then every gate's ground point): between
/// consecutive waypoints the shortest node path (`AiData::path`; the game asks its road network for each segment,
/// `FUN_006423e0`, and A*-searches the junction graph, `FUN_00527300`), joined into one spline.
pub fn route_through(ai: &mw_formats::AiData, waypoints: &[Vec3]) -> Option<AiRoute> {
    let mut nodes: Vec<usize> = Vec::new();
    for w in waypoints.windows(2) {
        let a = ai.nearest_node(w[0].to_array(), false)?;
        let b = ai.nearest_node(w[1].to_array(), false)?;
        let path = ai.path(a, b)?;
        let skip = usize::from(nodes.last() == path.first());
        nodes.extend(path.into_iter().skip(skip));
    }
    spline_through(ai, &nodes)
}

/// The centre line through road-graph `nodes`: a Catmull-Rom spline (tangents 0.5 × (next − previous),
/// `FUN_004e6160`) sampled every ~2 m, with the nodes' drivable half-widths; nodes of width 0 are skipped.
fn spline_through(ai: &mw_formats::AiData, nodes: &[usize]) -> Option<AiRoute> {
    let pos = |i: usize| Vec3::from(ai.nodes[i].position);
    let nodes: Vec<usize> = nodes.iter().copied().filter(|&i| ai.nodes[i].width > 0.0).collect();
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
    /// The path's corner speeds for this car (`ai_driver::corner_speeds`), built on first use.
    pub corner_speeds: Option<Vec<f32>>,
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
            corner_speeds: None,
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

/// `--race`: the opponents of the event. Model decisions: the race's AI route is the road graph's shortest
/// path through the start and every gate of every lap (`route_through`); each grid slot drives the event's
/// allowed cars in turn (or the `--ai` cars), as the opponents' own cars (`AIPlayerInformation` `9b4e35f7` /
/// `2968e77a`, the `AIPlayerType`'s rollout) are not resolved yet. The rubber band is the event's
/// (`OfflineEvent` +0x94 / +0x98).
fn race_from_event(
    mut commands: Commands,
    event: Option<ResMut<crate::race::RaceState>>,
    mut opponents: ResMut<Opponents>,
    game: Res<crate::Game>,
    mut done: Local<bool>,
) {
    let Some(mut event) = event else { return };
    if *done {
        return;
    }
    *done = true;
    let graph = match game.store.ai_data() {
        Ok(g) => g,
        Err(e) => {
            warn!("AI: no road graph ({e}); no opponents");
            return;
        }
    };
    let waypoints: Vec<Vec3> = event.entries.iter().map(|e| e.position).collect();
    let Some(route) = route_through(&graph, &waypoints) else {
        warn!("AI: no road-graph path through the race's gates; no opponents");
        return;
    };
    let grid: Vec<Transform> = event.grid.iter().map(|g| g.transform).collect();
    if opponents.bundles.is_empty() {
        let cars: Vec<String> =
            event.event.allowed_vehicles.iter().map(|v| format!("VEHICLES/VEH_{}_HI.BNDL", v.vehicle)).collect();
        if cars.is_empty() {
            return;
        }
        opponents.bundles = (0..grid.len()).map(|i| cars[i % cars.len()].clone()).collect();
    } else {
        let cars = opponents.bundles.clone();
        opponents.bundles = (0..grid.len()).map(|i| cars[i % cars.len()].clone()).collect();
    }
    opponents.spawned = vec![false; opponents.bundles.len()];
    info!(
        "AI: race route {:.0} m through {} gates along the road graph, {} opponents",
        route.length(),
        waypoints.len() - 1,
        grid.len()
    );
    event.hold = !grid.is_empty();
    // The rubber band: `OpponentData[slot]` of the event's `RaceBalancingData`, with the pace multiplier of the
    // allowed-vehicle entry matching the player's car.
    let (data, profile) = event.event.balancing;
    let player_bundle = match &game.request {
        crate::loader::Request::Drive { bundle, .. } => bundle.to_ascii_uppercase(),
        _ => String::new(),
    };
    let pace = event
        .event
        .allowed_vehicles
        .iter()
        .find(|v| player_bundle.contains(&format!("VEH_{}_", v.vehicle)))
        .map_or(1.0, |v| v.ai_pace);
    let id = |u: u32| mw_bundle::ResourceId(0x0100_0000_0000_0000 | u as u64);
    let balancing: Vec<Option<RubberBand>> = (0..grid.len())
        .map(|k| {
            if data == 0 || profile == 0 {
                return None;
            }
            rubber_band_from_data(&game.store, id(data), id(profile), k, pace)
                .inspect(|rb| info!("AI: opponent {k} rubber band {rb:?}"))
                .inspect_err(|e| warn!("AI: no rubber band for opponent {k}: {e:#}"))
                .ok()
        })
        .collect();
    commands.insert_resource(RaceSetup { route: Arc::new(route), grid, balancing });
}

/// Build the test race when there is none: once the player's car is on the road, a route along the AI road
/// graph from it and grid slots ahead of it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn setup_race(
    mut commands: Commands,
    race: Option<Res<RaceSetup>>,
    opponents: Res<Opponents>,
    game: Res<crate::Game>,
    test: Option<Res<AiTest>>,
    player: Query<(Entity, &Transform), (With<Player>, With<Car>)>,
    balancing: Option<Res<AiBalancingArg>>,
    event: Option<Res<crate::race::RaceState>>,
    mut done: Local<bool>,
) {
    if *done || race.is_some() || event.is_some() || (opponents.bundles.is_empty() && test.is_none()) {
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
        let req = CarRequest { bundle, paint_index: slot + 1, traffic: None, opponent: Some(slot) };
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
    mut event: Option<ResMut<crate::race::RaceState>>,
    time: Res<Time>,
    spatial: SpatialQuery,
) {
    let Some(race) = race else { return };
    // `--race`: hold the start until every opponent is on the grid (or 60 s have passed: a car failed to load).
    if let Some(ev) = event.as_mut()
        && ev.hold
        && (opponents.spawned.iter().all(|&s| s) || time.elapsed_secs() > 60.0)
    {
        ev.hold = false;
        info!("AI: {} opponents on the grid; the race may start", opponents.spawned.iter().filter(|&&s| s).count());
    }
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
                nitrous_rules: None,
                start,
                heading: fwd.x.atan2(fwd.z),
                velocity: Vec3::ZERO,
                damage: Some(crate::damage::DamageSource(spec.damage.clone().map(|d| (d, spec.impact_graph)))),
            },
        );
        if let Some(engine) = &spec.competitor_engine {
            commands.entity(car).insert(crate::car_audio::PendingCompetitorEngine(engine.clone()));
        }
        let mut driver = AiDriver::new(race.route.clone());
        driver.rubber_band = race.balancing.get(slot).copied().flatten();
        commands.entity(car).insert(driver);
        if event.is_some() {
            let name = spec.bundle.rsplit('/').next().unwrap_or(&spec.bundle).trim_end_matches(".BNDL").to_string();
            let ext = crate::drive::CarParams::new(&spec.handling).body_half_extents;
            commands
                .entity(car)
                .insert((crate::race::Racer::ai(name, slot, ext), crate::race::RacerProgress::default()));
        }
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
    event: Option<Res<crate::race::RaceState>>,
    progress: Query<&crate::race::RacerProgress>,
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
    for (entity, mut car, mut t, mut v, mut w, ai, mut ctl, is_player) in &mut cars {
        let Some(mut ai) = ai else { continue };
        let ai = &mut *ai;
        // Before go the race freezes every car (`FREEZE_CAR`, `FUN_006c1a80`: handbrake held).
        // An intro sequence's rolling start (`cutscene.rs`) lets them drive.
        if event.as_ref().is_some_and(|e| {
            matches!(e.phase, crate::race::RacePhase::Waiting | crate::race::RacePhase::Intro) && !e.rolling_start
        }) {
            *ctl = CarControls { handbrake: true, ..default() };
            continue;
        }
        // A racer that has finished gets AI goal 3, stop racing (`FUN_00843030`): ours brakes to a stop.
        // (Not the player's car: after its finish the outro sequence drives it, `cutscene.rs`.)
        if !is_player && progress.get(entity).is_ok_and(|p| p.finished.is_some()) {
            *ctl = CarControls { brake: 1.0, ..default() };
            continue;
        }
        ai.time += dt;
        // The first projection searches the route's start only: a race route repeats its laps.
        let (hint, window) = if ai.time > dt * 1.5 { (Some(ai.progress), 30.0) } else { (Some(0.0), 300.0) };
        let (s, lateral) = ai.route.project(t.translation, hint, window);
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
        let corner = corner_speed(ai, s, v.0.length(), &car, &settings.0);
        let input = crate::ai_driver::DriverInput {
            dt,
            route: &ai.route,
            progress: s,
            position: t.translation,
            forward: fwd,
            velocity: v.0,
            radius: ext.x.max(ext.z),
            steer_gains: car.params.ai_steer_pid,
            schedule_speed: match ai.rubber_band {
                Some(rb) if player.is_some_and(|(e, _)| e != entity) => {
                    let gap = player.map_or(0.0, |(_, p)| p.distance(t.translation));
                    // In a race: the race tracker's distances to the finish (`FUN_0063ef30`, `FUN_00654110`);
                    // otherwise both cars' progress along the AI route.
                    let race = event.as_ref().zip(player).and_then(|(ev, (pe, _))| {
                        let (me, them) = (progress.get(entity).ok()?, progress.get(pe).ok()?);
                        let total = ev.total_distance().max(1.0);
                        Some(((1.0 - me.to_finish / total).clamp(0.0, 1.0), them.to_finish - me.to_finish))
                    });
                    let (x, lead) = race.unwrap_or_else(|| {
                        let ps = player_progress(&ai.route).unwrap_or(0.0);
                        ((s / ai.route.length().max(1.0)).clamp(0.0, 1.0), s - ps)
                    });
                    ai.schedule_speed = rb.speed(x, lead, gap);
                    ai.schedule_speed
                }
                _ => ai.schedule_speed,
            },
            corner_speed: corner,
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

/// The corner speed limit at distance `s`. The game's physical racer has none (its target speed is the rubber
/// band, lowered only by obstacles; the node speeds of `ai_driver::corner_speeds` feed only the virtual
/// progress of a racer without a car), so this is `f32::MAX` unless `MW_AI_CORNER_SPEED=1` asks for the model
/// decision: the game's uncalled braking look-ahead (0x004ff480 → `FUN_004e5270`): of the nodes within the
/// stopping distance whose corner speed is below the current speed, the lowest one whose distance is within the
/// braking distance to its speed (`FUN_00518520`, here without its aero term: a = 0, b = μ g).
fn corner_speed(ai: &mut AiDriver, s: f32, speed: f32, car: &Car, cfg: &crate::ai_driver::AiConfig) -> f32 {
    if !*CORNER_SPEED_RULE {
        return f32::MAX;
    }
    let mu = car.params.tyres[1].pure_lateral.peak;
    let speeds = ai.corner_speeds.get_or_insert_with(|| {
        let pts: Vec<Vec2> = ai.route.nodes.iter().map(|n| n.position).collect();
        let turn: Vec<f32> = ai.route.nodes.iter().map(|n| n.turn).collect();
        // A race path's nodes have 5 m half-widths (`.rdata` 0x00fa8538): a full width of 10 m.
        crate::ai_driver::corner_speeds(&pts, &turn, 10.0, car.params.top_speed, mu, cfg.corner_speed_scale)
    });
    let b = mu * 9.81;
    let stop = crate::ai_driver::braking_distance(speed, 0.0, 0.0, b);
    let mut limit = f32::MAX;
    for (n, &v) in ai.route.nodes.iter().zip(speeds.iter()) {
        let d = n.s - s;
        if d <= 0.0 || d > stop {
            continue;
        }
        if v < speed && d <= crate::ai_driver::braking_distance(speed, v, 0.0, b) {
            limit = limit.min(v);
        }
    }
    limit
}

static CORNER_SPEED_RULE: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("MW_AI_CORNER_SPEED").is_ok_and(|v| v == "1"));

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
                "TEST RESULT AI {}{}: {:.0} m in {:.1} s ({:.1} km/h mean), finished {:?}, max lateral {:.2} m, {} world hits, {} resets, 100 m splits {:?}",
                name,
                if player { " (player car)" } else { "" },
                ai.progress,
                ai.time,
                ai.progress / ai.time.max(1e-3) * 3.6,
                ai.finished,
                ai.max_lateral,
                ai.hits,
                ai.resets,
                ai.splits.iter().map(|s| (s * 10.0).round() / 10.0).collect::<Vec<_>>(),
            );
        }
        exit.write(AppExit::Success);
    }
}
