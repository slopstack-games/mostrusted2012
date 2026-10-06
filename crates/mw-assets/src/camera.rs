//! Gameplay cameras from a car's `VehicleCameraContainer` GeneSys object.
//!
//! Every vehicle bundle carries one container with four cameras: `External` (the chase camera,
//! `CameraGameplayExternal`), `Bonnet` and `Bumper` (`CameraGameplayBonnet` / `Bumper`, the same
//! type layout) and `Rear_view` (the mirror). Field names were recovered by dictionary attack on
//! their hashes; fields still unnamed keep their hash, and their meaning here is read from the values
//! across all 79 containers in the install (docs/handling.md, "Cameras", says which is which).
//!
//! Fields of a GeneSys struct are stored sorted by size and then by name (case-insensitive), which
//! bounds what an unnamed field can be called: in `Camera_Roll`, the two unknowns that track
//! `Minimum_Angular_Velocity_for_Roll` / `Minimum_Lateral_Acceleration_for_Roll` sort before
//! `Maximum_Roll_Angle`.
//!
//! Units: speeds are mph and accelerations mph per second (the handling data's convention),
//! angles degrees, positions metres in car space (+Z forward, +Y up).

use mw_formats::genesys::{Object, Value};

mod h {
    use mw_formats::genesys::field_hash;

    pub const BONNET: u32 = field_hash("Bonnet");
    pub const BUMPER: u32 = field_hash("Bumper");
    pub const EXTERNAL: u32 = field_hash("External");
    pub const REAR_VIEW: u32 = field_hash("Rear_view");

    // Shared by External / Bonnet / Bumper.
    pub const EYE: u32 = 0x6e08f788;
    pub const MIN_FOV: u32 = field_hash("MinFieldOfView");
    pub const MAX_FOV: u32 = field_hash("MaxFieldOfView");

    // CameraGameplayExternal
    pub const LOOK_AT: u32 = 0x006e1686;
    pub const PIVOT: u32 = 0xd68bbc59;
    pub const YAW_DRIFT_SPRING: u32 = field_hash("YawDriftSpring");
    pub const SPEED_MOVEMENT: u32 = 0x7540a8a7;
    pub const SPEED_Z: u32 = 0x0a78c15f;
    pub const SPEED_LOOK_Y: u32 = 0xb028673b;
    pub const SPEED_EYE_Y: u32 = 0x7d4ec535;
    pub const SPEED_FULL: u32 = 0x53e4b4f9;
    pub const SPEED_MIN: u32 = 0xb6010ee1;
    pub const HIGH_SPEED_SHAKE: u32 = field_hash("High_Speed_Shake");
    pub const AMPLITUDE: u32 = field_hash("Amplitude");
    pub const FREQUENCY: u32 = field_hash("Frequency");
    pub const POWER: u32 = field_hash("Power");
    pub const MIN_SPEED_FOR_SHAKE: u32 = field_hash("Minimum_Speed_for_Shake");
    pub const MAX_SPEED_FOR_SHAKE: u32 = field_hash("Maximum_Speed_for_Shake");
    pub const ACCELERATION_MOVEMENT: u32 = field_hash("Acceleration_movement");
    pub const ACCEL_BACK: u32 = 0xbd3a7cfc;
    pub const ACCEL_FORWARD: u32 = 0x7c9039e2;
    pub const ACCEL_RATE: u32 = 0xfdd25ee3;
    pub const BRAKE_RATE: u32 = 0x285f2260;
    pub const VERT_GAIN_UP: u32 = 0x8e1310f6;
    pub const VERT_GAIN_DOWN: u32 = 0xa8cc9c9b;
    pub const VERT_MIN: u32 = 0x65b4f9c6;
    pub const VERT_MAX: u32 = 0x0da33093;
    pub const CAMERA_ROLL: u32 = field_hash("Camera_Roll");
    pub const ROLL_FACTOR_POWER: u32 = field_hash("Camera_Roll_Factor_Power");
    pub const ROLL_MAX_ANGULAR: u32 = 0x02dd11bc;
    pub const ROLL_MAX_LATERAL: u32 = 0x3f8ced67;
    pub const MAX_ROLL_ANGLE: u32 = field_hash("Maximum_Roll_Angle");
    pub const ROLL_MIN_ANGULAR: u32 = field_hash("Minimum_Angular_Velocity_for_Roll");
    pub const ROLL_MIN_LATERAL: u32 = field_hash("Minimum_Lateral_Acceleration_for_Roll");
    pub const ROLL_INTENSITY: u32 = field_hash("Roll_Angle_Change_Intensity");
    pub const YAW_SPRING: u32 = field_hash("Yaw_spring_modification");
    pub const DECAY_RATE: u32 = field_hash("Decay_rate");
    pub const LOW_SPEED: u32 = field_hash("Low_speed");
    pub const HIGH_SPEED: u32 = field_hash("High_speed");
    pub const YAW_LOW_SPRING: u32 = 0x5a74d44d;
    pub const YAW_HIGH_SPRING: u32 = 0x63fb2207;
    pub const DECEL_PITCH: u32 = field_hash("Deceleration_pitch_change");
    pub const MAX_DECEL_FOR_PITCH: u32 = field_hash("Maximum_deceleration_for_pitch_change");
    pub const MIN_DECEL_FOR_PITCH: u32 = field_hash("Minimum_deceleration_for_pitch_change");
    pub const MAX_PITCH_CHANGE: u32 = field_hash("Maximum_Pitch_Change");
    pub const PITCH_CURVE_POWER: u32 = field_hash("Pitch_change_curve_power");
    pub const PITCH_INTENSITY: u32 = field_hash("Pitch_change_intensity");
    pub const GLOBAL_PARAMS: u32 = field_hash("Global_params");
    pub const IMPACT_SHAKE: u32 = field_hash("Impact_shake");
    pub const MAX_AMPLITUDE: u32 = field_hash("Maximum_amplitude");
    pub const SHAKE_EFFECT: u32 = 0xf50dccb9;
    pub const TRANSLATION: u32 = field_hash("Translation");
    pub const ROTATION: u32 = field_hash("Rotation");
    pub const TRANSLATION_AXES: [u32; 3] = [0x5e9f1ee7, 0x5a5e0251, 0x571d278b];
    pub const ROTATION_AXES: [u32; 3] = [field_hash("Pitch"), field_hash("Roll"), 0x41bd8729];
    pub const DAMPING: u32 = field_hash("Damping");
    pub const TRANSLATION_DAMPING: u32 = 0x00fd0a79;
    pub const SHAKE_MAGNITUDE: u32 = field_hash("ShakeMagnitude");
    pub const WOBBLE_MAGNITUDE: u32 = field_hash("WobbleMagnitude");
    pub const MAX_FREQ_MULT: u32 = field_hash("Maximum_frequency_multiplier");
    // Impact_shake fields without a cracked name, by their use in FUN_005cd040 / FUN_005dfe90.
    pub const IMPACT_POWER: u32 = 0xd4c1621b;
    pub const IMPACT_DECAY: u32 = 0x79f982f0;
    pub const IMPACT_SCALE: u32 = 0x192db931;
    pub const IMPACT_HI: u32 = 0xa2c5d2eb;
    pub const IMPACT_CAP: u32 = 0x4ffbfd8f;
    pub const IMPACT_LO: u32 = 0x96f848b2;
    pub const IMPACT_SPRING: u32 = 0x91b99f00;
    pub const SPRING_COEFFICIENT: u32 = field_hash("Spring_Coefficient");
    pub const MIN_ANGLE: u32 = field_hash("Minimum_Angle");
    pub const MAX_ANGLE: u32 = field_hash("Maximum_Angle");
    pub const LOWER_LIMIT: u32 = field_hash("Lower_Translation_Limit");
    pub const UPPER_LIMIT: u32 = field_hash("Upper_Translation_Limit");

    // CameraGameplayBonnet / Bumper
    pub const IN_CAR_BACK_EYE: u32 = 0x3f017c6b;

    // CameraRearView (+ CameraRearViewGlobals)
    pub const REAR_OFFSET: u32 = 0x272746a6;
    pub const REAR_HORIZONTAL_FOV: u32 = field_hash("Horizontal_FOV");
}

/// The cameras of one car.
#[derive(Debug, Clone, Copy)]
pub struct VehicleCameras {
    pub external: ExternalCamera,
    pub bonnet: Option<InCarCamera>,
    pub bumper: Option<InCarCamera>,
    pub rear_view: Option<RearViewCamera>,
}

/// `CameraGameplayExternal`: the chase camera.
#[derive(Debug, Clone, Copy)]
pub struct ExternalCamera {
    /// `6e08f788`: eye position (911: 0, 1.6, −4.6).
    pub eye: [f32; 3],
    /// `006e1686`: look-at point (911: 0, 1.2, 0.75).
    pub look_at: [f32; 3],
    /// `d68bbc59`: (0, 0, look-at z) on every car; used as the point the camera orbits (hypothesis).
    pub pivot: [f32; 3],
    /// `MinFieldOfView` / `MaxFieldOfView` (70 / 95 on every car).
    pub fov: [f32; 2],
    /// `YawDriftSpring` (0.045–0.07): yaw spring while drifting, a fraction per 60 Hz tick (hypothesis,
    /// like `DriftBehaviour`'s per-tick rates).
    pub yaw_drift_spring: f32,
    pub speed: SpeedMovement,
    pub shake: HighSpeedShake,
    pub acceleration: AccelerationMovement,
    pub roll: CameraRoll,
    pub yaw_spring: YawSpring,
    pub pitch: DecelerationPitch,
    pub impact: ImpactShake,
}

/// `Speed_based_movement` (unnamed fields; meanings from how they track the eye and look-at points).
#[derive(Debug, Clone, Copy)]
pub struct SpeedMovement {
    /// `0a78c15f` (−1.2…0): eye z change at full speed (m; negative = further back).
    pub eye_z: f32,
    /// `7d4ec535` (1.45…4.4, ≈ eye height): eye height at full speed.
    pub eye_y: f32,
    /// `b028673b` (1.35…3.1, ≈ look-at height + 0.25): look-at height at full speed.
    pub look_y: f32,
    /// `53e4b4f9` (50 / 75 / 100): speed (mph) at which the change is complete.
    pub full_speed: f32,
    /// `b6010ee1` (0 on every car): speed (mph) at which the change starts (`FUN_005d12b0`).
    pub min_speed: f32,
}

/// `Speed_based_movement.High_Speed_Shake`.
#[derive(Debug, Clone, Copy)]
pub struct HighSpeedShake {
    pub amplitude: f32,
    pub frequency: f32,
    /// `Minimum_Speed_for_Shake` / `Maximum_Speed_for_Shake` (mph; 0 / 125–208).
    pub speed: [f32; 2],
    pub power: f32,
}

/// `Acceleration_movement`: 11 floats; the four below are read by `FUN_005d12b0` (decomp.md "Cameras").
/// Sub-offsets: +0x10 `bd3a7cfc`, +0x14 `7c9039e2`, +0x18 `285f2260`, +0x2c `fdd25ee3`; the vertical
/// stage (the eye's height spring) reads +0x24 `8e1310f6`, +0x28 `65b4f9c6`, +0x34 `a8cc9c9b`, +0x38 `0da33093`.
#[derive(Debug, Clone, Copy)]
pub struct AccelerationMovement {
    /// `bd3a7cfc` (−2.5…−1.9): eye z offset at full forward acceleration (m, negative = back).
    pub back: f32,
    /// `7c9039e2` (0.5): eye z offset at full deceleration (m, forward).
    pub forward: f32,
    /// `fdd25ee3` (2): rate (1/s) the eye's z offset follows its target while accelerating.
    pub accel_rate: f32,
    /// `285f2260` (2.5): the same while decelerating.
    pub brake_rate: f32,
    /// `8e1310f6` (1.5) / `a8cc9c9b` (1.75): gain on the car's vertical acceleration driving the eye's
    /// height spring, while it is upwards / not upwards.
    pub vertical_gain: [f32; 2],
    /// `65b4f9c6` (−1.2) / `0da33093` (2.5): limits of the eye's height spring (m).
    pub vertical_limits: [f32; 2],
}

/// `Camera_Roll`.
#[derive(Debug, Clone, Copy)]
pub struct CameraRoll {
    /// `Camera_Roll_Factor_Power` (2.5).
    pub factor_power: f32,
    /// `Minimum_Lateral_Acceleration_for_Roll` (10) and `3f8ced67` (30–40, the maximum by its place
    /// and its values), m/s² (`FUN_005c9380`: the body velocity's change per update along the car's row 0).
    pub lateral: [f32; 2],
    /// `Minimum_Angular_Velocity_for_Roll` (0.2) and `02dd11bc` (2.5–3, the maximum), rad/s.
    pub angular: [f32; 2],
    /// `Maximum_Roll_Angle` (3–6°).
    pub max_angle: f32,
    /// `Roll_Angle_Change_Intensity` (2–2.5): the roll moves by intensity · π/180 · 20 · dt of its gap per
    /// update (`FUN_005c9380`).
    pub intensity: f32,
}

/// `Yaw_spring_modification`: how tightly the camera's yaw follows the car's.
#[derive(Debug, Clone, Copy)]
pub struct YawSpring {
    /// `Low_speed` / `High_speed` (0 / 100 mph).
    pub speed: [f32; 2],
    /// `5a74d44d` / `63fb2207` (0.075 / 0.05): the spring at low / high speed, a fraction per 60 Hz
    /// tick (hypothesis: the order puts `5a74d44d` after `Low_speed` and `63fb2207` after `High_speed`).
    pub spring: [f32; 2],
    /// `Decay_rate` (0.01).
    pub decay_rate: f32,
}

/// `Deceleration_pitch_change`: nose-down tilt under braking.
#[derive(Debug, Clone, Copy)]
pub struct DecelerationPitch {
    /// `Minimum_` / `Maximum_deceleration_for_pitch_change` (2 / 11–12 mph/s).
    pub deceleration: [f32; 2],
    /// `Maximum_Pitch_Change` (2.5–6°).
    pub max_change: f32,
    /// `Pitch_change_curve_power` (2.25).
    pub curve_power: f32,
    /// `Pitch_change_intensity` (1.75–2).
    pub intensity: f32,
}

/// One axis of `CameraGameplayShakeEffect`: a damped spring kicked by impacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShakeAxis {
    pub amplitude: f32,
    /// `Damping` (rotation) / `00fd0a79` (translation, by its place next to `Amplitude`).
    pub damping: f32,
    pub spring: f32,
    /// `Minimum_Angle` / `Maximum_Angle` (degrees) or `Lower_` / `Upper_Translation_Limit` (m).
    pub limits: [f32; 2],
}

/// `CameraGameplayExternalGlobals.Impact_shake` and its `CameraGameplayShakeEffect`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImpactShake {
    /// Three translation axes (only the third, taken as z, has an amplitude) and pitch / roll / yaw.
    pub translation: [ShakeAxis; 3],
    pub rotation: [ShakeAxis; 3],
    /// `Maximum_amplitude` (2): the shake level's multiplier into degrees (`FUN_005cd040`'s last argument).
    pub max_amplitude: f32,
    /// The noise shake stage `FUN_005dfe90` (decomp.md "Camera effects"): `WobbleMagnitude` (a random
    /// velocity impulse per update, ±), `ShakeMagnitude` (a random offset per update, ±), `91b99f00` (the
    /// spring / damping fraction) and `Maximum_frequency_multiplier` (the step: `× dt`).
    pub wobble: [f32; 3],
    pub jitter: [f32; 3],
    pub spring: f32,
    pub frequency: f32,
    /// The level (`FUN_005cd040`): decays by `79f982f0` per update; a contact `x` raises it to
    /// `min((clamp((x − lo) / (hi − lo), 0, 1) × x × scale)^power, cap)` with lo `96f848b2`, hi `a2c5d2eb`,
    /// scale `192db931`, power `d4c1621b`, cap `4ffbfd8f`.
    pub decay: f32,
    pub contact: [f32; 2],
    pub scale: f32,
    pub power: f32,
    pub cap: f32,
}

/// `CameraGameplayBonnet` / `Bumper`.
#[derive(Debug, Clone, Copy)]
pub struct InCarCamera {
    pub eye: [f32; 3],
    /// `3f017c6b`: a second point (bonnet: on the roof, bumper: at the rear); taken as the eye of the
    /// look-back view (hypothesis).
    pub back_eye: [f32; 3],
    pub fov: [f32; 2],
}

/// `CameraRearView`: the mirror.
#[derive(Debug, Clone, Copy)]
pub struct RearViewCamera {
    pub offset: [f32; 3],
    pub horizontal_fov: f32,
}

fn float(o: &Object, hash: u32) -> Option<f32> {
    match o.get(hash)? {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        _ => None,
    }
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn vec3(o: &Object, hash: u32) -> Option<[f32; 3]> {
    match o.get(hash)? {
        Value::Array(a) => match a.as_slice() {
            [Value::Float(x), Value::Float(y), Value::Float(z)] => Some([*x, *y, *z]),
            _ => None,
        },
        _ => None,
    }
}

impl VehicleCameras {
    /// Read a `Genesys.Gen.VehicleCameraContainer` object.
    pub fn from_container(c: &Object) -> Option<Self> {
        Some(Self {
            external: ExternalCamera::read(sub(c, h::EXTERNAL)?)?,
            bonnet: sub(c, h::BONNET).and_then(InCarCamera::read),
            bumper: sub(c, h::BUMPER).and_then(InCarCamera::read),
            rear_view: sub(c, h::REAR_VIEW).and_then(|r| {
                Some(RearViewCamera {
                    offset: vec3(r, h::REAR_OFFSET)?,
                    horizontal_fov: sub(r, h::GLOBAL_PARAMS).and_then(|g| float(g, h::REAR_HORIZONTAL_FOV))?,
                })
            }),
        })
    }
}

impl Default for ExternalCamera {
    /// The Porsche 911 Carrera S's values (the most common ones in the install).
    fn default() -> Self {
        let axis = |amplitude, damping, spring, limits| ShakeAxis { amplitude, damping, spring, limits };
        Self {
            eye: [0.0, 1.6, -4.6],
            look_at: [0.0, 1.2, 0.75],
            pivot: [0.0, 0.0, 0.75],
            fov: [70.0, 95.0],
            yaw_drift_spring: 0.055,
            speed: SpeedMovement { eye_z: -1.0, eye_y: 1.7, look_y: 1.5, full_speed: 50.0, min_speed: 0.0 },
            shake: HighSpeedShake { amplitude: 1.2, frequency: 1.0, speed: [0.0, 180.0], power: 1.5 },
            acceleration: AccelerationMovement {
                back: -2.0,
                forward: 0.5,
                accel_rate: 2.0,
                brake_rate: 2.5,
                vertical_gain: [1.5, 1.75],
                vertical_limits: [-1.2, 2.5],
            },
            roll: CameraRoll {
                factor_power: 2.5,
                lateral: [10.0, 30.0],
                angular: [0.2, 2.5],
                max_angle: 4.7,
                intensity: 2.0,
            },
            yaw_spring: YawSpring { speed: [0.0, 100.0], spring: [0.075, 0.05], decay_rate: 0.01 },
            pitch: DecelerationPitch { deceleration: [2.0, 12.0], max_change: 4.5, curve_power: 2.25, intensity: 2.0 },
            impact: ImpactShake {
                translation: [ShakeAxis::default(), ShakeAxis::default(), axis(100.0, 0.5, 100.0, [-0.2, 0.2])],
                rotation: [axis(250.0, 0.4, 450.0, [-20.0, 20.0]), ShakeAxis::default(), ShakeAxis::default()],
                max_amplitude: 2.0,
                wobble: [0.5; 3],
                jitter: [1.0; 3],
                spring: 1.0,
                frequency: 30.0,
                decay: 0.165,
                contact: [5.0, 60.0],
                scale: 1.0,
                power: 0.75,
                cap: 4.0,
            },
        }
    }
}

impl ExternalCamera {
    fn read(e: &Object) -> Option<Self> {
        let d = Self::default();
        let f = |o: Option<&Object>, hash, default: f32| o.and_then(|o| float(o, hash)).unwrap_or(default);
        let speed = sub(e, h::SPEED_MOVEMENT);
        let shake = speed.and_then(|s| sub(s, h::HIGH_SPEED_SHAKE));
        let accel = sub(e, h::ACCELERATION_MOVEMENT);
        let roll = sub(e, h::CAMERA_ROLL);
        let yaw = sub(e, h::YAW_SPRING);
        let pitch = sub(e, h::DECEL_PITCH);
        let globals = sub(e, h::GLOBAL_PARAMS);
        let impact = globals.and_then(|g| sub(g, h::IMPACT_SHAKE));
        let effect = globals.and_then(|g| sub(g, h::SHAKE_EFFECT));
        let axes = |group: u32, names: [u32; 3], damping: u32, lo: u32, hi: u32, defaults: [ShakeAxis; 3]| {
            let g = effect.and_then(|e| sub(e, group));
            let scale = f(g, h::AMPLITUDE, 1.0);
            let mut out = defaults;
            for (axis, name) in out.iter_mut().zip(names) {
                if let Some(a) = g.and_then(|g| sub(g, name)) {
                    *axis = ShakeAxis {
                        amplitude: f(Some(a), h::AMPLITUDE, 0.0) * scale,
                        damping: f(Some(a), damping, 1.0),
                        spring: f(Some(a), h::SPRING_COEFFICIENT, 1.0),
                        limits: [f(Some(a), lo, -1.0), f(Some(a), hi, 1.0)],
                    };
                }
            }
            out
        };
        Some(Self {
            eye: vec3(e, h::EYE)?,
            look_at: vec3(e, h::LOOK_AT).unwrap_or(d.look_at),
            pivot: vec3(e, h::PIVOT).unwrap_or(d.pivot),
            fov: [float(e, h::MIN_FOV)?, float(e, h::MAX_FOV)?],
            yaw_drift_spring: float(e, h::YAW_DRIFT_SPRING).unwrap_or(d.yaw_drift_spring),
            speed: SpeedMovement {
                eye_z: f(speed, h::SPEED_Z, d.speed.eye_z),
                eye_y: f(speed, h::SPEED_EYE_Y, d.speed.eye_y),
                look_y: f(speed, h::SPEED_LOOK_Y, d.speed.look_y),
                full_speed: f(speed, h::SPEED_FULL, d.speed.full_speed),
                min_speed: f(speed, h::SPEED_MIN, d.speed.min_speed),
            },
            shake: HighSpeedShake {
                amplitude: f(shake, h::AMPLITUDE, 0.0),
                frequency: f(shake, h::FREQUENCY, 1.0),
                speed: [f(shake, h::MIN_SPEED_FOR_SHAKE, 0.0), f(shake, h::MAX_SPEED_FOR_SHAKE, 180.0)],
                power: f(shake, h::POWER, 1.0),
            },
            acceleration: AccelerationMovement {
                back: f(accel, h::ACCEL_BACK, d.acceleration.back),
                forward: f(accel, h::ACCEL_FORWARD, d.acceleration.forward),
                accel_rate: f(accel, h::ACCEL_RATE, d.acceleration.accel_rate),
                brake_rate: f(accel, h::BRAKE_RATE, d.acceleration.brake_rate),
                vertical_gain: [
                    f(accel, h::VERT_GAIN_UP, d.acceleration.vertical_gain[0]),
                    f(accel, h::VERT_GAIN_DOWN, d.acceleration.vertical_gain[1]),
                ],
                vertical_limits: [
                    f(accel, h::VERT_MIN, d.acceleration.vertical_limits[0]),
                    f(accel, h::VERT_MAX, d.acceleration.vertical_limits[1]),
                ],
            },
            roll: CameraRoll {
                factor_power: f(roll, h::ROLL_FACTOR_POWER, d.roll.factor_power),
                lateral: [f(roll, h::ROLL_MIN_LATERAL, 10.0), f(roll, h::ROLL_MAX_LATERAL, 30.0)],
                angular: [f(roll, h::ROLL_MIN_ANGULAR, 0.2), f(roll, h::ROLL_MAX_ANGULAR, 2.5)],
                max_angle: f(roll, h::MAX_ROLL_ANGLE, 0.0),
                intensity: f(roll, h::ROLL_INTENSITY, d.roll.intensity),
            },
            yaw_spring: YawSpring {
                speed: [f(yaw, h::LOW_SPEED, 0.0), f(yaw, h::HIGH_SPEED, 100.0)],
                spring: [f(yaw, h::YAW_LOW_SPRING, 0.075), f(yaw, h::YAW_HIGH_SPRING, 0.05)],
                decay_rate: f(yaw, h::DECAY_RATE, 0.01),
            },
            pitch: DecelerationPitch {
                deceleration: [f(pitch, h::MIN_DECEL_FOR_PITCH, 2.0), f(pitch, h::MAX_DECEL_FOR_PITCH, 12.0)],
                max_change: f(pitch, h::MAX_PITCH_CHANGE, 0.0),
                curve_power: f(pitch, h::PITCH_CURVE_POWER, 1.0),
                intensity: f(pitch, h::PITCH_INTENSITY, 2.0),
            },
            impact: ImpactShake {
                translation: axes(
                    h::TRANSLATION,
                    h::TRANSLATION_AXES,
                    h::TRANSLATION_DAMPING,
                    h::LOWER_LIMIT,
                    h::UPPER_LIMIT,
                    d.impact.translation,
                ),
                rotation: axes(
                    h::ROTATION,
                    h::ROTATION_AXES,
                    h::DAMPING,
                    h::MIN_ANGLE,
                    h::MAX_ANGLE,
                    d.impact.rotation,
                ),
                max_amplitude: f(impact, h::MAX_AMPLITUDE, 2.0),
                wobble: impact.and_then(|i| vec3(i, h::WOBBLE_MAGNITUDE)).unwrap_or(d.impact.wobble),
                jitter: impact.and_then(|i| vec3(i, h::SHAKE_MAGNITUDE)).unwrap_or(d.impact.jitter),
                spring: f(impact, h::IMPACT_SPRING, d.impact.spring),
                frequency: f(impact, h::MAX_FREQ_MULT, d.impact.frequency),
                decay: f(impact, h::IMPACT_DECAY, d.impact.decay),
                contact: [f(impact, h::IMPACT_LO, d.impact.contact[0]), f(impact, h::IMPACT_HI, d.impact.contact[1])],
                scale: f(impact, h::IMPACT_SCALE, d.impact.scale),
                power: f(impact, h::IMPACT_POWER, d.impact.power),
                cap: f(impact, h::IMPACT_CAP, d.impact.cap),
            },
        })
    }
}

impl InCarCamera {
    fn read(c: &Object) -> Option<Self> {
        let eye = vec3(c, h::EYE)?;
        Some(Self {
            eye,
            back_eye: vec3(c, h::IN_CAR_BACK_EYE).unwrap_or(eye),
            fov: [float(c, h::MIN_FOV)?, float(c, h::MAX_FOV)?],
        })
    }
}
