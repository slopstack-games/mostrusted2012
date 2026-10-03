//! Drivable cars: a raycast-suspension vehicle on avian3d, chase camera and
//! input.
//!
//! The game's handling code is in the encrypted executable, so this is a
//! reimplementation driven by the game's own handling data
//! ([`VehicleHandling`], from each car's GeneSys objects): mass and centre of
//! mass, wheel sizes, spring frequencies and damping ratios, the engine torque
//! curve, gear ratios and shift points, brake torques and top speed. Parts the
//! data does not pin down (tyre slip curves, drivetrain efficiency) use
//! documented defaults; see docs/handling.md.

use avian3d::prelude::*;
use bevy::input::gamepad::RawGamepadEvent;
use bevy::prelude::*;
use mw_assets::handling::VehicleHandling;

use crate::loader::CarSpec;

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CarInput>()
            .add_systems(Update, (spawn_car, read_input, chase_camera, wheel_visuals, log_car, run_test))
            .add_systems(FixedUpdate, car_physics);
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
    /// Scripted test in progress (`--test`); overrides other input.
    pub scripted: Option<(f32, f32)>,
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
    pub front_share: f32,
    pub top_speed: f32,
    /// Quadratic drag coefficient (N per (m/s)²), fitted to the top speed.
    pub drag: f32,
    pub engine_brake_torque: f32,
    pub handbrake_torque: f32,
    pub grip: f32,
}

/// Fraction of engine torque reaching the wheels (not in the data; typical).
const DRIVETRAIN_EFFICIENCY: f32 = 0.85;
/// Time with no drive torque while changing gear (the sport gearbox's 0.15 s field).
const SHIFT_TIME: f32 = 0.15;
const MAX_STEER_DEG: f32 = 35.0;
/// Maps the game's longitudinal grip-curve peak to a friction coefficient.
const GRIP_SCALE: f32 = 0.72;
/// Lateral slip angle at which the tyre reaches peak grip.
const PEAK_SLIP_ANGLE: f32 = 0.12;

impl CarParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let mass = h.mass.unwrap_or(1450.0);
        let ext = h.body_half_extents.unwrap_or([0.9, 0.55, 2.2]);
        // Solid box with the body's half extents.
        let inertia = Vec3::new(
            mass / 3.0 * (ext[1] * ext[1] + ext[2] * ext[2]),
            mass / 3.0 * (ext[0] * ext[0] + ext[2] * ext[2]),
            mass / 3.0 * (ext[0] * ext[0] + ext[1] * ext[1]),
        );
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
            reverse_ratio: gb.as_ref().map_or(3.0, |g| g.reverse.abs()),
            final_drive: gb.as_ref().map_or(3.5, |g| g.final_drive),
            ratios,
            front_share: h.front_drive_share.unwrap_or(0.0).clamp(0.0, 1.0),
            top_speed: h.top_speed_mph.unwrap_or(180.0) * 0.44704,
            drag: 0.4,
            engine_brake_torque: h.engine_brake_torque.unwrap_or(-250.0).abs(),
            handbrake_torque: h.handbrake_torque.unwrap_or(1200.0),
            // The tyre curve "peak" (1.3–1.6) is not a plain friction
            // coefficient; scaled to match real-world launch times until
            // in-game measurements exist (docs/handling.md).
            grip: (h.tyre_long_peak.unwrap_or(1.5) * GRIP_SCALE).clamp(0.7, 1.4),
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
        "car params: {:.0} kg, CoM {:?}, {} gears, final {:.2}, redline {:.0}, top {:.0} km/h, drag {:.3}, grip {:.2}, front drive {:.2}",
        params.mass,
        params.centre_of_mass,
        params.ratios.len(),
        params.final_drive,
        params.redline_rpm,
        params.top_speed * 3.6,
        params.drag,
        params.grip,
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
    if let Some((t, b)) = input.scripted {
        throttle = t;
        brake = b;
        steer = 0.0;
        handbrake = false;
    }
    input.throttle = throttle;
    input.brake = brake;
    input.steer = steer.clamp(-1.0, 1.0);
    input.handbrake = handbrake;
    input.reset = keys.just_pressed(KeyCode::KeyR) || input.reset;
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
            let down_rpm = p.downshift_rpm.get(g - 1).copied().unwrap_or(0.0);
            if rpm >= up_rpm && g < p.ratios.len() && car.shift_timer <= 0.0 {
                car.gear += 1;
                car.shift_timer = SHIFT_TIME;
            } else if g > 1 && rpm < down_rpm && car.shift_timer <= 0.0 {
                car.gear -= 1;
                car.shift_timer = SHIFT_TIME;
            }
        }
        car.shift_timer -= dt;
        let ratio = p.ratio(car.gear);
        rpm = wheel_rpm * ratio.abs() * p.final_drive;
        // Clutch slip at low speed: the engine can rev above the wheels.
        let launch = p.idle_rpm + throttle * (p.redline_rpm * 0.55 - p.idle_rpm).max(0.0);
        rpm = rpm.max(if throttle > 0.05 { launch } else { p.idle_rpm });
        car.rpm += (rpm.min(p.redline_rpm + 200.0) - car.rpm) * (1.0 - (-dt * 20.0).exp());

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

        // Steering: full lock at low speed, less at speed, smoothed.
        let max_steer = MAX_STEER_DEG.to_radians() / (1.0 + speed.abs() / 20.0);
        let target = input.steer * max_steer;
        car.steer += (target - car.steer) * (1.0 - (-dt * 8.0).exp());
        let steer = car.steer;

        let filter = SpatialQueryFilter::from_excluded_entities([entity, car.body_collider]);
        let corner_mass = p.mass / car.wheels.len().max(1) as f32;
        let grip = p.grip;
        let (front_share, handbrake_torque) = (p.front_share, p.handbrake_torque);
        let mut tyre_long_total = 0.0;
        for wheel in &mut car.wheels {
            let mount = pos + rot * (wheel.centre + Vec3::Y * wheel.travel);
            let reach = wheel.travel + wheel.radius;
            let Some(hit) = spatial.cast_ray(mount, Dir3::new(-up).unwrap_or(Dir3::NEG_Y), reach, true, &filter) else {
                wheel.compression = 0.0;
                continue;
            };
            let compression = (reach - hit.distance).clamp(0.0, wheel.travel);
            wheel.compression = compression;
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

            // Longitudinal: drive split by axle, brakes oppose rolling.
            let share = if wheel.front { front_share / n_front } else { (1.0 - front_share) / n_rear };
            let mut f_long = axle_force * share;
            let brake_force = brake * wheel.brake_torque / wheel.radius
                + if input.handbrake && !wheel.front { handbrake_torque / wheel.radius } else { 0.0 };
            // Smooth near zero so the car comes to rest instead of jittering.
            f_long -= brake_force * (v_long / 0.5).clamp(-1.0, 1.0);

            // Lateral: slip-angle model, velocity-cancelling at low speed.
            let slip_angle = v_side.atan2(v_long.abs().max(2.0));
            let mu = grip * if input.handbrake && !wheel.front { 0.45 } else { 1.0 };
            let f_side_slip = -mu * load * (slip_angle / PEAK_SLIP_ANGLE).clamp(-1.0, 1.0);
            let f_side_cancel = -v_side * corner_mass / dt * 0.5;
            let f_side = if f_side_cancel.abs() < f_side_slip.abs() { f_side_cancel } else { f_side_slip };
            // Friction circle.
            let limit = mu * load;
            let f = Vec2::new(f_long, f_side);
            let f = if f.length() > limit { f * (limit / f.length()) } else { f };
            forces.apply_force_at_point(fwd * f.x + side * f.y, contact);
            tyre_long_total += f.x;

            wheel.spin += v_long / wheel.radius * dt;
        }
        // Aerodynamic drag (fitted to the game's top speed) and a constant
        // rolling resistance (Crr 0.015), faded in near standstill.
        let rolling = velocity.normalize_or_zero() * (velocity.length() / 1.0).min(1.0) * 0.015 * p.mass * 9.81;
        let resist = -velocity * velocity.length() * p.drag - rolling;
        forces.apply_force(resist);
        car.debug_forces = [axle_force, tyre_long_total, resist.dot(forward)];

        // Fell out of the world: back to the spawn point.
        if pos.y < car.spawn.y - 200.0 {
            transform.translation = car.spawn;
            *forces.linear_velocity_mut() = Vec3::ZERO;
        }
    }
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

fn chase_camera(
    time: Res<Time>,
    cars: Query<&Transform, (With<Car>, Without<Camera3d>)>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    let (Ok(car), Ok(mut cam)) = (cars.single(), camera.single_mut()) else { return };
    let yaw = car.rotation.to_euler(EulerRot::YXZ).0;
    let target = car.translation + Quat::from_rotation_y(yaw) * Vec3::new(0.0, 2.2, -6.5);
    let k = 1.0 - (-time.delta_secs() * 6.0).exp();
    cam.translation = cam.translation.lerp(target, k);
    cam.look_at(car.translation + Vec3::Y * 1.0, Vec3::Y);
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
    started: Option<f32>,
    distance: f32,
    marks: Vec<(String, f32, f32)>,
    top_speed: f32,
    plateau: (f32, f32),
    braking_from: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// Full throttle from rest: 0–60 mph, 0–100/200 km/h, quarter mile, top speed.
    Accel,
    /// Accelerate to 100 km/h, then full brakes: stopping distance and time.
    Brake,
}

impl DriveTest {
    pub fn new(kind: TestKind) -> Self {
        Self {
            kind,
            started: None,
            distance: 0.0,
            marks: Vec::new(),
            top_speed: 0.0,
            plateau: (0.0, 0.0),
            braking_from: None,
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
    let now = time.elapsed_secs();
    // Let the suspension settle before starting.
    let t0 = *test.started.get_or_insert(now + 1.5);
    if now < t0 {
        input.scripted = Some((0.0, 0.0));
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
    let finish = |test: &DriveTest, exit: &mut MessageWriter<AppExit>| {
        info!(
            "TEST RESULT {:?}: {}",
            test.kind,
            test.marks.iter().map(|(n, t, _)| format!("{n}={t:.2}s")).collect::<Vec<_>>().join(" ")
        );
        exit.write(AppExit::Success);
    };
    match test.kind {
        TestKind::Accel => {
            input.scripted = Some((1.0, 0.0));
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
                    finish(&test, &mut exit);
                }
                test.plateau = (t, v);
            }
        }
        TestKind::Brake => match test.braking_from {
            None => {
                input.scripted = Some((1.0, 0.0));
                if v >= 27.778 {
                    test.braking_from = Some((t, test.distance));
                }
            }
            Some((tb, db)) => {
                input.scripted = Some((0.0, 1.0));
                if v < 0.3 {
                    let dist = test.distance - db;
                    info!("TEST 100-0 km/h: {:.2} s, {dist:.1} m", t - tb);
                    test.marks.push((format!("100-0kmh({dist:.1}m)"), t - tb, dist));
                    finish(&test, &mut exit);
                }
            }
        },
    }
}
