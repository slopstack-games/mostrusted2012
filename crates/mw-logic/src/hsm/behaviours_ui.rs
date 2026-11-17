//! Behaviour types of the UI flow: PageRunner2 contexts, widget groups, the press-start binding, the black
//! background, frame-rate mode and mute reasons. What the UI engine does is asked for with
//! [`Request`]s; what the behaviours wait on is read with [`Query`]s (both name the original's code).

use super::{Query, ask, ask_bool, binding_or_param, param_true};
use crate::hsm::{Behaviour, Cx, EXIT, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("RUN_PG_RNNR2", |_| RunPageContext::default());
    r.register_fn("ACTI_WDGT_GR", |_| ActivateWidgetGroup::default());
    r.register_fn("HSB_P1BIND", |_| BindPrimaryUser::default());
    r.register_fn("SETBLACKBACK", |_| SetBlackBackground);
    r.register_fn("CHANGEVSYNCS", |_| ChangeFrameRate::default());
    r.register_fn("MUTE_SOUND", |_| MuteSound::default());
}

/// "Run PageRunner2 Context" (`RUN_PG_RNNR2`, factory 0x77ee00, vtable 0xfdaf44, 0x58 B): on enter (0x76f7f0)
/// pushes the context `CONTEXT_NAME` with the page `DEF_PAGE_REF` (saving the focus group first with
/// `RESTOREGRP`); enters once the context exists (0x76ea90); exits through `EXIT` when the context is gone, or
/// through the exit point its pages asked for (0x76eac0); when exiting (0x76eb50) suspends (`SUSPEND`) or pops it
/// and finishes once it is no longer busy and the keyboard is idle (unless `IGNORE_KB`), restoring the focus
/// group.
#[derive(Default)]
pub struct RunPageContext {
    name: String,
    suspend: bool,
    ignore_keyboard: bool,
    restore_group: bool,
    saved_group: u64,
    first_exit: bool,
}

impl Behaviour for RunPageContext {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.first_exit = true;
        self.name = cx.param_text("CONTEXT_NAME");
        self.suspend = param_true(cx, "SUSPEND");
        self.ignore_keyboard = param_true(cx, "IGNORE_KB");
        let page = super::param_int(cx, "DEF_PAGE_REF");
        // RESTOREGRP is read only when the state has it (`FUN_007748e0`); a missing one is false either way.
        self.restore_group = param_true(cx, "RESTOREGRP");
        self.saved_group = if self.restore_group { ask(cx, Query::FocusGroup) } else { 0 };
        cx.host.request(Request::PushPageContext { name: self.name.clone(), page: format!("{page}") });
    }

    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::PageContextExists { name: self.name.clone() })
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::PageContextExists { name: self.name.clone() }) {
            cx.set_exit(EXIT);
            return true;
        }
        let exit = ask(cx, Query::PageContextExit { name: self.name.clone() });
        if exit != 0 {
            cx.set_exit(exit);
            return true;
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.first_exit {
            self.first_exit = false;
            cx.host.request(Request::LeavePageContext { name: self.name.clone(), suspend: self.suspend });
        }
        if ask_bool(cx, Query::PageContextBusy { name: self.name.clone() }) {
            return false;
        }
        if !self.ignore_keyboard && !ask_bool(cx, Query::KeyboardIdle) {
            return false;
        }
        if self.restore_group {
            cx.host.request(Request::RestoreFocusGroup { group: self.saved_group });
        }
        true
    }
}

/// `ONRESUME`: the exit point an `ACTI_WDGT_GR` takes when its screen comes back (`DAT_0117b070`).
pub const ONRESUME: u64 = 0xa10e_1091_c332_9000;

/// "Activate Widget Group" (`ACTI_WDGT_GR`, factory 0x77ee50, vtable 0xfdaf68, 0x60 B; the type of 529 states):
/// the group is the `BINDING`'s value, or `GROUP` (enter 0x76ff70 → 0x76f250). Update (0x76e4e0): activates the
/// group once the page is ready (only groups > 0); while active, exits through the exit point a widget asked for
/// when the parent lists it (`FUN_00774740`), clearing it; when the screen goes away, waits for it and exits
/// through `ONRESUME` if the parent lists that, else activates the group again. Exit begin (0x76e5e0) plays the
/// group's transition for the exit point; exit update (0x76ffd0) waits for it (at most 10 frames in transition
/// state 2), deactivates an activated group (instantly when `EXITBINDING` reads true or `INSTANTEXIT` is true,
/// 0x76f330), then waits for its out animation, or for the page to be idle.
#[derive(Default)]
pub struct ActivateWidgetGroup {
    group: i64,
    /// +0x56: activation pending.
    pending: bool,
    /// +0x57: the group was activated.
    activated: bool,
    /// +0x58: the screen went away.
    lost: bool,
    /// +0x59: deactivate when the transition is done.
    deactivate: bool,
    /// +0x5a: an exit transition started.
    transition: bool,
    /// +0x54: frames waited in transition state 2.
    frames: u16,
}

impl ActivateWidgetGroup {
    /// 0x76f330: `EXITBINDING` read as a bool when it is a non-empty path, else `INSTANTEXIT` == "true".
    fn instant(cx: &mut Cx) -> bool {
        let path = cx.param_text("EXITBINDING");
        if !path.is_empty() {
            return cx.host.binding_bool(&path) == Some(true);
        }
        param_true(cx, "INSTANTEXIT")
    }
}

impl Behaviour for ActivateWidgetGroup {
    fn on_enter(&mut self, cx: &mut Cx) {
        let path = cx.param_text("BINDING");
        self.group = binding_or_param(cx, &path, "GROUP");
        *self = Self { group: self.group, pending: true, ..Self::default() };
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.lost {
            if !ask_bool(cx, Query::ScreenActive) {
                return false;
            }
            if cx.parent_has_exit(ONRESUME) {
                cx.set_exit(ONRESUME);
                return true;
            }
            self.lost = false;
            self.pending = true;
        }
        if !self.pending {
            if !ask_bool(cx, Query::ScreenActive) {
                self.lost = true;
            }
            let exit = ask(cx, Query::WidgetExit);
            if self.group > 0 && exit != 0 && cx.parent_has_exit(exit) {
                cx.set_exit(exit);
                cx.host.request(Request::ClearWidgetExit);
                return true;
            }
        } else if self.group > 0 && ask_bool(cx, Query::PageReady) {
            cx.host.request(Request::ActivateWidgetGroup { group: self.group });
            self.pending = false;
            self.activated = true;
        }
        false
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        let exit = cx.exit();
        cx.host.request(Request::PlayWidgetGroupTransition { group: self.group, exit });
        self.transition = ask_bool(cx, Query::WidgetTransitionStarted { group: self.group });
        if self.activated {
            self.deactivate = true;
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if self.transition && !ask_bool(cx, Query::TransitionFinished) {
            if ask(cx, Query::TransitionState) == 2 {
                self.frames += 1;
            }
            if self.frames < 10 {
                return false;
            }
        }
        if self.deactivate {
            let instant = Self::instant(cx);
            cx.host.request(Request::DeactivateWidgetGroup { group: self.group, instant });
            self.deactivate = false;
            return !ask_bool(cx, Query::WidgetGroupOutTime);
        }
        !ask_bool(cx, Query::PageBusy)
    }
}

/// "Press Start" / bind player 1 (`HSB_P1BIND`, factory 0x77ad40, vtable 0xfd91dc, 0x48 B): enters once the
/// primary user is unbound (0x73e730: releases it meanwhile, then removes every player); listens for the
/// binding press (`ANYBUTTON`: any button) while unbound (0x744810), then creates the local player and exits
/// through `EXIT`; finishes once the local player exists (0x7448e0), or at once (listening off) when it exits
/// otherwise.
#[derive(Default)]
pub struct BindPrimaryUser {
    releasing: bool,
    listening: bool,
}

impl BindPrimaryUser {
    fn stop_listening(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ListenForPrimaryUser { listen: false, any_button: false });
        self.listening = false;
    }
}

impl Behaviour for BindPrimaryUser {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::PrimaryUserBound) {
            // The original sets G+0x2f721 every such frame; asking once is the same.
            if !self.releasing {
                cx.host.request(Request::ReleasePrimaryUser(true));
                self.releasing = true;
            }
            return false;
        }
        cx.host.request(Request::RemovePlayers);
        cx.host.request(Request::ReleasePrimaryUser(false));
        self.releasing = false;
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::PrimaryUserBound) {
            if !self.listening {
                let any_button = param_true(cx, "ANYBUTTON");
                cx.host.request(Request::ListenForPrimaryUser { listen: true, any_button });
                self.listening = true;
            }
            return false;
        }
        self.stop_listening(cx);
        cx.host.request(Request::CreateLocalPlayer);
        cx.set_exit(EXIT);
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if cx.exit() != EXIT {
            self.stop_listening(cx);
            return true;
        }
        ask_bool(cx, Query::LocalPlayerReady)
    }
}

/// "Set Black Background" (`SETBLACKBACK`, factory 0x77b9c0, vtable 0xfd971c): sets the UI's black background to
/// `ENABLED` on enter (0x745b10) and exits through `EXIT` at once (0x7405a0).
pub struct SetBlackBackground;

impl Behaviour for SetBlackBackground {
    fn on_enter(&mut self, cx: &mut Cx) {
        let on = param_true(cx, "ENABLED");
        cx.host.request(Request::SetBlackBackground(on));
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.set_exit(EXIT);
        true
    }
}

/// "Change VSync Settings" (`CHANGEVSYNCS`, factory 0x77c9b0, vtable 0xfd9e94): sets the 60 Hz mode to
/// `FULL_RATE` on enter (0x73b980) and restores the previous value on exit (0x73b9e0); runs its children
/// meanwhile and never exits by itself.
#[derive(Default)]
pub struct ChangeFrameRate {
    saved: bool,
}

impl Behaviour for ChangeFrameRate {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.saved = ask_bool(cx, Query::FullRate);
        let on = param_true(cx, "FULL_RATE");
        cx.host.request(Request::SetFullRate(on));
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SetFullRate(self.saved));
    }
}

/// The mute reasons (`PTR_s_Unknown_010c5c28`).
pub const MUTE_REASONS: [&str; 3] = ["Unknown", "LoadingScreen", "CarSelect"];

/// "Mute Sound" (`MUTE_SOUND`, factory 0x77c0f0, vtable 0xfd9a70): counts its `MUTEREASON` (1 LoadingScreen, 2
/// CarSelect, anything else 0, compared without case) up on enter (0x73e220) and down on exit (0x73e2a0); runs
/// its children meanwhile and never exits by itself.
#[derive(Default)]
pub struct MuteSound {
    reason: u8,
}

impl Behaviour for MuteSound {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("MUTEREASON");
        self.reason = (1..MUTE_REASONS.len()).find(|&i| MUTE_REASONS[i].eq_ignore_ascii_case(&name)).unwrap_or(0) as u8;
        cx.host.request(Request::MuteSound { reason: self.reason, on: true });
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::MuteSound { reason: self.reason, on: false });
    }
}
