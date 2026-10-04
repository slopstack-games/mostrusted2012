//! Game audio: Ginsu engine sounds driven by the car's RPM, and music from
//! the game's `UI/SONGS` streams. Decoding lives in `mw-audio`.

use std::num::NonZero;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_audio::Pcm;
use mw_audio::ginsu::{EngineMixer, Ginsu};

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

/// Live engine parameters shared with the audio thread.
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

/// A car's engine: Ginsu recordings on throttle (rising sweep) and off
/// throttle (falling sweep), resynthesised at the current RPM.
#[derive(Asset, TypePath, Clone)]
pub struct EngineAudio {
    pub on_load: Arc<Ginsu>,
    pub off_load: Option<Arc<Ginsu>>,
    pub params: Arc<EngineParams>,
}

pub struct EngineSource {
    mixer: EngineMixer,
    params: Arc<EngineParams>,
    counter: u32,
    target: (f32, f32),
}

impl Iterator for EngineSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        // Read the shared parameters every 64 samples; the mixer smooths them.
        if self.counter == 0 {
            self.target = self.params.get();
        }
        self.counter = (self.counter + 1) % 64;
        Some(self.mixer.next_sample(self.target.0, self.target.1))
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
        NonZero::new(self.mixer.sample_rate.max(1)).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for EngineAudio {
    type Decoder = EngineSource;
    fn decoder(&self) -> EngineSource {
        EngineSource {
            mixer: EngineMixer::new(self.on_load.clone(), self.off_load.clone()),
            params: self.params.clone(),
            counter: 0,
            target: (self.on_load.min_rpm, 0.0),
        }
    }
}

/// Marks the entity playing a car's engine.
#[derive(Component)]
pub struct EngineSound {
    pub params: Arc<EngineParams>,
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
        let params = audio.params.clone();
        let handle = assets.add(audio);
        commands.spawn((
            AudioPlayer::<EngineAudio>(handle),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(0.6)),
            EngineSound { params },
            ChildOf(car),
        ));
    }
    commands.remove_resource::<PendingEngine>();
}

fn update_engine_params(
    cars: Query<&crate::drive::Car>,
    input: Res<crate::drive::CarInput>,
    sounds: Query<&EngineSound>,
) {
    let Ok(car) = cars.single() else { return };
    for s in &sounds {
        let load = if car.gear < 0 { input.brake } else { input.throttle };
        s.params.set(car.rpm, load);
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
