//! One-off sounds of the world and of the player car's landings and crashes (docs/decomp.md "World-component
//! sounds", "Landings", "The crash's contact sounds"; data `mw_assets::world_sfx`).
//!
//! - **Objects** (`WCPlaySoundBehaviour`, every frame from the contacts of each object's body):
//!   - a *prop* (`FUN_0054e8d0`): each body-pair contact runs `FUN_0054bf70`: while the prop's car-hit timer is
//!     0, a contact with a race car closing faster than `392b1040` (2.235 m/s) plays a `Collision_Waves` wave
//!     (emitter type 5); every pair contact sets the timer to 0.6 s. The first contact that damages the prop
//!     (its impact effect) from a car faster than 20 m/s plays the `71f062ee` list (type 2). The world contact
//!     (`FUN_0054c680`), while the world timer is 0 and its value is above `0925bb50` (1 m/s): the `e8986e0a`
//!     list (type 3) when the prop moves faster than 5 m/s, else `Waves` (type 4); the timer is set to 0.35 s;
//!   - a *compound / dynamic object or a compound part* (runtime behaviour vtable 0x00faf308): a pair contact
//!     above 2.235 m/s with a race car plays type 5 (`FUN_005ab070`), the world contact above 1 m/s type 4
//!     (`FUN_005b10f0`), each only while none of the object's sounds plays (`FUN_005b8490` / `FUN_004b26b0`).
//!
//!   The channel is the behaviour's `MixerChannel`, else `World_Prop_Collision` for world contacts and the car's
//!   `…_Prop_Collision` channel for car hits (`FUN_00572430`). Requests go to eight emitters keyed by (type,
//!   behaviour) (`FUN_004b37e0` → `FUN_004ba990` → `FUN_004b2530`): one started less than 0.1 s ago
//!   (`.rdata` 0x00fa339c) takes the new object without a new wave; a full set steals the lowest type below the
//!   new one. The wave is `counter % count` of a global counter (`FUN_004b7b20`), gain 1, at the object's
//!   position plus the behaviour's `Offset`.
//! - **A car's first touch of a prop** (`FUN_00587960` → `FUN_00576440`): a contact with a prop it was not
//!   touching closing faster than 2.235 m/s, at most every 0.1 s, plays the MetalCar set's front / side / rear
//!   pool by the contact's direction on its channel `…7357a`; while the player crashes, once per crash, also
//!   pool `0960076c` on `Player_Wrecking_Collision`.
//! - **Landings** (`FUN_00576b40`, the player): a wheel touching down after more than 0.05 s in the air plays
//!   a `Landing` wave of its surface's set on `Landings_Large` (crashing: pool `5da2551f` on
//!   `Player_Wrecking_Collision`), one wheel per frame, gain `0.35 + 0.65 · min(v, 2.5) / 2.5`; when all four
//!   were in the air, the surface's landing sequence by the time in the air (`TimeInAir`, +0x1ae4): ≥ 1 s
//!   `d12772a1`, above 0.1 s `929af217`.
//! - **Crash contacts** (`FUN_0057caf0`'s crash branch, the player while crashing): a world contact plays a
//!   wave by where it touches the car (`FUN_00929da0`: roof, underside, front, sides, rear) on
//!   `Player_Wrecking_Collision`, when the zone changed or (upright) the contact closes faster than 2.235 m/s;
//!   `player_sfx.rs` skips its world impact meanwhile.
//! - **Collectable messages**: the waves of the free-roam HUD sequences (`freeroam::hud_sequences`: "Billboard
//!   +500", the speed camera's plate, the gate's speedwall panel) at their items' start times.
//!
//! Model decisions (marked where made): the contact value (record +0x2c / point.w, not traced) is the closing
//! speed; the AI's prop channel is `Competitor_Prop_Collision`; the landing gain's speed is the larger of the
//! car's velocity along its right and forward axes (the wheel frame's rows 0 and 2, not identified); the crash
//! zones use the body collider as the car's box.
//!
//! `MW_WORLD_SFX_LOG=1` logs every start (also in muted test runs, which otherwise load nothing);
//! `MW_AUDIO_METER=1` lists the voices once a second.

use crate::platform::PollLock;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::car_sfx::{OneShot, VOICE_GAIN, direction, world_impact_gain};
use mw_assets::props::ObjectKind;
use mw_assets::world_sfx::{
    DAMAGE_HIT_SPEED, LANDING_FULL_SPEED, LANDING_MIN_GAIN, PROP_PAIR_QUIET, PROP_WORLD_QUIET, SLIDE_SPEED, SfxChannel,
    WHEEL_GROUND_HOLD, WaveShots, WcSound, WorldSfx,
};
use mw_bundle::ResourceId;

use crate::car_audio::{Listener, SpatialAudio, place};
use crate::car_sfx::{Follow, Voice, start_with, volume};
use crate::drive::{Car, Player};
use crate::props::{PropBody, PropCarContact};

pub struct WorldSfxPlugin;

impl Plugin for WorldSfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldSfxState>().init_resource::<WorldSfxRequests>().add_systems(
            Update,
            (load, objects, landings, crash_contacts, hud_waves, requested, follow, meter)
                .chain()
                .after(crate::car_audio::update_listener),
        );
    }
}

/// `.rdata` 0x00fa339c: an emitter started less than this long ago takes a new object without a new wave.
const EMITTER_JOIN: f32 = 0.1;
/// `FUN_004b2530`: eight emitters.
const EMITTERS: usize = 8;
/// The global wave counter wraps at 1024 (`FUN_004b7b20`).
const COUNTER_WRAP: u32 = 1024;
/// `VehicleSound.567b6cc3` (+0x110, 0.1 s): between two prop-touch sounds of a car (`+0x1adc`), and between
/// two world contacts the crash branch handles (`+0x1ad8`).
const CONTACT_GAP: f32 = 0.1;
/// `FUN_005767a0`: a car's touch of a prop ends 0.1 s after its last contact (`+0xc7c` against +0x110).
const TOUCH_END: f32 = 0.1;
/// `FUN_0057caf0` crash branch: upside down below this up-axis height, on its side above this |right.y|
/// (`.bss` 0x0110cfe0 = −0.9, 0x0110c900 = 0.9, read in `snap2`).
const UPSIDE_DOWN: f32 = -0.9;
const ON_SIDE: f32 = 0.9;

fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_WORLD_SFX_LOG").is_some())
}

/// Where an object's sound plays (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Chan {
    WorldProp,
    PlayerProp,
    CompetitorProp,
    Mixer(ResourceId),
}

impl Chan {
    fn sfx(self) -> SfxChannel<'static> {
        match self {
            Chan::WorldProp => SfxChannel::Snapshot("World Prop Collision"),
            Chan::PlayerProp => SfxChannel::Snapshot("Player Vehicle Prop Collision"),
            Chan::CompetitorProp => SfxChannel::Snapshot("Competitor Prop Collision"),
            Chan::Mixer(id) => SfxChannel::Mixer(id),
        }
    }
}

type Built = Vec<((u64, Chan), Option<OneShot>)>;

/// A playing emitter (`FUN_004b2530`'s 0x180-byte records).
struct Emitter {
    kind: u8,
    behaviour: ResourceId,
    age: f32,
    objects: Vec<Entity>,
    voice: Voice,
    position: Vec3,
}

/// A one-shot voice at a fixed place (or at the listener: `None`).
struct Placed {
    voice: Voice,
    position: Option<Vec3>,
}

/// A HUD sequence's wave item, built.
struct HudWave {
    start: f32,
    /// The item's `Gain` value.
    gain: f32,
    shots: Vec<Arc<OneShot>>,
}

/// A car's current prop touch (`FUN_005767a0`: entity +0x1a6c, +0xc78 its age, +0xc7c since the last contact).
#[derive(Default, Clone, Copy)]
struct Touch {
    prop: Option<Entity>,
    quiet: f32,
    since_sound: f32,
}

/// The HUD waves being decoded on a worker thread.
type HudJob = Arc<Mutex<Option<HashMap<u32, Vec<HudWave>>>>>;

#[derive(Resource, Default)]
struct WorldSfxState {
    data: Option<Arc<WorldSfx>>,
    job: Option<Arc<Mutex<Option<WorldSfx>>>>,
    /// Object waves by (wave, channel); `None` = does not decode.
    shots: HashMap<(u64, Chan), Option<Arc<OneShot>>>,
    /// Behaviour lists asked for: (behaviour, channel, world lists).
    requested: HashSet<(ResourceId, Chan, bool)>,
    built: Arc<Mutex<Built>>,
    emitters: Vec<Emitter>,
    counter: u32,
    /// Per prop: the car-hit and world timers, and whether its damage list has played.
    props: HashMap<Entity, (f32, f32, bool)>,
    touches: HashMap<Entity, Touch>,
    crash_prop_played: bool,
    /// Landings: per wheel the time since it last touched (as the game's 0.05 s hold), time in the air, the
    /// car's last velocity.
    wheel_hold: [f32; 4],
    air_time: f32,
    last_velocity: Vec3,
    /// Crash branch: the last zone played (+0x18d) and the time since the last world contact handled.
    crash_zone: Option<u8>,
    since_world: f32,
    rr: [usize; 8],
    voices: Vec<Placed>,
    hud: HashMap<u32, Vec<HudWave>>,
    hud_job: Option<HudJob>,
    hud_clock: HashMap<u32, f32>,
    started: HashMap<&'static str, u64>,
}

impl WorldSfxState {
    fn next(&mut self, pool: usize, len: usize) -> usize {
        self.rr[pool] = (self.rr[pool] + 1) % len.max(1);
        self.rr[pool]
    }

    /// Build the waves of a behaviour's lists for a channel on a worker thread (once).
    fn request(
        &mut self,
        store: &Arc<mw_assets::AssetStore>,
        sound: &WcSound,
        waves: &[ResourceId],
        chan: Chan,
        world: bool,
    ) {
        if !self.requested.insert((sound.object, chan, world)) {
            return;
        }
        let waves: Vec<ResourceId> = waves.to_vec();
        let (store, out) = (store.clone(), self.built.clone());
        let label = sound.label.clone();
        crate::platform::thread::spawn(move || {
            let t0 = crate::platform::Instant::now();
            let shots = WaveShots::new(&store);
            let built: Built = waves.iter().map(|w| ((w.0, chan), shots.shot(*w, chan.sfx(), None))).collect();
            out.lock().unwrap().extend(built);
            if log_on() {
                info!("world sfx: built {label:?} on {chan:?} in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);
            }
        });
    }
}

fn load(game: Res<crate::Game>, mut state: ResMut<WorldSfxState>) {
    // Muted runs (tests) load nothing, unless the log asks for the sounds (headless checks).
    if game.mute && !log_on() {
        return;
    }
    let state = &mut *state;
    for (k, shot) in state.built.poll_lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default() {
        state.shots.insert(k, shot.map(Arc::new));
    }
    if state.data.is_none() {
        match state.job.clone() {
            Some(job) => {
                if let Some(data) = job.poll_lock().and_then(|mut s| s.take()) {
                    let sets: Vec<String> = data
                        .sets
                        .iter()
                        .map(|s| {
                            format!(
                                "landing {} crash {}/{}/{}/{}/{} sequences {}/{}",
                                s.landing.len(),
                                s.crash_landing.len(),
                                s.crash_roof.len(),
                                s.crash_front.len(),
                                s.crash_side.len(),
                                s.crash_rear.len(),
                                s.long_landing.is_some(),
                                s.short_landing.is_some()
                            )
                        })
                        .collect();
                    info!(
                        "world sounds: surface sets [{}] (default {:?}), prop hits {:?}, crash prop hit {}, rules {:?}",
                        sets.join("; "),
                        data.default_set,
                        data.prop_hit.iter().map(Vec::len).collect::<Vec<_>>(),
                        data.crash_prop_hit.len(),
                        data.rules
                    );
                    state.data = Some(Arc::new(data));
                    state.job = None;
                }
            }
            None => {
                let store = game.store.clone();
                let job = Arc::new(Mutex::new(None));
                let out = job.clone();
                crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(store.world_sfx()));
                state.job = Some(job);
            }
        }
    }
    if state.hud.is_empty() {
        match state.hud_job.clone() {
            Some(job) => {
                if let Some(hud) = job.poll_lock().and_then(|mut s| s.take()) {
                    state.hud = hud;
                }
            }
            None => {
                let store = game.store.clone();
                let job = Arc::new(Mutex::new(None));
                let out = job.clone();
                crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(hud_sequence_waves(&store)));
                state.hud_job = Some(job);
            }
        }
    }
}

/// The wave items of the collectables' HUD sequences.
fn hud_sequence_waves(store: &mw_assets::AssetStore) -> HashMap<u32, Vec<HudWave>> {
    use mw_assets::sequence::ItemKind;
    let shots = WaveShots::new(store);
    let mut out = HashMap::new();
    let ids = store.scoring().map(|s| crate::freeroam::hud_sequences(&s)).unwrap_or_default();
    for id in ids {
        let Ok(seq) = store.sequence(ResourceId(0x0100_0000_0000_0000 | id as u64)) else { continue };
        let waves: Vec<HudWave> = seq
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Wave(w) => Some(HudWave {
                    start: i.start,
                    gain: w.gain.value,
                    shots: w
                        .waves
                        .iter()
                        .filter_map(|&wave| {
                            let chan = w.mixer_channel.map_or(SfxChannel::Snapshot(""), SfxChannel::Mixer);
                            let mut s = shots.shot(wave, chan, Some(VOICE_GAIN))?;
                            // A 2D sound: no falloff.
                            s.spatial = Default::default();
                            Some(Arc::new(s))
                        })
                        .collect(),
                }),
                _ => None,
            })
            .collect();
        if !waves.is_empty() {
            out.insert(id, waves);
        }
    }
    out
}

/// One contact an object heard about this frame.
#[derive(Clone, Copy)]
struct PairHit {
    car: Option<(Entity, bool)>,
    closing: f32,
    car_speed: f32,
    damaging: bool,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn objects(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<crate::Game>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    collisions: Collisions,
    crash: Option<Res<crate::crash::CrashState>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut state: ResMut<WorldSfxState>,
    mut car_contacts: MessageReader<PropCarContact>,
    mut impacts: MessageReader<crate::particles::ImpactEvent>,
    props: Query<(Entity, &PropBody, &GlobalTransform, Option<&LinearVelocity>)>,
    cars: Query<(&Transform, &LinearVelocity, Has<Player>), With<Car>>,
    bodies: Query<(), Or<(With<PropBody>, With<Car>, With<crate::traffic::TrafficBody>)>>,
) {
    let state = &mut *state;
    let dt = time.delta_secs();
    let Some(data) = state.data.clone() else {
        car_contacts.clear();
        impacts.clear();
        return;
    };
    let vol = volume(&settings);
    let rules = data.rules;
    let crashing = crash.is_some_and(|c| c.active.is_some());
    // The race cars' contacts this frame: loose props (`prop_contacts`) and standing ones (avian).
    let mut pairs: HashMap<Entity, Vec<PairHit>> = HashMap::new();
    let mut touches: HashMap<Entity, (Entity, f32, Vec3)> = HashMap::new();
    let car_of = |e: Entity| cars.get(e).ok().map(|(_, v, player)| (v.0.length(), player));
    for c in car_contacts.read() {
        let (speed, player) = car_of(c.car).unwrap_or((c.car_velocity.length(), false));
        pairs.entry(c.prop).or_default().push(PairHit {
            car: Some((c.car, player)),
            closing: c.closing_speed,
            car_speed: speed,
            damaging: c.damaging,
        });
        let t = touches.entry(c.car).or_insert((c.prop, 0.0, c.point));
        if c.closing_speed > t.1 {
            *t = (c.prop, c.closing_speed, c.point);
        }
    }
    for e in impacts.read() {
        let Some(other) = e.other else { continue };
        if !props.contains(other) {
            continue;
        }
        let (speed, player) = car_of(e.car).unwrap_or((0.0, false));
        pairs.entry(other).or_default().push(PairHit {
            car: Some((e.car, player)),
            closing: e.closing_speed,
            car_speed: speed,
            damaging: false,
        });
        let t = touches.entry(e.car).or_insert((other, 0.0, e.position));
        if e.closing_speed > t.1 {
            *t = (other, e.closing_speed, e.position);
        }
    }

    // A car's first touch of a prop (`FUN_00587960` / `FUN_005767a0` / `FUN_00576440`).
    if !crashing {
        state.crash_prop_played = false;
    }
    let car_entities: Vec<Entity> = touches.keys().copied().chain(state.touches.keys().copied()).collect();
    for car in car_entities {
        let mut t = state.touches.get(&car).copied().unwrap_or_default();
        t.quiet += dt;
        t.since_sound += dt;
        if let Some(&(prop, closing, point)) = touches.get(&car) {
            let new = t.prop != Some(prop);
            if new {
                t.prop = Some(prop);
            }
            t.quiet = 0.0;
            let Ok((ct, _, player)) = cars.get(car) else { continue };
            if new && closing > rules.pair_threshold && t.since_sound > CONTACT_GAP {
                t.since_sound = 0.0;
                let dir = direction((point - ct.translation).normalize_or_zero().dot(ct.rotation * Vec3::Z)) as usize;
                let pool = &data.prop_hit[dir];
                if !pool.is_empty() {
                    let k = state.next(0, pool.len());
                    let shot = pool[k].clone();
                    play(&mut commands, &mut assets, &listener, vol, state, &shot, "car prop hit", point, &[]);
                }
                if crashing && player && !state.crash_prop_played && !data.crash_prop_hit.is_empty() {
                    state.crash_prop_played = true;
                    let k = state.next(1, data.crash_prop_hit.len());
                    let shot = data.crash_prop_hit[k].clone();
                    play(&mut commands, &mut assets, &listener, vol, state, &shot, "crash prop hit", point, &[]);
                }
            }
        }
        if t.quiet >= TOUCH_END {
            t.prop = None;
        }
        state.touches.insert(car, t);
    }
    state.touches.retain(|e, _| cars.contains(*e));

    // The emitters age; finished ones free their objects.
    for em in &mut state.emitters {
        em.age += dt;
    }
    state.emitters.retain(|em| !em.voice.done());
    for t in state.props.values_mut() {
        t.0 = (t.0 - dt).max(0.0);
        t.1 = (t.1 - dt).max(0.0);
    }

    let mut requests: Vec<(u8, Entity, WcSound, Chan, Vec3)> = Vec::new();
    for (e, prop, gt, v) in &props {
        let sounds = &prop.physics.sounds;
        // Objects without a body have no contacts (the game's world-component table holds bodies only).
        if sounds.is_empty() || !prop.active {
            continue;
        }
        let is_prop = prop.physics.kind == ObjectKind::Prop;
        // Ask for the waves once per behaviour (a car hit on either car channel, a world contact).
        for s in sounds {
            let world_chan = s.mixer_channel.map_or(Chan::WorldProp, Chan::Mixer);
            if state.requested.contains(&(s.object, world_chan, true)) {
                continue;
            }
            let car_lists: Vec<ResourceId> = s.collision_waves.iter().chain(&s.damage_waves).copied().collect();
            for chan in match s.mixer_channel {
                Some(m) => vec![Chan::Mixer(m)],
                None => vec![Chan::PlayerProp, Chan::CompetitorProp],
            } {
                state.request(&game.store, s, &car_lists, chan, false);
            }
            let world: Vec<ResourceId> = s.world_waves.iter().chain(&s.slide_waves).copied().collect();
            state.request(&game.store, s, &world, world_chan, true);
        }
        // This frame's contacts: car pairs, other bodies, the world.
        let mut hits: Vec<PairHit> = pairs.remove(&e).unwrap_or_default();
        let mut world: Option<f32> = None;
        for p in collisions.collisions_with(e) {
            if !p.is_touching() || p.manifolds.is_empty() {
                continue;
            }
            let first = p.body1 == Some(e) || p.collider1 == e;
            let other = if first { p.body2.unwrap_or(p.collider2) } else { p.body1.unwrap_or(p.collider1) };
            let closing = p.manifolds.iter().flat_map(|m| &m.points).map(|pt| -pt.normal_speed).fold(0.0f32, f32::max);
            if cars.contains(other) {
                continue; // in `impacts` above
            } else if bodies.contains(other) {
                hits.push(PairHit { car: None, closing, car_speed: 0.0, damaging: false });
            } else {
                world = Some(world.map_or(closing, |w: f32| w.max(closing)));
            }
        }
        if hits.is_empty() && world.is_none() {
            continue;
        }
        let at = gt.translation();
        let playing = state.emitters.iter().any(|em| em.objects.contains(&e));
        let car_chan = |s: &WcSound, h: &PairHit| {
            s.mixer_channel
                .map_or(if h.car.is_some_and(|c| c.1) { Chan::PlayerProp } else { Chan::CompetitorProp }, Chan::Mixer)
        };
        if is_prop {
            let s = &sounds[0];
            let timers = state.props.entry(e).or_insert((0.0, 0.0, false));
            for h in &hits {
                // `FUN_0054bf70`: the first damaging hit, then the car hit while the timer is 0.
                if h.damaging && h.car.is_some() && !timers.2 {
                    timers.2 = true;
                    if h.car_speed > DAMAGE_HIT_SPEED && h.closing > rules.pair_threshold {
                        requests.push((2, e, s.clone(), car_chan(s, h), at));
                    }
                }
                if timers.0 <= 0.0 && h.car.is_some() && h.closing > rules.pair_threshold {
                    requests.push((5, e, s.clone(), car_chan(s, h), at));
                }
                timers.0 = PROP_PAIR_QUIET;
            }
            if let Some(w) = world {
                if timers.1 <= 0.0 && w > rules.world_threshold {
                    let speed = v.map_or(0.0, |v| v.0.length());
                    let kind = if speed > SLIDE_SPEED && !s.slide_waves.is_empty() { 3 } else { 4 };
                    requests.push((kind, e, s.clone(), s.mixer_channel.map_or(Chan::WorldProp, Chan::Mixer), at));
                }
                timers.1 = PROP_WORLD_QUIET;
            }
            continue;
        }
        if playing {
            continue;
        }
        // `FUN_005ab070` / `FUN_005b10f0` for each behaviour.
        for s in sounds.iter().filter(|s| s.on_collision) {
            if let Some(h) = hits.iter().filter(|h| h.car.is_some()).max_by(|a, b| a.closing.total_cmp(&b.closing))
                && h.closing > rules.pair_threshold
            {
                requests.push((5, e, s.clone(), car_chan(s, h), at));
            } else if world.is_some_and(|w| w > rules.world_threshold) {
                requests.push((4, e, s.clone(), s.mixer_channel.map_or(Chan::WorldProp, Chan::Mixer), at));
            }
        }
    }
    state.props.retain(|e, _| props.contains(*e));

    // `FUN_004ba990`: the queued requests go to the emitters.
    for (kind, e, s, chan, at) in requests {
        let position = at + Vec3::from(s.offset);
        if let Some(em) =
            state.emitters.iter_mut().find(|em| em.kind == kind && em.behaviour == s.object && em.age < EMITTER_JOIN)
        {
            if !em.objects.contains(&e) {
                em.objects.push(e);
            }
            em.position = position;
            continue;
        }
        if state.emitters.len() >= EMITTERS {
            let Some((k, low)) =
                state.emitters.iter().enumerate().min_by_key(|(_, em)| em.kind).map(|(k, em)| (k, em.kind))
            else {
                continue;
            };
            if kind <= low {
                continue;
            }
            let old = state.emitters.remove(k);
            old.voice.params.stop(0.0);
        }
        let list = match kind {
            2 => &s.damage_waves,
            3 => &s.slide_waves,
            4 => &s.world_waves,
            _ => &s.collision_waves,
        };
        if list.is_empty() {
            continue;
        }
        let n = state.counter;
        state.counter = (state.counter + 1) % COUNTER_WRAP;
        let wave = list[n as usize % list.len()];
        let Some(Some(shot)) = state.shots.get(&(wave.0, chan)).cloned() else {
            if log_on() {
                let why = if state.shots.contains_key(&(wave.0, chan)) { "does not decode" } else { "not built yet" };
                info!("world sfx: {:?} wave {wave} on {chan:?} skipped: {why}", s.label);
            }
            continue;
        };
        let kind_name: &'static str = match kind {
            2 => "object damage",
            3 => "object slide",
            4 => "object world contact",
            _ => "object car hit",
        };
        *state.started.entry(kind_name).or_default() += 1;
        let voice = start_with(
            &mut commands,
            &mut assets,
            &listener,
            vol,
            &shot,
            kind_name,
            Follow::Entity(e),
            (position, Vec3::ZERO, Vec3::Z),
            &[],
        );
        if log_on() {
            info!(
                "world sfx: {kind_name} {:?} ({}) wave {wave} on {chan:?} at {position:.1}, {:.1} m, gain {:.1} dB",
                s.label,
                s.object,
                voice.last.0,
                20.0 * voice.last.1.max(1e-6).log10()
            );
        }
        state.emitters.push(Emitter { kind, behaviour: s.object, age: 0.0, objects: vec![e], voice, position });
    }
}

/// Start a one-shot at a fixed place.
#[allow(clippy::too_many_arguments)]
fn play(
    commands: &mut Commands,
    assets: &mut Assets<SpatialAudio>,
    listener: &Listener,
    vol: f32,
    state: &mut WorldSfxState,
    shot: &OneShot,
    kind: &'static str,
    at: Vec3,
    set: &[(&str, f32)],
) {
    *state.started.entry(kind).or_default() += 1;
    let voice = start_with(
        commands,
        assets,
        listener,
        vol,
        shot,
        kind,
        Follow::Entity(Entity::PLACEHOLDER),
        (at, Vec3::ZERO, Vec3::Z),
        set,
    );
    if log_on() {
        info!(
            "world sfx: {kind} {:016x} at {at:.1}, {:.1} m, gain {:.1} dB {set:?}",
            shot.id,
            voice.last.0,
            20.0 * voice.last.1.max(1e-6).log10()
        );
    }
    state.voices.push(Placed { voice, position: Some(at) });
}

/// `FUN_00576b40` for the player's car.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn landings(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    crash: Option<Res<crate::crash::CrashState>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut state: ResMut<WorldSfxState>,
    player: Query<(&Car, &Transform, &LinearVelocity), With<Player>>,
) {
    let state = &mut *state;
    let Some(data) = state.data.clone() else { return };
    let Ok((car, t, v)) = player.single() else { return };
    let dt = time.delta_secs();
    let vol = volume(&settings);
    let crashing = crash.is_some_and(|c| c.active.is_some());
    // The wheels' on-ground bytes (+0x24, `FUN_009df790`), not the loaded contacts: a wheel that unloads for a
    // moment over a bump has not left the ground.
    let contacts: Vec<Option<(Vec3, Option<usize>)>> = car.wheels_on_ground().take(4).collect();
    let set_of =
        |c: &(Vec3, Option<usize>)| c.1.and_then(|s| data.surface_set.get(s).copied().flatten()).or(data.default_set);
    let mut held = false;
    let mut landed: Option<usize> = None;
    let mut any_ground = false;
    for (i, c) in contacts.iter().enumerate() {
        held |= state.wheel_hold[i] > 0.0;
        let Some(c) = c else { continue };
        any_ground = true;
        if landed.is_some() || state.wheel_hold[i] > 0.0 {
            continue;
        }
        let set = set_of(c);
        landed = Some(set.unwrap_or(usize::MAX));
        let Some(set) = set.and_then(|k| data.sets.get(k)) else { continue };
        // Gain by the car's last velocity along its right and forward axes (model: the wheel frame's rows 0, 2).
        let lv = state.last_velocity;
        let s = lv.dot(t.rotation * Vec3::X).abs().max(lv.dot(t.rotation * Vec3::Z).abs());
        let a = s.min(LANDING_FULL_SPEED) / LANDING_FULL_SPEED;
        let gain = (1.0 - a) * LANDING_MIN_GAIN + a;
        let (pool, kind, rr) =
            if crashing { (&set.crash_landing, "crash landing", 2) } else { (&set.landing, "wheel landing", 3) };
        if !pool.is_empty() {
            let k = state.next(rr, pool.len());
            let shot = pool[k].clone();
            play(&mut commands, &mut assets, &listener, vol, state, &shot, kind, c.0, &[(VOICE_GAIN, gain)]);
        }
    }
    if log_on() && landed.is_some() {
        info!(
            "world sfx: touchdown, held {held}, {:.2} s in the air, wheel holds {:?}",
            state.air_time, state.wheel_hold
        );
    }
    if !held && let Some(k) = landed {
        let air = state.air_time;
        if let Some(set) = data.sets.get(k) {
            let seq = if air < data.rules.long_air {
                (air > data.rules.short_air).then_some(&set.short_landing).cloned().flatten()
            } else {
                set.long_landing.clone()
            };
            if let Some(seq) = seq {
                let kind = if air < data.rules.long_air { "landing (short)" } else { "landing (long)" };
                play(&mut commands, &mut assets, &listener, vol, state, &seq, kind, t.translation, &[]);
                if log_on() {
                    info!("world sfx: {kind} after {air:.2} s in the air");
                }
            }
        }
    }
    state.air_time = if any_ground { 0.0 } else { state.air_time + dt };
    for (i, c) in contacts.iter().enumerate() {
        state.wheel_hold[i] = if c.is_some() { WHEEL_GROUND_HOLD } else { (state.wheel_hold[i] - dt).max(0.0) };
    }
    state.last_velocity = v.0;
}

/// Where a point touches the car's box (`FUN_00929da0`): 0 roof, 1 underside, 2 front right, 3 front, 4 front
/// left, 5 left, 6 rear left, 7 rear, 8 rear right, 9 right, 10 inside. `p` is in the box's frame divided by its
/// half extents (x right, y up, z forward).
fn box_zone(p: Vec3) -> u8 {
    let m = p.abs().max_element();
    if m <= f32::EPSILON {
        return 10;
    }
    let p = p / m;
    let (x, y, z) = (p.x, p.y, p.z);
    let front = z > 0.7;
    let rear = z < -0.7;
    // The underside edge (`.rdata` 0x010246c0 0.8, 0x010245b0 0.5, `.data` 0x010cf910 2 × 0x010cf914 0.2).
    let bottom_edge = y < -0.8 && (x.abs() - 0.5) * 2.0 * 0.2 + 0.8 < y.abs();
    if x > 0.5 {
        if front {
            2
        } else if rear {
            8
        } else if bottom_edge {
            1
        } else {
            9
        }
    } else if x >= -0.5 {
        if front {
            3
        } else if rear {
            7
        } else if y > 0.8 {
            0
        } else if y < -0.8 {
            1
        } else {
            10
        }
    } else if front {
        4
    } else if rear {
        6
    } else if bottom_edge {
        1
    } else {
        5
    }
}

/// `FUN_0057caf0`'s crash branch for the player.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn crash_contacts(
    mut commands: Commands,
    time: Res<Time>,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    crash: Option<Res<crate::crash::CrashState>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut state: ResMut<WorldSfxState>,
    mut events: MessageReader<crate::particles::ImpactEvent>,
    player: Query<(Entity, &Car, &Transform, &LinearVelocity), With<Player>>,
    shapes: Query<(&Collider, &Transform), Without<Car>>,
    statics: Query<&RigidBody>,
) {
    let state = &mut *state;
    state.since_world += time.delta_secs();
    let crashing = crash.is_some_and(|c| c.active.is_some());
    let (Some(data), Ok((me, car, t, v)), true) = (state.data.clone(), player.single(), crashing) else {
        state.crash_zone = None;
        events.clear();
        return;
    };
    let vol = volume(&settings);
    // The hardest world contact this frame.
    let hit = events
        .read()
        .filter(|e| e.car == me && e.other.is_none_or(|o| statics.get(o).is_ok_and(|rb| rb.is_static())))
        .max_by(|a, b| a.closing_speed.total_cmp(&b.closing_speed))
        .copied();
    let Some(hit) = hit else { return };
    if state.since_world <= CONTACT_GAP {
        return;
    }
    state.since_world = 0.0;
    let Some(set) = data.default_set.and_then(|k| data.sets.get(k)) else { return };
    let up = t.rotation * Vec3::Y;
    let right = t.rotation * Vec3::X;
    let zone = if up.y < UPSIDE_DOWN {
        if state.crash_zone == Some(0) {
            return;
        }
        0
    } else if right.y.abs() > ON_SIDE {
        if state.crash_zone == Some(5) {
            return;
        }
        5
    } else {
        if hit.closing_speed <= data.rules.pair_threshold {
            return;
        }
        let (centre, half) = shapes
            .get(car.body_collider)
            .ok()
            .and_then(|(c, lt)| c.shape_scaled().as_cuboid().map(|b| (lt.translation, b.half_extents)))
            .unwrap_or((Vec3::ZERO, Vec3::new(0.9, 0.6, 2.2)));
        let local = t.rotation.inverse() * (hit.position - t.translation) - centre;
        box_zone(local / half.max(Vec3::splat(1e-3)))
    };
    let (pool, rr) = match zone {
        0 => (&set.crash_roof, 4),
        1 => (&set.crash_landing, 5),
        2..=4 => (&set.crash_front, 6),
        5 | 9 => (&set.crash_side, 7),
        6..=8 => (&set.crash_rear, 7),
        _ => return,
    };
    if pool.is_empty() {
        return;
    }
    let k = state.next(rr, pool.len());
    let shot = pool[k].clone();
    let gain = world_impact_gain(v.0.length());
    play(
        &mut commands,
        &mut assets,
        &listener,
        vol,
        state,
        &shot,
        "crash contact",
        hit.position,
        &[(VOICE_GAIN, gain)],
    );
    if log_on() {
        info!("world sfx: crash zone {zone}, closing {:.1} m/s", hit.closing_speed);
    }
    state.crash_zone = Some(zone);
}

/// The collectables' HUD sequences: their wave items at their start times.
#[allow(clippy::too_many_arguments)]
fn hud_waves(
    mut commands: Commands,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    hud: Option<Res<crate::hud::HudState>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut state: ResMut<WorldSfxState>,
) {
    let state = &mut *state;
    let Some(hud) = hud else { return };
    let vol = volume(&settings);
    let mut seen = HashMap::new();
    for p in &hud.freeroam.playing {
        let before = state.hud_clock.get(&p.sequence).copied().filter(|&c| c <= p.clock).unwrap_or(-1.0);
        seen.insert(p.sequence, p.clock);
        let Some(waves) = state.hud.get(&p.sequence) else { continue };
        let starts: Vec<(Arc<OneShot>, f32)> = waves
            .iter()
            .filter(|w| w.start > before && w.start <= p.clock && !w.shots.is_empty())
            .map(|w| (w.shots[0].clone(), w.gain))
            .collect();
        for (shot, gain) in starts {
            *state.started.entry("hud message").or_default() += 1;
            let voice = start_with(
                &mut commands,
                &mut assets,
                &listener,
                vol,
                &shot,
                "hud message",
                Follow::Entity(Entity::PLACEHOLDER),
                (listener.position, Vec3::ZERO, Vec3::Z),
                &[(VOICE_GAIN, gain)],
            );
            if log_on() {
                info!("world sfx: hud message {} wave {:016x} gain {gain}", p.sequence, shot.id);
            }
            state.voices.push(Placed { voice, position: None });
        }
    }
    state.hud_clock = seen;
}

/// One-shots other modules start at a place (`world_sequences.rs`: the wave items of world objects' sequences):
/// (sound, where, kind for the log).
#[derive(Resource, Default)]
pub struct WorldSfxRequests(pub Vec<(Arc<OneShot>, Vec3, &'static str)>);

fn requested(
    mut commands: Commands,
    listener: Res<Listener>,
    settings: Option<Res<crate::settings::AudioSettings>>,
    mut assets: ResMut<Assets<SpatialAudio>>,
    mut state: ResMut<WorldSfxState>,
    mut requests: ResMut<WorldSfxRequests>,
) {
    let vol = volume(&settings);
    for (shot, at, kind) in std::mem::take(&mut requests.0) {
        play(&mut commands, &mut assets, &listener, vol, &mut state, &shot, kind, at, &[]);
    }
}

/// Every frame: the voices are placed against the listener.
fn follow(listener: Res<Listener>, mut state: ResMut<WorldSfxState>) {
    let state = &mut *state;
    state.voices.retain(|v| !v.voice.done());
    for v in &mut state.voices {
        let at = v.position.unwrap_or(listener.position);
        let (gain, pan, pitch, dist) = place(&v.voice.spatial, &listener, at, Vec3::ZERO, Vec3::Z);
        v.voice.place.set(gain, pan, pitch);
        v.voice.last = (dist, gain, pan, pitch);
    }
    for em in &mut state.emitters {
        let (gain, pan, pitch, dist) = place(&em.voice.spatial, &listener, em.position, Vec3::ZERO, Vec3::Z);
        em.voice.place.set(gain, pan, pitch);
        em.voice.last = (dist, gain, pan, pitch);
    }
}

fn meter(time: Res<Time>, mut next: Local<f32>, state: Res<WorldSfxState>) {
    if std::env::var_os("MW_AUDIO_METER").is_none() {
        return;
    }
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 1.0;
    let lines: Vec<String> = state
        .voices
        .iter()
        .map(|v| &v.voice)
        .chain(state.emitters.iter().map(|e| &e.voice))
        .map(|v| {
            let (e, frames, _) = v.stats.take();
            let ms = e / (2.0 * frames.max(1) as f64);
            format!(
                "  {}: {:5.1} m  gain {:6.1} dB  out {:6.1} dBFS",
                v.kind,
                v.last.0,
                20.0 * v.last.1.max(1e-6).log10(),
                10.0 * (ms + 1e-12).log10()
            )
        })
        .collect();
    let mut started: Vec<_> = state.started.iter().map(|(k, n)| format!("{k} {n}")).collect();
    started.sort();
    info!("world sfx: started [{}]\n{}", started.join(", "), lines.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones() {
        assert_eq!(box_zone(Vec3::new(0.0, 1.0, 0.0)), 0);
        assert_eq!(box_zone(Vec3::new(0.0, -1.0, 0.0)), 1);
        assert_eq!(box_zone(Vec3::new(0.0, 0.0, 1.0)), 3);
        assert_eq!(box_zone(Vec3::new(0.0, 0.0, -1.0)), 7);
        assert_eq!(box_zone(Vec3::new(1.0, 0.0, 0.0)), 9);
        assert_eq!(box_zone(Vec3::new(-1.0, 0.0, 0.0)), 5);
        assert_eq!(box_zone(Vec3::new(1.0, 0.0, 0.9)), 2);
        assert_eq!(box_zone(Vec3::new(-1.0, 0.0, -0.9)), 6);
    }
}
