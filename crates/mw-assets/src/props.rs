//! Physics of world objects: props, compound objects and dynamic objects.
//!
//! Every `PropObject`, most `WorldObject`s and every `CompoundObject` point at a GeneSys
//! `PhysicalDefinition` (`mPhysicalDefinitionHandle`). It holds one or more rigid bodies, each with a
//! local transform, mass, friction, drag and a set of primitive collision volumes (boxes, spheres,
//! cylinders, capsules, convex hulls) in the body's frame. The definition's `AdditionalInfo` handle
//! points at a `DynamicAdditionalInfo` (props, dynamic objects) or `CompoundAdditionalInfo`
//! (compound objects) with the forces that damage / smash the object.
//!
//! Field names: `Mass`, `Friction`, `LinearDrag`, `AngularDrag`, `Inertia_Scale`, `RigidBodies`,
//! `BoxVolumes`..., `Radius`, `HalfLength`, `ComOffset`, `AdditionalInfo`, `ForceToDamage`,
//! `ForceToSmash`, `Strength`, `Score` are name-backed (hash matches); the box half extents
//! (`3727fead`), the volume / body transforms (`d084ac7c` / `0d0c29b4`) and the bounds (`62dbd683`
//! centre, `98e902ed` half extents) are identified from their values (the bounds enclose the volumes).
//! See docs/formats.md, "World object physics".

use mw_bundle::ResourceId;
use mw_formats::ObjectDef;
use mw_formats::genesys::{Object, Value, field_hash};

use crate::{AssetStore, Result};

/// Which instance list an object came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// `PropInstanceList` → `PropObject`: street furniture (cones, bins, barrels, signs, benches).
    Prop,
    /// `CompoundInstanceList` → `CompoundObject`: multi-part objects (lamp posts, traffic lights).
    Compound,
    /// `DynamicInstanceList` → `WorldObject`.
    Dynamic,
}

/// A primitive collision volume in its rigid body's frame. Transforms are row-major (`v * M`).
#[derive(Debug, Clone, PartialEq)]
pub enum Volume {
    Box {
        transform: [[f32; 4]; 4],
        half_extents: [f32; 3],
    },
    Sphere {
        transform: [[f32; 4]; 4],
        radius: f32,
    },
    /// Axis along the volume's local Z (a flat disc of radius 0.93 and `HalfLength` 0.25 lies with
    /// its Z row pointing up, matching the definition's bounds).
    Cylinder {
        transform: [[f32; 4]; 4],
        radius: f32,
        half_length: f32,
    },
    /// Axis along local Z, like the cylinder (assumed: no capsule appears on a world object).
    Capsule {
        transform: [[f32; 4]; 4],
        radius: f32,
        half_length: f32,
    },
    /// A `ConvexHull` resource (not decoded; the body's bounds stand in for it).
    Hull {
        transform: [[f32; 4]; 4],
        hull: Option<ResourceId>,
    },
}

/// One rigid body of a [`PhysicalDefinition`].
#[derive(Debug, Clone, PartialEq)]
pub struct RigidBodyDef {
    /// Body frame in object space (`0d0c29b4`).
    pub transform: [[f32; 4]; 4],
    pub com_offset: [f32; 3],
    pub inertia_scale: [f32; 3],
    /// Bounds in the body frame: centre and half extents.
    pub bounds: ([f32; 3], [f32; 3]),
    pub mass: f32,
    pub friction: f32,
    /// `44ac37c1` (0.1–0.65): probably restitution. Unnamed.
    pub restitution: f32,
    pub linear_drag: f32,
    pub angular_drag: f32,
    pub volumes: Vec<Volume>,
}

/// `DynamicAdditionalInfo` / `CompoundAdditionalInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AdditionalInfo {
    pub force_to_damage: f32,
    /// `67f0233c` (0, 10, 500, 5000, 10000): unnamed, a force threshold like its neighbours.
    pub force_67f0233c: f32,
    pub strength: f32,
    pub score: u32,
    /// Compound objects only.
    pub force_to_smash: Option<f32>,
}

/// A decoded `PhysicalDefinition`.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalDefinition {
    pub id: ResourceId,
    /// Object-space bounds: centre and half extents.
    pub bounds: ([f32; 3], [f32; 3]),
    pub bodies: Vec<RigidBodyDef>,
    pub info: Option<AdditionalInfo>,
}

/// Physics of one object definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPhysics {
    pub object: ResourceId,
    pub kind: ObjectKind,
    pub definition: Option<PhysicalDefinition>,
    /// Compound objects: the part definitions (in part order; `None` where a part has none).
    pub parts: Vec<Option<PhysicalDefinition>>,
    /// Props: distance (m) beyond which a knocked prop is put back (`mfResetDistanceSquared`).
    pub reset_distance: Option<f32>,
    pub remove_instead_of_reset: bool,
}

impl ObjectPhysics {
    /// Total mass of the main definition (kg).
    pub fn mass(&self) -> f32 {
        self.definition.as_ref().map_or(0.0, |d| d.bodies.iter().map(|b| b.mass).sum())
    }
}

const RIGID_BODIES: u32 = field_hash("RigidBodies");
const ADDITIONAL_INFO: u32 = field_hash("AdditionalInfo");
const BOUNDS_CENTRE: u32 = 0x62dbd683;
const BOUNDS_HALF: u32 = 0x98e902ed;
const BODY_TRANSFORM: u32 = 0x0d0c29b4;
const VOLUME_TRANSFORM: u32 = 0xd084ac7c;
const BOX_HALF_EXTENTS: u32 = 0x3727fead;
const RESTITUTION: u32 = 0x44ac37c1;
const HULL: u32 = 0xa954f748;
const CONVEX_HULL_VOLUMES: u32 = 0xaa22a10b;
const FORCE_67F0233C: u32 = 0x67f0233c;

fn vec3(o: &Object, hash: u32) -> Option<[f32; 3]> {
    match o.get(hash)? {
        Value::Vector(v) => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn float(o: &Object, name: u32) -> Option<f32> {
    match o.get(name)? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn uint(o: &Object, name: u32) -> Option<u64> {
    match o.get(name)? {
        Value::UInt(v) => Some(*v),
        Value::Int(v) => u64::try_from(*v).ok(),
        _ => None,
    }
}

fn matrix(o: &Object, hash: u32) -> [[f32; 4]; 4] {
    match o.get(hash) {
        Some(Value::Matrix(m)) => *m,
        _ => crate::IDENTITY,
    }
}

fn structs(o: &Object, hash: u32) -> impl Iterator<Item = &Object> {
    let items: &[Value] = match o.get(hash) {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    items.iter().filter_map(|v| if let Value::Struct(s) = v { Some(s) } else { None })
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(id) => Some(*id),
        _ => None,
    }
}

fn volumes(body: &Object) -> Vec<Volume> {
    let mut out = Vec::new();
    for v in structs(body, field_hash("BoxVolumes")) {
        let Some(h) = vec3(v, BOX_HALF_EXTENTS) else { continue };
        out.push(Volume::Box { transform: matrix(v, VOLUME_TRANSFORM), half_extents: h });
    }
    for v in structs(body, field_hash("SphereVolumes")) {
        let Some(radius) = float(v, field_hash("Radius")) else { continue };
        out.push(Volume::Sphere { transform: matrix(v, VOLUME_TRANSFORM), radius });
    }
    for (name, capsule) in [("CylinderVolumes", false), ("CapsuleVolumes", true)] {
        for v in structs(body, field_hash(name)) {
            let (Some(radius), Some(half_length)) =
                (float(v, field_hash("Radius")), float(v, field_hash("HalfLength")))
            else {
                continue;
            };
            let transform = matrix(v, VOLUME_TRANSFORM);
            out.push(if capsule {
                Volume::Capsule { transform, radius, half_length }
            } else {
                Volume::Cylinder { transform, radius, half_length }
            });
        }
    }
    for v in structs(body, CONVEX_HULL_VOLUMES) {
        out.push(Volume::Hull { transform: matrix(v, VOLUME_TRANSFORM), hull: handle(v, HULL) });
    }
    out
}

impl AssetStore {
    /// Decode a `PhysicalDefinition` GeneSys object and its `AdditionalInfo`.
    pub fn physical_definition(&self, id: ResourceId) -> Result<PhysicalDefinition> {
        let o = self.genesys_object(id)?;
        let bodies = structs(&o, RIGID_BODIES)
            .map(|b| RigidBodyDef {
                transform: matrix(b, BODY_TRANSFORM),
                com_offset: vec3(b, field_hash("ComOffset")).unwrap_or_default(),
                inertia_scale: vec3(b, field_hash("Inertia_Scale")).unwrap_or([1.0; 3]),
                bounds: (vec3(b, BOUNDS_CENTRE).unwrap_or_default(), vec3(b, BOUNDS_HALF).unwrap_or_default()),
                mass: float(b, field_hash("Mass")).unwrap_or(0.0),
                friction: float(b, field_hash("Friction")).unwrap_or(0.8),
                restitution: float(b, RESTITUTION).unwrap_or(0.1),
                linear_drag: float(b, field_hash("LinearDrag")).unwrap_or(0.0),
                angular_drag: float(b, field_hash("AngularDrag")).unwrap_or(0.0),
                volumes: volumes(b),
            })
            .collect();
        let info = handle(&o, ADDITIONAL_INFO).and_then(|h| self.genesys_object(h).ok()).map(|i| AdditionalInfo {
            force_to_damage: float(&i, field_hash("ForceToDamage")).unwrap_or(0.0),
            force_67f0233c: float(&i, FORCE_67F0233C).unwrap_or(0.0),
            strength: float(&i, field_hash("Strength")).unwrap_or(0.0),
            score: uint(&i, field_hash("Score")).unwrap_or(0) as u32,
            force_to_smash: float(&i, field_hash("ForceToSmash")),
        });
        Ok(PhysicalDefinition {
            id,
            bounds: (vec3(&o, BOUNDS_CENTRE).unwrap_or_default(), vec3(&o, BOUNDS_HALF).unwrap_or_default()),
            bodies,
            info,
        })
    }

    /// The object definition behind a prop / compound / dynamic placement.
    pub fn object_def(&self, object: ResourceId, kind: ObjectKind) -> Result<ObjectDef> {
        let res = self.load(object)?;
        Ok(match kind {
            ObjectKind::Prop => ObjectDef::parse_prop(&res)?,
            ObjectKind::Compound => ObjectDef::parse_compound(&res)?,
            ObjectKind::Dynamic => ObjectDef::parse_world_object(&res)?,
        })
    }

    /// Physics of an object definition. Definitions that fail to decode are left out (`None`).
    pub fn object_physics(&self, object: ResourceId, kind: ObjectKind) -> Result<ObjectPhysics> {
        let def = self.object_def(object, kind)?;
        let load = |id: Option<ResourceId>| id.and_then(|id| self.physical_definition(id).ok());
        Ok(ObjectPhysics {
            object,
            kind,
            definition: load(def.physics),
            parts: def.parts.iter().map(|p| load(*p)).collect(),
            reset_distance: def.reset_distance_sq.filter(|d| *d > 0.0).map(f32::sqrt),
            remove_instead_of_reset: def.remove_instead_of_reset,
        })
    }
}
