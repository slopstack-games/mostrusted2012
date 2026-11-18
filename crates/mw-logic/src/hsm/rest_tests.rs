//! The last batch of behaviour types (`behaviours_rest`), as `flow_tests`: one state as a machine's root (or the
//! child of a parent that handles its exits), the engine side recorded (requests) and scripted (queries by name,
//! per player slot, and a network inbox).

use std::collections::HashMap;

use mw_formats::hsm::{Action, ChildExitPoint, ChildExits, EntryPoint, Fallback, Spawn, key};

use super::behaviours::Query;
use super::behaviours::online::NetMessage;
use super::tests::{DT, data, param, state};
use super::*;
use crate::natives::Request;

#[derive(Default)]
struct Rec {
    reqs: Vec<Request>,
    answers: HashMap<&'static str, u64>,
    per_slot: HashMap<(&'static str, u32), u64>,
    inbox: Vec<(u32, u64)>,
    published: Vec<(String, Json)>,
    retracted: Vec<String>,
}

impl Host for Rec {
    fn request(&mut self, r: Request) {
        self.reqs.push(r);
    }
    fn query(&mut self, q: &Query) -> Option<u64> {
        match q {
            Query::PlayerSlotUsed { index } | Query::PlayerMidGameJoining { index } => {
                if let Some(&v) = self.per_slot.get(&(q.name(), *index)) {
                    return Some(v);
                }
            }
            Query::TakeNetMessage { kind } => {
                let i = self.inbox.iter().position(|(k, _)| k == kind)?;
                return Some(self.inbox.remove(i).1);
            }
            _ => {}
        }
        self.answers.get(q.name()).copied()
    }
    fn publish(&mut self, path: &str, data: &Json) {
        self.published.push((path.to_owned(), data.clone()));
    }
    fn retract(&mut self, path: &str) {
        self.retracted.push(path.to_owned());
    }
}

fn machine(code: &str, params: &[(&str, &str)]) -> (Hsm, Rec, u32) {
    let mut s = state(1, code, Fallback::RunChildren);
    s.parameters = params.iter().map(|(n, v)| param(n, v)).collect();
    s.entry_points.push(EntryPoint { id: key("ENTER"), action: Action::Child, spawns: vec![] });
    let mut hsm = Hsm::new(data(vec![s], vec![]), Registry::structural());
    let mut host = Rec::default();
    let serial = hsm.start(1, &mut host).unwrap();
    (hsm, host, serial)
}

/// A parent running one child of behaviour `code`, which the parent leaves through `exits`.
fn wrapped(code: &str, params: &[(&str, &str)], exits: &[&str]) -> (Hsm, Rec, u32) {
    let mut child = state(2, code, Fallback::RunChildren);
    child.parameters = params.iter().map(|(n, v)| param(n, v)).collect();
    child.entry_points.push(EntryPoint { id: key("ENTER"), action: Action::Child, spawns: vec![] });
    let mut parent = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    parent.entry_points.push(EntryPoint {
        id: key("ENTER"),
        action: Action::Child,
        spawns: vec![Spawn { entry_point: key("ENTER"), state: 2 }],
    });
    parent.children.push(ChildExits {
        state: 2,
        exits: exits
            .iter()
            .map(|e| ChildExitPoint { parent_exit: key(e), child_exit: key(e), action: Action::Exit, spawns: vec![] })
            .collect(),
    });
    let mut hsm = Hsm::new(data(vec![parent, child], vec![]), Registry::structural());
    let mut host = Rec::default();
    let s = hsm.start(1, &mut host).unwrap();
    (hsm, host, s)
}

fn step(hsm: &mut Hsm, host: &mut Rec, n: usize) {
    for _ in 0..n {
        hsm.update(host, DT);
    }
}

fn net_sends(host: &Rec) -> Vec<&NetMessage> {
    host.reqs
        .iter()
        .filter_map(|r| match r {
            Request::NetSend { message, .. } => Some(message),
            _ => None,
        })
        .collect()
}

#[test]
fn controller_binding_and_debug_types() {
    // BINDCONTROLL: released while bound; unbound: neutral values re-read, waits for the press to end, listens.
    let (mut hsm, mut host, s) = machine("BINDCONTROLL", &[("ANYBUTTON", "True")]);
    host.answers.insert("PrimaryUserBound", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::ReleasePrimaryUser(true)]);
    host.answers.insert("PrimaryUserBound", 0);
    host.answers.insert("PadInputPending", 1);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs[1..], [Request::ReleasePrimaryUser(false), Request::ResetNeutralValues]);
    host.answers.insert("PadInputPending", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::ListenForPrimaryUser { listen: true, any_button: true }));
    // The press binds the user: listening stops, EXIT; nothing creates a player.
    host.answers.insert("PrimaryUserBound", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert_eq!(host.reqs.last(), Some(&Request::ListenForPrimaryUser { listen: false, any_button: false }));
    assert!(!host.reqs.contains(&Request::CreateLocalPlayer));

    // DBGAUTOBIND: players removed, the auto-bind message every frame while unbound, then the local player.
    let (mut hsm, mut host, s) = machine("DBGAUTOBIND", &[]);
    host.answers.insert("PrimaryUserBound", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs.iter().filter(|r| **r == Request::DebugAutobindPad).count(), 3);
    assert!(host.reqs.contains(&Request::RemovePlayers));
    assert!(host.reqs.contains(&Request::ListenForPrimaryUser { listen: true, any_button: true }));
    host.answers.insert("PrimaryUserBound", 1);
    host.answers.insert("LocalPlayerReady", 0);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.contains(&Request::CreateLocalPlayer));
    assert!(hsm.is_running(s), "waits for the local player");
    host.answers.insert("LocalPlayerReady", 1);
    step(&mut hsm, &mut host, 1);
    assert_eq!(hsm.result(s), Some(EXIT));

    // DBGSTARTFRID: waits for the event data and the world, then the pending event.
    let (mut hsm, mut host, s) = machine("DBGSTARTFRID", &[]);
    host.answers.insert("DebugEventDataSet", 1);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
    host.answers.insert("DebugWorldReady", 1);
    host.answers.insert("DebugFreeRideEvent", 390_703);
    step(&mut hsm, &mut host, 2);
    assert_eq!(
        host.reqs,
        [Request::SetSinglePlayerRollout, Request::SetPendingEvent { event: 390_703, online: false }]
    );
    assert_eq!(hsm.result(s), Some(EXIT));

    // DEBUGUNLOCKS exits at once; HSM.ENABLEDE / HSM.CHECKFOR only run their children.
    let (mut hsm, mut host, s) = machine("DEBUGUNLOCKS", &[]);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    for code in ["HSM.ENABLEDE", "HSM.CHECKFOR"] {
        let (mut hsm, mut host, s) = machine(code, &[("DEMOMODE", "2")]);
        step(&mut hsm, &mut host, 4);
        assert!(hsm.is_running(s) && host.reqs.is_empty(), "{code}");
    }

    // HSM.SELECTEA: the shortcut picks the exit; 0x1d stores class 1 first. Unhandled exits stay.
    let (mut hsm, mut host, s) = wrapped("HSM.SELECTEA", &[], &["EZ_CARS", "EZ_SPEC_MOD"]);
    step(&mut hsm, &mut host, 2);
    assert!(hsm.is_running(s));
    host.answers.insert("EasyDriveShortcut", 0x1d);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[0], Request::SetKinectSelectedClass { class: 1 });
    assert_eq!(hsm.result(s), Some(key("EZ_SPEC_MOD")));
    let (mut hsm, mut host, s) = wrapped("HSM.SELECTEA", &[], &["EZ_CARS"]);
    host.answers.insert("EasyDriveShortcut", 0xd);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s), "EZ_RACES is not handled by the parent");
    host.answers.insert("EasyDriveShortcut", 0xf);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("EZ_CARS")));
}

#[test]
fn blaze_game_types() {
    // LAUNCH offline: the string is set, nothing is sent (not the admin), the update holds.
    let (mut hsm, mut host, s) = machine("LAUNCH", &[]);
    host.answers.insert("NetRequestState", 0);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.contains(&Request::SetSignInStatusString { string: 117_673 }));
    assert!(net_sends(&host).is_empty() && hsm.is_running(s));
    // The admin with everyone connected launches once, then exits when the game started.
    host.answers.insert("NetGameAdmin", 1);
    step(&mut hsm, &mut host, 3);
    assert_eq!(net_sends(&host), [&NetMessage::LaunchGame]);
    host.answers.insert("NetGameStarted", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));

    // ENDGAME: exits at once when no game runs; in a game the admin sends once, and counts the game on exit.
    let (mut hsm, mut host, s) = machine("ENDGAME", &[]);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    let (mut hsm, mut host, s) = machine("ENDGAME", &[]);
    host.answers.insert("NetGameStarted", 1);
    host.answers.insert("NetGameAdmin", 1);
    host.answers.insert("NetRequestState", 0);
    step(&mut hsm, &mut host, 4);
    assert_eq!(net_sends(&host), [&NetMessage::EndGame]);
    host.answers.insert("NetGameStarted", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert_eq!(host.reqs.last(), Some(&Request::CountOnlineGame));

    // RESETGAME: only in the post-game state; no sent flag (the busy object guards).
    let (mut hsm, mut host, s) = machine("RESETGAME", &[]);
    host.answers.insert("SessionRaceState", 1);
    host.answers.insert("NetGameAdmin", 1);
    host.answers.insert("NetRequestState", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(net_sends(&host).len(), 3);
    assert!(net_sends(&host).iter().all(|m| **m == NetMessage::ResetGame));
    host.answers.insert("SessionRaceState", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));

    // HSM.UPLOADON: offline (state done by default) acknowledged and EXIT; nothing sent: idle, EXIT at once.
    let (mut hsm, mut host, s) = machine("HSM.UPLOADON", &[]);
    step(&mut hsm, &mut host, 3);
    assert_eq!(net_sends(&host), [&NetMessage::UploadUnlocks]);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert!(host.reqs.contains(&Request::NetAck { slot: 0 }));

    // WAIT4ALLSYN, client: reports, exits on the host's message.
    let (mut hsm, mut host, s) = machine("WAIT4ALLSYN", &[]);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::NetSyncReport]);
    host.inbox.push((0x47, 0));
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    // Host: slots 0 (itself), 1 and 2 used, 2 joining mid game; done once slot 1 reported.
    let (mut hsm, mut host, s) = machine("WAIT4ALLSYN", &[]);
    host.answers.insert("OnlineHost", 1);
    host.answers.insert("LocalPlayerSlot", 0);
    for i in 0..3 {
        host.per_slot.insert(("PlayerSlotUsed", i), 1);
    }
    host.per_slot.insert(("PlayerMidGameJoining", 2), 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s) && host.reqs.is_empty());
    host.inbox.push((0x46, 1));
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::NetSyncComplete]);
    assert_eq!(hsm.result(s), Some(EXIT));
    // Migrating: nothing at all.
    let (mut hsm, mut host, s) = machine("WAIT4ALLSYN", &[]);
    host.answers.insert("NetHostMigrating", 1);
    host.inbox.push((0x47, 0));
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s) && host.reqs.is_empty());
}

#[test]
fn online_free_drive_types() {
    // JOINEDLEFT: slot 0 local, 1 present on enter; 2 joins, 1 leaves.
    let (mut hsm, mut host, s) = machine("JOINEDLEFT", &[]);
    host.answers.insert("LocalPlayerSlot", 0);
    host.per_slot.insert(("PlayerSlotUsed", 0), 1);
    host.per_slot.insert(("PlayerSlotUsed", 1), 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::CachePlayerName { slot: 0 }, Request::CachePlayerName { slot: 1 }]);
    host.reqs.clear();
    host.per_slot.insert(("PlayerSlotUsed", 2), 1);
    host.per_slot.insert(("PlayerSlotUsed", 1), 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(
        host.reqs,
        [
            Request::PlayerJoinedLeftFeedback { slot: 1, joined: false },
            Request::CachePlayerName { slot: 2 },
            Request::PlayerJoinedLeftFeedback { slot: 2, joined: true },
        ]
    );
    assert!(hsm.is_running(s));

    // MMPICONSONL: the icons every frame, never exits.
    let (mut hsm, mut host, s) = machine("MMPICONSONL", &[("WRECKEDPOI", "1013500")]);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, vec![Request::UpdateOnlineMinimapIcons; 3]);
    assert!(hsm.is_running(s));

    // ONLINECARSWA: damage off, rollout, sequence; the flag moves on to the swap; the vehicle ready: EXIT.
    let (mut hsm, mut host, s) = machine("ONLINECARSWA", &[("SEQUENCE", "1695714")]);
    host.answers.insert("LocalVehicle", 4);
    host.answers.insert("LocalVehicleAlive", 1);
    host.answers.insert("SequencePlaying", 1);
    host.answers.insert("LocalVehicleReady", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs[0], Request::VehicleDamage { vehicle: 4, deactivated: true });
    assert_eq!(host.reqs[1], Request::CommitPendingRollout);
    assert!(matches!(host.reqs[2], Request::PlaySequenceWithFlag { sequence: 1_695_714, player: None, .. }));
    assert_eq!(host.reqs.len(), 3);
    host.answers.insert("SequenceFlagSet", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::OnlineCarSwapApply));
    host.answers.insert("LocalVehicleReady", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    let tail = &host.reqs[host.reqs.len() - 3..];
    assert_eq!(
        tail,
        [
            Request::OnlineCarSwapFinish,
            Request::ResetVehicleDamage { vehicle: 4 },
            Request::VehicleDamage { vehicle: 4, deactivated: false },
        ]
    );

    // SHOWMEETUPPO: waits for the online event, shows the icons, publishes the root, removes them on exit.
    let (mut hsm, mut host, s) = machine("SHOWMEETUPPO", &[]);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    assert_eq!(host.published.len(), 1);
    host.answers.insert("OnlinePendingEvent", 77);
    host.answers.insert("MeetupTrigger", 5);
    host.answers.insert("MeetupInCheckpoint", 1);
    host.answers.insert("MeetupDistance", u64::from(12.5f32.to_bits()));
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::ShowMeetupPois { trigger: 5 }]);
    let (path, last) = host.published.last().unwrap();
    assert_eq!(path, "MeetupPOI");
    assert_eq!(last.get("LocalPlayerInCheckpoint"), Some(&Json::Bool(true)));
    assert_eq!(last.get("LocalPlayerDistance"), Some(&Json::Double(12.5)));
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::RemoveMeetupPois));
    assert_eq!(host.retracted, ["MeetupPOI"]);
}

#[test]
fn event_and_ui_types() {
    // KILLCAMSEQ: plays on the style-2 player until the flag; waits for the player when leaving.
    let (mut hsm, mut host, s) = machine("KILLCAMSEQ", &[("SEQUENCE", "287389"), ("INPUTSTRING1", "")]);
    host.answers.insert("SequencePlaying", 1);
    host.answers.insert("SequencePlayerBusy", 1);
    step(&mut hsm, &mut host, 3);
    assert!(matches!(host.reqs[..], [Request::PlaySequenceWithFlag { sequence: 287_389, player: Some(2), .. }]));
    host.answers.insert("SequenceFlagSet", 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s), "the player is still busy");
    host.answers.insert("SequencePlayerBusy", 0);
    step(&mut hsm, &mut host, 1);
    assert_eq!(hsm.result(s), Some(EXIT));
    // Without a sequence: EXIT at once.
    let (mut hsm, mut host, s) = machine("KILLCAMSEQ", &[("SEQUENCE", "")]);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));

    // SHOWCARSELEC: shown, focused once ready; hidden on the way out, finished once closed.
    let (mut hsm, mut host, s) = machine("SHOWCARSELEC", &[("SHOWLOADING", "False")]);
    host.answers.insert("CarSelectReady", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::ShowCarSelect { show_loading: false }]);
    host.answers.insert("CarSelectReady", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::FocusCarSelect));
    host.answers.insert("CarSelectClosed", 0);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs.last(), Some(&Request::HideCarSelect { show_loading: false }));
    assert!(hsm.is_running(s), "waits for the car select to close");
    host.answers.insert("CarSelectClosed", 1);
    step(&mut hsm, &mut host, 2);
    assert!(!hsm.is_running(s));

    // STARTCARSELE offline: the world byte kept, set, restored.
    let (mut hsm, mut host, s) = machine("STARTCARSELE", &[("ONLINE", "False"), ("CARSELCTDATA", "")]);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::StartCarSelect { online: false }, Request::SetWorldShownFlag { on: true }]);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[2..], [Request::SetWorldShownFlag { on: false }, Request::StopCarSelect]);
    // Online: the rollouts on exit.
    let (mut hsm, mut host, s) = machine("STARTCARSELE", &[("ONLINE", "True")]);
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[host.reqs.len() - 2..], [Request::SetSinglePlayerRollout, Request::CommitPendingRollout]);

    // SHOW_DIALOG: the layout and the published strings.
    let params = [
        ("LAYOUT", "105409"),
        ("TITLE", "117489"),
        ("BODY", "117013"),
        ("NAV_PROMPT3", "117629"),
        ("RUN_HIGHEST", "True"),
    ];
    let (mut hsm, mut host, s) = machine("SHOW_DIALOG", &params);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[0], Request::RunLayout { layout: 105_409, layer: 0x18 });
    let (path, data) = &host.published[0];
    assert_eq!(path, "UserFeedbackDialog");
    assert_eq!(data.get("Title"), Some(&Json::Int(117_489)));
    assert_eq!(data.get("Prompt1"), Some(&Json::Int(0)));
    assert_eq!(data.get("Prompt3"), Some(&Json::Int(117_629)));
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 5);
    assert!(!hsm.is_running(s));
    assert_eq!(host.retracted, ["UserFeedbackDialog"]);

    // TRYTOPLAYSPE: the second event plays; SUCCESS once it ended. Failed load: FAILURE.
    let (mut hsm, mut host, s) = machine("TRYTOPLAYSPE", &[("SPEECHEVENTS", "1871531")]);
    host.answers.insert("TryPlaySpeech", 0x13);
    host.answers.insert("SpeechPlaying", 1);
    step(&mut hsm, &mut host, 3);
    assert!(matches!(&host.reqs[..], [Request::LoadSpeechEvents { events, .. }] if *events == [1_871_531]));
    assert!(hsm.is_running(s));
    host.answers.insert("SpeechPlaying", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("SUCCESS")));
    let (mut hsm, mut host, s) = machine("TRYTOPLAYSPE", &[("SPEECHEVENTS", "1")]);
    host.answers.insert("SpeechLoadState", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("FAILURE")));
    let (mut hsm, mut host, s) = machine("TRYTOPLAYSPE", &[("SPEECHEVENTS", "1")]);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("FAILURE")), "nothing played");

    // WAYPOINTS: POIs, the route every frame unless blocked, removed on exit.
    let (mut hsm, mut host, s) = machine("WAYPOINTS", &[("POIID", "1013519")]);
    host.answers.insert("WaypointPoiCount", 3);
    step(&mut hsm, &mut host, 2);
    host.answers.insert("MapRouteBlocked", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(
        host.reqs,
        [
            Request::AddEventWaypointPois { poi: 1_013_519 },
            Request::RouteToFirstWaypointPoi,
            Request::RouteToFirstWaypointPoi
        ]
    );
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::RemoveEventWaypointPois));
}
