//! The race's rubber band: a target-speed schedule per opponent (manager at world+0x2f478, `DAT_01177868`; frame
//! function `FUN_00645c90`), pushed to the drivers' +0x5668 (`FUN_0083c820` → `FUN_00514590`). Not a grip or
//! torque multiplier.

use crate::{MPH, NO_SPEED};

/// One opponent's balancing: `RaceBalancingData.OpponentData[k]` (0x30-byte entries, k = player slot − 12),
/// the event's `RaceBalancingProfile` and pace multiplier, and the data's time terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RubberBand {
    /// +0x10 `AheadDistance` / +0x14 `BehindDistance` (m): the blend t runs between them (both 100 without data,
    /// `.rdata` 0x00fb69e0).
    pub ahead: f32,
    pub behind: f32,
    /// +0x24 / +0x18: race progress breakpoints (early, late).
    pub early_x: f32,
    pub late_x: f32,
    /// Speed factors [t = 0 (behind), t = 1 (ahead)]: early +0x2c / +0x28, late +0x20 / +0x1c.
    pub early: [f32; 2],
    pub late: [f32; 2],
    /// `RaceBalancingProfile` +0x10 `MaxSpeed` (mph) and +0x14 (passed to the route's speed factor).
    pub max_speed_mph: f32,
    pub profile_14: f32,
    /// Manager +0xa0: the event's pace multiplier for the player's car (`AllowedVehicleList` `a17a5e72`).
    pub pace: f32,
    /// `RaceBalancingData` +0x10 (s per player crash) and +0x14 (s): added to the expected time.
    pub crash_time: f32,
    pub time_offset: f32,
}

/// One opponent's race situation for a schedule update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RubberBandSlot {
    /// The player's remaining distance minus the AI's (race tracker world+0x2f47c, +0xdf0): > 0 = the AI leads.
    pub lead: f32,
    /// The straight distance between the two cars (m).
    pub gap: f32,
    /// Race progress 0..1 (`FUN_00654110`: 1 − remaining / total).
    pub progress: f32,
    /// Race time (s; the game's tick counter × its period).
    pub race_time: f32,
    /// The expected time at this progress (`FUN_00642da0`, see [`RubberBand::expected_time`]); `None` skips the
    /// time correction.
    pub expected_time: Option<f32>,
    /// The slot has a car (`FUN_0083bdf0`): the correction's K is 0.1, else 0.2 (`.rdata` 0x00f9f660 / 0x00fb69ec).
    pub has_car: bool,
}

impl RubberBand {
    /// `FUN_0063ef30`: t = 0 when lead < behind, 1 when lead ≥ ahead, else the clamped ramp of the gap signed by
    /// the lead between behind and ahead (`FUN_005114e0`).
    pub fn blend(&self, lead: f32, gap: f32) -> f32 {
        if lead < self.behind {
            return 0.0;
        }
        if self.ahead <= lead {
            return 1.0;
        }
        let s = if lead < 0.0 { -gap } else { gap };
        ((s - self.behind) / (self.ahead - self.behind)).clamp(0.0, 1.0)
    }

    fn factors(&self, t: f32) -> (f32, f32) {
        let a = (self.early[1] - self.early[0]) * t + self.early[0];
        let b = (self.late[1] - self.late[0]) * t + self.late[0];
        (a, b)
    }

    /// The schedule's top speed (m/s): `MaxSpeed` × pace × 0.44704 (`.data` 0x010c51b4).
    pub fn top_speed(&self) -> f32 {
        self.max_speed_mph * self.pace * MPH
    }

    /// `FUN_006432a0` (a racer with a car): the early factor before `early_x`, the late one from `late_x`, linear
    /// between, × the top speed.
    pub fn speed(&self, t: f32, x: f32) -> f32 {
        let (a, b) = self.factors(t);
        let f = if x < self.early_x {
            a
        } else if x < self.late_x {
            (x - self.early_x) / (self.late_x - self.early_x) * (b - a) + a
        } else {
            b
        };
        self.top_speed() * f
    }

    /// `FUN_00642da0`: the time the schedule expects at progress `x` on a route of `total` metres, with
    /// `route_factor(d0, d1)` = `FUN_00661eb0` (the route's speed factor between two distances; not ported, the
    /// runtime supplies it): Σ = the early part's F(0, xe L) · A · xe · L + the middle's (xb − xe) · lerp(A, B,
    /// (x − xe) / (xl − xe)) · F(xe L, xb L) · L + the late part's (x − xl) · F(xl, L x) · B · L (the late part's
    /// first distance is not scaled by L in the code), and time = (L x)² / Σ + crashes × `crash_time` +
    /// `time_offset`. 0 when x = 0.
    pub fn expected_time(
        &self,
        t: f32,
        x: f32,
        total: f32,
        crashes: u32,
        route_factor: impl Fn(f32, f32) -> f32,
    ) -> f32 {
        if x == 0.0 {
            return 0.0;
        }
        let (a, b) = self.factors(t);
        let (xe, xl, l) = (self.early_x, self.late_x, total);
        let mut sum = 0.0;
        if xe > 0.0 {
            let e = xe.min(x);
            sum = route_factor(0.0, e * l) * a * e * l;
        }
        if xe < x {
            let m = xl.min(x);
            let f = route_factor(xe * l, m * l);
            sum += (m - xe) * ((x - xe) / (xl - xe) * (b - a) + a) * f * l;
        }
        if xl < x {
            sum += (x - xl) * route_factor(xl, l * x) * b * l;
        }
        (l * x) / (sum / (l * x)) + crashes as f32 * self.crash_time + self.time_offset
    }

    /// `FUN_0063ef30` + `FUN_006432a0` + `FUN_006433f0`: the schedule speed (m/s) and the "on time" flag the
    /// manager passes to `FUN_0083c840` (false when more than 30 s ahead of the expected time, `.rdata`
    /// 0x00fb69e8). The correction: K = 0.1 with a car (0.2 without), k = K × speed,
    /// speed + clamp((race time − expected) × k / 30, ±k) (`.rdata` 0x00fa1db4).
    pub fn schedule(&self, s: &RubberBandSlot) -> (f32, bool) {
        let t = self.blend(s.lead, s.gap);
        let base = self.speed(t, s.progress);
        let Some(expected) = s.expected_time else { return (base, true) };
        let k = base * if s.has_car { 0.1 } else { 0.2 };
        let late = s.race_time - expected;
        let corr = (late * k * (1.0 / 30.0)).max(-k).min(k);
        (base + corr, late >= -30.0)
    }
}

/// The manager's frame (`FUN_00645c90`, state 3): one opponent slot (12..23) is re-evaluated per frame, round
/// robin; every slot's last speed is pushed to its driver every frame; the local car's crash flag (vehicle
/// +0x1774) counts crashes on its rising edge (manager +0xac, read by the expected time).
#[derive(Debug, Clone, PartialEq)]
pub struct RubberBandManager {
    /// Manager +0x00..+0x2c: the schedule speed per opponent k = slot − 12 (m/s). Initial value not traced
    /// (model decision: none).
    pub speeds: [f32; 12],
    /// Manager +0x9c: the next slot − 12.
    pub cursor: usize,
    /// Manager +0xac: player crashes.
    pub crashes: u32,
    crashed: bool,
}

impl Default for RubberBandManager {
    fn default() -> Self {
        Self { speeds: [NO_SPEED; 12], cursor: 0, crashes: 0, crashed: false }
    }
}

impl RubberBandManager {
    /// One frame: `eval(k)` returns the schedule (and on-time flag) for opponent k when that slot races (`None`
    /// skips it); returns the slot updated, the flag of an update, and leaves [`Self::speeds`] for the drivers.
    pub fn frame(
        &mut self,
        player_crashed: bool,
        mut eval: impl FnMut(usize) -> Option<(f32, bool)>,
    ) -> Option<(usize, bool)> {
        let k = self.cursor;
        let updated = eval(k).map(|(v, on_time)| {
            self.speeds[k] = v;
            (k, on_time)
        });
        self.cursor = (self.cursor + 1) % 12;
        if player_crashed && !self.crashed {
            self.crashes += 1;
        }
        self.crashed = player_crashed;
        updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band() -> RubberBand {
        RubberBand {
            ahead: 100.0,
            behind: -100.0,
            early_x: 0.1,
            late_x: 0.9,
            early: [1.0, 0.9],
            late: [0.8, 0.6],
            max_speed_mph: 200.0,
            profile_14: 0.0,
            pace: 1.0,
            crash_time: 3.0,
            time_offset: 0.0,
        }
    }

    #[test]
    fn behind_runs_faster_than_ahead() {
        let b = band();
        let slot = |lead: f32, x: f32| RubberBandSlot {
            lead,
            gap: lead.abs(),
            progress: x,
            race_time: 0.0,
            expected_time: None,
            has_car: true,
        };
        // Far behind early: the early "behind" factor; far ahead late: the late "ahead" factor.
        assert!((b.schedule(&slot(-500.0, 0.0)).0 - 200.0 * MPH).abs() < 1e-3);
        assert!((b.schedule(&slot(500.0, 1.0)).0 - 0.6 * 200.0 * MPH).abs() < 1e-3);
        // Level halfway: t = 0.5, between the breakpoints.
        let v = b.schedule(&slot(0.0, 0.5)).0;
        let (a, l) = (0.95, 0.7);
        assert!((v - 200.0 * MPH * (a + (l - a) * 0.5)).abs() < 1e-3, "{v}");
        assert!(b.schedule(&slot(-50.0, 0.5)).0 > b.schedule(&slot(50.0, 0.5)).0);
    }

    #[test]
    fn time_correction_speeds_up_a_late_racer() {
        let b = band();
        let base = RubberBandSlot {
            lead: 0.0,
            gap: 0.0,
            progress: 0.5,
            race_time: 60.0,
            expected_time: Some(45.0),
            has_car: true,
        };
        let v0 = b.speed(b.blend(0.0, 0.0), 0.5);
        let (v, on_time) = b.schedule(&base);
        assert!((v - v0 * (1.0 + 0.1 * 0.5)).abs() < 1e-3);
        assert!(on_time);
        // 40 s early: clamped to −K, flagged.
        let (v, on_time) = b.schedule(&RubberBandSlot { expected_time: Some(100.0), ..base });
        assert!((v - v0 * 0.9).abs() < 1e-3);
        assert!(!on_time);
    }

    #[test]
    fn expected_time_of_a_constant_schedule() {
        // One factor everywhere and F = top speed: time = distance / speed.
        let b = RubberBand { early: [1.0, 1.0], late: [1.0, 1.0], early_x: 0.0, late_x: 1.0, ..band() };
        let v = 50.0;
        let t = b.expected_time(0.0, 0.5, 1000.0, 1, |_, _| v);
        assert!((t - (500.0 / v + 3.0)).abs() < 1e-3, "{t}");
    }

    #[test]
    fn manager_round_robin_and_crash_count() {
        let mut m = RubberBandManager::default();
        for f in 0..13 {
            let up = m.frame(f == 3 || f == 4, |k| Some((k as f32, true)));
            assert_eq!(up.map(|u| u.0), Some(f % 12));
        }
        assert_eq!(m.speeds[11], 11.0);
        assert_eq!(m.crashes, 1);
    }
}
