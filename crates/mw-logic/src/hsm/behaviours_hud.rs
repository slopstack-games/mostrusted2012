//! More behaviour types of the free-drive machines and their HUD (`GAMELOGIC/GAMEMODES.BNDL`): the horn timer, the
//! system-menu wait, the pursuit watchers, the world / stats-timer switches, the easy-guide host and three one-shot
//! setters. Engine effects are [`Request`]s, engine state [`Query`]s (both name the original's code).

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("HORN_PRESS", |_| HornPress::default());
    r.register_fn("WAITSYSMENU", |_| WaitSystemMenu);
    r.register_fn("COSTSTATESC", |_| CostToStateScoring::default());
    r.register_fn("WAITFORCHASE", |_| WaitForPursuit::default());
    r.register_fn("DISABLETIME", |_| DisableStatsTimer);
    r.register_fn("DISABLEWORLD", |_| DisableWorld::default());
    r.register_fn("HOSTEZGUIDE", |_| HostEasyGuide);
    r.register_fn("CLEARUIPOSTF", |_| ClearUiPostFx);
    r.register_fn("SETPENDINGON", |_| SetPendingDialog);
    r.register_fn("OVERRIDEPAGE", |_| OverridePage);
}

/// "Wait for Sound Horn" (`HORN_PRESS`, factory 0x77f7e0, vtable 0xfdb564, 0x80 B): enter (0x741760) clears the
/// timer (+0x78 f32) and +0x7c; update (0x753530): while +0x7c is 0 (nothing sets it) the step 0x746b70 runs, then
/// 0 (never exits; the `HORN_SOUND` parameter is not read). The step: with the horn input not paused
/// ([`Query::HornPaused`], `FUN_0083c700`) a timer >= 0 counts down by the game clock (`FUN_0046ee10`: step ×
/// rate); then, when the local player's input record is not state 1, the camera's byte +0x36dd is clear and the
/// record's +0x58 is 2 ([`Query::HornDown`]) and the timer is <= 0, it is set to 1.0 (`.rdata` 0xf8a820). The
/// timer has no reader in the machine (the horn sound itself is played by the engine).
#[derive(Default)]
pub struct HornPress {
    timer: f32,
}

impl Behaviour for HornPress {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.timer = 0.0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::HornPaused) {
            return false;
        }
        if self.timer >= 0.0 {
            self.timer -= cx.clock.game.unwrap_or(0.0);
        }
        if ask_bool(cx, Query::HornDown) && self.timer <= 0.0 {
            self.timer = 1.0;
        }
        false
    }
}

/// "Wait for System Menu Active" (`WAITSYSMENU`, factory 0x77a0e0, vtable 0xfd9008, 0x48 B): update (0x747080)
/// exits through `EXIT` when the system menu flag (byte 0x011a2471, [`Query::SystemMenuActive`]) equals the
/// `ACTIVE` parameter read as "true".
pub struct WaitSystemMenu;

impl Behaviour for WaitSystemMenu {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::SystemMenuActive) == param_true(cx, "ACTIVE") {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// Seconds between two prunes of the cost-to-state list (`.rdata` 0xfd2120 = 5.0).
const COST_PRUNE_PERIOD: f32 = 5.0;

/// "Do Cost to State Scoring" (`COSTSTATESC`, factory 0x77db20, vtable 0xfda704, 0x110 B): enter (0x742cf0) sets
/// the list count (+0x108) to 0 and the prune timer (+0x10c) to 5.0. The update (0x7548e0, never exits) runs
/// only while the local player has a vehicle and the game clock runs ([`Query::CostStateActive`]: the player
/// slot (+0xa0) is valid, its object-manager entry (+0x55c48) is > -1, the clock's running byte +0x1c): the
/// timer counts down by the game clock, at <= 0 the list is pruned (0x74a580 drops the u16 ids at +0x48.. that
/// `FUN_0090cbd0` (G+0x53200) no longer knows: [`Request::CostStatePrune`]) and the timer restarts; then the
/// scan of the event's cost records (0x7548e0 from 0x754989 on: a walk over the game-mode records of four
/// kinds) is the engine's work ([`Request::CostStateScan`]).
/// **Model decision:** the scan body (~1.5 KB: the record walk and the cost-to-state scoring) is not ported.
#[derive(Default)]
pub struct CostToStateScoring {
    timer: f32,
}

impl Behaviour for CostToStateScoring {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.timer = COST_PRUNE_PERIOD;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::CostStateActive) {
            return false;
        }
        self.timer -= cx.clock.game.unwrap_or(0.0);
        if self.timer <= 0.0 {
            cx.host.request(Request::CostStatePrune);
            self.timer = COST_PRUNE_PERIOD;
        }
        cx.host.request(Request::CostStateScan);
        false
    }
}

/// "Wait for Pursuit End" (`WAITFORCHASE`, factory 0x77da80, vtable 0xfda6bc, 0x50 B): enter (0x741710) sets
/// +0x48 ("pursuit not seen yet"); update (0x7469c0) reads the local player's pursuit state (record
/// `G+0x2f410` + slot `G+0x2f3dc`+0xa6b4 × 0x8b0, +0x89c, [`Query::LocalPursuitState`]): while +0x48 is set and
/// the state is 0 it waits; any other state clears +0x48. Then 1 and 3 keep waiting, 2 exits through `BUSTED`
/// and anything else (0: the pursuit has gone) through `ESCAPED` (exit keys read from 0x117a000 / 0x117a4d0).
#[derive(Default)]
pub struct WaitForPursuit {
    waiting_for_start: bool,
}

impl Behaviour for WaitForPursuit {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.waiting_for_start = true;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let state = ask(cx, Query::LocalPursuitState);
        if self.waiting_for_start {
            if state == 0 {
                return false;
            }
            self.waiting_for_start = false;
        }
        match state {
            1 | 3 => false,
            2 => {
                cx.set_exit(key("BUSTED"));
                true
            }
            _ => {
                cx.set_exit(key("ESCAPED"));
                true
            }
        }
    }
}

/// "Disable Game Stats Timer" (`DISABLETIME`, factory 0x77f2c0, vtable 0xfdb160, 0x48 B): enter (0x73c4f0)
/// decrements the counter G+0x25624 (the stats timer runs while it is 0), exit (0x73c500) increments it;
/// runs its children.
pub struct DisableStatsTimer;

impl Behaviour for DisableStatsTimer {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::DisableStatsTimer { on: true });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::DisableStatsTimer { on: false });
    }
}

/// "Disable World Rendering" (`DISABLEWORLD`, factory 0x77b3d0, vtable 0xfd947c, 0x50 B): enter (0x742800) saves
/// the world-rendering byte (G+0x2efb4, [`Query::WorldRendering`]) into +0x48 (forced 1 when the `ENABLEONEXIT`
/// parameter exists and reads "true"), clears the byte, and with `DISABLEAUDIO` "true" (+0x49) also disables the
/// audio (`FUN_004b8b20`(`*0x010edb4c`, 1)); exit (0x73be50) restores the byte from +0x48 and re-enables the audio
/// when it was disabled. Runs its children.
#[derive(Default)]
pub struct DisableWorld {
    restore: bool,
    audio: bool,
}

impl Behaviour for DisableWorld {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.restore = ask_bool(cx, Query::WorldRendering) || param_true(cx, "ENABLEONEXIT");
        cx.host.request(Request::SetWorldRendering { enabled: false });
        self.audio = param_true(cx, "DISABLEAUDIO");
        if self.audio {
            cx.host.request(Request::DisableAudio { on: true });
        }
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SetWorldRendering { enabled: self.restore });
        if self.audio {
            cx.host.request(Request::DisableAudio { on: false });
        }
    }
}

/// "HSM.HostEasyGuide" (`HOSTEZGUIDE`, factory 0x77ccd0, vtable 0xfd9fd8, 0x50 B): the easy guide object (UI
/// manager G+0x2f480 +0x47640). Enter (0x74bb20) clears its field +0xac, sets its requested byte +0x58 and
/// shows it (`FUN_007997f0`: [`Request::HostEasyGuide`]); the enter update (0x73c8a0) waits for requested and
/// state +0x54 == 2 ([`Query::EasyGuideShown`]); exit begin (0x73c8c0) clears +0x58 and +0xf0; the exit update
/// (0x73c8d0) waits for +0x58 == 0 and state == 0 ([`Query::EasyGuideClosed`]). Runs its children.
pub struct HostEasyGuide;

impl Behaviour for HostEasyGuide {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::HostEasyGuide);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::EasyGuideShown)
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ReleaseEasyGuide);
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::EasyGuideClosed)
    }
}

/// "Clear UI Post Fx" (`CLEARUIPOSTF`, factory 0x77cfb0, vtable 0xfda160, 0x48 B): update (0x73bb30) clears the
/// UI post-effect field (`FUN_007746a0`'s object +0x2e6a0 = 0) and exits through `EXIT` at once.
pub struct ClearUiPostFx;

impl Behaviour for ClearUiPostFx {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::ClearUiPostFx);
        cx.set_exit(EXIT);
        true
    }
}

/// "Set Pending ..." (`SETPENDINGON`, factory 0x77e990, vtable 0xfdad4c, 0x48 B): update (0x743720) stores
/// `atoi(Dialog)` in G+0x2f70c ([`Request::SetPendingDialog`]) and exits through `EXIT`.
pub struct SetPendingDialog;

impl Behaviour for SetPendingDialog {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let value = param_int(cx, "Dialog") as i32;
        cx.host.request(Request::SetPendingDialog { value });
        cx.set_exit(EXIT);
        true
    }
}

/// "Override PageRunner Page" (`OVERRIDEPAGE`, factory 0x77ec20, vtable 0xfdae6c, 0x48 B): enter (0x76f750) hands
/// `CONTEXT_NAME` and `PAGE_NAME` to the page runner (`FUN_0083d6f0` on G+0x2ed74: the context's start page is
/// replaced); update (0x76e8c0) exits through `EXIT` at once.
pub struct OverridePage;

impl Behaviour for OverridePage {
    fn on_enter(&mut self, cx: &mut Cx) {
        let context = cx.param_text("CONTEXT_NAME");
        let page = cx.param_text("PAGE_NAME");
        cx.host.request(Request::OverridePage { context, page });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}
