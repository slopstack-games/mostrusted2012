//! Behaviour types of the online flows (`GAMELOGIC/GAMEMODES.BNDL`): game / playgroup sessions (`HSB_NET_*`,
//! `CREATEPLAYGR`, `LEAVEPLAYGRO`, `JOINPARTYUID`, `DISCONNECTFR`), invites (`SEND_INVITE`, `ISINVITEPEND`,
//! `INVITEFAILED`), result uploads (`UPLOADRESULT`, `UPLDCONCOURS`), the Autolog's session variables and
//! reconnect (`RECONNECT AL`, `DEL_PR_SESS`), the sign-in state setters and two small checks.
//!
//! All of them talk to the network layer (`*0x010edb58`, G+0x2f…). Every one is ported as read: the same states,
//! guards and exit points, the exits a request object's error code picks. What needs a server is a
//! [`Request::NetSend`] (the message the original builds with its id) and the request object's state / error are
//! [`Query`]s whose offline answers are "no session, no connection": the types that wait for the connection
//! (`HSB_NET_CRE`, `HSB_NET_QJO`) never leave their enter update, the ones that only send when a session exists
//! (`LEAVEPLAYGRO`, `HSB_NET_LEA`) exit `SUCCESS` at once. Slot order of every vtable: enter, enter update,
//! running, update, exit update, exit begin, exit; the factory is the `mov edx` of the name's registration
//! (0x777xxx..0x778xxx), the vtable the factory's `mov [eax], vtable`.
//!
//! The request objects (`FUN_004c27e0` init / `4c2840` clear / `4c2870` acknowledge / `4c4b30` pump / `4c4b40`
//! cancel) have state +8 of the owner (0 idle, 1 pending, 2 done) and error +0xc; `slot` 0 is the one at +0x48,
//! 1 the second of `HSB_NET_QJO` (+0x84). The message ids are the first dword of the message `FUN_004c59d0`
//! queues (`DAT_01105060` is the running request id).

use mw_formats::hsm::key;

use super::{Query, ask, ask_bool, param_int};
use crate::hsm::{Behaviour, Cx, Registry};
use crate::natives::Request;

pub fn register(r: &mut Registry) {
    r.register_fn("UPLOADRESULT", |_| UploadResults::default());
    r.register_fn("UPLDCONCOURS", |_| UploadConcours::default());
    r.register_fn("HSB_NET_CRE", |_| NetCreateGame::default());
    r.register_fn("CREATEPLAYGR", |_| CreatePlaygroup::default());
    r.register_fn("LEAVEPLAYGRO", |_| LeavePlaygroup::default());
    r.register_fn("HSB_NET_QJO", |_| QuickJoin::default());
    r.register_fn("HSB_NET_JOI", |_| NetJoinGame::default());
    r.register_fn("HSB_NET_LEA", |_| NetLeaveGame::default());
    r.register_fn("JOINPARTYUID", |_| JoinPlaygroupByUser::default());
    r.register_fn("DISCONNECTFR", |_| DisconnectFromBlaze);
    r.register_fn("RECONNECT AL", |_| ReconnectAutolog);
    r.register_fn("INVITEFAILED", |_| InviteFailed);
    r.register_fn("ISINVITEPEND", |_| IsInvitePending);
    r.register_fn("SEND_INVITE", |_| SendInvite::default());
    r.register_fn("DEL_PR_SESS", |_| DeleteSessionEntry);
    r.register_fn("VALID_CNX_MP", |_| ValidConnection::default());
    r.register_fn("SETFAILLOGIN", |_| SetLoginState { success: false });
    r.register_fn("SETSUCCESSLO", |_| SetLoginState { success: true });
    r.register_fn("BROWSER_IN", |_| BrowserInput);
    register_more(r);
}

/// A message of the network layer (the first dword is the id), queued with `FUN_004c59d0` on a request object.
/// Ids, sizes and the fields the original fills in are named in each variant; ids that live in game state (the
/// join targets of G+0x2f4a0..0x2f4c0, the invite's id) are filled in by the engine that answers
/// [`Request::NetSend`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetMessage {
    /// 0x45 (`UPLDCONCOURS` enter 0x7630d0): the local player's concours value rounded down to thousands
    /// (`FUN_006b9df0` / 1000 * 1000) as a 64-bit value.
    UploadConcours { value: i32 },
    /// `UPLOADRESULT` enter (0x763230): the online event's result record (the race's player list and scores,
    /// built from the event, scoring and player objects), sent when the network is up and the event is online.
    UploadRaceResult,
    /// 0x1d, size 0x434 (`FUN_00748450`, `HSB_NET_CRE` running): create a game; `visibility` 2 `INVITE_ONLY`, 3
    /// `FRIENDS_ONLY`, 4 `PUBLIC` (the parameter `VISIBILITY`, default 2), `flag` the function's argument.
    CreateGame { visibility: u8, flag: bool },
    /// 0x18, size 0x220 (`FUN_00748700`, `CREATEPLAYGR` enter): create a playgroup (the local player's net id,
    /// `FUN_00047a380` = 0 on the PC).
    CreatePlaygroup,
    /// 0x1c, size 0xc (`FUN_0074dbb0`, `LEAVEPLAYGRO` enter).
    LeavePlaygroup,
    /// 0x21, size 0x10 (`FUN_0074dad0`, `HSB_NET_LEA` enter): leave the game; `bring_playgroup` is the parameter
    /// `BringPlaygrp` (= "true").
    LeaveGame { bring_playgroup: bool },
    /// 0x39 (`FUN_007690b0`, `HSB_NET_JOI`): join the game of the pending invite (the invite object's id).
    JoinInvite,
    /// 0x38 (`FUN_007690f0`): join the friend G+0x2f4b0 / +0x2f4b4.
    JoinFriend,
    /// 0x20 (`FUN_00769130`): join the game id G+0x2f4c0.
    JoinGame,
    /// 0x1a (`FUN_007691b0`, `JOINPARTYUID`): join the playgroup of the invite G+0x2f4a0 / +0x2f4a4.
    JoinPartyByInvite,
    /// 0x19 (`FUN_007691f0`): join the playgroup of the user G+0x2f4b8 / +0x2f4bc.
    JoinPartyByUser,
    /// 0x1e, size 0x224 (`FUN_007580e0`, `HSB_NET_QJO`): the quick-join search (the local player's net id, the
    /// rollout's ids, `FUN_00640550` on G+0x2ab00; its completion callback is 0x7580d0).
    QuickJoinSearch,
    /// 0x34, size 0x218 (`FUN_007693d0`, `HSB_NET_QJO` exit update): the "GoToMatchmaking" info message.
    GoToMatchmaking,
    /// 0x37, size 0xc (`DISCONNECTFR` enter 0x7488a0): disconnect from Blaze.
    Disconnect,
}

/// A request object's state ([`Query::NetRequestState`] answers: 0 idle, 1 pending, 2 done).
const DONE: u64 = 2;

fn net_reset(cx: &mut Cx, slot: u8) {
    cx.host.request(Request::NetReset { slot });
}

fn net_ack(cx: &mut Cx, slot: u8) {
    cx.host.request(Request::NetAck { slot });
}

fn net_pump_state(cx: &mut Cx, slot: u8) -> u64 {
    // `FUN_004c4b30` pumps (the engine's job), the state is read after.
    cx.host.request(Request::NetPump { slot });
    ask(cx, Query::NetRequestState { slot })
}

fn net_error(cx: &mut Cx, slot: u8) -> u32 {
    ask(cx, Query::NetRequestError { slot }) as u32
}

fn exit_to(cx: &mut Cx, name: &str) -> bool {
    cx.set_exit(key(name));
    true
}

/// "Upload Results" (`UPLOADRESULT`, factory 0x77ff80, vtable 0xfdc1b8, 0x230 B): the request object is at +0x1f0
/// (state +0x1f8), +0x228 a flag. Enter (0x763230) resets the object and the flag; without a Blaze connection
/// (`FUN_004c2040`, [`Query::NetBlazeUp`]) the object is acknowledged and nothing is sent; for an online event
/// ([`Query::OnlineEvent`], `FUN_006dfed0`) the result record is built and sent
/// ([`NetMessage::UploadRaceResult`]). The update (0x7415c0) waits one tick (flag), then exits through `EXIT`.
/// The exit update (0x741600) pumps the object: idle finishes; done (state 2) first books the local player's XP
/// as the hook `UploadLocalPlayerXP` does (`FUN_00674520`, [`Request::UploadResultXp`]) and finishes; pending
/// waits.
#[derive(Default)]
pub struct UploadResults {
    waited: bool,
}

impl Behaviour for UploadResults {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.waited = false;
        if !ask_bool(cx, Query::NetBlazeUp) {
            net_ack(cx, 0);
            return;
        }
        if ask_bool(cx, Query::OnlineEvent) {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::UploadRaceResult });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !self.waited {
            self.waited = true;
            return false;
        }
        exit_to(cx, "EXIT")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        match net_pump_state(cx, 0) {
            0 => true,
            DONE => {
                cx.host.request(Request::UploadResultXp);
                true
            }
            _ => false,
        }
    }
}

/// "HSM.UploadConcoursRequest" (`UPLDCONCOURS`, factory 0x780060, vtable 0xfdc210, 0xc8 B): request object at +0x88
/// (state +0x90, error +0x94), +0xc0 the step, +0xc4 the value. Enter (0x7630d0) resets the object, takes the
/// local player's concours value (`FUN_006b9df0`, [`Query::ConcoursValue`]) rounded down to thousands and sends it
/// ([`NetMessage::UploadConcours`]), step 0. The update (0x758930): step 0 (0x741490) pumps the object; done: an
/// error exits through `EXIT` at once, else the player's pending value (+0x3fc) is cleared
/// ([`Request::ClearConcoursPending`]) and step 1; step 1 (0x753440) waits until the player's synced value
/// ([`Query::ConcoursSynced`], +0x408 equals the sent one) and then refreshes the progression objects
/// ([`Request::ApplyConcoursResult`]: `FUN_006fdb70` / `6fdd80` on G+0x2f418, `6e4d40` on G+0x2f420, `679250` on
/// G+0x2f3e4, and for an online event `683370` on G+0x2f404) and exits through `EXIT`.
#[derive(Default)]
pub struct UploadConcours {
    step: u32,
    value: i32,
}

impl Behaviour for UploadConcours {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.value = (ask(cx, Query::ConcoursValue) as i32) / 1000 * 1000;
        cx.host.request(Request::NetSend { slot: 0, message: NetMessage::UploadConcours { value: self.value } });
        self.step = 0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.step {
            0 => {
                if net_pump_state(cx, 0) == DONE {
                    if net_error(cx, 0) != 0 {
                        return exit_to(cx, "EXIT");
                    }
                    cx.host.request(Request::ClearConcoursPending);
                    self.step = 1;
                }
                false
            }
            1 => {
                if ask_bool(cx, Query::ConcoursSynced { value: self.value }) {
                    cx.host.request(Request::ApplyConcoursResult);
                    return exit_to(cx, "EXIT");
                }
                false
            }
            _ => false,
        }
    }
}

/// The common update of the create machines (`HSB_NET_CRE` 0x73bb80, `CREATEPLAYGR` 0x73bc50): the request
/// finished (state 2): an error exits `FAILED` / `FAIL` and clears the success flag (+0x80), no error `SUCCESS`
/// and sets it; the object is acknowledged. Returns whether it finished.
fn create_update(cx: &mut Cx, success: &mut bool, fail: &str, ok: &str) -> bool {
    if net_pump_state(cx, 0) != DONE {
        return false;
    }
    if net_error(cx, 0) != 0 {
        cx.set_exit(key(fail));
        *success = false;
    } else {
        cx.set_exit(key(ok));
        *success = true;
    }
    net_ack(cx, 0);
    true
}

/// The exit machinery of the create machines: the exit update waits, when the request succeeded, for the session
/// (`FUN_004c1ec0`, [`Query::OnlineGame`] / `FUN_004c1ea0` [`Query::NetInGame`]); the exit clears the object and
/// restores the UI's message layer (`FUN_007b3410` on UI +0x8c90, [`Request::RestoreUiMessages`]).
fn create_exit(cx: &mut Cx) {
    net_reset(cx, 0);
    cx.host.request(Request::RestoreUiMessages);
}

/// "Create Public Game" etc. (`HSB_NET_CRE`, factory 0x77b2e0, vtable 0xfd9410, 0x88 B): `VISIBILITY` (`INVITE_ONLY`
/// 2, `FRIENDS_ONLY` 3, `PUBLIC` 4; default 2; `MESSAGE` is not read). The enter update (0x73bb60) waits for the
/// network layer (`FUN_004c3750`, [`Query::NetReady`]: offline it never finishes). Running (0x753dc0) creates the
/// game (`FUN_00748450`(1): the wished game G+0x2f4c8 is set, the request object reset, [`Request::SetGameWish`],
/// [`NetMessage::CreateGame`], +0x80 = 0, the create-wished flag G+0x2f708 cleared:
/// [`Request::ClearCreateGameDesired`]). The update (0x73bb80) exits `SUCCESS` / `FAILED` ([`create_update`]); the
/// exit update (0x73bbf0) after a success waits for the session ([`Query::OnlineGame`]).
#[derive(Default)]
pub struct NetCreateGame {
    success: bool,
}

impl Behaviour for NetCreateGame {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        ask_bool(cx, Query::NetReady)
    }

    fn on_running(&mut self, cx: &mut Cx) {
        let visibility = match cx.param_text("VISIBILITY").as_str() {
            "INVITE_ONLY" => 2,
            "FRIENDS_ONLY" => 3,
            "PUBLIC" => 4,
            _ => 2,
        };
        net_reset(cx, 0);
        cx.host.request(Request::SetGameWish { local: true, flag: true });
        cx.host.request(Request::NetSend { slot: 0, message: NetMessage::CreateGame { visibility, flag: true } });
        cx.host.request(Request::ClearCreateGameDesired);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        create_update(cx, &mut self.success, "FAILED", "SUCCESS")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        !self.success || ask_bool(cx, Query::OnlineGame)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        create_exit(cx);
    }
}

/// "CreatePlaygroup" (`CREATEPLAYGR`, factory 0x77b330, vtable 0xfd9434, 0x88 B): enter (0x748700) resets the
/// object and sends the create ([`NetMessage::CreatePlaygroup`]), +0x80 = 0. The update (0x73bc50) exits `SUCCESS`
/// / `FAIL` as [`create_update`]; the exit update (0x73bcc0) after a success waits for `FUN_004c1ea0`
/// ([`Query::NetInGame`]); the exit as [`create_exit`].
#[derive(Default)]
pub struct CreatePlaygroup {
    success: bool,
}

impl Behaviour for CreatePlaygroup {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.success = false;
        cx.host.request(Request::NetSend { slot: 0, message: NetMessage::CreatePlaygroup });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        create_update(cx, &mut self.success, "FAIL", "SUCCESS")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        !self.success || ask_bool(cx, Query::NetInGame)
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        create_exit(cx);
    }
}

/// "LeavePlaygroup" (`LEAVEPLAYGRO`, factory 0x77b7c0, vtable 0xfd962c, 0x88 B): enter (0x74dbb0) resets the object,
/// +0x80 = 0, and when in a game session (`FUN_004c1ea0`, [`Query::NetInGame`]) sends the leave
/// ([`NetMessage::LeavePlaygroup`], +0x80 = 1). The update (0x73d8e0): not sent: `SUCCESS`; sent: pumps; an error
/// exits `FAILED` while the session still stands, else `SUCCESS`; no error: `SUCCESS` once done. The exit
/// (0x73d980) clears the object.
#[derive(Default)]
pub struct LeavePlaygroup {
    sent: bool,
}

impl Behaviour for LeavePlaygroup {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.sent = false;
        if ask_bool(cx, Query::NetInGame) {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::LeavePlaygroup });
            self.sent = true;
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !self.sent {
            return exit_to(cx, "SUCCESS");
        }
        cx.host.request(Request::NetPump { slot: 0 });
        if net_error(cx, 0) != 0 {
            let name = if ask_bool(cx, Query::NetInGame) { "FAILED" } else { "SUCCESS" };
            return exit_to(cx, name);
        }
        if ask(cx, Query::NetRequestState { slot: 0 }) == DONE {
            return exit_to(cx, "SUCCESS");
        }
        false
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
    }
}

/// "Internal Quick Join" (`HSB_NET_QJO`, factory 0x77b810, vtable 0xfd9650, 0xc0 B; data definition 5 holds the
/// numbers the screen shows): two request objects (+0x48 slot 0, +0x84 slot 1), +0x80 the success flag, +0x81 "the
/// info message was sent", +0x82 "the search was cancelled". The enter update (0x73eb90) waits for the network
/// layer ([`Query::NetReady`]) and clears +0x81 / +0x82. Running (0x75c500): both objects reset, the wished game
/// set ([`Request::SetGameWish`]), the search sent ([`NetMessage::QuickJoinSearch`], `FUN_007580e0`) unless the
/// session already stands ([`Query::NetInGame`] and [`Query::NetLobbyA`], `FUN_004c1eb0`), the wish set again and
/// the quick-join wish G+0x2f490 cleared ([`Request::ClearQuickJoinDesired`]). The update (0x75c5d0): lobby
/// ([`Query::NetLobbyA`]) with an idle search object sends the search again; the info object is pumped and
/// acknowledged when done; outside the lobby the three session numbers (`NumPlayers` / `MinAccepted` /
/// `MaxAccepted`, [`Query::QuickJoinInfo`]) are published into the data definition
/// ([`Request::PublishQuickJoinInfo`]) when all are there; in the playgroup ([`Query::OnlineGame`]) exit `SUCCESS`;
/// else with the matchmaking player state 2 ([`Query::MatchState`], `+0x298`): not yet cancelled and idle search
/// exits `CANCELLED`, not yet cancelled otherwise cancels the search (+0x82); a finished search exits by its error
/// (0 `SUCCESS` and +0x80 = 1, 2 `DISCONNECTED`, 4 `CANCELLED`, else `FAILED`), acknowledging the object. Exit
/// begin (0x73ebc0) cancels a pending search once. The exit update (0x74f790): acknowledges finished objects;
/// in the lobby with +0x81 clear sends the info message ([`NetMessage::GoToMatchmaking`]) once and waits; waits
/// while an object is busy; after a success it waits for the playgroup and for the session flag
/// ([`Query::QuickJoinSessionFlag`], `FUN_0063b0a0`); the exit (0x73ebf0) clears the first object.
#[derive(Default)]
pub struct QuickJoin {
    success: bool,
    info_sent: bool,
    cancelled: bool,
}

impl QuickJoin {
    fn finish(&mut self, cx: &mut Cx, name: &str, success: bool) -> bool {
        self.success = success;
        net_ack(cx, 0);
        exit_to(cx, name)
    }
}

impl Behaviour for QuickJoin {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::NetReady) {
            return false;
        }
        self.info_sent = false;
        self.cancelled = false;
        true
    }

    fn on_running(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        net_reset(cx, 1);
        cx.host.request(Request::SetGameWish { local: true, flag: true });
        self.success = false;
        if !ask_bool(cx, Query::NetInGame) || !ask_bool(cx, Query::NetLobbyA) {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::QuickJoinSearch });
            self.cancelled = false;
        }
        cx.host.request(Request::SetGameWish { local: true, flag: true });
        cx.host.request(Request::ClearQuickJoinDesired);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        let lobby = ask_bool(cx, Query::NetLobbyA);
        if lobby && ask(cx, Query::NetRequestState { slot: 0 }) == 0 {
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::QuickJoinSearch });
            self.cancelled = false;
        }
        if ask(cx, Query::NetRequestState { slot: 1 }) != 0 && net_pump_state(cx, 1) == DONE {
            net_ack(cx, 1);
        }
        if !ask_bool(cx, Query::NetLobbyA) {
            let info: Vec<u64> = (0..3).map(|which| ask(cx, Query::QuickJoinInfo { which })).collect();
            if info.iter().all(|v| *v != u64::MAX) {
                cx.host.request(Request::PublishQuickJoinInfo {
                    players: info[0] as i32,
                    min: info[1] as i32,
                    max: info[2] as i32,
                });
            }
        }
        if ask_bool(cx, Query::OnlineGame) {
            return exit_to(cx, "SUCCESS");
        }
        if ask(cx, Query::MatchState) == 2 {
            self.success = false;
            if !self.cancelled {
                if ask(cx, Query::NetRequestState { slot: 0 }) == 0 {
                    cx.set_exit(key("CANCELLED"));
                    return true;
                }
                cx.host.request(Request::NetCancel { slot: 0 });
                self.cancelled = true;
            }
        }
        if ask(cx, Query::NetRequestState { slot: 0 }) == 0 {
            return false;
        }
        if net_pump_state(cx, 0) != DONE {
            return false;
        }
        match net_error(cx, 0) {
            0 => {
                self.success = true;
                net_ack(cx, 0);
                exit_to(cx, "SUCCESS")
            }
            2 => self.finish(cx, "DISCONNECTED", false),
            4 => self.finish(cx, "CANCELLED", false),
            _ => self.finish(cx, "FAILED", false),
        }
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        for slot in 0..2 {
            if ask(cx, Query::NetRequestState { slot }) != 0 && net_pump_state(cx, slot) == DONE {
                net_ack(cx, slot);
            }
        }
        if ask(cx, Query::NetRequestState { slot: 1 }) == 0 && ask_bool(cx, Query::NetLobbyA) && !self.info_sent {
            cx.host.request(Request::NetSend { slot: 1, message: NetMessage::GoToMatchmaking });
            self.info_sent = true;
            return false;
        }
        if ask(cx, Query::NetRequestState { slot: 0 }) != 0 || ask(cx, Query::NetRequestState { slot: 1 }) != 0 {
            return false;
        }
        if self.success {
            if !ask_bool(cx, Query::OnlineGame) {
                return false;
            }
            if ask_bool(cx, Query::NetInGame) && ask_bool(cx, Query::QuickJoinSessionFlag) {
                return false;
            }
        }
        true
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        let state = ask(cx, Query::NetRequestState { slot: 0 });
        if state != 0 && state != DONE && !self.cancelled {
            cx.host.request(Request::NetCancel { slot: 0 });
            self.cancelled = true;
        }
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
    }
}

/// "Join Game" (`HSB_NET_JOI`, factory 0x77b860, vtable 0xfd9674, 0x88 B): request object +0x48, +0x80 the step,
/// +0x84 "joined". Enter (0x74d530) resets the object and sets the wished game ([`Request::SetGameWish`]). Step 0
/// (0x74d590): without a Blaze connection ([`Query::NetBlazeUp`]) exit `FAILED`; once the network layer is ready
/// ([`Query::NetReady`]) the join is sent for what is wished: an accepted invite ([`Query::InviteAccepted`]:
/// [`NetMessage::JoinInvite`]), else the friend wish ([`Query::JoinWish`] 1: [`NetMessage::JoinFriend`]) or the
/// game wish (2: [`NetMessage::JoinGame`]); the wishes are cleared ([`Request::ClearJoinWish`]) and step 1.
/// Step 1 (0x73d470): finished with no error: `SUCCESS`, +0x84 = 1; the error codes 45 `NOT_EXIST`, 46
/// `GAME_FULL`, 48 `INV_VER`, 49 `NO_PRIV`, 51 `UNENTITLED`, the others `FAILED`, +0x84 = 0; the object is
/// acknowledged. The exit update (0x73d5d0) waits while the invite object is ready ([`Query::InviteAccepted`]) and,
/// after a join, for the session ([`Query::OnlineGame`]). Exit begin (0x73d5b0): the quick-join wish G+0x2f490
/// cleared and the invite dropped ([`Request::ClearInvite`], `FUN_006bad40`); the exit clears the object and the UI
/// message layer.
#[derive(Default)]
pub struct NetJoinGame {
    step: u32,
    joined: bool,
}

impl Behaviour for NetJoinGame {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.step = 0;
        self.joined = false;
        cx.host.request(Request::SetGameWish { local: true, flag: true });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.step == 0 {
            if !ask_bool(cx, Query::NetBlazeUp) {
                return exit_to(cx, "FAILED");
            }
            if !ask_bool(cx, Query::NetReady) {
                return false;
            }
            let message = if ask_bool(cx, Query::InviteAccepted) {
                Some(NetMessage::JoinInvite)
            } else {
                match ask(cx, Query::JoinWish) {
                    1 => Some(NetMessage::JoinFriend),
                    2 => Some(NetMessage::JoinGame),
                    _ => None,
                }
            };
            if let Some(message) = message {
                cx.host.request(Request::NetSend { slot: 0, message });
            }
            cx.host.request(Request::ClearJoinWish);
            self.step = 1;
            return false;
        }
        if net_pump_state(cx, 0) != DONE {
            return false;
        }
        let code = net_error(cx, 0);
        if code == 0 {
            cx.set_exit(key("SUCCESS"));
            self.joined = true;
        } else {
            self.joined = false;
            let name = match code {
                45 => "NOT_EXIST",
                46 => "GAME_FULL",
                48 => "INV_VER",
                49 => "NO_PRIV",
                51 => "UNENTITLED",
                _ => "FAILED",
            };
            cx.set_exit(key(name));
        }
        net_ack(cx, 0);
        true
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::InviteAccepted) {
            return false;
        }
        !self.joined || ask_bool(cx, Query::OnlineGame)
    }

    fn on_exit_begin(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ClearQuickJoinDesired);
        cx.host.request(Request::ClearInvite);
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        create_exit(cx);
    }
}

/// "Leave Game" (`HSB_NET_LEA`, factory 0x77b8b0, vtable 0xfd9698, 0x88 B): `BringPlaygrp` ("true" = bring the
/// playgroup along). Enter (0x74dad0) resets the object, +0x80 = 0, and sends the leave
/// ([`NetMessage::LeaveGame`], +0x80 = 1) when a game session ([`Query::OnlineGame`]) or a lobby
/// ([`Query::NetLobbyB`], `FUN_004c23a0`) exists. The update (0x73d7f0): not sent: `SUCCESS`; an error: no Blaze
/// ([`Query::NetBlazeUp`]) `DISCONNECTED`, else `FAILED` while the playgroup stands, `SUCCESS` without; done:
/// `SUCCESS`. The exit update (0x73d8a0): idle done; done acknowledges and finishes; pending pumps; the exit
/// clears the object.
#[derive(Default)]
pub struct NetLeaveGame {
    sent: bool,
}

impl Behaviour for NetLeaveGame {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.sent = false;
        if ask_bool(cx, Query::OnlineGame) || ask_bool(cx, Query::NetLobbyB) {
            let bring_playgroup = cx.param_text("BRINGPLAYGRP").eq_ignore_ascii_case("true");
            cx.host.request(Request::NetSend { slot: 0, message: NetMessage::LeaveGame { bring_playgroup } });
            self.sent = true;
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if !self.sent {
            return exit_to(cx, "SUCCESS");
        }
        cx.host.request(Request::NetPump { slot: 0 });
        if net_error(cx, 0) != 0 {
            if !ask_bool(cx, Query::NetBlazeUp) {
                return exit_to(cx, "DISCONNECTED");
            }
            let name = if ask_bool(cx, Query::OnlineGame) { "FAILED" } else { "SUCCESS" };
            return exit_to(cx, name);
        }
        if ask(cx, Query::NetRequestState { slot: 0 }) == DONE {
            return exit_to(cx, "SUCCESS");
        }
        false
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        match ask(cx, Query::NetRequestState { slot: 0 }) {
            0 => true,
            DONE => {
                net_ack(cx, 0);
                true
            }
            _ => {
                cx.host.request(Request::NetPump { slot: 0 });
                false
            }
        }
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
    }
}

/// "Join Playgroup By User Id" (`JOINPARTYUID`, factory 0x77b770, vtable 0xfd9608, 0x88 B): request object +0x48,
/// +0x80 step, +0x84 "joined". Enter (0x73d6a0) resets both. Step 0 (0x74d8d0): without a Blaze connection
/// ([`Query::NetBlazeUp`]) `FAILED`; once the network layer is ready ([`Query::NetReady`]) sends the join of the
/// invite's playgroup (G+0x2f4a8 set, [`Query::PartyInvite`]: [`NetMessage::JoinPartyByInvite`]) or of the user's
/// ([`NetMessage::JoinPartyByUser`]), clears the wishes ([`Request::ClearPartyWish`]) and goes to step 1. Step 1
/// (0x73d6c0): done: an error exits `FAILED` (+0x84 = 0), else `PARTY` (+0x84 = 1); then the UI is refreshed
/// (`FUN_00ae1b20`(0): [`Request::RefreshPartyUi`]) and the object acknowledged. The exit update (0x73d730) is
/// done; the exit (0x73d740) clears the object.
#[derive(Default)]
pub struct JoinPlaygroupByUser {
    step: u32,
}

impl Behaviour for JoinPlaygroupByUser {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        self.step = 0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if self.step == 0 {
            if !ask_bool(cx, Query::NetBlazeUp) {
                return exit_to(cx, "FAILED");
            }
            if !ask_bool(cx, Query::NetReady) {
                return false;
            }
            let message = if ask_bool(cx, Query::PartyInvite) {
                NetMessage::JoinPartyByInvite
            } else {
                NetMessage::JoinPartyByUser
            };
            cx.host.request(Request::NetSend { slot: 0, message });
            cx.host.request(Request::ClearPartyWish);
            self.step = 1;
            return false;
        }
        if net_pump_state(cx, 0) != DONE {
            return false;
        }
        let failed = net_error(cx, 0) != 0;
        cx.set_exit(key(if failed { "FAILED" } else { "PARTY" }));
        cx.host.request(Request::RefreshPartyUi);
        net_ack(cx, 0);
        true
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
    }
}

/// "Disconnect From Blaze" (`DISCONNECTFR`, factory 0x77a1a0, vtable 0xfd902c, 0x80 B): enter (0x7488a0) resets
/// the object and sends the disconnect ([`NetMessage::Disconnect`]); the update (0x73be80) exits through `EXIT`
/// at once; the exit update (0x73bea0) waits for the request (pumped, state 2), then acknowledges and clears it.
pub struct DisconnectFromBlaze;

impl Behaviour for DisconnectFromBlaze {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        cx.host.request(Request::NetSend { slot: 0, message: NetMessage::Disconnect });
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        if net_pump_state(cx, 0) != DONE {
            return false;
        }
        net_ack(cx, 0);
        net_reset(cx, 0);
        true
    }
}

/// "Reconnect To Autolog Server" (`RECONNECT AL`, factory 0x77ecc0, vtable 0xfdaeb4, 0x48 B): enter (0x76e9d0) calls
/// `FUN_0083b660` on the Autolog object G+0x2ed74 ([`Request::AutologReconnect`]); the update (0x76e9f0) exits
/// through `EXIT`.
pub struct ReconnectAutolog;

impl Behaviour for ReconnectAutolog {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::AutologReconnect);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }
}

/// "Invite Failed" (`INVITEFAILED`, factory 0x77d0a0, vtable 0xfda1cc, 0x48 B): enter (0x73d2c0 =
/// `FUN_006bad40`) drops the invite ([`Request::ClearInvite`]: vtable +0xc of the invite object G+0xc9a0,
/// `FUN_00419240` on +0xcab8); the update (0x73d2d0) waits while an invite is pending
/// ([`Query::InvitePending`]: vtable +0 of the object), then exits through `EXIT`.
pub struct InviteFailed;

impl Behaviour for InviteFailed {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ClearInvite);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::InvitePending) {
            return false;
        }
        exit_to(cx, "EXIT")
    }
}

/// "Is Invite Pending" (`ISINVITEPEND`, factory 0x77d490, vtable 0xfda3d8, 0x48 B): the update (0x73d420) exits
/// through `YES` while an invite is pending ([`Query::InvitePending`]), else `NO`.
pub struct IsInvitePending;

impl Behaviour for IsInvitePending {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let pending = ask_bool(cx, Query::InvitePending);
        cx.set_exit(if pending { super::game::YES } else { super::game::NO });
        true
    }
}

/// "Send Invite" (`SEND_INVITE`, factory 0x77c000, vtable 0xfd9a04, 0x58 B): +0x48 step, +0x50 the friend id.
/// Enter (0x73f2d0) takes the friend invite id G+0x2f498 ([`Query::FriendInviteId`], set by the hook
/// `SetFriendInviteIdEasyDrive`; cleared: [`Request::ClearFriendInviteId`]) and starts at step 0, or at step 2
/// without one. The update (0x73f320): step 0 sends the invite (`FUN_00418b60` on the invite object,
/// [`Request::SendInvite`]) and falls into step 1, which sets step 1 and moves to 2 once the invite object says it
/// was sent ([`Query::InviteSent`], vtable +0x10); step 2 exits through `INVITE_SENT`. The exit update (0x73f390)
/// is done.
#[derive(Default)]
pub struct SendInvite {
    step: u32,
    id: u64,
}

impl Behaviour for SendInvite {
    fn on_enter(&mut self, cx: &mut Cx) {
        self.id = ask(cx, Query::FriendInviteId);
        if self.id != 0 {
            cx.host.request(Request::ClearFriendInviteId);
            self.step = 0;
        } else {
            self.step = 2;
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        match self.step {
            2 => exit_to(cx, "INVITE_SENT"),
            0 | 1 => {
                if self.step == 0 {
                    cx.host.request(Request::SendInvite { id: self.id });
                }
                self.step = 1;
                if ask_bool(cx, Query::InviteSent) {
                    self.step = 2;
                }
                false
            }
            _ => false,
        }
    }
}

/// "Delete From PageRunner Session" (`DEL_PR_SESS`, factory 0x77eb80, vtable 0xfdae24, 0x48 B): enter (0x76f680)
/// removes the entry named by `NAME` (when not empty) from the session store of G+0x2ed74 (`FUN_0083b630`,
/// [`Request::DeleteSessionEntry`]); the update (0x76e880) exits through `EXIT`.
pub struct DeleteSessionEntry;

impl Behaviour for DeleteSessionEntry {
    fn on_enter(&mut self, cx: &mut Cx) {
        let name = cx.param_text("NAME");
        if !name.is_empty() {
            cx.host.request(Request::DeleteSessionEntry { name });
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }
}

/// "HSM.CheckForValidConnectionForMP" (`VALID_CNX_MP`, factory 0x77f0d0, vtable 0xfdb088, 0x50 B): enter (0x73ba50)
/// sets the byte +0x48 = 1 and the update (0x73ba60) exits through `VALID` while it is set (it always is on the PC;
/// else `NOT_VALID`).
#[derive(Default)]
pub struct ValidConnection {
    valid: bool,
}

impl Behaviour for ValidConnection {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.valid = true;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, if self.valid { "VALID" } else { "NOT_VALID" })
    }
}

/// "Set Login State Fail / Success Online" (`SETFAILLOGIN` 0x77d3a0 / vtable 0xfda360, `SETSUCCESSLO` 0x77d350 /
/// 0xfda33c, 0x48 B): the update (0x770f90 / 0x770fe0) stores `atoi(STATE)` in the sign-in state object G+0x2f3e0:
/// the fail one at +0x10cc, the success one at +0x10c8 (and +0x10cc = 0 when it is 0), then exits through `EXIT`
/// ([`Request::SetLoginState`]).
pub struct SetLoginState {
    success: bool,
}

impl Behaviour for SetLoginState {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let state = param_int(cx, "STATE") as i32;
        cx.host.request(Request::SetLoginState { success: self.success, state });
        exit_to(cx, "EXIT")
    }
}

/// The browser commands `BROWSER_IN` recognises, the packed answer of [`Query::BrowserCommand`] (the pending
/// command string G+0x2f454 +0x4628 with the flag +0x4627 set, compared with the table 0x10c6464..0x10c6474):
/// 1 `PlayIt?`, 2 `TakeProfilePic?`, 3 `Join?`, 4 `Invite?`, 5 `Perspective?`.
pub const BROWSER_PLAY_IT: u64 = 1;
pub const BROWSER_TAKE_PROFILE_PIC: u64 = 2;
pub const BROWSER_JOIN: u64 = 3;
pub const BROWSER_INVITE: u64 = 4;
pub const BROWSER_PERSPECTIVE: u64 = 5;

/// "Handle Browser Input" (`BROWSER_IN`, factory 0x77eb30, vtable 0xfdae00, 0x68 B; state +0x48..0x60 cleared on
/// enter, 0x76e620). The update (0x770060) holds while the local player's net object (`FUN_00047aa20`) is in state
/// 1 ([`Query::BrowserBlocked`]) or no command is pending ([`Query::BrowserCommand`] 0). `PlayIt?` parses the
/// arguments (`FUN_0076f460`, [`Request::BrowserParse`]), sets the wished game (`FUN_0063e380`,
/// [`Request::SetGameWish`] with the session flag) and the pending flag off ([`Request::BrowserCommandDone`])
/// and exits `CLOSE`; `TakeProfilePic?` exits `CHANGE_PIC`; `Join?` parses and sets the join wish (a game id: 2 and
/// G+0x2f4c0, a friend id: 1 and G+0x2f4b0, copied to +0x2f4b8; engine: [`Request::BrowserParse`]) and exits
/// `JOIN_GAME`; `Invite?` parses, sets the friend invite id G+0x2f498 and exits `INVITE`; `Perspective?` only
/// parses and holds. Any other command holds.
pub struct BrowserInput;

impl Behaviour for BrowserInput {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::BrowserBlocked) {
            return false;
        }
        let command = ask(cx, Query::BrowserCommand);
        match command {
            BROWSER_PLAY_IT => {
                cx.host.request(Request::BrowserParse { command });
                let in_game = ask_bool(cx, Query::OnlineGame);
                cx.host.request(Request::SetGameWish { local: false, flag: in_game });
                cx.host.request(Request::BrowserCommandDone);
                exit_to(cx, "CLOSE")
            }
            BROWSER_TAKE_PROFILE_PIC => exit_to(cx, "CHANGE_PIC"),
            BROWSER_JOIN => {
                cx.host.request(Request::BrowserParse { command });
                exit_to(cx, "JOIN_GAME")
            }
            BROWSER_INVITE => {
                cx.host.request(Request::BrowserParse { command });
                exit_to(cx, "INVITE")
            }
            BROWSER_PERSPECTIVE => {
                cx.host.request(Request::BrowserParse { command });
                false
            }
            _ => false,
        }
    }
}

// ---- Further types of the same flows (the second batch of this module) ----

pub fn register_more(r: &mut Registry) {
    r.register_fn("HAS_INV_ACCT", |_| HasInviteAccepted);
    r.register_fn("ENUM_ENTMNTS", |_| EnumerateEntitlements);
    r.register_fn("SYSERROR", |_| SystemError::default());
    r.register_fn("HOSTSETGAMEM", |_| HostSetGameMode);
    r.register_fn("STOPMOVIE", |_| StopMovie);
    r.register_fn("ONLINESCORIN", |_| OnlineScoreInit);
    r.register_fn("SYNCGAMESTAT", |_| SyncGameState);
    r.register_fn("UPLDCHALSTE", |_| UploadChallengeState);
    r.register_fn("IsPRCtxtOpen", |_| IsContextOpen);
    r.register_fn("ONLNSCRACT", |_| OnlineScoreActions);
    r.register_fn("REBOOT", |_| Reboot);
    r.register_fn("SYNCTIME", |_| SyncTime);
}

/// "Has Invite Accepted" (`HAS_INV_ACCT`, factory 0x77bf60, vtable 0xfd99b0, 0x48 B): the update (0x73c850)
/// exits through `YES` when the invite object G+0xc9a0 says the invite was accepted (vtable +4,
/// [`Query::InviteAccepted`]), else `NO`.
pub struct HasInviteAccepted;

impl Behaviour for HasInviteAccepted {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let accepted = ask_bool(cx, Query::InviteAccepted);
        cx.set_exit(if accepted { super::game::YES } else { super::game::NO });
        true
    }
}

/// "Enumerate Entitlements" (`ENUM_ENTMNTS`, factory 0x77d760, vtable 0xfda538, 0x48 B): the enter update (0x73c600)
/// sets the entitlement list's request byte G+0x29d60 ([`Request::EnumerateEntitlements`]); the update (0x73c620)
/// exits through `EXIT`; the exit update (0x73c640) waits until the byte is cleared again
/// ([`Query::EntitlementEnumPending`], the entitlement manager's job).
pub struct EnumerateEntitlements;

impl Behaviour for EnumerateEntitlements {
    fn enter_update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::EnumerateEntitlements);
        true
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        !ask_bool(cx, Query::EntitlementEnumPending)
    }
}

/// "SysError" (`SYSERROR`, factory 0x77ce60, vtable 0xfda09c, 0x50 B): enter (0x740cc0) clears the step +0x48; the
/// update (0x740cd0) exits through `EXIT`; the exit update (0x740cf0) runs the step to 4 (every step 0..4 falls
/// through to 4) and is done; a step beyond 4 would never finish.
#[derive(Default)]
pub struct SystemError {
    step: u32,
}

impl Behaviour for SystemError {
    fn on_enter(&mut self, _cx: &mut Cx) {
        self.step = 0;
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }

    fn exit_update(&mut self, _cx: &mut Cx) -> bool {
        if self.step > 4 {
            return false;
        }
        self.step = 4;
        true
    }
}

/// "Host Set Game Mode" (`HOSTSETGAMEM`, factory 0x77c0a0, vtable 0xfd9a4c, 0x48 B): enter (0x73c8f0) lets the
/// session host (`FUN_006dfee0`: not in a game session, or the host, [`Query::SessionHost`]) set the game mode
/// ([`Request::HostSetGameMode`], `FUN_006f4680`); the update (0x73c930) exits through `EXIT` by
/// `FUN_00774dc0` (only when the parent handles it).
pub struct HostSetGameMode;

impl Behaviour for HostSetGameMode {
    fn on_enter(&mut self, cx: &mut Cx) {
        if ask_bool(cx, Query::SessionHost) {
            cx.host.request(Request::HostSetGameMode);
        }
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if cx.parent_handles(crate::hsm::EXIT) {
            cx.set_exit(crate::hsm::EXIT);
            return true;
        }
        false
    }
}

/// "Stop Movie" (`STOPMOVIE`, factory 0x77c570, vtable 0xfd9c94, 0x4c B): enter (0x740f60) stops the movie player
/// (`FUN_0079f1c0` on the UI +0x2deb0, [`Request::StopMovie`]); the update (0x779a50) exits through `EXIT`.
pub struct StopMovie;

impl Behaviour for StopMovie {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::StopMovie);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        exit_to(cx, "EXIT")
    }
}

/// "Online Scoring Init" (`ONLINESCORIN`, factory 0x77de10, vtable 0xfda860, 0x48 B): every slot is empty (the
/// update 0x73e400 returns 0, never exits; the exit update 0x73e420 is done): a placeholder the flow holds in.
pub struct OnlineScoreInit;

impl Behaviour for OnlineScoreInit {}

/// "Sync Game State" (`SYNCGAMESTAT`, factory 0x77d3f0, vtable 0xfda384, 0x48 B): enter (0x740fc0) runs
/// `FUN_006f4890` on the playlist manager G+0x2f3fc ([`Request::SyncGameState`]: for the host it sets the byte
/// +0x2e76 and the game start time, else it sends the sync message). The update (0x740fe0) exits through `EXIT`
/// once the byte is set ([`Query::GameStateSynced`]); otherwise, on a network layer with its byte +0xbe78
/// ([`Query::NetSyncing`], `FUN_004c2310`) it runs the sync again; it never exits on its own offline.
pub struct SyncGameState;

impl Behaviour for SyncGameState {
    fn on_enter(&mut self, cx: &mut Cx) {
        cx.host.request(Request::SyncGameState);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::GameStateSynced) {
            return exit_to(cx, "EXIT");
        }
        if ask_bool(cx, Query::NetSyncing) {
            cx.host.request(Request::SyncGameState);
        }
        false
    }
}

/// "Upload Challenge State" (`UPLDCHALSTE`, factory 0x77fff0, vtable 0xfdc1dc, 0x140 B): two request objects (+0xc8
/// slot 0, +0x100 slot 1). Enter (0x7413d0) resets both. The update (0x764790): when the Blaze session is
/// authenticated (`FUN_004c2050`: its state is 0xb, [`Query::NetAuthenticated`]) uploads the challenge state of
/// both lists (`FUN_00762df0`(0) and (1): [`Request::UploadChallengeState`]); exits through `EXIT`. The exit update
/// (0x7413f0) pumps and acknowledges both and is done when both are idle; the exit (0x741470) applies the
/// progression refresh `FUN_006fdb70` on G+0x2f418 ([`Request::ApplyChallengeResult`]).
pub struct UploadChallengeState;

impl Behaviour for UploadChallengeState {
    fn on_enter(&mut self, cx: &mut Cx) {
        net_reset(cx, 0);
        net_reset(cx, 1);
    }

    fn update(&mut self, cx: &mut Cx) -> bool {
        if ask_bool(cx, Query::NetAuthenticated) {
            cx.host.request(Request::UploadChallengeState { list: 0 });
            cx.host.request(Request::UploadChallengeState { list: 1 });
        }
        exit_to(cx, "EXIT")
    }

    fn exit_update(&mut self, cx: &mut Cx) -> bool {
        for slot in 0..2 {
            if ask(cx, Query::NetRequestState { slot }) != 0 && net_pump_state(cx, slot) == DONE {
                net_ack(cx, slot);
            }
        }
        ask(cx, Query::NetRequestState { slot: 0 }) == 0 && ask(cx, Query::NetRequestState { slot: 1 }) == 0
    }

    fn on_exit(&mut self, cx: &mut Cx) {
        cx.host.request(Request::ApplyChallengeResult);
    }
}

/// "Is PageRunner Context Open" (`IsPRCtxtOpen`, factory 0x77ebd0, vtable 0xfdae48, 0x48 B): the update (0x76f6c0)
/// exits through `YES` when the context named `CONTEXTNAME` is open in the poller G+0x2ee60 (`FUN_0083b8b0`) or
/// the Autolog G+0x2ed74 (`FUN_0083b580`, [`Query::AutologContextOpen`]), else `NO`.
pub struct IsContextOpen;

impl Behaviour for IsContextOpen {
    fn update(&mut self, cx: &mut Cx) -> bool {
        let name = cx.param_text("CONTEXTNAME");
        let open = ask_bool(cx, Query::AutologContextOpen { name });
        cx.set_exit(if open { super::game::YES } else { super::game::NO });
        true
    }
}

/// "Online Scoring Actions" (`ONLNSCRACT`, factory 0x77e770, vtable 0xfdac74, 0x48 B): every update (0x7436c0)
/// runs the online scoring actions of G+0x2f3cc (`FUN_006a3360`, [`Request::OnlineScoreActionsUpdate`]), holds
/// while the UI has messages up ([`Query::UiMessagesActive`]: the manager's word +0x12460 above 0) or the scoring
/// still has actions pending ([`Query::OnlineScoreBusy`], `FUN_006a6e50`), then exits through `EXIT`.
pub struct OnlineScoreActions;

impl Behaviour for OnlineScoreActions {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::OnlineScoreActionsUpdate);
        if ask_bool(cx, Query::UiMessagesActive) || ask_bool(cx, Query::OnlineScoreBusy) {
            return false;
        }
        exit_to(cx, "EXIT")
    }
}

/// "Reboot" (`REBOOT`, factory 0x779d90, vtable 0xfd8f54, 0x48 B): every update (0x75c830) posts the message 0xb on
/// the game message queue (`FUN_0076aa50`, [`Request::PostQueueMessage`]) and never exits.
pub struct Reboot;

impl Behaviour for Reboot {
    fn update(&mut self, cx: &mut Cx) -> bool {
        cx.host.request(Request::PostQueueMessage { function: 0x76aa50, id: 0xb });
        false
    }
}

/// "Sync Time" (`SYNCTIME`, factory 0x779540, vtable 0xfd8dc8, 0x48 B): the update (0x75cfb0) exits through `EXIT`
/// outside a game session ([`Query::OnlineGame`]) or once the clock is synced ([`Query::ClockSynced`]: the clock
/// object's state +0x6c is 2); while the clock is not syncing yet (state +0x6c not 1, [`Query::ClockSyncing`]) it
/// posts the message 5 (`FUN_0069a650`, [`Request::PostQueueMessage`]) every tick.
pub struct SyncTime;

impl Behaviour for SyncTime {
    fn update(&mut self, cx: &mut Cx) -> bool {
        if !ask_bool(cx, Query::OnlineGame) || ask_bool(cx, Query::ClockSynced) {
            return exit_to(cx, "EXIT");
        }
        if !ask_bool(cx, Query::ClockSyncing) {
            cx.host.request(Request::PostQueueMessage { function: 0x69a650, id: 5 });
        }
        false
    }
}
