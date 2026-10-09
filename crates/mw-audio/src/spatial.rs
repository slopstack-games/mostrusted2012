//! A mono voice placed in the world: gain, stereo balance and doppler pitch, written by the game thread
//! ([`SpatialParams`]) and applied per sample by the audio thread ([`SpatialRender`]). The values come from
//! `mw_assets::car_sound::Spatial` (the game's `FUN_00ab9690` / `FUN_00ab7260`); the pan law is ours (the game's
//! mixer takes an azimuth, `FUN_00ab5a60`, whose panner is not in the dump): linear balance with both sides at
//! full level in the centre.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Shared between the game thread (writer) and the audio thread (reader).
#[derive(Debug)]
pub struct SpatialParams {
    gain: AtomicU32,
    pan: AtomicU32,
    pitch: AtomicU32,
}

impl Default for SpatialParams {
    fn default() -> Self {
        Self { gain: AtomicU32::new(0), pan: AtomicU32::new(0), pitch: AtomicU32::new(1f32.to_bits()) }
    }
}

impl SpatialParams {
    /// `gain` linear, `pan` −1 (left) .. 1 (right), `pitch` the playback-rate factor (doppler).
    pub fn set(&self, gain: f32, pan: f32, pitch: f32) {
        let ok = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.gain.store(ok(gain, 0.0).max(0.0).to_bits(), Ordering::Relaxed);
        self.pan.store(ok(pan, 0.0).clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
        self.pitch.store(ok(pitch, 1.0).clamp(0.05, 8.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> (f32, f32, f32) {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        (f(&self.gain), f(&self.pan), f(&self.pitch))
    }
}

/// Samples between reads of the shared values; gain, pan and pitch ramp linearly across each block.
const BLOCK: u32 = 64;

/// Turns a mono source into a placed stereo one: resampled by the doppler factor (linear interpolation), then
/// scaled and balanced.
pub struct SpatialRender {
    params: Arc<SpatialParams>,
    cur: (f32, f32, f32),
    step: (f32, f32, f32),
    counter: u32,
    /// Position between `prev` and `next` (0..1).
    pos: f32,
    prev: f32,
    next: f32,
    started: bool,
}

impl SpatialRender {
    pub fn new(params: Arc<SpatialParams>) -> Self {
        Self {
            params,
            cur: (0.0, 0.0, 1.0),
            step: (0.0, 0.0, 0.0),
            counter: 0,
            pos: 0.0,
            prev: 0.0,
            next: 0.0,
            started: false,
        }
    }

    /// The next stereo frame, pulling as many samples from `source` as the pitch needs.
    pub fn frame(&mut self, source: &mut impl FnMut() -> f32) -> (f32, f32) {
        if self.counter == 0 {
            let target = self.params.get();
            if !self.started {
                // Start at the first values instead of ramping up from silence.
                self.cur = target;
                self.started = true;
                self.prev = source();
                self.next = source();
            }
            let n = BLOCK as f32;
            self.step = ((target.0 - self.cur.0) / n, (target.1 - self.cur.1) / n, (target.2 - self.cur.2) / n);
        }
        self.counter = (self.counter + 1) % BLOCK;
        self.cur = (self.cur.0 + self.step.0, self.cur.1 + self.step.1, self.cur.2 + self.step.2);
        self.pos += self.cur.2.clamp(0.05, 8.0);
        while self.pos >= 1.0 {
            self.prev = self.next;
            self.next = source();
            self.pos -= 1.0;
        }
        let s = (self.prev + (self.next - self.prev) * self.pos) * self.cur.0.max(0.0);
        let p = self.cur.1.clamp(-1.0, 1.0);
        (s * (1.0 - p).min(1.0), s * (1.0 + p).min(1.0))
    }

    /// The current gain (diagnostics).
    pub fn gain(&self) -> f32 {
        self.cur.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doppler_resamples_and_pans() {
        let p = Arc::new(SpatialParams::default());
        p.set(0.5, 1.0, 2.0);
        let mut r = SpatialRender::new(p);
        let mut k = 0.0f32;
        let mut src = || {
            k += 1.0;
            k
        };
        let (l, rr) = r.frame(&mut src);
        assert_eq!(l, 0.0);
        assert!(rr > 0.0);
        // Pitch 2: two source samples per output frame.
        let before = r.next;
        r.frame(&mut src);
        assert!((r.next - before - 2.0).abs() < 1e-6);
    }
}
