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
use crate::race::{RacePhase, RaceSetup, RaceState};
use crate::world_ui::WorldUiQuads;

/// `POIID`: an event's start.
pub const POI_EVENT: u32 = 1_013_512;
/// `MWPOIIDC`: a Most Wanted race not yet won in first place.
pub const POI_MOST_WANTED: u32 = 955_952;
/// `MWPOIIDUC`: a Most Wanted race already won.
pub const POI_MOST_WANTED_WON: u32 = 955_955;
/// `ACTIVERADIUS` (m).
const ACTIVE_RADIUS: f32 = 2000.0;
/// At most this many starts (`FUN_0074a600`: 0x14).
const MAX_STARTS: usize = 20;
/// The reference point is taken again after it moved this far (`.rdata` 0x00fd6790 = 100, squared).
const REFRESH_DISTANCE: f32 = 10.0;
/// Height differences count this much in the start radius (`.rdata` 0x00fd67b4).
const HEIGHT_WEIGHT: f32 = 5.0;
/// Throttle and brake count as held above this (`.rdata` 0x00f8a810).
const INPUT_HELD: f32 = 0.5;
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
            .add_systems(Update, (receive, update_starts, draw_starts).chain().after(crate::gate_marker::draw_markers));
    }
}

/// An event that can have a start point.
struct Candidate {
    setup: Arc<RaceSetup>,
    position: Vec3,
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
    /// What the list was built for: the reference point, the driven car, a pursuit.
    reference: Option<Vec3>,
    vehicle: Option<u32>,
    in_pursuit: bool,
    /// The burnout timer (+0x1a8) and whether an event was started (+0x1b1).
    hold: f32,
    started: bool,
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
    std::thread::spawn(move || {
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
            if e.game_mode == excluded {
                continue;
            }
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
    starts.reference = None;
    starts.world = Some(World {
        scene: loaded.scene,
        looks: loaded.looks,
        textures,
        anim: UiAnim::default(),
        triggers: HashMap::new(),
    });
}

/// `FUN_0074a600`: the starts for the driven car around `reference`.
fn build_list(starts: &mut EventStarts, reference: Vec3, vehicle: Option<u32>) {
    starts.shown.clear();
    for (i, c) in starts.candidates.iter().enumerate() {
        if starts.shown.len() >= MAX_STARTS {
            break;
        }
        // `FUN_006eca10(list, car, 1)`: a car in the list, or no list.
        if !c.vehicles.is_empty() && !vehicle.is_some_and(|v| c.vehicles.iter().any(|&(id, _)| id == v)) {
            continue;
        }
        // A Most Wanted race not yet won shows at any distance (and none counts as won here).
        let not_won_most_wanted = c.most_wanted;
        if !not_won_most_wanted && c.position.distance_squared(reference) > ACTIVE_RADIUS * ACTIVE_RADIUS {
            continue;
        }
        let key = if c.most_wanted { POI_MOST_WANTED } else { POI_EVENT };
        starts.shown.push((i, key));
    }
}

/// `FUN_00755220` / `FUN_0074ad50` per frame: keep the list, the prompt, the burnout start.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_starts(
    mut starts: ResMut<EventStarts>,
    car: Query<(&Transform, &LinearVelocity), (With<Car>, With<Player>)>,
    garage: Res<crate::garage::Garage>,
    pursuit: Option<Res<crate::police::PursuitHud>>,
    race: Option<Res<RaceState>>,
    mut input: ResMut<CarInput>,
    time: Res<Time>,
    mut hud: ResMut<crate::hud::HudState>,
    mut play: MessageWriter<crate::event_card::PlayEvent>,
) {
    hud.event_prompt = None;
    if starts.world.is_none() {
        return;
    }
    let free_roam = race.as_ref().is_none_or(|r| r.phase == RacePhase::Finished);
    let Ok((xf, velocity)) = car.single() else { return };
    if !free_roam {
        starts.shown.clear();
        starts.reference = None;
        return;
    }
    let in_pursuit = pursuit.as_ref().is_some_and(|p| p.in_chase || p.in_cooldown);
    let vehicle = garage.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle));
    if in_pursuit {
        // `FUN_0074aaf0`: the points go while a pursuit runs.
        starts.shown.clear();
        starts.in_pursuit = true;
        starts.hold = 0.0;
        return;
    }
    let p = xf.translation;
    let moved = starts.reference.is_none_or(|r| r.distance_squared(p) >= REFRESH_DISTANCE * REFRESH_DISTANCE);
    if moved || starts.in_pursuit || starts.vehicle != vehicle {
        starts.reference = Some(p);
        starts.vehicle = vehicle;
        starts.in_pursuit = false;
        build_list(&mut starts, p, vehicle);
    }
    // `FUN_0074ad50`, called below the start speed.
    let mph = velocity.0.length() / MPH;
    if mph >= starts.max_mph || starts.started {
        starts.hold = 0.0;
        return;
    }
    let test = std::env::var_os("MW_EVENT_START_TEST").is_some();
    let (throttle, brake) = input.scripted.map_or((input.throttle, input.brake), |s| (s[0], s[1]));
    let held = throttle > INPUT_HELD && brake > INPUT_HELD;
    let dt = time.delta_secs();
    let mut holding = false;
    let mut start = None;
    let r = starts.radius;
    for &(i, _) in &starts.shown {
        let c = &starts.candidates[i];
        let d = c.position - p;
        if d.x * d.x + (HEIGHT_WEIGHT * d.y).powi(2) + d.z * d.z >= r * r {
            continue;
        }
        hud.event_prompt = Some(starts.prompt_text.clone());
        if test && input.scripted.is_none() {
            info!("event start test: burnout at {} ({})", c.setup.name, c.setup.event.id);
            input.scripted = Some([1.0, 1.0, 0.0, 0.0]);
        }
        if held {
            holding = true;
            start = Some(i);
        }
    }
    if holding {
        starts.hold += dt;
    } else {
        starts.hold = 0.0;
    }
    if let Some(i) = start
        && starts.hold > HOLD_SECONDS
    {
        starts.started = true;
        starts.hold = 0.0;
        let c = &starts.candidates[i];
        if test {
            input.scripted = None;
        }
        if c.playable {
            info!("event start: {} ({}) after a {HOLD_SECONDS} s burnout", c.setup.name, c.setup.event.id);
            play.write(crate::event_card::PlayEvent(c.setup.clone()));
        } else {
            warn!("event start: {} ({}) is not a race our race code runs", c.setup.name, c.setup.event.id);
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
