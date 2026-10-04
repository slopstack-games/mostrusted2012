//! Most Rusted: an open-source runtime for Need for Speed: Most Wanted (2012).
//!
//! No game data is shipped. All assets are streamed at runtime from an
//! installed copy of the game (see `mw-gamedir` for how it is located).

mod audio;
mod convert;
mod drive;
mod loader;
mod materials;

use std::collections::HashMap;
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
use materials::{GameMaterial, Look, TexKey};

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
    /// Run a scripted handling test and exit: accel, brake, corner, handbrake, drift or engine (implies --test-track).
    #[arg(long)]
    test: Option<String>,
    /// Hold the throttle at this value (0–1), for automated driving tests.
    #[arg(long)]
    auto_throttle: Option<f32>,
    /// Debug: skip InstanceList backdrop entries.
    #[arg(long)]
    no_backdrop: bool,
    /// Debug: skip groundcover (grass and plant sprites).
    #[arg(long)]
    no_groundcover: bool,
    /// Debug: also draw backdrop LOD2 stand-ins (they overlap the detailed buildings).
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
    /// Body paint colour as `r,g,b` in 0–255 sRGB.
    #[arg(long, value_delimiter = ',', default_value = "170,20,24")]
    paint: Vec<u8>,
    /// Save a screenshot once loading has finished, then exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
}

#[derive(Resource)]
pub struct Game {
    pub store: Arc<AssetStore>,
    request: Request,
    camera: Option<Transform>,
    screenshot: Option<PathBuf>,
    screenshot_delay: u32,
    paint: Color,
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
    let [r, g, b] = cli.paint[..] else { anyhow::bail!("--paint expects r,g,b") };
    let paint = Color::srgb_u8(r, g, b);
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
        Request::World { max_units: cli.units, focus: None }
    };

    let test = match cli.test.as_deref() {
        None => None,
        Some("accel") => Some(drive::TestKind::Accel),
        Some("brake") => Some(drive::TestKind::Brake),
        Some("corner") => Some(drive::TestKind::Corner),
        Some("handbrake") => Some(drive::TestKind::Handbrake),
        Some("drift") => Some(drive::TestKind::Drift),
        Some("engine") => Some(drive::TestKind::Engine),
        Some(other) => anyhow::bail!("unknown --test {other} (accel, brake, corner, handbrake, drift, engine)"),
    };
    if test.is_some() && cli.drive.is_none() {
        anyhow::bail!("--test needs --drive <car>");
    }
    let test_track = matches!(request, Request::Drive { world: false, .. });
    let filter = loader::WorldFilter {
        no_backdrop: cli.no_backdrop,
        no_groundcover: cli.no_groundcover,
        keep_lod_proxies: cli.keep_lod_proxies,
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
    if cli.gamepad_debug {
        app.add_systems(Update, drive::log_gamepad_events);
    }
    app.add_plugins(FreeCameraPlugin)
        .add_plugins(MaterialPlugin::<GameMaterial>::default())
        .add_plugins((avian3d::PhysicsPlugins::default(), drive::DrivePlugin))
        .add_plugins(audio::GameAudioPlugin {
            music: !cli.no_music && !cli.mute && cli.screenshot.is_none() && test.is_none(),
        })
        .insert_resource(drive::CarInput { auto_throttle: cli.auto_throttle, ..default() })
        .insert_resource(Game {
            store,
            request,
            camera,
            screenshot: cli.screenshot,
            screenshot_delay: cli.screenshot_delay,
            paint,
            filter,
            mute: cli.mute || test.is_some(),
        })
        .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.85)))
        .init_resource::<Loaded>()
        .add_systems(Startup, setup)
        .add_systems(Update, (start_loader, receive_assets, update_status, auto_screenshot))
        .run();
    Ok(())
}

fn setup(mut commands: Commands, game: Res<Game>) {
    let camera = commands
        .spawn((
            Camera3d::default(),
            Transform::from_xyz(6.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.6, 0.0), Vec3::Y),
            Projection::Perspective(PerspectiveProjection { far: 20_000.0, ..default() }),
            DistanceFog {
                color: Color::srgb(0.55, 0.68, 0.85),
                falloff: FogFalloff::Linear { start: 1500.0, end: 6000.0 },
                ..default()
            },
        ))
        .id();
    // Screenshot runs must be deterministic: no input (gamepad drift moves the camera).
    if game.screenshot.is_none() && !matches!(game.request, Request::Drive { .. }) {
        commands.entity(camera).insert(FreeCamera { walk_speed: 30.0, run_speed: 150.0, ..default() });
    }
    commands.spawn((
        DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.8, -0.9, 0.0)),
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 600.0, ..default() });
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
    commands.insert_resource(loader::spawn(game.store.clone(), game.request.clone(), bc, game.filter.clone()));
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
                            let m = materials.add(part.look.material(tex, game.paint));
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
                commands.spawn((Transform::from_matrix(transform), Visibility::default())).with_children(|c| {
                    for (mesh, mat) in parts {
                        c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone())));
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
                let Request::Drive { spawn, heading, .. } = game.request else { continue };
                if !game.mute
                    && let Some((on_load, off_load)) = spec.engine.clone()
                {
                    commands.insert_resource(audio::PendingEngine(Some(audio::EngineAudio {
                        on_load,
                        off_load,
                        params: Default::default(),
                    })));
                }
                commands.insert_resource(drive::PendingCar { spec: *spec, spawn, heading, parts });
            }
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

/// `--screenshot`: wait for loading plus a few frames for GPU uploads, capture,
/// then exit.
fn auto_screenshot(
    mut commands: Commands,
    game: Res<Game>,
    loaded: Res<Loaded>,
    mut frames_after_done: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &game.screenshot else { return };
    if !loaded.done {
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
