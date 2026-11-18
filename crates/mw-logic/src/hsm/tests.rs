//! Lifecycle rules of the interpreter on small hand-built machines.

use std::collections::HashMap;

use mw_formats::hsm::{
    ChildExitPoint, ChildExits, DataDefinition, EntryPoint, ParamType, Parameter, Spawn, State, TopState,
};

use super::behaviours::{FALSE, TRUE};
use super::*;

fn k(s: &str) -> Key {
    key(s)
}

pub(super) fn state(id: u32, code: &str, fallback: Fallback) -> State {
    State {
        behaviour: behaviour_id(code),
        id,
        name: format!("S{id}"),
        fallback,
        entry_points: Vec::new(),
        children: Vec::new(),
        parameters: Vec::new(),
        data_definition: None,
    }
}

pub(super) fn param(name: &str, value: &str) -> Parameter {
    Parameter { id: k(name), kind: ParamType::String, values: vec![value.into()], structures: Vec::new() }
}

/// `ENTER` → start these children.
fn enter(s: &mut State, children: &[u32]) {
    s.entry_points.push(EntryPoint {
        id: k("ENTER"),
        action: Action::Child,
        spawns: children.iter().map(|&c| Spawn { entry_point: k("ENTER"), state: c }).collect(),
    });
}

/// Child `child` exiting through `exit` → `action` (spawns / parent exit point).
fn on(s: &mut State, child: u32, exit: &str, action: Action, spawns: &[u32], parent_exit: &str) {
    let point = ChildExitPoint {
        parent_exit: k(parent_exit),
        child_exit: k(exit),
        action,
        spawns: spawns.iter().map(|&c| Spawn { entry_point: k("ENTER"), state: c }).collect(),
    };
    match s.children.iter_mut().find(|c| c.state == child) {
        Some(c) => c.exits.push(point),
        None => s.children.push(ChildExits { state: child, exits: vec![point] }),
    }
}

pub(super) fn data(states: Vec<State>, defs: Vec<DataDefinition>) -> Arc<HsmData> {
    let root = states[0].id;
    Arc::new(HsmData::from_parts(
        vec![TopState { definition: root, behaviour: states[0].behaviour, instance: 1 }],
        states,
        defs,
        Vec::new(),
    ))
}

#[derive(Default)]
struct TestHost {
    log: Vec<String>,
    bools: HashMap<String, bool>,
    strings: HashMap<String, String>,
    hooks: HashMap<String, Key>,
    hook_args: Vec<Option<Json>>,
    published: Vec<String>,
}

impl Host for TestHost {
    fn binding_bool(&mut self, path: &str) -> Option<bool> {
        self.bools.get(path).copied()
    }
    fn binding_string(&mut self, path: &str) -> Option<String> {
        self.strings.get(path).cloned()
    }
    fn hook(&mut self, name: &str, args: Option<&Json>) -> Option<Key> {
        self.hook_args.push(args.cloned());
        self.hooks.get(name).copied()
    }
    fn publish(&mut self, path: &str, _data: &Json) {
        self.published.push(format!("+{path}"));
    }
    fn retract(&mut self, path: &str) {
        self.published.push(format!("-{path}"));
    }
    fn trace(&mut self, e: &Trace) {
        self.log.push(match e {
            Trace::Enter { state, .. } => format!("enter {state}"),
            Trace::Exit { state, exit, .. } => format!("exit {state} {}", key_name(*exit)),
            Trace::Finish { exit, .. } => format!("finish {}", key_name(*exit)),
            Trace::Missing { state, .. } => format!("missing {state}"),
            Trace::Hook { name, result, .. } => format!("hook {name} {}", key_name(*result)),
            Trace::Start { .. } => "start".into(),
        });
    }
}

pub(super) const DT: Clock = Clock { frame: 1.0 / 30.0, game: Some(1.0 / 30.0) };

#[test]
fn instant_chain_runs_in_one_frame_and_exit_points_exit_the_parent() {
    // Root runs 2; 2 (placeholder) exits through EXIT → start 3; 3 exits → the root exits through DONE.
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "EXIT", Action::Child, &[3], "");
    on(&mut root, 3, "EXIT", Action::Exit, &[], "DONE");
    let d = data(vec![root, state(2, "HSB_PLCHLDR", Fallback::Exit), state(3, "HSB_PLCHLDR", Fallback::Exit)], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert!(!hsm.is_running(serial));
    assert_eq!(hsm.result(serial), Some(k("DONE")));
    assert_eq!(
        host.log,
        ["start", "enter 1", "enter 2", "exit 2 EXIT", "enter 3", "exit 3 EXIT", "exit 1 DONE", "finish DONE"]
    );
}

#[test]
fn children_wait_while_the_parent_enters() {
    // BDELAY with ENTERTIME 0.1 s: its ENTER spawns fire at once but the child is not ticked until it runs.
    let mut root = state(1, "BDELAY", Fallback::RunChildren);
    root.parameters = vec![param("ENTERTIME", "0.1"), param("UPDATETIME", "100"), param("LEAVETIME", "0")];
    enter(&mut root, &[2]);
    on(&mut root, 2, "EXIT", Action::Nothing, &[], "");
    let d = data(vec![root, state(2, "HSB_PLCHLDR", Fallback::Exit)], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    hsm.update(&mut host, DT);
    assert_eq!(host.log, ["start", "enter 1"]);
    assert_eq!(hsm.active(serial).len(), 2);
    hsm.update(&mut host, DT); // 3 frames ≥ 0.1 s: running, the child runs and exits
    assert_eq!(host.log, ["start", "enter 1", "enter 2", "exit 2 EXIT"]);
    assert_eq!(hsm.active(serial).len(), 1);
}

#[test]
fn an_exit_waits_for_the_subtree() {
    // Root runs a long BDELAY (LEAVETIME 0.05 s) and a placeholder whose EXIT exits the root: the delay gets the
    // exit request and the root finishes only after the delay's leave time.
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2, 3]);
    on(&mut root, 3, "EXIT", Action::Exit, &[], "EXIT");
    let mut delay = state(2, "BDELAY", Fallback::RunChildren);
    delay.parameters = vec![param("ENTERTIME", "0"), param("UPDATETIME", "100"), param("LEAVETIME", "0.05")];
    let d = data(vec![root, delay, state(3, "HSB_PLCHLDR", Fallback::Exit)], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert!(hsm.is_running(serial));
    assert_eq!(
        hsm.active(serial).iter().map(|a| (a.1, a.2)).collect::<Vec<_>>(),
        [(1, Phase::Exiting), (2, Phase::Exiting)]
    );
    hsm.update(&mut host, DT); // 2 × 1/30 ≥ 0.05
    assert!(!hsm.is_running(serial));
    assert_eq!(hsm.result(serial), Some(EXIT));
    assert_eq!(host.log.last().unwrap(), "finish EXIT");
}

#[test]
fn unregistered_behaviours_use_the_fallback() {
    // 2: unknown code, RUN_CHILDREN → runs its child 3 (unknown, EXIT → placeholder) which exits.
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    let mut two = state(2, "NOT_A_BEHAVIOUR", Fallback::RunChildren);
    enter(&mut two, &[3]);
    on(&mut two, 3, "EXIT", Action::Exit, &[], "OUT");
    on(&mut root, 2, "OUT", Action::Exit, &[], "EXIT");
    let d = data(vec![root, two, state(3, "ALSO_UNKNOWN", Fallback::Exit)], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert_eq!(hsm.result(serial), Some(EXIT));
    assert!(host.log.contains(&"exit 2 OUT".to_owned()));
}

#[test]
fn hooks_get_the_data_definition_and_choose_the_exit() {
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "TRUE", Action::Child, &[3], "");
    on(&mut root, 3, "YES", Action::Exit, &[], "EXIT");
    let mut call = state(2, "CALLHSMHOOK", Fallback::Exit);
    call.parameters = vec![param("FUNCTION", "IsNewGame")];
    call.data_definition = Some(0);
    let mut poll = state(3, "HSMHOOKEXIT", Fallback::Exit);
    poll.parameters = vec![param("FUNCTION", "Ready")];
    let defs = vec![DataDefinition {
        id: 9,
        path: String::new(),
        name: String::new(),
        json: "{ \"Params\": { \"NewGame\": true } }".into(),
    }];
    let d = data(vec![root, call, poll], defs);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    host.hooks.insert("IsNewGame".into(), TRUE);
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    hsm.update(&mut host, DT);
    assert!(hsm.is_running(serial)); // "Ready" does not exist yet: HSMHOOKEXIT keeps polling
    assert_eq!(host.hook_args[0], Some(Json::Object(vec![("NewGame".into(), Json::Bool(true))])));
    host.hooks.insert("Ready".into(), k("YES"));
    hsm.update(&mut host, DT);
    assert_eq!(hsm.result(serial), Some(EXIT));
    assert_eq!(host.log.iter().filter(|l| l.starts_with("hook Ready")).count(), 3);
}

#[test]
fn bindings_select_exits_and_end_waits() {
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "TOGAME", Action::Child, &[3], "");
    on(&mut root, 3, "FALSE", Action::Child, &[4], "");
    on(&mut root, 4, "EXIT", Action::Exit, &[], "EXIT");
    let mut select = state(2, "HSB_SEL_EX_B", Fallback::Exit);
    select.parameters =
        vec![Parameter { id: behaviours::BINDING_PATH, ..param("", "Flow.Next") }, param("ONFAILEXIT", "")];
    let mut is_true = state(3, "HSM_B_TRUE", Fallback::Exit);
    is_true.parameters = vec![Parameter { id: behaviours::BINDING_PATH, ..param("", "Mode.Unset") }];
    let mut wait = state(4, "EXITBINDTRUE", Fallback::Exit);
    wait.parameters = vec![Parameter { id: behaviours::BINDING_PATH, ..param("", "Mode.Ready") }];
    let d = data(vec![root, select, is_true, wait], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    host.strings.insert("Flow.Next".into(), "TOGAME".into());
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    hsm.update(&mut host, DT);
    assert!(hsm.is_running(serial));
    assert_eq!(host.log.iter().filter(|l| l.starts_with("exit 3")).collect::<Vec<_>>(), ["exit 3 FALSE"]);
    host.bools.insert("Mode.Ready".into(), true);
    hsm.update(&mut host, DT);
    assert_eq!(hsm.result(serial), Some(EXIT));
    let _ = FALSE;
}

#[test]
fn missing_handling_is_reported_and_the_parent_keeps_running() {
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    let d = data(vec![root, state(2, "HSB_PLCHLDR", Fallback::Exit)], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert!(hsm.is_running(serial));
    assert!(host.log.contains(&"missing 1".to_owned()));
}

#[test]
fn view_models_are_published_while_active() {
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "EXIT", Action::Exit, &[], "EXIT");
    let mut wait = state(2, "WAIT4TIME", Fallback::Exit);
    wait.parameters = vec![param("TIME", "0.05")];
    wait.data_definition = Some(0);
    let defs = vec![DataDefinition {
        id: 9,
        path: "SafeForDialogs".into(),
        name: String::new(),
        json: "{ \"CanShow\": true }".into(),
    }];
    let d = data(vec![root, wait], defs);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert_eq!(host.published, ["+SafeForDialogs"]);
    hsm.update(&mut host, DT);
    assert_eq!(host.published, ["+SafeForDialogs", "-SafeForDialogs"]);
}

#[test]
fn wait_for_time_counts_ticks() {
    // 0.1 s at 1/30 s per frame: 300000 ticks less 3 × 100000 → 0 after the third update.
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "EXIT", Action::Exit, &[], "EXIT");
    let mut wait = state(2, "WAIT4TIME", Fallback::Exit);
    wait.parameters = vec![param("TIME", "0.1")];
    let d = data(vec![root, wait], vec![]);
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    let mut frames = 0;
    while hsm.is_running(serial) && frames < 10 {
        hsm.update(&mut host, Clock { frame: 0.1 / 3.0, game: None });
        frames += 1;
    }
    assert!((3..=4).contains(&frames), "{frames}");
}

#[test]
fn sub_networks_run_as_their_own_machine() {
    // Root runs RUNSUBNET of instance 2 (state 10); 10's child exits it through DONE, which the root handles.
    let mut root = state(1, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut root, &[2]);
    on(&mut root, 2, "DONE", Action::Exit, &[], "EXIT");
    let mut sub = state(2, "RUNSUBNET", Fallback::Exit);
    sub.parameters = vec![param("BINDING", ""), param("TOPSTATE", "2")];
    let mut net = state(10, "HSB_RUNCHILD", Fallback::RunChildren);
    enter(&mut net, &[11]);
    on(&mut net, 11, "EXIT", Action::Exit, &[], "DONE");
    let states = vec![root, sub, net, state(11, "HSB_PLCHLDR", Fallback::Exit)];
    let d = Arc::new(HsmData::from_parts(
        vec![
            TopState { definition: 1, behaviour: behaviour_id("HSB_RUNCHILD"), instance: 1 },
            TopState { definition: 10, behaviour: behaviour_id("HSB_RUNCHILD"), instance: 2 },
        ],
        states,
        vec![],
        vec![],
    ));
    let mut hsm = Hsm::new(d, Registry::structural());
    let mut host = TestHost::default();
    let serial = hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT); // the sub-network starts in slot 1 and runs to its end in the same frame
    hsm.update(&mut host, DT); // RUNSUBNET sees it finished through DONE
    assert_eq!(hsm.result(serial), Some(EXIT));
    assert!(host.log.contains(&"exit 2 DONE".to_owned()));
}

#[test]
fn integer_parameters_read_like_atoi() {
    // `param_int` is `atoi`: digits only, so "1e3" is 1 and "12.9" is 12 (not `atof`'s 1000).
    struct ReadN;
    impl Behaviour for ReadN {
        fn on_enter(&mut self, cx: &mut Cx) {
            for n in ["A", "B", "C", "D"] {
                let v = behaviours::param_int(cx, n);
                cx.host.publish(&format!("{n}={v}"), &Json::Null);
            }
        }
    }
    let mut root = state(1, "READN", Fallback::Exit);
    root.parameters = vec![param("A", "1e3"), param("B", " -12.9x"), param("C", "730003"), param("D", "")];
    let mut reg = Registry::structural();
    reg.register_fn("READN", |_| ReadN);
    let mut hsm = Hsm::new(data(vec![root], vec![]), reg);
    let mut host = TestHost::default();
    hsm.start(1, &mut host).unwrap();
    hsm.update(&mut host, DT);
    assert_eq!(host.published, ["+A=1", "+B=-12", "+C=730003", "+D=0"]);
}
