//! Vehicle damage: the game's deformation data and how to pose a car's skeleton with it.
//!
//! A car body is skinned to a small skeleton (`VehicleGraphicsSpec` → `Skeleton`, ~37 bones: crumple
//! points, bumpers, lights, mirrors, number plates). Its `VehicleDamageBehaviour` GeneSys object points
//! at an `AnimationList` of one clip per impact zone (front, rear, left, right and the four corners);
//! a clip's frames are increasing damage, from the undamaged bind pose (frame 0) to fully wrecked.
//! Bones that break off get scale 0 (and drop below the car) at the frame where they detach. So a
//! damaged car is posed by sampling each zone's clip at that zone's damage level and skinning the body,
//! the same way the vertex shaders apply `gBoneMatrixArray`.
//!
//! How much damage an impact does is not known from the data alone; [`ImpactGraph`] reads the global
//! `ImpactDamageGraphs` curve (impact speed → damage %) and is the current best guess (see
//! `docs/research-log.md`).

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::{Animation, AnimationList, BoneKey, CHANNEL_ROTATION, CHANNEL_SCALE, CHANNEL_TRANSLATION, Skeleton};

use crate::{AssetStore, Error, Result};

const ANIMATION_LIST: u32 = field_hash("AnimationList");
const BODYPARTS: u32 = 0x10d211bd;
const BODYPART_POSITION: u32 = 0xbc848321;
const BODYPART_MASS: u32 = field_hash("Mass");
const BODYPART_AREA: u32 = field_hash("DamageArea");
const BODYPART_LIMITS: [u32; 3] = [0x15605a23, 0x0804d1b7, 0xe077cde5];

/// A uniform-scale rigid transform: `p' = scale * rotate(p) + translation`. Uniform scale keeps the bone
/// hierarchy closed under composition, and zero scale (a detached part) stays representable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Similarity {
    pub scale: f32,
    /// Quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn qconj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

fn qrot(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let p = qmul(qmul(q, [v[0], v[1], v[2], 0.0]), qconj(q));
    [p[0], p[1], p[2]]
}

impl Similarity {
    pub const IDENTITY: Self = Self { scale: 1.0, rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3] };

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = qrot(self.rotation, p);
        std::array::from_fn(|i| self.scale * r[i] + self.translation[i])
    }

    /// `self` after `inner`: `(self ∘ inner)(p) = self(inner(p))`.
    pub fn then_after(&self, inner: &Self) -> Self {
        Self {
            scale: self.scale * inner.scale,
            rotation: qmul(self.rotation, inner.rotation),
            translation: self.apply(inner.translation),
        }
    }

    /// Inverse (only for non-zero scale; bind poses always have scale 1).
    pub fn inverse(&self) -> Self {
        let rotation = qconj(self.rotation);
        let scale = 1.0 / self.scale;
        let t = qrot(rotation, self.translation);
        Self { scale, rotation, translation: t.map(|c| -c * scale) }
    }

    /// Row-major D3D matrix (`v * M`).
    pub fn matrix(&self) -> [[f32; 4]; 4] {
        let s = [self.scale; 3];
        crate::trs_matrix(self.translation, self.rotation, s)
    }
}

/// Spherical interpolation along the shorter arc, normalised lerp when the angle is under 5° (cos > 0.9961947),
/// as `FUN_009f3250` does.
fn qslerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut cos: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let mut b = b;
    if cos < 0.0 {
        cos = -cos;
        b = b.map(|c| -c);
    }
    let (wa, wb) = if cos > 0.996_194_7 {
        (1.0 - t, t)
    } else {
        let angle = cos.clamp(-1.0, 1.0).acos();
        let s = angle.sin();
        (((1.0 - t) * angle).sin() / s, (t * angle).sin() / s)
    };
    let q: [f32; 4] = std::array::from_fn(|i| wa * a[i] + wb * b[i]);
    let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if len > 1e-6 { q.map(|c| c / len) } else { [0.0, 0.0, 0.0, 1.0] }
}

/// The damage level cap while the car is not wrecked (`FUN_005b62f0`: levels are capped at 0.99 while the
/// vehicle's used damage-bar segments are below its profile's count, so the last frame shows only on a
/// wrecked car). We have no wrecked state, so it always applies (as in `paint_wear.rs`).
pub const LEVEL_CAP: f32 = 0.99;

/// Frame of a damage clip at `level` (`FUN_005b62f0`): level → whole percent p = min(⌊100 level + 0.5⌋, 100),
/// time p / 100 of the clip, snapped down to a whole frame (no blend between frames). The clip's end time
/// being its last frame is inferred (the controller's set-up was not read).
pub fn damage_frame(level: f32, frames: usize) -> usize {
    let p = ((level * 100.0 + 0.5).floor() as usize).min(100);
    (p * frames.saturating_sub(1) / 100).min(frames.saturating_sub(1))
}

/// Impact zones in clip order. Every car's damage `AnimationList` lists its clips in this order (on 50
/// of the 72 cars the bones each clip moves point exactly this way; on the others a few clips move
/// bones placed off-centre), so the clip index is the zone.
pub const ZONES: [Zone; 8] = {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    [
        Zone { name: "front", direction: [0.0, 1.0] },
        Zone { name: "rear", direction: [0.0, -1.0] },
        Zone { name: "left", direction: [1.0, 0.0] },
        Zone { name: "right", direction: [-1.0, 0.0] },
        Zone { name: "front-left", direction: [D, D] },
        Zone { name: "front-right", direction: [-D, D] },
        Zone { name: "rear-left", direction: [D, -D] },
        Zone { name: "rear-right", direction: [-D, -D] },
    ]
};

/// Where on the car an impact zone (one damage clip) is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub name: &'static str,
    /// Unit direction in the car's XZ plane (x left, z forward), in units of the car's half size.
    pub direction: [f32; 2],
}

impl Zone {
    /// Sector name of a direction: 8 sectors of 45° (x left, z forward).
    pub fn sector(direction: [f32; 2]) -> &'static str {
        let a = direction[0].atan2(direction[1]).to_degrees();
        const NAMES: [&str; 8] =
            ["front", "front-left", "left", "rear-left", "rear", "rear-right", "right", "front-right"];
        NAMES[(((a + 360.0 + 22.5) % 360.0) / 45.0) as usize % 8]
    }
}

#[derive(Debug, Clone)]
pub struct DamageClip {
    pub zone: Zone,
    /// Direction of the centroid of the bones the clip moves (a check on `zone`).
    pub bones_direction: [f32; 2],
    pub animation: Animation,
}

/// A hinged body part (bonnet, bumpers, doors, mirrors) from `VehicleDamageBehaviour.Bodypart`.
#[derive(Debug, Clone)]
pub struct Bodypart {
    /// Hinge position (model space).
    pub position: [f32; 3],
    pub mass: f32,
    /// 1 front, 2 rear, 3 left, 4 right.
    pub area: u32,
    /// Angle limits (degrees) about X, Y and Z (field names unknown; doors swing −15..15 about Z and
    /// −5..15 about Y, the bonnet −15..0 about X).
    pub limits: [[f32; 2]; 3],
}

/// Everything needed to deform one car.
#[derive(Debug, Clone)]
pub struct VehicleDamage {
    pub skeleton: Skeleton,
    pub clips: Vec<DamageClip>,
    pub bodyparts: Vec<Bodypart>,
    /// Bind pose per bone: its position only. The game ignores the skeleton's bind rotations (`AnimBone`
    /// +0x10) both when posing and when skinning (`FUN_009f4560`, `FUN_0059fa70`).
    pub bind: Vec<Similarity>,
    /// Animated body parts (active aero) posed on the same skeleton (`crate::car_anim`).
    pub animations: Option<crate::car_anim::CarAnimations>,
}

impl VehicleDamage {
    pub fn new(skeleton: Skeleton, clips: Vec<Animation>, bodyparts: Vec<Bodypart>) -> Result<Self> {
        let n = skeleton.bones.len();
        let bind: Vec<Similarity> = skeleton
            .bones
            .iter()
            .map(|b| Similarity { scale: 1.0, rotation: [0.0, 0.0, 0.0, 1.0], translation: b.position })
            .collect();
        // Car half size from the bones (they sit on the body surface).
        let half = |axis: usize| skeleton.bones.iter().map(|b| b.position[axis].abs()).fold(0.1f32, f32::max);
        let (hx, hz) = (half(0), half(2));
        let clips = clips
            .into_iter()
            .enumerate()
            .map(|(i, animation)| {
                if animation.bone_count != n {
                    return Err(Error::Format(mw_formats::Error::Malformed(format!(
                        "damage clip has {} bones, skeleton {n}",
                        animation.bone_count
                    ))));
                }
                let moved: Vec<[f32; 3]> =
                    animation.bones.iter().filter(|&&b| b > 0).map(|&b| skeleton.bones[b].position).collect();
                let k = moved.len().max(1) as f32;
                let (x, z) = moved.iter().fold((0.0, 0.0), |(x, z), p| (x + p[0] / hx / k, z + p[2] / hz / k));
                let len = (x * x + z * z).sqrt().max(1e-6);
                let zone = *ZONES
                    .get(i)
                    .ok_or_else(|| Error::Format(mw_formats::Error::Malformed(format!("{} damage clips", i + 1))))?;
                Ok(DamageClip { zone, bones_direction: [x / len, z / len], animation })
            })
            .collect::<Result<_>>()?;
        Ok(Self { skeleton, clips, bodyparts, bind, animations: None })
    }

    /// Pose every bone (model space) with `levels[i]` (0..1) of damage in clip `i`.
    pub fn pose(&self, levels: &[f32]) -> Vec<Similarity> {
        self.pose_with(levels, &[])
    }

    /// Like [`Self::pose`], plus animated body part clips at a position each (0 = first frame, 1 = last).
    ///
    /// The game's pose (docs/decomp.md "Vehicle damage pose"): every bone starts at zero translation, identity
    /// rotation and scale 1 each frame (`FUN_009ecfe0`); `FUN_005b62f0` applies the damage clips in index order
    /// with weight 1 through the additive blend `FUN_009f3250`, skipping a clip whose level is 0. Per bone with
    /// mask weight w > 0 and only for the channels the clip animates: t += w (T[f] − T[r]); q = slerp(q, D ⊗ q,
    /// w) with D = Q[f] ⊗ Q[r]⁻¹; s = lerp(s, S[f], w) (absolute: the last clip that scales a bone wins); r is
    /// the clip's reference frame (0 in vehicles). So a key that is the same in every frame (the XKR's boot lid)
    /// adds nothing. Part clips go through the same blend after the damage (inferred: the aero player was not
    /// identified; `FUN_0047bd00`, a forward / backward clip player calling the same blend, is the candidate),
    /// at a fractional frame. Then `FUN_009f4560` builds model space without bind rotations.
    pub fn pose_with(&self, levels: &[f32], parts: &[(&Animation, f32)]) -> Vec<Similarity> {
        let n = self.skeleton.bones.len();
        let mut keys = vec![BoneKey::IDENTITY; n];
        let damage = self.clips.iter().zip(levels).filter_map(|(c, &level)| {
            let a = &c.animation;
            (level.abs() > f32::EPSILON).then(|| (a, damage_frame(level.min(LEVEL_CAP), a.frames.len()) as f32))
        });
        let parts = parts.iter().filter(|(_, p)| *p > 0.0).map(|&(a, p)| {
            let last = a.frames.len().saturating_sub(1) as f32;
            (a, p.min(1.0) * last)
        });
        for (animation, frame) in damage.chain(parts) {
            if animation.bone_count != n || animation.frames.is_empty() {
                continue;
            }
            let reference = &animation.frames[animation.reference_frame.min(animation.frames.len() - 1)];
            let sampled = animation.sample(frame);
            for (((&b, &channels), k), r) in animation.bones.iter().zip(&animation.channels).zip(sampled).zip(reference)
            {
                let w = animation.bone_mask.get(b).copied().unwrap_or(1.0);
                if w <= 0.0 {
                    continue;
                }
                let acc = &mut keys[b];
                if channels & CHANNEL_TRANSLATION != 0 {
                    acc.translation =
                        std::array::from_fn(|i| acc.translation[i] + w * (k.translation[i] - r.translation[i]));
                }
                if channels & CHANNEL_ROTATION != 0 {
                    let d = qmul(k.rotation, qconj(r.rotation));
                    acc.rotation = qslerp(acc.rotation, qmul(d, acc.rotation), w.min(1.0));
                }
                if channels & CHANNEL_SCALE != 0 {
                    acc.scale += (k.scale - acc.scale) * w;
                }
            }
        }
        // Model space (`FUN_009f4560`): q = q_parent ⊗ q_local; t = t_parent + s_parent rotate(q_parent,
        // (bind_b − bind_parent) + t_local); s = s_parent s_local.
        let mut world: Vec<Similarity> = Vec::with_capacity(n);
        for (i, bone) in self.skeleton.bones.iter().enumerate() {
            let k = &keys[i];
            let offset = match bone.parent {
                Some(p) => std::array::from_fn(|c| bone.position[c] - self.skeleton.bones[p].position[c]),
                None => bone.position,
            };
            let local = Similarity {
                scale: k.scale,
                rotation: k.rotation,
                translation: std::array::from_fn(|c| offset[c] + k.translation[c]),
            };
            world.push(bone.parent.map_or(local, |p| world[p].then_after(&local)));
        }
        world
    }

    /// Skinning transforms (posed ∘ inverse bind) for `pose`.
    pub fn skin(&self, pose: &[Similarity]) -> Vec<Similarity> {
        pose.iter().zip(&self.bind).map(|(p, b)| p.then_after(&b.inverse())).collect()
    }

    /// Damage weight of each clip for an impact at `point` (car space, metres, x left / z forward):
    /// 1 for the zone facing the impact, falling to 0 at 60° away from it (a model choice: the game's mapping
    /// is not known; 60° lets a corner hit also crumple the adjacent front / side a little).
    pub fn zone_weights(&self, point: [f32; 3]) -> Vec<f32> {
        let half = |axis: usize| self.skeleton.bones.iter().map(|b| b.position[axis].abs()).fold(0.1f32, f32::max);
        let (x, z) = (point[0] / half(0), point[2] / half(2));
        let len = (x * x + z * z).sqrt();
        if len < 1e-4 {
            return vec![0.0; self.clips.len()];
        }
        const COS_FALLOFF: f32 = 0.5;
        let w: Vec<f32> = self
            .clips
            .iter()
            .map(|c| {
                let cos = (c.zone.direction[0] * x + c.zone.direction[1] * z) / len;
                ((cos - COS_FALLOFF) / (1.0 - COS_FALLOFF)).max(0.0)
            })
            .collect();
        let max = w.iter().copied().fold(0.0, f32::max);
        if max <= 0.0 { w } else { w.iter().map(|v| v / max).collect() }
    }
}

/// Impact speed → damage, from the `ImpactDamageGraphs` object in `PHYSICS/PHYSICSCONFIG.BNDL`
/// (first graph). Read as three (speed, damage %) points; the field names are unknown, so the units
/// (taken as mph, like the handling data's speeds) are a guess.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactGraph {
    /// (speed in mph, damage 0..1), ascending speed.
    pub points: [[f32; 2]; 3],
}

impl Default for ImpactGraph {
    /// The values shipped in 1.4.0.0.
    fn default() -> Self {
        Self { points: [[5.0, 0.0], [80.0, 0.33], [160.0, 1.0]] }
    }
}

impl ImpactGraph {
    /// Damage (0..1 of a zone) for an impact at `speed` m/s along the contact normal.
    pub fn damage(&self, speed: f32) -> f32 {
        let mph = speed / 0.44704;
        let p = &self.points;
        if mph <= p[0][0] {
            return 0.0;
        }
        for w in p.windows(2) {
            let ([s0, d0], [s1, d1]) = (w[0], w[1]);
            if mph <= s1 {
                return d0 + (d1 - d0) * (mph - s0) / (s1 - s0).max(1e-3);
            }
        }
        p[2][1]
    }

    fn from_object(o: &Object) -> Option<Self> {
        let graph = match o.get(0x97aab742)? {
            Value::Struct(g) => g,
            _ => return None,
        };
        let f = |h: u32| match graph.get(h) {
            Some(Value::Float(v)) => Some(*v),
            _ => None,
        };
        let [hi_s, hi_d, mid_s, mid_d, lo_s, lo_d] =
            [0x64df1ca1, 0xf107cd5c, 0xa4657c04, 0xa77b7fdd, 0x5debcbe0, 0xaeeac48a].map(f);
        let points = [[lo_s?, lo_d? / 100.0], [mid_s?, mid_d? / 100.0], [hi_s?, hi_d? / 100.0]];
        (points[0][0] < points[1][0] && points[1][0] < points[2][0]).then_some(Self { points })
    }
}

fn floats4(v: Option<&Value>) -> Option<[f32; 4]> {
    match v? {
        Value::Vector(v) => Some(*v),
        _ => None,
    }
}

fn read_bodypart(o: &Object) -> Option<Bodypart> {
    let p = floats4(o.get(BODYPART_POSITION))?;
    let mass = match o.get(BODYPART_MASS)? {
        Value::Float(m) => *m,
        _ => return None,
    };
    let area = match o.get(BODYPART_AREA) {
        Some(Value::UInt(a)) => *a as u32,
        Some(Value::Int(a)) => *a as u32,
        _ => 0,
    };
    let limits = BODYPART_LIMITS.map(|h| floats4(o.get(h)).map_or([0.0; 2], |v| [v[0], v[1]]));
    Some(Bodypart { position: [p[0], p[1], p[2]], mass, area, limits })
}

impl AssetStore {
    /// The damage model of a vehicle bundle (`_HI`), or `None` if it has no skeleton or damage clips.
    pub fn vehicle_damage(&self, bundle: &Bundle) -> Result<Option<VehicleDamage>> {
        let Some(entry) = bundle.entries_of_type(ResourceType::VehicleGraphicsSpec).next() else {
            return Ok(None);
        };
        let spec = mw_formats::VehicleGraphicsSpec::parse(&bundle.load(entry)?)?;
        let Some(skeleton_id) = spec.skeleton else { return Ok(None) };
        let skeleton = Skeleton::parse(&self.load(skeleton_id)?)?;
        let mut anims: Option<ResourceId> = None;
        let mut bodyparts = Vec::new();
        for e in bundle.entries_of_type(ResourceType::GenesysObject) {
            let Ok(obj) = self.decode_genesys(&bundle.load(e)?) else { continue };
            if obj.type_name != "Genesys.Gen.VehicleDamageBehaviour" {
                continue;
            }
            if let Some(Value::Handle(Some(id))) = obj.get(ANIMATION_LIST) {
                anims = Some(*id);
            }
            if let Some(Value::Array(parts)) = obj.get(BODYPARTS) {
                bodyparts = parts
                    .iter()
                    .filter_map(|p| match p {
                        Value::Struct(s) => read_bodypart(s),
                        _ => None,
                    })
                    .collect();
            }
            break;
        }
        let Some(anims) = anims else { return Ok(None) };
        let list = AnimationList::parse(&self.load(anims)?)?;
        let mut damage = VehicleDamage::new(skeleton, list.clips, bodyparts)?;
        damage.animations = self.car_animations(bundle)?;
        Ok(Some(damage))
    }

    /// The global impact speed → damage curve (defaults if `PHYSICSCONFIG` can't be read).
    pub fn impact_graph(&self) -> ImpactGraph {
        let read = || -> Option<ImpactGraph> {
            let b = self.bundle("PHYSICS/PHYSICSCONFIG.BNDL")?;
            b.entries_of_type(ResourceType::GenesysObject).find_map(|e| {
                let obj = self.decode_genesys(&b.load(e).ok()?).ok()?;
                (obj.type_name == "Genesys.Gen.ImpactDamageGraphs").then(|| ImpactGraph::from_object(&obj))?
            })
        };
        read().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn similarity_compose_and_inverse() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let a = Similarity { scale: 2.0, rotation: [0.0, h, 0.0, h], translation: [1.0, 2.0, 3.0] };
        let p = [0.5, -1.0, 0.25];
        assert!(close(a.inverse().apply(a.apply(p)), p));
        let b = Similarity { scale: 1.0, rotation: [h, 0.0, 0.0, h], translation: [0.0, 1.0, 0.0] };
        assert!(close(a.then_after(&b).apply(p), a.apply(b.apply(p))));
        // Matrix form agrees (row vectors).
        let m = a.matrix();
        let mp: [f32; 3] = std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c]);
        assert!(close(mp, a.apply(p)));
    }

    #[test]
    fn impact_graph_interpolates() {
        let g = ImpactGraph::default();
        assert_eq!(g.damage(1.0), 0.0);
        let at80 = g.damage(80.0 * 0.44704);
        assert!((at80 - 0.33).abs() < 1e-4, "{at80}");
        assert_eq!(g.damage(200.0), 1.0);
    }

    /// A box-shaped stand-in car: bones at the corners and mid-sides, one empty clip per zone.
    fn test_car() -> VehicleDamage {
        let mut bones = vec![mw_formats::Bone { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], parent: None }];
        for (x, z) in [(1.0, 2.0), (-1.0, 2.0), (1.0, -2.0), (-1.0, -2.0)] {
            bones.push(mw_formats::Bone { position: [x, 0.5, z], rotation: [0.0, 0.0, 0.0, 1.0], parent: Some(0) });
        }
        let n = bones.len();
        let skeleton = Skeleton { ids: vec![0; n], bones };
        let clip = |bone: usize| Animation {
            id: 0,
            frame_rate: 6.0,
            bone_count: n,
            bones: vec![bone],
            channels: vec![mw_formats::CHANNEL_TRANSLATION | mw_formats::CHANNEL_SCALE],
            reference_frame: 0,
            bone_mask: Vec::new(),
            frames: vec![
                vec![BoneKey::IDENTITY],
                vec![BoneKey { translation: [0.0, -0.05, 0.0], scale: 0.5, ..BoneKey::IDENTITY }],
                vec![BoneKey { translation: [0.0, -0.1, 0.0], scale: 0.0, ..BoneKey::IDENTITY }],
            ],
        };
        VehicleDamage::new(skeleton, (0..8).map(|i| clip(1 + i % 4)).collect(), Vec::new()).unwrap()
    }

    #[test]
    fn zone_weights_pick_the_facing_zone() {
        let car = test_car();
        let w = car.zone_weights([0.0, 0.5, 2.1]);
        assert_eq!(w[0], 1.0, "{w:?}"); // front
        assert_eq!(w[1], 0.0); // rear
        assert!(w[4] > 0.0 && w[4] < 1.0 && w[4] == w[5], "{w:?}"); // corners share a little
        let w = car.zone_weights([1.0, 0.5, 2.0]);
        assert_eq!(w[4], 1.0, "{w:?}"); // front-left
    }

    #[test]
    fn pose_is_bind_when_undamaged_and_follows_clips() {
        let car = test_car();
        let pose = car.pose(&[0.0; 8]);
        assert_eq!(pose, car.bind);
        // Level 0.6 → 60 % → frame 1 of clip 0 (whole frames): bone 1 halfway down and half scale.
        let pose = car.pose(&[0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!(close(pose[1].translation, [1.0, 0.45, 2.0]), "{:?}", pose[1]);
        assert!((pose[1].scale - 0.5).abs() < 1e-6);
        // Level 0.49 stays on frame 0, and full damage is capped at 0.99 → frame 1 of 3.
        assert_eq!(car.pose(&[0.49, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), car.bind);
        assert!((car.pose(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])[1].scale - 0.5).abs() < 1e-6);
        // Skinning maps the bind position of a bone to its posed position.
        let skin = car.skin(&pose);
        assert!(close(skin[1].apply([1.0, 0.5, 2.0]), pose[1].translation));
    }

    #[test]
    fn constant_keys_add_nothing_and_scale_is_last_wins() {
        let mut car = test_car();
        // A key that is the same in every frame (the XKR's boot lid) is the clip's reference: no change.
        for c in &mut car.clips {
            for f in &mut c.animation.frames {
                f[0].translation[0] += 0.12;
            }
        }
        assert_eq!(car.pose(&[0.6; 8]), test_car().pose(&[0.6; 8]));
        // Clips 0 and 4 both move bone 1: translations add, the later clip's scale wins.
        car.clips[4].animation.frames[1][0].scale = 0.8;
        let pose = car.pose(&[1.0, 0.0, 0.0, 0.0, 0.6, 0.0, 0.0, 0.0]);
        assert!(close(pose[1].translation, [1.0, 0.4, 2.0]), "{:?}", pose[1]);
        assert!((pose[1].scale - 0.8).abs() < 1e-6);
    }

    #[test]
    fn zone_sectors_match_the_table() {
        for z in ZONES {
            assert_eq!(Zone::sector(z.direction), z.name);
        }
    }
}
