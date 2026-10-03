//! EA-XAS ADPCM (CD-XA style filters).
//!
//! NOTICE: algorithm from vgmstream `ea_xas_decoder.c`, Copyright (c) 2008-2025
//! the vgmstream authors, ISC licence: "Permission to use, copy, modify, and
//! distribute this software for any purpose with or without fee is hereby
//! granted, provided that the above copyright notice and this permission
//! notice appear in all copies."

/// CD-XA K0/K1 filter pairs (only four exist).
const COEFS: [[f32; 2]; 4] = [[0.0, 0.0], [0.9375, 0.0], [1.796875, -0.8125], [1.53125, -0.859375]];

/// Bytes per XAS v0 frame (mono): 4-byte header + 15 bytes of nibbles.
pub const V0_FRAME_BYTES: usize = 0x13;
/// Samples per XAS v0 frame: two header samples + 30 nibbles.
pub const V0_FRAME_SAMPLES: usize = 32;

fn clamp16(v: f32) -> i16 {
    v.clamp(-32768.0, 32767.0) as i16
}

/// Decode one 4-byte frame/group header plus its nibbles into `out`.
fn decode_group(header: u32, nibbles: impl Iterator<Item = u8>, out: &mut Vec<i16>) {
    let [c1, c2] = COEFS.get((header & 0x0F) as usize).copied().unwrap_or([0.0, 0.0]);
    let mut hist2 = (header & 0xFFF0) as u16 as i16;
    let mut hist1 = ((header >> 16) & 0xFFF0) as u16 as i16;
    let shift = (header >> 16) & 0x0F;
    out.push(hist2);
    out.push(hist1);
    for byte in nibbles {
        for nibble in [byte >> 4, byte & 0x0F] {
            // 16-bit sign extend, then scale.
            let s = (((nibble as u16) << 12) as i16 as i32) >> shift;
            let v = clamp16(s as f32 + hist1 as f32 * c1 + hist2 as f32 * c2);
            out.push(v);
            hist2 = hist1;
            hist1 = v;
        }
    }
}

/// Decode XAS v0 (mono, 0x13-byte frames of 32 samples), as used by Ginsu.
pub fn decode_v0(data: &[u8], max_samples: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(max_samples.min(data.len() / V0_FRAME_BYTES * V0_FRAME_SAMPLES));
    for frame in data.chunks_exact(V0_FRAME_BYTES) {
        if out.len() >= max_samples {
            break;
        }
        let header = u32::from_le_bytes(frame[0..4].try_into().unwrap());
        decode_group(header, frame[4..].iter().copied(), &mut out);
    }
    out.truncate(max_samples);
    out
}

/// Decode one XAS v1 block (0x4C bytes, 128 samples) of one channel.
pub fn decode_v1_block(block: &[u8], out: &mut Vec<i16>) {
    if block.len() < 0x4C {
        return;
    }
    for group in 0..4 {
        let header = u32::from_le_bytes(block[group * 4..group * 4 + 4].try_into().unwrap());
        let nibbles = (0..15).map(|row| block[16 + row * 4 + group]);
        decode_group(header, nibbles, out);
    }
}
