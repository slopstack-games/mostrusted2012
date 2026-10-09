//! The game's UI: screens, widgets and layouts, all authored as GeneSys objects (`UI/SCREENS2/*.BNDL`,
//! `UI/LAYOUTS/*.BNDL`, `EN_US/FEEDBACKGROUPS/*.BNDL`). There is no Flash/Apt movie and no Scaleform: a
//! layout is a flat list of elements (images, labels, shapes, masks, stacks) placed on a 1280×720 reference
//! frame, with data bindings (`Players.LocalPlayer.NitrousState.NitrousAmount(MULT:246,ADD:893)`) that the
//! game's code evaluates every frame, and tweened behaviours for idle / transition-in / transition-out /
//! triggered states. Screens group widgets; a widget names its layouts and how each is placed
//! (`UILayoutInstanceParams`: an anchor and translate / rotate / scale components, again with bindings).
//!
//! These types are built from decoded [`genesys::Object`]s (see [`crate::genesys::Decoder`]); the decoding
//! itself needs the `GenesysType` resources, which the asset store provides.
//!
//! Element base fields (every `UIElement_*`, 184 bytes, inherited):
//!
//! | off | field | notes |
//! |-----|-------|-------|
//! | 12  | quad {`TextureHandle`, material id, renderables[], count} | renderables are unit quads whose UVs crop the texture |
//! | 44  | `HeightScale_Binding`, `PositionX_Binding`, `PositionY_Binding`, `Rotation_Binding`, ?, `VisibilityBinding`, `WidthScale_Binding` | strings |
//! | 100 | `Material` | handle to a `UIMaterial` |
//! | 108 | id | UniqueId |
//! | 112 | `Sounds`[] | |
//! | 116 | `TransitionInSound`, `TransitionOutSound` | |
//! | 128 | `Height`, `PositionX`, `PositionY`, `Rotation` (degrees), `Width` | reference pixels |
//! | 148 | `IdleBehaviour`[], `TransitionInBehaviour`[], `TransitionOutBehaviour`[] | [`Behaviour`] |
//! | 164 | triggered behaviours[] | [`TriggeredBehaviour`] |
//! | 168 | `TechniqueAssetId` | |
//! | 174 | `Anchor` | 0–8: which point of the rectangle the position names (row-major 3×3, 4 = centre) |
//! | 180 | `TechniqueIndex` | |
//!
//! Derived types add: images `Tint` {`Value` rgba, `Binding`, palette}, `Opacity` {`Binding`, `Value` 0–100},
//! `ImageBinding`, `ImageSource` (texture asset id: the low bits of `TextureHandle`), `FlipHorizontal`,
//! `FlipVertical`; labels `TextProperties` {`ColourBinding`, `TextStyle`, `WordWrap`, `Justification`} and
//! `DisplayText` {`Binding`, Language string id}; backed labels also `Margins`, `MinHeight`, `MinWidth` and a
//! `UISubImage` backing image; shapes `BackgroundColour`, `BackgroundColourPalette`, `BorderWidth`; masks
//! `PopMask` (a mask pushes a clip rectangle for the elements after it, a `PopMask` one ends it; its width /
//! height scale bindings make bar gauges); the unnamed road-map type (`MAP_ELEMENT_TYPE`) eight colours
//! (one is `ShortcutColour`), three map parameters and a `Mask` image.

use mw_bundle::ResourceId;

use crate::genesys::{Object, Value, field_hash};

/// Field name hashes. Names recovered with `mwtool genesys-crack` are spelled out; the rest stay as hashes.
pub mod field {
    use crate::genesys::field_hash;

    pub const ID: u32 = 0x655d_3f5f;
    pub const NAME: u32 = field_hash("Name");
    pub const ELEMENTS: u32 = field_hash("Elements");
    pub const QUAD: u32 = 0x9c8f_13f0;
    pub const TEXTURE_HANDLE: u32 = field_hash("TextureHandle");
    pub const RENDERABLES: u32 = 0x6b73_9c66;
    pub const MATERIAL: u32 = field_hash("Material");
    pub const HEIGHT_SCALE_BINDING: u32 = field_hash("HeightScale_Binding");
    pub const POSITION_X_BINDING: u32 = field_hash("PositionX_Binding");
    pub const POSITION_Y_BINDING: u32 = field_hash("PositionY_Binding");
    pub const ROTATION_BINDING: u32 = field_hash("Rotation_Binding");
    pub const VISIBILITY_BINDING: u32 = field_hash("VisibilityBinding");
    pub const WIDTH_SCALE_BINDING: u32 = field_hash("WidthScale_Binding");
    pub const WIDTH: u32 = field_hash("Width");
    pub const HEIGHT: u32 = field_hash("Height");
    pub const POSITION_X: u32 = field_hash("PositionX");
    pub const POSITION_Y: u32 = field_hash("PositionY");
    pub const ROTATION: u32 = field_hash("Rotation");
    pub const ANCHOR: u32 = field_hash("Anchor");
    pub const IDLE_BEHAVIOUR: u32 = field_hash("IdleBehaviour");
    pub const TRANSITION_IN_BEHAVIOUR: u32 = field_hash("TransitionInBehaviour");
    pub const TRANSITION_OUT_BEHAVIOUR: u32 = field_hash("TransitionOutBehaviour");
    pub const TRIGGERED_BEHAVIOURS: u32 = 0x4020_57b7;
    pub const TINT: u32 = field_hash("Tint");
    pub const VALUE: u32 = field_hash("Value");
    pub const BINDING: u32 = field_hash("Binding");
    pub const COLOUR_PALETTE: u32 = 0x38e2_423f;
    pub const OPACITY: u32 = field_hash("Opacity");
    pub const IMAGE_BINDING: u32 = field_hash("ImageBinding");
    pub const IMAGE_SOURCE: u32 = field_hash("ImageSource");
    pub const FLIP_HORIZONTAL: u32 = field_hash("FlipHorizontal");
    pub const FLIP_VERTICAL: u32 = field_hash("FlipVertical");
    pub const TEXT_PROPERTIES: u32 = field_hash("TextProperties");
    pub const COLOUR_BINDING: u32 = field_hash("ColourBinding");
    pub const TEXT_STYLE: u32 = field_hash("TextStyle");
    pub const WORD_WRAP: u32 = field_hash("WordWrap");
    pub const JUSTIFICATION: u32 = field_hash("Justification");
    pub const DISPLAY_TEXT: u32 = field_hash("DisplayText");
    pub const STRING_ID: u32 = 0xcb33_ade5;
    pub const MARGINS: u32 = field_hash("Margins");
    pub const MIN_WIDTH: u32 = field_hash("MinWidth");
    pub const MIN_HEIGHT: u32 = field_hash("MinHeight");
    pub const SUB_IMAGE: u32 = 0x9459_d928;
    pub const ASSET: u32 = field_hash("Asset");
    pub const BACKGROUND_COLOUR: u32 = field_hash("BackgroundColour");
    pub const BORDER_WIDTH: u32 = field_hash("BorderWidth");
    pub const PARAMETERS: u32 = field_hash("Parameters");
    pub const SOUND: u32 = field_hash("Sound");
    pub const DELAY: u32 = field_hash("Delay");
    pub const DURATION: u32 = field_hash("Duration");
    pub const TYPE: u32 = field_hash("Type");
    pub const EASE: u32 = field_hash("Ease");
    pub const SEQUENCE_NAME: u32 = field_hash("SequenceName");
    pub const TRIGGER_BINDING: u32 = field_hash("TriggerBinding");
    pub const TRIGGER_TYPE: u32 = field_hash("Trigger_Type");
    // TextStyle
    pub const EFFECT_COLOUR: u32 = field_hash("Effect_Colour");
    pub const FONT_COLOUR: u32 = field_hash("Font_Colour");
    pub const FONT_ASSET: u32 = field_hash("Font_Asset");
    pub const FONT_ASSET_ALT: u32 = 0xc455_a691;
    pub const FONT_SIZE: u32 = field_hash("Font_Size");
    pub const FONT_SIZE_2: u32 = 0x9662_94f5;
    pub const CHARACTER_SPACING: u32 = field_hash("Character_Spacing");
    pub const LINE_SPACING: u32 = field_hash("Line_Spacing");
    pub const EFFECT_DISTANCE: u32 = field_hash("EffectDistance");
    pub const EFFECT_STRENGTH: u32 = 0x46a2_f495;
    pub const LOCALISATION_ASSET: u32 = field_hash("Localisation_Asset");
    pub const LOCALES: u32 = field_hash("Locales");
    pub const LANGUAGE: u32 = field_hash("Language");
    // Screens and widgets
    pub const WIDGET_GROUPS: u32 = field_hash("WidgetGroups");
    pub const WIDGETS: u32 = field_hash("Widgets");
    pub const LAYOUTS: u32 = field_hash("Layouts");
    pub const LAYOUT: u32 = field_hash("Layout");
    pub const LAYOUT_INSTANCE_PARAMS: u32 = field_hash("LayoutInstanceParams");
    pub const LAYOUT_LAYER: u32 = field_hash("LayoutLayer");
    pub const ADAPTERS: u32 = 0x58f4_df78;
    // UILayoutInstanceParams
    pub const LAYOUT_TRANSFORMS: u32 = field_hash("Layout_Transforms");
    pub const ANCHOR_TYPE: u32 = field_hash("AnchorType");
    pub const BINDING_REPLACEMENT_INDEX: u32 = field_hash("Binding_Replacement_Index");
    pub const VISIBILITY_TIMELINE: u32 = field_hash("Visibility_Timeline");
    pub const TRANSFORM_VALUE_X: u32 = 0x5e9f_1ee7;
    pub const TRANSFORM_VALUE_Y: u32 = 0x5a5e_0251;
    pub const TRANSFORM_VALUE_Z: u32 = 0x571d_278b;
    pub const TIMELINE_IDLE: u32 = 0xaedf_be15;
    pub const TIMELINE_OFF: u32 = 0x0b60_3df7;
    pub const COLOUR_TRANSFORMS: u32 = field_hash("Colour_Transforms");
    pub const BLEND_MODE: u32 = field_hash("BlendMode");
    // Masks and the road-map element
    pub const POP_MASK: u32 = field_hash("PopMask");
    pub const MASK: u32 = field_hash("Mask");
}

/// Type of the (unnamed) road-map element: the minimap, drawn from the `UIMapGrid` tiles.
pub const MAP_ELEMENT_TYPE: ResourceId = ResourceId(0x0100_0000_0000_2a7d);

/// UI layouts are authored for this frame (reference pixels).
pub const REFERENCE_SIZE: [f32; 2] = [1280.0, 720.0];

fn get(o: &Object, h: u32) -> Option<&Value> {
    o.get(h)
}

fn float(o: &Object, h: u32) -> f32 {
    match get(o, h) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::UInt(u)) => *u as f32,
        _ => 0.0,
    }
}

fn uint(o: &Object, h: u32) -> u64 {
    match get(o, h) {
        Some(Value::UInt(u)) => *u,
        Some(Value::Int(i)) => *i as u64,
        Some(Value::Bool(b)) => *b as u64,
        _ => 0,
    }
}

fn boolean(o: &Object, h: u32) -> bool {
    uint(o, h) != 0
}

fn string(o: &Object, h: u32) -> String {
    match get(o, h) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn handle(o: &Object, h: u32) -> Option<ResourceId> {
    match get(o, h) {
        Some(Value::Handle(id)) => *id,
        Some(Value::Ref(id)) => Some(*id),
        _ => None,
    }
}

fn object(o: &Object, h: u32) -> Option<&Object> {
    match get(o, h) {
        Some(Value::Struct(s)) => Some(s),
        _ => None,
    }
}

fn array(o: &Object, h: u32) -> &[Value] {
    match get(o, h) {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

fn floats<const N: usize>(o: &Object, h: u32) -> [f32; N] {
    let mut out = [0.0; N];
    for (slot, v) in out.iter_mut().zip(array(o, h)) {
        if let Value::Float(f) = v {
            *slot = *f;
        }
    }
    out
}

fn structs(o: &Object, h: u32) -> impl Iterator<Item = &Object> {
    array(o, h).iter().filter_map(|v| if let Value::Struct(s) = v { Some(s) } else { None })
}

/// Resource ID of a GeneSys object named by a UniqueId field (`0x01000000_00xxxxxx`).
pub fn object_id(unique_id: u32) -> ResourceId {
    ResourceId(0x0100_0000_0000_0000 | unique_id as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Image,
    Label,
    BackedLabel,
    Shape,
    Mask,
    ElementStack,
    ScrollingText,
    ScrollableLabel,
    MoviePlayer,
    Hitbox,
    SubImage,
    /// The minimap (`MAP_ELEMENT_TYPE`, no type name).
    Map,
    Other,
}

impl ElementKind {
    fn from_type_name(name: &str) -> Self {
        match name.trim_start_matches("Genesys.Gen.") {
            "UIElement_PrototypeImage" => Self::Image,
            "UIElement_PrototypeLabel" => Self::Label,
            "UIElement_BackedLabel" => Self::BackedLabel,
            "UIElement_PrototypeShape" => Self::Shape,
            "UIElement_Mask" => Self::Mask,
            "UIElement_ElementStack" => Self::ElementStack,
            "UIElement_PrototypeScrollingText" => Self::ScrollingText,
            "UIElement_ScrollableLabel" => Self::ScrollableLabel,
            "UIElement_MoviePlayer" => Self::MoviePlayer,
            "UIElement_PrototypeHitbox" => Self::Hitbox,
            "UISubElement_Image" => Self::SubImage,
            _ => Self::Other,
        }
    }

    pub fn has_text(self) -> bool {
        matches!(self, Self::Label | Self::BackedLabel | Self::ScrollingText | Self::ScrollableLabel)
    }
}

/// A tweened step: `Parameters` is a small command string ("IN", "OUT", "TO,WHITE", "TO,INVISIBLE",
/// "APPROACH (0, 300, 0, -100, EASIN)"), times in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Behaviour {
    pub parameters: String,
    pub sound: u32,
    pub delay: f32,
    pub duration: f32,
    pub kind: u8,
    pub ease: u8,
}

impl Behaviour {
    fn from_object(o: &Object) -> Self {
        Self {
            parameters: string(o, field::PARAMETERS),
            sound: uint(o, field::SOUND) as u32,
            delay: float(o, field::DELAY),
            duration: float(o, field::DURATION),
            kind: uint(o, field::TYPE) as u8,
            ease: uint(o, field::EASE) as u8,
        }
    }

    fn list(o: &Object, h: u32) -> Vec<Self> {
        structs(o, h).map(Self::from_object).collect()
    }
}

/// Behaviours started when a binding changes (`TriggerBinding`).
#[derive(Debug, Clone, PartialEq)]
pub struct TriggeredBehaviour {
    pub sequence: String,
    pub trigger: String,
    pub trigger_type: u8,
    pub idle: Vec<Behaviour>,
    pub transition_in: Vec<Behaviour>,
    pub transition_out: Vec<Behaviour>,
}

/// Bindings: expressions over the game's view models, evaluated by its code.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bindings {
    pub visibility: String,
    pub position_x: String,
    pub position_y: String,
    pub rotation: String,
    pub width_scale: String,
    pub height_scale: String,
    pub tint: String,
    pub opacity: String,
    pub image: String,
    pub text: String,
    pub text_colour: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub style: Option<ResourceId>,
    /// Language string id (0 = none; the text then comes from the binding).
    pub string_id: u32,
    pub word_wrap: bool,
    /// 0 left, 1 centre, 2 right.
    pub justification: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub kind: ElementKind,
    pub type_name: String,
    pub id: u32,
    /// Position of the [`anchor`](Self::anchor) point, reference pixels.
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// Degrees, clockwise.
    pub rotation: f32,
    /// 0–8, row-major 3×3 grid over the rectangle (0 top-left, 4 centre, 8 bottom-right).
    pub anchor: u8,
    pub bindings: Bindings,
    pub texture: Option<ResourceId>,
    pub renderables: Vec<ResourceId>,
    pub material: Option<ResourceId>,
    /// Texture asset id; the texture resource is `0x01010000_00xxxxxx`.
    pub image_source: u32,
    pub tint: [f32; 4],
    pub colour_palette: u32,
    /// 0–1.
    pub opacity: f32,
    pub flip: [bool; 2],
    pub text: Option<Text>,
    pub background: Option<[f32; 4]>,
    pub border_width: u8,
    /// Masks: `false` pushes a clip rectangle for the following elements, `true` (`PopMask`) ends it.
    pub pop_mask: bool,
    /// Backed labels: left, top, right, bottom.
    pub margins: [f32; 4],
    pub min_size: [f32; 2],
    pub sub_image: Option<Box<Element>>,
    /// `UIElement_ElementStack`: a list of layouts repeated over a data source.
    pub stack: Option<Box<Stack>>,
    pub idle: Vec<Behaviour>,
    pub transition_in: Vec<Behaviour>,
    pub transition_out: Vec<Behaviour>,
    pub triggered: Vec<TriggeredBehaviour>,
}

impl Element {
    pub fn from_object(o: &Object) -> Self {
        let kind =
            if o.type_id == MAP_ELEMENT_TYPE { ElementKind::Map } else { ElementKind::from_type_name(&o.type_name) };
        let quad = object(o, field::QUAD);
        let renderables = quad
            .map(|q| {
                array(q, field::RENDERABLES).iter().filter_map(|v| if let Value::Handle(h) = v { *h } else { None })
            })
            .into_iter()
            .flatten()
            .collect();
        let tint = object(o, field::TINT);
        let opacity = object(o, field::OPACITY);
        let text_props = object(o, field::TEXT_PROPERTIES);
        let display = object(o, field::DISPLAY_TEXT);
        let scroll = structs(o, crate::genesys::field_hash("ScrollingStrings")).next();
        let mut margins = [0.0; 4];
        for (m, v) in margins.iter_mut().zip(array(o, field::MARGINS)) {
            if let Value::Float(f) = v {
                *m = *f;
            }
        }
        Self {
            kind,
            type_name: o.type_name.trim_start_matches("Genesys.Gen.").to_string(),
            id: uint(o, field::ID) as u32,
            position: [float(o, field::POSITION_X), float(o, field::POSITION_Y)],
            size: [float(o, field::WIDTH), float(o, field::HEIGHT)],
            rotation: float(o, field::ROTATION),
            anchor: uint(o, field::ANCHOR) as u8,
            bindings: Bindings {
                visibility: string(o, field::VISIBILITY_BINDING),
                position_x: string(o, field::POSITION_X_BINDING),
                position_y: string(o, field::POSITION_Y_BINDING),
                rotation: string(o, field::ROTATION_BINDING),
                width_scale: string(o, field::WIDTH_SCALE_BINDING),
                height_scale: string(o, field::HEIGHT_SCALE_BINDING),
                tint: tint.map(|t| string(t, field::BINDING)).unwrap_or_default(),
                opacity: opacity.map(|t| string(t, field::BINDING)).unwrap_or_default(),
                image: string(o, field::IMAGE_BINDING),
                // A scrolling text's strings are `ScrollingStrings`; with neither binding nor string its
                // `DataSource` supplies the text (the loading strip's `Transitions.Tip`).
                text: display
                    .or(scroll)
                    .map(|t| string(t, field::BINDING))
                    .filter(|b| {
                        !b.is_empty() || display.is_some() || scroll.is_some_and(|s| uint(s, field::STRING_ID) != 0)
                    })
                    .unwrap_or_else(|| {
                        if kind == ElementKind::ScrollingText {
                            string(o, crate::genesys::field_hash("DataSource"))
                        } else {
                            String::new()
                        }
                    }),
                text_colour: text_props.map(|t| string(t, field::COLOUR_BINDING)).unwrap_or_default(),
            },
            texture: quad.and_then(|q| handle(q, field::TEXTURE_HANDLE)),
            renderables,
            material: handle(o, field::MATERIAL),
            image_source: uint(o, field::IMAGE_SOURCE) as u32,
            tint: tint.map_or([1.0; 4], |t| floats(t, field::VALUE)),
            colour_palette: tint.map_or(0, |t| uint(t, field::COLOUR_PALETTE) as u32),
            opacity: opacity.map_or(1.0, |t| float(t, field::VALUE) / 100.0),
            flip: [boolean(o, field::FLIP_HORIZONTAL), boolean(o, field::FLIP_VERTICAL)],
            text: kind.has_text().then(|| Text {
                style: text_props.and_then(|t| handle(t, field::TEXT_STYLE)),
                string_id: display.or(scroll).map_or(0, |d| uint(d, field::STRING_ID) as u32),
                word_wrap: text_props.is_some_and(|t| boolean(t, field::WORD_WRAP)),
                justification: text_props.map_or(0, |t| uint(t, field::JUSTIFICATION) as u8),
            }),
            background: (kind == ElementKind::Shape).then(|| match get(o, field::BACKGROUND_COLOUR) {
                Some(Value::Vector(v)) => *v,
                _ => [1.0; 4],
            }),
            border_width: uint(o, field::BORDER_WIDTH) as u8,
            pop_mask: boolean(o, field::POP_MASK),
            margins,
            min_size: [float(o, field::MIN_WIDTH), float(o, field::MIN_HEIGHT)],
            // Backed labels' backing image, or the minimap's mask image.
            stack: (kind == ElementKind::ElementStack).then(|| Box::new(Stack::from_object(o))),
            sub_image: object(o, field::SUB_IMAGE)
                .or_else(|| object(o, field::MASK))
                .and_then(|s| object(s, field::ASSET))
                .map(|a| Box::new(Element::from_object(a))),
            idle: Behaviour::list(o, field::IDLE_BEHAVIOUR),
            transition_in: Behaviour::list(o, field::TRANSITION_IN_BEHAVIOUR),
            transition_out: Behaviour::list(o, field::TRANSITION_OUT_BEHAVIOUR),
            triggered: structs(o, field::TRIGGERED_BEHAVIOURS)
                .map(|t| TriggeredBehaviour {
                    sequence: string(t, field::SEQUENCE_NAME),
                    trigger: string(t, field::TRIGGER_BINDING),
                    trigger_type: uint(t, field::TRIGGER_TYPE) as u8,
                    idle: Behaviour::list(t, field::IDLE_BEHAVIOUR),
                    transition_in: Behaviour::list(t, field::TRANSITION_IN_BEHAVIOUR),
                    transition_out: Behaviour::list(t, field::TRANSITION_OUT_BEHAVIOUR),
                })
                .collect(),
        }
    }

    /// Top-left corner of the rectangle (reference pixels), from the position and anchor.
    pub fn top_left(&self) -> [f32; 2] {
        let (col, row) = ((self.anchor % 3) as f32, (self.anchor / 3).min(2) as f32);
        [self.position[0] - self.size[0] * col * 0.5, self.position[1] - self.size[1] * row * 0.5]
    }
}

/// `UIElement_ElementStack` (field names from the GeneSys type table; `length` is the unnamed `3555b1f7`,
/// always the data source's `._length`). The stack draws `header`, then one cell per data-source entry (the
/// first template whose binding, with `#` = the entry's index, is true, else `default`), then `footer`. Cell
/// contents are `UILayout` objects (often stacks again: the Easydrive row is a one-cell stack over
/// `~Widget.EasyDrive.Items[#parent_index#]` whose templates pick the item type's layout).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stack {
    /// `DataSource`: the list (or, for a one-cell stack, the item) the cells' relative bindings refer to.
    pub data_source: String,
    /// Binding giving the number of cells; empty = one cell.
    pub length: String,
    pub header: Vec<ResourceId>,
    pub default: Vec<ResourceId>,
    pub footer: Vec<ResourceId>,
    /// (`Binding`, cell layouts) in order.
    pub templates: Vec<(String, Vec<ResourceId>)>,
}

impl Stack {
    fn from_object(o: &Object) -> Self {
        let cells = |o: &Object, name: &str| -> Vec<ResourceId> {
            structs(o, crate::genesys::field_hash(name)).filter_map(|c| handle(c, field_hash_const::CONTENT)).collect()
        };
        Self {
            data_source: string(o, crate::genesys::field_hash("DataSource")),
            length: string(o, 0x3555_b1f7),
            header: cells(o, "Header"),
            default: cells(o, "DefaultTemplate"),
            footer: cells(o, "Footer"),
            templates: structs(o, crate::genesys::field_hash("TemplateLayouts"))
                .map(|t| (string(t, field::BINDING), cells(t, "Cells")))
                .collect(),
        }
    }
}

mod field_hash_const {
    pub const CONTENT: u32 = crate::genesys::field_hash("Content");
}

/// `UILayout`: a flat list of elements.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: u32,
    pub elements: Vec<Element>,
}

impl Layout {
    pub fn from_object(o: &Object) -> Option<Self> {
        if o.type_name != "Genesys.Gen.UILayout" {
            return None;
        }
        Some(Self {
            id: uint(o, field::ID) as u32,
            elements: structs(o, field::ELEMENTS).map(Element::from_object).collect(),
        })
    }
}

/// `TextStyle`: font, size, colour and effect (outline / shadow) of a label. Localised overrides per language.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub id: u32,
    /// Font bundle number (`UI/FONTS/<n>.BNDL`).
    pub font: u32,
    pub size: f32,
    pub size_2: f32,
    pub colour: [f32; 4],
    pub effect_colour: [f32; 4],
    pub effect_distance: f32,
    pub effect_strength: f32,
    pub character_spacing: f32,
    pub line_spacing: f32,
    pub locales: Vec<TextStyleLocale>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyleLocale {
    pub language: u32,
    pub font: u32,
    pub size: f32,
    pub character_spacing: f32,
    pub line_spacing: f32,
    pub effect_distance: f32,
}

fn font_of(o: &Object) -> u32 {
    [field::FONT_ASSET, field::FONT_ASSET_ALT].iter().map(|&h| uint(o, h) as u32).find(|&f| f != 0).unwrap_or(0)
}

impl TextStyle {
    pub fn from_object(o: &Object) -> Option<Self> {
        if o.type_name != "Genesys.Gen.TextStyle" {
            return None;
        }
        let locales = object(o, field::LOCALISATION_ASSET)
            .map(|l| {
                structs(l, field::LOCALES)
                    .map(|loc| TextStyleLocale {
                        language: uint(loc, field::LANGUAGE) as u32,
                        font: font_of(loc),
                        size: float(loc, field::FONT_SIZE),
                        character_spacing: float(loc, field::CHARACTER_SPACING),
                        line_spacing: float(loc, field::LINE_SPACING),
                        effect_distance: float(loc, field::EFFECT_DISTANCE),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id: uint(o, field::ID) as u32,
            font: font_of(o),
            size: float(o, field::FONT_SIZE),
            size_2: float(o, field::FONT_SIZE_2),
            colour: floats(o, field::FONT_COLOUR),
            effect_colour: floats(o, field::EFFECT_COLOUR),
            effect_distance: float(o, field::EFFECT_DISTANCE),
            effect_strength: float(o, field::EFFECT_STRENGTH),
            character_spacing: float(o, field::CHARACTER_SPACING),
            line_spacing: float(o, field::LINE_SPACING),
            locales,
        })
    }
}

/// One component of a layout's placement.
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    /// 1 translate, 2 rotate (degrees), 3 scale.
    pub kind: u16,
    pub value: [f32; 3],
    pub binding: String,
}

/// `UILayoutInstanceParams`: where a widget puts a layout.
///
/// The first translate component is the screen point of the layout's anchor in the 1280×720 frame (for
/// a HUD corner layout `(1280, 720)` with `AnchorType` 8), so on the reference screen the translation is the
/// identity; bindings like `UIConfig.HudAspectCorrection.Right(AXES:X)` move it for other aspect ratios.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutInstance {
    pub id: u32,
    pub anchor_type: u16,
    pub transforms: Vec<Transform>,
    /// Behaviours of the whole layout ("APPROACH (0, 300, 0, -100, EASIN), FADE(0, 200, IN, EASEINOUT)").
    pub timeline: String,
    /// Binding that shows the layout (rarely set: most layouts are shown by the widget's code).
    pub visibility: String,
    /// `Colour_Transforms`: bound colour changes of the whole layout (the gate markers' distance fade).
    pub colours: Vec<ColourTransform>,
    /// Triggered timelines (`402057b7`): `on` plays when the trigger becomes true, `off` when it turns false
    /// (`FADE(0,300,OUT)` / `FADE(0,300,IN)` on `DistanceToCamera(LTEQ:10)`).
    pub timelines: Vec<InstanceTimeline>,
    /// `Binding_Replacement_Index`: what `#` stands for in the layout's bindings (`Transitions.Image#` of the
    /// loading screen's two image layers, 1 and 2).
    pub replacement_index: u32,
}

/// `UILayoutInstanceParams.ColourTransformComponents`.
#[derive(Debug, Clone, PartialEq)]
pub struct ColourTransform {
    pub binding: String,
    pub value: f32,
    pub blend_mode: u16,
    /// 3 (with blend mode 1 and a 0–1 binding) on the gate markers' distance fade.
    pub kind: u16,
}

/// `UILayoutInstanceParams.TimelineParameters` of the triggered list.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceTimeline {
    pub trigger: String,
    /// Played when the trigger becomes true (`aedfbe15`, the field the visibility timeline plays on show).
    pub on: String,
    /// Played when it becomes false (`0b603df7`).
    pub off: String,
}

impl LayoutInstance {
    pub fn from_object(o: &Object) -> Option<Self> {
        if o.type_name != "Genesys.Gen.UILayoutInstanceParams" {
            return None;
        }
        Some(Self {
            id: uint(o, field::ID) as u32,
            anchor_type: uint(o, field::ANCHOR_TYPE) as u16,
            transforms: structs(o, field::LAYOUT_TRANSFORMS)
                .map(|t| Transform {
                    kind: uint(t, field::TYPE) as u16,
                    value: [
                        float(t, field::TRANSFORM_VALUE_X),
                        float(t, field::TRANSFORM_VALUE_Y),
                        float(t, field::TRANSFORM_VALUE_Z),
                    ],
                    binding: string(t, field::BINDING),
                })
                .collect(),
            timeline: object(o, field::VISIBILITY_TIMELINE)
                .map(|t| string(t, field::TIMELINE_IDLE))
                .unwrap_or_default(),
            visibility: object(o, field::VISIBILITY_TIMELINE)
                .map(|t| string(t, field::TRIGGER_BINDING))
                .unwrap_or_default(),
            colours: structs(o, field::COLOUR_TRANSFORMS)
                .map(|c| ColourTransform {
                    binding: string(c, field::BINDING),
                    value: float(c, field::VALUE),
                    blend_mode: uint(c, field::BLEND_MODE) as u16,
                    kind: uint(c, field::TYPE) as u16,
                })
                .collect(),
            timelines: structs(o, field::TRIGGERED_BEHAVIOURS)
                .map(|t| InstanceTimeline {
                    trigger: string(t, field::TRIGGER_BINDING),
                    on: string(t, field::TIMELINE_IDLE),
                    off: string(t, field::TIMELINE_OFF),
                })
                .collect(),
            replacement_index: uint(o, field::BINDING_REPLACEMENT_INDEX) as u32,
        })
    }

    /// The anchor point of the 1280×720 frame named by `anchor_type` (same 3×3 grid as element anchors).
    pub fn anchor_point(&self) -> [f32; 2] {
        let a = self.anchor_type.min(8);
        [REFERENCE_SIZE[0] * (a % 3) as f32 * 0.5, REFERENCE_SIZE[1] * (a / 3) as f32 * 0.5]
    }
}

/// A widget's layout reference.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetLayout {
    pub layout: Option<ResourceId>,
    pub instance: Option<ResourceId>,
}

/// A widget of a screen (`Widget_*`): code-driven, with its layouts.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub name: String,
    pub type_name: String,
    pub id: u32,
    pub layer: u64,
    pub layouts: Vec<WidgetLayout>,
}

/// `Screen2`: widget groups.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub id: u32,
    pub widgets: Vec<Widget>,
}

impl Screen {
    pub fn from_object(o: &Object) -> Option<Self> {
        if o.type_name != "Genesys.Gen.Screen2" {
            return None;
        }
        let widgets = structs(o, field::WIDGET_GROUPS)
            .flat_map(|g| structs(g, field::WIDGETS))
            .map(|w| Widget {
                name: string(w, field::NAME),
                type_name: w.type_name.trim_start_matches("Genesys.Gen.").to_string(),
                id: uint(w, field::ID) as u32,
                layer: uint(w, field::LAYOUT_LAYER),
                layouts: structs(w, field::LAYOUTS)
                    .map(|l| WidgetLayout {
                        layout: handle(l, field::LAYOUT),
                        instance: handle(l, field::LAYOUT_INSTANCE_PARAMS),
                    })
                    .collect(),
            })
            .collect();
        Some(Self { id: uint(o, field::ID) as u32, widgets })
    }
}

/// `PointOfInterestVisuals` (listed by a screen's `Widget_PointOfInterest`, field `f7dfe43b`): how one kind of
/// point of interest is drawn. Game code names a kind by its `key` (`CheckPointsConfig.NormalPoi`, the
/// `MapPoiConfig` fields), not by the object's own UniqueId.
///
/// | field | meaning |
/// |---|---|
/// | `8daa3bb7` `World_Entity` | resource id (0 in every instance) |
/// | `b8a6b50a` | `{Layout, LayoutInstanceParams}`: the icon on the map |
/// | `f50e27b7` | a second `{Layout, Params}` (empty in every instance) |
/// | `655d3f5f` | UniqueId |
/// | `9a5a94d3` | the key |
/// | `bd550893` (count `2300858a`) | `{Layout, Params}` drawn in the world at the point's transform |
/// | `e3256f36`, `8ac61148`, `ea3ab789`, `d788383f`, `db617d4f` | flags (`ea3ab789` set exactly when there are world layouts) |
#[derive(Debug, Clone, PartialEq)]
pub struct PoiVisuals {
    pub id: u32,
    pub key: u32,
    pub map: Option<WidgetLayout>,
    pub world: Vec<WidgetLayout>,
}

impl PoiVisuals {
    pub const TYPE_NAME: &'static str = "Genesys.Gen.PointOfInterestVisuals";
    const MAP: u32 = 0xb8a6_b50a;
    const KEY: u32 = 0x9a5a_94d3;
    const WORLD: u32 = 0xbd55_0893;

    pub fn from_object(o: &Object) -> Option<Self> {
        if o.type_name != Self::TYPE_NAME {
            return None;
        }
        let layout = |l: &Object| {
            let layout = handle(l, field::LAYOUT)?;
            Some(WidgetLayout { layout: Some(layout), instance: handle(l, field::LAYOUT_INSTANCE_PARAMS) })
        };
        Some(Self {
            id: uint(o, field::ID) as u32,
            key: uint(o, Self::KEY) as u32,
            map: object(o, Self::MAP).and_then(layout),
            world: structs(o, Self::WORLD).filter_map(layout).collect(),
        })
    }
}

/// The `Widget_PointOfInterest` field listing a screen's [`PoiVisuals`] (handles).
pub const POI_VISUALS_LIST: u32 = 0xf7df_e43b;

/// UV rectangle of a UI quad renderable: four vertices `{f32 x, f32 y, u32 colour, f16 u, f16 v}` (16 bytes)
/// over the unit square; returns `[u0, v0, u1, v1]` at positions (0,0) and (1,1).
pub fn quad_uv(vertices: &[u8]) -> Option<[f32; 4]> {
    let vert = |i: usize| -> Option<([f32; 2], [f32; 2])> {
        let b = vertices.get(i * 16..i * 16 + 16)?;
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let h = |o: usize| crate::f16_to_f32(u16::from_le_bytes(b[o..o + 2].try_into().unwrap()));
        Some(([f(0), f(4)], [h(12), h(14)]))
    };
    let verts: Vec<_> = (0..4).map(vert).collect::<Option<_>>()?;
    let at =
        |x: f32, y: f32| verts.iter().find(|(p, _)| (p[0] - x).abs() < 1e-3 && (p[1] - y).abs() < 1e-3).map(|v| v.1);
    let (a, b) = (at(0.0, 0.0)?, at(1.0, 1.0)?);
    Some([a[0], a[1], b[0], b[1]])
}

/// Replace `{Binding.Path}` placeholders in a localised string with `f(path)` (e.g. a live value), keeping
/// unknown ones as empty text.
pub fn substitute(text: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => {
                out.push_str(&f(&rest[start + 1..start + end]).unwrap_or_default());
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Compile-time check that the names used above hash as recorded in the install.
const _: () = assert!(field_hash("PositionX") == 0x12d3_a8aa);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors() {
        let mut e = Element::from_object(&Object {
            type_name: "Genesys.Gen.UIElement_PrototypeImage".into(),
            type_id: ResourceId(0),
            fields: vec![],
        });
        e.position = [640.0, 618.0];
        e.size = [60.0, 40.0];
        e.anchor = 4;
        assert_eq!(e.top_left(), [610.0, 598.0]);
        e.anchor = 8;
        assert_eq!(e.top_left(), [580.0, 578.0]);
    }

    #[test]
    fn substitutes_bindings() {
        let s = substitute("{ScoringViewModel.CurrentSpeedPoints}\u{e18f}", |p| {
            (p == "ScoringViewModel.CurrentSpeedPoints").then(|| "1200".into())
        });
        assert_eq!(s, "1200\u{e18f}");
    }

    #[test]
    fn quad_uvs() {
        let mut v = Vec::new();
        for (x, y, u, w) in
            [(0.0f32, 1.0f32, 0.0f32, 0.5f32), (0.0, 0.0, 0.0, 0.0), (1.0, 1.0, 0.25, 0.5), (1.0, 0.0, 0.25, 0.0)]
        {
            v.extend(x.to_le_bytes());
            v.extend(y.to_le_bytes());
            v.extend(u32::MAX.to_le_bytes());
            v.extend(half(u).to_le_bytes());
            v.extend(half(w).to_le_bytes());
        }
        assert_eq!(quad_uv(&v), Some([0.0, 0.0, 0.25, 0.5]));
    }

    fn half(f: f32) -> u16 {
        match f {
            0.0 => 0,
            0.25 => 0x3400,
            0.5 => 0x3800,
            _ => 0x3c00,
        }
    }
}
