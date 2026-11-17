//! Behaviour types of the dialogs, the pause interrupt, the photo mode and the page runner (`GAMELOGIC/GAMEMODES.BNDL`):
//! the popup-dialog manager, the pause interrupt, the preference suppressor, the PageRunner context runner, the
//! photo states and the limited-edition check. Engine effects are [`Request`]s, engine state [`Query`]s (both
//! name the original's code). Slot order of every vtable: enter, enter update, running, update, exit update,
//! exit begin, exit.

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("SHOWDIALOG", |_| ShowDialog::default());
    r.register_fn("HSMPAUSEINTR", |_| PauseInterrupt::default());
    r.register_fn("SUPPRESSPREF", |_| SuppressPreferences);
    r.register_fn("RUNPRCONTEXT", |_| RunPrContext::default());
    r.register_fn("TAKEPHOTO", |_| TakePhoto);
    r.register_fn("PHOTO_LISTEN", |_| PhotoListen);
    r.register_fn("PHOTO_COMPL", |_| PhotoComplete);
    r.register_fn("PHOTO_INPUT", |_| PhotoInput);
    r.register_fn("LIMITEDEDSP", |_| LimitedEdition::default());
}

/// "Show Dialog" (`SHOWDIALOG`, factory 0x77df10, vtable 0xfda8cc, 0x68 B): enter (0x745e10) reads the parameters
/// `DIALOG` (`atoi`), `POS_EXIT` and `NEG_EXIT` (exit points, `FUN_009ed220`; keys from snapshot 1: 0x117a4c8 /
/// 0x117a6e0 / 0x117a0d8) and requests the dialog with `RequestDialog`'s function (`FUN_007bb330` on the dialog
/// manager G480+0x47740: [`Request::ShowDialog`], which the Lua native uses too). The update (0x740a20) reads the
/// result like `GetDialogResult` (`FUN_007b83f0`: [`Query::DialogResult`], 0xff = none yet = keep waiting),
/// takes it (`FUN_007b69b0` → `FUN_007c0400`: [`Request::TakeDialogResult`]) and exits through `POS_EXIT` for
/// the result 1, else `NEG_EXIT`. Exit (0x740a80): when the dialog is still open (`FUN_007aec80`:
/// [`Query::DialogOpen`]) `ClearDialog`'s function (`FUN_007b3a10`: [`Request::CloseDialog`]).
#[derive(Default)]
pub struct ShowDialog {
    id: u32,
    positive: u64,
    negative: u64,
}

impl Behaviour for ShowDialog {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.id = super::param_int(cx, "DIALOG") as i32 as u32;
        self.positive = key(&cx.param_text("POS_EXIT"));
        self.negative = key(&cx.param_text("NEG_EXIT"));
        cx.host.request(Request::ShowDialog { id: self.id });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let result = ask(cx, Query::DialogResult { id: self.id }) as u8 as i8;
        if result == -1 {
            return false;
        }
        cx.host.request(Request::TakeDialogResult { id: self.id });
        cx.set_exit(if result == 1 { self.positive } else { self.negative });
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if ask_bool(cx, Query::DialogOpen { id: self.id }) {
            // `last` is the dialog manager's to know: the host fills it in (`LogicHost::request`).
            cx.host.request(Request::CloseDialog { id: self.id, last: false });
        }
    }
}

/// Frames the interrupt is held before it may end (`0xf < frames`, 0x74d040).
const PAUSE_INTERRUPT_FRAMES: u32 = 15;

/// "Pause Interrupt" (`HSMPAUSEINTR`, factory 0x77ea80, vtable 0xfdadb8, 0x50 B): enter (0x73d100) clears the
/// active byte (+0x48) and the frame counter (+0x4c). Update (0x74d040, never exits): idle, it watches the UI
/// manager's screen state for an active pause-interrupt item ([`Query::PauseInterruptTriggered`], whose scan
/// is the engine's: items of type 0x5f6d2 in the timeline list checked by `FUN_00827400`); found, it starts
/// the interrupt ([`Request::PauseInterruptStart`]) and sets +0x48. Active, it runs the first frame's setup
/// ([`Request::PauseInterruptBegin`]), and once the interrupt is released ([`Query::PauseInterruptReleased`]) and
/// more than 15 frames have passed it ends ([`Request::PauseInterruptEnd`]); the counter counts every active
/// frame. Exit (0x74cfe0): an active interrupt ends.
#[derive(Default)]
pub struct PauseInterrupt {
    active: bool,
    frames: u32,
}

impl Behaviour for PauseInterrupt {
    fn on_enter(&mut self, _cx: &mut Cx) {
        *self = Self::default();
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.active {
            if self.frames == 0 {
                cx.host.request(Request::PauseInterruptBegin);
            }
            if ask_bool(cx, Query::PauseInterruptReleased) && self.frames > PAUSE_INTERRUPT_FRAMES {
                cx.host.request(Request::PauseInterruptEnd);
                self.active = false;
            }
            self.frames += 1;
        } else if ask_bool(cx, Query::PauseInterruptTriggered) {
            cx.host.request(Request::PauseInterruptStart);
            self.active = true;
            self.frames = 0;
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        if self.active {
            cx.host.request(Request::PauseInterruptEnd);
        }
    }
}

/// "Suppress Preferences" (`SUPPRESSPREF`, factory 0x77f170, vtable 0xfdb0d0, 0x50 B): enter (0x740f70) takes the
/// preference object (G+0x2f458 +0x1940) and raises its suppress counter ([`Request::SuppressPrefs`],
/// `FUN_007edab0`); the enter update (0x740f90) waits until its byte +0x6dc is clear
/// ([`Query::PrefsSuppressPending`]); exit (0x740fb0) lowers the counter ([`Request::ReleasePrefs`],
/// `FUN_007edb10`). Never exits itself.
pub struct SuppressPreferences;

impl Behaviour for SuppressPreferences {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SuppressPrefs);
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        !ask_bool(cx, Query::PrefsSuppressPending)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ReleasePrefs);
    }
}

/// "Run PageRunner Context" (`RUNPRCONTEXT`, factory 0x77edb0, vtable 0xfdaf20, 0x58 B): the sibling of
/// `RUN_PG_RNNR2` on the same context manager (G+0x2ed74) with other functions. Enter (0x76f960) reads
/// `CONTEXT_NAME`, `DEF_PG_NAME`, `SUSPEND` and `RESTOREGRP` (only when the state has it), saves the focus group
/// (G480 +0x31c04) when restoring, and pushes the context ([`Request::PushPrContext`], `FUN_0083d630`). The enter
/// update (0x76ec20) waits for the context ([`Query::PrContextExists`]); the update (0x76ec50) exits through
/// `EXIT` when it is gone, else through the exit point it asked for ([`Query::PrContextExit`]); the exit update
/// (0x76ece0) leaves the context once ([`Request::LeavePrContext`]: suspended with `SUSPEND`, else popped), waits
/// until it is no longer busy and the services' state is 2 ([`Query::KeyboardIdle`]), then restores the focus
/// group ([`Request::RestoreFocusGroup`]).
#[derive(Default)]
pub struct RunPrContext {
    name: String,
    page: String,
    suspend: bool,
    restore: bool,
    saved_group: u64,
    first_exit: bool,
}

impl Behaviour for RunPrContext {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.first_exit = true;
        self.name = cx.param_text("CONTEXT_NAME");
        self.page = cx.param_text("DEF_PG_NAME");
        self.suspend = param_true(cx, "SUSPEND");
        self.restore = param_true(cx, "RESTOREGRP");
        self.saved_group = if self.restore { ask(cx, Query::FocusGroup) } else { 0 };
        cx.host.request(Request::PushPrContext { name: self.name.clone(), page: self.page.clone() });
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::PrContextExists { name: self.name.clone() })
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::PrContextExists { name: self.name.clone() }) {
            cx.set_exit(EXIT);
            return true;
        }
        let exit = ask(cx, Query::PrContextExit { name: self.name.clone() });
        if exit != 0 {
            cx.set_exit(exit);
            return true;
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.first_exit {
            self.first_exit = false;
            if ask_bool(cx, Query::PrContextExists { name: self.name.clone() }) {
                cx.host.request(Request::LeavePrContext { name: self.name.clone(), suspend: self.suspend });
            }
        }
        if ask_bool(cx, Query::PrContextBusy { name: self.name.clone() }) || !ask_bool(cx, Query::KeyboardIdle) {
            return false;
        }
        if self.restore {
            cx.host.request(Request::RestoreFocusGroup { group: self.saved_group });
        }
        true
    }
}

/// "Take Photo" (`TAKEPHOTO`, factory 0x77d4e0, vtable 0xfda3fc, 0x48 B): enter (0x741110) asks the photo object
/// (G+0x2f474) to capture ([`Request::StartPhotoCapture`], `FUN_0083bac0`); the update (0x741130) exits through
/// `EXIT` once its state (+0) is 6 ([`Query::PhotoCaptured`]).
pub struct TakePhoto;

impl Behaviour for TakePhoto {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StartPhotoCapture);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::PhotoCaptured) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Photo Complete" (`PHOTO_COMPL`, factory 0x77b1f0, vtable 0xfd93a4, 0x48 B): the update (0x7419f0) exits
/// through `EXIT` once the photo object's state (+0) is 1 ([`Query::PhotoIdle`]).
pub struct PhotoComplete;

impl Behaviour for PhotoComplete {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::PhotoIdle) {
            cx.set_exit(EXIT);
            return true;
        }
        false
    }
}

/// "Photo Listen" (`PHOTO_LISTEN`, factory 0x77b1a0, vtable 0xfd9380, 0x48 B): the enter update (0x746de0) waits
/// until the photo buttons are released ([`Query::PhotoButtonHeld`]); the update (0x746e40) waits for the photo
/// press ([`Query::PhotoTriggered`]), posts the telemetry event "GAME / EVNT / SNAP" with the photo mode
/// ([`Request::PostPhotoSnapEvent`], [`Query::PhotoMode`]) and exits through `EXIT`.
pub struct PhotoListen;

impl Behaviour for PhotoListen {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        !ask_bool(cx, Query::PhotoButtonHeld)
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::PhotoTriggered) {
            return false;
        }
        let mode = ask(cx, Query::PhotoMode) as i32;
        cx.host.request(Request::PostPhotoSnapEvent { mode });
        cx.set_exit(EXIT);
        true
    }
}

/// "Photo Input" (`PHOTO_INPUT`, factory 0x77b240, vtable 0xfd93c8, 0x48 B): enter (0x746c60) switches the photo
/// input on ([`Request::PhotoInputBegin`]); exit (0x741a30) switches it off ([`Request::PhotoInputEnd`]). The
/// update (0x741a20) returns 0: it never exits itself.
pub struct PhotoInput;

impl Behaviour for PhotoInput {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PhotoInputBegin);
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::PhotoInputEnd);
    }
}

/// The entitlement checked first (`DAT_0117a7b8` = 0x1721a).
const LIMITED_FIRST: u32 = 0x1721a;
/// The four limited-edition entitlements checked in turn (`DAT_01179fe8`, `0117a84c`, `0117aadc`, `0117a56c`).
const LIMITED_CODES: [u32; 4] = [0x2045b9, 0x2195ac, 0x2195ad, 0x2195ae];

/// "Limited Edition ..." (`LIMITEDEDSP`, factory 0x7800c0, vtable 0xfdc234, 0xc8 B): checks the limited-edition
/// entitlements against the server in seven states (+0xc0; +0xc4 = the index of the code). Enter update (0x75aff0)
/// resets state and index and waits for the local player's online readiness ([`Query::LimitedEdReady`]: byte
/// +0x42f and `FUN_0076c000`; offline it never comes). Update (0x762650): 0 asks for the first entitlement
/// ([`Query::EntitlementOwned`] of 0x1721a and the player's byte +0x42e clear, [`Query::LimitedEdPlayerFlag`]) with a
/// request (1, polled, [`Query::LimitedEdRequest`]: an error exits through `FAILED`), else goes on to 3; 2 waits for
/// the player's +0x42e; 3 handles code `index`: not owned, or already in the player's set
/// ([`Query::LimitedEdPlayerBit`], `FUN_00768310`) skips to 6, else sends its request (4, polled; an error skips
/// to 6) and waits (5) for the player's bit; 6 goes to the next code, and after the fourth exits through `SUCCESS`.
/// The request itself ([`Request::LimitedEdSend`], the message of `FUN_007661f0`) is the engine's. The states'
/// order follows the jump table at 0x7629a0 (read from snapshot 1).
#[derive(Default)]
pub struct LimitedEdition {
    state: u32,
    index: usize,
}

impl LimitedEdition {
    /// The request of states 1 and 4 (the poll 0x4c4b30, then +0x50 == 2): `None` while pending.
    fn polled(cx: &mut Cx) -> Option<bool> {
        match ask(cx, Query::LimitedEdRequest) {
            0 => None,
            1 => Some(true),
            _ => Some(false),
        }
    }

    /// 0x76294c: state 6, past the last code the machine succeeds.
    fn advance(&mut self, cx: &mut Cx) -> bool {
        self.state = 6;
        if self.index >= 3 {
            cx.set_exit(key("SUCCESS"));
            return true;
        }
        self.index += 1;
        self.state = 3;
        false
    }

    /// 0x7627b4: the body of state 3.
    fn code(&mut self, cx: &mut Cx) -> bool {
        self.state = 3;
        let id = LIMITED_CODES[self.index.min(3)];
        let owned = ask_bool(cx, Query::EntitlementOwned { id });
        if !owned || ask_bool(cx, Query::LimitedEdPlayerBit { index: self.index as u32 }) {
            self.state = 6;
            return false;
        }
        cx.host.request(Request::LimitedEdSend { stage: self.index as i32 });
        self.state = 4;
        self.poll_code(cx)
    }

    /// 0x7628ea: state 4, the request of the code.
    fn poll_code(&mut self, cx: &mut Cx) -> bool {
        self.state = 4;
        match Self::polled(cx) {
            None => false,
            Some(false) => {
                self.state = 6;
                false
            }
            Some(true) => self.wait_bit(cx),
        }
    }

    /// 0x762927 / 0x762930: state 5, waits for the player's bit of the code.
    fn wait_bit(&mut self, cx: &mut Cx) -> bool {
        self.state = 5;
        if !ask_bool(cx, Query::LimitedEdPlayerBit { index: self.index as u32 }) {
            return false;
        }
        self.advance(cx)
    }

    /// 0x762749: state 1, the first request.
    fn poll_first(&mut self, cx: &mut Cx) -> bool {
        self.state = 1;
        match Self::polled(cx) {
            None => false,
            Some(false) => {
                cx.set_exit(key("FAILED"));
                true
            }
            Some(true) => self.wait_flag(cx),
        }
    }

    /// 0x76279d: state 2, waits for the player's byte +0x42e, then handles the first code in the same update.
    fn wait_flag(&mut self, cx: &mut Cx) -> bool {
        self.state = 2;
        if !ask_bool(cx, Query::LimitedEdPlayerFlag) {
            return false;
        }
        self.code(cx)
    }
}

impl Behaviour for LimitedEdition {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        self.state = 0;
        self.index = 0;
        ask_bool(cx, Query::LimitedEdReady)
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.state {
            0 => {
                let first = ask_bool(cx, Query::EntitlementOwned { id: LIMITED_FIRST });
                if !first || ask_bool(cx, Query::LimitedEdPlayerFlag) {
                    self.state = 3;
                    return false;
                }
                cx.host.request(Request::LimitedEdSend { stage: -1 });
                self.poll_first(cx)
            }
            1 => self.poll_first(cx),
            2 => self.wait_flag(cx),
            3 => self.code(cx),
            4 => self.poll_code(cx),
            5 => self.wait_bit(cx),
            6 => self.advance(cx),
            _ => false,
        }
    }
}
