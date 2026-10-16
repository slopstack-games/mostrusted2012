//! Navigation (GPS): the map's SET DESTINATION, the route over the AI road graph to it, the route on the minimap
//! and the world map, re-routing when the car leaves it and clearing it on arrival. Ported from the original's
//! UI map manager (docs/decomp.md "Navigation: SET DESTINATION and the GPS route"):
//!
//! - **Setting** (`Map:OnSetWaypoint` → `FUN_0080eb60`): an event (item types 1, 9, 10) routes to its start
//!   (`FUN_00796ff0`: the event's start trigger position), anything else to the item's position (`FUN_008091d0`:
//!   the trigger's position). The route goes from the local player's position (`FUN_006e02f0`) with its velocity.
//! - **Request** (`FUN_00796df0`): when the velocity is not zero, the start is moved onto the road graph two nodes
//!   ahead along it (`FUN_004375b0` nearest node, then `FUN_00436c00` twice); then an A* path request, type 9, to
//!   the destination (`FUN_00843240`; the search is the AI's, `FUN_00527300`, over the `AIData` graph).
//! - **Line** (`FUN_0079b890`, `FUN_0079e4f0`): the path's points, every ((count >> 9) + 1)th so at most 512, each
//!   with the node's drivable width (`AIData` node +0x25 × 0.25 m), its distance along / the path length (u) and a
//!   side vector normalise(−dz, 0, dx) of the neighbours' direction.
//! - **Following** (`FUN_007a0f20`, every frame): every 0.5 s (`.rdata` 0x00f8a810) the car is projected onto the
//!   path (`FUN_005232f0`, nearest point in 3D): progress = along / length, the remaining distance (the
//!   `UIMapManager.RouteDistance` binding) = length − along; at 15 m or more from that point (`.rdata`
//!   0x00fd6b48 = 225, squared) the route is requested again from the car. Each frame the progress is refined
//!   on the line: the nearest line point within ±50 m of the progress (`.rdata` 0x00fa9ed8), heights weighted by
//!   1.25 (`.rdata` 0x00fe0f90), projected onto its nearer neighbouring segment on the ground plane.
//! - **Arrival** (`FUN_007a2aa0` → `FUN_00673770`): within 5 m on the ground plane of the destination (`.data`
//!   0x010c61c8 = 25, squared, its initial value) every route is cleared. The event code clears them too
//!   (`FUN_00673770`'s callers 0x0075e100 / 0x00761d70 / 0x00781b70 / 0x00787c20): here when a race starts.
//!
//! Model decisions: the path is `AiData::path` (A* on node link lengths; the game's junction search adds a turn
//! penalty 5·max(0, 0.98 − cos Δ)³ × route-option factors whose angle reference was not resolved) and its points
//! are `ai::spline_through`'s ~2 m spline samples (the game's own path expansion `FUN_00520a50` was not read);
//! a failed search clears the destination (the game asks again); the path is computed on a thread as the game's
//! `AStarWorker` does. How the line is drawn was not found in the dump (the strip / ribbon renderer reading the
//! points), so its look is ours: a light-blue band of the road's drivable width (at least 2 px on the minimap,
//! 3 px on the world map), from the car's progress on.
//!
//! `MW_GPS_LOG=1` logs requests, routes and re-routes.

use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use mw_formats::AiData;

/// The 0.5 s between route checks (`FUN_00792390` resets the timer to it, `FUN_007a0f20` reloads it).
const CHECK_INTERVAL: f32 = 0.5;
/// Re-route when the car is this far (m) from its nearest point on the path (√225).
const OFF_ROUTE: f32 = 15.0;
/// Arrival radius (m) on the ground plane (√25).
const ARRIVAL: f32 = 5.0;
/// The progress search window along the line (m) and the height weight.
const PROGRESS_WINDOW: f32 = 50.0;
const HEIGHT_WEIGHT: f32 = 1.25;
/// At most this many line points (`FUN_0079b890`'s decimation `(count >> 9) + 1`).
const MAX_POINTS: usize = 512;
/// MODEL: the line's look (the renderer was not found).
pub const LINE_COLOUR: [f32; 4] = [0.35, 0.75, 1.0, 0.9];

pub struct GpsPlugin;

impl Plugin for GpsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SetDestination>().init_resource::<Gps>().add_systems(Update, (request, poll, follow).chain());
    }
}

/// SET DESTINATION: route to `position` (an event's start or a collectable's trigger).
#[derive(Message, Clone, Copy, Debug)]
pub struct SetDestination {
    pub position: Vec3,
    /// The event routed to (`UIMapManager` +0x2c8e4), if any.
    pub event: Option<u32>,
}

/// One point of the route line (0x20 bytes in the game: position, side, width, u).
#[derive(Debug, Clone, Copy)]
pub struct RoutePoint {
    pub pos: Vec3,
    /// Across the route on the ground plane, unit length. Kept as the game builds it; its reader (the line
    /// renderer) was not found, our drawing measures distances to the segments instead.
    #[allow(dead_code)]
    pub side: Vec3,
    /// The road's drivable width (m).
    pub width: f32,
    /// Distance along / the path length, 0..1.
    pub u: f32,
}

#[derive(Debug, Clone, Default)]
pub struct GpsRoute {
    pub points: Vec<RoutePoint>,
    /// The path length (m).
    pub length: f32,
    /// The car's progress (u) and the distance left (m).
    pub progress: f32,
    pub remaining: f32,
}

type Job = Arc<Mutex<Option<Option<GpsRoute>>>>;

#[derive(Resource, Default)]
pub struct Gps {
    pub destination: Option<Vec3>,
    pub event: Option<u32>,
    pub route: Option<GpsRoute>,
    job: Option<Job>,
    timer: f32,
    graph: Arc<Mutex<Option<Arc<AiData>>>>,
    in_race: bool,
}

impl Gps {
    pub fn clear(&mut self) {
        self.destination = None;
        self.event = None;
        self.route = None;
        self.job = None;
    }

    /// The route as drawn: the points from the car's progress on (the line point just before it included).
    pub fn ahead(&self) -> &[RoutePoint] {
        let Some(r) = &self.route else { return &[] };
        let first = r.points.iter().position(|p| p.u >= r.progress).unwrap_or(r.points.len());
        &r.points[first.saturating_sub(1)..]
    }

    fn start(&mut self, store: Arc<mw_assets::AssetStore>, from: Vec3, velocity: Vec3) {
        let Some(to) = self.destination else { return };
        let job: Job = Arc::new(Mutex::new(None));
        let (out, graph) = (job.clone(), self.graph.clone());
        let log = std::env::var_os("MW_GPS_LOG").is_some();
        let spawned = std::thread::Builder::new().name("mw-gps".into()).spawn(move || {
            let g = {
                let mut slot = graph.lock().unwrap();
                if slot.is_none() {
                    match store.ai_data() {
                        Ok(g) => *slot = Some(Arc::new(g)),
                        Err(e) => warn!("GPS: no road graph ({e})"),
                    }
                }
                slot.clone()
            };
            let route = g.and_then(|g| plan(&g, from, velocity, to));
            if log {
                match &route {
                    Some(r) => info!(
                        "GPS: route of {} points, {:.0} m: {} … {} … {}",
                        r.points.len(),
                        r.length,
                        r.points[0].pos,
                        r.points[r.points.len() / 2].pos,
                        r.points[r.points.len() - 1].pos
                    ),
                    None => info!("GPS: no route from {from} to {to}"),
                }
            }
            *out.lock().unwrap() = Some(route);
        });
        if spawned.is_ok() {
            self.job = Some(job);
        }
    }
}

/// `FUN_00796df0`: the start moved two road nodes ahead along a non-zero velocity, then the path to `to`.
pub fn plan(graph: &AiData, from: Vec3, velocity: Vec3, to: Vec3) -> Option<GpsRoute> {
    let pos = |i: usize| Vec3::from(graph.nodes[i].position);
    let mut start = from;
    if velocity.abs().max_element() > f32::EPSILON {
        let dir = [velocity.x, velocity.z];
        let n0 = graph.nearest_node(from.to_array(), false)?;
        let n1 = graph.next_node(n0, dir, None).unwrap_or(n0);
        let n2 = graph.next_node(n1, dir, None).unwrap_or(n1);
        start = pos(n2);
    }
    let a = graph.nearest_node(start.to_array(), false)?;
    let b = graph.nearest_node(to.to_array(), false)?;
    let nodes = graph.path(a, b)?;
    let spline = crate::ai::spline_through(graph, &nodes)?;
    let widths: Vec<f32> = spline.half_width.iter().map(|h| 2.0 * h).collect();
    line(&spline.points, &widths)
}

/// `FUN_0079b890` / `FUN_0079e4f0`: every ((n >> 9) + 1)th sample and the last, u along the path, side vectors.
pub fn line(samples: &[Vec3], widths: &[f32]) -> Option<GpsRoute> {
    if samples.len() < 2 {
        return None;
    }
    let length: f32 = samples.windows(2).map(|w| w[0].distance(w[1])).sum();
    let step = (samples.len() >> 9) + 1;
    let mut keep: Vec<usize> = (0..samples.len()).step_by(step).collect();
    if keep.last() != Some(&(samples.len() - 1)) {
        keep.push(samples.len() - 1);
    }
    keep.truncate(MAX_POINTS);
    let mut points: Vec<RoutePoint> = Vec::with_capacity(keep.len());
    let mut along = 0.0;
    for (k, &i) in keep.iter().enumerate() {
        if k > 0 {
            along += samples[keep[k - 1]].distance(samples[i]);
        }
        let (prev, next) = (samples[keep[k.saturating_sub(1)]], samples[keep[(k + 1).min(keep.len() - 1)]]);
        let d = next - prev;
        let side = Vec3::new(-d.z, 0.0, d.x).normalize_or_zero();
        let u = if length > 0.0 { (along / length).clamp(0.0, 1.0) } else { 0.0 };
        points.push(RoutePoint { pos: samples[i], side, width: widths.get(i).copied().unwrap_or(0.0), u });
    }
    Some(GpsRoute { points, length, progress: 0.0, remaining: length })
}

impl GpsRoute {
    /// `FUN_005232f0`: the nearest point on the line (3D) and the distance along to it.
    pub fn nearest(&self, p: Vec3) -> (Vec3, f32) {
        let mut best = (f32::MAX, self.points.first().map_or(p, |q| q.pos), 0.0);
        let mut along = 0.0;
        for w in self.points.windows(2) {
            let (a, b) = (w[0].pos, w[1].pos);
            let ab = b - a;
            let len2 = ab.length_squared().max(1e-6);
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let q = a + ab * t;
            let d2 = q.distance_squared(p);
            if d2 < best.0 {
                best = (d2, q, along + ab.length() * t);
            }
            along += ab.length();
        }
        (best.1, best.2)
    }

    /// The per-frame progress (`FUN_007a0f20`'s second half): the nearest line point within ±50 m of the progress
    /// (heights × 1.25), projected onto the nearer neighbouring segment on the ground plane.
    pub fn refine(&mut self, p: Vec3) {
        let n = self.points.len();
        if n < 2 || self.length <= 0.0 {
            return;
        }
        let window = PROGRESS_WINDOW / self.length;
        let mut best: Option<(f32, usize)> = None;
        for (i, q) in self.points.iter().enumerate() {
            if (self.progress - q.u).abs() >= window {
                continue;
            }
            let d = q.pos - p;
            let d2 = d.x * d.x + d.z * d.z + (d.y * HEIGHT_WEIGHT).powi(2);
            if best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, i));
            }
        }
        let Some((_, i)) = best else { return };
        let flat = |v: Vec3| Vec2::new(v.x, v.z);
        let seg = |a: usize, b: usize| {
            let (pa, pb) = (flat(self.points[a].pos), flat(self.points[b].pos));
            let ab = pb - pa;
            let t = ((flat(p) - pa).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            ((pa + ab * t).distance_squared(flat(p)), t)
        };
        let (a, b, t) = if i == 0 {
            (0, 1, seg(0, 1).1)
        } else if i == n - 1 {
            (n - 2, n - 1, seg(n - 2, n - 1).1)
        } else {
            let (back, fwd) = (seg(i - 1, i), seg(i, i + 1));
            if back.0 < fwd.0 { (i - 1, i, back.1) } else { (i, i + 1, fwd.1) }
        };
        let (ua, ub) = (self.points[a].u, self.points[b].u);
        self.progress = ua + (ub - ua) * t;
    }
}

#[allow(clippy::type_complexity)]
fn request(
    mut sets: MessageReader<SetDestination>,
    mut gps: ResMut<Gps>,
    game: Res<crate::Game>,
    car: Query<(&GlobalTransform, Option<&LinearVelocity>), (With<crate::drive::Car>, With<crate::drive::Player>)>,
) {
    let Some(set) = sets.read().last().copied() else { return };
    gps.clear();
    gps.destination = Some(set.position);
    gps.event = set.event;
    gps.timer = CHECK_INTERVAL;
    if std::env::var_os("MW_GPS_LOG").is_some() {
        info!("GPS: destination {} (event {:?})", set.position, set.event);
    }
    if let Ok((xf, v)) = car.single() {
        gps.start(game.store.clone(), xf.translation(), v.map_or(Vec3::ZERO, |v| v.0));
    }
}

fn poll(mut gps: ResMut<Gps>) {
    let Some(done) = gps.job.as_ref().and_then(|j| j.lock().unwrap().take()) else { return };
    gps.job = None;
    match done {
        Some(route) => gps.route = Some(route),
        None => {
            warn!("GPS: no road route to the destination; cleared");
            gps.clear();
        }
    }
}

#[allow(clippy::type_complexity)]
fn follow(
    mut gps: ResMut<Gps>,
    game: Res<crate::Game>,
    time: Res<Time>,
    race: Option<Res<crate::race::RaceState>>,
    car: Query<(&GlobalTransform, Option<&LinearVelocity>), (With<crate::drive::Car>, With<crate::drive::Player>)>,
) {
    // An event starting clears the routes (`FUN_00673770` from the event code).
    let in_race = race.is_some();
    if in_race && !gps.in_race {
        gps.clear();
    }
    gps.in_race = in_race;
    let Some(dest) = gps.destination else { return };
    let Ok((xf, v)) = car.single() else { return };
    let p = xf.translation();
    if Vec2::new(p.x - dest.x, p.z - dest.z).length_squared() < ARRIVAL * ARRIVAL {
        if std::env::var_os("MW_GPS_LOG").is_some() {
            info!("GPS: arrived");
        }
        gps.clear();
        return;
    }
    if gps.job.is_some() {
        return;
    }
    let gps = &mut *gps;
    let Some(route) = gps.route.as_mut() else { return };
    let mut reroute = false;
    gps.timer -= time.delta_secs();
    if gps.timer < 0.0 && route.length > 0.0 {
        gps.timer = CHECK_INTERVAL;
        let (q, along) = route.nearest(p);
        route.remaining = route.length - along;
        reroute = q.distance_squared(p) >= OFF_ROUTE * OFF_ROUTE;
        if std::env::var_os("MW_GPS_LOG").is_some() {
            info!(
                "GPS: {:.0} m along, {:.1} m off, {:.0} m left, progress {:.3}",
                along,
                q.distance(p),
                route.remaining,
                route.progress
            );
        }
    }
    route.refine(p);
    if reroute {
        if std::env::var_os("MW_GPS_LOG").is_some() {
            info!("GPS: off the route, re-routing");
        }
        let velocity = v.map_or(Vec3::ZERO, |v| v.0);
        gps.start(game.store.clone(), p, velocity);
    }
}

/// Draw the route into the minimap's pixels, laid out as `MapMosaic::render` lays out the map (centred on
/// `centre`, `forward` up, `metres_per_px`).
pub fn draw_on_minimap(
    points: &[RoutePoint],
    centre: Vec3,
    forward: Vec3,
    metres_per_px: f32,
    w: u32,
    h: u32,
    px: &mut [u8],
) {
    if points.len() < 2 {
        return;
    }
    let up = Vec2::new(forward.x, forward.z).normalize_or(Vec2::Y);
    let right = Vec2::new(-up.y, up.x) * mw_assets::ui::MAP_RIGHT_SIGN;
    let to_px = |p: Vec3| {
        let d = Vec2::new(p.x - centre.x, p.z - centre.z);
        Vec2::new(d.dot(right) / metres_per_px + w as f32 * 0.5 - 0.5, h as f32 * 0.5 - 0.5 - d.dot(up) / metres_per_px)
    };
    let [r, g, b, a] = LINE_COLOUR;
    let colour = [r, g, b].map(|c| c.powf(1.0 / 2.2) * 255.0);
    for s in points.windows(2) {
        let (pa, pb) = (to_px(s[0].pos), to_px(s[1].pos));
        // Half the band: the mean drivable width over two, at least 1 px (a 2 px line).
        let half = ((s[0].width + s[1].width) * 0.25 / metres_per_px).max(1.0);
        let lo = pa.min(pb) - Vec2::splat(half + 1.0);
        let hi = pa.max(pb) + Vec2::splat(half + 1.0);
        if hi.x < 0.0 || hi.y < 0.0 || lo.x >= w as f32 || lo.y >= h as f32 {
            continue;
        }
        let ab = pb - pa;
        let len2 = ab.length_squared().max(1e-6);
        for y in lo.y.max(0.0) as u32..(hi.y.ceil() as u32).min(h) {
            for x in lo.x.max(0.0) as u32..(hi.x.ceil() as u32).min(w) {
                let q = Vec2::new(x as f32, y as f32);
                let t = ((q - pa).dot(ab) / len2).clamp(0.0, 1.0);
                let d = q.distance(pa + ab * t);
                // A one-pixel soft edge.
                let cover = (half + 0.5 - d).clamp(0.0, 1.0) * a;
                if cover <= 0.0 {
                    continue;
                }
                let i = ((y * w + x) * 4) as usize;
                if let Some(p) = px.get_mut(i..i + 3) {
                    for (c, &v) in p.iter_mut().zip(&colour) {
                        *c = (*c as f32 * (1.0 - cover) + v * cover) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(n: usize) -> GpsRoute {
        let samples: Vec<Vec3> = (0..n).map(|i| Vec3::new(0.0, 0.0, i as f32 * 2.0)).collect();
        line(&samples, &vec![10.0; n]).unwrap()
    }

    #[test]
    fn line_decimates_to_512_and_keeps_the_end() {
        let r = straight(2000);
        assert!(r.points.len() <= MAX_POINTS);
        assert_eq!(r.points.last().unwrap().pos, Vec3::new(0.0, 0.0, 3998.0));
        assert!((r.points.last().unwrap().u - 1.0).abs() < 1e-5);
        assert!((r.length - 3998.0).abs() < 1e-2);
        // Side vector of a +Z line: normalise(−dz, 0, dx) = −X.
        assert!((r.points[3].side - Vec3::NEG_X).length() < 1e-5);
    }

    #[test]
    fn nearest_and_progress() {
        let mut r = straight(101);
        let (q, along) = r.nearest(Vec3::new(3.0, 0.0, 50.5));
        assert!((along - 50.5).abs() < 1e-3 && (q.x).abs() < 1e-5);
        // Progress follows within the ±50 m window.
        for z in [10.0, 40.0, 80.0, 120.0, 160.0] {
            r.refine(Vec3::new(1.0, 0.0, z));
        }
        assert!((r.progress * r.length - 160.0).abs() < 0.5, "{}", r.progress * r.length);
    }
}
