//! A screen's widgets with their JSON definitions and scripts: the graph screen → widget groups → widgets →
//! (JSON definition, screen scripts → Lua chunk or native script). Format: `mw_formats::widget`; how the game
//! builds and runs it: docs/decomp.md "Widget definitions (JSON) and widget scripts".

use mw_bundle::{ResourceId, ResourceType};
use mw_formats::genesys::{Object, Value, field_hash};
use mw_formats::json::Json;
use mw_formats::widget::{self, NativeScreenScript, WidgetDef};

use crate::{AssetStore, Error, Result};

/// The screen bundles' folder.
pub const SCREENS_DIR: &str = "UI/SCREENS2";

/// One screen bundle's widgets and scripts.
#[derive(Debug, Clone)]
pub struct ScreenGraph {
    pub bundle: String,
    /// The `Screen2` object's UniqueId (0 when the bundle has none).
    pub screen: u32,
    /// `ScreenName` (field `dfff737a`, e.g. "MainMap", "E3_TITLE").
    pub screen_name: String,
    pub widgets: Vec<ScreenWidget>,
    /// JSON `TextFile`s of the bundle that no widget of its screen names.
    pub orphan_json: Vec<ResourceId>,
    /// Lua chunks (`TextFile`s starting with `\x1bLua` after the length) in the bundle.
    pub lua_chunks: Vec<ResourceId>,
}

/// A `Widget_*` struct of the screen and what it refers to.
#[derive(Debug, Clone)]
pub struct ScreenWidget {
    /// The widget group's UniqueId.
    pub group: u32,
    /// GeneSys type without the `Genesys.Gen.` prefix (`Widget_MenuData`).
    pub type_name: String,
    pub name: String,
    pub id: u32,
    /// The JSON definition's resource id (`"<name>_<id>.json"`).
    pub json: ResourceId,
    /// The parsed definition, or why it could not be read.
    pub def: std::result::Result<WidgetDef, String>,
    /// The GeneSys struct's `LayoutLayer` / `ModalMode` enum values (compared with the JSON's names).
    pub layout_layer: Option<u64>,
    pub modal_mode: Option<u64>,
    /// The widget's scripts (from the GeneSys struct's `Scripts`, joined with the JSON's by position).
    pub scripts: Vec<ResolvedScript>,
}

/// A widget script and what runs it.
#[derive(Debug, Clone)]
pub struct ResolvedScript {
    pub name: String,
    /// The `<Name>_ScreenScript` GeneSys object the widget's `Scripts[].Script` handle points to.
    pub object: Option<ScreenScriptObject>,
    pub target: ScriptTarget,
}

/// What a widget script runs (the choice `FUN_007f2850` makes).
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptTarget {
    /// The JSON `Script` has `LuaAsset`: a Lua script object running chunk `CRC32("<name>.lua")`.
    Lua { chunk: ResourceId, found: bool },
    /// No `LuaAsset`: the native screen script of that name.
    Native(&'static NativeScreenScript),
    /// No `LuaAsset` and no native script of that name: the game fails the widget (`+0x2028` error flag).
    Unresolved,
}

/// A `*_ScreenScript` GeneSys object: the script header plus its typed configuration fields.
#[derive(Debug, Clone)]
pub struct ScreenScriptObject {
    pub id: ResourceId,
    pub type_name: String,
    pub lua_asset: Option<u32>,
    /// `LuaHooks`: the natives bound into the widget's Lua state before the chunk runs (`FUN_007f84c0` →
    /// `FUN_009f1c30`: a registered native becomes a global C closure; `tonumber`, `tostring`, `error`, `pairs`,
    /// `mfpFunction` are accepted as built in; any other name sets the error flag).
    pub lua_hooks: Vec<String>,
    /// `LuaModules` (`string`, `math`, `table`): the libraries opened when `LuaModulesIncluded` is set; when it
    /// is not, all five (`package`, `table`, `io`, `string`, `math`) are opened (`FUN_009ef470`).
    pub lua_modules: Vec<String>,
    pub lua_modules_included: Option<bool>,
}

impl ScreenGraph {
    /// Every script of every widget.
    pub fn scripts(&self) -> impl Iterator<Item = (&ScreenWidget, &ResolvedScript)> {
        self.widgets.iter().flat_map(|w| w.scripts.iter().map(move |s| (w, s)))
    }
}

impl AssetStore {
    /// The screen bundles (`UI/SCREENS2/*.BNDL`), sorted.
    pub fn ui_screen_bundles(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .bundle_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| p.to_ascii_uppercase().starts_with(SCREENS_DIR))
            .collect();
        v.sort();
        v
    }

    /// A widget's JSON (`TextFile`), parsed by the game's JSON reader.
    pub fn ui_widget_json(&self, id: ResourceId) -> Result<Json> {
        let res = self.load_typed(id, ResourceType::TextFile)?;
        Ok(Json::parse_text_file(res.header())?)
    }

    /// A widget script's Lua chunk: the TextFile's bytes after its u32 length (what `thunk_FUN_009ef4c0` loads).
    pub fn ui_lua_chunk(&self, id: ResourceId) -> Result<Vec<u8>> {
        let res = self.load_typed(id, ResourceType::TextFile)?;
        let h = res.header();
        let n = h.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        h.get(4..4 + n).filter(|c| c.starts_with(b"\x1bLua")).map(<[u8]>::to_vec).ok_or(Error::Missing(id))
    }

    /// The bundle of screen `id` (`UI/SCREENS2/<id>.BNDL`).
    pub fn ui_screen_bundle(id: u32) -> String {
        format!("UI/SCREENS2/{id}.BNDL")
    }

    /// The screen of a bundle with its widgets, their JSON definitions and scripts.
    pub fn ui_screen_graph(&self, bundle: &str) -> Result<ScreenGraph> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        let mut json_files = Vec::new();
        let mut lua_chunks = Vec::new();
        for e in b.entries_of_type(ResourceType::TextFile) {
            let res = b.load(e)?;
            if res.header().get(4..8) == Some(b"\x1bLua") {
                lua_chunks.push(e.id);
            } else {
                json_files.push(e.id);
            }
        }
        let screen = self.ui_screen_object(bundle).ok();
        let mut widgets = Vec::new();
        if let Some(s) = &screen {
            for g in structs(s, field_hash("WidgetGroups")) {
                let group = uint(g, ID).unwrap_or(0) as u32;
                for w in structs(g, field_hash("Widgets")) {
                    widgets.push(self.screen_widget(group, w));
                }
            }
        }
        let named: std::collections::HashSet<_> = widgets.iter().map(|w| w.json).collect();
        Ok(ScreenGraph {
            bundle: bundle.to_string(),
            screen: screen.as_ref().and_then(|s| uint(s, ID)).unwrap_or(0) as u32,
            screen_name: screen
                .as_ref()
                .and_then(|s| match s.get(SCREEN_NAME) {
                    Some(Value::Str(n)) => Some(n.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            widgets,
            orphan_json: json_files.into_iter().filter(|id| !named.contains(id)).collect(),
            lua_chunks,
        })
    }

    fn screen_widget(&self, group: u32, w: &Object) -> ScreenWidget {
        let name = match w.get(field_hash("Name")) {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let id = uint(w, ID).unwrap_or(0) as u32;
        let type_name = w.type_name.trim_start_matches("Genesys.Gen.").to_string();
        let json = widget::json_resource_id(&name, id);
        let def = self
            .ui_widget_json(json)
            .map_err(|e| e.to_string())
            .and_then(|j| WidgetDef::parse(&j, Some(&type_name)).map_err(|e| e.to_string()));
        // The GeneSys `Scripts[]` {Name, Script handle}: `FUN_007f84c0` finds a JSON script's header object by
        // comparing names (first match) and reads `LuaHooks` / `LuaModules` from it.
        let objects: Vec<(String, Option<ResourceId>)> = structs(w, field_hash("Scripts"))
            .map(|s| {
                let n = match s.get(field_hash("Name")) {
                    Some(Value::Str(n)) => n.clone(),
                    _ => String::new(),
                };
                let h = match s.get(field_hash("Script")) {
                    Some(Value::Handle(h)) => *h,
                    Some(Value::Ref(r)) => Some(*r),
                    _ => None,
                };
                (n, h)
            })
            .collect();
        let scripts = match &def {
            Ok(d) => d
                .scripts
                .iter()
                .map(|s| {
                    let object = objects
                        .iter()
                        .find(|(n, _)| *n == s.name)
                        .and_then(|(_, h)| *h)
                        .and_then(|h| self.screen_script_object(h));
                    let target = if s.lua_asset.is_some() {
                        let chunk = s.lua_chunk();
                        ScriptTarget::Lua { chunk, found: self.type_of(chunk) == Some(ResourceType::TextFile) }
                    } else {
                        s.native().map_or(ScriptTarget::Unresolved, ScriptTarget::Native)
                    };
                    ResolvedScript { name: s.name.clone(), object, target }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        ScreenWidget {
            group,
            type_name,
            name,
            id,
            json,
            def,
            layout_layer: uint(w, field_hash("LayoutLayer")),
            modal_mode: uint(w, field_hash("ModalMode")),
            scripts,
        }
    }

    /// A `*_ScreenScript` GeneSys object's header fields.
    pub fn screen_script_object(&self, id: ResourceId) -> Option<ScreenScriptObject> {
        let o = self.genesys_object(id).ok()?;
        let strings = |h: u32| match o.get(h) {
            Some(Value::Array(a)) => {
                a.iter().filter_map(|v| if let Value::Str(s) = v { Some(s.clone()) } else { None }).collect()
            }
            _ => Vec::new(),
        };
        Some(ScreenScriptObject {
            id,
            type_name: o.type_name.trim_start_matches("Genesys.Gen.").to_string(),
            lua_asset: uint(&o, field_hash("LuaAsset")).map(|u| u as u32),
            lua_hooks: strings(field_hash("LuaHooks")),
            lua_modules: strings(field_hash("LuaModules")),
            lua_modules_included: match o.get(field_hash("LuaModulesIncluded")) {
                Some(Value::Bool(b)) => Some(*b),
                _ => None,
            },
        })
    }
}

/// A `SequenceItems.AdapterHost` sequence item: while it runs (`StartTime`..`EndTime`, gated by
/// `EnabledBinding`) it publishes the adapters of a JSON definition under `Name` (bindings `Name.<Adapter>`).
/// The 13 JSON files of `EN_US/FEEDBACKGROUPS` ("Branding", "MiniMwl", "SequenceMwl", "TransitionImages") are
/// reached this way, by resource handle (field `Adapters`), not by a widget's name.
#[derive(Debug, Clone)]
pub struct AdapterHost {
    /// The `Sequence` holding the item (UniqueId).
    pub sequence: u32,
    pub id: u32,
    pub name: String,
    pub json: ResourceId,
    pub enabled_binding: String,
    pub start: f32,
    pub end: f32,
}

impl AssetStore {
    /// Every `SequenceItems.AdapterHost` of a bundle's sequences.
    pub fn ui_adapter_hosts(&self, bundle: &str) -> Result<Vec<AdapterHost>> {
        let b = self.bundle(bundle).ok_or(Error::Missing(ResourceId::NULL))?;
        let mut out = Vec::new();
        for e in b.entries_of_type(ResourceType::GenesysObject) {
            let res = b.load(e)?;
            let Some(ty) = res.import_at(0) else { continue };
            if !self.genesys_type(ty).is_some_and(|t| t.name.ends_with("Gen.Sequence")) {
                continue;
            }
            let Ok(seq) = self.decode_genesys(&res) else { continue };
            let sequence = uint(&seq, ID).unwrap_or(0) as u32;
            for it in structs(&seq, field_hash("SequenceItems")) {
                if !it.type_name.ends_with("SequenceItems.AdapterHost") {
                    continue;
                }
                let s = |h: u32| match it.get(h) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let f = |h: u32| match it.get(h) {
                    Some(Value::Float(f)) => *f,
                    _ => 0.0,
                };
                let json = match it.get(field_hash("Adapters")) {
                    Some(Value::Handle(Some(h))) | Some(Value::Ref(h)) => *h,
                    _ => continue,
                };
                out.push(AdapterHost {
                    sequence,
                    id: uint(it, ID).unwrap_or(0) as u32,
                    name: s(field_hash("Name")),
                    json,
                    enabled_binding: s(field_hash("EnabledBinding")),
                    start: f(field_hash("StartTime")),
                    end: f(field_hash("EndTime")),
                });
            }
        }
        Ok(out)
    }
}

/// `UniqueId` (`655d3f5f`).
const ID: u32 = 0x655d_3f5f;
/// `Screen2`'s name string.
const SCREEN_NAME: u32 = 0xdfff_737a;

fn uint(o: &Object, h: u32) -> Option<u64> {
    match o.get(h)? {
        Value::UInt(u) => Some(*u),
        Value::Int(i) => Some(*i as u64),
        Value::Bool(b) => Some(*b as u64),
        _ => None,
    }
}

fn structs(o: &Object, h: u32) -> impl Iterator<Item = &Object> {
    let a: &[Value] = match o.get(h) {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    a.iter().filter_map(|v| if let Value::Struct(s) = v { Some(s) } else { None })
}
