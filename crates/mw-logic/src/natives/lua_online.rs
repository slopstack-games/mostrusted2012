//! Online-area Lua natives answered as the original does with no service connected (research log 2026-11-18
//! "Lua natives, online batch"): the custom playlists, the online-session variables, the recommendation strings
//! and requests, the telemetry hooks and the cached speedwall. Handlers read from the exe's code
//! (`re/out/live/text.asm`, the dump). G = `*0x01177868`; G' = `*(*0x0117e9f4 + 0x2020)` (the UI-side object that
//! holds the recommendation strings and the telemetry); the playlist manager PM = `*(G+0x2f3fc)`.

use mw_formats::genesys::field_hash;
use mw_formats::json::Json;
use mw_lua::{Lua, Value};

use super::hook_args::strtol10;
use super::lua_args::{fit, ftol, int, push_int, string_or_invalid, toboolean, tonumber};
use super::{LogicState, Registry, Request};

type R = mw_lua::Result<Vec<Value>>;

/// Custom playlists in PM+0xc: `FUN_006eaf70` & co. index the array at +0x350 (0x188 bytes each); the array can
/// hold 10 before the manager's +0x13b0, each playlist 10 items of 0x20 bytes before its +0x140.
pub const MAX_LISTS: usize = 10;
pub const MAX_ITEMS: usize = 10;
/// The playlist's name buffer (+0x148): 0x19 characters.
pub const NAME_CAPACITY: usize = 0x19;

/// One item (0x20 bytes): +0 the id, +0x1c the restriction. The resource lookups of the id (+4..+0x18, from
/// `FUN_006e5dc0` / `FUN_006e5cb0` / `FUN_006e9c20`: searches in the event tables) are pure functions of the id:
/// the runtime derives them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlaylistItem {
    pub id: i32,
    pub restriction: i32,
}

/// One custom playlist (0x188 bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPlaylist {
    /// +0x140: the item count (`FUN_006e9cf0` sets 5).
    pub item_count: u32,
    pub items: [PlaylistItem; MAX_ITEMS],
    /// +0x14c: the name (wide characters in the original).
    pub name: String,
    /// +0x180.
    pub field_180: i32,
}

impl Default for CustomPlaylist {
    fn default() -> Self {
        Self { item_count: 5, items: [PlaylistItem::default(); MAX_ITEMS], name: String::new(), field_180: 0 }
    }
}

/// The custom playlist store (PM+0xc).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPlaylists {
    pub lists: Vec<CustomPlaylist>,
    /// +0x13b8: the store's state; `SaveCustomPlaylists` moves 5 (or more) to 6 = save pending, which the
    /// store's update consumes (runtime).
    pub state: i32,
    /// +0x13f4: a playlist changed since the last save.
    pub dirty: bool,
}

impl Default for CustomPlaylists {
    fn default() -> Self {
        Self { lists: vec![CustomPlaylist::default(); MAX_LISTS], state: 0, dirty: false }
    }
}

/// The recommendation strings and request state on G' (the "Recommends" download is the runtime's).
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendations {
    /// G'+0x2f620: the recommendation id (fixed string, 32 bytes of data: layout-derived, model decision).
    pub id: String,
    /// G'+0x2f648: the title string (128 bytes of data, layout-derived).
    pub title: String,
    /// G'+0x2f4d0: the name (wide, converted through a 0x11-character temporary).
    pub name: String,
    /// (G'.[0x2f454]+0x5a3c) +0x21: the index `FUN_007355b0` answered last (-1 = none yet / failed). The runtime
    /// sets it when it handles [`Request::RefreshRecommendations`].
    pub request_index: i32,
    /// +0x1c: the pending download's request id (-1 = no download pending).
    pub download_id: i32,
}

impl Default for Recommendations {
    fn default() -> Self {
        Self { id: String::new(), title: String::new(), name: String::new(), request_index: -1, download_id: -1 }
    }
}

/// The telemetry object at G'+0x2a068.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Telemetry {
    /// +0x318..+0x32c: six event ids (four-character codes).
    pub hooks: [u32; 6],
    /// +0..+0x50: up to ten (four-character code, string) pairs.
    pub data: Vec<(u32, String)>,
}

/// The id `RequestPurchaseOLP` asks for: `DAT_01182794`, set by a static initialiser with the immediate 0x17231
/// (the exe's `.data` snapshot; the pack id the store compares its items with).
pub const OLP_ITEM: i32 = 94769;
/// The store object's state in which it takes requests (+8).
pub const STORE_READY: i32 = 0xd;

/// The store / entitlement object's request fields (`FUN_0086f760` & co.). A request is taken only while the object
/// is in state 0xd and the service flag `*(+0xd70) +0x2a10c` is clear; the object's update consumes the flags
/// (the runtime).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreObject {
    /// +8: the object's state; the runtime sets [`STORE_READY`] when the store page is up (0 until then).
    pub state: i32,
    /// `*(+0xd70) +0x2a10c != 0`: the online service is busy; the runtime sets it.
    pub service_busy: bool,
    /// +0xdb8: the item of the pending purchase / download.
    pub item: i32,
    /// +0xdc8: a purchase is requested.
    pub purchase: bool,
    /// +0xdc9: a code redemption is requested.
    pub redeem: bool,
    /// +0xdca: a download is requested.
    pub download: bool,
}

impl StoreObject {
    fn takes_requests(&self) -> bool {
        self.state == STORE_READY && !self.service_busy
    }
}

/// A fixed wide-character buffer of the sign-in manager ({pointer, capacity} string: at most `cap - 1`
/// characters). Debug prints only the length: it holds credentials.
#[derive(Clone, Default, PartialEq)]
pub struct WideBuffer {
    pub units: Vec<u16>,
    pub cap: usize,
}

impl std::fmt::Debug for WideBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WideBuffer({} of {})", self.units.len(), self.cap)
    }
}

impl WideBuffer {
    fn with_cap(cap: usize) -> Self {
        Self { units: Vec::new(), cap }
    }

    /// The text held (lossy for unpaired surrogates).
    pub fn text(&self) -> String {
        String::from_utf16_lossy(&self.units)
    }

    /// `FUN_00a4c7b0` after the buffer was cleared: every byte of `text` widened with its sign (`movsx`), up to
    /// `cap - 1` characters.
    fn set_bytes(&mut self, text: &str) {
        self.units = text.bytes().take(self.cap.saturating_sub(1)).map(|b| b as i8 as i16 as u16).collect();
    }

    /// `FUN_00a4ab10` (`FUN_009e2810`): the text decoded as UTF-8 into 16-bit characters, up to `cap - 1`.
    fn set_utf8(&mut self, text: &str) {
        self.units = text.encode_utf16().take(self.cap.saturating_sub(1)).collect();
    }
}

/// The sign-in manager's e-mail / password / Origin id buffers (manager +0x414 / +0x6ac / +0x620 / +0x8b8 /
/// +0x944; capacities 0x101 / 0x101 / 0x41 / 0x41 / 0x11 from its constructor in 0x880000..).
#[derive(Debug, Clone, PartialEq)]
pub struct SignInCredentials {
    pub email: WideBuffer,
    pub new_email: WideBuffer,
    pub password: WideBuffer,
    pub new_password: WideBuffer,
    pub origin_id: WideBuffer,
}

impl Default for SignInCredentials {
    fn default() -> Self {
        Self {
            email: WideBuffer::with_cap(0x101),
            new_email: WideBuffer::with_cap(0x101),
            password: WideBuffer::with_cap(0x41),
            new_password: WideBuffer::with_cap(0x41),
            origin_id: WideBuffer::with_cap(0x11),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnlineLuaFields {
    pub custom_playlists: CustomPlaylists,
    /// The online session's variable tree (`*0x0117e9f4 + 0x470`): objects (namespaces) of variables.
    pub session: Json,
    pub recommendations: Recommendations,
    pub telemetry: Telemetry,
    /// The cached speedwall document (PM+0x20 +0x1918 +8, its root): `None` offline (nothing was downloaded).
    pub cached_speedwall: Option<Json>,
    /// The store / entitlement object (`*(G'+0x2f3e0) + 0x4ce0`), as the purchase natives touch it.
    pub store: StoreObject,
    /// The sign-in manager's credential buffers (`*(G'+0x2f3e0) + 0x10d0`).
    pub credentials: SignInCredentials,
    /// The image manager's (`*(G+0x2f430)`) +0x1cfc: its load state (1 / 2 = a transfer is running); the runtime
    /// sets it, 0 offline.
    pub image_state: i32,
}

impl Default for OnlineLuaFields {
    fn default() -> Self {
        Self {
            custom_playlists: CustomPlaylists::default(),
            session: Json::Object(Vec::new()),
            recommendations: Recommendations::default(),
            telemetry: Telemetry::default(),
            cached_speedwall: None,
            store: StoreObject::default(),
            credentials: SignInCredentials::default(),
            image_state: 0,
        }
    }
}

pub fn register(r: &mut Registry) {
    r.set_lua("PlayCriterionPlaylist", play_criterion_playlist);
    r.set_lua("PlayCustomPlaylist", play_custom_playlist);
    r.set_lua("EZPlayCustomPlaylist", ez_play_custom_playlist);
    r.set_lua("SetCustomPlaylistItem", set_custom_playlist_item);
    r.set_lua("SetCustomPlaylistRestriction", set_custom_playlist_restriction);
    r.set_lua("SaveCustomPlaylists", save_custom_playlists);
    r.set_lua("ClearCustomPlaylists", clear_custom_playlists);
    r.set_lua("DeleteCustomPlaylists", delete_custom_playlists);
    r.set_lua("RenameCustomPlaylist", rename_custom_playlist);
    r.set_lua("WriteIntToOnlineSession", write_int_to_online_session);
    r.set_lua("WriteBoolToOnlineSession", write_bool_to_online_session);
    r.set_lua("WriteDoubleToOnlineSession", write_double_to_online_session);
    r.set_lua("WriteStringToOnlineSession", write_string_to_online_session);
    r.set_lua("DeleteSessionNamespace", delete_session_namespace);
    r.set_lua("DeleteOnlineSessionVariable", delete_online_session_variable);
    r.set_lua("SetRecommendationId", set_recommendation_id);
    r.set_lua("SetRecommendationName", set_recommendation_name);
    r.set_lua("SetRecommendationTitleString", set_recommendation_title_string);
    r.set_lua("RequestRecommendationData", request_recommendation_data);
    r.set_lua("IsRecommendationDownloadComplete", is_recommendation_download_complete);
    r.set_lua("SetTelemetryHooks", set_telemetry_hooks);
    r.set_lua("SetTelemetryData", set_telemetry_data);
    r.set_lua("ReportTelemetryLiteGameModeStart", report_telemetry_lite_game_mode_start);
    r.set_lua("GetPositionInCachedSpeedwall", get_position_in_cached_speedwall);
    r.set_lua("UpdateCachedSpeedwall", update_cached_speedwall);
    r.set_lua("AreLobbyParamsValid", are_lobby_params_valid);
    r.set_lua("SyncTime", sync_time);
    r.set_lua("DisconnectFromServer", disconnect_from_server);
    r.set_lua("StartFriendRequest", start_friend_request);
    r.set_lua("StartFriendRequestByOriginId", start_friend_request_by_origin_id);
    r.set_lua("SpamFriend", spam_friend);
    r.set_lua("ShowFriendUi", show_friend_ui);
    r.set_lua("SetRolloutSweetRevenge", set_rollout_sweet_revenge);
    r.set_lua("IsSomethingDownloading", is_something_downloading);
    r.set_lua("RequestPurchaseOLP", request_purchase_olp);
    r.set_lua("RequestPurchase", request_purchase);
    r.set_lua("RequestRedeemCode", request_redeem_code);
    r.set_lua("RequestDownload", request_download);
    r.set_lua("RequestDownloadOnlinePass", request_download_online_pass);
    r.set_lua("SelectAdvert", select_advert);
    r.set_lua("SetEMail", set_email);
    r.set_lua("SetNewEMail", set_new_email);
    r.set_lua("SetPassword", set_password);
    r.set_lua("SetNewPassword", set_new_password);
    r.set_lua("SetOriginID", set_origin_id);
    r.set_lua("TryDeleteItemFromAutologViewModel", try_delete_item_from_autolog_view_model);
    r.set_lua("OnWallPostOrComment", on_wall_post_or_comment);
    r.set_lua("SendAddedRecoFriendTelemetry", send_added_reco_friend_telemetry);
    r.set_lua("SendTunedFriendTelemetry", send_tuned_friend_telemetry);
    r.set_lua("ReportedProfilePicForUser", reported_profile_pic_for_user);
}

// ---- playlists ------------------------------------------------------------------------------------------------

/// `PlayCriterionPlaylist()` (0x00675830): without a current playlist (`FUN_006e9a30`), `FUN_006f6040` on PM+0x14.
fn play_criterion_playlist(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    if !s.hook_playlist.playlist_active {
        s.requests.push(Request::PlayCriterionPlaylist);
    }
    Ok(vec![])
}

/// `PlayCustomPlaylist(index)` (0x0067a030): `_ftol2`(arg 1) -> `FUN_006f2730` on PM+0x14 (as the hook).
fn play_custom_playlist(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.requests.push(Request::PlayCustomPlaylist { index: int(&args, 1) });
    Ok(vec![])
}

/// `EZPlayCustomPlaylist(text)` (0x0067a070): `atoi(arg 1)` - 1 -> `FUN_006f2730`.
fn ez_play_custom_playlist(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let index = strtol10(&string_or_invalid(&args, 1)).wrapping_sub(1);
    s.requests.push(Request::PlayCustomPlaylist { index });
    Ok(vec![])
}

/// The playlist `list` of the store; the original does not check the index (the arrays are in the manager's
/// memory), model decision: indices outside the 10 playlists / 10 items its layout holds do nothing.
fn list_mut(s: &mut LogicState, list: i32) -> Option<&mut CustomPlaylist> {
    usize::try_from(list).ok().and_then(|i| s.online.custom_playlists.lists.get_mut(i))
}

fn item_mut(s: &mut LogicState, list: i32, item: i32) -> Option<&mut PlaylistItem> {
    let item = usize::try_from(item).ok()?;
    list_mut(s, list)?.items.get_mut(item)
}

/// `SetCustomPlaylistItem(list, item, id)` (0x0067a0c0 -> `FUN_006eaf70` on PM+0xc): item +0 = id (the resource
/// lookups follow from it), the store's dirty byte (+0x13f4) = 1.
fn set_custom_playlist_item(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (list, item, id) = (int(&args, 1), int(&args, 2), int(&args, 3));
    if let Some(it) = item_mut(s, list, item) {
        it.id = id;
    }
    s.online.custom_playlists.dirty = true;
    Ok(vec![])
}

/// `SetCustomPlaylistRestriction(list, item, value)` (0x0067a120 -> `FUN_006eb000` -> `FUN_006e9c20`): item +0x1c
/// = value, dirty.
fn set_custom_playlist_restriction(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (list, item, value) = (int(&args, 1), int(&args, 2), int(&args, 3));
    if let Some(it) = item_mut(s, list, item) {
        it.restriction = value;
    }
    s.online.custom_playlists.dirty = true;
    Ok(vec![])
}

/// `SaveCustomPlaylists()` (0x006758a0 -> `FUN_006e0d80`): dirty and state >= 5 (signed) -> state = 6.
fn save_custom_playlists(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let p = &mut s.online.custom_playlists;
    if p.dirty && p.state >= 5 {
        p.state = 6;
    }
    Ok(vec![])
}

/// `ClearCustomPlaylists(list)` (0x0067a180 -> `FUN_006eb040` -> `FUN_006e9f10`): the first `item_count` item ids
/// are set to 0 (restrictions stay), dirty.
fn clear_custom_playlists(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if let Some(l) = list_mut(s, int(&args, 1)) {
        let n = (l.item_count as usize).min(MAX_ITEMS);
        for it in &mut l.items[..n] {
            it.id = 0;
        }
    }
    s.online.custom_playlists.dirty = true;
    Ok(vec![])
}

/// `DeleteCustomPlaylists(list)` (0x0067a1c0 -> `FUN_006eb070`): the playlist is reset (+0x180 = 0, empty name,
/// `FUN_006e9cf0`: five empty items), dirty.
fn delete_custom_playlists(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if let Some(l) = list_mut(s, int(&args, 1)) {
        *l = CustomPlaylist::default();
    }
    s.online.custom_playlists.dirty = true;
    Ok(vec![])
}

/// `RenameCustomPlaylist(list, name)` (0x0067a200 -> `FUN_006e3230`): when the name is valid UTF-8
/// (`FUN_009e75b0`) and has at most 0x19 characters (`FUN_009e1f40` counts non-continuation bytes) it replaces the
/// name; dirty either way.
fn rename_custom_playlist(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let list = int(&args, 1);
    let name = string_or_invalid(&args, 2);
    if let Some(l) = list_mut(s, list)
        && name.chars().count() <= NAME_CAPACITY
    {
        l.name = name;
    }
    s.online.custom_playlists.dirty = true;
    Ok(vec![])
}

// ---- online session ---------------------------------------------------------------------------------------------

fn find(entries: &[(String, Json)], name: &str) -> Option<usize> {
    let h = field_hash(name);
    entries.iter().position(|(k, _)| field_hash(k) == h)
}

/// `FUN_007e1890`: the namespace node of a dotted path (objects created on the way when `create`); an empty
/// segment ends the walk at the node so far. Model decision: a segment that exists but is not an object fails.
fn namespace<'a>(root: &'a mut Json, ns: &str, create: bool) -> Option<&'a mut Json> {
    let mut node = root;
    let mut walked = false;
    for seg in fit(ns, 128).split_terminator('.') {
        if seg.is_empty() {
            return walked.then_some(node);
        }
        let Json::Object(entries) = node else { return None };
        let i = match find(entries, seg) {
            Some(i) => i,
            None if create => {
                entries.push((seg.to_string(), Json::Object(Vec::new())));
                entries.len() - 1
            }
            None => return None,
        };
        node = &mut entries[i].1;
        walked = true;
    }
    walked.then_some(node)
}

/// `FUN_009f6670`: set the member `name` of the node (replace in place, or append).
fn set_member(node: &mut Json, name: &str, value: Json) {
    if let Json::Object(entries) = node {
        match find(entries, name) {
            Some(i) => entries[i].1 = value,
            None => entries.push((name.to_string(), value)),
        }
    }
}

/// The write of the four `Write*ToOnlineSession` natives: `ns` (when not empty) is created, then the variable set.
fn write_session(s: &mut LogicState, name: &str, value: Json, ns: &str) {
    let root = &mut s.online.session;
    let target = if ns.is_empty() { Some(root) } else { namespace(root, ns, true) };
    if let Some(node) = target {
        set_member(node, name, value);
    }
}

/// The optional namespace argument: read when `lua_gettop` > `after`, else the empty string.
fn ns_arg(args: &[Value], i: usize) -> String {
    if args.len() >= i { string_or_invalid(args, i) } else { String::new() }
}

/// `WriteIntToOnlineSession(name, value [, namespace])` (0x007ee630 -> `FUN_007e3090`): a tag-3 variable, the
/// 64-bit `_ftol2` of the value.
fn write_int_to_online_session(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (name, v, ns) = (string_or_invalid(&args, 1), ftol(tonumber(&args, 2)), ns_arg(&args, 3));
    write_session(s, &name, Json::Int(v), &ns);
    Ok(vec![])
}

/// `WriteBoolToOnlineSession(name, value [, namespace])` (0x007ee6c0 -> `FUN_007e3100`): tag 5.
fn write_bool_to_online_session(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (name, v, ns) = (string_or_invalid(&args, 1), toboolean(&args, 2), ns_arg(&args, 3));
    write_session(s, &name, Json::Bool(v), &ns);
    Ok(vec![])
}

/// `WriteDoubleToOnlineSession(name, value [, namespace])` (0x007ee740 -> `FUN_007e3160`): tag 4.
fn write_double_to_online_session(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (name, v, ns) = (string_or_invalid(&args, 1), tonumber(&args, 2), ns_arg(&args, 3));
    write_session(s, &name, Json::Double(v), &ns);
    Ok(vec![])
}

/// `WriteStringToOnlineSession(name, value [, namespace])` (0x007ee7d0 -> `FUN_007e3020`): tag 2 (a copy of the
/// string; `"<invalid>"` when the argument is not a string).
fn write_string_to_online_session(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (name, v, ns) = (string_or_invalid(&args, 1), string_or_invalid(&args, 2), ns_arg(&args, 3));
    write_session(s, &name, Json::Str(v), &ns);
    Ok(vec![])
}

/// `DeleteSessionNamespace(path)` (0x007ee850 -> `FUN_007dd660`): the dotted path is walked through existing
/// objects (an empty segment, a missing one or a non-object ends it without a change); its last segment is
/// removed from its parent.
fn delete_session_namespace(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let path = string_or_invalid(&args, 1);
    let segs: Vec<&str> = fit(&path, 128).split_terminator('.').collect();
    let Some((last, parents)) = segs.split_last() else { return Ok(vec![]) };
    // The walk returns at the first empty segment, before anything is deleted.
    if segs.iter().any(|g| g.is_empty()) {
        return Ok(vec![]);
    }
    let mut node = &mut s.online.session;
    for seg in parents {
        let Json::Object(entries) = node else { return Ok(vec![]) };
        let Some(i) = find(entries, seg) else { return Ok(vec![]) };
        node = &mut entries[i].1;
    }
    if let Json::Object(entries) = node {
        entries.retain(|(k, _)| k != last);
    }
    Ok(vec![])
}

/// `DeleteOnlineSessionVariable(name [, namespace])` (0x007ee890 -> `FUN_007e31c0`): the member `name` (exact
/// string match, `FUN_009f16e0`) of the namespace (not created) or of the root.
fn delete_online_session_variable(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    let ns = if args.len() > 1 { string_or_invalid(&args, 2) } else { String::new() };
    let root = &mut s.online.session;
    let node = if ns.is_empty() { Some(root) } else { namespace(root, &ns, false) };
    if let Some(Json::Object(entries)) = node {
        entries.retain(|(k, _)| *k != name);
    }
    Ok(vec![])
}

// ---- recommendations ----------------------------------------------------------------------------------------------

/// `strncat(buffer, text, size - strlen(buffer) - 1)` on a fixed string of `size` data bytes (cut at a character
/// boundary).
fn strncat(buf: &mut String, text: &str, size: usize) {
    let room = size.saturating_sub(buf.len() + 1);
    buf.push_str(fit(text, room + 1));
}

/// `SetRecommendationId(text)` (0x007e9a90): appended (`strncat`) to the id string G'+0x2f620 (data size 32).
fn set_recommendation_id(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    strncat(&mut s.online.recommendations.id, &string_or_invalid(&args, 1), 32);
    Ok(vec![])
}

/// `SetRecommendationTitleString(text)` (0x007e9b00): appended to the title string G'+0x2f648 (data size 128).
fn set_recommendation_title_string(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    strncat(&mut s.online.recommendations.title, &string_or_invalid(&args, 1), 128);
    Ok(vec![])
}

/// `SetRecommendationName(text)` (0x007ee950): the text is converted to wide characters in a temporary of 0x11
/// (`FUN_00a4c7b0`), the target G'+0x2f4d0 is emptied and the temporary appended (`FUN_00a4d460`): the name is
/// the text cut to 16 characters.
fn set_recommendation_name(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.recommendations.name = string_or_invalid(&args, 1).chars().take(16).collect();
    Ok(vec![])
}

/// `RequestRecommendationData()` (0x007ea370): `FUN_007355b0`(0, 0) on (G'.[0x2f454])+0x5a3c, its result pushed.
/// When a result is cached (+0x21 != -1) the function answers it without work; otherwise the "Recommends"
/// document is requested ([`Request::RefreshRecommendations`]) and the answer is the index it leaves, -1 until the
/// runtime handled the request (no service: it stays -1).
fn request_recommendation_data(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let idx = s.online.recommendations.request_index;
    if idx == -1 {
        s.requests.push(Request::RefreshRecommendations);
    }
    Ok(vec![push_int(s.online.recommendations.request_index)])
}

/// `IsRecommendationDownloadComplete(id)` (0x007ea3c0): pushes whether the recommendations' request id (+0x1c,
/// `FUN_0071d250`; the argument is read and ignored) is -1.
fn is_recommendation_download_complete(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(s.online.recommendations.download_id == -1)])
}

// ---- telemetry ---------------------------------------------------------------------------------------------

/// `FUN_004c2a70`: a four-character string as a big-endian code (sign-extended bytes), else 0.
fn four_cc(text: &str) -> u32 {
    let b = text.as_bytes();
    if b.len() != 4 {
        return 0;
    }
    b.iter().fold(0i32, |acc, &c| (acc << 8) | (c as i8 as i32)) as u32
}

/// `SetTelemetryHooks(h1..h6)` (0x007ea550 -> `FUN_00655bc0` on G'+0x2a068): each non-empty string becomes a
/// four-character code in hook slot 1..6 (+0x318..+0x32c).
fn set_telemetry_hooks(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    for i in 0..6 {
        let text = string_or_invalid(&args, i + 1);
        if !text.is_empty() {
            s.online.telemetry.hooks[i] = four_cc(&text);
        }
    }
    Ok(vec![])
}

/// `SetTelemetryData(code, value)` (0x007ea630 -> `FUN_0065a3f0`): below ten entries, the pair (code, copy of the
/// value) is appended.
fn set_telemetry_data(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let (code, value) = (four_cc(&string_or_invalid(&args, 1)), string_or_invalid(&args, 2));
    if s.online.telemetry.data.len() != 10 {
        s.online.telemetry.data.push((code, value));
    }
    Ok(vec![])
}

/// `ReportTelemetryLiteGameModeStart()` (0x00677de0): only `lua_gettop`.
fn report_telemetry_lite_game_mode_start(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

// ---- cached speedwall ---------------------------------------------------------------------------------------------

/// `FUN_0081da70` (mode 0) on the cached document: the local player's position for the race clock `ms`.
/// `LocalPosition` (an int) is the start; no `speedwall` object -> that; no `Items` array -> 1; the first item
/// (before the one at `LocalPosition`) whose `TimeMs` is above `ms` gives its 1-based index; a result below 1 is
/// the item count + 1.
fn speedwall_position(doc: &Json, ms: i64) -> u32 {
    if !matches!(doc, Json::Object(_)) {
        return 0;
    }
    let mut pos = match doc.get("LocalPosition") {
        Some(Json::Int(v)) => *v as u32,
        _ => 0,
    };
    let Some(wall @ Json::Object(_)) = doc.get("speedwall") else { return pos };
    let Some(Json::Array(items)) = wall.get("Items") else { return 1 };
    let len = items.len().min(0xffff) as u32;
    let mut i: u32 = 0;
    if len != 0 {
        loop {
            let next = i + 1;
            if next == pos {
                break;
            }
            if let Some(item @ Json::Object(_)) = items.get(i as usize)
                && let Some(Json::Int(t)) = item.get("TimeMs")
                && ms < *t
            {
                pos = next;
                break;
            }
            i = next;
            if i >= len {
                break;
            }
        }
    }
    if (pos as i32) < 1 {
        pos = len + 1;
    }
    pos
}

/// `GetPositionInCachedSpeedwall()` (0x00675710 -> `FUN_006e0880` on PM+0x20): the race clock GMM+0x2d28 (ticks)
/// in milliseconds (`FUN_0046d130`: / 3 / 1000) against the cached document; 0 without one (offline).
fn get_position_in_cached_speedwall(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let ticks = (u64::from(s.game_modes.word_2d2c) << 32 | u64::from(s.game_modes.word_2d28)) as i64;
    let ms = ticks / 3 / 1000;
    let pos = s.online.cached_speedwall.as_ref().map_or(0, |doc| speedwall_position(doc, ms));
    Ok(vec![push_int(pos as i32)])
}

/// `UpdateCachedSpeedwall()` (0x006802b0 -> `FUN_0067fbf0`): the cached document takes the local player's result
/// (`FUN_006e08b0` for a time event, `FUN_006e0900` for a score event, by the event record's type byte): the
/// event record and the player's vehicle are the runtime's.
fn update_cached_speedwall(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    s.requests.push(Request::UpdateCachedSpeedwall);
    Ok(vec![])
}

// ---- service-facing natives (second online batch) ----------------------------------------------------------------

/// `AreLobbyParamsValid()` (0x00677bf0): `lua_settop(0)`, then pushes whether the pending event (G+0x2f6f8) is not 0.
fn are_lobby_params_valid(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![Value::Bool(s.game.pending_event != 0)])
}

/// `SyncTime()` (0x0068afc0): the message 5 is posted on the game message queue (`FUN_0069a650`, flag 8 of the
/// queue's owner set meanwhile).
fn sync_time(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    s.requests.push(Request::PostQueueMessage { function: 0x69a650, id: 5 });
    Ok(vec![])
}

/// `DisconnectFromServer()` (0x00677f40 -> `FUN_0083dc00` on G+0x2f068): the request object is reset, takes the
/// next request id (`DAT_01105060`) and queues the message 0x37 (size 0xc). The object is slot 2 of
/// [`Request::NetSend`] (0 / 1 are the HSM owners' objects).
fn disconnect_from_server(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    s.requests.push(Request::NetSend { slot: 2, message: crate::hsm::behaviours::online::NetMessage::Disconnect });
    Ok(vec![])
}

/// The friend-list action slot 1 (player manager +0xa6d0 +0x22ac / +0x22bd = +0xc97c / +0xc98d, the cells
/// `HasFriendListActionFinished` / `GetFriendListActionResult` read at index 1): result -1, not finished.
fn friend_slot_pending(s: &mut LogicState) {
    let q = &mut s.sim.queries;
    q.friend_action_result.insert(1, -1);
    q.friend_action_finished.insert(1, false);
}

/// `StartFriendRequest(name)` (0x007e9d70 -> `FUN_004174d0`): the name is copied (17 characters) and only tested
/// for being empty. Empty: the slot is finished with result 1. Else the slot is pending and the friends UI of
/// the platform service is opened (`FUN_00ae1ea0`(`FUN_00ae10d0`()), a no-op without the Origin singleton
/// `0x011a2778`; [`Request::ShowFriendUi`]). Pushes 1.
fn start_friend_request(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let name = string_or_invalid(&args, 1);
    friend_slot_pending(s);
    if name.is_empty() {
        let q = &mut s.sim.queries;
        q.friend_action_result.insert(1, 1);
        q.friend_action_finished.insert(1, true);
    } else {
        s.requests.push(Request::ShowFriendUi);
    }
    Ok(vec![push_int(1)])
}

/// `StartFriendRequestByOriginId(id)` (0x007e9df0 -> `FUN_00417520`): the argument is read (`lua_tonumber`, 64-bit
/// `__ftol`) and ignored; the slot is pending and the friends UI is opened. Pushes 1.
fn start_friend_request_by_origin_id(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    friend_slot_pending(s);
    s.requests.push(Request::ShowFriendUi);
    Ok(vec![push_int(1)])
}

/// `SpamFriend(name)` (0x007e9c00): the string is read, nothing is done.
fn spam_friend(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `ShowFriendUi()` (0x007e9ef0): the platform service's friends UI ([`Request::ShowFriendUi`]); pushes true.
fn show_friend_ui(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    s.requests.push(Request::ShowFriendUi);
    Ok(vec![Value::Bool(true)])
}

/// `SetRolloutSweetRevenge(n)` (0x0068d2a0): `(int)n` stored in the local player's rollout record (G+0x2f404,
/// PM+0xa6b4 x 0xd8 + 0x82b8), then `FUN_0068ba20`(0) applies it: the same effect as the hook
/// `SetRolloutSweetRevengeFromEasyDrive`. Model decision: a local player outside the 12 records stores nothing.
fn set_rollout_sweet_revenge(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let value = int(&args, 1);
    if let Some(slot) = s.players.local.filter(|&l| l < 12) {
        s.hook_playlist.sweet_revenge[slot] = value;
    }
    s.requests.push(Request::ApplyLocalRollout);
    Ok(vec![])
}

/// `IsSomethingDownloading()` (0x007f2480): the image manager's state (G+0x2f430 +0x1cfc) is 1 or 2, or the
/// download list (G+0x2f42c +0x4d8, count +0xb478) is not empty.
fn is_something_downloading(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let busy = matches!(s.online.image_state, 1 | 2) || !s.menus.downloads.is_empty();
    Ok(vec![Value::Bool(busy)])
}

/// `RequestPurchaseOLP()` (0x007ea220 -> `FUN_0086f790` on the store object): when it is in state 0xd and the
/// service flag is clear, the purchase flag (+0xdc8) is set with the item [`OLP_ITEM`] (+0xdb8).
fn request_purchase_olp(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let st = &mut s.online.store;
    if st.takes_requests() {
        st.purchase = true;
        st.item = OLP_ITEM;
    }
    Ok(vec![])
}

/// `RequestPurchase(item)` (0x007ea250 -> `FUN_0086f760`): as [`request_purchase_olp`] with the argument as item.
fn request_purchase(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let st = &mut s.online.store;
    if st.takes_requests() {
        st.purchase = true;
        st.item = int(&args, 1);
    }
    Ok(vec![])
}

/// `RequestRedeemCode()` (0x007ea2b0 -> `FUN_0086f740`): the redeem flag (+0xdc9) when the store takes requests.
fn request_redeem_code(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let st = &mut s.online.store;
    if st.takes_requests() {
        st.redeem = true;
    }
    Ok(vec![])
}

/// `RequestDownload(item)` (0x007ea2e0 -> `FUN_0086f7c0`): item (+0xdb8) and the download flag (+0xdca).
fn request_download(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let st = &mut s.online.store;
    if st.takes_requests() {
        st.item = int(&args, 1);
        st.download = true;
    }
    Ok(vec![])
}

/// `RequestDownloadOnlinePass()` (0x007ea340 -> `FUN_008739b0`): the item is the selected pack's id (the object
/// at store +0xd78, found by id in `FUN_00642a90`), the download flag is set. Model decision: with no pack
/// selected (the original uses its default pack object, whose id is a `.bss` global not resolved) the item is 0.
fn request_download_online_pass(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    let item = s.menus.selected_pack as i32;
    let st = &mut s.online.store;
    if st.takes_requests() {
        st.item = item;
        st.download = true;
    }
    Ok(vec![])
}

/// `SelectAdvert(kind, url, a, b)` (0x007ea4c0 -> `FUN_0065a4d0` on G'+0x2a068): only the kind "Advert" is
/// handled (byte compare). The strings a and b are copied into the advert object (32-byte buffers; one that does
/// not fit is not copied) and the advert's download is started through the service ([`Request::SelectAdvert`]).
fn select_advert(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    if string_or_invalid(&args, 1) == "Advert" {
        let keep = |i: usize| Some(string_or_invalid(&args, i)).filter(|t| t.len() < 0x20);
        s.requests.push(Request::SelectAdvert { url: string_or_invalid(&args, 2), a: keep(3), b: keep(4) });
    }
    Ok(vec![])
}

/// `SetEMail(text)` (0x007ea830): the sign-in manager's e-mail buffer (+0x414) cleared, then the bytes
/// appended widened with their sign (`FUN_00a4c7b0`), at most 0x100 characters.
fn set_email(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.credentials.email.set_bytes(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `SetNewEMail(text)` (0x007ea880): the same on the new e-mail buffer (+0x6ac).
fn set_new_email(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.credentials.new_email.set_bytes(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `SetPassword(text)` (0x007ea8d0): the same on the password buffer (+0x620, 0x40 characters).
fn set_password(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.credentials.password.set_bytes(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `SetNewPassword(text)` (0x007ea920): the same on the new password buffer (+0x8b8).
fn set_new_password(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.credentials.new_password.set_bytes(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `SetOriginID(text)` (0x007ea970): the Origin id buffer (+0x944, 0x10 characters) cleared, then the text decoded
/// as UTF-8 (`FUN_00a4ab10`).
fn set_origin_id(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.online.credentials.origin_id.set_utf8(&string_or_invalid(&args, 1));
    Ok(vec![])
}

/// `TryDeleteItemFromAutologViewModel(view model, item)` (0x007eabb0 -> `FUN_0081f270` on G+0x2f42c): only when a
/// view model of that name is loaded (`FUN_0081dfd0`) the item is deleted from it (`FUN_009f8800`: a path
/// into the view model, the runtime's, [`Request::DeleteViewModelItem`]).
fn try_delete_item_from_autolog_view_model(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    let view_model = string_or_invalid(&args, 1);
    if s.misc.loaded_view_models.contains(&view_model) {
        s.requests.push(Request::DeleteViewModelItem { view_model, item: string_or_invalid(&args, 2) });
    }
    Ok(vec![])
}

/// `OnWallPostOrComment()` (0x007eb3c0 -> `FUN_00639a90`, an empty function): nothing. (Not the hook of the same
/// name.)
fn on_wall_post_or_comment(_: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    Ok(vec![])
}

/// `SendAddedRecoFriendTelemetry()` (0x007eb3f0 -> `FUN_007e8d00`): the telemetry event `GAME` `ALOG` `FRAD` with
/// `cdur` = a session time (`FUN_006e3ab0` / 3 / 1000000), posted when the network layer allows
/// ([`Request::AddedRecoFriendTelemetry`]; the time is the runtime's).
fn send_added_reco_friend_telemetry(s: &mut LogicState, _: &mut Lua, _: Vec<Value>) -> R {
    s.requests.push(Request::AddedRecoFriendTelemetry);
    Ok(vec![])
}

/// `SendTunedFriendTelemetry(value)` (0x007eb410): the telemetry event `GAME` `ALOG` `TUNR` with `tune` = `(int)value`
/// ([`Request::TunedFriendTelemetry`]).
fn send_tuned_friend_telemetry(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.requests.push(Request::TunedFriendTelemetry { value: int(&args, 1) });
    Ok(vec![])
}

/// `ReportedProfilePicForUser(name)` (0x007eb470 -> `FUN_007e0750` on G+0x2f430): the image slot named `name`
/// (when its name resolves, `FUN_0081cca0`) whose kind (+0xe) is 5 and state (+8) 2 becomes 3 (released); the
/// image table is the runtime's ([`Request::ReportedProfilePic`]).
fn reported_profile_pic_for_user(s: &mut LogicState, _: &mut Lua, args: Vec<Value>) -> R {
    s.requests.push(Request::ReportedProfilePic { name: string_or_invalid(&args, 1) });
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::natives::LogicHost;

    fn setup() -> (Lua, LogicHost) {
        let mut lua = Lua::new();
        let host = LogicHost::default();
        host.install_lua(&mut lua);
        (lua, host)
    }

    fn call(lua: &mut Lua, name: &str, args: Vec<Value>) -> Vec<Value> {
        lua.call_global(name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn st(host: &LogicHost) -> std::cell::RefMut<'_, LogicState> {
        host.state.borrow_mut()
    }

    fn n(v: f64) -> Value {
        Value::Number(v)
    }

    fn s(v: &str) -> Value {
        Value::from(v)
    }

    #[test]
    fn playlists() {
        let (mut lua, host) = setup();
        call(&mut lua, "PlayCriterionPlaylist", vec![]);
        st(&host).hook_playlist.playlist_active = true;
        call(&mut lua, "PlayCriterionPlaylist", vec![]);
        call(&mut lua, "PlayCustomPlaylist", vec![n(2.9)]);
        call(&mut lua, "EZPlayCustomPlaylist", vec![s("3")]);
        assert_eq!(
            st(&host).requests,
            vec![
                Request::PlayCriterionPlaylist,
                Request::PlayCustomPlaylist { index: 2 },
                Request::PlayCustomPlaylist { index: 2 },
            ]
        );
        call(&mut lua, "SaveCustomPlaylists", vec![]);
        assert_eq!(st(&host).online.custom_playlists.state, 0);
        call(&mut lua, "SetCustomPlaylistItem", vec![n(1.0), n(2.0), n(77.0)]);
        call(&mut lua, "SetCustomPlaylistRestriction", vec![n(1.0), n(2.0), n(9.0)]);
        call(&mut lua, "RenameCustomPlaylist", vec![n(1.0), s("Mine")]);
        {
            let p = &st(&host).online.custom_playlists;
            assert!(p.dirty);
            assert_eq!(p.lists[1].items[2], PlaylistItem { id: 77, restriction: 9 });
            assert_eq!(p.lists[1].name, "Mine");
        }
        // Too long a name is refused; the store is still dirty.
        call(&mut lua, "RenameCustomPlaylist", vec![n(1.0), s("0123456789012345678901234567")]);
        assert_eq!(st(&host).online.custom_playlists.lists[1].name, "Mine");
        call(&mut lua, "ClearCustomPlaylists", vec![n(1.0)]);
        {
            let it = st(&host).online.custom_playlists.lists[1].items[2];
            assert_eq!((it.id, it.restriction), (0, 9));
        }
        st(&host).online.custom_playlists.state = 5;
        call(&mut lua, "SaveCustomPlaylists", vec![]);
        assert_eq!(st(&host).online.custom_playlists.state, 6);
        call(&mut lua, "DeleteCustomPlaylists", vec![n(1.0)]);
        assert_eq!(st(&host).online.custom_playlists.lists[1], CustomPlaylist::default());
        // An out-of-range list does nothing.
        call(&mut lua, "SetCustomPlaylistItem", vec![n(99.0), n(0.0), n(1.0)]);
        assert!(st(&host).unported.is_empty());
    }

    #[test]
    fn session_variables() {
        let (mut lua, host) = setup();
        call(&mut lua, "WriteIntToOnlineSession", vec![s("lap"), n(3.9)]);
        call(&mut lua, "WriteBoolToOnlineSession", vec![s("ready"), Value::Bool(true), s("Race.Info")]);
        call(&mut lua, "WriteDoubleToOnlineSession", vec![s("t"), n(1.5), s("Race")]);
        call(&mut lua, "WriteStringToOnlineSession", vec![s("name"), s("abc"), s("Race.Info")]);
        call(&mut lua, "WriteIntToOnlineSession", vec![s("lap"), n(4.0)]);
        let want = Json::parse(r#"{"lap": 4, "Race": {"Info": {"ready": true, "name": "abc"}, "t": 1.5}}"#).unwrap();
        assert_eq!(st(&host).online.session, want);
        call(&mut lua, "DeleteOnlineSessionVariable", vec![s("ready"), s("Race.Info")]);
        call(&mut lua, "DeleteOnlineSessionVariable", vec![s("lap")]);
        call(&mut lua, "DeleteOnlineSessionVariable", vec![s("x"), s("No.Such")]);
        assert_eq!(st(&host).online.session, Json::parse(r#"{"Race": {"Info": {"name": "abc"}, "t": 1.5}}"#).unwrap());
        call(&mut lua, "DeleteSessionNamespace", vec![s("Race.Info")]);
        assert_eq!(st(&host).online.session, Json::parse(r#"{"Race": {"t": 1.5}}"#).unwrap());
        call(&mut lua, "DeleteSessionNamespace", vec![s("Nope.Info")]);
        call(&mut lua, "DeleteSessionNamespace", vec![s("Race")]);
        assert_eq!(st(&host).online.session, Json::Object(Vec::new()));
    }

    #[test]
    fn recommendations() {
        let (mut lua, host) = setup();
        call(&mut lua, "SetRecommendationId", vec![s("ev")]);
        call(&mut lua, "SetRecommendationId", vec![s("_12")]);
        call(&mut lua, "SetRecommendationTitleString", vec![s("Title")]);
        call(&mut lua, "SetRecommendationName", vec![s("A name that is too long")]);
        {
            let r = &st(&host).online.recommendations;
            assert_eq!((r.id.as_str(), r.title.as_str()), ("ev_12", "Title"));
            assert_eq!(r.name, "A name that is t");
        }
        // The fixed string keeps its terminator: at most 31 characters.
        call(&mut lua, "SetRecommendationId", vec![s("0123456789012345678901234567890123456789")]);
        assert_eq!(st(&host).online.recommendations.id.len(), 31);
        let r = call(&mut lua, "RequestRecommendationData", vec![]);
        assert_eq!(r[0].as_number(), Some(-1.0));
        assert_eq!(st(&host).requests, vec![Request::RefreshRecommendations]);
        assert!(matches!(call(&mut lua, "IsRecommendationDownloadComplete", vec![n(1.0)])[0], Value::Bool(true)));
        {
            let mut g = st(&host);
            g.online.recommendations.request_index = 7;
            g.online.recommendations.download_id = 3;
        }
        let r = call(&mut lua, "RequestRecommendationData", vec![]);
        assert_eq!(r[0].as_number(), Some(7.0));
        assert_eq!(st(&host).requests.len(), 1);
        assert!(matches!(call(&mut lua, "IsRecommendationDownloadComplete", vec![n(1.0)])[0], Value::Bool(false)));
    }

    #[test]
    fn telemetry() {
        let (mut lua, host) = setup();
        call(&mut lua, "SetTelemetryHooks", vec![s("ABCD"), s(""), s("toolong")]);
        let t = st(&host).online.telemetry.hooks;
        assert_eq!(t[0], 0x4142_4344);
        assert_eq!(t[1], 0);
        // Missing arguments read as "<invalid>" (not empty): code 0.
        assert_eq!(t[2], 0);
        for i in 0..12 {
            call(&mut lua, "SetTelemetryData", vec![s("KEY1"), s(&format!("v{i}"))]);
        }
        let d = st(&host).online.telemetry.data.clone();
        assert_eq!(d.len(), 10);
        assert_eq!(d[9], (0x4b45_5931, "v9".to_string()));
        assert!(call(&mut lua, "ReportTelemetryLiteGameModeStart", vec![s("x")]).is_empty());
    }

    #[test]
    fn speedwall() {
        let (mut lua, host) = setup();
        assert_eq!(call(&mut lua, "GetPositionInCachedSpeedwall", vec![])[0].as_number(), Some(0.0));
        st(&host).online.cached_speedwall = Some(
            Json::parse(
                r#"{"LocalPosition": 5, "speedwall": {"Items": [{"TimeMs": 10}, {"TimeMs": 20}, {"TimeMs": 30}]}}"#,
            )
            .unwrap(),
        );
        // 15 ms = 45000 ticks: faster than item 2 -> position 2.
        st(&host).game_modes.word_2d28 = 45_000;
        assert_eq!(call(&mut lua, "GetPositionInCachedSpeedwall", vec![])[0].as_number(), Some(2.0));
        // Slower than all: LocalPosition (5) is never reached by the three items -> position stays 5.
        st(&host).game_modes.word_2d28 = 300_000;
        assert_eq!(call(&mut lua, "GetPositionInCachedSpeedwall", vec![])[0].as_number(), Some(5.0));
        st(&host).online.cached_speedwall = Some(Json::parse(r#"{"speedwall": {"Items": [{"TimeMs": 10}]}}"#).unwrap());
        assert_eq!(call(&mut lua, "GetPositionInCachedSpeedwall", vec![])[0].as_number(), Some(2.0));
        call(&mut lua, "UpdateCachedSpeedwall", vec![]);
        assert_eq!(st(&host).requests, vec![Request::UpdateCachedSpeedwall]);
    }

    #[test]
    fn second_batch_service_natives() {
        let (mut lua, host) = setup();
        assert!(matches!(call(&mut lua, "AreLobbyParamsValid", vec![])[0], Value::Bool(false)));
        st(&host).game.pending_event = 5;
        assert!(matches!(call(&mut lua, "AreLobbyParamsValid", vec![])[0], Value::Bool(true)));
        call(&mut lua, "SyncTime", vec![]);
        call(&mut lua, "DisconnectFromServer", vec![]);
        call(&mut lua, "SendAddedRecoFriendTelemetry", vec![]);
        call(&mut lua, "SendTunedFriendTelemetry", vec![n(3.9)]);
        call(&mut lua, "TryDeleteItemFromAutologViewModel", vec![s("Autolog"), s("x")]);
        st(&host).misc.loaded_view_models.push("Autolog".into());
        call(&mut lua, "TryDeleteItemFromAutologViewModel", vec![s("Autolog"), s("x")]);
        call(&mut lua, "ReportedProfilePicForUser", vec![s("bob")]);
        call(&mut lua, "OnWallPostOrComment", vec![]);
        assert_eq!(
            st(&host).requests,
            vec![
                Request::PostQueueMessage { function: 0x69a650, id: 5 },
                Request::NetSend { slot: 2, message: crate::hsm::behaviours::online::NetMessage::Disconnect },
                Request::AddedRecoFriendTelemetry,
                Request::TunedFriendTelemetry { value: 3 },
                Request::DeleteViewModelItem { view_model: "Autolog".into(), item: "x".into() },
                Request::ReportedProfilePic { name: "bob".into() },
            ]
        );
    }

    #[test]
    fn friends_and_downloads() {
        let (mut lua, host) = setup();
        assert_eq!(call(&mut lua, "StartFriendRequest", vec![s("")])[0].as_number(), Some(1.0));
        {
            let q = &st(&host).sim.queries;
            assert_eq!((q.friend_action_result[&1], q.friend_action_finished[&1]), (1, true));
        }
        assert!(st(&host).requests.is_empty());
        assert_eq!(call(&mut lua, "StartFriendRequest", vec![s("bob")])[0].as_number(), Some(1.0));
        assert_eq!(call(&mut lua, "StartFriendRequestByOriginId", vec![n(7.0)])[0].as_number(), Some(1.0));
        assert!(matches!(call(&mut lua, "ShowFriendUi", vec![])[0], Value::Bool(true)));
        call(&mut lua, "SpamFriend", vec![s("x")]);
        {
            let q = &st(&host).sim.queries;
            assert_eq!((q.friend_action_result[&1], q.friend_action_finished[&1]), (-1, false));
        }
        assert_eq!(st(&host).requests, vec![Request::ShowFriendUi; 3]);
        // IsSomethingDownloading: the image manager's state or the download list.
        assert!(matches!(call(&mut lua, "IsSomethingDownloading", vec![])[0], Value::Bool(false)));
        st(&host).online.image_state = 2;
        assert!(matches!(call(&mut lua, "IsSomethingDownloading", vec![])[0], Value::Bool(true)));
        st(&host).online.image_state = 3;
        st(&host).menus.downloads.push(Default::default());
        assert!(matches!(call(&mut lua, "IsSomethingDownloading", vec![])[0], Value::Bool(true)));
    }

    #[test]
    fn store_requests() {
        let (mut lua, host) = setup();
        // Not in state 0xd: nothing is taken.
        call(&mut lua, "RequestPurchase", vec![n(5.0)]);
        call(&mut lua, "RequestRedeemCode", vec![]);
        assert_eq!(st(&host).online.store, StoreObject::default());
        st(&host).online.store.state = STORE_READY;
        st(&host).online.store.service_busy = true;
        call(&mut lua, "RequestDownload", vec![n(9.0)]);
        assert_eq!(st(&host).online.store.item, 0);
        st(&host).online.store.service_busy = false;
        call(&mut lua, "RequestPurchase", vec![n(5.9)]);
        assert_eq!(
            {
                let b = st(&host);
                (b.online.store.item, b.online.store.purchase)
            },
            (5, true)
        );
        call(&mut lua, "RequestPurchaseOLP", vec![]);
        assert_eq!(st(&host).online.store.item, OLP_ITEM);
        call(&mut lua, "RequestRedeemCode", vec![s("CODE")]);
        assert!(st(&host).online.store.redeem);
        st(&host).menus.selected_pack = 2201004;
        call(&mut lua, "RequestDownloadOnlinePass", vec![]);
        assert_eq!(
            {
                let b = st(&host);
                (b.online.store.item, b.online.store.download)
            },
            (2201004, true)
        );
        call(&mut lua, "RequestDownload", vec![n(11.0)]);
        assert_eq!(st(&host).online.store.item, 11);
    }

    #[test]
    fn credentials_and_advert() {
        let (mut lua, host) = setup();
        call(&mut lua, "SetEMail", vec![s("a@b.c")]);
        call(&mut lua, "SetEMail", vec![s("x@y.z")]);
        call(&mut lua, "SetNewEMail", vec![s("n@b.c")]);
        call(&mut lua, "SetPassword", vec![s(&"p".repeat(100))]);
        call(&mut lua, "SetNewPassword", vec![s("new")]);
        call(&mut lua, "SetOriginID", vec![s("Origin\u{e9}Name12345678901")]);
        {
            let c = &st(&host).online.credentials;
            assert_eq!(c.email.text(), "x@y.z");
            assert_eq!(c.new_email.text(), "n@b.c");
            assert_eq!(c.password.units.len(), 0x40);
            assert_eq!(c.new_password.text(), "new");
            assert_eq!(c.origin_id.units.len(), 0x10);
            assert!(c.origin_id.text().starts_with("Origin\u{e9}"));
            assert!(!format!("{c:?}").contains("x@y.z"));
        }
        call(&mut lua, "SelectAdvert", vec![s("Other"), s("u"), s("a"), s("b")]);
        call(&mut lua, "SelectAdvert", vec![s("Advert"), s("http://u"), s("a"), s(&"b".repeat(40))]);
        assert_eq!(
            st(&host).requests,
            vec![Request::SelectAdvert { url: "http://u".into(), a: Some("a".into()), b: None }]
        );
    }

    #[test]
    fn sweet_revenge_native() {
        let (mut lua, host) = setup();
        st(&host).players.local = Some(3);
        call(&mut lua, "SetRolloutSweetRevenge", vec![n(7.8)]);
        assert_eq!(st(&host).hook_playlist.sweet_revenge[3], 7);
        assert_eq!(st(&host).requests, vec![Request::ApplyLocalRollout]);
    }
}
