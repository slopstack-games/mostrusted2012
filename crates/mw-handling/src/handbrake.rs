//! The handbrake object (vehicle +0x2a20; `FUN_009d27b0`, run as step 4 of the driving update `FUN_00904260`;
//! init `FUN_009cda00`, reset `FUN_009cda30`). docs/decomp.md "Handbrake and burnout".
//!
//! It holds a countdown per lane (+0x10), reset to `Minimum_handbrake_time` on every tick the control block's
//! handbrake mask (vehicle +0x1c40, "pulled above 0.1", or forced by the gearbox's standstill state) is set and
//! reduced by dt otherwise; the handbrake is **on** (+0x20, the vehicle's virtual +0x44) while any lane is ≥ 0,
//! and +0x21 keeps the previous tick's value. Its readers: the rear brakes' handbrake torque and the driveline's
//! axle split (both run earlier in the tick, so they see the previous tick's state), the tyre solver's lock flag
//! on the rear wheels (`FUN_00905910`, after the update: this tick's state) and the drift.
//!
//! A small state machine on the sign of the speed (vehicle +0x1620) and the yaw rate (the chassis's angular
//! velocity +0xc0, world Y) writes a steering-input range (+0x50, the vehicle's virtual +0x4c) that the steering
//! (`FUN_009aae10` → `FUN_009d73f0` → `FUN_009d31f0`) clamps the raw input to: [−1, 1], except for a second
//! after a handbrake pull that began moving forwards ends with the car stopped or rolling backwards, when only
//! the side opposite the yaw sign recorded while it was pulled is allowed.

/// The one-sided steering range lasts this long after the handbrake is let go (`.bss` 0x011870f0, read 1.0 in the
/// memory snapshot `re/out/live/snap1`).
pub const ONE_SIDED_TIME: f32 = 1.0;
/// Released and still turning: the state machine goes back to idle once the yaw rate in the recorded direction
/// drops below this (rad/s; `.bss` 0x01187070, 0.1 in the snapshot).
pub const YAW_RATE_IDLE: f32 = 0.1;

/// The handbrake object's state (vehicle +0x2a20).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handbrake {
    /// +0x04: 0 idle; 1 pulled moving forwards; 2 released, still turning; 3 stopped or rolling backwards after
    /// a pull (one-sided steering); 4 pulled while not moving forwards; 5 pulled while not moving forwards and
    /// then moving forwards.
    pub state: u8,
    /// +0x10: the countdown (one lane: the player's mask sets all four alike).
    pub countdown: f32,
    /// +0x20 / +0x21: on this tick / on the tick before.
    pub on: bool,
    pub was_on: bool,
    /// +0x30: the one-sided range's timer.
    pub timer: f32,
    /// +0x40: −sign(yaw rate) recorded while pulled moving forwards (−1, 0 or 1).
    pub yaw_sign: f32,
    /// +0x50 lanes 0 / 1: the steering input's range in the game's sense (positive = right).
    pub steer_range: [f32; 2],
}

impl Default for Handbrake {
    /// `FUN_009cda00`.
    fn default() -> Self {
        Self { state: 0, countdown: 0.0, on: false, was_on: false, timer: 0.0, yaw_sign: 0.0, steer_range: [-1.0, 1.0] }
    }
}

impl Handbrake {
    /// `FUN_009cda30` (the vehicle reset `FUN_009a9b10`): state 0 and the countdown 0; the flags, the timer and
    /// the range are kept.
    pub fn reset(&mut self) {
        self.state = 0;
        self.countdown = 0.0;
    }

    /// One tick of `FUN_009d27b0`. `pulled`: the control block's mask; `min_time`: `Minimum_handbrake_time`;
    /// `mph`: the signed speed (vehicle +0x1620); `yaw_rate`: the chassis's angular velocity about world Y.
    pub fn update(&mut self, pulled: bool, min_time: f32, mph: f32, yaw_rate: f32, dt: f32) {
        self.was_on = self.on;
        self.countdown = if pulled { min_time } else { self.countdown - dt };
        self.on = self.countdown >= 0.0;
        let forwards = mph > 0.0;
        let one_sided = |s: &mut Self| {
            s.state = 3;
            s.timer = ONE_SIDED_TIME;
        };
        match self.state {
            0 => {
                self.steer_range = [-1.0, 1.0];
                if self.on {
                    self.state = if forwards { 1 } else { 4 };
                }
            }
            1 => {
                // −sign of the yaw rate, recorded every tick while pulled.
                self.yaw_sign = if -yaw_rate < 0.0 {
                    -1.0
                } else if -yaw_rate > 0.0 {
                    1.0
                } else {
                    0.0
                };
                if !self.on {
                    self.state = 2;
                } else if !forwards {
                    one_sided(self);
                }
            }
            2 => {
                if !forwards {
                    one_sided(self);
                }
                // Checked after the above: the turn has died down (or reversed).
                if -yaw_rate * self.yaw_sign < YAW_RATE_IDLE {
                    self.state = 0;
                }
            }
            3 => {
                // Only the side opposite the recorded yaw sign: [−1, 0] when it is positive, else [0, 1].
                self.steer_range = if self.yaw_sign > 0.0 { [-1.0, 0.0] } else { [0.0, 1.0] };
                if self.on {
                    self.timer = ONE_SIDED_TIME;
                }
                self.timer -= dt;
                if self.timer < 0.0 || forwards {
                    self.state = 0;
                }
            }
            4 => {
                if !self.on {
                    self.state = 0;
                } else if forwards {
                    self.state = 5;
                }
            }
            5 => {
                if !self.on {
                    self.state = 0;
                }
            }
            _ => self.state = 0,
        }
    }

    /// The steering input (game sense, positive = right) clamped to the range (`FUN_009d31f0`).
    pub fn clamp_steer(&self, input: f32) -> f32 {
        input.max(self.steer_range[0]).min(self.steer_range[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn countdown_keeps_it_on_for_the_minimum_time() {
        let mut h = Handbrake::default();
        h.update(true, 0.1, 50.0, 0.0, DT);
        assert!(h.on && !h.was_on);
        let mut ticks = 0;
        while h.on {
            h.update(false, 0.1, 50.0, 0.0, DT);
            ticks += 1;
        }
        // 0.1 s at 60 Hz: on while the countdown is ≥ 0 (the sixth subtraction lands on 0 give or take rounding).
        assert!((6..=7).contains(&ticks), "{ticks}");
    }

    #[test]
    fn pull_forwards_then_stop_limits_steering_to_one_side_for_a_second() {
        let mut h = Handbrake::default();
        h.update(true, 0.1, 40.0, 1.5, DT);
        assert_eq!(h.state, 1);
        h.update(true, 0.1, 40.0, 1.5, DT);
        assert_eq!(h.yaw_sign, -1.0);
        // Stopped while still pulled.
        h.update(true, 0.1, 0.0, 1.5, DT);
        assert_eq!(h.state, 3);
        h.update(false, 0.1, -1.0, 1.0, DT);
        assert_eq!(h.steer_range, [0.0, 1.0]);
        assert_eq!(h.clamp_steer(-0.7), 0.0);
        for _ in 0..70 {
            h.update(false, 0.1, -1.0, 1.0, DT);
        }
        assert_eq!(h.state, 0);
        h.update(false, 0.1, -1.0, 1.0, DT);
        assert_eq!(h.steer_range, [-1.0, 1.0]);
    }

    #[test]
    fn released_turn_dies_down_to_idle() {
        let mut h = Handbrake::default();
        h.update(true, 0.0, 40.0, -1.0, DT);
        h.update(true, 0.0, 40.0, -1.0, DT);
        h.update(false, 0.0, 40.0, -1.0, DT);
        assert_eq!(h.state, 2);
        h.update(false, 0.0, 40.0, -0.5, DT);
        assert_eq!(h.state, 2);
        h.update(false, 0.0, 40.0, -0.05, DT);
        assert_eq!(h.state, 0);
    }
}
