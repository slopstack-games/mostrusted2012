# Handoff: 2026-11-18 (end of session): start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master): clippy 0, `mw-logic` 275 tests, `mwtool verify` 0 failures, wasm-check 0. The logic (HSM) runs
by default (`--no-hsm` to bisect); `--race <id>` starts through the logic's event start. 911: 0–60 3.60 s, brake
38.3 m, handbrake yaw 81°/s. Races 462174 27/27, 906146 10/10. Logic: 170 behaviour types, 160 / 187 hooks,
300 / 385 Lua natives. Races, event starts, car swaps and police run on the ported code.

**Unmerged agent branches** (stopped at session end; each has a research-log entry saying where it stopped):
- `worktree-agent-a3ae5e793088ee893` + follow-up `worktree-agent-a1b7e69661d49d206` (0084d7a, builds): AI racers on
  `mw-ai`; 462174 now 6 of 7 AI finish (219–225 s; the crashes were our driver starting on the wrong lap segment).
  906146 panicked after a track unload (the intermittent panic below?); accel not run. Run the checks, then merge both.
- `worktree-agent-ae89085bfd4e6c6f6` (48987e7, builds): engine sub-harmoniser ported (0x00aad950: |x|, never flips
  sign; ours was a square wave that could flip at the rail, matching the user's recording), distortion and BandPass
  from the code. Open: its DC (+0.045 / +0.083 vs the original's +0.0024; a DC block?), live before / after not run.
- `worktree-agent-abe4eab9f744d9496` (78c1b56): `ui_widgets.rs` draws HUD 371621 / 604153 from the logic's widgets,
  hud.rs layout drawing removed; accel / race pass, A/B in its `scratch/shots/`. Open: speedwall panel unchecked.
- `worktree-agent-a9b92c3379235e3c7` (7a3b08d, builds, untested): `logic_effects.rs` carries out chevrons, routes,
  nitrous, siren, engine, music, feedback groups. Unhandled: unlocks, feedback queues, speech, mix snapshots.
Check each: `git log master..<branch>`, build, run the checks, merge with `scratch/merge.sh <id>` (git-ignored
helper: union for doc conflicts) or redo.

**Open**:
- **Skid / crash audio** loud and distorted after a race start / teleport / pause / car change (user). Meter
  switches `MW_AUDIO_METER`, `MW_AUDIO_STALL`. AI engine voices cost ~4 % of the audio thread each.
- **Intermittent panic** (1 of 2 sprint runs): a command on a despawned entity right after a track unit unloaded.
- **Handbrake**: user to confirm in play; turn / brake-to-drift replays need `mwtool capture` first.
- **Police**: roadblocks, bust camera, spawn placement not wired (research log "mw-police in the runtime").
- **Pages CI**: whole-target-dir cache and verbose log are new: check a run's time and the `cargo-timings` artifact.
- Lua natives left: 29 offline (survey of their callees in the research log, 2026-11-18), 50 online, 6 platform / debug.
- Remaining HSM behaviour types (~30) call unnamed engine code: default model, not sonnet.
- Docs split (big docs) still queued.

**Agents**: `sonnet` at medium effort for well-specified ports (~100–180k tokens a batch); default model for RE
design and integration. Two agents porting overlapping topics duplicate work: give each a topic / file.
