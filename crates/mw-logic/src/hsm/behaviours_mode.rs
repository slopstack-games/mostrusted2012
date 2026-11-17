//! Behaviour types the game-mode machines run (`GAMELOGIC/GAMEMODES.BNDL`, first "Freedrive Game Mode" 390148):
//! traffic restart, speed-wall times, screen prefetches, simulation pauses, showing the world. Engine effects are
//! [`Request`]s, waits [`Query`]s (both name the original's code).

use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("REQTRAFRSTRT", |_| RequestTrafficRestart::default());
    r.register_fn("GETSPEEDWALL", |_| GetSpeedwallTimes);
    r.register_fn("PREFETCHSCRE", |_| PrefetchScreen::default());
    r.register_fn("ALLOWSIMPAUS", |_| AllowSimPause);
    r.register_fn("SETSIMPAUSES", |_| SetSimPaused);
    r.register_fn("HSB_RUNNING", |_| ShowWorld);
    r.register_fn("BPAUSED", |_| Paused);
    r.register_fn("ENABLETRIGS", |_| EnableTriggers);
    r.register_fn("HSB_DISCONGP", |_| DisableControlGroup::default());
}

/// `REQTRAFRSTRT`'s time-out: `DAT_0117aaf8` = 18,000,000 ticks (6 s at 3e6 ticks / s; `.bss`, read in snapshot 1).
pub const TRAFFIC_RESTART_TICKS: i64 = 18_000_000;

/// "RequestTrafficRestart" (`REQTRAFRSTRT`, factory 0x77da30, vtable 0xfda698, 0x58 B): on enter (0x73ec50) asks
/// for a traffic restart unless the traffic density is 0 and arms a 6 s time-out (+0x50); the update (0x744e70)
/// exits through `EXIT` when the density is 0 or the time-out ran out (clock +0x18 × 3e6 ticks per update, as
/// `WAIT4TIME`), else offline at once; online (`FUN_006dfed0`) it waits on the traffic's time stamp
/// (`FUN_005ea560`) with its state +0x48.
#[derive(Default)]
pub struct RequestTrafficRestart {
    /// +0x50: ticks left.
    ticks: i64,
    /// +0x48.
    state: u32,
}

impl Behaviour for RequestTrafficRestart {
    fn on_enter(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::TrafficDensityZero) {
            cx.host.request(Request::RestartTraffic);
        }
        self.ticks = TRAFFIC_RESTART_TICKS;
        self.state = 0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::TrafficDensityZero) {
            cx.set_exit(EXIT);
            return true;
        }
        self.ticks -= (f64::from(cx.clock.frame) * 3.0e6) as i64;
        if self.ticks < 0 {
            cx.set_exit(EXIT);
            return true;
        }
        let online = ask_bool(cx, Query::OnlineEvent);
        let go_on = match self.state {
            0 => !online || !ask_bool(cx, Query::TrafficTimeSet),
            s => s == 1,
        };
        if !go_on {
            return false;
        }
        self.state = 1;
        if ask_bool(cx, Query::OnlineEvent) && !ask_bool(cx, Query::TrafficTimeSet) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "GetSpeedwallTimesToBeat" (`GETSPEEDWALL`, factory 0x77dfb0, vtable 0xfda914, 0x48 B): on enter (0x73c790) asks
/// the speed-wall leaderboard (GMM+0x20, `FUN_006f1dc0`) for the pending event's times unless it is a free-drive
/// event (`FUN_006e5ea0`); exits through `EXIT` once no request is pending (update 0x73c7e0, `FUN_006e0860` →
/// `FUN_0086e260`), and on the way out waits for the same (exit update 0x73c820).
pub struct GetSpeedwallTimes;

impl Behaviour for GetSpeedwallTimes {
    fn on_enter(&mut self, cx: &mut Cx) {
        let event = ask(cx, Query::NextEvent) as u32;
        if !ask_bool(cx, Query::EventIsFreedrive { event }) {
            cx.host.request(Request::GetSpeedwallTimes { event });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::SpeedwallIdle) {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::SpeedwallIdle)
    }
}

/// "Prefetch Screen" (`PREFETCHSCRE`, factory 0x77f120, vtable 0xfdb0ac, 0x50 B): on enter (0x744c00) adds the
/// screen `SCREENASSET` (atoi) to the widget screen manager's prefetch list (G+0x2f458 +0x1940, `FUN_007f0e10`:
/// counted, loaded when new); enters once it is loaded (0x73eb50, `FUN_007f0ef0`); runs its children; on exit
/// (0x73eb70, `FUN_007f0e70`) counts it down (freed at 0).
#[derive(Default)]
pub struct PrefetchScreen {
    asset: i64,
}

impl Behaviour for PrefetchScreen {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.asset = param_int(cx, "SCREENASSET");
        cx.host.request(Request::PrefetchScreen { asset: self.asset });
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::ScreenPrefetched { asset: self.asset })
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ReleasePrefetchedScreen { asset: self.asset });
    }
}

/// "Allow Sim Pause" (`ALLOWSIMPAUS`, factory 0x77bab0, vtable 0xfd9788, 0x48 B): runs its children (update
/// 0x73aff0 returns 0); on exit (0x73b000), offline, resumes the simulation (`FUN_00845ce0`), undoing a "Pause Sim"
/// child.
pub struct AllowSimPause;

impl Behaviour for AllowSimPause {
    fn on_exit(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::OnlineEvent) {
            cx.host.request(Request::SimPause { paused: false });
        }
    }
}

/// "Set Sim Paused" (`SETSIMPAUSES`, factory 0x77bb00, vtable 0xfd97ac, 0x48 B): on enter (0x745d60), offline,
/// pauses the simulation when `PAUSED` is "true" (`FUN_00845c60`), else resumes it (`FUN_00845ce0`); exits
/// through `EXIT` at once (update 0x740740).
pub struct SetSimPaused;

impl Behaviour for SetSimPaused {
    fn on_enter(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::OnlineEvent) {
            let paused = param_true(cx, "PAUSED");
            cx.host.request(Request::SimPause { paused });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Show World" (`HSB_RUNNING`, factory 0x77f5a0, vtable 0xfdb250, 0x50 B): on enter (0x73f030) shows the world
/// (G+0x2ef70 +0x44 = 1, byte 0 posted on the queue `*0x010edb4c`) and counts the global `DAT_01179de0` up; runs
/// its children (update 0x73f080 returns 0); on exit begin (0x73f090) counts it down and, at 0, hides the world
/// again (+0x44 = 0, byte 1 posted) unless an event is requested (GMM+0x2ce4) in story mode (GMM+0x2e7a). The
/// enter also keeps the old +0x44 at +0x48 (0 with `HasTransitionedEvents`), which no slot reads again.
pub struct ShowWorld;

impl Behaviour for ShowWorld {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ShowWorld { show: true });
        cx.host.request(Request::AdjustShowWorldCount { delta: 1 });
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AdjustShowWorldCount { delta: -1 });
        if ask(cx, Query::ShowWorldCount) != 0 {
            return;
        }
        if ask(cx, Query::RequestedEvent) != 0 && ask_bool(cx, Query::StoryMode) {
            return;
        }
        cx.host.request(Request::ShowWorld { show: false });
    }
}

/// "Offline Pause" (`BPAUSED`, factory 0x77ba60, vtable 0xfd9764, 0x48 B): offline, pauses the simulation on enter
/// (0x73e520, `FUN_00845c60`) and resumes it on exit (0x73e560, `FUN_00845ce0`); runs its children (update
/// 0x73e550 returns 0).
pub struct Paused;

impl Behaviour for Paused {
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

/// "EnableTriggers" (`ENABLETRIGS`, factory 0x77d000, vtable 0xfda184, 0x48 B): enables the world's triggers on
/// enter (0x73c510 → `FUN_00846590`(1) on G+0x2f064) and disables them on exit (0x73c520, (0)); runs its
/// children (update 0x73c530 returns 0).
pub struct EnableTriggers;

impl Behaviour for EnableTriggers {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::EnableTriggers(true));
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::EnableTriggers(false));
    }
}

/// "Disable Control Group" (`HSB_DISCONGP`, factory 0x77bdd0, vtable 0xfd98f0, 0x50 B): on enter (0x7487b0) looks
/// up the control group `GROUP` (its name hashed, `FUN_009ed180`, in the user record's table `*0x010edad8`+4
/// +0xb218: 16-byte records, id +0xc, count +8, input-block groups from +0) and blocks bit 0 of each of its
/// groups in the car-control block set G+0x2f0a4 (`FUN_0083c960`); the update (0x748820) blocks them again every
/// frame and keeps running; on exit (0x748860) they are unblocked (`FUN_0083c930`).
#[derive(Default)]
pub struct DisableControlGroup {
    groups: Vec<u32>,
}

impl DisableControlGroup {
    fn block(&self, cx: &mut Cx, set: bool) {
        for &group in &self.groups {
            cx.host.request(Request::SetInputBlock { group, bit: 0, set });
        }
    }
}

impl Behaviour for DisableControlGroup {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("GROUP");
        let n = ask(cx, Query::ControlGroupCount { name: name.clone() });
        self.groups =
            (0..n as u32).map(|index| ask(cx, Query::ControlGroup { name: name.clone(), index }) as u32).collect();
        self.block(cx, true);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        self.block(cx, true);
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        self.block(cx, false);
    }
}
