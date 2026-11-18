# Handoff: 2026-11-18 (end of session): start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master, 2026-11-18 late): clippy 0, workspace tests pass, `mwtool verify` 0 failures. The logic (HSM) runs
by default; races 462174 27/27 (225.6 s) and 906146 10/10 (136.7 s) pass with it (the start hung before: placement
waited on a ground ray the paused sim could not answer, research log "races with the logic hung"). 911: 0–60 3.82 s
(the strut joint port: the wheels' mass rides with the car). Lua natives 385 / 385, HSM behaviour types 192 / 199
(+ drift / Smash-and-Grab scoring), hooks 187 / 187. Police: roadblocks and the bust camera wired.

**Agents running at handoff** (each writes a research-log entry; merge with `scratch/merge.sh <id>`): skid / crash
audio after resets, speech manager (police radio), HUD feedback-message queue, map POIs / event waypoints.

**Open**:
- **Bug hunt (2026-11-18, branch `claude/bug-hunting-kdkey0`)**: ~45 fixes, verified without the install. Run
  `mwtool verify` + `game_chunks` (new parser budgets, stricter Lua verifier); suspected items in research-log 2026-11-18.
- **Runtime inputs not fed**: drift / Smash-and-Grab scoring inputs (no event uses them); online requests (store,
  friends, telemetry, speedwall cache); car select, speech, waypoints logged-only until the agents above land.
- **Pursuit test with traffic**: busted after 8 s at 0 mph (also on the police branch); passes with the documented
  `--units 12 --no-traffic` (1 escape, 1 bust).
- **Model decisions to measure**: strut joint position parameter (+0x30) writer; `kerb100a` braking RMS 1.4 → 2.3 mph
  after the strut port; bust camera node semantics (code not in the dump).
- **Sub-harmoniser** DC (+0.045 vs +0.0024) and a live before / after (merged 2026-11-18).
- **Handbrake**: user to confirm in play; turn / brake-to-drift replays need `mwtool capture` first.
- **Pages CI**: check a run's time and the `cargo-timings` artifact. Docs split (big docs) still queued.

**Agents**: `sonnet` at medium effort for well-specified ports / wiring; default model for RE and integration. At most
~4 building at once.
