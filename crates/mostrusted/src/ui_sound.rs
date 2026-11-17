//! UI sounds the game logic asks for (`Request::PlayUiSound`, logic.rs): the EasyDrive hooks' sounds. The field
//! names a UniqueId of the global `MiscSound.UI` (`AssetStore::ui_sound`, `mw_assets::ui_sounds`); the original
//! maps it through `FUN_007983b0` (UIM+0x32858) and starts that sequence on the sim-timer sequence player
//! UIM+0x47748 (`FUN_00a45070`). Here the sequence (`AssetStore::ui_sequence`) loads on a thread with its waves
//! decoded, then its wave items start as its clock (game time: the sim-timer player) passes their start, on the
//! effects channel (the decomp's UI bus follows the effects volume).
//!
//! Model decisions: `FUN_007983b0`'s lookup is taken as "the sequence with that UniqueId" (its feedback-group
//! slots were not read); each wave item plays its first wave once, at item gain × wave header gain, whole (the
//! item's end and fades are not applied; the eight sequences' items are one-shots of 1–3 s).

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use bevy::audio::Volume;
use bevy::prelude::*;
use mw_assets::sequence::ItemKind;
use mw_audio::Pcm;

/// Under `--mute` the sounds still load and start (the log shows them); the output is silent.
pub struct UiSoundPlugin;

impl Plugin for UiSoundPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel();
        app.insert_resource(UiSounds { queue: Vec::new(), tx, rx: Mutex::new(rx), playing: Vec::new() })
            .add_systems(Update, play);
    }
}

/// A loaded UI sound: its wave items (start, the scaled clip).
struct Loaded {
    field: u32,
    sequence: u32,
    waves: Vec<(f32, Arc<Pcm>)>,
}

/// The UI sounds asked for and playing.
#[derive(Resource)]
pub struct UiSounds {
    queue: Vec<u32>,
    tx: Sender<Result<Loaded, String>>,
    rx: Mutex<Receiver<Result<Loaded, String>>>,
    /// Sequences playing: their waves not started yet and the clock.
    playing: Vec<(Vec<(f32, Arc<Pcm>)>, f32)>,
}

impl UiSounds {
    /// `PlayUiSound { field }`: play the UI sound at `field` of `MiscSound` (module doc).
    pub fn play(&mut self, field: u32) {
        self.queue.push(field);
    }
}

fn load(store: &mw_assets::AssetStore, field: u32) -> Result<Loaded, String> {
    let sequence = store.ui_sound(field).ok_or_else(|| format!("no UI sound at field {field:#x}"))?;
    let seq = store.ui_sequence(sequence).map_err(|e| format!("UI sound sequence {sequence}: {e}"))?;
    let mut waves = Vec::new();
    for item in &seq.items {
        let ItemKind::Wave(w) = &item.kind else { continue };
        let Some(&wave) = w.waves.first() else { continue };
        let Ok((pcm, header, _)) = store.wave_stream(wave) else { continue };
        let g = w.gain.value * header;
        let samples = pcm.samples.iter().map(|&s| (s as f32 * g).clamp(-32768.0, 32767.0) as i16).collect();
        waves.push((item.start, Arc::new(Pcm { channels: pcm.channels, sample_rate: pcm.sample_rate, samples })));
    }
    waves.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(Loaded { field, sequence, waves })
}

fn play(
    mut commands: Commands,
    mut sounds: ResMut<UiSounds>,
    game: Res<crate::Game>,
    time: Res<Time<Virtual>>,
    mut audio: ResMut<Assets<crate::audio::PcmAudio>>,
) {
    let sounds = &mut *sounds;
    for field in std::mem::take(&mut sounds.queue) {
        let (tx, store) = (sounds.tx.clone(), game.store.clone());
        let spawned = crate::platform::thread::Builder::new().name("mw-ui-sound".into()).spawn(move || {
            let _ = tx.send(load(&store, field));
        });
        if let Err(e) = spawned {
            warn!("ui sound: no loader thread: {e}");
        }
    }
    if let Ok(rx) = sounds.rx.lock() {
        while let Ok(got) = rx.try_recv() {
            match got {
                Ok(l) => {
                    info!("ui sound: field {:#x} plays sequence {} ({} waves)", l.field, l.sequence, l.waves.len());
                    sounds.playing.push((l.waves, 0.0));
                }
                Err(e) => warn!("ui sound: {e}"),
            }
        }
    }
    let dt = time.delta_secs();
    for (waves, clock) in &mut sounds.playing {
        while waves.first().is_some_and(|(start, _)| *start <= *clock) {
            let (_, pcm) = waves.remove(0);
            commands.spawn((
                AudioPlayer::<crate::audio::PcmAudio>(audio.add(crate::audio::PcmAudio { pcm })),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.0)),
                crate::settings::Channel::Sfx,
            ));
        }
        *clock += dt;
    }
    sounds.playing.retain(|(waves, _)| !waves.is_empty());
}
