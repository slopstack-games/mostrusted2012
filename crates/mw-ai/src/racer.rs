//! The AI driver's tick (`FUN_004e75f0`) with the racer state (state 8, Update `FUN_0050da90`) and the crash
//! recovery (state 11, Update `FUN_004eeba0`), and the hand-over to the vehicle (`AIPlayer::FillInput` 0x006c0890,
//! `FUN_009aacc0`).

use crate::{AiConfig, MPH, NO_SPEED, Path, Pid, V2};

/// One car as the AI code reads it: the AI vehicle state (0x94 bytes at world+0x42d60, refreshed every tick from
/// the vehicle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarState {
    /// +0x68: the player handle (the follow rule compares them; `FUN_004fdb60` tells the local player by it).
    pub id: i32,
    /// +0x24..+0x2c: position (world, m).
    pub position: [f32; 3],
    /// +0x30..+0x38: velocity (m/s); every "speed" below is its length.
    pub velocity: [f32; 3],
    /// +0x18..+0x20: the body's forward axis (the emergency brake's search centre, the recovery's probes).
    pub forward: [f32; 3],
    /// +0x3c / +0x40: the heading in the ground plane (unit (x, z)).
    pub heading: V2,
    /// +0x50: the car's radius for car-against-car tests (`FUN_00508070`, `FUN_005050d0`). Writer not traced.
    pub radius: f32,
    /// +0x58: inferred half length (the shunt window, the sweep's shortest length, the recovery's ahead point).
    pub half_length: f32,
    /// +0x74: read by mode 2's nitrous rule (`FUN_004fd940`: only when 0). Meaning not traced.
    pub field_74: f32,
    /// +0x78: the nitrous tank (`FUN_004ff620` compares it with `AIConfig` 0x140).
    pub nitrous_tank: f32,
    /// +0x88 bit 3: the car is physical (bit clear: a virtual racer, `FUN_005056b0`).
    pub physical: bool,
    /// +0x88 bit 1: nitrous allowed (`FUN_004ff620`; meaning inferred from that use).
    pub nitrous_allowed: bool,
    /// +0x88 bit 5: blocks mode 2's nitrous (`FUN_004fd940`). Meaning not traced.
    pub flag_5: bool,
    /// +0x88 bits 0 / 9 / 14: the contact flags of the crash check (`FUN_004fb9f0`; inferred: body contacts).
    pub contact: bool,
    /// Every wheel off the ground (`FUN_008f5350` on the vehicle): no steering, no brake.
    pub all_wheels_off: bool,
}

impl CarState {
    pub fn ground(&self) -> V2 {
        V2::new(self.position[0], self.position[2])
    }
    pub fn speed(&self) -> f32 {
        let v = self.velocity;
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }
    fn forward_ground(&self) -> V2 {
        V2::new(self.forward[0], self.forward[2])
    }
}

/// What the runtime hands the driver each fixed tick (1/60 s: the pedal integrator steps once per tick).
#[derive(Debug, Clone, Copy)]
pub struct RacerInput<'a> {
    /// The tick length × the clock's time scale (`FUN_0046ee10` +0x10 × +0x14).
    pub dt: f32,
    pub me: &'a CarState,
    /// The other race cars (the 16 player slots: the racer interaction's nearest car) and the AI vehicles the
    /// emergency brake's grid query finds (`FUN_005127a0`); ours: one list, self excluded.
    pub others: &'a [CarState],
    /// The local player's car (`FUN_006a0020` / `FUN_004e3710`).
    pub local_player: Option<&'a CarState>,
    /// The rubber band's schedule speed (driver +0x5668, m/s; [`NO_SPEED`] outside a race).
    pub schedule_speed: f32,
    /// The settings speed (driver +0x818; −1 for a racer, `FUN_006c0700`): used instead of the schedule when ≥ 0.
    pub settings_speed: f32,
    /// The lane planner's new offset when it ran this tick for this driver (`FUN_0050b000` → `FUN_0050a630`, one
    /// driver per frame); `None` otherwise. Not ported: 0 with no obstacle records around.
    pub lane_target: Option<f32>,
    /// The lane planner's look-ahead cap (driver +0x549c); `None` = 1000 (none).
    pub look_cap: Option<f32>,
    /// The obstacle sweep (`FUN_00503bf0` → `FUN_004ff270`, world geometry; see [`Driver::sweep_lengths`]):
    /// (wide box hit → brake 1, narrow box hit → target 0). Run by the runtime only above 20 mph and outside the
    /// ram mode.
    pub sweep: (bool, bool),
    /// The driver's nitrous mode (driver +0x828) is 2: no nitrous.
    pub nitrous_mode_2: bool,
    /// The racer interaction is enabled (`FUN_0051b320`).
    pub interaction: bool,
    /// The slot's collision record names this car (world+0x5de2c / +0x5de3c, +0x80): resets the shunt timer.
    pub contact_partner: Option<i32>,
    /// The recovery's probes (`FUN_00514ca0` sweeps along the body): +25° and −25° off the forward axis (2 m),
    /// forward (4 m), backward (0.25 m).
    pub probes: [bool; 4],
    /// The car is visible (`FUN_00443060`): the 3 s reset applies only when it is not.
    pub visible: bool,
}

/// The driver's command block (driver +0x57f0, 0x30 bytes), cleared every tick by `FUN_004e75f0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    /// +0x00 / +0x04 / +0x08: throttle, brake, hold.
    pub throttle: f32,
    pub brake: f32,
    pub hold: f32,
    /// +0x0c: steering, positive = right.
    pub steering: f32,
    /// +0x10: handbrake (analog, 0 or 1 here).
    pub handbrake: f32,
    /// +0x14: nitrous (0 or 1).
    pub nitrous: f32,
    /// +0x18: target speed (m/s, [`NO_SPEED`] = none).
    pub target_speed: f32,
    /// +0x2c / +0x2d: throttle / brake set: later stages keep them.
    pub throttle_set: bool,
    pub brake_set: bool,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            throttle: 0.0,
            brake: 0.0,
            hold: 0.0,
            steering: 0.0,
            handbrake: 0.0,
            nitrous: 0.0,
            target_speed: NO_SPEED,
            throttle_set: false,
            brake_set: false,
        }
    }
}

/// The player's input record (player +0x1928) as `AIPlayer::FillInput` fills it from the command block.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct InputRecord {
    /// +0x04 throttle, +0x08 brake, +0x0c hold, +0x10 handbrake, +0x14 steering, +0x1c nitrous.
    pub throttle: f32,
    pub brake: f32,
    pub hold: f32,
    pub handbrake: f32,
    pub steering: f32,
    pub nitrous: f32,
}

impl From<&Command> for InputRecord {
    /// 0x006c0890: +0x04 ← cmd+0x00, +0x08 ← +0x04, +0x0c ← +0x08, +0x10 ← +0x10, +0x14 ← +0x0c,
    /// +0x1c ← +0x14 (and the nitrous mode byte +0x33 = 0). Not ported: the scripted override (player +0x1f0 /
    /// +0x1f4 set → steering +0x5b4, brake 0, handbrake +0x5b8, throttle +0x5bc).
    fn from(c: &Command) -> Self {
        Self {
            throttle: c.throttle,
            brake: c.brake,
            hold: c.hold,
            handbrake: c.handbrake,
            steering: c.steering,
            nitrous: c.nitrous,
        }
    }
}

/// The vehicle's control record as `FUN_009aacc0` writes it from an input record.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct VehicleControls {
    /// +0x1bf0, positive = right.
    pub steering: f32,
    /// +0x1bf4.
    pub throttle: f32,
    /// +0x1c00.
    pub brake: f32,
    /// +0x1c10: the hold override.
    pub hold: f32,
    /// +0x1c40: the handbrake mask (input above 0.1, `.rdata` 0x01021cd8).
    pub handbrake: bool,
    /// +0x1c20: nitrous requested (> 0 starts / keeps it, `FUN_00662ea0`; else `FUN_00658a40` stops it).
    pub nitrous: bool,
}

impl From<&InputRecord> for VehicleControls {
    fn from(i: &InputRecord) -> Self {
        Self {
            steering: i.steering,
            throttle: i.throttle,
            brake: i.brake,
            hold: i.hold,
            handbrake: i.handbrake > 0.1,
            nitrous: i.nitrous > 0.0,
        }
    }
}

/// The racer interaction's mode (racer state +0x2c, `FUN_005050d0`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    None,
    /// 1: steer into the car alongside (`AIConfig` 0x198 × the lateral gap).
    Shunt,
    /// 2: hold 2 m ahead of a close, fast car behind with a lower handle (`FUN_00501860` / `FUN_004fd940`).
    HoldAhead,
}

/// The racer state's random numbers (state +0x130..+0x158): an MMIX LCG (× 0x5851f42d4c957f2d + 1) whose high
/// word's top 23 bits make a float in [1, 2), kept in a ring of 8 so a draw returns the value stored 8 draws ago.
/// The state's Enter (0x00506260) seeds it from the system time (`GetSystemTime` → FILETIME).
#[derive(Debug, Clone, PartialEq)]
pub struct AiRandom {
    state: u64,
    ring: [f32; 8],
    index: usize,
}

impl AiRandom {
    const A: u64 = 0x5851_f42d_4c95_7f2d;

    fn step(&mut self) -> f32 {
        let hi = (self.state >> 32) as u32;
        self.state = self.state.wrapping_mul(Self::A).wrapping_add(1);
        f32::from_bits(hi >> 9 | 0x3f80_0000)
    }

    /// Seeded as 0x00506260 does after its reseed: two warm-up steps, then eight draws into the ring.
    pub fn seeded(seed: u64) -> Self {
        let mut r = Self { state: seed, ring: [1.0; 8], index: 0 };
        r.state = r.state.wrapping_mul(Self::A).wrapping_add(1);
        r.state = r.state.wrapping_mul(Self::A).wrapping_add(1);
        for k in 0..8 {
            r.ring[k] = r.step();
        }
        r
    }

    /// A number in [0, 1): the ring's value minus 1, replaced by a fresh draw.
    pub fn draw(&mut self) -> f32 {
        let v = self.ring[self.index] - 1.0;
        self.ring[self.index] = self.step();
        self.index = (self.index + 1) & 7;
        v
    }
}

/// The crash check's timers (state +0x20 stuck, +0x24 free), one pair per state object.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct StuckTimers {
    stuck: f32,
    free: f32,
}

impl StuckTimers {
    /// `FUN_004fb9f0` without a target point: free of contact, the free time grows and past 0.2 s (`.rdata`
    /// 0x00fa8ec0) clears the stuck time; in contact, stuck += dt + free and free = 0 (as the code does: a contact
    /// after a long free run counts that run). Past 1 s both clear, and the car is stuck when slower than 10 mph
    /// (0x00fa929c).
    fn check(&mut self, contact: bool, speed: f32, dt: f32) -> bool {
        if !contact {
            self.free += dt;
            if self.free > 0.2 {
                self.stuck = 0.0;
            }
            return false;
        }
        self.stuck += dt + self.free;
        self.free = 0.0;
        if self.stuck > 1.0 {
            *self = Self::default();
            return speed < 10.0 * MPH;
        }
        false
    }
}

/// State 11's own fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recovery {
    /// +0x34: driving backwards. The Enter (0x004e0460) starts in reverse.
    pub reverse: bool,
    /// +0x35: the direction is locked; +0x2c its timer.
    locked: bool,
    lock_timer: f32,
    timers: StuckTimers,
}

impl Recovery {
    fn enter() -> Self {
        Self { reverse: true, locked: false, lock_timer: 0.0, timers: StuckTimers::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// State 8.
    Racer,
    /// State 11.
    Recovery(Recovery),
}

/// What a tick produces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickOutput {
    /// The command block as posted (cleared when the state posted nothing).
    pub command: Command,
    /// `FillInput` → `FUN_009aacc0`: the vehicle's control record.
    pub controls: VehicleControls,
    /// The reset request (driver +0x584e, consumed by `FUN_0083d4b0`).
    pub reset: bool,
}

/// One AI racer.
#[derive(Debug, Clone)]
pub struct Driver {
    pub state: State,
    /// The current path node (driver +0x8a8).
    pub node: usize,
    /// The steering PID (controller +0x00): `Vehicle_Gameplay.PIDController` (Proportional +0x18, Integral +0x14,
    /// Derivative +0x10), clamps 20 / 20 (`.rdata` 0x00fa3ba0).
    pub steer_pid: Pid,
    /// The speed PID (controller +0x38 area, `FUN_004f8100`): 0.6 / 0.05 / 0.5, clamps 9 / 9
    /// (`.rdata` 0x00fa8ecc / 0x00fa8ed0 / 0x00f8a810 / 0x00fa8edc).
    pub speed_pid: Pid,
    /// Controller +0x74: the pedal the speed PID integrates (> 0 throttle, else brake).
    pub pedal: f32,
    /// Controller +0x5c / +0x60 / +0x64 / +0x68: the lane offset's old and new values, blend and blend rate.
    pub lane_old: f32,
    pub lane_new: f32,
    pub lane_t: f32,
    pub lane_rate: f32,
    /// Controller +0x6c: nitrous fired last tick.
    pub nitrous_on: bool,
    /// The racer state's crash-check timers.
    racer_timers: StuckTimers,
    /// Racer state +0x2c / +0x120 / +0x124: interaction mode, shunt timer and delay.
    pub interaction: Interaction,
    pub shunt_time: f32,
    pub shunt_delay: f32,
    pub random: AiRandom,
    /// Driver +0x5854: time in the recovery (decays at 0.25 / s, `.rdata` 0x00fa8830, outside it).
    pub recovery_time: f32,
    /// Diagnostics: the last look-ahead time (s), distance (m), lateral error (m), raw steering (controller +0x70).
    pub look_time: f32,
    pub look: f32,
    pub lateral_error: f32,
    pub steer_raw: f32,
}

impl Driver {
    /// A racer on `node` with the car's steering gains [P, I, D]; `seed` for the state's random numbers (the game
    /// uses the system time).
    pub fn new(node: usize, steer_gains: [f32; 3], seed: u64) -> Self {
        Self {
            state: State::Racer,
            node,
            steer_pid: Pid::new(steer_gains[0], steer_gains[1], steer_gains[2], 20.0, 20.0),
            speed_pid: Pid::new(0.6, 0.05, 0.5, 9.0, 9.0),
            pedal: 0.0,
            lane_old: 0.0,
            lane_new: 0.0,
            lane_t: 0.0,
            lane_rate: 0.0,
            nitrous_on: false,
            racer_timers: StuckTimers::default(),
            interaction: Interaction::None,
            shunt_time: 0.0,
            shunt_delay: 0.0,
            random: AiRandom::seeded(seed),
            recovery_time: 0.0,
            look_time: 0.0,
            look: 0.0,
            lateral_error: 0.0,
            steer_raw: 0.0,
        }
    }

    /// The lane offset in use: lerp(old, new, t) (`FUN_00506c00`).
    pub fn lane_offset(&self) -> f32 {
        (self.lane_new - self.lane_old) * self.lane_t + self.lane_old
    }

    /// `FUN_004e75f0`: clear the command block, run the state, then the reset timer; FillInput maps the result.
    pub fn tick(&mut self, cfg: &AiConfig, path: &Path, i: &RacerInput) -> TickOutput {
        let mut cmd = Command::default();
        match self.state {
            State::Racer => self.racer(cfg, path, i, &mut cmd),
            State::Recovery(r) => self.recovery(r, path, i, &mut cmd),
        }
        let mut reset = false;
        if matches!(self.state, State::Recovery(_)) {
            self.recovery_time += i.dt;
            if self.recovery_time > cfg.reset_time || (self.recovery_time > cfg.reset_time_hidden && !i.visible) {
                reset = true;
                self.recovery_time = 0.0;
            }
        } else {
            self.recovery_time = (self.recovery_time - i.dt * 0.25).max(0.0);
        }
        let input = InputRecord::from(&cmd);
        TickOutput { command: cmd, controls: VehicleControls::from(&input), reset }
    }

    /// State 8's Update (`FUN_0050da90`) for a physical car. Not ported here: the path upkeep (vtable +0x18,
    /// `FUN_0050a070`) and the path check / re-plan (`FUN_00506d60`): the runtime owns the path; the virtual
    /// progress (vtable +0x1c, `FUN_005056b0`).
    fn racer(&mut self, cfg: &AiConfig, path: &Path, i: &RacerInput, cmd: &mut Command) {
        let me = i.me;
        // `FUN_0050b000`: a new planner offset restarts the blend from the current offset at `lane_rate` m/s.
        if let Some(target) = i.lane_target {
            self.lane_old = self.lane_offset();
            self.lane_new = target;
            self.lane_t = 0.0;
            let gap = (target - self.lane_old).abs();
            self.lane_rate = if gap > 0.0 { cfg.lane_rate / gap } else { 0.0 };
        }
        // Crash check (`FUN_004fb9f0` → state 11). Nothing is posted this tick.
        if self.racer_timers.check(me.contact, me.speed(), i.dt) {
            self.state = State::Recovery(Recovery::enter());
            return;
        }
        if !me.physical {
            return;
        }
        *cmd = self.command(cfg, path, i);
        self.player_nitrous(i, cmd);
        if i.interaction {
            self.interact(cfg, i, cmd);
        }
        // `FUN_005095b0`: the emergency brake, then the pedals.
        if emergency_brake(cfg, me, i.others) {
            cmd.brake = 1.0;
            cmd.brake_set = true;
        }
        self.pedals(me, cmd, i.dt);
    }

    /// `FUN_0050a740` with `FUN_00508d50`: lane blend, steering, target speed, the obstacle sweep's result, nitrous,
    /// then the node step.
    fn command(&mut self, cfg: &AiConfig, path: &Path, i: &RacerInput) -> Command {
        let me = i.me;
        self.lane_t = (self.lane_rate * i.dt + self.lane_t).min(1.0);
        let mut cmd = Command::default();
        if path.count_from(self.node) < 2 {
            return cmd;
        }
        let speed = me.speed();
        // Steering (`FUN_00503880`, `FUN_00506c00` → `FUN_00505e60`, the PID `FUN_00436920`).
        let t = path.look_time(cfg, self.node, speed, i.look_cap);
        let look = (speed * t + 3.0).max(cfg.min_look); // 3: `.data` 0x010c4414
        let pos = me.ground();
        let target_point = path.point_at(self.node, path.along(self.node, pos) + look);
        let e = (pos + me.heading * look - target_point).dot(me.heading.right());
        let u = self.steer_pid.step(e - self.lane_offset(), i.dt);
        self.look_time = t;
        self.look = look;
        self.lateral_error = e;
        self.steer_raw = u;
        let mut steering = u.clamp(-1.0, 1.0);
        if me.physical && me.all_wheels_off {
            steering = 0.0;
        }
        // Target speed (`FUN_004e03c0`: the settings speed when ≥ 0, else the schedule), lowered by the sweep.
        let schedule = if i.settings_speed >= 0.0 { i.settings_speed } else { i.schedule_speed };
        let mut target = schedule.min(NO_SPEED);
        let (wide, narrow) = i.sweep;
        if (wide || narrow) && path.count_from(self.node) > 2 {
            let cap = if narrow { 0.0 } else { NO_SPEED };
            if wide {
                cmd.brake = 1.0;
                cmd.brake_set = true;
            }
            target = target.min(cap);
        }
        cmd.steering = steering;
        cmd.target_speed = target;
        cmd.handbrake = 0.0;
        // Nitrous (`FUN_004ff620`).
        let gap = speed + 2.7 < target; // `.rdata` 0x00fa8f0c
        self.nitrous_on = self.nitrous(cfg, path, i, gap, schedule);
        if self.nitrous_on {
            cmd.nitrous = 1.0;
        }
        self.node = path.advance(self.node, pos);
        cmd
    }

    /// `FUN_004ff620`: only with a speed gap to the target, nitrous mode ≠ 2 and nitrous allowed (+0x88 bit 1).
    /// To start: tank > `nitrous_min_tank` and speed ≤ schedule − `nitrous_margin_mph`, and the turn ahead over
    /// speed × 1.5 m (`.rdata` 0x00fa8d30, `FUN_004e4eb0`) under `nitrous_start_turn`; to keep firing, under
    /// `nitrous_keep_turn`.
    fn nitrous(&self, cfg: &AiConfig, path: &Path, i: &RacerInput, gap: bool, schedule: f32) -> bool {
        let me = i.me;
        if i.nitrous_mode_2 || !gap || !me.nitrous_allowed {
            return false;
        }
        let speed = me.speed();
        let turn = path.turn_ahead(self.node, me.ground(), me.heading, speed * 1.5);
        let limit = if self.nitrous_on {
            cfg.nitrous_keep_turn
        } else {
            if me.nitrous_tank <= cfg.nitrous_min_tank || schedule - cfg.nitrous_margin_mph * MPH < speed {
                return false;
            }
            cfg.nitrous_start_turn
        };
        turn < limit * 0.017_453_3
    }

    /// `FUN_004fdb60`: the local player ahead along this car's heading, this car below its schedule speed and
    /// with a speed gap to the target → nitrous.
    fn player_nitrous(&self, i: &RacerInput, cmd: &mut Command) {
        let (me, Some(p)) = (i.me, i.local_player) else { return };
        if p.id == me.id || (p.ground() - me.ground()).dot(me.heading) <= 0.0 {
            return;
        }
        let schedule = if i.settings_speed >= 0.0 { i.settings_speed } else { i.schedule_speed };
        let speed = me.speed();
        if speed < schedule && speed + 2.7 < cmd.target_speed {
            cmd.nitrous = 1.0;
        }
    }

    /// The racer interaction (state 8 vtable +0x20, `FUN_005050d0`), with the nearest race car when both are
    /// physical.
    fn interact(&mut self, cfg: &AiConfig, i: &RacerInput, cmd: &mut Command) {
        let me = i.me;
        self.interaction = Interaction::None;
        if !me.physical {
            return;
        }
        let dist2 = |o: &CarState| {
            let d = [0, 1, 2].map(|k| me.position[k] - o.position[k]);
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
        };
        let Some(o) = i.others.iter().filter(|o| o.id != me.id).min_by(|a, b| dist2(a).total_cmp(&dist2(b))) else {
            return;
        };
        if !o.physical {
            return;
        }
        let (speed, other_speed) = (me.speed(), o.speed());
        // How far this car is ahead of the other along the other's heading.
        let ahead = (me.ground() - o.ground()).dot(o.heading);
        let dv = speed - other_speed;
        if -cfg.hold_behind <= ahead
            && ahead <= cfg.hold_ahead
            && -(cfg.hold_dv_min_mph * MPH) <= dv
            && dv <= cfg.hold_dv_max_mph * MPH
            && cfg.hold_min_mph * MPH <= other_speed
            && o.id < me.id
        {
            self.interaction = Interaction::HoldAhead;
            // State +0x128 = 0 (not read here).
        }
        if !(cfg.shunt_min_mph * MPH <= speed && speed <= cfg.shunt_max_mph * MPH) {
            return;
        }
        if ahead < cfg.shunt_behind * o.half_length * -2.0 || cfg.shunt_ahead * o.half_length * 2.0 < ahead {
            self.shunt_time = 0.0;
        } else {
            if self.shunt_time == 0.0 {
                let r = self.random.draw();
                self.shunt_delay = (cfg.shunt_delay_max - cfg.shunt_delay_min) * r + cfg.shunt_delay_min;
            }
            self.shunt_time += i.dt;
            // The other car's offset to this car's right.
            let lateral = (o.ground() - me.ground()).dot(me.heading.right());
            if lateral.abs() <= o.radius + me.radius + cfg.shunt_lateral
                && self.shunt_delay <= self.shunt_time
                && (speed - other_speed).abs() <= cfg.shunt_dv_mph * MPH
            {
                self.interaction = Interaction::Shunt;
                cmd.steering = (cfg.shunt_steer * lateral).clamp(-1.0, 1.0);
            }
        }
        if let Some(p) = i.local_player {
            let rel = (me.ground() - p.ground()).dot(p.heading);
            if rel < -cfg.player_window_behind || cfg.player_window_ahead < rel {
                self.interaction = Interaction::None;
                self.shunt_time = 0.0;
            }
        }
        if i.contact_partner == Some(o.id) {
            self.shunt_time = 0.0;
        }
        if self.interaction == Interaction::HoldAhead {
            let v = hold_speed(cfg, me, o);
            hold_apply(v, me, o, cmd);
        }
    }

    /// `FUN_004fa0e0`: the speed PID on (speed − target) integrates the pedal by 0.03 × u per tick (`.rdata`
    /// 0x00fa8ee0); > 0 is throttle, else brake, unless set earlier. A target under 0.5 m/s (`.rdata` 0x00f8a810)
    /// below 10 mph pulls the handbrake, and below 0.5 m/s also brakes fully. Every wheel off the ground: brake 0.
    fn pedals(&mut self, me: &CarState, cmd: &mut Command, dt: f32) {
        let speed = me.speed();
        let u = self.speed_pid.step(speed - cmd.target_speed, dt);
        self.pedal = (self.pedal + u * 0.03).clamp(-1.0, 1.0);
        if self.pedal <= 0.0 {
            if !cmd.throttle_set {
                cmd.throttle = 0.0;
            }
            if !cmd.brake_set {
                cmd.brake = -self.pedal;
            }
        } else {
            if !cmd.throttle_set {
                cmd.throttle = self.pedal;
            }
            if !cmd.brake_set {
                cmd.brake = 0.0;
            }
        }
        if cmd.target_speed.abs() < 0.5 && speed < 10.0 * MPH {
            cmd.handbrake = 1.0;
            if speed < 0.5 {
                if !cmd.throttle_set {
                    cmd.throttle = 0.0;
                }
                if !cmd.brake_set {
                    cmd.brake = 1.0;
                }
            }
        }
        if me.physical && me.all_wheels_off {
            cmd.brake = 0.0;
            cmd.brake_set = true;
        }
    }

    /// State 11's Update (`FUN_004eeba0`). Exits to the racer above 5 mph (`.rdata` 0x00fa8538) once the heading is
    /// within 45° (`.data` 0x010c422c) of the direction to the next node, the three forward probes are clear and
    /// no car's sphere crosses the 8 m (`.rdata` 0x00fa8124) ahead. Else: steering = clamp(−4 (`.rdata`
    /// 0x00fa88b0) × left · direction, ±1); the direction flips (at most once a second) when the backward probe
    /// hits or the car sticks again; the pedal of `FUN_004f9e80` towards 10 mph (0x00fa814c), never negative, is
    /// throttle forwards and brake (with mirrored steering) backwards; both pedals set.
    fn recovery(&mut self, mut r: Recovery, path: &Path, i: &RacerInput, cmd: &mut Command) {
        let me = i.me;
        let pos = me.ground();
        let speed = me.speed();
        // `FUN_004e3540`: towards the next node (the game also takes a nearby reset point, `FUN_004f2b20`).
        let next = if path.count_from(self.node) >= 2 { path.node(self.node, 1).position } else { pos };
        let dir = (next - pos).normalize();
        let angle = me.heading.dot(dir).clamp(-1.0, 1.0).acos();
        let left = me.heading.right() * -1.0;
        let mut ok = angle < std::f32::consts::FRAC_PI_4;
        // The segment test: cars near pos + forward × (half length + 3) (query radius 3, `.rdata` 0x00fa821c;
        // ours: within 3 m + their radius) whose sphere the 8 m segment ahead crosses.
        let fwd = me.forward_ground().normalize();
        let q = pos + fwd * (me.half_length + 3.0);
        let end = pos + fwd * 8.0;
        for o in i.others.iter().filter(|o| o.id != me.id && (o.ground() - q).length() <= 3.0 + o.radius) {
            if segment_hits_sphere(pos, end, o.ground(), o.radius) {
                ok = false;
                break;
            }
        }
        if i.probes[..3].iter().any(|&h| h) {
            ok = false;
        }
        if speed > 5.0 * MPH && ok {
            self.state = State::Racer;
            self.racer_timers = StuckTimers::default();
            return;
        }
        let mut steering = (left.dot(dir) * -4.0).clamp(-1.0, 1.0);
        if r.lock_timer > 0.0 {
            r.lock_timer -= i.dt;
            if r.lock_timer < 0.0 {
                r.locked = false;
            }
        }
        if (i.probes[3] || r.timers.check(me.contact, speed, i.dt)) && !r.locked {
            r.locked = true;
            r.reverse = !r.reverse;
            r.lock_timer = 1.0;
        }
        let pedal = recovery_pedal(10.0 * MPH, speed).clamp(0.0, 1.0);
        *cmd = Command { throttle_set: true, brake_set: true, ..Command::default() };
        if r.reverse {
            steering = -steering;
            cmd.brake = pedal;
        } else {
            cmd.throttle = pedal;
        }
        cmd.steering = steering;
        self.state = State::Recovery(r);
    }

    /// The obstacle sweep's lengths (`FUN_00503bf0`, for the runtime's `FUN_004ff270` sweeps):
    /// (short, long) = (min(`sweep_max`, max(speed × `sweep_speed_factor`, half length)),
    /// min(`sweep_max`, max(short, speed × `sweep_time_b` (or `_a`), the stopping distance))). Run only above
    /// 20 mph (`.rdata` 0x00fa3ba0 × mph); the box scales are `sweep_wide` (brakes) / `sweep_narrow` (stops).
    pub fn sweep_lengths(cfg: &AiConfig, me: &CarState, stop_distance: f32, flagged: bool) -> Option<(f32, f32)> {
        let speed = me.speed();
        if speed <= 20.0 * MPH {
            return None;
        }
        let a = (speed * if flagged { cfg.sweep_time_a } else { cfg.sweep_time_b }).max(stop_distance);
        let short = (speed * cfg.sweep_speed_factor).max(me.half_length).min(cfg.sweep_max);
        Some((short, short.max(a).min(cfg.sweep_max)))
    }

    /// The sweep itself (`FUN_00503bf0` → `FUN_00514ad0`, `FUN_004ff270` twice): not an obstacle test but a
    /// **turning-circle** test of the path ahead. `FUN_00514ad0` gives the car's tightest turn at its speed,
    /// R = |v|² / `lateral_accel` (`lateral_accel` = vehicle +0x550 × +0x5c4 × wheel 0's effective lateral surface
    /// grip (+0x17e0 +0x70) × 9.81, `.bss` 0x01106a10 read live), and the side unit vector normalize(−v.z, v.x)
    /// (`FUN_00513200` on v × up). `FUN_004ff270` walks the path from the car's arc length (`FUN_004e47f0`) +
    /// `long` back to + `short` in 3 m steps (`.rdata` 0x00fa8e5c), each point moved sideways by the lane offset
    /// (driver +0x60), and hits when one lies within scale × R of either circle centre pos ± scale × R × side:
    /// a point the car cannot turn onto at this speed. Scales `sweep_wide` (0x204, hit → brake 1) and
    /// `sweep_narrow` (0x208, hit → target speed 0). Returns (wide, narrow) for [`RacerInput::sweep`]. Ours: the
    /// test is in the ground plane (the game's is 3D with the path's height); the lane offset is moved along the
    /// segment's chord.
    pub fn sweep(
        &self,
        cfg: &AiConfig,
        path: &Path,
        me: &CarState,
        lateral_accel: f32,
        lengths: (f32, f32),
    ) -> (bool, bool) {
        if path.count_from(self.node) < 2 || lateral_accel <= 0.0 {
            return (false, false);
        }
        let (short, long) = lengths;
        let v = me.velocity;
        let radius = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) / lateral_accel;
        let side = V2::new(-v[2], v[0]).normalize();
        let pos = me.ground();
        let s0 = path.project(self.node, pos);
        let lane = self.lane_offset();
        let test = |scale: f32| {
            let r = scale * radius;
            let (c1, c2) = (pos + side * r, pos - side * r);
            let mut s = s0 + long;
            while s >= s0 + short {
                let p = path.point_at(self.node, s) + path.direction_at(self.node, s).right() * lane;
                let d = (p - c1).dot(p - c1).min((p - c2).dot(p - c2));
                if d < r * r {
                    return true;
                }
                s -= 3.0; // `.rdata` 0x00fa8e5c
            }
            false
        };
        (test(cfg.sweep_wide), test(cfg.sweep_narrow))
    }
}

/// `FUN_00508070` (without its first, optional sweep): above 70 mph (`.rdata` 0x00fa8eb8), the cars within
/// r = clamp(speed × `contact_time` × 0.5, 40, 150) (`.rdata` 0x00fa8bec / 0x00fa8eec) of pos + forward × (r − 10)
/// are tested for a 2D circle-circle time to contact with velocities heading × speed and radius sum
/// (+0x50 × 2 + other +0x50 × 2) × 0.5; overlapping = 0, closing slower than 1.19e-7 or separating = never. A
/// contact within `contact_time` brakes.
fn emergency_brake(cfg: &AiConfig, me: &CarState, others: &[CarState]) -> bool {
    let speed = me.speed();
    if speed <= 70.0 * MPH {
        return false;
    }
    let r = (speed * cfg.contact_time * 0.5).clamp(40.0, 150.0);
    let c = [0, 1, 2].map(|k| me.position[k] + me.forward[k] * (r - 10.0));
    others.iter().filter(|o| o.id != me.id).any(|o| {
        let d = [0, 1, 2].map(|k| o.position[k] - c[k]);
        if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > r {
            return false;
        }
        time_to_contact(me, o) < cfg.contact_time
    })
}

fn time_to_contact(me: &CarState, o: &CarState) -> f32 {
    let d = o.ground() - me.ground();
    let w = o.heading * o.speed() - me.heading * me.speed();
    let rr = (me.radius * 2.0 + o.radius * 2.0) * 0.5;
    let c = d.dot(d) - rr * rr;
    if c < 0.0 {
        return 0.0;
    }
    let a = w.dot(w);
    if a < 1.192_092_9e-7 {
        return NO_SPEED;
    }
    let b = w.dot(d);
    if b >= 0.0 {
        return NO_SPEED;
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return NO_SPEED;
    }
    (-b - disc.sqrt()) / a
}

/// `FUN_00501860` → `FUN_004fda60(…, −2)`: no limit when the other car is more than `hold_time_gap` seconds away
/// (gap along its heading / |relative velocity|); else the other car's speed − (gap − 2) × `hold_gain_mph`, within
/// ± `hold_window_mph` of it and at least `hold_min_target_mph`.
fn hold_speed(cfg: &AiConfig, me: &CarState, o: &CarState) -> f32 {
    let rel = [0, 1, 2].map(|k| o.velocity[k] - me.velocity[k]);
    let rv = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
    let ahead = (me.ground() - o.ground()).dot(o.heading);
    if (rv > 1.192_092_9e-7 || rv < -1.192_092_9e-7) && cfg.hold_time_gap < (ahead / rv).abs() {
        return NO_SPEED;
    }
    let vo = o.speed();
    let window = cfg.hold_window_mph * MPH;
    let mut v = vo - (ahead - 2.0) * cfg.hold_gain_mph * MPH;
    if v < vo - window {
        v = vo - window;
    }
    if vo + window <= v {
        v = vo + window;
    }
    v.max(cfg.hold_min_target_mph * MPH)
}

/// `FUN_004fd940`: target = min(target, v); nitrous only more than 5 m behind the other car (`.rdata` 0x00fa92d0)
/// or over 20 m from it, with +0x74 = 0 and +0x88 bit 5 clear, and a speed gap of 2.7 m/s to the target.
fn hold_apply(v: f32, me: &CarState, o: &CarState, cmd: &mut Command) {
    let target = cmd.target_speed.min(v);
    cmd.target_speed = target;
    let d = me.ground() - o.ground();
    let far = d.dot(o.heading) < -5.0 || 20.0 < d.length();
    cmd.nitrous = if far && me.field_74 == 0.0 && !me.flag_5 && me.speed() + 2.7 < target { 1.0 } else { 0.0 };
}

/// `FUN_004f9e80(target, speed)` with e = speed − target: 0 within 1 mph, −1 above 9 mph (`.rdata` 0x00fa8edc),
/// 1 at or below −1 mph, 1 − ramp(e; −1, 4 mph) up to 4 mph (0x00fa92b8; ramp = `FUN_005114e0`, a clamped
/// linear step), −ramp(e; 4, 9 mph) above.
pub fn recovery_pedal(target: f32, speed: f32) -> f32 {
    let ramp = |x: f32, a: f32, b: f32| ((x - a) / (b - a)).clamp(0.0, 1.0);
    let (lo, mid, hi) = (-MPH, 4.0 * MPH, 9.0 * MPH);
    let e = speed - target;
    if e.abs() < MPH {
        0.0
    } else if hi < e {
        -1.0
    } else if e <= mid {
        if e <= lo { 1.0 } else { 1.0 - ramp(e, lo, mid) }
    } else {
        -ramp(e, mid, hi)
    }
}

fn segment_hits_sphere(a: V2, b: V2, c: V2, r: f32) -> bool {
    let ab = b - a;
    let t = ((c - a).dot(ab) / ab.dot(ab).max(1e-6)).clamp(0.0, 1.0);
    (c - (a + ab * t)).length() <= r
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn car(id: i32, x: f32, z: f32, yaw: f32, speed: f32) -> CarState {
        // yaw 0 = +Z, positive towards +X (the car's left).
        let h = V2::new(yaw.sin(), yaw.cos());
        CarState {
            id,
            position: [x, 0.0, z],
            velocity: [h.x * speed, 0.0, h.z * speed],
            forward: [h.x, 0.0, h.z],
            heading: h,
            radius: 1.0,
            half_length: 2.3,
            field_74: 0.0,
            nitrous_tank: 1.0,
            physical: true,
            nitrous_allowed: true,
            flag_5: false,
            contact: false,
            all_wheels_off: false,
        }
    }

    fn input<'a>(me: &'a CarState, others: &'a [CarState]) -> RacerInput<'a> {
        RacerInput {
            dt: DT,
            me,
            others,
            local_player: None,
            schedule_speed: NO_SPEED,
            settings_speed: -1.0,
            lane_target: None,
            look_cap: None,
            sweep: (false, false),
            nitrous_mode_2: false,
            interaction: true,
            contact_partner: None,
            probes: [false; 4],
            visible: true,
        }
    }

    /// A toy kinematic car for the tests (not the game's physics): speed from the pedals, yaw rate from the
    /// steering (positive = right = yaw decreasing).
    fn step(c: &mut CarState, ctl: &VehicleControls, dt: f32) {
        let mut v = c.speed() + (ctl.throttle * 6.0 - ctl.brake * 10.0) * dt;
        v = v.clamp(0.0, 90.0);
        let yaw = c.heading.x.atan2(c.heading.z) - ctl.steering * 0.6 * v / 6.0 * dt;
        let h = V2::new(yaw.sin(), yaw.cos());
        c.heading = h;
        c.forward = [h.x, 0.0, h.z];
        c.velocity = [h.x * v, 0.0, h.z * v];
        c.position[0] += h.x * v * dt;
        c.position[2] += h.z * v * dt;
    }

    fn straight() -> Path {
        Path::new(&(0..60).map(|k| V2::new(0.0, 15.0 * k as f32)).collect::<Vec<_>>(), false)
    }

    /// 150 m north, then a right-hand 90° arc of radius 60 m (towards −X), then west.
    fn corner() -> Path {
        let mut pts: Vec<V2> = (0..=10).map(|k| V2::new(0.0, 15.0 * k as f32)).collect();
        for k in 1..=6 {
            let a = k as f32 / 6.0 * std::f32::consts::FRAC_PI_2;
            pts.push(V2::new(-60.0 + 60.0 * a.cos(), 150.0 + 60.0 * a.sin()));
        }
        pts.extend((1..20).map(|k| V2::new(-60.0 - 15.0 * k as f32, 210.0)));
        Path::new(&pts, false)
    }

    #[test]
    fn sweep_hits_a_corner_inside_the_turning_circle() {
        let cfg = AiConfig::default();
        let me = car(0, 0.0, 0.0, 0.0, 40.0);
        let d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let lengths = Driver::sweep_lengths(&cfg, &me, 100.0, false).unwrap();
        // R = 1600 / (2.8 × 9.81) ≈ 58 m. A straight road stays outside both circles.
        assert_eq!(d.sweep(&cfg, &straight(), &me, 2.8 * 9.81, lengths), (false, false));
        // A right-angle turn 30 m ahead: its exit lies inside the circle on that side.
        let mut pts: Vec<V2> = (0..3).map(|k| V2::new(0.0, 15.0 * k as f32)).collect();
        pts.extend((1..20).map(|k| V2::new(-15.0 * k as f32, 30.0)));
        let corner = Path::new(&pts, false);
        assert_eq!(d.sweep(&cfg, &corner, &me, 2.8 * 9.81, lengths), (true, true));
    }

    #[test]
    fn straight_full_throttle_and_no_steering() {
        let cfg = AiConfig::default();
        let path = straight();
        let mut me = car(1, 0.0, 1.0, 0.0, 0.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let mut last = TickOutput { command: Command::default(), controls: VehicleControls::default(), reset: false };
        for _ in 0..300 {
            last = d.tick(&cfg, &path, &input(&me, &[]));
            step(&mut me, &last.controls, DT);
        }
        assert_eq!(last.controls.throttle, 1.0);
        assert_eq!(last.controls.brake, 0.0);
        assert!(last.controls.steering.abs() < 1e-3, "{}", last.controls.steering);
        assert!(me.position[0].abs() < 0.05);
        assert!(d.node > 0);
    }

    #[test]
    fn steers_into_the_corner_and_follows_it() {
        let cfg = AiConfig::default();
        let path = corner();
        let mut me = car(1, 0.0, 1.0, 0.0, 20.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let mut max_err = 0.0f32;
        let mut steered_right_before = false;
        for _ in 0..60 * 20 {
            // The schedule holds 25 m/s (a rubber band cap).
            let out = d.tick(&cfg, &path, &RacerInput { schedule_speed: 25.0, ..input(&me, &[]) });
            if me.position[2] < 150.0 && me.position[2] > 120.0 && out.controls.steering > 0.1 {
                steered_right_before = true;
            }
            step(&mut me, &out.controls, DT);
            let p = me.ground();
            let s = path.nearest_segment(p);
            let a = path.nodes[s].position;
            let b = path.nodes[s + 1].position;
            let t = ((p - a).dot(b - a) / (b - a).dot(b - a)).clamp(0.0, 1.0);
            max_err = max_err.max((p - (a + (b - a) * t)).length());
        }
        assert!(steered_right_before, "no right steering approaching the right-hand corner");
        // Open: with this toy car the driver cuts the 60 m arc by ~12 m (not checked against the game's physics yet).
        assert!(max_err < 15.0, "{max_err}");
        assert!(me.position[0] < -150.0, "{:?}", me.position);
    }

    #[test]
    fn no_corner_braking_on_a_clear_road_but_brakes_down_to_the_schedule() {
        // The physical racer has no corner speed rule (0x004ff480 / 0x004ff550 are uncalled): approaching the
        // corner with no schedule it keeps the throttle.
        let cfg = AiConfig::default();
        let path = corner();
        let mut me = car(1, 0.0, 60.0, 0.0, 40.0);
        let mut d = Driver::new(path.nearest_segment(me.ground()), [0.3, 0.01, 0.1], 1);
        for _ in 0..60 {
            let out = d.tick(&cfg, &path, &input(&me, &[]));
            assert_eq!(out.controls.brake, 0.0);
            step(&mut me, &out.controls, DT);
        }
        // A schedule below the speed (the rubber band ahead): the pedal integrates down to braking.
        let mut braked = false;
        for _ in 0..120 {
            let out = d.tick(&cfg, &path, &RacerInput { schedule_speed: 20.0, ..input(&me, &[]) });
            braked |= out.controls.brake > 0.5;
        }
        assert!(braked);
    }

    #[test]
    fn emergency_brake_for_a_stopped_car_ahead() {
        let cfg = AiConfig::default();
        let path = straight();
        let me = car(1, 0.0, 10.0, 0.0, 40.0);
        let blocker = car(2, 0.0, 60.0, 0.0, 0.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &input(&me, &[blocker]));
        assert_eq!(out.controls.brake, 1.0);
        // Below 70 mph it does not act.
        let slow = car(1, 0.0, 10.0, 0.0, 30.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &input(&slow, &[blocker]));
        assert_eq!(out.controls.brake, 0.0);
    }

    #[test]
    fn recovery_backs_off_then_returns_to_racing() {
        let cfg = AiConfig::default();
        let path = straight();
        let mut me = car(1, 0.0, 10.0, 0.0, 0.0);
        me.contact = true;
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        // Contact under 10 mph: stuck after 1 s.
        let mut n = 0;
        while matches!(d.state, State::Racer) {
            d.tick(&cfg, &path, &input(&me, &[]));
            n += 1;
            assert!(n < 100);
        }
        assert!((55..=62).contains(&n), "{n}");
        // In reverse first (the Enter): the brake pedal is the drive pedal, both set.
        me.contact = false;
        let out = d.tick(&cfg, &path, &input(&me, &[]));
        assert_eq!(out.command.brake, 1.0);
        assert_eq!(out.command.throttle, 0.0);
        assert!(out.command.brake_set && out.command.throttle_set);
        // The backward probe flips the direction to forwards.
        let mut i = input(&me, &[]);
        i.probes[3] = true;
        let out = d.tick(&cfg, &path, &i);
        assert_eq!(out.command.throttle, 1.0);
        // Above 5 mph, facing the next node, clear: back to racing.
        let moving = car(1, 0.0, 10.0, 0.0, 4.0);
        d.tick(&cfg, &path, &input(&moving, &[]));
        assert_eq!(d.state, State::Racer);
    }

    #[test]
    fn reset_after_ten_seconds_in_the_recovery() {
        let cfg = AiConfig::default();
        let path = straight();
        let me = car(1, 0.0, 10.0, std::f32::consts::PI, 0.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        d.state = State::Recovery(Recovery::enter());
        let mut reset_at = None;
        for k in 0..60 * 11 {
            if d.tick(&cfg, &path, &input(&me, &[])).reset {
                reset_at = Some(k);
                break;
            }
        }
        assert!(reset_at.is_some_and(|k| (598..=600).contains(&k)), "{reset_at:?}");
        // Not visible: after 3 s.
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        d.state = State::Recovery(Recovery::enter());
        let hidden = RacerInput { visible: false, ..input(&me, &[]) };
        let k = (0..400).find(|_| d.tick(&cfg, &path, &hidden).reset);
        assert!(k.is_some_and(|k| (178..=180).contains(&k)), "{k:?}");
    }

    #[test]
    fn nitrous_on_the_straight_not_in_the_corner() {
        let cfg = AiConfig::default();
        let path = corner();
        let me = car(1, 0.0, 10.0, 0.0, 20.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &RacerInput { schedule_speed: 60.0, ..input(&me, &[]) });
        assert!(out.controls.nitrous);
        // Entering the arc: the turn ahead passes 5°.
        let me = car(1, -1.0, 148.0, 0.0, 20.0);
        let mut d = Driver::new(path.nearest_segment(me.ground()), [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &RacerInput { schedule_speed: 60.0, ..input(&me, &[]) });
        assert!(!out.controls.nitrous);
        // Without a schedule gap: none.
        let me = car(1, 0.0, 10.0, 0.0, 20.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &RacerInput { schedule_speed: 21.0, ..input(&me, &[]) });
        assert!(!out.controls.nitrous);
    }

    #[test]
    fn shunts_the_car_alongside_after_the_delay() {
        let cfg = AiConfig::default();
        let path = straight();
        let v = 100.0 * MPH;
        let me = car(5, 0.0, 10.0, 0.0, v);
        // 3 m to the right (−X), level.
        let other = car(9, -3.0, 10.0, 0.0, v);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 7);
        let mut first = None;
        for k in 0..120 {
            let out = d.tick(&cfg, &path, &input(&me, &[other]));
            if d.interaction == Interaction::Shunt {
                assert!(out.controls.steering > 0.4, "{}", out.controls.steering);
                first = Some(k);
                break;
            }
        }
        let k = first.expect("no shunt");
        let delay = d.shunt_delay;
        assert!((0.2..=1.0).contains(&delay));
        assert!(((k + 1) as f32 * DT - delay).abs() < 2.0 * DT, "{k} {delay}");
    }

    #[test]
    fn handbrake_and_full_brake_at_a_zero_target() {
        let cfg = AiConfig::default();
        let path = straight();
        let me = car(1, 0.0, 10.0, 0.0, 0.0);
        let mut d = Driver::new(0, [0.3, 0.01, 0.1], 1);
        let out = d.tick(&cfg, &path, &RacerInput { sweep: (false, true), ..input(&me, &[]) });
        assert_eq!(out.command.target_speed, 0.0);
        assert!(out.controls.handbrake);
        assert_eq!(out.controls.brake, 1.0);
    }

    #[test]
    fn recovery_pedal_bands() {
        let t = 10.0 * MPH;
        assert_eq!(recovery_pedal(t, t), 0.0);
        assert_eq!(recovery_pedal(t, 0.0), 1.0);
        assert_eq!(recovery_pedal(t, 20.0 * MPH), -1.0);
        assert!((recovery_pedal(t, 12.0 * MPH) - 0.4).abs() < 1e-5);
        assert!((recovery_pedal(t, 16.5 * MPH) + 0.5).abs() < 1e-5);
    }

    #[test]
    fn random_ring_returns_values_in_unit_range() {
        let mut r = AiRandom::seeded(0x01d9_0000_1234_5678);
        let v: Vec<f32> = (0..20).map(|_| r.draw()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        assert!(v.windows(2).any(|w| w[0] != w[1]));
    }
}
