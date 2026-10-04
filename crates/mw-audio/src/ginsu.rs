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

/// Real-time granular resynthesis: plays grains chosen by the current RPM,
/// cross-fading between them.
pub struct GinsuSynth {
    pub ginsu: std::sync::Arc<Ginsu>,
    grain: usize,
    pos: usize,
    end: usize,
    /// Continuation of the previous grain, faded out under the new one.
    fade: Vec<f32>,
    fade_pos: usize,
    /// Alternate up/down around the target grain to avoid a buzz from
    /// repeating one grain.
    flip: bool,
}

const FADE: usize = 96;

impl GinsuSynth {
    pub fn new(ginsu: std::sync::Arc<Ginsu>) -> Self {
        let (pos, end) = ginsu.grain(0);
        Self { ginsu, grain: 0, pos, end, fade: Vec::new(), fade_pos: 0, flip: false }
    }

    /// Next mono sample in -1..1 for the given RPM.
    pub fn next_sample(&mut self, rpm: f32) -> f32 {
        let samples = &self.ginsu.pcm.samples;
        if self.pos >= self.end || self.pos >= samples.len() {
            // Cross-fade the new grain with the old grain's natural
            // continuation in the recording (what would have played next).
            let cont = self.end.min(samples.len());
            self.fade = samples[cont..(cont + FADE).min(samples.len())].iter().map(|&s| s as f32).collect();
            self.fade_pos = 0;
            let target = self.ginsu.grain_for_rpm(rpm);
            self.flip = !self.flip;
            let next = if self.flip { target } else { (target + 1).min(self.ginsu.grains.len().saturating_sub(2)) };
            self.grain = next;
            let (a, b) = self.ginsu.grain(next);
            self.pos = a;
            self.end = b.max(a + 1);
        }
        let mut v = samples.get(self.pos).copied().unwrap_or(0) as f32;
        self.pos += 1;
        if self.fade_pos < self.fade.len() {
            let t = self.fade_pos as f32 / self.fade.len() as f32;
            v = v * t + self.fade[self.fade_pos] * (1.0 - t);
            self.fade_pos += 1;
        }
        v / 32768.0
    }
}

/// A car's engine voice: on-load and off-load synths mixed by throttle, with
/// the RPM and load smoothed per sample (the game updates them at 64 Hz).
/// Shared by the runtime and `mwtool engine-sound`, so offline renders match
/// what is heard in game.
pub struct EngineMixer {
    on: GinsuSynth,
    off: Option<GinsuSynth>,
    pub sample_rate: u32,
    rpm: f32,
    load: f32,
}

impl EngineMixer {
    pub fn new(on: std::sync::Arc<Ginsu>, off: Option<std::sync::Arc<Ginsu>>) -> Self {
        let rpm = on.min_rpm;
        Self { sample_rate: on.pcm.sample_rate, on: GinsuSynth::new(on), off: off.map(GinsuSynth::new), rpm, load: 0.0 }
    }

    /// Next sample in -1..1 towards the target RPM and load (0..1).
    pub fn next_sample(&mut self, rpm: f32, load: f32) -> f32 {
        let k = 1.0 / (0.03 * self.sample_rate.max(1) as f32);
        self.rpm += (rpm - self.rpm) * k;
        self.load += (load - self.load) * k * 0.5;
        let on = self.on.next_sample(self.rpm);
        match &mut self.off {
            Some(off) => {
                let off = off.next_sample(self.rpm);
                // Equal-power cross-fade: the two recordings are uncorrelated,
                // so a linear one dips by 3 dB halfway through.
                let a = self.load.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
                on * a.sin() + off * a.cos()
            }
            None => on * (0.4 + 0.6 * self.load),
        }
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
