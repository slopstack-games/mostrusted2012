//! The cop coordinator's rules (the AI coordinator `FUN_00526c30`, its record = `*(AI + 4) + 0xc`): the behaviour
//! sequencer, the roadblock task, how many cops of which type, where they come from, when they go, and what each
//! cop is told to do. docs/decomp.md "Police and pursuit (ported)".

use glam::{Vec2, Vec3};
use mw_assets::police::{Behaviour, CopSlot, CopSpawnFlags, HeatLevel};

/// `.rdata` 0x00fa9f50 / 0x00fa9f54: the lateral slot of a chasing cop (m; `FUN_00518cf0`).
pub const SLOT_LATERAL: f32 = 3.5;
/// `.rdata` 0x00fa2cc0: seconds without new cops after a pursuit has ended (`FUN_005286d0`).
pub const SPAWN_BLOCK: f32 = 90.0;
/// The coordinator checks the cop count once a second and spawns at most 3 cops per pass (`FUN_00528b40`,
/// `FUN_00526e30`: `local_6c < 3`).
pub const COORDINATOR_PERIOD: f32 = 1.0;
pub const SPAWNS_PER_PASS: usize = 3;
/// `FUN_0051ce70`: spawn regions ahead / side / behind by the cosine 0.707 to the player's forward, cops within
/// 20 m not counted.
const REGION_COS: f32 = 0.707;
const REGION_NEAR: f32 = 20.0;
/// The search (`FUN_004f7af0`, `FUN_0051a000`, `vt_0050dd40`): legs in five 72° sectors (0x010c44c4), each 20 m
/// (0x00fa9a0c) wider than the last, around the player; a leg ends at its point (20 m) or after 3 s still.
pub const SEARCH_SECTORS: usize = 5;
pub const SEARCH_STEP: f32 = 20.0;
pub const SEARCH_ARRIVED: f32 = 20.0;
pub const SEARCH_STUCK: f32 = 3.0;
/// `FUN_00658160`: the cooldown's first 7 s (`.rdata` 0x00fb74ec), or any time from heat level 2 on.
pub const COOLDOWN_JOIN: f32 = 7.0;
/// WaitingInterceptor (state 0x11, `FUN_004fbff0`): a waiting cop leaves when the player closes within 50 m
/// (`DAT_010c4330` = 2500) or moves away from it.
pub const WAIT_RELEASE: f32 = 50.0;
/// `AIConfig` +0x1a8 (`aef30d56`, 600 m) and +0x1ac (`5cf26e60`, 300 m): the roadblock trigger window along the
/// player's route (`FUN_005246f0`); the data values, read by the runtime with the rest of `AIConfig`.
pub const ROADBLOCK_FAR: f32 = 600.0;
pub const ROADBLOCK_NEAR: f32 = 300.0;
/// `.rdata` 0x00f8a810: a box trigger's bounding radius is half its size's length (`FUN_0042c0b0`).
pub const HALF: f32 = 0.5;

/// The behaviour sequencer (coordinator +0x6b4: +0x50 time, +0x54 phase, +0x58 the heat level it runs for, +0x60
/// the behaviour, +0x64 the cycle index, +0x68 the event count; `FUN_0051f010`).
#[derive(Debug, Clone)]
pub struct Sequencer {
    pub time: f32,
    pub phase: Phase,
    pub cycle_index: i32,
    pub events: u32,
    level: Option<u32>,
}

/// [`Sequencer::phase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// 0: the main behaviour (`Coordination` +0x30).
    #[default]
    Main,
    /// 1: a cycle behaviour (+0x58 + index × 0x24).
    Cycle,
    /// 2: the fallback (+0x0c).
    Fallback,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self { time: 0.0, phase: Phase::Main, cycle_index: -1, events: 0, level: None }
    }
}

impl Sequencer {
    /// The behaviour running now.
    pub fn behaviour<'a>(&self, level: &'a HeatLevel) -> &'a Behaviour {
        match self.phase {
            Phase::Main => &level.main,
            Phase::Fallback => &level.fallback,
            Phase::Cycle => {
                usize::try_from(self.cycle_index).ok().and_then(|k| level.cycle.get(k)).unwrap_or(&level.main)
            }
        }
    }

    /// An event the fallback counts (+0x68; its writer was not traced: the runtime's hook).
    pub fn count_event(&mut self) {
        self.events += 1;
    }

    fn restart(&mut self) {
        self.time = 0.0;
        self.events = 0;
    }

    /// `FUN_00514d90`: start a phase.
    fn enter(&mut self, level: &HeatLevel, phase: Phase) {
        self.phase = phase;
        if phase == Phase::Cycle {
            self.next_cycle(level);
        }
        self.restart();
    }

    /// The next cycle behaviour, or the main one without any.
    fn next_cycle(&mut self, level: &HeatLevel) {
        if level.cycle.is_empty() {
            self.phase = Phase::Main;
        } else {
            self.phase = Phase::Cycle;
            self.cycle_index = (self.cycle_index + 1).rem_euclid(level.cycle.len() as i32);
        }
    }

    /// One frame (`FUN_0051f010` → `FUN_00514e40`; only while the player has a car).
    pub fn tick(&mut self, level: &HeatLevel, dt: f32) {
        self.time += dt;
        if self.level != Some(level.id) {
            // A new heat level: the same phase again with its behaviours.
            self.level = Some(level.id);
            self.cycle_index = -1;
            self.enter(level, self.phase);
        }
        let b = self.behaviour(level).clone();
        let counted = b.fallback_after != 0 && self.events >= u32::from(b.fallback_after);
        match self.phase {
            Phase::Main | Phase::Cycle => {
                if self.time < b.duration {
                    if counted {
                        self.phase = Phase::Fallback;
                        self.restart();
                    }
                } else {
                    self.next_cycle(level);
                    self.restart();
                }
            }
            Phase::Fallback => {
                if self.time > b.duration {
                    self.phase = Phase::Main;
                    self.restart();
                }
                let now = self.behaviour(level);
                if b.fallback_after != 0 && self.events >= u32::from(now.fallback_after) {
                    self.restart();
                }
            }
        }
    }
}

/// A roadblock trigger region (`TriggerData` kind 0x12, collected by `FUN_0051bd50`).
#[derive(Debug, Clone, Copy)]
pub struct TriggerRegion {
    pub position: Vec3,
    /// `FUN_0042c0b0`: a box's half size length (its size × 0.5), a sphere's radius.
    pub radius: f32,
}

/// What the roadblock task reads of the player and the world.
#[derive(Debug, Clone, Copy)]
pub struct RoadblockQuery<'a> {
    /// The player is chased (`FUN_006beaa0`).
    pub chasing: bool,
    pub player: Vec3,
    /// The player's predicted route: road-graph node positions from the player on (coordinator +0x31c + 0x84 i).
    pub route: &'a [Vec3],
    pub triggers: &'a [TriggerRegion],
}

/// The roadblock task (`FUN_00526150`; task +0x8 timer, +0xc triggers collected, +0x10 the behaviour).
#[derive(Debug, Clone, Default)]
pub struct RoadblockTask {
    pub timer: f32,
    behaviour: Option<(u32, Phase, i32)>,
}

impl RoadblockTask {
    /// One frame: the behaviour's first delay when it changes, then on the timer `FUN_005246f0` along the player's
    /// predicted route (node positions from the player on). Returns the trigger to place the roadblock at.
    pub fn tick(&mut self, dt: f32, seq: &Sequencer, level: &HeatLevel, q: &RoadblockQuery) -> Option<(usize, u32)> {
        let RoadblockQuery { chasing, player, route, triggers } = *q;
        let b = seq.behaviour(level);
        let key = (level.id, seq.phase, seq.cycle_index);
        if self.behaviour != Some(key) {
            self.behaviour = Some(key);
            self.timer = b.roadblock_delay;
        }
        self.timer -= dt;
        if self.timer > 0.0 || !chasing || b.roadblock == 0 || route.len() < 2 {
            return None;
        }
        // Segments from the route's start until a segment begins 600 m away; triggers near a segment that begins
        // 300 m or more away.
        let mut hit = None;
        for w in route.windows(2) {
            let d = w[0].distance(player);
            if d >= ROADBLOCK_FAR {
                break;
            }
            if d < ROADBLOCK_NEAR {
                continue;
            }
            for (k, t) in triggers.iter().enumerate() {
                if closest_on_segment(w[0], w[1], t.position).distance_squared(t.position) <= t.radius * t.radius {
                    // `FUN_005226b0` posts the roadblock (world message 7) and the timer restarts.
                    self.timer = b.roadblock_repeat;
                    hit.get_or_insert((k, b.roadblock));
                }
            }
        }
        hit
    }
}

/// `FUN_005130c0`: the point of segment a–b closest to p.
pub fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    a + ab * t
}

/// `FUN_00518cf0`: the lateral slot of the behaviour's entry `index`: the first cop of a positioning −1 … 1 to one
/// side, the others to the other; other positionings none.
pub fn lateral_slot(behaviour: &Behaviour, index: usize) -> f32 {
    let Some(entry) = behaviour.cops.get(index) else { return 0.0 };
    if !(-1..=1).contains(&entry.positioning) {
        return 0.0;
    }
    let first = !behaviour.cops[..index].iter().any(|c| c.positioning == entry.positioning);
    if first { SLOT_LATERAL } else { -SLOT_LATERAL }
}

/// Where a new cop comes from (`FUN_005261d0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnKind {
    /// Outside a chase: on the ring, patrolling (`FUN_005227a0`, `FUN_0051b4e0`).
    Generic,
    /// Ahead of the player, oncoming (mode 1).
    Ahead,
    /// Ahead on the player's route, facing the same way (mode 0x10).
    AheadAlong,
    /// Behind the player (mode 2).
    Behind,
    /// Waiting in a side road (mode 8, WaitingInterceptor).
    Side,
    /// Waiting (parked) cop (`FUN_00522830`; weight 0 in free roam).
    Wait,
}

/// `FUN_005261d0`: the kind of a cop of `flags`, by the game mode's weights (`spawn_weights` side / ahead /
/// behind / wait) and the cops around the player (a region without cops counts double, `FUN_0051ce70`).
pub fn spawn_kind(
    chase: bool,
    flags: CopSpawnFlags,
    weights: [f32; 4],
    player: Vec3,
    forward: Vec3,
    cops: &[Vec3],
    random: f32,
) -> SpawnKind {
    if !chase {
        return SpawnKind::Generic;
    }
    let [w_side, w_ahead, w_behind, w_wait] = weights;
    let (mut ahead, mut side, mut behind) = (0, 0, 0);
    for c in cops {
        let to = *c - player;
        let d = to.length();
        if d <= REGION_NEAR {
            continue;
        }
        let cos = (to / d).dot(forward);
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
    let mut r = random * total;
    for (w, kind) in [
        (side_w, SpawnKind::Side),
        (wait_w, SpawnKind::Wait),
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

/// What the coordinator has a cop do (its view of the driver state; ids from `FUN_004efc40`'s table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    /// Patrolling (0xd): driving in traffic.
    Patrol,
    /// A pursuit state (`driver`).
    Chase,
    /// GoTo (4): to the point the coordinator sent it to (`FUN_004f8d80`).
    GoTo { target: Vec3 },
    /// Searching (0xc): legs around the player.
    Search,
    /// WaitingInterceptor (0x11): in a side road for the player.
    Waiting,
}

impl Role {
    pub fn state_id(&self) -> u8 {
        match self {
            Self::Patrol => 0xd,
            Self::Chase => 0,
            Self::GoTo { .. } => 4,
            Self::Search => 0xc,
            Self::Waiting => 0x11,
        }
    }
}

/// A search leg (`vt_0050dd40`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Leg {
    pub target: Vec3,
    pub sector: usize,
    pub still: f32,
    pub radius: f32,
}

impl Leg {
    /// A fresh search: the first leg is asked at once.
    pub fn start(at: Vec3) -> Self {
        Self { target: at, sector: 0, still: SEARCH_STUCK + 1.0, radius: 0.0 }
    }

    /// One tick: the next leg when this one is done, in the sector with the fewest searching cops
    /// (`FUN_0051a000`), 20 m wider than the last, around the player. The runtime snaps the point to the road
    /// graph. Returns true when a new leg began.
    pub fn tick(
        &mut self,
        dt: f32,
        position: Vec3,
        speed: f32,
        player: Vec3,
        sectors: &mut [u32],
        random: f32,
    ) -> bool {
        self.still = if speed < 1.0 { self.still + dt } else { 0.0 };
        if self.target.distance(position) >= SEARCH_ARRIVED && self.still <= SEARCH_STUCK {
            return false;
        }
        let sector = (0..sectors.len()).min_by_key(|&k| sectors[k]).unwrap_or(0);
        if let Some(s) = sectors.get_mut(sector) {
            *s += 1;
        }
        let width = core::f32::consts::TAU / SEARCH_SECTORS as f32;
        let theta = (sector as f32 + random) * width;
        self.radius += SEARCH_STEP;
        self.target = player + Vec3::new(theta.sin(), 0.0, theta.cos()) * self.radius;
        self.sector = sector;
        self.still = 0.0;
        true
    }
}

/// What the role decision reads for one cop.
#[derive(Debug, Clone, Copy)]
pub struct RoleInput {
    /// A chase runs; a chase or cooldown runs.
    pub chase: bool,
    pub pursued: bool,
    /// The player is wanted (`FUN_005192b0`); some cop knows where the player is (coordinator +0x400 + i); this cop
    /// knows.
    pub wanted: bool,
    pub known: bool,
    pub cop_knows: bool,
    /// Waiting: the player is within 50 m, or moves away from the cop.
    pub player_near: bool,
    pub player_leaving: bool,
    /// The cooldown's join window (`FUN_00658160`) holds.
    pub join_window: bool,
    /// GoTo: the driver reached its point (`FUN_004fbfa0`) or the go-to time ran past the level's timeout.
    pub goto_done: bool,
}

/// The coordinator's role switches for one cop (`FUN_005240a0`, `FUN_005255f0`, `FUN_0051a4e0`, `FUN_005156c0`).
/// The GoTo assignment itself is [`Coordinator`](crate::Police)'s (one cop per cooldown).
pub fn next_role(role: Role, i: &RoleInput) -> Role {
    match role {
        // While chased and known, every cop drives its behaviour.
        r if i.chase && i.known && i.wanted && r != Role::Waiting => Role::Chase,
        // A cop that knows a wanted player chases.
        Role::Patrol | Role::Search if i.cop_knows && i.wanted => Role::Chase,
        Role::Waiting => {
            if !i.pursued || i.player_leaving || i.player_near {
                if i.chase { Role::Chase } else { Role::Patrol }
            } else {
                Role::Waiting
            }
        }
        // `FUN_005156c0`: the target is no longer known.
        Role::Chase if !i.known => {
            if i.pursued {
                Role::Search
            } else {
                Role::Patrol
            }
        }
        Role::GoTo { .. } if !i.pursued => Role::Patrol,
        Role::GoTo { .. } if i.goto_done => Role::Search,
        Role::Patrol if i.pursued && !i.chase && i.join_window => Role::Search,
        Role::Search if !i.pursued => Role::Patrol,
        r => r,
    }
}

/// `FUN_00526e30`'s count for one cop type of the behaviour: how many entries want it.
pub fn wanted_of_type(behaviour: &Behaviour, cop_type: u32) -> usize {
    behaviour.cops.iter().filter(|c| c.cop_type == cop_type).count()
}

/// The entries of the behaviour still to be spawned this pass (`FUN_00526e30`): per type, wanted − (pending +
/// living uncrashed), at most 3 in all; only while the behaviour's `f3d7aeeb` holds. `have(type)` counts the
/// pending orders and living cops of a type.
pub fn missing(behaviour: &Behaviour, have: impl Fn(u32) -> usize) -> Vec<(usize, CopSlot)> {
    let mut out = Vec::new();
    if !behaviour.spawn_allowed {
        return out;
    }
    let mut types: Vec<u32> = behaviour.cops.iter().map(|c| c.cop_type).collect();
    types.sort_unstable();
    types.dedup();
    for ty in types {
        if out.len() >= SPAWNS_PER_PASS {
            break;
        }
        let want = wanted_of_type(behaviour, ty);
        let n = want.saturating_sub(have(ty)).min(SPAWNS_PER_PASS - out.len());
        // The entries of the type from the last one held: the ones not yet on the road.
        let entries: Vec<(usize, CopSlot)> =
            behaviour.cops.iter().copied().enumerate().filter(|(_, c)| c.cop_type == ty).collect();
        out.extend(entries.into_iter().rev().take(n));
    }
    out
}

/// The horizontal distance.
pub fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}
