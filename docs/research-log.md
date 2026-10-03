# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

---

## Open questions

### Textures
- Header flags at 0x20: values 0x00/0x10/0x20/0x30. Is one of them sRGB? (Currently all colour textures are
  treated as sRGB.)
- Byte 0x2C ranges 0–10 and does not mean missing mips (full chains are present). Streaming LOD bias?

### Renderable
- Mesh bytes 0x24–0x27 (sub-part code, then 1, 0, 4 in samples).
- The compressed OBB at mesh+0x00: the exporter documents its quantisation. Decode it for culling.
- Buffer descriptor words `{0, 1, 3|2}`: D3D11 bind flags?
- Is the SNORM position scale always 10.0, or does it ever vary per mesh/renderable?

### Vertex data
- Vehicles: `TexCoord(4)` / `TexCoord(5)` as `R16G16B16A16_SNORM` look like packed tangent-frame quaternions
  (the exporter writes normals through `normal_to_quaternion`). Derive normals from them.
- `Color` (RGBA8) on world and vehicle meshes: on roads it is baked lighting (VS: `rgb² × lightmap intensity ×
  0.318`). Check the other families.
- World layouts often have Tangent + Binormal but no Normal: is the normal `cross(T, B)`, and with which sign?
- RGBA8 `TEXCOORD2` (terrain) / `TEXCOORD3` (roads) is a packed tangent frame: the VS decodes it as
  `v * 2.828427 - 1.414214` (smallest-three quaternion style). Port the decode for proper normals.

### Materials / shaders
- Which UV set and texture does each lightmap shader use? (Answer with `mwtool shader-asm`.)
- Why does Bevy's `clearcoat` make vehicle paint meshes vanish? (Probably missing tangents.)
- Tyres render too light. Check the wheel shader's albedo maths.

### World
- InstanceList instance `unknown_04` (f32) and `unknown_08` (u32).
- ZoneHeader layout for MW (the exporter writes six import slots at 0x40+).
- What does `WORLDLIST.BNDL` contain, and how does it relate to `HAWAII/MAPS/*.BNDL`?

### Data
- GeneSys field/type name hash: not CRC32, FNV-1/1a (32/64), djb2, sdbm, x31/x65599, one-at-a-time, murmur3,
  xxh32/xxh64/xxh3, CRC-64 (ECMA/WE/XZ/ISO), or Jenkins lookup2/lookup3/lookup8 (seeds 0, 0xABCDEF00,
  0xABCDEF0011223344), on the exact/lower/upper-case name, tested against ~440k exe strings. The 64-bit value at
  type+0x10 may be a layout fingerprint rather than a name hash. (Resource IDs of types are CRC32 of the
  lower-cased type name, as usual.)
- GeneSys: partially decoded (see the 2026-10-03 "vehicles, names, shaders" findings). Field names are stored only as hashes; find the
  hash function and a name source.
- VehicleList entry fields beyond ID / names / year (floats look like stats; u16 near 0x6A may be RPM).

---

## Findings

### 2026-10-03 (night): GeneSys decoded, handling, LOD stand-ins, controller
- **GeneSys decoder works** (`mw-formats/src/genesys.rs`, `mwtool genesys`). Field flags: low byte = alignment,
  `0x100` pointer (in-chunk sub-object; with an import on the field = external ref), `0x200` fixed array,
  `0x800` dynamic array, `0x1000` array of pointers. Array lengths are trailing small unsigned fields, assigned
  to arrays sorted by (alignment descending, offset): RigidBody's bool array is laid out first but counted last.
  Every struct starts with a type import at +0.
- Field names of four characters or fewer are stored verbatim (`Mass` = `0x4d617373`); longer names are hashed
  with the still-unknown function.
- All vehicle handling data is in GeneSys; see [handling.md](handling.md) for the mappings and evidence.
- Three objects per vehicle bundle fail to decode (pointers read as huge offsets). Their types are not needed
  for handling; still open.
- **Wheel labels can lie**: the Veyron's spec names its rear wheels "Front". All cars face +Z, so wheels are now
  classified by position (front = z above the midpoint; mirror when x < 0).
- **Backdrop instances**: InstanceList entries past the "always loaded" count are world-space geometry at the
  identity transform (terrain, water, roads, horizon). They also include LOD2 stand-in buildings
  (shaders containing `LOD2`) that overlap the detailed buildings; those meshes are now stripped from backdrop
  placements (`--keep-lod-proxies` brings them back). This was the "misplaced assets" report.
- **Controller**: gilrs's bundled SDL mapping DB lacked the 8BitDo Ultimate 2 (`03000000c82d00000b31…`), so its
  triggers were unmapped. The current SDL GameControllerDB is bundled and passed via `SDL_GAMECONTROLLERCONFIG`;
  `--gamepad-debug` logs raw input.
- Physics pitfalls: avian `LinearDamping` acts like ~1.7 kN of drag at 240 km/h (now 0); suspension damping
  must use velocity along the ground normal, not the car's up axis (porpoising at speed).

### 2026-10-03 (evening): collision and driving
- **PolygonSoupList** decoded (layout in `mw-formats/src/collision.rs`): world-space, vertices `u16 ×
  500/32768 + i8 multiplier × 500 m`, quads before triangles. One list per track unit (TRK_UNIT1: 158 soups,
  6.9k triangles). It includes invisible walls / barriers.
- GenesysType kinds: 0 int, 1 uint, 2 float, 3 bool, 4 char, 5 enum, 6 compound value (Vector3, StringBase),
  7 class/struct, 8 resource handle. The type name is at `*(*(type+0x20))`, size at 0x18, fields inline from
  0x24 (0x1C each: type import, type-hash low32, 0, name hash, size, offset, u16 count, u16 align). Objects
  start with the base type's fields (0xC bytes for the common base).
- Drivable prototype: avian3d 0.7. Pitfalls hit: (1) wheel rays must exclude the body's child collider entity;
  (2) suspension damping from finite differences launches the car on first contact, so use the body velocity at
  the mount; (3) explicit `Mass` alone leaves collider-derived (density 1) inertia, so set `AngularInertia` and
  `NoAuto*` explicitly.

### 2026-10-03 (later): vehicles, names, shaders
- **Imports**: the high bit of the import offset word is a kind flag. `0` = pointer slot, `0x8xxxxxxx` = handle
  (the ID is stored in place). GenesysObject, ZoneHeader, the VehicleGraphicsSpec locator tables and others use
  handles. `mw_bundle::ImportKind`.
- **VehicleGraphicsSpec** decoded (layout in `mw-formats/src/vehicle.rs`): body part models, skeleton, collision
  PolygonSoupList, and four 0x90-byte wheels (pos, quat, scale, part models + spin mask + name). Wheel models
  are authored for the left (+X) side; right wheels are mirrored (scale.x = -1). Verified visually on the 911 and
  the Veyron.
- Vehicle IDs: `VEH_<id>_HI/LO.BNDL` uses the VehicleList entry ID (field 0x00). The GraphicsSpec resource ID
  is a GameChanger ID that also carries it (e.g. `0x0101000100108e4f` → 1085007).
- **Language**: `{u32 index, u32 count, ptr ids, ptr (offset,len) pairs}`, strings UTF-16LE.
  `UI/LANGUAGE/0001.BNDL` is English.
- **VehicleList**: 110 × 0x70-byte entries; 0x08/0x10 Language IDs, 0x6C year.
- **Shader parameters**: shaders store parameter names, hashes and defaults (0x14 / 0x1C / 0x20 / 0x24 /
  0x28), so material parameter hashes resolve to names (`PbrMaterialDiffuseColour`, `Tiling_Ratios_X`, ...).
- **ShaderProgramBuffer = DXBC** (SM4.0, with RDEF reflection, so names survive). Pixel programs are imported by
  the Shader; vertex programs sit behind VertexProgramState → ShaderProgramBuffer. `mwtool shader-asm <name>`.
  The renderer is deferred (4 G-buffer targets; albedo stored as `sqrt`).
- HLSL `TEXCOORDn` == VertexDescriptor `TexCoord(n)` (raw semantic 5+n).
- **DriveableSurface** albedo = `ColourMap(uv0) × DetailMap_Diffuse(world.xz × DetailMapUvScale) × 2`. Line,
  decal and paint variants are overlays.
- **TerrainPBR** albedo = Σ layer_i(uv1 × (Tiling_Ratios_X[i], Tiling_Ratios_Y[i])) × w_i × 2·mask.g, where
  `w.x = overlay(mask.r, noise)`, `w.z = overlay(mask.b, 1-noise)`, `w.y = 1 - mask.r - mask.b`, normalised;
  noise = `NoiseTexture(uv1 × Tiling_Ratio_Noise).g`.
- Vehicle bodies: `VehicleNFS13_BodyPaint` has no parameters (paint colour comes from elsewhere, likely player /
  GeneSys data); trim uses `VehicleNFS13_Body` with `PbrMaterialDiffuseColour`.
- GeneSys (started): `GenesysType` = reflected type. Header: `u8 2, u8 kind, u16 field count`, base-type import
  at 0x04, size at 0x0C, name hash at 0x10, name string near the end ("Genesys.Gen.NitrousParameters.TrafficNearMiss",
  "CgsCore.UniqueId", "uint16_t"...). Fields are 0x1C bytes: `{type import, type hash, 0, name hash, size,
  offset, u16 count, u16 align}`. Name hashes are not CRC32 of the full type name; the hash is still unknown.
- The `--screenshot` pose was being moved by gamepad stick drift; the camera controller is now disabled in
  screenshot mode.

### 2026-10-03: initial survey
- All bundles are `bnd2` v5, platform 1 (PC). Resource entries are 0x48 bytes (0x44 of data, padded for
  the u64). Every resource in the install decompresses to its declared size.
- `NFS13.exe` `.text` is encrypted (entropy 8.0); the entry point is in an appended DRM stub. See
  [reverse-engineering.md](reverse-engineering.md).
- Vertex attribute and texture formats are raw `DXGI_FORMAT` values.
- Vehicle/character positions: `R16G16B16A16_SNORM` × 10.0 (Porsche 911 = 4.49 × 1.97 × 1.30 m).
- Shaders import their VertexDescriptor at 0x9C (all 373 shaders).
- Material sampler `u16` keys match Shader sampler keys; shader sampler names identify the role.
- InstanceList matrices sometimes have `w = 1.0` in rows 0–2; ignore the w column.
- World: 169 `HAWAII/TRK_UNIT*.BNDL`, 35,424 InstanceList placements, ≈26M LOD0 triangles, 0 decode errors.
- Config bundles (`VEHICLELIST`, `GLOBALCONFIG`, `PHYSICSCONFIG`, `GAMEPLAY`) are almost entirely GeneSys
  types and objects.
- Audio: `.SPS` streams start with an EA SNR/SPS block header (`0x48`). Movies: `.VP6` with EA `MVhd`
  container.
- No signage is mirrored when the game's coordinates are used as-is (Y up, metres).
