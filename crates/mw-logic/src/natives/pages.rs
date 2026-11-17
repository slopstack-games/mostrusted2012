//! PageRunner2 page contexts and the widget screen manager's widget groups (PLAN.md M8 step 4, the runtime side of
//! docs/decomp.md "Widget definitions (JSON) and widget scripts"): what the HSM's `RUN_PG_RNNR2` /
//! `ACTI_WDGT_GR` nodes ask for ([`Request::PushPageContext`], [`Request::ActivateWidgetGroup`], ...) and the
//! page queries they wait on.
//!
//! - **Contexts**: a stack of {name, page}; a pushed page's screen ([`ScreenData`]) is loaded by the host
//!   ([`Pages::wanted`] → [`Pages::provide`]); the page is ready ([`Query::PageReady`]) once it is.
//! - **Widget groups**: activating a group builds each of its widgets (screen order) as the original does
//!   (`FUN_007f84c0`): one Lua state per widget with the natives bound, each Lua script's chunk run (it defines
//!   the global table `<name>`), `Construct(this)` with the script object as light userdata, then
//!   `SetConfigurationItem(key, value)` for each scalar entry of the JSON `Script` object in file order
//!   (`FUN_007ec680`), then `OnActivated()` (slot 2). Every frame `PreInputUpdate(dt)` (slot 4); deactivation
//!   calls `OnDeactivating()` (slot 3) and `Destruct()` (the destructor, slot 1). Calls go to
//!   `_G[<script>][<function>]` without `self`; a missing function is skipped; an error is logged and dropped
//!   (`lua_pcall`).
//! - **The `Widget` view model**: bindings `Widget.<widget>.<member>` (e.g. `PlaySequence`'s
//!   `Widget.Background.Sequence`) read the active widgets' typed fields.
//!
//! Model decisions (not traced in the code): the `Widget` root is one published JSON object of the active
//! widgets' typed JSON fields (the original resolves it through the script's view model, slot 8 `FUN_007e7a80`;
//! adapters are not published yet); every native of the registry is bound, not only the script's `LuaHooks`;
//! the page's exit point ([`Query::WidgetExit`], page +0x208) is the widget manager's group exit (WM+0x488) that
//! `ExitWidgetGroup` / `ExitGroupVia` set; native screen scripts are not run (each is its own C++ port).

use std::collections::HashMap;
use std::sync::Arc;

use mw_formats::json::Json;
use mw_formats::widget::{ConfigValue, WidgetDef};
use mw_lua::{Lua, Value};

use super::{LogicState, Request};
use crate::hsm::behaviours::Query;

/// One screen (`Screen2`) as the host loads it: its widgets in screen order.
#[derive(Debug, Clone, Default)]
pub struct ScreenData {
    pub id: u32,
    pub widgets: Vec<WidgetData>,
}

/// One widget of a screen: its group, JSON definition and scripts (in the definition's `Scripts` order).
#[derive(Debug, Clone)]
pub struct WidgetData {
    pub group: u32,
    pub def: WidgetDef,
    pub scripts: Vec<ScriptData>,
    /// How each of `def.actions` fires (its `UserActionDefinition`), same order.
    pub actions: Vec<ActionData>,
}

/// A widget action's `UserActionDefinition`: the input action it reads ([`input_action`] of its `InputMapping`) and
/// how it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionData {
    pub input: Option<InputAction>,
    /// `Analogue` (+0x34): the handler is called every frame with the value.
    pub analogue: bool,
    /// Field `2da735e3` (+0x35): fires on the press and on the auto-repeat ticks.
    pub repeat: bool,
}

/// An input action id of the menus (`FUN_007e7910`'s table) and its directional flag (the table's third column:
/// such actions fire only above [`DIRECTION_FIRE_MIN`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAction {
    pub id: u8,
    pub directional: bool,
}

/// `FUN_007e7910`: the `InputMapping` name → action id table (`.rdata` 0x00fe7260, 33 entries of {name, id, flag});
/// a name not in it gives 0x60 (no action here). Default bindings: keyboard `FUN_00483470`, pad `FUN_00482250`
/// (docs/decomp.md "Menu input and the map screen's controls").
pub const INPUT_MAPPINGS: [(&str, u8, bool); 33] = [
    ("menu_left", 42, true),
    ("menu_right", 43, true),
    ("menu_up", 44, true),
    ("menu_down", 45, true),
    ("menu_left_alt", 46, true),
    ("menu_right_alt", 47, true),
    ("menu_up_alt", 48, true),
    ("menu_down_alt", 49, true),
    ("scroll_up", 50, true),
    ("scroll_down", 51, true),
    ("accept", 52, false),
    ("cancel", 53, false),
    ("settings", 54, false),
    ("alt", 55, false),
    ("tab_left", 56, false),
    ("tab_right", 57, false),
    ("trigger_left", 58, false),
    ("trigger_right", 59, false),
    ("view_profile", 60, false),
    ("start", 61, false),
    ("to_main_menu", 62, false),
    ("play_it", 63, false),
    ("friend_recommends", 65, false),
    ("fullscreen_photo", 64, false),
    ("hitbox", 66, false),
    ("dbg_win_event", 67, false),
    ("dbg_switch_progression", 68, false),
    ("easy_left", 69, false),
    ("easy_right", 70, false),
    ("easy_up", 71, false),
    ("easy_down", 72, false),
    ("universal_accept", 73, false),
    ("universal_alt", 74, false),
];

/// [`INPUT_MAPPINGS`] lookup (exact, case-sensitive: the original compares bytes).
pub fn input_action(mapping: &str) -> Option<InputAction> {
    INPUT_MAPPINGS.iter().find(|(n, ..)| *n == mapping).map(|&(_, id, directional)| InputAction { id, directional })
}

/// A directional action fires (pressed or repeated) only above this value (`FUN_007f50e0`, actions 0x2a–0x31).
pub const DIRECTION_FIRE_MIN: f32 = 0.05;
/// A Lua script's cooldown after it handled or refused an action (`.rdata` 0x00fe73ec, `FUN_007eed50`).
pub const SCRIPT_COOLDOWN: f32 = 0.15;

/// One input action's state this frame, as the runtime's input reads it (the binding value pair and auto-repeat
/// of `FUN_00ac6850` / `FUN_0047c360`; for 42–45 already resolved against the `_alt` twin, `FUN_0047a3f0`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionInput {
    pub pressed: bool,
    /// An auto-repeat tick this frame (held).
    pub repeat: bool,
    pub value: f32,
}

#[derive(Debug, Clone)]
pub struct ScriptData {
    /// The script's name (its Lua table / chunk name, or the native script's).
    pub name: String,
    pub code: ScriptCode,
}

#[derive(Debug, Clone)]
pub enum ScriptCode {
    /// The Lua chunk (the bytecode after the TextFile's length).
    Lua(Vec<u8>),
    /// A native screen script (not run here).
    Native,
    /// Neither (the original sets the screen's error flag).
    Missing,
}

#[derive(Debug, Clone)]
struct Context {
    name: String,
    page: u32,
}

/// An active widget: its Lua state and the Lua scripts constructed in it.
struct ActiveWidget {
    name: String,
    lua: Lua,
    /// The constructed Lua scripts with their cooldowns (+0x14).
    scripts: Vec<String>,
    cooldowns: Vec<f32>,
    /// The native screen scripts (not run; an action offered to them is logged).
    natives: Vec<String>,
    data: WidgetData,
}

struct ActiveGroup {
    group: u32,
    widgets: Vec<ActiveWidget>,
}

/// The page contexts, loaded screens and active widget groups.
#[derive(Default)]
pub struct Pages {
    contexts: Vec<Context>,
    screens: HashMap<u32, Arc<ScreenData>>,
    /// Screens a pushed context needs that the host has not provided yet.
    pub wanted: Vec<u32>,
    groups: Vec<ActiveGroup>,
    next_this: usize,
    /// Script errors (`lua_pcall` drops them in the original), for the host's log.
    pub errors: Vec<String>,
}

impl Pages {
    /// The host's answer to [`Pages::wanted`].
    pub fn provide(&mut self, screen: ScreenData) {
        self.wanted.retain(|&w| w != screen.id);
        self.screens.insert(screen.id, Arc::new(screen));
    }

    /// The page of the top context.
    pub fn current_page(&self) -> Option<u32> {
        self.contexts.last().map(|c| c.page)
    }

    /// The active widget groups, oldest first.
    pub fn active_groups(&self) -> Vec<u32> {
        self.groups.iter().map(|g| g.group).collect()
    }

    /// The page queries this manager answers (`None`: not a page query).
    pub fn query(&self, state: &LogicState, q: &Query) -> Option<u64> {
        Some(match q {
            Query::PageContextExists { name } => self.contexts.iter().any(|c| &c.name == name) as u64,
            Query::PageReady => self.current_page().is_some_and(|p| self.screens.contains_key(&p)) as u64,
            Query::ScreenActive => self.contexts.iter().any(|c| self.screens.contains_key(&c.page)) as u64,
            Query::WidgetExit => state.screen.widget_group_exit,
            _ => return None,
        })
    }

    /// The widget groups' per-frame update (`FUN_007f6420`): every Lua script's `PreInputUpdate(dt)`.
    pub fn frame(&mut self, dt: f32) {
        for g in &mut self.groups {
            for w in &mut g.widgets {
                // The cooldown counts down while the widget has input (every active widget here: module doc).
                for c in &mut w.cooldowns {
                    *c -= dt;
                }
                for s in &w.scripts {
                    if let Err(e) = w.lua.call_field(s, "PreInputUpdate", vec![Value::from(dt as f64)]) {
                        self.errors.push(format!("{}.{s}: {e}", w.name));
                    }
                }
            }
        }
    }
}

/// What [`super::host::LogicHost`] does with a page request. `install` binds the natives into a new widget state.
pub(super) fn apply(
    pages: &mut Pages,
    state: &std::cell::RefCell<LogicState>,
    install: &dyn Fn(&mut Lua),
    r: &Request,
) {
    match r {
        Request::PushPageContext { name, page } => {
            let Ok(page) = page.parse::<u32>() else { return };
            pages.contexts.retain(|c| &c.name != name);
            pages.contexts.push(Context { name: name.clone(), page });
            if !pages.screens.contains_key(&page) && !pages.wanted.contains(&page) {
                pages.wanted.push(page);
            }
        }
        Request::LeavePageContext { name, .. } => {
            if let Some(i) = pages.contexts.iter().position(|c| &c.name == name) {
                let page = pages.contexts.remove(i).page;
                let groups: Vec<u32> =
                    pages.screens.get(&page).map_or(Vec::new(), |s| s.widgets.iter().map(|w| w.group).collect());
                for g in groups {
                    deactivate(pages, state, g);
                }
            }
        }
        Request::ActivateWidgetGroup { group } => {
            let group = *group as u32;
            if pages.groups.iter().any(|g| g.group == group) {
                return;
            }
            let Some(screen) = pages
                .contexts
                .iter()
                .rev()
                .filter_map(|c| pages.screens.get(&c.page))
                .find(|s| s.widgets.iter().any(|w| w.group == group))
            else {
                return;
            };
            let screen = screen.clone();
            let mut widgets = Vec::new();
            for w in screen.widgets.iter().filter(|w| w.group == group) {
                widgets.push(build(pages, install, w));
            }
            pages.groups.push(ActiveGroup { group, widgets });
            publish_widgets(pages, state);
            // `OnActivated` once every widget of the group exists (`FUN_007ebc80`, list order).
            let g = pages.groups.last_mut().expect("pushed");
            for w in &mut g.widgets {
                for s in &w.scripts {
                    if let Err(e) = w.lua.call_field(s, "OnActivated", vec![]) {
                        pages.errors.push(format!("{}.{s}: {e}", w.name));
                    }
                }
            }
        }
        Request::DeactivateWidgetGroup { group, .. } => deactivate(pages, state, *group as u32),
        Request::ClearWidgetExit => state.borrow_mut().screen.widget_group_exit = 0,
        _ => {}
    }
}

/// One widget built as `FUN_007f84c0` does (module doc).
fn build(pages: &mut Pages, install: &dyn Fn(&mut Lua), w: &WidgetData) -> ActiveWidget {
    let mut lua = Lua::new();
    install(&mut lua);
    let mut scripts = Vec::new();
    for (i, s) in w.scripts.iter().enumerate() {
        let ScriptCode::Lua(chunk) = &s.code else { continue };
        let main = match lua.load(chunk) {
            Ok(f) => Value::Function(f),
            Err(e) => {
                pages.errors.push(format!("{}.{}: load: {e}", w.def.name, s.name));
                continue;
            }
        };
        if let Err(e) = lua.call(&main, vec![]) {
            pages.errors.push(format!("{}.{}: main: {e}", w.def.name, s.name));
        }
        pages.next_this += 1;
        let this = Value::LightUserdata(0x1000_0000 + pages.next_this);
        if let Err(e) = lua.call_field(&s.name, "Construct", vec![this]) {
            pages.errors.push(format!("{}.{}: {e}", w.def.name, s.name));
        }
        if let Some(def) = w.def.scripts.get(i).filter(|d| d.name == s.name) {
            for (k, v) in &def.config {
                let v = match v {
                    ConfigValue::Str(t) => Value::from(t.as_str()),
                    ConfigValue::Int(n) => Value::from(*n as f64),
                    // Pushed as `(double)(float)value`.
                    ConfigValue::Double(d) => Value::from(*d as f32 as f64),
                    ConfigValue::Bool(b) => Value::from(*b),
                    ConfigValue::Json(_) => continue,
                };
                if let Err(e) = lua.call_field(&s.name, "SetConfigurationItem", vec![Value::from(k.as_str()), v]) {
                    pages.errors.push(format!("{}.{}: {e}", w.def.name, s.name));
                }
            }
        }
        scripts.push(s.name.clone());
    }
    let natives = w.scripts.iter().filter(|s| matches!(s.code, ScriptCode::Native)).map(|s| s.name.clone()).collect();
    let cooldowns = vec![0.0; scripts.len()];
    ActiveWidget { name: w.def.name.clone(), lua, scripts, cooldowns, natives, data: w.clone() }
}

fn deactivate(pages: &mut Pages, state: &std::cell::RefCell<LogicState>, group: u32) {
    let Some(i) = pages.groups.iter().position(|g| g.group == group) else { return };
    let mut g = pages.groups.remove(i);
    for w in &mut g.widgets {
        for s in &w.scripts {
            for f in ["OnDeactivating", "Destruct"] {
                if let Err(e) = w.lua.call_field(s, f, vec![]) {
                    pages.errors.push(format!("{}.{s}: {e}", w.name));
                }
            }
        }
    }
    publish_widgets(pages, state);
}

/// Publishes the `Widget` view model: the active widgets' typed fields by widget name.
fn publish_widgets(pages: &Pages, state: &std::cell::RefCell<LogicState>) {
    let mut members: Vec<(String, Json)> = Vec::new();
    for g in &pages.groups {
        let Some(screen) = pages
            .contexts
            .iter()
            .rev()
            .filter_map(|c| pages.screens.get(&c.page))
            .find(|s| s.widgets.iter().any(|w| w.group == g.group))
        else {
            continue;
        };
        for w in screen.widgets.iter().filter(|w| w.group == g.group) {
            members.retain(|(n, _)| n != &w.def.name);
            members.push((w.def.name.clone(), Json::Object(w.def.fields.clone())));
        }
    }
    let mut st = state.borrow_mut();
    if members.is_empty() {
        st.view_models.retract("Widget");
    } else {
        st.view_models.publish("Widget", Json::Object(members));
    }
}

/// Screen `id` read from the install (`UI/SCREENS2/<id>.BNDL`): its widgets with their definitions and the Lua
/// chunks of their scripts. A widget whose JSON does not parse is left out (the original would not load it).
#[cfg(feature = "assets")]
pub fn load_screen(store: &mw_assets::AssetStore, id: u32) -> mw_assets::Result<ScreenData> {
    use mw_assets::widgets::ScriptTarget;
    let graph = store.ui_screen_graph(&mw_assets::AssetStore::ui_screen_bundle(id))?;
    let mut widgets = Vec::new();
    for w in graph.widgets {
        let Ok(def) = w.def else { continue };
        let scripts = w
            .scripts
            .iter()
            .map(|s| ScriptData {
                name: s.name.clone(),
                code: match &s.target {
                    ScriptTarget::Lua { chunk, .. } => {
                        store.ui_lua_chunk(*chunk).map_or(ScriptCode::Missing, ScriptCode::Lua)
                    }
                    ScriptTarget::Native(_) => ScriptCode::Native,
                    ScriptTarget::Unresolved => ScriptCode::Missing,
                },
            })
            .collect();
        let actions = def
            .actions
            .iter()
            .map(|a| {
                let d = store.ui_action_definition(a.definition);
                ActionData {
                    input: d.as_ref().and_then(|d| input_action(&d.input_mapping)),
                    analogue: d.as_ref().is_some_and(|d| d.analogue),
                    repeat: d.as_ref().is_some_and(|d| d.repeat),
                }
            })
            .collect();
        widgets.push(WidgetData { group: w.group, def, scripts, actions });
    }
    Ok(ScreenData { id, widgets })
}

/// One fired action, routed as `FUN_007f40d0` does.
struct Fired {
    group: usize,
    widget: usize,
    action: usize,
    value: Option<f32>,
}

impl Pages {
    /// The widgets' user actions this frame (`FUN_007f50e0` per action, then `FUN_007f40d0`): `read` gives an input
    /// action's state, `hook` calls an HSM hook by name (a native callback that is not `ExitGroupVia`).
    pub(super) fn input(
        &mut self,
        state: &std::cell::RefCell<LogicState>,
        read: &dyn Fn(u8) -> ActionInput,
        hook: &mut dyn FnMut(&mut LogicState, &str),
    ) {
        let mut fired = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            for (wi, w) in g.widgets.iter().enumerate() {
                for (ai, a) in w.data.actions.iter().enumerate() {
                    let Some(input) = a.input else { continue };
                    let i = read(input.id);
                    let value = if a.analogue {
                        Some(i.value)
                    } else if a.repeat {
                        let on = i.pressed || i.repeat;
                        (on && (!input.directional || i.value > DIRECTION_FIRE_MIN)).then_some(i.value)
                    } else {
                        i.pressed.then_some(i.value)
                    };
                    if let Some(v) = value {
                        fired.push(Fired { group: gi, widget: wi, action: ai, value: a.analogue.then_some(v) });
                    }
                }
            }
        }
        for f in fired {
            self.route(state, f, hook);
        }
    }

    fn route(&mut self, state: &std::cell::RefCell<LogicState>, f: Fired, hook: &mut dyn FnMut(&mut LogicState, &str)) {
        let Some(w) = self.groups.get(f.group).and_then(|g| g.widgets.get(f.widget)) else { return };
        let Some(a) = w.data.def.actions.get(f.action).cloned() else { return };
        let active = |state: &std::cell::RefCell<LogicState>| {
            a.active_binding.is_empty() || state.borrow().resolve_bool(&a.active_binding) == Some(true)
        };
        let mut args: Vec<Value> =
            if a.args.is_empty() { Vec::new() } else { a.args.split(',').map(Value::from).collect() };
        if let Some(v) = f.value {
            // Model: an analogue value reaches a Lua callback as one more argument.
            args.push(Value::from(v as f64));
        }
        if let Some((target, func)) = a.callback.split_once("::") {
            // Cross-widget (`FUN_007eeb40`): offered to the target widget's scripts until one handles it.
            if !active(state) {
                return;
            }
            let Some((gi, wi)) = self
                .groups
                .iter()
                .enumerate()
                .find_map(|(gi, g)| g.widgets.iter().position(|w| w.name == target).map(|wi| (gi, wi)))
            else {
                return;
            };
            let tw = &mut self.groups[gi].widgets[wi];
            for s in tw.scripts.clone() {
                match tw.lua.call_field(&s, func, args.clone()) {
                    Ok(Some(_)) => return,
                    Ok(None) => {}
                    Err(e) => {
                        self.errors.push(format!("{target}.{s}: {e}"));
                        return;
                    }
                }
            }
            if let Some(n) = tw.natives.first() {
                self.errors.push(format!("{target}: native script {n} not run ({func})"));
            }
        } else if a.in_lua {
            // Every script of the widget (slot 5, `FUN_007eed50`): only out of its cooldown; the binding refusing
            // or the callback existing both start the cooldown.
            let ok = active(state);
            let w = &mut self.groups[f.group].widgets[f.widget];
            let mut handled = false;
            for (si, s) in w.scripts.clone().iter().enumerate() {
                if w.cooldowns[si] > 0.0 {
                    continue;
                }
                if !ok {
                    w.cooldowns[si] = SCRIPT_COOLDOWN;
                    continue;
                }
                match w.lua.call_field(s, &a.callback, args.clone()) {
                    Ok(Some(_)) => {
                        w.cooldowns[si] = SCRIPT_COOLDOWN;
                        handled = true;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        w.cooldowns[si] = SCRIPT_COOLDOWN;
                        self.errors.push(format!("{}.{s}: {e}", w.name));
                    }
                }
            }
            if let Some(n) = w.natives.first().filter(|_| ok && !handled) {
                self.errors.push(format!("{}: native script {n} not run ({})", w.name, a.callback));
            }
        } else {
            // Native (`FUN_007f2670`): `ExitGroupVia` sets the group's exit, anything else is an HSM hook.
            if !active(state) {
                return;
            }
            let mut st = state.borrow_mut();
            if a.callback == "ExitGroupVia" {
                let first = a.args.split(',').next().unwrap_or("").trim();
                super::lua_widgets::exit_group_via(&mut st, first);
            } else {
                hook(&mut st, &a.callback);
            }
        }
    }
}
