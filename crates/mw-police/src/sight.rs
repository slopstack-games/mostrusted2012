//! What a cop notices, and when the player becomes wanted: the cop's perception (`FUN_006c5810`, the ray callback
//! `FUN_006c6280`; the perception block is the AI player + 0x250) and the coordinator's per-player pass
//! (`FUN_005192b0`, with the traffic-hit window `FUN_00515090`). docs/decomp.md "When a pursuit starts
//! (2026-10-16)".
//!
//! A pursuit never starts because a cop is merely near: the coordinator sends a cop after the player only when the
//! player is *wanted* and the cop *knows* where the player is (`FUN_005240a0`), and the pursuit manager starts the
//! chase when a cop chases (`FUN_0066ae60`). Wanted is one of:
//! - a cop has the player in its cone while the player is faster than the heat level's speeding limit
//!   (`78f472fa`, 110 mph at level 0);
//! - the player hit two traffic cars within 10 s while a cop had the player in its cone (`FUN_00515090`);
//! - a cop was rammed: its contacts with the player's car added up past `AIConfig` +0x60 (2);
//! - a pursuit (chase or cooldown) is running.
//!
//! The game's terms (6) `FUN_006a0340` && `FUN_006a1570` and (7) coordinator byte +0x665 were not identified; free
//! roam is taken to leave them clear (MODEL). Term (4) (`FUN_00656140` false) never holds in free roam.
//!
//! Out of range or cone a cop still "sees" a human target within 100 m of a roadblock (`FUN_00657ee0`: target
//! index < 12, `FUN_0083ef30` over the roadblock manager world +0x2f484, whose records are the roadblocks: found
//! 2026-11-17).

use glam::Vec3;
use mw_assets::police::HeatLevel;

/// `.rdata` 0x00fa8538: a target at or below this speed (mph) uses the level's slow no-cone range (+0x90).
const SLOW_TARGET_MPH: f32 = 5.0;
/// `.rdata` 0x00fc4f70: a target within this distance (m) of where the cop last placed it is still seen.
const STILL_THERE: f32 = 20.0;
/// `.rdata` 0x00fc0600: a vehicle touching the cop adds to its ram sum at most once per this many seconds.
const CONTACT_REPEAT: f32 = 0.5;
/// `.rdata` 0x00fa99d0: the traffic-hit window (s, `FUN_00515090`).
const TRAFFIC_HIT_WINDOW: f32 = 10.0;

/// The cop's perception of the player (one target: the game keeps one slot per player, 24).
#[derive(Debug, Clone, Default)]
pub struct Perception {
    /// +0x105 + i: the cop sees the player (sticky while the player stays in range and cone).
    pub sees: bool,
    /// +0x11d + i: the player is inside the cop's cone angle on a clear line (the speeding and traffic-hit
    /// terms read this).
    pub in_cone: bool,
    /// +0x60 + 4i: seconds the cop still tracks the player's true position.
    pub memory: f32,
    /// +0x170 + 0x10i: where the cop last placed the player; +0x0 + 4i: seconds since.
    pub last_seen: Option<Vec3>,
    pub seen_age: f32,
    /// +0x104: the cop was rammed (its contacts added up past the threshold); cleared when the player leaves the
    /// cop's range or cone.
    pub rammed: bool,
    /// +0x100: the sum of contacts; +0xc0 + 4v: the per-vehicle repeat timer (one vehicle: the player's car).
    contact_sum: f32,
    contact_timer: f32,
}

/// One tick's view of the cop and the player.
#[derive(Debug, Clone, Copy)]
pub struct SightInput {
    pub dt: f32,
    pub cop_position: Vec3,
    pub cop_forward: Vec3,
    /// The cop's driver is in a chasing state (0, 0x10, 0x12, 0x16, 0x17, 0x18, 0x1a: `FUN_004f2b20`) or the
    /// coordinator has it in a chase role (`FUN_00516390`): the chasing range (+0xa4).
    pub chasing: bool,
    /// The coordinator marks the cop alerted (`FUN_00516400`): the alert range (+0xa0) and cone (+0x98).
    pub alert: bool,
    pub target: Vec3,
    pub target_mph: f32,
    /// The cop's crashed car sees nothing (the game drops crashed cops from the coordinator's lists).
    pub crashed: bool,
    /// A new contact between the cop's car and the player's this tick, with its strength (MODEL: the closing
    /// speed in m/s; the game adds the contact record's +0x1c, units not traced).
    pub contact: Option<f32>,
    /// `AIConfig` +0x60 (`a7a0cd14`, 2): the ram threshold.
    pub ram_threshold: f32,
    /// A roadblock stands within 100 m of the (human) target (`FUN_00657ee0`).
    pub roadblock_near: bool,
}

impl Perception {
    /// One tick of `FUN_006c5810` for the player. `line_clear` is the ray from the cop to the player (asked only
    /// when the player is in range and in the cone; the game queues it and reads it back in `FUN_006c6280`).
    pub fn tick(&mut self, level: &HeatLevel, i: &SightInput, line_clear: impl FnOnce() -> bool) {
        // The contacts: each vehicle once per 0.5 s while not yet rammed; past the threshold: rammed.
        self.contact_timer = (self.contact_timer - i.dt).max(0.0);
        if let Some(strength) = i.contact
            && self.contact_timer == 0.0
            && !self.rammed
        {
            self.contact_timer = CONTACT_REPEAT;
            self.contact_sum += strength;
        }
        if self.contact_sum > i.ram_threshold {
            self.rammed = true;
            self.contact_sum = 0.0;
        }
        // The memory: while it lasts the cop tracks the player's true position.
        self.seen_age += i.dt;
        if self.memory > 0.0 {
            self.last_seen = Some(i.target);
            self.seen_age = 0.0;
            self.memory = (self.memory - i.dt).max(0.0);
        }
        if i.crashed {
            self.sees = false;
            self.in_cone = false;
            return;
        }
        let range = if i.chasing {
            level.sight_chasing
        } else if i.alert {
            level.sight_alert
        } else {
            level.sight_idle
        };
        let to = i.target - i.cop_position;
        let d2 = to.length_squared();
        let cone = if i.alert { level.sight_cone_alert_deg } else { level.sight_cone_deg }.to_radians().cos();
        let dot = i.cop_forward.dot(to.normalize_or_zero());
        let near = if i.target_mph <= SLOW_TARGET_MPH { level.sight_near_slow } else { level.sight_near };
        if d2 >= range * range || (d2 > near * near && dot < cone) {
            // Out of range or cone: seen only near a roadblock (`FUN_00657ee0`).
            self.in_cone = false;
            self.sees = i.roadblock_near;
            self.rammed = false;
            return;
        }
        // The ray callback: a clear line makes the cop see; while it sees, the cone flag, the position and the
        // memory are refreshed.
        if line_clear() {
            self.sees = true;
        }
        if self.sees {
            self.in_cone = dot > cone;
            self.last_seen = Some(i.target);
            self.seen_age = 0.0;
            self.memory = level.memory;
        }
        // Still where the cop last placed it: seen.
        if self.last_seen.is_some_and(|p| p.distance_squared(i.target) < STILL_THERE * STILL_THERE) {
            self.sees = true;
        }
    }

    /// The cop knows where the player is (`FUN_005192b0`: seen or remembered).
    pub fn knows(&self) -> bool {
        self.sees || self.memory > 0.0
    }
}

/// The coordinator's wanted test for the player (`FUN_005192b0`), with the traffic-hit window.
#[derive(Debug, Clone, Default)]
pub struct Wanted {
    /// Coordinator +0x3e0 + 4i: the window's timer; +0x3e4 + 4i: traffic hits counted in it.
    window: f32,
    hits: u32,
    /// Why the player is wanted (the last evaluation; for the log and the test).
    pub reason: Option<WantedReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WantedReason {
    Pursuit,
    Speeding,
    TrafficHits,
    Rammed,
    /// Set by the caller (the runtime's `--test pursuit` / `MW_PURSUIT_DEMO`).
    Forced,
}

/// One cop's part in the wanted test.
#[derive(Debug, Clone, Copy)]
pub struct CopNotice {
    pub in_cone: bool,
    pub rammed: bool,
}

impl Wanted {
    /// One tick: `traffic_hits` = traffic cars the player's car hit this tick (lane followers and parked cars
    /// turned physical by its contact, the list at world +0x33144).
    pub fn tick(
        &mut self,
        dt: f32,
        level: &HeatLevel,
        cops: &[CopNotice],
        player_mph: f32,
        traffic_hits: u32,
        pursued: bool,
    ) -> bool {
        let in_cone = cops.iter().any(|c| c.in_cone);
        let speeding = in_cone && player_mph.abs() > level.speeding_mph;
        // `FUN_00515090`: the window runs down; when it has run out the count is cleared. Hits count only while a
        // cop has the player in its cone; the first one opens a 10 s window.
        if self.window >= 0.0 {
            self.window -= dt;
            if self.window < 0.0 {
                self.hits = 0;
            }
        }
        if in_cone {
            for _ in 0..traffic_hits {
                if self.hits == 0 {
                    self.window = TRAFFIC_HIT_WINDOW;
                }
                self.hits += 1;
            }
        }
        let rammed = cops.iter().any(|c| c.rammed);
        self.reason = if pursued {
            Some(WantedReason::Pursuit)
        } else if speeding {
            Some(WantedReason::Speeding)
        } else if self.hits > 1 {
            Some(WantedReason::TrafficHits)
        } else if rammed {
            Some(WantedReason::Rammed)
        } else {
            None
        };
        self.reason.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level() -> HeatLevel {
        HeatLevel {
            sight_chasing: 100.0,
            sight_alert: 90.0,
            sight_idle: 50.0,
            sight_near: 50.0,
            sight_near_slow: 50.0,
            sight_cone_deg: 40.0,
            sight_cone_alert_deg: 40.0,
            memory: 2.0,
            speeding_mph: 110.0,
            ..Default::default()
        }
    }

    fn input(target: Vec3, mph: f32) -> SightInput {
        SightInput {
            dt: 1.0 / 60.0,
            cop_position: Vec3::ZERO,
            cop_forward: Vec3::Z,
            chasing: false,
            alert: false,
            target,
            target_mph: mph,
            crashed: false,
            contact: None,
            ram_threshold: 2.0,
            roadblock_near: false,
        }
    }

    #[test]
    fn sight_needs_range_and_line() {
        let l = level();
        let mut p = Perception::default();
        p.tick(&l, &input(Vec3::new(0.0, 0.0, 70.0), 30.0), || true);
        assert!(!p.sees, "beyond the idle range");
        p.tick(&l, &input(Vec3::new(0.0, 0.0, 40.0), 30.0), || false);
        assert!(!p.sees, "blocked line");
        p.tick(&l, &input(Vec3::new(0.0, 0.0, 40.0), 30.0), || true);
        assert!(p.sees && p.in_cone && p.memory > 0.0);
        // Behind the cop, within the no-cone range: still seen, but not in the cone.
        p.tick(&l, &input(Vec3::new(0.0, 0.0, -30.0), 30.0), || true);
        assert!(p.sees && !p.in_cone);
    }

    #[test]
    fn passing_slowly_is_not_wanted() {
        let l = level();
        let mut w = Wanted::default();
        let cops = [CopNotice { in_cone: true, rammed: false }];
        assert!(!w.tick(1.0 / 60.0, &l, &cops, 40.0, 0, false));
        assert!(w.tick(1.0 / 60.0, &l, &cops, 120.0, 0, false), "speeding in the cone");
        let away = [CopNotice { in_cone: false, rammed: false }];
        assert!(!w.tick(1.0 / 60.0, &l, &away, 120.0, 0, false), "speeding out of sight");
    }

    #[test]
    fn two_traffic_hits_in_view_within_ten_seconds() {
        let l = level();
        let seen = [CopNotice { in_cone: true, rammed: false }];
        let mut w = Wanted::default();
        assert!(!w.tick(0.1, &l, &seen, 30.0, 1, false));
        for _ in 0..50 {
            assert!(!w.tick(0.1, &l, &seen, 30.0, 0, false));
        }
        assert!(w.tick(0.1, &l, &seen, 30.0, 1, false), "second hit 5 s later");
        let mut w = Wanted::default();
        assert!(!w.tick(0.1, &l, &seen, 30.0, 1, false));
        for _ in 0..110 {
            w.tick(0.1, &l, &seen, 30.0, 0, false);
        }
        assert!(!w.tick(0.1, &l, &seen, 30.0, 1, false), "second hit after the window");
    }

    #[test]
    fn ramming_adds_up() {
        let l = level();
        let mut p = Perception::default();
        let mut i = input(Vec3::new(0.0, 0.0, 5.0), 10.0);
        i.contact = Some(1.5);
        p.tick(&l, &i, || true);
        assert!(!p.rammed, "one soft touch");
        // The same contact again within 0.5 s does not count.
        p.tick(&l, &i, || true);
        assert!(!p.rammed);
        i.dt = 0.6;
        p.tick(&l, &i, || true);
        assert!(p.rammed, "a second touch later");
    }
}
