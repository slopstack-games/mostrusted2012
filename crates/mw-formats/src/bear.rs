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
    /// 0x33 `UpdateBindings` / 0x34 `UpdateBindings_Wheel` / 0x42 `UpdateWheelBindings`: per-layer
    /// emission controller (update and render groups). Four curves over the effect's inputs
    /// (`Intensity`, speeds, impact velocity, engine globals), combined into the layer's binding slots
    /// that spawners read ([`Bindings`]; decoded for 0x33 / 0x34 only).
    Controller { inputs: Vec<Operand>, curves: [Curve5; 4], scales: [f32; 4], bindings: Option<Bindings> },
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
    /// 0x0B `Initialise3dRotation` (compile `0x00449e20`, run `0x00453ee0` → `FUN_00453070`): a quaternion
    /// (x, y, z, w) and a world angular velocity (rad/s). `random_rotation` (byte +0x45): `normalize(2r − 1)` of four
    /// uniform randoms, else the Euler offset (radians, `qz ⊗ qy ⊗ qx`) ⊗ the `Transform`'s rotation.
    /// `random_spin` (byte +0x44): a uniform random axis at `revs` × 2π rad/s, else `local_spin` (rad/s, emitter
    /// space) turned by the `Transform`.
    Rotation3dInit {
        rotation: Operand,
        angular_velocity: Operand,
        euler: [f32; 3],
        local_spin: [f32; 3],
        revs: f32,
        random_spin: bool,
        random_rotation: bool,
    },
    /// 0x2D `Update3dRotation` (run `0x0045e2d0` → `FUN_0045ddf0`): unless every |ω| component is ≤ `FLT_EPSILON`,
    /// `q ← (ω̂ sin(|ω| dt / 2), cos(|ω| dt / 2)) ⊗ q` (not renormalised), then `ω ×= (1 − damping)^dt` (`powf`):
    /// `damping` is the fraction lost per second.
    Rotation3d { rotation: Operand, angular_velocity: Operand, damping: f32 },
    /// 0x40 `UpdatePosVelWithWorldCollision` (compile `0x0044bec0`, a plain copy; run `0x00462f00` → `FUN_00462750`):
    /// rigid debris. A particle whose collision radius is negative is at rest and skipped. Otherwise `p += dt·v`,
    /// `v = R·v + Δ` with `R = (1 − drag)^dt` and `Δ = (1 − R)·wind·W + (0, gravity·dt, 0) + turbulence·dt·(2r − 1)`;
    /// the segment from the old position to the new one plus `v̂ · CR` is tested against the world triangles cached
    /// around the player's car (front faces); on a hit the particle is put `CR` above the plane, `Δ` is taken out of
    /// its velocity, the tangential part kept by `k = rand(1 − keep[1] .. 1 − keep[0])`, the normal part reflected
    /// by `restitution`, and it comes to rest (velocity and spin 0, `CR` negated) when its speed is at most
    /// `rest_speed` on ground whose normal has y > 0.9; else its spin is scaled by `k`.
    WorldCollide {
        position: Operand,
        velocity: Operand,
        collision_radius: Operand,
        keep: [f32; 2],
        gravity: f32,
        drag: f32,
        wind: f32,
        turbulence: f32,
        restitution: f32,
        rest_speed: f32,
    },
    /// 0x11 `InitialiseCollisionRadius` (run `0x0044a380` → `FUN_0044a200`): `CR = 0.01 + table[serial & 15] · 0.99 /
    /// 255` (`.rdata` 0x00f9d474 / 0x00f9d478).
    CollisionRadiusInit { table: [u8; 16], radius: Operand },
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
    /// 0x1C `ParametricSetColourGradient` (`FUN_00449030` compiles, `FUN_004553b0` runs it): a piecewise
    /// linear gradient over the particle's age. With `u = frac(Age · rates[0])` and ramp weights
    /// `w_k = clamp(u · ramp[0][k] + ramp[1][k], 0, 1)`, `c = rows[0] + Σ w_k · rows[k + 1]`; then
    /// `Colour*.rgb = c.rgb · c.w` (the fourth column is a brightness) and `Colour*.a = alpha(frac(Age ·
    /// rates[1])) · slot[alpha_slot]` (a binding slot of the layer). With `use_tint` the colour parameter
    /// replaces rgb and multiplies the alpha.
    ColourCurve {
        age: Operand,
        out: Operand,
        tint: Operand,
        rows: [[f32; 4]; 5],
        ramp: [[f32; 4]; 2],
        alpha: Curve5,
        rates: [f32; 2],
        alpha_slot: u8,
        use_tint: bool,
    },
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
    /// 0x1A `ParametricPosVelToWorldSpace` / 0x1B `...WithLocalTransform` (render group): the layer's
    /// particles live in the emitter's space (spawned with 0x14 `InitialisePosOffset`) and are placed with
    /// the current `Transform` each frame, so they follow a moving emitter (the exhaust flames); 0x1B first
    /// applies its own rotation and offset.
    ToWorld { rotation: [f32; 4], offset: [f32; 3] },
    /// 0x2B `SuperEmitter` (compile `0x0044b2c0` → `FUN_00448370`, a plain copy; run `0x0045dc40` →
    /// `FUN_0045d020`, both outside the dump's functions): a layer whose particles are born along the
    /// particles of another layer (`parent`, from the high bits of the parent-channel operands at +0x24..+0x30,
    /// which carry 0x80 | 4·channel). Per parent particle with `t = Age · RecipLifetime < 1`, each update emits
    /// `(1 + (rate_end − 1)·t) · (per_metre · |v·dt| + rate · dt)` children (the fraction carried to the
    /// next update), each at `parent position − u·v·dt` (`u` uniform, along the parent's path this update),
    /// leaving at `cone` degrees (uniform, about a uniform azimuth) from the parent's backward direction
    /// `−v̂` with `rand(speed)` m/s, plus `rand(inherit)` × the parent's velocity; radius `rand(radius)` ×
    /// `(1 + (radius_end − 1)·t)` at the birth time. The byte at +0x50 picks a LOD class (not ported).
    SuperEmitter {
        parent: u8,
        cone: [f32; 2],
        speed: [f32; 2],
        inherit: [f32; 2],
        radius: [f32; 2],
        rate: f32,
        per_metre: f32,
        radius_end: f32,
        rate_end: f32,
    },
    /// Any other kind (kept as raw bytes in [`Action::data`]).
    Other,
}

/// How an `UpdateBindings` controller fills its layer's binding slots, from the game's code
/// (`FUN_0045fc60` compiles the payload, `FUN_0045ffd0` runs it every update; research log 2026-10-09
/// "exhaust effects"). Slot 0 is never written (read as 1 here: spawners scale by it where nothing is
/// bound). Curve `i` writes slot `1 + i` = `curve_i(x_i) + offsets[i]`, where `x_i` comes from
/// `sources[i]` (`FUN_00448620`; `s` = `scales[i]`, `t` = seconds since the layer started):
///
/// | source | x |
/// |--------|---|
/// | 0 | `s·t` |
/// | 1 | `frac(s·t)` |
/// | 3 | `s·|Velocity|` (the velocity input, m/s) |
/// | 4 | `s·` distance from the transform to the camera (global +0x30) |
/// | 7 / 8 | `0.5 + 0.5·sin / cos(2π·s·t)` |
/// | 9 | a random number drawn when the action is compiled (it replaces the scale) |
/// | 10 | `Intensity` |
/// | 15 | `frac(s·(t + r))`, `r` a random phase drawn per layer |
/// | 16 | `BearGlobalParameters` +0x74: the player car's engine load (vehicle +0x1f20) |
/// | 17 / 18 | `ImpactClosingVelocity / s`, `MovementSinceLastFrame / s` |
/// | 2, 5, 6, 11–14, 19 | other globals (+0x68 × 1/86400, `s` × +0x60, byte +0x7c, +0x6c, byte +0x7d (0), +0x70, byte +0x7e, +0x78) |
///
/// Then four combine steps (`FUN_004492a0`) write slot `5 + k` = clamp01(op(slot[b], slot[a])):
/// op 0 `b·a`, 1 `max`, 2 `min`, 3 `(b + a) / 2`, 4 `b + a`, 5 `(1 − a)·b + a`, 6 `a − b`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bindings {
    pub sources: [u8; 4],
    pub offsets: [f32; 4],
    /// `[a, b, op]` per combine step.
    pub combine: [[u8; 3]; 4],
}

impl Bindings {
    pub const SLOTS: usize = 9;

    /// Run the combine steps over `slot` (curves already in 1..=4).
    pub fn combine(&self, slot: &mut [f32; Self::SLOTS]) {
        for (k, &[a, b, op]) in self.combine.iter().enumerate() {
            let a = slot.get(a as usize).copied().unwrap_or(0.0);
            let b = slot.get(b as usize).copied().unwrap_or(0.0);
            let v = match op {
                0 => b * a,
                1 => b.max(a),
                2 => b.min(a),
                3 => (b + a) * 0.5,
                4 => b + a,
                5 => (1.0 - a) * b + a,
                6 => a - b,
                _ => b,
            };
            slot[5 + k] = v.clamp(0.0, 1.0);
        }
    }
}

/// `EmitterConePlus` (spawner 0x00) fields beyond the common ones, from the game's code (`FUN_0044ccc0`
/// compiles, `FUN_0044cf30` runs it): it emits only while `IsEmitting` is set, the layer's time is in
/// `time_window`, the squared distance from the camera (`BearGlobalParameters` +0x30) is in `camera_window` and the
/// velocity input's length is within `speed_window` (m/s). Per update it adds `(rate + per_metre ·
/// |Velocity|) · dt · slot[state[0]]` (× the distance fade over `lod`) to an accumulator that starts at
/// `burst + 0.9999` (× `slot[state[1]]` while it is above 1) and emits its rounded value; initial
/// speeds are scaled by `slot[state[2]]`, the cone angles by `slot[state[3]]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConeEmitter {
    pub time_window: [f32; 2],
    pub camera_window: [f32; 2],
    pub speed_window: [f32; 2],
    pub emitting: Operand,
    pub state: [u8; 4],
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
    /// `EmitterConePlus` (0x00) only.
    pub cone: Option<ConeEmitter>,
    /// `EmitterCollisionMesh` (0x01) only.
    pub mesh: Option<MeshEmitter>,
}

/// `EmitterCollisionMesh` (0x01; compile `FUN_00449630`, run `0x0044ccb0` → `FUN_0044bf90`): particles born on the
/// triangles of a car's collision mesh (the `PolygonSoupList` of its bundle, car space) around a contact point.
/// The compile step copies data word i to compiled word i + 4 for the speed and angle ranges and turns the angle
/// ranges into turns (`c0 = a_lo / 360`, `c1 = 4 (a_hi − a_lo) / 360`, `c2 = b_lo / 360 − c0`,
/// `c3 = 4 (b_hi − b_lo) / 360 − c1`; `.rdata` 0x00f9d1b0 = 1 / 360, 0x00f9c610 = 4).
///
/// Each update (while `IsEmitting` is set and the layer clock is within `time_window`, or the burst is still to
/// come): `n = burst · slot[burst] + rate · dt · slot[rate]` (the burst is spent), then for every polygon (its
/// first three vertices) whose centroid `C` lies within `radius · slot[radius]` of the contact point `P`, with
/// `(C − P) · N ≤ plane` and `(n̂ · N + 1) / 2 > facing` (`n̂` = the triangle's normal, `N` = the contact normal,
/// both in mesh space): `area · n · camera fade` particles (the fraction carried over, starting at 0.9999).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEmitter {
    /// Words 6 / 7 (+0x18): the layer clock window; past its end only a burst not yet spent emits.
    pub time_window: [f32; 2],
    /// Word 11 (+0x2c): the `IsEmitting` parameter; word 14 (+0x38): the `CollisionMeshParams` parameter.
    pub emitting: Operand,
    pub mesh: Operand,
    /// Words 18..20 (+0x48): the radius around the contact point (m), the largest offset of a triangle's centroid
    /// along the contact normal (m) and the facing threshold.
    pub radius: f32,
    pub plane: f32,
    pub facing: f32,
    /// Bytes +0x5c..+0x60: the binding slots that scale the radius, the rate, the burst, the speed and the
    /// angles (slot 0 reads 1).
    pub slots: [u8; 5],
    /// Byte +0x61: the emitter's LOD level (`.rdata` 0x00f9cf9c table; not used here).
    pub lod_level: u8,
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
    /// The word after it: the draw's opacity. The run functions store the pair at the draw state's +0x23c /
    /// +0x240 (`FUN_00458e90` for 0x24 words 7 / 8, `FUN_0045ab10` for 0x27 words 9 / 10, `FUN_0045b620`
    /// words 0x11 / 0x12), whose defaults are 0 and 1.0 (`FUN_004a…` reset); read as `gVfxConstants.z`,
    /// which the pixel programs multiply into the output alpha and alpha-test (inferred: the constant's
    /// writer for particle draws was not found). 0 on the nitrous glows (a `NoAlphaTest` material, so they
    /// add) and on a few alpha-tested layers (which then draw nothing); 1 almost everywhere else.
    pub opacity: f32,
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

    /// The emitter-space to world placement of a layer whose particles live in the emitter's space
    /// ([`Op::ToWorld`]): rotation and offset applied before the `Transform`.
    pub fn to_world(&self) -> Option<([f32; 4], [f32; 3])> {
        self.actions.iter().find_map(|a| match a.op {
            Op::ToWorld { rotation, offset } => Some((rotation, offset)),
            _ => None,
        })
    }

    /// The layer whose particles this layer's `SuperEmitter` follows, if it has one.
    pub fn super_emitter_parent(&self) -> Option<usize> {
        self.actions
            .iter()
            .find_map(|a| if let Op::SuperEmitter { parent, .. } = a.op { Some(parent as usize) } else { None })
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
            // `FUN_0045fc60`: offsets after the scales, then the source, `a`, `b` and op bytes.
            let bindings = if kind != 0x42 && data.len() >= 0x70 {
                let b = r.bytes(0x60, 16)?;
                Some(Bindings {
                    sources: [b[0], b[1], b[2], b[3]],
                    offsets: vec4(0x50)?,
                    combine: std::array::from_fn(|k| [b[4 + k], b[8 + k], b[12 + k]]),
                })
            } else {
                None
            };
            Op::Controller { inputs, curves, scales: vec4(curves_at + 40)?, bindings }
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
            cone: None,
            mesh: None,
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
            cone: None,
            mesh: None,
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
            cone: None,
            mesh: if data.len() >= 0x64 {
                let b = r.bytes(0x5c, 6)?;
                Some(MeshEmitter {
                    time_window: pair(0x18)?,
                    emitting: Operand::decode(r.u32(0x2c)?, nch, np).unwrap_or(Operand::None),
                    mesh: Operand::decode(r.u32(0x38)?, nch, np).unwrap_or(Operand::None),
                    radius: f(0x48)?,
                    plane: f(0x4c)?,
                    facing: f(0x50)?,
                    slots: [b[0], b[1], b[2], b[3], b[4]],
                    lod_level: b[5],
                })
            } else {
                None
            },
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
            cone: if data.len() >= 0x74 {
                let b = r.bytes(0x70, 4)?;
                Some(ConeEmitter {
                    time_window: pair(0x10)?,
                    camera_window: pair(0x18)?,
                    speed_window: pair(0x40)?,
                    emitting: Operand::decode(r.u32(0x50)?, nch, np).unwrap_or(Operand::None),
                    state: [b[0], b[1], b[2], b[3]],
                })
            } else {
                None
            },
            mesh: None,
        }),
        0x1A => Op::ToWorld { rotation: [0.0, 0.0, 0.0, 1.0], offset: [0.0; 3] },
        0x1B => Op::ToWorld { rotation: vec4(0)?, offset: [f(16)?, f(20)?, f(24)?] },
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
        0x0B if data.len() >= 0x48 => {
            let b = r.bytes(0x44, 2)?;
            Op::Rotation3dInit {
                rotation: opnd(4)?,
                angular_velocity: opnd(8)?,
                euler: [f(0x20)?, f(0x24)?, f(0x28)?],
                local_spin: [f(0x30)?, f(0x34)?, f(0x38)?],
                revs: f(0x40)?,
                random_spin: b[0] != 0,
                random_rotation: b[1] != 0,
            }
        }
        0x40 if data.len() >= 0x38 => Op::WorldCollide {
            position: opnd(8)?,
            velocity: opnd(12)?,
            collision_radius: opnd(20)?,
            keep: pair(24)?,
            gravity: f(32)?,
            drag: f(36)?,
            wind: f(40)?,
            turbulence: f(44)?,
            restitution: f(48)?,
            rest_speed: f(52)?,
        },
        0x11 if data.len() >= 0x14 => {
            let b = r.bytes(0, 16)?;
            Op::CollisionRadiusInit { table: std::array::from_fn(|i| b[i]), radius: opnd(16)? }
        }
        0x2D => Op::Rotation3d { rotation: opnd(4)?, angular_velocity: opnd(8)?, damping: f(12)? },
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
            ramp: [vec4(112)?, vec4(128)?],
            alpha: Curve5::read(&r, 144)?,
            rates: [f(156)?, f(160)?],
            alpha_slot: r.u8(154)?,
            use_tint: r.u8(155)? != 0,
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
                opacity: f(params + 4)?,
            })
        }
        // 3-D sprites (`Render3dSprite` 0x1E, `Render3dSpriteSubUV` 0x1F; run `FUN_00455670` / `FUN_00456110`): words
        // view, position, age, 1 / lifetime, radius, colour, rotation (quaternion), [SubUV], then the draw state's
        // +0x23c / +0x240 pair, a view-kind alpha factor, translucency, wrap, key as ambient and the material.
        0x1E | 0x1F => {
            let k = if kind == 0x1F { 1 } else { 0 };
            Op::Render(Renderer {
                kind,
                position: opnd(4)?,
                velocity: Operand::None,
                radius: opnd(16)?,
                rotation: opnd(24)?,
                colour: opnd(20)?,
                subuv: if kind == 0x1F { opnd(28)? } else { Operand::None },
                material: import(0x34 + 4 * k),
                pivot: [0.5; 2],
                lighting: [f(40 + 4 * k)?, f(48 + 4 * k)?, f(44 + 4 * k)?],
                depth_fade_guess: f(28 + 4 * k)?,
                opacity: f(32 + 4 * k)?,
            })
        }
        // Meshes (`RenderMesh` 0x22 with the car's paint colour, `RenderMeshNoPaintColour` 0x23; run `FUN_00457720`):
        // view, position, rotation, age, 1 / lifetime, radius, the Model import (+0x18; `material` holds it).
        0x22 | 0x23 => Op::Render(Renderer {
            kind,
            position: opnd(4)?,
            velocity: Operand::None,
            radius: opnd(20)?,
            rotation: opnd(8)?,
            colour: Operand::None,
            subuv: Operand::None,
            material: import(0x18),
            pivot: [0.5; 2],
            lighting: [0.0; 3],
            depth_fade_guess: 0.0,
            opacity: 1.0,
        }),
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
            opacity: 1.0,
        }),
        0x2B => Op::SuperEmitter {
            parent: (r.u32(0x24)? >> 23) as u8,
            cone: pair(0x00)?,
            speed: pair(0x08)?,
            inherit: pair(0x10)?,
            radius: pair(0x18)?,
            rate: f(0x40)?,
            per_metre: f(0x44)?,
            radius_end: f(0x48)?,
            rate_end: f(0x4c)?,
        },
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
