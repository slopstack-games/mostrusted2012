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
  model imports at 0x04 (high detail, skinned to the skeleton at 0x08: one bone per part), 0x24 (lower-poly
  variant, same bbox) and sometimes 0x28; behaviours at 0x0C (count at 0x01; 0x14-byte entries: GeneSys import
  at +0x00, type id at +0x0C), joints at 0x2C (count 0x3C, none in the install), parts at 0x30 (count 0x38,
  0x70 bytes: embedded `WorldObject` with its own behaviours and physical definition at +0x1C, skeleton bone
  +0x60, replaces / next / first replacement u8 part indices +0x64..+0x66, linked parts +0x68 / +0x6C) and a
  u32 hash per part at 0x34. Layout table in `mw_formats::ObjectDef`; how the game uses it in decomp.md
  "Props". **WorldObject (0x205)**: model import at 0x04.
  Compound/world objects stand roughly upright from their instance matrix, so drawing slot 0 is enough for
  static scenery. `mw_formats::{ObjectInstanceList, ObjectDef}`.
- **GroundcoverCollection (0x20F) / GroundcoverInstances (0x214)**: grass and plant sprites, no meshes (the
  `Groundcover` shader billboards them). Layout in the doc comment of `crates/mw-formats/src/groundcover.rs`:
  collection = materials (atlas texture + 16 regions each), 3 layers of a box tree whose leaves hold groups
  `(box, material, size exponent e, blob, start, count, radius r)`; instances are 16 bytes with u16 x/y/z as
  fractions of the group box, i8 width and height (BINORMAL.xy = byte / 127, times the shader's
  `GroundcoverConstants.x` = 2^e), ground normal, tint and region index. Draw distance (`FUN_00eb9500`, the CPU
  vertex builder): a group is drawn while the camera is within 125·r of its box (`DAT_010c6f74` = 125.0,
  .data); each sprite within R = ((w>>1&63) + (h>>1&63))·2^e/127·125 of the camera and in front of it, its
  width and height scaled by min(5·(1 − d²/R²), 1) (`DAT_01081000` = 5.0), at most 20000 sprites
  (`DAT_010c6f70`). The `Groundcover` vertex program turns each sprite about world up to face the camera. The x/y/z decode is confirmed against collision
  (`mwtool placement-check --groups`: swapped or flipped axes match 3-10% of groups, the decode 50%, the rest
  stand on render-only terrain). The vertex program anchors each sprite at its base (corner = position +
  width x (u - 0.5) x camera-perpendicular horizontal axis + height x v x up; sizes scaled by `cb1[0].x`), a
  cylindrical billboard.
  **Sprite vertices** (`FUN_00eb9500`, 4 × 0x2C bytes per sprite, the `Groundcover` layout): POSITION = the
  decoded instance position (all four corners the same), NORMAL = instance bytes +8/+9/+10 (i8) × 1/127
  (`DAT_01081004` = 0.007874), BINORMAL = (+6, +7, +11) × 1/127 with +6/+7 (width, height) first scaled by
  the range shrink, COLOR = bytes (+12, +13, +14, +15) as R, G, B, A (tint rgb, region index in A; a debug
  mode replaces it with green + the region), TEXCOORD (two halfs) = (0,0), (1,0), (1,1), (0,1): u across, v
  up from the base. The vertex program picks the atlas cell by `round(A × 255)` from `GroundcoverTextureRegions`
  (`uv = region.xy + region.zw × (u, 1 − v)`; a negative scale mirrors the cell: region 2 of many grass
  materials is (0.5, 0.5, −0.5, 0.5), used by 41 k sprites; no sprite indexes an all-zero region or past 16), sways the top by `sin(Time × speed + x) × amount × v` along the
  sprite's right axis, and outputs the normal `normalize(sign(w) · right · (u − 0.5) + up · v − 0.1 · d) +
  max(8 − height, 0.75) · (ground normal + (0, 0.25, 0))` (d = horizontal direction from the camera, height
  in metres), mostly the ground normal. `gObjectCB` (272 bytes = `GroundcoverConstants` + 16 regions) is the
  collection's own 0x110-byte material record (`collection +0x30 + material × 0x110`), so
  `GroundcoverConstants` is its first 16 bytes; on disk they are the material import slot (zero). The draw
  loop `FUN_008c9aa0` binds the 16 regions (`material + 0x10`) to slot 0x80 and writes `(float)(1 << e)` into
  all four components of slot 0x7f (`GroundcoverConstants`) whenever the draw record's size exponent changes
  (the record's byte +0xC, copied from the group's +0x30), so **a sprite is byte / 127 × 2^e metres**: up to 4 m
  for e = 2, 16 m for e = 4, 64 m for e = 6 and 256 m for e = 8 (the e = 6 / 8 groups are tree cards that cover
  hills; live reads of slot 0x7f gave 4 and 256). The per-sprite range R = 125 × the sprite's mean size.
  **Which collections are drawn**: `FUN_00eb9f50` walks a list of 64 collection slots (copied by
  `FUN_008d6900`), each layer only when its flag `DAT_010c6f78[layer]` is set. Every track unit lists 1–34
  collections (400 in all, one listed by up to 144 units); a zone and all its `ZoneList` neighbours list at
  most 63 distinct ones, which fits the 64 slots (`mwtool placement-check --owners`). **Pixel program** (G-buffer): `DiffuseSampler`
  only; discard where alpha < 0.5; RT1 (sqrt albedo) = texel.rgb / texel.a × sqrt(2 · tint), w = 1; RT0 =
  normalize(normal) × 0.5 + 0.5, w = 1; RT2 / RT3 are not written; no lighting in the shader. The second
  pixel program (forward / HDR) outputs texel.rgb / a × sqrt(2 tint) × `HDRConstants.x` with the texel alpha.

## Vehicles

`VehicleGraphicsSpec` (0x106) assembles a car: body part models at the origin, plus four wheels with
position, quaternion, scale and 2–4 part models each (tyre, rim, disc, caliper; a bitmask marks which spin).
Right-hand wheels reuse the left-hand models mirrored on X. Layout tables are in `mw-formats/src/vehicle.rs`.
It also holds the car's **locator groups** (+0x1C count, +0x20 ids, +0x24 `{n, locators, ?}`; a locator is a
quaternion and a position): where its coronas, lights and spot effects (`VehicleVfxBehaviour`) sit. The 911's
group 137267 holds the four tail pipes, turned half a turn about X so +Z leaves each pipe backwards.

### Vehicle damage (deformation)

There is no dedicated deformation resource. A car's body is **skinned to a small skeleton** and damage is a
set of **skeletal animation clips whose frames are damage levels**:

- `VehicleGraphicsSpec` +0x04 imports the `Skeleton` (0xB2, `CgsAnimation::AnimSkel`): 33–47 bones per car.
  Bone positions are model space (they sit on the body), rotations are mostly identity (every car's steering
  wheel is tilted, a few flaps too; the game never reads them), parents precede children. Bone IDs are CRC-32s of names that are not shipped (not `field_hash`, zlib or BZIP2 CRC of
  obvious names).
- Body and glass vertices carry `BLENDINDICES` (skeleton bone indices directly) and `BLENDWEIGHT`; the vertex
  shaders ignore weight `.x` and use `1 − (y + z + w)` for the first bone, then skin with `gBoneMatrixArray`
  (48 × 3×4 matrices). Checked on the 911: 28,893 body vertices, none outside the skeleton, and each bone's
  vertices cluster around it (doors, mirrors, bumpers, lights, spoiler, steering wheel).
- The `VehicleDamageBehaviour` GeneSys object's `AnimationList` handle points to the damage `AnimationList`
  (0xB0): 8 clips (7 on one car) in a fixed zone order **front, rear, left, right, front-left, front-right,
  rear-left, rear-right** (+X is the car's left). Each clip has 21 frames (two cars have one clip of 5 or
  202): frame 0 is the undamaged car, the last frame fully wrecked. Translations are offsets from the bind
  position in the parent's space; rotations are applied in the bone's own space. Frame 0 is the identity on
  almost every bone, but 30 cars have a few bones whose keys carry a constant offset in every frame, from a
  rig that differed from the shipped skeleton (XKR boot lid and tail lights 12–14 cm in all 8 clips; R34
  steering wheel 77 cm towards the left-hand-drive side in the front-left clip; Viper 0.57 m, F1 LM 0.38 m;
  `mwtool damage all` lists them, `mwtool damage <car> --keys` prints the keys); the game's blend uses keys as
  changes from the clip's reference frame (`Anim` +0x36, 0 here), so these cancel (decomp.md "Vehicle damage
  pose"). A part breaking off gets
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

- Liveries and two-paint masks are bound in the car's Material as `DiffuseTextureSampler` of the `VehicleNFS13_BodyPaint_*Livery*` / `*TwoPaintMask*` shaders; `mPaintColourIndex` (material constant, 0 / 1) picks the secondary paint on `_TwoPaint` meshes. `VehiclePaint` carries `DustColour` / `DirtColour` (the paint shaders' `GenericVector4_B` / `_C`, in that order), `PbrScratchColour` and the scratch settings (`PbrScratchRoughness`, `0802d981` = scratch F0), no texture.
- Badges, logos and decals are `VehicleNFS13_*_Alpha1bit_*` meshes: the G-buffer program discards where the mask's
  alpha < 0.5, the mask being `DiffuseTextureSampler` (`_Textured`) or else `NormalTextureSampler`; blend off
  (MaterialState `01000006d979a79a`, as opaque parts). `VehicleNFS13_*_Alpha_*` meshes have no G-buffer pass: forward,
  alpha-over (`010000063eb1486e`, rt0 `7908498b`), alpha = the diffuse alpha; `Wheel_Alpha_Textured_Normalmap`
  discards below 0.125 and `_BlurFade` scales the alpha by 1 − `g_wheelConstants.x` (research log 2026-10-09).
- `Genesys.Gen.VehicleColourPalette` (one per car bundle): `Colours` (`16671e74`, list of `VehiclePaint` handles) and `ColourGroups`, each a `ColourGroup` { `PrimaryColour` (`2968e77a`), `SecondaryColour` (`750cf08e`), `WheelColour` } of paint handles; `17acabdb` / `db17b169` unnamed (1 on police cars). A secondary paint has `PaintColour` 1. `DustColour.w` / `DirtColour.w` scale the dust / dirt levels (`FUN_008e9c00`, docs/decomp.md).
- `Genesys.Gen.SurfaceVehicleEffects` (72 bytes; the `VehicleEffects` handle of a `SurfaceType` weather block, `8db39261` at block +0xc8): `Dirt` (`44697274`, +0x0c) and `Dust` (`44757374`, +0x24) `Effect`s of (`ea1c1262` rate, `0979a504` time in s), `0d9ece32` full-effect speed (mph, 120 or 60), `f8bbba83` start speed (30 or 15). Four objects in `HAWAII/SURFACELIST.BNDL`: off-road (0.3 / 0.3), road (dirt 0.005, dust 0.05; the dry asphalt block has dirt 0), wash (−0.5 / −0.5, 15–60 mph: surfaces 27 and 43) and dust only (0 / 0.05). `mwtool surfaces` lists them per surface.
- `Genesys.Gen.VehicleInfo.76bfd668` (float, +0x84): the zone damage → paint scratch scale (10 on 173 cars, 1 on 24).

### Vehicle animations (active aero)

Moving body parts (spoilers, air brakes, flaps, vents) are `Animation` clips on the car's damage `Skeleton`,
started by a state machine. `Genesys.Vehicles.AnimationStateBehaviour` (44 bytes): +12 label, +20 UniqueId,
+24..26 three bools, +28 `0501d88b` handle → `HSMData` (0x501), +36 `Resources` (`3f227e59`) handle array →
the `Animation` clips, +40 their count. 16 models (18 `_HI` bundles) have one (`mwtool damage <car>` prints it): one clip for the
911 Carrera S (both bundles), LFA, SL65 Black Series (both), SLS, Panamera Turbo S, 918 Spyder, Agera R
(`gas`), Marussia B2 and Aventador J (vents with timers); several for the 918 concept and Aventador
LP700-4 (spoiler + vents), McLaren MP4-12C and Venom GT Spyder (spoiler + air brake), both Veyrons (handling /
top speed / air brake) and the Huayra (front / rear flaps on speed, steering and brake).

A clip is two (or a few) frames: frame 0 is the retracted pose (identity keys), the last the deployed one;
keys are offsets from the bind pose (the Veyron's transition clips, e.g. into the air brake, start from a
deployed pose, so the game probably layers them; its aero player is not identified), on 1-4 bones (the 911's bone 36: 30° about X and
2 cm up / 7 cm back). It plays at its own frame rate: the 911's 0.612 frames/s makes the deploy 1.63 s.

The `HSMData` is a serialized property tree (apparently hashed keys with values as strings; layout not decoded).
Its strings name states (`Spoiler Anim`, `Brake from Top Speed Anim`, `Reverse … Anim`, `Wait 0.4s`),
conditions (`Speed above 70mph`, `Brake ON and Speed above 80mph`), their operands as decimal text (`70`,
`40`), the variables `speedinmph` / `brake` / `steering` / `gas`, a clip by its resource id in decimal
(`904078` = `0dcb8e`), and the node types `HSM.AnimationCondition`, `HSM.AnimationState`, `Wait For Time`.

`Genesys.Gen.PhysicsDrivenVehicleAnimationBehaviour` is a different thing: exposed suspension of the Atom,
Mono, Caterham and Zonda R (`AnimatedBodyPart` = bone name `FrontLeftSuspension` / `…WheelAngle`, wheel
index, `ApplyPosition` / `ApplySpin` / `ApplySteering` / `Steering` flags), no aero.

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
saturate(ScatteringSettings.y/(1 + g·cosθ)²) × t × gradient alpha, with ScatteringSettings = (g, (1 − g²) ×
ScatteringStrength / 4π) and g = ScatteringPhase (decomp.md "Fog"); sky pixels only get the layer fog.

**Water** (`Water_Opaque_Singlesided`, `Water_Proto_Cheap`) is opaque and writes the four G-buffer targets
like the road (no forward or blended pass; fog comes from the Fog pass). Vertex layout of the opaque family:
TEXCOORD0 (diffuse, SpecDepthAndRefraction, SurfMask), TEXCOORD1 (waves, surf), TEXCOORD2 (river floor,
refraction lookup), TEXCOORD3 = packed tangent frame (no vertex colour). Its programs:
- waves: NormalTextureSampler at TEXCOORD1 × normalMapTile1 / 2, each scrolled in v by Time / waveAnimationSpeeds.x
  / .y, averaged; surf: SurfNormal replaces that normal by Surf.y × SurfMask.y, at (TEXCOORD1.u × surfTile.x,
  TEXCOORD1.v × surfTile.y + Time / surfAnimationSpeed + 0.05 Time − R), where R = SpecDepthAndRefraction.w
  sampled at 2 × TEXCOORD2 + (0, Time / 2).
- SpecDepthAndRefraction at TEXCOORD0 = s: roughness = lerp(specularAndRoughness.y × s.x,
  specularAndRoughness.y, surf); s.y = floor visibility; s.z = algae.
- albedo = lerp(lerp(D × F, algaeColour, s.z), 1, surf), D = DiffuseAndCaustics.rgb at TEXCOORD0,
  F = saturate(RiverFloor(TEXCOORD2 − R/4) × s.y + DiffuseAndCaustics.w(world xz / causticTile + R) × s.y)
  (caustics on the floor). RT2 = specularAndRoughness.x as F0, Fresnel 1.
- RT3 = cubeSky(R′, roughness × 5.5)² × 4 × (Refl.rgb + Refl.a) × LUT × (1 − s.y) × exposure, where R′ =
  reflect(−V, N) with R′.y ≤ 0.005 replaced by |R′.y| + 0.01, Refl = the material's ReflectionTextureSampler
  cube at R′ (a 128² skyline cube: rgb = grey mountain silhouette, a = 1 above it), LUT = AmbientSpec env-BRDF
  read at (dot(−V, N), roughness, F0), i.e. at a negative N·V on visible water: the clamped grazing column, so
  the reflection is a near-mirror whatever the angle.
- `Water_Proto_Cheap` (9 meshes, all far from roads): waves at TEXCOORD0 only, albedo 0, roughness = .y, the
  same RT3 reflection without the 1 − s.y factor.

### SamplerState (0x7)

64 bytes, bound per material slot next to the texture (`Material` 0x28). The game turns it into a
`D3D11_SAMPLER_DESC` field by field (the function at 0x00d8e0c0 in the dump, which then calls the device's
`CreateSamplerState`), so the layout is settled (parser: `mw-formats/src/sampler.rs`):

| off  | field |
|------|-------|
| 0x00 / 0x04 / 0x08 | AddressU / V / W, D3D11 numbering (1 wrap, 2 mirror, 3 clamp, 4 border) |
| 0x0C / 0x10 / 0x14 | mag / min / mip filter: 0 point, 1 linear, 2 anisotropic. Any field at 2 makes the sampler `D3D11_FILTER_ANISOTROPIC`, else filter = min<<4 \| mag<<2 \| mip |
| 0x18 / 0x1C | MinLOD / MaxLOD (−FLT_MAX / FLT_MAX in all 2742 instances) |
| 0x20 | MaxAnisotropy (1, 2, 4, 6, 8) |
| 0x24 | MipLODBias (f32: 0, −0.9, −1, +0.9, +1, ...) |
| 0x28 | comparison function, −1 = none (D3D11 ALWAYS, no comparison bit) |
| 0x2C | u8 border colour: 0 = transparent black, else opaque white (bytes 0x2D.. are not read) |
| 0x34 | runtime `ID3D11SamplerState*` |

`mwtool sampler-usage [--filter <shader>]` lists the states per shader slot over all track units
(`--textures`: the bound textures' format and mip count). Ground slots: every DriveableSurface `ColourMap`,
`DetailMap_Diffuse`, `DetailMap_Normal` and `Normal` sampler is wrap / 4× anisotropic / bias 0; `RoadDepth`,
`Line_*`, `Decal_NormalPlusSpec` and most `SpecColour` are wrap / trilinear / bias −0.9 (some `SpecColour`
+0.9); TerrainPBR / PlotPBR tiling maps are either 4× anisotropic or trilinear / −0.9 per material. Every
ground slot wraps; mirror / clamp appear only in 37 non-ground slot states.

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
  Reversing, six unnamed sequence lists (four are the gear changes: +0x50 `83ea3a92` upshift / +0x48 `ce819b08`
  downshift with the race gearbox, +0x54 `9492327c` / +0x4c `c342c096` with the cruising one; their counts are the
  bytes +0xfc / +0xfa / +0xfb / +0xf9), Backfire_Sequence, Gurgle_Sequence (bound to
  GetPopProbability), Pop_Sequence (bound to it too; started on a pop), Ignition_Sequence, c62d11ac (rev limiter), mixer channel list, DspParam
  lists (per-car plug-in overrides), Gains (per-car channel trims), MasterGain }`. Unnamed `EngineSound2`
  floats the game's code reads (research log "engine sound from the game code", decomp.md "Engine sound"):
  `99d4b91e` load response rate (× 60 per second), `a33d84fe` / `cb44d82d` / `913bb900` RPM-curve terms (full load /
  off load / alternate; normalised RPM = ((idle..max fraction − 0.05) / 0.95)^(2 − a); audible RPM = the span of
  the Ginsu items of `Sequences`, min `StartTime` .. max `EndTime`, at that fraction), the rev limiter's cycle
  rate `6e6be213`..`9529a29e`, depth `18adb27c`..`f81e0914` and floor `a1049379`..`34c8d45a`, `283e07de`
  sub-harmoniser, `7cbc11ca` / `7cbc11c9` tracked
  harmonic 1 / 2 and `dbcddcfc` tunnel harmonic as multiples of the firing frequency (RPM / 120 × cylinders
  `ed754b99`), `11a77b36` / `64cbe1ff` tunnel gain / Q, `f0291dd3` comb-rasp delay divisor. `DspParam
  { Mixer_channel, PlugIn (unique id), Value, Parameter_name }`: index 1 on a filter reads as Frequency, 0 on a
  Send as its gain; 4 (Delay) and 5 (BandPass) are unresolved.
- **`VehicleSound`** (resource type 0x207; one per car and role, id = CRC-32 of `<id>_VEHICLESOUND` for the
  player's car, `<id>_COMPETITORSOUND` for AI racers and traffic): +0x10 `TyreSound` import, +0x18 / +0x1c offset /
  count of a handle list (player's only), +0x48 `EngineSound2`, +0x50 `TransmissionWhine` (player's only), +0x60 /
  +0x64 offset / count of the horn sequences (imports every 8 bytes). `mw_assets::car_sound`.
- **3D placement**: `MixerChannel { DistanceFalloff (+0x0c), d6365a79 emitter directivity (+0x28), Focus (+0x2c,
  listener), 435ec694 doppler mode (+0x54: 1/2/3 = 680/340/170 m/s) }` → `SoundDistanceFalloff { CurveType
  (+0x10) and d13efb2a (+0x14) exponents, InitialGain (+0x28) at 0 m, PeakGain (+0x50) at MinDistance (+0x40),
  0 at f5bece87 (+0x34); a second curve 1ede038c (+0x54) at 7d704038 (+0x48) to 04f8e36b (+0x58) at fc29b05f
  (+0x3c); +0x18..+0x30, +0x38, +0x44, +0x4c feed filter-like outputs }` (decomp.md "Other cars' engine sounds").
  `TrafficEffectSequenceItem` has the layout of `RaceCarEffectSequenceItem`; property 0 is the traffic RPM.
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
`VfxParticles_UVDistortion` (the exhaust smoke's) is the diffuse sprite with its UVs offset by a scrolling
distortion texture (`UVDistortionScrollScale`); drawn without the offset here.

**Action kinds** are indices into the game's action table (`.data` 0x010c2d48, names from `.rdata` 0x00f9d664):
00 `EmitterConePlus`, 01 `EmitterCollisionMesh`, 02 `EmitterNull`, 03 `EmitterTrail`, 04 `EmitterVolume`,
05 `EmitterWheel`, 06 `EmitterWheelContacts`, 07 `EmitterWheelContactPlus`, 08 `EmitterWheelTrack`, 09
`EmitterWheelTracks`, 0A `Initialise2dRotation`, 0B `Initialise3dRotation`, 0C `InitialiseAgeAndLifetime`, 0D
`InitialiseAlpha`, 0E `InitialiseAndSubmitLight`, 0F `…LocalSpace`, 10 `InitialiseBakedLighting`, 11
`InitialiseCollisionRadius`, 12 `InitialiseRadius`, 13 `InitialiseBouncePlane`, 14 `InitialisePosOffset`, 15
`InitialisePosVelToWorldSpaceWithLocalTransform`, 16 `InitialiseVelocityInheritance`, 17
`ParametricApplyPerParticleAlpha`, 18 `ParametricEmitterVolume`, 19 `…SubUV`, 1A `ParametricPosVelToWorldSpace`,
1B `…WithLocalTransform`, 1C `ParametricSetColourGradient`, 1D `ParametricSetSubUV`, 1E `Render3dSprite`, 1F
`Render3dSpriteSubUV`, 20 `RenderCubeMapSprite`, 21 `RenderLightBeam`, 22 `RenderMesh`, 23
`RenderMeshNoPaintColour`, 24 `RenderSprite`, 25 `RenderSpriteMotionBlur`, 26 `RenderSpriteShadowMapSubUV`, 27
`RenderSpriteSubUV`, 28 `RenderSpriteUVDistortion`, 29 `RenderSpriteVelocity`, 2A `RenderTrail`, 2B
`SuperEmitter`, 2C `Update2dRotation`, 2D `Update3dRotation`, 2E `…BasedOnVelocity`, 2F `UpdateAge`, 30
`UpdateAndSubmitLight`, 31 `UpdateAxis`, 32 `UpdateAxisLocalSpace`, 33 `UpdateBindings`, 34
`UpdateBindings_Wheel`, 35 `UpdateOccluders`, 36 `UpdateParametric2dRotationAlignToVelocity`, 37
`UpdateParametricColour`, 38 `UpdateParametricFadeOverDistance`, 39 `UpdateParametricLighting`, 3A
`…WithOccluders`, 3B `…WithTunnels`, 3C `UpdateParametricRadius`, 3D `UpdatePosVel`, 3E
`UpdatePosVelWithBouncePlane`, 3F `UpdatePosVelWithVortex`, 40 `UpdatePosVelWithWorldCollision`, 41 `UpdateTrail`,
42 `UpdateWheelBindings`, 43 `RenderSpark`, 44 `UpdateBucketBoundingBox`, 45 `FrustumCullBucket`, 46
`FreezeOffscreen`, 47 `InitialiseRandomSeed`, 48 `UpdateParametricRandomTint`, 49 `PackVelocity`, 4A
`UnpackVelocity`, 4B `InitialisePackedVelocity`, 4C `EmitterSplatter`, 4D `RenderSplatter`.

Decoded from the game's code (research log 2026-10-09 "exhaust effects"; layouts in `mw-formats/src/bear.rs`):
- `UpdateBindings` (0x33 / 0x34; 6 input words, curves at +0x18, scales +0x40, offsets +0x50, then the source,
  `a`, `b` and op bytes at +0x60 / +0x64 / +0x68 / +0x6c): four curves over chosen inputs into the layer's
  binding slots 1–4, four combine steps into 5–8 (`Bindings`). Slot 0 is never written.
- `EmitterConePlus` (0x00): windows on the layer time (+0x10), the squared camera distance (+0x18) and the
  velocity's length (+0x40); `IsEmitting` operand +0x50; binding slots for the rate, burst, speed and cone angles
  in the bytes at +0x70 (`ConeEmitter`). Its particles leave along the emitter's +Z (inferred, not decoded).
- `ParametricSetColourGradient` (0x1C): rows at +0x20, ramp `A` / `B` at +0x70 / +0x80, alpha `Curve5` at +0x90,
  alpha slot +0x9a, tint flag +0x9b, colour / alpha rates +0x9c / +0xa0 (`Op::ColourCurve`): rgb = c.rgb × c.w.
- `InitialisePosOffset` (0x14) + `ParametricPosVelToWorldSpace` (0x1A, or 0x1B with its own rotation and offset
  at +0x00 / +0x10): an emitter-space layer.

**Car spot effects**: `VehicleVfxBehaviour` (one per car bundle) lists `Coronas` (0x50 bytes), `Lights` (0x40)
and `SpotEffects` (0x20: `SpotEffect` handle, `LocatorGroup`, `Group`, `da6ac384`, `Looping`, `Mirror`; counts
`9f8c05b0` / `e41cffe1` / `4c52a051`). `LocatorGroup` ids resolve in the car's `VehicleGraphicsSpec` locator
table. `Group` is a bit of the car's effect mask (decomp.md "Exhaust pops and backfires"); every car carries the
same set, from `GLOBALEFFECTS.BNDL` (`mwtool genesys all --type VehicleVfxBehaviour`):

| Group | Effect | Layers | Plays |
|-------|--------|--------|-------|
| 19 | `…24f2f` exhaust smoke (one car puts it on group 0, one adds `…f5859`) | 1, `RenderSpriteUVDistortion` | while the engine runs and the car is below 6 m/s; rate over the player's engine load |
| 8 | `…24f34` nitrous flame | 3, emitter-space, white → deep blue (brightness 2 → 8) | while nitrous burns |
| 14 | `…24f2d` backfire | orange glow, blue / yellow flame flickers (0.5 s window), a light | two updates per backfire |
| 9 | `…16205b` nitrous flame + backfire flames | 7 | nitrous mode 1 (+0x195b) |
| 20 | `…22030b` at the pipes, `…220308` at locator 1207805 | 1 each | nitrous mode 3 (+0x195b), more conditions |
| 12 | `…24f5a`, `…14023f` at the car's centre (locator 1311228) | 5 / 8 | wrecked |

- **Tyre marks**: `SurfaceVfx.TyreMarksMaterials` (`75cb5de9`) = { `eff16a09`, `Offroad`, `Road`, `Sport`, `Track` }
  `Vfx_TyreMarksNew` materials (tyre class), plus `SurfaceVfx.RimMarksMaterial`. Each material: `RunDiffuse`
  (rolling; transparent on tarmac), `SlipDiffuse` (sideways), `SpinDiffuse` (wheelspin / lock) layers, `WidthScale`
  (0.65 on tarmac), `Animation`, `Specular`. Per car `TyreVfxBehaviour.{Front,Rear}_Tyres.Skid_Marks` holds the
  intensity curves (`Lat_Slip`, `Long_Spin`, `64222c3f`; mostly unnamed). Shader: research log 2026-10-06 round 4.

## World object physics (`PhysicalDefinition`, `DynamicAdditionalInfo`, collision responses)

`mw_assets::props`; how the game uses the values: decomp.md "Props".
- `PropObject` +0x0C `PhysicalDefinition`, +0x14 impact effect (a `BearEffect`, 81% of props), +0x1C impact
  sound (a `WCPlaySoundBehaviour`: `Collision_Waves` handles, per-surface `Surface_Collisions`), +0x24
  `mfResetDistanceSquared` (10000 everywhere), +0x28 `mbRemoveInsteadOfReset`.
- `PhysicalDefinition`: +0x10 bounds centre, +0x20 half extents, +0x30 `AdditionalInfo`, +0x40 `RigidBodies`
  (0xe0 bytes each: +0x10 transform, `ComOffset`, `Inertia_Scale`, bounds, +0x98 `AngularDrag`, +0x9c
  `44ac37c1` (restitution), +0xa0 `Friction`, +0xa4 `LinearDrag`, +0xa8 `Mass`, volume arrays), +0x48 the
  main body's index.
- `DynamicAdditionalInfo` (`CompoundAdditionalInfo` adds +0x2c `d7f803ec`, +0x30 `ForceToSmash`, +0x34
  `08c87eeb` = seconds before a loose compound may smash): +0x0C `Sequence`, +0x18 `ForceToDamage` (N, contact
  force), +0x1C `67f0233c` (momentum kg m/s that knocks the object loose), +0x20 `Strength`, +0x24 `Score`,
  +0x26 `5586d09d` (may come loose), +0x27 `c2315b63`, +0x28 `94c94bab` (starts pinned; true exactly where
  `67f0233c` > 0).
- `CollisionResponses_RaceCar` (`010000000007f0c9`) `RaceCarVsDynamic` ×2 (+0x3d0 AI, +0x40c player):
  `BasicParams` (`e5ea1414`, `e3882eb1`), `1caa1e4f` object angular scale, `357a73bd` object linear scale,
  `207258c8` car angular scale, `e75eb28b` car linear scale, `Restitution`. `CollisionResponses_Global`
  (`…f0c6`) `TrafficVsDynamic` (+0x9c: `16c25fa9` / `31e69a76` the traffic car's linear / angular scales).

## ColourCube (0x52)

16-byte header: u32 edge N (16 everywhere), u32 N, 8 zero bytes; then N³ three-byte entries laid out as a D3D
volume texture without alpha: input red fastest, then green, then blue, each entry stored B, G, R (the output
for input (r, g, b) at 3 × (r + N g + N² b)). Gamma-encoded in and out; `…3e93` is the identity (symmetric, so
it does not decide the order). The order follows from how the game uses the bytes (blended and copied
unchanged into a B8G8R8A8-ordered volume sampled at x = red; research log 2026-10-06, inferred: the blend
kernels are missing from the dump). Until 2026-10-06 it was read as blue fastest with R, G, B entries, which
swapped red and blue and caused the yellow cast. Each environment keyframe names one (`ColourCube` handle);
PostFX states (`POSTFX.BNDL`) name more. Parser: `mw_formats::ColourCube`.


## TrafficLaneData (0x701)

One instance in the install: `HAWAII/TRAFFIC.BNDL` (`000000001fe4ff4c`, 2.3 MB). Parser:
`mw_formats::TrafficLaneData` (full layout table in `crates/mw-formats/src/traffic.rs`); `mwtool traffic`
prints counts, flows and vehicles, `--png` draws the lanes. Burnout Paradise's `TrafficData` is the ancestor,
but the record sizes and the flow / vehicle tables differ.

- **Header**: `u16` version 3, `u16` hull count 294; `+0x0C` pointer array (one per hull) to the hull lane
  data; `+0x10` / `+0x14` import list of 34 `TrafficFlow` GeneSys objects (Speed in mph, Density in cars per
  minute per lane at multiplier 1 (`FUN_005fb120`), FlowType);
  `+0x18` / `+0x1C` 23 `TrafficFlowType` objects (vehicle handles with `Proportion`); `+0x24` vehicle type count
  11, `+0x40` their `VehicleComponent` ids (low 32 bits, objects in `VEHICLES/TRAFFICATTRIBS.BNDL`);
  `+0x54` the hull cells.
- **Hull cell** (0x50): ~400 m quad (`[x, z]` points), up to 10 neighbouring cells (`u16`), AABB with
  infinite Y. The player's cell plus its neighbour list is the active traffic area (`FUN_005f0270`).
- **Hull lane data** (0x40 header): counts of sections, spans, junctions, stop lines, neighbours (all `u8`),
  static vehicles (`u8`), rungs (`u16`), then 8 pointers: sections, rungs, cumulative rung lengths,
  neighbours, spans, static vehicles, junctions, stop lines. Empty hulls share one zeroed record.
- **Section** (0x30): first rung, rung count, stop line, span, up to 3 forward and 3 backward links (hull,
  section), first neighbour and left / right neighbour counts, length (m), `TrafficFlow` index (`+0x24`,
  confirmed in `FUN_005f4d10`), `TrafficFlowType` index `+0x25` (confirmed: the spawner passes it to the
  vehicle picker `FUN_00608c80`). Adjacent lanes of one road are separate
  sections with their own flow.
- **Rung** (0x20): two `[x, y, z, 0]` points across the lane, ~4.5 m apart; cumulative lengths restart at 0 in
  each section. **Neighbour** (4): section, last rung. **Span** (12): max vehicles, its reciprocal, `u16` hull
  and `u8` span it continues into (0xFFFF none; the route choice sums the load along that chain,
  `FUN_00618470`).
- **Stop line** (2, confirmed in `FUN_00439b60`): a `u16` rung position in 8.8 fixed point (rung in the high
  byte, 1/256 fractions in the low byte). Section `+0x05` is the first stop line, `+0x06` how many.
- **Junction** (0x150, confirmed in `FUN_00605d30`, the light cycle): `u16[16]` phase durations at 0x04 in
  tenths of a second, `u8[16]` phase masks at 0x24 (bit k set: light group k holds its stop lines, i.e. red),
  `u8` phase count at 0x34, `u8` group count at 0x35, groups of 0x20 bytes from 0x40: `u16` light object ids
  at 0x00 (count `+0x17`), `u8` stop line indices at 0x04 and their `u16` hulls at 0x0A (count `+0x16`). All 189
  stop lines are referenced by exactly the junction groups, each owned by a section. Typical cycle: 15 s / 2 s
  all red / 15 s / 2 s all red.
- **Static vehicle** (0x50): a matrix, then (from `FUN_0060db90`) `u16` flow type at 0x40, `u8` spawn chance in
  percent at 0x42, flags at 0x43 (bit 0: not during a pursuit). Parked cars; not used yet.

## AIData (0x200)

`HAWAII/AI/AIDATA.BNDL`, one resource (599 168 bytes, no imports); parser `mw_formats::ai_data` (layout table in
its doc comment), checked by `mwtool verify` (links symmetric, every node in its cell). Names from burnout.wiki's
symbol-derived "AI Data (NFS Most Wanted)" page; every offset was checked against the data and the decomp's
readers (decomp.md "AI racers"). Not Burnout-style AI sections: a graph of road centre-line nodes.

- **Node graph** at header +0x10: 9153 nodes of 0x28 bytes, sorted into a 41 × 77 grid of 125 m cells from
  (−2573.4, −4304.4) (cell = `i16` count, `i16` first node). A node: up to 6 linked nodes (symmetric; link count
  at +0x26: 2 on a road, 3–6 at a junction, no dead ends), position, a side vector across the road (y = 0, not
  normalised), "fast" and drivable widths in 0.25 m units (median drivable width 9.5 m, 95 nodes 0 and skipped by
  the path builders), flags (0x1 shortcut / secondary road on 48 %, 0x8 / 0x10 / 0x20 around jumps).
- **Junctions**: 1160 records (0x30), one per node with a link count other than 2: per exit a cost (= edge
  length / 200 m × 511, to 1 %), the target junction, an angle (× 2 degrees, folded to 0–180°) and flags (0x1
  shortcut, 0x4 jump); a junction id and a road section id (0 in the file). Exits can be fewer than the links
  (one-way and jump chains).
- **Node mappings** (0x10 per node): the chain distances / 200 to the two end junctions and their indices.
- **Jump links**: 284 × two points (position, a half-width vector, a zone id filled at load, a traversable byte).
- Spans x −2573…2451, z −4304…5198, 274.8 km of links (median link 27 m). No speeds and no racing lines: the
  game builds those at runtime.

## TriggerData (0x202)

One instance, `HAWAII/TRIGGERS.BNDL`, version 8; loaded by `FUN_0053c2f0`. Parser `mw_formats::TriggerData`
(layout table in `crates/mw-formats/src/trigger.rs`), listing `mwtool freeroam`.

- Header: version (+0x00); boxes ptr/count (+0x04/+0x08, 0x30 bytes each), spheres (+0x0c/+0x10, 0x18 bytes),
  points (+0x14/+0x18, 0x20 bytes); spatial index ptr (+0x1c): per-cell lists of trigger indices over a
  45 × 39 grid (+0x20/+0x24), origin x, z (+0x28/+0x2c), cell size x, z (+0x30/+0x34, 250 m); cell index =
  `dimX × cz + cx` (read by `FUN_0053bc70`, which streams the cells around each player).
- Every record: position `f32[3]`, id `u32` (+0xc; unique), kind `u8` (+0x10), shape `u8` (+0x11, 0 box,
  1 sphere, 2 point, always matches the array), flags `u8` (+0x12; bit 0 = one-way box, counted only when
  the car's vector points along the box's local Z, `FUN_00536f70`; bit 1 unread by the trigger code). Box:
  quaternion x y z w (+0x14), size `f32[3]` (+0x24, **full** extents: the physics shape gets size × 0.5,
  `FUN_00539e50`). Sphere: radius (+0x14). Point: unit direction (+0x14); points get no physics shape.
- The code indexes the three arrays as one list in that order and keys it by id. Kind codes of the free-roam
  collectables (`FUN_006e4920`): '0' security gate, '2' billboard, '4' speed camera, '<' jack spot,
  '=' jack spot car, 6 body shop (letters 'A'..'O' = the same in other content packs). Base game: 152 / 216 /
  74 / 183 / 185 / 22. Rules in decomp.md "Free-roam rules".

## Events (`OfflineEvent`, GeneSys in `GAMELOGIC/GAMEMODES.BNDL`)

Reader `mw_assets::events` (layout table in its doc comment), listing `mwtool events`, one event with its
route drawn over the lane map `mwtool events <id> --png <file>`. Settled 2026-10-09; every reference resolves.

- **Where**: no event is its own resource. `GAMELOGIC/GAMEMODES.BNDL` holds one 240 KB GenesysObject
  (`0100000000045d5b`, untyped root `{655d3f5f, Item[]}`) whose polymorphic `Item` array carries 111
  `OfflineEvent`s, 143 `OnlineEvent`s, 12 `GameMode`s, `EventArena`s, `RaceBalancingData`, `Challenge.*`, ...
  `GAMELOGIC/GAMEPLAY.BNDL` has the same kind of containers for `EventList` (the career list: 109 event ids),
  `Gameplay.AllowedVehicleList` (95), `AIPlayerType` (200), `EventLocation`, `CarSwapSpot`, `GameplayTrigger`, ...
  References are `CgsCore.UniqueId` (u32) values matched against each item's `655d3f5f`, not resource IDs.
- **`OfflineEvent`** (248 B; the first 76 B are the `Event` base): id `655d3f5f` (+0x18); `GameMode` (+0x1c,
  730000 Race, 955569 Speed Run, 390185 ambush / pursuit modes, 390145 free roam, 2173009 / 2210000 DLC modes);
  `TypeName` (+0xa4, a language id: 979287 "CIRCUIT RACE", 979289 "SPRINT RACE", 457003 "MOST WANTED", 1472095
  "SPEED RUN", 1119989 "AMBUSH", 2193051 "DRIFT ATTACK"); `Name` (+0x84) and `Description` (+0x14) language ids;
  `Checkpoints` (+0x64, UniqueId[]); `SpawnPosition` (+0x9c); `Laps` (u8 +0xf6: 1 for sprints, 2–3 for
  circuits; "AROUND THE WORLD" is a circuit with 1 lap); `AllowedVehicleList` (+0x5c); `TargetSpeed` /
  `TargetTime` (float[3], speed runs / timed DLC modes); `1795ca2a` `AIPlayerInformation`[] (+0xc0:
  `AIPlayerType`, `9b4e35f7`, `Placement`, `2968e77a`); `c0756b63` `CustomChevron`[] (+0xb8); `d4b38a64`
  `AlternateRoutes`[] (+0xc4: `Checkpoint`, `fe6ef8d8` UniqueId[]); `CheckpointInfo`[] (+0xc8: `Sequence`,
  `Type`, two bools; on 7 events, the count rule gives more entries than checkpoints on some: open);
  `NitrousAllowed` (+0xec), `TrafficEnabled` (+0xf0); `RaceBalancingData` / `RaceBalancingProfile` ids
  (items of GAMEMODES). Career content: 22 circuits, 36 sprints, 14 Most Wanted races (one opponent, no car
  list: the player's own car), 18 speed runs, 7 ambushes, plus DLC drift attacks and timed modes.
- **Checkpoints and starts are `TriggerData` records** (`HAWAII/TRIGGERS.BNDL`, same ids): every checkpoint is a
  kind-0 **box**, 2 m deep, 36 m high (centre 18 m above the road), as wide as the road or the junction (27–186 m),
  its local Z the driving direction through it. The list is in driving order; on circuits the last gate stands
  next to the start (the start / finish line), on sprints it is the finish. `SpawnPosition` and each opponent's
  `Placement` are kind-4 **points** (position on the road, unit direction = the car's heading). In
  "CONTINENTAL DRIFT" four opponents start ahead of the player (42, 29, 15 and 4 m along the road) and four
  behind, the slots 11–14 m apart, alternating a few metres across the road.
- **`CustomChevron`** (36 B): `655d3f5f`, three float arrays `c6bc8827` / `c27d9491` / `cf3eb14b` (x, y, z of a
  polyline, 2–4 points on the road's edge at junctions the route passes), their counts, a bool `f8ccf73e`:
  inferred, the lines of arrow boards that close side roads.
- **`Gameplay.AllowedVehicleList`**: `193ad61b` `VehicleAndMods`[] (`Vehicle` = vehicle list id, `a17a5e72`
  float 1.25–1.35, `Mods` u8[2], `TargetScores` / `507d336c` float[3], `Difficulty`). The first entry is the car
  the event is offered with (the car whose five events it is); `--race` uses it.
- **`GameMode`**: `09d6656d` (+0x44) = 10, the crash reset delay `FUN_006d39d0` reads (decomp.md "Crashes"; the
  earlier note that no `GameMode` instance exists on PC is wrong: they are items of the GAMEMODES container).

- **Sequence ids** (names cracked 2026-10-09): `OfflineEvent` `AROutro` (+0x60), `EventIntro` (+0x70),
  `EventOutro` (+0x74), `FeedbackMessageGroup` (+0x78), `IntroMessageGroup` (+0x80); `GameMode` `AROutro` (+0x0c),
  `CountdownSequence` (+0x10), `HUD_HSM` (+0x20), `Intro` (+0x24), `ResultsSequence` (+0x34), `FeedbackGroupID`
  (u8 +0x75). Where those sequences live: "Sequences" below.

## Sequences (`Genesys.Gen.Sequence`, 2026-10-09)

Reader `mw_assets::sequence` (layout table in its doc comment); `mwtool events <id>` prints an event's intro and
outro item by item. Game-logic cutscenes (race intros and outros, the crash camera, tutorials) are `Sequence`s in
feedback message groups: `EN_US/FEEDBACKGROUPS/<UniqueId>.BNDL` holds one `FeedbackMessageGroup` whose
`Sequences2` lists them, plus everything they use (layouts, `Animation` / `Skeleton` clips, `Wave`s, post-FX
states). A sequence is referenced by its own `655d3f5f` UniqueId, never by resource id.

- `Sequence` (44 B): `Binding` (empty = a time axis in seconds), `655d3f5f`, `Binding_Min` / `_Max`,
  `SequenceItems` (polymorphic, +0x20), `TimelineControllers` (+0x24), counts. Every item starts with the
  `SequenceItem` base (36 B): `EnabledBinding`, `655d3f5f`, `StartTime` (+0x18), `EndTime` (+0x1c), a u16.
- Item types seen in the 108 race intros (by how many intros use them): `LayoutSequenceItem` 108,
  `BusMixerChannelSequenceItem` / `WaveSequenceItem` 106, `EventTriggerSequenceItem` 104 (no fields),
  `TeleportPlayerSequenceItem` 100, `DriveLocalPlayerWithAISequenceItem` 93, `AnimatedCameraSequenceItem` 92,
  `PostFXSequenceItem` 81, `HideHudSequenceItem` 74, `ClearTrafficSequenceItem` / `OverrideAvailableTraffic…` 61,
  `CameraSequenceItem` 59 (all ICE takes), `AddBehaviourToRaceCarSequenceItem` 51, `AddRaceCarSequenceItem` 26,
  `SlowMoSequenceItem` / `DynamicCameraSequenceItem` 25, `SpawnAICopCar` 21, `RaceCarEffectSequenceItem` 16,
  `MovieSequenceItem` / `DisableWorldRenderingSequenceItem` 15, then environment, time of day, speech and others.
- Field names cracked here: `AnimatedCameraSequenceItem` (240 B) `FocalDistance` / `FocalLength` / `FStop`
  (modulated values, +0x24 / +0x5c / +0x94), `6f3ea350` relative-to binding, `ShakeBinding`, `CameraAnimation`,
  `CameraSkeleton`; `TeleportPlayerSequenceItem` (112 B) `Direction`, `Position` (vec3), `37dea4a1` racer
  binding (`Players.LocalPlayer.RaceCarIndex`, `Players.Racer[n].RaceCarIndex`), `TransformBinding`, `Trigger`
  (a `TriggerData` point id), `Speed_MPH`; `DriveLocalPlayerWithAISequenceItem` `MaxSpeedMph`, `DrivingStyle`;
  timeline controllers `TriggerType`, `TriggerTime`, `DestinationTime` (`JumpTimelineController`,
  `JumpMonitorTimelineController`, `JumpNTimesTimelineController`, `QuitSequenceTimelineController`,
  `JumpOnButtonPress` + `Action`); `WaveSequenceItem.15a52404` = `Transform_Override_Binding`.
- `CameraSequenceItem.Camera` (60 B): `e996e4ce` / `44692821` strings, `Bystander_Params` handle (the crash
  camera), `0bd6ff75` bool[] (sample the binding once), `01130178` string[] (bindings → ICE avatars 1..),
  `7aef95c3` = the `CameraTake` UniqueId, `752a7ea9` f32, `650a0f6e` / `ca15aa55` u16, three bools,
  `Enable_Collision`.
- `AnimatedRaceCarBehaviour` (96 B): `Label`, three bools, three bone-name strings (`av_car`, `lv_car`, `car`),
  `Animation`, `Skeleton`, `DrivingControls` [{`BoneName`, `Control`, `VectorComponent`}], `Wheels` [three bone
  names], counts and flags. Its skeleton (18 bones, identity bind pose) animates bone 1 = the car (world space,
  codec-1 quaternions), 2–5 the wheels.
- Camera skeleton (`0x6cc42`): two bones, identity bind pose; clips animate bone 1 with f32 quaternions at 30 fps.

## ICE camera takes (`CameraTake` 0x20D, `CameraTakeList` 0x20E, 2026-10-09)

`CAMERAS.BNDL` holds 45 takes and the list ("IceList", loaded by `FUN_005c44b0`). Parser and evaluator
`mw_formats::camera_take` (full layout and rules in its doc comment), listing `mwtool camera-take`. A take is
named by its UniqueId (= the resource id's low 32 bits, e.g. 0x19c8b2 = 1689778 `Circuit_Race_Event_Outro_IC`).

- Header 0x20 bytes (count 1, end offset, ICE group id and key, pointer to the take at 0x20). Take: UniqueId (+0x08),
  name char[28] (+0x0c), length in s (+0x2c), 17 × (intervals u16, keys u16) per channel (+0x34), then per channel
  the key index of the inner intervals and the interval boundaries (fraction × 65535), then the element values
  (81 elements in a fixed order; floats as f32, the rest bit-packed MSB-first in 4-byte-padded blocks, decoded as
  signed / unsigned / hash / fixed point). Every one of the 45 takes parses to exactly its end offset.
- Channels: MAIN (eye, look, dutch, lens, spaces, avatars), LENS, EXPOSURE, RAWFOCUS, SHAKE, TIME, TAG, OVERLAY,
  LETTERBOX, FADE, ASSEMBLY, SHAKE_DATA, GAMEPLAY (game-camera blend), MOTION_BLUR, LAG, BYSTANDERCAM, STEREO3D.
- ICE axes: x forward, y left, z up (game = (y, z, x)). Behaviour: decomp.md "Cutscenes and sequences".

## UI blend states and sequence layouts

- **`MaterialState` (0x6)**, imported by a `Shader` (one per pass slot, at the program's slot + 4): u32 offsets of
  three blocks at +0 / +4 / +8 (blend, depth-stencil, rasterizer; seen 0x1c / 0x58 / 0x98), then the three states'
  hashes (+0xc blend, +0x10 depth-stencil, +0x14 rasterizer) and the size (0xc0). The blend block: one packed u32
  per render target (8), then the blend factor float4 (1, 1, 1, 1), then `00450000` (`00450100` when blending)
  and 1. The rasterizer block's second word is the D3D11 cull mode (1 none, 2 front, 3 back). `mwtool shader-asm`
  prints each pass's state. Packed target: bit 0 enable,
  bits 1–5 src, 6–10 dst, 11–13 op, 14–18 src alpha, 19–23 dst alpha, 24–26 op alpha, 27–30 write mask (D3D11
  enum values: 1 zero, 2 one, 5 src alpha, 6 inv src alpha; op 1 add). `0x7931498a` = disabled default,
  `0x7931498b` = alpha-over (`FlaptGenericShader`), `0x7911488b` = src alpha / one (`UIAdditiveShader`).
  A `UIMaterial` reaches it through `Material` (render material UniqueId) → `Shader`.
- **`Sequence`** (GeneSys): `Binding` (+ `Binding_Min` / `_Max`), `SequenceItems` (polymorphic). A
  `LayoutSequenceItem` shows `Layout` with `InstanceParams` from `StartTime` to `EndTime` (s), gated by
  `EnabledBinding`, with language ids `bf1ce9a1` (Text1), `Text2`, `Text3` that the layout reads as
  `UISequences.Params[#].TextN`, and a `LayoutLayer`. Loader `AssetStore::ui_sequence_layouts`. `GameMode`
  `4d1a6bfa` / `c9ccb25c` / `ResultsSequence` are sequence UniqueIds (0 for every offline mode).

## Points of interest and the minimap (`Widget_PointOfInterest`, `PointOfInterestVisuals`)

(2026-10-09) A screen's `Widget_PointOfInterest` (the race HUD 604153: widget "POI", script `POIDisplayScript`)
holds `f7dfe43b`, an array of handles to `PointOfInterestVisuals` objects (32 in 604153), a `UIMapCameraConfig`
(`0e636ecb`) and the minimap tile layout (`0e7b0e38`). Parsers `mw_formats::ui::PoiVisuals`,
`AssetStore::ui_poi_visuals(screen)`; the keys `AssetStore::poi_keys`.

`PointOfInterestVisuals` (104 bytes):

| off | field | meaning |
|---|---|---|
| 16 | `8daa3bb7` `World_Entity` (CgsResource.ID) | 0 in all 393 instances |
| 24 | `b8a6b50a` {`Layout`, `LayoutInstanceParams`} | the icon on the map; its instance is `T(Pos)` |
| 52 | `f50e27b7` {Layout, Params} | empty everywhere |
| 80 | `655d3f5f` | the object's UniqueId |
| 84 | `9a5a94d3` | the **key** game code names the look by (not the UniqueId; several objects share a key across screens) |
| 88 | `bd550893` [] {Layout, Params}, count `2300858a` | layouts drawn in the world at the point's matrix |
| 94–98 | `e3256f36`, `8ac61148`, `ea3ab789`, `d788383f`, `db617d4f` | flags (`ea3ab789` = has world layouts) |

Keys are fields of `GameLogicConfig`: `CheckPointsConfig` (+0x24 `e9d221fe` = 1013518 next gate, +0x28
`NormalPoi` = 1013519 target gate, `OnlinePoi`, per-Type fields) and `GameLogic.MapPoiConfig` (25 UniqueIds;
+0x28 `f4d5177c` = 1013510 finish, +0x2c `8d67562d` = 1695650 finish as the next gate, `29401278` = 1013507 the
player arrow, `5bfb3553` = 1013521 racer / cop arrows, `VisionCone`). In 604153 the gate looks are:

| key | world layouts (instance) | map icon |
|---|---|---|
| 1013519 target gate | 1014464 pole: image 4 × 115 px, anchor bottom centre, tex `01010000001e5d71`, `UIDepthTestedShader`; 1014477: "CHECK" (font 1022113, size 28) and `{~Mode.Time}` on glow strips (tex `01010000000f7aef`). Params 1013644 | 371998: tex `0101000000160b34` 23 × 22, `rot: Rot` |
| 1013518 next gate | 1014519: white shape 2 × 100 px at alpha 0.4; 1014516: "CHECK" size 18 at alpha 0.35. Params 1119494 | 604244 (11 × 9) |
| 1013510 finish | 1014464 pole; 1014487: "FINISH" and the time. Params 1560452 (`17cf84`) | 604240: tex `01010000000f789f` |
| 1695650 finish next | 1014464 pole; 1695642: faint "FINISH". Params 1014486 | 604240 |

World instance params share the transforms S(0.1, 0.1, 0.1) · S(`DistanceToCamera(LERP,MINVALUE:20,MAXVALUE:500,
MINRANGE:0.5,MAXRANGE:8)`) · R(`Rot`) · T(`Pos`) · T(0, 0.25, 0); 1013644 adds a colour transform
`DistanceToCamera(LERP,MINVALUE:20,MAXVALUE:60,MINRANGE:0,MAXRANGE:1)` (type 3, blend mode 1), 1119494 a
triggered timeline "FADE(0,300,OUT)" / "FADE(0,300,IN)" on `DistanceToCamera(LTEQ:10)`.

`UILayoutInstanceParams` fields now read: `Colour_Transforms` (`080b944c`) [] {`Binding`, `Value` f32,
`BlendMode` u16, `Type` u16}; the triggered list `402057b7` [] of `TimelineParameters` {`TriggerBinding`,
`aedfbe15` = played when the trigger turns true (the field the visibility timeline plays on show),
`0b603df7` = played when it turns false}.

`UIMapCameraConfig` (in the widget): `LookAt` bound to `Players.LocalPlayer.GetPosition`, `Distance` 600 bound to
`Mode.MinimapDistance`, `FieldOfView` 90, `LookAhead` 0, `LookAtRotation` bound to
`Players.LocalPlayer.GetPlayerDirection`, `ProjectionAngle` 0, `Shape` 1, `23c59c62` 1500, `82cf35f3` 0.1,
`029f7de0` 0.92. The tile layout 371427 is nine 100 × 100 px images bound to
`Widget.POI.POIDisplayScript.TileTex[1..9]` (additive `MapIconShaderAdditive`, opacity 0.7) under
S(`TileSize(DIV:100)`) · R(`MapRot`) · T(`MapOrigin`) · R(90, 0, 0): the map is a world-fixed plane the map
camera looks down on. `MapOrigin` (`FUN_007977c0`) is the corner of the tile one column and one row before the
current one, `MapRot` (`FUN_00797760`) the grid's own yaw (`UIMapGrid.Rotation`, identity).

## Easydrive (inside `Screen2`, `UI/SCREENS2/*.BNDL`)

Inline GeneSys structs in a driving screen's widget groups (group children: field `c7413006`); loader
`mw_assets::easydrive`, listing `mwtool ui easydrive`. All definitions share the widget base (Name, ids, handles,
`UserActionInstance` actions, data sources, layouts, scripts, 8 `u8` flags, 0x0C..0x44).

| type | own fields |
|------|------------|
| `Widget_EasyDriveTierDefinition` (88) | 68 HSM transition name (`TO_BASE` = root), 76 `Title` (Language id), 80 `Sources`[] {`053e93c0` condition binding, `2205fba3` length binding, `Path`, `Type` (item type name), `77b2f836` bool} |
| `Widget_EasyDriveLinkItemList` (76) | 68 `Entries`[] {`Token` (transition to the tier of that name, or `DISMISS` / `CONFIRM`), `Description`, `Name` (Language ids), `SpeechCommand` u16} |
| `Widget_EasyDriveHookItemList` (76) | 68 `Entries`[] {`aabe34d0` hook argument, `3ab7e7fa` HSM exit token, `b50ff8fe` hook token, description, name, speech} |
| `Widget_EasyDriveTypeDefinition` (108) | 88 properties[] {`Name`, `Path` binding with `[#]` = item index}, 92 / 96 selection callbacks[] {`aabe34d0` argument property, `781443e5` enabled property, `Name` (code callback), three u16 enums} |

### UI element stacks (`UIElement_ElementStack`) and the Easydrive menu layout

An element stack repeats `UILayout`s (resource IDs in `Cell.Content`) over a data source; parser
`mw_formats::ui::Stack`, drawing `UiScene::stack`.

| field | meaning |
|-------|---------|
| `DataSource` | the list the cells refer to (`Widget.EasyDrive.Items`), or for a one-cell stack the item itself (`~Widget.EasyDrive.Items[#parent_index#]`, `~Widget.EasyDrive.CurrentItem`) |
| `3555b1f7` | cell count binding (`<list>._length`, `~Widget.EasyDrive.EasyDrive.TierHistoryLength`); empty = one cell |
| `Header` / `DefaultTemplate` / `Footer` | `Cell`[] {`Content` layout, `CellIndex`, `Span`} |
| `TemplateLayouts` | {`Binding`, `Cells`[]}: the first whose binding holds (with `#` = the entry index) replaces the default for that entry (`Widget.EasyDrive.SelectedIndex(EQUALS:#)` = the selected row; `Widget.PAINTITEM.IsActive` & co. = the item type's row) |
| `Width`, `Height` | the stack's box; nested row stacks have Height 25 (selected row) or 0 (from their content) |

Not decoded: `CellSizeDefinition` (empty in Easydrive), `Padding`, `Shift`, `ScrollSpeed`, the unnamed counts
(`280402f1` = 3 on the menu stack). Inside a cell, bindings without a dot (`Name`, `Locked`, `DLCIconID`,
`PositionNotNull`, `Shutdown`, `New`) are fields of the entry.

Easydrive (`UI/SCREENS2/371621.BNDL`, widget `EasyDrive`): layout **1773380** = the menu, one stack at (3, 43)
299×231 over `Widget.EasyDrive.Items`: header `01000000001b0e24` (stack over the tier history: banner image
256×29 + title label `TierHistory[#].Name`), unselected row `010000000019a0db` → `01000000000f75b7` (dark row
image 256×29 at (7,−3), name label at (56,12) left-middle, lock / DLC / medal icons), selected row
`01000000001b0f4b` → `01000000001b0f03` (white bar 342×29 at (38,−3) and 354×37 at (34,−7), name at (87,12), arrow
16×22 at (354,11), "NEW" badge, six glow images `01010000001942a0`), footer `01000000001b1186` (scroll arrows
14×24 at (55,9), shown when `~Widget.EasyDrive.EasyDrive.Length(GTEQ:11)`). Layout **1805584** = the closed-menu
prompt (d-pad icon 39×39 at (8,6), alert variant with `Recommends.ShowAlert`, logo strip 100×16, "NEW" badge on
`Widget.Prompt.New`); **1713378** is empty. Widget `EasyDrivePanel` (layout 1902272, a 548×25 stack at (365,15)
moved down by `Widget.EasyDrive.ItemsAbovePanel × 26.42`) is the side panel next to the selected row.
