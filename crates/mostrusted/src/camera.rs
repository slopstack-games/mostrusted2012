//! Gameplay cameras driven by each car's `VehicleCameraContainer` (`mw_assets::camera`).
//!
//! Modes come from the data: `External` (chase), `Bonnet` and `Bumper`; C or the gamepad's Y button
//! cycles them (Back / Select already opens the car menu), `--camera-mode` picks one at startup. Every
//! mode looks back while B or the right stick button is held, or the right stick is pulled fully back.
//!
//! The chase camera:
//! - **Yaw spring**: the camera's heading follows the car's with a per-60 Hz-tick spring that blends
//!   from the low-speed to the high-speed value (`Yaw_spring_modification`), and `YawDriftSpring`
//!   while drifting, when it also leans towards the direction of travel. It orbits the data's pivot
//!   point (`d68bbc59`, at the look-at point's z).
//! - **Speed**: eye and look-at heights and the eye distance move to their full-speed values
//!   (`Speed_based_movement`); the field of view widens from `MinFieldOfView` to `MaxFieldOfView`.
//! - **Acceleration**: the eye falls back under acceleration and closes in under braking
//!   (`Acceleration_movement`).
//! - **Roll and pitch**: roll from lateral acceleration or yaw rate between the data's minimum and
//!   maximum, shaped by `Camera_Roll_Factor_Power`; a nose-down pitch from deceleration between
//!   `Minimum_` and `Maximum_deceleration_for_pitch_change`, shaped by `Pitch_change_curve_power`.
//!   Both approach their targets at their `…_Intensity` rate.
//! - **Shake**: a high-speed shake (`High_Speed_Shake`, between its minimum and maximum speeds) and an
//!   impact shake: body-collider contacts kick the damped springs of `CameraGameplayShakeEffect`
//!   (pitch and fore-aft translation).
//! - **Collision**: a ray from the look-at point to the eye keeps the camera on this side of walls.
//! - **Look around**: the right stick (and right mouse button + drag) orbits the camera; it springs
//!   back behind the car when released.
//!
//! What is data and what is a model decision is listed in docs/handling.md ("Cameras").

use std::f32::consts::{PI, TAU};
use std::io::Write;

use avian3d::prelude::*;
use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraState};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use mw_assets::camera::{ExternalCamera, ImpactShake, InCarCamera, ShakeAxis, VehicleCameras};

use crate::drift::Phase;
use crate::drive::Car;

/// m/s per mph: the camera data's speeds are mph, its accelerations mph per second.
const MPH: f32 = 0.44704;
/// Stick deflection below this is ignored (worn sticks drift).
const STICK_DEAD_ZONE: f32 = 0.2;
/// Right stick pulled back beyond this (and not far sideways): look back.
const STICK_LOOK_BACK: f32 = 0.85;
/// Orbit at full stick deflection: yaw and pitch (radians; pitch up raises the camera).
const STICK_YAW: f32 = 110.0 * PI / 180.0;
const STICK_PITCH: [f32; 2] = [-10.0 * PI / 180.0, 30.0 * PI / 180.0];
/// Mouse orbit per pixel (radians).
const MOUSE_YAW: f32 = 0.005;
const MOUSE_PITCH: f32 = 0.004;
/// Orbit follows the stick at this rate (1/s) and springs back behind the car at `ORBIT_RETURN`.
const ORBIT_FOLLOW: f32 = 10.0;
const ORBIT_RETURN: f32 = 4.0;
/// Share of the drift angle the chase camera leans towards the direction of travel while drifting.
const DRIFT_LEAN: f32 = 0.5;
/// How far the camera's yaw may trail the car's (radians).
const MAX_YAW_LAG: f32 = 0.6;
/// Damping ratio of the acceleration spring (critical: no overshoot when the throttle lifts).
const ACCEL_DAMPING: f32 = 1.0;
/// Rate (1/s) at which the field of view follows its target.
const FOV_RATE: f32 = 3.0;
/// High-speed shake: metres of eye movement per unit of `High_Speed_Shake.Amplitude`, and the
/// noise's base frequency (Hz) at `Frequency` 1.
const SHAKE_METRES: f32 = 0.012;
const SHAKE_HZ: f32 = 1.4;
/// Impact speed (m/s, approaching the body collider) that kicks the shake springs with strength 1.
const IMPACT_FULL: f32 = 10.0;
const IMPACT_MIN: f32 = 1.0;
/// Contacts this close together belong to one impact (seconds).
const IMPACT_GAP: f32 = 0.3;
/// Keep the eye this far in front of a wall (m), and never nearer to the look-at point than this.
const CLIP_MARGIN: f32 = 0.25;
const CLIP_MIN: f32 = 0.8;
/// Rate (1/s) at which the eye moves back out after a wall stops blocking it.
const CLIP_RELAX: f32 = 3.0;
/// Free camera: right stick look rate (rad/s at full deflection).
const FREE_LOOK_RATE: f32 = 2.2;

pub struct CameraPlugin {
    pub mode: CameraMode,
    /// `--camera-mode lookback`: hold the look-back view (for headless checks).
    pub look_back: bool,
}

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraSettings { mode: self.mode, look_back: self.look_back })
            .init_resource::<LookInput>()
            .init_resource::<ImpactKicks>()
            .add_systems(Update, (camera_input, follow_car).chain().after(crate::drive::read_input))
            .add_systems(Update, free_camera_stick)
            .add_systems(FixedPostUpdate, collect_impacts.after(PhysicsSystems::StepSimulation));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    #[default]
    Chase,
    Bonnet,
    Bumper,
}

impl CameraMode {
    pub fn parse(s: &str) -> Option<(Self, bool)> {
        Some(match s {
            "chase" | "external" => (Self::Chase, false),
            "bonnet" | "hood" => (Self::Bonnet, false),
            "bumper" => (Self::Bumper, false),
            "lookback" | "look-back" => (Self::Chase, true),
            _ => return None,
        })
    }

    /// The modes a car's data defines, in switching order.
    fn available(cams: &VehicleCameras) -> Vec<Self> {
        let mut out = vec![Self::Chase];
        if cams.bonnet.is_some() {
            out.push(Self::Bonnet);
        }
        if cams.bumper.is_some() {
            out.push(Self::Bumper);
        }
        out
    }
}

#[derive(Resource)]
pub struct CameraSettings {
    pub mode: CameraMode,
    pub look_back: bool,
}

/// Look-around input, as orbit targets (radians) relative to the camera's normal view.
#[derive(Resource, Default)]
struct LookInput {
    yaw: f32,
    pitch: f32,
    /// A stick or the mouse is holding the orbit (else it springs back).
    held: bool,
    look_back: bool,
    mouse: Vec2,
}

/// Impact strength waiting for the camera (filled in the fixed schedule).
#[derive(Resource, Default)]
struct ImpactKicks {
    pending: f32,
    event_peak: f32,
    quiet: f32,
}

#[allow(clippy::too_many_arguments)]
fn camera_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
    menu: Option<Res<crate::menu::CarMenu>>,
    cars: Query<&Car>,
    mut settings: ResMut<CameraSettings>,
    mut look: ResMut<LookInput>,
) {
    let menu_open = menu.is_some_and(|m| m.open);
    let mut switch = !menu_open && keys.just_pressed(KeyCode::KeyC);
    let mut look_back = !menu_open && keys.pressed(KeyCode::KeyB);
    let mut stick = Vec2::ZERO;
    for pad in &pads {
        switch |= !menu_open && pad.just_pressed(GamepadButton::North);
        look_back |= !menu_open && pad.pressed(GamepadButton::RightThumb);
        let s = Vec2::new(
            pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
            pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
        );
        if s.length() > stick.length() {
            stick = s;
        }
    }
    if menu_open {
        stick = Vec2::ZERO;
    }
    if let (true, Ok(car)) = (switch, cars.single())
        && let Some(cams) = car.params.camera
    {
        let modes = CameraMode::available(&cams);
        let i = modes.iter().position(|m| *m == settings.mode).map_or(0, |i| (i + 1) % modes.len());
        settings.mode = modes[i];
        info!("camera mode {:?}", settings.mode);
    }

    // Right stick: x orbits (right = look right), y raises / lowers the camera; fully back looks back.
    let deflection = (stick.length() - STICK_DEAD_ZONE).max(0.0) / (1.0 - STICK_DEAD_ZONE);
    let stick_held = deflection > 0.0;
    if stick_held && stick.y < -STICK_LOOK_BACK && stick.x.abs() < 0.5 {
        look_back = true;
    }
    let mouse_held = !menu_open && mouse_buttons.pressed(MouseButton::Right);
    if mouse_held {
        look.mouse.x -= mouse.delta.x * MOUSE_YAW;
        look.mouse.y = (look.mouse.y + mouse.delta.y * MOUSE_PITCH).clamp(STICK_PITCH[0], STICK_PITCH[1]);
    } else {
        look.mouse = Vec2::ZERO;
    }
    if stick_held && !look_back {
        let dir = stick.normalize_or_zero() * deflection.min(1.0);
        look.yaw = -dir.x * STICK_YAW;
        look.pitch = if dir.y >= 0.0 { dir.y * STICK_PITCH[1] } else { -dir.y * STICK_PITCH[0] };
    } else if mouse_held {
        look.yaw = look.mouse.x;
        look.pitch = look.mouse.y;
    } else {
        look.yaw = 0.0;
        look.pitch = 0.0;
    }
    look.held = (stick_held && !look_back) || mouse_held;
    look.look_back = look_back || settings.look_back;
}

/// Body-collider contacts of the driven car become impact kicks for the shake springs.
fn collect_impacts(time: Res<Time>, collisions: Collisions, cars: Query<&Car>, mut kicks: ResMut<ImpactKicks>) {
    let Ok(car) = cars.single() else { return };
    kicks.quiet += time.delta_secs();
    let speed = collisions
        .collisions_with(car.body_collider)
        .flat_map(|pair| pair.manifolds.iter())
        .flat_map(|m| m.points.iter())
        .map(|p| -p.normal_speed)
        .fold(0.0, f32::max);
    if speed < IMPACT_MIN {
        return;
    }
    if kicks.quiet > IMPACT_GAP {
        kicks.event_peak = 0.0;
    }
    kicks.quiet = 0.0;
    // One kick per impact: later contacts of the same crash only add what exceeds its peak so far.
    if speed > kicks.event_peak {
        kicks.pending += (speed - kicks.event_peak.max(IMPACT_MIN)) / IMPACT_FULL;
        kicks.event_peak = speed;
    }
}

/// A damped spring per shake axis (degrees or metres), kicked by impacts.
#[derive(Default, Clone, Copy)]
struct Spring {
    x: f32,
    v: f32,
}

impl Spring {
    fn step(&mut self, axis: &ShakeAxis, dt: f32) {
        if axis.amplitude == 0.0 {
            *self = Spring::default();
            return;
        }
        let k = axis.spring.max(1.0);
        let c = 2.0 * axis.damping * k.sqrt();
        // Substeps keep the stiff springs (k = 450) stable at low frame rates.
        let n = (dt * 240.0).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            self.v += (-k * self.x - c * self.v) * h;
            self.x += self.v * h;
            if self.x < axis.limits[0] || self.x > axis.limits[1] {
                self.x = self.x.clamp(axis.limits[0], axis.limits[1]);
                self.v = 0.0;
            }
        }
    }
}

/// The 3D camera while it follows a car (the free camera has its own controller).
type DrivingCamera = (With<Camera3d>, Without<FreeCamera>, Without<crate::reflection::PlanarReflectionCamera>);

/// Camera state, smoothed every frame.
#[derive(Default)]
struct CameraState {
    car: Option<Entity>,
    mode: Option<CameraMode>,
    yaw: f32,
    up: Vec3,
    /// Orbit offset (look-around) applied on top of the camera's yaw and pitch.
    orbit: Vec2,
    /// Eye z offset from acceleration (m) and its rate (m/s).
    accel_z: f32,
    accel_zv: f32,
    fov: f32,
    roll: f32,
    pitch: f32,
    /// Longitudinal acceleration (m/s²), measured between physics ticks.
    accel: f32,
    last_speed: f32,
    since_speed: f32,
    shake_t: f32,
    /// Distance from the look-at point to the eye the walls allow.
    clip: f32,
    /// Impact springs: translation x, y, z then rotation pitch, roll, yaw.
    springs: [Spring; 6],
    trace: Option<std::fs::File>,
    time: f32,
}

#[allow(clippy::too_many_arguments)]
fn follow_car(
    time: Res<Time>,
    settings: Res<CameraSettings>,
    look: Res<LookInput>,
    mut kicks: ResMut<ImpactKicks>,
    mut state: Local<CameraState>,
    spatial: SpatialQuery,
    cars: Query<(Entity, &Transform, &LinearVelocity, &Car), Without<Camera3d>>,
    mut car_visibility: Query<&mut Visibility, With<Car>>,
    mut camera: Query<(&mut Transform, &mut Projection), DrivingCamera>,
) {
    let (Ok((entity, car_t, vel, car)), Ok((mut cam, mut projection))) = (cars.single(), camera.single_mut()) else {
        return;
    };
    let dt = time.delta_secs().max(1e-4);
    let ease = |rate: f32| 1.0 - (-dt * rate).exp();
    let cams = car.params.camera.unwrap_or_else(|| VehicleCameras {
        external: ExternalCamera::default(),
        bonnet: None,
        bumper: None,
        rear_view: None,
    });
    let ext = &cams.external;
    let s = &mut *state;

    let forward = car_t.rotation * Vec3::Z;
    let car_yaw = forward.x.atan2(forward.z);
    let flat_v = Vec3::new(vel.x, 0.0, vel.z);
    let speed = flat_v.length();
    let mph = speed / MPH;
    if s.car != Some(entity) {
        let trace = s.trace.take();
        *s = CameraState {
            car: Some(entity),
            yaw: car_yaw,
            up: Vec3::Y,
            fov: ext.fov[0],
            last_speed: car.speed,
            clip: f32::MAX,
            trace,
            ..default()
        };
        if s.trace.is_none()
            && let Some(path) = std::env::var_os("MW_CAMERA_TRACE")
        {
            s.trace = std::fs::File::create(&path).ok();
            if let Some(f) = &mut s.trace {
                let _ = writeln!(
                    f,
                    "# t speed_kmh accel_ms2 yaw_rate lat_ms2 slip_deg mode eye_x eye_y eye_z yaw_lag_deg fov roll pitch accel_z shake_pitch"
                );
            }
        }
    }
    s.time += dt;

    // Longitudinal acceleration: the car's speed changes once per physics tick, so measure across ticks.
    s.since_speed += dt;
    if car.speed != s.last_speed {
        let raw = ((car.speed - s.last_speed) / s.since_speed.max(1e-3)).clamp(-40.0, 40.0);
        s.accel += (raw - s.accel) * (1.0 - (-s.since_speed * 8.0).exp());
        s.last_speed = car.speed;
        s.since_speed = 0.0;
    }
    let accel_mph = s.accel / MPH;

    // Orbit (look-around): follows the input, springs back when released; look-back is a cut.
    if look.look_back {
        s.orbit = Vec2::ZERO;
    } else {
        let rate = if look.held { ORBIT_FOLLOW } else { ORBIT_RETURN };
        s.orbit.x += wrap_angle(look.yaw - s.orbit.x) * ease(rate);
        s.orbit.y += (look.pitch - s.orbit.y) * ease(rate);
    }

    // Impact shake springs.
    let kick = std::mem::take(&mut kicks.pending).min(ext.impact.max_amplitude);
    step_springs(&mut s.springs, &ext.impact, kick, dt);

    // High-speed shake: grows with the speed fraction between the shake speeds, to the power `Power`.
    let sh = &ext.shake;
    let shake_f = ((mph - sh.speed[0]) / (sh.speed[1] - sh.speed[0]).max(1.0)).clamp(0.0, 1.0);
    s.shake_t += dt * sh.frequency * SHAKE_HZ * TAU;
    let shake = sh.amplitude * SHAKE_METRES * shake_f.powf(sh.power.max(0.1));
    let shake_v = Vec3::new(
        (s.shake_t * 1.7).sin() + 0.5 * (s.shake_t * 4.1).sin(),
        (s.shake_t * 2.3).sin() + 0.4 * (s.shake_t * 5.3).sin(),
        0.0,
    ) * shake;

    let mode = settings.mode;
    let in_car = match mode {
        CameraMode::Chase => None,
        CameraMode::Bonnet => cams.bonnet,
        CameraMode::Bumper => cams.bumper,
    };
    if s.mode != Some(mode) {
        s.mode = Some(mode);
        s.clip = f32::MAX;
    }
    // The bumper camera sits inside the body's nose (the original draws no car in that view).
    if let Ok(mut v) = car_visibility.get_mut(entity) {
        let want =
            if mode == CameraMode::Bumper && in_car.is_some() { Visibility::Hidden } else { Visibility::Inherited };
        if *v != want {
            *v = want;
        }
    }
    let speed_frac = (speed / car.params.top_speed.max(1.0)).clamp(0.0, 1.0);

    let (pos, target, up, fov) = if let Some(c) = in_car {
        in_car_view(&c, car_t, s, look.look_back, speed_frac)
    } else {
        chase_view(ext, car_t, car, s, &look, mph, accel_mph, speed_frac, flat_v, dt)
    };
    s.fov += (fov - s.fov) * ease(FOV_RATE);

    // Walls between the look-at point and the eye pull the eye in (in-car views sit inside the body).
    let mut pos = pos;
    if in_car.is_none() {
        let to_eye = pos - target;
        let full = to_eye.length();
        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider]);
        let allowed = Dir3::new(to_eye).ok().map_or(full, |dir| {
            spatial
                .cast_ray(target, dir, full + CLIP_MARGIN, true, &filter)
                .map_or(full, |hit| (hit.distance - CLIP_MARGIN).max(CLIP_MIN))
        });
        // In at once, back out gently.
        let current = s.clip.min(full);
        s.clip = if allowed < current { allowed } else { current + (allowed - current) * ease(CLIP_RELAX) };
        if s.clip < full {
            pos = target + to_eye * (s.clip / full);
        }
    }

    let frame = Transform::from_translation(pos).looking_at(target, up);
    let shake_pitch = s.springs[3].x;
    let mut t = frame;
    t.translation += frame.rotation * (shake_v + Vec3::new(s.springs[0].x, s.springs[1].x, -s.springs[2].x));
    t.rotate_local_x(-(s.pitch + shake_pitch).to_radians());
    t.rotate_local_y(s.springs[5].x.to_radians());
    t.rotate_local_z((s.roll + s.springs[4].x).to_radians());
    *cam = t;
    if let Projection::Perspective(p) = &mut *projection {
        // Vertical, as Bevy's: read as horizontal, 70° crops the car far tighter than the original's
        // chase view.
        p.fov = s.fov.to_radians();
    }

    if let Some(f) = &mut s.trace {
        let local = car_t.rotation.inverse() * (cam.translation - car_t.translation);
        let _ = writeln!(
            f,
            "{:.3} {:.2} {:.2} {:.3} {:.2} {:.2} {:?} {:.3} {:.3} {:.3} {:.2} {:.2} {:.2} {:.2} {:.3} {:.2}",
            s.time,
            car.speed * 3.6,
            s.accel,
            car.yaw_rate,
            car.lateral_accel,
            car.body_slip.to_degrees(),
            mode,
            local.x,
            local.y,
            local.z,
            wrap_angle(s.yaw - car_yaw).to_degrees(),
            s.fov,
            s.roll,
            s.pitch,
            s.accel_z,
            shake_pitch
        );
    }
}

fn step_springs(springs: &mut [Spring; 6], data: &ImpactShake, kick: f32, dt: f32) {
    let axes = data.translation.iter().chain(&data.rotation);
    for (spring, axis) in springs.iter_mut().zip(axes) {
        // A kick is an initial velocity: the spring's peak is about amplitude × strength / √k.
        spring.v += axis.amplitude * kick;
        spring.step(axis, dt);
    }
}

/// The chase camera: (eye, look-at point, up, target field of view).
#[allow(clippy::too_many_arguments)]
fn chase_view(
    ext: &ExternalCamera,
    car_t: &Transform,
    car: &Car,
    s: &mut CameraState,
    look: &LookInput,
    mph: f32,
    accel_mph: f32,
    speed_frac: f32,
    flat_v: Vec3,
    dt: f32,
) -> (Vec3, Vec3, Vec3, f32) {
    let ease = |rate: f32| 1.0 - (-dt * rate).exp();
    let forward = car_t.rotation * Vec3::Z;
    let car_yaw = forward.x.atan2(forward.z);
    let p = &ext.pitch;
    let range = |x: f32| ((x - p.deceleration[0]) / (p.deceleration[1] - p.deceleration[0]).max(0.1)).clamp(0.0, 1.0);

    // Yaw spring: per-60 Hz-tick fractions, low- to high-speed, or the drift spring while drifting.
    let ys = &ext.yaw_spring;
    let drifting = matches!(car.drift.phase, Phase::Drifting | Phase::Entering);
    let fraction = if drifting {
        ext.yaw_drift_spring
    } else {
        let t = ((mph - ys.speed[0]) / (ys.speed[1] - ys.speed[0]).max(1.0)).clamp(0.0, 1.0);
        ys.spring[0] + (ys.spring[1] - ys.spring[0]) * t
    };
    let rate = -(1.0 - fraction.clamp(0.001, 0.9)).ln() * 60.0;
    let mut target_yaw = car_yaw;
    if drifting && car.speed > 3.0 {
        let travel = flat_v.x.atan2(flat_v.z);
        target_yaw += wrap_angle(travel - car_yaw) * DRIFT_LEAN;
    }
    s.yaw += wrap_angle(target_yaw - s.yaw) * ease(rate);
    let lag = wrap_angle(s.yaw - target_yaw);
    if lag.abs() > MAX_YAW_LAG {
        s.yaw = target_yaw + lag.signum() * MAX_YAW_LAG;
    }
    // Follow the ground's slope slowly, not every bump.
    s.up = s.up.lerp(car_t.rotation * Vec3::Y, ease(2.0)).normalize_or(Vec3::Y);

    // Speed: eye and look-at heights and the eye distance reach their full-speed values at `full_speed`.
    let sp = &ext.speed;
    let f = (mph / sp.full_speed.max(1.0)).clamp(0.0, 1.0);
    let mut eye = Vec3::from(ext.eye);
    let mut look_at = Vec3::from(ext.look_at);
    eye.y += (sp.eye_y - eye.y) * f;
    eye.z += sp.eye_z * f;
    look_at.y += (sp.look_y - look_at.y) * f;

    // Acceleration: the eye hangs on a critically damped spring of angular frequency `frequency`, so it
    // trails by a / ω² under acceleration and runs ahead under braking, within the data's limits.
    let am = &ext.acceleration;
    let w = am.frequency.max(0.5);
    let a = if car.speed < 0.0 { 0.0 } else { s.accel };
    let n = (dt * 240.0).ceil().max(1.0);
    let h = dt / n;
    for _ in 0..n as usize {
        s.accel_zv += (-w * w * s.accel_z - 2.0 * ACCEL_DAMPING * w * s.accel_zv - a) * h;
        s.accel_z += s.accel_zv * h;
        if s.accel_z < am.back || s.accel_z > am.forward {
            s.accel_z = s.accel_z.clamp(am.back, am.forward);
            s.accel_zv = 0.0;
        }
    }
    eye.z += s.accel_z;

    // Roll: from lateral acceleration (mph/s) or yaw rate (rad/s), whichever is further into its range.
    let r = &ext.roll;
    let lat = car.lateral_accel / MPH;
    let f_lat = ((lat.abs() - r.lateral[0]) / (r.lateral[1] - r.lateral[0]).max(0.1)).clamp(0.0, 1.0);
    let f_ang = ((car.yaw_rate.abs() - r.angular[0]) / (r.angular[1] - r.angular[0]).max(0.01)).clamp(0.0, 1.0);
    let roll_target = lat.signum() * r.max_angle * f_lat.max(f_ang).powf(r.factor_power.max(0.1));
    s.roll += (roll_target - s.roll) * ease(r.intensity);
    // Pitch: nose down under deceleration.
    let decel = (-accel_mph).max(0.0);
    let pitch_target = if car.speed > 0.5 { p.max_change * range(decel).powf(p.curve_power.max(0.1)) } else { 0.0 };
    s.pitch += (pitch_target - s.pitch) * ease(p.intensity);

    // Orbit the pivot: the look-around offsets turn and raise the eye around it; looking back turns
    // the whole view round.
    let back = if look.look_back { PI } else { 0.0 };
    let frame = Quat::from_rotation_arc(Vec3::Y, s.up) * Quat::from_rotation_y(s.yaw);
    let orbit = Quat::from_rotation_y(s.orbit.x + back) * Quat::from_rotation_x(s.orbit.y);
    let pivot = Vec3::from(ext.pivot);
    let eye_w = car_t.translation + frame * (pivot + orbit * (eye - pivot));
    let look_w = car_t.translation + frame * (pivot + Quat::from_rotation_y(s.orbit.x + back) * (look_at - pivot));
    let fov = ext.fov[0] + (ext.fov[1] - ext.fov[0]) * speed_frac * speed_frac;
    (eye_w, look_w, s.up, fov)
}

/// Bonnet and bumper views: fixed in the car, looking along it (or back from `back_eye`).
fn in_car_view(
    c: &InCarCamera,
    car_t: &Transform,
    s: &mut CameraState,
    look_back: bool,
    speed_frac: f32,
) -> (Vec3, Vec3, Vec3, f32) {
    s.roll = 0.0;
    s.pitch = 0.0;
    s.accel_z = 0.0;
    s.accel_zv = 0.0;
    let (eye, dir) = if look_back { (c.back_eye, -1.0) } else { (c.eye, 1.0) };
    let orbit = Quat::from_rotation_y(s.orbit.x) * Quat::from_rotation_x(-s.orbit.y * 0.5);
    let eye = Vec3::from(eye);
    let ahead = eye + orbit * Vec3::new(0.0, 0.0, 10.0 * dir);
    let fov = c.fov[0] + (c.fov[1] - c.fov[0]) * speed_frac * speed_frac;
    (car_t.transform_point(eye), car_t.transform_point(ahead), car_t.rotation * Vec3::Y, fov)
}

/// Free camera (no car): the right stick looks around, as the mouse does.
fn free_camera_stick(
    time: Res<Time>,
    pads: Query<&Gamepad>,
    mut cams: Query<(&mut Transform, &mut FreeCameraState), With<FreeCamera>>,
) {
    let Ok((mut t, mut state)) = cams.single_mut() else { return };
    if !state.enabled {
        return;
    }
    let mut stick = Vec2::ZERO;
    for pad in &pads {
        let s = Vec2::new(
            pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
            pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
        );
        if s.length() > stick.length() {
            stick = s;
        }
    }
    let len = stick.length();
    if len <= STICK_DEAD_ZONE {
        return;
    }
    let stick = stick * ((len - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE)).min(1.0) / len;
    let dt = time.delta_secs();
    state.yaw -= stick.x * FREE_LOOK_RATE * dt;
    state.pitch = (state.pitch + stick.y * FREE_LOOK_RATE * dt).clamp(-1.54, 1.54);
    t.rotation = Quat::from_euler(EulerRot::ZYX, 0.0, state.yaw, state.pitch);
}

fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}
