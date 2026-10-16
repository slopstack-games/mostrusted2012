//! Police: the cops around the player, what they see, how they drive, and the pursuit they start (docs/decomp.md
//! "Police and pursuits", "How cops drive (2026-10-14)"). The pursuit rules themselves (heat, chase / cooldown /
//! busted / escaped, the bust meter) are `pursuit.rs`; how a cop drives in a pursuit is `cop_driver.rs`; this
//! module is the game's cop coordinator (`FUN_00526c30`) and the cops' side of it:
//!
//! - **How many cops, which cars** (`FUN_00526e30`, once a second): one per `CopBehaviours` entry of the heat
//!   level's main `PursuitBehaviour`, at most 3 new per pass; crashed cops do not count (they are replaced), cops
//!   beyond the wanted count (farthest first) become surplus.
//! - **Where** (`FUN_005261d0`): outside a chase anywhere on the ring of the level's spawn distance r (random
//!   direction, the nearest road within 60 m, at least r away; `FUN_0051b4e0`), patrolling at 20 mph. In a chase
//!   by the free-roam game mode's weights and the cop type's flags: behind the player or ahead of it (on the road
//!   between r and √2·r, facing the player, at the type's spawn speed: `FUN_0051ca20`, `FUN_005220b0`), or waiting
//!   in a side road 100–250 m ahead (`FUN_0051fd10`); a region without cops doubles its weight (`FUN_0051ce70`).
//! - **Removal** (`FUN_00522c40`, `FUN_00522dc0`, `FUN_00522f40`): only off screen (a 4 m sphere outside the
//!   view, `FUN_006bf380`), beyond the level's removal distance (surplus cops beyond 100 m, crashed ones beyond
//!   125 m); when a pursuit ends every cop off screen goes at once and no cops come for 90 s (`FUN_005286d0`).
//! - **What a cop sees** (`FUN_006c5810`, `FUN_006c6280`) and **wanted** (`FUN_005192b0`): as before.
//! - **States** (`FUN_005240a0`, `FUN_005255f0`, `FUN_0051a4e0`): while chased every cop drives its behaviour
//!   (`cop_driver`), near a slow player the low-speed chase and its box-in (`FUN_00520f10`); in the cooldown one
//!   cop goes to the last-seen point and the others search in legs around the player (`vt_0050dd40`); otherwise
//!   they patrol.
//!
//! Model decisions are marked MODEL and listed in decomp.md.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;
use mw_assets::police::{CopSlot, HeatLevel, PoliceConfig};

use crate::ai::{AiDriver, AiRoute};
use crate::cop_driver::{Assigned, CarState, CopConfig, CopDriver, CopInput, DriveState, Probes};
use crate::cop_patrol::{PatrolDrive, PatrolInput, PatrolMode};
use crate::cop_sight::{CopNotice, Perception, SightInput, Wanted};
use crate::drive::{Car, CarBuild, CarInput, CarParts, ExternalControls, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests};
use crate::pursuit::{CopView, Pursuit, PursuitInput, PursuitState};

/// Cop cars are requested from the loader as opponents with slots from here up (`main.rs` hands them back).
pub const SLOT_BASE: usize = 1000;

const MPH: f32 = 0.44704;

/// `.rdata` 0x00fa9f50 / 0x00fa9f54: the lateral slot of a chasing cop (m): the first cop of a positioning −1, 0
/// or 1 to one side, the others to the other side; other positionings none (`FUN_00518cf0`).
const SLOT_LATERAL: f32 = 3.5;
/// `.rdata` 0x00fa2cc0: seconds without new cops after a pursuit has ended (`FUN_005286d0`).
const SPAWN_BLOCK: f32 = 90.0;
/// The coordinator checks the cop count once a second and spawns at most 3 cops per pass (`FUN_00528b40`,
/// `FUN_00526e30`).
const COORDINATOR_PERIOD: f32 = 1.0;
const SPAWNS_PER_PASS: usize = 3;
/// `.data` 0x01106228 (20 mph): a free-roam (patrol) cop's spawn speed (`FUN_00522590`).
const PATROL_SPAWN_MPH: f32 = 20.0;
/// The generic spawn: up to 10 tries, the nearest road within 60 m of the ring point, no other car closer than
/// 200 m (`FUN_0051b4e0`; 0x010c44cc = 40000).
const GENERIC_TRIES: usize = 10;
const GENERIC_SNAP: f32 = 60.0;
const GENERIC_DY: f32 = 5.0;
/// Side-road spawns (`FUN_0051fd10`): junctions 100–250 m along the player's route, 20 m down the branch (at least
/// 10 m), at least 100 m from the player, out of the player's sight.
const SIDE_ROAD_MIN: f32 = 100.0;
const SIDE_ROAD_MAX: f32 = 250.0;
const SIDE_ROAD_DEPTH: f32 = 20.0;
/// The view test of removal: a sphere of 4 m (0x40800000) against the camera frustum (`FUN_006bf380`).
const VIEW_SPHERE: f32 = 4.0;
/// `FUN_0051ce70`: regions of the spawn rebalancing: ahead / side / behind by the cosine 0.707 to the player's
/// forward, cops within 20 m not counted.
const REGION_COS: f32 = 0.707;
const REGION_NEAR: f32 = 20.0;
/// A cop in the cooldown's search: legs in five 72° sectors (0x010c44c4), each 20 m (0x00fa9a0c) wider than the
/// last, around the player's position; a leg ends at its point (within 20 m), or after 3 s without moving
/// (`FUN_004f7af0`, `FUN_0051a000`, `vt_0050dd40`).
const SEARCH_SECTORS: usize = 5;
const SEARCH_STEP: f32 = 20.0;
const SEARCH_ARRIVED: f32 = 20.0;
const SEARCH_STUCK: f32 = 3.0;
/// `FUN_005255f0`: patrolling cops join the search during the first 7 s of a cooldown (`FUN_00658160`).
const COOLDOWN_JOIN: f32 = 7.0;
/// WaitingInterceptor (state 0x11, `FUN_004fbff0`): a waiting cop leaves when the player closes within 50 m
/// (`DAT_010c4330` = 2500) or moves away from it.
const WAIT_RELEASE: f32 = 50.0;
/// `FUN_005005a0`: 10 mph while a cop turns round (its path ahead lies behind it) below 40 mph.
const TURN_ROUND_MPH: f32 = 10.0;
const TURN_ROUND_BELOW_MPH: f32 = 40.0;
/// `FUN_004fa750`: a cop uses nitrous outside lead −5 … 15 m or more than 20 m away, 2.7 m/s below its target.
const NITROUS_LEAD: [f32; 2] = [-5.0, 15.0];
const NITROUS_RANGE: f32 = 20.0;
const NITROUS_GAP: f32 = 2.7;
/// The box-in (`FUN_00520f10`): one second after a cop blocks, the low-speed cops creep in.
const BOX_IN_TIME: f32 = 1.0;
/// The road path is planned again when the target's point 0.5 s ahead is more than 10 m (0x00fa929c) off it
/// (`FUN_00508fd0`).
const REPLAN_DISTANCE: f32 = 10.0;
/// MODEL: the drivable straight line of the direct path (the game's nav-mesh line query `FUN_00e49bb0`) is a
/// physics ray at this height over the world's collision.
const LINE_HEIGHT: f32 = 0.8;
/// The busted sequence 1014946 and the escaped sequence 1394359 last 13 s (HSM 457517 "Do Pursuits"; the busted
/// one disables the car's controls).
const OUTCOME_SECONDS: f32 = 13.0;
/// `FUN_005240a0`: the low-speed chase only within 3.5 m of height.
const LOW_SPEED_HEIGHT: f32 = 3.5;

mod start_test;

pub struct PolicePlugin {
    pub enabled: bool,
    /// `--test pursuit`.
    pub test: bool,
    /// `--test cops`: when a pursuit starts (`start_test.rs`).
    pub start_test: bool,
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
        if self.start_test {
            app.insert_resource(start_test::StartTest::default())
                .add_systems(Update, start_test::run_start_test.before(coordinate));
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

/// What a cop does (the coordinator's view of its driver state).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CopMode {
    /// Driving the roads (the game's Patrolling, state 0xd).
    Patrol,
    /// A pursuit state (`cop_driver`).
    Chase,
    /// Driving to the last point the player was seen at (GoTo, state 4).
    GoTo,
    /// Searching in legs around the player (state 0xc).
    Search,
    /// Waiting in a side road for the player (WaitingInterceptor, state 0x11).
    Waiting,
}

/// A search leg (`vt_0050dd40`).
#[derive(Debug, Clone, Copy, Default)]
struct Leg {
    target: Vec3,
    sector: usize,
    still: f32,
}

/// A cop car.
#[derive(Component, Debug)]
pub struct Cop {
    slot: usize,
    cop_type: u32,
    mode: CopMode,
    /// What the cop notices of the player (`cop_sight`).
    sight: Perception,
    /// The Patrolling state's mode and ghost (`cop_patrol`).
    patrol: PatrolMode,
    /// The `CopBehaviours` entry's speed limit (mph, driver +0x5860) and the lateral slot (record +0x14).
    speed_mph: f32,
    lateral: f32,
    driver: CopDriver,
    /// The behaviour still wants this cop (record +0x24); surplus cops are removed at 100 m.
    wanted: bool,
    route_timer: f32,
    mode_time: f32,
    /// The current road path aims at this point (`FUN_00508fd0`: re-planned when the target leaves it).
    planned: Option<Vec3>,
    direct: bool,
    /// The chase core's stop flags of the last tick (`cop_driver::CopOutput::blocked`).
    blocked: u8,
    search_radius: f32,
    leg: Leg,
    crashed: bool,
    /// The cop and the player's car touched last tick (for SLAM and rams).
    touching: bool,
}

/// What a cop is doing, for other modules (sirens and light bars, chatter, the map). Updated every fixed tick.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct CopStatus {
    pub activity: CopActivity,
    /// The game's driver state id: 0xd Patrolling, 0 Chasing, 0x16 LowSpeedChasing, 0x18 ChasingBruiser, 0x12
    /// Rhino, 0x13 HeadOn, 4 GoTo, 0xc Searching, 0x11 WaitingInterceptor (`FUN_004efc40`'s table).
    pub driver_state: u8,
    /// The Patrolling state's mode (+0x90: 0 find a lane, 1 drive to it, 2 ask for a ghost, 3 merge, 4 ride the
    /// ghost in traffic), when patrolling.
    pub patrol_mode: Option<u8>,
    /// The game's "chasing" predicate (`FUN_006bbb70`, which `FUN_006beaa0` exposes for cop players): the driver
    /// is in state 0, 0x10, 0x12, 0x16, 0x17, 0x18 or 0x1a. The pursuit manager counts these cops.
    pub chasing: bool,
    /// The cop takes part in a pursuit of the player (chasing, going to the last-seen point, searching or
    /// waiting while a chase or cooldown runs).
    pub in_pursuit: bool,
    /// The cop sees the player (`cop_sight`).
    pub sees_player: bool,
    /// The cop's car has crashed.
    pub wrecked: bool,
}

/// [`CopStatus::activity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopActivity {
    /// Driving in traffic (Patrolling).
    #[default]
    Patrol,
    Chase,
    /// Driving to where the player was last seen.
    GoTo,
    Search,
    /// Waiting in a side road for the player (WaitingInterceptor).
    Waiting,
}

// Read by the siren / light-bar / radio modules (`police_fx`, `police_speech`) and the map.
impl Cop {
    /// What the cop does.
    pub fn mode(&self) -> CopMode {
        self.mode
    }
    /// The cop's slot (its order number; the loader hands its car back as opponent `SLOT_BASE + slot`).
    pub fn slot(&self) -> usize {
        self.slot
    }
    /// The patrol's mode number (+0x90) while patrolling (`cop_patrol.rs`).
    #[allow(dead_code)]
    pub fn patrol_mode(&self) -> Option<u8> {
        (self.mode == CopMode::Patrol).then(|| self.patrol.number())
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

/// Where a new cop comes from (`FUN_005261d0`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum SpawnKind {
    /// Outside a chase: on the ring, patrolling (`FUN_005227a0`).
    Generic,
    /// Ahead of the player, oncoming (mode 1).
    Ahead,
    /// Ahead on the player's route, facing the same way (mode 0x10).
    AheadAlong,
    /// Behind the player (mode 2).
    Behind,
    /// Waiting in a side road (mode 8, WaitingInterceptor).
    Side,
}

#[derive(Resource, Default)]
pub struct Police {
    cfg: Option<PoliceConfig>,
    cop_cfg: CopConfig,
    graph: Option<Arc<mw_formats::AiData>>,
    pub pursuit: Pursuit,
    orders: Vec<Order>,
    arrived: Vec<(usize, Box<CarSpec>, CarParts)>,
    next_slot: usize,
    coordinator_timer: f32,
    /// No new cops for this long (after a pursuit).
    spawn_block: f32,
    /// The last pass saw a chase or cooldown (`FUN_005286d0` +0x1f0).
    was_pursuit: bool,
    /// Seconds the outcome has stood.
    outcome: f32,
    /// A cop was taken down since the last pursuit tick; heat owed (takedowns, SLAM, traffic hits).
    taken_down: bool,
    /// The coordinator's wanted test (`FUN_005192b0`).
    pub wanted: Wanted,
    owed_heat: f32,
    /// The box-in timer (coordinator +0x720).
    box_in: f32,
    /// Seconds without throttle or brake from the player (cop state +0x90).
    idle_input: f32,
    /// `--test pursuit`: the player counts as wanted; cops are placed this close; no new cops.
    force_wanted: bool,
    spawn_distance_override: Option<f32>,
    hold_spawns: bool,
    /// `--test cops`: patrolling cops stand still where the test puts them.
    test_hold: bool,
    rng: u64,
    /// Totals for the test.
    spawned: u32,
    removed: u32,
    slams: u32,
    /// Spawn distances and kinds, for the test.
    spawn_log: Vec<(SpawnKind, f32)>,
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

    /// Index into the heat level's distance triples: no pursuit, chase, cooldown (`FUN_00513e80`).
    fn phase(&self) -> usize {
        match self.pursuit.state {
            PursuitState::Chase => 1,
            PursuitState::Cooldown => 2,
            _ => 0,
        }
    }
}

/// Read the cop states' `AIConfig` fields by hash (instance offsets in `cop_driver::CopConfig`).
fn load_cop_config(store: &mw_assets::AssetStore) -> CopConfig {
    use mw_formats::genesys::Value;
    let mut c = CopConfig::default();
    let Ok(obj) = store.genesys_object(mw_bundle::ResourceId(0x010000000007fb0e)) else { return c };
    let f = |hash: u32, slot: &mut f32| {
        if let Some(Value::Float(v)) = obj.get(hash) {
            *slot = *v;
        }
    };
    f(0xa8397bbd, &mut c.lead_time); // 0x48
    f(0x15bf72c3, &mut c.close_range); // 0x50
    f(0xa7a0cd14, &mut c.ram_threshold); // 0x60
    f(0x8bab1401, &mut c.misaligned_deg); // 0x64
    f(0xcda1c099, &mut c.misaligned_distance); // 0x68
    f(0xb8a139ea, &mut c.probe_stop_distance); // 0x6c
    f(0x97ecb8e6, &mut c.misaligned_mph); // 0x70
    f(0x7424b241, &mut c.probe_stop_mph); // 0x74
    f(0x168d4d25, &mut c.probe_forward); // 0x78
    f(0xb6e1dd51, &mut c.probe_side); // 0x7c
    f(0x3d50f2d3, &mut c.probe_angle_deg); // 0x80
    f(0xed0bc167, &mut c.alongside_ahead); // 0x84
    f(0x007c5485, &mut c.alongside_behind); // 0x88
    f(0xea5c87a7, &mut c.low_speed_range); // 0xec
    f(0xf7b3c5b2, &mut c.ls_brake); // 0xf0
    f(0xf2fd2d52, &mut c.ls_lead_time); // 0xf8
    f(0x8949c5de, &mut c.ls_block_along); // 0xfc
    f(0x29c6c627, &mut c.ls_block_lat); // 0x100
    f(0x63ce38e2, &mut c.ls_behind_along); // 0x104
    f(0x4145fe02, &mut c.ls_ahead_along); // 0x108
    f(0x03836e24, &mut c.ls_behind_lat); // 0x10c
    f(0xcad6f50d, &mut c.ls_alongside_along); // 0x110
    f(0x8b06bc1c, &mut c.ls_slow_mph); // 0x114
    f(0x1ad129b8, &mut c.ls_alongside_lat); // 0x118
    f(0x299536a1, &mut c.ls_spin_steer); // 0x11c
    f(0xfa57a8a7, &mut c.max_lead); // 0x12c
    c
}

fn load_police(game: Res<crate::Game>, mut police: ResMut<Police>) {
    match game.store.police_config() {
        Ok(cfg) if !cfg.levels.is_empty() => {
            info!(
                "police: {} heat levels, bust within {} m below {} mph for {} s, {} cop types, spawn weights {:?}",
                cfg.levels.len(),
                cfg.bust_distance,
                cfg.bust_speed_mph,
                cfg.bust_time,
                cfg.cop_vehicles.len(),
                cfg.spawn_weights
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
    police.cop_cfg = load_cop_config(&game.store);
    // `MW_PURSUIT_DEMO=1`: the player counts as wanted and cops are placed close by (for screenshots of the HUD).
    if std::env::var("MW_PURSUIT_DEMO").is_ok() {
        police.force_wanted = true;
        police.spawn_distance_override = Some(TEST_SPAWN_DISTANCE);
    }
    // `MW_COP_DISTANCE=<m>`: free-roam cops are placed this far away (to watch them patrol).
    if let Some(d) = std::env::var("MW_COP_DISTANCE").ok().and_then(|v| v.parse::<f32>().ok()) {
        police.spawn_distance_override = Some(d);
    }
    police.pursuit.radar = 1.0;
    police.rng = 0x2545_f491_4f6c_dd1d;
}

/// A cop is on screen when a 4 m sphere around it meets the camera's frustum (`FUN_006bf380`).
fn on_screen(frustum: Option<&Frustum>, p: Vec3) -> bool {
    frustum.is_some_and(|f| f.intersects_sphere(&Sphere { center: p.into(), radius: VIEW_SPHERE }, true))
}

/// The coordinator's once-a-second pass (`FUN_00528b40` → `FUN_00526e30`): drop the cops it no longer keeps,
/// order the ones the heat level's behaviour lacks.
#[allow(clippy::type_complexity)]
fn coordinate(
    mut commands: Commands,
    time: Res<Time>,
    mut police: ResMut<Police>,
    requests: Option<Res<LoaderRequests>>,
    player: Query<&Transform, (With<Player>, With<Car>)>,
    mut cops: Query<(Entity, &Transform, &mut Cop, Has<crate::car_impacts::AiCrash>), Without<Player>>,
    frustum: Query<&Frustum, crate::MainCamera>,
) {
    let police = &mut *police;
    let dt = time.delta_secs();
    if police.spawn_block > 0.0 {
        police.spawn_block -= dt;
    }
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
    let frustum = frustum.iter().next();
    let pursuit = police.pursuit.in_chase_or_cooldown();
    let flat = |p: Vec3| Vec2::new(p.x - player.translation.x, p.z - player.translation.z).length();

    // The behaviour's wanted cops per type, nearest first; the rest are surplus (`FUN_00526e30`).
    let behaviour = &level.main;
    let mut by_distance: Vec<(f32, Entity, u32)> = cops
        .iter()
        .filter(|c| !c.3)
        .map(|(e, t, cop, _)| (t.translation.distance(player.translation), e, cop.cop_type))
        .collect();
    by_distance.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut kept: std::collections::HashMap<u32, usize> = Default::default();
    for (_, e, ty) in &by_distance {
        let want = behaviour.cops.iter().filter(|c| c.cop_type == *ty).count();
        let n = kept.entry(*ty).or_default();
        if let Ok((_, _, mut cop, _)) = cops.get_mut(*e) {
            cop.wanted = *n < want;
        }
        *n += 1;
    }

    // Removal, only while off screen: a crashed cop beyond 125 m (`FUN_00522f40`); in a chase beyond the chase
    // removal distance, a surplus cop beyond 100 m (`FUN_00522c40`); otherwise beyond the free / cooldown removal
    // distance (`FUN_00522dc0`). The first pass after a pursuit removes every cop off screen (`FUN_005286d0`).
    let ended = police.was_pursuit && !pursuit;
    police.was_pursuit = pursuit;
    if ended {
        police.spawn_block = SPAWN_BLOCK;
    }
    for (e, t, cop, crashed) in &cops {
        if on_screen(frustum, t.translation) {
            continue;
        }
        let d = flat(t.translation);
        let limit = if crashed {
            level.crashed_remove_distance
        } else if phase == 1 && !cop.wanted {
            level.surplus_remove_distance
        } else {
            level.remove_distance[phase]
        };
        let far = if crashed { t.translation.distance(player.translation) >= limit } else { d > limit };
        if far || ended {
            info!("police: cop {} removed {d:.0} m away off screen (limit {limit:.0} m, crashed {crashed})", cop.slot);
            commands.entity(e).despawn();
            police.orders.retain(|o| o.slot != cop.slot);
            police.removed += 1;
        }
    }
    // A car that never came (60 s) is forgotten.
    police.orders.retain(|o| o.entity.is_some() || o.age < 60.0);

    if !behaviour.spawn_allowed || police.spawn_block > 0.0 || police.hold_spawns {
        return;
    }
    // Crashed cops do not count: they are replaced (`FUN_00526e30` counts living, uncrashed cops and spawns).
    let crashed_slots: Vec<usize> = cops.iter().filter(|c| c.3).map(|c| c.2.slot).collect();
    let mut new = 0;
    for (i, entry) in behaviour.cops.iter().enumerate() {
        let wanted = behaviour.cops.iter().filter(|c| c.cop_type == entry.cop_type).count();
        let have = police
            .orders
            .iter()
            .filter(|o| o.entry.cop_type == entry.cop_type && !crashed_slots.contains(&o.slot))
            .count();
        if have >= wanted || new >= SPAWNS_PER_PASS {
            continue;
        }
        let Some(&vehicle) = cfg.cop_vehicles.get(&entry.cop_type) else {
            warn!("police: cop type {} has no car", entry.cop_type);
            continue;
        };
        // The first cop of a positioning −1 … 1 to one side, the others to the other (`FUN_00518cf0`).
        let lateral = if (-1..=1).contains(&entry.positioning) {
            let first = !behaviour.cops[..i].iter().any(|c| c.positioning == entry.positioning);
            if first { SLOT_LATERAL } else { -SLOT_LATERAL }
        } else {
            0.0
        };
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
        police.orders.push(Order { slot, entry: *entry, lateral, entity: None, age: 0.0 });
        new += 1;
    }
    // Crashed cops' orders no longer hold a place (their cars stay until removed).
    police.orders.retain(|o| !crashed_slots.contains(&o.slot));
}

/// Walk the road graph from the node nearest `from` along `dir` (the way the game's route walkers pick the best
/// aligned link, `FUN_00436f50`), calling `visit(node, along, previous)` until it returns false; a node with fewer
/// than 2 links ends the walk.
fn walk(graph: &mw_formats::AiData, from: Vec3, dir: Vec2, mut visit: impl FnMut(usize, f32, usize) -> bool) {
    let Some(mut cur) = graph.nearest_node(from.to_array(), false) else { return };
    let mut prev: Option<usize> = None;
    let mut d = dir;
    let mut along = 0.0;
    for _ in 0..400 {
        let Some(next) = graph.next_node(cur, d.to_array(), prev) else { return };
        let (a, b) = (Vec3::from(graph.nodes[cur].position), Vec3::from(graph.nodes[next].position));
        along += a.distance(b);
        d = Vec2::new(b.x - a.x, b.z - a.z).normalize_or(d);
        prev = Some(cur);
        cur = next;
        if graph.nodes[cur].links.len() < 2 || !visit(cur, along, prev.unwrap()) {
            return;
        }
    }
}

/// A spawn point and facing (yaw) and speed (m/s) for a cop, by kind (`FUN_005229e0`, `FUN_0051ca20`,
/// `FUN_0051c610`, `FUN_0051fd10`, `FUN_0051b4e0`).
#[allow(clippy::too_many_arguments)]
fn spawn_point(
    police: &mut Police,
    graph: &mw_formats::AiData,
    kind: SpawnKind,
    player: &Transform,
    r: f32,
    spawn_speed: f32,
    taken: &[Vec3],
    spatial: &SpatialQuery,
    traffic: Option<&crate::traffic::TrafficSystem>,
) -> Option<(Vec3, f32, f32)> {
    let ground = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let drop = |p: Vec3| -> Option<Vec3> {
        let origin = p + Vec3::Y * 5.0;
        let hit = spatial.cast_ray(origin, Dir3::NEG_Y, 40.0, true, &ground)?;
        Some(origin - Vec3::Y * hit.distance + Vec3::Y * 0.15)
    };
    let free = |p: Vec3| !taken.iter().any(|t| t.distance(p) < 15.0);
    let pp = player.translation;
    let fwd = player.rotation * Vec3::Z;
    let fwd2 = Vec2::new(fwd.x, fwd.z).normalize_or(Vec2::Y);
    let yaw = |d: Vec2| d.x.atan2(d.y);
    let node = |i: usize| Vec3::from(graph.nodes[i].position);
    match kind {
        SpawnKind::Generic => {
            for _ in 0..GENERIC_TRIES {
                let theta = police.random() * std::f32::consts::TAU;
                let q = pp + Vec3::new(theta.sin(), 0.0, theta.cos()) * r;
                // The ring point snapped to the nearest road-graph node, then the nearest traffic-lane point within
                // 60 m and 5 m of height, facing along the lane (`FUN_0051b4e0` → `FUN_005f3350`).
                let Some(n) = graph.nearest_node(q.to_array(), false) else { continue };
                let p = node(n);
                if p.distance(q) > GENERIC_SNAP || graph.nodes[n].links.is_empty() {
                    continue;
                }
                let (p, dir) = match traffic {
                    Some(t) => {
                        let Some(lane) = t.nearest_lane(p, GENERIC_SNAP, Some(GENERIC_DY), false) else { continue };
                        (lane.position, lane.direction)
                    }
                    None => {
                        // MODEL (no traffic data): along one of the node's links at random.
                        let links = &graph.nodes[n].links;
                        let l = links[(police.random() * links.len() as f32) as usize % links.len()] as usize;
                        (p, node(l) - p)
                    }
                };
                let flat = Vec2::new(p.x - pp.x, p.z - pp.z).length();
                if flat < r || !free(p) {
                    continue;
                }
                let Some(at) = drop(p) else { continue };
                return Some((at, yaw(Vec2::new(dir.x, dir.z)), PATROL_SPAWN_MPH * MPH));
            }
            None
        }
        SpawnKind::Ahead | SpawnKind::Behind => {
            // `FUN_0051ca20`: along ±forward until past 1.5 r² horizontally; a node between r and √2·r, facing
            // back along the walk (towards the player).
            let dir = if kind == SpawnKind::Ahead { fwd2 } else { -fwd2 };
            let mut found = None;
            walk(graph, pp, dir, |n, _, prev| {
                let p = node(n);
                let h2 = (p.x - pp.x).powi(2) + (p.z - pp.z).powi(2);
                let d2 = p.distance_squared(pp);
                if d2 >= r * r && d2 <= 2.0 * r * r {
                    let back = node(prev) - p;
                    found = Some((p, yaw(Vec2::new(back.x, back.z))));
                    return false;
                }
                h2 <= 1.5 * r * r
            });
            let (p, heading) = found?;
            if !free(p) {
                return None;
            }
            Some((drop(p)?, heading, spawn_speed))
        }
        SpawnKind::AheadAlong => {
            // `FUN_0051c610`: along the player's route until farther than r, facing along it.
            let mut found = None;
            walk(graph, pp, fwd2, |n, _, prev| {
                let p = node(n);
                if p.distance(pp) > r {
                    let on = p - node(prev);
                    found = Some((p, yaw(Vec2::new(on.x, on.z))));
                    return false;
                }
                true
            });
            let (p, heading) = found?;
            if !free(p) {
                return None;
            }
            Some((drop(p)?, heading, spawn_speed))
        }
        SpawnKind::Side => {
            // `FUN_0051fd10`: a junction 100–250 m along the route; 20 m down a branch off it, facing along the
            // walk no more than 120° from the player's forward, at least 100 m away and out of the player's sight.
            let mut route = Vec::new();
            walk(graph, pp, fwd2, |n, along, _| {
                route.push((n, along));
                along < SIDE_ROAD_MAX
            });
            let on_route: Vec<usize> = route.iter().map(|r| r.0).collect();
            let sight = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
            for (k, &(j, along)) in route.iter().enumerate() {
                if !(SIDE_ROAD_MIN..=SIDE_ROAD_MAX).contains(&along) || graph.nodes[j].links.len() <= 2 {
                    continue;
                }
                let before = if k > 0 { Some(route[k - 1].0) } else { None };
                for &b in &graph.nodes[j].links {
                    let b = b as usize;
                    if on_route.contains(&b) || Some(b) == before {
                        continue;
                    }
                    // Down the branch 20 m.
                    let (mut cur, mut prev) = (b, j);
                    let mut depth = node(j).distance(node(b));
                    while depth < SIDE_ROAD_DEPTH {
                        let d = node(cur) - node(prev);
                        let Some(n) = graph.next_node(cur, [d.x, d.z], Some(prev)) else { break };
                        depth += node(cur).distance(node(n));
                        prev = cur;
                        cur = n;
                    }
                    if depth < SIDE_ROAD_DEPTH * 0.5 {
                        continue;
                    }
                    let p = node(cur);
                    let walked = Vec2::new(p.x - node(prev).x, p.z - node(prev).z).normalize_or_zero();
                    if walked.dot(fwd2) < -0.5 || p.distance(pp) < SIDE_ROAD_MIN || !free(p) {
                        continue;
                    }
                    let eye = pp + Vec3::Y;
                    let to = p + Vec3::Y - eye;
                    let Ok(dir) = Dir3::new(to) else { continue };
                    if spatial.cast_ray(eye, dir, to.length(), true, &sight).is_none() {
                        continue;
                    }
                    // MODEL: the waiting cop faces back towards the junction it will pull out of (the spawner's
                    // facing was not read).
                    return Some((drop(p)?, yaw(-walked), 0.0));
                }
            }
            None
        }
    }
}

/// Choose the spawn kind (`FUN_005261d0`): outside a chase the generic one; in a chase by the game mode's weights
/// gated by the cop type's flags, a region without cops counting double (`FUN_0051ce70`).
fn spawn_kind(police: &mut Police, cop_type: u32, player: &Transform, cops: &[Vec3]) -> SpawnKind {
    let Some(cfg) = police.cfg.as_ref() else { return SpawnKind::Generic };
    if police.pursuit.state != PursuitState::Chase {
        return SpawnKind::Generic;
    }
    let flags = cfg.cop_flags.get(&cop_type).copied().unwrap_or_default();
    let [w_side, w_ahead, w_behind, w_wait] = cfg.spawn_weights;
    let fwd = player.rotation * Vec3::Z;
    let (mut ahead, mut side, mut behind) = (0, 0, 0);
    for c in cops {
        let to = *c - player.translation;
        let d = to.length();
        if d <= REGION_NEAR {
            continue;
        }
        let cos = (to / d).dot(fwd);
        if cos > REGION_COS {
            ahead += 1;
        } else if cos >= -REGION_COS {
            side += 1;
        } else {
            behind += 1;
        }
    }
    let x2 = |n: i32| if n == 0 { 2.0 } else { 1.0 };
    let side_w = if flags.side { w_side * x2(side) } else { 0.0 };
    let wait_w = if flags.wait { w_wait * x2(side) } else { 0.0 };
    let ahead_w = if flags.ahead || flags.ahead_along { w_ahead * x2(ahead) } else { 0.0 };
    let behind_w = if flags.behind { w_behind * x2(behind) } else { 0.0 };
    let total = side_w + wait_w + ahead_w + behind_w;
    if total <= 0.0 {
        return SpawnKind::Generic;
    }
    let mut r = police.random() * total;
    for (w, kind) in [
        (side_w, SpawnKind::Side),
        // MODEL: the waiting spawn (`FUN_00522830`, weight 0 in free roam) is not ported; it falls to a side road.
        (wait_w, SpawnKind::Side),
        (ahead_w, if flags.ahead_along { SpawnKind::AheadAlong } else { SpawnKind::Ahead }),
        (behind_w, SpawnKind::Behind),
    ] {
        if r < w {
            return kind;
        }
        r -= w;
    }
    SpawnKind::Behind
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
    traffic: Option<Res<crate::traffic::TrafficSystem>>,
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
        let entry = police.orders[order].entry;
        // `AIPlayerType.SpawnSpeed` (mph; `FUN_005220b0`).
        let spawn_speed = cfg.cop_spawn_speed.get(&entry.cop_type).copied().unwrap_or(0.0).max(0.0) * MPH;
        let mut kind = spawn_kind(police, entry.cop_type, player, &taken);
        let lanes = traffic.as_deref();
        let mut found = spawn_point(police, &graph, kind, player, distance, spawn_speed, &taken, &spatial, lanes);
        if found.is_none() && police.spawn_distance_override.is_some() {
            // The test places cops close by, anywhere on the ring.
            kind = SpawnKind::Generic;
            found = spawn_point(police, &graph, kind, player, distance, spawn_speed, &taken, &spatial, lanes);
        }
        let Some((start, heading, mut speed)) = found else {
            debug!("police: no {kind:?} spawn point yet for cop {slot}");
            waiting.push((slot, spec, parts));
            continue;
        };
        taken.push(start);
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
        driver.keep_wrecked = true;
        // Free-roam spawns patrol (command 12 = the cop type's default, `FUN_006b8a40`); pursuit spawns are
        // handed their behaviour by the coordinator; side-road ones wait (command 16).
        let mode = match kind {
            SpawnKind::Generic => CopMode::Patrol,
            SpawnKind::Side => CopMode::Waiting,
            _ => CopMode::Chase,
        };
        commands.entity(car).insert((
            driver,
            CopStatus {
                activity: CopActivity::Patrol,
                driver_state: 0xd,
                patrol_mode: Some(0),
                chasing: false,
                in_pursuit: false,
                sees_player: false,
                wrecked: false,
            },
            Cop {
                slot,
                cop_type: entry.cop_type,
                mode,
                sight: Perception::default(),
                patrol: PatrolMode::default(),
                speed_mph: entry.speed_mph,
                lateral: police.orders[order].lateral,
                driver: CopDriver::new(Assigned::from_behaviour(entry.behaviour)),
                wanted: true,
                route_timer: 0.0,
                mode_time: 0.0,
                planned: None,
                direct: false,
                blocked: 0,
                search_radius: 0.0,
                leg: Leg::default(),
                crashed: false,
                touching: false,
            },
        ));
        police.orders[order].entity = Some(car);
        police.spawned += 1;
        let d = start.distance(player.translation);
        police.spawn_log.push((kind, d));
        info!(
            "police: cop {slot} (type {}, {}) placed {kind:?} at {start:.0?}, {d:.0} m from the player, {:.0} mph",
            entry.cop_type,
            spec.bundle,
            speed / MPH
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

/// The direct path (`FUN_004ee230`): the straight line from the cop to its aim point, continued 200 m (ours, so
/// that the racer's driver does not stop at the line's end).
fn direct_route(from: Vec3, to: Vec3) -> AiRoute {
    let d = Vec3::new(to.x - from.x, 0.0, to.z - from.z).normalize_or(Vec3::Z);
    AiRoute::new(vec![from, to, to + d * 200.0], false)
}

fn set_route(driver: &mut AiDriver, route: AiRoute) {
    driver.route = Arc::new(route);
    driver.finished = None;
    driver.time = 0.0;
    driver.progress = 0.0;
    driver.corner_speeds = None;
}

/// Every fixed tick, before the AI drivers: what each cop sees, what it does about it, where it drives and how
/// fast.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn cop_brains(
    mut commands: Commands,
    time: Res<Time>,
    mut police: ResMut<Police>,
    input: Res<CarInput>,
    spatial: SpatialQuery,
    player: Query<(Entity, &Transform, &LinearVelocity, &Car), With<Player>>,
    mut cops: Query<
        (
            Entity,
            &Transform,
            &mut LinearVelocity,
            &Car,
            &mut Cop,
            &mut AiDriver,
            Has<crate::car_impacts::AiCrash>,
            &crate::drive::CarControls,
            &mut AngularVelocity,
            &mut CopStatus,
        ),
        Without<Player>,
    >,
    mut traffic: Option<ResMut<crate::traffic::TrafficSystem>>,
    frustum: Query<&Frustum, crate::MainCamera>,
) {
    let dt = time.delta_secs();
    let police = &mut *police;
    let (Some(cfg), Some(graph)) = (police.cfg.as_ref(), police.graph.clone()) else { return };
    let Ok((player_entity, pt, pv, pcar)) = player.single() else { return };
    let Some(level) = police.pursuit.level(cfg).cloned() else { return };
    let cop_cfg = police.cop_cfg;
    let player_pos = pt.translation;
    let player_mph = pv.0.length() / MPH;
    let pursued = police.pursuit.in_chase_or_cooldown();
    let chase = police.pursuit.state == PursuitState::Chase;
    let world = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let target = CarState::from_transform(pt, pv.0);
    police.idle_input = if input.throttle > 0.0 || input.brake > 0.0 { 0.0 } else { police.idle_input + dt };

    // Sight (`FUN_006c5810`, `FUN_006c6280`) and contact, per cop.
    let player_hits = traffic.as_mut().map_or(0, |t| t.take_hits(player_entity));
    if pursued {
        police.owed_heat += cfg.traffic_hit_heat * player_hits as f32;
    }
    let mut notices = Vec::new();
    for (_, t, v, car, mut cop, _, crashed, _, _, _) in &mut cops {
        let d = t.translation.distance(player_pos);
        // A cop that crashes next to the player was taken down (`FUN_006ccd20` → `FUN_006b8d00`; the cop
        // type's takedown scoring action adds heat). MODEL: "by the player" = within 30 m of the player's car.
        if crashed && !cop.crashed && d < 30.0 {
            police.taken_down = true;
            police.owed_heat += cfg.takedown_heat.get(&cop.cop_type).copied().unwrap_or(0.0);
            info!("police: cop {} taken down", cop.slot);
        }
        cop.crashed = crashed;
        // MODEL: contact = the cop's centre within the player's box grown by the cop's half width. A new contact
        // adds its closing speed to the cop's ram sum; in a chase every new contact is a "SLAM" (scoring action
        // 955544, 0.2 heat: the original's heat rose by 0.2 at each cop contact in the capture, the poster was not
        // traced).
        let touch = touching(pt, pcar, t, car);
        let new_touch = touch && !cop.touching;
        if new_touch && pursued {
            police.owed_heat += cfg.slam_heat;
            police.slams += 1;
        }
        cop.touching = touch;
        let input = SightInput {
            dt,
            cop_position: t.translation,
            cop_forward: t.rotation * Vec3::Z,
            chasing: cop.mode == CopMode::Chase,
            // MODEL: the coordinator's "alerted" record flags (`FUN_00516400`) are taken as the cop's part in a
            // pursuit (going to the last-seen point, searching or waiting).
            alert: pursued && matches!(cop.mode, CopMode::GoTo | CopMode::Search | CopMode::Waiting),
            target: player_pos,
            target_mph: player_mph,
            crashed,
            contact: new_touch.then(|| (pv.0 - v.0).length()),
            ram_threshold: cop_cfg.ram_threshold,
        };
        let was_rammed = cop.sight.rammed;
        cop.sight.tick(&level, &input, || {
            let eye = t.translation + Vec3::Y;
            Dir3::new(player_pos + Vec3::Y - eye)
                .map_or(true, |dir| spatial.cast_ray(eye, dir, d, true, &world).is_none())
        });
        if cop.sight.rammed && !was_rammed {
            info!("police: cop {} rammed by the player", cop.slot);
        }
        notices.push(CopNotice { in_cone: cop.sight.in_cone, rammed: cop.sight.rammed });
    }
    // `FUN_005192b0`: wanted. The coordinator knows where the player is while any cop does.
    let was_wanted = police.wanted.reason;
    let mut wanted = police.wanted.tick(dt, &level, &notices, player_mph, player_hits, pursued);
    if police.force_wanted && !wanted {
        wanted = true;
        police.wanted.reason = Some(crate::cop_sight::WantedReason::Forced);
    }
    if police.wanted.reason != was_wanted && police.wanted.reason.is_some() {
        info!(
            "police: the player is wanted: {:?} ({player_mph:.0} mph, {player_hits} traffic hits)",
            police.wanted.reason
        );
    }
    let known = police.force_wanted || cops.iter().any(|c| c.4.sight.knows());
    let last_seen = cops
        .iter()
        .filter_map(|c| Some((c.4.sight.seen_age, c.4.sight.last_seen?)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|c| c.1);
    let cooldown_time = police.pursuit.cooldown_time;
    let player_crashed = input.crashed;

    // The cooldown's GoTo cop (`FUN_0051a4e0`): MODEL: the nearest cop to the last-seen point (the chooser
    // `FUN_0051a3f0` was not read).
    let goto_cop = if police.pursuit.state == PursuitState::Cooldown && !cops.iter().any(|c| c.4.mode == CopMode::GoTo)
    {
        last_seen.and_then(|ls| {
            cops.iter()
                .filter(|c| !c.6 && matches!(c.4.mode, CopMode::Search | CopMode::Patrol | CopMode::Chase))
                .min_by(|a, b| a.1.translation.distance(ls).total_cmp(&b.1.translation.distance(ls)))
                .map(|c| c.0)
        })
    } else {
        None
    };
    // Searching cops per sector (`FUN_0051a000`).
    let mut sectors = [0u32; SEARCH_SECTORS];
    for c in &cops {
        if c.4.mode == CopMode::Search {
            sectors[c.4.leg.sector] += 1;
        }
    }
    // The box-in (`FUN_00520f10`): a cop blocking in the low-speed chase pulls every other into it.
    let blocking =
        cops.iter().filter(|c| !c.6 && c.4.mode == CopMode::Chase && c.4.driver.in_low_speed_block()).count();
    if blocking > 0 {
        let low = cops.iter().filter(|c| matches!(c.4.driver.state, DriveState::LowSpeed { .. })).count();
        police.box_in += dt * low as f32;
    } else {
        police.box_in = 0.0;
    }
    let boxed = police.box_in > BOX_IN_TIME;

    let on_screen_at = {
        let f = frustum.iter().next().cloned();
        move |p: Vec3| on_screen(f.as_ref(), p)
    };
    for (entity, t, mut v, car, mut cop, mut driver, crashed, controls, mut w, mut status) in &mut cops {
        driver.overrides = Default::default();
        if crashed {
            if let Some(tr) = traffic.as_mut() {
                cop.patrol.leave(tr);
            }
            status.wrecked = true;
            status.chasing = false;
            continue;
        }
        let cop = &mut *cop;
        let before = cop.mode;
        // The coordinator's states (`FUN_005240a0`, `FUN_005255f0`, `FUN_0051a4e0`).
        cop.mode = match cop.mode {
            // While chased and known, every cop drives its behaviour.
            _ if chase && known && wanted && cop.mode != CopMode::Waiting => CopMode::Chase,
            // Before any pursuit a cop that sees a wanted player chases.
            CopMode::Patrol | CopMode::Search if cop.sight.knows() && wanted => CopMode::Chase,
            CopMode::Waiting => {
                // WaitingInterceptor: until the player comes within 50 m or moves away.
                let to = (player_pos - t.translation).normalize_or_zero();
                let leaving = to.dot(v.0 - pv.0) < 0.0;
                if !pursued || leaving || t.translation.distance(player_pos) < WAIT_RELEASE {
                    if chase { CopMode::Chase } else { CopMode::Patrol }
                } else {
                    CopMode::Waiting
                }
            }
            _ if Some(entity) == goto_cop => CopMode::GoTo,
            // `FUN_005156c0`: the target is no longer known.
            CopMode::Chase if !known => {
                if pursued {
                    CopMode::Search
                } else {
                    CopMode::Patrol
                }
            }
            CopMode::GoTo if !pursued => CopMode::Patrol,
            CopMode::GoTo if known || cop.mode_time > level.goto_timeout => CopMode::Search,
            CopMode::Patrol if police.pursuit.state == PursuitState::Cooldown && cooldown_time < COOLDOWN_JOIN => {
                CopMode::Search
            }
            CopMode::Search if !pursued => CopMode::Patrol,
            m => m,
        };
        cop.mode_time += dt;
        cop.route_timer -= dt;
        if cop.mode != before {
            debug!("police: cop {} {:?} -> {:?}", cop.slot, before, cop.mode);
            cop.mode_time = 0.0;
            cop.route_timer = 0.0;
            cop.planned = None;
            if cop.mode == CopMode::Chase {
                cop.driver.reassign();
            }
            if cop.mode == CopMode::Search {
                cop.search_radius = 0.0;
                cop.leg = Leg { target: t.translation, sector: 0, still: SEARCH_STUCK + 1.0 };
            }
            // Leaving Patrolling releases the ghost (`FUN_00506120`); a cop that searched starts its patrol at a
            // random point 150 m away (+0x97, set by Searching when it hands over, 0x50de32).
            if before == CopMode::Patrol
                && let Some(tr) = traffic.as_mut()
            {
                cop.patrol.leave(tr);
            }
            if cop.mode == CopMode::Patrol {
                cop.patrol = PatrolMode::Find { fails: 0, wander: matches!(before, CopMode::Search | CopMode::GoTo) };
            }
        }
        let fwd = t.rotation * Vec3::Z;
        let me = CarState::from_transform(t, v.0);
        let speed = v.0.length();
        driver.state.lane_offset = 0.0;
        match cop.mode {
            CopMode::Chase => {
                // `FUN_005240a0`: the low-speed chase near a slow (or crashed) player, and back.
                let gap = t.translation.distance(player_pos);
                let near_slow = (player_mph < level.low_speed_mph
                    && gap < cop_cfg.low_speed_range
                    && (t.translation.y - player_pos.y).abs() < LOW_SPEED_HEIGHT)
                    || player_crashed;
                let low = DriveState::LowSpeed { sub: 0, timer: 0.0, behind: false, boxed: false };
                let interceptor = cop.driver.assigned == Assigned::Interceptor;
                match cop.driver.state {
                    DriveState::Chasing { .. } | DriveState::Bruiser { .. } if near_slow => cop.driver.state = low,
                    // The box-in pulls the other chasing cops in (not an Interceptor, `FUN_00511870`).
                    DriveState::Chasing { .. } | DriveState::Bruiser { .. } | DriveState::Rhino
                        if blocking > 0 && !interceptor =>
                    {
                        cop.driver.state = low
                    }
                    DriveState::LowSpeed { .. }
                        if player_mph > level.low_speed_mph || (gap > cop_cfg.low_speed_range && !player_crashed) =>
                    {
                        cop.driver.reassign();
                    }
                    _ => {}
                }
                if let DriveState::LowSpeed { boxed: b, .. } = &mut cop.driver.state {
                    *b = boxed;
                }
                // `FUN_00518cf0`: a Rhino behind its target becomes a Bruiser.
                if cop.driver.state == DriveState::Rhino && (t.translation - player_pos).dot(target.forward) < 0.0 {
                    cop.driver.state = DriveState::Bruiser { sub: 1 };
                }
                // The world queries of the chase core (MODEL: physics rays for the game's nav-mesh queries).
                let mut probes = Probes { forward_hit: f32::MAX, ..Default::default() };
                let ray = |from: Vec3, dir: Vec3, len: f32| {
                    Dir3::new(dir).ok().and_then(|d| spatial.cast_ray(from, d, len, true, &world)).map(|h| h.distance)
                };
                let eye = t.translation + Vec3::Y * LINE_HEIGHT;
                let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::Z);
                probes.forward_hit = ray(eye, flat_fwd, cop_cfg.probe_forward).unwrap_or(f32::MAX);
                let a = cop_cfg.probe_angle_deg.to_radians();
                probes.side_hit = [a, -a]
                    .iter()
                    .any(|&r| ray(eye, Quat::from_rotation_y(r) * flat_fwd, cop_cfg.probe_side).is_some());
                // The aim point first (the line test needs it); the line test, then the tick again with it.
                let mut probe_driver = cop.driver.clone();
                let mut inp = CopInput {
                    dt,
                    cfg: &cop_cfg,
                    cop: me,
                    target,
                    human: true,
                    bust_meter: police.pursuit.bust_meter,
                    idle_input: police.idle_input,
                    lateral: cop.lateral,
                    probes,
                };
                let first = probe_driver.tick(&inp);
                if first.direct {
                    let to = first.aim + Vec3::Y * LINE_HEIGHT - eye;
                    inp.probes.line_clear = ray(eye, to, to.length()).is_none();
                }
                let out = cop.driver.tick(&inp);
                driver.overrides = out.overrides;
                cop.blocked = out.blocked;
                cop.direct = out.direct && inp.probes.line_clear;
                if cop.direct {
                    // `FUN_004ee230`: the straight path to the aim point.
                    set_route(&mut driver, direct_route(t.translation, out.aim));
                    cop.planned = None;
                } else {
                    // `FUN_00508fd0`: a road path to the aim point, planned again when the target's point 0.5 s
                    // ahead is more than 10 m off it.
                    let pred = player_pos + target.forward * pv.0.length() * 0.5;
                    let off = {
                        let (s, lat) = driver.route.project(pred, None, 0.0);
                        let end = driver.route.length() - s;
                        lat.abs() > REPLAN_DISTANCE || end < 30.0
                    };
                    let near_end = driver.route.length() - driver.progress < 60.0;
                    if cop.planned.is_none() || off || near_end {
                        let heading = if pv.0.length() > 2.0 { pv.0 } else { target.forward };
                        if let Some(route) =
                            route_via(&graph, t.translation, out.aim, heading).filter(|r| r.length() > 30.0)
                        {
                            set_route(&mut driver, route);
                            cop.planned = Some(out.aim);
                        } else {
                            set_route(&mut driver, direct_route(t.translation, out.aim));
                            cop.planned = None;
                        }
                    }
                }
                // The speed (`FUN_005043d0`): the state's target, the cop's cap (driver +0x5860, `FUN_004f76c0`),
                // 10 mph while turning round (`FUN_005005a0`).
                let mut v_target = out.target_speed;
                if cop.speed_mph > 0.0 {
                    v_target = v_target.min(cop.speed_mph * MPH);
                }
                let (s_now, _) = driver.route.project(t.translation, None, 0.0);
                let (ahead_point, _) = driver.route.at(s_now + 10.0);
                if (ahead_point - t.translation).dot(fwd) < 0.0
                    && speed <= TURN_ROUND_BELOW_MPH * MPH
                    && controls.steer.abs() >= 0.99
                {
                    v_target = v_target.min(TURN_ROUND_MPH * MPH);
                }
                driver.schedule_speed = v_target;
                // `FUN_004fa750`: nitrous.
                let lead = (t.translation - player_pos).dot(target.forward);
                let gap2 = Vec2::new(t.translation.x - player_pos.x, t.translation.z - player_pos.z).length();
                let outside = lead < NITROUS_LEAD[0] || lead > NITROUS_LEAD[1] || gap2 > NITROUS_RANGE;
                if driver.overrides.nitrous.is_none() {
                    driver.overrides.nitrous = Some(outside && speed + NITROUS_GAP < v_target);
                }
            }
            CopMode::GoTo | CopMode::Search => {
                // GoTo: the last-seen point; Search: legs around the player (`vt_0050dd40`, `FUN_0051a000`).
                if cop.mode == CopMode::Search {
                    cop.leg.still = if speed < 1.0 { cop.leg.still + dt } else { 0.0 };
                    let arrived = cop.leg.target.distance(t.translation) < SEARCH_ARRIVED;
                    if arrived || cop.leg.still > SEARCH_STUCK {
                        let sector = (0..SEARCH_SECTORS).min_by_key(|&k| sectors[k]).unwrap_or(0);
                        sectors[sector] += 1;
                        let width = std::f32::consts::TAU / SEARCH_SECTORS as f32;
                        let theta = (sector as f32 + police.random()) * width;
                        cop.search_radius += SEARCH_STEP;
                        let p = player_pos + Vec3::new(theta.sin(), 0.0, theta.cos()) * cop.search_radius;
                        let p =
                            graph.nearest_node(p.to_array(), false).map_or(p, |n| Vec3::from(graph.nodes[n].position));
                        cop.leg = Leg { target: p, sector, still: 0.0 };
                        cop.planned = None;
                    }
                }
                let goal = if cop.mode == CopMode::GoTo { last_seen.unwrap_or(player_pos) } else { cop.leg.target };
                if cop.planned.is_none_or(|p| p.distance(goal) > 1.0) || cop.route_timer <= 0.0 {
                    cop.route_timer = 2.0;
                    if let Some(route) = route_via(&graph, t.translation, goal, fwd).filter(|r| r.length() > 30.0) {
                        set_route(&mut driver, route);
                        cop.planned = Some(goal);
                    }
                }
                // `FUN_004f76c0`: the level's go-to / search speed limit.
                driver.schedule_speed = level.search_speed_mph * MPH;
            }
            CopMode::Patrol if police.test_hold => {
                driver.schedule_speed = 0.0;
                driver.overrides.handbrake = Some(true);
                driver.overrides.throttle = Some(0.0);
                // The brake pedal at a standstill selects reverse: brake only while rolling.
                driver.overrides.brake = Some(if speed > 1.0 { 1.0 } else { 0.0 });
            }
            CopMode::Patrol => {
                let Some(tr) = traffic.as_mut() else {
                    // MODEL: without traffic (`--no-traffic`) there are no lanes: the patrol drives the road graph at
                    // the level's go-to / search speed.
                    let near_end = driver.route.length() - driver.progress < 150.0;
                    if near_end || before != CopMode::Patrol {
                        if let Some(route) =
                            crate::ai::route_from_ai_graph(&graph, t.translation, fwd.x.atan2(fwd.z), 2000.0)
                        {
                            set_route(&mut driver, route);
                        }
                    }
                    driver.schedule_speed = level.search_speed_mph * MPH;
                    continue;
                };
                let input = PatrolInput {
                    dt,
                    cop: entity,
                    position: t.translation,
                    forward: fwd,
                    velocity: v.0,
                    player: player_pos,
                    on_screen: on_screen_at(t.translation),
                    half_length: car.params.body_half_extents.z,
                    random: police.random(),
                };
                match cop.patrol.tick(&input, tr) {
                    PatrolDrive::Stop => {
                        driver.schedule_speed = 0.0;
                        driver.overrides.throttle = Some(0.0);
                        driver.overrides.brake = Some(if speed > 1.0 { 1.0 } else { 0.0 });
                        driver.overrides.handbrake = Some(speed <= 1.0);
                    }
                    PatrolDrive::Road { target, speed, handbrake } => {
                        if cop.planned.is_none_or(|p| p.distance(target) > 1.0) {
                            let route = route_via(&graph, t.translation, target, fwd)
                                .filter(|r| r.length() > 30.0)
                                .unwrap_or_else(|| direct_route(t.translation, target));
                            set_route(&mut driver, route);
                            cop.planned = Some(target);
                        }
                        driver.schedule_speed = speed.min(level.search_speed_mph * MPH);
                        if handbrake {
                            driver.overrides.handbrake = Some(true);
                        }
                    }
                    PatrolDrive::Path { path, speed } => {
                        if cop.route_timer <= 0.0 && path.len() > 1 {
                            cop.route_timer = 0.5;
                            set_route(&mut driver, AiRoute::new(path, false));
                        }
                        cop.planned = None;
                        driver.schedule_speed = speed;
                    }
                    PatrolDrive::Ride { pose, path, snap } => {
                        if cop.route_timer <= 0.0 && path.len() > 1 {
                            cop.route_timer = 0.5;
                            set_route(&mut driver, AiRoute::new(path, false));
                        }
                        cop.planned = None;
                        driver.schedule_speed = pose.speed;
                        driver.overrides.throttle = Some(0.0);
                        // Brake 1 while the ghost stands (`FUN_0050d490`); ours holds it on the handbrake, as the
                        // brake pedal at a standstill selects reverse in our car model.
                        driver.overrides.brake = Some(0.0);
                        driver.overrides.handbrake = Some(pose.speed.abs() < 0.01);
                        let rotation = Quat::from_mat3(&Mat3::from_cols(
                            pose.up.cross(pose.forward).normalize_or(Vec3::X),
                            pose.up,
                            pose.forward,
                        ));
                        if snap {
                            let at = pose.position + Vec3::Y * 0.15;
                            commands.entity(entity).insert((Position(at), Rotation(rotation)));
                            v.0 = pose.forward * pose.speed;
                            w.0 = Vec3::ZERO;
                        } else {
                            // Physics message 0x2e sets the body to the ghost's pose and speed. MODEL: ours steers
                            // the body there by velocity in the ground plane and yaw, leaving height, pitch and roll
                            // to the suspension (a hard set fights the wheel model).
                            let next = pose.position + pose.forward * pose.speed * dt;
                            let err = next - t.translation;
                            v.0.x = err.x / dt;
                            v.0.z = err.z / dt;
                            let a = Vec2::new(fwd.x, fwd.z).normalize_or(Vec2::Y);
                            let b = Vec2::new(pose.forward.x, pose.forward.z).normalize_or(a);
                            // Yaw about +Y from a to b (x = sin, z = cos convention).
                            let yaw = b.x.atan2(b.y) - a.x.atan2(a.y);
                            let yaw =
                                (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                            w.0.y = yaw / dt;
                        }
                    }
                }
            }
            CopMode::Waiting => {
                driver.schedule_speed = 0.0;
                driver.overrides.handbrake = Some(true);
            }
        }
    }
    // MODEL: every cop car that is not riding a ghost is an obstacle for the lane followers, as crashed cars are
    // (the game's obstacle scan over the AI vehicles, `FUN_00511540`, was not read for cops).
    if let Some(tr) = traffic.as_mut() {
        let obstacles = cops
            .iter()
            .filter(|c| !matches!(c.4.patrol, PatrolMode::Ride { .. }) || c.4.mode != CopMode::Patrol || c.6)
            .map(|c| (c.0, c.1.translation, c.2.0, c.1.rotation * Vec3::X))
            .collect();
        tr.set_cop_obstacles(obstacles);
    }
    // The public state.
    for (_, _, _, _, cop, _, crashed, _, _, mut status) in &mut cops {
        let driver_state = match cop.mode {
            CopMode::Patrol => 0xd,
            CopMode::Chase => cop.driver.state_id(),
            CopMode::GoTo => 4,
            CopMode::Search => 0xc,
            CopMode::Waiting => 0x11,
        };
        let next = CopStatus {
            activity: match cop.mode {
                CopMode::Patrol => CopActivity::Patrol,
                CopMode::Chase => CopActivity::Chase,
                CopMode::GoTo => CopActivity::GoTo,
                CopMode::Search => CopActivity::Search,
                CopMode::Waiting => CopActivity::Waiting,
            },
            driver_state,
            patrol_mode: (cop.mode == CopMode::Patrol).then(|| cop.patrol.number()),
            chasing: !crashed && matches!(driver_state, 0 | 0x10 | 0x12 | 0x16 | 0x17 | 0x18 | 0x1a),
            in_pursuit: pursued && cop.mode != CopMode::Patrol,
            sees_player: cop.sight.sees,
            wrecked: crashed,
        };
        if *status != next {
            *status = next;
        }
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
        .filter(|c| c.3.sight.knows())
        .filter_map(|c| Some((c.3.sight.seen_age, c.3.sight.last_seen?)))
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
            cop_taken_down: std::mem::take(&mut police.taken_down),
            // Not wired: our crash rules have no wrecker (`crash.rs`), and there are no roadblocks.
            wrecked_by_cop: false,
            roadblock_near: false,
        },
    );
    let mut events = events;
    let owed = std::mem::take(&mut police.owed_heat);
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
    // The outcome's sequence plays (13 s; the busted one with the car's controls off), then free roam goes on
    // (HSM 457517 "Do Pursuits": after a bust the heat is reset and every cop removed, `RemoveAllCops`).
    if matches!(police.pursuit.state, PursuitState::Busted | PursuitState::Escaped) {
        let busted = police.pursuit.state == PursuitState::Busted;
        if busted {
            // "Disable Car Controls and Easydrive".
            input.scripted = Some([0.0, 1.0, 0.0, 1.0]);
        }
        police.outcome += dt;
        if police.outcome >= OUTCOME_SECONDS {
            police.outcome = 0.0;
            police.pursuit.reset(busted);
            if busted {
                input.scripted = None;
                for (e, _, _, cop, _) in &cops {
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
    traffic: Option<Res<crate::traffic::TrafficSystem>>,
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
            "  cop {} (type {}) {:?}/{:#x} {:?}: {:.0} m away, {:.0} mph (target {:.0}), sees {} memory {:.1}, direct {} blocked {:#b}, route {:.0}/{:.0} m, recovering {}, lateral {:+.1}, patrol mode {}{}",
            cop.slot,
            cop.cop_type,
            cop.mode,
            cop.driver.state_id(),
            cop.driver.state,
            t.translation.distance(pt.translation),
            v.0.length() / MPH,
            driver.schedule_speed.min(999.0) / MPH,
            cop.sight.sees,
            cop.sight.memory,
            cop.direct,
            cop.blocked,
            driver.progress,
            driver.route.length(),
            driver.state.recovering.is_some(),
            cop.lateral,
            cop.patrol.number(),
            match (cop.patrol.ghost(), traffic.as_ref()) {
                (Some(g), Some(tr)) => tr.ghost_peek(g).map_or(" (ghost gone)".to_string(), |p| format!(
                    " (ghost {:.1} m {}, {:.0} mph)",
                    p.position.distance(t.translation),
                    if (p.position - t.translation).dot(t.rotation * Vec3::Z) > 0.0 { "ahead" } else { "behind" },
                    p.speed / MPH
                )),
                _ => String::new(),
            }
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
///
/// Measured against the original's pursuit capture (research log 2026-10-14): the bust meter's fill time at a
/// standstill, the heat's rise per second at level 1, the cops' approach speed and closest distance.
#[derive(Resource, Default)]
struct PursuitTest {
    phase: u8,
    timer: f32,
    /// Smallest and largest distance of the nearest cop during the chase (m), the highest heat.
    nearest: [f32; 2],
    max_heat: f32,
    chase_seconds: f32,
    cooldown_seconds: f32,
    /// The bust meter's first time above 0 and at 1 (test time, s).
    bust_start: Option<f32>,
    /// The heat at the chase's start and 10 s in, for the rate.
    heat_rate: (Option<f32>, Option<f32>),
    /// Fastest cop speed while chasing (mph).
    top_cop_mph: f32,
    /// Where the player's car started: the bust run starts there again.
    start: Option<Transform>,
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
    mut player: Query<(Entity, &mut Transform, &mut LinearVelocity), (With<Player>, With<Car>)>,
    cops: Query<(Entity, &Transform, &LinearVelocity, &Cop), Without<Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    let police = &mut *police;
    let Ok((me, mut pt_mut, mut pv_mut)) = player.single_mut() else { return };
    let (pt, pv) = (*pt_mut, *pv_mut);
    let (pt, pv) = (&pt, &pv);
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
            test.start = Some(*pt);
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
            test.heat_rate = (Some(p.heat), None);
        }
        // The chase: cops follow; then they are taken away.
        2 => {
            if let Some(d) = nearest {
                test.nearest = [test.nearest[0].min(d), test.nearest[1].max(d)];
            }
            for (_, _, v, cop) in &cops {
                if cop.mode == CopMode::Chase {
                    test.top_cop_mph = test.top_cop_mph.max(v.0.length() / MPH);
                }
            }
            if test.timer >= 10.0 && test.heat_rate.1.is_none() {
                test.heat_rate.1 = Some(p.heat);
            }
            test.max_heat = test.max_heat.max(p.heat);
            if test.timer >= TEST_CHASE_SECONDS || p.state != PursuitState::Chase {
                info!(
                    "TEST RESULT pursuit chase: {:.0} s, state {:?}, {} cops ({} chasing), nearest cop {:.0} to {:.0} m, fastest chasing cop {:.0} mph, heat {:.2} (level {}), {} slams, bust meter {:.2}, player {:.0} mph",
                    test.timer,
                    p.state,
                    cops.iter().count(),
                    p.cops_chasing,
                    test.nearest[0],
                    test.nearest[1],
                    test.top_cop_mph,
                    p.heat,
                    p.level_number(&cfg),
                    police.slams,
                    p.bust_meter,
                    pv.0.length() / MPH
                );
                test.chase_seconds = test.timer;
                police.hold_spawns = true;
                police.force_wanted = false;
                for (e, _, _, cop) in &cops {
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
            // Back to the start (the chase may have left the car anywhere).
            if let Some(s) = test.start.take() {
                *pt_mut = s;
                // Position / Rotation too: a Transform write alone does not always move a body.
                commands.entity(me).insert((Position(s.translation), Rotation(s.rotation)));
                pv_mut.0 = Vec3::ZERO;
            }
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
            test.bust_start = None;
        }
        7 => {
            // The fill that busts: from the meter's last time at 0.
            if p.bust_meter <= 0.0 {
                test.bust_start = None;
            }
            if p.bust_meter > 0.0 && test.bust_start.is_none() {
                test.bust_start = Some(test.timer);
            }
            if p.state == PursuitState::Busted {
                let fill = test.bust_start.map_or(-1.0, |s| test.timer - s);
                info!(
                    "TEST RESULT pursuit outcome 2: BUSTED {:.1} s after the chase began, the meter filled in {fill:.1} s (bust time {} s within {} m; the original: 4.0 s), nearest cop {:.0} m, heat {:.2}, {} busts",
                    test.timer,
                    cfg.bust_time,
                    cfg.bust_distance,
                    nearest.unwrap_or(-1.0),
                    p.heat,
                    p.busts
                );
                if let (Some(a), Some(b)) = test.heat_rate {
                    info!(
                        "TEST RESULT pursuit heat: {:.4}/s over the chase's first 10 s (the original at level 1: 0.0067/s from time, plus 0.2 per cop contact, 0.1 per traffic hit, 1 per takedown)",
                        (b - a) / 10.0
                    );
                }
                test.phase = 8;
                test.timer = 0.0;
            }
        }
        8 if p.state == PursuitState::None => {
            info!(
                "TEST RESULT pursuit: {} chases, {} escapes, {} busts, {} cops spawned, {} removed, {} left, heat {:.2}, spawns {:?}",
                p.chases,
                p.escapes,
                p.busts,
                police.spawned,
                police.removed,
                cops.iter().count(),
                p.heat,
                police.spawn_log
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
