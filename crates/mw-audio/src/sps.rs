//! EA SPS streams (EAAudioCore v1) with EALayer3 v1 ("EL31", codec 5).
//!
//! Blocks: `u8 id, u24 size (BE, incl. header)`. `H` (0x48) carries the
//! EAAC header; `D` (0x44) blocks hold `u32 samples` then codec data; `E`
//! (0x45) ends the stream. `Wave` resources contain an SPS stream at +0x80.
//!
//! EAAC header: `u32 BE: version:4 codec:4 channels-1:6 rate:18`,
//! `u32 BE: type:2 loop:1 samples:29`.
//!
//! EALayer3 frames each carry one MPEG Layer III granule with a compacted
//! header and side info (plus optional PCM blocks). We rebuild standard MPEG
//! frames, two granules per MPEG-1 frame, written as "free format" so no bit
//! reservoir is needed, and decode them with nanomp3. Multichannel audio is
//! interleaved EA-frames of several stereo/mono sub-streams.
//!
//! NOTICE: frame layout and rebuild procedure from vgmstream
//! (`mpeg_custom_utils_ealayer3.c`, `ea_eaac.c`), Copyright (c) 2008-2025 the
//! vgmstream authors, ISC licence: "Permission to use, copy, modify, and
//! distribute this software for any purpose with or without fee is hereby
//! granted, provided that the above copyright notice and this permission
//! notice appear in all copies." Originally reverse engineered by Zench.

use crate::bits::{BitReader, BitWriter};
use crate::{Error, Pcm, Result};

pub const CODEC_EALAYER3_V1: u8 = 5;

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub version: u8,
    pub codec: u8,
    pub channels: u16,
    pub sample_rate: u32,
    pub looping: bool,
    pub samples: u32,
}

/// Parse the leading `H` block. Returns the header and the offset after it.
pub fn header(data: &[u8]) -> Result<(Header, usize)> {
    let be = |o: usize| -> Result<u32> {
        Ok(u32::from_be_bytes(data.get(o..o + 4).ok_or(Error::Truncated)?.try_into().unwrap()))
    };
    let block = be(0)?;
    if block >> 24 != 0x48 {
        return Err(Error::Format("SPS stream must start with an H block".into()));
    }
    let (h1, h2) = (be(4)?, be(8)?);
    Ok((
        Header {
            version: (h1 >> 28) as u8,
            codec: ((h1 >> 24) & 0x0F) as u8,
            channels: ((h1 >> 18) & 0x3F) as u16 + 1,
            sample_rate: h1 & 0x3FFFF,
            looping: (h2 >> 29) & 1 != 0,
            samples: h2 & 0x1FFF_FFFF,
        },
        (block & 0x00FF_FFFF) as usize,
    ))
}

/// Concatenate the codec payload of all `D` blocks after `start`.
fn payload(data: &[u8], mut off: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while off + 8 <= data.len() {
        let block = u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
        let (id, size) = ((block >> 24) as u8, (block & 0x00FF_FFFF) as usize);
        if size < 4 || id == 0x45 {
            break;
        }
        if id == 0x44 {
            out.extend_from_slice(&data[(off + 8).min(data.len())..(off + size).min(data.len())]);
        }
        off += size;
    }
    out
}

/// One parsed EALayer3 frame (= one granule of one sub-stream).
#[derive(Debug, Clone, Default)]
struct EaFrame {
    version_index: u32,
    sample_rate_index: u32,
    channel_mode: u32,
    mode_extension: u32,
    granule: u32,
    scfsi: [u32; 2],
    main_data_bits: [u32; 2],
    others_1: [u32; 2],
    others_2: [u32; 2],
    channels: usize,
    mpeg1: bool,
    sample_rate: u32,
    /// Bit offset of the main data in the source buffer.
    data_bit: usize,
    /// PCM block (planar, big-endian) following the frame.
    pcm: Vec<i16>,
    pcm_samples: usize,
}

const SAMPLE_RATES: [[u32; 3]; 4] = [[11025, 12000, 8000], [0, 0, 0], [22050, 24000, 16000], [44100, 48000, 32000]];

/// Parse one EALayer3 v1b frame at byte `off`; returns the frame and its size.
fn parse_frame(buf: &[u8], off: usize) -> Option<(EaFrame, usize)> {
    let mut r = BitReader::new(buf);
    r.set_pos(off * 8);
    let pcm_flag = r.get(8)?;
    if pcm_flag != 0x00 && pcm_flag != 0xEE {
        return None;
    }
    let start = r.pos();
    let mut f = EaFrame {
        version_index: r.get(2)?,
        sample_rate_index: r.get(2)?,
        channel_mode: r.get(2)?,
        mode_extension: r.get(2)?,
        ..Default::default()
    };
    if f.version_index == 1
        || f.sample_rate_index == 3
        || (f.version_index | f.sample_rate_index | f.channel_mode | f.mode_extension) == 0
    {
        return None;
    }
    f.mpeg1 = f.version_index == 3;
    f.channels = if f.channel_mode == 3 { 1 } else { 2 };
    f.sample_rate = SAMPLE_RATES[f.version_index as usize][f.sample_rate_index as usize];
    let others_2_bits = if f.mpeg1 { 15 } else { 19 };
    f.granule = r.get(1)?;
    if f.mpeg1 && f.granule == 1 {
        for c in 0..f.channels {
            f.scfsi[c] = r.get(4)?;
        }
    }
    for c in 0..f.channels {
        f.main_data_bits[c] = r.get(12)?;
        f.others_1[c] = r.get(32)?;
        f.others_2[c] = r.get(others_2_bits)?;
    }
    f.data_bit = r.pos();
    let base = r.pos() - start;
    let data: usize = f.main_data_bits[..f.channels].iter().map(|&b| b as usize).sum();
    let padding = (8 - (base + data) % 8) % 8;
    r.set_pos(r.pos() + data + padding);
    if r.pos() > buf.len() * 8 {
        return None;
    }
    if pcm_flag == 0xEE {
        let _offset_samples = r.get(16)?;
        f.pcm_samples = r.get(16)? as usize;
        let _unknown = r.get(32)?;
        let at = r.pos() / 8;
        let bytes = buf.get(at..at + f.pcm_samples * f.channels * 2)?;
        // Planar in v1b: all of channel 0, then channel 1. Interleave here.
        let mut pcm = vec![0i16; f.pcm_samples * f.channels];
        for c in 0..f.channels {
            for i in 0..f.pcm_samples {
                let o = (c * f.pcm_samples + i) * 2;
                pcm[i * f.channels + c] = i16::from_be_bytes([bytes[o], bytes[o + 1]]);
            }
        }
        f.pcm = pcm;
        r.set_pos((at + bytes.len()) * 8);
    }
    Some((f, r.pos() / 8 - off))
}

/// Rebuild one MPEG frame from one (MPEG-2) or two (MPEG-1) granules.
fn rebuild(buf: &[u8], g0: &EaFrame, g1: Option<&EaFrame>, frame_bytes: usize) -> Option<Vec<u8>> {
    let mut w = BitWriter::default();
    w.put(11, 0x7FF);
    w.put(2, g0.version_index);
    w.put(2, 1); // layer III
    w.put(1, 1); // no CRC
    w.put(4, 0); // free format
    w.put(2, g0.sample_rate_index);
    w.put(1, 0);
    w.put(1, 0);
    w.put(2, g0.channel_mode);
    w.put(2, g0.mode_extension);
    w.put(1, 1);
    w.put(1, 1);
    w.put(2, 0);
    let mut r = BitReader::new(buf);
    if g0.mpeg1 {
        let g1 = g1?;
        w.put(9, 0); // main_data_begin: no reservoir
        w.put(if g0.channels == 1 { 5 } else { 3 }, 0);
        for c in 0..g1.channels {
            w.put(4, g1.scfsi[c]);
        }
        for g in [g0, g1] {
            for c in 0..g.channels {
                w.put(12, g.main_data_bits[c]);
                w.put(32, g.others_1[c]);
                w.put(15, g.others_2[c]);
            }
        }
        for g in [g0, g1] {
            r.set_pos(g.data_bit);
            for c in 0..g.channels {
                w.copy(&mut r, g.main_data_bits[c] as usize)?;
            }
        }
    } else {
        w.put(8, 0);
        w.put(if g0.channels == 1 { 1 } else { 2 }, 0);
        for c in 0..g0.channels {
            w.put(12, g0.main_data_bits[c]);
            w.put(32, g0.others_1[c]);
            w.put(19, g0.others_2[c]);
        }
        r.set_pos(g0.data_bit);
        for c in 0..g0.channels {
            w.copy(&mut r, g0.main_data_bits[c] as usize)?;
        }
    }
    w.align();
    if w.data.len() > frame_bytes {
        return None;
    }
    w.pad_to(frame_bytes);
    Some(w.data)
}

/// Decode one sub-stream's EA-frames to interleaved PCM.
fn decode_stream(buf: &[u8], frames: &[EaFrame]) -> Option<(usize, Vec<i16>)> {
    let first = frames.first()?;
    let channels = first.channels;
    // Free-format frames must all have one size; use the spec's 640/320 kbps
    // size, capped by minimp3's free-format limit.
    let nominal = if first.mpeg1 { 144 * 640_000 } else { 72 * 320_000 } / first.sample_rate.max(1) as usize;
    let frame_bytes = nominal.min(2304);
    let mut mp3 = Vec::new();
    let mut prefix_pcm: Vec<i16> = Vec::new();
    let mut discard = 529 + 576; // decoder delay + first granule
    let mut i = 0;
    while i < frames.len() {
        let g0 = &frames[i];
        let g1 = if g0.mpeg1 { frames.get(i + 1) } else { None };
        for g in std::iter::once(g0).chain(g1) {
            if g.pcm_samples > 0 {
                // PCM blocks replace the matching decoded samples.
                prefix_pcm.extend_from_slice(&g.pcm);
                discard += g.pcm_samples;
            }
        }
        if let Some(frame) = rebuild(buf, g0, g1, frame_bytes) {
            mp3.extend_from_slice(&frame);
        }
        i += if g0.mpeg1 { 2 } else { 1 };
    }
    let decoded = nanomp3::decode_all::<i16>(&mp3);
    let ch = decoded.channels.map_or(channels, |c| c.num() as usize);
    let mut out = prefix_pcm;
    out.extend_from_slice(decoded.samples.get(discard * ch..).unwrap_or(&[]));
    Some((ch, out))
}

/// Decode a whole SPS stream to PCM (multichannel is downmixed to stereo).
pub fn decode(data: &[u8]) -> Result<Pcm> {
    let (h, header_size) = header(data)?;
    if h.codec != CODEC_EALAYER3_V1 {
        return Err(Error::Codec(h.codec));
    }
    let buf = payload(data, header_size);
    let mut frames = Vec::new();
    let mut off = 0;
    while off < buf.len() {
        let Some((f, size)) = parse_frame(&buf, off) else { break };
        frames.push(f);
        off += size;
    }
    let per_frame = frames.first().map_or(1, |f| f.channels);
    let streams = (h.channels as usize).div_ceil(per_frame).max(1);
    let mut decoded = Vec::new();
    for s in 0..streams {
        let mine: Vec<EaFrame> = frames.iter().skip(s).step_by(streams).cloned().collect();
        if let Some(d) = decode_stream(&buf, &mine) {
            decoded.push(d);
        }
    }
    let Some(frames_out) = decoded.iter().map(|(c, s)| s.len() / c.max(&1)).min() else {
        return Err(Error::Format("no decodable frames".into()));
    };
    let frames_out = frames_out.min(h.samples as usize).max(if h.samples == 0 { frames_out } else { 0 });
    // Gather all channels in stream order, then reduce to at most stereo.
    let all: Vec<Vec<f32>> = decoded
        .iter()
        .flat_map(|(c, s)| (0..*c).map(move |k| (0..frames_out).map(|i| s[i * c + k] as f32).collect()))
        .collect();
    let (out_ch, mixed): (u16, Vec<i16>) = match all.len() {
        0 => return Err(Error::Format("no channels".into())),
        1 => (1, all[0].iter().map(|&v| v as i16).collect()),
        2..=5 => (2, (0..frames_out).flat_map(|i| [all[0][i] as i16, all[1][i] as i16]).collect()),
        _ => {
            // EA 5.1 order: FL FC FR BL BR LFE.
            let mix = |i: usize, side: usize| -> i16 {
                let (front, back) = if side == 0 { (all[0][i], all[3][i]) } else { (all[2][i], all[4][i]) };
                ((front + 0.7 * all[1][i] + 0.6 * back) * 0.6).clamp(-32768.0, 32767.0) as i16
            };
            (2, (0..frames_out).flat_map(|i| [mix(i, 0), mix(i, 1)]).collect())
        }
    };
    Ok(Pcm { channels: out_ch, sample_rate: h.sample_rate, samples: mixed })
}
