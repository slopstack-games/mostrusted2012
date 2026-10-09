//! Skeletal animation: `Skeleton` (0xB2), `Animation` (0xB3) and `AnimationList` (0xB0).
//!
//! Field names follow burnout.wiki (`CgsAnimation::AnimSkel`, `Anim`, `AnimationCollection`, from the
//! debug symbols of a 2012 PS3 build). Vehicle damage is stored as an `AnimationList` of per-zone
//! clips whose frames are damage levels (see `docs/formats.md`, "Vehicle damage").
//!
//! `Skeleton` (`AnimSkel`):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u16  | version (2) |
//! | 0x02 | u16  | bone count |
//! | 0x04 | ptr  | bones (0x30 bytes each) |
//! | 0x08 | ptr  | IK bone indices |
//! | 0x0C | ptr  | u32 bone IDs (CRC-32 of the original names; the hash is not `field_hash`) |
//! | 0x10 | u16  | IK bone count |
//!
//! Bone (`AnimBone`, 0x30 bytes): `f32×4 position`, `f32×4 rotation quaternion (x, y, z, w)`,
//! `i32 parent`, `i32 peer` (previous sibling), `i32 child` (last child), `i32 remap index`. Positions are in
//! model space (a vehicle's bones sit on its body), not relative to the parent; parents come before children.
//!
//! `Animation` (`Anim`); inside an `AnimationList` the pointers are relative to the clip's start:
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u32  | version (2) |
//! | 0x04 | u32  | frame count |
//! | 0x08 | u32  | bone count (matches the skeleton) |
//! | 0x0C | u32  | rotated bones (Q) |
//! | 0x10 | u32  | translated bones (T) |
//! | 0x14 | u32  | scaled bones (S) |
//! | 0x18 | u32  | IK bones |
//! | 0x1C | u32  | bytes per frame: rotations (4·Q for codec 1, 16·Q for codec 0) |
//! | 0x20 | u32  | bytes per frame: translations (16·T) |
//! | 0x24 | u32  | bytes per frame: scales (4·S) |
//! | 0x28 | u32  | bytes per frame: IK |
//! | 0x2C | u32  | total size |
//! | 0x30 | f32  | frame rate |
//! | 0x34 | u16  | quaternion codec: 0 = f32×4, 1 = 32-bit (vehicles) |
//! | 0x36 | u16  | reference frame: keys blend as changes from it (0 in vehicle clips) |
//! | 0x38 | ptr  | root translations (f32×4 per frame) |
//! | 0x3C | ptr  | translations (f32×4 per bone per frame) |
//! | 0x40 | ptr  | IK positions |
//! | 0x44 | ptr  | bone mask (f32 blend weight per bone) |
//! | 0x48 | ptr  | translation remap (u8 per bone: channel or 0xFF) |
//! | 0x4C | ptr  | rotations (codec-encoded, per bone per frame) |
//! | 0x50 | ptr  | scale remap (u8 per bone) |
//! | 0x54 | ptr  | rotation remap (u8 per bone) |
//! | 0x58 | ptr  | scales (f32 per bone per frame, uniform) |
//! | 0x5C | i32  | animation ID |
//!
//! Translations are offsets from the bind pose (frame 0 of a damage clip is all zeros), in the parent's
//! space. Rotation codec 1 is 32 bits per quaternion, "smallest three": bits 30–31 give the index of the
//! largest component (which is positive and rebuilt from the unit length), and three 10-bit fields
//! (bits 20–29, 10–19, 0–9) hold the others in order, mapped from 0..1023 to ±1/√2. `0xDFF7FDFF` is the
//! identity.
//!
//! `AnimationList` (`AnimationCollection`): `u16 version (2)`, `u16 count`, `u32 size`, `ptr` to `u32`
//! clip offsets, then three `ptr`s to per-clip start / middle / end sound handles (zero in vehicles).

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bone {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub parent: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
    /// CRC-32 name hashes, one per bone.
    pub ids: Vec<u32>,
}

impl Skeleton {
    pub fn parse(res: &Resource) -> Result<Self> {
        Self::parse_bytes(res.header())
    }

    pub fn parse_bytes(data: &[u8]) -> Result<Self> {
        let r = Reader(data);
        let version = r.u16(0)?;
        if version != 2 {
            return Err(Error::Unsupported(format!("skeleton version {version}")));
        }
        let n = r.u16(2)? as usize;
        let bones_ptr = r.u32(4)? as usize;
        let ids_ptr = r.u32(0x0C)? as usize;
        let mut bones = Vec::with_capacity(n);
        for i in 0..n {
            let b = bones_ptr + i * 0x30;
            let [x, y, z, _] = r.f32_array::<4>(b)?;
            let rotation = r.f32_array::<4>(b + 0x10)?;
            let parent = r.i32(b + 0x20)?;
            // Parents precede their children; anything else would make posing order-dependent.
            let parent = match parent {
                p if p < 0 => None,
                p if (p as usize) < i => Some(p as usize),
                p => return Err(Error::Malformed(format!("bone {i} has parent {p}"))),
            };
            bones.push(Bone { position: [x, y, z], rotation, parent });
        }
        let ids = (0..n).map(|i| r.u32(ids_ptr + i * 4)).collect::<std::result::Result<_, _>>()?;
        Ok(Self { bones, ids })
    }
}

/// One frame's transform of one bone, relative to the bind pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneKey {
    /// Offset from the bind position, in the parent's space.
    pub translation: [f32; 3],
    /// Rotation applied in the bone's own space.
    pub rotation: [f32; 4],
    pub scale: f32,
}

impl BoneKey {
    pub const IDENTITY: Self = Self { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0 };

    /// Linear blend (normalised lerp for the rotation, along the shorter arc).
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let l = |a: f32, b: f32| a + (b - a) * t;
        let dot: f32 = (0..4).map(|i| self.rotation[i] * other.rotation[i]).sum();
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut q: [f32; 4] = std::array::from_fn(|i| l(self.rotation[i], other.rotation[i] * s));
        let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
        if len > 1e-6 {
            q = q.map(|c| c / len);
        } else {
            q = [0.0, 0.0, 0.0, 1.0];
        }
        Self {
            translation: std::array::from_fn(|i| l(self.translation[i], other.translation[i])),
            rotation: q,
            scale: l(self.scale, other.scale),
        }
    }
}

/// A decoded animation: per-frame keys for the bones it moves.
#[derive(Debug, Clone)]
pub struct Animation {
    pub id: i32,
    pub frame_rate: f32,
    pub bone_count: usize,
    /// Bones with at least one channel.
    pub bones: Vec<usize>,
    /// Which channels each of `bones` has: [`CHANNEL_TRANSLATION`] | [`CHANNEL_ROTATION`] | [`CHANNEL_SCALE`].
    /// The game's blend leaves a channel a clip does not animate as it is (`FUN_009f3250`).
    pub channels: Vec<u8>,
    /// `frames[f][i]` is the key of `bones[i]` at frame `f`.
    pub frames: Vec<Vec<BoneKey>>,
    /// The reference frame (`Anim` +0x36): the game's blend adds each key's change from this frame
    /// (`FUN_009f3250`, docs/decomp.md "Vehicle damage pose"). 0 in every vehicle clip.
    pub reference_frame: usize,
    /// Blend weight per skeleton bone (`Anim` +0x44); empty when the clip has no mask.
    pub bone_mask: Vec<f32>,
}

pub const CHANNEL_TRANSLATION: u8 = 1;
pub const CHANNEL_ROTATION: u8 = 2;
pub const CHANNEL_SCALE: u8 = 4;

/// Decode a codec-1 quaternion (see the module docs).
pub fn decode_quat32(v: u32) -> [f32; 4] {
    let largest = (v >> 30) as usize;
    let small = [(v >> 20) & 0x3FF, (v >> 10) & 0x3FF, v & 0x3FF]
        .map(|c| (c as f32 / 1023.0 * 2.0 - 1.0) * std::f32::consts::FRAC_1_SQRT_2);
    let big = (1.0 - small.iter().map(|c| c * c).sum::<f32>()).max(0.0).sqrt();
    let mut q = [0.0; 4];
    let mut s = small.iter();
    for (i, c) in q.iter_mut().enumerate() {
        *c = if i == largest { big } else { *s.next().unwrap() };
    }
    q
}

impl Animation {
    pub fn parse(res: &Resource) -> Result<Self> {
        Self::parse_bytes(res.header())
    }

    /// Parse a clip; `data` starts at the clip header (pointers are relative to it).
    pub fn parse_bytes(data: &[u8]) -> Result<Self> {
        let r = Reader(data);
        let version = r.u32(0)?;
        if version != 2 {
            return Err(Error::Unsupported(format!("animation version {version}")));
        }
        let [frames, bone_count, nq, nt, ns, _nik, fq, ft, fs] = r.u32_array::<9>(4)?.map(|v| v as usize);
        if bone_count > 256 || frames > 10_000 {
            return Err(Error::Malformed(format!("animation with {bone_count} bones, {frames} frames")));
        }
        let frame_rate = r.f32(0x30)?;
        let codec = r.u16(0x34)?;
        let quat_size = match codec {
            0 => 16,
            1 => 4,
            _ if nq == 0 => 0,
            _ => return Err(Error::Unsupported(format!("quaternion codec {codec}"))),
        };
        if fq < nq * quat_size {
            return Err(Error::Malformed(format!("{fq} rotation bytes per frame for {nq} bones (codec {codec})")));
        }
        if ft < nt * 16 || fs < ns * 4 {
            return Err(Error::Malformed("frame sizes smaller than their channels".into()));
        }
        let [root, trans, _ik, mask, t_remap, rots, s_remap, r_remap, scales] =
            r.u32_array::<9>(0x38)?.map(|v| v as usize);
        let reference_frame = r.u16(0x36)? as usize;
        if frames > 0 && reference_frame >= frames {
            return Err(Error::Malformed(format!("reference frame {reference_frame} of {frames}")));
        }
        let bone_mask = if mask == 0 {
            Vec::new()
        } else {
            (0..bone_count).map(|b| r.f32(mask + b * 4)).collect::<std::result::Result<_, _>>()?
        };
        let id = r.i32(0x5C)?;
        let remap = |ptr: usize, channels: usize| -> Result<Vec<Option<usize>>> {
            (0..bone_count)
                .map(|b| {
                    let c = r.u8(ptr + b)?;
                    Ok(if c == 0xFF || channels == 0 { None } else { Some(c as usize) })
                })
                .map(|c| match c {
                    Ok(Some(c)) if c >= channels => Err(Error::Malformed(format!("channel {c} of {channels}"))),
                    other => other,
                })
                .collect()
        };
        let t_map = remap(t_remap, nt)?;
        let r_map = remap(r_remap, nq)?;
        let s_map = remap(s_remap, ns)?;
        let bones: Vec<usize> =
            (0..bone_count).filter(|&b| t_map[b].is_some() || r_map[b].is_some() || s_map[b].is_some()).collect();
        let channels: Vec<u8> = bones
            .iter()
            .map(|&b| {
                (t_map[b].is_some() as u8) * CHANNEL_TRANSLATION
                    | (r_map[b].is_some() as u8) * CHANNEL_ROTATION
                    | (s_map[b].is_some() as u8) * CHANNEL_SCALE
            })
            .collect();
        let mut out = Vec::with_capacity(frames);
        for f in 0..frames {
            let mut keys = Vec::with_capacity(bones.len());
            for &b in &bones {
                let mut k = BoneKey::IDENTITY;
                if let Some(c) = t_map[b] {
                    let [x, y, z, _] = r.f32_array::<4>(trans + f * ft + c * 16)?;
                    k.translation = [x, y, z];
                }
                if b == 0 && root != 0 {
                    // Root motion (zero in every vehicle clip).
                    let [x, y, z, _] = r.f32_array::<4>(root + f * 16)?;
                    k.translation = std::array::from_fn(|i| k.translation[i] + [x, y, z][i]);
                }
                if let Some(c) = r_map[b] {
                    let at = rots + f * fq + c * quat_size;
                    k.rotation = if codec == 0 { r.f32_array::<4>(at)? } else { decode_quat32(r.u32(at)?) };
                }
                if let Some(c) = s_map[b] {
                    k.scale = r.f32(scales + f * fs + c * 4)?;
                }
                keys.push(k);
            }
            out.push(keys);
        }
        Ok(Self { id, frame_rate, bone_count, bones, channels, frames: out, reference_frame, bone_mask })
    }

    /// Keys at a fractional frame position (clamped to the clip), for the bones in `self.bones`.
    pub fn sample(&self, frame: f32) -> Vec<BoneKey> {
        let Some(last) = self.frames.len().checked_sub(1) else { return Vec::new() };
        let f = frame.clamp(0.0, last as f32);
        let i = (f.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let t = f - i as f32;
        self.frames[i].iter().zip(&self.frames[j]).map(|(a, b)| a.lerp(b, t)).collect()
    }
}

/// `AnimationList`: a collection of clips.
#[derive(Debug, Clone)]
pub struct AnimationList {
    pub clips: Vec<Animation>,
}

impl AnimationList {
    pub fn parse(res: &Resource) -> Result<Self> {
        Self::parse_bytes(res.header())
    }

    pub fn parse_bytes(data: &[u8]) -> Result<Self> {
        let r = Reader(data);
        let version = r.u16(0)?;
        if version != 2 {
            return Err(Error::Unsupported(format!("animation list version {version}")));
        }
        let n = r.u16(2)? as usize;
        let table = r.u32(8)? as usize;
        let clips = (0..n)
            .map(|i| {
                let off = r.u32(table + i * 4)? as usize;
                let clip = data.get(off..).ok_or_else(|| Error::Malformed(format!("clip {i} at {off:#x}")))?;
                Animation::parse_bytes(clip)
            })
            .collect::<Result<_>>()?;
        Ok(Self { clips })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_quat32() {
        let q = decode_quat32(0xDFF7_FDFF);
        assert!(q[0].abs() < 2e-3 && q[1].abs() < 2e-3 && q[2].abs() < 2e-3 && (q[3] - 1.0).abs() < 1e-5, "{q:?}");
    }

    #[test]
    fn quat32_largest_slot() {
        // Largest component index 0 (x), others at mid-range: a 180° turn about X.
        let q = decode_quat32(0x1FF << 20 | 0x1FF << 10 | 0x1FF);
        assert!((q[0] - 1.0).abs() < 1e-5, "{q:?}");
    }

    #[test]
    fn lerp_takes_short_arc() {
        let a = BoneKey::IDENTITY;
        let b = BoneKey { rotation: [0.0, 0.0, 0.0, -1.0], ..BoneKey::IDENTITY };
        let m = a.lerp(&b, 0.5);
        assert!((m.rotation[3].abs() - 1.0).abs() < 1e-6, "{m:?}");
    }
}
