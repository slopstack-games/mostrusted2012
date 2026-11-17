//! HSM hooks of the easy-drive / event selection: the seen flags of the easy-drive options, the selected world,
//! the highlighted vehicle and rollout, the pending rollout, the target score and the events started from a
//! binding. Ported from the handlers (addresses in the docs; research-log 2026-11-17 "sixth 20 HSM hooks");
//! what only the engine (the profile database, the UI manager's lists) can do is a [`Request`].

use super::hook_args::{self as arg, strtol10};
use super::hooks_flow::exit_if;
use super::{LogicState, Registry, Request};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;
use std::collections::BTreeMap;

type Args<'a> = Option<&'a Json>;

/// `liOptionId` (`DAT_01177a98`, the easy-drive option hooks).
pub const LI_OPTION_ID: u32 = 0x74b4_f5f4;
/// `liSelectedWorldId` (`DAT_01177bd8`, `SetSelectedWorld`).
pub const LI_SELECTED_WORLD_ID: u32 = 0x4ee0_ce22;

/// Fields the hooks of this file touch.
#[derive(Debug, Default)]
pub struct SelectFields {
    /// The profile's "ez_options" table (`option_id` to the seen byte; `FUN_0072eb90` reads it, `FUN_007291b0`
    /// writes it). Model decision: a mirror of the rows the hooks set; the table itself is the progression
    /// database's (the runtime also gets [`Request::SetEasyDriveOptionSeen`]), and the "ez_mods" branch of
    /// `FUN_00674d20` (an id known to the vehicle list) is not mirrored.
    pub easydrive_options_seen: BTreeMap<i32, bool>,
    /// UIM+0x2c8e0 (`FUN_0078ff50` on UIM+0x1f8f0: +0xcff0 = the event, +0xd544 = 1): the event the distance is
    /// calculated to, and the request flag.
    pub distance_event: i32,
    pub distance_requested: bool,
    /// The rollout manager G+0x2f404 +0x1c4: the highlighted rollout (`SetHighlightedRolloutByIdString`).
    pub highlighted_rollout: i32,
    /// The rollout manager +0x1c8: the highlighted vehicle (`FUN_0067ef40`).
    pub highlighted_vehicle: i32,
    /// UIM+0x2c5e0 (`SetWaypointToEventFromBinding` sets it to -1).
    pub waypoint_2c5e0: i32,
}

pub fn register(r: &mut Registry) {
    r.set_hook("CancelMilestoneTracking", cancel_milestone_tracking);
    r.set_hook("HasEasyDriveOptionBeenSeen", has_easydrive_option_been_seen);
    r.set_hook("SetEasyDriveOptionSeen", set_easydrive_option_seen);
    r.set_hook("SetEasyDriveOptionSeenToTrue", set_easydrive_option_seen_to_true);
    r.set_hook("SetSelectedWorld", set_selected_world);
    r.set_hook("SetAvailableVehicleListEvent", set_available_vehicle_list_event);
    r.set_hook("CalculateDistanceToEventByIdString", calculate_distance_to_event_by_id_string);
    r.set_hook("SetRolloutFromVehicleIdString", set_rollout_from_vehicle_id_string);
    r.set_hook("SetHighlightedRolloutByIdString", set_highlighted_rollout_by_id_string);
    r.set_hook("SetHighlightedVehicleByIdString", set_highlighted_vehicle_by_id_string);
    r.set_hook("IsTargetScoreHit", is_target_score_hit);
    r.set_hook("ShowTargetScoreReminderMessage", show_target_score_reminder_message);
    r.set_hook("SelectPaintPackFromEasyDrive", select_paint_pack_from_easydrive);
    r.set_hook("UpdatePendingRollout_EasyDrive", update_pending_rollout_easydrive);
    r.set_hook("UpdatePendingRollout_EasyDrive_Stock", update_pending_rollout_easydrive_stock);
    r.set_hook("SetFinalEventData", set_final_event_data);
    r.set_hook("StartEventFromBinding", start_event_from_binding);
    r.set_hook("SetWaypointToEventFromBinding", set_waypoint_to_event_from_binding);
    r.set_hook("UpdateProgressAwardsStrings", update_progress_awards_strings);
}

/// The `lpcValue` string read as `strtol(s, NULL, 10)` (`FUN_00d751e0`), as these handlers do.
fn value_number(args: Args) -> i32 {
    strtol10(arg::string(args, arg::LPC_VALUE))
}

/// `CancelMilestoneTracking` (0x675e50): `FUN_0071d1e0` on the profile database +0x5a3c (the milestone tracking
/// fields +0x6f4..+0x718 zeroed, +0x6fc = -1, three tables' first entries cleared).
fn cancel_milestone_tracking(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::CancelMilestoneTracking);
    EXIT
}

/// `HasEasyDriveOptionBeenSeen` (0x676920): `liOptionId` → `FUN_0072eb90` on the profile database: the seen byte
/// of the option's "ez_options" row, 0 without one.
fn has_easydrive_option_been_seen(s: &mut LogicState, args: Args) -> Key {
    let id = arg::int(args, LI_OPTION_ID);
    exit_if(s.select.easydrive_options_seen.get(&id).copied().unwrap_or(false))
}

/// `SetEasyDriveOptionSeen` (0x676970): `liOptionId`, `lbSeen` → `FUN_00674d20`(id, seen): the "ez_options" row
/// (`FUN_007291b0`), or the "ez_mods" one (`FUN_007316d0`) for an id the vehicle list knows (`FUN_006ecae0`).
fn set_easydrive_option_seen(s: &mut LogicState, args: Args) -> Key {
    let option = arg::int(args, LI_OPTION_ID);
    let seen = arg::boolean(args, arg::LB_SEEN);
    s.select.easydrive_options_seen.insert(option, seen);
    s.requests.push(Request::SetEasyDriveOptionSeen { option, seen });
    EXIT
}

/// `SetEasyDriveOptionSeenToTrue` (0x6769d0): `lpcValue` → `FUN_00674d90`: with the online-event flag
/// ((GMM+4)+0xe8) set and a non-zero id, `FUN_00703c10`(id) alone; otherwise the row is set as
/// [`set_easydrive_option_seen`] does with seen = 1, then `FUN_007040b0`(id) (`FUN_006fc550`, `FUN_00702710`).
fn set_easydrive_option_seen_to_true(s: &mut LogicState, args: Args) -> Key {
    let option = value_number(args);
    let online = s.game_modes.online_event && option != 0;
    if !online {
        s.select.easydrive_options_seen.insert(option, true);
    }
    s.requests.push(Request::SetEasyDriveOptionSeenToTrue { option, online });
    EXIT
}

/// `SetSelectedWorld` (0x675ad0): `liSelectedWorldId` → `FUN_00645780` on the world list G+0x2f468.
fn set_selected_world(s: &mut LogicState, args: Args) -> Key {
    s.requests.push(Request::SetSelectedWorld { world: arg::int(args, LI_SELECTED_WORLD_ID) });
    EXIT
}

/// `SetAvailableVehicleListEvent` (0x675cc0): `liEventId` → `FUN_00871330` on (G+0x2f3e0)+0x5ee0.
fn set_available_vehicle_list_event(s: &mut LogicState, args: Args) -> Key {
    s.requests.push(Request::SetAvailableVehicleListEvent { event: arg::int(args, arg::LI_EVENT_ID) });
    EXIT
}

/// `CalculateDistanceToEventByIdString` (0x675e00): `FUN_0078ff50` on UIM+0x1f8f0: the event, request flag set.
fn calculate_distance_to_event_by_id_string(s: &mut LogicState, args: Args) -> Key {
    s.select.distance_event = value_number(args);
    s.select.distance_requested = true;
    EXIT
}

/// `SetRolloutFromVehicleIdString` (0x675ee0): a vehicle id above 0 → `FUN_006c7950`(the local player's record,
/// id, 0) (the rollout of that vehicle applied through `FUN_006c2020`).
fn set_rollout_from_vehicle_id_string(s: &mut LogicState, args: Args) -> Key {
    let vehicle = value_number(args);
    if vehicle > 0 {
        s.requests.push(Request::SetRolloutFromVehicle { vehicle });
    }
    EXIT
}

/// `SetHighlightedRolloutByIdString` (0x675f40): the rollout manager's +0x1c4.
fn set_highlighted_rollout_by_id_string(s: &mut LogicState, args: Args) -> Key {
    s.select.highlighted_rollout = value_number(args);
    EXIT
}

/// `SetHighlightedVehicleByIdString` (0x680390) → `FUN_0067ef40`: only a changed id is stored (+0x1c8); the
/// vehicle's record is then copied into +0x1cc (0x2b dwords) when the database has it, else cleared
/// (`FUN_0067e990`): [`Request::LoadHighlightedVehicle`].
fn set_highlighted_vehicle_by_id_string(s: &mut LogicState, args: Args) -> Key {
    let vehicle = value_number(args);
    if s.select.highlighted_vehicle != vehicle {
        s.select.highlighted_vehicle = vehicle;
        s.requests.push(Request::LoadHighlightedVehicle { vehicle });
    }
    EXIT
}

/// `IsTargetScoreHit` (0x6803e0): the local player's score (record +0x418) is at least the current event's
/// target score (the event record's +0xcc, behind (GMM+4)+0xf4 / +0xf8).
fn is_target_score_hit(s: &mut LogicState, _: Args) -> Key {
    let score = s.players.local_player().map_or(0, |p| p.score);
    exit_if(score >= s.game_modes.event_target_score)
}

/// `ShowTargetScoreReminderMessage` (0x680440) → `FUN_0067f7f0`: when the object at UIM+0x7230 reports
/// different values through its vtable +0x14 and +0x10, the HUD message 0x5f5ee with "score" = target - the
/// local player's score is sent ([`Request::ShowTargetScoreReminder`]).
fn show_target_score_reminder_message(s: &mut LogicState, _: Args) -> Key {
    let score = s.players.local_player().map_or(0, |p| p.score);
    s.requests.push(Request::ShowTargetScoreReminder { remaining: s.game_modes.event_target_score - score });
    EXIT
}

/// `SelectPaintPackFromEasyDrive` (0x68d3a0): `lpcValue` through `atol` → `FUN_0068d200` (the paint pack of the
/// rollout manager's local-player record; `FUN_00704e80` when its +0x34 changed).
fn select_paint_pack_from_easydrive(s: &mut LogicState, args: Args) -> Key {
    s.requests.push(Request::SelectPaintPack { pack: value_number(args) });
    EXIT
}

/// `UpdatePendingRollout_EasyDrive` (0x6903a0) → `FUN_00690030`: the vehicle id from `lpcValue` (`strtol`); the
/// pending rollout of the local player becomes that vehicle's (kind 1 or 2 from the mods it owns).
fn update_pending_rollout_easydrive(s: &mut LogicState, args: Args) -> Key {
    let vehicle = value_number(args);
    s.requests.push(Request::UpdatePendingRollout { vehicle, stock: false });
    EXIT
}

/// `UpdatePendingRollout_EasyDrive_Stock` (0x68ae60) → `FUN_0068aa40`: the UI sound at +0x64 first, then the
/// pending rollout of that vehicle with the stock kind (0).
fn update_pending_rollout_easydrive_stock(s: &mut LogicState, args: Args) -> Key {
    let vehicle = value_number(args);
    s.requests.push(Request::PlayUiSound { field: 0x64 });
    s.requests.push(Request::UpdatePendingRollout { vehicle, stock: true });
    EXIT
}

/// `SetFinalEventData` (0x6766f0): `FUN_006b9d00` on the local player's record: +0x5b8 = (GMM+0x2ca3) + 1,
/// +0x5c8 = GMM+0x2d28, +0x5cc = GMM+0x2d2c, +0x5d0 = 0.
fn set_final_event_data(s: &mut LogicState, _: Args) -> Key {
    let g = &s.game_modes;
    let (a, b, c) = (i32::from(g.byte_2ca3) + 1, g.word_2d28, g.word_2d2c);
    if let Some(p) = s.players.local_player_mut() {
        p.final_5b8 = a;
        p.final_5c8 = b;
        p.final_5cc = c;
        p.final_5d0 = false;
    }
    EXIT
}

/// `StartEventFromBinding` (0x676790) → `FUN_00674be0`: `lpcValue` is a binding (`FUN_00a50650` on G+0xe8),
/// read as an int (`FUN_00a4f220`, -1 when it does not resolve); not -1: `FUN_006e74f0`(id, 0).
fn start_event_from_binding(s: &mut LogicState, args: Args) -> Key {
    let event = binding_event(s, args);
    if event != -1 {
        s.game_modes.request_event(event as u32, false);
    }
    EXIT
}

/// `SetWaypointToEventFromBinding` (0x6767d0) → `FUN_00674c60`: the event of the binding as above; not -1:
/// `FUN_00792390`, `FUN_00796ff0`(event) (the waypoint to the event's start) and UIM+0x2c5e0 = -1.
fn set_waypoint_to_event_from_binding(s: &mut LogicState, args: Args) -> Key {
    let event = binding_event(s, args);
    if event != -1 {
        s.requests.push(Request::SetWaypointToEvent { event });
        s.select.waypoint_2c5e0 = -1;
    }
    EXIT
}

fn binding_event(s: &LogicState, args: Args) -> i32 {
    // The out value starts at -1 and is written only by a binding that resolves.
    match crate::binding::Binding::parse(arg::string(args, arg::LPC_VALUE)) {
        Ok(b) => {
            let r = b.eval_int(s);
            if r.ok { r.value } else { -1 }
        }
        Err(_) => -1,
    }
}

/// `UpdateProgressAwardsStrings` (0x6768c0): `FUN_00724680` (`FUN_0071fd20`(-1, +0x10e8)) and `FUN_00724690`
/// (`FUN_0071fd20`(6, +0x12e8)) on the progress-awards object G+0x2f3f8.
fn update_progress_awards_strings(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::UpdateProgressAwardsStrings);
    EXIT
}
