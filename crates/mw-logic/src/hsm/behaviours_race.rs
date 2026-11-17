//! Behaviour types of the race game mode's machine (`GAMELOGIC/GAMEMODES.BNDL`, "Race Game Mode" 730003), in the
//! order the free roam → race route enters them (docs/decomp.md "Route free roam → race"). Engine effects are
//! [`Request`]s, engine state [`Query`]s; each doc comment names the original's function.

use super::{Query, ask, ask_bool, param_int, param_true};
use mw_formats::hsm::key;

use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("PAUSESIM", |_| PauseSim);
    r.register_fn("ENABLEALERTS", |_| EnableAlerts);
    r.register_fn("CLRSELEVENT", |_| ClearSelectedEvent);
    r.register_fn("HSB_NOSCORES", |_| NoScores);
    r.register_fn("CRASHESCAPE", |_| CrashEscape::default());
    r.register_fn("CHECK_ENT", |_| CheckEntitlement);
    r.register_fn("HSB_TOG_LS", |_| ToggleLoadingScreen::default());
}

/// "GameLogic.HSMBehaviourPauseSim" (`PAUSESIM`, factory 0x77d990, vtable 0xfda650, 0x48 B): offline
/// (`FUN_006dfed0` false), pauses the simulation on enter (0x73e590 → `FUN_00845c60` on `FUN_007746e0`'s object) and
/// resumes it when it is exited (slot 6, 0x73e5c0 → `FUN_00845ce0`); runs its children, never exits itself. The
/// same pause as `BPAUSED`, on other slots.
pub struct PauseSim;

impl Behaviour for PauseSim {
    fn on_enter(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::OnlineEvent) {
            cx.host.request(Request::SimPause { paused: true });
        }
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::OnlineEvent) {
            cx.host.request(Request::SimPause { paused: false });
        }
    }
}

/// "EnableAutologAlertPolling" (`ENABLEALERTS`, factory 0x77dc60, vtable 0xfda7a4, 0x48 B): every slot but the shared
/// tick is the base's no-op, so the state does nothing itself; the Autolog alert poll is switched by the state
/// being active (the poller is not in the machine code; open: its reader). Runs its children.
pub struct EnableAlerts;

impl Behaviour for EnableAlerts {}

/// "Clear Selected Event From Map" (`CLRSELEVENT`, factory 0x77b6d0, vtable 0xfd95c0, 0x48 B; update 0x73baa0):
/// on the UI object G+0x2f480 clears the map's selected event (+0x2c8e4 = 0, byte +0x2ce33 = 0, `FUN_00792390` on
/// +0x28190, +0x2c5d0 = 0.0, +0x2c5e0 = −1, `FUN_00792390` on +0x1f8f0, +0x23d30 = 0.0, `FUN_00792390` on
/// +0x23d40, +0x28180 = 0.0) and exits through `EXIT` at once ([`Request::ClearSelectedEvent`]).
pub struct ClearSelectedEvent;

impl Behaviour for ClearSelectedEvent {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::ClearSelectedEvent);
        cx.set_exit(EXIT);
        true
    }
}

/// "Disable Scoring" (`HSB_NOSCORES`, factory 0x77b380, vtable 0xfd9458, 0x48 B): raises the scoring manager's
/// (G+0x2f3cc) counter +0x124 on enter (0x73be10 → `FUN_006a0c10`, `inc`) and lowers it on exit (0x73be30 →
/// `FUN_006a0c20`, `dec`); runs its children ([`Request::AdjustScoringDisabled`]).
pub struct NoScores;

impl Behaviour for NoScores {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdjustScoringDisabled { delta: 1 });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdjustScoringDisabled { delta: -1 });
    }
}

/// `.rdata` 0xfd2010: the crash-escape check's period, 0.2 s.
pub const CRASH_ESCAPE_PERIOD: f32 = 0.2;
/// `.rdata` 0xfd201c: a player's cool-down must be below this (4.0 s) to be checked again; 0xfd2120: what it is set
/// to when the escape is scored (5.0).
pub const CRASH_ESCAPE_ARMED: f32 = 4.0;
pub const CRASH_ESCAPE_COOLDOWN: f32 = 5.0;

/// "HSM.Crash Escapes" (`CRASHESCAPE`, factory 0x77b290, vtable 0xfd93ec, 0xb0 B): scores escaping from a crash.
/// Enter (0x742750) zeroes the timer (+0x48) and the 24 per-player timers (+0x4c). The update (0x748150), only
/// while the game clock runs (`FUN_0046ee10` +0x1c; step = +0x10 × +0x14), counts the timer down by the step and
/// every 0.2 s (then reset to 0.2) looks at the other players when the local player's vehicle is fine: a player
/// whose vehicle is gone, or is not wrecked (+0x23f clear), or was wrecked by the local player (+0x59c = the
/// local id) has its timer zeroed; a wrecked player whose timer is below 4.0 and who is within 15 m and ahead of the
/// local vehicle (dot >= 0.1) scores the escape (`Request::CrashEscape`, the timer set to 5.0), and every wrecked
/// player's timer then grows by the step. Never exits.
pub struct CrashEscape {
    timer: f32,
    cooldown: [f32; 24],
}

impl Default for CrashEscape {
    fn default() -> Self {
        CrashEscape { timer: 0.0, cooldown: [0.0; 24] }
    }
}

impl Behaviour for CrashEscape {
    fn on_enter(&mut self, _cx: &mut Cx) {
        *self = CrashEscape::default();
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let Some(step) = cx.clock.game else { return false };
        self.timer -= step;
        if self.timer > 0.0 {
            return false;
        }
        self.timer = CRASH_ESCAPE_PERIOD;
        if !ask_bool(cx, Query::CrashLocalOk) {
            return false;
        }
        let count = ask(cx, Query::CrashPlayerCount) as u32;
        for index in 0..count {
            let state = ask(cx, Query::CrashPlayerState { index });
            let id = (state >> 8) as usize;
            let (exists, wrecked, by_local) = (state & 1 != 0, state & 2 != 0, state & 4 != 0);
            if id >= self.cooldown.len() {
                continue;
            }
            if !exists || !wrecked || by_local {
                self.cooldown[id] = 0.0;
                continue;
            }
            if self.cooldown[id] < CRASH_ESCAPE_ARMED {
                if ask_bool(cx, Query::CrashPlayerNear { index }) {
                    cx.host.request(Request::CrashEscape { player: id as u32 });
                    self.cooldown[id] = CRASH_ESCAPE_COOLDOWN;
                }
                self.cooldown[id] += step;
            }
        }
        false
    }
}

/// "Check for Entitlement" (`CHECK_ENT`, factory 0x77d300, vtable 0xfda318, 0x48 B): the update (0x73ba30) exits
/// through `HAVE` (`DAT_0117a148`) at once, whatever `ENTITLEMENT` says (the parameter is not read by any slot of
/// the type: the online pass check, "The Screen" 1867357, has the entitlement on this build).
pub struct CheckEntitlement;

impl Behaviour for CheckEntitlement {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(key("HAVE"));
        true
    }
}

/// "Toggle Loading Screen" (`HSB_TOG_LS`, factory 0x77b970, vtable 0xfd96f8, 0x50 B): a counter (+0x48) is zeroed
/// on enter (0x741320), counts the enter updates (0x741330, always done) and the exit updates (0x741350: done once
/// it is above 0). The update (0x746650): `SHOW` (`stricmp` with "true") shows the loading screen with `LAYOUT`
/// (`atoi`; the UI manager's loading-screen slot +0x2d1b0, `FUN_0078f860`(5, layout), or `FUN_00799a60`(5) with no
/// layout), anything else hides it (UI manager +0x2d204 &= ~0x20, +0x2d21c = 0); then exits through `EXIT`
/// ([`Request::ToggleLoadingScreen`]).
#[derive(Default)]
pub struct ToggleLoadingScreen {
    count: i32,
}

impl Behaviour for ToggleLoadingScreen {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.count = 0;
    }

    fn enter_update(&mut self, _cx: &mut Cx) -> bool {
        self.count += 1;
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let show = param_true(cx, "SHOW");
        let layout = if show { param_int(cx, "LAYOUT") } else { 0 };
        cx.host.request(Request::ToggleLoadingScreen { show, layout });
        cx.set_exit(EXIT);
        true
    }

    fn exit_update(&mut self, _cx: &mut Cx) -> bool {
        let done = self.count > 0;
        self.count += 1;
        done
    }
}
