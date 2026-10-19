# Most Rusted: working notes

Rust reimplementation of NFS Most Wanted (2012) that runs on the original assets, read at runtime from the
user's install (IW4L-style). **The end goal is a game as close as possible to the original in every respect**
(how it drives, looks, sounds and plays, its UI, cutscenes and game logic): see "Fidelity rule" below. Start with
the first section of docs/next-session.md; the other docs are references to search, not to read whole (see
"Working economically"):

- [docs/PLAN.md](docs/PLAN.md): milestones and what to do next. Update the checkboxes when things land.
- [docs/research-log.md](docs/research-log.md): open format questions and dated findings. Add to it whenever
  something new is learned, including dead ends.
- [docs/formats.md](docs/formats.md): settled format knowledge.
- [docs/reverse-engineering.md](docs/reverse-engineering.md): why the exe can't be statically analysed, and the
  rules for the user's decompiled dump in `re/`.
- [docs/decomp.md](docs/decomp.md): what the game's code says (handling maths, update loop), from that dump.
- [docs/handling.md](docs/handling.md): GeneSys handling field mappings, model constants, test results.
- [docs/probe.md](docs/probe.md): code inside the running original (measurement hooks, pokes, in-place
  verification of ports); its policy is in reverse-engineering.md "Code inside the running game".
- [docs/next-session.md](docs/next-session.md): handoff from the last session (if present): start there. Older
  handoffs: [docs/handoffs-archive.md](docs/handoffs-archive.md).
- [docs/commands.md](docs/commands.md): every tool, flag, test and switch.

## Hard rules

- **Never commit game data or anything derived from it**: extracted resources, DDS/PNG/OBJ exports,
  Ghidra databases, decompiled code (findings go into docs in prose only). Use the session scratchpad or git-ignored dirs (`/extracted`, `/re/out`).
- Community references (burnout.wiki, DGIorio's MIT tools) may be read for format knowledge; don't copy code.
- **Modding the original is allowed** (docs/reverse-engineering.md): code mods run in the
  user's own copy, offline. Data files may be replaced if backed up and restorable. Byte signatures and anything from the dump stay in git-ignored `re/`.

## Fidelity rule: the original is the reference

The goal is to play and look exactly like the original, not like a plausible racing game. Reverse engineering
the original's inner workings (physics, rendering, audio, AI, rules) to get there is allowed and preferred:
the dump, shader disassembly, the probe, memory reads and frame captures (docs/reverse-engineering.md).
**How vehicles drive is read from the decompiled code**, not guessed or
calibrated from data: trace each rule and constant to its function (`.rdata` constants from the exe on disk,
`.bss` ones from their initialiser or a live read). A value becomes a model decision only after a documented,
failed search of the code. Every behaviour or look change names its source: a data field, a function in the decomp ([docs/decomp.md](docs/decomp.md)), a
constant or trace read from the running game, or a capture of the original (`mwtool capture`, Steam F12
screenshots of the original at a known place and hour, the game's own audio stream). No tuning by feel: a model
value that is not from one of those is marked as such (handling.md "Model decisions") and replaced when it can be
measured. For handling: record the original (`mwtool capture <name>`), replay the same inputs
(`mostrusted --drive <car> --replay ref/<name>/telemetry.csv`) and compare (`mwtool compare`) before and after a
change. Reference captures live in the git-ignored `ref/`.

## Layout

```
crates/mw-gamedir   install detection, case-insensitive paths      (no deps on other crates)
crates/mw-bundle    bnd2 v5 reader, ResourceId, ResourceType, Reader
crates/mw-formats   per-type parsers; take &mw_bundle::Resource     (never depends on Bevy)
crates/mw-audio     EA audio decoders (SPS/EALayer3, Ginsu/XAS) and the engine-sound synth
crates/mw-assets    AssetStore: index over all bundles, cross-bundle loads, scene helpers
crates/mw-handling  the game's handling maths, engine-free (tyres, drivetrain, drift, nitrous, surfaces)
crates/mwtool       CLI for inspection / extraction / previews
crates/mostrusted   Bevy 0.19 runtime (loader thread → channel → ECS), avian3d 0.7 physics (drive.rs)
crates/mw-probe     DLL loaded into the running original (docs/probe.md; i686-pc-windows-gnu)
```

Game install on this machine: `~/.local/share/Steam/steamapps/common/Need for Speed(TM) Most Wanted`
(found automatically).

Windows hosts build and run the same workspace: docs/commands.md "Windows".

## Commands

The full list (every mwtool subcommand, runtime flag, test and `MW_*` switch) is in
[docs/commands.md](docs/commands.md): grep it for the system you work on instead of reading it whole. The ones
used in nearly every session:

```sh
tools/b                                      # quiet build of mostrusted + mwtool (warnings / errors only)
tools/t --drive 911 --test accel --mute      # quiet run: result lines + exit code (full log: scratch/run.log)
tools/hl ./target/release/mostrusted ...     # any run in a headless gamescope: never pops up on the desktop
cargo clippy --release --workspace          # keep at 0 warnings
cargo test --release --workspace
cargo fmt                                    # rustfmt.toml: max_width 120
./target/release/mwtool verify               # must report 0 failures

# Handling tests on a flat track (4x real time): TEST RESULT lines, then exit (accel, brake, corner, drift, ...)
./target/release/mostrusted --drive 911 --test accel
# Visual check without a human: renders, saves, exits (viewpoints: docs/viewpoints.md)
./target/release/mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 13 --screenshot <scratch>/road.png
./target/release/mostrusted --drive 911 --units 12 --auto-throttle 0.4 --screenshot <scratch>/drive.png --screenshot-delay 400
# Reference runs of the original: record, replay the inputs, compare
./target/release/mwtool capture corner1
./target/release/mostrusted --drive 911 --replay ref/corner1/telemetry.csv
./target/release/mwtool compare ref/corner1/telemetry.csv ref/corner1/telemetry.ours.csv
# The dump (git-ignored helpers in .claude/tools): fn <addr>, callers <addr>, exe-const <va> [n] [f32|u32|str]
```

Use `--camera=` with `=` when the first value is negative. Release builds are needed: dependencies are
optimised in dev too, but world loading is slow without `--release`.

## Working economically (context is the budget)

Accuracy comes from reading the right source, not from reading everything. Habits that keep context small:
- The docs are large (research-log ~580 KB, decomp ~470 KB, formats ~100 KB, PLAN ~60 KB): never read them
  whole. List headings (`grep -n '^##' docs/decomp.md`, `grep -n '^### ' docs/research-log.md`: dated, newest
  first) or grep for the function / type / flag, then read that section with an offset and limit.
- **No windows on the user's desktop**: every `mostrusted` run (tests, screenshots) goes through `tools/t` or
  `tools/hl` (headless gamescope; `MW_SHOW=1` only when the user asks to watch).
- Build with `tools/b` (only warnings / errors and the result) and run with `tools/t <mostrusted args>` (only the
  TEST / BENCH RESULT lines, errors, panics and the exit code; the full log is in scratch/run.log for when you
  need it). `NO_COLOR=1` (set in `.claude/settings.json`) drops ANSI codes. Per-frame logging stays at `debug!`.
- Open items have one home: the newest section of docs/next-session.md. The README roadmap and PLAN.md
  checkboxes are updated once at the end of a session, not after every change.
- Screenshots: look at the ones that decide something. For A/B, stack before / after into one downscaled image
  (`python3 tools/stack.py out.png a.png b.png`) instead of reading two full-size ones.
- Dump: `fn <addr>` / `callers <addr>` for one function at a time; don't read whole decompiled files.
- Agents: a subagent's report is all the integrator sees, so it should hold the result, the sources, the model
  decisions and what's open; no step-by-step narration. Long tables / logs go into the research log or a
  git-ignored file, and the report points to them.

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
The engine's A/B switches (`MW_AUDIO_*`, `MW_SOUND_TRACE`) are in docs/commands.md "Audio".

## Parallel agents

Worktree agents get [docs/agent-brief.md](docs/agent-brief.md) (setup, rules, report) plus a short task: the
integration branch, the task, what the other agents work on, the co-author line. Builds must be throttled
(handoffs-archive.md "Working with parallel agents"): at most ~4 agents building at once,
`.claude/worktrees/.cargo/config.toml` caps their jobs, each worktree gets a real copy of the dependency build.
The integrator merges, runs clippy / tests / `mwtool verify` once on the merged tree, moves stray command lines
into docs/commands.md, and removes merged worktrees and branches (≈12 GB each).

## Conventions

- Game coordinates are used as-is: metres, Y up, D3D row-major matrices (`v * M`). Convert with
  `loader::game_matrix`. Don't flip axes; signage confirms handedness is already right.
- Resource IDs are displayed as 16 hex digits (`{:016x}`), matching the bundles' debug string tables.
  `ResourceId::community_string()` gives the `AA_BB_CC_DD` form used by modding tools.
- Parsers return `mw_formats::Result` and must not panic on malformed data.
- Bevy-specific code stays in `crates/mostrusted`. Heavy decode work happens on the loader thread, which
  sends ready-made `Mesh` / `Image` values to the main thread.
