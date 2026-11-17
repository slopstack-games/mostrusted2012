//! The `Net` binding root: the network module's output state (`FUN_004c4740`, root `.rdata` 0x00fa5d40, type
//! "NetworkModuleState": 28 getters, no fields). Every getter reads the Blaze / session objects of the network module
//! (module +0x5340.. +0xbe7c); the logic runs offline, so each answers its "no connection" branch.
//!
//! Source of the answers (disassembly of the getters):
//! - `IsPlayerConnectedToFirstyParty` (0x4cf2a0) and `IsInX360Party` (0x4cf410) ignore the module: true / false.
//! - `AreAllPlayersConnected` (0x4cf5c0 -> 0xa83320) loops over the 12 session slots and counts an empty slot as
//!   fine: true without a session; `NumPlayersInGame` (0x4cf3d0 -> 0xa83640) counts the used slots: 0.
//! - `IsInOnlineGame` (module +0x6a20), `IsInLobby` / `IsGameStarted` (the same pointer, state +0xbe7c),
//!   `IsInPlaygroup` (+0x7078), `AreWeHostMigrating` (+0xbe77): the object / flag is clear offline.
//! - `IsConnectedToServer` (0xa82680), `IsLoggedInToServer` (0xa82e60), `IsGameAdmin` (0xa82c40), `IsHost`
//!   (0xa82ca0), `HasLostConnectionToHost`, `IsLocalPlayerPlaygroupLeader`, `IsMatchmakingInProgress` need a Blaze
//!   connection object (module +0x5340 / +0x5bb0 -> +0x358 / +0xe70): without one false.
//!
//! Model decision: the Blaze connection and session objects are never created offline (no network code is ported),
//! so those branches are taken as read from the code, not from a capture. Counts (`GetNumFriends`,
//! `NumPlayersInPlaygroup`, `MaxPlayers`, `MaxPlaygroupPlayers`) are 0 offline. The indexed members
//! (`IsPlayerInGame`, `GetPlayerName`, `IsPlayerMuted`) do not resolve.

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

/// `Net.<member>[...]`.
pub fn lookup(_s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let Some(name) = MEMBERS.iter().find(|n| crate::binding::hash(n) == first.hash) else { return Lookup::Null };
    match *name {
        "IsPlayerConnectedToFirstyParty" | "AreAllPlayersConnected" => Lookup::Found(Value::Bool(true)),
        "IsConnectedToServer"
        | "IsLoggedInToServer"
        | "IsInOnlineGame"
        | "IsGameAdmin"
        | "IsHost"
        | "IsMatchmakingInProgress"
        | "IsGameStarted"
        | "IsInPlaygroup"
        | "IsLocalPlayerPlaygroupLeader"
        | "AreWeHostMigrating"
        | "HasLostConnectionToHost"
        | "IsInLobby"
        | "IsInX360Party" => Lookup::Found(Value::Bool(false)),
        "GetNumOnlineFriends"
        | "GetNumFriends"
        | "MaxPlayers"
        | "NumPlayersInGame"
        | "NumPlayersInPlaygroup"
        | "MaxPlaygroupPlayers" => Lookup::Found(Value::Int(0)),
        _ => Lookup::Null,
    }
}

const MEMBERS: &[&str] = &[
    "IsPlayerConnectedToFirstyParty",
    "AreAllPlayersConnected",
    "IsConnectedToServer",
    "IsLoggedInToServer",
    "IsInOnlineGame",
    "IsGameAdmin",
    "IsHost",
    "IsMatchmakingInProgress",
    "IsGameStarted",
    "IsInPlaygroup",
    "IsLocalPlayerPlaygroupLeader",
    "AreWeHostMigrating",
    "HasLostConnectionToHost",
    "IsInLobby",
    "IsInX360Party",
    "GetNumOnlineFriends",
    "GetNumFriends",
    "MaxPlayers",
    "NumPlayersInGame",
    "NumPlayersInPlaygroup",
    "MaxPlaygroupPlayers",
];

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    #[test]
    fn net_root() {
        let s = LogicState::default();
        assert_eq!(s.resolve_bool("Net.IsInOnlineGame"), Some(false));
        assert_eq!(s.resolve_bool("Net.IsInOnlineGame(NOT)"), Some(true));
        assert_eq!(s.resolve_bool("Net.IsLoggedInToServer"), Some(false));
        assert_eq!(s.resolve_bool("Net.IsPlayerConnectedToFirstyParty"), Some(true));
        assert_eq!(s.resolve_bool("Net.AreAllPlayersConnected"), Some(true));
        assert_eq!(s.resolve_int("Net.NumPlayersInGame"), Some(0));
    }
}
