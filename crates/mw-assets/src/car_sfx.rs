//! The other cars' one-shot sounds: traffic horns, collision sounds of traffic and AI racers, and the traffic
//! skid loop (docs/decomp.md "Traffic horns, crashes and skids"). Each is a [`OneShot`]: a one-cue
//! `mw_audio::sfx_player` bank plus how its first item's channel places it in the world.
//!
//! | sound | data | picked by |
//! |---|---|---|
//! | horn | `<component>_COMPETITORSOUND` (`VehicleSound` +0x60 / +0x64), `horns[car slot % count]` | `FUN_006042c0` |
//! | traffic car hits a car | `SOUND/METALCAR.BNDL` `CollisionSoundSet` slot 12 (`333a28fc`, +0x6c) | `FUN_006042c0` |
//! | AI racer impact | the same set's competitor slots by level × direction (table below) | `FUN_0057e920` |
//! | world impact | the surface's `Collision_Sounds` set, wave pool `7e01f1e6` (+0x118) on `Competitor_World_Collision`, gain by speed | `FUN_0057caf0` |
//! | traffic skid | the surface's `SurfaceSounds.6bdefb10` (+0x54): a 4 s loop timeline (jump 4 → 1 s) | `FUN_0060d290` |
//!
//! Competitor slots of `CollisionSoundSet` (`FUN_0057e920`, `param_12` = 0; the player's set is the other half):
//! light 0x6c / 0x74 / 0x7c, medium 0x54 / 0x5c / 0x64, heavy 0x34 / 0x3c / 0x44, for a contact in front of the
//! car (within 30° of its forward axis, `FUN_00570b60` with cos 0.866 read live at `.bss` 0x0110d0f0), behind it,
//! or on its side.
//!
//! The player's car ([`PlayerCarSfx`], docs/decomp.md "The player's horn, impacts and scrapes"): its horn
//! (`<id>_VEHICLESOUND` +0x60 / +0x64 on `Player Vehicle Horn`), its impacts by the same table's player half
//! (`FUN_0057e920`, `param_12` = 1) and the world-impact wave on `Player Vehicle World Collision`, the grind
//! against other vehicles (`FUN_0057f2e0`: pool `97b6a045` (+0x140) on `Player Vehicle Scraping`, `Sparks`
//! (+0x14c) on `Player Vehicle Debris`) and the damage sounds (`FUN_00576220`: pool `1ae351a5` (+0x11c) on
//! `Player Vehicle Glass Smash`, `Scratch` (+0x148) on `Player Vehicle Debris`).
//!
//! Items without a `Mixer_Channel` name a channel of the mix snapshot (`Snapshot_Property`, e.g. "Competitor
//! Vehicle Collision Small" → the `Competitor_Vehicle_Collision_Small` channel of `Mix_Main`, `…72a0`, reached
//! from `MiscSound.20c447e8` → `MixSnapshotSet`); its gain scales the item and its falloff places the voice.

use std::collections::HashMap;
use std::sync::Arc;

use mw_audio::sfx_player::{Bank, Fade, Item, ItemKind, ModValue, Sequence};
use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

use crate::AssetStore;
use crate::car_sound::Spatial;
use crate::sfx_loader::{SoundLoader, handle, handles, number};

/// The cue every [`OneShot`] bank plays.
pub const CUE: &str = "play";
/// The world-impact wave's gain (`FUN_0057caf0`'s `local_90`, set before the cue).
pub const IMPACT_GAIN: &str = "Custom.ImpactGain";

/// A sound started by an event, placed at a car.
#[derive(Debug, Clone)]
pub struct OneShot {
    /// The sequence's resource id (0 for a pool assembled here).
    pub id: u64,
    pub bank: Bank,
    pub spatial: Spatial,
}

/// Thresholds from the global `VehicleSound` (`SOUND/GLOBAL.BNDL`; the race-car sound object's +0x5c points at
/// it, and `FUN_006042c0` reads the same object through `world+0xb270`). Speeds in m/s (inferred: 2.235 is
/// 5 mph).
#[derive(Debug, Clone, Copy)]
pub struct CollisionRules {
    /// `392b1040` (+0x98, 2.235): a traffic car's contact, and any world impact, must close faster than this.
    pub threshold: f32,
    /// `edc5149f` (+0xcc, 30) / `ad43123c` (+0xfc, 14): race car against race car, heavy above / medium from.
    pub car_heavy: f32,
    pub car_medium: f32,
    /// `89672c4c` (+0xc8, 30) / `945a7617` (+0xf8, 14): a race car's world impact, medium above / light above.
    pub world_medium: f32,
    pub world_light: f32,
    /// `567b6cc3` (+0x110, 0.1 s): a race car's world impacts are this far apart at least.
    pub cooldown: f32,
}

impl Default for CollisionRules {
    fn default() -> Self {
        Self {
            threshold: 2.235,
            car_heavy: 30.0,
            car_medium: 14.0,
            world_medium: 30.0,
            world_light: 14.0,
            cooldown: 0.1,
        }
    }
}

/// Impact level (`FUN_0057e920`'s `param_8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Light = 0,
    Medium = 1,
    Heavy = 2,
}

/// Where the contact is relative to the car (`FUN_00570b60`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Front = 0,
    Side = 1,
    Rear = 2,
}

/// `FUN_00570b60`: the contact direction's cosine with the car's forward axis above 0.866 is the front, below
/// −0.866 the rear, else the side.
pub fn direction(cos_forward: f32) -> Direction {
    const COS_30: f32 = 0.866_025_4;
    if cos_forward > COS_30 {
        Direction::Front
    } else if cos_forward < -COS_30 {
        Direction::Rear
    } else {
        Direction::Side
    }
}

impl CollisionRules {
    /// Race car against race car (`FUN_00591ac0`): `s` = the larger of the two cars' impact values.
    pub fn car_level(&self, s: f32) -> Level {
        if s > self.car_heavy {
            Level::Heavy
        } else if s >= self.car_medium {
            Level::Medium
        } else {
            Level::Light
        }
    }

    /// A race car's world impact (`FUN_00591ac0` after `FUN_0057caf0`): no collision-set sound up to
    /// `world_light` (only the surface's wave), light up to `world_medium`, medium above.
    pub fn world_level(&self, s: f32) -> Option<Level> {
        if s > self.world_medium {
            Some(Level::Medium)
        } else if s > self.world_light {
            Some(Level::Light)
        } else {
            None
        }
    }
}

/// `FUN_0057caf0`'s gain of a world-impact wave from the car's speed `v` (m/s): 0.714 at 30 mph and below,
/// rising linearly to 1 at 100 mph (`.data` 0x010c4730 = 0.44704, `.rdata` 0x00faba74 = 30, 0x00faba78 = 100,
/// 0x00fae1c8 = 0.286, 0x00faba7c = 0.714). Read as the wave's gain (it is the last argument of the emitter's
/// play call, `vtable+0x50`).
pub fn world_impact_gain(speed: f32) -> f32 {
    let mph = 0.447_04;
    let t = ((speed - mph * 30.0) / (mph * 100.0 - mph * 30.0)).clamp(0.0, 1.0);
    t * 0.286 + 0.714
}

/// Everything the other cars' one-shots need.
#[derive(Debug, Clone, Default)]
pub struct OtherCarSfx {
    pub rules: CollisionRules,
    /// MetalCar competitor slots by `[level][direction]`.
    pub impacts: [[Option<Arc<OneShot>>; 3]; 3],
    /// The default surface's world-impact wave pool, gain bound to [`IMPACT_GAIN`].
    pub world_impact: Option<Arc<OneShot>>,
    /// The traffic skid loop (default surface).
    pub skid: Option<Arc<OneShot>>,
    /// Distinct horn sequences.
    pub horns: Vec<Arc<OneShot>>,
    /// Per traffic `VehicleComponent` id: its horn list (indices into `horns`, in the resource's order).
    pub horn_lists: HashMap<u32, Vec<usize>>,
    /// The car alarm (`MiscSound.c1109b6e`, an 8 s timeline; `FUN_006042c0`).
    pub alarm: Option<Arc<OneShot>>,
    /// `MiscSound` +0xe0 (`7f18169d`, 0.4): a car sounds its alarm when `(car index % 10) × 0.1` is at most this.
    pub alarm_share: f32,
}

/// `CollisionSoundSet` competitor slots by `[level][direction]` (field hashes; offsets in the module doc).
const COMPETITOR_SLOTS: [[u32; 3]; 3] = [
    [0x333a_28fc, 0x33e4_a9d8, 0x59b8_870e],
    [0xc630_8362, 0xa334_be48, 0xc968_909e],
    [0x3aec_f1ae, 0x156f_dafa, 0x7f33_f42c],
];
/// `CollisionSoundSet` player slots by `[level][direction]` (`FUN_0057e920` with `param_12` = 1): light +0xd4 /
/// +0xe4 / +0xec, medium +0x9c / +0xa4 / +0xac, heavy +0x0c / +0x1c / +0x24 (front / rear / side, here in
/// [`Direction`]'s order: front, side, rear).
const PLAYER_SLOTS: [[u32; 3]; 3] = [
    [0x077e_73d9, 0x5566_ce08, 0x3f3a_e0de],
    [0x455d_bd2e, 0x9770_e56d, 0xfd2c_cbbb],
    [0xc90e_8812, 0xb377_9198, 0xd92b_bf4e],
];
/// `CollisionSoundSet` +0x108: a player's heavy impact adds one wave of this pool.
const HEAVY_POOL: u32 = 0xa8f4_4b4f;
/// `CollisionSoundSet` +0x140: the grind's waves (`FUN_00576a20` case 0).
const GRIND_POOL: u32 = 0x97b6_a045;
/// `CollisionSoundSet` +0x11c: the damage grid's threshold sounds (`FUN_00576220`).
const GLASS_POOL: u32 = 0x1ae3_51a5;

/// Bindings of a player voice's gain and pitch (set before the cue, and every frame for the grind).
pub const VOICE_GAIN: &str = "Custom.VoiceGain";
pub const VOICE_PITCH: &str = "Custom.VoicePitch";

/// The player car's contact-sound rules: `VehicleSound` fields (`SOUND/GLOBAL.BNDL`; `FUN_00594870` copies them
/// into the per-frame block `FUN_00591ac0` passes on) and code constants.
#[derive(Debug, Clone, Copy)]
pub struct PlayerSfxRules {
    /// `.bss` 0x0110cac0 (0.05, read in `snap2`): a contact's value must reach this to grind (`FUN_0057f2e0`).
    pub grind_threshold: f32,
    /// `e40a7d46` (+0xd4, 0.278 s): a grind wave this close to its end hands over to the next one.
    pub grind_crossfade: f32,
    /// `bf4ff152` (+0xd8, 1): the grind's pitch is `R^(2x − 1)` clamped to [1/R, R], x = speed / top speed.
    pub grind_pitch_range: f32,
    /// `.bss` 0x0110ce70 (1.09, read in `snap2`): the grind's gain is `1.09^(−t)`, t = seconds since it started.
    pub grind_decay: f32,
    /// `567b6cc3` (+0x110, 0.1 s): the grind fades out over this after its last contact; also the sparks' and
    /// the world impacts' spacing.
    pub fade: f32,
    /// `.rdata` 0x00fac2b4 (0.05 s): the release of a stopped grind voice (`FUN_00aba570`).
    pub release: f32,
    /// `.rdata` 0x00fac27c (0.3): the sparks' pitch is `1 + 0.3 u`, u uniform in [0, 1) (`FUN_00575750`).
    pub sparks_pitch: f32,
    /// `b0407c0b` (+0xc0, 0.121): a damage grid cell's threshold flag rises by more than this.
    pub glass_threshold: f32,
    /// `f5ddbc19` (+0xe0, 0.106): a damage grid cell's scratch level rises by more than this.
    pub scratch_threshold: f32,
}

impl Default for PlayerSfxRules {
    fn default() -> Self {
        Self {
            grind_threshold: 0.05,
            grind_crossfade: 0.278,
            grind_pitch_range: 1.0,
            grind_decay: 1.09,
            fade: 0.1,
            release: 0.05,
            sparks_pitch: 0.3,
            glass_threshold: 0.121,
            scratch_threshold: 0.106,
        }
    }
}

/// The player car's contact sounds ([`AssetStore::player_car_sfx`]).
#[derive(Debug, Clone, Default)]
pub struct PlayerCarSfx {
    pub rules: PlayerSfxRules,
    /// The impact thresholds (the same global `VehicleSound` as the other cars').
    pub collision: CollisionRules,
    /// MetalCar player slots by `[level][direction]`.
    pub impacts: [[Option<Arc<OneShot>>; 3]; 3],
    /// MetalCar +0x108 waves.
    pub heavy: Vec<Arc<OneShot>>,
    /// The default surface's world-impact waves (pool `7e01f1e6`) on `Player Vehicle World Collision`, gain
    /// bound to [`IMPACT_GAIN`]. (`FUN_0057caf0` also starts the set's player slot +0xf4 for the player: in every
    /// surface set that is `…0b3935`, a `RaceCarEffectSequenceItem` timeline of race-car properties 1 and 3,
    /// no sound; not ported.)
    pub world_impact: Vec<Arc<OneShot>>,
    /// Grind waves (+0x140), gain [`VOICE_GAIN`] and pitch [`VOICE_PITCH`] bound.
    pub grind: Vec<Arc<OneShot>>,
    /// `Sparks` (+0x14c) waves, pitch [`VOICE_PITCH`] bound.
    pub sparks: Vec<Arc<OneShot>>,
    /// +0x11c waves and `Scratch` (+0x148) waves, gain [`VOICE_GAIN`] bound.
    pub glass: Vec<Arc<OneShot>>,
    pub scratch: Vec<Arc<OneShot>>,
}

impl OneShot {
    /// The latest item end of the bank's timelines (s): how long a one-wave shot plays.
    pub fn duration(&self) -> f32 {
        self.bank.sequences.iter().flat_map(|s| &s.items).map(|i| i.end).fold(0.0, f32::max)
    }
}

/// `SurfaceType.Collision_Sounds` and the world-impact pool of a `CollisionSoundSet` (+0x118).
const WORLD_IMPACT_POOL: u32 = 0x7e01_f1e6;
/// `SurfaceSounds` +0x54: the traffic skid timeline.
const TRAFFIC_SKID: u32 = 0x6bde_fb10;
/// Dry conditions of a `SurfaceType` (as `sfx.rs`).
const SURFACE_DRY: u32 = 0xf395_b019;
/// `MiscSound.20c447e8`: the `MixSnapshotSet`.
const SNAPSHOT_SET: u32 = 0x20c4_47e8;

fn first_object(store: &AssetStore, bundle: &str, type_name: &str) -> Option<Object> {
    let b = store.bundle(bundle)?;
    b.entries_of_type(ResourceType::GenesysObject)
        .filter_map(|e| store.decode_genesys(&b.load(e).ok()?).ok())
        .find(|o| o.type_name == type_name)
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn handle_at(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

/// [`CollisionRules`] from the global `VehicleSound`.
fn collision_rules(vs: &Object) -> CollisionRules {
    let f = |h: u32, d: f32| number(vs.get(h)).unwrap_or(d);
    let d = CollisionRules::default();
    CollisionRules {
        threshold: f(0x392b_1040, d.threshold),
        car_heavy: f(0xedc5_149f, d.car_heavy),
        car_medium: f(0xad43_123c, d.car_medium),
        world_medium: f(0x8967_2c4c, d.world_medium),
        world_light: f(0x945a_7617, d.world_light),
        cooldown: f(0x567b_6cc3, d.cooldown),
    }
}

/// The mix snapshot's channels by name hash (`field_hash` of the name with underscores).
struct Snapshot {
    channels: HashMap<u32, Object>,
}

impl Snapshot {
    fn load(store: &AssetStore) -> Self {
        let channels = (|| {
            let misc = first_object(store, "SOUND/GLOBAL.BNDL", "Genesys.Gen.MiscSound")?;
            let set = store.genesys_object(handle_at(&misc, SNAPSHOT_SET)?).ok()?;
            // `External_Camera` (the bonnet camera names the same snapshot).
            let snap = store.genesys_object(handle(&set, "External_Camera")?).ok()?;
            Some(
                snap.fields
                    .iter()
                    .filter_map(|(h, v)| match v {
                        Value::Struct(o) if o.type_name.ends_with("MixerChannel") => Some((*h, o.clone())),
                        _ => None,
                    })
                    .collect(),
            )
        })()
        .unwrap_or_default();
        Self { channels }
    }

    fn channel(&self, property: &str) -> Option<&Object> {
        self.channels.get(&field_hash(&property.replace(' ', "_")))
    }

    fn gains(&self) -> HashMap<u32, f32> {
        self.channels.iter().map(|(h, o)| (*h, number(o.get(field_hash("Gain"))).unwrap_or(1.0))).collect()
    }

    fn spatial(&self, store: &AssetStore, property: &str) -> Option<Spatial> {
        let ch = self.channel(property)?;
        let falloff = handle_at(ch, field_hash("DistanceFalloff")).and_then(|f| store.genesys_object(f).ok());
        Some(Spatial::from_channel(ch, falloff.as_ref()))
    }
}

impl AssetStore {
    /// One sequence as a [`OneShot`]: its items' snapshot channels applied, placed by its first wave item's
    /// channel (a `Mixer_Channel`, else the snapshot channel it names).
    fn one_shot(&self, snapshot: &Snapshot, id: ResourceId) -> Option<OneShot> {
        let mut loader = SoundLoader::new(self).with_snapshot(snapshot.gains());
        let seq = loader.sequence(id)?;
        if seq.items.is_empty() {
            return None;
        }
        let o = self.genesys_object(id).ok()?;
        let spatial = match o.get(field_hash("SequenceItems")) {
            Some(Value::Array(items)) => items.iter().find_map(|it| {
                let Value::Struct(it) = it else { return None };
                if let Some(ch) = handle(it, "Mixer_Channel") {
                    return self.channel_spatial(ch);
                }
                match it.get(field_hash("Snapshot_Property")) {
                    Some(Value::Str(p)) if !p.is_empty() => snapshot.spatial(self, p),
                    _ => None,
                }
            }),
            _ => None,
        };
        let bank = Bank {
            channels: loader.channels(),
            sequences: vec![seq],
            cues: vec![(CUE.to_string(), vec![0])],
            master_gain: 1.0,
        };
        Some(OneShot { id: id.0, bank, spatial: spatial.unwrap_or_default() })
    }

    /// The other cars' one-shot sounds. `traffic_components` are the traffic vehicle types' `VehicleComponent`
    /// ids (their horn lists).
    pub fn other_car_sfx(&self, traffic_components: &[u32]) -> OtherCarSfx {
        let snapshot = Snapshot::load(self);
        let mut out = OtherCarSfx::default();
        if let Some(vs) = first_object(self, "SOUND/GLOBAL.BNDL", "Genesys.Gen.VehicleSound") {
            out.rules = collision_rules(&vs);
        }
        if let Some(set) = first_object(self, "SOUND/METALCAR.BNDL", "Genesys.Gen.CollisionSoundSet") {
            for (level, slots) in COMPETITOR_SLOTS.iter().enumerate() {
                for (dir, &h) in slots.iter().enumerate() {
                    out.impacts[level][dir] =
                        handle_at(&set, h).and_then(|id| self.one_shot(&snapshot, id)).map(Arc::new);
                }
            }
        }
        // The default surface: its collision set and its tyre sounds.
        let surface = first_object(self, "HAWAII/SURFACELIST.BNDL", "Genesys.Gen.Surfaces")
            .and_then(|s| handle(&s, "Default_surface"))
            .and_then(|id| self.genesys_object(id).ok());
        if let Some(st) = &surface {
            out.world_impact = handle(st, "Collision_Sounds")
                .and_then(|id| self.genesys_object(id).ok())
                .and_then(|set| self.world_impact(&snapshot, &set))
                .map(Arc::new);
            out.skid = sub(st, SURFACE_DRY)
                .and_then(|c| sub(c, field_hash("Normal")))
                .and_then(|n| handle(n, "Surface_Sounds"))
                .and_then(|id| self.genesys_object(id).ok())
                .and_then(|ss| handle_at(&ss, TRAFFIC_SKID))
                .and_then(|id| self.one_shot(&snapshot, id))
                .map(Arc::new);
        }
        let mut index: HashMap<u64, usize> = HashMap::new();
        for &c in traffic_components {
            let Some(vs) = self.vehicle_sound(&format!("{c}_COMPETITORSOUND")) else { continue };
            let mut list = Vec::new();
            for h in vs.horns {
                let k = match index.get(&h.0) {
                    Some(&k) => k,
                    None => {
                        let Some(o) = self.one_shot(&snapshot, h) else { continue };
                        out.horns.push(Arc::new(o));
                        index.insert(h.0, out.horns.len() - 1);
                        out.horns.len() - 1
                    }
                };
                list.push(k);
            }
            out.horn_lists.insert(c, list);
        }
        if let Some(misc) = first_object(self, "SOUND/GLOBAL.BNDL", "Genesys.Gen.MiscSound") {
            out.alarm = handle_at(&misc, 0xc110_9b6e).and_then(|id| self.one_shot(&snapshot, id)).map(Arc::new);
            out.alarm_share = number(misc.get(0x7f18_169d)).unwrap_or(0.0);
        }
        out
    }

    /// The player car's horn: the first of `<vehicle>_VEHICLESOUND`'s horn sequences (+0x60 / +0x64; the 911's
    /// is one 5 s wave item on the snapshot channel `Player Vehicle Horn`).
    pub fn player_horn(&self, vehicle: u32) -> Option<OneShot> {
        let vs = self.vehicle_sound(&format!("{vehicle}_VEHICLESOUND"))?;
        let snapshot = Snapshot::load(self);
        vs.horns.iter().find_map(|&h| self.one_shot(&snapshot, h))
    }

    /// The player car's impact, grind and damage sounds ([`PlayerCarSfx`]).
    pub fn player_car_sfx(&self) -> PlayerCarSfx {
        let snapshot = Snapshot::load(self);
        let loader = SoundLoader::new(self);
        let mut out = PlayerCarSfx::default();
        if let Some(vs) = first_object(self, "SOUND/GLOBAL.BNDL", "Genesys.Gen.VehicleSound") {
            out.collision = collision_rules(&vs);
            let f = |h: u32, d: f32| number(vs.get(h)).unwrap_or(d);
            let d = PlayerSfxRules::default();
            out.rules = PlayerSfxRules {
                grind_crossfade: f(0xe40a_7d46, d.grind_crossfade),
                grind_pitch_range: f(0xbf4f_f152, d.grind_pitch_range),
                fade: f(0x567b_6cc3, d.fade),
                glass_threshold: f(0xb040_7c0b, d.glass_threshold),
                scratch_threshold: f(0xf5dd_bc19, d.scratch_threshold),
                ..d
            };
        }
        let pool = |set: &Object, hash: u32, channel: &str, gain: Option<&str>, pitch: Option<&str>| {
            handles(set, hash)
                .into_iter()
                .filter_map(|w| loader.wave(w))
                .filter_map(|w| self.wave_shot(&snapshot, vec![w], channel, gain, pitch).map(Arc::new))
                .collect::<Vec<_>>()
        };
        if let Some(set) = first_object(self, "SOUND/METALCAR.BNDL", "Genesys.Gen.CollisionSoundSet") {
            for (level, slots) in PLAYER_SLOTS.iter().enumerate() {
                for (dir, &h) in slots.iter().enumerate() {
                    out.impacts[level][dir] =
                        handle_at(&set, h).and_then(|id| self.one_shot(&snapshot, id)).map(Arc::new);
                }
            }
            out.heavy = pool(&set, HEAVY_POOL, "Player Vehicle Collision Big", None, None);
            out.grind = pool(&set, GRIND_POOL, "Player Vehicle Scraping", Some(VOICE_GAIN), Some(VOICE_PITCH));
            out.sparks = pool(&set, field_hash("Sparks"), "Player Vehicle Debris", None, Some(VOICE_PITCH));
            out.glass = pool(&set, GLASS_POOL, "Player Vehicle Glass Smash", Some(VOICE_GAIN), None);
            out.scratch = pool(&set, field_hash("Scratch"), "Player Vehicle Debris", Some(VOICE_GAIN), None);
        }
        let surface = first_object(self, "HAWAII/SURFACELIST.BNDL", "Genesys.Gen.Surfaces")
            .and_then(|s| handle(&s, "Default_surface"))
            .and_then(|id| self.genesys_object(id).ok());
        let set =
            surface.as_ref().and_then(|st| handle(st, "Collision_Sounds")).and_then(|id| self.genesys_object(id).ok());
        if let Some(set) = set {
            out.world_impact = pool(&set, WORLD_IMPACT_POOL, "Player Vehicle World Collision", Some(IMPACT_GAIN), None);
        }
        out
    }

    /// One wave as a [`OneShot`] on a snapshot channel (its gain scales the wave, its falloff places it), played
    /// once; `gain` / `pitch` name bindings mapping 0..4 onto 0..4 (× the channel's gain for the gain), else 1.
    fn wave_shot(
        &self,
        snapshot: &Snapshot,
        waves: Vec<Arc<mw_audio::sfx_player::Clip>>,
        channel: &str,
        gain: Option<&str>,
        pitch: Option<&str>,
    ) -> Option<OneShot> {
        if waves.is_empty() {
            return None;
        }
        let channel_gain = snapshot.channel(channel).and_then(|c| number(c.get(field_hash("Gain")))).unwrap_or(1.0);
        let bound = |name: Option<&str>, scale: f32| {
            let mut v = ModValue::constant(scale);
            if let Some(b) = name {
                v.binding = b.to_string();
                v.range = (0.0, 4.0);
                v.out = (0.0, 4.0 * scale);
            }
            v
        };
        let end = waves.iter().map(|w| w.samples.len() as f32 / w.rate.max(1) as f32).fold(0.0, f32::max);
        let item = Item {
            start: 0.0,
            end,
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            enabled: String::new(),
            channel: None,
            kind: ItemKind::Wave {
                waves,
                gain: bound(gain, channel_gain),
                pitch: bound(pitch, 1.0),
                probability: None,
            },
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
        let bank = Bank {
            channels: Vec::new(),
            sequences: vec![seq],
            cues: vec![(CUE.to_string(), vec![0])],
            master_gain: 1.0,
        };
        Some(OneShot { id: 0, bank, spatial: snapshot.spatial(self, channel).unwrap_or_default() })
    }

    /// `FUN_0057caf0`'s world-impact sound: one wave of the set's pool `7e01f1e6` on the snapshot channel
    /// `Competitor_World_Collision` (the traffic channel set `world+0x12a00` equals the competitors' `+0x12950`
    /// in `snap2`; its +0x60 is that channel), gain [`IMPACT_GAIN`] × the channel's gain.
    fn world_impact(&self, snapshot: &Snapshot, set: &Object) -> Option<OneShot> {
        let loader = SoundLoader::new(self);
        let waves: Vec<_> = handles(set, WORLD_IMPACT_POOL).into_iter().filter_map(|w| loader.wave(w)).collect();
        if waves.is_empty() {
            return None;
        }
        const CHANNEL: &str = "Competitor World Collision";
        let channel_gain = snapshot.channel(CHANNEL).and_then(|c| number(c.get(field_hash("Gain")))).unwrap_or(1.0);
        let mut gain = ModValue::constant(1.0);
        gain.binding = IMPACT_GAIN.to_string();
        gain.range = (0.0, 1.0);
        gain.out = (0.0, channel_gain);
        let end = waves.iter().map(|w| w.samples.len() as f32 / w.rate.max(1) as f32).fold(0.0, f32::max);
        let item = Item {
            start: 0.0,
            end,
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            enabled: String::new(),
            channel: None,
            kind: ItemKind::Wave { waves, gain, pitch: ModValue::constant(1.0), probability: None },
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
        let bank = Bank {
            channels: Vec::new(),
            sequences: vec![seq],
            cues: vec![(CUE.to_string(), vec![0])],
            master_gain: 1.0,
        };
        Some(OneShot { id: 0, bank, spatial: snapshot.spatial(self, CHANNEL).unwrap_or_default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_directions() {
        let r = CollisionRules::default();
        assert_eq!(r.car_level(5.0), Level::Light);
        assert_eq!(r.car_level(14.0), Level::Medium);
        assert_eq!(r.car_level(30.0), Level::Medium);
        assert_eq!(r.car_level(30.5), Level::Heavy);
        assert_eq!(r.world_level(14.0), None);
        assert_eq!(r.world_level(20.0), Some(Level::Light));
        assert_eq!(r.world_level(31.0), Some(Level::Medium));
        assert_eq!(direction(0.9), Direction::Front);
        assert_eq!(direction(0.0), Direction::Side);
        assert_eq!(direction(-0.9), Direction::Rear);
        assert!((world_impact_gain(0.0) - 0.714).abs() < 1e-6);
        assert!((world_impact_gain(100.0 * 0.447_04) - 1.0).abs() < 1e-6);
    }
}
