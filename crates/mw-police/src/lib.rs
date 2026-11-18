//! The original's police and pursuit logic, ported from its code (docs/decomp.md "Police and pursuits" for the
//! readings, "Police and pursuit (ported)" for this crate and its runtime wiring). Engine-free and wasm-safe: the
//! runtime gives the player's and the cops' states and the road data as plain values every fixed tick and acts on
//! what comes back (spawns, removals, roadblocks, roles, the cop drivers' control records, events).
//!
//! - [`pursuit`]: the pursuit manager (`FUN_0066ae60`): heat, chase / cooldown / busted / escaped, bust meter,
//!   pursuit bar, the scoring actions ([`score`]).
//! - [`sight`]: a cop's perception (`FUN_006c5810`) and the wanted test (`FUN_005192b0`).
//! - [`coordinator`]: the AI coordinator (`FUN_00526c30`): sequencer, roadblock task, cop count, spawn kinds,
//!   removal, roles, search legs.
//! - [`driver`]: the cop driver states (Chasing, LowSpeedChasing, Bruiser, Rhino, HeadOn).
//! - [`patrol`]: the Patrolling state on a traffic ghost (`FUN_005101e0`).
//! - [`roadblock`]: where a roadblock's items stand (the roadblock manager, `FUN_00844ff0` / `FUN_00841710`).
//! - [`Police`]: one tick of all of it for the local player.

pub mod coordinator;
pub mod driver;
pub mod heat;
pub mod patrol;
pub mod pursuit;
pub mod roadblock;
pub mod score;
pub mod sight;

use glam::Vec3;
use mw_assets::police::{CopSlot, HeatLevel, PoliceConfig};

use coordinator::{Leg, RoadblockTask, Role, RoleInput, Sequencer, SpawnKind, TriggerRegion};
use driver::{Assigned, ChaseContext, CopConfig, CopDriver};
use pursuit::{CopView, Pursuit, PursuitEvent, PursuitInput, PursuitMode, PursuitState, Spotter};
use sight::{CopNotice, Perception, SightInput, Wanted, WantedReason};

pub use glam;

/// m/s per mph.
pub const MPH: f32 = 0.44704;
/// `.data` 0x01106228: a patrol cop's spawn speed (`FUN_00522590`; 20 mph, not verified by a live read).
pub const PATROL_SPAWN_MPH: f32 = 20.0;
/// The box-in (`FUN_00520f10`): one second after a cop blocks, the low-speed cops creep in.
pub const BOX_IN_TIME: f32 = 1.0;
/// MODEL: an order whose car never arrives is forgotten after this long (the game's spawn tasks fail at once).
pub const ORDER_TIMEOUT: f32 = 60.0;

/// The player as the police sees it.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerState {
    pub position: Vec3,
    pub forward: Vec3,
    pub velocity: Vec3,
    /// Signed speed (mph; vehicle +0x1620).
    pub speed_mph: f32,
    /// The player's car has crashed (the low-speed chase closes in).
    pub crashed: bool,
    /// Wrecked by an AI car (`FUN_0065c880`).
    pub wrecked_by_ai: bool,
    /// Traffic cars the player's car hit this tick (the list at traffic world +0x33144).
    pub traffic_hits: u32,
    /// The player made a cop crash this tick (`rec + 0x8a4`).
    pub took_down_cop: bool,
    /// The player's car touches a roadblock's car (`FUN_0065b930`).
    pub touches_roadblock: bool,
    /// Seconds without throttle or brake input (the low-speed chase's creep, state +0x90).
    pub idle_input: f32,
    /// The player's vehicle +0x1a84 is 0 (cooldown × 4; false for the player's car).
    pub fast_cooldown: bool,
}

/// A cop car on the road, as the runtime sees it this tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct CopState {
    /// The id the cop was ordered with ([`SpawnRequest::id`]), or any unique id for a cop the runtime placed.
    pub id: u32,
    /// Its `Gameplay.CopType`.
    pub cop_type: u32,
    pub position: Vec3,
    pub forward: Vec3,
    pub velocity: Vec3,
    pub crashed: bool,
    /// The cop's 4 m sphere is in the camera's view (`FUN_006bf380`).
    pub on_screen: bool,
    /// The cop's car touches the player's.
    pub touching: bool,
    /// A new contact with the player's car this tick and its strength (the ram sum; MODEL: closing speed m/s).
    pub contact: Option<f32>,
    /// The ray from the cop to the player is clear (the perception's raycast).
    pub line_clear: bool,
    /// GoTo: the driver reached its point (`FUN_004fbfa0`).
    pub arrived: bool,
}

/// The road data of the tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoadState<'a> {
    /// The roadblocks standing (the roadblock manager's records, world +0x2f484, at most 2).
    pub roadblocks: &'a [Vec3],
    /// The player's predicted route: road-graph node positions from the player on (coordinator +0x31c).
    pub route: &'a [Vec3],
    /// The roadblock trigger regions (`TriggerData` kind 0x12).
    pub roadblock_triggers: &'a [TriggerRegion],
}

/// One tick's inputs.
#[derive(Debug, Clone, Copy)]
pub struct PoliceInput<'a> {
    pub dt: f32,
    pub player: PlayerState,
    pub cops: &'a [CopState],
    pub road: RoadState<'a>,
    pub mode: PursuitMode,
    /// Heat of the other scoring actions the runtime posted this tick (takedowns, SLAM, traffic hits, roadblock
    /// smashes: each action's `1a88cb4c`, `FUN_006d08a0`).
    pub scoring_heat: f32,
    /// The caller forces the player wanted (tests, demos).
    pub force_wanted: bool,
}

/// A cop to spawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnRequest {
    pub id: u32,
    pub slot: CopSlot,
    /// The car (`VehicleList` id, `CopType` → `AIPlayerType.Rollout` → `Rollout.Vehicle`), when known.
    pub vehicle: Option<u32>,
    pub kind: SpawnKind,
    /// The ring: between `distance` and √2 × `distance` from the player for the chase spawns (`FUN_00513e80`), at
    /// least `distance` for the generic one (`FUN_0051b4e0`).
    pub distance: f32,
    /// Spawn speed (mph): the type's `SpawnSpeed` in a chase, 20 mph for a patrol.
    pub speed_mph: f32,
    /// The lateral slot (`FUN_00518cf0`).
    pub lateral: f32,
}

/// What a tick asks of the runtime.
#[derive(Debug, Clone, Default)]
pub struct PoliceOutput {
    pub events: Vec<PursuitEvent>,
    pub spawns: Vec<SpawnRequest>,
    /// Cops to remove (ids).
    pub removals: Vec<u32>,
    /// A roadblock to place: the trigger region's index and the roadblock definition (`FUN_005226b0`, message 7).
    pub roadblock: Option<(usize, u32)>,
    /// Cops whose role changed: id, before, after.
    pub roles: Vec<(u32, Role, Role)>,
    /// A new search leg for these cops (the runtime snaps the point to the road graph).
    pub legs: Vec<u32>,
}

/// One cop's brain.
#[derive(Debug, Clone)]
pub struct Cop {
    pub id: u32,
    pub cop_type: u32,
    pub slot: CopSlot,
    /// Coordinator record +0x14.
    pub lateral: f32,
    pub role: Role,
    pub driver: CopDriver,
    pub sight: Perception,
    /// Record +0x24: the behaviour still wants the cop (surplus cops go at 100 m).
    pub wanted: bool,
    pub leg: Leg,
    pub role_time: f32,
}

impl Cop {
    /// `FUN_006bbb70` for the pursuit manager: the driver in a chasing state while the role is a chase.
    pub fn chasing(&self) -> bool {
        self.role == Role::Chase && self.driver.chasing_state()
    }
}

#[derive(Debug, Clone)]
struct Order {
    id: u32,
    slot: CopSlot,
    lateral: f32,
    kind: SpawnKind,
    age: f32,
}

/// The police for the local player.
#[derive(Debug, Clone)]
pub struct Police {
    pub pursuit: Pursuit,
    pub wanted: Wanted,
    pub sequencer: Sequencer,
    pub roadblock: RoadblockTask,
    pub cops: Vec<Cop>,
    pub cop_cfg: CopConfig,
    orders: Vec<Order>,
    next_id: u32,
    timer: f32,
    /// No new cops for this long (after a pursuit; `FUN_005286d0`).
    pub spawn_block: f32,
    pub was_pursuit: bool,
    /// Tests: no new cops are ordered; the ring distance of the orders.
    pub hold_spawns: bool,
    pub spawn_distance_override: Option<f32>,
    /// Coordinator +0x666 (the chase last frame), +0x667 (a GoTo cop is owed), +0x724 (the GoTo time).
    was_chase: bool,
    goto_armed: bool,
    goto_time: f32,
    /// Coordinator +0x720: the box-in timer.
    box_in: f32,
    /// The bust state is left to the caller (the game's HSM action 0xf); by default [`PursuitEvent::BustCaught`]
    /// sets it at once.
    pub manual_bust: bool,
    rng: u64,
}

impl Default for Police {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Police {
    pub fn new(seed: u64) -> Self {
        Self {
            pursuit: Pursuit { radar: 1.0, ..Default::default() },
            wanted: Wanted::default(),
            sequencer: Sequencer::default(),
            roadblock: RoadblockTask::default(),
            cops: Vec::new(),
            cop_cfg: CopConfig::default(),
            orders: Vec::new(),
            next_id: 1,
            timer: 0.0,
            spawn_block: 0.0,
            was_pursuit: false,
            hold_spawns: false,
            spawn_distance_override: None,
            was_chase: false,
            goto_armed: false,
            goto_time: 0.0,
            box_in: 0.0,
            manual_bust: false,
            rng: seed,
        }
    }

    /// A uniform float in [0, 1) (the game's LCG, `FUN_005dfe90`: × 0x5851f42d4c957f2d + 1).
    pub fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x5851_f42d_4c95_7f2d).wrapping_add(1);
        ((self.rng >> 40) as f32) / (1u64 << 24) as f32
    }

    /// The player's heat level row (`FUN_006bdc00`).
    pub fn level<'a>(&self, cfg: &'a PoliceConfig) -> Option<&'a HeatLevel> {
        self.pursuit.level(cfg)
    }

    /// The runtime could not place the car of order `id` (no road): the order is dropped.
    pub fn cancel_order(&mut self, id: u32) {
        self.orders.retain(|o| o.id != id);
    }

    /// Drop every pending order (tests).
    pub fn clear_orders(&mut self) {
        self.orders.clear();
    }

    pub fn cop(&self, id: u32) -> Option<&Cop> {
        self.cops.iter().find(|c| c.id == id)
    }

    pub fn cop_mut(&mut self, id: u32) -> Option<&mut Cop> {
        self.cops.iter_mut().find(|c| c.id == id)
    }

    /// The distance triple's index: no pursuit, chase, cooldown (`FUN_00513e80`).
    fn phase(&self) -> usize {
        match self.pursuit.state {
            PursuitState::Chase => 1,
            PursuitState::Cooldown => 2,
            _ => 0,
        }
    }

    /// A new brain for a cop that appeared (an order's slot, or the level's first entry of its type).
    fn adopt(&mut self, c: &CopState, level: &HeatLevel) -> Cop {
        let order = self.orders.iter().position(|o| o.id == c.id).map(|k| self.orders.remove(k));
        let (slot, lateral, kind) = match order {
            Some(o) => (o.slot, o.lateral, o.kind),
            None => {
                let b = self.sequencer.behaviour(level);
                let k = b.cops.iter().position(|s| s.cop_type == c.cop_type);
                let slot = k.map(|k| b.cops[k]).unwrap_or(CopSlot {
                    cop_type: c.cop_type,
                    speed_mph: 0.0,
                    behaviour: 6,
                    positioning: 0,
                });
                (slot, k.map_or(0.0, |k| coordinator::lateral_slot(b, k)), SpawnKind::Generic)
            }
        };
        let role = match kind {
            SpawnKind::Generic => Role::Patrol,
            SpawnKind::Side | SpawnKind::Wait => Role::Waiting,
            _ => Role::Chase,
        };
        Cop {
            id: c.id,
            cop_type: c.cop_type,
            slot,
            lateral,
            role,
            driver: CopDriver::new(Assigned::from_behaviour(slot.behaviour)),
            sight: Perception::default(),
            wanted: true,
            leg: Leg::start(c.position),
            role_time: 0.0,
        }
    }

    /// One fixed tick.
    pub fn tick(&mut self, cfg: &PoliceConfig, i: &PoliceInput) -> PoliceOutput {
        let mut out = PoliceOutput::default();
        let dt = i.dt;
        let p = &i.player;
        let Some(level) = self.level(cfg).cloned() else { return out };
        // Brains for new cops; gone cops forgotten.
        for c in i.cops {
            if self.cop(c.id).is_none() {
                let brain = self.adopt(c, &level);
                self.cops.push(brain);
            }
        }
        self.cops.retain(|b| i.cops.iter().any(|c| c.id == b.id));
        let state = |id: u32| i.cops.iter().find(|c| c.id == id).copied().unwrap_or_default();
        let pursued = self.pursuit.in_chase_or_cooldown();
        let roadblock_near = i.road.roadblocks.iter().any(|r| r.distance(p.position) < pursuit::NEAR_RANGE);

        // Perception (`FUN_006c5810`) for every cop, then wanted (`FUN_005192b0`).
        let ram_threshold = self.cop_cfg.ram_threshold;
        let mut notices = Vec::with_capacity(self.cops.len());
        for cop in &mut self.cops {
            let s = state(cop.id);
            let chasing = cop.chasing();
            // MODEL: the alerted flag (`FUN_00516400`, coordinator bytes +0x1c / +0x1f, writer not traced) is a
            // pursuit with the cop going to a point, searching or waiting.
            let alert = pursued && matches!(cop.role, Role::GoTo { .. } | Role::Search | Role::Waiting);
            let si = SightInput {
                dt,
                cop_position: s.position,
                cop_forward: s.forward,
                chasing,
                alert,
                target: p.position,
                target_mph: p.speed_mph.abs(),
                crashed: s.crashed,
                contact: s.contact,
                ram_threshold,
                roadblock_near,
            };
            cop.sight.tick(&level, &si, || s.line_clear);
            notices.push(CopNotice { in_cone: cop.sight.in_cone, rammed: cop.sight.rammed });
        }
        let mut wanted = self.wanted.tick(dt, &level, &notices, p.speed_mph, p.traffic_hits, pursued);
        if i.force_wanted && !wanted {
            wanted = true;
            self.wanted.reason = Some(WantedReason::Forced);
        }
        let known = self.cops.iter().any(|c| c.sight.knows());
        let last_seen = self
            .cops
            .iter()
            .filter_map(|c| Some((c.sight.seen_age, c.sight.last_seen?)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|c| c.1);

        if i.mode.cops_active {
            self.coordinate(cfg, &level, i, wanted, known, &mut out);
        }

        // The pursuit manager (`FUN_0066ae60`).
        let views: Vec<CopView> = self
            .cops
            .iter()
            .map(|c| {
                let s = state(c.id);
                CopView {
                    position: s.position,
                    chasing: c.chasing(),
                    crashed: s.crashed,
                    excluded: false,
                    touching: s.touching,
                }
            })
            .collect();
        let spotter = self.cops.iter().find(|c| c.sight.knows()).map(|c| Spotter { wrecked: state(c.id).crashed });
        let pi = PursuitInput {
            dt,
            mode: i.mode,
            position: p.position,
            speed_mph: p.speed_mph,
            cops: &views,
            last_seen: if known { last_seen } else { None },
            hidden: !known,
            spotter,
            spotter_flag: false,
            took_down_cop: p.took_down_cop,
            touches_roadblock: p.touches_roadblock,
            roadblock_near,
            wrecked_by_ai: p.wrecked_by_ai,
            fast_cooldown: p.fast_cooldown,
        };
        let mut events = self.pursuit.tick(cfg, &pi);
        if !self.manual_bust && events.iter().any(|e| matches!(e, PursuitEvent::BustCaught { .. })) {
            self.pursuit.set_busted(&mut events);
        }
        self.pursuit.add_heat(cfg, &i.mode, i.scoring_heat, &mut events);
        out.events = events;
        out
    }

    /// The coordinator's frame (`FUN_00526c30`).
    fn coordinate(
        &mut self,
        cfg: &PoliceConfig,
        level: &HeatLevel,
        i: &PoliceInput,
        wanted: bool,
        known: bool,
        out: &mut PoliceOutput,
    ) {
        let dt = i.dt;
        let p = &i.player;
        let state = |id: u32| i.cops.iter().find(|c| c.id == id).copied().unwrap_or_default();
        let pursued = self.pursuit.in_chase_or_cooldown();
        let chase = self.pursuit.state == PursuitState::Chase;
        let cooldown = self.pursuit.state == PursuitState::Cooldown;

        // `FUN_00519040`: the chase's rising edge owes a GoTo cop for the next cooldown.
        if chase && !self.was_chase {
            self.goto_armed = true;
            self.goto_time = 0.0;
        }
        self.was_chase = chase;
        // `FUN_0051f010`: the behaviour sequencer; `FUN_00526150`: the roadblock task.
        self.sequencer.tick(level, dt);
        let q = coordinator::RoadblockQuery {
            chasing: chase,
            player: p.position,
            route: i.road.route,
            triggers: i.road.roadblock_triggers,
        };
        out.roadblock = self.roadblock.tick(dt, &self.sequencer, level, &q);

        // `FUN_0051a4e0`: in the cooldown's join window the nearest cop to the player goes where the player is.
        let join = cooldown && (self.pursuit.cooldown_time < coordinator::COOLDOWN_JOIN || level.level > 1);
        if join {
            if !self.goto_armed {
                self.goto_time += dt;
            }
            let has_goto = self.cops.iter().any(|c| matches!(c.role, Role::GoTo { .. }));
            if !has_goto && self.goto_armed {
                let pick =
                    self.cops.iter_mut().filter(|c| !state(c.id).crashed && c.role != Role::Waiting).min_by(|a, b| {
                        let da = state(a.id).position.distance_squared(p.position);
                        let db = state(b.id).position.distance_squared(p.position);
                        da.total_cmp(&db)
                    });
                if let Some(c) = pick {
                    let before = c.role;
                    c.role = Role::GoTo { target: p.position };
                    c.role_time = 0.0;
                    out.roles.push((c.id, before, c.role));
                    self.goto_armed = false;
                }
            }
        }

        // The box-in (`FUN_00520f10`).
        let blocking =
            self.cops.iter().any(|c| !state(c.id).crashed && c.role == Role::Chase && c.driver.in_low_speed_block());
        if blocking {
            let low =
                self.cops.iter().filter(|c| matches!(c.driver.state, driver::DriveState::LowSpeed { .. })).count();
            self.box_in += dt * low as f32;
        } else {
            self.box_in = 0.0;
        }
        let boxed = self.box_in > BOX_IN_TIME;

        // Roles and the chase states.
        let mut sectors = [0u32; coordinator::SEARCH_SECTORS];
        for c in &self.cops {
            if c.role == Role::Search {
                sectors[c.leg.sector] += 1;
            }
        }
        let goto_timeout = level.goto_timeout;
        let goto_time = self.goto_time;
        let cop_cfg = self.cop_cfg;
        let randoms: Vec<f32> = (0..self.cops.len()).map(|_| self.random()).collect();
        for (cop, r) in self.cops.iter_mut().zip(randoms) {
            let s = state(cop.id);
            if s.crashed {
                continue;
            }
            let to = (p.position - s.position).normalize_or_zero();
            let ri = RoleInput {
                chase,
                pursued,
                wanted,
                known,
                cop_knows: cop.sight.knows(),
                player_near: s.position.distance(p.position) < coordinator::WAIT_RELEASE,
                player_leaving: to.dot(s.velocity - p.velocity) < 0.0,
                join_window: join,
                goto_done: s.arrived || goto_time > goto_timeout,
            };
            let before = cop.role;
            cop.role = coordinator::next_role(cop.role, &ri);
            cop.role_time += dt;
            if cop.role != before {
                cop.role_time = 0.0;
                if cop.role == Role::Chase {
                    cop.driver.reassign();
                }
                if cop.role == Role::Search {
                    cop.leg = Leg::start(s.position);
                }
                out.roles.push((cop.id, before, cop.role));
            }
            match cop.role {
                Role::Chase => {
                    let ctx = ChaseContext {
                        player_mph: p.speed_mph.abs(),
                        low_speed_mph: level.low_speed_mph,
                        gap: s.position.distance(p.position),
                        dy: s.position.y - p.position.y,
                        player_crashed: p.crashed,
                        blocking,
                        boxed,
                        behind_target: (s.position - p.position).dot(p.forward) < 0.0,
                    };
                    cop.driver.coordinate(&cop_cfg, &ctx);
                }
                Role::Search if cop.leg.tick(dt, s.position, s.velocity.length(), p.position, &mut sectors, r) => {
                    out.legs.push(cop.id);
                }
                _ => {}
            }
        }

        // The pass once a second (`FUN_00528b40`): removal and the cop count.
        self.spawn_block = (self.spawn_block - dt).max(0.0);
        for o in &mut self.orders {
            o.age += dt;
        }
        self.orders.retain(|o| o.age < ORDER_TIMEOUT);
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = coordinator::COORDINATOR_PERIOD;
        self.roster(cfg, level, i, out);
    }

    /// Removal (`FUN_00522c40`, `FUN_00522dc0`, `FUN_00522f40`, `FUN_005286d0`) and the count (`FUN_00526e30`).
    fn roster(&mut self, cfg: &PoliceConfig, level: &HeatLevel, i: &PoliceInput, out: &mut PoliceOutput) {
        let p = &i.player;
        let state = |id: u32| i.cops.iter().find(|c| c.id == id).copied().unwrap_or_default();
        let phase = self.phase();
        let pursuit = self.pursuit.in_chase_or_cooldown();
        let behaviour = self.sequencer.behaviour(level).clone();

        // The behaviour's wanted cops per type, nearest first; the rest are surplus.
        let mut order: Vec<(f32, usize)> = self
            .cops
            .iter()
            .enumerate()
            .filter(|(_, c)| !state(c.id).crashed)
            .map(|(k, c)| (state(c.id).position.distance(p.position), k))
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kept: Vec<(u32, usize)> = Vec::new();
        for (_, k) in order {
            let ty = self.cops[k].cop_type;
            let want = coordinator::wanted_of_type(&behaviour, ty);
            let n = match kept.iter_mut().find(|e| e.0 == ty) {
                Some(e) => {
                    e.1 += 1;
                    e.1 - 1
                }
                None => {
                    kept.push((ty, 1));
                    0
                }
            };
            self.cops[k].wanted = n < want;
        }

        // Removal, only off screen: a crashed cop beyond its distance; in a chase a surplus cop beyond 100 m; else
        // beyond the phase's distance (horizontal). The first pass after a pursuit removes every cop off screen.
        let ended = self.was_pursuit && !pursuit;
        self.was_pursuit = pursuit;
        if ended {
            self.spawn_block = coordinator::SPAWN_BLOCK;
        }
        for c in &self.cops {
            let s = state(c.id);
            if s.on_screen {
                continue;
            }
            let far = if s.crashed {
                s.position.distance(p.position) >= level.crashed_remove_distance
            } else {
                let limit =
                    if phase == 1 && !c.wanted { level.surplus_remove_distance } else { level.remove_distance[phase] };
                coordinator::flat(s.position, p.position) > limit
            };
            if far || ended {
                out.removals.push(c.id);
            }
        }
        self.cops.retain(|c| !out.removals.contains(&c.id));

        if self.spawn_block > 0.0 || self.hold_spawns {
            return;
        }
        // The count: pending orders and living, uncrashed cops of the type.
        let living: Vec<u32> = self.cops.iter().filter(|c| !state(c.id).crashed).map(|c| c.cop_type).collect();
        let orders: Vec<u32> = self.orders.iter().map(|o| o.slot.cop_type).collect();
        let missing = coordinator::missing(&behaviour, |ty| {
            living.iter().filter(|&&t| t == ty).count() + orders.iter().filter(|&&t| t == ty).count()
        });
        let positions: Vec<Vec3> = i.cops.iter().map(|c| c.position).collect();
        for (index, slot) in missing {
            let flags = cfg.cop_flags.get(&slot.cop_type).copied().unwrap_or_default();
            let r = self.random();
            let kind = coordinator::spawn_kind(
                self.pursuit.state == PursuitState::Chase,
                flags,
                cfg.spawn_weights,
                p.position,
                p.forward,
                &positions,
                r,
            );
            let id = self.next_id;
            self.next_id += 1;
            let lateral = coordinator::lateral_slot(&behaviour, index);
            let speed_mph = if kind == SpawnKind::Generic {
                PATROL_SPAWN_MPH
            } else {
                cfg.cop_spawn_speed.get(&slot.cop_type).copied().unwrap_or(PATROL_SPAWN_MPH)
            };
            out.spawns.push(SpawnRequest {
                id,
                slot,
                vehicle: cfg.cop_vehicles.get(&slot.cop_type).copied(),
                kind,
                distance: self.spawn_distance_override.unwrap_or(level.spawn_distance[phase]),
                speed_mph,
                lateral,
            });
            self.orders.push(Order { id, slot, lateral, kind, age: 0.0 });
        }
    }
}
