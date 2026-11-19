# Handoff: 2026-11-20: start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master, 2026-11-20): clippy 0, tests pass, `mwtool verify` 0 failures. HSM logic on by default; races
462174 27/27 (225.6 s), 906146 10/10 (136.7 s). Lua natives 385/385, HSM types 195/199, hooks 187/187.
**Sequence items (PLAN M8.6), merged 2026-11-20**: timeline controllers (`mw_assets::timeline`), picture items
(`sequence_picture.rs`), intro spawns (`cutscene_spawns.rs`), damage-sequence VFX (`damage_vfx.rs`); research log
2026-11-19.

**Open**:
- **Sequences left**: weapons / world-prop items (AddBehaviour, BehaviourEnabled, Animation, WaveSelect,
  ApplyVehicleKick, JumpNTimes), VfxSpot, Gameplay (heat level: HARBOR RUN's two intro cops are removed 0.75 s after
  spawning without it), SpawnDynamicEntity / SpawnTraffic; HUD sequences' controllers; picture items on the
  top-level clock only; motion blur / DOF / DisableWorldRendering in the renderer; wrecked state (169 damage items).
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
