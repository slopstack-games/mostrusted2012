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
    /// +0x18: the contact force (N) that swaps in the damaged state and starts the impact effect
    /// (`FUN_0054bf70`, once per knocked lifetime).
    pub force_to_damage: f32,
    /// +0x1c `67f0233c` (unnamed; 0, 10, 500, 5000, 10000): the **momentum** (kg m/s) a hitting body must
    /// carry to knock the object loose: `|mass × velocity|` of the hitter, its whole velocity
    /// (`FUN_0054e570`, run from the bounding-box overlap test `FUN_00554d80`).
    pub force_67f0233c: f32,
    pub strength: f32,
    pub score: u32,
    /// +0x26 `5586d09d`: can be knocked loose at all (false on the 100000 / mass-0 objects; `FUN_0054e570`
    /// returns at once without it).
    pub knockable: bool,
    /// +0x28 `94c94bab`: starts pinned (`FUN_0054dfe0` copies it into the pinned set; true exactly where the
    /// threshold is above 0).
    pub starts_pinned: bool,
    /// Compound objects only: +0x30 `ForceToSmash` (N, against the contact force) and +0x34 `08c87eeb`, the
    /// time (s) a knocked compound waits before it can smash (`FUN_00555c90`).
    pub force_to_smash: Option<f32>,
    pub smash_delay: Option<f32>,
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
    /// Props: the impact effect (`BearEffect`) and sound (`WCPlaySoundBehaviour`).
    pub impact_effect: Option<ResourceId>,
    pub impact_sound: Option<ResourceId>,
    /// Compound objects: how the object breaks into its parts.
    pub compound: Option<CompoundPhysics>,
}

/// How a compound object comes apart (docs/decomp.md, "Props": compound smash). The compound stands and
/// falls as one body (its own `PhysicalDefinition`) until it smashes; then its parts replace it, each a body
/// of its own placed at its skeleton bone (`FUN_00554500`, `FUN_00532b00`). Parts whose `replaces` is set
/// appear only when that part smashes in turn (its `SmashProxyBehaviour`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundPhysics {
    pub parts: Vec<CompoundPartPhysics>,
    /// `EnableCompoundBehaviour` with `543527d0` (+0x1a) set: the compound is split into its parts as soon as
    /// it streams in (`FUN_00554230` queues it for `FUN_00558bb0`).
    pub enabled_at_start: bool,
    /// Joint definitions between parts (GeneSys; `FUN_005547f0` → `FUN_0054e120` reads the two part hashes
    /// at +0x10 / +0x14 and a value at +0x18). Not ported.
    pub joints: Vec<ResourceId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompoundPartPhysics {
    pub definition: Option<PhysicalDefinition>,
    /// The part's frame in the compound's object space: its skeleton bone's bind pose (row-major, `v * M`;
    /// `FUN_005321b0` builds it from the bone's quaternion and position).
    pub bone: [[f32; 4]; 4],
    /// Skeleton bone index (+0x60).
    pub bone_index: u32,
    pub replaces: Option<usize>,
    pub next: Option<usize>,
    pub first_replacement: Option<usize>,
    pub links: Vec<usize>,
    /// `SmashProxyBehaviour.ForceToSmash` (`e7f6608b`, N): the contact force that replaces this part by its
    /// replacement chain.
    pub smash_force: Option<f32>,
    /// The part's impact sound (its first `WCPlaySoundBehaviour`).
    pub impact_sound: Option<ResourceId>,
}

/// Row-major matrix of a bone's bind pose (unit quaternion `x y z w`, position).
fn bone_matrix(q: [f32; 4], p: [f32; 3]) -> [[f32; 4]; 4] {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0],
        [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0],
        [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0],
        [p[0], p[1], p[2], 1.0],
    ]
}

/// `WCPlaySoundBehaviour` type id.
const PLAY_SOUND_BEHAVIOUR: u32 = 0x2ad8;

impl ObjectPhysics {
    /// Total mass of the main definition (kg).
    pub fn mass(&self) -> f32 {
        self.definition.as_ref().map_or(0.0, |d| d.bodies.iter().map(|b| b.mass).sum())
    }
}

/// How a car's contact with a dynamic object (a knocked prop) is solved: the share of the contact impulse
/// each side receives, linear and angular (`CollisionResponses_RaceCar.RaceCarVsDynamic`,
/// `CollisionResponses_Global.TrafficVsDynamic`). The game's contact solver (`FUN_009be420`, the friction
/// pass `FUN_009bed70`) computes the impulse from the relative velocity and adds `impulse × scale` to each
/// body's accumulator: with 0.1 / 0.05 a race car hardly feels a lamp post it knocks down (docs/decomp.md,
/// "Props").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicResponse {
    /// The car's linear and angular impulse scales.
    pub car_linear: f32,
    pub car_angular: f32,
    /// The object's.
    pub object_linear: f32,
    pub object_angular: f32,
    pub restitution: f32,
}

/// The responses per kind of car (`FUN_00960f20` picks them by the pair's categories, `FUN_00929350`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicResponses {
    /// Category 0, the player's race car (`FUN_00941170`, `RaceCarVsDynamic` at +0x40c, field `c7d59471`).
    pub player: DynamicResponse,
    /// Category 1, AI race cars (`FUN_009415f0`, +0x3d0, `2e3c3d30`).
    pub ai: DynamicResponse,
    /// Category 3, traffic (`FUN_00940d50`, `CollisionResponses_Global` +0x9c, `09f6c358`).
    pub traffic: DynamicResponse,
}

impl Default for DynamicResponses {
    /// The values in `PHYSICS/PHYSICSCONFIG.BNDL` (read 2026-10-09), for runs that cannot load it.
    fn default() -> Self {
        let r = |car_linear, car_angular, object_angular| DynamicResponse {
            car_linear,
            car_angular,
            object_linear: 1.0,
            object_angular,
            restitution: 0.0,
        };
        Self { player: r(0.1, 0.05, 0.4), ai: r(0.01, 0.01, 0.4), traffic: r(1.0, 0.55, 0.4) }
    }
}

const COLLISION_RESPONSES_RACE_CAR: ResourceId = ResourceId(0x0100_0000_0007_f0c9);
const COLLISION_RESPONSES_GLOBAL: ResourceId = ResourceId(0x0100_0000_0007_f0c6);

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

fn boolean(o: &Object, name: u32) -> Option<bool> {
    match o.get(name)? {
        Value::Bool(b) => Some(*b),
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
            knockable: boolean(&i, 0x5586d09d).unwrap_or(true),
            starts_pinned: boolean(&i, 0x94c94bab).unwrap_or(true),
            force_to_smash: float(&i, field_hash("ForceToSmash")),
            smash_delay: float(&i, 0x08c87eeb),
        });
        Ok(PhysicalDefinition {
            id,
            bounds: (vec3(&o, BOUNDS_CENTRE).unwrap_or_default(), vec3(&o, BOUNDS_HALF).unwrap_or_default()),
            bodies,
            info,
        })
    }

    /// The car-against-dynamic-object contact responses (`PHYSICS/PHYSICSCONFIG.BNDL`). Field offsets from the
    /// readers: `RaceCarVsDynamic` +0x34 → the car's linear scale, +0x30 its angular one, +0x2c the object's
    /// linear, +0x28 its angular, +0x38 `Restitution` (`FUN_00941170` / `FUN_009415f0`); `TrafficVsDynamic`
    /// +0x20, +0x1c, +0x14, +0x10, +0x18 (`FUN_00940d50`).
    pub fn dynamic_responses(&self) -> Result<DynamicResponses> {
        let race = self.genesys_object(COLLISION_RESPONSES_RACE_CAR)?;
        let global = self.genesys_object(COLLISION_RESPONSES_GLOBAL)?;
        let sub = |o: &Object, id: ResourceId, hash: u32| -> Result<Object> {
            match o.get(hash) {
                Some(Value::Struct(s)) => Ok(s.clone()),
                _ => Err(crate::Error::Missing(id)),
            }
        };
        let f = |o: &Object, hash: u32| float(o, hash).unwrap_or(0.0);
        let race_car = |o: Object| DynamicResponse {
            car_linear: f(&o, 0xe75eb28b),
            car_angular: f(&o, 0x207258c8),
            object_linear: f(&o, 0x357a73bd),
            object_angular: f(&o, 0x1caa1e4f),
            restitution: f(&o, field_hash("Restitution")),
        };
        let t = sub(&global, COLLISION_RESPONSES_GLOBAL, 0x09f6c358)?;
        Ok(DynamicResponses {
            player: race_car(sub(&race, COLLISION_RESPONSES_RACE_CAR, 0xc7d59471)?),
            ai: race_car(sub(&race, COLLISION_RESPONSES_RACE_CAR, 0x2e3c3d30)?),
            traffic: DynamicResponse {
                car_linear: f(&t, 0x16c25fa9),
                car_angular: f(&t, 0x31e69a76),
                object_linear: f(&t, 0x357a73bd),
                object_angular: f(&t, 0x1caa1e4f),
                restitution: f(&t, field_hash("Restitution")),
            },
        })
    }

    /// The waves of a prop's impact sound (`WCPlaySoundBehaviour.Collision_Waves`, e.g.
    /// "PlayConeCollisionSound": six cone knocks). The per-surface lists (`Surface_Collisions`) are not used.
    pub fn impact_sound_waves(&self, id: ResourceId) -> Result<Vec<ResourceId>> {
        let o = self.genesys_object(id)?;
        let waves: &[Value] = match o.get(field_hash("Collision_Waves")) {
            Some(Value::Array(a)) => a,
            _ => &[],
        };
        Ok(waves
            .iter()
            .filter_map(|v| match v {
                Value::Handle(h) => *h,
                Value::Ref(id) => Some(*id),
                _ => None,
            })
            .collect())
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
        let parts: Vec<Option<PhysicalDefinition>> = def.parts.iter().map(|p| load(*p)).collect();
        let compound = (kind == ObjectKind::Compound && !def.compound_parts.is_empty())
            .then(|| self.compound_physics(&def, &parts))
            .transpose()?;
        Ok(ObjectPhysics {
            object,
            kind,
            definition: load(def.physics),
            parts,
            reset_distance: def.reset_distance_sq.filter(|d| *d > 0.0).map(f32::sqrt),
            remove_instead_of_reset: def.remove_instead_of_reset,
            impact_effect: def.impact_effect,
            impact_sound: def.impact_sound,
            compound,
        })
    }

    /// The parts of a compound object, placed by its skeleton's bind pose.
    fn compound_physics(&self, def: &ObjectDef, parts: &[Option<PhysicalDefinition>]) -> Result<CompoundPhysics> {
        let skeleton = match def.skeleton {
            Some(id) => Some(mw_formats::Skeleton::parse(&self.load(id)?)?),
            None => None,
        };
        let behaviour = |b: &mw_formats::Behaviour| b.object.and_then(|id| self.genesys_object(id).ok());
        let index = |i: u8| (usize::from(i) < def.compound_parts.len()).then_some(usize::from(i));
        let out = def
            .compound_parts
            .iter()
            .zip(parts)
            .map(|(p, definition)| {
                let bone = skeleton
                    .as_ref()
                    .and_then(|s| s.bones.get(p.bone as usize))
                    .map_or(crate::IDENTITY, |b| bone_matrix(b.rotation, b.position));
                let smash_force = p
                    .behaviours
                    .iter()
                    .filter(|b| b.type_id == mw_formats::SMASH_PROXY_BEHAVIOUR)
                    .find_map(|b| float(&behaviour(b)?, field_hash("ForceToSmash")));
                let impact_sound =
                    p.behaviours.iter().find(|b| b.type_id == PLAY_SOUND_BEHAVIOUR).and_then(|b| b.object);
                CompoundPartPhysics {
                    definition: definition.clone(),
                    bone,
                    bone_index: p.bone,
                    replaces: p.replaces.and_then(index),
                    next: p.next.and_then(index),
                    first_replacement: p.first_replacement.and_then(index),
                    links: p.links.iter().filter_map(|&l| index(l)).collect(),
                    smash_force,
                    impact_sound,
                }
            })
            .collect();
        // `EnableCompoundBehaviour` +0x1a (`543527d0`) set: split at stream-in (`FUN_00554230`).
        let enabled_at_start = def
            .behaviours
            .iter()
            .filter(|b| b.type_id == mw_formats::ENABLE_COMPOUND_BEHAVIOUR)
            .any(|b| behaviour(b).is_some_and(|o| boolean(&o, 0x543527d0) == Some(true)));
        let joints = def.joints.iter().flatten().copied().collect();
        Ok(CompoundPhysics { parts: out, enabled_at_start, joints })
    }
}
