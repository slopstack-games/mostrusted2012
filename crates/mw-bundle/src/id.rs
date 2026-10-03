use std::fmt;

/// 64-bit resource identifier.
///
/// Normal IDs are a CRC32 of the lower-cased resource name in the low 32 bits.
/// The high byte selects the ID kind (`EIdType`): 0x00 normal, 0x01
/// "GameChanger" (game database ID + type + index), 0x80 resource list, 0xC0
/// delta bundle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ResourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Normal,
    GameChanger { index: u8, resource_type: u16, db_id: u32 },
    ResourceList,
    DeltaBundle,
    Unknown(u8),
}

impl ResourceId {
    pub const NULL: Self = Self(0);

    /// ID of a named resource: CRC32 of the lower-cased name.
    pub fn from_name(name: &str) -> Self {
        Self(crc32(name.to_ascii_lowercase().as_bytes()) as u64)
    }

    pub fn kind(self) -> IdKind {
        match (self.0 >> 56) as u8 {
            0x00 => IdKind::Normal,
            0x01 => IdKind::GameChanger {
                index: (self.0 >> 48) as u8,
                resource_type: (self.0 >> 32) as u16,
                db_id: self.0 as u32,
            },
            0x80 => IdKind::ResourceList,
            0xC0 => IdKind::DeltaBundle,
            other => IdKind::Unknown(other),
        }
    }

    /// Community tools (Bundle Manager, DGIorio's scripts) write IDs as the
    /// little-endian bytes of the low 32 bits, e.g. `AA_BB_CC_DD`.
    pub fn community_string(self) -> String {
        let b = (self.0 as u32).to_le_bytes();
        format!("{:02X}_{:02X}_{:02X}_{:02X}", b[0], b[1], b[2], b[3])
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches("0x");
        if s.contains('_') {
            let hex: String = s.split('_').collect();
            let v = u32::from_str_radix(&hex, 16).ok()?;
            return Some(Self(v.swap_bytes() as u64));
        }
        u64::from_str_radix(s, 16).ok().map(Self)
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceId({:016x})", self.0)
    }
}

/// Standard CRC-32 (IEEE, reflected, poly 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    };
    !data.iter().fold(!0u32, |c, &b| TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn community_roundtrip() {
        let id = ResourceId(0x0B3F_E48A);
        assert_eq!(id.community_string(), "8A_E4_3F_0B");
        assert_eq!(ResourceId::parse("8A_E4_3F_0B"), Some(id));
        assert_eq!(ResourceId::parse("000000000b3fe48a"), Some(id));
    }
}
