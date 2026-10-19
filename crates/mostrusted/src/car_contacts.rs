//! Race car against race car: the contact response, as the game solves it (docs/decomp.md "Car against car:
//! contact response").
//!
//! The game builds a contact record per body pair (`FUN_00960f20` by the pair's categories, `FUN_00929350`: 0 the
//! player's car, 1 an AI car, 2 a crashing race car) and its contact pass (`FUN_009be420`, the friction pass
//! `FUN_009bed70`) computes each point's impulse from the relative velocity and the unscaled effective mass, then
//! adds impulse × the record's linear lane to each body's velocity and the angular impulse × its angular lane
//! (`FUN_009bb640` copies record +0x20 / +0x30 / +0x40 / +0x50 into constraint +0x90..+0x9c). The lanes
//! (`mw_assets::crash::RaceCarResponses`, `PHYSICS/PHYSICSCONFIG.BNDL`):
//!
//! | pair | handler | the first car | the second car | friction |
//! |------|---------|---------------|----------------|----------|
//! | player – AI | `FUN_009588c0` | by its damage 0 → 100: 0.4 / 0.05 → 1 / 0.4; crashed 1 / 0.7; the AI crashed 0.7 / 0.1 | by its damage: 0.5 / 0.1 → 1 / 0.1; crashed 0.7 / 0.4 | 0 |
//! | AI – AI | `FUN_009575e0` | by its damage 0 → 200: 0.4 / 0.05 → 1 / 0.4; crashed 1 / 0.7 | the same | 0.3 |
//! | player – crashing | `FUN_00959210` | by the faster car's speed 15 → 25 mph: 0.25 / 0.7 → 0.7 / 0.1 | 1 / 1 | 0.1 → 0.05 |
//! | AI – crashing | `FUN_00959730` | 0.25 / 0.7 → 0.7 / 0.25 | 1 / 1 → 1 / 0.7 | 0.1 → 0.05 |
//! | crashing – crashing | `CrashingRaceCarVsCrashingRaceCar` | 1 / 1 | 1 / 1 | 0.3 |
//!
//! (linear / angular). So a car takes a fraction of a rigid contact's push and very little of its spin until the
//! impact is a crash; the impact's damage and crash verdicts (`car_impacts.rs`) are decided by the same handler,
//! before the solve. A car that crashed in the impact (record +0x132 / +0x133 → constraint +0xf1 / +0xf2) cannot
//! be lifted by the pass: its vertical velocity gains at most clamp(3 − v_y, 0, 2) m/s per pass (`.rdata`
//! 0x010258f0 = 3, 0x01025840 = 2, mask 0x01025800 = (0, 1, 0)).
//!
//! avian keeps detecting race-car pairs (its contacts feed the effects and sounds), but the hook in `props.rs`
//! takes them out of its solver ([`neutralise`]); [`solve_race_contacts`] finds and solves them before each step
//! with the props solver's passes and push-out (`props.rs`), and [`report_impulses`] writes the impulses and the
//! pre-solve closing speeds back into avian's contacts after the step.
//!
//! Model decisions: the first pass of the game's island (`FUN_009be060`, linear only, on points whose predicted
//! separation is ≥ 0, so not on an approach) is left out, as for props;
//! restitution (−0.1 in the data) is not applied; between two impacts of a pair (0.5 s, `car_impacts.rs`) the
//! damage is 0 (the game's re-test gate vehicle +0x1640 is not traced); the crashing handlers blend towards
//! profile `68378547` (the register's choice between it and `7d03de76`, which differ only in restitution, was not
//! traced); the pass's velocity cap for crashed bodies (body +0x2f) is not applied.
//!
//! `MW_CAR_CONTACT_LOG=1` logs every solved pair.

use std::collections::{HashMap, HashSet};

use avian3d::collision::collider::contact_query;
use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::crash::{DamageScales, ImpulseScales, RaceCarResponses};

use crate::ai::AiDriver;
use crate::car_impacts::{AiCrash, Kind, Party, Verdict, apply_outcome, decide};
use crate::drive::{Car, Player};
use crate::props::{CONTACT_PASSES, PUSH_OUT, PUSH_OUT_MAX, PUSH_OUT_SLOP};

/// m/s per mph (`.bss` 0x0118a010).
const MPH: f32 = 0.44704;
/// MODEL (as `car_impacts.rs`): a pair's impact is evaluated again only after this long (s).
const PAIR_COOLDOWN: f32 = 0.5;
/// A crashed body's vertical velocity after a pass: at most 3 m/s, raised by at most 2 m/s
/// (`FUN_009be420`, `.rdata` 0x010258f0 / 0x01025840).
const CRASHED_LIFT_TOP: f32 = 3.0;
const CRASHED_LIFT_STEP: f32 = 2.0;
/// The speculative margin added to the step's closing distance (m), as `props.rs`.
const MARGIN: f32 = 0.05;

/// Marks a race car's body collider: its contacts with other race cars are solved here.
#[derive(Component)]
pub struct RaceCarBody;

/// `MW_RACE_CAR_CONTACTS=0`: leave race-car pairs to avian's rigid solve and `car_impacts.rs`'s after-step
/// evaluation (the old behaviour, for A/B).
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MW_RACE_CAR_CONTACTS").map_or(true, |v| v != "0"))
}

pub struct CarContactsPlugin;

impl Plugin for CarContactsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SolvedPairs>()
            .init_resource::<LastRaceImpact>()
            .add_systems(Update, mark_bodies)
            .add_systems(
                FixedPostUpdate,
                (
                    solve_race_contacts.before(PhysicsSystems::StepSimulation).after(crate::props::prop_contacts),
                    report_impulses.after(PhysicsSystems::StepSimulation).before(crate::car_impacts::EvaluateImpacts),
                ),
            );
    }
}

/// The last impact between two race cars (for the tests).
#[derive(Resource, Default, Debug, Clone)]
pub struct LastRaceImpact {
    pub count: u32,
    pub a: Option<Entity>,
    pub b: Option<Entity>,
    pub closing_mph: f32,
    pub damage: (f32, f32),
    pub crash: (bool, bool),
    pub scales: (ImpulseScales, ImpulseScales),
}

/// The pairs solved this step: (collider 1, collider 2, Σ normal impulse, pre-solve closing speed).
#[derive(Resource, Default)]
struct SolvedPairs(Vec<(Entity, Entity, f32, f32)>);

fn mark_bodies(mut commands: Commands, cars: Query<&Car, Added<Car>>) {
    for car in &cars {
        commands.entity(car.body_collider).insert((RaceCarBody, ActiveCollisionHooks::MODIFY_CONTACTS));
    }
}

/// The hook's part (`props.rs`): keep avian's contacts between two race cars (effects and sounds read them) but
/// let its solver push nothing: every point becomes a far speculative one.
pub fn neutralise(contacts: &mut ContactPair) {
    for m in contacts.manifolds.iter_mut() {
        m.friction = 0.0;
        m.restitution = 0.0;
        for p in m.points.iter_mut() {
            p.penetration = -1.0e3;
        }
    }
}

/// The pair's category (`FUN_00929350`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Category {
    Player,
    Ai,
    Crashing,
}

impl Category {
    fn rank(self) -> u8 {
        match self {
            Category::Player => 0,
            Category::Ai => 1,
            Category::Crashing => 2,
        }
    }
}

#[derive(Clone, Copy)]
struct Body {
    v: Vec3,
    w: Vec3,
    inv_mass: f32,
    inv_inertia: Mat3,
    com: Vec3,
}

impl Body {
    fn point_velocity(&self, p: Vec3) -> Vec3 {
        self.v + self.w.cross(p - self.com)
    }
    fn inv_effective(&self, p: Vec3, dir: Vec3) -> f32 {
        let rn = (p - self.com).cross(dir);
        self.inv_mass + rn.dot(self.inv_inertia * rn)
    }
    fn apply(&mut self, p: Vec3, impulse: Vec3, s: ImpulseScales) {
        self.v += s.linear * self.inv_mass * impulse;
        self.w += s.angular * (self.inv_inertia * (p - self.com).cross(impulse));
    }
}

/// The record's lanes for one pair, the first car (lower category) first.
#[derive(Debug, Clone, Copy)]
struct Lanes {
    a: ImpulseScales,
    b: ImpulseScales,
    friction: f32,
    /// The bodies that crashed in the impact (constraint +0xf1 / +0xf2).
    lift_a: bool,
    lift_b: bool,
}

fn lanes(
    r: &RaceCarResponses,
    a: Category,
    b: Category,
    va: Option<Verdict>,
    vb: Option<Verdict>,
    speed: f32,
) -> Lanes {
    let (crash_a, crash_b) = (va.is_some_and(|v| v.crash), vb.is_some_and(|v| v.crash));
    let (da, db) = (va.map_or(0.0, |v| v.damage), vb.map_or(0.0, |v| v.damage));
    match (a, b) {
        (Category::Player, Category::Ai) => {
            let p = &r.player_vs_ai;
            Lanes {
                // `FUN_009588c0`: the AI's crash first, then the player's own over it.
                a: if crash_a {
                    p.player_crashed
                } else if crash_b {
                    p.player_ai_crashed
                } else {
                    DamageScales::blend(&p.player[0], &p.player[1], da)
                },
                b: if crash_b { p.ai_crashed } else { DamageScales::blend(&p.ai[0], &p.ai[1], db) },
                friction: p.friction,
                lift_a: crash_a,
                lift_b: crash_b,
            }
        }
        (Category::Crashing, Category::Crashing) => Lanes {
            a: r.crashing_vs_crashing,
            b: r.crashing_vs_crashing,
            friction: r.crashing_vs_crashing_friction,
            lift_a: true,
            lift_b: true,
        },
        (first, Category::Crashing) => {
            let vs = if first == Category::Player { &r.player_vs_crashing } else { &r.ai_vs_crashing };
            let p = vs.at(speed);
            Lanes { a: p.car, b: p.crashing, friction: p.friction, lift_a: crash_a, lift_b: true }
        }
        // AI against AI (`FUN_009575e0`; the player against the player does not happen offline).
        _ => {
            let p = &r.ai_vs_ai;
            let pick = |crash: bool, d: f32| {
                if crash { p.crashed } else { DamageScales::blend(&p.scales[0], &p.scales[1], d) }
            };
            Lanes { a: pick(crash_a, da), b: pick(crash_b, db), friction: p.friction, lift_a: crash_a, lift_b: crash_b }
        }
    }
}

/// One contact point of a pair: `normal` from the first car to the second; `depth` > 0 overlap, < 0 gap.
struct Point {
    point: Vec3,
    normal: Vec3,
    depth: f32,
    normal_impulse: f32,
    friction_impulse: f32,
}

/// The game's passes over one pair's points (as `props.rs`'s `solve_contacts`, with lanes on both sides).
fn solve_pair(a: &mut Body, b: &mut Body, points: &mut [Point], l: &Lanes, dt: f32) {
    for _ in 0..CONTACT_PASSES {
        let (vy_a, vy_b) = (a.v.y, b.v.y);
        for c in points.iter_mut() {
            let n = c.normal;
            let vn = (a.point_velocity(c.point) - b.point_velocity(c.point)).dot(n);
            let target = if c.depth > 0.0 {
                -(PUSH_OUT * (c.depth - PUSH_OUT_SLOP).max(0.0) / dt).min(PUSH_OUT_MAX)
            } else {
                -c.depth / dt
            };
            let k = a.inv_effective(c.point, n) + b.inv_effective(c.point, n);
            if k <= 0.0 {
                continue;
            }
            let total = (c.normal_impulse + (vn - target) / k).max(0.0);
            let dl = total - c.normal_impulse;
            c.normal_impulse = total;
            a.apply(c.point, -dl * n, l.a);
            b.apply(c.point, dl * n, l.b);
            let rel = a.point_velocity(c.point) - b.point_velocity(c.point);
            let vt = rel - n * rel.dot(n);
            let speed = vt.length();
            if speed < 1e-4 || l.friction <= 0.0 {
                continue;
            }
            let t = vt / speed;
            let kt = a.inv_effective(c.point, t) + b.inv_effective(c.point, t);
            if kt <= 0.0 {
                continue;
            }
            let limit = l.friction * c.normal_impulse;
            let total = (c.friction_impulse + speed / kt).clamp(-limit, limit);
            let dl = total - c.friction_impulse;
            c.friction_impulse = total;
            a.apply(c.point, -dl * t, l.a);
            b.apply(c.point, dl * t, l.b);
        }
        // A crashed body cannot be lifted by the pass (`FUN_009be420` with constraint +0xf1 / +0xf2).
        let cap = |vy0: f32| (CRASHED_LIFT_TOP - vy0).clamp(0.0, CRASHED_LIFT_STEP);
        if l.lift_a {
            a.v.y = a.v.y.min(vy_a + cap(vy_a));
        }
        if l.lift_b {
            b.v.y = b.v.y.min(vy_b + cap(vy_b));
        }
    }
}

type CarItem<'a> = (
    Entity,
    &'a Car,
    &'a Position,
    &'a Rotation,
    &'a mut LinearVelocity,
    &'a mut AngularVelocity,
    &'a ComputedMass,
    &'a ComputedAngularInertia,
    &'a ComputedCenterOfMass,
    Has<Player>,
    Has<AiDriver>,
    Has<AiCrash>,
);

#[allow(clippy::too_many_arguments)]
fn solve_race_contacts(
    mut commands: Commands,
    time: Res<Time>,
    config: Option<Res<crate::crash::CrashConfig>>,
    race: Option<Res<crate::race::RaceState>>,
    mut crash: ResMut<crate::crash::CrashState>,
    mut cars: Query<CarItem>,
    shapes: Query<(&Collider, &Transform), Without<Car>>,
    mut solved: ResMut<SolvedPairs>,
    mut last: ResMut<LastRaceImpact>,
    mut cooldown: Local<HashMap<(Entity, Entity), f32>>,
    mut touching: Local<HashSet<(Entity, Entity)>>,
) {
    solved.0.clear();
    let dt = time.delta_secs();
    if dt <= 0.0 || !enabled() {
        return;
    }
    let now = time.elapsed_secs();
    cooldown.retain(|_, t| now - *t < PAIR_COOLDOWN);
    let data = config.as_ref().and_then(|c| c.0.impacts.as_ref());
    let responses = data.and_then(|d| d.responses);
    let log = std::env::var_os("MW_CAR_CONTACT_LOG").is_some();
    let player_crashed = crash.active.as_ref().is_some_and(|a| !a.reset_done);

    struct Entry {
        entity: Entity,
        collider: Entity,
        category: Category,
        is_player: bool,
        crashed: bool,
        position: Vec3,
        rotation: Quat,
        shape: (Collider, Vec3, Quat),
        radius: f32,
        body: Body,
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (e, car, pos, rot, v, w, mass, inertia, com, is_player, is_ai, ai_crashed) in cars.iter() {
        if !is_player && !is_ai {
            continue;
        }
        let Ok((shape, local)) = shapes.get(car.body_collider) else { continue };
        let crashed = if is_player { player_crashed } else { ai_crashed };
        let inv = inertia.rotated(rot.0).inverse();
        let half = shape.shape_scaled().as_cuboid().map_or(Vec3::splat(2.5), |c| c.half_extents);
        entries.push(Entry {
            entity: e,
            collider: car.body_collider,
            category: if crashed {
                Category::Crashing
            } else if is_player {
                Category::Player
            } else {
                Category::Ai
            },
            is_player,
            crashed,
            position: pos.0,
            rotation: rot.0,
            shape: (shape.clone(), pos.0 + rot.0 * local.translation, rot.0 * local.rotation),
            radius: half.length(),
            body: Body {
                v: v.0,
                w: w.0,
                inv_mass: mass.inverse(),
                inv_inertia: Mat3::from_cols(inv * Vec3::X, inv * Vec3::Y, inv * Vec3::Z),
                com: pos.0 + rot.0 * com.0,
            },
        });
    }
    let mut now_touching: HashSet<(Entity, Entity)> = HashSet::new();
    let mut changed = vec![false; entries.len()];
    let mut manifolds = Vec::new();
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            // The first car is the lower category (the handlers' order).
            let (ia, ib) = if entries[j].category.rank() < entries[i].category.rank() { (j, i) } else { (i, j) };
            let (ea, eb) = (&entries[ia], &entries[ib]);
            let rel = ea.body.v - eb.body.v;
            let margin = rel.length() * dt + MARGIN;
            if ea.shape.1.distance(eb.shape.1) > ea.radius + eb.radius + margin {
                continue;
            }
            manifolds.clear();
            contact_query::contact_manifolds(
                &ea.shape.0,
                ea.shape.1,
                ea.shape.2,
                &eb.shape.0,
                eb.shape.1,
                eb.shape.2,
                margin,
                &mut manifolds,
            );
            let mut points: Vec<Point> = manifolds
                .iter()
                .flat_map(|m| {
                    m.points.iter().map(move |p| Point {
                        point: p.point,
                        normal: m.normal,
                        depth: p.penetration,
                        normal_impulse: 0.0,
                        friction_impulse: 0.0,
                    })
                })
                .filter(|p| p.normal.length_squared() > 0.5 && p.point.is_finite())
                .collect();
            if points.is_empty() {
                continue;
            }
            // Does the solve push this step: a point that overlaps, or a gap the bodies close within the step.
            let closing = points
                .iter()
                .map(|p| (ea.body.point_velocity(p.point) - eb.body.point_velocity(p.point)).dot(p.normal))
                .fold(0.0f32, f32::max);
            let pushes = points.iter().any(|p| {
                p.depth > 0.0
                    || (ea.body.point_velocity(p.point) - eb.body.point_velocity(p.point)).dot(p.normal) * dt > -p.depth
            });
            if !pushes {
                continue;
            }
            let key = (ea.entity.min(eb.entity), ea.entity.max(eb.entity));
            now_touching.insert(key);
            // A new impact: the pair's handler decides the damage and the crash verdicts before the solve.
            let mut verdicts = (None, None);
            if !touching.contains(&key) && !cooldown.contains_key(&key) {
                if let Some(data) = data {
                    cooldown.insert(key, now);
                    let point =
                        points.iter().max_by(|x, y| x.depth.total_cmp(&y.depth)).map_or(ea.position, |p| p.point);
                    let normal = points[0].normal;
                    let party = |e: &Entry| {
                        let car = cars.get(e.entity).map(|c| c.1).ok()?;
                        Some(Party::race_car(
                            data,
                            e.entity,
                            car,
                            e.position,
                            e.rotation,
                            e.body.v,
                            e.is_player,
                            e.crashed,
                        ))
                    };
                    if let (Some(pa), Some(pb)) = (party(ea), party(eb)) {
                        let imp = crate::car_impacts::RawImpact {
                            a: ea.entity,
                            b: eb.entity,
                            point,
                            normal,
                            va: ea.body.v,
                            vb: eb.body.v,
                        };
                        let (va, vb, dmg, (la, lb)) = decide(data, &pa, &pb, &imp, race.is_some());
                        let closing_n = (ea.body.v - eb.body.v).dot(normal).max(0.0) / MPH;
                        let crashing = |k: Kind, v: Verdict| match (k, v.crash) {
                            (Kind::Player, true) => " → player CRASHES",
                            (_, true) => " → AI CRASHES",
                            _ => "",
                        };
                        info!(
                            "race car impact: {:?} zone {la} vs {:?} zone {lb}, closing {closing_n:.1} mph, speeds {:.0} / {:.0} mph, damage {:.1} / {:.1} (base {:.1} / {:.1}){}{}",
                            ea.category,
                            eb.category,
                            ea.body.v.length() / MPH,
                            eb.body.v.length() / MPH,
                            va.damage,
                            vb.damage,
                            dmg.base_a,
                            dmg.base_b,
                            crashing(pa.kind, va),
                            crashing(pb.kind, vb),
                        );
                        for (p, v, other) in [(&pa, va, eb.entity), (&pb, vb, ea.entity)] {
                            if v.crash {
                                let pre = p.velocity.length() / MPH;
                                apply_outcome(
                                    &mut commands,
                                    &mut crash,
                                    None,
                                    p.entity,
                                    p.kind,
                                    v,
                                    closing_n,
                                    pre,
                                    Some(other),
                                );
                                if p.kind == Kind::Ai
                                    && other
                                        == entries
                                            .iter()
                                            .find(|e| e.is_player)
                                            .map_or(Entity::PLACEHOLDER, |e| e.entity)
                                {
                                    info!("takedown: the player took down AI car {} ({})", p.entity, v.rule);
                                }
                            }
                        }
                        last.count += 1;
                        last.a = Some(ea.entity);
                        last.b = Some(eb.entity);
                        last.closing_mph = closing_n;
                        last.damage = (va.damage, vb.damage);
                        last.crash = (va.crash, vb.crash);
                        verdicts = (Some(va), Some(vb));
                    }
                }
            }
            let speed = ea.body.v.length().max(eb.body.v.length());
            let l = match &responses {
                Some(r) => lanes(r, ea.category, eb.category, verdicts.0, verdicts.1, speed),
                None => Lanes {
                    a: ImpulseScales::FULL,
                    b: ImpulseScales::FULL,
                    friction: 0.3,
                    lift_a: false,
                    lift_b: false,
                },
            };
            if verdicts.0.is_some() {
                last.scales = (l.a, l.b);
            }
            let (mut ba, mut bb) = (entries[ia].body, entries[ib].body);
            let before = (ba.v, ba.w, bb.v, bb.w);
            solve_pair(&mut ba, &mut bb, &mut points, &l, dt);
            let impulse: f32 = points.iter().map(|p| p.normal_impulse).sum();
            if log {
                info!(
                    "car contact {:?} – {:?}: {} points, closing {closing:.2} m/s, impulse {impulse:.0}, lanes {:.2}/{:.2} – {:.2}/{:.2}, friction {:.2}; first v {:.2} → {:.2}, w {:.2} → {:.2}; second v {:.2} → {:.2}, w {:.2} → {:.2}",
                    entries[ia].category,
                    entries[ib].category,
                    points.len(),
                    l.a.linear,
                    l.a.angular,
                    l.b.linear,
                    l.b.angular,
                    l.friction,
                    before.0,
                    ba.v,
                    before.1,
                    ba.w,
                    before.2,
                    bb.v,
                    before.3,
                    bb.w,
                );
            }
            entries[ia].body = ba;
            entries[ib].body = bb;
            changed[ia] = true;
            changed[ib] = true;
            solved.0.push((entries[ia].collider, entries[ib].collider, impulse, closing));
        }
    }
    *touching = now_touching;
    for (k, e) in entries.iter().enumerate() {
        if !changed[k] {
            continue;
        }
        if let Ok((.., mut v, mut w, _, _, _, _, _, _)) = cars.get_mut(e.entity) {
            v.0 = e.body.v;
            w.0 = e.body.w;
        }
    }
}

/// After the step: avian's contacts between the solved pairs carry our impulse and the pre-solve closing speed
/// (the effects, sounds and camera read them).
fn report_impulses(solved: Res<SolvedPairs>, mut graph: ResMut<ContactGraph>) {
    for &(c1, c2, impulse, closing) in &solved.0 {
        let Some((_, pair)) = graph.get_mut(c1, c2) else { continue };
        let n: usize = pair.manifolds.iter().map(|m| m.points.len()).sum();
        if n == 0 {
            continue;
        }
        let share = impulse / n as f32;
        for m in pair.manifolds.iter_mut() {
            for p in m.points.iter_mut() {
                p.normal_impulse = share;
                p.normal_speed = p.normal_speed.min(-closing);
            }
        }
    }
}
