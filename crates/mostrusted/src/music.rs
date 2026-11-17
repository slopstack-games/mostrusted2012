//! Music: the title's sequence waves and the song manager (docs/decomp.md "Music"; data in `mw_assets::music`).
//!
//! **Sequence waves** (`WaveSequenceItem`): a front-end sequence's wave items play while its clock is inside
//! them ([`SequenceWaves`], written by `frontend.rs`): the title sequence 1022357 plays the intro stream
//! 1772189 (28–73.385 s, gain 0.7, 0.385 s fade out), the bridge 1870153 (73–73.9 s) and the loop 1772190
//! (73.385 s on, gain 0.7, a looping stream) while the clock jumps 100.88 → 73.952 s; an item the clock jumps
//! inside of keeps playing.
//!
//! **Song manager** ([`songs::SongManager`]): the licensed songs by situation, see its notes.

use crate::platform::PollLock;
use std::num::NonZero;
use std::sync::{Arc, Mutex};

use bevy::audio::{AddAudioSource, AudioSinkPlayback, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_assets::sequence::WaveItem;
use mw_audio::Pcm;
use mw_bundle::ResourceId;

use crate::settings::{AudioSettings, Channel};

mod songs;

pub struct MusicPlugin {
    pub enabled: bool,
}

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<MusicClip>()
            .insert_resource(MusicEnabled(self.enabled))
            .init_resource::<SequenceWaves>()
            .init_resource::<Decoded>()
            .init_resource::<songs::SongManager>()
            .add_systems(Update, (sequence_waves, songs::song_manager, voices).chain());
    }
}

#[derive(Resource)]
struct MusicEnabled(bool);

/// `MW_MUSIC_LOG=1`: log what starts and stops.
fn log_on() -> bool {
    std::env::var_os("MW_MUSIC_LOG").is_some()
}

// --- Clips ---------------------------------------------------------------------------------------------------

/// A decoded stream, played from `start` (frames), looping or not.
#[derive(Asset, TypePath, Clone)]
pub struct MusicClip {
    pub pcm: Arc<Pcm>,
    pub start: usize,
    pub looping: bool,
}

pub struct MusicSource {
    pcm: Arc<Pcm>,
    pos: usize,
    looping: bool,
}

impl Iterator for MusicSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        if self.pos >= self.pcm.samples.len() {
            if !self.looping || self.pcm.samples.is_empty() {
                return None;
            }
            self.pos = 0;
        }
        let s = self.pcm.samples[self.pos];
        self.pos += 1;
        Some(s as f32 / 32768.0)
    }
}

impl Source for MusicSource {
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
        None
    }
}

impl Decodable for MusicClip {
    type Decoder = MusicSource;
    fn decoder(&self) -> MusicSource {
        let ch = self.pcm.channels.max(1) as usize;
        MusicSource { pcm: self.pcm.clone(), pos: (self.start * ch).min(self.pcm.samples.len()), looping: self.looping }
    }
}

/// One stream's decode: (pcm, header gain, looping); outer `None` while decoding, inner `None` when it failed.
type DecodeSlot = Arc<Mutex<Option<Option<(Arc<Pcm>, f32, bool)>>>>;

/// Decoded `Wave` streams by resource id (decoded on worker threads; `None` while decoding or when it failed).
#[derive(Resource, Default)]
pub struct Decoded {
    slots: std::collections::HashMap<ResourceId, DecodeSlot>,
}

impl Decoded {
    /// The stream if decoded; starts decoding it otherwise. (pcm, header gain, looping).
    pub fn get(&mut self, game: &crate::Game, id: ResourceId) -> Option<(Arc<Pcm>, f32, bool)> {
        let slot = self.slots.entry(id).or_insert_with(|| {
            let slot = Arc::new(Mutex::new(None));
            let out = slot.clone();
            let store = game.store.clone();
            crate::platform::thread::spawn(move || {
                let r = store.wave_stream(id);
                if let Err(e) = &r {
                    warn!("music: wave {id}: {e}");
                }
                *out.lock().unwrap() = Some(r.ok().map(|(p, g, l)| (Arc::new(p), g, l)));
            });
            slot
        });
        slot.poll_lock().and_then(|s| s.clone()).flatten()
    }

    /// Start decoding without waiting.
    pub fn prefetch(&mut self, game: &crate::Game, id: ResourceId) {
        let _ = self.get(game, id);
    }

    /// Forget decoded streams not in `keep` (songs are large).
    pub fn retain(&mut self, keep: &[ResourceId]) {
        self.slots.retain(|id, s| keep.contains(id) || s.poll_lock().is_none_or(|s| s.is_none()));
    }
}

// --- Voices --------------------------------------------------------------------------------------------------

/// A playing stream: its gain (item × wave header), fades, and when it was asked to stop.
#[derive(Component)]
pub struct Voice {
    pub gain: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub age: f32,
    pub stopping: Option<f32>,
    /// A level set by the mixer state (the in-game menu's fade).
    pub duck: f32,
    pub key: VoiceKey,
}

/// Who owns a voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceKey {
    /// A sequence's wave item (its UniqueId).
    Item(u32),
    /// A song (its wave).
    Song(ResourceId),
}

pub fn spawn_voice(
    commands: &mut Commands,
    clips: &mut Assets<MusicClip>,
    clip: MusicClip,
    gain: f32,
    fades: (f32, f32),
    key: VoiceKey,
) -> Entity {
    let handle = clips.add(clip);
    commands
        .spawn((
            AudioPlayer::<MusicClip>(handle),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)),
            Voice { gain, fade_in: fades.0, fade_out: fades.1, age: 0.0, stopping: None, duck: 1.0, key },
        ))
        .id()
}

/// Fades and volumes; voices that faded out are despawned.
fn voices(
    mut commands: Commands,
    time: Res<Time<Real>>,
    settings: Res<AudioSettings>,
    mut q: Query<(Entity, &mut Voice, Option<&mut AudioSink>)>,
) {
    let dt = time.delta_secs().min(0.1);
    let master = settings.gain(Channel::Music);
    for (e, mut v, sink) in &mut q {
        v.age += dt;
        let mut env = if v.fade_in > 0.0 { (v.age / v.fade_in).min(1.0) } else { 1.0 };
        if let Some(s) = v.stopping {
            let s = s + dt;
            v.stopping = Some(s);
            let out = if v.fade_out > 0.0 { 1.0 - s / v.fade_out } else { 0.0 };
            if out <= 0.0 {
                commands.entity(e).despawn();
                continue;
            }
            env *= out;
        }
        if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear(master * v.gain * env * v.duck));
        }
    }
}

// --- Sequence waves ------------------------------------------------------------------------------------------

/// The wave items a front-end sequence has active now: (item UniqueId, item, seconds into the item).
#[derive(Resource, Default)]
pub struct SequenceWaves {
    pub active: Vec<(u32, WaveItem, f32)>,
    /// Waves the screens will play (decoded ahead).
    pub upcoming: Vec<ResourceId>,
}

fn sequence_waves(
    mut commands: Commands,
    enabled: Res<MusicEnabled>,
    waves: Res<SequenceWaves>,
    game: Res<crate::Game>,
    mut decoded: ResMut<Decoded>,
    mut clips: ResMut<Assets<MusicClip>>,
    mut playing: Query<(Entity, &mut Voice)>,
) {
    if !enabled.0 {
        return;
    }
    for &w in &waves.upcoming {
        decoded.prefetch(&game, w);
    }
    // Items that ended fade out.
    for (_, mut v) in &mut playing {
        if let VoiceKey::Item(id) = v.key
            && v.stopping.is_none()
            && !waves.active.iter().any(|(i, ..)| *i == id)
        {
            if log_on() {
                info!("music: sequence wave item {id} stops");
            }
            v.stopping = Some(0.0);
        }
    }
    for (id, item, offset) in &waves.active {
        if playing.iter().any(|(_, v)| v.key == VoiceKey::Item(*id) && v.stopping.is_none()) {
            continue;
        }
        let Some(&wave) = item.waves.first() else { continue };
        let Some((pcm, header_gain, looping)) = decoded.get(&game, wave) else { continue };
        let looping = looping || item.kind == 4;
        let mut start = (offset * pcm.sample_rate as f32) as usize;
        if looping {
            start %= pcm.frames().max(1);
        } else if start >= pcm.frames() {
            continue;
        }
        if log_on() {
            info!("music: sequence wave item {id} plays {wave} from {offset:.2} s (loop {looping})");
        }
        spawn_voice(
            &mut commands,
            &mut clips,
            MusicClip { pcm, start, looping },
            item.gain.value * header_gain,
            (item.fade_in, item.fade_out),
            VoiceKey::Item(*id),
        );
    }
}
