//! Crashes and the crash camera, as the original runs them (docs/decomp.md, "Crashes").
//!
//! - **Decision** (`FUN_009a9fd0`): an impact on the car body against static scenery is a crash when its
//!   closing speed in mph exceeds the offline player's `WorldRules` threshold (70) or its damage exceeds
//!   the damage threshold (30). Our damage model has no damage-bar units for world impacts yet, so only the
//!   closing speed is tested there, on contact points that touch ([`touching`]: not avian's speculative ones). Impacts with traffic and other cars are decided in `car_impacts.rs`
//!   (the game's car-against-car damage and `TrafficRules` / "AI impact" / landing-on-a-car rules), which
//!   hands a crash over through [`CrashState::car_crash`].
//! - **While crashed** the player has no control (`Player.CanDriveAway`); the car keeps tumbling.
//! - **The crash sequence** (`mw_assets::crash::CrashCamera`, a feedback `Sequence`) runs on real time:
//!   slow motion from 1 to 0.1 over the first half second (the gameplay camera still shows), then the
//!   `Freeway` bystander shot at 0.14 and the car-relative shot at 0.22, letterbox bars and no HUD, and
//!   the gameplay camera back at 4.5 s.
//! - **Reset** (`FUN_006c9fb0`, `FUN_006d0180`): the car goes back to a pose saved before the crash, upright,
//!   rolling at `min(max(30, 0.7 × pre-crash mph), 50)` mph (`GameLogicConfig`).
//!
//! Model decisions (no source yet; docs/handling.md "Crash camera"): the reset happens when the cameras end;
//! the saved pose is the one a second before the impact (the game saves every frame and then moves the
//! pose onto the road network, `FUN_00648060`, which we don't have); the `Freeway` analyser's offset is
//! taken in the pre-crash heading's frame with its height replaced by the ground there; the car-relative
//! shot turns with the car's heading only; aim tracking is a per-60 Hz-tick fraction `Tracking_Speed`;
//! the screen offset pitches the view so the target sits that fraction of the half-height above centre;
//! with `Use_Zoom` off the field of view is `Maximum_FOV` (horizontal, as every game FOV: `camera::projection_fov`).
//!
//! `MW_CRASH_TRACE=<file>` logs every impact's decision and the sequence frame by frame.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::crash::{CrashData, Shot};

use crate::drive::{Car, CarInput, Player};

/// m/s per mph.
const MPH: f32 = 0.44704;
/// Impacts closing slower than this (mph) are not traced.
const TRACE_MIN_MPH: f32 = 5.0;
/// Model decision: the reset pose is the one saved this long (game seconds) before the impact.
const RESET_LOOKBACK: f32 = 1.0;
/// Safe poses kept (game seconds).
const HISTORY: f32 = 3.0;
/// Model decision: `Tracking_Speed` is the fraction of the aim error closed per 60 Hz tick.
const TICK: f32 = 1.0 / 60.0;
/// Contacts with normals steeper than this are the body landing on the ground (the game's
/// `LandingRules`, empty for the offline player), not a crash into scenery.
const MAX_WALL_NORMAL_Y: f32 = 0.7;

pub struct CrashPlugin;

impl Plugin for CrashPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrashState>()
            .add_systems(FixedUpdate, (hold_and_reset).before(crate::drive::car_physics))
            .add_systems(FixedPostUpdate, detect_crash.after(PhysicsSystems::StepSimulation))
            .add_systems(Update, run_sequence.after(crate::camera::GameplayCamera));
    }
}

/// The crash data from the install (`AssetStore::crash_data`).
#[derive(Resource, Clone)]
pub struct CrashConfig(pub Arc<CrashData>);

/// A pose the car held while driving normally.
#[derive(Debug, Clone, Copy)]
struct SafePose {
    time: f32,
    translation: Vec3,
    rotation: Quat,
    mph: f32,
}

/// The crash in progress.
#[derive(Debug, Clone)]
pub struct ActiveCrash {
    /// Sequence time (real seconds since the impact).
    pub t: f32,
    pub closing_mph: f32,
    /// Speed before the impact (mph) and the direction of travel (horizontal, unit).
    pre_mph: f32,
    travel: Vec3,
    reset_pose: Option<SafePose>,
    /// Set when the reset is due; the fixed schedule moves the car.
    reset_pending: bool,
    pub reset_done: bool,
    shot: Option<ShotView>,
    /// `Time<Virtual>` speed before the crash (tests run faster than real time).
    base_speed: f32,
}

#[derive(Debug, Clone, Copy)]
struct ShotView {
    index: usize,
    /// Static eye (scene-analyser shots); car-relative shots recompute it every frame.
    eye: Option<Vec3>,
    aim: Vec3,
}

/// A crash decided by a car-against-car impact (`car_impacts.rs`), started by [`detect_crash`] on the same tick.
#[derive(Debug, Clone, Copy)]
pub struct CarCrash {
    pub closing_mph: f32,
    pub damage: f32,
    /// The rule that fired ("traffic impact", "AI impact", "landing on a car", ...).
    pub rule: &'static str,
}

#[derive(Resource, Default)]
pub struct CrashState {
    pub active: Option<ActiveCrash>,
    /// Set by `car_impacts.rs` when the player's car crashed into another car this tick.
    pub car_crash: Option<CarCrash>,
    pub crashes: u32,
    history: VecDeque<SafePose>,
    game_time: f32,
    trace: Option<std::fs::File>,
    trace_opened: bool,
}

impl CrashState {
    fn trace(&mut self, line: std::fmt::Arguments) {
        if !self.trace_opened {
            self.trace_opened = true;
            if let Some(path) = std::env::var_os("MW_CRASH_TRACE") {
                self.trace = std::fs::File::create(path).ok();
                if let Some(f) = &mut self.trace {
                    let _ = writeln!(
                        f,
                        "# impact <game_t> <closing_mph> <rule_mph> <crash>\n\
                         # crash <game_t> <closing_mph> <pre_mph>\n\
                         # seq <t_real> <time_scale> <shot> <eye x y z> <aim x y z> <fov> <car x y z> <car_mph>\n\
                         # reset <t_real> <x y z> <heading_deg> <mph>\n# end <t_real>"
                    );
                }
            }
        }
        if let Some(f) = &mut self.trace {
            let _ = f.write_fmt(line);
            let _ = f.write_all(b"\n");
        }
    }
}

/// `MW_SPECULATIVE_CONTACTS=1`: count avian's speculative contacts as impacts again (the old behaviour, for A/B).
pub(crate) fn speculative_contacts() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MW_SPECULATIVE_CONTACTS").is_ok_and(|v| v == "1"))
}

/// Whether a body's contact point is a contact the solver acted on this step. Avian keeps speculative contacts
/// up to the distance the bodies cover in one step (|v| dt: 0.65 m at 150 km/h, 1.1 m at 250 km/h) and reports
/// them with the closing speed they would have; the solver pushes only on those it predicts to close within the
/// step. The game's collision analysis gets the contacts its collision job found (decomp.md "Crashes"), so a
/// contact that never touched is not an impact: without this, a wall or kerb face a metre ahead counted as a hit
/// at full speed, a tick or more before (or without) the real one.
pub(crate) fn touching(p: &ContactPoint) -> bool {
    p.normal_impulse > 0.0 || p.penetration > 0.0 || speculative_contacts()
}

/// Record safe poses and test body impacts against the world rule.
#[allow(clippy::too_many_arguments)]
pub(crate) fn detect_crash(
    time: Res<Time>,
    config: Option<Res<CrashConfig>>,
    mut state: ResMut<CrashState>,
    virtual_time: Res<Time<Virtual>>,
    collisions: Collisions,
    cars: Query<(&Car, &Transform, &LinearVelocity), With<Player>>,
    collider_of: Query<&ColliderOf>,
    bodies: Query<&RigidBody>,
    props: Query<(), With<crate::props::PropBody>>,
) {
    let Some(config) = config else { return };
    let Ok((car, transform, velocity)) = cars.single() else { return };
    state.game_time += time.delta_secs();
    let now = state.game_time;
    let car_crash = state.car_crash.take();
    if state.active.is_some() {
        return;
    }
    // Static scenery only (no body, or a static one); props and other cars have their own rules.
    let world = |e: Entity| {
        !props.contains(e)
            && collider_of.get(e).ok().is_none_or(|c| bodies.get(c.body).is_ok_and(|b| *b == RigidBody::Static))
    };
    let collider = car.body_collider;
    if std::env::var_os("MW_CRASH_CONTACTS").is_some() {
        // Every body-world contact point: closing speed (m/s), penetration, impulse, normal (from the world at
        // the car), the point in the car's frame.
        let inv = transform.rotation.inverse();
        let mut lines = Vec::new();
        for pair in collisions.collisions_with(collider) {
            let other = if pair.collider1 == collider { pair.collider2 } else { pair.collider1 };
            if !world(other) {
                continue;
            }
            let sign = if pair.collider1 == collider { -1.0 } else { 1.0 };
            for m in &pair.manifolds {
                for p in &m.points {
                    let l = inv * (p.point - transform.translation);
                    let n = m.normal * sign;
                    lines.push(format!(
                        "contact {now:.3} v {:.1} cl {:.2} pen {:.3} imp {:.0} n {:.2} {:.2} {:.2} local {:.2} {:.2} {:.2}",
                        velocity.length(),
                        -p.normal_speed,
                        p.penetration,
                        p.normal_impulse,
                        n.x,
                        n.y,
                        n.z,
                        l.x,
                        l.y,
                        l.z
                    ));
                }
            }
        }
        for l in lines {
            state.trace(format_args!("{l}"));
        }
    }
    // The strongest wall contact: closing speed, point, normal, penetration, impulse.
    let best = collisions
        .collisions_with(collider)
        .filter(|pair| world(if pair.collider1 == collider { pair.collider2 } else { pair.collider1 }))
        .flat_map(|pair| {
            // The normal from the world at the car.
            let sign = if pair.collider1 == collider { -1.0 } else { 1.0 };
            pair.manifolds.iter().map(move |m| (m, m.normal * sign))
        })
        .filter(|(m, _)| m.normal.y.abs() < MAX_WALL_NORMAL_Y)
        .flat_map(|(m, n)| {
            m.points
                .iter()
                .filter(|p| touching(p))
                .map(move |p| (-p.normal_speed, p.point, n, p.penetration, p.normal_impulse))
        })
        .fold(
            None,
            |a: Option<(f32, Vec3, Vec3, f32, f32)>, b| if a.is_some_and(|a| a.0 >= b.0) { a } else { Some(b) },
        );
    let closing = best.map_or(0.0, |b| b.0.max(0.0));
    let mut closing_mph = closing / MPH;
    let rule = config.0.rules.world;
    let is_world_crash = rule.is_some_and(|r| r.is_crash(None, closing_mph));
    if let Some(c) = car_crash
        && !is_world_crash
    {
        closing_mph = c.closing_mph;
    }
    let is_crash = is_world_crash || car_crash.is_some();
    if closing_mph > TRACE_MIN_MPH {
        let limit = rule.and_then(|r| r.closing_mph).unwrap_or(f32::INFINITY);
        // The contact in the car's frame (x right, y up, z forward from the origin at road level).
        let (local, n, pen, imp) = best.map_or((Vec3::NAN, Vec3::NAN, f32::NAN, f32::NAN), |b| {
            (transform.rotation.inverse() * (b.1 - transform.translation), b.2, b.3, b.4)
        });
        // The world's side of the contact (world space).
        let wp = best.map_or(Vec3::NAN, |b| b.1 + b.2 * b.3 * 0.5);
        state.trace(format_args!(
            "impact {now:.3} {closing_mph:.1} {limit:.1} {} local {:.2} {:.2} {:.2} n {:.2} {:.2} {:.2} pen {pen:.3} imp {imp:.1} at {:.1} {:.2} {:.1} wp {:.2} {:.3} {:.2}",
            if is_crash { 1 } else { 0 },
            local.x,
            local.y,
            local.z,
            n.x,
            n.y,
            n.z,
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
            wp.x,
            wp.y,
            wp.z
        ));
    }
    if !is_crash {
        state.history.push_back(SafePose {
            time: now,
            translation: transform.translation,
            rotation: transform.rotation,
            mph: velocity.length() / MPH,
        });
        while state.history.front().is_some_and(|p| now - p.time > HISTORY) {
            state.history.pop_front();
        }
        return;
    }
    // The impact itself: the last safe pose is the one before this tick.
    let pre = state.history.back().copied();
    let pre_mph = pre.map_or(velocity.length() / MPH, |p| p.mph);
    let reset_pose =
        state.history.iter().rev().find(|p| now - p.time >= RESET_LOOKBACK).or(state.history.front()).copied();
    // The heading before the impact (this tick's velocity is already the rebound).
    let heading = pre.map_or(transform.rotation, |p| p.rotation) * Vec3::Z;
    let travel = Vec3::new(heading.x, 0.0, heading.z).normalize_or(Vec3::Z);
    state.crashes += 1;
    state.trace(format_args!("crash {now:.3} {closing_mph:.1} {pre_mph:.1}"));
    match car_crash.filter(|_| !is_world_crash) {
        Some(c) => info!(
            "CRASH: {} (damage {:.1}, closing {:.1} mph, pre-crash {pre_mph:.0} mph); crash camera {}",
            c.rule, c.damage, c.closing_mph, config.0.camera.id
        ),
        None => info!(
            "CRASH: closing {closing_mph:.1} mph > WorldRules {:?} mph (pre-crash {pre_mph:.0} mph); crash camera {}",
            rule.and_then(|r| r.closing_mph),
            config.0.camera.id
        ),
    }
    state.active = Some(ActiveCrash {
        t: 0.0,
        closing_mph,
        pre_mph,
        travel,
        reset_pose,
        reset_pending: false,
        reset_done: false,
        shot: None,
        base_speed: virtual_time.relative_speed(),
    });
}

/// While crashed the player has no control; when the reset is due, put the car back on its saved pose.
#[allow(clippy::type_complexity)]
fn hold_and_reset(
    config: Option<Res<CrashConfig>>,
    mut state: ResMut<CrashState>,
    mut input: ResMut<CarInput>,
    mut cars: Query<(&mut Transform, &mut LinearVelocity, &mut AngularVelocity), (With<Car>, With<Player>)>,
    facing: Res<crate::race::RespawnFacing>,
) {
    let Some(config) = config else { return };
    let crashed = state.active.as_ref().is_some_and(|a| !a.reset_done);
    input.crashed = crashed;
    let Some(active) = state.active.as_mut() else { return };
    if !active.reset_pending {
        return;
    }
    active.reset_pending = false;
    active.reset_done = true;
    input.crashed = false;
    let Ok((mut transform, mut lin, mut ang)) = cars.single_mut() else { return };
    let pose = active.reset_pose.unwrap_or(SafePose {
        time: 0.0,
        translation: transform.translation,
        rotation: transform.rotation,
        mph: active.pre_mph,
    });
    // In a race the reset faces the target gate (`FUN_006c9fb0`: gate position − saved position).
    let fwd = match facing.0 {
        Some(gate) if (gate - pose.translation).with_y(0.0).length() > 0.1 => gate - pose.translation,
        _ => pose.rotation * Vec3::Z,
    };
    let yaw = fwd.x.atan2(fwd.z);
    let rotation = Quat::from_rotation_y(yaw);
    let mph = config.0.reset.after_crash(active.pre_mph);
    transform.translation = pose.translation;
    transform.rotation = rotation;
    lin.0 = rotation * Vec3::Z * mph * MPH;
    ang.0 = Vec3::ZERO;
    let (t, p) = (active.t, pose.translation);
    info!(
        "crash reset: back to ({:.1}, {:.1}, {:.1}) heading {:.0}°, rolling at {mph:.0} mph",
        p.x,
        p.y,
        p.z,
        yaw.to_degrees()
    );
    state.trace(format_args!("reset {t:.3} {:.2} {:.2} {:.2} {:.1} {mph:.1}", p.x, p.y, p.z, yaw.to_degrees()));
}

/// Advance the sequence: time scale, HUD, letterbox, the shots' camera, and the reset when the cameras end.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub(crate) fn run_sequence(
    real: Res<Time<Real>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    config: Option<Res<CrashConfig>>,
    mut state: ResMut<CrashState>,
    mut hud: ResMut<crate::hud::HudSettings>,
    mut snap: ResMut<crate::camera::SnapCamera>,
    spatial: SpatialQuery,
    cars: Query<(Entity, &Transform, &LinearVelocity, &Car), (Without<Camera3d>, With<Player>)>,
    mut car_visibility: Query<&mut Visibility, With<Car>>,
    mut camera: Query<(&mut Transform, &mut Projection), crate::camera::DrivingCamera>,
    mut letterbox: ResMut<crate::cutscene::LetterboxBars>,
) {
    let Some(config) = config else { return };
    let cam_data = &config.0.camera;
    let Some(mut active) = state.active.take() else { return };
    active.t += real.delta_secs();
    let t = active.t;

    // Slow motion (the simulation's clock); the sequence itself runs on real time.
    let scale = cam_data.time_scale(t);
    virtual_time.set_relative_speed(active.base_speed * scale);

    // HUD and letterbox.
    hud.hidden_by_sequence = cam_data.hide_hud.is_some_and(|(a, b)| t >= a && t < b);
    if let Some(lb) = cam_data.letterbox
        && let Some(out) = lb.state(t)
    {
        letterbox.show(lb.bars, out);
    }

    // The reset when the gameplay camera comes back.
    if t >= cam_data.cameras_end() && !active.reset_done && !active.reset_pending {
        active.reset_pending = true;
        snap.0 = true;
    }

    // The shots' camera (the gameplay camera already placed itself this frame; override it).
    let car = cars.single().ok();
    let mut traced = None;
    match (cam_data.shot_at(t), car) {
        (Some((index, shot)), Some((entity, car_t, vel, car))) => {
            if active.shot.is_none_or(|s| s.index != index) {
                active.shot = Some(start_shot(index, shot, car_t, active.travel, &spatial, entity, car.body_collider));
            }
            if let Ok(mut v) = car_visibility.get_mut(entity)
                && *v != Visibility::Inherited
            {
                *v = Visibility::Inherited;
            }
            let view = active.shot.as_mut().expect("set above");
            let b = &shot.camera;
            let target = car_t.translation + car_t.rotation * Vec3::from(b.target_offset);
            let eye = view.eye.unwrap_or_else(|| {
                let fwd = car_t.rotation * Vec3::Z;
                Quat::from_rotation_y(fwd.x.atan2(fwd.z)) * Vec3::from(b.offset) + car_t.translation
            });
            let ticks = real.delta_secs() / TICK;
            view.aim += (target - view.aim) * (1.0 - (1.0 - b.tracking.speed.clamp(0.0, 1.0)).powf(ticks));
            let fov = if b.zoom.use_zoom { b.zoom.min_fov } else { b.zoom.max_fov };
            if let Ok((mut cam, mut projection)) = camera.single_mut() {
                let mut frame = Transform::from_translation(eye).looking_at(view.aim, Vec3::Y);
                // Screen offsets in half-widths / half-heights; the FOV is horizontal (`camera::projection_fov`).
                let (half_x, half_y) = match &*projection {
                    Projection::Perspective(p) => (
                        (fov.to_radians() / 2.0).tan(),
                        (crate::camera::projection_fov(fov, p.aspect_ratio) / 2.0).tan(),
                    ),
                    _ => ((fov.to_radians() / 2.0).tan(), (fov.to_radians() / 2.0).tan()),
                };
                frame.rotate_local_x(-(b.tracking.screen_offset[1] * half_y).atan());
                frame.rotate_local_y((b.tracking.screen_offset[0] * half_x).atan());
                *cam = frame;
                if let Projection::Perspective(p) = &mut *projection {
                    p.fov = crate::camera::projection_fov(fov, p.aspect_ratio);
                }
            }
            traced = Some((index as i32, eye, view.aim, fov, car_t.translation, vel.length() / MPH));
        }
        _ => {
            active.shot = None;
            if let (Some((_, car_t, vel, _)), Ok((cam, projection))) = (car, camera.single_mut()) {
                let fov = match &*projection {
                    Projection::Perspective(p) => crate::camera::game_fov(p),
                    _ => 0.0,
                };
                traced = Some((
                    -1,
                    cam.translation,
                    cam.translation + cam.forward() * 10.0,
                    fov,
                    car_t.translation,
                    vel.length() / MPH,
                ));
            }
        }
    }
    if let Some((shot, e, a, fov, c, mph)) = traced {
        state.trace(format_args!(
            "seq {t:.3} {scale:.3} {shot} {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} {fov:.1} {:.2} {:.2} {:.2} {mph:.1}",
            e.x, e.y, e.z, a.x, a.y, a.z, c.x, c.y, c.z
        ));
    }

    let letterbox_end = cam_data.letterbox.map_or(0.0, |l| l.end + l.transition_out);
    if t >= cam_data.length.max(letterbox_end) && active.reset_done {
        virtual_time.set_relative_speed(active.base_speed);
        hud.hidden_by_sequence = false;
        state.trace(format_args!("end {t:.3}"));
        info!("crash sequence over after {t:.2} s");
        return;
    }
    state.active = Some(active);
}

/// Place a shot's camera at its start.
fn start_shot(
    index: usize,
    shot: &Shot,
    car_t: &Transform,
    travel: Vec3,
    spatial: &SpatialQuery,
    car: Entity,
    body: Entity,
) -> ShotView {
    let b = &shot.camera;
    let target = car_t.translation + car_t.rotation * Vec3::from(b.target_offset);
    let eye = b.analyser.map(|a| {
        // The analyser's space: its offset in the pre-crash heading's frame, on the ground there.
        let frame = Quat::from_rotation_y(travel.x.atan2(travel.z));
        let mut p = car_t.translation + frame * Vec3::new(a.translation[0], 0.0, a.translation[2]);
        let filter = SpatialQueryFilter::from_excluded_entities([car, body]).with_mask(crate::props::GameLayer::World);
        let from = p + Vec3::Y * 20.0;
        p.y = spatial
            .cast_ray(from, Dir3::NEG_Y, 60.0, true, &filter)
            .map_or(car_t.translation.y, |h| from.y - h.distance);
        p + frame * Vec3::from(b.offset)
    });
    info!("crash shot {index}: {} ({:.1}–{:.1} s), eye {:?}", b.camera_binding, shot.start, shot.end, eye);
    ShotView { index, eye, aim: target }
}
