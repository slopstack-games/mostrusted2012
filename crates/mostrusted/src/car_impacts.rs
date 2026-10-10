//! Car against car: whether a hit on traffic or another car is a crash, as the original decides it
//! (docs/decomp.md "Crashes", "Car against car"). The game's pair handler (`FUN_00960f20`, by the categories of
//! `FUN_00929350`: player, AI, crashing race car, traffic, other) computes a damage for each car from the impact
//! and tests it against the car's `CrashingRules` (`FUN_009a9fd0`):
//!
//! | pair | handler | damage | the first car's rule | the second car's rule |
//! |------|---------|--------|----------------------|-----------------------|
//! | player – traffic | `FUN_00959be0` | generic | `TrafficRules`, or `5ccf9942` when the traffic car has crashed | its `PlayerRules` |
//! | AI – traffic | `FUN_0095a680` | generic | the same | its "AI impact" (`22061460`) |
//! | traffic – traffic | `FUN_00958130` | generic | `TrafficRules` / `5ccf9942` | the same |
//! | player – AI | `FUN_009588c0` | race cars | "AI impact", only while an event runs (`FUN_006a0040`) | its `PlayerRules` |
//! | AI – AI | `FUN_009575e0` | race cars | "AI impact" | "AI impact" |
//!
//! - **Impact location** (`FUN_00929da0`): the contact point in the car's box, scaled to the unit box; front /
//!   rear beyond 0.7, sides beyond 0.5, roof / underside beyond 0.8 → one of 11 zones.
//! - **Generic damage** (`FUN_00941a50`): `ImpactDamageGraphs` graph 1 of the closing speed along the normal
//!   (5 mph → 0, 80 → 33, 160 → 100, not capped), × min(faster car's speed / 25 mph, 1), × the car's location
//!   scale (`DamageBarProfile`), × the other car's `Strength` over this car's defence stat (`FUN_0092ea20`).
//!   A glancing contact (the paths cross at more than 45° and the boxes overlap by less than 0.5 m across the
//!   path, `FUN_0093c0b0` → `FUN_0092e580`) caps the race car's damage at 4.
//! - **Race car damage** (`FUN_0093c240`): each car's weight is its speed × (1 + 0.1 for a front hit − the
//!   other's); graph 2 of the weighted relative velocity is split by `w cos(α / 2)` shares (α between a car's
//!   travel and the weighted relative velocity, `FUN_00924bb0`): a car takes the other's share.
//! - **Landing on a car** (`FUN_008fbb90`): a car in the air whose contact point lies inside 0.8 of its own
//!   box (x and z) on a grounded car crashes outright.
//!
//! What follows a crash: the player's car → the crash camera and reset (`crash.rs`); an AI car → no control,
//! then its reset ([`AiCrash`], `ai.rs`); a traffic car → wrecked (later hits are "crashing traffic impacts").
//!
//! Model decisions (docs/decomp.md "Car against car"): the per-location table at vehicle +0x16a0 is taken as
//! zero (it was zero on the player's car in both snapshots); the input-dependent extras of `FUN_0092e7b0`
//! (≤ 0.1, from vehicle +0x1bf0…+0x1c20) are left out; the component flag `05cd1d74` is taken as clear (it is on
//! the cars checked); a pair is tested when its contact starts and then not again for 0.5 s (`FUN_009575e0`
//! tests vehicle +0x1640 + 4·other ≥ 0.5, whose writer was not found); the glancing test uses the current poses
//! (the game predicts them one frame ahead); "grounded" for the landing rule is a wheel on the ground (the
//! game's vehicle +0x1630 ≥ 0.2 is not identified); the damage bar (`FUN_009b2530`: wrecked when used up) is not
//! kept, so only the rules decide.
//!
//! `MW_CAR_IMPACT_LOG=1` logs every evaluated impact.

use std::collections::{HashMap, HashSet};

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::crash::{CarImpactData, DamageProfile, DamageStats, ImpactRule};

use crate::ai::AiDriver;
use crate::drive::{Car, Player};
use crate::traffic::{TrafficCar, TrafficSystem};

/// m/s per mph (`.bss` 0x0118a010).
const MPH: f32 = 0.44704;
/// Below this closing speed (m/s) an impact does no damage (`DAT_01024720`).
const MIN_CLOSING: f32 = 0.25;
/// The front zones' bonus in the race-car weights (`DAT_01023b64`: 0.1 for zones 2, 3, 4).
const FRONT_BONUS: f32 = 0.1;
/// A glancing contact's damage cap (`DAT_01185d60`, read in a snapshot).
const GLANCING_CAP: f32 = 4.0;
/// `CollisionInfo` `226dddf1` (0.7) and `12236fc8` (0.5): the glancing test's angle and overlap.
const GLANCING_COS: f32 = 0.7;
const GLANCING_OVERLAP: f32 = 0.5;
/// The landing rule's footprint fraction (`DAT_010cf570`).
const LANDING_FOOTPRINT: f32 = 0.8;
/// MODEL: a pair is tested again only after this long (s).
const PAIR_COOLDOWN: f32 = 0.5;

pub struct CarImpactsPlugin;

impl Plugin for CarImpactsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingImpacts>().add_systems(
            FixedPostUpdate,
            (evaluate_impacts.before(crate::crash::detect_crash), remember_velocities)
                .chain()
                .after(PhysicsSystems::StepSimulation),
        );
    }
}

/// Impacts found outside the contact loop: a car touching a lane-following traffic car (`traffic.rs` turns it
/// into a crashing car; the contact hook drops that first contact).
#[derive(Resource, Default)]
pub struct PendingImpacts(pub Vec<RawImpact>);

/// One impact between two car bodies (root entities).
#[derive(Debug, Clone, Copy)]
pub struct RawImpact {
    pub a: Entity,
    pub b: Entity,
    /// Contact point (world).
    pub point: Vec3,
    /// Unit contact normal from `a` to `b`.
    pub normal: Vec3,
    /// Velocities before the impact.
    pub va: Vec3,
    pub vb: Vec3,
}

/// A body's velocity at the end of the last physics step: the "before" velocity of the next step's contacts.
#[derive(Component)]
struct StepVelocity(Vec3);

/// An AI car that crashed: it has no control until `ai.rs` resets it.
#[derive(Component, Debug, Clone)]
pub struct AiCrash {
    /// Seconds since the crash.
    pub time: f32,
    /// Speed before the crash (mph), for the rolling reset.
    pub pre_mph: f32,
    pub rule: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Player,
    Ai,
    Traffic { wrecked: bool },
}

/// One side of an impact.
struct Party<'a> {
    entity: Entity,
    kind: Kind,
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    /// The physical box in car space.
    centre: Vec3,
    half: Vec3,
    stats: DamageStats,
    profile: &'a DamageProfile,
    grounded: bool,
    /// Already crashed: its rules do not fire again (vehicle +0x1774).
    crashed: bool,
}

/// Damage to each car (`dmg`) and before the strength factor (`base`, the rules' third threshold).
#[derive(Debug, Default, Clone, Copy)]
struct Damage {
    a: f32,
    base_a: f32,
    b: f32,
    base_b: f32,
}

/// The impact zone (`FUN_00929da0`): 0 roof, 1 underside, 2 / 4 front corners, 3 front, 5 / 9 sides, 6 / 8 rear
/// corners, 7 rear, 10 centre.
fn location(p: &Party, point: Vec3) -> u8 {
    let local = p.rotation.inverse() * (point - p.position) - p.centre;
    let q = local / p.half.max(Vec3::splat(1e-3));
    let m = q.abs().max_element();
    if m.is_nan() || m <= f32::EPSILON {
        return 10;
    }
    let (x, y, z) = (q.x / m, q.y / m, q.z / m);
    let (front, rear) = (z > 0.7, z < -0.7);
    let under = y < -0.8 && y.abs() > 0.8 + (x.abs() - 0.5) * 2.0 * 0.2;
    if x > 0.5 {
        if front {
            2
        } else if rear {
            8
        } else if under {
            1
        } else {
            9
        }
    } else if x >= -0.5 {
        if front {
            3
        } else if rear {
            7
        } else if y > 0.8 {
            0
        } else if y < -0.8 {
            1
        } else {
            10
        }
    } else if front {
        4
    } else if rear {
        6
    } else if under {
        1
    } else {
        5
    }
}

/// The strength factors (`FUN_0092ea20`): the other car's `Strength` over this car's defence (the scale the
/// handler passes; the per-location table is zero, the `05cd1d74` flag clear).
fn strength_factors(a: &Party, b: &Party, scale_a: f32, scale_b: f32) -> (f32, f32) {
    (b.stats.strength / scale_a.max(1e-3), a.stats.strength / scale_b.max(1e-3))
}

/// `FUN_00941a50`: every pair but race car against race car.
#[allow(clippy::too_many_arguments)]
fn generic_damage(
    data: &CarImpactData,
    a: &Party,
    b: &Party,
    normal: Vec3,
    loc_a: u8,
    loc_b: u8,
    scale_a: f32,
    scale_b: f32,
) -> Damage {
    let closing = if normal.length_squared() > 0.0 {
        (a.velocity - b.velocity).dot(normal)
    } else {
        (b.velocity - a.velocity).length()
    };
    let base = if closing >= MIN_CLOSING { data.graphs.generic.damage(closing) } else { 0.0 };
    if base.abs() <= f32::EPSILON {
        return Damage::default();
    }
    let (fa, fb) = strength_factors(a, b, scale_a, scale_b);
    let reference = (data.graphs.reference_mph * MPH).max(0.5);
    let speed = (a.velocity.length().max(b.velocity.length()) / reference).min(1.0);
    let base_a = speed * a.profile.location_scale(loc_a) * base;
    let base_b = speed * b.profile.location_scale(loc_b) * base;
    Damage { a: base_a * fa, base_a, b: base_b * fb, base_b }
}

/// `FUN_0093c240`: race car against race car.
fn race_damage(data: &CarImpactData, a: &Party, b: &Party, loc_a: u8, loc_b: u8, scale_a: f32, scale_b: f32) -> Damage {
    let (sa, sb) = (a.velocity.length(), b.velocity.length());
    let da = if sa >= 1e-5 { a.velocity / sa } else { Vec3::ZERO };
    let db = if sb >= 1e-5 { b.velocity / sb } else { Vec3::ZERO };
    if (a.velocity - b.velocity).length() < MIN_CLOSING {
        return Damage::default();
    }
    let bonus = |loc: u8| if matches!(loc, 2..=4) { FRONT_BONUS } else { 0.0 };
    let (ca, cb) = (bonus(loc_a), bonus(loc_b));
    let wa = (ca - cb + 1.0) * sa;
    let wb = (cb - ca + 1.0) * sb;
    let w = da * wa - db * wb;
    let m = w.length();
    let wd = if m > 0.0 { w / m } else { Vec3::ZERO };
    // `FUN_00924bb0`: shares w·cos(α/2), α = the angle between a car's travel and ±the weighted relative velocity.
    let share = |weight: f32, cos: f32| {
        let angle = if weight.abs() < 1e-5 { 0.0 } else { cos.clamp(-1.0, 1.0).acos() };
        (angle * 0.5).cos() * weight
    };
    let (pa, pb) = (share(wa, wd.dot(da)), share(wb, -wd.dot(db)));
    let sum = pa + pb;
    if sum.is_nan() || sum == 0.0 {
        return Damage::default();
    }
    let base = data.graphs.race_cars.damage(m);
    if base.abs() <= 0.1 {
        return Damage::default();
    }
    let (share_a, share_b) = (pa / sum * base, pb / sum * base);
    let (fa, fb) = strength_factors(a, b, scale_a, scale_b);
    let reference = data.graphs.reference_mph * MPH;
    let base_a = (wb / reference).min(1.0) * a.profile.location_scale(loc_a) * share_b;
    let base_b = (wa / reference).min(1.0) * b.profile.location_scale(loc_b) * share_a;
    Damage { a: base_a * fa, base_a, b: base_b * fb, base_b }
}

/// A car's motion frame (`FUN_009ae1c0`): forward along the velocity (the body's forward when nearly at rest),
/// right = up × forward.
fn motion_frame(p: &Party) -> (Vec3, Vec3) {
    let fwd = p.velocity.try_normalize().unwrap_or(p.rotation * Vec3::Z);
    let right = (p.rotation * Vec3::Y).cross(fwd).try_normalize().unwrap_or(p.rotation * Vec3::X);
    (fwd, right)
}

fn corners(p: &Party) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        let s = Vec3::new(
            if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
        );
        p.position + p.rotation * (p.centre + p.half * s)
    })
}

/// `FUN_0093c0b0` → `FUN_0092e580` both ways: the second car's path crosses the first's right axis at more than
/// 45° and, across the second car's path, the two boxes overlap by less than 0.5 m.
fn glancing(a: &Party, b: &Party) -> bool {
    let test = |p: &Party, q: &Party| {
        let (_, right_p) = motion_frame(p);
        let (fwd_q, right_q) = motion_frame(q);
        if fwd_q.dot(right_p).abs() > GLANCING_COS {
            return false;
        }
        let span = |c: [Vec3; 8]| {
            c.iter().map(|v| v.dot(right_q)).fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
        };
        let ((min_p, max_p), (min_q, max_q)) = (span(corners(p)), span(corners(q)));
        max_q - GLANCING_OVERLAP < min_p || max_p < min_q + GLANCING_OVERLAP
    };
    test(a, b) || test(b, a)
}

/// `FUN_008fbb90`: `a` in the air comes down on grounded `b` with the contact inside 0.8 of its own footprint.
fn lands_on(a: &Party, b: &Party, point: Vec3) -> bool {
    if a.crashed || b.crashed || a.grounded || !b.grounded {
        return false;
    }
    let local = a.rotation.inverse() * (point - a.position) - a.centre;
    local.x.abs() <= a.half.x * LANDING_FOOTPRINT && local.z.abs() <= a.half.z * LANDING_FOOTPRINT
}

/// The outcome for one car of a pair.
#[derive(Debug, Clone, Copy)]
struct Verdict {
    crash: bool,
    rule: &'static str,
    damage: f32,
}

fn test_rule(
    rule: Option<ImpactRule>,
    name: &'static str,
    party: &Party,
    closing: f32,
    damage: f32,
    base: f32,
) -> Verdict {
    // `FUN_009a9fd0`: a crashed car (or one flagged never to crash) does not crash again.
    let crash = !party.crashed
        && rule.is_some_and(|r| r.is_crash(Some(damage), closing / MPH) || r.third.is_some_and(|t| base > t));
    Verdict { crash, rule: name, damage }
}

fn traffic_rule<'a>(profile: &DamageProfile, other: &Party<'a>) -> (Option<ImpactRule>, &'static str) {
    match other.kind {
        Kind::Traffic { wrecked: true } => (profile.rules.crashing_traffic(), "crashing traffic impact"),
        _ => (profile.rules.traffic, "traffic impact"),
    }
}

/// Decide one impact: (verdict for a, verdict for b, the damages, the zones).
fn decide(
    data: &CarImpactData,
    a: &Party,
    b: &Party,
    imp: &RawImpact,
    event: bool,
) -> (Verdict, Verdict, Damage, (u8, u8)) {
    let (loc_a, loc_b) = (location(a, imp.point), location(b, imp.point));
    let closing = (a.velocity - b.velocity).dot(imp.normal).max(0.0);
    let none = Verdict { crash: false, rule: "", damage: 0.0 };
    let (mut va, mut vb, dmg) = match (a.kind, b.kind) {
        (Kind::Player | Kind::Ai, Kind::Traffic { .. }) => {
            let ai = a.kind == Kind::Ai;
            let (sa, sb) = if ai { (a.stats.s15f4, b.stats.s15f8) } else { (a.stats.s15f4, b.stats.s15fc) };
            let mut d = generic_damage(data, a, b, imp.normal, loc_a, loc_b, sa, sb);
            if glancing(a, b) {
                d.a = d.a.min(GLANCING_CAP);
            }
            let (rule, name) = traffic_rule(a.profile, b);
            let va = test_rule(rule, name, a, closing, d.a, d.base_a);
            let vb = if ai {
                test_rule(b.profile.rules.ai(), "AI impact", b, closing, d.b, d.base_b)
            } else {
                test_rule(b.profile.rules.player, "player impact", b, closing, d.b, d.base_b)
            };
            (va, vb, d)
        }
        (Kind::Traffic { .. }, Kind::Traffic { .. }) => {
            let d = generic_damage(data, a, b, imp.normal, loc_a, loc_b, a.stats.s15f4, b.stats.s15f4);
            let (ra, na) = traffic_rule(a.profile, b);
            let (rb, nb) = traffic_rule(b.profile, a);
            (test_rule(ra, na, a, closing, d.a, d.base_a), test_rule(rb, nb, b, closing, d.b, d.base_b), d)
        }
        (Kind::Player, Kind::Ai) => {
            let d = race_damage(data, a, b, loc_a, loc_b, a.stats.s15f8, b.stats.s15fc);
            let mut va = test_rule(a.profile.rules.ai(), "AI impact", a, closing, d.a, d.base_a);
            // `FUN_006a0040`: the player's crash only counts while an event runs.
            va.crash &= event;
            (va, test_rule(b.profile.rules.player, "player impact", b, closing, d.b, d.base_b), d)
        }
        (Kind::Ai, Kind::Ai) => {
            let d = race_damage(data, a, b, loc_a, loc_b, a.stats.s15f8, b.stats.s15f8);
            (
                test_rule(a.profile.rules.ai(), "AI impact", a, closing, d.a, d.base_a),
                test_rule(b.profile.rules.ai(), "AI impact", b, closing, d.b, d.base_b),
                d,
            )
        }
        // The other orders are swapped by the caller.
        _ => (none, none, Damage::default()),
    };
    // Landing on a car (`FUN_00901160` tries a on b, then b on a).
    if matches!(a.kind, Kind::Player | Kind::Ai) && lands_on(a, b, imp.point) {
        va = Verdict { crash: true, rule: "landing on a car", damage: va.damage };
    } else if matches!(b.kind, Kind::Player | Kind::Ai) && lands_on(b, a, imp.point) {
        vb = Verdict { crash: true, rule: "landing on a car", damage: vb.damage };
    }
    (va, vb, dmg, (loc_a, loc_b))
}

fn rank(k: Kind) -> u8 {
    match k {
        Kind::Player => 0,
        Kind::Ai => 1,
        Kind::Traffic { .. } => 3,
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn evaluate_impacts(
    mut commands: Commands,
    time: Res<Time>,
    config: Option<Res<crate::crash::CrashConfig>>,
    mut pending: ResMut<PendingImpacts>,
    collisions: Collisions,
    collider_of: Query<&ColliderOf>,
    cars: Query<(&Car, &Transform, &LinearVelocity, Option<&StepVelocity>, Has<Player>, Has<AiDriver>, Has<AiCrash>)>,
    traffic_cars: Query<(&TrafficCar, &Transform, &LinearVelocity, Option<&StepVelocity>)>,
    mut traffic: Option<ResMut<TrafficSystem>>,
    race: Option<Res<crate::race::RaceState>>,
    mut crash: ResMut<crate::crash::CrashState>,
    mut cooldown: Local<HashMap<(Entity, Entity), f32>>,
    mut touching: Local<HashSet<(Entity, Entity)>>,
) {
    let Some(config) = config else {
        pending.0.clear();
        return;
    };
    let Some(data) = config.0.impacts.as_ref() else {
        pending.0.clear();
        return;
    };
    let now = time.elapsed_secs();
    cooldown.retain(|_, t| now - *t < PAIR_COOLDOWN);

    // The impacts: lane-follower hits from the hook, and contacts that started this step between car bodies.
    let mut impacts: Vec<RawImpact> = std::mem::take(&mut pending.0);
    let is_car = |e: Entity| cars.contains(e) || traffic_cars.contains(e);
    let step_velocity = |e: Entity| -> Option<Vec3> {
        if let Ok((_, _, v, sv, ..)) = cars.get(e) {
            return Some(sv.map_or(v.0, |s| s.0));
        }
        traffic_cars.get(e).ok().map(|(_, _, v, sv)| sv.map_or(v.0, |s| s.0))
    };
    // Car pairs touching now; a pair not touching on the last step is a new impact (avian clears its own
    // "started touching" flag within the step).
    let mut seen: HashSet<(Entity, Entity)> = HashSet::new();
    for pair in collisions.iter() {
        if !pair.is_touching() {
            continue;
        }
        let (Ok(c1), Ok(c2)) = (collider_of.get(pair.collider1), collider_of.get(pair.collider2)) else { continue };
        let (a, b) = (c1.body, c2.body);
        if a == b || !is_car(a) || !is_car(b) || !seen.insert((a.min(b), a.max(b))) {
            continue;
        }
        if touching.contains(&(a.min(b), a.max(b))) {
            continue;
        }
        let Some(m) = pair.manifolds.first() else { continue };
        let Some(p) = m.points.first() else { continue };
        let (Some(va), Some(vb)) = (step_velocity(a), step_velocity(b)) else { continue };
        impacts.push(RawImpact { a, b, point: p.point, normal: m.normal, va, vb });
    }
    *touching = seen;
    if impacts.is_empty() {
        return;
    }

    let player_crashed = crash.active.as_ref().is_some_and(|a| !a.reset_done);
    let party = |e: Entity, v: Vec3| -> Option<Party> {
        if let Ok((car, t, _, _, is_player, is_ai, ai_crashed)) = cars.get(e) {
            let kind = if is_player {
                Kind::Player
            } else if is_ai {
                Kind::Ai
            } else {
                return None;
            };
            let stats = car.params.damage_stats;
            return Some(Party {
                entity: e,
                kind,
                position: t.translation,
                rotation: t.rotation,
                velocity: v,
                centre: car.params.centre_of_mass,
                half: car.params.body_half_extents,
                stats: if is_player { stats.offline } else { stats.ai },
                profile: if is_player { &data.player } else { &data.ai },
                grounded: car.wheels_down() > 0,
                crashed: if is_player { player_crashed } else { ai_crashed },
            });
        }
        let (tc, t, _, _) = traffic_cars.get(e).ok()?;
        let sys = traffic.as_deref()?;
        let vt = sys.vehicles().get(tc.vehicle)?;
        Some(Party {
            entity: e,
            kind: Kind::Traffic { wrecked: sys.wrecked(e).unwrap_or(false) },
            position: t.translation,
            rotation: t.rotation,
            velocity: v,
            centre: Vec3::from(vt.box_centre),
            half: Vec3::from(vt.half_extents),
            stats: vt.damage_stats.map(|s| s.offline).unwrap_or_default(),
            profile: &data.traffic,
            grounded: true,
            crashed: sys.wrecked(e).unwrap_or(false),
        })
    };
    let event = race.is_some();
    let log = std::env::var_os("MW_CAR_IMPACT_LOG").is_some();
    let mut outcomes: Vec<(Entity, Kind, Verdict, f32, f32)> = Vec::new();
    for imp in impacts {
        let key = (imp.a.min(imp.b), imp.a.max(imp.b));
        if cooldown.contains_key(&key) {
            continue;
        }
        let (Some(pa), Some(pb)) = (party(imp.a, imp.va), party(imp.b, imp.vb)) else { continue };
        cooldown.insert(key, now);
        // The handlers take the lower category first.
        let (a, b, imp) = if rank(pa.kind) <= rank(pb.kind) {
            (pa, pb, imp)
        } else {
            (pb, pa, RawImpact { a: imp.b, b: imp.a, normal: -imp.normal, va: imp.vb, vb: imp.va, ..imp })
        };
        let (va, vb, dmg, (la, lb)) = decide(data, &a, &b, &imp, event);
        let closing = (a.velocity - b.velocity).dot(imp.normal).max(0.0);
        if log || va.crash || vb.crash {
            info!(
                "car impact: {:?} zone {la} vs {:?} zone {lb}, closing {:.1} mph, speeds {:.0} / {:.0} mph, damage {:.1} / {:.1} (base {:.1} / {:.1}){}{}",
                a.kind,
                b.kind,
                closing / MPH,
                a.velocity.length() / MPH,
                b.velocity.length() / MPH,
                dmg.a,
                dmg.b,
                dmg.base_a,
                dmg.base_b,
                if va.crash { format!(", first CRASHES ({})", va.rule) } else { String::new() },
                if vb.crash { format!(", second CRASHES ({})", vb.rule) } else { String::new() },
            );
        }
        for (p, v) in [(&a, va), (&b, vb)] {
            if v.crash {
                outcomes.push((p.entity, p.kind, v, closing / MPH, p.velocity.length() / MPH));
            }
        }
    }
    for (entity, kind, v, closing_mph, pre_mph) in outcomes {
        match kind {
            Kind::Player => {
                crash.car_crash = Some(crate::crash::CarCrash { closing_mph, damage: v.damage, rule: v.rule });
            }
            Kind::Ai => {
                commands.entity(entity).insert(AiCrash { time: 0.0, pre_mph, rule: v.rule });
            }
            Kind::Traffic { .. } => {
                if let Some(sys) = traffic.as_deref_mut() {
                    sys.wreck(entity);
                }
            }
        }
    }
}

/// Keep every car body's velocity for the next step's contacts.
#[allow(clippy::type_complexity)]
fn remember_velocities(
    mut commands: Commands,
    bodies: Query<(Entity, &LinearVelocity, Option<&mut StepVelocity>), Or<(With<Car>, With<TrafficCar>)>>,
) {
    for (e, v, sv) in bodies {
        match sv {
            Some(mut sv) => sv.0 = v.0,
            None => {
                commands.entity(e).insert(StepVelocity(v.0));
            }
        }
    }
}

/// `--test crash-traffic` / `--test crash-ai`: put the player's car on a collision course with a lane-following
/// traffic car (head on, in its lane) or the first AI car (into its side), then log the impact's verdicts and
/// take screenshots of what follows.
#[derive(Resource)]
pub struct ImpactTest {
    kind: ImpactTestKind,
    out: Option<std::path::PathBuf>,
    phase: u8,
    timer: f32,
    shots: usize,
    target: Option<Entity>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImpactTestKind {
    Traffic,
    Ai,
    /// Into the front of the nearest parked car with a model (`traffic.rs`).
    Parked,
}

impl ImpactTest {
    pub fn new(kind: ImpactTestKind, out: Option<std::path::PathBuf>) -> Self {
        Self { kind, out, phase: 0, timer: 0.0, shots: 0, target: None }
    }

    fn path(&self, suffix: &str) -> Option<std::path::PathBuf> {
        let out = self.out.as_ref()?;
        let stem = out.file_stem()?.to_string_lossy();
        let ext = out.extension().map_or("png".into(), |e| e.to_string_lossy());
        Some(out.with_file_name(format!("{stem}-{suffix}.{ext}")))
    }

    fn shoot(&self, commands: &mut Commands, suffix: &str) {
        if let Some(p) = self.path(suffix) {
            info!("TEST {:?}: screenshot {suffix}", self.kind);
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(p));
        }
    }
}

pub struct ImpactTestPlugin;

impl Plugin for ImpactTestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, run_impact_test);
    }
}

/// Speed of the player's car on the collision course (m/s; about 67 / 78 mph).
const TEST_SPEED_TRAFFIC: f32 = 30.0;
const TEST_SPEED_AI: f32 = 35.0;
/// The AI car is placed this far ahead of the player's car (m), across the road at this speed (m/s).
const TEST_AI_AHEAD: f32 = 20.0;
const TEST_AI_ROLL: f32 = 0.0;
/// The player's car starts this far in front of a parked car (m), at this speed (m/s; about 27 mph).
const TEST_PARKED_AHEAD: f32 = 10.0;
/// A shot this long after the setup (s), to see the parked car moving.
const TEST_PARKED_SHOT: f32 = 1.5;
const TEST_SPEED_PARKED: f32 = 12.0;
/// Crash-camera screenshots (real seconds after the impact), as `--test crash` takes them.
const TEST_SHOTS: [(f32, &str); 4] =
    [(0.3, "crashcam-0-slowmo"), (1.25, "crashcam-1-freeway"), (3.25, "crashcam-2-car"), (5.5, "crashcam-3-reset")];
const TEST_AI_SHOTS: [(f32, &str); 2] = [(0.4, "ai-crash"), (2.0, "ai-crash-2")];

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_impact_test(
    mut commands: Commands,
    time: Res<Time>,
    mut test: ResMut<ImpactTest>,
    mut input: ResMut<crate::drive::CarInput>,
    traffic: Option<Res<TrafficSystem>>,
    crash: Res<crate::crash::CrashState>,
    spatial: SpatialQuery,
    mut player: Query<
        (Entity, &mut Transform, &mut LinearVelocity, &mut AngularVelocity),
        (With<Player>, With<Car>, Without<AiDriver>),
    >,
    mut ai_cars: Query<
        (Entity, &mut Transform, &mut LinearVelocity, Option<&AiCrash>),
        (With<AiDriver>, Without<Player>),
    >,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((me, mut t, mut v, mut w)) = player.single_mut() else { return };
    test.timer += time.delta_secs();
    match test.phase {
        // Let the world stream in and the traffic fill the area.
        0 => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            if test.timer < 10.0 {
                return;
            }
            let ground = |p: Vec3| {
                let filter = SpatialQueryFilter::from_mask(crate::props::GameLayer::World);
                let from = p + Vec3::Y * 10.0;
                spatial.cast_ray(from, Dir3::NEG_Y, 40.0, true, &filter).map_or(p.y, |h| from.y - h.distance)
            };
            let course = match test.kind {
                ImpactTestKind::Traffic => {
                    let Some(sys) = traffic.as_ref() else { return };
                    let target = sys
                        .cars_near(t.translation, 600.0)
                        .into_iter()
                        .filter(|c| !c.crashed && c.entity.is_some() && c.velocity.length() > 8.0)
                        .min_by(|a, b| {
                            a.position.distance(t.translation).total_cmp(&b.position.distance(t.translation))
                        });
                    target.map(|c| {
                        let dir = c.velocity.normalize();
                        let p = c.position + dir * 35.0;
                        (c.entity.expect("filtered"), Vec3::new(p.x, ground(p) + 0.6, p.z), -dir, TEST_SPEED_TRAFFIC)
                    })
                }
                ImpactTestKind::Parked => {
                    let Some(sys) = traffic.as_ref() else { return };
                    // Only a car standing on drivable ground with drivable ground in front of it at its height
                    // (many parked cars stand in yards that have no collision); the record's height is its floor
                    // (a ray from far above would find a car park's upper deck).
                    let floor = |p: Vec3| {
                        let filter = SpatialQueryFilter::from_mask(crate::props::GameLayer::World);
                        let from = p + Vec3::Y * 1.5;
                        spatial.cast_ray(from, Dir3::NEG_Y, 3.0, true, &filter).map(|h| from.y - h.distance)
                    };
                    let on_floor = |p: Vec3| floor(p).is_some_and(|y| (y - p.y).abs() < 0.5);
                    let ahead = |pos: Vec3, rot: Quat| {
                        let fwd = (rot * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z);
                        (fwd, pos + fwd * TEST_PARKED_AHEAD)
                    };
                    let near = sys.parked_near(t.translation, 400.0);
                    let target = near
                        .into_iter()
                        .filter(|&(_, pos, rot)| on_floor(pos) && on_floor(ahead(pos, rot).1))
                        .min_by(|a, b| a.1.distance(t.translation).total_cmp(&b.1.distance(t.translation)));
                    target.map(|(e, pos, rot)| {
                        let (fwd, p) = ahead(pos, rot);
                        (e, Vec3::new(p.x, pos.y + 0.6, p.z), -fwd, TEST_SPEED_PARKED)
                    })
                }
                // The AI car is put across the road ahead of the player's car, rolling slowly; the player's car
                // drives into its side.
                ImpactTestKind::Ai => {
                    let fwd = (t.rotation * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z);
                    let start = t.translation;
                    ai_cars.iter_mut().next().map(|(e, mut at, mut av, _)| {
                        let p = start + fwd * TEST_AI_AHEAD;
                        let across = Vec3::Y.cross(fwd);
                        at.translation = Vec3::new(p.x, ground(p) + 0.6, p.z);
                        at.rotation = Quat::from_rotation_y(across.x.atan2(across.z));
                        // The physics pose too (the spatial query holds `Position` here).
                        commands.entity(e).insert((Position(at.translation), Rotation(at.rotation)));
                        av.0 = across * TEST_AI_ROLL;
                        (e, start + Vec3::Y * 0.3, fwd, TEST_SPEED_AI)
                    })
                }
            };
            let Some((target, at, dir, speed)) = course else {
                if test.timer > 60.0 {
                    info!("TEST {:?}: no target found", test.kind);
                    exit.write(AppExit::Success);
                }
                return;
            };
            t.translation = at;
            t.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
            v.0 = dir * speed;
            w.0 = Vec3::ZERO;
            test.target = Some(target);
            info!("TEST {:?}: player at {at:?} heading {dir:?}, {speed:.0} m/s toward {target}", test.kind);
            test.shoot(&mut commands, "before");
            test.phase = 1;
            test.timer = 0.0;
        }
        // Drive into the target.
        1 => {
            input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
            if let Some((_, at, av, _)) = test.target.and_then(|e| ai_cars.get(e).ok())
                && (test.timer * 4.0).fract() < time.delta_secs() * 4.0
            {
                info!(
                    "TEST {:?}: player {:?} target {:?} ({:.0} m/s), {:.1} m apart",
                    test.kind,
                    t.translation,
                    at.translation,
                    av.0.length(),
                    t.translation.distance(at.translation)
                );
            }
            if test.kind == ImpactTestKind::Parked
                && test.timer >= TEST_PARKED_SHOT
                && test.timer - time.delta_secs() < TEST_PARKED_SHOT
            {
                test.shoot(&mut commands, "hit");
            }
            let ai_crashed = test.target.and_then(|e| ai_cars.get(e).ok()).is_some_and(|c| c.3.is_some());
            if crash.active.is_some() || ai_crashed {
                info!("TEST {:?}: crash (player {}, AI {ai_crashed})", test.kind, crash.active.is_some());
                test.phase = 2;
                test.timer = 0.0;
                test.shots = 0;
            } else if test.timer > 6.0 {
                info!("TEST {:?}: no crash 6 s after the setup", test.kind);
                test.shoot(&mut commands, "after");
                test.phase = 3;
                test.timer = 0.0;
            }
        }
        // The aftermath: the crash camera for the player's crash, else shots of the AI car.
        2 => {
            input.scripted = Some([0.0, 0.0, 0.0, 1.0]);
            let (shots, at): (&[(f32, &str)], f32) = match &crash.active {
                Some(a) => (&TEST_SHOTS, a.t),
                None => (&TEST_AI_SHOTS, test.timer),
            };
            if let Some(&(when, name)) = shots.get(test.shots)
                && at >= when
            {
                test.shoot(&mut commands, name);
                test.shots += 1;
            }
            let shots_done = test.shots >= shots.len();
            let ai_back = test.target.and_then(|e| ai_cars.get(e).ok()).is_none_or(|c| c.3.is_none());
            if shots_done && (ai_back || test.timer > 12.0) {
                info!("TEST {:?}: done (player car {me}, AI back {ai_back})", test.kind);
                test.phase = 3;
                test.timer = 0.0;
            }
        }
        _ => {
            if test.timer > 1.0 {
                exit.write(AppExit::Success);
            }
        }
    }
}
