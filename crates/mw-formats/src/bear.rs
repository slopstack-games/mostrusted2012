//! `BearEffect` (0x301): Criterion's "Bear" particle effects (tyre smoke, sparks, debris, splashes).
//!
//! All 214 effects of the 1.4.0.0 install live in `GLOBALEFFECTS.BNDL`; GeneSys objects (`VfxGlobals`,
//! `VfxMisc`, `VfxVehicleCollisions`, `SurfaceVfx`, `VfxWheelEffects`) refer to them by handle. An
//! effect is a small program: each *layer* is one particle pool whose particles carry named *channels*
//! (`Position`, `Velocity`, `Age`, `Radius`, `Colour`, ...), and a list of typed *actions* (spawners,
//! initialisers, integrators, curves, renderers) that read and write those channels. The game feeds
//! per-instance *parameters* (`Transform`, `Velocity`, `Intensity`, `IsEmitting`, `WheelParams`, ...).
//! Layer and table layouts follow burnout.wiki ("Bear Effect/NFS Most Wanted"); the action payloads
//! were decoded from the data (research log 2026-10-04, "particles").
//!
//! All offsets are u16 and relative to the start of the resource (effect tables) or of the layer
//! (layer tables). Effect header:
//!
//! | off  | type    | field |
//! |------|---------|-------|
//! | 0x00 | u32     | version `0x269987a1` |
//! | 0x04 | u16     | layer count (0 for 26 empty placeholder effects) |
//! | 0x06 | u16     | implemented action groups mask (3: update + render) |
//! | 0x08 | u16     | parameter table offset (the table itself always starts at 0x20) |
//! | 0x0A–0x0E | u16 × 3 | pre-initialised parameter data (0 in retail) |
//! | 0x10 | u16     | parameter handle array: `{u16 offset | flags, u16 global parameter id}` per parameter |
//! | 0x12 | u16     | layer offset array (u16 per layer) |
//! | 0x14 | u16     | total size |
//!
//! Parameter table (at 0x20): `u8 count, u8 0, u16 block size, u32 ?, u8 kind[count]`, then `count`
//! NUL-terminated names. Kinds: 0 bool, 2 float, 3 matrix, 4 vector, 0x0C `VehicleParams`, 0x0D
//! `WheelParams`, 0x10 `CollisionMeshParams`, others for the wheel contact structs. The handle array
//! gives each parameter's byte offset in the instance's parameter block (low bit = set by the game).
//!
//! Layer (0x20-byte header, then the name):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | u16  | max particles |
//! | 0x02 | u16  | channel table offset |
//! | 0x04 | u16  | action table offset |
//! | 0x06 | u16  | action group table offset |
//! | 0x08 | u16  | inline data size |
//! | 0x0A | u16  | layer size |
//! | 0x0C | u8   | flags (1 buckets, 2 overwrite oldest particles, 4 parametric emission) |
//! | 0x0D | u8   | action count |
//! | 0x0E | u8   | action group count (4) |
//! | 0x0F | i8   | sort group |
//! | 0x10 | u8   | particles per bucket |
//! | 0x11 | u8   | pre-roll time |
//! | 0x20 | char[] | name (`<number>_Test`, `..._Control`, `..._SuperEmitter`) |
//!
//! Channel table: `u8 count, u8 external count, u16 bytes per bucket, u8 kind[count]`, then names.
//! Channel kinds: 0 float, 2 vector, 3 colour, 4 quaternion, 6 light, 7 plane, 8 seed, 9 packed
//! velocity. Action table: `u16 offset[count]` (layer-relative), then `u8 kind[count]`; an action's
//! size runs to the next action (the last one to the group table). Action group table: four u16
//! group offsets (0 = absent; group 0 = update, group 1 = render), each group `u8 sequences, u8
//! primaries, u8 secondaries, pad`, then `{u8 primaries, u8 per bucket}` per sequence, `{u8 action,
//! u8 secondaries}` per primary and `u8 action` per secondary. Spawners are primaries whose
//! secondaries initialise each new particle.
//!
//! Action payloads are packed structs of 4-byte words. Channel and parameter references are
//! [`Operand`]s; material imports sit in the renderers (see [`Op`]).

use mw_bundle::{Reader, Resource, ResourceId};

use crate::{Error, Result};

pub const VERSION: u32 = 0x2699_87a1;

/// One parameter the game sets on an effect instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    /// 0 bool, 2 float, 3 matrix, 4 vector, 0x0C VehicleParams, 0x0D WheelParams, 0x10 CollisionMeshParams.
    pub kind: u8,
    /// Byte offset in the instance's parameter block.
    pub offset: u16,
    /// Global parameter id (`0x2980 + 2i`, `0x0c00 + 2i`, ...).
    pub id: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    /// 0 float, 2 vector, 3 colour, 4 quaternion, 6 light, 7 plane, 8 seed, 9 packed velocity.
    pub kind: u8,
}

/// A reference to data an action reads or writes (one 4-byte word: `u16 lo, u16 hi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// Particle channel `index` (hi = 4 × index). `scratch` (lo bit 0) selects the per-frame
    /// temporary copy (`Radius*`, `Colour*`, unpacked `Velocity*`) instead of stored particle data.
    Channel { index: u8, scratch: bool },
    /// Effect parameter `index` (hi = 0x40 + 2 × index; lo = its offset in the parameter block).
    Param { index: u8 },
    /// Engine-side data (hi = 0xEB4x): space 0x40 is the layer's own state (`.1` written by the
    /// emission controllers, `.0` read by everything else), 0x42 the view (`.a0`, renderers).
    Engine { space: u8, value: u16 },
    /// A zero word (unused slot).
    None,
}

impl Operand {
    pub fn decode(word: u32, channels: usize, params: usize) -> Option<Self> {
        let (lo, hi) = (word as u16, (word >> 16) as u16);
        if word == 0 {
            return Some(Self::None);
        }
        if hi & 0xFFF0 == 0xEB40 {
            return Some(Self::Engine { space: (hi & 0xFF) as u8, value: lo });
        }
        // Stored channels live after a 0x30-byte bucket header, so lo is never 0 for them.
        if hi < 0x40 && hi % 4 == 0 && lo != 0 && ((hi / 4) as usize) < channels {
            return Some(Self::Channel { index: (hi / 4) as u8, scratch: lo & 1 == 1 });
        }
        if (0x40..0x80).contains(&hi) && hi % 2 == 0 && (((hi - 0x40) / 2) as usize) < params {
            return Some(Self::Param { index: ((hi - 0x40) / 2) as u8 });
        }
        None
    }

    pub fn channel(self) -> Option<u8> {
        match self {
            Self::Channel { index, .. } => Some(index),
            _ => None,
        }
    }

    pub fn param(self) -> Option<u8> {
        match self {
            Self::Param { index } => Some(index),
            _ => None,
        }
    }
}

/// A 5-key curve of u8 values over u8 times (both ÷255), as used by the emission controllers and
/// the colour action's alpha. Before the first key the curve holds its first value, after the last
/// its last; keys at times 0xFC–0xFF pad short curves (`ff ff ff ff ff | 00 fc fd fe ff` = 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve5 {
    pub values: [u8; 5],
    pub times: [u8; 5],
}

impl Curve5 {
    pub fn read(r: &Reader, o: usize) -> Result<Self> {
        let b = r.bytes(o, 10)?;
        Ok(Self { values: [b[0], b[1], b[2], b[3], b[4]], times: [b[5], b[6], b[7], b[8], b[9]] })
    }

    pub fn eval(&self, x: f32) -> f32 {
        let t = |i: usize| self.times[i] as f32 / 255.0;
        let v = |i: usize| self.values[i] as f32 / 255.0;
        if x <= t(0) {
            return v(0);
        }
        for i in 1..5 {
            if x <= t(i) {
                let span = t(i) - t(i - 1);
                let f = if span > 0.0 { (x - t(i - 1)) / span } else { 1.0 };
                return v(i - 1) + (v(i) - v(i - 1)) * f;
            }
        }
        v(4)
    }
}

/// Decoded payload of the action kinds the runtime interprets. Field meanings are read from the
/// data (value ranges, which channels each kind touches, which group it runs in); names ending in
/// `_guess` are the least certain. Unlisted kinds stay [`Op::Other`].
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// 0x33 / 0x34 / 0x42: per-layer emission controller (update and render groups). Four curves
    /// over the effect's inputs (`Intensity`, speeds, impact velocity); writes layer state `.1`.
    Controller { inputs: Vec<Operand>, curves: [Curve5; 4], scales: [f32; 4] },
    /// Spawners (update-group primaries with initialiser secondaries): 0x00 at the transform, 0x01
    /// from the collision mesh, 0x05 at a wheel, 0x07 at a wheel's ground contact.
    Spawn(Spawner),
    /// 0x0C: `Age = 0`, `RecipLifetime = 1 / rand(min, max)` (seconds).
    Lifetime { age: Operand, recip_lifetime: Operand, min: f32, max: f32 },
    /// 0x12: `Radius = rand(min, max)` (metres).
    Radius { radius: Operand, min: f32, max: f32 },
    /// 0x0A: sprite rotation `rand(min, max)` turns and angular velocity in turns per second.
    Rotation2dInit { rotation: Operand, angular_velocity: Operand, min: f32, max: f32, spin: f32 },
    /// 0x0D: `Alpha = rand(min, max)`.
    AlphaInit { alpha: Operand, min: f32, max: f32 },
    /// 0x13: the particle's bounce plane from a parameter (the ground under the emitter).
    PlaneInit { source: Operand, plane: Operand },
    /// 0x15: move the spawned particle into the emitter's frame (`Transform` parameter) with an
    /// extra rotation (quaternion) and offset; 0x14 is the same without a parameter (local space).
    PlaceInFrame { transform: Operand, rotation: [f32; 4], offset: [f32; 3], position: Operand, velocity: Operand },
    /// 0x16: add `rand(min, max)` × a velocity parameter (the emitter's) to the particle.
    InheritVelocity { velocity_param: Operand, min: f32, max: f32, velocity: Operand },
    /// 0x2F: `Age += dt × RecipLifetime`; the particle dies at 1.
    Age { age: Operand, recip_lifetime: Operand },
    /// 0x2C: `Rotation += AngularVelocity × dt`, with damping.
    Rotation2d { rotation: Operand, angular_velocity: Operand, damping: f32 },
    /// 0x3D: integrate: vertical acceleration (m/s², -9.8 for debris), drag (1/s).
    Integrate { position: Operand, velocity: Operand, gravity: f32, drag: f32, extra_guess: [f32; 2] },
    /// 0x3E: integrate and collide with the bounce plane: restitution and friction ranges, then
    /// gravity and drag as in 0x3D.
    Bounce {
        position: Operand,
        velocity: Operand,
        plane: Operand,
        restitution: [f32; 2],
        friction: [f32; 2],
        gravity: f32,
        drag: f32,
        extra_guess: [f32; 2],
    },
    /// 0x3C: `Radius* = Radius × bezier(Age; p0..p3)`.
    RadiusCurve { age: Operand, radius: Operand, out: Operand, bezier: [f32; 4] },
    /// 0x1C: `Colour*.rgb = Σ rows[k] · Ageᵏ` (k = 0..4, columns r g b and a brightness factor
    /// that multiplies rgb), `Colour*.a = alpha(Age) × alpha_scale`; a colour parameter may tint it.
    ColourCurve { age: Operand, out: Operand, tint: Operand, rows: [[f32; 4]; 5], alpha: Curve5, alpha_scale: f32 },
    /// 0x17: `Colour*.a *= Alpha`.
    AlphaApply { alpha: Operand, colour: Operand },
    /// 0x38: camera-relative adjustments of `Radius*` / `Colour*`: near fade over `near_fade` metres.
    ViewFade { near_fade: f32, far_guess: f32 },
    /// 0x1D: `SubUV* = Age` (the shader picks and blends atlas frames from it).
    SubUv { out: Operand, age: Operand },
    /// 0x47: a random seed per particle.
    SeedInit { seed: Operand },
    /// 0x48: random brightness in `[min, max]` from the seed.
    SeedTint { colour: Operand, seed: Operand, min: f32, max: f32 },
    /// Renderers (render-group primaries): camera-facing quads (0x24, 0x27 with atlas frames, 0x28
    /// seeded) and motion-blurred sprites along the velocity (0x25, 0x29).
    Render(Renderer),
    /// Any other kind (kept as raw bytes in [`Action::data`]).
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spawner {
    pub kind: u8,
    /// Particles per second at full emission (0x05 / 0x07 / 0x00: f32; 0x01: u32, while scraping).
    pub rate: f32,
    /// Particles per metre the emitter moves (0x05 at 0x70: per metre of sliding, a guess; 0x07, 0x00).
    pub per_metre: f32,
    /// Particles spawned at once when the effect starts or on an impact (0x01, 0x00).
    pub burst: u32,
    /// Initial speed range (m/s).
    pub speed: [f32; 2],
    /// Two angle ranges in degrees (elevation-like, then azimuth-like).
    pub angles: [[f32; 2]; 2],
    /// Camera distance range over which emission fades out (metres; 15000–16000 = never).
    pub lod: [f32; 2],
    /// 0x05: lateral spread across the tyre (in half widths) and height offset (metres).
    pub lateral: [f32; 2],
    pub height: [f32; 2],
    pub position: Operand,
    pub velocity: Operand,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Renderer {
    pub kind: u8,
    pub position: Operand,
    pub velocity: Operand,
    pub radius: Operand,
    pub rotation: Operand,
    pub colour: Operand,
    pub subuv: Operand,
    pub material: Option<ResourceId>,
    /// Sprite pivot (0.5, 0.5 = centred) for the quad renderers.
    pub pivot: [f32; 2],
    /// Lighting inputs passed to the particle shader for the quad renderers: (translucency, key
    /// light as ambient, wrap); the vertex program reads them as TEXCOORD1.zw and TEXCOORD3.w.
    pub lighting: [f32; 3],
    /// First parameter word (0.01–4; soft-particle depth fade distance in metres, a guess).
    pub depth_fade_guess: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub kind: u8,
    /// Resource offset of the payload.
    pub offset: usize,
    pub data: Vec<u8>,
    pub op: Op,
}

impl Action {
    pub fn f32(&self, o: usize) -> f32 {
        Reader(&self.data).f32(o).unwrap_or(0.0)
    }
}

/// One step of an action sequence: an action and the actions it runs for each particle it spawns.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub action: u8,
    pub secondaries: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    /// Runs once per bucket of particles (otherwise once per layer).
    pub per_bucket: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionGroup {
    pub sequences: Vec<Sequence>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub max_particles: u16,
    pub flags: u8,
    pub sort_group: i8,
    pub particles_per_bucket: u8,
    pub pre_roll: u8,
    pub channels: Vec<Channel>,
    pub actions: Vec<Action>,
    /// Group 0 = update (spawn, simulate), 1 = render; 2 and 3 are unused.
    pub groups: [Option<ActionGroup>; 4],
}

impl Layer {
    pub fn channel(&self, name: &str) -> Option<u8> {
        self.channels.iter().position(|c| c.name == name).map(|i| i as u8)
    }

    /// The layer's renderer, if it has one.
    pub fn renderer(&self) -> Option<&Renderer> {
        self.actions.iter().find_map(|a| if let Op::Render(r) = &a.op { Some(r) } else { None })
    }

    pub fn spawner(&self) -> Option<&Spawner> {
        self.actions.iter().find_map(|a| if let Op::Spawn(s) = &a.op { Some(s) } else { None })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BearEffect {
    pub id: ResourceId,
    pub params: Vec<Param>,
    pub layers: Vec<Layer>,
}

fn cstr(r: &Reader, o: usize) -> Result<(String, usize)> {
    let mut end = o;
    while r.u8(end)? != 0 {
        end += 1;
    }
    Ok((String::from_utf8_lossy(r.bytes(o, end - o)?).into_owned(), end + 1))
}

impl BearEffect {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let version = r.u32(0)?;
        if version != VERSION {
            return Err(Error::Malformed(format!("BearEffect version {version:#x}")));
        }
        let layer_count = r.u16(4)? as usize;
        let mut params = Vec::new();
        if layer_count > 0 {
            let n = r.u8(0x20)? as usize;
            let kinds = r.bytes(0x28, n)?;
            let handles = r.u16(0x10)? as usize;
            let mut o = 0x28 + n;
            for (i, &kind) in kinds.iter().enumerate() {
                let (name, next) = cstr(&r, o)?;
                o = next;
                params.push(Param {
                    name,
                    kind,
                    offset: r.u16(handles + 4 * i)? & !1,
                    id: r.u16(handles + 4 * i + 2)?,
                });
            }
        }
        let layer_table = r.u16(0x12)? as usize;
        let layers = (0..layer_count)
            .map(|i| parse_layer(res, &r, r.u16(layer_table + 2 * i)? as usize, params.len()))
            .collect::<Result<_>>()?;
        Ok(Self { id: res.id, params, layers })
    }

    pub fn param(&self, name: &str) -> Option<u8> {
        self.params.iter().position(|p| p.name == name).map(|i| i as u8)
    }
}

fn parse_layer(res: &Resource, r: &Reader, base: usize, nparams: usize) -> Result<Layer> {
    let max_particles = r.u16(base)?;
    let ch = base + r.u16(base + 2)? as usize;
    let act = base + r.u16(base + 4)? as usize;
    let groups_at = base + r.u16(base + 6)? as usize;
    let size = r.u16(base + 0xA)? as usize;
    let flags = r.u8(base + 0xC)?;
    let action_count = r.u8(base + 0xD)? as usize;
    let (name, _) = cstr(r, base + 0x20)?;

    let nch = r.u8(ch)? as usize;
    let kinds = r.bytes(ch + 4, nch)?;
    let mut o = ch + 4 + nch;
    let mut channels = Vec::with_capacity(nch);
    for &kind in kinds {
        let (name, next) = cstr(r, o)?;
        o = next;
        channels.push(Channel { name, kind });
    }

    let offsets: Vec<usize> = (0..action_count)
        .map(|k| r.u16(act + 2 * k).map(|v| base + v as usize))
        .collect::<std::result::Result<_, _>>()?;
    let kinds = r.bytes(act + 2 * action_count, action_count)?;
    let mut actions = Vec::with_capacity(action_count);
    for k in 0..action_count {
        let start = offsets[k];
        let end = if k + 1 < action_count { offsets[k + 1] } else { groups_at };
        if end < start {
            return Err(Error::Malformed(format!("BearEffect action {k} of layer {name} ends before it starts")));
        }
        let data = r.bytes(start, end - start)?.to_vec();
        let op = decode_op(res, kinds[k], start, &data, nch, nparams)?;
        actions.push(Action { kind: kinds[k], offset: start, data, op });
    }

    let mut groups: [Option<ActionGroup>; 4] = Default::default();
    for (g, slot) in groups.iter_mut().enumerate() {
        let off = r.u16(groups_at + 2 * g)? as usize;
        if off == 0 {
            continue;
        }
        let p = base + off;
        let (nseq, nprim, nsec) = (r.u8(p)? as usize, r.u8(p + 1)? as usize, r.u8(p + 2)? as usize);
        let seqs = r.bytes(p + 4, 2 * nseq)?;
        let prims = r.bytes(p + 4 + 2 * nseq, 2 * nprim)?;
        let secs = r.bytes(p + 4 + 2 * nseq + 2 * nprim, nsec)?;
        let check = |a: u8| {
            if (a as usize) < action_count {
                Ok(a)
            } else {
                Err(Error::Malformed(format!("BearEffect group {g} of layer {name} names action {a}")))
            }
        };
        let (mut pi, mut si) = (0usize, 0usize);
        let mut sequences = Vec::with_capacity(nseq);
        for s in seqs.as_chunks::<2>().0 {
            let mut steps = Vec::with_capacity(s[0] as usize);
            for _ in 0..s[0] {
                let pr = prims.get(2 * pi..2 * pi + 2).ok_or_else(|| Error::Malformed("BearEffect sequence".into()))?;
                pi += 1;
                let n = pr[1] as usize;
                let sec = secs.get(si..si + n).ok_or_else(|| Error::Malformed("BearEffect secondaries".into()))?;
                si += n;
                steps.push(Step {
                    action: check(pr[0])?,
                    secondaries: sec.iter().map(|&a| check(a)).collect::<Result<_>>()?,
                });
            }
            sequences.push(Sequence { per_bucket: s[1] != 0, steps });
        }
        if pi != nprim || si != nsec {
            return Err(Error::Malformed(format!("BearEffect group {g} of layer {name}: counts disagree")));
        }
        *slot = Some(ActionGroup { sequences });
    }

    if groups_at > base + size {
        return Err(Error::Malformed(format!("BearEffect layer {name}: actions overrun the layer")));
    }

    Ok(Layer {
        name,
        max_particles,
        flags,
        sort_group: r.u8(base + 0xF)? as i8,
        particles_per_bucket: r.u8(base + 0x10)?,
        pre_roll: r.u8(base + 0x11)?,
        channels,
        actions,
        groups,
    })
}

/// Decode a known action kind; payloads that are too short or reference missing channels are errors.
fn decode_op(res: &Resource, kind: u8, start: usize, data: &[u8], nch: usize, np: usize) -> Result<Op> {
    let r = Reader(data);
    let f = |o: usize| r.f32(o);
    let opnd = |o: usize| -> Result<Operand> {
        let w = r.u32(o)?;
        Operand::decode(w, nch, np).ok_or_else(|| {
            Error::Malformed(format!("BearEffect action {kind:#04x}: word {w:#010x} at +{o:#x} is no operand"))
        })
    };
    let pair = |o: usize| -> Result<[f32; 2]> { Ok([f(o)?, f(o + 4)?]) };
    let vec4 = |o: usize| -> Result<[f32; 4]> { r.f32_array::<4>(o).map_err(Into::into) };
    let import = |o: usize| res.import_at((start + o) as u32);
    Ok(match kind {
        0x33 | 0x34 | 0x42 => {
            let (ninputs, curves_at) = if kind == 0x42 { (4, 0x10) } else { (6, 0x18) };
            let inputs = (0..ninputs).map(|i| opnd(4 * i)).collect::<Result<Vec<_>>>()?;
            let curves = [
                Curve5::read(&r, curves_at)?,
                Curve5::read(&r, curves_at + 10)?,
                Curve5::read(&r, curves_at + 20)?,
                Curve5::read(&r, curves_at + 30)?,
            ];
            Op::Controller { inputs, curves, scales: vec4(curves_at + 40)? }
        }
        0x05 => Op::Spawn(Spawner {
            kind,
            rate: f(0x74)?,
            per_metre: f(0x70)?,
            burst: 0,
            speed: pair(0x10)?,
            angles: [pair(0x18)?, pair(0x20)?],
            lod: pair(0x08)?,
            lateral: pair(0x38)?,
            height: pair(0x40)?,
            position: opnd(0x60)?,
            velocity: opnd(0x64)?,
        }),
        0x07 => Op::Spawn(Spawner {
            kind,
            rate: f(0x64)?,
            per_metre: f(0x60)?,
            burst: 0,
            speed: pair(0x20)?,
            angles: [pair(0x28)?, pair(0x30)?],
            lod: pair(0x18)?,
            lateral: [0.0; 2],
            height: [0.0; 2],
            position: opnd(0x50)?,
            velocity: opnd(0x54)?,
        }),
        0x01 => Op::Spawn(Spawner {
            kind,
            rate: r.u32(0x54)? as f32,
            per_metre: 0.0,
            burst: r.u32(0x58)?,
            speed: pair(0x00)?,
            angles: [pair(0x08)?, pair(0x10)?],
            lod: pair(0x20)?,
            lateral: [0.0; 2],
            height: [0.0; 2],
            position: opnd(0x3C)?,
            velocity: opnd(0x40)?,
        }),
        0x00 => Op::Spawn(Spawner {
            kind,
            rate: f(0x60)?,
            per_metre: f(0x64)?,
            burst: r.u32(0x6C)?,
            speed: pair(0x38)?,
            angles: [pair(0x28)?, pair(0x30)?],
            lod: pair(0x20)?,
            lateral: [0.0; 2],
            height: [0.0; 2],
            position: opnd(0x58)?,
            velocity: opnd(0x5C)?,
        }),
        0x0C => Op::Lifetime { age: opnd(4)?, recip_lifetime: opnd(8)?, min: f(12)?, max: f(16)? },
        0x12 => Op::Radius { radius: opnd(4)?, min: f(8)?, max: f(12)? },
        0x0A => Op::Rotation2dInit {
            min: f(0)?,
            max: f(4)?,
            rotation: opnd(12)?,
            angular_velocity: opnd(16)?,
            spin: f(20)?,
        },
        0x0D => Op::AlphaInit { min: f(0)?, max: f(4)?, alpha: opnd(12)? },
        0x13 => Op::PlaneInit { source: opnd(0)?, plane: opnd(4)? },
        0x15 => Op::PlaceInFrame {
            rotation: vec4(0)?,
            offset: [f(16)?, f(20)?, f(24)?],
            transform: opnd(52)?,
            position: opnd(64)?,
            velocity: opnd(68)?,
        },
        0x14 => Op::PlaceInFrame {
            rotation: vec4(0)?,
            offset: [f(16)?, f(20)?, f(24)?],
            transform: Operand::None,
            position: opnd(52)?,
            velocity: opnd(56)?,
        },
        0x16 => Op::InheritVelocity { min: f(0)?, max: f(4)?, velocity_param: opnd(8)?, velocity: opnd(28)? },
        0x2F => Op::Age { age: opnd(4)?, recip_lifetime: opnd(8)? },
        0x2C => Op::Rotation2d { angular_velocity: opnd(4)?, rotation: opnd(8)?, damping: f(12)? },
        0x3D => Op::Integrate {
            position: opnd(4)?,
            velocity: opnd(8)?,
            gravity: f(12)?,
            drag: f(16)?,
            extra_guess: pair(20)?,
        },
        0x3E => Op::Bounce {
            position: opnd(12)?,
            velocity: opnd(16)?,
            plane: opnd(24)?,
            restitution: pair(28)?,
            friction: pair(36)?,
            gravity: f(44)?,
            drag: f(48)?,
            extra_guess: pair(52)?,
        },
        0x3C => Op::RadiusCurve { age: opnd(4)?, radius: opnd(12)?, out: opnd(16)?, bezier: vec4(20)? },
        0x1C => Op::ColourCurve {
            age: opnd(0)?,
            out: opnd(8)?,
            tint: opnd(12)?,
            rows: [vec4(32)?, vec4(48)?, vec4(64)?, vec4(80)?, vec4(96)?],
            alpha: Curve5::read(&r, 144)?,
            alpha_scale: f(156)?,
        },
        0x17 => Op::AlphaApply { alpha: opnd(0)?, colour: opnd(8)? },
        0x38 => Op::ViewFade { near_fade: f(80)?, far_guess: f(84)? },
        0x1D => Op::SubUv { out: opnd(0)?, age: opnd(4)? },
        0x47 => Op::SeedInit { seed: opnd(4)? },
        0x48 => Op::SeedTint { min: f(0)?, max: f(4)?, colour: opnd(16)?, seed: opnd(20)? },
        0x24 | 0x27 | 0x28 => {
            // Common shape: references, then (depth fade, colour scale ×3/4, pivot, translucency,
            // ambient key, wrap, ...) and the material slot.
            let (refs, params, material) = match kind {
                0x24 => (4, 28, 72),
                0x27 => (8, 36, 80),
                _ => (40, 68, 112),
            };
            let at = |i: usize| opnd(refs + 4 * i);
            let (position, radius, rotation, colour, subuv) = match kind {
                0x24 => (opnd(4)?, opnd(16)?, opnd(20)?, opnd(24)?, Operand::None),
                0x27 => (at(0)?, at(3)?, at(4)?, at(5)?, at(6)?),
                _ => (at(0)?, at(3)?, at(4)?, at(5)?, Operand::None),
            };
            let (pivot_at, light_at) = match kind {
                0x27 => (params + 16, params + 24),
                _ => (params + 20, params + 28),
            };
            Op::Render(Renderer {
                kind,
                position,
                velocity: Operand::None,
                radius,
                rotation,
                colour,
                subuv,
                material: import(material),
                pivot: pair(pivot_at)?,
                lighting: [f(light_at)?, f(light_at + 4)?, f(light_at + 8)?],
                depth_fade_guess: f(params)?,
            })
        }
        0x25 | 0x29 => Op::Render(Renderer {
            kind,
            position: opnd(8)?,
            velocity: opnd(12)?,
            radius: opnd(24)?,
            rotation: Operand::None,
            colour: opnd(28)?,
            subuv: Operand::None,
            material: import(32),
            pivot: [0.5; 2],
            lighting: [0.0; 3],
            depth_fade_guess: 0.0,
        }),
        _ => Op::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve5() {
        let one = Curve5 { values: [255; 5], times: [0, 0xFC, 0xFD, 0xFE, 0xFF] };
        assert_eq!(one.eval(0.3), 1.0);
        // Tyre smoke emission: ramps from 0 at 36/255 to 1 at the end.
        let ramp = Curve5 { values: [0, 74, 149, 210, 255], times: [36, 52, 84, 158, 255] };
        assert_eq!(ramp.eval(0.0), 0.0);
        assert!((ramp.eval(52.0 / 255.0) - 74.0 / 255.0).abs() < 1e-6);
        assert_eq!(ramp.eval(1.0), 1.0);
    }

    #[test]
    fn operands() {
        assert_eq!(Operand::decode(0x0000_0030, 6, 8), Some(Operand::Channel { index: 0, scratch: false }));
        assert_eq!(Operand::decode(0x000c_0031, 6, 8), Some(Operand::Channel { index: 3, scratch: true }));
        assert_eq!(Operand::decode(0x0042_0040, 6, 8), Some(Operand::Param { index: 1 }));
        assert_eq!(Operand::decode(0xeb42_00a0, 6, 8), Some(Operand::Engine { space: 0x42, value: 0xa0 }));
        assert_eq!(Operand::decode(0x3f80_0000, 6, 8), None);
    }
}
