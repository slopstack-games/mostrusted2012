//! Vehicle handling parameters extracted from a car's GeneSys data.
//!
//! GeneSys field names are stored as hashes (names of four characters or
//! fewer appear verbatim, e.g. `Mass` = 0x4d617373). The meanings below were
//! inferred by comparing cars with well-known real specs; see
//! `docs/handling.md` for the evidence and confidence of each mapping.

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value};

use crate::{AssetStore, Result};

/// Field name hashes (see docs/handling.md).
mod h {
    // RaceCarPhysicalDefinition.PhysicalDefinition.RigidBody
    pub const RB_TRANSFORM: u32 = 0x0d0c29b4;
    pub const RB_HALF_EXTENTS: u32 = 0x98e902ed;
    pub const RB_MASS: u32 = 0x4d617373; // "Mass"
    pub const PD_BODIES: u32 = 0x535a06b7;
    pub const RCPD_DEFINITION: u32 = 0xf373ff3e;
    // Engine
    pub const ENGINE: u32 = 0xde711f43;
    pub const POWER_CURVE: u32 = 0xa1a3b0a2;
    pub const CURVE_POINTS: u32 = 0xfd4709e4;
    pub const POINT_XY: u32 = 0x44f09125;
    pub const POINT_IN: u32 = 0x3e827829;
    pub const POINT_OUT: u32 = 0xdb35bfa5;
    pub const IDLE_RPM: u32 = 0xcc887670;
    pub const REDLINE_RPM: u32 = 0x1cdde238;
    pub const TOP_SPEED_MPH: u32 = 0xf2368dfe;
    pub const ENGINE_BRAKE: u32 = 0x4623dd88;
    pub const DIFFERENTIALS: u32 = 0x65e56fe5;
    pub const FRONT_SHARE: u32 = 0xbef03791;
    pub const GEARBOX_A: u32 = 0xfd49acb2;
    pub const GEARBOX_B: u32 = 0x2b64bdcf;
    // Gearbox
    pub const REVERSE: u32 = 0xd38b2bfc;
    pub const FINAL_DRIVE: u32 = 0xad14f0ff;
    pub const GEARS: u32 = 0xfe57124a;
    pub const GEAR_DOWN_RPM: u32 = 0x8f085ebd;
    pub const GEAR_RATIO: u32 = 0xe8ce7d74;
    pub const GEAR_UP_RPM: u32 = 0xc80f787a;
    // HandlingBehaviour
    pub const BRAKES: u32 = 0x96a12a3b;
    pub const BRAKE_FRONT: u32 = 0x6ade4bb6;
    pub const BRAKE_REAR: u32 = 0x66251131;
    pub const HANDBRAKE: u32 = 0xacf4df7d;
    pub const HANDBRAKE_TORQUE: u32 = 0x4af55f8a;
    pub const TYRES: u32 = 0xa316f8e3;
    pub const LONG_GRIP_PEAK: u32 = 0x66676f77;
    // WheelSuspensionSpringConstraint
    pub const SPRING_DAMPING: u32 = 0x87e422a9;
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

/// Everything the physics model takes from the game. Fields are `None` when
/// a car lacks the data (and the model falls back to defaults).
#[derive(Debug, Clone, Default)]
pub struct VehicleHandling {
    pub mass: Option<f32>,
    /// Body rigid-body origin in car space (used as the centre of mass).
    pub centre_of_mass: Option<[f32; 3]>,
    pub body_half_extents: Option<[f32; 3]>,
    pub wheels: Vec<WheelBody>,
    pub torque_curve: Vec<CurvePoint>,
    pub idle_rpm: Option<f32>,
    pub redline_rpm: Option<f32>,
    pub top_speed_mph: Option<f32>,
    /// Engine braking torque (Nm, negative).
    pub engine_brake_torque: Option<f32>,
    /// Share of drive torque sent to the front axle (0 = RWD).
    pub front_drive_share: Option<f32>,
    pub gearbox: Option<Gearbox>,
    pub brake_torque_front: Option<f32>,
    pub brake_torque_rear: Option<f32>,
    pub handbrake_torque: Option<f32>,
    /// Peak longitudinal friction coefficient of the first grip curve.
    pub tyre_long_peak: Option<f32>,
    /// Front, rear.
    pub springs: Vec<Spring>,
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
            self.front_drive_share = sub(engine, h::DIFFERENTIALS).and_then(|d| float(d, h::FRONT_SHARE));
            // Two gearboxes per car; the second shifts near the redline and
            // is used for the player (hypothesis, see docs/handling.md).
            let gb = sub(engine, h::GEARBOX_B).or_else(|| sub(engine, h::GEARBOX_A));
            self.gearbox = gb.map(|g| {
                let mut gearbox = Gearbox {
                    ratios: Vec::new(),
                    reverse: sub(g, h::REVERSE).and_then(|r| float(r, h::GEAR_RATIO)).unwrap_or(-3.0),
                    final_drive: float(g, h::FINAL_DRIVE).unwrap_or(3.5),
                    upshift_rpm: Vec::new(),
                    downshift_rpm: Vec::new(),
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
        self.tyre_long_peak = sub(hb, h::TYRES).and_then(first_long_peak);
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

fn first_long_peak(tyres: &Object) -> Option<f32> {
    fn walk(o: &Object) -> Option<f32> {
        if o.type_name.ends_with("Long_Grip_Curve") {
            return float(o, h::LONG_GRIP_PEAK);
        }
        o.fields.iter().find_map(|(_, v)| match v {
            Value::Struct(s) => walk(s),
            Value::Array(a) => a.iter().find_map(|v| if let Value::Struct(s) = v { walk(s) } else { None }),
            _ => None,
        })
    }
    walk(tyres)
}
