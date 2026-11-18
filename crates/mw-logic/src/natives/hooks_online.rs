//! The eighth twenty HSM hooks (research-log 2026-11-17 "eighth 20 HSM hooks"): the recommended events of the
//! online speedwall, the platform sign-in, the DLC message indicator, the store overlay, the thank-you queue and
//! a few small online ones. Handlers read from the snapshot's disassembly; the recommendations live in the
//! "Recommends" document the online service delivers (offline: none), so what only the engine can do is a
//! [`Request`] and the state the handlers read is a field the runtime fills.

use super::hook_args::{self as arg, strtol10};
use super::hooks_flow::exit_if;
use super::{LogicState, Registry, Request, lua_menus};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;
use std::collections::BTreeSet;

type Args<'a> = Option<&'a Json>;

/// The integer member of `SetDlcMessageIndicator` (`DAT_01177fac` = 0x298c2ce3; name not known, hash read from the
/// snapshot's `.bss`, not found among the names tried).
pub const DLC_INDICATOR_VALUE: u32 = 0x298c_2ce3;

/// The recommendation manager (G+0x2f454, +0x5a3c) and the other objects the hooks of this file touch.
#[derive(Debug)]
pub struct HookOnlineFields {
    /// `FUN_0081e130("Recommends")` found the document and its "HasRecommendations" is true: the guard of every
    /// recommendation function (`FUN_00725130`, `FUN_00723300`, `FUN_00724ef0`, `FUN_00723b10`). Filled by the
    /// runtime; false offline.
    pub has_recommendations: bool,
    /// The "Items" list of the document: each item's "EventId" (`FUN_00723300` reads it by index). Runtime.
    pub event_ids: Vec<i32>,
    /// +0xa0 / +0xa4: the item index and parameter of the last target / selection (-1: none).
    pub target_index: i32,
    pub target_param: i32,
    /// +0x9c: the index the "current" event is (`SetCurrentRecommendedEvent` copies +0xa0 into it).
    pub current_index: i32,
    /// The event id `SetCarListForRecommendedEvent` handed to the pack page's car list (`FUN_00871330`).
    pub car_list_event: i32,
    /// `FUN_006e0440` (the byte +0x2e91 or +0x2e92 of the game state): while set, the event 0x1d21f2 is not
    /// selectable by id. The object it is read from is not clear from the call (ecx is not set before it): model
    /// decision, a field the runtime sets, false by default.
    pub special_event_locked: bool,
    /// `FUN_006e5ea0(event)`: the event is one of the two the race manager already runs (+0xd8, +0x130 of the
    /// player's race state); such an event is not selectable by id. Runtime.
    pub events_in_progress: BTreeSet<i32>,
    /// (G+0x2f3fc)+0x2ca5: the post-event recommendation is wanted (`FUN_0071d040`).
    pub post_event_wanted: bool,
    /// `FUN_006e0950` / `FUN_006e0970` (+0x1974 / +0x1978 of the game state): events done / events recommended.
    pub events_done: i32,
    pub events_total: i32,
    /// +0x2e4: `GenerateRecomendationPostEventString` result: the post-event recommendation string is shown.
    pub post_event_string: bool,
    /// +0x2e8: the handle of the autolog alert (a sound / UI object; 0 = none) and +0xd0 its "active" word.
    pub autolog_alert_handle: u32,
    pub autolog_alert_active: bool,
    /// (G+0x2f404)+0x8f9c: the vehicle class the Kinect voice selection holds (-1 = none).
    pub kinect_selected_class: i32,
    /// The "CachedMWL" lock byte (+0x36 of (G+0x2f454)+0x4af0): set by `CacheMwlAndLock`, cleared by `UnlockMwlCache`.
    pub mwl_cache_locked: bool,
    /// `FUN_00ae3740`(2, ...) answers "true": the Origin in-game overlay is available (the platform's query, key
    /// 2). Runtime / platform; false offline.
    pub origin_igo_available: bool,
}

impl Default for HookOnlineFields {
    fn default() -> Self {
        HookOnlineFields {
            has_recommendations: false,
            event_ids: Vec::new(),
            target_index: -1,
            target_param: 0,
            current_index: -1,
            car_list_event: 0,
            special_event_locked: false,
            events_in_progress: BTreeSet::new(),
            post_event_wanted: false,
            events_done: 0,
            events_total: 0,
            post_event_string: false,
            autolog_alert_handle: 0,
            autolog_alert_active: false,
            kinect_selected_class: 0,
            mwl_cache_locked: false,
            origin_igo_available: false,
        }
    }
}

pub fn register(r: &mut Registry) {
    r.set_hook("PopThankYou", pop_thank_you);
    r.set_hook("StopAutologAlert", stop_autolog_alert);
    r.set_hook("SetDlcMessageIndicator", set_dlc_message_indicator);
    r.set_hook("OpenStoreOverlay", open_store_overlay);
    r.set_hook("RemoveOnlineSeenUnlocksMenu", remove_online_seen_unlocks_menu);
    r.set_hook("RefreshTopOfSpeedwallViewModel", refresh_top_of_speedwall_view_model);
    r.set_hook("ResetPlaylistScoreState", reset_playlist_score_state);
    r.set_hook("InvalidateKinectSelectedClass", invalidate_kinect_selected_class);
    r.set_hook("IsOriginIgoAvailable", is_origin_igo_available);
    r.set_hook("SignInStarted", sign_in_started);
    r.set_hook("SilentSignInStarted", silent_sign_in_started);
    r.set_hook("SignInFinished", sign_in_finished);
    r.set_hook("SetTargetRecommendedEvent", set_target_recommended_event);
    r.set_hook("SetCurrentRecommendedEvent", set_current_recommended_event);
    r.set_hook("SetCarListForRecommendedEvent", set_car_list_for_recommended_event);
    r.set_hook("SelectRecommendationDomByIndexString", select_recommendation_dom_by_index_string);
    r.set_hook("SelectRecommendationDomByEventIdString", select_recommendation_dom_by_event_id_string);
    r.set_hook("RefreshRecommendations", refresh_recommendations);
    r.set_hook("GenerateRecomendationPostEventString", generate_recomendation_post_event_string);
    r.set_hook("CacheMwlAndLock", cache_mwl_and_lock);
}

/// `PopThankYou` (0x676840): `FUN_0063cdb0` on the entitlement manager (G+0x29280): `ThankYouCount` (+0x9d4) is
/// decremented and the queue shifts. Model decision: nothing happens at a count of 0 (the original would count
/// below 0 and loop over the queue; the data never pops an empty one).
fn pop_thank_you(s: &mut LogicState, _: Args) -> Key {
    if s.entitlements.thank_you_count > 0 {
        s.entitlements.thank_you_count -= 1;
        s.requests.push(Request::PopThankYou);
    }
    EXIT
}

/// `StopAutologAlert` (0x676860): `FUN_0071d190`: the "active" word +0xd0 = 0, and `FUN_00a43150` on the handle
/// +0x2e8 when there is one.
fn stop_autolog_alert(s: &mut LogicState, _: Args) -> Key {
    let f = &mut s.hook_online;
    f.autolog_alert_active = false;
    if f.autolog_alert_handle != 0 {
        s.requests.push(Request::StopAutologAlert);
    }
    EXIT
}

/// `SetDlcMessageIndicator` (0x677110): the int member → `FUN_0063a500` on the entitlement manager: the profile
/// value 0x11 (`FUN_0069fce0`, G+0x2f454 + 0x4964) and the "last_played" row (`FUN_0072b120`); the manager's
/// `DlcMessageIndicator`.
fn set_dlc_message_indicator(s: &mut LogicState, args: Args) -> Key {
    s.entitlements.dlc_message_indicator = arg::int(args, DLC_INDICATOR_VALUE);
    EXIT
}

/// `OpenStoreOverlay` (0x677150): `FUN_00655fa0` on G+0x2f438 (the platform call, engine).
fn open_store_overlay(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::OpenStoreOverlay);
    EXIT
}

/// `RemoveOnlineSeenUnlocksMenu` (0x676d70): looks one member up (`DAT_01177c10`, the result is not used), then
/// `FUN_006f44e0` on G+0x14c08: the code of `ClearOnlineEasyDrive`.
fn remove_online_seen_unlocks_menu(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ClearOnlineEasyDrive);
    EXIT
}

/// `RefreshTopOfSpeedwallViewModel` (0x6768f0): `FUN_007357f0`(0) on (G+0x2f454)+0x6c8c.
fn refresh_top_of_speedwall_view_model(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::RefreshTopOfSpeedwall);
    EXIT
}

/// `ResetPlaylistScoreState` (0x676ac0): `FUN_006e32a0` and `FUN_006bf460`.
fn reset_playlist_score_state(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ResetPlaylistScoreState);
    EXIT
}

/// `InvalidateKinectSelectedClass` (0x676d00): (G+0x2f404)+0x8f9c = -1.
fn invalidate_kinect_selected_class(s: &mut LogicState, _: Args) -> Key {
    s.hook_online.kinect_selected_class = -1;
    EXIT
}

/// `IsOriginIgoAvailable` (0x7eb760): the platform's value for key 2 (`FUN_00ae3740`) hashed (`FUN_009ed180`) and
/// compared with the hash of "true": TRUE when equal, else FALSE (the handler returns copies of the two values).
fn is_origin_igo_available(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.hook_online.origin_igo_available)
}

/// `SignInStarted` (0x7eb520): `FUN_00881f10(1)` on OM+0x10d0, the Lua native's code.
fn sign_in_started(s: &mut LogicState, _: Args) -> Key {
    lua_menus::start_sign_in(s, true);
    EXIT
}

/// `SilentSignInStarted` (0x7eb550): `FUN_00881f10(0)`: the same without the flag at +0x158a.
fn silent_sign_in_started(s: &mut LogicState, _: Args) -> Key {
    lua_menus::start_sign_in(s, false);
    EXIT
}

/// `SignInFinished` (0x7eb580): `FUN_00886f00`, the Lua native's code.
fn sign_in_finished(s: &mut LogicState, _: Args) -> Key {
    lua_menus::finish_sign_in(s);
    EXIT
}

/// `SetTargetRecommendedEvent` (0x675bb0): `FUN_00725130`(`strtol(lpcValue)`, 0, 0): TRUE when the recommendations
/// exist (the item becomes the target), else FALSE.
fn set_target_recommended_event(s: &mut LogicState, args: Args) -> Key {
    let index = strtol10(arg::string(args, arg::LPC_VALUE));
    let f = &mut s.hook_online;
    if !f.has_recommendations {
        return exit_if(false);
    }
    f.target_index = index;
    f.target_param = 0;
    s.requests.push(Request::SetRecommendedEvent { index, param: 0 });
    exit_if(true)
}

/// `SetCurrentRecommendedEvent` (0x675c20): `FUN_00726c80`: +0x9c = +0xa0, then `FUN_00725130`(+0xa0, +0xa4, 0).
fn set_current_recommended_event(s: &mut LogicState, _: Args) -> Key {
    let f = &mut s.hook_online;
    f.current_index = f.target_index;
    if f.has_recommendations {
        let (index, param) = (f.target_index, f.target_param);
        s.requests.push(Request::SetRecommendedEvent { index, param });
    }
    EXIT
}

/// `SetCarListForRecommendedEvent` (0x675c50): `strtol(lpcValue)` above 0 and `FUN_00723300`: the "EventId" of
/// `Items[n]` (0 when missing) → `FUN_00871330`; TRUE when it ran, else FALSE.
fn set_car_list_for_recommended_event(s: &mut LogicState, args: Args) -> Key {
    let n = strtol10(arg::string(args, arg::LPC_VALUE));
    let f = &mut s.hook_online;
    if n <= 0 || !f.has_recommendations {
        return exit_if(false);
    }
    let event_id = f.event_ids.get(n as usize).copied().unwrap_or(0);
    f.car_list_event = event_id;
    s.requests.push(Request::SetCarListForEvent { event_id });
    exit_if(true)
}

/// `FUN_00724ef0`(index, param): nothing without recommendations or for -1; else the DOM object of the item is
/// selected and +0xa0 / +0xa4 are kept.
pub(super) fn select_dom(s: &mut LogicState, index: i32, param: i32) {
    let f = &mut s.hook_online;
    if !f.has_recommendations || index == -1 {
        return;
    }
    f.target_index = index;
    f.target_param = param;
    s.requests.push(Request::SelectRecommendationDom { index, param });
}

/// `SelectRecommendationDomByIndexString` (0x675d10): `strtol(lpcValue)` → `FUN_00724ef0`(index, 0).
fn select_recommendation_dom_by_index_string(s: &mut LogicState, args: Args) -> Key {
    let index = strtol10(arg::string(args, arg::LPC_VALUE));
    select_dom(s, index, 0);
    EXIT
}

/// The event whose id `FUN_00723b10` special-cases (it is refused while `FUN_006e0440`).
const SPECIAL_EVENT: i32 = 0x1d21f2;

/// `SelectRecommendationDomByEventIdString` (0x675d70): `FUN_00723b10`(`strtol(lpcValue)`, &index, 0) finds the
/// item with that "EventId" (refused for the special event while it is locked, and for an event the race manager
/// already runs); found → `FUN_00724ef0`(index, 0), else `FUN_0071d270` releases the selected DOM objects.
/// The index `FUN_00723b10` passes on is the item's position counted from 1 (`uVar5` is incremented before it is
/// stored): kept as read, not confirmed against the data (open).
fn select_recommendation_dom_by_event_id_string(s: &mut LogicState, args: Args) -> Key {
    let event = strtol10(arg::string(args, arg::LPC_VALUE));
    let f = &s.hook_online;
    let found = if (event == SPECIAL_EVENT && f.special_event_locked)
        || f.events_in_progress.contains(&event)
        || !f.has_recommendations
    {
        None
    } else {
        f.event_ids.iter().position(|&e| e == event).map(|p| p as i32 + 1)
    };
    match found {
        Some(index) => select_dom(s, index, 0),
        None => s.requests.push(Request::ReleaseRecommendationDom),
    }
    EXIT
}

/// `RefreshRecommendations` (0x676810): `FUN_007355b0`(0, 0) on (G+0x2f454)+0x5a3c.
fn refresh_recommendations(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::RefreshRecommendations);
    EXIT
}

/// `GenerateRecomendationPostEventString` (0x676890): `FUN_0071d040`: +0x2e4 = the post-event flag
/// ((G+0x2f3fc)+0x2ca5) and events done (+0x1974) < events total (+0x1978).
fn generate_recomendation_post_event_string(s: &mut LogicState, _: Args) -> Key {
    let f = &mut s.hook_online;
    f.post_event_string = f.post_event_wanted && f.events_done < f.events_total;
    EXIT
}

/// `CacheMwlAndLock` (0x676dc0): `FUN_0071caa0`: "CachedMWL" is published as "OldCachedMWL", lock +0x36 = 1.
fn cache_mwl_and_lock(s: &mut LogicState, _: Args) -> Key {
    s.hook_online.mwl_cache_locked = true;
    s.requests.push(Request::CacheMwl);
    EXIT
}
