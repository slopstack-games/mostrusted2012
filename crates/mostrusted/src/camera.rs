//! Gameplay cameras driven by each car's `VehicleCameraContainer` (`mw_assets::camera`).
//!
//! Modes come from the data: `External` (chase), `Bonnet` and `Bumper`; C or the gamepad's Y button
//! cycles them (Back / Select already opens the car menu), `--camera-mode` picks one at startup. Every
//! mode looks back while B or the right stick button is held, or the right stick is pulled fully back.
//!
//! The chase camera (`FUN_005d4970`; checked against a recording of the original with `--camera-replay`,
//! `tools/camcompare.py`):
//! - **Followed frame**: the car frame of the wheels' plane (`Car::wheel_frame`, not the body on its springs),
//!   its forward's height low-passed (0.12 per update), rebuilt round the world's up: no roll. Its Euler angles
//!   are smoothed per angle with `angle += (1 − retain) · wrap(target − angle)`, `retain = 1 − clamp(K · dt · 60,
//!   0, 1)`; the pitch's retain is scaled by `min(mph · 0.02, 1)`. No limit on the lag. The yaw stiffness runs
//!   from `63fb2207` to `5a74d44d` over the speed band and towards `YawDriftSpring` with the slip angle
//!   (`FUN_005c90a0`); the pitch stiffness eases by 0.1 per update to 0.55, or to 0.1 in the air (`FUN_005c8e40`).
//! - **Composition** (`FUN_005c2130`): the eye swings round the pivot (`d68bbc59`) with the smoothed angles, its
//!   spring and z offsets scaled by |cos(yaw lag)|; the look-at point rides in the followed frame itself, so the
//!   view keeps the car in sight while the eye swings out.
//! - **Speed**: eye and look-at heights and the eye distance move to their full-speed values
//!   (`Speed_based_movement`); the field of view widens from `MinFieldOfView` to `MaxFieldOfView` by the speed
//!   fraction to the power max / min, plus a lagged acceleration term (`FUN_005c76a0`). The eye closes in as the
//!   car turns side-on (`FUN_005c8ad0`) and dollies along its line to the look-at point (`FUN_005c1f90`).
//! - **Acceleration**: the eye falls back under acceleration and closes in under braking
//!   (`Acceleration_movement`, `FUN_005d12b0`, normalised by 22.35 m/s²); braking also turns the look direction
//!   down (`Deceleration_pitch_change`, `FUN_005c7a80`).
//! - **Slopes** (`Gradient_Adjustments`, `FUN_005c7a80`): uphill the eye rises and the view turns down by half
//!   the slope, downhill the eye drops; held in the air.
//! - **Weight**: the eye's height rides a damped spring (`FUN_005d12b0`'s vertical stage) kicked by the
//!   car's vertical acceleration on the ground and its vertical speed in the air, stiffer for 0.3 s after
//!   a landing; the look-at point follows half of it (0.4 in the air, more while a landing squashes it).
//! - **Roll** (`FUN_005c9380`) from the lateral acceleration (m/s²) or the yaw rate between the data's minimum
//!   and maximum, shaped by `Camera_Roll_Factor_Power`, leaning out of the turn and following at
//!   `intensity · π/180 · 20` per second.
//! - **Shake**: a high-speed shake (`High_Speed_Shake`: a slow quaternion curve scaled by
//!   `Amplitude · f^Power / 10`, turning the view about a point 10 m behind the eye; `FUN_005e2a70`) and an
//!   impact shake (`FUN_005cd040` level, `FUN_005dfe90` noise spring).
//! - **Collision** (ours: no world query exists in the original's chase code): a ray from the look-at point to
//!   the eye keeps the camera on this side of walls.
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
use mw_assets::camera::{ExternalCamera, ImpactShake, InCarCamera, VehicleCameras};

use crate::drive::{Car, Player};

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
/// `FUN_005c9380`: the roll moves by `intensity · (π/180) · ROLL_RATE · dt` of its gap per update
/// (`_DAT_00fb0630` = π/180, `_DAT_00fb07f0` = 20); ranges narrower than `ROLL_EPS` (`_DAT_00fb0890`) count as it.
const ROLL_RATE: f32 = 20.0;
const ROLL_EPS: f32 = 2.384_19e-7;
/// The per-player default pitch stiffness (`state + 0x70`, live read 0.55; recorded cam + 0x6d0 sits there
/// except in the air), per 60 Hz tick. Its writer is not read. (The roll stiffness cam + 0x6dc reads 1.0, but
/// the followed frame has no roll at all: see `chase_view`.)
const K_PITCH_DEFAULT: f32 = 0.55;
/// Per-player default yaw stiffness (state + 0x74, live read 0.09): the value cam + 0x6d4 holds at a standstill.
const K_YAW_STANDING: f32 = 0.09;
/// Model decision (fit): below this speed (m/s) cam + 0x6d4 holds the standing value. In `ref/live1` it leaves 0.09
/// between 1.0 and 1.13 m/s and returns between 1.05 and 1.4 m/s (`FUN_005c89c0`'s reset flag, car + 0x3c, unread).
const K_YAW_STANDING_SPEED: f32 = 1.15;
/// FOV speed band (state + 0x88 / + 0x8c, live read 0 / 125 mph; FUN_005c76a0).
const FOV_FULL_MPH: f32 = 125.0;
/// `FUN_005c8e40`: the pitch stiffness moves this share of the way to its default each update
/// (`DAT_00f9f660`, not scaled by the time step).
const K_PITCH_EASE: f32 = 0.1;
/// `FUN_005c89c0`: the pitch's retain is scaled by `min(|speed| · this, 1)` (`_DAT_00fb0880`, m/s).
const PITCH_RETAIN_PER_SPEED: f32 = 0.02;
/// `FUN_005e14d0`: within this of a vertical forward axis the previous angles are kept (`DAT_00f9d474`).
const GIMBAL_EPS: f32 = 0.01;
/// `FUN_005c8e40`: while the car is airborne the pitch stiffness eases to this instead (`DAT_00f9f660` again).
const K_PITCH_AIR: f32 = 0.1;
/// `FUN_005c90a0`: the yaw stiffness is all `YawDriftSpring` at this slip angle (degrees; `.bss` 0x0110e8f0 = 45,
/// read from a memory snapshot of the running game).
const YAW_SLIP_FULL_DEG: f32 = 45.0;
/// `FUN_005d4970`: the followed frame's forward height moves this share of its gap per update (`_DAT_00fb0a20`)
/// and stays within this of the car frame's (`_DAT_00fafb8c`).
const FWD_Y_RATE: f32 = 0.12;
const FWD_Y_RANGE: f32 = 0.3;
/// `FUN_005d12b0`'s z stage, `.bss` vectors read from a memory snapshot: the acceleration (m/s²) that gives the
/// full z offset and its clamp (`DAT_0110e770` = 22.352, i.e. 50 mph/s), the clamp of the gap the rate follows
/// (`DAT_0110e4d0` = 0.75 m), the clamp of the offset (`DAT_0110e680` = 4 m), the squared speed below which the
/// rate is zero (`DAT_0110e610` = 0.01), and the speed above which the airborne term runs (`DAT_0110e640` = 5
/// mph); `_DAT_00fb0770` the vector-length epsilon. The airborne term eases at 0.1 per update (`_DAT_00fb07a0`)
/// to 1.85 m/s (`_DAT_00fb09d0`).
const ACCEL_FULL: f32 = 22.352;
const ACCEL_Z_STEP: f32 = 0.75;
const ACCEL_Z_LIMIT: f32 = 4.0;
const ACCEL_Z_STILL: f32 = 0.01;
const ACCEL_Z_AIR_MPH: f32 = 5.0;
const ACCEL_EPS: f32 = 1.19e-7;
const ACCEL_Z_AIR: f32 = 1.85;
const ACCEL_Z_AIR_RATE: f32 = 0.1;
/// `FUN_005c8e40`'s landing request: on the first grounded update after the air, the impact shake's level is
/// raised to `clamp((|a_y| − 10) · 0.00204, 0, 1) · 6.5` (`DAT_00fa929c`, `_DAT_00fb0eac`, `_DAT_00fafe14`), a_y the
/// vertical acceleration (m/s², `FUN_005d9610` lane 1). Checked on `ref/live1`'s four landings: the recorded level
/// is the request after one update's decay (1.34 → 1.12, 1.23 → 1.03, 2.71 → 2.27, 4.58 → 3.82).
const LANDING_A_MIN: f32 = 10.0;
/// `FUN_005c7a80`: the deceleration pitch only while the car travels within 10° of its nose (`.bss` 0x0110e4e0 =
/// cos 10°, live read).
const DECEL_PITCH_COS: f32 = 0.9848;
/// `FUN_005c7a80`: the look-at point follows `LOOK_BOB[0]` (`_DAT_00fb05e0`) of the eye's vertical spring, 0.4 in
/// the air (`_DAT_00fb0850`), and `0.4 + 2.6 · u` (`_DAT_00fb0840`) while the spring is below zero and falling in
/// the landing window, u = `clamp((peak a_y − 10) · 0.00204)`; it eases by 0.05 per update (`.bss` 0x0110e790),
/// in that landing case by `lerp(0.05, 1, u)` (0x0110e550 = 1).
const LOOK_BOB: [f32; 3] = [0.5, 0.4, 2.6];
const LOOK_BOB_RATE: f32 = 0.05;
const LANDING_A_SCALE: f32 = 0.00204;
const LANDING_LEVEL: f32 = 6.5;
/// `FUN_005c1f90`, the FOV dolly: the eye moves along its line to the look-at point by
/// `DOLLY_C + DOLLY_OFFSET − DOLLY_C / tan(FOV / 2)` metres (`.rdata` 0x00fb0e90 = 3.15, 0x00fb0720 = 0.15):
/// −1.2 (back) at 70°, +0.41 (in) at 95°, so the car keeps its size on screen as the FOV widens.
const DOLLY_C: f32 = 3.15;
const DOLLY_OFFSET: f32 = 0.15;
/// `FUN_005c8ad0`, the side-on pull-in: the eye's x / z scale by `1 + |sin(yaw lag)| × (width / length − 1) ×
/// SIDE_ON_WEIGHT` (`DAT_00f8a810` = 0.5).
const SIDE_ON_WEIGHT: f32 = 0.5;
/// `FUN_005c76a0`, the acceleration FOV term (cam + 0x6ec): target `|a|` (m/s²) clamped to 20
/// (`_DAT_00fb07f0`) while the car accelerates forwards, approached by 0.01 per update rising and 0.05
/// falling (`_DAT_00fb0810` / `_DAT_00fb0800`), added to the FOV in degrees.
const FOV_ACCEL_MAX: f32 = 20.0;
const FOV_ACCEL_RISE: f32 = 0.01;
const FOV_ACCEL_FALL: f32 = 0.05;
/// Rate (1/s) at which the field of view follows its target.
const FOV_RATE: f32 = 3.0;
/// The original's camera update rate: `FUN_005cd040`'s decay is per update, not scaled by dt.
const UPDATE_HZ: f32 = 60.0;
/// At most this many impact-noise updates per frame; a longer frame drops the rest, as the original's loop
/// drops time behind its 2/60 s catch-up step (decomp.md "Update loop"). The noise is stationary, so dropped
/// updates change nothing but its phase.
const SHAKE_MAX_STEPS: u32 = 8;
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
            .init_resource::<SnapCamera>()
            .add_systems(
                Update,
                (camera_input, follow_car).chain().in_set(GameplayCamera).after(crate::drive::read_input),
            )
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

/// The largest contact value (normal speed into the contact, m/s) since the camera last ran (filled in the
/// fixed schedule; `FUN_005cd040` takes the maximum over the contact list, never a sum).
#[derive(Resource, Default)]
struct ImpactKicks {
    contact: f32,
}

#[allow(clippy::too_many_arguments)]
fn camera_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
    menu: Option<Res<crate::menu::CarMenu>>,
    cars: Query<&Car, With<Player>>,
    mut settings: ResMut<CameraSettings>,
    mut look: ResMut<LookInput>,
) {
    let menu_open = menu.is_some_and(|m| m.open);
    // The original's defaults (input.rs): CHANGECAMERA = C / LB; ROTATECAMERA* = Delete, Page Down, Home,
    // End / right stick. LOOK_BACK has no default binding: End or the stick held fully back looks back below.
    use crate::input::defaults as act;
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let switch = !menu_open && crate::input::Devices { keys: &keys, pads: &pad_list }.just_pressed(act::CHANGE_CAMERA);
    let mut look_back = false;
    // Keys as a stick; the pads' right stick (radial, below) overrides them when deflected further.
    let keys_only = crate::input::Devices { keys: &keys, pads: &[] };
    let mut stick = Vec2::new(
        keys_only.value(act::CAMERA_RIGHT) - keys_only.value(act::CAMERA_LEFT),
        keys_only.value(act::CAMERA_UP) - keys_only.value(act::CAMERA_DOWN),
    );
    for pad in &pads {
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

/// Body-collider contacts of the driven car: the largest normal speed this tick feeds the impact shake.
fn collect_impacts(collisions: Collisions, cars: Query<&Car, With<Player>>, mut kicks: ResMut<ImpactKicks>) {
    let Ok(car) = cars.single() else { return };
    let speed = collisions
        .collisions_with(car.body_collider)
        .flat_map(|pair| pair.manifolds.iter())
        .flat_map(|m| m.points.iter())
        .map(|p| -p.normal_speed)
        .fold(0.0, f32::max);
    kicks.contact = kicks.contact.max(speed);
}

/// The impact shake (`FUN_005cd040` -> `FUN_005dfe90`, decomp.md "Camera effects"): a level raised by contacts
/// and decaying per update, times a noise-driven spring, gives three view angles (degrees).
#[derive(Default)]
struct ImpactShakeState {
    level: f32,
    /// Spring offset and velocity of the first two lanes (the original stores no state for the third).
    x: [f32; 2],
    v: [f32; 2],
    rng: u64,
    /// Output angles (degrees) about the camera's own x, y, z (cam + 0x120).
    angles: Vec3,
    /// Frame time not yet consumed by the noise stage's 1/60 s updates, and the stage's last output
    /// (x + jitter per lane, before the amplitude).
    pending: f32,
    noise: Vec3,
    /// `FUN_005c8e40`'s air flag (cam + 0x714): set while the car is airborne, cleared on the update that
    /// requests the landing shake.
    airborne: bool,
}

impl ImpactShakeState {
    /// The game's LCG (`FUN_005dfe90`, multiplier 0x5851f42d4c957f2d, + 1): a uniform float in [0, 1).
    fn uniform(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        f32::from_bits(((self.rng >> 41) as u32) | 0x3f80_0000) - 1.0
    }

    fn step(&mut self, d: &ImpactShake, contact: f32, dt: f32) {
        // FUN_005cd040: decay by a fraction per update, then raise to the normalised contact (max, not add).
        self.level *= (1.0 - d.decay).max(0.0).powf(dt * UPDATE_HZ);
        if contact > 0.0 {
            let t = ((contact - d.contact[0]) / (d.contact[1] - d.contact[0]).max(f32::EPSILON)).clamp(0.0, 1.0);
            let raised = (t * contact * d.scale).powf(d.power).clamp(0.0, d.cap);
            self.level = self.level.max(raised);
        }
        // FUN_005dfe90: per lane a jitter in +-ShakeMagnitude; lanes 0-1 also a spring kicked by a velocity in
        // +-WobbleMagnitude each update: v += kick + (-x - v) * spring; x += v * frequency * dt. The original
        // runs it once per 1/60 s tick (h = frequency · dt = 0.5 in `ref/live1` for 13 417 of 13 430 updates,
        // also right after a 4.7 s stall), so it runs here in 1/60 s steps whatever the frame time: with
        // spring 1 the map is x ← (1 − h)·x + h·kick, which grows without bound once h > 2 (frames longer than
        // 1/15 s at frequency 30). Stepped with the frame time it reached 1e30 in ordinary low-frame-rate
        // driving and then turned the view by huge angles or, past f32 overflow, into NaN (a white screen).
        self.pending = (self.pending + dt).min(SHAKE_MAX_STEPS as f32 / UPDATE_HZ);
        let h = d.frequency / UPDATE_HZ;
        let amplitude = d.max_amplitude * self.level;
        while self.pending >= 1.0 / UPDATE_HZ {
            self.pending -= 1.0 / UPDATE_HZ;
            let mut out = [0.0; 3];
            for (lane, o) in out.iter_mut().enumerate() {
                let jitter = (2.0 * self.uniform() - 1.0) * d.jitter[lane];
                let mut x = 0.0;
                if lane < 2 {
                    let kick = (2.0 * self.uniform() - 1.0) * d.wobble[lane];
                    self.v[lane] += kick + (-self.x[lane] - self.v[lane]) * d.spring;
                    self.x[lane] += self.v[lane] * h;
                    x = self.x[lane];
                }
                *o = x + jitter;
            }
            self.noise = Vec3::from(out);
        }
        self.angles = amplitude * self.noise;
    }

    /// Rotates the view about its own axes (the original pre-multiplies the angle matrix onto the camera
    /// matrix; its rotation pivot, `FUN_005dfe90`'s fourth argument, is not ported: we pivot on the eye).
    fn apply(&self, t: &mut Transform) {
        let a = self.angles * (std::f32::consts::PI / 180.0);
        t.rotation *= Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z);
    }
}

/// Samples in the high-speed shake's curve, and their rate (the original's period is 398 / 30 = 13.27 s:
/// the 199 samples are played forwards then backwards).
const SPEED_SHAKE_SAMPLES: usize = 199;
const SPEED_SHAKE_RATE: f32 = 30.0;
/// `FUN_005e34e0`'s constant vectors: the amplitude divisor (also the pivot distance, m), the pitch bias factor.
const SPEED_SHAKE_DIVISOR: f32 = 10.0;
const SPEED_SHAKE_BIAS: f32 = 0.04;
/// `FUN_005e2a70`'s random-walk state (cam + 0x440) as recorded: it stayed at 1.0692–1.0696 for the whole of
/// `ref/live1` (model decision: its update is not understood, see decomp.md "High-speed shake").
const SPEED_SHAKE_WALK: f32 = 1.0693;
/// Stand-in for the original's curve (a 199-sample `.bss` table at 0x01188300, the ICE `SHAKE_DATA`
/// quaternion x / y / z, filled at run time and not reproducible from the code): smoothed noise with the
/// recorded per-lane mean, standard deviation and smoothness (Gaussian width in samples). Model decision.
const SPEED_SHAKE_MEAN: [f32; 3] = [-0.00486, -0.00633, -0.00587];
const SPEED_SHAKE_STD: [f32; 3] = [0.0123, 0.0106, 0.00285];
const SPEED_SHAKE_WIDTH: [f32; 3] = [3.8, 7.6, 4.0];

/// The high-speed shake (`FUN_005d0b90` -> `FUN_005e34e0` -> `FUN_005e2a70`, decomp.md "High-speed shake"):
/// a quaternion curve played at `Frequency`, scaled by `Amplitude · f^Power / 10` (f = the speed's place
/// between the shake speeds), as a small rotation of the view about a point 10 m behind the eye.
struct SpeedShakeState {
    curve: [[f32; 3]; SPEED_SHAKE_SAMPLES],
    /// Play position in seconds (cam + 0x420), wrapped at the ping-pong period.
    phase: f32,
    /// Rotation about the camera's own right, up and forward axes (radians; pitch positive = looking down,
    /// yaw positive = towards the game's right row, roll positive = the right row rising) and the drop of
    /// the eye along its own up axis (m).
    angles: Vec3,
    drop: f32,
}

impl Default for SpeedShakeState {
    fn default() -> Self {
        // Deterministic white noise from the game's LCG, smoothed per lane and normalised.
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut curve = [[0.0; 3]; SPEED_SHAKE_SAMPLES];
        for lane in 0..3 {
            let pad = 40usize;
            let noise: Vec<f32> = (0..SPEED_SHAKE_SAMPLES + 2 * pad)
                .map(|_| {
                    rng = rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
                    f32::from_bits(((rng >> 41) as u32) | 0x3f80_0000) * 2.0 - 3.0
                })
                .collect();
            let w = SPEED_SHAKE_WIDTH[lane];
            let kernel: Vec<f32> =
                (-(pad as i32)..=pad as i32).map(|k| (-(k * k) as f32 / (2.0 * w * w)).exp()).collect();
            let smooth: Vec<f32> = (0..SPEED_SHAKE_SAMPLES)
                .map(|i| kernel.iter().enumerate().map(|(k, g)| g * noise[i + k]).sum())
                .collect();
            let n = SPEED_SHAKE_SAMPLES as f32;
            let mean = smooth.iter().sum::<f32>() / n;
            let std = (smooth.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n).sqrt().max(1e-6);
            for (c, x) in curve.iter_mut().zip(smooth) {
                c[lane] = SPEED_SHAKE_MEAN[lane] + (x - mean) / std * SPEED_SHAKE_STD[lane];
            }
        }
        Self { curve, phase: 0.0, angles: Vec3::ZERO, drop: 0.0 }
    }
}

impl SpeedShakeState {
    fn step(&mut self, d: &mw_assets::camera::HighSpeedShake, mph: f32, dt: f32) {
        // FUN_005d0b90: f = clamp((speed − Minimum) / max(Maximum − Minimum, ε), 0, 1); amplitude = Amplitude ·
        // f^Power. FUN_005e2a70: the play position advances by Frequency · dt.
        let f = ((mph.abs() - d.speed[0]) / (d.speed[1] - d.speed[0]).max(f32::EPSILON)).clamp(0.0, 1.0);
        let k = d.amplitude * f.powf(d.power) / SPEED_SHAKE_DIVISOR;
        let period = (2 * SPEED_SHAKE_SAMPLES) as f32 / SPEED_SHAKE_RATE;
        self.phase = (self.phase + d.frequency * dt).rem_euclid(period);
        // FUN_009e7a30: index = phase · 30, forwards for the first 199 samples, then mirrored; interpolated
        // between neighbours (a slerp in the original; these are 1-2° rotations, so linear).
        let x = self.phase * SPEED_SHAKE_RATE;
        let i = x.floor() as usize;
        let frac = x - i as f32;
        let last = SPEED_SHAKE_SAMPLES - 1;
        let n2 = 2 * SPEED_SHAKE_SAMPLES;
        let (a, b) = if i < SPEED_SHAKE_SAMPLES {
            (i.min(last), (i + 1).min(last))
        } else {
            (n2.saturating_sub(i).min(last), (n2 - 1).saturating_sub(i).min(last))
        };
        let q: [f32; 3] = std::array::from_fn(|l| self.curve[a][l] + (self.curve[b][l] - self.curve[a][l]) * frac);
        // The quaternion's x, y, z scaled by the amplitude / 10 become the view's pitch, yaw, roll (twice the
        // half-angle); the pitch also gets amplitude / 10 · walk · 0.04 (local_30 in FUN_005e2a70). The matrix's
        // y translation is −10 · sin(pitch): the view turns about a point 10 m behind the eye.
        self.angles =
            Vec3::new(2.0 * q[0] * k + k * SPEED_SHAKE_WALK * SPEED_SHAKE_BIAS, 2.0 * q[1] * k, 2.0 * q[2] * k);
        self.drop = SPEED_SHAKE_DIVISOR * self.angles.x.sin();
    }

    /// Pre-multiplied onto the camera matrix in the original (before the impact shake). The game's camera rows
    /// are right, up, forward with right × up = forward; Bevy's camera looks down −Z, so the game's right row is
    /// Bevy's −X: looking down is a negative turn about local X, the game's yaw / roll turn the other way.
    fn apply(&self, t: &mut Transform) {
        let down = t.rotation * Vec3::NEG_Y;
        t.translation += down * self.drop;
        t.rotation *= Quat::from_euler(EulerRot::XYZ, -self.angles.x, self.angles.y, -self.angles.z);
    }
}

/// `FUN_005d12b0`'s vertical stage: spring stiffness (cam + 0x564, `_DAT_00fb07f0`) and the mass factor
/// (cam + 0x560: recorded 1.0 throughout, its writer not read).
const HEIGHT_K: f32 = 20.0;
const HEIGHT_MASS: f32 = 1.0;
/// Damping ratios, the spring's rate rising / falling (`_DAT_00fb0970` / `_DAT_00fb0980`); `FUN_005dbbc0`
/// turns them into `2 · ζ · √(k / m)` (`_DAT_00fb0600` = 2): recorded cam + 0x568 7.155 / 6.708.
const HEIGHT_ZETA: [f32; 2] = [0.8, 0.75];
/// Grounded input `−g·a_y · clamp(clamp(|g·a_y| / N, 0, 1)^E, 0, 1)`: N and E for an upward / other
/// vertical acceleration (`_DAT_00fb0960` / `0950`, `_DAT_00fb0970` / `05e0`).
const HEIGHT_NORM: [f32; 2] = [185.0, 15.0];
const HEIGHT_EXP: [f32; 2] = [0.8, 0.5];
/// Airborne input `−v_y · f`: rising `4 − 3 · k60 · clamp(v_y / 7)` (`_DAT_00fb06b0`, `09b0`, `09a0`),
/// falling `8 − 5 · k60 · clamp(mph / 60)` (`_DAT_00fb06c0`, `0790`, `0730`; mph from the `.bss` scale
/// 0x0118a060 = 2.23694, the horizontal speed).
const HEIGHT_AIR_UP: [f32; 3] = [4.0, -3.0, 7.0];
const HEIGHT_AIR_DOWN: [f32; 3] = [8.0, -5.0, 60.0];
const MPH_PER_MS: f32 = 2.23694;
/// Below this (m/s, per axis) the car counts as standing and the spring gets no input (`_DAT_00fb07a0`).
const HEIGHT_STILL: f32 = 0.1;
/// The landing window (`FUN_005d4970`'s tail): for 0.3 s after touch-down (`_DAT_00fafb8c`, flag cam + 0x716)
/// the gain is 3 (`_DAT_00fb0900`) and the falling damping ratio `0.5 + clamp((peak a_y − 10) / 490) · k60 ·
/// 0.25` (`_DAT_00fb05e0`, `0820`, `0830`, `0990`), peak a_y (cam + 0x6b0) the largest upward acceleration
/// in the window so far.
const LANDING_WINDOW: f32 = 0.3;
const LANDING_GAIN: f32 = 3.0;

/// The eye's vertical spring (`FUN_005d12b0`'s vertical stage, cam + 0x550..0x56c; the step is
/// `FUN_005bd470`). Its position is added to the eye's height in the followed frame (cam + 0x584); the
/// look-at point does not move with it, so the view tilts as the eye bobs. Replayed against `ref/live1`
/// to 6 mm RMS (tools/camreplay.py).
struct HeightSpring {
    /// Position (m, cam + 0x550) and rate (m/s, cam + 0x554).
    x: f32,
    v: f32,
    /// Damping coefficient (cam + 0x568 / 0x56c), set while the car moves and kept while it stands.
    damping: f32,
    /// cam + 0x715 / 0x716 / 0x70c / 0x6b0.
    was_airborne: bool,
    landing: bool,
    landing_time: f32,
    landing_peak: f32,
}

impl Default for HeightSpring {
    fn default() -> Self {
        Self {
            x: 0.0,
            v: 0.0,
            damping: 2.0 * HEIGHT_ZETA[0] * (HEIGHT_K / HEIGHT_MASS).sqrt(),
            was_airborne: false,
            landing: false,
            landing_time: 0.0,
            landing_peak: 0.0,
        }
    }
}

impl HeightSpring {
    /// `vel` / `accel`: the car's world velocity (m/s) and acceleration (m/s², `FUN_005d9610`'s role);
    /// `airborne`: no wheel on the ground (`FUN_008f5350`).
    fn step(&mut self, d: &mw_assets::camera::AccelerationMovement, vel: Vec3, accel: Vec3, airborne: bool, dt: f32) {
        let k60 = dt * UPDATE_HZ;
        let (mut vy, mut ay) = (vel.y, accel.y);
        if vel.x.abs() <= HEIGHT_STILL && vel.z.abs() <= HEIGHT_STILL {
            vy = 0.0;
            ay = 0.0;
        }
        let moving = vel.x.abs() > HEIGHT_STILL || vy.abs() > HEIGHT_STILL || vel.z.abs() > HEIGHT_STILL;
        if moving {
            let up = ay > 0.0;
            let gain = if self.landing { LANDING_GAIN } else { d.vertical_gain[usize::from(!up)] };
            let zeta = if self.landing && self.v < 0.0 {
                0.5 + ((self.landing_peak - 10.0) / 490.0).clamp(0.0, 1.0) * k60 * 0.25
            } else {
                HEIGHT_ZETA[usize::from(self.v < 0.0)]
            };
            self.damping = 2.0 * zeta * (HEIGHT_K / HEIGHT_MASS).sqrt();
            let input = if airborne {
                let f = if vel.y > 0.0 {
                    let [base, slope, full] = HEIGHT_AIR_UP;
                    base + slope * k60 * (vy / full).clamp(0.0, 1.0)
                } else {
                    let [base, slope, full] = HEIGHT_AIR_DOWN;
                    base + slope * k60 * (Vec2::new(vel.x, vel.z).length() * MPH_PER_MS / full).clamp(0.0, 1.0)
                };
                -vy * f
            } else {
                let i = usize::from(!up);
                let g = ay * gain;
                -g * (g.abs() / HEIGHT_NORM[i]).clamp(0.0, 1.0).powf(HEIGHT_EXP[i]).clamp(0.0, 1.0)
            };
            self.v += HEIGHT_MASS * input * dt;
        }
        // FUN_005bd470: damping, then the spring, then the position; at a limit the rate is zeroed.
        self.v -= self.damping * self.v * dt * HEIGHT_MASS;
        self.v -= HEIGHT_K * self.x * dt * HEIGHT_MASS;
        self.x += self.v * dt;
        let [lo, hi] = d.vertical_limits;
        if self.x <= lo || self.x >= hi {
            self.x = self.x.clamp(lo, hi);
            self.v = 0.0;
        }
        // FUN_005d4970's tail: a touch-down opens the landing window; while it is open its peak upward
        // acceleration is kept.
        if !airborne && self.was_airborne {
            self.landing = true;
            self.was_airborne = false;
            return;
        }
        if self.landing {
            self.landing_time += dt;
            self.landing_peak = self.landing_peak.max(accel.y);
            if self.landing_time > LANDING_WINDOW {
                self.landing = false;
                self.landing_time = 0.0;
                self.landing_peak = 0.0;
            }
        }
        self.was_airborne = airborne;
    }
}

/// The 3D camera while it follows a car (the free camera has its own controller).
pub(crate) type DrivingCamera =
    (With<Camera3d>, Without<FreeCamera>, Without<crate::reflection::PlanarReflectionCamera>);

/// The gameplay camera's systems (the crash camera overrides what they placed).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameplayCamera;

/// Set when the car is moved (the crash reset): the camera starts over behind it instead of springing
/// across the jump.
#[derive(Resource, Default)]
pub struct SnapCamera(pub bool);

/// Camera state, smoothed every frame.
#[derive(Default)]
struct CameraState {
    car: Option<Entity>,
    mode: Option<CameraMode>,
    /// The followed frame's smoothed Euler angles (cam + 0x590 in the game) and the pitch stiffness
    /// (cam + 0x6d0).
    yaw: f32,
    frame_pitch: f32,
    frame_roll: f32,
    k_pitch: f32,
    /// Orbit offset (look-around) applied on top of the camera's yaw and pitch.
    orbit: Vec2,
    /// Eye z offset (m, cam + 0x588) and its airborne rate term (m/s, cam + 0x6c8).
    accel_z: f32,
    accel_z_air: f32,
    /// The followed frame's low-passed forward height (cam + 0x4f4).
    fwd_y: f32,
    /// `FUN_005c7a80`: the deceleration pitch (rad, cam + 0x670), the gradient's look pitch (cam + 0x680) and
    /// eye drop (m, cam + 0x694), and the share of the eye's vertical spring the look-at point follows
    /// (cam + 0x6a4), with the spring's position of the previous update it uses.
    decel_pitch: f32,
    grad_pitch: f32,
    grad_eye: f32,
    look_bob: f32,
    height_prev: f32,
    fov: f32,
    roll: f32,
    /// Longitudinal acceleration (m/s²), measured between physics ticks.
    accel: f32,
    /// Acceleration vector (m/s²) between physics ticks (`FUN_005d9610`'s role) and the velocity it is
    /// measured from; the FOV's acceleration term (cam + 0x6ec).
    accel_vec: Vec3,
    last_vel: Vec3,
    fov_accel: f32,
    last_speed: f32,
    since_speed: f32,
    speed_shake: SpeedShakeState,
    height: HeightSpring,
    /// Distance from the look-at point to the eye the walls allow.
    clip: f32,
    impact: ImpactShakeState,
    trace: Option<std::fs::File>,
    time: f32,
}

/// The followed car as the camera reads it each update: `follow_car` fills it from the ECS, [`replay_recording`]
/// from a recording of the original.
pub(crate) struct Followed {
    /// The car's model origin and orientation (local +Z forward).
    pub transform: Transform,
    /// The frame the chase camera follows: the wheels' plane (`Car::wheel_frame`, vehicle + 0x2db0).
    pub ground: Quat,
    /// World velocity (m/s).
    pub velocity: Vec3,
    /// Signed speed along the car (m/s, `Car::speed`).
    pub speed: f32,
    /// Yaw rate (rad/s, positive = left).
    pub yaw_rate: f32,
    pub wheels_down: usize,
    /// Seconds since a wheel last left the ground.
    pub air_time: f32,
    pub half_extents: Vec3,
    /// m/s (the in-car views' FOV).
    pub top_speed: f32,
}

/// One update's view before collision and effects: eye, look-at point, up, and whether it is an in-car view.
struct View {
    pos: Vec3,
    target: Vec3,
    up: Vec3,
    in_car: bool,
}

impl CameraState {
    /// Starts over behind the car.
    fn reset(&mut self, car: Option<Entity>, f: &Followed, fov0: f32) {
        let trace = self.trace.take();
        let forward = f.transform.rotation * Vec3::Z;
        *self = CameraState {
            car,
            yaw: forward.x.atan2(forward.z),
            frame_pitch: f.transform.rotation.to_euler(EulerRot::YXZ).1,
            frame_roll: f.transform.rotation.to_euler(EulerRot::YXZ).2,
            k_pitch: K_PITCH_DEFAULT,
            fwd_y: (f.ground * Vec3::Z).y,
            look_bob: LOOK_BOB[0],
            fov: fov0,
            last_speed: f.speed,
            last_vel: f.velocity,
            clip: f32::MAX,
            trace,
            ..default()
        };
    }

    /// One camera update (everything but the wall push-in, which needs the world, and the effects that
    /// [`CameraState::finish`] applies).
    fn update(
        &mut self,
        cams: &VehicleCameras,
        mode: CameraMode,
        look: &LookInput,
        f: &Followed,
        contact: f32,
        dt: f32,
    ) -> View {
        let ext = &cams.external;
        let s = self;
        let ease = |rate: f32| 1.0 - (-dt * rate).exp();
        let vel = f.velocity;
        let flat_v = Vec3::new(vel.x, 0.0, vel.z);
        let speed = flat_v.length();
        let mph = speed / MPH;
        s.time += dt;

        // Longitudinal acceleration: the car's speed changes once per physics tick, so measure across ticks.
        s.since_speed += dt;
        if f.speed != s.last_speed {
            let raw = ((f.speed - s.last_speed) / s.since_speed.max(1e-3)).clamp(-40.0, 40.0);
            s.accel += (raw - s.accel) * (1.0 - (-s.since_speed * 8.0).exp());
            s.accel_vec = (vel - s.last_vel) / s.since_speed.max(1e-3);
            s.last_vel = vel;
            s.last_speed = f.speed;
            s.since_speed = 0.0;
        }

        // Orbit (look-around): follows the input, springs back when released; look-back is a cut.
        if look.look_back {
            s.orbit = Vec2::ZERO;
        } else {
            let rate = if look.held { ORBIT_FOLLOW } else { ORBIT_RETURN };
            s.orbit.x += wrap_angle(look.yaw - s.orbit.x) * ease(rate);
            s.orbit.y += (look.pitch - s.orbit.y) * ease(rate);
        }

        // Impact shake: body contacts (FUN_005cd040), and FUN_005c8e40's landing request on the first update
        // with a wheel down after the air (it runs before FUN_005d0b90's decay).
        let airborne = f.wheels_down == 0 && f.air_time > 0.0;
        if airborne {
            s.impact.airborne = true;
        } else if std::mem::take(&mut s.impact.airborne) {
            let request = ((s.accel_vec.y.abs() - LANDING_A_MIN) * LANDING_A_SCALE).clamp(0.0, 1.0) * LANDING_LEVEL;
            s.impact.level = s.impact.level.max(request);
        }
        s.impact.step(&ext.impact, contact, dt);

        // High-speed shake (FUN_005d0b90's first stage): vehicle + 0x1620, mph.
        s.speed_shake.step(&ext.shake, mph, dt);
        // The eye's vertical spring (FUN_005d12b0), driven by the car's vertical acceleration on the ground and
        // its vertical speed in the air. FUN_005c7a80 (earlier in the update) reads its previous position.
        s.height_prev = s.height.x;
        s.height.step(&ext.acceleration, vel, s.accel_vec, f.wheels_down == 0, dt);

        let in_car = match mode {
            CameraMode::Chase => None,
            CameraMode::Bonnet => cams.bonnet,
            CameraMode::Bumper => cams.bumper,
        };
        if s.mode != Some(mode) {
            s.mode = Some(mode);
            s.clip = f32::MAX;
        }
        let speed_frac = (speed / f.top_speed.max(1.0)).clamp(0.0, 1.0);
        let (pos, target, up, fov) = if let Some(c) = in_car {
            in_car_view(&c, &f.transform, s, look.look_back, speed_frac)
        } else {
            chase_view(ext, f, s, look, mph, flat_v, dt)
        };
        // The chase FOV is set directly each frame (FUN_005c76a0 has no smoothing); in-car views ease.
        s.fov = if in_car.is_none() { fov } else { s.fov + (fov - s.fov) * ease(FOV_RATE) };
        View { pos, target, up, in_car: in_car.is_some() }
    }

    /// The camera's transform from the view (after the wall push-in): the roll, then the
    /// camera effects (FUN_005d0b90): the high-speed shake (chase view only), then the impact shake.
    fn finish(&self, v: &View) -> Transform {
        let mut t = Transform::from_translation(v.pos).looking_at(v.target, v.up);
        t.rotate_local_z(self.roll.to_radians());
        if !v.in_car {
            self.speed_shake.apply(&mut t);
        }
        self.impact.apply(&mut t);
        t
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn follow_car(
    time: Res<Time>,
    settings: Res<CameraSettings>,
    mut snap: ResMut<SnapCamera>,
    look: Res<LookInput>,
    mut kicks: ResMut<ImpactKicks>,
    mut state: Local<CameraState>,
    spatial: SpatialQuery,
    cars: Query<(Entity, &Transform, &LinearVelocity, &Car), (Without<Camera3d>, With<Player>)>,
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
    let followed = Followed {
        transform: *car_t,
        ground: car.wheel_frame(car_t),
        velocity: vel.0,
        speed: car.speed,
        yaw_rate: car.yaw_rate,
        wheels_down: car.wheels_down(),
        air_time: car.air_time,
        half_extents: car.params.body_half_extents,
        top_speed: car.params.top_speed,
    };

    let forward = car_t.rotation * Vec3::Z;
    let car_yaw = forward.x.atan2(forward.z);
    if std::mem::take(&mut snap.0) {
        s.car = None;
    }
    if s.car != Some(entity) {
        s.reset(Some(entity), &followed, ext.fov[0]);
        if s.trace.is_none()
            && let Some(path) = std::env::var_os("MW_CAMERA_TRACE")
        {
            s.trace = std::fs::File::create(&path).ok();
            if let Some(f) = &mut s.trace {
                let _ = writeln!(
                    f,
                    "# t speed_kmh accel_ms2 yaw_rate lat_ms2 slip_deg mode eye_x eye_y eye_z yaw_lag_deg fov roll pitch accel_z shake_pitch shake_level speed_shake_pitch_deg speed_shake_drop eye_spring"
                );
            }
        }
    }
    let mode = settings.mode;
    let contact = std::mem::take(&mut kicks.contact);
    let view = s.update(&cams, mode, &look, &followed, contact, dt);
    // The bumper camera sits inside the body's nose (the original draws no car in that view).
    if let Ok(mut v) = car_visibility.get_mut(entity) {
        let want = if mode == CameraMode::Bumper && view.in_car { Visibility::Hidden } else { Visibility::Inherited };
        if *v != want {
            *v = want;
        }
    }
    let View { pos, target, up, in_car } = view;

    // Walls between the look-at point and the eye pull the eye in (in-car views sit inside the body).
    let mut pos = pos;
    if !in_car {
        let to_eye = pos - target;
        let full = to_eye.length();
        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider])
            .with_mask(crate::props::GameLayer::World);
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

    *cam = s.finish(&View { pos, target, up, in_car });
    let shake_pitch = s.impact.angles.x;
    if let Projection::Perspective(p) = &mut *projection {
        // The game's FOV is horizontal ([`projection_fov`]).
        p.fov = projection_fov(s.fov, p.aspect_ratio);
    }

    if let Some(f) = &mut s.trace {
        let local = car_t.rotation.inverse() * (cam.translation - car_t.translation);
        let _ = writeln!(
            f,
            "{:.3} {:.2} {:.2} {:.3} {:.2} {:.2} {:?} {:.3} {:.3} {:.3} {:.2} {:.2} {:.2} {:.2} {:.3} {:.3} {:.3} {:.4} {:.4} {:.4}",
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
            s.frame_pitch.to_degrees(),
            s.accel_z,
            shake_pitch,
            s.impact.level,
            s.speed_shake.angles.x.to_degrees(),
            s.speed_shake.drop,
            s.height.x
        );
    }
}

/// The chase camera (`FUN_005d4970`): (eye, look-at point, up, field of view).
#[allow(clippy::too_many_arguments)]
fn chase_view(
    ext: &ExternalCamera,
    car: &Followed,
    s: &mut CameraState,
    look: &LookInput,
    mph: f32,
    flat_v: Vec3,
    dt: f32,
) -> (Vec3, Vec3, Vec3, f32) {
    let k60 = dt * UPDATE_HZ;
    // Per-update fractions (the original runs at 60 Hz; not scaled by dt there) for other frame times.
    let per_update = |f: f32| 1.0 - (1.0 - f).max(0.0).powf(k60);
    let airborne = car.wheels_down == 0 && car.air_time > 0.0;

    // The car frame (vehicle + 0x2db0, the wheels' plane, not the body) with its forward's height low-passed
    // (cam + 0x4f4: 0.12 of the gap per update, `_DAT_00fb0a20`, kept within ±0.3 of the current value,
    // `_DAT_00fafb8c`), then rebuilt round the world's up (`FUN_005d4970`'s cross products with (0, 1, 0)): the
    // frame the look-at point rides in and whose Euler angles the camera follows. It has no roll.
    let ground_fwd = car.ground * Vec3::Z;
    s.fwd_y = (s.fwd_y + (ground_fwd.y - s.fwd_y) * per_update(FWD_Y_RATE))
        .clamp(ground_fwd.y - FWD_Y_RANGE, ground_fwd.y + FWD_Y_RANGE);
    let fwd = Vec3::new(ground_fwd.x, s.fwd_y, ground_fwd.z).normalize_or(ground_fwd);
    let side = Vec3::Y.cross(fwd).normalize_or(car.ground * Vec3::X);
    let followed = Quat::from_mat3(&Mat3::from_cols(side, fwd.cross(side), fwd));
    let car_yaw = fwd.x.atan2(fwd.z);

    // Yaw stiffness (`FUN_005c90a0`): K1 = `63fb2207` → `5a74d44d` over the `Yaw_spring_modification` speed band
    // (the yaw-rate term, cam + 0x6d8, is 1 with the shipped data), then towards `YawDriftSpring` (the mods
    // profile's value 0x29) by the slip angle: `K1 + (drift − K1) · clamp(|slip°| / 45)` (`.bss` 0x0110e8f0 = 45,
    // live read). Replayed on `ref/live1` to 0.00035 RMS (0.0013 with K1 alone). At a standstill the per-player
    // default (state + 0x74, `FUN_005c89c0`'s reset path).
    let ys = &ext.yaw_spring;
    let fraction = if car.velocity.length() < K_YAW_STANDING_SPEED {
        K_YAW_STANDING
    } else {
        let t = ((mph - ys.speed[0]) / (ys.speed[1] - ys.speed[0]).max(1.0)).clamp(0.0, 1.0);
        let k1 = ys.spring[1] + (ys.spring[0] - ys.spring[1]) * t;
        let slip =
            if flat_v.length() > 0.1 { wrap_angle(flat_v.x.atan2(flat_v.z) - car_yaw).to_degrees().abs() } else { 0.0 };
        k1 + (ext.yaw_drift_spring - k1) * (slip / YAW_SLIP_FULL_DEG).clamp(0.0, 1.0)
    };
    // The followed frame: the Euler angles (FUN_005e14d0; the previous ones near a vertical forward axis),
    // per-axis gains (FUN_005c89c0), one step towards them (FUN_005c75b0). No roll (recorded cam + 0x598 stays
    // within 0.25°; the view's roll is FUN_005c9380's).
    let target_pitch = if fwd.y.abs() > 1.0 - GIMBAL_EPS { s.frame_pitch } else { -fwd.y.clamp(-1.0, 1.0).asin() };
    let retain = |k: f32| 1.0 - (k * k60).clamp(0.0, 1.0);
    // vehicle + 0x1620 is mph (recording): the pitch is rigid at a standstill, smoothed above 50 mph.
    let retain_pitch = retain(s.k_pitch) * (mph.abs() * PITCH_RETAIN_PER_SPEED).min(1.0);
    s.frame_pitch += (1.0 - retain_pitch) * wrap_angle(target_pitch - s.frame_pitch);
    s.yaw = wrap_angle(s.yaw + (1.0 - retain(fraction)) * wrap_angle(car_yaw - s.yaw));
    s.frame_roll = 0.0;
    // FUN_005c8e40 (late in the update, so for the next one): the pitch stiffness eases by 0.1 per update to its
    // default, or to 0.1 while the car is in the air (vehicle + 0x1630 > 0 and no wheel down): in a jump the view
    // keeps its pitch and the car tips in front of it (recorded: 0.10-0.34 in the air).
    let k_target = if airborne { K_PITCH_AIR } else { K_PITCH_DEFAULT };
    s.k_pitch += (k_target - s.k_pitch) * per_update(K_PITCH_EASE);

    // FUN_005c7a80 (before the speed movement): the deceleration pitch, the gradient adjustments and the look-at
    // point's share of the eye's spring. Replayed on `ref/live1`: cam + 0x670 to 0.0003 rad RMS, 0x680 0.0007,
    // 0x694 0.005 m (correlations 0.999).
    let a = s.accel_vec;
    let v = car.velocity;
    let dp = &ext.pitch;
    let mut decel_target = 0.0;
    if let (Some(vh), Some(ah)) = (v.try_normalize(), a.try_normalize()) {
        let along = ah.dot(vh);
        // Travelling forwards (the car frame's forward within 10° of the velocity) and slowing down: the
        // deceleration along the velocity (m/s²) placed between the data's minimum and maximum, shaped by
        // `Pitch_change_curve_power`, times `Maximum_Pitch_Change`.
        if ground_fwd.dot(vh) > DECEL_PITCH_COS && along < 0.0 {
            let x = a.length() * along.abs();
            let range = (dp.deceleration[1] - dp.deceleration[0]).max(f32::EPSILON);
            let f = ((x - dp.deceleration[0]) / range).clamp(0.0, 1.0);
            decel_target = f.powf(dp.curve_power) * k60 * dp.max_change.to_radians();
        }
    }
    // A step of `Pitch_change_intensity` · dt of the gap (clamped to it): a first-order lag at 2/s.
    let gap = decel_target - s.decel_pitch;
    let step = dt * dp.intensity * gap;
    s.decel_pitch += step.min((-step).max(gap));
    // The gradient: the car frame's pitch (nose down positive) clamped to ±`Maximum_Gradient`; uphill uses
    // `Upwards`, downhill `Downwards`. Eased by 0.1 per update, held while airborne.
    let gr = &ext.gradient;
    let max_g = gr.max_gradient.to_radians();
    let slope = (-ground_fwd.y.clamp(-1.0, 1.0).asin()).clamp(-max_g, max_g);
    let [g_pitch, g_eye] = if slope < 0.0 { gr.upwards } else { gr.downwards };
    let g_rate = if airborne { 0.0 } else { ((K_PITCH_EASE * k60).clamp(0.0, 1.0) * k60).min(1.0) };
    s.grad_pitch += (g_pitch * slope - s.grad_pitch) * g_rate;
    s.grad_eye += (g_eye * slope - s.grad_eye) * g_rate;
    let spring = &s.height;
    let landing_u = ((spring.landing_peak - LANDING_A_MIN) * LANDING_A_SCALE).clamp(0.0, 1.0);
    let squash = spring.landing && spring.v < 0.0 && s.height_prev < 0.0;
    let (bob_target, bob_rate) = if airborne {
        (LOOK_BOB[1], LOOK_BOB_RATE)
    } else if squash {
        (LOOK_BOB[1] + LOOK_BOB[2] * landing_u * k60, LOOK_BOB_RATE + (1.0 - LOOK_BOB_RATE) * landing_u)
    } else {
        (LOOK_BOB[0], LOOK_BOB_RATE)
    };
    s.look_bob += (bob_target - s.look_bob) * (bob_rate * k60).min(1.0);

    // Speed and acceleration movement (FUN_005d12b0). h = the speed's place in the band
    // [b6010ee1, 53e4b4f9] (mph, vehicle + 0x1620); the eye and look-at heights move to their full-speed
    // values directly (cam + 0x5a4 / 0x5c4).
    let sp = &ext.speed;
    let h = ((mph.abs() - sp.min_speed) / (sp.full_speed - sp.min_speed).max(1.0)).clamp(0.0, 1.0);
    let mut eye = Vec3::from(ext.eye);
    // FUN_005c7a80's results: the look-at point turned about the eye by the deceleration pitch less the
    // gradient's (positive = looking down), raised by its share of the spring; the eye lowered by the gradient's
    // drop (raised uphill).
    let turn = Quat::from_rotation_x(s.decel_pitch - s.grad_pitch);
    let mut look_at = eye + turn * (Vec3::from(ext.look_at) - eye);
    look_at.y += s.height_prev * s.look_bob;
    eye.y -= s.grad_eye;
    eye.y += (sp.eye_y - ext.eye[1]) * h;
    look_at.y += (sp.look_y - ext.look_at[1]) * h;

    // FUN_005c76a0: FOV = lerp(min, max, clamp(mph / 125)) + state + 0x90 (10) × cam + 0x284 (0 in the
    // recording) + the acceleration term cam + 0x6ec. Its rates are per update (60 Hz in the original).
    let a_target = if fwd.dot(a) > 0.0 && fwd.dot(flat_v) > 0.0 { a.length().min(FOV_ACCEL_MAX) } else { 0.0 };
    let a_rate = if a_target > s.fov_accel { FOV_ACCEL_RISE } else { FOV_ACCEL_FALL };
    s.fov_accel += (a_target - s.fov_accel) * per_update(a_rate);
    // The speed fraction is raised to the power max / min (`powf` with `state + 0x80 / state + 0x7c`, from the
    // disassembly; the decompiler dropped it): 95 / 70 = 1.36, so the FOV widens late. With it the acceleration
    // term's factor fits 1.00 per 60 Hz update on `ref/live1` (FOV RMS 0.44° against 2.3° without the power).
    let frac = (mph.abs() / FOV_FULL_MPH).clamp(0.0, 1.0);
    let fov = ext.fov[0] + (ext.fov[1] - ext.fov[0]) * frac.powf(ext.fov[1] / ext.fov[0].max(1.0)) + s.fov_accel;

    // FUN_005c8ad0: the eye closes in as the car turns side-on to the camera (yaw lag), by the body's
    // width-to-length ratio. The original's box is the target object's +0xc0..+0xd8 (not identified; the
    // body's collision box here).
    let lag = wrap_angle(s.yaw - car_yaw);
    let half = car.half_extents;
    let ratio = if half.z > 0.1 { half.x / half.z } else { 1.0 };
    let side_on = 1.0 + lag.sin().abs() * (ratio - 1.0) * SIDE_ON_WEIGHT;
    eye.x *= side_on;
    eye.z *= side_on;
    // FUN_005c1f90: the FOV dolly along the eye's line to the look-at point.
    let dolly = DOLLY_C + DOLLY_OFFSET - DOLLY_C / (fov.to_radians() * 0.5).tan();
    if let Ok(dir) = Dir3::new(look_at - eye) {
        eye += *dir * dolly;
    }

    // The eye's z offset (FUN_005d12b0, cam + 0x588; `.bss` constants read from a memory snapshot): target =
    // 0a78c15f · h + (bd3a7cfc accelerating | 7c9039e2 decelerating) · |a_long| / 22.352, a_long = the
    // acceleration along the velocity clamped to ±22.352 m/s² (`DAT_0110e770`); the rate (cam + 0x578) is
    // (fdd25ee3 | 285f2260) times the gap clamped to ±0.75 m (`DAT_0110e4d0`), zero below 0.1 m/s
    // (`DAT_0110e610` on the squared speed), less a term that eases (0.1 per update) to 1.85 m/s while the car
    // is airborne above 5 mph (`DAT_0110e640`; cam + 0x6c8) and to 0 otherwise; the offset is clamped to ±4 m
    // (`DAT_0110e680`). Replayed on `ref/live1` to 0.011 m RMS (recorded std 0.32 m).
    let am = &ext.acceleration;
    let (mut z_target, mut decel) = (0.0, false);
    if v.abs().max_element() > ACCEL_EPS && a.abs().max_element() > ACCEL_EPS {
        let a_long = a.dot(v.normalize()).clamp(-ACCEL_FULL, ACCEL_FULL);
        decel = a_long < 0.0;
        z_target = sp.eye_z * h + if decel { am.forward } else { am.back } * a_long.abs() / ACCEL_FULL;
    }
    let w = if decel { am.brake_rate } else { am.accel_rate };
    // Model decision: the rate's `g · k60 · (1 − w)` term (g = vehicle + 0x2b20, a value in [−1, 1] that is 0 on
    // 94 % of `ref/live1`'s updates, not identified) is left out: 0.028 m RMS against the recording, 0.011 with it.
    let mut rate = if v.length_squared() < ACCEL_Z_STILL {
        0.0
    } else {
        w * (z_target - s.accel_z).clamp(-ACCEL_Z_STEP, ACCEL_Z_STEP)
    };
    let air_target = if airborne && mph.abs() > ACCEL_Z_AIR_MPH { ACCEL_Z_AIR } else { 0.0 };
    s.accel_z_air += (air_target - s.accel_z_air) * ACCEL_Z_AIR_RATE * k60;
    rate -= s.accel_z_air;
    s.accel_z = (s.accel_z + rate * dt).clamp(-ACCEL_Z_LIMIT, ACCEL_Z_LIMIT);

    // Roll (FUN_005c9380, cam + 0x660): the body's acceleration (m/s², the velocity's change per update)
    // along the car frame's row 0 (forward × up: the car's right side, Bevy local −X) and its yaw rate (rad/s),
    // each placed in its range; magnitude = Maximum_Roll_Angle · max(f_lat, f_yaw)^power, cam + 0x660 =
    // −magnitude while the car accelerates to its right, else +magnitude. The view's right side rises by
    // −cam + 0x660 (recorded slope −1.06), so `s.roll` (that rise, degrees) is +magnitude in a right turn:
    // the camera leans out of the turn.
    let r = &ext.roll;
    let lat = a.dot(car.ground * Vec3::NEG_X);
    let band = |x: f32, [lo, hi]: [f32; 2]| ((x.abs() - lo) / (hi - lo).max(ROLL_EPS)).clamp(0.0, 1.0);
    let f = band(lat, r.lateral).powf(r.factor_power).max(band(car.yaw_rate, r.angular).powf(r.factor_power));
    let magnitude = f * r.max_angle;
    // In cam + 0x660's sign: a step of intensity · π/180 · 20 · dt of the gap (0.7/s at intensity 2), the
    // code's clamp of the step to [−step, gap].
    let state = -s.roll;
    let error = if lat < 0.0 { magnitude } else { -magnitude } - state;
    let step = r.intensity * ROLL_RATE.to_radians() * dt * error;
    s.roll = -(state + step.min((-step).max(error)));
    // `Deceleration_pitch_change` is read by no function of the chase update, and the recorded view direction
    // is the line from the eye to the look-at point to 0.15° RMS (the high-speed shake): no braking pitch.

    // The composition (FUN_005c2130): the eye's offsets cam + 0x580..0x58c (the vertical spring's position and
    // the z offset) are scaled by |cos(yaw lag)|, the eye (less the pivot, cam + 0x5b0 = state + 0x50) is turned
    // by the smoothed angles and the pivot added back along the followed frame's forward, also × |cos lag|; the
    // look-at point rides in the followed frame itself (not the smoothed one), so the view turns to the car while
    // the eye swings out: recorded forward vs eye → look-at 0.15° RMS (pitch) / 0.035° (yaw). The look-around
    // turns the eye round the pivot; looking back turns it right round.
    let c = lag.cos().abs();
    let pivot = Vec3::from(ext.pivot);
    let offset = Vec3::new(0.0, s.height.x, s.accel_z) * c;
    let back = if look.look_back { PI } else { 0.0 };
    let smoothed = Quat::from_euler(EulerRot::YXZ, s.yaw, s.frame_pitch, s.frame_roll);
    let orbit = Quat::from_rotation_y(s.orbit.x + back) * Quat::from_rotation_x(s.orbit.y);
    let pos = car.transform.translation;
    let eye_w = pos + smoothed * (orbit * (eye + offset - pivot)) + fwd * (pivot.z * c);
    let look_w = pos + followed * (pivot + Quat::from_rotation_y(s.orbit.x + back) * (look_at - pivot));
    (eye_w, look_w, smoothed * Vec3::Y, fov)
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
    s.accel_z = 0.0;
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

/// The game's field of view (degrees) is horizontal; Bevy's projection takes the vertical one (radians) for its
/// aspect ratio. Captures of the original at 1280×720 (2026-10-09, docs/research-log.md): the 4C at rest in the
/// chase view (`MinFieldOfView` 70) and the first shot of 462174's intro (35 mm, `2 atan(18 / f)` = 54.4°) both
/// frame 1.78× (the aspect ratio) tighter than the same values read as vertical. `MW_FOV_VERTICAL=1` reads them
/// as vertical again (A/B).
pub fn projection_fov(horizontal_deg: f32, aspect: f32) -> f32 {
    let h = horizontal_deg.clamp(1.0, 170.0).to_radians();
    if std::env::var_os("MW_FOV_VERTICAL").is_some() || aspect.is_nan() || aspect <= 0.0 {
        return h;
    }
    2.0 * ((h * 0.5).tan() / aspect).atan()
}

/// The inverse of [`projection_fov`]: the game's horizontal FOV (degrees) of a projection.
pub fn game_fov(p: &PerspectiveProjection) -> f32 {
    if std::env::var_os("MW_FOV_VERTICAL").is_some() || p.aspect_ratio.is_nan() || p.aspect_ratio <= 0.0 {
        return p.fov.to_degrees();
    }
    (2.0 * ((p.fov * 0.5).tan() * p.aspect_ratio).atan()).to_degrees()
}

/// `--camera-replay <inputs>,<out>`: runs the chase camera offline over a followed car's recorded motion (one
/// update per line of `inputs`, a CSV with the header `t,dt,car,px,py,pz,qx,qy,qz,qw,gqx,gqy,gqz,gqw,vx,vy,vz,speed,
/// yaw_rate,wheels_down,air_time`; `car` is a vehicle id whose camera data is loaded from the install) and writes the
/// camera per update: eye, forward, up, FOV and the main state values. `tools/camcompare.py` makes the inputs
/// from a recording of the original (`ref/<name>/cam.bin`) and compares the result with the original's camera.
pub fn replay_recording(
    store: &mw_assets::AssetStore,
    input: &std::path::Path,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    let text = std::fs::read_to_string(input)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let col = |name: &str| header.iter().position(|h| *h == name).ok_or_else(|| anyhow::anyhow!("no column {name}"));
    let names = [
        "t",
        "dt",
        "car",
        "px",
        "py",
        "pz",
        "qx",
        "qy",
        "qz",
        "qw",
        "gqx",
        "gqy",
        "gqz",
        "gqw",
        "vx",
        "vy",
        "vz",
        "speed",
        "yaw_rate",
        "wheels_down",
        "air_time",
    ];
    let idx: Vec<usize> = names.iter().map(|n| col(n)).collect::<anyhow::Result<_>>()?;
    let mut cars: HashMap<String, crate::drive::CarParams> = HashMap::new();
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    writeln!(
        out,
        "t,ex,ey,ez,fx,fy,fz,ux,uy,uz,fov,yaw,frame_pitch,height,accel_z,fov_accel,roll,accel_z_air,speed_shake_pitch,impact_level"
    )?;
    let mut state = CameraState::default();
    let mut current = String::new();
    let look = LookInput::default();
    for line in lines {
        let v: Vec<&str> = line.split(',').collect();
        let num = |i: usize| v.get(idx[i]).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
        let car = v.get(idx[2]).copied().unwrap_or_default().to_string();
        if !cars.contains_key(&car) {
            let info = store.find_vehicle(&car)?.ok_or_else(|| anyhow::anyhow!("no car {car}"))?;
            let path = info.hi_bundle.ok_or_else(|| anyhow::anyhow!("{car} has no bundle"))?;
            let bundle = store.bundle(&path).ok_or_else(|| anyhow::anyhow!("bundle not found: {path}"))?;
            let h = mw_assets::handling::VehicleHandling::from_bundle(store, &bundle)?;
            println!("camera replay: car {car} = {} ({path})", info.name);
            cars.insert(car.clone(), crate::drive::CarParams::new(&h));
        }
        let params = &cars[&car];
        let cams = params.camera.unwrap_or_else(|| VehicleCameras {
            external: ExternalCamera::default(),
            bonnet: None,
            bumper: None,
            rear_view: None,
        });
        let f = Followed {
            transform: Transform::from_xyz(num(3), num(4), num(5))
                .with_rotation(Quat::from_xyzw(num(6), num(7), num(8), num(9)).normalize()),
            ground: Quat::from_xyzw(num(10), num(11), num(12), num(13)).normalize(),
            velocity: Vec3::new(num(14), num(15), num(16)),
            speed: num(17),
            yaw_rate: num(18),
            wheels_down: num(19) as usize,
            air_time: num(20),
            half_extents: params.body_half_extents,
            top_speed: params.top_speed,
        };
        if car != current {
            state.reset(None, &f, cams.external.fov[0]);
            current = car.clone();
        }
        let dt = num(1).max(1e-4);
        let view = state.update(&cams, CameraMode::Chase, &look, &f, 0.0, dt);
        let t = state.finish(&view);
        let (fw, up) = (t.rotation * Vec3::NEG_Z, t.rotation * Vec3::Y);
        let s = &state;
        let row = [
            num(0),
            t.translation.x,
            t.translation.y,
            t.translation.z,
            fw.x,
            fw.y,
            fw.z,
            up.x,
            up.y,
            up.z,
            s.fov,
            s.yaw,
            s.frame_pitch,
            s.height.x,
            s.accel_z,
            s.fov_accel,
            s.roll,
            s.accel_z_air,
            s.speed_shake.angles.x,
            s.impact.level,
        ];
        writeln!(out, "{}", row.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))?;
    }
    Ok(())
}
