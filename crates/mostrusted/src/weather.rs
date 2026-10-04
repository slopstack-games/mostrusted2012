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
    let colour_cube = keyframe
        .colour_cube
        .and_then(|id| store.load(id).ok())
        .and_then(|res| mw_formats::ColourCube::parse(&res).ok())
        .map(|c| tonemap::colour_cube_image(&c));
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
    commands.insert_resource(tonemap::GameTonemap {
        settings: b.keyframe.exposure.clone(),
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
    ));
    // Lights the forward (translucent) materials; the game's lighting pass takes the key light from
    // `GameLighting`. Its cascaded shadow maps are rendered by Bevy and sampled by the game's lighting
    // pass (`deferred.rs`).
    commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: deferred::SHADOW_MAP_SIZE });
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
        app.add_systems(Update, (weather_keys, follow_timeline, request_bake, apply_bake).chain());
        if self.hud {
            app.add_systems(Startup, spawn_hud).add_systems(Update, update_hud);
        }
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
