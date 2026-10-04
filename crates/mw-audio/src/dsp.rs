//! Signal processing for the game's mixer plug-ins (`Genesys.Gen.*DspPlugIn`).
//!
//! The data only names the plug-ins and their parameters; the maths here is standard (RBJ "Audio EQ
//! Cookbook" biquads, Butterworth cascades, a tanh waveshaper, a flip-flop octave divider). Which formula
//! the game uses for each is not known, so they are stand-ins chosen to match the parameter names and
//! ranges seen in the data (research log 2026-10-04, "sound sequences"):
//!
//! | plug-in | parameters (data) | here |
//! |---------|-------------------|------|
//! | Peaking | Frequency (Hz), Gain (linear, 1 = flat: the neutral `20000 Hz, 1` instances), Q | RBJ peaking EQ |
//! | HighShelf | Frequency, Gain (linear) | RBJ high shelf, S = 1 |
//! | Low/HighPassButterworth | Frequency, Order (2 or 4) | cascaded biquads; 0 Hz / ≥ Nyquist = bypass |
//! | BandPass | Bandwidth (Hz), Frequency | RBJ band-pass (0 dB peak), Q = f / bandwidth |
//! | Distortion | Distortion (0.4 – 4), Model (2) | `tanh(d · x)` |
//! | SubHarmoniser | Interval (1) | octave divider: a square wave toggled every 2^Interval zero crossings, scaled by the input envelope |
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

/// Octave divider for `SubHarmoniserDspPlugIn`: follows the input's zero crossings (with hysteresis
/// relative to its envelope) and toggles a square wave every 2^interval crossings, so its fundamental is
/// `interval` octaves below the input's. The square is scaled by the input envelope.
#[derive(Debug, Clone, Default)]
pub struct SubHarmoniser {
    env: f32,
    positive: bool,
    crossings: u32,
    state: f32,
    smooth: f32,
    /// Samples since the last zero crossing, and the length of the last half cycle.
    since_flip: f32,
    half_cycle: f32,
}

/// Inputs whose fundamental is above this (half cycles shorter than `sr / 2f`) are not tracked: an engine
/// sub-harmoniser follows firing orders, not hiss. The 911's per-car data high-passes one of its
/// sub-harmoniser buses at 6.8 kHz, which reads as switching that bus off.
const SUB_TRACK_MAX_HZ: f32 = 1500.0;

impl SubHarmoniser {
    pub fn process(&mut self, x: f32, interval: u32, sr: f32) -> f32 {
        // ~10 ms envelope follower.
        let k = 1.0 / (0.01 * sr);
        self.env += (x.abs() - self.env) * k;
        let thr = self.env * 0.25;
        let flip = if self.positive { x < -thr } else { x > thr };
        self.since_flip += 1.0;
        if flip {
            self.half_cycle = self.since_flip;
            self.since_flip = 0.0;
            self.positive = !self.positive;
            self.crossings += 1;
            if self.crossings >= 1 << interval.clamp(1, 3) {
                self.crossings = 0;
                self.state = if self.state > 0.0 { -1.0 } else { 1.0 };
            }
        }
        // Round the square's edges (~2 ms) so the toggles don't click and the upper harmonics stay below
        // the input's. Level: the input's RMS (mean |x| × π/2 is a sine's peak; × 1/√2 its RMS). The
        // game's sub-harmoniser code is not in the decompiled exe, so this stays a stand-in.
        self.smooth += (self.state - self.smooth) * (1.0 / (0.002 * sr)).min(1.0);
        if self.half_cycle < sr / (2.0 * SUB_TRACK_MAX_HZ) {
            return 0.0;
        }
        self.smooth * self.env * (std::f32::consts::FRAC_PI_2 * std::f32::consts::FRAC_1_SQRT_2)
    }
}

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

/// `DistortionDspPlugIn`. The game's waveshaper is not in the decompiled code (research log 2026-10-04,
/// "engine sound from the game code"); its parameter is a drive of 1e-5..1000 (default 1). This stand-in is
/// a drive-compensated soft clip, `tanh(d · x) / d`: unity gain for small signals, so the strain buses
/// (drive 4, fed at about -16 dB) add rasp without becoming as loud as the clean path, and transparent at
/// the data's gentle drive of 0.4. [`distort_raw`] is the uncompensated `tanh(d · x)` (`MW_AUDIO_DIST=raw`).
#[inline]
pub fn distort(x: f32, amount: f32) -> f32 {
    let d = amount.max(1e-3);
    (x * d).tanh() / d
}

/// `tanh(d · x)`: the previous stand-in, kept for A/B listening.
#[inline]
pub fn distort_raw(x: f32, amount: f32) -> f32 {
    (x * amount.max(0.0)).tanh()
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

    #[test]
    fn sub_harmoniser_halves_the_frequency() {
        let sr = 48_000.0;
        let mut s = SubHarmoniser::default();
        let f = 200.0;
        let out: Vec<f32> = (0..48_000).map(|i| s.process((TAU * f * i as f32 / sr).sin(), 1, sr)).collect();
        // Count sign changes of the output over the last half second: 100 Hz square = 100 per second.
        let changes = out[24_000..].windows(2).filter(|w| (w[0] > 0.0) != (w[1] > 0.0)).count();
        assert!((95..=105).contains(&changes), "{changes}");
        // Nothing to divide above the tracking limit (a 6.8 kHz high-passed input is hiss, not firing).
        let mut s = SubHarmoniser::default();
        let out: Vec<f32> = (0..48_000).map(|i| s.process((TAU * 3000.0 * i as f32 / sr).sin(), 1, sr)).collect();
        assert!(out[24_000..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn distortion_keeps_small_signals_at_unity() {
        assert!((distort(0.01, 4.0) / 0.01 - 1.0).abs() < 0.01);
        assert!((distort(0.01, 0.4) / 0.01 - 1.0).abs() < 0.001);
        // Large signals compress: the drive only decides where.
        assert!(distort(1.0, 4.0) < 0.26 && distort(1.0, 4.0) > 0.24);
        assert!((distort_raw(1.0, 4.0) - 4f32.tanh()).abs() < 1e-6);
    }
}
