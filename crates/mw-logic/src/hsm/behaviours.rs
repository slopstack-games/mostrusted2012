//! The ported behaviour types. Each names its factory and vtable in the original (the factory allocates the
//! instance and sets the vtable; docs/decomp.md "The HSM interpreter" lists the slots). Binding reads go through
//! [`super::Host`]: a binding that does not resolve counts as false / empty, like the original's evaluators.

use mw_formats::hsm::key;

use super::{Behaviour, Cx, EXIT, EXITINVALID, Key, Registry};

#[path = "behaviours_event.rs"]
pub mod event;
#[path = "behaviours_game.rs"]
pub mod game;
#[path = "behaviours_mode.rs"]
pub mod mode;
#[path = "behaviours_mode2.rs"]
pub mod mode2;
#[path = "behaviours_ui.rs"]
pub mod ui;

/// `TRUE` (`DAT_0117aac0`).
pub const TRUE: Key = 0xbec0_6eaf_9900_0000;
/// `FALSE` (`DAT_0117a960`).
pub const FALSE: Key = 0x6ac1_7617_bea0_0000;

/// The `BINDING` parameter id of most binding behaviours: "BINDING", a NUL, then "PATH" (0x54a48941b5db2e54,
/// read from the exe's constants `DAT_0117a570` / `0117a150` / `0117a218` / `0117a288` / `0117a050`; the data's
/// 484 `BINDING` parameters carry it). `BINDINGTIMED` and `RUNSUBNET` use the plain `BINDING`
/// (`DAT_0117a778`, `DAT_0117abb8`). Parameter lookups compare all 64 bits (`FUN_00774890`).
pub const BINDING_PATH: Key = 0x54a4_8941_b5db_2e54;

/// Registers every ported behaviour.
pub fn register(r: &mut Registry) {
    r.register_fn("HSB_RUNCHILD", |_| RunChild);
    r.register_fn("HSB_PLCHLDR", |_| Placeholder);
    r.register_fn("WAITCHILDREN", |_| WaitChildren);
    r.register_fn("CALLHSMHOOK", |_| CallHook::default());
    r.register_fn("CALLHSMHOOKS", |_| CallHookScoped);
    r.register_fn("HSMHOOKEXIT", |_| HookExit::default());
    r.register_fn("EXITBINDTRUE", |_| ExitWhenBindingTrue::default());
    r.register_fn("HSB_SEL_EX_B", |_| SelectExitFromBinding);
    r.register_fn("HSM_B_TRUE", |_| IsBindingTrue);
    r.register_fn("HSB_B_NOT_0", |_| IsBindingNotNull);
    r.register_fn("HSB_EX_BI_CH", |_| ExitWhenBindingChanges::default());
    r.register_fn("BINDINGTIMED", |_| BindingTrueForTime::default());
    r.register_fn("BDELAY", |_| Delay::default());
    r.register_fn("WAIT4TIME", |_| WaitForTime::default());
    r.register_fn("RUNSUBNET", |_| RunSubNetwork::default());
    r.register_fn("RUNHUDSUBNET", |_| RunSubNetwork::default());
    r.register_fn("LOCALECHECK", |_| LocaleCheck);
    r.register_fn("BWAIT4GFX", |_| WaitForGraphics);
    r.register_fn("HSB_WAIT4LS", |_| WaitForLoadingScreen { showing: true });
    r.register_fn("HSB_WAIT4NL", |_| WaitForLoadingScreen { showing: false });
    ui::register(r);
    game::register(r);
    mode::register(r);
    mode2::register(r);
    event::register(r);
}

/// Engine state a behaviour reads, asked through [`Host::query`](super::Host::query). Answers are `u64`: bools are
/// 0 / 1, exit points their packed key; each variant names where the original reads it and its
/// [`default_answer`](Self::default_answer) (what an engine that does not answer gives: the value the original
/// has on the boot path, or the offline one).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Query {
    /// The PageRunner2 context `name` is on the context stack (`FUN_0083b8b0` → `FUN_007dc950`). Default true
    /// (the push is synchronous in the original).
    PageContextExists { name: String },
    /// Takes the exit point the context's pages asked for (context +0x1f8, `FUN_0083b8c0` has one, `FUN_0083b8d0`
    /// takes and clears it); 0 = none. Default 0.
    PageContextExit { name: String },
    /// The context is still busy leaving (`FUN_0083b8e0` → `FUN_007dc9c0`: its byte +1, on the stack or the
    /// suspended one). Default false.
    PageContextBusy { name: String },
    /// The services' state G+0x2f454 +0x38f8 is 2 (one entry of an array at +0x38ec that `FUN_006e24e0` resets
    /// to −1; 2 in both live snapshots; meaning open: `IGNORE_KB` bypasses it, so likely the on-screen
    /// keyboard). Default true.
    KeyboardIdle,
    /// The UI manager's focus group (`*`(G+0x2f480) +0x31c04), saved by `RUN_PG_RNNR2` with `RESTOREGRP`.
    /// Default 0.
    FocusGroup,
    /// The widget screen manager has a screen (G+0x2f458 +8 ≠ 0). Default true.
    ScreenActive,
    /// The current page's exit point asked by a widget (page +0x208, e.g. by the `ExitWidgetGroup` natives); 0 =
    /// none. Default 0.
    WidgetExit,
    /// The current page can activate a group (`FUN_007f2f30`: page state 2 and its pending load done). Default
    /// true.
    PageReady,
    /// [`PlayWidgetGroupTransition`](crate::natives::Request::PlayWidgetGroupTransition) of `group` started a
    /// transition (the original's return value of `FUN_007f78a0` / `FUN_007f58e0`). Default false.
    WidgetTransitionStarted { group: i64 },
    /// The screen transition finished (G+0x2f458 +0x4b0 +0x3c4). Default true.
    TransitionFinished,
    /// The screen transitions' state (G+0x2f458 +0x4b0 +0x3ac; `ACTI_WDGT_GR` counts frames in state 2, at most
    /// 10 are waited). Default 2.
    TransitionState,
    /// The page's out animation runs after [`DeactivateWidgetGroup`](crate::natives::Request::DeactivateWidgetGroup)
    /// (page +0x68 × `.rdata` 0x00f9f608 > 0). Default false.
    WidgetGroupOutTime,
    /// The page's busy count (+0x4c, u16) is nonzero. Default false.
    PageBusy,
    /// The save system is ready to load (G+0x2f454 +0x4934). Default true.
    ProfileReady,
    /// The profile load ended with data (G+0x2f454: (+0x4941 clear or +0x4935 set), +0x4942 clear, +0x4964 ≠ 0).
    /// Default true.
    ProfileLoaded,
    /// The primary user is bound (the user record `*`(0x010edad8)+4 +0xb28c ≠ 1). Default false.
    PrimaryUserBound,
    /// The local player exists (players G+0x2f3dc: +0xa6b4 ≠ −1 and its slot +0xa5d0 ≠ 0). Default true.
    LocalPlayerReady,
    /// The 60 Hz mode flag G+0x2f723. Default false.
    FullRate,
    /// In an online game (`FUN_004c1ec0`: the network object +0x6a20 ≠ 0). Default false (offline).
    OnlineGame,
    /// Online and the session lost (GAMEMODE: GMM+4 +0xe8 set and not `FUN_004c1ec0`). Default false.
    SessionLost,
    /// The event to run next (G+0x2f6f8). Default 0.
    NextEvent,
    /// The game-mode manager has started a mode (`FUN_006dfe00`: GMM+0x2ce0 ≠ 0 and the mode's state +0xdc =
    /// 4). Default true.
    GameModeStarted,
    /// The running event (GMM+0x2ce0); 0 = none. Default: [`Self::default_answer`] gives `u64::MAX` = "the
    /// event `RUNGAMEMODE` started is still running".
    GameModeEvent,
    /// The HSM machine serial of the running mode (GMM+0x2cfc). Default `u64::MAX` (none).
    GameModeMachine,
    /// The sequence `playback` still plays (`FUN_00a422d0` / `FUN_00a42390`). Default false (as for a sequence
    /// that is not loaded).
    SequencePlaying { playback: u32 },
    /// The sequence player `player` (2–5) still plays something (its count at +0x58). Default false.
    SequencePlayerBusy { player: u8 },
    /// `BINGAME` still leaving the game: two or more players (players +0xa6a0), or one of the 16 slots of
    /// `FUN_00573390` busy (`FUN_0083bd40`). Default false.
    GameLeaving,
    /// `DBAUTOSELWD`: a world can be selected without asking: the GeneSys config's debug world (`DAT_010edac0`
    /// +0x1c → +0x10 ≠ 0) or the world list's only entry (G+0x2f468 +0x80 count 1, 0x7702e0). Default true.
    AutoSelectableWorld,
    /// `SELECT_PROF`: the platform user's persona id (`FUN_004129f0` is true once it is known, `FUN_00412a50`
    /// reads it from `DAT_010eb010`); 0 = not yet. Default 1 (a model decision: offline there is no persona).
    UserId,
    /// The feedback group `group` is loaded (`FUN_00792590`: one of the 7 slots of the UI manager +0x32858 has
    /// the id and state 3). Default true.
    FeedbackGroupLoaded { group: i64 },
    /// The vehicle list is loaded (`FUN_0083bd20`: the vehicle resource +0x2108 > 0). Default true.
    VehicleListLoaded,
    /// The profile is brand new (G+0x2f454 +0x4964 +5, `FUN_0069fce0`, 0x73d350). Default true (no save).
    NewProfile,
    /// `REBOOT_REQ`: a system reboot was requested (`FUN_0046ef70`: the byte at +0xc48 of the object behind
    /// `*0x010edacc`+4, the same object as the HSM clocks). Default false.
    RebootRequested,
    /// `COLLECTABLES`: the world state is loaded (`FUN_00532dc0` on `*0x010edaf0`: +0xc10 == 3). Default true.
    CollectablesReady,
    /// `PRELOADCOPS`: a vehicle bundle load or release it asked for is still in progress (its two resource
    /// requests +0x48 (`FUN_00ad7610`, loads) / +0x70 (`FUN_00ad7a80`, releases) in state 2). Default false.
    VehicleStreamBusy,
    /// The running event is online ((GMM+4)+0xe8, `FUN_006dfed0`; [`GameModeManager::online_event`]
    /// (crate::natives::game_modes::GameModeManager::online_event)). Default false.
    OnlineEvent,
    /// The local player's vehicle id (its vtable +0x18 → vehicle +0x30, `FUN_006ba320`). Default 0.
    PlayerVehicle,
    /// GMM+0x2e79: set by `FUN_00708120` when the event's data allows it (event +0xe8) and the player's state
    /// (`FUN_006c2d80`) does not forbid it; the pursuit code reads it (0x513e5e, 0x521378): cops take part.
    /// Default false.
    CopsAllowed,
    /// `PRELOADCOPS` with [`Self::CopsAllowed`]: the +0x70 field of what `FUN_006bdc00` returns for the local
    /// player (`FUN_0066aa30` on the player's +0x3d8, the heat block; the record's meaning is open); a change
    /// re-lists the cop vehicles. Default 0.
    HeatRecordKey,
    /// The number of players (PM+0xa694). Default 1 (offline: the local player).
    PlayerCount,
    /// The vehicle id of player `index` (PM+0xa664[index], its vtable +0x18 → vehicle +0x30). Default 0.
    PlayerVehicleOf { index: u32 },
    /// GMM+0x2e90 `IsFreedriveEvent` (`FUN_006e03d0`). Default true (the boot's mode is free drive).
    FreedriveEvent,
    /// `PRELOADCOPS`: the entries of the cop vehicle table (`*0x010edb50`+4 → +0xc → +0x714: u16 count +0x20,
    /// 0x18-byte entries at +0x1c). Default 0.
    CopVehicleCount,
    /// ... entry `index`'s vehicle: entry +0xc looked up in the GeneSys list G+0x14c08 (`FUN_006ec9d0`), its +0xc
    /// in the event data (`FUN_006e6140`), that +0x14 in the rollout table G+0x2f404 (`FUN_00683420`), the
    /// rollout's vehicle (+0x30). Default 0.
    CopVehicle { index: u32 },
    /// `HSB_RNLAYOUT`: the UI manager's layout slot for `layout` is ready (`FUN_00791bd0` → slot vtable +0x14).
    /// Default true.
    LayoutReady { layout: i64 },
    /// `LIST_INVITE`: the invite listener (PM+0xc9a0) has an invite (its vtable +0). Default false (offline).
    InvitePending,
    /// ... the invite can be taken: not `FUN_006402e0`, the listener's vtable +4, and its id (vtable +8) passes
    /// `FUN_004c2640` and `FUN_004c2560` (online session checks). Default false.
    InviteJoinable,
    /// The traffic density world+0xa2b74 is 0 (`FUN_005ea590` on `*0x010edae8`; 1.0 in free roam). Default false.
    TrafficDensityZero,
    /// The traffic's 64-bit time stamp world+0xae560 is set (not i64::MIN; `FUN_005ea560`; `REQTRAFRSTRT` online
    /// only). Default false.
    TrafficTimeSet,
    /// `FUN_006e5ea0`(event): the event's mode is a free-drive mode (`GameLogicConfig` +0xd8 or +0x130). Default true.
    EventIsFreedrive { event: u32 },
    /// The speed-wall leaderboard (GMM+0x20) has no request pending (`FUN_0086e260`: +0x4c < 0, or
    /// `FUN_0081d1b0` of it). Default true.
    SpeedwallIdle,
    /// The prefetched screen `asset` is loaded (`FUN_007f0ef0`). Default true.
    ScreenPrefetched { asset: i64 },
    /// `DAT_01179de0`, the count of running `HSB_RUNNING` states. Default 0.
    ShowWorldCount,
    /// GMM+0x2ce4: the requested event ([`GameModeManager::requested_event`]
    /// (crate::natives::game_modes::GameModeManager::requested_event)). Default 0.
    RequestedEvent,
    /// GMM+0x2e7a: story mode. Default false.
    StoryMode,
    /// `HSB_DISCONGP`: the input-block groups of the control group `name` (the user record's table, see
    /// [`DisableControlGroup`](mode::DisableControlGroup)): their count (no record: 0). Default 0.
    ControlGroupCount { name: String },
    /// ... and group `index`. Default 0.
    ControlGroup { name: String, index: u32 },
    /// `PLAYSONGLIST` (0x74f540, first fallback): the event's own song list: when GMM+0x2ce0 is set with mode
    /// 4 (`FUN_006dfe00`) and the event is offline (`FUN_006dfed0`), byte +0x68 of the event record
    /// (`FUN_006e66a0`). Default 0 (none: the next fallback).
    CurrentEventSongList,
    /// `PLAYSONGLIST` (0x74f540, last fallback): the current event's default list (GMM+0x2cf4 record +0x2c).
    /// Default 0.
    DefaultEventSongList,
    /// `WEAPONSOK` (0x753ad0): byte +0xf2 of the event record (`FUN_006e66a0`): weapons allowed in the event.
    /// Default false (model decision: the free-drive record is not read).
    EventWeaponsAllowed,
    /// `WEAPONSOK` exit: byte GMM+0x2e80. Default false (so the weapon state is reset on exit).
    GmmFlag2e80,
    /// `SYNCPROFILEW` (0x763a00 state 0, `FUN_004c2050` → `FUN_00a82e60`): the online profile service is
    /// connected. Default false (offline).
    ProfileServiceConnected,
    /// `SYNCPROFILEW` enter (0x7462e0): byte G+0x2ee5c. Default false.
    ProfileSyncFlag,
    /// `MODEENDCHECK` (`FUN_006dfee0` on the GMM: no session, or the session's host `FUN_004c22c0`): this machine decides the mode's end. Default true (offline).
    ModeAuthority,
    /// `MODEENDCHECK` (`FUN_006f1530`): the mode's time limit (GMM+0x2d28 / 3 against +0x2d28 stamps), the offline time limit or a player's score reaching the score limit (+0x2e64). Needs the players' scores: the runtime answers. Default false (free roam has no limit).
    ModeLimitReached,
    /// `MODEENDCHECK` / `SETENDREASON` (`FUN_006f46e0`): online it is GMM+0x2e78; offline a medal / time derivation from the event data (not ported: the runtime answers). The logic host answers GMM+0x2e78 online only. Default false.
    ModeWon,
    /// `MODEENDCHECK` (`FUN_004c1f30` on `*0x010edb58`): a session exists and its state (+0xbe7c) is 4. Default false.
    SessionRaceState,
    /// `MODEENDCHECK` (`FUN_0074e790`): offline with the mode timer running, the first end trigger of the event's list (`FUN_006f4b70` per entry, type +0x1e) that fired: 0 none, 1 = type 0 (the game mode is won: `FUN_00673ff0`(1)), 2 = type 1 (lost: (0)). Needs the event data: the runtime answers. Default 0.
    EventEndTrigger,
    /// GMM+0x2d08, the end reason ([`GameModeManager::end_reason`](crate::natives::game_modes::GameModeManager::end_reason)). Default 0.
    EndReason,
    /// GMM+0x2f10 ([`GameModeManager::display_timeout`](crate::natives::game_modes::GameModeManager::display_timeout)). Default false.
    DisplayTimeoutOn,
    /// `MODEENDCHECK` (`FUN_006e27e0`): the ticks left until the display deadline (GMM+0x2f18 as i64 minus the clock `FUN_0046ee10`+8), 0 when it has none or passed; as an i64 in the u64. Not ported: the runtime answers. Default 0 (= timed out).
    DisplayTimeLeft,
    /// GMM+0x2e75 (`MODEENDCHECK` online: the end was announced; written by the online code, not ported). Default false.
    ModeEndFlag,
    /// `HSB_WAIT4IN` (`FUN_007470e0`'s test on the local player's input record `FUN_0047aa20`): an input action (the 12 action blocks at +0x214..+0x268, +0x646..+0x64d, the 5 menu-state words) is still held, so the wait is not released yet. Default false.
    InputHeld,
    /// `HSB_WAIT4IN` (0x753540): the `index`th candidate exit point (packed; 0 = no more) in the original's order: the 0x28c / 0x10c / 0x298 menu-state words == 2 (the first and third also play a cue, `FUN_0079fb40`), then the actions 0x73a020 / 0x73a070 / 0x73a110 / 0x73a0c0 (given `autorepeat`, the `AUTOREPEAT` parameter), then the words 0x304 / 0x2b0 (== 2), 0x358 / 0x364 / 0x370 / 0x37c (== 3), 0x208 (== 2), and last any device whose record +0 is 0 (`FUN_0047aa40`). The behaviour takes the first the parent handles. Default 0.
    InputExit { index: u32, autorepeat: bool },
    /// `SET_LAY_SET`: the UI manager's layer set, (*(G+0x2f480))+0x31c08. Default 0.
    LayerSet,
    /// `HSMCARSWAPLO` (+0x74c8): the car-swap logic has been started by the engine's phase machine. Default false.
    CarSwapActive,
    /// `CHECKPTS`: the checkpoint manager's (G+0x2f47c) count +0x510. Default 0.
    CheckpointCount,
    /// `CHECKPTS` enter update (the manager's vtable +8): the checkpoints are loaded. Default true.
    CheckpointsLoaded,
    /// `CHECKPTS` update (0x75da20): the last checkpoint index was reached and the widget manager's state G+0x2f458 +0x85c is not 3. Default true.
    CheckpointsFinished,
    /// `EVENTSTARTS` update (0x755220): not in a vehicle (`FUN_006c2410`), slower than the event start's limit (`FUN_006ba3e0`) and `FUN_0074ad50` == 1: the player stands in an event start. Default false.
    EventStartTriggered,
    /// `DOBLACKLISTS` (+0x4c): 2 or 3 holds the machine in phase 0 and in its exit. Default 0.
    BlacklistMode,
    /// `DOBLACKLISTS` (+0x84): the blacklist rival's player slot (non-zero = its record's active byte +0x23d is set; the slot's id is +0x98). Default 0.
    BlacklistSlot,
    /// `DOBLACKLISTS` (`FUN_006ba2a0`): the rival's vehicle is alive. Default false.
    BlacklistRivalAlive,
    /// `DOBLACKLISTS`: squared distance between the rival and the player (`FUN_006b9ea0` positions) as f32 bits; `FUN_006ba2a0` false: `FLT_MAX` (`DAT_00fd1304`). Default `FLT_MAX`.
    BlacklistRivalDistance,
    /// `DOBLACKLISTS` phase 2 without a slot (`FUN_00748a40`). Default false.
    BlacklistReady,
    /// `DOBLACKLISTS` phase 3: the rival's handle (slot +0xb0), -1 (= u32::MAX) when it has none. Default -1.
    BlacklistRivalHandle,
    /// `DOBLACKLISTS` phases 1 / 4 / 5 (`FUN_00748e50`, `FUN_007428b0`, `FUN_00753fa0`): the phase (+0x70) the step left behind: 0 = unchanged, else 1 + the phase. Default 0.
    BlacklistPhaseAfter,
    /// `DOBLACKLISTS` (+0xa4, set by the steps): the credits sequence follows the shutdown. Default false.
    BlacklistCreditsPending,
    /// `DOBLACKLISTS` (`FUN_0066e0f0`): the general sequence runs. Default false.
    CreditsSequenceActive,
}

impl Query {
    /// The answer when the host does not know (see each variant).
    pub fn default_answer(&self) -> u64 {
        match self {
            Query::PageContextExists { .. }
            | Query::KeyboardIdle
            | Query::ScreenActive
            | Query::PageReady
            | Query::ProfileReady
            | Query::ProfileLoaded
            | Query::LocalPlayerReady
            | Query::GameModeStarted
            | Query::TransitionFinished
            | Query::AutoSelectableWorld
            | Query::UserId
            | Query::FeedbackGroupLoaded { .. }
            | Query::VehicleListLoaded
            | Query::NewProfile
            | Query::CollectablesReady
            | Query::PlayerCount
            | Query::FreedriveEvent
            | Query::LayoutReady { .. }
            | Query::EventIsFreedrive { .. }
            | Query::SpeedwallIdle
            | Query::ScreenPrefetched { .. }
            | Query::ModeAuthority
            | Query::CheckpointsLoaded
            | Query::CheckpointsFinished => 1,
            Query::TransitionState => 2,
            Query::GameModeEvent | Query::GameModeMachine | Query::BlacklistRivalHandle => u64::MAX,
            Query::BlacklistRivalDistance => 0x7f7f_ffff,
            _ => 0,
        }
    }

    /// The variant's name (drivers answer by it).
    pub fn name(&self) -> &'static str {
        match self {
            Query::PageContextExists { .. } => "PageContextExists",
            Query::PageContextExit { .. } => "PageContextExit",
            Query::PageContextBusy { .. } => "PageContextBusy",
            Query::KeyboardIdle => "KeyboardIdle",
            Query::FocusGroup => "FocusGroup",
            Query::ScreenActive => "ScreenActive",
            Query::WidgetExit => "WidgetExit",
            Query::PageReady => "PageReady",
            Query::WidgetTransitionStarted { .. } => "WidgetTransitionStarted",
            Query::TransitionFinished => "TransitionFinished",
            Query::TransitionState => "TransitionState",
            Query::WidgetGroupOutTime => "WidgetGroupOutTime",
            Query::PageBusy => "PageBusy",
            Query::ProfileReady => "ProfileReady",
            Query::ProfileLoaded => "ProfileLoaded",
            Query::PrimaryUserBound => "PrimaryUserBound",
            Query::LocalPlayerReady => "LocalPlayerReady",
            Query::FullRate => "FullRate",
            Query::OnlineGame => "OnlineGame",
            Query::SessionLost => "SessionLost",
            Query::NextEvent => "NextEvent",
            Query::GameModeStarted => "GameModeStarted",
            Query::GameModeEvent => "GameModeEvent",
            Query::GameModeMachine => "GameModeMachine",
            Query::SequencePlaying { .. } => "SequencePlaying",
            Query::SequencePlayerBusy { .. } => "SequencePlayerBusy",
            Query::GameLeaving => "GameLeaving",
            Query::AutoSelectableWorld => "AutoSelectableWorld",
            Query::UserId => "UserId",
            Query::FeedbackGroupLoaded { .. } => "FeedbackGroupLoaded",
            Query::VehicleListLoaded => "VehicleListLoaded",
            Query::NewProfile => "NewProfile",
            Query::RebootRequested => "RebootRequested",
            Query::CollectablesReady => "CollectablesReady",
            Query::VehicleStreamBusy => "VehicleStreamBusy",
            Query::OnlineEvent => "OnlineEvent",
            Query::PlayerVehicle => "PlayerVehicle",
            Query::CopsAllowed => "CopsAllowed",
            Query::HeatRecordKey => "HeatRecordKey",
            Query::PlayerCount => "PlayerCount",
            Query::PlayerVehicleOf { .. } => "PlayerVehicleOf",
            Query::FreedriveEvent => "FreedriveEvent",
            Query::CopVehicleCount => "CopVehicleCount",
            Query::CopVehicle { .. } => "CopVehicle",
            Query::LayoutReady { .. } => "LayoutReady",
            Query::InvitePending => "InvitePending",
            Query::InviteJoinable => "InviteJoinable",
            Query::TrafficDensityZero => "TrafficDensityZero",
            Query::TrafficTimeSet => "TrafficTimeSet",
            Query::EventIsFreedrive { .. } => "EventIsFreedrive",
            Query::SpeedwallIdle => "SpeedwallIdle",
            Query::ScreenPrefetched { .. } => "ScreenPrefetched",
            Query::ShowWorldCount => "ShowWorldCount",
            Query::RequestedEvent => "RequestedEvent",
            Query::StoryMode => "StoryMode",
            Query::ControlGroupCount { .. } => "ControlGroupCount",
            Query::ControlGroup { .. } => "ControlGroup",
            Query::CurrentEventSongList => "CurrentEventSongList",
            Query::DefaultEventSongList => "DefaultEventSongList",
            Query::EventWeaponsAllowed => "EventWeaponsAllowed",
            Query::GmmFlag2e80 => "GmmFlag2e80",
            Query::ProfileServiceConnected => "ProfileServiceConnected",
            Query::ProfileSyncFlag => "ProfileSyncFlag",
            Query::ModeAuthority => "ModeAuthority",
            Query::ModeLimitReached => "ModeLimitReached",
            Query::ModeWon => "ModeWon",
            Query::SessionRaceState => "SessionRaceState",
            Query::EventEndTrigger => "EventEndTrigger",
            Query::EndReason => "EndReason",
            Query::DisplayTimeoutOn => "DisplayTimeoutOn",
            Query::DisplayTimeLeft => "DisplayTimeLeft",
            Query::ModeEndFlag => "ModeEndFlag",
            Query::InputHeld => "InputHeld",
            Query::InputExit { .. } => "InputExit",
            Query::LayerSet => "LayerSet",
            Query::CarSwapActive => "CarSwapActive",
            Query::CheckpointCount => "CheckpointCount",
            Query::CheckpointsLoaded => "CheckpointsLoaded",
            Query::CheckpointsFinished => "CheckpointsFinished",
            Query::EventStartTriggered => "EventStartTriggered",
            Query::BlacklistMode => "BlacklistMode",
            Query::BlacklistSlot => "BlacklistSlot",
            Query::BlacklistRivalAlive => "BlacklistRivalAlive",
            Query::BlacklistRivalDistance => "BlacklistRivalDistance",
            Query::BlacklistReady => "BlacklistReady",
            Query::BlacklistRivalHandle => "BlacklistRivalHandle",
            Query::BlacklistPhaseAfter => "BlacklistPhaseAfter",
            Query::BlacklistCreditsPending => "BlacklistCreditsPending",
            Query::CreditsSequenceActive => "CreditsSequenceActive",
        }
    }
}

/// Asks the host, falling back to the query's default.
pub fn ask(cx: &mut Cx, q: Query) -> u64 {
    cx.host.query(&q).unwrap_or_else(|| q.default_answer())
}

/// [`ask`] as a bool.
pub fn ask_bool(cx: &mut Cx, q: Query) -> bool {
    ask(cx, q) != 0
}

/// A parameter read as the original's `__stricmp(value, "true") == 0` (false when missing).
pub fn param_true(cx: &Cx, name: &str) -> bool {
    cx.param_text(name).eq_ignore_ascii_case("true")
}

/// A parameter read with `atoi` (`FUN_00edb8aa`; 0 when missing or not a number).
pub fn param_int(cx: &Cx, name: &str) -> i64 {
    super::atof(&cx.param_text(name)) as i64
}

/// An integer binding (`FUN_00a50650` parse, `FUN_00a4f220` read) when `path` is not empty, else the parameter
/// `fallback` as `atoi`: the "binding or parameter" pattern of `ACTI_WDGT_GR` (0x76f250) and `RUNGAMELOGIC`
/// (0x744fe0). An unresolved binding is 0.
pub fn binding_or_param(cx: &mut Cx, path: &str, fallback: &str) -> i64 {
    if path.is_empty() { param_int(cx, fallback) } else { cx.host.binding_int(path).unwrap_or(0) }
}

/// "Run Child Behaviour" (`HSB_RUNCHILD`, factory 0x77ab20, vtable 0xfd9194): runs its children until one of
/// their exit points exits it (update 0x73ed50 returns 0).
pub struct RunChild;
impl Behaviour for RunChild {}

/// `HSB_PLCHLDR` (factory 0x77aa80, vtable 0xfd914c): exits through `EXIT` on its first update (0x73e670); the
/// fallback of states whose behaviour is not registered.
pub struct Placeholder;
impl Behaviour for Placeholder {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "HSM.Wait For Children" (`WAITCHILDREN`, factory 0x77aad0, vtable 0xfd9170): exits through `EXIT` once it
/// has no children left (0x73e690).
pub struct WaitChildren;
impl Behaviour for WaitChildren {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.has_children() {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "CallHSMHook" (`CALLHSMHOOK`, factory 0x77a990, vtable 0xfd90e0): on enter (0x76f0f0) the hook named by
/// `FUNCTION`; on its first update (0x76e420) calls it with the data definition's `Params` and exits through
/// whatever the hook returns ([`EXITINVALID`] for a missing hook).
#[derive(Default)]
pub struct CallHook {
    function: String,
}
impl Behaviour for CallHook {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.function = cx.param_text("FUNCTION");
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let r = cx.call_hook(&self.function, "Params");
        cx.set_exit(r);
        true
    }
}

/// "CallHSMHookScoped" (`CALLHSMHOOKS`, factory 0x77a9e0, vtable 0xfd9104): calls `ONENTERHOOK` with
/// `EnterParams` on enter (0x76f120) and `ONEXITHOOK` with `ExitParams` when finished (0x76f1a0), when named;
/// the results are ignored; runs its children in between.
pub struct CallHookScoped;
impl Behaviour for CallHookScoped {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("ONENTERHOOK");
        if !name.is_empty() {
            cx.call_hook(&name, "EnterParams");
        }
    }
    fn on_exit(&mut self, cx: &mut Cx) {
        let name = cx.param_text("ONEXITHOOK");
        if !name.is_empty() {
            cx.call_hook(&name, "ExitParams");
        }
    }
}

/// "Select Exit From HSM Hook" (`HSMHOOKEXIT`, factory 0x77aa30, vtable 0xfd9128): calls `FUNCTION` with
/// `Params` every update (0x76e480) until it returns something other than [`EXITINVALID`], and exits through it.
#[derive(Default)]
pub struct HookExit {
    function: String,
}
impl Behaviour for HookExit {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.function = cx.param_text("FUNCTION");
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let r = cx.call_hook(&self.function, "Params");
        if r == EXITINVALID {
            return false;
        }
        cx.set_exit(r);
        true
    }
}

/// "Exit When Binding Is True" (`EXITBINDTRUE`, factory 0x55f790, vtable 0xfab66c): parses `BINDING` on enter
/// (0x742fc0); every update (0x743000), when the binding parsed, exits through `EXIT` once it reads true. Runs
/// its children meanwhile.
#[derive(Default)]
pub struct ExitWhenBindingTrue {
    binding: String,
}
impl Behaviour for ExitWhenBindingTrue {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_id_text(BINDING_PATH);
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        // Model: "parsed" (+0xbc > 0 after FUN_00a50650) = a non-empty path, until the binding parser is ported.
        if self.binding.is_empty() || cx.host.binding_bool(&self.binding) != Some(true) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Select Exit Point From Binding" (`HSB_SEL_EX_B`, factory 0x77bc90, vtable 0xfd9860; update 0x7457d0): exits
/// at once through the exit point named by the binding's text value, or through `ONFAILEXIT` when it does not
/// resolve.
pub struct SelectExitFromBinding;
impl Behaviour for SelectExitFromBinding {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let exit = match cx.host.binding_string(&binding) {
            Some(text) => key(&text),
            None => key(&cx.param_text("ONFAILEXIT")),
        };
        cx.set_exit(exit);
        true
    }
}

/// "HSM.Is Binding True" (`HSM_B_TRUE`, factory 0x77bd30, vtable 0xfd98a8; update 0x7459b0): exits at once
/// through `TRUE` or `FALSE` (also when it does not resolve).
pub struct IsBindingTrue;
impl Behaviour for IsBindingTrue {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let v = cx.host.binding_bool(&binding) == Some(true);
        cx.set_exit(if v { TRUE } else { FALSE });
        true
    }
}

/// "Is Binding Not Null" (`HSB_B_NOT_0`, factory 0x77bce0, vtable 0xfd9884; update 0x7458a0): exits at once
/// through `TRUE` when the binding's text is neither empty nor "0", else `FALSE`.
pub struct IsBindingNotNull;
impl Behaviour for IsBindingNotNull {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_id_text(BINDING_PATH);
        let text = cx.host.binding_string(&binding).unwrap_or_default();
        cx.set_exit(if !text.is_empty() && text != "0" { TRUE } else { FALSE });
        true
    }
}

/// "Exit When Binding Changes" (`HSB_EX_BI_CH`, factory 0x77bd80, vtable 0xfd98cc): the first update that
/// resolves the binding (0x74b840 → 0x742f60) keeps its value; later updates exit through `EXIT` once a resolved
/// value differs.
#[derive(Default)]
pub struct ExitWhenBindingChanges {
    binding: String,
    first: Option<i64>,
}
impl Behaviour for ExitWhenBindingChanges {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_id_text(BINDING_PATH);
        self.first = None;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let Some(first) = self.first else {
            self.first = cx.host.binding_int(&self.binding);
            return false;
        };
        if self.binding.is_empty() {
            return false;
        }
        match cx.host.binding_int(&self.binding) {
            Some(v) if v != first => {
                cx.set_exit(EXIT);
                true
            }
            _ => false,
        }
    }
}

/// "BindingIsTrueForTime" (`BINDINGTIMED`, factory 0x779c60, vtable 0xfd8ee8; enter 0x7420d0, update 0x742140):
/// counts `TIME` seconds down on the game clock while the binding reads true (not while that clock is stopped),
/// restarts the count when it reads false, exits through `EXIT` at 0.
#[derive(Default)]
pub struct BindingTrueForTime {
    binding: String,
    time: f32,
    left: f32,
}
impl Behaviour for BindingTrueForTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.binding = cx.param_text("BINDING");
        self.time = cx.param_f32("TIME");
        self.left = self.time;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.binding.is_empty() {
            return false;
        }
        if cx.host.binding_bool(&self.binding) != Some(true) {
            self.left = self.time;
            return false;
        }
        let Some(dt) = cx.clock.game else { return false };
        self.left -= dt;
        if self.left <= 0.0 {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// `BDELAY` (factory 0x779280, vtable 0xfd8d80): waits `ENTERTIME` s while entering (0x73bd40), `UPDATETIME` s
/// while running before exiting through `EXIT` (0x73bd80), `LEAVETIME` s while exiting (0x73bdd0), each on the
/// frame clock (enter 0x742770 reads the three parameters).
#[derive(Default)]
pub struct Delay {
    times: [f32; 3],
    elapsed: [f32; 3],
}
impl Delay {
    fn wait(&mut self, i: usize, dt: f32) -> bool {
        self.elapsed[i] += dt;
        self.elapsed[i] >= self.times[i]
    }
}
impl Behaviour for Delay {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.times = [cx.param_f32("ENTERTIME"), cx.param_f32("UPDATETIME"), cx.param_f32("LEAVETIME")];
        self.elapsed = [0.0; 3];
    }
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.wait(0, cx.clock.frame)
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.wait(1, cx.clock.frame) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.wait(2, cx.clock.frame)
    }
}

/// Ticks per second of the timers (`.rdata` 0x00f9f604 = 3e6).
const TICKS: f64 = 3.0e6;

/// "Wait For Time" (`WAIT4TIME`, factory 0x5a23f0, vtable 0xfae1a4): `TIME` s as 3 MHz ticks (enter 0x758950),
/// counted down by the frame clock (update 0x741ac0) until less than one microsecond (ticks / 3 ≤ 0) is left, then
/// `EXIT`. With `REGBINDING` true (the default) the timer is also published as the view model
/// `CurrentHSMTimer` (not ported: open).
#[derive(Default)]
pub struct WaitForTime {
    left: i64,
}
impl Behaviour for WaitForTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.left = (super::atof(&cx.param_text("TIME")) * TICKS) as i64;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        self.left -= (cx.clock.frame as f64 * TICKS) as i64;
        if self.left / 3 <= 0 {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Run Sub Network" (`RUNSUBNET`, factory 0x77dd50, vtable 0xfda810): starts the top-level state whose
/// instance id is the `BINDING`'s value, or `TOPSTATE` when that is empty (0x745710), as its own machine
/// (0x7500a0), waits for it (0x73f1e0) and exits through the machine's exit point when its own parent handles
/// that, else through `EXIT`; an exit from outside first asks the machine to exit and waits for it
/// (0x73f170, 0x73f1b0).
#[derive(Default)]
pub struct RunSubNetwork {
    serial: Option<u32>,
}
impl Behaviour for RunSubNetwork {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        let binding = cx.param_text("BINDING");
        let instance = if binding.is_empty() {
            cx.param_text("TOPSTATE").trim().parse::<i64>().unwrap_or(0)
        } else {
            cx.host.binding_int(&binding).unwrap_or(0)
        };
        self.serial = cx.start_machine(instance as u32);
        true
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let Some(serial) = self.serial else {
            cx.set_exit(EXIT);
            return true;
        };
        if cx.machine_running(serial) {
            return false;
        }
        match cx.machine_result(serial) {
            Some(exit) if cx.parent_handles(exit) => cx.set_exit(exit),
            _ => cx.set_exit(EXIT),
        }
        true
    }
    fn on_exit_begin(&mut self, cx: &mut Cx) {
        if let Some(s) = self.serial
            && cx.machine_running(s)
            && !cx.machine_exit_requested(s)
        {
            cx.request_machine_exit(s);
        }
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.serial.is_some_and(|s| cx.machine_running(s)) {
            return false;
        }
        self.serial = None;
        true
    }
}

/// "Locale Check" (`LOCALECHECK`, factory 0x779ff0, vtable 0xfd8fc0): on PC exits through `EXIT` at once
/// (0x73e0a0); the console builds pick a language-select exit point instead.
pub struct LocaleCheck;
impl Behaviour for LocaleCheck {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Wait For Graphics Prepare Behaviour" (`BWAIT4GFX`, factory 0x7792e0, vtable 0xfd8da4): exits through `EXIT`
/// at once (0x73e0a0-like 0x741720) but finishes only once the graphics are prepared (exit update 0x741740).
pub struct WaitForGraphics;
impl Behaviour for WaitForGraphics {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.graphics_prepared()
    }
}

/// "Wait For Loading Screen Showing" (`HSB_WAIT4LS`, factory 0x77c1f0, vtable 0xfd9adc, update 0x741c10) and
/// "... Not Showing" (`HSB_WAIT4NL`, factory 0x77c1a0, vtable 0xfd9ab8, update 0x741c50): exit through `EXIT`
/// once the loading screen shows / no longer shows; run their children meanwhile.
pub struct WaitForLoadingScreen {
    showing: bool,
}
impl Behaviour for WaitForLoadingScreen {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.host.loading_screen_showing() != self.showing {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// A stand-in for a behaviour that is not ported: exits through a fixed exit point on its first update, or
/// runs its children forever (`None`). For headless drivers and tests; not a port of anything.
pub struct Stub {
    pub exit: Option<Key>,
}
impl Behaviour for Stub {
    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.exit {
            Some(e) => {
                cx.set_exit(e);
                true
            }
            None => false,
        }
    }
}
