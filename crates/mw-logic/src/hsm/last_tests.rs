//! `PICKNEXTFDE`, `HSMSHAKEDOWN` and `TELEPORTTOME` (`behaviours_last`), as `rest_tests`: one state as a machine's
//! root, the engine side recorded (requests) and scripted (queries by name).

use std::collections::HashMap;

use mw_formats::hsm::{Action, EntryPoint, Fallback, key};

use super::behaviours::Query;
use super::tests::{DT, data, param, state};
use super::*;
use crate::natives::Request;

#[derive(Default)]
struct Rec {
    reqs: Vec<Request>,
    answers: HashMap<&'static str, u64>,
    by_index: HashMap<u32, u64>,
}

impl Host for Rec {
    fn request(&mut self, r: Request) {
        self.reqs.push(r);
    }
    fn query(&mut self, q: &Query) -> Option<u64> {
        if let Query::ShakedownParam { index } = q {
            return self.by_index.get(index).copied();
        }
        self.answers.get(q.name()).copied()
    }
}

fn machine(code: &str, params: &[(&str, &str)], host: &mut Rec) -> (Hsm, u32) {
    let mut s = state(1, code, Fallback::RunChildren);
    s.parameters = params.iter().map(|(n, v)| param(n, v)).collect();
    s.entry_points.push(EntryPoint { id: key("ENTER"), action: Action::Child, spawns: vec![] });
    let mut hsm = Hsm::new(data(vec![s], vec![]), Registry::structural());
    let serial = hsm.start(1, host).unwrap();
    (hsm, serial)
}

fn step(hsm: &mut Hsm, host: &mut Rec, n: usize) {
    for _ in 0..n {
        hsm.update(host, DT);
    }
}

#[test]
fn pick_next_event_host_side() {
    let mut host = Rec::default();
    host.answers.insert("PlaylistWaitingForHost", 0);
    host.answers.insert("PlaylistValid", 0);
    host.answers.insert("PlaylistCurrentEvent", 77);
    host.answers.insert("OnlinePendingEvent", 462_174);
    let (mut hsm, s) = machine("PICKNEXTFDE", &[], &mut host);
    // The enter steps the playlist; no playlist yet: the picked event is cleared, state 2 waits for it.
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    assert_eq!(host.reqs, [Request::AdvancePlaylist, Request::SetPickedEvent { event: 0 }]);
    // The playlist appears: stepped again (state 0), then the pick: event kept, mode swapped, EXIT held for 3 s.
    host.answers.insert("PlaylistValid", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(
        host.reqs[2..],
        [
            Request::AdvancePlaylist,
            Request::SetPickedEvent { event: 0 },
            Request::SetPickedEvent { event: 77 },
            Request::SwapGameMode { mode: 462_174 }
        ]
    );
    assert!(hsm.is_running(s), "the exit update holds a picked event for 3 s");
    step(&mut hsm, &mut host, 100);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn pick_next_event_client_side_never_exits() {
    let mut host = Rec::default();
    host.answers.insert("PlaylistWaitingForHost", 1);
    host.answers.insert("PlaylistValid", 1);
    host.answers.insert("PlaylistFinished", 1);
    let (mut hsm, s) = machine("PICKNEXTFDE", &[], &mut host);
    step(&mut hsm, &mut host, 10);
    assert!(hsm.is_running(s));
    assert_eq!(host.reqs, [Request::AdvancePlaylist]);
}

fn shakedown(host: &mut Rec) -> (Hsm, u32) {
    host.answers.insert("GameModeClockRunning", 1);
    host.answers.insert("ShakedownParamKind", 0);
    // Words as plain 32-bit seconds: gold 1 s, silver 2 s, no score after 3 s, target 0.5 s.
    host.by_index.extend([(0, 1), (1, 2), (2, 3), (3, 0)]);
    machine("HSMSHAKEDOWN", &[], host)
}

#[test]
fn shakedown_medal_and_exits() {
    // Evaded within the gold time (target 0 s: over at once, not in a pursuit): medal 0 + 1.
    let mut host = Rec::default();
    let (mut hsm, s) = shakedown(&mut host);
    step(&mut hsm, &mut host, 1);
    assert_eq!(hsm.result(s), Some(key("EVADED")));
    let result = host.reqs.iter().find_map(|r| match r {
        Request::SetShakedownResult { medal, ticks } => Some((*medal, *ticks)),
        _ => None,
    });
    assert_eq!(result, Some((1, 100_000)), "1/30 s = 100000 ticks");
    assert!(host.reqs.contains(&Request::SetGameModeElapsedTicks { ticks: 100_000 }));
}

#[test]
fn shakedown_pursuit_and_limits() {
    // In a pursuit past the target time: runs on, then the no-score limit (3 s) ends it.
    let mut host = Rec::default();
    let (mut hsm, s) = shakedown(&mut host);
    host.answers.insert("LocalPlayerInPursuit", 1);
    step(&mut hsm, &mut host, 80);
    assert!(hsm.is_running(s));
    assert!(host.reqs.contains(&Request::SetShakedownOver { over: true }));
    step(&mut hsm, &mut host, 20);
    assert_eq!(hsm.result(s), Some(key("NOSCORE")));
    assert!(matches!(host.reqs.last(), Some(Request::SetShakedownResult { medal: 4, .. })));

    // Busted: exits at once through BUSTED, whatever the clock does.
    let mut host = Rec::default();
    let (mut hsm, s) = shakedown(&mut host);
    host.answers.insert("LocalPursuitState", 2);
    step(&mut hsm, &mut host, 1);
    assert_eq!(hsm.result(s), Some(key("BUSTED")));

    // The clock stopped: nothing counts.
    let mut host = Rec::default();
    let (mut hsm, s) = shakedown(&mut host);
    host.answers.insert("GameModeClockRunning", 0);
    step(&mut hsm, &mut host, 5);
    assert!(hsm.is_running(s));
    assert!(host.reqs.is_empty());
}

#[test]
fn shakedown_medal_times_from_ticks_parameter() {
    // Mode 3: 64-bit values / 3 / 1e6: 6e6 ticks → 2 s; the fourth word stays 32-bit (target 1 s).
    let mut host = Rec::default();
    host.answers.insert("GameModeClockRunning", 1);
    host.answers.insert("ShakedownParamKind", 3);
    host.by_index.extend([(0, 3_000_000), (1, 6_000_000), (2, 9_000_000), (3, 1)]);
    let (mut hsm, s) = machine("HSMSHAKEDOWN", &[], &mut host);
    // 40 frames = 1.33 s: past the target (1 s) and the gold time (1 s), within silver (2 s): medal 1 + 1.
    step(&mut hsm, &mut host, 40);
    assert_eq!(hsm.result(s), Some(key("EVADED")));
    let medal = host.reqs.iter().find_map(|r| match r {
        Request::SetShakedownResult { medal, .. } => Some(*medal),
        _ => None,
    });
    assert_eq!(medal, Some(2));
}

#[test]
fn teleport_to_meetup() {
    let mut host = Rec::default();
    host.answers.insert("OnlinePendingEvent", 5);
    host.answers.insert("MeetupTrigger", 9);
    host.answers.insert("MeetupTeleport", 1);
    host.answers.insert("MeetupAreaReady", 0);
    let (mut hsm, s) = machine("TELEPORTTOME", &[("Sequence", "1695714")], &mut host);
    // A wrecked player waits.
    host.answers.insert("LocalPlayerWrecked", 1);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    host.answers.insert("LocalPlayerWrecked", 0);
    step(&mut hsm, &mut host, 1);
    assert!(matches!(host.reqs[0], Request::PlaySequence { sequence: 1_695_714, player: None, .. }));
    assert_eq!(host.reqs[1], Request::SetMeetupTeleporting { on: true });
    // The exit holds while the area loads and the 7.5 s countdown runs (225 frames), then lets go.
    step(&mut hsm, &mut host, 100);
    assert!(hsm.is_running(s));
    host.answers.insert("MeetupAreaReady", 1);
    step(&mut hsm, &mut host, 100);
    assert!(hsm.is_running(s), "the countdown has not ended");
    step(&mut hsm, &mut host, 40);
    assert_eq!(hsm.result(s), Some(EXIT));
    let n = host.reqs.len();
    assert_eq!(host.reqs[n - 3..n - 1], [Request::PostMeetupEndMessage, Request::SetMeetupTeleporting { on: false }]);
    assert!(matches!(host.reqs[n - 1], Request::StopPlayback { .. }));
}

#[test]
fn teleport_to_meetup_refused_or_already_there() {
    // Refused destination: the manager is told, nothing is held on exit.
    let mut host = Rec::default();
    host.answers.insert("MeetupTeleport", 0);
    let (mut hsm, s) = machine("TELEPORTTOME", &[], &mut host);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert!(host.reqs.contains(&Request::MeetupTeleportRefused));
    assert!(!host.reqs.contains(&Request::SetMeetupTeleporting { on: true }));

    // Already in the trigger: no teleport at all.
    let mut host = Rec::default();
    host.answers.insert("MeetupInCheckpoint", 1);
    let (mut hsm, s) = machine("TELEPORTTOME", &[], &mut host);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert!(!host.reqs.contains(&Request::MeetupTeleportRefused));
}
