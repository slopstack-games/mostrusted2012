//! One-off sounds of the world: what props, compound objects and their parts play when something hits them
//! (`WCPlaySoundBehaviour`), and the player car's landings and crash contacts (docs/decomp.md "World-component
//! sounds", "Landings"). Runtime: `crates/mostrusted/src/world_sfx.rs`.
//!
//! A `WCPlaySoundBehaviour` (GeneSys type 0x2AD8, 80 bytes) is read by the prop code (`FUN_0054bf70`,
//! `FUN_0054c680`; a `PropObject` names it at +0x1c) and by its runtime behaviour on compound objects, dynamic
//! objects and compound parts (vtable 0x00faf308: slot 8 `FUN_005ab070` for a body-pair contact, slot 9
//! `FUN_005b10f0` for the world contact). Its fields:
//!
//! | off  | field | use |
//! |------|-------|-----|
//! | 0x18 | `0228a2f8` (bool) | the behaviour reacts to contacts (true on all 41) |
//! | 0x19 | `713589ad` (bool) | a contact switches it off |
//! | 0x20 | `Offset` (vector) | added to the object's position for the voice (`FUN_005491a0`, `FUN_005aaf60`) |
//! | 0x30 | `MixerChannel` (handle) | the voices' channel; none: `World_Prop_Collision` for world contacts, the hitting car's `…_Prop_Collision` for car hits |
//! | 0x38 / 0x44 | `Collision_Waves` | a car hit (emitter type 5) |
//! | 0x3c / 0x46 | `8dfd5f40` | type 5 when three or more objects share the playing emitter (empty on all 41) |
//! | 0x40 / 0x48 | `Surface_Collisions` | `PropSurfaceSound` structs; only the first is read |
//!
//! A `PropSurfaceSound` (40 bytes): `e8986e0a` +0x14 (count +0x20, type 3: a prop's world contact while it moves
//! faster than 5 m/s), `71f062ee` +0x18 (count +0x22, type 2: a prop's first damaging car hit from a car faster
//! than 20 m/s), `Waves` +0x1c (count +0x24, type 4: a world contact).

use std::sync::Arc;

use mw_audio::sfx_player::{Bank, Clip, Fade, Item, ItemKind, ModValue, Sequence};
use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::car_sfx::{CUE, OneShot, Snapshot};
use crate::sfx_loader::{SoundLoader, handles, number};

/// `WCPlaySoundBehaviour` type id.
pub const PLAY_SOUND_BEHAVIOUR: u32 = 0x2ad8;

/// A decoded `WCPlaySoundBehaviour` (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct WcSound {
    pub object: ResourceId,
    pub label: String,
    /// +0x18 `0228a2f8`: contacts play it (the runtime behaviour's +0x16).
    pub on_collision: bool,
    /// +0x19 `713589ad`.
    pub off_on_collision: bool,
    /// +0x20 `Offset`, added to the object's position (not rotated).
    pub offset: [f32; 3],
    /// +0x30 `MixerChannel`.
    pub mixer_channel: Option<ResourceId>,
    /// +0x38 `Collision_Waves` (type 5) and +0x3c `8dfd5f40` (type 5 with three or more objects).
    pub collision_waves: Vec<ResourceId>,
    pub many_waves: Vec<ResourceId>,
    /// The first `Surface_Collisions` entry's lists: `e8986e0a` (type 3), `71f062ee` (type 2), `Waves` (type 4).
    pub slide_waves: Vec<ResourceId>,
    pub damage_waves: Vec<ResourceId>,
    pub world_waves: Vec<ResourceId>,
}

fn label(o: &Object) -> String {
    match o.get(field_hash("Label")) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn boolean(o: &Object, hash: u32) -> bool {
    matches!(o.get(hash), Some(Value::Bool(true)))
}

fn handle_at(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

impl WcSound {
    pub fn from_object(id: ResourceId, o: &Object) -> Self {
        let first = match o.get(field_hash("Surface_Collisions")) {
            Some(Value::Array(a)) => a.iter().find_map(|v| if let Value::Struct(s) = v { Some(s) } else { None }),
            _ => None,
        };
        let list = |s: Option<&Object>, hash: u32| s.map(|s| handles(s, hash)).unwrap_or_default();
        Self {
            object: id,
            label: label(o),
            on_collision: boolean(o, 0x0228_a2f8),
            off_on_collision: boolean(o, 0x7135_89ad),
            offset: match o.get(field_hash("Offset")) {
                Some(Value::Vector(v)) => [v[0], v[1], v[2]],
                _ => [0.0; 3],
            },
            mixer_channel: handle_at(o, field_hash("MixerChannel")),
            collision_waves: handles(o, field_hash("Collision_Waves")),
            many_waves: handles(o, 0x8dfd_5f40),
            slide_waves: list(first, 0xe898_6e0a),
            damage_waves: list(first, 0x71f0_62ee),
            world_waves: list(first, field_hash("Waves")),
        }
    }
}

/// The thresholds and timings from the global `VehicleSound` (`SOUND/GLOBAL.BNDL`, the object at
/// `[DAT_010edb08 + 4] + 0xb270`), with `.rdata` / `.bss` constants of the code that reads them.
#[derive(Debug, Clone, Copy)]
pub struct WorldSfxRules {
    /// +0x98 `392b1040` (2.235): a body-pair contact's value must exceed it (`FUN_0054bf70`, `FUN_005ab070`).
    pub pair_threshold: f32,
    /// +0x94 `0925bb50` (1): a world contact's (`FUN_0054c680`, `FUN_005b10f0`).
    pub world_threshold: f32,
    /// +0x88 `3453bdbb` (1 s) and +0x110 `567b6cc3` (0.1 s): the time in the air that picks a landing
    /// sequence (`FUN_00576b40`).
    pub long_air: f32,
    pub short_air: f32,
}

impl Default for WorldSfxRules {
    fn default() -> Self {
        Self { pair_threshold: 2.235, world_threshold: 1.0, long_air: 1.0, short_air: 0.1 }
    }
}

/// A prop's car-hit timer (`FUN_0054bf70`: +0xbc set to `.rdata` 0x00faaf30 = 0.6 s after every pair contact)
/// and world-contact timer (`FUN_0054c680`: +0xc0 set to `.rdata` 0x00faaf2c = 0.35 s), counted down per frame.
pub const PROP_PAIR_QUIET: f32 = 0.6;
pub const PROP_WORLD_QUIET: f32 = 0.35;
/// `.bss` 0x0110c420 = 400 (read in `snap2`): the hitter's squared speed above which a prop's first damaging hit
/// plays its type-2 list (`FUN_0054bf70`).
pub const DAMAGE_HIT_SPEED: f32 = 20.0;
/// `.bss` 0x0110c1a0 = 25: a prop's squared speed above which its world contact plays the type-3 list
/// (`FUN_0054c680`).
pub const SLIDE_SPEED: f32 = 5.0;
/// Landing wave gain (`FUN_00576b40`): `0.35 + 0.65 · min(v, 2.5) / 2.5` (`.bss` 0x0110cd80 = 0.35, 0x0110d0c0 =
/// 2.5, read in `snap2`).
pub const LANDING_MIN_GAIN: f32 = 0.35;
pub const LANDING_FULL_SPEED: f32 = 2.5;
/// A wheel is "on the ground" for this long after it last touched (`.rdata` 0x00fac2b4, entity +0x1b6c..).
pub const WHEEL_GROUND_HOLD: f32 = 0.05;

/// The sounds of one surface's `CollisionSoundSet` that the landing and crash code plays.
#[derive(Debug, Clone, Default)]
pub struct SurfaceCrashSfx {
    /// `Landing` (+0x124): a wheel touching down (not crashing), on `Landings_Large`, gain bound to
    /// [`crate::car_sfx::VOICE_GAIN`].
    pub landing: Vec<Arc<OneShot>>,
    /// +0x158 `5da2551f`: a wheel touching down while crashing, and crash zone 1 (the underside), on
    /// `Player_Wrecking_Collision`, gain bound to [`crate::car_sfx::VOICE_GAIN`].
    pub crash_landing: Vec<Arc<OneShot>>,
    /// Crash zones (`FUN_0057caf0` with `FUN_00929da0`): +0x15c `1db229d6` (roof), +0x118 `7e01f1e6` (front),
    /// +0x164 `1cb422d5` (sides), +0x154 `3578b364` (rear), on `Player_Wrecking_Collision`, gain bound to
    /// [`crate::car_sfx::VOICE_GAIN`] (the impact's speed gain).
    pub crash_roof: Vec<Arc<OneShot>>,
    pub crash_front: Vec<Arc<OneShot>>,
    pub crash_side: Vec<Arc<OneShot>>,
    pub crash_rear: Vec<Arc<OneShot>>,
    /// The player's landing sequences: +0x14 `d12772a1` (in the air ≥ [`WorldSfxRules::long_air`]) and +0xdc
    /// `929af217` (longer than [`WorldSfxRules::short_air`]).
    pub long_landing: Option<Arc<OneShot>>,
    pub short_landing: Option<Arc<OneShot>>,
}

/// Everything [`AssetStore::world_sfx`] loads.
#[derive(Debug, Clone, Default)]
pub struct WorldSfx {
    pub rules: WorldSfxRules,
    /// Per surface (the order of `Surfaces.Surfaces`): the index into [`Self::sets`].
    pub surface_set: Vec<Option<usize>>,
    /// The default surface's set index.
    pub default_set: Option<usize>,
    pub sets: Vec<SurfaceCrashSfx>,
    /// A race car's first contact with a prop (`FUN_00587960` → `FUN_00576440`): the MetalCar set's pools
    /// +0x130 `939c92be` (front), +0x134 `ae166d1e` (side), +0x138 `d02a49d8` (rear), by
    /// [`crate::car_sfx::Direction`], on the set's channel +0xc4 (`b26e4dfe`, `…7357a`), gain 1.
    pub prop_hit: [Vec<Arc<OneShot>>; 3],
    /// The same while the player crashes, once per crash: pool +0x150 `0960076c` on `Player_Wrecking_Collision`.
    pub crash_prop_hit: Vec<Arc<OneShot>>,
}

/// Where a wave plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SfxChannel<'a> {
    /// A channel of the mix snapshot by its name (`World Prop Collision`).
    Snapshot(&'a str),
    /// A `MixerChannel` GeneSys object.
    Mixer(ResourceId),
}

/// A builder of one-wave [`OneShot`]s (the snapshot and the decoded waves are kept).
pub struct WaveShots<'a> {
    store: &'a AssetStore,
    snapshot: Snapshot,
    loader: SoundLoader<'a>,
}

impl<'a> WaveShots<'a> {
    pub fn new(store: &'a AssetStore) -> Self {
        Self { store, snapshot: Snapshot::load(store), loader: SoundLoader::new(store) }
    }

    /// One wave on a channel: the channel's `Gain` scales it (× a gain bound to `gain` when given, 0..4 → 0..4),
    /// its falloff places it. `None` when the wave does not decode.
    pub fn shot(&self, wave: ResourceId, channel: SfxChannel, gain: Option<&str>) -> Option<OneShot> {
        let clip = self.loader.wave(wave)?;
        let (channel_gain, spatial) = match channel {
            SfxChannel::Snapshot(name) => (
                self.snapshot.channel(name).and_then(|c| number(c.get(field_hash("Gain")))).unwrap_or(1.0),
                self.snapshot.spatial(self.store, name),
            ),
            SfxChannel::Mixer(id) => (
                self.store.genesys_object(id).ok().and_then(|c| number(c.get(field_hash("Gain")))).unwrap_or(1.0),
                self.store.channel_spatial(id),
            ),
        };
        Some(OneShot {
            id: wave.0,
            bank: wave_bank(vec![clip], channel_gain, gain),
            spatial: spatial.unwrap_or_default(),
        })
    }

    /// A whole sequence as a [`OneShot`] (its items' channels applied).
    pub fn sequence(&self, id: ResourceId) -> Option<OneShot> {
        self.store.one_shot(&self.snapshot, id)
    }
}

/// A one-cue bank playing one of `waves` once.
fn wave_bank(waves: Vec<Arc<Clip>>, channel_gain: f32, gain: Option<&str>) -> Bank {
    let end = waves.iter().map(|w| w.samples.len() as f32 / w.rate.max(1) as f32).fold(0.0, f32::max);
    let mut g = ModValue::constant(channel_gain);
    if let Some(b) = gain {
        g.binding = b.to_string();
        g.range = (0.0, 4.0);
        g.out = (0.0, 4.0 * channel_gain);
    }
    let item = Item {
        start: 0.0,
        end,
        fade_in: Fade::default(),
        fade_out: Fade::default(),
        enabled: String::new(),
        channel: None,
        kind: ItemKind::Wave { waves, gain: g, pitch: ModValue::constant(1.0), probability: None },
    };
    let seq = Sequence {
        id: 0,
        binding: String::new(),
        min: 0.0,
        max: 0.0,
        items: vec![item],
        gate: String::new(),
        retrigger: false,
        jumps: Vec::new(),
    };
    Bank { channels: Vec::new(), sequences: vec![seq], cues: vec![(CUE.to_string(), vec![0])], master_gain: 1.0 }
}

/// `CollisionSoundSet` fields read here.
const LANDING: u32 = 0x70a9_e657;
const CRASH_LANDING: u32 = 0x5da2_551f;
const CRASH_ROOF: u32 = 0x1db2_29d6;
const CRASH_FRONT: u32 = 0x7e01_f1e6;
const CRASH_SIDE: u32 = 0x1cb4_22d5;
const CRASH_REAR: u32 = 0x3578_b364;
const LONG_LANDING: u32 = 0xd127_72a1;
const SHORT_LANDING: u32 = 0x929a_f217;

impl AssetStore {
    /// The `WCPlaySoundBehaviour`s among an object's (or compound part's) behaviours.
    pub fn wc_sounds(&self, behaviours: &[mw_formats::Behaviour]) -> Vec<WcSound> {
        behaviours
            .iter()
            .filter(|b| b.type_id == PLAY_SOUND_BEHAVIOUR)
            .filter_map(|b| b.object)
            .filter_map(|id| self.wc_sound(id))
            .collect()
    }

    /// One `WCPlaySoundBehaviour` by id.
    pub fn wc_sound(&self, id: ResourceId) -> Option<WcSound> {
        let o = self.genesys_object(id).ok()?;
        Some(WcSound::from_object(id, &o))
    }

    /// The landing and crash sounds of every surface, and the rules ([`WorldSfx`]).
    pub fn world_sfx(&self) -> WorldSfx {
        let shots = WaveShots::new(self);
        let mut out = WorldSfx::default();
        if let Some(b) = self.bundle("SOUND/GLOBAL.BNDL") {
            let vs = b
                .entries_of_type(ResourceType::GenesysObject)
                .filter_map(|e| self.decode_genesys(&b.load(e).ok()?).ok())
                .find(|o| o.type_name == "Genesys.Gen.VehicleSound");
            if let Some(vs) = vs {
                let f = |h: u32, d: f32| number(vs.get(h)).unwrap_or(d);
                let d = WorldSfxRules::default();
                out.rules = WorldSfxRules {
                    pair_threshold: f(0x392b_1040, d.pair_threshold),
                    world_threshold: f(0x0925_bb50, d.world_threshold),
                    long_air: f(0x3453_bdbb, d.long_air),
                    short_air: f(0x567b_6cc3, d.short_air),
                };
            }
        }
        let surfaces = self.surfaces().unwrap_or_default();
        let mut ids: Vec<ResourceId> = Vec::new();
        for s in &surfaces {
            let k = s.collision_sounds.map(|id| match ids.iter().position(|&x| x == id) {
                Some(k) => k,
                None => {
                    ids.push(id);
                    ids.len() - 1
                }
            });
            out.surface_set.push(k);
        }
        let default = self
            .bundle("HAWAII/SURFACELIST.BNDL")
            .and_then(|b| {
                b.entries_of_type(ResourceType::GenesysObject)
                    .filter_map(|e| self.decode_genesys(&b.load(e).ok()?).ok())
                    .find(|o| o.type_name == "Genesys.Gen.Surfaces")
            })
            .and_then(|s| handle_at(&s, field_hash("Default_surface")))
            .and_then(|id| self.genesys_object(id).ok())
            .and_then(|st| handle_at(&st, field_hash("Collision_Sounds")));
        if let Some(d) = default {
            out.default_set = Some(match ids.iter().position(|&x| x == d) {
                Some(k) => k,
                None => {
                    ids.push(d);
                    ids.len() - 1
                }
            });
        }
        const WRECK: SfxChannel = SfxChannel::Snapshot("Player Wrecking Collision");
        let metal = self.bundle("SOUND/METALCAR.BNDL").and_then(|b| {
            b.entries_of_type(ResourceType::GenesysObject)
                .filter_map(|e| self.decode_genesys(&b.load(e).ok()?).ok())
                .find(|o| o.type_name == "Genesys.Gen.CollisionSoundSet")
        });
        if let Some(set) = &metal {
            let pool = |hash: u32, channel: SfxChannel| -> Vec<Arc<OneShot>> {
                handles(set, hash).into_iter().filter_map(|w| shots.shot(w, channel, None).map(Arc::new)).collect()
            };
            if let Some(ch) = handle_at(set, 0xb26e_4dfe) {
                let ch = SfxChannel::Mixer(ch);
                out.prop_hit = [pool(0x939c_92be, ch), pool(0xae16_6d1e, ch), pool(0xd02a_49d8, ch)];
            }
            out.crash_prop_hit = pool(0x0960_076c, WRECK);
        }
        for id in ids {
            let Ok(set) = self.genesys_object(id) else {
                out.sets.push(SurfaceCrashSfx::default());
                continue;
            };
            let pool = |hash: u32, channel: SfxChannel, gain: Option<&str>| {
                handles(&set, hash).into_iter().filter_map(|w| shots.shot(w, channel, gain).map(Arc::new)).collect()
            };
            let seq = |hash: u32| handle_at(&set, hash).and_then(|s| shots.sequence(s)).map(Arc::new);
            let impact = Some(crate::car_sfx::VOICE_GAIN);
            out.sets.push(SurfaceCrashSfx {
                landing: pool(LANDING, SfxChannel::Snapshot("Landings Large"), Some(crate::car_sfx::VOICE_GAIN)),
                crash_landing: pool(CRASH_LANDING, WRECK, Some(crate::car_sfx::VOICE_GAIN)),
                crash_roof: pool(CRASH_ROOF, WRECK, impact),
                crash_front: pool(CRASH_FRONT, WRECK, impact),
                crash_side: pool(CRASH_SIDE, WRECK, impact),
                crash_rear: pool(CRASH_REAR, WRECK, impact),
                long_landing: seq(LONG_LANDING),
                short_landing: seq(SHORT_LANDING),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_field_is_named() {
        assert_eq!(field_hash("Landing"), LANDING);
    }
}
