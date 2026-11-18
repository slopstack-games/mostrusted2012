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

/// Coefficients `[b0, b1, b2, a0, a1, a2]` of `BandPassIir2` as the game sets them up (`FUN_00ce65b0`, design
/// `FUN_00cdfef0`), or `None` when the game skips the filter (its buffer passes unchanged and its state is zeroed).
/// Frequency and Bandwidth (Hz) become angles `2π f / sr` (`.rdata` 0x0106f5a8); the centre is clamped to
/// 0.001π..0.999π, the bandwidth to at least 0.001π (0x0106f9c8 / 0x0106fa38). The filter runs while the band fits:
/// bandwidth ≤ 2 (π − w) for w ≤ π/2, ≤ 2 w above (0x00f9c360 = 2). Design: RBJ constant-0-dB band-pass with
/// Q = w / bandwidth capped at 20 (0x00fa3ba0) and no lower bound.
pub fn band_pass_game(sr: f32, freq: f32, bandwidth: f32) -> Option<[f32; 6]> {
    let k = TAU / sr;
    let (lo, hi) = (0.001 * PI, 0.999 * PI);
    let mut w = freq * k;
    if w < lo {
        w = lo;
    }
    if w > hi {
        w = hi;
    }
    let mut b = bandwidth * k;
    if b < lo {
        b = lo;
    }
    let runs = (w > PI / 2.0 || b <= (PI - w) * 2.0) && (w <= PI / 2.0 || b <= w * 2.0);
    if !runs {
        return None;
    }
    let q = (w / b).min(20.0);
    let alpha = w.sin() / (q + q);
    Some([alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * w.cos(), 1.0 - alpha])
}

/// Q values of the biquad sections of an even-order Butterworth filter.
pub fn butterworth_q(order: u32) -> Vec<f32> {
    let n = order.clamp(2, 8) & !1;
    (0..n / 2).map(|k| 1.0 / (2.0 * (PI * (2 * k + 1) as f32 / (2 * n) as f32).cos())).collect()
}

/// `SubHarmoniserDspPlugIn` ('SUBH', class descriptor `0x010d2410`; process `0x00aad950`, init `0x00ab2000`, both
/// in a gap of the decompiled dump, read from the disassembly). Its state is the last input sample (+0x38) and a
/// half-cycle counter (+0x3c), both zeroed only by the init (the plug-in's creation); nothing else writes them. On
/// every sign change between the previous and the current sample (`x < 0` against `prev < 0`) the counter steps
/// modulo `Interval` (an integer parameter, type 5: the data value rounded; 0 reads as 1), and the output is `|x|`
/// while the counter is 0, else 0. With the data's `Interval` 1 that is a full-wave rectifier (|x| always); 2 passes
/// every other half cycle. Bounded by the input: it cannot latch.
#[derive(Debug, Clone, Default)]
pub struct SubHarmoniser {
    prev: f32,
    count: u32,
}

impl SubHarmoniser {
    #[inline]
    pub fn process(&mut self, x: f32, interval: u32) -> f32 {
        let n = interval.max(1);
        if (self.prev < 0.0) != (x < 0.0) {
            self.count = (self.count + 1) % n;
        }
        self.prev = x;
        if self.count == 0 { x.abs() } else { 0.0 }
    }
}

/// `Interval` as the plug-in stores it: parameter type 5, `ROUND` of the value (`FUN_00ce30f0`), as an unsigned
/// divisor.
#[inline]
pub fn sub_interval(v: f32) -> u32 {
    v.round_ties_even() as u32
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

/// `DistortionDspPlugIn` ('Dst0', class descriptor `0x010d1930`; process `0x00ab0e60`, in a gap of the decompiled
/// dump, read from the disassembly). Stateless. `Model` (+0x38, an integer parameter, type 0: truncated) picks the
/// shaper: 2 = `FUN_00aae7a0`, a hard clip of `x · (Distortion + 0.99999)` to ±1 (`.rdata` 0x0100e950, 0x00f8f340,
/// 0x00f8a820; a NaN product gives +1 as the `comiss` chain does); 1 and 3 leave the buffer untouched; any other
/// model is `FUN_00aac300`, `f(d · x) / f(d)` with `f` the CRT routine `0x00edc7b2` (not identified; the data only
/// uses model 2, so `tanh` stands in for it). `Distortion` is a float parameter, default 1, 1e-5..1000.
#[inline]
pub fn distort(x: f32, drive: f32, model: i32) -> f32 {
    match model {
        1 | 3 => x,
        2 => {
            let v = x * (drive + 0.99999);
            if -1.0 > v {
                -1.0
            } else if 1.0 > v {
                v
            } else {
                1.0
            }
        }
        // MODEL: `f` of `FUN_00aac300` not identified; tanh.
        _ => (x * drive).tanh() / drive.tanh(),
    }
}

/// `Model` as the plug-in stores it: parameter type 0, truncated to an integer (`FUN_00ce30f0`).
#[inline]
pub fn distortion_model(v: f32) -> i32 {
    v as i32
}

/// The stand-in used before the port, `tanh(d · x) / d`, kept for A/B listening (`MW_AUDIO_DIST=standin`).
#[inline]
pub fn distort_standin(x: f32, amount: f32) -> f32 {
    let d = amount.max(1e-3);
    (x * d).tanh() / d
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
    fn sub_harmoniser_follows_the_code() {
        let sr = 48_000.0;
        let sine = |i: usize| (TAU * 200.0 * i as f32 / sr).sin();
        // Interval 1 (the data's): |x| on every sample.
        let mut s = SubHarmoniser::default();
        assert!((0..4800).all(|i| s.process(sine(i), 1) == sine(i).abs()));
        // Interval 2: every other half cycle passes (rectified), the rest is 0; never negative.
        let mut s = SubHarmoniser::default();
        let out: Vec<f32> = (0..4800).map(|i| s.process(sine(i), 2)).collect();
        assert!(out.iter().all(|v| *v >= 0.0));
        let zeros = out.iter().filter(|v| **v == 0.0).count();
        assert!((2300..=2500).contains(&zeros), "{zeros}");
        // Interval 0 reads as 1; a NaN input does not stick.
        let mut s = SubHarmoniser::default();
        assert_eq!(s.process(-0.5, 0), 0.5);
        assert!(s.process(f32::NAN, sub_interval(1.0)).is_nan());
        assert_eq!(s.process(0.25, 1), 0.25);
        assert_eq!(sub_interval(1.4), 1);
        assert_eq!(sub_interval(-3.0), 0);
    }

    #[test]
    fn band_pass_follows_the_code() {
        let sr = 48_000.0;
        // The sub-harmoniser channel's band: 24 kHz wide around a 150 Hz firing tone runs, with Q ~0.006 (no floor).
        let c = band_pass_game(sr, 150.0, 24_000.0).unwrap();
        let w = TAU * 150.0 / sr;
        let alpha = w.sin() / (2.0 * w / PI);
        assert!((c[0] - alpha).abs() < 1e-5 && (c[3] - (1.0 + alpha)).abs() < 1e-5);
        // A band wider than 2 (π − w) is skipped: identity, state reset.
        assert!(band_pass_game(sr, 150.0, 1e9).is_none());
        let mut bq = Biquad::IDENTITY;
        bq.band_pass_game(sr, 150.0, 1e9);
        assert_eq!(bq.process(0.3), 0.3);
        // Above π/2 the limit is 2 w.
        assert!(band_pass_game(sr, 20_000.0, 30_000.0).unwrap()[0] > 0.0);
    }

    #[test]
    fn distortion_follows_the_code() {
        // Model 2: hard clip of x · (d + 0.99999).
        assert!((distort(0.1, 4.0, 2) - 0.499_999).abs() < 1e-6);
        assert_eq!(distort(0.5, 4.0, 2), 1.0);
        assert_eq!(distort(-0.5, 0.4, 2), -0.699_995);
        assert_eq!(distort(f32::NAN, 1.0, 2), 1.0);
        // Models 1 and 3 pass the input through.
        assert_eq!(distort(3.0, 4.0, 1), 3.0);
        assert_eq!(distort(3.0, 4.0, 3), 3.0);
        assert_eq!(distortion_model(2.0), 2);
        assert!((distort_standin(0.01, 4.0) / 0.01 - 1.0).abs() < 0.01);
    }
}
