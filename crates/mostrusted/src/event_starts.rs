//! Free-roam event starts: the panels over each event's start point, their minimap diamonds and starting an
//! event with a burnout there (research log 2026-10-19 "event starts, cop arrows, search circle").
//!
//! The game-logic HSM's "Do Event Starts" behaviour (nodes 1560429 / 816214, vtable 0x00fd950c: enter
//! `vt_00754f30`, update `FUN_00755220`) reads its parameters by name: `POIID` 1013512 (an event),
//! `MWPOIIDC` 955952 (a Most Wanted race not yet won in first place, `FUN_006e3a40`), `MWPOIIDUC` 955955 (one
//! won), `RECPOIID` 1132924 (an Autolog recommendation, not ported), `ACTIVERADIUS` 2000, `RSPCTRADIUS` true,
//! `CHECKENTRY` true, `TRACKEDONLY` false.
//! - **Which events** (`FUN_0074a600`, rebuilt when the driven car changes, a pursuit ends or the reference
//!   point moved 10 m, `.rdata` 0x00fd6790 = 100 squared): the world's event list in order, offline events
//!   (type 390131) whose game mode is not `GameLogicConfig.1b3c1ddc` (+0xd8, the freedrive events) and whose
//!   `AllowedVehicleList` holds the driven car (`FUN_006eca10`; no list = allowed), within 2000 m of the
//!   reference point (Most Wanted races not yet won: at any distance), at most 20 (0x14). Each gets a point of
//!   interest at its start trigger (`FUN_006e56f0`, `FUN_0053c020`) with yaw 0 (`FUN_00793350`).
//! - **None in a pursuit** (`FUN_006c2410`: the points are removed, `FUN_0074aaf0`, and come back after).
//! - **Starting** (`FUN_0074ad50`, only below `GameLogicConfig.ff4aef6f` (+0x284) = 70 mph): within
//!   `GameLogicConfig.49caf423` (+0x244) = 22 m of a start, the height counted 5× (`.rdata` 0x00fd67b4), the
//!   prompt `GameLogicConfig.PromptEvent` (+0x160) = 18364 "HOLD … + … TO START EVENT" shows (game-mode manager
//!   +0x2c70 / +0x2ca7, the prompt bar); throttle and brake both above 0.5 (input record +0x30 / +0x3c,
//!   `FUN_006c1a80` reads them as throttle and brake; 0x00f8a810) add the frame time to a hold timer, which
//!   resets when they are let go; above 1.0 s (0x00f8a820) the event is set as the next one (`FUN_006e74f0`),
//!   as the map's START RACE does: here [`crate::event_card::PlayEvent`].
//! - **Looks** (the free-roam HUD's `Widget_PointOfInterest`, screen 371621): each look's map icon (the green
//!   diamond `0f789a`, the Most Wanted red one `0e962f`) is drawn by `hud.rs`; its world layouts here: the
//!   panel 1454873 (difficulty, name, type, rewards, the Most Wanted car) on a bracket, shown while
//!   `Widget.POI.ShowSpeedwall` is false, faded out beyond 200 m and outside 5–140 m from the camera (instance
//!   colour transform and timeline), scaled 0.09 m per pixel × 0.4…1 between 20 and 150 m. The look's flag
//!   `d788383f` turns it to the camera (the panel faces the camera from every side in the captures).
//!
//! **Runtime on the ported manager** (research log 2026-11-18): `fill_inputs` supplies `EventStartInputs`
//! (`mw_logic::natives::event_starts`, run by `world_logic.rs`: the list, the 2000 m radius, the pursuit, the prompt
//! and the burnout hold are the manager's); `apply_outcomes` draws its `shown` list, shows the prompt it set
//! (GMM+0x2c70) and, in plain play, starts `EventStartState::chosen` as a [`crate::event_card::PlayEvent`].
//!
//! Model decisions (not traced; research log): the reference point (UI manager +0x2cd50) is the player's car;
//! no event counts as won or recommended (no results are kept), every event as unlocked for the profile
//! (`FUN_00688f60`, `FUN_0068f190`, `FUN_006f0b60`, `FUN_006e58a0` not ported); the panel turns about the
//! vertical only; the rewards (`AvailableReward`, `EventReward1/2`) are not modelled and print nothing; the
//! prompt's keys are the original's keyboard defaults (Z brake, A accelerate) or LT / RT with a pad; markers show
//! in free roam and after a race has finished, not while one runs.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use mw_assets::ui::{PlacedLayout, UiAnim, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::BindValue;
use mw_formats::ui::LayoutInstance;

use crate::drive::{Car, CarInput, Player};
use crate::gate_marker::{WorldLayoutOut, draw_world_layout};
use crate::race::RaceSetup;
use crate::world_logic::{WorldLogic, WorldLogicSet};
use crate::world_ui::WorldUiQuads;
use mw_logic::natives::Request;
use mw_logic::natives::event_starts::StartEvent;

/// `POIID`: an event's start.
pub const POI_EVENT: u32 = 1_013_512;
/// `MWPOIIDC`: a Most Wanted race not yet won in first place.
pub const POI_MOST_WANTED: u32 = 955_952;
/// `MWPOIIDUC`: a Most Wanted race already won.
pub const POI_MOST_WANTED_WON: u32 = 955_955;
/// The speed run and ambush game modes (`GameLogicConfig` +0x1ec / +0x60; the hint only).
const SPEED_RUN_MODE: u32 = 955_569;
const AMBUSH_MODE: u32 = 390_185;
/// Seconds of holding that start the event (`.rdata` 0x00f8a820).
const HOLD_SECONDS: f32 = 1.0;
/// `GameLogicConfig` (GLOBALCONFIG.BNDL) and its fields the code reads.
const GAME_LOGIC_CONFIG: ResourceId = ResourceId(0x0100_0000_0004_5e6f);
const FIELD_START_RADIUS: u32 = 0x49ca_f423;
const FIELD_START_MAX_MPH: u32 = 0xff4a_ef6f;
const FIELD_EXCLUDED_MODE: u32 = 0x1b3c_1ddc;
const FIELD_PROMPT_EVENT: u32 = 0xd8f8_2827;
/// The free-roam HUD, whose points-of-interest widget lists the looks.
const POI_SCREEN: &str = mw_assets::ui::FREE_ROAM_HUD;
const MPH: f32 = 0.44704;

pub struct EventStartsPlugin;

impl Plugin for EventStartsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EventStarts>()
            .add_systems(Startup, start_loading)
            .add_systems(Update, receive)
            .add_systems(Update, fill_inputs.in_set(WorldLogicSet::Inputs))
            .add_systems(Update, apply_outcomes.in_set(WorldLogicSet::Outcomes))
            .add_systems(Update, draw_starts.after(apply_outcomes).after(crate::gate_marker::draw_markers));
    }
}

/// An event that can have a start point.
struct Candidate {
    setup: Arc<RaceSetup>,
    position: Vec3,
    /// The start trigger's id (`SpawnPosition`).
    trigger: u32,
    most_wanted: bool,
    /// Our race code runs it (a race mode event with gates).
    playable: bool,
    type_name: String,
    /// The Most Wanted race's car (its first opponent's).
    blacklist_car: String,
    /// `AllowedVehicleList`: vehicle id → difficulty (1 easy, 2 medium, 3 hard).
    vehicles: Vec<(u32, i64)>,
}

struct Look {
    key: u32,
    face_camera: bool,
    /// Indices into the scene's layouts, with their instances.
    layouts: Vec<(usize, Option<LayoutInstance>)>,
}

struct Loaded {
    candidates: Vec<Candidate>,
    scene: UiScene,
    looks: Vec<Look>,
    images: Vec<(ResourceId, Image)>,
    radius: f32,
    max_mph: f32,
    prompt: String,
    prompt_id: u32,
    excluded: u32,
}

#[derive(Resource)]
struct Loading(Mutex<Receiver<Loaded>>);

struct World {
    scene: UiScene,
    looks: Vec<Look>,
    textures: HashMap<ResourceId, Handle<Image>>,
    anim: UiAnim,
    /// Per event: its layouts' timeline states.
    triggers: HashMap<u32, LayoutTriggers>,
}

/// Timeline state per (layout, element).
type LayoutTriggers = HashMap<(usize, usize), (bool, f64)>;

/// The shown starts, for the minimap and the world panels.
#[derive(Resource, Default)]
pub struct EventStarts {
    candidates: Vec<Candidate>,
    /// Indices into `candidates` with a point of interest now, and their look keys.
    shown: Vec<(usize, u32)>,
    world: Option<World>,
    radius: f32,
    max_mph: f32,
    prompt_text: String,
    /// The free-drive game mode, never listed (`GameLogicConfig` +0xd8), and the prompt's string id (+0x160).
    excluded: u32,
    prompt_id: u32,
    /// The events were handed to the manager (`EventStartInputs::events`).
    pushed: bool,
    /// The chosen event was handed to the event card (plain play).
    dispatched: bool,
}

impl EventStarts {
    /// The shown starts: (look key, start position).
    pub fn icons(&self) -> impl Iterator<Item = (u32, Vec3)> + '_ {
        self.shown.iter().map(|&(i, key)| (key, self.candidates[i].position))
    }
}

fn start_loading(mut commands: Commands, game: Res<crate::Game>) {
    let store = game.store.clone();
    let (tx, rx) = channel();
    crate::platform::thread::spawn(move || {
        use mw_formats::genesys::Value;
        let (Ok(events), Ok(lang)) = (store.events(), store.language(mw_assets::DEFAULT_LANGUAGE)) else {
            warn!("event starts: no events");
            return;
        };
        let cfg = store.genesys_object(GAME_LOGIC_CONFIG).ok();
        let num = |h: u32| match cfg.as_ref().and_then(|o| o.get(h)) {
            Some(Value::Float(f)) => Some(*f),
            Some(Value::UInt(u)) => Some(*u as f32),
            Some(Value::Int(i)) => Some(*i as f32),
            _ => None,
        };
        let excluded = num(FIELD_EXCLUDED_MODE).map_or(0, |v| v as u32);
        let names: HashMap<u32, String> =
            store.vehicles(&lang).unwrap_or_default().into_iter().map(|v| (v.id, v.name)).collect();
        let mut candidates = Vec::new();
        for e in &events.events {
            let Some(spawn) = e.spawn else { continue };
            let name = lang.get(e.name).map_or_else(|| format!("EVENT {}", e.id), str::to_uppercase);
            let type_name = lang.get(e.type_name).map(str::to_uppercase).unwrap_or_else(|| e.kind.label());
            let blacklist_car = e
                .opponents
                .iter()
                .find_map(|o| o.rollout.as_ref())
                .and_then(|r| names.get(&r.vehicle).cloned())
                .unwrap_or_default();
            let game_mode = events.game_mode(e.game_mode).copied();
            candidates.push(Candidate {
                position: Vec3::from_array(spawn.position),
                trigger: spawn.id,
                most_wanted: e.first_place_only,
                playable: e.game_mode == crate::map_screen::RACE_MODE && !e.checkpoints.is_empty(),
                type_name,
                blacklist_car,
                vehicles: e.allowed_vehicles.iter().map(|v| (v.vehicle, v.difficulty)).collect(),
                setup: Arc::new(RaceSetup { event: e.clone(), game_mode, name, test: false }),
            });
        }
        // The prompt: the original's keyboard defaults for BRAKE and ACCELERATE (Z, A), in key boxes.
        let pad = false;
        let key = |k: &str, fallback: &str| {
            store
                .keyboard_button_prompt(k)
                .or_else(|| store.keyboard_button_text(k))
                .and_then(|id| lang.get(id).map(str::to_string))
                .unwrap_or_else(|| fallback.to_string())
        };
        let (brake, accelerate) = if pad { ("LT".into(), "RT".into()) } else { (key("KEY_Z", "Z"), key("KEY_A", "A")) };
        let template = num(FIELD_PROMPT_EVENT)
            .and_then(|id| lang.get(id as u32))
            .unwrap_or(
                "HOLD {Controls.ButtonPrompt[\"BRAKE\"]} + {Controls.ButtonPrompt[\"ACCELERATE\"]} TO START EVENT",
            )
            .to_string();
        let prompt = mw_formats::ui::substitute(&template, |expr| {
            if expr.contains("\"BRAKE\"") {
                Some(brake.clone())
            } else if expr.contains("\"ACCELERATE\"") {
                Some(accelerate.clone())
            } else {
                None
            }
        });
        // The looks' world layouts (without their instances: `draw_world_layout` applies them in the world).
        let pois = store.ui_poi_visuals(POI_SCREEN).unwrap_or_default();
        let mut layouts: Vec<PlacedLayout> = Vec::new();
        let mut looks = Vec::new();
        for key in [POI_EVENT, POI_MOST_WANTED, POI_MOST_WANTED_WON] {
            let Some(poi) = pois.iter().find(|p| p.key == key) else {
                warn!("event starts: no look for point of interest {key}");
                continue;
            };
            let mut look = Look { key, face_camera: poi.face_camera, layouts: Vec::new() };
            for l in &poi.world {
                let mut l = l.clone();
                look.layouts.push((layouts.len(), l.instance.take()));
                layouts.push(l);
            }
            looks.push(look);
        }
        let scene = store.ui_scene(layouts, &lang);
        let images = scene
            .textures()
            .into_iter()
            .filter_map(|id| Some((id, crate::game_text::decode_texture(&store, id)?)))
            .collect();
        info!(
            "event starts: {} events with a start, {} looks, start radius {:?} m below {:?} mph",
            candidates.len(),
            looks.len(),
            num(FIELD_START_RADIUS),
            num(FIELD_START_MAX_MPH)
        );
        let _ = tx.send(Loaded {
            candidates,
            scene,
            looks,
            images,
            radius: num(FIELD_START_RADIUS).unwrap_or(22.0),
            max_mph: num(FIELD_START_MAX_MPH).unwrap_or(70.0),
            prompt,
            prompt_id: num(FIELD_PROMPT_EVENT).map_or(0, |v| v as u32),
            excluded,
        });
    });
    commands.insert_resource(Loading(Mutex::new(rx)));
}

fn receive(
    mut commands: Commands,
    loading: Option<Res<Loading>>,
    mut starts: ResMut<EventStarts>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(loading) = loading else { return };
    let Ok(loaded) = loading.0.lock().unwrap().try_recv() else { return };
    commands.remove_resource::<Loading>();
    let textures = loaded.images.into_iter().map(|(id, img)| (id, images.add(img))).collect();
    starts.candidates = loaded.candidates;
    starts.radius = loaded.radius;
    starts.max_mph = loaded.max_mph;
    starts.prompt_text = loaded.prompt;
    starts.prompt_id = loaded.prompt_id;
    starts.excluded = loaded.excluded;
    starts.pushed = false;
    starts.world = Some(World {
        scene: loaded.scene,
        looks: loaded.looks,
        textures,
        anim: UiAnim::default(),
        triggers: HashMap::new(),
    });
}

/// The event list the manager builds its starts from (`StartEvent`, `FUN_0074a600`'s questions).
fn start_events(candidates: &[Candidate], excluded: u32) -> Vec<StartEvent> {
    candidates
        .iter()
        .map(|c| StartEvent {
            event: c.setup.event.id,
            mode: c.setup.event.game_mode,
            offline: true,
            most_wanted: c.most_wanted,
            // No results are kept: no Most Wanted race counts as won (model decision, module doc).
            most_wanted_won: false,
            race: matches!(
                c.setup.event.kind,
                mw_assets::events::EventKind::Circuit
                    | mw_assets::events::EventKind::Sprint
                    | mw_assets::events::EventKind::MostWanted
            ),
            laps: c.setup.event.laps,
            start: Some((c.trigger, c.position.to_array())),
            allowed_vehicles: c.vehicles.iter().map(|&(id, _)| id).collect(),
            // The profile checks are not ported (module doc): every event unlocked and available.
            unlocked: c.setup.event.game_mode != excluded,
            available: true,
            recommendation: None,
        })
        .collect()
}

/// Fills `EventStartInputs` each frame (`FUN_00755220`'s reads: the car, its speed and pedals, the reference point,
/// the pursuit, the game clock) before the manager steps.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fill_inputs(
    mut starts: ResMut<EventStarts>,
    logic: NonSend<WorldLogic>,
    car: Query<(&Transform, &LinearVelocity), (With<Car>, With<Player>)>,
    garage: Res<crate::garage::Garage>,
    pursuit: Option<Res<crate::police::PursuitHud>>,
    input: Res<CarInput>,
    time: Res<Time>,
) {
    if starts.world.is_none() {
        return;
    }
    let Ok((xf, velocity)) = car.single() else { return };
    let mut st = logic.state.borrow_mut();
    let i = &mut st.event_starts.inputs;
    if !starts.pushed {
        i.events = start_events(&starts.candidates, starts.excluded);
        starts.pushed = true;
    }
    let (throttle, brake) = input.scripted.map_or((input.throttle, input.brake), |s| (s[0], s[1]));
    let p = xf.translation.to_array();
    i.vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle)).unwrap_or(0);
    i.position = p;
    // The reference point (UI manager +0x2cd50) is the player's car (model decision, module doc).
    i.reference = p;
    i.speed_mph = velocity.0.length() / MPH;
    i.throttle = throttle;
    i.brake = brake;
    i.in_pursuit = pursuit.as_ref().is_some_and(|p| p.in_chase || p.in_cooldown);
    i.dt = time.delta_secs();
    i.paused = false;
    i.gmm_2e83 = false;
    // Model decisions: the list may always be built, no intro, a player present, the prefetch idle, the UI idle.
    i.list_allowed = true;
    i.intro = false;
    i.any_player = true;
    i.prefetch_idle = true;
    i.ui_idle = true;
    i.freedrive_mode = starts.excluded;
    i.start_radius = starts.radius;
    i.max_mph = starts.max_mph;
    i.prompt = starts.prompt_id;
    i.speed_run_mode = logic.values.as_ref().map_or(SPEED_RUN_MODE, |v| v.speed_run_mode);
    i.ambush_mode = AMBUSH_MODE;
}

/// Carries out the manager's outcomes: the shown starts (its list, from `EventStartState::shown`), the prompt
/// (GMM+0x2c70), and in plain play the chosen event (`FUN_006e74f0` → `PlayEvent`).
#[allow(clippy::type_complexity)]
fn apply_outcomes(
    mut starts: ResMut<EventStarts>,
    mut logic: NonSendMut<WorldLogic>,
    mut input: ResMut<CarInput>,
    mut hud: ResMut<crate::hud::HudState>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
) {
    // The markers' requests: this module draws the manager's list instead; the hints, the prefetch and the
    // recommendations have no runtime counterpart.
    logic.take(|r| matches!(r, Request::EventStart(_)));
    hud.event_prompt = None;
    if starts.world.is_none() {
        return;
    }
    let test = std::env::var_os("MW_EVENT_START_TEST").is_some();
    let (shown, prompting, chosen) = {
        let st = logic.state.borrow();
        let shown: Vec<(usize, u32)> = st
            .event_starts
            .state
            .shown
            .iter()
            .filter_map(|e| Some((starts.candidates.iter().position(|c| c.setup.event.id == e.event)?, e.key)))
            .collect();
        (shown, st.game_modes.prompt == Some(starts.prompt_id), st.event_starts.state.chosen)
    };
    starts.shown = shown;
    if prompting {
        hud.event_prompt = Some(starts.prompt_text.clone());
        if test && input.scripted.is_none() {
            info!("event start test: burnout at a start");
            input.scripted = Some([1.0, 1.0, 0.0, 0.0]);
        }
    }
    if !logic.plain.awaiting_event {
        starts.dispatched = false;
        return;
    }
    if logic.driven || starts.dispatched {
        return;
    }
    // Plain play: the state exited with `chosen` (`CHECKENTRY`).
    starts.dispatched = true;
    if test {
        input.scripted = None;
    }
    logic.state.borrow_mut().game_modes.request_event(0, false);
    let c = starts.candidates.iter().find(|c| c.setup.event.id == chosen);
    match c {
        Some(c) if c.playable => {
            info!("event start: {} ({}) after a {HOLD_SECONDS} s burnout", c.setup.name, c.setup.event.id);
            play.write(crate::event_card::PlayEvent(c.setup.clone()));
        }
        other => {
            if let Some(c) = other {
                warn!("event start: {} ({}) is not a race our race code runs", c.setup.name, c.setup.event.id);
            }
            // The manager enters again at once for an event the runtime cannot run.
            logic.plain.awaiting_event = false;
            logic.plain.event_seen = false;
            starts.dispatched = false;
        }
    }
}

/// The shown starts' world panels, after the gate markers in the frame's world UI.
fn draw_starts(
    mut starts: ResMut<EventStarts>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    hud: Option<Res<crate::hud::HudSettings>>,
    garage: Res<crate::garage::Garage>,
    time: Res<Time>,
    mut out: ResMut<WorldUiQuads>,
) {
    let Some(cam) = camera.iter().next().map(|c| c.translation()) else { return };
    if hud.is_some_and(|h| !h.enabled || h.hidden_by_sequence) {
        return;
    }
    let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
    let now = time.elapsed_secs_f64();
    let EventStarts { candidates, shown, world, .. } = &mut *starts;
    let Some(World { scene, looks, textures, anim, triggers }) = world.as_mut() else { return };
    let mut batches = Vec::new();
    let mut batch_of = HashMap::new();
    for &(i, key) in shown.iter() {
        let c = &candidates[i];
        let Some(look) = looks.iter().find(|l| l.key == key) else { continue };
        let distance = cam.distance(c.position) as f64;
        if distance > 250.0 {
            // Beyond the layouts' own fade (none shows past 200 m): skip the work.
            continue;
        }
        let rotation = if look.face_camera {
            let d = c.position - cam;
            Quat::from_rotation_y(d.x.atan2(d.z))
        } else {
            Quat::IDENTITY
        };
        let difficulty = vehicle.and_then(|v| c.vehicles.iter().find(|&&(id, _)| id == v)).map_or(0, |&(_, d)| d);
        let mut lookup = |path: &str, arg: Option<&str>| -> Option<BindValue> {
            Some(match path {
                "DistanceToCamera" => distance.into(),
                "Widget.POI.ShowSpeedwall" | "Players.LocalPlayer.Blinded" | "Net.IsInOnlineGame" => false.into(),
                "Signals.GetSawWave" => {
                    let period = arg.and_then(|a| a.trim().parse::<f64>().ok()).filter(|&p| p > 0.0)?;
                    ((now * 1000.0 / period).fract()).into()
                }
                "Events.Locations.EventByIdPlus1" => {
                    let field = arg?.split_once(']')?.1.trim_start_matches('.');
                    match field {
                        "Name" => c.setup.name.clone().into(),
                        "EventTypeName" => c.type_name.clone().into(),
                        "Difficulty" => (difficulty as f64).into(),
                        "IsBlacklistEvent" => c.most_wanted.into(),
                        "BlacklistVehicle" => c.blacklist_car.clone().into(),
                        "DLCIconID" => false.into(),
                        // Rewards: not modelled (module doc).
                        "AvailableReward" | "EventReward1" | "EventReward2" => String::new().into(),
                        _ => return None,
                    }
                }
                _ => return None,
            })
        };
        let event_triggers = triggers.entry(c.setup.event.id).or_default();
        for (index, instance) in &look.layouts {
            let mut o = WorldLayoutOut { batches: &mut batches, batch_of: &mut batch_of, textures };
            draw_world_layout(
                scene,
                *index,
                instance.as_ref(),
                c.position,
                rotation,
                &mut lookup,
                anim,
                event_triggers,
                now,
                &mut o,
            );
        }
    }
    if std::env::var_os("MW_EVENT_START_LOG").is_some()
        && (now * 2.0).floor() != ((now - time.delta_secs_f64()) * 2.0).floor()
    {
        let tris: usize = batches.iter().map(|b: &crate::world_ui::WorldUiBatch| b.vertices.len() / 3).sum();
        info!("event starts: {} shown, {tris} panel triangles", shown.len());
    }
    out.0.extend(batches);
}
