//! Drivable cars: a raycast-suspension vehicle on avian3d, and input. The
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
use bevy::prelude::*;
use mw_assets::camera::VehicleCameras;
use mw_assets::handling::{LateralCurve, LongCurve, SlipBlend, Steering, SurfaceLink, TyreAxle, VehicleHandling};

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
    /// Scripted test in progress (`--test`): throttle, brake, steer, handbrake
    /// (> 0.5 = pulled). Overrides other input.
    pub scripted: Option<[f32; 4]>,
    /// The car is crashed (`crash.rs`): no control until the reset (`Player.CanDriveAway`).
    pub crashed: bool,
}

/// Speed below which the standstill hold acts (`.bss` 0x01186e60, not read; assumed 0 mph: the original's
/// launch in `ref/xkr_launch` pulls away at 6 m/s² from 0.03 m/s, so the hold is off there).
const STANDSTILL_HOLD_MPH: f32 = 0.0;

/// Physical parameters of a car, derived from the game data.
#[derive(Debug, Clone)]
pub struct CarParams {
    pub mass: f32,
    /// Standstill hold deceleration (`Brakes.99407bbc`, m/s²; `FUN_009b1180`).
    pub hold_decel: f32,
    pub inertia: Vec3,
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

/// Bump travel above the rest position (m): the game's strut on the 911 allows ~0.06 m (inferred from its
/// geometry; not read per car yet).
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
const MPH: f32 = 0.44704;
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
    air_time: f32,
    /// The handbrake's countdown (reset to `Minimum_handbrake_time` on every tick it is pulled) and whether it is
    /// on (`FUN_009d27b0`).
    handbrake_timer: f32,
    pub handbrake_on: bool,
    /// Seconds since the body last touched the world, and the low-speed turn-around assist's state
    /// (`FUN_008f8360`).
    wall_time: f32,
    turn_assist: TurnAssist,
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
    /// Current suspension compression (m), and its rate (m/s, positive = compressing).
    compression: f32,
    compression_rate: f32,
    /// How far the ground reached past full compression on the last tick (m): the wheel inside the ground.
    overtravel: f32,
    spin: f32,
    pivot: Entity,
    spinner: Entity,
    /// Ground contact in the last physics step (read by `particles.rs`).
    contact: Option<WheelContact>,
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
}

impl Car {
    /// Each wheel's (slip ratio, slip angle in degrees) from the last physics step, front wheels first in spec
    /// order.
    pub fn wheel_slips(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.wheels.iter().map(|w| w.slip)
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
) {
    let Some(pending) = pending else { return };
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
    let (modded, nitrous_burn) = catalogue.modded(&selection);
    info!("mods: {}", catalogue.describe(&selection));
    commands.insert_resource(catalogue);
    let params = CarParams::new(&modded);
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
            Name::new("player car"),
            Transform::from_translation(start).with_rotation(Quat::from_rotation_y(pending.heading)),
            Visibility::default(),
            RigidBody::Dynamic,
            LinearVelocity(pending.velocity),
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
            AngularDamping(0.4),
            LinearDamping(0.0),
            // Render between physics ticks (64 Hz): without this the car and
            // the chase camera judder on faster displays.
            TransformInterpolation,
            TransformHermiteEasing,
        ))
        .id();
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
    commands.spawn((
        Collider::cuboid(1.7, 0.35, 4.1),
        Transform::from_xyz(0.0, 0.375, 0.0),
        crate::props::GameLayer::car_bumper(),
        ChildOf(car),
    ));
    for m in &pending.spec.body {
        for (mesh, mat) in pending.parts.get(m).into_iter().flatten() {
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                crate::shadow_cull::ShadowClass::Car,
                ChildOf(car),
            ));
        }
    }

    // Static load per corner from the centre of mass between the axles.
    let h = &pending.spec.handling;
    let z_front = pending.spec.wheels.iter().filter(|w| w.front).map(|w| w.position.z).fold(f32::MIN, f32::max);
    let z_rear = pending.spec.wheels.iter().filter(|w| !w.front).map(|w| w.position.z).fold(f32::MAX, f32::min);
    let front_load = ((params.centre_of_mass.z - z_rear) / (z_front - z_rear).max(0.5)).clamp(0.2, 0.8);
    let mut wheels = Vec::new();
    for w in &pending.spec.wheels {
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
        // A cylinder of the wheel body's mass, plus the transmission inertia on driven wheels.
        let wheel_mass = h
            .wheels
            .iter()
            .min_by(|a, b| {
                Vec3::from(a.position).distance(w.position).total_cmp(&Vec3::from(b.position).distance(w.position))
            })
            .map_or(25.0, |b| b.mass)
            .max(5.0);
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
            for (mesh, mat) in pending.parts.get(&m).into_iter().flatten() {
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
            "wheel {:?} front={} radius {radius:.3} k {k:.0} c {c:.0}/{:.0} sag {sag:.3} corner {corner_mass:.0} kg",
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
            travel: BUMP_TRAVEL + sag,
            sag,
            brake_torque: brake,
            omega: pending.velocity.dot(Quat::from_rotation_y(pending.heading) * Vec3::Z) / radius,
            inertia,
            corner_mass,
            slip: (0.0, 0.0),
            compression: 0.0,
            compression_rate: 0.0,
            overtravel: 0.0,
            spin: 0.0,
            pivot,
            spinner,
            contact: None,
        });
    }
    // A car swapped in at speed starts in the gear that speed needs.
    let speed = pending.velocity.dot(Quat::from_rotation_y(pending.heading) * Vec3::Z);
    let radius = wheels.iter().map(|w| w.radius).sum::<f32>() / wheels.len().max(1) as f32;
    let wheel_rpm = speed.max(0.0) / radius.max(0.1) * 60.0 / std::f32::consts::TAU;
    let dp = &params.drivetrain;
    let gear = (1..=dp.gears as i32)
        .find(|&g| wheel_rpm * dp.ratio(g) * dp.final_drive < params.redline_rpm * 0.8)
        .unwrap_or(dp.gears.max(1) as i32);
    let drivetrain = Drivetrain::new(dp, gear);
    commands.entity(car).insert(Car {
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
        roll: 0.0,
        pitch: 0.0,
        body_contact: false,
        gear,
        rpm: params.idle_rpm,
        rpm_prev: params.idle_rpm,
        drivetrain,
        nitrous: Default::default(),
        nitrous_params: {
            let mut n = mw_handling::nitrous::NitrousParams::new(&modded);
            n.burn = nitrous_burn.unwrap_or(n.burn);
            n
        },
        brake_ramp: 0.0,
        debug_forces: [0.0; 3],
        tyre_yaw: [0.0; 2],
        spawn: start,
        params,
        body_collider,
    });
    if let Ok(mut cam) = camera.single_mut() {
        let behind = Quat::from_rotation_y(pending.heading) * Vec3::new(0.0, 2.5, -7.0);
        *cam = Transform::from_translation(start + behind).looking_at(start, Vec3::Y);
    }
    commands.remove_resource::<PendingCar>();
    info!("car spawned at {start:?}");
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
        input.nitrous = dev.held(act::NITROUS);
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
            RawGamepadEvent::Axis(a) if a.value.abs() > 0.05 => info!("gamepad axis {:?} = {:.2}", a.axis, a.value),
            RawGamepadEvent::Button(b) => info!("gamepad button {:?} = {:.2}", b.button, b.value),
            RawGamepadEvent::Connection(c) => info!("gamepad connection {:?}", c),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn car_physics(
    time: Res<Time>,
    mut input: ResMut<CarInput>,
    spatial: SpatialQuery,
    collisions: Collisions,
    layers: Query<&CollisionLayers>,
    mut cars: Query<(Entity, &mut Car, Forces, &mut Transform)>,
    surfaces: Res<crate::props::WorldSurfaces>,
    world_tags: Query<(&Collider, &crate::props::SurfaceTags)>,
) {
    let dt = time.delta_secs();
    input.apply_script();
    if input.crashed {
        input.throttle = 0.0;
        input.brake = 0.0;
        input.steer = 0.0;
        input.handbrake = false;
    }
    for (entity, mut car, mut forces, mut transform) in &mut cars {
        if std::mem::take(&mut input.reset) {
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
        let velocity = forces.linear_velocity();
        let speed = velocity.dot(forward);
        car.speed = speed;
        car.yaw_rate = forces.angular_velocity().dot(up);
        car.lateral_accel = car.yaw_rate * speed;
        car.body_slip = velocity.dot(rot * Vec3::X).atan2(speed.abs().max(1.0));
        // The game's speed-banded lookups (steering, handbrake, drift) use the full speed signed by the
        // direction of travel, not the forward component (which reads low in a slide).
        let band_speed = velocity.length().copysign(speed);

        // --- Pedals, reverse ---------------------------------------------------
        let (mut throttle, mut brake) = (input.throttle, input.brake);
        if car.drivetrain.gear < 0 {
            // In reverse the pedals swap roles.
            std::mem::swap(&mut throttle, &mut brake);
            if input.throttle > 0.1 && speed > -1.0 {
                car.drivetrain.engage(1);
            }
        } else if input.brake > 0.1 && input.throttle < 0.05 && speed < 0.5 {
            car.drivetrain.engage(-1);
        }
        let mph = speed / MPH;
        // Standstill hold (`FUN_009d6170` writes vehicle +0x1aa8, `FUN_009b1180` reads it): below a speed
        // threshold (`.bss` 0x01186e60, signed forward speed, not read; assumed 0, see handling.md "Model
        // decisions") the input is the brake pedal (1 with the handbrake up), raised to the throttle in a forward
        // gear. The body's velocity then changes by −sign(speed) × `Brakes.99407bbc` × input × dt along the car's
        // forward axis, every tick, with no clamp at zero.
        let hold_in = if mph < STANDSTILL_HOLD_MPH {
            let parked = if input.handbrake { 1.0 } else { brake };
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
        car.handbrake_timer = if input.handbrake { p.handbrake.min_time } else { car.handbrake_timer - dt };
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
        let held = input.nitrous && !input.crashed;
        let n_params = car.nitrous_params;
        let amount = car.nitrous.step(&n_params, held, dt);
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
        let mut steer_input = input.steer;
        let mut drift_rate = None;
        if let Some(d) = &p.drift {
            if let Some((mapped, lo)) = car.drift.steering(d, input.steer, car.rear_slip) {
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
            input.steer,
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
        let anti_roll = anti_roll_loads(&car.wheels, p);
        let mut ground_normal = Vec3::ZERO;
        // Wheels whose ground is within one tick of the strut's end: (contact, normal, signed overtravel).
        let mut stops: Vec<(Vec3, Vec3, f32)> = Vec::new();
        for ((wheel, bar_load), mut brake_left) in car.wheels.iter_mut().zip(anti_roll).zip(brake_budget) {
            // The mount sits `travel - sag` above the rest position, so the wheel is at its spec position
            // (in its arch) under the static load.
            let mount = pos + rot * (wheel.centre + Vec3::Y * (wheel.travel - wheel.sag));
            let reach = wheel.travel + wheel.radius;
            let ray_dir = Dir3::new(-up).unwrap_or(Dir3::NEG_Y);
            // The ray starts one wheel radius above the mount so that ground which has come up past the strut
            // still registers (from inside a solid it returns a zero normal, through a trimesh's back face
            // nothing). Stopgap, handling.md "Model decisions": the game's wheel collides as a body.
            let lift = wheel.radius;
            let Some(mut hit) = spatial.cast_ray(mount + up * lift, ray_dir, reach + lift, true, &filter) else {
                wheel.compression_rate = 0.0;
                wheel.compression = 0.0;
                wheel.overtravel = 0.0;
                wheel.contact = None;
                let step = (brake_left / wheel.inertia).min(wheel.omega.abs());
                wheel.omega -= step * wheel.omega.signum();
                wheel.spin += wheel.omega * dt;
                continue;
            };
            hit.distance -= lift;
            let compression = (reach - hit.distance).clamp(0.0, wheel.travel);
            wheel.compression = compression;
            wheel.overtravel = (reach - hit.distance - wheel.travel).max(0.0);
            stops.push((mount - up * hit.distance, hit.normal, reach - hit.distance - wheel.travel));
            grounded = true;
            wheels_down += 1;
            ground_normal += hit.normal;
            // Damp with the mount's velocity along the ground normal: using the
            // car's up axis couples forward speed into the damper whenever the
            // body pitches, and the car porpoises at speed.
            let compression_speed = -forces.velocity_at_point(mount).dot(hit.normal);
            wheel.compression_rate = compression_speed;
            let max_load = wheel.corner_mass * 9.81 * 4.0;
            let damper = if compression_speed >= 0.0 { wheel.damper } else { wheel.rebound };
            let load = (wheel.spring * compression + damper * compression_speed + bar_load).clamp(0.0, max_load);
            let contact = mount - up * hit.distance;
            let normal = hit.normal;
            forces.apply_force_at_point(normal * load, contact);

            // Tyre frame on the ground plane.
            let yaw = if wheel.front { Quat::from_axis_angle(up, steer) } else { Quat::IDENTITY };
            let axis = yaw * (rot * Vec3::X);
            let fwd = (yaw * forward).reject_from(normal).normalize_or_zero();
            let side = normal.cross(fwd).normalize_or_zero();
            let axle = usize::from(!wheel.front);
            let tyre = &p.tyres[axle];
            let on_ground = if normal.y > TYRE_NORMAL_MIN_UP { 1.0 } else { 0.0 };
            // The surface under the wheel and how this car's tyres treat it (`FUN_009d8bd0`).
            let surface = WheelSurface::at(&surfaces, &world_tags, hit.entity, (mount, ray_dir, reach), p, wheel.front);
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
            let kappa = tyre::slip_ratio(wheel.omega * wheel.radius, v_long);
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
                * surface.grip[1]
                * if driven { 1.0 - burnout } else { 1.0 }
                * on_ground;
            let lat_cap = mu_lat * n_grip * tyre.lateral_scale * surface.grip[0] * on_ground;
            let (long_cap, lat_cap) = (finite_or_zero(long_cap), finite_or_zero(lat_cap));
            // Longitudinal, in the game's order (`FUN_009c75b0`): the brake (an angular impulse budget that
            // can stop the wheel but never reverse it) acts on the wheel's spin first, then one pass of the
            // tyre constraint pulls the tread towards the road speed (the wheel's spin inertia against the
            // chassis's effective mass at the contact) within the cap. A wheel the handbrake locks stays locked.
            let r = wheel.radius;
            let step = (brake_left / wheel.inertia).min(wheel.omega.abs());
            wheel.omega -= step * wheel.omega.signum();
            brake_left -= step * wheel.inertia;
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
                    "tyre front={} α {slip_deg:.1}° κ {kappa:.2} μ {mu_long:.2}/{mu_lat:.2} load {load:.0} N {n_grip:.0} long {f_long:.0} side {f_side:.0} resist {f_resist:.0} grip {:.2?} steer {:.1}°",
                    wheel.front,
                    surface.grip,
                    steer.to_degrees()
                );
            }

            wheel.spin += wheel.omega * dt;
            wheel.contact = Some(WheelContact {
                point: contact,
                normal,
                slip: side * v_side + fwd * (v_long - wheel.omega * r),
                velocity: v,
                tread_slip: wheel.omega * r - v_long,
            });
        }
        // --- Low-speed turn-around assist (`FUN_008f8360`) -------------------------------------------------
        // The body's contact with the world (not props) on the last physics step, as the wall normal pointing
        // at the car; the game counts the time since the last one (vehicle +0x3010).
        let wall = world_contact(&collisions, &layers, car.body_collider);
        car.body_contact = wall.is_some();
        car.wall_time = if wall.is_some() { 0.0 } else { car.wall_time + dt };
        let side_axis = rot * Vec3::X;
        let (dv, dw) = car.turn_assist.step(
            &TurnAssistInput {
                steer: input.steer,
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
        // Vehicle +0x1ed0 (`FUN_009ae840`): grows while any wheel touches, 0 in the air; the next tick's
        // gearbox gates its downshifts on it.
        car.drivetrain.grounded_time = if wheels_down > 0 { car.drivetrain.grounded_time + dt } else { 0.0 };
        let flat_v = velocity.reject_from(up);
        if let Some(drift) = &p.drift {
            // The drift reads the steering relative to the current range (full range = full lock).
            let range = steer_range(&p.steering, band_speed, car.gear < 0, pedal);
            let out = car.drift.step(
                drift,
                &DriftInput {
                    steer: (input.steer * p.steering.max_angle / range.max(1.0)).clamp(-1.0, 1.0),
                    throttle,
                    brake: input.brake,
                    handbrake: car.handbrake_on,
                    handbrake_prev: handbrake,
                    touching_world: wall.is_some(),
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
        if let Some(n) = ground_normal.try_normalize() {
            car.roll = -(rot * Vec3::X).dot(n).clamp(-1.0, 1.0).asin();
            car.pitch = forward.dot(n).clamp(-1.0, 1.0).asin();
        }
        // The game's per-axis drag (with its over-speed ramp) and lift (downforce). Rolling resistance is per
        // wheel, from the surface (above).
        if stops.iter().any(|s| s.2 > -0.5) {
            let (mut v, mut w) = (forces.linear_velocity(), forces.angular_velocity());
            strut_stops(&stops, com, p.mass, &inv_inertia, dt, (&mut v, &mut w));
            *forces.linear_velocity_mut() = v;
            *forces.angular_velocity_mut() = w;
        }
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
        let driven_radius = car.wheels.iter().filter(|w| !w.front).map(|w| w.radius).next().unwrap_or(0.33);
        car.debug_forces = [drive_torque / driven_radius, tyre_long_total, drag.dot(forward) + resist_total];
        car.tyre_yaw = tyre_yaw;
        trace_engine(time.elapsed_secs(), car.rpm, throttle, car.gear, speed);

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
    /// (lateral, longitudinal) grip factors on the tyre caps.
    grip: [f32; 2],
    /// `RoadOverrides` rolling resistance per newton of load.
    rolling: f32,
    /// Slide drag: the surface's scale × the car's link (drag ramp ends, N per mph; slip angles in degrees).
    drag_scale: f32,
    drag: [f32; 2],
    drag_slip: [f32; 2],
}

impl WheelSurface {
    /// The surface a wheel's ray (`origin`, `dir`, `reach`) hit on `entity`. World colliders carry per-triangle
    /// surface tags; anything else (the flat test track) counts as the game's `Surfaces.Default_surface`, the
    /// list's first entry. Without a surface list at all the tyre gets full grip and no drag.
    fn at(
        surfaces: &crate::props::WorldSurfaces,
        world_tags: &Query<(&Collider, &crate::props::SurfaceTags)>,
        entity: Entity,
        (origin, dir, reach): (Vec3, Dir3, f32),
        p: &CarParams,
        front: bool,
    ) -> Self {
        let tagged = world_tags
            .get(entity)
            .ok()
            .and_then(|(collider, tags)| surfaces.surface_at(collider, tags, origin, *dir, reach))
            .and_then(|hit| hit.surface);
        let info = tagged.or_else(|| surfaces.list.first().cloned());
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
        Self {
            grip,
            rolling: info.as_ref().and_then(|s| s.rolling).unwrap_or(0.0),
            drag_scale: if link.is_some() { info.as_ref().and_then(|s| s.slide_drag).unwrap_or(0.0) } else { 0.0 },
            drag: link.map_or([0.0; 2], |l| l.drag),
            drag_slip: link.map_or([0.0, 1.0], |l| l.drag_slip),
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

/// The end of the strut as a hard contact on the chassis (`FUN_009c5a90` → `FUN_009c2670`, decomp.md
/// "Suspension, wheels, aerodynamics"): the game predicts each wheel contact's penetration at the end of the
/// tick (positions plus velocity × dt); where the ground would come past the strut's end point on the chassis
/// (constraint +0x460) it sets a flag (+0x4ae) and the contact's non-negative normal impulse then acts on the
/// chassis at that point, through the chassis's own mass and inertia, until the predicted penetration is 0.
/// `over` is the ground's reach past the strut's end (m, negative = travel left): the normal velocity at the
/// contact must be at least `over / dt`. Sequential impulses over the contacts (`STOP_ITERATIONS` passes).
fn strut_stops(
    stops: &[(Vec3, Vec3, f32)],
    com: Vec3,
    mass: f32,
    inv_inertia: &Mat3,
    dt: f32,
    (v, w): (&mut Vec3, &mut Vec3),
) {
    let mut total = vec![0.0f32; stops.len()];
    for _ in 0..STOP_ITERATIONS {
        for (&(point, n, over), total) in stops.iter().zip(&mut total) {
            let r = point - com;
            let arm = r.cross(n);
            let m = 1.0 / (1.0 / mass + arm.dot(*inv_inertia * arm)).max(1e-6);
            let vn = (*v + w.cross(r)).dot(n);
            let next = (*total + (over / dt - vn) * m).max(0.0);
            let impulse = next - *total;
            *total = next;
            *v += n * impulse / mass;
            *w += *inv_inertia * (arm * impulse);
        }
    }
}
/// Passes over the strut stops per tick: the game iterates its contacts in sub-steps whose count was not read
/// (handling.md "Model decisions").
const STOP_ITERATIONS: usize = 4;

/// Anti-roll bar load (N) on each wheel from last tick's compressions: per axle a torsion bar between the left
/// and right wheels (`BodyMovement` `782f05c1` front / `2bb0f6a0` rear × 60), damped with
/// c = 2 × `Front_` / `Rear_roll_damping` × √(data stiffness × body roll inertia) as the bar's builder
/// `FUN_00933a60` stores it (the × 60 is not inside the root, so the bar runs at ratio / √60 ≈ 0.15 of critical),
/// turned into forces at the wheels over the track width. The solve that applies the bar was not found
/// (docs/decomp.md): the input (compression difference) and the explicit form are ours.
fn anti_roll_loads(wheels: &[WheelState], p: &CarParams) -> Vec<f32> {
    wheels
        .iter()
        .map(|w| {
            let Some(other) = wheels.iter().find(|o| o.front == w.front && o.centre.x * w.centre.x < 0.0) else {
                return 0.0;
            };
            let axle = usize::from(!w.front);
            let track2 = (w.centre.x - other.centre.x).powi(2).max(0.5);
            let k = p.chassis.anti_roll[axle];
            // 2 ζ √(k_data · I): the data stiffness without the × 60 (`FUN_00933a60`).
            let c = p.chassis.roll_damping[axle] * p.inertia.z.max(0.0).sqrt();
            if w.compression <= 0.0 || other.compression <= 0.0 {
                return 0.0;
            }
            (k * (w.compression - other.compression) + c * (w.compression_rate - other.compression_rate)) / track2
        })
        .collect()
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

/// The body collider's contact with the world on the last physics step, as the contact normal pointing from the
/// world at the car (props do not count: the game's flag is set by world contacts).
fn world_contact(collisions: &Collisions, layers: &Query<&CollisionLayers>, body: Entity) -> Option<Vec3> {
    collisions.collisions_with(body).find_map(|pair| {
        let (other, sign) = if pair.collider1 == body { (pair.collider2, -1.0) } else { (pair.collider1, 1.0) };
        let world = layers.get(other).map_or(true, |l| l.memberships.has_all(crate::props::GameLayer::World));
        if !world {
            return None;
        }
        pair.manifolds.first().map(|m| m.normal * sign)
    })
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

/// `MW_ENGINE_TRACE=<file>`: append `seconds rpm throttle gear speed` every
/// physics tick, for `mwtool engine-sound --trace`.
fn trace_engine(t: f32, rpm: f32, throttle: f32, gear: i32, speed: f32) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("MW_ENGINE_TRACE")?;
        std::fs::File::create(path).ok().map(std::sync::Mutex::new)
    });
    if let Some(f) = file {
        _ = writeln!(f.lock().unwrap(), "{t:.4} {rpm:.1} {throttle:.3} {gear} {speed:.3}");
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
fn log_car(time: Res<Time>, mut next: Local<f32>, cars: Query<(&Car, &Transform)>) {
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
}

/// Target speed for `--test corner` (km/h) and how long the steering takes to wind in.
const CORNER_TEST_KMH: f32 = 80.0;
const CORNER_RAMP_SECS: f32 = 15.0;
const HANDBRAKE_TAP_SECS: f32 = 0.4;
const DRIFT_TEST_KMH: f32 = 110.0;
const DRIFT_TAP_SECS: f32 = 0.3;
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
    cars: Query<&Car>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut test), Ok(car)) = (test, cars.single()) else { return };
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
                input.nitrous = true;
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
            }
            let Some((ts, v0)) = test.braking_from else {
                input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
                return;
            };
            let dt = t - ts;
            let tap = dt < DRIFT_TAP_SECS;
            // Steer left throughout; brake during the tap, then part throttle.
            let (k_steer, k_throttle) = (test_env("MW_TEST_STEER", 0.8), test_env("MW_TEST_THROTTLE", 0.6));
            // Optionally let go of the steering after a while (exit check).
            let k_steer = if dt > test_env("MW_TEST_RELEASE", f32::INFINITY) { 0.0 } else { k_steer };
            input.scripted = Some([if tap { 0.0 } else { k_throttle }, if tap { 1.0 } else { 0.0 }, k_steer, 0.0]);
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
        TestKind::Landing | TestKind::Climb | TestKind::Jitter => {}
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
        _ => return None,
    };
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (i, &(z, y)) in profile.iter().enumerate() {
        vertices.push(Vec3::new(-100.0, y, z));
        vertices.push(Vec3::new(100.0, y, z));
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
    cars: Query<(Entity, &Car, &Transform, &LinearVelocity, &AngularVelocity)>,
    mut state: Local<GroundTest>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut test), Ok((entity, car, tf, vel, ang))) = (test, cars.single()) else { return };
    if test.done || !matches!(test.kind, TestKind::Landing | TestKind::Climb | TestKind::Jitter) {
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
        _ => (test_env("MW_TEST_KMH", 120.0), CLIMB_Z, CLIMB_Z + CLIMB_LENGTH + 120.0),
    };
    let target = target_kmh / 3.6;
    let z = tf.translation.z;
    // Hold the speed up to the obstacle, then full throttle through it (the player keeps the pedal down).
    let throttle = if z < event_z - 20.0 { ((target - v) * 0.5).clamp(0.0, 1.0) } else { 1.0 };
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
            let _ = writeln!(
                f,
                "{t:.3} z {z:.2} y {:.3} sink {sink:.3} vy {:.2} v {:.1} pitch {:.1} body {} wheels {}",
                tf.translation.y,
                vel.y,
                v * 3.6,
                pitch.to_degrees(),
                u8::from(car.body_contact),
                comp.join(" ")
            );
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
