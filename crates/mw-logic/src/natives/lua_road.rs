//! The last Lua natives (handlers read from the exe's code, `re/out/live/text.asm`, callees from the dump;
//! research log 2026-11-18, "Lua natives, last batch"): the vector maths (`CalculateInterpolatedLocation`,
//! `CalculateRotatedLocationXZ`, `IsCompetitorWithinVisibilityCone`), the road network and route natives, the
//! teleports / resets / spawn, world streaming, the camera target and ICE camera, `ShowFeedbackMessage`,
//! `ShowLoadingScreen`, `GetValue`, the debug stubs and the PC platform natives. State the handlers read is set by
//! the runtime ([`RoadFields`]); the natives that act on the engine record a [`Request`].
//!
//! The maths is the handlers' SSE code lane for lane: the sums in its order, the vectorised cosine
//! ([`cos_sse`]) with its `.rdata` constants. One model decision covers every normalisation: `rsqrtps` (an
//! estimate whose bits are the CPU's) is replaced by the exact `1/sqrt`, the two Newton steps the code applies
//! after it are kept ([`rsqrt_nr`]); the result differs from the original's by an ulp at most.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mw_formats::ZoneList;
use mw_formats::road_data::RoadData;
use mw_lua::{Lua, Value};

use super::feedback_queue::{Part, PartValue};
use super::lua_args::{fit, ftol32, int, push_str, push_uint, string_or_invalid, toboolean, tonumber};
use super::lua_vehicles::table_vec3;
use super::lua_world::vec3;
use super::{LogicState, Registry, Request};
use crate::binding;

type R = mw_lua::Result<Vec<Value>>;
type V3 = [f32; 3];

/// `.rdata` 0x01075470: −1/(2π), the cosine's range reduction (turns, negated).
const NEG_INV_TWO_PI: f32 = f32::from_bits(0xbe22_f983);
/// `.rdata` 0x01075480: 1.5 × 2²³, added and subtracted to round to an integer.
const ROUND_MAGIC: f32 = f32::from_bits(0x4b40_0000);
/// `.rdata` 0x010754c0..0x01075550: the odd Taylor coefficients of sin(2πy), y¹ to y¹⁹.
const SIN_2PI: [f32; 10] = [
    f32::from_bits(0x40c9_0fdb),
    f32::from_bits(0xc225_5de7),
    f32::from_bits(0x42a3_35e3),
    f32::from_bits(0xc299_6966),
    f32::from_bits(0x4228_3c1a),
    f32::from_bits(0xc171_83a8),
    f32::from_bits(0x4074_7a1a),
    f32::from_bits(0xbf37_d6dd),
    f32::from_bits(0x3dd5_7619),
    f32::from_bits(0xbc45_2021),
];
/// `.rdata` 0x010754b0: π/2 (sin x is computed as the cosine of x − π/2).
const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);
/// `.rdata` 0x00fbcbf0: 5° in radians; `CalculateInterpolatedLocation` interpolates linearly above its cosine.
const FIVE_DEGREES: f32 = f32::from_bits(0x3db2_b8c3);
/// `.rdata` 0x00fb869c: degrees to radians.
const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
/// `.rdata` 0x00fb1380 / 0x00fbcb80: the up axis.
const UP: V3 = [0.0, 1.0, 0.0];
/// `.rdata` 0x00fbcb70: the direction `TeleportVehicle` / `SpawnVehicle` use without one.
const DEFAULT_DIRECTION: V3 = [1.0, 0.0, 0.0];
/// `.rdata` 0x00fbcc10 / 0x00fbcc20: `TeleportToNamedRoad` places the car 5 m back along the road and 3 m to
/// the side (`direction × up`).
const NAMED_ROAD_BACK: f32 = 5.0;
const NAMED_ROAD_SIDE: f32 = 3.0;
/// `.rdata` 0x00fa929c: the radius `HasFinishedStreamingWorld` asks to be resident.
const STREAMED_RADIUS: f32 = 10.0;

/// A resource the runtime shares with the logic (compared by identity).
#[derive(Debug, Clone)]
pub struct Shared<T>(pub Option<Arc<T>>);

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Some(Arc::new(value)))
    }

    pub fn get(&self) -> Option<&T> {
        self.0.as_deref()
    }
}

/// State read by the handlers of this file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RoadFields {
    /// The road network (`RoadData`, the handle at G+0x153a8 / offset G+0x153ac): `GetRoadStartDirection`,
    /// `TeleportToNamedRoad`.
    pub roads: Shared<RoadData>,
    /// The text database (UIM+0x2ce40, vtable +4: the string of a Language id): the road names
    /// `TeleportToNamedRoad` compares. An id the runtime has no text for is not found.
    pub texts: HashMap<u32, String>,
    /// The streaming zones (`ZoneList`) and the track units loaded (the stream list +0xe60ac with its loaded
    /// bytes +0xc3e74 of the world object `*0x010edadc`): Resident(p, r) of `HasFinishedStreamingWorld`.
    pub zones: Shared<ZoneList>,
    pub loaded_units: HashSet<u32>,
    /// The race manager (G+0x2f47c) +0xdf0[slot]: the slot's route length (`TeleportToDistanceFromFinish`).
    pub route_length: HashMap<i32, f32>,
    /// `FUN_00662670` for the slot's car (the race route's direction at its position), present while the race
    /// manager's vtable +8 holds (`CalculateForwardDirection`; +0xc is [`FeedbackFields::race_active`]
    /// (super::lua_feedback::FeedbackFields::race_active)).
    pub route_forward: HashMap<i32, V3>,
    /// The loading-screen manager (LSM = UIM+0x2d1b0) slot values +0x58 + reason × 4 for reasons 0 and 2 (UIM+0x2d208
    /// / +0x2d210); the mask is [`GameLogic::loading_screen_requests`](super::game_logic::GameLogic).
    pub loading_slot_0: u32,
    pub loading_slot_2: u32,
    /// LSM +0x48 (rotate the layouts), the ring at LSM+0 (count +0x10) and its index +0x14 (`FUN_00799a60`).
    pub loading_rotation: bool,
    pub loading_layouts: Vec<u32>,
    pub loading_layout_index: u32,
    /// The camera system (G+0x2f470) +0x1c, the serial of the last ICE camera started (`StopUsingIceCamera`
    /// stops only this one), and +0x18.
    pub ice_camera_serial: u32,
    pub ice_camera_18: i32,
}

fn boolean(b: bool) -> R {
    Ok(vec![Value::Bool(b)])
}

fn arg_f32(args: &[Value], i: usize) -> f32 {
    tonumber(args, i) as f32
}

fn arg_v3(args: &[Value], first: usize) -> V3 {
    [arg_f32(args, first), arg_f32(args, first + 1), arg_f32(args, first + 2)]
}

/// `maxps`: the second operand unless the first is greater (NaN gives the second).
fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// The handlers' dot product: `(z + y) + x` (`shufps 2`, `shufps 1`, two `addps`).
fn dot(a: V3, b: V3) -> f32 {
    (a[2] * b[2] + a[1] * b[1]) + a[0] * b[0]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn scale(v: V3, s: f32) -> V3 {
    v.map(|c| c * s)
}

/// `rsqrtps` and two Newton steps `r = (1 − r·r·x) · (0.5·r) + r` (`.rdata` 0x00fbcb60 = 1, 0x00fbcbd0 = 0.5).
/// Model decision: the estimate is the exact `1/sqrt(x)` (see the module doc).
pub fn rsqrt_nr(x: f32) -> f32 {
    let r0 = 1.0 / x.sqrt();
    let r1 = (1.0 - r0 * r0 * x) * (0.5 * r0) + r0;
    (1.0 - r1 * r1 * x) * (0.5 * r1) + r1
}

/// The handlers' normalisation: `v · rsqrt_nr(v·v)` (NaN for a zero vector, as in the original).
pub fn normalize(v: V3) -> V3 {
    scale(v, rsqrt_nr(dot(v, v)))
}

/// The cosine the handlers inline (`CalculateInterpolatedLocation`, `IsCompetitorWithinVisibilityCone`): `a =
/// |x| · (−1/2π)`, `r = round(a) − a − 0.5` (rounded with ±1.5·2²³), `y = |r| − 0.25`, then sin(2πy) as the odd
/// series y..y¹⁹ with the `.rdata` coefficients, summed y³ + y, then y⁵ .. y¹⁹ in order.
pub fn cos_sse(x: f32) -> f32 {
    let a = maxps(-x, x) * NEG_INV_TWO_PI;
    let r = (((a - ROUND_MAGIC) + ROUND_MAGIC) - a) - 0.5;
    let y = maxps(-r, r) - 0.25;
    let y2 = y * y;
    let mut p = y;
    let mut terms = [0.0f32; 10];
    for (i, c) in SIN_2PI.iter().enumerate() {
        if i > 0 {
            p *= y2;
        }
        terms[i] = p * c;
    }
    terms[2..].iter().fold(terms[1] + terms[0], |s, t| s + t)
}

/// sin x = cos(x − π/2) (`subps` 0x010754b0, then [`cos_sse`]).
pub fn sin_sse(x: f32) -> f32 {
    cos_sse(x - HALF_PI)
}

/// `CalculateInterpolatedLocation`'s maths (0x00681160): the position `p0 + (p1 − p0)·t`; the direction is the
/// normalised linear blend when `d0·d1` > cos 5°, else the spherical one `(sin(tθ)·d1 + sin((1−t)θ)·d0) / sin θ`
/// with θ = `acosf(d0·d1)` (the CRT's `acosf` 0x00eddb3b on each lane; model decision: Rust's `acos`).
pub fn interpolate(p0: V3, d0: V3, p1: V3, d1: V3, t: f32) -> (V3, V3) {
    let position = std::array::from_fn(|i| (p1[i] - p0[i]) * t + p0[i]);
    let c = dot(d1, d0);
    let direction = if c > cos_sse(FIVE_DEGREES) {
        normalize(std::array::from_fn(|i| t * (d1[i] - d0[i]) + d0[i]))
    } else {
        let theta = c.acos();
        let a = sin_sse(t * theta) / sin_sse(theta);
        let b = sin_sse((1.0 - t) * theta) / sin_sse(theta);
        std::array::from_fn(|i| a * d1[i] + b * d0[i])
    };
    (position, direction)
}

/// `FUN_005e39d0`: orthonormalise three rows. Each is normalised; with all three non-zero the pair whose
/// absolute dot product is largest decides which two are rebuilt (the `maxps` of `−1 ×` (`.rdata` 0x00fb1360)
/// and the dots); `p4 = n(p2 × p3)`, then `p3 = n(p4 × p2)`.
pub fn orthonormalize(rows: [V3; 3]) -> [V3; 3] {
    let mut m = rows.map(normalize);
    let len2 = rows.map(|r| dot(r, r));
    // (p2, p3, p4) as row indices.
    let (p2, p3, p4) = if len2[0] <= 0.0 {
        (1, 2, 0)
    } else if len2[1] <= 0.0 {
        (2, 0, 1)
    } else if len2[2] > 0.0 {
        let d_ab = dot(m[1], m[0]);
        let d_ac = dot(m[2], m[0]);
        let d_bc = dot(m[2], m[1]);
        let (bc, ac, ab) = (maxps(-d_bc, d_bc), maxps(-d_ac, d_ac), maxps(-d_ab, d_ab));
        if ac <= bc {
            if ac < ab { (2, 0, 1) } else { (0, 1, 2) }
        } else if bc < ab {
            (1, 2, 0)
        } else {
            (0, 1, 2)
        }
    } else {
        (0, 1, 2)
    };
    m[p4] = normalize(cross(m[p2], m[p3]));
    m[p3] = normalize(cross(m[p4], m[p2]));
    m
}

/// The frame `FUN_005de2d0` builds for a direction `f`: rows `up × f`, `f × (up × f)`, `f`, orthonormalised.
fn look_frame(f: V3) -> [V3; 3] {
    let a = cross(UP, f);
    orthonormalize([a, cross(f, a), f])
}

/// `CalculateRotatedLocationXZ`'s maths (0x00681a60 → `FUN_005de2d0`): the position turned about `centre` in
/// the ground plane to lie towards `target` at its horizontal distance from the centre (keeping its height), and
/// the direction turned by the change of frame from (centre − position) to (centre − new position).
pub fn rotate_xz(position: V3, direction: V3, centre: V3, target: V3) -> (V3, V3) {
    let dx = centre[0] - position[0];
    let dz = centre[2] - position[2];
    let dist = ((dz * dz + 0.0) + dx * dx).sqrt();
    let v = [target[0] - centre[0], 0.0, target[2] - centre[2]];
    let r = rsqrt_nr((v[2] * v[2] + 0.0) + v[0] * v[0]);
    let out = [r * v[0] * dist + centre[0], position[1], r * v[2] * dist + centre[2]];
    let m1 = look_frame(normalize(std::array::from_fn(|i| centre[i] - position[i])));
    let m2 = look_frame(normalize(std::array::from_fn(|i| centre[i] - out[i])));
    let local = m1.map(|row| (direction[1] * row[1] + direction[0] * row[0]) + direction[2] * row[2]);
    let dir = std::array::from_fn(|i| (local[1] * m2[1][i] + local[0] * m2[0][i]) + local[2] * m2[2][i]);
    (out, dir)
}

/// `CalculateInterpolatedLocation(p0, d0, p1, d1, t)` (0x00681160): four `{x, y, z}` tables (`FUN_00697120`) and
/// `t`; pushes the position and the direction (`FUN_00699ea0`).
fn calculate_interpolated_location(_: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let [p0, d0, p1, d1] = [1, 2, 3, 4].map(|i| table_vec3(&args, i));
    let (p, d) = interpolate(p0, d0, p1, d1, arg_f32(&args, 5));
    Ok([vec3(lua, p)?, vec3(lua, d)?].concat())
}

/// `CalculateRotatedLocationXZ(position, direction, centre, target)` (0x00681a60): pushes the turned position and
/// direction.
fn calculate_rotated_location_xz(_: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let [p, d, c, t] = [1, 2, 3, 4].map(|i| table_vec3(&args, i));
    let (p, d) = rotate_xz(p, d, c, t);
    Ok([vec3(lua, p)?, vec3(lua, d)?].concat())
}

fn player(state: &LogicState, slot: i32) -> Option<&super::players::Player> {
    state.players.slots.get(usize::try_from(slot).ok()?)?.as_ref()
}

/// `CycleCameraTarget()` (0x00681b10): from the camera target (`FUN_006bf100`: PM+0xa6bc, the local slot when
/// −1 or empty), when that slot is occupied, the next occupied of the 10 slots after it (mod 24), else the
/// local slot; `FUN_006cab10` makes it the target (message 0x2c when it changes to an occupied slot); when the
/// target moved, `FUN_007b1150`(UIM+0x30840, mask 0xfffffffd) gives the feedback sequence slot 0 the null
/// handle (UIM+0x30a5c). Model decision: without a local player (the original reads slot −1) nothing happens.
fn cycle_camera_target(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let pm = &state.players;
    let local = pm.local;
    let current = match pm.camera_target {
        Some(t) if pm.slots.get(t).is_some_and(Option::is_some) => Some(t),
        _ => local,
    };
    let Some(start) = current else { return Ok(vec![]) };
    if pm.slots.get(start).is_none_or(Option::is_none) {
        return Ok(vec![]);
    }
    let mut candidate = start;
    let mut tries = 0;
    let slot = loop {
        candidate = (candidate + 1) % super::players::SLOTS;
        tries += 1;
        if tries > 10 {
            break local;
        }
        if pm.slots[candidate].is_some() {
            break Some(candidate);
        }
    };
    // FUN_006cab10: the message when the target changes to an occupied slot (its record +0x98 checked by the
    // runtime), then PM+0xa6bc = the slot.
    let message = slot.filter(|&s| pm.camera_target != Some(s) && pm.slots[s].is_some());
    if let Some(s) = message {
        state.requests.push(Request::CameraTargetChanged { slot: s });
    }
    state.players.camera_target = slot;
    if slot != Some(start) {
        state.sim.feedback.sequence_handle = 0;
    }
    Ok(vec![])
}

/// `SwitchToIceCamera(take, [flag])` (0x0067b800 → `FUN_00843a60`(camera system G+0x2f470, 0, take, −1, flag,
/// 0)): message 4 {0, take, 1, flag, 0} on the camera queue, +0x18 = −1, +0x1c += 1; pushes the new +0x1c.
/// `flag` is read only with exactly two arguments.
fn switch_to_ice_camera(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let take = int(&args, 1);
    let flag = args.len() == 2 && toboolean(&args, 2);
    let f = &mut state.sim.road;
    f.ice_camera_18 = -1;
    f.ice_camera_serial = f.ice_camera_serial.wrapping_add(1);
    let serial = f.ice_camera_serial;
    state.requests.push(Request::IceCameraStart { take, flag });
    Ok(vec![push_uint(serial)])
}

/// The key / value pairs of a table in `lua_next` order (nothing for a non-table).
fn pairs(v: &Value) -> Vec<(Value, Value)> {
    let Value::Table(t) = v else { return vec![] };
    let mut out = Vec::new();
    let mut key = Value::Nil;
    while let Ok(Some((k, v))) = t.next(&key) {
        out.push((k.clone(), v));
        key = k;
    }
    out
}

fn key_name(k: &Value) -> String {
    Lua::tolstring(k).map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned()).unwrap_or_default()
}

/// `ShowFeedbackMessage(text, id, [queue], [parameters])` (0x00682210). The queue name (≤ 63 bytes) is
/// argument 3 unless it is nil (an absent argument is `LUA_TNONE`, read as `"<invalid>"`); argument 4 (only
/// with more than three arguments, a table) gives the message parts, in `lua_next` order: `GamerTagParameters`
/// (name → player slot, `FUN_007c1aa0`(name, slot, 1): the slot's name; an empty slot cancels the message),
/// `NumberParameters` (name → float, `FUN_007c1bd0`) and `StringIDParameters` (name → id, `FUN_007c2000`). The
/// queue (`FUN_007b6110`: by name in UIM+0x17e0, else the Hud queue UIM+0x7230) takes the message unless it is
/// full (`FUN_007b96b0` → `FUN_007b8080` → `FUN_007b6280`: appended with the next serial; `feedback_queue.rs`);
/// pushes the serial, else nil. The original's part arrays hold 4 entries and the pointer list 4 in all, without a
/// check (model decision: every part is kept).
fn show_feedback_message(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 2);
    let text = string_or_invalid(&args, 1);
    let queue = match args.get(2) {
        Some(Value::Nil) => String::new(),
        _ => fit(&string_or_invalid(&args, 3), 0x40).to_string(),
    };
    let mut all_present = true;
    let mut parts = Vec::new();
    if args.len() > 3
        && let Value::Table(t) = &args[3]
    {
        for (k, v) in pairs(&t.get_str("GamerTagParameters")) {
            let slot = ftol32(Lua::tonumber(&v).unwrap_or(0.0));
            match player(state, slot) {
                Some(p) => parts.push(Part { name: key_name(&k), value: PartValue::Text(p.name.clone()) }),
                None => all_present = false,
            }
        }
        for (k, v) in pairs(&t.get_str("NumberParameters")) {
            parts.push(Part { name: key_name(&k), value: PartValue::Number(Lua::tonumber(&v).unwrap_or(0.0) as f32) });
        }
        for (k, v) in pairs(&t.get_str("StringIDParameters")) {
            let value = PartValue::StringId(ftol32(Lua::tonumber(&v).unwrap_or(0.0)));
            parts.push(Part { name: key_name(&k), value });
        }
    }
    let f = &mut state.feedback_queues;
    let q = f.queue_or_hud(&queue);
    if f.queues[q].full() || !all_present {
        return Ok(vec![Value::Nil]);
    }
    let serial = f.post(q, None, id as u32, &text, parts);
    Ok(vec![push_uint(serial)])
}

/// `ShowLoadingScreen(show, [layout])` (0x0067e430). Show: reason 2 of the loading-screen manager
/// (`FUN_0078f860` with a non-zero layout, else `FUN_00799a60`: the next layout of the ring when LSM +0x48 is set,
/// else 0; either only when the slot is empty), then reason 0 cleared (bit 0, slot 0). Hide: reason 2 cleared
/// (bit 2, slot 2) when set. Model decision: an empty ring (the original divides by its count) gives 0.
fn show_loading_screen(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let show = toboolean(&args, 1);
    let layout = if args.len() > 1 { int(&args, 2) } else { 0 };
    let f = &mut state.sim.road;
    let mask = &mut state.game.loading_screen_requests;
    if show {
        if f.loading_slot_2 == 0 {
            f.loading_slot_2 = if layout != 0 {
                layout as u32
            } else if f.loading_rotation && !f.loading_layouts.is_empty() {
                let n = f.loading_layouts.len() as u32;
                let i = f.loading_layout_index % n;
                f.loading_layout_index = (i + 1) % n;
                f.loading_layouts[i as usize]
            } else {
                0
            };
        }
        *mask |= 1 << 2;
        if *mask & 1 != 0 {
            *mask &= !1;
            f.loading_slot_0 = 0;
        }
    } else if *mask & 4 != 0 {
        *mask &= !4;
        f.loading_slot_2 = 0;
    }
    Ok(vec![])
}

/// `GetValue(binding)` (0x0067afd0): the text (≤ 255 bytes) parsed (`FUN_00a50650`, inner form), evaluated
/// (`FUN_00a48590`) and pushed by the Lua visitor (vtable 0x00fbd4b8, `FUN_00a4f280` → dispatch `FUN_00a4ef40`):
/// bool → boolean; i32 / u32 / i64 / float / double → number; strings → string; null, out of range and
/// unsupported → nil (slots 25–27). No operators apply. Model decisions: u64 (slot 16 pushes nothing while the
/// handler returns 1), objects and lists (no visitor slot read for them) and text that does not parse give nil.
fn get_value(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let text = string_or_invalid(&args, 1);
    let Ok(b) = binding::Binding::parse(fit(&text, 0x100)) else { return Ok(vec![Value::Nil]) };
    let v = match binding::Host::lookup(&*state, &b.segments) {
        binding::Lookup::Found(v) => match v {
            binding::Value::Bool(b) => Value::Bool(b),
            binding::Value::Int(i) => Value::Number(f64::from(i)),
            binding::Value::UInt(u) => Value::Number(f64::from(u)),
            binding::Value::Int64(i) => Value::Number(i as f64),
            binding::Value::Float(f) => Value::Number(f64::from(f)),
            binding::Value::Double(d) => Value::Number(d),
            binding::Value::Str(s) => push_str(&s),
            binding::Value::UInt64(_) | binding::Value::Object(_) | binding::Value::List(_) => Value::Nil,
        },
        _ => Value::Nil,
    };
    Ok(vec![v])
}

/// The record of a slot (`PM+0xa5d0`[slot]) and its vehicle (+0xa0); `None` for an empty slot (the original
/// dereferences the null record).
fn slot_vehicle(state: &LogicState, slot: i32) -> Option<i32> {
    player(state, slot).map(|p| p.vehicle)
}

/// `TeleportToDistanceFromFinish(slot, distance)` (0x0067aef0): while a race runs (race manager G+0x2f47c,
/// vtable +0xc) and the slot's route length (+0xdf0[slot]) exceeds `distance`, `FUN_00665ae0`(slot, length −
/// distance) gives the pose on the route and, when the slot's vehicle exists, it is placed there
/// (`FUN_00647e00`, reason 2). Model decision: an empty slot does nothing.
fn teleport_to_distance_from_finish(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = int(&args, 1);
    let distance = arg_f32(&args, 2);
    let Some(vehicle) = slot_vehicle(state, slot) else { return Ok(vec![]) };
    if !state.sim.feedback.race_active {
        return Ok(vec![]);
    }
    let Some(&length) = state.sim.road.route_length.get(&slot) else { return Ok(vec![]) };
    if length > distance && vehicle != -1 {
        state.requests.push(Request::TeleportAlongRoute { slot, vehicle, distance: length - distance });
    }
    Ok(vec![])
}

/// `GetRoadStartDirection(road)` (0x00688060): the normalised tangent of the road's first point (road +0x28 →
/// section 0 +0x14 → point 0 +0x10), pushed as three numbers. A road index past the count reads road 0. Model
/// decisions: a negative index (the original reads before the array) is road 0 too; without a road network the
/// three numbers are 0.
fn get_road_start_direction(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let i = int(&args, 1);
    let d = state
        .sim
        .road
        .roads
        .get()
        .and_then(|r| r.roads.get(usize::try_from(i).ok().filter(|&i| i < r.roads.len()).unwrap_or(0)))
        .and_then(|road| road.sections.first()?.points.first())
        .map_or([0.0; 3], |p| normalize(p.tangent));
    Ok(d.map(|c| Value::Number(f64::from(c))).to_vec())
}

/// `FUN_00a4ae10`: a wide string narrowed to ≤ 255 bytes, a unit ≥ 0x80 becoming `?`.
fn narrow(s: &str) -> String {
    s.encode_utf16().take(255).map(|u| if u < 0x80 { u as u8 as char } else { '?' }).collect()
}

/// `TeleportToNamedRoad(name, where)` (0x00680ca0): the first road whose name (its Language id +0x1c in the text
/// database, narrowed) equals `name`; `where` "End" = the last point of the last section facing back along the
/// road, "Middle" = the middle point of the middle section (counts halved toward zero), else the first point.
/// The local car is placed 5 m back along the normalised tangent and 3 m to the side (`direction × up`), facing
/// along it (`FUN_00647e00`, reason 2); returns nothing. No such road: pushes false. Model decisions: a road
/// without sections or points is not found; without a local player nothing is placed.
fn teleport_to_named_road(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let place = string_or_invalid(&args, 2);
    let f = &state.sim.road;
    let found = f
        .roads
        .get()
        .and_then(|r| r.roads.iter().find(|road| f.texts.get(&road.name).is_some_and(|t| narrow(t) == name)));
    let point = found.and_then(|road| match place.as_str() {
        "End" => {
            let p = road.sections.last()?.points.last()?;
            Some((p.position, p.tangent.map(|c| 0.0 - c)))
        }
        "Middle" => {
            let s = road.sections.get(road.sections.len() / 2)?;
            let p = s.points.get(s.points.len() / 2)?;
            Some((p.position, p.tangent))
        }
        _ => {
            let p = road.sections.first()?.points.first()?;
            Some((p.position, p.tangent))
        }
    });
    let Some((at, tangent)) = point else { return boolean(false) };
    let direction = normalize(tangent);
    let side = [
        UP[2] * direction[1] - UP[1] * direction[2],
        UP[0] * direction[2] - UP[2] * direction[0],
        UP[1] * direction[0] - UP[0] * direction[1],
    ];
    let position = std::array::from_fn(|i| side[i] * NAMED_ROAD_SIDE + (at[i] - NAMED_ROAD_BACK * direction[i]));
    if let Some(p) = state.players.local_player()
        && p.vehicle != -1
    {
        let vehicle = p.vehicle;
        state.requests.push(Request::PlaceVehicle {
            vehicle,
            position,
            direction,
            mph: 0.0,
            keep: false,
            reason: 2,
            flags: 0,
        });
    }
    Ok(vec![])
}

/// `IsCompetitorWithinVisibilityCone(a, b, degrees)` (0x0067b560): both slots' cars exist (+0xa0 ≠ −1) and are
/// loaded (`FUN_006ba2f0`), and a's forward direction (`FUN_006ba380`: the transform +0x20) dotted with b's
/// position − a's (`FUN_006b9ea0`, not normalised) is ≥ the cosine of the angle (`× 0.0174533`, [`cos_sse`]).
/// Model decision: an empty slot (the original dereferences it) gives false.
fn is_competitor_within_visibility_cone(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let angle = (tonumber(&args, 3) * f64::from(DEG_TO_RAD)) as f32;
    let car = |slot: i32| {
        player(state, slot).filter(|p| p.vehicle != -1 && p.loaded).map(|p| {
            state.sim.world.vehicles.get(&p.vehicle).map_or(([0.0; 3], [0.0; 3]), |v| (v.position, v.direction))
        })
    };
    let (Some((pa, fa)), Some((pb, _))) = (car(int(&args, 1)), car(int(&args, 2))) else { return boolean(false) };
    let d = std::array::from_fn(|i| pb[i] - pa[i]);
    boolean(dot(fa, d) >= cos_sse(angle))
}

/// A direction from three arguments, normalised.
fn arg_direction(args: &[Value], first: usize) -> V3 {
    normalize(arg_v3(args, first))
}

/// `SpawnVehicle(slot, a, x, y, z, _, dx, dy, dz, [b], [_])` (0x0067bfa0): `FUN_006cd550` on the slot's record
/// (PM+0x10 + slot × 0x7f0) with `a`, `b` (0 below ten arguments), the position and the direction (normalised
/// arguments 7–9 with nine or more, else (1, 0, 0)); argument 6 is not read, argument 11 read and dropped. The
/// record's +0x28c / +0x290 (the car-still words) are cleared there.
fn spawn_vehicle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let slot = int(&args, 1);
    let direction = if args.len() >= 9 { arg_direction(&args, 7) } else { DEFAULT_DIRECTION };
    let b = if args.len() >= 10 { int(&args, 10) } else { 0 };
    if let Some(p) = usize::try_from(slot).ok().and_then(|i| state.players.slots.get_mut(i)).and_then(Option::as_mut) {
        p.car_still = 0;
        p.car_still_290 = false;
    }
    state.requests.push(Request::SpawnVehicle { slot, a: int(&args, 2), b, position: arg_v3(&args, 3), direction });
    Ok(vec![])
}

/// `TeleportVehicle(vehicle, x, y, z, keep, [dx, dy, dz, [mph]])` (0x0067c1a0): `FUN_00647e00`(vehicle,
/// position, direction, mph, keep, 3, 0): the direction normalised from arguments 6–8 with eight or nine
/// arguments, else (1, 0, 0); the speed only with nine.
fn teleport_vehicle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let direction = if matches!(args.len(), 8 | 9) { arg_direction(&args, 6) } else { DEFAULT_DIRECTION };
    let mph = if args.len() == 9 { arg_f32(&args, 9) } else { 0.0 };
    state.requests.push(Request::PlaceVehicle {
        vehicle: int(&args, 1),
        position: arg_v3(&args, 2),
        direction,
        mph,
        keep: toboolean(&args, 5),
        reason: 3,
        flags: 0,
    });
    Ok(vec![])
}

/// `ResetAlongRoad(slot, x, y, z, dx, dy, dz, mph, search, keep)` (0x0067c480): `FUN_0064a0e0` for the slot's
/// vehicle: the pose put onto the road network (`FUN_00648060` with flags 6 and `search`), then placed
/// (`FUN_00647e00`, reason 8, flags 6). Model decision: an empty slot does nothing.
fn reset_along_road(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let Some(vehicle) = slot_vehicle(state, int(&args, 1)) else { return Ok(vec![]) };
    state.requests.push(Request::ResetAlongRoad {
        vehicle,
        position: arg_v3(&args, 2),
        direction: arg_direction(&args, 5),
        mph: arg_f32(&args, 8),
        search: arg_f32(&args, 9),
        keep: toboolean(&args, 10),
    });
    Ok(vec![])
}

/// `ResetOnRoad(slot, x, y, z, dx, dy, dz, mph, keep, [flag])` (0x0067c650): `FUN_00647e00`(the slot's vehicle,
/// position, normalised direction, mph, keep, 4, flags) with flags 2 when there are ten arguments and the tenth is
/// true, else 6. Model decision: an empty slot does nothing.
fn reset_on_road(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let Some(vehicle) = slot_vehicle(state, int(&args, 1)) else { return Ok(vec![]) };
    let flags = if args.len() == 10 && toboolean(&args, 10) { 2 } else { 6 };
    state.requests.push(Request::PlaceVehicle {
        vehicle,
        position: arg_v3(&args, 2),
        direction: arg_direction(&args, 5),
        mph: arg_f32(&args, 8),
        keep: toboolean(&args, 9),
        reason: 4,
        flags,
    });
    Ok(vec![])
}

/// Resident(p, r) (`FUN_0054b540` → `FUN_0054b340`): the zone holding p (`FUN_0054a360` → `FUN_00548ab0`) is
/// loaded and every neighbour that is not loaded is farther than r from p (`FUN_00561660`, circle against the
/// zone's polygon). Not ported: the world state 3 branch (+0xef868) that reads the zone from another table.
pub fn resident(f: &RoadFields, p: V3, r: f32) -> bool {
    let Some(zones) = f.zones.get() else { return false };
    let Some(zone) = zones.zone_at(p[0], p[2]) else { return false };
    f.loaded_units.contains(&zone.id)
        && zone.neighbours.iter().all(|&(i, _)| {
            zones.zones.get(i).is_none_or(|n| f.loaded_units.contains(&n.id) || n.distance(p[0], p[2]) > r)
        })
}

/// `HasFinishedStreamingWorld([x, y, z])` (0x0067c840): with a local player whose car exists, the point (the
/// three arguments when there are exactly three, else the car's position) is streamed: the camera's world has
/// finished streaming (`FUN_005470e0`) and Resident(point, 10 m). Model decision: the car's position is the
/// vehicle record's +0x110 read as the world transform's position.
fn has_finished_streaming_world(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let Some(vehicle) = state.players.local_player().map(|p| p.vehicle).filter(|&v| v != -1) else {
        return boolean(false);
    };
    let point = if args.len() == 3 {
        arg_v3(&args, 1)
    } else {
        state.sim.world.vehicles.get(&vehicle).map_or([0.0; 3], |v| v.position)
    };
    boolean(state.sim.world.camera_world_streamed && resident(&state.sim.road, point, STREAMED_RADIUS))
}

/// `CalculateForwardDirection([slot])` (0x00681cb0): the record of the slot (one argument) or the local player;
/// `FUN_00667b10` on the race manager with its +0x98 (the slot): while a race runs (vtable +0xc) and +8 holds,
/// the route's direction at the car (`FUN_00662670`) is pushed as `{x, y, z}`, else nil. Model decision: no local
/// player gives nil.
fn calculate_forward_direction(state: &mut LogicState, lua: &mut Lua, args: Vec<Value>) -> R {
    let slot = if args.len() == 1 { Some(int(&args, 1)) } else { state.players.local.map(|i| i as i32) };
    let forward =
        slot.filter(|_| state.sim.feedback.race_active).and_then(|s| state.sim.road.route_forward.get(&s).copied());
    match forward {
        Some(d) => vec3(lua, d),
        None => Ok(vec![Value::Nil]),
    }
}

/// `HACKPlayFirstCustomPlaylist()` (0x00675870): `FUN_006f2730`(0) on GMM+0x14, as `PlayCustomPlaylist(0)`.
fn hack_play_first_custom_playlist(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::PlayCustomPlaylist { index: 0 });
    Ok(vec![])
}

/// `DEMOMODEWriteDemoConfig(name, value, text)` (0x007ea180) reads its arguments and drops them;
/// `DEMOMODEDeleteDemoConfig()` (0x007ea1b0) only reads the count: retail stubs.
fn demo_mode_stub(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `RegisterSpeechCommand(id, name, text)` (0x007ea780 → `FUN_0079d390` on UIM+0x32dd8).
fn register_speech_command(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::RegisterSpeechCommand {
        id: int(&args, 1),
        name: string_or_invalid(&args, 2),
        text: string_or_invalid(&args, 3),
    });
    Ok(vec![])
}

/// `IsOriginIgoAvailable()` (0x007eb320): `FUN_00ae3740`(2, ...) answers "true" (hash compare), as the hook of
/// the same name ([`HookOnlineFields::origin_igo_available`](super::hooks_online::HookOnlineFields)).
fn is_origin_igo_available(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    boolean(state.hook_online.origin_igo_available)
}

/// `ShowOriginIgo()` (0x007eb380 → `FUN_0086d5f0` on (G+0x2f3e0)+0x4ce0).
fn show_origin_igo(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ShowOriginIgo);
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("CalculateInterpolatedLocation", calculate_interpolated_location);
    r.set_lua("CalculateRotatedLocationXZ", calculate_rotated_location_xz);
    r.set_lua("CycleCameraTarget", cycle_camera_target);
    r.set_lua("SwitchToIceCamera", switch_to_ice_camera);
    r.set_lua("ShowFeedbackMessage", show_feedback_message);
    r.set_lua("ShowLoadingScreen", show_loading_screen);
    r.set_lua("GetValue", get_value);
    r.set_lua("TeleportToDistanceFromFinish", teleport_to_distance_from_finish);
    r.set_lua("GetRoadStartDirection", get_road_start_direction);
    r.set_lua("TeleportToNamedRoad", teleport_to_named_road);
    r.set_lua("IsCompetitorWithinVisibilityCone", is_competitor_within_visibility_cone);
    r.set_lua("SpawnVehicle", spawn_vehicle);
    r.set_lua("TeleportVehicle", teleport_vehicle);
    r.set_lua("ResetAlongRoad", reset_along_road);
    r.set_lua("ResetOnRoad", reset_on_road);
    r.set_lua("HasFinishedStreamingWorld", has_finished_streaming_world);
    r.set_lua("CalculateForwardDirection", calculate_forward_direction);
    r.set_lua("HACKPlayFirstCustomPlaylist", hack_play_first_custom_playlist);
    r.set_lua("DEMOMODEWriteDemoConfig", demo_mode_stub);
    r.set_lua("DEMOMODEDeleteDemoConfig", demo_mode_stub);
    r.set_lua("RegisterSpeechCommand", register_speech_command);
    r.set_lua("IsOriginIgoAvailable", is_origin_igo_available);
    r.set_lua("ShowOriginIgo", show_origin_igo);
}

#[cfg(test)]
mod tests {
    use mw_formats::Zone;
    use mw_formats::json::Json;
    use mw_formats::road_data::{Road, RoadPoint, RoadSection};

    use super::super::LogicHost;
    use super::super::lua_world::WorldVehicle;
    use super::super::players::Player;
    use super::*;

    fn setup() -> (Lua, LogicHost) {
        let mut lua = Lua::new();
        let host = LogicHost::default();
        host.install_lua(&mut lua);
        (lua, host)
    }

    fn call(lua: &mut Lua, name: &str, args: Vec<Value>) -> Vec<Value> {
        lua.call_global(name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    fn s(x: &str) -> Value {
        push_str(x)
    }

    fn t3(lua: &mut Lua, v: V3) -> Value {
        vec3(lua, v).unwrap().remove(0)
    }

    fn read3(v: &Value) -> V3 {
        let Value::Table(t) = v else { panic!("not a table: {v:?}") };
        ["x", "y", "z"].map(|k| t.get_str(k).as_number().unwrap() as f32)
    }

    fn close(a: V3, b: V3, eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
    }

    fn is_true(r: &[Value]) -> bool {
        matches!(r[0], Value::Bool(true))
    }

    fn point(position: V3, tangent: V3) -> RoadPoint {
        RoadPoint { position, radius: 5.0, tangent, oncoming: [[0; 2]; 2] }
    }

    #[test]
    fn cosine_and_normalisation() {
        for x in [0.0f32, 0.3, 1.0, 2.0, 3.1, -2.5, 10.0] {
            assert!((cos_sse(x) - x.cos()).abs() < 2e-6, "{x}");
            assert!((sin_sse(x) - x.sin()).abs() < 2e-6, "{x}");
        }
        assert!(close(normalize([3.0, 0.0, 4.0]), [0.6, 0.0, 0.8], 1e-7));
        assert!(normalize([0.0; 3])[0].is_nan());
        // Degenerate first row: rebuilt from the other two.
        let m = orthonormalize([[0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 2.0]]);
        assert!(close(m[0], [1.0, 0.0, 0.0], 1e-7) && close(m[2], [0.0, 0.0, 1.0], 1e-7));
    }

    #[test]
    fn interpolated_location() {
        let (mut lua, _host) = setup();
        let args = vec![
            t3(&mut lua, [0.0; 3]),
            t3(&mut lua, [1.0, 0.0, 0.0]),
            t3(&mut lua, [10.0, 20.0, 30.0]),
            t3(&mut lua, [0.0, 0.0, 1.0]),
            n(0.5),
        ];
        let r = call(&mut lua, "CalculateInterpolatedLocation", args);
        assert_eq!(read3(&r[0]), [5.0, 10.0, 15.0]);
        // 90 degrees apart: the spherical blend.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(read3(&r[1]), [h, 0.0, h], 1e-5), "{:?}", read3(&r[1]));
        // Within 5 degrees: the normalised linear blend.
        let (_, d) = interpolate([0.0; 3], [1.0, 0.0, 0.0], [0.0; 3], [1.0, 0.0, 0.02], 0.5);
        assert!(close(d, normalize([1.0, 0.0, 0.01]), 1e-7));
        // Missing tables read as zeros: the dot is 0, θ = 90°, the blend is 0.
        let (_, d) = interpolate([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], 0.5);
        assert_eq!(d, [0.0; 3]);
    }

    #[test]
    fn rotated_location_xz() {
        let (mut lua, _host) = setup();
        let args = vec![
            t3(&mut lua, [10.0, 2.0, 0.0]),
            t3(&mut lua, [0.0, 0.0, 1.0]),
            t3(&mut lua, [0.0; 3]),
            t3(&mut lua, [0.0, 7.0, 10.0]),
        ];
        let r = call(&mut lua, "CalculateRotatedLocationXZ", args);
        assert!(close(read3(&r[0]), [0.0, 2.0, 10.0], 1e-5), "{:?}", read3(&r[0]));
        // A quarter turn about up: +z becomes -x.
        assert!(close(read3(&r[1]), [-1.0, 0.0, 0.0], 1e-5), "{:?}", read3(&r[1]));
    }

    #[test]
    fn visibility_cone() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.players.slots[0] = Some(Player { vehicle: 1, loaded: true, ..Default::default() });
            st.players.slots[1] = Some(Player { vehicle: 2, loaded: true, ..Default::default() });
            st.players.slots[2] = Some(Player { vehicle: 3, loaded: false, ..Default::default() });
            let v = &mut st.sim.world.vehicles;
            v.insert(1, WorldVehicle { direction: [0.0, 0.0, 1.0], ..Default::default() });
            v.insert(2, WorldVehicle { position: [1.0, 0.0, 1.0], ..Default::default() });
        }
        // b is 45 degrees off a's heading: forward . (b - a) = 1 (not normalised) against the cosine.
        let cone = |lua: &mut Lua, a: f64, b: f64, deg: f64| {
            is_true(&call(lua, "IsCompetitorWithinVisibilityCone", vec![n(a), n(b), n(deg)]))
        };
        assert!(cone(&mut lua, 0.0, 1.0, 30.0));
        // Vehicle 2 has no direction: dot 0 >= cos 90°, < cos 30°.
        assert!(cone(&mut lua, 1.0, 0.0, 90.0));
        assert!(!cone(&mut lua, 1.0, 0.0, 30.0));
        assert!(!cone(&mut lua, 0.0, 2.0, 90.0));
        assert!(!cone(&mut lua, 0.0, 5.0, 90.0));
    }

    #[test]
    fn camera_target_and_ice_camera() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.players.local = Some(0);
            st.players.slots[0] = Some(Player::default());
            st.players.slots[5] = Some(Player::default());
            st.sim.feedback.sequence_handle = 9;
        }
        call(&mut lua, "CycleCameraTarget", vec![]);
        assert_eq!(host.state.borrow().players.camera_target, Some(5));
        assert_eq!(host.state.borrow().sim.feedback.sequence_handle, 0);
        // From 5 the next ten slots (6..15) are empty: back to the local slot.
        call(&mut lua, "CycleCameraTarget", vec![]);
        assert_eq!(host.state.borrow().players.camera_target, Some(0));
        let serial = call(&mut lua, "SwitchToIceCamera", vec![n(12.0), Value::Bool(true)]);
        assert_eq!(serial[0].as_number(), Some(1.0));
        let serial = call(&mut lua, "SwitchToIceCamera", vec![n(13.0)]);
        assert_eq!(serial[0].as_number(), Some(2.0));
        let st = host.state.borrow();
        assert_eq!(st.sim.road.ice_camera_18, -1);
        assert_eq!(
            st.requests,
            vec![
                Request::CameraTargetChanged { slot: 5 },
                Request::CameraTargetChanged { slot: 0 },
                Request::IceCameraStart { take: 12, flag: true },
                Request::IceCameraStart { take: 13, flag: false },
            ]
        );
    }

    #[test]
    fn feedback_message() {
        use super::super::feedback_queue::{HUD, MessageDef, SERIAL_WRAP, SONG};
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.players.slots[3] = Some(Player { name: "Rival".into(), ..Default::default() });
            let defs = [4, 5, 6, 7, 1].map(|id| MessageDef { id, time: 1.0, ..Default::default() });
            st.feedback_queues.set_group(9, Some(defs.to_vec()));
        }
        let params = lua.create_table();
        let tags = lua.create_table();
        tags.set_str("Rival", n(3.0));
        params.set_str("GamerTagParameters", Value::Table(tags));
        let nums = lua.create_table();
        nums.set_str("Score", n(2.5));
        params.set_str("NumberParameters", Value::Table(nums));
        let ids = lua.create_table();
        ids.set_str("Car", n(77.0));
        params.set_str("StringIDParameters", Value::Table(ids));
        let r = call(&mut lua, "ShowFeedbackMessage", vec![s("hello"), n(4.0), s("Song"), Value::Table(params)]);
        assert_eq!(r[0].as_number(), Some(1.0));
        // Two arguments: the queue reads "<invalid>", the Hud queue.
        let r = call(&mut lua, "ShowFeedbackMessage", vec![s("x"), n(5.0)]);
        assert_eq!(r[0].as_number(), Some(2.0));
        // A gamer tag of an empty slot cancels the message; a full queue too.
        let params = lua.create_table();
        let tags = lua.create_table();
        tags.set_str("Nobody", n(9.0));
        params.set_str("GamerTagParameters", Value::Table(tags));
        let r = call(&mut lua, "ShowFeedbackMessage", vec![s("x"), n(6.0), Value::Nil, Value::Table(params)]);
        assert!(r[0].is_nil());
        {
            let st = host.state.borrow();
            let song = &st.feedback_queues.queues[SONG].messages[0];
            assert_eq!((song.def.id, song.name.as_str(), song.serial), (4, "hello", 1));
            assert_eq!(
                song.parts,
                [
                    Part { name: "Rival".into(), value: PartValue::Text("Rival".into()) },
                    Part { name: "Score".into(), value: PartValue::Number(2.5) },
                    Part { name: "Car".into(), value: PartValue::StringId(77) },
                ]
            );
            assert_eq!(st.feedback_queues.queues[HUD].messages[0].serial, 2);
        }
        for _ in 0..6 {
            call(&mut lua, "ShowFeedbackMessage", vec![s("x"), n(7.0), Value::Nil]);
        }
        assert!(host.state.borrow().feedback_queues.queues[HUD].full());
        assert!(call(&mut lua, "ShowFeedbackMessage", vec![s("x"), n(7.0), Value::Nil])[0].is_nil());
        {
            let mut st = host.state.borrow_mut();
            st.feedback_queues.queues[HUD].clear();
            st.feedback_queues.serial = SERIAL_WRAP - 1;
        }
        assert_eq!(call(&mut lua, "ShowFeedbackMessage", vec![s("x"), n(1.0)])[0].as_number(), Some(0.0));
    }

    #[test]
    fn loading_screen() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.game.loading_screen_requests = 0x1;
            st.sim.road.loading_slot_0 = 4;
        }
        call(&mut lua, "ShowLoadingScreen", vec![Value::Bool(true), n(7.0)]);
        {
            let st = host.state.borrow();
            assert_eq!(st.game.loading_screen_requests, 0x4);
            assert_eq!((st.sim.road.loading_slot_0, st.sim.road.loading_slot_2), (0, 7));
        }
        call(&mut lua, "ShowLoadingScreen", vec![Value::Bool(false)]);
        assert_eq!(host.state.borrow().game.loading_screen_requests, 0);
        assert_eq!(host.state.borrow().sim.road.loading_slot_2, 0);
        {
            let mut st = host.state.borrow_mut();
            st.sim.road.loading_rotation = true;
            st.sim.road.loading_layouts = vec![11, 12];
            st.sim.road.loading_layout_index = 1;
        }
        call(&mut lua, "ShowLoadingScreen", vec![Value::Bool(true)]);
        let st = host.state.borrow();
        assert_eq!((st.sim.road.loading_slot_2, st.sim.road.loading_layout_index), (12, 0));
    }

    #[test]
    fn get_value() {
        let (mut lua, host) = setup();
        let data = Json::parse(r#"{"B": true, "I": 7, "D": 2.5, "S": "hi"}"#).unwrap();
        host.state.borrow_mut().view_models.publish("Test", data);
        let get = |lua: &mut Lua, b: &str| call(lua, "GetValue", vec![s(b)]).remove(0);
        assert!(matches!(get(&mut lua, "Test.B"), Value::Bool(true)));
        assert_eq!(get(&mut lua, "Test.I").as_number(), Some(7.0));
        assert_eq!(get(&mut lua, "Test.D").as_number(), Some(2.5));
        assert_eq!(get(&mut lua, "Test.S").as_str_lossy().as_deref(), Some("hi"));
        // No operators apply.
        assert_eq!(get(&mut lua, "Test.I(ADD:1)").as_number(), Some(7.0));
        assert!(get(&mut lua, "Test.Nope").is_nil());
        assert!(call(&mut lua, "GetValue", vec![])[0].is_nil());
    }

    fn roads() -> RoadData {
        let section = |points: Vec<RoadPoint>| RoadSection { bounds: [0.0; 4], id: 0, points };
        let road = |id, name, sections| Road { bounds: [0.0; 4], id, name, flags: 1, sections };
        RoadData {
            version: 9,
            roads: vec![
                road(0, 100, vec![section(vec![point([1.0, 0.0, 0.0], [0.0, 0.0, 3.0])])]),
                road(
                    1,
                    101,
                    vec![
                        section(vec![point([0.0; 3], [4.0, 0.0, 0.0]), point([10.0, 0.0, 0.0], [4.0, 0.0, 0.0])]),
                        section(vec![
                            point([20.0, 0.0, 0.0], [4.0, 0.0, 0.0]),
                            point([30.0, 0.0, 0.0], [0.0, 0.0, 2.0]),
                            point([40.0, 1.0, 0.0], [0.0, 0.0, 2.0]),
                        ]),
                    ],
                ),
            ],
        }
    }

    #[test]
    fn road_natives() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.sim.road.roads = Shared::new(roads());
            st.sim.road.texts.insert(101, "Caf\u{e9} Road".into());
            st.players.local = Some(0);
            st.players.slots[0] = Some(Player { vehicle: 4, ..Default::default() });
        }
        let d = call(&mut lua, "GetRoadStartDirection", vec![n(1.0)]);
        assert_eq!(d.iter().map(|v| v.as_number().unwrap()).collect::<Vec<_>>(), [1.0, 0.0, 0.0]);
        let d = call(&mut lua, "GetRoadStartDirection", vec![n(7.0)]);
        assert_eq!(d[2].as_number(), Some(1.0));
        assert!(!is_true(&call(&mut lua, "TeleportToNamedRoad", vec![s("Nowhere"), s("End")])));
        assert!(call(&mut lua, "TeleportToNamedRoad", vec![s("Caf? Road"), s("Start")]).is_empty());
        call(&mut lua, "TeleportToNamedRoad", vec![s("Caf? Road"), s("Middle")]);
        call(&mut lua, "TeleportToNamedRoad", vec![s("Caf? Road"), s("End")]);
        let place = |position, direction| Request::PlaceVehicle {
            vehicle: 4,
            position,
            direction,
            mph: 0.0,
            keep: false,
            reason: 2,
            flags: 0,
        };
        // 5 m back along the tangent, 3 m along direction x up.
        assert_eq!(
            host.state.borrow().requests,
            vec![
                place([-5.0, 0.0, 3.0], [1.0, 0.0, 0.0]),
                place([27.0, 0.0, -5.0], [0.0, 0.0, 1.0]),
                place([43.0, 1.0, 5.0], [0.0, 0.0, -1.0]),
            ]
        );
    }

    #[test]
    fn teleports_and_resets() {
        let (mut lua, host) = setup();
        host.state.borrow_mut().players.slots[2] =
            Some(Player { vehicle: 8, car_still: 1, car_still_290: true, ..Default::default() });
        let v = |x: f64, y: f64, z: f64| vec![n(x), n(y), n(z)];
        let pos = v(1.0, 2.0, 3.0);
        call(&mut lua, "TeleportVehicle", [vec![n(8.0)], pos.clone(), vec![Value::Bool(true)]].concat());
        let tail = [vec![Value::Nil], v(0.0, 0.0, 2.0), vec![n(30.0)]].concat();
        call(&mut lua, "TeleportVehicle", [vec![n(8.0)], pos.clone(), tail].concat());
        let reset = [vec![n(2.0)], pos.clone(), v(0.0, 3.0, 0.0), vec![n(40.0)]].concat();
        call(&mut lua, "ResetOnRoad", [reset.clone(), vec![Value::Bool(true), Value::Bool(true)]].concat());
        call(&mut lua, "ResetOnRoad", [reset.clone(), vec![Value::Bool(false)]].concat());
        call(&mut lua, "ResetOnRoad", [vec![n(6.0)], reset[1..].to_vec()].concat());
        call(&mut lua, "ResetAlongRoad", [reset.clone(), vec![n(15.0), Value::Bool(true)]].concat());
        let spawn = [vec![n(2.0), n(33.0)], pos.clone(), vec![n(99.0)], v(0.0, 0.0, -4.0), vec![n(5.0)]].concat();
        call(&mut lua, "SpawnVehicle", spawn);
        call(&mut lua, "SpawnVehicle", [vec![n(2.0), n(34.0)], pos].concat());
        let st = host.state.borrow();
        let p = st.players.slots[2].as_ref().unwrap();
        assert_eq!((p.car_still, p.car_still_290), (0, false));
        let (at, up) = ([1.0, 2.0, 3.0], [0.0, 1.0, 0.0]);
        let place = |direction, mph, keep, reason, flags| Request::PlaceVehicle {
            vehicle: 8,
            position: at,
            direction,
            mph,
            keep,
            reason,
            flags,
        };
        assert_eq!(
            st.requests,
            vec![
                place(DEFAULT_DIRECTION, 0.0, true, 3, 0),
                place([0.0, 0.0, 1.0], 30.0, false, 3, 0),
                place(up, 40.0, true, 4, 2),
                place(up, 40.0, false, 4, 6),
                Request::ResetAlongRoad {
                    vehicle: 8,
                    position: at,
                    direction: up,
                    mph: 40.0,
                    search: 15.0,
                    keep: true
                },
                Request::SpawnVehicle { slot: 2, a: 33, b: 5, position: at, direction: [0.0, 0.0, -1.0] },
                Request::SpawnVehicle { slot: 2, a: 34, b: 0, position: at, direction: DEFAULT_DIRECTION },
            ]
        );
    }

    fn square(id: u32, x0: f32, neighbours: Vec<(usize, u32)>) -> Zone {
        Zone {
            id,
            district: 0,
            arena: 0,
            flag: 0,
            points: vec![[x0, 0.0], [x0 + 100.0, 0.0], [x0 + 100.0, 100.0], [x0, 100.0]],
            bounds: [x0, 0.0, x0 + 100.0, 100.0],
            neighbours,
        }
    }

    #[test]
    fn streaming_and_route() {
        let (mut lua, host) = setup();
        {
            let mut st = host.state.borrow_mut();
            st.players.local = Some(1);
            st.players.slots[1] = Some(Player { vehicle: 6, ..Default::default() });
            st.sim.world.vehicles.insert(6, WorldVehicle { position: [50.0, 0.0, 50.0], ..Default::default() });
            st.sim.world.camera_world_streamed = true;
            let zones = ZoneList { zones: vec![square(1, 0.0, vec![(1, 3)]), square(2, 100.0, vec![(0, 3)])] };
            st.sim.road.zones = Shared::new(zones);
            st.sim.road.loaded_units = HashSet::from([1]);
        }
        // The car's zone is loaded, its unloaded neighbour 50 m away is outside 10 m.
        assert!(is_true(&call(&mut lua, "HasFinishedStreamingWorld", vec![])));
        // A point 5 m from the unloaded neighbour.
        assert!(!is_true(&call(&mut lua, "HasFinishedStreamingWorld", vec![n(95.0), n(0.0), n(50.0)])));
        host.state.borrow_mut().sim.world.camera_world_streamed = false;
        assert!(!is_true(&call(&mut lua, "HasFinishedStreamingWorld", vec![])));

        assert!(call(&mut lua, "CalculateForwardDirection", vec![])[0].is_nil());
        {
            let mut st = host.state.borrow_mut();
            st.sim.feedback.race_active = true;
            st.sim.road.route_forward.insert(1, [0.0, 0.0, 1.0]);
            st.sim.road.route_length.insert(1, 1000.0);
        }
        assert_eq!(read3(&call(&mut lua, "CalculateForwardDirection", vec![])[0]), [0.0, 0.0, 1.0]);
        assert!(call(&mut lua, "CalculateForwardDirection", vec![n(4.0)])[0].is_nil());
        call(&mut lua, "TeleportToDistanceFromFinish", vec![n(1.0), n(200.0)]);
        call(&mut lua, "TeleportToDistanceFromFinish", vec![n(1.0), n(1000.0)]);
        call(&mut lua, "TeleportToDistanceFromFinish", vec![n(3.0), n(10.0)]);
        assert_eq!(
            host.state.borrow().requests,
            vec![Request::TeleportAlongRoute { slot: 1, vehicle: 6, distance: 800.0 }]
        );
    }

    #[test]
    fn platform_and_debug() {
        let (mut lua, host) = setup();
        assert!(!is_true(&call(&mut lua, "IsOriginIgoAvailable", vec![])));
        host.state.borrow_mut().hook_online.origin_igo_available = true;
        assert!(is_true(&call(&mut lua, "IsOriginIgoAvailable", vec![])));
        call(&mut lua, "ShowOriginIgo", vec![]);
        call(&mut lua, "RegisterSpeechCommand", vec![n(3.0), s("go")]);
        call(&mut lua, "HACKPlayFirstCustomPlaylist", vec![]);
        assert!(call(&mut lua, "DEMOMODEWriteDemoConfig", vec![s("a"), n(1.0), s("b")]).is_empty());
        assert!(call(&mut lua, "DEMOMODEDeleteDemoConfig", vec![]).is_empty());
        assert_eq!(
            host.state.borrow().requests,
            vec![
                Request::ShowOriginIgo,
                Request::RegisterSpeechCommand { id: 3, name: "go".into(), text: "<invalid>".into() },
                Request::PlayCustomPlaylist { index: 0 },
            ]
        );
    }
}
