//! Progression-side behaviour types: the unlock notifications of the challenge / online event flows, the online
//! event winner record, the feedback message player, the carjacker listener of the takedown sequence and the data
//! state write (`ShowInGameMenu`). Slot order of every vtable: enter, enter update, running, update, exit update,
//! exit begin, exit. Engine effects are [`Request`]s, engine state [`Query`]s (both name the original's code).
//! Parameter keys decoded from snapshot 1 (`QUEUE` 0x117a200, `MESSAGE` 0x117ac08, `CLEARQONEXIT` 0x117a448,
//! `WINNER` 0x117aa90, `DATASTATE` 0x117a7b0, `FIELD` 0x117a190, `VALUE` 0x117a0e0, `EXIT` 0x117a970).

use mw_formats::json::Json;

use super::{Query, ask, ask_bool, param_int, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("CONSMUNLOCKS", |_| ConsumeAndShowUnlocks);
    r.register_fn("RCDONEVNTWNR", |_| RecordOnlineEventWinner);
    r.register_fn("PLAYFEEDBACK", |_| PlayFeedbackMessage::default());
    r.register_fn("CARJACKERACT", |_| ListenForCarjackerActivated);
    r.register_fn("WRITETODATAS", |_| WriteToDataState);
}

/// The progression object's (G+0x2f3e4) state +0x44 value under which unlocks are consumed (0x75bf12 `cmp 4`).
const PROGRESSION_ACTIVE: u64 = 4;

/// No feedback message (`PLAYFEEDBACK` enter 0x74f00d: +0x48 = 0x7fffffff).
pub const NO_FEEDBACK_MESSAGE: u64 = 0x7fff_ffff;

/// "Consume and Show Unlocks" (`CONSMUNLOCKS`, factory 0x77e310, vtable 0xfdaa7c, 0x48 B): only the update
/// (0x75bf00) is not a default stub. Every frame, when the progression object G+0x2f3e4 is in state 4 (+0x44,
/// [`Query::ProgressionState`]) it pops the next pending unlock (`FUN_0067f430`: the first of the 11 unlock queues
/// at +0x1208 (0x1c B each, 8-entry rings of i16 ids, read index +0x10, count +0x18) with an entry; the id goes
/// through the object's vtable +8; [`Query::PopPendingUnlock`]) and, with one, builds its notification (the
/// unlock's vtable +0x14 / +0x2c (kinds 6 and 3 take the alternative text) / +0x34 / +0x8, the attribute tables of
/// 0x10edac0, a 0x31c-byte UI message (vtable 0xfbf9ac, `FUN_00824f90`) on the UI manager's message queue
/// G+0x2f480 +0x1245c, `FUN_007983b0` on +0x32858): [`Request::ShowUnlock`]. It returns 0: it never exits by
/// itself, its children run.
pub struct ConsumeAndShowUnlocks;

impl Behaviour for ConsumeAndShowUnlocks {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask(cx, Query::ProgressionState) == PROGRESSION_ACTIVE {
            let unlock = ask(cx, Query::PopPendingUnlock);
            if unlock != u64::MAX {
                cx.host.request(Request::ShowUnlock { unlock });
            }
        }
        false
    }
}

/// "RecordOnlineEventWinner" (`RCDONEVNTWNR`, factory 0x77e7c0, vtable 0xfdac98, 0x48 B): enter (0x74fa90) runs
/// only on the session's host (`FUN_004c22c0` → `FUN_00a82ca0`, [`Query::OnlineHost`]): the winner is the
/// integer value of the binding path `WINNER` (`FUN_00744dd0`: −1 when the parameter is missing or the binding does
/// not resolve), recorded in the game mode's online event list ([`Request::RecordOnlineEventWinner`]). The update
/// (0x73ec00) exits through `EXIT`. Offline the enter does nothing.
pub struct RecordOnlineEventWinner;

impl Behaviour for RecordOnlineEventWinner {
    fn on_enter(&mut self, cx: &mut Cx) {
        if !ask_bool(cx, Query::OnlineHost) {
            return;
        }
        let path = cx.param_text("WINNER");
        let winner = if cx.param("WINNER").is_some() { cx.host.binding_int(&path).unwrap_or(-1) } else { -1 };
        cx.host.request(Request::RecordOnlineEventWinner { winner });
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Play Feedback Message" (`PLAYFEEDBACK`, factory 0x77d7b0, vtable 0xfda55c, 0x58 B): enter (0x74f000) sets the
/// message handle +0x48 to 0x7fffffff, looks the feedback queue `QUEUE` up by name in the UI manager's queue list
/// (G+0x2f480 +0x17e0, `FUN_007b2940`: exact compare with each queue's +0x1c; [`Query::FeedbackQueueExists`]) into
/// +0x4c and, with one, posts message `atoi(MESSAGE)` to it (`FUN_007b96b0`(queue, id, *0x10c5c34, 0, 0,
/// 0x10c5c70) → the handle, [`Query::PostFeedbackMessage`]); +0x50 = `CLEARQONEXIT` equals "true" (`_stricmp`
/// with 0xfd12dc). The update (0x73e7a0) runs while a message of the queue has the handle at +0x330
/// (`FUN_007aeb10`, [`Query::FeedbackMessageQueued`]), then exits through `EXIT`. The exit (0x73e7d0) clears the
/// queue with `CLEARQONEXIT` (`FUN_007b3410`, [`Request::ClearFeedbackQueue`]). Without the queue the original
/// calls through a null pointer in the update; the port exits instead.
#[derive(Default)]
pub struct PlayFeedbackMessage {
    queue: Option<String>,
    handle: u64,
    clear_on_exit: bool,
}

impl Behaviour for PlayFeedbackMessage {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.handle = NO_FEEDBACK_MESSAGE;
        let queue = cx.param_text("QUEUE");
        self.queue = ask_bool(cx, Query::FeedbackQueueExists { queue: queue.clone() }).then_some(queue);
        if let Some(queue) = self.queue.clone() {
            let message = param_int(cx, "MESSAGE");
            self.handle = ask(cx, Query::PostFeedbackMessage { queue, message });
        }
        self.clear_on_exit = param_true(cx, "CLEARQONEXIT");
    }
    fn update(&mut self, cx: &mut Cx) -> bool {
        if let Some(queue) = self.queue.clone()
            && ask_bool(cx, Query::FeedbackMessageQueued { queue, handle: self.handle })
        {
            return false;
        }
        cx.set_exit(EXIT);
        true
    }
    fn on_exit(&mut self, cx: &mut Cx) {
        if self.clear_on_exit
            && let Some(queue) = self.queue.clone()
        {
            cx.host.request(Request::ClearFeedbackQueue { queue });
        }
    }
}

/// "ListenForCarjackerActivated" (`CARJACKERACT`, factory 0x77e680, vtable 0xfdac08, 0x50 B; in "Run Takedown
/// Sequence"): only the update (0x74dc30) is not a default stub. It waits (returns 0) until the rollout manager's
/// record of the local player's slot is a carjacker one (`FUN_00769e40` on G+0x2f404 +0x10 with player manager
/// +0xa6b4, [`Query::CarjackerActive`]) and the local player's input record (`FUN_0047aa20` of player +0x254) has
/// its word +0xf4 at 2 ([`Query::CarjackerButton`]). Then, unless the takedown target's car (player +0x2d8[+0x2ec];
/// that player's vtable +0x18, `FUN_00678960`) is one the local player has not collected yet
/// ([`Query::CarjackerCarNotCollected`]), the jack is recorded ([`Request::CarjackerActivated`]); either way it
/// exits through `EXIT`.
pub struct ListenForCarjackerActivated;

impl Behaviour for ListenForCarjackerActivated {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::CarjackerActive) || !ask_bool(cx, Query::CarjackerButton) {
            return false;
        }
        if !ask_bool(cx, Query::CarjackerCarNotCollected) {
            cx.host.request(Request::CarjackerActivated);
        }
        cx.set_exit(EXIT);
        true
    }
}

/// "Write to DataState" (`WRITETODATAS`, factory 0x77a300, vtable 0xfd9074, 0x48 B): only the update (0x747310)
/// is not a default stub: `VALUE` is read as JSON (`FUN_009f8fc0` with the machine's allocator `FUN_007749a0`) and
/// stored as member `FIELD` of the data state `DATASTATE` ([`Cx::write_data_state`]); then it exits through
/// `EXIT`. The data uses it for `ActiveGameMode.ShowInGameMenu` = true / false. A missing data state is a null
/// pointer in the original; the port skips the write.
pub struct WriteToDataState;

impl Behaviour for WriteToDataState {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if let Ok(value) = Json::parse(&cx.param_text("VALUE")) {
            let (name, field) = (cx.param_text("DATASTATE"), cx.param_text("FIELD"));
            cx.write_data_state(&name, &field, value);
        }
        cx.set_exit(EXIT);
        true
    }
}
