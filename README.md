# Most Rusted 2012

An open-source runtime for **Need for Speed: Most Wanted (2012)**, written in Rust on top of [Bevy](https://bevy.org).

Most Rusted ships **no game data**. Like [IW4L](https://github.com/vladtrc/iw4L) does for MW2, it reads every
model, texture and map directly from your own installed copy of the game at runtime, without modifying it.

> Status: early and experimental. You can explore the city in a free camera or drive a car around it with
> collision; there is no gameplay yet. See the [roadmap](#roadmap).

## Requirements

- A legitimate install of Need for Speed: Most Wanted (2012) for PC (Steam or EA app).
- Rust 1.96+ (`rustup`), and a GPU with Vulkan, Metal or DX12 that supports BC texture compression.

The install is found automatically in this order:

1. `--game-dir <path>`
2. the `MW2012_DIR` environment variable
3. every Steam library (app 1262560), including Flatpak Steam
4. the default EA app / Origin install folders (Windows)

## Running

```sh
# Explore Fairhaven (streams all 169 track units, nearest first)
cargo run --release -p mostrusted

# Only the 20 track units nearest the city centre
cargo run --release -p mostrusted -- --units 20

# Drive a car through the city (W/S throttle/brake, A/D steer, space handbrake, R reset;
# gamepad: right/left trigger, left stick, A = handbrake)
cargo run --release -p mostrusted -- --drive "911"

# Handling tests on a flat track: accel (0-100, quarter mile, top speed), brake, corner (skidpad),
# handbrake, drift (brake-tap drift), engine (a pedal sequence for engine-sound checks)
cargo run --release -p mostrusted -- --drive "veyron" --test accel

# Skip the grass sprites (faster loading)
cargo run --release -p mostrusted -- --no-groundcover

# Audio: engine sound follows the RPM, music plays from the game's own soundtrack
# (--no-music, --mute)

# Look at a car, by name, vehicle ID or bundle path; optional paint colour
cargo run --release -p mostrusted -- --vehicle "911" --paint 20,60,170
```

Free-camera controls (world and `--vehicle` modes): WASD to move, Q/E to go down and up, hold the right mouse button to look, Shift to go faster, and the
scroll wheel to change speed.

`--camera x,y,z,look_x,look_y,look_z` and `--screenshot out.png` set the camera and capture one frame after loading,
which is useful for automated checks.

## mwtool

A command-line tool for exploring the game data:

```sh
cargo run --release -p mwtool -- info                         # detected install
cargo run --release -p mwtool -- verify                       # decompress every resource in every bundle
cargo run --release -p mwtool -- list WORLDLIST.BNDL          # resource table of a bundle
cargo run --release -p mwtool -- extract <bundle> <dir>       # raw resource chunks + import tables
cargo run --release -p mwtool -- textures <bundle> <dir> --png
cargo run --release -p mwtool -- models <bundle> <dir>        # LOD0 geometry as .obj + preview render
cargo run --release -p mwtool -- world <dir> [--bundle HAWAII/TRK_UNIT45.BNDL] [--obj]
cargo run --release -p mwtool -- shaders                      # every shader: samplers, parameters, vertex layout
cargo run --release -p mwtool -- shader-asm DriveableSurface  # disassemble a shader's DXBC programs
cargo run --release -p mwtool -- survey VehicleGraphicsSpec   # per-field statistics across all instances
cargo run --release -p mwtool -- vehicles                     # car catalogue with names and bundles
cargo run --release -p mwtool -- texture <id> out.png
cargo run --release -p mwtool -- collision HAWAII/TRK_UNIT1.BNDL <dir>   # collision mesh .obj + preview
cargo run --release -p mwtool -- audio UI/SONGS/116502.SPS <dir>          # game audio -> .wav
cargo run --release -p mwtool -- shader-usage                             # world triangles per shader
cargo run --release -p mwtool -- find <resource id>
```

Anything you extract is the game's copyrighted data. Keep it local and never commit it (`.gitignore` covers the
default locations).

## Layout

| crate | purpose |
|-------|---------|
| [`mw-gamedir`](crates/mw-gamedir) | finds and validates the game install; case-insensitive path resolution |
| [`mw-bundle`](crates/mw-bundle) | reads Criterion BundleV2 (`bnd2` v5) archives: header, resource table, zlib chunks, imports, debug names |
| [`mw-formats`](crates/mw-formats) | resource parsers: textures, meshes, materials, shaders, models, world layers (instances, props, compounds, groundcover), collision, vehicles, GeneSys; BC1/2/3 decoding; DDS export |
| [`mw-assets`](crates/mw-assets) | game-wide resource index across all bundles, cross-bundle import resolution, model to mesh helpers |
| [`mw-audio`](crates/mw-audio) | EA audio: SPS/EALayer3 (via nanomp3) and Ginsu engine sounds (EA-XAS), granular engine synth |
| [`mwtool`](crates/mwtool) | CLI for inspection, extraction and previews |
| [`mostrusted`](crates/mostrusted) | the Bevy runtime: background streaming loader, conversion to GPU assets, free camera |

If a controller's triggers or buttons don't respond, run with `--gamepad-debug` and check what it reports; a
current SDL GameControllerDB is bundled, and custom mappings can be added via `SDL_GAMECONTROLLERCONFIG`.

Format notes live in [docs/formats.md](docs/formats.md), open questions and findings in
[docs/research-log.md](docs/research-log.md), and the full milestone plan in [docs/PLAN.md](docs/PLAN.md). Notes on
reverse engineering the executable are in [docs/reverse-engineering.md](docs/reverse-engineering.md).

## Roadmap

Summary below; see [docs/PLAN.md](docs/PLAN.md) for milestones, acceptance criteria and risks.

- [x] Bundle reader, verified against all 2,020 bundles (432k resources)
- [x] Textures (BC1/BC3/RGBA8/R8/RGBA16F, cube maps) uploaded to the GPU still compressed
- [x] Renderables, vertex layouts, materials and shaders resolved across bundles
- [x] World streaming from InstanceLists (35k placements) and the free-fly camera
- [x] Vehicle assembly from VehicleGraphicsSpec (wheels, mirrored right side), car names from VehicleList + Language
- [x] Game shaders disassembled from their DXBC; road and terrain albedo ported to WGSL
- [x] Remaining world layers: props, compound and dynamic objects (56k placements), groundcover (1.36 M grass
  sprites); lights parsed but not drawn yet
- [ ] Remaining shader families: lightmaps, plots, road overlays, water, glass, the real vehicle paint
- [x] Collision from PolygonSoupList; a first drivable car (raycast suspension, chase camera)
- [x] Audio: EALayer3 and Ginsu decoders, RPM-driven engine sound, soundtrack playback
- [~] Handling from the game's own physics data (engine curve, gears, mass, springs, brakes, tyre grip curves,
  steering); drift assist and calibration against the original game still to do (see
  [docs/handling.md](docs/handling.md))
- [~] GeneSys data (MW2012 has no AttribSys resources): reader and field-name hash done, ~1,200 field names recovered; vehicle list and names done.
  Still open: game rules, cameras, paint
- [ ] AI traffic and cops, events, sound effects and speech

## Credits

Format knowledge builds on the community documentation at [burnout.wiki](https://burnout.wiki) and on
[DGIorio](https://github.com/DGIorio)'s MIT-licensed NFS Most Wanted tools, used as a reference only. Audio
format knowledge comes from [vgmstream](https://github.com/vgmstream/vgmstream) (ISC; see
`crates/mw-audio/NOTICE`). The bundled SDL GameControllerDB is zlib-licensed
(`crates/mostrusted/assets/gamecontrollerdb.LICENSE`).

Not affiliated with or endorsed by Electronic Arts or Criterion Games. "Need for Speed" is a trademark of
Electronic Arts Inc.
