//! The `GameLogicState` binding root: `FUN_006a38c0` registers "GameLogicState" (type "GameLogicModuleState",
//! 0xc bytes) on the game-logic object with three getters, bound to the code at their addresses:
//!
//! - `IsAccessingProfile` (0x6ad680): the platform services object G+0x2f454: `(+0x4941 && +0x4935) || +0x4942`
//!   (a profile load is under way and has data, or a save runs). Offline both are clear: false.
//! - `HasSaveGameDeviceBeenUnmounted` (0x6ad6c0): always false (the code stores 0).
//! - `IsControlGroupEnabled["NAME"]` (0x6b1770): the named control group's list (the input manager's table at
//!   `*0x010edad8`+4 +0xb218: entries of 0x10 B, up to two group numbers, the count at +8, the name hash at
//!   +0xc; looked up by the key hash) is enabled when every listed group's block count byte (G+0x2f0c0 + group,
//!   [`InputBlocks::counts`](super::game_logic::InputBlocks)) is zero. The input manager's constructor (the
//!   function with the `CAR_ONLY` literal, around 0x47d...) builds the table: `CAR_ONLY` = {1}, `MENU` = {2},
//!   `DIALOG` = {3}, `DEBUG` = {4}, `UNIVERSAL` = {5}, `EASYDRIVE` = {6}, `CAR` = {1, 6}. An unknown name has
//!   no entry, which the getter answers as enabled (its loop runs over an empty list).
//!
//! The groups' bits are set and cleared by [`Request::SetInputBlock`](super::Request) (the HSM's control-group
//! behaviours), applied by [`mode_machine::apply`](super::mode_machine::apply). The `IsAccessingProfile` bytes
//! belong to the platform layer: the runtime sets [`ProfileIo`] while a profile load / save runs.

use crate::binding::{Index, Lookup, Segment, Value, hash};

use super::LogicState;

/// The profile access bytes of the services object G+0x2f454.
#[derive(Debug, Clone, Default)]
pub struct ProfileIo {
    /// +0x4941: a profile load was started.
    pub loading: bool,
    /// +0x4935: the load delivered data.
    pub loaded: bool,
    /// +0x4942: a save runs.
    pub saving: bool,
}

impl ProfileIo {
    /// 0x6ad680.
    pub fn accessing(&self) -> bool {
        (self.loading && self.loaded) || self.saving
    }
}

/// The control groups of the input manager's table: name, the group numbers it lists.
const CONTROL_GROUPS: &[(&str, &[usize])] = &[
    ("CAR_ONLY", &[1]),
    ("MENU", &[2]),
    ("DIALOG", &[3]),
    ("DEBUG", &[4]),
    ("UNIVERSAL", &[5]),
    ("EASYDRIVE", &[6]),
    ("CAR", &[1, 6]),
];

/// 0x6b1770: no listed group has a block count.
pub fn control_group_enabled(s: &LogicState, name_hash: u32) -> bool {
    let counts = &s.game.input_blocks.counts;
    CONTROL_GROUPS
        .iter()
        .find(|(n, _)| hash(n) == name_hash)
        .is_none_or(|(_, groups)| groups.iter().all(|&g| counts[g] == 0))
}

/// `GameLogicState.<member>[...]`.
pub fn lookup(s: &LogicState, members: &[Segment]) -> Lookup {
    let Some((first, _)) = members.split_first() else { return Lookup::Found(Value::Object(true)) };
    let is = |n: &str| first.hash == hash(n);
    if is("IsAccessingProfile") {
        Lookup::Found(Value::Bool(s.profile_io.accessing()))
    } else if is("HasSaveGameDeviceBeenUnmounted") {
        Lookup::Found(Value::Bool(false))
    } else if is("IsControlGroupEnabled") {
        match &first.index {
            Index::Key { hash, .. } => Lookup::Found(Value::Bool(control_group_enabled(s, *hash))),
            Index::Pos(_) => Lookup::Null,
        }
    } else {
        Lookup::Null
    }
}

#[cfg(test)]
mod tests {
    use super::super::LogicState;
    use super::super::Request;
    use super::super::mode_machine::apply;

    #[test]
    fn game_logic_state_root() {
        let mut s = LogicState::default();
        assert_eq!(s.resolve_bool("GameLogicState.IsAccessingProfile"), Some(false));
        s.profile_io.loading = true;
        assert_eq!(s.resolve_bool("GameLogicState.IsAccessingProfile"), Some(false));
        s.profile_io.loaded = true;
        assert_eq!(s.resolve_bool("GameLogicState.IsAccessingProfile"), Some(true));
        s.profile_io = Default::default();
        s.profile_io.saving = true;
        assert_eq!(s.resolve_bool("GameLogicState.IsAccessingProfile"), Some(true));
        assert_eq!(s.resolve_bool("GameLogicState.HasSaveGameDeviceBeenUnmounted"), Some(false));
        // The menu group is group 2: blocked while a bit of it is set.
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"MENU\"]"), Some(true));
        apply(&mut s, &Request::SetInputBlock { group: 2, bit: 0, set: true });
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"MENU\"]"), Some(false));
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"DIALOG\"]"), Some(true));
        // CAR lists groups 1 and 6.
        apply(&mut s, &Request::SetInputBlock { group: 6, bit: 3, set: true });
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"CAR\"]"), Some(false));
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"CAR_ONLY\"]"), Some(true));
        apply(&mut s, &Request::SetInputBlock { group: 6, bit: 3, set: false });
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"CAR\"]"), Some(true));
        // An unknown name has no entry: enabled.
        assert_eq!(s.resolve_bool("GameLogicState.IsControlGroupEnabled[\"NOPE\"]"), Some(true));
    }
}
