//! The cops' siren sounds (docs/decomp.md "Police sirens, light bars and radio"): `CopSoundSet` and its
//! `SirenSoundSet`s in `VEHICLES/WAVES.BNDL`.
//!
//! - `CopSoundSet` (`…23013`, 28 bytes): +0x10 / +0x18 `efd7715f` (one set, `…b3af6`) and +0x14 / +0x1a
//!   `9f467b30` (two sets, `…23039`, `…23014`).
//! - `SirenSoundSet` (60 bytes): +0x0c `3b47deee` (empty in all three), +0x14 `403328c3` the voices' mixer channel
//!   (`…76b17`: gain 0.186, doppler mode 1 = 680 m/s, falloff `…76b20`), +0x1c `89ecdfc4` a sequence (`…b3883`:
//!   a 0.5 s yelp item, gain 3, fade out 0.25 s), +0x28 `Malfunctions` (empty in all three), +0x2c `Blips`
//!   (4–5 waves, ~0.8–1.1 s chirps), +0x30 `Sirens` (three `SirenLevel`s, each a list of waves), +0x38
//!   `21cee134` (0 in all three).
//!
//! The waves (numeric analysis, research log 2026-10-16): in `…23014` level 0 `…2301e` is a 5 s wail (656 →
//! 1641 → 680 Hz), level 1 `…23022` a yelp (600 → 1690 Hz, three sweeps a second), level 2 `…2301b` a hi-lo
//! (1066 / 1559 Hz alternating); the blips are 0.8–1.1 s chirps between 560 and 1650 Hz.

use std::sync::Arc;

use mw_audio::sfx_player::Clip;
use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::car_sound::Spatial;
use crate::sfx_loader::{SoundLoader, number};

/// `CopSoundSet` fields.
const SETS_A: u32 = 0xefd7_715f;
const SETS_B: u32 = 0x9f46_7b30;
/// `SirenSoundSet` fields.
const CHANNEL: u32 = 0x4033_28c3;
const SEQUENCE: u32 = 0x89ec_dfc4;
const FLAG: u32 = 0x21ce_e134;

/// One `SirenSoundSet`.
#[derive(Debug, Clone)]
pub struct SirenSet {
    pub id: ResourceId,
    /// `Sirens`: the tones; each a list of waves.
    pub levels: Vec<Vec<Arc<Clip>>>,
    /// `Blips`: short chirps.
    pub blips: Vec<Arc<Clip>>,
    /// `Malfunctions` (empty in the data).
    pub malfunctions: Vec<Arc<Clip>>,
    /// `403328c3`: the channel's static gain and how it places the voice.
    pub gain: f32,
    pub spatial: Spatial,
    /// `89ecdfc4`: the sequence's first wave, its gain, length (s) and fade-out (s).
    pub burst: Option<Burst>,
    /// `89ecdfc4` as a one-shot: the dying siren (`…b3883`: the yelp for 0.5 s, then one of four 2 s
    /// wind-downs), played once when a car with its siren on is wrecked (`FUN_0058bdc0`).
    pub dying: Option<Arc<crate::car_sfx::OneShot>>,
    /// `21cee134`.
    pub flag: u8,
}

/// A one-item sequence of a siren set (`89ecdfc4`).
#[derive(Debug, Clone)]
pub struct Burst {
    pub id: ResourceId,
    pub wave: Arc<Clip>,
    pub gain: f32,
    pub length: f32,
    pub fade_out: f32,
}

/// The `CopSoundSet`: its two lists of siren sets, and the mix snapshot's siren channels.
#[derive(Debug, Clone, Default)]
pub struct CopSounds {
    /// `efd7715f` (+0x10; no reader found).
    pub sets_a: Vec<Arc<SirenSet>>,
    /// `9f467b30` (+0x14): a car whose `VehicleSound` +0x40 is empty (every cop) takes
    /// `sets_b[vehicle slot % count]` (`FUN_00593ac0`).
    pub sets_b: Vec<Arc<SirenSet>>,
    /// The snapshot channels the siren voice plays on (`FUN_0058bdc0` → `FUN_00574510`, the entity's channel
    /// set +0x40 / +0x48): `Sirens_Competitor` for other cars, `Player_Vehicle_Sirens` for the player's,
    /// `Sirens_Roadblock` for roadblock cars.
    pub competitor: Option<SirenChannel>,
    pub player: Option<SirenChannel>,
    pub roadblock: Option<SirenChannel>,
}

/// A mix snapshot channel: its static gain, placement and `1653b84c` (3 / 10 / 1 for the competitor / player /
/// roadblock siren channels; read as the channel's voice limit, inferred).
#[derive(Debug, Clone)]
pub struct SirenChannel {
    pub gain: f32,
    pub spatial: Spatial,
    pub voices: u32,
}

fn list(o: &Object, hash: u32) -> &[Value] {
    match o.get(hash) {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    }
}

fn handle_of(v: &Value) -> Option<ResourceId> {
    match v {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn handle_at(o: &Object, hash: u32) -> Option<ResourceId> {
    handle_of(o.get(hash)?)
}

/// The first wave of a struct list's `Wave` field (a handle or a list of handles).
fn waves(loader: &SoundLoader, o: &Object) -> Vec<Arc<Clip>> {
    match o.get(field_hash("Wave")) {
        Some(Value::Array(a)) => a.iter().filter_map(handle_of).filter_map(|id| loader.wave(id)).collect(),
        Some(v) => handle_of(v).and_then(|id| loader.wave(id)).into_iter().collect(),
        None => Vec::new(),
    }
}

impl AssetStore {
    fn siren_set(&self, loader: &SoundLoader, snapshot: &crate::car_sfx::Snapshot, id: ResourceId) -> Option<SirenSet> {
        let o = self.genesys_object(id).ok()?;
        if o.type_name != "Genesys.Gen.SirenSoundSet" {
            return None;
        }
        let structs = |name: &str| -> Vec<Object> {
            list(&o, field_hash(name))
                .iter()
                .filter_map(|v| match v {
                    Value::Struct(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        };
        let levels = structs("Sirens").iter().map(|l| waves(loader, l)).collect();
        let blips = structs("Blips").iter().flat_map(|b| waves(loader, b)).collect();
        let malfunctions = list(&o, field_hash("Malfunctions"))
            .iter()
            .filter_map(|v| match v {
                Value::Struct(s) => Some(waves(loader, s)),
                v => handle_of(v).and_then(|id| loader.wave(id)).map(|w| vec![w]),
            })
            .flatten()
            .collect();
        let channel = handle_at(&o, CHANNEL);
        let gain = channel
            .and_then(|c| self.genesys_object(c).ok())
            .and_then(|c| number(c.get(field_hash("Gain"))))
            .unwrap_or(1.0);
        let spatial = channel.and_then(|c| self.channel_spatial(c)).unwrap_or_default();
        let burst = handle_at(&o, SEQUENCE).and_then(|sid| {
            let s = self.genesys_object(sid).ok()?;
            let Some(Value::Array(items)) = s.get(field_hash("SequenceItems")) else { return None };
            items.iter().find_map(|it| {
                let Value::Struct(it) = it else { return None };
                let wave = list(it, field_hash("Waves")).iter().filter_map(handle_of).find_map(|w| loader.wave(w))?;
                let n = |o: &Object, name: &str| number(o.get(field_hash(name)));
                let sub = |name: &str| match it.get(field_hash(name)) {
                    Some(Value::Struct(s)) => Some(s.clone()),
                    _ => None,
                };
                Some(Burst {
                    id: sid,
                    wave,
                    gain: sub("Gain").and_then(|g| n(&g, "Value")).unwrap_or(1.0),
                    length: n(it, "EndTime").unwrap_or(0.0) - n(it, "StartTime").unwrap_or(0.0),
                    fade_out: sub("Fade_Out").and_then(|f| n(&f, "Time")).unwrap_or(0.0),
                })
            })
        });
        let dying = handle_at(&o, SEQUENCE).and_then(|sid| self.one_shot(snapshot, sid)).map(Arc::new);
        Some(SirenSet {
            id,
            dying,
            levels,
            blips,
            malfunctions,
            gain,
            spatial,
            burst,
            flag: number(o.get(FLAG)).unwrap_or(0.0) as u8,
        })
    }

    /// The `CopSoundSet` in `VEHICLES/WAVES.BNDL` with its siren sets.
    pub fn cop_sounds(&self) -> Option<CopSounds> {
        let b = self.bundle("VEHICLES/WAVES.BNDL")?;
        let set = b
            .entries_of_type(ResourceType::GenesysObject)
            .filter_map(|e| self.decode_genesys(&b.load(e).ok()?).ok())
            .find(|o| o.type_name == "Genesys.Gen.CopSoundSet")?;
        let loader = SoundLoader::new(self);
        let snapshot = crate::car_sfx::Snapshot::load(self);
        let sets = |hash: u32| -> Vec<Arc<SirenSet>> {
            list(&set, hash)
                .iter()
                .filter_map(handle_of)
                .filter_map(|id| self.siren_set(&loader, &snapshot, id))
                .map(Arc::new)
                .collect()
        };
        let channel = |name: &str| {
            let o = snapshot.channel(name)?;
            Some(SirenChannel {
                gain: snapshot.gain(name),
                spatial: snapshot.spatial(self, name)?,
                voices: number(o.get(0x1653_b84c)).unwrap_or(1.0) as u32,
            })
        };
        Some(CopSounds {
            sets_a: sets(SETS_A),
            sets_b: sets(SETS_B),
            competitor: channel("Sirens Competitor"),
            player: channel("Player Vehicle Sirens"),
            roadblock: channel("Sirens Roadblock"),
        })
    }
}

// --- The siren voice ---------------------------------------------------------------------------------------------

/// What a car's siren plays (the entity's siren state, `FUN_0058bdc0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SirenTone {
    #[default]
    Off,
    /// `Sirens[level − 1]` (level 1..=3, the entity's +0x186c; clamped to the set's count).
    Level(u8),
    /// Random `Blips`, one after another (the 0.8 s window after request 5).
    Blip,
}

impl SirenTone {
    pub fn code(self) -> u32 {
        match self {
            SirenTone::Off => 0,
            SirenTone::Level(l) => l.clamp(1, 3) as u32,
            SirenTone::Blip => 4,
        }
    }

    pub fn from_code(c: u32) -> Self {
        match c {
            1..=3 => SirenTone::Level(c as u8),
            4 => SirenTone::Blip,
            _ => SirenTone::Off,
        }
    }
}

/// The voice's gain (`FUN_0058bdc0` → `FUN_00571f30`: `.rdata` 0x00fad920 = 1.6, times the fade).
pub const SIREN_GAIN: f32 = 1.6;
/// A new wave fades in over this much of its play time (`FUN_00574510` state 2, `.bss` 0x0110d020 = 0.2,
/// read in `snap1`); the wave it replaces fades out by the same ramp.
pub const SIREN_FADE_IN: f32 = 0.2;
/// A stopped siren fades out over this long (`FUN_00571ef0` / `FUN_00574510` state 4, `.bss` 0x0110c940 = 0.15).
pub const SIREN_FADE_OUT: f32 = 0.15;
/// Request 5 (blip) holds for this long (`FUN_0058d770`, `.rdata` 0x00fac278).
pub const BLIP_SECONDS: f32 = 0.8;

struct SirenWave {
    clip: Arc<Clip>,
    pos: f64,
    step: f64,
    /// Seconds played.
    age: f32,
    tone: SirenTone,
}

impl SirenWave {
    fn new(clip: Arc<Clip>, tone: SirenTone, out_rate: u32) -> Self {
        let step = clip.rate as f64 / out_rate as f64;
        Self { clip, pos: 0.0, step, age: 0.0, tone }
    }

    /// The next sample, or `None` once a one-shot has ended.
    fn next(&mut self, out_rate: u32) -> Option<f32> {
        let n = self.clip.samples.len();
        if n == 0 {
            return None;
        }
        if self.pos >= n as f64 {
            if !self.clip.looping {
                return None;
            }
            self.pos -= n as f64;
        }
        let i = self.pos as usize;
        let f = (self.pos - i as f64) as f32;
        let a = self.clip.samples[i] as f32 / 32768.0;
        let b = if i + 1 < n {
            self.clip.samples[i + 1] as f32
        } else if self.clip.looping {
            self.clip.samples[0] as f32
        } else {
            0.0
        } / 32768.0;
        self.pos += self.step;
        self.age += 1.0 / out_rate as f32;
        Some(a + (b - a) * f)
    }
}

/// One car's siren voice (engine-free; `police_audio.rs` places it, `mwtool siren` renders it): the port of the
/// entity's siren object (`FUN_0058bdc0`, `FUN_00574510`, `FUN_00571f30`, `FUN_00572070`). It plays one wave at
/// a time in two slots: a level's tone `Sirens[level − 1].Wave[slot % count]`, or random blips while blipping;
/// a change starts the new wave at once with a 0.2 s fade-in while the old one fades by the same ramp, and
/// switching off fades the wave out over 0.15 s. A one-shot wave that ends is picked again while its tone is
/// still wanted (the game re-requests a wave whenever its pending slot is empty). Output is mono at `out_rate`,
/// × [`SIREN_GAIN`].
pub struct SirenPlayer {
    set: Arc<SirenSet>,
    slot: usize,
    out_rate: u32,
    rng: u64,
    cur: Option<SirenWave>,
    old: Option<(SirenWave, f32)>,
    /// Seconds left of the switch-off fade (state 4 without a new wave).
    stopping: f32,
}

impl SirenPlayer {
    pub fn new(set: Arc<SirenSet>, slot: usize, out_rate: u32) -> Self {
        Self { set, slot, out_rate, rng: 0x9e37_79b9_7f4a_7c15 ^ slot as u64, cur: None, old: None, stopping: 0.0 }
    }

    /// The game's LCG (`DAT_0110c690`: × 0x5851f42d4c957f2d + 1, high word).
    fn random(&mut self, n: usize) -> usize {
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        ((self.rng >> 32) as usize) % n.max(1)
    }

    fn pick(&mut self, tone: SirenTone) -> Option<Arc<Clip>> {
        match tone {
            SirenTone::Off => None,
            SirenTone::Blip => {
                let n = self.set.blips.len();
                if n == 0 {
                    return None;
                }
                let i = self.random(n);
                Some(self.set.blips[i].clone())
            }
            SirenTone::Level(l) => {
                let levels = &self.set.levels;
                let i = (l.max(1) as usize - 1).min(levels.len().checked_sub(1)?);
                let waves = &levels[i];
                (!waves.is_empty()).then(|| waves[self.slot % waves.len()].clone())
            }
        }
    }

    /// The tone the voice is playing (or fading in), for logs.
    pub fn playing(&self) -> SirenTone {
        self.cur.as_ref().map_or(SirenTone::Off, |w| w.tone)
    }

    pub fn next_sample(&mut self, want: SirenTone) -> f32 {
        let have = self.cur.as_ref().map(|w| w.tone);
        if have != Some(want) && !(want == SirenTone::Off && have.is_none()) {
            let next = self.pick(want).map(|c| SirenWave::new(c, want, self.out_rate));
            if let Some(cur) = self.cur.take() {
                let g = (cur.age / SIREN_FADE_IN).min(1.0);
                self.old = Some((cur, g));
                if next.is_none() {
                    self.stopping = SIREN_FADE_OUT;
                }
            }
            self.cur = next;
        }
        let mut out = 0.0;
        let mut fade_in = 1.0;
        let mut ended = false;
        if let Some(w) = &mut self.cur {
            fade_in = (w.age / SIREN_FADE_IN).min(1.0);
            match w.next(self.out_rate) {
                Some(s) => out += s * fade_in,
                None => ended = true,
            }
        }
        if ended {
            self.cur = None;
        }
        if let Some((w, g0)) = &mut self.old {
            let g = if self.cur.is_some() {
                *g0 * (1.0 - fade_in)
            } else {
                self.stopping -= 1.0 / self.out_rate as f32;
                *g0 * (self.stopping / SIREN_FADE_OUT).max(0.0)
            };
            match w.next(self.out_rate) {
                Some(s) if g > 0.0 => out += s * g,
                _ => self.old = None,
            }
        }
        out * SIREN_GAIN
    }
}
