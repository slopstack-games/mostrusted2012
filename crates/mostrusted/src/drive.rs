//! Drivable cars: a raycast-suspension vehicle on avian3d, chase camera and
//! input.
//!
//! The game's handling code is in the encrypted executable, so this is a
//! reimplementation driven by the game's own handling data
//! ([`VehicleHandling`], from each car's GeneSys objects): mass and centre of
//! mass, wheel sizes, spring frequencies and damping ratios, the engine torque
//! curve, gear ratios and shift points, brake torques, top speed, the tyres'
//! lateral and longitudinal grip curves and the speed-banded steering. Parts
//! the data does not pin down (the scale of the grip coefficients, drivetrain
//! efficiency) are calibrated constants; see docs/handling.md.

use avian3d::prelude::*;
use bevy::input::gamepad::RawGamepadEvent;
use bevy::prelude::*;
use mw_assets::handling::{ChaseCamera, LateralCurve, LongCurve, Steering, TyreAxle, VehicleHandling};

use crate::loader::CarSpec;

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        // Test scripting runs in the fixed schedule, right before the physics,
        // so every run feeds the same input on the same tick and `--test`
        // results are reproducible.
        app.init_resource::<CarInput>()
            .add_systems(Update, (spawn_car, read_input, chase_camera, wheel_visuals, log_car))
            .add_systems(FixedUpdate, (run_test, car_physics).chain());
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
}

/// Render parts for each body / wheel model, resolved on the main thread.
pub type CarParts = std::collections::HashMap<mw_bundle::ResourceId, crate::ModelParts>;

#[derive(Resource, Default)]
pub struct CarInput {
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
    pub handbrake: bool,
    pub reset: bool,
    /// Fixed throttle for automated runs (`--auto-throttle`).
    pub auto_throttle: Option<f32>,
    /// Scripted test in progress (`--test`): throttle, brake, steer, handbrake
    /// (> 0.5 = pulled). Overrides other input.
    pub scripted: Option<[f32; 4]>,
}

/// Physical parameters of a car, derived from the game data.
#[derive(Debug, Clone)]
pub struct CarParams {
    pub mass: f32,
    pub inertia: Vec3,
    pub centre_of_mass: Vec3,
    pub handling: VehicleHandling,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub ratios: Vec<f32>,
    pub reverse_ratio: f32,
    pub final_drive: f32,
    pub upshift_rpm: Vec<f32>,
    pub downshift_rpm: Vec<f32>,
    /// Off-throttle downshift points (the cruising gearbox's).
    pub coast_downshift_rpm: Vec<f32>,
    pub front_share: f32,
    pub top_speed: f32,
    /// Quadratic drag coefficient (N per (m/s)²), fitted to the top speed.
    pub drag: f32,
    pub engine_brake_torque: f32,
    pub handbrake_torque: f32,
    /// Front, rear.
    pub tyres: [TyreAxle; 2],
    pub steering: Steering,
    pub shift_time: f32,
    pub in_air_gravity: f32,
    /// Engine rotating inertia (kg m²).
    pub engine_inertia: f32,
    pub camera: ChaseCamera,
}

/// Fraction of engine torque reaching the wheels (not in the data; typical).
const DRIVETRAIN_EFFICIENCY: f32 = 0.85;
/// Map the game's grip-curve force coefficients to friction coefficients. The
/// lateral and longitudinal curves are on different scales in the data (2.9 vs
/// 1.8 peaks on the same tyre); calibrated so the 911 launches like the real
/// car (0–100 km/h in about 4 s, braking at μ ≈ 1.1) and holds about 1.0 g on a
/// skidpad (docs/handling.md).
const LONG_SCALE: f32 = 0.6;
const LAT_SCALE: f32 = 0.44;
const MPH: f32 = 0.44704;
/// The clutch locks at this multiple of the launch rpm.
const CLUTCH_LOCK: f32 = 1.3;

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
    }
}

fn lerp_lat(a: &LateralCurve, b: &LateralCurve, t: f32) -> LateralCurve {
    LateralCurve {
        gradient: a.gradient + (b.gradient - a.gradient) * t,
        peak: a.peak + (b.peak - a.peak) * t,
        peak_angle: a.peak_angle + (b.peak_angle - a.peak_angle) * t,
    }
}

/// (peak, sliding) longitudinal coefficients between two curves.
fn lerp_long(a: &LongCurve, b: &LongCurve, t: f32) -> (f32, f32) {
    (a.peak + (b.peak - a.peak) * t, a.max_slip + (b.max_slip - a.max_slip) * t)
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
        let gb = h.gearbox.clone();
        let ratios = gb
            .as_ref()
            .map(|g| g.ratios.clone())
            .filter(|r| !r.is_empty())
            .unwrap_or(vec![3.5, 2.2, 1.5, 1.15, 0.95, 0.8]);
        let redline_rpm = h.redline_rpm.unwrap_or(7000.0);
        let mut p = Self {
            mass,
            inertia,
            centre_of_mass: Vec3::from(h.centre_of_mass.unwrap_or([0.0, 0.4, 0.0])),
            handling: h.clone(),
            idle_rpm: h.idle_rpm.unwrap_or(900.0),
            redline_rpm,
            upshift_rpm: gb.as_ref().map(|g| g.upshift_rpm.clone()).unwrap_or_default(),
            downshift_rpm: gb.as_ref().map(|g| g.downshift_rpm.clone()).unwrap_or_default(),
            coast_downshift_rpm: gb.as_ref().map(|g| g.coast_downshift_rpm.clone()).unwrap_or_default(),
            reverse_ratio: gb.as_ref().map_or(3.0, |g| g.reverse.abs()),
            final_drive: gb.as_ref().map_or(3.5, |g| g.final_drive),
            ratios,
            front_share: h.front_drive_share.unwrap_or(0.0).clamp(0.0, 1.0),
            top_speed: h.top_speed_mph.unwrap_or(180.0) * 0.44704,
            drag: 0.4,
            engine_brake_torque: h.engine_brake_torque.unwrap_or(-250.0).abs(),
            handbrake_torque: h.handbrake_torque.unwrap_or(1200.0),
            tyres: h.tyres.unwrap_or_else(default_tyres),
            steering: h.steering.unwrap_or_else(default_steering),
            shift_time: h.gear_change_time.unwrap_or(0.15).clamp(0.05, 1.0),
            in_air_gravity: h.in_air_gravity.unwrap_or(1.0).clamp(1.0, 3.0),
            engine_inertia: h.engine_inertia.unwrap_or(0.25).clamp(0.05, 2.0),
            camera: h.camera.unwrap_or(ChaseCamera {
                eye: [0.0, 1.6, -4.6],
                look_at: [0.0, 1.2, 0.75],
                min_fov: 70.0,
                max_fov: 95.0,
                max_roll: 4.7,
                max_pitch_change: 4.5,
                shake_amplitude: 1.2,
                shake_frequency: 1.0,
                shake_power: 1.5,
            }),
        };
        // Fit drag so the best gear's wheel force balances it at top speed.
        let radius = h.wheels.first().map_or(0.33, |w| w.radius);
        let v = p.top_speed;
        let force = p
            .ratios
            .iter()
            .map(|&g| {
                let rpm = v / radius * g * p.final_drive * 60.0 / std::f32::consts::TAU;
                if rpm > p.redline_rpm {
                    0.0
                } else {
                    p.engine_torque(rpm) * g * p.final_drive * DRIVETRAIN_EFFICIENCY / radius
                }
            })
            .fold(0.0, f32::max);
        if force > 0.0 {
            p.drag = force / (v * v);
        }
        p
    }

    pub fn engine_torque(&self, rpm: f32) -> f32 {
        if self.handling.torque_curve.is_empty() {
            return 400.0;
        }
        self.handling.torque_at(rpm).max(0.0)
    }

    fn ratio(&self, gear: i32) -> f32 {
        match gear {
            g if g > 0 => self.ratios.get(g as usize - 1).copied().unwrap_or(1.0),
            g if g < 0 => -self.reverse_ratio,
            _ => 0.0,
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
    /// In the game's drift state (see `drift.rs`).
    pub drifting: bool,
    /// Body roll and pitch relative to the ground plane under the wheels (rad;
    /// roll positive = right side down, pitch positive = nose up).
    pub roll: f32,
    pub pitch: f32,
    pub gear: i32,
    pub rpm: f32,
    shift_timer: f32,
    /// Diagnostics from the last physics step (N): engine force at the axle,
    /// sum of applied longitudinal tyre forces, drag + rolling resistance.
    pub debug_forces: [f32; 3],
    spawn: Vec3,
    pub params: CarParams,
    /// Wheel rays must not hit the car's own collider (a child entity).
    body_collider: Entity,
}

struct WheelState {
    /// Wheel centre at rest, car space.
    centre: Vec3,
    radius: f32,
    front: bool,
    spring: f32,
    damper: f32,
    travel: f32,
    brake_torque: f32,
    /// Current suspension compression (m).
    compression: f32,
    spin: f32,
    pivot: Entity,
    spinner: Entity,
}

fn spawn_car(
    mut commands: Commands,
    pending: Option<Res<PendingCar>>,
    spatial: SpatialQuery,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(pending) = pending else { return };
    // Wait until the collision under the spawn point has streamed in.
    let origin = pending.spawn + Vec3::Y * 30.0;
    let Some(hit) = spatial.cast_ray(origin, Dir3::NEG_Y, 200.0, true, &SpatialQueryFilter::default()) else {
        return;
    };
    let ground = origin - Vec3::Y * hit.distance;
    let start = ground + Vec3::Y * 0.15;

    let params = CarParams::new(&pending.spec.handling);
    info!(
        "car params: {:.0} kg, CoM {:?}, {} gears, final {:.2}, redline {:.0}, top {:.0} km/h, drag {:.3}, peak μ lat {:.2}/{:.2} long {:.2}, front drive {:.2}",
        params.mass,
        params.centre_of_mass,
        params.ratios.len(),
        params.final_drive,
        params.redline_rpm,
        params.top_speed * 3.6,
        params.drag,
        params.tyres[0].pure_lateral.peak * LAT_SCALE,
        params.tyres[1].pure_lateral.peak * LAT_SCALE,
        params.tyres[1].traction_pure_long.peak * LONG_SCALE,
        params.front_share
    );
    let car = commands
        .spawn((
            Name::new("player car"),
            Transform::from_translation(start).with_rotation(Quat::from_rotation_y(pending.heading)),
            Visibility::default(),
            RigidBody::Dynamic,
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
    let body_collider =
        commands.spawn((Collider::cuboid(1.8, 0.75, 4.2), Transform::from_xyz(0.0, 0.85, 0.0), ChildOf(car))).id();
    for m in &pending.spec.body {
        for (mesh, mat) in pending.parts.get(m).into_iter().flatten() {
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), ChildOf(car)));
        }
    }

    // Static load per corner from the centre of mass between the axles.
    let h = &pending.spec.handling;
    let z_front = pending.spec.wheels.iter().filter(|w| w.front).map(|w| w.position.z).fold(f32::MIN, f32::max);
    let z_rear = pending.spec.wheels.iter().filter(|w| !w.front).map(|w| w.position.z).fold(f32::MAX, f32::min);
    let front_load = ((params.centre_of_mass.z - z_rear) / (z_front - z_rear).max(0.5)).clamp(0.2, 0.8);
    let mut wheels = Vec::new();
    for w in &pending.spec.wheels {
        let corner_mass = params.mass * if w.front { front_load } else { 1.0 - front_load } / 2.0;
        let spring = h.springs.get(if w.front { 0 } else { 1 }).copied();
        let (freq, zeta, travel) = spring.map_or((1.8, 0.4, 0.12), |s| (s.frequency, s.damping_ratio, s.travel));
        let k = corner_mass * (std::f32::consts::TAU * freq).powi(2);
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
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), ChildOf(parent)));
            }
        }
        debug!(
            "wheel {:?} front={} radius {radius:.3} k {k:.0} c {c:.0} travel {:.3} corner {corner_mass:.0} kg",
            w.position,
            w.front,
            travel.max(0.05) + corner_mass * 9.81 / k
        );
        wheels.push(WheelState {
            centre: w.position,
            radius,
            front: w.front,
            spring: k,
            damper: c,
            // Travel above the rest position plus room for the static sag.
            travel: travel.max(0.05) + corner_mass * 9.81 / k,
            brake_torque: brake,
            compression: 0.0,
            spin: 0.0,
            pivot,
            spinner,
        });
    }
    commands.entity(car).insert(Car {
        wheels,
        steer: 0.0,
        speed: 0.0,
        yaw_rate: 0.0,
        lateral_accel: 0.0,
        body_slip: 0.0,
        drifting: false,
        roll: 0.0,
        pitch: 0.0,
        gear: 1,
        rpm: params.idle_rpm,
        shift_timer: 0.0,
        debug_forces: [0.0; 3],
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

fn read_input(keys: Res<ButtonInput<KeyCode>>, pads: Query<&Gamepad>, mut input: ResMut<CarInput>) {
    let key = |k: &[KeyCode]| -> f32 { if k.iter().any(|k| keys.pressed(*k)) { 1.0 } else { 0.0 } };
    let mut throttle = key(&[KeyCode::KeyW, KeyCode::ArrowUp]);
    let mut brake = key(&[KeyCode::KeyS, KeyCode::ArrowDown]);
    let mut steer = key(&[KeyCode::KeyA, KeyCode::ArrowLeft]) - key(&[KeyCode::KeyD, KeyCode::ArrowRight]);
    let mut handbrake = keys.pressed(KeyCode::Space);
    input.reset = keys.just_pressed(KeyCode::KeyR) || input.reset;
    if input.scripted.is_some() {
        // A `--test` script drives the car; `car_physics` reads it per tick.
        return;
    }
    for pad in &pads {
        throttle = throttle.max(trigger(pad, GamepadButton::RightTrigger2, GamepadAxis::RightZ));
        brake = brake.max(trigger(pad, GamepadButton::LeftTrigger2, GamepadAxis::LeftZ));
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        // Generous deadzone: drifting sticks are common.
        if x.abs() > 0.2 {
            steer = -x;
        }
        handbrake |= pad.pressed(GamepadButton::South);
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

/// Analog trigger value in 0..1. Mapped pads report triggers as analog
/// buttons; some report them as Z axes instead.
fn trigger(pad: &Gamepad, button: GamepadButton, axis: GamepadAxis) -> f32 {
    let b = pad.get(button).unwrap_or(0.0);
    let a = pad.get(axis).unwrap_or(0.0);
    let digital = if pad.pressed(button) { 1.0 } else { 0.0 };
    b.max(a).max(digital).clamp(0.0, 1.0)
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

fn car_physics(
    time: Res<Time>,
    mut input: ResMut<CarInput>,
    spatial: SpatialQuery,
    mut cars: Query<(Entity, &mut Car, Forces, &mut Transform)>,
) {
    use std::f32::consts::TAU;
    let dt = time.delta_secs();
    input.apply_script();
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

        // --- Gear selection (automatic) -------------------------------------
        let (mut throttle, mut brake) = (input.throttle, input.brake);
        if car.gear < 0 {
            // In reverse the pedals swap roles.
            std::mem::swap(&mut throttle, &mut brake);
            if input.throttle > 0.1 && speed > -1.0 {
                car.gear = 1;
            }
        } else if input.brake > 0.1 && input.throttle < 0.05 && speed < 0.5 {
            car.gear = -1;
        }
        let driven_radius = car.wheels.iter().filter(|w| !w.front).map(|w| w.radius).next().unwrap_or(0.33);
        let wheel_rpm = speed.abs() / driven_radius * 60.0 / TAU;
        let ratio = p.ratio(car.gear);
        let mut rpm = wheel_rpm * ratio.abs() * p.final_drive;
        if car.gear > 0 {
            let g = car.gear as usize;
            // Shift no later than just below the limiter (some cars' shift
            // points lie above their redline).
            let up_rpm = p
                .upshift_rpm
                .get(g - 1)
                .copied()
                .filter(|&r| r > 0.0)
                .unwrap_or(p.redline_rpm)
                .min(p.redline_rpm - 100.0);
            // The race gearbox's downshift points are kick-down points: off
            // the throttle the box shifts down at the cruising gearbox's (much
            // lower) points instead, blending between the two with the pedal.
            // Using the race points throughout made the revs leap up by
            // ~1800 rpm every few seconds while coasting or braking.
            let race_down = p.downshift_rpm.get(g - 1).copied().unwrap_or(0.0);
            let coast_down = p.coast_downshift_rpm.get(g - 1).copied().unwrap_or(race_down).min(race_down);
            let down_rpm = coast_down + (race_down - coast_down) * throttle;
            if rpm >= up_rpm && g < p.ratios.len() && car.shift_timer <= 0.0 {
                car.gear += 1;
                car.shift_timer = p.shift_time;
            } else if g > 1 && rpm < down_rpm && car.shift_timer <= 0.0 {
                car.gear -= 1;
                car.shift_timer = p.shift_time;
            }
        }
        car.shift_timer -= dt;
        let ratio = p.ratio(car.gear);
        rpm = wheel_rpm * ratio.abs() * p.final_drive;
        // Clutch slip when pulling away (first gear or reverse only): the
        // engine revs above the wheels, starting at the launch rpm and rising
        // with them until the clutch locks, so the engine note never holds
        // still while the car accelerates. In the higher gears the clutch is
        // locked and the revs follow the wheels; applying the slip there made
        // every throttle tap jump the revs by ~600 rpm.
        let launch = p.idle_rpm + throttle * (p.redline_rpm * 0.55 - p.idle_rpm).max(0.0);
        rpm = if throttle > 0.05 && car.gear.abs() == 1 {
            let lock = launch * CLUTCH_LOCK;
            if rpm < lock { launch + (lock - launch) * rpm / lock } else { rpm }
        } else {
            rpm.max(p.idle_rpm)
        };
        // The engine's own inertia limits how fast the revs move: up by its
        // torque, down by its resistance (both over `Engine_inertia`).
        let to_rpm = 60.0 / TAU / p.engine_inertia * dt;
        let rev_up = p.engine_torque(car.rpm).max(100.0) * to_rpm;
        let rev_down = p.engine_brake_torque.max(100.0) * to_rpm;
        car.rpm += (rpm.min(p.redline_rpm + 200.0) - car.rpm).clamp(-rev_down, rev_up);

        // --- Engine torque to the wheels -------------------------------------
        let engine = if car.shift_timer > 0.0 || car.gear == 0 {
            0.0
        } else if throttle > 0.01 && car.rpm < p.redline_rpm {
            p.engine_torque(car.rpm) * throttle
        } else {
            // Engine braking, stronger at high revs.
            -p.engine_brake_torque * ((car.rpm - p.idle_rpm) / (p.redline_rpm - p.idle_rpm)).clamp(0.0, 1.0)
        };
        let axle_force = engine * ratio * p.final_drive * DRIVETRAIN_EFFICIENCY / driven_radius;
        // No drive beyond the game's top speed (drag is fitted to it too).
        let axle_force = if speed.abs() > p.top_speed && axle_force * speed.signum() > 0.0 { 0.0 } else { axle_force };
        let n_front = car.wheels.iter().filter(|w| w.front).count().max(1) as f32;
        let n_rear = car.wheels.iter().filter(|w| !w.front).count().max(1) as f32;

        car.steer = steer_towards(&p.steering, car.steer, input.steer, speed, car.gear < 0, dt);
        let steer = car.steer;

        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider]);
        let corner_mass = p.mass / car.wheels.len().max(1) as f32;
        let (front_share, handbrake_torque) = (p.front_share, p.handbrake_torque);
        let mut tyre_long_total = 0.0;
        let mut grounded = false;
        let mut ground_normal = Vec3::ZERO;
        for wheel in &mut car.wheels {
            let mount = pos + rot * (wheel.centre + Vec3::Y * wheel.travel);
            let reach = wheel.travel + wheel.radius;
            let Some(hit) = spatial.cast_ray(mount, Dir3::new(-up).unwrap_or(Dir3::NEG_Y), reach, true, &filter) else {
                wheel.compression = 0.0;
                continue;
            };
            let compression = (reach - hit.distance).clamp(0.0, wheel.travel);
            wheel.compression = compression;
            grounded = true;
            ground_normal += hit.normal;
            // Damp with the mount's velocity along the ground normal: using the
            // car's up axis couples forward speed into the damper whenever the
            // body pitches, and the car porpoises at speed.
            let compression_speed = -forces.velocity_at_point(mount).dot(hit.normal);
            let max_load = corner_mass * 9.81 * 4.0;
            let load = (wheel.spring * compression + wheel.damper * compression_speed).clamp(0.0, max_load);
            let contact = mount - up * hit.distance;
            let normal = hit.normal;
            forces.apply_force_at_point(normal * load, contact);

            // Tyre frame on the ground plane.
            let yaw = if wheel.front { Quat::from_axis_angle(up, steer) } else { Quat::IDENTITY };
            let fwd = (yaw * forward).reject_from(normal).normalize_or_zero();
            let side = normal.cross(fwd).normalize_or_zero();
            let v = forces.velocity_at_point(contact);
            let (v_long, v_side) = (v.dot(fwd), v.dot(side));

            // Longitudinal demand: drive split by axle, brakes oppose rolling.
            let share = if wheel.front { front_share / n_front } else { (1.0 - front_share) / n_rear };
            let drive = axle_force * share;
            let locked = input.handbrake && !wheel.front;
            let brake_force =
                brake * wheel.brake_torque / wheel.radius + if locked { handbrake_torque / wheel.radius } else { 0.0 };
            // Smooth near zero so the car comes to rest instead of jittering.
            let demand = drive - brake_force * (v_long / 0.5).clamp(-1.0, 1.0);
            let braking = locked || demand * v_long < 0.0;

            // The game's tyre model: separate "pure" and "slip" curves for
            // braking and traction, blended by how much the tyre slides in the
            // other direction (docs/handling.md).
            let tyre = &p.tyres[if wheel.front { 0 } else { 1 }];
            let slip_deg = v_side.atan2(v_long.abs().max(2.0)).to_degrees();
            let lat_use = (slip_deg.abs() / tyre.pure_lateral.peak_angle.max(1.0)).min(1.0);
            let (pure_long, slip_long) = if braking {
                (&tyre.brake_pure_long, &tyre.brake_slip_long)
            } else {
                (&tyre.traction_pure_long, &tyre.traction_slip_long)
            };
            let (peak, sliding) = lerp_long(pure_long, slip_long, lat_use);
            let long_scale = LONG_SCALE * tyre.longitudinal_scale;
            let long_limit = peak.max(sliding) * long_scale * load;
            // Beyond the peak a wheel spins up or locks and grip falls to the
            // curve's sliding value. Driver aids hold the peak instead: ABS on
            // the service brakes, traction control on the drive unless the
            // handbrake is pulled or the car already slides sideways (so the
            // throttle still holds a drift). A handbraked wheel locks.
            let assisted = if braking { !locked } else { !input.handbrake && lat_use < 0.5 };
            let long_slipping = locked || (!assisted && demand.abs() > long_limit);
            let f_long = if locked {
                -sliding * long_scale * load * (v_long / 0.5).clamp(-1.0, 1.0)
            } else if long_slipping {
                demand.signum() * sliding * long_scale * load
            } else {
                demand.clamp(-long_limit, long_limit)
            };
            let long_use = if long_slipping { 1.0 } else { (f_long.abs() / long_limit.max(1.0)).powi(2) };
            let slip_curve = if braking { &tyre.brake_slip_lateral } else { &tyre.traction_slip_lateral };
            let curve = lerp_lat(&tyre.pure_lateral, slip_curve, long_use);
            let mu_lat = curve.at(slip_deg) * LAT_SCALE * tyre.lateral_scale;
            let f_side_slip = -mu_lat * load;
            // At walking pace slip angles are meaningless: cancel sideways velocity instead.
            let f_side_cancel = -v_side * corner_mass / dt * 0.5;
            let f_side = if f_side_cancel.abs() < f_side_slip.abs() { f_side_cancel } else { f_side_slip };
            // Combined grip: an ellipse between the two limits.
            let lat_limit = (curve.peak * LAT_SCALE * tyre.lateral_scale * load).max(1.0);
            let k = ((f_long / long_limit.max(1.0)).powi(2) + (f_side / lat_limit).powi(2)).sqrt();
            let f = Vec2::new(f_long, f_side) / k.max(1.0);
            forces.apply_force_at_point(fwd * f.x + side * f.y, contact);
            tyre_long_total += f.x;
            if tyre_debug() && (time.elapsed_secs() * 2.0).fract() < dt * 2.0 {
                info!(
                    "tyre front={} slip {slip_deg:.1}° μ {mu_lat:.2} load {load:.0} long {f_long:.0} side {f_side:.0} steer {:.1}°",
                    wheel.front,
                    steer.to_degrees()
                );
            }

            wheel.spin += v_long / wheel.radius * dt;
        }
        if let Some(n) = ground_normal.try_normalize() {
            car.roll = -(rot * Vec3::X).dot(n).clamp(-1.0, 1.0).asin();
            car.pitch = forward.dot(n).clamp(-1.0, 1.0).asin();
        }
        // Aerodynamic drag (fitted to the game's top speed) and a constant
        // rolling resistance (Crr 0.015), faded in near standstill.
        let rolling = velocity.normalize_or_zero() * (velocity.length() / 1.0).min(1.0) * 0.015 * p.mass * 9.81;
        let resist = -velocity * velocity.length() * p.drag - rolling;
        forces.apply_force(resist);
        // The game pulls airborne cars down harder than gravity.
        if !grounded {
            forces.apply_force(Vec3::NEG_Y * (p.in_air_gravity - 1.0) * p.mass * 9.81);
        }
        car.debug_forces = [axle_force, tyre_long_total, resist.dot(forward)];
        trace_engine(time.elapsed_secs(), car.rpm, throttle, car.gear, speed);

        // Fell out of the world: back to the spawn point.
        if pos.y < car.spawn.y - 200.0 {
            transform.translation = car.spawn;
            *forces.linear_velocity_mut() = Vec3::ZERO;
        }
    }
}

/// Move the front wheel angle (radians) towards the input at the game's
/// steering rates: separate rates for steering further, steering across to the
/// other side and returning to centre, each for low speed, high speed and
/// reverse. Below `Medium_speed` the low-speed values apply, above `High_speed`
/// the high-speed ones, with a linear blend between.
fn steer_towards(s: &Steering, current: f32, input: f32, speed: f32, reverse: bool, dt: f32) -> f32 {
    let mph = speed.abs() / MPH;
    let pick = |v: [f32; 3]| {
        if reverse {
            return v[2];
        }
        let t = ((mph - s.medium_speed_mph) / (s.high_speed_mph - s.medium_speed_mph).max(1.0)).clamp(0.0, 1.0);
        v[0] + (v[1] - v[0]) * t
    };
    let target = (input * pick(s.range)).to_radians();
    let rate = if input.abs() < 0.05 {
        pick(s.center_rate)
    } else if target * current < 0.0 {
        pick(s.counter_rate)
    } else if target.abs() < current.abs() {
        // Easing off part of the way: as fast as centring.
        pick(s.center_rate).max(pick(s.rate))
    } else {
        pick(s.rate)
    };
    let step = rate.to_radians() * dt;
    current + (target - current).clamp(-step, step)
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
                t.translation = w.centre + Vec3::Y * w.compression;
                t.rotation = if w.front { Quat::from_rotation_y(car.steer) } else { Quat::IDENTITY };
            }
            if let Ok(mut t) = transforms.get_mut(w.spinner) {
                t.rotation = Quat::from_rotation_x(w.spin);
            }
        }
    }
}

/// Chase-camera state, smoothed every frame.
#[derive(Default)]
struct ChaseState {
    init: bool,
    yaw: f32,
    up: Vec3,
    /// Extra distance (m, + = further back) from acceleration.
    pull: f32,
    fov: f32,
    roll: f32,
    pitch: f32,
    speed: f32,
    accel: f32,
    shake_t: f32,
}

/// The game's external camera (`VehicleCameraContainer.External`): eye and
/// look-at points behind the car, a field of view that widens with speed,
/// roll into corners, a dip under braking and a high-speed shake. Rendering
/// uses the physics-interpolated car pose, so it is smooth at any frame rate.
/// The camera's yaw trails the car's and leans towards the direction of
/// travel, so drifts show; only car-relative offsets are smoothed, so the
/// distance does not grow with speed.
fn chase_camera(
    time: Res<Time>,
    mut state: Local<ChaseState>,
    cars: Query<(&Transform, &LinearVelocity, &Car), Without<Camera3d>>,
    mut camera: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let (Ok((car_t, vel, car)), Ok((mut cam, mut projection))) = (cars.single(), camera.single_mut()) else {
        return;
    };
    let dt = time.delta_secs().max(1e-4);
    let ease = |rate: f32| 1.0 - (-dt * rate).exp();
    let c = car.params.camera;
    let s = &mut *state;

    let forward = car_t.rotation * Vec3::Z;
    let car_yaw = forward.x.atan2(forward.z);
    let flat_v = Vec3::new(vel.x, 0.0, vel.z);
    let speed = flat_v.length();
    // Lean towards the velocity direction at speed (about a third of the
    // drift angle), never when reversing.
    let travel_yaw = flat_v.x.atan2(flat_v.z);
    let lean = if car.speed > 3.0 { (speed / 15.0).min(1.0) * 0.35 } else { 0.0 };
    let target_yaw = car_yaw + wrap_angle(travel_yaw - car_yaw) * lean;
    if !s.init {
        *s = ChaseState { init: true, yaw: target_yaw, up: Vec3::Y, fov: c.min_fov, speed, ..default() };
    }
    s.yaw += wrap_angle(target_yaw - s.yaw) * ease(6.0);
    // Follow the ground's slope slowly, not every bump.
    s.up = s.up.lerp(car_t.rotation * Vec3::Y, ease(2.0)).normalize_or(Vec3::Y);

    // Longitudinal acceleration: fall back when accelerating, close in when braking.
    let accel = (speed - s.speed) / dt;
    s.speed = speed;
    s.accel += (accel.clamp(-30.0, 30.0) - s.accel) * ease(4.0);
    s.pull += ((s.accel * 0.05).clamp(-0.6, 0.8) - s.pull) * ease(3.0);

    let frame = Quat::from_rotation_arc(Vec3::Y, s.up) * Quat::from_rotation_y(s.yaw);
    let eye = Vec3::from(c.eye) - Vec3::Z * s.pull;
    let target = car_t.translation + frame * Vec3::from(c.look_at);
    let mut pos = car_t.translation + frame * eye;

    // Field of view: from MinFieldOfView at rest to MaxFieldOfView at top speed.
    let t = (speed / car.params.top_speed.max(1.0)).clamp(0.0, 1.0);
    s.fov += (c.min_fov + (c.max_fov - c.min_fov) * t * t - s.fov) * ease(3.0);
    // Roll into corners and dip under braking (degrees from the game data).
    let lat_g = (car.lateral_accel / 9.81).clamp(-1.2, 1.2);
    s.roll += (lat_g * c.max_roll - s.roll) * ease(4.0);
    let brake_g = (-s.accel / 9.81).clamp(0.0, 1.0);
    s.pitch += (brake_g * c.max_pitch_change - s.pitch) * ease(5.0);

    // High-speed shake: grows with (speed fraction)^Power.
    s.shake_t += dt * c.shake_frequency * 9.0;
    let shake = c.shake_amplitude * 0.012 * t.powf(c.shake_power.max(0.1));
    pos += frame
        * Vec3::new((s.shake_t * 1.7).sin() + 0.5 * (s.shake_t * 4.1).sin(), (s.shake_t * 2.3).sin(), 0.0)
        * shake;

    cam.translation = pos;
    cam.look_at(target, s.up);
    cam.rotate_local_x(-s.pitch.to_radians());
    cam.rotate_local_z(s.roll.to_radians());
    if let Projection::Perspective(p) = &mut *projection {
        // Vertical, as Bevy's: read as horizontal, 70° crops the car far
        // tighter than the original's chase view.
        p.fov = s.fov.to_radians();
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
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
        "car at {:.1} {:.3} {:.1}, {:.0} km/h, gear {} at {:.0} rpm, pitch {:.2}°, forces {:?}, compression {:?}",
        t.translation.x,
        t.translation.y,
        t.translation.z,
        car.speed * 3.6,
        car.gear,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// Full throttle from rest: 0–60 mph, 0–100/200 km/h, quarter mile, top speed.
    Accel,
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
    /// A human-like pedal sequence for engine-sound checks (record it with
    /// `MW_ENGINE_TRACE`): full throttle, lift and coast, throttle taps,
    /// braking, full throttle again.
    Engine,
}

/// Target speed for `--test corner` (km/h) and how long the steering takes to wind in.
const CORNER_TEST_KMH: f32 = 80.0;
const CORNER_RAMP_SECS: f32 = 15.0;
const HANDBRAKE_TAP_SECS: f32 = 0.4;
const DRIFT_TEST_KMH: f32 = 110.0;
const DRIFT_TAP_SECS: f32 = 0.3;
const DRIFT_HOLD_SECS: f32 = 6.0;
/// Body slip beyond which the drift test counts the car as drifting.
const DRIFT_HELD_DEG: f32 = 10.0;

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
        TestKind::Accel => {
            input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
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
            let target = DRIFT_TEST_KMH / 3.6;
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
            input.scripted = Some([if tap { 0.0 } else { 0.6 }, if tap { 1.0 } else { 0.0 }, 0.8, 0.0]);
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
            if (dt * 2.0).floor() != ((dt - step) * 2.0).floor() {
                info!(
                    "TEST drift +{dt:.1} s: {:.0} km/h, body slip {:.1}°, yaw {:.0}°/s, drifting {}",
                    v * 3.6,
                    car.body_slip.to_degrees(),
                    car.yaw_rate.to_degrees(),
                    car.drifting
                );
            }
            if dt >= DRIFT_HOLD_SECS || v < 3.0 {
                let [peak, held, sum, yaw] = test.corner_best;
                let mean = if held > 0.0 { sum / held } else { 0.0 };
                info!(
                    "TEST drift: peak {peak:.1}°, beyond {DRIFT_HELD_DEG}° for {held:.2} s (mean {mean:.1}°), peak yaw {yaw:.0}°/s, {:.0} -> {:.0} km/h",
                    v0 * 3.6,
                    v * 3.6
                );
                let mark = format!("peak={peak:.1}deg,held={held:.2}s,mean={mean:.1}deg,exit={:.0}kmh", v * 3.6);
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
