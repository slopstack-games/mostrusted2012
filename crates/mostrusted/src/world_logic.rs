//! The original's world managers in every mode: the checkpoint manager of `CHECKPTS` (`FUN_0075da20`), the event
//! starts of `EVENTSTARTS` (`FUN_00755220`) and the jack spots / car swap of `HSMCARSWAPLO` (`FUN_00761d70`), ported
//! engine-free in `mw_logic::natives::{checkpoints, event_starts, car_swap}` (decomp.md "The checkpoint manager in
//! the logic", "Event starts", "Jack spots").
//!
//! [`WorldLogic`] holds the `LogicState` they run on: under `--hsm` the logic host's own (logic.rs hands it over when
//! the machine is loaded; the HSM's behaviours issue the managers' enter / step / exit requests and the host carries
//! them out), otherwise one owned here, on which [`step_plain`] issues what the free-drive machine's "Do Event Starts"
//! (node 816214, its parameters read from the HSM data) and "Car Swap Logic" do each frame, and `race.rs` what the
//! race mode's "Checkpoints" does. Each frame the runtime fills the managers' documented inputs
//! ([`WorldLogicSet::Inputs`]: `race.rs`, `event_starts.rs`, `jackspots.rs`), the managers step (the HSM's
//! `LogicTick` or [`WorldLogicSet::Step`]) and the runtime carries out their outcomes ([`WorldLogicSet::Outcomes`]:
//! the `Request::Checkpoint` / `EventStart` / `CarSwap` requests in [`WorldLogic::outbox`], the logic state they
//! changed). The `GameLogicConfig` values the managers read come from the data ([`WorldLogic::values`]).
//!
//! Plain-play stand-ins (model decisions): the free-drive machine's managers run while no event is loaded
//! (`RaceState` absent); after an event start the event-start manager waits until the event has run (or, for an
//! event the runtime cannot run, enters again at once); the HSM's prompt clearer (`LogicHost::frame`) is done here
//! at the start of each frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::prelude::*;
use mw_assets::frontend::GameLogicValues;
use mw_logic::natives::event_starts::EventStartParams;
use mw_logic::natives::players::Player;
use mw_logic::natives::{LogicState, Request, car_swap, checkpoints, event_starts};

/// The HSM node of the free drive's "Do Event Starts" (`EVENTSTARTS`; 1560429 is the intro's, same parameters).
const EVENT_STARTS_NODE: u32 = 816_214;
const HSM_BUNDLE: &str = "GAMELOGIC/GAMEMODES.BNDL";

pub struct WorldLogicPlugin {
    /// `--hsm`: the HSM issues the managers' requests.
    pub driven: bool,
}

/// The order of the managers' frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorldLogicSet {
    /// The runtime fills `CheckpointInputs` / `EventStartInputs` / `CarSwapInputs`.
    Inputs,
    /// Plain play: the managers step ([`step_plain`]); under `--hsm` the HSM's tick does.
    Step,
    /// The runtime carries out the outcomes.
    Outcomes,
}

impl Plugin for WorldLogicPlugin {
    fn build(&self, app: &mut App) {
        let driven = self.driven;
        app.configure_sets(Update, (WorldLogicSet::Inputs, WorldLogicSet::Step, WorldLogicSet::Outcomes).chain())
            .configure_sets(Update, WorldLogicSet::Inputs.before(crate::logic::LogicTick))
            .configure_sets(Update, WorldLogicSet::Outcomes.after(crate::logic::LogicTick))
            .add_systems(Startup, move |world: &mut World| {
                world.insert_non_send(WorldLogic::new(driven));
            })
            .add_systems(Startup, start_loading)
            .add_systems(Update, (receive, step_plain).chain().in_set(WorldLogicSet::Step));
    }
}

/// What the plain-play driver has entered.
#[derive(Debug, Default)]
pub struct PlainFlow {
    pub event_starts: bool,
    pub car_swap: bool,
    /// An event start chose an event: the manager waits for it (cleared by the event's run, or by `event_starts.rs`
    /// when the runtime cannot run it).
    pub awaiting_event: bool,
    /// The event's `RaceState` was seen while awaiting.
    pub event_seen: bool,
}

/// The managers' logic state and their outcomes (main thread: the state is shared with the HSM's Lua).
pub struct WorldLogic {
    pub state: Rc<RefCell<LogicState>>,
    /// `--hsm`: the HSM issues the requests and logic.rs routes the outcomes here.
    pub driven: bool,
    /// The managers' outcomes not yet carried out (`Request::Checkpoint` / `EventStart` / `CarSwap`).
    pub outbox: Rc<RefCell<Vec<Request>>>,
    /// The `GameLogicConfig` values (`None` until read).
    pub values: Option<GameLogicValues>,
    /// "Do Event Starts"'s parameters (plain play).
    pub event_start_params: Option<EventStartParams>,
    pub plain: PlainFlow,
}

impl WorldLogic {
    fn new(driven: bool) -> Self {
        let state: Rc<RefCell<LogicState>> = Rc::default();
        {
            // The game-mode manager's constructor sets +0x2e78 and +0x2e7a (`LogicHost::new`); one local player in
            // slot 0 with a vehicle (the runtime's player car).
            let mut s = state.borrow_mut();
            s.game_modes.won = true;
            s.game_modes.story_mode = true;
            s.players.local = Some(0);
            s.players.slots[0] = Some(Player { vehicle: 0, ..Default::default() });
        }
        WorldLogic {
            state,
            driven,
            outbox: Rc::default(),
            values: None,
            event_start_params: None,
            plain: PlainFlow::default(),
        }
    }

    /// The outcomes `keep` selects, taken out of the outbox in order.
    pub fn take(&self, keep: impl Fn(&Request) -> bool) -> Vec<Request> {
        let mut out = self.outbox.borrow_mut();
        let (taken, rest) = std::mem::take(&mut *out).into_iter().partition(|r| keep(r));
        *out = rest;
        taken
    }

    /// Plain play: carry out one of the managers' requests on the state as the logic host does
    /// (`LogicHost::apply`) and move the outcomes to the outbox.
    pub fn run(&self, r: &Request) {
        let mut st = self.state.borrow_mut();
        checkpoints::apply(&mut st, r);
        event_starts::apply(&mut st, r);
        car_swap::apply(&mut st, r);
        let queued = std::mem::take(&mut st.requests);
        let (outcomes, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|r| matches!(r, Request::Checkpoint(_) | Request::EventStart(_) | Request::CarSwap(_)));
        // Requests no runtime system carries out in plain play (input blocks: the logic keeps them itself).
        drop(rest);
        self.outbox.borrow_mut().extend(outcomes);
    }
}

/// What the loader thread reads.
struct Loaded {
    values: Option<GameLogicValues>,
    params: Option<EventStartParams>,
}

#[derive(Resource)]
struct Loading(Mutex<Receiver<Loaded>>);

fn start_loading(mut commands: Commands, game: Res<crate::Game>) {
    let store = game.store.clone();
    let (tx, rx) = channel();
    crate::platform::thread::spawn(move || {
        let values =
            store.start_config().map_err(|e| warn!("world logic: no GameLogicConfig ({e})")).ok().map(|c| c.logic);
        let params = store.hsm_data(HSM_BUNDLE).ok().and_then(|d| {
            let s = d.state(EVENT_STARTS_NODE)?;
            Some(EventStartParams::read(|name| s.parameter(name).unwrap_or_default().to_owned()))
        });
        if let Some(v) = &values {
            info!("world logic: GameLogicConfig {v:?}");
        }
        info!("world logic: event-start parameters {params:?}");
        let _ = tx.send(Loaded { values, params });
    });
    commands.insert_resource(Loading(Mutex::new(rx)));
}

fn receive(mut commands: Commands, loading: Option<Res<Loading>>, mut logic: NonSendMut<WorldLogic>) {
    let Some(loading) = loading else { return };
    let Ok(loaded) = loading.0.lock().unwrap().try_recv() else { return };
    commands.remove_resource::<Loading>();
    logic.values = loaded.values;
    logic.event_start_params = loaded.params;
}

/// Plain play: the free-drive machine's "Do Event Starts" and "Car Swap Logic" (enter, a step per frame, exit) while
/// no event is loaded. Under `--hsm` the HSM does this.
fn step_plain(mut logic: NonSendMut<WorldLogic>, race: Option<Res<crate::race::RaceState>>) {
    if logic.driven {
        return;
    }
    // The prompt bar is set again each frame by whoever wants it (`LogicHost::frame`).
    logic.state.borrow_mut().game_modes.prompt = None;
    let event = race.is_some();
    if logic.plain.awaiting_event {
        if event {
            logic.plain.event_seen = true;
        } else if logic.plain.event_seen {
            logic.plain.awaiting_event = false;
            logic.plain.event_seen = false;
        }
    }
    let free = !event && !logic.plain.awaiting_event;
    // EVENTSTARTS: entered once its parameters and the event list are in.
    let ready = logic.event_start_params.is_some() && !logic.state.borrow().event_starts.inputs.events.is_empty();
    if free && !logic.plain.event_starts && ready {
        let params = logic.event_start_params.expect("checked");
        logic.run(&Request::EventStartParams(params));
        logic.plain.event_starts = true;
    }
    if !free && logic.plain.event_starts {
        logic.run(&Request::EventStartsLeave);
        logic.plain.event_starts = false;
    }
    if logic.plain.event_starts {
        logic.run(&Request::EventStartsStep);
        if logic.state.borrow().event_starts.state.triggered {
            // `CHECKENTRY`: the state exits through `EXIT` (its exit removes the markers) and free drive goes to the
            // event.
            logic.run(&Request::EventStartsLeave);
            logic.plain.event_starts = false;
            logic.plain.awaiting_event = true;
        }
    }
    // HSMCARSWAPLO: entered once the spots are in.
    let spots = !logic.state.borrow().car_swap.inputs.spots.is_empty();
    if !event && !logic.plain.car_swap && spots {
        logic.run(&Request::CarSwapEnter);
        logic.plain.car_swap = true;
    }
    if event && logic.plain.car_swap {
        logic.run(&Request::CarSwapExit);
        logic.plain.car_swap = false;
    }
    if logic.plain.car_swap && logic.state.borrow().car_swap.inputs.started {
        logic.run(&Request::CarSwapStep);
    }
}
