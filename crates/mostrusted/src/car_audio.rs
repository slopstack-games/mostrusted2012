//! Engine sounds of the cars the player does not drive (docs/decomp.md "Other cars' engine sounds"):
//!
//! - **AI racers**: every opponent plays its car's competitor engine (`<id>_COMPETITORSOUND`, loaded by the
//!   loader thread for opponents) through the same engine driver as the player's, from its own drivetrain, and
//!   the surface's competitor skid sequences through the player's skid driver (`mw_assets::sfx`).
//! - **Traffic**: the game's pool of five traffic engine instances (`FUN_0060bc20`): every 0.3 s the traffic
//!   cars within 85 m of the player's car are scored by speed, distance and whether they are in view, the best
//!   five keep or get an engine, cars beyond 85 m lose theirs (1.5 s fade), and when more cars want one than
//!   instances are free or fading, the oldest is taken. Each plays its vehicle type's traffic bank
//!   (`mw_assets::car_sound::TrafficEngine`).
//! - **Placement**: the listener is the camera. Gain, doppler and the listener / emitter focus follow the
//!   bank's first channel (`Spatial`, the game's `FUN_00ab9690` / `FUN_00ab7260`); the stereo balance is ours
//!   ([`mw_audio::spatial`]).
//!
//! `MW_AUDIO_METER=1` prints the voices once a second (distance, gain, doppler, RPM, the level the audio thread
//! rendered) and the audio thread's time per voice. The player's engine (`audio.rs`) is unchanged.

use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;
use mw_assets::car_sound::{
    CompetitorEngine, Spatial, TrafficEngine, TrafficEngineDriver, TrafficEngineInput, pool, traffic_normalised_speed,
};
use mw_assets::engine_driver::{DriveState, EngineDriver, EngineInput};
use mw_audio::sequence::{OUTPUT_RATE, Params, Player};
use mw_audio::spatial::{SpatialParams, SpatialRender};

use crate::drive::{Car, CarControls};

pub struct CarAudioPlugin;

impl Plugin for CarAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<SpatialAudio>()
            .init_resource::<TrafficVoices>()
            .init_resource::<Listener>()
            .init_resource::<CompetitorTyres>()
            .add_systems(
                Update,
                (
                    update_listener,
                    attach_ai_voices,
                    update_ai_voices,
                    attach_ai_tyres,
                    update_ai_tyres,
                    load_traffic_banks,
                    traffic_pool,
                    meter,
                )
                    .chain(),
            );
    }
}

/// The AI's gear-change sequences in the competitor banks bind `LocalPlayer.GetPlayerNormalisedGearChangeTimeElapsed`
/// (the same name as the player's bank): read literally, an AI car's gear-change sound runs on the local
/// player's shift clock. Kept as the data says.
const AI_GEAR_CLOCK_FROM_PLAYER: bool = true;

/// Game-thread time spent in this module's updates (ns), for `MW_AUDIO_METER`.
static GAME_NANOS: AtomicU64 = AtomicU64::new(0);
static GAME_FRAMES: AtomicU64 = AtomicU64::new(0);

/// Adds the time until it is dropped to [`GAME_NANOS`].
struct Timed(std::time::Instant);

impl Timed {
    fn start() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        GAME_NANOS.fetch_add(self.0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

// --- The audio source ----------------------------------------------------------------------------------------

/// Counters the audio thread fills for `MW_AUDIO_METER`.
#[derive(Default)]
pub(crate) struct VoiceStats {
    /// Sum of squares of the rendered output (both channels), as f64 bits, and the frames.
    energy: AtomicU64,
    frames: AtomicU64,
    /// Time spent rendering (ns).
    nanos: AtomicU64,
}

impl VoiceStats {
    /// Mean square since the last call (resets).
    pub(crate) fn take(&self) -> (f64, u64, u64) {
        let e = f64::from_bits(self.energy.swap(0f64.to_bits(), Ordering::Relaxed));
        (e, self.frames.swap(0, Ordering::Relaxed), self.nanos.swap(0, Ordering::Relaxed))
    }
}

/// What a placed voice renders: an engine bank (`mw_audio::sequence`), a tyre bank (`mw_audio::sfx_player`), or
/// a one-shot (`car_sfx.rs`: an `sfx_player` bank whose source ends once its cue has played, and whose clock runs
/// on while it is out of earshot).
#[derive(Clone)]
pub(crate) enum VoiceBank {
    Engine(mw_audio::sequence::Bank, Arc<Params>),
    Tyres(mw_audio::sfx_player::Bank, Arc<mw_audio::sfx_player::Params>),
    OneShot(mw_audio::sfx_player::Bank, Arc<mw_audio::sfx_player::Params>),
}

enum VoicePlayer {
    Engine(Box<Player>),
    Tyres(Box<mw_audio::sfx_player::Player>),
}

impl VoicePlayer {
    fn next_sample(&mut self) -> f32 {
        match self {
            VoicePlayer::Engine(p) => p.next_sample(),
            VoicePlayer::Tyres(p) => p.next_sample(),
        }
    }
}

/// A placed voice of another car (its engine or its tyres).
#[derive(Asset, TypePath, Clone)]
pub struct SpatialAudio {
    bank: VoiceBank,
    spatial: Arc<SpatialParams>,
    stats: Arc<VoiceStats>,
}

pub struct SpatialSource {
    player: VoicePlayer,
    /// A one-shot's parameters: the source ends when they report the cue finished.
    one_shot: Option<Arc<mw_audio::sfx_player::Params>>,
    render: SpatialRender,
    right: Option<f32>,
    stats: Arc<VoiceStats>,
    /// Energy, frames and render time (ns) since the last flush into `stats`.
    block: (f64, u64, u64),
    /// Time the rendering (`MW_AUDIO_METER`).
    timing: bool,
}

impl Iterator for SpatialSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        if self.one_shot.as_ref().is_some_and(|p| p.finished()) {
            return None;
        }
        let t0 = self.timing.then(std::time::Instant::now);
        let player = &mut self.player;
        let (l, r) = self.render.frame(&mut || player.next_sample());
        if let Some(t0) = t0 {
            self.block.2 += t0.elapsed().as_nanos() as u64;
        }
        self.right = Some(r);
        self.block.0 += (l * l + r * r) as f64;
        self.block.1 += 1;
        if self.block.1 == 1024 {
            let s = &self.stats;
            let e = f64::from_bits(s.energy.load(Ordering::Relaxed)) + self.block.0;
            s.energy.store(e.to_bits(), Ordering::Relaxed);
            s.frames.fetch_add(self.block.1, Ordering::Relaxed);
            s.nanos.fetch_add(self.block.2, Ordering::Relaxed);
            self.block = (0.0, 0, 0);
        }
        Some(l)
    }
}

impl Source for SpatialSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(2).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(OUTPUT_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for SpatialAudio {
    type Decoder = SpatialSource;
    fn decoder(&self) -> SpatialSource {
        let dry = std::env::var_os("MW_AUDIO_DRY").is_some();
        let player = match &self.bank {
            VoiceBank::Engine(bank, params) => {
                let mut p = Player::new(bank.clone(), params.clone());
                p.dry = dry;
                p.bypass = mw_audio::sequence::Bypass::from_env();
                VoicePlayer::Engine(Box::new(p))
            }
            VoiceBank::Tyres(bank, params) | VoiceBank::OneShot(bank, params) => {
                let mut p = mw_audio::sfx_player::Player::new(bank.clone(), params.clone());
                p.dry = dry;
                VoicePlayer::Tyres(Box::new(p))
            }
        };
        let one_shot = match &self.bank {
            VoiceBank::OneShot(_, params) => Some(params.clone()),
            _ => None,
        };
        let render = SpatialRender::new(self.spatial.clone());
        SpatialSource {
            player,
            render: if one_shot.is_some() { render.keep_running() } else { render },
            one_shot,
            right: None,
            stats: self.stats.clone(),
            block: (0.0, 0, 0),
            timing: std::env::var_os("MW_AUDIO_METER").is_some(),
        }
    }
}

fn spawn_voice(
    commands: &mut Commands,
    assets: &mut Assets<SpatialAudio>,
    bank: VoiceBank,
) -> (Entity, Arc<SpatialParams>, Arc<VoiceStats>) {
    let spatial = Arc::new(SpatialParams::default());
    let stats = Arc::new(VoiceStats::default());
    let channel = match bank {
        VoiceBank::Engine(..) => crate::settings::Channel::Engine,
        VoiceBank::Tyres(..) | VoiceBank::OneShot(..) => crate::settings::Channel::Sfx,
    };
    let handle = assets.add(SpatialAudio { bank, spatial: spatial.clone(), stats: stats.clone() });
    // ONCE: the source never ends (see audio.rs); starts silent until `settings::apply_volumes` sets it.
    let e = commands
        .spawn((AudioPlayer::<SpatialAudio>(handle), PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)), channel))
        .id();
    (e, spatial, stats)
}

/// A one-shot voice (`car_sfx.rs`): plays `bank`'s cue once and despawns when it has finished. Spawned at the
/// effects volume (a sink added at volume 0 would lose the first frame's attack).
pub(crate) fn spawn_one_shot(
    commands: &mut Commands,
    assets: &mut Assets<SpatialAudio>,
    bank: mw_audio::sfx_player::Bank,
    params: Arc<mw_audio::sfx_player::Params>,
    volume: f32,
) -> (Entity, Arc<SpatialParams>, Arc<VoiceStats>) {
    let spatial = Arc::new(SpatialParams::default());
    let stats = Arc::new(VoiceStats::default());
    let handle = assets.add(SpatialAudio {
        bank: VoiceBank::OneShot(bank, params),
        spatial: spatial.clone(),
        stats: stats.clone(),
    });
    let e = commands
        .spawn((
            AudioPlayer::<SpatialAudio>(handle),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
            crate::settings::Channel::Sfx,
        ))
        .id();
    (e, spatial, stats)
}

// --- The listener ----------------------------------------------------------------------------------------------

/// The camera as the listener: position, axes and velocity (from its movement).
#[derive(Resource, Default)]
pub(crate) struct Listener {
    pub position: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub velocity: Vec3,
    pub valid: bool,
    /// The previous frame's time step: the camera's global transform read here is the one its follow system set
    /// in the previous frame (it propagates in `PostUpdate`), so it moved over that frame's step.
    last_dt: f32,
}

pub(crate) fn update_listener(
    time: Res<Time>,
    mut l: ResMut<Listener>,
    cam: Query<&GlobalTransform, (crate::MainCamera, With<Frustum>)>,
) {
    let Some(t) = cam.iter().next() else { return };
    let p = t.translation();
    // `MW_LISTENER_DT=current` divides by this frame's step instead (A/B: the velocity then jitters with the frame
    // times, and with it every other car's doppler pitch).
    let current = std::env::var("MW_LISTENER_DT").is_ok_and(|v| v == "current");
    let dt = if current { time.delta_secs() } else { l.last_dt };
    l.last_dt = time.delta_secs();
    if l.valid && dt > 0.0 {
        let v = (p - l.position) / dt;
        // A cut (teleport, camera switch) is not a velocity.
        l.velocity = if v.length() < 150.0 { v } else { Vec3::ZERO };
    }
    listener_trace(time.elapsed_secs(), time.delta_secs(), l.velocity);
    l.position = p;
    l.forward = t.forward().as_vec3();
    l.right = t.right().as_vec3();
    l.valid = true;
}

/// `MW_LISTENER_TRACE=<file>`: one line per frame, `seconds dt speed vx vy vz` of the listener (the camera).
fn listener_trace(t: f32, dt: f32, v: Vec3) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE
        .get_or_init(|| std::fs::File::create(std::env::var_os("MW_LISTENER_TRACE")?).ok().map(std::sync::Mutex::new));
    if let Some(f) = file {
        _ = writeln!(f.lock().unwrap(), "{t:.4} {dt:.5} {:.3} {:.3} {:.3} {:.3}", v.length(), v.x, v.y, v.z);
    }
}

/// Gain, pan and doppler of an emitter at `position` moving at `velocity` facing `forward`.
pub(crate) fn place(s: &Spatial, l: &Listener, position: Vec3, velocity: Vec3, forward: Vec3) -> (f32, f32, f32, f32) {
    let d = position - l.position;
    let dist = d.length();
    let u = if dist > 1e-3 { d / dist } else { l.forward };
    let gain = s.gain(dist, forward.dot(u), l.forward.dot(u));
    let pitch = s.doppler((velocity - l.velocity).dot(u));
    (gain, u.dot(l.right), pitch, dist)
}

// --- AI racers -------------------------------------------------------------------------------------------------

/// Put on an opponent's car by `ai.rs`: its competitor engine, to be started here.
#[derive(Component)]
pub struct PendingCompetitorEngine(pub Arc<CompetitorEngine>);

/// An AI car's engine voice (on the sound entity, a child of the car).
#[derive(Component)]
struct AiVoice {
    car: Entity,
    driver: EngineDriver,
    spatial: Spatial,
    params: Arc<SpatialParams>,
    stats: Arc<VoiceStats>,
    last: (f32, f32, f32, f32),
}

fn attach_ai_voices(
    mut commands: Commands,
    game: Res<crate::Game>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    cars: Query<(Entity, &PendingCompetitorEngine)>,
) {
    for (car, pending) in &cars {
        commands.entity(car).remove::<PendingCompetitorEngine>();
        if game.mute {
            continue;
        }
        let eb = &pending.0.engine;
        let params = Arc::new(Params::new(eb.bank.param_names()));
        let driver = EngineDriver::new(eb, params.clone());
        let (e, sp, stats) = spawn_voice(&mut commands, &mut assets, VoiceBank::Engine(eb.bank.clone(), params));
        commands.entity(e).insert((
            AiVoice { car, driver, spatial: pending.0.spatial, params: sp, stats, last: (0.0, 0.0, 1.0, 0.0) },
            ChildOf(car),
        ));
        commands.entity(car).insert(NeedsTyreVoice);
    }
}

/// An AI car whose tyre voice is still to start (the competitor tyre bank loads on a worker thread).
#[derive(Component)]
struct NeedsTyreVoice;

/// The competitor skid sequences (`mw_assets::sfx::competitor_tyre_bank`) and the skid tuning, shared by every AI
/// car's tyre voice.
type TyreBank = (mw_audio::sfx_player::Bank, Spatial, Option<mw_assets::vehicle_driver::TyreSoundParams>);

#[derive(Resource, Default)]
struct CompetitorTyres {
    bank: Option<Arc<TyreBank>>,
    job: Option<Arc<Mutex<Option<TyreBank>>>>,
}

/// An AI car's tyre voice (skids from its own skid wheel, as the player's `sfx.rs`).
#[derive(Component)]
struct AiTyreVoice {
    car: Entity,
    driver: mw_assets::vehicle_driver::VehicleDriver,
    spatial: Spatial,
    params: Arc<SpatialParams>,
    stats: Arc<VoiceStats>,
    last: (f32, f32, f32, f32),
}

fn attach_ai_tyres(
    mut commands: Commands,
    game: Res<crate::Game>,
    mut tyres: ResMut<CompetitorTyres>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    cars: Query<Entity, With<NeedsTyreVoice>>,
) {
    if cars.is_empty() {
        return;
    }
    let Some(bank) = tyres.bank.clone() else {
        match tyres.job.clone() {
            None => {
                let job = Arc::new(Mutex::new(None));
                let out = job.clone();
                let store = game.store.clone();
                std::thread::spawn(move || {
                    let (bank, spatial) = store.competitor_tyre_bank().unwrap_or_else(|e| {
                        warn!("competitor tyre sounds: {e}");
                        Default::default()
                    });
                    *out.lock().unwrap() = Some((bank, spatial, store.tyre_sound_params()));
                });
                tyres.job = Some(job);
            }
            Some(job) => {
                if let Some(b) = job.lock().unwrap().take() {
                    info!("competitor tyre sounds: {} sequences, 3D {:?}", b.0.sequences.len(), b.1.falloff);
                    tyres.bank = Some(Arc::new(b));
                }
            }
        }
        return;
    };
    for car in &cars {
        commands.entity(car).remove::<NeedsTyreVoice>();
        if bank.0.sequences.is_empty() {
            continue;
        }
        let params = Arc::new(mw_audio::sfx_player::Params::new(bank.0.param_names()));
        let mut driver = mw_assets::vehicle_driver::VehicleDriver::new(params.clone());
        if let Some(t) = bank.2 {
            driver.tyre = t;
        }
        let (e, sp, stats) = spawn_voice(&mut commands, &mut assets, VoiceBank::Tyres(bank.0.clone(), params));
        commands.entity(e).insert((
            AiTyreVoice { car, driver, spatial: bank.1, params: sp, stats, last: (0.0, 0.0, 1.0, 0.0) },
            ChildOf(car),
        ));
    }
}

fn update_ai_tyres(
    time: Res<Time>,
    listener: Res<Listener>,
    cars: Query<(&Car, &CarControls, &Transform, &LinearVelocity), Without<crate::drive::Player>>,
    mut voices: Query<&mut AiTyreVoice>,
) {
    let _timed = Timed::start();
    for mut v in &mut voices {
        let Ok((car, controls, t, vel)) = cars.get(v.car) else { continue };
        let i = mw_assets::vehicle_driver::VehicleInput {
            speed_mph: car.speed.abs() / 0.447_04,
            skid_wheel: car.skid_wheel(),
            brake: controls.brake,
            handbrake: controls.handbrake,
        };
        v.driver.update(time.delta_secs(), &i);
        let placed = place(&v.spatial, &listener, t.translation, vel.0, t.rotation * Vec3::Z);
        v.params.set(placed.0, placed.1, placed.2);
        v.last = placed;
    }
}

#[allow(clippy::type_complexity)]
fn update_ai_voices(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    listener: Res<Listener>,
    cars: Query<(&Car, &CarControls, &Transform, &LinearVelocity), Without<crate::drive::Player>>,
    player: Query<&Car, With<crate::drive::Player>>,
    mut voices: Query<&mut AiVoice>,
) {
    let _timed = Timed::start();
    let player_elapsed = player.iter().next().map(|c| c.drivetrain.gear_change_elapsed());
    for mut v in &mut voices {
        let Ok((car, controls, t, vel)) = cars.get(v.car) else { continue };
        let throttle = if car.gear < 0 { controls.brake } else { controls.throttle };
        let rpm = car.rpm_prev + (car.rpm - car.rpm_prev) * fixed.overstep_fraction();
        let dt = &car.drivetrain;
        let dp = &car.params.drivetrain;
        let gear_change_elapsed = match player_elapsed {
            Some(e) if AI_GEAR_CLOCK_FROM_PLAYER => e,
            _ => dt.gear_change_elapsed(),
        };
        let drive = DriveState {
            engine_throttle: dt.throttle,
            clutch: dt.clutch,
            gear_change_elapsed,
            shifts_started: dt.shifts_started,
            last_shift_up: dt.last_shift_up,
            cruising: dt.cruising,
            top_gears: dt.in_top_gears(dp),
            gearbox_throttle: dt.gearbox_throttle,
            transmission_mph: mw_assets::engine_driver::transmission_mph(
                rpm,
                dp.active(dt.cruising).ratio(car.gear) * dp.active(dt.cruising).final_drive,
                car.rear_wheel_radius(),
            ),
        };
        let forward = t.rotation * Vec3::Z;
        let placed = place(&v.spatial, &listener, t.translation, vel.0, forward);
        if let Some(s) = dp.handling.param("Engine.937338e7") {
            v.driver.set_speed_load(s);
        }
        v.driver.set_listener_distance(Some(placed.3));
        let input = EngineInput {
            rpm,
            throttle,
            gear: car.gear,
            speed_mph: car.speed.abs() / 0.447_04,
            idle_rpm: car.params.idle_rpm,
            redline_rpm: car.params.redline_rpm,
            top_speed_mph: car.params.top_speed / 0.447_04,
            drive: Some(drive),
        };
        v.driver.update(time.delta_secs(), &input);
        v.params.set(placed.0, placed.1, placed.2);
        v.last = placed;
    }
}

// --- Traffic ---------------------------------------------------------------------------------------------------

struct TrafficSlot {
    key: u32,
    vehicle: usize,
    entity: Entity,
    driver: TrafficEngineDriver,
    spatial: Spatial,
    top_speed_mph: f32,
    random: f32,
    params: Arc<SpatialParams>,
    stats: Arc<VoiceStats>,
    /// Order of starting (the game steals the instance with the lowest voice handle: the oldest).
    serial: u64,
    /// Fading out (seconds left of the 1.5 s release).
    releasing: Option<f32>,
    /// The car's last state: position, velocity, forward.
    car: (Vec3, Vec3, Vec3),
    last: (f32, f32, f32, f32),
}

type BankJob = Arc<Mutex<Option<Vec<Option<Arc<TrafficEngine>>>>>>;

#[derive(Resource, Default)]
struct TrafficVoices {
    banks: Option<Vec<Option<Arc<TrafficEngine>>>>,
    job: Option<BankJob>,
    slots: Vec<Option<TrafficSlot>>,
    timer: f32,
    serial: u64,
    started: u64,
    stolen: u64,
}

fn load_traffic_banks(
    game: Res<crate::Game>,
    traffic: Option<Res<crate::traffic::TrafficSystem>>,
    mut voices: ResMut<TrafficVoices>,
) {
    if voices.banks.is_some() || game.mute {
        return;
    }
    if let Some(job) = voices.job.clone() {
        if let Some(banks) = job.lock().unwrap().take() {
            let n = banks.iter().flatten().count();
            info!("traffic sounds: {n} of {} vehicle types have an engine", banks.len());
            voices.banks = Some(banks);
            voices.job = None;
        }
        return;
    }
    let Some(traffic) = traffic else { return };
    let components: Vec<u32> = traffic.vehicles().iter().map(|v| v.component).collect();
    let store = game.store.clone();
    let job: BankJob = Arc::new(Mutex::new(None));
    let out = job.clone();
    std::thread::spawn(move || {
        let banks = components
            .iter()
            .map(|&c| match store.traffic_engine(c) {
                Ok(e) => e.map(Arc::new),
                Err(e) => {
                    warn!("traffic sound {c}: {e}");
                    None
                }
            })
            .collect();
        *out.lock().unwrap() = Some(banks);
    });
    voices.job = Some(job);
    voices.slots = (0..pool::SLOTS).map(|_| None).collect();
}

#[allow(clippy::too_many_arguments)]
fn traffic_pool(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    traffic: Option<Res<crate::traffic::TrafficSystem>>,
    env: Option<Res<crate::weather::EnvironmentState>>,
    player: Query<&Transform, (With<Car>, With<crate::drive::Player>)>,
    frustum: Query<&Frustum, crate::MainCamera>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut voices: ResMut<TrafficVoices>,
) {
    let _timed = Timed::start();
    GAME_FRAMES.fetch_add(1, Ordering::Relaxed);
    let voices = &mut *voices;
    let (Some(banks), Some(traffic)) = (voices.banks.clone(), traffic) else { return };
    let dt = time.delta_secs();
    // Cars near enough to matter: the pool's 85 m plus a margin for the ones still sounding.
    let centre = player.iter().next().map(|t| t.translation);
    let near = centre.map(|c| traffic.sound_cars(c, pool::RADIUS * 1.5)).unwrap_or_default();
    // `IsWetSurface`: the world's wet flag; ours is the weather's bad-weather keyframes (`--weather`, F7).
    let wet = env.is_some_and(|e| e.want.alternate);

    // The selection, every 0.3 s while the player has a car.
    voices.timer += dt;
    if let Some(centre) = centre
        && voices.timer >= pool::PERIOD
    {
        voices.timer = 0.0;
        let frustum = frustum.iter().next();
        let mut scored: Vec<(f32, &crate::traffic::TrafficSoundCar)> = near
            .iter()
            .filter(|c| banks.get(c.vehicle).is_some_and(|b| b.is_some()))
            .filter(|c| c.position.distance_squared(centre) <= pool::RADIUS * pool::RADIUS)
            .map(|c| {
                let in_view = frustum.is_some_and(|f| {
                    f.intersects_sphere(&Sphere { center: c.position.into(), radius: pool::FRUSTUM_SPHERE }, true)
                });
                (pool::score(c.position.distance_squared(centre), c.velocity.length_squared(), in_view), c)
            })
            .filter(|(s, _)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(pool::SLOTS);
        // Engines whose car is beyond 85 m (or gone) start their release.
        for s in voices.slots.iter_mut().flatten() {
            let car = near.iter().find(|c| c.key == s.key);
            let far = car.is_none_or(|c| c.position.distance_squared(centre) > pool::RADIUS * pool::RADIUS);
            if far && s.releasing.is_none() {
                s.releasing = Some(pool::RELEASE_FADE);
            }
        }
        let fading = voices.slots.iter().flatten().filter(|s| s.releasing.is_some()).count();
        let mut missing = 0;
        for (_, c) in &scored {
            if voices.slots.iter().flatten().any(|s| s.key == c.key) {
                continue;
            }
            let Some(free) = voices.slots.iter().position(|s| s.is_none()) else {
                missing += 1;
                continue;
            };
            let Some(Some(engine)) = banks.get(c.vehicle) else { continue };
            let params = Arc::new(Params::new(engine.bank.param_names()));
            let driver = TrafficEngineDriver::new(engine, params.clone());
            let (entity, sp, stats) =
                spawn_voice(&mut commands, &mut assets, VoiceBank::Engine(engine.bank.clone(), params));
            voices.serial += 1;
            voices.started += 1;
            voices.slots[free] = Some(TrafficSlot {
                key: c.key,
                vehicle: c.vehicle,
                entity,
                driver,
                spatial: engine.spatial,
                top_speed_mph: engine.top_speed_mph,
                random: c.random,
                params: sp,
                stats,
                serial: voices.serial,
                releasing: None,
                car: (c.position, c.velocity, c.forward),
                last: (0.0, 0.0, 1.0, 0.0),
            });
        }
        // More cars want an engine than slots are free or fading: release the oldest playing ones.
        for _ in fading..missing {
            let oldest = voices.slots.iter_mut().flatten().filter(|s| s.releasing.is_none()).min_by_key(|s| s.serial);
            if let Some(s) = oldest {
                s.releasing = Some(pool::RELEASE_FADE);
                voices.stolen += 1;
            }
        }
    }

    // Every frame: the bindings and the placement of each engine.
    for slot in voices.slots.iter_mut() {
        let Some(s) = slot else { continue };
        if let Some(c) = near.iter().find(|c| c.key == s.key) {
            s.car = (c.position, c.velocity, c.forward);
        }
        let (pos, vel, fwd) = s.car;
        let forward_speed = vel.dot(fwd);
        s.driver.update(&TrafficEngineInput {
            normalised_speed: traffic_normalised_speed(forward_speed, s.top_speed_mph),
            wet,
            reversing: forward_speed < 0.0,
            random: s.random,
        });
        let mut placed = place(&s.spatial, &listener, pos, vel, fwd);
        if let Some(left) = s.releasing.as_mut() {
            *left -= dt;
            placed.0 *= (*left / pool::RELEASE_FADE).clamp(0.0, 1.0);
            if *left <= 0.0 {
                commands.entity(s.entity).despawn();
                *slot = None;
                continue;
            }
        }
        s.params.set(placed.0, placed.1, placed.2);
        s.last = placed;
    }
}

// --- Diagnostics -----------------------------------------------------------------------------------------------

fn meter(
    time: Res<Time>,
    mut next: Local<f32>,
    voices: Res<TrafficVoices>,
    ai: Query<&AiVoice>,
    tyres: Query<&AiTyreVoice>,
    player: Query<&Transform, (With<Car>, With<crate::drive::Player>)>,
) {
    if std::env::var_os("MW_AUDIO_METER").is_none() {
        return;
    }
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let db = |x: f64| 10.0 * (x + 1e-12).log10();
    let line = |kind: &str, key: String, last: (f32, f32, f32, f32), rpm: f32, stats: &VoiceStats| {
        let (e, frames, nanos) = stats.take();
        let ms = e / (2.0 * frames.max(1) as f64);
        // Audio-thread time per second of audio rendered.
        let cpu = nanos as f64 / 1e9 / (frames.max(1) as f64 / OUTPUT_RATE as f64) * 100.0;
        format!(
            "  {kind} {key}: {:5.1} m  gain {:6.1} dB  doppler {:.3}  pan {:+.2}  rpm {rpm:5.0}  out {:6.1} dBFS  cpu {cpu:.2} %",
            last.3,
            20.0 * last.0.max(1e-6).log10(),
            last.2,
            last.1,
            db(ms)
        )
    };
    let mut out = Vec::new();
    for v in &ai {
        out.push(line("ai", format!("{:?}", v.car), v.last, v.driver.state().audible, &v.stats));
    }
    for v in &tyres {
        let skid = v.driver.last();
        out.push(line(
            "ai tyres",
            format!("{:?} skid {:.2}/{:.2}/{:.2}", v.car, skid[1], skid[2], skid[3]),
            v.last,
            0.0,
            &v.stats,
        ));
    }
    for s in voices.slots.iter().flatten() {
        let kind = if s.releasing.is_some() { "traffic (releasing)" } else { "traffic" };
        out.push(line(kind, format!("{} (type {})", s.key, s.vehicle), s.last, s.driver.rpm(), &s.stats));
    }
    let pos = player.iter().next().map(|t| t.translation);
    let frames = GAME_FRAMES.swap(0, Ordering::Relaxed).max(1);
    let game_us = GAME_NANOS.swap(0, Ordering::Relaxed) as f64 / frames as f64 / 1000.0;
    info!(
        "car audio at {:.0?}: {} AI, {} traffic voices (started {}, stolen {}), game thread {game_us:.0} µs/frame\n{}",
        pos,
        ai.iter().count(),
        voices.slots.iter().flatten().count(),
        voices.started,
        voices.stolen,
        out.join("\n")
    );
}
