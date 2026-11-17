//! Behaviour types of the race / free-roam HUD machine, the pause menu and the machines they start that deal with
//! the Autolog (the game's online service layer), downloads, the store, pending error dialogs and the game-error
//! screen (`GAMELOGIC/GAMEMODES.BNDL`). Offline each of them is a few frames long. Engine effects are
//! [`Request`]s, engine state [`Query`]s (both name the original's code). Slot order of every vtable: enter, enter
//! update, running, update, exit update, exit begin, exit; the factory is the `mov edx` of the name's registration
//! (0x777xxx..0x778xxx), the vtable the factory's `mov [eax], vtable`.

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_int};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("DWNLDPROG", |_| DownloadProgression::default());
    r.register_fn("RFRSHALMANIF", |_| RefreshManifest);
    r.register_fn("RUNSTOREENUM", |_| RunStoreEnumeration);
    r.register_fn("PAUSEAUTOLOG", |_| PauseAutolog::default());
    r.register_fn("SHOWPENDINGO", |_| ShowPendingDialog::default());
    r.register_fn("TAKEPPIC", |_| TakeProfilePicture);
    r.register_fn("SHOWVITASYST", |_| ShowVitaSystemMessage);
    r.register_fn("REFRESHENT", |_| RefreshEntitlements);
    r.register_fn("SET_GAME_ERR", |_| SetGameError);
    r.register_fn("GAMEERROR", |_| ShowGameError::default());
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

/// "HSM.Wait For Downloadable Progression Data" (`DWNLDPROG`, factory 0x77e8f0, vtable 0xfdad04, 0x50 B): state
/// +0x48. Enter (0x73ccd0) resets the transient game state (a pending state of the three player-state objects
/// G+0x2f3e4 +0x44 = 5 / G+0x2f418 +0xbd8 = 7 / G+0x2f420 +0x1a8 = 8 and the scoring manager's +0xe90 = 3 are
/// cleared, the mode's pause `FUN_006e0d20` released with `FUN_006e0d10`) and starts the download on G+0x2f454
/// +0x4af0 (`FUN_0071dfe0`): [`Request::DownloadProgressionBegin`]. The update (0x74bfc0) goes by the state:
/// 0 waits until the download object's state (G+0x2f454 +0x2d14) is 0 or 2 ([`Query::DownloadProgState`]), then
/// (like 1) checks the request object (`FUN_007daf20`, [`Query::DownloadProgValid`]): not valid exits through
/// `FAILED`; blocked by a player / scoring state or a pause ([`Query::DownloadProgBlocked`]) also `FAILED`;
/// ready to apply ([`Query::DownloadProgReady`]: the first player object's byte +0x30, the second's state 6,
/// `FUN_0073aad0`, the download's +0x4b20 = -1, scoring state 2, `FUN_006e0d30`) applies it
/// ([`Request::DownloadProgressionApply`], `FUN_00679250`) and goes to state 2; otherwise keeps waiting. State 2
/// finishes (`FUN_00686e70` on G+0x2f3e4, drops the scratch buffer G+0x2f420 +0x200 / +0x1f8:
/// [`Request::DownloadProgressionFinish`]) and exits through `EXIT`; each exit goes through `FUN_00774dc0`, so
/// it is only taken when the parent handles it. Offline model decision: the host answers "not valid".
#[derive(Default)]
pub struct DownloadProgression {
    state: u32,
}

impl Behaviour for DownloadProgression {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.state = 0;
        cx.host.request(Request::DownloadProgressionBegin);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.state {
            0 => {
                let d = ask(cx, Query::DownloadProgState);
                if d != 0 && d != 2 {
                    return false;
                }
                self.check(cx)
            }
            1 => self.check(cx),
            2 => {
                cx.host.request(Request::DownloadProgressionFinish);
                exit_through(cx, EXIT)
            }
            _ => false,
        }
    }
}

impl DownloadProgression {
    /// 0x74c06a: state 1, then the validity / blocked / ready checks.
    fn check(&mut self, cx: &mut Cx) -> bool {
        self.state = 1;
        let failed = key("FAILED");
        if !ask_bool(cx, Query::DownloadProgValid) || ask_bool(cx, Query::DownloadProgBlocked) {
            return exit_through(cx, failed);
        }
        if ask_bool(cx, Query::DownloadProgReady) {
            cx.host.request(Request::DownloadProgressionApply);
            self.state = 2;
        }
        false
    }
}

/// "Refresh Autolog Server Manifest" (`RFRSHALMANIF`, factory 0x77ed10, vtable 0xfdaed8, 0x48 B): enter (0x76ea10,
/// `FUN_0083b670` -> `FUN_007dae50` on the Autolog object G+0x2ed74) starts the manifest refresh
/// ([`Request::AutologRefreshManifest`]); the update (0x76ea30, `FUN_0083b680`) exits through `EXIT` once the
/// object's state (+0x2d0c) is 0 or 2 ([`Query::AutologManifestIdle`], default idle: offline nothing runs).
pub struct RefreshManifest;

impl Behaviour for RefreshManifest {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AutologRefreshManifest);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::AutologManifestIdle) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "RunStoreEnumeration" (`RUNSTOREENUM`, factory 0x77d530, vtable 0xfda420, 0x48 B): enter (0x73f120) starts the
/// store enumeration on the sign-in object G+0x2f3e0 +0x4ce0 (`FUN_0086f710`, [`Request::StoreEnumerate`]); the
/// update (0x73f140) exits through `EXIT` at once.
pub struct RunStoreEnumeration;

impl Behaviour for RunStoreEnumeration {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StoreEnumerate);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Ensure Connection To Autolog" (`PAUSEAUTOLOG`, factory 0x77ec70, vtable 0xfdae90, 0x50 B): holds the two
/// Autolog pollers (G+0x2ed74 and G+0x2ee60) still. Enter (0x76e8e0) remembers which are busy (+0x48 / +0x49:
/// `FUN_0083b5e0` = +0x2da4 != 0, `FUN_0083b910` = +0x8 != 0, [`Query::AutologBusy`]) and pauses those that are
/// (`FUN_0083b5c0` / `FUN_0083b8f0`, [`Request::AutologPause`]); the enter update (0x76e950) is done when neither
/// is busy any more, the exit (0x76e990) resumes the ones it paused (`FUN_0083b5d0` / `FUN_0083b900`). The
/// update is the base's (no exit of its own; the children decide).
#[derive(Default)]
pub struct PauseAutolog {
    paused: [bool; 2],
}

impl Behaviour for PauseAutolog {
    fn on_enter(&mut self, cx: &mut Cx) {
        for poller in 0..2 {
            self.paused[poller as usize] = ask_bool(cx, Query::AutologBusy { poller });
        }
        for poller in 0..2 {
            if self.paused[poller as usize] {
                cx.host.request(Request::AutologPause { poller, paused: true });
            }
        }
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        !ask_bool(cx, Query::AutologBusy { poller: 0 }) && !ask_bool(cx, Query::AutologBusy { poller: 1 })
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        for poller in 0..2 {
            if self.paused[poller as usize] {
                cx.host.request(Request::AutologPause { poller, paused: false });
            }
        }
    }
}

/// "HSM.ShowPendingOnlineErrorDialog" (`SHOWPENDINGO`, factory 0x77e940, vtable 0xfdad28, 0x58 B): shows the error
/// dialog an earlier failure left pending. Enter (0x740be0) takes the pending id (G+0x2f70c,
/// [`Query::PendingDialogId`]) and clears it ([`Request::ClearPendingDialog`]), and shows it
/// ([`Request::ShowDialog`], `FUN_007bb330` on the dialog manager G480+0x47740) unless it is 0. The update
/// (0x745f20): id 0 exits through `EXIT` at once; else it waits for the dialog's result
/// ([`Query::DialogResult`], 0xff = none), takes it once ([`Request::TakeDialogResult`]) and exits through `EXIT`.
/// The exit update (0x745f90, always done) closes a dialog still open (`FUN_007b3a10`), and the exit (0x740c30)
/// again, then takes a result that is there.
#[derive(Default)]
pub struct ShowPendingDialog {
    id: u32,
    taken: bool,
}

impl Behaviour for ShowPendingDialog {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.id = ask(cx, Query::PendingDialogId) as u32;
        self.taken = false;
        cx.host.request(Request::ClearPendingDialog);
        if self.id != 0 {
            cx.host.request(Request::ShowDialog { id: self.id });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.id != 0 && !self.taken {
            if ask(cx, Query::DialogResult { id: self.id }) as u8 == 0xff {
                return false;
            }
            cx.host.request(Request::TakeDialogResult { id: self.id });
            self.taken = true;
        }
        cx.set_exit(EXIT);
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.id != 0 && ask_bool(cx, Query::DialogOpen { id: self.id }) {
            cx.host.request(Request::CloseDialog { id: self.id, last: false });
        }
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if self.id == 0 {
            return;
        }
        if ask_bool(cx, Query::DialogOpen { id: self.id }) {
            cx.host.request(Request::CloseDialog { id: self.id, last: false });
        }
        if ask(cx, Query::DialogResult { id: self.id }) as u8 != 0xff {
            cx.host.request(Request::TakeDialogResult { id: self.id });
        }
    }
}

/// "Take a ProfilePicture" (`TAKEPPIC`, factory 0x77be70, vtable 0xfd9944, 0x58 B): nothing on enter (0x741170 is
/// a bare return); the update (0x741190) exits through `EXIT` at once; the exit (0x7411c0) removes the entry
/// "TakePic" of the owner "HSMBehaviourTakeProfilePic" from the registry at G+0xe8 (`FUN_00a4d3f0`:
/// [`Request::UnregisterHandler`]).
pub struct TakeProfilePicture;

impl Behaviour for TakeProfilePicture {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host
            .request(Request::UnregisterHandler { owner: "HSMBehaviourTakeProfilePic".into(), name: "TakePic".into() });
    }
}

/// "ShowVITASystemMessage" (`SHOWVITASYST`, factory 0x779d40, vtable 0xfd8f30, 0x48 B): the `SYSMESSAGE`
/// parameter is not read on the PC; enter (0x740e70) is a bare return, the update (0x740e80) exits through `EXIT`.
pub struct ShowVitaSystemMessage;

impl Behaviour for ShowVitaSystemMessage {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Refresh User Entitlements" (`REFRESHENT`, factory 0x77d800, vtable 0xfda590, 0x48 B): the update (0x73ec20)
/// calls `FUN_0063a4a0` on the entitlement manager G+0x29280 (its byte +0xd1c = "the check must run": true when
/// the network layer `FUN_004c2050` is not up, else +0xcec != 0: [`Request::RefreshEntitlements`]) and exits
/// through `EXIT`.
pub struct RefreshEntitlements;

impl Behaviour for RefreshEntitlements {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::RefreshEntitlements);
        cx.set_exit(EXIT);
        true
    }
}

/// "Set Autolog Disconnected Error String" (`SET_GAME_ERR`, factory 0x779cf0, vtable 0xfd8f0c, 0x48 B): enter
/// (0x75cdf0) reads the parameter "ERROR STRING" (key 0x676e259c15f55363 from `DAT_0117a378`, which the dump shows
/// as `ERROR`; `atoi`) and posts the message {9, 0, string} on the queue `*0x010edb0c` +0x10 (`FUN_0076abd0`, flag 8
/// set meanwhile: [`Request::PostGameMessage`]); the update (0x7405c0) exits through `EXIT`.
pub struct SetGameError;

impl Behaviour for SetGameError {
    fn on_enter(&mut self, cx: &mut Cx) {
        let value = param_int(cx, "ERROR STRING") as i32;
        cx.host.request(Request::PostGameMessage { id: 9, value: Some(value) });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Show Game Error Message" (`GAMEERROR`, factory 0x77ceb0, vtable 0xfda0c0, 0x50 B): enter (0x740ab0) sets a
/// counter (+0x48) to 3 and shows the generic error (`FUN_007b3410` on the object `FUN_007746a0` + 0x11190:
/// [`Request::ShowGameError`]); the update (0x740ad0) counts it down and, once below 0, exits through `EXIT`
/// when the clock object's word +0xc4c (`FUN_0046efa0`, [`Query::GameErrorHold`]) is 0; the exit (0x75ce50) posts
/// the message {0xa, 0} on the queue `*0x010edb0c` +0x10 (`FUN_0076ac50`).
#[derive(Default)]
pub struct ShowGameError {
    count: i32,
}

impl Behaviour for ShowGameError {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.count = 3;
        cx.host.request(Request::ShowGameError);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        self.count -= 1;
        if self.count >= 0 || ask_bool(cx, Query::GameErrorHold) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PostGameMessage { id: 0xa, value: None });
    }
}
