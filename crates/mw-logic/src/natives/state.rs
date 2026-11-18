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
    /// Rollout record and online-object fields of the career Lua natives.
    pub career: super::lua_career::CareerFields,
    /// The menus' managers (`lua_menus.rs`).
    pub menus: super::lua_menus::MenuFields,
    /// The remaining offline natives' fields (`lua_misc.rs`).
    pub misc: super::lua_misc::MiscFields,
    /// The sim clock, camera and units fields (`lua_sim.rs`).
    pub sim: super::lua_sim::SimFields,
    /// The garage, ranks, awards and vehicle-focus fields (`lua_garage.rs`).
    pub garage: super::lua_garage::GarageFields,
    /// The easy-drive / event selection hooks' fields (`hooks_select.rs`).
    pub select: super::hooks_select::SelectFields,
    /// The UI sequence services' state (`UISequences` / `GeneralSequence` roots, `sequence_view.rs`).
    pub sequences: super::sequence_view::UiSequences,
    /// The entitlement and game-pack managers (EM G+0x29280, GPM G+0x2ab00): offline, base game only.
    pub entitlements: super::entitlements::Entitlements,
    /// The fields of the seventh twenty HSM hooks (`hooks_misc.rs`).
    pub hook_misc: super::hooks_misc::HookMiscFields,
    /// The fields of the eighth twenty HSM hooks (`hooks_online.rs`).
    pub hook_online: super::hooks_online::HookOnlineFields,
    /// The fields of the ninth twenty HSM hooks (`hooks_playlist.rs`).
    pub hook_playlist: super::hooks_playlist::HookPlaylistFields,
    /// The screen-transition manager (`Transitions` root, `transitions_view.rs`).
    pub transitions: super::transitions_view::Transitions,
    /// The profile access bytes of G+0x2f454 (`GameLogicState.IsAccessingProfile`, `game_logic_view.rs`).
    pub profile_io: super::game_logic_view::ProfileIo,
    /// `CHECKPTS`'s checkpoint manager (`checkpoints.rs`): runtime inputs and the logic's progress.
    pub checkpoints: super::checkpoints::Checkpoints,
    /// `EVENTSTARTS`'s event-start markers (`event_starts.rs`).
    pub event_starts: super::event_starts::EventStarts,
    /// `HSMCARSWAPLO`'s jack spots and swap (`car_swap.rs`).
    pub car_swap: super::car_swap::CarSwap,
}

/// The binding host the logic reads through: the game-mode manager's `Mode` root ([`super::mode_view`]) first,
/// then the view models (registered roots, `Signals`, the published data definitions). A `Mode` member the table
/// does not answer falls through to a data definition published as "Mode" (the original's dispatch `FUN_00a4ef40`).
impl crate::binding::Host for LogicState {
    fn lookup(&self, path: &[crate::binding::Segment]) -> crate::binding::Lookup {
        use crate::binding::Lookup;
        if let Some((root, members)) = path.split_first()
            && let Some((_, f)) = ROOTS.iter().find(|(h, _)| *h == root.hash)
        {
            let r = f(self, members);
            if matches!(r, Lookup::Found(_) | Lookup::Unsupported) {
                return r;
            }
        }
        self.view_models.lookup(path)
    }
}

type RootFn = fn(&LogicState, &[crate::binding::Segment]) -> crate::binding::Lookup;

/// The registered roots the logic answers itself, by root hash (one module per root, `natives/<root>_view.rs`).
static ROOTS: std::sync::LazyLock<Vec<(u32, RootFn)>> = std::sync::LazyLock::new(|| {
    let h = crate::binding::hash;
    vec![
        (h("Mode"), super::mode_view::lookup as RootFn),
        (h("ModeVM"), super::modevm_view::lookup),
        (h("Net"), super::net_view::lookup),
        (h("JoinGameState"), super::online_state_view::lookup),
        (h("CreateMatchState"), super::online_state_view::lookup),
        (h("MatchMakeState"), super::online_state_view::lookup),
        (h("UIConfig"), super::ui_config_view::lookup),
        (h("SignInState"), super::sign_in_view::lookup),
        (h("UISequences"), super::sequence_view::ui_lookup),
        (h("GeneralSequence"), super::sequence_view::general_lookup),
        (h("EntitlementManager"), super::entitlement_view::lookup),
        (h("Players"), super::players_view::lookup),
        (h("Event"), super::event_view::lookup),
        (h("Transitions"), super::transitions_view::lookup),
        (h("GameLogicState"), super::game_logic_view::lookup),
    ]
});

impl LogicState {
    /// `ResolveBoolBinding` / an HSM condition: [`super::ViewModels::resolve_bool`]'s rules over every root.
    pub fn resolve_bool(&self, binding: &str) -> Option<bool> {
        crate::binding::Binding::parse(binding).ok().map(|b| b.eval_bool(self).value)
    }
    /// `ResolveIntBinding` ([`super::ViewModels::resolve_int`]'s rules).
    pub fn resolve_int(&self, binding: &str) -> Option<i64> {
        crate::binding::Binding::parse(binding).ok().map(|b| b.eval_int(self).value as i64)
    }
    /// `ResolveDoubleBinding`.
    pub fn resolve_double(&self, binding: &str) -> Option<f64> {
        crate::binding::Binding::parse(binding).ok().map(|b| b.eval_double(self).value)
    }
    /// `ResolveStringBinding`: `None` when it does not parse or resolve.
    pub fn resolve_string(&self, binding: &str) -> Option<String> {
        let r = crate::binding::Binding::parse(binding).ok()?.eval_text(self);
        r.ok.then_some(r.value)
    }
    /// The HSM's string reads (`FUN_00a4f2e0`, the string visitor).
    pub fn resolve_hsm_string(&self, binding: &str) -> Option<String> {
        let r = crate::binding::Binding::parse(binding).ok()?.eval_string(self);
        r.ok.then_some(r.value)
    }
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
    /// `FUN_0068acb0` (`deactivated` = 1, `DeactivateDamage`) / `FUN_0068ad50` (0, `ReactivateDamage`): message 0x1f
    /// `{vehicle, deactivated}` through `FUN_0069a5e0`, sent only when the local car is loaded.
    VehicleDamage { vehicle: i32, deactivated: bool },
    /// `FUN_006926b0`: the local car's rollout (vtable +0x18, 0x2b dwords, two fields zeroed) re-applied clean
    /// (`FUN_0068e9d0`(player +0x98, copy, 2)), then, when it has a vehicle (+0xa0), `FUN_00844940`(vehicle):
    /// message 0x16, its damage reset. `vehicle` −1: none.
    CleanAndRepairVehicle { vehicle: i32 },
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
    /// `ClearPendingResets` (0x676080): `FUN_0063d410`(G+0x25690, i) for i in 0..16 (clears the pending reset of
    /// vehicle slot i; when it is the local car's: UIM+0x2d204 bit 12 and +0x2d238 cleared).
    ClearPendingResets,
    /// `ForceKeyboardShutdown` (0x677060) -> `FUN_00845dc0`: bit 3 of `*DAT_010edb0c` set, message {0xf, 0}
    /// posted (`FUN_0084dcc0`), bit cleared: closes the on-screen keyboard.
    ForceKeyboardShutdown,
    /// `TransferCurrentHeatToVehicle` (0x6761c0): the local player's current vehicle id (`FUN_006ba320`: the
    /// vtable +0x18 object's +0x30); when non-zero `FUN_0072a280`(the profile's "garage" table (G+0x2f454)+0x4964,
    /// id, 0): the "heat" column of that vehicle's row is set to 0.
    TransferCurrentHeatToVehicle,
    /// `SetCollectablesNotInWorld` (0x676330): `FUN_0070d260` then `FUN_007000d0` on the collectables
    /// manager G+0x14e40.
    SetCollectablesNotInWorld,
    /// `SetRollout` (0x68d2f0): `liRolloutId` → `FUN_0068bbf0`(the rollout manager G+0x2f404, id): the rollout
    /// with that id is copied into the local player's slot record and applied.
    SetRollout { rollout: i32 },
    /// `SetValidProgressionData` (0x687750): `FUN_00686e70`(G+0x2f3e4) (11 entries zeroed), its +0xa4 = `valid`,
    /// G+0x2f3e8's +0x70 = 0.
    SetValidProgressionData { valid: bool },
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
    /// `DBAUTOSELWD` (0x7702e0): select the world without asking (`FUN_00645780` on the world list G+0x2f468 with
    /// the debug world of the GeneSys config, else the list's only world).
    AutoSelectWorld,
    /// `COLLECTABLES` enter update (0x73c2f0), once the world state is 3: `FUN_0070d260` on G+0x14e40 (resets the
    /// collectables' state and recounts them per player slot).
    StartCollectables,
    /// `COLLECTABLES` exit (0x73c330): `FUN_006ecf80` on G+0x14e40 (+0x2fe6 = 0, `FUN_006e42a0`, +0x41c = 0).
    StopCollectables,
    /// `LOAD_FDBK_GR` enter (0x743ee0): `FUN_007982e0` on the UI manager +0x32858: load the feedback message
    /// group (count it up when loaded, else start loading it with G+0x2ed60).
    LoadFeedbackGroup { group: i64 },
    /// `LOAD_FDBK_GR` exit (0x743ff0) with `UNLOADONEXIT` true: `FUN_00798370` counts the group down.
    UnloadFeedbackGroup { group: i64 },
    /// `ENABLETIME` enter / exit (0x73c4d0 / 0x73c4e0): G+0x25624 += `delta` (+1 / -1), a counter (meaning open;
    /// "enable time" by the behaviour's name).
    AdjustTimeEnabled { delta: i32 },
    /// `SELECT_PROF` (0x73f270): name the profile after the platform user: `FUN_00428860` prints `%llu` of the
    /// persona id into the buffer `DAT_010ecc10`.
    SetProfileUser { user: u64 },
    /// `SELECT_PROF` exit (0x75cd00): post message 0xc (`FUN_0076aad0` on the queue `DAT_010edb0c` +0x10, flag 1).
    ProfileUserSelected,
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
    /// `SelectChallengeVehicleSet`: `FUN_006f3f90` on G+0x2f41c recounts the challenge page's entries for the new
    /// set (`ChallengeFields::challenge_vehicle_set`).
    RecountChallengeVehicles,
    /// `SelectVehicleCategory`: `FUN_0068b7c0` rebuilds the vehicle list for `MenuFields::vehicle_category`.
    RebuildVehicleList,
    /// `SetSelectedVehicleColour`: `FUN_00657b50` (the selected vehicle's record gets the colour, the preview
    /// car is repainted by `FUN_00844660`).
    SetSelectedVehicleColour { colour: i32 },
    /// `SetVehicleModifier`: `FUN_00657d10`(kind, item): the customisation of the selected vehicle changes, its
    /// preview refreshes (`FUN_0068e000`).
    SetVehicleModifier { kind: i32, item: i32 },
    /// `SelectPack`: `FUN_00642a90`(id): the pack with that id (0: the default) is the selected one.
    SelectPack { id: u32 },
    /// `UpdateAwardProgressBindings`: `FUN_0071fd20`(-1) and (6) refresh the award progress bindings.
    UpdateAwardProgressBindings,
    /// `UpdateDeferredViewModel` (0x007f96a0 → `FUN_007f72f0` on the screen manager's page, `*0x0117e9f4`+0x280): for
    /// the page's deferred view-model definition named `name`, start its load (`FUN_00823b20` → `FUN_00820550`
    /// on G+0x2f42c). The runtime answers by setting `MiscFields::deferred_ids[name]` (the id the native returns).
    UpdateDeferredViewModel { name: String },
    /// `ClearViewModel` (`FUN_0081ec00`): the loaded view model `name` is released (its entry swapped with the
    /// last, `ViewModels` data freed); the downloads of that name are already updated in [`LogicState`].
    ReleaseViewModel { name: String },
    /// `SetCameraFov` / `ResetCameraFov` (0x0067bab0 / 0x00677ff0 → `FUN_00843ca0`(G+0x2f470, 0, set, fov)): the
    /// camera message 0xf; the camera's reaction is the runtime's.
    CameraFov { set: bool, fov: f32 },
    /// `SetUseOverheadCamera` (0x0067bc20 -> `FUN_008439a0`(G+0x2f470, 0, on)): the camera message 0xd.
    CameraOverhead { on: bool },
    /// `SetUseFinishLineCamera` (0x0067bc70 -> `FUN_008439f0`(G+0x2f470, 0, on, id)): the camera message 0xe; `id`
    /// is the second argument when `on`, else -1.
    CameraFinishLine { on: bool, id: i32 },
    /// `SnapCameraToBehindCar` (0x00677f70 -> `FUN_00843cf0`(G+0x2f470, 0)): the camera message 0x1.
    CameraSnapBehind,
    /// `EnableLocalRaceCarRumble` (0x0067b450 -> `FUN_00844900`(on)): the message 9 (rumble on / off).
    RumbleEnable { on: bool },
    /// `StopAllSounds` / `StopSpeech` / `StopGameplayStream` (0x00678420 / 0x006784e0 / 0x006784a0 ->
    /// `FUN_007a2140`(UIM+0x2d2f0, kind)): stop the sounds of one kind (0 = all, 2 = speech, 4 = gameplay stream).
    StopSounds { kind: u32 },
    /// `LoadFeedbackGroup` / `UnloadFeedbackGroup` (0x0067de60 / 0x0067deb0 -> `FUN_007982e0` / `FUN_00798370` on
    /// UIM+0x32858): load or release the feedback group `id`.
    FeedbackGroup { load: bool, id: i32 },
    /// `RemoveMessage` (0x0067dce0 -> `FUN_007b6130` on UIM+0x17e0): drop the HUD message `id`.
    RemoveMessage { id: i32 },
    /// `StopSound` (0x0067e160 -> `FUN_007a2100` on UIM+0x2d2f0): stop the sound `id`.
    StopSound { id: i32 },
    /// `ApplyMixSnapshotSet` / `ClearMixSnapshotSet` (0x0067e3e0 / 0x006785d0 -> `FUN_00798fb0`(UIM+0x2d2f0, id, on)).
    MixSnapshot { id: i32, on: bool },
    /// `PlayMusic` (0x0067e210 -> `FUN_0079f7b0` on UIM+0x30370).
    PlayMusic { id: i32 },
    /// `PlaySpeech` (0x0067e260 -> `FUN_008408a0`(G+0x2c3c0, a, b, c, 0)).
    PlaySpeech { a: i32, b: i32, c: i32 },
    /// `StopTraffic` / `ResetTraffic` / `StartTraffic` (0x00678240 / 0x00678270 / 0x006782a0 -> `FUN_008461b0` /
    /// `FUN_00846240` / `FUN_00846270`): the message 6 / 8 / 0xa (argument 0) sent to the object at `*0x010edb30`+0x10
    /// (`FUN_0084e350` / `FUN_0084e4d0` / `FUN_0084e550`); the traffic system's reaction is the runtime's.
    TrafficMessage { kind: u32 },
    /// `SetRaceCarInvulnerable` / `SetRaceCarDamageAllowed` (0x0068b010 / 0x0068b070 -> `FUN_0069a5e0` /
    /// `FUN_0069a6d0` on `*0x010edb2c`+0x10): a message (kind 0x1f / 0x1e) with the competitor and the flag.
    RaceCarFlag { kind: u32, competitor: i32, on: bool },
    /// `SetTrafficDensity(on, density)` (0x0067da40 -> `FUN_00845fb0` on G+0x2ef64): the density is 0 unless `on`.
    TrafficDensity { on: bool, density: f32 },
    /// `RefreshGarageItemsCache` (0x006759c0 -> `FUN_00701ca0` on G'+0x14c08).
    RefreshGarageItemsCache,
    /// `UnregisterSpeechCommand(id)` (0x007ea7f0 -> `FUN_00792c90` on UIM+0x32dd8).
    UnregisterSpeechCommand { id: i32 },
    /// `StopMusic` (0x00678460 -> `FUN_007908e0` on UIM+0x30370).
    StopMusic,
    /// `StopUnwantedSound` (0x00678520): `FUN_008404c0`(G+0x2c3c0), the byte G+0x2ae19 = 0, then
    /// `FUN_00799060`(UIM+0x2d2f0, 0).
    StopUnwantedSound,
    /// `ClearAllFeedbackQueues` / `ClearAllNonBackgroundFeedbackQueues` (0x00678580 / 0x00682750): `FUN_007b3410`
    /// (UIM+0x7230), then `FUN_007b1120` (every queue) or `FUN_007b1150` with the mask ~2 (`background` false:
    /// the background queue is kept) on UIM+0x30840.
    ClearFeedbackQueues { background: bool },
    /// `StartActionBind` (0x007eb200 → `FUN_00878790` on the settings (OM+0x4210)): wait for a control press to
    /// bind `action` (+0x1beb4) with `arg` (+0x1beb8); the state change 4 is posted (`FUN_0087ed10`) and the
    /// bind's start time (`FUN_0046ee00` +8 / +0xc) kept at +0x1bec8 / +0x1bed0.
    StartActionBind { action: i32, arg: i32 },
    /// `PrepareControlSettings` (0x007eb140 → `FUN_00879010`): the control page is built (buffer 0x1bed8 bytes
    /// allocated, `FUN_0086ea10`, `FUN_008781d0`); [`ControlSettings`](super::lua_misc::ControlSettings) is set.
    PrepareControlSettings,
    /// `ReleaseSettings` (0x007eb170 → `FUN_00879c70`): `FUN_008795c0` and `FUN_008721e0` before the buffer is
    /// freed (the state is in [`ControlSettings`](super::lua_misc::ControlSettings)).
    ReleaseControlSettings,
    /// `ChangeVehicleOnCustomRollout` (0x00692760 → `FUN_00691110` on the rollout manager G+0x2f404): try the
    /// vehicle as the custom rollout (`FUN_00691050`), else rebuild the default one for it (`FUN_00690b10`).
    ChangeVehicleOnCustomRollout { vehicle: i32 },
    /// `PRELOADCOPS` (`FUN_0074f640`): load the bundle `VEHICLES\VEH_<vehicle>_HI.BNDL` (`hi`, the local player's
    /// car) or `..._LO.BNDL` (the cops and other players) with its `GRAPHICS`, `PHYSICS`, `SOUNDGLOBAL` and `SOUND`
    /// parts; `Query::VehicleStreamBusy` answers while it runs.
    StreamVehicle { vehicle: u32, hi: bool },
    /// `PRELOADCOPS` (`FUN_00744cd0`): release that bundle again.
    ReleaseVehicle { vehicle: u32, hi: bool },
    /// `HSB_RNLAYOUT` enter (0x74fc80 → `FUN_007a2a00` → `FUN_007a0ba0` on the UI manager): run the UI layout
    /// `layout` in a layout slot, `layer` 0x18 with `RUN_HIGHEST`, else 0; the slot is named by the layout id
    /// afterwards (the original keeps the slot handle).
    RunLayout { layout: i64, layer: u32 },
    /// `HSB_RNLAYOUT` exit update (`FUN_00791d60`(slot, 0)): deactivate the layout (its slot's vtable +0x20 with
    /// false, when active).
    DeactivateLayout { layout: i64 },
    /// `HSB_RNLAYOUT` exit (0x745360 → `FUN_00791a10`): free the layout's slot.
    ReleaseLayout { layout: i64 },
    /// `LIST_INVITE` (0x74dd80): take the pending invite (the listener's vtable +0xc) and set G+0x2f70c from the
    /// GeneSys config (the object at config +0x34, its +0x94).
    TakeInvite,
    /// `REQTRAFRSTRT` enter (0x73ec50 → `FUN_00846210` on the queue G+0x2ef64): post message 7, the traffic restart
    /// (not asked when the traffic density world+0xa2b74 is 0).
    RestartTraffic,
    /// `GETSPEEDWALL` enter (0x73c790 → `FUN_006f1dc0` on the speed-wall leaderboard GMM+0x20): when `event` is not
    /// the one it holds (+0x1968), reset it and ask for that event's times (its vtable +0x14, +0x10).
    GetSpeedwallTimes { event: u32 },
    /// `PREFETCHSCRE` enter (`FUN_007f0e10` on the widget screen manager's prefetch list, G+0x2f458 +0x1940): count
    /// the screen `asset` up, loading it (`FUN_00623cb0`) when it is new; `Query::ScreenPrefetched` answers.
    PrefetchScreen { asset: i64 },
    /// `PREFETCHSCRE` exit (`FUN_007f0e70`): count it down; at 0 it is freed (`FUN_00622170`) and dropped.
    ReleasePrefetchedScreen { asset: i64 },
    /// `SETSIMPAUSES` / `ALLOWSIMPAUS` (offline only): pause (`FUN_00845c60`, message (0, 0, 2) through
    /// `FUN_0084db50`) or resume (`FUN_00845ce0`, (1, 0, 2) through `FUN_0084dbd0`) the simulation, on G+0x2f065
    /// with the queue flag `*0x010edb0c` | 8 held meanwhile. Reading the 2 as the pause reason is inferred.
    SimPause { paused: bool },
    /// `HSB_RUNNING` (0x73f030 / 0x73f090): show the world (G+0x2ef70 +0x44 = 1, byte 0 posted with
    /// `FUN_004b8b20` on `*0x010edb4c`) or hide it (+0x44 = 0, byte 1). Its meaning is read from the states that
    /// use it ("Show World Freedrive", "Show World States").
    ShowWorld { show: bool },
    /// `HSB_RUNNING`: the global `DAT_01179de0` += `delta` (`Query::ShowWorldCount` reads it).
    AdjustShowWorldCount { delta: i32 },
    /// `ENABLETRIGS` (`FUN_00846590` on G+0x2f064: message (0, 0, flag) through `FUN_0084ea50`): enable or disable
    /// the world's triggers.
    EnableTriggers(bool),
    /// `HSB_DISCONGP`: block (`FUN_0083c960`) or unblock (`FUN_0083c930`) `bit` of input-block `group` in the
    /// car-control block set G+0x2f0a4 ([`InputBlocks`](super::game_logic::InputBlocks); the logic host applies it).
    SetInputBlock { group: u32, bit: u32, set: bool },
    /// `ApplyModUpgrade` (0x0068adf0 → `FUN_0068ab10`(id)): the upgrade record `GMM->FUN_006e75f0`(id) is applied
    /// to the local player's vehicle (`FUN_006830a0`, `FUN_0069fce0`, `FUN_007316d0`, `FUN_00702710`);
    /// `GarageFields::mod_flag` (GMM+0x251c) is set.
    ApplyModUpgrade { id: i32 },
    /// `ReleaseAllImages` (0x007e9c30 → `FUN_007e1340` on (G+0x2f454)+8): every image slot in state 2 (+0x20 of
    /// the 0x34-byte records of `(*(+0x4830)+0x2f430)`) becomes 3 (released).
    ReleaseAllImages,
    /// `ResetNeutalVaues` (0x007f5090): flag 8 of `*0x010edb18` is set, the message {8, 0} is queued
    /// (`FUN_0076a370`) and the flag cleared: the controllers' neutral values are re-read.
    ResetNeutralValues,
    /// `SetAvailableVehicleListEvent` (0x00679b30 → `FUN_00871330` on OM+0x5ee0): the list (count +0x300 reset to
    /// 0, at most 0x40 records of 12 bytes) of the vehicles available for the event is rebuilt.
    SetAvailableVehicleListEvent { event: i32 },
    /// `FUN_006800a0`(vehicle) (Lua `SetVehicleAndRolloutFromSaveGameByVehicleID`, 0x00680340; the hook of the same
    /// name, 0x00680680, ends in it): for vehicle > 0 the saved-game record of that vehicle (`FUN_00730ec0`, the
    /// zeroed one when none) is applied to the local player's record (`FUN_006c7950`).
    SetVehicleAndRolloutFromSaveGame { vehicle: i32 },
    /// `MUTEALLRUMBL` enter (0x73e170) / exit (0x73e1f0, `FUN_0079f5f0` on G+0x2f480 +0x325f0): the controller
    /// rumble mute flag of `reason` (`MUTEREASON` in the table at 0x010c5c10: 0 Unknown, 1 LoadingScreen, 2
    /// CarSelect, 3 Pause, 4 Autolog, 5 Settings; no match 0) set (`on`) or cleared (byte +0x260 + reason); rumble
    /// is off while any flag is set, and a rumble message {2, 0, any set} is queued on `*0x010edb18`.
    MuteRumble { reason: u8, on: bool },
    /// `PAUSEMSGS` enter (0x74e890) / exit (0x74ee10): pause (`on`) or resume the in-game message system
    /// (G+0x2f480). `queues` (the `QUEUES` list, at most 8 names) are looked up in the message queue table
    /// (+0x17e0, `FUN_007b2940`); each found queue's pause byte (+0x25) is set / cleared. `flags` (the
    /// `PAUSE_*` bits of `behaviours_mode2`) count the HUD message categories up / down by one: the counter
    /// bytes at G480 +0x124b8 / +0x124b9 (HUD), the per-slot bytes +0x121cc / +0x121cd (stride 0x7c, the
    /// +0x12458 slots) with +0x12738 / 9 (instant) and +0x13fe4 cleared, +0x12548 / 9, +0x12640 / 1, +0x126bc / d,
    /// +0x125c4 / 5; `PAUSE_SPEECH` sets G+0x2ed68 to `on`; `PAUSE_DEFER_HUD` counts +0x1245c +0x8c; byte
    /// +0x325e8 = `on`.
    PauseMessages { queues: Vec<String>, flags: u16, on: bool },
    /// `PAUSEMSGS` update (0x74edd0, with `FORCEPAUSE`): every found queue whose pause byte (+0x25) is 0 gets 1.
    ForcePauseMessageQueues { queues: Vec<String> },
    /// `PLAYSONGLIST` enter (0x757e90 → `FUN_0079f7b0` on G+0x2f480 +0x30370): start the song `list` (resumes
    /// the previous one when it is the same and nothing is current, `FUN_0079f760`); `allow_playout` is
    /// `ALLOWPLAYOUT`.
    PlaySongList { list: i64, allow_playout: bool },
    /// `PLAYSONGLIST` exit begin (0x744b00 → `FUN_007908e0`): the current list (+0x400) becomes the previous
    /// (+0x404) and is cleared, +6 = 0 (stopped).
    StopSongList,
    /// `WEAPONSOK` enter (0x753ad0) / exit begin (0x741bb0): `FUN_006bb6e0` on the object manager
    /// (G+0x2f400): byte +0x6070 = `allowed`.
    WeaponsAllowed(bool),
    /// `WEAPONSOK` exit begin, unless `Query::GmmFlag2e80` (`FUN_006c8e90` on G+0x2f400): each player's id
    /// (+0xa0) is queued on the object manager's list (+0x5ff0, count +0x5ffc, total +0x6000).
    ResetPlayerWeapons,
    /// `MINIMAPICONS` update (0x7579b0), every frame: for the 12 minimap slots (G+0x2f3d8, 0x50 B each) the
    /// icon-visible byte (+0x92) is cleared; for every present player (G+0x2f3dc, +0xa5d0): when the visibility
    /// check (0x74e0b0) passes the player's +0x2f8 is set to 1.0 (`FUN_006b9410`); the slot's state (+0x80) =
    /// 2 when +0x2f8 > 0, else 0; its flag (+0x14 + 4 slot) = 4 when the player's +0x9c is nonzero, else 0.
    UpdateMinimapIcons,
    /// `SYNCPROFILEW` finish (0x7646f2): when the flag saved on enter (G+0x2ee5c) was 0 it is set to 1.
    SetProfileSyncFlag,
    /// `HIDECHEV` enter (0x75bbf0) / exit (0x75bc20): message {0x1b, 0, `hide`} posted on the queue
    /// `*0x010edb1c` +0x10 (`FUN_0076a970`): the HUD chevron is hidden / shown.
    HideChevron { hide: bool },
    /// `PAUSEAI` enter (0x73e450) / exit (0x73e4c0): the 16 bytes G+0x2ef50..0x2ef5f = `paused`, G+0x2ef60 =
    /// `!paused`.
    PauseAi { paused: bool },
    /// `DOBODYSHOP` update (0x749220), every frame: the body-shop watcher over the 24 player slots (see
    /// `behaviours_mode2::DoBodyShop`).
    UpdateBodyShop,
    /// `ACTIVATESPOT` enter (0x73afa0) `on`: byte +0x98 of the spotting object G+0x2f434 = 1; exit (0x73afd0,
    /// `FUN_0063e2d0`) `!on`: +0x98 = 0, 0x90 bytes from +4 zeroed, float 0xfb5f94 into +0x94.
    ActivateSpotting { on: bool },
    /// `CACHESCORE` enter (0x73b2b0): the local player's (`FUN_006bf330` on G+0x2f3dc) +0x41c = `FUN_006b9df0`
    /// (online +0x3f8, else services +0x4b0c + player +0x3fc).
    CachePlayerScore,
    /// `ACCT_INVITE` exit update (0x73af10): `FUN_006bad40` on the player manager (the invite listener's vtable
    /// +0xc, then `FUN_00419240` on +0xcab8).
    AcceptInvite,
    /// `SWAPGAMEMODE` update (0x746100): `FUN_006e74f0`(GMM, `mode`, 1): GMM+0x2ce4 = mode, +0x2e7e / +0x2e80 =
    /// mode ≠ 0, the mode's record looked up into GMM+0x2d10.
    SwapGameMode { mode: i32 },
    /// `WRT_PR_SESSN` enter (0x7704b0, `FUN_0083b5f0` on the Autolog G+0x2ed74): the boolean session variable.
    WriteAutologBool { name: String, value: bool },
    /// `WRT_PR_SESSN` enter (`FUN_0083b610`): the integer session variable.
    WriteAutologInt { name: String, value: i64 },
    /// `ENDSETTINGSS` exit (0x73c5c0, `FUN_0071c1c0` on G+0x2f454): with services +0x4968 or G+0x2f3e0 +0x14e0
    /// set, services +0x4940 = 1.
    EndSettingsScreen,
    /// `RUNCHALL` enter (0x74fc30): `FUN_00789240` on the challenge object G+0x2f428 with the current event
    /// record (GMM+4 +0xf4 / +0xf8).
    StartChallenge,
    /// `RUNCHALL` exit (0x73ed30): `FUN_00785d90` on the challenge object.
    StopChallenge,
    /// `HSM.STARTPHO` enter (0x740f20, `FUN_0083e710` on the photo-mode object G+0x2f474).
    StartPhotoMode,
    /// `CONSMUNLOCKS` update (0x75bf00): the notification of the unlock `unlock` just popped
    /// ([`Query::PopPendingUnlock`](crate::hsm::behaviours::Query::PopPendingUnlock)): the unlock object (the
    /// progression's vtable +8), its texts by kind (vtable +0x2c: 6 / 3), a 0x31c-byte UI message (vtable
    /// 0xfbf9ac, `FUN_00824f90`) queued on G+0x2f480 +0x1245c (vtable +0x14) and `FUN_007983b0` on +0x32858.
    ShowUnlock { unlock: u64 },
    /// `RCDONEVNTWNR` enter (0x74fa90), host only: the winner's name (player record +0x50, a 17-character slot,
    /// `FUN_006eb640` with the clock stamp `FUN_006b8fb0`) or an empty one when `winner`'s slot is free
    /// (`FUN_006eb710`) at index (GMM+0x14)+4 − 1 of the event list, then the list is sent to the session
    /// (`FUN_006f66b0`).
    RecordOnlineEventWinner { winner: i64 },
    /// `PLAYFEEDBACK` exit (0x73e7d0, `FUN_007b3410`): every message of the feedback queue `queue` is dropped.
    ClearFeedbackQueue { queue: String },
    /// `CARJACKERACT` update (0x74dc30): the local player's byte +0x2c3 = 1 (`FUN_006b9d50`), the progression event
    /// 0x19 for the target player (`FUN_00705190` → `FUN_007038b0`(0x19, 0, 0, 1, 0, player) on G+0x2f418) and,
    /// online (`FUN_004c2040`), the telemetry record with the codes `GAME` `REVG` `CRJA` (`FUN_004c3890` /
    /// `4c2900` / `4c2930` / `4c39f0`).
    CarjackerActivated,
    /// `FUN_00673ff0` (`MODEENDCHECK`, `SETENDREASON`): GMM+0x2e78 ([`GameModeManager::won`]) = `won`; applied by the host.
    SetGameModeWon { won: bool },
    /// `FUN_006dff20` (`SETENDREASON`): GMM+0x2d08 (u16, [`GameModeManager::end_reason`]) = `reason`; applied by the host.
    SetEndReason { reason: u16 },
    /// `FUN_006f4680` (`MODEENDCHECK` online): the mode-end message (the clock stamp `FUN_0046ee10`+8, posted with
    /// `FUN_00715f40` while the message flag 8 of `*0x010edb5c` is set).
    SendModeEndMessage,
    /// `SET_LAY_SET`: (*(G+0x2f480))+0x31c08 = `set` (the UI manager's layer set).
    SetLayerSet { set: u32 },
    /// `HSMCARSWAPLO` enter (`FUN_00758ba0`): the car-swap logic initialises its 0x74d0-byte instance (jack spots,
    /// `FUN_00747aa0`) and registers the view model `JackSpots` ("HSMBehaviourCarSwapLogic").
    CarSwapEnter,
    /// `HSMCARSWAPLO` update (`FUN_00761d70`): one step of the engine-side car-swap phase machine (7 phases).
    CarSwapStep,
    /// `HSMCARSWAPLO` exit (`FUN_0075d510`): release the spots (`FUN_007592a0`), GMM+0x252f = 0, G+0x2f718 / +0x2f71c
    /// = -1 (`FUN_0069fd90`), unregister `JackSpots`, unblock input group 2 bit 7 when it was blocked.
    CarSwapExit,
    /// `CHECKPTS` enter (`FUN_00749670`): reset the checkpoint progress (index +0x50 = 0, lap +0x54 = 1).
    CheckpointsEnter,
    /// `CHECKPTS` enter update, once loaded (0x75d8d0): place the checkpoints (`FUN_00759860` per player slot).
    CheckpointsPlace,
    /// `CHECKPTS` update (0x75da20): one step (passing checkpoints, rank, split times).
    CheckpointsStep,
    /// `CHECKPTS` exit (0x75d960): remove the checkpoints (`FUN_00759fa0`, `FUN_00843030` per slot).
    CheckpointsLeave,
    /// `EVENTSTARTS` enter (0x754f30): read `CHECKENTRY`, `POIID`, `RECPOIID`, `MWPOIIDC`, `MWPOIIDUC` and
    /// `TRACKEDONLY` and build the event starts.
    EventStartsEnter { check_entry: bool },
    /// `EVENTSTARTS` update (0x755220): refresh the starts (`FUN_0074ab90`, `FUN_0074a600`, `FUN_00755160`).
    EventStartsStep,
    /// `EVENTSTARTS` exit (0x74ab80 -> `FUN_0074aaf0`): remove the starts.
    EventStartsLeave,
    /// `DOBLACKLISTS` phases 1 / 4 / 5: `FUN_00748e50` / `FUN_007428b0` / `FUN_00753fa0` (the engine's blacklist
    /// shutdown steps).
    BlacklistStep { phase: u32 },
    /// `DOBLACKLISTS` phase 3 (`FUN_00843030`(handle, 3, 0)): the rival's handle is released.
    BlacklistRelease { handle: u32 },
    /// `DOBLACKLISTS` (`FUN_006ce180`(slot id +0x98, 3)): the rival's slot is removed.
    BlacklistRemoveSlot,
    /// `DOBLACKLISTS` enter: GMM+0x2f26 ([`GameModeManager::credits_transition`]) = `on`; applied by the host.
    SetCreditsTransition { on: bool },
    /// `DOBLACKLISTS` credits tail: the telemetry event "GAME CRED FNSH" (`FUN_004c3890`, `FUN_0072e5e0`(9),
    /// `FUN_004c2900`, `FUN_004c39f0`).
    GameCreditsFinished,
    // --- The sixth twenty hooks (`hooks_select.rs`).
    /// `CancelMilestoneTracking` (0x675e50): `FUN_0071d1e0` on the profile database +0x5a3c.
    CancelMilestoneTracking,
    /// `SetEasyDriveOptionSeen` (0x676970 -> `FUN_00674d20`): the "ez_options" row of `option` (`FUN_007291b0`) or,
    /// for an id the vehicle list knows (`FUN_006ecae0`), the "ez_mods" one (`FUN_007316d0`), set to `seen`.
    SetEasyDriveOptionSeen { option: i32, seen: bool },
    /// `SetEasyDriveOptionSeenToTrue` (0x6769d0 -> `FUN_00674d90`): `online` (the online-event flag set, id
    /// non-zero): `FUN_00703c10`(id) alone; else the rows as above with seen = 1, then `FUN_007040b0`(id).
    SetEasyDriveOptionSeenToTrue { option: i32, online: bool },
    /// `SetSelectedWorld` (0x675ad0 -> `FUN_00645780` on the world list G+0x2f468).
    SetSelectedWorld { world: i32 },
    /// `SetRolloutFromVehicleIdString` (0x675ee0): `FUN_006c7950`(the local player's record, vehicle, 0).
    SetRolloutFromVehicle { vehicle: i32 },
    /// `SetHighlightedVehicleByIdString` (0x680390 -> `FUN_0067ef40`): the vehicle's record into the rollout
    /// manager +0x1cc (0x2b dwords; `FUN_00730ec0`), or cleared (`FUN_0067e990`) when the database lacks it.
    LoadHighlightedVehicle { vehicle: i32 },
    /// `ShowTargetScoreReminderMessage` (0x680440 -> `FUN_0067f7f0`): when UIM+0x7230's vtable +0x14 and +0x10
    /// answers differ, the HUD message 0x5f5ee with "score" = `remaining` (`FUN_007c1c00`, `FUN_007b96b0`).
    ShowTargetScoreReminder { remaining: i32 },
    /// `SelectPaintPackFromEasyDrive` (0x68d3a0 -> `FUN_0068d200`): the paint pack of the rollout manager's
    /// local-player record (`FUN_0068c480`); `FUN_00704e80` when its +0x34 changed.
    SelectPaintPack { pack: i32 },
    /// `UpdatePendingRollout_EasyDrive` (0x6903a0 -> `FUN_00690030`) / `_Stock` (0x68ae60 -> `FUN_0068aa40`):
    /// the pending rollout of the local player (`FUN_006830a0`(vehicle, kind, 0), `FUN_006ca4f0`); the kind is
    /// 1 or 2 from the mods owned (`FUN_0068c3d0` / `FUN_0068cc30` / `FUN_00688c90`), 0 when `stock`.
    UpdatePendingRollout { vehicle: i32, stock: bool },
    /// `SetWaypointToEventFromBinding` (0x6767d0 -> `FUN_00674c60`): `FUN_00792390`, `FUN_00796ff0`(event).
    SetWaypointToEvent { event: i32 },
    /// `UpdateProgressAwardsStrings` (0x6768c0): `FUN_0071fd20`(-1, +0x10e8) and (6, +0x12e8) on G+0x2f3f8.
    UpdateProgressAwardsStrings,
    /// `CalculateBlacklistDescription` (0x676a70 -> `FUN_006f9380` on (G+0x2f3fc)+0x20): the blacklist member of
    /// rank `rank` (= the hook's number - 1): which text the screen shows (locked by a game pack / vehicle not
    /// owned / not yet raced), the flags +0x2c48..+0x2c4a and the HUD message with "gamepack" / "VehicleName".
    CalculateBlacklistDescription { rank: i32 },
    /// `UpdateDeferredViewModelSilent` (0x7f9940): `FUN_007f72f0` on the screen manager's page (SM+0x280) with the
    /// silent flag: the load of the page's deferred view model `name` starts with kind 2 (`UpdateDeferredViewModel`: 1).
    UpdateDeferredViewModelSilent { name: String },
    /// `ClearNetworkedTimer` (0x68ae20): the message 6 into the networked-timer queue (`*0x010edb0c` +0x10, flag
    /// bit 3 set while it is posted).
    ClearNetworkedTimer,
    /// `ClearOnlineEasyDrive` (0x676d50 -> `FUN_006f44e0` on G+0x14c08): the list of online easy-drive entries emptied.
    ClearOnlineEasyDrive,
    /// `RefreshOnlineEasyDrive` (0x676d30 -> `FUN_00706ec0`): the list emptied, then refilled (`FUN_00706c90`).
    RefreshOnlineEasyDrive,
    /// `SetOnlineRaceCarFrozen` / `Unfrozen` (0x68aea0 / 0x68af10): the current race car (`FUN_006bf330` +0xa0, when
    /// not -1) gets the engine message 0x37 (`FUN_00844f70` on G+0x2ed70) with `frozen`, and the message 2 with
    /// `frozen` goes into the queue `*0x010edb18` +0x10.
    SetRaceCarFrozen { frozen: bool },
    /// `SetTargetRecommendedEvent` (0x675bb0 -> `FUN_00725130`(index, 0, 0) on (G+0x2f454)+0x5a3c) and
    /// `SetCurrentRecommendedEvent` (0x675c20: the same for the target kept in +0xa0 / +0xa4): the recommended
    /// event `index` of the "Items" list becomes the target (title, gamertag, speedwall bounty, DOM selection).
    SetRecommendedEvent { index: i32, param: i32 },
    /// `SetCarListForRecommendedEvent` (0x675c50 -> `FUN_00723300` -> `FUN_00871330`): the pack page's car list for
    /// the event id of the recommendation item.
    SetCarListForEvent { event_id: i32 },
    /// `SelectRecommendationDomByIndexString` (0x675d10, `FUN_00724ef0`) / `...ByEventIdString` (0x675d70,
    /// `FUN_00723b10` then `FUN_00724ef0`): the recommendation's DOM object `index` of the "Items" list is selected
    /// (`FUN_009eee60`), the result string state (+0xa0c) set from its award / "IsBiggerBetter".
    SelectRecommendationDom { index: i32, param: i32 },
    /// `...ByEventIdString` when the event is not among the items: `FUN_0071d270` releases the three selected DOM
    /// objects.
    ReleaseRecommendationDom,
    /// `RefreshRecommendations` (0x676810 -> `FUN_007355b0`(0, 0) on (G+0x2f454)+0x5a3c): the recommendation list
    /// is rebuilt from the "Recommends" document.
    RefreshRecommendations,
    /// `RefreshTopOfSpeedwallViewModel` (0x6768f0 -> `FUN_007357f0`(0) on (G+0x2f454)+0x6c8c).
    RefreshTopOfSpeedwall,
    /// `ResetPlaylistScoreState` (0x676ac0): `FUN_006e32a0` on (G+0x2f3fc)+0x10 clears the playlist scores (+0x1b8,
    /// 12 records of 4 dwords from +0x1c0, +0x2d0, +0x328), `FUN_006bf460` on G+0x2f3dc clears byte +0x44b of
    /// each of the 12 player records (stride 0x7f0) whose byte +0x24d is set.
    ResetPlaylistScoreState,
    /// `PopThankYou` (0x676840 -> `FUN_0063cdb0` on G+0x29280): the first queued thank-you message is released
    /// (`FUN_00729e80`) and the rest shift down by one (15 dwords each).
    PopThankYou,
    /// `StopAutologAlert` (0x676860 -> `FUN_0071d190`): `FUN_00a43150` on the alert's handle (+0x2e8) when set.
    StopAutologAlert,
    /// `OpenStoreOverlay` (0x677150 -> `FUN_00655fa0` on G+0x2f438): the platform store overlay (Origin) is opened
    /// on the page id table `0x010c52f4`[+0x14]; on success +0x10 = 1.
    OpenStoreOverlay,
    /// `CacheMwlAndLock` (0x676dc0 -> `FUN_0071caa0` on (G+0x2f454)+0x4af0): the data definition "CachedMWL" is
    /// published again as "OldCachedMWL" (`FUN_00820840`), the lock byte +0x36 set.
    CacheMwl,
    /// `COSTSTATESC` (0x74a580, every 5 s): drop the u16 ids of the list (+0x48.., count +0x108) that
    /// `FUN_0090cbd0` (G+0x53200) no longer knows.
    CostStatePrune,
    /// `COSTSTATESC` update (0x7548e0 after the prune): the scan of the event's cost records (engine work).
    CostStateScan,
    /// `DISABLETIME` enter / exit (0x73c4f0 / 0x73c500): the counter G+0x25624 is decremented (`on`) / incremented.
    DisableStatsTimer { on: bool },
    /// `DISABLEWORLD`: the world-rendering byte G+0x2efb4 = `enabled`.
    SetWorldRendering { enabled: bool },
    /// `DISABLEWORLD`: `FUN_004b8b20`(`*0x010edb4c`, `on`), the audio is disabled / enabled.
    DisableAudio { on: bool },
    /// `HOSTEZGUIDE` enter (0x74bb20): easy guide object (G+0x2f480 +0x47640): +0xac = 0, +0x58 = 1, `FUN_007997f0`.
    HostEasyGuide,
    /// `HOSTEZGUIDE` exit begin (0x73c8c0): easy guide +0x58 = 0, +0xf0 = 0.
    ReleaseEasyGuide,
    /// `CLEARUIPOSTF` update (0x73bb30): `FUN_007746a0`'s object +0x2e6a0 = 0.
    ClearUiPostFx,
    /// `SETPENDINGON` update (0x743720): G+0x2f70c = `value`.
    SetPendingDialog { value: i32 },
    /// `OVERRIDEPAGE` enter (0x76f750): `FUN_0083d6f0` on G+0x2ed74 with the context and page names.
    OverridePage { context: String, page: String },
    /// `SHOWDIALOG` update (0x740a20): `FUN_007b69b0` (→ `FUN_007c0400`) takes the dialog result that
    /// `GetDialogResult` read (`Dialogs::results`), so the next read is "none" again. `LogicHost` applies it.
    TakeDialogResult { id: u32 },
    /// `HSMPAUSEINTR`, the interrupt starts (0x74d040): the UI manager's byte +0x2d260 = 1 and the flags
    /// G+0x2f3fc +0x2ea8 |= 2.
    PauseInterruptStart,
    /// `HSMPAUSEINTR`, its first frame (0x74d040): +0x2d260 = 0, the UI counter +0x125c4 += 1 and the fade
    /// `FUN_006a0f30`(UI+0x15290, 4, 0, `.rdata` 0xf8a820).
    PauseInterruptBegin,
    /// `HSMPAUSEINTR`, the interrupt ends (0x74d040 when released, exit 0x74cfe0): flags +0x2ea8 &= ~2, the counter
    /// +0x125c4 -= 1, the fade `FUN_006a0f30`(UI+0x15290, 4, c, c) with c = `.rdata` 0xf8a820.
    PauseInterruptEnd,
    /// `SUPPRESSPREF` enter (0x740f70 → `FUN_007edab0` on G+0x2f458 +0x1940): the preference object's suppress
    /// counter +0x6d8 goes up (at 0 the entries are released: `FUN_00622170` each) and its byte +0x6dc is set.
    SuppressPrefs,
    /// `SUPPRESSPREF` exit (0x740fb0 → `FUN_007edb10`): the counter goes down; at 0 the entries are re-applied
    /// (`FUN_00623cb0` each).
    ReleasePrefs,
    /// `TAKEPHOTO` enter (0x741110 → `FUN_0083bac0` on the photo object G+0x2f474): unless the photo is already
    /// requested (+0x146) or busy (+0x145) it takes the pause-screen's capture, sets the state (+0) to 2 and
    /// +0x146 = 1.
    StartPhotoCapture,
    /// `PHOTO_INPUT` enter (0x746c60): UI message 0xb (`FUN_008438e0`, on = 1), photo object +0x140 = 1,
    /// +0x144 = 0, `FUN_0083bc30` (a no-op), then the telemetry event "GAME / EVNT / OTHP" with the player's
    /// position (`FUN_00872590`).
    PhotoInputBegin,
    /// `PHOTO_INPUT` exit (0x741a30): UI message 0xb (on = 0), +0x140 = 0, control group 0 bit 4 of G+0x2f0a4
    /// cleared (`FUN_0083c930`).
    PhotoInputEnd,
    /// `PHOTO_LISTEN` update (0x746e40): the telemetry event "GAME / EVNT / SNAP" (`FUN_00872590` on
    /// G+0x2f3e0 +0x5ac8) with the player's position, the photo mode and the event id (GMM+0x2ce0). Only when the
    /// local player's vehicle is valid (`FUN_006ba2a0`); the runtime checks that.
    PostPhotoSnapEvent { mode: i32 },
    /// `RUNPRCONTEXT` enter (0x76f960 → `FUN_0083d630` on G+0x2ed74): pushes the PageRunner context `name` with
    /// the start page `page` (the parameter `DEF_PG_NAME`).
    PushPrContext { name: String, page: String },
    /// `RUNPRCONTEXT` exit (0x76ece0): the context leaves: `suspend` (parameter `SUSPEND`) = `FUN_007e1140`,
    /// else `FUN_007e1190`.
    LeavePrContext { name: String, suspend: bool },
    /// `LIMITEDEDSP` (0x762650): starts the server request of the stage (-1: the first check, `n`: the code
    /// `n` of the four, with the message built by `FUN_007661f0`) on the request object (+0x48). The result
    /// comes back through `Query::LimitedEdRequest`.
    LimitedEdSend { stage: i32 },
    /// `FUN_006f1720` state 1 (`FUN_006f06d0`(requested event)): the second event data manager starts loading the
    /// requested event's data while the running mode ends; the runtime sets
    /// [`GameModeManager::event_data_ready`](super::game_modes::GameModeManager::event_data_ready) when it is in.
    LoadEventData { event: u32 },
    /// `CLRSELEVENT` update (0x73baa0): the map's selected event on G+0x2f480 is cleared (see the behaviour).
    ClearSelectedEvent,
    /// `HSB_NOSCORES` enter (0x73be10) / exit (0x73be30): the scoring manager's (G+0x2f3cc) counter +0x124 `delta`
    /// (`FUN_006a0c10` increments, `FUN_006a0c20` decrements); scores are not awarded while it is above 0.
    AdjustScoringDisabled { delta: i32 },
    /// `CRASHESCAPE` update (0x748150): player `player` (its +0x98 id) crashed near the local player who got away:
    /// the scoring action (`FUN_006552a0` on the local player's id, `FUN_006a8050`) and the stats (`FUN_00656a90`).
    CrashEscape { player: u32 },
    /// `HSB_TOG_LS` update (0x746650): show the loading screen with `layout` (0 = the default) or hide it.
    ToggleLoadingScreen { show: bool, layout: i64 },
    /// `HSM_SERV_OP` (`start` function of table 0xfd5208 entry `op`, +8): builds and sends the operation's
    /// request on the request object (+0x50).
    ServerOpStart { op: u32 },
    /// `HSM_SERV_OP` exit update (0x73f490, `FUN_004c2870`): acknowledges a finished request.
    ServerOpAck,
    /// `HSM_SERV_OP` exit begin (0x73f450): cancels a pending request (`FUN_004c4b40`) and posts the message
    /// (1, 0x1f8) (`FUN_008820c0`).
    ServerOpCancel,
    /// `HSM_SERV_OP` error handlers (0x73f4d0 / 0x73f620 ...): shows the error `code` (`FUN_008820c0` of
    /// `FUN_00881fa0`) and ends the operation's busy state (`FUN_00882d00`). Code 0 only ends the busy state.
    ServerOpReport { code: u32 },
    /// `HSM_SERV_OP` success handler (+0x10 of the table entry of the current state) of operation `op`.
    ServerOpHandle { op: u32 },
    /// `HSM_SERV_OP` `PCLogin` / `Login` error 0x13 (0x73f620): the terms of service must be shown (flag
    /// G+0x2f3e0 +0x2436 = 1, `FUN_00889450` / `FUN_008894a0` with the request's +0x6c / +0x70).
    ServerOpTermsRequired,
    /// `WRITESESSION` enter (0x747280, `FUN_007e94f0` on G+0x2f458 +0x280): sets the session variable.
    WriteSession { name: String, value: String },
    /// `LOADSTRSET` enter update (0x74de80): loads `UI\LANGUAGE\<language>\<id>.BNDL` (`FUN_00adcda0`).
    LoadStringSet { id: i64 },
    /// `LOADSTRSET` enter update: the loaded set becomes the current one (`FUN_0046fea0`).
    ActivateStringSet { id: i64 },
    /// `LOADSTRSET` exit update (0x7441a0): drops the set (`FUN_00472e00`) and unloads its bundle.
    UnloadStringSet { id: i64 },
    /// `SIGNIN_CNTRL` enter / exit (`FUN_00884d40` on the sign-in form G+0x2f3e0 +0x10c8): the control
    /// `context` is shown / hidden.
    SignInFormActive { context: String, active: bool },
    /// `SI_EDT_EMPT` update (`FUN_008852f0`): the edit control's "empty" flag (+0x23c).
    SignInEditEmptyFlag { context: String, empty: bool },
    /// `HSM_KEYB_ENT` enter (0x7722b0, `FUN_00885080` then `FUN_00845d60` on G+0x2f065): the on-screen keyboard
    /// for the control `context` (its text, title, description and length); `flags` 4 password, 2 e-mail,
    /// 0x20 product key, 8 gamertag.
    OpenKeyboard { context: String, flags: u8 },
    /// `HSM_KEYB_ENT` update (0x7724f0, `FUN_008850e0`): the keyboard's text goes into the control when
    /// `accepted` (result 3).
    KeyboardStore { context: String, accepted: bool },
    /// `HSM_KEYB_ENT` update (`FUN_00845e00`): closes the keyboard.
    CloseKeyboard,
    /// `SI_ADD_CNTRL` / `SI_SHOW_ERR` enter (0x7730c0 / 0x7735f0, `FUN_00adcda0`): loads the string sets the
    /// state's lists name.
    SignInFormLoadStrings { state: u32 },
    /// ... exit begin (0x771030 / 0x772600): unloads them.
    SignInFormUnloadStrings { state: u32 },
    /// `SI_ADD_CNTRL` running (0x773870): the form `screen` (`FUN_00883750`) gets the controls of the state's
    /// `BACKGROUND`, `BOOLEAN`, `COMMAND`, `DATE`, `EDITTEXT`, `LIST` and `LONGMESSAGE` parameters.
    SignInFormBuild { state: u32, screen: String },
    /// `SI_ADD_CNTRL` exit (0x7729c0): the form's controls of the six kinds and the screen are removed
    /// (`FUN_00887960`).
    SignInFormRemove { screen: String },
    /// `SI_SHOW_ERR` running (0x773f40): builds the error screen from the error `SignInState.Error` picks.
    SignInErrorShow { state: u32 },
    /// `SI_SHOW_ERR` update (0x773a10): the per-frame layer set and button focus handling.
    SignInErrorFrame { state: u32 },
    /// `SI_SHOW_ERR` exit (0x772720): restores the layer set, removes the buttons and the screen
    /// (`FUN_00887960`, with `show_load` = `SHOWLOAD`, default true).
    SignInErrorClose { show_load: bool },
    /// `LoadLastOnlineRollout` (0x693c50 -> `FUN_00692800` on G+0x2f404): the last online car / rollout from the profile.
    LoadLastOnlineRollout,
    /// `UploadLocalPlayerXP` (0x676370 -> `FUN_00734f60` on G+0x2f3f8): the stat upload of `xp` (type 4) for the
    /// local player.
    UploadLocalPlayerXp { xp: i32 },
    /// `ProcessOnlinePostEventDrivingTests` (0x680560 -> `FUN_00704bf0` on G+0x2f418): the online post-event
    /// ranking messages (UI messages 6/7, 6/8).
    ProcessOnlineDrivingTests,
    /// `SetupPlaylistsVote` (0x676430 -> `FUN_006f60f0` on the playlist manager (G+0x2f3fc)+0x14): a new vote set
    /// up and kept at G+0x2a9d8.
    SetupPlaylistsVote,
    /// `PlayCriterionPlaylist` (0x676450 -> `FUN_006f6040`): the playlist the criteria pick starts.
    PlayCriterionPlaylist,
    /// `PlayCustomPlaylist` / `EZPlayCustomPlaylist` (0x676480 / 0x676520 -> `FUN_006f2730` = `FUN_006eae00`): the
    /// custom playlist `index` (0-based) is created as the current one.
    PlayCustomPlaylist { index: i32 },
    /// `HostCancelPlaylist` / `SoloPlayerCancelPlaylist` (0x6764c0 / 0x6764f0 -> `FUN_006f5f00`(solo)): the
    /// playlist-cancel event is sent.
    CancelPlaylist { solo: bool },
    /// `MatchmakePlaygroup` (0x6765c0 -> `FUN_006747d0`): the matchmaking session request (`FUN_0083db60`).
    MatchmakePlaygroup,
    /// `SetRolloutSweetRevengeFromEasyDrive` (0x68d330): after the record (+0x82b8) was stored,
    /// `FUN_0068ba20`(0) applies the local player's rollout record.
    ApplyLocalRollout,
    /// `DWNLDPROG` enter (0x73ccd0): the transient states (player objects G+0x2f3e4 +0x44 / G+0x2f418 +0xbd8 /
    /// G+0x2f420 +0x1a8 / scoring +0xe90 left from 5 / 7 / 8 / 3 to 0, the mode pause released) are reset and the
    /// progression download is started (`FUN_0071dfe0` on G+0x2f454 +0x4af0).
    DownloadProgressionBegin,
    /// `DWNLDPROG` state 1 -> 2 (0x74c164): apply the downloaded progression (`FUN_00679250` on G+0x2f3e4).
    DownloadProgressionApply,
    /// `DWNLDPROG` state 2 (0x74bfdf): `FUN_00686e70` on G+0x2f3e4, then the scratch buffer G+0x2f420 +0x200 is
    /// freed (`FUN_00cc4780`, +0x200 = 0) and +0x1f8 = 0.
    DownloadProgressionFinish,
    /// `RFRSHALMANIF` enter (0x76ea10 -> `FUN_007dae50` on G+0x2ed74): refresh the Autolog server manifest.
    AutologRefreshManifest,
    /// `RUNSTOREENUM` enter (0x73f120 -> `FUN_0086f710` on G+0x2f3e0 +0x4ce0): start the store enumeration.
    StoreEnumerate,
    /// `PAUSEAUTOLOG` enter / exit: poller 0 (G+0x2ed74: `FUN_007daee0` / `FUN_007dcf70`) or 1 (G+0x2ee60:
    /// `FUN_007e9350` / `FUN_007f0b30`) is paused / resumed.
    AutologPause { poller: u32, paused: bool },
    /// `SHOWPENDINGO` enter (0x740be0): G+0x2f70c = 0 (the pending error dialog id is taken).
    ClearPendingDialog,
    /// `TAKEPPIC` exit (0x7411c0, `FUN_00a4d3f0` on G+0xe8): remove the registry entry `name` of `owner`.
    UnregisterHandler { owner: String, name: String },
    /// `REFRESHENT` update (0x73ec20, `FUN_0063a4a0` on G+0x29280): the entitlement manager's byte +0xd1c = the
    /// network layer is not up (`FUN_004c2050`) or +0xcec != 0.
    RefreshEntitlements,
    /// `SET_GAME_ERR` enter (0x75cdf0, `FUN_0076abd0` with {9, 0, value}) and `GAMEERROR` exit (0x75ce50,
    /// `FUN_0076ac50` with {0xa, 0}): a message on the queue `*0x010edb0c` +0x10 (flag 8 of its owner set while
    /// posting).
    PostGameMessage { id: u32, value: Option<i32> },
    /// `GAMEERROR` enter (0x740ab0): show the generic error message (`FUN_007b3410` on `FUN_007746a0` +
    /// 0x11190).
    ShowGameError,
    /// `CHECKPTS`'s outcomes for the engine (`natives::checkpoints`).
    Checkpoint(super::checkpoints::CheckpointOut),
    /// `EVENTSTARTS` enter (0x754f30): the parameters, applied by the logic (`natives::event_starts::enter`).
    EventStartParams(super::event_starts::EventStartParams),
    /// `EVENTSTARTS`'s outcomes for the engine (`natives::event_starts`).
    EventStart(super::event_starts::EventStartOut),
    /// `HSMCARSWAPLO`'s outcomes for the engine (`natives::car_swap`).
    CarSwap(super::car_swap::CarSwapOut),
    /// `GetMostWantedListFromServer` (0x7eb4b0, `FUN_0071dfe0` on G+0x2f454 +0x4af0): request "GetMostWantedList"
    /// from the notification centre when no request is pending (+0x30 < 0) and the retry timer (+0x3c) ran out.
    GetMostWantedList,
    /// `ProcessPlaylistEndMilestones` (0x676da0, `FUN_00704b30` on G+0x2f418): ranking messages 0x1c / 0, and
    /// 0x1c / 0xf when the local player is the last of the vote order, 0x12 / 0 when `FUN_006eb940` holds.
    ProcessPlaylistEndMilestones,
    /// `RestartGamesComShowFloorFlow` (0x680630, `FUN_0067ff80`): the session object's +0x230 / +0x234 / +0x238
    /// reset (0 / -1 / 0), then `FUN_006ce6f0` releases the player objects.
    RestartShowFloorFlow,
    /// `MutePlayer` (0x7eea30, `FUN_007ecdc0`): bit `index` of the network layer's mute mask (+0x80ec).
    MutePlayer { index: u32 },
    /// `UnmutePlayer` (0x7eea70, `FUN_007ece70`): bit `index` of the network layer's unmute mask (+0x80f0).
    UnmutePlayer { index: u32 },
    /// A message of the network layer queued on request object `slot` (0: the owner's +0x48, 1: +0x84; for
    /// `UPLOADRESULT` / `UPLDCONCOURS` the object at +0x1f0 / +0x88): `FUN_004c59d0`. See
    /// [`NetMessage`](crate::hsm::behaviours::online::NetMessage); the ids it needs from game state (join
    /// targets, invite) are the engine's to fill in.
    NetSend { slot: u8, message: crate::hsm::behaviours::online::NetMessage },
    /// A request object is reset / cleared (`FUN_004c27e0` / `FUN_004c2840`).
    NetReset { slot: u8 },
    /// A finished request object is acknowledged: idle again (`FUN_004c2870`).
    NetAck { slot: u8 },
    /// A request object is pumped (`FUN_004c4b30`: the network layer's callbacks run, its state advances).
    NetPump { slot: u8 },
    /// A pending request object is cancelled (`FUN_004c4b40`).
    NetCancel { slot: u8 },
    /// `UPLOADRESULT` exit update (0x741600 -> `FUN_00674520`): the local player's online XP above 0 is uploaded
    /// as a stat (type 4) and cleared, as the hook `UploadLocalPlayerXP` does.
    UploadResultXp,
    /// `UPLDCONCOURS` step 0 done (0x741490): the local player's +0x3fc (the pending concours value) = 0.
    ClearConcoursPending,
    /// `UPLDCONCOURS` step 1 done (0x753440): `FUN_006fdb70` / `FUN_006fdd80` on G+0x2f418, `FUN_006e4d40` on
    /// G+0x2f420, `FUN_00679250` on G+0x2f3e4, and for an online event `FUN_00683370` on G+0x2f404 with the
    /// current rollout id.
    ApplyConcoursResult,
    /// The UI's message layer is restored (`FUN_007b3410` on G+0x2f480 +0x8c90): the message texts cleared,
    /// the saved focus group put back, the shown groups re-applied.
    RestoreUiMessages,
    /// `FUN_0063e380` on G+0x2f4c8: the wished game object's +0x230 = the local player's id value (`local`, from
    /// the rollout slot +0xcc, else 0), +0x234 = -1, +0x238 = `flag`.
    SetGameWish { local: bool, flag: bool },
    /// `HSB_NET_CRE` (0x748450): the create-game wish G+0x2f708 = 0.
    ClearCreateGameDesired,
    /// `HSB_NET_QJO` running / `HSB_NET_JOI` exit begin: the quick-join wish G+0x2f490 = 0.
    ClearQuickJoinDesired,
    /// `HSB_NET_QJO` update: the session's `NumMatchedPlayers` / `MinAcceptedPlayerCount` /
    /// `MaxAcceptedPlayerCount` are written into the state's data definition (members `NumPlayers` /
    /// `MinAccepted` / `MaxAccepted`).
    PublishQuickJoinInfo { players: i32, min: i32, max: i32 },
    /// `HSB_NET_JOI` step 0 done: G+0x2f4c0 / +0x2f4b0 / +0x2f4b4 / +0x2f4ac = 0.
    ClearJoinWish,
    /// `INVITEFAILED` enter / `HSB_NET_JOI` exit begin (`FUN_006bad40`): vtable +0xc of the invite object
    /// G+0xc9a0, then `FUN_00419240` on G+0xcab8.
    ClearInvite,
    /// `JOINPARTYUID` step 0 done: G+0x2f4a0 / +0x2f4a4 / +0x2f4a8 / +0x2f4b8 / +0x2f4bc = 0.
    ClearPartyWish,
    /// `JOINPARTYUID` step 1 done: `FUN_00ae1b20`(0).
    RefreshPartyUi,
    /// `RECONNECT AL` enter (0x76e9d0, `FUN_0083b660` on G+0x2ed74): the Autolog server connection is renewed.
    AutologReconnect,
    /// `SEND_INVITE` enter (0x73f2d0): G+0x2f498 = 0 (the id was taken).
    ClearFriendInviteId,
    /// `SEND_INVITE` step 0 (0x73f320, `FUN_00418b60` on the invite object G+0xc9a0): invite the friend `id`.
    SendInvite { id: u64 },
    /// `DEL_PR_SESS` enter (0x76f680, `FUN_0083b630` on G+0x2ed74): the entry `name` of the session store is removed.
    DeleteSessionEntry { name: String },
    /// `SETFAILLOGIN` / `SETSUCCESSLO`: the sign-in state object G+0x2f3e0's +0x10cc (fail) / +0x10c8 (success;
    /// +0x10cc = 0 when `state` is 0) = `state`.
    SetLoginState { success: bool, state: i32 },
    /// `BROWSER_IN`: the pending browser command's arguments are parsed (`FUN_0076f460`, command number as
    /// [`BROWSER_PLAY_IT`](crate::hsm::behaviours::online::BROWSER_PLAY_IT)..) and stored in the join /
    /// invite wishes (`Join?` G+0x2f4ac / +0x2f4b0 / +0x2f4b8 / +0x2f4c0, `Invite?` G+0x2f498).
    BrowserParse { command: u64 },
    /// `BROWSER_IN` `PlayIt?`: the pending flag G+0x2f454 +0x4627 = 0.
    BrowserCommandDone,
    /// `ENUM_ENTMNTS` enter update (0x73c600): the entitlement list's request byte G+0x29d60 = 1.
    EnumerateEntitlements,
    /// `HOSTSETGAMEM` enter (0x73c8f0, `FUN_006f4680`): the session host sets the game mode (a message with the
    /// mode flags 6 on the playlist manager's queue).
    HostSetGameMode,
    /// `STOPMOVIE` enter (0x740f60, `FUN_0079f1c0` on `FUN_007746a0` +0x2deb0): the movie player stops.
    StopMovie,
    /// `SYNCGAMESTAT` (`FUN_006f4890` on G+0x2f3fc): for the host the game start time (+0x2d18) and the synced
    /// byte (+0x2e76) are set, else the sync message is sent.
    SyncGameState,
    /// `UPLDCHALSTE` update (0x764790, `FUN_00762df0`(`list`)): the challenge state of list 0 / 1 is uploaded.
    UploadChallengeState { list: u8 },
    /// `UPLDCHALSTE` exit (0x741470, `FUN_006fdb70` on G+0x2f418).
    ApplyChallengeResult,
    /// `ONLNSCRACT` update (0x7436c0, `FUN_006a3360` on G+0x2f3cc): the online scoring actions advance.
    OnlineScoreActionsUpdate,
    /// `REBOOT` / `SYNCTIME` updates: the message `id` is posted with `function` (0x76aa50 / 0x69a650) on the
    /// game message queue `*0x010edb0c` +0x10 (flag 8 of the owner set while posting).
    PostQueueMessage { function: u32, id: u32 },
    /// `StoreTotalSP` (0x7ea700 -> `FUN_0072a840`): the store's speed point total.
    StoreTotalSp { sp: i32 },
    /// `MarkStoreItemAsSeen` (0x7ea740 -> `FUN_00873a30`).
    MarkStoreItemAsSeen { item: i32 },
    /// `UpdateCachedMostWantedList` (0x7e9a50 -> `FUN_0071dfe0`; its string argument is read and dropped).
    UpdateCachedMostWantedList,
    /// `GrantCloudCompeteAward` (0x7ea1c0): `FUN_00736ad0`(`FUN_009ed180`(name), amount).
    GrantCloudCompeteAward { name: String, amount: i32 },
    /// `GrantAwardForSpeedwallMovement` (0x6801e0): `FUN_00736a70` on the object reached through the online
    /// manager's (`*0x010edac8`+4) +0x6e6ec chain (when non-null).
    GrantAwardForSpeedwallMovement,
    /// `TryToGiveAwardForBeingFirstInSpeedwall` (0x680250): the same object, `FUN_00736a70` on G+0x2f3f8.
    TryToGiveAwardFirstInSpeedwall,
    /// `LoadOnlineSeenUnlocks` (0x67a5b0 -> `FUN_00703bf0` on G+0x14c08).
    LoadOnlineSeenUnlocks { arg: i32 },
    /// `SaveOnlineSeenUnlocks` (0x67a5f0 -> `FUN_006e2400` on G+0x14c08).
    SaveOnlineSeenUnlocks { arg: i32 },
    /// `RemoveOnlineSeenUnlocksMenu` (0x67a630 -> `FUN_006f44e0` on G+0x14c08).
    RemoveOnlineSeenUnlocksMenu,
    /// `SetOnlineUnlockToSeen` (0x67a660 -> `FUN_00703ca0`(unlock, value) on G+0x14c08).
    SetOnlineUnlockToSeen { unlock: i32, value: i32 },
}
