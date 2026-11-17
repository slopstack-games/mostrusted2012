//! HSM hooks of the game modes and events: the mode timer, requesting / retrying events, the post-event awards,
//! the racers and cops, respawns, heat and damage. Ported from the handlers (addresses in the docs; decomp.md
//! "Native registries: HSM hooks"); what only the engine can do is a [`Request`].

use super::game_modes::set_timer_running;
use super::hook_args::{self as arg};
use super::hooks_flow::exit_if;
use super::players::{FIRST_AI_SLOT, SLOTS};
use super::progression::LAST_EVENT;
use super::{LogicState, Registry, Request};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;

type Args<'a> = Option<&'a Json>;

pub fn register(r: &mut Registry) {
    r.set_hook("StartGameModeTimer", start_game_mode_timer);
    r.set_hook("StopGameModeTimer", stop_game_mode_timer);
    r.set_hook("ResetGameMode", reset_game_mode);
    r.set_hook("IsOnlineEvent", is_online_event);
    r.set_hook("ShouldShowEscapeSequence", should_show_escape_sequence);
    r.set_hook("RetryLastEvent", retry_last_event);
    r.set_hook("FixUpRecommendationRetry", fix_up_recommendation_retry);
    r.set_hook("StartEventFromEasyDrive", start_event_from_easydrive);
    r.set_hook("SetEventFromAvailableVehicles", set_event_from_available_vehicles);
    r.set_hook("ResetCurrentEventStats", reset_current_event_stats);
    r.set_hook("SetTimeOfDayFromEvent", set_time_of_day_from_event);
    r.set_hook("SyncTimeOfDay", sync_time_of_day);
    r.set_hook("PreparePostEventAwards", prepare_post_event_awards);
    r.set_hook("AwardPostEventAwards", award_post_event_awards);
    r.set_hook("ClearPostEventAwards", clear_post_event_awards);
    r.set_hook("AddAllAIPlayers", add_all_ai_players);
    r.set_hook("AreAllAIPlayersLoaded", are_all_ai_players_loaded);
    r.set_hook("RemoveAllRaceCars", remove_all_race_cars);
    r.set_hook("RemoveAllCops", remove_all_cops);
    r.set_hook("RemoveRoadblocks", remove_roadblocks);
    r.set_hook("ResetProps", reset_props);
    r.set_hook("RequestRespawn", request_respawn);
    r.set_hook("RequestRespawnAtEventStart", request_respawn_at_event_start);
    r.set_hook("RequestRespawnToLastSwapSpot", request_respawn_to_last_swap_spot);
    r.set_hook("ResetChaseStats", reset_chase_stats);
    r.set_hook("ResetLocalPlayerHeat", reset_local_player_heat);
    r.set_hook("TransferVehicleHeatToPlayer", transfer_vehicle_heat_to_player);
    r.set_hook("HasLocalPlayerSetRollout", has_local_player_set_rollout);
    r.set_hook("ResetLocalPlayerDamage", reset_local_player_damage);
    r.set_hook("DeactivateDamage", deactivate_damage);
    r.set_hook("ReactivateDamage", reactivate_damage);
    r.set_hook("CleanAndRepairVehicle", clean_and_repair_vehicle);
    r.set_hook("ClearNitrous", clear_nitrous);
}

/// `StartGameModeTimer` (0x675ea0): `FUN_006f4650`(GMM, 1).
fn start_game_mode_timer(s: &mut LogicState, _: Args) -> Key {
    set_timer_running(s, true);
    EXIT
}

/// `StopGameModeTimer` (0x675ec0): `FUN_006f4650`(GMM, 0).
fn stop_game_mode_timer(s: &mut LogicState, _: Args) -> Key {
    set_timer_running(s, false);
    EXIT
}

/// `ResetGameMode` (0x687730) → `FUN_00687240`: a telemetry record ("GAME" "EVNT" "RSTR": the event id, the race
/// clock GMM+0x2d28 in seconds, the progress, the car's position; not sent here, model decision), then
/// GMM+0x2e7f = 0 and `FUN_006f89f0` (the mode's reset, [`Request::ResetGameMode`]).
fn reset_game_mode(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.has_transitioned_events = false;
    s.requests.push(Request::ResetGameMode);
    EXIT
}

/// `IsOnlineEvent` (0x676af0): `FUN_006dfed0`, (GMM+4)+0xe8.
fn is_online_event(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.online_event)
}

/// `ShouldShowEscapeSequence` (0x6805b0): a free-drive event (GMM+0x2e90) or the current game mode is
/// `GameLogicConfig` +0x60.
fn should_show_escape_sequence(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.game_modes.freedrive_event || s.game_modes.current_mode == s.config.escape_mode)
}

/// `RetryLastEvent` (0x676240): `FUN_006e74f0`(GMM, `last_played`[1] (its low 32 bits), 0).
fn retry_last_event(s: &mut LogicState, _: Args) -> Key {
    let last = s.progression.last_played(LAST_EVENT) as u32;
    s.game_modes.request_event(last, false);
    EXIT
}

/// `FixUpRecommendationRetry` (0x676280) → `FUN_006e04d0`: in a free-drive event, `FUN_00726ca0`(`last_played`[1])
/// on the recommendations.
fn fix_up_recommendation_retry(s: &mut LogicState, _: Args) -> Key {
    if s.game_modes.freedrive_event {
        let event = s.progression.last_played(LAST_EVENT) as u32;
        s.requests.push(Request::RecommendationRetry { event });
    }
    EXIT
}

/// `StartEventFromEasyDrive` (0x676740): `lpcValue` as a decimal id; unless −1, `FUN_006e74f0`(GMM, id, 0).
fn start_event_from_easydrive(s: &mut LogicState, args: Args) -> Key {
    let id = arg::strtol10(arg::string(args, arg::LPC_VALUE));
    if id != -1 {
        s.game_modes.request_event(id as u32, false);
    }
    EXIT
}

/// `SetEventFromAvailableVehicles` (0x675e80) → `FUN_00673d70`: the recommendations' event for the available
/// vehicles ([`Request::SetEventFromAvailableVehicles`]).
fn set_event_from_available_vehicles(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetEventFromAvailableVehicles);
    EXIT
}

/// `ResetCurrentEventStats` (0x680510) → `FUN_0067f700` ([`Request::ResetCurrentEventStats`]).
fn reset_current_event_stats(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ResetCurrentEventStats {
        event: s.game_modes.current_event,
        mode: s.game_modes.current_mode,
        online: s.game_modes.online_event,
    });
    EXIT
}

/// `SetTimeOfDayFromEvent` (0x680370) → `FUN_0067f570`.
fn set_time_of_day_from_event(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetTimeOfDayFromEvent);
    EXIT
}

/// `SyncTimeOfDay` (0x676e60) → `FUN_006f4960`.
fn sync_time_of_day(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SyncTimeOfDay);
    EXIT
}

/// `PreparePostEventAwards` (0x675fb0) → `FUN_006fbb60` ([`Request::PreparePostEventAwards`]).
fn prepare_post_event_awards(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::PreparePostEventAwards);
    EXIT
}

/// `AwardPostEventAwards` (0x675fd0) → `FUN_006e65e0`: hands the GMM's award fields to the progression
/// ([`Request::AwardPostEventAwards`]).
fn award_post_event_awards(s: &mut LogicState, _: Args) -> Key {
    let g = &s.game_modes;
    s.requests.push(Request::AwardPostEventAwards {
        event: g.current_event,
        amount: g.award_amount,
        award_2ce8: g.award_2ce8,
        awards: g.awards.clone(),
    });
    EXIT
}

/// `ClearPostEventAwards` (0x676ea0) → `FUN_006e6670`.
fn clear_post_event_awards(s: &mut LogicState, _: Args) -> Key {
    s.game_modes.clear_post_event_awards();
    EXIT
}

/// `AddAllAIPlayers` (0x687710) → `FUN_00686f30`.
fn add_all_ai_players(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::AddAllAIPlayers);
    EXIT
}

/// `AreAllAIPlayersLoaded` (0x67a710) → `FUN_00679600`: every occupied slot 12..23 is loaded (`FUN_006ba2f0`).
fn are_all_ai_players_loaded(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.players.slots[FIRST_AI_SLOT..].iter().flatten().all(|p| p.loaded))
}

/// `RemoveAllRaceCars` (0x675f90) → `FUN_006d1310` on PM.
fn remove_all_race_cars(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::RemoveAllRaceCars);
    EXIT
}

/// `RemoveAllCops` (0x67a740) → `FUN_006795c0`: every occupied slot whose player is AI-driven (+0x9c = 2) and a
/// cop (`FUN_006bf190`) is removed, `FUN_006ce180`(slot, 3).
fn remove_all_cops(s: &mut LogicState, _: Args) -> Key {
    for slot in 0..SLOTS {
        if s.players.slots[slot].as_ref().is_some_and(|p| p.kind == 2 && p.cop) {
            s.requests.push(Request::RemovePlayer { slot, reason: 3 });
        }
    }
    EXIT
}

/// `RemoveRoadblocks` (0x675b90) → `FUN_008456d0` on the roadblock manager G+0x2f484.
fn remove_roadblocks(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::RemoveRoadblocks);
    EXIT
}

/// `ResetProps` (0x675b70) → `FUN_00846830` on G+0x2ef6c: posts message 0x15.
fn reset_props(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ResetProps);
    EXIT
}

/// The respawn of the local player's record (`FUN_006cd340` / `FUN_006cd380` / `FUN_006cd3a0`): its target event
/// +0x2c4 (and +0x435), then `FUN_006c9fb0`(0, 0).
fn respawn(s: &mut LogicState, event: u32, to_swap_spot: bool) {
    if let Some(p) = s.players.local_player_mut() {
        p.respawn_event = event;
        if to_swap_spot {
            p.respawn_to_swap_spot = true;
        }
    }
    s.requests.push(Request::Respawn { event, to_swap_spot });
}

/// `RequestRespawn` (0x675b10) → `FUN_006cd340`: target 0.
fn request_respawn(s: &mut LogicState, _: Args) -> Key {
    respawn(s, 0, false);
    EXIT
}

/// `RequestRespawnAtEventStart` (0x675b40) → `FUN_006cd380`(the pending event G+0x2f6f8).
fn request_respawn_at_event_start(s: &mut LogicState, _: Args) -> Key {
    let event = s.game.pending_event;
    respawn(s, event, false);
    EXIT
}

/// `RequestRespawnToLastSwapSpot` (0x676f70) → `FUN_006cd3a0`: target 0, +0x435 = 1.
fn request_respawn_to_last_swap_spot(s: &mut LogicState, _: Args) -> Key {
    respawn(s, 0, true);
    EXIT
}

/// The local player's heat values +0x3d8, +0x3dc, +0x3e8 = 0.
fn zero_local_heat(s: &mut LogicState) {
    if let Some(p) = s.players.local_player_mut() {
        p.heat_3d8 = 0.0;
        p.heat_3dc = 0.0;
        p.heat_3e8 = 0.0;
    }
}

/// `ResetChaseStats` (0x6760b0): `FUN_0065b830`(the pursuit manager G+0x2f410, 1), then the local heat zeroed.
fn reset_chase_stats(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::ResetChaseStats);
    zero_local_heat(s);
    EXIT
}

/// `ResetLocalPlayerHeat` (0x676120).
fn reset_local_player_heat(s: &mut LogicState, _: Args) -> Key {
    zero_local_heat(s);
    EXIT
}

/// `TransferVehicleHeatToPlayer` (0x676210): the local player's +0x3dc = 0.
fn transfer_vehicle_heat_to_player(s: &mut LogicState, _: Args) -> Key {
    if let Some(p) = s.players.local_player_mut() {
        p.heat_3dc = 0.0;
    }
    EXIT
}

/// `HasLocalPlayerSetRollout` (0x6804c0): the local slot is occupied and its vehicle +0xa0 ≠ −1.
fn has_local_player_set_rollout(s: &mut LogicState, _: Args) -> Key {
    exit_if(s.players.local_player().is_some_and(|p| p.vehicle != -1))
}

/// `ResetLocalPlayerDamage` (0x6763d0): the local player's vehicle exists (`FUN_006ba2a0`) and the GMM has not
/// transitioned events (+0x2e7f): message 0x16 for the vehicle (`FUN_00844940`).
fn reset_local_player_damage(s: &mut LogicState, _: Args) -> Key {
    if let Some(p) = s.players.local_player()
        && p.vehicle != -1
        && p.vehicle_alive
        && !s.game_modes.has_transitioned_events
    {
        let vehicle = p.vehicle;
        s.requests.push(Request::ResetVehicleDamage { vehicle });
    }
    EXIT
}

/// `DeactivateDamage` (0x68af80) → `FUN_0068acb0`.
fn deactivate_damage(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::VehicleDamage { active: false });
    EXIT
}

/// `ReactivateDamage` (0x68afa0) → `FUN_0068ad50`.
fn reactivate_damage(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::VehicleDamage { active: true });
    EXIT
}

/// `CleanAndRepairVehicle` (0x6927b0) → `FUN_006926b0`.
fn clean_and_repair_vehicle(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::CleanAndRepairVehicle);
    EXIT
}

/// `ClearNitrous` (0x676b70): `FUN_0065de30`(G+0x264e0, PM+0xa6b4).
fn clear_nitrous(s: &mut LogicState, _: Args) -> Key {
    let slot = s.players.local.map_or(-1, |l| l as i32);
    s.requests.push(Request::ClearNitrous { slot });
    EXIT
}
