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
    r.register_fn("DBAUTOSELWD", |_| AutoSelectWorld);
    r.register_fn("SELECT_PROF", |_| SelectProfile);
    r.register_fn("WAIT4FEEDBK", |_| WaitForFeedbackGroup);
    r.register_fn("WAITFORVEHIC", |_| WaitForVehicleList);
    r.register_fn("ISNEWPROFILE", |_| IsNewProfile);
    r.register_fn("ISAUTOTESTIN", |_| IsAutoTesting);
    r.register_fn("BINGAME", |_| InGame::default());
    r.register_fn("HSM_PLAYING", |_| Playing);
    r.register_fn("HSB_STRSCRPT", |_| StartupScript);
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
            cx.set_exit(key(DISCONNECT));
            return true;
        }
        false
    }
}

/// Playback ids for [`Request::PlaySequence`] (the original's handle comes back from the player).
static PLAYBACK: AtomicU32 = AtomicU32::new(1);

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
            cx.host.request(Request::StopSequence { playback });
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

/// "Wait For Feedback Group" (`WAIT4FEEDBK`, factory 0x77d2b0, vtable 0xfda2f4): exits through `EXIT` once the
/// feedback group (the `GROUPBINDING` binding's value when that is a path, else `GROUP`) is loaded, or at once
/// for group 0 (0x746a50).
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

/// "Run Startup Script" (`HSB_STRSCRPT`, factory 0x77bba0, vtable 0xfd97f4): every slot is the base class's
/// (its enter update 0x73f110 returns 1 like the base): runs its children.
pub struct StartupScript;

impl Behaviour for StartupScript {}
