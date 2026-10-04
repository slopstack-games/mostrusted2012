# Most Rusted 2012

An open-source runtime for **Need for Speed: Most Wanted (2012)**, written in Rust on top of [Bevy](https://bevy.org).

Most Rusted ships **no game data**. Like [IW4L](https://github.com/vladtrc/iw4L) does for MW2, it reads every
model, texture and map directly from your own installed copy of the game at runtime, without modifying it.

> Status: early and experimental. You can explore Fairhaven in a free camera, or pick any of the 93 cars and
> drive it through the city with collision, crash damage, engine and tyre sounds, music and a basic HUD. It does
> not play like Most Wanted yet: the handling still differs from the original, there are no AI vehicles, no
> events or pursuits, no Easydrive or front end, and the image has a yellow tint. See the [roadmap](#roadmap).

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

# Drive a car through the city
cargo run --release -p mostrusted -- --drive "911"

# Handling tests on a flat track: accel (0-100, quarter mile, top speed), brake, corner (skidpad),
# handbrake, drift (brake-tap drift), donut, engine (a pedal sequence for engine-sound checks), crash
cargo run --release -p mostrusted -- --drive "veyron" --test accel

# Skip the grass sprites (faster loading)
cargo run --release -p mostrusted -- --no-groundcover

# Audio: engine and tyre sounds from each car's sound data, music from the game's soundtrack
# (--no-music, --mute, --volume master=0.8,engine=1,sfx=0.6,music=30%)

# Look at a car, by name, vehicle ID or bundle path; optional paint colour
cargo run --release -p mostrusted -- --vehicle "911" --paint 20,60,170
```

Driving controls (our own layout for now; matching the original's bindings is on the roadmap):

| Action | Keyboard | Gamepad |
|--------|----------|---------|
| Throttle / brake (reverse at a standstill) | W / S or arrows | right / left trigger |
| Steer | A / D or arrows | left stick |
| Handbrake | Space | A |
| Camera: chase / bonnet / bumper | C | Y |
| Look back / look around | (right mouse drag) | B or right stick back / right stick |
| Car menu (any car, any paint, free camera) | Tab or F1 | Start |
| Volume settings | F2 | Select |
| Time of day / weather | F5 / F6 hour, F7 weather | |
| HUD debug text | F3 | |
| Reset the car / repair | R | |

Free-camera controls (world and `--vehicle` modes): WASD to move, Q/E to go down and up, hold the right mouse
button to look, Shift to go faster, and the scroll wheel to change speed.

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
cargo run --release -p mwtool -- handling 911                             # a car's handling data
cargo run --release -p mwtool -- engine-sound 911 out.wav                 # render the engine's rev sweep
cargo run --release -p mwtool -- memscan                                  # read-only: inspect the running original
cargo run --release -p mwtool -- telemetry handling.watch -o run.csv      # record the running original's car state
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
| [`mostrusted`](crates/mostrusted) | the Bevy runtime: streaming loader, the game's rendering passes, vehicle physics (`drive`, `drivetrain`, `tyre`, `chassis`, `drift`), cameras, damage, particles, audio, HUD and menus |

If a controller's triggers or buttons don't respond, run with `--gamepad-debug` and check what it reports; a
current SDL GameControllerDB is bundled, and custom mappings can be added via `SDL_GAMECONTROLLERCONFIG`.

Format notes live in [docs/formats.md](docs/formats.md), open questions and findings in
[docs/research-log.md](docs/research-log.md), and the full milestone plan in [docs/PLAN.md](docs/PLAN.md). Notes on
reverse engineering the executable are in [docs/reverse-engineering.md](docs/reverse-engineering.md), and what
the game's code says about handling and the update loop is in [docs/decomp.md](docs/decomp.md) (prose only: no
decompiled code is part of this repository).

## Roadmap

Summary below; see [docs/PLAN.md](docs/PLAN.md) for milestones, the gap list against the original and what's next.

Done:
- [x] Bundle reader, verified against all 2,020 bundles (432k resources); GeneSys reader with ~1,200 recovered
  field names
- [x] World streaming: instances, props, compounds, dynamic objects, groundcover, collision; zone-composed backdrops
- [x] Game shaders ported from their DXBC (roads, terrain, world, trees, water, vehicle paint, lights and glass),
  the game's G-buffer, key light, shadows, fog, exposure and tonemap; the environment timeline (hour, weather)
- [x] Cars from VehicleGraphicsSpec, a car menu with every car and paint, crash damage from the damage skeleton
- [x] Audio: EA decoders, the engine's full sound bank and mixer, tyre / road / wind / impact sounds, music,
  volume settings
- [x] Particle effects (tyre smoke, kick-up, sparks), data-driven chase / bonnet / bumper cameras, a basic HUD

In progress or missing (user review, 2026-10-04):
- [~] Handling: rebuilt from the game's own code (drivetrain, tyres, suspension, aero, steering, drift; notes in
  [docs/decomp.md](docs/decomp.md), [docs/handling.md](docs/handling.md)) but it **does not drive like the
  original yet**; next is measuring the running game (`mwtool memscan` / `telemetry`)
- [ ] Tyre screech sounds like a horn held at the same time
- [ ] Shading: a yellow tint over the whole image
- [ ] Animations (driver, steering wheel, animated body parts, world animations)
- [ ] Input mapping like the original game
- [ ] UI: Easydrive and the game's front end (only a HUD and our own menus so far)
- [ ] Crash camera
- [~] Prop collisions: simulated, but opt-in (`--prop-physics`) because the car wedges against knocked props
- [ ] AI vehicles: traffic, racers, cops
- [ ] Game logic: events, pursuits, the Most Wanted list, jack spots, progression

## Credits

Format knowledge builds on the community documentation at [burnout.wiki](https://burnout.wiki) and on
[DGIorio](https://github.com/DGIorio)'s MIT-licensed NFS Most Wanted tools, used as a reference only. Audio
format knowledge comes from [vgmstream](https://github.com/vgmstream/vgmstream) (ISC; see
`crates/mw-audio/NOTICE`). The bundled SDL GameControllerDB is zlib-licensed
(`crates/mostrusted/assets/gamecontrollerdb.LICENSE`).

Not affiliated with or endorsed by Electronic Arts or Criterion Games. "Need for Speed" is a trademark of
Electronic Arts Inc.
