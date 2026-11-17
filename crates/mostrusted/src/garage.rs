//! Spawning, swapping and removing the player car while the session runs.
//!
//! The car menu (`menu.rs`) and the `--swap-to` test hook send [`GarageAction`]s. A car is loaded by
//! the loader thread ([`CarRequest`]); when it arrives it replaces the car being driven in place
//! (same position, heading and velocity), or, with no car, is dropped onto the ground below the free
//! camera. The old car is despawned with everything attached to it (body, wheels, colliders, engine
//! sound); its meshes and materials stay in the model cache, so swapping back reuses them.

use crate::platform::PollLock;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use avian3d::prelude::{LinearVelocity, SpatialQuery, SpatialQueryFilter};
use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraState};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use mw_assets::AssetStore;

use crate::drive::{Car, CarParts, PendingCar};
use crate::loader::{CarRequest, CarSpec, LoaderRequests, Request};
use crate::{Game, Loaded, audio};

pub struct GaragePlugin {
    /// Steps of `--swap-to`, run once loading has finished.
    pub script: Option<SwapScript>,
}

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GarageAction>()
            .init_resource::<Garage>()
            .add_systems(Startup, start_catalogue)
            .add_systems(Update, (poll_jobs, run_script, handle_actions, place_arrived_car).chain());
        if let Some(script) = &self.script {
            app.insert_resource(script.clone());
        }
    }
}

/// What the menu (or the test script) asks for.
#[derive(Message, Debug, Clone)]
pub enum GarageAction {
    /// Load a car and drive it: in place of the current car, or dropped below the free camera.
    Drive { name: String, bundle: String, paint_index: usize },
    /// Leave the car and fly the free camera.
    FreeCamera,
}

/// A drivable car of the catalogue (`VehicleList` joined with the language table).
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    pub id: u32,
    /// Display name split into manufacturer and model, ASCII only (the UI font has no accents).
    pub make: String,
    pub model: String,
    pub name: String,
    pub year: u16,
    pub bundle: String,
}

/// The car being driven.
#[derive(Debug, Clone)]
pub struct CurrentCar {
    pub bundle: String,
    pub paint_index: usize,
    pub paint_count: usize,
}

type Job<T> = Arc<Mutex<Option<T>>>;

/// A loaded car waiting to be placed; `name: None` for the startup car.
struct Arrived {
    spec: Box<CarSpec>,
    parts: CarParts,
    name: Option<String>,
}

#[derive(Resource)]
pub struct Garage {
    /// Drivable cars sorted by name; empty until the catalogue thread is done.
    pub catalogue: Vec<CatalogueEntry>,
    catalogue_job: Option<Job<Vec<CatalogueEntry>>>,
    /// Name of the car the loader is working on for us.
    pub loading: Option<String>,
    pub current: Option<CurrentCar>,
    /// Last thing that happened (shown by the menu) and when (`Time::elapsed_secs`).
    pub status: String,
    pub status_time: f32,
    /// With `--drive`, the first car from the loader is the startup car (placed at `--spawn`).
    startup_car: bool,
    /// Car requests sent, answered (loaded or failed), and cancelled (requests up to this count are
    /// dropped when they arrive). Only the latest request is placed; earlier ones are superseded.
    requested: u32,
    answered: u32,
    cancelled: u32,
    /// A car the loader finished, waiting to be placed.
    arrived: Option<Arrived>,
    /// Paint swatches (linear diffuse colours) per bundle, read on a helper thread.
    paints: HashMap<String, Vec<[f32; 3]>>,
    paint_job: Option<(String, Job<Vec<[f32; 3]>>)>,
    /// Where the next requested car is put down (position at road level, yaw), at rest, instead of in place
    /// of the current car: a jack-spot swap puts the player's car on the parked car's pose (`jackspots.rs`).
    pub place_at: Option<(Vec3, f32)>,
}

impl Default for Garage {
    fn default() -> Self {
        Self {
            catalogue: Vec::new(),
            catalogue_job: None,
            loading: None,
            current: None,
            status: String::new(),
            status_time: f32::NEG_INFINITY,
            startup_car: false,
            requested: 0,
            answered: 0,
            cancelled: 0,
            arrived: None,
            paints: HashMap::new(),
            paint_job: None,
            place_at: None,
        }
    }
}

impl Garage {
    /// Called by the loader-message pump when a car is ready.
    pub fn arrived(&mut self, spec: Box<CarSpec>, parts: CarParts) {
        // The loader serves the startup request before any runtime request.
        if std::mem::take(&mut self.startup_car) {
            self.arrived = Some(Arrived { spec, parts, name: None });
            return;
        }
        let name = Some(self.loading.clone().unwrap_or_default());
        let n = self.answer();
        if n > self.cancelled && n == self.requested {
            self.arrived = Some(Arrived { spec, parts, name });
        }
    }

    /// Called by the loader-message pump when a requested car could not be loaded.
    pub fn failed(&mut self, message: String) {
        warn!("car load failed: {message}");
        self.answer();
        self.status = format!("could not load car: {message}");
        self.status_time = f32::NAN; // stamped by `poll_jobs`
    }

    /// Count an answer from the loader; returns which request it answered.
    fn answer(&mut self) -> u32 {
        self.answered = (self.answered + 1).min(self.requested);
        if self.answered == self.requested {
            self.loading = None;
        }
        self.answered
    }

    /// Paint swatches of a car, if known; otherwise starts reading them (one car at a time).
    pub fn paints(&mut self, bundle: &str, store: &Arc<AssetStore>) -> Option<&[[f32; 3]]> {
        if !self.paints.contains_key(bundle) && self.paint_job.is_none() {
            let job: Job<Vec<[f32; 3]>> = Arc::new(Mutex::new(None));
            let (out, store, path) = (job.clone(), store.clone(), bundle.to_string());
            crate::platform::thread::Builder::new()
                .name("mw-paints".into())
                .spawn(move || {
                    let paints = store
                        .bundle(&path)
                        .and_then(|b| store.vehicle_paints(&b).ok())
                        .unwrap_or_default()
                        .iter()
                        .map(|p| p.diffuse)
                        .collect();
                    *out.lock().unwrap() = Some(paints);
                })
                .expect("spawn paint thread");
            self.paint_job = Some((bundle.to_string(), job));
        }
        self.paints.get(bundle).map(Vec::as_slice)
    }

    fn set_status(&mut self, s: String, now: f32) {
        info!("garage: {s}");
        self.status = s;
        self.status_time = now;
    }
}

/// Read the catalogue on a helper thread (language table + vehicle list).
fn start_catalogue(mut garage: ResMut<Garage>, game: Res<Game>) {
    garage.startup_car = matches!(game.request, Request::Drive { .. });
    let job: Job<Vec<CatalogueEntry>> = Arc::new(Mutex::new(None));
    let (out, store) = (job.clone(), game.store.clone());
    crate::platform::thread::Builder::new()
        .name("mw-catalogue".into())
        .spawn(move || {
            let list = store
                .language(mw_assets::DEFAULT_LANGUAGE)
                .and_then(|lang| store.vehicles(&lang))
                .map_err(|e| warn!("car catalogue unavailable: {e}"))
                .unwrap_or_default();
            *out.lock().unwrap() = Some(catalogue(list));
        })
        .expect("spawn catalogue thread");
    garage.catalogue_job = Some(job);
}

/// Drivable cars with display names, sorted; repeated names (liveries) get a number.
fn catalogue(list: Vec<mw_assets::VehicleInfo>) -> Vec<CatalogueEntry> {
    let mut cars: Vec<CatalogueEntry> = list
        .into_iter()
        .filter_map(|v| {
            let bundle = v.hi_bundle?;
            let name = ascii_fold(&v.name);
            let (make, model) = split_make(&name);
            Some(CatalogueEntry { id: v.id, make, model, name, year: v.year, bundle })
        })
        .collect();
    cars.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    let mut seen: HashMap<String, usize> = HashMap::new();
    let totals = cars.iter().fold(HashMap::<String, usize>::new(), |mut m, c| {
        *m.entry(c.name.clone()).or_default() += 1;
        m
    });
    for c in &mut cars {
        if totals[&c.name] > 1 {
            let n = seen.entry(c.name.clone()).or_default();
            *n += 1;
            c.model = format!("{} ({n})", c.model);
        }
    }
    cars
}

/// Manufacturer and model from a display name ("Aston Martin DB5" -> "Aston Martin", "DB5").
fn split_make(name: &str) -> (String, String) {
    const TWO_WORD: [&str; 3] = ["Alfa Romeo", "Aston Martin", "Range Rover"];
    let lower = name.to_lowercase();
    for m in TWO_WORD {
        if lower.starts_with(&m.to_lowercase()) && name.len() > m.len() {
            return (m.to_string(), name[m.len()..].trim().to_string());
        }
    }
    match name.split_once(' ') {
        Some((make, model)) => (make.to_string(), model.to_string()),
        None => (name.to_string(), String::new()),
    }
}

/// The UI font is ASCII only: drop accents and symbols ("Coupé" -> "Coupe", "GT500™" -> "GT500").
pub fn ascii_fold(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            c if c.is_ascii() => Some(c),
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => Some('a'),
            'è' | 'é' | 'ê' | 'ë' => Some('e'),
            'ì' | 'í' | 'î' | 'ï' => Some('i'),
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => Some('o'),
            'ù' | 'ú' | 'û' | 'ü' => Some('u'),
            'ç' => Some('c'),
            'ñ' => Some('n'),
            'É' => Some('E'),
            'Ö' => Some('O'),
            'Ü' => Some('U'),
            _ => None,
        })
        .collect()
}

fn poll_jobs(mut garage: ResMut<Garage>, time: Res<Time>) {
    if garage.status_time.is_nan() {
        garage.status_time = time.elapsed_secs();
    }
    let list = garage.catalogue_job.as_ref().and_then(|job| job.poll_lock().and_then(|mut s| s.take()));
    if let Some(list) = list {
        info!("car catalogue: {} drivable cars", list.len());
        garage.catalogue = list;
        garage.catalogue_job = None;
    }
    let paints = garage
        .paint_job
        .as_ref()
        .and_then(|(bundle, job)| Some((bundle.clone(), job.poll_lock().and_then(|mut s| s.take())?)));
    if let Some((bundle, paints)) = paints {
        garage.paints.insert(bundle, paints);
        garage.paint_job = None;
    }
}

/// Send car requests to the loader; leave the car for the free camera.
#[allow(clippy::too_many_arguments)]
fn handle_actions(
    mut commands: Commands,
    mut actions: MessageReader<GarageAction>,
    mut garage: ResMut<Garage>,
    requests: Option<Res<LoaderRequests>>,
    cars: Query<Entity, (With<Car>, With<crate::drive::Player>)>,
    mut camera: Query<(Entity, &mut Transform, &mut Projection), crate::MainCamera>,
    game: Res<Game>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    for action in actions.read() {
        match action {
            GarageAction::Drive { name, bundle, paint_index } => {
                // `FUN_0073c180`: a car whose DLC pack is not owned stays listed but cannot be driven (`dlc.rs`).
                if let Some(pack) = crate::dlc::car_locked(bundle) {
                    garage.set_status(format!("{name}: {pack} not owned (DLC locked)"), now);
                    continue;
                }
                let Some(requests) = &requests else {
                    garage.set_status("the loader has not started yet".into(), now);
                    continue;
                };
                let req =
                    CarRequest { bundle: bundle.clone(), paint_index: *paint_index, traffic: None, opponent: None };
                if requests.0.send(req).is_err() {
                    garage.set_status("the loader thread has stopped".into(), now);
                    continue;
                }
                garage.requested += 1;
                garage.loading = Some(name.clone());
                garage.set_status(format!("loading {name} (paint {})", paint_index + 1), now);
            }
            GarageAction::FreeCamera => {
                for e in &cars {
                    commands.entity(e).despawn();
                }
                commands.remove_resource::<PendingCar>();
                commands.remove_resource::<audio::PendingEngine>();
                garage.loading = None;
                garage.cancelled = garage.requested;
                garage.current = None;
                garage.arrived = None;
                if let Ok((cam, mut t, mut projection)) = camera.single_mut() {
                    // Level the chase camera's roll; restore Bevy's default field of view.
                    let (yaw, pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
                    t.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
                    if let Projection::Perspective(p) = &mut *projection {
                        p.fov = PerspectiveProjection::default().fov;
                    }
                    // Screenshot runs stay deterministic: no input-driven camera.
                    if game.screenshot.is_none() {
                        commands.entity(cam).insert((free_camera(), FreeCameraState::default()));
                    }
                }
                garage.set_status("free camera".into(), now);
            }
        }
    }
}

/// The free camera's settings (as in `setup`).
pub fn free_camera() -> FreeCamera {
    FreeCamera { walk_speed: 30.0, run_speed: 150.0, ..default() }
}

type CameraNotCar = (With<Camera3d>, Without<Car>, Without<crate::reflection::PlanarReflectionCamera>);

/// Place a car the loader finished: the startup car at `--spawn`, otherwise in place of the
/// current car (keeping its position, heading and velocity) or on the ground below the camera.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn place_arrived_car(
    mut commands: Commands,
    mut garage: ResMut<Garage>,
    game: Res<Game>,
    time: Res<Time>,
    cars: Query<(Entity, &Transform, &LinearVelocity), (With<Car>, With<crate::drive::Player>)>,
    camera: Query<(Entity, &Transform), CameraNotCar>,
    mut cursors: Query<&mut CursorOptions>,
    spatial: SpatialQuery,
) {
    let Some(Arrived { spec, parts, name }) = garage.arrived.take() else { return };
    let now = time.elapsed_secs();
    let (spawn, heading, probe, velocity) = match name {
        None => {
            let Request::Drive { spawn, heading, .. } = game.request else { return };
            (spawn, heading, game.start_probe, Vec3::ZERO)
        }
        Some(name) => {
            if let Some((at, heading)) = garage.place_at.take() {
                garage.set_status(format!("{name} placed at {at:.1}"), now);
                (at, heading, 1.5, Vec3::ZERO)
            } else if let Some((_, t, v)) = cars.iter().next() {
                let fwd = t.rotation * Vec3::Z;
                garage.set_status(format!("{name} swapped in at {:.0} km/h", v.length() * 3.6), now);
                // The car's origin is at road level: a short probe finds the road it is on.
                (t.translation, fwd.x.atan2(fwd.z), 1.5, v.0)
            } else {
                let Ok((_, cam)) = camera.single() else { return };
                let ground = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
                let down = spatial.cast_ray(cam.translation, Dir3::NEG_Y, 5000.0, true, &ground);
                let Some(hit) = down else {
                    garage.set_status(format!("no ground below the camera for {name}"), now);
                    return;
                };
                let fwd = cam.forward();
                let heading = if fwd.x.abs() + fwd.z.abs() > 1e-3 { fwd.x.atan2(fwd.z) } else { 0.0 };
                garage.set_status(format!("{name} spawned below the camera"), now);
                (cam.translation - Vec3::Y * hit.distance, heading, 1.0, Vec3::ZERO)
            }
        }
    };

    // Despawning the car takes its children with it: wheels, body collider and the engine sound.
    for (e, ..) in &cars {
        commands.entity(e).despawn();
    }
    match spec.engine.clone() {
        Some(voice) if !game.mute || crate::audio_tap::on() => {
            commands.insert_resource(audio::PendingEngine(Some(audio::EngineAudio::new(voice))));
        }
        _ => commands.remove_resource::<audio::PendingEngine>(),
    }
    garage.current =
        Some(CurrentCar { bundle: spec.bundle.clone(), paint_index: spec.paint_index, paint_count: spec.paint_count });
    // Driving: the chase camera takes over from the free camera.
    if let Ok((cam, _)) = camera.single() {
        commands.entity(cam).remove::<(FreeCamera, FreeCameraState)>();
        for mut c in &mut cursors {
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
    }
    commands.insert_resource(PendingCar { spec: *spec, spawn, heading, parts, probe, velocity });
}

/// One step of `--swap-to`.
#[derive(Debug, Clone)]
pub enum ScriptStep {
    Drive { name: String, bundle: String, paint_index: usize },
    FreeCamera,
}

/// `--swap-to`: after loading, wait `delay` frames, take a step, wait for the car to be on the road,
/// repeat. `--screenshot` captures after the last step.
#[derive(Resource, Debug, Clone)]
pub struct SwapScript {
    steps: VecDeque<ScriptStep>,
    delay: u32,
    frames: u32,
    waiting: bool,
}

impl SwapScript {
    pub fn new(steps: Vec<ScriptStep>, delay: u32) -> Self {
        Self { steps: steps.into(), delay, frames: 0, waiting: false }
    }

    pub fn finished(&self) -> bool {
        self.steps.is_empty() && !self.waiting
    }
}

#[allow(clippy::too_many_arguments)]
fn run_script(
    script: Option<ResMut<SwapScript>>,
    loaded: Res<Loaded>,
    garage: Res<Garage>,
    pending: Option<Res<PendingCar>>,
    mut actions: MessageWriter<GarageAction>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<crate::materials::GameMaterial>>,
    entities: Query<()>,
    engines: Query<(), With<audio::EngineSound>>,
) {
    let Some(mut script) = script else { return };
    let settled = loaded.done
        && garage.loading.is_none()
        && garage.arrived.is_none()
        && !garage.startup_car
        && pending.is_none()
        && garage.catalogue_job.is_none();
    if !settled {
        return;
    }
    if script.waiting {
        script.waiting = false;
        script.frames = 0;
    }
    script.frames += 1;
    if script.frames < script.delay {
        return;
    }
    if script.frames == script.delay {
        // Repeated swaps must not grow these (models are cached, old cars despawned).
        info!(
            "--swap-to: {} models, {} meshes, {} materials, {} entities, {} engine sounds",
            loaded.models.len(),
            meshes.len(),
            materials.len(),
            entities.iter().count(),
            engines.iter().count()
        );
    }
    let Some(step) = script.steps.pop_front() else { return };
    script.frames = 0;
    info!("--swap-to step: {step:?}");
    match step {
        ScriptStep::Drive { name, bundle, paint_index } => {
            actions.write(GarageAction::Drive { name, bundle, paint_index });
            script.waiting = true;
        }
        ScriptStep::FreeCamera => {
            actions.write(GarageAction::FreeCamera);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes() {
        assert_eq!(split_make("Aston Martin DB5"), ("Aston Martin".into(), "DB5".into()));
        assert_eq!(split_make("Porsche 911 GT2"), ("Porsche".into(), "911 GT2".into()));
        assert_eq!(split_make("BAC"), ("BAC".into(), String::new()));
    }

    #[test]
    fn fold() {
        assert_eq!(ascii_fold("BMW M3 Coupé"), "BMW M3 Coupe");
        assert_eq!(ascii_fold("Shelby GT500™"), "Shelby GT500");
    }
}
