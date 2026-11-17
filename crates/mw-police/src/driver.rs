//! How a cop drives in a pursuit: the game's cop driver states (docs/decomp.md "Police and pursuits", "Police and
//! pursuit (ported)"). The runtime gathers the cars' state and the world queries every fixed tick, calls
//! [`CopDriver::tick`] and hands the result to the AI racers' driver (the command builder `FUN_0050a740`, the pedal
//! controller and the emergency brake are the racer's, as in the game). [`CopDriver::coordinate`] is the
//! coordinator's part (`FUN_005240a0`, `FUN_00520f10`, `FUN_00518cf0`): which chase state a cop is in.
//!
//! States (driver state ids; vtables in `.rdata` 0x00fa8724 ..):
//! - **Chasing** (0, update `FUN_00510420` → chase core `FUN_0050e370`): aims at the target plus a lead and the
//!   cop's lateral slot (`FUN_004ffcb0`); within 40 m of a human target substate 2 aims 0.5 s ahead and may drive
//!   straight at it (`FUN_00505f90`, `FUN_00508fd0`). Flat out under the cop's speed cap (`FUN_004fa480` returns
//!   FLT_MAX for a cop with a physical car; the gap law `FUN_004fa580` only paces cops without one).
//! - **LowSpeedChasing** (0x16, `FUN_0050fe80`): near a slow player: block spin, creep, brake or ram substates
//!   (`FUN_004fc3c0`, `FUN_004fc810`, `FUN_004fcc10`).
//! - **ChasingBruiser** (0x18, `FUN_0050f5f0`): approach, line up alongside, steer into the car (`FUN_004facd0`,
//!   `FUN_005044d0`, `FUN_0050a900`).
//! - **Rhino** (0x12, `FUN_005102f0`): drives at the point 0.5 s ahead of the target, flat out.
//! - **HeadOn** (0x13, `FUN_0050b7f0`): entered by Chasing / Bruiser when the target closes head-on above 110 mph
//!   within 90 m (`FUN_004fab50`): handbrake and full lock to the target's side until it has passed.
//!
//! Signs: the game's steering is positive to the right; ours (`CarControls`) is positive to the left. A car's
//! "right" is its matrix row 0, taken as −X of our transform (inferred, decomp.md open points).

use glam::{Quat, Vec2, Vec3};

const MPH: f32 = 0.44704;

/// The `AIConfig` fields the cop states read (`GLOBALCONFIG.BNDL` 010000000007fb0e, by instance offset; the
/// defaults are the data's values, `police.rs` reads them from the data).
#[derive(Debug, Clone, Copy)]
pub struct CopConfig {
    /// 0x48: lead time of the aim point (s).
    pub lead_time: f32,
    /// 0x12c: longest lead (m).
    pub max_lead: f32,
    /// 0x50: within this the chase switches to substate 2 and the Bruiser lines up (m).
    pub close_range: f32,
    /// 0x64, 0x70, 0x68: the "misaligned" test of the direct path: heading error (deg), speed (mph), distance (m).
    pub misaligned_deg: f32,
    pub misaligned_mph: f32,
    pub misaligned_distance: f32,
    /// 0x6c, 0x74: the forward probe's distance (m) and speed (mph) that stop the direct path.
    pub probe_stop_distance: f32,
    pub probe_stop_mph: f32,
    /// 0x78, 0x7c, 0x80: forward probe length, side probe length (m), side probe angle (deg).
    pub probe_forward: f32,
    pub probe_side: f32,
    pub probe_angle_deg: f32,
    /// 0x84, 0x88: the Bruiser's alongside window along the target (m: −0x88 < a < 0x84).
    pub alongside_ahead: f32,
    pub alongside_behind: f32,
    /// 0x60 (`a7a0cd14`): a cop whose contact sum passes this was rammed (`FUN_006c5810`).
    pub ram_threshold: f32,
    /// 0xec: the low-speed chase's range (m).
    pub low_speed_range: f32,
    /// LowSpeedChasing: 0xf0 brake, 0xf8 lead time, 0xfc / 0x100 block (m), 0x104 / 0x10c behind (m), 0x108 ahead
    /// (m), 0x110 / 0x118 alongside (m), 0x114 slow target (mph), 0x11c block-spin steering.
    pub ls_brake: f32,
    pub ls_lead_time: f32,
    pub ls_block_along: f32,
    pub ls_block_lat: f32,
    pub ls_behind_along: f32,
    pub ls_behind_lat: f32,
    pub ls_ahead_along: f32,
    pub ls_alongside_along: f32,
    pub ls_alongside_lat: f32,
    pub ls_slow_mph: f32,
    pub ls_spin_steer: f32,
}

impl Default for CopConfig {
    fn default() -> Self {
        Self {
            lead_time: 1.0,
            max_lead: 50.0,
            close_range: 40.0,
            misaligned_deg: 20.0,
            misaligned_mph: 20.0,
            misaligned_distance: 12.0,
            probe_stop_distance: 30.0,
            probe_stop_mph: 25.0,
            probe_forward: 50.0,
            probe_side: 3.0,
            probe_angle_deg: 25.0,
            alongside_ahead: 0.0,
            alongside_behind: 2.0,
            ram_threshold: 2.0,
            low_speed_range: 60.0,
            ls_brake: 0.6,
            ls_lead_time: 1.0,
            ls_block_along: 1.2,
            ls_block_lat: 4.0,
            ls_behind_along: 5.0,
            ls_behind_lat: 0.5,
            ls_ahead_along: 7.0,
            ls_alongside_along: 3.0,
            ls_alongside_lat: 4.5,
            ls_slow_mph: 5.0,
            ls_spin_steer: 0.7,
        }
    }
}

/// `DAT_0110612c` (49.1744 m/s = 110 mph) and 90 m (8100 = 90²): the HeadOn switch (`FUN_004fab50`).
const HEAD_ON_CLOSING: f32 = 49.1744;
const HEAD_ON_RANGE: f32 = 90.0;
/// `.rdata` 0x00fa8ec0: the target counts as abeam of the cop below this |heading · direction| (`FUN_004ffcb0`).
const ABEAM: f32 = 0.2;
/// 0x00f8a810: the half-second lead of substate 2 and the Rhino.
const HALF: f32 = 0.5;
/// The heading history: one sample every 0.1 s, ten kept (`FUN_004ffbe0`).
const HISTORY_PERIOD: f32 = 0.1;
const HISTORY_LEN: usize = 10;
/// The Bruiser's alongside test: within 5 m to the target's side (`FUN_005044d0`).
const BRUISER_LATERAL: f32 = 5.0;
/// The Bruiser's ram steering: 0.2 per metre of the target's offset (`FUN_0050a900`); also the substate-1
/// steering of the chase (`FUN_004f83e0`, unused).
const RAM_GAIN: f32 = 0.2;
/// LowSpeedChasing (`FUN_004fc810`, `FUN_004fcc10`): only within 4 m of height; substate 3 holds 1.5 s; the
/// creep's 20 mph over 33 m and 7 m stop; 5 s without player input within 15 m; aim offsets 1 / 3 / 2 m.
const LS_HEIGHT: f32 = 4.0;
const LS_BRAKE_HOLD: f32 = 1.5;
const LS_CREEP_MPH: f32 = 20.0;
const LS_CREEP_RANGE: f32 = 33.0;
const LS_STOP: f32 = 7.0;
const LS_IDLE_INPUT: f32 = 5.0;
const LS_IDLE_RANGE: f32 = 15.0;

/// A cop's driver state (the game's state id in brackets).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriveState {
    /// Chasing (0): substate 0 or 2 (S+0x180).
    Chasing { sub: u8 },
    /// LowSpeedChasing (0x16): substate (+0x104), its timer (+0x100), the "directly behind" flag (+0x10c) of the
    /// last tick, and the box-in flag (+0xba).
    LowSpeed { sub: u8, timer: f32, behind: bool, boxed: bool },
    /// ChasingBruiser (0x18): substate 1 approach, 2 line up, 0 ram (+0x100).
    Bruiser { sub: u8 },
    /// Rhino (0x12).
    Rhino,
    /// HeadOn (0x13): the state to go back to, and the side picked on entry (game sign: +1 = right).
    HeadOn { back: BackState, side: f32 },
}

/// The state HeadOn returns to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackState {
    Chasing,
    Bruiser,
}

/// A cop's behaviour from its `CopBehaviours` entry (`FUN_005139a0`: 0 / 1 Chasing, 2 Bruiser, 3 Rhino, 4
/// Interceptor, 5 Idle, else keep the type's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assigned {
    Chasing,
    Bruiser,
    Rhino,
    /// MODEL: the Interceptor (0x1a, `vt_0050f970`) is not ported; it drives as Chasing. No free-roam level's
    /// main behaviour uses it (only their cycle behaviours, which are not run).
    Interceptor,
}

impl Assigned {
    pub fn from_behaviour(b: i32) -> Self {
        match b {
            2 => Self::Bruiser,
            3 => Self::Rhino,
            4 => Self::Interceptor,
            _ => Self::Chasing,
        }
    }

    /// The driver state the behaviour starts (`FUN_004f0920`).
    pub fn state(self) -> DriveState {
        match self {
            Self::Bruiser => DriveState::Bruiser { sub: 1 },
            Self::Rhino => DriveState::Rhino,
            Self::Chasing | Self::Interceptor => DriveState::Chasing { sub: 0 },
        }
    }
}

/// Overrides of the racer driver's commands (steering positive = left, ours).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlOverride {
    pub steer: Option<f32>,
    pub throttle: Option<f32>,
    pub brake: Option<f32>,
    pub handbrake: Option<bool>,
    pub nitrous: Option<bool>,
}

/// A car as the cop states see it (the game's 0x94-byte vehicle state).
#[derive(Debug, Clone, Copy)]
pub struct CarState {
    pub position: Vec3,
    pub forward: Vec3,
    /// Row 0 (inferred: the car's right).
    pub right: Vec3,
    pub velocity: Vec3,
}

impl CarState {
    /// From a body pose (forward = +Z, right = −X, the runtime's convention).
    pub fn from_pose(position: Vec3, rotation: Quat, velocity: Vec3) -> Self {
        Self { position, forward: rotation * Vec3::Z, right: rotation * Vec3::NEG_X, velocity }
    }
    fn heading2(&self) -> Vec2 {
        Vec2::new(self.forward.x, self.forward.z).normalize_or(Vec2::Y)
    }
}

/// World queries `police.rs` makes for the tick (physics rays; the game asks its nav mesh: model decision).
#[derive(Debug, Clone, Copy, Default)]
pub struct Probes {
    /// The straight line from the cop to the aim point is drivable (`FUN_00e49bb0`'s nav line test).
    pub line_clear: bool,
    /// A side probe (±25°, 3 m) hits (flag +0xc6).
    pub side_hit: bool,
    /// The forward probe's hit distance (m; `f32::MAX` for none).
    pub forward_hit: f32,
}

/// One tick's inputs.
pub struct CopInput<'a> {
    pub dt: f32,
    pub cfg: &'a CopConfig,
    pub cop: CarState,
    pub target: CarState,
    /// The target is a human player (`FUN_006c3790`).
    pub human: bool,
    /// The target's bust meter (rec +0x874).
    pub bust_meter: f32,
    /// Seconds without throttle or brake from the player (state +0x90).
    pub idle_input: f32,
    /// The cop's lateral slot (coordinator record +0x14: ±3.5 m or 0, `FUN_00518cf0`).
    pub lateral: f32,
    pub probes: Probes,
}

/// What the tick asks of the driver.
#[derive(Debug, Clone, Copy, Default)]
pub struct CopOutput {
    /// The point to drive to (state +0x68).
    pub aim: Vec3,
    /// The point may be driven to straight (+0xb7); with a clear line the path is the straight line (+0xb8).
    pub direct: bool,
    /// Target speed (m/s; `f32::MAX` = flat out under the cop's cap).
    pub target_speed: f32,
    pub overrides: ControlOverride,
    /// Which of the core's stop flags fired (bit 0 side probe, 1 misaligned, 2 wall ahead; for the log).
    pub blocked: u8,
}

/// A cop's driver.
#[derive(Debug, Clone)]
pub struct CopDriver {
    pub state: DriveState,
    pub assigned: Assigned,
    /// The target's heading every 0.1 s, the last second of it (S+0x100).
    history: Vec<Vec2>,
    history_timer: f32,
}

impl CopDriver {
    pub fn new(assigned: Assigned) -> Self {
        Self { state: assigned.state(), assigned, history: Vec::new(), history_timer: 0.0 }
    }

    /// Back to the assigned behaviour (`FUN_004f0920`).
    pub fn reassign(&mut self) {
        self.state = self.assigned.state();
    }

    pub fn state_id(&self) -> u8 {
        match self.state {
            DriveState::Chasing { .. } => 0,
            DriveState::LowSpeed { .. } => 0x16,
            DriveState::Bruiser { .. } => 0x18,
            DriveState::Rhino => 0x12,
            DriveState::HeadOn { .. } => 0x13,
        }
    }

    pub fn in_low_speed_block(&self) -> bool {
        matches!(self.state, DriveState::LowSpeed { sub: 1, .. })
    }

    /// One tick.
    pub fn tick(&mut self, i: &CopInput) -> CopOutput {
        let c = &i.cop;
        let t = &i.target;
        let cfg = i.cfg;
        // `FUN_004ffbe0`: the target's heading every 0.1 s.
        self.history_timer -= i.dt;
        if self.history_timer <= 0.0 {
            self.history_timer = HISTORY_PERIOD;
            self.history.push(t.heading2());
            if self.history.len() > HISTORY_LEN {
                self.history.remove(0);
            }
        }
        let to = t.position - c.position;
        let dist = to.length();
        // The target's travel direction (`FUN_004fa920`: its forward; the game uses a ground tangent when it is
        // on the ground, `FUN_004f7480`).
        let p = t.forward;
        let speed_t = t.velocity.length();
        let lead_aim = |lead_time: f32| p * ((p.dot(t.velocity)) * lead_time).min(cfg.max_lead);
        let mut out = CopOutput { aim: t.position, direct: false, target_speed: f32::MAX, ..Default::default() };

        // The HeadOn switch (`FUN_004fab50`, from Chasing and the Bruiser).
        if let DriveState::Chasing { .. } | DriveState::Bruiser { .. } = self.state {
            let away = (c.position - t.position).normalize_or_zero();
            if t.velocity.dot(c.forward) < 0.0
                && away.dot(t.velocity - c.velocity) > HEAD_ON_CLOSING
                && dist < HEAD_ON_RANGE
            {
                let back = if matches!(self.state, DriveState::Bruiser { .. }) {
                    BackState::Bruiser
                } else {
                    BackState::Chasing
                };
                // On entry the side is picked once: to the side the target is on (`FUN_0050b7f0`).
                let side = if c.right.dot(to.normalize_or_zero()) > 0.0 { 1.0 } else { -1.0 };
                self.state = DriveState::HeadOn { back, side };
            }
        }

        match self.state {
            DriveState::Chasing { ref mut sub } => {
                // `FUN_00505f90`: substate 2 within 40 m of a human target (`FUN_004faad0`).
                *sub = if i.human && dist < cfg.close_range { 2 } else { 0 };
                // `FUN_004ffcb0`: the target, the cop's lateral slot along its own right axis, and the lead.
                let mut aim = t.position + c.right * i.lateral;
                if *sub == 0 {
                    aim += lead_aim(cfg.lead_time);
                } else {
                    let dir2 = Vec2::new(to.x, to.z).normalize_or_zero();
                    let avg: Vec2 = self.history.iter().copied().sum();
                    let d = if avg.length_squared() > 1e-6 && c.heading2().dot(dir2).abs() < ABEAM {
                        let a = avg.normalize();
                        Vec3::new(a.x, 0.0, a.y)
                    } else {
                        t.forward
                    };
                    aim += d * speed_t * HALF;
                    out.direct = true;
                }
                out.aim = aim;
            }
            DriveState::Rhino => {
                // `FUN_005102f0`: 0.5 s ahead of the target, no 50 m cap, straight when possible.
                out.aim = t.position + p * speed_t * HALF;
                out.direct = true;
            }
            DriveState::Bruiser { ref mut sub } => {
                // `FUN_005044d0`: approach (1), line up (2), ram (0).
                let a = (c.position - t.position).dot(t.forward);
                let lat = t.right.dot(c.position - t.position).abs();
                let alongside = -cfg.alongside_behind < a && a < cfg.alongside_ahead && lat < BRUISER_LATERAL;
                if !i.human {
                    *sub = 1;
                } else {
                    *sub = match *sub {
                        1 if alongside => 0,
                        1 if dist < cfg.close_range => 2,
                        2 if alongside => 0,
                        2 if dist >= cfg.close_range => 1,
                        0 if !alongside => 1,
                        s => s,
                    };
                }
                // `FUN_004facd0`: the aim point per substate.
                out.aim = match *sub {
                    1 => t.position + lead_aim(cfg.lead_time),
                    2 => t.position + t.forward * HALF * speed_t + c.right * i.lateral,
                    _ => t.position + t.forward * HALF * speed_t - c.right * i.lateral,
                };
                out.direct = *sub != 1;
                if *sub == 0 {
                    // `FUN_0050a900`: steer into the car, 0.2 per metre of its offset to the cop's right.
                    let right2 = Vec2::new(c.right.x, c.right.z);
                    let off = Vec2::new(to.x, to.z).dot(right2);
                    out.overrides.steer = Some(-(RAM_GAIN * off).clamp(-1.0, 1.0));
                }
            }
            DriveState::HeadOn { back, side } => {
                // `FUN_0050b7f0`: the path command with handbrake and full lock to the picked side, until the
                // target no longer comes towards the cop.
                out.aim = t.position;
                out.overrides.handbrake = Some(true);
                out.overrides.steer = Some(-side);
                if c.velocity.length() > 100.0 {
                    out.overrides.brake = Some(1.0);
                }
                let away = (c.position - t.position).normalize_or_zero();
                if away.dot(t.velocity) <= 0.0 {
                    self.state = match back {
                        BackState::Chasing => DriveState::Chasing { sub: 0 },
                        BackState::Bruiser => DriveState::Bruiser { sub: 1 },
                    };
                }
            }
            DriveState::LowSpeed { ref mut sub, ref mut timer, ref mut behind, boxed } => {
                // `FUN_004fc3c0`: the aim point.
                let dir = if speed_t > 1.5e-5 { t.velocity / speed_t } else { t.forward };
                let along = dir.dot(to);
                let lat = t.right.dot(to).abs();
                let alongside = along.abs() < cfg.ls_alongside_along && lat < cfg.ls_alongside_lat;
                let mut aim = t.position + c.right * (i.lateral * 0.5) + lead_aim(cfg.ls_lead_time);
                if alongside {
                    aim = t.position + t.forward;
                }
                if *sub != 1 && along < -2.5 {
                    aim += t.forward * 3.0;
                }
                if *sub == 1 {
                    aim += t.forward * 2.0;
                }
                out.aim = aim;
                out.direct = true;
                // `FUN_004fc810`: the substate.
                *timer += i.dt;
                let set = |s: &mut u8, timer: &mut f32, v: u8| {
                    *s = v;
                    *timer = 0.0;
                };
                if (t.position.y - c.position.y).abs() > LS_HEIGHT {
                    if *sub != 0 {
                        set(sub, timer, 0);
                    }
                } else {
                    if *sub == 1 || *sub == 2 || (*sub == 3 && *timer > LS_BRAKE_HOLD) {
                        set(sub, timer, 0);
                    }
                    if along < -cfg.ls_ahead_along && t.forward.dot(c.forward) > 0.0 {
                        set(sub, timer, 3);
                    }
                    if along < -cfg.ls_block_along && lat < cfg.ls_block_lat {
                        set(sub, timer, 1);
                    }
                    if *sub == 3 && *behind {
                        set(sub, timer, 0);
                    }
                    *behind = along > cfg.ls_behind_along && lat < cfg.ls_behind_lat;
                    // +0x10d: set by the coordinator while the cop stays in the state.
                    if *behind {
                        set(sub, timer, 3);
                    }
                    let creep = (i.human && i.bust_meter > 0.0 && speed_t <= cfg.ls_slow_mph * MPH)
                        || (i.idle_input > LS_IDLE_INPUT && dist < LS_IDLE_RANGE)
                        || (boxed && *sub != 1);
                    if creep {
                        set(sub, timer, 2);
                    }
                }
                // `FUN_004fcc10`: the commands of the substate.
                match *sub {
                    1 => {
                        // Block spin: handbrake, steer across the target's lane.
                        out.overrides.handbrake = Some(true);
                        let s = c.right.dot((aim - c.position).normalize_or_zero());
                        let g = if c.forward.dot(t.forward) < 0.0 { -1.0 } else { 1.0 };
                        let game = if s > 0.0 { -cfg.ls_spin_steer * g } else { cfg.ls_spin_steer * g };
                        out.overrides.steer = Some(-game);
                    }
                    2 => {
                        let d = if dist < LS_STOP { 0.0 } else { dist };
                        out.target_speed = LS_CREEP_MPH * MPH * (d / LS_CREEP_RANGE).clamp(0.0, 1.0);
                    }
                    3 => {
                        if c.velocity.length() > 0.0 {
                            out.overrides.throttle = Some(0.0);
                            out.overrides.brake = Some(cfg.ls_brake);
                        } else {
                            out.target_speed = 0.0;
                        }
                    }
                    _ => {}
                }
                if dist < LS_STOP && (c.velocity.length() < 1.5e-5 || speed_t < 1.5e-5) {
                    out.overrides.handbrake = Some(true);
                    out.overrides.steer = Some(0.0);
                }
            }
        }

        // The chase core (`FUN_0050e370`) on the direct path: a cop pointed away from its aim point at speed, or
        // with a wall close ahead, aims at the target itself and brakes (flags +0xc6 / +0xc7 / +0xc8).
        if out.direct && i.probes.line_clear {
            let speed = c.velocity.length();
            let aim_dir = Vec2::new(out.aim.x - c.position.x, out.aim.z - c.position.z).normalize_or_zero();
            let err = c.heading2().angle_to(aim_dir).abs().to_degrees();
            let misaligned =
                err > cfg.misaligned_deg && speed > cfg.misaligned_mph * MPH && dist > cfg.misaligned_distance;
            let wall = i.probes.forward_hit < cfg.probe_stop_distance && speed > cfg.probe_stop_mph * MPH;
            // MODEL: the side probes count only above the misaligned test's speed (20 mph). The game's probes find
            // the nav mesh's border and stop a cop at any speed; ours are physics rays, which a cop parked by a
            // wall would hit for ever (it would never move off).
            let side = i.probes.side_hit && speed > cfg.misaligned_mph * MPH;
            if side || misaligned || wall {
                out.aim = t.position;
                out.target_speed = 0.0;
                out.overrides = Default::default();
                out.blocked = u8::from(side) | u8::from(misaligned) << 1 | u8::from(wall) << 2;
            }
        }
        out
    }
}

/// `FUN_005240a0`: the low-speed chase only within 3.5 m of height.
pub const LOW_SPEED_HEIGHT: f32 = 3.5;

/// What the coordinator knows when it sets a chasing cop's state (`FUN_005240a0`, `FUN_00520f10`,
/// `FUN_00518cf0`).
#[derive(Debug, Clone, Copy)]
pub struct ChaseContext {
    /// The player's speed (mph, unsigned) and the heat level's low-speed limit (`Coordination.7a65b100`).
    pub player_mph: f32,
    pub low_speed_mph: f32,
    /// The cop's distance to the player and the height difference (m).
    pub gap: f32,
    pub dy: f32,
    /// The player's car has crashed.
    pub player_crashed: bool,
    /// Some cop blocks in the low-speed chase (the box-in, `FUN_00520f10`), and the box-in has run 1 s.
    pub blocking: bool,
    pub boxed: bool,
    /// The cop is behind its target (`(cop − target) · target forward < 0`): a Rhino turns Bruiser.
    pub behind_target: bool,
}

impl CopDriver {
    /// The coordinator's switches of a chasing cop's state.
    pub fn coordinate(&mut self, cfg: &CopConfig, c: &ChaseContext) {
        let near_slow =
            (c.player_mph < c.low_speed_mph && c.gap < cfg.low_speed_range && c.dy.abs() < LOW_SPEED_HEIGHT)
                || c.player_crashed;
        let low = DriveState::LowSpeed { sub: 0, timer: 0.0, behind: false, boxed: false };
        let interceptor = self.assigned == Assigned::Interceptor;
        match self.state {
            DriveState::Chasing { .. } | DriveState::Bruiser { .. } if near_slow => self.state = low,
            // The box-in pulls the other chasing cops in (not an Interceptor, `FUN_00511870`).
            DriveState::Chasing { .. } | DriveState::Bruiser { .. } | DriveState::Rhino
                if c.blocking && !interceptor =>
            {
                self.state = low
            }
            DriveState::LowSpeed { .. }
                if c.player_mph > c.low_speed_mph || (c.gap > cfg.low_speed_range && !c.player_crashed) =>
            {
                self.reassign();
            }
            _ => {}
        }
        if let DriveState::LowSpeed { boxed, .. } = &mut self.state {
            *boxed = c.boxed;
        }
        // `FUN_00518cf0`: a Rhino behind its target becomes a Bruiser.
        if self.state == DriveState::Rhino && c.behind_target {
            self.state = DriveState::Bruiser { sub: 1 };
        }
    }

    /// The game's chasing predicate's driver half (`FUN_004f2b20`: states 0, 0x10, 0x12, 0x16, 0x17, 0x18, 0x1a).
    pub fn chasing_state(&self) -> bool {
        matches!(self.state_id(), 0 | 0x10 | 0x12 | 0x16 | 0x17 | 0x18 | 0x1a)
    }
}
