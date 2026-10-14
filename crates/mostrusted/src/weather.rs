//! Time of day and weather at runtime.
//!
//! What the game's data has for weather (research log 2026-10-04 "weather"):
//! - **Which timeline**: a `WorldPaintMap` paints the 25 `EnvironmentTimeline`s over the map, so the
//!   lighting follows the camera from district to district ([`EnvironmentState::follow`]).
//! - **Time of day**: hourly keyframes, blended linearly between a timeline's entries
//!   (`AssetStore::environment_at`).
//! - **Bad weather**: each keyframe's `Alternate`, one of seven hazier twins (nearer fog, dimmer key light,
//!   other colour grade). Nothing in the data says when the game uses them; `--weather` / F7 switch.
//! - **Wind** (`Weather` block: bearing 130° ± 25°, 1–3.5 m/s on every keyframe) and a night-time
//!   `Thunderstorm_Intensity` (0.23). Neither has a known consumer: tree sway in the tree shader is
//!   per-material (`amplitude`, `speed`) and time-driven, the sky's cloud offsets are CPU-side.
//! - **Wet roads**: the road shader's wet path (puddle map, RoadDepth, vertex wetness mask) has no global
//!   wetness input, so roads are always as wet as the puddle map says (`layered.wgsl` mode 1/4;
//!   `--road-wetness` scales it for comparison).
//! - **No rain**: no precipitation field, texture, particle effect, sound or shader exists in the PC
//!   data. A rain effect would therefore be an invention and is not drawn.
//!
//! Every change re-bakes the environment (sky, reflection and irradiance cubes, colour cube) on a
//! background thread and swaps the images in place, so materials keep their handles.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::light::{GeneratedEnvironmentMapLight, Skybox};
use bevy::prelude::*;
use mw_assets::AssetStore;
use mw_assets::environment::{DEFAULT_TIMELINE, EnvironmentKeyframe, TIMELINE_PAINT_MAP};
use mw_bundle::ResourceId;

use crate::materials::GameEnvironment;
use crate::{deferred, sky, tonemap};

/// cd/m² (and lux, for the key light) per unit of the game's HDR values (environment keyframes, sky).
/// The camera's exposure divides it out again, so the HDR target holds game units for the game's tonemap
/// (`tonemap.rs`); the scale only matters to Bevy's forward lighting (translucent materials).
pub const GAME_UNIT: f32 = 1200.0;

/// What the environment is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvironmentKey {
    pub timeline: ResourceId,
    pub hour: f32,
    pub alternate: bool,
}

/// The wanted environment, and what is on screen.
#[derive(Resource)]
pub struct EnvironmentState {
    pub want: EnvironmentKey,
    shown: Option<EnvironmentKey>,
    /// Pick the timeline painted under the camera (off with `--timeline`, the car viewer and the test
    /// track).
    pub follow: bool,
    /// Bake on the main thread (screenshot and test runs must not depend on thread timing).
    pub sync: bool,
    paint_map: Option<mw_formats::WorldPaintMap>,
    pending: Option<(EnvironmentKey, Mutex<Receiver<Option<Baked>>>)>,
    store: Arc<AssetStore>,
}

impl EnvironmentState {
    pub fn new(store: Arc<AssetStore>, want: EnvironmentKey, follow: bool, sync: bool) -> Self {
        let paint_map = store
            .load(TIMELINE_PAINT_MAP)
            .ok()
            .and_then(|r| mw_formats::WorldPaintMap::parse(&r).map_err(|e| warn!("timeline paint map: {e}")).ok());
        Self { want, shown: None, follow, sync, paint_map, pending: None, store }
    }

    /// The timeline painted at a world position.
    pub fn timeline_at(&self, p: Vec3) -> ResourceId {
        self.paint_map
            .as_ref()
            .and_then(|m| m.entry_at(p.x, p.z))
            .map_or(DEFAULT_TIMELINE, |e| ResourceId(0x0100_0000_0000_0000 | e as u64))
    }
}

/// Handles of the environment images, kept for the session: re-bakes replace their contents.
#[derive(Resource, Clone)]
pub struct EnvironmentImages {
    sky: Handle<Image>,
    reflection: Handle<Image>,
    irradiance: Handle<Image>,
    colour_cube: Handle<Image>,
}

/// The key light entity (lights the forward materials and renders the shadow cascades).
#[derive(Component)]
pub struct KeyLight;

#[derive(Component)]
struct WeatherText;

/// A baked environment, ready to show.
pub struct Baked {
    key: EnvironmentKey,
    keyframe: EnvironmentKeyframe,
    /// Towards the key light.
    to_sun: Vec3,
    sky: Image,
    reflection: Image,
    irradiance: Image,
    colour_cube: Option<Image>,
}

/// Build everything the renderer takes from the environment (blocking: ~0.1–0.3 s for the cubes).
pub fn bake(store: &AssetStore, key: EnvironmentKey) -> Option<Baked> {
    let started = std::time::Instant::now();
    let baked = bake_inner(store, key);
    info!("environment bake {:x} at {:.2} h took {:.0?}", key.timeline.0 & 0xFF_FFFF, key.hour, started.elapsed());
    baked
}

fn bake_inner(store: &AssetStore, key: EnvironmentKey) -> Option<Baked> {
    let keyframe = match store.environment_at(key.timeline, key.hour, key.alternate) {
        Ok(k) => k,
        Err(e) => {
            warn!("no environment for {key:?}: {e}");
            return None;
        }
    };
    let to_sun = match store.sun_moon_path() {
        Ok(path) => Vec3::from(path.key_light_direction(key.hour.rem_euclid(24.0))),
        Err(e) => {
            warn!("no sun path ({e})");
            Vec3::new(0.45, 0.78, 0.43)
        }
    }
    .normalize();
    let s = sky::Sky::new(store, &keyframe, to_sun);
    // TintBlend (`FUN_008e0930`): the two keyframes' cubes with the timeline weights; a keyframe without one
    // falls back to the base slot (the identity cube in free roam).
    let load_cube = |id: Option<ResourceId>| {
        id.and_then(|id| store.load(id).ok()).and_then(|res| mw_formats::ColourCube::parse(&res).ok())
    };
    // `MW_NO_CUBE_BLEND=1`: the earlier keyframe's cube alone (A/B).
    let colour_cube = match keyframe.colour_cube_blend.filter(|_| std::env::var_os("MW_NO_CUBE_BLEND").is_none()) {
        Some((next, t)) if keyframe.colour_cube.is_some() || next.is_some() => {
            tonemap::blend_colour_cubes(&[(load_cube(keyframe.colour_cube), 1.0 - t), (load_cube(next), t)])
        }
        _ => load_cube(keyframe.colour_cube).map(|c| tonemap::colour_cube_image(&c)),
    };
    Some(Baked {
        key,
        to_sun,
        sky: sky::sky_cube(&s),
        reflection: sky::reflection_cube(&s),
        irradiance: sky::irradiance_cube(&keyframe, to_sun),
        colour_cube,
        keyframe,
    })
}

fn to_bevy(c: [f32; 3]) -> Vec3 {
    Vec3::from(c) * GAME_UNIT
}

fn game_lighting(b: &Baked, irradiance: Handle<Image>) -> deferred::GameLighting {
    let k = &b.keyframe;
    let f = &k.fog;
    let cs = |i: usize| f.colour_sun.get(i).copied().unwrap_or(0.0);
    deferred::GameLighting {
        key_colour: to_bevy(k.key_light),
        key_direction: -b.to_sun,
        sky_brightness: GAME_UNIT,
        irradiance,
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
    }
}

fn key_illuminance(b: &Baked) -> f32 {
    to_bevy(b.keyframe.key_light).dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn describe(b: &Baked) -> String {
    format!(
        "environment: {:02}:{:02} timeline {:x} keyframe {:x}{} (fog {:.0}–{:.0} m, key {:.2?})",
        b.key.hour as u32,
        (b.key.hour.fract() * 60.0) as u32,
        b.key.timeline.0 & 0xFF_FFFF,
        b.keyframe.id.0 & 0xFF_FFFF,
        if b.key.alternate { " Alternate" } else { "" },
        b.keyframe.fog.near,
        b.keyframe.fog.far,
        b.keyframe.key_light,
    )
}

/// Bake the starting environment and spawn what it drives: the key light, the camera's skybox, and the
/// lighting / tonemap / material resources. Blocking, so the first frame is already lit.
pub fn spawn_environment(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    mut state: EnvironmentState,
    camera: Entity,
    road_wetness: f32,
) {
    let Some(mut b) = bake(&state.store, state.want) else {
        let handles = EnvironmentImages {
            sky: images.add(Image::default()),
            reflection: images.add(Image::default()),
            irradiance: images.add(Image::default()),
            colour_cube: images.add(Image::default()),
        };
        warn!("no environment keyframe; flat ambient");
        commands.insert_resource(GlobalAmbientLight { brightness: 600.0, ..default() });
        commands.spawn((
            DirectionalLight { illuminance: 9_000.0, shadow_maps_enabled: true, ..default() },
            deferred::shadow_cascades(),
            Transform::IDENTITY.looking_to(Vec3::new(-0.45, -0.78, -0.43), Vec3::Y),
            KeyLight,
        ));
        commands.insert_resource(state);
        commands.insert_resource(handles);
        return;
    };
    info!("{}", describe(&b));
    // The first images become the handles that later bakes overwrite.
    let has_cube = b.colour_cube.is_some();
    let handles = EnvironmentImages {
        sky: images.add(std::mem::take(&mut b.sky)),
        reflection: images.add(std::mem::take(&mut b.reflection)),
        irradiance: images.add(std::mem::take(&mut b.irradiance)),
        colour_cube: images.add(b.colour_cube.take().unwrap_or_default()),
    };
    let post_fx =
        mw_assets::environment::PostFxKeyframe::load(&state.store, mw_assets::environment::DEFAULT_POSTFX_KEYFRAME)
            .unwrap_or_else(|e| {
                warn!("no PostFX keyframe ({e}); no vignette");
                Default::default()
            });
    commands.insert_resource(tonemap::GameTonemap {
        settings: b.keyframe.exposure.clone(),
        post_fx,
        dt: 0.0,
        reset: true,
        colour_cube: has_cube.then(|| handles.colour_cube.clone()),
    });
    commands.insert_resource(GameEnvironment {
        reflection: Some(handles.reflection.clone()),
        reflection_mips: sky::REFLECTION_MIPS,
        sky_brightness: GAME_UNIT,
        road_wetness,
        planar: None,
    });
    commands.insert_resource(game_lighting(&b, handles.irradiance.clone()));
    commands.entity(camera).insert((
        Skybox { image: Some(handles.sky.clone()), brightness: GAME_UNIT, ..default() },
        GeneratedEnvironmentMapLight { environment_map: handles.sky.clone(), intensity: GAME_UNIT, ..default() },
        SkyFilter { sky: handles.sky.clone() },
    ));
    // Lights the forward (translucent) materials; the game's lighting pass takes the key light from
    // `GameLighting`. Its cascaded shadow maps are rendered by Bevy and sampled by the game's lighting
    // pass (`deferred.rs`).
    commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: deferred::shadow_map_size() });
    commands.spawn((
        DirectionalLight { illuminance: key_illuminance(&b), shadow_maps_enabled: true, ..default() },
        deferred::shadow_cascades(),
        Transform::IDENTITY.looking_to(-b.to_sun, Vec3::Y),
        KeyLight,
    ));
    state.shown = Some(b.key);
    commands.insert_resource(state);
    commands.insert_resource(handles);
}

fn store_images(images: &mut Assets<Image>, h: &EnvironmentImages, b: Baked) {
    let _ = images.insert(&h.sky, b.sky);
    let _ = images.insert(&h.reflection, b.reflection);
    let _ = images.insert(&h.irradiance, b.irradiance);
    if let Some(c) = b.colour_cube {
        let _ = images.insert(&h.colour_cube, c);
    }
}

pub struct WeatherPlugin {
    /// Show the key help / state line (interactive runs).
    pub hud: bool,
}

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (weather_keys, follow_timeline, request_bake, apply_bake, settle_sky_filter).chain());
        let runs = SkyFilterRuns::default();
        app.insert_resource(runs.clone());
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.insert_resource(runs).add_systems(
                bevy::render::Render,
                count_sky_filter_runs
                    .after(bevy::pbr::generate::filtering_system)
                    .before(bevy::render::RenderSystems::Render),
            );
        }
        if self.hud {
            app.add_systems(Startup, spawn_hud).add_systems(Update, update_hud);
        }
    }
}

/// The forward materials' image-based light (Bevy's `EnvironmentMapLight`) is filtered from the sky cube by
/// Bevy's `GeneratedEnvironmentMapLight`, which runs its whole filter every frame (copy, downsample, GGX
/// radiance for each of the 512² cube's 10 mips, irradiance) although the sky only changes with a bake. The
/// camera keeps [`SkyFilter`]; the generator component stays until the filter has run
/// [`SKY_FILTER_RUNS`] times on the current sky and is then removed: the `EnvironmentMapLight` it created
/// keeps the filtered cubes. A changed sky image (a bake) puts it back. `MW_SKY_FILTER_EVERY_FRAME=1` keeps
/// Bevy's per-frame filtering (A/B).
#[derive(Component)]
struct SkyFilter {
    sky: Handle<Image>,
}

/// Filter passes run since the sky last changed (written by the render world, reset by the main world).
#[derive(Resource, Clone, Default)]
struct SkyFilterRuns(Arc<std::sync::atomic::AtomicU32>);

/// Render frames that ran the filter before it is removed. The new sky's GPU image is used from the second
/// render frame after the change (extraction reads the previous frame's render assets) and the render frame
/// in flight may still count once for the old one, so four leave margin.
const SKY_FILTER_RUNS: u32 = 4;

fn settle_sky_filter(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<Image>>,
    runs: Res<SkyFilterRuns>,
    cameras: Query<(Entity, &SkyFilter, Option<&GeneratedEnvironmentMapLight>)>,
) {
    use std::sync::atomic::Ordering;
    let changed: Vec<AssetId<Image>> = events
        .read()
        .filter_map(|e| match e {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    for (entity, filter, generated) in &cameras {
        if changed.contains(&filter.sky.id()) {
            runs.0.store(0, Ordering::Relaxed);
            if generated.is_none() {
                commands.entity(entity).insert(GeneratedEnvironmentMapLight {
                    environment_map: filter.sky.clone(),
                    intensity: GAME_UNIT,
                    ..default()
                });
            }
        } else if generated.is_some()
            && runs.0.load(Ordering::Relaxed) >= SKY_FILTER_RUNS
            && std::env::var_os("MW_SKY_FILTER_EVERY_FRAME").is_none()
        {
            debug!("sky filter settled");
            commands.entity(entity).remove::<GeneratedEnvironmentMapLight>();
        }
    }
}

/// Counts render frames in which Bevy's environment-map filter dispatched (its pipelines are ready and a
/// camera has its bind groups), right after its `filtering_system`.
fn count_sky_filter_runs(
    runs: Res<SkyFilterRuns>,
    probes: Query<
        (),
        (With<bevy::pbr::generate::GeneratorBindGroups>, With<bevy::pbr::generate::RenderEnvironmentMap>),
    >,
    pipelines: Option<Res<bevy::pbr::generate::GeneratorPipelines>>,
    cache: Res<bevy::render::render_resource::PipelineCache>,
) {
    let Some(p) = pipelines else { return };
    let ready = [p.downsample_first, p.downsample_second, p.copy, p.radiance, p.irradiance]
        .into_iter()
        .all(|id| cache.get_compute_pipeline(id).is_some());
    if ready && !probes.is_empty() {
        runs.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// F5 / F6: an hour back / forward; F7: toggle the Alternate (bad-weather) keyframes.
fn weather_keys(keys: Res<ButtonInput<KeyCode>>, state: Option<ResMut<EnvironmentState>>) {
    let Some(mut state) = state else { return };
    if keys.just_pressed(KeyCode::F5) {
        state.want.hour = (state.want.hour.floor() - 1.0).rem_euclid(24.0);
    }
    if keys.just_pressed(KeyCode::F6) {
        state.want.hour = (state.want.hour.floor() + 1.0).rem_euclid(24.0);
    }
    if keys.just_pressed(KeyCode::F7) {
        state.want.alternate = !state.want.alternate;
    }
}

/// The timeline painted under the camera, checked twice a second once the world has loaded (until then
/// the camera may not be in place yet; the starting timeline comes from the requested view).
fn follow_timeline(
    time: Res<Time>,
    state: Option<ResMut<EnvironmentState>>,
    loaded: Res<crate::Loaded>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut since: Local<f32>,
) {
    let Some(mut state) = state else { return };
    *since += time.delta_secs();
    if !state.follow || !loaded.done || *since < 0.5 {
        return;
    }
    *since = 0.0;
    let Some(cam) = camera.iter().next() else { return };
    let timeline = state.timeline_at(cam.translation());
    if timeline != state.want.timeline {
        state.want.timeline = timeline;
    }
}

/// Start a bake when the wanted environment differs from the shown one.
fn request_bake(state: Option<ResMut<EnvironmentState>>) {
    let Some(mut state) = state else { return };
    if state.shown == Some(state.want) || state.pending.as_ref().is_some_and(|(k, _)| *k == state.want) {
        return;
    }
    if state.pending.is_some() {
        // A bake for an older wish is running; its result is dropped when it arrives.
        return;
    }
    let (tx, rx) = channel();
    let (store, key) = (state.store.clone(), state.want);
    if state.sync {
        let _ = tx.send(bake(&store, key));
    } else {
        std::thread::spawn(move || {
            let _ = tx.send(bake(&store, key));
        });
    }
    state.pending = Some((key, Mutex::new(rx)));
}

/// Show a finished bake: swap the images, update the lighting, tonemap and key light.
#[allow(clippy::too_many_arguments)]
fn apply_bake(
    state: Option<ResMut<EnvironmentState>>,
    handles: Option<Res<EnvironmentImages>>,
    mut images: ResMut<Assets<Image>>,
    lighting: Option<ResMut<deferred::GameLighting>>,
    tonemap: Option<ResMut<tonemap::GameTonemap>>,
    mut key_light: Query<(&mut DirectionalLight, &mut Transform), With<KeyLight>>,
) {
    let (Some(mut state), Some(handles)) = (state, handles) else { return };
    let Some((key, rx)) = state.pending.as_ref() else { return };
    let key = *key;
    let result = match rx.lock().map(|r| r.try_recv()) {
        Ok(Ok(r)) => r,
        Ok(Err(std::sync::mpsc::TryRecvError::Empty)) => return,
        _ => None,
    };
    state.pending = None;
    let Some(b) = result else {
        // Don't retry a failing environment every frame.
        state.shown = Some(key);
        return;
    };
    if key != state.want {
        return;
    }
    info!("{}", describe(&b));
    if let Some(mut l) = lighting {
        *l = game_lighting(&b, handles.irradiance.clone());
    }
    if let Some(mut t) = tonemap {
        t.settings = b.keyframe.exposure.clone();
        t.colour_cube = b.colour_cube.is_some().then(|| handles.colour_cube.clone());
    }
    for (mut light, mut transform) in &mut key_light {
        light.illuminance = key_illuminance(&b);
        *transform = Transform::IDENTITY.looking_to(-b.to_sun, Vec3::Y);
    }
    state.shown = Some(key);
    store_images(&mut images, &handles, b);
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        Node { position_type: PositionType::Absolute, bottom: Val::Px(8.0), left: Val::Px(8.0), ..default() },
        WeatherText,
    ));
}

fn update_hud(state: Option<Res<EnvironmentState>>, mut text: Query<&mut Text, With<WeatherText>>) {
    let (Some(state), Ok(mut text)) = (state, text.single_mut()) else { return };
    if !state.is_changed() {
        return;
    }
    let w = state.want;
    text.0 = format!(
        "{:02}:{:02} timeline {:x}{}{} | F5/F6 hour, F7 weather (Alternate keyframes)",
        w.hour as u32,
        (w.hour.fract() * 60.0) as u32,
        w.timeline.0 & 0xFF_FFFF,
        if w.alternate { ", Alternate" } else { "" },
        if state.pending.is_some() { " (baking...)" } else { "" },
    );
}
