//! Ginsu (`Gnsu20`/`Gnsu30`): EA's granular engine sounds.
//!
//! A Ginsu asset is a mono recording of an engine sweeping through its rev
//! range, cut into grains. Playback picks grains by RPM and stitches them
//! together, which is how the game's engine pitch follows the RPM without
//! resampling artefacts.
//!
//! Layout (offsets from the `Gnsu` magic; `Ginsu` resources prefix it with
//! 8 bytes):
//!
//! | off  | type | field |
//! |------|------|-------|
//! | 0x00 | char[4] | `Gnsu` |
//! | 0x04 | char[4] | version, e.g. `30\0\0` |
//! | 0x08 | f32  | minimum RPM |
//! | 0x0C | f32  | maximum RPM |
//! | 0x10 | u32  | n1: RPM table entries − 1 |
//! | 0x14 | u32  | n2: grain table entries − 1 |
//! | 0x18 | u32  | total samples |
//! | 0x1C | u32  | sample rate |
//! | 0x20 | u32  | unknown (0) |
//! | 0x24 | u32 × (n1+1) | RPM table: recording position (samples) at evenly spaced RPMs from min to max |
//! | …    | u32 × (n2+1) | grain boundaries (samples), ascending, ending at the sample count |
//! | …    | XAS v0 frames | audio |
//!
//! Header from vgmstream `meta/gin.c` (ISC). The extra field at 0x20 and the
//! table meanings were verified on the data: with the audio 4 bytes later
//! than vgmstream's offset, XAS frame boundaries are as smooth as the frame
//! interiors, and the RPM table rises for acceleration sweeps and falls for
//! deceleration ones.

use crate::{Error, Pcm, Result, xas};

#[derive(Debug, Clone)]
pub struct Ginsu {
    pub min_rpm: f32,
    pub max_rpm: f32,
    /// For evenly spaced RPMs between min and max: position in the recording
    /// (samples). Stored as u32 but signed: short recordings start at about
    /// -2800, so read entries with [`Ginsu::rising`] / `as i32`.
    pub rpm_table: Vec<u32>,
    /// Grain boundaries (samples), ascending.
    pub grains: Vec<u32>,
    pub pcm: Pcm,
}

fn u32le(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).ok_or(Error::Truncated)?.try_into().unwrap()))
}

impl Ginsu {
    /// Parse a Ginsu resource or raw `.gin` data (finds the `Gnsu` magic).
    pub fn parse(data: &[u8]) -> Result<Self> {
        let base = (0..data.len().min(0x40))
            .find(|&i| data[i..].starts_with(b"Gnsu"))
            .ok_or_else(|| Error::Format("no Gnsu magic".into()))?;
        let d = &data[base..];
        let f = |o: usize| u32le(d, o).map(f32::from_bits);
        let (n1, n2) = (u32le(d, 0x10)? as usize, u32le(d, 0x14)? as usize);
        if n1 > 1 << 16 || n2 > 1 << 20 {
            return Err(Error::Format("implausible Ginsu tables".into()));
        }
        let samples = u32le(d, 0x18)? as usize;
        let sample_rate = u32le(d, 0x1C)?;
        let rpm_table = (0..=n1).map(|i| u32le(d, 0x24 + i * 4)).collect::<Result<Vec<_>>>()?;
        let grains_at = 0x24 + (n1 + 1) * 4;
        let grains = (0..=n2).map(|i| u32le(d, grains_at + i * 4)).collect::<Result<Vec<_>>>()?;
        let audio = d.get(grains_at + (n2 + 1) * 4..).ok_or(Error::Truncated)?;
        Ok(Self {
            min_rpm: f(0x08)?,
            max_rpm: f(0x0C)?,
            rpm_table,
            grains,
            pcm: Pcm { channels: 1, sample_rate, samples: xas::decode_v0(audio, samples) },
        })
    }

    /// Whether the recording sweeps up in RPM (on load) rather than down.
    pub fn rising(&self) -> bool {
        let signed = |v: Option<&u32>| v.map(|&v| v as i32);
        signed(self.rpm_table.last()) > signed(self.rpm_table.first())
    }

    /// Position in the recording (samples) for an RPM, interpolated.
    pub fn position_for_rpm(&self, rpm: f32) -> f32 {
        let n = self.rpm_table.len();
        if n == 0 {
            return 0.0;
        }
        let t = ((rpm - self.min_rpm) / (self.max_rpm - self.min_rpm).max(1.0)).clamp(0.0, 1.0) * (n - 1) as f32;
        let (i, f) = (t.floor() as usize, t.fract());
        // Entries are signed: short recordings start slightly before zero.
        let a = self.rpm_table[i] as i32 as f32;
        let b = self.rpm_table[(i + 1).min(n - 1)] as i32 as f32;
        (a + (b - a) * f).max(0.0)
    }

    /// RPM at which a recording position was recorded: the inverse of [`Ginsu::position_for_rpm`] (the
    /// table is monotonic, rising or falling).
    pub fn rpm_at_position(&self, pos: f32) -> f32 {
        let n = self.rpm_table.len();
        if n < 2 {
            return self.min_rpm;
        }
        let at = |i: usize| self.rpm_table[i] as i32 as f32;
        let rising = self.rising();
        // First entry past `pos` in sweep order.
        let k = (1..n).find(|&i| if rising { at(i) >= pos } else { at(i) <= pos }).unwrap_or(n - 1);
        let (a, b) = (at(k - 1), at(k));
        let f = if (b - a).abs() > f32::EPSILON { ((pos - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
        let step = (self.max_rpm - self.min_rpm) / (n - 1) as f32;
        self.min_rpm + ((k - 1) as f32 + f) * step
    }

    /// Grain containing the recording position for an RPM.
    pub fn grain_for_rpm(&self, rpm: f32) -> usize {
        let pos = self.position_for_rpm(rpm) as u32;
        let i = self.grains.partition_point(|&g| g <= pos).saturating_sub(1);
        i.min(self.grains.len().saturating_sub(2))
    }

    /// Sample range of grain `i`.
    pub fn grain(&self, i: usize) -> (usize, usize) {
        let a = self.grains.get(i).copied().unwrap_or(0) as usize;
        let b = self.grains.get(i + 1).copied().map_or(self.pcm.samples.len(), |b| b as usize);
        (a.min(self.pcm.samples.len()), b.clamp(a, self.pcm.samples.len()))
    }
}

/// Granular resynthesis of one Ginsu recording by overlap-add of engine cycles. [`EngineMixer`] drives it.
///
/// Grains are engine cycles: grain boundaries mark cycle starts, and a grain's length matches the cycle
/// period at the RPM it was recorded at (`120 · sample_rate / rpm` for these four-stroke recordings,
/// research log 2026-10-04). Output is a stream of periods set by the mixer. During each period the
/// current grain's cycle fades out while the next grain's preceding cycle fades in, with complementary
/// windows (cos² + sin² = 1). Both are resampled so that one source cycle fills exactly one output period,
/// so the pitch follows the RPM continuously.
///
/// The next grain is usually the recording's following cycle, so mostly the recording plays straight
/// through: consecutive grains share their samples and the blend is seamless. Now and then, and when the
/// walk leaves a window of a few cycles around the RPM's position, it jumps to a random cycle near the
/// target. This replaced concatenating whole grains with a 2 ms cross-fade while alternating between two of
/// them, which sounded like looped snippets.
pub struct GinsuSynth {
    pub ginsu: std::sync::Arc<Ginsu>,
    /// RPM each grain was recorded at (at its centre).
    grain_rpm: Vec<f32>,
    /// Grain whose cycle fades out, and grain whose preceding cycle fades in, during this period.
    current: usize,
    next: usize,
}

/// The walk stays within this many grains of the RPM's position in the recording.
const WINDOW: usize = 2;

/// Catmull-Rom interpolation of the recording at a fractional position.
fn sample_at(samples: &[i16], pos: f32) -> f32 {
    let i = pos.floor() as isize;
    let t = pos - pos.floor();
    let s = |k: isize| samples.get((i + k).max(0) as usize).copied().unwrap_or(0) as f32;
    let (p0, p1, p2, p3) = (s(-1), s(0), s(1), s(2));
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

impl GinsuSynth {
    pub fn new(ginsu: std::sync::Arc<Ginsu>) -> Self {
        let grain_rpm: Vec<f32> = (0..ginsu.grains.len().saturating_sub(1))
            .map(|i| {
                let (a, b) = ginsu.grain(i);
                ginsu.rpm_at_position((a + b) as f32 * 0.5)
            })
            .collect();
        let start = ginsu.grain_for_rpm(ginsu.min_rpm);
        Self { ginsu, grain_rpm, current: start, next: start + 1 }
    }

    fn grains(&self) -> usize {
        self.grain_rpm.len()
    }

    /// Start the next output period: the incoming grain becomes current and a new one is picked. `rnd` is
    /// shared by all layers of an engine, so layers recorded together (same grain table) pick the same
    /// cycle and stay coherent.
    pub fn start_period(&mut self, rpm: f32, rnd: u32) {
        self.current = self.next;
        self.next = self.pick_next(self.current, rpm, rnd);
    }

    /// The grain after `from` for this RPM: the next cycle of the recording seven periods in eight while it
    /// stays within ±2 grains of the RPM's grain, else a random grain in that window. Tuned on steady holds
    /// (2-20 Hz loudness modulation, research log): a strict walk with jump-backs made a short loop, and
    /// wider windows or more frequent jumps modulate more.
    fn pick_next(&self, from: usize, rpm: f32, rnd: u32) -> usize {
        let n = self.grains();
        if n < 2 {
            return 0;
        }
        let target = self.ginsu.grain_for_rpm(rpm).min(n - 1);
        let following = from + 1;
        if following < n && following.abs_diff(target) <= WINDOW && !rnd.is_multiple_of(8) {
            return following;
        }
        let r = ((rnd >> 8) % (2 * WINDOW as u32 + 1)) as usize;
        (target + r).saturating_sub(WINDOW).min(n - 2)
    }

    /// Output period (samples) for this RPM: the current grain's length scaled by its recorded RPM over
    /// the RPM.
    pub fn period_for(&self, rpm: f32) -> f32 {
        let (a, b) = self.ginsu.grain(self.current);
        let recorded = self.grain_rpm.get(self.current).copied().unwrap_or(rpm).max(1.0);
        ((b - a).max(1) as f32 * recorded / rpm.max(1.0)).clamp(16.0, 16384.0)
    }

    /// Sample at phase `x` (0..1) of the current period, in -1..1.
    pub fn sample(&self, x: f32) -> f32 {
        if self.grains() == 0 {
            return 0.0;
        }
        let samples = &self.ginsu.pcm.samples;
        let (a, b) = self.ginsu.grain(self.current);
        let out = sample_at(samples, a as f32 + x * (b - a) as f32);
        let (c, _) = self.ginsu.grain(self.next);
        let (p, _) = self.ginsu.grain(self.next.saturating_sub(1));
        let inc = sample_at(samples, p as f32 + x * (c - p) as f32);
        let w = (x * std::f32::consts::FRAC_PI_2).sin().powi(2);
        (out * (1.0 - w) + inc * w) / 32768.0
    }
}

/// One recording in an engine mix: a Ginsu, its mixer channel gain and its gain curve against engine
/// load (`(load, gain)` points, piecewise linear; empty = 1).
#[derive(Debug, Clone)]
pub struct EngineLayer {
    pub ginsu: std::sync::Arc<Ginsu>,
    pub gain: f32,
    pub load_curve: Vec<(f32, f32)>,
}

fn curve_at(curve: &[(f32, f32)], x: f32) -> f32 {
    let Some(first) = curve.first() else { return 1.0 };
    if x <= first.0 {
        return first.1.max(0.0);
    }
    for w in curve.windows(2) {
        let ((a, ya), (b, yb)) = (w[0], w[1]);
        if x <= b {
            let f = if b > a { (x - a) / (b - a) } else { 0.0 };
            return (ya + (yb - ya) * f).max(0.0);
        }
    }
    curve.last().map_or(1.0, |l| l.1.max(0.0))
}

/// A car's engine voice as the game mixes it: every layer plays at once through its own granular synth,
/// weighted by its channel gain and its load curve (the 911: two microphone sets of full-load, part-load
/// and off-load recordings, crossfaded by load). RPM and load are smoothed per sample (the game updates
/// them at 64 Hz). Shared by the runtime and `mwtool engine-sound`, so offline renders match what is
/// heard in game.
/// A layer's synth with its channel gain and load curve.
struct Voice {
    synth: GinsuSynth,
    gain: f32,
    load_curve: Vec<(f32, f32)>,
}

pub struct EngineMixer {
    voices: Vec<Voice>,
    master: f32,
    pub sample_rate: u32,
    rpm: f32,
    load: f32,
    /// Shared period schedule: every layer starts its cycles together, so the firing pulses of layers
    /// recorded in the same run (two microphones) and of the load layers line up.
    t: f32,
    period: f32,
    seed: u32,
}

/// Overall output scale before the soft limiter: the layers sum to roughly twice one recording's level.
const MIX_SCALE: f32 = 0.6;

impl EngineMixer {
    pub fn new(layers: Vec<EngineLayer>, master_gain: f32) -> Self {
        let sample_rate = layers.first().map_or(48_000, |l| l.ginsu.pcm.sample_rate);
        let rpm = layers.iter().map(|l| l.ginsu.min_rpm).fold(f32::MAX, f32::min);
        let voices: Vec<Voice> = layers
            .into_iter()
            .map(|l| Voice { synth: GinsuSynth::new(l.ginsu), gain: l.gain, load_curve: l.load_curve })
            .collect();
        let rpm = if rpm.is_finite() { rpm } else { 1000.0 };
        let period = voices.first().map_or(1000.0, |v| v.synth.period_for(rpm));
        Self { voices, master: master_gain, sample_rate, rpm, load: 0.0, t: 0.0, period, seed: 0x9e37_79b9 }
    }

    /// Fallback for cars without engine-sound data: an on-load and an off-load recording with
    /// equal-power load curves.
    pub fn on_off_layers(on: std::sync::Arc<Ginsu>, off: Option<std::sync::Arc<Ginsu>>) -> Vec<EngineLayer> {
        let pts = |f: fn(f32) -> f32| (0..=8).map(|i| i as f32 / 8.0).map(|x| (x, f(x))).collect::<Vec<_>>();
        let mut layers = vec![EngineLayer {
            ginsu: on,
            gain: 1.0,
            load_curve: if off.is_some() {
                pts(|x| (x * std::f32::consts::FRAC_PI_2).sin())
            } else {
                vec![(0.0, 0.4), (1.0, 1.0)]
            },
        }];
        if let Some(off) = off {
            layers.push(EngineLayer {
                ginsu: off,
                gain: 1.0,
                load_curve: pts(|x| (x * std::f32::consts::FRAC_PI_2).cos()),
            });
        }
        layers
    }

    /// Next sample in -1..1 towards the target RPM and load (0..1).
    pub fn next_sample(&mut self, rpm: f32, load: f32) -> f32 {
        let sr = self.sample_rate.max(1) as f32;
        // 10 ms bridges the 64 Hz RPM updates without zipper noise; load moves over 50 ms so throttle
        // taps crossfade the layers instead of switching them.
        self.rpm += (rpm - self.rpm) / (0.01 * sr);
        self.load += (load.clamp(0.0, 1.0) - self.load) / (0.05 * sr);
        if self.t >= self.period {
            self.t -= self.period;
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            for v in &mut self.voices {
                v.synth.start_period(self.rpm, self.seed);
            }
            // The first layer (the main rev-up) sets the period.
            self.period = self.voices.first().map_or(self.period, |v| v.synth.period_for(self.rpm));
        }
        let x = (self.t / self.period).min(1.0);
        self.t += 1.0;
        let mut sum = 0.0;
        for v in &self.voices {
            // Silent layers keep their place so they come back in step.
            let w = v.gain * curve_at(&v.load_curve, self.load);
            if w > 0.0 {
                sum += v.synth.sample(x) * w;
            }
        }
        (sum * self.master * MIX_SCALE).tanh()
    }
}

/// Pick the on-load and off-load recordings from a vehicle's Ginsu resources
/// (in bundle order).
///
/// A car has two sets of three long recordings, in resource order (likely
/// engine and exhaust microphones): the main rev-up under load, a second
/// rising sweep from lower RPM, and a falling sweep (off load). It also has a
/// short copy of each main rev-up with fewer grains (same RPM range, same first
/// and last grains), presumably for the lower audio quality setting. So the
/// main rev-up is the rising recording that has a short copy, and its off-load
/// partner is the falling recording of the same set, the nearest in resource
/// order. Mixing the sets changed timbre, loudness and RPM range at every
/// throttle change. Without short copies this falls back to the rising
/// recording with the most grains.
pub fn pick_engine(ginsus: Vec<Ginsu>) -> Option<(std::sync::Arc<Ginsu>, Option<std::sync::Arc<Ginsu>>)> {
    let same_range = |a: &Ginsu, b: &Ginsu| (a.min_rpm - b.min_rpm).abs() < 1.0 && (a.max_rpm - b.max_rpm).abs() < 1.0;
    let rising: Vec<usize> = (0..ginsus.len()).filter(|&i| ginsus[i].rising()).collect();
    let has_short_copy = |i: usize| {
        rising
            .iter()
            .any(|&j| j != i && ginsus[j].grains.len() < ginsus[i].grains.len() && same_range(&ginsus[i], &ginsus[j]))
    };
    let on = rising
        .iter()
        .copied()
        .find(|&i| has_short_copy(i))
        .or_else(|| rising.iter().copied().max_by_key(|&i| (ginsus[i].grains.len(), std::cmp::Reverse(i))))?;
    let off = (0..ginsus.len()).filter(|&i| !ginsus[i].rising()).min_by_key(|&i| i.abs_diff(on));
    let mut slots: Vec<Option<Ginsu>> = ginsus.into_iter().map(Some).collect();
    let on_g = slots[on].take()?;
    let off_g = off.and_then(|i| slots[i].take());
    Some((std::sync::Arc::new(on_g), off_g.map(std::sync::Arc::new)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ginsu(min_rpm: f32, max_rpm: f32, rpm_table: Vec<u32>, grains: usize) -> Ginsu {
        Ginsu {
            min_rpm,
            max_rpm,
            rpm_table,
            grains: (0..=grains as u32).map(|g| g * 100).collect(),
            pcm: Pcm { channels: 1, sample_rate: 48000, samples: vec![0; grains * 100] },
        }
    }

    #[test]
    fn rpm_at_position_inverts_the_table() {
        let up = ginsu(1000.0, 3000.0, vec![0, 100, 400], 3);
        assert_eq!(up.rpm_at_position(0.0), 1000.0);
        assert_eq!(up.rpm_at_position(100.0), 2000.0);
        assert_eq!(up.rpm_at_position(250.0), 2500.0);
        let down = ginsu(1000.0, 3000.0, vec![400, 100, 0], 3);
        assert_eq!(down.rpm_at_position(400.0), 1000.0);
        assert_eq!(down.rpm_at_position(50.0), 2500.0);
    }

    #[test]
    fn signed_rpm_table() {
        // Short recordings start just below zero (-2832 on the 911).
        let g = ginsu(1575.0, 7814.0, vec![(-2832i32) as u32, 2704, 141160], 10);
        assert!(g.rising());
        assert_eq!(g.position_for_rpm(1575.0), 0.0);
    }

    #[test]
    fn picks_main_rev_up_and_falling_sweep_of_its_set() {
        // The 911's layout: two sets of (main rev-up, low rising sweep,
        // falling sweep), then short copies of the main rev-ups.
        let up = |lo, n| ginsu(lo, 7814.0, vec![721, 500_000], n);
        let down = |lo, n| ginsu(lo, 7771.0, vec![365_067, 721], n);
        let short = |lo| ginsu(lo, 7814.0, vec![(-2800i32) as u32, 141_160], 134);
        let set = vec![
            up(1575.0, 505),
            up(1125.0, 431),
            down(1189.0, 270),
            up(1614.0, 505),
            up(1081.0, 432),
            down(1116.0, 277),
            short(1575.0),
            short(1614.0),
        ];
        let (on, off) = pick_engine(set).unwrap();
        assert_eq!((on.min_rpm, on.grains.len()), (1575.0, 506));
        let off = off.unwrap();
        assert!(!off.rising());
        assert_eq!(off.min_rpm, 1189.0);
    }
}
