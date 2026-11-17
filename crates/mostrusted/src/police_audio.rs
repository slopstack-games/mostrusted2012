//! The siren voices (docs/decomp.md "Police sirens, light bars and radio"; data `mw_assets::police_sound`).
//!
//! - **Which set**: a car whose `VehicleSound` +0x40 names a `SirenSoundSet` uses it; every other car with a
//!   siren takes `CopSoundSet.9f467b30[slot % 2]` (`FUN_00593ac0`). Our cop bundles' `_COMPETITORSOUND`s have
//!   +0x40 empty for the Interceptor 122713 and the Corvette 122699 (510072 / the SWAT car name `…b3af6`: not read
//!   here yet, they take the list too).
//! - **The voice**: [`mw_assets::police_sound::SirenPlayer`] (tone per level, blips, the 0.2 s crossfade and
//!   0.15 s stop), × 1.6, on the mix snapshot's `Sirens_Competitor` channel (gain 0.501, falloff `…7276`: 4 m to
//!   400 m, focus 0.6, doppler with c = 680 m/s) for other cars and `Player_Vehicle_Sirens` (0.708, c = 340 m/s)
//!   for the player's car, placed and doppler-shifted at the car like the other cars' voices (`car_audio::place`).
//! - **How many**: MODEL: the three nearest sirens sound (the channel's `1653b84c` = 3, read as its voice
//!   limit; the game runs one voice per siren car and its limiter was not read).
//! - **Dying siren**: when a car whose siren is on is wrecked, `SirenSoundSet.89ecdfc4` (`…b3883`) plays once at
//!   the car, one car at a time (`FUN_0058bdc0`, the manager's slot +0x339d0).
//!
//! Not ported: the siren's pitch wobble after impacts (`FUN_00aabc10`: ((0.5 + 0.5 sin φ)·D + 1 − D), φ at
//! 9.253 Hz, D = 0.067 / 0.042 / 0.009 by impact decaying over 0.375 s) and the pitch factor B+0x28 (input open).
//! `MW_AUDIO_METER=1` prints the siren voices once a second; `MW_SIREN_LOG=1` logs starts and the dying siren.

use crate::platform::PollLock;
use std::num::NonZero;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use avian3d::prelude::LinearVelocity;
use bevy::audio::{AddAudioSource, Decodable, Sample, Source, Volume};
use bevy::prelude::*;
use mw_assets::police_sound::{CopSounds, SirenChannel, SirenPlayer, SirenSet, SirenTone};
use mw_audio::sequence::OUTPUT_RATE;
use mw_audio::spatial::{SpatialParams, SpatialRender};

use crate::car_audio::{Listener, place, spawn_one_shot};
use crate::drive::Player;
use crate::police_fx::{FlashCar, siren_log};

pub struct PoliceAudioPlugin;

impl Plugin for PoliceAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<SirenAudio>().init_resource::<Sirens>().add_systems(
            Update,
            (load, voices, meter)
                .chain()
                .after(crate::police_fx::cop_fx_state)
                .after(crate::car_audio::update_listener),
        );
    }
}

/// MODEL: sirens that sound at once (the `Sirens_Competitor` channel's `1653b84c`).
const DEFAULT_VOICES: usize = 3;

type Job = Arc<Mutex<Option<Option<CopSounds>>>>;

#[derive(Resource, Default)]
struct Sirens {
    sounds: Option<Arc<CopSounds>>,
    job: Option<Job>,
    voices: Vec<Voice>,
    /// The dying siren playing (one car at a time).
    dying: Option<(Entity, Arc<mw_audio::sfx_player::Params>, Arc<SpatialParams>, Entity)>,
}

struct Voice {
    car: Entity,
    entity: Entity,
    tone: Arc<AtomicU32>,
    place: Arc<SpatialParams>,
    stats: Arc<Stats>,
    channel: SirenChannel,
    /// Was on last frame (for the dying siren).
    sounding: bool,
    last: (f32, f32, f32, f32),
}

#[derive(Default)]
struct Stats {
    energy: AtomicU64,
    frames: AtomicU64,
}

/// A placed siren voice.
#[derive(Asset, TypePath, Clone)]
pub struct SirenAudio {
    set: Arc<SirenSet>,
    slot: usize,
    tone: Arc<AtomicU32>,
    spatial: Arc<SpatialParams>,
    stats: Arc<Stats>,
}

pub struct SirenSource {
    player: SirenPlayer,
    tone: Arc<AtomicU32>,
    render: SpatialRender,
    right: Option<f32>,
    stats: Arc<Stats>,
    block: (f64, u64),
}

impl Iterator for SirenSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let want = SirenTone::from_code(self.tone.load(Ordering::Relaxed));
        let player = &mut self.player;
        let (l, r) = self.render.frame(&mut || player.next_sample(want));
        self.right = Some(r);
        self.block.0 += (l * l + r * r) as f64;
        self.block.1 += 1;
        if self.block.1 == 1024 {
            let e = f64::from_bits(self.stats.energy.load(Ordering::Relaxed)) + self.block.0;
            self.stats.energy.store(e.to_bits(), Ordering::Relaxed);
            self.stats.frames.fetch_add(self.block.1, Ordering::Relaxed);
            self.block = (0.0, 0);
        }
        Some(l)
    }
}

impl Source for SirenSource {
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

impl Decodable for SirenAudio {
    type Decoder = SirenSource;
    fn decoder(&self) -> SirenSource {
        SirenSource {
            player: SirenPlayer::new(self.set.clone(), self.slot, OUTPUT_RATE),
            tone: self.tone.clone(),
            // Keep the clock running at zero gain: the voice is silent but its tone keeps its place.
            render: SpatialRender::new(self.spatial.clone()),
            right: None,
            stats: self.stats.clone(),
            block: (0.0, 0),
        }
    }
}

fn load(game: Option<Res<crate::Game>>, mut sirens: ResMut<Sirens>) {
    if sirens.sounds.is_some() {
        return;
    }
    let Some(game) = game else { return };
    match &sirens.job {
        None => {
            let job: Job = Arc::new(Mutex::new(None));
            let out = job.clone();
            let store = game.store.clone();
            crate::platform::thread::spawn(move || {
                let s = store.cop_sounds();
                *out.lock().unwrap() = Some(s);
            });
            sirens.job = Some(job);
        }
        Some(job) => {
            let Some(done) = job.poll_lock().and_then(|mut s| s.take()) else { return };
            match done {
                Some(s) => {
                    info!(
                        "sirens: {} + {} siren sets, competitor channel {}",
                        s.sets_a.len(),
                        s.sets_b.len(),
                        s.competitor.is_some()
                    );
                    sirens.sounds = Some(Arc::new(s));
                }
                None => {
                    warn!("sirens: no CopSoundSet; no sirens");
                    sirens.sounds = Some(Arc::new(CopSounds::default()));
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn voices(
    mut commands: Commands,
    mut sirens: ResMut<Sirens>,
    mut assets: ResMut<Assets<SirenAudio>>,
    mut one_shots: ResMut<Assets<crate::car_audio::SpatialAudio>>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    cars: Query<(Entity, &FlashCar, &GlobalTransform, Option<&LinearVelocity>, Has<Player>)>,
) {
    let sirens = &mut *sirens;
    let Some(sounds) = sirens.sounds.clone() else { return };
    if sounds.sets_b.is_empty() || !listener.valid {
        return;
    }
    // The dying siren follows its car until it has played.
    if let Some((entity, params, sp, car)) = &sirens.dying {
        if params.finished() {
            sirens.dying = None;
        } else if let Ok((_, _, t, v, _)) = cars.get(*car)
            && let Some(set) = sounds.sets_b.first()
            && let Some(shot) = &set.dying
        {
            let vel = v.map_or(Vec3::ZERO, |v| v.0);
            let (g, p, pitch, _) = place(&shot.spatial, &listener, t.translation(), vel, t.forward().as_vec3());
            sp.set(g, p, pitch);
        } else {
            commands.entity(*entity).try_despawn();
            sirens.dying = None;
        }
    }
    // Voices of cars that are gone.
    sirens.voices.retain(|v| {
        let alive = cars.get(v.car).is_ok();
        if !alive {
            commands.entity(v.entity).despawn();
        }
        alive
    });
    // The sirens that sound: the nearest few cars whose siren is on (the player's always).
    let mut on: Vec<(f32, Entity)> = cars
        .iter()
        .filter(|(_, f, ..)| f.lights.has_siren && f.tone() != SirenTone::Off)
        .map(|(e, _, t, _, player)| (if player { -1.0 } else { t.translation().distance(listener.position) }, e))
        .collect();
    on.sort_by(|a, b| a.0.total_cmp(&b.0));
    let limit = sounds.competitor.as_ref().map_or(DEFAULT_VOICES, |c| c.voices.max(1) as usize);
    let audible: Vec<Entity> = on.iter().take(limit + 1).map(|x| x.1).collect();
    for (e, f, t, v, player) in &cars {
        if !f.lights.has_siren {
            continue;
        }
        let tone = f.tone();
        let idx = sirens.voices.iter().position(|v| v.car == e);
        let idx = match idx {
            Some(i) => i,
            None if tone != SirenTone::Off => {
                let set = sounds.sets_b[f.slot % sounds.sets_b.len()].clone();
                let channel = match player {
                    true => sounds.player.clone(),
                    false => sounds.competitor.clone(),
                };
                let Some(channel) = channel else { continue };
                let tone_code = Arc::new(AtomicU32::new(tone.code()));
                let spatial = Arc::new(SpatialParams::default());
                let stats = Arc::new(Stats::default());
                let handle = assets.add(SirenAudio {
                    set: set.clone(),
                    slot: f.slot,
                    tone: tone_code.clone(),
                    spatial: spatial.clone(),
                    stats: stats.clone(),
                });
                let entity = commands
                    .spawn((
                        AudioPlayer::<SirenAudio>(handle),
                        PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
                        crate::settings::Channel::Sfx,
                    ))
                    .id();
                if siren_log() {
                    info!("siren voice: car {e:?} slot {} set {:016x} ({tone:?})", f.slot, set.id.0);
                }
                sirens.voices.push(Voice {
                    car: e,
                    entity,
                    tone: tone_code,
                    place: spatial,
                    stats,
                    channel,
                    sounding: false,
                    last: (0.0, 0.0, 1.0, 0.0),
                });
                sirens.voices.len() - 1
            }
            None => continue,
        };
        let voice = &mut sirens.voices[idx];
        voice.tone.store(tone.code(), Ordering::Relaxed);
        let pos = t.translation();
        let vel = v.map_or(Vec3::ZERO, |v| v.0);
        let (gain, pan, pitch, dist) = place(&voice.channel.spatial, &listener, pos, vel, t.forward().as_vec3());
        let gain = if audible.contains(&e) { gain * voice.channel.gain } else { 0.0 };
        voice.place.set(gain, pan, pitch);
        voice.last = (dist, gain, pan, pitch);
        // The dying siren: the car is wrecked while its siren was on.
        let sounding_now = f.siren_level > 0 && !f.disabled;
        if voice.sounding && f.disabled && sirens.dying.is_none() {
            let set = &sounds.sets_b[f.slot % sounds.sets_b.len()];
            if let Some(shot) = &set.dying {
                let params = Arc::new(mw_audio::sfx_player::Params::new(shot.bank.param_names()));
                params.trigger(mw_assets::car_sfx::CUE);
                let vol = settings.as_ref().map_or(0.6, |s| s.gain(crate::settings::Channel::Sfx));
                let (entity, sp, _stats) =
                    spawn_one_shot(&mut commands, &mut one_shots, shot.bank.clone(), params.clone(), vol);
                let (g, p, pitch, _) = place(&shot.spatial, &listener, pos, vel, t.forward().as_vec3());
                sp.set(g, p, pitch);
                if siren_log() {
                    info!("siren: dying siren at car {e:?}");
                }
                sirens.dying = Some((entity, params, sp, e));
            }
        }
        sirens.voices[idx].sounding = sounding_now;
    }
}

/// `MW_AUDIO_METER=1`: the siren voices once a second.
fn meter(time: Res<Time>, mut next: Local<f32>, sirens: Res<Sirens>, cars: Query<&FlashCar>) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MW_AUDIO_METER").is_some());
    if !*ON || time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs().floor() + 1.0;
    for v in &sirens.voices {
        let e = f64::from_bits(v.stats.energy.swap(0f64.to_bits(), Ordering::Relaxed));
        let n = v.stats.frames.swap(0, Ordering::Relaxed);
        let db = if n > 0 && e > 0.0 { 10.0 * (e / (2.0 * n as f64)).log10() } else { -120.0 };
        let tone = cars.get(v.car).map(|f| f.tone()).unwrap_or_default();
        let (dist, gain, pan, pitch) = v.last;
        info!(
            "  siren {:?}: {tone:?} {dist:5.1} m  gain {:6.1} dB  doppler {pitch:.3}  pan {pan:+.2}  out {db:6.1} dBFS",
            v.car,
            20.0 * gain.max(1e-6).log10()
        );
    }
}
