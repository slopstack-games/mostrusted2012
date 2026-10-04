//! Game audio: the car's engine from its sound data (sequence bank, `mw_audio::sequence`), and music
//! from the game's `UI/SONGS` streams. Decoding and the sound runtime live in `mw-audio` / `mw-assets`.

use std::num::NonZero;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_assets::engine_driver::{EngineDriver, EngineInput};
use mw_assets::engine_sound::EngineVoice;
use mw_audio::Pcm;
use mw_audio::ginsu::EngineMixer;
use mw_audio::sequence::{Params, Player};

pub struct GameAudioPlugin {
    pub music: bool,
}

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PcmAudio>()
            .add_audio_source::<EngineAudio>()
            .init_resource::<MusicState>()
            .add_systems(Update, (engine_sound, update_engine_params));
        if self.music {
            app.add_systems(Update, music);
        }
    }
}

// --- Decoded clips -----------------------------------------------------------

/// A decoded clip (music, effects).
#[derive(Asset, TypePath, Clone)]
pub struct PcmAudio {
    pub pcm: Arc<Pcm>,
}

pub struct PcmSource {
    pcm: Arc<Pcm>,
    pos: usize,
}

impl Iterator for PcmSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        let s = *self.pcm.samples.get(self.pos)?;
        self.pos += 1;
        Some(s as f32 / 32768.0)
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(self.pcm.channels.max(1)).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(self.pcm.sample_rate.max(1)).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs_f32(self.pcm.duration_secs()))
    }
}

impl Decodable for PcmAudio {
    type Decoder = PcmSource;
    fn decoder(&self) -> PcmSource {
        PcmSource { pcm: self.pcm.clone(), pos: 0 }
    }
}

// --- Engine ------------------------------------------------------------------

/// Live engine parameters shared with the audio thread (cars without engine-sound data).
#[derive(Default)]
pub struct EngineParams {
    rpm: AtomicU32,
    throttle: AtomicU32,
}

impl EngineParams {
    pub fn set(&self, rpm: f32, throttle: f32) {
        self.rpm.store(rpm.to_bits(), Ordering::Relaxed);
        self.throttle.store(throttle.to_bits(), Ordering::Relaxed);
    }
    fn get(&self) -> (f32, f32) {
        (f32::from_bits(self.rpm.load(Ordering::Relaxed)), f32::from_bits(self.throttle.load(Ordering::Relaxed)))
    }
}

/// A car's engine. With `EngineSound2` data it is the car's whole sequence bank (Ginsu layers, idle loop,
/// load crossfades, the mixer graph with its EQ / distortion / sub-harmoniser, pops, backfires, limiter,
/// gear-change clunks) rendered by `mw_audio::sequence::Player`; otherwise Ginsu layers.
#[derive(Asset, TypePath, Clone)]
pub struct EngineAudio {
    pub voice: EngineVoice,
    /// The bank's bindings, written by the game thread through an `EngineDriver`.
    pub bank_params: Option<Arc<Params>>,
    pub legacy_params: Arc<EngineParams>,
}

impl EngineAudio {
    pub fn new(voice: EngineVoice) -> Self {
        let bank_params = match &voice {
            EngineVoice::Bank(b) => Some(Arc::new(Params::new(b.bank.param_names()))),
            EngineVoice::Layers(..) => None,
        };
        Self { voice, bank_params, legacy_params: Default::default() }
    }
}

enum EngineSynth {
    Bank(Box<Player>),
    Layers { mixer: EngineMixer, params: Arc<EngineParams>, counter: u32, target: (f32, f32) },
}

pub struct EngineSource {
    synth: EngineSynth,
    sample_rate: u32,
}

impl Iterator for EngineSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        Some(match &mut self.synth {
            EngineSynth::Bank(p) => p.next_sample(),
            EngineSynth::Layers { mixer, params, counter, target } => {
                // Read the shared parameters every 64 samples; the mixer smooths them.
                if *counter == 0 {
                    *target = params.get();
                }
                *counter = (*counter + 1) % 64;
                mixer.next_sample(target.0, target.1)
            }
        })
    }
}

impl Source for EngineSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(1).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(self.sample_rate.max(1)).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for EngineAudio {
    type Decoder = EngineSource;
    fn decoder(&self) -> EngineSource {
        match (&self.voice, &self.bank_params) {
            (EngineVoice::Bank(b), Some(params)) => {
                let mut player = Player::new(b.bank.clone(), params.clone());
                // `MW_AUDIO_DRY=1`: bypass the mixer plug-ins, for A/B listening.
                player.dry = std::env::var_os("MW_AUDIO_DRY").is_some();
                EngineSource {
                    synth: EngineSynth::Bank(Box::new(player)),
                    sample_rate: mw_audio::sequence::OUTPUT_RATE,
                }
            }
            _ => {
                let (layers, master) = match &self.voice {
                    EngineVoice::Layers(l, m) => (l.clone(), *m),
                    EngineVoice::Bank(_) => (Vec::new(), 1.0),
                };
                let rate = layers.first().map_or(48_000, |l| l.ginsu.pcm.sample_rate);
                let start = layers.first().map_or(1000.0, |l| l.ginsu.min_rpm);
                EngineSource {
                    synth: EngineSynth::Layers {
                        mixer: EngineMixer::new(layers, master),
                        params: self.legacy_params.clone(),
                        counter: 0,
                        target: (start, 0.0),
                    },
                    sample_rate: rate,
                }
            }
        }
    }
}

/// Marks the entity playing a car's engine.
#[derive(Component)]
pub struct EngineSound {
    pub legacy: Arc<EngineParams>,
    /// Game state → the bank's bindings (cars with `EngineSound2` data).
    pub driver: Option<EngineDriver>,
}

/// Engine sound to attach to the player car once it exists.
#[derive(Resource)]
pub struct PendingEngine(pub Option<EngineAudio>);

fn engine_sound(
    mut commands: Commands,
    pending: Option<ResMut<PendingEngine>>,
    mut assets: ResMut<Assets<EngineAudio>>,
    cars: Query<Entity, With<crate::drive::Car>>,
) {
    let (Some(mut pending), Ok(car)) = (pending, cars.single()) else { return };
    if let Some(audio) = pending.0.take() {
        let driver = match (&audio.voice, &audio.bank_params) {
            (EngineVoice::Bank(b), Some(p)) => Some(EngineDriver::new(b, p.clone())),
            _ => None,
        };
        let legacy = audio.legacy_params.clone();
        let handle = assets.add(audio);
        commands.spawn((
            AudioPlayer::<EngineAudio>(handle),
            // ONCE, not LOOP: the source never ends, and looping makes rodio buffer it 32768 samples (0.74 s)
            // at a time, so the engine held each rpm for most of a second and then jumped.
            PlaybackSettings::ONCE.with_volume(Volume::Linear(0.6)),
            EngineSound { legacy, driver },
            ChildOf(car),
        ));
    }
    commands.remove_resource::<PendingEngine>();
}

fn update_engine_params(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cars: Query<&crate::drive::Car>,
    input: Res<crate::drive::CarInput>,
    mut sounds: Query<&mut EngineSound>,
) {
    let Ok(car) = cars.single() else { return };
    let throttle = if car.gear < 0 { input.brake } else { input.throttle };
    // The physics ticks at 60 Hz and the frames come at their own rate, so a frame sees one tick, then two, ...:
    // the raw rpm advances unevenly per frame and the engine note steps at the frame rate. Interpolate between
    // the last two ticks by how far the clock is into the next one (like the car's transform).
    let rpm = car.rpm_prev + (car.rpm - car.rpm_prev) * fixed.overstep_fraction();
    for mut s in &mut sounds {
        s.legacy.set(rpm, throttle);
        if let Some(d) = &mut s.driver {
            let i = EngineInput {
                rpm,
                throttle,
                gear: car.gear,
                speed_mph: car.speed.abs() / 0.447_04,
                idle_rpm: car.params.idle_rpm,
                redline_rpm: car.params.redline_rpm,
            };
            d.update(time.delta_secs(), &i);
        }
    }
}
// --- Music -------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct MusicState {
    decoding: Option<Arc<Mutex<Option<Pcm>>>>,
    playlist: Vec<std::path::PathBuf>,
    next: usize,
}

#[derive(Component)]
struct Music;

/// Plays the game's songs one after another, decoding each on a worker
/// thread just before it is needed.
fn music(
    mut commands: Commands,
    mut state: ResMut<MusicState>,
    mut assets: ResMut<Assets<PcmAudio>>,
    game: Res<crate::Game>,
    playing: Query<(), With<Music>>,
) {
    if !playing.is_empty() {
        return;
    }
    if state.playlist.is_empty() {
        let mut songs: Vec<_> = game
            .store
            .game()
            .files_with_extension("SPS")
            .into_iter()
            .filter(|p| p.to_string_lossy().to_ascii_uppercase().starts_with("UI/SONGS"))
            .collect();
        if songs.is_empty() {
            return;
        }
        // Shuffle with a time-based seed.
        let mut seed =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        for i in (1..songs.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            songs.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        state.playlist = songs;
    }
    match state.decoding.clone() {
        None => {
            let path = game.store.game().root().join(&state.playlist[state.next % state.playlist.len()]);
            state.next += 1;
            let slot = Arc::new(Mutex::new(None));
            let out = slot.clone();
            std::thread::spawn(move || {
                let pcm = std::fs::read(&path).ok().and_then(|d| mw_audio::sps::decode(&d).ok());
                if pcm.is_none() {
                    warn!("could not decode {}", path.display());
                }
                *out.lock().unwrap() = Some(pcm.unwrap_or_default());
            });
            state.decoding = Some(slot);
        }
        Some(slot) => {
            let Some(pcm) = slot.lock().unwrap().take() else { return };
            state.decoding = None;
            if pcm.samples.is_empty() {
                return;
            }
            info!("music: {:.0} s, {} Hz, {} ch", pcm.duration_secs(), pcm.sample_rate, pcm.channels);
            let handle = assets.add(PcmAudio { pcm: Arc::new(pcm) });
            commands.spawn((
                AudioPlayer::<PcmAudio>(handle),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.35)),
                Music,
            ));
        }
    }
}
