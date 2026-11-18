//! The last batch of behaviour types `GAMELOGIC/GAMEMODES.BNDL` uses: controller binding and the debug flows
//! (`BINDCONTROLL`, `DBGAUTOBIND`, `DBGSTARTFRID`, `DEBUGUNLOCKS`, `HSM.ENABLEDE`, `HSM.CHECKFOR`, `HSM.SELECTEA`),
//! the Blaze game's life (`LAUNCH`, `ENDGAME`, `RESETGAME`, `HSM.UPLOADON`, `WAIT4ALLSYN`), the online free drive
//! (`JOINEDLEFT`, `MMPICONSONL`, `ONLINECARSWA`, `SHOWMEETUPPO`) and event / UI helpers (`KILLCAMSEQ`,
//! `SHOWCARSELEC`, `STARTCARSELE`, `SHOW_DIALOG`, `TRYTOPLAYSPE`, `WAYPOINTS`).
//!
//! Method as the earlier batches: the factory is the `mov edx` of the name stub (the function after the factory,
//! e.g. 0x77adc0 for `BINDCONTROLL`), the vtable the factory's `mov [eax], vtable`, the slot bodies from the dump or
//! `text.asm`, the `.bss` parameter and exit keys decoded from snapshot 1. Slot order: enter, enter update,
//! running, update, exit update, exit begin, exit. Engine effects are [`Request`]s, engine state [`Query`]s.
//! Debug-variable registrations (`FUN_00a4e2c0` / `FUN_00a4e350` under "HSMBehaviour…", the registry `FUN_00a4d2a0`
//! / `FUN_00a4d3f0` on G+0xe8) are not ported unless a binding root reads them (`SHOW_DIALOG`, `SHOWMEETUPPO`).

use std::sync::atomic::Ordering;

use mw_formats::hsm::key;
use mw_formats::json::Json;

use super::game::PLAYBACK;
use super::online::NetMessage;
use super::ui::RunLayout;
use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("BINDCONTROLL", |_| BindController::default());
    r.register_fn("DBGAUTOBIND", |_| DebugAutobind::default());
    r.register_fn("DBGSTARTFRID", |_| DebugStartFreeRide);
    r.register_fn("DEBUGUNLOCKS", |_| AwardDebugUnlocks);
    r.register_fn("HSM.ENABLEDE", |_| EnableDemoMode);
    r.register_fn("HSM.CHECKFOR", |_| CheckForFalseStarts);
    r.register_fn("HSM.SELECTEA", |_| SelectEasyDriveShortcut);
    r.register_fn("LAUNCH", |_| LaunchGame::default());
    r.register_fn("ENDGAME", |_| EndGame::default());
    r.register_fn("RESETGAME", |_| ResetGame);
    r.register_fn("HSM.UPLOADON", |_| UploadOnlineUnlocks);
    r.register_fn("WAIT4ALLSYN", |_| WaitForAllSynced::default());
    r.register_fn("JOINEDLEFT", |_| PlayerJoinedLeft::default());
    r.register_fn("MMPICONSONL", |_| MinimapIconsOnline);
    r.register_fn("ONLINECARSWA", |_| OnlineCarSwap::default());
    r.register_fn("KILLCAMSEQ", |_| TakedownSequence::default());
    r.register_fn("SHOWCARSELEC", |_| ShowCarSelect);
    r.register_fn("STARTCARSELE", |_| StartCarSelect::default());
    r.register_fn("SHOW_DIALOG", |_| ShowUserFeedbackDialog::default());
    r.register_fn("TRYTOPLAYSPE", |_| TryToPlaySpeech::default());
    r.register_fn("WAYPOINTS", |_| IntroWaypoints::default());
    r.register_fn("SHOWMEETUPPO", |_| ShowMeetupPoi::default());
}

/// `FUN_00774dc0`: exit through `exit` when the parent handles it; whether it did.
fn exit_through(cx: &mut Cx, exit: u64) -> bool {
    if cx.parent_handles(exit) {
        cx.set_exit(exit);
        true
    } else {
        false
    }
}

fn exit(cx: &mut Cx, exit: u64) -> bool {
    cx.set_exit(exit);
    true
}

// ---------------------------------------------------------------------------------------------------------------
// Controller binding and debug flows.

/// The binding listen of `HSB_P1BIND` (G+0x2f720 / +0x2f722 / user slot +0x2f48c), sent when it changes.
#[derive(Default)]
struct Listen {
    on: bool,
}

impl Listen {
    fn set(&mut self, cx: &mut Cx, on: bool, any_button: bool) {
        if self.on != on {
            cx.host.request(Request::ListenForPrimaryUser { listen: on, any_button });
            self.on = on;
        }
    }
}

/// The release of G+0x2f721 while the primary user is still bound, sent when it changes.
#[derive(Default)]
struct Release {
    on: bool,
}

impl Release {
    fn set(&mut self, cx: &mut Cx, on: bool) {
        if self.on != on {
            cx.host.request(Request::ReleasePrimaryUser(on));
            self.on = on;
        }
    }
}

/// "Bind Controller Behaviour" (`BINDCONTROLL`, factory 0x77ad90, vtable 0xfd9200, 0x50 B): `HSB_P1BIND` without
/// the player creation. The enter update (0x758a80) releases the primary user (G+0x2f721 = 1) while one is bound
/// (the user record `FUN_0047aa20`(0) is not 1), then clears the release, sets the poll flag +0x48 and re-reads
/// the pads' neutral values (flag 8 of `*0x010edb18`, message {8, 0}: [`Request::ResetNeutralValues`]). The update
/// (0x741fd0) first waits while the flag is set and user 0 still has events of kind 0x27 pending
/// ([`Query::PadInputPending`]), then clears it; unbound it listens for the binding press (`ANYBUTTON` "true":
/// any button; key 0x117a2e0) and stays; bound it stops listening and exits through `EXIT`. The exit update
/// (0x73b270) stops the listen when it exits otherwise.
#[derive(Default)]
pub struct BindController {
    poll: bool,
    release: Release,
    listen: Listen,
}

impl Behaviour for BindController {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::PrimaryUserBound) {
            self.release.set(cx, true);
            return false;
        }
        self.release.on = true;
        self.release.set(cx, false);
        self.poll = true;
        cx.host.request(Request::ResetNeutralValues);
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.poll && ask_bool(cx, Query::PadInputPending) {
            return false;
        }
        self.poll = false;
        if !ask_bool(cx, Query::PrimaryUserBound) {
            let any_button = param_true(cx, "ANYBUTTON");
            self.listen.set(cx, true, any_button);
            return false;
        }
        self.listen.on = true;
        self.listen.set(cx, false, false);
        exit(cx, EXIT)
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if cx.exit() != EXIT {
            self.listen.on = true;
            self.listen.set(cx, false, false);
        }
        true
    }
}

/// "DEBUG Autobind Pad 0" (`DBGAUTOBIND`, factory 0x77d620, vtable 0xfda4a8, 0x48 B): the enter update (0x76e7b0)
/// is `HSB_P1BIND`'s (release while bound, then every player removed, `FUN_006ce500`(2), players +0xa6b8 = −1);
/// the update (0x770610) listens with any button on user slot 0 and queues the auto-bind message {5, 0}
/// ([`Request::DebugAutobindPad`]) every frame while unbound; bound, it stops listening, creates the local player
/// (`FUN_006d6090`) and exits through `EXIT` (key 0x117b218). The exit update (0x76f600) is `HSB_P1BIND`'s: stop
/// listening when exited otherwise, else finish once the local player exists (players +0xa6b8 = 0).
#[derive(Default)]
pub struct DebugAutobind {
    release: Release,
    listen: Listen,
}

impl Behaviour for DebugAutobind {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::PrimaryUserBound) {
            self.release.set(cx, true);
            return false;
        }
        cx.host.request(Request::RemovePlayers);
        self.release.on = true;
        self.release.set(cx, false);
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::PrimaryUserBound) {
            self.listen.set(cx, true, true);
            cx.host.request(Request::DebugAutobindPad);
            return false;
        }
        self.listen.on = true;
        self.listen.set(cx, false, false);
        cx.host.request(Request::CreateLocalPlayer);
        exit(cx, EXIT)
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if cx.exit() != EXIT {
            self.listen.on = true;
            self.listen.set(cx, false, false);
            return true;
        }
        ask_bool(cx, Query::LocalPlayerReady)
    }
}

/// "DEBUG Start Free Ride" (`DBGSTARTFRID`, factory 0x77d710, vtable 0xfda514, 0x48 B): only the update
/// (0x7703d0, `text.asm`): once the current event data is set ([`Query::DebugEventDataSet`]) and
/// `FUN_0083bd20` answers ([`Query::DebugWorldReady`]), the single-player rollout (`FUN_0068fd40` →
/// `FUN_0068e5c0`) and the free-ride event as the pending one (`FUN_0063e380` on G+0x2f4c8 with the online flag
/// `FUN_004c1ec0`), then `EXIT` (key 0x117b218).
pub struct DebugStartFreeRide;

impl Behaviour for DebugStartFreeRide {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::DebugEventDataSet) || !ask_bool(cx, Query::DebugWorldReady) {
            return false;
        }
        let event = ask(cx, Query::DebugFreeRideEvent) as u32;
        cx.host.request(Request::SetSinglePlayerRollout);
        let online = ask_bool(cx, Query::OnlineGame);
        cx.host.request(Request::SetPendingEvent { event, online });
        exit(cx, EXIT)
    }
}

/// "HSMBehaviourAwardDebugTicketsMilestonesAndUnlocks" (`DEBUGUNLOCKS`, factory 0x77e810, vtable 0xfdacbc, 0x48 B):
/// the retail enter (0x73b240) is a bare `ret` (the award is compiled out); the update (0x73b250) exits through
/// `EXIT`.
pub struct AwardDebugUnlocks;

impl Behaviour for AwardDebugUnlocks {
    fn update(&mut self, cx: &mut Cx) -> bool {
        exit(cx, EXIT)
    }
}

/// "HSM.EnableDemoMode" (`HSM.ENABLEDE`, factory 0x77e4a0, vtable 0xfdab30, 0x48 B): the enter (0x756770) only
/// registers the debug variables of "HSMBehaviourHSMEnableDemoMode" (`IsMultiplayerShowFloor`,
/// `IsMultiplayerAndSinglePlayerShowFloorCombo`, `IsE3VITADemo`, `StationId`: getters 0x7687e0..0x768810), the
/// update (0x73c950) is 0 (it runs its children) and the exit (0x73c960) unregisters them. The `DEMOMODE`
/// parameter is not read by the retail code.
pub struct EnableDemoMode;

impl Behaviour for EnableDemoMode {}

/// "HSM.CheckForFalseStarts" (`HSM.CHECKFOR`, factory 0x77e220, vtable 0xfdaa10, 0x58 B): the enter (0x7568c0)
/// registers a debug variable and caches the current event's record (`FUN_006e5770` on GMM+4 with GMM+0x2ce0 into
/// +0x4c; +0x48 = 0, +0x50 = −1) that only the debug getter (0x768820) reads; the update (0x73cdb0) is 0 (it runs
/// its children); the exit (0x73cdc0) unregisters. No false-start rule is in the retail behaviour.
pub struct CheckForFalseStarts;

impl Behaviour for CheckForFalseStarts {}

/// The Easy Drive shortcut values of UIM+0x33608 and the exit points `HSM.SELECTEA` takes for them (keys from
/// snapshot 1: 0x117ac18 `EZ_RACES`, 0x117a010 `EZ_MODS`, 0x117a0b0 `EZ_CARS`, 0x117a1f0 `EZ_M_WANTED`,
/// 0x117ac28 `EZ_RECOMM`, 0x117a948 `EZ_MULTIPLAY`, 0x117a340 `EZ_RETRY`, 0x117a700 `EZ_LEAVE`, 0x117a060
/// `EZ_SPEC_MOD`).
pub const EASY_DRIVE_EXITS: [(u64, &str); 8] = [
    (0xd, "EZ_RACES"),
    (0x10, "EZ_MODS"),
    (0xf, "EZ_CARS"),
    (0x11, "EZ_M_WANTED"),
    (0x12, "EZ_RECOMM"),
    (0xe, "EZ_MULTIPLAY"),
    (0x1a, "EZ_RETRY"),
    (0x1b, "EZ_LEAVE"),
];

/// "HSM.SelectEasyDriveShortcut" (`HSM.SELECTEA`, factory 0x77e630, vtable 0xfdabe4, 0x48 B): the update
/// (0x73ca40) maps the shortcut the UI picked ([`Query::EasyDriveShortcut`]) to an exit point
/// ([`EASY_DRIVE_EXITS`]); 0x1c..0x20 first store the vehicle class 0..4 at (G+0x2f404)+0x8f9c
/// ([`Request::SetKinectSelectedClass`]) and exit through `EZ_SPEC_MOD`. Each exit goes through `FUN_00774dc0`
/// (only when the parent handles it); any other value stays.
pub struct SelectEasyDriveShortcut;

impl Behaviour for SelectEasyDriveShortcut {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let v = ask(cx, Query::EasyDriveShortcut);
        if (0x1c..=0x20).contains(&v) {
            cx.host.request(Request::SetKinectSelectedClass { class: (v - 0x1c) as i32 });
            return exit_through(cx, key("EZ_SPEC_MOD"));
        }
        match EASY_DRIVE_EXITS.iter().find(|(s, _)| *s == v) {
            Some(&(_, name)) => exit_through(cx, key(name)),
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The Blaze game.

/// A request object's state ([`Query::NetRequestState`]): 0 idle, 1 pending, 2 done.
const IDLE: u64 = 0;
const DONE: u64 = 2;

fn net_state(cx: &mut Cx) -> u64 {
    ask(cx, Query::NetRequestState { slot: 0 })
}

/// The exit update both `ENDGAME` (0x73c560) and `RESETGAME` (0x73ecb0) use: a done request is acknowledged and
/// finishes, an idle one finishes, a pending one is pumped.
fn finish_request(cx: &mut Cx) -> bool {
    match net_state(cx) {
        DONE => {
            cx.host.request(Request::NetAck { slot: 0 });
            true
        }
        IDLE => true,
        _ => {
            cx.host.request(Request::NetPump { slot: 0 });
            false
        }
    }
}

/// "Launch Blaze Game" (`LAUNCH`, factory 0x77d140, vtable 0xfda220, 0x88 B; request object +0x48, sent flag
/// +0x80). Enter (0x73d750): the object reset, the flag cleared, the sign-in state's string +0x102c = 117673
/// ([`Request::SetSignInStatusString`]). Update (0x74da00): pumps the object; a done one is acknowledged, after an
/// error (+0x54) only when its reply word +0x64 is 2 ([`Query::NetRequestResult`]), and then the flag is cleared
/// (sent again); once the game started (`FUN_004c1f00`, [`Query::NetGameStarted`]) exits through `EXIT`; else,
/// not migrating, the game admin, everyone connected, the object idle and nothing sent: message 0x22
/// ([`NetMessage::LaunchGame`]). Exit update (0x73d7c0): a busy object is pumped until done, then acknowledged.
#[derive(Default)]
pub struct LaunchGame {
    sent: bool,
}

/// `LAUNCH`'s string (`mov [eax+0x102c], 0x1cba9` at 0x73d771).
pub const LAUNCH_STATUS_STRING: u32 = 0x1_cba9;

impl Behaviour for LaunchGame {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::NetReset { slot: 0 });
        self.sent = false;
        cx.host.request(Request::SetSignInStatusString { string: LAUNCH_STATUS_STRING });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::NetPump { slot: 0 });
        let mut state = net_state(cx);
        if state == DONE {
            let error = ask(cx, Query::NetRequestError { slot: 0 });
            let ack = error == 0 || ask(cx, Query::NetRequestResult { slot: 0 }) == 2;
            if ack {
                if error != 0 {
                    self.sent = false;
                }
                cx.host.request(Request::NetAck { slot: 0 });
                state = IDLE;
            }
        }
        if ask_bool(cx, Query::NetGameStarted) {
            return exit(cx, EXIT);
        }
        if !ask_bool(cx, Query::NetHostMigrating)
            && ask_bool(cx, Query::NetGameAdmin)
            && ask_bool(cx, Query::NetAllPlayersConnected)
            && state == IDLE
            && !self.sent
        {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::LaunchGame });
            self.sent = true;
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if net_state(cx) != IDLE {
            cx.host.request(Request::NetPump { slot: 0 });
            if net_state(cx) != DONE {
                return false;
            }
            cx.host.request(Request::NetAck { slot: 0 });
        }
        true
    }
}

/// "End Blaze Game" (`ENDGAME`, factory 0x77d940, vtable 0xfda620, 0x88 B; request object +0x48, sent flag +0x80).
/// Enter (0x73c540): reset, flag cleared. Update (0x74b780): exits through `EXIT` (`FUN_009ed220("EXIT")`) once the
/// game is no longer started ([`Query::NetGameStarted`]); a done object is acknowledged; nothing sent, idle, the
/// game admin and not migrating: message 0x23 ([`NetMessage::EndGame`]); otherwise the object is pumped. Exit
/// update (0x73c560): [`finish_request`]. Exit (0x73c590): as the game admin, G+0x2a9dc += 1
/// ([`Request::CountOnlineGame`]).
#[derive(Default)]
pub struct EndGame {
    sent: bool,
}

impl Behaviour for EndGame {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::NetReset { slot: 0 });
        self.sent = false;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::NetGameStarted) {
            return exit(cx, EXIT);
        }
        let state = net_state(cx);
        if state == DONE {
            cx.host.request(Request::NetAck { slot: 0 });
        } else if !self.sent
            && state == IDLE
            && ask_bool(cx, Query::NetGameAdmin)
            && !ask_bool(cx, Query::NetHostMigrating)
        {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::EndGame });
            self.sent = true;
        } else {
            cx.host.request(Request::NetPump { slot: 0 });
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        finish_request(cx)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if ask_bool(cx, Query::NetGameAdmin) {
            cx.host.request(Request::CountOnlineGame);
        }
    }
}

/// "Reset Blaze Game" (`RESETGAME`, factory 0x77a240, vtable 0xfd9050, 0x80 B; request object +0x48). Enter
/// (0x73eca0): reset. Update (0x74fb10): exits through `EXIT` unless the game is in its post-game state
/// (`FUN_004c1f30`, state 4: [`Query::SessionRaceState`]); a done object is acknowledged; idle, the game admin and
/// not migrating: message 0x24 ([`NetMessage::ResetGame`], no sent flag: the object being busy is the guard);
/// otherwise pumped. Exit update (0x73ecb0): [`finish_request`].
pub struct ResetGame;

impl Behaviour for ResetGame {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::NetReset { slot: 0 });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::SessionRaceState) {
            return exit(cx, EXIT);
        }
        let state = net_state(cx);
        if state == DONE {
            cx.host.request(Request::NetAck { slot: 0 });
        } else if state == IDLE && ask_bool(cx, Query::NetGameAdmin) && !ask_bool(cx, Query::NetHostMigrating) {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::ResetGame });
        } else {
            cx.host.request(Request::NetPump { slot: 0 });
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        finish_request(cx)
    }
}

/// "UploadOnlineUnlocks" (`HSM.UPLOADON`, factory 0x77e6d0, vtable 0xfdac2c, 0x1a0 B; request object +0x48).
/// Enter (0x74beb0): reset (+0x80 cleared) and the unlocks message ([`NetMessage::UploadUnlocks`]; the original
/// builds and sends it only when it found unlocks). Update (0x73cc30): an idle object exits through `EXIT`; else
/// pumped, and once done acknowledged and `EXIT`. Exit update (0x73cc90): a done object is acknowledged; finishes
/// when idle, else pumps. Exit (0x73ccc0): the object cleared (`FUN_004c2840`).
pub struct UploadOnlineUnlocks;

impl Behaviour for UploadOnlineUnlocks {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::NetReset { slot: 0 });
        cx.host.request(Request::NetSend { slot: 0, message: NetMessage::UploadUnlocks });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if net_state(cx) == IDLE {
            return exit(cx, EXIT);
        }
        cx.host.request(Request::NetPump { slot: 0 });
        if net_state(cx) == DONE {
            cx.host.request(Request::NetAck { slot: 0 });
            return exit(cx, EXIT);
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        let mut state = net_state(cx);
        if state == DONE {
            cx.host.request(Request::NetAck { slot: 0 });
            state = IDLE;
        }
        if state == IDLE {
            return true;
        }
        cx.host.request(Request::NetPump { slot: 0 });
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::NetReset { slot: 0 });
    }
}

/// The network message kinds `WAIT4ALLSYN` reads from the inbox G+0x2f424: a client's "synced" (0x46, word +4 its
/// slot) and the host's "all synced" (0x47).
pub const MSG_SYNCED: u32 = 0x46;
pub const MSG_ALL_SYNCED: u32 = 0x47;
/// Player slots of an online game (`FUN_00746940`, `JOINEDLEFT`: 12).
pub const ONLINE_SLOTS: u32 = 12;

/// "HSM.Wait for all Active Players to be Synced" (`WAIT4ALLSYN`, factory 0x77ead0, vtable 0xfdaddc, 0x50 B; the
/// synced slots' mask +0x48). Enter (0x75f060): mask 0, then, unless the host is migrating (`FUN_004c22f0`), the
/// report `FUN_0075d1f0`: the host marks its own slot, a client sends "synced" ([`Request::NetSyncReport`]).
/// Update (0x75f090), not while migrating: syncing again (`FUN_004c2310`, [`Query::NetSyncing`]) repeats the
/// report; a client exits through `EXIT` when the host's "all synced" (kind 0x47) arrives; the host collects the
/// "synced" messages (kind 0x46, `FUN_007468d0`) into the mask and, once every used slot is in it or joining mid
/// game (`FUN_00746940`), broadcasts "all synced" ([`Request::NetSyncComplete`]) and exits through `EXIT`.
#[derive(Default)]
pub struct WaitForAllSynced {
    mask: u32,
}

impl WaitForAllSynced {
    fn report(&mut self, cx: &mut Cx) {
        if ask_bool(cx, Query::OnlineHost) {
            let slot = ask(cx, Query::LocalPlayerSlot);
            if slot < 32 {
                self.mask |= 1 << slot;
            }
        } else {
            cx.host.request(Request::NetSyncReport);
        }
    }

    /// `FUN_00746940`.
    fn all_synced(&self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::OnlineHost) {
            return false;
        }
        (0..ONLINE_SLOTS).all(|i| {
            !ask_bool(cx, Query::PlayerSlotUsed { index: i })
                || self.mask & (1 << i) != 0
                || ask_bool(cx, Query::PlayerMidGameJoining { index: i })
        })
    }
}

impl Behaviour for WaitForAllSynced {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.mask = 0;
        if !ask_bool(cx, Query::NetHostMigrating) {
            self.report(cx);
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::NetHostMigrating) {
            return false;
        }
        if ask_bool(cx, Query::NetSyncing) {
            self.report(cx);
        }
        if !ask_bool(cx, Query::OnlineHost) {
            if ask(cx, Query::TakeNetMessage { kind: MSG_ALL_SYNCED }) != u64::MAX {
                return exit(cx, EXIT);
            }
            return false;
        }
        loop {
            let slot = ask(cx, Query::TakeNetMessage { kind: MSG_SYNCED });
            if slot == u64::MAX {
                break;
            }
            if slot < 32 {
                self.mask |= 1 << slot;
            }
        }
        if self.all_synced(cx) {
            cx.host.request(Request::NetSyncComplete);
            return exit(cx, EXIT);
        }
        false
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The online free drive.

/// "Show Player Joined or Left Feedback" (`JOINEDLEFT`, factory 0x77cd20, vtable 0xfd9ffc, 0x298 B: 12 name
/// buffers of 0x2c B at +0x48, 12 values at +0x258, the config's three ids at +0x288..+0x290, the present mask
/// +0x294). Enter (0x751c80): every present player (players +0xa5d0[i]) is marked and its name kept
/// ([`Request::CachePlayerName`]). Update (0x75ef50), every slot but the local one (+0xa6b4): a newly present
/// player is kept, its "joined" message posted ([`Request::PlayerJoinedLeftFeedback`]) and marked; a marked slot
/// that is empty now posts "left" with the kept name and is unmarked. Never exits (update 0).
#[derive(Default)]
pub struct PlayerJoinedLeft {
    present: u32,
}

impl Behaviour for PlayerJoinedLeft {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.present = 0;
        for i in 0..ONLINE_SLOTS {
            if ask_bool(cx, Query::PlayerSlotUsed { index: i }) {
                self.present |= 1 << i;
                cx.host.request(Request::CachePlayerName { slot: i });
            }
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let local = ask(cx, Query::LocalPlayerSlot);
        for i in 0..ONLINE_SLOTS {
            if u64::from(i) == local {
                continue;
            }
            let bit = 1 << i;
            if ask_bool(cx, Query::PlayerSlotUsed { index: i }) {
                if self.present & bit == 0 {
                    cx.host.request(Request::CachePlayerName { slot: i });
                    cx.host.request(Request::PlayerJoinedLeftFeedback { slot: i, joined: true });
                    self.present |= bit;
                }
            } else if self.present & bit != 0 {
                cx.host.request(Request::PlayerJoinedLeftFeedback { slot: i, joined: false });
                self.present &= !bit;
            }
        }
        false
    }
}

/// "Minimap Icons Online" (`MMPICONSONL`, factory 0x77c4d0, vtable 0xfd9c4c, 0x200 B): the enter (0x744520) sets
/// the 12 slots' positions to 0, directions to (0, 0, 1, 0) (`.rdata` 0xfd6730) and alphas to 1.0; the update
/// (0x757a80) runs the three passes of [`Request::UpdateOnlineMinimapIcons`] every frame and never exits; the
/// exit (0x73e0f0) is empty. The `WRECKEDPOI` parameter is not read.
pub struct MinimapIconsOnline;

impl Behaviour for MinimapIconsOnline {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::UpdateOnlineMinimapIcons);
        false
    }
}

/// The local player's vehicle when it has one and it is alive (the guard of `ONLINECARSWA`'s enter and exit).
fn live_vehicle(cx: &mut Cx) -> Option<i32> {
    let v = ask(cx, Query::LocalVehicle);
    (v != u64::MAX && ask_bool(cx, Query::LocalVehicleAlive)).then_some(v as i32)
}

/// "OnlineCarSwap" (`ONLINECARSWA`, factory 0x77e2c0, vtable 0xfdaa58, 0x58 B: the sequence's flag +0x48, the
/// state +0x4c, the playback +0x50). Enter (0x757c50): state 0; the local car's damage off (message 0x1f with 1,
/// `FUN_00844fb0`: [`Request::VehicleDamage`]) when it has a live vehicle. Update (0x75bcd0): 0: the pending
/// rollout committed ([`Request::CommitPendingRollout`]) and the `Sequence` (`atoi`) played with the flag
/// ([`Request::PlaySequenceWithFlag`]), state 1; 1: once the sequence set the flag or stopped, state 2 in the
/// same frame; 2: the swap applied ([`Request::OnlineCarSwapApply`]), state 3; 3: once the local vehicle is ready
/// (`FUN_006b9430`) the player's words reset and its damage repaired (message 0x16), `EXIT`. Exit (0x73e2d0): the
/// damage back on (message 0x1f with 0) when the vehicle lives.
#[derive(Default)]
pub struct OnlineCarSwap {
    state: u8,
    playback: u32,
}

impl Behaviour for OnlineCarSwap {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.state = 0;
        self.playback = 0;
        if let Some(vehicle) = live_vehicle(cx) {
            cx.host.request(Request::VehicleDamage { vehicle, deactivated: true });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.state == 0 {
            cx.host.request(Request::CommitPendingRollout);
            let sequence = param_int(cx, "Sequence") as u32;
            self.playback = PLAYBACK.fetch_add(1, Ordering::Relaxed);
            cx.host.request(Request::PlaySequenceWithFlag { playback: self.playback, sequence, player: None });
            self.state = 1;
            return false;
        }
        if self.state == 1 {
            let playing = ask_bool(cx, Query::SequencePlaying { playback: self.playback });
            if !ask_bool(cx, Query::SequenceFlagSet { playback: self.playback }) && playing {
                return false;
            }
            self.state = 2;
        }
        if self.state == 2 {
            cx.host.request(Request::OnlineCarSwapApply);
            self.state = 3;
            return false;
        }
        if ask_bool(cx, Query::LocalVehicleReady) {
            cx.host.request(Request::OnlineCarSwapFinish);
            let vehicle = ask(cx, Query::LocalVehicle) as i64 as i32;
            cx.host.request(Request::ResetVehicleDamage { vehicle });
            return exit(cx, EXIT);
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if let Some(vehicle) = live_vehicle(cx) {
            cx.host.request(Request::VehicleDamage { vehicle, deactivated: false });
        }
    }
}

/// `SHOWMEETUPPO`'s distance before the first measurement (`.rdata` 0x00fd6b5c).
pub const MEETUP_DISTANCE: f32 = 50_000.0;

/// "ShowMeetupPOI" (`SHOWMEETUPPO`, factory 0x77b580, vtable 0xfd9530, 0x70 B), published as the binding root
/// `MeetupPOI` (type "HSMBehaviourShowMeetupPOI": `LocalPlayerInCheckpoint` +0x54, `LocalPlayerDistance` +0x58).
/// Enter (0x758440): no icons (+0x48 / +0x4c = 0x7fffffff), trigger +0x50 = 0, not in, distance 50000. Enter update
/// (0x751b10): waits for the playlist's online event (G+0x2a9d8, [`Query::OnlinePendingEvent`]); its meet-up
/// trigger ([`Query::MeetupTrigger`]), when there is one, gets its two map icons ([`Request::ShowMeetupPois`]).
/// Update (0x740b10), every frame: with a trigger, whether the local player is in it and its distance from the
/// centre. Exit (0x745eb0): the icons removed, the root unregistered.
#[derive(Default)]
pub struct ShowMeetupPoi {
    trigger: u32,
    in_checkpoint: bool,
    distance: f32,
    published: Option<(bool, u32)>,
}

impl ShowMeetupPoi {
    fn publish(&mut self, cx: &mut Cx) {
        let now = (self.in_checkpoint, self.distance.to_bits());
        if self.published == Some(now) {
            return;
        }
        self.published = Some(now);
        let data = Json::Object(vec![
            ("LocalPlayerInCheckpoint".to_owned(), Json::Bool(self.in_checkpoint)),
            ("LocalPlayerDistance".to_owned(), Json::Double(f64::from(self.distance))),
        ]);
        cx.host.publish("MeetupPOI", &data);
    }
}

impl Behaviour for ShowMeetupPoi {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.trigger = 0;
        self.in_checkpoint = false;
        self.distance = MEETUP_DISTANCE;
        self.published = None;
        self.publish(cx);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        let event = ask(cx, Query::OnlinePendingEvent) as u32;
        if event == 0 {
            return false;
        }
        let trigger = ask(cx, Query::MeetupTrigger { event }) as u32;
        if trigger != 0 {
            self.trigger = trigger;
        }
        if self.trigger != 0 {
            cx.host.request(Request::ShowMeetupPois { trigger: self.trigger });
        }
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        self.in_checkpoint = false;
        if self.trigger != 0 {
            let trigger = self.trigger;
            self.in_checkpoint = ask_bool(cx, Query::MeetupInCheckpoint { trigger });
            self.distance = f32::from_bits(ask(cx, Query::MeetupDistance { trigger }) as u32);
        }
        self.publish(cx);
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if self.trigger != 0 {
            cx.host.request(Request::RemoveMeetupPois);
        }
        cx.host.retract("MeetupPOI");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Event and UI helpers.

/// "Run Takedown Sequence" (`KILLCAMSEQ`, factory 0x77dc10, vtable 0xfda780, 0x68 B; registered by name only, the
/// name stub 0x77dc40). Enter (0x75cae0): `Sequence` (`atoi`) resolved and played on the style-2 player
/// (UIM+0x12568, `FUN_00a44f30`) with its flag (+0x58, [`Request::PlaySequenceWithFlag`]); `InputString1` /
/// `InputString2` are read (`atoi` into +0x5c / +0x60) but nothing reads them. Update (0x73ef90): `EXIT` unless the
/// sequence resolved, its flag is clear and it still plays. Exit update (0x7451b0): with a sequence, waits until the
/// style-2 player is idle ([`Query::SequencePlayerBusy`]). Exit begin (0x73efe0) is empty: the sequence is not
/// stopped.
#[derive(Default)]
pub struct TakedownSequence {
    playback: Option<u32>,
}

impl Behaviour for TakedownSequence {
    fn on_enter(&mut self, cx: &mut Cx) {
        let sequence = param_int(cx, "Sequence") as u32;
        self.playback = None;
        if sequence != 0 {
            let playback = PLAYBACK.fetch_add(1, Ordering::Relaxed);
            cx.host.request(Request::PlaySequenceWithFlag { playback, sequence, player: Some(2) });
            self.playback = Some(playback);
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if let Some(playback) = self.playback
            && !ask_bool(cx, Query::SequenceFlagSet { playback })
            && ask_bool(cx, Query::SequencePlaying { playback })
        {
            return false;
        }
        exit(cx, EXIT)
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.playback.is_none() || !ask_bool(cx, Query::SequencePlayerBusy { player: 2 })
    }
}

/// `SHOWLOADING` of `SHOWCARSELEC` (key 0x117a738): true when missing, else "true" (`_stricmp`).
fn show_loading(cx: &Cx) -> bool {
    cx.param("SHOWLOADING").is_none() || param_true(cx, "SHOWLOADING")
}

/// "Show Car Select" (`SHOWCARSELEC`, factory 0x77cc80, vtable 0xfd9fb4, 0x48 B) on the car-select manager
/// G+0x2f40c. Enter (0x7408e0): [`Request::ShowCarSelect`]. Enter update (0x740950): once its handle +0x5240 is set
/// ([`Query::CarSelectReady`]) the list gets the focus ([`Request::FocusCarSelect`]). Runs its children. Exit begin
/// (0x7409a0): when its state +0x5258 is 2, [`Request::HideCarSelect`]. Exit update (0x745dd0): waits for
/// [`Query::CarSelectClosed`].
pub struct ShowCarSelect;

impl Behaviour for ShowCarSelect {
    fn on_enter(&mut self, cx: &mut Cx) {
        let show_loading = show_loading(cx);
        cx.host.request(Request::ShowCarSelect { show_loading });
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::CarSelectReady) {
            return false;
        }
        cx.host.request(Request::FocusCarSelect);
        true
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        let show_loading = show_loading(cx);
        if ask(cx, Query::CarSelectState) == 2 {
            cx.host.request(Request::HideCarSelect { show_loading });
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::CarSelectClosed)
    }
}

/// "Start Car Select" (`STARTCARSELE`, factory 0x77cbe0, vtable 0xfd9f6c, 0x50 B: the kept world byte +0x48, online
/// +0x49). Enter (0x751fd0): `ONLINE` ("true", key 0x1179ff0) picks the car the select starts from
/// ([`Request::StartCarSelect`]); offline the world-shown byte G+0x2ef70 +0x44 is kept and set
/// ([`Request::SetWorldShownFlag`]). `CARSELCTDATA` is not read. Exit (0x740ea0): the byte restored, the car select
/// stopped ([`Request::StopCarSelect`]) and, online, the single-player rollout and the pending one committed
/// (`FUN_0068e5c0`, `FUN_0068e770`). Model decision: the online path never writes +0x48, which the exit restores;
/// taken as 0 (a fresh instance), not read live.
#[derive(Default)]
pub struct StartCarSelect {
    online: bool,
    world_shown: bool,
}

impl Behaviour for StartCarSelect {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.online = param_true(cx, "ONLINE");
        cx.host.request(Request::StartCarSelect { online: self.online });
        if !self.online {
            self.world_shown = ask_bool(cx, Query::WorldShownFlag);
            cx.host.request(Request::SetWorldShownFlag { on: true });
        }
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SetWorldShownFlag { on: self.world_shown });
        cx.host.request(Request::StopCarSelect);
        if self.online {
            cx.host.request(Request::SetSinglePlayerRollout);
            cx.host.request(Request::CommitPendingRollout);
        }
    }
}

/// The binding root `SHOW_DIALOG` registers ("HSMBehaviourShowUserFeedbackDialog").
pub const USER_FEEDBACK_DIALOG: &str = "UserFeedbackDialog";

/// "ShowUserFeedbackDialog" (`SHOW_DIALOG`, factory 0x77f950, vtable 0xfdb93c, 0x70 B): `HSB_RNLAYOUT` with the
/// dialog's strings. Enter (0x758800): the layout's enter (0x74fc80: `LAYOUT`, `RUN_HIGHEST`), then `TITLE`,
/// `BODY`, `NAV_PROMPT1..3` (`atoi`; keys 0x117a9c8, 0x117ab48, 0x117a7e8, 0x117aaa8, 0x117a878) into +0x58..+0x68,
/// published as the root `UserFeedbackDialog` (getters `Title`, `Body`, `Prompt1`..`Prompt3`, `FUN_00740db0`).
/// The enter update, exit begin and exit update are `HSB_RNLAYOUT`'s (0x745280, 0x741340, 0x7452d0); the exit
/// (0x7460b0) frees the layout's slot and unregisters the root.
#[derive(Default)]
pub struct ShowUserFeedbackDialog {
    layout: RunLayout,
}

impl Behaviour for ShowUserFeedbackDialog {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.layout.on_enter(cx);
        let fields = [("Title", "TITLE"), ("Body", "BODY"), ("Prompt1", "NAV_PROMPT1")];
        let more = [("Prompt2", "NAV_PROMPT2"), ("Prompt3", "NAV_PROMPT3")];
        let data = fields.iter().chain(more.iter()).map(|&(n, p)| (n.to_owned(), Json::Int(param_int(cx, p))));
        let data = Json::Object(data.collect());
        cx.host.publish(USER_FEEDBACK_DIALOG, &data);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.layout.enter_update(cx)
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        self.layout.on_exit_begin(cx);
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.layout.exit_update(cx)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        self.layout.on_exit(cx);
        cx.host.retract(USER_FEEDBACK_DIALOG);
    }
}

/// `FUN_00842f10`'s answer when the speech plays.
pub const SPEECH_PLAYED: u64 = 0x13;

/// Load ids for [`Request::LoadSpeechEvents`].
static SPEECH_LOAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// "Try to play speech" (`TRYTOPLAYSPE`, factory 0x77cac0, vtable 0xfd9f00, 0xa0 B: the load request +0x48 (state
/// +0x4c), the handles +0x70 + 8i, the playing one +0x90, the state +0x98, the count +0x9c). Enter (0x753260):
/// the `SPEECHEVENTS` values (key 0x117a318) loaded ([`Request::LoadSpeechEvents`]), state 1. Enter update
/// (0x7412d0): waits while the load is busy (2 / 3); state 3 when it loaded (0), else 2. Update (0x753310): 2:
/// `FAILURE`; 3: the events tried in order ([`Query::TryPlaySpeech`]), the first that plays is kept (state 4),
/// none: `FAILURE`; 4: `SUCCESS` once the speech manager no longer plays it ([`Query::SpeechPlaying`]). Exit update
/// (0x741300): waits while the load is busy. Exit keys from snapshot 1: 0x117a5a8 `FAILURE`, 0x117a360 `SUCCESS`.
#[derive(Default)]
pub struct TryToPlaySpeech {
    load: u32,
    events: Vec<u32>,
    state: u8,
    playing: u32,
}

fn speech_busy(cx: &mut Cx, load: u32) -> bool {
    matches!(ask(cx, Query::SpeechLoadState { load }), 2 | 3)
}

impl Behaviour for TryToPlaySpeech {
    fn on_enter(&mut self, cx: &mut Cx) {
        let values = cx.param("SPEECHEVENTS").map(|p| p.values.clone()).unwrap_or_default();
        self.events = values.iter().map(|v| crate::hsm::atof(v) as i64 as u32).collect();
        self.load = SPEECH_LOAD.fetch_add(1, Ordering::Relaxed);
        cx.host.request(Request::LoadSpeechEvents { load: self.load, events: self.events.clone() });
        self.playing = 0;
        self.state = 1;
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        let state = ask(cx, Query::SpeechLoadState { load: self.load });
        if matches!(state, 2 | 3) {
            return false;
        }
        self.state = if state != 0 { 2 } else { 3 };
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.state {
            2 => exit(cx, key("FAILURE")),
            3 => {
                for &event in &self.events.clone() {
                    if ask(cx, Query::TryPlaySpeech { event }) == SPEECH_PLAYED {
                        self.playing = event;
                        self.state = 4;
                        return false;
                    }
                }
                exit(cx, key("FAILURE"))
            }
            4 => {
                if ask_bool(cx, Query::SpeechPlaying { event: self.playing }) {
                    return false;
                }
                self.playing = 0;
                exit(cx, key("SUCCESS"))
            }
            _ => false,
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        !speech_busy(cx, self.load)
    }
}

/// "Do Intro Waypoints" (`WAYPOINTS`, factory 0x77b680, vtable 0xfd959c, 0x90 B: up to 16 POI handles +0x48, the
/// count +0x88, `POIID` +0x8c). Enter (0x74b530): one map POI of type `POIID` (`atoi`, key 0x117a728) per
/// waypoint of the current event ([`Request::AddEventWaypointPois`]). Update (0x74b6c0), every frame while no
/// other screen owns the route ([`Query::MapRouteBlocked`]) and there are POIs: the GPS to the first one
/// ([`Request::RouteToFirstWaypointPoi`]); never exits. Exit (0x74b660): the POIs removed, count 0.
#[derive(Default)]
pub struct IntroWaypoints {
    count: u64,
}

impl Behaviour for IntroWaypoints {
    fn on_enter(&mut self, cx: &mut Cx) {
        let poi = param_int(cx, "POIID");
        cx.host.request(Request::AddEventWaypointPois { poi });
        self.count = ask(cx, Query::WaypointPoiCount);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.count != 0 && !ask_bool(cx, Query::MapRouteBlocked) {
            cx.host.request(Request::RouteToFirstWaypointPoi);
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if self.count != 0 {
            cx.host.request(Request::RemoveEventWaypointPois);
        }
        self.count = 0;
    }
}
