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
    // ModulatingDoubleValue
    pub const VALUE: u32 = field_hash("Value");
    pub const OUTPUT_MIN: u32 = field_hash("Output_Value_Min");
    pub const OUTPUT_MAX: u32 = field_hash("Output_Value_Max");
    pub const ANIM_POINTS: u32 = field_hash("AnimPoints");
    pub const POSITION_X: u32 = field_hash("PositionX");
    pub const POSITION_Y: u32 = field_hash("PositionY");
    pub const TANGENT_IN_X: u32 = field_hash("TangentInX");
    pub const TANGENT_IN_Y: u32 = field_hash("TangentInY");
    pub const TANGENT_OUT_X: u32 = field_hash("TangentOutX");
    pub const TANGENT_OUT_Y: u32 = field_hash("TangentOutY");
    // BystanderCameraParams
    pub const CAMERA_BINDING: u32 = field_hash("Camera_Transform_Binding");
    pub const TARGET_BINDING: u32 = field_hash("Target_Transform_Binding");
    pub const OFFSET: [u32; 3] = [field_hash("X_Offset"), field_hash("Y_Offset"), field_hash("Z_Offset")];
    pub const TARGET_OFFSET: [u32; 3] =
        [field_hash("Target_X_Offset"), field_hash("Target_Y_Offset"), field_hash("Target_Z_Offset")];
    pub const UNKNOWN_86: u32 = 0x8698_0811;
    pub const UNKNOWN_CC: u32 = 0xcc83_12fe;
    pub const UNKNOWN_FLAG: u32 = 0x005c_8fc7;
    pub const TRACKING: u32 = field_hash("Tracking_Params");
    pub const SCREEN_OFFSET: [u32; 2] = [field_hash("Screen_Offset_X"), field_hash("Screen_Offset_Y")];
    pub const TRACKING_SPEED: u32 = field_hash("Tracking_Speed");
    pub const TRACKING_ACCELERATION: u32 = field_hash("TrackingAcceleration");
    pub const ZOOM: u32 = field_hash("Zoom_Params");
    pub const ZOOM_UNKNOWN: u32 = 0x72b0_9fef;
    pub const MAXIMUM_FOV: u32 = field_hash("Maximum_FOV");
    pub const MINIMUM_FOV: u32 = field_hash("Minimum_FOV");
    pub const USE_ZOOM: u32 = field_hash("Use_Zoom");
    // SceneAnalysis
    pub const ANALYSERS: u32 = field_hash("Analysers");
    pub const OFFSET_STRUCT: u32 = field_hash("Offset");
    pub const NAME: u32 = field_hash("Name");
    pub const TRANSLATION: u32 = field_hash("Translation");
    pub const ROTATION: u32 = field_hash("Rotation");
    // UILayout element geometry (the letterbox bars)
    pub const ELEMENTS: u32 = field_hash("Elements");
    pub const HEIGHT: u32 = field_hash("Height");
    pub const ELEMENT_Y: u32 = field_hash("PositionY");
    pub const ANCHOR: u32 = field_hash("Anchor");
    pub const TRANSITION_OUT: u32 = field_hash("TransitionOutBehaviour");
    pub const DURATION: u32 = field_hash("Duration");
    // GameLogicConfig: the rolling reset (FUN_006d0180 reads 0x220 / 0x224 / 0x24c)
    pub const RESET_MIN_MPH: u32 = 0x75d8_2dd5;
    pub const RESET_SPEED_SCALE: u32 = 0xa44e_b2f3;
    pub const RESET_CRASH_MAX_MPH: u32 = 0x7452_1df4;
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

/// A key of a `ModulatingDoubleValue` curve (x = time through the item, 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
    pub tangent_in: [f32; 2],
    pub tangent_out: [f32; 2],
}

/// `SequenceItem.ModulatingDoubleValue` without its binding (the crash sequence binds nothing).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Modulated {
    pub value: f32,
    pub output: [f32; 2],
    pub curve: Vec<CurvePoint>,
}

impl Modulated {
    /// Value at `t` (0..1 through the item). Keys are joined by cubic Hermite segments whose end slopes
    /// are the keys' tangents (dy/dx); with no keys the constant `Value`.
    pub fn at(&self, t: f32) -> f32 {
        let Some(first) = self.curve.first() else { return self.value };
        let t = t.clamp(0.0, 1.0);
        let y = if t <= first.x {
            first.y
        } else if let Some(w) = self.curve.windows(2).find(|w| t <= w[1].x) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.x - a.x).max(1e-6);
            let s = (t - a.x) / dx;
            let slope = |v: [f32; 2]| if v[0].abs() > 1e-6 { v[1] / v[0] } else { 0.0 };
            let (m0, m1) = (slope(a.tangent_out) * dx, slope(b.tangent_in) * dx);
            let (s2, s3) = (s * s, s * s * s);
            (2.0 * s3 - 3.0 * s2 + 1.0) * a.y + (s3 - 2.0 * s2 + s) * m0 + (-2.0 * s3 + 3.0 * s2) * b.y + (s3 - s2) * m1
        } else {
            self.curve.last().map_or(self.value, |p| p.y)
        };
        self.output[0] + y * (self.output[1] - self.output[0])
    }
}

/// `SlowMoSequenceItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlowMo {
    pub start: f32,
    pub end: f32,
    pub scale: Modulated,
}

/// `BystanderCameraParams.TrackingParameters`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tracking {
    pub screen_offset: [f32; 2],
    pub speed: f32,
    pub acceleration: f32,
}

/// `BystanderCameraParams.ZoomParameters`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Zoom {
    /// `72b09fef` (10 / 3.07): sorts before `Maximum_FOV`; unknown (a framing size?).
    pub unknown: f32,
    pub max_fov: f32,
    pub min_fov: f32,
    pub use_zoom: bool,
}

/// `BystanderCameraParams`: where a crash shot's camera stands and what it watches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bystander {
    pub id: ResourceId,
    /// `Camera_Transform_Binding`: `SAS.Analyser["Freeway"].Transform` or `Players.CameraTarget.GetTransform`.
    pub camera_binding: String,
    pub target_binding: String,
    /// `X/Y/Z_Offset`: added to the camera transform (m, in that transform's frame).
    pub offset: [f32; 3],
    /// `Target_X/Y/Z_Offset`: added to the target transform.
    pub target_offset: [f32; 3],
    /// `86980811` (10 / 1000) and `cc8312fe` (50 / 5.4): unknown (sort before `Target_X_Offset`).
    pub unknown: [f32; 2],
    /// `005c8fc7`: true on the scene-analyser shot, false on the car-relative one.
    pub flag: bool,
    pub tracking: Tracking,
    pub zoom: Zoom,
    /// The scene analyser the camera binding names (`SAS.Analyser["<name>"]`), if any.
    pub analyser: Option<SceneAnalyser>,
}

/// A `SimpleSceneAnalyser`: a space defined by an offset from the player.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SceneAnalyser {
    pub name_hash: u32,
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
}

/// One `CameraSequenceItem` with its (single, in the crash sequence) bystander camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub start: f32,
    pub end: f32,
    pub camera: Bystander,
}

/// The letterbox `UILayout` the sequence shows (two black bars on the 1280×720 reference frame).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Letterbox {
    pub start: f32,
    pub end: f32,
    /// Visible bar heights (reference pixels of 720): top, bottom.
    pub bars: [f32; 2],
    /// Slide-out transition (seconds) after `end`.
    pub transition_out: f32,
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

fn string(o: &Object, hash: u32) -> String {
    match o.get(hash) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
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

impl Modulated {
    fn read(o: &Object) -> Self {
        let curve = structs(o, h::ANIM_POINTS)
            .into_iter()
            .map(|p| {
                let f = |hash| float(p, hash).unwrap_or(0.0);
                CurvePoint {
                    x: f(h::POSITION_X),
                    y: f(h::POSITION_Y),
                    tangent_in: [f(h::TANGENT_IN_X), f(h::TANGENT_IN_Y)],
                    tangent_out: [f(h::TANGENT_OUT_X), f(h::TANGENT_OUT_Y)],
                }
            })
            .collect();
        Self {
            value: float(o, h::VALUE).unwrap_or(1.0),
            output: [float(o, h::OUTPUT_MIN).unwrap_or(0.0), float(o, h::OUTPUT_MAX).unwrap_or(1.0)],
            curve,
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

    /// Rules, crash camera and reset speeds together.
    pub fn crash_data(&self) -> Result<CrashData> {
        Ok(CrashData {
            rules: self.player_crashing_rules()?,
            camera: self.crash_camera()?,
            reset: self.crash_reset_speed()?,
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

impl Bystander {
    fn read(id: ResourceId, o: &Object) -> Self {
        let f = |o: Option<&Object>, hash| o.and_then(|o| float(o, hash)).unwrap_or(0.0);
        let tracking = sub(o, h::TRACKING);
        let zoom = sub(o, h::ZOOM);
        Self {
            id,
            camera_binding: string(o, h::CAMERA_BINDING),
            target_binding: string(o, h::TARGET_BINDING),
            offset: h::OFFSET.map(|hash| f(Some(o), hash)),
            target_offset: h::TARGET_OFFSET.map(|hash| f(Some(o), hash)),
            unknown: [f(Some(o), h::UNKNOWN_86), f(Some(o), h::UNKNOWN_CC)],
            flag: boolean(o, h::UNKNOWN_FLAG).unwrap_or(false),
            tracking: Tracking {
                screen_offset: h::SCREEN_OFFSET.map(|hash| f(tracking, hash)),
                speed: f(tracking, h::TRACKING_SPEED),
                acceleration: f(tracking, h::TRACKING_ACCELERATION),
            },
            zoom: Zoom {
                unknown: f(zoom, h::ZOOM_UNKNOWN),
                max_fov: f(zoom, h::MAXIMUM_FOV),
                min_fov: f(zoom, h::MINIMUM_FOV),
                use_zoom: zoom.and_then(|z| boolean(z, h::USE_ZOOM)).unwrap_or(false),
            },
            analyser: None,
        }
    }
}

impl Letterbox {
    /// Two full-width bars anchored at the top-centre (1) and bottom-centre (7) of their rectangles
    /// (`Anchor` is row-major 3×3): the visible part is what lies inside the 720-pixel-high frame.
    fn read(start: f32, end: f32, layout: &Object) -> Option<Self> {
        let mut bars = [0.0f32; 2];
        let mut transition_out = 0.0f32;
        for e in structs(layout, h::ELEMENTS) {
            let (y, height) = (float(e, h::ELEMENT_Y)?, float(e, h::HEIGHT)?);
            let anchor_row = name_anchor(e) / 3;
            let top = match anchor_row {
                0 => y,
                1 => y - height / 2.0,
                _ => y - height,
            };
            let (a, b) = (top.max(0.0), (top + height).min(720.0));
            if b <= a {
                continue;
            }
            if a <= 0.0 {
                bars[0] = bars[0].max(b);
            } else if b >= 720.0 {
                bars[1] = bars[1].max(720.0 - a);
            }
            for t in structs(e, h::TRANSITION_OUT) {
                transition_out = transition_out.max(float(t, h::DURATION).unwrap_or(0.0) / 1000.0);
            }
        }
        (bars[0] > 0.0 || bars[1] > 0.0).then_some(Self { start, end, bars, transition_out })
    }
}

fn name_anchor(e: &Object) -> u32 {
    float(e, h::ANCHOR).map_or(4, |a| a as u32)
}
