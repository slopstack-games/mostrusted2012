# Most Rusted 2012

An open-source runtime for **Need for Speed: Most Wanted (2012)**, written in Rust on top of [Bevy](https://bevy.org).

Most Rusted ships **no game data**. Like [IW4L](https://github.com/vladtrc/iw4L) does for MW2, it reads every
model, texture and map directly from your own installed copy of the game at runtime, without modifying it.

**The goal is a game that plays, looks and sounds as close to the original as possible**: every rule (handling,
AI, cameras, rendering, audio, game logic) is read from the original's code and data or measured in the running
original, not approximated.

> Status: in progress. The front end, free roam in Fairhaven with all 93 cars, traffic, races with AI racers,
> collectables and a first version of pursuits work, with handling, cameras, lighting and post-processing read from
> the game's code. Many parts are still missing or differ from the original. See the [roadmap](#roadmap).

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

Driving controls (the original's defaults, read from its code: docs/decomp.md "Input and control bindings";
our debug keys F1–F7, Tab and R on top):

| Action | Keyboard | Gamepad |
|--------|----------|---------|
| Accelerate / brake (reverse at a standstill) | A / Z | right / left trigger |
| Steer | Left / Right arrows | left stick |
| Handbrake | Space | X |
| Camera: chase / bonnet / bumper | C | LB |
| Look around / look back (held fully down) | Delete, Page Down, Home, End (or right mouse drag) | right stick |
| Car menu (any car, any paint, free camera) | Esc, Tab or F1 | Start or Back |
| Volume settings | F2 | Back |
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

The Steam Controller (2026; USB, Bluetooth or its wireless puck) is read directly from its HID interface, since
the OS does not list it as a game controller: it works without Steam, with the pad bindings above (the trackpads,
gyro and back grips are not used). While Steam runs and the game was not started through it, Steam's desktop layout
keeps sending keys and mouse moves from the same controller: either add `mostrusted` to Steam as a non-Steam game
(Steam Input then provides a virtual pad and the direct reader steps aside), or pick an empty desktop layout, or
quit Steam. `MW_STEAM_CONTROLLER=0` turns the direct reader off, `=1` forces it on.

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
- [x] World streaming: instances, props, compounds, dynamic objects, groundcover, collision; zone-composed
  backdrops; model LODs and draw distances by the game's rule
- [x] Game shaders ported from their DXBC (roads, terrain, world, trees, water, vehicle paint, lights and glass),
  the game's G-buffer, key light, shadows, fog, exposure and tonemap; the environment timeline (hour, weather),
  the game's sun path and sky; night lighting (street lamps, lit windows)
- [x] Post-processing from the game's code: bloom, depth of field, colour cube and grade, vignette
- [x] Cars from VehicleGraphicsSpec, a car menu with every car and paint, crash damage from the damage skeleton,
  dirt and scratches, wheel placement by the game's rule
- [x] Handling read from the game's code (drivetrain, tyres, point-mass suspension, aero, steering, drift,
  nitrous, surfaces) and compared against recordings of the running original
- [x] Audio: EA decoders, the engine's full sound bank and mixer, tyre / road / wind / impact / grind sounds, the
  horn, traffic and AI engines, music, volume settings
- [x] Particle effects (tyre smoke, kick-up, sparks, exhaust pops), chase / bonnet / bumper cameras ported from
  the game's code, the race intro cutscenes, the crash camera
- [x] Input mapping like the original (keyboard, gamepads, Steam Controller), menu input with the original's
  repeat and map controls
- [x] Front end: title, loading, pause menu, Easydrive, the map screen (event selection) and event-loading card
- [x] Traffic and parked cars, AI racers, races (gates, laps, results), speed cameras, billboards, gates, jack
  spots, body shops, a first playable pursuit

Still missing or not like the original (user play test, 2026-10-14; details in
[docs/next-session.md](docs/next-session.md)):
- [ ] Physics: some trash bags are indestructible; road collisions sometimes harsh; sidewalks affect the car too
  much; brake-to-drift understeers at higher speeds
- [ ] World: water floating above where it should be in places; terrain sticking through the road in places
- [ ] Navigation (route to a destination); map icons (jack spots, race locations, cops; Most Wanted races shown
  as normal races)
- [~] Police: chase behaviour reworked from the game's code (not merged yet); no sirens or light bars, no radio
  chatter, roadblocks and spike strips missing; crashing racers and cops does not work yet
- [ ] Music on the loading screen, music matched to free roam / races
- [ ] Front end and UI: the start screen overlaps its two prompts; race end screen; the UI in general
- [ ] Cutscenes: changing cars, the start-up cutscene (entering the car, with its stats)
- [~] Crash camera: there, but not quite like the original
- [ ] Progression: mods unlocked by races, Most Wanted cars unlocked by beating them; takedowns / wrecks from
  car-to-car crashes in races
- [ ] Billboards: intact with their artwork by default (broken only once smashed), showing the Easydrive records
- [ ] Collectables: the speed camera flash; the stats UI (top speed per speed camera, gates smashed, jack spots
  found, etc.)
- [ ] Gas stations: the clean transition when driving through one
- [ ] Landing on the roof or out of bounds: crash camera, then a reset onto the nearest road
- [ ] Lights: car lights do not light up the world properly; speed cameras light the world all the time, not only
  when they flash
- [ ] Car sounds and effects: idle sound, nitrous sound, backfires, engine start / stop; tyre marks and nitrous
  flames look off; grass / dirt still give road smoke and marks
- [ ] Traffic reactions: horns, headlight flashing when you drive towards them
- [ ] Animations: driver, steering wheel, animated body parts, world animations, tree wind

## Credits

Format knowledge builds on the community documentation at [burnout.wiki](https://burnout.wiki) and on
[DGIorio](https://github.com/DGIorio)'s MIT-licensed NFS Most Wanted tools, used as a reference only. Audio
format knowledge comes from [vgmstream](https://github.com/vgmstream/vgmstream) (ISC; see
`crates/mw-audio/NOTICE`). The bundled SDL GameControllerDB is zlib-licensed
(`crates/mostrusted/assets/gamecontrollerdb.LICENSE`).

Not affiliated with or endorsed by Electronic Arts or Criterion Games. "Need for Speed" is a trademark of
Electronic Arts Inc.
