//! Reader for Criterion's BundleV2 container (`bnd2`, version 5, PC).
//!
//! Every asset in Most Wanted (2012) lives in a `.BNDL` file. A bundle holds a
//! table of resources; each resource is split across up to four memory chunks
//! (on PC: chunk 0 = main memory header, chunk 1 = GPU/body data such as pixels
//! or vertex buffers, chunks 2/3 rarely used). Chunks may be individually zlib
//! compressed. Resources reference each other through an import table that is
//! appended to chunk 0.
//!
//! Layout reference: <https://burnout.wiki/wiki/Bundle_2/Need_for_Speed_Most_Wanted>

/// Minimal bitflags without the dependency.
macro_rules! bitflags_lite {
    ($(#[$m:meta])* pub struct $name:ident: $t:ty { $(const $f:ident = $v:expr;)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub $t);
        impl $name {
            $(pub const $f: Self = Self($v);)*
            pub const fn from_bits(bits: $t) -> Self { Self(bits) }
            pub const fn bits(self) -> $t { self.0 }
            pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut first = true;
                write!(f, "{}(", stringify!($name))?;
                $(if self.contains(Self::$f) {
                    if !first { write!(f, " | ")?; }
                    first = false;
                    write!(f, "{}", stringify!($f))?;
                })*
                let known = 0 $(| $v)*;
                if self.0 & !known != 0 {
                    if !first { write!(f, " | ")?; }
                    write!(f, "{:#x}", self.0 & !known)?;
                }
                write!(f, ")")
            }
        }
    };
}

mod id;
mod types;

use std::io::{Read, Seek};
use std::path::Path;

pub use id::{ResourceId, crc32};
pub use types::ResourceType;

pub const MAGIC: &[u8; 4] = b"bnd2";
pub const SUPPORTED_VERSION: u16 = 5;
pub const NUM_CHUNKS: usize = 4;
const HEADER_SIZE: usize = 0x70;
const ENTRY_SIZE: usize = 0x48;
const IMPORT_SIZE: usize = 0x10;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a bnd2 bundle")]
    BadMagic,
    #[error("unsupported bundle version {0} (expected 5)")]
    UnsupportedVersion(u16),
    #[error("unsupported platform {0} (expected 1 = PC)")]
    UnsupportedPlatform(u16),
    #[error("bundle truncated: needed {needed:#x} bytes at {offset:#x}, have {len:#x}")]
    Truncated { offset: usize, needed: usize, len: usize },
    #[error("resource {id} chunk {chunk}: zlib error: {source}")]
    Decompress { id: ResourceId, chunk: usize, source: std::io::Error },
    #[error("resource {id} chunk {chunk}: decompressed {got:#x} bytes, expected {expected:#x}")]
    SizeMismatch { id: ResourceId, chunk: usize, got: usize, expected: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

bitflags_lite! {
    /// `eBundleFlags`
    pub struct BundleFlags: u32 {
        const ZLIB_COMPRESSION = 0x1;
        const CONTAINS_DEBUG_DATA = 0x2;
        const MULTISTREAM_BUNDLE = 0x8;
        const DELTA_BUNDLE = 0x10;
        const CONTAINS_DEFAULT_RESOURCE = 0x20;
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: u16,
    pub platform: u16,
    pub debug_data_offset: u32,
    pub resource_count: u32,
    pub resource_entries_offset: u32,
    pub resource_data_offsets: [u32; NUM_CHUNKS],
    pub flags: BundleFlags,
    pub default_resource_id: ResourceId,
    pub default_stream_index: i32,
    /// Stream names for multistream bundles (e.g. `GRAPHICS`, `SOUND`, `PHYSICS`).
    pub stream_names: [String; 4],
}

/// One resource table entry (`CgsResource::BundleV2::ResourceEntry`).
#[derive(Debug, Clone)]
pub struct ResourceEntry {
    pub id: ResourceId,
    /// Packed size (low 28 bits) and log2 alignment (high 4 bits) per chunk.
    pub uncompressed_size_and_alignment: [u32; NUM_CHUNKS],
    pub size_on_disk: [u32; NUM_CHUNKS],
    pub disk_offset: [u32; NUM_CHUNKS],
    pub imports_offset: u32,
    pub type_id: u32,
    pub import_count: u16,
    pub flags: u8,
    pub stream_index: u8,
}

impl ResourceEntry {
    pub fn resource_type(&self) -> ResourceType {
        ResourceType::from_id(self.type_id)
    }

    pub fn uncompressed_size(&self, chunk: usize) -> usize {
        (self.uncompressed_size_and_alignment[chunk] & 0x0FFF_FFFF) as usize
    }

    pub fn alignment(&self, chunk: usize) -> usize {
        1 << (self.uncompressed_size_and_alignment[chunk] >> 28)
    }
}

/// A resolved import: at `offset` inside chunk 0 the runtime patches in a
/// reference to resource `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Import {
    pub id: ResourceId,
    pub offset: u32,
    pub kind: ImportKind,
}

/// The high bit of an import's offset word selects how it is patched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// A 32-bit pointer slot, zero on disk.
    Pointer,
    /// A resource handle; the 64-bit ID is usually also stored at the offset.
    /// Used throughout GenesysObject, ZoneHeader and others.
    Handle,
}

/// Fully decompressed resource.
#[derive(Debug, Clone)]
pub struct Resource {
    pub id: ResourceId,
    pub type_id: u32,
    pub chunks: [Vec<u8>; NUM_CHUNKS],
    pub imports: Vec<Import>,
}

impl Resource {
    pub fn resource_type(&self) -> ResourceType {
        ResourceType::from_id(self.type_id)
    }

    /// Chunk 0: the main-memory header struct.
    pub fn header(&self) -> &[u8] {
        &self.chunks[0]
    }

    /// Chunk 1: the "body" (texture pixels, vertex/index buffers...).
    pub fn body(&self) -> &[u8] {
        &self.chunks[1]
    }

    /// Resource ID imported at a given pointer slot in chunk 0, if any.
    pub fn import_at(&self, offset: u32) -> Option<ResourceId> {
        self.imports.iter().find(|i| i.offset == offset).map(|i| i.id)
    }
}

/// An in-memory bundle. The whole file is kept in memory; resources are
/// decompressed on demand.
pub struct Bundle {
    data: Vec<u8>,
    pub header: Header,
    pub entries: Vec<ResourceEntry>,
    names: Vec<(ResourceId, String, String)>,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle").field("header", &self.header).field("entries", &self.entries.len()).finish()
    }
}

impl Bundle {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(std::fs::read(path)?)
    }

    /// Parse the header and resource table from the start of a bundle file.
    pub fn parse_toc(data: &[u8]) -> Result<(Header, Vec<ResourceEntry>)> {
        let r = Reader(data);
        if r.bytes(0, 4)? != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = r.u16(4)?;
        if version != SUPPORTED_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let platform = r.u16(6)?;
        if platform != 1 {
            return Err(Error::UnsupportedPlatform(platform));
        }
        let stream_names = std::array::from_fn(|i| {
            let raw = r.bytes(0x34 + i * 15, 15).unwrap_or(&[]);
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        });
        let header = Header {
            version,
            platform,
            debug_data_offset: r.u32(0x08)?,
            resource_count: r.u32(0x0C)?,
            resource_entries_offset: r.u32(0x10)?,
            resource_data_offsets: r.u32_array(0x14)?,
            flags: BundleFlags::from_bits(r.u32(0x24)?),
            default_resource_id: ResourceId(r.u64(0x28)?),
            default_stream_index: r.u32(0x30)? as i32,
            stream_names,
        };
        r.bytes(0, HEADER_SIZE)?;
        // The whole table must be in `data` before reserving room for it: a corrupt count would otherwise abort
        // on allocation instead of returning an error.
        let table_len = (header.resource_count as usize).saturating_mul(ENTRY_SIZE);
        r.bytes(header.resource_entries_offset as usize, table_len)?;

        let mut entries = Vec::with_capacity(header.resource_count as usize);
        for i in 0..header.resource_count as usize {
            let o = header.resource_entries_offset as usize + i * ENTRY_SIZE;
            entries.push(ResourceEntry {
                id: ResourceId(r.u64(o)?),
                uncompressed_size_and_alignment: r.u32_array(o + 0x08)?,
                size_on_disk: r.u32_array(o + 0x18)?,
                disk_offset: r.u32_array(o + 0x28)?,
                imports_offset: r.u32(o + 0x38)?,
                type_id: r.u32(o + 0x3C)?,
                import_count: r.u16(o + 0x40)?,
                flags: r.u8(o + 0x42)?,
                stream_index: r.u8(o + 0x43)?,
            });
        }
        Ok((header, entries))
    }

    /// Read only the header and resource table, without loading resource data.
    pub fn read_toc(path: impl AsRef<Path>) -> Result<(Header, Vec<ResourceEntry>)> {
        let mut f = std::fs::File::open(path)?;
        let file_len = f.metadata()?.len();
        Self::read_toc_with(|offset, len| {
            // Past the end of the file (a corrupt table size): fail before allocating `len` bytes.
            if offset.saturating_add(len as u64) > file_len {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            f.seek(std::io::SeekFrom::Start(offset))?;
            let mut buf = vec![0; len];
            f.read_exact(&mut buf)?;
            Ok(buf)
        })
    }

    /// [`Bundle::read_toc`] over any random-access reader: `read(offset, len)` returns those bytes of the file.
    pub fn read_toc_with(
        mut read: impl FnMut(u64, usize) -> std::io::Result<Vec<u8>>,
    ) -> Result<(Header, Vec<ResourceEntry>)> {
        let mut head = read(0, HEADER_SIZE)?;
        let r = Reader(&head);
        let (entries_at, count) = (r.u32(0x10)? as usize, r.u32(0x0C)? as usize);
        let end = count.checked_mul(ENTRY_SIZE).and_then(|n| n.checked_add(entries_at)).ok_or(Error::Truncated {
            offset: HEADER_SIZE,
            needed: usize::MAX,
            len: head.len(),
        })?;
        if end > HEADER_SIZE {
            head.extend(read(HEADER_SIZE as u64, end - HEADER_SIZE)?);
        }
        Self::parse_toc(&head)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let (header, entries) = Self::parse_toc(&data)?;
        let names = if header.flags.contains(BundleFlags::CONTAINS_DEBUG_DATA) {
            parse_string_table(&data[(header.debug_data_offset as usize).min(data.len())..])
        } else {
            Vec::new()
        };

        Ok(Self { data, header, entries, names })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn find(&self, id: ResourceId) -> Option<&ResourceEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn entries_of_type(&self, ty: ResourceType) -> impl Iterator<Item = &ResourceEntry> {
        self.entries.iter().filter(move |e| e.resource_type() == ty)
    }

    /// Debug name from the embedded `ResourceStringTable`, if present.
    pub fn debug_name(&self, id: ResourceId) -> Option<&str> {
        self.names.iter().find(|(i, _, _)| *i == id).map(|(_, _, n)| n.as_str())
    }

    /// Raw (possibly compressed) bytes of one chunk as stored on disk.
    pub fn raw_chunk(&self, entry: &ResourceEntry, chunk: usize) -> Result<&[u8]> {
        let stored = self.stored_size(entry, chunk);
        if stored == 0 {
            return Ok(&[]);
        }
        let off = self.header.resource_data_offsets[chunk] as usize + entry.disk_offset[chunk] as usize;
        Reader(&self.data).bytes(off, stored)
    }

    fn stored_size(&self, entry: &ResourceEntry, chunk: usize) -> usize {
        if self.header.flags.contains(BundleFlags::ZLIB_COMPRESSION) {
            entry.size_on_disk[chunk] as usize
        } else {
            entry.uncompressed_size(chunk)
        }
    }

    /// Decompress a single chunk.
    pub fn chunk(&self, entry: &ResourceEntry, chunk: usize) -> Result<Vec<u8>> {
        let raw = self.raw_chunk(entry, chunk)?;
        let expected = entry.uncompressed_size(chunk);
        if !self.header.flags.contains(BundleFlags::ZLIB_COMPRESSION) || raw.is_empty() {
            return Ok(raw.to_vec());
        }
        let mut out = Vec::with_capacity(expected);
        // One byte past `expected` is enough to report a size mismatch; a corrupt stream can't inflate without bound.
        flate2::read::ZlibDecoder::new(raw)
            .take(expected as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|source| Error::Decompress { id: entry.id, chunk, source })?;
        if out.len() != expected {
            return Err(Error::SizeMismatch { id: entry.id, chunk, got: out.len(), expected });
        }
        Ok(out)
    }

    /// Decompress all chunks of a resource and decode its import table.
    pub fn load(&self, entry: &ResourceEntry) -> Result<Resource> {
        let mut chunks: [Vec<u8>; NUM_CHUNKS] = Default::default();
        for (i, c) in chunks.iter_mut().enumerate() {
            *c = self.chunk(entry, i)?;
        }
        let r = Reader(&chunks[0]);
        let mut imports = Vec::with_capacity(entry.import_count as usize);
        for i in 0..entry.import_count as usize {
            let o = entry.imports_offset as usize + i * IMPORT_SIZE;
            let word = r.u32(o + 8)?;
            let kind = if word & 0x8000_0000 != 0 { ImportKind::Handle } else { ImportKind::Pointer };
            imports.push(Import { id: ResourceId(r.u64(o)?), offset: word & 0x7FFF_FFFF, kind });
        }
        Ok(Resource { id: entry.id, type_id: entry.type_id, chunks, imports })
    }

    pub fn load_id(&self, id: ResourceId) -> Option<Result<Resource>> {
        self.find(id).map(|e| self.load(e))
    }
}

/// Parse `<ResourceStringTable><Resource id=".." type=".." name=".."/>...`.
/// Hand-rolled because the format is trivial and fixed.
fn parse_string_table(data: &[u8]) -> Vec<(ResourceId, String, String)> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let text = String::from_utf8_lossy(&data[..end]);
    let attr = |line: &str, key: &str| -> Option<String> {
        let start = line.find(&format!("{key}=\""))? + key.len() + 2;
        let len = line[start..].find('"')?;
        Some(xml_unescape(&line[start..start + len]))
    };
    text.split("<Resource ")
        .skip(1)
        .filter_map(|line| {
            let id = u64::from_str_radix(&attr(line, "id")?, 16).ok()?;
            Some((ResourceId(id), attr(line, "type").unwrap_or_default(), attr(line, "name")?))
        })
        .collect()
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Bounds-checked little-endian reader over a byte slice.
#[derive(Clone, Copy)]
pub struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
    pub fn bytes(&self, offset: usize, len: usize) -> Result<&'a [u8]> {
        self.0.get(offset..offset.saturating_add(len)).ok_or(Error::Truncated {
            offset,
            needed: len,
            len: self.0.len(),
        })
    }
    pub fn u8(&self, o: usize) -> Result<u8> {
        Ok(self.bytes(o, 1)?[0])
    }
    pub fn u16(&self, o: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(o, 2)?.try_into().unwrap()))
    }
    pub fn i16(&self, o: usize) -> Result<i16> {
        Ok(self.u16(o)? as i16)
    }
    pub fn u32(&self, o: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(o, 4)?.try_into().unwrap()))
    }
    pub fn i32(&self, o: usize) -> Result<i32> {
        Ok(self.u32(o)? as i32)
    }
    pub fn u64(&self, o: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(o, 8)?.try_into().unwrap()))
    }
    pub fn f32(&self, o: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32(o)?))
    }
    pub fn u32_array<const N: usize>(&self, o: usize) -> Result<[u32; N]> {
        let mut out = [0; N];
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.u32(o + i * 4)?;
        }
        Ok(out)
    }
    pub fn f32_array<const N: usize>(&self, o: usize) -> Result<[f32; N]> {
        Ok(self.u32_array::<N>(o)?.map(f32::from_bits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-resource uncompressed PC bundle: header, entry table at 0x70, chunk 0 at 0xC0.
    fn bundle(chunk0: &[u8], import_count: u16) -> Vec<u8> {
        let mut d = vec![0u8; 0xC0];
        d[0..4].copy_from_slice(MAGIC);
        d[4..6].copy_from_slice(&5u16.to_le_bytes());
        d[6..8].copy_from_slice(&1u16.to_le_bytes());
        d[0x0C..0x10].copy_from_slice(&1u32.to_le_bytes());
        d[0x10..0x14].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        d[0x14..0x18].copy_from_slice(&0xC0u32.to_le_bytes());
        let e = HEADER_SIZE;
        d[e..e + 8].copy_from_slice(&7u64.to_le_bytes());
        d[e + 8..e + 0xC].copy_from_slice(&(chunk0.len() as u32).to_le_bytes());
        d[e + 0x40..e + 0x42].copy_from_slice(&import_count.to_le_bytes());
        d.extend_from_slice(chunk0);
        d
    }

    #[test]
    fn synthetic_bundle_loads() {
        let b = Bundle::from_bytes(bundle(&[1, 2, 3, 4], 0)).unwrap();
        assert_eq!(b.load(&b.entries[0]).unwrap().chunks[0], [1, 2, 3, 4]);
    }

    #[test]
    fn huge_resource_count_is_an_error_not_an_abort() {
        let mut d = bundle(&[], 0);
        d[0x0C..0x10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Bundle::parse_toc(&d), Err(Error::Truncated { .. })));
        let res = Bundle::read_toc_with(|o, len| {
            let o = o as usize;
            d.get(o..o.saturating_add(len)).map(<[u8]>::to_vec).ok_or(std::io::ErrorKind::UnexpectedEof.into())
        });
        assert!(res.is_err());
    }

    #[test]
    fn read_toc_rejects_table_past_file_end() {
        let mut d = bundle(&[], 0);
        d[0x0C..0x10].copy_from_slice(&u32::MAX.to_le_bytes());
        let path = std::env::temp_dir().join(format!("mw-bundle-toc-{}.bndl", std::process::id()));
        std::fs::write(&path, &d).unwrap();
        let res = Bundle::read_toc(&path);
        std::fs::remove_file(&path).ok();
        assert!(res.is_err());
    }

    #[test]
    fn zlib_chunk_larger_than_declared_is_a_size_mismatch() {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&[0u8; 1 << 16]).unwrap();
        let packed = z.finish().unwrap();
        let mut d = bundle(&packed, 0);
        d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes()); // ZLIB_COMPRESSION
        let e = HEADER_SIZE;
        d[e + 8..e + 0xC].copy_from_slice(&16u32.to_le_bytes()); // declared: 16 bytes
        d[e + 0x18..e + 0x1C].copy_from_slice(&(packed.len() as u32).to_le_bytes());
        let b = Bundle::from_bytes(d).unwrap();
        assert!(matches!(b.chunk(&b.entries[0], 0), Err(Error::SizeMismatch { got: 17, expected: 16, .. })));
    }
}
