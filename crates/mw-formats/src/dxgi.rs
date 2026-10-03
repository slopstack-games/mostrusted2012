//! The subset of `DXGI_FORMAT` that Most Wanted uses for textures and vertex
//! attributes. The game stores raw D3D11 enum values on disk.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DxgiFormat {
    R32G32B32A32Float,
    R32G32B32A32Uint,
    R32G32B32A32Sint,
    R32G32B32Float,
    R32G32B32Uint,
    R32G32B32Sint,
    R16G16B16A16Float,
    R16G16B16A16Unorm,
    R16G16B16A16Uint,
    R16G16B16A16Snorm,
    R16G16B16A16Sint,
    R32G32Float,
    R32G32Uint,
    R32G32Sint,
    R8G8B8A8Unorm,
    R8G8B8A8UnormSrgb,
    R8G8B8A8Uint,
    R8G8B8A8Snorm,
    R8G8B8A8Sint,
    R16G16Float,
    R16G16Unorm,
    R16G16Uint,
    R16G16Snorm,
    R16G16Sint,
    R32Float,
    R32Uint,
    R32Sint,
    R8G8Unorm,
    R8G8Uint,
    R8G8Snorm,
    R8G8Sint,
    R16Float,
    R16Uint,
    R16Sint,
    R8Unorm,
    R8Uint,
    R8Snorm,
    R8Sint,
    Bc1Unorm,
    Bc2Unorm,
    Bc3Unorm,
    B8G8R8A8Unorm,
    Unknown(u32),
}

impl DxgiFormat {
    pub fn from_raw(v: u32) -> Self {
        use DxgiFormat::*;
        match v {
            2 => R32G32B32A32Float,
            3 => R32G32B32A32Uint,
            4 => R32G32B32A32Sint,
            6 => R32G32B32Float,
            7 => R32G32B32Uint,
            8 => R32G32B32Sint,
            10 => R16G16B16A16Float,
            11 => R16G16B16A16Unorm,
            12 => R16G16B16A16Uint,
            13 => R16G16B16A16Snorm,
            14 => R16G16B16A16Sint,
            16 => R32G32Float,
            17 => R32G32Uint,
            18 => R32G32Sint,
            28 => R8G8B8A8Unorm,
            29 => R8G8B8A8UnormSrgb,
            30 => R8G8B8A8Uint,
            31 => R8G8B8A8Snorm,
            32 => R8G8B8A8Sint,
            34 => R16G16Float,
            35 => R16G16Unorm,
            36 => R16G16Uint,
            37 => R16G16Snorm,
            38 => R16G16Sint,
            41 => R32Float,
            42 => R32Uint,
            43 => R32Sint,
            49 => R8G8Unorm,
            50 => R8G8Uint,
            51 => R8G8Snorm,
            52 => R8G8Sint,
            54 => R16Float,
            57 => R16Uint,
            59 => R16Sint,
            61 => R8Unorm,
            62 => R8Uint,
            63 => R8Snorm,
            64 => R8Sint,
            71 => Bc1Unorm,
            74 => Bc2Unorm,
            77 => Bc3Unorm,
            87 => B8G8R8A8Unorm,
            other => Unknown(other),
        }
    }

    pub fn raw(self) -> u32 {
        use DxgiFormat::*;
        match self {
            R32G32B32A32Float => 2,
            R32G32B32A32Uint => 3,
            R32G32B32A32Sint => 4,
            R32G32B32Float => 6,
            R32G32B32Uint => 7,
            R32G32B32Sint => 8,
            R16G16B16A16Float => 10,
            R16G16B16A16Unorm => 11,
            R16G16B16A16Uint => 12,
            R16G16B16A16Snorm => 13,
            R16G16B16A16Sint => 14,
            R32G32Float => 16,
            R32G32Uint => 17,
            R32G32Sint => 18,
            R8G8B8A8Unorm => 28,
            R8G8B8A8UnormSrgb => 29,
            R8G8B8A8Uint => 30,
            R8G8B8A8Snorm => 31,
            R8G8B8A8Sint => 32,
            R16G16Float => 34,
            R16G16Unorm => 35,
            R16G16Uint => 36,
            R16G16Snorm => 37,
            R16G16Sint => 38,
            R32Float => 41,
            R32Uint => 42,
            R32Sint => 43,
            R8G8Unorm => 49,
            R8G8Uint => 50,
            R8G8Snorm => 51,
            R8G8Sint => 52,
            R16Float => 54,
            R16Uint => 57,
            R16Sint => 59,
            R8Unorm => 61,
            R8Uint => 62,
            R8Snorm => 63,
            R8Sint => 64,
            Bc1Unorm => 71,
            Bc2Unorm => 74,
            Bc3Unorm => 77,
            B8G8R8A8Unorm => 87,
            Unknown(v) => v,
        }
    }

    /// For block-compressed formats: bytes per 4x4 block.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Bc1Unorm => Some(8),
            Self::Bc2Unorm | Self::Bc3Unorm => Some(16),
            _ => None,
        }
    }

    /// Size in bytes of one element (vertex attribute or uncompressed texel).
    pub fn element_size(self) -> Option<usize> {
        use DxgiFormat::*;
        Some(match self {
            R32G32B32A32Float | R32G32B32A32Uint | R32G32B32A32Sint => 16,
            R32G32B32Float | R32G32B32Uint | R32G32B32Sint => 12,
            R16G16B16A16Float | R16G16B16A16Unorm | R16G16B16A16Uint | R16G16B16A16Snorm | R16G16B16A16Sint
            | R32G32Float | R32G32Uint | R32G32Sint => 8,
            R8G8B8A8Unorm | R8G8B8A8UnormSrgb | R8G8B8A8Uint | R8G8B8A8Snorm | R8G8B8A8Sint | R16G16Float
            | R16G16Unorm | R16G16Uint | R16G16Snorm | R16G16Sint | R32Float | R32Uint | R32Sint | B8G8R8A8Unorm => 4,
            R8G8Unorm | R8G8Uint | R8G8Snorm | R8G8Sint | R16Float | R16Uint | R16Sint => 2,
            R8Unorm | R8Uint | R8Snorm | R8Sint => 1,
            Bc1Unorm | Bc2Unorm | Bc3Unorm | Unknown(_) => return None,
        })
    }

    /// Number of components.
    pub fn components(self) -> usize {
        use DxgiFormat::*;
        match self {
            R32G32B32A32Float | R32G32B32A32Uint | R32G32B32A32Sint | R16G16B16A16Float | R16G16B16A16Unorm
            | R16G16B16A16Uint | R16G16B16A16Snorm | R16G16B16A16Sint | R8G8B8A8Unorm | R8G8B8A8UnormSrgb
            | R8G8B8A8Uint | R8G8B8A8Snorm | R8G8B8A8Sint | B8G8R8A8Unorm | Bc1Unorm | Bc2Unorm | Bc3Unorm => 4,
            R32G32B32Float | R32G32B32Uint | R32G32B32Sint => 3,
            R32G32Float | R32G32Uint | R32G32Sint | R16G16Float | R16G16Unorm | R16G16Uint | R16G16Snorm
            | R16G16Sint | R8G8Unorm | R8G8Uint | R8G8Snorm | R8G8Sint => 2,
            R32Float | R32Uint | R32Sint | R16Float | R16Uint | R16Sint | R8Unorm | R8Uint | R8Snorm | R8Sint => 1,
            Unknown(_) => 0,
        }
    }

    /// Decode one element to up to four floats. Integer formats are returned
    /// as their numeric value, normalized formats are mapped to [0,1]/[-1,1].
    pub fn decode(self, b: &[u8]) -> Option<[f32; 4]> {
        use DxgiFormat::*;
        let size = self.element_size()?;
        let b = b.get(..size)?;
        let mut out = [0.0, 0.0, 0.0, 1.0];
        let n = self.components();
        let u16_at = |i: usize| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o = match self {
                R32G32B32A32Float | R32G32B32Float | R32G32Float | R32Float => f32::from_bits(u32_at(i)),
                R32G32B32A32Uint | R32G32B32Uint | R32G32Uint | R32Uint => u32_at(i) as f32,
                R32G32B32A32Sint | R32G32B32Sint | R32G32Sint | R32Sint => u32_at(i) as i32 as f32,
                R16G16B16A16Float | R16G16Float | R16Float => f16_to_f32(u16_at(i)),
                R16G16B16A16Unorm | R16G16Unorm => u16_at(i) as f32 / 65535.0,
                R16G16B16A16Uint | R16G16Uint | R16Uint => u16_at(i) as f32,
                R16G16B16A16Snorm | R16G16Snorm => (u16_at(i) as i16 as f32 / 32767.0).max(-1.0),
                R16G16B16A16Sint | R16G16Sint | R16Sint => u16_at(i) as i16 as f32,
                R8G8B8A8Unorm | R8G8B8A8UnormSrgb | R8G8Unorm | R8Unorm => b[i] as f32 / 255.0,
                B8G8R8A8Unorm => b[[2, 1, 0, 3][i]] as f32 / 255.0,
                R8G8B8A8Uint | R8G8Uint | R8Uint => b[i] as f32,
                R8G8B8A8Snorm | R8G8Snorm | R8Snorm => (b[i] as i8 as f32 / 127.0).max(-1.0),
                R8G8B8A8Sint | R8G8Sint | R8Sint => b[i] as i8 as f32,
                Bc1Unorm | Bc2Unorm | Bc3Unorm | Unknown(_) => return None,
            };
        }
        Some(out)
    }
}

/// IEEE 754 half to single precision.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    let bits = match (exp, mant) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: renormalize.
            let shift = m.leading_zeros() - 21;
            let m = (m << shift) & 0x3FF;
            sign | ((127 - 15 + 1 - shift) << 23) | (m << 13)
        }
        (0x1F, m) => sign | 0x7F80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x0001), 2.0f32.powi(-24));
        assert_eq!(f16_to_f32(0x7BFF), 65504.0);
    }
}
