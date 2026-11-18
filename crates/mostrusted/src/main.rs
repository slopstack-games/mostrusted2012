//! Most Rusted: an open-source runtime for Need for Speed: Most Wanted (2012).
//!
//! No game data is shipped. All assets are streamed at runtime from an
//! installed copy of the game (see `mw-gamedir` for how it is located).

mod ai;
mod ai_driver;
mod audio;
mod audio_tap;
mod bench;
mod bodyshop;
mod camera;
mod car_anim;
mod car_audio;
mod car_contacts;
mod car_impacts;
mod car_sfx;
mod chassis;
mod chevron;
use mw_handling::{drift, drivetrain, tyre};
mod car_lights;
mod convert;
mod cop_driver;
mod cop_patrol;
mod cop_sight;
mod coronas;
mod crash;
mod crash_vfx;
mod cutscene;
mod damage;
mod deferred;
mod dlc;
mod drive;
mod easydrive;
mod event_card;
mod event_starts;
mod exhaust;
mod flash_probe;
mod frame_log;
mod freeroam;
mod frontend;
mod game_text;
mod garage;
mod gate_marker;
mod gps;
mod groundcover;
mod hud;
mod ignition;
mod input;
mod input_script;
mod jackspots;
mod loader;
mod lod;
mod logic;
mod logic_effects;
mod logic_loads;
mod map_screen;
mod materials;
mod menu;
mod mods_menu;
mod movie;
mod music;
mod nitrous;
mod paint_wear;
mod particles;
mod pass_mask;
mod pause;
mod platform;
mod player_sfx;
mod police;
mod police_audio;
mod police_fx;
mod police_lightbar;
mod police_speech;
mod prewarm;
mod prop_vfx;
mod props;
mod pursuit;
mod race;
mod reflection;
mod replay;
mod sequence_fx;
mod settings;
mod sfx;
mod shadow_cull;
mod sky;
#[cfg(not(target_arch = "wasm32"))]
mod steam_controller;
mod streaming;
mod tonemap;
mod traffic;
mod traffic_lamps;
mod traffic_reaction;
mod tyre_marks;
mod ui_quads;
mod ui_sequence;
mod ui_sound;
mod weather;
#[cfg(target_arch = "wasm32")]
mod web;
mod window_test;
mod world_coronas;
mod world_lights;
mod world_logic;
mod world_sfx;
mod world_ui;
mod zone_view;

use crate::platform::Instant;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::prelude::*;
use bevy::render::view::screenshot::Screenshot;
use clap::Parser;
use mw_assets::AssetStore;
use mw_bundle::ResourceId;
use mw_gamedir::GameDir;

use loader::{LoaderChannel, Msg, Request};
use materials::{GameEnvironment, GameMaterial, Look, TexKey};

#[derive(Parser, Debug)]
#[command(version, about = "Most Rusted: NFS Most Wanted (2012) runtime")]
struct Cli {
    /// Game install directory (defaults to $MW2012_DIR or Steam auto-detection).
    #[arg(long)]
    game_dir: Option<PathBuf>,
    /// View a car instead of the world: a name ("porsche 911"), a vehicle ID,
    /// or a bundle path such as VEHICLES/VEH_1085007_HI.BNDL.
    #[arg(long)]
    vehicle: Option<String>,
    /// Drive a car (name, ID or bundle) through the world.
    #[arg(long)]
    drive: Option<String>,
    /// Spawn point for `--drive` as `x,y,z[,heading_degrees]` (default: a road west of downtown; with
    /// `--race`, the event's start, which this overrides for the player's car).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    spawn: Vec<f32>,
    /// Race an offline event (an id from `mwtool events`): its car (unless --drive names one) on its start,
    /// the countdown, checkpoints, laps and the result (`race.rs`). `--test race` runs it through
    /// scripted and exits.
    #[arg(long)]
    race: Option<u32>,
    /// With --race: don't play the event's intro / outro sequences (`cutscene.rs`); the race then starts with
    /// the game mode's frozen intro and 3-2-1.
    #[arg(long)]
    no_cutscenes: bool,
    /// With --race: `outro` plays the event's outro sequence as soon as the car is placed (for screenshots).
    #[arg(long)]
    cutscene: Option<String>,
    /// Drive on an empty flat test track instead of the city (with --drive).
    #[arg(long)]
    test_track: bool,
    /// The collision surface (index into the game's surface list, `mwtool surfaces`) the flat test track
    /// counts as: 0 is the game's default surface, 8 asphalt, 4 grass, 7 dirt. Tests compare surfaces with it.
    #[arg(long, default_value_t = 0)]
    track_surface: usize,
    /// Run a scripted test and exit: accel, brake, corner, handbrake, drift, donut, engine, crash (drive into a
    /// wall; with --screenshot, saves `<name>-before` / `<name>-after` images), or smash (drive through a cone,
    /// a sign, a street lamp and a fence into an immovable wall made of the game's props, at --smash-speed; before /
    /// after screenshots like crash). Implies --test-track.
    #[arg(long)]
    test: Option<String>,
    /// Replay a recording of the original (`mwtool capture`): drive with its recorded inputs from its starting
    /// speed on the test track, write our telemetry next to it (`<file>.ours.csv`) and exit. Compare with
    /// `mwtool compare <file> <file>.ours.csv`.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// `<inputs.csv>,<out.csv>`: run the chase camera offline over a recorded car motion and exit, no window
    /// (`camera::replay_recording`; `tools/camcompare.py` makes the inputs from a recording of the original).
    #[arg(long, value_delimiter = ',')]
    camera_replay: Vec<PathBuf>,
    /// Smash test: speed (km/h) the car holds through the props.
    #[arg(long, default_value_t = 60.0)]
    smash_speed: f32,
    /// Smash test: what stands on the track: `props` (cone, sign, lamp, fence, wall), `compounds` (six
    /// compound objects that break into their parts, filmed from beside the last knock) or `bags` (a trash bag
    /// and crates without an `AdditionalInfo`).
    #[arg(long, default_value = "props")]
    smash_set: String,
    /// Draw props, compound and dynamic objects without collision (smashable street furniture is on by
    /// default; `props.rs`). `--test smash` keeps it on.
    #[arg(long)]
    no_prop_physics: bool,
    /// Log the mean and worst frame time every 5 s once loading has finished (turns vsync off).
    #[arg(long)]
    frame_stats: bool,
    /// Benchmark: once loaded and warmed up, measure this many seconds, print a `BENCH RESULT` line and exit
    /// (turns vsync off; `bench.rs`).
    #[arg(long)]
    bench: Option<f32>,
    /// Debug: the key light casts no shadows (benchmark attribution).
    #[arg(long)]
    no_shadows: bool,
    /// Crash test: wall angle in degrees (0 = head-on, 30 = the front-left corner hits first).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    crash_angle: f32,
    /// Crash test: distance to the wall in metres (sets the impact speed).
    #[arg(long, default_value_t = 80.0)]
    crash_distance: f32,
    /// Crash test: wall width in metres (a narrow one, like a bridge pillar, leaves the crash camera's
    /// roadside shot a view past it).
    #[arg(long, default_value_t = 30.0)]
    crash_wall_width: f32,
    /// Hold the throttle at this value (0–1), for automated driving tests.
    #[arg(long)]
    auto_throttle: Option<f32>,
    /// Hold nitrous (with `--auto-throttle`, for looking at the nitrous flames).
    #[arg(long)]
    nitrous: bool,
    /// Performance mods on the driven car: `slot=category[,…]`, indices as `mwtool mods <car>` prints them.
    #[arg(long, default_value = "")]
    mods: String,
    /// Debug: skip InstanceList backdrop entries.
    #[arg(long)]
    no_backdrop: bool,
    /// Debug: skip groundcover (grass and plant sprites).
    #[arg(long)]
    no_groundcover: bool,
    /// Merge each track unit's single-use static placements per material (`loader::Worker::send_merged`).
    /// Off by default: it removed ~10 % of the visible mesh entities and no measurable frame time
    /// (research log, 2026-10-06 round 5).
    #[arg(long)]
    merge_static: bool,
    /// Debug: draw the backdrop of every track unit unfiltered (all LOD tiers stacked: coarse terrain and
    /// building stand-ins overlap the detailed world). The default draws the camera zone's backdrop.
    #[arg(long, conflicts_with_all = ["no_backdrop", "backdrop_zone"])]
    all_backdrops: bool,
    /// Debug: draw only this track unit's backdrop, as the game does while the player is in that unit.
    #[arg(long, conflicts_with = "no_backdrop")]
    backdrop_zone: Option<u32>,
    /// Draw one backdrop composed per place from the zone owning it, and every unit's main geometry (the
    /// behaviour before 2026-10-19). The default follows the camera's zone as the game does (`zone_view.rs`).
    #[arg(long, conflicts_with_all = ["no_backdrop", "all_backdrops", "backdrop_zone"])]
    composed_backdrop: bool,
    /// Debug: with --all-backdrops or --backdrop-zone, also draw backdrop LOD2 stand-ins.
    #[arg(long)]
    keep_lod_proxies: bool,
    /// Debug: skip these model resource IDs (comma separated).
    #[arg(long, value_delimiter = ',')]
    exclude_models: Vec<String>,
    /// Disable the background music.
    #[arg(long)]
    no_music: bool,
    /// Disable all audio (also `MW_MUTE=1`, which `tools/hl` sets for headless runs).
    #[arg(long)]
    mute: bool,
    /// Volumes for this run, over the saved settings: `master=0.8,engine=1,sfx=0.5,music=0.3` (0..1 or %).
    /// F2 / gamepad Select opens the volume settings in game (saved to ~/.config/mostrusted/settings.cfg;
    /// %APPDATA%\mostrusted on Windows).
    #[arg(long)]
    volume: Option<String>,
    /// Open the volume settings at startup (for screenshots).
    #[arg(long)]
    show_settings: bool,
    /// Log raw gamepad input (to diagnose controller mappings).
    #[arg(long)]
    gamepad_debug: bool,
    /// Frames to wait after loading before `--screenshot` captures.
    #[arg(long, default_value_t = 30)]
    screenshot_delay: u32,
    /// Capture `--screenshot` once this much game time (seconds, as simulated: 4x real time during
    /// `--test`) has passed since startup, instead of after `--screenshot-delay` frames.
    #[arg(long)]
    screenshot_time: Option<f32>,
    /// Capture `--screenshot` this many seconds of game time after the first hard impact (closing at over
    /// 12 m/s), for checking collision effects.
    #[arg(long)]
    screenshot_after_impact: Option<f32>,
    /// Capture `--screenshot` this many seconds of game time after the first exhaust backfire (`exhaust.rs`).
    #[arg(long)]
    screenshot_after_backfire: Option<f32>,
    /// Load this many track units once, nearest first, instead of streaming the focus zone and its neighbours
    /// (`streaming.rs`).
    #[arg(long)]
    units: Option<usize>,
    /// Load every track unit once and keep it (the behaviour before streaming; A/B and full-map views). The
    /// default for a free camera without `--drive` (its aerial view needs every unit, `streaming.rs`).
    #[arg(long, conflicts_with = "stream")]
    load_all: bool,
    /// Without `--drive`: stream around the free camera as the game streams around its camera, instead of
    /// loading every unit.
    #[arg(long)]
    stream: bool,
    /// Camera pose as `x,y,z,look_x,look_y,look_z` (overrides the default view; with --drive in the world, a
    /// fixed camera instead of the chase camera).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    camera: Option<Vec<f32>>,
    /// With --camera: the fixed camera's horizontal field of view in degrees, as the game measures it (its chase
    /// camera at rest: 70; for matched pairs with captures of the original, `ref/shots/*/camera.txt`). Without it
    /// Bevy's default 45° vertical is used.
    #[arg(long)]
    fov: Option<f32>,
    /// Body paint colour as `r,g,b` in 0–255 sRGB, overriding the car's own paint colour.
    #[arg(long, value_delimiter = ',')]
    paint: Option<Vec<u8>>,
    /// Which of the car's paint options (`VehiclePaint` objects, in ID order) to use.
    #[arg(long, default_value_t = 0)]
    paint_index: usize,
    /// The car's starting dirt as `dirt[,dust]` (0–1, the game's vehicle +0x2c84 / +0x2c80; dust defaults to
    /// dirt). Driving builds them up from there (`paint_wear.rs`).
    #[arg(long, value_delimiter = ',')]
    dirt: Option<Vec<f32>>,
    /// Save a screenshot once loading has finished, then exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Hour of the day (0–24, fractions blend between the timeline's keyframes). F5 / F6 change it.
    #[arg(long, default_value_t = 13.0)]
    hour: f32,
    /// Bad weather: use the keyframes' Alternate twins (hazier, dimmer). F7 toggles it.
    #[arg(long)]
    weather: bool,
    /// Environment timeline (resource ID, or `default`) instead of the one painted under the camera.
    #[arg(long)]
    timeline: Option<String>,
    /// Scale the roads' wet amount (1 = the game; 0 = dry roads, for comparison).
    #[arg(long, default_value_t = 1.0)]
    road_wetness: f32,
    /// Render the wet roads' planar reflection with a second, mirrored camera (doubles the scene's
    /// rendering; without it wet roads reflect the sky cube).
    #[arg(long)]
    planar_reflection: bool,
    /// Vehicle light state `brake,headlights,reverse,tail` (0–1 each; the game's
    /// `g_selfIlluminationMask`). Driving switches brake and reverse with the pedals.
    #[arg(long, value_delimiter = ',', default_value = "0,1,0,1")]
    lights: Vec<f32>,
    /// Test the car menu's swap headlessly: after loading, swap to these cars in turn (comma
    /// separated names or IDs, `name@paint` picks a paint index, `free` leaves the car for the free
    /// camera). `--screenshot` captures after the last swap.
    #[arg(long, value_delimiter = ',')]
    swap_to: Vec<String>,
    /// Frames to wait before each `--swap-to` step (after loading, and after each car is placed).
    #[arg(long, default_value_t = 120)]
    swap_delay: u32,
    /// Pin animated body parts (active aero clips, `mwtool damage <car>` lists them) at a position:
    /// `all=1`, `0=0.5`. Without it, `--drive` runs the game's rule from the car's speed.
    #[arg(long, value_delimiter = ',')]
    anim: Vec<String>,
    /// Start the player car damaged: `zone=level` per damage zone (`front`, `rear-left`, ... or the clip
    /// index; `all=0.3` sets every zone), level 0..1, as `mwtool damage --level` takes them.
    #[arg(long, value_delimiter = ',')]
    damage: Vec<String>,
    /// Open the car menu at startup (also in `--screenshot` runs, to capture it).
    #[arg(long)]
    show_menu: bool,
    /// Open Easydrive, the game's d-pad menu, at startup (also in `--screenshot` runs, to capture it).
    #[arg(long)]
    show_easydrive: bool,
    /// With --show-easydrive: enter these rows one after the other (e.g. `4,0,1,1` = Customize car, first class,
    /// its first mod, the PRO level), for headless checks of the menu's pages
    #[arg(long, value_delimiter = ',')]
    easydrive_path: Vec<usize>,
    /// Driving camera at startup: chase, bonnet, bumper, or lookback (the chase camera looking back).
    /// C / gamepad Y cycles the modes the car's data defines.
    #[arg(long, default_value = "chase")]
    camera_mode: String,
    /// HUD speed and distance in mph / miles (the game's `Language.IsMetric` off).
    #[arg(long)]
    mph: bool,
    /// No driving HUD (the debug status text stays; F3 toggles it when the HUD is up).
    #[arg(long)]
    no_hud: bool,
    /// No traffic cars (`traffic.rs`; traffic runs in `--drive` sessions in the world).
    #[arg(long)]
    no_traffic: bool,
    /// No police (`police.rs`; cops patrol and chase in `--drive` sessions in the world, outside races, tests
    /// and benchmarks). `--test pursuit` runs a pursuit start to end and prints TEST RESULT lines; `--test cops` checks when a
    /// pursuit starts (passing a cop slowly, speeding past it, ramming it).
    #[arg(long)]
    no_police: bool,
    /// Multiply the traffic density (1 = the game's; for tests).
    #[arg(long, default_value_t = 1.0)]
    traffic_density: f32,
    /// AI opponents (with --drive): comma-separated car names or IDs, placed on a route along the traffic lanes
    /// ahead of the player (`ai.rs`). `--test ai` also drives the player's car with the same driver and logs both.
    #[arg(long, value_delimiter = ',')]
    ai: Vec<String>,
    /// Seconds an `--test ai` run lasts.
    #[arg(long, default_value_t = 60.0)]
    ai_seconds: f32,
    /// Rubber band for the `--ai` opponents: `<RaceBalancingData id>,<RaceBalancingProfile id>` (GeneSys ids in
    /// GAMELOGIC/GAMEMODES.BNDL, e.g. 01000000000f0258,01000000000f0259).
    #[arg(long, value_delimiter = ',')]
    ai_balancing: Vec<String>,
    /// Run the game's start-up flow (title screen, boot transition over the loading, free-roam start); the
    /// default when no mode flag (--drive, --vehicle, --camera, --race, --test, --replay) is given.
    #[arg(long)]
    front_end: bool,
    /// The original's game-logic state machine ("Top State") runs by default (user 2026-11-18) and logs what it
    /// does (`hsm:` lines); the requests carried out so far are listed in logic.rs's module doc. Kept as a no-op.
    #[arg(long)]
    hsm: bool,
    /// Don't run the logic (the hand-written flows alone): for bisecting a regression while the port moves over.
    #[arg(long)]
    no_hsm: bool,
    /// With the logic (implies --hsm and the start-up flow): once free roam is reached, start this event the way the
    /// map's START RACE does; the logic runs it (logic.rs). With `--test race`: the scripted run through its gates.
    #[arg(long)]
    hsm_event: Option<u32>,
    /// Keep the sound in test runs (`--test`, crash / smash / race tests are silent otherwise): audio checks such
    /// as the browser build's recording (`tools/web/headless ... <record_seconds>`).
    #[arg(long)]
    sound: bool,
    /// Start a new game: the default car at the free-roam start, ignoring the saved profile.
    #[arg(long)]
    new_game: bool,
    /// Front end without a human: "press a key" this many seconds after the title's prompt shows.
    #[arg(long)]
    auto_start: Option<f32>,
    /// Front end: save screenshots of the title, the transition and the first seconds of play into this
    /// directory, print the timing log (`FRONTEND TIMING` lines) and exit.
    #[arg(long)]
    front_end_shots: Option<PathBuf>,
    /// Show the in-game loading screen (layout 108177) until the start area is resident, also in headless runs
    /// (interactive world drives show it anyway).
    #[arg(long)]
    show_loading: bool,
    /// Open the pause menu (screen 431082) at startup (also in `--screenshot` runs, to capture it).
    #[arg(long)]
    show_pause: bool,
    /// The profile file (the car and where it was, saved every 10 s while driving; read by the start-up flow to
    /// continue). Default: ~/.config/mostrusted/profile.cfg (%APPDATA%\mostrusted\profile.cfg on Windows).
    #[arg(long)]
    profile: Option<PathBuf>,
    /// With --show-pause: choose its MAP item (the map screen 899991).
    #[arg(long)]
    pause_map: bool,
    /// Open the map screen (899991, the pause menu's MAP) once the car is placed (also in `--screenshot` runs).
    #[arg(long)]
    show_map: bool,
    /// With --show-map: select this event (`mwtool events` ids).
    #[arg(long)]
    map_select: Option<u32>,
    /// With --show-map --map-select: open the event's options (SET DESTINATION / START RACE).
    #[arg(long)]
    map_options: bool,
    /// Choose START RACE for this event on the map: the event-loading card, then the race with its intro.
    #[arg(long)]
    map_start: Option<u32>,
    /// Choose SET DESTINATION for this event on the map (the GPS route; the map closes again unless --show-map).
    #[arg(long)]
    map_waypoint: Option<u32>,
}

#[derive(Resource)]
pub struct Game {
    environment: weather::EnvironmentKey,
    /// `--timeline` given: don't follow the timeline paint map.
    fixed_timeline: bool,
    road_wetness: f32,
    /// Bake environment changes on the main thread (deterministic screenshot / test runs).
    sync_environment: bool,
    pub store: Arc<AssetStore>,
    request: Request,
    camera: Option<Transform>,
    /// `--fov`: horizontal degrees for the fixed camera.
    fov: Option<f32>,
    screenshot: Option<PathBuf>,
    screenshot_delay: u32,
    screenshot_time: Option<f32>,
    screenshot_after_impact: Option<f32>,
    screenshot_after_backfire: Option<f32>,
    paint: Option<Color>,
    filter: loader::WorldFilter,
    mute: bool,
    /// How far above the start the ground ray begins (m): 30 for `--spawn` points, small for the game's own
    /// start points (road level; a swap spot's car box is centred ~1.5 m up).
    start_probe: f32,
}

/// The player's view: every `Camera3d` except the planar reflection's mirrored camera (`reflection.rs`).
pub type MainCamera = (With<Camera3d>, Without<reflection::PlanarReflectionCamera>);

/// GPU meshes and materials making up one game Model.
pub type ModelParts = Vec<(Handle<Mesh>, Handle<GameMaterial>)>;

/// Bevy-side caches keyed by game resource IDs.
#[derive(Resource, Default)]
pub(crate) struct Loaded {
    textures: HashMap<TexKey, Handle<Image>>,
    models: HashMap<ResourceId, ModelParts>,
    materials: HashMap<Look, Handle<GameMaterial>>,
    /// Materials of alpha-tested parts (`shadow_cull::ShadowClass::Alpha`).
    mask_materials: HashSet<AssetId<GameMaterial>>,
    /// Models' own shadow distances (`loader::Msg::ShadowRange`).
    shadow_ranges: HashMap<ResourceId, f32>,
    /// Models' pass masks (`loader::Msg::PassMask`, `pass_mask.rs`).
    pass_masks: HashMap<ResourceId, u8>,
    instances: usize,
    /// The tag for the world placements being received (`loader::Msg::ZoneTag`, `zone_view.rs`).
    zone_tag: Option<zone_view::ZoneTag>,
    /// Placements with physics (`props.rs`).
    props: usize,
    status: String,
    pub(crate) done: bool,
    started: Option<Instant>,
}

/// The debug status text (top left); hidden behind the HUD while driving unless F3 is pressed.
#[derive(Component)]
pub struct StatusText;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let game = GameDir::locate(cli.game_dir.as_deref())?;
    run(cli, game)
}

/// In a browser the page starts the game (`web.rs`) once it has the install's files.
#[cfg(target_arch = "wasm32")]
fn main() {}

/// Everything after the install is found: the same on the desktop and in a browser.
fn run(mut cli: Cli, game: GameDir) -> anyhow::Result<()> {
    drive::install_gamepad_mappings();
    cli.mute |= std::env::var_os("MW_MUTE").is_some_and(|v| v != "0");
    cli.hsm = !cli.no_hsm || cli.hsm_event.is_some();
    // With the logic on, `--race <id>` is the logic's event start (as `--hsm-event`: boot, free roam, START RACE),
    // pressed after 1 s unless told otherwise; `--no-hsm --race` keeps the direct start.
    if cli.hsm && cli.hsm_event.is_none() && cli.race.is_some() {
        cli.hsm_event = cli.race.take();
        cli.auto_start = cli.auto_start.or(Some(1.0));
    }
    if cli.bench.is_some() && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
        // Wayland offers no Immediate present mode and paces Mailbox at the monitor's refresh: benchmark
        // through XWayland, where `AutoNoVsync` gets Immediate (`bench.rs`).
        // SAFETY: no other thread has started yet.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
    println!("Most Wanted install: {} (version {})", game.root().display(), game.version().unwrap_or("?"));
    // The player must own the game: a known retail build of the executable (`GameDir::ownership`, user decision
    // 2026-11-17). DLC ownership is never taken from the files (every pack ships in the install).
    match game.ownership() {
        mw_gamedir::Ownership::Retail { release } => println!("retail copy: {release}"),
        mw_gamedir::Ownership::Unknown { sha256 } => anyhow::bail!(
            "this install is not a known retail copy of Need for Speed Most Wanted (2012): NFS13.exe has SHA-256 \
             {sha256}. Most Rusted runs only on a copy you own, as installed by Steam or the EA app; if yours is \
             one, please report this hash so its release can be added."
        ),
    }
    let t = Instant::now();
    let store = Arc::new(AssetStore::new(game)?);
    println!("indexed {} resources in {:?}", store.resource_count(), t.elapsed());
    // The DLC lock: every DLC pack counts as not owned (`dlc.rs`).
    dlc::init(&store);
    if let [input, output] = cli.camera_replay.as_slice() {
        return camera::replay_recording(&store, input, output);
    }

    if cli.camera.as_ref().is_some_and(|c| c.len() != 6) {
        anyhow::bail!("--camera expects 6 comma-separated values: x,y,z,look_x,look_y,look_z");
    }
    match cli.dirt.as_deref() {
        None => {}
        Some(&[d]) => materials::set_dirt_level(d, d),
        Some(&[d, u]) => materials::set_dirt_level(d, u),
        Some(_) => anyhow::bail!("--dirt expects dirt[,dust]"),
    }
    let paint = match cli.paint.as_deref() {
        None => None,
        Some(&[r, g, b]) => Some(Color::srgb_u8(r, g, b)),
        Some(_) => anyhow::bail!("--paint expects r,g,b"),
    };
    let camera =
        cli.camera.map(|c| Transform::from_xyz(c[0], c[1], c[2]).looking_at(Vec3::new(c[3], c[4], c[5]), Vec3::Y));
    let resolve = |q: &str| -> anyhow::Result<String> {
        if q.to_ascii_uppercase().ends_with(".BNDL") {
            return Ok(q.to_string());
        }
        let v =
            store.find_vehicle(q)?.ok_or_else(|| anyhow::anyhow!("no car matches {q:?} (see `mwtool vehicles`)"))?;
        println!("vehicle: {} ({}, id {})", v.name, v.year, v.id);
        Ok(v.hi_bundle.expect("find_vehicle only returns cars with bundles"))
    };
    let race_test = cli.test.as_deref() == Some("race");
    if race_test && cli.race.is_none() && cli.hsm_event.is_none() {
        anyhow::bail!("--test race needs --race <event id> or --hsm-event <event id>");
    }
    // An event's race set-up: its start and checkpoints, its game mode and name.
    let event_setup = |id: u32, test: bool| -> anyhow::Result<race::RaceSetup> {
        let events = store.events()?;
        let event = events.get(id).cloned().ok_or_else(|| anyhow::anyhow!("no event {id} (see `mwtool events`)"))?;
        let name = store
            .language(mw_assets::DEFAULT_LANGUAGE)
            .ok()
            .and_then(|l| l.get(event.name).map(str::to_owned))
            .unwrap_or_else(|| format!("event {id}"));
        if event.spawn.is_none() || event.checkpoints.is_empty() {
            anyhow::bail!("event {id} ({name}) has no start or no checkpoints: not a race");
        }
        println!(
            "race: {name} ({}, {} laps, {} checkpoints, {} opponents)",
            event.kind.label(),
            event.laps,
            event.checkpoints.len(),
            event.opponents.len()
        );
        let game_mode = events.game_mode(event.game_mode).copied();
        Ok(race::RaceSetup { event, game_mode, name, test })
    };
    // `--race`: the event's start and (unless --drive names one) its first allowed car.
    let race = cli.race.map(|id| event_setup(id, race_test)).transpose()?;
    // `--hsm-event`: started from free roam through the logic (the race test is the plugin's `test_runtime`).
    let hsm_event = cli.hsm_event.map(|id| event_setup(id, false)).transpose()?.map(Arc::new);
    // The event's intro and outro sequences (`cutscene.rs`).
    let cutscenes = match &race {
        Some(r) if !cli.no_cutscenes => {
            let (intro, outro) = store.event_sequences(&r.event, r.game_mode.as_ref()).unwrap_or_else(|e| {
                println!("cutscenes: {e}");
                (None, None)
            });
            println!(
                "cutscenes: intro {} ({:.1} s), outro {} ({:.1} s)",
                r.event.event_intro,
                intro.as_ref().map_or(0.0, |s| s.length),
                r.event.event_outro,
                outro.as_ref().map_or(0.0, |s| s.length)
            );
            Some(Arc::new(cutscene::CutsceneSetup {
                intro,
                outro,
                auto_skip: race_test,
                force_outro: cli.cutscene.as_deref() == Some("outro"),
            }))
        }
        _ => None,
    };
    if let Some(r) = &race
        && let Some(pack) = dlc::event_locked(&r.event)
    {
        anyhow::bail!("{} is part of the {pack}, which is not owned (DLC is locked: dlc.rs)", r.name);
    }
    let no_race = race.is_none();
    // The start-up flow: a plain run (no mode flag) or --front-end.
    let boot = cli.front_end
        || cli.hsm_event.is_some()
        || (cli.drive.is_none()
            && cli.vehicle.is_none()
            && camera.is_none()
            && cli.race.is_none()
            && cli.test.is_none()
            && cli.replay.is_none());
    // Where free roam starts (`mw_assets::frontend`): a continued game's car near its saved position (the
    // nearest swap spot holding it, else the default start), or a new game's default car at the default start.
    let mut boot_start: Option<(String, Vec3, f32)> = None;
    let profile_file = cli.profile.clone().or_else(frontend::default_profile_path);
    if boot && race.is_none() {
        let cfg = store.start_config().map_err(|e| anyhow::anyhow!("free-roam start data: {e}"))?;
        let profile = if cli.new_game { None } else { profile_file.as_deref().and_then(frontend::load_profile) };
        let bundle = match (&cli.drive, &profile, cfg.default_vehicle) {
            (Some(q), _, _) => Some(resolve(q)?),
            (None, Some(p), _) => Some(p.bundle.clone()),
            (None, None, Some(v)) => Some(resolve(&v.to_string())?),
            _ => None,
        };
        // A saved car from a DLC pack is not drivable (`dlc.rs`): the default car at the default start instead.
        let mut replaced = false;
        let bundle = match bundle {
            Some(b) if cli.drive.is_none() && dlc::car_locked(&b).is_some() => {
                warn!(
                    "the profile's car {b} is DLC ({}), not owned: starting in the default car",
                    dlc::car_locked(&b).unwrap_or_default()
                );
                replaced = true;
                cfg.default_vehicle.map(|v| resolve(&v.to_string())).transpose()?
            }
            other => other,
        };
        if let Some(bundle) = bundle {
            let pose = match (&profile, frontend::vehicle_id(&bundle)) {
                (Some(p), Some(v)) if !cli.new_game && !replaced => {
                    cfg.continue_pose(v, p.position.map_or([0.0; 3], |p| p.to_array()))
                }
                _ => cfg.default_start,
            };
            let pose = pose.ok_or_else(|| anyhow::anyhow!("no free-roam start point in the data"))?;
            println!(
                "free-roam start: {} at trigger {} {:?} ({})",
                bundle,
                pose.trigger,
                pose.position,
                if profile.is_some() && !cli.new_game { "continue" } else { "new game" }
            );
            let d = pose.direction;
            boot_start = Some((bundle, Vec3::from_array(pose.position), d[0].atan2(d[2])));
        }
    }
    let drive = match (&cli.drive, &race) {
        _ if boot_start.is_some() => boot_start.as_ref().map(|b| b.0.clone()),
        (Some(q), _) => Some(q.clone()),
        (None, Some(r)) => Some(r.event.default_vehicle().map(|v| v.to_string()).ok_or_else(|| {
            anyhow::anyhow!("event {} names no car (a Most Wanted race): pass --drive <car>", r.event.id)
        })?),
        (None, None) => None,
    };
    if let Some(q) = &drive {
        let bundle = resolve(q)?;
        if let Some(pack) = dlc::car_locked(&bundle) {
            anyhow::bail!("{q} is part of the {pack}, which is not owned (DLC is locked: dlc.rs)");
        }
    }
    let request = if let Some(q) = &drive {
        let (spawn, heading) = match (&race, &cli.spawn[..]) {
            (None, []) if boot_start.is_some() => {
                let (_, p, h) = boot_start.as_ref().expect("checked");
                (*p, *h)
            }
            (Some(r), []) => {
                let s = r.event.spawn.expect("checked above");
                (Vec3::from_array(s.position), s.direction[0].atan2(s.direction[2]))
            }
            (None, []) if cli.test.as_deref() == Some("freeroam") => (Vec3::from_array(freeroam::TEST_SPAWN), 0.0),
            (None, []) => (Vec3::new(-1158.0, 17.5, 1700.0), 0.0),
            (_, [x, y, z]) => (Vec3::new(*x, *y, *z), 0.0),
            (_, [x, y, z, h]) => (Vec3::new(*x, *y, *z), h.to_radians()),
            _ => anyhow::bail!("--spawn expects x,y,z[,heading_degrees]"),
        };
        // `--test ai` and `--test race` drive the streets; the other tests run on the flat track.
        let street_test = race_test
            || matches!(
                cli.test.as_deref(),
                Some("ai")
                    | Some("crash-traffic")
                    | Some("crash-ai")
                    | Some("crash-parked")
                    | Some("freeroam")
                    | Some("jackspot")
                    | Some("bodyshop")
                    | Some("pursuit")
                    | Some("cops")
            );
        // A replay with `--spawn` runs in the world from that point (kerbs and road against the original's capture).
        let track =
            cli.test_track || (cli.test.is_some() && !street_test) || (cli.replay.is_some() && cli.spawn.is_empty());
        let spawn = if track { Vec3::new(0.0, 1.0, 0.0) } else { spawn };
        let props =
            if cli.test.as_deref() == Some("smash") { props::smash_test_props(&cli.smash_set) } else { Vec::new() };
        let bundle = if q.to_ascii_uppercase().ends_with(".BNDL") { q.clone() } else { resolve(q)? };
        Request::Drive { bundle, spawn, heading, max_units: cli.units, world: !track, props }
    } else if let Some(q) = &cli.vehicle {
        Request::Vehicle { bundle: resolve(q)? }
    } else {
        // Stream nearest-first from the `--camera` pose, so `--units` loads what it sees.
        Request::World { max_units: cli.units, focus: camera.map(|c| c.translation) }
    };

    let crash = cli.test.as_deref() == Some("crash");
    let Some((camera_mode, look_back)) = camera::CameraMode::parse(&cli.camera_mode) else {
        anyhow::bail!("unknown --camera-mode {} (chase, bonnet, bumper, lookback)", cli.camera_mode);
    };
    let smash = cli.test.as_deref() == Some("smash");
    let test = match cli.test.as_deref() {
        None
        | Some("crash")
        | Some("smash")
        | Some("ai")
        | Some("race")
        | Some("crash-traffic")
        | Some("crash-ai")
        | Some("crash-parked")
        | Some("freeroam")
        | Some("jackspot")
        | Some("bodyshop")
        | Some("pursuit")
        | Some("cops") => None,
        Some("accel") => Some(drive::TestKind::Accel),
        Some("nitrous") => Some(drive::TestKind::Nitrous),
        Some("brake") => Some(drive::TestKind::Brake),
        Some("corner") => Some(drive::TestKind::Corner),
        Some("handbrake") => Some(drive::TestKind::Handbrake),
        Some("drift") => Some(drive::TestKind::Drift),
        Some("donut") => Some(drive::TestKind::Donut),
        Some("engine") => Some(drive::TestKind::Engine),
        Some("landing") => Some(drive::TestKind::Landing),
        Some("climb") => Some(drive::TestKind::Climb),
        Some("jitter") => Some(drive::TestKind::Jitter),
        Some("kerb") => Some(drive::TestKind::Kerb),
        Some("exhaust") => Some(drive::TestKind::Exhaust),
        Some(other) => {
            anyhow::bail!(
                "unknown --test {other} (accel, brake, corner, handbrake, drift, donut, engine, exhaust, landing, climb, jitter, kerb, crash, smash, ai, race, crash-traffic, crash-ai, crash-parked, freeroam, jackspot, bodyshop, pursuit, cops)"
            )
        }
    };
    if (cli.test.is_some() || cli.replay.is_some()) && drive.is_none() {
        anyhow::bail!("--test / --replay need --drive <car>");
    }
    let mut replay = cli.replay.as_deref().map(replay::Replay::load).transpose()?;
    if let Some(r) = &mut replay {
        r.world = !cli.spawn.is_empty();
    }
    let script = if cli.swap_to.is_empty() {
        None
    } else {
        let mut steps = Vec::new();
        for q in &cli.swap_to {
            if q.eq_ignore_ascii_case("free") {
                steps.push(garage::ScriptStep::FreeCamera);
                continue;
            }
            let (q, paint_index) = match q.rsplit_once('@') {
                Some((q, p)) => (q, p.parse().map_err(|_| anyhow::anyhow!("bad paint index in {q:?}@{p}"))?),
                None => (q.as_str(), 0),
            };
            let v = store
                .find_vehicle(q)?
                .ok_or_else(|| anyhow::anyhow!("no car matches {q:?} (see `mwtool vehicles`)"))?;
            let bundle = v.hi_bundle.expect("find_vehicle only returns cars with bundles");
            steps.push(garage::ScriptStep::Drive { name: garage::ascii_fold(&v.name), bundle, paint_index });
        }
        Some(garage::SwapScript::new(steps, cli.swap_delay))
    };
    // The menu is for interactive sessions in the world (or on the test track).
    let menu_enabled =
        cli.screenshot.is_none() && test.is_none() && !race_test && !matches!(request, Request::Vehicle { .. });
    let test_track = matches!(request, Request::Drive { world: false, .. });
    let filter = loader::WorldFilter {
        backdrop: match (cli.no_backdrop, cli.all_backdrops, cli.backdrop_zone) {
            (true, _, _) => loader::BackdropMode::None,
            (_, true, _) => loader::BackdropMode::All,
            (_, _, Some(unit)) => loader::BackdropMode::Zone(unit),
            _ if cli.composed_backdrop => loader::BackdropMode::Composed,
            _ => loader::BackdropMode::CameraZone,
        },
        no_groundcover: cli.no_groundcover,
        no_merge: !cli.merge_static,
        load_all: cli.load_all || (matches!(request, Request::World { .. }) && !cli.stream),
        keep_lod_proxies: cli.keep_lod_proxies,
        paint_index: cli.paint_index,
        no_prop_physics: cli.no_prop_physics && !smash,
        exclude: cli
            .exclude_models
            .iter()
            .map(|s| ResourceId::parse(s).ok_or_else(|| anyhow::anyhow!("bad resource id {s}")))
            .collect::<anyhow::Result<_>>()?,
    };
    let ai_test = cli.test.as_deref() == Some("ai");
    let police_enabled = !cli.no_police
        && matches!(request, Request::Drive { world: true, .. })
        && race.is_none()
        && cli.bench.is_none()
        && matches!(cli.test.as_deref(), None | Some("pursuit") | Some("cops"));
    let world_drive = matches!(request, Request::Drive { world: true, .. });
    let front_phase = if boot_start.is_some() && (cli.screenshot.is_none() || cli.front_end) {
        frontend::Phase::Title
    } else if world_drive && (menu_enabled || cli.show_loading) {
        frontend::Phase::Loading
    } else {
        frontend::Phase::Playing
    };
    let ai_cars = cli.ai.iter().map(|q| resolve(q)).collect::<anyhow::Result<Vec<_>>>()?;
    // The profile file this run writes (the car and its place: `frontend.rs`; free-roam progress: `freeroam.rs`).
    let save_profile = (cli.profile.is_some()
        || (world_drive
            && (menu_enabled || boot)
            && cli.front_end_shots.is_none()
            && cli.auto_start.is_none()
            // Runs without a human must not move the saved car.
            && cli.bench.is_none()
            && cli.auto_throttle.is_none()
            && cli.swap_to.is_empty()))
    .then(|| profile_file.clone())
    .flatten();
    let freeroam_profile = save_profile.clone();
    let mut app = App::new();
    app.insert_resource(car_anim::AnimOverride::parse(&cli.anim)?);
    app.insert_resource(damage::InitialDamage::parse(&cli.damage)?);
    if matches!(request, Request::Drive { world: true, .. }) {
        let balancing = match &cli.ai_balancing[..] {
            [] => None,
            [d, p] => Some((
                ResourceId::parse(d).ok_or_else(|| anyhow::anyhow!("bad resource id {d}"))?,
                ResourceId::parse(p).ok_or_else(|| anyhow::anyhow!("bad resource id {p}"))?,
            )),
            _ => anyhow::bail!("--ai-balancing expects <RaceBalancingData id>,<RaceBalancingProfile id>"),
        };
        app.add_plugins(ai::AiPlugin { cars: ai_cars, test: ai_test.then_some(cli.ai_seconds), balancing });
        if ai_test {
            app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(2.0));
        }
    } else {
        app.init_resource::<ai::Opponents>();
    }
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Most Rusted".into(),
            // In a browser: the page's canvas, sized by its parent.
            canvas: cfg!(target_arch = "wasm32").then(|| "#mostrusted".into()),
            fit_canvas_to_parent: cfg!(target_arch = "wasm32"),
            // Unthrottled frames for `--frame-stats`.
            present_mode: if cli.frame_stats || cli.bench.is_some() {
                bevy::window::PresentMode::AutoNoVsync
            } else {
                bevy::window::PresentMode::default()
            },
            ..default()
        }),
        ..default()
    }));
    // `--mute` / `MW_MUTE=1`: silence at the output too, so no voice that misses its module's own mute check is heard
    // (headless runs through `tools/hl` must never be audible). After DefaultPlugins, which inserts `GlobalVolume`.
    if cli.mute {
        app.insert_resource(bevy::audio::GlobalVolume::new(bevy::audio::Volume::SILENT));
    }
    // The game advances at most 2/60 s per frame (decomp.md "Update loop": one 2/60 catch-up step, even after
    // a stall). Bevy's default catches up 250 ms of fixed steps after a slow frame, which makes the next frames
    // slower too. The clamp is on real time, before the relative speed (`--test` runs 4x, so 8 steps there).
    // `MW_UNCAPPED_DELTA=1` restores Bevy's default (A/B).
    if std::env::var_os("MW_UNCAPPED_DELTA").is_none() {
        app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_max_delta(Duration::from_secs_f64(2.0 / 60.0)));
    }
    bevy::asset::embedded_asset!(app, "layered.wgsl");
    bevy::asset::embedded_asset!(app, "groundcover.wgsl");
    app.add_systems(Startup, (materials::load_groundcover_shader, materials::insert_game_sampler));
    if let Some(r) = replay {
        app.insert_resource(r)
            .add_systems(FixedUpdate, replay::run_replay.before(drive::car_physics))
            .add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(4.0));
    }
    if let Some(kind) = test {
        app.insert_resource(drive::DriveTest::new(kind));
        // Run the simulation faster than real time.
        app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(4.0));
    }
    if test_track {
        app.add_systems(Startup, spawn_test_track);
        app.insert_resource(props::WorldSurfaces {
            list: Vec::new(),
            untagged: cli.track_surface,
            ..Default::default()
        });
    }
    // Crash rules and the crash camera (driving only).
    if matches!(request, Request::Drive { .. }) {
        match store.crash_data() {
            Ok(data) => {
                app.insert_resource(crash::CrashConfig(Arc::new(data)));
            }
            Err(e) => warn!("no crash data ({e}): crashes and the crash camera are off"),
        }
    }
    let freeroam_test = cli.test.as_deref() == Some("freeroam");
    let freeroam_shots = if freeroam_test { cli.screenshot.clone() } else { None };
    // The crash and smash tests take their own before / after screenshots.
    let screenshot = if crash {
        app.insert_resource(damage::CrashTest::new(
            cli.screenshot.clone(),
            cli.crash_angle,
            cli.crash_distance,
            cli.crash_wall_width,
        ));
        None
    } else if let Some(kind) = match cli.test.as_deref() {
        Some("crash-traffic") => Some(car_impacts::ImpactTestKind::Traffic),
        Some("crash-ai") => Some(car_impacts::ImpactTestKind::Ai),
        Some("crash-parked") => Some(car_impacts::ImpactTestKind::Parked),
        _ => None,
    } {
        app.insert_resource(car_impacts::ImpactTest::new(kind, cli.screenshot.clone()));
        app.add_plugins(car_impacts::ImpactTestPlugin);
        None
    } else if freeroam_test {
        // The free-roam test takes its own screenshots (one after each collectable).
        None
    } else if smash {
        app.insert_resource(props::SmashTest::new(
            cli.screenshot.clone(),
            cli.smash_speed / 3.6,
            cli.smash_set == "compounds",
            props::smash_test_props(&cli.smash_set).len(),
        ));
        app.add_plugins(props::SmashTestPlugin);
        app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(2.0));
        None
    } else if matches!(cli.test.as_deref(), Some("jackspot") | Some("bodyshop")) {
        // These tests take their own before / after screenshots (`jackspots.rs`, `bodyshop.rs`).
        None
    } else {
        cli.screenshot.clone()
    };
    // `--camera` with `--drive`: a fixed camera instead of the chase camera.
    if let (Some(c), Request::Drive { world: true, .. }) = (camera, &request) {
        app.insert_resource(props::FixedCamera(c))
            .add_systems(PostUpdate, props::apply_fixed_camera.before(bevy::transform::TransformSystems::Propagate));
    }
    if cli.frame_stats {
        app.add_systems(Update, log_frame_stats);
    }
    app.add_plugins(frame_log::FrameLogPlugin);
    if let Some(seconds) = cli.bench {
        let label = std::env::args().skip(1).filter(|a| a != "--mute").collect::<Vec<_>>().join(" ");
        app.add_plugins(bench::BenchPlugin { seconds, label });
    }
    if cli.no_shadows {
        app.add_systems(PostUpdate, bench::disable_shadows);
    }
    if cli.gamepad_debug {
        app.add_systems(Update, drive::log_gamepad_events);
    }
    // The Steam Controller is read from its HID interface: not in a browser (its Gamepad API covers pads).
    #[cfg(not(target_arch = "wasm32"))]
    app.add_plugins(steam_controller::SteamControllerPlugin);
    app.add_plugins((input::MenuInputPlugin, input_script::InputScriptPlugin));
    // The environment timeline: `--timeline`, else the one painted where the view starts (the camera
    // pose or the drive spawn; the default timeline until the world view has a focus).
    let fixed_timeline =
        cli.timeline.is_some() || !matches!(request, Request::World { .. } | Request::Drive { world: true, .. });
    let start_timeline = match cli.timeline.as_deref() {
        Some("default") | None if fixed_timeline => mw_assets::environment::DEFAULT_TIMELINE,
        Some(s) => ResourceId::parse(s).ok_or_else(|| anyhow::anyhow!("bad --timeline {s}"))?,
        None => {
            let focus = match &request {
                Request::Drive { spawn, .. } => Some(*spawn),
                _ => camera.map(|c| c.translation),
            };
            focus.map_or(mw_assets::environment::DEFAULT_TIMELINE, |p| {
                store.environment_timeline_at(p.x, p.z).unwrap_or(mw_assets::environment::DEFAULT_TIMELINE)
            })
        }
    };
    let sync_environment = screenshot.is_some() || test.is_some() || crash;
    // The wet roads' planar reflection: a mirrored camera, only where there is a world to reflect.
    let reflection = cli.planar_reflection
        && cli.road_wetness > 0.0
        && matches!(request, Request::World { .. } | Request::Drive { world: true, .. });
    app.add_plugins(weather::WeatherPlugin { hud: menu_enabled })
        .add_plugins((shadow_cull::ShadowCullPlugin, pass_mask::PassMaskPlugin))
        .add_plugins(lod::LodPlugin)
        .add_plugins(prewarm::PrewarmPlugin)
        .add_plugins(reflection::PlanarReflectionPlugin { enabled: reflection })
        .add_plugins(FreeCameraPlugin)
        .add_plugins(MaterialPlugin::<GameMaterial>::default())
        .add_plugins(deferred::GameLightingPlugin)
        .add_plugins(world_lights::WorldLightsPlugin)
        .add_plugins(tonemap::GameTonemapPlugin)
        .add_plugins(flash_probe::FlashProbePlugin)
        .init_resource::<GameEnvironment>()
        .insert_resource(materials::VehicleLights::from_slice(&cli.lights))
        .add_plugins((
            avian3d::PhysicsPlugins::default().with_collision_hooks::<props::PropHooks>(),
            drive::DrivePlugin,
            damage::DamagePlugin,
            car_anim::CarAnimPlugin,
            props::PropsPlugin,
            particles::ParticlesPlugin,
            prop_vfx::PropVfxPlugin,
            crash_vfx::CrashVfxPlugin,
            sfx::SfxPlugin,
        ))
        // Avian optimises its collider trees on the async compute pool and blocks on the result later in the
        // same step; that pool also compiles pipelines, so a shader compile (or a busy machine) stalled the
        // physics step. Inline it is ~0.05 ms a step (research log 2026-10-09). `MW_AVIAN_ASYNC=1`: as before.
        .insert_resource(avian3d::collider_tree::ColliderTreeOptimization {
            use_async_tasks: std::env::var_os("MW_AVIAN_ASYNC").is_some(),
            ..default()
        })
        .add_plugins(camera::CameraPlugin { mode: camera_mode, look_back })
        .add_plugins(crash::CrashPlugin)
        .add_plugins(car_impacts::CarImpactsPlugin)
        .add_plugins(car_contacts::CarContactsPlugin)
        .add_plugins(nitrous::NitrousPlugin)
        .add_plugins(exhaust::ExhaustPlugin)
        .add_plugins(ignition::IgnitionPlugin { crank_at_spawn: cli.test.is_none() && cli.replay.is_none() })
        .add_plugins(tyre_marks::TyreMarksPlugin)
        .add_plugins(paint_wear::PaintWearPlugin)
        .add_plugins(freeroam::FreeroamPlugin {
            profile: freeroam_profile,
            new_game: cli.new_game,
            test: freeroam_test.then(|| freeroam::FreeroamTest::new(freeroam_shots)),
        })
        // Jack spots and body shops; their finds live in the profile (`frontend::Collected`).
        .insert_resource(
            profile_file
                .as_deref()
                .filter(|_| !cli.new_game)
                .and_then(frontend::load_profile)
                .map(|p| p.collected)
                .unwrap_or_default(),
        )
        .add_plugins(jackspots::JackSpotsPlugin {
            test: (cli.test.as_deref() == Some("jackspot")).then(|| cli.screenshot.clone()),
        })
        .add_plugins(bodyshop::BodyShopPlugin {
            test: (cli.test.as_deref() == Some("bodyshop")).then(|| cli.screenshot.clone()),
        })
        .add_plugins(police::PolicePlugin {
            enabled: police_enabled,
            test: cli.test.as_deref() == Some("pursuit"),
            start_test: cli.test.as_deref() == Some("cops"),
        })
        .add_plugins(race::RacePlugin { setup: race.map(Arc::new), test_runtime: race_test && cli.hsm_event.is_some() })
        .add_plugins(cutscene::CutscenePlugin { setup: cutscenes })
        .add_plugins(traffic::TrafficPlugin {
            enabled: !cli.no_traffic && matches!(request, Request::Drive { world: true, .. }),
            density: cli.traffic_density.max(0.0),
        })
        .add_plugins((garage::GaragePlugin { script }, menu::MenuPlugin { enabled: menu_enabled, open: cli.show_menu }))
        .add_plugins(easydrive::EasyDrivePlugin {
            enabled: menu_enabled || cli.show_easydrive,
            open: cli.show_easydrive,
            path: cli.easydrive_path.clone(),
        })
        .insert_resource(frontend::Residency::new(world_drive || matches!(request, Request::World { .. })))
        .add_plugins(frontend::FrontEndPlugin {
            start: front_phase,
            // The shots run has nobody to press a key: it starts 1 s after the prompt unless told otherwise.
            auto_start: cli.auto_start.or(cli.front_end_shots.as_ref().map(|_| 1.0)),
            shots: cli.front_end_shots.clone(),
            profile: save_profile.clone(),
            driven: cli.hsm,
        })
        .add_plugins(dlc::DlcPlugin { free_roam: no_race && matches!(request, Request::Drive { .. }) })
        .add_plugins(world_logic::WorldLogicPlugin { driven: cli.hsm })
        .add_plugins(logic::LogicPlugin {
            enabled: cli.hsm,
            event: hsm_event,
            new_profile: cli.new_game || profile_file.as_deref().and_then(frontend::load_profile).is_none(),
            profile: profile_file.clone().filter(|_| !cli.new_game),
            volume: cli.volume.clone(),
        })
        .add_plugins(pause::PausePlugin {
            enabled: (menu_enabled || cli.show_pause) && matches!(request, Request::Drive { .. }),
            open: cli.show_pause && !cli.pause_map,
            map: false,
        })
        .add_plugins(map_screen::MapScreenPlugin {
            enabled: (menu_enabled
                || cli.show_pause
                || cli.show_map
                || cli.map_start.is_some()
                || cli.map_waypoint.is_some())
                && matches!(request, Request::Drive { .. }),
            open: cli.show_map || (cli.show_pause && cli.pause_map),
            select: cli.map_select,
            options: cli.map_options,
            start: cli.map_start,
            waypoint: cli.map_waypoint,
        })
        .add_plugins(event_card::EventCardPlugin)
        .add_plugins(ui_sequence::UiSequencePlugin)
        .add_plugins(gps::GpsPlugin)
        .add_plugins(game_text::GameTextPlugin)
        .add_plugins(ui_quads::UiQuadsPlugin)
        .add_plugins(window_test::WindowTestPlugin)
        .add_plugins((world_ui::WorldUiPlugin, gate_marker::GateMarkerPlugin, event_starts::EventStartsPlugin))
        .add_plugins(hud::HudPlugin {
            enabled: !cli.no_hud && test.is_none() && !crash && !smash && !matches!(request, Request::Vehicle { .. }),
            metric: !cli.mph,
        })
        .add_plugins(settings::SettingsPlugin {
            settings: settings::load(cli.volume.as_deref()),
            open: cli.show_settings,
        })
        .add_plugins((
            car_audio::CarAudioPlugin,
            car_sfx::CarSfxPlugin,
            player_sfx::PlayerSfxPlugin,
            world_sfx::WorldSfxPlugin,
        ))
        .add_plugins(police_fx::PoliceFxPlugin)
        .add_plugins(coronas::CoronasPlugin)
        .add_plugins((
            world_coronas::WorldCoronasPlugin,
            sequence_fx::SequenceFxPlugin,
            traffic_lamps::TrafficLampsPlugin,
        ))
        .add_plugins(car_lights::CarLightsPlugin)
        .add_plugins(audio::GameAudioPlugin)
        .add_plugins(ui_sound::UiSoundPlugin)
        .add_plugins(audio_tap::AudioTapPlugin)
        .add_plugins(music::MusicPlugin {
            enabled: std::env::var("MW_MUSIC").is_ok_and(|v| v == "1")
                || (!cli.no_music
                    && !cli.mute
                    && screenshot.is_none()
                    && test.is_none()
                    && !race_test
                    && !crash
                    && !smash
                    && cli.replay.is_none()),
        })
        .insert_resource(drive::CarInput { auto_throttle: cli.auto_throttle, auto_nitrous: cli.nitrous, ..default() })
        .insert_resource(mods_menu::ModSelection::parse(&cli.mods, &[]))
        .init_resource::<mods_menu::ModCatalogue>()
        .add_systems(Update, mods_menu::apply_live)
        .insert_resource(Game {
            store,
            request,
            camera,
            fov: cli.fov,
            screenshot,
            screenshot_delay: cli.screenshot_delay,
            screenshot_time: cli.screenshot_time,
            screenshot_after_impact: cli.screenshot_after_impact,
            screenshot_after_backfire: cli.screenshot_after_backfire,
            environment: weather::EnvironmentKey {
                timeline: start_timeline,
                hour: cli.hour.rem_euclid(24.0),
                alternate: cli.weather,
            },
            fixed_timeline,
            road_wetness: cli.road_wetness.max(0.0),
            sync_environment,
            paint,
            filter,
            mute: cli.mute || ((test.is_some() || crash || smash || race_test) && !cli.sound),
            start_probe: if boot_start.is_some() && cli.spawn.is_empty() { 3.0 } else { 30.0 },
        })
        .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.85)))
        .init_resource::<Loaded>()
        .init_resource::<groundcover::GroundcoverResidency>()
        .init_resource::<zone_view::ZoneView>()
        .init_resource::<streaming::StreamRegistry>()
        .init_resource::<streaming::PlacementFocus>()
        .add_systems(Startup, (setup, cpu_light_clustering))
        .add_systems(Update, (start_loader, receive_assets, update_status, auto_screenshot))
        .add_systems(
            Update,
            (streaming::send_focus, streaming::despawn_streamed, streaming::streaming_stall).after(receive_assets),
        )
        .add_systems(
            PostUpdate,
            (zone_view::zone_view_visibility, groundcover::groundcover_visibility)
                .chain()
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        )
        .add_systems(Update, (drive_vehicle_lights, apply_vehicle_lights).chain());
    // The main world's schedules run their systems one after another on the main thread (the render world and
    // the systems' own parallel queries still use the task pool). Spreading the main world's many small systems
    // over the pool only paid off on an idle machine (equal there); under contention every dispatch waits for a
    // worker to be scheduled: main world 16.7 -> 12.8 ms, a physics step 5.1 -> 3.6 ms with 16 niced busy loops
    // (research log 2026-10-10). Same results (handling and race tests). `MW_MAIN_MULTI=1`: Bevy's default.
    if std::env::var_os("MW_MAIN_MULTI").is_none() {
        use bevy::ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};
        let labels: [bevy::ecs::intern::Interned<dyn ScheduleLabel>; 12] = [
            First.intern(),
            PreUpdate.intern(),
            RunFixedMainLoop.intern(),
            FixedFirst.intern(),
            FixedPreUpdate.intern(),
            FixedUpdate.intern(),
            FixedPostUpdate.intern(),
            FixedLast.intern(),
            Update.intern(),
            SpawnScene.intern(),
            PostUpdate.intern(),
            Last.intern(),
        ];
        for label in labels {
            app.edit_schedule(label, |s| {
                s.set_executor(SingleThreadedExecutor::new());
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    web::install_panic_hook();
    app.run();
    Ok(())
}

use weather::GAME_UNIT;

/// Light clustering on the CPU (Bevy's fallback for devices without compute) instead of the GPU: there are no
/// point, spot or probe lights to cluster (the key light is directional, the sky light is the view's environment
/// map), but GPU clustering rebuilt its per-view buffers and bind groups and encoded its passes every frame.
/// The render world copies the setting from the main world each frame. `MW_GPU_CLUSTERING=1`: Bevy's default.
fn cpu_light_clustering(mut settings: ResMut<bevy::light::cluster::GlobalClusterSettings>) {
    if std::env::var_os("MW_GPU_CLUSTERING").is_none() {
        settings.gpu_clustering = None;
    }
}

fn setup(mut commands: Commands, game: Res<Game>, mut images: ResMut<Assets<Image>>) {
    // The game renders deferred: the camera keeps a depth + deferred G-buffer prepass (which the game
    // materials fill with the game's G-buffer) and renders in HDR.
    let camera = commands
        .spawn((
            Camera3d::default(),
            bevy::core_pipeline::prepass::DepthPrepass,
            bevy::core_pipeline::prepass::DeferredPrepass,
            bevy::camera::Hdr,
            // The game's tonemap and auto exposure replace Bevy's (`tonemap.rs`); this exposure makes the
            // HDR target hold the game's units (view.exposure = 1 / GAME_UNIT).
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy::camera::Exposure { ev100: (GAME_UNIT / 1.2).log2() },
            Msaa::Off,
            Transform::from_xyz(6.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.6, 0.0), Vec3::Y),
            Projection::Perspective(PerspectiveProjection {
                far: 20_000.0,
                // `--fov` is horizontal (the game's convention); the window is 16:9 (1280 × 720).
                fov: game.fov.map_or(std::f32::consts::FRAC_PI_4, |f| camera::projection_fov(f, 16.0 / 9.0)),
                ..default()
            }),
        ))
        .id();
    if indirect_drawing_off() {
        commands.entity(camera).insert(bevy::render::view::NoIndirectDrawing);
    }
    // Time of day and weather from the environment timeline (`weather.rs`): key light, ambient rig, sky,
    // fog and colour grade; rebuilt when the hour, the weather or the painted timeline changes.
    let state = weather::EnvironmentState::new(
        game.store.clone(),
        game.environment,
        !game.fixed_timeline,
        game.sync_environment,
    );
    weather::spawn_environment(&mut commands, &mut images, state, camera, game.road_wetness);
    // Screenshot runs must be deterministic: no input (gamepad drift moves the camera).
    if game.screenshot.is_none() && !matches!(game.request, Request::Drive { .. }) {
        commands.entity(camera).insert(FreeCamera { walk_speed: 30.0, run_speed: 150.0, ..default() });
    }
    commands.spawn((
        Text::new("Most Rusted"),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), left: Val::Px(8.0), ..default() },
        StatusText,
    ));
}

/// Draw every view with direct draws (GPU mesh preprocessing still on) instead of Bevy's multi-draw-indirect
/// path. That path keeps per batch set (one per material bind group: ~950 materials are visible downtown) GPU
/// buffers, a bin-unpacking dispatch and bind groups in every view, which cost most of the render thread's time
/// (research log 2026-10-09, frame drops). `MW_INDIRECT=1` turns it back on for A/B runs.
pub fn indirect_drawing_off() -> bool {
    std::env::var_os("MW_INDIRECT").is_none()
}

/// Start streaming once the renderer reports which compressed formats work.
fn start_loader(
    mut commands: Commands,
    game: Res<Game>,
    support: Option<Res<CompressedImageFormatSupport>>,
    channel: Option<Res<LoaderChannel>>,
    mut loaded: ResMut<Loaded>,
) {
    if channel.is_some() {
        return;
    }
    let Some(support) = support else { return };
    let bc = support.0.contains(CompressedImageFormats::BC);
    loaded.started = Some(Instant::now());
    let (channel, requests, focus) = loader::spawn(game.store.clone(), game.request.clone(), bc, game.filter.clone());
    commands.insert_resource(channel);
    commands.insert_resource(requests);
    commands.insert_resource(focus);
}

/// Turn loader messages into assets and entities, within a per-frame budget.
#[allow(clippy::too_many_arguments)]
fn receive_assets(
    mut commands: Commands,
    channel: Option<Res<LoaderChannel>>,
    mut loaded: ResMut<Loaded>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GameMaterial>>,
    mut camera: Query<&mut Transform, MainCamera>,
    game: Res<Game>,
    env: Res<GameEnvironment>,
    lights: Res<materials::VehicleLights>,
    mut garage: ResMut<garage::Garage>,
    mut opponents: ResMut<ai::Opponents>,
    mut surfaces: ResMut<props::WorldSurfaces>,
    (mut traffic_models, mut parked_models): (Option<ResMut<traffic::TrafficModels>>, ResMut<jackspots::ParkedModels>),
    (mut residency, mut zone_view): (ResMut<groundcover::GroundcoverResidency>, ResMut<zone_view::ZoneView>),
    (mut streamed, mut registry): (ResMut<frontend::Residency>, ResMut<streaming::StreamRegistry>),
) {
    let Some(channel) = channel else { return };
    let rx = channel.0.lock().unwrap();
    let begin = Instant::now();
    let deadline = begin + Duration::from_millis(8);
    let mut applied = 0u64;
    while Instant::now() < deadline {
        let Ok(msg) = rx.try_recv() else { break };
        applied += 1;
        match msg {
            Msg::Focus(p) => {
                if let Ok(mut t) = camera.single_mut() {
                    if let Some(c) = game.camera {
                        *t = c;
                    } else if matches!(game.request, Request::World { .. }) {
                        *t = Transform::from_translation(p + Vec3::new(0.0, 120.0, 250.0)).looking_at(p, Vec3::Y);
                    }
                }
            }
            Msg::Texture(id, img) => {
                let h = images.add(img);
                loaded.textures.insert(id, h);
            }
            Msg::Model(id, parts) => {
                let mut handles = Vec::with_capacity(parts.len());
                for part in parts {
                    let mat = match loaded.materials.get(&part.look) {
                        Some(m) => m.clone(),
                        None => {
                            let tex = |k: Option<TexKey>| k.and_then(|k| loaded.textures.get(&k).cloned());
                            let m = materials.add(part.look.material(tex, game.paint, &env, &lights));
                            loaded.materials.insert(part.look, m.clone());
                            m
                        }
                    };
                    if part.look.blend == materials::Blend::Mask {
                        loaded.mask_materials.insert(mat.id());
                    }
                    handles.push((meshes.add(part.mesh), mat));
                }
                loaded.models.insert(id, handles);
            }
            Msg::ShadowRange(id, d) => {
                loaded.shadow_ranges.insert(id, d);
            }
            Msg::PassMask(id, m) => {
                loaded.pass_masks.insert(id, m);
            }
            Msg::Instance { model, transform, lod } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                // Groundcover (model keys 0x6…, `loader::groundcover`) casts no shadows.
                let casts_shadow = model.0 >> 60 != 6;
                // The LODs' meshes, each with the mask of LODs it is drawn at (`lod.rs`).
                let sets = lod::mesh_sets(lod.as_ref(), (model, parts), &loaded.models);
                let mut lod_meshes = Vec::new();
                let mut root = commands.spawn((Transform::from_matrix(transform), Visibility::default()));
                root.with_children(|c| {
                    for (key, parts, mask) in &sets {
                        let range = loaded.shadow_ranges.get(key).copied().map(shadow_cull::ShadowRange);
                        for (mesh, mat) in parts {
                            let class = if loaded.mask_materials.contains(&mat.id()) {
                                shadow_cull::ShadowClass::Alpha
                            } else if key.0 >> 62 != 0 {
                                shadow_cull::ShadowClass::Backdrop
                            } else {
                                shadow_cull::ShadowClass::World
                            };
                            let mut e = c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), class));
                            if let Some(r) = range {
                                e.insert(r);
                            }
                            if !casts_shadow {
                                e.insert(bevy::light::NotShadowCaster);
                            }
                            if lod.is_some() {
                                e.insert((lod::LodShadow(true), lod::initial_visibility(*mask)));
                                lod_meshes.push((e.id(), *mask));
                            }
                        }
                    }
                });
                if let Some(l) = lod {
                    root.insert(lod::ModelLod::new(l.table, l.scale, lod_meshes));
                }
                if let Some(&m) = loaded.pass_masks.get(&model) {
                    root.insert(pass_mask::PassMask(m));
                }
                if let Some(tag) = &loaded.zone_tag {
                    root.insert(tag.clone());
                }
                registry.register(root.id());
                loaded.instances += 1;
            }
            Msg::ZoneTag(tag) => loaded.zone_tag = tag,
            Msg::Zones(zones) => {
                zone_view.set_zones(zones.clone());
                streamed.set_zones(zones.clone());
                residency.set_zones(zones);
            }
            Msg::UnitLoaded(unit) => {
                registry.begin_unit(None);
                streamed.unit_loaded(unit);
                zone_view.unit_loaded(unit);
            }
            Msg::UnitBegin(unit) => registry.begin_unit(Some(unit)),
            Msg::StreamKey(key) => registry.set_key(key),
            Msg::UnitUnloaded(u) => {
                let unit = u.unit;
                streamed.unit_unloaded(unit);
                zone_view.unit_unloaded(unit);
                streaming::unloaded(*u, &mut registry, &mut loaded);
                commands.queue(move |world: &mut World| {
                    world.resource_mut::<world_lights::LightSources>().remove_unit(unit)
                });
            }
            Msg::Streaming => {
                registry.streaming = true;
                zone_view.set_streaming();
            }
            Msg::GroundcoverListing { unit, collections } => residency.add_listing(unit, collections),
            Msg::Groundcover { model, collection, min, max, range, bounds } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                let range = groundcover::GroundcoverRange { collection, min, max, range };
                // The vertex shader expands each sprite from its base point (`groundcover.wgsl`), so the
                // mesh's own bounds are too small: the loader's cover every turned, swaying sprite.
                let aabb = bevy::camera::primitives::Aabb::from_min_max(bounds.0, bounds.1);
                let mut root = commands.spawn((Transform::IDENTITY, Visibility::Hidden, range));
                registry.register_groundcover(collection, root.id());
                root.with_children(|c| {
                    for (mesh, mat) in parts {
                        c.spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(mat.clone()),
                            bevy::light::NotShadowCaster,
                            aabb,
                            bevy::camera::visibility::NoAutoAabb,
                        ));
                    }
                });
                loaded.instances += 1;
            }
            Msg::Prop { model, transform, physics, pieces, lod, standing, found } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                let range = loaded.shadow_ranges.get(&model).copied().map(shadow_cull::ShadowRange);
                let get = |k: &Option<mw_bundle::ResourceId>| k.and_then(|k| loaded.models.get(&k).cloned());
                let pieces = pieces.map(|(parts, rest)| props::CompoundMeshes {
                    parts: parts.iter().map(get).collect(),
                    rest: get(&rest),
                });
                let with_ranges = |sets: Vec<(mw_bundle::ResourceId, ModelParts, u8)>| -> Vec<_> {
                    sets.into_iter()
                        .map(|(k, parts, mask)| {
                            (parts, mask, loaded.shadow_ranges.get(&k).copied().map(shadow_cull::ShadowRange))
                        })
                        .collect()
                };
                let sets = with_ranges(lod::mesh_sets(lod.as_ref(), (model, parts), &loaded.models));
                // A billboard's second standing model (the found one), with its own LODs.
                let found = found.and_then(|(key, lod)| {
                    let parts = loaded.models.get(&key)?;
                    Some(props::MeshSets {
                        sets: with_ranges(lod::mesh_sets(lod.as_ref(), (key, parts), &loaded.models)),
                        lod: lod.map(|l| (l.table, l.scale)),
                    })
                });
                let root = props::spawn_prop(
                    &mut commands,
                    physics,
                    Transform::from_matrix(transform),
                    &sets,
                    range,
                    pieces,
                    lod.map(|l| (l.table, l.scale)),
                    standing,
                    found,
                );
                if let Some(&m) = loaded.pass_masks.get(&model) {
                    commands.entity(root).insert(pass_mask::PassMask(m));
                }
                registry.register(root);
                loaded.instances += 1;
                loaded.props += 1;
            }
            Msg::Collider(c, tags) => {
                let e = commands.spawn((avian3d::prelude::RigidBody::Static, c, props::SurfaceTags(tags))).id();
                registry.register(e);
            }
            Msg::Surfaces(list) => surfaces.list = list,
            Msg::Lights(lights) => {
                let unit = registry.unit();
                commands.queue(move |world: &mut World| {
                    world.resource_mut::<world_lights::LightSources>().add_unit(unit, lights)
                });
            }
            Msg::PropResponses(r) => {
                info!("prop contact responses: {r:?}");
                surfaces.responses = r;
            }
            Msg::Car(spec) => {
                let parts = spec
                    .body
                    .iter()
                    .chain(spec.wheels.iter().flat_map(|w| w.parts.iter().map(|(m, _)| m)))
                    .filter_map(|m| Some((*m, loaded.models.get(m)?.clone())))
                    .collect();
                // An opponent's car goes to the AI (`ai.rs`); the player's is placed by the garage: at `--spawn` at
                // startup, later in place of the current car.
                match spec.opponent {
                    Some(slot) => opponents.arrived(slot, spec, parts),
                    None => garage.arrived(spec, parts),
                }
            }
            Msg::TrafficCar(spec) => {
                // A jack spot's parked car (`jackspots.rs`) or a traffic model.
                if spec.vehicle >= jackspots::MODEL_KEY_BASE {
                    parked_models.arrived(&spec, &loaded.models);
                } else if let Some(t) = traffic_models.as_mut() {
                    t.arrived(&spec, &loaded.models);
                }
            }
            Msg::CarFailed(e) => garage.failed(e),
            Msg::Progress(s) => {
                info!("{s}");
                loaded.status = s;
            }
            Msg::Done => {
                loaded.done = true;
                // While streaming, residency follows the loaded units (`streaming.rs`).
                streamed.all_sent = !registry.streaming;
                if let Some(t) = loaded.started {
                    info!(
                        "loading finished in {:.1?}: {} instances, {} with physics",
                        t.elapsed(),
                        loaded.instances,
                        loaded.props
                    );
                }
            }
        }
    }
    if applied > 0 {
        use std::sync::atomic::Ordering;
        frame_log::LOADER_MSGS.fetch_add(applied, Ordering::Relaxed);
        frame_log::LOADER_NS.fetch_add(begin.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

fn update_status(
    loaded: Res<Loaded>,
    mut text: Query<&mut Text, With<StatusText>>,
    cam: Query<&Transform, MainCamera>,
    car: Query<&drive::Car, With<drive::Player>>,
) {
    #[cfg(target_arch = "wasm32")]
    web::report(&loaded.status, loaded.done);
    let Ok(mut text) = text.single_mut() else { return };
    let pos = cam.single().map(|t| t.translation).unwrap_or_default();
    let controls = match car.single() {
        Ok(car) => format!(
            "{:.0} km/h\nA / Z throttle / brake, arrows steer, space handbrake, left shift nitrous, R reset (pad: RT / LT, stick, X, A)\nC / LB: camera, End / right stick back: look back, right stick or right mouse drag: look around\nE / numpad 6 / d-pad right: Easydrive",
            car.speed.abs() * 3.6
        ),
        Err(_) => "WASD/QE move, right mouse or right stick look, shift run, scroll speed".to_string(),
    };
    let controls = format!("{controls}\nEsc / Tab / F1 / gamepad Start: car menu");
    text.0 = format!(
        "Most Rusted - {}\n{} models, {} textures, {} instances{}\ncamera {:.0} {:.0} {:.0}\n{controls}",
        loaded.status,
        loaded.models.len(),
        loaded.textures.len(),
        loaded.instances,
        if loaded.done { "" } else { " (loading...)" },
        pos.x,
        pos.y,
        pos.z
    );
}

/// Brake lights follow the brake pedal and reversing lights the reverse gear while driving.
fn drive_vehicle_lights(
    input: Res<drive::CarInput>,
    car: Query<&drive::Car, With<drive::Player>>,
    mut lights: ResMut<materials::VehicleLights>,
) {
    let Ok(car) = car.single() else { return };
    let mut mask = lights.base;
    mask.x = mask.x.max(if input.brake > 0.05 { 1.0 } else { 0.0 });
    mask.z = mask.z.max(if car.gear < 0 { 1.0 } else { 0.0 });
    if lights.mask != mask {
        lights.mask = mask;
    }
}

/// Push the light state into the materials that read it (lightmapped lights, lamp interiors, glass).
fn apply_vehicle_lights(
    lights: Res<materials::VehicleLights>,
    loaded: Res<Loaded>,
    mut materials: ResMut<Assets<GameMaterial>>,
) {
    if !lights.is_changed() {
        return;
    }
    for (look, handle) in &loaded.materials {
        if look.uses_lights()
            && let Some(mut m) = materials.get_mut(handle)
        {
            m.extension.params.lights = lights.mask;
        }
    }
}

/// `--screenshot`: wait for loading plus a few frames for GPU uploads, capture,
/// then exit.
#[allow(clippy::too_many_arguments)]
fn auto_screenshot(
    mut commands: Commands,
    game: Res<Game>,
    loaded: Res<Loaded>,
    script: Option<Res<garage::SwapScript>>,
    time: Res<Time<Virtual>>,
    impact: Option<Res<particles::FirstImpact>>,
    backfire: Option<Res<exhaust::FirstBackfire>>,
    mut frames_after_done: Local<u32>,
    real: Res<Time<Real>>,
    mut since_done: Local<f32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &game.screenshot else { return };
    // With `--swap-to`, capture after the last swap.
    if !loaded.done || script.is_some_and(|s| !s.finished()) {
        return;
    }
    let at = match (game.screenshot_time, game.screenshot_after_impact, game.screenshot_after_backfire) {
        (_, Some(after), _) => Some(impact.map_or(f32::MAX, |i| i.0 + after)),
        (_, None, Some(after)) => Some(backfire.map_or(f32::MAX, |b| b.0 + after)),
        (at, None, None) => at,
    };
    if let Some(at) = at {
        if *frames_after_done == 0 && time.elapsed_secs() >= at {
            *frames_after_done = game.screenshot_delay.max(1);
        } else if *frames_after_done == 0 {
            return;
        } else {
            *frames_after_done += 1;
        }
        match *frames_after_done {
            n if n == game.screenshot_delay.max(1) => {
                info!("screenshot at {:.2} s game time", time.elapsed_secs());
                commands.spawn(Screenshot::primary_window()).observe(platform::save_screenshot(path.clone()));
            }
            n if n == game.screenshot_delay.max(1) + 30 => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
        return;
    }
    // The delay counts frames at 60 per second: on a machine that renders faster it also waits that long in
    // real time, so late arrivals (the viewer's car, textures) are in the picture.
    *since_done += real.delta_secs();
    if *frames_after_done + 1 == game.screenshot_delay && *since_done < game.screenshot_delay as f32 / 60.0 {
        return;
    }
    *frames_after_done += 1;
    match *frames_after_done {
        n if n == game.screenshot_delay => {
            commands.spawn(Screenshot::primary_window()).observe(platform::save_screenshot(path.clone()));
        }
        n if n == game.screenshot_delay + 30 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// A large flat plane for handling tests.
fn spawn_test_track(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    test: Option<Res<drive::DriveTest>>,
) {
    use avian3d::prelude::{Collider, RigidBody};
    // Ramps for the landing and climb tests, a trimesh like the world's collision.
    if let Some((vertices, indices)) = test.and_then(|t| drive::test_obstacle(t.kind)) {
        let mut mesh =
            Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices.iter().map(|v| v.to_array()).collect::<Vec<_>>());
        mesh.insert_indices(bevy::mesh::Indices::U32(indices.iter().flatten().copied().collect()));
        mesh.compute_smooth_normals();
        commands.spawn((
            RigidBody::Static,
            // As the world's colliders (`loader.rs`).
            Collider::trimesh_with_config(vertices, indices, avian3d::prelude::TrimeshFlags::FIX_INTERNAL_EDGES),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.5, 0.45, 0.4), ..default() })),
        ));
    }
    commands.spawn((RigidBody::Static, Collider::cuboid(40_000.0, 1.0, 40_000.0), Transform::from_xyz(0.0, -0.5, 0.0)));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40_000.0, 40_000.0))),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.35, 0.36, 0.38), ..default() })),
    ));
    // Distance markers every 100 m along +Z.
    let marker = meshes.add(Cuboid::new(30.0, 0.02, 1.0));
    let white = materials.add(StandardMaterial { base_color: Color::WHITE, ..default() });
    for i in 0..400 {
        commands.spawn((
            Mesh3d(marker.clone()),
            MeshMaterial3d(white.clone()),
            Transform::from_xyz(0.0, 0.01, i as f32 * 100.0),
        ));
    }
}

/// `--frame-stats`: mean and worst frame time over 5 s windows after loading.
fn log_frame_stats(
    time: Res<Time<Real>>,
    loaded: Res<Loaded>,
    props: Res<props::PropStats>,
    mut acc: Local<(f32, f32, u32)>,
) {
    if !loaded.done {
        return;
    }
    let dt = time.delta_secs();
    acc.0 += dt;
    acc.1 = acc.1.max(dt);
    acc.2 += 1;
    if acc.0 >= 5.0 {
        info!(
            "frame stats: {:.2} ms mean, {:.2} ms worst over {} frames ({} instances, {} with physics, {} active)",
            acc.0 * 1000.0 / acc.2 as f32,
            acc.1 * 1000.0,
            acc.2,
            loaded.instances,
            loaded.props,
            props.active
        );
        *acc = (0.0, 0.0, 0);
    }
}
