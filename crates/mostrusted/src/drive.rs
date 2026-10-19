//! Drivable cars: a point-mass-wheel suspension vehicle on avian3d, and input. The
//! cameras that follow the car are in `camera.rs`.
//!
//! The game's handling code is in the encrypted executable, so this is a
//! reimplementation driven by the game's own handling data
//! ([`VehicleHandling`], from each car's GeneSys objects): mass and centre of
//! mass, wheel sizes, spring frequencies and damping ratios, the engine torque
//! curve, gear ratios and shift points, brake torques, top speed, the tyres'
//! lateral and longitudinal grip curves, the speed-banded steering, the
//! handbrake's speed bands and the drift state (`drift.rs`). Parts
//! the data does not pin down (the scale of the grip coefficients) are
//! calibrated constants; see docs/handling.md.

use avian3d::prelude::*;
use bevy::input::gamepad::RawGamepadEvent;
use bevy::math::DVec3;
use bevy::prelude::*;
use mw_assets::camera::VehicleCameras;
use mw_assets::handling::{LateralCurve, LongCurve, SlipBlend, Steering, SurfaceLink, TyreAxle, VehicleHandling};
use mw_handling::surface::{self, SurfaceGrip};
use mw_handling::suspension;
use mw_handling::tyre_vfx;

use crate::chassis::ChassisParams;
use crate::drift::{self, DriftInput, DriftParams, DriftState};
use crate::drivetrain::{Drivetrain, DrivetrainParams};
use crate::loader::CarSpec;
use crate::tyre;

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        // Test scripting runs in the fixed schedule, right before the physics,
        // so every run feeds the same input on the same tick and `--test`
        // results are reproducible.
        // The game steps vehicles at a fixed 60 Hz (once per tick, no sub-steps); per-tick data rates
        // (drift damping, decay) assume it.
        app.insert_resource(Time::<Fixed>::from_hz(60.0))
            .init_resource::<CarInput>()
            .add_systems(Update, (spawn_car, read_input, wheel_visuals, log_car))
            .add_systems(FixedUpdate, (run_test, ground_test, car_physics).chain());
    }
}

/// A car waiting for ground under its spawn point.
#[derive(Resource)]
pub struct PendingCar {
    pub spec: CarSpec,
    pub spawn: Vec3,
    /// Yaw in radians; 0 faces +Z.
    pub heading: f32,
    pub parts: CarParts,
    /// The ground ray starts this far above `spawn` (m): 30 at startup, small for a swap in place so a
    /// bridge overhead is not mistaken for the ground.
    pub probe: f32,
    /// Initial velocity (m/s): a swapped car keeps the old one's.
    pub velocity: Vec3,
}

/// Render parts for each body / wheel model, resolved on the main thread.
pub type CarParts = std::collections::HashMap<mw_bundle::ResourceId, crate::ModelParts>;

#[derive(Resource, Default)]
pub struct CarInput {
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
    pub handbrake: bool,
    /// Nitrous held (Left Shift / pad A; set by `--test nitrous` in scripted runs).
    pub nitrous: bool,
    pub reset: bool,
    /// Fixed throttle for automated runs (`--auto-throttle`).
    pub auto_throttle: Option<f32>,
    /// Hold nitrous in automated runs (`--nitrous`).
    pub auto_nitrous: bool,
    /// Scripted test in progress (`--test`): throttle, brake, steer, handbrake
    /// (> 0.5 = pulled). Overrides other input.
    pub scripted: Option<[f32; 4]>,
    /// The car is crashed (`crash.rs`): no control until the reset (`Player.CanDriveAway`).
    pub crashed: bool,
}

/// Signed forward speed below which the standstill hold acts (`.bss` 0x01186e60 = −2 mph, read live
/// 2026-10-06): only when rolling backwards faster than 2 mph, so it never acts at a standstill or on a launch.
const STANDSTILL_HOLD_MPH: f32 = -2.0;

/// Body speed (m/s) above which a car with no wheel on the ground counts as airborne (`FUN_009ae840`, `.bss`
/// 0x01186630 = 1.8, read live 2026-10-06); slower, the time on the ground keeps growing.
const AIRBORNE_MIN_SPEED: f32 = 1.8;

/// Physical parameters of a car, derived from the game data.
#[derive(Debug, Clone)]
pub struct CarParams {
    pub mass: f32,
    /// Standstill hold deceleration (`Brakes.99407bbc`, m/s²; `FUN_009b1180`).
    pub hold_decel: f32,
    pub inertia: Vec3,
    /// The body's half extents (m; x width, z length): the chase camera's side-on pull-in (`FUN_005c8ad0`).
    pub body_half_extents: Vec3,
    pub centre_of_mass: Vec3,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// Engine, clutch, gearbox (`drivetrain.rs`).
    pub drivetrain: DrivetrainParams,
    /// `Top_speed_MPH` (m/s): not read by the game's physics; the camera's FOV scales with it.
    pub top_speed: f32,
    pub handbrake: Handbrake,
    /// Front, rear.
    pub tyres: [TyreAxle; 2],
    pub steering: Steering,
    /// Drag, lift, anti-roll bars, airborne behaviour (`chassis.rs`).
    pub chassis: ChassisParams,
    /// The car's cameras (`camera.rs`); `None` uses the 911's.
    pub camera: Option<VehicleCameras>,
    /// The game's drift model (`None` if the car has no drift data).
    pub drift: Option<DriftParams>,
    /// How the tyres treat each surface (grip blend, slide drag): `VehicleSurfaceProfile` links.
    pub surface_links: Vec<SurfaceLink>,
    /// `TyreVfxBehaviour` (front, rear): the tyre smoke and tyre mark ramps (`mw_handling::tyre_vfx`).
    pub tyre_vfx: Option<[mw_assets::handling::TyreVfxAxle; 2]>,
    /// The AI driver's steering PID gains (Proportional, Integral, Derivative), `Vehicle_Gameplay.PIDController`
    /// (`ai_driver.rs`; the 911's 0.3 / 0.01 / 0.1 when a car has none).
    pub ai_steer_pid: [f32; 3],
    /// `Vehicle_Gameplay` damage stats for car-against-car impacts (`car_impacts.rs`).
    pub damage_stats: mw_assets::crash::VehicleDamageStats,
}

/// `HandbrakeAbility`: the handbrake's strength blends from its low-speed to its high-speed value between
/// `Low_speed_mph` and `High_speed_mph` (`FUN_009b1180`), and it stays on for `Minimum_handbrake_time` after it is
/// let go (`FUN_009d27b0`).
#[derive(Debug, Clone, Copy)]
pub struct Handbrake {
    /// `Brake_strength_at_low_speed` / `_at_high_speed` (N m per rear wheel).
    pub strength: [f32; 2],
    /// `Low_speed_mph`, `High_speed_mph`.
    pub speeds_mph: [f32; 2],
    pub min_time: f32,
}

impl Handbrake {
    fn new(h: &VehicleHandling) -> Self {
        let f = |name: &str, default: f32| h.param(&format!("Handbrake.{name}")).unwrap_or(default);
        let high = h.handbrake_torque.unwrap_or(1200.0);
        Self {
            strength: [f("Brake_strength_at_low_speed", high), f("Brake_strength_at_high_speed", high)],
            speeds_mph: [f("Low_speed_mph", 30.0), f("High_speed_mph", 135.0)],
            min_time: f("Minimum_handbrake_time", 0.0).clamp(0.0, 1.0),
        }
    }

    /// Strength in N m per rear wheel at `speed` (m/s, signed: reversing counts as the low speed).
    fn at(&self, speed: f32) -> f32 {
        let [lo, hi] = self.speeds_mph;
        let t = ((speed / MPH - lo) / (hi - lo).max(f32::EPSILON)).clamp(0.0, 1.0);
        self.strength[0] + (self.strength[1] - self.strength[0]) * t
    }
}

/// Bump travel above the rest position (m) for a car without the strut data (`wheel_layout`; ours).
const BUMP_TRAVEL: f32 = 0.06;
fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x.max(0.0) } else { 0.0 }
}
/// The game writes a tyre's force caps only while its contact's `+0x14` lane exceeds 0.5 (`FUN_009c5da0`,
/// `DAT_00f8a810`); read as the ground normal's world-up component (inferred), so a wheel against a wall or a
/// steep bank has no tyre grip.
const TYRE_NORMAL_MIN_UP: f32 = 0.5;
/// Floor on the slide-drag ramp's slip-angle span (`DAT_01025ab0`, FLT_EPSILON in `FUN_009cdd90`).
const DRAG_SLIP_SPAN_MIN: f32 = f32::EPSILON;
pub(crate) const MPH: f32 = 0.44704;
/// Tyre width (m) for a car without wheel bodies (model decision).
const TYRE_WIDTH_DEFAULT: f32 = 0.25;
/// Steering outwards at the full steer rate needs |raw input| ≥ this; below it the game uses the raw
/// `High_speed_steer_rate` (`FUN_009d73f0`; `.bss` 0x01186e90, read from the running game).
const STEER_FULL_INPUT: f32 = 0.99;

/// Used when a car lacks tyre data: a generic tyre on both axles.
fn default_tyres() -> [TyreAxle; 2] {
    let lat = LateralCurve { gradient: 0.8, peak: 2.8, peak_angle: 10.0 };
    let long = LongCurve { peak: 1.8, peak_ratio: 0.1, max_slip: 1.5, max_slip_ratio: 0.6 };
    let axle = TyreAxle {
        pure_lateral: lat,
        brake_slip_lateral: lat,
        traction_slip_lateral: lat,
        brake_pure_long: long,
        traction_pure_long: long,
        brake_slip_long: long,
        traction_slip_long: long,
        brake_lateral_blend: SlipBlend { exponent: 1.25, full: 0.25, max: 1.0, start: 0.0 },
        traction_lateral_blend: SlipBlend { exponent: 1.5, full: 0.8, max: 1.0, start: 0.0 },
        brake_long_blend: SlipBlend { exponent: 1.25, full: 10.0, max: 1.0, start: 0.0 },
        traction_long_blend: SlipBlend { exponent: 1.25, full: 10.0, max: 1.0, start: 0.0 },
        camber: [7.5, 75.0, 0.3],
        lateral_scale: 1.0,
        longitudinal_scale: 1.0,
    };
    [axle, axle]
}

fn default_steering() -> Steering {
    Steering {
        medium_speed_mph: 65.0,
        high_speed_mph: 135.0,
        range: [35.0, 20.0, 35.0],
        rate: [32.0, 22.0, 40.0],
        counter_rate: [90.0, 42.0, 80.0],
        center_rate: [42.0, 10.0, 80.0],
        reverse_speed_mph: -30.0,
        throttle_range: [24.0, 19.0],
        max_angle: 35.0,
        neutral_gain: 0.3,
        offset_range_gain: [1.0, 0.0],
    }
}

impl CarParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let mass = h.mass.unwrap_or(1450.0);
        let ext = h.body_half_extents.unwrap_or([0.9, 0.55, 2.2]);
        // Solid box with the body's half extents, times the body's inertia scale.
        let inertia = Vec3::new(
            mass / 3.0 * (ext[1] * ext[1] + ext[2] * ext[2]),
            mass / 3.0 * (ext[0] * ext[0] + ext[2] * ext[2]),
            mass / 3.0 * (ext[0] * ext[0] + ext[1] * ext[1]),
        ) * Vec3::from(h.inertia_scale.unwrap_or([1.0; 3]));
        Self {
            mass,
            hold_decel: h.brake_hold_decel.unwrap_or(15.0),
            inertia,
            body_half_extents: Vec3::from(ext),
            centre_of_mass: Vec3::from(h.centre_of_mass.unwrap_or([0.0, 0.4, 0.0])),
            idle_rpm: h.idle_rpm.unwrap_or(900.0),
            redline_rpm: h.redline_rpm.unwrap_or(7000.0),
            drivetrain: DrivetrainParams::new(h),
            top_speed: h.top_speed_mph.unwrap_or(180.0) * 0.44704,
            handbrake: Handbrake::new(h),
            tyres: h.tyres.unwrap_or_else(default_tyres),
            steering: h.steering.unwrap_or_else(default_steering),
            chassis: ChassisParams::new(h),
            camera: h.camera,
            drift: DriftParams::new(h),
            surface_links: h.surface_links.clone(),
            tyre_vfx: h.tyre_vfx,
            ai_steer_pid: h.ai_steering_pid.unwrap_or([0.3, 0.01, 0.1]),
            damage_stats: h.damage_stats.unwrap_or_default(),
        }
    }
}

#[derive(Component)]
pub struct Car {
    wheels: Vec<WheelState>,
    steer: f32,
    pub speed: f32,
    /// Yaw rate (rad/s, positive = left) and lateral acceleration (m/s²).
    pub yaw_rate: f32,
    pub lateral_accel: f32,
    /// Angle between heading and velocity (rad).
    pub body_slip: f32,
    /// The game's drift state (see `drift.rs`).
    pub drift: DriftState,
    /// Mean slip angle of the rear tyres on the last tick (degrees, positive = nose left of their travel):
    /// the game's drift entry and exit measure this.
    pub rear_slip: f32,
    /// Seconds since a wheel last left the ground (0 while all are down).
    pub(crate) air_time: f32,
    /// The handbrake's countdown (reset to `Minimum_handbrake_time` on every tick it is pulled) and whether it is
    /// on (`FUN_009d27b0`).
    handbrake_timer: f32,
    pub handbrake_on: bool,
    /// Seconds since the body last touched the world, and the low-speed turn-around assist's state
    /// (`FUN_008f8360`).
    wall_time: f32,
    turn_assist: TurnAssist,
    /// The yaw damping after a collision (`FUN_008f7950`).
    impact_yaw: ImpactYawDamping,
    /// Body roll and pitch relative to the ground plane under the wheels (rad;
    /// roll positive = right side down, pitch positive = nose up).
    pub roll: f32,
    pub pitch: f32,
    /// The body collider touched the world on the last physics step (`--test landing / climb` logs it).
    pub body_contact: bool,
    pub gear: i32,
    pub rpm: f32,
    /// Engine rpm at the previous physics tick (the audio interpolates between the two).
    pub rpm_prev: f32,
    /// Engine, clutch and gearbox (`drivetrain.rs`); `gear` and `rpm` mirror it.
    pub drivetrain: Drivetrain,
    /// Nitrous tank and data (`mw_handling::nitrous`).
    pub nitrous: mw_handling::nitrous::Nitrous,
    pub nitrous_params: mw_handling::nitrous::NitrousParams,
    /// Brake pressure build-up (0..1).
    brake_ramp: f32,
    /// Diagnostics from the last physics step (N): engine force at the axle,
    /// sum of applied longitudinal tyre forces, drag + rolling resistance.
    pub debug_forces: [f32; 3],
    /// Yaw moment of the front and rear tyre forces about the centre of mass
    /// (N m, positive = left), from the last physics step.
    pub tyre_yaw: [f32; 2],
    spawn: Vec3,
    pub params: CarParams,
    /// Wheel rays must not hit the car's own collider (a child entity).
    pub(crate) body_collider: Entity,
    /// The low bumper box that only meets props; its contacts count as object contacts.
    pub(crate) bumper_collider: Entity,
    /// The strongest closing speed (m/s) against a knocked prop on the last physics step. Those contacts are
    /// solved by `props::prop_contacts` (the game's scaled car-against-object response), not by avian, so
    /// they do not show in its contact graph.
    pub(crate) prop_impact: f32,
    /// The strut stops' total impulse on the chassis on the last physics step (N s; telemetry).
    pub(crate) stop_impulse: f32,
}

struct WheelState {
    /// Wheel centre at rest, car space.
    centre: Vec3,
    radius: f32,
    front: bool,
    spring: f32,
    /// Damping (N s/m) in compression and in rebound.
    damper: f32,
    rebound: f32,
    /// Total suspension range: droop below the rest position (the static sag) plus the bump travel.
    travel: f32,
    /// The strut's bottom below the rest position (m): `caac331c` + `7fc0dc41` (strut object +0x50), the strut
    /// joint's lower end (`wheel_layout`); the static sag without the data. And the joint's position parameter
    /// (+0x30: 0 at the bottom, 1 at the top), taken from the wheel's position before the sub-steps.
    droop: f32,
    strut_t: f32,
    /// Compression under the car's static weight. The graphics spec's wheel position is the wheel at rest,
    /// so the wheel sits there at this compression, droops below it, and rises at most `travel - sag` above.
    sag: f32,
    brake_torque: f32,
    /// Wheel spin (rad/s), rotational inertia (kg m², with the transmission's share on driven wheels) and
    /// the sprung mass on this corner (kg; the tyre constraint's effective mass).
    omega: f32,
    inertia: f32,
    corner_mass: f32,
    /// Slip ratio after the tyre pass and slip angle (degrees) at the last tick, for telemetry (`--replay`) and
    /// the slide drag.
    slip: (f32, f32),
    /// Current suspension compression (m): where the wheel's point mass stands on its strut, `sag` at rest (the
    /// wheel at its spec position); and its rate (m/s, positive = compressing).
    compression: f32,
    compression_rate: f32,
    /// How far the ground reached past full compression on the last tick (m): the ground inside a tyre centred at
    /// the strut's end.
    overtravel: f32,
    /// The strut stop's slack (m): the tyre's sidewall, wheel radius − rim radius (constraint +0x470).
    stop_slack: f32,
    /// The wheel as the game's solver has it (`mw_handling::suspension`): a point mass (the wheel body's data mass,
    /// wheel record +0x50) on the strut. Its centre in the world after the last tick's integration (`None` until the
    /// first tick: at rest), its velocity along the car's up axis (m/s), whether its ground contact was
    /// activated by the last contact pass (`FUN_009c5a90` / `FUN_009c52d0`), the strut stop's share from that pass
    /// (constraint +0x474 while +0x4ae is set) and the ground's normal impulse on it this tick (N s).
    mass: f32,
    world: Option<Vec3>,
    vel: f32,
    touching: bool,
    stop_share: Option<f32>,
    normal_impulse: f32,
    /// The wheel's velocity along the strut after this tick's sub-steps, before the contact passes (telemetry: what
    /// the original's wheel record holds between ticks).
    vel_substeps: f32,
    spin: f32,
    pivot: Entity,
    spinner: Entity,
    /// Ground contact in the last physics step (read by `particles.rs`).
    contact: Option<WheelContact>,
    /// The grip of the surface under the wheel and of those it just left (`mw_handling::surface`).
    surface: SurfaceGrip,
    /// The wheel's tyre smoke and tyre mark ramps (`FUN_009cef90`: vehicle +0x2c90 + 0x30 i, lanes 0..2 and
    /// 4, 5, 7).
    smoke: tyre_vfx::Ramps,
    marks: tyre_vfx::Ramps,
    /// Full tyre width (m): twice the wheel body's half extent across the car (the strip width the game passes
    /// with each tyre mark point is wheel +0xbc, read here as the tyre width; not traced).
    width: f32,
}

/// A wheel's ground contact, for effects (tyre smoke, kick-up). World space.
#[derive(Clone, Copy, Debug)]
pub struct WheelContact {
    pub point: Vec3,
    pub normal: Vec3,
    /// Velocity of the contact patch over the ground (m/s): sideways sliding, plus the full ground speed
    /// for a locked wheel and an assumed spin-up for a wheel spinning under power.
    pub slip: Vec3,
    /// Velocity of the car at the contact point (m/s).
    pub velocity: Vec3,
    /// Tread speed minus ground speed along the tyre (m/s): positive spinning, negative locked / dragged.
    pub tread_slip: f32,
    /// The surface's `VehicleEffects` (dry block): how fast it makes the car dirty (`paint_wear.rs`).
    pub effects: Option<mw_assets::surfaces::VehicleEffects>,
    /// The surface under the wheel: its index in the game's surface list (`Surfaces.Surfaces`, the collision
    /// tag's bits 13–19; `mw_assets::vfx::VfxLibrary::surfaces` has the same order).
    pub surface: Option<usize>,
    /// The wheel's smoothed smoke and tyre mark intensities (`mw_handling::tyre_vfx`).
    pub smoke: tyre_vfx::Ramps,
    pub marks: tyre_vfx::Ramps,
    /// Full tyre width (m).
    pub width: f32,
    /// The tyre's lateral axis on the ground (unit; the steered wheel's, from the tyre frame).
    pub across: Vec3,
}

impl Car {
    /// Wheels touching the ground (the camera's landing trigger).
    pub(crate) fn wheels_down(&self) -> usize {
        self.wheels.iter().filter(|w| w.contact.is_some()).count()
    }

    /// The car frame the game's cameras follow (vehicle + 0x2db0, built each update by `FUN_0090a5f0`, read
    /// by `FUN_005dbe80`): forward = (front − rear) summed over both sides, across = (one side − the other)
    /// summed over both axles, of four points (vehicle + 0x14..+0x20; by the frame's behaviour in `ref/live1`
    /// the wheels: it leaves out the body's roll, 3° RMS, and its pitch under acceleration, which the
    /// physics body carries). Here the wheel hubs (rest position plus the suspension's compression).
    pub(crate) fn wheel_frame(&self, t: &Transform) -> Quat {
        let (mut fwd, mut across) = (Vec3::ZERO, Vec3::ZERO);
        for w in &self.wheels {
            let p = t.transform_point(w.centre + Vec3::Y * (w.compression - w.sag));
            fwd += if w.front { p } else { -p };
            across += if w.centre.x > 0.0 { p } else { -p };
        }
        let (Some(z), Some(x)) = (fwd.try_normalize(), across.try_normalize()) else { return t.rotation };
        // Bevy's car space: +Z forward, +Y up, +X = Y × Z.
        let Some(y) = z.cross(x).try_normalize() else { return t.rotation };
        let x = y.cross(z);
        Quat::from_mat3(&Mat3::from_cols(x, y, z))
    }

    /// Each wheel's (slip ratio, slip angle in degrees) from the last physics step, front wheels first in spec
    /// order.
    pub fn wheel_slips(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.wheels.iter().map(|w| w.slip)
    }

    /// Each wheel's spin (rad/s), in the order of [`Car::wheel_slips`].
    pub fn wheel_spins(&self) -> impl Iterator<Item = f32> + '_ {
        self.wheels.iter().map(|w| w.omega)
    }

    /// The game's placement of a car moving at `speed` (m/s along its heading; the caller sets the body's
    /// velocity): `FUN_008f8190` with the moving flag. The wheels roll at road speed (`FUN_009b0430` gives the
    /// wheel bodies the chassis's motion; `ref/kerb100a` row 0: ω r = v on all four), the vehicle reset
    /// `FUN_009a9b10` clears the brake ramp (+0x1610), the handbrake countdown (`FUN_009cda30`, +0x2a20) and the
    /// drift (`FUN_009cd3f0`, +0x2ab0), and the drivetrain is placed by `FUN_009b3550` ([`Drivetrain::place_moving`]).
    pub fn place_moving(&mut self, speed: f32) {
        for w in &mut self.wheels {
            w.omega = speed / w.radius;
        }
        self.brake_ramp = 0.0;
        // `FUN_009b1e90` (any placement) zeroes the air time (+0x1630) and the downshift gate's clock (+0x1ed0):
        // no downshift for the first second (`ref/kerb100a` holds third at 4 470 rpm, below its 4 600 down rpm).
        self.air_time = 0.0;
        self.drivetrain.grounded_time = 0.0;
        self.handbrake_timer = -1.0;
        self.handbrake_on = false;
        self.drift = DriftState::default();
        // The driven wheels' mean spin as the driveline takes it (`FUN_009d5520`, no handbrake or burnout).
        let dp = &self.params.drivetrain;
        let mut omega = 0.0;
        for (axle, share) in [dp.front_share, 1.0 - dp.front_share].into_iter().enumerate() {
            let wheels: Vec<&WheelState> = self.wheels.iter().filter(|w| usize::from(!w.front) == axle).collect();
            omega += share * wheels.iter().map(|w| w.omega).sum::<f32>() / wheels.len().max(1) as f32;
        }
        self.drivetrain.place_moving(dp, omega, speed / MPH);
        self.gear = self.drivetrain.gear;
        self.rpm = self.drivetrain.rpm.min(dp.max_rpm);
        self.rpm_prev = self.rpm;
    }

    /// The front wheels' steering angle (rad, positive = left) the next tick's tyres use.
    pub fn steer_angle(&self) -> f32 {
        self.steer
    }

    /// Each wheel's ground contact from the last physics step (`None` while the wheel is in the air).
    pub fn wheel_contacts(&self) -> impl Iterator<Item = Option<WheelContact>> + '_ {
        self.wheels.iter().map(|w| w.contact)
    }

    /// The wheel the game's skid sounds follow (`FUN_009d1bf0`): wheel 0 (the first front wheel) when the
    /// engine drives only the front axle (`Power_To_Front` = 1), else wheel 2 (the first rear wheel). Its slip
    /// angle and tread-minus-ground speed, `None` while it is in the air.
    pub fn skid_wheel(&self) -> Option<mw_assets::vehicle_driver::WheelSlip> {
        let index = if self.params.drivetrain.front_share >= 1.0 { 0 } else { 2 };
        let w = self.wheels.get(index).or(self.wheels.last())?;
        let c = w.contact?;
        Some(mw_assets::vehicle_driver::WheelSlip { slip_angle: w.slip.1, slip_speed: c.tread_slip })
    }
}

fn spawn_car(
    mut commands: Commands,
    pending: Option<Res<PendingCar>>,
    selection: Res<crate::mods_menu::ModSelection>,
    spatial: SpatialQuery,
    mut camera: Query<&mut Transform, crate::MainCamera>,
    residency: Option<Res<crate::frontend::Residency>>,
    crank: Option<Res<crate::ignition::CrankAtSpawn>>,
) {
    let Some(pending) = pending else { return };
    // A placement waits until its target is resident within max(200 m, 3 s of travel) (`FUN_00647e00`), so
    // the ground and what stands around it have streamed in before the car is put down.
    let radius = crate::frontend::PLACE_RADIUS.max(pending.velocity.length() * 3.0);
    if residency.is_some_and(|r| !r.resident(pending.spawn, radius)) {
        return;
    }
    // Wait until the collision under the spawn point has streamed in.
    let origin = pending.spawn + Vec3::Y * pending.probe;
    let ground_only = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let Some(hit) = spatial.cast_ray(origin, Dir3::NEG_Y, 200.0, true, &ground_only) else {
        return;
    };
    let ground = origin - Vec3::Y * hit.distance;
    let start = ground + Vec3::Y * 0.15;

    // Performance mods (`mods_menu.rs`): the selection applied to the stock handling.
    let catalogue = pending.spec.mod_catalogue.clone();
    let (modded, nitrous_rules) = catalogue.modded(&selection);
    info!("mods: {}", catalogue.describe(&selection));
    commands.insert_resource(catalogue);
    let car = build_car(
        &mut commands,
        &CarBuild {
            name: "player car",
            spec: &pending.spec,
            parts: &pending.parts,
            handling: &modded,
            nitrous_rules: Some(nitrous_rules),
            start,
            heading: pending.heading,
            velocity: pending.velocity,
            damage: None,
        },
    );
    commands.entity(car).insert((Player, crate::nitrous::NitrousFeed::new(pending.spec.top_speed_mph)));
    // The engine's start-up states (`ignition.rs`), timed by the car's ignition sequence.
    let ignition = match &pending.spec.engine {
        Some(mw_assets::engine_sound::EngineVoice::Bank(b)) => b.ignition,
        _ => None,
    };
    commands.entity(car).insert(crate::ignition::EngineStart::new(crank.is_some_and(|c| c.0), ignition));
    info!(
        "nitrous: burn {}/s, start from {} units and {} mph, top speed {} mph (HighSpeed above {}%)",
        nitrous_rules.usage.burn,
        nitrous_rules.min_amount,
        nitrous_rules.min_mph,
        pending.spec.top_speed_mph,
        nitrous_rules.events[1].speed
    );
    if let Ok(mut cam) = camera.single_mut() {
        let behind = Quat::from_rotation_y(pending.heading) * Vec3::new(0.0, 2.5, -7.0);
        *cam = Transform::from_translation(start + behind).looking_at(start, Vec3::Y);
    }
    commands.remove_resource::<PendingCar>();
    info!("car spawned at {start:?}");
}

/// What [`build_car`] needs to put a car in the world.
pub struct CarBuild<'a> {
    pub name: &'a str,
    pub spec: &'a CarSpec,
    pub parts: &'a CarParts,
    /// The handling the physics runs (the player's with its mods applied).
    pub handling: &'a VehicleHandling,
    /// The nitrous rules (the car's `NitrousParameters` variant); `None` takes `NitrousRules::default()`.
    pub nitrous_rules: Option<mw_handling::nitrous::NitrousRules>,
    /// Where the car's origin (road level) starts, its yaw (0 faces +Z) and velocity.
    pub start: Vec3,
    pub heading: f32,
    pub velocity: Vec3,
    /// The car's damage model; `None` takes it from the pending player car (`damage.rs`).
    pub damage: Option<crate::damage::DamageSource>,
}

/// Spawn a drivable car: rigid body, colliders, body and wheel meshes, [`Car`] and [`CarControls`]. Every car
/// (player or AI) is built and simulated the same way. Returns the car's entity.
pub fn build_car(commands: &mut Commands, b: &CarBuild) -> Entity {
    let params = CarParams::new(b.handling);
    info!(
        "car params: {:.0} kg, inertia {:.0?}, drift {}, CoM {:?}, {} gears, final {:.2}, redline {:.0}, top {:.0} km/h, drag {:.2?}, peak μ lat {:.2}/{:.2} long {:.2}, front drive {:.2}",
        params.mass,
        params.inertia,
        params.drift.is_some(),
        params.centre_of_mass,
        params.drivetrain.gears,
        params.drivetrain.final_drive,
        params.redline_rpm,
        params.top_speed * 3.6,
        params.chassis.drag,
        params.tyres[0].pure_lateral.peak * params.tyres[0].lateral_scale,
        params.tyres[1].pure_lateral.peak * params.tyres[1].lateral_scale,
        params.tyres[1].traction_pure_long.peak * params.tyres[1].longitudinal_scale,
        params.drivetrain.front_share
    );
    let car = commands
        .spawn((
            Name::new(b.name.to_string()),
            Transform::from_translation(b.start).with_rotation(Quat::from_rotation_y(b.heading)),
            Visibility::default(),
            RigidBody::Dynamic,
            LinearVelocity(b.velocity),
            // Explicit mass properties; the collider box would otherwise
            // contribute a ~6 kg (density 1) inertia and the car flips at once.
            Mass(params.mass),
            AngularInertia::new(params.inertia),
            CenterOfMass(params.centre_of_mass),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
            // Drag and rolling resistance are modelled explicitly; avian's
            // linear damping would add ~1.7 kN at 240 km/h and cap top speed.
            // Model decision: the original damps a driving car's rotation only in a drift and after a collision
            // (its `DrivingVehicle` damping constraint is empty, read live); this 0.4 stands in for a yaw
            // resistance ours lacks in a slide (handling.md "Model decisions"). MW_ANGULAR_DAMPING=<v> for A/B.
            AngularDamping(std::env::var("MW_ANGULAR_DAMPING").ok().and_then(|v| v.parse().ok()).unwrap_or(0.4)),
            LinearDamping(0.0),
            // Render between physics ticks (64 Hz): without this the car and
            // the chase camera judder on faster displays.
            TransformInterpolation,
            TransformHermiteEasing,
        ))
        .id();
    commands.entity(car).insert(crate::paint_wear::ScratchScale(b.spec.scratch_scale));
    // Body collider: a box around the cabin, raised so the wheels do the work.
    let body_collider = commands
        .spawn((
            Collider::cuboid(1.8, 0.75, 4.2),
            Transform::from_xyz(0.0, 0.85, 0.0),
            crate::props::GameLayer::car(),
            ChildOf(car),
        ))
        .id();
    // Low bumper box that only meets props (cones would pass under the body box). Its bottom is at
    // 0.2 m so the car can roll over a fallen lamp post.
    let bumper_collider = commands
        .spawn((
            Collider::cuboid(1.7, 0.35, 4.1),
            Transform::from_xyz(0.0, 0.375, 0.0),
            crate::props::GameLayer::car_bumper(),
            ChildOf(car),
        ))
        .id();
    for m in &b.spec.body {
        for (mesh, mat) in b.parts.get(m).into_iter().flatten() {
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                crate::shadow_cull::ShadowClass::Car,
                ChildOf(car),
            ));
        }
    }

    // Static load per corner from the centre of mass between the axles.
    let h = &b.spec.handling;
    let z_front = b.spec.wheels.iter().filter(|w| w.front).map(|w| w.position.z).fold(f32::MIN, f32::max);
    let z_rear = b.spec.wheels.iter().filter(|w| !w.front).map(|w| w.position.z).fold(f32::MAX, f32::min);
    let front_load = ((params.centre_of_mass.z - z_rear) / (z_front - z_rear).max(0.5)).clamp(0.2, 0.8);
    let mut wheels = Vec::new();
    for w in &b.spec.wheels {
        // The game's spring holds the static corner load W at the data's sag (k = W / sag), its damper is
        // 2ζ√(k·W/g) one way and `0c5f89b3` times that the other (docs/decomp.md).
        let corner_mass = params.mass * if w.front { front_load } else { 1.0 - front_load } / 2.0;
        let spring = h.springs.get(if w.front { 0 } else { 1 }).copied();
        let (ratio, zeta, sag) = spring.map_or((1.5, 0.35, 0.08), |s| (s.damper_ratio, s.damping_ratio, s.sag));
        let sag = sag.clamp(0.02, 0.3);
        let k = corner_mass * 9.81 / sag;
        let c = 2.0 * zeta * (k * corner_mass).sqrt();
        // Body wheel data matches by side/axle; the graphics spec gives the position.
        let radius = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .map_or(w.position.y.max(0.25), |b| b.radius);
        let brake = if w.front { h.brake_torque_front } else { h.brake_torque_rear }.unwrap_or(1200.0);
        // The strut stop gives over the tyre's sidewall: max(wheel radius − the axle's rim radius, 0)
        // (`FUN_0095b4d0` → constraint +0x470; 0, a hard stop, without the data).
        let stop_slack = h.rim_radius[usize::from(!w.front)].map_or(0.0, |rim| (radius - rim).max(0.0));
        // A cylinder of the wheel body's mass, plus the transmission inertia on driven wheels.
        let wheel_mass = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .map_or(25.0, |b| b.mass)
            .max(5.0);
        let width = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .map_or(TYRE_WIDTH_DEFAULT, |b| 2.0 * b.half_width);
        // Bump travel above the rest position: up to the strut's top, `caac331c` + the suspension constraint's
        // `b867a203` (`wheel_layout`; 0.174 / 0.224 m on the Raptor, 0.056 / 0.067 m on the XKR, read live).
        let bump = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .and_then(|b| b.strut.map(|[top, _]| top - b.position[1]))
            .filter(|t| *t > 0.0)
            .unwrap_or(BUMP_TRAVEL);
        let droop = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .and_then(|b| b.strut.map(|[_, bottom]| b.position[1] - bottom))
            .filter(|t| *t > 0.0)
            .unwrap_or(sag);
        let share = if w.front { params.drivetrain.front_share } else { 1.0 - params.drivetrain.front_share };
        let inertia =
            0.5 * wheel_mass * radius * radius + if share > 0.0 { params.drivetrain.transmission_inertia } else { 0.0 };
        let pivot = commands.spawn((Transform::from_translation(w.position), Visibility::default(), ChildOf(car))).id();
        let mirror = if w.mirror { Vec3::new(-1.0, 1.0, 1.0) } else { Vec3::ONE };
        let spinner = commands
            .spawn((Transform::from_rotation(w.rotation).with_scale(mirror), Visibility::default(), ChildOf(pivot)))
            .id();
        let fixed = commands
            .spawn((Transform::from_rotation(w.rotation).with_scale(mirror), Visibility::default(), ChildOf(pivot)))
            .id();
        for &(m, spins) in &w.parts {
            for (mesh, mat) in b.parts.get(&m).into_iter().flatten() {
                let parent = if spins { spinner } else { fixed };
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    crate::shadow_cull::ShadowClass::Car,
                    ChildOf(parent),
                ));
            }
        }
        debug!(
            "wheel {:?} front={} radius {radius:.3} k {k:.0} c {c:.0}/{:.0} sag {sag:.3} stop slack {stop_slack:.3} corner {corner_mass:.0} kg",
            w.position,
            w.front,
            c * ratio
        );
        wheels.push(WheelState {
            centre: w.position,
            radius,
            front: w.front,
            spring: k,
            damper: c,
            rebound: c * ratio.max(1.0),
            travel: bump + sag,
            droop,
            strut_t: 0.5,
            sag,
            brake_torque: brake,
            omega: b.velocity.dot(Quat::from_rotation_y(b.heading) * Vec3::Z) / radius,
            inertia,
            corner_mass,
            slip: (0.0, 0.0),
            compression: sag,
            compression_rate: 0.0,
            overtravel: 0.0,
            stop_slack,
            mass: wheel_mass,
            world: None,
            vel: b.velocity.y,
            touching: false,
            stop_share: None,
            normal_impulse: 0.0,
            vel_substeps: 0.0,
            spin: 0.0,
            pivot,
            spinner,
            contact: None,
            surface: SurfaceGrip::default(),
            smoke: tyre_vfx::Ramps::default(),
            marks: tyre_vfx::Ramps::default(),
            width,
        });
    }
    // A car swapped in at speed starts in the gear that speed needs.
    let speed = b.velocity.dot(Quat::from_rotation_y(b.heading) * Vec3::Z);
    let radius = wheels.iter().map(|w| w.radius).sum::<f32>() / wheels.len().max(1) as f32;
    let wheel_rpm = speed.max(0.0) / radius.max(0.1) * 60.0 / std::f32::consts::TAU;
    let dp = &params.drivetrain;
    let gear = (1..=dp.gears as i32)
        .find(|&g| wheel_rpm * dp.ratio(g) * dp.final_drive < params.redline_rpm * 0.8)
        .unwrap_or(dp.gears.max(1) as i32);
    let drivetrain = Drivetrain::new(dp, gear);
    let mut e = commands.entity(car);
    if let Some(d) = b.damage.clone() {
        e.insert(d);
    }
    e.insert((
        CarControls::default(),
        Car {
            wheels,
            steer: 0.0,
            speed,
            yaw_rate: 0.0,
            lateral_accel: 0.0,
            body_slip: 0.0,
            drift: DriftState::default(),
            rear_slip: 0.0,
            air_time: 0.0,
            handbrake_timer: -1.0,
            handbrake_on: false,
            wall_time: f32::MAX,
            turn_assist: TurnAssist::default(),
            impact_yaw: ImpactYawDamping::default(),
            roll: 0.0,
            pitch: 0.0,
            body_contact: false,
            gear,
            rpm: params.idle_rpm,
            rpm_prev: params.idle_rpm,
            drivetrain,
            nitrous: Default::default(),
            nitrous_params: {
                let mut n = mw_handling::nitrous::NitrousParams::new(b.handling);
                n.rules = b.nitrous_rules.unwrap_or_default();
                n
            },
            brake_ramp: 0.0,
            debug_forces: [0.0; 3],
            tyre_yaw: [0.0; 2],
            spawn: b.start,
            params,
            body_collider,
            bumper_collider,
            prop_impact: 0.0,
            stop_impulse: 0.0,
        },
    ));
    // Lamp coronas (`coronas.rs`).
    if !b.spec.coronas.is_empty() {
        e.insert(crate::coronas::CarCoronas::new(b.spec.coronas.clone(), Vec3::ZERO));
    }
    // The light sources it casts on the world (`car_lights.rs`).
    if !b.spec.light_sources.is_empty() {
        e.insert(crate::car_lights::CarLights(b.spec.light_sources.clone()));
    }
    // Sirens and light bars (`police_fx.rs`).
    if let Some(l) = &b.spec.flash_lights {
        e.insert(crate::police_fx::FlashCar::new(l.clone()));
    }
    // Exhaust pops and the car's spot effects (`exhaust.rs`).
    let tuning = match &b.spec.engine {
        Some(mw_assets::engine_sound::EngineVoice::Bank(b)) => b.tuning,
        _ => Default::default(),
    };
    // Its collision mesh for the collision effects (`crash_vfx.rs`).
    e.insert(crate::crash_vfx::CrashVfxCar { bundle: b.spec.bundle.clone(), paint_index: b.spec.paint_index });
    e.insert(crate::exhaust::Exhaust::new(
        b.spec.spot_effects.clone(),
        mw_assets::engine_driver::PopParams::from_handling(b.handling),
        tuning,
    ));
    car
}

pub(crate) fn read_input(keys: Res<ButtonInput<KeyCode>>, pads: Query<&Gamepad>, mut input: ResMut<CarInput>) {
    // The original's default bindings and mapping (input.rs; FUN_00483470 / FUN_00482250 / FUN_00ac6850).
    // Our steer is positive to the left, the game's (+0x1bf0) to the right: steer = left − right.
    use crate::input::defaults as act;
    let pads: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pads };
    let mut throttle = dev.value(act::ACCELERATE);
    let mut brake = dev.value(act::BRAKE);
    let mut steer = dev.value(act::STEER_LEFT) - dev.value(act::STEER_RIGHT);
    let handbrake = dev.held(act::HANDBRAKE);
    if input.scripted.is_none() {
        input.nitrous = dev.held(act::NITROUS) || input.auto_nitrous;
    }
    input.reset = keys.just_pressed(KeyCode::KeyR) || input.reset;
    if input.scripted.is_some() {
        // A `--test` script drives the car; `car_physics` reads it per tick.
        return;
    }
    if let Some(t) = input.auto_throttle {
        throttle = t;
        brake = 0.0;
        steer = 0.0;
    }
    input.throttle = throttle;
    input.brake = brake;
    input.steer = steer.clamp(-1.0, 1.0);
    input.handbrake = handbrake;
}

/// The car the player drives. Its [`CarControls`] come from [`CarInput`]; the cameras, HUD, engine sound,
/// replay, crash and test systems follow this car. Other cars (`ai.rs`) run the same physics.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Player;

/// The car's [`CarControls`] are written by another system (an AI driver, `ai.rs`) even on the player's car.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct ExternalControls;

/// One car's controls for its next physics tick: the game's per-vehicle control block (steering +0x1bf0,
/// throttle +0x1bf4, brake +0x1c00, nitrous +0x1c20, handbrake +0x1c40; decomp.md "Update loop"), written for
/// every race car the same way. The player's are copied from [`CarInput`] each tick, an AI driver writes its own.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct CarControls {
    pub throttle: f32,
    pub brake: f32,
    /// Positive = left (the game's +0x1bf0 is positive to the right).
    pub steer: f32,
    pub handbrake: bool,
    pub nitrous: bool,
    /// Put the car back on its wheels (R for the player); cleared after one tick.
    pub reset: bool,
    /// No drive while crashed (`crash.rs`).
    pub crashed: bool,
}

impl CarControls {
    fn from_input(input: &mut CarInput) -> Self {
        Self {
            throttle: input.throttle,
            brake: input.brake,
            steer: input.steer,
            handbrake: input.handbrake,
            nitrous: input.nitrous,
            reset: std::mem::take(&mut input.reset),
            crashed: input.crashed,
        }
    }
}

impl CarInput {
    /// Apply a scripted test input (set in the fixed schedule, so the physics
    /// tick that follows sees it).
    fn apply_script(&mut self) {
        if let Some([t, b, s, h]) = self.scripted {
            self.throttle = t;
            self.brake = b;
            self.steer = s.clamp(-1.0, 1.0);
            self.handbrake = h > 0.5;
        }
    }
}

/// Install the bundled SDL GameControllerDB (newer than the copy inside
/// gilrs) through `SDL_GAMECONTROLLERCONFIG`, keeping any user mappings.
/// Must run before the app starts any threads.
pub fn install_gamepad_mappings() {
    const DB: &str = include_str!("../assets/gamecontrollerdb.txt");
    let user = std::env::var("SDL_GAMECONTROLLERCONFIG").unwrap_or_default();
    // User mappings first so they take precedence over the bundled ones.
    let combined = format!("{user}\n{DB}");
    // SAFETY: called at the very start of `main`, before any other thread exists.
    unsafe { std::env::set_var("SDL_GAMECONTROLLERCONFIG", combined) };
}

/// `--gamepad-debug`: log every raw gamepad event.
pub fn log_gamepad_events(mut events: MessageReader<RawGamepadEvent>) {
    for e in events.read() {
        match e {
            RawGamepadEvent::Axis(a) if a.value.abs() > 0.05 => {
                info!("gamepad {} axis {:?} = {:.2}", a.gamepad, a.axis, a.value)
            }
            RawGamepadEvent::Button(b) => info!("gamepad {} button {:?} = {:.2}", b.gamepad, b.button, b.value),
            RawGamepadEvent::Connection(c) => info!("gamepad connection {:?}", c),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub(crate) fn car_physics(
    time: Res<Time>,
    mut input: ResMut<CarInput>,
    spatial: SpatialQuery,
    collisions: Collisions,
    layers: Query<&CollisionLayers>,
    mut cars: Query<(
        Entity,
        &mut Car,
        Forces,
        &mut Transform,
        &mut CarControls,
        Has<Player>,
        Has<ExternalControls>,
        Option<&crate::nitrous::NitrousFeed>,
    )>,
    surfaces: Res<crate::props::WorldSurfaces>,
    world_tags: Query<(&Collider, &crate::props::SurfaceTags)>,
    world_shapes: WorldShapes,
) {
    let dt = time.delta_secs();
    input.apply_script();
    if input.crashed {
        input.throttle = 0.0;
        input.brake = 0.0;
        input.steer = 0.0;
        input.handbrake = false;
    }
    for (entity, mut car, mut forces, mut transform, mut controls, player, external, feed) in &mut cars {
        // Every car runs the same tick from its own control block (the game writes each of its 16 race-car slots'
        // controls with `FUN_009aacc0` from `FUN_0091c5c0`); the player's comes from the input devices.
        if player && !external {
            *controls = CarControls::from_input(&mut input);
        }
        let mut ctl = *controls;
        controls.reset = false;
        if ctl.reset {
            let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
            transform.rotation = Quat::from_rotation_y(yaw);
            transform.translation += Vec3::Y * 1.5;
            *forces.linear_velocity_mut() = Vec3::ZERO;
            *forces.angular_velocity_mut() = Vec3::ZERO;
            continue;
        }
        let car = &mut *car;
        let p = &car.params;
        let rot = transform.rotation;
        let pos = transform.translation;
        let up = rot * Vec3::Y;
        let forward = rot * Vec3::Z;
        // The contacts of the last physics step: the game's collision analysis (`FUN_00918600`) runs before the
        // vehicle's tick, whose first stages damp the yaw after a collision (`FUN_008f7950`).
        let contacts = car_contacts(&collisions, &layers, car);
        if contacts.wall.is_some() && !car.body_contact {
            debug!("impact yaw damping: new world contact ({:.1} m/s)", contacts.world_impact);
        }
        car.impact_yaw.hit(
            contacts.wall.is_some() && !car.body_contact,
            contacts.car_impact > 0.0,
            contacts.object_impact > 0.0,
        );
        let w = car.impact_yaw.step(forces.angular_velocity(), up, dt);
        *forces.angular_velocity_mut() = w;
        let velocity = forces.linear_velocity();
        let speed = velocity.dot(forward);
        car.speed = speed;
        car.yaw_rate = forces.angular_velocity().dot(up);
        car.lateral_accel = car.yaw_rate * speed;
        car.body_slip = velocity.dot(rot * Vec3::X).atan2(speed.abs().max(1.0));
        // The game's speed-banded lookups (steering, handbrake, drift) use the full speed signed by the
        // direction of travel, not the forward component (which reads low in a slide).
        let band_speed = velocity.length().copysign(speed);

        // --- Nitrous tank (`mw_handling::nitrous`): the game logic's input, refill events and burn, then the
        // control block (`FUN_006c1a80`): while the tank burns the throttle is 1 once the control amount is above
        // 0, or below 15 mph.
        let held = ctl.nitrous && !ctl.crashed;
        let n_params = car.nitrous_params;
        let world = match feed {
            Some(f) => mw_handling::nitrous::World {
                events: true,
                top_mph: f.top_mph,
                lane: f.lane,
                shortcut: f.shortcut,
                slipstream: f.slipstream,
                touching_competitor: f.touching_competitor,
                near_cars: &f.near_cars,
                near_miss_cancel: f.near_miss_cancel,
                near_miss_window: mw_handling::nitrous::NEAR_MISS_WINDOW,
                ..Default::default()
            },
            None => mw_handling::nitrous::World::default(),
        };
        let world = mw_handling::nitrous::World {
            mph: velocity.length() / MPH,
            forward_mph: speed / MPH,
            drifting: car.drift.active(),
            donut_weight: car.drift.donut_weight,
            air_time: car.air_time,
            ..world
        };
        let nitrous_amount = car.nitrous.update(&n_params.rules, &world, held, dt);
        if car.nitrous.mode != 0
            && (nitrous_amount > 0.0 || speed / MPH < mw_handling::nitrous::FULL_THROTTLE_BELOW_MPH)
        {
            ctl.throttle = 1.0;
        }

        // --- Pedals, reverse ---------------------------------------------------
        let (mut throttle, mut brake) = (ctl.throttle, ctl.brake);
        if car.drivetrain.gear < 0 {
            // In reverse the pedals swap roles.
            std::mem::swap(&mut throttle, &mut brake);
            if ctl.throttle > 0.1 && speed > -1.0 {
                car.drivetrain.engage(1);
            }
        } else if ctl.brake > 0.1 && ctl.throttle < 0.05 && speed < 0.5 {
            car.drivetrain.engage(-1);
        }
        let mph = speed / MPH;
        // Standstill hold (`FUN_009d6170` writes vehicle +0x1aa8, `FUN_009b1180` reads it): below a speed
        // threshold (`STANDSTILL_HOLD_MPH`: signed forward speed, so only when rolling backwards) the input is the brake pedal (1 with the handbrake up), raised to the throttle in a forward
        // gear. The body's velocity then changes by −sign(speed) × `Brakes.99407bbc` × input × dt along the car's
        // forward axis, every tick, with no clamp at zero.
        let hold_in = if mph < STANDSTILL_HOLD_MPH {
            let parked = if ctl.handbrake { 1.0 } else { brake };
            if car.drivetrain.gear > 0 { parked.max(throttle) } else { parked }
        } else {
            0.0
        };
        if hold_in > 0.0 && speed != 0.0 {
            *forces.linear_velocity_mut() -= forward * (speed.signum() * p.hold_decel * hold_in.min(1.0) * dt);
        }
        // Brake pressure builds at 10 × pedal per second; the torque goes with the pedal squared.
        car.brake_ramp = if brake > 0.1 { (car.brake_ramp + 10.0 * brake * dt).min(1.0) } else { 0.0 };
        // A burnout (both pedals at low speed, `FUN_009ab230`) releases one axle's brakes: the front on a
        // front-driven car (`Power_To_Front` = 1), else the rear (`FUN_009b1180`).
        let burnout = drift::burnout(band_speed / MPH, throttle, brake);

        // --- Handbrake (`FUN_009d27b0`) ---------------------------------------------------------------------
        // A countdown reset to `Minimum_handbrake_time` on every tick the handbrake is pulled; it is on while the
        // countdown has not run out, so it stays on that long after it is let go. In the game's tick the brakes
        // and the driveline run before this update and see the previous tick's state; the drift sees this tick's
        // and the previous one.
        let handbrake = car.handbrake_on;
        car.handbrake_timer = if ctl.handbrake { p.handbrake.min_time } else { car.handbrake_timer - dt };
        car.handbrake_on = car.handbrake_timer >= 0.0;
        let handbrake_strength = p.handbrake.at(band_speed);

        // --- Engine, clutch, gearbox (drivetrain.rs) -------------------------------
        let dp = &p.drivetrain;
        // The driveline's axle split (`FUN_009d5520`, `FUN_009d0380`, `FUN_009d7ac0`): with the handbrake on, the
        // front axle takes all of the drive (share 1, rear 0) on every car, rear-driven ones included; the
        // burnout measure then blends each share towards the burnout axle's (front if `Power_To_Front` = 1, else
        // rear).
        let burnout_front = if dp.front_share >= 1.0 { 1.0 } else { 0.0 };
        let (front, rear) = if handbrake { (1.0, 0.0) } else { (dp.front_share, 1.0 - dp.front_share) };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let axle_share = [lerp(front, burnout_front, burnout), lerp(rear, 1.0 - burnout_front, burnout)];
        let (mut omega, mut inv_inertia) = (0.0, 0.0);
        for (axle, share) in axle_share.iter().enumerate() {
            let wheels: Vec<&WheelState> = car.wheels.iter().filter(|w| usize::from(!w.front) == axle).collect();
            let n = wheels.len().max(1) as f32;
            omega += share * wheels.iter().map(|w| w.omega).sum::<f32>() / n;
            inv_inertia += share.powi(2) * wheels.iter().map(|w| 1.0 / w.inertia).sum::<f32>() / n;
        }
        let coupled = axle_share.iter().any(|&s| s > 0.0);
        // Update step 5 (FUN_009d2aa0, mode 0) before the engine: the tank and the torque multiplier.
        let amount = nitrous_amount;
        car.drivetrain.nitrous = amount > 0.0;
        car.drivetrain.nitrous_scale = n_params.multiplier(amount);
        let drive_torque = car.drivetrain.step(dp, throttle, brake, omega, mph, inv_inertia, coupled, dt);
        car.gear = car.drivetrain.gear;
        car.rpm_prev = car.rpm;
        car.rpm = car.drivetrain.rpm.min(dp.max_rpm);
        let n_axle = [
            car.wheels.iter().filter(|w| w.front).count().max(1) as f32,
            car.wheels.iter().filter(|w| !w.front).count().max(1) as f32,
        ];

        // --- Steering (`FUN_009d73f0` → `FUN_009d31f0`) -----------------------------------------------------
        // The game steers after the tyres, so this tick's tyres use the angle set on the previous tick. In a
        // drift the input is remapped (no input holds some lock into the drift, counter-steer is limited) and
        // the wheels may not stand further across than that limit; while counter-steering the rate blends
        // towards `Counter_steer_rate_in_drift` (drift.rs).
        let steer = car.steer;
        let mut current = car.steer;
        let mut steer_input = ctl.steer;
        let mut drift_rate = None;
        if let Some(d) = &p.drift {
            if let Some((mapped, lo)) = car.drift.steering(d, ctl.steer, car.rear_slip) {
                let dir = car.drift.direction;
                let max = p.steering.max_angle.to_radians();
                current = dir * (current * dir).clamp(lo * max, max);
                steer_input = mapped;
            }
            drift_rate = Some((d.counter_steer_rate, car.drift.counter_rate_blend(d)));
        }
        let pedal = throttle - brake;
        car.steer = steer_towards(
            &p.steering,
            current,
            ctl.steer,
            steer_input,
            band_speed,
            car.gear < 0,
            pedal,
            drift_rate,
            steer_slip_fraction(velocity.dot(rot * Vec3::X), speed),
            car.drift.active(),
            dt,
        );

        // Wheels only stand on the world (props are pushed by the body and bumper boxes).
        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider])
            .with_mask(crate::props::GameLayer::World);

        // --- Wheel spin: drive torque, and the brake budget (applied before the tyre below) ---------------
        let mut brake_budget = Vec::with_capacity(car.wheels.len());
        for wheel in &mut car.wheels {
            let axle = usize::from(!wheel.front);
            wheel.omega += drive_torque * axle_share[axle] / n_axle[axle] * dt / wheel.inertia;
            let mut t_brake = wheel.brake_torque * brake * brake * car.brake_ramp;
            if axle == if dp.front_share >= 1.0 { 0 } else { 1 } {
                t_brake *= 1.0 - burnout;
            }
            // The handbrake is a rear brake torque, combined with the service brake by max (`FUN_009cdfd0`). It
            // does not lock the wheels outright: the game's lock mask is set by other states, not the handbrake.
            if handbrake && !wheel.front {
                t_brake = t_brake.max(handbrake_strength);
            }
            brake_budget.push(t_brake * dt);
        }
        limited_slip(&mut car.wheels, &dp.diff);

        // The downforce the tyres feel, per axle (chassis.rs): it adds to the static wheel loads that set the
        // grip caps; the suspension force does not (docs/decomp.md "Tyres").
        let z_front = car.wheels.iter().filter(|w| w.front).map(|w| w.centre.z).fold(f32::MIN, f32::max);
        let z_rear = car.wheels.iter().filter(|w| !w.front).map(|w| w.centre.z).fold(f32::MAX, f32::min);
        let axle_downforce = p.chassis.axle_downforce(rot, velocity, car.brake_ramp, z_front, z_rear);
        let mph_abs = velocity.length() / MPH;

        let mut tyre_long_total = 0.0;
        let mut resist_total = 0.0;
        let mut tyre_yaw = [0.0; 2];
        let com = pos + rot * p.centre_of_mass;
        // The chassis's effective mass at a contact along a direction (`FUN_009ca610` / `FUN_009e8230`): the
        // game's tyre constraint acts on the chassis body, one wheel after the other.
        let inv_inertia =
            Mat3::from_quat(rot) * Mat3::from_diagonal(p.inertia.recip()) * Mat3::from_quat(rot.inverse());
        let effective_mass = |offset: Vec3, dir: Vec3| {
            let arm = offset.cross(dir);
            1.0 / (1.0 / p.mass + arm.dot(inv_inertia * arm)).max(1e-6)
        };
        let mut grounded = false;
        let mut wheels_down = 0;
        let mut rear_slip = (0.0, 0);
        let mut ground_normal = Vec3::ZERO;
        // --- The wheels' point masses and the ground under them (decomp.md "Suspension, wheels, aerodynamics") -----
        // Each wheel is a point mass on its strut (the game's wheel record: centre, velocity, the wheel body's data
        // mass) that the island step integrates like a body; ours moves along the car's up axis only (handling.md
        // "Model decisions"). Where the last tick's integration left it gives the compression, and the wheel
        // body's cylinder coming down the strut finds the ground under it: this tick's contact record (`probe_ground`).
        let ray_dir = Dir3::new(-up).unwrap_or(Dir3::NEG_Y);
        let mut grounds: Vec<Option<Ground>> = Vec::with_capacity(car.wheels.len());
        for wheel in car.wheels.iter_mut() {
            let rest = pos + rot * wheel.centre;
            // The strut's end sits `travel - sag` above the rest position, so at rest (compression = sag) the wheel
            // is at its spec position (in its arch).
            let mount = rest + up * (wheel.travel - wheel.sag);
            let y = wheel.world.map_or(f32::NAN, |w| (w - rest).dot(up));
            // A wheel far from its strut (the first tick, a teleport or reset) starts again at rest, moving with the
            // body.
            if !y.is_finite() || y.abs() > WHEEL_RESYNC {
                wheel.vel = forces.velocity_at_point(rest).dot(up);
                wheel.touching = false;
                wheel.stop_share = None;
                wheel.compression = wheel.sag;
            } else {
                wheel.compression = y + wheel.sag;
            }
            wheel.overtravel = 0.0;
            // The wheel's axle: the car's x axis, turned by the steering on the front wheels (the game's wheel
            // shape takes the wheel's world matrix, `FUN_00902930`).
            let axle = if wheel.front { Quat::from_axis_angle(up, steer) } else { Quat::IDENTITY } * (rot * Vec3::X);
            let Some(hit) = probe_ground(&spatial, &filter, &world_shapes, wheel, mount, up, axle, ray_dir) else {
                grounds.push(None);
                continue;
            };
            let (mut point, mut normal) = (hit.point, hit.normal);
            // The surface under the contact and how this car's tyres treat it (`FUN_009d8bd0`).
            let surface = WheelSurface::at(
                &surfaces,
                &world_tags,
                hit.entity,
                (point + normal * 0.05, Dir3::new(-normal).unwrap_or(ray_dir), 0.1),
                p,
                wheel.front,
            );
            // The surface's bumps (`FUN_009d77b0` → `FUN_009d4650`): the contact point moves along the ground normal
            // by Perlin noise of its position and the normal tilts with it, and both are written into the wheel's
            // contact and its constraint's contact record (+0x10 point, +0x20 normal; `FUN_009aaf30`), so the
            // suspension and the tyre see the bumped ground. The game does this only under a flag that reads as
            // "the car has a human player" (`FUN_00904260`: the car's player record +0x9c is 0; inferred), so AI and
            // traffic cars skip it.
            if player
                && rough_on()
                && let Some((frequency, amplitude)) = surface.roughness
            {
                let (moved, tilted, _) = surface::roughness(point.to_array(), normal.to_array(), frequency, amplitude);
                (point, normal) = (Vec3::from(moved), Vec3::from(tilted).normalize_or(normal));
            }
            // The ground inside a tyre centred at the strut's end (telemetry; the strut stop's own measure).
            wheel.overtravel = (point - (mount + hit.offset)).dot(normal).max(0.0);
            grounds.push(Some(Ground { normal, point, offset: hit.offset, surface }));
        }

        // --- The island's sub-steps (`FUN_0093f200`: world +0x464 = 2, at dt / 2) ------------------------------------
        // Each runs the wheel contacts' impulse pass (`FUN_009c2670`: a touching wheel's approach to the ground is
        // stopped; the strut stop that the last contact pass set takes its share of the chassis's closing speed at
        // the strut's end), then the force elements (`FUN_00932f90`): every damper, then every spring, then the
        // anti-roll bars; then gravity (`FUN_009282d0`: the wheels' tiny one here, the chassis's is avian's).
        let h = dt / SOLVER_SUBSTEPS as f32;
        let mut stop_total = 0.0;
        let mut ch =
            Chassis { v: forces.linear_velocity(), w: forces.angular_velocity(), com, mass: p.mass, inv_inertia };
        let (joint_pos, joint_vel) = strut_joint_passes();
        for wheel in car.wheels.iter_mut() {
            wheel.normal_impulse = 0.0;
            // The strut joint's position parameter (+0x30) from where the wheel stands now.
            let (y, bump) = (wheel.compression - wheel.sag, wheel.travel - wheel.sag);
            wheel.strut_t = (y + wheel.droop) / (bump + wheel.droop).max(1e-4);
        }
        for _ in 0..SOLVER_SUBSTEPS {
            // The strut joints' velocity pass (`FUN_00928fb0` → `FUN_009c62c0` → `FUN_009c3ae0`), before the wheel
            // contacts' impulse pass. Ours moves the wheel along the car's up axis only (across, it moves with the
            // chassis), so only the along-strut limit remains.
            if joint_vel {
                for wheel in car.wheels.iter_mut() {
                    let centre = wheel.world_centre(pos, rot);
                    let s = wheel.vel - ch.velocity_at(centre).dot(up);
                    let err = suspension::strut_velocity_error(s, wheel.strut_t);
                    strut_joint_impulse(wheel, &mut ch, centre, up, err);
                }
            }
            for (wheel, ground) in car.wheels.iter_mut().zip(&grounds) {
                let Some(g) = ground else { continue };
                if wheel.touching {
                    let centre = wheel.world_centre(pos, rot);
                    let vn = ch.wheel_velocity(centre, up, wheel.vel).dot(g.normal);
                    let dvn = suspension::contact_impulse(vn);
                    wheel.normal_impulse += wheel_ground_impulse(wheel, &mut ch, centre, g.normal, up, dvn);
                }
                if let Some(share) = wheel.stop_share {
                    let mount = wheel.strut_end(pos, rot);
                    let vn = ch.velocity_at(mount).dot(g.normal);
                    let impulse = share * ch.effective_mass(mount, g.normal) * (-vn).max(0.0);
                    ch.push(mount, g.normal * impulse);
                    stop_total += impulse;
                }
            }
            // The dampers (`FUN_0092f160`) act on the chassis alone (the wheel's share is masked by its record's
            // +0x0c, which is 1), the springs (`FUN_0092f510`) on both; both along the strut, from the anchor on the
            // chassis to the wheel centre (here the car's −up axis), the chassis taking −λ n at the anchor.
            for wheel in car.wheels.iter() {
                let anchor = wheel.strut_end(pos, rot);
                let v_rel = ch.velocity_at(anchor).dot(up) - wheel.vel;
                let coef = if v_rel > 0.0 { wheel.rebound } else { wheel.damper };
                let lambda = suspension::damper_impulse(v_rel, coef, ch.inv_mass_at(anchor, up), h);
                ch.push(anchor, up * lambda);
            }
            for wheel in car.wheels.iter_mut() {
                let anchor = wheel.strut_end(pos, rot);
                let v_rel = ch.velocity_at(anchor).dot(up) - wheel.vel;
                let inv_mass = ch.inv_mass_at(anchor, up) + 1.0 / wheel.mass;
                let lambda = suspension::spring_impulse(v_rel, -wheel.compression, wheel.spring, inv_mass, h, 1.0 / dt);
                ch.push(anchor, up * lambda);
                wheel.vel -= lambda / wheel.mass;
            }
            // The anti-roll bars (`FUN_0092ee70`), after the springs in the same element pass.
            anti_roll(&car.wheels, p, (pos, rot), &mut ch, dt, h);
            for wheel in car.wheels.iter_mut() {
                wheel.vel += suspension::wheel_gravity(h) * up.y;
            }
        }
        *forces.linear_velocity_mut() = ch.v;
        *forces.angular_velocity_mut() = ch.w;

        for ((wheel, mut brake_left), ground) in car.wheels.iter_mut().zip(brake_budget).zip(&grounds) {
            // The tyre acts where the wheel's contact carries a normal impulse (`FUN_009c75b0` skips it otherwise).
            let Some(g) = ground.as_ref().filter(|_| wheel.touching && wheel.normal_impulse > 0.0) else {
                wheel.contact = None;
                // In the air the ramps head for 0 (model decision: what the game's wheel inputs read off the
                // ground was not traced).
                if let Some(v) = &p.tyre_vfx {
                    let a = &v[usize::from(!wheel.front)];
                    wheel.smoke.step(&a.smoke, &tyre_vfx::WheelInput::default(), dt);
                    wheel.marks.step(&a.skid_marks, &tyre_vfx::WheelInput::default(), dt);
                }
                let step = (brake_left / wheel.inertia).min(wheel.omega.abs());
                wheel.omega -= step * wheel.omega.signum();
                wheel.spin += wheel.omega * dt;
                continue;
            };
            let surface = &g.surface;
            grounded = true;
            wheels_down += 1;
            ground_normal += g.normal;
            let contact = g.point;
            let normal = g.normal;

            // Tyre frame on the ground plane.
            let yaw = if wheel.front { Quat::from_axis_angle(up, steer) } else { Quat::IDENTITY };
            let axis = yaw * (rot * Vec3::X);
            let fwd = (yaw * forward).reject_from(normal).normalize_or_zero();
            let side = normal.cross(fwd).normalize_or_zero();
            let axle = usize::from(!wheel.front);
            let tyre = &p.tyres[axle];
            let on_ground = if normal.y > TYRE_NORMAL_MIN_UP { 1.0 } else { 0.0 };
            // The surface grip this tick: the surface's own, or lower while the grip of one just left fades
            // (`FUN_009d4460`).
            let grip = wheel.surface.step(surface.uid, surface.grip, surface.target, dt);
            // Grip load: the static corner load plus half this axle's downforce, not the spring force
            // (`FUN_00905910`; the XKR reference run shows it constant through launch and braking).
            let static_load = wheel.corner_mass * 9.81;
            let n_grip = tyre::grip_load(static_load, axle_downforce[axle]);
            // Rolling resistance and slide drag along the wheel's heading, against its motion (`FUN_009d4a10`,
            // in the vehicle update before the solver, so the tyre pass below sees it): the surface's
            // coefficient × the grip load, plus |mph| × the surface's slide-drag scale × this car's drag for
            // that surface, which ramps in with the slip angle (last tick's).
            let v = forces.velocity_at_point(contact);
            // Ours: at most what brings the contact to rest along the heading, so the force cannot flip sign
            // every tick at a standstill (the game applies it in full against the sign of the motion).
            let v_heading = v.dot(fwd);
            let f_resist = -v_heading.signum() * surface.resistance(n_grip, mph_abs, wheel.slip.1.abs()) * on_ground;
            let rest = v_heading.abs() * effective_mass(contact - com, fwd) / dt;
            let f_resist = f_resist.clamp(-rest, rest);
            forces.apply_linear_impulse(fwd * f_resist * dt);
            let arm = (contact - com).cross(fwd * f_resist * dt);
            forces.apply_angular_impulse(arm - forward * arm.dot(forward));
            let v = forces.velocity_at_point(contact);
            let (v_long, v_side) = (v.dot(fwd), v.dot(side));

            // The game's tyre: slip angle and slip ratio set the grip caps (tyre.rs), and within them the
            // contact takes whatever force stops it sliding.
            let slip_deg = v_side.atan2(v_long.abs().max(1e-3)).to_degrees();
            if !wheel.front {
                rear_slip = (rear_slip.0 - slip_deg, rear_slip.1 + 1);
            }
            // The brake (`FUN_009bbff0`, an angular impulse budget that can stop the wheel but never reverse it)
            // acts on the wheel's spin first, and the caps take κ from the spin as it stands after it (and after
            // the clutch): `FUN_009c75b0` runs the brake step before the tyre pass `FUN_009c6f00`, whose cap
            // function `FUN_009c5da0` computes κ (`FUN_009ba3e0`) from the stepped wheel (docs/decomp.md "Tyres").
            // Taking κ before the brake step (as this did) put a rolling wheel's braking cap at μ(κ ≈ 0) ≈ 0, so
            // the wheel stayed slowed: in the replay of `ref/live1` 97.6–101.5 s (MP4-12C, full brake at 120 mph)
            // our front wheels slipped at κ −0.03…−0.10 before the drift and −0.20…−0.26 in it, where the
            // original's stay at ≈ +0.01 and −0.05…−0.07; taken after the step they read ≈ 0 and −0.03…−0.07.
            let r = wheel.radius;
            let step = (brake_left / wheel.inertia).min(wheel.omega.abs());
            wheel.omega -= step * wheel.omega.signum();
            brake_left -= step * wheel.inertia;
            let kappa = tyre::slip_ratio(wheel.omega * r, v_long);
            // Camber: the wheel's spin axis against the ground-plane side axis (0 when the frame degenerates).
            let camber = if side.length_squared() > 0.5 && axis.length_squared() > 0.5 {
                let a = axis.angle_between(side).to_degrees();
                a.min(180.0 - a)
            } else {
                0.0
            };
            let (mu_long, mu_lat) = tyre::coefficients(tyre, kappa, slip_deg, camber, static_load / n_grip);
            wheel.slip = (kappa, slip_deg);
            let driven = axle_share[axle] > 0.0;
            // The caps (`FUN_009c5da0`): μ × N × the axle's data scale × the surface grip (lateral and
            // longitudinal blends of `Surface_Grip`) × the per-wheel gameplay grip multipliers (1 outside the
            // game's timed effects, so left out) × camber (inside the coefficients); the driven wheels'
            // longitudinal cap shrinks with the burnout. No caps at all when the ground is too steep.
            let long_cap = mu_long
                * n_grip
                * tyre.longitudinal_scale
                * grip[1]
                * if driven { 1.0 - burnout } else { 1.0 }
                * on_ground;
            let lat_cap = mu_lat * n_grip * tyre.lateral_scale * grip[0] * on_ground;
            let (long_cap, lat_cap) = (finite_or_zero(long_cap), finite_or_zero(lat_cap));
            // Longitudinal, in the game's order (`FUN_009c75b0`): after the brake step (above), one pass of the
            // tyre constraint pulls the tread towards the road speed (the wheel's spin inertia against the
            // chassis's effective mass at the contact) within the cap.
            // The constraint (`FUN_009bc0c0`) works in two parts along the slip direction. First the brake
            // torque the spin step left over holds the tread like a locked wheel: an impulse against the
            // chassis alone (its effective mass at the contact), at most that torque / r (unbounded when the
            // wheel is locked) and the cap; the wheel's spin is not touched. Then the slip that remains is
            // taken out with the chassis and the wheel's spin inertia together (1/m + r²/I), within what is
            // left of the cap; only this part turns the wheel.
            let m_chassis = effective_mass(contact - com, fwd);
            let slip = wheel.omega * r - v_long;
            let cap = long_cap * dt;
            let hold = (brake_left / r).min(slip.abs() * m_chassis).min(cap);
            let k_eff = r * r / wheel.inertia + 1.0 / m_chassis;
            let coupled = ((slip.abs() - hold / m_chassis) / k_eff).min(cap - hold).max(0.0);
            let impulse = slip.signum() * (hold + coupled);
            wheel.omega -= slip.signum() * coupled * r / wheel.inertia;
            let f_long = impulse / dt;
            // Telemetry keeps the slip ratio the tyre pass leaves (tread against the road after the impulse),
            // which is what the original's wheel field shows: in `ref/xkr_launch` its rear tyres read κ ≈ 0
            // while accelerating at 0.6 g in 2nd, as ours do after the pass (inferred from that comparison).
            wheel.slip.0 = tyre::slip_ratio(wheel.omega * r, v_long + impulse / effective_mass(contact - com, fwd));
            // Lateral: the impulse that stops the sideways slide, within the cap.
            let f_side = (-v_side * effective_mass(contact - com, side) / dt).clamp(-lat_cap, lat_cap);
            let force = fwd * f_long + side * f_side;
            // Applied as an impulse, so the next wheel sees the velocity this one left (`FUN_009c2910`). The
            // game's tyre forces do not roll the body: the angular part about the car's long axis is dropped.
            let torque = (contact - com).cross(force);
            forces.apply_linear_impulse(force * dt);
            forces.apply_angular_impulse((torque - forward * torque.dot(forward)) * dt);
            tyre_long_total += f_long;
            resist_total += f_resist;
            tyre_yaw[axle] += torque.dot(up);
            if tyre_debug() && (time.elapsed_secs() * 2.0).fract() < dt * 2.0 {
                info!(
                    "tyre front={} α {slip_deg:.1}° κ {kappa:.2} μ {mu_long:.2}/{mu_lat:.2} N {n_grip:.0} long {f_long:.0} side {f_side:.0} resist {f_resist:.0} surface {} grip {:.3?} (own {:.3?}) steer {:.1}°",
                    wheel.front,
                    surface.uid,
                    grip,
                    surface.grip,
                    steer.to_degrees()
                );
            }

            wheel.spin += wheel.omega * dt;
            // The smoke and tyre mark ramps (`FUN_009cef90`, each tick after the dirt build-up): |tread − ground|,
            // |α| and the car's speed; κ splits spinning from locked wheels.
            if let Some(v) = &p.tyre_vfx {
                let input = tyre_vfx::WheelInput {
                    tread_slip: wheel.omega * r - v_long,
                    slip_deg: wheel.slip.1,
                    speed: mph_abs * MPH,
                    kappa: wheel.slip.0,
                };
                wheel.smoke.step(&v[axle].smoke, &input, dt);
                wheel.marks.step(&v[axle].skid_marks, &input, dt);
            }
            wheel.contact = Some(WheelContact {
                point: contact,
                normal,
                slip: side * v_side + fwd * (v_long - wheel.omega * r),
                velocity: v,
                tread_slip: wheel.omega * r - v_long,
                effects: surface.effects,
                surface: surface.index,
                smoke: wheel.smoke,
                marks: wheel.marks,
                width: wheel.width,
                across: side,
            });
        }
        // --- Low-speed turn-around assist (`FUN_008f8360`) -------------------------------------------------
        // The body's contact with the world (not props) on the last physics step, as the wall normal pointing
        // at the car; the game counts the time since the last one (vehicle +0x3010).
        let wall = contacts.wall;
        car.body_contact = wall.is_some();
        car.wall_time = if wall.is_some() { 0.0 } else { car.wall_time + dt };
        let side_axis = rot * Vec3::X;
        let (dv, dw) = car.turn_assist.step(
            &TurnAssistInput {
                steer: ctl.steer,
                throttle,
                mph: band_speed / MPH,
                wall_time: car.wall_time,
                wall,
                forward,
                side: side_axis,
                v_side: velocity.dot(side_axis),
                yaw_rate: car.yaw_rate,
            },
            dt,
        );
        if dv != 0.0 || dw != 0.0 {
            *forces.linear_velocity_mut() += side_axis * dv;
            *forces.angular_velocity_mut() += up * dw;
        }

        // --- The game's drift model (drift.rs) ---------------------------------
        car.rear_slip = if rear_slip.1 > 0 { rear_slip.0 / rear_slip.1 as f32 } else { 0.0 };
        car.air_time = if wheels_down == car.wheels.len() { 0.0 } else { car.air_time + dt };
        // Vehicle +0x1ed0 (`FUN_009ae840`): reset only while all four wheels are off the ground and the body moves
        // faster than `.bss` 0x01186630 = 1.8 m/s (read live), else it grows by dt; the next tick's gearbox gates
        // its downshifts on it.
        car.drivetrain.grounded_time = if wheels_down == 0 && velocity.length() > AIRBORNE_MIN_SPEED {
            0.0
        } else {
            car.drivetrain.grounded_time + dt
        };
        let flat_v = velocity.reject_from(up);
        if let Some(drift) = &p.drift {
            // The drift reads the steering relative to the current range (full range = full lock).
            let range = steer_range(&p.steering, band_speed, car.gear < 0, pedal);
            let out = car.drift.step(
                drift,
                &DriftInput {
                    steer: (ctl.steer * p.steering.max_angle / range.max(1.0)).clamp(-1.0, 1.0),
                    throttle,
                    brake: ctl.brake,
                    handbrake: car.handbrake_on,
                    handbrake_prev: handbrake,
                    touching_world: wall.is_some(),
                    world_impact: contacts.world_impact,
                    object_impact: contacts.object_impact,
                    speed: velocity.length(),
                    mph: band_speed / MPH,
                    angle: -car.body_slip,
                    rear_slip: car.rear_slip,
                    yaw_rate: car.yaw_rate,
                    mass: p.mass,
                    all_grounded: wheels_down == car.wheels.len(),
                    any_grounded: grounded,
                    air_time: car.air_time,
                    reverse_gear: car.gear < 0,
                    surface: surface::drift_factor(car.wheels.iter().map(|w| &w.surface)),
                },
                dt,
            );
            forces.apply_torque(up * out.yaw_torque);
            let left = (rot * Vec3::X).reject_from(up).normalize_or_zero();
            let hold = flat_v.normalize_or_zero() * out.hold_force + forward.reject_from(up) * out.hold_forward;
            forces.apply_force(left * out.side_force + hold);
            // A drift that ends while counter-steering scales the wheels' angle by `Drift_exit.ca545e03`
            // (`FUN_009d1120`): the next tick's tyres and steering start from it.
            if let Some(k) = out.exit_steer_scale
                && car.steer * car.drift.direction < 0.0
            {
                car.steer *= k;
            }
            // The drift damps the yaw rate and the sideways velocity directly.
            if out.yaw_rate_scale < 1.0 || out.lateral_scale < 1.0 {
                let w = forces.angular_velocity();
                *forces.angular_velocity_mut() = w - up * w.dot(up) * (1.0 - out.yaw_rate_scale);
                let v = forces.linear_velocity();
                *forces.linear_velocity_mut() = v - left * v.dot(left) * (1.0 - out.lateral_scale);
            }
        }
        // --- Wall scrape (`FUN_008f7ab0`, profiler scope "Update wall scrape", after the drift) ---------------
        // A world contact above 6 zeroes the steering's fraction and angle (+0x1c80, +0x1c8c; the range +0x1c88 is
        // kept): the next tick's tyres run straight and the steering ramps out again from the centre.
        if contacts.world_impact > mw_handling::drift::HARD_IMPACT {
            debug!(
                "wall scrape: contact {:.1} m/s, steering {:.1}° -> 0",
                contacts.world_impact,
                car.steer.to_degrees()
            );
            car.steer = 0.0;
        }
        if let Some(n) = ground_normal.try_normalize() {
            car.roll = -(rot * Vec3::X).dot(n).clamp(-1.0, 1.0).asin();
            car.pitch = forward.dot(n).clamp(-1.0, 1.0).asin();
        }
        // The game's per-axis drag (with its over-speed ramp) and lift (downforce). Rolling resistance is per
        // wheel, from the surface (above).
        let drag = p.chassis.drag_force(rot, velocity);
        forces.apply_force(drag);
        let (lift, lift_at) = p.chassis.lift(rot, velocity, p.centre_of_mass);
        forces.apply_force_at_point(up * lift, pos + rot * lift_at);
        // Once a wheel leaves the ground the game pulls the car down harder than gravity and, for a moment,
        // damps the body's rotation (take-off).
        if wheels_down < car.wheels.len() {
            forces.apply_force(Vec3::NEG_Y * (p.chassis.in_air_gravity - 1.0) * p.mass * 9.81);
            if car.air_time < p.chassis.take_off_time {
                let w = rot.inverse() * forces.angular_velocity();
                let k = p.chassis.take_off_damping(w, band_speed / MPH, dt);
                *forces.angular_velocity_mut() = rot * (w * k);
            }
        }
        // --- The contact passes and the integration that open the game's next tick ------------------------------
        // The game's tick runs two contact passes (`FUN_00932190` → `FUN_009c5a90`), integrates the positions
        // (`FUN_009324f0`), then the sub-steps and the tyres; ours runs from its sub-steps, so these come last, before
        // avian integrates the chassis. Per pass and wheel: the penetration of the ground into the tyre predicted for
        // the end of the tick; where it is not negative the contact is activated and the wheel's velocity along the
        // normal rises by it / dt, and the strut stop moves the chassis by its excess past the sidewall.
        if wheels_down < car.wheels.len() {
            // The in-air gravity pulls the wheels too (decomp.md "Suspension, wheels, aerodynamics", airborne).
            for wheel in car.wheels.iter_mut() {
                wheel.vel -= (p.chassis.in_air_gravity - 1.0) * 9.81 * dt * up.y;
            }
        }
        let mut ch =
            Chassis { v: forces.linear_velocity(), w: forces.angular_velocity(), com, mass: p.mass, inv_inertia };
        for wheel in car.wheels.iter_mut() {
            wheel.touching = false;
            wheel.vel_substeps = wheel.vel;
        }
        for _ in 0..STOP_CONTACT_PASSES {
            for (wheel, ground) in car.wheels.iter_mut().zip(&grounds) {
                // The stop's flag (+0x4ae) is cleared at the top of every call.
                wheel.stop_share = None;
                let Some(g) = ground else { continue };
                // The ground point against the wheel's contact point (its centre + the record's offset), along the
                // normal: d = (ground point − (centre + offset + v dt)) · n.
                let centre = wheel.world_centre(pos, rot);
                let vn = ch.wheel_velocity(centre, up, wheel.vel).dot(g.normal);
                let depth = (g.point - (centre + g.offset)).dot(g.normal);
                let Some(dvn) = suspension::contact_correction(depth - vn * dt, dt) else { continue };
                wheel.touching = true;
                wheel_ground_impulse(wheel, &mut ch, centre, g.normal, up, dvn);
                // The strut stop (+0x4ac): the same with the contact offset taken from the strut's end (+0x460).
                // `FUN_0095b4d0` rebuilds the constraint each tick with +0x4ac = vehicle +0x1774, the crashed state
                // (`FUN_009b2d40`; 0 for the player in both snapshots): the stop only acts on a crashed car.
                if !(ctl.crashed || strut_stop_forced()) {
                    continue;
                }
                let mount = wheel.strut_end(pos, rot);
                let vn = ch.velocity_at(mount).dot(g.normal);
                let over = (g.point - (mount + g.offset)).dot(g.normal) - vn * dt;
                if let Some((excess, share)) = suspension::stop_excess(over, wheel.stop_slack) {
                    wheel.stop_share = Some(share);
                    let impulse = ch.effective_mass(mount, g.normal) * excess / dt;
                    ch.push(mount, g.normal * impulse);
                    stop_total += impulse;
                }
            }
            // The strut joints' position pass (`FUN_00928cd0` → `FUN_009c6230`), after the wheel constraints: the
            // wheel's predicted position relative to the strut's (both + v dt) kept between the strut's ends.
            if joint_pos {
                for wheel in car.wheels.iter_mut() {
                    let centre = wheel.world_centre(pos, rot);
                    let v_rel = wheel.vel - ch.velocity_at(centre).dot(up);
                    let y_pred = wheel.compression - wheel.sag + v_rel * dt;
                    let err = suspension::strut_position_error(y_pred, -wheel.droop, wheel.travel - wheel.sag);
                    strut_joint_impulse(wheel, &mut ch, centre, up, err / dt);
                }
            }
        }
        // The wheels' integration (x += v dt; the chassis is avian's).
        for wheel in car.wheels.iter_mut() {
            let centre = wheel.world_centre(pos, rot);
            wheel.compression_rate = wheel.vel - ch.velocity_at(centre).dot(up);
            wheel.world = Some(centre + ch.wheel_velocity(centre, up, wheel.vel) * dt);
        }
        *forces.linear_velocity_mut() = ch.v;
        *forces.angular_velocity_mut() = ch.w;
        car.stop_impulse = stop_total;
        let driven_radius = car.wheels.iter().filter(|w| !w.front).map(|w| w.radius).next().unwrap_or(0.33);
        car.debug_forces = [drive_torque / driven_radius, tyre_long_total, drag.dot(forward) + resist_total];
        car.tyre_yaw = tyre_yaw;
        if player {
            trace_engine(time.elapsed_secs(), car, throttle, speed);
            trace_wheels(time.elapsed_secs(), car);
        }

        // Fell out of the world: back to the spawn point.
        if pos.y < car.spawn.y - 200.0 {
            transform.translation = car.spawn;
            *forces.linear_velocity_mut() = Vec3::ZERO;
        }
    }
}

/// What the ground under one wheel does to its tyre (`FUN_009d8bd0`: `FUN_0054d750` looks the surface up,
/// `FUN_009d4b70` finds the car's link for it, `FUN_009cec40` blends the grip, `FUN_009d4a10` the drag).
struct WheelSurface {
    /// The surface's index in the game's surface list (the collision tag's bits 13–19, or the test track's).
    index: Option<usize>,
    /// The `SurfacePhysics` unique id (`u32::MAX` when the surface is not known).
    uid: u32,
    /// (lateral, longitudinal) grip factors on the tyre caps, before the fade of a surface just left.
    grip: [f32; 2],
    /// The car's link grip for this surface (1 without a link): what the fades head for.
    target: f32,
    /// `Surface_roughness` as (noise frequency per metre, amplitude in metres); `None` when flat.
    roughness: Option<(f32, f32)>,
    /// `RoadOverrides` rolling resistance per newton of load.
    rolling: f32,
    /// Slide drag: the surface's scale × the car's link (drag ramp ends, N per mph; slip angles in degrees).
    drag_scale: f32,
    drag: [f32; 2],
    drag_slip: [f32; 2],
    /// The surface's dirt and dust build-up (`FUN_009d19c0`; the dry weather block, like the grip).
    effects: Option<mw_assets::surfaces::VehicleEffects>,
}

impl WheelSurface {
    /// The surface a wheel's ray (`origin`, `dir`, `reach`) hit on `entity`. World colliders carry per-triangle
    /// surface tags; anything else (the flat test track) counts as the game's `Surfaces.Default_surface`, the
    /// list's first entry (or the `--track-surface` entry). Without a surface list at all the tyre gets full grip
    /// and no drag.
    fn at(
        surfaces: &crate::props::WorldSurfaces,
        world_tags: &Query<(&Collider, &crate::props::SurfaceTags)>,
        entity: Entity,
        (origin, dir, reach): (Vec3, Dir3, f32),
        p: &CarParams,
        front: bool,
    ) -> Self {
        let hit = world_tags
            .get(entity)
            .ok()
            .and_then(|(collider, tags)| surfaces.surface_at(collider, tags, origin, *dir, reach));
        let index = Some(hit.as_ref().map_or(surfaces.untagged, |h| h.tag.surface as usize));
        let tagged = hit.and_then(|hit| hit.surface);
        let info = tagged.or_else(|| surfaces.list.get(surfaces.untagged).or(surfaces.list.first()).cloned());
        let base = info.as_ref().and_then(|s| s.grip).unwrap_or(1.0);
        let uid = info.as_ref().and_then(|s| s.physics_uid);
        let link = uid.and_then(|u| p.surface_links.iter().find(|l| l.surfaces.contains(&u)));
        // The car's link blends the surface grip towards its own value, per axle and direction.
        let grip = match link {
            Some(l) => {
                let f = if front { l.front } else { l.rear };
                [base + (l.grip - base) * f[0], base + (l.grip - base) * f[1]]
            }
            None => [base, base],
        };
        let roughness =
            info.as_ref().and_then(|s| s.roughness).map(surface::roughness_scale).filter(|&(f, a)| f > 0.0 && a > 0.0);
        Self {
            index,
            uid: uid.unwrap_or(u32::MAX),
            grip,
            target: link.map_or(1.0, |l| l.grip),
            roughness,
            rolling: info.as_ref().and_then(|s| s.rolling).unwrap_or(0.0),
            drag_scale: if link.is_some() { info.as_ref().and_then(|s| s.slide_drag).unwrap_or(0.0) } else { 0.0 },
            drag: link.map_or([0.0; 2], |l| l.drag),
            drag_slip: link.map_or([0.0, 1.0], |l| l.drag_slip),
            effects: info.as_ref().and_then(|s| s.vehicle_effects[0]),
        }
    }

    /// Resistance (N, a magnitude) for a wheel with grip load `load` (N) at `mph` with `slip_deg` of slip angle.
    fn resistance(&self, load: f32, mph: f32, slip_deg: f32) -> f32 {
        let [lo, hi] = self.drag_slip;
        let t = ((slip_deg - lo) / (hi - lo).max(DRAG_SLIP_SPAN_MIN)).clamp(0.0, 1.0);
        let drag = self.drag[0] + (self.drag[1] - self.drag[0]) * t;
        self.rolling * load + mph * self.drag_scale * drag
    }
}

/// The axle limited-slip differentials: each axle's wheel speeds may differ by at most a ratio of 1 / c
/// (`Front_` / `Rear_Differential`; 0 open, 1 locked). Excess is shared out between the two wheels.
fn limited_slip(wheels: &mut [WheelState], diff: &[f32; 2]) {
    for (axle, &c) in diff.iter().enumerate() {
        let idx: Vec<usize> = (0..wheels.len()).filter(|&i| usize::from(!wheels[i].front) == axle).collect();
        let &[i, j] = idx.as_slice() else { continue };
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0 {
            continue;
        }
        let (a, b) = (wheels[i].omega, wheels[j].omega);
        let sum = a + b;
        if sum.abs() < 1e-3 {
            continue;
        }
        let ratio = (a - b).abs() / sum.abs();
        let bias = (1.0 - c) / (1.0 + c);
        if ratio > bias {
            let shift = (ratio - bias) * sum.abs() / 2.0 * (a - b).signum();
            wheels[i].omega -= shift / 2.0;
            wheels[j].omega += shift / 2.0;
        }
    }
}

/// The ground under a wheel this tick (its contact record, constraint +0x80: normal, ground point, the contact's
/// offset from the wheel centre) and the surface there.
struct Ground {
    normal: Vec3,
    point: Vec3,
    offset: Vec3,
    surface: WheelSurface,
}

/// What the wheel probe found: the world collider, the ground point, its normal (out of the ground) and the
/// contact point on the wheel relative to the wheel's centre (the contact record's +0x10, +0x00 and +0x20).
struct ProbeHit {
    entity: Entity,
    point: Vec3,
    normal: Vec3,
    offset: Vec3,
}

/// The world's static shapes the wheels stand on (`GameLayer::World`).
type WorldShapes<'w, 's> = Query<
    'w,
    's,
    (
        &'static Collider,
        &'static Position,
        &'static Rotation,
        Option<&'static CollisionLayers>,
        Option<&'static crate::props::SurfaceTags>,
    ),
>;

/// The wheel's contact with the world (`mw_handling::wheel_probe`): the wheel body's cylinder (radius, the body's
/// half width along the axle) coming down its strut onto the
/// nearby world triangles; the first it meets is the contact (the game takes the first of its wheel's contact list,
/// `FUN_009e04a0`; which one its job lists first is not known: model decision, handling.md). `MW_WHEEL_RAY=1` uses
/// the ray along the strut of before (a point under the centre) for A/B.
#[allow(clippy::too_many_arguments)]
fn probe_ground(
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    shapes: &WorldShapes,
    wheel: &WheelState,
    mount: Vec3,
    up: Vec3,
    axle: Vec3,
    down: Dir3,
) -> Option<ProbeHit> {
    use mw_handling::wheel_probe;
    static RAY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ray = *RAY.get_or_init(|| std::env::var_os("MW_WHEEL_RAY").is_some_and(|v| v != "0"));
    // Triangles flagged as decals (collision tag bit 3, `CollisionTag::DECAL`: road plates and strips lying up to
    // 0.1 m above the road, posts) are not under the wheels: the original crosses them without a bump (`ref/live1`
    // 77.7 s and 83.8 s, MP4-12C at 70 / 99 mph; ours meeting them: 4.6× the original's vertical-acceleration RMS
    // over that stretch, skipping them: 0.93×). `MW_WHEEL_DECAL=1` keeps them for A/B.
    static SKIP_DECAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let skip_decal = *SKIP_DECAL.get_or_init(|| std::env::var_os("MW_WHEEL_DECAL").is_none_or(|v| v == "0"));
    // Ground a wheel can reach within a tick: down to `WHEEL_DROOP_REACH` below full droop.
    let reach = wheel.travel + WHEEL_DROOP_REACH;
    let r = wheel.radius;
    if ray {
        let hit = spatial.cast_ray(mount + up * r, down, reach + 2.0 * r, true, filter)?;
        let normal = hit.normal.normalize_or_zero();
        return (normal.dot(up) > 0.0).then(|| ProbeHit {
            entity: hit.entity,
            point: mount + up * (r - hit.distance),
            normal,
            offset: -up * r,
        });
    }
    let half_length = 0.5 * wheel.width;
    let cyl = wheel_probe::Wheel {
        axis: axle.normalize_or(Vec3::X).as_dvec3().to_array(),
        up: up.as_dvec3().to_array(),
        radius: r as f64,
        half_length: half_length as f64,
    };
    let extent = Vec3::splat(2.0 * r + half_length + reach);
    let (lo, hi) = (mount - extent, mount + extent);
    let mut profile = wheel_probe::Profile::new(cyl).with_limits(r as f64, -(reach as f64));
    let mut consider = |entity: Entity, tri: [Vec3; 3]| {
        profile.add(&tri.map(|p| (p.as_dvec3() - mount.as_dvec3()).to_array()), entity.to_bits());
    };
    for entity in spatial.aabb_intersections_with_aabb(ColliderAabb::from_min_max(lo, hi)) {
        let Ok((collider, position, rotation, layers, tags)) = shapes.get(entity) else { continue };
        if layers.is_some_and(|l| !l.memberships.has_all(crate::props::GameLayer::World))
            || !filter.test(entity, layers.copied().unwrap_or_default())
        {
            continue;
        }
        let (p, q) = (position.0, rotation.0);
        let to_world = |v: Vec3| p + q * v;
        let shape = collider.shape_scaled();
        if let Some(mesh) = shape.as_trimesh() {
            // The query box in the mesh's frame.
            let corners = [lo, hi].map(|c| q.inverse() * (c - p));
            let centre = (corners[0] + corners[1]) * 0.5;
            let half = Mat3::from_quat(q.inverse()).abs() * (extent);
            let (a, b) = (centre - half, centre + half);
            let aabb = avian3d::parry::bounding_volume::Aabb::new(a.to_array().into(), b.to_array().into());
            for i in mesh.bvh().intersect_aabb(&aabb) {
                if skip_decal
                    && tags
                        .and_then(|t| t.0.get(i as usize))
                        .is_some_and(|&t| mw_formats::CollisionTag::from_u32(t).has(mw_formats::CollisionTag::DECAL))
                {
                    continue;
                }
                let t = mesh.triangle(i);
                let v = |x: [f32; 3]| to_world(Vec3::from(x));
                consider(entity, [v(t.a.into()), v(t.b.into()), v(t.c.into())]);
            }
        } else if let Some(cuboid) = shape.as_cuboid() {
            let h: [f32; 3] = cuboid.half_extents.into();
            let h = Vec3::from(h);
            let corner =
                |i: usize| to_world(h * Vec3::new([-1.0, 1.0][i & 1], [-1.0, 1.0][(i >> 1) & 1], [-1.0, 1.0][i >> 2]));
            // Two triangles per face, corners indexed by bits (x, y, z).
            const FACES: [[usize; 4]; 6] =
                [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
            for f in FACES {
                consider(entity, [corner(f[0]), corner(f[1]), corner(f[2])]);
                consider(entity, [corner(f[0]), corner(f[2]), corner(f[3])]);
            }
        }
    }
    let rest = profile.resolve()?;
    let entity = Entity::try_from_bits(rest.tag)?;
    let normal = DVec3::from(rest.normal).as_vec3();
    // `FUN_009e04a0`: a contact whose normal points against the chassis's up axis is dropped.
    if normal.dot(up) <= 0.0 {
        return None;
    }
    Some(ProbeHit {
        entity,
        point: mount + DVec3::from(rest.point).as_vec3(),
        normal,
        offset: DVec3::from(rest.offset).as_vec3(),
    })
}

impl WheelState {
    /// The strut's end on the chassis (the wheel centre at full bump), which is also where ours anchors the
    /// spring-damper element (the game's anchor is higher up the same line: element +0x20, 0.353 m above rest on
    /// the XKR; along the strut only the rest length minus that distance, the sag, matters).
    fn strut_end(&self, pos: Vec3, rot: Quat) -> Vec3 {
        pos + rot * (self.centre + Vec3::Y * (self.travel - self.sag))
    }

    /// The wheel's point mass in the world: `compression - sag` above its rest position along the car's up axis.
    fn world_centre(&self, pos: Vec3, rot: Quat) -> Vec3 {
        pos + rot * (self.centre + Vec3::Y * (self.compression - self.sag))
    }
}

/// The chassis's velocity state while the island's passes change it with impulses (applied at once, as the
/// game's `FUN_0097e440` + `FUN_00907840` do).
struct Chassis {
    v: Vec3,
    w: Vec3,
    com: Vec3,
    mass: f32,
    inv_inertia: Mat3,
}

impl Chassis {
    fn velocity_at(&self, point: Vec3) -> Vec3 {
        self.v + self.w.cross(point - self.com)
    }

    /// A wheel record's velocity: the chassis's at its centre across the strut, its own along it.
    fn wheel_velocity(&self, centre: Vec3, up: Vec3, along: f32) -> Vec3 {
        self.velocity_at(centre).reject_from(up) + up * along
    }

    /// The inverse effective mass at a point along a direction (`FUN_009e8230`).
    fn inv_mass_at(&self, point: Vec3, dir: Vec3) -> f32 {
        let arm = (point - self.com).cross(dir);
        1.0 / self.mass + arm.dot(self.inv_inertia * arm)
    }

    fn effective_mass(&self, point: Vec3, dir: Vec3) -> f32 {
        1.0 / self.inv_mass_at(point, dir).max(1e-6)
    }

    fn push(&mut self, point: Vec3, impulse: Vec3) {
        self.v += impulse / self.mass;
        self.w += self.inv_inertia * (point - self.com).cross(impulse);
    }
}

/// The ground raises a wheel's velocity along its normal by `dvn` (the contact pass's correction or the impulse
/// pass's stop). Ours slides along the car's up axis only, so it takes the change along that axis, and the part of
/// the ground's impulse across the strut goes into the chassis at the wheel centre (the strut joints' reaction,
/// which keep the game's wheel in place on its strut; ours, handling.md "Model decisions"). Returns the ground's
/// impulse (N s).
fn wheel_ground_impulse(
    wheel: &mut WheelState,
    ch: &mut Chassis,
    centre: Vec3,
    normal: Vec3,
    up: Vec3,
    dvn: f32,
) -> f32 {
    let along = normal.dot(up).max(TYRE_NORMAL_MIN_UP);
    let du = dvn / along;
    wheel.vel += du;
    let impulse = wheel.mass * du / along;
    ch.push(centre, (normal - up * normal.dot(up)) * impulse);
    impulse
}

/// The strut joint's impulse for a velocity change `dv` of the wheel relative to the chassis along `up` (removed):
/// through the effective mass (the chassis's at the wheel centre, `FUN_009e8230`, plus the wheel's 1/m), onto the
/// chassis at the wheel centre (`FUN_009ca290` / `FUN_0097e440` + `FUN_00907840`) and against the wheel.
fn strut_joint_impulse(wheel: &mut WheelState, ch: &mut Chassis, centre: Vec3, up: Vec3, dv: f32) {
    if dv == 0.0 {
        return;
    }
    let j = dv / (ch.inv_mass_at(centre, up) + 1.0 / wheel.mass);
    ch.push(centre, up * j);
    wheel.vel -= j / wheel.mass;
}

/// The strut joint's passes in use, (position, velocity): both by default; `MW_STRUT_JOINT=0` leaves the joint out
/// (the model before 2026-10-19), `=pos` / `=vel` keeps one pass (A/B).
fn strut_joint_passes() -> (bool, bool) {
    static ON: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("MW_STRUT_JOINT").as_deref() {
        Ok("0") => (false, false),
        Ok("pos") => (true, false),
        Ok("vel") => (false, true),
        _ => (true, true),
    })
}

/// `MW_ROUGH=0` leaves the surface bumps out (`FUN_009d77b0`; A/B: the yaw they cause, research log 2026-10-19).
fn rough_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_ROUGH").is_none_or(|v| v != "0"))
}

/// `MW_STRUT_STOP=1` runs the strut stop on every car, crashed or not (A/B against the model before 2026-10-19).
fn strut_stop_forced() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_STRUT_STOP").is_some_and(|v| v == "1"))
}

/// Sub-steps of the game's island solve (`FUN_0093f200` loops world +0x464 = 2 times, set by the world's
/// constructor `FUN_009330d0` and read live): each runs the wheel contacts' impulse pass (`FUN_009c2670`) and the
/// force elements (`FUN_00932f90`: dampers, springs, anti-roll bars) at dt / 2.
const SOLVER_SUBSTEPS: usize = 2;
/// The island's contact passes (`FUN_00932190` loops world +0x460 = 2 times; same constructor, read live): each
/// runs the wheel constraints' contact pass `FUN_009c5a90` (the wheel onto the ground, the strut stop).
const STOP_CONTACT_PASSES: usize = 2;
/// A wheel further than this from its rest position (m) is put back there: the car was moved (ours).
const WHEEL_RESYNC: f32 = 1.0;
/// How far below the fully drooped wheel the ground probe still looks (m; ours): a contact record exists for ground
/// the wheel can reach within a tick.
const WHEEL_DROOP_REACH: f32 = 0.3;

/// The anti-roll bars, one sub-step of `h` (`FUN_0092ee70`, reached through `FUN_00932f90` for the type-2 elements
/// that `FUN_00933a60` builds; decomp.md "Suspension, wheels, aerodynamics"). Per axle: the two wheel centres in
/// the body's frame give the axle line's roll against the chassis, θ = atan(Δy / Δx); the bar's force is
/// F = (k θ − c ω·forward) / Δx with the chassis's own roll rate (not relative to the axle), k = data × 60 and
/// c = 2 ζ √(k_data I) (element +0x20 / +0x30). Each wheel's share is capped at |Δy| / (4 W dt²) with W the
/// chassis's inverse effective mass along its up axis at the wheel (`FUN_009275b0`, the full tick's dt), then +F up
/// at one wheel centre and −F at the other act on the chassis only (the wheel records' +0x0c is 1 in every read,
/// which masks the wheels' share). θ is taken from the positions, which the sub-steps do not move.
fn anti_roll(wheels: &[WheelState], p: &CarParams, (pos, rot): (Vec3, Quat), ch: &mut Chassis, dt: f32, h: f32) {
    let (up, forward) = (rot * Vec3::Y, rot * Vec3::Z);
    let centre = |wheel: &WheelState| wheel.centre + Vec3::Y * (wheel.compression - wheel.sag);
    for front in [true, false] {
        let mut axle = wheels.iter().filter(|w| w.front == front);
        let (Some(a), Some(b)) = (axle.next(), axle.next()) else { continue };
        let (ca, cb) = (centre(a), centre(b));
        let (dx, dy) = (ca.x - cb.x, ca.y - cb.y);
        if dx.abs() < 0.1 {
            continue;
        }
        let theta = (dy / dx).atan();
        let i = usize::from(!front);
        let k = p.chassis.anti_roll[i];
        let c = p.chassis.roll_damping[i] * p.inertia.z.max(0.0).sqrt();
        let force = (k * theta - c * ch.w.dot(forward)) / dx;
        for (point, sign) in [(pos + rot * ca, 1.0), (pos + rot * cb, -1.0)] {
            let cap = dy.abs() * ch.effective_mass(point, up) / (4.0 * dt * dt);
            ch.push(point, up * (sign * force.clamp(-cap, cap) * h));
        }
    }
}

/// The steering range (wheel angle at full input, degrees) at `speed` (m/s, signed) the way the game sets it
/// (`SteeringBehaviour`): from 0 mph to `High_speed` off the throttle; on the throttle a three-point curve
/// through `Medium_speed`, blended by `pedal` (throttle minus brake); in reverse gear towards
/// `Reverse_steering_range` by `Reverse_speed`.
fn steer_range(s: &Steering, speed: f32, reverse: bool, pedal: f32) -> f32 {
    let mph = speed / MPH;
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let range = if reverse {
        lerp(s.range[0], s.range[2], (mph / s.reverse_speed_mph.min(-1.0)).clamp(0.0, 1.0))
    } else {
        let coast = lerp(s.range[0], s.range[1], (mph / s.high_speed_mph.max(1.0)).clamp(0.0, 1.0));
        let on_throttle = if mph < s.medium_speed_mph {
            lerp(s.range[0], s.throttle_range[0], (mph / s.medium_speed_mph.max(1.0)).clamp(0.0, 1.0))
        } else {
            let t = (mph - s.medium_speed_mph) / (s.high_speed_mph - s.medium_speed_mph).max(1.0);
            lerp(s.throttle_range[0], s.throttle_range[1], t.clamp(0.0, 1.0))
        };
        lerp(coast, on_throttle, pedal.clamp(0.0, 1.0))
    };
    range.min(s.max_angle)
}

/// Move the front wheel angle (radians) towards the input the way the game does (`FUN_009d31f0`; docs/decomp.md
/// "Steering"). Rates blend from 0 mph to `High_speed`; in reverse gear the reverse values apply. `raw_input` is
/// the stick before the in-drift remap (the full-rate test reads it), `input` the one steered to. `drift_rate`
/// (degrees per second, blend) pulls the rate towards the drift's counter-steer rate.
///
/// With no input the wheels stand at the neutral offset q × `Maximum_steering_angle` × gain (gain `553bc073`
/// forwards, 1 in reverse gear, 0 in a drift); q is the game's vehicle +0x2c70, the body slip as a fraction
/// ([`steer_slip_fraction`]), signed here so that the offset turns the front wheels towards the direction of
/// travel (the game's −q in its own sign). Each side's limit grows by |offset| × `a40a6b17` on the side the offset
/// points to and × `16fd6022` on the other (capped at `Maximum_steering_angle`); the input steers from the offset
/// to the side's limit.
#[allow(clippy::too_many_arguments)]
fn steer_towards(
    s: &Steering,
    current: f32,
    raw_input: f32,
    input: f32,
    speed: f32,
    reverse: bool,
    pedal: f32,
    drift_rate: Option<(f32, f32)>,
    slip_q: f32,
    drifting: bool,
    dt: f32,
) -> f32 {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let t_high = (speed / MPH / s.high_speed_mph.max(f32::EPSILON)).clamp(0.0, 1.0);
    let range = steer_range(s, speed, reverse, pedal);
    let gain = if drifting {
        0.0
    } else if reverse {
        1.0
    } else {
        s.neutral_gain
    };
    let offset = slip_q * s.max_angle * gain;
    let side_limit = |side: f32| {
        let k = if offset * side > 0.0 { s.offset_range_gain[0] } else { s.offset_range_gain[1] };
        (range + k * offset.abs()).min(s.max_angle)
    };
    let (limit_pos, limit_neg) = (side_limit(1.0), side_limit(-1.0));
    let input = input.clamp(-1.0, 1.0);
    let side = if input >= 0.0 { limit_pos } else { -limit_neg };
    let target = (offset + input.abs() * (side - offset)).clamp(-limit_neg, limit_pos);
    let cur = current.to_degrees();
    let pick = |v: [f32; 3]| if reverse { v[2] } else { lerp(v[0], v[1], t_high) };
    // Moving back towards centre (the target lies on the same side as the neutral position): the counter-steer
    // rate when the input points across the current angle, otherwise the centring rate (easing off part of the
    // way counts as centring). Steering outwards: the steer rate, or the raw high-speed one when the stick is
    // short of full travel.
    let towards_centre = (target - cur).signum() == (offset - cur).signum() && cur != offset;
    let rate = if towards_centre {
        if cur * input < 0.0 { pick(s.counter_rate) } else { pick(s.center_rate) }
    } else if raw_input.abs() >= STEER_FULL_INPUT {
        pick(s.rate)
    } else {
        s.rate[1]
    };
    let rate = match drift_rate {
        Some((drift, blend)) => lerp(rate, drift, blend),
        None => rate,
    };
    let step = rate * dt;
    (cur + (target - cur).clamp(-step, step)).clamp(-limit_neg, limit_pos).to_radians()
}

/// The steering's q (the game's vehicle +0x2c70, read by the steering through `FUN_00904260` /
/// `FUN_00902930` → `FUN_009d73f0` → `FUN_009d31f0`; its writer is not found in the code). Read from the
/// original's telemetry (`ref/xkr_launch`, 2026-10-06): q = body slip / 35°, the slip being
/// atan2(lateral, |forward|) of the body velocity, the ratio constant to four digits over 0.5°–33°; below 10 mph
/// it fades in proportion to the speed (|velocity| / 10 mph, constant over 1–4 m/s). `lateral` and `forward` are
/// the body velocity's components along up × forward and forward (m/s). The ±1 clamp is a model decision (the
/// run's largest was 0.95).
fn steer_slip_fraction(lateral: f32, forward: f32) -> f32 {
    let speed = lateral.hypot(forward);
    if speed < 1e-3 {
        return 0.0;
    }
    let slip = lateral.atan2(forward.abs()).to_degrees();
    (slip / STEER_Q_SLIP_DEG).clamp(-1.0, 1.0) * (speed / (STEER_Q_FADE_MPH * MPH)).min(1.0)
}

/// Body slip (degrees) at which the steering's q reaches 1 (telemetry reading, [`steer_slip_fraction`]). It equals
/// the XKR's `Maximum_steering_angle`; one car cannot tell a constant from that field.
const STEER_Q_SLIP_DEG: f32 = 35.0;
/// Speed (mph) below which q fades in proportion (telemetry reading, [`steer_slip_fraction`]).
const STEER_Q_FADE_MPH: f32 = 10.0;

/// The car's contacts on the last physics step, in the shape of the game's contact table (vehicle +0x303c):
/// whether and where the body touches the world, and the strongest world and object contact.
#[derive(Debug, Default, Clone, Copy)]
struct Contacts {
    /// The body's contact normal with the world, pointing from the world at the car (props do not count: the
    /// game's flag is set by world contacts).
    wall: Option<Vec3>,
    /// The strongest contact with the world, and with anything else (props, through the body or the bumper
    /// box). The game's value is the table's record +0x1c / entry +0xc, whose unit is not traced; ours is the
    /// largest normal closing speed (m/s) before the solve, the stand-in the camera's impact shake uses.
    world_impact: f32,
    object_impact: f32,
    /// The strongest contact with another car's body (m/s, as above).
    car_impact: f32,
}

fn car_contacts(collisions: &Collisions, layers: &Query<&CollisionLayers>, car: &Car) -> Contacts {
    let mut c = Contacts::default();
    for collider in [car.body_collider, car.bumper_collider] {
        for pair in collisions.collisions_with(collider) {
            let (other, sign) = if pair.collider1 == collider { (pair.collider2, -1.0) } else { (pair.collider1, 1.0) };
            let world = layers.get(other).map_or(true, |l| l.memberships.has_all(crate::props::GameLayer::World));
            let speed =
                pair.manifolds.iter().flat_map(|m| m.points.iter()).map(|p| -p.normal_speed).fold(0.0, f32::max);
            let other_car = layers.get(other).is_ok_and(|l| l.memberships.has_all(crate::props::GameLayer::Car));
            if other_car {
                c.car_impact = c.car_impact.max(speed);
            } else if world {
                c.world_impact = c.world_impact.max(speed);
                if collider == car.body_collider && c.wall.is_none() {
                    c.wall = pair.manifolds.first().map(|m| m.normal * sign);
                }
            } else {
                c.object_impact = c.object_impact.max(speed);
            }
        }
    }
    c.object_impact = c.object_impact.max(car.prop_impact);
    c
}

/// Yaw damping after a collision (`FUN_008f7950`, called from the tick `FUN_00904da0` before the driveline). The
/// collision analysis (`FUN_00918600`) sets one of three timers to 1 (vehicle +0x2e80 / +0x2e90 / +0x2ea0, through
/// `FUN_008f5620` / `5640` / `5660`): a world contact on its first tick (`FUN_00918120`; the analyzer's per-car
/// flag at +0x220 holds it off while the contact lasts), a contact with another car (`FUN_00917bb0`) or with an
/// object (`FUN_009175e0`). Each tick the yaw rate in body axes (the up axis only) is multiplied by
/// (1 − 0.05 × the largest timer)^(60 dt) (`FUN_00907b50`; 0.05 `.rdata` 0x01021fd0), then the timers fall
/// linearly, the first two over 1 s (0x01021ef0), the object's over 0.3 s (0x01021fe0). Read from the
/// disassembly; not modelled: the analyzer's gates (`FUN_006a3b50`, the special state +0x1774). A car contact
/// sets its timer on every tick it lasts (whether `FUN_00917bb0` gates it like the world record is not read).
#[derive(Debug, Default, Clone, Copy)]
struct ImpactYawDamping {
    /// World, car, object.
    timers: [f32; 3],
}

const IMPACT_YAW_DAMP: f32 = 0.05;
const IMPACT_YAW_TIMES: [f32; 3] = [1.0, 1.0, 0.3];

impl ImpactYawDamping {
    fn hit(&mut self, new_world_contact: bool, car_contact: bool, object_contact: bool) {
        if new_world_contact {
            self.timers[0] = 1.0;
        }
        if car_contact {
            self.timers[1] = 1.0;
        }
        if object_contact {
            self.timers[2] = 1.0;
        }
    }

    /// Damps the angular velocity `w` about the body's `up` axis and runs the timers down; returns the new `w`.
    fn step(&mut self, w: Vec3, up: Vec3, dt: f32) -> Vec3 {
        let d = IMPACT_YAW_DAMP * self.timers.iter().copied().fold(0.0, f32::max);
        for (t, time) in self.timers.iter_mut().zip(IMPACT_YAW_TIMES) {
            *t = (*t - dt / time).max(0.0);
        }
        if d <= f32::EPSILON {
            return w;
        }
        let yaw = w.dot(up);
        w - up * yaw * (1.0 - (1.0 - d).powf(dt * 60.0))
    }
}

/// The game's low-speed turn-around assist (`FUN_008f8360`): after the body has hit a wall nearly head-on, full
/// throttle and steering away from the wall (or any steering below 5 mph) slide the car sideways and yaw it
/// towards the steering, so it can turn away from the wall. All constants are `.rdata` (0x01021ee0..0x010220ac).
#[derive(Debug, Default, Clone, Copy)]
struct TurnAssist {
    /// Strength 0..1: grows at 1/s while the conditions hold, is held 0.5 s after, then fades at 2/s (+0x3020).
    amount: f32,
    hold: f32,
    /// The steering side the strength belongs to; it restarts from 0 when the steering changes side (+0x3028).
    side: f32,
}

struct TurnAssistInput {
    /// Steering input (positive = left), throttle, signed speed in mph.
    steer: f32,
    throttle: f32,
    mph: f32,
    /// Seconds since the body last touched the world, and that contact's normal (pointing at the car) if it is
    /// touching now.
    wall_time: f32,
    wall: Option<Vec3>,
    /// The body's forward and side (+X, left) axes, the velocity along the side axis and the yaw rate.
    forward: Vec3,
    side: Vec3,
    v_side: f32,
    yaw_rate: f32,
}

/// Recent wall contact (s), slow speed (mph), throttle / steering thresholds, hold time (s).
const ASSIST_WALL_TIME: f32 = 0.4;
const ASSIST_SLOW_MPH: f32 = 5.0;
const ASSIST_PEDAL: f32 = 0.5;
const ASSIST_HOLD: f32 = 0.5;
/// Sideways: target speed (m/s) and acceleration cap (m/s²). Yaw: target rate 1 − 0.5 h (rad/s) and
/// acceleration cap 20 − 10 h (rad/s²), h = how head-on the hit was.
const ASSIST_SIDE_SPEED: f32 = 0.5;
const ASSIST_SIDE_ACCEL: f32 = 30.0;

impl TurnAssist {
    /// Returns the change of the sideways velocity (m/s along +X) and of the yaw rate (rad/s) this tick.
    fn step(&mut self, i: &TurnAssistInput, dt: f32) -> (f32, f32) {
        if self.amount.abs() <= f32::EPSILON && i.wall_time >= ASSIST_WALL_TIME {
            return (0.0, 0.0);
        }
        let sign = |x: f32| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        };
        let side = sign(i.steer);
        let mut amount = if side == self.side { self.amount } else { 0.0 };
        let hold = if side == self.side { self.hold } else { 0.0 };
        // How head-on the wall is, and whether the steering points away from it (or the car is nearly stopped).
        let lo = 0.5 - 0.4 * (i.mph / ASSIST_SLOW_MPH).clamp(0.0, 1.0);
        let (head_on, away, h) = match i.wall {
            Some(n) => {
                let head = (-n).dot(i.forward).clamp(0.0, 1.0);
                let away = sign(n.dot(i.side)) != -side || i.mph < ASSIST_SLOW_MPH;
                (head > 0.0, away, ((head - lo) / (0.8 - lo)).clamp(0.0, 1.0))
            }
            None => (false, true, 0.0),
        };
        let active = i.throttle > ASSIST_PEDAL
            && i.wall_time < ASSIST_WALL_TIME
            && i.steer.abs() > ASSIST_PEDAL
            && head_on
            && away;
        self.hold = if active { ASSIST_HOLD } else { (hold - dt).max(0.0) };
        if active {
            amount += dt;
        } else if self.hold <= 0.0 {
            amount -= dt / ASSIST_HOLD;
        }
        self.amount = amount.clamp(0.0, 1.0);
        self.side = side;
        let k = i.steer.abs() * self.amount;
        if k <= f32::EPSILON {
            return (0.0, 0.0);
        }
        let dir = if i.steer >= 0.0 { 1.0 } else { -1.0 };
        let dv = ((ASSIST_SIDE_SPEED * dir - i.v_side) * dir).clamp(0.0, ASSIST_SIDE_ACCEL * dt * k) * dir;
        let dw = (((1.0 - 0.5 * h) * dir - i.yaw_rate) * dir).clamp(0.0, (20.0 - 10.0 * h) * dt * k) * dir;
        (dv, dw)
    }
}

/// `MW_ENGINE_TRACE=<file>`: append `seconds rpm throttle gear speed engine_throttle clutch gear_change_elapsed
/// shifts_started last_shift_up cruising top_gears` every physics tick of the player's car, for `mwtool
/// engine-sound --trace` (the columns after the speed are what the engine sound reads from the drivetrain).
fn trace_engine(t: f32, car: &Car, throttle: f32, speed: f32) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("MW_ENGINE_TRACE")?;
        std::fs::File::create(path).ok().map(std::sync::Mutex::new)
    });
    if let Some(f) = file {
        let d = &car.drivetrain;
        _ = writeln!(
            f.lock().unwrap(),
            "{t:.4} {:.1} {throttle:.3} {} {speed:.3} {:.4} {:.4} {:.4} {} {} {} {}",
            car.rpm,
            car.gear,
            d.throttle,
            d.clutch,
            d.gear_change_elapsed().min(1000.0),
            d.shifts_started,
            u8::from(d.last_shift_up),
            u8::from(d.cruising),
            u8::from(d.in_top_gears(&car.params.drivetrain)),
        );
    }
}

/// `MW_WHEEL_TRACE=<file>`: the player's wheel point masses every tick: time, speed (m/s), then per wheel the
/// compression (m), its rate (m/s), the wheel's velocity along the car's up axis after the sub-steps and after the
/// contact passes (m/s), the ground's normal impulse
/// this tick (N s) and flags (t = contact activated, s = strut stop set, g = tyre acting).
fn trace_wheels(t: f32, car: &Car) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("MW_WHEEL_TRACE")?;
        std::fs::File::create(path).ok().map(std::sync::Mutex::new)
    });
    if let Some(f) = file {
        let wheels: Vec<String> = car
            .wheels
            .iter()
            .map(|w| {
                format!(
                    "{:.4} {:.3} {:.3} {:.3} {:.1} {}{}{}",
                    w.compression,
                    w.compression_rate,
                    w.vel_substeps,
                    w.vel,
                    w.normal_impulse,
                    if w.touching { "t" } else { "-" },
                    if w.stop_share.is_some() { "s" } else { "-" },
                    if w.contact.is_some() { "g" } else { "-" },
                )
            })
            .collect();
        _ = writeln!(f.lock().unwrap(), "{t:.4} {:.3} | {}", car.speed, wheels.join(" | "));
    }
}

/// `MW_TYRE_DEBUG=1`: log each tyre's slip angle and forces twice a second.
fn tyre_debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_TYRE_DEBUG").is_some())
}

fn wheel_visuals(cars: Query<&Car>, mut transforms: Query<&mut Transform>) {
    for car in &cars {
        for w in &car.wheels {
            if let Ok(mut t) = transforms.get_mut(w.pivot) {
                t.translation = w.centre + Vec3::Y * (w.compression - w.sag);
                t.rotation = if w.front { Quat::from_rotation_y(car.steer) } else { Quat::IDENTITY };
            }
            if let Ok(mut t) = transforms.get_mut(w.spinner) {
                t.rotation = Quat::from_rotation_x(w.spin);
            }
        }
    }
}

/// Once a second: position and speed, for automated runs.
fn log_car(time: Res<Time>, mut next: Local<f32>, cars: Query<(&Car, &Transform), With<Player>>) {
    let Ok((car, t)) = cars.single() else { return };
    if time.elapsed_secs() < *next {
        return;
    }
    let interval = std::env::var("MW_CAR_LOG_INTERVAL").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    *next = time.elapsed_secs() + interval;
    info!(
        "car at {:.1} {:.3} {:.1}, {:.0} km/h, gear {}{} at {:.0} rpm, pitch {:.2}°, forces {:?}, compression {:?}",
        t.translation.x,
        t.translation.y,
        t.translation.z,
        car.speed * 3.6,
        car.gear,
        if car.drivetrain.cruising { " (cruising box)" } else { "" },
        car.rpm,
        (t.rotation * Vec3::Z).y.asin().to_degrees(),
        car.debug_forces.map(|f| f.round()),
        car.wheels.iter().map(|w| (w.compression * 1000.0).round() / 1000.0).collect::<Vec<_>>()
    );
}

/// A scripted handling test (`--test`), run on the flat `--test-track`.
#[derive(Resource)]
pub struct DriveTest {
    pub kind: TestKind,
    /// Physics ticks since the car spawned (the test clock).
    ticks: u32,
    done: bool,
    distance: f32,
    marks: Vec<(String, f32, f32)>,
    top_speed: f32,
    plateau: (f32, f32),
    braking_from: Option<(f32, f32)>,
    /// Corner test: smoothed (lateral accel, body slip), and the best seen as
    /// (lateral accel, body slip, steering input, speed).
    corner_smooth: [f32; 2],
    corner_best: [f32; 4],
    /// Drift test: time spent in the drift state.
    drift_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// Full throttle from rest: 0–60 mph, 0–100/200 km/h, quarter mile, top speed.
    Accel,
    /// `Accel` with nitrous held from the start: the same marks plus the tank every 0.5 s and when it runs dry.
    Nitrous,
    /// Accelerate to 100 km/h, then full brakes: stopping distance and time.
    Brake,
    /// At the corner-test speed: half steering plus a handbrake tap, then hold
    /// the steering on part throttle. Peak body slip and yaw rate.
    Handbrake,
    /// Hold a speed and slowly wind in steering: best lateral g, the input it
    /// needs, body slip angle.
    Corner,
    /// The game's brake-tap drift: at the drift-test speed, steer and tap the
    /// brake, then hold the steering on part throttle. How long the car stays
    /// beyond `DRIFT_HELD_DEG` of body slip, the mean angle and the speed lost.
    Drift,
    /// From rest, full throttle on full lock: a donut. Yaw rate, speed and
    /// circle radius over the last seconds.
    Donut,
    /// A human-like pedal sequence for engine-sound checks (record it with
    /// `MW_ENGINE_TRACE`): full throttle, lift and coast, throttle taps,
    /// braking, full throttle again.
    Engine,
    /// Hold 130 km/h into a kicker ramp that ends in a 3 m drop: the landing (sink, wheel overtravel, body
    /// contacts, settling time).
    Landing,
    /// Hold 120 km/h into a 30° incline (an abrupt kink from the flat): same measures as the landing.
    Climb,
    /// At rest, then at a steady 100 km/h: peak-to-peak and dominant frequency of the body's height, pitch,
    /// roll and their rates.
    Jitter,
    /// At a steady speed (`MW_TEST_KMH`, 50 km/h) over a kerb: a slab `MW_TEST_KERB_H` high (0.168 m, the downtown
    /// kerbs' faces, `mwtool surfaces --near 800,1688 --steps`) and `MW_TEST_KERB_DEPTH` deep (3 m) with vertical
    /// faces, its edge turned `MW_TEST_KERB_ANGLE` degrees (30) from square to the car's path: peak vertical
    /// speed, pitch / roll rates and swings, time with a wheel off the ground, strut-stop impulses.
    /// `MW_TEST_KERB_UP` (lower than the slab) makes the near face that high, the road before it raised by the
    /// difference: an island the car climbs onto and drops off (`ref/live1` at 66.4 s: 0.122 m up, 0.153 m down).
    Kerb,
    /// Exhaust pops and backfires (`exhaust.rs`): full throttle through the gears, a lift at high revs, nitrous;
    /// counts the backfires and pops.
    Exhaust,
}

/// Target speed for `--test corner` (km/h) and how long the steering takes to wind in.
const CORNER_TEST_KMH: f32 = 80.0;
const CORNER_RAMP_SECS: f32 = 15.0;
const HANDBRAKE_TAP_SECS: f32 = 0.4;
const DRIFT_TEST_KMH: f32 = 110.0;
/// `--test drift` plays the brake-tap entry the original made at `ref/live1` 98.98 s (MP4-12C): full lock one way
/// off the throttle, centred, then full brake with full lock the other way, released onto the throttle with the lock
/// held. Phase ends (s from the start of the flick). A plain brake tap while turning in from straight does not start
/// a drift in the original either (166.42 s): its brake-tap entries all follow yaw already built.
const DRIFT_FLICK_SECS: [f32; 3] = [0.2, 0.3, 0.75];
const DRIFT_HOLD_SECS: f32 = 6.0;
const DONUT_SECS: f32 = 8.0;
/// Body slip beyond which the drift test counts the car as drifting.
const DRIFT_HELD_DEG: f32 = 10.0;

/// Overrides for the drift test's script (`MW_TEST_STEER`, `MW_TEST_THROTTLE`,
/// `MW_TEST_KMH`, `MW_TEST_HOLD`, `MW_TEST_RELEASE`), for calibration sweeps.
fn test_env(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl DriveTest {
    pub fn new(kind: TestKind) -> Self {
        Self {
            kind,
            ticks: 0,
            done: false,
            distance: 0.0,
            marks: Vec::new(),
            top_speed: 0.0,
            plateau: (0.0, 0.0),
            braking_from: None,
            corner_smooth: [0.0; 2],
            corner_best: [0.0; 4],
            drift_time: 0.0,
        }
    }
}

fn run_test(
    time: Res<Time>,
    test: Option<ResMut<DriveTest>>,
    mut input: ResMut<CarInput>,
    mut cars: Query<&mut Car, With<Player>>,
    exhaust: Query<&crate::exhaust::Exhaust>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut test), Ok(mut car)) = (test, cars.single_mut()) else { return };
    if test.done {
        return;
    }
    // The clock counts physics ticks from the car's first one, so the script
    // does not depend on frame timing.
    test.ticks += 1;
    let now = test.ticks as f32 * time.delta_secs();
    // Let the suspension settle before starting.
    let t0 = 1.5;
    if now < t0 {
        input.scripted = Some([0.0; 4]);
        // The nitrous tests start with a full tank (the game's tank starts empty, `FUN_00658c90`).
        if matches!(test.kind, TestKind::Nitrous | TestKind::Exhaust) {
            car.nitrous = mw_handling::nitrous::Nitrous::full();
        }
        return;
    }
    let t = now - t0;
    let v = car.speed;
    test.distance += v.abs() * time.delta_secs();
    test.top_speed = test.top_speed.max(v);
    let mark = |test: &mut DriveTest, name: &str, cond: bool| {
        if cond && !test.marks.iter().any(|(n, _, _)| n == name) {
            info!("TEST {name}: {t:.2} s at {:.1} km/h, {:.0} m", v * 3.6, test.distance);
            test.marks.push((name.to_string(), t, v));
        }
    };
    let finish = |test: &mut DriveTest, exit: &mut MessageWriter<AppExit>| {
        test.done = true;
        info!(
            "TEST RESULT {:?}: {}",
            test.kind,
            test.marks.iter().map(|(n, t, _)| format!("{n}={t:.2}s")).collect::<Vec<_>>().join(" ")
        );
        exit.write(AppExit::Success);
    };
    match test.kind {
        TestKind::Accel | TestKind::Nitrous => {
            input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
            if test.kind == TestKind::Nitrous {
                // A press starts the burn only from `Restrictions.d5b487c7` (10 mph) on: press once past it.
                input.nitrous = v / MPH >= car.nitrous_params.rules.min_mph;
                let dt = time.delta_secs();
                if (t / 0.5).floor() != ((t - dt) / 0.5).floor() {
                    info!(
                        "TEST nitrous tank {:.0}% at {t:.1} s, {:.1} km/h, gear {}",
                        car.nitrous.fill() * 100.0,
                        v * 3.6,
                        car.gear
                    );
                }
                mark(&mut test, "tank-empty", car.nitrous.tank <= 0.0);
            }
            let quarter = test.distance >= 402.336;
            mark(&mut test, "0-60mph", v >= 26.822);
            mark(&mut test, "0-100kmh", v >= 27.778);
            mark(&mut test, "0-200kmh", v >= 55.556);
            mark(&mut test, "quarter-mile", quarter);
            // Top speed: stop once speed has not risen 0.2 km/h in 5 s.
            if t - test.plateau.0 > 5.0 {
                if v - test.plateau.1 < 0.2 / 3.6 && t > 15.0 || t > 180.0 {
                    info!(
                        "TEST top speed: {:.1} km/h ({:.1} mph) after {t:.0} s",
                        test.top_speed * 3.6,
                        test.top_speed / 0.44704
                    );
                    let top = format!("top={:.1}kmh", test.top_speed * 3.6);
                    test.marks.push((top, t, v));
                    finish(&mut test, &mut exit);
                }
                test.plateau = (t, v);
            }
        }
        TestKind::Corner => {
            // Hold the speed and wind the steering in slowly (quasi steady
            // state); report the best lateral acceleration, smoothed over 0.5 s.
            let target = CORNER_TEST_KMH / 3.6;
            let throttle = ((target - v) * 0.5).clamp(0.0, 1.0);
            let brake = ((v - target - 1.0) * 0.5).clamp(0.0, 1.0);
            if test.braking_from.is_none() && v >= target - 0.5 {
                test.braking_from = Some((t, 0.0));
            }
            let Some((ts, _)) = test.braking_from else {
                input.scripted = Some([throttle, brake, 0.0, 0.0]);
                return;
            };
            let ramp = ((t - ts) / CORNER_RAMP_SECS).clamp(0.0, 1.0);
            input.scripted = Some([throttle, brake, ramp, 0.0]);
            let k = 1.0 - (-time.delta_secs() / 0.5).exp();
            let [a, slip] = &mut test.corner_smooth;
            *a += (car.lateral_accel.abs() - *a) * k;
            *slip += (car.body_slip.abs() - *slip) * k;
            let (a, slip) = (*a, *slip);
            if a > test.corner_best[0] {
                test.corner_best = [a, slip, ramp, v];
            }
            if ramp >= 1.0 {
                let [a, slip, input_at, v_at] = test.corner_best;
                let g = a / 9.81;
                info!(
                    "TEST corner: best {g:.2} g at {:.0} km/h, steering input {input_at:.2}, radius {:.0} m, body slip {:.1}°, roll {:.1}°",
                    v_at * 3.6,
                    v_at * v_at / a.max(0.01),
                    slip.to_degrees(),
                    car.roll.to_degrees()
                );
                test.marks.push((format!("corner={g:.2}g@{input_at:.2}"), t, v));
                finish(&mut test, &mut exit);
            }
        }
        TestKind::Handbrake => {
            let target = CORNER_TEST_KMH / 3.6;
            if test.braking_from.is_none() && v >= target {
                test.braking_from = Some((t, 0.0));
            }
            let Some((ts, _)) = test.braking_from else {
                input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
                return;
            };
            let dt = t - ts;
            let pulled = if dt < HANDBRAKE_TAP_SECS { 1.0 } else { 0.0 };
            input.scripted = Some([if pulled > 0.0 { 0.0 } else { 0.3 }, 0.0, 0.5, pulled]);
            let best = &mut test.corner_best;
            best[0] = best[0].max(car.body_slip.abs());
            best[1] = best[1].max(car.yaw_rate.abs());
            if [1.0, 2.0, 3.0, 5.0].iter().any(|&m| dt >= m && dt - time.delta_secs() < m) {
                info!(
                    "TEST handbrake +{dt:.0} s: {:.0} km/h, body slip {:.1}°, yaw {:.0}°/s",
                    v * 3.6,
                    car.body_slip.to_degrees(),
                    car.yaw_rate.to_degrees()
                );
            }
            if dt >= 5.0 {
                let mark = format!("slip={:.1}deg,yaw={:.0}deg/s", best[0].to_degrees(), best[1].to_degrees());
                test.marks.push((mark, t, v));
                finish(&mut test, &mut exit);
            }
        }
        TestKind::Drift => {
            let target = test_env("MW_TEST_KMH", DRIFT_TEST_KMH) / 3.6;
            if test.braking_from.is_none() && v >= target {
                test.braking_from = Some((t, v));
                if let Some(d) = &car.params.drift {
                    info!(
                        "TEST drift entry: rear slip above {:.1}° ({:.1}° after a brake tap, recovering at {:.1}°/s; \
                         {:.1}° on full brake), faster than {:.0} mph",
                        d.enter_slip, d.enter_slip_tap, d.enter_recovery, d.enter_slip_brake, d.enter_min_speed
                    );
                }
            }
            let Some((ts, v0)) = test.braking_from else {
                input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
                return;
            };
            let dt = t - ts;
            let (k_steer, k_throttle) = (test_env("MW_TEST_STEER", 1.0), test_env("MW_TEST_THROTTLE", 1.0));
            let [flick, centre, braked] = DRIFT_FLICK_SECS;
            let (throttle, brake, steer) = if dt < flick {
                (0.0, 0.0, -k_steer)
            } else if dt < centre {
                (0.0, 0.0, 0.0)
            } else if dt < braked {
                (0.0, 1.0, k_steer)
            } else {
                // Optionally let go of the steering after a while (exit check).
                (k_throttle, 0.0, if dt > test_env("MW_TEST_RELEASE", f32::INFINITY) { 0.0 } else { k_steer })
            };
            input.scripted = Some([throttle, brake, steer, 0.0]);
            let slip = car.body_slip.abs().to_degrees();
            // corner_best: [peak slip, time beyond the threshold, Σ slip while beyond, yaw peak]
            let step = time.delta_secs();
            let best = &mut test.corner_best;
            best[0] = best[0].max(slip);
            if slip > DRIFT_HELD_DEG {
                best[1] += step;
                best[2] += slip * step;
            }
            best[3] = best[3].max(car.yaw_rate.abs().to_degrees());
            if car.drift.active() {
                test.drift_time += step;
            }
            if (dt * 4.0).floor() != ((dt - step) * 4.0).floor() {
                info!(
                    "TEST drift +{dt:.1} s: {:.0} km/h, body slip {:.1}°, yaw {:.0}°/s, {:?}/{:?} (scale {:.2}, rear slip {:.1}°, wheel {:.0}°, torque {:.0} N m, side force {:.0} N, hold {:.0} N, tyre yaw {:.0}/{:.0})",
                    v * 3.6,
                    car.body_slip.to_degrees(),
                    car.yaw_rate.to_degrees(),
                    car.drift.phase,
                    car.drift.exit_reason,
                    car.drift.scale,
                    car.rear_slip,
                    car.steer.to_degrees(),
                    car.drift.last.yaw_torque,
                    car.drift.last.side_force,
                    car.drift.last.hold_force,
                    car.tyre_yaw[0],
                    car.tyre_yaw[1]
                );
            }
            if dt >= test_env("MW_TEST_HOLD", DRIFT_HOLD_SECS) || v < 3.0 {
                let [peak, held, sum, yaw] = test.corner_best;
                let mean = if held > 0.0 { sum / held } else { 0.0 };
                info!(
                    "TEST drift: peak {peak:.1}°, beyond {DRIFT_HELD_DEG}° for {held:.2} s (mean {mean:.1}°), peak yaw {yaw:.0}°/s, {:.0} -> {:.0} km/h",
                    v0 * 3.6,
                    v * 3.6
                );
                let drift = test.drift_time;
                let mark = format!(
                    "peak={peak:.1}deg,held={held:.2}s,mean={mean:.1}deg,drift={drift:.2}s,exit={:.0}kmh",
                    v * 3.6
                );
                test.marks.push((mark, t, v));
                finish(&mut test, &mut exit);
            }
        }
        TestKind::Donut => {
            // Full lock and throttle from rest; `MW_TEST_HANDBRAKE` (seconds) also pulls the handbrake at the
            // start to kick the tail out.
            let pull = t < test_env("MW_TEST_HANDBRAKE", 0.0);
            input.scripted = Some([1.0, 0.0, 1.0, if pull { 1.0 } else { 0.0 }]);
            let step = time.delta_secs();
            // corner_best: [Σ yaw rate, Σ speed, seconds averaged, peak body slip]
            let best = &mut test.corner_best;
            if t >= DONUT_SECS - 3.0 {
                best[0] += car.yaw_rate * step;
                best[1] += v.abs() * step;
                best[2] += step;
            }
            best[3] = best[3].max(car.body_slip.abs().to_degrees());
            if (t * 2.0).floor() != ((t - step) * 2.0).floor() {
                info!(
                    "TEST donut +{t:.1} s: {:.0} km/h, body slip {:.1}°, rear slip {:.1}°, yaw {:.0}°/s, {:?}, gear {}, rpm {:.0}",
                    v * 3.6,
                    car.body_slip.to_degrees(),
                    car.rear_slip,
                    car.yaw_rate.to_degrees(),
                    car.drift.phase,
                    car.gear,
                    car.rpm
                );
            }
            if t >= DONUT_SECS {
                let [yaw, speed, n, slip] = test.corner_best;
                let (yaw, speed) = (yaw / n.max(1e-3), speed / n.max(1e-3));
                let radius = speed / yaw.abs().max(1e-3);
                info!(
                    "TEST donut: yaw {:.0}°/s at {:.0} km/h, radius {radius:.1} m, peak body slip {slip:.0}°",
                    yaw.to_degrees(),
                    speed * 3.6
                );
                let mark = format!("yaw={:.0}deg/s,speed={:.0}kmh,radius={radius:.1}m", yaw.to_degrees(), speed * 3.6);
                test.marks.push((mark, t, v));
                finish(&mut test, &mut exit);
            }
        }
        TestKind::Engine => {
            // (until seconds, throttle, brake); taps alternate every 0.3 s.
            const SCRIPT: [(f32, f32, f32); 7] = [
                (7.0, 1.0, 0.0),
                (10.0, 0.0, 0.0),
                (13.0, -1.0, 0.0),
                (15.0, 0.4, 0.0),
                (17.0, 0.0, 0.8),
                (21.0, 1.0, 0.0),
                (24.0, 0.0, 0.0),
            ];
            let Some(&(_, throttle, brake)) = SCRIPT.iter().find(|s| t < s.0) else {
                info!("TEST engine: {:.0} km/h, gear {}, {:.0} rpm", v * 3.6, car.gear, car.rpm);
                test.marks.push(("engine".into(), t, v));
                finish(&mut test, &mut exit);
                return;
            };
            let throttle = if throttle < 0.0 { if (t / 0.3) as i32 % 2 == 0 { 1.0 } else { 0.0 } } else { throttle };
            input.scripted = Some([throttle, brake, 0.0, 0.0]);
        }
        TestKind::Exhaust => {
            // (until seconds, throttle, nitrous)
            const SCRIPT: [(f32, f32, bool); 5] =
                [(9.0, 1.0, false), (11.0, 0.0, false), (13.0, 1.0, false), (16.0, 1.0, true), (18.0, 0.0, false)];
            let (backfires, pops) = exhaust.iter().next().map_or((0, 0), |e| (e.backfires, e.pop_count));
            mark(&mut test, "first-backfire", backfires > 0);
            mark(&mut test, "first-pop", pops > 0);
            let Some(&(_, throttle, nitrous)) = SCRIPT.iter().find(|s| t < s.0) else {
                info!("TEST exhaust: {backfires} backfires, {pops} pops");
                test.marks.push((format!("backfires-{backfires}-pops-{pops}"), t, v));
                finish(&mut test, &mut exit);
                return;
            };
            input.nitrous = nitrous;
            input.scripted = Some([throttle, 0.0, 0.0, 0.0]);
        }
        TestKind::Landing | TestKind::Climb | TestKind::Jitter | TestKind::Kerb => {}
        TestKind::Brake => match test.braking_from {
            None => {
                input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
                if v >= 27.778 {
                    test.braking_from = Some((t, test.distance));
                }
            }
            Some((tb, db)) => {
                input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
                if v < 0.3 {
                    let dist = test.distance - db;
                    info!("TEST 100-0 km/h: {:.2} s, {dist:.1} m", t - tb);
                    test.marks.push((format!("100-0kmh({dist:.1}m)"), t - tb, dist));
                    finish(&mut test, &mut exit);
                }
            }
        },
    }
}

/// Obstacles on the test track for `--test landing` and `--test climb`: a profile (z, height) along +Z, 200 m
/// wide, built as a trimesh like the world's collision. Landing: a 5.7° kicker up to a 3 m plateau that ends in
/// a sheer drop at z = 460. Climb: flat into a 30° incline at z = 400 (an abrupt kink), up to a plateau.
pub fn test_obstacle(kind: TestKind) -> Option<(Vec<Vec3>, Vec<[u32; 3]>)> {
    let profile: &[(f32, f32)] = match kind {
        TestKind::Landing => &[(LANDING_RAMP_Z, 0.0), (430.0, 3.0), (LANDING_DROP_Z, 3.0), (LANDING_DROP_Z, 0.0)],
        TestKind::Climb => {
            let rise = CLIMB_LENGTH * test_env("MW_TEST_SLOPE", CLIMB_SLOPE_DEG).to_radians().tan();
            &*Box::leak(Box::new([
                (CLIMB_Z, 0.0),
                (CLIMB_Z + CLIMB_LENGTH, rise),
                (CLIMB_Z + CLIMB_LENGTH + 200.0, rise),
            ]))
        }
        TestKind::Kerb => {
            let (h, depth) = (test_env("MW_TEST_KERB_H", KERB_HEIGHT), test_env("MW_TEST_KERB_DEPTH", 3.0));
            // `MW_TEST_KERB_UP` lower than the slab: the near face is that high, on a plateau of the difference
            // that starts 100 m before it (an island between roads of two levels).
            let base = (h - test_env("MW_TEST_KERB_UP", h)).max(0.0);
            let mut profile = vec![(KERB_Z, base), (KERB_Z, h), (KERB_Z + depth, h), (KERB_Z + depth, 0.0)];
            if base > 0.0 {
                profile.splice(0..0, [(KERB_Z - 100.0, 0.0), (KERB_Z - 100.0, base)]);
            }
            &*Box::leak(profile.into_boxed_slice())
        }
        _ => return None,
    };
    // The kerb's edge is turned about the point where the car's path (x = 0) meets it.
    let turn = if kind == TestKind::Kerb { test_env("MW_TEST_KERB_ANGLE", 30.0).to_radians() } else { 0.0 };
    let place = |x: f32, y: f32, z: f32| {
        let (s, c) = turn.sin_cos();
        Vec3::new(x * c - (z - KERB_Z) * s, y, KERB_Z + x * s + (z - KERB_Z) * c)
    };
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (i, &(z, y)) in profile.iter().enumerate() {
        vertices.push(place(-100.0, y, z));
        vertices.push(place(100.0, y, z));
        if i > 0 {
            let [a, b, c, d] = [2 * i as u32 - 2, 2 * i as u32 - 1, 2 * i as u32, 2 * i as u32 + 1];
            indices.push([a, c, b]);
            indices.push([b, c, d]);
        }
    }
    Some((vertices, indices))
}
const LANDING_RAMP_Z: f32 = 400.0;
const LANDING_DROP_Z: f32 = 460.0;
const CLIMB_Z: f32 = 400.0;
const CLIMB_LENGTH: f32 = 40.0;
const CLIMB_SLOPE_DEG: f32 = 30.0;
const KERB_Z: f32 = 400.0;
/// The downtown kerbs' face height (m): `mwtool surfaces --near 800,1688 --steps` (0.15–0.18 m, mostly 0.168).
const KERB_HEIGHT: f32 = 0.168;

/// State of `--test landing / climb / jitter` (`ground_test`).
#[derive(Default)]
pub(crate) struct GroundTest {
    /// Body origin's height above the ground along the body's down axis at rest (m).
    rest: Option<f32>,
    deepest: f32,
    body_contact_ticks: u32,
    max_overtravel: f32,
    min_wheels: usize,
    event_t: Option<f32>,
    settled_since: Option<f32>,
    settle_time: Option<f32>,
    slow_since: Option<f32>,
    trace: Option<std::fs::File>,
    /// Kerb test: peak |vertical speed| (m/s), |pitch rate|, |roll rate| (°/s); pitch and roll (min, max, °); time
    /// with at least one / all wheels off the ground (s); the strut stops' total impulse (N s).
    kerb_peaks: [f32; 3],
    kerb_pitch: (f32, f32),
    kerb_roll: (f32, f32),
    kerb_air: [f32; 2],
    kerb_stop: f32,
    kerb_rise: f32,
    /// Jitter samples: (y, pitch, roll, vy, pitch rate, roll rate); phase 0 = at rest, 1 = at 100 km/h.
    samples: Vec<[f32; 6]>,
    phase: u32,
    phase_t: f32,
}

/// Peak-to-peak and the dominant frequency (Hz, DFT over 0.25 Hz .. Nyquist, mean removed) of a series.
fn spectrum_peak(x: &[f32], rate: f32) -> (f32, f32) {
    let n = x.len().max(1);
    let mean = x.iter().sum::<f32>() / n as f32;
    let (lo, hi) = x.iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    let mut best = (0.0, 0.0);
    for k in 1..=n / 2 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &v) in x.iter().enumerate() {
            let ph = std::f32::consts::TAU * (k * i) as f32 / n as f32;
            re += (v - mean) * ph.cos();
            im += (v - mean) * ph.sin();
        }
        let p = re * re + im * im;
        if p > best.0 {
            best = (p, k as f32 * rate / n as f32);
        }
    }
    (hi - lo, best.1)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn ground_test(
    time: Res<Time>,
    test: Option<ResMut<DriveTest>>,
    mut input: ResMut<CarInput>,
    spatial: SpatialQuery,
    cars: Query<(Entity, &Car, &Transform, &LinearVelocity, &AngularVelocity), With<Player>>,
    mut state: Local<GroundTest>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut test), Ok((entity, car, tf, vel, ang))) = (test, cars.single()) else { return };
    if test.done || !matches!(test.kind, TestKind::Landing | TestKind::Climb | TestKind::Jitter | TestKind::Kerb) {
        return;
    }
    let dt = time.delta_secs();
    let now = test.ticks as f32 * dt;
    let rot = tf.rotation;
    let up = rot * Vec3::Y;
    let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider])
        .with_mask(crate::props::GameLayer::World);
    // From 2 m above the origin, so the cast starts above the ground even when the body is in it.
    let height = Dir3::new(-up)
        .ok()
        .and_then(|d| spatial.cast_ray(tf.translation + up * 2.0, d, 50.0, true, &filter))
        .map_or(f32::NAN, |h| h.distance - 2.0);
    let (pitch, roll, _) = {
        let (yaw, pitch, roll) = rot.to_euler(EulerRot::YXZ);
        (pitch, roll, yaw)
    };
    if now < 1.5 {
        if now > 1.2 {
            state.rest = Some(height);
            state.min_wheels = 4;
        }
        return;
    }
    let rest = state.rest.unwrap_or(height);
    let t = now - 1.5;
    let v = car.speed;
    let sink = rest - height;
    let wheels_down = car.wheels.iter().filter(|w| w.contact.is_some()).count();
    if test.kind == TestKind::Jitter {
        // 4 s at rest, then 100 km/h held; sample 4 s once the speed has been steady for 3 s.
        let target = if state.phase == 0 && t < 4.0 { 0.0 } else { 100.0 / 3.6 };
        let throttle = if target > 0.0 { ((target - v) * 0.5).clamp(0.0, 1.0) } else { 0.0 };
        input.scripted = Some([throttle, 0.0, 0.0, 0.0]);
        let local_w = rot.inverse() * ang.0;
        let sample = [tf.translation.y, pitch, roll, vel.y, local_w.x, local_w.z];
        let steady = target == 0.0 || (v - target).abs() < 0.5 && t - state.phase_t > 3.0;
        if target > 0.0 && state.phase == 0 {
            state.phase = 1;
            state.phase_t = t;
            report_jitter("rest", &state.samples, 1.0 / dt);
            state.samples.clear();
        }
        if steady {
            state.samples.push(sample);
        }
        if state.phase == 1 && state.samples.len() as f32 * dt >= 4.0 {
            let m = report_jitter("100kmh", &state.samples, 1.0 / dt);
            test.marks.push((m, t, v));
            test.done = true;
            info!(
                "TEST RESULT {:?}: {}",
                test.kind,
                test.marks.iter().map(|m| m.0.clone()).collect::<Vec<_>>().join(" ")
            );
            exit.write(AppExit::Success);
        }
        if state.phase == 0 && target == 0.0 && t >= 0.0 {
            // Rest samples are taken above.
        }
        return;
    }
    let (target_kmh, event_z, end_z) = match test.kind {
        TestKind::Landing => (test_env("MW_TEST_KMH", 130.0), LANDING_RAMP_Z, LANDING_DROP_Z + 150.0),
        TestKind::Kerb => (test_env("MW_TEST_KMH", 50.0), KERB_Z, KERB_Z + test_env("MW_TEST_KERB_DEPTH", 3.0) + 30.0),
        _ => (test_env("MW_TEST_KMH", 120.0), CLIMB_Z, CLIMB_Z + CLIMB_LENGTH + 120.0),
    };
    let target = target_kmh / 3.6;
    let z = tf.translation.z;
    // Hold the speed up to the obstacle, then full throttle through it (the player keeps the pedal down); over the
    // kerb the speed is held throughout.
    let hold = ((target - v) * 0.5).clamp(0.0, 1.0);
    let throttle = if z < event_z - 20.0 || test.kind == TestKind::Kerb { hold } else { 1.0 };
    input.scripted = Some([throttle, 0.0, 0.0, 0.0]);
    if state.trace.is_none()
        && let Some(path) = std::env::var_os("MW_SUSP_TRACE")
    {
        state.trace = std::fs::File::create(path).ok();
    }
    if z > event_z - 10.0 {
        state.event_t.get_or_insert(t);
        if let Some(f) = state.trace.as_mut() {
            use std::io::Write;
            let comp: Vec<String> = car
                .wheels
                .iter()
                .map(|w| {
                    format!(
                        "{:.3}/{:.3}+{:.3}{}",
                        w.compression,
                        w.travel,
                        w.overtravel,
                        if w.contact.is_some() { "" } else { "x" }
                    )
                })
                .collect();
            let local_w = rot.inverse() * ang.0;
            let _ = writeln!(
                f,
                "{t:.3} z {z:.2} y {:.3} sink {sink:.3} vy {:.2} v {:.1} pitch {:.2} roll {:.2} rates {:.1} {:.1} body {} stop {:.0} wheels {}",
                tf.translation.y,
                vel.y,
                v * 3.6,
                pitch.to_degrees(),
                roll.to_degrees(),
                local_w.x.to_degrees(),
                local_w.z.to_degrees(),
                u8::from(car.body_contact),
                car.stop_impulse,
                comp.join(" ")
            );
        }
        if test.kind == TestKind::Kerb {
            let local_w = rot.inverse() * ang.0;
            let p = &mut state.kerb_peaks;
            *p =
                [p[0].max(vel.y.abs()), p[1].max(local_w.x.abs().to_degrees()), p[2].max(local_w.z.abs().to_degrees())];
            let (pd, rd) = (pitch.to_degrees(), roll.to_degrees());
            state.kerb_pitch = (state.kerb_pitch.0.min(pd), state.kerb_pitch.1.max(pd));
            state.kerb_roll = (state.kerb_roll.0.min(rd), state.kerb_roll.1.max(rd));
            if wheels_down < 4 {
                state.kerb_air[0] += dt;
            }
            if wheels_down == 0 {
                state.kerb_air[1] += dt;
            }
            state.kerb_stop += car.stop_impulse;
            state.kerb_rise = state.kerb_rise.max(-sink);
        }
        state.deepest = state.deepest.max(sink);
        state.body_contact_ticks += u32::from(car.body_contact);
        state.max_overtravel = state.max_overtravel.max(car.wheels.iter().map(|w| w.overtravel).fold(0.0, f32::max));
        state.min_wheels = state.min_wheels.min(wheels_down);
        // Settled: within 2 cm of the rest height, all wheels down, |vertical velocity| < 0.1 m/s, for 0.5 s.
        let calm = sink.abs() < 0.02 && wheels_down == 4 && vel.y.abs() < 0.1;
        let past = match test.kind {
            TestKind::Landing => z > LANDING_DROP_Z + 5.0,
            _ => z > CLIMB_Z,
        };
        if past && calm {
            let since = *state.settled_since.get_or_insert(t);
            if t - since > 0.5 && state.settle_time.is_none() {
                state.settle_time = Some(since - state.event_t.unwrap_or(t));
            }
        } else {
            state.settled_since = None;
            if !calm {
                state.settle_time = None;
            }
        }
        state.slow_since = if v < 2.0 { state.slow_since.or(Some(t)) } else { None };
    }
    let stuck = state.slow_since.is_some_and(|s| t - s > 2.0);
    if test.kind == TestKind::Kerb && (z > end_z || stuck || t > 60.0) {
        let m = format!(
            "kmh={target_kmh:.0} vy_peak={:.2}m/s pitch_rate={:.1}deg/s roll_rate={:.1}deg/s pitch={:.2}..{:.2}deg roll={:.2}..{:.2}deg rise={:.3}m wheel_off={:.3}s all_off={:.3}s stop={:.0}Ns overtravel={:.3}m body_contact={:.2}s{}",
            state.kerb_peaks[0],
            state.kerb_peaks[1],
            state.kerb_peaks[2],
            state.kerb_pitch.0,
            state.kerb_pitch.1,
            state.kerb_roll.0,
            state.kerb_roll.1,
            state.kerb_rise,
            state.kerb_air[0],
            state.kerb_air[1],
            state.kerb_stop,
            state.max_overtravel,
            state.body_contact_ticks as f32 * dt,
            if stuck { " STUCK" } else { "" }
        );
        test.marks.push((m, t, v));
        test.done = true;
        info!("TEST RESULT {:?}: {}", test.kind, test.marks.iter().map(|m| m.0.clone()).collect::<Vec<_>>().join(" "));
        exit.write(AppExit::Success);
        return;
    }
    if z > end_z || stuck || t > 60.0 {
        let below = sink > 0.02;
        let m = format!(
            "end={} sink={:.3}m deepest={:.3}m overtravel={:.3}m body_contact={:.2}s min_wheels={} settle={} v={:.0}kmh",
            if stuck {
                "STUCK"
            } else if below {
                "BELOW_REST"
            } else {
                "ok"
            },
            sink,
            state.deepest,
            state.max_overtravel,
            state.body_contact_ticks as f32 * dt,
            state.min_wheels,
            state.settle_time.map_or("never".into(), |s| format!("{s:.2}s")),
            v * 3.6
        );
        test.marks.push((m, t, v));
        test.done = true;
        info!("TEST RESULT {:?}: {}", test.kind, test.marks.iter().map(|m| m.0.clone()).collect::<Vec<_>>().join(" "));
        exit.write(AppExit::Success);
    }
}

fn report_jitter(label: &str, samples: &[[f32; 6]], rate: f32) -> String {
    let names = ["y", "pitch", "roll", "vy", "pitch_rate", "roll_rate"];
    let mut out = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let series: Vec<f32> = samples.iter().map(|s| s[i]).collect();
        let (pp, f) = spectrum_peak(&series, rate);
        let pp = if (1..3).contains(&i) || i >= 4 { pp.to_degrees() } else { pp };
        out.push(format!("{name}_pp={pp:.4}@{f:.1}Hz"));
    }
    let line = format!("{label}:{}", out.join(","));
    info!("TEST jitter {line} ({} samples)", samples.len());
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_yaw_damping_follows_the_timers() {
        let mut d = ImpactYawDamping::default();
        let (w, up, dt) = (Vec3::new(0.3, 1.0, 0.2), Vec3::Y, 1.0 / 60.0);
        // No collision: nothing changes.
        assert_eq!(d.step(w, up, dt), w);
        // A new world contact removes 5 % of the yaw rate on the next 60 Hz tick, the other axes untouched.
        d.hit(true, false, false);
        let w1 = d.step(w, up, dt);
        assert!((w1.y - 0.95).abs() < 1e-5 && w1.x == w.x && w1.z == w.z, "{w1}");
        // The world timer runs out after 1 s, the object timer after 0.3 s.
        for _ in 0..59 {
            d.step(w, up, dt);
        }
        assert_eq!(d.step(w, up, dt), w);
        d.hit(false, false, true);
        for _ in 0..18 {
            d.step(w, up, dt);
        }
        assert_eq!(d.step(w, up, dt), w);
    }
}
