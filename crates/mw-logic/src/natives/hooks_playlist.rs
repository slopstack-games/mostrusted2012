//! The ninth twenty HSM hooks (research-log 2026-11-17 "ninth 20 HSM hooks"): the playlists, the matchmaking and
//! join wishes, the friend list the join hooks read and a few online / easy-drive ones. Handlers read from the
//! snapshot's disassembly. The networking behind them is engine work: a [`Request`]; what the handlers read or
//! store themselves is a field here.

use super::hook_args::{self as arg, strtol10};
use super::hooks_flow::exit_if;
use super::{LogicState, Registry, Request};
use crate::hsm::{EXIT, Key};
use mw_formats::json::Json;

type Args<'a> = Option<&'a Json>;

/// One entry of the friend list (G+0x2f3e0's list, `FUN_0086f2f0`: 0x68 bytes each).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Friend {
    /// +0x38 / +0x3c: the friend's 64-bit id.
    pub id: u64,
    /// +0x63 (0 = the friend is not in a game that needs the airport; `FUN_006748d0` answers TRUE for those).
    pub in_airport_game: bool,
}

#[derive(Debug, Default)]
pub struct HookPlaylistFields {
    /// The friend list (offline: empty; the runtime fills it).
    pub friends: Vec<Friend>,
    /// `FUN_006e9a30`: the playlist manager (G+0x2f3fc)+0xc has a current playlist with a state (+0x140 != 0).
    /// Runtime; false offline.
    pub playlist_active: bool,
    /// G+0x2f4ac / +0x2f4b0..: `SetJoinFriendIdByIndexBinding` chose a friend: flag and the id (stored twice).
    pub join_friend_set: bool,
    pub join_friend_id: u64,
    pub join_friend_id_copy: u64,
    /// G+0x2f498: `SetFriendInviteIdEasyDrive`: the invite's friend id (0 = none).
    pub friend_invite_id: u64,
    /// G+0x2f490 (word): `QuickJoinGameDesired`.
    pub quick_join_desired: bool,
    /// G+0x2f708 (byte): `CreateGameDesired`.
    pub create_game_desired: bool,
    /// (G+0x2f3fc)+0x2ce4 / +0x2e7e / +0x2e80 / +0x2d10 / +0x2d14 (`FUN_006e74f0`(0, 0): the wished game is cleared).
    pub desired_game: i32,
    pub desired_game_set: bool,
    pub desired_game_flag: bool,
    pub desired_game_pair: (u32, u32),
    /// The playlist manager's byte +0x84 (cleared by `OnMidGameJoin`) and its +4 (= G+0x2a9e0, a runtime value).
    pub mid_game_join_flag: bool,
    pub playlist_mgr_4: u32,
    pub playlist_source_2a9e0: u32,
    /// (G+0x2f3fc)+0x2cf4/0x2cf8 -> byte +0x71: the post-event driving tests of the online race are pending
    /// (`ProcessOnlinePostEventDrivingTests`). Runtime.
    pub driving_tests_pending: bool,
    /// (G+0x2f404) + PM+0xa6b4 x 0xd8 + 0x82b8: the rollout record's sweet-revenge value of the local player.
    pub sweet_revenge: [i32; 12],
    /// (G+0x2f3e0)+0x14 (word): the friend carousel view model is active.
    pub friend_carousel_active: bool,
}

pub fn register(r: &mut Registry) {
    r.set_hook("LoadLastOnlineRollout", load_last_online_rollout);
    r.set_hook("UploadLocalPlayerXP", upload_local_player_xp);
    r.set_hook("ProcessOnlinePostEventDrivingTests", process_online_post_event_driving_tests);
    r.set_hook("SetupPlaylistsVote", setup_playlists_vote);
    r.set_hook("PlayCriterionPlaylist", play_criterion_playlist);
    r.set_hook("PlayCustomPlaylist", play_custom_playlist);
    r.set_hook("HostCancelPlaylist", host_cancel_playlist);
    r.set_hook("SoloPlayerCancelPlaylist", solo_player_cancel_playlist);
    r.set_hook("EZPlayCustomPlaylist", ez_play_custom_playlist);
    r.set_hook("MatchmakePlaygroup", matchmake_playgroup);
    r.set_hook("QuickJoinGameDesired", quick_join_game_desired);
    r.set_hook("CreateGameDesired", create_game_desired);
    r.set_hook("SetRolloutSweetRevengeFromEasyDrive", set_rollout_sweet_revenge_from_easy_drive);
    r.set_hook("SetFriendInviteIdEasyDrive", set_friend_invite_id_easy_drive);
    r.set_hook("SetJoinFriendIdByIndexBinding", set_join_friend_id_by_index_binding);
    r.set_hook("CanJoinAirportGameByFriendIndexBinding", can_join_airport_game_by_friend_index_binding);
    r.set_hook("OnMidGameJoin", on_mid_game_join);
    r.set_hook("ActivateFriendCarouselViewModel", activate_friend_carousel_view_model);
    r.set_hook("DeactivateFriendCarouselViewModel", deactivate_friend_carousel_view_model);
    r.set_hook("UnlockMwlCache", unlock_mwl_cache);
}

/// `LoadLastOnlineRollout` (0x693c50): `FUN_00692800` on the rollout manager G+0x2f404.
fn load_last_online_rollout(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::LoadLastOnlineRollout);
    EXIT
}

/// `UploadLocalPlayerXP` (0x676370): the local player's XP (+0x3fc) above 0 is uploaded as a stat (`FUN_00734f60`
/// on G+0x2f3f8, type 4); then +0x3fc = 0 either way.
fn upload_local_player_xp(s: &mut LogicState, _: Args) -> Key {
    if let Some(p) = s.players.local_player_mut() {
        let xp = std::mem::take(&mut p.online_xp);
        if xp > 0 {
            s.requests.push(Request::UploadLocalPlayerXp { xp });
        }
    }
    EXIT
}

/// `ProcessOnlinePostEventDrivingTests` (0x680560): online event (`FUN_006dfed0` = (G+0x2f3fc)+4 +0xe8) and the
/// race state's byte +0x71 set -> `FUN_00704bf0` on G+0x2f418.
fn process_online_post_event_driving_tests(s: &mut LogicState, _: Args) -> Key {
    if s.game_modes.online_event && s.hook_playlist.driving_tests_pending {
        s.requests.push(Request::ProcessOnlineDrivingTests);
    }
    EXIT
}

/// `SetupPlaylistsVote` (0x676430): `FUN_006f60f0` on the playlist manager (G+0x2f3fc)+0x14.
fn setup_playlists_vote(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::SetupPlaylistsVote);
    EXIT
}

/// `PlayCriterionPlaylist` (0x676450): without a current playlist (`FUN_006e9a30`), `FUN_006f6040`.
fn play_criterion_playlist(s: &mut LogicState, _: Args) -> Key {
    if !s.hook_playlist.playlist_active {
        s.requests.push(Request::PlayCriterionPlaylist);
    }
    EXIT
}

/// `PlayCustomPlaylist` (0x676480): `liPlaylistIndex` (int) -> `FUN_006f2730`.
fn play_custom_playlist(s: &mut LogicState, args: Args) -> Key {
    s.requests.push(Request::PlayCustomPlaylist { index: arg::int(args, arg::LI_PLAYLIST_INDEX) });
    EXIT
}

/// `FUN_006f5f00`(solo): acts only while no vote index is set (+0x8c == -1), a playlist is current (`FUN_006e9a30`)
/// and the network session answers (`FUN_004c22b0`). Model decision: the current playlist is the guard here; the
/// other two are the runtime's to check when it handles the request.
fn cancel_playlist(s: &mut LogicState, solo: bool) {
    if s.hook_playlist.playlist_active {
        s.requests.push(Request::CancelPlaylist { solo });
    }
}

/// `HostCancelPlaylist` (0x6764c0): `FUN_006f5f00`(0).
fn host_cancel_playlist(s: &mut LogicState, _: Args) -> Key {
    cancel_playlist(s, false);
    EXIT
}

/// `SoloPlayerCancelPlaylist` (0x6764f0): `FUN_006f5f00`(1).
fn solo_player_cancel_playlist(s: &mut LogicState, _: Args) -> Key {
    cancel_playlist(s, true);
    EXIT
}

/// `EZPlayCustomPlaylist` (0x676520): `atoi(lpcValue)` (`FUN_00edb8aa`) - 1 -> `FUN_006f2730`.
fn ez_play_custom_playlist(s: &mut LogicState, args: Args) -> Key {
    let index = strtol10(arg::string(args, arg::LPC_VALUE)).wrapping_sub(1);
    s.requests.push(Request::PlayCustomPlaylist { index });
    EXIT
}

/// `MatchmakePlaygroup` (0x6765c0): `FUN_006747d0`, the matchmaking request.
fn matchmake_playgroup(s: &mut LogicState, _: Args) -> Key {
    s.requests.push(Request::MatchmakePlaygroup);
    EXIT
}

/// `FUN_006e74f0`(0, 0) on (G+0x2f3fc): the wished game is cleared: +0x2ce4 = 0, +0x2e7e = (0 != 0), +0x2e80 = 0,
/// +0x2d10 / +0x2d14 = the 64-bit "no game" value (`DAT_01179528` / `2c`, both 0 in the snapshot); the loop over
/// the game list is skipped for a zero id.
fn clear_desired_game(s: &mut LogicState) {
    let f = &mut s.hook_playlist;
    f.desired_game = 0;
    f.desired_game_set = false;
    f.desired_game_flag = false;
    f.desired_game_pair = (0, 0);
}

/// `QuickJoinGameDesired` (0x6765e0): G+0x2f490 (word) = 1, then `FUN_006e74f0`(0, 0).
fn quick_join_game_desired(s: &mut LogicState, _: Args) -> Key {
    s.hook_playlist.quick_join_desired = true;
    clear_desired_game(s);
    EXIT
}

/// `CreateGameDesired` (0x676610): G+0x2f708 (byte) = 1, then `FUN_006e74f0`(0, 0).
fn create_game_desired(s: &mut LogicState, _: Args) -> Key {
    s.hook_playlist.create_game_desired = true;
    clear_desired_game(s);
    EXIT
}

/// `SetRolloutSweetRevengeFromEasyDrive` (0x68d330): `atoi(lpcValue)` is stored in the local player's rollout
/// record (G+0x2f404, PM+0xa6b4 x 0xd8 + 0x82b8), then `FUN_0068ba20`(0) applies it. Model decision: a local
/// player outside the 12 records (the original writes out of the array) stores nothing.
fn set_rollout_sweet_revenge_from_easy_drive(s: &mut LogicState, args: Args) -> Key {
    let value = strtol10(arg::string(args, arg::LPC_VALUE));
    if let Some(slot) = s.players.local.filter(|&l| l < 12) {
        s.hook_playlist.sweet_revenge[slot] = value;
    }
    s.requests.push(Request::ApplyLocalRollout);
    EXIT
}

/// `SetFriendInviteIdEasyDrive` (0x67a800): `FUN_006797f0`: the string is evaluated (`FUN_00a50650`) and read as a
/// 64-bit decimal (`strtoll`) into G+0x2f498, 0 when it does not evaluate. Model decision: the string is the
/// number itself (no binding expression is evaluated).
fn set_friend_invite_id_easy_drive(s: &mut LogicState, args: Args) -> Key {
    s.hook_playlist.friend_invite_id = strtoll10(arg::string(args, arg::LPC_VALUE));
    EXIT
}

/// `strtoll(s, NULL, 10)` as a u64 (a negative value wraps as in the original's 64-bit store); 0 without digits.
fn strtoll10(s: &str) -> u64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for b in t.bytes().take_while(u8::is_ascii_digit) {
        v = v.saturating_mul(10).saturating_add((b - b'0') as i64);
    }
    (if neg { v.wrapping_neg() } else { v }) as u64
}

/// The friend `FUN_006749a0` / `FUN_006748d0` pick: the string is evaluated to a 1-based index (`FUN_00a4f220`,
/// minus 1) and must be inside the list (`FUN_0086f2f0`). Model decision: the string is the number itself.
fn friend_by_index(s: &LogicState, text: &str) -> Option<usize> {
    let i = strtol10(text).checked_sub(1)?;
    (i >= 0 && (i as usize) < s.hook_playlist.friends.len()).then_some(i as usize)
}

/// `SetJoinFriendIdByIndexBinding` (0x676640): `FUN_006749a0`: a friend in the list -> G+0x2f4ac = 1 and its id
/// into +0x2f4b0 and +0x2f4b8.
fn set_join_friend_id_by_index_binding(s: &mut LogicState, args: Args) -> Key {
    if let Some(i) = friend_by_index(s, arg::string(args, arg::LPC_VALUE)) {
        let f = &mut s.hook_playlist;
        let id = f.friends[i].id;
        f.join_friend_set = true;
        f.join_friend_id = id;
        f.join_friend_id_copy = id;
    }
    EXIT
}

/// `CanJoinAirportGameByFriendIndexBinding` (0x676680): `FUN_006748d0`: TRUE when the local player owns the
/// airport's pack (`FUN_00643c90`, `Entitlements::airport_pack_owned`), else when the picked friend's byte +0x63
/// is 0; FALSE for no such friend (offline: the list is empty).
fn can_join_airport_game_by_friend_index_binding(s: &mut LogicState, args: Args) -> Key {
    if s.entitlements.airport_pack_owned() {
        return exit_if(true);
    }
    let ok = friend_by_index(s, arg::string(args, arg::LPC_VALUE))
        .is_some_and(|i| !s.hook_playlist.friends[i].in_airport_game);
    exit_if(ok)
}

/// `OnMidGameJoin` (0x6766d0): `FUN_006e0c10` on the playlist manager: byte +0x84 = 0, +4 = G+0x2a9e0.
fn on_mid_game_join(s: &mut LogicState, _: Args) -> Key {
    let f = &mut s.hook_playlist;
    f.mid_game_join_flag = false;
    f.playlist_mgr_4 = f.playlist_source_2a9e0;
    EXIT
}

/// `ActivateFriendCarouselViewModel` (0x676cc0): (G+0x2f3e0)+0x14 (word) = 1.
fn activate_friend_carousel_view_model(s: &mut LogicState, _: Args) -> Key {
    s.hook_playlist.friend_carousel_active = true;
    EXIT
}

/// `DeactivateFriendCarouselViewModel` (0x676ce0): (G+0x2f3e0)+0x14 = 0.
fn deactivate_friend_carousel_view_model(s: &mut LogicState, _: Args) -> Key {
    s.hook_playlist.friend_carousel_active = false;
    EXIT
}

/// `UnlockMwlCache` (0x676df0): `FUN_0071cad0`: the lock byte +0x36 = 0 (`CacheMwlAndLock` sets it).
fn unlock_mwl_cache(s: &mut LogicState, _: Args) -> Key {
    s.hook_online.mwl_cache_locked = false;
    EXIT
}
