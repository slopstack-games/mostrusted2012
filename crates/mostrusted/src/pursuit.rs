//! The pursuit rules, engine-free: heat, the chase / cooldown / busted / escaped state machine and the bust
//! meter, ported from the game's pursuit manager (docs/decomp.md "Police and pursuits"; per-player record of
//! 0x8b0 bytes at `*(world + 0x2f410)`, updated by `FUN_0066ae60`). `police.rs` gathers the inputs every fixed
//! tick (the player's car, the cops, what they see) and acts on the result.
//!
//! Every constant names its source: a `GameLogicConfig` / `HeatLevel` field (`mw_assets::police`) or an `.rdata`
//! value read from the exe on disk.

use bevy::prelude::*;
use mw_assets::police::{HeatLevel, PoliceConfig};

/// `rec + 0x89c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PursuitState {
    /// 0: no pursuit.
    #[default]
    None,
    /// 1: chased (`Player.IsInChase`).
    Chase,
    /// 2: busted.
    Busted,
    /// 3: the cooldown (`Player.IsInCooldown`): the cops lost the player and the heat cools.
    Cooldown,
    /// 4: escaped.
    Escaped,
}

/// What changed in a tick, for the log, the HUD's messages and the test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PursuitEvent {
    /// The chase began ("CHASE INITIATED").
    Started,
    /// The cops lost the player: the cooldown began.
    Cooldown,
    /// A cop found the player in the cooldown: chased again.
    Resumed,
    /// "HEAT LEVEL INCREASED" / "HEAT LEVEL DECREASED" (the level number shown).
    LevelUp(u8),
    LevelDown(u8),
    Busted,
    Escaped,
}

/// A cop as the pursuit manager sees it.
#[derive(Debug, Clone, Copy)]
pub struct CopView {
    pub position: Vec3,
    /// The cop is in a pursuit state or has the player as a seen target (`FUN_006bbb70`).
    pub chasing: bool,
    /// The cop's car has crashed (`player + 0x23f`).
    pub crashed: bool,
    /// The cop's car touches the player's (the player's body is in the cop car's contact list).
    pub touching: bool,
}

/// One tick's inputs.
#[derive(Debug, Clone, Copy)]
pub struct PursuitInput<'a> {
    pub dt: f32,
    pub position: Vec3,
    /// The player's speed (mph; the game reads the vehicle's speed in mph, vehicle + 0x1620).
    pub speed_mph: f32,
    pub cops: &'a [CopView],
    /// Where the cops last saw the player, while they have such a point (the AI coordinator's per-player flag
    /// and position, `FUN_00513720` + 0x460 / (index + 0x4e) × 0x10).
    pub last_seen: Option<Vec3>,
    /// The player made a cop crash since the last tick (`FUN_006ccd20` → `FUN_006b8d00` sets `rec + 0x8a4`), or
    /// touches a roadblock's car (`FUN_0065b930`).
    pub cop_taken_down: bool,
    /// The player's car was wrecked by an AI car (`FUN_0065c880`: `Player.Wrecked` with a wrecker index ≥ 12).
    pub wrecked_by_cop: bool,
    /// A roadblock stands within [`ROADBLOCK_RANGE`] of the player (`FUN_0083ef30`).
    pub roadblock_near: bool,
}

/// `.rdata` 0x00fb77e4: range (m) of the nearest-cop term of the pursuit bar, and of the roadblock check.
pub const ROADBLOCK_RANGE: f32 = 100.0;
/// `.rdata` 0x00fb74c8: the heat does not cool while a cop is within this distance (m; `FUN_006615c0`).
const COOLDOWN_CLEAR_RANGE: f32 = 20.0;
/// `.rdata` 0x00fb82d0: a cop more than this far above or below the player cannot bust (m; `FUN_0065cda0`).
const BUST_HEIGHT: f32 = 4.0;
/// `.rdata` 0x00f8a810: the bust speed's factor for a cop that is near but not touching (`FUN_0065cda0`).
const BUST_NEAR_SPEED_FACTOR: f32 = 0.5;
/// `.rdata` 0x00f8bb34: the heat stops this far below the last level's threshold (`FUN_006673a0`).
const HEAT_CAP_MARGIN: f32 = 0.001;
/// `.rdata` 0x00fc0608: the BUSTING bar shows once the bust meter passes this (`FUN_006bdf50`), until it is
/// back at zero.
const BUST_BAR_SHOWN_ABOVE: f32 = 0.3;

/// The pursuit bar's three ranges (`FUN_00661500`, manager + 0x8b0 / 0x8bc / 0x8c8: lower bound, upper bound,
/// kind): 0 = being busted, 1 = cops close, 2 = getting away (`Player.PursuitBarEnum`).
const BAR_RANGES: [(f32, f32, u8); 3] = [(-1.0, -0.5, 0), (-0.65, 0.0, 1), (0.0, 1.0, 2)];

/// The player's pursuit record.
#[derive(Debug, Clone, Default)]
pub struct Pursuit {
    pub state: PursuitState,
    /// `player + 0x3d8` (`Player.ChaseHeat`).
    pub heat: f32,
    /// `rec + 0x874` (`Player.StaticBustProgress`, 0–1).
    pub bust_meter: f32,
    /// `rec + 0x870`: the pursuit bar, −1 (busted) … 1 (far away).
    pub bar: f32,
    /// Index into [`BAR_RANGES`] (`rec + 0x8a0`).
    pub bar_range: usize,
    /// `rec + 0x860`: where the cops last saw the player.
    pub last_seen: Vec3,
    /// `rec + 0x880`: seconds in the cooldown.
    pub cooldown_time: f32,
    /// `player + 0x3e8` (`Player.TimeTilEvaded`).
    pub time_til_evaded: f32,
    /// `rec + 0x0`: seconds chased or cooling (`Player.TimeInChase`).
    pub chase_time: f32,
    /// `rec + 0x898`: the highest level number reached (`Player.MaxHeatLevelReachedInChase`).
    pub max_level: u8,
    /// `rec + 0x88c`: cops chasing (`Player.CopsInChase`).
    pub cops_chasing: u32,
    /// `rec + 0x8a5`: the player is farther from `last_seen` than the level's lose radius.
    pub out_of_range: bool,
    /// `rec + 0x8a7` (`Player.CopsCloseEnoughToBust`).
    pub close_enough: bool,
    /// `player + 0x432` (`Player.ShowStaticBustBar`).
    pub show_bust_bar: bool,
    /// `Player.CopRadarProgress`: the nearest cop's distance over the radar's range (1 = none within range).
    pub radar: f32,
    /// Chases started, busts and escapes so far (ours, for the test and the log).
    pub chases: u32,
    pub busts: u32,
    pub escapes: u32,
}

impl Pursuit {
    /// The heat level the game works with (`FUN_00669640`): level 0 with no heat outside a chase, else the
    /// first level (from the second on) whose threshold lies above the heat.
    pub fn level<'a>(&self, cfg: &'a PoliceConfig) -> Option<&'a HeatLevel> {
        level_for(cfg, self.heat, self.state == PursuitState::Chase)
    }

    /// `Player.HeatLevelInt` (`FUN_0066a7b0`): the level's number.
    pub fn level_number(&self, cfg: &PoliceConfig) -> u8 {
        self.level(cfg).map_or(0, |l| l.level)
    }

    /// Progress through the current level's heat band, 0–1 (`FUN_006bdc50`, the fraction `Player.HeatLevelFloat`
    /// adds to the level number).
    pub fn heat_progress(&self, cfg: &PoliceConfig) -> f32 {
        let Some(level) = self.level(cfg) else { return 0.0 };
        let low = level_below(cfg, self.heat).map_or(0.0, |l| l.threshold);
        ((self.heat - low) / (level.threshold - low).max(1e-3)).clamp(0.0, 1.0)
    }

    /// `Player.PursuitBarEnum` and `Player.PursuitBarProgress` (`FUN_00658080`, `FUN_006580a0`).
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

    /// Add heat (`FUN_006bdac0`): capped just below the last level's threshold (`FUN_006673a0`; every level
    /// counts as unlocked here), with the level-up message when the level number rises.
    pub fn add_heat(&mut self, cfg: &PoliceConfig, amount: f32, events: &mut Vec<PursuitEvent>) {
        if amount <= 0.0 {
            return;
        }
        let before = self.level_number(cfg);
        let cap = cfg.levels.last().map_or(0.0, |l| l.threshold - HEAT_CAP_MARGIN).max(0.0);
        self.heat = (self.heat + amount).min(cap);
        let after = self.level_number(cfg);
        // `FUN_0065d080`: the highest level reached and the message.
        if after != before && before > 0 {
            self.max_level = self.max_level.max(after);
            events.push(PursuitEvent::LevelUp(after));
        }
    }

    /// Remove heat (`FUN_006bdb70`), with the level-down message (`FUN_0065d160`: only for a level whose
    /// number is not 0).
    fn remove_heat(&mut self, cfg: &PoliceConfig, amount: f32, events: &mut Vec<PursuitEvent>) {
        if amount <= 0.0 {
            return;
        }
        let before = self.level_number(cfg);
        self.heat = (self.heat - amount).max(0.0);
        let after = self.level_number(cfg);
        if after != before && after != 0 {
            events.push(PursuitEvent::LevelDown(after));
        }
    }

    /// `FUN_0065c550`: the chase starts (or resumes) here.
    fn start_chase(&mut self, position: Vec3) {
        self.state = PursuitState::Chase;
        self.last_seen = position;
        self.out_of_range = false;
    }

    /// The outcome has been shown: back to no pursuit. The game's flow (HSM "Do Pursuits", 457517): after
    /// BUSTED the busted sequence, `ResetLocalPlayerHeat`, `ResetProps`, `RemoveAllCops`, `RemoveRoadblocks`,
    /// `ResetChaseStats`; after ESCAPED the escaped sequence and `ResetChaseStats`.
    pub fn reset(&mut self, clear_heat: bool) {
        let (chases, busts, escapes) = (self.chases, self.busts, self.escapes);
        let heat = if clear_heat { 0.0 } else { self.heat };
        *self = Self { heat, chases, busts, escapes, radar: 1.0, ..Default::default() };
    }

    /// One update (`FUN_0066ae60`).
    pub fn tick(&mut self, cfg: &PoliceConfig, i: &PursuitInput) -> Vec<PursuitEvent> {
        let mut events = Vec::new();
        let dt = i.dt;
        // `FUN_0065ba70`: the cops chasing the player (not crashed).
        self.cops_chasing = i.cops.iter().filter(|c| c.chasing && !c.crashed).count() as u32;
        // `FUN_0065d470`: the cop radar.
        let nearest = i.cops.iter().map(|c| c.position.distance(i.position)).fold(cfg.radar_range, f32::min);
        self.radar = if cfg.radar_range > 0.0 { nearest / cfg.radar_range } else { 1.0 };
        // `FUN_00664d60`: the bust meter and the pursuit bar (chase only).
        if self.bust(cfg, i) {
            events.push(PursuitEvent::Busted);
        }
        match self.state {
            PursuitState::None => {
                // A cop chases the player, or the player took one down: "CHASE INITIATED" (one of the scoring
                // actions `ScoringConfig` +0x48 / +0x4c / +0x50 / +0x54, by who saw the player and how; each adds
                // the same heat in the data, `FUN_006d08a0` → `FUN_006bdac0`).
                if self.cops_chasing > 0 || i.cop_taken_down {
                    self.start_chase(i.position);
                    self.chases += 1;
                    self.chase_time = 0.0;
                    events.push(PursuitEvent::Started);
                    self.add_heat(cfg, cfg.chase_started_heat, &mut events);
                }
            }
            PursuitState::Chase => {
                let level = self.level(cfg).cloned().unwrap_or_default();
                // `FUN_0065bca0`: where the cops last saw the player, and whether the player has left the lose
                // radius around that point.
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
                // `FUN_00657e60`: the heat rises with time, by the level's threshold over its time.
                if level.heat_time > 0.0 {
                    self.add_heat(cfg, level.threshold / level.heat_time * dt, &mut events);
                }
                // `FUN_0065c880`: wrecked by a cop → busted.
                if i.wrecked_by_cop && self.state == PursuitState::Chase {
                    self.state = PursuitState::Busted;
                    events.push(PursuitEvent::Busted);
                }
                // `FUN_00661810` → `FUN_0065cbe0`: out of the lose radius → the cooldown.
                if self.state == PursuitState::Chase && self.out_of_range {
                    self.state = PursuitState::Cooldown;
                    self.cooldown_time = 0.0;
                    self.cops_chasing = 0;
                    events.push(PursuitEvent::Cooldown);
                }
            }
            PursuitState::Cooldown => {
                if i.wrecked_by_cop {
                    self.state = PursuitState::Busted;
                    events.push(PursuitEvent::Busted);
                } else {
                    self.cooldown(cfg, i, &mut events);
                }
            }
            PursuitState::Busted | PursuitState::Escaped => {}
        }
        if self.in_chase_or_cooldown() {
            self.chase_time += dt;
        }
        if matches!(events.last(), Some(PursuitEvent::Busted)) {
            self.busts += 1;
        }
        events
    }

    /// The cooldown (`FUN_006615c0`): with no cop within 20 m the heat cools through the current level's band
    /// in the level's cooldown time; a chasing cop or a roadblock near turns it back into a chase
    /// (`FUN_0065c970`), and with no heat left the player has escaped (`FUN_0065ca70`).
    fn cooldown(&mut self, cfg: &PoliceConfig, i: &PursuitInput, events: &mut Vec<PursuitEvent>) {
        self.cooldown_time += i.dt;
        let clear = !i.cops.iter().any(|c| c.position.distance(i.position) < COOLDOWN_CLEAR_RANGE);
        if clear {
            let level = self.level(cfg).cloned().unwrap_or_default();
            let low = level_below(cfg, self.heat).map_or(0.0, |l| l.threshold);
            // The game multiplies the rate by 4 (`.rdata` 0x00fb74b0) when the player's vehicle field +0x1a84
            // is 0; what that field is was not settled (decomp.md), so the factor is left out.
            let rate = (level.threshold - low).max(1.0) / level.cooldown_time.max(1e-3);
            self.time_til_evaded = if self.heat == 0.0 {
                0.0
            } else if level.level < 2 {
                self.heat / rate
            } else {
                level.cooldown_time
            };
            self.remove_heat(cfg, rate * i.dt, events);
        }
        if self.cops_chasing > 0 || i.roadblock_near {
            self.start_chase(i.position);
            events.push(PursuitEvent::Resumed);
            // `ScoringConfig` +0x58: a scoring action without a title that adds heat.
            self.add_heat(cfg, cfg.chase_resumed_heat, events);
        } else if self.time_til_evaded <= 0.0 {
            self.state = PursuitState::Escaped;
            self.escapes += 1;
            events.push(PursuitEvent::Escaped);
        }
    }

    /// The bust meter (`FUN_0065cda0`) and the pursuit bar (`FUN_00664d60`); true when the bar reaches −1: busted.
    fn bust(&mut self, cfg: &PoliceConfig, i: &PursuitInput) -> bool {
        if self.state != PursuitState::Chase {
            self.bar = 0.0;
            return false;
        }
        // The nearest chasing cop.
        let nearest = i
            .cops
            .iter()
            .filter(|c| c.chasing)
            .map(|c| (c.position.distance(i.position), c))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let speed = i.speed_mph.abs();
        let mut filling = false;
        self.close_enough = false;
        if let Some((distance, cop)) = nearest
            && !cop.crashed
        {
            let touching = cop.touching && speed < cfg.bust_speed_mph;
            self.close_enough = (cop.position.y - i.position.y).abs() < BUST_HEIGHT && distance < cfg.bust_distance;
            let near = self.close_enough && speed < cfg.bust_speed_mph * BUST_NEAR_SPEED_FACTOR;
            filling = touching || near;
        }
        let step = i.dt / cfg.bust_time.max(1e-3);
        self.bust_meter = (self.bust_meter + if filling { step } else { -step }).clamp(0.0, 1.0);
        // `FUN_006bdf50`: the BUSTING bar.
        if self.bust_meter > BUST_BAR_SHOWN_ABOVE {
            self.show_bust_bar = true;
        } else if self.bust_meter <= f32::EPSILON {
            self.show_bust_bar = false;
        }

        // The bar: from the nearest chasing cop's distance d (at most 100 m) and the player's speed.
        // 0.8 (`.rdata` 0x00fb74cc), 0.2 (0x00fb867c), 50 m (0x00fb821c), 300 mph (0x00fb74bc).
        let d = nearest.map_or(ROADBLOCK_RANGE, |n| n.0.min(ROADBLOCK_RANGE));
        let b = cfg.bust_distance;
        let far = ((d - b) / (ROADBLOCK_RANGE - b)).clamp(0.0, 1.0);
        let close = 1.0 - ((d - b) / (50.0 - b)).clamp(0.0, 1.0);
        let fast = ((speed - cfg.bust_speed_mph) / (300.0 - cfg.bust_speed_mph)).clamp(0.0, 1.0);
        let free = (far * 0.8 - 0.5 + 0.2 - (1.0 - fast) * close * 0.2).clamp(-0.5, 0.5);
        // 0.01 (`.rdata` 0x00f9d474): with a bust meter, the bar runs from −0.5 to −1 with it.
        self.bar = if self.bust_meter < 0.01 { free } else { (self.bust_meter * -0.5 - 0.5).min(0.0) };
        // A level without a cooldown keeps the bar at −0.2 or below (`.rdata` 0x00fa9318).
        if !self.level(cfg).is_some_and(|l| l.cooldown_allowed) {
            self.bar = self.bar.min(-0.2);
        }
        if let Some(r) = BAR_RANGES.iter().position(|&(low, high, _)| self.bar > low && self.bar < high) {
            self.bar_range = r;
        }
        if self.bar <= -1.0 {
            // The game posts camera event 0x1a (the player's car, the cop's, 100) and the game logic's state
            // machine sets state 2 (`FUN_00854990` action 0xf → `FUN_00658140`).
            self.state = PursuitState::Busted;
            self.bar = 0.0;
            self.bar_range = 1;
            return true;
        }
        false
    }
}

/// `FUN_00669640`: the level for a heat value.
pub fn level_for(cfg: &PoliceConfig, heat: f32, in_chase: bool) -> Option<&HeatLevel> {
    if heat == 0.0 && !in_chase {
        return cfg.levels.first();
    }
    cfg.levels.iter().skip(1).find(|l| heat < l.threshold).or(cfg.levels.last())
}

/// `FUN_00669860`: the level below the heat's level (its threshold is the lower bound of the heat's band).
pub fn level_below(cfg: &PoliceConfig, heat: f32) -> Option<&HeatLevel> {
    let n = cfg.levels.len();
    let mut previous = cfg.levels.first();
    for l in cfg.levels.iter().take(n.saturating_sub(1)) {
        if heat < l.threshold {
            return previous;
        }
        previous = Some(l);
    }
    previous
}
