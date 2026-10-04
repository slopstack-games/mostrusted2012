//! Audio decoders for NFS Most Wanted (2012).
//!
//! - **Ginsu** (`Gnsu20`/`Gnsu30`, resource type 0x80): granular engine
//!   recordings, EA-XAS v0 ADPCM. See [`ginsu`].
//! - **EA SPS** (`.SPS` streams, `Wave` resources at +0x80): EAAudioCore
//!   blocks carrying EALayer3, an MP3 variant. See [`sps`].
//!
//! The codec knowledge comes from vgmstream (ISC licence; see `NOTICE` in this
//! crate's source files), reimplemented in Rust.

mod bits;
pub mod dsp;
pub mod ginsu;
pub mod sequence;
pub mod sps;
pub mod xas;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a recognised audio stream: {0}")]
    Format(String),
    #[error("unsupported audio codec {0}")]
    Codec(u8),
    #[error("truncated audio data")]
    Truncated,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Decoded PCM, interleaved.
#[derive(Debug, Clone, Default)]
pub struct Pcm {
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_secs(&self) -> f32 {
        self.frames() as f32 / self.sample_rate.max(1) as f32
    }

    /// Encode as a 16-bit PCM `.wav` file.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        let mut put = |b: &[u8]| out.extend_from_slice(b);
        put(b"RIFF");
        put(&(36 + data_len).to_le_bytes());
        put(b"WAVEfmt ");
        put(&16u32.to_le_bytes());
        put(&1u16.to_le_bytes());
        put(&self.channels.to_le_bytes());
        put(&self.sample_rate.to_le_bytes());
        put(&(self.sample_rate * self.channels as u32 * 2).to_le_bytes());
        put(&(self.channels * 2).to_le_bytes());
        put(&16u16.to_le_bytes());
        put(b"data");
        put(&data_len.to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}
