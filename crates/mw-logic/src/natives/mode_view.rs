//! The `Mode` binding root: the game-mode manager's view model (`FUN_0070a120` registers it as "Mode", `.rdata`
//! 0x00fc7ef0, type "GameModeManager": 34 fields read at an offset of the manager and 86 getters). Bindings such as
//! `Mode.WorldLoaded`, `Mode.HudHsm` or `Mode.Data.FeedbackGroupID` read it; [`LogicState`] answers the root
//! (its `binding::Host`), and a member this table does not answer falls through to a data definition published
//! under "Mode" (the original's rule for registered roots, `FUN_00a4ef40`).
//!
//! Ported: every field member whose manager byte the logic keeps ([`super::game_modes::GameModeManager`]), the
//! getters the HSM reads (each with its address below), and `Data`: the current mode's `GameMode` object (handle
//! +0x2cf4 / +0x2cf8, property type 0x1a), whose members are its GeneSys fields ([`data_fields`]). Members not
//! ported (the times, the online / Autolog / recommendation getters that need services, the career statistics)
//! do not resolve.

use crate::binding::{self, Lookup, Segment, Value};

use super::LogicState;

/// `Mode.<member>[...]` (the members after the root).
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let gm = &s.game_modes;
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    let i = |v: i32| Lookup::Found(Value::Int(v));
    let Some(name) = MEMBERS.iter().find(|(h, _)| *h == first.hash).map(|(_, n)| *n) else { return Lookup::Null };
    match name {
        // Fields (`FUN_00a4e2f0`: offset into the manager).
        "ScoreLimit" => i(gm.score_limit),
        "EventScoreLimit" => i(gm.event_score_limit),
        "TimerRunning" => b(gm.timer_running),
        "EndReason" => i(gm.end_reason),
        "PlayerBustedInEvent" => b(gm.player_busted_in_event),
        "PlayerBustedInLastEvent" => b(gm.player_busted_in_last_event),
        "StoryMode" => b(gm.story_mode),
        "ShowTakedownCameras" => b(gm.show_takedown_cameras),
        "ForceEnd" => b(gm.force_end),
        "HasNextEvent" => b(gm.event_requested),
        "ShowRecap" => b(gm.recap),
        "HasTransitionedEvents" => b(gm.has_transitioned_events),
        "WillTransitionEvents" => b(gm.event_request_flag),
        "ShowSplitTime" => b(gm.show_split_time),
        "IsBiggerBetter" => b(gm.bigger_better),
        "NeedTheMapScreen" => b(gm.need_map_screen),
        "IsAmbushEvent" => b(gm.ambush_event),
        "IsScoreBasedEvent" => b(gm.score_based_event),
        "IsFreedriveEvent" => b(gm.freedrive_event),
        "IsRaceEvent" => b(gm.race_event),
        "DisplayTimeout" => b(gm.display_timeout),
        "HasSeenEventsForVehicle" => b(gm.seen_events_for_vehicle),
        "RetryAtEndOfEvent" => b(gm.retry_at_end),
        "LastEventWasRace" => b(gm.last_event_was_race),
        "RollCredits" => b(gm.credits_transition),
        "JumpToStore" => b(gm.jump_to_store),
        "InMeetup" => b(gm.in_meetup),
        "UsePauseMap" => b(gm.use_pause_map),
        "UpgradesToAward" => i(gm.award_2eb4 as i32),
        // Getters (`FUN_00a4e350`).
        // 0x712010: an event (+0x2ce0) and the world-loaded byte +0x2e77.
        "WorldLoaded" => b(gm.current_event != 0 && gm.world_loaded),
        // 0x719bd0 → `FUN_006f49c0`.
        "ShowLoadingScreen" => b(show_loading_screen(s)),
        // 0x70da10: +0x2e82.
        "HasIntroLoadCompleted" => b(gm.intro_load_complete),
        // 0x711e90: +0x2e91 or +0x2e92; 0x711e70: +0x2e91.
        "IsInGameIntro" => b(gm.in_game_intro_freedrive || gm.in_game_intro_other),
        "IsInGameIntroFreedrive" => b(gm.in_game_intro_freedrive),
        // 0x716f40: the current mode's `GameMode` +0x20, 0 without a mode.
        "HudHsm" => Lookup::Found(Value::UInt(current_mode(s).map_or(0, |m| m.hud_hsm))),
        // 0x7172a0 → `FUN_006e62a0`.
        "IsPauseAllowed" => b(pause_allowed(s)),
        // 0x711ee0: +0x2eac.
        "RetryLastEventEnabled" => b(gm.byte_2eac),
        // `FUN_006fe520`: the field at +0x252b.
        "DisablePauseMenu" => b(gm.disable_pause_menu),
        // 0x711f30: +0x2e9a and the local player's +0x2c8 clear (that byte is not kept: taken as clear).
        "LeaveGameRequested" => b(gm.leave_game && !s.players.local_player().is_some_and(|p| p.teleporting)),
        // 0x7120c0 / `RecommendsEventActive` (the same getter): a recommendation is set (G+0x2f6d0 not the null id,
        // or G+0x2f6d8 > 0); offline there is none.
        "ShowRecommendsIntro" | "RecommendsEventActive" => b(false),
        // 0x711f70 / 0x711fc0 / 0x711fe0 / 0x711f00 (online sessions): offline false.
        "IsHostMigrating" | "IsInPublicGame" | "IsHostMigrationCountdownRunning" | "IsSuitableHost" => b(false),
        // 0x717010 → `FUN_006e7210`: the pack of the local player's car.
        "CurrentVehicleGamepack" => super::entitlement_view::current_vehicle_gamepack(s, rest),
        "Data" => match current_mode(s) {
            None => Lookup::Null,
            Some(m) => match rest.first() {
                None => Lookup::Found(Value::Object(true)),
                Some(f) => m.data.iter().find(|(h, _)| *h == f.hash).map(|(_, v)| v.clone()).into(),
            },
        },
        _ => Lookup::Null,
    }
}

/// The current mode's record (`GameMode` behind the handle +0x2cf4 / +0x2cf8).
pub(super) fn current_mode(s: &LogicState) -> Option<&super::mode_machine::ModeInfo> {
    let id = s.game_modes.current_mode;
    (id != 0).then(|| s.game_modes.modes.iter().find(|m| m.id == id)).flatten()
}

/// `FUN_006f49c0`: false unless the loading screen is allowed (+0x2e81) and the events have not transitioned
/// (+0x2e7f); true with no event, online without +0x2e76, or the world not loaded (+0x2e77); else while a bit of
/// the two-bit set +0x2ea8 is set.
pub fn show_loading_screen(s: &LogicState) -> bool {
    let gm = &s.game_modes;
    if !gm.allow_loading_screen || gm.has_transitioned_events {
        return false;
    }
    if gm.current_event == 0 || (!gm.byte_2e76 && gm.online_event) || !gm.world_loaded {
        return true;
    }
    gm.loading_bits & 3 != 0
}

/// `FUN_006e62a0`: with a free-drive event running offline and an event requested (+0x2ce4), pausing is allowed
/// only when the requested event is free drive too (`FUN_006e5ea0`); otherwise always.
pub fn pause_allowed(s: &LogicState) -> bool {
    let gm = &s.game_modes;
    if super::mode_machine::is_freedrive_event(s, gm.current_event) && !gm.online_event && gm.requested_event != 0 {
        return super::mode_machine::is_freedrive_event(s, gm.requested_event);
    }
    true
}

/// A `GameMode` object's scalar GeneSys fields as binding values (`Mode.Data.<field>`): numbers, bools, strings,
/// and handles / references as their UniqueId (the low 32 bits of the resource id).
pub fn data_fields(fields: &[(u32, mw_formats::genesys::Value)]) -> Vec<(u32, Value)> {
    use mw_formats::genesys::Value as G;
    fields
        .iter()
        .filter_map(|(h, v)| {
            let v = match v {
                G::Bool(b) => Value::Bool(*b),
                G::Int(n) => Value::Int64(*n),
                G::UInt(n) => Value::UInt64(*n),
                G::Float(f) => Value::Float(*f),
                G::Str(t) => Value::Str(t.clone()),
                G::Handle(Some(r)) | G::Ref(r) => Value::UInt(r.0 as u32),
                G::Handle(None) => Value::UInt(0),
                _ => return None,
            };
            Some((*h, v))
        })
        .collect()
}

/// The member names the table answers, by binding hash.
static MEMBERS: std::sync::LazyLock<Vec<(u32, &'static str)>> = std::sync::LazyLock::new(|| {
    [
        "ScoreLimit",
        "EventScoreLimit",
        "TimerRunning",
        "EndReason",
        "PlayerBustedInEvent",
        "PlayerBustedInLastEvent",
        "StoryMode",
        "ShowTakedownCameras",
        "ForceEnd",
        "HasNextEvent",
        "ShowRecap",
        "HasTransitionedEvents",
        "WillTransitionEvents",
        "ShowSplitTime",
        "IsBiggerBetter",
        "NeedTheMapScreen",
        "IsAmbushEvent",
        "IsScoreBasedEvent",
        "IsFreedriveEvent",
        "IsRaceEvent",
        "DisplayTimeout",
        "HasSeenEventsForVehicle",
        "RetryAtEndOfEvent",
        "LastEventWasRace",
        "RollCredits",
        "JumpToStore",
        "InMeetup",
        "UsePauseMap",
        "UpgradesToAward",
        "WorldLoaded",
        "ShowLoadingScreen",
        "HasIntroLoadCompleted",
        "IsInGameIntro",
        "IsInGameIntroFreedrive",
        "HudHsm",
        "IsPauseAllowed",
        "RetryLastEventEnabled",
        "LeaveGameRequested",
        "ShowRecommendsIntro",
        "RecommendsEventActive",
        "IsHostMigrating",
        "IsInPublicGame",
        "IsHostMigrationCountdownRunning",
        "IsSuitableHost",
        "Data",
        "CurrentVehicleGamepack",
        "DisablePauseMenu",
    ]
    .into_iter()
    .map(|n| (binding::hash(n), n))
    .collect()
});

#[cfg(test)]
mod tests {
    use super::super::LogicState;
    use super::super::mode_machine::ModeInfo;
    use crate::binding::{Value, hash};

    #[test]
    fn mode_root() {
        let mut s = LogicState::default();
        // No event: the world counts as not loaded; the loading screen shows when allowed.
        s.game_modes.world_loaded = true;
        assert_eq!(s.resolve_bool("Mode.WorldLoaded"), Some(false));
        s.game_modes.allow_loading_screen = true;
        assert_eq!(s.resolve_bool("Mode.ShowLoadingScreen"), Some(true));
        // The free-drive event and mode: HudHsm and Data from the mode record.
        s.game_modes.current_event = 390703;
        s.game_modes.current_mode = 390145;
        s.game_modes.modes = vec![ModeInfo {
            id: 390145,
            hud_hsm: 371633,
            data: vec![(hash("FeedbackGroupID"), Value::UInt64(2))],
            ..Default::default()
        }];
        assert_eq!(s.resolve_bool("Mode.WorldLoaded"), Some(true));
        assert_eq!(s.resolve_bool("Mode.ShowLoadingScreen"), Some(false));
        assert_eq!(s.resolve_int("Mode.HudHsm"), Some(371633));
        assert_eq!(s.resolve_int("Mode.Data.FeedbackGroupID"), Some(2));
        assert_eq!(s.resolve_bool("Mode.ShowRecommendsIntro"), Some(false));
        s.game_modes.disable_pause_menu = true;
        assert_eq!(s.resolve_bool("Mode.DisablePauseMenu"), Some(true));
        s.game_modes.leave_game = true;
        assert_eq!(s.resolve_bool("Mode.LeaveGameRequested"), Some(true));
        s.players.local = Some(0);
        s.players.slots[0] = Some(Default::default());
        s.players.slots[0].as_mut().unwrap().teleporting = true;
        assert_eq!(s.resolve_bool("Mode.LeaveGameRequested"), Some(false));
        s.game_modes.need_map_screen = true;
        assert_eq!(s.resolve_bool("Mode.NeedTheMapScreen"), Some(true));
        // A member the table does not answer falls through to a published "Mode" data definition.
        s.view_models.publish("Mode", mw_formats::json::Json::parse(r#"{"Time": 5}"#).unwrap());
        assert_eq!(s.resolve_int("Mode.Time"), Some(5));
    }
}
