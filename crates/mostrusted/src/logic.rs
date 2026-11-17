//! The original's game logic inside the runtime (PLAN.md M8 step 7): the HSM machine "Top State" (instance 20527 of
//! `GAMELOGIC/GAMEMODES.BNDL`) run by `mw_logic::hsm` on the logic host (`mw_logic::natives::LogicHost`: the ported
//! hooks and the shared `LogicState`), once per frame, beside the hand-written flows.
//!
//! **`--hsm`**: the machine runs and logs what it does (`hsm:` lines: states entered and left, hooks, the engine
//! `Request`s it makes). Requests move from "logged" ("not carried out yet") to "done" one at a time, and the
//! hand-written flow each one replaces goes. Done so far:
//! - page contexts and widget groups (`mw_logic::natives::pages` inside the logic host; the screens they need are
//!   loaded here on a thread): the widgets' Lua scripts run as in the original;
//! - the boot page's front end: the title sequence plays when the "Press Start" group's `PlaySequence` script
//!   starts it (`StartSequence` of `Widget.Background.Sequence` = [`fe::TITLE_SEQUENCE`]); the key press only binds
//!   the user (`frontend.rs` in driven mode); the boot transition starts on the group's
//!   `PlayWidgetGroupTransition` and answers `WidgetTransitionStarted` / `TransitionFinished`.
//!
//! - conductors: `StartSequence` of the title goes to `frontend.rs`; any other sequence runs headless
//!   ([`Playback`]: its clock with its jumps, the conductor idle again at its end; nothing drawn yet);
//!   `StopSequence` / `DestroyConductor` end it (the title's stops the title);
//! - widget input: the menu actions (`input::MenuInput`, ids of `FUN_007e7910`'s table) reach the active widgets'
//!   user actions every frame before the HSM update (`LogicHost::input`: Lua callbacks, `ExitGroupVia`, hooks).
//!
//! Everything else (world, profile and feedback-group loads, the game mode, the HUD screen's requests) is still
//! logged only: `freeroam.rs`, `race.rs` and the rest of `frontend.rs` drive the game.
//!
//! What the runtime answers from its own state: the title sequence's general trigger "BOOT"
//! (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`, set by `frontend.rs` when the trigger item passes and
//! handed to the logic with `LogicState::set_general_trigger`), the
//! loading screen (`HSB_WAIT4LS` / `HSB_WAIT4NL`: `frontend::Phase::Loading`), the primary user bound by the
//! title's key press (`Query::PrimaryUserBound` while `ListenForPrimaryUser` listens), whether the profile is
//! brand new (`Query::NewProfile`: `--new-game` or no saved profile), and the progression / local player the boot
//! reads (loaded at start-up; the intro counts as played for a saved profile). `Mode.*` bindings are the game-mode
//! manager's own table (`mw_logic::natives::mode_view`); the runtime sets its world-loaded byte +0x2e77 from
//! `FrontEnd::ready`.
//!
//! Stand-ins (model decisions, the same answers `mwtool hsm --run` documents in docs/commands.md): the engine
//! bindings: none left but `Transitions.Transitioning` (`Net.*`, `UIConfig.*`, `SignInState.*`, `UISequences.*` and
//! `GeneralSequence.*` are the logic's own offline roots); state 1222788 `ConnectToAutolog` (online Autolog sign-in) exits
//! DONE at once; behaviour types that are not ported yet hold (run their children, never exit); hooks that are
//! not ported answer EXIT (most `CALLHSMHOOK` nodes go on on EXIT).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::frontend as fe;
use mw_assets::sequence::Sequence;
use mw_formats::hsm::{HsmData, State, key, key_name};
use mw_formats::json::Json;
use mw_logic::hsm::behaviours::{Query, Stub};
use mw_logic::hsm::{Clock, Host, Hsm, Key, Registry, Trace, original_code};
use mw_logic::natives::game_modes::EventInfo;
use mw_logic::natives::mode_machine::{self, ModeInfo};
use mw_logic::natives::mode_view;
use mw_logic::natives::pages::{self, ActionInput, ScreenData};
use mw_logic::natives::{LogicHost, Request, progression};

use crate::frontend::{FrontEnd, Phase};

const BUNDLE: &str = "GAMELOGIC/GAMEMODES.BNDL";
/// "Top State": the boot machine.
const TOP_STATE: u32 = 20527;
/// `ConnectToAutolog` (online).
const CONNECT_TO_AUTOLOG: u32 = 1222788;
/// The boot page's "Press Start" widget group (HSM state 491835 `GROUP`): its `Background` widget plays
/// [`fe::TITLE_SEQUENCE`], its `ScreenTransition` is the boot transition [`fe::BOOT_TRANSITION_SEQUENCE`].
const PRESS_START_GROUP: i64 = 491_032;
/// The title sequence's general trigger (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`).
const BOOT_TRIGGER_NAME: &str = "BOOT";

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
    pending: Option<Mutex<Receiver<Result<Loaded, String>>>>,
}

/// What the loader thread reads: the machines, and the event / game-mode data and `GameLogicConfig` ids the
/// game-mode manager reads.
struct Loaded {
    data: HsmData,
    events: mw_assets::events::Events,
    ids: mw_assets::frontend::LogicConfigIds,
}

/// A screen load's result.
type ScreenLoad = Result<ScreenData, String>;
/// A conductor's sequence load: (conductor, sequence id, result).
type SequenceLoad = (String, u32, Result<Arc<Sequence>, String>);

/// The running machine and its host (not `Send`: the logic state is shared with Lua through `Rc`).
struct Logic {
    hsm: Hsm,
    host: RuntimeHost,
    serial: u32,
    frame: u64,
    /// Screens the page contexts asked for, loaded on a thread ([`mw_logic::natives::pages::load_screen`]).
    screens: (Sender<ScreenLoad>, Receiver<ScreenLoad>),
    /// Conductors' sequences (conductor, sequence id, the loaded sequence).
    sequences: (Sender<SequenceLoad>, Receiver<SequenceLoad>),
    store: Arc<mw_assets::AssetStore>,
}

fn start_loading(game: Res<crate::Game>, mut setup: ResMut<Setup>) {
    let store = game.store.clone();
    let (tx, rx) = channel();
    let spawned = crate::platform::thread::Builder::new().name("mw-logic".into()).spawn(move || {
        let load = || -> anyhow::Result<Loaded> {
            Ok(Loaded { data: store.hsm_data(BUNDLE)?, events: store.events()?, ids: store.start_config()?.ids })
        };
        let _ = tx.send(load().map_err(|e| e.to_string()));
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
    let Loaded { data, events, ids } = match got {
        Ok(l) => l,
        Err(e) => {
            warn!("hsm: {BUNDLE}: {e}");
            return;
        }
    };
    let data = Arc::new(data);
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
        // The game-mode manager's data (`natives::mode_machine`), filled as `mwtool hsm --run` does. Stand-ins
        // (model decisions, as there): an event is available when it is in an `EventList` (`FUN_006e58a0` looks it
        // up in the selected world's list), every rollout check passes, the event data and weapon lists are
        // loaded at once.
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
            .map(|e| EventInfo {
                id: e.id,
                mode: e.game_mode,
                available: listed.contains(&e.id),
                rollout_ok: true,
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
    }
    let mut host = RuntimeHost {
        data,
        logic,
        boot_trigger: false,
        new_profile,
        transitioning: false,
        carry: Vec::new(),
        playbacks: Vec::new(),
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
    let store = world.resource::<crate::Game>().store.clone();
    world.insert_non_send(Logic { hsm, host, serial, frame: 0, screens: channel(), sequences: channel(), store });
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

fn tick(
    logic: Option<NonSendMut<Logic>>,
    time: Res<Time>,
    mut front: Option<ResMut<FrontEnd>>,
    menu: Res<crate::input::MenuInput>,
) {
    let Some(mut logic) = logic else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Logic { hsm, host, serial, frame, screens, sequences, store } = &mut *logic;
    while let Ok((conductor, sequence, got)) = sequences.1.try_recv() {
        let Some(p) = host.playbacks.iter_mut().find(|p| p.conductor == conductor && p.sequence == sequence) else {
            continue;
        };
        match got {
            Ok(seq) => p.seq = Some(seq),
            Err(e) => {
                warn!("hsm: sequence {sequence}: {e}");
                host.playbacks.retain(|p| p.conductor != conductor);
                set_idle(&host.logic, &conductor);
            }
        }
    }
    *frame += 1;
    host.frame = *frame;
    // Screens: start the loads the page contexts asked for, hand over the finished ones.
    for id in std::mem::take(&mut host.logic.pages.wanted) {
        let (tx, store) = (screens.0.clone(), store.clone());
        let spawned = crate::platform::thread::Builder::new().name("mw-screen".into()).spawn(move || {
            let _ = tx.send(pages::load_screen(&store, id).map_err(|e| format!("screen {id}: {e}")));
        });
        if let Err(e) = spawned {
            warn!("hsm: screen {id}: no loader thread: {e}");
        }
    }
    while let Ok(got) = screens.1.try_recv() {
        match got {
            Ok(screen) => {
                info!("hsm: {:5} screen {} loaded ({} widgets)", host.frame, screen.id, screen.widgets.len());
                host.logic.pages.provide(screen);
            }
            Err(e) => warn!("hsm: {e}"),
        }
    }
    if let Some(front) = &front {
        host.boot_trigger = front.boot_trigger() || front.phase != Phase::Title;
        host.pressed = front.pressed() || front.phase != Phase::Title;
        // The game-mode manager's world-loaded byte (+0x2e77, `Mode.WorldLoaded` / `Mode.ShowLoadingScreen`).
        host.logic.state.borrow_mut().game_modes.world_loaded = front.ready;
        // Requested and not yet started counts as playing.
        host.transitioning =
            front.phase == Phase::Transition || host.carry.iter().any(|c| matches!(c, Carry::BootTransition));
        host.logic.state.borrow_mut().engine.loading_screen_showing = front.phase == Phase::Loading;
    } else {
        host.boot_trigger = true;
        host.pressed = true;
        host.logic.state.borrow_mut().game_modes.world_loaded = true;
    }
    // The title sequence's general trigger item "BOOT" (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`), set in
    // the logic's trigger table as the sequence player sets it.
    if host.boot_trigger {
        let name = mw_logic::binding::hash(BOOT_TRIGGER_NAME);
        host.logic.state.borrow_mut().set_general_trigger(name, 1);
    }
    host.logic.frame(dt);
    step_playbacks(host, dt);
    // The widgets' user actions (before the HSM's update, which reads the group exit they set).
    host.logic.input(&|id| {
        let (pressed, repeat, value) = menu.by_id(id).unwrap_or_default();
        ActionInput { pressed, repeat, value }
    });
    let was_running = hsm.is_running(*serial);
    hsm.update(host, Clock::both(dt));
    // The game-mode manager's update right after the HSM's (`FUN_006abc10`): starts the mode's machine. Stand-in:
    // the weapon list loads at once.
    let state = host.logic.state.clone();
    {
        let mut st = state.borrow_mut();
        st.game_modes.weapon_list_loaded = st.game_modes.weapon_list;
    }
    mode_machine::update(&state, hsm, host);
    // Requests the ported hooks / Lua natives queued on the shared state.
    let queued = std::mem::take(&mut host.logic.state.borrow_mut().requests);
    for r in queued {
        host.request(r);
    }
    if was_running && !hsm.is_running(*serial) {
        info!("hsm: machine {serial} finished");
    }
    for e in std::mem::take(&mut host.logic.pages.errors) {
        warn!("hsm: script {e}");
    }
    // The requests the runtime carries out (module doc). Conductors' sequences load on a thread.
    for c in &host.carry {
        if let Carry::Play { conductor, sequence } = c {
            let (tx, store, conductor, sequence) = (sequences.0.clone(), store.clone(), conductor.clone(), *sequence);
            host.playbacks.push(Playback { conductor: conductor.clone(), sequence, seq: None, clock: 0.0 });
            let _ = crate::platform::thread::Builder::new().name("mw-sequence".into()).spawn(move || {
                let _ = tx.send((
                    conductor,
                    sequence,
                    store.ui_sequence(sequence).map(Arc::new).map_err(|e| e.to_string()),
                ));
            });
        }
    }
    for r in std::mem::take(&mut host.carry) {
        let Some(front) = front.as_deref_mut() else { continue };
        match r {
            Carry::Title => front.start_title(),
            Carry::StopTitle => front.stop_title(),
            Carry::Play { .. } | Carry::Nothing => {}
            Carry::BootTransition => front.start_transition(),
        }
    }
}

/// A request the runtime carries out.
enum Carry {
    /// `StartSequence` of the title sequence (the "Press Start" group's `PlaySequence` script).
    Title,
    /// `PlayWidgetGroupTransition` of the "Press Start" group: its `ScreenTransition` (the boot transition).
    BootTransition,
    /// `StopSequence` / `DestroyConductor` of the title's conductor.
    StopTitle,
    /// `StartSequence` of any other sequence: its clock runs here ([`Playback`]).
    Play { conductor: String, sequence: u32 },
    /// Carried out with nothing to do (`CreateConductor`: the conductor is logic state).
    Nothing,
}

/// The title's conductor (the "Press Start" `Background` widget's `ConductorName`).
const TITLE_CONDUCTOR: &str = "PressStart";

/// A conductor's sequence that the runtime does not draw yet: its clock runs with the sequence's jump controllers
/// (`frontend::step`, bindings through the logic's view models) and, once it reaches the sequence's end, the
/// conductor is idle again (`HasSequenceFinished` / `IsConductorIdle`), so scripts waiting on it keep the
/// original's timing.
struct Playback {
    conductor: String,
    sequence: u32,
    seq: Option<Arc<Sequence>>,
    clock: f32,
}

/// The conductor's `playing` cleared (the runtime finished or could not play its sequence).
fn set_idle(logic: &LogicHost, conductor: &str) {
    let h = mw_formats::genesys::field_hash(conductor);
    if let Some(c) = logic.state.borrow_mut().screen.conductors.iter_mut().find(|c| c.name_hash == h) {
        c.playing = None;
    }
}

/// Steps the conductors' sequences (see [`Playback`]).
fn step_playbacks(host: &mut RuntimeHost, dt: f32) {
    let mut finished = Vec::new();
    for p in &mut host.playbacks {
        let Some(seq) = &p.seq else { continue };
        let vm = &host.logic.state;
        let (next, _) = crate::frontend::step(seq, p.clock, dt, &mut |b| vm.borrow().resolve_bool(b) == Some(true));
        p.clock = next;
        if next >= seq.length {
            finished.push((p.conductor.clone(), p.sequence));
        }
    }
    for (conductor, sequence) in finished {
        info!("hsm: {:5} sequence {sequence} on conductor {conductor:?} finished", host.frame);
        host.playbacks.retain(|p| p.conductor != conductor);
        set_idle(&host.logic, &conductor);
    }
}

/// The logic host plus the runtime's answers (see the module doc).
struct RuntimeHost {
    data: Arc<HsmData>,
    logic: LogicHost,
    boot_trigger: bool,
    new_profile: bool,
    /// The boot screen transition plays (`Transitions.Transitioning`; also true from its request until the front
    /// end has started it).
    transitioning: bool,
    /// Requests for `tick` to carry out.
    carry: Vec<Carry>,
    /// The conductors' sequences the runtime runs without drawing.
    playbacks: Vec<Playback>,
    /// "Press Start" was pressed (`frontend.rs` left the title on the key's release).
    pressed: bool,
    /// The primary-user listener is on (`ListenForPrimaryUser`, until `ReleasePrimaryUser`).
    listening: bool,
    frame: u64,
    /// Requests seen, by kind (shadow mode logs the first of each and counts the rest).
    requests: HashMap<String, u32>,
}

impl RuntimeHost {
    /// Engine bindings the logic host has no root for yet (the game-mode manager's `Mode` table, the
    /// transitions): the runtime's own state where it has it, else the stand-ins of the module doc.
    fn engine_binding(&self, path: &str) -> Option<&'static str> {
        let bool = |b: bool| if b { "true" } else { "false" };
        Some(match path {
            "Transitions.Transitioning(NOT)" => bool(!self.transitioning),
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
            Some(v) => Some(v.parse().unwrap_or((v == "true") as i64)),
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
        // The parts of the original's call the logic does itself (the game-mode start, input blocks, page
        // contexts and widget groups).
        self.logic.apply(&r);
        let carried = match &r {
            Request::StartSequence { conductor, sequence } => {
                let id = self.logic.state.borrow().resolve_int(sequence);
                // A new sequence replaces what the conductor played.
                self.playbacks.retain(|p| &p.conductor != conductor);
                match id {
                    Some(id) if id == i64::from(fe::TITLE_SEQUENCE) => Some(Carry::Title),
                    Some(id) if id > 0 => Some(Carry::Play { conductor: conductor.clone(), sequence: id as u32 }),
                    _ => {
                        warn!("hsm: {:5} StartSequence {sequence:?}: no sequence id", self.frame);
                        // Nothing to play: the conductor is idle again (`HasSequenceFinished`).
                        set_idle(&self.logic, conductor);
                        None
                    }
                }
            }
            Request::StopSequence { conductor } | Request::DestroyConductor { name: conductor } => {
                self.playbacks.retain(|p| &p.conductor != conductor);
                (conductor == TITLE_CONDUCTOR).then_some(Carry::StopTitle)
            }
            Request::CreateConductor { .. } => Some(Carry::Nothing),
            Request::PlayWidgetGroupTransition { group, .. } if *group == PRESS_START_GROUP => {
                self.transitioning = true;
                Some(Carry::BootTransition)
            }
            _ => None,
        };
        let done = carried.is_some()
            || matches!(
                r,
                Request::PushPageContext { .. }
                    | Request::LeavePageContext { .. }
                    | Request::ActivateWidgetGroup { .. }
                    | Request::DeactivateWidgetGroup { .. }
                    | Request::ClearWidgetExit
                    | Request::StartSequence { .. }
                    | Request::StopSequence { .. }
                    | Request::DestroyConductor { .. }
            );
        self.carry.extend(carried);
        let kind = format!("{r:?}");
        let kind = kind.split([' ', '(', '{']).next().unwrap_or_default().to_owned();
        let n = self.requests.entry(kind).or_default();
        *n += 1;
        if done {
            info!("hsm: {:5} request {r:?}", self.frame);
        } else if *n == 1 {
            info!("hsm: {:5} request {r:?} (not carried out yet)", self.frame);
        }
    }
    fn query(&mut self, q: &Query) -> Option<u64> {
        match q {
            // The press that binds the primary user is the title's key press (frontend.rs).
            Query::PrimaryUserBound => Some((self.listening && self.pressed) as u64),
            // `ISNEWPROFILE`: no saved profile was loaded (frontend.rs reads ours at start-up).
            Query::NewProfile => Some(self.new_profile as u64),
            // The boot transition (the only group transition the runtime plays so far).
            Query::WidgetTransitionStarted { group } => Some((*group == PRESS_START_GROUP) as u64),
            Query::TransitionFinished => Some(!self.transitioning as u64),
            _ => self.logic.query(q),
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
