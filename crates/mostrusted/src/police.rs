//! Police: the cops around the player, what they see, and the pursuit they start (docs/decomp.md "Police and
//! pursuits"). The pursuit rules themselves (heat, chase / cooldown / busted / escaped, the bust meter) are
//! `pursuit.rs`; this module is the game's cop coordinator (`FUN_00526c30`) and the cops' side of it:
//!
//! - **How many cops, which cars**: one per `CopBehaviours` entry of the heat level's main `PursuitBehaviour`
//!   (`FUN_00526e30`, checked once a second, at most 3 new cops per pass); the entry's `Gameplay.CopType` gives
//!   the car through its `AIPlayerType`'s rollout. Heat level 0 fields one cop: the patrol car of free roam.
//! - **Where**: on the road graph between d and √2·d from the player in a straight line, d from the heat level
//!   (`FUN_00513e80`: 350 m with no pursuit, 200 m in a chase, 400 m in the cooldown at level 0–1); removed
//!   beyond the level's removal distance (`FUN_00522c40`).
//! - **What a cop sees** (`FUN_006c5810`): the player within the level's range (a chasing cop's, or any other's),
//!   inside the cone of its forward direction unless very close, with a clear line; it remembers the player for
//!   the level's memory time after losing sight (`FUN_006c6280`).
//! - **Wanted** (`FUN_005192b0`): a cop sees the player above the level's speed limit (110 mph at level 0), or a
//!   cop was rammed, or the pursuit already runs. A cop that knows a wanted player chases; a chasing cop that no
//!   longer knows where the player is drives to the last point it saw and searches (`FUN_005156c0`).
//! - **Chasing** (state 0, `FUN_004fa580`): the target speed is the player's speed corrected by the cop's
//!   distance from its slot (`Positioning`), 1.5 mph per metre within ± 60 mph and at least 30 mph, below the
//!   entry's speed limit; flat out when far away.
//!
//! Cops drive the player's physics with the AI racers' driver (`ai.rs`, `ai_driver.rs`) along road-graph
//! routes this module rebuilds. Model decisions are marked MODEL and listed in decomp.md.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::police::{CopSlot, HeatLevel, PoliceConfig};

use crate::ai::{AiDriver, AiRoute};
use crate::drive::{Car, CarBuild, CarInput, CarParts, ExternalControls, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests};
use crate::pursuit::{CopView, Pursuit, PursuitInput, PursuitState};

/// Cop cars are requested from the loader as opponents with slots from here up (`main.rs` hands them back).
pub const SLOT_BASE: usize = 1000;

const MPH: f32 = 0.44704;

/// `AIConfig` +0x1c4 (−5) and +0x5c (12): a chasing cop's place relative to its target is
/// −5 − `Positioning` × 12 metres behind it (`FUN_00518cf0`).
const FOLLOW_BASE: f32 = -5.0;
const FOLLOW_STEP: f32 = 12.0;
/// `.rdata` 0x00fa9f50 / 0x00fa9f54: the lateral offset of a cop's slot (m): the first cop of a positioning to
/// one side, the others to the other side.
const SLOT_LATERAL: f32 = 3.5;
/// `AIConfig` +0x1d8 (1.5 mph per metre), +0x1dc (60 mph), +0x1d0 (30 mph), +0x1cc (20 s): the chase speed
/// law (`FUN_004fa580`, `FUN_004fa480`).
const CHASE_GAIN: f32 = 1.5;
const CHASE_BAND_MPH: f32 = 60.0;
const CHASE_MIN_MPH: f32 = 30.0;
const CHASE_REACH_TIME: f32 = 20.0;
/// `AIConfig` +0x50 (40 m): within it a chasing cop aims at the target's position 0.5 s ahead (`FUN_004ffcb0`).
const RAM_RANGE: f32 = 40.0;
const RAM_LEAD: f32 = 0.5;
/// `.rdata` 0x00fa2cc0: seconds without new cops after a pursuit has ended (`FUN_00526e30`).
const SPAWN_BLOCK: f32 = 90.0;
/// The coordinator checks the cop count once a second and spawns at most 3 cops per pass (`FUN_00528b40`,
/// `FUN_00526e30`).
const COORDINATOR_PERIOD: f32 = 1.0;
const SPAWNS_PER_PASS: usize = 3;
/// `HeatLevel` +0xc0 (`be25ae81`, 10 s on every level): a cop sent to the last known position gives up the
/// drive there after this long and searches (`FUN_0051a4e0`).
const GOTO_TIMEOUT: f32 = 10.0;

/// MODEL: a cop's route is rebuilt this often (s). The game keeps a 64-node path ring per driver and extends
/// it every tick (`FUN_0050e370`, not read).
const ROUTE_PERIOD: f32 = 0.5;
/// MODEL: how long the outcome (busted / escaped) stands before free roam goes on (s). The game plays the
/// busted sequence 1014946 / the escaped sequence 1394359 here (HSM 457517 "Do Pursuits").
const OUTCOME_SECONDS: f32 = 5.0;
/// MODEL: a cop counts as rammed when the player's car touches it with more than this relative speed (m/s).
/// The game sums contact impacts on the cop and compares with `AIConfig` +0x60 = 2 (units not traced).
const RAM_SPEED: f32 = 5.0;
/// MODEL: spawn points are tried down to this distance when no ground has streamed in farther out (m).
const MIN_SPAWN_DISTANCE: f32 = 25.0;
/// `AIConfig` +0xec (60 m) and the 3.5 m of height: within them, with the player slower than the level's
/// low-speed limit, a chasing cop is in the low-speed chase (state 0x16, `FUN_005240a0`).
const LOW_SPEED_RANGE: f32 = 60.0;
const LOW_SPEED_HEIGHT: f32 = 3.5;
/// MODEL: in the low-speed chase the cop closes in at up to 30 mph and stops this far from the player's car
/// (m). The game's substates (pin, ram, stop: `FUN_004fc3c0`, `FUN_004fcc10`) are not ported.
const LOW_SPEED_STOP: f32 = 10.0;

pub struct PolicePlugin {
    pub enabled: bool,
    /// `--test pursuit`.
    pub test: bool,
}

impl Plugin for PolicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Police>().init_resource::<PursuitHud>();
        if !self.enabled {
            return;
        }
        app.add_systems(Startup, load_police)
            .add_systems(Update, (coordinate, spawn_cops, log_police).chain())
            .add_systems(FixedUpdate, (cop_brains, pursuit_tick).chain().before(crate::ai::ai_drive));
        if self.test {
            app.insert_resource(PursuitTest::default()).add_systems(Update, run_pursuit_test.before(coordinate));
        }
    }
}

/// What the HUD's pursuit bindings read (`hud.rs`).
#[derive(Resource, Default, Debug, Clone)]
pub struct PursuitHud {
    /// `Players.LocalPlayer.IsInChase` / `IsInCooldown`.
    pub in_chase: bool,
    pub in_cooldown: bool,
    /// `Players.LocalPlayer.HeatLevelInt`.
    pub heat_level: u8,
    /// Progress through the level's heat band (`FUN_006bdc50`).
    pub heat_progress: f32,
    /// `Players.LocalPlayer.StaticBustProgress` and `ShowStaticBustBar`.
    pub bust_progress: f32,
    pub show_bust_bar: bool,
    /// Cop positions and facings for the minimap.
    pub cops: Vec<(Vec3, Vec3)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CopMode {
    /// Driving the roads (the game's Patrolling, state 0xd).
    Patrol,
    /// Chasing (state 0).
    Chase,
    /// Driving to the last point the player was seen at, then searching (GoTo / Searching, states 4 / 0xc).
    Search,
}

/// A cop car.
#[derive(Component, Debug)]
pub struct Cop {
    slot: usize,
    cop_type: u32,
    mode: CopMode,
    /// The cop sees the player now; seconds it still remembers the player; where it last saw the player.
    sees: bool,
    memory: f32,
    last_seen: Option<Vec3>,
    /// Seconds since `last_seen` was written.
    seen_age: f32,
    /// The `CopBehaviours` entry: speed limit (mph) and slot relative to the target.
    speed_mph: f32,
    follow: f32,
    lateral: f32,
    route_timer: f32,
    mode_time: f32,
    crashed: bool,
}

impl Cop {
    /// For `police_fx.rs` (sirens, light bars, radio): what the cop is doing, and its slot.
    pub fn mode(&self) -> CopMode {
        self.mode
    }
    pub fn slot(&self) -> usize {
        self.slot
    }
}

/// A cop car on order or on the road.
struct Order {
    slot: usize,
    entry: CopSlot,
    lateral: f32,
    entity: Option<Entity>,
    /// Seconds since the order (a car that never arrives is given up).
    age: f32,
}

#[derive(Resource, Default)]
pub struct Police {
    cfg: Option<PoliceConfig>,
    graph: Option<Arc<mw_formats::AiData>>,
    pub pursuit: Pursuit,
    orders: Vec<Order>,
    arrived: Vec<(usize, Box<CarSpec>, CarParts)>,
    next_slot: usize,
    coordinator_timer: f32,
    /// No new cops for this long (after a pursuit).
    spawn_block: f32,
    /// Seconds the outcome has stood.
    outcome: f32,
    /// A cop was rammed or taken down since the last pursuit tick; heat owed for takedowns.
    rammed: bool,
    takedown_heat: f32,
    /// `--test pursuit`: the player counts as wanted; cops are placed this close; no new cops.
    force_wanted: bool,
    spawn_distance_override: Option<f32>,
    hold_spawns: bool,
    rng: u64,
    /// Totals for the test.
    spawned: u32,
    removed: u32,
}

impl Police {
    /// A uniform float in [0, 1) (the game's LCG, `FUN_005dfe90`).
    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        ((self.rng >> 40) as f32) / (1u64 << 24) as f32
    }

    fn level(&self) -> Option<&HeatLevel> {
        self.pursuit.level(self.cfg.as_ref()?)
    }

    /// Index into the heat level's distance triples: no pursuit, chase, cooldown.
    fn phase(&self) -> usize {
        match self.pursuit.state {
            PursuitState::Chase => 1,
            PursuitState::Cooldown => 2,
            _ => 0,
        }
    }
}

fn load_police(game: Res<crate::Game>, mut police: ResMut<Police>) {
    match game.store.police_config() {
        Ok(cfg) if !cfg.levels.is_empty() => {
            info!(
                "police: {} heat levels, bust within {} m below {} mph for {} s, {} cop types",
                cfg.levels.len(),
                cfg.bust_distance,
                cfg.bust_speed_mph,
                cfg.bust_time,
                cfg.cop_vehicles.len()
            );
            police.cfg = Some(cfg);
        }
        Ok(_) => warn!("police: no heat levels in the data; no police"),
        Err(e) => warn!("police: no data ({e}); no police"),
    }
    match game.store.ai_data() {
        Ok(g) => police.graph = Some(Arc::new(g)),
        Err(e) => warn!("police: no road graph ({e}); no police"),
    }
    // `MW_PURSUIT_DEMO=1`: the player counts as wanted and cops are placed close by (for screenshots of the HUD).
    if std::env::var("MW_PURSUIT_DEMO").is_ok() {
        police.force_wanted = true;
        police.spawn_distance_override = Some(TEST_SPAWN_DISTANCE);
    }
    police.pursuit.radar = 1.0;
    police.rng = 0x2545_f491_4f6c_dd1d;
}

/// The coordinator's once-a-second pass (`FUN_00526e30`): order the cops the heat level's behaviour lacks,
/// drop the ones it no longer wants or that are too far away.
fn coordinate(
    mut commands: Commands,
    time: Res<Time>,
    mut police: ResMut<Police>,
    requests: Option<Res<LoaderRequests>>,
    player: Query<&Transform, (With<Player>, With<Car>)>,
    cops: Query<(Entity, &Transform, &Cop), Without<Player>>,
) {
    let police = &mut *police;
    let dt = time.delta_secs();
    police.spawn_block = (police.spawn_block - dt).max(0.0);
    police.coordinator_timer -= dt;
    for o in &mut police.orders {
        o.age += dt;
    }
    if police.coordinator_timer > 0.0 {
        return;
    }
    police.coordinator_timer = COORDINATOR_PERIOD;
    let (Some(cfg), Some(requests)) = (police.cfg.as_ref(), requests) else { return };
    let Ok(player) = player.single() else { return };
    let Some(level) = police.pursuit.level(cfg).cloned() else { return };
    let phase = police.phase();

    // Removal: beyond the level's distance in the ground plane (`FUN_00522c40`). MODEL: the game removes a cop
    // only while it is off screen (`FUN_006bf380`); here the distance alone decides.
    let limit = level.remove_distance[phase];
    for (e, t, cop) in &cops {
        let d = Vec2::new(t.translation.x - player.translation.x, t.translation.z - player.translation.z).length();
        if d > limit {
            info!("police: cop {} removed {d:.0} m away (limit {limit:.0} m)", cop.slot);
            commands.entity(e).despawn();
            police.orders.retain(|o| o.slot != cop.slot);
            police.removed += 1;
        }
    }
    // A car that never came (60 s) is forgotten.
    police.orders.retain(|o| o.entity.is_some() || o.age < 60.0);

    // The cops the behaviour wants, by type: one per entry (`FUN_00526e30`).
    let behaviour = &level.main;
    if !behaviour.spawn_allowed || police.spawn_block > 0.0 || police.hold_spawns {
        return;
    }
    let mut new = 0;
    for (i, entry) in behaviour.cops.iter().enumerate() {
        let wanted = behaviour.cops.iter().filter(|c| c.cop_type == entry.cop_type).count();
        let have = police.orders.iter().filter(|o| o.entry.cop_type == entry.cop_type).count();
        if have >= wanted || new >= SPAWNS_PER_PASS {
            continue;
        }
        let Some(&vehicle) = cfg.cop_vehicles.get(&entry.cop_type) else {
            warn!("police: cop type {} has no car", entry.cop_type);
            continue;
        };
        // The first cop of a positioning to one side, the others to the other (`FUN_00518cf0`).
        let first = !behaviour.cops[..i].iter().any(|c| c.positioning == entry.positioning);
        let slot = police.next_slot;
        police.next_slot += 1;
        let req = CarRequest {
            bundle: format!("VEHICLES/VEH_{vehicle}_HI.BNDL"),
            paint_index: 0,
            traffic: None,
            opponent: Some(SLOT_BASE + slot),
        };
        if requests.0.send(req).is_err() {
            warn!("police: the loader thread has stopped");
            return;
        }
        info!("police: cop {slot} ordered (type {}, VEH_{vehicle})", entry.cop_type);
        police.orders.push(Order {
            slot,
            entry: *entry,
            lateral: if first { SLOT_LATERAL } else { -SLOT_LATERAL },
            entity: None,
            age: 0.0,
        });
        new += 1;
    }
}

/// A spawn point for a cop: a point of the road graph between d and √2·d from the player in a straight line
/// (`FUN_0051ca20`'s test), facing along the road towards the player. MODEL: which node — the game picks
/// ahead of or behind the player or a side road by the game mode's weights (`FUN_005261d0`, `GameMode` not
/// decoded); here a random point of the ring whose ground has streamed in, the ring shrinking when none has.
fn spawn_point(
    police: &mut Police,
    graph: &mw_formats::AiData,
    player: Vec3,
    distance: f32,
    taken: &[Vec3],
    spatial: &SpatialQuery,
) -> Option<(Vec3, f32)> {
    let ground = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    // Points every 10 m along the road graph's links around the player, with the link's direction.
    let reach = distance * std::f32::consts::SQRT_2 + 50.0;
    let mut points: Vec<(Vec3, Vec3)> = Vec::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        let a = Vec3::from(n.position);
        if n.width <= 0.0 || a.distance(player) > reach {
            continue;
        }
        for &l in &n.links {
            let m = &graph.nodes[l as usize];
            if (l as usize) < i || m.width <= 0.0 {
                continue;
            }
            let b = Vec3::from(m.position);
            let steps = (a.distance(b) / 10.0).ceil().max(1.0) as usize;
            points.extend((0..steps).map(|k| (a.lerp(b, k as f32 / steps as f32), b - a)));
        }
    }
    let mut d = distance;
    let (mut rings, mut tried, mut blocked) = (0, 0, 0);
    while d >= MIN_SPAWN_DISTANCE {
        let ring: Vec<&(Vec3, Vec3)> = points
            .iter()
            .filter(|(p, _)| {
                let r = p.distance(player);
                r >= d && r <= d * std::f32::consts::SQRT_2
            })
            .collect();
        rings += ring.len();
        for _ in 0..ring.len().min(12) {
            let &(p, along) = ring[(police.random() * ring.len() as f32) as usize % ring.len()];
            // MODEL: one cop per stretch of road (the game spreads a spawn's cars across the road's width,
            // `FUN_005220b0`).
            if taken.iter().any(|t| t.distance(p) < 15.0) {
                blocked += 1;
                continue;
            }
            tried += 1;
            let origin = p + Vec3::Y * 5.0;
            let Some(hit) = spatial.cast_ray(origin, Dir3::NEG_Y, 40.0, true, &ground) else { continue };
            // Facing along the road, the way that leads towards the player.
            let dir = if along.dot(player - p) >= 0.0 { along } else { -along };
            return Some((origin - Vec3::Y * hit.distance + Vec3::Y * 0.15, dir.x.atan2(dir.z)));
        }
        d *= 0.7;
    }
    debug!(
        "police: no spawn point within {distance:.0} m of {player:.0?}: {rings} points, {tried} without ground, {blocked} taken"
    );
    None
}

/// Place the cop cars that have arrived from the loader.
#[allow(clippy::type_complexity)]
fn spawn_cops(
    mut commands: Commands,
    mut police: ResMut<Police>,
    mut opponents: ResMut<crate::ai::Opponents>,
    player: Query<(Entity, &Transform, &LinearVelocity), (With<Player>, With<Car>)>,
    cops: Query<&Transform, With<Cop>>,
    spatial: SpatialQuery,
) {
    let police = &mut *police;
    // Cop cars come back from the loader through the opponents' hand-over (`ai::Opponents::arrived`).
    if !opponents.cops.is_empty() {
        police.arrived.append(&mut opponents.cops);
    }
    if police.arrived.is_empty() {
        return;
    }
    let (Some(cfg), Some(graph)) = (police.cfg.clone(), police.graph.clone()) else { return };
    let Ok((player_entity, player, player_velocity)) = player.single() else { return };
    let Some(level) = police.pursuit.level(&cfg).cloned() else { return };
    let distance = police.spawn_distance_override.unwrap_or(level.spawn_distance[police.phase()]);
    let mut taken: Vec<Vec3> = cops.iter().map(|t| t.translation).collect();
    let mut waiting = Vec::new();
    for (slot, spec, parts) in std::mem::take(&mut police.arrived) {
        let Some(order) = police.orders.iter().position(|o| o.slot == slot) else { continue };
        let Some((start, heading)) = spawn_point(police, &graph, player.translation, distance, &taken, &spatial) else {
            debug!("police: no spawn point yet for cop {slot}");
            waiting.push((slot, spec, parts));
            continue;
        };
        taken.push(start);
        let entry = police.orders[order].entry;
        // The spawn speed is the cop type's (`AIPlayerType.SpawnSpeed`, mph; `FUN_005220b0`).
        let mut speed = cfg.cop_spawn_speed.get(&entry.cop_type).copied().unwrap_or(0.0).max(0.0) * MPH;
        if police.spawn_distance_override.is_some() {
            // The test places cops close by: no faster than the player.
            speed = speed.min(player_velocity.0.length());
        }
        let rotation = Quat::from_rotation_y(heading);
        let car = crate::drive::build_car(
            &mut commands,
            &CarBuild {
                name: &format!("cop {slot}"),
                spec: &spec,
                parts: &parts,
                handling: &spec.handling,
                nitrous_rules: None,
                start,
                heading,
                velocity: rotation * Vec3::Z * speed,
                damage: Some(crate::damage::DamageSource(spec.damage.clone().map(|d| (d, spec.impact_graph)))),
            },
        );
        if let Some(engine) = &spec.competitor_engine {
            commands.entity(car).insert(crate::car_audio::PendingCompetitorEngine(engine.clone()));
        }
        let Some(route) = crate::ai::route_from_ai_graph(&graph, start, heading, 2000.0) else {
            warn!("police: no road at {start:.0?} for cop {slot}");
            commands.entity(car).despawn();
            police.orders.remove(order);
            continue;
        };
        let mut driver = AiDriver::new(Arc::new(route));
        driver.ignore = Some(player_entity);
        commands.entity(car).insert((
            driver,
            Cop {
                slot,
                cop_type: entry.cop_type,
                mode: CopMode::Patrol,
                sees: false,
                memory: 0.0,
                last_seen: None,
                seen_age: 0.0,
                speed_mph: entry.speed_mph,
                follow: FOLLOW_BASE - entry.positioning as f32 * FOLLOW_STEP,
                lateral: police.orders[order].lateral,
                route_timer: 0.0,
                mode_time: 0.0,
                crashed: false,
            },
        ));
        police.orders[order].entity = Some(car);
        police.spawned += 1;
        info!(
            "police: cop {slot} (type {}, {}) placed at {start:.0?}, {:.0} m from the player",
            entry.cop_type,
            spec.bundle,
            start.distance(player.translation)
        );
    }
    police.arrived = waiting;
}

/// A road-graph route from `from` through `to` and on along `onward` (a ground-plane direction).
fn route_via(graph: &mw_formats::AiData, from: Vec3, to: Vec3, onward: Vec3) -> Option<AiRoute> {
    let a = graph.nearest_node(from.to_array(), false)?;
    let b = graph.nearest_node(to.to_array(), false)?;
    let mut nodes = graph.path(a, b)?;
    // Onward from the goal, the way the path builders walk the graph (`ai::route_from_ai_graph`).
    let mut dir = Vec2::new(onward.x, onward.z).normalize_or(Vec2::Y);
    let mut prev = nodes.len().checked_sub(2).map(|i| nodes[i]);
    let mut cur = b;
    let mut total = 0.0;
    while total < 400.0 {
        let Some(next) = graph.next_node(cur, dir.to_array(), prev) else { break };
        let d = Vec3::from(graph.nodes[next].position) - Vec3::from(graph.nodes[cur].position);
        total += d.length();
        dir = Vec2::new(d.x, d.z).normalize_or(dir);
        prev = Some(cur);
        cur = next;
        nodes.push(cur);
    }
    crate::ai::spline_through(graph, &nodes)
}

/// Every fixed tick, before the AI drivers: what each cop sees, what it does about it, where it drives and how
/// fast.
#[allow(clippy::type_complexity)]
fn cop_brains(
    time: Res<Time>,
    mut police: ResMut<Police>,
    spatial: SpatialQuery,
    player: Query<(&Transform, &LinearVelocity, &Car), With<Player>>,
    mut cops: Query<
        (&Transform, &LinearVelocity, &Car, &mut Cop, &mut AiDriver, Has<crate::car_impacts::AiCrash>),
        Without<Player>,
    >,
) {
    let dt = time.delta_secs();
    let police = &mut *police;
    let (Some(cfg), Some(graph)) = (police.cfg.as_ref(), police.graph.clone()) else { return };
    let Ok((pt, pv, pcar)) = player.single() else { return };
    let Some(level) = police.pursuit.level(cfg).cloned() else { return };
    let player_pos = pt.translation;
    let player_mph = pv.0.length() / MPH;
    let pursued = police.pursuit.in_chase_or_cooldown();
    let world = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);

    // Sight (`FUN_006c5810`, `FUN_006c6280`) and contact.
    let mut speeding_seen = false;
    for (t, v, car, mut cop, _, crashed) in &mut cops {
        let to = player_pos - t.translation;
        let d = to.length();
        let range = if cop.mode == CopMode::Chase { level.sight_chasing } else { level.sight_idle };
        let mut sees = d <= range;
        if sees && d > level.sight_near {
            let fwd = t.rotation * Vec3::Z;
            sees = fwd.dot(to / d.max(1e-3)) >= level.sight_cone_deg.to_radians().cos();
        }
        if sees && d > 2.0 {
            let eye = t.translation + Vec3::Y;
            if let Ok(dir) = Dir3::new(player_pos + Vec3::Y - eye) {
                sees = spatial.cast_ray(eye, dir, d, true, &world).is_none();
            }
        }
        cop.sees = sees && !crashed;
        // A cop that crashes next to the player was taken down (`FUN_006ccd20` → `FUN_006b8d00`; the cop
        // type's takedown scoring action adds heat). MODEL: "by the player" = within 30 m of the player's car.
        if crashed && !cop.crashed && d < 30.0 {
            police.rammed = true;
            police.takedown_heat += cfg.takedown_heat.get(&cop.cop_type).copied().unwrap_or(0.0);
            info!("police: cop {} taken down", cop.slot);
        }
        cop.crashed = crashed;
        cop.seen_age += dt;
        if cop.sees {
            cop.memory = level.memory;
            cop.last_seen = Some(player_pos);
            cop.seen_age = 0.0;
            speeding_seen |= player_mph > level.speeding_mph;
        } else {
            cop.memory = (cop.memory - dt).max(0.0);
        }
        // MODEL: contact = the cop's centre within the player's box grown by the cop's half width.
        if touching(pt, pcar, t, car) && (pv.0 - v.0).length() > RAM_SPEED {
            police.rammed = true;
        }
    }
    // `FUN_005192b0`: wanted.
    let wanted = pursued || speeding_seen || police.rammed || police.force_wanted;

    for (t, v, _, mut cop, mut driver, crashed) in &mut cops {
        if crashed {
            continue;
        }
        let known = cop.sees || cop.memory > 0.0;
        let before = cop.mode;
        cop.mode = match cop.mode {
            CopMode::Patrol | CopMode::Search if known && wanted => CopMode::Chase,
            // `FUN_005156c0`: the target is no longer known.
            CopMode::Chase if !known => {
                if cop.last_seen.is_some() {
                    CopMode::Search
                } else {
                    CopMode::Patrol
                }
            }
            // MODEL: at the last known point (or after the level's 10 s) the cop patrols on; the game's
            // search legs (random 72° sectors at a growing radius, `FUN_005255f0`) are not ported.
            CopMode::Search
                if cop.mode_time > GOTO_TIMEOUT || cop.last_seen.is_none_or(|p| p.distance(t.translation) < 30.0) =>
            {
                CopMode::Patrol
            }
            m => m,
        };
        cop.mode_time += dt;
        cop.route_timer -= dt;
        if cop.mode != before {
            debug!("police: cop {} {:?} -> {:?}", cop.slot, before, cop.mode);
            cop.mode_time = 0.0;
            cop.route_timer = 0.0;
        }
        let fwd = t.rotation * Vec3::Z;
        // The route.
        let near_end = driver.route.length() - driver.progress < 150.0;
        let rebuild = match cop.mode {
            CopMode::Patrol => near_end || (cop.mode != before),
            _ => cop.route_timer <= 0.0,
        };
        if rebuild {
            cop.route_timer = ROUTE_PERIOD;
            let route = match cop.mode {
                CopMode::Patrol => crate::ai::route_from_ai_graph(&graph, t.translation, fwd.x.atan2(fwd.z), 2000.0),
                CopMode::Chase => {
                    // Within 40 m the cop aims where the target will be in 0.5 s (`FUN_004ffcb0`).
                    let lead =
                        if t.translation.distance(player_pos) < RAM_RANGE { pv.0 * RAM_LEAD } else { Vec3::ZERO };
                    let heading = if pv.0.length() > 2.0 { pv.0 } else { pt.rotation * Vec3::Z };
                    route_via(&graph, t.translation, player_pos + lead, heading)
                }
                CopMode::Search => cop.last_seen.and_then(|p| route_via(&graph, t.translation, p, fwd)),
            };
            // A route too short to drive (the cop and its goal on one node at a dead end) is not taken.
            if let Some(route) = route.filter(|r| r.length() > 60.0) {
                driver.route = Arc::new(route);
                driver.finished = None;
                // Project onto the new route from its start.
                driver.time = 0.0;
                driver.progress = 0.0;
                driver.corner_speeds = None;
            }
        }
        // The cop's slot beside the target (`FUN_004ffcb0`: the lateral offset, kept on the road): the
        // driver's offset from its path.
        driver.state.lane_offset = if cop.mode == CopMode::Chase { cop.lateral } else { 0.0 };
        // The speed.
        driver.schedule_speed = match cop.mode {
            CopMode::Chase => {
                // `FUN_004fa580`: the target's speed, corrected by how far the cop is from its slot.
                let (s_cop, _) = driver.route.project(t.translation, None, 0.0);
                let (s_player, _) = driver.route.project(player_pos, None, 0.0);
                let lead = s_cop - s_player;
                let mph = (player_mph - (lead + cop.follow) * CHASE_GAIN)
                    .clamp(player_mph - CHASE_BAND_MPH, player_mph + CHASE_BAND_MPH)
                    .max(CHASE_MIN_MPH);
                // `FUN_004fa480`: only within 20 s of the target; flat out otherwise. MODEL: the time is
                // the distance over the cop's own speed.
                let reach = lead.abs() / v.0.length().max(1.0);
                let mph = if reach <= CHASE_REACH_TIME { mph } else { f32::MAX };
                let gap = t.translation.distance(player_pos);
                let low_speed = player_mph < level.low_speed_mph
                    && gap < LOW_SPEED_RANGE
                    && (t.translation.y - player_pos.y).abs() < LOW_SPEED_HEIGHT;
                let mph = if low_speed {
                    player_mph + ((gap - LOW_SPEED_STOP) * CHASE_GAIN).clamp(0.0, CHASE_MIN_MPH)
                } else {
                    mph
                };
                mph.min(if cop.speed_mph > 0.0 { cop.speed_mph } else { f32::MAX }) * MPH
            }
            // `FUN_004f76c0`: the level's limit for driving to a point and searching. MODEL: also the patrol
            // speed (the patrol state drives traffic lanes at their speed, `FUN_005101e0`, not read).
            CopMode::Search | CopMode::Patrol => level.search_speed_mph * MPH,
        };
    }
}

/// MODEL: the cars touch when the cop's centre lies within the player's box grown by the cop's half width and
/// a hand's breadth (the game reads the physics contact list, `FUN_0065cda0`).
fn touching(pt: &Transform, pcar: &Car, ct: &Transform, ccar: &Car) -> bool {
    let local = pt.rotation.inverse() * (ct.translation - pt.translation);
    let (pe, ce) = (pcar.params.body_half_extents, ccar.params.body_half_extents);
    let r = ce.x.max(ce.z) + 0.3;
    local.x.abs() < pe.x + r && local.z.abs() < pe.z + r && local.y.abs() < 2.0
}

/// Every fixed tick: the pursuit manager's update (`FUN_0066ae60`) and what follows from it.
#[allow(clippy::type_complexity)]
fn pursuit_tick(
    mut commands: Commands,
    time: Res<Time>,
    mut police: ResMut<Police>,
    mut hud: ResMut<PursuitHud>,
    mut input: ResMut<CarInput>,
    player: Query<(&Transform, &LinearVelocity, &Car), With<Player>>,
    cops: Query<(Entity, &Transform, &Car, &Cop, Has<crate::car_impacts::AiCrash>), Without<Player>>,
) {
    let dt = time.delta_secs();
    let police = &mut *police;
    let Some(cfg) = police.cfg.as_ref() else { return };
    let Ok((pt, pv, pcar)) = player.single() else { return };
    let views: Vec<CopView> = cops
        .iter()
        .map(|(_, t, car, cop, crashed)| CopView {
            position: t.translation,
            chasing: cop.mode == CopMode::Chase,
            crashed,
            touching: touching(pt, pcar, t, car),
        })
        .collect();
    // The freshest point a cop saw the player at, while a cop knows where the player is (`FUN_005192b0`).
    let last_seen = cops
        .iter()
        .filter(|c| c.3.sees || c.3.memory > 0.0)
        .filter_map(|c| Some((c.3.seen_age, c.3.last_seen?)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|c| c.1);
    let events = police.pursuit.tick(
        cfg,
        &PursuitInput {
            dt,
            position: pt.translation,
            speed_mph: pv.0.length() / MPH,
            cops: &views,
            last_seen,
            cop_taken_down: std::mem::take(&mut police.rammed),
            // Not wired: our crash rules have no wrecker (`crash.rs`), and there are no roadblocks.
            wrecked_by_cop: false,
            roadblock_near: false,
        },
    );
    let mut events = events;
    let owed = std::mem::take(&mut police.takedown_heat);
    if police.pursuit.in_chase_or_cooldown() {
        police.pursuit.add_heat(cfg, owed, &mut events);
    }
    for e in &events {
        let p = &police.pursuit;
        info!(
            "police: {e:?} (heat {:.2}, level {}, {} cops chasing, {:.0} s chased)",
            p.heat,
            p.level_number(cfg),
            p.cops_chasing,
            p.chase_time
        );
    }
    // The outcome stands for a moment, then free roam goes on (HSM "Do Pursuits": after a bust the heat is
    // reset and every cop removed; either way no cops for a while, `FUN_00526e30`).
    if matches!(police.pursuit.state, PursuitState::Busted | PursuitState::Escaped) {
        let busted = police.pursuit.state == PursuitState::Busted;
        if busted {
            // The busted car does not drive on.
            input.scripted = Some([0.0, 1.0, 0.0, 1.0]);
        }
        police.outcome += dt;
        if police.outcome >= OUTCOME_SECONDS {
            police.outcome = 0.0;
            police.pursuit.reset(busted);
            police.spawn_block = SPAWN_BLOCK;
            if busted {
                input.scripted = None;
            }
            for (e, t, _, cop, _) in &cops {
                // MODEL: after an escape the game removes the cops that are off screen; here those out of
                // their sight range.
                if busted || t.translation.distance(pt.translation) > 100.0 {
                    commands.entity(e).despawn();
                    police.orders.retain(|o| o.slot != cop.slot);
                    police.removed += 1;
                }
            }
        }
    }
    let p = &police.pursuit;
    *hud = PursuitHud {
        in_chase: p.state == PursuitState::Chase,
        in_cooldown: p.state == PursuitState::Cooldown,
        heat_level: p.level_number(cfg),
        heat_progress: p.heat_progress(cfg),
        bust_progress: p.bust_meter,
        show_bust_bar: p.show_bust_bar,
        cops: cops.iter().map(|(_, t, ..)| (t.translation, t.rotation * Vec3::Z)).collect(),
    };
}

/// Once a second while something is going on (or with `MW_POLICE_LOG=1`): the pursuit and every cop.
fn log_police(
    time: Res<Time>,
    mut next: Local<f32>,
    police: Res<Police>,
    player: Query<(&Transform, &LinearVelocity), With<Player>>,
    cops: Query<(&Transform, &LinearVelocity, &Cop, &AiDriver), Without<Player>>,
) {
    static LOG: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("MW_POLICE_LOG").is_ok());
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now.floor() + 1.0;
    let Some(cfg) = police.cfg.as_ref() else { return };
    let p = &police.pursuit;
    if !*LOG && p.state == PursuitState::None {
        return;
    }
    let Ok((pt, pv)) = player.single() else { return };
    info!(
        "pursuit {:?}: heat {:.2} (level {}), bust {:.2}, bar {:+.2} (kind {}, {:.2}), {} chasing, player {:.0} mph, {:.0} m from last seen, {:.0} s",
        p.state,
        p.heat,
        p.level_number(cfg),
        p.bust_meter,
        p.bar,
        p.bar_kind(),
        p.bar_progress(),
        p.cops_chasing,
        pv.0.length() / MPH,
        p.last_seen.distance(pt.translation),
        p.chase_time
    );
    for (t, v, cop, driver) in &cops {
        info!(
            "  cop {} (type {}) {:?}: {:.0} m away, {:.0} mph (target {:.0}), sees {} memory {:.1}, route {:.0}/{:.0} m lateral {:+.1}, recovering {} finished {}, slot {:+.0} m {:+.1} m",
            cop.slot,
            cop.cop_type,
            cop.mode,
            t.translation.distance(pt.translation),
            v.0.length() / MPH,
            driver.schedule_speed.min(999.0) / MPH,
            cop.sees,
            cop.memory,
            driver.progress,
            driver.route.length(),
            driver.lateral,
            driver.state.recovering.is_some(),
            driver.finished.is_some(),
            cop.follow,
            cop.lateral
        );
    }
}

/// `--test pursuit`: a pursuit near the player, start to end, twice.
///
/// 1. *Escape*: the player's car drives off along the road (the AI racers' driver); the level's cops are
///    placed close behind and the player counts as wanted, so the first cop to see it starts the chase. After
///    [`TEST_CHASE_SECONDS`] the cops are taken away (as if shaken off): the player leaves the lose radius, the
///    cooldown runs the heat down, escaped.
/// 2. *Bust*: the player's car stands still; a cop is placed close, sees it, drives up, and the bust meter
///    fills.
#[derive(Resource, Default)]
struct PursuitTest {
    phase: u8,
    timer: f32,
    /// Smallest and largest distance of the nearest cop during the chase (m), the highest heat.
    nearest: [f32; 2],
    max_heat: f32,
    chase_seconds: f32,
    cooldown_seconds: f32,
}

const TEST_CHASE_SECONDS: f32 = 25.0;
/// Within the level-0 sight range of a cop that is not chasing (50 m).
const TEST_SPAWN_DISTANCE: f32 = 30.0;
const TEST_TIMEOUT: f32 = 240.0;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_pursuit_test(
    mut commands: Commands,
    time: Res<Time>,
    mut test: ResMut<PursuitTest>,
    mut police: ResMut<Police>,
    mut input: ResMut<CarInput>,
    mut virtual_time: ResMut<Time<Virtual>>,
    player: Query<(Entity, &Transform, &LinearVelocity), (With<Player>, With<Car>)>,
    cops: Query<(Entity, &Transform, &Cop), Without<Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    let police = &mut *police;
    let Ok((me, pt, pv)) = player.single() else { return };
    let (Some(cfg), Some(graph)) = (police.cfg.clone(), police.graph.clone()) else {
        info!("TEST RESULT pursuit: no police data");
        exit.write(AppExit::Success);
        return;
    };
    let dt = time.delta_secs();
    test.timer += dt;
    let nearest = cops.iter().map(|c| c.1.translation.distance(pt.translation)).reduce(f32::min);
    let p = police.pursuit.clone();
    match test.phase {
        // Set up the escape run.
        0 => {
            virtual_time.set_relative_speed(2.0);
            police.force_wanted = true;
            police.spawn_distance_override = Some(TEST_SPAWN_DISTANCE);
            police.spawn_block = 0.0;
            let fwd = pt.rotation * Vec3::Z;
            if let Some(route) = crate::ai::route_from_ai_graph(&graph, pt.translation, fwd.x.atan2(fwd.z), 6000.0) {
                // The player's car keeps to 60 mph, so that the cops (placed close, at its speed) can follow.
                let mut driver = AiDriver::new(Arc::new(route));
                driver.schedule_speed = 60.0 * MPH;
                commands.entity(me).insert((driver, ExternalControls));
            } else {
                info!("TEST RESULT pursuit: no road at the player's car");
                exit.write(AppExit::Success);
            }
            test.nearest = [f32::MAX, 0.0];
            test.phase = 1;
            test.timer = 0.0;
        }
        // Wait for the chase.
        1 if p.state == PursuitState::Chase => {
            info!(
                "TEST RESULT pursuit start: chase after {:.1} s, {} cops spawned, nearest {:.0} m, heat {:.2} (level {}), player {:.0} mph",
                test.timer,
                police.spawned,
                nearest.unwrap_or(-1.0),
                p.heat,
                p.level_number(&cfg),
                pv.0.length() / MPH
            );
            test.phase = 2;
            test.timer = 0.0;
        }
        // The chase: cops follow; then they are taken away.
        2 => {
            if let Some(d) = nearest {
                test.nearest = [test.nearest[0].min(d), test.nearest[1].max(d)];
            }
            test.max_heat = test.max_heat.max(p.heat);
            if test.timer >= TEST_CHASE_SECONDS || p.state != PursuitState::Chase {
                info!(
                    "TEST RESULT pursuit chase: {:.0} s, state {:?}, {} cops ({} chasing), nearest cop {:.0} to {:.0} m, heat {:.2} (level {}), bust meter {:.2}, player {:.0} mph",
                    test.timer,
                    p.state,
                    cops.iter().count(),
                    p.cops_chasing,
                    test.nearest[0],
                    test.nearest[1],
                    p.heat,
                    p.level_number(&cfg),
                    p.bust_meter,
                    pv.0.length() / MPH
                );
                test.chase_seconds = test.timer;
                police.hold_spawns = true;
                police.force_wanted = false;
                for (e, _, cop) in &cops {
                    commands.entity(e).despawn();
                    police.orders.retain(|o| o.slot != cop.slot);
                }
                police.orders.clear();
                test.phase = 3;
                test.timer = 0.0;
            }
        }
        // Out of the lose radius: the cooldown.
        3 if p.state == PursuitState::Cooldown => {
            info!(
                "TEST RESULT pursuit cooldown: began {:.1} s after the cops were gone, {:.0} m from the last seen point (lose radius {:.0} m), heat {:.2}",
                test.timer,
                p.last_seen.distance(pt.translation),
                police.level().map_or(0.0, |l| l.lose_radius),
                p.heat
            );
            test.phase = 4;
            test.timer = 0.0;
        }
        4 if p.state == PursuitState::Escaped => {
            info!(
                "TEST RESULT pursuit outcome: ESCAPED after {:.1} s of cooldown (max heat {:.2}, max level {}), {} escapes",
                test.timer, test.max_heat, p.max_level, p.escapes
            );
            test.cooldown_seconds = test.timer;
            test.phase = 5;
            test.timer = 0.0;
        }
        // Free roam again: stop the car, then the bust run.
        5 if p.state == PursuitState::None => {
            commands.entity(me).remove::<(AiDriver, ExternalControls)>();
            input.scripted = Some([0.0, 1.0, 0.0, 0.0]);
            if pv.0.length() < 0.5 {
                police.hold_spawns = false;
                police.spawn_block = 0.0;
                police.force_wanted = true;
                police.spawn_distance_override = Some(TEST_SPAWN_DISTANCE);
                test.phase = 6;
                test.timer = 0.0;
            }
        }
        6 if p.state == PursuitState::Chase => {
            info!(
                "TEST RESULT pursuit start 2: chase after {:.1} s with the player at rest, nearest cop {:.0} m, heat {:.2}",
                test.timer,
                nearest.unwrap_or(-1.0),
                p.heat
            );
            test.phase = 7;
            test.timer = 0.0;
        }
        7 if p.state == PursuitState::Busted => {
            info!(
                "TEST RESULT pursuit outcome 2: BUSTED {:.1} s after the chase began (bust time {} s within {} m), nearest cop {:.0} m, heat {:.2}, {} busts",
                test.timer,
                cfg.bust_time,
                cfg.bust_distance,
                nearest.unwrap_or(-1.0),
                p.heat,
                p.busts
            );
            test.phase = 8;
            test.timer = 0.0;
        }
        8 if p.state == PursuitState::None => {
            info!(
                "TEST RESULT pursuit: {} chases, {} escapes, {} busts, {} cops spawned, {} removed, {} left, heat {:.2}",
                p.chases,
                p.escapes,
                p.busts,
                police.spawned,
                police.removed,
                cops.iter().count(),
                p.heat
            );
            exit.write(AppExit::Success);
        }
        _ => {}
    }
    if test.timer > TEST_TIMEOUT {
        info!(
            "TEST RESULT pursuit: TIMEOUT in phase {} (state {:?}, {} cops, nearest {:?} m, heat {:.2}, bust {:.2})",
            test.phase,
            p.state,
            cops.iter().count(),
            nearest,
            p.heat,
            p.bust_meter
        );
        exit.write(AppExit::Success);
    }
}
