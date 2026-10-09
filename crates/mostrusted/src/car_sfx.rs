//! The other cars' horns, crash and skid sounds (docs/decomp.md "Traffic horns, crashes and skids"; data in
//! `mw_assets::car_sfx`). Every sound is a one-shot voice placed at its car (`car_audio::spawn_one_shot`), which
//! despawns once its timeline has ended.
//!
//! - **Horns** (`FUN_00ebe8c0` → `FUN_005ebb40`, `FUN_006042c0`): every lane follower within 150 m of the player
//!   (`TrafficVehicleConfig.6e54f849`, the radius of the full traffic behaviour) counts a timer that starts at
//!   FLT_MAX; once it is above 55 s (`.rdata` 0x01082f08) and the car is stopping for an obstacle (mode 0), the
//!   timer restarts at 0 and the car wants its horn when its personality is at most `TrafficParamConfig.
//!   831be501` (0.4). Within 60 m of the player (`.data` 0x010c4e04 = 60²) a car that wants its horn and has not
//!   sounded it plays `horns[slot % count]` of its type when one of the two horn slots (`world+0xa2b9c`) is free.
//! - **Traffic crashes** (`FUN_006042c0`, crashed cars within 60 m): the strongest contact with another car that
//!   closes faster than `VehicleSound.392b1040` (2.235 m/s) plays `CollisionSoundSet` slot 12 (the competitor
//!   light front impact) on this car, unless the other car is nearer to the player (the nearer car of a pair
//!   plays it; a contact with the player's car is the player's sound). A world contact closing faster than the
//!   threshold plays the surface's impact wave (`FUN_0057caf0`), its gain by the car's speed.
//! - **AI racers' crashes** (`FUN_00591ac0`, `FUN_0057e920`): race car against race car by the larger impact
//!   value (light, medium from 14, heavy above 30), the direction of the contact (front / side / rear by 30°);
//!   world impacts (also traffic, as the game's dynamic-contact list) play the surface wave above 2.235 m/s and
//!   the collision set's light (above 14) or medium (above 30) slot, at most every 0.1 s.
//! - **Traffic skids** (`FUN_0060d290`): six slots; each frame the traffic car nearest the player (within 60 m)
//!   starts the surface's skid loop when it is physical and skidding (flag 0x1000, `FUN_00606210`: its first
//!   wheel leaves marks and the body moves faster than 5 m/s, `.bss` 0x0110f580 = 25 read in `snap2`); a slot
//!   whose car stops skidding fades out over 0.1 s (`.rdata` 0x00fb1a94).
//!
//! `MW_AUDIO_METER=1` prints the live voices and the counts once a second; `MW_CAR_SFX_LOG=1` logs every start.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use avian3d::prelude::{Collisions, LinearVelocity, RigidBody};
use bevy::prelude::*;
use mw_assets::car_sfx::{CUE, IMPACT_GAIN, Level, OneShot, OtherCarSfx, direction, world_impact_gain};
use mw_audio::spatial::SpatialParams;

use crate::car_audio::{Listener, SpatialAudio, VoiceStats, place, spawn_one_shot};
use crate::drive::{Car, Player};
use crate::traffic::{TrafficCar, TrafficSoundCar, TrafficSystem};

pub struct CarSfxPlugin;

impl Plugin for CarSfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CarSfx>().add_systems(
            Update,
            (load, contacts, traffic_horns, traffic_skids, follow, meter)
                .chain()
                .after(crate::car_audio::update_listener),
        );
    }
}

/// `FUN_00ebeb00`: the traffic behaviour (and so the horn timer) runs for cars within this distance
/// (`TrafficVehicleConfig.6e54f849` = 150 m, `GLOBALCONFIG.BNDL`).
const BEHAVIOUR_RADIUS: f32 = 150.0;
/// `FUN_0060d290` handles traffic car events within 60 m of the player (`.data` 0x010c4e04 = 3600).
const EVENT_RADIUS: f32 = 60.0;
/// The horn timer's period (`.rdata` 0x01082f08, seconds: the timer counts the behaviour's frame time).
const HORN_PERIOD: f32 = 55.0;
/// Horn slots (`world+0xa2b9c`, two words) and traffic skid slots (`world+0xb174c`, six records).
const HORN_SLOTS: usize = 2;
const SKID_SLOTS: usize = 6;
/// A traffic skid ends with this fade (`FUN_00a44e40(voice, 0.1)`, `.rdata` 0x00fb1a94).
const SKID_RELEASE: f32 = 0.1;
/// Flag 0x1000's body speed (`.bss` 0x0110f580 = 25 = 5², read in `snap2`).
const SKID_SPEED: f32 = 5.0;
/// MODEL: impact one-shots alive at once (the game draws them from voice pools of 10 / 32 / 6 / 16 whose use by
/// these sounds was not traced).
const MAX_IMPACTS: usize = 8;

/// What a voice follows.
#[derive(Clone, Copy)]
enum Follow {
    /// A lane follower or crashed car by its sound key.
    Traffic(u32),
    /// A race car (or crashed traffic) by entity.
    Entity(Entity),
}

struct Voice {
    entity: Entity,
    params: Arc<mw_audio::sfx_player::Params>,
    place: Arc<SpatialParams>,
    stats: Arc<VoiceStats>,
    spatial: mw_assets::car_sound::Spatial,
    follow: Follow,
    kind: &'static str,
    /// Released (a skid whose car stopped skidding).
    stopping: bool,
    /// The car's last known state (position, velocity, forward).
    car: (Vec3, Vec3, Vec3),
    /// Distance, gain, pan, doppler last set.
    last: (f32, f32, f32, f32),
}

impl Voice {
    fn done(&self) -> bool {
        self.params.finished()
    }
}

/// A lane follower's horn state (car +0x48 timer, +0x60 bits 0x200 / 0x800).
struct Horn {
    timer: f32,
    wants: bool,
    sounded: bool,
    seen: f32,
}

type Job = Arc<Mutex<Option<(OtherCarSfx, f32)>>>;

#[derive(Resource, Default)]
struct CarSfx {
    data: Option<Arc<OtherCarSfx>>,
    /// `TrafficParamConfig.831be501`.
    horn_chance: f32,
    job: Option<Job>,
    horns: HashMap<u32, Horn>,
    horn_slots: [Option<Voice>; HORN_SLOTS],
    /// Skid slots: the car key and its voice.
    skid_slots: [Option<(u32, Voice)>; SKID_SLOTS],
    impacts: Vec<Voice>,
    /// Last impact sound per car (game time), for the 0.1 s spacing.
    last_impact: HashMap<Entity, f32>,
    /// Crashed traffic cars touching the world this frame.
    grounded: Vec<Entity>,
    started: HashMap<&'static str, u64>,
    /// Impacts not started because [`MAX_IMPACTS`] voices were playing, by kind.
    skipped: HashMap<&'static str, u64>,
}

fn load(game: Res<crate::Game>, mut sfx: ResMut<CarSfx>) {
    if sfx.data.is_some() || game.mute {
        return;
    }
    if let Some(job) = sfx.job.clone() {
        if let Some((data, chance)) = job.lock().unwrap().take() {
            info!(
                "other cars' sounds: {} horns, {} impact slots, world impact {}, traffic skid {}; rules {:?}",
                data.horns.len(),
                data.impacts.iter().flatten().flatten().count(),
                data.world_impact.is_some(),
                data.skid.is_some(),
                data.rules
            );
            sfx.data = Some(Arc::new(data));
            sfx.horn_chance = chance;
            sfx.job = None;
        }
        return;
    }
    let store = game.store.clone();
    let job: Job = Arc::new(Mutex::new(None));
    let out = job.clone();
    std::thread::spawn(move || {
        let traffic = store.traffic_data().ok();
        let components: Vec<u32> = traffic.iter().flat_map(|t| t.vehicles.iter().map(|v| v.component)).collect();
        let chance = traffic.map_or(0.4, |t| t.params.horn_chance);
        *out.lock().unwrap() = Some((store.other_car_sfx(&components), chance));
    });
    sfx.job = Some(job);
}

#[allow(clippy::too_many_arguments)]
fn start(
    commands: &mut Commands,
    assets: &mut Assets<SpatialAudio>,
    listener: &Listener,
    volume: f32,
    shot: &OneShot,
    kind: &'static str,
    follow: Follow,
    car: (Vec3, Vec3, Vec3),
    impact_gain: Option<f32>,
) -> Voice {
    let params = Arc::new(mw_audio::sfx_player::Params::new(shot.bank.param_names()));
    if let Some(g) = impact_gain {
        params.set(IMPACT_GAIN, g);
    }
    params.trigger(CUE);
    let (entity, place_params, stats) = spawn_one_shot(commands, assets, shot.bank.clone(), params.clone(), volume);
    let (gain, pan, pitch, dist) = place(&shot.spatial, listener, car.0, car.1, car.2);
    place_params.set(gain, pan, pitch);
    if std::env::var_os("MW_CAR_SFX_LOG").is_some() {
        info!("car sfx: {kind} {:016x} at {dist:.1} m, gain {:.1} dB", shot.id, 20.0 * gain.max(1e-6).log10());
    }
    Voice {
        entity,
        params,
        place: place_params,
        stats,
        spatial: shot.spatial,
        follow,
        kind,
        stopping: false,
        car,
        last: (dist, gain, pan, pitch),
    }
}

fn volume(settings: &Option<Res<crate::settings::AudioSettings>>) -> f32 {
    settings.as_ref().map_or(1.0, |s| s.gain(crate::settings::Channel::Sfx))
}

/// What a contact's body is.
#[derive(Clone, Copy, PartialEq)]
enum Body {
    World,
    Player(Entity),
    Ai(Entity),
    /// A lane follower (kinematic) or a crashed (dynamic) traffic car.
    Traffic(Entity, bool),
    Other,
}

/// The contacts of the last physics step: crash sounds of crashed traffic and AI racers, and which crashed cars
/// touch the world (their skids).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn contacts(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    collisions: Collisions,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut sfx: ResMut<CarSfx>,
    cars: Query<(&Transform, &LinearVelocity, Has<Player>), With<Car>>,
    traffic: Query<(&Transform, &LinearVelocity, &RigidBody), With<TrafficCar>>,
    bodies: Query<&RigidBody>,
) {
    let sfx = &mut *sfx;
    sfx.grounded.clear();
    let Some(data) = sfx.data.clone() else { return };
    let now = time.elapsed_secs();
    let rules = data.rules;
    let player_pos = cars.iter().find(|c| c.2).map(|c| c.0.translation);
    let kind = |b: Option<Entity>| -> Body {
        let Some(e) = b else { return Body::World };
        if let Ok((_, _, is_player)) = cars.get(e) {
            return if is_player { Body::Player(e) } else { Body::Ai(e) };
        }
        if let Ok((_, _, rb)) = traffic.get(e) {
            return Body::Traffic(e, rb.is_dynamic());
        }
        match bodies.get(e) {
            Ok(rb) if rb.is_static() => Body::World,
            Ok(_) => Body::Other,
            Err(_) => Body::World,
        }
    };
    // The strongest candidate per car and kind: (closing speed, contact point, the other body).
    let mut traffic_car: HashMap<Entity, (f32, Vec3, Body)> = HashMap::new();
    let mut traffic_world: HashMap<Entity, (f32, Vec3)> = HashMap::new();
    let mut ai_car: HashMap<Entity, (f32, Vec3, Entity)> = HashMap::new();
    let mut ai_world: HashMap<Entity, (f32, Vec3)> = HashMap::new();
    for pair in collisions.iter() {
        let (a, b) = (kind(pair.body1), kind(pair.body2));
        let mut closing = 0f32;
        let mut point = None;
        for m in &pair.manifolds {
            for p in &m.points {
                if -p.normal_speed >= closing {
                    closing = -p.normal_speed;
                    point = Some(p.point);
                }
            }
        }
        let Some(point) = point else { continue };
        for (me, other) in [(a, b), (b, a)] {
            match (me, other) {
                (Body::Traffic(e, true), Body::World) => {
                    sfx.grounded.push(e);
                    let c = traffic_world.entry(e).or_insert((0.0, point));
                    if closing > c.0 {
                        *c = (closing, point);
                    }
                }
                (Body::Traffic(e, true), o @ (Body::Traffic(..) | Body::Ai(_) | Body::Player(_))) => {
                    let c = traffic_car.entry(e).or_insert((0.0, point, o));
                    if closing > c.0 {
                        *c = (closing, point, o);
                    }
                }
                (Body::Ai(e), Body::Ai(o)) => {
                    let c = ai_car.entry(e).or_insert((0.0, point, o));
                    if closing > c.0 {
                        *c = (closing, point, o);
                    }
                }
                (Body::Ai(e), Body::World | Body::Traffic(..)) => {
                    let c = ai_world.entry(e).or_insert((0.0, point));
                    if closing > c.0 {
                        *c = (closing, point);
                    }
                }
                _ => {}
            }
        }
    }
    let vol = volume(&settings);
    let state = |e: Entity| -> Option<(Vec3, Vec3, Vec3)> {
        if let Ok((t, v, _)) = cars.get(e) {
            return Some((t.translation, v.0, t.rotation * Vec3::Z));
        }
        traffic.get(e).ok().map(|(t, v, _)| (t.translation, v.0, t.rotation * Vec3::Z))
    };
    let near_player = |p: Vec3| player_pos.is_some_and(|pp| p.distance_squared(pp) <= EVENT_RADIUS * EVENT_RADIUS);
    sfx.impacts.retain(|v| !v.done());
    let mut play = |sfx: &mut CarSfx, shot: &Option<Arc<OneShot>>, kind: &'static str, car: Entity, gain| {
        let Some(shot) = shot else { return };
        let Some(s) = state(car) else { return };
        // MODEL: an impact beyond its channel's falloff (zero gain) is not started (the game starts a virtual
        // voice; a 2.5 s one-shot cannot become audible before it ends unless the listener runs at it).
        if place(&shot.spatial, &listener, s.0, s.1, s.2).0 <= 0.0 {
            *sfx.skipped.entry("inaudible").or_default() += 1;
            return;
        }
        if sfx.impacts.len() >= MAX_IMPACTS {
            *sfx.skipped.entry(kind).or_default() += 1;
            return;
        }
        let v = start(&mut commands, &mut assets, &listener, vol, shot, kind, Follow::Entity(car), s, gain);
        sfx.impacts.push(v);
        *sfx.started.entry(kind).or_default() += 1;
    };
    let spaced = |sfx: &mut CarSfx, car: Entity| -> bool {
        let ok = sfx.last_impact.get(&car).is_none_or(|t| now - t >= rules.cooldown);
        if ok {
            sfx.last_impact.insert(car, now);
        }
        ok
    };
    // Crashed traffic against other cars (`FUN_006042c0`): slot 12 on the nearer car of the pair.
    for (car, (closing, _, other)) in traffic_car {
        let Some((pos, ..)) = state(car) else { continue };
        if closing <= rules.threshold || !near_player(pos) {
            continue;
        }
        let other_pos = match other {
            Body::Player(_) => continue,
            Body::Traffic(o, _) | Body::Ai(o) => state(o).map(|s| s.0),
            _ => None,
        };
        let pp = player_pos.unwrap_or(pos);
        if other_pos.is_some_and(|o| pos.distance_squared(pp) > o.distance_squared(pp)) {
            continue;
        }
        // MODEL: the game's contact records are per impact; ours are per physics step, spaced like world impacts.
        if spaced(sfx, car) {
            play(sfx, &data.impacts[Level::Light as usize][0], "traffic car impact", car, None);
        }
    }
    // Crashed traffic against the world (`FUN_0057caf0` from `FUN_006042c0`): the surface's wave.
    for (car, (closing, _)) in traffic_world {
        let Some((pos, vel, _)) = state(car) else { continue };
        if closing <= rules.threshold || !near_player(pos) || !spaced(sfx, car) {
            continue;
        }
        play(sfx, &data.world_impact, "traffic world impact", car, Some(world_impact_gain(vel.length())));
    }
    // AI racers against each other (`FUN_00591ac0`): the louder side plays the competitor slot.
    for (car, (closing, point, other)) in ai_car {
        let (Some((pos, vel, fwd)), Some((_, ovel, _))) = (state(car), state(other)) else { continue };
        // MODEL: one closing speed for both cars, so the faster car plays the pair's sound (the game compares
        // the two cars' own impact values).
        // MODEL: the game tests only that either value is above 0 (its pair list holds new impacts); ours are
        // every step's contacts, so the 2.235 m/s threshold of the other contacts applies.
        if closing <= rules.threshold || vel.length_squared() < ovel.length_squared() || !spaced(sfx, car) {
            continue;
        }
        let dir = direction((point - pos).normalize_or_zero().dot(fwd));
        let level = rules.car_level(closing);
        play(sfx, &data.impacts[level as usize][dir as usize], "ai car impact", car, None);
    }
    // AI racers against the world and traffic: the surface's wave, then the collision set by level.
    for (car, (closing, point)) in ai_world {
        let Some((pos, vel, fwd)) = state(car) else { continue };
        if closing <= rules.threshold || !spaced(sfx, car) {
            continue;
        }
        play(sfx, &data.world_impact, "ai world impact", car, Some(world_impact_gain(vel.length())));
        if let Some(level) = rules.world_level(closing) {
            let dir = direction((point - pos).normalize_or_zero().dot(fwd));
            play(sfx, &data.impacts[level as usize][dir as usize], "ai world impact (set)", car, None);
        }
    }
    sfx.last_impact.retain(|_, t| now - *t < 5.0);
}

/// Traffic horns: the behaviour's timer and wish (`FUN_00ebe8c0`), then the sound (`FUN_006042c0`).
#[allow(clippy::too_many_arguments)]
fn traffic_horns(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    traffic: Option<Res<TrafficSystem>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut sfx: ResMut<CarSfx>,
    player: Query<&Transform, (With<Car>, With<Player>)>,
) {
    let sfx = &mut *sfx;
    let (Some(data), Some(traffic)) = (sfx.data.clone(), traffic) else { return };
    let Some(centre) = player.iter().next().map(|t| t.translation) else { return };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for slot in sfx.horn_slots.iter_mut() {
        if slot.as_ref().is_some_and(|v| v.done()) {
            *slot = None;
        }
    }
    let vol = volume(&settings);
    let vehicles = traffic.vehicles();
    for car in traffic.sound_cars(centre, BEHAVIOUR_RADIUS).iter().filter(|c| !c.crashed) {
        let h = sfx.horns.entry(car.key).or_insert(Horn { timer: f32::MAX, wants: false, sounded: false, seen: now });
        h.seen = now;
        // `FUN_00ebe8c0`: the timer runs while it is below 55 s or the car is not stopping for an obstacle.
        if h.timer <= HORN_PERIOD || car.mode != 0 {
            h.wants = false;
            h.timer += dt;
        } else {
            // `FUN_005ebb40(personality, TrafficParamConfig +0x20)`.
            h.timer = 0.0;
            h.wants = car.random <= sfx.horn_chance;
        }
        // `FUN_006042c0` (cars within 60 m): bit 0x800 marks the horn as sounded until the wish clears.
        if !h.wants {
            h.sounded = false;
            continue;
        }
        if h.sounded || car.position.distance_squared(centre) > EVENT_RADIUS * EVENT_RADIUS {
            continue;
        }
        let Some(list) = vehicles.get(car.vehicle).and_then(|v| data.horn_lists.get(&v.component)) else { continue };
        if list.is_empty() {
            continue;
        }
        let Some(free) = sfx.horn_slots.iter().position(|s| s.is_none()) else { continue };
        let shot = &data.horns[list[car.slot % list.len()]];
        h.sounded = true;
        let v = start(
            &mut commands,
            &mut assets,
            &listener,
            vol,
            shot,
            "horn",
            Follow::Traffic(car.key),
            (car.position, car.velocity, car.forward),
            None,
        );
        sfx.horn_slots[free] = Some(v);
        *sfx.started.entry("horn").or_default() += 1;
    }
    sfx.horns.retain(|_, h| now - h.seen < 5.0);
}

/// Traffic skids (`FUN_0060d290`, after the per-car events).
#[allow(clippy::too_many_arguments)]
fn traffic_skids(
    mut commands: Commands,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    traffic: Option<Res<TrafficSystem>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut sfx: ResMut<CarSfx>,
    player: Query<&Transform, (With<Car>, With<Player>)>,
) {
    let sfx = &mut *sfx;
    let (Some(data), Some(traffic)) = (sfx.data.clone(), traffic) else { return };
    let Some(shot) = data.skid.clone() else { return };
    let Some(centre) = player.iter().next().map(|t| t.translation) else { return };
    let near = traffic.sound_cars(centre, EVENT_RADIUS);
    // Flag 0x1000: physical, touching the ground, faster than 5 m/s (MODEL: our crashed cars have no wheels, so
    // "its first wheel leaves marks" is read as "touches the world").
    let skidding = |c: &TrafficSoundCar| {
        c.crashed && c.entity.is_some_and(|e| sfx.grounded.contains(&e)) && c.velocity.length() > SKID_SPEED
    };
    for slot in sfx.skid_slots.iter_mut() {
        let Some((key, voice)) = slot else { continue };
        if voice.done() {
            *slot = None;
            continue;
        }
        if !voice.stopping && !near.iter().any(|c| c.key == *key && skidding(c)) {
            voice.params.stop(SKID_RELEASE);
            voice.stopping = true;
        }
    }
    let nearest =
        near.iter().min_by(|a, b| a.position.distance_squared(centre).total_cmp(&b.position.distance_squared(centre)));
    let Some(car) = nearest else { return };
    let playing = sfx.skid_slots.iter().flatten().any(|(k, v)| *k == car.key && !v.stopping);
    if playing || !skidding(car) {
        return;
    }
    let Some(free) = sfx.skid_slots.iter().position(|s| s.is_none()) else { return };
    let v = start(
        &mut commands,
        &mut assets,
        &listener,
        volume(&settings),
        &shot,
        "traffic skid",
        Follow::Traffic(car.key),
        (car.position, car.velocity, car.forward),
        None,
    );
    sfx.skid_slots[free] = Some((car.key, v));
    *sfx.started.entry("traffic skid").or_default() += 1;
}

/// Every frame: each voice follows its car (the last known state once the car is gone).
fn follow(
    listener: Res<Listener>,
    traffic: Option<Res<TrafficSystem>>,
    mut sfx: ResMut<CarSfx>,
    bodies: Query<(&Transform, &LinearVelocity)>,
    player: Query<&Transform, (With<Car>, With<Player>)>,
) {
    let sfx = &mut *sfx;
    let centre = player.iter().next().map(|t| t.translation).unwrap_or(listener.position);
    let near = traffic.map(|t| t.sound_cars(centre, BEHAVIOUR_RADIUS)).unwrap_or_default();
    let voices = sfx
        .horn_slots
        .iter_mut()
        .flatten()
        .chain(sfx.skid_slots.iter_mut().flatten().map(|(_, v)| v))
        .chain(sfx.impacts.iter_mut());
    for v in voices {
        match v.follow {
            Follow::Traffic(key) => {
                if let Some(c) = near.iter().find(|c| c.key == key) {
                    v.car = (c.position, c.velocity, c.forward);
                }
            }
            Follow::Entity(e) => {
                if let Ok((t, vel)) = bodies.get(e) {
                    v.car = (t.translation, vel.0, t.rotation * Vec3::Z);
                }
            }
        }
        let (gain, pan, pitch, dist) = place(&v.spatial, &listener, v.car.0, v.car.1, v.car.2);
        v.place.set(gain, pan, pitch);
        v.last = (dist, gain, pan, pitch);
    }
}

fn meter(time: Res<Time>, mut next: Local<f32>, sfx: Res<CarSfx>) {
    if std::env::var_os("MW_AUDIO_METER").is_none() {
        return;
    }
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let mut lines = Vec::new();
    let voices = sfx
        .horn_slots
        .iter()
        .flatten()
        .chain(sfx.skid_slots.iter().flatten().map(|(_, v)| v))
        .chain(sfx.impacts.iter());
    for v in voices {
        let (e, frames, _) = v.stats.take();
        let ms = e / (2.0 * frames.max(1) as f64);
        lines.push(format!(
            "  {} {:?}: {:5.1} m  gain {:6.1} dB  doppler {:.3}  pan {:+.2}  out {:6.1} dBFS",
            v.kind,
            v.entity,
            v.last.0,
            20.0 * v.last.1.max(1e-6).log10(),
            v.last.3,
            v.last.2,
            10.0 * (ms + 1e-12).log10()
        ));
    }
    let wanting = sfx.horns.values().filter(|h| h.wants || h.sounded).count();
    let armed = sfx.horns.values().filter(|h| h.timer > HORN_PERIOD).count();
    let mut started: Vec<_> = sfx.started.iter().map(|(k, n)| format!("{k} {n}")).collect();
    started.sort();
    info!(
        "car sfx: {} horn timers ({armed} armed, {wanting} sounding), started [{}], skipped {:?}, {} impact voices\n{}",
        sfx.horns.len(),
        started.join(", "),
        sfx.skipped,
        sfx.impacts.len(),
        lines.join("\n")
    );
}
