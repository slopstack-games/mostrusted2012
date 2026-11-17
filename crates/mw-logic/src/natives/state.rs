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
    /// What the runtime reports about the engine ([`Engine`]).
    pub engine: Engine,
    /// G (`*0x01177868`): the fields the hooks touch directly.
    pub game: super::game_logic::GameLogic,
    /// `GameLogicConfig` fields the hooks read (filled by the runtime from the install).
    pub config: super::game_logic::LogicConfig,
    /// GMM (G+0x2f3fc).
    pub game_modes: super::game_modes::GameModeManager,
    /// PM (G+0x2f3dc).
    pub players: super::players::PlayerManager,
    /// The progression database and the profile manager (G+0x14c08).
    pub progression: super::progression::Progression,
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
    /// UIM (G+0x2f480), the Lua natives' fields.
    pub ui_manager: super::lua_career::UiManagerFields,
    /// G+0x2f418.
    pub challenges: super::lua_career::ChallengeFields,
}

/// Engine facts the logic reads, set by the runtime (or a tool) before the logic runs.
#[derive(Debug)]
pub struct Engine {
    /// `FUN_00897960` on `*0x010edafc`: the graphics are prepared (`BWAIT4GFX`).
    pub graphics_prepared: bool,
    /// The loading-screen manager's request mask (UI manager +0x2d204) is nonzero and +0x2d200 clear
    /// (`HSB_WAIT4LS` / `HSB_WAIT4NL`).
    pub loading_screen_showing: bool,
    /// The UI clock (`*0x010edacc` → +4 → +0x18): ticks at 3,000,000 per second (i64 at +8) and the u32 at +0,
    /// advanced by [`LogicHost::frame`](super::LogicHost::frame).
    pub ui_ticks: i64,
    pub ui_counter: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { graphics_prepared: true, loading_screen_showing: false, ui_ticks: 0, ui_counter: 0 }
    }
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub). Each names the original function that
/// does the work; the runtime does what that function does and updates [`LogicState`] where the doc says so.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {
    /// `FUN_0068fd50` (`SetNewGame`, `SetIntroEventNext`), the parts outside [`LogicState`]. New game:
    /// `FUN_0072bf20` (reset the progression tables), `FUN_0068e5c0` (the single-player rollout), the profile's
    /// car from `default_rollout` (`FUN_00683480` / `FUN_00729f20`), `FUN_00707380`, `FUN_00655e90`,
    /// `FUN_00847d70`. Continue: `FUN_00847d70`, and when `saved_position` (`last_played`[0xd]) ≥ 0 the saved
    /// position (`FUN_0083c7b0` → `FUN_00847780`). Both: the world's collectables (`FUN_0070d260`,
    /// `FUN_007000d0`), the recommendations (`FUN_007355b0`(1, 0), `FUN_007357f0`(0)), GMM+0x2feb
    /// (`FUN_006ecfa0`, `GameLogicConfig` +0x1f0).
    NewGame { new_game: bool, default_rollout: u32, saved_position: i64 },
    /// `FUN_0068e5c0` on the rollout manager G+0x2f404: the single-player car and its rollout.
    SetSinglePlayerRollout,
    /// `FUN_00692550`: the car and rollout from the profile.
    SetRolloutFromProfile,
    /// `FUN_006f90c0` (GMM: the game state into the save) and `FUN_00733c70` (the progression database's
    /// snapshot, the save time); then the save itself.
    SaveProfile,
    /// `FUN_0086e0a0`: the settings (G+0x2f3e0)+0x4210 refresh their four pages from the profile.
    PopulateSettings,
    /// The settings are written to the profile and applied: the hook `SaveSettings` (`FUN_0086fe20`) and the Lua
    /// native `SaveSettings` (its commit: `FUN_0071c1c0`, `FUN_0046efe0`, `FUN_00414e50`).
    SaveSettings,
    /// `DAT_011a2470` = 1: the game quits.
    ShutdownGame,
    /// The mode timer started (`FUN_006f0f80`: the event's `Timeline` sequences, the timeout, the wrong-way
    /// state) or stopped (`FUN_006f1120`); GMM+0x2e74 already holds `running`.
    GameModeTimer { running: bool },
    /// `FUN_006f89f0`: the game mode resets.
    ResetGameMode,
    /// `FUN_00726ca0`(event) on the recommendations (Autolog's recommended event after a retry).
    RecommendationRetry { event: u32 },
    /// `FUN_00673d70`: the recommendations' event for the available vehicles (`FUN_00723b10` / `FUN_00725130`),
    /// else `FUN_006e74f0`(that id, 0) on the GMM ([`GameModeManager::request_event`](super::game_modes::GameModeManager::request_event)).
    SetEventFromAvailableVehicles,
    /// `FUN_0067f700`: the pursuit and event statistics restart (`FUN_00659d00`(1), `FUN_00656920`,
    /// `FUN_006600d0`(event, mode, online), `FUN_00656f60`(the car)).
    ResetCurrentEventStats { event: u32, mode: u32, online: bool },
    /// `FUN_0067f570`: the time of day from the current event.
    SetTimeOfDayFromEvent,
    /// `FUN_006f4960`: posts the time-of-day sync message (offline only).
    SyncTimeOfDay,
    /// `FUN_006fbb60`: collect the post-event awards into the GMM ([`GameModeManager`](super::game_modes::GameModeManager)
    /// `award_amount`, `award_2ce8`, `awards`).
    PreparePostEventAwards,
    /// `FUN_006e65e0`: `FUN_00736a90`(award_2ce8, event, amount) when amount > 0; each award: `FUN_00736ab0`(award,
    /// event) when `FUN_0071f7e0` knows it, else `FUN_006792b0`(award).
    AwardPostEventAwards { event: u32, amount: i32, award_2ce8: u32, awards: Vec<u32> },
    /// `FUN_00686f30`: the event's AI racers are added (slots 12..23).
    AddAllAIPlayers,
    /// `FUN_006d1310`: each slot's vtable +0x20, the AI slots 12..23 removed (`FUN_006ce180`(slot, 3)), PM+0xa660 /
    /// +0xa694 / +0xa698 = 0.
    RemoveAllRaceCars,
    /// `FUN_006ce180`(slot, reason): the player in the slot is removed (the runtime clears the slot).
    RemovePlayer { slot: usize, reason: u32 },
    /// `FUN_008456d0` on G+0x2f484: every roadblock is removed.
    RemoveRoadblocks,
    /// `FUN_00846830`: message 0x15, the props reset.
    ResetProps,
    /// `FUN_006c9fb0`(0, 0) on the local player: respawn at `event`'s start (0 = where the player is) or at the
    /// last swap spot (decomp.md "Start position").
    Respawn { event: u32, to_swap_spot: bool },
    /// `FUN_0065b830`(the pursuit manager G+0x2f410, 1): the chase statistics restart.
    ResetChaseStats,
    /// `FUN_00844940`(vehicle): message 0x16, the vehicle's damage is reset.
    ResetVehicleDamage { vehicle: i32 },
    /// `FUN_0068acb0` (inactive) / `FUN_0068ad50` (active): the local car's damage model.
    VehicleDamage { active: bool },
    /// `FUN_006926b0`: the local car's rollout re-applied clean (`FUN_0068e9d0`(…, 2)) and its damage reset.
    CleanAndRepairVehicle,
    /// `FUN_0065de30`(G+0x264e0, slot): the player's nitrous is emptied.
    ClearNitrous { slot: i32 },
    /// `FUN_0067f8c0`: the HUD's message queues are emptied (UIM+0x12458 records, +0x12468, +0x124f8, +0x12574).
    ClearHudMessageQueues,
    /// UIM+0x126e8's message freed, +0x126e0 = 0.
    ClearHudInstantQueue,
    /// `FUN_00a44dd0` on the sequence players UIM+0x47748 and +0x4774c.
    StopAllSequences,
    /// `FUN_008423a0` on the speech manager G+0x2c3c0.
    StopSpeech,
    /// `FUN_0079f760` on the song manager UIM+0x30370.
    SkipCurrentTrack,
    /// The UniqueId at `field` of the object behind the handle `[DAT_010edb08]+4` +0xb278 (type open), mapped by
    /// `FUN_007983b0` (UIM+0x32858) and started on the sequence player UIM+0x47748 (`FUN_00a45070`).
    PlayUiSound { field: u32 },
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
    // --- From HSM behaviours (`hsm::behaviours`; each names the behaviour and the original's call).
    /// `RUN_PG_RNNR2` enter (0x76f7f0 → `FUN_0083dc70` on the PageRunner2 at G+0x2ee60): push the PageRunner2
    /// context `name` with its default page `page` (the `DEF_PAGE_REF` id printed "%d"); a context of that name
    /// already on the stack is replaced. The runtime then answers `Query::PageContextExists` /
    /// `PageContextExit` / `PageContextBusy` for it.
    PushPageContext { name: String, page: String },
    /// `RUN_PG_RNNR2` exit (0x76eb50): first `FUN_0083b920` (→ `FUN_007f5f80` on the page runner; what it does
    /// is open), then, if the context is still on the stack, suspend it (`suspend`: `FUN_007e0c80` flags the top
    /// context +0x40 / +0x41, it stays) or pop it (`FUN_007f09d0` → `FUN_007e0cd0`).
    LeavePageContext { name: String, suspend: bool },
    /// `RUN_PG_RNNR2` with `RESTOREGRP` true, when it finishes: the UI manager's focus group (`*`(G+0x2f480)
    /// +0x31c04) = `group` (saved on enter, `Query::FocusGroup`), with +0x31c1c and +0x31c1e set (restore
    /// pending).
    RestoreFocusGroup { group: u64 },
    /// `ACTI_WDGT_GR` update (0x76e4e0 → `FUN_007f9350` on the screen manager's page, G+0x2f458 +0x280):
    /// activate the widget group `group` of the current page (its widgets show and take input).
    ActivateWidgetGroup { group: i64 },
    /// `ACTI_WDGT_GR` exit begin (0x76e5e0): play the page's `ScreenTransition` for leaving `group` through
    /// `exit` (`FUN_007f78a0`: a transition entry of the group whose exit point is `exit` or empty), else the
    /// group's own one (`FUN_007f58e0`); neither starts while the transitions' state (G+0x2f458 +0x4b0 +0x3ac)
    /// is 3. `Query::WidgetTransitionStarted` answers whether one started.
    PlayWidgetGroupTransition { group: i64, exit: u64 },
    /// `ACTI_WDGT_GR` exit update (0x76ffd0 → `FUN_007f9bf0` on the page): deactivate `group` (`instant`: no out
    /// animation); when the page's out time (+0x68) is 0 its widgets are released at once (`FUN_007f1b40` on
    /// page +8), else when the out animation ends. `Query::WidgetGroupOutTime` answers the out animation.
    DeactivateWidgetGroup { group: i64, instant: bool },
    /// `ACTI_WDGT_GR` update: the page's exit point (+0x208) was taken by this group: clear it.
    ClearWidgetExit,
    /// `SETBLACKBACK` enter (0x745b10): the UI manager's black background (UIM+0x2d260) on / off.
    SetBlackBackground(bool),
    /// `CHANGEVSYNCS` enter / exit (0x73b980, 0x73b9e0): G+0x2f723, the 60 Hz ("full rate") mode on / off (exit
    /// restores the value saved on enter).
    SetFullRate(bool),
    /// `MUTE_SOUND` enter (0x73e220) / exit (0x73e2a0): count a mute `reason` (`MUTEREASON` in the table at
    /// 0x010c5c28: 0 Unknown, 1 LoadingScreen, 2 CarSelect) up (`on`) or down (`FUN_00790920` on UIM+0x2d2f0:
    /// one byte per reason at +2999); all sound is muted while any count is nonzero (`FUN_004b8b20`).
    MuteSound { reason: u8, on: bool },
    /// `LOADWORLD` enter (0x74e040): load the world selected in the world list (G+0x2f468: entry +0x8c of the
    /// list +0x88): `FUN_008465c0` on G+0x2ef6c with it, and the load-world message (type 3, the world's name,
    /// `FUN_00844190` on G+0x2ef70 → `FUN_0084c960`).
    LoadWorld,
    /// `LOADPROFILE` enter update (0x73da30 → `FUN_0071f010`): start loading the profile (save system
    /// G+0x2f454: buffer cleared, a load request; +0x4942 = loading). `Query::ProfileLoaded` answers the end.
    LoadProfile,
    /// `LOADPROFILE` exit (0x744040 → `FUN_0073daf0` on G+0x2f454 +0x4964): apply the loaded profile's
    /// settings (audio volumes from the options record `FUN_0046efe0` +0x16c / +0x170 / +0x174 × 1/100 to the
    /// mixer channels 3–8 `FUN_004b8b90`, the UI manager's copies at +0x306c8, ...).
    ApplyProfileSettings,
    /// `HSB_P1BIND` (0x73e730): release the primary user (G+0x2f721: the sign-in manager sets the user record
    /// `*`(0x010edad8)+4 +0xb28c back to 1 = unbound) while `true`.
    ReleasePrimaryUser(bool),
    /// `HSB_P1BIND` update (0x744810): listen for the press that binds the primary user (G+0x2f720, user slot
    /// G+0x2f48c = 0; `any_button` G+0x2f722: any button, else start / enter) while `listen`.
    ListenForPrimaryUser { listen: bool, any_button: bool },
    /// `HSB_P1BIND` (0x73e730 → `FUN_006ce500(2)` on the players G+0x2f3dc): remove every player; players
    /// +0xa6b8 = −1.
    RemovePlayers,
    /// `HSB_P1BIND` (0x744810 → `FUN_006d6090(0, 0)`): create the local player for the bound user;
    /// `Query::LocalPlayerReady` answers when it exists (players +0xa6b8 is then set to 0).
    CreateLocalPlayer,
    /// `RUNGAMEMODE` enter (0x7450e0 → `FUN_00701cb0` on the game-mode manager G+0x2f3fc): start the event /
    /// game mode `event` (G+0x2f6f8): its mode's HSM machine runs (the runtime starts the instance from the
    /// mode's data and answers `Query::GameModeMachine` with its serial). Unless the local player exists with
    /// its byte G+0x25694[player +0xa0] set, also posts message 5 (`FUN_00846710`; meaning open).
    StartGameMode { event: u32 },
    /// `RUNGAMEMODE` exit while the mode still runs (0x73eed0): end reason 2 (GMM+0x2d08), the UI manager's
    /// mode lists freed (`FUN_0067f8c0`, `FUN_006796b0`), the current event cleared (`FUN_006e74f0(0, 0)`); the
    /// behaviour asks the mode's machine to exit itself.
    StopGameMode,
    /// `RUNGAMELOGIC` enter (0x75c870): play the sequence `sequence` (a sequence resource of a loaded bundle,
    /// `FUN_007983b0`; none found: nothing plays) on `player` (`Style` 0 = the UI manager's default player
    /// +0x4774c, 1 = the sim-timer player +0x47748 (also `UseSimTimer` true), 2–5 = the players at +0x12568 /
    /// +0x126dc / +0x1245c / +0x124ec, played with `FUN_00a45070`; `None` = no style: played on its own with
    /// `FUN_00a44f30`); `playback` names it for `Query::SequencePlaying` / [`StopPlayback`](Self::StopPlayback).
    PlaySequence { playback: u32, sequence: u32, player: Option<u8> },
    /// `RUNGAMELOGIC` exit (0x73edd0): stop the playback if it still plays (`FUN_00a44d50` / `FUN_00a43150`).
    StopPlayback { playback: u32 },
    /// `GAMEMODE` update (0x75e6f0), the online session was lost: end reason 2 (`FUN_006dff20`(2), GMM+0x2d08),
    /// `FUN_0067f9e0`, and G+0x2f70c = the disconnect reason (the config record's +0xe0, or +0xdc when
    /// `FUN_004c2050`); the behaviour exits through `DISCONNECT`.
    SessionLost,
    /// `LOAD_ALL` (0x740f00): open the gate `DAT_010f77ed` that world loading waits for.
    LoadAll,
    /// `BINGAME` enter update, once (0x73d130): post the in-game start messages: `FUN_008433f0(0, v)` on G+0x2f470
    /// (queue `DAT_010edb24`, v = profile option 11 (`FUN_0072f610(0xb)`) clamped to 0–3, kept at +4; then +8 set)
    /// and `FUN_00847d70` (queue `DAT_010edb38`).
    EnterGame,
    /// `BINGAME` exit update, once (0x73d1d0): remove the players (`FUN_006ce500(0)`), the local player's vtable
    /// +0x20 when it has one, reset the UI manager's two lists (`FUN_007b3410` ×2) and stop the two sequence
    /// players' sequences (`FUN_00a44dd0` ×2). `Query::GameLeaving` answers the wait.
    LeaveGame,
    /// `HSM_PLAYING` enter (0x73e7e0): `FUN_006fa720` on G+0x14e40 (meaning open), the session counters (G+0x2f3ec
    /// +0x130 = profile stat 9, +0x134 = 0); the "GAMESESSSTRT" telemetry is not ported.
    StartPlaying,
    /// `HSM_PLAYING` exit (0x73e930): reset the players' slots (`FUN_006d1310`), clear the loading-screen request
    /// bit 10 (UIM+0x2d204) and UIM+0x2d230; the "GAMESESSENDS" telemetry is not ported.
    StopPlaying,
}
