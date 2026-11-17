//! `mwtool hsm`: the game logic's state machines (`mw_formats::hsm`): listing, trees, search, and a headless
//! run of the interpreter (`mw_logic::hsm`) on the logic host (`mw_logic::natives::LogicHost`: ported hooks,
//! published view models, `Signals`), with stand-ins for unported hooks / behaviours and `--answer` overrides.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, bail};
use mw_assets::AssetStore;
use mw_formats::hsm::{Action, HsmData, ParamType, Parameter, State, behaviour_id, key, key_name};
use mw_formats::json::Json;
use mw_logic::hsm::{Clock, Host, Hsm, Key, ORIGINAL_BEHAVIOURS, Registry, Trace, behaviours::Stub, original_code};
use mw_logic::natives::LogicHost;

pub struct Options<'a> {
    pub bundle: &'a str,
    pub node: Option<u32>,
    pub find: Option<&'a str>,
    pub depth: usize,
    pub parents: bool,
    pub data: bool,
    pub run: Option<u32>,
    pub steps: usize,
    pub dt: f32,
    pub answers: &'a [String],
}

/// Parses every `HSMData` of the install (`hsm --bundle all`) and prints its counts.
fn check_all(store: &AssetStore) -> Result<()> {
    let (mut ok, mut failed) = (0, 0);
    for path in store.bundle_paths() {
        let Some(b) = store.bundle(path) else { continue };
        for e in b.entries_of_type(mw_bundle::ResourceType::HSMData) {
            let res = b.load(e)?;
            match HsmData::parse(&res) {
                Ok(d) => {
                    ok += 1;
                    let spawns: usize = d.states.iter().map(|s| s.entry_points.len() + s.children.len()).sum();
                    let unknown = d
                        .states
                        .iter()
                        .flat_map(|s| s.entry_points.iter().flat_map(|e| &e.spawns))
                        .filter(|sp| d.state(sp.state).is_none())
                        .count();
                    println!(
                        "{} {}: v{} {} states, {} top, {} data defs, {} interfaces, {spawns} entry/child records, {unknown} spawns of states elsewhere",
                        path.display(),
                        e.id,
                        d.version,
                        d.states.len(),
                        d.top_states.len(),
                        d.data_definitions.len(),
                        d.behaviours.len()
                    );
                }
                Err(err) => {
                    failed += 1;
                    println!("{} {}: {err}", path.display(), e.id);
                }
            }
        }
    }
    println!("{ok} parsed, {failed} failed");
    Ok(())
}

pub fn run(store: &AssetStore, o: Options) -> Result<()> {
    if o.bundle == "all" {
        return check_all(store);
    }
    let data = store.hsm_data(o.bundle)?;
    if let Some(instance) = o.run {
        return drive(Arc::new(data), instance, o.steps, o.dt, o.answers);
    }
    if o.data {
        for d in &data.data_definitions {
            println!("== data definition {} path {:?}\n{}", d.id, d.path, d.json.trim_end());
        }
        return Ok(());
    }
    if let (true, Some(id)) = (o.parents, o.node) {
        // Who starts this state: a parent's entry point, or a child exit point of its parent.
        for s in &data.states {
            for e in s.entry_points.iter().filter(|e| e.spawns.iter().any(|sp| sp.state == id)) {
                println!("{:8} {:?}: entry point {} starts it", s.id, s.name, key_name(e.id));
            }
            for c in &s.children {
                for x in c.exits.iter().filter(|x| x.spawns.iter().any(|sp| sp.state == id)) {
                    println!("{:8} {:?}: child {} on {} starts it", s.id, s.name, c.state, key_name(x.child_exit));
                }
            }
        }
        return Ok(());
    }
    if let Some(text) = o.find {
        for s in data.states.iter().filter(|s| s.name.to_lowercase().contains(&text.to_lowercase())) {
            println!("{:8} {} {}", s.id, code(&data, s.behaviour), s.name);
        }
        return Ok(());
    }
    let Some(root) = o.node else {
        let children: HashSet<u32> = data.states.iter().flat_map(|s| s.children.iter().map(|c| c.state)).collect();
        println!(
            "version {}, {} states, {} top-level states, {} data definitions, {} behaviour interfaces",
            data.version,
            data.states.len(),
            data.top_states.len(),
            data.data_definitions.len(),
            data.behaviours.len()
        );
        println!("top-level states (instance → state):");
        for t in &data.top_states {
            let name = data.state(t.definition).map_or("?", |s| s.name.as_str());
            println!("{:8} → {:8} {} {name}", t.instance, t.definition, code(&data, t.behaviour));
        }
        println!("other roots with children:");
        let tops: HashSet<u32> = data.top_states.iter().map(|t| t.definition).collect();
        for s in data.states.iter().filter(|s| !children.contains(&s.id) && !tops.contains(&s.id)) {
            if !s.children.is_empty() {
                println!("{:8} {} ({} children)", s.id, s.name, s.children.len());
            }
        }
        return Ok(());
    };
    show(&data, root, 0, o.depth);
    Ok(())
}

/// A behaviour id as its code name (from the interface table or the original's registry), or hex.
fn code(data: &HsmData, behaviour: u32) -> String {
    original_code(behaviour)
        .map(str::to_owned)
        .or_else(|| data.behaviour_code(behaviour))
        .unwrap_or_else(|| format!("{behaviour:08x}"))
}

fn params(ps: &[Parameter]) -> String {
    ps.iter()
        .map(|p| match p.kind {
            ParamType::Structure => format!(
                "{}={{{}}}",
                key_name(p.id),
                p.structures.iter().map(|s| params(s)).collect::<Vec<_>>().join("}{")
            ),
            _ if p.values.len() == 1 => format!("{}={:?}", key_name(p.id), p.values[0]),
            _ => format!("{}={:?}", key_name(p.id), p.values),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn action(data: &HsmData, a: Action, spawns: &[mw_formats::hsm::Spawn], parent_exit: Option<u64>) -> String {
    match a {
        Action::Nothing => "nothing".into(),
        Action::Exit => format!("exit through {}", parent_exit.map_or("?".into(), key_name)),
        Action::Child => spawns
            .iter()
            .map(|s| {
                let entry = if s.entry_point == key("ENTER") {
                    String::new()
                } else {
                    format!(" via {}", key_name(s.entry_point))
                };
                format!("{} {:?}{entry}", s.state, data.state(s.state).map_or("?", |n| n.name.as_str()))
            })
            .collect::<Vec<_>>()
            .join(", "),
        Action::Other(v) => format!("action {v}"),
    }
}

fn show(data: &HsmData, id: u32, depth: usize, max: usize) {
    let pad = "  ".repeat(depth);
    let Some(s) = data.state(id) else {
        println!("{pad}? {id}");
        return;
    };
    let dd = data.data_definition(s).map_or(String::new(), |d| {
        let text: String = d.json.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = if text.len() > 80 { format!("{}…", &text[..text.floor_char_boundary(80)]) } else { text };
        if d.path.is_empty() { format!(" data {text}") } else { format!(" data {}={text}", d.path) }
    });
    println!(
        "{pad}{} {} {:?} {}{}{}",
        s.id,
        code(data, s.behaviour),
        s.name,
        params(&s.parameters),
        if s.fallback == mw_formats::hsm::Fallback::RunChildren {
            String::new()
        } else {
            format!(" fallback {:?}", s.fallback)
        },
        dd
    );
    for e in &s.entry_points {
        if e.id != key("ENTER") || e.action != Action::Child {
            println!("{pad}  entry {}: {}", key_name(e.id), action(data, e.action, &e.spawns, None));
        } else if !e.spawns.is_empty() {
            println!("{pad}  starts {}", action(data, e.action, &e.spawns, None));
        }
    }
    if depth >= max {
        return;
    }
    for c in &s.children {
        show(data, c.state, depth + 1, max);
        for x in &c.exits {
            println!(
                "{pad}    on {} -> {}",
                key_name(x.child_exit),
                action(data, x.action, &x.spawns, Some(x.parent_exit))
            );
        }
    }
}

/// The logic host plus the driver's stand-ins: answers by state id, behaviour code, hook name or binding path
/// override it; a hook the original has but that is not ported answers EXIT (most CALLHSMHOOK nodes go on on
/// EXIT).
struct Driver {
    data: Arc<HsmData>,
    logic: LogicHost,
    hooks: HashMap<String, Key>,
    bindings: HashMap<String, String>,
    frame: usize,
    /// Unported hooks the driver answered EXIT.
    unported_hooks: HashMap<String, usize>,
    /// Hook names the original does not have (EXITINVALID, as in the original).
    unknown_hooks: HashMap<String, usize>,
}

impl Host for Driver {
    fn binding_bool(&mut self, path: &str) -> Option<bool> {
        match self.bindings.get(path) {
            Some(v) => Some(matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes")),
            None => self.logic.binding_bool(path),
        }
    }
    fn binding_string(&mut self, path: &str) -> Option<String> {
        match self.bindings.get(path) {
            Some(v) => Some(v.clone()),
            None => self.logic.binding_string(path),
        }
    }
    fn binding_int(&mut self, path: &str) -> Option<i64> {
        match self.bindings.get(path) {
            Some(v) => v.parse().ok(),
            None => self.logic.binding_int(path),
        }
    }
    fn hook(&mut self, name: &str, args: Option<&Json>) -> Option<Key> {
        if let Some(&k) = self.hooks.get(name) {
            return Some(k);
        }
        match self.logic.registry.hook(name).map(|n| n.ported()) {
            Some(true) => self.logic.hook(name, args),
            Some(false) => {
                *self.unported_hooks.entry(name.to_owned()).or_default() += 1;
                Some(key("EXIT"))
            }
            None => {
                *self.unknown_hooks.entry(name.to_owned()).or_default() += 1;
                None
            }
        }
    }
    fn loading_screen_showing(&mut self) -> bool {
        match self.bindings.get("Host.LoadingScreenShowing") {
            Some(_) => self.binding_bool("Host.LoadingScreenShowing").unwrap_or(false),
            None => self.logic.loading_screen_showing(),
        }
    }
    fn graphics_prepared(&mut self) -> bool {
        match self.bindings.get("Host.GraphicsPrepared") {
            Some(_) => self.binding_bool("Host.GraphicsPrepared").unwrap_or(true),
            None => self.logic.graphics_prepared(),
        }
    }
    fn publish(&mut self, path: &str, data: &Json) {
        println!("{:5}      publish {path} ({} members)", self.frame, data.entries().len());
        self.logic.publish(path, data);
    }
    fn retract(&mut self, path: &str) {
        println!("{:5}      retract {path}", self.frame);
        self.logic.retract(path);
    }
    fn trace(&mut self, e: &Trace) {
        let f = self.frame;
        match e {
            Trace::Start { serial, state } => println!("{f:5} machine {serial} starts at {state}"),
            Trace::Enter { depth, state, name, entry, behaviour } => {
                let via = if *entry == key("ENTER") { String::new() } else { format!(" via {}", key_name(*entry)) };
                let params = self.data.state(*state).map_or(String::new(), |s| params(&s.parameters));
                println!("{f:5} {}+ {state} {name:?} [{behaviour}]{via} {params}", "  ".repeat(*depth));
            }
            Trace::Exit { depth, state, name, exit } => {
                println!("{f:5} {}- {state} {name:?} → {}", "  ".repeat(*depth), key_name(*exit))
            }
            Trace::Hook { state, name, result } => {
                println!("{f:5}      hook {name} ({state}) → {}", key_name(*result))
            }
            Trace::Finish { serial, exit } => println!("{f:5} machine {serial} finished → {}", key_name(*exit)),
            Trace::Missing { state, what } => println!("{f:5}      !! {state}: {what}"),
        }
    }
}

fn drive(data: Arc<HsmData>, instance: u32, steps: usize, dt: f32, answers: &[String]) -> Result<()> {
    let mut registry = Registry::structural();
    let mut hooks = HashMap::new();
    let mut bindings = HashMap::new();
    let mut by_state: HashMap<u32, Option<Key>> = HashMap::new();
    let mut by_code: HashMap<u32, Option<Key>> = HashMap::new();
    for a in answers {
        let Some((name, value)) = a.split_once('=') else { bail!("--answer {a:?}: expected NAME=VALUE") };
        let exit = (!value.eq_ignore_ascii_case("run")).then(|| key(value));
        if let Ok(id) = name.parse::<u32>() {
            by_state.insert(id, exit);
        } else if name.contains('.') && !name.starts_with("HSM.") {
            bindings.insert(name.to_owned(), value.to_owned());
        } else if ORIGINAL_BEHAVIOURS.contains(&name) || data.behaviours.iter().any(|b| key_name(b.id) == name) {
            by_code.insert(behaviour_id(name), exit);
        } else {
            hooks.insert(name.to_owned(), key(value));
        }
    }
    // Stand-ins for the behaviours that are not ported: an answer by state id or code, else they hold (run their
    // children and never exit, like a wait that does not end).
    let ported: HashSet<u32> = data.states.iter().map(|s| s.behaviour).filter(|b| registry.contains(*b)).collect();
    let codes: HashSet<u32> = data.states.iter().map(|s| s.behaviour).collect();
    // Ids the original does not register keep their fallback (that is the original's rule).
    for &b in codes.iter().filter(|b| (!ported.contains(b) && original_code(**b).is_some()) || by_code.contains_key(b))
    {
        let (by_state, by_code) = (by_state.clone(), by_code.get(&b).copied());
        let name = code(&data, b);
        registry.register_id(
            b,
            &name,
            Arc::new(move |s: &State| {
                let exit = by_state.get(&s.id).copied().or(by_code).flatten();
                Box::new(Stub { exit })
            }),
        );
    }
    // State-id answers for ported behaviours replace them too.
    let mut overrides: HashMap<u32, Option<Key>> = HashMap::new();
    for (&id, &exit) in &by_state {
        if let Some(s) = data.state(id)
            && ported.contains(&s.behaviour)
        {
            overrides.insert(id, exit);
        }
    }
    if !overrides.is_empty() {
        let host_data = data.clone();
        let original = registry.clone();
        for b in overrides.keys().filter_map(|id| data.state(*id)).map(|s| s.behaviour).collect::<HashSet<_>>() {
            let (orig, ov) = (original.clone(), overrides.clone());
            let name = code(&host_data, b);
            registry.register_id(
                b,
                &name,
                Arc::new(move |s: &State| match ov.get(&s.id) {
                    Some(&exit) => Box::new(Stub { exit }),
                    None => orig.create_for(s).expect("registered"),
                }),
            );
        }
    }
    let mut hsm = Hsm::new(data.clone(), registry);
    let logic = LogicHost::default();
    {
        // Stand-in for what the engine and the profile give a running game: the progression database loaded, a
        // local player, the intro already played (a continued profile).
        let mut st = logic.state.borrow_mut();
        st.progression.loaded = true;
        st.progression.world_count_ready = true;
        st.progression.collectables_ready = true;
        st.progression.set_last_played(mw_logic::natives::progression::INTRO_PLAYED, 1);
        st.players.local = Some(0);
        st.players.slots[0] = Some(Default::default());
    }
    let mut host = Driver {
        data,
        logic,
        hooks,
        bindings,
        frame: 0,
        unported_hooks: HashMap::new(),
        unknown_hooks: HashMap::new(),
    };
    let Some(serial) = hsm.start(instance, &mut host) else { bail!("no top-level state with instance {instance}") };
    let clock = Clock::both(dt);
    for f in 0..steps {
        host.frame = f;
        host.logic.frame(dt);
        hsm.update(&mut host, clock);
        if !hsm.is_running(serial) {
            break;
        }
    }
    println!("== after {} frames: machines {:?}", host.frame + 1, hsm.machines().collect::<Vec<_>>());
    for (s, _) in hsm.machines().collect::<Vec<_>>() {
        for (d, id, phase) in hsm.active(s) {
            let name = host.data.state(id).map_or("?", |s| s.name.as_str());
            println!("{}{id} {name:?} {phase:?}", "  ".repeat(d + 1));
        }
    }
    for (what, m) in [
        ("unported hooks answered EXIT by the driver", &host.unported_hooks),
        ("hooks the original does not have (EXITINVALID)", &host.unknown_hooks),
    ] {
        if !m.is_empty() {
            let mut v: Vec<_> = m.iter().collect();
            v.sort();
            println!("{what}: {v:?}");
        }
    }
    let unported = host.logic.take_unported();
    if !unported.is_empty() {
        println!("unported natives called through the logic host: {}", unported.len());
    }
    let published: Vec<String> =
        host.logic.state.borrow().view_models.published.iter().map(|(p, _)| p.clone()).collect();
    println!("published view models at the end: {published:?}");
    Ok(())
}
