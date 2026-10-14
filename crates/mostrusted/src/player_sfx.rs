//! The player car's horn, impact, grind and damage sounds (docs/decomp.md "The player's horn, impacts and
//! scrapes"; data in `mw_assets::car_sfx::PlayerCarSfx`). Every sound is a one-shot voice at the player's car
//! (`car_sfx::start_with`), placed by its snapshot channel.
//!
//! - **Horn**: HORN (action 5; W / pad B, `input.rs`) plays the first horn sequence of the car's
//!   `<id>_VEHICLESOUND` while held. MODEL: the runtime that plays the player's horn list was not found (the
//!   `HORN_SOUND` / `HORN_PRESS` names are an event id and an HSM condition); the voice starts on the press and
//!   stops with a 0.1 s release (the game's skid release, `.rdata` 0x00fb1a94) on the release; the 911's horn
//!   item ends after 5 s.
//! - **Impacts** (`FUN_00591ac0` → `FUN_0057caf0` / `FUN_0057e920` with the player's half of the table): a world
//!   contact closing faster than 2.235 m/s plays a wave of the surface's pool on `Player Vehicle World
//!   Collision` (gain by speed; the set's player slot +0xf4 is a race-car effect timeline, no sound), then the
//!   player slot by level (light above 14, medium above 30) and direction, at most every 0.1 s; a car contact
//!   plays the player slot by the pair's level (medium from 14, heavy above 30), a heavy one also a wave of pool
//!   +0x108. The same model decisions as the AI racers' (`car_sfx.rs`): closing speed for the impact value,
//!   contacts of every physics step (`particles::ImpactEvent`) spaced 0.1 s, the default surface.
//! - **Grind** (`FUN_0057f2e0`): while the car touches another vehicle (an AI racer, traffic, a parked car), one
//!   wave of MetalCar pool `97b6a045` on `Player Vehicle Scraping`, the next one started `e40a7d46` (0.278 s)
//!   before a wave ends and the old one faded over what is left; gain `1.09^(−t)` (t since the grind started),
//!   × (0.1 − s) / 0.1 for s seconds after the last contact, then a 0.05 s release; pitch `R^(2x − 1)` with
//!   R = `bf4ff152` (1), x = speed / top speed. A new grind (another vehicle) also plays a `Sparks` wave (pitch
//!   1..1.3) when the last one was more than 0.1 s ago (`FUN_00575750`). MODEL: the game's contact value
//!   (record +0x2c, ≥ 0.05) is read as "the pair pushes" (a positive normal impulse this step).
//! - **Damage** (`FUN_00585030` → `FUN_00576220`, on a frame with a world contact or a contact with a race car):
//!   each cell of the damage grid (`paint_wear::damage_grid`) whose zone level first passes its threshold
//!   (`.bss` 0x0110c990: 0.01 at the four light corners, else 0.15 / 0.25) plays a wave of MetalCar +0x11c on
//!   `Player Vehicle Glass Smash`; with a race car in contact, a cell's scratch level rising by more than
//!   `f5ddbc19` (0.106) in a frame plays a `Scratch` wave, gain `(Δ − 0.106) / (1 − 0.106)`.
//!
//! `MW_AUDIO_METER=1` lists the voices once a second; `MW_CAR_SFX_LOG=1` logs every start; `MW_HORN_AT=t0,t1`
//! holds the horn from t0 to t1 seconds of game time (headless checks).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use avian3d::prelude::{LinearVelocity, RigidBody};
use bevy::prelude::*;
use mw_assets::car_sfx::{
    IMPACT_GAIN, Level, OneShot, PlayerCarSfx, VOICE_GAIN, VOICE_PITCH, direction, world_impact_gain,
};

use crate::car_audio::{Listener, SpatialAudio, place};
use crate::car_sfx::{Follow, Voice, start_with, volume};
use crate::damage::CarDamage;
use crate::drive::{Car, Player};
use crate::traffic::TrafficCar;

pub struct PlayerSfxPlugin;

impl Plugin for PlayerSfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerSfx>().add_systems(
            Update,
            (load, horn, contacts, follow, meter).chain().after(crate::car_audio::update_listener),
        );
    }
}

/// `.bss` 0x0110c990 (read in `snap2`): the damage grid cells' glass thresholds, in the grid's order (back row
/// right to left, …, front row).
const GLASS_THRESHOLDS: [f32; 15] =
    [0.01, 0.15, 0.01, 0.25, 0.15, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.01, 0.25, 0.01];
/// MODEL: the horn's release when the button lets go (the traffic skid's release, `.rdata` 0x00fb1a94).
const HORN_RELEASE: f32 = 0.1;
/// MODEL: one-shot voices of the player's car alive at once (the game's voice pools were not traced).
const MAX_VOICES: usize = 12;

type Job<T> = Arc<Mutex<Option<T>>>;

/// The grind voice (`FUN_0057f2e0`, entity +0x1a58 / +0x1a5c / +0x1a68).
struct Grind {
    other: Entity,
    /// Seconds since the grind started (+0xc30) and since its last contact (+0x1afc).
    age: f32,
    quiet: f32,
    /// The playing wave and its end (in `age` seconds); the wave being faded out.
    voice: Voice,
    wave_end: f32,
    old: Option<Voice>,
}

#[derive(Resource, Default)]
struct PlayerSfx {
    data: Option<Arc<PlayerCarSfx>>,
    job: Option<Job<PlayerCarSfx>>,
    /// The vehicle whose horn is loaded (or loading), and the horn.
    horn_for: Option<u32>,
    horn: Option<Arc<OneShot>>,
    horn_job: Option<Job<Option<OneShot>>>,
    horn_voice: Option<Voice>,
    grind: Option<Grind>,
    voices: Vec<Voice>,
    /// Round-robin indices (the per-pool bytes at `+0x50` of the per-frame block): grind, sparks, glass,
    /// scratch, world wave, heavy wave.
    rr: [usize; 6],
    /// Seconds since the last world impact / car impact sound (`+0x1ad8`) and the last sparks (`+0x1ae0`).
    since_world: f32,
    since_car: f32,
    since_sparks: f32,
    /// The damage grid's last scratch levels and glass flags (entity +0xab0, 16 bytes per cell).
    grid: Option<([f32; 15], [f32; 15])>,
    rng: u32,
    started: HashMap<&'static str, u64>,
}

impl PlayerSfx {
    fn next(&mut self, pool: usize, len: usize) -> usize {
        self.rr[pool] = (self.rr[pool] + 1) % len.max(1);
        self.rr[pool]
    }

    /// A uniform value in [0, 1).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn load(game: Res<crate::Game>, garage: Option<Res<crate::garage::Garage>>, mut sfx: ResMut<PlayerSfx>) {
    if game.mute {
        return;
    }
    let sfx = &mut *sfx;
    if sfx.data.is_none() {
        match sfx.job.clone() {
            Some(job) => {
                if let Some(data) = job.lock().unwrap().take() {
                    info!(
                        "player car sounds: {} impact slots, {} grind / {} sparks / {} glass / {} scratch waves, {} \
                         world waves; {:?}",
                        data.impacts.iter().flatten().flatten().count(),
                        data.grind.len(),
                        data.sparks.len(),
                        data.glass.len(),
                        data.scratch.len(),
                        data.world_impact.len(),
                        data.rules
                    );
                    sfx.data = Some(Arc::new(data));
                    sfx.job = None;
                }
            }
            None => {
                let store = game.store.clone();
                let job: Job<PlayerCarSfx> = Arc::new(Mutex::new(None));
                let out = job.clone();
                std::thread::spawn(move || *out.lock().unwrap() = Some(store.player_car_sfx()));
                sfx.job = Some(job);
                sfx.rng = 0x2f6b_1d35;
            }
        }
    }
    // The horn of the car being driven.
    let vehicle = garage.and_then(|g| g.current.as_ref().and_then(|c| crate::frontend::vehicle_id(&c.bundle)));
    if let Some(job) = sfx.horn_job.clone()
        && let Some(horn) = job.lock().unwrap().take()
    {
        if horn.is_none() {
            warn!("player horn: {:?} has no horn sequence", sfx.horn_for);
        }
        sfx.horn = horn.map(Arc::new);
        sfx.horn_job = None;
    }
    if vehicle.is_some() && vehicle != sfx.horn_for && sfx.horn_job.is_none() {
        sfx.horn_for = vehicle;
        sfx.horn = None;
        let store = game.store.clone();
        let id = vehicle.unwrap_or_default();
        let job: Job<Option<OneShot>> = Arc::new(Mutex::new(None));
        let out = job.clone();
        std::thread::spawn(move || *out.lock().unwrap() = Some(store.player_horn(id)));
        sfx.horn_job = Some(job);
    }
}

/// The car's state for a voice: position, velocity, forward.
fn car_state(t: &Transform, v: &LinearVelocity) -> (Vec3, Vec3, Vec3) {
    (t.translation, v.0, t.rotation * Vec3::Z)
}

#[allow(clippy::too_many_arguments)]
fn horn(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    menu: Option<Res<crate::menu::CarMenu>>,
    pause: Option<Res<crate::pause::PauseMenu>>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut sfx: ResMut<PlayerSfx>,
    player: Query<(Entity, &Transform, &LinearVelocity), (With<Car>, With<Player>)>,
) {
    let sfx = &mut *sfx;
    if sfx.horn_voice.as_ref().is_some_and(|v| v.done()) {
        sfx.horn_voice = None;
    }
    let pad_list: Vec<&Gamepad> = pads.iter().collect();
    let dev = crate::input::Devices { keys: &keys, pads: &pad_list };
    let mut held = !menu.is_some_and(|m| m.open) && !pause.is_some_and(|p| p.open) && dev.held(crate::input::defaults::HORN);
    if let Some((t0, t1)) = scripted_horn() {
        let now = time.elapsed_secs();
        held |= now >= t0 && now < t1;
    }
    let Ok((car, t, v)) = player.single() else { return };
    let sounding = sfx.horn_voice.as_ref().is_some_and(|v| !v.stopping);
    if held && !sounding {
        let Some(shot) = sfx.horn.clone() else { return };
        let vol = volume(&settings);
        let voice = start_with(
            &mut commands,
            &mut assets,
            &listener,
            vol,
            &shot,
            "player horn",
            Follow::Entity(car),
            car_state(t, v),
            &[],
        );
        sfx.horn_voice = Some(voice);
        *sfx.started.entry("player horn").or_default() += 1;
    } else if !held && sounding {
        if let Some(v) = sfx.horn_voice.as_mut() {
            v.params.stop(HORN_RELEASE);
            v.stopping = true;
        }
    }
}

/// `MW_HORN_AT=t0,t1`: hold the horn between these game times.
fn scripted_horn() -> Option<(f32, f32)> {
    static AT: std::sync::OnceLock<Option<(f32, f32)>> = std::sync::OnceLock::new();
    *AT.get_or_init(|| {
        let s = std::env::var("MW_HORN_AT").ok()?;
        let (a, b) = s.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    })
}

/// What the player's car touches.
#[derive(Clone, Copy, PartialEq)]
enum Other {
    World,
    /// A race car (the game's object kind 7).
    Racer(Entity),
    /// Traffic or a parked car (kind 8, the traffic vehicle table).
    Traffic(Entity),
    Ignored,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn contacts(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    mut events: MessageReader<crate::particles::ImpactEvent>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut sfx: ResMut<PlayerSfx>,
    player: Query<
        (Entity, &Transform, &LinearVelocity, &Car, Option<&CarDamage>, Option<&crate::paint_wear::ScratchScale>),
        With<Player>,
    >,
    racers: Query<(), (With<Car>, Without<Player>)>,
    traffic: Query<(), With<TrafficCar>>,
    bodies: Query<&RigidBody>,
) {
    let sfx = &mut *sfx;
    let Some(data) = sfx.data.clone() else { return };
    let Ok((me, t, v, car, damage, scale)) = player.single() else {
        sfx.grind = None;
        events.clear();
        return;
    };
    let dt = time.delta_secs();
    let state = car_state(t, v);
    let vol = volume(&settings);
    let (rules, coll) = (data.rules, data.collision);
    sfx.since_world += dt;
    sfx.since_car += dt;
    sfx.since_sparks += dt;
    sfx.voices.retain(|v| !v.done());
    let classify = |e: Option<Entity>| match e {
        None => Other::World,
        Some(e) if racers.contains(e) => Other::Racer(e),
        Some(e) if traffic.contains(e) => Other::Traffic(e),
        Some(e) => match bodies.get(e) {
            Ok(rb) if rb.is_static() => Other::World,
            Ok(_) => Other::Ignored,
            Err(_) => Other::World,
        },
    };
    // The physics steps' contacts since the last frame (`particles::ImpactEvent`, one per manifold with a
    // positive normal impulse): the hardest world contact, and per vehicle the hardest closing speed and point.
    let mut world: Option<(f32, Vec3)> = None;
    let mut cars: Vec<(Other, f32, Vec3)> = Vec::new();
    for e in events.read().filter(|e| e.car == me) {
        let other = classify(e.other);
        match other {
            Other::Ignored => {}
            Other::World => {
                if world.is_none_or(|(c, _)| e.closing_speed > c) {
                    world = Some((e.closing_speed, e.position));
                }
            }
            _ => match cars.iter_mut().find(|c| c.0 == other) {
                Some(c) if e.closing_speed > c.1 => *c = (other, e.closing_speed, e.position),
                Some(_) => {}
                None => cars.push((other, e.closing_speed, e.position)),
            },
        }
    }
    let world_touch = world.is_some();
    let dir_of = |point: Vec3| direction((point - state.0).normalize_or_zero().dot(state.2));
    let shoot = |commands: &mut Commands,
                 assets: &mut Assets<SpatialAudio>,
                 sfx: &mut PlayerSfx,
                 shot: &OneShot,
                 kind: &'static str,
                 set: &[(&str, f32)]|
     -> Voice {
        *sfx.started.entry(kind).or_default() += 1;
        start_with(commands, assets, &listener, vol, shot, kind, Follow::Entity(me), state, set)
    };
    let play = |commands: &mut Commands,
                assets: &mut Assets<SpatialAudio>,
                sfx: &mut PlayerSfx,
                shot: &OneShot,
                kind: &'static str,
                set: &[(&str, f32)]| {
        if sfx.voices.len() < MAX_VOICES {
            let v = shoot(commands, assets, sfx, shot, kind, set);
            sfx.voices.push(v);
        }
    };
    let (c, a) = (&mut commands, &mut *assets);
    // World impacts (`FUN_0057caf0` with the player's channel set, then `FUN_0057e920`).
    if let Some((closing, point)) = world
        && closing > coll.threshold
        && sfx.since_world >= coll.cooldown
    {
        sfx.since_world = 0.0;
        if !data.world_impact.is_empty() {
            let k = sfx.next(4, data.world_impact.len());
            let g = world_impact_gain(v.0.length());
            play(c, a, sfx, &data.world_impact[k], "player world impact", &[(IMPACT_GAIN, g)]);
        }
        if let Some(level) = coll.world_level(closing)
            && let Some(s) = &data.impacts[level as usize][dir_of(point) as usize]
        {
            play(c, a, sfx, s, "player world impact (set)", &[]);
        }
    }
    // Car impacts: the hardest car contact by level and direction; a heavy one adds a wave of +0x108.
    let hardest = cars.iter().filter(|c| c.1 > coll.threshold).max_by(|a, b| a.1.total_cmp(&b.1)).copied();
    if let Some((_, closing, point)) = hardest
        && sfx.since_car >= coll.cooldown
    {
        sfx.since_car = 0.0;
        let level = coll.car_level(closing);
        if let Some(s) = &data.impacts[level as usize][dir_of(point) as usize] {
            play(c, a, sfx, s, "player car impact", &[]);
        }
        if level == Level::Heavy && !data.heavy.is_empty() {
            let k = sfx.next(5, data.heavy.len());
            play(c, a, sfx, &data.heavy[k], "player car impact (heavy)", &[]);
        }
    }
    // The grind against another vehicle (`FUN_0057f2e0`).
    let touching: Vec<Entity> = cars
        .iter()
        .filter_map(|c| match c.0 {
            Other::Racer(e) | Other::Traffic(e) => Some(e),
            _ => None,
        })
        .collect();
    let x = if car.params.top_speed > 0.0 { (car.speed.abs() / car.params.top_speed).clamp(0.0, 1.0) } else { 0.0 };
    let r = rules.grind_pitch_range.max(1e-3);
    let pitch = r.powf(2.0 * x - 1.0).clamp(1.0 / r, r);
    if let Some(g) = sfx.grind.as_mut() {
        g.age += dt;
        g.quiet += dt;
    }
    let same = sfx.grind.as_ref().is_some_and(|g| touching.contains(&g.other));
    if same {
        let (age, wave_end) = sfx.grind.as_ref().map(|g| (g.age, g.wave_end)).unwrap_or_default();
        if let Some(g) = sfx.grind.as_mut() {
            g.quiet = 0.0;
        }
        // Hand over to the next wave before this one ends; the old one fades over what is left of it.
        if age >= wave_end - rules.grind_crossfade && !data.grind.is_empty() {
            let k = sfx.next(0, data.grind.len());
            let shot = &data.grind[k];
            let gain = rules.grind_decay.powf(-age);
            let nv = shoot(c, a, sfx, shot, "player grind", &[(VOICE_GAIN, gain), (VOICE_PITCH, pitch)]);
            if let Some(g) = sfx.grind.as_mut() {
                if let Some(old) = g.old.take() {
                    old.params.stop(0.0);
                }
                let mut prev = std::mem::replace(&mut g.voice, nv);
                prev.params.stop((wave_end - age).max(0.0));
                prev.stopping = true;
                g.old = Some(prev);
                g.wave_end = age + shot.duration();
            }
        }
    } else if let Some(&other) = touching.first() {
        // Another vehicle: the old grind stops with the 0.05 s release, a new one starts, with sparks.
        if let Some(g) = sfx.grind.take() {
            g.voice.params.stop(rules.release);
            if let Some(o) = g.old {
                o.params.stop(rules.release);
            }
        }
        if !data.grind.is_empty() {
            let k = sfx.next(0, data.grind.len());
            let shot = &data.grind[k];
            let voice = shoot(c, a, sfx, shot, "player grind", &[(VOICE_GAIN, 1.0), (VOICE_PITCH, pitch)]);
            sfx.grind = Some(Grind { other, age: 0.0, quiet: 0.0, voice, wave_end: shot.duration(), old: None });
        }
        if sfx.since_sparks > rules.fade && !data.sparks.is_empty() {
            sfx.since_sparks = 0.0;
            let k = sfx.next(1, data.sparks.len());
            let p = 1.0 + sfx.random() * rules.sparks_pitch;
            play(c, a, sfx, &data.sparks[k], "player sparks", &[(VOICE_PITCH, p)]);
        }
    }
    // The grind's level: 1.09^(−age), faded over 0.1 s after the last contact, then released.
    if let Some(g) = sfx.grind.as_mut() {
        if g.quiet < rules.fade {
            let gain = rules.grind_decay.powf(-g.age) * (rules.fade - g.quiet) / rules.fade;
            for v in std::iter::once(&g.voice).chain(g.old.as_ref()) {
                v.params.set(VOICE_GAIN, gain);
                v.params.set(VOICE_PITCH, pitch);
            }
        } else {
            g.voice.params.stop(rules.release);
            if let Some(o) = &g.old {
                o.params.stop(rules.release);
            }
            sfx.grind = None;
        }
    }
    // Damage sounds (`FUN_00585030` / `FUN_00576220`): the grid's state follows the damage every frame.
    let Some(damage) = damage else { return };
    let raw = crate::paint_wear::damage_grid_raw(damage.zone_levels());
    let scaled = crate::paint_wear::damage_grid(damage.zone_levels(), scale.map_or(10.0, |s| s.0));
    let flags = std::array::from_fn::<f32, 15, _>(|i| if raw[i] > GLASS_THRESHOLDS[i] { 1.0 } else { 0.0 });
    let (prev_scaled, prev_flags) = match sfx.grid {
        // A repaired car (every level back to 0) starts over.
        Some(g) if raw.iter().any(|&x| x > 0.0) => g,
        _ => ([0.0; 15], [0.0; 15]),
    };
    let flags = std::array::from_fn::<f32, 15, _>(|i| flags[i].max(prev_flags[i]));
    let glass = (0..15).map(|i| flags[i] - prev_flags[i]).fold(0.0, f32::max);
    let scratch = (0..15).map(|i| scaled[i] - prev_scaled[i]).fold(0.0, f32::max);
    sfx.grid = Some((scaled, flags));
    let racer = cars.iter().any(|c| matches!(c.0, Other::Racer(_)));
    if !(world_touch || racer) {
        return;
    }
    if glass > rules.glass_threshold && !data.glass.is_empty() {
        let k = sfx.next(2, data.glass.len());
        let g = (glass - rules.glass_threshold) / (1.0 - rules.glass_threshold);
        play(c, a, sfx, &data.glass[k], "player glass", &[(VOICE_GAIN, g)]);
    }
    if racer && scratch > rules.scratch_threshold && !data.scratch.is_empty() {
        let k = sfx.next(3, data.scratch.len());
        let g = (scratch - rules.scratch_threshold) / (1.0 - rules.scratch_threshold);
        play(c, a, sfx, &data.scratch[k], "player scratch", &[(VOICE_GAIN, g)]);
    }
}

/// Every frame: the voices follow the player's car.
fn follow(
    listener: Res<Listener>,
    mut sfx: ResMut<PlayerSfx>,
    player: Query<(&Transform, &LinearVelocity), (With<Car>, With<Player>)>,
) {
    let sfx = &mut *sfx;
    let state = player.single().ok().map(|(t, v)| car_state(t, v));
    let grind = sfx.grind.iter_mut().flat_map(|g| std::iter::once(&mut g.voice).chain(g.old.as_mut()));
    for v in sfx.voices.iter_mut().chain(sfx.horn_voice.as_mut()).chain(grind) {
        if let Some(s) = state {
            v.car = s;
        }
        let (gain, pan, pitch, dist) = place(&v.spatial, &listener, v.car.0, v.car.1, v.car.2);
        v.place.set(gain, pan, pitch);
        v.last = (dist, gain, pan, pitch);
    }
}

fn meter(time: Res<Time>, mut next: Local<f32>, sfx: Res<PlayerSfx>) {
    if std::env::var_os("MW_AUDIO_METER").is_none() {
        return;
    }
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let grind = sfx.grind.iter().flat_map(|g| std::iter::once(&g.voice).chain(g.old.as_ref()));
    let lines: Vec<String> = sfx
        .voices
        .iter()
        .chain(sfx.horn_voice.as_ref())
        .chain(grind)
        .map(|v| {
            let (e, frames, _) = v.stats.take();
            let ms = e / (2.0 * frames.max(1) as f64);
            format!(
                "  {}: {:5.1} m  gain {:6.1} dB  out {:6.1} dBFS{}",
                v.kind,
                v.last.0,
                20.0 * v.last.1.max(1e-6).log10(),
                10.0 * (ms + 1e-12).log10(),
                if v.stopping { " (releasing)" } else { "" }
            )
        })
        .collect();
    let mut started: Vec<_> = sfx.started.iter().map(|(k, n)| format!("{k} {n}")).collect();
    started.sort();
    let grind = sfx.grind.as_ref().map(|g| format!("grind {:.2} s (quiet {:.2} s)", g.age, g.quiet));
    info!("player sfx: started [{}], {}\n{}", started.join(", "), grind.unwrap_or_default(), lines.join("\n"));
}
