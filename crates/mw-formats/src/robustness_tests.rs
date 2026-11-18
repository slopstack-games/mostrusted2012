//! Malformed-input regressions: parsers return errors (or clamp) instead of panicking or aborting on
//! allocation. Synthetic bytes only.

use mw_bundle::{Resource, ResourceId};

use crate::*;

fn res(c0: Vec<u8>) -> Resource {
    Resource { id: ResourceId(1), type_id: 0, chunks: [c0, vec![], vec![], vec![]], imports: Vec::new() }
}

fn put(d: &mut [u8], o: usize, v: &[u8]) {
    d[o..o + v.len()].copy_from_slice(v);
}

/// A u32 element count of 0xFFFFFFFF used to reserve its whole capacity up front and abort the process.
#[test]
fn huge_counts_are_errors_not_allocation_aborts() {
    let with = |len: usize, at: usize| {
        let mut d = vec![0u8; len];
        put(&mut d, at, &u32::MAX.to_le_bytes());
        res(d)
    };
    assert!(VertexDescriptor::parse(&with(0x10, 0x0C)).is_err());
    assert!(Language::parse(&with(0x10, 0x04)).is_err());
    assert!(PolygonSoupList::parse(&with(0x2C, 0x28)).is_err());
    assert!(InstanceList::parse(&with(0x0C, 0x04)).is_err());
    let mut font = with(0x28, 0x1C);
    put(&mut font.chunks[0], 0, &10u32.to_le_bytes());
    put(&mut font.chunks[0], 8, &1.0f32.to_le_bytes());
    assert!(Font::parse(&font, [256, 256]).is_err());
}

/// A wheel with 40 parts: the spin mask has 32 bits, so parts 32+ don't spin (the shift used to overflow and,
/// in release, wrap to bit `i % 32`). Part / model tables near 4 GiB don't wrap to offset 0.
#[test]
fn wheel_parts_past_32_and_tables_at_the_top_of_u32() {
    let mut d = vec![0u8; 0x40 + 0x90];
    put(&mut d, 0x00, &0xFFFF_FFF8u32.to_le_bytes()); // models
    put(&mut d, 0x0C, &0x40u32.to_le_bytes()); // wheels
    put(&mut d, 0x10, &8u16.to_le_bytes()); // parts
    d[0x13] = 1;
    put(&mut d, 0x40 + 0x78, &0xFFFF_FFF0u32.to_le_bytes());
    put(&mut d, 0x40 + 0x7C, &1u32.to_le_bytes());
    put(&mut d, 0x40 + 0x80, &40u16.to_le_bytes());
    let mut r = res(d);
    r.imports.push(mw_bundle::Import { id: ResourceId(5), offset: 0, kind: mw_bundle::ImportKind::Pointer });
    let spec = VehicleGraphicsSpec::parse(&r).unwrap();
    let parts = &spec.wheels[0].parts;
    assert_eq!(parts.len(), 40);
    assert!(parts[0].spins && !parts[32].spins);
    assert!(parts.iter().all(|p| p.model.is_none()));
    assert!(spec.parts.iter().all(Option::is_none));
}

/// Sampler / renderable tables at the top of the u32 range used to overflow (debug) or wrap to the import at
/// offset 0 (release: the material's shader as its texture).
#[test]
fn material_and_model_tables_do_not_wrap() {
    let mut d = vec![0u8; 0x40];
    d[0x20] = 2;
    put(&mut d, 0x24, &0x30u32.to_le_bytes());
    put(&mut d, 0x28, &0xFFFF_FFFCu32.to_le_bytes());
    put(&mut d, 0x2C, &0xFFFF_FFFCu32.to_le_bytes());
    let mut r = res(d);
    r.imports.push(mw_bundle::Import { id: ResourceId(5), offset: 0, kind: mw_bundle::ImportKind::Pointer });
    let m = Material::parse(&r).unwrap();
    assert!(m.samplers.iter().all(|s| s.texture.is_none() && s.sampler_state.is_none()));

    let mut d = vec![0u8; 0x20];
    put(&mut d, 0, &0xFFFF_FFFCu32.to_le_bytes());
    d[0x14] = 2;
    r.chunks[0] = d;
    assert_eq!(Model::parse(&r).unwrap().renderables, [None, None]);
}

fn texture(format: u32, size: u16, mips: u8, data: Vec<u8>) -> Resource {
    let mut d = vec![0u8; 0x30];
    put(&mut d, 0x08, &7u32.to_le_bytes());
    put(&mut d, 0x1C, &format.to_le_bytes());
    for o in [0x24, 0x26, 0x28, 0x2A] {
        put(&mut d, o, &size.to_le_bytes());
    }
    d[0x2D] = mips;
    let mut r = res(d);
    r.chunks[1] = data;
    r
}

/// Mip levels past 31 are 1×1 (the shift used to overflow; in release it wrapped to the full size).
#[test]
fn texture_mip_counts_past_32() {
    let mut r = texture(28, 4, 40, vec![0; 4096]); // R8G8B8A8_UNORM 4×4×4, 40 mips
    put(&mut r.chunks[0], 0x28, &1u16.to_le_bytes()); // depth 1
    put(&mut r.chunks[0], 0x2A, &1u16.to_le_bytes()); // array size 1
    let t = Texture::parse(&r).unwrap();
    assert_eq!(t.slice_size(), 64 + 16 + 38 * 4);
    assert_eq!(t.pixel_data().len(), t.slice_size());
    assert!(t.decode_rgba8(35).is_some());
}

/// 65535³ texels × 65535 slices × 16 bytes overflows usize: the payload is clamped to the data, not a panic.
#[test]
fn texture_payload_size_overflow() {
    let t = Texture::parse(&texture(2, u16::MAX, 255, vec![1, 2, 3])).unwrap();
    assert_eq!(t.pixel_data(), [1, 2, 3]);
}

/// A blob import table at the top of the u32 range: no overflow panic, no wrap to the import at offset 0.
#[test]
fn groundcover_blob_table_does_not_wrap() {
    let mut d = vec![0u8; 0x40];
    put(&mut d, 0x38, &2u32.to_le_bytes());
    put(&mut d, 0x3C, &0xFFFF_FFFCu32.to_le_bytes());
    let mut r = res(d);
    r.imports.push(mw_bundle::Import { id: ResourceId(5), offset: 0, kind: mw_bundle::ImportKind::Pointer });
    assert!(GroundcoverCollection::parse(&r).unwrap().blobs.is_empty());
}
