//! `Language` (0x201) string tables and the `VehicleList` (0x105).

use std::collections::HashMap;

use mw_bundle::{Reader, Resource};

use crate::{Error, Result};

/// Localised strings (`UI/LANGUAGE/*.BNDL`, one Language resource each).
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x00 | u32  | language index |
/// | 0x04 | u32  | string count |
/// | 0x08 | ptr  | string IDs (u32[n], sorted) |
/// | 0x0C | ptr  | `{u32 offset, u32 length}`[n]; absolute offsets, UTF-16LE, length in code units incl. NUL |
#[derive(Debug, Clone)]
pub struct Language {
    pub index: u32,
    pub strings: HashMap<u32, String>,
}

impl Language {
    pub fn parse(res: &Resource) -> Result<Self> {
        let data = res.header();
        let r = Reader(data);
        let n = r.u32(0x04)? as usize;
        let (ids, pairs) = (r.u32(0x08)? as usize, r.u32(0x0C)? as usize);
        let mut strings = HashMap::with_capacity(n);
        for i in 0..n {
            let off = r.u32(pairs + i * 8)? as usize;
            let len = r.u32(pairs + i * 8 + 4)? as usize;
            let units: Vec<u16> =
                r.bytes(off, len * 2)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            strings.insert(r.u32(ids + i * 4)?, String::from_utf16_lossy(&units[..end]));
        }
        Ok(Self { index: r.u32(0)?, strings })
    }

    pub fn get(&self, id: u32) -> Option<&str> {
        self.strings.get(&id).map(String::as_str)
    }
}

/// `VehicleList` (`VEHICLES/VEHICLELIST.BNDL`).
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x00 | u32  | unknown (0x3F9) |
/// | 0x04 | u32  | entry count |
/// | 0x0C | ptr  | entries (0x70 bytes each) |
/// | 0x10 | ptr  | trailing table (unknown) |
///
/// Entry (0x70 bytes, partially known):
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x00 | u32  | vehicle ID: `VEHICLES/VEH_<id>_HI.BNDL` / `_LO.BNDL` (when the car has its own bundles) |
/// | 0x04 | u32  | related vehicle ID (sometimes also a bundle ID) |
/// | 0x08 | u32  | Language string ID: full name |
/// | 0x10 | u32  | Language string ID: name (same text in English) |
/// | 0x6C | u16  | model year |
#[derive(Debug, Clone)]
pub struct VehicleList {
    pub entries: Vec<VehicleEntry>,
}

#[derive(Debug, Clone)]
pub struct VehicleEntry {
    pub id: u32,
    pub related_id: u32,
    pub name_string: u32,
    pub short_name_string: u32,
    pub year: u16,
    /// Raw entry bytes for fields not decoded yet.
    pub raw: Vec<u8>,
}

impl VehicleList {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let n = r.u32(0x04)? as usize;
        let base = r.u32(0x0C)? as usize;
        if n > 4096 {
            return Err(Error::Malformed(format!("vehicle count {n}")));
        }
        let entries = (0..n)
            .map(|i| {
                let o = base + i * 0x70;
                Ok(VehicleEntry {
                    id: r.u32(o)?,
                    related_id: r.u32(o + 0x04)?,
                    name_string: r.u32(o + 0x08)?,
                    short_name_string: r.u32(o + 0x10)?,
                    year: r.u16(o + 0x6C)?,
                    raw: r.bytes(o, 0x70)?.to_vec(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self { entries })
    }
}
