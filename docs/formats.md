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
- **Packed tangent frames**: layouts without a NORMAL carry a quaternion in the lowest four-component
  RGBA8 / SNORM16 TEXCOORD (world 2 or 3, trees 1, vehicles 4; wheels also have 5 for the mirrored technique).
  RGBA8 decodes as `v·2√2 − √2`, SNORM16 as `v·√2`; tangent = the rotated X axis, normal = rotated X × rotated
  Y, negated (and the bitangent sign flipped) when `q.w < 0`; bitangent = sign · (N × T).
  `mw_formats::tangent_frame`, checked against face normals (98.8 % world / 97.3 % 911 agree within 45°, and
  all point to the counter-clockwise side).
- **Renderable (0x5, version 12)**: mesh list; topology 5 (triangle strip, restart index 0xFFFF) is the norm.
  Vehicle and character positions are `R16G16B16A16_SNORM` scaled by **10.0**, which was confirmed by the
  Porsche 911 decoding to its real 4.49 × 1.97 × 1.30 m. World geometry uses `R32G32B32_FLOAT`.
- **Material (0x2)**: shader import at 0x08 and per-sampler texture and sampler-state imports. Each material
  sampler carries a `u16` key that matches the key of the same-named sampler in the Shader.
- **Shader (0x53)**: name at ptr 0x08, sampler names at ptr 0x68, vertex descriptor import at 0x9C (true for all
  373 shaders).
- **Model (0x51)**: renderable imports and an 8-entry state table (LOD0–5, shadow, occlusion).
- **InstanceList (0x50)**: 0x60-byte instances, each a model import plus a row-major D3D matrix at +0x20
  (`v * M`, translation in row 3). The matrix's w column is per-instance data: the world vertex shaders pass it
  to the pixel shader, which multiplies albedo by its xyz (tint) and the AO map by its w (AO scale,
  0.90–1.0). +0x10 points to a per-instance **material override** block (11,974 of 35,431 instances): named
  constants (`diffuseTintColour` 10,100×, `AdditiveIntensities`, `InstanceAdditiveAndAO`) and named samplers
  (`AOMapTextureSampler` 781×, a SamplerState + Texture import each). `diffuseTintColour` is not a shader
  parameter; the engine folds it into the instance tint. Layout in `mw-formats/src/world.rs`
  (`InstanceOverrides`).
- **InstanceList backdrop**: entries past the always-loaded count are the unit's backdrop, the world-space
  geometry the game draws while the player is *in that unit* (terrain tiles, building blocks, water planes,
  horizon) at the LOD that suits the distance; other units list other tiers of the same tiles. Drawing every
  unit's backdrop stacks the tiers. `mw_assets::backdrop` composes one backdrop per place from the zone that
  owns it (research log 2026-10-04, night).
- **ZoneList (0x90)**, the only resource in `HAWAII/PVS.BNDL`: one zone per track unit. Header: ptr points
  (16 bytes each: x, z, 0, 0), ptr zones (0x50 each), ptr per-zone first-point index (u32), ptr per-zone
  point count (u16), u32 zone count (169), u32 point count (941), u32 3. Zone: ptr points, bounds min/max at
  0x10/0x20 as (x, ∓FLT_MAX, z, 0), ptr neighbours (`ptr zone, u32 flags`: 3 shared edge, 1 nearby) at 0x30,
  zone ID = track unit number at 0x34, district ID at 0x38 (`DISTRICT_<id>.BNDL`), arena ID or 0 at 0x3C
  (`ARENA_<id>.BNDL`), u8 flag at 0x40, point count at 0x42, neighbour count at 0x43.
  `mw_formats::ZoneList`.
- **Prop/Compound/Dynamic/LightInstanceList (0x218 / 0x216 / 0x204 / 0x213)**: header `u32 version`
  (2/2/3/1), `ptr` 0x10, `u32 count`, 0; then 0x60-byte records: a 3x4 row-major matrix at +0x00 (axis rows,
  translation row 3; column 3 ignored, row 3 w is often 0.94, maybe a packed tint byte), an object import at
  +0x40, an instance id at +0x48 (+0x4C for compound/light), and a pointer at +0x50 into a second per-instance
  block after the records (props: `InstanceAdditiveAndAO`). Unlike `InstanceList` the matrix is at +0x00, not
  +0x20. Props, compounds and dynamics import a `PropObject` / `CompoundObject` / `WorldObject`; lights
  import a GeneSys handle (light definition, not decoded).
- **PropObject (0x217)**: model imports at 0x04 and 0x08 (same model in practice). **CompoundObject (0x215)**:
  model imports at 0x04 (high detail), 0x24 (lower-poly variant, same bbox) and sometimes 0x28; the rest
  (part transforms, bounds, GeneSys handles) is not decoded. **WorldObject (0x205)**: model import at 0x04.
  Compound/world objects stand roughly upright from their instance matrix, so drawing slot 0 is enough for
  static scenery. `mw_formats::{ObjectInstanceList, ObjectDef}`.
- **GroundcoverCollection (0x20F) / GroundcoverInstances (0x214)**: grass and plant sprites, no meshes (the
  `Groundcover` shader billboards them). Layout in the doc comment of `crates/mw-formats/src/groundcover.rs`:
  collection = materials (atlas texture + 16 regions each), 3 layers of a box tree whose leaves hold groups
  `(box, material, quads, blob, start, count)`; instances are 16 bytes with u16 x/y/z as fractions of the
  group box, yaw, size, ground normal, tint and region index.

## Vehicles

`VehicleGraphicsSpec` (0x106) assembles a car: body part models at the origin, plus four wheels with
position, quaternion, scale and 2–4 part models each (tyre, rim, disc, caliper; a bitmask marks which spin).
Right-hand wheels reuse the left-hand models mirrored on X. Layout tables are in `mw-formats/src/vehicle.rs`.

### Vehicle damage (deformation)

There is no dedicated deformation resource. A car's body is **skinned to a small skeleton** and damage is a
set of **skeletal animation clips whose frames are damage levels**:

- `VehicleGraphicsSpec` +0x04 imports the `Skeleton` (0xB2, `CgsAnimation::AnimSkel`): 33–47 bones per car.
  Bone positions are model space (they sit on the body), rotations are mostly identity, parents precede
  children. Bone IDs are CRC-32s of names that are not shipped (not `field_hash`, zlib or BZIP2 CRC of
  obvious names).
- Body and glass vertices carry `BLENDINDICES` (skeleton bone indices directly) and `BLENDWEIGHT`; the vertex
  shaders ignore weight `.x` and use `1 − (y + z + w)` for the first bone, then skin with `gBoneMatrixArray`
  (48 × 3×4 matrices). Checked on the 911: 28,893 body vertices, none outside the skeleton, and each bone's
  vertices cluster around it (doors, mirrors, bumpers, lights, spoiler, steering wheel).
- The `VehicleDamageBehaviour` GeneSys object's `AnimationList` handle points to the damage `AnimationList`
  (0xB0): 8 clips (7 on one car) in a fixed zone order **front, rear, left, right, front-left, front-right,
  rear-left, rear-right** (+X is the car's left). Each clip has 21 frames (two cars have one clip of 5 or
  202): frame 0 is the bind pose, the last frame fully wrecked. Translations are offsets from the bind
  position in the parent's space; rotations are applied in the bone's own space. A part breaking off gets
  scale 0 (and drops ~0.7 m) from the frame where it detaches (911 front clip: number plate at frame 2,
  a bumper piece at 7, a headlight at 20; side clips: mirror at 5).
- `Animation` / clip layout (`CgsAnimation::Anim`, field names from burnout.wiki) is in
  `mw-formats/src/animation.rs`: per-frame arrays of translations (f32×4), rotations (codec 1: 32-bit
  smallest-three, 2-bit largest index + 3 × 10 bits mapped to ±1/√2; codec 0: f32×4) and uniform scales,
  each with a per-bone remap table (0xFF = not animated). All 542 skeletons, 1,297 animations and 379
  animation lists in the install parse.
- `VehicleDamageBehaviour.Bodypart` (7–10 per car): hinged parts (`BonnetHinge`, `FrontBumperHinge`,
  `FrontLeftDoorHinge`, ..., names are `CgsCore.StringBase` strings): hinge position, mass, `DamageArea`
  (1 front, 2 rear, 3 left, 4 right), per-axis angle limits in degrees (unnamed fields), spring-like
  vectors and `DamageThreshold` lists (damage → looseness). Not used yet.
- The vehicle shaders also crumple the surface: `gavDamageVolumeValues` is a 3 × 5 grid of damage values
  over the car's XZ footprint (`gvDamageVolumeParam` maps model XZ to grid cells), bilinearly sampled per
  vertex, `max`ed with `g_damageConstants.x`, and used to blend in `CrumpleTextureSampler`'s normal map and
  darken the surface. Not ported yet.
- Global tuning: `ImpactDamageGraphs` in `PHYSICS/PHYSICSCONFIG.BNDL` has two 3-point curves; the first
  reads as (5, 0 %), (80, 33 %), (160, 100 %) impact speed → damage. Field names and units are unknown.

`VehicleList` (0x105, `VEHICLES/VEHICLELIST.BNDL`) lists 110 cars; the entry ID names the
`VEHICLES/VEH_<id>_HI.BNDL` / `_LO.BNDL` pair, and names come from `Language` (0x201, `UI/LANGUAGE/0001.BNDL`
for English).

## Collision (`PolygonSoupList` 0x60)

One list per track unit, world space. Vertices are `u16 × 500/32768 + i8 multiplier × 500` metres. Polygons
are 0xC bytes (`u16 tag[2]`, `u8 indices[4]`, `u8 edge cosines[4]`), quads first. Quads are in triangle-strip
order: split them as (a, b, c) + (b, d, c). Layout in `mw-formats/src/collision.rs`.

## GeneSys (`GenesysType` 0x14, `GenesysObject` 0x15)

Criterion's reflected data system; most configuration (vehicle handling, gameplay rules, audio mixing) lives
here. A type has a kind (int, uint, float, bool, char, enum, value, struct, resource handle), a size, a name
string and 0x1C-byte field records `{type import, type hash, 0, name hash, size, offset, u16 count, u16
flags}`. Flags: low byte = alignment, `0x100` pointer (in-chunk sub-object; with an import on the field, a
reference to another object), `0x200` fixed array, `0x800` dynamic array, `0x1000` array of pointers. Dynamic
array lengths are trailing small unsigned fields, assigned to the arrays sorted by (alignment descending,
offset). Exceptions: a `WaveSequenceItem`'s `Waves` are counted by its second trailing u8 (`WavesCount`), and
handle arrays are laid out back to back, so a handle array whose slots up to the object's next array all hold
handle imports is that long (EngineSound2's and VehicleSound's lists; their count fields don't follow the
rule). `CgsCore.StringBase` = `{i32 offset from the field to the text, u32 length incl. NUL}`. Objects start
with a type import at +0. Layout tables are in `mw-formats/src/genesys.rs`.

Field names are stored only as hashes: CRC-32 (0x04C11DB7, MSB first) computed without augmentation, with the
register preloaded with the complement of the first four bytes and the result complemented, so names of four
bytes are stored verbatim (`Mass` = 0x4d617373). `mw_formats::genesys::field_hash` implements it, and the names
recovered so far are in `genesys_names.txt` next to it. Type names are stored as strings, e.g.
`Genesys.Gen.HandlingBehaviour`; nested groups are types named `Parent.Child`, held in a field named `Child`.

Fields are laid out by in-struct size (descending; nested structs first, then arrays, floats, pointers, counts)
and, within a size, by name, case-insensitively (`Maximum_deceleration_for_pitch_change` before
`Maximum_Pitch_Change`; holds for every named camera field). The `655d3f5f` UniqueId sorts first in its group.
So an unnamed field's name sorts between its neighbours' names, which rules out most dictionary hits.

## Shaders

`Shader` (0x53) also carries named parameters with hashes and defaults; materials override them by hash.
Compiled programs are `ShaderProgramBuffer` (0x8) resources whose chunk 1 is plain D3D11 DXBC (SM4.0) with
reflection data. `mwtool shader-asm <ShaderName>` disassembles a shader's pixel and vertex programs; it is the
reference for porting a shader family to WGSL (see `crates/mostrusted/src/layered.wgsl`).

World space is metres, Y up. Signage renders unmirrored without flipping any axis.

The renderer is deferred. G-buffer pass outputs (common to every family decoded so far): RT0 = world normal
·0.5+0.5 and **roughness** (not gloss; the reflection cube mip is roughness × 5.5), RT1 = sqrt(linear albedo)
and AO, RT2 = F0 and a Fresnel strength, RT3 = sqrt(indirect × exposure) (cube reflection × env-BRDF LUT ×
AO², lightmap, vertex light). Specular maps: WorldPBR `spec.x` → F0 through the cubic
`((0.307841 s + 0.686033) s + 0.006163) s + 0.001527`, `spec.y` = roughness; ColouredSpecular takes F0 from
`SpecularColourTextureSampler` × MaterialDiffuse. Normal maps are rgb ×2−1 in the vertex tangent frame (terrain
tiling normals: xy + roughness in w, z reconstructed). Dry roads and plots have no cube reflection (only a
planar reflection scaled by the wet mask). Alpha-tested shaders discard below 129/255. Per-family details:
research log 2026-10-04 (shaders) and the comments in `layered.wgsl`.

Car paint is not in the material: `VehicleNFS13_BodyPaint` reads `PbrPaint*` object constants that come from the
car's `Genesys.Gen.VehiclePaint` objects (`PbrDiffuseColour`, `PbrSpecularColour`, `PbrSpecularRoughness`,
`PbrClearcoatFresnel`, `PbrClearcoatRoughness`, `PbrClearcoatEnabled`, `PbrPearlescentColour`, ...; 9 per car
for the 911). `mw_assets::paint`.

**Environment / time of day** is GeneSys in `HAWAII/ENVIRONMENT.BNDL` (no 0x112–0x114 resources exist):
`EnvironmentTimeline` (25) lists `{Keyframe handle, hour}` (field `ccdbc945`, hour `da6ac384`; 21 entries,
hours 11/15/16 absent). `EnvironmentKeyframe` (175) holds `LightRig` (KeyLightColour; Key/Shadow/Up/Down/Left/
Right fill colours; AmbientIrradianceScale), Sky (`7dfab0cc`: SunColour, SkyReflectionAverageColour/Light,
SkyColorGradient = 6 unsorted stops of (position 0 horizon .. 1 zenith, r, g, b, 1)), Clouds (BaseCloudTexture
= low bits of a Texture ID, DarkColour, LightColour, Opacity, Feathering, Scale, Speed), Fog (`6ada1ed8`:
Colour, ColourSun (5 floats), NearDistance, FarDistance, Power, ScatteringColour/Phase/Strength, LayerBase/
Height), Camera (bloom, exposure, tint), HeatHaze, Mini_DOF, Vfx, Weather, a ColourCube handle and an
`Alternate` (bad-weather twin) handle. The keyframe has no sun direction (EnvironmentSettings' SunMoonPath is
only partly named). `mw_assets::environment`; values are game HDR units.

**Which timeline**: the `WorldPaintMap` `…2823d` (below) paints the 25 timelines over the map;
`EnvironmentSettings` field `409f546f` holds its ID's low bits. Timelines repeat a keyframe over consecutive
hours, so the environment at an hour is a blend of the two entries around it (interpolation assumed).
**Alternates**: 154 keyframes name one of 7 `Alternate` keyframes, one per time-of-day band (night, dawn,
morning, noon, evening, front end): hazier twins (fog far 550–1200 m, key light ×0.6–0.8, own colour cube).
No timeline lists them; what selects them is not in the data. **Weather** block: `WindBearing` (130°),
`WindBearingVariability` (25°), `WindSpeedMin` / `WindSpeedMax` (1 / 3.5), `Thunderstorm_Intensity` (0.23 at
night), unnamed `0141d842` (0.25 / 0.15) and `e8eb5d48` (0). There is no rain anywhere in the data.

`HAWAII/ENVIRONMENT.BNDL` also holds HDR panoramas (256×64 RGBA16F, equirectangular upper hemisphere,
horizon at row ~58) and cloud layers (2048×512 / 2048×256 BC3, alpha = coverage, each with a 256×64 copy) for
the `NewSky` dome shader (two 1-D gradients, four cloud layers, sun glow). The panoramas are a sunrise capture
(glow ~3° above the horizon) that no keyframe references.

The deferred lighting pass `LightBuffer_KeylightAndAmbient`: key light = KeyLightColour × CSM shadow ×
(1 − 0.8(1 − AO)); diffuse Lambert × (1 − F0), or for RT3.w > 0.992 (vehicles) the Ashikhmin-Shirley term
28/(23π)(1 − F0)(1 − (1 − N·L/2)⁵)(1 − (1 − N·V/2)⁵); specular normalised Blinn-Phong, power exp(11 − 11·
roughness), (n+1)/8π, visibility 1/(max(N·L, N·V)·V·H + 0.0013), Schlick Fresnel with RT2.w scaling the
(1 − F0) part; ambient = irradiance cube(N) (stored sqrt(x/4); vehicles blend a second cube by distance) ×
AO × (1 − F0) × albedo; plus RT3² × exposure. The Fog pass: amount pow(saturate(d/(far − near) −
near/(far − near)), Power) × gradient alpha, height-layer fog, sun-corona in-scattering
strength/(1 + phase·cosθ)², sky pixels only get the layer fog.

## WorldPaintMap (0x91)

A value painted over the world, one quadtree per 1 km cell (`mw_formats::WorldPaintMap`, `mwtool paint-map`).
Three in the install: `ENVIRONMENT.BNDL` `…2823d` (the 25 environment timelines), `…285cb` (the 19
`LayerFog` objects), `GLOBALEFFECTS.BNDL` `…285cc` (one `ParticleLighting`). Header: version 1, entry count,
offset of the entry list (u32 low bits of resource IDs; entry 0 = nothing), grid width and height in cells,
grid origin in cells (i32 x, z), cell size (1000.0) and its inverse, an unknown u32, offset of the cell table
(`u32 offset, u32 size` per cell, row-major; size 0 = unpainted). A cell's block is a quadtree of 8-byte
nodes, root first: four child bytes then four offset bytes, children ordered (x0 z0, x1 z0, x0 z1, x1 z1);
a child byte < 0x80 is a leaf holding an entry index, else the child node sits at block offset
`(byte & 0x7F) << 8 | offset byte`. World → cell: `floor(x / 1000) + origin.x`, `floor(z / 1000) + origin.z`
(the zone outlines line up with the painted regions).

## Road wetness

The `DriveableSurface` wet path (ported in `layered.wgsl`): wet = puddle.w × saturate(TEXCOORD5.x), where the
puddle map is the engine-bound `puddleMapSampler` = `GLOBALTEXTUREDICTIONARY.BNDL` `…7811` (256², tiled over
world xz × `WetScaleParams.x`) and TEXCOORD5.x a per-vertex mask (0 or ≥ 1). k = lerp(1, (RoadDepth.y ×
puddle.x, puddle.y, puddle.z), wet): the normal falls back towards the vertex normal by k.x, roughness × k.y,
albedo × k.z, F0 = lerp(0.02, F0, k.x). The road's only indirect specular is the planar reflection, read at
the mirrored projection shifted by (N − N_vertex).xz × `WetScaleParams.y`, at mip roughness ×
`WetScaleParams.z`, × env-BRDF × wet. No global wetness input exists: roads always carry the puddle map's
wetness. `PlotPBR_AO_StandingWater` does the same with its own `PuddleTextureSampler` (TEXCOORD1).
`SurfaceType` objects have `Dry_Conditions` and `Wet_Conditions` blocks (physics, rumble, sounds, vfx).

## Diffuse sampler names

Most world and vehicle shaders use `DiffuseTextureSampler`. Exceptions:

- Roads (`DriveableSurface*`) use `DetailMap_Diffuse_Sampler` (tiling asphalt) plus `ColourMap_Sampler`,
  `Line_*`, `Decal_*` and `RoadDepth_Sampler`.
- Terrain and plots (`TerrainPBR*`, `PlotPBR*`) blend `Tiling1/2/3TextureSampler` through `MaskTextureSampler`.
- Ground texture coordinates (DXBC, research log 2026-10-06 "ground texture scale audit"):
  - Roads: ColourMap / Normal / SpecColour / RoadDepth at raw TEXCOORD0; DetailMap_Diffuse / DetailMap_Normal
    at world xz × `DetailMapUvScale.xy`; puddle map at world xz × a global scale. `Line_*` and `Decal_*` at raw
    TEXCOORD2, lerped over the road by their alpha. FloatingDecal(_1Bit) and RetroreflectivePaint_Lightmap
    write the plain road in the G-buffer pass.
  - TerrainPBR: mask and macro normal at raw TEXCOORD0; layer i at TEXCOORD1 × (`Tiling_Ratios_X[i]`,
    `Tiling_Ratios_Y[i]`); noise at TEXCOORD1 × `Tiling_Ratio_Noise`.
  - PlotPBR: layer i at TEXCOORD0 × (`Tiling_Ratios_X[i]`, `Tiling_Ratios_Y[i]`); mask at TEXCOORD0 ×
    `maskTilingRatios.xy`; decal at TEXCOORD0 × `decalTilingRatios.xy`; AO map and lightmap at raw TEXCOORD1.
  - Material parameters are float4 each; the shader's cbuffer takes the leading components (the
    `Tiling_Ratios_*` are float3, `Tiling_Ratio_Noise` a float).

Run `mwtool shaders` for the full list.

## Audio

See `crates/mw-audio` (decoders reimplemented from vgmstream's documentation of the formats; ISC, see its
`NOTICE`).

- **SPS** (`.SPS`, and `Wave` resources from offset 0x80): blocks of `u8 id, u24 size`; `H` = EAAC header
  (`version:4 codec:4 channels-1:6 rate:18`, `type:2 loop:1 samples:29`), `D` = `u32 samples` + codec data,
  `E` = end. MW uses codec 5, EALayer3 v1: one MPEG Layer III granule per EA frame with a compacted header and
  side info (+ optional big-endian PCM blocks). Multichannel = interleaved stereo/mono sub-streams. Sample
  rates include 32 kHz (MPEG-1): rebuilt free-format frames must stay under 2304 bytes, or minimp3 finds none.
- **Ginsu** (`Ginsu` resources, 8-byte prefix, then `Gnsu30`): min/max RPM, table sizes, sample count and rate,
  a u32 at 0x20, an RPM → recording-position table, grain boundaries, then EA-XAS v0 (19-byte frames, 32 mono
  samples).
- **Wave** resources: 0x80-byte header (f32 1.0 at 0x10, duration in ms as f32 at 0x14), then an SPS stream; its
  `H` block's loop flag separates loops (engine idle, skid, wind) from one-shots (pops, clunks, impacts).
- **Sound sequences** (GeneSys, `mw_audio::sequence` has the full model): `Sequence { Binding (StringBase: a game
  value such as "Players.LocalPlayer.GetPlayerAudibleRPM", optionally with a test "(NOT)", "(GTEQ:100)",
  "(BETWEEN,20,60)"), Binding_Min, Binding_Max, SequenceItems }`; an empty binding makes a timeline in seconds.
  Items sit between `StartTime` and `EndTime` on that axis: `WaveSequenceItem { Gain, Pitch (modulating
  values), Fade_In / Fade_Out {Curve, Time} (axis units, inside the range), Mixer_Channel, Waves,
  Trigger_Probability }`, `GinsuSequenceItem { Fade_In, Fade_Out, Ginsu, Mixer_Channel }`,
  `MixerChannelSequenceItem { Mixer_Channel, Double_Params [{PlugIn (unique id, 0 = the channel), Value,
  Parameter_Name (field hash of the target, e.g. Frequency), Automated_Property (0 = channel gain, 9 = plug-in
  parameter)}] }`. `ModulatingDoubleValue { Binding, Binding_Exponent, Binding_Range_Min/Max, Output_Value_Min/
  Max, Value, AnimPoints {x 12d3a8aa, y 12d3a8ab, TangentIn/OutX/Y}, Binding_Modulation_Type,
  Animation_Modulation_Type, Binding_Evaluate_Once, Binding_Invert }`. `MixerChannel { Gain (linear),
  MixingGroup, plug-in chain }`; chains hold `Submix (Mixing_Groups it collects)`, `Send (Voice_Routing → channel,
  Gain)`, `Peaking (Frequency, Gain linear, Q)`, `HighShelf`, `Low/HighPassButterworth (Frequency, Order)`,
  `BandPass (Bandwidth, Frequency)`, `Distortion (Distortion, Model)`, `SubHarmoniser (Interval)`, `Delay
  (Feedback, Time)`, pans, `GinsuDspPlugIn`. `EngineSound2 { Sequences (always on), Burnout, Drift, In_air,
  Reversing and six unnamed sequence lists (gear changes), Backfire_Sequence, Gurgle_Sequence (bound to
  GetPopProbability), Pop_Sequence, Ignition_Sequence, c62d11ac (rev limiter), mixer channel list, DspParam
  lists (per-car plug-in overrides), Gains (per-car channel trims), MasterGain }`. Unnamed `EngineSound2`
  floats the game's code reads (research log "engine sound from the game code"): `99d4b91e` load response
  rate, `a33d84fe` / `cb44d82d` / `913bb900` RPM-curve terms (full load / off load / alternate; audible RPM =
  1000 + 7000 × (idle..max fraction)^(2 − a)), `283e07de` sub-harmoniser, `7cbc11ca` / `7cbc11c9` tracked
  harmonic 1 / 2 and `dbcddcfc` tunnel harmonic as multiples of the firing frequency (RPM / 120 × cylinders
  `ed754b99`), `11a77b36` / `64cbe1ff` tunnel gain / Q, `f0291dd3` comb-rasp delay divisor. `DspParam
  { Mixer_channel, PlugIn (unique id), Value, Parameter_name }`: index 1 on a filter reads as Frequency, 0 on a
  Send as its gain; 4 (Delay) and 5 (BandPass) are unresolved.
- **Surface and collision sounds**: `Surfaces { Default_surface, Chevron_surface, Surfaces }` →
  `SurfaceType { two condition sets (dry / wet) { Flats, Normal, Offroad, Rims, Track: { Surface_Physics,
  Surface_Rumble, Surface_Sounds } } }` → `SurfaceSounds { Brake, Drift, Spin (+ _Competitor), Reflection, two
  speed-bound road sequences, Bumps, Transitions, TyreParams }` (`HAWAII/SURFACELIST.BNDL`). Skid sequences
  bind `RaceCars.Entities["Id"].{LatSlip,Spin,SpinBraking}IntensityForSkidSound` (0..1). `Vehicle_Sound
  { Collision_Sounds, Horn (UniqueIds) }` → `CollisionSoundSet` (`SOUND/METALCAR.BNDL`): 31 unnamed timeline
  sequence slots and 24 wave pools (`Scratch`, `Sparks`, `Landing`). `TyreSound` in `VEHICLES/WAVES.BNDL`
  holds only the silent placeholder wave `…79ff`.

## Particle effects (`BearEffect` 0x301)

All 214 effects are in `GLOBALEFFECTS.BNDL`. An effect has named parameters the game sets per instance
(`Transform`, `Velocity`, `Intensity`, `IsEmitting`, `WheelParams`, `CollisionMeshParams`, ...) and layers;
a layer is a particle pool with named channels (`Position`, `Velocity`, `Age`, `RecipLifetime`, `Radius`,
`Colour`, `SubUV`, ...) and typed actions that read and write them, run in four action groups (0 = update:
controller, per-particle updates, spawners whose secondaries initialise new particles; 1 = render). Action
payloads are 4-byte words: floats, channel / parameter / engine operands (`u16 offset, u16 4·channel |
0x40 + 2·param | 0xEB4x`) and, in renderers, material or model imports. 5-key curves are `u8 values[5],
u8 times[5]`. Layout tables, operand decoding and the decoded action kinds: `mw-formats/src/bear.rs`
(`mwtool effect <id>` prints one, `mwtool effect all` / `library` list them).

Which effect plays where is GeneSys: `HAWAII/SURFACELIST.BNDL` `Surfaces.Surfaces` → `SurfaceType` (dry /
wet) → `SurfaceVfx` (`WheelVfx` kick-up, `PlayerWheelEffects` / `AIWheelEffects` → `VfxWheelEffects`
`DefaultTyre` smoke / `OnRims` sparks, ten `*ImpactEffect` by direction); `GLOBALEFFECTS.BNDL` `VfxGlobals`
(`VehicleImpacts`, `VehicleScrapes`, `Misc`). `mw_assets::vfx`.

Shaders: `VfxParticles_Diffuse*` draw CPU-expanded, per-vertex-lit quads with premultiplied output (atlas
frames from `NumTiles` / `EnableLooping`); `VfxParticles_MotionBlurSpriteUntex` draws sparks as moving discs
with analytic coverage, additive. Ports: `crates/mostrusted/src/particles.wgsl`, `sparks.wgsl`.

## ColourCube (0x52)

16-byte header: u32 edge N (16 everywhere), u32 N, 8 zero bytes; then N³ three-byte entries laid out as a D3D
volume texture without alpha: input red fastest, then green, then blue, each entry stored B, G, R (the output
for input (r, g, b) at 3 × (r + N g + N² b)). Gamma-encoded in and out; `…3e93` is the identity (symmetric, so
it does not decide the order). The order follows from how the game uses the bytes (blended and copied
unchanged into a B8G8R8A8-ordered volume sampled at x = red; research log 2026-10-06, inferred: the blend
kernels are missing from the dump). Until 2026-10-06 it was read as blue fastest with R, G, B entries, which
swapped red and blue and caused the yellow cast. Each environment keyframe names one (`ColourCube` handle);
PostFX states (`POSTFX.BNDL`) name more. Parser: `mw_formats::ColourCube`.

