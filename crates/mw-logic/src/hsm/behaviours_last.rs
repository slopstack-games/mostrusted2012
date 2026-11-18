//! The online free-drive and shakedown behaviour types: `PICKNEXTFDE`, `HSMSHAKEDOWN`, `TELEPORTTOME`.
//!
//! Method as the earlier batches: the factory is the function before the name stub (the `mov edx` of the name's
//! registration), the vtable the factory's `mov [eax], vtable`, the slot bodies from `text.asm` / the dump, the
//! `.bss` exit keys from snapshot 1. Slot order: enter, enter update, running, update, exit update, exit begin, exit.
//! The debug-variable registrations (`FUN_00a4e2c0` / `FUN_00a4d2a0` under "HSMBehaviour…") and their
//! unregistrations (the exit slots 0x73e650, 0x73d110) are not ported. Engine effects are [`Request`]s, engine
//! state [`Query`]s.

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_int};
use crate::hsm::{Behaviour, Cx, EXIT, Key, Registry};
use crate::natives::Request;

use super::game::PLAYBACK;
use std::sync::atomic::Ordering;

pub fn register(r: &mut Registry) {
    r.register_fn("PICKNEXTFDE", |_| PickNextFreedriveEvent::default());
    r.register_fn("HSMSHAKEDOWN", |_| ShakedownUpdate::default());
    r.register_fn("TELEPORTTOME", |_| TeleportToMeetup::default());
}

/// Ticks per second of the timers (`.rdata` 0x00f9f604 = 3e6).
const TICKS: f64 = 3.0e6;
/// Seconds per tick (`.rdata` 0x00f9f608 = 3.33333e-7, as the f32 it is).
const SECONDS_PER_TICK: f32 = 3.333_33e-7;

/// `_ftol` of the engine's `FUN_00eda646` on a step in ticks (truncation; the original's rounding helper is a
/// float-to-int64 conversion, not read further).
fn ticks_of(seconds: f32) -> i64 {
    (f64::from(seconds) * TICKS) as i64
}

fn exit(cx: &mut Cx, exit: Key) -> bool {
    cx.set_exit(exit);
    true
}

// ---------------------------------------------------------------------------------------------------------------
// PICKNEXTFDE

/// How long the exit update holds a picked event (`.rdata` 0x00f92eb0 = 3.0 s).
pub const PICK_HOLD_SECONDS: f32 = 3.0;

/// "Pick Next Event" (`PICKNEXTFDE`, factory 0x77d1e0, vtable 0xfda268, 0x58 B; debug variable
/// `HSMBehaviourPickNextFreedriveEvent` with `RunIntermission` / `WaitingForPlayers` not ported): the online
/// free drive's next-event pick from the playlist runner (GMM+0x14). Object: state +0x48 (0 / 1 / 2), timer +0x4c,
/// picked flag +0x50. Enter (0x757d70) steps the playlist (`FUN_00705880`, [`Request::AdvancePlaylist`]) and clears
/// all three. Update (0x7446e0), every frame adds the clock step (`FUN_0046ee10`'s +0x14 × +0x10) to the timer;
/// in state 2 it waits for the playlist (`FUN_006e9a30`, [`Query::PlaylistValid`]) and steps it, back to state 0;
/// on the picking side (the runner's byte +0x7c clear, [`Query::PlaylistWaitingForHost`]) it clears the picked
/// event (GMM+0x1c +4 = 0, [`Request::SetPickedEvent`]), and with a playlist takes the current entry's event
/// into it (`FUN_006e98a0`), swaps the game mode to the event G+0x2a9d8 holds (`FUN_006e74f0`(GMM, id, 1),
/// [`Request::SwapGameMode`]), sets the picked flag and exits through `EXIT`; without a playlist it goes to state
/// 2. On the other side it only notes (state 1) that the playlist ran out (index +4 ≥ the entry count +0x140).
/// Exit update (0x73e610): the timer keeps counting; done when nothing was picked or 3.0 s have passed.
///
/// Model decision: the step is the game clock's even while its running flag is clear (the original multiplies the
/// +0x14 and +0x10 fields without looking at +0x1c); the interpreter's `Clock::game` is `None` then, taken as 0.
#[derive(Default)]
pub struct PickNextFreedriveEvent {
    state: u32,
    timer: f32,
    picked: bool,
}

impl PickNextFreedriveEvent {
    fn step(cx: &Cx) -> f32 {
        cx.clock.game.unwrap_or(0.0)
    }
}

impl Behaviour for PickNextFreedriveEvent {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdvancePlaylist);
        *self = PickNextFreedriveEvent::default();
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        self.timer += Self::step(cx);
        if self.state == 2 {
            if ask_bool(cx, Query::PlaylistValid) {
                cx.host.request(Request::AdvancePlaylist);
                self.state = 0;
            }
            return false;
        }
        if !ask_bool(cx, Query::PlaylistWaitingForHost) {
            cx.host.request(Request::SetPickedEvent { event: 0 });
            if ask_bool(cx, Query::PlaylistValid) {
                let event = ask(cx, Query::PlaylistCurrentEvent) as u32;
                if event != 0 {
                    cx.host.request(Request::SetPickedEvent { event });
                }
                let mode = ask(cx, Query::OnlinePendingEvent) as u32;
                cx.host.request(Request::SwapGameMode { mode: mode as i32 });
                self.picked = true;
                return exit(cx, EXIT);
            }
            self.state = 2;
            return false;
        }
        if ask_bool(cx, Query::PlaylistValid) && ask_bool(cx, Query::PlaylistFinished) {
            self.state = 1;
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        self.timer += Self::step(cx);
        !(self.picked && self.timer < PICK_HOLD_SECONDS)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// HSMSHAKEDOWN

/// `HSMSHAKEDOWN` result for a player who ran out of time (`FUN_006b9d00`'s medal word).
pub const SHAKEDOWN_NO_SCORE: u32 = 4;

/// "Ambush HSM Behaviour" (`HSMSHAKEDOWN`, factory 0x77e100, vtable 0xfda9a4, 0x60 B; debug variable
/// `HSMBehaviourHSMShakedownUpdate` with `TargetTime`, `MedalIndex` and the elapsed time, not ported): the
/// shakedown mode's timer and medals. Object: targets +0x48 / +0x4c / +0x50 (the first two medal times, the
/// no-score limit) and +0x54 (the target time), elapsed ticks +0x58 (u64).
///
/// Enter (0x7572f0): +0x54 = −1.0 (`.rdata` 0x00f8f340), ticks 0; the current event record's parameter of type 10
/// (the event data's parameter list, `FUN_006e66a0`): with mode word 3 (`FUN_006e7610`) the first three are
/// 64-bit values divided by 3 and then by 1 000 000 (`__alldiv`, as a float), otherwise 32-bit words
/// (unsigned → float); the fourth is always the 32-bit word at +0xc ([`Query::ShakedownParamKind`] /
/// [`Query::ShakedownParam`]). Update (0x74d280): the local player busted (pursuit state +0x89c = 2): exit
/// `BUSTED` (key 0x117a588). Only while the game-mode clock runs (GMM+0x2e74 and the clock's +0x1c) the elapsed
/// ticks grow by the step × 3e6 and the over-time flag (G+0x2f410 +0xa00) is set when the elapsed seconds (ticks ×
/// 3.33333e-7) exceed the target +0x54 ([`Request::SetShakedownOver`]); when they do and the local player is not
/// in a pursuit (`FUN_006c2410`, [`Query::LocalPlayerInPursuit`]) the medal (`FUN_00743de0`: 0 up to the first
/// time × 3e6 ticks, 1 up to the second, else 2) + 1 and the ticks go into the player's result
/// ([`Request::SetShakedownResult`]), GMM+0x2d28 = the ticks, exit `EVADED` (0x117a6d8); else, past the third time
/// × 3e6 ticks, the result is 4 (no score) and the exit `NOSCORE` (0x117a708).
#[derive(Default)]
pub struct ShakedownUpdate {
    targets: [f32; 4],
    ticks: i64,
}

impl ShakedownUpdate {
    fn threshold(&self, index: usize) -> i64 {
        (f64::from(self.targets[index]) * TICKS) as i64
    }

    /// `FUN_00743de0` (the clock runs when it is called).
    fn medal(&self) -> u32 {
        if self.ticks <= self.threshold(0) {
            0
        } else if self.ticks <= self.threshold(1) {
            1
        } else {
            2
        }
    }
}

impl Behaviour for ShakedownUpdate {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.targets = [0.0, 0.0, 0.0, -1.0];
        self.ticks = 0;
        let kind = ask(cx, Query::ShakedownParamKind);
        if kind == u64::MAX {
            return;
        }
        for i in 0..3u32 {
            let raw = ask(cx, Query::ShakedownParam { index: i });
            self.targets[i as usize] =
                if kind == 3 { ((raw as i64) / 3 / 1_000_000) as f32 } else { raw as u32 as f32 };
        }
        self.targets[3] = ask(cx, Query::ShakedownParam { index: 3 }) as u32 as f32;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask(cx, Query::LocalPursuitState) == 2 {
            return exit(cx, key("BUSTED"));
        }
        if !ask_bool(cx, Query::GameModeClockRunning) {
            return false;
        }
        let Some(step) = cx.clock.game else { return false };
        self.ticks += ticks_of(step);
        let seconds = self.ticks as f64 * f64::from(SECONDS_PER_TICK);
        let over = f64::from(self.targets[3]) < seconds;
        cx.host.request(Request::SetShakedownOver { over });
        if over && !ask_bool(cx, Query::LocalPlayerInPursuit) {
            let medal = self.medal() + 1;
            cx.host.request(Request::SetShakedownResult { medal, ticks: self.ticks as u64 });
            cx.host.request(Request::SetGameModeElapsedTicks { ticks: self.ticks as u64 });
            return exit(cx, key("EVADED"));
        }
        if self.ticks > self.threshold(2) {
            cx.host.request(Request::SetShakedownResult { medal: SHAKEDOWN_NO_SCORE, ticks: self.ticks as u64 });
            return exit(cx, key("NOSCORE"));
        }
        false
    }
}

// ---------------------------------------------------------------------------------------------------------------
// TELEPORTTOME

/// The countdown of a teleport that did not land on the meet-up checkpoint (`.bss` 0x117a028 = 22 500 000 ticks,
/// read from snapshot 1: 7.5 s).
pub const TELEPORT_COUNTDOWN_TICKS: i64 = 22_500_000;

/// "Teleport To Meetup" (`TELEPORTTOME`, factory 0x77f1c0, vtable 0xfdb0f4, 0x80 B): moves the local player to the
/// online meet-up of the picked event. Object: the meet-up trigger +0x60, teleported flag +0x64, countdown +0x68
/// (u64 ticks; high word 0x80000000 = none), the destination +0x50..+0x5c, the sequence handle +0x70.
///
/// Enter (0x752b30): flag 0; the trigger is the picked event's (G+0x2a9d8, [`Query::OnlinePendingEvent`]) meet-up
/// trigger (`FUN_006e5dc0`'s data +0x5c, [`Query::MeetupTrigger`]); no countdown, no sequence. Update (0x752ba0): a
/// wrecked local player (`FUN_006ba290`, [`Query::LocalPlayerWrecked`]) waits; one already in the trigger
/// (`FUN_006e90a0`, [`Query::MeetupInCheckpoint`]) just exits; otherwise the host picks the destination and
/// teleports the vehicle ([`Query::MeetupTeleport`]: the geometry of 0x752ba0 (spawn list or random zone point,
/// ground ray `FUN_004b3b20`, orientation, `FUN_00844770`) is the engine's). Refused: flag 0, the online manager's
/// byte +0x2e9a = 1 and the pending event reset ([`Request::MeetupTeleportRefused`]). Teleported: flag 1; unless
/// the local player already is at the checkpoint (`FUN_006e9200`, [`Query::MeetupLocalAtCheckpoint`]) the 7.5 s
/// countdown starts; the state's `Sequence` parameter (`atoi`) plays as a free sequence
/// ([`Request::PlaySequence`], handle +0x70). Either way the local player's byte +0x2c8 takes the flag
/// ([`Request::SetMeetupTeleporting`]) and the state exits through `EXIT`. Exit update (0x7464f0): done at once
/// when nothing was teleported; else the countdown runs down by the frame clock's step (a negative value ends it)
/// and the flag stays set until it has ended and the area around the destination has loaded
/// ([`Query::MeetupAreaReady`]: `FUN_005470e0` and `FUN_0054b540`), mirrored into +0x2c8 every frame; never done
/// in that frame. Exit (0x75d050): the end message ([`Request::PostMeetupEndMessage`], online telemetry first, not
/// ported), +0x2c8 cleared, the sequence stopped.
#[derive(Default)]
pub struct TeleportToMeetup {
    trigger: u32,
    teleported: bool,
    countdown: Option<i64>,
    playback: Option<u32>,
}

impl Behaviour for TeleportToMeetup {
    fn on_enter(&mut self, cx: &mut Cx) {
        let event = ask(cx, Query::OnlinePendingEvent) as u32;
        self.trigger = ask(cx, Query::MeetupTrigger { event }) as u32;
        self.teleported = false;
        self.countdown = None;
        self.playback = None;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::LocalPlayerWrecked) {
            return false;
        }
        if !ask_bool(cx, Query::MeetupInCheckpoint { trigger: self.trigger }) {
            if ask_bool(cx, Query::MeetupTeleport { trigger: self.trigger }) {
                self.teleported = true;
                if !ask_bool(cx, Query::MeetupLocalAtCheckpoint) {
                    self.countdown = Some(TELEPORT_COUNTDOWN_TICKS);
                }
                let sequence = param_int(cx, "Sequence") as u32;
                if sequence != 0 {
                    let playback = PLAYBACK.fetch_add(1, Ordering::Relaxed);
                    cx.host.request(Request::PlaySequence { playback, sequence, player: None });
                    self.playback = Some(playback);
                }
            } else {
                self.teleported = false;
                cx.host.request(Request::MeetupTeleportRefused);
            }
        }
        cx.host.request(Request::SetMeetupTeleporting { on: self.teleported });
        exit(cx, EXIT)
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if !self.teleported {
            return true;
        }
        if let Some(left) = self.countdown {
            let left = left - ticks_of(cx.clock.frame);
            self.countdown = if left < 0 { None } else { Some(left) };
        }
        let ready = ask_bool(cx, Query::MeetupAreaReady);
        self.teleported = !(ready && self.countdown.is_none());
        cx.host.request(Request::SetMeetupTeleporting { on: self.teleported });
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PostMeetupEndMessage);
        cx.host.request(Request::SetMeetupTeleporting { on: false });
        if let Some(playback) = self.playback.take() {
            cx.host.request(Request::StopPlayback { playback });
        }
    }
}
