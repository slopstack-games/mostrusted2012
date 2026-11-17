//! Offline Lua natives of the garage, the ranks, the awards, the easy-drive guide's category selection and the
//! vehicle focus (handlers verified against the dump and `re/out/live/text.asm`, research log 2026-11-17 "Lua
//! natives, fourth batch"). G = `*0x01177868`; the profile manager PPM = G+0x14c08 (the `this` of the rank
//! functions `FUN_006ec470` / `FUN_006ec4f0` / `FUN_006ec5a0` and of the garage-item insert `FUN_006fe490`);
//! the options manager OM = `*(G+0x2f3e0)`; the UI manager UIM = `*(G+0x2f480)`.

use std::collections::{BTreeMap, HashSet};

use mw_lua::{Lua, Value};

use super::lua_args::{fit, int, push_int, string_or_invalid, toboolean};
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// One rank of the PPM's rank list (entries at PPM+0x94, count +0x98; each a (record, offset) pair).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RankEntry {
    /// The rank number (`*(*(rec+0x28)+0x18+off)`).
    pub rank: i32,
    /// The rank's SP threshold: `FUN_006893c0` of the entry's key (+0xc), the smallest positive value of the
    /// matching rows, 0 when there is none. The runtime fills it from the data.
    pub sp: i32,
}

/// What the natives of this file read and write.
#[derive(Debug, Default)]
pub struct GarageFields {
    /// PPM's rank list.
    pub ranks: Vec<RankEntry>,
    /// `(*(G+0x2f3e4)+0x44) == 4` (the rollout mode that uses the rank table; the rank functions return their
    /// fallback otherwise).
    pub rank_mode: bool,
    /// The local player's record +0x408 (`FUN_006bf330` of PM): the SP the rank table is counted from.
    pub sp_base: i32,
    /// The garage's items (sorted map at the garage manager +0xc, count +4): item id to its flag byte. Model
    /// decision: `known_items` is the validity check (`FUN_006f8950` over the item-type tree), which the runtime
    /// fills; `None` accepts every (kind, id).
    pub garage_items: BTreeMap<u32, bool>,
    pub known_items: Option<HashSet<(i32, i32)>>,
    /// The offline awards earned: (AwardId, UniqueId) rows of the "OfflineAward" table (`FUN_007322a0`; the
    /// progression database, G+0x2f454 +0x4964). The runtime fills it.
    pub earned_awards: HashSet<(i32, i32)>,
    /// GMM+0x251c (`ApplyModUpgrade` sets it).
    pub mod_flag: bool,
    /// The vehicle-focus object OM+0x4c90: +0xc (and the owner's +0x524c) = the focused vehicle id, +0x10
    /// cleared by `SetFocusVehicleId`.
    pub focus_vehicle_id: i32,
    pub focus_10: i32,
    /// The local player's license registration (record +4 of its 0x1c-byte entry from +0x38, `FUN_006ee6f0`,
    /// at most 8 characters), by player slot.
    pub registrations: BTreeMap<i32, String>,
}

impl GarageFields {
    /// `FUN_006ec470`: the highest rank of the list (0 when empty).
    pub fn max_rank(&self) -> i32 {
        self.ranks.iter().map(|r| r.rank).fold(0, i32::max)
    }

    /// `FUN_006ec4f0`(sp): with the rank-table rollout, the rank whose threshold is the nearest at or below
    /// `sp` (the first on a tie); 1 otherwise. When no threshold is at or below (the original dereferences a
    /// null record) the result is 1 (model decision).
    pub fn rank_for_sp(&self, sp: i32) -> i32 {
        if !self.rank_mode {
            return 1;
        }
        let mut best: Option<(i32, i32)> = None;
        for e in &self.ranks {
            let d = sp.wrapping_sub(e.sp);
            if d >= 0 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, e.rank));
            }
        }
        best.map_or(1, |(_, r)| r)
    }

    /// `FUN_006ec5a0`(rank): rank 0 is 1; above the highest rank (`FUN_006ec230`) the largest value of any
    /// listed rank (−1 for an empty list); otherwise the first entry of that rank's threshold when the rollout
    /// mode is 4, else 0 (also when no entry has the rank).
    pub fn sp_for_rank(&self, rank: i32) -> i32 {
        if rank == 0 {
            return 1;
        }
        if self.max_rank() < rank {
            return self.ranks.iter().map(|e| self.sp_for_rank(e.rank)).fold(-1, i32::max);
        }
        match self.ranks.iter().find(|e| e.rank == rank) {
            Some(e) if self.rank_mode => e.sp,
            _ => 0,
        }
    }
}

/// `AddNewGarageItem(kind, id, flag)` (0x0067a6b0 → `FUN_006fe490` = `FUN_006fe380` on G+0x14c08): `_ftol2` of
/// the first two, `lua_toboolean` of the third. When the item is valid (`FUN_006f8950`) the sorted map entry
/// for `id` is created (or found) and its flag byte set.
fn add_new_garage_item(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (kind, id, flag) = (int(&args, 1), int(&args, 2), toboolean(&args, 3));
    let g = &mut state.garage;
    if g.known_items.as_ref().is_none_or(|k| k.contains(&(kind, id))) {
        g.garage_items.insert(id as u32, flag);
    }
    Ok(vec![])
}

/// `ApplyModUpgrade(id)` (0x0068adf0 → `FUN_0068ab10`): `_ftol2` of the argument; the work is the engine's
/// ([`Request::ApplyModUpgrade`]).
fn apply_mod_upgrade(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.garage.mod_flag = true;
    state.requests.push(Request::ApplyModUpgrade { id: int(&args, 1) });
    Ok(vec![])
}

/// `GetMaxRank()` (0x00675980): `FUN_006ec470` on PPM, pushed as a number.
fn get_max_rank(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![push_int(state.garage.max_rank())])
}

/// `GetRankForSP(sp)` (0x0067a4b0): `FUN_006ec4f0`(`_ftol2`(sp) − the local record's +0x408) on PPM.
fn get_rank_for_sp(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let g = &state.garage;
    Ok(vec![push_int(g.rank_for_sp(int(&args, 1).wrapping_sub(g.sp_base)))])
}

/// `GetSPForRank(rank)` (0x0067a530): `FUN_006ec5a0`(`_ftol2`(rank)) on PPM plus the record's +0x408.
fn get_sp_for_rank(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let g = &state.garage;
    Ok(vec![push_int(g.sp_for_rank(int(&args, 1)).wrapping_add(g.sp_base))])
}

/// `HasEarnedAward(awardId, uniqueId)` (0x0067a320 → `FUN_007322a0` on G+0x2f454 +0x4964): the
/// "OfflineAward" query (AwardId, UniqueId) finds a row; pushed as a boolean.
fn has_earned_award(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let key = (int(&args, 1), int(&args, 2));
    Ok(vec![Value::Bool(state.garage.earned_awards.contains(&key))])
}

/// `ReleaseAllImages()` (0x007e9c30): [`Request::ReleaseAllImages`].
fn release_all_images(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ReleaseAllImages);
    Ok(vec![])
}

/// `ResetNeutalVaues()` (0x007f5090): [`Request::ResetNeutralValues`] (the original's spelling is kept in the
/// native's name).
fn reset_neutral_values(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::ResetNeutralValues);
    Ok(vec![])
}

/// `SelectEasyGuideCategory(id)` (0x0067a2e0 → `FUN_00792fc0` on UIM+0x47640): when the current page
/// (+0xd8 / +0xdc) exists, the first of its categories with that id is entered: the current page is pushed onto
/// the stack (+0x5c / +0x60, depth +0xac) and the category's page becomes current. No match: nothing.
fn select_easy_guide_category(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let id = int(&args, 1);
    let g = &mut state.misc.easy_guide;
    if g.current.0 == 0 {
        return Ok(vec![]);
    }
    let found = g.children.get(&g.current).and_then(|l| l.iter().find(|(i, _)| *i == id)).map(|(_, p)| *p);
    if let Some(page) = found {
        g.stack.push(g.current);
        g.current = page;
    }
    Ok(vec![])
}

/// `SetAvailableVehicleListEvent(event)` (0x00679b30 → `FUN_00871330` on OM+0x5ee0):
/// [`Request::SetAvailableVehicleListEvent`].
fn set_available_vehicle_list_event(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.requests.push(Request::SetAvailableVehicleListEvent { event: int(&args, 1) });
    Ok(vec![])
}

/// `SetFocusVehicleId(id)` (0x00679fb0 → `FUN_0086bbf0`(id, 0) on OM+0x4c90): +0xc and the owner's +0x524c =
/// id, +0x10 = 0 (the flag is 0: `FUN_00655e00` is not called).
fn set_focus_vehicle_id(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    state.garage.focus_vehicle_id = int(&args, 1);
    state.garage.focus_10 = 0;
    Ok(vec![])
}

/// `SetLocalPlayerLicenseRegistration(text)` (0x00679d60 → `FUN_00679760`): the string (`"<invalid>"` when not
/// one) cut to 8 characters (`FUN_009e20d0` copies 9 bytes) is the local player's registration (`FUN_006ee6f0`:
/// the 9-byte field of the local player's record is cleared and the text appended). Slot −1 when there is no
/// local player (the original's record index).
fn set_local_player_license_registration(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let text = string_or_invalid(&args, 1);
    let slot = state.players.local.map_or(-1, |s| s as i32);
    state.garage.registrations.insert(slot, fit(&text, 9).to_string());
    Ok(vec![])
}

/// `SetRolloutFromProfile()` (0x00692740 → `FUN_00692550`): the same request as the hook.
fn set_rollout_from_profile(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::SetRolloutFromProfile);
    Ok(vec![])
}

/// `SetSinglePlayerRollout()` (0x00690270 → `FUN_0068e5c0` on G+0x2f404): the same request as the hook.
fn set_single_player_rollout(state: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    state.requests.push(Request::SetSinglePlayerRollout);
    Ok(vec![])
}

/// `SetVehicleAndRolloutFromSaveGameByVehicleID(vehicle)` (0x00680340 → `FUN_006800a0`): `_ftol2` of the
/// argument; nothing unless it is above 0, else [`Request::SetVehicleAndRolloutFromSaveGame`].
fn set_vehicle_and_rollout_from_save_game(state: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let vehicle = int(&args, 1);
    if vehicle > 0 {
        state.requests.push(Request::SetVehicleAndRolloutFromSaveGame { vehicle });
    }
    Ok(vec![])
}

pub fn register(r: &mut Registry) {
    r.set_lua("SetVehicleAndRolloutFromSaveGameByVehicleID", set_vehicle_and_rollout_from_save_game);
    r.set_lua("AddNewGarageItem", add_new_garage_item);
    r.set_lua("ApplyModUpgrade", apply_mod_upgrade);
    r.set_lua("GetMaxRank", get_max_rank);
    r.set_lua("GetRankForSP", get_rank_for_sp);
    r.set_lua("GetSPForRank", get_sp_for_rank);
    r.set_lua("HasEarnedAward", has_earned_award);
    r.set_lua("ReleaseAllImages", release_all_images);
    r.set_lua("ResetNeutalVaues", reset_neutral_values);
    r.set_lua("SelectEasyGuideCategory", select_easy_guide_category);
    r.set_lua("SetAvailableVehicleListEvent", set_available_vehicle_list_event);
    r.set_lua("SetFocusVehicleId", set_focus_vehicle_id);
    r.set_lua("SetLocalPlayerLicenseRegistration", set_local_player_license_registration);
    r.set_lua("SetRolloutFromProfile", set_rollout_from_profile);
    r.set_lua("SetSinglePlayerRollout", set_single_player_rollout);
}

#[cfg(test)]
mod tests {
    use super::super::LogicHost;
    use super::*;

    fn setup() -> (Lua, LogicHost) {
        let mut lua = Lua::new();
        let host = LogicHost::default();
        host.install_lua(&mut lua);
        (lua, host)
    }

    fn call(lua: &mut Lua, name: &str, args: Vec<Value>) -> Vec<Value> {
        lua.call_global(name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn num(r: &[Value]) -> f64 {
        match r.first() {
            Some(Value::Number(n)) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    fn n(v: f64) -> Value {
        Value::Number(v)
    }

    #[test]
    fn ranks() {
        let (mut lua, host) = setup();
        assert_eq!(num(&call(&mut lua, "GetMaxRank", vec![])), 0.0);
        {
            let mut s = host.state.borrow_mut();
            s.garage.ranks = [(1, 0), (2, 100), (3, 500)].map(|(rank, sp)| RankEntry { rank, sp }).to_vec();
            s.garage.sp_base = 50;
        }
        assert_eq!(num(&call(&mut lua, "GetMaxRank", vec![])), 3.0);
        // Not the rank-table rollout: 1 / 0 (+ base).
        assert_eq!(num(&call(&mut lua, "GetRankForSP", vec![n(700.0)])), 1.0);
        assert_eq!(num(&call(&mut lua, "GetSPForRank", vec![n(2.0)])), 50.0);
        assert_eq!(num(&call(&mut lua, "GetSPForRank", vec![n(0.0)])), 51.0);
        host.state.borrow_mut().garage.rank_mode = true;
        assert_eq!(num(&call(&mut lua, "GetRankForSP", vec![n(700.0)])), 3.0, "700 - 50 = 650: rank 3 at 500");
        assert_eq!(num(&call(&mut lua, "GetRankForSP", vec![n(610.0)])), 3.0);
        assert_eq!(num(&call(&mut lua, "GetRankForSP", vec![n(540.0)])), 2.0, "490 is below rank 3 at 500");
        assert_eq!(num(&call(&mut lua, "GetSPForRank", vec![n(2.0)])), 150.0);
        assert_eq!(num(&call(&mut lua, "GetSPForRank", vec![n(9.0)])), 550.0, "above the max: the largest");
        assert_eq!(num(&call(&mut lua, "GetSPForRank", vec![n(0.0)])), 51.0);
    }

    #[test]
    fn garage_and_awards() {
        let (mut lua, host) = setup();
        call(&mut lua, "AddNewGarageItem", vec![n(3.0), n(77.0), Value::Bool(true)]);
        assert_eq!(host.state.borrow().garage.garage_items.get(&77), Some(&true));
        host.state.borrow_mut().garage.known_items = Some([(3, 5)].into());
        call(&mut lua, "AddNewGarageItem", vec![n(3.0), n(78.0), Value::Bool(true)]);
        assert!(!host.state.borrow().garage.garage_items.contains_key(&78), "invalid item: nothing");
        call(&mut lua, "AddNewGarageItem", vec![n(3.0), n(5.0), Value::Nil]);
        assert_eq!(host.state.borrow().garage.garage_items.get(&5), Some(&false));

        let has = |lua: &mut Lua| call(lua, "HasEarnedAward", vec![n(4.0), n(9.0)]);
        assert!(matches!(has(&mut lua)[..], [Value::Bool(false)]));
        host.state.borrow_mut().garage.earned_awards.insert((4, 9));
        assert!(matches!(has(&mut lua)[..], [Value::Bool(true)]));
    }

    #[test]
    fn easy_guide_focus_registration_requests() {
        let (mut lua, host) = setup();
        {
            let mut s = host.state.borrow_mut();
            let g = &mut s.misc.easy_guide;
            g.current = (1, 0);
            g.children.insert((1, 0), vec![(5, (2, 0)), (6, (3, 0)), (5, (4, 0))]);
        }
        call(&mut lua, "SelectEasyGuideCategory", vec![n(9.0)]);
        assert_eq!(host.state.borrow().misc.easy_guide.current, (1, 0));
        call(&mut lua, "SelectEasyGuideCategory", vec![n(5.0)]);
        {
            let s = host.state.borrow();
            assert_eq!((s.misc.easy_guide.current, s.misc.easy_guide.stack.clone()), ((2, 0), vec![(1, 0)]));
        }

        call(&mut lua, "SetFocusVehicleId", vec![n(42.9)]);
        assert_eq!(host.state.borrow().garage.focus_vehicle_id, 42);
        host.state.borrow_mut().players.local = Some(0);
        call(&mut lua, "SetLocalPlayerLicenseRegistration", vec![Value::from("ABCDEFGHIJ")]);
        assert_eq!(host.state.borrow().garage.registrations[&0], "ABCDEFGH");

        for name in ["ReleaseAllImages", "ResetNeutalVaues", "SetRolloutFromProfile", "SetSinglePlayerRollout"] {
            call(&mut lua, name, vec![]);
        }
        call(&mut lua, "ApplyModUpgrade", vec![n(12.0)]);
        call(&mut lua, "SetAvailableVehicleListEvent", vec![n(7.0)]);
        call(&mut lua, "SetVehicleAndRolloutFromSaveGameByVehicleID", vec![n(0.0)]);
        call(&mut lua, "SetVehicleAndRolloutFromSaveGameByVehicleID", vec![n(31.0)]);
        let s = host.state.borrow();
        assert!(s.garage.mod_flag);
        assert_eq!(
            s.requests,
            [
                Request::ReleaseAllImages,
                Request::ResetNeutralValues,
                Request::SetRolloutFromProfile,
                Request::SetSinglePlayerRollout,
                Request::ApplyModUpgrade { id: 12 },
                Request::SetAvailableVehicleListEvent { event: 7 },
                Request::SetVehicleAndRolloutFromSaveGame { vehicle: 31 },
            ]
        );
    }
}
