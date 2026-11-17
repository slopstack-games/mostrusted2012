//! Lua natives of the widget scripts' own world: bindings, the script's variables, leaving the widget group,
//! calls into other widgets, modal mode, sequences on conductors, popup dialogs, text entry, the language
//! (decomp.md "Native registries: Lua natives"). The managers they act on:
//!
//! - **WM** = `*0x0117e9f4`, the widget-screen manager (the one that builds the screens' scripts,
//!   `FUN_007f84c0`): [`ScreenManager`]; its +0x2020 is the game object G (`*0x01177868`), +0x2024 the binding
//!   manager the bindings resolve with.
//! - The Lua script objects (0xbc bytes, vtable 0x00fe7f78), passed to the script's `Construct` as light
//!   userdata and handed back by the scripts to the `Set*Variable` natives: [`WidgetScripts`].

use mw_formats::json::Json;
use mw_lua::{Lua, Value};

use super::lua_args::{
    fit, ftol, int, push_int, push_str, round_int, string_or_invalid, toboolean, tolstring, tonumber, touserdata,
};
use super::{LogicState, Registry, Request, vm_path};
use crate::hsm::Key;

type R = mw_lua::Result<Vec<Value>>;

/// The widget-screen manager (WM, `*0x0117e9f4`) as far as the Lua natives use it.
#[derive(Debug, Default)]
pub struct ScreenManager {
    /// +0x488: the widget group's exit code, the packed name (`FUN_009ed220`) `ExitWidgetGroup` / the native
    /// action `ExitGroupVia` set; the HSM's "Activate Widget Group" node takes it as its exit. 0 = none.
    pub widget_group_exit: Key,
    /// +0x4a8: `SetModalMode`.
    pub modal_mode: bool,
    /// +0x478: the conductors by name hash (`FUN_007ffcd0`: sorted records of 0x50 bytes, the hash first).
    pub conductors: Vec<Conductor>,
    /// G+0x2f480 → +0x47740: the popup-dialog manager.
    pub dialogs: Dialogs,
    /// The language (`FUN_0046ef30`: text database +4 → +0xb78), the engine's language code.
    pub language: u32,
}

/// A conductor (`CreateConductor`): plays feedback sequences one after another.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Conductor {
    /// The key: `FUN_009ed180(name)` (GeneSys field hash).
    pub name_hash: u32,
    pub name: String,
    /// What it plays: `None` = idle, i.e. the current sequence id (+0x10/+0x14) is the null id
    /// (`DAT_0117f0b0/4`) and nothing is queued (+0x24, +0x28 zero). Set by `StartSequence`; the runtime clears it
    /// when the sequence ends.
    pub playing: Option<String>,
}

/// The popup-dialog manager (UIM+0x47740).
#[derive(Debug, Default)]
pub struct Dialogs {
    /// The open dialogs (+0x10: records of 0x18 bytes, count +0x88), by the dialog definition's id (+0x20).
    pub open: Vec<u32>,
    /// The answers not collected yet: ids sorted (+0x94, count +0x90), each with its result byte (+0xa8).
    pub results: Vec<(u32, i8)>,
}

impl Dialogs {
    /// The runtime reports the button the player chose (the original stores it in the result map).
    pub fn set_result(&mut self, id: u32, result: i8) {
        match self.results.binary_search_by_key(&id, |&(i, _)| i) {
            Ok(i) => self.results[i].1 = result,
            Err(i) => self.results.insert(i, (id, result)),
        }
    }
}

/// One Lua script object (0xbc bytes, vtable 0x00fe7f78; built by `FUN_007f2850`, initialised by `FUN_007f29b0`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptObject {
    /// The light userdata the script was constructed with (the object's address in the original).
    pub handle: usize,
    /// The widget's name (the widget's key +0x88 is its field hash).
    pub widget: String,
    /// +0x98: the script's name (≤ 16 characters), its global table.
    pub name: String,
    /// +0xb4: the script's own view model; bindings `Widget.<widget>.<…>` resolve into it (slot 8,
    /// `FUN_007e7a80`).
    pub view_model: Json,
}

/// Every live Lua script object.
#[derive(Debug, Default)]
pub struct WidgetScripts {
    pub scripts: Vec<ScriptObject>,
    next: usize,
}

/// Where the handles of [`WidgetScripts`] start (any non-null value works; tools use 0x10000000).
pub const SCRIPT_HANDLE_BASE: usize = 0x1000_0000;

impl WidgetScripts {
    /// A new script object of `widget` named `name`; returns the light userdata to pass to its `Construct`.
    pub fn create(&mut self, widget: &str, name: &str) -> usize {
        let handle = SCRIPT_HANDLE_BASE + self.next * 0x100;
        self.next += 1;
        self.scripts.push(ScriptObject {
            handle,
            widget: widget.to_string(),
            name: fit(name, 17).to_string(),
            view_model: Json::Object(Vec::new()),
        });
        handle
    }

    pub fn get(&self, handle: usize) -> Option<&ScriptObject> {
        self.scripts.iter().find(|s| s.handle == handle)
    }

    pub fn destroy(&mut self, handle: usize) {
        self.scripts.retain(|s| s.handle != handle);
    }

    /// The view model a binding `Widget.<widget>` sees: the members of the widget's scripts' view models in
    /// script order (a binding is offered to each script in turn, so the first script that has the member wins).
    pub fn view_model(&self, widget: &str) -> Json {
        Json::Object(
            self.scripts
                .iter()
                .filter(|s| s.widget == widget)
                .flat_map(|s| s.view_model.entries().iter().cloned())
                .collect(),
        )
    }
}

/// Apply `f` to the view model of the script `handle` and republish the `Widget` root; no script (the original: a null object, it would crash) does nothing.
fn with_view_model(state: &mut LogicState, handle: Option<usize>, f: impl FnOnce(&mut Json) -> bool) {
    let Some(script) = handle.and_then(|h| state.widget_scripts.scripts.iter_mut().find(|s| s.handle == h)) else {
        return;
    };
    if f(&mut script.view_model) {
        // Model: the bindings' `Widget` root (slot 8 of the script objects resolves `Widget.<widget>.<…>`) is
        // one published object with a member per widget that has scripts.
        let mut widgets: Vec<String> = Vec::new();
        for s in &state.widget_scripts.scripts {
            if !widgets.contains(&s.widget) {
                widgets.push(s.widget.clone());
            }
        }
        let root = Json::Object(widgets.iter().map(|w| (w.clone(), state.widget_scripts.view_model(w))).collect());
        state.view_models.publish("Widget", root);
    }
}

/// The path the `Set*Variable` natives build (`FUN_007ebeb0` & co.): `<prefix>.<name>` when the optional 4th
/// argument is not empty, in a 64-byte buffer.
fn var_path(args: &[Value], name_arg: usize, prefix_arg: usize) -> String {
    let name = string_or_invalid(args, name_arg);
    let prefix = if args.len() >= prefix_arg { string_or_invalid(args, prefix_arg) } else { String::new() };
    let mut path = String::new();
    if !prefix.is_empty() {
        path.push_str(fit(&prefix, 64));
        path.push('.');
    }
    path.push_str(&name);
    fit(&path, 64).to_string()
}

/// `SetBoolVariable(script, name, value [, prefix])` (0x007ee200 → `FUN_007ebeb0` → JSON bool, tag 5).
fn set_bool_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 4);
    let v = toboolean(&args, 3);
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Bool(v)));
    Ok(vec![])
}

/// `SetIntVariable(script, name, value [, prefix])` (0x007ee290: `lua_tonumber` → `_ftol2` 64-bit →
/// `FUN_007ebfb0` → JSON int, tag 3).
fn set_int_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 4);
    let v = ftol(tonumber(&args, 3));
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Int(v)));
    Ok(vec![])
}

/// `SetDoubleVariable(script, name, value [, prefix])` (0x007ee330 → `FUN_007ec0b0` → JSON double, tag 4).
fn set_double_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 4);
    let v = tonumber(&args, 3);
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Double(v)));
    Ok(vec![])
}

/// `SetStringVariable(script, name, value [, prefix])` (0x007ee3c0 → `FUN_007ec1c0` → JSON string, tag 2).
fn set_string_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 4);
    let v = string_or_invalid(&args, 3);
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Str(v)));
    Ok(vec![])
}

/// `SetUniqueIdVariable(script, name, id [, prefix])` (0x007ee460: `_ftol2`, low 32 bits → `FUN_007ec2c0` →
/// `FUN_009f8ed0`, a JSON node of tag 7, vtable 0x00feb8a0: a unique id). Model decision: stored as an int (the
/// typed JSON has no tag 7).
fn set_unique_id_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 4);
    let v = ftol(tonumber(&args, 3)) as u32;
    with_view_model(state, handle, |vm| vm_path::set(vm, &path, Json::Int(v as i64)));
    Ok(vec![])
}

/// `DeleteVariable(script, name [, prefix])` (0x007ee4f0 → `FUN_007ec3c0` → `FUN_009f8800`).
fn delete_variable(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let handle = touserdata(&args, 1);
    let path = var_path(&args, 2, 3);
    with_view_model(state, handle, |vm| vm_path::delete(vm, &path));
    Ok(vec![])
}

/// `ResolveBoolBinding(binding)` (0x00680120 → `FUN_0067f4e0`: parse with the binding manager `*0x01177864`,
/// evaluate as bool, false when it does not parse or resolve).
fn resolve_bool_binding(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let b = string_or_invalid(&args, 1);
    Ok(vec![Value::Bool(state.resolve_bool(&b).unwrap_or(false))])
}

/// `ResolveIntBinding(binding)` (0x00679900 → `FUN_006794e0`: int, 0 when unresolved; pushed as a number).
fn resolve_int_binding(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let b = string_or_invalid(&args, 1);
    Ok(vec![push_int(state.resolve_int(&b).unwrap_or(0) as i32)])
}

/// `ResolveDoubleBinding(binding)` (0x00679950 → `FUN_00679550`: double, 0.0 when unresolved).
fn resolve_double_binding(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let b = string_or_invalid(&args, 1);
    Ok(vec![Value::Number(state.resolve_double(&b).unwrap_or(0.0))])
}

/// `ResolveStringBinding(binding)` (0x007e97e0 → `FUN_007e7cc0`: the to-text visitor into a 0x3ff-byte buffer,
/// "" when it does not parse or fails).
fn resolve_string_binding(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let b = string_or_invalid(&args, 1);
    let s = state.resolve_string(&b).unwrap_or_default();
    Ok(vec![push_str(fit(&s, 0x3ff))])
}

/// `ResolveWideStringBinding(binding)` (0x007ee570 → `FUN_007ecac0`: the localising to-text visitor
/// `FUN_0046f480` with the text database (G+0x2f480 +0x2ce40), into 0x400 wide characters, back to UTF-8 in
/// 0x3ff bytes; "" when unresolved). Model decision: the to-text result (string-id values are not looked up in
/// the text database yet).
fn resolve_wide_string_binding(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let b = string_or_invalid(&args, 1);
    let s = state.resolve_string(&b).unwrap_or_default();
    Ok(vec![push_str(fit(&s, 0x3ff))])
}

/// `ExitWidgetGroup(code)` (0x007f22e0 → `FUN_007eef40`): the text (≤ 255 bytes); if it contains `{`, the text
/// between its first and last character is resolved as a binding to text (≤ 15 characters, "" on failure); the
/// packed name (`FUN_009ed220`) of the result, when not 0, becomes the widget group's exit (WM+0x488).
fn exit_widget_group(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let text = string_or_invalid(&args, 1);
    exit_group_via(state, fit(&text, 256));
    Ok(vec![])
}

/// `FUN_007eef40` (also called by two native scripts, `FUN_008037c0` / `FUN_008071e0`); the native action
/// `ExitGroupVia` does the same in its own copy (`FUN_007f2530`).
pub fn exit_group_via(state: &mut LogicState, text: &str) {
    let code = if text.contains('{') {
        let inner = fit(text.get(1..text.len().saturating_sub(1)).unwrap_or(""), 64);
        let s = state.resolve_string(inner).unwrap_or_default();
        fit(&s, 16).to_string()
    } else {
        text.to_string()
    };
    let key = mw_formats::hsm::key(&code);
    if key != 0 {
        state.screen.widget_group_exit = key;
    }
}

/// `DoRpc(widget, function, args)` (0x007f2310 → `FUN_007ef0b0`): finds the screen's widget whose key is the
/// field hash of `widget` (WM+0x2b4, count +0x2ac) and offers `function(args)` to its scripts (`FUN_007eb9a0` →
/// vtable slot 6; Lua scripts get `args` split at `,`), synchronously. Here: [`Request::WidgetRpc`].
fn do_rpc(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::WidgetRpc {
        widget: string_or_invalid(&args, 1),
        function: string_or_invalid(&args, 2),
        args: string_or_invalid(&args, 3),
    });
    Ok(vec![])
}

/// `SetModalMode(on)` (0x007e98c0): WM+0x4a8.
fn set_modal_mode(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.screen.modal_mode = toboolean(&args, 1);
    Ok(vec![])
}

/// `print(text)` (0x007e9920): `lua_tolstring(L, 1)` and nothing else (the release build prints nothing).
fn print(_: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let _ = tolstring(&args, 1);
    Ok(vec![])
}

/// `GetPlatformName()` (0x007e98f0): the string at `PTR_DAT_010d050c` = "PC" (`.rdata` 0x01029f90).
fn get_platform_name(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_str("PC")])
}

/// `SmoothStep(a, b, x)` (0x0067a410): t = (x − a) / (b − a) with a, b as floats (x87, stored as a float),
/// clamped to [0, 1] (`.rdata` 0x00f8a820 = 1), then t² (3 − 2t) in single precision (0x00f92eb0 = 3, 0x00f8a808 = 2).
fn smooth_step(_: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let a = tonumber(&args, 1) as f32;
    let b = tonumber(&args, 2) as f32;
    let x = tonumber(&args, 3);
    let t = ((x - a as f64) / (b as f64 - a as f64)) as f32;
    // `fldz; fcomip`: 0 > t → 0 (NaN falls through to the upper test, which is false too).
    let t = t.clamp(0.0, 1.0);
    Ok(vec![Value::Number(((3.0f32 - t * 2.0) * (t * t)) as f64)])
}

/// The language codes behind the language-select menu's indices (`FUN_007f4220` / `FUN_007e9720`).
const LANGUAGE_BY_INDEX: [u32; 9] = [1, 3, 5, 6, 4, 0xd, 0xe, 7, 8];

/// `SetLanguage(index)` (0x007f5060 → `FUN_007f4220`): index 0..8 → language code (others → 1); sends message
/// 0xd {0, code} (`FUN_007feef0`) with bit 3 of `*DAT_010edb0c` set during the send. Here:
/// [`Request::SetLanguage`]; the state's language changes with it (model decision: the message's handler is
/// the engine's).
fn set_language(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let index = int(&args, 1);
    let code = usize::try_from(index).ok().and_then(|i| LANGUAGE_BY_INDEX.get(i)).copied().unwrap_or(1);
    state.screen.language = code;
    state.requests.push(Request::SetLanguage { code });
    Ok(vec![])
}

/// `GetInitialLanguageSelectLanguage()` (0x007e9720): the current language code → menu index (unknown → 0).
fn get_initial_language_select_language(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let index = LANGUAGE_BY_INDEX.iter().position(|&c| c == state.screen.language).unwrap_or(0);
    Ok(vec![push_int(index as i32)])
}

fn conductor_mut<'a>(state: &'a mut LogicState, name: &str) -> Option<&'a mut Conductor> {
    let h = mw_formats::genesys::field_hash(name);
    state.screen.conductors.iter_mut().find(|c| c.name_hash == h)
}

fn conductor_idle(state: &LogicState, name: &str) -> bool {
    let h = mw_formats::genesys::field_hash(name);
    state.screen.conductors.iter().find(|c| c.name_hash == h).is_none_or(|c| c.playing.is_none())
}

/// `CreateConductor(name)` (0x007f9db0): a conductor keyed by the name's field hash in WM+0x478 (`FUN_00800140`,
/// which keeps an existing one), registered with the feedback system (UIM+0x47380, `FUN_00a42850`).
fn create_conductor(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let name_hash = mw_formats::genesys::field_hash(&name);
    if conductor_mut(state, &name).is_none() {
        let at = state.screen.conductors.partition_point(|c| c.name_hash < name_hash);
        state.screen.conductors.insert(at, Conductor { name_hash, name: name.clone(), playing: None });
    }
    state.requests.push(Request::CreateConductor { name });
    Ok(vec![])
}

/// `DestroyConductor(name)` (0x007f9710): stops it (`FUN_00a43ff0`, `FUN_00a42a40`) and removes it
/// (`FUN_007ffa00`).
fn destroy_conductor(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let h = mw_formats::genesys::field_hash(&name);
    state.screen.conductors.retain(|c| c.name_hash != h);
    state.requests.push(Request::DestroyConductor { name });
    Ok(vec![])
}

/// `StartSequence(sequence binding, conductor)` (0x007f9780): the binding is parsed with the binding manager;
/// when it parses, `FUN_007f7740` resolves it to a sequence handle (visitor 0x00fe0e78) and plays it on the
/// conductor (`FUN_00a43c10`).
fn start_sequence(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let sequence = string_or_invalid(&args, 1);
    let conductor = string_or_invalid(&args, 2);
    if let Some(c) = conductor_mut(state, &conductor) {
        c.playing = Some(sequence.clone());
    }
    state.requests.push(Request::StartSequence { conductor, sequence });
    Ok(vec![])
}

/// `StopSequence(conductor)` (0x007f98f0 → `FUN_00a42b60`).
fn stop_sequence(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let conductor = string_or_invalid(&args, 1);
    if let Some(c) = conductor_mut(state, &conductor) {
        c.playing = None;
    }
    state.requests.push(Request::StopSequence { conductor });
    Ok(vec![])
}

/// `HasSequenceFinished(sequence binding, conductor)` (0x007f9800 → `FUN_007f8fa0`): true when the binding
/// does not parse or the conductor is idle (which sequence is not checked).
fn has_sequence_finished(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let _sequence = string_or_invalid(&args, 1);
    let conductor = string_or_invalid(&args, 2);
    Ok(vec![Value::Bool(conductor_idle(state, &conductor))])
}

/// `IsConductorIdle(conductor)` (0x007f9860). An unknown conductor (a null dereference in the original) is
/// idle.
fn is_conductor_idle(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let conductor = string_or_invalid(&args, 1);
    Ok(vec![Value::Bool(conductor_idle(state, &conductor))])
}

/// `RequestDialog(id)` (0x007e9c60 → `FUN_007bb330`): opens the dialog definition with that id (+0x20), first
/// among the current screen's, then the global list (`FUN_007bb020`).
fn request_dialog(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1) as u32;
    if !state.screen.dialogs.open.contains(&id) {
        state.screen.dialogs.open.push(id);
    }
    state.requests.push(Request::ShowDialog { id });
    Ok(vec![])
}

/// `GetDialogResult(id)` (0x007e9ca0 → `FUN_007b83f0`): the dialog's result byte, which is then removed
/// (`FUN_007c0400`); −1 (0xff, sign-extended) when there is none.
fn get_dialog_result(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1) as u32;
    let d = &mut state.screen.dialogs;
    let r = match d.results.binary_search_by_key(&id, |&(i, _)| i) {
        Ok(i) => d.results.remove(i).1,
        Err(_) => -1,
    };
    Ok(vec![push_int(r as i32)])
}

/// `ClearDialog(id)` (0x007e9d30 → `FUN_007b3a10`): closes the open dialog with that id; when none is left the
/// front-end control groups come back (`FUN_0083c960(3, 5)`, `FUN_0083c930(2|1|6, 5)`).
fn clear_dialog(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1) as u32;
    let d = &mut state.screen.dialogs;
    if let Some(i) = d.open.iter().position(|&o| o == id) {
        d.open.remove(i);
        state.requests.push(Request::CloseDialog { id, last: state.screen.dialogs.open.is_empty() });
    }
    Ok(vec![])
}

/// The platform text-entry object ((G+0x2f454)+8, `FUN_0069fcd0`): the OS / overlay keyboard.
#[derive(Debug, Default)]
pub struct TextEntry {
    /// +0x54 (0x80 wide characters).
    pub title: String,
    /// +0x154 (0x100).
    pub description: String,
    /// +0x354 (0x200): the initial text.
    pub text: String,
    /// +0x754.
    pub password: bool,
    /// +0x48: what `GetTextEntryState` returns; the runtime sets it.
    pub state: i32,
    /// +0x38fc: what `GetEnteredText` returns; the runtime sets it.
    pub entered: String,
}

fn chars(s: &str, n: usize) -> String {
    s.chars().take(n - 1).collect()
}

/// `StartTextEntry(title, description, text, max length, password [, flag bit])` (0x007f2380 →
/// `FUN_0081caa0`): the texts to wide strings, the keyboard opened (`FUN_00845d60`) with the length and the
/// flags byte `1 << (bit & 31)` (the 6th argument rounded to nearest; 0 without a 6th argument or bit 0).
fn start_text_entry(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let title = chars(&string_or_invalid(&args, 1), 0x80);
    let description = chars(&string_or_invalid(&args, 2), 0x100);
    let text = chars(&string_or_invalid(&args, 3), 0x200);
    let max_len = int(&args, 4);
    let password = toboolean(&args, 5);
    let bit = if args.len() > 5 { round_int(&args, 6) } else { 0 };
    let flags: u8 = if bit != 0 { (1u32 << (bit & 31)) as u8 } else { 0 };
    let t = &mut state.text_entry;
    t.title = title.clone();
    t.description = description.clone();
    t.text = text.clone();
    t.password = password;
    state.requests.push(Request::StartTextEntry { title, description, text, max_len, flags });
    Ok(vec![])
}

/// `GetTextEntryState()` (0x007e9b70 → `FUN_0081cb20`: +0x48).
fn get_text_entry_state(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(state.text_entry.state)])
}

/// `GetEnteredText()` (0x007e9bc0: the string at +0x38fc).
fn get_entered_text(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_str(&state.text_entry.entered)])
}

pub fn register(r: &mut Registry) {
    r.set_lua("print", print);
    r.set_lua("ResolveBoolBinding", resolve_bool_binding);
    r.set_lua("ResolveIntBinding", resolve_int_binding);
    r.set_lua("ResolveDoubleBinding", resolve_double_binding);
    r.set_lua("ResolveStringBinding", resolve_string_binding);
    r.set_lua("ResolveWideStringBinding", resolve_wide_string_binding);
    r.set_lua("ExitWidgetGroup", exit_widget_group);
    r.set_lua("SetBoolVariable", set_bool_variable);
    r.set_lua("SetIntVariable", set_int_variable);
    r.set_lua("SetDoubleVariable", set_double_variable);
    r.set_lua("SetStringVariable", set_string_variable);
    r.set_lua("SetUniqueIdVariable", set_unique_id_variable);
    r.set_lua("DeleteVariable", delete_variable);
    r.set_lua("DoRpc", do_rpc);
    r.set_lua("SetModalMode", set_modal_mode);
    r.set_lua("GetPlatformName", get_platform_name);
    r.set_lua("SmoothStep", smooth_step);
    r.set_lua("SetLanguage", set_language);
    r.set_lua("GetInitialLanguageSelectLanguage", get_initial_language_select_language);
    r.set_lua("CreateConductor", create_conductor);
    r.set_lua("DestroyConductor", destroy_conductor);
    r.set_lua("StartSequence", start_sequence);
    r.set_lua("StopSequence", stop_sequence);
    r.set_lua("HasSequenceFinished", has_sequence_finished);
    r.set_lua("IsConductorIdle", is_conductor_idle);
    r.set_lua("RequestDialog", request_dialog);
    r.set_lua("GetDialogResult", get_dialog_result);
    r.set_lua("ClearDialog", clear_dialog);
    r.set_lua("StartTextEntry", start_text_entry);
    r.set_lua("GetTextEntryState", get_text_entry_state);
    r.set_lua("GetEnteredText", get_entered_text);
}
