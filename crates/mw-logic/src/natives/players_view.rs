//! The `Players` binding root: the player manager PM (G+0x2f3dc). `FUN_006d4350` registers the type
//! "PlayerManager" (0xcc60 bytes) and ends with the root "Players" on it (`FUN_006d3c30`'s table); the player
//! records are the types "PlayerBase" (`FUN_006c...`, 0x250) and "Player" (`FUN_006d...`, 230 members), 0x7f0 B
//! records inline from PM+0x10. The getters are the ones the HSM reads plus the manager's own and the record
//! fields the UI data reads that are plain state; each is bound to its code address below. What the original
//! computes from the engine (the car, the pursuit, the physics) is a field the runtime sets
//! ([`Player`](super::players::Player), one doc line each).
//!
//! **PM members** (all 1-based for arrays; the getters take the 0-based index):
//!
//! | Member | Code |
//! |---|---|
//! | `LocalPlayer`, `GetLocalCompetitor` | 0x6dbef0: slot +0xa6b4 when occupied (+0xa5d0) |
//! | `CameraTarget`, `GetCameraTarget` | 0x6dbf20: slot +0xa6bc when occupied, else the local player |
//! | `Player[n]`, `GetCompetitor[n]`, `PlayerBase[n]`, `AIPlayer[n]` (0x18) | 0x6dbe60 / 0x6dbe30 / 0x6dbec0: the slot when occupied |
//! | `IsPlayerActive[n]` | 0x6d9d10: the slot is occupied |
//! | `Racer[n]` / `Cop[n]` (0xc) | 0x6dd690 / 0x6dd6c0: slot at the n-th place of the racer list +0xa664 (count +0xa694) / cop list +0xa630 (count +0xa660) |
//! | `RacerCount` / `CopCount` | 0x6dbf80 / 0x6dbfa0: those counts |
//! | `NumNonLocalPlayers` | 0x6d9cf0: +0xa6a4 - 1 |
//! | `NumNonMidGameJoinPlayers` | 0x6dbda0 (0x6bef60): occupied slots whose +0x474 is clear |
//! | `NumTeamPlayers`, `NumOpponents`, `HasAnyPlayerFinishedCurrentEvent` | fields +0xa6ac, +0xa6b0, +0xcbe9 |
//!
//! **Player members**: fields `IsActive` +0x23d, `Wrecked` +0x23e, `Crashed` +0x23f, `PlayerIndex` +0x98 (the slot),
//! `RaceCarIndex` +0xa0 (the vehicle), `IsTeleporting` +0x2c8, `WasBusted` +0x434, `IsFallingOffCliff` +0x2f6,
//! `IsInBuilding` +0x285, `IsMidGameJoining` +0x474; getters `LocalPlayer` (0x6d7d10: kind +0x9c == 0), `IsCop`
//! (0x6d7ce0: kind 2 and the cop byte), `ResetInProgress` (0x6bc410 through the vtable: +0x2c1 set, or the
//! vehicle's world flag G+0x25694[vehicle]), `IsEngineOn` (`FUN_006d8ab0`: a live vehicle whose engine runs),
//! `IsInWaitForZone` (0x6d8c20), `IsWreckSuicide` (0x6d8b90: +0x2ea ? +0x2e8 : false), `HasChaseStarted` (0x6db740:
//! the pursuit record of the slot), `CurrentNitrousType` (`FUN_006bde60`: `NONE` for class 0, `BURN` / `DUMP` /
//! `JUMP` for class 1 and kind 3 / 4 / 0xd, `BURN_PRO` / `DUMP_PRO` / `JUMP_PRO` for class 2, a null string
//! without a record or for another kind) and `GetVehicleInfo.ID` / `.GamePackID` (0x6d8380: the vehicle list's
//! entry of a live vehicle). Not ported (they need the physics, the pursuit or the rollout): speeds, RPM, the heat,
//! chase, position and score members, `GetTransform`, `SelectedRollout`, ... they do not resolve (a published
//! "Players" definition still answers).
//!
//! Model decisions: `RacerCount` / `Racers` are the AI-driven (+0x9c == 2) non-cop slots and `CopCount` / `Cops`
//! the cop ones, in slot order (the original keeps explicit lists that its add / remove paths maintain);
//! `NumNonLocalPlayers` counts the occupied slots minus one (the original's +0xa6a4 is kept by the match code).

use crate::binding::{Index, Lookup, Segment, Value, hash};

use super::LogicState;
use super::players::{PlayerManager, SLOTS};

/// 0-based array index of a segment (`[n]` is 1-based; 0 or past `count` is out of range).
fn array_index(seg: &Segment, count: usize) -> Result<usize, Lookup> {
    match &seg.index {
        Index::Pos(n) if *n >= 1 && (*n as usize) <= count => Ok(*n as usize - 1),
        Index::Pos(_) => Err(Lookup::OutOfRange),
        Index::Key { .. } => Err(Lookup::Null),
    }
}

/// The AI-driven non-cop slots, then the cop slots (the racer / cop lists, in slot order).
fn racers(pm: &PlayerManager, cops: bool) -> Vec<usize> {
    pm.slots
        .iter()
        .enumerate()
        .filter(|(_, p)| p.as_ref().is_some_and(|p| p.kind == 2 && p.cop == cops))
        .map(|(i, _)| i)
        .collect()
}

fn occupied(pm: &PlayerManager, slot: usize) -> bool {
    pm.slots.get(slot).is_some_and(Option::is_some)
}

/// `CameraTarget` (0x6dbf20): the camera target's slot when occupied, else the local player's.
fn camera_slot(pm: &PlayerManager) -> Option<usize> {
    let cam = pm.camera_target.filter(|&c| occupied(pm, c));
    cam.or(pm.local.filter(|&l| occupied(pm, l)))
}

/// `Players.<member>[...]`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let pm = &s.players;
    let is = |n: &str| first.hash == hash(n);
    let int = |v: usize| Lookup::Found(Value::Int(v as i32));
    let slot_object = |slot: Option<usize>| match slot {
        Some(slot) => player_lookup(s, slot, rest),
        None => Lookup::Null,
    };
    if is("LocalPlayer") || is("GetLocalCompetitor") {
        slot_object(pm.local.filter(|&l| occupied(pm, l)))
    } else if is("CameraTarget") || is("GetCameraTarget") {
        slot_object(camera_slot(pm))
    } else if is("Player") || is("GetCompetitor") || is("PlayerBase") || is("AIPlayer") {
        match array_index(first, SLOTS) {
            Ok(i) => slot_object(occupied(pm, i).then_some(i)),
            Err(l) => l,
        }
    } else if is("IsPlayerActive") {
        match array_index(first, SLOTS) {
            Ok(i) => Lookup::Found(Value::Bool(occupied(pm, i))),
            Err(l) => l,
        }
    } else if is("Racer") || is("Cop") {
        let list = racers(pm, is("Cop"));
        match array_index(first, 12) {
            Ok(i) => slot_object(list.get(i).copied()),
            Err(l) => l,
        }
    } else if is("RacerCount") {
        int(racers(pm, false).len())
    } else if is("CopCount") {
        int(racers(pm, true).len())
    } else if is("NumNonLocalPlayers") {
        Lookup::Found(Value::Int(pm.slots.iter().flatten().count() as i32 - 1))
    } else if is("NumNonMidGameJoinPlayers") {
        int(pm.slots.iter().flatten().filter(|p| !p.mid_game_joining).count())
    } else if is("NumTeamPlayers") {
        Lookup::Found(Value::Int(pm.team_players))
    } else if is("NumOpponents") {
        Lookup::Found(Value::Int(pm.opponents))
    } else if is("HasAnyPlayerFinishedCurrentEvent") {
        Lookup::Found(Value::Bool(pm.any_finished))
    } else {
        Lookup::Null
    }
}

/// A member of the player record in `slot` (it is occupied).
fn player_lookup(s: &LogicState, slot: usize, members: &[Segment]) -> Lookup {
    let Some((first, rest)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let Some(p) = s.players.slots.get(slot).and_then(Option::as_ref) else { return Lookup::Null };
    let is = |n: &str| first.hash == hash(n);
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    // `FUN_00573390`(vehicle) on the world: the vehicle exists.
    let live_vehicle = p.vehicle != -1 && p.vehicle_alive;
    if is("IsActive") {
        b(p.active)
    } else if is("Wrecked") {
        b(p.wrecked)
    } else if is("Crashed") {
        b(p.crashed)
    } else if is("PlayerIndex") {
        Lookup::Found(Value::Int(slot as i32))
    } else if is("RaceCarIndex") {
        Lookup::Found(Value::Int(p.vehicle))
    } else if is("IsTeleporting") {
        b(p.teleporting)
    } else if is("WasBusted") {
        b(p.was_busted)
    } else if is("IsFallingOffCliff") {
        b(p.falling_off_cliff)
    } else if is("IsInBuilding") {
        b(p.in_building)
    } else if is("IsMidGameJoining") {
        b(p.mid_game_joining)
    } else if is("LocalPlayer") {
        b(p.kind == 0)
    } else if is("IsCop") {
        b(p.kind == 2 && p.cop)
    } else if is("ResetInProgress") {
        b(p.reset_pending || (p.vehicle != -1 && p.vehicle_resetting))
    } else if is("IsEngineOn") {
        b(live_vehicle && p.engine_on)
    } else if is("IsInWaitForZone") {
        b(p.in_wait_for_zone)
    } else if is("IsWreckSuicide") {
        b(p.wreck_cause_known && p.wreck_suicide)
    } else if is("HasChaseStarted") {
        b(p.chase_started)
    } else if is("CurrentNitrousType") {
        match nitrous_name(p.nitrous) {
            Some(n) => Lookup::Found(Value::Str(n.to_owned())),
            None => Lookup::Found(Value::Str(String::new())),
        }
    } else if is("GetVehicleInfo") {
        if !live_vehicle {
            return Lookup::Null;
        }
        match rest.first() {
            None => Lookup::Found(Value::Object(true)),
            Some(f) if f.hash == hash("ID") => Lookup::Found(Value::UInt(p.vehicle_id)),
            Some(f) if f.hash == hash("GamePackID") => {
                Lookup::Found(Value::UInt(s.entitlements.vehicle_pack(p.vehicle_id)))
            }
            Some(_) => Lookup::Null,
        }
    } else {
        Lookup::Null
    }
}

/// `FUN_006bde60`: the nitrous type's name (`.data` 0x010c56f4..0x010c570c: pointers to the `.rdata` strings).
fn nitrous_name(n: Option<(u32, u32)>) -> Option<&'static str> {
    let (class, kind) = n?;
    match (class, kind) {
        (0, _) => Some("NONE"),
        (1, 3) => Some("BURN"),
        (1, 4) => Some("DUMP"),
        (1, 0xd) => Some("JUMP"),
        (2, 3) => Some("BURN_PRO"),
        (2, 4) => Some("DUMP_PRO"),
        (2, 0xd) => Some("JUMP_PRO"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;
    use super::super::players::Player;

    fn state() -> LogicState {
        let mut s = LogicState::default();
        s.players.local = Some(0);
        s.players.slots[0] = Some(Player { vehicle: 3, vehicle_alive: true, vehicle_id: 77, ..Default::default() });
        s
    }

    fn resolves(s: &LogicState, path: &str) -> bool {
        crate::binding::Binding::parse(path).unwrap().eval_int(s).ok
    }

    #[test]
    fn local_player_members() {
        let mut s = state();
        // The respawn: pending until the runtime clears it, or while the vehicle's world flag is set.
        assert_eq!(s.resolve_bool("Players.LocalPlayer.ResetInProgress(NOT)"), Some(true));
        s.players.slots[0].as_mut().unwrap().reset_pending = true;
        assert_eq!(s.resolve_bool("Players.LocalPlayer.ResetInProgress"), Some(true));
        assert_eq!(s.resolve_bool("Players.LocalPlayer.ResetInProgress(NOT)"), Some(false));
        {
            let p = s.players.slots[0].as_mut().unwrap();
            p.reset_pending = false;
            p.vehicle_resetting = true;
        }
        assert_eq!(s.resolve_bool("Players.LocalPlayer.ResetInProgress"), Some(true));
        s.players.slots[0].as_mut().unwrap().vehicle = -1;
        assert_eq!(s.resolve_bool("Players.LocalPlayer.ResetInProgress"), Some(false));
        s.players.slots[0].as_mut().unwrap().vehicle = 3;
        s.players.slots[0].as_mut().unwrap().vehicle_resetting = false;
        // Fields.
        for (name, set) in [
            ("Wrecked", (|p: &mut Player| p.wrecked = true) as fn(&mut Player)),
            ("Crashed", |p| p.crashed = true),
            ("WasBusted", |p| p.was_busted = true),
            ("IsTeleporting", |p| p.teleporting = true),
            ("IsFallingOffCliff", |p| p.falling_off_cliff = true),
            ("IsInBuilding", |p| p.in_building = true),
            ("IsMidGameJoining", |p| p.mid_game_joining = true),
            ("IsEngineOn", |p| p.engine_on = true),
            ("IsInWaitForZone", |p| p.in_wait_for_zone = true),
            ("HasChaseStarted", |p| p.chase_started = true),
        ] {
            let path = format!("Players.LocalPlayer.{name}");
            assert_eq!(s.resolve_bool(&path), Some(false), "{name}");
            set(s.players.slots[0].as_mut().unwrap());
            assert_eq!(s.resolve_bool(&path), Some(true), "{name}");
        }
        // The wreck is a suicide only when its cause is known.
        s.players.slots[0].as_mut().unwrap().wreck_suicide = true;
        assert_eq!(s.resolve_bool("Players.LocalPlayer.IsWreckSuicide"), Some(false));
        s.players.slots[0].as_mut().unwrap().wreck_cause_known = true;
        assert_eq!(s.resolve_bool("Players.LocalPlayer.IsWreckSuicide"), Some(true));
        // The engine needs a live vehicle.
        s.players.slots[0].as_mut().unwrap().vehicle_alive = false;
        assert_eq!(s.resolve_bool("Players.LocalPlayer.IsEngineOn"), Some(false));
        // Nitrous names and the vehicle info.
        assert_eq!(s.resolve_string("Players.LocalPlayer.CurrentNitrousType").as_deref(), Some(""));
        s.players.slots[0].as_mut().unwrap().nitrous = Some((2, 4));
        assert_eq!(s.resolve_string("Players.LocalPlayer.CurrentNitrousType").as_deref(), Some("DUMP_PRO"));
        s.players.slots[0].as_mut().unwrap().nitrous = Some((0, 0));
        assert_eq!(s.resolve_string("Players.LocalPlayer.CurrentNitrousType").as_deref(), Some("NONE"));
        assert!(!resolves(&s, "Players.LocalPlayer.GetVehicleInfo.ID"));
        s.players.slots[0].as_mut().unwrap().vehicle_alive = true;
        assert_eq!(s.resolve_int("Players.LocalPlayer.GetVehicleInfo.ID"), Some(77));
        assert_eq!(s.resolve_bool("Players.LocalPlayer.LocalPlayer"), Some(true));
        assert_eq!(s.resolve_int("Players.LocalPlayer.PlayerIndex"), Some(0));
    }

    #[test]
    fn manager_members() {
        let mut s = state();
        // No player at all: the local player is a null object.
        let none = LogicState::default();
        assert!(!resolves(&none, "Players.LocalPlayer.Wrecked"));
        assert_eq!(s.resolve_int("Players.NumNonLocalPlayers"), Some(0));
        s.players.slots[12] = Some(Player { kind: 2, ..Default::default() });
        s.players.slots[13] = Some(Player { kind: 2, mid_game_joining: true, ..Default::default() });
        s.players.slots[14] = Some(Player { kind: 2, cop: true, ..Default::default() });
        assert_eq!(s.resolve_int("Players.NumNonLocalPlayers"), Some(3));
        assert_eq!(s.resolve_int("Players.NumNonMidGameJoinPlayers"), Some(3));
        assert_eq!(s.resolve_int("Players.RacerCount"), Some(2));
        assert_eq!(s.resolve_int("Players.CopCount"), Some(1));
        assert_eq!(s.resolve_int("Players.Racer[2].PlayerIndex"), Some(13));
        assert_eq!(s.resolve_bool("Players.Cop[1].IsCop"), Some(true));
        assert!(!resolves(&s, "Players.Racer[3].IsActive"));
        assert_eq!(s.resolve_bool("Players.IsPlayerActive[13]"), Some(true));
        assert_eq!(s.resolve_bool("Players.IsPlayerActive[2]"), Some(false));
        assert_eq!(s.resolve_int("Players.Player[13].PlayerIndex"), Some(12));
        // The camera follows the local player unless its target is occupied.
        s.players.camera_target = Some(14);
        assert_eq!(s.resolve_int("Players.CameraTarget.PlayerIndex"), Some(14));
        s.players.camera_target = Some(20);
        assert_eq!(s.resolve_int("Players.CameraTarget.PlayerIndex"), Some(0));
        s.players.any_finished = true;
        assert_eq!(s.resolve_bool("Players.HasAnyPlayerFinishedCurrentEvent"), Some(true));
        // An unregistered member falls through to a published definition.
        s.view_models.publish("Players", mw_formats::json::Json::parse(r#"{"X": 4}"#).unwrap());
        assert_eq!(s.resolve_int("Players.X"), Some(4));
    }
}
