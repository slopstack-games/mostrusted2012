//! The `ModeVM` binding root: `GameModeViewModel` (`FUN_006fe520`, root "ModeVM", type size 0x2c90). Its object is
//! the view model embedded in the game-mode manager at manager +0x20 (`FUN_0070be40` constructs it with
//! `FUN_006e7670` on `esi + 0x20`, then `FUN_0070a120` calls `FUN_006fe520` with ECX = that object; the argument
//! `param_2 + 0xe8` is the binding manager the registrations go to). A view-model offset is therefore manager
//! offset - 0x20: `ForceOpenEasyDrive` (VM +0x2c89) is `GameModeManager::force_open_easydrive` (+0x2ca9),
//! `ModsAppliedInOutro` (VM +0x24fc) is manager +0x251c (`GarageFields::mod_flag`). The VM also keeps a pointer
//! to the manager at +0x1914 and to the game logic at +0x1910.
//!
//! Ported: the two fields, and the getters whose code is local to the manager:
//! `CanSkipIntro` (0x711d90: two calls that only look something up, then always true), `ShowSkipPrompt`
//! (0x711dc0: float VM +0x24f0 > the `.rdata` 0.0 at 0xf8f344), `ShowPositionFromCheckpoint` (0x716e50: VM +0x2506
//! clear and the current mode's id is not 0x212851). Not ported (they need the player / UI / events): the
//! blacklist table getters, `IsWaitingForBustRetry` (needs `UISequences.IsActionHostUsed`),
//! `WasBlacklistBeatenAndNotShutdown`, `GetFinishedString`, `ShouldShowInCarPosition` (a UI global),
//! `CurrentContext`. A member this table does not answer falls through to a data definition published under
//! "ModeVM" (`ModeVM.Event.speedwall.Items`).

use crate::binding::{Lookup, Segment, Value};

use super::LogicState;

/// The mode id whose events never show the position at a checkpoint (0x716e50).
const NO_CHECKPOINT_POSITION_MODE: u32 = 0x212851;

/// `ModeVM.<member>[...]`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let gm = &s.game_modes;
    let b = |v: bool| Lookup::Found(Value::Bool(v));
    let is = |n: &str| first.hash == crate::binding::hash(n);
    if is("ForceOpenEasyDrive") {
        b(gm.force_open_easydrive)
    } else if is("ModsAppliedInOutro") {
        b(s.garage.mod_flag)
    } else if is("CanSkipIntro") {
        b(true)
    } else if is("ShowSkipPrompt") {
        b(gm.skip_prompt_time > 0.0)
    } else if is("ShowPositionFromCheckpoint") {
        b(!gm.has_target_speed && gm.current_mode != NO_CHECKPOINT_POSITION_MODE)
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;

    #[test]
    fn modevm_root() {
        let mut s = LogicState::default();
        assert_eq!(s.resolve_bool("ModeVM.ForceOpenEasyDrive"), Some(false));
        s.game_modes.force_open_easydrive = true;
        assert_eq!(s.resolve_bool("ModeVM.ForceOpenEasyDrive"), Some(true));
        s.garage.mod_flag = true;
        assert_eq!(s.resolve_bool("ModeVM.ModsAppliedInOutro(NOT)"), Some(false));
        assert_eq!(s.resolve_bool("ModeVM.CanSkipIntro"), Some(true));
        assert_eq!(s.resolve_bool("ModeVM.ShowSkipPrompt"), Some(false));
        s.game_modes.skip_prompt_time = 1.0;
        assert_eq!(s.resolve_bool("ModeVM.ShowSkipPrompt"), Some(true));
        assert_eq!(s.resolve_bool("ModeVM.ShowPositionFromCheckpoint"), Some(true));
        s.game_modes.current_mode = 0x212851;
        assert_eq!(s.resolve_bool("ModeVM.ShowPositionFromCheckpoint"), Some(false));
        // Falls through to a published definition.
        s.view_models.publish("ModeVM", mw_formats::json::Json::parse(r#"{"EventID": 7}"#).unwrap());
        assert_eq!(s.resolve_int("ModeVM.EventID"), Some(7));
    }
}
