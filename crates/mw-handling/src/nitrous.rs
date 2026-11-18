//! Nitrous: the per-tick physics of `FUN_009d2aa0` (update-loop step 5) and the gameplay tank behind its input
//! (docs/decomp.md "Nitrous and upgrades" and "Nitrous refill", docs/handling.md "Nitrous").
//!
//! **Physics.** The step reads the nitrous amount (vehicle controls +0x1c20, four lanes, all the same) and the
//! mode (+0x1c60) and writes a four-lane multiplier on the nitrous component (+0x10..+0x1c, default 1.0 from
//! `.rdata` 0x01026060):
//! - mode 0 (ordinary nitrous): multiplier = lerp(1, `Engine.Nitrous_strength.e9de413e`, amount)
//!   (1.9 on the 911).
//! - mode 1 (punch): multiplier = the strength; with a wheel down, a forward force
//!   `(1 − clamp((mph − Punch.5cb7c5aa) / (top mph − Punch.5cb7c5aa), 0, 1)) · amount · Punch.0798d257 · k · mass`
//!   (k = `.bss` 0x0118a018 = 0.44704, read live 2026-10-06: mph → m/s, so `0798d257` is an acceleration in
//!   mph per second; the 911's 40 is 17.9 m/s²).
//! - mode 3 (jump, airborne only): the same force with the `Jump` fields, multiplier lerp(1, `Jump.e9de413e`).
//!
//! The gearbox sees nitrous too (`FUN_009d4c20`, drivetrain.rs): upshifts on rpm alone and the top gear unlocked.
//!
//! **Tank** ([`Nitrous`], [`NitrousRules`]): the per-player nitrous state of the game logic (a 200-byte record per
//! player at game +0x264f0), driven each tick by `FUN_00669ab0`: the refill events (`FUN_00667c10`, the near
//! misses `FUN_00663270`), then the burn. The amount runs 0–100 (`.rdata` 0x00fb77e4); every award goes through
//! `FUN_0066e030` (× the balancing multiplier, 1 offline, clamped to 0..100). The rules and amounts are the car's
//! `NitrousParameters` variant (the fitted nitrous mod's, else `Vehicle_Gameplay.Nitrous_Offline` /
//! `_Online`, `FUN_0065de50`).

use mw_assets::handling::VehicleHandling;

/// The tank's capacity (`.rdata` 0x00fb77e4 = 100: the clamp of every award and `FUN_00658a90`'s 0.01 scale to
/// the HUD's 0..1).
pub const TANK: f32 = 100.0;

/// The car's nitrous data.
#[derive(Debug, Clone, Copy)]
pub struct NitrousParams {
    /// `Engine.Nitrous_strength.e9de413e` (engine +0x118 → +0x3c): the multiplier at full nitrous.
    pub strength: f32,
    /// `Nitrous_strength.Punch` {force `0798d257`, fade start mph `5cb7c5aa`} (+0x30, +0x34).
    pub punch: [f32; 2],
    /// `Nitrous_strength.Jump` {force `0798d257`, multiplier `e9de413e`, fade start mph `b959e02f`} (+0x18..+0x20).
    pub jump: [f32; 3],
    /// The tank's rules (the car's `NitrousParameters` variant).
    pub rules: NitrousRules,
}

impl NitrousParams {
    pub fn new(h: &VehicleHandling) -> Self {
        let g = |path: &str, d: f32| h.param(&format!("Engine.Nitrous_strength.{path}")).unwrap_or(d);
        Self {
            strength: g("e9de413e", 1.0),
            punch: [g("Punch.0798d257", 0.0), g("Punch.5cb7c5aa", 0.0)],
            jump: [g("Jump.0798d257", 0.0), g("Jump.e9de413e", 1.0), g("Jump.b959e02f", 0.0)],
            rules: NitrousRules::default(),
        }
    }

    /// Mode 0 of `FUN_009d2aa0`: the component's multiplier at nitrous `amount` (0–1).
    pub fn multiplier(&self, amount: f32) -> f32 {
        if amount <= 0.0 { 1.0 } else { 1.0 + (self.strength - 1.0) * amount }
    }

    /// Modes 1 / 3: the forward acceleration (m/s², to be multiplied by the mass) of the punch or jump force for
    /// its `0798d257` value in mph per second (`FUN_009d2aa0`, `PUNCH_SCALE`). Not run yet (only mode 0 is).
    pub fn force_accel(mph_per_s: f32, fade: f32, amount: f32) -> f32 {
        fade * amount * mph_per_s * PUNCH_SCALE
    }

    /// Modes 1 / 3: the speed fade of the punch and jump forces, `1 − clamp((mph − from) / (top − from), 0, 1)`.
    pub fn fade(mph: f32, from: f32, top_mph: f32) -> f32 {
        1.0 - ((mph - from) / (top_mph - from)).clamp(0.0, 1.0)
    }
}

/// mph → m/s on the punch and jump forces (`.bss` 0x0118a018 = 0.44704, read live 2026-10-06).
pub const PUNCH_SCALE: f32 = 0.44704;

/// One usage block (`NitrousParameters.NitrousUsage` / `PunchUsage`, the pointer the tank keeps at +0x70 while
/// burning): `Enabled` (`fc79a57e`), the burn per second (`053c5a6a`), the control ramp time (`845c3879`) and
/// the block's word at +0x1c: the minimum burn time (`NitrousUsage.7ebf362a` = 0.1 s) and, for the punch, also
/// the amount needed to start (`PunchUsage.fd8a5ed5`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub enabled: bool,
    pub burn: f32,
    pub ramp: f32,
    pub hold: f32,
    /// `PunchUsage.a82e62ca` (`PunchDelay`): no burn until this long after the start (0 for the ordinary usage).
    pub delay: f32,
}

/// A refill event's numbers (`NitrousParameters.<event>`). `award` is `aeba495e` (the event adds to the tank
/// only when set), `speed` its speed threshold in mph (`d629c55f`, or the event's own field), `amount` its rate
/// per second or its one-off amount (`d63cff99`), `time` its delay (or the event's other field).
/// `4d907940` is in every event but not read by the refill code.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Refill {
    pub award: bool,
    pub speed: f32,
    pub amount: f32,
    pub time: f32,
    /// A second time (oncoming: the grace after leaving the lane, `d6eeeb05`).
    pub time2: f32,
}

/// The refill events in the order of the game's per-player flag bytes (state +0x00..+0x0c; `FUN_00667c10`'s
/// switch, `FUN_00663270` for 8 / 9). Indices name the HUD messages too.
pub const EVENTS: [&str; 13] = [
    "min-speed",
    "high-speed",
    "shortcut",
    "oncoming",
    "slipstream",
    "drifting",
    "donut",
    "air",
    "near-miss",
    "oncoming-near-miss",
    "competitor",
    "takedown",
    "crash-escape",
];

/// The rules of the car's `NitrousParameters` variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NitrousRules {
    /// `NitrousUsage` (tank mode 1).
    pub usage: Usage,
    /// `PunchUsage` (tank mode 2, the Powershot nitrous).
    pub punch: Usage,
    /// `SpeedbreakerUsage.Enabled`, `JumpUsage.Enabled` (tank modes 3 and 4; not run).
    pub speedbreaker: bool,
    pub jump: bool,
    /// `Restrictions.c441f502`: the amount needed to start the ordinary nitrous (`FUN_00662a50`).
    pub min_amount: f32,
    /// `Restrictions.d5b487c7`: the speed (mph, |vehicle +0x1620|) needed to start any mode.
    pub min_mph: f32,
    /// Per event (index as [`EVENTS`]).
    pub events: [Refill; 13],
}

/// Field hashes of `NitrousParameters`' sub-blocks (`mwtool genesys-types`).
mod h {
    pub const ONCOMING: u32 = 0x6cd6d859;
    pub const DONUT: u32 = 0x663b6e21;
    pub const DRIFTING: u32 = 0x3de6e32c;
    pub const COMPETITOR: u32 = 0xbf7d7ffe;
    pub const PUNCH: u32 = 0xbc0e8824;
    pub const SLIPSTREAM: u32 = 0xa16337bd;
    pub const AIR: u32 = 0xe84409d7;
    pub const CRASH_ESCAPE: u32 = 0xf87103a0;
    pub const HIGH_SPEED: u32 = 0x78bbb7be;
    pub const MIN_SPEED: u32 = 0xee3a306e;
    pub const USAGE: u32 = 0x8ccdeab9;
    pub const SHORTCUT: u32 = 0x6dfa1226;
    pub const NEAR_MISS: u32 = 0x41390de7;
    pub const ONCOMING_NEAR_MISS: u32 = 0xa54c8ef1;
    pub const FATAL_HIT: u32 = 0x955d992e;
    pub const JUMP: u32 = 0x4a756d70;
    pub const RESTRICTIONS: u32 = 0x376b7d45;
    pub const SPEEDBREAKER: u32 = 0x722b7f00;
    // fields
    pub const ENABLED: u32 = 0xfc79a57e;
    pub const BURN: u32 = 0x053c5a6a;
    pub const RAMP: u32 = 0x845c3879;
    pub const AWARD: u32 = 0xaeba495e;
    pub const SPEED: u32 = 0xd629c55f;
    pub const AMOUNT: u32 = 0xd63cff99;
}

impl Default for NitrousRules {
    /// The ordinary nitrous of `Nitrous_Online` (the 911's `…6a590`, "BURN NITROUS") without any refill: used
    /// when a car has no `NitrousParameters` (the AI's cars refill on their own, `ai.rs`).
    fn default() -> Self {
        Self {
            usage: Usage { enabled: true, burn: 24.0, ramp: 1.0, hold: 0.1, delay: 0.0 },
            punch: Usage::default(),
            speedbreaker: false,
            jump: false,
            min_amount: 1.0,
            min_mph: 10.0,
            events: [Refill::default(); 13],
        }
    }
}

impl NitrousRules {
    /// From a `NitrousParameters` object's numbers: `get(block hash, field hash)` (bools as 0 / 1,
    /// `mw_assets::mods::nitrous_parameters`). Field offsets as the game reads them (docs/decomp.md
    /// "Nitrous refill").
    pub fn from_fields(get: impl Fn(u32, u32) -> Option<f32>) -> Self {
        let f = |b: u32, k: u32| get(b, k).unwrap_or(0.0);
        let on = |b: u32, k: u32| f(b, k) != 0.0;
        let usage = |b: u32, hold: u32, delay: u32| Usage {
            enabled: on(b, h::ENABLED),
            burn: f(b, h::BURN),
            ramp: f(b, h::RAMP),
            hold: f(b, hold),
            delay: if delay == 0 { 0.0 } else { f(b, delay) },
        };
        let ev = |b: u32, speed: u32, amount: u32, time: u32, time2: u32| Refill {
            award: on(b, h::AWARD),
            speed: if speed == 0 { 0.0 } else { f(b, speed) },
            amount: f(b, amount),
            time: if time == 0 { 0.0 } else { f(b, time) },
            time2: if time2 == 0 { 0.0 } else { f(b, time2) },
        };
        let mut events = [Refill::default(); 13];
        events[0] = ev(h::MIN_SPEED, h::SPEED, h::AMOUNT, 0, 0);
        // HighSpeed: `21948e6c` = the percentage of the car's top speed.
        events[1] = ev(h::HIGH_SPEED, 0x21948e6c, h::AMOUNT, 0, 0);
        events[2] = ev(h::SHORTCUT, h::SPEED, h::AMOUNT, 0, 0);
        // TrafficOncoming: `1732c75e` delay, `d6eeeb05` grace.
        events[3] = ev(h::ONCOMING, h::SPEED, h::AMOUNT, 0x1732c75e, 0xd6eeeb05);
        // Slipstreaming: `efb8084e` = the slipstream percentage needed.
        events[4] = ev(h::SLIPSTREAM, h::SPEED, h::AMOUNT, 0, 0);
        events[4].time = f(h::SLIPSTREAM, 0xefb8084e);
        // Drifting: `TimeDrifting` (`f003dfe5`).
        events[5] = ev(h::DRIFTING, h::SPEED, h::AMOUNT, 0xf003dfe5, 0);
        // Donutting: `02318b5f` = the speed limit (mph, at most), `959a0ec3` the delay.
        events[6] = ev(h::DONUT, 0x02318b5f, h::AMOUNT, 0x959a0ec3, 0);
        // CatchingAir: `c8e9863d` = the time in the air needed.
        events[7] = ev(h::AIR, 0, h::AMOUNT, 0xc8e9863d, 0);
        events[8] = ev(h::NEAR_MISS, h::SPEED, h::AMOUNT, 0, 0);
        events[9] = ev(h::ONCOMING_NEAR_MISS, h::SPEED, h::AMOUNT, 0, 0);
        // HittingAnotherCompetitor: `1732c75e` = the contact time.
        events[10] = ev(h::COMPETITOR, h::SPEED, h::AMOUNT, 0x1732c75e, 0);
        events[11] = ev(h::FATAL_HIT, 0, h::AMOUNT, 0, 0);
        events[12] = ev(h::CRASH_ESCAPE, h::SPEED, h::AMOUNT, 0, 0);
        Self {
            usage: usage(h::USAGE, 0x7ebf362a, 0),
            punch: usage(h::PUNCH, 0xfd8a5ed5, 0xa82e62ca),
            speedbreaker: on(h::SPEEDBREAKER, h::ENABLED),
            jump: on(h::JUMP, h::ENABLED),
            min_amount: f(h::RESTRICTIONS, 0xc441f502),
            min_mph: f(h::RESTRICTIONS, 0xd5b487c7),
            events,
        }
    }
}

/// Seconds → the game's timer ticks are 3e6 per second (`.rdata` 0x00f9f604); a countdown has run out once
/// under 3 ticks (`FUN_0064cd40`: ticks / 3 < 1).
const EXPIRED: f32 = 3.0 / 3.0e6;

fn expired(t: f32) -> bool {
    t < EXPIRED
}

/// Grace after leaving an oncoming lane off the road network (`.rdata` 0x00fb865c = 9e6 ticks = 3 s).
pub const ONCOMING_OFF_ROAD_GRACE: f32 = 3.0;
/// Donut: the drift's donut weight (drift +0x80, 1 − the yaw torque's speed blend) must be at most this
/// (`.rdata` 0x00fad860 = 0.95, the racer vehicle's virtual +0x6c `FUN_00572860`).
pub const DONUT_WEIGHT_MAX: f32 = 0.95;
/// While the tank burns the throttle is forced to 1 once the control amount is above 0 or below this speed
/// (mph, signed; `.rdata` 0x00fc397c, `FUN_006c1a80`).
pub const FULL_THROTTLE_BELOW_MPH: f32 = 15.0;
/// At most this many distinct cars per near-miss window (the id lists at state +0x78 / +0x90, `FUN_0065e280`).
pub const NEAR_MISS_LIST: usize = 10;
/// The near-miss window (`FUN_0065e280`: `.rdata` 0x00fb8660 = 1.5e6 ticks = 0.5 s).
pub const NEAR_MISS_WINDOW: f32 = 0.5;
/// The near-miss query (`FUN_00665f30` → the world's sphere query `FUN_00aa0440`): traffic cars (broadphase
/// category bit 3) whose bounding sphere meets a sphere of this radius (m, `.rdata` 0x00fb82b0, splatted into
/// `.bss` 0x011777a0 at start-up) around the car's position (vehicle +0x110).
pub const NEAR_MISS_RADIUS: f32 = 5.0;

/// Where the car is on the road network for the oncoming test (`FUN_0065eec0` on the road position the game
/// keeps per player).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lane {
    /// No road under the car (point index −1).
    #[default]
    OffRoad,
    /// On a road: in one of its oncoming lanes or not.
    OnRoad { oncoming: bool },
}

/// What the game logic sees of the car this tick (the refill events' inputs).
#[derive(Debug, Clone, Default)]
pub struct World<'a> {
    /// Run the refill events (the player; the AI's cars have their own constant refill, `ai.rs`).
    pub events: bool,
    /// |velocity| in mph (vehicle +0x120 × `.bss` 0x0118a090 = 2.23694): the events' speed.
    pub mph: f32,
    /// The signed forward speed in mph (vehicle +0x1620): the start's and the throttle rule's speed.
    pub forward_mph: f32,
    /// The car's top speed for HighSpeed (`VehicleList` entry +0x3c, mph).
    pub top_mph: f32,
    /// The drift is on (drift +0x14, the vehicle's virtual +0x34).
    pub drifting: bool,
    /// The drift's donut weight (drift +0x80).
    pub donut_weight: f32,
    /// Seconds in the air (vehicle +0x1630).
    pub air_time: f32,
    pub lane: Lane,
    /// On a shortcut road (`RoadData` road flag bit 1).
    pub shortcut: bool,
    /// The slipstream amount, 0..1 (racer vehicle +0xc60, `FUN_0063c790`).
    pub slipstream: f32,
    /// Touching another race car (`FUN_00658e80`: the contact's player is an AI racer).
    pub touching_competitor: bool,
    /// This car took another car down this tick (`FUN_006b8d00` sets state +0x76).
    pub takedown: bool,
    /// The crash-escape count (game stats +0x2c, `FUN_00656a90`).
    pub crash_escapes: u32,
    /// The near-miss query's traffic cars this tick ([`NEAR_MISS_RADIUS`]): (id, same direction: the car's
    /// forward axis · our velocity ≥ 0).
    pub near_cars: &'a [(u32, bool)],
    /// Touching a traffic car (`FUN_006564c0`): no query, the window is cancelled (`FUN_00665f30`).
    pub near_miss_cancel: bool,
    /// Seconds of the near-miss window ([`NEAR_MISS_WINDOW`]).
    pub near_miss_window: f32,
}

/// The tank: the game's per-player nitrous state (`FUN_00658c90` clears it: everything 0).
#[derive(Debug, Clone, Default)]
pub struct Nitrous {
    /// 0..=[`TANK`] (state +0x10).
    pub tank: f32,
    /// 0 idle, 1 nitrous, 2 punch (state +0x6c; 3 / 4 speedbreaker / jump are not run).
    pub mode: u8,
    /// The tank burns (the HUD's `UsingNitrous`, mode ≠ 0).
    pub burning: bool,
    /// The control amount sent to the physics (state +0x18).
    pub control: f32,
    /// Seconds since the burn started (state +0x1c).
    elapsed: f32,
    /// Minimum burn countdown (state +0x60).
    hold: f32,
    held: bool,
    /// Per event, active this tick (state +0x00..+0x0c) and the amount it added.
    pub active: [bool; 13],
    pub added: [f32; 13],
    /// Countdowns (s): drifting +0x28, donut +0x30, oncoming +0x40 / +0x48 / +0x50, competitor +0x58.
    drift_timer: f32,
    donut_timer: f32,
    oncoming_delay: f32,
    oncoming_grace: f32,
    oncoming_off_road: f32,
    competitor_timer: f32,
    /// Crash escapes seen (state +0xa8).
    crash_escapes: u32,
    /// Near misses: the window (+0x74, +0x38), the cars of this window (+0x78, +0x90) and their counts.
    near_window: bool,
    near_cancelled: bool,
    near_timer: f32,
    pub near_same: Vec<u32>,
    pub near_oncoming: Vec<u32>,
    /// The window closed on the last tick: its lists are cleared before this tick's cars.
    near_spent: bool,
}

impl Nitrous {
    /// A full tank (gas stations award 100, `FUN_00748ff0`; tests start full).
    pub fn full() -> Self {
        Self { tank: TANK, ..Default::default() }
    }

    /// Tank fill, 0–1 (the HUD's `NitrousState.NitrousAmount`, `FUN_00658a90`).
    pub fn fill(&self) -> f32 {
        self.tank / TANK
    }

    /// `FUN_00658ad0`: add to the tank (clamped to 0..100), e.g. the AI's constant refill.
    pub fn add(&mut self, units: f32) {
        self.tank = (self.tank + units).clamp(0.0, TANK);
    }

    /// `FUN_0066e030`: an event's award, × the balancing refill multiplier (state +0xb8, 1 offline).
    fn award(&mut self, event: usize, units: f32) {
        let before = self.tank;
        self.add(units);
        self.added[event] += self.tank - before;
    }

    /// One game-logic tick: the input (`FUN_006cd3e0`), the refill events (`FUN_00667c10`, `FUN_00663270`) and
    /// the burn (`FUN_00669ab0`). Returns the control amount for the physics (controls +0x1c, `FUN_006c1a80`).
    pub fn update(&mut self, r: &NitrousRules, w: &World, held: bool, dt: f32) -> f32 {
        let pressed = held && !self.held;
        self.held = held;
        // Input (`FUN_006cd3e0`): a press starts a burn in the first enabled mode (`FUN_00662e30`); letting go
        // stops it once the minimum burn time has run out (`FUN_00658a40`).
        if pressed && self.mode == 0 {
            let speed_ok = w.forward_mph.abs() >= r.min_mph;
            if r.usage.enabled {
                // `FUN_00662a50` / `FUN_00662ea0`.
                if self.tank >= r.min_amount && speed_ok {
                    self.start(1, r.usage.hold);
                }
            } else if r.punch.enabled {
                // `FUN_00662b40` / `FUN_00662f20`: the punch needs `fd8a5ed5` in the tank.
                if self.tank >= r.punch.hold && speed_ok {
                    self.start(2, r.punch.hold);
                }
            }
        } else if !held && self.mode != 0 && expired(self.hold) {
            self.mode = 0;
        }
        self.added = [0.0; 13];
        if w.events {
            self.refill(r, w, dt);
            self.near_misses(r, w, dt);
        }
        self.burn(r, w, dt);
        self.burning = self.mode != 0;
        self.control
    }

    fn start(&mut self, mode: u8, hold: f32) {
        self.mode = mode;
        self.hold = hold;
    }

    /// `FUN_00669ab0` after the events: burn `053c5a6a` per second (the punch only after its delay), ramp the
    /// control amount over `845c3879` seconds (instant when 0: the game's 0/0 is NaN and clamps to 0 on the first
    /// tick, ∞ after), stop when the tank is empty, run the minimum-burn countdown.
    fn burn(&mut self, r: &NitrousRules, _w: &World, dt: f32) {
        if self.mode == 0 {
            (self.elapsed, self.control) = (0.0, 0.0);
            return;
        }
        let u = if self.mode == 2 { &r.punch } else { &r.usage };
        if self.mode != 2 || u.delay <= self.elapsed {
            self.tank = (self.tank - u.burn * dt).max(0.0);
            self.control = 0.0;
            if self.tank > 0.0 {
                let x = self.elapsed / u.ramp;
                self.control = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
            }
        }
        self.elapsed += dt;
        if self.tank <= 0.0 {
            self.mode = 0;
            self.control = 0.0;
        } else if !expired(self.hold) {
            self.hold -= dt;
        }
    }

    /// `FUN_00667c10`: the continuous and one-off refills (events 0–7, 10–12).
    fn refill(&mut self, r: &NitrousRules, w: &World, dt: f32) {
        let e = &r.events;
        let v = w.mph;
        // 0 MinSpeed: faster than `d629c55f`.
        self.active[0] = v > e[0].speed;
        if self.active[0] && e[0].award {
            self.award(0, e[0].amount * dt);
        }
        // 1 HighSpeed: speed / top speed × 100 above `21948e6c` (× the balancing multiplier +0xb0, 1).
        self.active[1] = w.top_mph > 0.0 && v / w.top_mph * TANK > e[1].speed;
        if self.active[1] && e[1].award {
            self.award(1, e[1].amount * dt);
        }
        // 2 TakingShortcut: faster than `d629c55f` on a shortcut road.
        self.active[2] = v > e[2].speed && w.shortcut;
        if self.active[2] && e[2].award {
            self.award(2, e[2].amount * dt);
        }
        // 3 TrafficOncoming: faster than `d629c55f`, `1732c75e` s in an oncoming lane, then the rate while in it
        // and for `d6eeeb05` s after leaving it (3 s when off the road network).
        let o = e[3];
        let reset = |s: &mut Self| {
            (s.oncoming_delay, s.oncoming_grace, s.oncoming_off_road) = (o.time, o.time2, ONCOMING_OFF_ROAD_GRACE);
            s.active[3] = false;
        };
        if v <= o.speed {
            reset(self);
        } else if w.lane == (Lane::OnRoad { oncoming: true }) {
            if expired(self.oncoming_delay) {
                if o.award {
                    self.award(3, o.amount * dt);
                }
                (self.oncoming_grace, self.oncoming_off_road) = (o.time2, ONCOMING_OFF_ROAD_GRACE);
                self.active[3] = true;
            } else {
                self.oncoming_delay -= dt;
                self.active[3] = false;
            }
        } else {
            let grace = if w.lane == Lane::OffRoad { self.oncoming_off_road } else { self.oncoming_grace };
            if self.active[3] && !expired(grace) {
                if o.award {
                    self.award(3, o.amount * dt);
                }
                if w.lane == Lane::OffRoad {
                    self.oncoming_off_road -= dt;
                } else {
                    self.oncoming_grace -= dt;
                }
            } else {
                reset(self);
            }
        }
        // 4 Slipstreaming: slipstream × 100 above `efb8084e` and faster than `d629c55f`.
        self.active[4] = w.slipstream * TANK > e[4].time && v > e[4].speed;
        if self.active[4] && e[4].award {
            self.award(4, e[4].amount * dt);
        }
        // 5 Drifting: in a drift at `d629c55f` or more for `TimeDrifting` s.
        let d = e[5];
        if !(w.drifting && v >= d.speed) {
            (self.drift_timer, self.active[5]) = (d.time, false);
        } else if expired(self.drift_timer) {
            if d.award {
                self.award(5, d.amount * dt);
            }
            self.active[5] = true;
        } else {
            (self.drift_timer, self.active[5]) = (self.drift_timer - dt, false);
        }
        // 6 Donutting: a drift with the donut weight at most 0.95, at `02318b5f` mph or less, for `959a0ec3` s.
        let d = e[6];
        if !(w.drifting && w.donut_weight <= DONUT_WEIGHT_MAX && v <= d.speed) {
            (self.donut_timer, self.active[6]) = (d.time, false);
        } else if expired(self.donut_timer) {
            if d.award {
                self.award(6, d.amount * dt);
            }
            self.active[6] = true;
        } else {
            (self.donut_timer, self.active[6]) = (self.donut_timer - dt, false);
        }
        // 7 CatchingAir: more than `c8e9863d` s in the air.
        self.active[7] = w.air_time > e[7].time;
        if self.active[7] && e[7].award {
            self.award(7, e[7].amount * dt);
        }
        // 10 HittingAnotherCompetitor: touching a racer faster than `d629c55f` for `1732c75e` s: once per contact.
        let c = e[10];
        if !(v > c.speed && w.touching_competitor) {
            (self.competitor_timer, self.active[10]) = (c.time, false);
        } else if expired(self.competitor_timer) {
            if !self.active[10] {
                if c.award {
                    self.award(10, c.amount);
                }
                self.active[10] = true;
            }
        } else {
            (self.competitor_timer, self.active[10]) = (self.competitor_timer - dt, false);
        }
        // 11 FatalHit (a takedown): the amount at once.
        if w.takedown && e[11].award {
            self.award(11, e[11].amount);
        }
        self.active[11] = w.takedown;
        // 12 CrashEscape: the count went up while faster than `d629c55f`: the amount at once.
        self.active[12] = v > e[12].speed && self.crash_escapes < w.crash_escapes;
        if self.active[12] && e[12].award {
            self.award(12, e[12].amount);
        }
        self.crash_escapes = w.crash_escapes;
    }

    /// `FUN_00665f30` / `FUN_0065e280` / `FUN_00663270`: the near-miss window. The first car in the query starts
    /// it; every distinct car until it closes counts (same direction or oncoming, at most 10 each); when it
    /// closes, each kind with a count awards its amount once if faster than its `d629c55f`.
    fn near_misses(&mut self, r: &NitrousRules, w: &World, dt: f32) {
        let fire = if w.near_miss_cancel {
            // Touching traffic: no query this tick, the window closes and stays cancelled (+0x75) until a query
            // comes back empty.
            (self.near_window, self.near_cancelled) = (false, true);
            self.near_same.clear();
            self.near_oncoming.clear();
            false
        } else {
            if std::mem::take(&mut self.near_spent) {
                self.near_same.clear();
                self.near_oncoming.clear();
            }
            for &(id, same) in w.near_cars {
                let list = if same { &mut self.near_same } else { &mut self.near_oncoming };
                if !list.contains(&id) && list.len() < NEAR_MISS_LIST {
                    list.push(id);
                }
            }
            if self.near_cancelled {
                (self.near_window, self.near_cancelled) = (false, !w.near_cars.is_empty());
                self.near_same.clear();
                self.near_oncoming.clear();
                false
            } else if !self.near_window {
                if w.near_cars.is_empty() {
                    self.near_same.clear();
                    self.near_oncoming.clear();
                } else {
                    (self.near_window, self.near_timer) = (true, w.near_miss_window);
                }
                false
            } else if expired(self.near_timer) {
                self.near_window = false;
                true
            } else {
                self.near_timer -= dt;
                false
            }
        };
        for (k, n) in [(8, self.near_same.len()), (9, self.near_oncoming.len())] {
            let ev = r.events[k];
            self.active[k] = fire && w.mph > ev.speed && n > 0;
            if self.active[k] && ev.award {
                self.award(k, ev.amount);
            }
        }
        // The window has closed: its cars are spent (distinct ids per window; a car still near opens the next).
        // Cleared on the next tick, so the lists still show the closed window's cars until then.
        self.near_spent = fire;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 911's `Nitrous_Online` (`…6a590`, BURN NITROUS) as `mwtool genesys` prints it.
    fn online() -> NitrousRules {
        let mut r = NitrousRules::default();
        let ev = |award, speed, amount, time, time2| Refill { award, speed, amount, time, time2 };
        r.events = [
            ev(false, 0.0, 1.8, 0.0, 0.0),
            ev(true, 35.0, 1.5, 0.0, 0.0),
            ev(false, 70.0, 0.6, 0.0, 0.0),
            ev(true, 20.0, 1.2, 0.25, 0.5),
            ev(true, 50.0, 3.0, 50.0, 0.0),
            ev(true, 30.0, 6.0, 0.75, 0.0),
            ev(true, 30.0, 2.0, 0.75, 0.0),
            ev(true, 0.0, 24.0, 0.1, 0.0),
            ev(true, 80.0, 3.0, 0.0, 0.0),
            ev(true, 80.0, 6.0, 0.0, 0.0),
            ev(true, 50.0, 18.0, 0.05, 0.0),
            ev(true, 0.0, 100.0, 0.0, 0.0),
            ev(true, 50.0, 100.0, 0.0, 0.0),
        ];
        r
    }

    const DT: f32 = 1.0 / 60.0;

    fn run(n: &mut Nitrous, r: &NitrousRules, w: &World, held: bool, secs: f32) {
        for _ in 0..(secs / DT).round() as usize {
            n.update(r, w, held, DT);
        }
    }

    #[test]
    fn drifting_refills_after_the_delay() {
        let r = online();
        let mut n = Nitrous::default();
        let w = World { events: true, mph: 60.0, top_mph: 190.0, drifting: true, ..Default::default() };
        // A tick out of the drift arms the delay (the countdowns start at 0).
        n.update(&r, &World { drifting: false, ..w.clone() }, false, DT);
        // 60 / 190 = 32 % of the top speed: no HighSpeed.
        run(&mut n, &r, &w, false, 0.75);
        assert!(n.tank < 0.1, "{}", n.tank);
        run(&mut n, &r, &w, false, 2.0);
        assert!((n.tank - 12.0).abs() < 0.3, "{}", n.tank);
    }

    #[test]
    fn high_speed_and_air() {
        let r = online();
        let mut n = Nitrous::default();
        let w = World { events: true, mph: 100.0, top_mph: 190.0, air_time: 0.5, ..Default::default() };
        run(&mut n, &r, &w, false, 1.0);
        assert!((n.tank - 25.5).abs() < 0.2, "{}", n.tank);
        assert!(n.active[1] && n.active[7]);
    }

    #[test]
    fn oncoming_grace() {
        let r = online();
        let mut n = Nitrous::default();
        let on = World { events: true, mph: 40.0, lane: Lane::OnRoad { oncoming: true }, ..Default::default() };
        n.update(&r, &World { mph: 0.0, ..on.clone() }, false, DT);
        run(&mut n, &r, &on, false, 1.25);
        assert!((n.tank - 1.2).abs() < 0.05, "{}", n.tank);
        let off = World { lane: Lane::OnRoad { oncoming: false }, ..on.clone() };
        run(&mut n, &r, &off, false, 1.0);
        // 0.5 s more after leaving the lane.
        assert!((n.tank - 1.8).abs() < 0.05, "{}", n.tank);
    }

    #[test]
    fn competitor_once_per_contact() {
        let r = online();
        let mut n = Nitrous::default();
        let w = World { events: true, mph: 60.0, touching_competitor: true, ..Default::default() };
        run(&mut n, &r, &w, false, 1.0);
        assert_eq!(n.tank, 18.0);
        let apart = World { touching_competitor: false, ..w.clone() };
        run(&mut n, &r, &apart, false, 0.1);
        run(&mut n, &r, &w, false, 0.2);
        assert_eq!(n.tank, 36.0);
    }

    #[test]
    fn near_miss_window_awards_once() {
        let r = online();
        let mut n = Nitrous::default();
        let cars = [(7, true), (9, false)];
        let w = World { events: true, mph: 90.0, near_cars: &cars, near_miss_window: 0.5, ..Default::default() };
        n.update(&r, &w, false, DT);
        let quiet = World { near_cars: &[], ..w.clone() };
        run(&mut n, &r, &quiet, false, 1.0);
        assert!((n.tank - 9.0).abs() < 1e-4, "{}", n.tank);
        // Too slow: nothing.
        let mut m = Nitrous::default();
        let slow = World { mph: 70.0, ..w.clone() };
        m.update(&r, &slow, false, DT);
        run(&mut m, &r, &World { near_cars: &[], ..slow.clone() }, false, 1.0);
        assert_eq!(m.tank, 0.0);
    }

    #[test]
    fn near_miss_lists_are_per_window() {
        // An oncoming car in one window does not count again in the next one ("distinct ids per window").
        let r = online();
        let mut n = Nitrous::default();
        let w =
            World { events: true, mph: 90.0, near_cars: &[(9, false)], near_miss_window: 0.5, ..Default::default() };
        n.update(&r, &w, false, DT);
        let same = World { near_cars: &[(7, true)], ..w.clone() };
        let mut ticks = 0;
        while n.tank == 0.0 {
            n.update(&r, &same, false, DT);
            ticks += 1;
            assert!(ticks < 120);
        }
        // Both kinds in the first window.
        assert!((n.tank - 9.0).abs() < 1e-4, "{}", n.tank);
        // The same-direction car opens a second window at once: only its kind awards when it closes.
        n.update(&r, &same, false, DT);
        run(&mut n, &r, &World { near_cars: &[], ..w.clone() }, false, 1.0);
        assert!((n.tank - 12.0).abs() < 1e-4, "{}", n.tank);
    }

    #[test]
    fn burn_starts_on_a_press_and_ramps() {
        let r = online();
        let mut n = Nitrous::full();
        let w = World { forward_mph: 5.0, ..Default::default() };
        // Too slow to start (`Restrictions.d5b487c7` = 10 mph).
        n.update(&r, &w, true, DT);
        assert_eq!(n.mode, 0);
        // Still held when fast enough: no new press, no start.
        let fast = World { forward_mph: 40.0, ..w.clone() };
        n.update(&r, &fast, true, DT);
        assert_eq!(n.mode, 0);
        n.update(&r, &fast, false, DT);
        let c = n.update(&r, &fast, true, DT);
        assert_eq!((n.mode, c), (1, 0.0));
        run(&mut n, &r, &fast, true, 0.5);
        assert!((n.control - 0.5).abs() < 0.02, "{}", n.control);
        // 24 per second: empty about 4.2 s after the start.
        run(&mut n, &r, &fast, true, 3.7);
        assert_eq!(n.mode, 0);
        assert_eq!(n.tank, 0.0);
    }

    #[test]
    fn short_tap_burns_the_minimum_time() {
        let r = online();
        let mut n = Nitrous::full();
        let w = World { forward_mph: 40.0, ..Default::default() };
        n.update(&r, &w, true, DT);
        run(&mut n, &r, &w, false, 0.05);
        assert_eq!(n.mode, 1);
        run(&mut n, &r, &w, false, 0.1);
        assert_eq!(n.mode, 0);
    }

    #[test]
    fn rules_from_fields() {
        let mut m = std::collections::HashMap::new();
        m.insert((h::DRIFTING, h::AMOUNT), 6.0);
        m.insert((h::DRIFTING, h::AWARD), 1.0);
        m.insert((h::DRIFTING, 0xf003dfe5), 0.75);
        m.insert((h::USAGE, h::ENABLED), 1.0);
        m.insert((h::USAGE, h::BURN), 24.0);
        m.insert((h::RESTRICTIONS, 0xd5b487c7), 10.0);
        let r = NitrousRules::from_fields(|b, f| m.get(&(b, f)).copied());
        assert_eq!(r.events[5], Refill { award: true, speed: 0.0, amount: 6.0, time: 0.75, time2: 0.0 });
        assert!(r.usage.enabled && r.usage.burn == 24.0 && r.min_mph == 10.0);
    }
}
