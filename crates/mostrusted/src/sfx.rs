//! The player car's tyre, road and wind sounds, from the game's sound data (`mw_assets::sfx`): one sequence bank
//! rendered by `mw_audio::sfx_player::Player`, driven by the skid wheel (`Car::skid_wheel`, as the game picks
//! it). The body's impacts, the grind against other cars and the damage sounds are `player_sfx.rs`; landings
//! are `world_sfx.rs` (`FUN_00576b40`; the old loudness-sorted landing thump here was a model).

use crate::platform::PollLock;
use std::num::NonZero;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_assets::sfx::PlayerSurfaceSounds;
use mw_assets::vehicle_driver::{TyreSoundParams, VehicleDriver, VehicleInput};
use mw_audio::sfx_player::{Bank, OUTPUT_RATE, Params, Player};

use crate::drive::{Car, CarControls};

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<SfxAudio>()
            .init_resource::<SfxState>()
            .add_systems(Update, (load_bank, update_params).chain());
    }
}

#[derive(Asset, TypePath, Clone)]
pub struct SfxAudio {
    bank: Bank,
    params: Arc<Params>,
    stats: Arc<SfxStats>,
}

/// What the audio thread measures of the bank for `MW_AUDIO_METER` (flushed every 1024 samples).
#[derive(Default)]
struct SfxStats {
    /// Sums of squares before and after the soft limiter (f64 bits), the samples, the largest sample before the
    /// limiter (f32 bits), the sounding voices at the last flush, the render time (ns).
    linear: AtomicU64,
    out: AtomicU64,
    samples: AtomicU64,
    peak: AtomicU32,
    voices: AtomicU32,
    nanos: AtomicU64,
}

pub struct SfxSource {
    player: Box<Player>,
    stats: Option<Arc<SfxStats>>,
    /// Linear energy, output energy, samples, peak and render time since the last flush.
    block: (f64, f64, u64, f32, u64),
    /// `MW_AUDIO_STALL` is set: [`STALL_MS`] is read.
    stall: bool,
}

/// `MW_AUDIO_STALL=<s>|teleport,<ms>[,<n>,<every s>]`: at `s` seconds of game time (or at the first frame whose
/// speed jumps by more than 30 mph: a teleport) the audio thread is held `ms` ms, `n` times `every` seconds apart: an
/// underrun of the output, as the user's load bursts gave (a test switch).
static STALL_MS: AtomicU64 = AtomicU64::new(0);

/// A stall plan: start (`None`: at a teleport), ms, count, spacing (s).
type StallPlan = (Option<f32>, u64, u32, f32);

fn stall_test(now: f32, jump: bool, done: &mut u32, start: &mut Option<f32>) {
    static PLAN: std::sync::OnceLock<Option<StallPlan>> = std::sync::OnceLock::new();
    let Some((at, ms, n, every)) = *PLAN.get_or_init(|| {
        let v = std::env::var("MW_AUDIO_STALL").ok()?;
        let mut it = v.split(',');
        let at = it.next()?;
        let f: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
        let at = if at == "teleport" { None } else { Some(at.parse().ok()?) };
        Some((at, *f.first()? as u64, f.get(1).map_or(1, |&n| n as u32), f.get(2).copied().unwrap_or(0.3)))
    }) else {
        return;
    };
    if start.is_none() && (at.is_some_and(|t| now >= t) || (at.is_none() && jump)) {
        *start = Some(now);
    }
    let Some(t0) = *start else { return };
    if *done < n && now >= t0 + *done as f32 * every {
        *done += 1;
        info!("MW_AUDIO_STALL: holding the audio thread {ms} ms at {now:.2} s");
        STALL_MS.store(ms, Relaxed);
    }
}

impl Iterator for SfxSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        if self.stall {
            // `MW_AUDIO_STALL`: the game thread asked to hold the audio thread (an underrun, as a load burst gives).
            let ms = STALL_MS.swap(0, Relaxed);
            if ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
        let Some(stats) = &self.stats else { return Some(self.player.next_sample()) };
        let t0 = crate::platform::Instant::now();
        let x = self.player.next_sample_linear();
        let y = x.tanh();
        let b = &mut self.block;
        b.4 += t0.elapsed().as_nanos() as u64;
        b.0 += (x * x) as f64;
        b.1 += (y * y) as f64;
        b.2 += 1;
        b.3 = b.3.max(x.abs());
        if b.2 == 1024 {
            let add = |a: &AtomicU64, v: f64| a.store((f64::from_bits(a.load(Relaxed)) + v).to_bits(), Relaxed);
            add(&stats.linear, b.0);
            add(&stats.out, b.1);
            stats.samples.fetch_add(b.2, Relaxed);
            stats.nanos.fetch_add(b.4, Relaxed);
            stats.peak.fetch_max(b.3.to_bits(), Relaxed);
            stats.voices.store(self.player.voice_count() as u32, Relaxed);
            *b = (0.0, 0.0, 0, 0.0, 0);
        }
        Some(y)
    }
}

impl Source for SfxSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(1).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(OUTPUT_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for SfxAudio {
    type Decoder = SfxSource;
    fn decoder(&self) -> SfxSource {
        let mut p = Player::new(self.bank.clone(), self.params.clone());
        p.dry = std::env::var_os("MW_AUDIO_DRY").is_some();
        let stats = std::env::var_os("MW_AUDIO_METER").is_some().then(|| self.stats.clone());
        let stall = std::env::var_os("MW_AUDIO_STALL").is_some();
        SfxSource { player: Box::new(p), stats, block: (0.0, 0.0, 0, 0.0, 0), stall }
    }
}

/// What the worker thread builds: the bank, its surface sound sets and the skid tuning.
type Built = (Bank, PlayerSurfaceSounds, Option<TyreSoundParams>);

/// The surface sound sets' fades (`FUN_0058fe10`): each wheel picks a set by its surface and the tyre class
/// (`FUN_0054a5d0`); the car keeps two road slots (+0x1968, two 0x20-byte records), a set taking a slot fades in
/// over its +0x80 and one losing its slot is released over its +0x84. Here: the (at most two) sets under the most
/// wheels in contact are on, a wheel in the air counts for none (its slot entry is cleared, so the road noise fades
/// out in the air), ties keep the sets already playing.
#[derive(Default)]
struct SurfaceFades {
    picks: PlayerSurfaceSounds,
    gain: Vec<f32>,
    names: Vec<String>,
    params: Option<Arc<Params>>,
}

/// `FUN_0058fe10`'s two road slots.
const SURFACE_SLOTS: usize = 2;

impl SurfaceFades {
    fn new(picks: PlayerSurfaceSounds, params: Arc<Params>) -> Self {
        let n = picks.fades.len();
        Self {
            picks,
            gain: vec![0.0; n],
            names: (0..n).map(mw_assets::sfx::surface_gate).collect(),
            params: Some(params),
        }
    }

    fn update(&mut self, dt: f32, sets: &[Option<usize>]) {
        let Some(params) = self.params.clone() else { return };
        let n = self.gain.len();
        let mut count = vec![0u32; n];
        for k in sets.iter().flatten().filter(|&&k| k < n) {
            count[*k] += 1;
        }
        let mut order: Vec<usize> = (0..n).filter(|&k| count[k] > 0).collect();
        order.sort_by(|&a, &b| count[b].cmp(&count[a]).then(self.gain[b].total_cmp(&self.gain[a])));
        order.truncate(SURFACE_SLOTS);
        for k in 0..n {
            let on = order.contains(&k);
            let (fade_in, fade_out) = self.picks.fades[k];
            let g = &mut self.gain[k];
            let before = *g;
            *g = if on {
                if fade_in > 0.0 { (*g + dt / fade_in).min(1.0) } else { 1.0 }
            } else if fade_out > 0.0 {
                (*g - dt / fade_out).max(0.0)
            } else {
                0.0
            };
            if *g != before || on {
                params.set(&self.names[k], *g);
            }
        }
    }
}

#[derive(Resource, Default)]
struct SfxState {
    /// The bank, built on a worker thread (it decodes a few hundred waves).
    building: Option<Arc<Mutex<Option<Built>>>>,
    started: bool,
    driver: Option<VehicleDriver>,
    surfaces: SurfaceFades,
    params: Option<Arc<Params>>,
    /// The bank's channels carry the game's bus gains (`mw_assets::buses`), set against the engine's route.
    bus_mix: bool,
    /// The level last set (logged when it changes).
    last_level: f32,
    /// The live source's measurements (`MW_AUDIO_METER`) and when they were last printed.
    stats: Option<Arc<SfxStats>>,
    next_meter: f32,
    /// `MW_AUDIO_STALL`: holds done, when they started, the last frame's speed (mph).
    stalls: u32,
    stall_start: Option<f32>,
    last_mph: f32,
    last_car: Option<Entity>,
}

/// The bank's level against the player's engine. Its channels play at the data's level times their bus chain over
/// the engine's ("Player Car", `mw_assets::buses`), which is right while the engine also plays at its data level;
/// but the engine bank is normalised (`EngineBank::normalise`, a model), on the 911 Carrera S by -8.5 dB, so the
/// bank follows that gain. Only downwards: an engine bank under the normalised level may lack the part of its mix
/// that leaves through sends to buses outside the bundle, so its data level says nothing (a model decision).
/// `MW_SFX_ENGINE_LEVEL=0` leaves it out (A/B); without the bus gains (`MW_AUDIO_NO_BUS`) the old model gains,
/// measured against the normalised engine, stand.
fn bank_level(engine: Option<f32>, bus_mix: bool) -> f32 {
    if !bus_mix || std::env::var("MW_SFX_ENGINE_LEVEL").is_ok_and(|v| v == "0") {
        return 1.0;
    }
    engine.unwrap_or(1.0).min(1.0)
}

fn load_bank(
    mut commands: Commands,
    mut state: ResMut<SfxState>,
    mut assets: ResMut<Assets<SfxAudio>>,
    game: Res<crate::Game>,
) {
    if state.driver.is_some() || (game.mute && std::env::var_os("MW_SFX_TRACE").is_none() && !crate::audio_tap::on()) {
        return;
    }
    if !state.started {
        state.started = true;
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let store = game.store.clone();
        crate::platform::thread::spawn(move || {
            let (bank, picks) = store.vehicle_sfx_bank().unwrap_or_else(|e| {
                warn!("vehicle sounds: {e}");
                (Bank::default(), PlayerSurfaceSounds::default())
            });
            let tyre = store.tyre_sound_params();
            if tyre.is_none() {
                warn!("vehicle sounds: no TyreSoundParams, using the built-in copy");
            }
            *out.lock().unwrap() = Some((bank, picks, tyre));
        });
        state.building = Some(slot);
        return;
    }
    let Some(slot) = state.building.clone() else { return };
    let Some((bank, picks, tyre)) = slot.poll_lock().and_then(|mut s| s.take()) else { return };
    state.building = None;
    info!(
        "vehicle sounds: {} sequences, {} channels, {} surface sound sets",
        bank.sequences.len(),
        bank.channels.len(),
        picks.fades.len()
    );

    let params = Arc::new(Params::new(bank.param_names()));
    state.surfaces = SurfaceFades::new(picks, params.clone());
    state.params = Some(params.clone());
    state.bus_mix = !game.store.bus_mix().is_empty();
    let mut driver = VehicleDriver::new(params.clone());
    if let Some(t) = tyre {
        driver.tyre = t;
    }
    state.driver = Some(driver);
    crate::audio_tap::add_sfx(Player::new(bank.clone(), params.clone()));
    let stats = Arc::new(SfxStats::default());
    state.stats = Some(stats.clone());
    let handle = assets.add(SfxAudio { bank, params, stats });
    // ONCE: a live source never ends; LOOP would buffer it 0.74 s at a time (see audio.rs).
    // Starts silent; `settings::apply_volumes` sets the effects volume once the sink exists.
    commands.spawn((
        AudioPlayer::<SfxAudio>(handle),
        PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
        crate::settings::Channel::Sfx,
    ));
}

fn update_params(
    time: Res<Time>,
    mut state: ResMut<SfxState>,
    engine: Option<Res<crate::audio::PlayerEngineLevel>>,
    cars: Query<(Entity, &Car, &CarControls), With<crate::drive::Player>>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let state = &mut *state;
    let Some(driver) = state.driver.as_mut() else { return };
    if let Some(p) = &state.params {
        let level = bank_level(engine.map(|e| e.0), state.bus_mix);
        if (level - state.last_level).abs() > 1e-4 {
            info!("vehicle sounds: bank level {:.3} -> {level:.3}", state.last_level);
            state.last_level = level;
        }
        p.set_level(level);
    }
    let car = cars.single().ok();
    // A new vehicle (car change, respawn) starts its skid intensities silent (`VehicleDriver::reset`), instead of
    // carrying the old car's smoothed state.
    let id = car.map(|(e, _, _)| e);
    if id != state.last_car {
        if id.is_some() {
            driver.reset();
        }
        state.last_car = id;
    }
    // The player car's control block (its pedals also when the AI drives it in a cutscene).
    let (brake, handbrake) = car.map_or((0.0, false), |(_, _, c)| (c.brake, c.handbrake));
    let mut i = VehicleInput { brake, handbrake, ..default() };
    let mut landing = 0f32;
    let mut sets = [None; 4];
    if let Some((_, car, _)) = car {
        i.speed_mph = car.speed.abs() / 0.447_04;
        i.skid_wheel = car.skid_wheel();
        for c in car.wheel_contacts().flatten() {
            landing = landing.max(-c.velocity.dot(c.normal));
        }
        for (k, c) in car.wheel_contacts().enumerate().take(4) {
            sets[k] = c.and_then(|c| state.surfaces.picks.pick(c.surface, car.params.tyre_class));
        }
    }
    state.surfaces.update(dt, &sets);
    driver.update(dt, &i);
    trace(now, driver.last(), landing, &sets);
    let mph = car.map_or(0.0, |(_, c, _)| c.speed.abs() / 0.447_04);
    stall_test(now, (mph - state.last_mph).abs() > 30.0, &mut state.stalls, &mut state.stall_start);
    state.last_mph = mph;
    if let Some(stats) = &state.stats
        && std::env::var_os("MW_AUDIO_METER").is_some()
        && now >= state.next_meter
    {
        state.next_meter = now + 1.0;
        meter(stats, driver.last());
    }
}

/// `MW_AUDIO_METER`: the live vehicle bank once a second: its level before the soft limiter (RMS and peak), after
/// it, the sounding voices and the audio thread's render time per second of audio.
fn meter(stats: &SfxStats, skid: [f32; 4]) {
    let take = |a: &AtomicU64| f64::from_bits(a.swap(0f64.to_bits(), Relaxed));
    let (linear, out) = (take(&stats.linear), take(&stats.out));
    let n = stats.samples.swap(0, Relaxed).max(1) as f64;
    let peak = f32::from_bits(stats.peak.swap(0, Relaxed));
    let cpu = stats.nanos.swap(0, Relaxed) as f64 / 1e9 / (n / OUTPUT_RATE as f64) * 100.0;
    let db = |ms: f64| 10.0 * (ms + 1e-12).log10();
    info!(
        "vehicle sounds: linear {:6.1} dBFS (peak {:5.2}), out {:6.1} dBFS, {} voices, cpu {cpu:.2} %, {:.0} mph, skid {:.2}/{:.2}/{:.2}",
        db(linear / n),
        peak,
        db(out / n),
        stats.voices.load(Relaxed),
        skid[0],
        skid[1],
        skid[2],
        skid[3]
    );
}

/// `MW_SFX_TRACE=<file>`: one line per frame, `seconds mph lateral spin braking landing s0 s1 s2 s3` (skid
/// intensities 0..1, the wheels' touchdown speed in m/s, each wheel's surface sound set or -1 in the air).
fn trace(t: f32, v: [f32; 4], landing: f32, sets: &[Option<usize>; 4]) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| std::fs::File::create(std::env::var_os("MW_SFX_TRACE")?).ok().map(Mutex::new));
    if let Some(f) = file {
        let s = sets.map(|s| s.map_or(-1, |k| k as i32));
        _ = writeln!(
            f.lock().unwrap(),
            "{t:.4} {:.1} {:.3} {:.3} {:.3} {landing:.2} {} {} {} {}",
            v[0],
            v[1],
            v[2],
            v[3],
            s[0],
            s[1],
            s[2],
            s[3]
        );
    }
}
