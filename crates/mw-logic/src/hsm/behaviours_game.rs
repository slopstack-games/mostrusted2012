//! Behaviour types of the game flow: loading the world and the profile, running game modes and game-logic
//! sequences, online checks. What the engine does is asked for with [`Request`]s; what the behaviours wait on is
//! read with [`Query`]s (both name the original's code).

use std::sync::atomic::{AtomicU32, Ordering};

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, binding_or_param, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("LOADWORLD", |_| LoadWorld);
    r.register_fn("LOADPROFILE", |_| LoadProfile);
    r.register_fn("IN_ON_GAME", |_| InOnlineGame);
    r.register_fn("RUNGAMEMODE", |_| RunGameMode::default());
    r.register_fn("GAMEMODE", |_| GameMode);
    r.register_fn("RUNGAMELOGIC", |_| RunGameLogic::default());
    r.register_fn("LOAD_ALL", |_| LoadAll);
    r.register_fn("BINGAME", |_| InGame::default());
    r.register_fn("HSM_PLAYING", |_| Playing);
    r.register_fn("DBAUTOSELWD", |_| AutoSelectWorld);
    r.register_fn("SELECT_PROF", |_| SelectProfile);
    r.register_fn("WAIT4FEEDBK", |_| WaitForFeedbackGroup);
    r.register_fn("WAITFORVEHIC", |_| WaitForVehicleList);
    r.register_fn("ISNEWPROFILE", |_| IsNewProfile);
    r.register_fn("ISAUTOTESTIN", |_| IsAutoTesting);
    r.register_fn("HSB_STRSCRPT", |_| StartupScript);
    r.register_fn("REBOOT_REQ", |_| RebootRequest);
    r.register_fn("ENABLETIME", |_| EnableTime);
    r.register_fn("COLLECTABLES", |_| Collectables);
    r.register_fn("LOAD_FDBK_GR", |_| LoadFeedbackGroup::default());
    r.register_fn("PRELOADCOPS", |_| PreloadCops::default());
    r.register_fn("LIST_INVITE", |_| ListenForInvite);
}

/// `YES` (`DAT_0117a9b0`) and `NO` (`DAT_0117a020`).
pub const YES: u64 = 0xd9f4_777e_1800_0000;
pub const NO: u64 = 0x9b44_d8f4_c000_0000;

/// "Load World" (`LOADWORLD`, factory 0x77bc40, vtable 0xfd983c): asks for the selected world on enter (0x74e040)
/// and exits through `EXIT` at once (0x73ddf0); the load itself is waited for elsewhere (`WorldLoaded`).
pub struct LoadWorld;

impl Behaviour for LoadWorld {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::LoadWorld);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Load Profile" (`LOADPROFILE`, factory 0x77bec0, vtable 0xfd9968): enters once the save system is ready and
/// starts the load (0x73da30); exits once it ended with data (0x73da60), through `EXIT_NO_PRF` when entered
/// through `ENTER_NO_PRF`, else `EXIT`; applies the profile's settings on exit (0x744040).
pub struct LoadProfile;

impl Behaviour for LoadProfile {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::ProfileReady) {
            return false;
        }
        cx.host.request(Request::LoadProfile);
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::ProfileLoaded) {
            return false;
        }
        let exit = if cx.entry_point() == key("ENTER_NO_PRF") { key("EXIT_NO_PRF") } else { EXIT };
        cx.set_exit(exit);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ApplyProfileSettings);
    }
}

/// "Is In Online Game" (`IN_ON_GAME`, factory 0x779590, vtable 0xfd8dec): exits at once through `YES` when in an
/// online game, else `NO` (0x73d3d0).
pub struct InOnlineGame;

impl Behaviour for InOnlineGame {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let online = ask_bool(cx, Query::OnlineGame);
        cx.set_exit(if online { YES } else { NO });
        true
    }
}

/// "Run Game Mode" (`RUNGAMEMODE`, factory 0x77c340, vtable 0xfd9b8c, 0x58 B): on enter (0x7450e0) starts the
/// next event (G+0x2f6f8) in the game-mode manager; enters once the mode started (0x73ee30); while that event
/// runs keeps its HSM machine's serial (0x73ee50); when it ended exits through the machine's exit point if the
/// parent handles it (`FUN_00774dc0`), else `EXIT`. Exited from outside while the mode runs (0x73eed0) it stops
/// the mode and its machine and waits for the machine to finish (0x73ef30).
#[derive(Default)]
pub struct RunGameMode {
    event: u32,
    running: bool,
    machine: Option<u32>,
}

impl Behaviour for RunGameMode {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.event = ask(cx, Query::NextEvent) as u32;
        cx.host.request(Request::StartGameMode { event: self.event });
        self.running = true;
        self.machine = None;
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::GameModeStarted)
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let current = ask(cx, Query::GameModeEvent);
        if current == u64::MAX || current == u64::from(self.event) {
            let m = ask(cx, Query::GameModeMachine);
            self.machine = u32::try_from(m).ok();
            return false;
        }
        self.running = false;
        match self.machine.and_then(|m| cx.machine_result(m)) {
            Some(exit) if cx.parent_handles(exit) => cx.set_exit(exit),
            _ => cx.set_exit(EXIT),
        }
        true
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        if self.running {
            cx.host.request(Request::StopGameMode);
            if let Some(m) = self.machine {
                cx.request_machine_exit(m);
            }
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        !(self.running && self.machine.is_some_and(|m| cx.machine_running(m)))
    }
}

/// `DISCONNECT`, GAMEMODE's exit point when an online session is lost (0x75e6f0, `FUN_009ed220("DISCONNECT")`).
pub const DISCONNECT: &str = "DISCONNECT";

/// "Game Mode" (`GAMEMODE`, factory 0x77d8f0, vtable 0xfda5fc, 0xe0 B): the top-level state of a mode's machine;
/// runs its children. Ported: the update's exit through `DISCONNECT` when an online session is lost (0x75e6f0).
/// Not ported (online and telemetry; listed for the next step): enter 0x7430f0 (the `ActiveGameMode` data
/// definition looked up, Autolog refreshes on G+0x2f454 +0x4af0 / +0x4b40 / +0x59f0, the "GAMEEVNTSTRT"
/// telemetry event), the per-update upkeep of the `ActiveGameMode` view model's 12 player slots (0x755c70,
/// 0x743560, 0x743330, 0x7562a0, 0x75ada0), exit 0x73c6a0 (G+0x2f454 +0x59e0 / +0x5a05 cleared, `FUN_0072ca20`).
pub struct GameMode;

impl Behaviour for GameMode {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::SessionLost) {
            cx.host.request(Request::SessionLost);
            cx.set_exit(key(DISCONNECT));
            return true;
        }
        false
    }
}

/// Playback ids for [`Request::PlaySequence`] (the original's handle comes back from the player).
pub(crate) static PLAYBACK: AtomicU32 = AtomicU32::new(1);

/// "Run Game Logic" (`RUNGAMELOGIC`, factory 0x77dbc0, vtable 0xfda75c, 0x60 B): on enter (0x75c870) plays the
/// sequence given by the `BindingPath` binding, or `Sequence` (0x744fe0), on the player chosen by `Style`
/// (0–5), else by `UseSimTimer` (true: 1, else 0), else none; exits through `EXIT` once it no longer plays
/// (0x73ed60; at once when there is none); exited from outside, stops it (0x73edd0) and waits until a style 2–5
/// player is idle (0x744fa0).
#[derive(Default)]
pub struct RunGameLogic {
    playback: Option<u32>,
    player: Option<u8>,
}

impl RunGameLogic {
    /// The player of 0x75c870: `Style` when the state has it (values outside 0–5 pick none), else 1 for
    /// `UseSimTimer` true, 0 for `UseSimTimer` false, none without either.
    fn player(cx: &Cx) -> Option<u8> {
        if cx.param("Style").is_some() {
            return u8::try_from(param_int(cx, "Style")).ok().filter(|&s| s <= 5);
        }
        if cx.param("UseSimTimer").is_some() {
            return Some(if param_true(cx, "UseSimTimer") { 1 } else { 0 });
        }
        None
    }
}

impl Behaviour for RunGameLogic {
    fn on_enter(&mut self, cx: &mut Cx) {
        let path = cx.param_text("BindingPath");
        let sequence = binding_or_param(cx, &path, "Sequence") as u32;
        self.playback = None;
        self.player = None;
        if sequence == 0 {
            return;
        }
        let playback = PLAYBACK.fetch_add(1, Ordering::Relaxed);
        self.player = Self::player(cx);
        cx.host.request(Request::PlaySequence { playback, sequence, player: self.player });
        self.playback = Some(playback);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if let Some(playback) = self.playback
            && ask_bool(cx, Query::SequencePlaying { playback })
        {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        if let Some(playback) = self.playback.take() {
            cx.host.request(Request::StopPlayback { playback });
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        match self.player {
            Some(p @ 2..=5) => !ask_bool(cx, Query::SequencePlayerBusy { player: p }),
            _ => true,
        }
    }
}

/// "Load All" (`LOAD_ALL`, factory 0x779990, vtable 0xfd8ec4): opens the world-loading gate and exits through
/// `EXIT` at once (0x740f00).
pub struct LoadAll;

impl Behaviour for LoadAll {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::LoadAll);
        cx.set_exit(EXIT);
        true
    }
}

/// "InGame" (`BINGAME`, factory 0x77ba10, vtable 0xfd9740, 0x50 B): on enter (0x74d460; its UI call
/// `FUN_00790130` is empty) nothing; the first enter update posts the in-game start (0x73d130) and enters; runs
/// its children; when exiting, leaves the game once (0x73d1d0) and finishes when that is done.
#[derive(Default)]
pub struct InGame {
    started: bool,
    leaving: bool,
}

impl Behaviour for InGame {
    fn on_enter(&mut self, _cx: &mut Cx) {
        *self = Self::default();
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if !self.started {
            cx.host.request(Request::EnterGame);
            self.started = true;
        }
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if !self.leaving {
            cx.host.request(Request::LeaveGame);
            self.leaving = true;
        }
        !ask_bool(cx, Query::GameLeaving)
    }
}

/// "Playing" (`HSM_PLAYING`, factory 0x77c430, vtable 0xfd9c04, 0x50 B): starts the play session on enter
/// (0x73e7e0) and stops it on exit (0x73e930); runs its children. Its exit update (0x73e8e0, a switch whose cases
/// fall through) finishes at once.
pub struct Playing;

impl Behaviour for Playing {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StartPlaying);
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StopPlaying);
    }
}

/// "DEBUG Autoselect World" (`DBAUTOSELWD`, factory 0x77d670, vtable 0xfda4cc): selects the configured debug
/// world, else the world list's only world, and exits through `SELECTED`; with neither, `UNSELECTED`
/// (0x7702e0).
pub struct AutoSelectWorld;

impl Behaviour for AutoSelectWorld {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::AutoSelectableWorld) {
            cx.host.request(Request::AutoSelectWorld);
            cx.set_exit(key("SELECTED"));
        } else {
            cx.set_exit(key("UNSELECTED"));
        }
        true
    }
}

/// "Select Profile" (`SELECT_PROF`, factory 0x77bf10, vtable 0xfd998c): waits for the platform user's id
/// (0x73f270), names the profile after it and exits through `SUCCESS`; posts message 0xc on exit (0x75cd00).
pub struct SelectProfile;

impl Behaviour for SelectProfile {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let user = ask(cx, Query::UserId);
        if user == 0 {
            return false;
        }
        cx.host.request(Request::SetProfileUser { user });
        cx.set_exit(key("SUCCESS"));
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ProfileUserSelected);
    }
}

/// "Wait For Feedback Group" (`WAIT4FEEDBK`, factory 0x77d2b0, vtable 0xfda2f4, update 0x746a50): the group is
/// `GROUP` (atoi), replaced by the value of the `GROUPBINDING` binding when that parameter is a non-empty path
/// (parameter names read from the snapshot's keys at `DAT_0117a650` / `DAT_0117a580`); exits through `EXIT` once
/// the group is loaded (`FUN_00792590`), or at once for group 0.
pub struct WaitForFeedbackGroup;

impl Behaviour for WaitForFeedbackGroup {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let path = cx.param_text("GROUPBINDING");
        let group = binding_or_param(cx, &path, "GROUP");
        if group != 0 && !ask_bool(cx, Query::FeedbackGroupLoaded { group }) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Wait For Vehicle List" (`WAITFORVEHIC`, factory 0x77d850, vtable 0xfda5b4): exits through `EXIT` once the
/// vehicle list is loaded (0x741b70).
pub struct WaitForVehicleList;

impl Behaviour for WaitForVehicleList {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::VehicleListLoaded) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "HSM.Is Brand New Profile" (`ISNEWPROFILE`, factory 0x77e540, vtable 0xfdab78): exits at once through `Yes`
/// for a new profile, else `No` (0x73d350).
pub struct IsNewProfile;

impl Behaviour for IsNewProfile {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let new = ask_bool(cx, Query::NewProfile);
        cx.set_exit(if new { YES } else { NO });
        true
    }
}

/// "Is Auto Testing" (`ISAUTOTESTIN`, factory 0x77e590, vtable 0xfdab9c): exits at once through `No` on the
/// retail PC build (0x76e8a0).
pub struct IsAutoTesting;

impl Behaviour for IsAutoTesting {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(NO);
        true
    }
}

/// "Run Startup Script" (`HSB_STRSCRPT`, factory 0x77bba0, vtable 0xfd97f4): its only own slot is the enter
/// update 0x73f110 (`return 1`, like the base's): runs its children.
pub struct StartupScript;

impl Behaviour for StartupScript {}

/// "Wait For System Reboot Request" (`REBOOT_REQ`, factory 0x779de0, vtable 0xfd8f78): exits through `EXIT` once
/// a reboot was requested (update 0x741a90).
pub struct RebootRequest;

impl Behaviour for RebootRequest {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::RebootRequested) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Enable Time" (`ENABLETIME`, factory 0x77f270, vtable 0xfdb13c): counts G+0x25624 up on enter (0x73c4d0) and
/// down on exit (0x73c4e0); runs its children.
pub struct EnableTime;

impl Behaviour for EnableTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdjustTimeEnabled { delta: 1 });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdjustTimeEnabled { delta: -1 });
    }
}

/// "Collectables" (`COLLECTABLES`, factory 0x77b4c0, vtable 0xfd94e8): enters once the world state is loaded and
/// resets the collectables (enter update 0x73c2f0); stops them on exit (0x73c330); runs its children (update
/// 0x73c350 returns 0, on enter 0x73c2e0 is empty).
pub struct Collectables;

impl Behaviour for Collectables {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::CollectablesReady) {
            return false;
        }
        cx.host.request(Request::StartCollectables);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StopCollectables);
    }
}

/// "Load Feedback Message Group" (`LOAD_FDBK_GR`, factory 0x77cf60, vtable 0xfda13c): the group is `GROUP`
/// (atoi) or the `GROUPBINDING` binding's value (as `WAIT4FEEDBK`); on enter (0x743ee0) a nonzero group is loaded;
/// enters once it is loaded, at once for group 0 (0x73d990 / 0x73d9a0); on exit (0x743ff0) it is released when
/// `UNLOADONEXIT` is "true" (`_stricmp`).
#[derive(Default)]
pub struct LoadFeedbackGroup {
    group: i64,
}

impl Behaviour for LoadFeedbackGroup {
    fn on_enter(&mut self, cx: &mut Cx) {
        let path = cx.param_text("GROUPBINDING");
        self.group = binding_or_param(cx, &path, "GROUP");
        if self.group != 0 {
            cx.host.request(Request::LoadFeedbackGroup { group: self.group });
        }
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.group == 0 || ask_bool(cx, Query::FeedbackGroupLoaded { group: self.group })
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if param_true(cx, "UNLOADONEXIT") && self.group != 0 {
            cx.host.request(Request::UnloadFeedbackGroup { group: self.group });
        }
    }
}

/// The vehicle `PRELOADCOPS` always lists first when cops take part (0x75c313: `mov [esi+0xcc], 0x1df59`).
pub const DEFAULT_COP_VEHICLE: u32 = 0x1df59;

/// "Preload Cop Cars" (`PRELOADCOPS`, factory 0x77c140, vtable 0xfd9a94, 0x118 B): keeps the vehicle bundles of
/// the running game mode streamed in. Update 0x75c1c0 (never exits; nothing while a load or release runs): with
/// a started mode, a local player and an offline event it watches the event, the local player's car and, when cops
/// take part, a heat record field; on a change (or the first time players exist) it re-lists the wanted cars: the
/// default cop car and the cop vehicle table's cars when cops take part, then, outside free drive, every player's
/// car; without those it forgets everything. Each update then streams one step (`FUN_00757f10`). On exit (exit
/// update 0x758030) it lets everything go and finishes once nothing is loaded. On enter (0x744c50) all is reset.
#[derive(Default)]
pub struct PreloadCops {
    /// +0xcc, count +0xfc: the cars wanted in low detail.
    wanted: Vec<u32>,
    /// +0x98, count +0xc8: the cars loaded in low detail.
    loaded: Vec<u32>,
    /// +0x100: the heat record field last seen.
    heat: u64,
    /// +0x104: the event last seen (GMM+0x2ce0).
    event: u64,
    /// +0x108: the local player's car wanted in high detail.
    player_vehicle: u32,
    /// +0x10c: ... and loaded.
    loaded_player: u32,
    /// +0x110: the players have been listed.
    players_listed: bool,
}

impl PreloadCops {
    /// 0x75c4ca / 0x758030: forget the inputs and the wanted list (the loaded ones stay until streamed out).
    fn forget(&mut self) {
        self.wanted.clear();
        self.heat = 0;
        self.event = 0;
        self.player_vehicle = 0;
        self.players_listed = false;
    }

    /// `FUN_00757f10`, one step: swap the high-detail car when it changed (and nothing else this step); else load
    /// the first wanted car not loaded, then release the first loaded car no longer wanted.
    fn stream(&mut self, cx: &mut Cx) {
        if self.player_vehicle != self.loaded_player {
            if self.loaded_player != 0 {
                cx.host.request(Request::ReleaseVehicle { vehicle: self.loaded_player, hi: true });
            }
            if self.player_vehicle != 0 {
                cx.host.request(Request::StreamVehicle { vehicle: self.player_vehicle, hi: true });
            }
            self.loaded_player = self.player_vehicle;
            return;
        }
        if let Some(&v) = self.wanted.iter().find(|v| !self.loaded.contains(v)) {
            cx.host.request(Request::StreamVehicle { vehicle: v, hi: false });
            self.loaded.push(v);
        }
        if let Some(&v) = self.loaded.iter().find(|v| !self.wanted.contains(v)) {
            cx.host.request(Request::ReleaseVehicle { vehicle: v, hi: false });
            self.loaded.retain(|&x| x != v);
        }
    }

    fn push_new(&mut self, v: u32) {
        if !self.wanted.contains(&v) {
            self.wanted.push(v);
        }
    }
}

impl Behaviour for PreloadCops {
    fn on_enter(&mut self, _cx: &mut Cx) {
        *self = Self::default();
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::VehicleStreamBusy) {
            return false;
        }
        let ready = ask_bool(cx, Query::GameModeStarted)
            && ask_bool(cx, Query::LocalPlayerReady)
            && !ask_bool(cx, Query::OnlineEvent);
        if !ready {
            self.forget();
            self.stream(cx);
            return false;
        }
        let mut changed = false;
        let event = ask(cx, Query::GameModeEvent);
        let car = ask(cx, Query::PlayerVehicle) as u32;
        if event != self.event {
            self.players_listed = false;
            self.event = event;
            changed = true;
        }
        if car != self.player_vehicle {
            self.player_vehicle = car;
            changed = true;
        }
        if ask_bool(cx, Query::CopsAllowed) {
            let heat = ask(cx, Query::HeatRecordKey);
            if heat != self.heat {
                self.heat = heat;
                changed = true;
            }
        }
        let relist = if !self.players_listed && ask(cx, Query::PlayerCount) > 0 {
            self.players_listed = true;
            true
        } else {
            changed
        };
        if relist {
            self.wanted.clear();
            if ask_bool(cx, Query::CopsAllowed) {
                self.wanted.push(DEFAULT_COP_VEHICLE);
                for index in 0..ask(cx, Query::CopVehicleCount) as u32 {
                    let v = ask(cx, Query::CopVehicle { index }) as u32;
                    self.push_new(v);
                }
            }
            if self.players_listed && !ask_bool(cx, Query::FreedriveEvent) {
                for index in 0..ask(cx, Query::PlayerCount) as u32 {
                    let v = ask(cx, Query::PlayerVehicleOf { index }) as u32;
                    self.push_new(v);
                }
            }
        }
        self.stream(cx);
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::VehicleStreamBusy) {
            return false;
        }
        self.forget();
        self.stream(cx);
        self.loaded.is_empty() && self.loaded_player == 0 && !ask_bool(cx, Query::VehicleStreamBusy)
    }
}

/// `DONT_JOIN`, `LIST_INVITE`'s exit when it took the invite (0x74dd80).
pub const DONT_JOIN: u64 = 0x6128_ab34_4e91_3400;

/// "Listen for Invite" (`LIST_INVITE`, factory 0x77bfb0, vtable 0xfd99e0, only the update 0x74dd80 is its own):
/// waits for an invite on the invite listener (PM+0xc9a0); a joinable one is taken (exit `DONT_JOIN`), any other
/// exits through `EXIT`. Offline no invite comes, so it waits for ever.
pub struct ListenForInvite;

impl Behaviour for ListenForInvite {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::InvitePending) {
            return false;
        }
        if ask_bool(cx, Query::InviteJoinable) {
            cx.host.request(Request::TakeInvite);
            cx.set_exit(DONT_JOIN);
        } else {
            cx.set_exit(EXIT);
        }
        true
    }
}
