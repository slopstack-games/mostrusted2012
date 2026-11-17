//! The original's game logic inside the runtime (PLAN.md M8 step 7): the HSM machine "Top State" (instance 20527 of
//! `GAMELOGIC/GAMEMODES.BNDL`) run by `mw_logic::hsm` on the logic host (`mw_logic::natives::LogicHost`: the ported
//! hooks and the shared `LogicState`), once per frame, beside the hand-written flows.
//!
//! **Shadow mode** (`--hsm`, this first step): the machine runs and logs what it does (`hsm:` lines: states
//! entered and left, hooks, the engine `Request`s it makes), but no `Request` is carried out yet; the hand-written
//! flows (`frontend.rs`, `freeroam.rs`, `race.rs`) still drive the game. Each `Request` then moves from "logged" to
//! "done by the runtime" one at a time, and the hand-written flow it replaces goes.
//!
//! What the runtime answers from its own state: the title sequence's general trigger "BOOT"
//! (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`, set by `frontend.rs` when the trigger item passes), the
//! loading screen (`HSB_WAIT4LS` / `HSB_WAIT4NL`: `frontend::Phase::Loading`), the primary user bound by the
//! title's key press (`Query::PrimaryUserBound` while `ListenForPrimaryUser` listens), whether the profile is
//! brand new (`Query::NewProfile`: `--new-game` or no saved profile), and the progression / local player the boot
//! reads (loaded at start-up; the intro counts as played for a saved profile).
//!
//! Stand-ins (model decisions, the same answers `mwtool hsm --run` documents in docs/commands.md): the engine
//! bindings `UIConfig.Platform` = "PC", `Net.IsLoggedInToServer` = false (offline),
//! `SignInState.Params.IsChatRestricted` = true; state 1222788 `ConnectToAutolog` (online Autolog sign-in) exits
//! DONE at once; behaviour types that are not ported yet hold (run their children, never exit); hooks that are
//! not ported answer EXIT (most `CALLHSMHOOK` nodes go on on EXIT).

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_formats::hsm::{HsmData, State, key, key_name};
use mw_formats::json::Json;
use mw_logic::hsm::behaviours::{Query, Stub};
use mw_logic::hsm::{Clock, Host, Hsm, Key, Registry, Trace, original_code};
use mw_logic::natives::{LogicHost, Request, progression};

use crate::frontend::{FrontEnd, Phase};

const BUNDLE: &str = "GAMELOGIC/GAMEMODES.BNDL";
/// "Top State": the boot machine.
const TOP_STATE: u32 = 20527;
/// `ConnectToAutolog` (online).
const CONNECT_TO_AUTOLOG: u32 = 1222788;
const BOOT_TRIGGER: &str = "GeneralSequence.Triggers[\"BOOT\"].IsTriggerSet[1]";

pub struct LogicPlugin {
    /// `--hsm`: run the machine (shadow mode).
    pub enabled: bool,
    /// `--new-game`, or no saved profile was loaded: the profile is brand new (`ISNEWPROFILE` → YES) and the
    /// intro is not marked played.
    pub new_profile: bool,
}

impl Plugin for LogicPlugin {
    fn build(&self, app: &mut App) {
        if !self.enabled {
            return;
        }
        app.insert_resource(Setup { new_profile: self.new_profile, pending: None })
            .add_systems(Startup, start_loading)
            .add_systems(Update, (receive, tick).chain());
    }
}

#[derive(Resource)]
struct Setup {
    new_profile: bool,
    pending: Option<Mutex<Receiver<Result<HsmData, String>>>>,
}

/// The running machine and its host (not `Send`: the logic state is shared with Lua through `Rc`).
struct Logic {
    hsm: Hsm,
    host: RuntimeHost,
    serial: u32,
    frame: u64,
}

fn start_loading(game: Res<crate::Game>, mut setup: ResMut<Setup>) {
    let store = game.store.clone();
    let (tx, rx) = channel();
    let spawned = crate::platform::thread::Builder::new().name("mw-logic".into()).spawn(move || {
        let _ = tx.send(store.hsm_data(BUNDLE).map_err(|e| e.to_string()));
    });
    match spawned {
        Ok(_) => setup.pending = Some(Mutex::new(rx)),
        Err(e) => warn!("hsm: no loader thread: {e}"),
    }
}

fn receive(world: &mut World) {
    let Some(mut setup) = world.get_resource_mut::<Setup>() else { return };
    let Some(rx) = &setup.pending else { return };
    let got = match rx.lock().map(|rx| rx.try_recv()) {
        Ok(Ok(got)) => got,
        _ => return,
    };
    setup.pending = None;
    let new_profile = setup.new_profile;
    let data = match got {
        Ok(d) => Arc::new(d),
        Err(e) => {
            warn!("hsm: {BUNDLE}: {e}");
            return;
        }
    };
    let registry = registry(&data);
    let mut hsm = Hsm::new(data.clone(), registry);
    let logic = LogicHost::default();
    {
        // What the start-up gives the boot: the progression database loaded, a local player, and (a continued
        // profile) the intro played.
        let mut st = logic.state.borrow_mut();
        st.progression.loaded = true;
        st.progression.world_count_ready = true;
        st.progression.collectables_ready = true;
        if !new_profile {
            st.progression.set_last_played(progression::INTRO_PLAYED, 1);
        }
        st.players.local = Some(0);
        st.players.slots[0] = Some(Default::default());
    }
    let mut host = RuntimeHost {
        data,
        logic,
        boot_trigger: false,
        new_profile,
        pressed: false,
        listening: false,
        frame: 0,
        requests: HashMap::new(),
    };
    let Some(serial) = hsm.start(TOP_STATE, &mut host) else {
        warn!("hsm: no top-level state {TOP_STATE}");
        return;
    };
    info!("hsm: machine {serial} started at {TOP_STATE} (shadow mode)");
    world.insert_non_send(Logic { hsm, host, serial, frame: 0 });
}

/// The structural and ported behaviour types; types the original has but we have not ported hold, and the online
/// Autolog state exits DONE.
fn registry(data: &HsmData) -> Registry {
    let mut registry = Registry::structural();
    let autolog = data.state(CONNECT_TO_AUTOLOG).map(|s| s.behaviour);
    let original = registry.clone();
    let mut codes: Vec<u32> = data.states.iter().map(|s| s.behaviour).collect();
    codes.sort_unstable();
    codes.dedup();
    for b in codes {
        let ported = original.contains(b);
        if !ported && original_code(b).is_none() && autolog != Some(b) {
            // Ids the original does not register keep their fallback (the original's rule).
            continue;
        }
        if ported && autolog != Some(b) {
            continue;
        }
        let orig = original.clone();
        let name = original_code(b).unwrap_or("?").to_owned();
        registry.register_id(
            b,
            &name,
            Arc::new(move |s: &State| {
                if s.id == CONNECT_TO_AUTOLOG {
                    Box::new(Stub { exit: Some(key("DONE")) })
                } else if ported {
                    orig.create_for(s).expect("registered")
                } else {
                    Box::new(Stub { exit: None })
                }
            }),
        );
    }
    registry
}

fn tick(logic: Option<NonSendMut<Logic>>, time: Res<Time>, front: Option<Res<FrontEnd>>) {
    let Some(mut logic) = logic else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Logic { hsm, host, serial, frame } = &mut *logic;
    *frame += 1;
    host.frame = *frame;
    if let Some(front) = &front {
        host.boot_trigger = front.boot_trigger() || front.phase != Phase::Title;
        host.pressed = front.phase != Phase::Title;
        host.logic.state.borrow_mut().engine.loading_screen_showing = front.phase == Phase::Loading;
    } else {
        host.boot_trigger = true;
        host.pressed = true;
    }
    host.logic.frame(dt);
    let was_running = hsm.is_running(*serial);
    hsm.update(host, Clock::both(dt));
    // Requests the ported hooks / Lua natives queued on the shared state.
    let queued = std::mem::take(&mut host.logic.state.borrow_mut().requests);
    for r in queued {
        host.request(r);
    }
    if was_running && !hsm.is_running(*serial) {
        info!("hsm: machine {serial} finished");
    }
}

/// The logic host plus the runtime's answers (see the module doc).
struct RuntimeHost {
    data: Arc<HsmData>,
    logic: LogicHost,
    boot_trigger: bool,
    new_profile: bool,
    /// "Press Start" was pressed (`frontend.rs` left the title on the key's release).
    pressed: bool,
    /// The primary-user listener is on (`ListenForPrimaryUser`, until `ReleasePrimaryUser`).
    listening: bool,
    frame: u64,
    /// Requests seen, by kind (shadow mode logs the first of each and counts the rest).
    requests: HashMap<String, u32>,
}

impl RuntimeHost {
    fn engine_binding(&self, path: &str) -> Option<&'static str> {
        Some(match path {
            BOOT_TRIGGER => {
                if self.boot_trigger {
                    "true"
                } else {
                    "false"
                }
            }
            "UIConfig.Platform" => "PC",
            "Net.IsLoggedInToServer" => "false",
            "SignInState.Params.IsChatRestricted" => "true",
            _ => return None,
        })
    }
}

impl Host for RuntimeHost {
    fn binding_bool(&mut self, path: &str) -> Option<bool> {
        match self.engine_binding(path) {
            Some(v) => Some(v == "true"),
            None => self.logic.binding_bool(path),
        }
    }
    fn binding_string(&mut self, path: &str) -> Option<String> {
        match self.engine_binding(path) {
            Some(v) => Some(v.to_owned()),
            None => self.logic.binding_string(path),
        }
    }
    fn binding_int(&mut self, path: &str) -> Option<i64> {
        match self.engine_binding(path) {
            Some(v) => Some((v == "true") as i64),
            None => self.logic.binding_int(path),
        }
    }
    fn hook(&mut self, name: &str, args: Option<&Json>) -> Option<Key> {
        match self.logic.registry.hook(name).map(|n| n.ported()) {
            Some(true) => {
                let exit = self.logic.hook(name, args);
                // The hook's requests come before whatever the machine asks next (the original's calls are
                // synchronous).
                let queued = std::mem::take(&mut self.logic.state.borrow_mut().requests);
                for r in queued {
                    self.request(r);
                }
                exit
            }
            Some(false) => Some(key("EXIT")),
            None => None,
        }
    }
    fn graphics_prepared(&mut self) -> bool {
        self.logic.graphics_prepared()
    }
    fn loading_screen_showing(&mut self) -> bool {
        self.logic.loading_screen_showing()
    }
    fn request(&mut self, r: Request) {
        match &r {
            Request::ListenForPrimaryUser { listen, .. } => self.listening = *listen,
            Request::ReleasePrimaryUser(_) => self.listening = false,
            _ => {}
        }
        let kind = format!("{r:?}");
        let kind = kind.split([' ', '(', '{']).next().unwrap_or_default().to_owned();
        let n = self.requests.entry(kind).or_default();
        *n += 1;
        if *n == 1 {
            info!("hsm: {:5} request {r:?} (not carried out: shadow mode)", self.frame);
        }
    }
    fn query(&mut self, q: &Query) -> Option<u64> {
        match q {
            // The press that binds the primary user is the title's key press (frontend.rs).
            Query::PrimaryUserBound => Some((self.listening && self.pressed) as u64),
            // `ISNEWPROFILE`: no saved profile was loaded (frontend.rs reads ours at start-up).
            Query::NewProfile => Some(self.new_profile as u64),
            _ => None,
        }
    }
    fn publish(&mut self, path: &str, data: &Json) {
        self.logic.publish(path, data);
    }
    fn retract(&mut self, path: &str) {
        self.logic.retract(path);
    }
    fn trace(&mut self, e: &Trace) {
        let f = self.frame;
        match e {
            Trace::Start { serial, state } => info!("hsm: {f:5} machine {serial} starts at {state}"),
            Trace::Enter { depth, state, name, behaviour, .. } => {
                info!("hsm: {f:5} {}+ {state} {name:?} [{behaviour}]", "  ".repeat(*depth))
            }
            Trace::Exit { depth, state, name, exit } => {
                info!("hsm: {f:5} {}- {state} {name:?} → {}", "  ".repeat(*depth), key_name(*exit))
            }
            Trace::Hook { name, result, .. } => info!("hsm: {f:5}      hook {name} → {}", key_name(*result)),
            Trace::Finish { serial, exit } => info!("hsm: {f:5} machine {serial} finished → {}", key_name(*exit)),
            Trace::Missing { state, what } => {
                let name = self.data.state(*state).map_or("?", |s| s.name.as_str());
                warn!("hsm: {f:5} {state} {name:?}: {what}")
            }
        }
    }
}
