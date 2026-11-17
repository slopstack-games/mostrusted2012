//! The online match-making state roots: `JoinGameState`, `CreateMatchState` and `MatchMakeState`
//! (`MatchMakingState`), registered next to each other in the 0x630000 block of the dump. Their getters are:
//! `JoinGameState.IsValid` (0x64d820: object +0xc nonzero), `CreateMatchState.IsValid` (0x64d870) and
//! `MatchMakeState.IsValid` (0x64d850) (a validity byte at +0), `MatchMakeState.DoRematchmaking` (0x651660: the
//! playgroup pointer module +0x7078 nonzero, then more). The objects are filled by the online match-making code,
//! which is not ported; offline they are zero: all false (model decision, from the zero-initialised state).

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

/// `JoinGameState.` / `CreateMatchState.` / `MatchMakeState.<member>`.
pub fn lookup(_s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    if first.hash == crate::binding::hash("IsValid") || first.hash == crate::binding::hash("DoRematchmaking") {
        Lookup::Found(Value::Bool(false))
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    #[test]
    fn online_states() {
        let s = LogicState::default();
        for b in ["JoinGameState.IsValid", "CreateMatchState.IsValid", "MatchMakeState.DoRematchmaking"] {
            assert_eq!(s.resolve_bool(b), Some(false), "{b}");
        }
    }
}
