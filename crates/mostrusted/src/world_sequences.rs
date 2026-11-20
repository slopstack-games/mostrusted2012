//! Sequences that world objects play (docs/decomp.md "World-object sequences"; data `mw_assets::world_sequence`):
//! the road-work boards' and warning lights' flashing, the harbour crane's and the animated props' loops with
//! their sounds, a hydrant's spray sound when hit, the metal fences' rattle when a car hits them.
//!
//! - **Which objects**: every placed object whose definition carries a `WCSequenceBehaviour` or a
//!   `CollisionSequenceBehaviour`: the props with a body (`PropBody`, which then also follow the body when it is
//!   knocked loose) and the placements without physics (read again from the track unit's layers when it loads).
//! - **When a sequence runs**: a `WCSequenceBehaviour` switched on (`FUN_005b7190`: from the start with
//!   `543527d0`, or by a car's contact with `0228a2f8`, slot 8 `0x005a79e0`) ticks its sequence every frame
//!   (slot 6 `0x005a79f0` → `FUN_00a45220`) with the timeline controllers (`mw_assets::timeline`: the loops are
//!   `JumpTimelineController`s and `JumpNTimesTimelineController`s); nothing switches the ones here off. A
//!   `CollisionSequenceBehaviour` plays `Vehicle_Hit` on a car's contact (`FUN_005b8620`: contact value ≥ 5, the
//!   object rising slower than 6 m/s, 0.6 s since the last one).
//! - **The items** (item loop `FUN_00a42b90`: enter once in [start, end) on the item's clock, stop once outside):
//!   `BehaviourEnabledSequenceItem` switches the object's behaviours of its label on and off again at the stop
//!   (`0x00532180` / `0x00532190`; the gameplay variant switches off only with `856325e8`); `AddBehaviourSequenceItem`
//!   adds an instance of its behaviour for the span (`0x00533880` / `FUN_00532150`), drawn as a second set of
//!   the behaviour's coronas; a `WaveSequenceItem` starts its wave at the object on its `Mixer_Channel` when it
//!   enters (the waves' own players: `mw_audio`; the item's gain, pitch and fades are not applied here);
//!   `AnimationSequenceItem` is entered and stopped but drives nothing: placed world objects have no animated
//!   skeleton in the runtime (the item samples its clip into the behaviour's 60-bone palette in the original,
//!   `0x008620a0`); `CameraShake`, `ApplyVehicleKick`, `VfxSpotEffect`, `WaveSelect` belong to the spike strip's
//!   launch / deploy sequences, which nothing plays (decomp.md "World-object sequences": the player's weapons).
//!
//! Model decisions: a time-driven corona (fewer than all five environment flags) of a behaviour that a sequence
//! switches shows only while the behaviour is on (the flashing lights' sequences only make sense so; the
//! update `FUN_005b31a0` was not followed to its caller); the contact value of `FUN_005b8620` is the car's
//! closing speed (as `world_sfx.rs`); a wave item plays its first wave.
//!
//! `MW_WORLD_SEQ_LOG=1` logs every object picked up, each sequence start, jump and item enter / stop.

use crate::platform::PollLock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::car_sfx::OneShot;
use mw_assets::sequence::{ItemKind, Sequence};
use mw_assets::timeline::SequenceClock;
use mw_assets::world_sequence::{
    COLLISION_MAX_RISE, COLLISION_MIN_CONTACT, COLLISION_REARM, EntityItem, ObjectSequences,
};
use mw_assets::world_sfx::{SfxChannel, WaveShots};
use mw_bundle::ResourceId;

use crate::props::{PropBody, PropCarContact};
use crate::world_coronas::{ObjectBehaviours, ObjectFx};
use crate::world_lights::LightSources;

fn log_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MW_WORLD_SEQ_LOG").is_some())
}

pub struct WorldSequencesPlugin;

impl Plugin for WorldSequencesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldSequenceUnits>()
            .init_resource::<Cache>()
            .add_systems(Update, (collect, attach_props, spawn_placements, contacts, run).chain());
    }
}

/// Track units loaded (`true`) and unloaded since the last frame (`main.rs` feeds it).
#[derive(Resource, Default)]
pub struct WorldSequenceUnits(pub Vec<(u32, bool)>);

/// One `WCSequenceBehaviour`, read.
struct Behaviour {
    data: mw_assets::world_sequence::WcSequence,
    sequence: Arc<Sequence>,
    /// Per item: the wave a `WaveSequenceItem` starts.
    waves: Vec<Option<Arc<OneShot>>>,
}

/// What an object definition plays.
struct ObjectData {
    fx: Arc<ObjectFx>,
    behaviours: Vec<Behaviour>,
    /// `CollisionSequenceBehaviour`s: their `Vehicle_Hit` sequence as one sound.
    vehicle_hits: Vec<Option<Arc<OneShot>>>,
}

type Job<T> = Arc<Mutex<Option<T>>>;

/// A placement without a body: its object, list and transform.
type Placed = (ResourceId, mw_assets::props::ObjectKind, Mat4);

#[derive(Resource, Default)]
struct Cache {
    done: HashMap<ResourceId, Option<Arc<ObjectData>>>,
    jobs: HashMap<ResourceId, Job<Option<Arc<ObjectData>>>>,
    /// Units being read: the placements without physics whose objects play sequences.
    units: Vec<(u32, Job<Vec<Placed>>)>,
    /// Objects whose corona definitions were handed to the corona renderer.
    coronas_sent: std::collections::HashSet<ResourceId>,
    /// The entities spawned for each unit's placements.
    spawned: HashMap<u32, Vec<Entity>>,
}

fn read_object(
    store: &mw_assets::AssetStore,
    object: ResourceId,
    kind: mw_assets::props::ObjectKind,
) -> Option<Arc<ObjectData>> {
    let def = store.object_def(object, kind).ok()?;
    let seqs: ObjectSequences = store.object_sequences(&def);
    if seqs.is_empty() {
        return None;
    }
    let shots = WaveShots::new(store);
    let mut behaviours = Vec::new();
    for b in seqs.sequences {
        let Some(id) = b.sequence else { continue };
        let Ok(sequence) = store.sequence(id) else { continue };
        let waves = sequence
            .items
            .iter()
            .map(|it| match &it.kind {
                ItemKind::Wave(w) => {
                    let (wave, chan) = (w.waves.first()?, w.mixer_channel?);
                    shots.shot(*wave, SfxChannel::Mixer(chan), None).map(Arc::new)
                }
                _ => None,
            })
            .collect();
        behaviours.push(Behaviour { data: b, sequence: Arc::new(sequence), waves });
    }
    let vehicle_hits =
        seqs.collisions.iter().map(|c| c.vehicle_hit.and_then(|s| shots.sequence(s)).map(Arc::new)).collect();
    Some(Arc::new(ObjectData { fx: Arc::new(ObjectFx::read(store, &def)), behaviours, vehicle_hits }))
}

/// A behaviour's running sequence (state 2 of the runtime, +0x6c).
struct Run {
    clock: SequenceClock,
    /// Per item: entered and not yet stopped.
    inside: Vec<bool>,
}

/// An object playing its sequences.
#[derive(Component)]
pub struct WorldSequences {
    object: ResourceId,
    data: Arc<ObjectData>,
    /// Per behaviour: its sequence while switched on.
    runs: Vec<Option<Run>>,
    /// Per collision behaviour: time since its last `Vehicle_Hit` (the runtime's +0x20).
    since_hit: Vec<f32>,
}

impl WorldSequences {
    fn new(object: ResourceId, data: Arc<ObjectData>) -> Self {
        let runs = data.behaviours.iter().map(|_| None).collect();
        let since_hit = vec![COLLISION_REARM; data.vehicle_hits.len()];
        let mut s = Self { object, data, runs, since_hit };
        for i in 0..s.data.behaviours.len() {
            if s.data.behaviours[i].data.at_start {
                s.switch_on(i);
            }
        }
        s
    }

    /// Slot 1 (`FUN_005b7190`): start the sequence unless it runs.
    fn switch_on(&mut self, i: usize) {
        if self.runs[i].is_some() {
            return;
        }
        let b = &self.data.behaviours[i];
        if log_on() {
            info!("world seq {}: {:?} starts sequence {}", self.object, b.data.label, b.sequence.id);
        }
        self.runs[i] = Some(Run {
            clock: SequenceClock::new(&b.sequence.clocks, 0.0),
            inside: vec![false; b.sequence.items.len()],
        });
    }
}

/// Pick up the units loaded and unloaded; read new units' layers on a worker thread.
fn collect(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    mut units: ResMut<WorldSequenceUnits>,
    mut cache: ResMut<Cache>,
) {
    let Some(game) = game else { return };
    for (unit, loaded) in std::mem::take(&mut units.0) {
        if !loaded {
            cache.units.retain(|(u, _)| *u != unit);
            for e in cache.spawned.remove(&unit).unwrap_or_default() {
                commands.entity(e).try_despawn();
            }
            continue;
        }
        let job: Job<Vec<Placed>> = Arc::new(Mutex::new(None));
        let out = job.clone();
        let store = game.store.clone();
        crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(unit_placements(&store, unit)));
        cache.units.push((unit, job));
    }
}

/// The unit's placements without a body (no `PropBody`) whose objects play sequences.
fn unit_placements(store: &mw_assets::AssetStore, unit: u32) -> Vec<Placed> {
    let Some(b) = store.bundle(format!("HAWAII/TRK_UNIT{unit}.BNDL")) else { return Vec::new() };
    let Ok(layers) = store.bundle_layers(&b) else { return Vec::new() };
    let mut known: HashMap<ResourceId, bool> = HashMap::new();
    let mut out = Vec::new();
    for p in layers.props.iter().chain(&layers.compounds).chain(&layers.dynamics) {
        let Some((object, kind)) = p.object else { continue };
        let wanted = *known.entry(object).or_insert_with(|| {
            let Ok(def) = store.object_def(object, kind) else { return false };
            let bodyless =
                store.object_physics(object, kind).ok().is_none_or(|ph| crate::props::PropPhysics::new(&ph).is_none());
            bodyless && !store.object_sequences(&def).is_empty()
        });
        if wanted {
            out.push((object, kind, crate::loader::game_matrix(&p.transform)));
        }
    }
    out
}

/// Start (or take) the worker read of an object definition's sequences.
fn lookup(
    cache: &mut Cache,
    store: &Arc<mw_assets::AssetStore>,
    object: ResourceId,
    kind: mw_assets::props::ObjectKind,
) -> Option<Option<Arc<ObjectData>>> {
    if let Some(d) = cache.done.get(&object) {
        return Some(d.clone());
    }
    if let Some(job) = cache.jobs.get(&object) {
        let d = job.poll_lock().and_then(|mut s| s.take())?;
        cache.jobs.remove(&object);
        cache.done.insert(object, d.clone());
        return Some(d);
    }
    let job: Job<Option<Arc<ObjectData>>> = Arc::new(Mutex::new(None));
    let out = job.clone();
    let store = store.clone();
    crate::platform::thread::spawn(move || *out.lock().unwrap() = Some(read_object(&store, object, kind)));
    cache.jobs.insert(object, job);
    None
}

/// Objects with no sequences (checked once).
#[derive(Component)]
struct NoSequences;

/// Props with a body: their sequences ride on the prop entity.
#[allow(clippy::type_complexity)]
fn attach_props(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    mut cache: ResMut<Cache>,
    props: Query<
        (Entity, &PropBody),
        (Without<WorldSequences>, Without<NoSequences>, Without<crate::props::CompoundPiece>),
    >,
) {
    let Some(game) = game else { return };
    for (e, prop) in &props {
        let (object, kind) = (prop.physics.object, prop.physics.kind);
        match lookup(&mut cache, &game.store, object, kind) {
            Some(Some(data)) => {
                if log_on() {
                    info!("world seq {object}: prop at {:.1}", prop.home.translation);
                }
                commands.entity(e).insert(WorldSequences::new(object, data));
            }
            Some(None) => {
                commands.entity(e).insert(NoSequences);
            }
            None => {}
        }
    }
}

/// Placements without a body: an entity of their own each, with the object's behaviours.
fn spawn_placements(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    mut cache: ResMut<Cache>,
    mut coronas: Option<ResMut<crate::coronas::WorldCoronas>>,
) {
    let Some(game) = game else { return };
    let cache = &mut *cache;
    let mut i = 0;
    while i < cache.units.len() {
        let ready = cache.units[i].1.poll_lock().is_some_and(|s| s.is_some());
        if !ready {
            i += 1;
            continue;
        }
        let list = cache.units[i].1.poll_lock().and_then(|mut s| s.take()).unwrap_or_default();
        let unit = cache.units[i].0;
        // Wait until every object's data is read, then spawn them all.
        let mut all = Vec::with_capacity(list.len());
        for &(object, kind, m) in &list {
            match lookup(cache, &game.store, object, kind) {
                Some(d) => all.push((object, m, d)),
                None => break,
            }
        }
        if all.len() < list.len() {
            *cache.units[i].1.lock().unwrap() = Some(list);
            i += 1;
            continue;
        }
        cache.units.remove(i);
        for (object, m, data) in all {
            let Some(data) = data else { continue };
            let t = Transform::from_matrix(m);
            if log_on() {
                info!("world seq {object}: placement at {:.1} (TRK_UNIT{unit})", t.translation);
            }
            let fx = data.fx.clone();
            // The corona renderer learns the object's corona definitions (as `world_coronas::attach` does).
            if let Some(w) = coronas.as_mut()
                && cache.coronas_sent.insert(object)
            {
                for c in &fx.coronas {
                    w.new_defs.extend(c.entries.iter().map(|(d, _)| d.clone()));
                }
            }
            let e = commands.spawn((t, WorldSequences::new(object, data), ObjectBehaviours::new(object, fx))).id();
            cache.spawned.entry(unit).or_default().push(e);
        }
    }
}

/// Cars' contacts: `0228a2f8` sequences switch on, `CollisionSequenceBehaviour`s play `Vehicle_Hit`.
fn contacts(
    mut events: MessageReader<PropCarContact>,
    mut objects: Query<(&mut WorldSequences, &GlobalTransform, Option<&LinearVelocity>)>,
    mut sfx: Option<ResMut<crate::world_sfx::WorldSfxRequests>>,
) {
    for c in events.read() {
        let Ok((mut s, t, v)) = objects.get_mut(c.prop) else { continue };
        for i in 0..s.data.behaviours.len() {
            if s.data.behaviours[i].data.on_by_contact {
                s.switch_on(i);
            }
        }
        let rise = v.map_or(0.0, |v| v.0.y);
        for k in 0..s.data.vehicle_hits.len() {
            if rise < COLLISION_MAX_RISE
                && c.closing_speed >= COLLISION_MIN_CONTACT
                && s.since_hit[k] >= COLLISION_REARM
            {
                s.since_hit[k] = 0.0;
                if let Some(shot) = s.data.vehicle_hits[k].clone() {
                    if log_on() {
                        info!("world seq {}: vehicle hit at {:.1} m/s", s.object, c.closing_speed);
                    }
                    if let Some(sfx) = sfx.as_mut() {
                        sfx.0.push((shot, t.translation(), "object vehicle hit"));
                    }
                }
            }
        }
    }
}

/// Tick every running sequence and carry out its items.
fn run(
    time: Res<Time>,
    mut objects: Query<(&mut WorldSequences, &GlobalTransform, Option<&mut ObjectBehaviours>)>,
    lights: Option<ResMut<LightSources>>,
    mut sfx: Option<ResMut<crate::world_sfx::WorldSfxRequests>>,
) {
    let Some(mut lights) = lights else { return };
    let dt = time.delta_secs();
    for (mut s, t, mut ob) in &mut objects {
        let s = &mut *s;
        for h in &mut s.since_hit {
            *h += dt;
        }
        for i in 0..s.runs.len() {
            let Some(run) = s.runs[i].as_mut() else { continue };
            let b = &s.data.behaviours[i];
            let seq = &b.sequence;
            let mut holds = |binding: &str| binding.is_empty();
            let mut env = crate::seq_clock::GameEnv { holds: &mut holds, menu: None, skip: 0 };
            let jumps = run.clock.tick(&seq.clocks, dt, &mut env);
            if log_on() && !jumps.is_empty() {
                info!("world seq {}: {:?} jumps to {:.2}", s.object, b.data.label, run.clock.time());
            }
            for (k, item) in seq.items.iter().enumerate() {
                let at = run.clock.frame_time(&seq.clocks, item.clock);
                let inside = at.is_some_and(|t| t >= item.start && t < item.end)
                    && (item.enabled.is_empty() || item.enabled.ends_with("(NOT)"));
                if inside == run.inside[k] {
                    continue;
                }
                run.inside[k] = inside;
                if log_on() {
                    info!(
                        "world seq {}: {:?} item {k} {} {}",
                        s.object,
                        b.data.label,
                        kind_name(&item.kind),
                        if inside { "enters" } else { "stops" }
                    );
                }
                match &item.kind {
                    ItemKind::Entity(EntityItem::BehaviourEnabled { label, off_at_stop, .. }) => {
                        if let Some(ob) = ob.as_deref_mut()
                            && (inside || *off_at_stop)
                        {
                            let hit: Vec<usize> = ob.labelled(label).collect();
                            if log_on() {
                                info!(
                                    "world seq {}: {label:?} {} of {:?}, coronas of {:?}",
                                    s.object,
                                    hit.len(),
                                    ob.fx.behaviours.iter().map(|b| &b.1).collect::<Vec<_>>(),
                                    ob.fx.coronas.iter().map(|c| c.behaviour).collect::<Vec<_>>()
                                );
                            }
                            for j in hit {
                                ob.gated[j] = true;
                                ob.switch(j, inside, t, 8.0, &mut lights);
                            }
                        }
                    }
                    ItemKind::Entity(EntityItem::AddBehaviour { behaviour: Some(id) }) => {
                        if let Some(ob) = ob.as_deref_mut()
                            && let Some(j) = ob.fx.behaviours.iter().position(|x| x.0 == *id)
                        {
                            ob.extra[j] =
                                if inside { ob.extra[j].saturating_add(1) } else { ob.extra[j].saturating_sub(1) };
                        }
                    }
                    ItemKind::Wave(_) if inside => {
                        if let (Some(shot), Some(sfx)) = (b.waves[k].clone(), sfx.as_mut()) {
                            sfx.0.push((shot, t.translation(), "object sequence"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn kind_name(k: &ItemKind) -> &'static str {
    match k {
        ItemKind::Entity(e) => e.name(),
        ItemKind::Wave(_) => "WaveSequenceItem",
        ItemKind::EventTrigger => "EventTriggerSequenceItem",
        ItemKind::VfxSpot(_) => "VfxSpotEffectSequenceItem",
        ItemKind::Picture(_) => "picture item",
        _ => "other item",
    }
}
