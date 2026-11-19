//! The intros' and outros' spawn items (`mw_assets::sequence` "spawns"; docs/decomp.md "Sequence spawn items"),
//! carried out on the playing cutscene's clock (`cutscene.rs` [`CutsceneState::clock`]). The sequence item loop
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
//! - `GameplaySequenceItem`, `SpawnDynamicEntitySequenceItem`, `SpawnTrafficSequenceItem`: read, logged once, not
//!   carried out (their targets are not in the runtime; research log 2026-11-19).
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
use mw_assets::sequence::{AddRaceCar, ItemKind, RacerRef, SpawnCop, cop_command};
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
}

#[derive(Resource, Default)]
struct Spawns {
    /// The sequence the states belong to.
    seq: Option<(ResourceId, Which)>,
    items: HashMap<usize, ItemState>,
    /// Cars that came back from the loader, by slot.
    arrived: HashMap<usize, (Box<CarSpec>, CarParts)>,
    next_slot: usize,
    /// Cops waiting for their brain: (brain id, role).
    roles: Vec<(u32, Role)>,
    /// Poses for the next fixed tick: entity, position, rotation, velocity.
    poses: Vec<(Entity, Vec3, Quat, Vec3)>,
    traffic_override: bool,
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
    cutscene: Res<CutsceneState>,
    mut spawns: ResMut<Spawns>,
    mut opponents: ResMut<crate::ai::Opponents>,
    requests: Option<Res<LoaderRequests>>,
    (mut traffic, mut models): (Option<ResMut<crate::traffic::TrafficSystem>>, ResMut<crate::traffic::TrafficModels>),
    mut police: Option<ResMut<Police>>,
    racers: Query<(Entity, &Racer)>,
    player: Query<Entity, (With<Car>, With<Player>)>,
    mut damage: Query<&mut crate::damage::CarDamage, With<Player>>,
    spatial: SpatialQuery,
) {
    let spawns = &mut *spawns;
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
    let clock = cutscene.clock().map(|(w, s, t)| (w, s.clone(), t));
    let key = clock.as_ref().map(|(w, s, _)| (s.id, *w));
    if key != spawns.seq {
        // The sequence ended or changed: every running item stops.
        stop_all(&mut commands, spawns, traffic.as_deref_mut());
        spawns.seq = key;
    }
    let Some((_, seq, t)) = clock else { return };
    let player = player.single().ok();
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
    for (index, item) in seq.items.iter().enumerate() {
        let in_range = t >= item.start && t < item.end;
        let past = t >= item.end;
        let local = t - item.start;
        let state = spawns.items.entry(index).or_default();
        if state.stopped {
            continue;
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
                    // `FUN_00852de0`: released only with the flag (`FUN_006ce180`(slot, 3)).
                    if c.release
                        && let Some(e) = state.entity
                    {
                        commands.entity(e).despawn();
                    }
                    state.stopped = true;
                }
            }
            ItemKind::AnimatedTraffic(a) => {
                if in_range && let (Some(k), Some(sys)) = (a.key_at(local), traffic.as_deref_mut()) {
                    let pose = Transform::from_translation(Vec3::from(k.translation))
                        .with_rotation(Quat::from_array(k.rotation).normalize());
                    match state.entity {
                        Some(e) => {
                            commands.entity(e).insert(pose);
                        }
                        None => match sys.vehicle_index(a.vehicle) {
                            Some(v) => {
                                state.entity =
                                    crate::traffic::spawn_scripted(&mut commands, sys, &mut models, requests, v, pose);
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
                        spawns.traffic_override = true;
                        state.entered = true;
                    }
                    if past && state.entered {
                        sys.set_vehicle_override(None);
                        spawns.traffic_override = false;
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
            ItemKind::Gameplay { data, trigger_type } if in_range && !state.entered => {
                state.entered = true;
                info!(
                    "cutscene: GameplaySequenceItem {trigger_type} (data {data}, binding {:?}) not carried out",
                    item.enabled
                );
            }
            ItemKind::DynamicEntity(d) if in_range && !state.entered => {
                state.entered = true;
                info!("cutscene: SpawnDynamicEntitySequenceItem {:?} not carried out", d.world_component);
            }
            ItemKind::SpawnTraffic(s) if in_range && !state.entered => {
                state.entered = true;
                info!("cutscene: SpawnTrafficSequenceItem {} not carried out", s.vehicle);
            }
            _ => {}
        }
    }
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
    let (cop, status) = Cop::scripted(id, 0, mode);
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
        "cutscene: SpawnAICopCar: AI type {} ({}) at {start:.0?}, {} mph, command {name}",
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

/// The sequence stopped: every item still running stops (owned cars go; kept cops stay), the traffic override
/// ends.
fn stop_all(commands: &mut Commands, spawns: &mut Spawns, traffic: Option<&mut crate::traffic::TrafficSystem>) {
    for (_, state) in spawns.items.drain() {
        if !state.stopped
            && state.owned
            && let Some(e) = state.entity
        {
            commands.entity(e).despawn();
        }
    }
    if spawns.traffic_override
        && let Some(sys) = traffic
    {
        sys.set_vehicle_override(None);
    }
    spawns.traffic_override = false;
    spawns.arrived.clear();
    spawns.poses.clear();
}

fn apply_poses(
    mut spawns: ResMut<Spawns>,
    mut cars: Query<(&mut Transform, &mut LinearVelocity, &mut AngularVelocity)>,
) {
    for (entity, position, rotation, velocity) in spawns.poses.drain(..) {
        if let Ok((mut t, mut lin, mut ang)) = cars.get_mut(entity) {
            t.translation = position;
            t.rotation = rotation;
            lin.0 = velocity;
            ang.0 = Vec3::ZERO;
        }
    }
}
