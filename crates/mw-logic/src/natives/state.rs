//! The original's managers, as far as the natives read and write them (one sub-struct per manager, in the file of
//! the area that owns it; fields name their offset in the original, as decomp.md does).

/// The logic state the natives act on.
#[derive(Debug, Default)]
pub struct LogicState {
    /// What the natives asked the engine to do, in call order; the runtime drains it every frame.
    pub requests: Vec<Request>,
    /// Calls of natives that are known but not ported yet (`"hook Name"` / `"lua Name"`), for logs and tools.
    pub unported: Vec<String>,
    /// What bindings read ([`ViewModels`](super::ViewModels)).
    pub view_models: super::ViewModels,
    /// The widget-screen manager (`*0x0117e9f4`): exit code, modal mode, conductors, dialogs, language.
    pub screen: super::lua_widgets::ScreenManager,
    /// The Lua script objects and their view models (`Set*Variable`).
    pub widget_scripts: super::lua_widgets::WidgetScripts,
    /// The platform text entry ((G+0x2f454)+8).
    pub text_entry: super::lua_widgets::TextEntry,
    /// The settings manager ((G+0x2f3e0)+0x4210).
    pub settings: super::lua_settings::SettingsManager,
    /// The post-FX settings ((G+0x2f3e0)+0x5ac8).
    pub post_fx: super::lua_settings::PostFxSettings,
    /// G's fields the Lua natives write.
    pub game: super::lua_career::GameFields,
    /// GMM (G+0x2f3fc), the Lua natives' fields.
    pub game_modes: super::lua_career::GameModeFields,
    /// UIM (G+0x2f480), the Lua natives' fields.
    pub ui_manager: super::lua_career::UiManagerFields,
    /// G+0x2f418.
    pub challenges: super::lua_career::ChallengeFields,
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {
    /// `DoRpc`: offer `function(args)` to the scripts of the current screen's widget named `widget` (the
    /// widget whose key is its field hash), Lua scripts with `args` split at `,` as string arguments, until one
    /// has the function; unknown widget: nothing. The original does it synchronously, inside the native.
    WidgetRpc { widget: String, function: String, args: String },
    /// `SetLanguage`: send the language-change message 0xd with this language code (`FUN_007feef0`).
    SetLanguage { code: u32 },
    /// `CreateConductor`: a conductor of feedback sequences registered with the feedback system.
    CreateConductor { name: String },
    /// `DestroyConductor`: stop and remove it.
    DestroyConductor { name: String },
    /// `StartSequence`: resolve `sequence` (a binding) to a feedback sequence and play it on the conductor;
    /// when it ends, clear the conductor's `playing` (`ScreenManager::conductors`).
    StartSequence { conductor: String, sequence: String },
    /// `StopSequence`.
    StopSequence { conductor: String },
    /// `RequestDialog`: open the popup dialog definition with this id (the screen's first, then the global
    /// ones); the chosen button goes to `Dialogs::set_result`.
    ShowDialog { id: u32 },
    /// `ClearDialog`: close it; `last`: no dialog is left open, the front-end control groups 1, 2, 3, 6 are
    /// enabled again (`FUN_0083c960` / `FUN_0083c930`).
    CloseDialog { id: u32, last: bool },
    /// `StartTextEntry`: open the platform keyboard; the runtime updates `LogicState::text_entry` (`state`,
    /// `entered`).
    StartTextEntry { title: String, description: String, text: String, max_len: i32, flags: u8 },
    /// `SaveSettings`: the settings were committed; write the profile / apply them (`FUN_0071c1c0`,
    /// `FUN_0046efe0`, `FUN_00414e50`).
    SaveSettings,
}
