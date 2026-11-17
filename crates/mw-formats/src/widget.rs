//! Widget definitions: the JSON `TextFile`s (0x70) of the UI screens (`UI/SCREENS2/*.BNDL`, a few in
//! `EN_US/FEEDBACKGROUPS`), typed. Spec and the code that reads each part: docs/decomp.md "Widget definitions
//! (JSON) and widget scripts"; format notes: docs/formats.md "Widget definitions (JSON)".
//!
//! Every `Widget_*` struct inside a screen's `Screen2` GeneSys object (widget groups → `Widgets`) has a JSON twin
//! named `"<Name>_<UniqueId>.json"`: resource id = CRC-32 of the lower-cased name ([`json_resource_id`],
//! `FUN_007edd50` builds the name with `"%s_%d.json"`). The JSON holds the same data as the GeneSys struct, by
//! field name (the keys hash to the GeneSys field hashes), and is what the widget code reads for its scripts,
//! adapters and view models. The widget's type is not in the JSON; it is the GeneSys struct's type name.
//!
//! Common fields (every widget type; [`WidgetDef`]): `Name`, `LayoutLayer`, `ModalMode`, `Layouts`, `Actions`,
//! `Adapters`, `Scripts`, `Timers`, `DeferredViewModels`, `OnlineViewModels`. Type-specific fields are typed
//! for the types the runtime needs first ([`WidgetKind`]); every other key stays readable in
//! [`WidgetDef::fields`] (file order). [`WidgetDef::unknown_keys`] lists the keys neither part reads.
//!
//! Bindings (`"Widget.Menu.SelectedIndex(EQUALS:2)"`, `"{Mode.Data.Online}"` splices) stay strings here; the
//! binding evaluator parses them.

use mw_bundle::ResourceId;

use crate::json::Json;
use crate::{Error, Result};

/// Resource id of a widget's JSON: CRC-32 of `lower("<name>_<unique_id>.json")`.
pub fn json_resource_id(name: &str, unique_id: u32) -> ResourceId {
    ResourceId::from_name(&json_file_name(name, unique_id))
}

/// `"<name>_<unique_id>.json"` (`FUN_007edd50`).
pub fn json_file_name(name: &str, unique_id: u32) -> String {
    format!("{name}_{unique_id}.json")
}

/// Resource id of a screen script's Lua chunk: CRC-32 of `lower("<script name>.lua")` (all 115 chunks).
pub fn lua_chunk_id(script_name: &str) -> ResourceId {
    ResourceId::from_name(&format!("{script_name}.lua"))
}

/// The 24 native (C++) screen scripts: the factory table at screen manager +0x308 (pairs {`field_hash(name)`,
/// factory}, count at +0x408, room for 32), read from the running game (snapshot `snap1`, table at 0x24473868).
/// `FUN_007f2850` builds a native script when the widget's `Scripts[].Script` has no `LuaAsset` key, looking
/// its `Name` hash up in this table. (name, factory, object size; vtable where the factory sets it directly).
pub const NATIVE_SCREEN_SCRIPTS: &[NativeScreenScript] = &[
    NativeScreenScript::new("MilestoneRecVM", 0x007f_f030, 0xc0),
    NativeScreenScript::new("SessionBestAnim", 0x007f_d830, 0xd8),
    NativeScreenScript::new("POIDisplayScript", 0x007f_f850, 0x7b30),
    NativeScreenScript::new("PlaySequenceFast", 0x007f_d8b0, 0x1e8),
    NativeScreenScript::new("RearViewMirror", 0x007f_d910, 0xc4),
    NativeScreenScript::new("FindMyNearest", 0x007f_d970, 0x2bc),
    NativeScreenScript::new("ExitOnIdleScript", 0x007f_d9d0, 0xe8),
    NativeScreenScript::new("MapCarouselItems", 0x007f_da30, 0x1d14),
    NativeScreenScript::new("MpPlatfmActions", 0x007f_da90, 0xc4),
    NativeScreenScript::new("LiveMapManager", 0x007f_f0f0, 0x26f0),
    NativeScreenScript::new("NotifiCenterVITA", 0x007f_db50, 0x16c),
    NativeScreenScript::new("MWL_VITA", 0x007f_dbb0, 0x270),
    NativeScreenScript::new("EasyDrive", 0x007f_dc20, 0x11a0),
    NativeScreenScript::new("EasyDriveSpeech", 0x007f_dc80, 0x114),
    NativeScreenScript::new("JackspotScript", 0x007f_dce0, 0xe4),
    NativeScreenScript::new("FilterCtrls_VITA", 0x007f_dd40, 0xc8),
    NativeScreenScript::new("ScoreFormat", 0x007f_dda0, 0x17c),
    NativeScreenScript::new("ImageStreamer", 0x007f_de00, 0xcc),
    NativeScreenScript::new("MapFriendsList", 0x007f_f180, 0x710),
    NativeScreenScript::new("ActionSeries", 0x007f_dec0, 0xd4),
    NativeScreenScript::new("Menu_Fast", 0x007f_df20, 0x2d8),
    NativeScreenScript::new("FriendHelperFast", 0x007f_df80, 0x1c0),
    NativeScreenScript::new("FriendsFast", 0x007f_dfe0, 0x2a8),
    NativeScreenScript::new("WaitPhotoDtlFAST", 0x007f_e040, 0x13c),
];

/// One entry of [`NATIVE_SCREEN_SCRIPTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeScreenScript {
    pub name: &'static str,
    /// Factory function in the exe (`(this manager context) -> script object`).
    pub factory: u32,
    /// Size of the script object it allocates.
    pub size: u32,
}

impl NativeScreenScript {
    const fn new(name: &'static str, factory: u32, size: u32) -> Self {
        Self { name, factory, size }
    }

    /// The native script of that name (the lookup compares name hashes, so the case must match).
    pub fn find(name: &str) -> Option<&'static NativeScreenScript> {
        let h = crate::genesys::field_hash(name);
        NATIVE_SCREEN_SCRIPTS.iter().find(|n| crate::genesys::field_hash(n.name) == h)
    }
}

/// `LayoutLayer`: the draw layer of the widget's layouts. The GeneSys twin stores the enum value
/// ([`LayoutLayer::genesys_value`]; every one of the 1,876 screen widgets agrees, `mwtool widgets`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutLayer {
    BackgroundBelowMaps,
    BackgroundAboveMaps,
    Midground,
    Foreground,
    ModalBackground,
    ModalForeground,
    Overlay,
    Other(String),
}

impl LayoutLayer {
    const NAMES: [&'static str; 7] = [
        "BACKGROUND_BELOWMAPS",
        "BACKGROUND_ABOVEMAPS",
        "MIDGROUND",
        "FOREGROUND",
        "MODAL_BACKGROUND",
        "MODAL_FOREGROUND",
        "OVERLAY",
    ];

    pub fn parse(s: &str) -> Self {
        match Self::NAMES.iter().position(|n| *n == s) {
            Some(i) => Self::from_genesys(i as u64),
            None => Self::Other(s.to_string()),
        }
    }

    /// From the GeneSys enum value (0–6).
    pub fn from_genesys(v: u64) -> Self {
        match v {
            0 => Self::BackgroundBelowMaps,
            1 => Self::BackgroundAboveMaps,
            2 => Self::Midground,
            3 => Self::Foreground,
            4 => Self::ModalBackground,
            5 => Self::ModalForeground,
            6 => Self::Overlay,
            o => Self::Other(o.to_string()),
        }
    }

    /// The GeneSys enum value: the draw order, back to front.
    pub fn genesys_value(&self) -> Option<u64> {
        Self::NAMES.iter().position(|n| *n == self.name()).map(|i| i as u64)
    }

    pub fn name(&self) -> &str {
        match self {
            Self::BackgroundBelowMaps => Self::NAMES[0],
            Self::BackgroundAboveMaps => Self::NAMES[1],
            Self::Midground => Self::NAMES[2],
            Self::Foreground => Self::NAMES[3],
            Self::ModalBackground => Self::NAMES[4],
            Self::ModalForeground => Self::NAMES[5],
            Self::Overlay => Self::NAMES[6],
            Self::Other(s) => s,
        }
    }
}

/// `ModalMode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalMode {
    /// `E_NON_MODAL`
    NonModal,
    /// `E_MODAL`
    Modal,
    /// `E_BOTH`
    Both,
    Other(String),
}

impl ModalMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "E_NON_MODAL" => Self::NonModal,
            "E_MODAL" => Self::Modal,
            "E_BOTH" => Self::Both,
            o => Self::Other(o.to_string()),
        }
    }

    /// The GeneSys enum value (1 non-modal, 2 modal, 3 both; agrees on every screen widget).
    pub fn genesys_value(&self) -> Option<u64> {
        match self {
            Self::NonModal => Some(1),
            Self::Modal => Some(2),
            Self::Both => Some(3),
            Self::Other(_) => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::NonModal => "E_NON_MODAL",
            Self::Modal => "E_MODAL",
            Self::Both => "E_BOTH",
            Self::Other(s) => s,
        }
    }
}

/// `Layouts[]`, `MapTileLayouts[]`: a `UILayout` and its `UILayoutInstanceParams` (GeneSys UniqueIds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRef {
    pub layout: u32,
    pub instance_params: u32,
}

/// `Actions[]` (`UserActionInstance`): an input action and what it calls.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// `ActionDefinition`: the `UserActionDefinition` (the input it listens to, its sounds).
    pub definition: u32,
    /// `ActiveBinding`: the action is live only while this holds (empty = always).
    pub active_binding: String,
    /// `SuccessCallback`: the callback's name. `"Script::Fn"` names a function of another script table.
    pub callback: String,
    /// `CallbackArgs`: arguments, split at `,` and passed to a Lua callback as strings.
    pub args: String,
    /// `IsCallbackInLua`: the callback is a function of the widget's Lua script, else a native widget callback
    /// (`ExitGroupVia`, `OnUp`, ...).
    pub in_lua: bool,
    /// `SuccessBinding`: a further condition (empty = none).
    pub success_binding: String,
}

/// `Adapters[]`: a named view-model value built from binding splices (`Widget.<widget>.<Name>`).
#[derive(Debug, Clone, PartialEq)]
pub struct Adapter {
    pub name: String,
    pub splices: Vec<BindingSplice>,
}

/// `BindingSplices[]` (read by `FUN_00a510e0`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BindingSplice {
    /// `dataSources[]` {`source`, `length`} (`FUN_00a50930`).
    pub sources: Vec<DataSource>,
    /// `mappedSources[]` {`source`, `key`, `expectsHash`} (`FUN_00a50860`).
    pub mapped_sources: Vec<MappedSource>,
    /// `dataSourceIndex`: binding choosing which of `sources` is used.
    pub source_index: Option<String>,
    /// `dataSourceOffset`: added to that index.
    pub source_offset: Option<i64>,
    /// `index`: binding of the element index.
    pub index: Option<String>,
    /// `offset`: added to the element index.
    pub offset: Option<i64>,
    /// `numElements` (a string holding a number).
    pub num_elements: Option<String>,
    /// `appendElement`: a binding whose value is appended.
    pub append_element: Option<String>,
    pub respect_length: Option<bool>,
    pub expects_hash: Option<bool>,
    pub wrap: Option<bool>,
    /// `operation` {`Operator`, `Operand`}: AND, OR, ADD, SUBTRACT, MIN, MAX, SUBTRACT_FLOAT.
    pub operation: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataSource {
    pub source: String,
    /// `length`: a number or a binding (`"Signals.GetIntValue[1]"`), as written.
    pub length: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MappedSource {
    pub source: String,
    pub key: String,
    pub expects_hash: bool,
}

/// `Scripts[]`: a script the widget runs. `Script` holds the configuration items passed to
/// `SetConfigurationItem` and, for Lua scripts, `LuaAsset`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptDef {
    /// `Name`: the script's name = its Lua table / chunk name, or the native script's.
    pub name: String,
    /// `Script.LuaAsset` (a UniqueId; its presence selects a Lua script, `FUN_007f2850`).
    pub lua_asset: Option<u32>,
    /// Every `Script` entry (`LuaAsset` included), in file order.
    pub config: Vec<(String, ConfigValue)>,
}

impl ScriptDef {
    /// The Lua chunk's resource id (only meaningful when [`Self::lua_asset`] is set).
    pub fn lua_chunk(&self) -> ResourceId {
        lua_chunk_id(&self.name)
    }

    /// The native script, when there is no `LuaAsset`.
    pub fn native(&self) -> Option<&'static NativeScreenScript> {
        if self.lua_asset.is_some() { None } else { NativeScreenScript::find(&self.name) }
    }

    /// The items `FUN_007ec680` passes to the Lua `SetConfigurationItem(name, value)`: string, int, double
    /// (rounded through f32) and bool entries in file order; arrays and objects are skipped. `LuaAsset` is an
    /// int entry of `Script` like the others and is passed too.
    pub fn lua_configuration(&self) -> Vec<(&str, ConfigValue)> {
        let rounded = |v: &ConfigValue| match v {
            ConfigValue::Double(d) => ConfigValue::Double(*d as f32 as f64),
            o => o.clone(),
        };
        self.config.iter().filter(|(_, v)| v.passed_to_lua()).map(|(k, v)| (k.as_str(), rounded(v))).collect()
    }

    /// A configuration item by name.
    pub fn config(&self, key: &str) -> Option<&ConfigValue> {
        self.config.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// A configuration item's value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    Str(String),
    Int(i64),
    Double(f64),
    Bool(bool),
    /// Arrays and objects (`CollectablesName`): read by native scripts only, never passed to Lua.
    Json(Json),
}

impl ConfigValue {
    fn from_json(j: &Json) -> Self {
        match j {
            Json::Str(s) => Self::Str(s.clone()),
            Json::Int(i) => Self::Int(*i),
            Json::Double(d) => Self::Double(*d),
            Json::Bool(b) => Self::Bool(*b),
            o => Self::Json(o.clone()),
        }
    }

    pub fn passed_to_lua(&self) -> bool {
        !matches!(self, Self::Json(_))
    }
}

/// `DeferredViewModels[]` / `OnlineViewModels[]`: a named request object whose string values may hold
/// `{binding}` splices (`{"getComponent": "photos", "action": "Index", ...}`), read by
/// `FUN_007f5b10` / `FUN_007f5b80` / `FUN_007f5db0` / `FUN_007f5f80` / `FUN_007f72f0` / `FUN_007f7510`.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewModelDef {
    pub name: String,
    pub view_model: Json,
}

/// The fields of one widget definition.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetDef {
    pub name: String,
    pub layout_layer: Option<LayoutLayer>,
    pub modal_mode: Option<ModalMode>,
    pub layouts: Vec<LayoutRef>,
    pub actions: Vec<Action>,
    pub adapters: Vec<Adapter>,
    pub scripts: Vec<ScriptDef>,
    /// `Timers[]`: `ScreenTimer` UniqueIds.
    pub timers: Vec<u32>,
    pub deferred_view_models: Vec<ViewModelDef>,
    pub online_view_models: Vec<ViewModelDef>,
    /// `ImagePalettes[]`: `UIImagePalette` UniqueIds (a base field of every widget type).
    pub image_palettes: Vec<u32>,
    /// Type-specific fields, typed where [`WidgetKind`] knows the widget type.
    pub kind: WidgetKind,
    /// Every key that is not one of the common fields above, in file order (also the ones `kind` reads).
    pub fields: Vec<(String, Json)>,
    /// Keys the parser does not read (neither common nor of `kind`): empty for every shipped definition except
    /// the ones listed by `mwtool widgets --check`.
    pub unknown_keys: Vec<String>,
}

const COMMON_KEYS: &[&str] = &[
    "Name",
    "LayoutLayer",
    "ModalMode",
    "Layouts",
    "Actions",
    "Adapters",
    "Scripts",
    "Timers",
    "DeferredViewModels",
    "OnlineViewModels",
    "ImagePalettes",
];

impl WidgetDef {
    /// Parses a widget JSON. `type_name` is the GeneSys widget type (`"Widget_MenuData"`, with or without the
    /// `Genesys.Gen.` prefix); `None` parses the common fields only.
    pub fn parse(json: &Json, type_name: Option<&str>) -> Result<Self> {
        let Json::Object(entries) = json else { return Err(Error::Malformed("widget json: not an object".into())) };
        let mut r = Fields::new(json);
        let name = r.string("Name")?.unwrap_or_default();
        let layout_layer = r.string("LayoutLayer")?.map(|s| LayoutLayer::parse(&s));
        let modal_mode = r.string("ModalMode")?.map(|s| ModalMode::parse(&s));
        let layouts = r.list("Layouts", layout_ref)?;
        let actions = r.list("Actions", action)?;
        let adapters = r.list("Adapters", adapter)?;
        let scripts = r.list("Scripts", script)?;
        let timers = r.list("Timers", |j| uid(j, "Timers[]"))?;
        let deferred_view_models = r.list("DeferredViewModels", view_model)?;
        let online_view_models = r.list("OnlineViewModels", view_model)?;
        let image_palettes = r.list("ImagePalettes", |j| uid(j, "ImagePalettes[]"))?;
        let type_name = type_name.map(|t| t.trim_start_matches("Genesys.Gen."));
        let kind = WidgetKind::parse(type_name, &mut r)?;
        let fields = entries.iter().filter(|(k, _)| !COMMON_KEYS.contains(&k.as_str())).cloned().collect();
        let unknown_keys = r.unread();
        Ok(Self {
            name,
            layout_layer,
            modal_mode,
            layouts,
            actions,
            adapters,
            scripts,
            timers,
            deferred_view_models,
            online_view_models,
            image_palettes,
            kind,
            fields,
            unknown_keys,
        })
    }

    /// A type-specific field by key.
    pub fn field(&self, key: &str) -> Option<&Json> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// Reads keys of one JSON object and remembers which were read.
struct Fields<'a> {
    obj: &'a Json,
    read: Vec<&'a str>,
}

impl<'a> Fields<'a> {
    fn new(obj: &'a Json) -> Self {
        Self { obj, read: Vec::new() }
    }

    fn get(&mut self, key: &'a str) -> Option<&'a Json> {
        self.read.push(key);
        self.obj.get(key)
    }

    fn string(&mut self, key: &'a str) -> Result<Option<String>> {
        self.get(key).map(|v| str_of(v, key)).transpose()
    }

    fn int(&mut self, key: &'a str) -> Result<Option<i64>> {
        self.get(key).map(|v| v.as_i64().ok_or_else(|| wrong(key, "an int", v))).transpose()
    }

    fn number(&mut self, key: &'a str) -> Result<Option<f64>> {
        self.get(key).map(|v| v.as_f64().ok_or_else(|| wrong(key, "a number", v))).transpose()
    }

    fn uid(&mut self, key: &'a str) -> Result<Option<u32>> {
        self.get(key).map(|v| uid(v, key)).transpose()
    }

    fn boolean(&mut self, key: &'a str) -> Result<Option<bool>> {
        self.get(key).map(|v| v.as_bool().ok_or_else(|| wrong(key, "a bool", v))).transpose()
    }

    fn list<T>(&mut self, key: &'a str, f: impl Fn(&Json) -> Result<T>) -> Result<Vec<T>> {
        match self.get(key) {
            None => Ok(Vec::new()),
            Some(Json::Array(a)) => a.iter().map(f).collect(),
            Some(v) => Err(wrong(key, "an array", v)),
        }
    }

    fn object(&mut self, key: &'a str) -> Result<Option<&'a Json>> {
        match self.get(key) {
            None => Ok(None),
            Some(v @ Json::Object(_)) => Ok(Some(v)),
            Some(v) => Err(wrong(key, "an object", v)),
        }
    }

    /// Keys present but never asked for.
    fn unread(&self) -> Vec<String> {
        self.obj.entries().iter().map(|(k, _)| k).filter(|k| !self.read.contains(&k.as_str())).cloned().collect()
    }

    /// Fails on keys nobody read (for nested records with a fixed shape).
    fn finish(self, what: &str) -> Result<()> {
        match self.unread().first() {
            Some(k) => Err(Error::Malformed(format!("{what}: unknown key {k:?}"))),
            None => Ok(()),
        }
    }
}

fn wrong(key: &str, want: &str, v: &Json) -> Error {
    Error::Malformed(format!("widget json: {key} is a {} (expected {want})", v.kind_name()))
}

fn str_of(v: &Json, key: &str) -> Result<String> {
    v.as_str().map(str::to_string).ok_or_else(|| wrong(key, "a string", v))
}

fn uid(v: &Json, key: &str) -> Result<u32> {
    v.as_i64().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| wrong(key, "a UniqueId", v))
}

fn req<T>(v: Option<T>, what: &str) -> Result<T> {
    v.ok_or_else(|| Error::Malformed(format!("widget json: missing {what}")))
}

fn layout_ref(j: &Json) -> Result<LayoutRef> {
    let mut r = Fields::new(j);
    let layout = req(r.uid("Layout")?, "Layout")?;
    let instance_params = req(r.uid("LayoutInstanceParams")?, "LayoutInstanceParams")?;
    r.finish("Layouts[]")?;
    Ok(LayoutRef { layout, instance_params })
}

fn action(j: &Json) -> Result<Action> {
    let mut r = Fields::new(j);
    let a = Action {
        definition: req(r.uid("ActionDefinition")?, "ActionDefinition")?,
        active_binding: r.string("ActiveBinding")?.unwrap_or_default(),
        callback: r.string("SuccessCallback")?.unwrap_or_default(),
        args: r.string("CallbackArgs")?.unwrap_or_default(),
        in_lua: r.boolean("IsCallbackInLua")?.unwrap_or(false),
        success_binding: r.string("SuccessBinding")?.unwrap_or_default(),
    };
    r.finish("Actions[]")?;
    Ok(a)
}

fn adapter(j: &Json) -> Result<Adapter> {
    let mut r = Fields::new(j);
    let name = r.string("Name")?.unwrap_or_default();
    let splices = r.list("BindingSplices", splice)?;
    r.finish("Adapters[]")?;
    Ok(Adapter { name, splices })
}

fn splice(j: &Json) -> Result<BindingSplice> {
    let mut r = Fields::new(j);
    let s = BindingSplice {
        sources: r.list("dataSources", |j| {
            let mut r = Fields::new(j);
            let d = DataSource {
                source: r.string("source")?.unwrap_or_default(),
                length: r.get("length").map(|v| match v {
                    Json::Int(i) => i.to_string(),
                    Json::Str(s) => s.clone(),
                    o => o.kind_name().to_string(),
                }),
            };
            r.finish("dataSources[]")?;
            Ok(d)
        })?,
        mapped_sources: r.list("mappedSources", |j| {
            let mut r = Fields::new(j);
            let m = MappedSource {
                source: r.string("source")?.unwrap_or_default(),
                key: r.string("key")?.unwrap_or_default(),
                expects_hash: r.boolean("expectsHash")?.unwrap_or(false),
            };
            r.finish("mappedSources[]")?;
            Ok(m)
        })?,
        source_index: r.string("dataSourceIndex")?,
        source_offset: r.int("dataSourceOffset")?,
        index: r.string("index")?,
        offset: r.int("offset")?,
        num_elements: r.string("numElements")?,
        append_element: r.string("appendElement")?,
        respect_length: r.boolean("respectLength")?,
        expects_hash: r.boolean("expectsHash")?,
        wrap: r.boolean("wrap")?,
        operation: match r.object("operation")? {
            Some(o) => {
                let mut r = Fields::new(o);
                let op = (r.string("Operator")?.unwrap_or_default(), r.string("Operand")?.unwrap_or_default());
                r.finish("operation")?;
                Some(op)
            }
            None => None,
        },
    };
    r.finish("BindingSplices[]")?;
    Ok(s)
}

fn script(j: &Json) -> Result<ScriptDef> {
    let mut r = Fields::new(j);
    let name = r.string("Name")?.unwrap_or_default();
    let body = r.object("Script")?;
    r.finish("Scripts[]")?;
    let mut lua_asset = None;
    let mut config = Vec::new();
    for (k, v) in body.map(Json::entries).unwrap_or_default() {
        if k == "LuaAsset" && lua_asset.is_none() {
            lua_asset = Some(uid(v, "LuaAsset")?);
        }
        config.push((k.clone(), ConfigValue::from_json(v)));
    }
    Ok(ScriptDef { name, lua_asset, config })
}

fn view_model(j: &Json) -> Result<ViewModelDef> {
    let mut r = Fields::new(j);
    let name = r.string("Name")?.unwrap_or_default();
    let view_model = r.object("ViewModel")?.cloned().unwrap_or(Json::Object(Vec::new()));
    r.finish("view model")?;
    Ok(ViewModelDef { name, view_model })
}

/// Type-specific fields of the widget types the runtime needs first. Other types are [`WidgetKind::Other`]
/// (their fields stay in [`WidgetDef::fields`]).
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetKind {
    /// `Widget_Default`, `Widget_ShowLayouts`, `Widget_RunScript`: no own fields.
    Plain,
    /// `Widget_PlaySequence`: `Sequence` (a `Sequence` UniqueId).
    PlaySequence { sequence: Option<u32> },
    /// `Widget_MenuData`, `Widget_CarSelectUpgradeSlots`: `CustomItems`, `Title`.
    MenuData { title: Option<u32>, items: Vec<MenuItem> },
    /// `Widget_Title`: `Title`, `CustomTitle` (Language ids), `TitleBinding`.
    Title { title: Option<u32>, custom_title: Option<u32>, binding: Option<String> },
    /// `Widget_StringPalette` / `Widget_StringSelector`: `Strings` (Language ids).
    StringPalette { strings: Vec<u32> },
    /// `Widget_ImagePalette`: `Image` (texture ids).
    ImagePalette { images: Vec<u32> },
    /// `Widget_SequencePalette`: `Sequences` (UniqueIds).
    SequencePalette { sequences: Vec<u32> },
    /// `Widget_HudSelection`: `ExternalDefault`, `ExternalEasyDriveOff` (UniqueIds of widget groups / layouts
    /// the HUD shows by default and with Easydrive closed; meaning from the names, the reader is not traced).
    HudSelection { external_default: Option<u32>, external_easydrive_off: Option<u32> },
    /// `Widget_WindowedMenu`: `WindowSize` (rows shown), `Title`.
    WindowedMenu { window_size: Option<i64>, title: Option<u32> },
    /// `Widget_DlcMessage`: `Entitlement`.
    DlcMessage { entitlement: Option<i64> },
    /// `Widget_Alert`: `Sequence`, `IsActionable`.
    Alert { sequence: Option<u32>, actionable: bool },
    /// `Widget_Map`: `InterestingPoiTypes` (`PointOfInterestVisuals` keys).
    Map { interesting_poi_types: Vec<u32> },
    /// `Widget_CarSelectMenu`: property aliases `NameAlias`, `IsLockedAlias`, `IsNewAlias`.
    CarSelectMenu { name_alias: String, is_locked_alias: String, is_new_alias: String },
    /// `Widget_NavigationTabs`: `Tabs`, `VITATabs`.
    NavigationTabs { tabs: Vec<Tab>, vita_tabs: Vec<Tab> },
    /// `Widget_EasyDriveTierDefinition`: `HsmTransitionName`, `Title`, `Sources`.
    EasyDriveTier { hsm_transition: String, title: Option<u32>, sources: Vec<TierSource> },
    /// `Widget_EasyDriveLinkItemList` / `Widget_EasyDriveHookItemList` / `Widget_EastDriveRpcItemList`
    /// (sic): `Entries`.
    EasyDriveItems { entries: Vec<EasyDriveEntry> },
    /// `Widget_EasyDriveTypeDefinition`: `ExposedProperties`, `Callbacks`, `HighlightCallbacks`,
    /// `CallbackEnabledProperty`, `SpeechCommandProperty`, `HighlightDelay`.
    EasyDriveType {
        properties: Vec<(String, String)>,
        callbacks: Vec<Callback>,
        highlight_callbacks: Vec<Callback>,
        callback_enabled_property: String,
        speech_command_property: String,
        highlight_delay: Option<i64>,
    },
    /// `Widget_ActionTriggerSeries`: `ScriptedActions`.
    ActionTriggerSeries { actions: Vec<ScriptedAction> },
    /// `Widget_StreamedImageSet`: `ImageSources`, `SourceName`.
    StreamedImageSet { sources: Vec<(String, String)>, source_name: String },
    /// The settings lists (`Widget_AudioSettingsList`, `Widget_GameplaySettingsList`, `Widget_DisplaySettingsList`,
    /// `Widget_GraphicsSettingList`, `Widget_PostFxSettingsList`, `Widget_AccountSettingsList`,
    /// `Widget_AdditionalSettings`): `Settings`, `OptionMap` (a list, or one object).
    SettingsList { settings: Vec<Setting>, option_map: Vec<(String, String)> },
    /// `Widget_SettingsCategoryList`, `Widget_AwardCategories`, `Widget_AwardCategoriesDLC`: `Categories`.
    Categories { categories: Vec<Category> },
    /// `Widget_PointOfInterest` (map and minimap widgets): what is drawn and the map camera.
    PointOfInterest {
        draw_ar_layouts: bool,
        draw_map_layouts: bool,
        draw_world_layouts: bool,
        enable_clamping: bool,
        /// `MarginProportion` (4 values).
        margin_proportion: Vec<f64>,
        map: Option<Box<MapConfiguration>>,
        tile_layouts: Vec<LayoutRef>,
        /// `PointsOfInterest`: `PointOfInterestVisuals` UniqueIds.
        points_of_interest: Vec<u32>,
    },
    /// `Widget_MapFilters`: `Filters`, `DefaultFilterIndex`, `SpeechAction`, `SpeechWidget`.
    MapFilters { filters: Vec<MapFilter>, default_index: Option<i64>, speech_action: String, speech_widget: String },
    /// `Widget_LanguageSelect`: `Languages` {`Language`, `FlagImage`}.
    LanguageSelect { languages: Vec<(String, u32)> },
    /// `Widget_CarStats`: `Stats` {`Name` (Language id), `ValueBinding`}.
    CarStats { stats: Vec<(u32, String)> },
    /// The JSON twin of a sequence's `SequenceItems.AdapterHost` item (type name `"SequenceItems.AdapterHost"`):
    /// `EnabledBinding`, `EnabledBindingEvaluationMode` (`Constantly` / `On Entry`; GeneSys `84531672` 0 / 2),
    /// `StartTime`, `EndTime` (s), `Group`, `IsLocked`; its `Adapters` are the common field.
    AdapterHost { enabled_binding: String, evaluation_mode: String, start: f64, end: f64, group: String, locked: bool },
    /// Any other type (or no type given).
    Other,
}

/// `CustomItems[]` of `Widget_MenuData`.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// `Action`: the token the menu script acts on (`RESUME`, `TO_SETTINGS`, `Map:OnSetWaypoint`, ...).
    pub action: String,
    pub active: bool,
    /// `Name` (Language id).
    pub name: Option<u32>,
    /// `Description` (Language id).
    pub description: Option<u32>,
    /// `DescriptionBinding`: a context token (`NO_CONTEXT`, ...).
    pub description_binding: String,
    /// `Icon` (texture id).
    pub icon: Option<u32>,
    /// `UpgradeSlot` (`PowerTrain`, `Nitrous`, ...).
    pub upgrade_slot: Option<String>,
}

/// `Tabs[]` / `VITATabs[]` of `Widget_NavigationTabs`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub tab_name: String,
    pub name: Option<u32>,
    pub image: Option<u32>,
    pub key: String,
}

/// `Sources[]` of `Widget_EasyDriveTierDefinition`.
#[derive(Debug, Clone, PartialEq)]
pub struct TierSource {
    pub item_type: String,
    pub path: String,
    pub length_binding: String,
    pub active_binding: String,
    pub changes_length: bool,
}

/// `Entries[]` of the Easydrive item lists (each list uses a subset).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EasyDriveEntry {
    pub name: Option<u32>,
    pub description: Option<u32>,
    pub token: Option<String>,
    pub speech_command: Option<String>,
    pub argument: Option<String>,
    pub hsm_exit_point: Option<String>,
    pub hsm_token: Option<String>,
    pub rpc: Option<String>,
}

/// `Callbacks[]` / `HighlightCallbacks[]` of `Widget_EasyDriveTypeDefinition`.
#[derive(Debug, Clone, PartialEq)]
pub struct Callback {
    /// `CallbackType`: `HsmTransition`, `HsmHook`, `RpcCall`.
    pub callback_type: String,
    pub name: String,
    /// `NameType`: `String` (the name as written) or `PropertyValue` (the item's property of that name).
    pub name_type: String,
    pub argument: String,
    /// `ArgumentType`: `String`, `ID`, `PropertyValue`.
    pub argument_type: String,
    pub enabled_property: String,
}

/// `ScriptedActions[]` of `Widget_ActionTriggerSeries`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedAction {
    pub action: Action,
    pub delay: Option<i64>,
    pub repeat_count: Option<i64>,
    pub stop_condition: String,
}

impl WidgetKind {
    fn parse<'a>(type_name: Option<&str>, r: &mut Fields<'a>) -> Result<Self> {
        let Some(t) = type_name else { return Ok(Self::Other) };
        Ok(match t {
            "Widget_Default" | "Widget_ShowLayouts" | "Widget_RunScript" => Self::Plain,
            "Widget_PlaySequence" => Self::PlaySequence { sequence: r.uid("Sequence")? },
            "Widget_MenuData" | "Widget_CarSelectUpgradeSlots" => {
                Self::MenuData { title: r.uid("Title")?, items: r.list("CustomItems", menu_item)? }
            }
            "Widget_Title" => Self::Title {
                title: r.uid("Title")?,
                custom_title: r.uid("CustomTitle")?,
                binding: r.string("TitleBinding")?,
            },
            "Widget_StringPalette" | "Widget_StringSelector" => {
                Self::StringPalette { strings: r.list("Strings", |j| uid(j, "Strings[]"))? }
            }
            "Widget_ImagePalette" => Self::ImagePalette { images: r.list("Image", |j| uid(j, "Image[]"))? },
            "Widget_SequencePalette" => {
                Self::SequencePalette { sequences: r.list("Sequences", |j| uid(j, "Sequences[]"))? }
            }
            "Widget_HudSelection" => Self::HudSelection {
                external_default: r.uid("ExternalDefault")?,
                external_easydrive_off: r.uid("ExternalEasyDriveOff")?,
            },
            "Widget_WindowedMenu" => Self::WindowedMenu { window_size: r.int("WindowSize")?, title: r.uid("Title")? },
            "Widget_DlcMessage" => Self::DlcMessage { entitlement: r.int("Entitlement")? },
            "Widget_Alert" => {
                Self::Alert { sequence: r.uid("Sequence")?, actionable: r.boolean("IsActionable")?.unwrap_or(false) }
            }
            "Widget_Map" => {
                Self::Map { interesting_poi_types: r.list("InterestingPoiTypes", |j| uid(j, "InterestingPoiTypes[]"))? }
            }
            "Widget_CarSelectMenu" => Self::CarSelectMenu {
                name_alias: r.string("NameAlias")?.unwrap_or_default(),
                is_locked_alias: r.string("IsLockedAlias")?.unwrap_or_default(),
                is_new_alias: r.string("IsNewAlias")?.unwrap_or_default(),
            },
            "Widget_NavigationTabs" => {
                Self::NavigationTabs { tabs: r.list("Tabs", tab)?, vita_tabs: r.list("VITATabs", tab)? }
            }
            "Widget_EasyDriveTierDefinition" => Self::EasyDriveTier {
                hsm_transition: r.string("HsmTransitionName")?.unwrap_or_default(),
                title: r.uid("Title")?,
                sources: r.list("Sources", tier_source)?,
            },
            "Widget_EasyDriveLinkItemList" | "Widget_EasyDriveHookItemList" | "Widget_EastDriveRpcItemList" => {
                Self::EasyDriveItems { entries: r.list("Entries", easydrive_entry)? }
            }
            "Widget_EasyDriveTypeDefinition" => Self::EasyDriveType {
                properties: r.list("ExposedProperties", |j| {
                    let mut r = Fields::new(j);
                    let p = (r.string("Name")?.unwrap_or_default(), r.string("Path")?.unwrap_or_default());
                    r.finish("ExposedProperties[]")?;
                    Ok(p)
                })?,
                callbacks: r.list("Callbacks", callback)?,
                highlight_callbacks: r.list("HighlightCallbacks", callback)?,
                callback_enabled_property: r.string("CallbackEnabledProperty")?.unwrap_or_default(),
                speech_command_property: r.string("SpeechCommandProperty")?.unwrap_or_default(),
                highlight_delay: r.int("HighlightDelay")?,
            },
            "Widget_ActionTriggerSeries" => {
                Self::ActionTriggerSeries { actions: r.list("ScriptedActions", scripted_action)? }
            }
            "Widget_StreamedImageSet" => Self::StreamedImageSet {
                sources: r.list("ImageSources", |j| {
                    let mut r = Fields::new(j);
                    let p = (r.string("ImageIds")?.unwrap_or_default(), r.string("Length")?.unwrap_or_default());
                    r.finish("ImageSources[]")?;
                    Ok(p)
                })?,
                source_name: r.string("SourceName")?.unwrap_or_default(),
            },
            "Widget_AudioSettingsList"
            | "Widget_GameplaySettingsList"
            | "Widget_DisplaySettingsList"
            | "Widget_GraphicsSettingList"
            | "Widget_PostFxSettingsList"
            | "Widget_AccountSettingsList"
            | "Widget_AdditionalSettings" => Self::SettingsList {
                settings: r.list("Settings", setting)?,
                option_map: match r.get("OptionMap") {
                    None => Vec::new(),
                    Some(Json::Array(a)) => a.iter().map(option_pair).collect::<Result<_>>()?,
                    Some(o) => vec![option_pair(o)?],
                },
            },
            "Widget_SettingsCategoryList" | "Widget_AwardCategories" | "Widget_AwardCategoriesDLC" => {
                Self::Categories { categories: r.list("Categories", category)? }
            }
            "Widget_PointOfInterest" => Self::PointOfInterest {
                draw_ar_layouts: r.boolean("DrawARLayouts")?.unwrap_or(false),
                draw_map_layouts: r.boolean("DrawMapLayouts")?.unwrap_or(false),
                draw_world_layouts: r.boolean("DrawWorldLayouts")?.unwrap_or(false),
                enable_clamping: r.boolean("EnableClamping")?.unwrap_or(false),
                margin_proportion: r
                    .get("MarginProportion")
                    .map(|v| numbers(v, "MarginProportion"))
                    .transpose()?
                    .unwrap_or_default(),
                map: r.object("Map configuration")?.map(map_configuration).transpose()?.map(Box::new),
                tile_layouts: r.list("MapTileLayouts", layout_ref)?,
                points_of_interest: r.list("PointsOfInterest", |j| uid(j, "PointsOfInterest[]"))?,
            },
            "Widget_MapFilters" => Self::MapFilters {
                filters: r.list("Filters", map_filter)?,
                default_index: r.int("DefaultFilterIndex")?,
                speech_action: r.string("SpeechAction")?.unwrap_or_default(),
                speech_widget: r.string("SpeechWidget")?.unwrap_or_default(),
            },
            "Widget_LanguageSelect" => Self::LanguageSelect {
                languages: r.list("Languages", |j| {
                    let mut r = Fields::new(j);
                    let l = (r.string("Language")?.unwrap_or_default(), r.uid("FlagImage")?.unwrap_or(0));
                    r.finish("Languages[]")?;
                    Ok(l)
                })?,
            },
            "Widget_CarStats" => Self::CarStats {
                stats: r.list("Stats", |j| {
                    let mut r = Fields::new(j);
                    let s = (r.uid("Name")?.unwrap_or(0), r.string("ValueBinding")?.unwrap_or_default());
                    r.finish("Stats[]")?;
                    Ok(s)
                })?,
            },
            "SequenceItems.AdapterHost" => Self::AdapterHost {
                enabled_binding: r.string("EnabledBinding")?.unwrap_or_default(),
                evaluation_mode: r.string("EnabledBindingEvaluationMode")?.unwrap_or_default(),
                start: r.number("StartTime")?.unwrap_or(0.0),
                end: r.number("EndTime")?.unwrap_or(0.0),
                group: r.string("Group")?.unwrap_or_default(),
                locked: r.boolean("IsLocked")?.unwrap_or(false),
            },
            _ => Self::Other,
        })
    }
}

/// `Settings[]` of the settings lists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setting {
    /// `Setting`: the option's key (`VOL_MUSIC`, `Speed_Units`, ...).
    pub setting: String,
    /// `Type`: `OPTION`, `CONTINUOUS`, `DISCRETE_DATA_ONLY`, `DISCRETE_RUNTIME_ONLY`, `DISCRETE_WITH_RUNTIME`, ...
    pub setting_type: String,
    pub display_name: Option<u32>,
    pub name: Option<u32>,
    pub dialog: Option<u32>,
    pub initial_value: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub step: Option<i64>,
    pub needs_apply: bool,
    /// `Options`: Language ids of the choices.
    pub options: Vec<u32>,
}

/// `Categories[]` (settings categories and award categories use different subsets).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Category {
    pub category_type: Option<String>,
    pub category: Option<String>,
    pub award_category: Option<String>,
    pub name: Option<u32>,
    pub display_name: Option<u32>,
    pub description: Option<u32>,
    pub data_source: Option<String>,
    pub exit_point: Option<String>,
    pub progress: Option<String>,
    pub friend_progress: Option<String>,
    pub release: Option<String>,
    pub id: Option<String>,
    pub can_select: Option<bool>,
    pub requires_online: Option<bool>,
}

/// A value with an optional binding that overrides it (`{Binding, Value}`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoundValue {
    pub binding: String,
    /// One number, or a vector (`LookAt` 3, `ScreenLookAtOffset` 2).
    pub value: Vec<f64>,
}

/// `Map configuration` of `Widget_PointOfInterest`: the map camera.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapConfiguration {
    pub distance: BoundValue,
    pub field_of_view: BoundValue,
    pub look_ahead: BoundValue,
    pub look_at: BoundValue,
    pub look_at_rotation: BoundValue,
    pub projection_angle: BoundValue,
    pub screen_look_at_offset: BoundValue,
    pub near_clip: f64,
    pub far_clip: f64,
    pub margin_proportion: Vec<f64>,
    pub rotate_with_player: bool,
    pub update_look_at_position: bool,
    pub visible_proportion: f64,
    /// `Shape` (`SQUARE`).
    pub shape: String,
}

/// `Filters[]` of `Widget_MapFilters`.
#[derive(Debug, Clone, PartialEq)]
pub struct MapFilter {
    pub filter_type: String,
    pub name: Option<u32>,
    pub custom_filter_data: Option<u32>,
    pub speech_command: String,
}

fn setting(j: &Json) -> Result<Setting> {
    let mut r = Fields::new(j);
    let s = Setting {
        setting: r.string("Setting")?.unwrap_or_default(),
        setting_type: r.string("Type")?.unwrap_or_default(),
        display_name: r.uid("DisplayName")?,
        name: r.uid("Name")?,
        dialog: r.uid("Dialog")?,
        initial_value: r.int("InitialValue")?,
        min: r.int("Min")?,
        max: r.int("Max")?,
        step: r.int("Step")?,
        needs_apply: r.boolean("NeedsApply")?.unwrap_or(false),
        options: r.list("Options", |j| uid(j, "Options[]"))?,
    };
    r.finish("Settings[]")?;
    Ok(s)
}

fn option_pair(j: &Json) -> Result<(String, String)> {
    let mut r = Fields::new(j);
    let p = (r.string("Setting")?.unwrap_or_default(), r.string("Data")?.unwrap_or_default());
    r.finish("OptionMap")?;
    Ok(p)
}

fn category(j: &Json) -> Result<Category> {
    let mut r = Fields::new(j);
    let c = Category {
        category_type: r.string("Type")?,
        category: r.string("Category")?,
        award_category: r.string("AwardCategory")?,
        name: r.uid("Name")?,
        display_name: r.uid("DisplayName")?,
        description: r.uid("Description")?,
        data_source: r.string("DataSource")?,
        exit_point: r.string("ExitPoint")?,
        progress: r.string("Progress")?,
        friend_progress: r.string("FriendProgress")?,
        release: r.string("Release")?,
        id: r.string("ID")?,
        can_select: r.boolean("CanSelect")?,
        requires_online: r.boolean("RequiresOnline")?,
    };
    r.finish("Categories[]")?;
    Ok(c)
}

fn numbers(j: &Json, key: &str) -> Result<Vec<f64>> {
    match j {
        Json::Array(a) => a.iter().map(|v| v.as_f64().ok_or_else(|| wrong(key, "a number", v))).collect(),
        v => Ok(vec![v.as_f64().ok_or_else(|| wrong(key, "a number", v))?]),
    }
}

fn bound_value<'a>(r: &mut Fields<'a>, key: &'static str) -> Result<BoundValue> {
    let Some(o) = r.object(key)? else { return Ok(BoundValue::default()) };
    let mut r = Fields::new(o);
    let b = BoundValue {
        binding: r.string("Binding")?.unwrap_or_default(),
        value: r.get("Value").map(|v| numbers(v, key)).transpose()?.unwrap_or_default(),
    };
    r.finish(key)?;
    Ok(b)
}

fn map_configuration(j: &Json) -> Result<MapConfiguration> {
    let mut r = Fields::new(j);
    let m = MapConfiguration {
        distance: bound_value(&mut r, "Distance")?,
        field_of_view: bound_value(&mut r, "FieldOfView")?,
        look_ahead: bound_value(&mut r, "LookAhead")?,
        look_at: bound_value(&mut r, "LookAt")?,
        look_at_rotation: bound_value(&mut r, "LookAtRotation")?,
        projection_angle: bound_value(&mut r, "ProjectionAngle")?,
        screen_look_at_offset: bound_value(&mut r, "ScreenLookAtOffset")?,
        near_clip: r.number("NearClip")?.unwrap_or(0.0),
        far_clip: r.number("FarClip")?.unwrap_or(0.0),
        margin_proportion: r
            .get("MarginProportion")
            .map(|v| numbers(v, "MarginProportion"))
            .transpose()?
            .unwrap_or_default(),
        rotate_with_player: r.boolean("RotateWithPlayer")?.unwrap_or(false),
        update_look_at_position: r.boolean("UpdateLookAtPosition")?.unwrap_or(false),
        visible_proportion: r.number("VisibleProportion")?.unwrap_or(0.0),
        shape: r.string("Shape")?.unwrap_or_default(),
    };
    r.finish("Map configuration")?;
    Ok(m)
}

fn map_filter(j: &Json) -> Result<MapFilter> {
    let mut r = Fields::new(j);
    let f = MapFilter {
        filter_type: r.string("FilterType")?.unwrap_or_default(),
        name: r.uid("FilterName")?,
        custom_filter_data: r.uid("CustomFilterData")?,
        speech_command: r.string("SpeechCommand")?.unwrap_or_default(),
    };
    r.finish("Filters[]")?;
    Ok(f)
}

fn menu_item(j: &Json) -> Result<MenuItem> {
    let mut r = Fields::new(j);
    let m = MenuItem {
        action: r.string("Action")?.unwrap_or_default(),
        active: r.boolean("Active")?.unwrap_or(false),
        name: r.uid("Name")?,
        description: r.uid("Description")?,
        description_binding: r.string("DescriptionBinding")?.unwrap_or_default(),
        icon: r.uid("Icon")?,
        upgrade_slot: r.string("UpgradeSlot")?,
    };
    r.finish("CustomItems[]")?;
    Ok(m)
}

fn tab(j: &Json) -> Result<Tab> {
    let mut r = Fields::new(j);
    let t = Tab {
        tab_name: r.string("TabName")?.unwrap_or_default(),
        name: r.uid("Name")?,
        image: r.uid("Image")?,
        key: r.string("Key")?.unwrap_or_default(),
    };
    r.finish("Tabs[]")?;
    Ok(t)
}

fn tier_source(j: &Json) -> Result<TierSource> {
    let mut r = Fields::new(j);
    let s = TierSource {
        item_type: r.string("Type")?.unwrap_or_default(),
        path: r.string("Path")?.unwrap_or_default(),
        length_binding: r.string("LengthBinding")?.unwrap_or_default(),
        active_binding: r.string("ActiveBinding")?.unwrap_or_default(),
        changes_length: r.boolean("ChangesLength")?.unwrap_or(false),
    };
    r.finish("Sources[]")?;
    Ok(s)
}

fn easydrive_entry(j: &Json) -> Result<EasyDriveEntry> {
    let mut r = Fields::new(j);
    let e = EasyDriveEntry {
        name: r.uid("Name")?,
        description: r.uid("Description")?,
        token: r.string("Token")?,
        speech_command: r.string("SpeechCommand")?,
        argument: r.string("Argument")?,
        hsm_exit_point: r.string("HsmExitPoint")?,
        hsm_token: r.string("HsmToken")?,
        rpc: r.string("Rpc")?,
    };
    r.finish("Entries[]")?;
    Ok(e)
}

fn callback(j: &Json) -> Result<Callback> {
    let mut r = Fields::new(j);
    let c = Callback {
        callback_type: r.string("CallbackType")?.unwrap_or_default(),
        name: r.string("Name")?.unwrap_or_default(),
        name_type: r.string("NameType")?.unwrap_or_default(),
        argument: r.string("Argument")?.unwrap_or_default(),
        argument_type: r.string("ArgumentType")?.unwrap_or_default(),
        enabled_property: r.string("EnabledProperty")?.unwrap_or_default(),
    };
    r.finish("Callbacks[]")?;
    Ok(c)
}

fn scripted_action(j: &Json) -> Result<ScriptedAction> {
    let mut r = Fields::new(j);
    let action = action(req(r.object("Action")?, "ScriptedActions[].Action")?)?;
    let s = ScriptedAction {
        action,
        delay: r.int("Delay")?,
        repeat_count: r.int("RepeatCount")?,
        stop_condition: r.string("StopCondition")?.unwrap_or_default(),
    };
    r.finish("ScriptedActions[]")?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        // Checked against the install: UI/SCREENS2/1128433.BNDL holds these TextFiles.
        assert_eq!(json_resource_id("TitleScreen", 1128436), ResourceId(0xca32_f2c5));
        assert_eq!(json_resource_id("Background", 1022356), ResourceId(0xfd9e_3727));
        assert_eq!(lua_chunk_id("PlaySequence"), ResourceId(0x3ca0_27a0));
        assert_eq!(NativeScreenScript::find("EasyDrive").map(|n| n.factory), Some(0x007f_dc20));
    }

    #[test]
    fn widget() {
        let j = Json::parse(
            r#"{"LayoutLayer": "BACKGROUND_ABOVEMAPS", "Layouts": [{"Layout": 1689284, "LayoutInstanceParams": 264486}],
            "ModalMode": "E_NON_MODAL", "Name": "Background",
            "Scripts": [{"Name": "PlaySequence", "Script": {"ConductorName": "PressStart", "LuaAsset": 264693,
            "Speed": 0.1, "List": [1]}}], "Sequence": 1022357}"#,
        )
        .unwrap();
        let w = WidgetDef::parse(&j, Some("Genesys.Gen.Widget_PlaySequence")).unwrap();
        assert_eq!(w.layout_layer, Some(LayoutLayer::BackgroundAboveMaps));
        assert_eq!(w.kind, WidgetKind::PlaySequence { sequence: Some(1022357) });
        assert!(w.unknown_keys.is_empty());
        let s = &w.scripts[0];
        assert_eq!(s.lua_asset, Some(264693));
        let cfg = s.lua_configuration();
        assert_eq!(cfg.len(), 3);
        assert_eq!(cfg[1], ("LuaAsset", ConfigValue::Int(264693)));
        assert_eq!(cfg[2], ("Speed", ConfigValue::Double(0.1f32 as f64)));
        assert_eq!(WidgetDef::parse(&j, None).unwrap().unknown_keys, vec!["Sequence".to_string()]);
    }
}
