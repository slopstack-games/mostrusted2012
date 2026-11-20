# Handoff: 2026-11-20: start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master, 2026-11-20): clippy 0, tests pass, `mwtool verify` 0 failures. HSM logic on by default; races
462174 27/27 (218.2 s: go is now the logic's game-mode timer at the intro's end, not its event trigger) and 906146
10/10 (135.9 s); pursuit passes with and without traffic. Logic requests not carried out in a race: 45 → 1
(`PreparePostEventAwards`). Merged 2026-11-20: sequence items (timeline controllers, picture, intro spawns, damage
VFX), Gameplay switches / heat levels, world-object sequences, race / game-mode requests (`logic_race.rs`), profile /
UI requests (`logic_profile.rs`); research log 2026-11-19 / 20.

**Open**:
- **Sequences left**: spike strips (roadblock strip placement, tyre burst; the player's weapon has no offline
  event), animated world props (no skinned world objects), in-race feedback-group sequences (SpawnTraffic, most
  Gameplay items live there; `feedback.rs` plays layouts only), HUD sequences' controllers, picture items on the
  top-level clock only, motion blur / DOF / DisableWorldRendering in the renderer, wrecked state (169 damage items).
- **Logic**: `PreparePostEventAwards` (`FUN_006fbb60`: best finish, unlock tables, reward unit); the event Timeline
  sequences and 900 s timeout the game-mode timer arms; roaming blacklist rival; AI racers seen falling at 462174's
  start by an agent (not reproduced in an intro shot at 23 s with the full world: check after go).
- **Feedback queue**: song popup over the Easy Drive prompt (HUD mode +0x8c); message sounds / movies / speech
  logged only (research-log 2026-11-19).
- **POIs**: gates, event starts, collectables, racers, cops bypass the list; `FUN_0080cb60` clamping (2026-11-18).
- **UI report**: logic unaware of the runtime's open Easydrive (POSITION / LAP under the menu), speed / odometer digit
  pitch, a gate-marker capture (research log 2026-11-19). Races starting when played: user to confirm PR #8.
- **AI**: the racer AI rams a slow car 30 m ahead in its lane.
- **Runtime inputs not fed**: drift / Smash-and-Grab scoring; online requests; car select, speech, waypoints.
- **To measure**: strut joint +0x30 writer; `kerb100a` braking RMS 1.4 → 2.3 mph; bust camera nodes; sub-harmoniser
  DC (+0.045 vs +0.0024); handbrake (user to confirm; capture turn / brake-to-drift first).
- **Bug hunt** leftovers (research-log 2026-11-18). **Pages CI** timing; docs split queued.

**Agents**: `sonnet` for well-specified ports / wiring; default model for RE and integration. At most ~4 building.
