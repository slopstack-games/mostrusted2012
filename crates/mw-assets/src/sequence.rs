//! Game-logic `Sequence`s: the timelines behind the race intros and outros, the crash camera and the other
//! cutscenes (docs/decomp.md "Cutscenes and sequences"; formats in docs/formats.md "Sequences").
//!
//! A `Genesys.Gen.Sequence` (44 B) is a list of polymorphic `SequenceItem`s, each active between its
//! `StartTime` and `EndTime` on the sequence clock (real seconds; an empty `Binding` makes the axis time), and
//! a list of `TimelineControllers` that move the clock (skip on a button, loops waiting for a trigger).
//! Sequences live in feedback message groups (`EN_US/FEEDBACKGROUPS/<group>.BNDL`, a `FeedbackMessageGroup`
//! whose `Sequences2` lists them) and are named by their `CgsCore.UniqueId` (`655d3f5f`): an `OfflineEvent`'s
//! `EventIntro` lives in its `IntroMessageGroup`, its `EventOutro` / `AROutro` in the game mode's group
//! (604180 for `FeedbackGroupID` 2).
//!
//! | type (size) | field | meaning |
//! |------|-------|---------|
//! | `SequenceItem` base (36) | `4531983b` StringBase (+0x0c) | `EnabledBinding` |
//! | | `ddb9c614` / `6c82122b` f32 (+0x18 / +0x1c) | `StartTime` / `EndTime` (s) |
//! | `Sequence` (44) | `SequenceItems` (+0x20), `TimelineControllers` (+0x24) | items, clock controllers |
//! | `AnimatedCameraSequenceItem` (240) | `FocalDistance` (`b649881c`), `FocalLength` (`4b852710`), `FStop` (`cc8312fe`) | modulated values (mm / m) |
//! | | `6f3ea350` StringBase | the transform the clip is relative to (`Players.LocalPlayer.GetTransform`; empty = world) |
//! | | `ShakeBinding`, `CameraAnimation` (`c8496172`), `CameraSkeleton` (`a3fc3e26`), `b4bc74c3` bool | clip on a two-bone camera skeleton |
//! | `CameraSequenceItem` (44) | `Cameras` [`Camera` (60)] | `Bystander_Params` handle, or an ICE take (`7aef95c3` = `CameraTake` UniqueId) with bindings `01130178`[] and flags `0bd6ff75`[] |
//! | `SequenceItems.DynamicCameraSequenceItem` (56) | `DynamicCamera` handle, `f43d80c2` [`AnimatedChannel` {`Asset` = a process node id, `Modulation`, `PropertyName`}] | blends to the gameplay camera |
//! | `TeleportPlayerSequenceItem` (112) | `Direction`, `Position` (vec3), `37dea4a1` racer binding, `TransformBinding`, `Trigger` (UniqueId), `Speed_MPH` (`77beef06`) | puts a racer on a trigger point, rolling |
//! | `DriveLocalPlayerWithAISequenceItem` (48) | `MaxSpeedMph` (`b4cbecff`), `DrivingStyle` | the AI drives the player's car |
//! | `AddBehaviourToRaceCarSequenceItem` (52) | `37dea4a1` racer, `Behaviour` handle (`AnimatedRaceCarBehaviour`: `Animation`, `Skeleton`, bone names) | the car follows a clip |
//! | `SlowMoSequenceItem` | `ae525972` modulated | the simulation's time scale |
//! | `LayoutSequenceItem` | `Layout` handle, `LayoutLayer` | a UI layout (the letterbox is `0x19c66`) |
//! | `ConductorSequenceItem` | `Sequence` handle | a nested sequence, started at the item's start |
//! | `JumpOnButtonPress` (40) | `TriggerType`, `DestinationTime`, `TriggerTime`, `Action` | skip: after `TriggerTime`, a press jumps to `DestinationTime` |
//!
//! Field names were cracked with `mwtool genesys-crack` (hashes kept where no name fits).

use std::collections::HashMap;
use std::sync::Arc;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::camera_take::CameraTake;
use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::{Animation, Trigger, TriggerShape};

use crate::{AssetStore, Error, Result};

mod h {
    use mw_formats::genesys::field_hash;

    pub const ID: u32 = 0x655d_3f5f;
    pub const SEQUENCE_ITEMS: u32 = field_hash("SequenceItems");
    pub const WAVES: u32 = field_hash("Waves");
    pub const GAIN: u32 = field_hash("Gain");
    pub const FADE_IN: u32 = field_hash("Fade_In");
    pub const FADE_OUT: u32 = field_hash("Fade_Out");
    pub const TIME: u32 = field_hash("Time");
    pub const TYPE: u32 = field_hash("Type");
    pub const MIXER_CHANNEL: u32 = field_hash("Mixer_Channel");
    pub const SONG_LIST: u32 = field_hash("SongList");
    pub const STATE: u32 = field_hash("State");
    pub const TIMELINE_CONTROLLERS: u32 = field_hash("TimelineControllers");
    pub const SEQUENCES2: u32 = field_hash("Sequences2");
    pub const START_TIME: u32 = field_hash("StartTime");
    pub const END_TIME: u32 = field_hash("EndTime");
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
    pub const BINDING: u32 = field_hash("Binding");
    // AnimatedCameraSequenceItem
    pub const FOCAL_DISTANCE: u32 = field_hash("FocalDistance");
    pub const FOCAL_LENGTH: u32 = field_hash("FocalLength");
    pub const F_STOP: u32 = field_hash("FStop");
    pub const RELATIVE_TO: u32 = 0x6f3e_a350;
    pub const SHAKE_BINDING: u32 = field_hash("ShakeBinding");
    pub const CAMERA_ANIMATION: u32 = field_hash("CameraAnimation");
    pub const CAMERA_SKELETON: u32 = field_hash("CameraSkeleton");
    // CameraSequenceItem
    pub const CAMERAS: u32 = field_hash("Cameras");
    pub const BYSTANDER_PARAMS: u32 = field_hash("Bystander_Params");
    pub const TAKE: u32 = 0x7aef_95c3;
    pub const TAKE_BINDINGS: u32 = 0x0113_0178;
    pub const TAKE_FLAGS: u32 = 0x0bd6_ff75;
    pub const ENABLE_COLLISION: u32 = field_hash("Enable_Collision");
    pub const CAMERA_752A: u32 = 0x752a_7ea9;
    pub const CAMERA_650A: u32 = 0x650a_0f6e;
    pub const CAMERA_CA15: u32 = 0xca15_aa55;
    // DynamicCameraSequenceItem
    pub const DYNAMIC_CAMERA: u32 = field_hash("DynamicCamera");
    pub const CHANNELS: u32 = 0xf43d_80c2;
    pub const ASSET: u32 = field_hash("Asset");
    pub const MODULATION: u32 = field_hash("Modulation");
    // TeleportPlayerSequenceItem
    pub const DIRECTION: u32 = field_hash("Direction");
    pub const POSITION: u32 = field_hash("Position");
    pub const RACER: u32 = 0x37de_a4a1;
    pub const TRANSFORM_BINDING: u32 = field_hash("TransformBinding");
    pub const TRIGGER: u32 = field_hash("Trigger");
    pub const SPEED_MPH: u32 = field_hash("Speed_MPH");
    // DriveLocalPlayerWithAISequenceItem
    pub const MAX_SPEED_MPH: u32 = field_hash("MaxSpeedMph");
    pub const DRIVING_STYLE: u32 = field_hash("DrivingStyle");
    // AddBehaviourToRaceCarSequenceItem / AnimatedRaceCarBehaviour
    pub const BEHAVIOUR: u32 = field_hash("Behaviour");
    pub const ANIMATION: u32 = field_hash("Animation");
    // SlowMoSequenceItem
    pub const SLOW_MO_VALUE: u32 = 0xae52_5972;
    // LayoutSequenceItem
    pub const LAYOUT: u32 = field_hash("Layout");
    pub const LAYOUT_LAYER: u32 = field_hash("LayoutLayer");
    // ClearTrafficSequenceItem
    pub const RADIUS: u32 = field_hash("Radius");
    pub const CLEAR_TRAFFIC_BINDING: u32 = 0x79e4_8d67;
    // ConductorSequenceItem
    pub const SEQUENCE: u32 = field_hash("Sequence");
    pub const ENABLED_BINDING: u32 = field_hash("EnabledBinding");
    /// `JumpTimelineController`'s destination (unnamed; `sfx_loader` reads the same field).
    pub const JUMP_DESTINATION: u32 = 0xb6a5_1260;
    // LayoutSequenceItem texts (`Text1` is unnamed)
    pub const TEXT1: u32 = 0xbf1c_e9a1;
    pub const TEXT2: u32 = field_hash("Text2");
    pub const TEXT3: u32 = field_hash("Text3");
    pub const INSTANCE_PARAMS: u32 = field_hash("InstanceParams");
    // MovieSequenceItem
    pub const MOVIE: u32 = field_hash("Movie");
    pub const LOOP: u32 = field_hash("Loop");
    // StreamedImageSequenceItem
    pub const IMAGE_TARGET: u32 = 0xbe87_ac54;
    pub const IMAGE_INDEX: u32 = 0xcd5c_111c;
    // GeneralTriggerSequenceItem
    pub const NAME: u32 = field_hash("Name");
    // Timeline controllers
    pub const TRIGGER_TYPE: u32 = field_hash("TriggerType");
    pub const TRIGGER_TIME: u32 = field_hash("TriggerTime");
    pub const DESTINATION_TIME: u32 = field_hash("DestinationTime");
    pub const ACTION: u32 = field_hash("Action");
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
    // UILayout element geometry (the letterbox bars)
    pub const ELEMENTS: u32 = field_hash("Elements");
    pub const HEIGHT: u32 = field_hash("Height");
    pub const ELEMENT_Y: u32 = field_hash("PositionY");
    pub const ANCHOR: u32 = field_hash("Anchor");
    pub const TRANSITION_OUT: u32 = field_hash("TransitionOutBehaviour");
    pub const DURATION: u32 = field_hash("Duration");
}

/// The feedback group a game mode's `FeedbackGroupID` selects (the HSM's "Select Game Mode Feedback Group"
/// state lists 604180 then 604179; races, speed runs and the DLC modes have 2, ambushes 1; inferred order).
pub fn game_mode_feedback_group(feedback_group_id: u8) -> Option<u32> {
    match feedback_group_id {
        2 => Some(604_180),
        1 => Some(604_179),
        _ => None,
    }
}

/// The global feedback group (loaded at boot: "Load Feedback Message Group - Global"; holds the crash camera).
pub const GLOBAL_FEEDBACK_GROUP: u32 = 457_707;

// ---------------------------------------------------------------------------------------------------------
// Shared value types (also used by `crate::crash`).

/// A key of a `ModulatingDoubleValue` curve (x = time through the item, 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
    pub tangent_in: [f32; 2],
    pub tangent_out: [f32; 2],
}

/// `SequenceItem.ModulatingDoubleValue` without its binding modulation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Modulated {
    pub value: f32,
    pub output: [f32; 2],
    pub curve: Vec<CurvePoint>,
    /// `Binding` (empty in the cutscenes ported here).
    pub binding: String,
}

impl Modulated {
    /// A constant.
    pub fn constant(value: f32) -> Self {
        Self { value, output: [0.0, 1.0], curve: Vec::new(), binding: String::new() }
    }

    /// Value at `t` (0..1 through the item). Keys are joined by cubic Hermite segments whose end slopes
    /// are the keys' tangents (dy/dx); the result is mapped onto the output range. With no keys the constant
    /// `Value`.
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

    pub(crate) fn read(o: &Object) -> Self {
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
            binding: string(o, h::BINDING),
        }
    }

    fn field(o: &Object, hash: u32, default: f32) -> Self {
        sub(o, hash).map_or_else(|| Self::constant(default), Self::read)
    }
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

/// `BystanderCameraParams`: a camera standing at a bound transform and watching another.
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

impl Bystander {
    pub(crate) fn read(id: ResourceId, o: &Object) -> Self {
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

/// A `SimpleSceneAnalyser`: a space defined by an offset from the player.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SceneAnalyser {
    pub name_hash: u32,
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
}

/// The letterbox `UILayout` (`0x19c66`): two black bars on the 1280×720 reference frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Letterbox {
    pub start: f32,
    pub end: f32,
    /// Visible bar heights (reference pixels of 720): top, bottom.
    pub bars: [f32; 2],
    /// Slide-out transition (seconds) after `end`.
    pub transition_out: f32,
}

impl Letterbox {
    /// Two full-width bars anchored at the top-centre (1) and bottom-centre (7) of their rectangles
    /// (`Anchor` is row-major 3×3): the visible part is what lies inside the 720-pixel-high frame.
    pub(crate) fn read(start: f32, end: f32, layout: &Object) -> Option<Self> {
        let mut bars = [0.0f32; 2];
        let mut transition_out = 0.0f32;
        for e in structs(layout, h::ELEMENTS) {
            let (y, height) = (float(e, h::ELEMENT_Y)?, float(e, h::HEIGHT)?);
            let anchor_row = float(e, h::ANCHOR).map_or(4, |a| a as u32) / 3;
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

    /// Whether the bars show at `t`, and how far they have slid out (0..1).
    pub fn state(&self, t: f32) -> Option<f32> {
        let out = ((t - self.end) / self.transition_out.max(1e-3)).clamp(0.0, 1.0);
        (t >= self.start && out < 1.0).then_some(out)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Sequence items.

/// A pose of the camera bone of an `AnimatedCameraSequenceItem`'s clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraKey {
    /// Position (m) in the clip's space (world, or the bound transform's frame).
    pub translation: [f32; 3],
    /// Orientation (quaternion x, y, z, w) in the same space.
    pub rotation: [f32; 4],
}

/// `AnimatedCameraSequenceItem`: a camera clip with lens values.
#[derive(Debug, Clone)]
pub struct AnimatedCamera {
    /// `FocalDistance` (m; depth of field).
    pub focal_distance: Modulated,
    /// `FocalLength` (mm): the field of view is `2 atan(18 / f)` (the 0x2dc0 director camera, `FUN_005c6c10`:
    /// `.rdata` 0x00fb0e94 = 18, i.e. a 36 mm frame).
    pub focal_length: Modulated,
    /// `FStop` (aperture = focal length / f-stop, `FUN_005c6c10`).
    pub f_stop: Modulated,
    /// `6f3ea350`: the transform the clip is expressed in (`Players.LocalPlayer.GetTransform`); empty = world.
    pub relative_to: String,
    pub shake_binding: String,
    pub animation_id: Option<ResourceId>,
    pub skeleton_id: Option<ResourceId>,
    /// The camera bone's keys, one per frame at `frame_rate`.
    pub keys: Vec<CameraKey>,
    pub frame_rate: f32,
    /// `b4bc74c3` (true everywhere).
    pub flag: bool,
}

impl AnimatedCamera {
    /// The field of view (degrees) a focal length gives (`FUN_005c6c10`: `2 atan(18 / f)` in degrees).
    pub fn fov_degrees(focal_length_mm: f32) -> f32 {
        2.0 * (18.0 / focal_length_mm.max(1e-3)).atan().to_degrees()
    }

    /// The camera bone at `t` seconds into the item (clamped to the clip; frames linearly interpolated,
    /// rotations normalised-lerped).
    pub fn key_at(&self, t: f32) -> Option<CameraKey> {
        let last = self.keys.len().checked_sub(1)?;
        let f = (t * self.frame_rate).clamp(0.0, last as f32);
        let i = (f.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let s = f - i as f32;
        let (a, b) = (self.keys[i], self.keys[j]);
        let dot: f32 = (0..4).map(|k| a.rotation[k] * b.rotation[k]).sum();
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut q: [f32; 4] = std::array::from_fn(|k| a.rotation[k] + (b.rotation[k] * sign - a.rotation[k]) * s);
        let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
        q = if len > 1e-6 { q.map(|c| c / len) } else { [0.0, 0.0, 0.0, 1.0] };
        Some(CameraKey {
            translation: std::array::from_fn(|k| a.translation[k] + (b.translation[k] - a.translation[k]) * s),
            rotation: q,
        })
    }

    /// Clip length (s).
    pub fn length(&self) -> f32 {
        self.keys.len().saturating_sub(1) as f32 / self.frame_rate.max(1e-3)
    }
}

/// One camera of a `CameraSequenceItem`: a bystander shot or an ICE take (`CameraTake`, CAMERAS.BNDL).
#[derive(Debug, Clone)]
pub struct CameraShot {
    pub bystander: Option<Bystander>,
    /// `7aef95c3`: the `CameraTake` UniqueId (its resource id's low 32 bits), 0 = none.
    pub take: u32,
    /// The take itself (`mw_formats::camera_take`, CAMERAS.BNDL).
    pub take_data: Option<Arc<CameraTake>>,
    /// `01130178`: the transforms the take is bound to (eye, target: `Players.LocalPlayer.GetTransform`).
    pub bindings: Vec<String>,
    /// `0bd6ff75`: one flag per binding (true everywhere seen): sample the binding once, when the take starts
    /// (`FUN_0049a910` re-reads a binding every frame only while its flag is clear).
    pub binding_flags: Vec<bool>,
    /// `752a7ea9` f32, `650a0f6e` / `ca15aa55` u16 (0, 2, 2 in the outros): unknown.
    pub unknown: (f32, u32, u32),
    pub enable_collision: bool,
}

/// A `DynamicCameraSequenceItem` channel: a lerp node of the `DynamicCamera` driven by a curve.
#[derive(Debug, Clone)]
pub struct DynamicChannel {
    /// The process node the channel drives (by its UniqueId).
    pub asset: u32,
    /// What that node is: the transform lerp (`CameraTransformLerp`) or the FOV lerp (`ProcessNodeFloat32Lerp`).
    pub target: DynamicTarget,
    /// The lerp factor over the item (0 = the previous camera, 1 = the bound one).
    pub factor: Modulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicTarget {
    Transform,
    Fov,
    Other,
}

/// `DynamicCameraSequenceItem` with a `Camera.DynamicCamera` whose process nodes read the gameplay camera
/// (`Camera.GameplayExternalCameraTransform`, `Camera.GameplayExternalFOV`) and lerp to it.
#[derive(Debug, Clone)]
pub struct DynamicCamera {
    pub id: Option<ResourceId>,
    /// The bindings the camera's process nodes read (transform, FOV).
    pub bindings: Vec<String>,
    pub channels: Vec<DynamicChannel>,
}

impl DynamicCamera {
    /// Blend factors (transform, FOV) at `u` (0..1 through the item).
    pub fn factors(&self, u: f32) -> (f32, f32) {
        let get = |target| self.channels.iter().find(|c| c.target == target).map(|c| c.factor.at(u));
        let transform = get(DynamicTarget::Transform).unwrap_or(u);
        (transform, get(DynamicTarget::Fov).unwrap_or(transform))
    }
}

/// Which racer a `37dea4a1` binding names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacerRef {
    /// `Players.LocalPlayer.RaceCarIndex`.
    LocalPlayer,
    /// `Players.Racer[n].RaceCarIndex` (n ≥ 1: the event's opponents in `AIPlayerInformation` order,
    /// inferred from the teleport triggers: Racer[1] goes to the first opponent's `Placement`).
    Racer(u32),
    Other,
}

impl RacerRef {
    pub fn parse(binding: &str) -> Self {
        if binding.starts_with("Players.LocalPlayer") || binding.starts_with("Player.LocalPlayer") {
            return Self::LocalPlayer;
        }
        binding
            .strip_prefix("Players.Racer[")
            .and_then(|r| r.split(']').next())
            .and_then(|n| n.parse().ok())
            .map_or(Self::Other, Self::Racer)
    }
}

/// `TeleportPlayerSequenceItem` with its trigger resolved.
#[derive(Debug, Clone)]
pub struct Teleport {
    pub racer: RacerRef,
    pub racer_binding: String,
    pub transform_binding: String,
    pub trigger: u32,
    /// The trigger point's position and unit direction, when it resolves (else `Position` / `Direction`).
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub resolved: bool,
    /// `Speed_MPH`: the car's speed along `direction` after the teleport.
    pub speed_mph: f32,
}

/// `AddBehaviourToRaceCarSequenceItem` with an `AnimatedRaceCarBehaviour`: the car follows a clip.
#[derive(Debug, Clone)]
pub struct AnimatedCar {
    pub racer: RacerRef,
    pub behaviour: Option<ResourceId>,
    pub label: String,
    /// The car bone (the skeleton's bone 1, parent of the four wheel bones; bone names `car`, `av_car` (angular
    /// velocity), `lv_car` (linear velocity), wheels and `controls_*` are listed in the behaviour, the
    /// skeleton stores CRC ids) per frame, world space.
    pub keys: Vec<CameraKey>,
    pub frame_rate: f32,
}

impl AnimatedCar {
    pub fn key_at(&self, t: f32) -> Option<CameraKey> {
        let cam = AnimatedCamera {
            focal_distance: Modulated::default(),
            focal_length: Modulated::default(),
            f_stop: Modulated::default(),
            relative_to: String::new(),
            shake_binding: String::new(),
            animation_id: None,
            skeleton_id: None,
            keys: self.keys.clone(),
            frame_rate: self.frame_rate,
            flag: false,
        };
        cam.key_at(t)
    }
}

/// `ClearTrafficSequenceItem`.
#[derive(Debug, Clone)]
pub struct ClearTraffic {
    pub binding: String,
    pub points: Vec<[f32; 3]>,
    pub radius: f32,
}

/// What an item does.
#[derive(Debug, Clone)]
pub enum ItemKind {
    AnimatedCamera(Box<AnimatedCamera>),
    Camera(Vec<CameraShot>),
    DynamicCamera(DynamicCamera),
    Teleport(Teleport),
    DriveWithAi {
        max_speed_mph: f32,
        style: u32,
    },
    AnimatedCar(Box<AnimatedCar>),
    HideRacecar(RacerRef),
    HideHud,
    Letterbox(Letterbox),
    /// Any other UI layout (results panels, titles, the front end's screens). `texts` are `Text1` (`bf1ce9a1`),
    /// `Text2`, `Text3`: language ids the layout reads as `UISequences.Params[#].TextN`.
    Layout {
        layout: Option<ResourceId>,
        layer: u32,
        texts: [u32; 3],
        /// `InstanceParams`: the `UILayoutInstanceParams` placing it.
        instance: Option<ResourceId>,
    },
    /// `MovieSequenceItem`: a movie (`UI/MOVIES/<movie>.VP6`) on the `MoviePlayer` element, looped or not.
    Movie {
        movie: u32,
        looping: bool,
    },
    /// `BlackQuadSequenceItem`: a full-screen black quad (under the sequence's layouts; our reading).
    BlackQuad,
    /// `SequenceItems.StreamedImageSequenceItem`: streams a picture into the image binding `target`
    /// (`Transitions.Image1`), its index from the binding `index` (`Signals.GetIntValue[1]`).
    StreamedImage {
        target: String,
        index: String,
    },
    /// `GeneralTriggerSequenceItem`: sets a named trigger (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`):
    /// `name` is the name's hash (1112493908 = "BOOT" on the title), `trigger` the index.
    GeneralTrigger {
        name: u32,
        trigger: u32,
    },
    /// `ConductorSequenceItem` kept as its own sequence (only when not flattening, see
    /// [`SequenceContext::flatten`]): it runs on its own clock (with its own controllers) from the item's start.
    Conductor(Box<Sequence>),
    SlowMo(Modulated),
    /// Signals the game logic (the intros end with one; inferred: the race starts on it).
    EventTrigger,
    ClearTraffic(ClearTraffic),
    /// `WaveSequenceItem`: plays one of its `Waves` (the title music's streams).
    Wave(WaveItem),
    /// `SetSongListSequenceItem`: hands the song manager a `SongList` (`mw_assets::music`); 0 = none.
    SetSongList {
        list: u32,
        /// `3c4e5e2b`, `f28251e6`, `cb97beb9` (meaning: docs/decomp.md "Music").
        flags: [bool; 3],
    },
    /// `MusicStateSequenceItem`: sets the dynamic music manager's state name ("Mapscreen", "Multiplayer").
    MusicState(String),
    /// The picture and environment items (PostFX states, image palettes, world rendering, time of day,
    /// environment timeline, ...): `crate::sequence_picture`.
    Picture(crate::sequence_picture::PictureItem),
    /// Audio, post-processing, speech, music and the other item types not ported (type name).
    Other(String),
}

/// A `WaveSequenceItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveItem {
    /// `Wave` resources (one is picked; the title's items hold one each).
    pub waves: Vec<ResourceId>,
    pub gain: Modulated,
    /// `Fade_In.Time` / `Fade_Out.Time`, s.
    pub fade_in: f32,
    pub fade_out: f32,
    /// `Type`: 7 on the title's intro, 0 on its one-shot bridge, 4 on its loop (our reading: 4 loops).
    pub kind: u32,
    pub mixer_channel: Option<ResourceId>,
}

/// One item on the timeline.
#[derive(Debug, Clone)]
pub struct Item {
    pub unique_id: u32,
    pub start: f32,
    pub end: f32,
    pub kind: ItemKind,
    /// Nesting depth (0 = the sequence's own item; items of conductor sequences are flattened in, shifted
    /// by the conductor's start and cut at its end).
    pub depth: u32,
    /// `EnabledBinding` (empty = always on).
    pub enabled: String,
}

/// A timeline controller.
#[derive(Debug, Clone, PartialEq)]
pub enum Controller {
    /// `JumpOnButtonPress`: once the clock passes `trigger_time`, the skip button jumps it to `destination`.
    JumpOnButtonPress { trigger_type: u32, trigger_time: f32, destination: f32, action: u32 },
    /// `JumpTimelineController` / `JumpMonitorTimelineController` / ...: jumps driven by game-logic triggers
    /// (waits for input in the outros); not run by the cutscenes. `enabled` is its `EnabledBinding` (the title's
    /// `Language.IsJapanese(NOT)` skip to 28.1 s); the destination is the unnamed `b6a51260` where
    /// `DestinationTime` is absent.
    Jump { kind: String, trigger_type: u32, trigger_time: f32, destination: f32, enabled: String },
}

/// A decoded `Sequence`.
#[derive(Debug, Clone)]
pub struct Sequence {
    pub id: ResourceId,
    pub unique_id: u32,
    pub items: Vec<Item>,
    pub controllers: Vec<Controller>,
    /// The last item end (s).
    pub length: f32,
}

impl Sequence {
    /// The skip: `(trigger time, destination)` of the first `JumpOnButtonPress`.
    pub fn skip(&self) -> Option<(f32, f32)> {
        self.controllers.iter().find_map(|c| match c {
            Controller::JumpOnButtonPress { trigger_time, destination, .. } => Some((*trigger_time, *destination)),
            _ => None,
        })
    }

    /// Items active at `t`.
    pub fn active(&self, t: f32) -> impl Iterator<Item = (usize, &Item)> {
        self.items.iter().enumerate().filter(move |(_, i)| t >= i.start && t < i.end)
    }

    /// Counts per item kind (for logs and `mwtool`).
    pub fn summary(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for i in &self.items {
            let name = match &i.kind {
                ItemKind::AnimatedCamera(_) => "AnimatedCamera".to_string(),
                ItemKind::Camera(_) => "Camera".into(),
                ItemKind::DynamicCamera(_) => "DynamicCamera".into(),
                ItemKind::Teleport(_) => "Teleport".into(),
                ItemKind::DriveWithAi { .. } => "DriveWithAi".into(),
                ItemKind::AnimatedCar(_) => "AnimatedCar".into(),
                ItemKind::HideRacecar(_) => "HideRacecar".into(),
                ItemKind::HideHud => "HideHud".into(),
                ItemKind::Letterbox(_) => "Letterbox".into(),
                ItemKind::Layout { .. } => "Layout".into(),
                ItemKind::SlowMo(_) => "SlowMo".into(),
                ItemKind::EventTrigger => "EventTrigger".into(),
                ItemKind::ClearTraffic(_) => "ClearTraffic".into(),
                ItemKind::Movie { .. } => "Movie".into(),
                ItemKind::BlackQuad => "BlackQuad".into(),
                ItemKind::StreamedImage { .. } => "StreamedImage".into(),
                ItemKind::GeneralTrigger { .. } => "GeneralTrigger".into(),
                ItemKind::Conductor(_) => "Conductor".into(),
                ItemKind::Wave(_) => "Wave".into(),
                ItemKind::SetSongList { .. } => "SetSongList".into(),
                ItemKind::MusicState(_) => "MusicState".into(),
                ItemKind::Picture(p) => p.name().into(),
                ItemKind::Other(n) => n.clone(),
            };
            match counts.iter_mut().find(|(n, _)| *n == name) {
                Some((_, c)) => *c += 1,
                None => counts.push((name, 1)),
            }
        }
        counts
    }

    /// The simulation's time scale at `t` (1 outside the slow-motion items; the deepest active item wins).
    pub fn time_scale(&self, t: f32) -> f32 {
        self.active(t)
            .filter_map(|(_, i)| match &i.kind {
                ItemKind::SlowMo(m) => Some((i.depth, m.at((t - i.start) / (i.end - i.start).max(1e-6)))),
                _ => None,
            })
            .max_by_key(|(d, _)| *d)
            .map_or(1.0, |(_, s)| s)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Reading.

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

fn uint(o: &Object, hash: u32) -> u32 {
    match o.get(hash) {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
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

fn vec3(v: Option<&Value>) -> Option<[f32; 3]> {
    match v? {
        Value::Vector(v) => Some([v[0], v[1], v[2]]),
        Value::Array(a) if a.len() >= 3 => Some([num(&a[0])?, num(&a[1])?, num(&a[2])?]),
        _ => None,
    }
}

fn short(o: &Object) -> &str {
    let n = o.type_name.strip_prefix("Genesys.Gen.").unwrap_or(&o.type_name);
    n.strip_prefix("SequenceItems.").unwrap_or(n)
}

/// The keys of the bone that a camera / car clip animates: the first bone with both a translation and a
/// rotation channel (bone 1 of the two-bone camera skeleton; the `car` bone of the race-car skeleton), plus
/// the root motion. Bind poses are identity in every such skeleton, so the keys are the bone's pose in the
/// clip's space.
fn bone_keys(animation: &Animation) -> Vec<CameraKey> {
    let Some(slot) = animation.bones.iter().position(|&b| b > 0) else { return Vec::new() };
    animation
        .frames
        .iter()
        .map(|f| {
            let k = f[slot];
            CameraKey { translation: k.translation, rotation: k.rotation }
        })
        .collect()
}

/// Where the sequences are found, and the trigger points they place cars on.
pub struct SequenceContext<'a> {
    pub triggers: HashMap<u32, Trigger>,
    pub store: &'a AssetStore,
    /// Sequences decoded so far (conductor recursion guard).
    depth: u32,
    /// Flatten conductor sequences into their parent (the cutscenes); `false` keeps each as an
    /// [`ItemKind::Conductor`] with its own controllers (the front end's looping image sequence).
    pub flatten: bool,
}

impl<'a> SequenceContext<'a> {
    pub fn new(store: &'a AssetStore) -> Result<Self> {
        let triggers = store.trigger_data()?.triggers.into_iter().map(|t| (t.id, t)).collect();
        Ok(Self { triggers, store, depth: 0, flatten: true })
    }

    fn animation(&self, id: Option<ResourceId>) -> Option<Animation> {
        let id = id?;
        Animation::parse(&self.store.load(id).ok()?).ok()
    }

    /// Read a `Genesys.Gen.Sequence` resource.
    pub fn read(&mut self, id: ResourceId) -> Result<Sequence> {
        let o = self.store.genesys_object(id)?;
        if short(&o) != "Sequence" {
            return Err(Error::WrongType {
                id,
                expected: ResourceType::GenesysObject,
                actual: ResourceType::GenesysObject,
            });
        }
        let mut seq =
            Sequence { id, unique_id: uint(&o, h::ID), items: Vec::new(), controllers: Vec::new(), length: 0.0 };
        for c in structs(&o, h::TIMELINE_CONTROLLERS) {
            let (trigger_type, trigger_time, destination) = (
                uint(c, h::TRIGGER_TYPE),
                float(c, h::TRIGGER_TIME).unwrap_or(0.0),
                float(c, h::DESTINATION_TIME).unwrap_or(0.0),
            );
            seq.controllers.push(match short(c) {
                "JumpOnButtonPress" => Controller::JumpOnButtonPress {
                    trigger_type,
                    trigger_time,
                    destination,
                    action: uint(c, h::ACTION),
                },
                kind => Controller::Jump {
                    kind: kind.to_string(),
                    trigger_type,
                    trigger_time,
                    destination: float(c, h::DESTINATION_TIME).or(float(c, h::JUMP_DESTINATION)).unwrap_or(0.0),
                    enabled: string(c, h::ENABLED_BINDING),
                },
            });
        }
        for item in structs(&o, h::SEQUENCE_ITEMS) {
            let (start, end) = (float(item, h::START_TIME).unwrap_or(0.0), float(item, h::END_TIME).unwrap_or(0.0));
            seq.length = seq.length.max(end);
            if short(item) == "ConductorSequenceItem" && !self.flatten {
                if self.depth < 4
                    && let Some(child_id) = handle(item.get(h::SEQUENCE))
                {
                    self.depth += 1;
                    let child = self.read(child_id);
                    self.depth -= 1;
                    if let Ok(child) = child {
                        seq.items.push(Item {
                            unique_id: uint(item, h::ID),
                            start,
                            end,
                            kind: ItemKind::Conductor(Box::new(child)),
                            depth: 0,
                            enabled: string(item, h::ENABLED_BINDING),
                        });
                    }
                }
                continue;
            }
            if short(item) == "ConductorSequenceItem" {
                if self.depth < 4
                    && let Some(child_id) = handle(item.get(h::SEQUENCE))
                {
                    self.depth += 1;
                    let child = self.read(child_id);
                    self.depth -= 1;
                    if let Ok(child) = child {
                        for mut c in child.items {
                            c.start += start;
                            c.end = (c.end + start).min(end);
                            c.depth += 1;
                            if c.end > c.start {
                                seq.items.push(c);
                            }
                        }
                    }
                }
                continue;
            }
            let kind = self.item_kind(item, start, end);
            let enabled = string(item, h::ENABLED_BINDING);
            seq.items.push(Item { unique_id: uint(item, h::ID), start, end, kind, depth: 0, enabled });
        }
        seq.items.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(seq)
    }

    fn racer(item: &Object) -> RacerRef {
        RacerRef::parse(&string(item, h::RACER))
    }

    fn item_kind(&self, item: &Object, start: f32, end: f32) -> ItemKind {
        match short(item) {
            "AnimatedCameraSequenceItem" => {
                let animation_id = handle(item.get(h::CAMERA_ANIMATION));
                let animation = self.animation(animation_id);
                ItemKind::AnimatedCamera(Box::new(AnimatedCamera {
                    focal_distance: Modulated::field(item, h::FOCAL_DISTANCE, 20.0),
                    focal_length: Modulated::field(item, h::FOCAL_LENGTH, 35.0),
                    f_stop: Modulated::field(item, h::F_STOP, 64.0),
                    relative_to: string(item, h::RELATIVE_TO),
                    shake_binding: string(item, h::SHAKE_BINDING),
                    animation_id,
                    skeleton_id: handle(item.get(h::CAMERA_SKELETON)),
                    keys: animation.as_ref().map(bone_keys).unwrap_or_default(),
                    frame_rate: animation.as_ref().map_or(30.0, |a| a.frame_rate),
                    flag: boolean(item, 0xb4bc_74c3).unwrap_or(false),
                }))
            }
            "CameraSequenceItem" => ItemKind::Camera(
                structs(item, h::CAMERAS)
                    .into_iter()
                    .map(|c| {
                        let bystander = handle(c.get(h::BYSTANDER_PARAMS))
                            .and_then(|b| Some(Bystander::read(b, &self.store.genesys_object(b).ok()?)));
                        let strings = match c.get(h::TAKE_BINDINGS) {
                            Some(Value::Array(a)) => a
                                .iter()
                                .filter_map(|v| if let Value::Str(s) = v { Some(s.clone()) } else { None })
                                .collect(),
                            _ => Vec::new(),
                        };
                        let flags = match c.get(h::TAKE_FLAGS) {
                            Some(Value::Array(a)) => a.iter().map(|v| matches!(v, Value::Bool(true))).collect(),
                            _ => Vec::new(),
                        };
                        let take = uint(c, h::TAKE);
                        let take_data = (take != 0)
                            .then(|| self.store.load(ResourceId(0x0100_0000_0000_0000 | take as u64)).ok())
                            .flatten()
                            .and_then(|r| CameraTake::parse(&r).ok())
                            .map(Arc::new);
                        CameraShot {
                            bystander,
                            take,
                            take_data,
                            bindings: strings,
                            binding_flags: flags,
                            unknown: (
                                float(c, h::CAMERA_752A).unwrap_or(0.0),
                                uint(c, h::CAMERA_650A),
                                uint(c, h::CAMERA_CA15),
                            ),
                            enable_collision: boolean(c, h::ENABLE_COLLISION).unwrap_or(false),
                        }
                    })
                    .collect(),
            ),
            "DynamicCameraSequenceItem" => {
                let id = handle(item.get(h::DYNAMIC_CAMERA));
                // The camera's process nodes: which node id is the transform lerp and which the FOV lerp.
                let mut nodes: HashMap<u32, DynamicTarget> = HashMap::new();
                let mut bindings = Vec::new();
                if let Some(cam) = id.and_then(|id| self.store.genesys_object(id).ok()) {
                    collect_nodes(&cam, &mut nodes, &mut bindings);
                }
                let channels = structs(item, h::CHANNELS)
                    .into_iter()
                    .map(|c| {
                        let asset = uint(c, h::ASSET);
                        DynamicChannel {
                            asset,
                            target: nodes.get(&asset).copied().unwrap_or(DynamicTarget::Other),
                            factor: Modulated::field(c, h::MODULATION, 1.0),
                        }
                    })
                    .collect();
                ItemKind::DynamicCamera(DynamicCamera { id, bindings, channels })
            }
            "TeleportPlayerSequenceItem" => {
                let trigger = uint(item, h::TRIGGER);
                let (position, direction, resolved) = match self.triggers.get(&trigger) {
                    Some(Trigger { position, shape: TriggerShape::Point { direction }, .. }) => {
                        (*position, *direction, true)
                    }
                    Some(t) => (t.position, [0.0, 0.0, 1.0], true),
                    None => (
                        vec3(item.get(h::POSITION)).unwrap_or_default(),
                        vec3(item.get(h::DIRECTION)).unwrap_or([0.0, 0.0, 1.0]),
                        false,
                    ),
                };
                let racer_binding = string(item, h::RACER);
                ItemKind::Teleport(Teleport {
                    racer: RacerRef::parse(&racer_binding),
                    racer_binding,
                    transform_binding: string(item, h::TRANSFORM_BINDING),
                    trigger,
                    position,
                    direction,
                    resolved,
                    speed_mph: float(item, h::SPEED_MPH).unwrap_or(0.0),
                })
            }
            "DriveLocalPlayerWithAISequenceItem" => ItemKind::DriveWithAi {
                max_speed_mph: float(item, h::MAX_SPEED_MPH).unwrap_or(999.0),
                style: uint(item, h::DRIVING_STYLE),
            },
            "AddBehaviourToRaceCarSequenceItem" => {
                let behaviour = handle(item.get(h::BEHAVIOUR));
                let b = behaviour.and_then(|b| self.store.genesys_object(b).ok());
                match b.filter(|b| short(b) == "AnimatedRaceCarBehaviour") {
                    Some(b) => {
                        let animation = self.animation(handle(b.get(h::ANIMATION)));
                        ItemKind::AnimatedCar(Box::new(AnimatedCar {
                            racer: Self::racer(item),
                            behaviour,
                            label: string(&b, field_hash("Label")),
                            keys: animation.as_ref().map(bone_keys).unwrap_or_default(),
                            frame_rate: animation.as_ref().map_or(30.0, |a| a.frame_rate),
                        }))
                    }
                    None => ItemKind::Other("AddBehaviourToRaceCarSequenceItem".into()),
                }
            }
            "HideRacecarSequenceItem" => ItemKind::HideRacecar(Self::racer(item)),
            "HideHudSequenceItem" => ItemKind::HideHud,
            "LayoutSequenceItem" => {
                let layout = handle(item.get(h::LAYOUT));
                let letterbox = layout
                    .and_then(|l| self.store.genesys_object(l).ok())
                    .and_then(|l| Letterbox::read(start, end, &l))
                    .filter(|_| layout == Some(LETTERBOX_LAYOUT));
                match letterbox {
                    Some(l) => ItemKind::Letterbox(l),
                    None => ItemKind::Layout {
                        layout,
                        layer: uint(item, h::LAYOUT_LAYER),
                        texts: [uint(item, h::TEXT1), uint(item, h::TEXT2), uint(item, h::TEXT3)],
                        instance: handle(item.get(h::INSTANCE_PARAMS)),
                    },
                }
            }
            "MovieSequenceItem" => {
                ItemKind::Movie { movie: uint(item, h::MOVIE), looping: boolean(item, h::LOOP).unwrap_or(false) }
            }
            "BlackQuadSequenceItem" => ItemKind::BlackQuad,
            "StreamedImageSequenceItem" => {
                ItemKind::StreamedImage { target: string(item, h::IMAGE_TARGET), index: string(item, h::IMAGE_INDEX) }
            }
            "GeneralTriggerSequenceItem" => {
                ItemKind::GeneralTrigger { name: uint(item, h::NAME), trigger: uint(item, h::TRIGGER) }
            }
            "SlowMoSequenceItem" => ItemKind::SlowMo(Modulated::field(item, h::SLOW_MO_VALUE, 1.0)),
            "EventTriggerSequenceItem" => ItemKind::EventTrigger,
            "ClearTrafficSequenceItem" => ItemKind::ClearTraffic(ClearTraffic {
                binding: string(item, h::CLEAR_TRAFFIC_BINDING),
                points: match item.get(h::TRIGGER) {
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(|v| match v {
                            Value::UInt(u) => self.triggers.get(&(*u as u32)).map(|t| t.position),
                            Value::Int(i) => self.triggers.get(&(*i as u32)).map(|t| t.position),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                },
                radius: float(item, h::RADIUS).unwrap_or(0.0),
            }),
            "WaveSequenceItem" => ItemKind::Wave(WaveItem {
                waves: match item.get(h::WAVES) {
                    Some(Value::Array(a)) => a.iter().filter_map(|v| handle(Some(v))).collect(),
                    _ => Vec::new(),
                },
                gain: Modulated::field(item, h::GAIN, 1.0),
                fade_in: sub(item, h::FADE_IN).and_then(|f| float(f, h::TIME)).unwrap_or(0.0),
                fade_out: sub(item, h::FADE_OUT).and_then(|f| float(f, h::TIME)).unwrap_or(0.0),
                kind: uint(item, h::TYPE),
                mixer_channel: handle(item.get(h::MIXER_CHANNEL)),
            }),
            "SetSongListSequenceItem" => ItemKind::SetSongList {
                list: uint(item, h::SONG_LIST),
                flags: [0x3c4e_5e2b, 0xf282_51e6, 0xcb97_beb9].map(|k| boolean(item, k).unwrap_or(false)),
            },
            "MusicStateSequenceItem" => ItemKind::MusicState(string(item, h::STATE)),
            other => crate::sequence_picture::PictureItem::parse(other, item)
                .map_or_else(|| ItemKind::Other(other.to_string()), ItemKind::Picture),
        }
    }
}

/// The letterbox layout the cutscenes and the crash camera show.
pub const LETTERBOX_LAYOUT: ResourceId = ResourceId(0x0100_0000_0001_9c66);

/// Walk a `Camera.DynamicCamera`'s process-node lists: lerp nodes by id, and the bindings the nodes read.
fn collect_nodes(o: &Object, nodes: &mut HashMap<u32, DynamicTarget>, bindings: &mut Vec<String>) {
    let kind = short(o);
    let target = if kind.ends_with("CameraTransformLerp") {
        Some(DynamicTarget::Transform)
    } else if kind.ends_with("ProcessNodeFloat32Lerp") {
        Some(DynamicTarget::Fov)
    } else {
        None
    };
    if let Some(t) = target {
        nodes.insert(uint(o, h::ID), t);
    }
    let b = string(o, h::BINDING);
    if !b.is_empty() {
        bindings.push(b);
    }
    for (_, v) in &o.fields {
        match v {
            Value::Struct(s) => collect_nodes(s, nodes, bindings),
            Value::Array(a) => {
                for v in a {
                    if let Value::Struct(s) = v {
                        collect_nodes(s, nodes, bindings);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A `FeedbackMessage` (offsets in its data; `feedback_group_messages`). Field names in backticks are hashes not
/// cracked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeedbackMessageInfo {
    /// +0x88 UniqueId.
    pub id: u32,
    /// +0x90 `LayoutStyle`: the `UILayout` that shows it.
    pub layout_style: u32,
    /// +0x4c `bca59513`: the `UILayoutInstanceParams` placing it.
    pub layout_instance: Option<ResourceId>,
    /// `Title` (+0xb0), `0a899744` (+0xa8), `0a899747` (+0xac): language ids.
    pub texts: [u32; 3],
    /// +0xc4.
    pub time: f32,
    /// +0xd8 `a0ba493a`.
    pub layer: u16,
    /// +0xe6 `fffa9b53`, +0xe7 `13b2a760`, +0xe8 `0cefee9e`, +0xec `2d000db9`.
    pub extend_by_sequence: bool,
    pub flag_e7: bool,
    pub exclusive: bool,
    pub pause_with_game: bool,
    /// +0xf1 `f472d91c` with `CutsceneData.Cutscene` set.
    pub cutscene_clock: bool,
    /// +0xb8.
    pub transition_message: u32,
    /// +0xb4 `5fc73ed7`, +0xbc `0b910820`, +0xa4 `StreamedSound`, +0x94 `Movie`, +0x98 `Movie_Sound`, +0x9c
    /// `Speech` (first), +0x84 `CameraTake`, +0x8c `056664c3`.
    pub start_sound: u32,
    pub exit_sound: u32,
    pub streamed_sound: u32,
    pub movie: u32,
    pub movie_sound: u32,
    pub speech: u32,
    pub camera_take: u32,
    pub hud_mode: u32,
}

impl AssetStore {
    /// The `Sequence` with UniqueId `unique_id` in the feedback groups `groups` (searched in order; a group is
    /// `EN_US/FEEDBACKGROUPS/<group>.BNDL`).
    pub fn find_sequence(&self, groups: &[u32], unique_id: u32) -> Result<ResourceId> {
        for &g in groups {
            let Some(b) = self.bundle(format!("EN_US/FEEDBACKGROUPS/{g}.BNDL")) else { continue };
            for e in b.entries_of_type(ResourceType::GenesysObject) {
                let Ok(o) = self.decode_genesys(&b.load(e)?) else { continue };
                if o.type_name == "Genesys.Gen.Sequence" && uint(&o, h::ID) == unique_id {
                    return Ok(e.id);
                }
            }
        }
        Err(Error::Missing(ResourceId(unique_id as u64)))
    }

    /// Read a sequence by resource id (with its conductor sequences flattened in).
    pub fn sequence(&self, id: ResourceId) -> Result<Sequence> {
        SequenceContext::new(self)?.read(id)
    }

    /// A UI sequence by UniqueId (the front end's title and transitions): conductor sequences stay nested
    /// ([`ItemKind::Conductor`]), each with its own controllers.
    pub fn ui_sequence(&self, unique_id: u32) -> Result<Sequence> {
        let mut cx = SequenceContext::new(self)?;
        cx.flatten = false;
        cx.read(ResourceId(0x0100_0000_0000_0000 | unique_id as u64))
    }

    /// The `Sequences2` resource ids of a feedback group.
    pub fn feedback_group_sequences(&self, group: u32) -> Result<Vec<ResourceId>> {
        let b = self.bundle(format!("EN_US/FEEDBACKGROUPS/{group}.BNDL")).ok_or(Error::Missing(ResourceId::NULL))?;
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let Ok(o) = self.decode_genesys(&b.load(e)?) else { continue };
            if o.type_name == "Genesys.Gen.FeedbackMessageGroup" {
                return Ok(match o.get(h::SEQUENCES2) {
                    Some(Value::Array(a)) => a.iter().filter_map(|v| handle(Some(v))).collect(),
                    _ => Vec::new(),
                });
            }
        }
        Ok(Vec::new())
    }

    /// The `Messages` of a feedback group (`FeedbackMessageGroup` +0x10, count +0x1c), in order: the fields the UI
    /// message queues read (`mw_logic::natives::feedback_queue`; offsets in the object's data).
    pub fn feedback_group_messages(&self, group: u32) -> Result<Vec<FeedbackMessageInfo>> {
        let b = self.bundle(format!("EN_US/FEEDBACKGROUPS/{group}.BNDL")).ok_or(Error::Missing(ResourceId::NULL))?;
        let mut handles = Vec::new();
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let Ok(o) = self.decode_genesys(&b.load(e)?) else { continue };
            if o.type_name == "Genesys.Gen.FeedbackMessageGroup" {
                if let Some(Value::Array(a)) = o.get(field_hash("Messages")) {
                    handles = a.iter().filter_map(|v| handle(Some(v))).collect();
                }
                break;
            }
        }
        let mut out = Vec::new();
        for h in handles {
            let Ok(m) = self.genesys_object(h) else { continue };
            let flag = |hash| boolean(&m, hash).unwrap_or(false);
            let cutscene = sub(&m, 0xd6d3d9f6).and_then(|c| handle(c.get(field_hash("Cutscene")))).is_some();
            let speech = match m.get(0x5965_4d81) {
                Some(Value::Array(a)) => a.first().and_then(num).map_or(0, |f| f as u32),
                _ => 0,
            };
            out.push(FeedbackMessageInfo {
                id: uint(&m, h::ID),
                layout_style: uint(&m, field_hash("LayoutStyle")),
                layout_instance: handle(m.get(0xbca5_9513)),
                texts: [uint(&m, field_hash("Title")), uint(&m, 0x0a89_9744), uint(&m, 0x0a89_9747)],
                time: float(&m, field_hash("Time")).unwrap_or(0.0),
                layer: uint(&m, 0xa0ba_493a) as u16,
                extend_by_sequence: flag(0xfffa_9b53),
                flag_e7: flag(0x13b2_a760),
                exclusive: flag(0x0cef_ee9e),
                pause_with_game: flag(0x2d00_0db9),
                cutscene_clock: flag(0xf472_d91c) && cutscene,
                transition_message: uint(&m, field_hash("Transition_Message")),
                start_sound: uint(&m, 0x5fc7_3ed7),
                exit_sound: uint(&m, 0x0b91_0820),
                streamed_sound: uint(&m, field_hash("StreamedSound")),
                movie: uint(&m, field_hash("Movie")),
                movie_sound: uint(&m, field_hash("Movie_Sound")),
                speech,
                camera_take: uint(&m, field_hash("CameraTake")),
                hud_mode: uint(&m, 0x0566_64c3),
            });
        }
        Ok(out)
    }

    /// Load an event's intro and outro sequences (`EventIntro` from its `IntroMessageGroup`, `EventOutro` from
    /// the game mode's feedback group).
    #[allow(clippy::type_complexity)]
    pub fn event_sequences(
        &self,
        event: &crate::events::Event,
        mode: Option<&crate::events::GameMode>,
    ) -> Result<(Option<Arc<Sequence>>, Option<Arc<Sequence>>)> {
        let mut ctx = SequenceContext::new(self)?;
        let mut groups = vec![event.intro_message_group, event.feedback_message_group];
        if let Some(g) = mode.and_then(|m| game_mode_feedback_group(m.feedback_group_id)) {
            groups.push(g);
        }
        groups.push(GLOBAL_FEEDBACK_GROUP);
        groups.retain(|&g| g != 0);
        let mut load = |id: u32| -> Option<Arc<Sequence>> {
            if id == 0 {
                return None;
            }
            let rid = self.find_sequence(&groups, id).ok()?;
            ctx.read(rid).ok().map(Arc::new)
        };
        let intro = load(event.event_intro);
        let outro = load(event.event_outro);
        Ok((intro, outro))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focal_length_to_fov() {
        // 36 mm frame: a 35 mm lens gives 54.4°, 18 mm gives 90°.
        assert!((AnimatedCamera::fov_degrees(18.0) - 90.0).abs() < 1e-3);
        assert!((AnimatedCamera::fov_degrees(35.0) - 54.43).abs() < 0.01);
    }

    #[test]
    fn racer_bindings() {
        assert_eq!(RacerRef::parse("Players.LocalPlayer.RaceCarIndex"), RacerRef::LocalPlayer);
        assert_eq!(RacerRef::parse("Players.Racer[3].RaceCarIndex"), RacerRef::Racer(3));
        assert_eq!(RacerRef::parse("Signals.Foo"), RacerRef::Other);
    }

    #[test]
    fn modulated_curve_maps_to_output() {
        let m = Modulated {
            value: 10.0,
            output: [10.0, 20.0],
            curve: vec![
                CurvePoint { x: 0.0, y: 1.0, tangent_in: [1.0, 0.0], tangent_out: [1.0, 0.0] },
                CurvePoint { x: 1.0, y: 0.0, tangent_in: [1.0, 0.0], tangent_out: [1.0, 0.0] },
            ],
            binding: String::new(),
        };
        assert!((m.at(0.0) - 20.0).abs() < 1e-4);
        assert!((m.at(1.0) - 10.0).abs() < 1e-4);
        assert!((m.at(0.5) - 15.0).abs() < 1e-4);
        assert_eq!(Modulated::constant(3.0).at(0.5), 3.0);
    }
}
