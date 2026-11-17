//! HSM hooks of the UI manager (UIM = G+0x2f480): HUD message queues, sequences, speech, music, the loading
//! screen and Easydrive's sounds. Ported from the handlers (addresses in the docs; decomp.md "Native registries:
//! HSM hooks").

use super::{LogicState, Registry, Request};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;

type Args<'a> = Option<&'a Json>;

/// Hooks ported from their handlers by an earlier, unfinished pass: not verified against the dump yet, so not
/// part of [`Registry::original`]; tests install them with [`Registry::original_with_parked`].
pub fn register_parked(r: &mut Registry) {
    r.set_hook("ClearHudMessageQueues", clear_hud_message_queues);
    r.set_hook("ClearHudInstantQueue", clear_hud_instant_queue);
    r.set_hook("StopAllSequences", stop_all_sequences);
    r.set_hook("StopSpeech", stop_speech);
    r.set_hook("SkipCurrentTrack", skip_current_track);
    r.set_hook("ToggleLoadingScreenOff", toggle_loading_screen_off);
    r.set_hook("OnOpenEasyDrive", on_open_easydrive);
    r.set_hook("OnStartEvent", on_start_event);
    r.set_hook("OnSelectEasyDriveOption", on_select_easydrive_option);
    r.set_hook("OnOpenEasyDriveMods", on_open_easydrive_mods);
    r.set_hook("OnCloseEasyDriveMods", on_close_easydrive_mods);
}

/// `ClearHudMessageQueues` (0x680460) → `FUN_0067f8c0`.
fn clear_hud_message_queues(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearHudMessageQueues);
    EXIT
}

/// `ClearHudInstantQueue` (0x67a7b0): frees the message at UIM+0x126e8 and zeroes UIM+0x126e0.
fn clear_hud_instant_queue(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearHudInstantQueue);
    EXIT
}

/// `StopAllSequences` (0x680480): `FUN_0067f8c0`, then `FUN_00a44dd0` on both sequence players UIM+0x47748 and
/// +0x4774c.
fn stop_all_sequences(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearHudMessageQueues);
    s.requests.push(Request::StopAllSequences);
    EXIT
}

/// `StopSpeech` (0x676100) → `FUN_008423a0` on the speech manager G+0x2c3c0.
fn stop_speech(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::StopSpeech);
    EXIT
}

/// `SkipCurrentTrack` (0x676b40) → `FUN_0079f760` on the song manager UIM+0x30370.
fn skip_current_track(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SkipCurrentTrack);
    EXIT
}

/// `ToggleLoadingScreenOff` (0x677010): clears the loading-screen request bit 5 (UIM+0x2d204, `HSB_TOG_LS`'s
/// reason) and UIM+0x2d21c.
fn toggle_loading_screen_off(s: &mut LogicState, _: Args) -> Key {
    s.game.loading_screen_requests &= !0x20;
    s.game.loading_screen_2d21c = 0;
    EXIT
}

fn ui_sound(s: &mut LogicState, field: u32) -> Key {
    s.requests.push(Request::PlayUiSound { field });
    EXIT
}

/// `OnOpenEasyDrive` (0x6877b0): the sound at +0x50.
fn on_open_easydrive(s: &mut LogicState, _: Args) -> Key {
    ui_sound(s, 0x50)
}

/// `OnStartEvent` (0x687830): +0x60.
fn on_start_event(s: &mut LogicState, _: Args) -> Key {
    ui_sound(s, 0x60)
}

/// `OnSelectEasyDriveOption` (0x6878b0): +0x58.
fn on_select_easydrive_option(s: &mut LogicState, _: Args) -> Key {
    ui_sound(s, 0x58)
}

/// `OnOpenEasyDriveMods` (0x687930): +0x54.
fn on_open_easydrive_mods(s: &mut LogicState, _: Args) -> Key {
    ui_sound(s, 0x54)
}

/// `OnCloseEasyDriveMods` (0x6879b0): +0x4c.
fn on_close_easydrive_mods(s: &mut LogicState, _: Args) -> Key {
    ui_sound(s, 0x4c)
}
