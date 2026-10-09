//! Crashes: when an impact counts as a crash (`Physics.CrashingRules`), the crash camera (a feedback
//! `Sequence` of slow-motion, bystander-camera, HUD and letterbox items) and the rolling reset that follows
//! (`GameLogicConfig`). What the game's code does with these is in docs/decomp.md ("Crashes").
//!
//! Layouts (field hashes; names where cracked, see genesys_names.txt):
//!
//! | type | field | meaning |
//! |------|-------|---------|
//! | `Physics.CrashingRules` (76 B) | ten `ImpactRules` arrays at 0x10–0x34, their `u16` counts at 0x38–0x4a | one rule set per impact kind; `WorldRules` (0x34) for static scenery |
//! | `ImpactRules` (36 B) | `1617a710` float[] (0x0c) | crash when the impact's damage exceeds it (`FUN_009a9fd0`, strictly greater) |
//! | | `2414aa68` float[] (0x10) | crash when the closing speed in mph exceeds it |
//! | | `d6c0e908` float[] (0x14) | a third threshold, empty in every instance |
//! | | `DamageOverride` `DamageToDeal`[] (0x18) | damage dealt instead (`DamageToDeal`, `1b41f6fc`) when the rule fires |
//! | `DamageBarProfile` (120 B) | `CrashingRules` handle (0x34) | the rules a car's damage bar uses; `DamageBarProfiles.PlayerOffline` is the player's |
//! | `Sequence` (44 B) | `SequenceItems` (polymorphic), `StartTime` / `EndTime` per item | real-time timeline |
//! | `SlowMoSequenceItem` | `ae525972` `ModulatingDoubleValue` | the simulation's time scale while the item runs |
//! | `CameraSequenceItem` | `Cameras` [`CameraSequenceItem.Camera`] | `Bystander_Params` handle per camera |
//! | `BystanderCameraParams` (148 B) | `Camera_Transform_Binding`, `Target_Transform_Binding`, `X/Y/Z_Offset`, `Target_X/Y/Z_Offset`, `Tracking_Params`, `Zoom_Params` | a bystander shot |
//! | `SimpleSceneAnalyser` | `Offset.Translation` / `.Rotation`, `Name` (hash) | the space a `SAS.Analyser["…"]` binding names |
//!
//! Units: thresholds in mph and damage-bar units, offsets in metres (car space: +X left, +Y up, +Z
//! forward), FOVs in degrees, times in seconds of real time (the sequence clock is not slowed by its own
//! slow motion: at the data's 0.14 / 0.22 the shots would otherwise last 10 s each).

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};

pub use crate::sequence::{Bystander, CurvePoint, Letterbox, Modulated, SceneAnalyser, Tracking, Zoom};
use crate::{AssetStore, Error, Result};

mod h {
    use mw_formats::genesys::field_hash;

    // DamageBarProfiles / DamageBarProfile
    pub const PLAYER_OFFLINE: u32 = field_hash("PlayerOffline");
    pub const CRASHING_RULES: u32 = field_hash("CrashingRules");
    // CrashingRules
    pub const WORLD_RULES: u32 = field_hash("WorldRules");
    pub const TRAFFIC_RULES: u32 = field_hash("TrafficRules");
    pub const LANDING_RULES: u32 = field_hash("LandingRules");
    pub const PLAYER_RULES: u32 = field_hash("PlayerRules");
    pub const ROADBLOCK_RULES: u32 = field_hash("RoadblockRules");
    /// Unnamed rule sets: the AI / cop cars (`FUN_008fbb90` picks 0x10 for a `RaceCarPhysicsObject` cop),
    /// then two more read by the prop and payload impacts.
    pub const RULES_10: u32 = 0x2206_1460;
    pub const RULES_14: u32 = 0x1dfd_d45e;
    pub const RULES_18: u32 = 0x5ccf_9942;
    pub const RULES_20: u32 = 0x2897_efad;
    pub const RULES_28: u32 = 0x3e26_fbca;
    // ImpactRules
    pub const DAMAGE: u32 = 0x1617_a710;
    pub const CLOSING_MPH: u32 = 0x2414_aa68;
    pub const THIRD: u32 = 0xd6c0_e908;
    pub const DAMAGE_OVERRIDE: u32 = field_hash("DamageOverride");
    pub const DAMAGE_TO_DEAL: u32 = field_hash("DamageToDeal");
    pub const DAMAGE_OVERRIDE_FLAG: u32 = 0x1b41_f6fc;
    // FeedbackMessageGroup / Sequence
    pub const SEQUENCES2: u32 = field_hash("Sequences2");
    pub const SEQUENCE_ITEMS: u32 = field_hash("SequenceItems");
    pub const START_TIME: u32 = field_hash("StartTime");
    pub const END_TIME: u32 = field_hash("EndTime");
    pub const SLOW_MO_VALUE: u32 = 0xae52_5972;
    pub const CAMERAS: u32 = field_hash("Cameras");
    pub const BYSTANDER_PARAMS: u32 = field_hash("Bystander_Params");
    pub const SCENE_ANALYSIS: u32 = field_hash("SceneAnalysis");
    pub const LAYOUT: u32 = field_hash("Layout");
    // SceneAnalysis
    pub const ANALYSERS: u32 = field_hash("Analysers");
    pub const OFFSET_STRUCT: u32 = field_hash("Offset");
    pub const NAME: u32 = field_hash("Name");
    pub const TRANSLATION: u32 = field_hash("Translation");
    pub const ROTATION: u32 = field_hash("Rotation");
    // GameLogicConfig: the rolling reset (FUN_006d0180 reads 0x220 / 0x224 / 0x24c)
    pub const RESET_MIN_MPH: u32 = 0x75d8_2dd5;
    pub const RESET_SPEED_SCALE: u32 = 0xa44e_b2f3;
    pub const RESET_CRASH_MAX_MPH: u32 = 0x7452_1df4;
    // Car against car (docs/decomp.md "Crashes", "Car against car")
    pub const PROFILE_AI: u32 = 0x2ae1_7c10;
    pub const PROFILE_TRAFFIC: u32 = field_hash("Traffic");
    pub const PROFILE_LOCATION_SCALE: u32 = 0x9950_c913;
    pub const LOCATION_SCALES: [u32; 7] =
        [0x4450_ab62, 0x9249_ac64, 0x4387_7a60, 0xffa2_91a8, 0xbc4f_adb9, 0x459f_6c6c, 0xde41_1a20];
    pub const GRAPH_GENERIC: u32 = 0x97aa_b742;
    pub const GRAPH_RACE_CARS: u32 = 0x3fd6_9cce;
    pub const GRAPH_REFERENCE_MPH: u32 = 0x674e_a963;
    /// `ImpactDamageGraphs.Graph`: (high speed, damage), (mid speed, damage), (low speed, damage).
    pub const GRAPH_POINTS: [u32; 6] = [0x64df_1ca1, 0xf107_cd5c, 0xa465_7c04, 0xa77b_7fdd, 0x5deb_cbe0, 0xaeea_c48a];
    pub const STATS_AI: u32 = 0xac7a_3b27;
    pub const STATS_OFFLINE: u32 = 0x8570_ce40;
    pub const STATS_ONLINE: u32 = 0x304e_4b58;
    pub const STATS: [u32; 6] = [0xbf2e_83db, 0x45f8_d01f, 0x07c6_b471, 0x4fbd_9ab4, 0x04df_1a2e, 0x074a_9349];
    pub const STRENGTH: u32 = field_hash("Strength");
}

/// Where the data lives (resolved by type within these bundles, not by fixed resource IDs).
const PHYSICS_CONFIG: &str = "PHYSICS/PHYSICSCONFIG.BNDL";
const GLOBAL_CONFIG: &str = "GLOBALCONFIG.BNDL";
/// The feedback message group whose second sequence (`Sequences2[1]`) is the crash camera: the only one
/// with slow motion and bystander cameras that is about five seconds long (docs/decomp.md, "Crashes").
const FEEDBACK_GROUP: &str = "EN_US/FEEDBACKGROUPS/457707.BNDL";
pub const CRASH_SEQUENCE_INDEX: usize = 1;

/// One `ImpactRules` entry: a crash happens when any present threshold is exceeded.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImpactRule {
    /// `1617a710`: impact damage (damage-bar units) above which the impact is a crash.
    pub damage: Option<f32>,
    /// `2414aa68`: closing speed (mph) above which the impact is a crash.
    pub closing_mph: Option<f32>,
    /// `d6c0e908`: third threshold (empty everywhere).
    pub third: Option<f32>,
    /// `DamageOverride`: damage to deal instead when the rule fires, and its flag (`1b41f6fc`).
    pub damage_override: Option<(f32, bool)>,
}

impl ImpactRule {
    /// The game's test (`FUN_009a9fd0`): each threshold that is present must be exceeded strictly.
    pub fn is_crash(&self, damage: Option<f32>, closing_mph: f32) -> bool {
        let over = |limit: Option<f32>, v: Option<f32>| matches!((limit, v), (Some(l), Some(v)) if v > l);
        over(self.damage, damage) || over(self.closing_mph, Some(closing_mph))
    }
}

/// `Physics.CrashingRules`.
#[derive(Debug, Clone, Default)]
pub struct CrashingRules {
    pub id: ResourceId,
    pub world: Option<ImpactRule>,
    pub traffic: Option<ImpactRule>,
    pub landing: Option<ImpactRule>,
    pub player: Option<ImpactRule>,
    pub roadblock: Option<ImpactRule>,
    /// The unnamed sets, by field hash.
    pub other: Vec<(u32, ImpactRule)>,
}

impl CrashingRules {
    /// `22061460` (+0x10): "AI impact", against a race car that is not a player's (`FUN_009575e0`,
    /// `FUN_009588c0`, `FUN_0095a680`).
    pub fn ai(&self) -> Option<ImpactRule> {
        self.other_rule(h::RULES_10)
    }

    /// `5ccf9942` (+0x18): "crashing traffic impact", against a traffic car that has crashed (`FUN_00959be0`).
    pub fn crashing_traffic(&self) -> Option<ImpactRule> {
        self.other_rule(h::RULES_18)
    }

    fn other_rule(&self, hash: u32) -> Option<ImpactRule> {
        self.other.iter().find(|(h, _)| *h == hash).map(|(_, r)| *r)
    }
}

/// One `ImpactDamageGraphs.Graph` (`FUN_009250d0`): damage against impact speed, linear from (low mph, low
/// damage) to (mid mph, mid damage) with the speed floored at the low point, then on through (high mph, high
/// damage) without a cap. Speeds in mph, damage in damage-bar units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageGraph {
    pub high: [f32; 2],
    pub mid: [f32; 2],
    pub low: [f32; 2],
}

impl DamageGraph {
    /// Damage for an impact speed in m/s (`FUN_009250d0`; the game's mph → m/s factor is 0.44704).
    pub fn damage(&self, speed: f32) -> f32 {
        let (hi, mid, lo) = (self.high[0] * MPH, self.mid[0] * MPH, self.low[0] * MPH);
        if speed < mid {
            let t = ((speed - lo) / (mid - lo).max(1.0)).max(0.0);
            self.low[1] + t * (self.mid[1] - self.low[1])
        } else {
            let t = (speed - mid) / (hi - mid).max(1.0);
            self.mid[1] + t * (self.high[1] - self.mid[1])
        }
    }

    fn read(o: &Object) -> Option<Self> {
        let [a, b, c, d, e, f] = h::GRAPH_POINTS.map(|hash| float(o, hash));
        Some(Self { high: [a?, b?], mid: [c?, d?], low: [e?, f?] })
    }
}

/// `ImpactDamageGraphs` (PHYSICS/PHYSICSCONFIG, `CollisionInfo.Damage_Graphs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactDamageGraphs {
    /// `97aab742`: every car-against-car impact except race car against race car (`FUN_00941a50`).
    pub generic: DamageGraph,
    /// `3fd69cce`: race car against race car (`FUN_0093c240`).
    pub race_cars: DamageGraph,
    /// `674ea963` (+0x58, mph): below this speed the damage scales down with the faster car's speed.
    pub reference_mph: f32,
}

/// `Vehicle_Gameplay.DamageStats` as `FUN_008ffb20` copies it onto the vehicle (+0x15f0…+0x1608): all at least
/// 1, the strength at least 0. Field names are unhashed; they are named after the vehicle offsets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageStats {
    /// `074a9349` → +0x15f0.
    pub s15f0: f32,
    /// `04df1a2e` → +0x15f4: the defence against traffic (and traffic's against traffic).
    pub s15f4: f32,
    /// `bf2e83db` → +0x15f8: the defence against AI / race cars (player against AI, AI against AI).
    pub s15f8: f32,
    /// `4fbd9ab4` → +0x15fc: the defence in player-against-player hits; traffic's against the player.
    pub s15fc: f32,
    /// `45f8d01f` → +0x1600.
    pub s1600: f32,
    /// `07c6b471` → +0x1604: the defence used when the other car's component flag `05cd1d74` is set.
    pub s1604: f32,
    /// `Strength` → +0x1608: the attack.
    pub strength: f32,
}

impl Default for DamageStats {
    fn default() -> Self {
        Self { s15f0: 50.0, s15f4: 50.0, s15f8: 50.0, s15fc: 50.0, s1600: 50.0, s1604: 50.0, strength: 50.0 }
    }
}

impl DamageStats {
    fn read(o: &Object) -> Option<Self> {
        let [s15f8, s1600, s1604, s15fc, s15f4, s15f0] = h::STATS.map(|hash| float(o, hash).unwrap_or(1.0).max(1.0));
        Some(Self { s15f0, s15f4, s15f8, s15fc, s1600, s1604, strength: float(o, h::STRENGTH)?.max(0.0) })
    }
}

/// The three `DamageStats` of a `Vehicle_Gameplay` (`FUN_008ffb20` picks one: +0x44 for an online player,
/// +0xc for a car whose driver kind +0x30e0 is 2 (AI; read in a snapshot), +0x28 otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VehicleDamageStats {
    pub ai: DamageStats,
    pub offline: DamageStats,
    pub online: DamageStats,
}

impl VehicleDamageStats {
    /// Read from a `Genesys.Gen.Vehicle_Gameplay` object.
    pub fn from_gameplay(o: &Object) -> Option<Self> {
        let get = |hash| sub(o, hash).and_then(DamageStats::read);
        Some(Self { ai: get(h::STATS_AI)?, offline: get(h::STATS_OFFLINE)?, online: get(h::STATS_ONLINE)? })
    }
}

/// A `Physics.DamageBarProfile`: the impact-location damage scales and the crashing rules.
#[derive(Debug, Clone, Default)]
pub struct DamageProfile {
    pub id: ResourceId,
    /// `ImpactLocationDamageScale` (+0x18…+0x30), in field order.
    pub location_scale: [f32; 7],
    pub rules: CrashingRules,
}

impl DamageProfile {
    /// The scale for an impact location (`FUN_0092ebf0`; locations from `FUN_00929da0`, see `crash.rs`): 1 →
    /// +0x18, front corners 2 / 4 → +0x1c, front 3 → +0x20, rear corners 6 / 8 → +0x24, rear 7 → +0x28, sides
    /// 5 / 9 → +0x2c, roof 0 → +0x30, the centre (10) 1.
    pub fn location_scale(&self, location: u8) -> f32 {
        match location {
            1 => self.location_scale[0],
            2 | 4 => self.location_scale[1],
            3 => self.location_scale[2],
            6 | 8 => self.location_scale[3],
            7 => self.location_scale[4],
            5 | 9 => self.location_scale[5],
            0 => self.location_scale[6],
            _ => 1.0,
        }
    }
}

/// What the car-against-car impacts need (`FUN_00960f20`'s handlers).
#[derive(Debug, Clone)]
pub struct CarImpactData {
    pub graphs: ImpactDamageGraphs,
    /// `DamageBarProfiles.PlayerOffline`, `2ae17c10` (the AI cars' profile: a snapshot's AI car, driver kind 2,
    /// held its location scales 1.125 / 1.65) and `Traffic`.
    pub player: DamageProfile,
    pub ai: DamageProfile,
    pub traffic: DamageProfile,
}

/// The game's mph → m/s factor (`.bss` 0x0118a010, read in a snapshot).
const MPH: f32 = 0.44704;

/// `SlowMoSequenceItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlowMo {
    pub start: f32,
    pub end: f32,
    pub scale: Modulated,
}

/// One `CameraSequenceItem` with its (single, in the crash sequence) bystander camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub start: f32,
    pub end: f32,
    pub camera: Bystander,
}

/// The crash camera sequence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CrashCamera {
    pub id: ResourceId,
    pub slow_mo: Vec<SlowMo>,
    pub shots: Vec<Shot>,
    /// `HideHudSequenceItem`.
    pub hide_hud: Option<(f32, f32)>,
    pub letterbox: Option<Letterbox>,
    /// Last item end (the audio items run to 5.5 s).
    pub length: f32,
}

impl CrashCamera {
    /// The simulation's time scale at sequence time `t` (1 outside the slow-motion items).
    pub fn time_scale(&self, t: f32) -> f32 {
        self.slow_mo
            .iter()
            .find(|s| t >= s.start && t < s.end)
            .map_or(1.0, |s| s.scale.at((t - s.start) / (s.end - s.start).max(1e-6)))
    }

    /// The shot at sequence time `t`, if any (else the gameplay camera shows).
    pub fn shot_at(&self, t: f32) -> Option<(usize, &Shot)> {
        self.shots.iter().enumerate().find(|(_, s)| t >= s.start && t < s.end)
    }

    /// When the last camera item ends: the gameplay camera is back from here.
    pub fn cameras_end(&self) -> f32 {
        self.shots.iter().map(|s| s.end).fold(0.0, f32::max)
    }
}

/// `GameLogicConfig`'s rolling reset after a crash (`FUN_006d0180`): speed
/// `min(max(min_mph, scale × pre-crash mph), crash_max_mph)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResetSpeed {
    pub min_mph: f32,
    pub scale: f32,
    pub crash_max_mph: f32,
}

impl ResetSpeed {
    pub fn after_crash(&self, pre_crash_mph: f32) -> f32 {
        (self.scale * pre_crash_mph.abs()).max(self.min_mph).min(self.crash_max_mph)
    }
}

/// Everything the crash code needs.
#[derive(Debug, Clone)]
pub struct CrashData {
    /// The offline player's rules (`DamageBarProfiles.PlayerOffline`).
    pub rules: CrashingRules,
    pub camera: CrashCamera,
    pub reset: ResetSpeed,
    /// Car-against-car impacts (`None` when the data is missing: those impacts never crash then).
    pub impacts: Option<CarImpactData>,
}

fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::UInt(u) => Some(*u as f32),
        _ => None,
    }
}

fn float(o: &Object, hash: u32) -> Option<f32> {
    num(o.get(hash)?)
}

fn boolean(o: &Object, hash: u32) -> Option<bool> {
    match o.get(hash)? {
        Value::Bool(b) => Some(*b),
        v => num(v).map(|f| f != 0.0),
    }
}

fn sub(o: &Object, hash: u32) -> Option<&Object> {
    match o.get(hash)? {
        Value::Struct(s) => Some(s),
        _ => None,
    }
}

fn structs(o: &Object, hash: u32) -> Vec<&Object> {
    match o.get(hash) {
        Some(Value::Array(a)) => {
            a.iter().filter_map(|v| if let Value::Struct(s) = v { Some(s) } else { None }).collect()
        }
        Some(Value::Struct(s)) => vec![s],
        _ => Vec::new(),
    }
}

fn handle(v: Option<&Value>) -> Option<ResourceId> {
    match v? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn first_float(o: &Object, hash: u32) -> Option<f32> {
    match o.get(hash)? {
        Value::Array(a) => a.first().and_then(num),
        v => num(v),
    }
}

fn vec3(v: Option<&Value>) -> Option<[f32; 3]> {
    match v? {
        Value::Vector(v) => Some([v[0], v[1], v[2]]),
        Value::Array(a) if a.len() >= 3 => Some([num(&a[0])?, num(&a[1])?, num(&a[2])?]),
        _ => None,
    }
}

fn times(o: &Object) -> (f32, f32) {
    (float(o, h::START_TIME).unwrap_or(0.0), float(o, h::END_TIME).unwrap_or(0.0))
}

fn is(o: &Object, short: &str) -> bool {
    o.type_name.rsplit('.').next() == Some(short)
}

impl ImpactRule {
    fn read(o: &Object) -> Self {
        let over = structs(o, h::DAMAGE_OVERRIDE).first().map(|d| {
            (float(d, h::DAMAGE_TO_DEAL).unwrap_or(0.0), boolean(d, h::DAMAGE_OVERRIDE_FLAG).unwrap_or(false))
        });
        Self {
            damage: first_float(o, h::DAMAGE),
            closing_mph: first_float(o, h::CLOSING_MPH),
            third: first_float(o, h::THIRD),
            damage_override: over,
        }
    }
}

impl CrashingRules {
    /// Read a `Genesys.Gen.Physics.CrashingRules` object.
    pub fn from_object(id: ResourceId, o: &Object) -> Self {
        let rule = |hash| structs(o, hash).first().map(|r| ImpactRule::read(r));
        Self {
            id,
            world: rule(h::WORLD_RULES),
            traffic: rule(h::TRAFFIC_RULES),
            landing: rule(h::LANDING_RULES),
            player: rule(h::PLAYER_RULES),
            roadblock: rule(h::ROADBLOCK_RULES),
            other: [h::RULES_10, h::RULES_14, h::RULES_18, h::RULES_20, h::RULES_28]
                .into_iter()
                .filter_map(|hash| rule(hash).map(|r| (hash, r)))
                .collect(),
        }
    }
}

/// The analyser a `SAS.Analyser["<name>"]...` binding names.
fn analyser_name(binding: &str) -> Option<&str> {
    let rest = binding.split("Analyser[\"").nth(1)?;
    rest.split('"').next()
}

impl AssetStore {
    /// GeneSys objects of one type in a bundle.
    fn genesys_of_type(&self, bundle: &str, type_name: &str) -> Result<Vec<(ResourceId, Object)>> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        let mut out = Vec::new();
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            if let Ok(o) = self.decode_genesys(&b.load(e)?)
                && o.type_name == type_name
            {
                out.push((e.id, o));
            }
        }
        Ok(out)
    }

    /// The offline player's crashing rules: `DamageBarProfiles.PlayerOffline` → `CrashingRules`.
    pub fn player_crashing_rules(&self) -> Result<CrashingRules> {
        let (_, profiles) = self
            .genesys_of_type(PHYSICS_CONFIG, "Genesys.Gen.DamageBarProfiles")?
            .into_iter()
            .next()
            .ok_or(Error::Missing(ResourceId::NULL))?;
        let profile_id = handle(profiles.get(h::PLAYER_OFFLINE)).ok_or(Error::Missing(ResourceId::NULL))?;
        let profile = self.genesys_object(profile_id)?;
        let rules_id = handle(profile.get(h::CRASHING_RULES)).ok_or(Error::Missing(profile_id))?;
        Ok(CrashingRules::from_object(rules_id, &self.genesys_object(rules_id)?))
    }

    /// The crash camera sequence (`Sequences2[1]` of feedback group 457707).
    pub fn crash_camera(&self) -> Result<CrashCamera> {
        let (_, group) = self
            .genesys_of_type(FEEDBACK_GROUP, "Genesys.Gen.FeedbackMessageGroup")?
            .into_iter()
            .next()
            .ok_or(Error::Missing(ResourceId::NULL))?;
        let id = match group.get(h::SEQUENCES2) {
            Some(Value::Array(a)) => handle(a.get(CRASH_SEQUENCE_INDEX)),
            _ => None,
        }
        .ok_or(Error::Missing(ResourceId::NULL))?;
        self.read_crash_camera(id)
    }

    /// Read a `Genesys.Gen.Sequence` as a crash camera.
    pub fn read_crash_camera(&self, id: ResourceId) -> Result<CrashCamera> {
        let seq = self.genesys_object(id)?;
        let mut cam = CrashCamera { id, ..Default::default() };
        let items = structs(&seq, h::SEQUENCE_ITEMS);
        // The scene analysis the sequence switches on (its analysers are what `SAS.Analyser[...]` names).
        let analysis = items
            .iter()
            .find(|i| is(i, "SceneAnalysisSequenceItem"))
            .and_then(|i| handle(i.get(h::SCENE_ANALYSIS)))
            .and_then(|a| self.genesys_object(a).ok());
        for item in &items {
            let (start, end) = times(item);
            cam.length = cam.length.max(end);
            if is(item, "SlowMoSequenceItem") {
                if let Some(v) = sub(item, h::SLOW_MO_VALUE) {
                    cam.slow_mo.push(SlowMo { start, end, scale: Modulated::read(v) });
                }
            } else if is(item, "CameraSequenceItem") {
                for c in structs(item, h::CAMERAS) {
                    let Some(bid) = handle(c.get(h::BYSTANDER_PARAMS)) else { continue };
                    let b = self.genesys_object(bid)?;
                    let mut camera = Bystander::read(bid, &b);
                    if let (Some(name), Some(analysis)) = (analyser_name(&camera.camera_binding), &analysis) {
                        let want = field_hash(name);
                        camera.analyser = structs(analysis, h::ANALYSERS)
                            .into_iter()
                            .find(|a| name_of(a) == Some(want))
                            .map(|a| SceneAnalyser {
                                name_hash: want,
                                translation: sub(a, h::OFFSET_STRUCT)
                                    .and_then(|o| vec3(o.get(h::TRANSLATION)))
                                    .unwrap_or_default(),
                                rotation: sub(a, h::OFFSET_STRUCT)
                                    .and_then(|o| vec3(o.get(h::ROTATION)))
                                    .unwrap_or_default(),
                            });
                    }
                    cam.shots.push(Shot { start, end, camera });
                }
            } else if is(item, "HideHudSequenceItem") {
                cam.hide_hud = Some((start, end));
            } else if is(item, "LayoutSequenceItem")
                && cam.letterbox.is_none()
                && let Some(layout) = handle(item.get(h::LAYOUT)).and_then(|l| self.genesys_object(l).ok())
            {
                cam.letterbox = Letterbox::read(start, end, &layout);
            }
        }
        cam.shots.sort_by(|a, b| a.start.total_cmp(&b.start));
        cam.slow_mo.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(cam)
    }

    /// `GameLogicConfig`'s rolling-reset speeds.
    pub fn crash_reset_speed(&self) -> Result<ResetSpeed> {
        let (_, cfg) = self
            .genesys_of_type(GLOBAL_CONFIG, "Genesys.Gen.GameLogicConfig")?
            .into_iter()
            .next()
            .ok_or(Error::Missing(ResourceId::NULL))?;
        let f = |hash| float(&cfg, hash).ok_or(Error::Missing(ResourceId::NULL));
        Ok(ResetSpeed {
            min_mph: f(h::RESET_MIN_MPH)?,
            scale: f(h::RESET_SPEED_SCALE)?,
            crash_max_mph: f(h::RESET_CRASH_MAX_MPH)?,
        })
    }

    /// A `DamageBarProfiles` entry with its location scales and rules.
    fn damage_profile(&self, profiles: &Object, field: u32) -> Result<DamageProfile> {
        let id = handle(profiles.get(field)).ok_or(Error::Missing(ResourceId::NULL))?;
        let profile = self.genesys_object(id)?;
        let mut location_scale = [1.0; 7];
        if let Some(s) = sub(&profile, h::PROFILE_LOCATION_SCALE) {
            for (v, hash) in location_scale.iter_mut().zip(h::LOCATION_SCALES) {
                *v = float(s, hash).unwrap_or(1.0);
            }
        }
        let rules_id = handle(profile.get(h::CRASHING_RULES)).ok_or(Error::Missing(id))?;
        let rules = CrashingRules::from_object(rules_id, &self.genesys_object(rules_id)?);
        Ok(DamageProfile { id, location_scale, rules })
    }

    /// The damage graphs and the player / AI / traffic damage profiles for car-against-car impacts.
    pub fn car_impact_data(&self) -> Result<CarImpactData> {
        let (_, profiles) = self
            .genesys_of_type(PHYSICS_CONFIG, "Genesys.Gen.DamageBarProfiles")?
            .into_iter()
            .next()
            .ok_or(Error::Missing(ResourceId::NULL))?;
        let (_, graphs) = self
            .genesys_of_type(PHYSICS_CONFIG, "Genesys.Gen.ImpactDamageGraphs")?
            .into_iter()
            .next()
            .ok_or(Error::Missing(ResourceId::NULL))?;
        let graph = |hash| sub(&graphs, hash).and_then(DamageGraph::read).ok_or(Error::Missing(ResourceId::NULL));
        Ok(CarImpactData {
            graphs: ImpactDamageGraphs {
                generic: graph(h::GRAPH_GENERIC)?,
                race_cars: graph(h::GRAPH_RACE_CARS)?,
                reference_mph: float(&graphs, h::GRAPH_REFERENCE_MPH).unwrap_or(25.0),
            },
            player: self.damage_profile(&profiles, h::PLAYER_OFFLINE)?,
            ai: self.damage_profile(&profiles, h::PROFILE_AI)?,
            traffic: self.damage_profile(&profiles, h::PROFILE_TRAFFIC)?,
        })
    }

    /// Rules, crash camera and reset speeds together.
    pub fn crash_data(&self) -> Result<CrashData> {
        Ok(CrashData {
            rules: self.player_crashing_rules()?,
            camera: self.crash_camera()?,
            reset: self.crash_reset_speed()?,
            impacts: self.car_impact_data().ok(),
        })
    }
}

/// A scene analyser's `Name` (a `u32` hash).
fn name_of(o: &Object) -> Option<u32> {
    match o.get(h::NAME)? {
        Value::UInt(u) => Some(*u as u32),
        Value::Int(i) => Some(*i as u32),
        _ => None,
    }
}
