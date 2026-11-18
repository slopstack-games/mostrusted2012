//! Where the game puts a car's wheels (docs/decomp.md "Wheel placement", research log 2026-10-10).
//!
//! Neither the `VehicleGraphicsSpec` wheel positions nor the physics `RigidBody` wheel transforms are where
//! the game draws or simulates a wheel: on many cars they hold placeholders (y 0.311 / 0.316 on the Challenger,
//! Crown Victoria, Caterham; 0.325 on every traffic car; the Venom's front axle at z 0.125). The vehicle build
//! (`FUN_009b40c0`, called by `FUN_009b4e60`) overwrites each wheel body's position from the car's `VehicleInfo`
//! (the physical definition's `AdditionalInfo`), axle by axle:
//!
//! - x = `SuspensionWheel.dcbe38af` (each of the axle's `1048818a` wheel handles),
//! - y = `VehicleInfo.caac331c` + R − `Axle.2f5fb582`,
//! - z = `Axle.eaee225f`,
//!
//! with R the wheel body's cylinder radius (`RigidBodies[i].CylinderVolumes[0].Radius`, the body named by the
//! `SuspensionWheel.Child`). The spring is built at that geometry (rest length = distance + W / k), so at rest
//! the wheel sits there, and each frame the wheel is drawn at its physics body's centre in car space
//! (`FUN_005798a0`, `FUN_00902930`). Checked live on the F-150 Raptor (probe dump, 2026-10-10): the drawn
//! wheel centres are (±0.925, 0.354, 1.675) and (±0.93, 0.324, −1.725), the rule's values, against the spec's
//! (±0.935, 0.436, 1.68 / −1.72).
//!
//! The strut runs from y = `caac331c` + `WheelSuspensionConstraint.b867a203` (top, constraint +0x24) down to
//! `caac331c` + `7fc0dc41` (bottom, +0x14), the constraint named by `SuspensionWheel.Suspension_Constraint`. Read
//! live on the Raptor (strut object +0x40 / +0x50, body frame = model − 0.75): top −0.222 / −0.202, bottom
//! −0.512 / −0.557, so 0.174 / 0.224 m of bump travel above the wheel's rest; on the XKR (snapshot snap1) 0.056 /
//! 0.067 m, both by this rule (2026-10-19).
//!
//! The strut joint's builder (`FUN_009c3650`, from `FUN_009b40c0` through `FUN_00933790`) takes the same
//! constraint's `Angle` (`efd741ee`, +0x10, degrees), arm length `c2a87807` (+0x20) and arm flag `b1c680d5` (+0x28):
//! [`RestWheel::strut_joint`]. The spring element's chassis anchor (element +0x20) sits at the wheel's x and z,
//! y = `caac331c` + `WheelSuspensionSpringConstraint.02f99a20` (+0x1c of the wheel's spring constraint, read in
//! `FUN_009b40c0`; snap1: the XKR's anchor at 0.22 in the body frame = 0.143 + 0.527 − 0.45 ✓):
//! [`RestWheel::spring_anchor`] (2026-11-18).
//!
//! Traffic (kinematic, `FUN_005ee7f0`) uses the same axles with the front-left radius for every wheel and no
//! spring: (x, R_FL, z) − T, T = the translation of `VehicleInfo.509931cd`. The tyre's bottom then sits at
//! y = −T.y in model space, as the player's does at rest (`caac331c` − `2f5fb582` ≈ −T.y on every car).

use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;

const NAMES: u32 = 0x6d10_b948;
const INFO_REFERENCE: u32 = 0xcaac_331c;
const INFO_FRAME: u32 = 0x5099_31cd;
const AXLE_GROUND: u32 = 0x2f5f_b582;
const AXLE_Z: u32 = 0xeaee_225f;
const AXLE_WHEELS: u32 = 0x1048_818a;
const WHEEL_X: u32 = 0xdcbe_38af;
const HALF_EXTENTS: u32 = 0x98e9_02ed;
/// `WheelSuspensionConstraint` +0x24 / +0x14: the strut's top and bottom above `caac331c`.
const STRUT_TOP: u32 = 0xb867_a203;
const STRUT_BOTTOM: u32 = 0x7fc0_dc41;
/// `WheelSuspensionConstraint` +0x10 / +0x20 / +0x28: the strut's tilt (degrees), arm length and arm flag.
const STRUT_ANGLE: u32 = 0xefd7_41ee;
const STRUT_ARM: u32 = 0xc2a8_7807;
const STRUT_ARM_ON: u32 = 0xb1c6_80d5;
/// `WheelSuspensionSpringConstraint` +0x1c: the spring's chassis anchor above `caac331c`.
const SPRING_ANCHOR: u32 = 0x02f9_9a20;
const SPRING_TYPE: &str = "Genesys.Gen.WheelSuspensionSpringConstraint";

/// One wheel where the game places it.
#[derive(Debug, Clone)]
pub struct RestWheel {
    /// Index into the physical definition's `RigidBodies` (0 is the chassis).
    pub body: usize,
    /// The body's name (`FrontLeft.WheelRigidBody`, ...).
    pub name: String,
    /// Wheel centre at rest, car (model) space.
    pub position: [f32; 3],
    /// Cylinder radius of the wheel body.
    pub radius: f32,
    /// The strut's top and bottom, model-space y (`None` without the constraint's data).
    pub strut: Option<[f32; 2]>,
    /// The strut joint's shape from the same constraint (`None` without it).
    pub strut_joint: Option<StrutJointData>,
    /// The spring element's chassis anchor, model-space y (`None` without the spring constraint's data).
    pub spring_anchor: Option<f32>,
}

/// The `WheelSuspensionConstraint` fields the strut joint's builder `FUN_009c3650` takes besides the strut's ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrutJointData {
    /// `Angle` (`efd741ee`): the strut's tilt, degrees.
    pub angle: f32,
    /// `c2a87807`: the arm's length (m), from the strut's middle to its pivot.
    pub arm: f32,
    /// `b1c680d5`: the wheel moves on the arm's arc (joint +0x80), not along the strut.
    pub arm_on: bool,
}

#[derive(Debug, Clone)]
pub struct WheelLayout {
    pub wheels: Vec<RestWheel>,
    /// `VehicleInfo.509931cd` rows and translation: the traffic pose's frame.
    pub frame: [[f32; 4]; 4],
}

impl WheelLayout {
    /// The radius every traffic wheel is placed with: the front-left body's (`FUN_005ee7f0`).
    fn traffic_radius(&self) -> Option<f32> {
        self.wheels.iter().find(|w| w.name.starts_with("FrontLeft")).or(self.wheels.first()).map(|w| w.radius)
    }

    /// Wheel centres of a kinematic traffic car, model space: R·((x, R_FL, z) − T).
    pub fn traffic_positions(&self) -> Vec<[f32; 3]> {
        let r = self.traffic_radius().unwrap_or(0.33);
        let m = &self.frame;
        self.wheels
            .iter()
            .map(|w| {
                let p = [w.position[0] - m[3][0], r - m[3][1], w.position[2] - m[3][2]];
                std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c])
            })
            .collect()
    }

    /// Height of the model origin above the road under a traffic car (T.y).
    pub fn traffic_ride_height(&self) -> f32 {
        self.frame[3][1]
    }
}

fn float(o: &Object, hash: u32) -> Option<f32> {
    match o.get(hash)? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn array(o: &Object, hash: u32) -> &[Value] {
    match o.get(hash) {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

fn handle(v: Option<&Value>) -> Option<mw_bundle::ResourceId> {
    match v? {
        Value::Handle(Some(id)) | Value::Ref(id) => Some(*id),
        _ => None,
    }
}

/// The wheel body's radius: its cylinder volume, else the body's half extent across (they agree on every car).
fn body_radius(body: &Object) -> Option<f32> {
    let cylinder = array(body, field_hash("CylinderVolumes")).iter().find_map(|c| match c {
        Value::Struct(c) => float(c, field_hash("Radius")),
        _ => None,
    });
    cylinder.or_else(|| match body.get(HALF_EXTENTS)? {
        Value::Vector(v) => Some(v[0]),
        _ => None,
    })
}

impl AssetStore {
    /// The game's wheel layout for a `RaceCarPhysicalDefinition` (see the module docs); `None` when the data
    /// is missing (then the spec's positions stand).
    pub fn wheel_layout(&self, rcpd: &Object) -> Option<WheelLayout> {
        let Some(Value::Struct(def)) = rcpd.get(field_hash("PhysicalDefinition")) else { return None };
        let names: Vec<&str> = array(def, NAMES)
            .iter()
            .map(|v| match v {
                Value::Str(s) => s.as_str(),
                _ => "",
            })
            .collect();
        let bodies = array(def, field_hash("RigidBodies"));
        let info = self.genesys_object(handle(def.get(field_hash("AdditionalInfo")))?).ok()?;
        let reference = float(&info, INFO_REFERENCE)?;
        let frame = match info.get(INFO_FRAME) {
            Some(Value::Matrix(m)) => *m,
            _ => [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0; 4]],
        };
        let mut wheels = Vec::new();
        for axle in array(&info, field_hash("Axles")) {
            let Value::Struct(axle) = axle else { continue };
            let (Some(ground), Some(z)) = (float(axle, AXLE_GROUND), float(axle, AXLE_Z)) else { continue };
            for h in array(axle, AXLE_WHEELS) {
                let Some(wheel) = handle(Some(h)).and_then(|id| self.genesys_object(id).ok()) else { continue };
                let (Some(Value::Str(child)), Some(x)) = (wheel.get(field_hash("Child")), float(&wheel, WHEEL_X))
                else {
                    continue;
                };
                let Some(body) = names.iter().position(|n| n == child) else { continue };
                let Some(Value::Struct(b)) = bodies.get(body) else { continue };
                let Some(radius) = body_radius(b) else { continue };
                let constraint =
                    handle(wheel.get(field_hash("Suspension_Constraint"))).and_then(|id| self.genesys_object(id).ok());
                let strut = constraint
                    .as_ref()
                    .and_then(|c| Some([reference + float(c, STRUT_TOP)?, reference + float(c, STRUT_BOTTOM)?]));
                let strut_joint = constraint.as_ref().and_then(|c| {
                    Some(StrutJointData {
                        angle: float(c, STRUT_ANGLE)?,
                        arm: float(c, STRUT_ARM)?,
                        arm_on: matches!(c.get(STRUT_ARM_ON), Some(Value::Bool(true))),
                    })
                });
                // The wheel's spring constraint: whichever of its handles names one.
                let spring_anchor = wheel.fields.iter().find_map(|(_, v)| {
                    let c = self.genesys_object(handle(Some(v))?).ok()?;
                    if c.type_name == SPRING_TYPE { float(&c, SPRING_ANCHOR).map(|a| reference + a) } else { None }
                });
                wheels.push(RestWheel {
                    body,
                    name: child.clone(),
                    position: [x, reference + radius - ground, z],
                    radius,
                    strut,
                    strut_joint,
                    spring_anchor,
                });
            }
        }
        (!wheels.is_empty()).then_some(WheelLayout { wheels, frame })
    }

    /// The wheel layout of a car bundle: its own physical definition (`_HI` bundles), or for a traffic bundle
    /// (`VEH_<component>_TR`, no GeneSys of its own) the one the traffic attributes hold for the component, whose
    /// id is the graphics spec's low 32 bits with 0x0100_0002 above (as `traffic_vehicle` finds it).
    pub fn vehicle_wheel_layout(&self, bundle: &mw_bundle::Bundle) -> Option<WheelLayout> {
        for e in bundle.entries_of_type(mw_bundle::ResourceType::GenesysObject) {
            let Ok(res) = bundle.load(e) else { continue };
            let Ok(obj) = self.decode_genesys(&res) else { continue };
            if obj.type_name == "Genesys.Gen.RaceCarPhysicalDefinition" {
                return self.wheel_layout(&obj);
            }
        }
        let spec = bundle.entries_of_type(mw_bundle::ResourceType::VehicleGraphicsSpec).next()?;
        let component = spec.id.0 & 0xffff_ffff;
        let rcpd = self.genesys_object(mw_bundle::ResourceId(0x0100_0002_0000_0000 | component)).ok()?;
        self.wheel_layout(&rcpd)
    }
}

/// Move each spec wheel to the matching layout position: the one on the same side (sign of x) and axle (front
/// = the larger z). Some specs label sides or axles the other way round, so names are not used. Returns how
/// many wheels moved.
pub fn place_spec_wheels(spec: &mut mw_formats::VehicleGraphicsSpec, positions: &[[f32; 3]]) -> usize {
    let key = |p: [f32; 3], mid: f32| (p[0] < 0.0, p[2] > mid);
    let mid = |zs: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = zs.fold((0.0, 0), |(s, n), z| (s + z, n + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    };
    let spec_mid = mid(&mut spec.wheels.iter().map(|w| w.position[2]));
    let layout_mid = mid(&mut positions.iter().map(|p| p[2]));
    let mut moved = 0;
    for w in &mut spec.wheels {
        let k = key(w.position, spec_mid);
        if let Some(p) = positions.iter().find(|p| key(**p, layout_mid) == k) {
            w.position = *p;
            moved += 1;
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_wheels_match_by_side_and_axle() {
        let wheel = |name: &str, p: [f32; 3]| mw_formats::Wheel {
            name: name.into(),
            position: p,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            parts: Vec::new(),
        };
        // The Venom's front axle at z 0.125 in the spec; the layout puts it at 1.3.
        let mut spec = mw_formats::VehicleGraphicsSpec {
            parts: Vec::new(),
            skeleton: None,
            collision: None,
            wheels: vec![
                wheel("FrontLeft", [0.8, 0.334, 0.125]),
                wheel("FrontRight", [-0.8, 0.334, 0.125]),
                wheel("RearLeft", [0.81, 0.356, -1.505]),
                wheel("RearRight", [-0.81, 0.356, -1.505]),
            ],
            locator_groups: Vec::new(),
        };
        let layout = [[0.8, 0.32, 1.3], [-0.8, 0.32, 1.3], [0.81, 0.35, -1.5], [-0.81, 0.35, -1.5]];
        assert_eq!(place_spec_wheels(&mut spec, &layout), 4);
        assert_eq!(spec.wheels[1].position, [-0.8, 0.32, 1.3]);
        assert_eq!(spec.wheels[2].position, [0.81, 0.35, -1.5]);
    }
}
