//! Signal processing for the game's mixer plug-ins (`Genesys.Gen.*DspPlugIn`).
//!
//! The filters follow the game's designs (RBJ "Audio EQ
//! Cookbook" biquads, Butterworth cascades); the Distortion and SubHarmoniser are ported from the disassembly
//! (research log 2026-11-18). Parameter names and ranges from the data
//! (research log 2026-10-04, "sound sequences"):
//!
//! | plug-in | parameters (data) | here |
//! |---------|-------------------|------|
//! | Peaking | Frequency (Hz), Gain (linear, 1 = flat: the neutral `20000 Hz, 1` instances), Q | RBJ peaking EQ |
//! | HighShelf | Frequency, Gain (linear) | RBJ high shelf, S = 1 |
//! | Low/HighPassButterworth | Frequency, Order (2 or 4) | cascaded biquads; 0 Hz / ≥ Nyquist = bypass |
//! | BandPass | Bandwidth (Hz), Frequency | as `FUN_00ce65b0`: RBJ band-pass (0 dB peak), Q = f / bandwidth ≤ 20, skipped when the band does not fit |
//! | Distortion | Distortion (0.4 – 4), Model (2) | ported (`0x00ab0e60`): model 2 hard-clips `x · (d + 0.99999)` to ±1 |
//! | SubHarmoniser | Interval (1) | ported (`0x00aad950`): `abs(x)` on the half cycles where a counter mod Interval is 0 |
//! | Delay | Time (s), Feedback | feedback delay line; Time 0 = bypass |

use std::f32::consts::{PI, TAU};

/// Direct-form-1 biquad section.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Default for Biquad {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Biquad {
    pub const IDENTITY: Self = Self { b: [1.0, 0.0, 0.0], a: [0.0, 0.0], x: [0.0; 2], y: [0.0; 2] };

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let inv = 1.0 / a0;
        self.b = [b0 * inv, b1 * inv, b2 * inv];
        self.a = [a1 * inv, a2 * inv];
        if !self.b.iter().chain(&self.a).all(|v| v.is_finite()) {
            self.b = [1.0, 0.0, 0.0];
            self.a = [0.0, 0.0];
        }
    }

    pub fn bypass(&mut self) {
        self.b = [1.0, 0.0, 0.0];
        self.a = [0.0, 0.0];
    }

    /// `w0` and `alpha` for a frequency and Q, or `None` when the frequency is outside (0, Nyquist).
    fn omega(sr: f32, freq: f32, q: f32) -> Option<(f32, f32, f32)> {
        if !(freq > 1.0 && freq < sr * 0.49) {
            return None;
        }
        let w0 = TAU * freq / sr;
        let (s, c) = w0.sin_cos();
        Some((s, c, s / (2.0 * q.max(0.05))))
    }

    pub fn peaking(&mut self, sr: f32, freq: f32, gain: f32, q: f32) {
        let Some((_, c, alpha)) = Self::omega(sr, freq, q) else { return self.bypass() };
        let a = gain.max(1e-4).sqrt();
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    pub fn high_shelf(&mut self, sr: f32, freq: f32, gain: f32) {
        let Some((s, c, _)) = Self::omega(sr, freq, 1.0) else { return self.bypass() };
        let a = gain.max(1e-4).sqrt();
        let alpha = s / 2.0 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        );
    }

    pub fn low_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let Some((_, c, alpha)) = Self::omega(sr, freq, q) else { return self.bypass() };
        self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn high_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let Some((_, c, alpha)) = Self::omega(sr, freq, q) else { return self.bypass() };
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    /// `BandPassIir2` as the game runs it (`FUN_00ce65b0`, design `FUN_00cdfef0`): see
    /// [`crate::dsp::band_pass_game`]. A filter the game skips passes its input and restarts from zero state.
    pub fn band_pass_game(&mut self, sr: f32, freq: f32, bandwidth: f32) {
        match crate::dsp::band_pass_game(sr, freq, bandwidth) {
            Some([b0, b1, b2, a0, a1, a2]) => self.set(b0, b1, b2, a0, a1, a2),
            None => *self = Self::IDENTITY,
        }
    }

    pub fn band_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let Some((_, c, alpha)) = Self::omega(sr, freq, q) else { return self.bypass() };
        self.set(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        // Flush denormals and recover from blow-ups (coefficients change while running).
        let y = if y.is_finite() && y.abs() > 1e-20 { y } else { 0.0 };
        self.y = [y, self.y[0]];
        y
    }
}

/// Q values of the biquad sections of an even-order Butterworth filter.
pub fn butterworth_q(order: u32) -> Vec<f32> {
    let n = order.clamp(2, 8) & !1;
    (0..n / 2).map(|k| 1.0 / (2.0 * (PI * (2 * k + 1) as f32 / (2 * n) as f32).cos())).collect()
}

pub use crate::dsp::{SubHarmoniser, distort, distortion_model, sub_interval};

/// Feedback delay line (`DelayDspPlugIn`).
#[derive(Debug, Clone, Default)]
pub struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

impl Delay {
    pub fn process(&mut self, x: f32, time: f32, feedback: f32, sr: f32) -> f32 {
        let n = (time * sr) as usize;
        if n == 0 || n > sr as usize * 2 {
            return x;
        }
        if self.buf.len() != n {
            self.buf = vec![0.0; n];
            self.pos = 0;
        }
        let d = self.buf[self.pos];
        self.buf[self.pos] = x + d * feedback.clamp(0.0, 0.95);
        self.pos = (self.pos + 1) % n;
        x + d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(bq: &mut Biquad, sr: f32, f: f32) -> f32 {
        let n = (sr * 0.5) as usize;
        let mut peak: f32 = 0.0;
        for i in 0..n {
            let y = bq.process((TAU * f * i as f32 / sr).sin());
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn peaking_boosts_its_band_only() {
        let sr = 48_000.0;
        let mut bq = Biquad::IDENTITY;
        bq.peaking(sr, 1000.0, 4.0, 1.0);
        assert!((gain_at(&mut bq.clone(), sr, 1000.0) - 4.0).abs() < 0.1);
        assert!((gain_at(&mut bq.clone(), sr, 50.0) - 1.0).abs() < 0.05);
        assert!((gain_at(&mut bq, sr, 15000.0) - 1.0).abs() < 0.1);
    }

    #[test]
    fn butterworth_sections() {
        let q = butterworth_q(4);
        assert!((q[0] - 0.5412).abs() < 1e-3 && (q[1] - 1.3066).abs() < 1e-3);
        let sr = 48_000.0;
        let mut bq = Biquad::IDENTITY;
        bq.low_pass(sr, 1000.0, butterworth_q(2)[0]);
        assert!((gain_at(&mut bq.clone(), sr, 1000.0) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02);
        assert!(gain_at(&mut bq, sr, 8000.0) < 0.05);
    }
}
