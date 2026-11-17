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

#[test]
fn auto_select_world_selects_or_leaves_it_unselected() {
    let (mut hsm, mut host, s) = machine("DBAUTOSELWD", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::AutoSelectWorld]);
    assert_eq!(hsm.result(s), Some(key("SELECTED")));
    let (mut hsm, mut host, s) = machine("DBAUTOSELWD", &[], "ENTER");
    host.answers.insert("AutoSelectableWorld", 0);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    assert_eq!(hsm.result(s), Some(key("UNSELECTED")));
}

#[test]
fn select_profile_waits_for_the_user_then_names_the_profile() {
    let (mut hsm, mut host, s) = machine("SELECT_PROF", &[], "ENTER");
    host.answers.insert("UserId", 0);
    step(&mut hsm, &mut host, 4);
    assert!(hsm.is_running(s) && host.reqs.is_empty(), "no persona yet: waits");
    host.answers.insert("UserId", 1234);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::SetProfileUser { user: 1234 }, Request::ProfileUserSelected]);
    assert_eq!(hsm.result(s), Some(key("SUCCESS")));
}

#[test]
fn wait_for_feedback_group_waits_until_loaded_and_skips_group_zero() {
    let (mut hsm, mut host, s) = machine("WAIT4FEEDBK", &[("GROUP", "5")], "ENTER");
    host.answers.insert("FeedbackGroupLoaded", 0);
    step(&mut hsm, &mut host, 4);
    assert!(hsm.is_running(s));
    host.answers.insert("FeedbackGroupLoaded", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    let (mut hsm, mut host, s) = machine("WAIT4FEEDBK", &[("GROUP", "0")], "ENTER");
    host.answers.insert("FeedbackGroupLoaded", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    // the binding overrides GROUP
    let (mut hsm, mut host, s) = machine("WAIT4FEEDBK", &[("GROUP", "5"), ("GROUPBINDING", "A.B")], "ENTER");
    host.answers.insert("FeedbackGroupLoaded", 0);
    host.ints.insert("A.B".into(), 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn wait_for_vehicle_list_waits_until_loaded() {
    let (mut hsm, mut host, s) = machine("WAITFORVEHIC", &[], "ENTER");
    host.answers.insert("VehicleListLoaded", 0);
    step(&mut hsm, &mut host, 4);
    assert!(hsm.is_running(s));
    host.answers.insert("VehicleListLoaded", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn is_new_profile_and_is_auto_testing_exit_at_once() {
    let (mut hsm, mut host, s) = machine("ISNEWPROFILE", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(YES));
    let (mut hsm, mut host, s) = machine("ISNEWPROFILE", &[], "ENTER");
    host.answers.insert("NewProfile", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(NO));
    let (mut hsm, mut host, s) = machine("ISAUTOTESTIN", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(NO));
}

#[test]
fn startup_script_runs_its_children_like_the_base() {
    let (mut hsm, mut host, s) = machine("HSB_STRSCRPT", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s), "no exit of its own");
}

#[test]
fn reboot_request_exits_once_requested() {
    let (mut hsm, mut host, s) = machine("REBOOT_REQ", &[], "ENTER");
    step(&mut hsm, &mut host, 4);
    assert!(hsm.is_running(s));
    host.answers.insert("RebootRequested", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn enable_time_counts_up_and_down() {
    let (mut hsm, mut host, s) = machine("ENABLETIME", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::AdjustTimeEnabled { delta: 1 }, Request::AdjustTimeEnabled { delta: -1 }]);
}

#[test]
fn collectables_wait_for_the_world_then_reset_and_stop() {
    let (mut hsm, mut host, s) = machine("COLLECTABLES", &[], "ENTER");
    host.answers.insert("CollectablesReady", 0);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
    host.answers.insert("CollectablesReady", 1);
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::StartCollectables, Request::StopCollectables]);
}

#[test]
fn load_feedback_group_loads_waits_and_unloads_on_request() {
    let (mut hsm, mut host, s) = machine("LOAD_FDBK_GR", &[("GROUP", "7"), ("UNLOADONEXIT", "true")], "ENTER");
    host.answers.insert("FeedbackGroupLoaded", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::LoadFeedbackGroup { group: 7 }]);
    host.answers.insert("FeedbackGroupLoaded", 1);
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[1], Request::UnloadFeedbackGroup { group: 7 });
    let (mut hsm, mut host, s) = machine("LOAD_FDBK_GR", &[("GROUP", "0"), ("UNLOADONEXIT", "true")], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty(), "group 0 is never loaded or released");
}

#[test]
fn preload_cops_lists_streams_and_lets_go() {
    use super::behaviours::game::DEFAULT_COP_VEHICLE;
    let (mut hsm, mut host, s) = machine("PRELOADCOPS", &[], "ENTER");
    host.answers.insert("GameModeEvent", 5);
    host.answers.insert("PlayerVehicle", 77);
    host.answers.insert("CopsAllowed", 1);
    host.answers.insert("CopVehicleCount", 2);
    host.answers.insert("CopVehicle", 88);
    host.answers.insert("FreedriveEvent", 0);
    host.answers.insert("PlayerVehicleOf", 77);
    step(&mut hsm, &mut host, 1);
    // First the player's car in high detail, alone in its step.
    assert_eq!(host.reqs, [Request::StreamVehicle { vehicle: 77, hi: true }]);
    step(&mut hsm, &mut host, 4);
    let lo: Vec<_> = host.reqs.iter().skip(1).cloned().collect();
    assert_eq!(
        lo,
        [
            Request::StreamVehicle { vehicle: DEFAULT_COP_VEHICLE, hi: false },
            Request::StreamVehicle { vehicle: 88, hi: false },
            Request::StreamVehicle { vehicle: 77, hi: false },
        ],
        "one per update: the default cop, the table's cop, the players' cars outside free drive"
    );
    // Busy: nothing happens.
    host.reqs.clear();
    host.answers.insert("VehicleStreamBusy", 1);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty() && hsm.is_running(s));
    host.answers.insert("VehicleStreamBusy", 0);
    step(&mut hsm, &mut host, 6);
    assert_eq!(host.reqs[0], Request::ReleaseVehicle { vehicle: 77, hi: true });
    assert_eq!(host.reqs.len(), 4, "the high car, then the three low ones one per update");
    assert!(!hsm.is_running(s));
}

#[test]
fn run_layout_runs_waits_and_deactivates_twice() {
    let (mut hsm, mut host, s) = machine("HSB_RNLAYOUT", &[("LAYOUT", "100143"), ("RUN_HIGHEST", "True")], "ENTER");
    host.answers.insert("LayoutReady", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::RunLayout { layout: 100143, layer: 0x18 }]);
    host.answers.insert("LayoutReady", 1);
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert!(hsm.is_running(s), "the third exit update finishes");
    step(&mut hsm, &mut host, 2);
    assert!(!hsm.is_running(s));
    assert_eq!(
        host.reqs[1..],
        [
            Request::DeactivateLayout { layout: 100143 },
            Request::DeactivateLayout { layout: 100143 },
            Request::ReleaseLayout { layout: 100143 },
        ]
    );
}

#[test]
fn listen_for_invite_waits_offline_and_takes_a_joinable_one() {
    use super::behaviours::game::DONT_JOIN;
    let (mut hsm, mut host, s) = machine("LIST_INVITE", &[], "ENTER");
    step(&mut hsm, &mut host, 5);
    assert!(hsm.is_running(s));
    host.answers.insert("InvitePending", 1);
    host.answers.insert("InviteJoinable", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(DONT_JOIN));
    assert_eq!(host.reqs, [Request::TakeInvite]);
    let (mut hsm, mut host, s) = machine("LIST_INVITE", &[], "ENTER");
    host.answers.insert("InvitePending", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn traffic_restart_asks_and_exits_offline() {
    let (mut hsm, mut host, s) = machine("REQTRAFRSTRT", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::RestartTraffic]);
    assert_eq!(hsm.result(s), Some(EXIT));
    // Online and the time stamp not set: waits until the 6 s time-out.
    let (mut hsm, mut host, s) = machine("REQTRAFRSTRT", &[], "ENTER");
    host.answers.insert("OnlineEvent", 1);
    step(&mut hsm, &mut host, 100);
    assert!(hsm.is_running(s));
    step(&mut hsm, &mut host, 300);
    assert_eq!(hsm.result(s), Some(EXIT));
    // No traffic: no request, exit at once.
    let (mut hsm, mut host, s) = machine("REQTRAFRSTRT", &[], "ENTER");
    host.answers.insert("TrafficDensityZero", 1);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn speedwall_times_are_asked_outside_free_drive_and_waited_for() {
    let (mut hsm, mut host, s) = machine("GETSPEEDWALL", &[], "ENTER");
    host.answers.insert("NextEvent", 12);
    host.answers.insert("EventIsFreedrive", 0);
    host.answers.insert("SpeedwallIdle", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs, [Request::GetSpeedwallTimes { event: 12 }]);
    assert!(hsm.is_running(s));
    host.answers.insert("SpeedwallIdle", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    let (mut hsm, mut host, _) = machine("GETSPEEDWALL", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty(), "free drive asks nothing");
}

#[test]
fn prefetch_screen_counts_up_waits_and_down() {
    let (mut hsm, mut host, s) = machine("PREFETCHSCRE", &[("SCREENASSET", "371621")], "ENTER");
    host.answers.insert("ScreenPrefetched", 0);
    step(&mut hsm, &mut host, 3);
    assert_eq!(hsm.active(s)[0].2, Phase::Entering);
    host.answers.insert("ScreenPrefetched", 1);
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(
        host.reqs,
        [Request::PrefetchScreen { asset: 371621 }, Request::ReleasePrefetchedScreen { asset: 371621 }]
    );
}

#[test]
fn sim_pauses_offline_only() {
    let (mut hsm, mut host, s) = machine("SETSIMPAUSES", &[("PAUSED", "True")], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::SimPause { paused: true }]);
    assert_eq!(hsm.result(s), Some(EXIT));
    let (mut hsm, mut host, _) = machine("SETSIMPAUSES", &[("PAUSED", "False")], "ENTER");
    host.answers.insert("OnlineEvent", 1);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.is_empty());
    // ALLOWSIMPAUS resumes on exit; BPAUSED pauses for its lifetime.
    for (code, expect) in [
        ("ALLOWSIMPAUS", vec![Request::SimPause { paused: false }]),
        ("BPAUSED", vec![Request::SimPause { paused: true }, Request::SimPause { paused: false }]),
    ] {
        let (mut hsm, mut host, s) = machine(code, &[], "ENTER");
        step(&mut hsm, &mut host, 3);
        assert!(hsm.is_running(s), "{code} runs its children");
        hsm.request_exit(s);
        step(&mut hsm, &mut host, 2);
        assert_eq!(host.reqs, expect, "{code}");
    }
}

#[test]
fn show_world_shows_and_hides_with_the_last_one() {
    let (mut hsm, mut host, s) = machine("HSB_RUNNING", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::ShowWorld { show: true }, Request::AdjustShowWorldCount { delta: 1 }]);
    host.answers.insert("ShowWorldCount", 1);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.len(), 3, "another one still runs: not hidden");
    let (mut hsm, mut host, s) = machine("HSB_RUNNING", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::ShowWorld { show: false }));
}

#[test]
fn enable_triggers_for_the_lifetime() {
    let (mut hsm, mut host, s) = machine("ENABLETRIGS", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs, [Request::EnableTriggers(true), Request::EnableTriggers(false)]);
}

#[test]
fn disable_control_group_blocks_every_frame_and_unblocks() {
    let (mut hsm, mut host, s) = machine("HSB_DISCONGP", &[("GROUP", "CAR")], "ENTER");
    host.answers.insert("ControlGroupCount", 1);
    host.answers.insert("ControlGroup", 3);
    step(&mut hsm, &mut host, 3);
    let block = Request::SetInputBlock { group: 3, bit: 0, set: true };
    assert!(host.reqs.len() >= 2 && host.reqs.iter().all(|r| *r == block));
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs.last(), Some(&Request::SetInputBlock { group: 3, bit: 0, set: false }));
}

#[test]
fn run_hud_sub_network_is_run_sub_network() {
    let r = Registry::structural();
    let s = state(1, "RUNHUDSUBNET", Fallback::Exit);
    assert!(r.contains(s.behaviour));
}

#[test]
fn the_game_mode_manager_starts_the_mode_machine() {
    use crate::natives::LogicHost;
    use crate::natives::game_modes::EventInfo;
    use crate::natives::mode_machine::{self, ModeInfo};
    let mut hsm = Hsm::new(data(vec![state(1, "HSB_RUNCHILD", Fallback::RunChildren)], vec![]), Registry::structural());
    let mut host = LogicHost::default();
    {
        let mut st = host.state.borrow_mut();
        st.game.pending_event = 390703;
        st.config.freedrive_mode = 390145;
        st.game_modes.events = vec![EventInfo { id: 390703, mode: 390145, ..Default::default() }];
        st.game_modes.modes = vec![ModeInfo { id: 390145, hsm: 1, weapon_list: 457359, ..Default::default() }];
    }
    assert_eq!(host.query(&Query::NextEvent), Some(390703));
    host.request(Request::StartGameMode { event: 390703 });
    let state = host.state.clone();
    mode_machine::update(&state, &mut hsm, &mut host);
    assert_eq!(host.query(&Query::GameModeStarted), Some(0), "the event data is not loaded yet");
    state.borrow_mut().game_modes.event_data_ready = true;
    mode_machine::update(&state, &mut hsm, &mut host);
    assert_eq!(state.borrow().game_modes.weapon_list, 457359);
    assert!(hsm.machines().next().is_none(), "waits for the weapon list");
    state.borrow_mut().game_modes.weapon_list_loaded = 457359;
    mode_machine::update(&state, &mut hsm, &mut host);
    let serial = state.borrow().game_modes.machine.expect("started");
    assert!(hsm.is_running(serial));
    assert_eq!(host.query(&Query::GameModeMachine), Some(u64::from(serial)));
    assert_eq!(host.query(&Query::FreedriveEvent), Some(1));
    assert_eq!(host.query(&Query::GameModeEvent), Some(390703));
}

fn reqs_of(host: &Rec) -> Vec<&Request> {
    host.reqs.iter().collect()
}

#[test]
fn set_end_reason_follows_the_entry_point() {
    for (entry, reason) in [("ENDED", 1), ("RETRY", 3), ("OTHER", 2)] {
        let (mut hsm, mut host, s) = machine("SETENDREASON", &[], entry);
        step(&mut hsm, &mut host, 2);
        assert_eq!(hsm.result(s), Some(EXIT));
        assert!(host.reqs.contains(&Request::SetEndReason { reason }));
        assert!(host.reqs.contains(&Request::SetGameModeWon { won: false }));
    }
    // ENDED with the mode won keeps "won".
    let (mut hsm, mut host, _) = machine("SETENDREASON", &[], "ENDED");
    host.answers.insert("ModeWon", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(reqs_of(&host), [&Request::SetEndReason { reason: 1 }]);
}

#[test]
fn mode_end_check_exits_offline_and_times_out() {
    let (mut hsm, mut host, s) = machine("MODEENDCHECK", &[], "ENTER");
    host.answers.insert("ModeWon", 1);
    host.answers.insert("EventEndTrigger", 0);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s), "won but no end trigger yet");
    host.answers.insert("EventEndTrigger", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert!(host.reqs.contains(&Request::SetGameModeWon { won: true }));
    // An end reason set holds it; a passed display timeout exits through TIME_OUT.
    let (mut hsm, mut host, s) = machine("MODEENDCHECK", &[], "ENTER");
    host.answers.insert("EndReason", 2);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    host.answers.insert("EndReason", 0);
    host.answers.insert("DisplayTimeoutOn", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("TIME_OUT")));
    // Without authority nothing happens.
    let (mut hsm, mut host, s) = machine("MODEENDCHECK", &[], "ENTER");
    host.answers.insert("ModeAuthority", 0);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
}

#[test]
fn mode_end_check_online_waits_for_the_flag() {
    let (mut hsm, mut host, s) = machine("MODEENDCHECK", &[], "ENTER");
    host.answers.insert("OnlineEvent", 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    assert!(host.reqs.contains(&Request::SendModeEndMessage));
    host.answers.insert("ModeEndFlag", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn wait_for_input_waits_for_release_then_takes_a_handled_exit() {
    let (mut hsm, mut host, s) = machine("HSB_WAIT4IN", &[], "ENTER");
    host.answers.insert("InputHeld", 1);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    host.answers.insert("InputHeld", 0);
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    // The root has no parent that handles the candidate: it keeps waiting.
    host.answers.insert("InputExit", key("ACCEPT"));
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
}

#[test]
fn layer_set_sets_forces_and_stays_off_for_zero() {
    let (mut hsm, mut host, _) = machine("SET_LAY_SET", &[("LAYOUTSET", "7"), ("FORCE_SET", "true")], "ENTER");
    host.answers.insert("LayerSet", 3);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.len() >= 2 && host.reqs.iter().all(|r| *r == Request::SetLayerSet { set: 7 }));
    // A binding wins over the parameter; 0 is not set on enter; RESTORE_OLD false keeps it.
    let (mut hsm, mut host, s) =
        machine("SET_LAY_SET", &[("LAYOUTPATH", "Mode.Layer"), ("RESTORE_OLD", "false")], "ENTER");
    host.ints.insert("Mode.Layer".into(), 0);
    step(&mut hsm, &mut host, 3);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.is_empty());
}

#[test]
fn layer_set_restores_the_old_one_on_exit() {
    let (mut hsm, mut host, s) = machine("SET_LAY_SET", &[("LAYOUTSET", "7")], "ENTER");
    host.answers.insert("LayerSet", 3);
    step(&mut hsm, &mut host, 2);
    assert_eq!(reqs_of(&host), [&Request::SetLayerSet { set: 7 }]);
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs.last(), Some(&Request::SetLayerSet { set: 3 }));
}

#[test]
fn blacklist_shutdowns_walk_their_phases_and_end_in_the_credits() {
    let (mut hsm, mut host, s) = machine("DOBLACKLISTS", &[], "ENTER");
    step(&mut hsm, &mut host, 2);
    assert_eq!(host.reqs[0], Request::SetCreditsTransition { on: false });
    // Phase 1's step runs; scripted, it moves on to phase 2 (answer = 1 + phase).
    host.answers.insert("BlacklistPhaseAfter", 3);
    step(&mut hsm, &mut host, 1);
    assert!(host.reqs.contains(&Request::BlacklistStep { phase: 1 }));
    host.answers.insert("BlacklistPhaseAfter", 0);
    host.answers.insert("BlacklistReady", 1);
    step(&mut hsm, &mut host, 2);
    assert!(hsm.is_running(s));
    // The credits tail: the sequence runs with its trigger unset, then it is over.
    host.answers.insert("BlacklistCreditsPending", 1);
    host.answers.insert("CreditsSequenceActive", 1);
    step(&mut hsm, &mut host, 1);
    assert!(hsm.is_running(s));
    host.answers.insert("CreditsSequenceActive", 0);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(key("CREDITS")));
    assert!(host.reqs.contains(&Request::GameCreditsFinished));
    assert!(host.reqs.contains(&Request::SetCreditsTransition { on: true }));
}

#[test]
fn blacklist_rival_is_released_and_removed_on_exit() {
    let (mut hsm, mut host, s) = machine("DOBLACKLISTS", &[], "ENTER");
    host.answers.insert("BlacklistSlot", 1);
    host.answers.insert("BlacklistRivalAlive", 1);
    host.answers.insert("BlacklistRivalHandle", 9);
    step(&mut hsm, &mut host, 2);
    host.answers.insert("BlacklistPhaseAfter", 3);
    step(&mut hsm, &mut host, 1);
    host.answers.insert("BlacklistPhaseAfter", 0);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.contains(&Request::BlacklistRelease { handle: 9 }));
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.contains(&Request::BlacklistRemoveSlot));
}

#[test]
fn checkpoints_wait_for_load_step_and_exit_when_finished() {
    // None: exits at once.
    let (mut hsm, mut host, s) = machine("CHECKPTS", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert_eq!(host.reqs.first(), Some(&Request::CheckpointsEnter));
    assert!(host.reqs.contains(&Request::CheckpointsLeave));
    // Some: waits for the load, places them, steps until finished.
    let (mut hsm, mut host, s) = machine("CHECKPTS", &[], "ENTER");
    host.answers.insert("CheckpointCount", 5);
    host.answers.insert("CheckpointsLoaded", 0);
    host.answers.insert("CheckpointsFinished", 0);
    step(&mut hsm, &mut host, 3);
    assert!(!host.reqs.contains(&Request::CheckpointsPlace));
    host.answers.insert("CheckpointsLoaded", 1);
    step(&mut hsm, &mut host, 3);
    assert!(host.reqs.contains(&Request::CheckpointsPlace));
    assert!(host.reqs.contains(&Request::CheckpointsStep));
    assert!(hsm.is_running(s));
    host.answers.insert("CheckpointsFinished", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
}

#[test]
fn event_starts_exit_on_entry_when_asked() {
    let (mut hsm, mut host, s) = machine("EVENTSTARTS", &[("CHECKENTRY", "true")], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert!(hsm.is_running(s));
    assert_eq!(host.reqs[0], Request::EventStartsEnter { check_entry: true });
    host.answers.insert("EventStartTriggered", 1);
    step(&mut hsm, &mut host, 2);
    assert_eq!(hsm.result(s), Some(EXIT));
    assert_eq!(host.reqs.last(), Some(&Request::EventStartsLeave));
    // Without CHECKENTRY it never exits by itself.
    let (mut hsm, mut host, s) = machine("EVENTSTARTS", &[], "ENTER");
    host.answers.insert("EventStartTriggered", 1);
    step(&mut hsm, &mut host, 5);
    assert!(hsm.is_running(s));
}

#[test]
fn car_swap_logic_steps_once_started_and_never_exits() {
    let (mut hsm, mut host, s) = machine("HSMCARSWAPLO", &[], "ENTER");
    step(&mut hsm, &mut host, 3);
    assert_eq!(reqs_of(&host), [&Request::CarSwapEnter]);
    host.answers.insert("CarSwapActive", 1);
    step(&mut hsm, &mut host, 2);
    assert!(host.reqs.contains(&Request::CarSwapStep));
    assert!(hsm.is_running(s));
    hsm.request_exit(s);
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.reqs.last(), Some(&Request::CarSwapExit));
}
