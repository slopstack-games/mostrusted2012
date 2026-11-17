//! The boot-flow behaviour types (`behaviours_game`, `behaviours_ui`): one state as a machine's root, the engine
//! side recorded (requests) and scripted (queries by name).

use std::collections::HashMap;

use mw_formats::hsm::{Action, EntryPoint, Fallback, key};

use super::behaviours::Query;
use super::behaviours::game::{NO, YES};
use super::tests::{DT, data, param, state};
use super::*;
use crate::natives::Request;

#[derive(Default)]
struct Rec {
    reqs: Vec<Request>,
    answers: HashMap<&'static str, u64>,
    ints: HashMap<String, i64>,
}

impl Host for Rec {
    fn request(&mut self, r: Request) {
        self.reqs.push(r);
    }
    fn query(&mut self, q: &Query) -> Option<u64> {
        self.answers.get(q.name()).copied()
    }
    fn binding_int(&mut self, path: &str) -> Option<i64> {
        self.ints.get(path).copied()
    }
}

/// One state of behaviour `code` as the machine's root, with its parameters and the entry point `entry`.
fn machine(code: &str, params: &[(&str, &str)], entry: &str) -> (Hsm, Rec, u32) {
    let mut s = state(1, code, Fallback::RunChildren);
    s.parameters = params.iter().map(|(n, v)| param(n, v)).collect();
    s.entry_points.push(EntryPoint { id: key(entry), action: Action::Child, spawns: vec![] });
    let mut hsm = Hsm::new(data(vec![s], vec![]), Registry::structural());
    let mut host = Rec::default();
    let serial = hsm.start(1, &mut host).unwrap();
    (hsm, host, serial)
}

fn step(hsm: &mut Hsm, host: &mut Rec, n: usize) {
    for _ in 0..n {
        hsm.update(host, DT);
    }
}

#[test]
fn load_world_asks_once_and_exits() {
    let (mut hsm, mut host, s) = machine("LOADWORLD", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::LoadWorld]);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn load_profile_waits_for_the_save_system_and_the_load() {
    let (mut hsm, mut host, s) = machine("LOADPROFILE", &[], "ENTER_NO_PRF");
    host.answers.insert("ProfileReady", 0);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
    host.answers.insert("ProfileReady", 1);
    host.answers.insert("ProfileLoaded", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::LoadProfile]);
    assert!(hsm.is_running(s));
    host.answers.insert("ProfileLoaded", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("EXIT_NO_PRF")));
    assert_eq!(host.reqs.last(), Some(&Request::ApplyProfileSettings));
    // Entered through ENTER: EXIT.
    let (mut hsm, mut host, s) = machine("LOADPROFILE", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn in_online_game_picks_yes_or_no() {
    let (mut hsm, mut host, s) = machine("IN_ON_GAME", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(NO));
    let (mut hsm, mut host, s) = machine("IN_ON_GAME", &[], "ENTER");
    host.answers.insert("OnlineGame", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(YES));
}

#[test]
fn run_game_mode_runs_the_event_and_exits_when_it_ends() {
    let (mut hsm, mut host, s) = machine("RUNGAMEMODE", &[], "ENTER");
    host.answers.insert("GameModeEvent", 0);
    step(&mut hsm, &mut host, 3);
    // NextEvent defaults to 0 and the running event 0 == it: held.
    assert!(hsm.is_running(s));
    assert_eq!(host.reqs, [Request::StartGameMode { event: 0 }]);
    host.answers.insert("GameModeEvent", 9);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert!(!host.reqs.contains(&Request::StopGameMode), "ended by itself: nothing to stop");
}

#[test]
fn run_game_mode_exited_from_outside_stops_the_mode() {
    let (mut hsm, mut host, s) = machine("RUNGAMEMODE", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.contains(&Request::StopGameMode));
    assert!(!hsm.is_running(s));
}

#[test]
fn game_mode_exits_through_disconnect_when_the_session_is_lost() {
    let (mut hsm, mut host, s) = machine("GAMEMODE", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    host.answers.insert("SessionLost", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("DISCONNECT")));
    assert_eq!(host.reqs, [Request::SessionLost]);
}

#[test]
fn run_game_logic_plays_a_sequence_until_it_ends() {
    let (mut hsm, mut host, s) = machine("RUNGAMELOGIC", &[("Sequence", "42"), ("Style", "2")], "ENTER");
    host.answers.insert("SequencePlaying", 1);
    step(&mut hsm, &mut host, 3);
    let Some(Request::PlaySequence { playback, sequence: 42, player: Some(2) }) = host.reqs.first().cloned() else {
        panic!("{:?}", host.reqs)
    };
    assert!(hsm.is_running(s));
    host.answers.insert("SequencePlaying", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert_eq!(host.reqs.last(), Some(&Request::StopPlayback { playback }));
}

#[test]
fn run_game_logic_without_a_sequence_exits_at_once_and_reads_the_binding() {
    let (mut hsm, mut host, s) = machine("RUNGAMELOGIC", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    assert_eq!(hsm.result(s), Some(EXIT));
    // BindingPath wins over Sequence; UseSimTimer picks the sim-timer player.
    let (mut hsm, mut host, _) =
        machine("RUNGAMELOGIC", &[("BindingPath", "A.B"), ("Sequence", "1"), ("UseSimTimer", "true")], "ENTER");
    host.ints.insert("A.B".into(), 9);
    step(&mut hsm, &mut host, 1);
    assert!(matches!(host.reqs[0], Request::PlaySequence { sequence: 9, player: Some(1), .. }));
}

#[test]
fn page_context_is_pushed_waited_for_and_left() {
    let params = [("CONTEXT_NAME", "boot"), ("DEF_PAGE_REF", "491012"), ("RESTOREGRP", "true")];
    let (mut hsm, mut host, s) = machine("RUN_PG_RNNR2", &params, "ENTER");
    host.answers.insert("PageContextExists", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::PushPageContext { name: "boot".into(), page: "491012".into() }]);
    // Entered once the context exists; it asks for an exit point.
    host.answers.insert("PageContextExists", 1);
    host.answers.insert("PageContextExit", 77);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(77));
    assert!(host.reqs.contains(&Request::LeavePageContext { name: "boot".into(), suspend: false }));
    assert_eq!(host.reqs.last(), Some(&Request::RestoreFocusGroup { group: 0 }));
}

#[test]
fn page_context_exit_waits_while_busy() {
    let (mut hsm, mut host, s) = machine("RUN_PG_RNNR2", &[("CONTEXT_NAME", "c"), ("SUSPEND", "true")], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    host.answers.insert("PageContextBusy", 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    assert!(host.reqs.contains(&Request::LeavePageContext { name: "c".into(), suspend: true }));
    host.answers.insert("PageContextBusy", 0);
    step(&mut hsm, &mut host, 2);
    assert!(!hsm.is_running(s));
}

#[test]
fn widget_group_is_activated_and_deactivated() {
    let (mut hsm, mut host, s) = machine("ACTI_WDGT_GR", &[("GROUP", "12"), ("INSTANTEXIT", "true")], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::ActivateWidgetGroup { group: 12 }]);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.contains(&Request::DeactivateWidgetGroup { group: 12, instant: true }));
    assert!(!hsm.is_running(s));
}

#[test]
fn widget_group_waits_for_a_page_and_a_group_above_zero() {
    let (mut hsm, mut host, _) = machine("ACTI_WDGT_GR", &[("GROUP", "0")], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
    let (mut hsm, mut host, _) = machine("ACTI_WDGT_GR", &[("GROUP", "3")], "ENTER");
    host.answers.insert("PageReady", 0);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
}

#[test]
fn primary_user_is_released_then_bound() {
    let (mut hsm, mut host, s) = machine("HSB_P1BIND", &[("ANYBUTTON", "true")], "ENTER");
    host.answers.insert("PrimaryUserBound", 1);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::ReleasePrimaryUser(true)]);
    host.answers.insert("PrimaryUserBound", 0);
    step(&mut hsm, &mut host, 1);
    assert_eq!(&host.reqs[1..3], [Request::RemovePlayers, Request::ReleasePrimaryUser(false)]);
    assert!(host.reqs.contains(&Request::ListenForPrimaryUser { listen: true, any_button: true }));
    host.answers.insert("PrimaryUserBound", 1);
    host.answers.insert("LocalPlayerReady", 0);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.contains(&Request::CreateLocalPlayer));
    assert!(hsm.is_running(s), "the exit waits for the local player");
    host.answers.insert("LocalPlayerReady", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn black_background_sets_and_exits() {
    let (mut hsm, mut host, s) = machine("SETBLACKBACK", &[("ENABLED", "true")], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::SetBlackBackground(true)]);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn frame_rate_mode_is_restored_on_exit() {
    let (mut hsm, mut host, s) = machine("CHANGEVSYNCS", &[("FULL_RATE", "true")], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::SetFullRate(true), Request::SetFullRate(false)]);
}

#[test]
fn mute_sound_counts_its_reason_up_and_down() {
    let (mut hsm, mut host, s) = machine("MUTE_SOUND", &[("MUTEREASON", "carselect")], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::MuteSound { reason: 2, on: true }, Request::MuteSound { reason: 2, on: false }]);
}

#[test]
fn load_all_opens_the_gate_and_exits() {
    let (mut hsm, mut host, s) = machine("LOAD_ALL", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::LoadAll]);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn in_game_enters_once_and_waits_for_the_game_to_be_left() {
    let (mut hsm, mut host, s) = machine("BINGAME", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::EnterGame]);
    hsm.request_exit(s);
    host.answers.insert("GameLeaving", 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    assert_eq!(host.reqs, [Request::EnterGame, Request::LeaveGame], "leaving starts once");
    host.answers.insert("GameLeaving", 0);
    step(&mut hsm, &mut host, 2);
    assert!(!hsm.is_running(s));
}

#[test]
fn playing_starts_and_stops_the_session() {
    let (mut hsm, mut host, s) = machine("HSM_PLAYING", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::StartPlaying, Request::StopPlaying]);
    assert!(!hsm.is_running(s));
}
