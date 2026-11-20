//! The intros' and outros' spawn items (`mw_assets::sequence` "spawns"; docs/decomp.md "Sequence spawn items"),
//! carried out on the playing cutscene's clock (`cutscene.rs` [`CutsceneState::clock`]) and on the clocks of the
//! event's `Timeline` sequences (`event_timeline.rs`; each sequence its own items' states). The sequence item loop
//! (`FUN_00a42b90`) calls an item's prepare method (+8) from its lead time (+4) before the start until it is
//! ready, its enter method (+0xc) once in range, its update (+0x14) every tick in range and its exit (+0x28 →
//! stop +0x24) once the clock has left the range; ours does the same per item:
//!
//! - **`AddRaceCarSequenceItem`**: from 2 s before the start (`0x0085edd0`) the car of `VehicleComponent` is ordered
//!   from the loader; in range it stands on its `AnimatedRaceCarBehaviour` clip's car bone every tick (as the
//!   cutscene's animated race cars), and the stop removes it (`FUN_00853da0`: an owned car is released).
//! - **`SpawnAICopCar`**: once at the start (`FUN_00852e40`) a cop car of the AI type's rollout vehicle stands on
//!   the spawn trigger facing its direction at `Speed` mph and becomes a police cop (`police.rs`) with the role of
//!   its command; with the release flag the stop removes it (`FUN_00852de0`), else it stays.
//! - **`SpawnAnimatedTrafficSequenceItem`**: a traffic car of the type, posed on the clip's bone 1 every tick
//!   (`FUN_008532c0`), removed at the stop (`FUN_008503a0`).
//! - **`OverrideAvailableTrafficSequenceItem`**: traffic picks only the listed types while in range (`FUN_00851f00`
//!   / `FUN_00850050`, traffic messages 0x17 / 0x18).
//! - **`SetPlayerDamageSequenceItem`**: the player's car's zone damage at the start (`FUN_00855110`).
//! - **`GameplaySequenceItem`** (`FUN_00854990`): type 0 `Mode.ForceEnd`, 1 the forced heat level (`police.rs`
//!   [`Police::force_heat_level`]), 0xc the AI cars released, 0xe the heat set to a level's start; the other types
//!   are logged (their targets are not in the runtime; research log 2026-11-20).
//! - **`SpawnDynamicEntitySequenceItem`**: from 2 s before the start the world object is ordered from the loader
//!   (`FUN_008534f0`, world message 0), in range it stands on its `AnimatedDynamicEntityBehaviour` clip
//!   (`FUN_00855520`, message 8), the stop removes it (`FUN_008503e0`, message 3).
//! - **`SpeechEventSequenceItem`** (enter 0x00834710): the first of its speech events whose conditions hold is
//!   requested from the speech manager (`police_speech.rs` [`crate::police_speech::SpeechCore::request_first`]).
//! - **`SpawnTrafficSequenceItem`**: a lane traffic car at the first trigger (`FUN_00853660` → traffic message 0x15,
//!   [`crate::traffic::TrafficSystem::spawn_sequence_car`]).
//!
//! Every item waits for its `EnabledBinding` (the item loop `FUN_00a42b90`; read through the logic's bindings).
//!
//! Model decisions (marked MODEL below): cars for these items are loaded from the sequence's start (the loader
//! needs time; the game streams them from the lead time on); an `AddRaceCar` whose binding names a racer moves that
//! racer's own car (the game copies its vehicle and colour into a new car, or with `8f98df6b` takes a car through
//! the sequence queue's message 4, not traced); paints are the car's first; a cop's brain gets the role of the
//! command (Chasing / LowSpeedChasing → chase, WaitingInterceptor / IdleLights → waiting, FreeRoam → patrol); the
//! damage values go to the zones in clip order in the order the handler posts them.

use std::collections::HashMap;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::sequence::{AddRaceCar, ItemKind, RacerRef, Sequence, SpawnCop, cop_command};
use mw_bundle::ResourceId;
use mw_police::coordinator::Role;

use crate::cutscene::{CutsceneState, Which};
use crate::drive::{Car, CarBuild, CarParts, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests};
use crate::police::{Cop, CopMode, Police};
use crate::race::Racer;

/// Opponent slots of the loader from here up are this module's cars (`ai::Opponents::arrived`).
pub const SLOT: usize = 3_000_000;
/// Brain ids of the cops placed by sequences (`police.rs` uses the cop's slot as its id).
const COP_ID_BASE: usize = 2_000_000;
/// `AddRaceCarSequenceItem`'s lead time (`0x0085edd0` returns `.rdata` 0x00f8a808 = 2 s).
const ADD_RACE_CAR_LEAD: f32 = 2.0;
const MPH: f32 = 0.447_04;

pub struct SpawnsPlugin;

impl Plugin for SpawnsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spawns>()
            .add_systems(Update, run_spawns.after(crate::camera::GameplayCamera))
            .add_systems(FixedUpdate, apply_poses.before(crate::drive::car_physics));
    }
}

/// Per item of the playing sequence.
#[derive(Default)]
struct ItemState {
    /// The loader slot of the car ordered for it.
    slot: Option<usize>,
    /// The car / traffic car placed for it (owned: removed at the stop).
    entity: Option<Entity>,
    /// The racer's own car it moves.
    racer: Option<Entity>,
    /// The placed car goes when the item stops.
    owned: bool,
    entered: bool,
    stopped: bool,
    /// The `EnabledBinding`'s cached value (binding modes 2 / 3).
    gate: Option<bool>,
}

/// A playing sequence whose items run here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Track {
    /// The intro / outro (`cutscene.rs`).
    Cutscene(ResourceId, Which),
    /// One of the event's `Timeline` sequences (`event_timeline.rs`), by UniqueId.
    Timeline(u32),
}

/// The items' states of one playing sequence.
#[derive(Default)]
struct TrackState {
    items: HashMap<usize, ItemState>,
    traffic_override: bool,
}

#[derive(Resource, Default)]
struct Spawns {
    /// The playing sequences' item states.
    tracks: HashMap<Track, TrackState>,
    /// Cars that came back from the loader, by slot.
    arrived: HashMap<usize, (Box<CarSpec>, CarParts)>,
    next_slot: usize,
    /// Cops waiting for their brain: (brain id, role).
    roles: Vec<(u32, Role)>,
    /// Poses for the next fixed tick: entity, position, rotation, velocity.
    poses: Vec<(Entity, Vec3, Quat, Vec3)>,
    /// Dynamic entities' poses for the next fixed tick.
    entity_poses: Vec<(Entity, Vec3, Quat)>,
    /// The next loader tag (+ [`ENTITY_TAG`]).
    next_tag: u32,
}

/// The cop's brain role for its command (MODEL, see the module doc).
fn role_of(command: u8) -> (Role, CopMode) {
    match command {
        9 | 0x16 => (Role::Chase, CopMode::Chase),
        3 => (Role::Patrol, CopMode::Patrol),
        _ => (Role::Waiting, CopMode::Waiting),
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_spawns(
    mut commands: Commands,
    (cutscene, timeline, speech): (
        Res<CutsceneState>,
        Option<Res<crate::event_timeline::EventTimeline>>,
        Option<Res<crate::police_speech::SpeechLink>>,
    ),
    mut spawns: ResMut<Spawns>,
    mut opponents: ResMut<crate::ai::Opponents>,
    requests: Option<Res<LoaderRequests>>,
    (mut traffic, mut models): (
        Option<ResMut<crate::traffic::TrafficSystem>>,
        Option<ResMut<crate::traffic::TrafficModels>>,
    ),
    (mut police, logic): (Option<ResMut<Police>>, Option<NonSend<crate::world_logic::WorldLogic>>),
    (racers, cops, entities): (Query<(Entity, &Racer)>, Query<(Entity, &Cop)>, Query<(Entity, &SequenceEntity)>),
    player: Query<(Entity, &Transform), (With<Car>, With<Player>)>,
    mut damage: Query<&mut crate::damage::CarDamage, With<Player>>,
    spatial: SpatialQuery,
) {
    let spawns = &mut *spawns;
    let mut released: Vec<u32> = Vec::new();
    for (slot, spec, parts) in opponents.scripted.drain(..) {
        spawns.arrived.insert(slot, (spec, parts));
    }
    // A cop's brain exists once the police have adopted the car: give it the command's role.
    if let Some(police) = police.as_mut() {
        spawns.roles.retain(|&(id, role)| match police.cop_mut(id) {
            Some(brain) => {
                brain.role = role;
                false
            }
            None => true,
        });
    }
    // The playing sequences: the cutscene's and the event's `Timeline` ones.
    let mut sources: Vec<(Track, Arc<Sequence>, f32)> = Vec::new();
    if let Some((w, s, t)) = cutscene.clock() {
        sources.push((Track::Cutscene(s.id, w), s.clone(), t));
    }
    if let Some(tl) = timeline.as_deref() {
        sources.extend(tl.clocks().map(|(id, s, t)| (Track::Timeline(id), s.clone(), t)));
    }
    // A sequence that ended or changed: every item of it still running stops.
    let gone: Vec<Track> = spawns.tracks.keys().filter(|k| !sources.iter().any(|(s, ..)| s == *k)).copied().collect();
    for k in gone {
        if let Some(track) = spawns.tracks.remove(&k) {
            stop_track(&mut commands, spawns, track, traffic.as_deref_mut());
        }
    }
    // Objects that arrive for an item no longer running (the sequence stopped while they loaded) go.
    for (e, tag) in &entities {
        let live = spawns
            .tracks
            .values()
            .flat_map(|t| t.items.values())
            .any(|st| !st.stopped && st.slot == Some(tag.0 as usize));
        if !live {
            commands.entity(e).despawn();
        }
    }
    if sources.is_empty() {
        return;
    }
    let player_pos = player.single().ok().map(|(_, t)| t.translation);
    let player = player.single().ok().map(|(e, _)| e);
    let racer_entity = |r: RacerRef| -> Option<Entity> {
        match r {
            RacerRef::LocalPlayer => player,
            RacerRef::Racer(n) => racers
                .iter()
                .find(|(_, rc)| !rc.player && (n as usize).checked_sub(1).is_some_and(|k| rc.grid_slot == Some(k)))
                .map(|(e, _)| e),
            RacerRef::Other => None,
        }
    };
    let requests = requests.as_deref();
    for (key, seq, t) in &sources {
        let t = *t;
        let mut track = spawns.tracks.remove(key).unwrap_or_default();
        for (index, item) in seq.items.iter().enumerate() {
            let in_range = t >= item.start && t < item.end;
            let past = t >= item.end;
            let local = t - item.start;
            let state = track.items.entry(index).or_default();
            if state.stopped {
                continue;
            }
            // `FUN_00a42b90`: an item whose `EnabledBinding` does not hold is neither prepared nor entered (mode 3:
            // read once; 2: once per stay in range; else every frame). Read through the logic's bindings; without
            // the logic (`--no-hsm`) it holds (MODEL). An item already started is not stopped by its binding here
            // (MODEL: the original's every-frame modes would exit it).
            let started = state.entered || state.slot.is_some() || state.entity.is_some() || state.racer.is_some();
            if !item.enabled.is_empty() && !started && carried_here(&item.kind) {
                let holds = match state.gate {
                    Some(v) if matches!(item.binding_mode, 2 | 3) => v,
                    _ => {
                        let v =
                            logic.as_deref().is_none_or(|l| l.state.borrow().resolve_bool(&item.enabled) == Some(true));
                        if state.gate != Some(v) {
                            info!(
                                "cutscene: item {index} binding {:?} (mode {}): {v}",
                                item.enabled, item.binding_mode
                            );
                        }
                        state.gate = Some(v);
                        v
                    }
                };
                if !holds {
                    continue;
                }
            }
            match &item.kind {
                ItemKind::AddRaceCar(c) => {
                    let Some(clip) = c.clips.iter().find(|k| !k.keys.is_empty()) else { continue };
                    if c.racer != RacerRef::Other {
                        // MODEL: the bound racer's own car follows the clip.
                        if in_range {
                            state.racer = racer_entity(c.racer);
                        }
                    } else if state.slot.is_none() && !past && t >= item.start - ADD_RACE_CAR_LEAD {
                        state.slot = spawns_order(&mut spawns.next_slot, requests, c.vehicle);
                    }
                    if in_range
                        && let (Some(k0), None, None) = (clip.key_at(local), state.entity, state.racer)
                        && let Some((spec, parts)) = state.slot.and_then(|s| spawns.arrived.remove(&s))
                    {
                        let car = build(
                            &mut commands,
                            "sequence race car",
                            &spec,
                            &parts,
                            Vec3::from(k0.translation),
                            0.0,
                            Vec3::ZERO,
                        );
                        info!("cutscene: AddRaceCar item {index}: car {} placed at {:.2} s", c.vehicle, t);
                        state.entity = Some(car);
                        state.owned = true;
                    }
                    if in_range
                        && let Some(e) = state.entity.or(state.racer)
                        && let Some(k0) = clip.key_at(local)
                    {
                        let k1 = clip.key_at(local + 1.0 / clip.frame_rate.max(1.0)).unwrap_or(k0);
                        let (a, b) = (Vec3::from(k0.translation), Vec3::from(k1.translation));
                        let mut velocity = (b - a) * clip.frame_rate.max(1.0);
                        if velocity.length() > 120.0 {
                            velocity = Vec3::ZERO;
                        }
                        spawns.poses.push((e, a, Quat::from_array(k0.rotation).normalize(), velocity));
                    }
                    if past {
                        stop_race_car(&mut commands, state, c);
                    }
                }
                ItemKind::SpawnCop(c) => {
                    // MODEL: ordered from the sequence's start.
                    if state.slot.is_none() && !past {
                        match c.vehicle {
                            Some(v) => state.slot = spawns_order(&mut spawns.next_slot, requests, v),
                            None => {
                                info!("cutscene: SpawnAICopCar item {index}: AI type {} has no vehicle", c.ai_type);
                                state.stopped = true;
                                continue;
                            }
                        }
                    }
                    if (in_range || past)
                        && state.entity.is_none()
                        && let Some((spec, parts)) = state.slot.and_then(|s| spawns.arrived.remove(&s))
                    {
                        let id = COP_ID_BASE + state.slot.unwrap_or_default();
                        let car = spawn_cop(&mut commands, police.as_deref(), &spatial, player, c, id, &spec, &parts);
                        let (role, _) = role_of(cop_command(c.command).0);
                        spawns.roles.push((id as u32, role));
                        state.entity = Some(car);
                        state.owned = c.release;
                    }
                    if past && state.entity.is_some() {
                        // `FUN_00852de0`: released only with the flag (`FUN_006ce180`(slot, 3)) once `FUN_006b8840` allows: the
                        // spawn's timer (+0x5c8) is −1 (`FUN_006ce030`'s last argument, `.rdata` 0x00f8f340), so at once.
                        // Without the flag the cop stays, as any AI cop: the coordinator adopts it (`FUN_00521610`, record
                        // +0x1c / +0x24 set) and removes it off screen beyond the level's distance (`FUN_00522c40`).
                        if c.release
                            && let Some(e) = state.entity
                        {
                            commands.entity(e).despawn();
                        }
                        state.stopped = true;
                    }
                }
                ItemKind::AnimatedTraffic(a) => {
                    if in_range
                        && let (Some(k), Some(sys), Some(models)) =
                            (a.key_at(local), traffic.as_deref_mut(), models.as_deref_mut())
                    {
                        let pose = Transform::from_translation(Vec3::from(k.translation))
                            .with_rotation(Quat::from_array(k.rotation).normalize());
                        match state.entity {
                            Some(e) => {
                                commands.entity(e).insert(pose);
                            }
                            None => match sys.vehicle_index(a.vehicle) {
                                Some(v) => {
                                    state.entity =
                                        crate::traffic::spawn_scripted(&mut commands, sys, models, requests, v, pose);
                                    state.owned = true;
                                    if state.entity.is_some() {
                                        info!("cutscene: animated traffic car {} placed at {t:.2} s", a.vehicle);
                                    }
                                }
                                None => {
                                    info!("cutscene: animated traffic car {}: not a traffic type", a.vehicle);
                                    state.stopped = true;
                                }
                            },
                        }
                    }
                    if past {
                        if let Some(e) = state.entity.take() {
                            commands.entity(e).despawn();
                        }
                        state.stopped = true;
                    }
                }
                ItemKind::OverrideTraffic(list) => {
                    if let Some(sys) = traffic.as_deref_mut() {
                        if in_range && !state.entered {
                            let vehicles: Vec<usize> = list.iter().filter_map(|&c| sys.vehicle_index(c)).collect();
                            info!("cutscene: traffic picks only {list:?} ({} types)", vehicles.len());
                            sys.set_vehicle_override(Some(vehicles));
                            track.traffic_override = true;
                            state.entered = true;
                        }
                        if past && state.entered {
                            sys.set_vehicle_override(None);
                            track.traffic_override = false;
                            state.stopped = true;
                        }
                    }
                }
                ItemKind::PlayerDamage(d) if in_range && !state.entered => {
                    state.entered = true;
                    if let Ok(mut dmg) = damage.single_mut() {
                        dmg.set_levels(&d.posted());
                        info!("cutscene: player damage set to {:?}", d.posted());
                    }
                }
                ItemKind::Gameplay { data, trigger_type } => {
                    if in_range && !state.entered {
                        state.entered = true;
                        let ctx = GameplayCtx {
                            index,
                            data: *data,
                            police: police.as_deref_mut(),
                            logic: logic.as_deref(),
                            released: &mut released,
                        };
                        gameplay_enter(*trigger_type, item.enabled.as_str(), ctx);
                    }
                    if past && state.entered && !state.stopped {
                        // `FUN_00851b20`: type 6 → `FUN_008433b0`(0); type 7 repeats the respawn once.
                        if matches!(*trigger_type, 6 | 7) {
                            info!(
                                "cutscene: GameplaySequenceItem {trigger_type} stop: not carried out (see its enter)"
                            );
                        }
                        state.stopped = true;
                    }
                }
                ItemKind::DynamicEntity(d) => {
                    // `FUN_008534f0` from the lead time (2 s, `0x0085eec0`): the entity is ordered once.
                    let Some(object) = d.world_component else {
                        if in_range && !state.entered {
                            state.entered = true;
                            info!("cutscene: SpawnDynamicEntitySequenceItem {index}: no world component");
                        }
                        continue;
                    };
                    if state.slot.is_none() && !past && t >= item.start - ENTITY_LEAD {
                        state.slot = order_entity(&mut spawns.next_tag, requests, object, d);
                    }
                    if state.entity.is_none()
                        && let Some(tag) = state.slot
                        && let Some((e, _)) = entities.iter().find(|(_, s)| s.0 as usize == tag)
                    {
                        state.entity = Some(e);
                        state.owned = true;
                    }
                    if in_range && !state.entered && state.entity.is_some() {
                        // `FUN_00855520`: the behaviours attach (message 8).
                        state.entered = true;
                        info!(
                            "cutscene: dynamic entity {object} placed at {t:.2} s ({} clip frames{})",
                            d.keys.len(),
                            if d.other_behaviours.is_empty() {
                                String::new()
                            } else {
                                format!("; not carried out: {:?}", d.other_behaviours)
                            }
                        );
                    }
                    if in_range
                        && state.entered
                        && let Some(e) = state.entity
                    {
                        let pose = match d.key_at(local) {
                            Some(k) => (Vec3::from(k.translation), Quat::from_array(k.rotation).normalize()),
                            None => (Vec3::from(d.position), Quat::from_array(d.rotation).normalize()),
                        };
                        spawns.entity_poses.push((e, pose.0, pose.1));
                    }
                    if past {
                        // `FUN_008503e0`: message 3, the entity goes.
                        if let Some(e) = state.entity.take() {
                            commands.entity(e).despawn();
                        }
                        state.stopped = true;
                    }
                }
                ItemKind::Speech(events) if in_range && !state.entered => {
                    // 0x00834710: the first event whose conditions hold, when the speech manager is loaded.
                    state.entered = true;
                    let queued = speech.as_deref().is_some_and(|l| l.0.lock().unwrap().request_first(events));
                    info!("cutscene: SpeechEventSequenceItem {index} {events:?} at {t:.2} s: queued {queued}");
                }
                ItemKind::SpawnTraffic(s) if in_range && !state.entered => {
                    // `FUN_00853660`: the first trigger's point (every item in the data uses one trigger; the other
                    // branches, the first `Position` entry or `PositionBinding`, are unused) → traffic message 0x15.
                    state.entered = true;
                    let placed = match (s.points.first(), traffic.as_deref_mut()) {
                        (Some((p, _)), Some(sys)) => match sys.vehicle_index(s.vehicle) {
                            Some(v) => sys.spawn_sequence_car(Vec3::from(*p), v, s.payload, player_pos, &mut commands),
                            None => false,
                        },
                        _ => false,
                    };
                    info!("cutscene: SpawnTraffic {} at trigger {:?}: placed {placed}", s.vehicle, s.triggers.first());
                }
                _ => {}
            }
        }
        spawns.tracks.insert(*key, track);
    }
    // `GameplaySequenceItem` 0xc: the AI cars of the listed types go (`FUN_006ce180`(slot, 4)) where the car exists
    // (`FUN_006ba2a0`) and may be released (`FUN_006b8840`: every AI spawn sets the timer +0x5c8 to −1, so only the
    // unread +0x1c0 / +0x1d0 and the vehicle's +0x18f could hold one; taken as clear). The AI cars are the player
    // manager's slots 12..23: the racers, the coordinator's cops and the sequences' cops. MODEL: only the sequences'
    // cops carry their `AIPlayerType` here, so a type other than 0 (all) matches only those (the data only uses 0).
    for ty in released {
        let mut n = 0;
        for (key, seq, _) in &sources {
            let Some(track) = spawns.tracks.get_mut(key) else { continue };
            for (index, item) in seq.items.iter().enumerate() {
                if let ItemKind::SpawnCop(c) = &item.kind
                    && (ty == 0 || c.ai_type == ty)
                    && let Some(state) = track.items.get_mut(&index)
                    && let Some(e) = state.entity.take()
                {
                    commands.entity(e).despawn();
                    state.stopped = true;
                    n += 1;
                }
            }
        }
        if ty == 0 {
            for (e, _) in cops.iter() {
                commands.entity(e).try_despawn();
                n += 1;
            }
            for (e, r) in racers.iter() {
                if !r.player {
                    commands.entity(e).try_despawn();
                    n += 1;
                }
            }
        }
        info!("cutscene: GameplaySequenceItem 12: {n} AI cars of type {ty} released");
    }
}

/// The item kinds this module carries out (the others are other players' and their bindings theirs).
fn carried_here(kind: &ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::AddRaceCar(_)
            | ItemKind::SpawnCop(_)
            | ItemKind::AnimatedTraffic(_)
            | ItemKind::OverrideTraffic(_)
            | ItemKind::PlayerDamage(_)
            | ItemKind::Gameplay { .. }
            | ItemKind::DynamicEntity(_)
            | ItemKind::SpawnTraffic(_)
            | ItemKind::Speech(_)
    )
}

/// What a `GameplaySequenceItem`'s enter reaches.
struct GameplayCtx<'a> {
    index: usize,
    data: u32,
    police: Option<&'a mut Police>,
    logic: Option<&'a crate::world_logic::WorldLogic>,
    /// The AI types to release after the pass (type 0xc).
    released: &'a mut Vec<u32>,
}

/// `FUN_00854990`: the switch on `TriggerType` with `Data`. Carried out: 0 (`Mode.ForceEnd`, GMM+0x2e7c), 1 (the
/// forced heat level), 0xc (AI cars released, after the pass), 0xe (the heat set to a level's start). The others
/// reach systems the runtime does not have and are logged (research log 2026-11-20).
fn gameplay_enter(trigger_type: u16, binding: &str, c: GameplayCtx) {
    let GameplayCtx { index, data, police, logic, released } = c;
    match trigger_type {
        0 => match logic {
            // GMM+0x2e7c = 1: `Mode.ForceEnd` (the race machine's 730009 ends the mode on it).
            Some(l) => {
                l.state.borrow_mut().game_modes.force_end = true;
                info!("cutscene: GameplaySequenceItem {index}: Mode.ForceEnd set");
            }
            None => info!("cutscene: GameplaySequenceItem {index}: Mode.ForceEnd (no logic to read it)"),
        },
        // `FUN_006675e0` on G+0x2f414, then `FUN_00656180` (empty).
        1 => match police {
            Some(p) => {
                let rose = p.force_heat_level(data);
                let row = p.heat_level().map(|l| (l.id, l.level));
                info!("cutscene: GameplaySequenceItem {index}: heat level {data} forced (row {row:?})");
                if rose {
                    info!("cutscene: the forced level rose: its scoring action (`FUN_006a8050`) is not posted");
                }
            }
            None => info!("cutscene: GameplaySequenceItem {index}: heat level {data}: no police"),
        },
        0xc => released.push(data),
        0xe => {
            let done = police.is_some_and(|p| p.set_heat_to_level(data));
            info!("cutscene: GameplaySequenceItem {index}: heat set to the start of level {data}: {done}");
        }
        // 2: sequence message 0xb; 3: `FUN_006561a0`; 5: GMM+0x2ead; 6: `FUN_008433b0`(1); 7: the player's respawn
        // (`FUN_006cd340`: no respawn request outside the crash reset here); 8: `FUN_006f5d50`(Data); 0xd:
        // `FUN_006cd360`; 0xf: camera event 0x1a and `FUN_00658140`; 0x10: `FUN_006ca4f0`; 0x13: `FUN_008456d0`
        // (the world +0x2f484 records cleared). Other values: no case.
        other => info!(
            "cutscene: GameplaySequenceItem {index} type {other} (data {data}, binding {binding:?}) not carried out"
        ),
    }
}

/// A world object a `SpawnDynamicEntitySequenceItem` placed (the loader's `Msg::Prop` with a tag from
/// [`ENTITY_TAG`] on): this module poses it on its clip and removes it at the stop.
#[derive(Component)]
pub struct SequenceEntity(pub u32);

/// Loader tags of the objects ordered here (`roadblock.rs` uses the ones below).
pub const ENTITY_TAG: u32 = 0xc000_0000;
/// `SpawnDynamicEntitySequenceItem`'s lead time (`0x0085eec0` returns `.rdata` 0x00f8a808 = 2 s).
const ENTITY_LEAD: f32 = 2.0;

/// `FUN_008534f0`'s world message 0: the object at the item's position, facing its rotation's heading (the pose is
/// set in full once it stands). Returns the tag as the state's slot.
fn order_entity(
    next: &mut u32,
    requests: Option<&LoaderRequests>,
    object: ResourceId,
    d: &mw_assets::sequence::DynamicEntity,
) -> Option<usize> {
    let requests = requests?;
    let tag = ENTITY_TAG + *next;
    *next += 1;
    let rot = Quat::from_array(d.rotation).normalize();
    let f = rot * Vec3::Z;
    let prop = crate::loader::TestProp {
        object,
        kind: mw_assets::props::ObjectKind::Dynamic,
        position: Vec3::from(d.position),
        yaw: f.x.atan2(f.z),
    };
    let req =
        CarRequest { bundle: String::new(), paint_index: 0, traffic: None, opponent: None, prop: Some((prop, tag)) };
    requests.0.send(req).ok().map(|_| tag as usize)
}

fn spawns_order(next: &mut usize, requests: Option<&LoaderRequests>, vehicle: u32) -> Option<usize> {
    let requests = requests?;
    let slot = *next;
    *next += 1;
    let req = CarRequest {
        bundle: format!("VEHICLES/VEH_{vehicle}_HI.BNDL"),
        paint_index: 0,
        traffic: None,
        opponent: Some(SLOT + slot),
        prop: None,
    };
    requests.0.send(req).ok().map(|_| slot)
}

fn build(
    commands: &mut Commands,
    name: &str,
    spec: &CarSpec,
    parts: &CarParts,
    start: Vec3,
    heading: f32,
    velocity: Vec3,
) -> Entity {
    let car = crate::drive::build_car(
        commands,
        &CarBuild {
            name,
            spec,
            parts,
            handling: &spec.handling,
            nitrous_rules: None,
            start,
            heading,
            velocity,
            damage: Some(crate::damage::DamageSource(spec.damage.clone().map(|d| (d, spec.impact_graph)))),
        },
    );
    if let Some(engine) = &spec.competitor_engine {
        commands.entity(car).insert(crate::car_audio::PendingCompetitorEngine(engine.clone()));
    }
    car
}

/// `FUN_00852e40`: the cop on its trigger (on the road under it, as the teleports place cars), rolling.
#[allow(clippy::too_many_arguments)]
fn spawn_cop(
    commands: &mut Commands,
    police: Option<&Police>,
    spatial: &SpatialQuery,
    player: Option<Entity>,
    c: &SpawnCop,
    id: usize,
    spec: &CarSpec,
    parts: &CarParts,
) -> Entity {
    let position = Vec3::from(c.position);
    let dir = Vec3::from(c.direction).normalize_or(Vec3::Z);
    let flat = dir.with_y(0.0).normalize_or(Vec3::Z);
    let heading = flat.x.atan2(flat.z);
    let from = position + Vec3::Y * 5.0;
    let filter = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let ground = spatial.cast_ray(from, Dir3::NEG_Y, 40.0, true, &filter).map(|h| from.y - h.distance);
    let start = position.with_y(ground.unwrap_or(position.y) + 0.02);
    let car = build(commands, &format!("sequence cop {id}"), spec, parts, start, heading, dir * c.speed_mph * MPH);
    let (command, name) = cop_command(c.command);
    let (_, mode) = role_of(command);
    // The cop's type: the `Gameplay.CopType` of its `AIPlayerType` (the coordinator counts cops of the behaviour's
    // types by it, `FUN_00526e30`); 0 (the default slot) when no cop type names it.
    let cop_type = police.map_or(0, |p| p.cop_type_of_ai(c.ai_type));
    let (cop, status) = Cop::scripted(id, cop_type, mode);
    commands.entity(car).insert((cop, status));
    match police.and_then(|p| p.graph()).and_then(|g| crate::ai::route_from_ai_graph(g, start, heading, 2000.0)) {
        Some(route) => {
            let mut driver = crate::ai::AiDriver::new(Arc::new(route));
            driver.ignore = player;
            driver.keep_wrecked = true;
            commands.entity(car).insert(driver);
        }
        None => warn!("cutscene: no road for the sequence cop at {start:.0?}"),
    }
    info!(
        "cutscene: SpawnAICopCar: AI type {} (cop type {cop_type}, {}) at {start:.0?}, {} mph, command {name}",
        c.ai_type, spec.bundle, c.speed_mph
    );
    car
}

/// `FUN_00853da0`: the owned car is released; a racer's car is left where the clip put it.
fn stop_race_car(commands: &mut Commands, state: &mut ItemState, _c: &AddRaceCar) {
    if let Some(e) = state.entity.take() {
        commands.entity(e).despawn();
    }
    state.racer = None;
    state.stopped = true;
}

/// A sequence stopped: every item of it still running stops (owned cars go; kept cops stay), its traffic override
/// ends, its cars still loading are dropped.
fn stop_track(
    commands: &mut Commands,
    spawns: &mut Spawns,
    track: TrackState,
    traffic: Option<&mut crate::traffic::TrafficSystem>,
) {
    for (_, state) in track.items {
        if let Some(slot) = state.slot {
            spawns.arrived.remove(&slot);
        }
        if !state.stopped
            && state.owned
            && let Some(e) = state.entity
        {
            commands.entity(e).despawn();
        }
    }
    if track.traffic_override
        && let Some(sys) = traffic
    {
        sys.set_vehicle_override(None);
    }
    spawns.poses.clear();
    spawns.entity_poses.clear();
}

#[allow(clippy::type_complexity)]
fn apply_poses(
    mut spawns: ResMut<Spawns>,
    mut cars: Query<(&mut Transform, &mut LinearVelocity, &mut AngularVelocity), Without<SequenceEntity>>,
    mut entities: Query<(&mut Transform, Option<&mut crate::props::PropBody>), With<SequenceEntity>>,
) {
    // MODEL: the `AnimatedDynamicEntityBehaviour` (its handler not read) stands the object on its clip's bone, as the
    // animated race cars and traffic; its home (the prop reset's pose) follows.
    for (entity, position, rotation) in spawns.entity_poses.drain(..) {
        if let Ok((mut t, body)) = entities.get_mut(entity) {
            t.translation = position;
            t.rotation = rotation;
            if let Some(mut b) = body {
                b.home = *t;
            }
        }
    }
    for (entity, position, rotation, velocity) in spawns.poses.drain(..) {
        if let Ok((mut t, mut lin, mut ang)) = cars.get_mut(entity) {
            t.translation = position;
            t.rotation = rotation;
            lin.0 = velocity;
            ang.0 = Vec3::ZERO;
        }
    }
}
