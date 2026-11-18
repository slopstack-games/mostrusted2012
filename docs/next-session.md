# Handoff: 2026-11-18: start here

Keep this file short (≤ ~3 KB, user 2026-11-18): open items with a pointer each; details live in the research log.
Earlier handoffs: [handoffs-archive.md](handoffs-archive.md).

**State** (master): clippy 0, `mw-logic` 252 tests, `mwtool verify` 0 failures, wasm-check 0. 911: 0–60 3.60 s,
brake 38.3 m, corner 2.57 g, drift 37.3°, handbrake yaw 81°/s. `--race 462174` 27/27 gates (position 7, as before
the switch), sprint 906146 10/10. Logic: 170 behaviour types, 160 / 187 hooks, 269 / 385 Lua natives.

**Open** (research-log entries of 2026-11-17 / 18 hold the details):
- **Skid / crash audio** loud and distorted after a race start, teleport, pause or car change (user, both builds).
  The skid and impact sound logic is being ported from the code (2026-11-18 agent; read its entry). Meter switches:
  `MW_AUDIO_METER`, `MW_AUDIO_STALL`. AI engine voices cost ~4 % of the audio thread each: races underrun.
- **Handbrake** ported (`mw_handling::handbrake`): user to confirm in play. Open: decomp.md / handling.md text,
  the turn and brake-to-drift replays, crashed lock lanes (+0x1774).
- **Runtime on the logic's managers** (`world_logic.rs`): races done; event starts (`event_starts.rs`) and the car
  swap (`jackspots.rs`) still to move; then `--hsm-event`, jack spot and world-marker checks.
- **mw-ai / mw-police** crates ported, not wired in: specs in decomp.md "AI racers (ported)" (police spec still
  to write). Unported AI parts: lane planner, racing line, re-plan.
- **UI port**: survey only (research log "UI port (widget-tree renderer)"). Decision for the user: make `--hsm` the
  default (the HUD drawn from the logic's widgets needs the logic in every run). Pause menu needs the menu widget's
  native code first.
- New Requests without a runtime handler: ActivateSpotting, CachePlayerScore, SwapGameMode, StartChallenge, ... (research log).
- **Intermittent panic** (1 of 2 sprint runs, 2026-11-18): a command on a despawned entity right after a track unit
  unloaded (`scratch/run.log` lost; no police / traffic reset / swap ran). Find the system (bevy `debug` feature names it).
- Docs split (big docs) still queued.

**Agents**: `sonnet` at medium effort for well-specified ports (a 15-native batch costs ~100k tokens vs ~300k for
default-model tasks); default model for RE design and integration.
