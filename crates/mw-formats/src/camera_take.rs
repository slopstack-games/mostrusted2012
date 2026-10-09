//! `CameraTake` (0x20D): one take of EA's ICE in-game camera editor, 45 of them in `CAMERAS.BNDL` (listed by
//! the `CameraTakeList` "IceList", loaded by `FUN_005c44b0`, registered by `FUN_0049b000` → `FUN_00e2c4a0`).
//! A take is a set of keyed channels (eye, look-at, roll, lens, focus, shake, blends ...) played over its
//! length; a `CameraSequenceItem` plays it by UniqueId with transform bindings (see "Spaces" below). Same
//! family as Burnout Paradise's ICE Take Dictionary, but with 17 channels and 81 elements.
//!
//! # Resource (chunk 0)
//!
//! | off  | type    | meaning                                                                             |
//! |------|---------|-------------------------------------------------------------------------------------|
//! | 0x00 | u32     | 1 (one take; Burnout's dictionary entry count, inferred)                            |
//! | 0x04 | u32     | end of the take data (0x20 + take size; checked on all 45)                         |
//! | 0x08 | u32     | ICE group id (0 or 1), `FUN_00e2c4a0` → `FUN_00e2bb20` (code)                      |
//! | 0x0c | u32     | ICE group key (two values in the install), same (code)                             |
//! | 0x10 | u32 ptr | the take (0x20), same (code)                                                       |
//! | 0x14 | 12 B    | zero                                                                                |
//!
//! # Take (`ICETakeData`, at +0x20; offsets relative to it)
//!
//! | off  | type        | meaning                                                                          |
//! |------|-------------|----------------------------------------------------------------------------------|
//! | 0x00 | u32 x2      | list links (runtime)                                                             |
//! | 0x08 | u32         | UniqueId (= the resource id's low bits; what `CameraSequenceItem` names)         |
//! | 0x0c | char[28]    | name                                                                             |
//! | 0x28 | u32         | ICE group pointer (runtime, `FUN_00e2c4a0`)                                      |
//! | 0x2c | f32         | length, seconds (`FUN_00e2ac40`: fraction += dt / length)                        |
//! | 0x30 | u32         | 0 (Burnout's `muAllocated`)                                                      |
//! | 0x34 | 17 x u16 x2 | per channel: intervals, keys (`FUN_00e26fb0`)                                    |
//! | 0x78 | u16[]       | key index of intervals 1..n-2, `max(intervals - 2, 0)` per channel             |
//! |      | u16[]       | interval boundaries (fraction × 65535), `max(intervals - 1, 0)` per channel   |
//! |      | align 4     |                                                                                  |
//! |      | values      | per element in index order (`ELEMENTS`): `keys` values for a keyed element,     |
//! |      |             | `intervals` for a per-interval one; floats as f32 LE, the rest bit-packed       |
//! |      |             | MSB-first (`FUN_00e25e50`); each element's block padded to 4 bytes:            |
//! |      |             | `((count * bits + 31) >> 3) & !3` bytes                                         |
//!
//! All from code (`FUN_00e26fb0` lays the data out); the sizes add up to header +0x04 on all 45 takes.
//!
//! # Evaluation (all read from the code)
//!
//! - The take plays on a fraction `f = t / length`, clamped to 0..1 (`FUN_00e2a820`).
//! - Each channel has `n` intervals between boundaries `b(0) = 0`, `b(i) = boundary[i-1] / 65535`,
//!   `b(n) = 1`. The interval is found by a binary search on 16-bit values (`round(f × 65535)`, half up),
//!   `FUN_00e269a0`. Interval `i` runs from key `k(i)` to `k(i) + 1`: `k(0) = 0`, `k(i) = index[i-1]` for
//!   `0 < i < n-1`, `k(n-1) = keys - 2`, so a cut is a pair of keys of its own.
//! - Keyed elements are cubic Hermite between the two keys (`FUN_00e28cc0`, constants 2 and 3 at
//!   `.rdata` 0x00f9c360 / 0x00fa2e30): `((a·u + b)·u + t0)·u + p0` with `u` the fraction inside the
//!   interval, `a = t0 + t1 − 2(p1 − p0)`, `b = 3(p1 − p0) − 2·t0 − t1`. Integer elements are then rounded
//!   half up.
//! - Tangents (`FUN_00e28aa0`): `p1 − p0` (linear) unless the element's cubic flag (a per-interval element,
//!   e.g. `CUBIC_EYE`) is set; then, when the neighbouring interval on that side continues from the same key,
//!   `a·(p1 − p0) + b·(neighbour's key delta)` with `a = Lj / (L + Lj)`, `b = (1 − a)·L / Lj` (`L`, `Lj` the
//!   two intervals' lengths, both guarded by 1e-5 at 0x0107c110), all scaled by the tangent element (e.g.
//!   `TANGENT_EYE`, 0..8, default 1) at the endpoint's key.
//! - Per-interval elements hold their value for the interval. A channel with no intervals gives defaults.
//! - Fixed-point values (`FUN_00e25d80` / `FUN_00e25d10`): `max_raw = 2^bits − 1`,
//!   `up = round((max − default) · max_raw / (max − min))`, `mid = max_raw − up`; raw `r` decodes to
//!   `default + (r − mid)·(default − min)/mid` below `mid`, `default + (r − mid)·(max − default)/up` above.
//!
//! # The camera (`FUN_00e29c20`, `FUN_00e27a70`, `FUN_00e278c0`)
//!
//! ICE works in its own axes, x forward, y left, z up; the game interface converts with
//! `game = (ice.y, ice.z, ice.x)` (`FUN_00495b70` sets the camera matrix, `FUN_00496560` reads the look-at
//! target, `FUN_004ad090` converts a bound transform the other way). This module converts at the edges and
//! works in game axes: [`TakeSample::eye`] and [`TakeSample::look`] are `(EYE_Y, EYE_Z, EYE_X)` etc.
//!
//! Eye and look each live in a space chosen per interval by `SPACE_*` and `AVATAR_*`:
//! - 0 Player: the transform of avatar `AVATAR_*` (1-based: binding `AVATAR − 1` of the playing
//!   `CameraSequenceItem`, registered in order as "BoundAvatar%i" by `FUN_0049a3c0`; 0 = the ICE player's own
//!   subject). `PLAYER_SPACE_NOROLL` keeps only the heading (forward flattened, world up).
//! - 1 World: identity (eye / look are world positions).
//! - 2 Hybrid: world axes at the avatar's position.
//! - 3 Scene: a transform from the playing sequence plus a ground-height correction (not modelled).
//!
//! When `SPACE_*_2` / `AVATAR_*_2` differ from the start pair, the end space is blended in by
//! `EYE_SPACE_BLEND` / `LOOK_SPACE_BLEND` (`FUN_0049a5c0` → `FUN_00491050`; read as rotation slerp plus
//! position lerp, inferred: the 2 KB blend was not read in full). The camera looks from the eye at the
//! look point with world up (or the subject's up with `PLAYER_SPACE_LOCK_TO_RENDER_OBJECT` in Player space),
//! then rolls by `DUTCH` turns about its forward axis (`dutch × 65536` truncated to a 16-bit angle). Its field
//! of view is `2·atan(15.96 / lens)` with the lens in mm clamped to 10..500 (`FUN_00e27340`; the game's atan
//! is a 16-bit table, `FUN_00e229e0`), written as degrees to the game camera's FOV field (+0x90 via
//! `FUN_00495b20` / `FUN_00495b70`).
//!
//! Not modelled (stateful or needing the world): `PLAYER_SPACE_LAG` (`FUN_00e27580`), `PLAYER_SPACE_BUNGEE`
//! (a per-frame filter in `FUN_00e27a70`), Scene space, the shake (`FUN_00e29590`: `SHAKE_TYPE` picks a shake
//! take's `SHAKE_DATA` samples, else a procedural shake), sub-takes (channel 10, unused by the install).

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

pub const CHANNEL_COUNT: usize = 17;
pub const ELEMENT_COUNT: usize = 81;
/// No cubic / tangent element.
pub const NONE: u8 = 0xff;

/// Channel names, `.rdata` table 0x010e7858.
pub const CHANNEL_NAMES: [&str; CHANNEL_COUNT] = [
    "MAIN",
    "LENS",
    "EXPOSURE",
    "RAWFOCUS",
    "SHAKE",
    "TIME",
    "TAG",
    "OVERLAY",
    "LETTERBOX",
    "FADE",
    "ASSEMBLY",
    "SHAKE_DATA",
    "GAMEPLAY",
    "MOTION_BLUR",
    "LAG",
    "BYSTANDERCAM",
    "STEREO3D",
];

/// Space names (`SPACE_*` tokens, `.rdata` 0x010e7684).
pub const SPACE_NAMES: [&str; 4] = ["Player", "World", "Hybrid", "Scene"];
/// `SHAKE_TYPE` tokens (`.rdata` 0x010e78b0).
pub const SHAKE_NAMES: [&str; 14] = [
    "Default",
    "HandheldBig",
    "HandheldSmall",
    "Jog",
    "RunFast",
    "RunLow",
    "Still",
    "StrafeRight",
    "WalkBack",
    "WalkFast",
    "WalkForward",
    "WalkGetUp",
    "WalkLow",
    "WalkSlow",
];
/// `FADE_TO_COLOR` tokens (`.rdata` 0x010e789c).
pub const FADE_COLOUR_NAMES: [&str; 5] = ["Black", "White", "Red", "Green", "Blue"];

/// Element value types (descriptor +0x18; decoded by `FUN_00e28440`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// 0: sign-extended `bits`-bit integer.
    Int,
    /// 1: unsigned integer (or token index).
    UInt,
    /// 2: 32-bit hash.
    Hash,
    /// 3: quantised float between `min` and `max` (see the module docs).
    Fixed,
    /// 4: f32.
    Float,
}

/// One element descriptor (`ICEElementDesc`, 0x6c bytes, built by `FUN_00e2ccc0`; checked against a memory
/// snapshot of the running game).
#[derive(Debug, Clone, Copy)]
pub struct ElementDesc {
    pub name: &'static str,
    pub channel: u8,
    /// Descriptor +0x14: one value per interval (held) instead of one per key (interpolated).
    pub per_interval: bool,
    pub ty: ValueType,
    pub bits: u8,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    /// Descriptor +0x3c: the per-interval flag element that makes this one cubic, or [`NONE`].
    pub cubic: u8,
    /// Descriptor +0x40: the keyed element scaling this one's tangents, or [`NONE`].
    pub tangent: u8,
}

#[allow(clippy::too_many_arguments)]
const fn key(
    name: &'static str,
    ch: u8,
    ty: ValueType,
    bits: u8,
    d: f32,
    lo: f32,
    hi: f32,
    c: u8,
    t: u8,
) -> ElementDesc {
    ElementDesc { name, channel: ch, per_interval: false, ty, bits, default: d, min: lo, max: hi, cubic: c, tangent: t }
}

const fn step(name: &'static str, ch: u8, ty: ValueType, bits: u8, d: f32, hi: f32) -> ElementDesc {
    ElementDesc {
        name,
        channel: ch,
        per_interval: true,
        ty,
        bits,
        default: d,
        min: 0.0,
        max: hi,
        cubic: NONE,
        tangent: NONE,
    }
}

const fn tangent(name: &'static str, ch: u8) -> ElementDesc {
    key(name, ch, ValueType::Fixed, 6, 1.0, 0.0, 8.0, NONE, NONE)
}

const fn flag(name: &'static str, ch: u8, d: f32) -> ElementDesc {
    step(name, ch, ValueType::UInt, 1, d, 1.0)
}

use ValueType::{Fixed, Float, Hash, UInt};

/// Element indices (the game's descriptor slots; also the order of the values in a take).
pub mod el {
    pub const EYE_X: usize = 0;
    pub const EYE_Y: usize = 1;
    pub const EYE_Z: usize = 2;
    pub const LOOK_X: usize = 3;
    pub const LOOK_Y: usize = 4;
    pub const LOOK_Z: usize = 5;
    pub const DUTCH: usize = 6;
    pub const LENS_LENGTH: usize = 9;
    pub const FISHEYE_WARP: usize = 10;
    pub const FISHEYE_SCALE: usize = 11;
    pub const FISHEYE_POWER: usize = 12;
    pub const EXPOSURE_BIAS: usize = 14;
    pub const APERTURE: usize = 15;
    pub const NEAR_FOCUS: usize = 17;
    pub const FAR_FOCUS: usize = 18;
    pub const BLUR_FALLOFF: usize = 19;
    pub const BLUR_INTENSITY: usize = 20;
    pub const SHAKE_AMPLITUDE: usize = 22;
    pub const SHAKE_FREQUENCY: usize = 23;
    pub const TIME_SCALE: usize = 24;
    pub const LETTERBOX: usize = 25;
    pub const FADE: usize = 26;
    pub const GAME_CAM_BLEND: usize = 33;
    pub const EYE_SPACE_BLEND: usize = 34;
    pub const LOOK_SPACE_BLEND: usize = 35;
    pub const MOTION_BLUR_VEHICLE: usize = 36;
    pub const MOTION_BLUR_WORLD: usize = 37;
    pub const BYSTANDER_PERCEIVED_DISTANCE: usize = 41;
    pub const BYSTANDER_MIN_FOV: usize = 42;
    pub const BYSTANDER_MAX_FOV: usize = 43;
    pub const CUBIC_EYE: usize = 48;
    pub const CUBIC_LOOK: usize = 49;
    pub const SPACE_EYE: usize = 50;
    pub const SPACE_LOOK: usize = 51;
    pub const AVATAR_EYE: usize = 52;
    pub const AVATAR_LOOK: usize = 53;
    pub const AVOID_DYNAMIC_OBJECTS: usize = 54;
    pub const AVOID_STATIC_OBJECTS: usize = 55;
    pub const LOCK_TO_RENDER_OBJECT: usize = 56;
    pub const NOROLL: usize = 57;
    pub const LAG: usize = 58;
    pub const BUNGEE: usize = 59;
    pub const SPACE_EYE_2: usize = 60;
    pub const SPACE_LOOK_2: usize = 61;
    pub const AVATAR_EYE_2: usize = 62;
    pub const AVATAR_LOOK_2: usize = 63;
    pub const LENS_OVERRIDE: usize = 65;
    pub const RAWFOCUS_OVERRIDE: usize = 68;
    pub const SHAKE_TYPE: usize = 69;
    pub const EVENT_TAG: usize = 70;
    pub const OVERLAY: usize = 71;
    pub const FADE_TO_COLOR: usize = 72;
    pub const ENVMAP_OVERRIDE: usize = 76;
    pub const BYSTANDER_ENABLE: usize = 79;
}

/// The 81 element descriptors, by index (`FUN_00e2ccc0`; defaults / ranges from `.rdata`).
pub const ELEMENTS: [ElementDesc; ELEMENT_COUNT] = [
    key("EYE_X", 0, Float, 32, -4.7, -1e5, 1e5, 0x30, 7),
    key("EYE_Y", 0, Float, 32, 0.0, -1e5, 1e5, 0x30, 7),
    key("EYE_Z", 0, Float, 32, 1.6, -1e5, 1e5, 0x30, 7),
    key("LOOK_X", 0, Float, 32, 0.0, -1e5, 1e5, 0x31, 8),
    key("LOOK_Y", 0, Float, 32, 0.0, -1e5, 1e5, 0x31, 8),
    key("LOOK_Z", 0, Float, 32, 0.0, -1e5, 1e5, 0x31, 8),
    key("DUTCH", 0, Fixed, 10, 0.0, -0.25, 0.25, 0x30, 7),
    tangent("TANGENT_EYE", 0),
    tangent("TANGENT_LOOK", 0),
    key("LENS_LENGTH", 0, Fixed, 9, 24.0, 10.0, 500.0, 0x30, 7),
    key("LENS_FISHEYE_WARP", 1, Fixed, 7, 0.0, -0.5, 0.5, 0x40, 0xd),
    key("LENS_FISHEYE_SCALE", 1, Fixed, 7, 1.0, 0.5, 2.0, 0x40, 0xd),
    key("LENS_FISHEYE_POWER", 1, Fixed, 7, 2.0, 0.0, 2.0, 0x40, 0xd),
    tangent("TANGENT_LENS", 1),
    key("EXPOSURE_BIAS", 2, Fixed, 8, 0.0, -8.0, 8.0, 0x42, 0x10),
    key("APERTURE", 2, UInt, 6, 37.0, 0.0, 37.0, 0x42, 0x10),
    tangent("TANGENT_EXPOSURE", 2),
    key("NEAR_FOCUS", 3, Fixed, 16, 0.0, 0.0, 10000.0, 0x43, 0x15),
    key("FAR_FOCUS", 3, Fixed, 16, 0.0, 0.0, 10000.0, 0x43, 0x15),
    key("BLUR_FALLOFF", 3, Fixed, 7, 0.0, 0.0, 1.0, 0x43, 0x15),
    key("BLUR_INTENSITY", 3, Fixed, 7, 0.0, 0.0, 1.0, 0x43, 0x15),
    tangent("TANGENT_RAWFOCUS", 3),
    key("SHAKE_AMPLITUDE", 4, Fixed, 7, 0.0, 0.0, 1.0, NONE, NONE),
    key("SHAKE_FREQUENCY", 4, Fixed, 7, 0.0, 0.0, 1.0, NONE, NONE),
    key("TIME_SCALE", 5, UInt, 7, 100.0, 0.0, 100.0, NONE, NONE),
    key("LETTERBOX", 8, UInt, 7, 0.0, 0.0, 100.0, NONE, NONE),
    key("FADE", 9, UInt, 7, 0.0, 0.0, 100.0, NONE, NONE),
    key("SHAKE_QUAT_X", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("SHAKE_QUAT_Y", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("SHAKE_QUAT_Z", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("SHAKE_POS_X", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("SHAKE_POS_Y", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("SHAKE_POS_Z", 11, Fixed, 16, 0.0, -1.0, 1.0, NONE, NONE),
    key("GAME_CAM_BLEND", 12, Float, 32, 0.0, 0.0, 1.0, NONE, NONE),
    key("EYE_SPACE_BLEND", 12, Float, 32, 0.0, 0.0, 1.0, NONE, NONE),
    key("LOOK_SPACE_BLEND", 12, Float, 32, 0.0, 0.0, 1.0, NONE, NONE),
    key("MOTION_BLUR_VEHICLE", 13, Float, 32, 0.0, 0.0, 1.0, NONE, NONE),
    key("MOTION_BLUR_WORLD", 13, Float, 32, 0.0, 0.0, 1.0, NONE, NONE),
    key("LAG_ACCELERATION_MAX", 14, Float, 32, 1.0, 0.1, 100.0, 0x4d, 0x28),
    key("LAG_ACCELERATION_SCALE", 14, Float, 32, 0.05, 0.0, 1.0, 0x4d, 0x28),
    tangent("TANGENT_LAG", 14),
    key("BYSTANDER_PERCEIVED_DISTANCE", 15, Fixed, 16, 10.0, 0.0, 1000.0, 0x4e, 0x2c),
    key("BYSTANDER_MIN_FOV", 15, Fixed, 12, 10.0, 10.0, 180.0, 0x4e, 0x2c),
    key("BYSTANDER_MAX_FOV", 15, Fixed, 12, 90.0, 10.0, 180.0, 0x4e, 0x2c),
    tangent("TANGENT_BYSTANDER", 15),
    key("S3D_DISTORTION", 16, Float, 32, 0.5, 0.0, 1.0, 0x50, 0x2f),
    key("S3D_SCREENDISTANCE", 16, Float, 32, 0.0, 0.0, 100.0, 0x50, 0x2f),
    tangent("TANGENT_STEREO3D", 16),
    flag("CUBIC_EYE", 0, 1.0),
    flag("CUBIC_LOOK", 0, 1.0),
    step("SPACE_EYE", 0, UInt, 3, 0.0, 4.0),
    step("SPACE_LOOK", 0, UInt, 3, 0.0, 4.0),
    step("AVATAR_EYE", 0, UInt, 5, 0.0, 31.0),
    step("AVATAR_LOOK", 0, UInt, 5, 0.0, 31.0),
    flag("AVOID_DYNAMIC_OBJECTS", 0, 0.0),
    flag("AVOID_STATIC_OBJECTS", 0, 0.0),
    flag("PLAYER_SPACE_LOCK_TO_RENDER_OBJECT", 0, 0.0),
    flag("PLAYER_SPACE_NOROLL", 0, 0.0),
    flag("PLAYER_SPACE_LAG", 0, 0.0),
    flag("PLAYER_SPACE_BUNGEE", 0, 0.0),
    step("SPACE_EYE_2", 0, UInt, 3, 0.0, 4.0),
    step("SPACE_LOOK_2", 0, UInt, 3, 0.0, 4.0),
    step("AVATAR_EYE_2", 0, UInt, 5, 0.0, 31.0),
    step("AVATAR_LOOK_2", 0, UInt, 5, 0.0, 31.0),
    flag("CUBIC_LENS", 1, 1.0),
    flag("LENS_OVERRIDE", 1, 0.0),
    flag("CUBIC_EXPOSURE", 2, 1.0),
    flag("CUBIC_RAWFOCUS", 3, 1.0),
    flag("RAWFOCUS_OVERRIDE", 3, 0.0),
    step("SHAKE_TYPE", 4, UInt, 5, 0.0, 14.0),
    step("EVENT_TAG", 6, Hash, 32, 0.0, 268_435_455.0),
    step("OVERLAY", 7, UInt, 4, 0.0, 15.0),
    step("FADE_TO_COLOR", 9, UInt, 3, 0.0, 5.0),
    step("TAKE_START", 10, Fixed, 16, 0.0, 1.0),
    step("TAKE_NUMBER", 10, UInt, 6, 0.0, 63.0),
    flag("CONTAINS_SUBTAKE", 10, 0.0),
    flag("ENVMAP_OVERRIDE", 12, 0.0),
    flag("CUBIC_LAG", 14, 1.0),
    flag("CUBIC_BYSTANDER", 15, 1.0),
    flag("BYSTANDER_ENABLE", 15, 0.0),
    flag("CUBIC_STEREO3D", 16, 1.0),
];

/// Round half up, as `FUN_00e21f00` / `FUN_00e21f60` / `FUN_00e26f30` do.
fn round_half_up(x: f64) -> f64 {
    (x + 0.5).floor()
}

impl ElementDesc {
    /// `(up steps, mid)` of a fixed-point element (descriptor +0x2c / +0x34, `FUN_00e25d80`).
    pub fn fixed_steps(&self) -> (f64, f64) {
        let max_raw = ((1u64 << self.bits.min(32)) - 1) as f64;
        let span = self.max as f64 - self.min as f64;
        let up = if span > 0.0 { round_half_up((self.max as f64 - self.default as f64) * max_raw / span) } else { 0.0 };
        (up, max_raw - up)
    }

    /// The value of a raw (stored) word: floats from their bits, integers as their value, fixed point decoded
    /// (`FUN_00e28440`). Hashes become f32 as in the game, so large ones lose precision.
    pub fn decode(&self, raw: u32) -> f32 {
        match self.ty {
            ValueType::Float => f32::from_bits(raw),
            ValueType::Int => {
                let shift = 32 - u32::from(self.bits.clamp(1, 32));
                (((raw << shift) as i32) >> shift) as f32
            }
            ValueType::UInt | ValueType::Hash => raw as f32,
            ValueType::Fixed => {
                let (up, mid) = self.fixed_steps();
                let d = raw as f64 - mid;
                let def = self.default as f64;
                if d < 0.0 && mid > 0.0 {
                    (d * (def - self.min as f64) / mid + def) as f32
                } else if d > 0.0 && up > 0.0 {
                    (d * (self.max as f64 - def) / up + def) as f32
                } else {
                    self.default
                }
            }
        }
    }

    fn is_float(&self) -> bool {
        matches!(self.ty, ValueType::Fixed | ValueType::Float)
    }

    /// The default as the game stores it in its value array (integers truncated).
    fn stored_default(&self) -> f32 {
        if self.is_float() { self.default } else { self.default.trunc() }
    }
}

/// One channel's interval / key layout.
#[derive(Debug, Clone, Default)]
pub struct ChannelLayout {
    pub intervals: u16,
    pub keys: u16,
    /// Key index of intervals 1..intervals-2.
    pub key_index: Vec<u16>,
    /// Interval boundaries as fraction × 65535 (between intervals 0|1, 1|2, ...).
    pub boundaries: Vec<u16>,
}

impl ChannelLayout {
    /// Boundary `m` as a 16-bit fraction: `b(0) = 0`, `b(intervals) = 65535`.
    fn boundary_u16(&self, m: usize) -> u16 {
        if m == 0 {
            0
        } else if m < self.intervals as usize {
            self.boundaries.get(m - 1).copied().unwrap_or(u16::MAX)
        } else {
            u16::MAX
        }
    }

    fn boundary(&self, m: usize) -> f32 {
        self.boundary_u16(m) as f32 / 65535.0
    }

    /// Length of interval `i` as a fraction of the take (`FUN_00e24f50`).
    fn duration(&self, i: usize) -> f32 {
        self.boundary(i + 1) - self.boundary(i)
    }

    /// First key of interval `i` (`FUN_00e28cc0`, `FUN_00e28aa0`).
    pub fn key_of_interval(&self, i: usize) -> usize {
        if i == 0 {
            0
        } else if i + 1 < self.intervals as usize {
            self.key_index.get(i - 1).map_or(0, |&k| k as usize)
        } else {
            (self.keys as usize).saturating_sub(2)
        }
    }

    /// The interval at take fraction `f` and its start / end fractions (`FUN_00e269a0`).
    pub fn interval_at(&self, f: f32) -> (usize, f32, f32) {
        let n = self.intervals as usize;
        if n == 0 {
            return (0, 0.0, 1.0);
        }
        // FUN_00e24870: clamp, × 65535 in f32, round half up.
        let q = (f.clamp(0.0, 1.0) * 65535.0) as f64;
        let t = round_half_up(q).min(65535.0) as u16;
        let (mut lo, mut hi) = (0usize, n - 1);
        let (mut start, mut end) = (0u16, u16::MAX);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let b = self.boundary_u16(mid + 1);
            if t < b {
                end = b;
                hi = mid;
            } else {
                start = b;
                lo = mid + 1;
            }
        }
        (lo, start as f32 / 65535.0, end as f32 / 65535.0)
    }
}

/// A parsed take.
#[derive(Debug, Clone)]
pub struct CameraTake {
    /// Header +0x08 / +0x0c: the ICE group the take is registered in.
    pub group: u32,
    pub group_key: u32,
    pub unique_id: u32,
    pub name: String,
    /// Seconds.
    pub length: f32,
    pub channels: Vec<ChannelLayout>,
    /// Raw words per element (f32 bits for floats).
    pub raw: Vec<Vec<u32>>,
}

/// Read value `i` of a bit-packed block, MSB first (`FUN_00e25e50`).
fn read_bits(block: &[u8], bits: usize, i: usize) -> Option<u32> {
    let start = bits * i;
    let mut v = 0u64;
    for b in start..start + bits {
        let byte = *block.get(b >> 3)?;
        v = (v << 1) | u64::from((byte >> (7 - (b & 7))) & 1);
    }
    Some(v as u32)
}

impl CameraTake {
    pub fn parse(res: &Resource) -> Result<Self> {
        Self::parse_bytes(res.header())
    }

    pub fn parse_bytes(data: &[u8]) -> Result<Self> {
        let r = Reader(data);
        let entries = r.u32(0)?;
        if entries != 1 {
            return Err(Error::Unsupported(format!("CameraTake with {entries} entries")));
        }
        let end = r.u32(4)? as usize;
        let group = r.u32(8)?;
        let group_key = r.u32(0xc)?;
        let t = r.u32(0x10)? as usize;
        let unique_id = r.u32(t + 8)?;
        let name_bytes = r.bytes(t + 0xc, 28)?;
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..name_len]).into_owned();
        let length = r.f32(t + 0x2c)?;
        let mut channels = Vec::with_capacity(CHANNEL_COUNT);
        for c in 0..CHANNEL_COUNT {
            let intervals = r.u16(t + 0x34 + c * 4)?;
            let keys = r.u16(t + 0x36 + c * 4)?;
            channels.push(ChannelLayout { intervals, keys, ..Default::default() });
        }
        let mut p = t + 0x78;
        for ch in channels.iter_mut() {
            for _ in 0..(ch.intervals as usize).saturating_sub(2) {
                ch.key_index.push(r.u16(p)?);
                p += 2;
            }
        }
        for ch in channels.iter_mut() {
            for _ in 0..(ch.intervals as usize).saturating_sub(1) {
                ch.boundaries.push(r.u16(p)?);
                p += 2;
            }
        }
        p = (p + 3) & !3;
        let mut raw = Vec::with_capacity(ELEMENT_COUNT);
        for e in &ELEMENTS {
            let ch = &channels[e.channel as usize];
            let count = if e.per_interval { ch.intervals } else { ch.keys } as usize;
            let bits = e.bits as usize;
            let size = ((count * bits + 31) >> 3) & !3;
            let block = r.bytes(p, size)?;
            let mut vals = Vec::with_capacity(count);
            for i in 0..count {
                let v = if e.ty == ValueType::Float {
                    Reader(block).u32(i * 4)?
                } else {
                    read_bits(block, bits, i).ok_or_else(|| Error::Malformed(format!("{} value {i}", e.name)))?
                };
                vals.push(v);
            }
            raw.push(vals);
            p += size;
        }
        if p != end {
            return Err(Error::Malformed(format!("take {name}: data ends at {p:#x}, header says {end:#x}")));
        }
        Ok(Self { group, group_key, unique_id, name, length, channels, raw })
    }

    /// Element `e`'s decoded value number `i` (a key or an interval), the default when its channel is empty
    /// (`FUN_00e28950`); integer types truncated as the game stores them.
    pub fn value(&self, e: usize, i: usize) -> f32 {
        let d = &ELEMENTS[e];
        if self.channels[d.channel as usize].intervals == 0 {
            return d.stored_default();
        }
        let v = self.raw[e].get(i).map_or(d.default, |&w| d.decode(w));
        if d.is_float() { v } else { v.trunc() }
    }

    /// Hermite tangent of keyed element `e` in interval `i` at its start (`end = false`) or end
    /// (`FUN_00e28aa0`), in value per interval.
    fn tangent(&self, e: usize, i: usize, end: bool, delta: f32) -> f32 {
        let d = &ELEMENTS[e];
        if d.cubic == NONE {
            return delta;
        }
        if self.value(d.cubic as usize, i) == 0.0 {
            return delta;
        }
        let ch = &self.channels[d.channel as usize];
        let n = ch.intervals as usize;
        let k = ch.key_of_interval(i) + end as usize;
        let mut t = delta;
        // The neighbouring interval on this side, if it continues from the same key.
        let j = if end { Some(i + 1) } else { i.checked_sub(1) };
        if let Some(j) = j.filter(|&j| j < n) {
            let kj = ch.key_of_interval(j);
            if k == kj + (!end) as usize {
                let dn = self.value(e, kj + 1) - self.value(e, kj);
                let (l, lj) = (ch.duration(i), ch.duration(j));
                let (mut a, mut b) = (1.0f32, 0.0f32);
                if 1e-5 < l + lj {
                    a = lj / (l + lj);
                    b = 1.0 - a;
                }
                if 1e-5 < lj {
                    b *= l / lj;
                }
                t = dn * b + a * delta;
            }
        }
        let scale = if d.tangent == NONE { 1.0 } else { self.value(d.tangent as usize, k) };
        scale * t
    }

    /// Take fraction at `seconds` (clamped to the take).
    pub fn fraction(&self, seconds: f32) -> f32 {
        if self.length > 0.0 { (seconds / self.length).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Every element's value at `seconds` into the take.
    pub fn sample(&self, seconds: f32) -> TakeSample {
        self.sample_fraction(self.fraction(seconds))
    }

    /// Every element's value at take fraction `f` (`FUN_00e2a820` → `FUN_00e28cc0` per channel).
    pub fn sample_fraction(&self, f: f32) -> TakeSample {
        let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) };
        let mut values = [0.0f32; ELEMENT_COUNT];
        let mut intervals = [0usize; CHANNEL_COUNT];
        for (c, ch) in self.channels.iter().enumerate() {
            let (i, start, end) = ch.interval_at(f);
            intervals[c] = i;
            let u = if end > start { (f - start) / (end - start) } else { 0.0 };
            let k = ch.key_of_interval(i);
            for (e, d) in ELEMENTS.iter().enumerate().filter(|(_, d)| d.channel as usize == c) {
                values[e] = if ch.intervals == 0 {
                    d.stored_default()
                } else if d.per_interval {
                    self.value(e, i)
                } else {
                    let (p0, p1) = (self.value(e, k), self.value(e, k + 1));
                    let t0 = self.tangent(e, i, false, p1 - p0);
                    let t1 = self.tangent(e, i, true, p1 - p0);
                    let a = t1 + t0 - (p1 - p0) * 2.0;
                    let b = (p1 - p0) * 3.0 - t1 - t0 * 2.0;
                    let v = ((a * u + b) * u + t0) * u + p0;
                    if d.is_float() { v } else { round_half_up(v as f64) as f32 }
                };
            }
        }
        TakeSample { fraction: f, values, intervals }
    }

    /// The camera at `seconds` into the take, with the transforms it is bound to.
    pub fn camera(&self, seconds: f32, bindings: &Bindings) -> IceCamera {
        self.sample(seconds).camera(bindings)
    }
}

/// All elements evaluated at one moment: the game's value array (ICE player state +0x10 + 4·n).
#[derive(Debug, Clone)]
pub struct TakeSample {
    pub fraction: f32,
    pub values: [f32; ELEMENT_COUNT],
    /// The interval each channel is in.
    pub intervals: [usize; CHANNEL_COUNT],
}

/// A space choice for the eye or the look point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    /// 0 Player, 1 World, 2 Hybrid, 3 Scene ([`SPACE_NAMES`]).
    pub kind: u8,
    /// 0 = the ICE player's own subject, n = binding n − 1.
    pub avatar: u8,
}

impl Space {
    pub fn name(self) -> &'static str {
        SPACE_NAMES.get(self.kind as usize).copied().unwrap_or("?")
    }
}

impl TakeSample {
    /// Value of element `e` as an integer (`FUN_00e26f30`: floats rounded half up).
    pub fn int(&self, e: usize) -> i32 {
        round_half_up(self.values[e] as f64) as i32
    }

    fn vec(&self, x: usize, y: usize, z: usize) -> [f32; 3] {
        // ICE (x forward, y left, z up) → game (ice.y, ice.z, ice.x).
        [self.values[y], self.values[z], self.values[x]]
    }

    /// The eye in its space, game axes.
    pub fn eye(&self) -> [f32; 3] {
        self.vec(el::EYE_X, el::EYE_Y, el::EYE_Z)
    }

    /// The look-at point in its space, game axes.
    pub fn look(&self) -> [f32; 3] {
        self.vec(el::LOOK_X, el::LOOK_Y, el::LOOK_Z)
    }

    fn space(&self, kind: usize, avatar: usize) -> Space {
        Space { kind: self.int(kind).clamp(0, 255) as u8, avatar: self.int(avatar).clamp(0, 255) as u8 }
    }

    pub fn eye_space(&self) -> Space {
        self.space(el::SPACE_EYE, el::AVATAR_EYE)
    }
    pub fn eye_space_end(&self) -> Space {
        self.space(el::SPACE_EYE_2, el::AVATAR_EYE_2)
    }
    pub fn look_space(&self) -> Space {
        self.space(el::SPACE_LOOK, el::AVATAR_LOOK)
    }
    pub fn look_space_end(&self) -> Space {
        self.space(el::SPACE_LOOK_2, el::AVATAR_LOOK_2)
    }

    /// Lens focal length, mm, clamped to the element's range (`FUN_00e27340`).
    pub fn lens_mm(&self) -> f32 {
        self.values[el::LENS_LENGTH].clamp(10.0, 500.0)
    }

    /// Field of view, degrees: `2·atan(15.96 / lens)` (`FUN_00e27340`, 15.96 at 0x0107c190).
    pub fn fov_degrees(&self) -> f32 {
        2.0 * 15.96f32.atan2(self.lens_mm()).to_degrees()
    }

    /// The 16-bit roll angle `FUN_00e29c20` passes on (`DUTCH × 65536`, truncated).
    pub fn dutch_angle(&self) -> u16 {
        ((self.values[el::DUTCH] as f64 * 65536.0) as i32) as u16
    }

    /// Roll about the view direction, degrees (−180..180).
    pub fn dutch_degrees(&self) -> f32 {
        self.dutch_angle() as i16 as f32 * (360.0 / 65536.0)
    }

    /// World time scale the take requests (`FUN_00e27420`: `TIME_SCALE × 0.01` clamped to 0.01..1).
    pub fn time_scale(&self) -> f32 {
        (self.values[el::TIME_SCALE] * 0.01).clamp(0.01, 1.0)
    }

    /// Weight of the gameplay camera against the take's (`FUN_004962f0(0)`, blended by `FUN_005e09f0` in
    /// `FUN_005c9a70`); 0 = the take's camera (by the data: takes start and end at 1).
    pub fn game_camera_blend(&self) -> f32 {
        self.values[el::GAME_CAM_BLEND]
    }

    /// The scripted camera's depth-of-field / post values as `FUN_005c9a70` stores them via
    /// `FUN_004962f0`: (far focus +0xa8, blur intensity³ × 1000 +0xa4, blur falloff × 1000 +0xac).
    /// The cube is `FUN_00ed97b0` with 3.0, read as `pow` (inferred).
    pub fn focus(&self) -> [f32; 3] {
        [
            self.values[el::FAR_FOCUS],
            self.values[el::BLUR_INTENSITY].powi(3) * 1000.0,
            self.values[el::BLUR_FALLOFF] * 1000.0,
        ]
    }

    /// The camera given the bound transforms.
    pub fn camera(&self, b: &Bindings) -> IceCamera {
        let mut approx = Approximations {
            lag: self.int(el::LAG) != 0,
            bungee: self.int(el::BUNGEE) != 0,
            shake: self.values[el::SHAKE_AMPLITUDE] != 0.0,
            scene_space: false,
        };
        let noroll = self.int(el::NOROLL) != 0;
        let mut place = |p: [f32; 3], s: Space, s2: Space, blend: usize| {
            approx.scene_space |= s.kind == 3 || (s2 != s && s2.kind == 3);
            let mut m = space_frame(s, b, noroll);
            if s2 != s {
                m = Frame::blend(&m, &space_frame(s2, b, noroll), self.values[blend]);
            }
            m.transform_point(p)
        };
        let eye = place(self.eye(), self.eye_space(), self.eye_space_end(), el::EYE_SPACE_BLEND);
        let look = place(self.look(), self.look_space(), self.look_space_end(), el::LOOK_SPACE_BLEND);
        // FUN_00e278c0: forward to the target, left = up × forward, up = forward × left, then the roll.
        let lock = self.eye_space().kind == 0 && self.int(el::LOCK_TO_RENDER_OBJECT) != 0;
        let up_hint = if lock { b.subject.y } else { [0.0, 1.0, 0.0] };
        let z = normalize_or(sub(look, eye), [0.0, 0.0, 1.0]);
        let x = normalize_or(cross(up_hint, z), [0.0, 0.0, 1.0]);
        let y = cross(z, x);
        let (s, c) = (self.dutch_angle() as f32 * (std::f32::consts::TAU / 65536.0)).sin_cos();
        let frame = Frame { x: add(scale(x, c), scale(y, s)), y: add(scale(x, -s), scale(y, c)), z, pos: eye };
        IceCamera {
            eye,
            look,
            frame,
            fov_degrees: self.fov_degrees(),
            lens_mm: self.lens_mm(),
            dutch_degrees: self.dutch_degrees(),
            game_camera_blend: self.game_camera_blend(),
            approximations: approx,
        }
    }
}

/// The transform of a space (`FUN_00e27a70`), game axes.
fn space_frame(s: Space, b: &Bindings, noroll: bool) -> Frame {
    let bound =
        || if s.avatar == 0 { b.subject } else { b.avatars.get(s.avatar as usize - 1).copied().unwrap_or(b.subject) };
    match s.kind {
        0 => {
            let mut m = bound();
            if noroll {
                // Forward flattened to the horizontal, world up, left = up × forward.
                let z = normalize_or([m.z[0], 0.0, m.z[2]], [0.0, 0.0, 1.0]);
                let x = cross([0.0, 1.0, 0.0], z);
                m = Frame { x, y: cross(z, x), z, pos: m.pos };
            }
            m
        }
        2 => Frame { pos: bound().pos, ..Frame::IDENTITY },
        _ => Frame::IDENTITY,
    }
}

/// A transform in the game's convention: rows x, y, z and the position, `v · M`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: [f32; 3],
    pub y: [f32; 3],
    pub z: [f32; 3],
    pub pos: [f32; 3],
}

impl Frame {
    pub const IDENTITY: Frame = Frame { x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0], pos: [0.0; 3] };

    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        add(add(add(scale(self.x, p[0]), scale(self.y, p[1])), scale(self.z, p[2])), self.pos)
    }

    /// Rotation slerp and position lerp (the reading of `FUN_0049a5c0` / `FUN_00491050`, inferred).
    pub fn blend(a: &Frame, b: &Frame, t: f32) -> Frame {
        if t <= 0.0 {
            return *a;
        }
        if t >= 1.0 {
            return *b;
        }
        let (qa, mut qb) = (a.quat(), b.quat());
        let mut d: f32 = (0..4).map(|i| qa[i] * qb[i]).sum();
        if d < 0.0 {
            qb = qb.map(|v| -v);
            d = -d;
        }
        let (wa, wb) = if d > 0.9995 {
            (1.0 - t, t)
        } else {
            let th = d.acos();
            (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin())
        };
        let q: [f32; 4] = std::array::from_fn(|i| qa[i] * wa + qb[i] * wb);
        let mut m = Frame::from_quat(q);
        m.pos = add(scale(a.pos, 1.0 - t), scale(b.pos, t));
        m
    }

    /// Rotation as a quaternion (x, y, z, w) for row vectors.
    fn quat(&self) -> [f32; 4] {
        let m = [self.x, self.y, self.z];
        let tr = m[0][0] + m[1][1] + m[2][2];
        let q = if tr > 0.0 {
            let s = (tr + 1.0).sqrt() * 2.0;
            [(m[1][2] - m[2][1]) / s, (m[2][0] - m[0][2]) / s, (m[0][1] - m[1][0]) / s, 0.25 * s]
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            [0.25 * s, (m[0][1] + m[1][0]) / s, (m[2][0] + m[0][2]) / s, (m[1][2] - m[2][1]) / s]
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[2][0] - m[0][2]) / s]
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            [(m[2][0] + m[0][2]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[0][1] - m[1][0]) / s]
        };
        let n = q.iter().map(|v| v * v).sum::<f32>().sqrt();
        if n > 0.0 { q.map(|v| v / n) } else { [0.0, 0.0, 0.0, 1.0] }
    }

    fn from_quat([x, y, z, w]: [f32; 4]) -> Frame {
        Frame {
            x: [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
            y: [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
            z: [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
            pos: [0.0; 3],
        }
    }
}

/// The transforms a take is played against, game space.
#[derive(Debug, Clone, Copy)]
pub struct Bindings<'a> {
    /// Avatar 0: the ICE player's own subject (the game interface's transform). Also the fallback for a
    /// missing binding (a model decision: the game returns a default slot then).
    pub subject: Frame,
    /// The `CameraSequenceItem` bindings in order (avatar n = `avatars[n - 1]`).
    pub avatars: &'a [Frame],
}

/// Parts of the original's camera the evaluation skipped at this moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Approximations {
    pub lag: bool,
    pub bungee: bool,
    pub shake: bool,
    pub scene_space: bool,
}

/// The evaluated camera, game axes.
#[derive(Debug, Clone, Copy)]
pub struct IceCamera {
    pub eye: [f32; 3],
    pub look: [f32; 3],
    /// Camera matrix as `FUN_00495b70` hands it to the game: x left, y up, z forward (after the roll), pos =
    /// the eye.
    pub frame: Frame,
    pub fov_degrees: f32,
    pub lens_mm: f32,
    pub dutch_degrees: f32,
    pub game_camera_blend: f32,
    pub approximations: Approximations,
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// `FUN_00e264b0`: zero length gives ICE +x (game +z).
fn normalize_or(a: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-point steps as the running game computed them (descriptor +0x2c / +0x34 in a memory snapshot).
    #[test]
    fn fixed_steps_match_the_game() {
        for (e, up, mid) in [
            (el::DUTCH, 512.0, 511.0),
            (7, 55.0, 8.0),
            (el::LENS_LENGTH, 496.0, 15.0),
            (el::FISHEYE_SCALE, 85.0, 42.0),
            (el::FISHEYE_POWER, 0.0, 127.0),
            (el::EXPOSURE_BIAS, 128.0, 127.0),
            (el::NEAR_FOCUS, 65535.0, 0.0),
            (27, 32768.0, 32767.0),
            (el::BYSTANDER_PERCEIVED_DISTANCE, 64880.0, 655.0),
            (el::BYSTANDER_MIN_FOV, 4095.0, 0.0),
            (el::BYSTANDER_MAX_FOV, 2168.0, 1927.0),
        ] {
            assert_eq!(ELEMENTS[e].fixed_steps(), (up, mid), "{}", ELEMENTS[e].name);
        }
        assert_eq!(ELEMENTS[el::DUTCH].decode(511), 0.0);
        assert_eq!(ELEMENTS[el::DUTCH].decode(1023), 0.25);
        assert_eq!(ELEMENTS[7].decode(8), 1.0);
    }

    /// Writes `vals` bit-packed MSB first, padded to 4 bytes.
    fn pack(out: &mut Vec<u8>, bits: usize, vals: &[u32]) {
        let size = ((vals.len() * bits + 31) >> 3) & !3;
        let base = out.len();
        out.resize(base + size, 0);
        for (i, &v) in vals.iter().enumerate() {
            for k in 0..bits {
                if (v >> (bits - 1 - k)) & 1 != 0 {
                    let b = i * bits + k;
                    out[base + (b >> 3)] |= 0x80 >> (b & 7);
                }
            }
        }
    }

    /// A synthetic take: MAIN with 2 intervals over 3 keys (eye moves, then holds), linear eye.
    fn synthetic() -> Vec<u8> {
        let mut d = vec![0u8; 0x20];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[0x10..0x14].copy_from_slice(&0x20u32.to_le_bytes());
        let mut t = vec![0u8; 0x78];
        t[8..12].copy_from_slice(&1234u32.to_le_bytes());
        t[0xc..0xc + 4].copy_from_slice(b"Test");
        t[0x2c..0x30].copy_from_slice(&10.0f32.to_le_bytes());
        t[0x34..0x38].copy_from_slice(&[2, 0, 3, 0]); // MAIN: 2 intervals, 3 keys
        d.extend_from_slice(&t);
        d.extend_from_slice(&32768u16.to_le_bytes()); // boundary at ~0.5; no key indices
        while d.len() % 4 != 0 {
            d.push(0);
        }
        for (e, desc) in ELEMENTS.iter().enumerate() {
            if desc.channel != 0 {
                continue;
            }
            let count = if desc.per_interval { 2 } else { 3 };
            if desc.ty == ValueType::Float {
                let v: [f32; 3] = match e {
                    el::EYE_X => [0.0, 10.0, 10.0],
                    el::EYE_Z => [2.0, 2.0, 2.0],
                    el::LOOK_X => [20.0, 20.0, 20.0],
                    el::LOOK_Z => [2.0, 2.0, 2.0],
                    _ => [0.0; 3],
                };
                for x in &v[..count] {
                    d.extend_from_slice(&x.to_le_bytes());
                }
            } else {
                let raw = match e {
                    el::DUTCH => 511,
                    7 | 8 => 8,                          // tangent scale 1
                    el::LENS_LENGTH => 15,               // 24 mm
                    el::CUBIC_EYE | el::CUBIC_LOOK => 0, // linear
                    el::AVATAR_EYE | el::AVATAR_LOOK => 1,
                    _ => 0,
                };
                pack(&mut d, desc.bits as usize, &[raw; 3][..count]);
            }
        }
        let end = d.len() as u32;
        d[4..8].copy_from_slice(&end.to_le_bytes());
        d
    }

    #[test]
    fn parse_and_evaluate() {
        let take = CameraTake::parse_bytes(&synthetic()).unwrap();
        assert_eq!(take.unique_id, 1234);
        assert_eq!(take.name, "Test");
        let s = take.sample(0.0);
        assert_eq!(s.eye(), [0.0, 2.0, 0.0]);
        let s = take.sample(2.5); // u = 0.5 in the first interval (linear)
        assert!((s.values[el::EYE_X] - 5.0).abs() < 1e-3, "{}", s.values[el::EYE_X]);
        let s = take.sample(8.0); // hold
        assert_eq!(s.values[el::EYE_X], 10.0);
        assert_eq!(s.eye_space(), Space { kind: 0, avatar: 1 });
        assert!((s.fov_degrees() - 67.25).abs() < 0.05, "{}", s.fov_degrees());
        // Bound to a car 100 m along +x, turned 90 degrees (its forward is world +x).
        let car = Frame { x: [0.0, 0.0, -1.0], y: [0.0, 1.0, 0.0], z: [1.0, 0.0, 0.0], pos: [100.0, 0.0, 0.0] };
        let cam = s.camera(&Bindings { subject: Frame::IDENTITY, avatars: &[car] });
        assert!((cam.eye[0] - 110.0).abs() < 1e-4 && (cam.eye[1] - 2.0).abs() < 1e-4, "{:?}", cam.eye);
        assert!((cam.frame.z[0] - 1.0).abs() < 1e-4, "{:?}", cam.frame.z);
    }

    #[test]
    fn rejects_truncated() {
        let d = synthetic();
        assert!(CameraTake::parse_bytes(&d[..d.len() - 4]).is_err());
        assert!(CameraTake::parse_bytes(&d[..0x40]).is_err());
    }
}
