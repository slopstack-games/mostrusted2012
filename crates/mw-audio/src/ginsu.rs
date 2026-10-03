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
//! | 0x20 | u32 × (n1+1) | RPM table: grain index for evenly spaced RPMs |
//! | …    | u32 × (n2+1) | grain start sample offsets |
//! | …    | XAS v0 frames | audio |
//!
//! Format from vgmstream `meta/gin.c` (ISC); the table semantics are our
//! interpretation (see docs/research-log.md).

use crate::{Error, Pcm, Result, xas};

#[derive(Debug, Clone)]
pub struct Ginsu {
    pub min_rpm: f32,
    pub max_rpm: f32,
    /// For evenly spaced RPMs between min and max: index into `grains`.
    pub rpm_table: Vec<u32>,
    /// Grain start offsets (samples), ascending.
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
        let rpm_table = (0..=n1).map(|i| u32le(d, 0x20 + i * 4)).collect::<Result<Vec<_>>>()?;
        let grains_at = 0x20 + (n1 + 1) * 4;
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

    /// Grain index for an RPM (clamped to the recorded range).
    pub fn grain_for_rpm(&self, rpm: f32) -> usize {
        let n = self.rpm_table.len().max(1);
        let t = ((rpm - self.min_rpm) / (self.max_rpm - self.min_rpm).max(1.0)).clamp(0.0, 1.0);
        let i = (t * (n - 1) as f32).round() as usize;
        (self.rpm_table.get(i).copied().unwrap_or(0) as usize).min(self.grains.len().saturating_sub(2))
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
    /// Previous grain tail for the cross-fade.
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
            // Keep the tail of the finished grain to fade out under the next one.
            let tail_start = self.end.saturating_sub(FADE).max(self.pos.saturating_sub(FADE));
            self.fade = samples.get(tail_start..self.end.min(samples.len())).unwrap_or(&[]).iter().map(|&s| s as f32).collect();
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
