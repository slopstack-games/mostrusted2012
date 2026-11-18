//! The game modes' scoring subsystems that run as behaviours: drift scoring (`HSMDODRIFTS`, "Drift Game Mode") and
//! smash-and-grab scoring (`DOSMSHNGRB`, "Smash And Grab Mode" 2210003). As `CHECKPTS`, the behaviour keeps the
//! lifecycle and the logic's managers do the work (`natives::drifts`, `natives::smash_grab`): each slot is a
//! [`Request`] the host carries out at once, and the queries read the outcome.

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool};
use crate::hsm::{Behaviour, Cx, Registry};
use crate::natives::Request;
use crate::natives::drifts::Slot;

pub fn register(r: &mut Registry) {
    r.register_fn("HSMDODRIFTS", |_| DriftScoring);
    r.register_fn("DOSMSHNGRB", |_| SmashGrabScoring);
}

/// "HSMDoDriftScoring" (`HSMDODRIFTS`, factory 0x77fc80, vtable 0xfdc148, 0x1340 B): enter 0x75b2f0, enter update
/// `FUN_0074c370` (loads the marker list and models, builds the gates), update 0x75e7e0 (never exits by itself:
/// returns 0), slots 2 / 4 / 5 the defaults, exit `FUN_0075b460`.
pub struct DriftScoring;

impl Behaviour for DriftScoring {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::DriftScoring(Slot::Enter));
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::DriftScoring(Slot::EnterUpdate));
        ask_bool(cx, Query::DriftScoringReady)
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::DriftScoring(Slot::Update));
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::DriftScoring(Slot::Exit));
    }
}

/// "HSM.DoSmashAndGrabScoring" (`DOSMSHNGRB`, factory 0x77e150, vtable 0xfda9c8, 0x1258 B): enter `FUN_0075b820`,
/// enter update `FUN_007612e0`, update `FUN_00761450` (exits through `WIN` (`DAT_0117ab20`) or `LOSE`
/// (`DAT_0117a6b8`), read from snapshot 1), exit update `FUN_0075ed20`, exit 0x73cf00; slots 2 / 5 the defaults.
pub struct SmashGrabScoring;

impl Behaviour for SmashGrabScoring {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SmashGrabScoring(Slot::Enter));
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::SmashGrabScoring(Slot::EnterUpdate));
        ask_bool(cx, Query::SmashGrabReady)
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::SmashGrabScoring(Slot::Update));
        let exit = match ask(cx, Query::SmashGrabEnd) {
            1 => key("WIN"),
            2 => key("LOSE"),
            _ => return false,
        };
        cx.set_exit(exit);
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::SmashGrabScoring(Slot::ExitUpdate));
        ask_bool(cx, Query::SmashGrabCleared)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SmashGrabScoring(Slot::Exit));
    }
}
