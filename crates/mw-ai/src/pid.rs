//! The game's PID controller (`FUN_00436920`, initialised by `FUN_004368d0`).

/// State [0] previous error, [1] integral; gains [2] Kp, [3] Ki, [4] Kd; clamps [5] error, [6] integral.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Pid {
    pub previous: f32,
    pub integral: f32,
    pub kp: f32,
    pub ki: f32,
    pub kd: f32,
    pub error_clamp: f32,
    pub integral_clamp: f32,
}

impl Pid {
    /// `FUN_004368d0(Kp, Ki, Kd, error clamp, integral clamp)`: also zeroes the previous error and the integral.
    pub fn new(kp: f32, ki: f32, kd: f32, error_clamp: f32, integral_clamp: f32) -> Self {
        Self { previous: 0.0, integral: 0.0, kp, ki, kd, error_clamp, integral_clamp }
    }

    /// `FUN_00436920(e, dt)`: e clamped to ±[5]; the integral (clamped to ±[6]) is updated after the output, which
    /// uses the integral from before this tick; the derivative is (e − previous) / dt with previous = 0 after
    /// `new`. Returns −(Ki·I_old + Kp·e + Kd·Δe/dt).
    pub fn step(&mut self, e: f32, dt: f32) -> f32 {
        let e = e.clamp(-self.error_clamp, self.error_clamp);
        let i_old = self.integral;
        let prev = self.previous;
        self.previous = e;
        self.integral = (e * dt + i_old).clamp(-self.integral_clamp, self.integral_clamp);
        -(self.ki * i_old + self.kp * e + (e - prev) / dt * self.kd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_uses_the_old_integral_and_a_zero_first_previous() {
        let mut p = Pid::new(1.0, 1.0, 0.0, 20.0, 20.0);
        assert_eq!(p.step(2.0, 0.5), -2.0);
        assert_eq!(p.step(2.0, 0.5), -3.0);
        let mut d = Pid::new(0.0, 0.0, 1.0, 20.0, 20.0);
        // previous = 0 after new: the first tick has a derivative kick.
        assert_eq!(d.step(1.0, 0.5), -2.0);
        // Error clamped to ±20.
        let mut c = Pid::new(1.0, 0.0, 0.0, 20.0, 20.0);
        assert_eq!(c.step(-100.0, 1.0), 20.0);
    }
}
