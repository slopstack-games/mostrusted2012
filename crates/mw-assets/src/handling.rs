//! Vehicle handling parameters extracted from a car's GeneSys data.
//!
//! GeneSys stores field names as hashes ([`field_hash`]); most handling fields
//! have been named (`Peak_Slip_Angle`, `Low_speed_steering_range`, ...). The
//! units were checked against well-known real specs; see `docs/handling.md`.

use mw_bundle::{Bundle, ResourceId, ResourceType};
#[cfg(doc)]
use mw_formats::genesys::field_hash;
use mw_formats::genesys::{Object, Value};

use crate::camera::VehicleCameras;
use crate::{AssetStore, Result};

/// Field name hashes. Names come from [`field_hash`]; fields whose name is
/// still unknown keep their raw hash (see docs/handling.md).
mod h {
    use mw_formats::genesys::field_hash;

    // RaceCarPhysicalDefinition.PhysicalDefinition.RigidBody
    pub const RB_TRANSFORM: u32 = 0x0d0c29b4;
    pub const RB_HALF_EXTENTS: u32 = 0x98e902ed;
    pub const RB_INERTIA_SCALE: u32 = field_hash("Inertia_Scale");
    pub const RB_MASS: u32 = field_hash("Mass");
    pub const PD_BODIES: u32 = field_hash("RigidBodies");
    pub const RCPD_DEFINITION: u32 = field_hash("PhysicalDefinition");
    // Engine
    pub const ENGINE: u32 = field_hash("Engine");
    pub const POWER_CURVE: u32 = field_hash("Power_curve");
    pub const CURVE_POINTS: u32 = field_hash("TorqueCurve");
    pub const POINT_XY: u32 = 0x44f09125;
    pub const POINT_IN: u32 = field_hash("TangentIn");
    pub const POINT_OUT: u32 = field_hash("TangentOut");
    pub const IDLE_RPM: u32 = field_hash("Idle_RPM");
    pub const REDLINE_RPM: u32 = field_hash("Max_RPM");
    pub const TOP_SPEED_MPH: u32 = field_hash("Top_speed_MPH");
    pub const ENGINE_BRAKE: u32 = field_hash("Engine_resistance");
    pub const ENGINE_INERTIA: u32 = field_hash("Engine_inertia");
    pub const DIFFERENTIALS: u32 = field_hash("Differentials");
    pub const FRONT_SHARE: u32 = field_hash("Power_To_Front");
    pub const CRUISING_GEARBOX: u32 = field_hash("CruisingGearbox");
    pub const RACE_GEARBOX: u32 = field_hash("RaceGearbox");
    // Gearbox
    pub const REVERSE: u32 = field_hash("Reverse_gear");
    pub const FINAL_DRIVE: u32 = field_hash("Differential");
    pub const GEARS: u32 = field_hash("Gears");
    pub const GEAR_CHANGE_TIME: u32 = field_hash("Gear_change_time");
    pub const GEAR_DOWN_RPM: u32 = field_hash("Gear_down_RPM");
    pub const GEAR_RATIO: u32 = field_hash("Gear_ratio");
    pub const GEAR_UP_RPM: u32 = field_hash("Gear_up_RPM");
    // HandlingBehaviour
    pub const AERO: u32 = field_hash("Aerodynamics");
    pub const IN_AIR_GRAVITY: u32 = field_hash("In_air_gravity_coefficient");
    pub const BRAKES: u32 = field_hash("Brakes");
    pub const BRAKE_FRONT: u32 = 0x6ade4bb6;
    pub const BRAKE_REAR: u32 = 0x66251131;
    pub const HANDBRAKE: u32 = field_hash("Handbrake");
    pub const HANDBRAKE_TORQUE: u32 = 0x4af55f8a;
    pub const STEERING: u32 = field_hash("Steering");
    pub const TYRES: u32 = field_hash("Tyres");
    // Tyres
    pub const FRONT_TYRES: u32 = field_hash("Front_tyres");
    pub const REAR_TYRES: u32 = field_hash("Rear_tyres");
    pub const FRONT_LAT_SCALE: u32 = field_hash("Front_lateral_scale");
    pub const FRONT_LONG_SCALE: u32 = field_hash("Front_longitudinal_scale");
    pub const REAR_LAT_SCALE: u32 = field_hash("Rear_lateral_scale");
    pub const REAR_LONG_SCALE: u32 = field_hash("Rear_longitudinal_scale");
    pub const PURE_LAT: u32 = field_hash("Pure_Lateral_Grip");
    pub const BRAKE_SLIP_LAT: u32 = field_hash("Brake_Slip_Lateral_Grip");
    pub const TRACTION_SLIP_LAT: u32 = field_hash("Traction_Slip_Lateral_Grip");
    pub const BRAKE_PURE_LONG: u32 = field_hash("Brake_Pure_Long_Grip");
    pub const TRACTION_PURE_LONG: u32 = field_hash("Traction_Pure_Long_Grip");
    pub const BRAKE_SLIP_LONG: u32 = field_hash("Brake_Slip_Long_Grip");
    pub const TRACTION_SLIP_LONG: u32 = field_hash("Traction_Slip_Long_Grip");
    pub const GRADIENT: u32 = field_hash("Gradient");
    pub const PEAK_FORCE: u32 = field_hash("Peak_Force_Coefficient");
    pub const PEAK_SLIP_ANGLE: u32 = field_hash("Peak_Slip_Angle");
    pub const PEAK_SLIP_RATIO: u32 = field_hash("Peak_Slip_Ratio");
    pub const MAX_SLIP_FORCE: u32 = field_hash("Max_Slip_Force_Coefficient");
    pub const MAX_SLIP_RATIO: u32 = field_hash("Max_Slip_Ratio");
    // SteeringBehaviour
    pub const MEDIUM_SPEED: u32 = field_hash("Medium_speed");
    pub const HIGH_SPEED: u32 = field_hash("High_speed");
    pub const LOW_RANGE: u32 = field_hash("Low_speed_steering_range");
    pub const HIGH_RANGE: u32 = field_hash("High_speed_steering_range");
    pub const REVERSE_RANGE: u32 = field_hash("Reverse_steering_range");
    pub const LOW_RATE: u32 = field_hash("Low_speed_steer_rate");
    pub const HIGH_RATE: u32 = field_hash("High_speed_steer_rate");
    pub const REVERSE_RATE: u32 = field_hash("Reverse_steer_rate");
    pub const LOW_COUNTER: u32 = field_hash("Low_speed_counter_steer_rate");
    pub const HIGH_COUNTER: u32 = field_hash("High_speed_counter_steer_rate");
    pub const REVERSE_COUNTER: u32 = field_hash("Reverse_counter_steer_rate");
    pub const LOW_CENTER: u32 = field_hash("Low_speed_center_steer_rate");
    pub const HIGH_CENTER: u32 = field_hash("High_speed_center_steer_rate");
    pub const REVERSE_CENTER: u32 = field_hash("Reverse_center_steer_rate");
    // WheelSuspensionSpringConstraint
    pub const SPRING_DAMPING: u32 = field_hash("Damping_Ratio");
    pub const SPRING_FREQUENCY: u32 = 0x0c5f89b3;
    pub const SPRING_TRAVEL: u32 = 0x41543f85;
}

#[derive(Debug, Clone, Copy)]
pub struct CurvePoint {
    pub rpm: f32,
    pub torque: f32,
    /// Hermite tangents (Δrpm, Δtorque) in and out.
    pub tangent_in: [f32; 2],
    pub tangent_out: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct Gearbox {
    /// Forward gear ratios, first gear first.
    pub ratios: Vec<f32>,
    pub reverse: f32,
    pub final_drive: f32,
    pub upshift_rpm: Vec<f32>,
    pub downshift_rpm: Vec<f32>,
    /// Downshift points of the cruising gearbox (same ratios, early shifts):
    /// where the box shifts down off the throttle.
    pub coast_downshift_rpm: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct WheelBody {
    /// Wheel centre in car space.
    pub position: [f32; 3],
    pub radius: f32,
    pub half_width: f32,
    pub mass: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    /// Natural frequency in Hz.
    pub frequency: f32,
    pub damping_ratio: f32,
    /// Suspension travel in metres.
    pub travel: f32,
}

/// A lateral grip curve: force coefficient against slip angle. It rises from
/// zero with slope `gradient` (per degree) to `peak` at `peak_angle` degrees
/// with a flat tangent (a cubic Hermite; on most cars `gradient` is exactly
/// 3 × peak / angle, the steepest monotonic case), and stays there.
#[derive(Debug, Clone, Copy)]
pub struct LateralCurve {
    pub gradient: f32,
    pub peak: f32,
    pub peak_angle: f32,
}

impl LateralCurve {
    /// Force coefficient at a slip angle in degrees (sign follows the angle).
    pub fn at(&self, angle_deg: f32) -> f32 {
        let a = self.peak_angle.max(0.1);
        let t = (angle_deg.abs() / a).min(1.0);
        let m0 = self.gradient * a;
        let (t2, t3) = (t * t, t * t * t);
        let f = (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * self.peak;
        f.max(0.0) * angle_deg.signum()
    }
}

/// A longitudinal grip curve: `peak` at slip ratio `peak_ratio`, then
/// `max_slip` at `max_slip_ratio` (wheelspin or lock-up) and beyond.
#[derive(Debug, Clone, Copy)]
pub struct LongCurve {
    pub peak: f32,
    pub peak_ratio: f32,
    pub max_slip: f32,
    pub max_slip_ratio: f32,
}

/// One axle's tyres. "Pure" curves apply with no slip in the other direction;
/// "slip" curves apply while the tyre also slides the other way (lateral grip
/// while braking or spinning up, longitudinal grip while sliding sideways).
#[derive(Debug, Clone, Copy)]
pub struct TyreAxle {
    pub pure_lateral: LateralCurve,
    pub brake_slip_lateral: LateralCurve,
    pub traction_slip_lateral: LateralCurve,
    pub brake_pure_long: LongCurve,
    pub traction_pure_long: LongCurve,
    pub brake_slip_long: LongCurve,
    pub traction_slip_long: LongCurve,
    pub lateral_scale: f32,
    pub longitudinal_scale: f32,
}

/// `SteeringBehaviour`: wheel angle range and steering speed in two speed
/// bands (below `medium_speed` and above `high_speed`, in mph) and reverse.
/// Angles in degrees, rates in degrees per second.
#[derive(Debug, Clone, Copy)]
pub struct Steering {
    pub medium_speed_mph: f32,
    pub high_speed_mph: f32,
    /// (low speed, high speed, reverse) for each quantity.
    pub range: [f32; 3],
    pub rate: [f32; 3],
    /// Steering towards the other side.
    pub counter_rate: [f32; 3],
    /// Returning to centre with no input.
    pub center_rate: [f32; 3],
}

/// Everything the physics model takes from the game. Fields are `None` when
/// a car lacks the data (and the model falls back to defaults).
#[derive(Debug, Clone, Default)]
pub struct VehicleHandling {
    pub mass: Option<f32>,
    /// Body rigid-body origin in car space (used as the centre of mass).
    pub centre_of_mass: Option<[f32; 3]>,
    pub body_half_extents: Option<[f32; 3]>,
    /// Per-axis multiplier on the body's inertia.
    pub inertia_scale: Option<[f32; 3]>,
    pub wheels: Vec<WheelBody>,
    pub torque_curve: Vec<CurvePoint>,
    pub idle_rpm: Option<f32>,
    pub redline_rpm: Option<f32>,
    pub top_speed_mph: Option<f32>,
    /// Engine braking torque (Nm, negative).
    pub engine_brake_torque: Option<f32>,
    /// Engine rotating inertia (kg m²).
    pub engine_inertia: Option<f32>,
    /// Share of drive torque sent to the front axle (0 = RWD).
    pub front_drive_share: Option<f32>,
    pub gearbox: Option<Gearbox>,
    pub brake_torque_front: Option<f32>,
    pub brake_torque_rear: Option<f32>,
    pub handbrake_torque: Option<f32>,
    /// Front, rear.
    pub tyres: Option<[TyreAxle; 2]>,
    pub steering: Option<Steering>,
    /// Time without drive while changing gear (s).
    pub gear_change_time: Option<f32>,
    /// Gravity multiplier while airborne.
    pub in_air_gravity: Option<f32>,
    /// The car's gameplay cameras (`VehicleCameraContainer`).
    pub camera: Option<VehicleCameras>,
    /// Front, rear.
    pub springs: Vec<Spring>,
    /// Every numeric field of `HandlingBehaviour`, by path of field hashes
    /// (array elements are skipped). Read with [`VehicleHandling::param`].
    pub params: std::collections::HashMap<Vec<u32>, f32>,
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

fn items(o: &Object, hash: u32) -> &[Value] {
    match o.get(hash) {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

fn pair(o: &Object, hash: u32) -> Option<[f32; 2]> {
    match items(o, hash) {
        [Value::Float(a), Value::Float(b)] => Some([*a, *b]),
        _ => None,
    }
}

impl VehicleHandling {
    /// Read the handling data from a vehicle's `_HI` bundle.
    pub fn from_bundle(store: &AssetStore, bundle: &Bundle) -> Result<Self> {
        let mut out = Self::default();
        let mut springs: Vec<(u32, Spring)> = Vec::new();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(obj) = store.decode_genesys(&bundle.load(e)?) else { continue };
            match obj.type_name.as_str() {
                "Genesys.Gen.RaceCarPhysicalDefinition" => out.read_bodies(&obj),
                "Genesys.Gen.HandlingBehaviour" => out.read_handling(&obj),
                "Genesys.Gen.VehicleCameraContainer" => out.camera = VehicleCameras::from_container(&obj),
                "Genesys.Gen.WheelSuspensionSpringConstraint" => {
                    if let (Some(frequency), Some(damping_ratio), Some(travel)) = (
                        float(&obj, h::SPRING_FREQUENCY),
                        float(&obj, h::SPRING_DAMPING),
                        float(&obj, h::SPRING_TRAVEL),
                    ) {
                        springs.push((e.id.0 as u32, Spring { frequency, damping_ratio, travel }));
                    }
                }
                _ => {}
            }
        }
        // Lower resource IDs belong to the front axle in every car checked.
        springs.sort_by_key(|(id, _)| *id);
        out.springs = springs.into_iter().map(|(_, s)| s).collect();
        Ok(out)
    }

    fn read_bodies(&mut self, rcpd: &Object) {
        let Some(def) = sub(rcpd, h::RCPD_DEFINITION) else { return };
        for (i, body) in items(def, h::PD_BODIES).iter().enumerate() {
            let Value::Struct(b) = body else { continue };
            let pos = match b.get(h::RB_TRANSFORM) {
                Some(Value::Matrix(m)) => [m[3][0], m[3][1], m[3][2]],
                _ => continue,
            };
            let ext = match b.get(h::RB_HALF_EXTENTS) {
                Some(Value::Vector(v)) => [v[0], v[1], v[2]],
                _ => continue,
            };
            let mass = float(b, h::RB_MASS);
            if i == 0 {
                self.mass = mass;
                if let Some(Value::Vector(v)) = b.get(h::RB_INERTIA_SCALE) {
                    self.inertia_scale = Some([v[0], v[1], v[2]]);
                }
                self.centre_of_mass = Some(pos);
                self.body_half_extents = Some(ext);
            } else {
                self.wheels.push(WheelBody {
                    position: pos,
                    radius: ext[0],
                    half_width: ext[2],
                    mass: mass.unwrap_or(25.0),
                });
            }
        }
    }

    fn read_handling(&mut self, hb: &Object) {
        collect_params(hb, &mut Vec::new(), &mut self.params);
        if let Some(engine) = sub(hb, h::ENGINE) {
            if let Some(curve) = sub(engine, h::POWER_CURVE) {
                self.idle_rpm = float(curve, h::IDLE_RPM);
                self.redline_rpm = float(curve, h::REDLINE_RPM);
                for p in items(curve, h::CURVE_POINTS) {
                    let Value::Struct(p) = p else { continue };
                    let Some([rpm, torque]) = pair(p, h::POINT_XY) else { continue };
                    let tangent = |hash| sub(p, hash).and_then(|t| pair(t, h::POINT_XY)).unwrap_or([0.0, 0.0]);
                    self.torque_curve.push(CurvePoint {
                        rpm,
                        torque,
                        tangent_in: tangent(h::POINT_IN),
                        tangent_out: tangent(h::POINT_OUT),
                    });
                }
            }
            self.top_speed_mph = float(engine, h::TOP_SPEED_MPH);
            self.engine_brake_torque = float(engine, h::ENGINE_BRAKE);
            self.engine_inertia = float(engine, h::ENGINE_INERTIA);
            self.front_drive_share = sub(engine, h::DIFFERENTIALS).and_then(|d| float(d, h::FRONT_SHARE));
            // Two gearboxes per car: "Cruising" (early shifts, slow changes)
            // and "Race" (shifts near the limiter); the player uses the race one.
            let gb = sub(engine, h::RACE_GEARBOX).or_else(|| sub(engine, h::CRUISING_GEARBOX));
            let downshifts = |g: &Object| -> Vec<f32> {
                items(g, h::GEARS)
                    .iter()
                    .filter_map(|gear| match gear {
                        Value::Struct(gear) if float(gear, h::GEAR_RATIO).unwrap_or(0.0) > 0.0 => {
                            Some(float(gear, h::GEAR_DOWN_RPM).unwrap_or(0.0))
                        }
                        _ => None,
                    })
                    .collect()
            };
            let coast = sub(engine, h::CRUISING_GEARBOX).map(downshifts).unwrap_or_default();
            self.gear_change_time = gb.and_then(|g| float(g, h::GEAR_CHANGE_TIME));
            self.gearbox = gb.map(|g| {
                let mut gearbox = Gearbox {
                    ratios: Vec::new(),
                    reverse: sub(g, h::REVERSE).and_then(|r| float(r, h::GEAR_RATIO)).unwrap_or(-3.0),
                    final_drive: float(g, h::FINAL_DRIVE).unwrap_or(3.5),
                    upshift_rpm: Vec::new(),
                    downshift_rpm: Vec::new(),
                    coast_downshift_rpm: coast.clone(),
                };
                for gear in items(g, h::GEARS) {
                    let Value::Struct(gear) = gear else { continue };
                    let ratio = float(gear, h::GEAR_RATIO).unwrap_or(0.0);
                    if ratio <= 0.0 {
                        continue; // neutral
                    }
                    gearbox.ratios.push(ratio);
                    gearbox.upshift_rpm.push(float(gear, h::GEAR_UP_RPM).unwrap_or(0.0));
                    gearbox.downshift_rpm.push(float(gear, h::GEAR_DOWN_RPM).unwrap_or(0.0));
                }
                gearbox
            });
        }
        if let Some(b) = sub(hb, h::BRAKES) {
            self.brake_torque_front = float(b, h::BRAKE_FRONT);
            self.brake_torque_rear = float(b, h::BRAKE_REAR);
        }
        self.handbrake_torque = sub(hb, h::HANDBRAKE).and_then(|b| float(b, h::HANDBRAKE_TORQUE));
        self.tyres = sub(hb, h::TYRES).and_then(read_tyres);
        self.steering = sub(hb, h::STEERING).and_then(read_steering);
        self.in_air_gravity = sub(hb, h::AERO).and_then(|a| float(a, h::IN_AIR_GRAVITY));
    }

    /// A `HandlingBehaviour` field by dotted path, e.g.
    /// `"Drift.Yaw_torque.Maximum_yaw_torque"`. Unnamed fields are written as
    /// their eight-digit hex hash (`"Drift.Drift_trigger.8e4032c0"`).
    pub fn param(&self, path: &str) -> Option<f32> {
        let key: Vec<u32> = path
            .split('.')
            .map(|c| match u32::from_str_radix(c, 16) {
                Ok(h) if c.len() == 8 => h,
                _ => mw_formats::genesys::field_hash(c),
            })
            .collect();
        self.params.get(&key).copied()
    }

    /// Engine torque (Nm) at `rpm`, Hermite-interpolated along the curve.
    pub fn torque_at(&self, rpm: f32) -> f32 {
        let c = &self.torque_curve;
        match c.len() {
            0 => 0.0,
            1 => c[0].torque,
            _ => {
                if rpm <= c[0].rpm {
                    return c[0].torque;
                }
                let Some(i) = c.windows(2).position(|w| rpm <= w[1].rpm) else {
                    return c[c.len() - 1].torque;
                };
                let (a, b) = (&c[i], &c[i + 1]);
                let span = (b.rpm - a.rpm).max(1.0);
                let t = (rpm - a.rpm) / span;
                // Tangents are given as (Δrpm, Δtorque); convert to slopes.
                let slope = |tan: [f32; 2]| if tan[0].abs() > 1e-3 { tan[1] / tan[0] } else { 0.0 };
                let (m0, m1) = (slope(a.tangent_out) * span, slope(b.tangent_in) * span);
                let (t2, t3) = (t * t, t * t * t);
                (2.0 * t3 - 3.0 * t2 + 1.0) * a.torque
                    + (t3 - 2.0 * t2 + t) * m0
                    + (-2.0 * t3 + 3.0 * t2) * b.torque
                    + (t3 - t2) * m1
            }
        }
    }

    /// Find the handling data for a vehicle bundle path.
    pub fn for_vehicle(store: &AssetStore, bundle: &str) -> Result<Self> {
        let b = store.bundle(bundle).ok_or(crate::Error::Missing(ResourceId::NULL))?;
        Self::from_bundle(store, &b)
    }
}

fn collect_params(o: &Object, path: &mut Vec<u32>, out: &mut std::collections::HashMap<Vec<u32>, f32>) {
    for (hash, v) in &o.fields {
        path.push(*hash);
        match v {
            Value::Float(f) => _ = out.insert(path.clone(), *f),
            Value::Int(i) => _ = out.insert(path.clone(), *i as f32),
            Value::UInt(u) => _ = out.insert(path.clone(), *u as f32),
            Value::Bool(b) => _ = out.insert(path.clone(), if *b { 1.0 } else { 0.0 }),
            Value::Struct(s) => collect_params(s, path, out),
            _ => {}
        }
        path.pop();
    }
}

fn read_tyres(tyres: &Object) -> Option<[TyreAxle; 2]> {
    // `Tyres` holds a `Tyres.Tyres` with the two axles, plus per-axle scales.
    let axles = sub(tyres, h::TYRES)?;
    let lat = |t: &Object, hash| {
        let c = sub(t, hash)?;
        Some(LateralCurve {
            gradient: float(c, h::GRADIENT)?,
            peak: float(c, h::PEAK_FORCE)?,
            peak_angle: float(c, h::PEAK_SLIP_ANGLE)?,
        })
    };
    let long = |t: &Object, hash| {
        let c = sub(t, hash)?;
        Some(LongCurve {
            peak: float(c, h::PEAK_FORCE)?,
            peak_ratio: float(c, h::PEAK_SLIP_RATIO)?,
            max_slip: float(c, h::MAX_SLIP_FORCE)?,
            max_slip_ratio: float(c, h::MAX_SLIP_RATIO)?,
        })
    };
    let axle = |hash, lat_scale, long_scale| {
        let t = sub(axles, hash)?;
        Some(TyreAxle {
            pure_lateral: lat(t, h::PURE_LAT)?,
            brake_slip_lateral: lat(t, h::BRAKE_SLIP_LAT)?,
            traction_slip_lateral: lat(t, h::TRACTION_SLIP_LAT)?,
            brake_pure_long: long(t, h::BRAKE_PURE_LONG)?,
            traction_pure_long: long(t, h::TRACTION_PURE_LONG)?,
            brake_slip_long: long(t, h::BRAKE_SLIP_LONG)?,
            traction_slip_long: long(t, h::TRACTION_SLIP_LONG)?,
            lateral_scale: float(tyres, lat_scale).unwrap_or(1.0),
            longitudinal_scale: float(tyres, long_scale).unwrap_or(1.0),
        })
    };
    Some([
        axle(h::FRONT_TYRES, h::FRONT_LAT_SCALE, h::FRONT_LONG_SCALE)?,
        axle(h::REAR_TYRES, h::REAR_LAT_SCALE, h::REAR_LONG_SCALE)?,
    ])
}

fn read_steering(s: &Object) -> Option<Steering> {
    let f = |hash| float(s, hash);
    Some(Steering {
        medium_speed_mph: f(h::MEDIUM_SPEED)?,
        high_speed_mph: f(h::HIGH_SPEED)?,
        range: [f(h::LOW_RANGE)?, f(h::HIGH_RANGE)?, f(h::REVERSE_RANGE)?],
        rate: [f(h::LOW_RATE)?, f(h::HIGH_RATE)?, f(h::REVERSE_RATE)?],
        counter_rate: [f(h::LOW_COUNTER)?, f(h::HIGH_COUNTER)?, f(h::REVERSE_COUNTER)?],
        center_rate: [f(h::LOW_CENTER)?, f(h::HIGH_CENTER)?, f(h::REVERSE_CENTER)?],
    })
}
