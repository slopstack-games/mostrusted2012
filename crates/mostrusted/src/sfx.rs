//! The player car's tyre, road and wind sounds, from the game's sound data (`mw_assets::sfx`): one sequence bank
//! rendered by `mw_audio::sfx_player::Player`, driven by the skid wheel (`Car::skid_wheel`, as the game picks
//! it). The body's impacts, the grind against other cars and the damage sounds are `player_sfx.rs`; landings
//! are `world_sfx.rs` (`FUN_00576b40`; the old loudness-sorted landing thump here was a model).

use std::num::NonZero;
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
}

pub struct SfxSource(Box<Player>);

impl Iterator for SfxSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        Some(self.0.next_sample())
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
        SfxSource(Box::new(p))
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
        std::thread::spawn(move || {
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
    let Some((bank, picks, tyre)) = slot.lock().unwrap().take() else { return };
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
    let handle = assets.add(SfxAudio { bank, params });
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
        p.set_level(bank_level(engine.map(|e| e.0), state.bus_mix));
    }
    let car = cars.single().ok();
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
