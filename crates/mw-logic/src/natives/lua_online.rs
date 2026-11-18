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

#[derive(Debug, Clone, PartialEq)]
pub struct OnlineLuaFields {
    pub custom_playlists: CustomPlaylists,
    /// The online session's variable tree (`*0x0117e9f4 + 0x470`): objects (namespaces) of variables.
    pub session: Json,
    pub recommendations: Recommendations,
    pub telemetry: Telemetry,
    /// The cached speedwall document (PM+0x20 +0x1918 +8, its root): `None` offline (nothing was downloaded).
    pub cached_speedwall: Option<Json>,
}

impl Default for OnlineLuaFields {
    fn default() -> Self {
        Self {
            custom_playlists: CustomPlaylists::default(),
            session: Json::Object(Vec::new()),
            recommendations: Recommendations::default(),
            telemetry: Telemetry::default(),
            cached_speedwall: None,
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
}
