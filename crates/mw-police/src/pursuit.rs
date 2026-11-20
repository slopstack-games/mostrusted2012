//! The pursuit manager: heat, the chase / cooldown / busted / escaped states, the bust meter, the pursuit bar and
//! the scoring actions a pursuit posts. One record per player (0x8b0 bytes at `*(world + 0x2f410)`), updated by
//! `FUN_0066b900` → `FUN_0066ae60` every frame. docs/decomp.md "Police and pursuits" and "Police and pursuit
//! (ported)".
//!
//! Every constant names its source: a `GameLogicConfig` / `HeatLevel` field (`mw_assets::police`) or an `.rdata`
//! value read from the exe on disk (`exe-const`).

use glam::Vec3;
use mw_assets::police::{HeatLevel, PoliceConfig};

use crate::heat;
use crate::score::ScoreSlot;

/// `.rdata` 0x00fb77e4: range (m) of the nearest-cop term of the pursuit bar, of the roadblock checks
/// (`FUN_0083ef30` callers) and of a roadblock's sight (`FUN_00657ee0`).
pub const NEAR_RANGE: f32 = 100.0;
/// `.rdata` 0x00fb74c8: the heat does not cool while a cop is within this distance (m; `FUN_006615c0`).
pub const COOLDOWN_CLEAR_RANGE: f32 = 20.0;
/// `.rdata` 0x00fb74b0: the cooldown rate's factor when the player's vehicle +0x1a84 is 0 (`FUN_006615c0`; the
/// player's own car reads 2 in a live read, 2026-10-14).
pub const COOLDOWN_FAST: f32 = 4.0;
/// `.rdata` 0x00fb82d0: a cop more than this far above or below the player cannot bust (m; `FUN_0065cda0`).
pub const BUST_HEIGHT: f32 = 4.0;
/// `.rdata` 0x00f8a810: the bust speed's factor for a cop that is near but not touching (`FUN_0065cda0`).
pub const BUST_NEAR_SPEED_FACTOR: f32 = 0.5;
/// `.rdata` 0x00fc0608: the BUSTING bar shows once the bust meter passes this (`FUN_006bdf50`), until it is back
/// at zero.
pub const BUST_BAR_SHOWN_ABOVE: f32 = 0.3;
/// The pursuit bar (`FUN_00664d60`): 0.8 (`.rdata` 0x00fb74cc), 0.5 (0x00f8a810), 0.2 (0x00fb867c), 50 m
/// (0x00fb821c), 300 mph (0x00fb74bc), the clamp ±0.5 (0x00f9c3a0 / 0x00f8a810), the bust meter's 0.01
/// (0x00f9d474), the cap −0.2 without a cooldown (0x00fa9318) and the floor −0.9 (0x00fb8678).
const BAR_FAR: f32 = 0.8;
const BAR_CLOSE: f32 = 0.2;
const BAR_CLOSE_RANGE: f32 = 50.0;
const BAR_TOP_MPH: f32 = 300.0;
const BAR_LIMIT: f32 = 0.5;
const BAR_METER: f32 = 0.01;
const BAR_NO_COOLDOWN: f32 = -0.2;
const BAR_FLOOR: f32 = -0.9;
/// `.rdata` 0x00fb77ec: seconds in the "being busted" range before the manager posts cue 1 (`FUN_006f3f30`).
const BAR_CUE_TIME: f32 = 3.0;

/// The pursuit bar's three ranges (`FUN_00661500`, manager +0x8b0 / +0x8bc / +0x8c8: lower bound, upper bound,
/// kind): 0 = being busted, 1 = cops close, 2 = getting away (`Player.PursuitBarEnum`).
pub const BAR_RANGES: [(f32, f32, u8); 3] = [(-1.0, -0.5, 0), (-0.65, 0.0, 1), (0.0, 1.0, 2)];

/// `rec + 0x89c` (`Player.IsInChase` and friends read it; the telemetry names "Busted" / "Escaped").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PursuitState {
    /// 0: no pursuit.
    #[default]
    None,
    /// 1: chased.
    Chase,
    /// 2: busted (set by the game logic's action 0xf, `FUN_00658140`).
    Busted,
    /// 3: the cooldown: the cops lost the player and the heat cools.
    Cooldown,
    /// 4: escaped.
    Escaped,
}

/// What a pursuit tick reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PursuitEvent {
    /// A scoring action posted (`FUN_006a8050`); its heat has been added when it has one in the config.
    Score(ScoreSlot),
    /// The chase began (`FUN_0065c550` from no pursuit).
    Started,
    /// The cops lost the player: the cooldown began (`FUN_0065cbe0`).
    Cooldown,
    /// A cop or a roadblock found the player in the cooldown: chased again (`FUN_0065c970`).
    Resumed,
    /// The level number rose / fell to this (`FUN_0065d080` / `FUN_0065d160`; the HUD message is the
    /// [`ScoreSlot::HeatUp`] / [`ScoreSlot::HeatDown`] action, posted only on the game's conditions).
    LevelUp(u8),
    LevelDown(u8),
    /// The bar reached −1: camera event 0x1a (the player's car, the cop's, 100) posted (`FUN_00664d60` →
    /// `FUN_0066fae0`); the game logic's state machine sets the state to busted (action 0xf, `FUN_00658140`;
    /// inferred link). `cop` = index of the cop in the input.
    BustCaught {
        cop: usize,
    },
    /// The state became busted (`FUN_00658140`, or `FUN_0065c880` for a wreck).
    Busted,
    /// `FUN_0065ca70`.
    Escaped,
    /// `FUN_006f3f30(n)`: the manager's cue (n = 0xc / 0xd at a chase start, 1 after 3 s of "being busted"; what it
    /// drives, music or speech, was not identified).
    Cue(u8),
}

/// A cop as the pursuit manager sees it (the AI player list world +0xa630, count +0xa660).
#[derive(Debug, Clone, Copy, Default)]
pub struct CopView {
    pub position: Vec3,
    /// `FUN_006bbb70`: the driver is in a chasing state (0, 0x10, 0x12, 0x16, 0x17, 0x18, 0x1a; `FUN_004f2b20`) or
    /// the coordinator has the cop in its chase role (`FUN_00516390`). `FUN_0065ba70` counts these.
    pub chasing: bool,
    /// Player +0x23f (`FUN_006ba290`): the cop's car has crashed.
    pub crashed: bool,
    /// `FUN_006bed90`: player +0x1f0 != 0 or +0x1f4 != INT_MIN (not identified; such a cop is not the bust cop).
    pub excluded: bool,
    /// The player's body is in the cop car's contact list (`FUN_00904880` / `FUN_00904850`, ids +0x60 / +0x62).
    pub touching: bool,
}

/// The game mode's switches the manager reads (free roam: [`Default`]).
#[derive(Debug, Clone, Copy)]
pub struct PursuitMode {
    /// Manager +0xa00 (the pursuit system is set up); with the level's `249ed427` it is `FUN_00656140`, "the level
    /// has a cooldown". Without it a chase starts at once and never cools.
    pub system: bool,
    /// GMM +0x2e74: cops and pursuits run (`FUN_00661810`, the coordinator, `FUN_0065c5e0`).
    pub cops_active: bool,
    /// GMM +0x2e96 (not identified): holds the immediate chase of a mode without a cooldown.
    pub hold_chase: bool,
    /// GMM +0x2e8d (`FUN_006e0400`, not identified): the first heat-up of a chase posts no message.
    pub quiet_first_heat_up: bool,
    /// `FUN_006f0f00(2)` (not identified): the bar stays at −0.9 or above (the player cannot be busted).
    pub bar_floor: bool,
    /// The camera has no override running (world +0x2f480 +0x1258c … +0x1256c): the bust camera may start.
    pub camera_free: bool,
    /// Rows of the heat list unlocked by the player's progression (`FUN_0068f190`; None: all).
    pub unlocked_levels: Option<usize>,
}

impl Default for PursuitMode {
    fn default() -> Self {
        Self {
            system: true,
            cops_active: true,
            hold_chase: false,
            quiet_first_heat_up: false,
            bar_floor: false,
            camera_free: true,
            unlocked_levels: None,
        }
    }
}

/// One tick's inputs.
#[derive(Debug, Clone, Copy)]
pub struct PursuitInput<'a> {
    pub dt: f32,
    pub mode: PursuitMode,
    pub position: Vec3,
    /// The player's speed (mph, signed; `FUN_006ba3e0`, vehicle +0x1620).
    pub speed_mph: f32,
    pub cops: &'a [CopView],
    /// The coordinator's freshest sighting while a cop knows where the player is (`FUN_00513720` + 0x460,
    /// (index + 0x4e) × 0x10).
    pub last_seen: Option<Vec3>,
    /// `IsHiddenFromCops` (`FUN_006bd9e0`: coordinator byte +0x418 + i clear).
    pub hidden: bool,
    /// The cop that knows the player (`FUN_00514aa0`: coordinator +0x478 + 4i while +0x400 + i), if it has a car
    /// (`FUN_0052f4f0`), and whether that cop is wrecked (its player +0x23e).
    pub spotter: Option<Spotter>,
    /// Coordinator byte +0x448 + i (writer not traced): the chase start posts the roadblock variant.
    pub spotter_flag: bool,
    /// The player made a cop crash (`FUN_006ccd20` → `FUN_006b8d00` sets `rec + 0x8a4`).
    pub took_down_cop: bool,
    /// The player's car touches a roadblock's car (`FUN_0065b930`).
    pub touches_roadblock: bool,
    /// A roadblock stands within [`NEAR_RANGE`] (`FUN_0083ef30`).
    pub roadblock_near: bool,
    /// The player's car is wrecked (player +0x23e) by an AI player (wrecker index ≥ 12; `FUN_0065c880`).
    pub wrecked_by_ai: bool,
    /// The player's vehicle +0x1a84 is 0 (cooldown × 4; 2 for the player's car in a live read).
    pub fast_cooldown: bool,
}

/// [`PursuitInput::spotter`].
#[derive(Debug, Clone, Copy)]
pub struct Spotter {
    pub wrecked: bool,
}

impl<'a> PursuitInput<'a> {
    /// Inputs with nothing happening (for tests and callers that fill in what they have).
    pub fn new(dt: f32, position: Vec3, speed_mph: f32, cops: &'a [CopView]) -> Self {
        Self {
            dt,
            mode: PursuitMode::default(),
            position,
            speed_mph,
            cops,
            last_seen: None,
            hidden: true,
            spotter: None,
            spotter_flag: false,
            took_down_cop: false,
            touches_roadblock: false,
            roadblock_near: false,
            wrecked_by_ai: false,
            fast_cooldown: false,
        }
    }
}

/// The player's pursuit record.
#[derive(Debug, Clone, Default)]
pub struct Pursuit {
    pub state: PursuitState,
    /// Player +0x3d8 (`Player.ChaseHeat`).
    pub heat: f32,
    /// `rec + 0x874` (`Player.StaticBustProgress`, 0–1).
    pub bust_meter: f32,
    /// `rec + 0x870`: the pursuit bar, −1 (busted) … 1 (far away).
    pub bar: f32,
    /// Index into [`BAR_RANGES`] (`rec + 0x8a0`).
    pub bar_range: usize,
    /// `rec + 0x87c`: seconds in the current bar range.
    pub bar_time: f32,
    /// `rec + 0x860`: where the cops last saw the player.
    pub last_seen: Vec3,
    /// `rec + 0x878`: seconds since the chase (re)started (`FUN_0065c550` clears it).
    pub since_start: f32,
    /// `rec + 0x880`: seconds in the cooldown.
    pub cooldown_time: f32,
    /// Player +0x3e8 (`Player.TimeTilEvaded`).
    pub time_til_evaded: f32,
    /// `rec + 0x0`: seconds chased or cooling (`Player.TimeInChase`).
    pub chase_time: f32,
    /// `rec + 0x898`: the highest level number reached (`Player.MaxHeatLevelReachedInChase`).
    pub max_level: u8,
    /// `rec + 0x858`: bit n − 1 = level n's own scoring action has been posted (`FUN_0065c5e0`).
    pub levels_announced: u32,
    /// `rec + 0x88c`: cops chasing (`Player.CopsInChase`).
    pub cops_chasing: u32,
    /// `rec + 0x8a5`: the player is farther from `last_seen` than the level's lose radius.
    pub out_of_range: bool,
    /// `rec + 0x8a6`: the cops see the player (`FUN_0065c740`'s edge flag).
    pub sighted: bool,
    /// `rec + 0x8a7` (`Player.CopsCloseEnoughToBust`).
    pub close_enough: bool,
    /// `rec + 0x8a8`: a heat-up has happened in this chase (`FUN_0065d080`).
    pub heated_up: bool,
    /// `rec + 0x8a9`: a "chase initiated" action has been posted (`FUN_006691d0`).
    pub initiated: bool,
    /// Player +0x432 (`Player.ShowStaticBustBar`).
    pub show_bust_bar: bool,
    /// `Player.CopRadarProgress` (`FUN_0065d470`): the nearest cop's distance over the radar's range.
    pub radar: f32,
}

impl Pursuit {
    pub fn level<'a>(&self, cfg: &'a PoliceConfig) -> Option<&'a HeatLevel> {
        heat::level_row(cfg, self.heat, self.state == PursuitState::Chase)
    }

    /// `Player.HeatLevelInt` (`FUN_0066a7b0`).
    pub fn level_number(&self, cfg: &PoliceConfig) -> u8 {
        heat::level_number(cfg, self.heat, self.state == PursuitState::Chase)
    }

    /// `FUN_006bdc50`.
    pub fn heat_progress(&self, cfg: &PoliceConfig) -> f32 {
        heat::band_progress_cfg(cfg, self.heat, self.state == PursuitState::Chase)
    }

    /// `Player.PursuitBarEnum` / `Player.PursuitBarProgress` (`FUN_00658080`, `FUN_006580a0`).
    pub fn bar_kind(&self) -> u8 {
        BAR_RANGES[self.bar_range].2
    }
    pub fn bar_progress(&self) -> f32 {
        let (low, high, _) = BAR_RANGES[self.bar_range];
        ((self.bar - low) / (high - low)).clamp(0.0, 1.0)
    }

    pub fn in_chase_or_cooldown(&self) -> bool {
        matches!(self.state, PursuitState::Chase | PursuitState::Cooldown)
    }

    /// `FUN_006bdac0`: add heat, capped below the highest unlocked level (`FUN_006673a0`); a level change from a
    /// level above 0 runs `FUN_0065d080`.
    pub fn add_heat(&mut self, cfg: &PoliceConfig, mode: &PursuitMode, amount: f32, ev: &mut Vec<PursuitEvent>) {
        if amount <= 0.0 {
            return;
        }
        let before = self.level_number(cfg);
        self.heat = (self.heat + amount).min(heat::heat_cap(&cfg.levels, mode.unlocked_levels));
        let after = self.level_number(cfg);
        if after != before && before > 0 {
            // `FUN_0065d080`: the highest level; the message unless the mode keeps the first one quiet, and only for
            // a step of one level.
            self.max_level = self.max_level.max(after);
            ev.push(PursuitEvent::LevelUp(after));
            if (!mode.quiet_first_heat_up || self.heated_up) && (after as i32 - before as i32) < 2 {
                ev.push(PursuitEvent::Score(ScoreSlot::HeatUp));
            }
            self.heated_up = true;
        }
    }

    /// `FUN_006bdb70`: remove heat; a level change runs `FUN_0065d160` (the message unless the new level is 0).
    pub fn remove_heat(&mut self, cfg: &PoliceConfig, amount: f32, ev: &mut Vec<PursuitEvent>) {
        if amount <= 0.0 {
            return;
        }
        let before = self.level_number(cfg);
        self.heat = (self.heat - amount).max(0.0);
        let after = self.level_number(cfg);
        if after != before {
            ev.push(PursuitEvent::LevelDown(after));
            if after != 0 {
                ev.push(PursuitEvent::Score(ScoreSlot::HeatDown));
            }
        }
    }

    /// Post a scoring action (`FUN_006a8050`) and add its heat (`FUN_006d08a0` → `FUN_006bdac0`; the config holds
    /// the heat of the chase actions; other actions' heat is the caller's, through [`Pursuit::add_heat`]).
    fn post(&mut self, cfg: &PoliceConfig, mode: &PursuitMode, slot: ScoreSlot, ev: &mut Vec<PursuitEvent>) {
        ev.push(PursuitEvent::Score(slot));
        if slot.is_chase_initiated() {
            self.initiated = true;
        }
        let heat = match slot {
            s if s.is_chase_initiated() => cfg.chase_started_heat,
            ScoreSlot::ChaseResumed => cfg.chase_resumed_heat,
            _ => 0.0,
        };
        self.add_heat(cfg, mode, heat, ev);
    }

    /// `FUN_0065c550`: the chase starts (or resumes) here.
    fn start_chase(&mut self, position: Vec3) {
        self.state = PursuitState::Chase;
        self.last_seen = position;
        self.since_start = 0.0;
        self.out_of_range = false;
    }

    /// `FUN_00658140` (the game logic's action 0xf): busted.
    pub fn set_busted(&mut self, ev: &mut Vec<PursuitEvent>) {
        if self.state != PursuitState::Busted {
            self.state = PursuitState::Busted;
            ev.push(PursuitEvent::Busted);
        }
    }

    /// `ResetChaseStats` (`FUN_0065b830`(pursuit, 1), then player +0x3d8 / +0x3dc / +0x3e8 = 0) and
    /// `ResetLocalPlayerHeat` (HSM 457517 "Do Pursuits": after BUSTED both, after ESCAPED the first only; the heat
    /// clear of `ResetChaseStats` itself is the research log's 2026-11 reading of the native).
    pub fn reset(&mut self) {
        *self = Self { radar: 1.0, ..Default::default() };
    }

    /// One update (`FUN_0066ae60`).
    pub fn tick(&mut self, cfg: &PoliceConfig, i: &PursuitInput) -> Vec<PursuitEvent> {
        let mut ev = Vec::new();
        let mode = i.mode;
        // `FUN_0065ba70`: cops chasing the player (cop type, physical car, not crashed, `FUN_006bbb70`).
        self.cops_chasing = i.cops.iter().filter(|c| c.chasing && !c.crashed).count() as u32;
        // `FUN_0065d470`: the cop radar.
        let nearest = i.cops.iter().map(|c| c.position.distance(i.position)).fold(cfg.radar_range, f32::min);
        self.radar = if cfg.radar_range > 0.0 { nearest / cfg.radar_range } else { 1.0 };
        self.since_start += i.dt;
        // `FUN_00664d60`: the bust meter and the pursuit bar (chase only).
        self.bust(cfg, i, &mut ev);
        let cooldown_mode = mode.system && self.level(cfg).is_some_and(|l| l.cooldown_allowed);
        match self.state {
            PursuitState::None => self.idle(cfg, i, cooldown_mode, &mut ev),
            PursuitState::Chase => {
                self.track(cfg, i);
                self.sight(cfg, i, &mut ev);
                // `FUN_00657e60`: the heat rises with time, by the level's threshold over its time.
                if let Some(level) = self.level(cfg).cloned()
                    && level.heat_time > 0.0
                {
                    self.add_heat(cfg, &mode, level.threshold / level.heat_time * i.dt, &mut ev);
                }
                self.announce(cfg, i, &mut ev);
                self.wrecked(cfg, i, &mut ev);
                // `FUN_00661810` → `FUN_0065cbe0`: out of the lose radius → the cooldown.
                if mode.cops_active && self.out_of_range && self.state == PursuitState::Chase && cooldown_mode {
                    self.state = PursuitState::Cooldown;
                    self.cooldown_time = 0.0;
                    self.cops_chasing = 0;
                    ev.push(PursuitEvent::Cooldown);
                    self.post(cfg, &mode, ScoreSlot::Cooldown, &mut ev);
                }
            }
            PursuitState::Cooldown => {
                // `FUN_0065c880` only acts in a chase.
                self.cooldown(cfg, i, &mut ev);
            }
            PursuitState::Busted | PursuitState::Escaped => {}
        }
        if self.in_chase_or_cooldown() {
            self.chase_time += i.dt;
        }
        ev
    }

    /// State 0: the chase starts when a cop chases the player or the player took one down, at a roadblock, or at
    /// once in a mode without a cooldown.
    fn idle(&mut self, cfg: &PoliceConfig, i: &PursuitInput, cooldown_mode: bool, ev: &mut Vec<PursuitEvent>) {
        let slot = if !cooldown_mode && !i.mode.hold_chase {
            ScoreSlot::ChaseUnspotted
        } else if i.touches_roadblock {
            ScoreSlot::ChaseRoadblock
        } else if self.cops_chasing > 0 || i.took_down_cop {
            match i.spotter {
                None => ScoreSlot::ChaseUnspotted,
                Some(_) if i.spotter_flag => {
                    ev.push(PursuitEvent::Cue(0xd));
                    ScoreSlot::ChaseRoadblock
                }
                Some(s) if !s.wrecked => {
                    ev.push(PursuitEvent::Cue(0xc));
                    ScoreSlot::ChaseSpotted
                }
                Some(_) => {
                    ev.push(PursuitEvent::Cue(0xd));
                    ScoreSlot::ChaseCopWrecked
                }
            }
        } else {
            return;
        };
        self.start_chase(i.position);
        self.chase_time = 0.0;
        ev.push(PursuitEvent::Started);
        self.post(cfg, &i.mode, slot, ev);
    }

    /// `FUN_0065bca0`: where the cops last saw the player, and whether the player has left the lose radius.
    fn track(&mut self, cfg: &PoliceConfig, i: &PursuitInput) {
        let level = self.level(cfg).cloned().unwrap_or_default();
        match i.last_seen {
            Some(p) => self.last_seen = p,
            None if i.roadblock_near => self.last_seen = i.position,
            None => {}
        }
        if level.cooldown_allowed {
            self.out_of_range = self.last_seen.distance(i.position) > level.lose_radius;
        } else {
            self.last_seen = i.position;
            self.out_of_range = false;
        }
    }

    /// `FUN_0065c740`: sight lost (+0x90) while cops still chase, sight regained (+0x94).
    fn sight(&mut self, cfg: &PoliceConfig, i: &PursuitInput, ev: &mut Vec<PursuitEvent>) {
        if !self.sighted {
            if !i.hidden {
                self.post(cfg, &i.mode, ScoreSlot::SightRegained, ev);
                self.sighted = true;
            }
        } else if i.hidden && !i.wrecked_by_ai && self.cops_chasing > 0 {
            self.post(cfg, &i.mode, ScoreSlot::SightLost, ev);
            self.sighted = false;
        }
    }

    /// `FUN_0065c5e0`: a "chase initiated" action if none was posted, and a level's own action the first time the
    /// chase reaches it.
    fn announce(&mut self, cfg: &PoliceConfig, i: &PursuitInput, ev: &mut Vec<PursuitEvent>) {
        if !self.initiated && i.mode.cops_active {
            self.post(cfg, &i.mode, ScoreSlot::ChaseUnspotted, ev);
        }
        if self.max_level > 0 {
            let bit = 1u32 << ((self.max_level - 1) & 31);
            if self.levels_announced & bit == 0 {
                if let Some(level) = self.level(cfg)
                    && level.level_up_action != 0
                {
                    ev.push(PursuitEvent::Score(ScoreSlot::Level(level.level_up_action)));
                }
                self.levels_announced |= bit;
            }
        }
    }

    /// `FUN_0065c880`: wrecked by an AI car in a chase → busted.
    fn wrecked(&mut self, cfg: &PoliceConfig, i: &PursuitInput, ev: &mut Vec<PursuitEvent>) {
        if self.state == PursuitState::Chase && i.wrecked_by_ai {
            self.post(cfg, &i.mode, ScoreSlot::WreckedByAi, ev);
            self.set_busted(ev);
        }
    }

    /// The cooldown (`FUN_006615c0`).
    fn cooldown(&mut self, cfg: &PoliceConfig, i: &PursuitInput, ev: &mut Vec<PursuitEvent>) {
        // No cop within 20 m (every cop-type player with a physical car).
        let clear = !i.cops.iter().any(|c| c.position.distance(i.position) < COOLDOWN_CLEAR_RANGE);
        self.cooldown_time += i.dt;
        if clear {
            let level = self.level(cfg).cloned().unwrap_or_default();
            // The band of the level (integer thresholds), at least 1, over the level's cooldown time.
            let band = (level.threshold - heat::level_below_cfg(cfg, self.heat).map_or(0.0, |l| l.threshold)).max(1.0);
            let mut rate = band / level.cooldown_time;
            if i.fast_cooldown {
                rate *= COOLDOWN_FAST;
            }
            self.time_til_evaded = if self.heat == 0.0 {
                0.0
            } else if level.level < 2 {
                self.heat / rate
            } else {
                level.cooldown_time
            };
            self.remove_heat(cfg, rate * i.dt, ev);
        }
        // `FUN_0065c970`: a chasing cop or a roadblock near → chased again.
        if self.cops_chasing > 0 || i.roadblock_near {
            self.start_chase(i.position);
            ev.push(PursuitEvent::Resumed);
            self.post(cfg, &i.mode, ScoreSlot::ChaseResumed, ev);
        }
        // `FUN_0065ca70`: escaped once the time left is out (player +0x3e8 ≤ 0).
        if self.state != PursuitState::Chase && self.time_til_evaded <= 0.0 {
            self.state = PursuitState::Escaped;
            ev.push(PursuitEvent::Escaped);
            self.post(cfg, &i.mode, ScoreSlot::Escaped, ev);
        }
    }

    /// The bust meter (`FUN_0065cda0`) and the pursuit bar (`FUN_00664d60`).
    fn bust(&mut self, cfg: &PoliceConfig, i: &PursuitInput, ev: &mut Vec<PursuitEvent>) {
        if self.state != PursuitState::Chase {
            self.bar = 0.0;
            return;
        }
        // The nearest chasing cop with a car that `FUN_006bed90` does not exclude; and the nearest distance, at
        // most 100 m, for the bar.
        let mut best: Option<(usize, f32)> = None;
        let mut near = NEAR_RANGE;
        for (k, c) in i.cops.iter().enumerate() {
            if !c.chasing || c.excluded {
                continue;
            }
            let d = c.position.distance(i.position);
            if best.is_none_or(|b| d < b.1) {
                best = Some((k, d));
            }
            near = near.min(d);
        }
        let speed = i.speed_mph.abs();
        // The meter: no cop → drains; a crashed cop → holds; else fills while the cop touches a player slower than
        // 25 mph, or is within 20 m (and 4 m of height) of one slower than 12.5 mph; drains otherwise.
        let step = match best {
            None => -i.dt,
            Some((k, _)) if i.cops[k].crashed => 0.0,
            Some((k, d)) => {
                let c = &i.cops[k];
                let touching = c.touching && speed < cfg.bust_speed_mph;
                self.close_enough = (c.position.y - i.position.y).abs() < BUST_HEIGHT && d < cfg.bust_distance;
                let near_slow = self.close_enough && speed < cfg.bust_speed_mph * BUST_NEAR_SPEED_FACTOR;
                if touching || near_slow { i.dt } else { -i.dt }
            }
        };
        self.bust_meter = (self.bust_meter + step / cfg.bust_time.max(1e-3)).clamp(0.0, 1.0);
        // `FUN_006bdf50`: the BUSTING bar.
        if self.bust_meter > BUST_BAR_SHOWN_ABOVE {
            self.show_bust_bar = true;
        } else if self.bust_meter <= 0.0 {
            self.show_bust_bar = false;
        }

        // The bar.
        let b = cfg.bust_distance;
        let far = ((near - b) / (NEAR_RANGE - b)).clamp(0.0, 1.0);
        let close = 1.0 - ((near - b) / (BAR_CLOSE_RANGE - b)).clamp(0.0, 1.0);
        let fast = ((speed - cfg.bust_speed_mph) / (BAR_TOP_MPH - cfg.bust_speed_mph)).clamp(0.0, 1.0);
        let free =
            (far * BAR_FAR - BAR_LIMIT + BAR_CLOSE - (1.0 - fast) * close * BAR_CLOSE).clamp(-BAR_LIMIT, BAR_LIMIT);
        self.bar = if self.bust_meter < BAR_METER { free } else { (self.bust_meter * -BAR_LIMIT - BAR_LIMIT).min(0.0) };
        if !(i.mode.system && self.level(cfg).is_some_and(|l| l.cooldown_allowed)) {
            self.bar = self.bar.min(BAR_NO_COOLDOWN);
        }
        if i.mode.bar_floor {
            self.bar = self.bar.max(BAR_FLOOR);
        }
        // The range: kept while the bar is inside it (its timer runs; 3 s in "being busted" posts cue 1), else the
        // first range that holds the bar.
        let (low, high, kind) = BAR_RANGES[self.bar_range];
        if self.bar >= high || self.bar <= low {
            if let Some(r) = BAR_RANGES.iter().position(|&(l, h, _)| self.bar > l && self.bar < h) {
                self.bar_range = r;
            }
            self.bar_time = 0.0;
        } else {
            let before = self.bar_time;
            self.bar_time += i.dt;
            if before < BAR_CUE_TIME && self.bar_time >= BAR_CUE_TIME && kind == 0 {
                ev.push(PursuitEvent::Cue(1));
            }
        }
        if self.bar > -1.0 || !i.mode.camera_free {
            return;
        }
        if let Some((cop, _)) = best {
            ev.push(PursuitEvent::BustCaught { cop });
        }
        self.bar_range = 1;
        self.bar = 0.0;
    }
}
