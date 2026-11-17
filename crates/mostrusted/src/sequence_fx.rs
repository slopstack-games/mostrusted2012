//! The game-world items of the HUD's message sequences (`mw_assets::sequence_fx`): the speed camera's flash
//! (docs/decomp.md "Speed camera flash"). A counted pass (`SpeedCameraTriggered`, `freeroam.rs`) starts sequence
//! 1107082 on the HUD; besides its plate it
//! - switches the behaviours labelled "SpeedCameraLight" within 50 m of the player on for 0.1 s at 0 s and again
//!   at 1 s (`GameLogic.ModifyBehavioursInAreaSequenceItem`: start `FUN_00856370`, end `0x00851e20`, through
//!   game-logic messages 0xd / 0xe to the object, `FUN_00548510` / `FUN_00548560`). The camera's behaviour carries
//!   its flash light (`Light_Point` `…f872d`: white × 500, threshold 0.1, 70.7 m) and its corona (`…f872c`), both
//!   off otherwise (`543527d0` clear);
//! - brightens the picture twice (`PostFxOverrideSequenceItem`, 0–1.54 s, property 0: a curve with spikes of 0.5 at
//!   77 ms and 0.656 at 0.34 s; `FUN_008635a0` → `FUN_0083b7a0` → `FUN_008ecf80`: brightness term = value × 0.6);
//! - plays its two waves at 1.333 s and 5.967 s (`…f690e`, `…f690d`, on mixer channel `…7538`, gain 0.224).
//!
//! MODEL: `EnabledBinding`s are taken as false (with `(NOT)`: true): offline the speed wall has no friends'
//! entries, so `ShowMiniWall` is false; the sequence's timeline controllers (wait for the speed wall's download,
//! time out) are not run: the clock runs straight, as on the HUD. `TransformBinding` is the player's position.
//! The behaviours are matched on objects within `Radius` of that point by their origin (the game's query, flag
//! 0x2000 via `FUN_004f4b00`, takes up to 10 objects; its shape test was not read). Not ported: property 2 (the
//! PostFX state's +0x1504 / +0x1508), the `BusMixerChannelSequenceItem` that ducks the "Dominant 3D" bus to 0.435
//! rising to 1 over 4 s.

use crate::platform::PollLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mw_assets::sequence_fx::{FxItem, FxKind};

use crate::drive::{Car, Player};
use crate::props::PropBody;
use crate::world_coronas::ObjectBehaviours;
use crate::world_lights::LightSources;

pub struct SequenceFxPlugin;

impl Plugin for SequenceFxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SequenceFx>().add_systems(Update, run);
    }
}

/// `FUN_008ecf80`: property 0 → brightness × 0.6 (`.rdata` 0x010219a4); property 1 → contrast × 0.8 above 0
/// (0x010219a0), × 0.4 otherwise (0x00fae200).
const BRIGHTNESS_SCALE: f32 = 0.6;
const CONTRAST_SCALE_UP: f32 = 0.8;
const CONTRAST_SCALE_DOWN: f32 = 0.4;

/// `MW_SEQUENCE_FX_LOG=1`: log item starts and ends.
fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_SEQUENCE_FX_LOG").is_some())
}

type Pcm = Arc<mw_audio::Pcm>;
type Job<T> = Arc<Mutex<Option<T>>>;

#[derive(Resource, Default)]
struct SequenceFx {
    /// Per sequence UniqueId: its world items (empty: none), being read, with decoded waves.
    data: HashMap<u32, Option<Arc<Loaded>>>,
    jobs: HashMap<u32, Job<Loaded>>,
    /// The sequences playing (several of one sequence may run at once: the HUD's message per find).
    runs: Vec<Run>,
    next_run: u64,
    /// Behaviours a run's item switched: (run, item index) → (entity, behaviour index).
    switched: HashMap<(u64, usize), Vec<(Entity, usize)>>,
}

/// One playing HUD sequence, told apart from others of the same sequence by when it started.
struct Run {
    id: u64,
    sequence: u32,
    /// Game time it started (now − its clock).
    start: f64,
    /// The clock seen last frame.
    clock: f32,
    seen: bool,
}

struct Loaded {
    items: Vec<FxItem>,
    /// Decoded waves (gain applied) by item index.
    waves: HashMap<usize, Pcm>,
}

fn enabled(binding: &str) -> bool {
    binding.is_empty() || binding.ends_with("(NOT)")
}

fn load(store: &mw_assets::AssetStore, sequence: u32, mute: bool) -> Loaded {
    let items = store.sequence_fx(sequence);
    let mut waves = HashMap::new();
    if !mute {
        for (i, it) in items.iter().enumerate() {
            let FxKind::Wave { wave, gain } = &it.kind else { continue };
            let Ok((pcm, header, _)) = store.wave_stream(*wave) else { continue };
            let g = gain * header;
            let samples = pcm.samples.iter().map(|&s| (s as f32 * g).clamp(-32768.0, 32767.0) as i16).collect();
            waves.insert(i, Arc::new(mw_audio::Pcm { channels: pcm.channels, sample_rate: pcm.sample_rate, samples }));
        }
    }
    Loaded { items, waves }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    hud: Option<Res<crate::hud::HudState>>,
    time: Res<Time>,
    mut fx: ResMut<SequenceFx>,
    post: Option<ResMut<crate::tonemap::PostView>>,
    lights: Option<ResMut<LightSources>>,
    mut audio: ResMut<Assets<crate::audio::PcmAudio>>,
    player: Query<&GlobalTransform, (With<Car>, With<Player>)>,
    mut objects: Query<(Entity, &GlobalTransform, &PropBody, &mut ObjectBehaviours)>,
) {
    let (Some(game), Some(hud), Some(mut post), Some(mut lights)) = (game, hud, post, lights) else { return };
    let fx = &mut *fx;
    // Sequences that started: read their items (on a worker thread).
    for p in &hud.freeroam.playing {
        if fx.data.contains_key(&p.sequence) || fx.jobs.contains_key(&p.sequence) {
            continue;
        }
        let job: Job<Loaded> = Arc::new(Mutex::new(None));
        let out = job.clone();
        let (store, seq, mute) = (game.store.clone(), p.sequence, game.mute);
        crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(load(&store, seq, mute)));
        fx.jobs.insert(p.sequence, job);
    }
    let finished: Vec<u32> =
        fx.jobs.iter().filter(|(_, j)| j.poll_lock().is_some_and(|s| s.is_some())).map(|(s, _)| *s).collect();
    for s in finished {
        let loaded = fx.jobs.remove(&s).and_then(|j| j.poll_lock().and_then(|mut s| s.take()));
        fx.data.insert(s, loaded.filter(|l| !l.items.is_empty()).map(Arc::new));
    }

    // Match the HUD's playing sequences with the runs (by when they started); runs that stopped put back what
    // they switched first, so that a new run of the same sequence keeps what it switches.
    let now = time.elapsed_secs_f64();
    for r in &mut fx.runs {
        r.seen = false;
    }
    let mut active: Vec<(u64, f32, f32)> = Vec::new();
    for p in &hud.freeroam.playing {
        if !matches!(fx.data.get(&p.sequence), Some(Some(_))) {
            continue;
        }
        let start = now - p.clock as f64;
        let run = fx.runs.iter_mut().find(|r| !r.seen && r.sequence == p.sequence && (r.start - start).abs() < 0.05);
        let (id, t0) = match run {
            Some(r) => {
                r.seen = true;
                let t0 = r.clock.min(p.clock);
                r.clock = p.clock;
                (r.id, t0)
            }
            None => {
                let id = fx.next_run;
                fx.next_run += 1;
                fx.runs.push(Run { id, sequence: p.sequence, start, clock: p.clock, seen: true });
                (id, -1e-3)
            }
        };
        active.push((id, t0, p.clock));
    }
    let stopped: Vec<u64> = fx.runs.iter().filter(|r| !r.seen).map(|r| r.id).collect();
    fx.runs.retain(|r| r.seen);
    for id in stopped {
        end_all(fx, id, &mut objects, &mut lights);
    }

    let mut grade = Vec2::ZERO;
    for (run, t0, t) in active {
        let Some(sequence) = fx.runs.iter().find(|r| r.id == run).map(|r| r.sequence) else { continue };
        let Some(Some(data)) = fx.data.get(&sequence).cloned() else { continue };
        // `MW_SEQUENCE_FX_SHOTS=<dir>`: screenshots at fixed sequence times (checks of the flash).
        if let Some(dir) = std::env::var_os("MW_SEQUENCE_FX_SHOTS") {
            for at in [0.05f32, 0.08, 0.34, 0.6, 1.05, 1.5] {
                if t0 < at && at <= t {
                    let path =
                        std::path::Path::new(&dir).join(format!("seq{}-{:03}.png", sequence, (at * 100.0) as u32));
                    info!("sequence fx: screenshot {}", path.display());
                    commands
                        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                        .observe(bevy::render::view::screenshot::save_to_disk(path));
                }
            }
        }
        for (i, it) in data.items.iter().enumerate() {
            if !enabled(&it.enabled) {
                continue;
            }
            let starts = t0 < it.start && it.start <= t;
            let ends = t0 < it.end && it.end <= t;
            match &it.kind {
                FxKind::PostFx(values) => {
                    let Some(x) = it.phase(t) else { continue };
                    for (property, curve) in values {
                        let v = curve.at(x);
                        match property {
                            0 => grade.x = v * BRIGHTNESS_SCALE,
                            1 => grade.y = v * if v > 0.0 { CONTRAST_SCALE_UP } else { CONTRAST_SCALE_DOWN },
                            _ => {}
                        }
                    }
                }
                FxKind::ModifyBehaviours { label, offset, radius, action, .. } => {
                    if starts {
                        let Some(at) = player.iter().next().map(|t| t.translation() + Vec3::from(*offset)) else {
                            continue;
                        };
                        let mut hit = Vec::new();
                        for (e, t, _, b) in objects.iter() {
                            if t.translation().distance(at) >= *radius {
                                continue;
                            }
                            hit.extend(b.labelled(label).map(|k| (e, k)));
                        }
                        hit.truncate(10);
                        let on = *action != 1;
                        for &(e, k) in &hit {
                            switch(e, k, on, &mut objects, &mut lights);
                        }
                        if log_on() {
                            info!("sequence fx {}: item {i} switches {} \"{label}\" {}", sequence, hit.len(), on);
                        }
                        fx.switched.insert((run, i), hit);
                    }
                    if ends && let Some(hit) = fx.switched.remove(&(run, i)) {
                        for (e, k) in hit {
                            switch(e, k, *action == 1, &mut objects, &mut lights);
                        }
                        if log_on() {
                            info!("sequence fx {}: item {i} ends", sequence);
                        }
                    }
                }
                FxKind::Wave { .. } => {
                    if starts && let Some(pcm) = data.waves.get(&i) {
                        if log_on() {
                            info!("sequence fx {}: wave item {i} at {t:.2} s", sequence);
                        }
                        let handle = audio.add(crate::audio::PcmAudio { pcm: pcm.clone() });
                        commands.spawn((
                            AudioPlayer::<crate::audio::PcmAudio>(handle),
                            PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(0.0)),
                            crate::settings::Channel::Sfx,
                        ));
                    }
                }
            }
        }
    }
    if post.override_grade != grade {
        post.override_grade = grade;
    }
}

#[allow(clippy::type_complexity)]
fn end_all(
    fx: &mut SequenceFx,
    run: u64,
    objects: &mut Query<(Entity, &GlobalTransform, &PropBody, &mut ObjectBehaviours)>,
    lights: &mut LightSources,
) {
    let keys: Vec<(u64, usize)> = fx.switched.keys().copied().filter(|k| k.0 == run).collect();
    for k in keys {
        for (e, b) in fx.switched.remove(&k).unwrap_or_default() {
            let back = objects.get(e).ok().map(|(.., ob)| ob.fx.behaviours[b].2);
            if let Some(back) = back {
                switch(e, b, back, objects, lights);
            }
        }
    }
}

/// Switch behaviour `k` of object `e` on or off ([`ObjectBehaviours::switch`]).
#[allow(clippy::type_complexity)]
fn switch(
    e: Entity,
    k: usize,
    on: bool,
    objects: &mut Query<(Entity, &GlobalTransform, &PropBody, &mut ObjectBehaviours)>,
    lights: &mut LightSources,
) {
    let Ok((_, t, prop, mut b)) = objects.get_mut(e) else { return };
    let radius = prop.physics.radius + 2.0;
    b.switch(k, on, t, radius, lights);
}
