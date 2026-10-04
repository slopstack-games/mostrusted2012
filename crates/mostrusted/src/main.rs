//! Most Rusted: an open-source runtime for Need for Speed: Most Wanted (2012).
//!
//! No game data is shipped. All assets are streamed at runtime from an
//! installed copy of the game (see `mw-gamedir` for how it is located).

mod audio;
mod convert;
mod damage;
mod deferred;
mod drift;
mod drive;
mod garage;
mod loader;
mod materials;
mod menu;
mod sky;
mod tonemap;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::light::{GeneratedEnvironmentMapLight, Skybox};
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
    /// Drive on an empty flat test track instead of the city (with --drive).
    #[arg(long)]
    test_track: bool,
    /// Run a scripted test and exit: accel, brake, corner, handbrake, drift, donut, engine, or crash (drive into a
    /// wall; with --screenshot, saves `<name>-before` / `<name>-after` images). Implies --test-track.
    #[arg(long)]
    test: Option<String>,
    /// Crash test: wall angle in degrees (0 = head-on, 30 = the front-left corner hits first).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    crash_angle: f32,
    /// Crash test: distance to the wall in metres (sets the impact speed).
    #[arg(long, default_value_t = 80.0)]
    crash_distance: f32,
    /// Hold the throttle at this value (0–1), for automated driving tests.
    #[arg(long)]
    auto_throttle: Option<f32>,
    /// Debug: skip InstanceList backdrop entries.
    #[arg(long)]
    no_backdrop: bool,
    /// Debug: skip groundcover (grass and plant sprites).
    #[arg(long)]
    no_groundcover: bool,
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
    /// Log raw gamepad input (to diagnose controller mappings).
    #[arg(long)]
    gamepad_debug: bool,
    /// Frames to wait after loading before `--screenshot` captures.
    #[arg(long, default_value_t = 30)]
    screenshot_delay: u32,
    /// Limit the number of streamed track units (nearest first).
    #[arg(long)]
    units: Option<usize>,
    /// Camera pose as `x,y,z,look_x,look_y,look_z` (overrides the default view).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    camera: Option<Vec<f32>>,
    /// Body paint colour as `r,g,b` in 0–255 sRGB, overriding the car's own paint colour.
    #[arg(long, value_delimiter = ',')]
    paint: Option<Vec<u8>>,
    /// Which of the car's paint options (`VehiclePaint` objects, in ID order) to use.
    #[arg(long, default_value_t = 0)]
    paint_index: usize,
    /// Save a screenshot once loading has finished, then exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Hour of the day (0–23) on the environment timeline.
    #[arg(long, default_value_t = 13)]
    hour: u32,
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
    /// Open the car menu at startup (also in `--screenshot` runs, to capture it).
    #[arg(long)]
    show_menu: bool,
}

#[derive(Resource)]
pub struct Game {
    hour: u32,
    pub store: Arc<AssetStore>,
    request: Request,
    camera: Option<Transform>,
    screenshot: Option<PathBuf>,
    screenshot_delay: u32,
    paint: Option<Color>,
    filter: loader::WorldFilter,
    mute: bool,
}

/// GPU meshes and materials making up one game Model.
pub type ModelParts = Vec<(Handle<Mesh>, Handle<GameMaterial>)>;

/// Bevy-side caches keyed by game resource IDs.
#[derive(Resource, Default)]
struct Loaded {
    textures: HashMap<TexKey, Handle<Image>>,
    models: HashMap<ResourceId, ModelParts>,
    materials: HashMap<Look, Handle<GameMaterial>>,
    instances: usize,
    status: String,
    done: bool,
    started: Option<Instant>,
}

#[derive(Component)]
struct StatusText;

fn main() -> anyhow::Result<()> {
    drive::install_gamepad_mappings();
    let cli = Cli::parse();
    let game = GameDir::locate(cli.game_dir.as_deref())?;
    println!("Most Wanted install: {} (version {})", game.root().display(), game.version().unwrap_or("?"));
    let t = Instant::now();
    let store = Arc::new(AssetStore::new(game)?);
    println!("indexed {} resources in {:?}", store.resource_count(), t.elapsed());

    if cli.camera.as_ref().is_some_and(|c| c.len() != 6) {
        anyhow::bail!("--camera expects 6 comma-separated values: x,y,z,look_x,look_y,look_z");
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
    let request = if let Some(q) = &cli.drive {
        let (spawn, heading) = match cli.spawn[..] {
            [x, y, z] => (Vec3::new(x, y, z), 0.0),
            [x, y, z, h] => (Vec3::new(x, y, z), h.to_radians()),
            _ => anyhow::bail!("--spawn expects x,y,z[,heading_degrees]"),
        };
        let track = cli.test_track || cli.test.is_some();
        let spawn = if track { Vec3::new(0.0, 1.0, 0.0) } else { spawn };
        Request::Drive { bundle: resolve(q)?, spawn, heading, max_units: cli.units, world: !track }
    } else if let Some(q) = &cli.vehicle {
        Request::Vehicle { bundle: resolve(q)? }
    } else {
        // Stream nearest-first from the `--camera` pose, so `--units` loads what it sees.
        Request::World { max_units: cli.units, focus: camera.map(|c| c.translation) }
    };

    let crash = cli.test.as_deref() == Some("crash");
    let test = match cli.test.as_deref() {
        None | Some("crash") => None,
        Some("accel") => Some(drive::TestKind::Accel),
        Some("brake") => Some(drive::TestKind::Brake),
        Some("corner") => Some(drive::TestKind::Corner),
        Some("handbrake") => Some(drive::TestKind::Handbrake),
        Some("drift") => Some(drive::TestKind::Drift),
        Some("donut") => Some(drive::TestKind::Donut),
        Some("engine") => Some(drive::TestKind::Engine),
        Some(other) => {
            anyhow::bail!("unknown --test {other} (accel, brake, corner, handbrake, drift, donut, engine, crash)")
        }
    };
    if cli.test.is_some() && cli.drive.is_none() {
        anyhow::bail!("--test needs --drive <car>");
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
    let menu_enabled = cli.screenshot.is_none() && test.is_none() && !matches!(request, Request::Vehicle { .. });
    let test_track = matches!(request, Request::Drive { world: false, .. });
    let filter = loader::WorldFilter {
        backdrop: match (cli.no_backdrop, cli.all_backdrops, cli.backdrop_zone) {
            (true, _, _) => loader::BackdropMode::None,
            (_, true, _) => loader::BackdropMode::All,
            (_, _, Some(unit)) => loader::BackdropMode::Zone(unit),
            _ => loader::BackdropMode::Composed,
        },
        no_groundcover: cli.no_groundcover,
        keep_lod_proxies: cli.keep_lod_proxies,
        paint_index: cli.paint_index,
        exclude: cli
            .exclude_models
            .iter()
            .map(|s| ResourceId::parse(s).ok_or_else(|| anyhow::anyhow!("bad resource id {s}")))
            .collect::<anyhow::Result<_>>()?,
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Most Rusted".into(), ..default() }),
            ..default()
        }),
    );
    bevy::asset::embedded_asset!(app, "layered.wgsl");
    if let Some(kind) = test {
        app.insert_resource(drive::DriveTest::new(kind));
        // Run the simulation faster than real time.
        app.add_systems(Startup, |mut t: ResMut<Time<Virtual>>| t.set_relative_speed(4.0));
    }
    if test_track {
        app.add_systems(Startup, spawn_test_track);
    }
    // The crash test takes its own before / after screenshots.
    let screenshot = if crash {
        app.insert_resource(damage::CrashTest::new(cli.screenshot.clone(), cli.crash_angle, cli.crash_distance));
        None
    } else {
        cli.screenshot
    };
    if cli.gamepad_debug {
        app.add_systems(Update, drive::log_gamepad_events);
    }
    app.add_plugins(FreeCameraPlugin)
        .add_plugins(MaterialPlugin::<GameMaterial>::default())
        .add_plugins(deferred::GameLightingPlugin)
        .add_plugins(tonemap::GameTonemapPlugin)
        .init_resource::<GameEnvironment>()
        .insert_resource(materials::VehicleLights::from_slice(&cli.lights))
        .add_plugins((avian3d::PhysicsPlugins::default(), drive::DrivePlugin, damage::DamagePlugin))
        .add_plugins((garage::GaragePlugin { script }, menu::MenuPlugin { enabled: menu_enabled, open: cli.show_menu }))
        .add_plugins(audio::GameAudioPlugin {
            music: !cli.no_music && !cli.mute && screenshot.is_none() && test.is_none() && !crash,
        })
        .insert_resource(drive::CarInput { auto_throttle: cli.auto_throttle, ..default() })
        .insert_resource(Game {
            store,
            request,
            camera,
            screenshot,
            screenshot_delay: cli.screenshot_delay,
            hour: cli.hour.min(23),
            paint,
            filter,
            mute: cli.mute || test.is_some() || crash,
        })
        .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.85)))
        .init_resource::<Loaded>()
        .add_systems(Startup, setup)
        .add_systems(Update, (start_loader, receive_assets, update_status, auto_screenshot))
        .add_systems(Update, (drive_vehicle_lights, apply_vehicle_lights).chain())
        .run();
    Ok(())
}

/// cd/m² (and lux, for the key light) per unit of the game's HDR values (environment keyframes, sky).
/// The camera's exposure divides it out again, so the HDR target holds game units for the game's tonemap
/// (`tonemap.rs`); the scale only matters to Bevy's forward lighting (translucent materials).
const GAME_UNIT: f32 = 1200.0;

fn setup(mut commands: Commands, game: Res<Game>, mut images: ResMut<Assets<Image>>) {
    // The key light's direction from the environment's sun / moon path at this hour.
    let to_sun = match game.store.sun_moon_path() {
        Ok(path) => Vec3::from(path.key_light_direction(game.hour as f32)),
        Err(e) => {
            warn!("no sun path ({e})");
            Vec3::new(0.45, 0.78, 0.43)
        }
    };
    info!("key light from {to_sun:.3}");
    let sun = Transform::IDENTITY.looking_to(-to_sun, Vec3::Y);
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
    // Time of day from the environment timeline: key light, ambient rig, sky and fog. The sky becomes
    // the skybox, the reflection cube the material passes read, and a Bevy environment light for the
    // forward (translucent) materials; the rig becomes the lighting pass's irradiance cube.
    let sun_dir = sun.back().as_vec3();
    let keyframe = game.store.environment_keyframe_at(mw_assets::environment::DEFAULT_TIMELINE, game.hour);
    let mut key_illuminance = 9_000.0;
    match keyframe {
        Ok(k) => {
            info!("environment: hour {} keyframe {} exposure {:?}", game.hour, k.id, k.exposure);
            let colour_cube = k.colour_cube.and_then(|id| match game.store.load(id) {
                Ok(res) => mw_formats::ColourCube::parse(&res).ok(),
                Err(_) => None,
            });
            commands.insert_resource(tonemap::GameTonemap {
                settings: k.exposure.clone(),
                dt: 0.0,
                reset: true,
                colour_cube: colour_cube.map(|c| images.add(tonemap::colour_cube_image(&c))),
            });
            let to_bevy = |c: [f32; 3]| Vec3::from(c) * GAME_UNIT;
            let key_colour = to_bevy(k.key_light);
            key_illuminance = key_colour.dot(Vec3::new(0.2126, 0.7152, 0.0722));
            let sky = sky::Sky::new(&game.store, &k, sun_dir);
            commands.insert_resource(GameEnvironment {
                reflection: Some(images.add(sky::reflection_cube(&sky))),
                reflection_mips: sky::REFLECTION_MIPS,
                sky_brightness: GAME_UNIT,
            });
            let f = &k.fog;
            let cs = |i: usize| f.colour_sun.get(i).copied().unwrap_or(0.0);
            commands.insert_resource(deferred::GameLighting {
                key_colour,
                key_direction: sun.forward().as_vec3(),
                sky_brightness: GAME_UNIT,
                irradiance: images.add(sky::irradiance_cube(&k, sun_dir)),
                fog: deferred::FogSettings {
                    colour: to_bevy(f.colour),
                    colour_sun: to_bevy([cs(0), cs(1), cs(2)]),
                    near: f.near,
                    far: f.far,
                    power: f.power,
                    scattering_colour: to_bevy(f.scattering_colour),
                    scattering_phase: f.scattering_phase,
                    scattering_strength: f.scattering_strength,
                },
            });
            let skybox = images.add(sky::sky_cube(&sky));
            commands.entity(camera).insert((
                Skybox { image: Some(skybox.clone()), brightness: GAME_UNIT, ..default() },
                GeneratedEnvironmentMapLight { environment_map: skybox, intensity: GAME_UNIT, ..default() },
            ));
        }
        Err(e) => {
            warn!("no environment keyframe ({e}); flat ambient");
            commands.insert_resource(GlobalAmbientLight { brightness: 600.0, ..default() });
        }
    }
    // Screenshot runs must be deterministic: no input (gamepad drift moves the camera).
    if game.screenshot.is_none() && !matches!(game.request, Request::Drive { .. }) {
        commands.entity(camera).insert(FreeCamera { walk_speed: 30.0, run_speed: 150.0, ..default() });
    }
    // Lights the forward (translucent) materials; the game's lighting pass takes the key light from
    // `GameLighting`.
    // Its cascaded shadow maps are rendered by Bevy and sampled by the game's lighting pass (`deferred.rs`).
    commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: deferred::SHADOW_MAP_SIZE });
    commands.spawn((
        DirectionalLight { illuminance: key_illuminance, shadow_maps_enabled: true, ..default() },
        deferred::shadow_cascades(),
        sun,
    ));
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
    mut camera: Query<&mut Transform, With<Camera3d>>,
    game: Res<Game>,
    env: Res<GameEnvironment>,
    lights: Res<materials::VehicleLights>,
    mut garage: ResMut<garage::Garage>,
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
                    handles.push((meshes.add(part.mesh), mat));
                }
                loaded.models.insert(id, handles);
            }
            Msg::Instance { model, transform } => {
                let Some(parts) = loaded.models.get(&model) else { continue };
                // Groundcover (model keys 0x6…, `loader::groundcover`) casts no shadows.
                let casts_shadow = model.0 >> 60 != 6;
                commands.spawn((Transform::from_matrix(transform), Visibility::default())).with_children(|c| {
                    for (mesh, mat) in parts {
                        let mut e = c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone())));
                        if !casts_shadow {
                            e.insert(bevy::light::NotShadowCaster);
                        }
                    }
                });
                loaded.instances += 1;
            }
            Msg::Collider(c) => {
                commands.spawn((avian3d::prelude::RigidBody::Static, c));
            }
            Msg::Car(spec) => {
                let parts = spec
                    .body
                    .iter()
                    .chain(spec.wheels.iter().flat_map(|w| w.parts.iter().map(|(m, _)| m)))
                    .filter_map(|m| Some((*m, loaded.models.get(m)?.clone())))
                    .collect();
                // Placed by the garage: at `--spawn` at startup, later in place of the current car.
                garage.arrived(spec, parts);
            }
            Msg::CarFailed(e) => garage.failed(e),
            Msg::Progress(s) => {
                info!("{s}");
                loaded.status = s;
            }
            Msg::Done => {
                loaded.done = true;
                if let Some(t) = loaded.started {
                    info!("loading finished in {:.1?}", t.elapsed());
                }
            }
        }
    }
}

fn update_status(
    loaded: Res<Loaded>,
    mut text: Query<&mut Text, With<StatusText>>,
    cam: Query<&Transform, With<Camera3d>>,
    car: Query<&drive::Car>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    let pos = cam.single().map(|t| t.translation).unwrap_or_default();
    let controls = match car.single() {
        Ok(car) => format!(
            "{:.0} km/h\nW/S throttle/brake, A/D steer, space handbrake, R reset (or gamepad triggers + stick)",
            car.speed.abs() * 3.6
        ),
        Err(_) => "WASD/QE move, right mouse look, shift run, scroll speed".to_string(),
    };
    let controls = format!("{controls}\nTab / F1 / gamepad Start: car menu");
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
    car: Query<&drive::Car>,
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
fn auto_screenshot(
    mut commands: Commands,
    game: Res<Game>,
    loaded: Res<Loaded>,
    script: Option<Res<garage::SwapScript>>,
    mut frames_after_done: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &game.screenshot else { return };
    // With `--swap-to`, capture after the last swap.
    if !loaded.done || script.is_some_and(|s| !s.finished()) {
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
) {
    use avian3d::prelude::{Collider, RigidBody};
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
