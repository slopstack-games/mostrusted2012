# Most Wanted (2012) PC data formats

These notes describe what Most Rusted currently parses. Offsets are little-endian; "ptr" fields are offsets into
the resource's chunk 0, and pointers to other resources are filled from the import table. The parsers in
`crates/mw-formats` are the authoritative versions; every struct has its layout table in its doc comment.

## Bundles (`.BNDL`, `bnd2` version 5, platform 1)

Header (0x70 bytes): magic, `u16 version = 5`, `u16 platform = 1`, debug-data offset, resource count, resource
table offset, four per-chunk data base offsets, flags, default resource ID, default stream index, and four 15-char
stream names (`GRAPHICS`, `SOUND`, `PHYSICS`, `NAV`, `UI`...).

Flags: `0x1` zlib, `0x2` debug data (`<ResourceStringTable>` XML with names), `0x8` multistream, `0x10` delta,
`0x20` default resource.

The resource table has 0x48-byte entries: `u64 id`, `u32 uncompressed_size_and_alignment[4]` (low 28 bits are the
size, high 4 bits are log2 alignment), `u32 size_on_disk[4]`, `u32 disk_offset[4]`, `u32 imports_offset`,
`u32 type`, `u16 import_count`, `u8 flags`, `u8 stream_index`. Each chunk is compressed separately.

Imports are appended to chunk 0 at `imports_offset`, 0x10 bytes each: `u64 id`, `u32 offset`, padding. The
offset's high bit marks a *handle* import (the ID is stored at the offset) as opposed to a pointer slot.

Resource IDs are CRC32 of the lower-cased name. IDs with high byte `0x01` are "GameChanger" IDs, and those also
encode a resource type in bits 32–47, e.g. `0x0100005300007813` is a Shader.

Survey of a 1.4.0.0 install: 2,020 bundles and 432k resources, all of which decompress to their declared sizes.

## Resource graph for rendering

```
InstanceList ──> Model ──(LOD state)──> Renderable ──(per mesh)──> Material ──> Shader ──> VertexDescriptor
                                            │                         └──> Texture / SamplerState (per sampler)
                                            └── chunk 1: index + vertex buffers
```

- **Texture (0x1)**: D3D11 description (`DXGI_FORMAT` at 0x1C, dimensions at 0x24, mip count at 0x2D); chunk 1
  is the raw mip chain. Formats seen: BC1, BC3, RGBA8, R8, RGBA16F. Dimension 7 is 2D and 9 is a cube map.
- **VertexDescriptor (0x3)**: D3D11 input layout. Attribute `format` values are plain `DXGI_FORMAT` codes.
  Semantics: 1 POSITION, 3 NORMAL, 4 COLOR, 5–12 TEXCOORD0–7, 13 BLENDINDICES, 14 BLENDWEIGHT, 15 TANGENT,
  16 BINORMAL.
- **Renderable (0x5, version 12)**: mesh list; topology 5 (triangle strip, restart index 0xFFFF) is the norm.
  Vehicle and character positions are `R16G16B16A16_SNORM` scaled by **10.0**, which was confirmed by the
  Porsche 911 decoding to its real 4.49 × 1.97 × 1.30 m. World geometry uses `R32G32B32_FLOAT`.
- **Material (0x2)**: shader import at 0x08 and per-sampler texture and sampler-state imports. Each material
  sampler carries a `u16` key that matches the key of the same-named sampler in the Shader.
- **Shader (0x53)**: name at ptr 0x08, sampler names at ptr 0x68, vertex descriptor import at 0x9C (true for all
  373 shaders).
- **Model (0x51)**: renderable imports and an 8-entry state table (LOD0–5, shadow, occlusion).
- **InstanceList (0x50)**: 0x60-byte instances, each a model import plus a row-major D3D matrix at +0x20
  (`v * M`, translation in row 3; some rows carry `w = 1.0`, which must be ignored).

## Vehicles

`VehicleGraphicsSpec` (0x106) assembles a car: body part models at the origin, plus four wheels with
position, quaternion, scale and 2–4 part models each (tyre, rim, disc, caliper; a bitmask marks which spin).
Right-hand wheels reuse the left-hand models mirrored on X. Layout tables are in `mw-formats/src/vehicle.rs`.

`VehicleList` (0x105, `VEHICLES/VEHICLELIST.BNDL`) lists 110 cars; the entry ID names the
`VEHICLES/VEH_<id>_HI.BNDL` / `_LO.BNDL` pair, and names come from `Language` (0x201, `UI/LANGUAGE/0001.BNDL`
for English).

## Shaders

`Shader` (0x53) also carries named parameters with hashes and defaults; materials override them by hash.
Compiled programs are `ShaderProgramBuffer` (0x8) resources whose chunk 1 is plain D3D11 DXBC (SM4.0) with
reflection data. `mwtool shader-asm <ShaderName>` disassembles a shader's pixel and vertex programs; it is the
reference for porting a shader family to WGSL (see `crates/mostrusted/src/layered.wgsl`).

World space is metres, Y up. Signage renders unmirrored without flipping any axis.

## Diffuse sampler names

Most world and vehicle shaders use `DiffuseTextureSampler`. Exceptions:

- Roads (`DriveableSurface*`) use `DetailMap_Diffuse_Sampler` (tiling asphalt) plus `ColourMap_Sampler`,
  `Line_*`, `Decal_*` and `RoadDepth_Sampler`.
- Terrain and plots (`TerrainPBR*`, `PlotPBR*`) blend `Tiling1/2/3TextureSampler` through `MaskTextureSampler`.

Run `mwtool shaders` for the full list.
