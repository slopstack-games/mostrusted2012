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
//!   user actions every frame before the HSM update (`LogicHost::input`: Lua callbacks, `ExitGroupVia`, hooks);
//! - the song list (`PLAYSONGLIST` → `PlaySongList` / `StopSongList`): `music::SongListRequest` replaces the song
//!   manager's hand-written choice (the free-drive list 1622455 through `Query::DefaultEventSongList`, the current
//!   mode's `MusicPlaylist`); `free_drive` follows GMM+0x2e90;
//! - `PlayUiSound`: the `MiscSound.UI` sequence at the field plays (`ui_sound.rs`);
//! - `SimPause` (`SETSIMPAUSES`, `ALLOWSIMPAUS`, `BPAUSED`): the game clock (`Time<Virtual>`) pauses as the pause
//!   menu pauses it and resumes when the logic resumes what it paused; the HSM itself runs on the frame clock
//!   (its game clock stops meanwhile);
//! - `ShowWorld` (`HSB_RUNNING`): `FrontEnd::world_shown`; in driven mode the boot transition's `Screen.IsActive`
//!   also waits for it; `AdjustShowWorldCount` is the logic's count;
//! - `MuteSound`: counted per reason; while any counts the world's sound is muted (`FrontEnd::logic_mute`, besides
//!   the front end's own mute); `MuteRumble` and `SetFullRate` are kept (`Query::FullRate`), the runtime has no
//!   rumble and renders at the display's rate;
//! - `HideChevron`: the route markers (`chevron::ChevronsHidden`); `EnableTriggers`: the free-roam trigger hits
//!   (`freeroam::TriggersEnabled`, off until the logic enables them).
//! - **races** (decomp.md "Route free roam → race"): an event start in the runtime (map START RACE, Easy Drive,
//!   pause menu, world markers: `event_card::PlayEvent`; `--hsm-event` once free roam waits on the Easy Drive
//!   prompt) becomes the logic's event request: the free-roam prompt's input wait 908987 exits `EZ_RIGHT` and the
//!   "Easy Drive Menu" 1135029 calls the list item's hook `StartEventFromEasyDrive` (`lpcValue` = the event) and exits
//!   `TO_EVENT` ([`EventChoice`], as `mwtool hsm --event`); the free-drive machine ends, the GMM's event change asks
//!   `LoadEventData` (ready: the runtime has the event's `RaceSetup`) and starts "Race Game Mode" 730003. Its
//!   `RequestRespawnAtEventStart` (`Respawn` of the event) starts `race.rs` (`StartRace`: the car to the start, held
//!   until resident); the card (`event_card.rs`, the chosen menu's transition) hands its "TRANSITION" trigger to the
//!   logic; `RUNGAMELOGIC` of `Event.Data.EventIntro` / `EventOutro` (`PlaySequence`) starts the intro / outro
//!   (`cutscene.rs`, driven); the `CHECKPTS` requests are `race.rs`'s gates; when the GMM starts free drive again
//!   (`StartGameMode` of another event) `race.rs` ends the event (`RaceDriven`) and free roam resumes.
//!
//! Everything else (world, profile and feedback-group loads, the HUD screen's requests) is still logged only:
//! `freeroam.rs`, the rest of `race.rs` (stepping, AI, results) and of `frontend.rs` drive the game. Queries nobody
//! answers log their default once (`hsm: ... query X: default answer N`).
//!
//! What the runtime answers from its own state: the title sequence's general trigger "BOOT"
//! (`GeneralSequence.Triggers["BOOT"].IsTriggerSet[1]`, set by `frontend.rs` when the trigger item passes and
//! handed to the logic with `LogicState::set_general_trigger`), the
//! loading screen (`HSB_WAIT4LS` / `HSB_WAIT4NL`: `frontend::Phase::Loading`), the primary user bound by the
//! title's key press (`Query::PrimaryUserBound` while `ListenForPrimaryUser` listens), whether the profile is
//! brand new (`Query::NewProfile`: `--new-game` or no saved profile), and the progression / local player the boot
//! reads (loaded at start-up; the intro counts as played for a saved profile). `Mode.*` bindings are the game-mode
//! manager's own table (`mw_logic::natives::mode_view`); the runtime sets its world-loaded byte +0x2e77 from
//! `FrontEnd::ready`. Also: `UIMessages.NISRunning` (a cutscene plays, `cutscene.rs`), `Query::CollectablesReady`
//! (freeroam.rs read its triggers), `Query::FullRate`, `Query::DefaultEventSongList`. For the race (from
//! `race.rs`, `cutscene.rs`, `hud.rs`): `Query::CheckpointCount` / `CheckpointsLoaded` / `CheckpointsFinished` (the
//! event's gates, the player past the last), `Query::SequencePlaying` of the intro / outro playbacks (the cutscene
//! plays; the outro's also while the results after it run), `Event.Data.EventIntro` / `EventOutro` (the event's ids),
//! `Players.LocalPlayer.ResetInProgress` (the car held at the start), `Mode.HasIntroLoadCompleted` (GMM+0x2e82).
//!
//! Race stand-ins (model decisions): `Mode.HasIntroLoadCompleted` is set once the race is placed with its
//! opponents (what the original's intro load waits for was not traced); the "TRANSITION" general trigger stays set
//! from the card's item until the event ends (the original clears it when the item ends; the logic may reach its
//! wait after that here); `Players.LocalPlayer.WasBusted` is false (no police in the runtime's races); the outro's
//! playback also holds through the hand-written results (the offline results screen is not ported).
//!
//! Stand-ins (model decisions, the same answers `mwtool hsm --run` documents in docs/commands.md): the engine
//! bindings: none left but `Transitions.Transitioning` (`Net.*`, `UIConfig.*`, `SignInState.*`, `UISequences.*` and
//! `GeneralSequence.*` are the logic's own offline roots); state 1222788 `ConnectToAutolog` (online Autolog sign-in) exits
//! DONE at once; server operations (`HSM_SERV_OP`: the Blaze `PCLogin` the silent sign-in starts, since the
//! code's `IsChatRestricted` is false) exit FAILURE, as with no server; `Query::ModeWon` is the logic's GMM+0x2e78 (1
//! while a mode runs, as the manager's constructor and reset set it; `FUN_006f46e0`'s offline derivation is not ported); behaviour types that are not
//! ported yet hold (run their children, never exit); hooks that are not ported answer EXIT (most `CALLHSMHOOK` nodes
//! go on on EXIT).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
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

use crate::cutscene::{CutsceneState, Which};
use crate::frontend::{FrontEnd, Phase};
use crate::race::{RaceDriven, RaceSetup, RaceState, RacerProgress, StartRace};

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
    /// `--hsm-event`: this event is chosen (as the map's START RACE chooses it) once free roam is reached.
    pub event: Option<Arc<RaceSetup>>,
    /// `--new-game`, or no saved profile was loaded: the profile is brand new (`ISNEWPROFILE` → YES) and the
    /// intro is not marked played.
    pub new_profile: bool,
}

impl Plugin for LogicPlugin {
    fn build(&self, app: &mut App) {
        if !self.enabled {
            return;
        }
        // The systems the logic's requests drive (absent without `--hsm`: those systems keep their hand-written
        // rules).
        app.insert_resource(Setup { new_profile: self.new_profile, pending: None })
            .insert_resource(crate::music::SongListRequest::default())
            .insert_resource(crate::chevron::ChevronsHidden(false))
            .insert_resource(crate::freeroam::TriggersEnabled(false))
            .insert_resource(RaceDriven::default())
            .insert_resource(HsmEvent(self.event.clone()))
            .add_systems(Startup, |mut c: ResMut<CutsceneState>| c.logic_driven = true)
            .add_systems(Startup, start_loading)
            .add_systems(Update, (receive, choose_event, tick, race_shots).chain());
    }
}

/// `--hsm-event`: the event to choose once free roam is reached (taken when sent).
#[derive(Resource)]
struct HsmEvent(Option<Arc<RaceSetup>>);

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

/// `MUTEREASON`s (`MUTE_SOUND`'s table at 0x010c5c28 has 3, `MUTEALLRUMBL`'s at 0x010c5c10 has 6).
const MUTE_REASONS: usize = 6;

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
    /// The logic paused the game clock (`SimPause`) and has not resumed it yet.
    sim_paused: bool,
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
    let choice = Arc::new(AtomicU32::new(0));
    let registry = registry(&data, &choice);
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
    let playlists = events.game_modes.iter().map(|m| (m.id, m.music_playlist)).collect();
    let mut host = RuntimeHost {
        data,
        playlists,
        full_rate: false,
        mute: [0; MUTE_REASONS],
        rumble_mute: [false; MUTE_REASONS],
        collectables_ready: false,
        nis_running: false,
        defaults: HashSet::new(),
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
        race: RaceLink { choice, ..Default::default() },
    };
    let Some(serial) = hsm.start(TOP_STATE, &mut host) else {
        warn!("hsm: no top-level state {TOP_STATE}");
        return;
    };
    info!("hsm: machine {serial} started at {TOP_STATE} (shadow mode)");
    let store = world.resource::<crate::Game>().store.clone();
    world.insert_non_send(Logic {
        hsm,
        host,
        serial,
        frame: 0,
        screens: channel(),
        sequences: channel(),
        store,
        sim_paused: false,
    });
}

/// The server operations (`HSM_SERV_OP`: Blaze / Origin logins, entitlements, uploads).
const SERVER_OPERATION: &str = "HSM_SERV_OP";

/// The structural and ported behaviour types; types the original has but we have not ported hold, the online
/// Autolog state exits DONE and server operations exit FAILURE. The free-roam Easy Drive prompt's input wait and
/// the Easy Drive menu take the runtime's event choice ([`EventChoice`]).
fn registry(data: &HsmData, choice: &Arc<AtomicU32>) -> Registry {
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
        let server = !ported && name == SERVER_OPERATION;
        registry.register_id(
            b,
            &name,
            Arc::new(move |s: &State| {
                if s.id == CONNECT_TO_AUTOLOG {
                    Box::new(Stub { exit: Some(key("DONE")) })
                } else if ported {
                    orig.create_for(s).expect("registered")
                } else if server {
                    Box::new(Stub { exit: Some(key("FAILURE")) })
                } else {
                    Box::new(Stub { exit: None })
                }
            }),
        );
    }
    for id in [EZ_PROMPT_INPUT, EASY_DRIVE_MENU] {
        let Some(b) = data.state(id).map(|s| s.behaviour) else { continue };
        let (orig, choice) = (registry.clone(), choice.clone());
        let name = registry.code(b).unwrap_or("?").to_owned();
        registry.register_id(
            b,
            &name,
            Arc::new(move |s: &State| {
                let inner = orig.create_for(s).unwrap_or_else(|| Box::new(Stub { exit: None }));
                if s.id == EZ_PROMPT_INPUT || s.id == EASY_DRIVE_MENU {
                    Box::new(EventChoice { state: s.id, choice: choice.clone(), inner, choosing: false })
                } else {
                    inner
                }
            }),
        );
    }
    registry
}

/// "Wait for UI Input" (`HSB_WAIT4IN`) of the free-roam Easy Drive prompt ("Run Child - Wait for EZDrive Input
/// Prompt" 1445535 leaves through `EZ_SELECT` on its `EZ_RIGHT`).
const EZ_PROMPT_INPUT: u32 = 908_987;
/// "Easy Drive Menu" (`HSB_RUNCHILD` under the Easy Drive widget group 1135023): its event list's item calls the
/// hook `StartEventFromEasyDrive` and the state exits `TO_EVENT` (→ 908973 `START_EVENT` → the free-drive HUD
/// machine ends `START_EVENT`, decomp.md "Route free roam → race").
const EASY_DRIVE_MENU: u32 = 1_135_029;

/// The runtime's event choice made the logic's (stand-in for the Easy Drive widgets' input and list item, which the
/// runtime draws with its own menus: map START RACE, Easy Drive, pause menu, world markers all send
/// `event_card::PlayEvent`): with an event chosen, 908987 exits `EZ_RIGHT` (the prompt opens the menu) and
/// 1135029 calls the list item's hook with the event (`lpcValue`) and exits `TO_EVENT`, as `mwtool hsm --event`
/// does; otherwise both are the original behaviours.
struct EventChoice {
    state: u32,
    choice: Arc<AtomicU32>,
    inner: Box<dyn mw_logic::hsm::Behaviour>,
    /// 1135029 entered with a choice: it is taken on the first update.
    choosing: bool,
}

impl mw_logic::hsm::Behaviour for EventChoice {
    fn on_enter(&mut self, cx: &mut mw_logic::hsm::Cx) {
        self.choosing = self.state == EASY_DRIVE_MENU && self.choice.load(Ordering::Relaxed) != 0;
        if !self.choosing {
            self.inner.on_enter(cx);
        }
    }
    fn enter_update(&mut self, cx: &mut mw_logic::hsm::Cx) -> bool {
        self.choosing || self.inner.enter_update(cx)
    }
    fn on_running(&mut self, cx: &mut mw_logic::hsm::Cx) {
        if !self.choosing {
            self.inner.on_running(cx);
        }
    }
    fn update(&mut self, cx: &mut mw_logic::hsm::Cx) -> bool {
        let event = self.choice.load(Ordering::Relaxed);
        match self.state {
            EZ_PROMPT_INPUT if event != 0 => {
                cx.set_exit(key("EZ_RIGHT"));
                true
            }
            EASY_DRIVE_MENU if self.choosing => {
                self.choice.store(0, Ordering::Relaxed);
                let args = Json::parse(&format!(r#"{{ "lpcValue": "{event}" }}"#)).expect("json");
                cx.host.hook("StartEventFromEasyDrive", Some(&args));
                cx.set_exit(key("TO_EVENT"));
                true
            }
            _ => self.inner.update(cx),
        }
    }
    fn exit_update(&mut self, cx: &mut mw_logic::hsm::Cx) -> bool {
        self.choosing || self.inner.exit_update(cx)
    }
    fn on_exit_begin(&mut self, cx: &mut mw_logic::hsm::Cx) {
        if !self.choosing {
            self.inner.on_exit_begin(cx);
        }
    }
    fn on_exit(&mut self, cx: &mut mw_logic::hsm::Cx) {
        if !self.choosing {
            self.inner.on_exit(cx);
        }
    }
}

/// `MW_HSM_SHOTS=<dir>`: screenshots along a race the logic runs (headless checks): `card` 3 s into the event card,
/// `intro` 6 s into the intro, `race` 10 s after the go, `outro` 5 s into the outro, `freeroam` 4 s after the event
/// ended; then exit (real seconds).
#[allow(clippy::type_complexity)]
fn race_shots(
    mut commands: Commands,
    real: Res<Time<Real>>,
    card: Option<Res<crate::event_card::EventCard>>,
    cutscene: Res<CutsceneState>,
    race: Option<Res<RaceState>>,
    mut state: Local<(usize, Option<f32>, bool)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = std::env::var_os("MW_HSM_SHOTS").map(std::path::PathBuf::from) else { return };
    let (step, since, raced) = &mut *state;
    let now = real.elapsed_secs();
    let racing = race.as_ref().is_some_and(|r| r.phase == crate::race::RacePhase::Racing);
    *raced |= race.is_some();
    // What each step waits for, and how long after it began.
    let (on, wait, name) = match *step {
        0 => (card.as_ref().is_some_and(|c| c.showing()), 3.0, "card"),
        1 => (cutscene.playing() && !cutscene.finished(Which::Intro), 6.0, "intro"),
        2 => (racing && !cutscene.playing(), 10.0, "race"),
        3 => (cutscene.playing() && cutscene.finished(Which::Intro), 5.0, "outro"),
        4 => (*raced && race.is_none(), 4.0, "freeroam"),
        5 => (true, 1.0, ""),
        _ => return,
    };
    if !on {
        *since = None;
        return;
    }
    if now - *since.get_or_insert(now) < wait {
        return;
    }
    *since = None;
    *step += 1;
    if name.is_empty() {
        exit.write(AppExit::Success);
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}-{name}.png", *step));
    info!("hsm: screenshot {}", path.display());
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
}

/// `--hsm-event`: once the logic's free roam waits on the Easy Drive prompt (and the front end is playing), the event
/// is chosen as the map's START RACE chooses it.
fn choose_event(
    logic: Option<NonSend<Logic>>,
    mut event: ResMut<HsmEvent>,
    front: Option<Res<FrontEnd>>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
) {
    let (Some(logic), Some(setup)) = (logic, &event.0) else { return };
    if front.is_some_and(|f| f.phase != Phase::Playing) {
        return;
    }
    let waiting = logic.hsm.machines().any(|(m, _)| logic.hsm.active(m).iter().any(|&(_, s, _)| s == EZ_PROMPT_INPUT));
    if waiting {
        info!("hsm: {:5} --hsm-event: {} ({}) chosen in free roam", logic.frame, setup.name, setup.event.id);
        play.write(crate::event_card::PlayEvent(setup.clone()));
        event.0 = None;
    }
}

/// The systems the requests drive.
#[derive(bevy::ecs::system::SystemParam)]
struct Targets<'w, 's> {
    front: Option<ResMut<'w, FrontEnd>>,
    songs: Option<ResMut<'w, crate::music::SongListRequest>>,
    ui_sounds: Option<ResMut<'w, crate::ui_sound::UiSounds>>,
    chevrons: Option<ResMut<'w, crate::chevron::ChevronsHidden>>,
    triggers: Option<ResMut<'w, crate::freeroam::TriggersEnabled>>,
    collectables: Option<Res<'w, crate::freeroam::Collectables>>,
    cutscene: Option<ResMut<'w, CutsceneState>>,
    sim: ResMut<'w, Time<Virtual>>,
    race: Race<'w, 's>,
}

/// The race the logic drives (`race.rs`, `event_card.rs`, `cutscene.rs` in driven mode).
#[derive(bevy::ecs::system::SystemParam)]
struct Race<'w, 's> {
    state: Option<Res<'w, RaceState>>,
    driven: Option<ResMut<'w, RaceDriven>>,
    player: bevy::ecs::system::Query<'w, 's, &'static RacerProgress, With<crate::drive::Player>>,
    card: Option<Res<'w, crate::event_card::EventCard>>,
    hud: Option<Res<'w, crate::hud::HudState>>,
    loaded: Res<'w, crate::Loaded>,
    plays: MessageReader<'w, 's, crate::event_card::PlayEvent>,
    start: MessageWriter<'w, StartRace>,
}

fn tick(logic: Option<NonSendMut<Logic>>, real: Res<Time<Real>>, mut t: Targets, menu: Res<crate::input::MenuInput>) {
    let Some(mut logic) = logic else { return };
    // The HSM runs on the frame clock while the simulation is paused (it is what resumes it); its game clock stops
    // then (`Clock::game` None). The frame step is capped as the game clock is (main.rs: 2/60 s).
    let dt = real.delta_secs().min(t.sim.max_delta().as_secs_f32());
    if dt <= 0.0 {
        return;
    }
    let game_dt = (!t.sim.is_paused()).then(|| t.sim.delta_secs());
    let front = &mut t.front;
    let Logic { hsm, host, serial, frame, screens, sequences, store, .. } = &mut *logic;
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
        // The screen transitions playing: the boot transition, the event card (the chosen menu's `PLAY_EVENT`
        // transition, `event_card.rs`).
        host.transitioning = front.phase == Phase::Transition
            || host.carry.iter().any(|c| matches!(c, Carry::BootTransition))
            || t.race.card.as_ref().is_some_and(|c| c.showing());
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
    host.collectables_ready = t.collectables.as_ref().is_some_and(|c| c.ready());
    host.nis_running = t.cutscene.as_ref().is_some_and(|c| c.playing());
    // The runtime's event choice (map START RACE, Easy Drive, pause menu, world markers) becomes the logic's.
    if let Some(crate::event_card::PlayEvent(setup)) = t.race.plays.read().last().cloned() {
        let id = setup.event.id;
        if host.race.setup.is_some() || crate::dlc::event_locked(&setup.event).is_some() {
            info!("hsm: {:5} event {id} chosen: not taken (an event runs, or DLC)", host.frame);
        } else {
            info!("hsm: {:5} event {id} chosen in the runtime: the logic's event request", host.frame);
            host.race.choice.store(id, Ordering::Relaxed);
            host.race = RaceLink { choice: host.race.choice.clone(), setup: Some(setup), ..Default::default() };
        }
    }
    host.race.observe(&t.race, t.cutscene.as_deref(), &host.logic);
    host.logic.frame(dt);
    step_playbacks(host, dt);
    // The widgets' user actions (before the HSM's update, which reads the group exit they set).
    host.logic.input(&|id| {
        let (pressed, repeat, value) = menu.by_id(id).unwrap_or_default();
        ActionInput { pressed, repeat, value }
    });
    let was_running = hsm.is_running(*serial);
    hsm.update(host, Clock { frame: dt, game: game_dt });
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
    let free_drive = host.logic.state.borrow().game_modes.freedrive_event;
    let carry = std::mem::take(&mut host.carry);
    let muted = host.mute.iter().any(|&n| n > 0);
    for r in carry {
        match r {
            Carry::Title => front.as_deref_mut().map_or((), |f| f.start_title()),
            Carry::StopTitle => front.as_deref_mut().map_or((), |f| f.stop_title()),
            Carry::BootTransition => front.as_deref_mut().map_or((), |f| f.start_transition()),
            Carry::ShowWorld(show) => {
                if let Some(f) = front.as_deref_mut() {
                    f.world_shown = show;
                }
            }
            Carry::SongList { list, allow_playout } => {
                if let Some(songs) = t.songs.as_deref_mut() {
                    songs.list = list;
                    songs.allow_playout = allow_playout;
                }
            }
            Carry::UiSound(field) => {
                if let Some(ui) = t.ui_sounds.as_deref_mut() {
                    ui.play(field);
                }
            }
            Carry::SimPause(paused) => {
                // As the pause menu does (pause.rs): the game clock stops; resumed only by whoever paused it.
                if paused && !t.sim.is_paused() {
                    t.sim.pause();
                    logic.sim_paused = true;
                } else if !paused && std::mem::take(&mut logic.sim_paused) {
                    t.sim.unpause();
                }
            }
            Carry::HideChevron(hide) => {
                if let Some(c) = t.chevrons.as_deref_mut() {
                    c.0 = hide;
                }
            }
            Carry::EnableTriggers(on) => {
                if let Some(e) = t.triggers.as_deref_mut() {
                    e.0 = on;
                }
            }
            Carry::StartRace(setup) => {
                if let Some(c) = t.cutscene.as_deref_mut() {
                    c.reset();
                }
                t.race.start.write(StartRace(setup));
            }
            Carry::PlayCutscene(which) => {
                if let Some(c) = t.cutscene.as_deref_mut() {
                    match which {
                        Which::Intro => c.logic_intro = true,
                        Which::Outro => c.logic_outro = true,
                    }
                }
            }
            Carry::EndRace => {
                if let Some(d) = t.race.driven.as_deref_mut() {
                    d.end = true;
                }
            }
            Carry::Play { .. } | Carry::Nothing => {}
        }
    }
    if let Some(songs) = t.songs.as_deref_mut()
        && songs.free_drive != free_drive
    {
        songs.free_drive = free_drive;
    }
    // `FUN_004b8b20`: the world's sound is muted while any `MUTE_SOUND` reason counts.
    if let Some(f) = t.front.as_deref_mut()
        && f.logic_mute != muted
    {
        f.logic_mute = muted;
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
    /// `PLAYSONGLIST` / its exit: the song manager's list (`music::SongListRequest`; 0 = stopped).
    SongList { list: u32, allow_playout: bool },
    /// `PlayUiSound`: `ui_sound.rs`.
    UiSound(u32),
    /// `SETSIMPAUSES` / `ALLOWSIMPAUS` / `BPAUSED`: the game clock (`Time<Virtual>`) paused / resumed.
    SimPause(bool),
    /// `HSB_RUNNING`: the world shown / hidden (`FrontEnd::world_shown`).
    ShowWorld(bool),
    /// `HIDECHEV`: the route markers (`chevron::ChevronsHidden`).
    HideChevron(bool),
    /// `ENABLETRIGS`: the free-roam trigger hits (`freeroam::TriggersEnabled`).
    EnableTriggers(bool),
    /// `RequestRespawnAtEventStart` of the chosen event: `race::StartRace` (the car to the start, the race loads).
    StartRace(Arc<RaceSetup>),
    /// `RUNGAMELOGIC` of `Event.Data.EventIntro` / `EventOutro`: `cutscene.rs` plays it.
    PlayCutscene(Which),
    /// The logic's mode machine went back to free drive: `race.rs` ends the event.
    EndRace,
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
    /// Each game mode's `MusicPlaylist` (`Query::DefaultEventSongList`).
    playlists: HashMap<u32, u32>,
    /// `CHANGEVSYNCS`: G+0x2f723, the 60 Hz mode (`Query::FullRate`).
    full_rate: bool,
    /// `MUTE_SOUND`: the count per reason (UIM+0x2d2f0 +2999).
    mute: [u32; MUTE_REASONS],
    /// `MUTEALLRUMBL`: the flag per reason (G+0x2f480 +0x325f0 +0x260).
    rumble_mute: [bool; MUTE_REASONS],
    /// The free-roam collectables are read (`Query::CollectablesReady`).
    collectables_ready: bool,
    /// A cutscene (NIS) plays (`cutscene.rs`; `UIMessages.NISRunning`).
    nis_running: bool,
    /// Queries answered with their default (logged once each).
    defaults: HashSet<String>,
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
    race: RaceLink,
}

/// The general trigger item of the event card ([`crate::event_card::EventCard::transition`]) that the EventIntro
/// context's "Screen Transition Voodoo" waits for (`GeneralSequence.Triggers["TRANSITION"].IsTriggerSet[1]`).
const TRANSITION_TRIGGER_NAME: &str = "TRANSITION";

/// The event the runtime chose and the logic runs: its requests for the race and the runtime's answers (module doc).
#[derive(Default)]
struct RaceLink {
    /// The chosen event not yet taken by the logic's Easy Drive menu ([`EventChoice`]); 0 = none.
    choice: Arc<AtomicU32>,
    /// The chosen event's set-up, from the choice until the logic is back in free drive.
    setup: Option<Arc<RaceSetup>>,
    /// The logic asked for the respawn at the event's start: `race.rs` runs it.
    started: bool,
    /// `RUNGAMELOGIC`'s playbacks of the event's intro and outro.
    intro_playback: Option<u32>,
    outro_playback: Option<u32>,
    /// The card's "TRANSITION" was handed to the logic.
    transition: bool,
    // The runtime's answers, refreshed every frame ([`RaceLink::observe`]).
    /// `race.rs` has the event (`RaceState`), and for how many frames (the AI sets the race up on its first).
    loaded: bool,
    loaded_frames: u32,
    /// The player's car is held at the start until it is resident (`RaceState::placing`).
    placing: bool,
    /// The race is ready to show: placed, opponents placed (`RaceState::hold`), the world loaded.
    ready: bool,
    gates: usize,
    /// The player passed the last checkpoint.
    finished: bool,
    intro_done: bool,
    /// The outro is over and so are the results after it (`hud.rs`).
    outro_done: bool,
}

impl RaceLink {
    fn observe(&mut self, r: &Race, cutscene: Option<&CutsceneState>, logic: &LogicHost) {
        if self.setup.is_none() {
            return;
        }
        let race = r.state.as_deref().filter(|_| self.started);
        self.loaded = race.is_some();
        self.loaded_frames = if self.loaded { self.loaded_frames + 1 } else { 0 };
        self.placing = self.started && race.is_none_or(|r| r.placing);
        self.ready = race.is_some_and(|r| !r.placing && !r.hold) && r.loaded.done && self.loaded_frames > 2;
        self.gates = race.map_or(0, |r| r.checkpoints.len());
        self.finished = race.is_some() && r.player.iter().any(|p| p.finished.is_some());
        let finished = |w| cutscene.is_none_or(|c| c.finished(w));
        self.intro_done = finished(Which::Intro);
        let results_over = r.hud.as_ref().is_none_or(|h| {
            h.results_clock.is_some_and(|t| t >= crate::race::RESULTS_SECS)
                || race.is_some_and(|r| r.phase != crate::race::RacePhase::Finished)
        });
        self.outro_done = finished(Which::Outro) && results_over;
        // `Mode.HasIntroLoadCompleted` (+0x2e82): the event is loaded and placed (model decision, module doc).
        let mut st = logic.state.borrow_mut();
        if self.ready && st.game_modes.machine.is_some() && st.game_modes.current_event == self.event() {
            st.game_modes.intro_load_complete = true;
        }
        // The card's "TRANSITION" (set from the item on until the event's end: model decision, module doc).
        if !self.transition && r.card.as_ref().is_some_and(|c| c.transition()) {
            self.transition = true;
            st.set_general_trigger(mw_logic::binding::hash(TRANSITION_TRIGGER_NAME), 1);
        }
    }

    fn event(&self) -> u32 {
        self.setup.as_ref().map_or(0, |s| s.event.id)
    }

    /// The event's own data the logic reads (`Event.Data.*`, `Players.LocalPlayer.*` of the race).
    fn binding(&self, path: &str, current_event: u32) -> Option<String> {
        let setup = self.setup.as_ref()?;
        let bool = |b: bool| if b { "true" } else { "false" }.to_owned();
        let current = current_event == setup.event.id;
        Some(match path {
            "Event.Data.EventIntro" if current => setup.event.event_intro.to_string(),
            "Event.Data.EventOutro" if current => setup.event.event_outro.to_string(),
            // The respawn at the event's start: `race.rs` holds the car there until the start is resident.
            "Players.LocalPlayer.ResetInProgress(NOT)" if self.started => bool(!self.placing),
            "Players.LocalPlayer.ResetInProgress" if self.started => bool(self.placing),
            // No busting in the runtime's races (police are off in them).
            "Players.LocalPlayer.WasBusted" if current => bool(false),
            _ => return None,
        })
    }
}

impl RuntimeHost {
    /// Engine bindings the logic host has no root for yet (the game-mode manager's `Mode` table, the
    /// transitions): the runtime's own state where it has it, else the stand-ins of the module doc.
    fn engine_binding(&self, path: &str) -> Option<String> {
        let bool = |b: bool| if b { "true" } else { "false" };
        Some(
            match path {
                "Transitions.Transitioning(NOT)" => bool(!self.transitioning),
                // The UI message manager's NIS flag: a cutscene plays (`cutscene.rs`).
                "UIMessages.NISRunning" => bool(self.nis_running),
                "UIMessages.NISRunning(NOT)" => bool(!self.nis_running),
                _ => {
                    let current = self.logic.state.borrow().game_modes.current_event;
                    return self.race.binding(path, current);
                }
            }
            .to_owned(),
        )
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
            Some(v) => Some(v),
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
            Request::PlaySongList { list, allow_playout } => {
                Some(Carry::SongList { list: u32::try_from(*list).unwrap_or(0), allow_playout: *allow_playout })
            }
            Request::StopSongList => Some(Carry::SongList { list: 0, allow_playout: false }),
            Request::PlayUiSound { field } => Some(Carry::UiSound(*field)),
            Request::SimPause { paused } => Some(Carry::SimPause(*paused)),
            Request::ShowWorld { show } => Some(Carry::ShowWorld(*show)),
            Request::HideChevron { hide } => Some(Carry::HideChevron(*hide)),
            Request::EnableTriggers(on) => Some(Carry::EnableTriggers(*on)),
            // The count is the logic's (`mode_machine::apply`); `Query::ShowWorldCount` reads it.
            Request::AdjustShowWorldCount { .. } => Some(Carry::Nothing),
            Request::SetFullRate(on) => {
                // Kept for `Query::FullRate`; the runtime renders every frame at the display's rate already.
                self.full_rate = *on;
                Some(Carry::Nothing)
            }
            Request::MuteSound { reason, on } => {
                let n = &mut self.mute[usize::from(*reason).min(MUTE_REASONS - 1)];
                *n = if *on { *n + 1 } else { n.saturating_sub(1) };
                Some(Carry::Nothing)
            }
            Request::MuteRumble { reason, on } => {
                // Kept as the original keeps it; the runtime has no controller rumble to mute.
                self.rumble_mute[usize::from(*reason).min(MUTE_REASONS - 1)] = *on;
                Some(Carry::Nothing)
            }
            Request::PlayWidgetGroupTransition { group, .. } if *group == PRESS_START_GROUP => {
                self.transitioning = true;
                Some(Carry::BootTransition)
            }
            // The chosen event's data is in once the runtime has its set-up (`RaceSetup`); events the runtime did
            // not choose (free drive) at once, as before.
            Request::LoadEventData { event } => {
                let ready = self.race.setup.as_ref().is_none_or(|s| s.event.id == *event)
                    || mode_machine::is_freedrive_event(&self.logic.state.borrow(), *event);
                self.logic.state.borrow_mut().game_modes.event_data_ready = ready;
                Some(Carry::Nothing)
            }
            Request::Respawn { event, .. } if *event != 0 && *event == self.race.event() && !self.race.started => {
                self.race.started = true;
                // The reset is pending from the request on (`race.rs` puts the car down once the start is in).
                self.race.placing = true;
                self.race.ready = false;
                self.race.setup.clone().map(Carry::StartRace)
            }
            Request::PlaySequence { playback, sequence, .. }
                if self.race.setup.as_ref().is_some_and(|s| *sequence == s.event.event_intro) =>
            {
                self.race.intro_playback = Some(*playback);
                Some(Carry::PlayCutscene(Which::Intro))
            }
            Request::PlaySequence { playback, sequence, .. }
                if self.race.setup.as_ref().is_some_and(|s| *sequence == s.event.event_outro) =>
            {
                self.race.outro_playback = Some(*playback);
                Some(Carry::PlayCutscene(Which::Outro))
            }
            // The race's gates are `race.rs`'s (stepped every tick); the logic reads them through its queries.
            Request::CheckpointsEnter
            | Request::CheckpointsPlace
            | Request::CheckpointsStep
            | Request::CheckpointsLeave
                if self.race.started =>
            {
                Some(Carry::Nothing)
            }
            // Free drive again after the race: the event ends (`race.rs`).
            Request::StartGameMode { event } if self.race.started && *event != self.race.event() => {
                let name = mw_logic::binding::hash(TRANSITION_TRIGGER_NAME);
                self.logic.state.borrow_mut().clear_general_trigger(name, 1);
                self.race = RaceLink { choice: self.race.choice.clone(), ..Default::default() };
                Some(Carry::EndRace)
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
        // Requests made every frame are logged once.
        let every_frame = matches!(r, Request::CheckpointsStep);
        if done && (!every_frame || *n == 1) {
            info!("hsm: {:5} request {r:?}", self.frame);
        } else if !done && *n == 1 {
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
            Query::FullRate => Some(self.full_rate as u64),
            // `PLAYSONGLIST`'s last fallback: the current mode's `MusicPlaylist` (decomp.md "Music").
            Query::DefaultEventSongList => {
                let mode = self.logic.state.borrow().game_modes.current_mode;
                Some(u64::from(self.playlists.get(&mode).copied().unwrap_or(0)))
            }
            // `COLLECTABLES`: the world state the collectables need (freeroam.rs has read the triggers).
            Query::CollectablesReady => Some(self.collectables_ready as u64),
            // The event's intro / outro (`cutscene.rs`; the outro also holds through the results after it).
            Query::SequencePlaying { playback } if self.race.intro_playback == Some(*playback) => {
                Some(!self.race.intro_done as u64)
            }
            Query::SequencePlaying { playback } if self.race.outro_playback == Some(*playback) => {
                Some(!self.race.outro_done as u64)
            }
            // `CHECKPTS`: the race's gates (`race.rs`).
            Query::CheckpointCount if self.race.started => Some(self.race.gates as u64),
            Query::CheckpointsLoaded if self.race.started => Some(self.race.loaded as u64),
            Query::CheckpointsFinished if self.race.started => Some(self.race.finished as u64),
            // The logic answers GMM+0x2e78 (1 while a mode runs: the manager's constructor / reset); the fallback
            // (free drive) is only for a logic without that byte. `FUN_006f46e0`'s offline derivation is not ported.
            Query::ModeWon => {
                Some(self.logic.query(q).unwrap_or_else(|| self.logic.state.borrow().game_modes.freedrive_event as u64))
            }
            _ => {
                let a = self.logic.query(q);
                if a.is_none() {
                    let kind = format!("{q:?}");
                    let kind = kind.split([' ', '(', '{']).next().unwrap_or_default().to_owned();
                    if self.defaults.insert(kind) {
                        info!("hsm: {:5} query {q:?}: default answer {}", self.frame, q.default_answer());
                    }
                }
                a
            }
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
