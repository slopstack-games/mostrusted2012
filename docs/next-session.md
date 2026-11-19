# Handoff: 2026-11-18 (end of session): start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master, 2026-11-19): clippy 0, workspace tests pass, `mwtool verify` 0 failures. The logic (HSM) runs
by default; races 462174 27/27 (225.6 s) and 906146 10/10 (136.7 s) pass with it (the start hung before: placement
waited on a ground ray the paused sim could not answer, research log "races with the logic hung"). 911: 0–60 3.82 s
(the strut joint port: the wheels' mass rides with the car). Lua natives 385 / 385, HSM behaviour types 192 / 199
(+ drift / Smash-and-Grab scoring), hooks 187 / 187. Police: roadblocks and the bust camera wired.

**Merged 2026-11-19**: PR #8 (bug hunt, checked on the install), sfx_player one-shot exit rule, the HUD
feedback-message queue, the map manager's POI list / intro waypoints (both finished from agents' uncommitted work).
One worktree is left to remove: `agent-ae1caf9e7b2a6f6d8` (merged; the session ran inside it).

**Open**:
- **Feedback queue**: the song popup draws over the Easy Drive prompt (message HUD mode +0x8c not modelled); sounds /
  movies / speech of a message logged only (research-log 2026-11-19).
- **POIs**: gates, event starts, collectables, racers and cops still bypass the list; map-edge clamping
  (`FUN_0080cb60`) not ported (research-log 2026-11-18 "POI list").
- **Bug hunt**: suspected items left unchanged are listed in research-log 2026-11-18.
- **User report: UI wrong almost everywhere** (races starting when played: fixed by PR #8, user to confirm). Shots at 1920×1080 (`MW_HL_SIZE=1920x1080
  MW_HSM_SHOTS=<dir> tools/t --race 462174`): intro camera upside down (≈180° roll); race HUD hugs the edges, the
  speedometer is cut off at the bottom right, a large rotated "CHECK…" message at the top right (new feedback queue?);
  card tip line at the left edge. The HUD is drawn from the logic's widgets since `ui_widgets.rs` (merged 2026-11-18,
  checked only at 1280×720). Compare 720p vs 1080p first, then bisect today's merges if needed.
- **Pursuit test** passes again, with and without traffic (the time-out and the 8 s bust were the test's set-up, not
  speech or a rule: research-log 2026-11-19). Open: the racer AI rams a slow car 30 m ahead in its lane.
- **Runtime inputs not fed**: drift / Smash-and-Grab scoring inputs (no event uses them); online requests (store,
  friends, telemetry, speedwall cache); car select, speech, waypoints logged-only until the agents above land.
- **Model decisions to measure**: strut joint position parameter (+0x30) writer; `kerb100a` braking RMS 1.4 → 2.3 mph
  after the strut port; bust camera node semantics (code not in the dump).
- **Sub-harmoniser** DC (+0.045 vs +0.0024) and a live before / after (merged 2026-11-18).
- **Handbrake**: user to confirm in play; turn / brake-to-drift replays need `mwtool capture` first.
- **Pages CI**: check a run's time and the `cargo-timings` artifact. Docs split (big docs) still queued.

**Agents**: `sonnet` at medium effort for well-specified ports / wiring; default model for RE and integration. At most
~4 building at once.
