//! Game audio: the car's engine from its sound data (sequence bank, `mw_audio::sequence`). Music is
//! `music.rs`. Decoding and the sound runtime live in `mw-audio` / `mw-assets`.

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

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PcmAudio>()
            .add_audio_source::<EngineAudio>()
            .add_systems(Update, (engine_sound, update_engine_params));
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
                // `MW_AUDIO_DRY=1`: bypass the mixer plug-ins, for A/B listening (`Bypass` for one family).
                player.dry = std::env::var_os("MW_AUDIO_DRY").is_some();
                player.bypass = mw_audio::sequence::Bypass::from_env();
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
    cars: Query<Entity, (With<crate::drive::Car>, With<crate::drive::Player>)>,
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
            PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
            crate::settings::Channel::Engine,
            EngineSound { legacy, driver },
            ChildOf(car),
        ));
    }
    commands.remove_resource::<PendingEngine>();
}

fn update_engine_params(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    cars: Query<
        (&crate::drive::Car, &crate::drive::CarControls, Option<&crate::ignition::EngineStart>),
        With<crate::drive::Player>,
    >,
    mut sounds: Query<&mut EngineSound>,
) {
    let Ok((car, controls, start)) = cars.single() else { return };
    // The car's own control block (the player's input, or the AI's while it drives the player's car in a
    // cutscene), with the pedals swapped in reverse as the physics does.
    let throttle = if car.gear < 0 { controls.brake } else { controls.throttle };
    // The physics ticks at 60 Hz and the frames come at their own rate, so a frame sees one tick, then two, ...:
    // the raw rpm advances unevenly per frame and the engine note steps at the frame rate. Interpolate between
    // the last two ticks by how far the clock is into the next one (like the car's transform).
    let rpm = car.rpm_prev + (car.rpm - car.rpm_prev) * fixed.overstep_fraction();
    let dt = &car.drivetrain;
    let dp = &car.params.drivetrain;
    let drive = mw_assets::engine_driver::DriveState {
        engine_throttle: dt.throttle,
        clutch: dt.clutch,
        gear_change_elapsed: dt.gear_change_elapsed(),
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
    for mut s in &mut sounds {
        s.legacy.set(rpm, throttle);
        if let Some(d) = &mut s.driver {
            if let Some(v) = dp.handling.param("Engine.937338e7") {
                d.set_speed_load(v);
            }
            // The engine's start-up states (`ignition.rs`): `EngineOn` gates the engine buses; the ignition
            // sequence plays on its snapshot channel at the car (`player_sfx.rs`), not in the engine bank.
            if let Some(e) = start {
                d.set_manual_ignition();
                d.set_engine_on(e.on);
            }
            d.set_nitrous(car.drivetrain.nitrous);
            let i = EngineInput {
                rpm,
                throttle,
                gear: car.gear,
                speed_mph: car.speed.abs() / 0.447_04,
                idle_rpm: car.params.idle_rpm,
                redline_rpm: car.params.redline_rpm,
                top_speed_mph: car.params.top_speed / 0.447_04,
                drive: Some(drive),
            };
            d.update(time.delta_secs(), &i);
            sound_trace(time.elapsed_secs(), car.rpm, d.state());
        }
    }
}

/// `MW_SOUND_TRACE=<file>`: one line per frame, `seconds rpm load normalised audible gear_elapsed rpm_offset
/// load_multiplier load_floor limiter` (what the engine sound plays from).
pub fn sound_trace(t: f32, rpm: f32, s: mw_assets::engine_driver::SoundState) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| std::fs::File::create(std::env::var_os("MW_SOUND_TRACE")?).ok().map(Mutex::new));
    if let Some(f) = file {
        _ = writeln!(
            f.lock().unwrap(),
            "{t:.4} {rpm:.1} {:.4} {:.4} {:.1} {:.3} {:.4} {:.4} {:.4} {}",
            s.load,
            s.normalised,
            s.audible,
            s.gear_elapsed.min(99.0),
            s.effects.rpm_offset,
            s.effects.load_multiplier,
            s.effects.load_floor,
            u8::from(s.limiter)
        );
    }
}
