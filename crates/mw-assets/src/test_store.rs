//! A synthetic [`AssetStore`] for unit tests: an in-memory install with hand-built bundles (no game data).

use std::sync::Arc;

use mw_bundle::ResourceId;
use mw_gamedir::{GameDir, MemSource};

use crate::AssetStore;

/// One resource of a synthetic bundle: id, type id, chunk 0 (header) and chunk 1 (body).
pub struct Res {
    pub id: ResourceId,
    pub type_id: u32,
    pub header: Vec<u8>,
    pub body: Vec<u8>,
}

/// An uncompressed PC bnd2 v5 bundle holding `resources` (entry table at 0x70, chunk 0 data, then chunk 1 data).
pub fn bundle(resources: &[Res]) -> Vec<u8> {
    const HEADER: usize = 0x70;
    const ENTRY: usize = 0x48;
    let table_end = HEADER + resources.len() * ENTRY;
    let chunk0_at = table_end.next_multiple_of(16);
    let chunk0_len: usize = resources.iter().map(|r| r.header.len().next_multiple_of(16)).sum();
    let chunk1_at = chunk0_at + chunk0_len;
    let mut d = vec![0u8; chunk1_at];
    d[0..4].copy_from_slice(b"bnd2");
    d[4..6].copy_from_slice(&5u16.to_le_bytes());
    d[6..8].copy_from_slice(&1u16.to_le_bytes());
    d[0x0C..0x10].copy_from_slice(&(resources.len() as u32).to_le_bytes());
    d[0x10..0x14].copy_from_slice(&(HEADER as u32).to_le_bytes());
    d[0x14..0x18].copy_from_slice(&(chunk0_at as u32).to_le_bytes());
    d[0x18..0x1C].copy_from_slice(&(chunk1_at as u32).to_le_bytes());
    let (mut o0, mut o1) = (0usize, 0usize);
    let mut body = Vec::new();
    for (i, r) in resources.iter().enumerate() {
        let e = HEADER + i * ENTRY;
        d[e..e + 8].copy_from_slice(&r.id.0.to_le_bytes());
        d[e + 0x08..e + 0x0C].copy_from_slice(&(r.header.len() as u32).to_le_bytes());
        d[e + 0x0C..e + 0x10].copy_from_slice(&(r.body.len() as u32).to_le_bytes());
        d[e + 0x28..e + 0x2C].copy_from_slice(&(o0 as u32).to_le_bytes());
        d[e + 0x2C..e + 0x30].copy_from_slice(&(o1 as u32).to_le_bytes());
        d[e + 0x3C..e + 0x40].copy_from_slice(&r.type_id.to_le_bytes());
        d[chunk0_at + o0..chunk0_at + o0 + r.header.len()].copy_from_slice(&r.header);
        o0 += r.header.len().next_multiple_of(16);
        body.extend_from_slice(&r.body);
        body.resize(body.len().next_multiple_of(16), 0);
        o1 = body.len();
    }
    d.extend_from_slice(&body);
    d
}

/// A 2D `R8G8B8A8_UNORM` texture resource with one mip.
pub fn texture(id: ResourceId, width: u16, height: u16, rgba: Vec<u8>) -> Res {
    let mut h = vec![0u8; 0x30];
    h[0x08..0x0C].copy_from_slice(&7u32.to_le_bytes());
    h[0x1C..0x20].copy_from_slice(&28u32.to_le_bytes());
    h[0x24..0x26].copy_from_slice(&width.to_le_bytes());
    h[0x26..0x28].copy_from_slice(&height.to_le_bytes());
    h[0x2D] = 1;
    Res { id, type_id: mw_bundle::ResourceType::Texture.id(), header: h, body: rgba }
}

/// A store over an install whose required files are empty bundles, plus `extra` bundles (path, resources).
pub fn store(extra: &[(&str, Vec<Res>)]) -> AssetStore {
    let mut src = MemSource::new();
    src.insert(mw_gamedir::EXECUTABLE, Vec::new());
    for name in ["GLOBALTEXTUREDICTIONARY.BNDL", "SHADERS.BNDL", "WORLDLIST.BNDL"] {
        src.insert(name, bundle(&[]));
    }
    for (path, res) in extra {
        src.insert(path, bundle(res));
    }
    AssetStore::new(GameDir::from_source(Arc::new(src)).expect("synthetic install")).expect("synthetic store")
}
