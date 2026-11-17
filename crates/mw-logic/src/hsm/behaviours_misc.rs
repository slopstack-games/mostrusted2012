//! A batch of small behaviour types of the free-drive / event / online machines (`GAMELOGIC/GAMEMODES.BNDL`):
//! spotting, the score cache, the game-mode timer exit, the invite acceptance, the game-mode swap, the Autolog
//! session variable write, the results-sync deadline, the settings-screen end, the car-select check and the
//! challenge runner. Slot order of every vtable: enter, enter update, running, update, exit update, exit begin,
//! exit. Engine effects are [`Request`]s, engine state [`Query`]s (both name the original's code).

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("ACTIVATESPOT", |_| ActivateSpotting);
    r.register_fn("CACHESCORE", |_| CachePlayersScore);
    r.register_fn("EXITWHENGAME", |_| ExitWhenGameModeTime::default());
    r.register_fn("ACCT_INVITE", |_| AcceptInvite::default());
    r.register_fn("SWAPGAMEMODE", |_| SwapGameMode);
    r.register_fn("WRT_PR_SESSN", |_| WriteSessionVariable);
    r.register_fn("SYNCRESSTART", |_| SyncOnlineResultsStart::default());
    r.register_fn("ENDSETTINGSS", |_| EndSettingsScreen);
    r.register_fn("SHOULDSHOWCA", |_| ShouldShowCarSelect);
    r.register_fn("RUNCHALL", |_| RunChallenge);
    r.register_fn("ROLLOUTACHVM", |_| RolloutAchievement);
    r.register_fn("CARCOLCORRCT", |_| CarColourCorrect);
    r.register_fn("HSM.STARTPHO", |_| StartPhotoMode);
    r.register_fn("HSM.INPUTDET", |_| InputDeviceDetect);
}

/// Ticks per second of the timers (`.rdata` 0x00f9f604 = 3e6).
const TICKS: f64 = 3.0e6;

/// "ActivateSpotting" (`ACTIVATESPOT`, factory 0x77f080, vtable 0xfdb064, 0x48 B): enter (0x73afa0) sets byte +0x98
/// of the spotting object G+0x2f434 (the state `FUN_0063e2d0` clears); exit (0x73afd0 → `FUN_0063e2d0`) clears it
/// again, zeroing 0x90 bytes from +4 and storing the `.rdata` float 0xfb5f94 at +0x94. The update (0x73afc0) is 0:
/// it runs its children.
pub struct ActivateSpotting;

impl Behaviour for ActivateSpotting {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ActivateSpotting { on: true });
    }
    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ActivateSpotting { on: false });
    }
}

/// "Cache Players Score" (`CACHESCORE`, factory 0x77e860, vtable 0xfdace0, 0x48 B): enter (0x73b2b0) stores the
/// local player's score into player +0x41c ([`Request::CachePlayerScore`]: the local player `FUN_006bf330` on
/// G+0x2f3dc, then `FUN_006b9df0`: online player +0x3f8, else services +0x4b0c + player +0x3fc); the update
/// (0x73b2e0) exits through `EXIT`.
pub struct CachePlayersScore;

impl Behaviour for CachePlayersScore {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::CachePlayerScore);
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Exit When Game Mode Time Has Passed" (`EXITWHENGAME`, factory 0x77ea30, vtable 0xfdad94, 0x50 B): enter
/// (0x755ba0) keeps the `TIME` parameter (`atof`) times 3e6 as an i64 tick count at +0x48 (and registers its debug
/// variable `HSMBehaviourExitWhenGameModeTimeHasPassed`, not ported); the update (0x74b8e0) exits through `EXIT`
/// when the game-mode clock runs (GMM+0x2e74, [`Query::GameModeClockRunning`]) and the ticks minus the elapsed
/// ticks (GMM+0x2d28, [`Query::GameModeElapsedTicks`]) are at most 0 (the constant compared with is the `fldz`
/// before its `_ftol`). The exit (0x73c660) only unregisters the debug variable.
#[derive(Default)]
pub struct ExitWhenGameModeTime {
    ticks: i64,
}

impl Behaviour for ExitWhenGameModeTime {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.ticks = (crate::hsm::atof(&cx.param_text("TIME")) * TICKS) as i64;
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::GameModeClockRunning) {
            return false;
        }
        let elapsed = ask(cx, Query::GameModeElapsedTicks) as i64;
        if self.ticks.wrapping_sub(elapsed) <= 0 {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Accept Invite" (`ACCT_INVITE`, factory 0x77d440, vtable 0xfda3b4, 0x50 B): enter (0x753b50) zeroes the state
/// (+0x48) and notes whether the invite listener (player manager +0xc9a0) is processing an invite (+0x4c, the
/// debug variable `ProcessingInvite`; the registration of `HSMBehaviourAcceptInvite`'s variables is not ported).
/// The exit update (0x73af10) runs the state: 0 = if an invite is pending ([`Query::InvitePending`], the
/// listener's vtable +0) accept it ([`Request::AcceptInvite`], `FUN_006bad40`); then 1 = wait while the listener
/// still has one; 2 = finished (unregisters the debug variable). The update is 0: it runs its children.
#[derive(Default)]
pub struct AcceptInvite {
    state: u8,
}

impl Behaviour for AcceptInvite {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.state = 0;
    }
    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        match self.state {
            0 | 1 => {
                if self.state == 0 && ask_bool(cx, Query::InvitePending) {
                    cx.host.request(Request::AcceptInvite);
                }
                self.state = 1;
                if ask_bool(cx, Query::InvitePending) {
                    return false;
                }
                self.state = 2;
                true
            }
            _ => {
                self.state = 2;
                true
            }
        }
    }
}

/// "Swap Game Mode" (`SWAPGAMEMODE`, factory 0x77efe0, vtable 0xfdb01c, 0x48 B): the update (0x746100) reads the
/// binding path `EventBinding`; when it is not empty its integer value, and when that is 0 (or the path is empty)
/// the `Event` parameter (`atoi`), as the new game mode: `FUN_006e74f0`(GMM, id, 1) ([`Request::SwapGameMode`]:
/// GMM+0x2ce4 = id, +0x2e7e = id ≠ 0, +0x2e80 = id ≠ 0, and the lookup of the mode's record in the mode table).
/// Then exits through `EXIT`.
pub struct SwapGameMode;

impl Behaviour for SwapGameMode {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let path = cx.param_text("EventBinding");
        let mut mode = if path.is_empty() { 0 } else { cx.host.binding_int(&path).unwrap_or(0) };
        if mode == 0 {
            mode = param_int(cx, "Event");
        }
        cx.host.request(Request::SwapGameMode { mode: mode as i32 });
        cx.set_exit(EXIT);
        true
    }
}

/// "Write Show Summary Screen Session Variable" (`WRT_PR_SESSN`, factory 0x77eef0, vtable 0xfdafb0, 0x48 B): enter
/// (0x7704b0) writes the Autolog session variable `NAME` (G+0x2ed74; nothing when the name is empty). The kind
/// (0x76fcd0): `IS_BOOL` true → boolean, else `IS_REF` true → integer, else boolean. Boolean: `BOOL_VALUE` equals
/// "true" without case (`FUN_0083b5f0`, [`Request::WriteAutologBool`]); integer: `atoi(REF_VALUE)` (`FUN_0083b610`,
/// [`Request::WriteAutologInt`]). The update (0x76efb0) exits through `EXIT`.
pub struct WriteSessionVariable;

impl Behaviour for WriteSessionVariable {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("NAME");
        if name.is_empty() {
            return;
        }
        let integer = !(cx.param("IS_BOOL").is_some() && param_true(cx, "IS_BOOL"))
            && cx.param("IS_REF").is_some()
            && param_true(cx, "IS_REF");
        if integer {
            let value = param_int(cx, "REF_VALUE");
            cx.host.request(Request::WriteAutologInt { name, value });
        } else {
            let value = param_true(cx, "BOOL_VALUE");
            cx.host.request(Request::WriteAutologBool { name, value });
        }
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Sync Online Results Start" (`SYNCRESSTART`, factory 0x77dec0, vtable 0xfda8a8, 0x50 B): enter (0x746210) sets
/// the deadline (+0x48, an i64 of clock ticks): with the game-mode delay GMM+0x2d20 set ([`Query::ResultsSyncDelay`],
/// not i64::MIN) and `1.5e6 − now + delay < 1.5e7` (`.rdata` 0xfb8660 / 0xfb2894, `_ftol`) it is `1.5e6 + delay`,
/// else `1.5e6 + now` (`now` = the clock `FUN_0046ee10` +8, [`Query::ClockTicks`]). The update (0x741050) exits
/// through `EXIT` once the deadline is before now.
#[derive(Default)]
pub struct SyncOnlineResultsStart {
    deadline: i64,
}

impl Behaviour for SyncOnlineResultsStart {
    fn on_enter(&mut self, cx: &mut Cx) {
        const SHORT: i64 = 1_500_000;
        const LONG: i64 = 15_000_000;
        let now = ask(cx, Query::ClockTicks) as i64;
        let delay = ask(cx, Query::ResultsSyncDelay) as i64;
        self.deadline = if delay != i64::MIN && SHORT.wrapping_sub(now).wrapping_add(delay) < LONG {
            SHORT.wrapping_add(delay)
        } else {
            SHORT.wrapping_add(now)
        };
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        let now = ask(cx, Query::ClockTicks) as i64;
        if self.deadline < now {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "End ToS / EA Account Screen" (`ENDSETTINGSS`, factory 0x77cd70, vtable 0xfda020): only the exit (0x73c5c0)
/// does anything, `FUN_0071c1c0` on the services G+0x2f454: with +0x4968 or the flag +0x14e0 of G+0x2f3e0 set it
/// sets +0x4940 ([`Request::EndSettingsScreen`]). The `SCREEN` parameter (`TOS`, `EA_ACCOUNT`) is not read.
pub struct EndSettingsScreen;

impl Behaviour for EndSettingsScreen {
    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::EndSettingsScreen);
    }
}

/// "Should Show Car Select" (`SHOULDSHOWCA`, factory 0x77cc30, vtable 0xfd9f90): the update (0x7407c0) exits
/// through `FALSE` in story mode (GMM+0x2e7a, [`Query::StoryMode`]) and `TRUE` otherwise (the keys at 0x117a960 /
/// 0x117aac0 read from snapshot 1).
pub struct ShouldShowCarSelect;

impl Behaviour for ShouldShowCarSelect {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let story = ask_bool(cx, Query::StoryMode);
        cx.set_exit(key(if story { "FALSE" } else { "TRUE" }));
        true
    }
}

/// "HSM.RunChallenge" (`RUNCHALL`, factory 0x77e400, vtable 0xfdaae8, 0x50 B): enter (0x74fc30) stores the
/// challenge object G+0x2f428 and starts it on the current event record (`FUN_00789240`, the table pointer and
/// offset GMM+4 +0xf4 / +0xf8: [`Request::StartChallenge`]); it runs (update 0 = children) until its parent
/// exits, then the exit (0x73ed30) stops it (`FUN_00785d90`, [`Request::StopChallenge`]) and clears +0x48.
pub struct RunChallenge;

impl Behaviour for RunChallenge {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StartChallenge);
    }
    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StopChallenge);
    }
}

/// "Rollout Achievements" (`ROLLOUTACHVM`, factory 0x77e720, vtable 0xfdac50): enter is the empty stub 0x73ba00,
/// the update (0x73ba10) only stores the `.bss` key at 0x117a970 (= `EXIT`, read from snapshot 1) as the exit.
pub struct RolloutAchievement;

impl Behaviour for RolloutAchievement {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Car Colour Correct" (`CARCOLCORRCT`, factory 0x77e450, vtable 0xfdab0c): only the update (0x73c5e0) is not a
/// default stub; it stores the key at 0x117a970 (= `EXIT`) as the exit.
pub struct CarColourCorrect;

impl Behaviour for CarColourCorrect {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "HSM.StartPhotoMode" (`HSM.STARTPHO`, factory 0x77b150, vtable 0xfd935c): enter (0x740f20) calls `FUN_0083e710`
/// on the photo-mode object G+0x2f474 ([`Request::StartPhotoMode`]); the update (0x740f40) exits through the key
/// at 0x117a970 (= `EXIT`) in the same frame.
pub struct StartPhotoMode;

impl Behaviour for StartPhotoMode {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StartPhotoMode);
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "HSM.InputDeviceDetect" (`HSM.INPUTDET`, factory 0x77e1d0, vtable 0xfda9ec): the update (0x73d0b0) takes the
/// local player (`FUN_006bf330` on G+0x2f3dc), the input-device record of its +0x254 (`FUN_0047aa20` on the input
/// system 0x10edad8) and exits through the key at 0x117a018 (= `EXIT`) once that record's byte +0x499 is set
/// ([`Query::InputDeviceDetected`]); until then it keeps running.
pub struct InputDeviceDetect;

impl Behaviour for InputDeviceDetect {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::InputDeviceDetected) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}
