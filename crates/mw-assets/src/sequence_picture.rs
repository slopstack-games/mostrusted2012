//! The picture and environment items of a sequence (event intros and outros, UI and HUD sequences), and the
//! PostFX states they request (docs/decomp.md "Sequence PostFX states"). Field offsets are the GeneSys data's,
//! which the runtime items read in place (item data = `*(item + 8) + 0x28` + `*(item + 0xc)`).
//!
//! | Item (type id, factory, vtable) | Fields | What the game does |
//! |------|--------|--------------------|
//! | `PostFXSequenceItem` (0x3f861, `0x0086a630`, 0x00ff2fec) | `PostFXState` (+0x10c), `Intensity` (+0x24), `82d1338f` (+0x5c), `1a62fa2c` / `1a62fa2d` (+0x94 / +0xcc), `cb232034` (+0x104), mode `1e1303f0` (+0x114), `090eb59d` (+0x116), `354b8d7c` (+0x117) | requests the state on game logic +0x2ef70 at the start (`FUN_00863810` → `FUN_0083d850`), feeds it per frame (`FUN_00863960`), releases it at the end (`FUN_00863de0` → `FUN_0083d9e0`) |
//! | `ImagePaletteSequenceItem` (0x19df3d, `0x00830130`, 0x00fee7a4) | `1c29a1a2` palette (+0x24) | keeps a UI image palette requested while it runs (`0x00827df0` → `FUN_00796160`, ready = `FUN_007915f0`; end `FUN_00824bc0` → `FUN_007915b0`) |
//! | `DisableWorldRenderingSequenceItem` (0x9013d, `0x0086a450`, 0x00ff2ee8) | `c38f91c8` restore (+0x24) | start `FUN_008619c0`: game logic +0x2efb4 (= +0x2ef70 + 0x44) = 0; end `FUN_00863270`: back to 1 when `c38f91c8` |
//! | `SetTimeOfDaySequenceItem` (0x58f75, `0x00860700`, 0x00ff1a34) | `23198e19` / `da6ac384` times (+0x24 / +0x3c), `d6529487` (+0x54), flags +0x58…+0x5b | start `FUN_00855480`: the time to the environment manager (`FUN_00852c60` → `FUN_00847870` → `FUN_00847780`, messages 2 / 4 / 8); end `FUN_00850280`: the values saved at init (`0x00852c00` → `FUN_00897a20`) |
//! | `EnvironmentTimelineSequenceItem` (0x7bc83, `0x00860880`, 0x00ff1b04) | `5baabd32` timeline (+0x24) | start `FUN_00851900` → `FUN_00844060`(timeline, 1): message 7; end `0x0084fe90`: message 7 with no timeline |
//! | `SceneAnalysisSequenceItem` (0x6cc84, `0x008609a0`) | `77fde950` (+0x24) | not followed |
//! | `CameraShakeSequenceItem` (0x756e8, `0x008607c0`) | `d620d3bf` name (+0x24), a `CameraGameplayShakeEffect` (+0x2c) | not followed |
//! | `SetVisionModeTypeSequenceItem` (0x7bc89, `0x0086a3f0`) | `e2948eac` (+0x24) | not followed |

use mw_bundle::ResourceId;
use mw_formats::genesys::{Object, Value};

use crate::sequence::Modulated;
use crate::{AssetStore, Result};

/// A `PostFXSequenceItem`.
#[derive(Debug, Clone)]
pub struct PostFxItem {
    /// `PostFXState` (+0x10c): the `Genesys.Gen.PostFXState` requested while the item runs.
    pub state: Option<ResourceId>,
    /// `Intensity` (`f6726e73`, +0x24): the state's amount, when `set_intensity`.
    pub intensity: Modulated,
    /// `82d1338f` (+0x5c): the blend between the state's two keyframes, when `set_transition`.
    pub transition: Modulated,
    /// `1a62fa2c` / `1a62fa2d` (+0x94 / +0xcc): with mode 1 the vector (x, y, x, x) (`FUN_00863960`).
    pub vector: [Modulated; 2],
    /// `cb232034` (+0x104): with mode 2 a binding evaluated to the vector.
    pub vector_binding: String,
    /// `1e1303f0` (+0x114): 1 = the vector from `vector`, 2 = from `vector_binding`, else none.
    pub mode: u16,
    /// `090eb59d` (+0x116): the item sets the amount (`FUN_0083da30`).
    pub set_intensity: bool,
    /// `354b8d7c` (+0x117): the item sets the transition (`FUN_0083daf0`).
    pub set_transition: bool,
}

/// `SetTimeOfDaySequenceItem.Time`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TimeOfDay {
    pub hours: i32,
    pub minutes: i32,
    pub seconds: i32,
}

impl TimeOfDay {
    /// Seconds into the day (`FUN_00852c60`: h × 3600 + m × 60 + s, `.rdata` 0x01029fd8 / 0x01029fdc).
    pub fn seconds(&self) -> f32 {
        (self.hours * 3600 + self.minutes * 60 + self.seconds) as f32
    }
}

/// A `SetTimeOfDaySequenceItem`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetTimeOfDay {
    /// `23198e19` (+0x24): `FUN_00847870`'s third argument; above 0 it turns the jump into a transition.
    pub time0: TimeOfDay,
    /// `da6ac384` (+0x3c): the time the item sets (`FUN_00847870`'s first two message words).
    pub time1: TimeOfDay,
    /// `d6529487` (+0x54): passed through (flag 0x20 when non-zero).
    pub d6529487: f32,
    /// `FUN_00852c60`: +0x5a → 1, +0x59 → 4, +0x5b → 8 (`Thunder`), +0x58 → 0x10, `d6529487` ≠ 0 → 0x20.
    pub flags: u32,
}

/// The picture and environment items.
#[derive(Debug, Clone)]
pub enum PictureItem {
    PostFx(Box<PostFxItem>),
    ImagePalette {
        palette: u32,
    },
    /// `c38f91c8`: world rendering comes back at the end.
    DisableWorldRendering {
        restore: bool,
    },
    SetTimeOfDay(SetTimeOfDay),
    EnvironmentTimeline {
        timeline: Option<ResourceId>,
    },
    SceneAnalysis {
        analysis: Option<ResourceId>,
    },
    CameraShake {
        name: String,
    },
    SetVisionModeType {
        vision_mode: Option<ResourceId>,
    },
}

impl PictureItem {
    /// The item's kind, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PostFx(_) => "PostFX",
            Self::ImagePalette { .. } => "ImagePalette",
            Self::DisableWorldRendering { .. } => "DisableWorldRendering",
            Self::SetTimeOfDay(_) => "SetTimeOfDay",
            Self::EnvironmentTimeline { .. } => "EnvironmentTimeline",
            Self::SceneAnalysis { .. } => "SceneAnalysis",
            Self::CameraShake { .. } => "CameraShake",
            Self::SetVisionModeType { .. } => "SetVisionModeType",
        }
    }

    /// The item of type `short` (the type name without its namespace), if it is one of these.
    pub fn parse(short: &str, o: &Object) -> Option<Self> {
        Some(match short {
            "PostFXSequenceItem" => Self::PostFx(Box::new(PostFxItem {
                state: handle(o, 0x693d7f53),
                intensity: modulated(o, 0xf6726e73),
                transition: modulated(o, 0x82d1338f),
                vector: [modulated(o, 0x1a62fa2c), modulated(o, 0x1a62fa2d)],
                vector_binding: text(o, 0xcb232034),
                mode: num(o, 0x1e1303f0) as u16,
                set_intensity: boolean(o, 0x090eb59d),
                set_transition: boolean(o, 0x354b8d7c),
            })),
            "ImagePaletteSequenceItem" => Self::ImagePalette { palette: num(o, 0x1c29a1a2) as u32 },
            "DisableWorldRenderingSequenceItem" => Self::DisableWorldRendering { restore: boolean(o, 0xc38f91c8) },
            "SetTimeOfDaySequenceItem" => {
                let d6529487 = num(o, 0xd6529487);
                let mut flags = 0;
                for (hash, bit) in [(0xd4efa547, 1), (0x59c92710, 4), (0x56e0d395, 8), (0xa2b493e8, 0x10)] {
                    if boolean(o, hash) {
                        flags |= bit;
                    }
                }
                if d6529487 != 0.0 {
                    flags |= 0x20;
                }
                Self::SetTimeOfDay(SetTimeOfDay {
                    time0: time(o, 0x23198e19),
                    time1: time(o, 0xda6ac384),
                    d6529487,
                    flags,
                })
            }
            "EnvironmentTimelineSequenceItem" => Self::EnvironmentTimeline { timeline: handle(o, 0x5baabd32) },
            "SceneAnalysisSequenceItem" => Self::SceneAnalysis { analysis: handle(o, 0x77fde950) },
            "CameraShakeSequenceItem" => Self::CameraShake { name: text(o, 0xd620d3bf) },
            "SetVisionModeTypeSequenceItem" => Self::SetVisionModeType { vision_mode: handle(o, 0xe2948eac) },
            _ => return None,
        })
    }
}

/// `PostFXState.ColourCubeSettings` (28 bytes at +0xc and +0x28).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CubeSetting {
    /// `ColourCube` (`33029c92`).
    pub cube: Option<ResourceId>,
    /// `c208764f` (+0x18): without a cube, zero the slot's weight (`FUN_008e4a90`).
    pub clear: bool,
}

/// `PostFXState.ValueModifier`: how one block scales the state's amount.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ValueModifier {
    /// `Modification_Value` (`78eaebc4`).
    pub value: f32,
    /// `Modification_Type` (`2c75acad`): 1 = amount ^ value (`FUN_00ed9e1b`, SSE `powf`), 2 = amount × value,
    /// else the amount as it is.
    pub kind: u16,
}

impl ValueModifier {
    /// The modified amount, clamped to [0, 1] as every block does.
    pub fn apply(&self, amount: f32) -> f32 {
        let a = match self.kind {
            1 => amount.powf(self.value),
            2 => amount * self.value,
            _ => amount,
        };
        if a.is_nan() { 0.0 } else { a.clamp(0.0, 1.0) }
    }
}

/// A `Genesys.Gen.PostFXState` (248 bytes; `POSTFX.BNDL`'s seven base states and the sequences' own).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostFxState {
    pub id: u64,
    /// `6a1a9971` (+0xc, +0x28): the two colour-cube settings (TintBlend's two state slots).
    pub cubes: [CubeSetting; 2],
    /// `dd9c7a34` (+0x44): bloom (`FUN_008e9610`).
    pub bloom_modifier: ValueModifier,
    /// `213edc81` (+0x5c): the colour cubes (`FUN_008e4a90`).
    pub cube_modifier: ValueModifier,
    /// `d3753388` (+0x74): depth of field (`FUN_008e4f20`, reads +0x84 / +0x88; inferred from the layout).
    pub dof_modifier: ValueModifier,
    /// `f4d7592c` (+0x8c): the General block (`FUN_008e4d00`).
    pub general_modifier: ValueModifier,
    /// `77ceda16` (+0xa4): the vignette (`FUN_008e4b30`).
    pub vignette_modifier: ValueModifier,
    /// `582a5d55` (+0xbc): the state's enable binding (slot +0x8; off: no colour cubes).
    pub enable_binding: String,
    /// `b2bcd17a` (+0xc4): the transition binding (slot +0x80).
    pub transition_binding: String,
    /// `ValueBinding` (`8a90778d`, +0xcc): the amount's target (slot +0xf8).
    pub value_binding: String,
    /// `ccdbc907` (+0xd4): the state's keyframe.
    pub keyframe: Option<ResourceId>,
    /// `ccdbc904` (+0xdc): the keyframe blended towards by the transition.
    pub next_keyframe: Option<ResourceId>,
    /// `8af3cdfc` (+0xe8): the amount's rate (`FUN_008e4970`).
    pub rate: f32,
    /// `Value` (`b28ad39b`, +0xec): the amount a new slot starts from (`FUN_008e48b0`).
    pub value: f32,
    /// `ChangeBehaviour` (`13424acd`, +0xf0): 0 = jump to the target, 1 = linear at `rate` per second,
    /// 2 = exponential (`exp(−rate × dt)` of the gap left).
    pub change: u16,
    /// `0ee8a5ee` (+0xf4): blend the bloom.
    pub bloom: bool,
    /// `c2fb09c4` (+0xf5): blend the depth of field.
    pub dof: bool,
    /// `b5880b63` (+0xf6): blend the General block.
    pub general: bool,
    /// `28e0a4ad` (+0xf7): blend the vignette.
    pub vignette: bool,
}

impl PostFxState {
    pub fn from_object(id: u64, o: &Object) -> Self {
        let mut cubes = [CubeSetting::default(); 2];
        if let Some(Value::Array(a)) = o.get(0x6a1a9971) {
            for (slot, v) in cubes.iter_mut().zip(a) {
                if let Value::Struct(s) = v {
                    *slot = CubeSetting { cube: handle(s, 0x33029c92), clear: boolean(s, 0xc208764f) };
                }
            }
        }
        let modifier = |hash| match o.get(hash) {
            Some(Value::Struct(s)) => ValueModifier { value: num(s, 0x78eaebc4), kind: num(s, 0x2c75acad) as u16 },
            _ => ValueModifier::default(),
        };
        Self {
            id,
            cubes,
            bloom_modifier: modifier(0xdd9c7a34),
            cube_modifier: modifier(0x213edc81),
            dof_modifier: modifier(0xd3753388),
            general_modifier: modifier(0xf4d7592c),
            vignette_modifier: modifier(0x77ceda16),
            enable_binding: text(o, 0x582a5d55),
            transition_binding: text(o, 0xb2bcd17a),
            value_binding: text(o, 0x8a90778d),
            keyframe: handle(o, 0xccdbc907),
            next_keyframe: handle(o, 0xccdbc904),
            rate: num(o, 0x8af3cdfc),
            value: num(o, 0xb28ad39b),
            change: num(o, 0x13424acd) as u16,
            bloom: boolean(o, 0x0ee8a5ee),
            dof: boolean(o, 0xc2fb09c4),
            general: boolean(o, 0xb5880b63),
            vignette: boolean(o, 0x28e0a4ad),
        }
    }

    pub fn load(store: &AssetStore, id: ResourceId) -> Result<Self> {
        Ok(Self::from_object(id.0, &store.genesys_object(id)?))
    }
}

fn num(o: &Object, hash: u32) -> f32 {
    match o.get(hash) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        Some(Value::Bool(b)) => f32::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn boolean(o: &Object, hash: u32) -> bool {
    match o.get(hash) {
        Some(Value::Bool(b)) => *b,
        Some(Value::UInt(u)) => *u != 0,
        Some(Value::Int(i)) => *i != 0,
        _ => false,
    }
}

fn text(o: &Object, hash: u32) -> String {
    match o.get(hash) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn handle(o: &Object, hash: u32) -> Option<ResourceId> {
    match o.get(hash)? {
        Value::Handle(h) => *h,
        Value::Ref(r) => Some(*r),
        _ => None,
    }
}

fn modulated(o: &Object, hash: u32) -> Modulated {
    match o.get(hash) {
        Some(Value::Struct(s)) => Modulated::read(s),
        _ => Modulated::constant(1.0),
    }
}

fn time(o: &Object, hash: u32) -> TimeOfDay {
    match o.get(hash) {
        Some(Value::Struct(s)) => TimeOfDay {
            hours: num(s, 0xcc0caff7) as i32,
            minutes: num(s, 0xea240296) as i32,
            seconds: num(s, 0x1903a0e4) as i32,
        },
        _ => TimeOfDay::default(),
    }
}
