//! The scoring behaviours (`behaviours_scoring`) in a machine on the logic's own host: the slots reach
//! `natives::drifts` / `natives::smash_grab`, the queries read their outcome, the roots answer bindings.

use mw_formats::hsm::{Action, ChildExitPoint, ChildExits, EntryPoint, Fallback, Spawn, key};

use super::tests::{DT, data, state};
use super::*;
use crate::natives::players::Player;
use crate::natives::smash_grab::KIND_ITEM;
use crate::natives::{LogicHost, Request};

/// A parent running one child of behaviour `code`, leaving through `exits`, on a [`LogicHost`] with a local player.
fn wrapped(code: &str, exits: &[&str]) -> (Hsm, LogicHost, u32) {
    let mut child = state(2, code, Fallback::RunChildren);
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
    let mut host = LogicHost::default();
    {
        let mut st = host.state.borrow_mut();
        st.players.local = Some(0);
        st.players.slots[0] = Some(Player { vehicle: 1, vehicle_alive: true, ..Default::default() });
        st.game_modes.timer_running = true;
        st.game_modes.won = true;
    }
    let s = hsm.start(1, &mut host).unwrap();
    (hsm, host, s)
}

fn step(hsm: &mut Hsm, host: &mut LogicHost, n: usize) {
    for _ in 0..n {
        hsm.update(host, DT);
    }
}

#[test]
fn drift_scoring_runs_until_its_parent_ends() {
    let (mut hsm, mut host, s) = wrapped("HSMDODRIFTS", &[]);
    host.state.borrow_mut().drift.inputs.loading = true;
    step(&mut hsm, &mut host, 3);
    assert_eq!(host.state.borrow().drift.state.phase, 1, "the enter update waits for the loads");
    host.state.borrow_mut().drift.inputs.loading = false;
    step(&mut hsm, &mut host, 3);
    let st = host.state.borrow();
    assert_eq!(st.drift.state.phase, 4);
    assert_eq!(st.resolve_bool("DriftScoring.ScoringActive"), Some(true));
    assert!(st.requests.contains(&Request::DriftScoring(crate::natives::drifts::Slot::Update)));
    assert_eq!(hsm.result(s), None, "the update never exits by itself");
}

#[test]
fn smash_and_grab_exits_win_or_lose() {
    for (score_to_reach, exit) in [(50, "WIN"), (1000, "LOSE")] {
        let (mut hsm, mut host, s) = wrapped("DOSMSHNGRB", &["WIN", "LOSE"]);
        // The machine is entered before the inputs are set: the enter read the defaults; set them and re-enter.
        {
            let mut st = host.state.borrow_mut();
            let i = &mut st.smash_grab.inputs;
            i.event_triggers = vec![(10, KIND_ITEM)];
            i.time_limit = 60.0;
            i.medals = [score_to_reach, score_to_reach, score_to_reach];
        }
        host.request(Request::SmashGrabScoring(crate::natives::drifts::Slot::Enter));
        step(&mut hsm, &mut host, 3);
        assert_eq!(host.state.borrow().smash_grab.state.items, [10]);
        host.state.borrow_mut().smash_grab.inputs.contacts = vec![10];
        step(&mut hsm, &mut host, 6);
        assert_eq!(hsm.result(s), Some(key(exit)), "{exit}");
        let st = host.state.borrow();
        assert!(st.requests.iter().any(|r| matches!(r, Request::SpeedPoints { sp: 50, .. })));
        assert!(!st.smash_grab.state.active);
    }
}
