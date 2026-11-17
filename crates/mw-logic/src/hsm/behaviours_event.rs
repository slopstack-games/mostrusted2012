//! Behaviour types the game-mode machines run to end, check and set up an event: the end reason, the end check,
//! the wait for input, the layer set, the blacklist shutdowns, the checkpoints, the event starts and the car-swap
//! logic. The big ones (`CHECKPTS`, `EVENTSTARTS`, `HSMCARSWAPLO`) keep their lifecycle here and their per-frame
//! work in the logic's managers (`natives::checkpoints`, `natives::event_starts`, `natives::car_swap`), which the
//! host runs on the behaviours' requests; the steps of `DOBLACKLISTS` are still a [`Request`] the runtime answers.

use mw_formats::hsm::key;
use mw_formats::json::Json;

use super::{Query, ask, ask_bool, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Key, Registry};
use crate::natives::Request;
use crate::natives::event_starts::EventStartParams;

pub fn register(r: &mut Registry) {
    r.register_fn("SETENDREASON", |_| SetEndReason);
    r.register_fn("MODEENDCHECK", |_| ModeEndCheck);
    r.register_fn("HSB_WAIT4IN", |_| WaitForInput::default());
    r.register_fn("SET_LAY_SET", |_| SetLayerSet::default());
    r.register_fn("DOBLACKLISTS", |_| DoBlacklistShutdowns::default());
    r.register_fn("CHECKPTS", |_| Checkpoints);
    r.register_fn("EVENTSTARTS", |_| EventStarts::default());
    r.register_fn("HSMCARSWAPLO", |_| CarSwapLogic);
}

/// "Set End Reason" (`SETENDREASON`, factory 0x77b100, vtable 0xfd9338, 0x48 B; update 0x7405e0): by the entry point
/// the state was entered through, `ENDED` (`DAT_0117a2d8`), `RETRY` (`DAT_0117a668`) or any other, sets the end
/// reason GMM+0x2d08 to 1 / 3 / 2 and clears GMM+0x2e78 ("won"; for `ENDED` only when `FUN_006f46e0` says the mode
/// was not won), then exits through `EXIT`.
pub struct SetEndReason;

impl Behaviour for SetEndReason {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let entry = cx.entry_point();
        let reason = if entry == key("ENDED") {
            if !ask_bool(cx, Query::ModeWon) {
                cx.host.request(Request::SetGameModeWon { won: false });
            }
            1
        } else if entry == key("RETRY") {
            cx.host.request(Request::SetGameModeWon { won: false });
            3
        } else {
            cx.host.request(Request::SetGameModeWon { won: false });
            2
        };
        cx.host.request(Request::SetEndReason { reason });
        cx.set_exit(EXIT);
        true
    }
}

/// "Mode End Check" (`MODEENDCHECK`, factory 0x77c050, vtable 0xfd9a28, 0x48 B; update 0x757af0): only the session's
/// authority (`FUN_006dfee0`) decides. Unless the limit is reached (`FUN_006f1530`) or the mode is not won
/// (`FUN_006f46e0`) or the session is in state 4 (`FUN_004c1f30`), an end trigger of the event must have fired
/// (`FUN_0074e790`, which also sets "won"). Then nothing happens while an end reason is set (GMM+0x2d08). Offline:
/// exits through `EXIT`, or through `TIME_OUT` when the display timeout (GMM+0x2f10) has run out
/// (`FUN_006e27e0`); online: posts the end message (`FUN_006f4680`) and exits through `EXIT` once GMM+0x2e75 is set.
pub struct ModeEndCheck;

impl Behaviour for ModeEndCheck {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::ModeAuthority) {
            return false;
        }
        let skip = ask_bool(cx, Query::ModeLimitReached)
            || !ask_bool(cx, Query::ModeWon)
            || ask_bool(cx, Query::SessionRaceState);
        if !skip {
            match ask(cx, Query::EventEndTrigger) {
                0 => return false,
                1 => cx.host.request(Request::SetGameModeWon { won: true }),
                _ => cx.host.request(Request::SetGameModeWon { won: false }),
            }
        }
        if ask(cx, Query::EndReason) as u16 != 0 {
            return false;
        }
        if !ask_bool(cx, Query::OnlineEvent) {
            let timed_out = ask_bool(cx, Query::DisplayTimeoutOn) && (ask(cx, Query::DisplayTimeLeft) as i64) <= 0;
            cx.set_exit(if timed_out { key("TIME_OUT") } else { EXIT });
            return true;
        }
        cx.host.request(Request::SendModeEndMessage);
        if ask_bool(cx, Query::ModeEndFlag) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Wait For Input" (`HSB_WAIT4IN`, factory 0x77afc0, vtable 0xfd92a8, 0x50 B): on enter (0x741b50) waits for the
/// input to be released (+0x48 = 1; +0x4c = 0.25, `.rdata` 0xfd6688, which no slot reads again). The update
/// (0x753540) first waits until none of the local player's input actions is held (`FUN_007470e0`), then exits
/// through the first candidate exit point the parent handles (`FUN_00774790` / `FUN_00774dc0`), the candidates
/// being the input actions of [`Query::InputExit`]; runs its children meanwhile.
#[derive(Default)]
pub struct WaitForInput {
    waiting: bool,
}

impl Behaviour for WaitForInput {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.waiting = true;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.waiting {
            if !ask_bool(cx, Query::InputHeld) {
                self.waiting = false;
            }
            return false;
        }
        let autorepeat = param_true(cx, "AUTOREPEAT");
        for index in 0..32 {
            let exit = ask(cx, Query::InputExit { index, autorepeat });
            if exit == 0 {
                break;
            }
            if cx.parent_handles(exit) {
                cx.set_exit(exit);
                return true;
            }
        }
        false
    }
}

/// "Set Layout Set" (`SET_LAY_SET`, factory 0x77c290, vtable 0xfd9b24, 0x50 B): the UI manager's layer set
/// ((*(G+0x2f480))+0x31c08). On enter (0x7518e0) keeps the old one (+0x48) and sets the new one unless it is 0; the
/// update (0x751920) sets it again every frame when `FORCE_SET` is "true"; on exit (0x745c20) restores the old one
/// unless `RESTORE_OLD` is given and not "true". The new one (`FUN_00745c80`): the integer binding `LAYOUTPATH` when
/// it is not empty, else `LAYOUTSET` as `atoi`, not below 0.
#[derive(Default)]
pub struct SetLayerSet {
    old: u32,
}

impl SetLayerSet {
    fn value(cx: &mut Cx) -> u32 {
        let path = cx.param_text("LAYOUTPATH");
        if path.is_empty() {
            super::param_int(cx, "LAYOUTSET").max(0) as u32
        } else {
            cx.host.binding_int(&path).unwrap_or(0) as u32
        }
    }
}

impl Behaviour for SetLayerSet {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.old = ask(cx, Query::LayerSet) as u32;
        let set = Self::value(cx);
        if set != 0 {
            cx.host.request(Request::SetLayerSet { set });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.param("FORCE_SET").is_some() && param_true(cx, "FORCE_SET") {
            let set = Self::value(cx);
            cx.host.request(Request::SetLayerSet { set });
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if cx.param("RESTORE_OLD").is_none() || param_true(cx, "RESTORE_OLD") {
            cx.host.request(Request::SetLayerSet { set: self.old });
        }
    }
}

/// The distance (squared) beyond which a shut-down rival is removed (`.rdata` 0xfd2108 = 90000, 300 m).
pub const BLACKLIST_REMOVE_DISTANCE_SQ: f32 = 90_000.0;
/// "Do Blacklist Shutdowns" (`DOBLACKLISTS`, factory 0x77e0a0, vtable 0xfda980, 0x1a8 B): a 7-phase machine that
/// shuts down a blacklist rival. Enter (0x753dd0): phase 0, no rival, GMM+0x2f26 (credits transition) cleared, and
/// the view model `BlacklistShutDown` ("HSMBehaviourDoBlacklistShutdowns": `IsSpotted` +0xa0, `LastShutdownName`
/// +0x88) registered; exit (0x73bfc0) unregisters it and removes the rival's slot (`FUN_006ce180`(id, 3)); the exit
/// update (0x73bfa0) waits while the mode (+0x4c) is 2 or 3. Update (0x754290), by phase: 0 goes to 1 unless the
/// mode is 2 / 3 (then returns at once); 1, 4, 5 run engine steps (`FUN_00748e50`, `FUN_007428b0`, `FUN_00753fa0`);
/// 2 goes to 3 when the rival is alive (or, without one, `FUN_00748a40`); 3 releases the rival's handle
/// (`FUN_00843030`(h, 3, 0)) and goes to 4, or without a rival goes to 1; 6 removes the rival once farther than
/// [`BLACKLIST_REMOVE_DISTANCE_SQ`] and goes to 1. With the credits flag (+0xa4) set: while the general sequence
/// runs (`FUN_0066e0f0`) and its trigger `EXIT_TO_CREDITS` is not set, +0xa5 is set; with +0xa5, GMM+0x2f26 is set,
/// the telemetry event "GAME CRED FNSH" is posted and the state exits through `CREDITS`.
/// Model decisions: the steps' effect on the phase and the +0xa4 / rival fields is read through queries (the steps
/// are engine code, not ported); the view model is published with its defaults (`IsSpotted` false, no name).
#[derive(Default)]
pub struct DoBlacklistShutdowns {
    phase: u32,
    credits_seen: bool,
}

impl Behaviour for DoBlacklistShutdowns {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.phase = 0;
        self.credits_seen = false;
        cx.host.request(Request::SetCreditsTransition { on: false });
        let data = Json::Object(vec![
            ("IsSpotted".into(), Json::Bool(false)),
            ("LastShutdownName".into(), Json::Str(String::new())),
        ]);
        cx.host.publish("BlacklistShutDown", &data);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let mode = ask(cx, Query::BlacklistMode);
        let slot = ask(cx, Query::BlacklistSlot) != 0;
        let mut distance = f32::MAX;
        if slot && ask_bool(cx, Query::BlacklistRivalAlive) {
            distance = f32::from_bits(ask(cx, Query::BlacklistRivalDistance) as u32);
        }
        match self.phase {
            0 => {
                if mode == 2 || mode == 3 {
                    return false;
                }
                self.phase = 1;
            }
            1 | 4 | 5 => {
                cx.host.request(Request::BlacklistStep { phase: self.phase });
                self.after_step(cx);
            }
            2 => {
                let go =
                    if slot { ask_bool(cx, Query::BlacklistRivalAlive) } else { ask_bool(cx, Query::BlacklistReady) };
                if go {
                    self.phase = 3;
                }
            }
            3 => {
                if slot {
                    let handle = ask(cx, Query::BlacklistRivalHandle) as u32;
                    if handle != u32::MAX {
                        cx.host.request(Request::BlacklistRelease { handle });
                        self.phase = 4;
                    }
                } else {
                    self.phase = 1;
                }
            }
            _ => {
                if slot && distance <= BLACKLIST_REMOVE_DISTANCE_SQ {
                    // Stays in phase 6 while the rival is near.
                } else {
                    if slot {
                        cx.host.request(Request::BlacklistRemoveSlot);
                    }
                    self.phase = 1;
                }
            }
        }
        if ask_bool(cx, Query::BlacklistCreditsPending) {
            if ask_bool(cx, Query::CreditsSequenceActive) {
                let path = "GeneralSequence.Triggers[\"EXIT_TO_CREDITS\"].IsTriggerSet[1]";
                if !cx.host.binding_bool(path).unwrap_or(false) {
                    self.credits_seen = true;
                    return false;
                }
            }
            if self.credits_seen {
                cx.host.request(Request::SetCreditsTransition { on: true });
                cx.host.request(Request::GameCreditsFinished);
                cx.set_exit(key("CREDITS") as Key);
                return true;
            }
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        let mode = ask(cx, Query::BlacklistMode);
        mode != 2 && mode != 3
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.retract("BlacklistShutDown");
        if ask_bool(cx, Query::BlacklistSlot) {
            cx.host.request(Request::BlacklistRemoveSlot);
        }
    }
}

impl DoBlacklistShutdowns {
    fn after_step(&mut self, cx: &mut Cx) {
        let after = ask(cx, Query::BlacklistPhaseAfter) as u32;
        if after != 0 {
            self.phase = after - 1;
        }
    }
}

/// "Checkpoints" (`CHECKPTS`, factory 0x77b420, vtable 0xfd94a0, 0x200 B): the race's checkpoints (the manager
/// G+0x2f47c, count +0x510). Enter (0x749670) resets the progress; the enter update (0x75d8d0) places them once
/// loaded (at once when there are none); the update (0x75da20) steps them each frame (the checkpoint passed,
/// split times, the next one's marker) and exits through `EXIT` when the last one was reached and the widget
/// manager's state G+0x2f458 +0x85c is not 3, at once with no checkpoints; exit (0x75d960) removes them. The
/// stepping is `natives::checkpoints` (run by the host on `CheckpointsStep`; the queries are its answers).
pub struct Checkpoints;

impl Behaviour for Checkpoints {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::CheckpointsEnter);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if ask(cx, Query::CheckpointCount) as i64 > 0 {
            if !ask_bool(cx, Query::CheckpointsLoaded) {
                return false;
            }
            cx.host.request(Request::CheckpointsPlace);
        }
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask(cx, Query::CheckpointCount) as i64 > 0 {
            cx.host.request(Request::CheckpointsStep);
            if !ask_bool(cx, Query::CheckpointsFinished) {
                return false;
            }
        }
        cx.set_exit(EXIT);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::CheckpointsLeave);
    }
}

/// "Event Starts" (`EVENTSTARTS`, factory 0x77b510, vtable 0xfd950c, 0x1f0 B): the event start markers of the
/// world. Enter (0x754f30) reads `CHECKENTRY` (true: the player entering a start ends the state), `POIID`,
/// `RECPOIID`, `MWPOIIDC`, `MWPOIIDUC` and `TRACKEDONLY`; the update (0x755220) refreshes the starts and, with
/// `CHECKENTRY`, exits through `EXIT` when the player stands in one ([`Query::EventStartTriggered`]); exit
/// (0x74ab80) removes them. The markers, the prompt and the burnout are `natives::event_starts` (run by the host on
/// `EventStartParams` / `EventStartsStep` / `EventStartsLeave`).
#[derive(Default)]
pub struct EventStarts {
    check_entry: bool,
}

impl Behaviour for EventStarts {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.check_entry = param_true(cx, "CHECKENTRY");
        cx.host.request(Request::EventStartsEnter { check_entry: self.check_entry });
        let int = |cx: &Cx, name: &str| super::param_int(cx, name).max(0) as u32;
        let params = EventStartParams {
            check_entry: self.check_entry,
            poi: int(cx, "POIID"),
            rec_poi: int(cx, "RECPOIID"),
            mw_poi: int(cx, "MWPOIIDC"),
            mw_poi_won: int(cx, "MWPOIIDUC"),
            tracked_only: param_true(cx, "TRACKEDONLY"),
            respect_radius: param_true(cx, "RSPCTRADIUS"),
            active_radius: cx.param_f32("ACTIVERADIUS"),
        };
        cx.host.request(Request::EventStartParams(params));
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::EventStartsStep);
        if self.check_entry && ask_bool(cx, Query::EventStartTriggered) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::EventStartsLeave);
    }
}

/// "Car Swap Logic" (`HSMCARSWAPLO`, factory 0x77f790, vtable 0xfdb4c8, 0x74d0 B): the car swap at a jack spot,
/// a 7-phase machine of the engine. Enter (0x758ba0) initialises it and registers the view model `JackSpots`; the
/// update (0x761d70) does nothing until the engine has started it (+0x74c8), then runs a step per frame and never
/// exits by itself; exit (0x75d510) releases it. The phases are engine code (not ported).
pub struct CarSwapLogic;

impl Behaviour for CarSwapLogic {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::CarSwapEnter);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::CarSwapActive) {
            cx.host.request(Request::CarSwapStep);
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::CarSwapExit);
    }
}
