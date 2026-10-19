> **Standing goal (every session):** the game must end up as close as possible to the original NFS Most Wanted
> (2012) in every respect: driving, look, sound, AI, UI, cutscenes and game logic. Work from the original's code
> (the dump, REA on the carved image), its data, and measurements / captures of the running original (CLAUDE.md
> "Fidelity rule"); never settle for "plausible". Open differences are tracked below, in docs/PLAN.md and the
> README roadmap.

# Handoff: 2026-10-19 (session 11, Linux): start here

**State**: branch `session-11` (from master after PR #6); about 20 worktree agents merged. Clippy 0 warnings, 199
tests, `mwtool verify` 0 failures; 911 accel 0–60 mph 3.58 s / quarter mile 11.60 s, nitrous 0–60 3.20 s, brake
38.1 m, corner 2.57 g, drift peak 37.3°; `--race 462174 --test race` 27/27 gates, Most Wanted races spawn their
rival. Every run of our game is headless and silent (`tools/t`, `tools/hl`; never a window or sound on the user's
desktop: CLAUDE.md); the original too (`tools/orig`, null sink). The research log has one dated 2026-10-19 entry per
area below.

| Area | What landed (research log 2026-10-19) | Open |
|---|---|---|
| Kerbs / road harshness | wheel = the game's cylinder probe; strut travel from data; decal triangles skipped; placement at speed; strut joint; strut stop only when crashed | chassis lifts more than the original at a kerb; yaw from surface bumps; launch: ours spins one rear wheel, the original both (tyre / differential) |
| **False crashes at kerbs (user high priority)** | the crash rule and wall-scrape counted avian's speculative contacts; now only touching / pushed ones | **needs the user's play test**; the original's chassis hulls as our body; a capture on the kerb-end line |
| Terrain / floating trees | pass mask per view; per-zone backdrop and groundcover swap (`zone_view.rs`) | stand-in shadows (capture at 17:30); view 0x10 |
| GPU out of memory | world streaming as the game does it (`streaming.rs`): ~1.3 GB instead of ~4.5 GB | idle-machine frame times; free camera still loads everything |
| Probe capture helpers | `mwtool tp / car / mods / conditions / inputs`, `capture --probe`, tick-exact scripted inputs, Hodges strip | far placements kill the original (GPU OOM suspected); airport arena not reachable |
| Nitrous | no downshift while it burns (`FUN_009d93b0`); mods cheat in the probe | a capture with the game's own tank; every car gets nitrous in our handling even without the mod |
| Races | Most Wanted rivals (per-slot rollouts), loading card on race starts, progress kept across a respray | Most Wanted 7's cops; the win's shutdown / jack spot / unlock |
| UI | placement / scaling from the code (HUD aspect correction, menu letterbox), free-drive HUD screen, Easydrive prompt, SP strip, plates, nitrous gauge by mod, pause rows | odometer spacing, digit glow, GPS / pursuit / ticker layouts, Most Wanted list panel |
| Surfaces | tyre classes; kick-up rule; per-surface lookup verified (cobbles are dirt-like in the data) | a coast capture on paving vs asphalt |
| Sounds | traffic pitch (whine), transmission / turbo whines, bus gains, per-wheel surface sounds, doppler fix, skids follow the engine's level | **user: listen to drifts and the rest**; Ginsu grain player; bus EQ / compressors; collisions / props / AI levels vs the engine |

Decisions with the user: portrait loading screens keep the original's bars.

## Open items (user play tests; newest first)

- **Races and free roam** (2026-10-19; agents were finishing at the end of the session, see the research log):
  race intros lack their special shaders / visuals; jack-spot swap broken after a race; races and their start
  markers missing in free roam; "checkpoint missed" always shown; racers don't react to hits as in the original;
  racers stay after a race; cops red not blue on the minimap, no blink, no escape circle.
- **Exhaust flames**: backfire / nitrous flames look framey and not attached to the exhaust (`exhaust.rs`,
  `particles.rs`: emitter space, in-frame spawn interpolation, flipbook rate).
- **Textures**: the tyre tread is blank (tyre material family / samplers, `mwtool shaders`).
- **Wet roads in shadow** still look lit (wet layer's reflection not shadowed).
- **Wheel placement** wrong on some cars, e.g. the Veyron Super Sport.
- **Missing animations** (not listed yet: survey).
- **Menu / Easydrive functionality** missing in many places (survey first).
- **UI content** left: see the table.
- **Sounds**: cars still a bit glitchy (the game's Ginsu player needs window W from `0x00aaef50` / `0x00aad630`).
- From session 10's list, not worked on yet: collectable effects (speed camera flash on pass), gas station
  transition (fade / cut, repair, respray, nitrous refill `FUN_00748ff0`), collectable stats UI (speed lists,
  counts), roof landings and out of bounds (crash camera + reset onto the nearest road), car lights on the world,
  crash particles (glass, debris), camera clipping (find what the original does), dirt cloud size and dirt feel.

New this session (docs/commands.md): `tools/hl`, `tools/silent-sink`, `mwtool tp / car / mods / conditions / inputs /
probe`, `capture --probe --script`, `mwtool surfaces --flat / --tris / --classes`, `placement-check --floating`,
`terrain-check` LOD modes, `--load-all`, `--stream`, `--composed-backdrop`, `--replay ... --spawn`, `MW_MUTE`,
`MW_AUDIO_TAP`, `MW_SFX_ENGINE_LEVEL`, `MW_AUDIO_NO_BUS`, `MW_SURFACE_LOG`, `MW_SPECULATIVE_CONTACTS`,
`MW_CRASH_CONTACTS`, `MW_WINDOW` / `MW_RESIZE_AT`, `MW_UI_TEXT_LOG`, `MW_WHEEL_RAY`, `MW_STRUT_JOINT`.

