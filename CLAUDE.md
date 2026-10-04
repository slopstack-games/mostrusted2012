# Most Rusted: working notes

Rust reimplementation of NFS Most Wanted (2012) that runs on the original assets, read at runtime from the
user's install (IW4L-style). Read these before starting work:

- [docs/PLAN.md](docs/PLAN.md): milestones and what to do next. Update the checkboxes when things land.
- [docs/research-log.md](docs/research-log.md): open format questions and dated findings. Add to it whenever
  something new is learned, including dead ends.
- [docs/formats.md](docs/formats.md): settled format knowledge.
- [docs/reverse-engineering.md](docs/reverse-engineering.md): why the exe can't be statically analysed.
- [docs/handling.md](docs/handling.md): GeneSys handling field mappings, model constants, test results.
- [docs/next-session.md](docs/next-session.md): handoff from the last session (if present): start there.

## Hard rules

- **Never commit game data or anything derived from it**: extracted resources, DDS/PNG/OBJ exports,
  Ghidra databases, decompiled code. Use the session scratchpad or git-ignored dirs (`/extracted`, `/re/out`).
- Community references (burnout.wiki, DGIorio's MIT tools) may be read for format knowledge; don't copy code.

## Layout

```
crates/mw-gamedir   install detection, case-insensitive paths      (no deps on other crates)
crates/mw-bundle    bnd2 v5 reader, ResourceId, ResourceType, Reader
crates/mw-formats   per-type parsers; take &mw_bundle::Resource     (never depends on Bevy)
crates/mw-audio     EA audio decoders (SPS/EALayer3, Ginsu/XAS) and the engine-sound synth
crates/mw-assets    AssetStore: index over all bundles, cross-bundle loads, scene helpers
crates/mwtool       CLI for inspection / extraction / previews
crates/mostrusted   Bevy 0.19 runtime (loader thread → channel → ECS), avian3d 0.7 physics (drive.rs)
```

Game install on this machine: `~/.local/share/Steam/steamapps/common/Need for Speed(TM) Most Wanted`
(found automatically).

## Commands

```sh
cargo build --release --workspace
cargo clippy --release --workspace          # keep at 0 warnings
cargo test --release --workspace
cargo fmt                                    # rustfmt.toml: max_width 120

./target/release/mwtool verify               # must report 0 failures
./target/release/mwtool list <bundle>        # bundle paths are relative to the game dir
./target/release/mwtool extract <bundle> <dir>
./target/release/mwtool models <bundle> <dir>      # writes preview.png (software render, viewable)
./target/release/mwtool world <dir> --bundle HAWAII/TRK_UNIT1.BNDL
./target/release/mwtool shaders                    # samplers, parameters, vertex layout per shader
./target/release/mwtool shader-asm DriveableSurface # DXBC disassembly of a shader's programs
./target/release/mwtool survey VehicleGraphicsSpec --len 0x60   # per-field stats across all instances
./target/release/mwtool vehicles                   # car catalogue (id, year, name, bundle)
./target/release/mwtool texture <id> out.png
./target/release/mwtool genesys VEHICLES/VEH_1085007_HI.BNDL --type HandlingBehaviour   # or: genesys all --type X
./target/release/mwtool handling 911                # typed handling parameters (docs/handling.md)
./target/release/mwtool genesys-types > types.tsv    # every GeneSys type and field (hash, offset, type)
./target/release/mwtool genesys-crack --words a.txt --max-words 4 --type Steering   # guess field names (check hits!)
./target/release/mwtool shader-usage               # world triangles per shader (prioritise ports)
./target/release/mwtool audio UI/SONGS/116502.SPS <dir>   # or a bundle: its Ginsu/Wave resources -> .wav
./target/release/mwtool engine-sound 911 out.wav   # rev sweep through the engine synth (--trace: `seconds rpm load` lines)
# Crash damage (skeletal damage clips): inspect / preview a car, or drive into a wall and save before/after shots
./target/release/mwtool damage 911 --level front=0.5 --preview <scratch>/dmg.png
./target/release/mostrusted --drive 911 --test crash --screenshot <scratch>/crash.png
# Measuring the original game (read-only, Linux/Proton; docs/handling.md "Measuring the original")
./target/release/mwtool memscan --watch handling.watch        # interactive value scanner; `help`
./target/release/mwtool telemetry handling.watch -o run.csv --hz 120

# Visual check without a human: renders, saves, exits
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --screenshot <scratch>/road.png   # see docs/viewpoints.md
./target/release/mostrusted --vehicle "911" --camera 4.5,1.4,4.0,0,0.4,0 --screenshot <scratch>/car.png
# Handling tests on a flat track (4x real time), prints TEST RESULT lines and exits
./target/release/mostrusted --drive 911 --test accel     # or brake, corner (skidpad g), handbrake, drift, engine
# Engine RPM every physics tick, for `mwtool engine-sound --trace` (use with --test engine)
MW_ENGINE_TRACE=<scratch>/rpm.trace ./target/release/mostrusted --drive 911 --test engine
# Driving without a human: fixed throttle, logs "car at x y z, speed, compression" once a second
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.4 --screenshot <scratch>/drive.png --screenshot-delay 400
# Car menu (Tab / F1 / gamepad Start in interactive runs). Headless: swap cars in turn (`name@paint`, `free` =
# free camera), screenshot after the last; logs model / mesh / material / entity counts per step (leak check)
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.3 --swap-to "veyron super,aventador@2" --swap-delay 200 --screenshot <scratch>/swap.png
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 12 --swap-to 911,free,veyron --screenshot <scratch>/spawn.png
./target/release/mostrusted --drive 911 --units 12 --show-menu --screenshot <scratch>/menu.png --screenshot-delay 60
```

Use `--camera=` with `=` when the first value is negative. Release builds are needed: dependencies are
optimised in dev too, but world loading is slow without `--release`.

## Workflow for a new resource format

1. `mwtool extract` a bundle with the type; hexdump `chunk0` (and `chunk1` if it has a body) plus
   `imports.txt`. Import offsets show where the pointer-to-resource fields are.
2. Survey **all** instances across the install with `mwtool survey <Type>` to separate constants, pointers,
   import slots, counts, floats and enums.
3. Check burnout.wiki and DGIorio's `nfsmw_exporter` (its `write_*` functions show layouts) for prior
   knowledge.
4. Write the parser in `mw-formats` with a layout table in the doc comment. Use `mw_bundle::Reader`
   (bounds-checked) and `Resource::import_at(offset)` for resource pointers.
5. Verify against every instance, plus one ground-truth check (visual or real-world value).
6. Record findings and open questions in `docs/research-log.md`; move settled facts into `docs/formats.md`.

## Shader work

The game's shaders are DXBC: port the real maths instead of guessing. Run `mwtool shaders` to find a family's
samplers, parameters and vertex layout, and `mwtool shader-asm <name>` for the pixel and vertex programs (the
longest `ps_4_0` is usually the main G-buffer pass; render target 1 holds `sqrt(albedo)`). Ported families live
in `crates/mostrusted/src/layered.wgsl`, selected by `materials::Look`.

## Audio work

I can't listen to audio, so verify decoders numerically (see the research log): spectral flatness against
white noise, sample-to-sample smoothness (diff/RMS), discontinuities at codec frame or grain boundaries, and
spectral centroid against RPM for engine sounds. Export with `mwtool audio` and analyse the `.wav` in Python.

## Conventions

- Game coordinates are used as-is: metres, Y up, D3D row-major matrices (`v * M`). Convert with
  `loader::game_matrix`. Don't flip axes; signage confirms handedness is already right.
- Resource IDs are displayed as 16 hex digits (`{:016x}`), matching the bundles' debug string tables.
  `ResourceId::community_string()` gives the `AA_BB_CC_DD` form used by modding tools.
- Parsers return `mw_formats::Result` and must not panic on malformed data.
- Bevy-specific code stays in `crates/mostrusted`. Heavy decode work happens on the loader thread, which
  sends ready-made `Mesh` / `Image` values to the main thread.
