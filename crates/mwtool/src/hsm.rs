//! `mwtool hsm`: the game logic's state machines (`mw_formats::hsm`): listing, trees, search, and a headless
//! run of the interpreter (`mw_logic::hsm`) on the logic host (`mw_logic::natives::LogicHost`: ported hooks,
//! published view models, `Signals`), with stand-ins for unported hooks / behaviours and `--answer` overrides.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, bail};
use mw_assets::AssetStore;
use mw_formats::hsm::{Action, HsmData, ParamType, Parameter, State, behaviour_id, key, key_name};
use mw_formats::json::Json;
use mw_logic::hsm::behaviours::{Query, Stub};
use mw_logic::hsm::{Clock, Host, Hsm, Key, ORIGINAL_BEHAVIOURS, Registry, Trace, original_code};
use mw_logic::natives::mode_machine::{self, ModeInfo};
use mw_logic::natives::mode_view;
use mw_logic::natives::{LogicHost, Request};

pub struct Options<'a> {
    pub bundle: &'a str,
    pub node: Option<u32>,
    pub find: Option<&'a str>,
    pub depth: usize,
    pub parents: bool,
    pub data: bool,
    pub codes: bool,
    pub run: Option<u32>,
    pub steps: usize,
    pub dt: f32,
    pub answers: &'a [String],
    pub event: Option<u32>,
    pub pause_at: Option<usize>,
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
        return drive(store, Arc::new(data), instance, &o);
    }
    if o.codes {
        // Every behaviour code the bundle's states use, with its use count and whether the registry ports it.
        let registry = Registry::structural();
        let mut uses: std::collections::BTreeMap<String, (usize, bool)> = Default::default();
        for s in &data.states {
            let e = uses.entry(code(&data, s.behaviour)).or_insert((0, registry.contains(s.behaviour)));
            e.0 += 1;
        }
        if let Some(text) = o.find {
            // With --find: the states of the matching codes, with their parameters and data definition.
            for s in data.states.iter().filter(|s| code(&data, s.behaviour).contains(text)) {
                println!(
                    "{:8} {} {:?} {} def {:?}",
                    s.id,
                    code(&data, s.behaviour),
                    s.name,
                    params(&s.parameters),
                    s.data_definition
                );
            }
            return Ok(());
        }
        let missing = uses.values().filter(|(_, p)| !p).count();
        for (name, (n, ported)) in &uses {
            println!("{n:5} {} {name}", if *ported { "ported  " } else { "UNPORTED" });
        }
        println!("{} codes, {missing} unported", uses.len());
        return Ok(());
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
    /// A headless press: the primary user binds as soon as the binding press is listened for.
    user_bound: bool,
    /// Per-frame re-assertions (`HSB_DISCONGP`'s update) are printed once.
    printed_blocks: HashSet<String>,
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
    fn request(&mut self, r: Request) {
        match &r {
            Request::ReleasePrimaryUser(true) => self.user_bound = false,
            Request::ListenForPrimaryUser { listen: true, .. } => self.user_bound = true,
            _ => {}
        }
        let text = format!("{r:?}");
        let repeat = matches!(r, Request::SetInputBlock { set: true, .. }) && !self.printed_blocks.insert(text.clone());
        if let Request::SetInputBlock { set: false, .. } = r {
            self.printed_blocks.clear();
        }
        if !repeat {
            println!("{:5}      request {text}", self.frame);
        }
        self.logic.apply(&r);
    }
    /// `--answer Query.<Name>=<value>` (true / false / a number / an exit point name), else the driver's own
    /// state (the primary user), else the query's default.
    fn query(&mut self, q: &Query) -> Option<u64> {
        if let Some(v) = self.bindings.get(&format!("Query.{}", q.name())) {
            return Some(match v.to_ascii_lowercase().as_str() {
                "true" | "yes" => 1,
                "false" | "no" => 0,
                _ => v.parse().unwrap_or_else(|_| key(v)),
            });
        }
        match q {
            Query::PrimaryUserBound => Some(self.user_bound as u64),
            _ => self.logic.query(q),
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

/// `--event`: stands in for the Easy Drive menu "Easy Drive Menu" (1135029) after the player picked an event: the
/// list item's hook `StartEventFromEasyDrive` (`lpcValue` = the event id, `FUN_006e74f0`) and the exit `TO_EVENT`.
struct ChooseEvent {
    event: u32,
}

impl mw_logic::hsm::Behaviour for ChooseEvent {
    fn update(&mut self, cx: &mut mw_logic::hsm::Cx) -> bool {
        let args = Json::parse(&format!(r#"{{ "lpcValue": "{}" }}"#, self.event)).expect("json");
        cx.host.hook("StartEventFromEasyDrive", Some(&args));
        cx.set_exit(key("TO_EVENT"));
        true
    }
}

fn drive(store: &AssetStore, data: Arc<HsmData>, instance: u32, o: &Options) -> Result<()> {
    let Options { steps, dt, answers, event, pause_at, .. } = *o;
    // `--event`: the event's route for the logic's checkpoint manager (`natives::checkpoints`) and a driver stand-in
    // that drives the local player's car along its gates.
    let mut lap = match event {
        Some(id) => SyntheticLap::new(store, id)?,
        None => None,
    };
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
    // `--event`: the player's choice in the Easy Drive menu (the widget actions are Lua and input, which the
    // headless run does not have): the first input of "Wait for UI Input" opens the menu (EZ_RIGHT), and the
    // menu's state calls the hook the event list's item calls and exits TO_EVENT.
    if let Some(id) = event {
        by_state.entry(908987).or_insert(Some(key("EZ_RIGHT")));
        if let Some(s) = data.state(1135029) {
            let b = s.behaviour;
            let name = code(&data, b);
            let orig = registry.clone();
            // The player chooses once; the way back to free roam after the event is not chosen again.
            let chosen = std::sync::atomic::AtomicBool::new(false);
            registry.register_id(
                b,
                &name,
                Arc::new(move |s: &State| {
                    if s.id == 1135029 && !chosen.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        Box::new(ChooseEvent { event: id })
                    } else {
                        orig.create_for(s).expect("registered")
                    }
                }),
            );
        }
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
        // (its car exists and the engine runs: `Players.LocalPlayer.IsEngineOn`, which the engine owns)
        st.players.slots[0] =
            Some(mw_logic::natives::players::Player { vehicle_alive: true, engine_on: true, ..Default::default() });
        // The game-mode data and `GameLogicConfig` ids the GMM reads (what the runtime fills from the install).
        // Stand-ins: an event is available when it is in an `EventList` (`FUN_006e58a0` looks it up in the
        // selected world's list; GAMEPLAY has one list), every rollout check passes, the event data and the
        // weapon lists load at once.
        let events = store.events()?;
        let ids = store.start_config()?.ids;
        // The packs (game-pack manager) and each car's pack; nothing is owned (the user's offline decision,
        // `natives::entitlements`).
        st.entitlements = mw_logic::natives::entitlements::load(store)?;
        st.config.escape_mode = ids.escape_mode;
        st.config.default_rollout = ids.default_rollout;
        st.config.freedrive_mode = ids.freedrive_mode;
        st.config.intro_event = ids.intro_event;
        st.config.intro_race_event = ids.intro_race_event;
        st.config.online_freedrive_mode = ids.online_freedrive_mode;
        let listed: HashSet<u32> = events.event_lists.iter().flat_map(|(_, l)| l.iter().copied()).collect();
        st.game_modes.events = events
            .events
            .iter()
            .map(|e| mw_logic::natives::game_modes::EventInfo {
                id: e.id,
                mode: e.game_mode,
                available: listed.contains(&e.id),
                rollout_ok: true,
                data: events
                    .event_fields
                    .iter()
                    .find(|(id, _)| *id == e.id)
                    .map_or_else(Vec::new, |(_, f)| mode_view::data_fields(f)),
                ..Default::default()
            })
            .collect();
        st.game_modes.modes = events
            .game_modes
            .iter()
            .map(|m| ModeInfo {
                id: m.id,
                hsm: m.hsm,
                weapon_list: m.weapon_list,
                hud_hsm: m.hud_hsm,
                data: events
                    .game_mode_fields
                    .iter()
                    .find(|(id, _)| *id == m.id)
                    .map_or_else(Vec::new, |(_, f)| mode_view::data_fields(f)),
            })
            .collect();
        st.game_modes.event_data_ready = true;
        // The world loads at once too (the game-mode manager's +0x2e77, `Mode.WorldLoaded`).
        st.game_modes.world_loaded = true;
    }
    // The title sequence's "BOOT" general trigger item has fired (the runtime sets it the same way when the title's
    // trigger item passes): `GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`.
    logic.state.borrow_mut().set_general_trigger(mw_logic::binding::hash("BOOT"), 1);
    let mut host = Driver {
        data,
        logic,
        hooks,
        bindings,
        frame: 0,
        unported_hooks: HashMap::new(),
        unknown_hooks: HashMap::new(),
        user_bound: false,
        printed_blocks: HashSet::new(),
    };
    let Some(serial) = hsm.start(instance, &mut host) else { bail!("no top-level state with instance {instance}") };
    let clock = Clock::both(dt);
    for f in 0..steps {
        host.frame = f;
        // The screens the page contexts asked for, loaded at once (the runtime loads them on its loader thread).
        for id in std::mem::take(&mut host.logic.pages.wanted) {
            match mw_logic::natives::pages::load_screen(store, id) {
                Ok(screen) => {
                    println!("{f:5}      screen {id} loaded ({} widgets)", screen.widgets.len());
                    host.logic.pages.provide(screen);
                }
                Err(e) => println!("{f:5}      !! screen {id}: {e}"),
            }
        }
        host.logic.frame(dt);
        if let Some(lap) = &mut lap {
            lap.frame(&mut host.logic.state.borrow_mut(), dt, f);
        }
        hsm.update(&mut host, clock);
        for e in std::mem::take(&mut host.logic.pages.errors) {
            println!("{f:5}      !! script {e}");
        }
        // The GMM's update after the HSM's (`FUN_006abc10`): starts the mode's machine.
        let state = host.logic.state.clone();
        {
            let mut st = state.borrow_mut();
            st.game_modes.weapon_list_loaded = st.game_modes.weapon_list;
            // The mode's intro load (`Mode.HasIntroLoadCompleted`, +0x2e82) completes at once, like the other loads.
            if event.is_some() && st.game_modes.machine.is_some() {
                st.game_modes.intro_load_complete = true;
            }
        }
        mode_machine::update(&state, &mut hsm, &mut host);
        if pause_at.is_some_and(|p| f >= p) {
            host.bindings.insert("Query.InputExit".into(), "PAUSE".into());
        }
        if event.is_some_and(|id| state.borrow().game_modes.current_event == id) {
            // The intro's UI sequence fires the "TRANSITION" general trigger item when its screen transition runs
            // (the runtime does; "Screen Transition Voodoo" waits for it).
            state.borrow_mut().set_general_trigger(mw_logic::binding::hash("TRANSITION"), 1);
        }
        let hook_requests = std::mem::take(&mut state.borrow_mut().requests);
        for r in hook_requests {
            println!("{f:5}      hook request {r:?}");
        }
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

/// `--event`: the runtime's part of the checkpoint manager, headless: the route the distance-to-finish manager would
/// build (`FUN_00645290`: the spawn, then the event's gates for every lap), the local player's car box, the race
/// clock (GMM+0x2d28, advanced by the frame time while the car drives), and a driver stand-in that drives the car
/// straight at the logic's current target gate at a fixed speed once the gates are placed. The route is there
/// while the game-mode manager runs the event (the original builds it on `CHECKPTS` enter from the current event;
/// free drive has none). The logic decides the passing, laps and the finish (`mw_logic::natives::checkpoints`).
struct SyntheticLap {
    event: u32,
    inputs: mw_logic::natives::checkpoints::CheckpointInputs,
    installed: bool,
    position: [f32; 3],
    heading: [f32; 3],
    driving: bool,
    passed: usize,
}

impl SyntheticLap {
    /// The stand-in's speed (m/s): a fast race pace (no handling: straight lines between the gates).
    const SPEED: f32 = 60.0;
    /// A car's half extents (m).
    const HALF: [f32; 3] = [1.0, 0.7, 2.3];

    fn new(store: &AssetStore, id: u32) -> Result<Option<Self>> {
        use mw_logic::natives::checkpoints::{CheckpointInputs, route_from_gates};
        use mw_logic::natives::trigger_boxes::{TriggerBox, cross};
        let events = store.events()?;
        let Some(e) = events.events.iter().find(|e| e.id == id) else { return Ok(None) };
        if e.checkpoints.is_empty() {
            return Ok(None);
        }
        let boxes: Vec<TriggerBox> = e
            .checkpoints
            .iter()
            .map(|g| {
                let (x, z) = (g.across(), g.normal());
                TriggerBox { id: g.id, centre: g.position, axes: [x, cross(z, x), z], size: g.size, one_way: false }
            })
            .collect();
        let (position, heading) = e.spawn.map_or((boxes[0].ground(), boxes[0].axes[2]), |p| (p.position, p.direction));
        let laps = usize::from(e.laps.max(1));
        println!("      synthetic lap: event {id}, {} gates × {laps} laps", boxes.len());
        let inputs = CheckpointInputs {
            route: route_from_gates(position, &boxes, laps),
            per_lap: boxes.len(),
            loaded: true,
            markers: events.game_modes.iter().find(|m| m.id == e.game_mode).is_some_and(|m| m.gate_markers),
            first_place_only: e.first_place_only,
            ..Default::default()
        };
        Ok(Some(SyntheticLap { event: id, inputs, installed: false, position, heading, driving: false, passed: 0 }))
    }

    fn frame(&mut self, st: &mut mw_logic::natives::LogicState, dt: f32, f: usize) {
        use mw_logic::natives::checkpoints::RaceCar;
        use mw_logic::natives::trigger_boxes::{CarBox, add, dist2, scale, sub};
        let running = st.game_modes.current_event == self.event;
        if running != self.installed {
            self.installed = running;
            st.checkpoints.inputs = if running { self.inputs.clone() } else { Default::default() };
            (self.driving, self.passed) = (false, 0);
        }
        let c = &st.checkpoints;
        let (cp, lap) = (c.progress.cp, c.progress.lap);
        if self.driving && cp != self.passed && running {
            println!("{f:5}      synthetic lap: gate {cp} of {} passed (lap {lap})", c.inputs.route.len() - 1);
            self.passed = cp;
        }
        if !c.progress.gates.is_empty() && !c.progress.finished && cp + 1 < c.inputs.route.len() {
            if !self.driving {
                println!("{f:5}      synthetic lap: the gates are placed, driving");
                self.driving = true;
            }
            let e = c.inputs.route[cp + 1];
            let target = e.gate.map_or(e.position, |g| g.centre);
            let to = sub([target[0], self.position[1], target[2]], self.position);
            let d = dist2(to, [0.0; 3]).sqrt();
            if d > 1e-3 {
                self.heading = scale(to, 1.0 / d);
                self.position = add(self.position, scale(self.heading, (Self::SPEED * dt).min(d)));
            }
            self.position[1] = e.position[1] + Self::HALF[1];
            let gm = &mut st.game_modes;
            let ticks =
                (u64::from(gm.word_2d28) | (u64::from(gm.word_2d2c) << 32)) + (f64::from(dt) * 3e6).round() as u64;
            (gm.word_2d28, gm.word_2d2c) = (ticks as u32, (ticks >> 32) as u32);
        }
        let car = CarBox::level(self.position, self.heading, Self::HALF);
        st.checkpoints.inputs.cars[0] = Some(RaceCar { car, has_vehicle: true, ..Default::default() });
    }
}
