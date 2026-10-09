//! Most Rusted: an open-source runtime for Need for Speed: Most Wanted (2012).
//!
//! No game data is shipped. All assets are streamed at runtime from an
//! installed copy of the game (see `mw-gamedir` for how it is located).

mod ai;
mod ai_driver;
mod audio;
mod bench;
mod camera;
mod car_anim;
mod chassis;
mod chevron;
use mw_handling::{drift, drivetrain, tyre};
mod convert;
mod crash;
mod cutscene;
mod damage;
mod deferred;
mod drive;
mod easydrive;
mod exhaust;
mod freeroam;
mod game_text;
mod garage;
mod hud;
mod input;
mod loader;
mod materials;
mod menu;
mod mods_menu;
mod particles;
mod props;
mod race;
mod reflection;
mod replay;
mod settings;
mod sfx;
mod shadow_cull;
mod sky;
mod tonemap;
mod traffic;
mod tyre_marks;
mod weather;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
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
    /// Spawn point for `--drive` as `x,y,z[,heading_degrees]` (default: a road west of downtown).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true, default_value = "-1158,17.5,1700")]
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
    /// Run a scripted test and exit: accel, brake, corner, handbrake, drift, donut, engine, crash (drive into a
    /// wall; with --screenshot, saves `<name>-before` / `<name>-after` images), or smash (drive through a cone,
    /// a street lamp and a fence into an immovable wall made of the game's props, at --smash-speed; before /
    /// after screenshots like crash). Implies --test-track.
    #[arg(long)]
    test: Option<String>,
    /// Replay a recording of the original (`mwtool capture`): drive with its recorded inputs from its starting
    /// speed on the test track, write our telemetry next to it (`<file>.ours.csv`) and exit. Compare with
    /// `mwtool compare <file> <file>.ours.csv`.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Smash test: speed (km/h) the car holds through the props.
    #[arg(long, default_value_t = 60.0)]
    smash_speed: f32,
    /// Give props, compound and dynamic objects collision (smashable street furniture; `props.rs`).
    /// Off by default while it is work in progress: knocked lamps and crash barrels can still wedge the
    /// car (research log, props session). `--test smash` turns it on.
    #[arg(long)]
    prop_physics: bool,
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
    /// building stand-ins overlap the detailed world). The default composes one backdrop per place from the
    /// zone that owns it.
    #[arg(long, conflicts_with_all = ["no_backdrop", "backdrop_zone"])]
    all_backdrops: bool,
    /// Debug: draw only this track unit's backdrop, as the game does while the player is in that unit.
    #[arg(long, conflicts_with = "no_backdrop")]
    backdrop_zone: Option<u32>,
    /// Debug: with --all-backdrops or --backdrop-zone, also draw backdrop LOD2 stand-ins.
    #[arg(long)]
    keep_lod_proxies: bool,
    /// Debug: skip these model resource IDs (comma separated).
    #[arg(long, value_delimiter = ',')]
    exclude_models: Vec<String>,
    /// Disable the background music.
    #[arg(long)]
    no_music: bool,
    /// Disable all audio.
    #[arg(long)]
    mute: bool,
    /// Volumes for this run, over the saved settings: `master=0.8,engine=1,sfx=0.5,music=0.3` (0..1 or %).
    /// F2 / gamepad Select opens the volume settings in game (saved to ~/.config/mostrusted/settings.cfg).
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
    /// Limit the number of streamed track units (nearest first).
    #[arg(long)]
    units: Option<usize>,
    /// Camera pose as `x,y,z,look_x,look_y,look_z` (overrides the default view; with --drive in the world, a
    /// fixed camera instead of the chase camera).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    camera: Option<Vec<f32>>,
    /// Body paint colour as `r,g,b` in 0–255 sRGB, overriding the car's own paint colour.
    #[arg(long, value_delimiter = ',')]
    paint: Option<Vec<u8>>,
    /// Which of the car's paint options (`VehiclePaint` objects, in ID order) to use.
    #[arg(long, default_value_t = 0)]
    paint_index: usize,
    /// Car dirt as `dirt[,dust]` (0–1, the paint shaders' GenericVector4_A.xy; dust defaults to dirt).
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
    screenshot: Option<PathBuf>,
    screenshot_delay: u32,
    screenshot_time: Option<f32>,
    screenshot_after_impact: Option<f32>,
    screenshot_after_backfire: Option<f32>,
    paint: Option<Color>,
    filter: loader::WorldFilter,
    mute: bool,
}

/// The player's view: every `Camera3d` except the planar reflection's mirrored camera (`reflection.rs`).
pub type MainCamera = (With<Camera3d>, Without<reflection::PlanarReflectionCamera>);

/// A groundcover group's box and draw distance (`loader::Msg::Groundcover`).
#[derive(Component)]
struct GroundcoverRange {
    min: Vec3,
    max: Vec3,
    range: f32,
}

/// The game's group test (`FUN_00eb9500`): a group is drawn while the distance from the camera to the
/// nearest point of its box is at most 125 x its radius (`Group::radius`). The game also drops and shrinks
/// single sprites near their own range (`GroundcoverInstance::fade`); merged group meshes cannot do that.
fn groundcover_distance(
    camera: Query<&GlobalTransform, MainCamera>,
    mut groups: Query<(&GroundcoverRange, &mut Visibility)>,
) {
    let Some(eye) = camera.iter().next().map(|t| t.translation()) else { return };
    for (g, mut vis) in &mut groups {
        let d = eye.clamp(g.min, g.max).distance(eye);
        let want = if d <= g.range { Visibility::Inherited } else { Visibility::Hidden };
        vis.set_if_neq(want);
    }
}

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
    instances: usize,
    /// Placements with physics (`props.rs`).
    props: usize,
    status: String,
    pub(crate) done: bool,
    started: Option<Instant>,
}

/// The debug status text (top left); hidden behind the HUD while driving unless F3 is pressed.
#[derive(Component)]
pub struct StatusText;

fn main() -> anyhow::Result<()> {
    drive::install_gamepad_mappings();
    let cli = Cli::parse();
    if cli.bench.is_some() && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
        // Wayland offers no Immediate present mode and paces Mailbox at the monitor's refresh: benchmark
        // through XWayland, where `AutoNoVsync` gets Immediate (`bench.rs`).
        // SAFETY: no other thread has started yet.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
    let game = GameDir::locate(cli.game_dir.as_deref())?;
    println!("Most Wanted install: {} (version {})", game.root().display(), game.version().unwrap_or("?"));
    let t = Instant::now();
    let store = Arc::new(AssetStore::new(game)?);
    println!("indexed {} resources in {:?}", store.resource_count(), t.elapsed());

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
    if race_test && cli.race.is_none() {
        anyhow::bail!("--test race needs --race <event id>");
    }
    // `--race`: the event's start and (unless --drive names one) its first allowed car.
    let race = match cli.race {
        None => None,
        Some(id) => {
            let events = store.events()?;
            let event =
                events.get(id).cloned().ok_or_else(|| anyhow::anyhow!("no event {id} (see `mwtool events`)"))?;
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
            Some(race::RaceSetup { event, game_mode, name, test: race_test })
        }
    };
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
    let drive = match (&cli.drive, &race) {
        (Some(q), _) => Some(q.clone()),
        (None, Some(r)) => Some(r.event.default_vehicle().map(|v| v.to_string()).ok_or_else(|| {
            anyhow::anyhow!("event {} names no car (a Most Wanted race): pass --drive <car>", r.event.id)
        })?),
        (None, None) => None,
    };
    let request = if let Some(q) = &drive {
        let (spawn, heading) = match (&race, &cli.spawn[..]) {
            (Some(r), _) => {
                let s = r.event.spawn.expect("checked above");
                (Vec3::from_array(s.position), s.direction[0].atan2(s.direction[2]))
            }
            (None, [x, y, z]) => (Vec3::new(*x, *y, *z), 0.0),
            (None, [x, y, z, h]) => (Vec3::new(*x, *y, *z), h.to_radians()),
            _ => anyhow::bail!("--spawn expects x,y,z[,heading_degrees]"),
        };
        // `--test ai` and `--test race` drive the streets; the other tests run on the flat track.
        let street_test = race_test || cli.test.as_deref() == Some("ai");
        let track = cli.test_track || (cli.test.is_some() && !street_test) || cli.replay.is_some();
        let spawn = if track { Vec3::new(0.0, 1.0, 0.0) } else { spawn };
        let props = if cli.test.as_deref() == Some("smash") { props::smash_test_props() } else { Vec::new() };
        Request::Drive { bundle: resolve(q)?, spawn, heading, max_units: cli.units, world: !track, props }
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
        None | Some("crash") | Some("smash") | Some("ai") | Some("race") => None,
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
        Some("exhaust") => Some(drive::TestKind::Exhaust),
        Some(other) => {
            anyhow::bail!(
                "unknown --test {other} (accel, brake, corner, handbrake, drift, donut, engine, exhaust, landing, climb, jitter, crash, smash, ai, race)"
            )
        }
    };
    if (cli.test.is_some() || cli.replay.is_some()) && drive.is_none() {
        anyhow::bail!("--test / --replay need --drive <car>");
    }
    let replay = cli.replay.as_deref().map(replay::Replay::load).transpose()?;
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
            _ => loader::BackdropMode::Composed,
        },
        no_groundcover: cli.no_groundcover,
        no_merge: !cli.merge_static,
        keep_lod_proxies: cli.keep_lod_proxies,
        paint_index: cli.paint_index,
        no_prop_physics: !(cli.prop_physics || smash),
        exclude: cli
            .exclude_models
            .iter()
            .map(|s| ResourceId::parse(s).ok_or_else(|| anyhow::anyhow!("bad resource id {s}")))
            .collect::<anyhow::Result<_>>()?,
    };
    let ai_test = cli.test.as_deref() == Some("ai");
    let ai_cars = cli.ai.iter().map(|q| resolve(q)).collect::<anyhow::Result<Vec<_>>>()?;
    let mut app = App::new();
    app.insert_resource(car_anim::AnimOverride::parse(&cli.anim)?);
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
    bevy::asset::embedded_asset!(app, "layered.wgsl");
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
    // The crash and smash tests take their own before / after screenshots.
    let screenshot = if crash {
        app.insert_resource(damage::CrashTest::new(
            cli.screenshot.clone(),
            cli.crash_angle,
            cli.crash_distance,
            cli.crash_wall_width,
        ));
        None
    } else if smash {
        app.insert_resource(props::SmashTest::new(cli.screenshot.clone(), cli.smash_speed / 3.6));
        app.add_plugins(props::SmashTestPlugin);
        app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(2.0));
        None
    } else {
        cli.screenshot
    };
    // `--camera` with `--drive`: a fixed camera instead of the chase camera.
    if let (Some(c), Request::Drive { world: true, .. }) = (camera, &request) {
        app.insert_resource(props::FixedCamera(c))
            .add_systems(PostUpdate, props::apply_fixed_camera.before(bevy::transform::TransformSystems::Propagate));
    }
    if cli.frame_stats {
        app.add_systems(Update, log_frame_stats);
    }
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
        .add_plugins(shadow_cull::ShadowCullPlugin)
        .add_plugins(reflection::PlanarReflectionPlugin { enabled: reflection })
        .add_plugins(FreeCameraPlugin)
        .add_plugins(MaterialPlugin::<GameMaterial>::default())
        .add_plugins(deferred::GameLightingPlugin)
        .add_plugins(tonemap::GameTonemapPlugin)
        .init_resource::<GameEnvironment>()
        .insert_resource(materials::VehicleLights::from_slice(&cli.lights))
        .add_plugins((
            avian3d::PhysicsPlugins::default().with_collision_hooks::<props::PropHooks>(),
            drive::DrivePlugin,
            damage::DamagePlugin,
            car_anim::CarAnimPlugin,
            props::PropsPlugin,
            particles::ParticlesPlugin,
            sfx::SfxPlugin,
        ))
        .add_plugins(camera::CameraPlugin { mode: camera_mode, look_back })
        .add_plugins(crash::CrashPlugin)
        .add_plugins(exhaust::ExhaustPlugin)
        .add_plugins(tyre_marks::TyreMarksPlugin)
        .add_plugins(freeroam::FreeroamPlugin)
        .add_plugins(race::RacePlugin { setup: race.map(Arc::new) })
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
        .add_plugins(game_text::GameTextPlugin)
        .add_plugins(hud::HudPlugin {
            enabled: !cli.no_hud && test.is_none() && !crash && !smash && !matches!(request, Request::Vehicle { .. }),
            metric: !cli.mph,
        })
        .add_plugins(settings::SettingsPlugin {
            settings: settings::load(cli.volume.as_deref()),
            open: cli.show_settings,
        })
        .add_plugins(audio::GameAudioPlugin {
            music: !cli.no_music
                && !cli.mute
                && screenshot.is_none()
                && test.is_none()
                && !race_test
                && !crash
                && !smash
                && cli.replay.is_none(),
        })
        .insert_resource(drive::CarInput { auto_throttle: cli.auto_throttle, auto_nitrous: cli.nitrous, ..default() })
        .insert_resource(mods_menu::ModSelection::parse(&cli.mods, &[]))
        .init_resource::<mods_menu::ModCatalogue>()
        .add_systems(Update, mods_menu::apply_live)
        .insert_resource(Game {
            store,
            request,
            camera,
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
            mute: cli.mute || test.is_some() || crash || smash || race_test,
        })
        .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.85)))
        .init_resource::<Loaded>()
        .add_systems(Startup, setup)
        .add_systems(Update, (start_loader, receive_assets, update_status, auto_screenshot))
        .add_systems(
            PostUpdate,
            groundcover_distance.before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        )
        .add_systems(Update, (drive_vehicle_lights, apply_vehicle_lights).chain())
        .run();
    Ok(())
}

use weather::GAME_UNIT;

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
            Projection::Perspective(PerspectiveProjection { far: 20_000.0, ..default() }),
        ))
        .id();
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
    let (channel, requests) = loader::spawn(game.store.clone(), game.request.clone(), bc, game.filter.clone());
    commands.insert_resource(channel);
    commands.insert_resource(requests);
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
    mut traffic_models: Option<ResMut<traffic::TrafficModels>>,
) {
    let Some(channel) = channel else { return };
    let rx = channel.0.lock().unwrap();
    let deadline = Instant::now() + Duration::from_millis(8);
    while Instant::now() < deadline {
        let Ok(msg) = rx.try_recv() else { break };
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
            Msg::Instance { model, transform } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                // Groundcover (model keys 0x6…, `loader::groundcover`) casts no shadows.
                let casts_shadow = model.0 >> 60 != 6;
                let range = loaded.shadow_ranges.get(&model).copied().map(shadow_cull::ShadowRange);
                commands.spawn((Transform::from_matrix(transform), Visibility::default())).with_children(|c| {
                    for (mesh, mat) in parts {
                        let class = if loaded.mask_materials.contains(&mat.id()) {
                            shadow_cull::ShadowClass::Alpha
                        } else if model.0 >> 62 != 0 {
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
                    }
                });
                loaded.instances += 1;
            }
            Msg::Groundcover { model, min, max, range } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                let range = GroundcoverRange { min, max, range };
                commands.spawn((Transform::IDENTITY, Visibility::Hidden, range)).with_children(|c| {
                    for (mesh, mat) in parts {
                        c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), bevy::light::NotShadowCaster));
                    }
                });
                loaded.instances += 1;
            }
            Msg::Prop { model, transform, physics } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                let range = loaded.shadow_ranges.get(&model).copied().map(shadow_cull::ShadowRange);
                props::spawn_prop(&mut commands, physics, Transform::from_matrix(transform), parts, range);
                loaded.instances += 1;
                loaded.props += 1;
            }
            Msg::Collider(c, tags) => {
                commands.spawn((avian3d::prelude::RigidBody::Static, c, props::SurfaceTags(tags)));
            }
            Msg::Surfaces(list) => surfaces.list = list,
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
                if let Some(t) = traffic_models.as_mut() {
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
}

fn update_status(
    loaded: Res<Loaded>,
    mut text: Query<&mut Text, With<StatusText>>,
    cam: Query<&Transform, MainCamera>,
    car: Query<&drive::Car, With<drive::Player>>,
) {
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
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
            }
            n if n == game.screenshot_delay.max(1) + 30 => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
        return;
    }
    *frames_after_done += 1;
    match *frames_after_done {
        n if n == game.screenshot_delay => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
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
            Collider::trimesh(vertices, indices),
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
