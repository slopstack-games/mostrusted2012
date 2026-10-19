> **Standing goal (every session):** the game must end up as close as possible to the original NFS Most Wanted
> (2012) in every respect: driving, look, sound, AI, UI, cutscenes and game logic. Work from the original's code
> (the dump, REA on the carved image), its data, and measurements / captures of the running original (CLAUDE.md
> "Fidelity rule"); never settle for "plausible". Open differences are tracked below, in docs/PLAN.md and the
> README roadmap.

# Handoff: 2026-10-16 (session 10, Linux): start here

**State**: branch `session-10` (from master after PR #5), two rounds of worktree agents merged: round 1 (water,
terrain, prop-break particles, police driving incl. session 8's WIP branch, sirens / light bars / radio / coronas)
and round 2 (world sounds, car lights, speed camera flash + world coronas, crash particles + particle drag,
particle opacity + dirt cloud). Clippy 0 warnings, 192 tests, `mwtool verify` 0 failures; the 911's accel
unchanged (0–60 mph 3.58 s, quarter mile 11.60 s); `--test pursuit --units 30` (with `--units 12` the car does not
reach speed and the escape run is skipped), `--test cops`, `--test smash`, `--test freeroam`, the crash tests and
the race test pass. The original was not run this session: everything below comes from the dump / REA and the
data, and each area's 2026-10-16 research-log entry lists the captures that would confirm it. Context economy:
CLAUDE.md is slim, every command is in docs/commands.md, old handoffs in handoffs-archive.md.

| User item | State | Main open item |
|---|---|---|
| Water floats above where it should | fixed: a zone draws only the water it lists (our nearest-zone fill-in flooded the industrial coast and hung a 13 m sheet over the river gorge); `mwtool water` | the rule is inferred from the data (zone selection code not traced); leftovers near (40, 2280) and `9fd6d20a` in zone 158; captures listed |
| Terrain through the road | fixed: `Model` +0x19 is a per-view skip mask (`FUN_00553810`, `FUN_008cffa0`); 230 shadow / cube / reflection stand-ins no longer drawn in the main view; `mwtool terrain-check` | skipped stand-ins cast no shadow here; mask bits 1 / 2 / 8 unused; view 0x10 unknown; the 0x1f models |
| Particles when props break | added (`prop_vfx.rs`): `WCVfxBehaviour`s (impact, spot effects at locators, ground-slide sparks, lights out) by `FUN_0053cba0`; super emitter (action 0x2B) decoded; sparks were invisible (alpha) | renderer opacity word confirmed (`FUN_004a40a0`, research log 2026-10-16); dirt cloud fade / size from the code; impact light flash, ambient world effects |
| Police drive anywhere, chase too easily | fixed: patrol rides a traffic ghost in the lanes (`cop_patrol.rs`, `FUN_005101e0` modes 0–4); wanted / sight rules (`cop_sight.rs`, `FUN_005192b0`, `FUN_006c5810`); WIP chase states merged | `.bss` 0x011061f8 / 0x01106228 live reads; two wanted terms unidentified; ghost catch-up often takes two tries |
| Cops: no sounds, lights | added: sirens (`police_audio.rs`), light bars and their light (`police_lightbar.rs`, cop-light shader), radio (`police_speech.rs`, 110 events), coronas at every car's lamps (`coronas.rs`) | **who turns AI sirens on is a model** (chasing / searching in a pursuit); world-lamp coronas, headlight glint, corona distance scale s, flare layer; radio static / car-model lines; the sign glare under a cop light matches the code but needs a night capture |
| Missing one-off sounds | added (`world_sfx.rs`): `WCPlaySoundBehaviour` of lamps / signs / gates, props' sounds by `FUN_0054bf70`, a car's first prop touch (billboards), landings `FUN_00576b40`, crash zone contacts, the collectables' message waves | the crash camera's bus ducking (bus ids unmapped) and slow-motion audio; AI racers' crash zones |
| Car lights don't light the world | added (`car_lights.rs`): race cars' `VehicleVfxBehaviour.Lights` (projector, spots, cones) by `FUN_005818f0`; the player's projector with its shadow map (`HeadlightShadows`); traffic has coronas only, as in the code | ~1–2 ms render thread for the headlight shadow map; electrical flicker; group 17 projector |
| Speed cameras always lit; no flash | fixed (`sequence_fx.rs`): behaviour lights start off; the HUD sequence 1107082 switches them on twice, brightness flash (`PostFxOverride`), sounds; world coronas (`world_coronas.rs`); 0.5 s brake-light hold | traffic-light lamps follow the junction cycle (`traffic_lamps.rs`; pole = the light group's instance id, 101 / 104 matched; unset lamps show red: model); with `--no-traffic` lamps stay dark; PostFX property 2 |
| Crash particles | added (`crash_vfx.rs`, `particle_mesh.rs`): `VfxVehicleCollisions` by zone and car type (`FUN_00584260`, `FUN_00600220`), glass cards, mesh debris, collision-mesh emitter; exponential drag and bounce for 0x3D / 0x3E | crash dust looks heavy in daylight (data; needs a capture); the wind term; a rare segfault on exit seen twice by the agent, not reproduced in 4 runs here |
| Dirt cloud too big | fixed: the game's near-camera fade (`FUN_00460970`), piecewise radius curve, wheel-controller emission slots; renderer lighting words were one off | tarmac launch smoke bigger now (by the code; capture); kick-up → `WheelContact` mapping |

Model decisions worth a capture first: the siren rule, the nitrous flame colour (lit by the irradiance only), the
corona distance scale, the 128 m water gap distance.

Added by the user (2026-10-16); the ones marked (2026-10-16: ...) were worked on this session:
- **Collectable effects**: the speed camera's flash when the player passes one is missing.
- **Gas stations**: driving through one has no clean transition: the original fades / cuts through the repair and
  respray (and refills nitrous, `FUN_00748ff0`, decomp.md "Speed Points").
- **Collectable stats UI**: the speed lists and progress counts are missing. Examples: the player's top speed at
  each speed camera, how many gates are smashed, how many jack spots are found, billboards, etc. (in the HUD
  pop-ups and the map / Easydrive screens).
- **Roof landings and out of bounds**: landing on the roof must end in the crash camera and a reset onto the
  nearest road; the same when the car lands out of bounds.
- **Car lights**: vehicles' headlights / tail lights do not light up the world properly yet.
- **Speed camera lights**: speed cameras seem to light up the world all the time instead of only during a flash.
- **Crash particles**: car crashes have few or none of the original's particle effects, e.g. broken glass when
  crashing into traffic cars and other crash debris / sparks in general (the prop-break effects of session 10's
  `prop_vfx.rs` and its super-emitter port may be reusable).
- **Missing sound effects**: smashing through billboards, hitting light poles (and props in general), some
  light crashes, landing after a jump, the cinematic crash (crash camera) sound, and many other one-off sounds.
  (2026-10-16: props', lamps', signs' and gates' sound behaviours, a car's touch of a prop (billboards), landings,
  crash contacts and the collectables' message waves added, `world_sfx.rs`; open: the crash camera's bus ducking
  and slow-motion audio, AI racers' crash zones: research log.)
- **Dirt**: the dirt cloud effect is too big and blocks the view too much; driving on dirt still does not feel
  like dirt the way the original does (handling on the surface, not only the effects).
- **Camera clipping**: the chase camera is pretty good but clips into buildings and other objects. Note: session
  8 read the original's per-frame camera path and found no world query (our push-in is `MW_CAMERA_PUSH_IN=1`,
  off by default); find what the original does instead (near-plane / fade of occluding geometry, a collision
  step outside the camera code, or the camera simply clipping too) and confirm with a capture backed into a wall.
- **Nitrous**: does not work like the original yet: using it makes the car downshift and not really go faster,
  and the gearbox then has trouble shifting (nitrous in the drivetrain / shift logic: `nitrous.rs`,
  `mw-handling`, decomp.md "Nitrous"; compare `--test nitrous` with a capture of the original).
- **Floating trees**: some trees and foliage float near the street exit leading to the large airport area (place
  not measured; compare the research log's backdrop entry, "trees in a few far places float where no zone lists
  ground under them", and `mwtool placement-check`).
- **GPU out of memory** (user crash 2026-10-19, wgpu "Out of Memory" on a mesh slab resize): a full-map run needs
  ~4.0 GB of GPU memory (`--composed-backdrop`) / ~4.3 GB (session 11's zone view); with the desktop (~2.6 GB) and the
  original or another headless run on the 8 GB card it fails near unit 168/169. Bevy's slab cap (128 MiB) made no
  difference (reverted). Fix in progress: stream units as the original does (`FUN_00549fb0`). Until then play with
  `--units 60` while agents run.
- **Wheel placement** (added 2026-10-19): some cars' wheels still sit in the wrong place, e.g. the Bugatti Veyron
  Super Sport (compare research log 2026-10-10 "wheel placement: the game builds its wheels from VehicleInfo's
  axles"; check the Veyron SS's axle data and wheel models against a screenshot of the original).
- **Missing animations** (added 2026-10-19): some of the original's animations are still missing in the game (which
  ones not listed yet; survey against the original's car / world / UI animations, `car_anim.rs`).
- **UI scaling and positioning** (added 2026-10-19): the UI's scale and placement are still wrong (HUD, menus,
  pop-ups): compare against screenshots of the original at the same resolution and take the layout from the
  game's own UI data / code, not by eye.


New this session: `mwtool water`, `mwtool terrain-check`, `mwtool siren`, `mwtool speech`, `mwtool ai-graph`,
`--test cops`, `MW_ALL_PASSES=1`, `MW_PROP_VFX_LOG=1`, `MW_VFX_DEMO`, `MW_SIREN_LOG` / `MW_SPEECH_LOG` /
`MW_SIREN_PRESS`, `MW_LIGHTBAR_SHOTS` / `MW_COP_LIGHTS` / `MW_NO_LIGHTBAR`, `MW_NO_CORONAS` / `MW_CORONA_LOG` /
`MW_VIEW_BEHIND`, `MW_POLICE_LOG`, `MW_COPS_SHOTS`, `MW_COP_DISTANCE` (CLAUDE.md).

# Handoff: 2026-10-15 (session 9, Windows)

**State**: branch `session-9` (PR to master): six worktree branches merged, each working on items of the
2026-10-14 play-test list below. Clippy 0 warnings, 178 tests, `mwtool verify` 0 failures; the 911's accel /
brake / corner results are unchanged (0–60 mph 3.58 s, 100–0 km/h 38.1 m, 2.57 g), the race test passes. The
original was not run this session: the agents worked from the dump and the data, so each area below lists the
captures / live reads that would confirm it. Every area has a 2026-10-15 research-log entry (sources, model
decisions, open items) and a decomp.md section. Also: `mwtool lods` builds on Windows again (a cfg slip).

| Play-test item | State | Main open item |
|---|---|---|
| Grass / dirt behave like road (smoke, marks) | fixed: effects and marks per wheel surface from `SurfaceVfx` and `TyreVfxBehaviour` (`FUN_00589e20`, `FUN_009cef90`) | tyre-mark vertex builder (width is a model decision, ~0.19 m); traffic's own mark path `FUN_009dfd40` |
| Tyre marks look off | partly: when / how strong from the code, per-surface material | tarmac smoke density and grass mark colour against a capture |
| Map icons missing; Most Wanted races look normal | fixed: the map's filters and item looks from `LiveMapManager_ScreenScript` 1135185 (`mwtool ui map-items`); red diamond for Most Wanted races; body shops / found jack spots on the minimap | the `Collect3` table that limits listed collectables (all 74 cameras / 216 billboards show) |
| Cops not shown as cops | fixed on map and minimap (cop arrow look) | pursuit capture: blue / red alternation |
| Navigation | ported (`gps.rs`): SET DESTINATION, A* route, re-route at 15 m off, arrival 5 m, line on map and minimap | the route line's look (renderer not found: model decision), the A* turn penalty, the race GPS line |
| Start screen shows both prompts | fixed (trigger steps apply after the pulse) | none |
| No music on the loading screen; music not matched | fixed: the game's song manager (list 1622455 everywhere, 2026345 in the intro race), title sequence music, pause fades; `mwtool music` | audio capture of boot and pause; event +0x68 song lists |
| Backfires not heard / seen | fixed: the pop rule now gets the sound's RPM offsets; crackles read at the item's position | `--test exhaust` stays at 0 (muted tests have no offsets) |
| Nitrous sounds | added: `HeatLevelSoundSet.NitrousLocal` timeline | its starter is a model decision |
| Engine start / stop | added (`ignition.rs`): cranking states and sequence from the code | the ignition key (I / L3) is ours; spawn state needs a live read |
| Idle sound missing | measured, not changed: the data's mix puts idle ~30 dB under full load | an audio capture of the original at standstill |
| Nitrous flames look off | not changed | a screenshot of the original under nitrous |
| Billboards broken by default; no records on them | fixed: standing compounds draw their +0x24 model (also lamps etc.), found billboards the WANTED poster with the offline silhouette | the poster frame's `World_GbufferBlend` program; online pictures |
| Trash bags indestructible | fixed: props without AdditionalInfo are loose (`FUN_0054dfe0`); `--smash-set bags` | none |
| Traffic horns / headlight flashing | partly: hit cars flash (3 patterns), honk (40 %), parked cars' alarms (manoeuvre 1, `traffic_reaction.rs`) | near-miss requests 1 / 3 / 5 (writer not found), manoeuvres 0 / 2 / 4 / 6, the cars' light sources |

Not started (still open from the list): police items (the branch is on the Linux machine), road / kerb harshness,
brake-to-drift at speed, water height, terrain through the road, race end screen, car-change and start-up
cutscenes, crash camera, mods / Most Wanted unlocks, takedowns in races.

New this session: `mwtool music`, `mwtool ui map-items`, `--smash-set bags`, `--map-waypoint <event>`,
`MW_GPS_LOG=1`, `MW_XINPUT_CONNECTED=0|1`, `MW_CAR_SFX_LOG=1`, `MW_NITROUS_AT=t0,t1` (`mwtool engine-sound`).

# Handoff: 2026-10-14 (session 8, part 2)

## User play test (end of 2026-10-14): what is still wrong, by area

Reported by the user after session 8; none of it is investigated yet. Each needs its source (code, data or a
capture of the original) before a fix (CLAUDE.md fidelity rule).

**Physics and world**
- Some trash bags are indestructible and very solid (props: their physics / breakable data).
- Road collisions are sometimes a bit harsh (seams / steps in the road collision; suspension contact).
- Sidewalks affect the car too much (kerb / step response).
- Brake-to-drift does not work enough at higher speeds: the car just understeers.
- In some places the water floats above where it should be (water height; see also session 7's "water level
  is too low" elsewhere).
- Terrain still sticks through the road in some places.

**Navigation, map and HUD**
- Navigation (SET DESTINATION route / GPS) does not work yet.
- Cops are not shown as cops on the map / minimap.
- Many map icons are missing: jack spots, race locations, etc.
- Most Wanted races are shown like normal races on the map.

**Input: the Steam Controller** (`steam_controller.rs`, PR #3, research log 2026-10-10)
- Works on Windows over its wireless puck. Not tried: Linux (hidraw needs Steam's udev rules), USB and Bluetooth.
- With Steam running and the game not started through it, Steam's desktop layout also sends keys and mouse moves
  from the controller (README); which keys, and whether they clash with the driving keys, is not checked.
- The reader steps aside on `SteamVirtualGamepadInfo` (Steam Input's virtual pad); not checked with the game
  added to Steam as a non-Steam game.

**Police**
- Cops have no sirens (sound) or light bars. (2026-10-16: sirens, light bars and their light added; who turns
  the AI sirens on is a model; coronas not drawn. Research log 2026-10-16.)
- No police chatter / radio sounds in chases. (2026-10-16: the radio's main events and the 5 s chatter added;
  clicks / static bed, vocabulary lines and the narrator open.)
- Crashing into other racers and police does not work yet (see the police branch, still unmerged).

**Audio**
- No music on the loading screen.
- Music is not matched to the situation (free roam vs races, etc.).

**Front end, UI and cutscenes**
- The start screen overlaps "press button to start" and "press any key to start".
- The race end screen is not quite like the original.
- The UI in general still needs work.
- The cutscenes for changing cars are missing.
- The game's start-up cutscene (entering the car, with its stats) is missing.

**Cameras**
- The crash camera is not yet quite like the original.

**Game logic and progression**
- Car mods do not work like the original (unlocked by winning races per car, etc.).
- Unlocking Most Wanted cars by beating them is missing.
- In races, neither the player nor the racers are affected by crashing into other cars (takedowns, wrecks).

**Car sounds and effects**
- Cars' idle sounds are still missing.
- Nitrous sounds are still missing.
- Backfire sounds and effects are still missing (the exhaust pop code exists: check why it is not heard / seen).
- Turning the car on and off (engine start / stop) is missing.
- Tyre marks and nitrous flames still look off.
- Surfaces: driving on grass / dirt still behaves like road: tyre smoke clouds and road-style tyre marks instead
  of the surface's own effects.

**Traffic**
- Traffic sounds and effects are mostly missing: horns and headlight flashing when you drive towards them, etc.

**Billboards**
- Billboards show the broken version by default; they should be intact with their artwork, and broken only
  after being smashed.
- Billboards do not show the Easydrive / Autolog records on them.


**State**: master has three agent branches merged on top of part 1: the post chain (bloom levels fixed
320×176 / 80×44 / 20×11, depth of field + colour cube final pass, sun-facing grade, vignette from the code, cube
blend), menu input and the map screen's controls (the original's 13 menu actions, stick / d-pad / auto-repeat,
flick / free cursor / snap from `MainMapConfig`), and lighting (the game's sun path, sky constants, ambient from
the sky, fences, 325 m shadows; the original's time of day found at environment manager + 0x1b604). Clippy 0
warnings, 166 tests, `mwtool verify` 0 failures, 911 accel / corner unchanged. Not pushed. Each area's
research-log entry (2026-10-14) lists its sources and what is open.

- **Police: NOT merged.** Branch `worktree-agent-a6c7a3ed5babae9f8` (worktree kept, commit 4d9bd0d, WIP):
  `cop_driver.rs` (chase aim / lead, LowSpeedChasing block / creep / ram, box-in, Bruiser, Rhino, HeadOn from the
  code), spawns / removal / search / heat per contact from the code, `mwtool ai-graph`. **`--test pursuit`'s bust
  phase timed out in its last run**: find why, run clippy / tests, then merge. decomp.md still says the 1.5 mph/m
  speed law applies to every cop; the code (`FUN_004fa480`) applies it only to cops without a car (fix the doc).
  Next there: roadblocks (rules read, research log), traffic-lane patrol (`FUN_005101e0`), sirens / light bars.
- **Lighting open**: the ambient's brightness (the game's `IrradianceQuadric` CPU build not traced; ours ~0.3–0.5
  vs 0.8–1.0 upward), wet surfaces without `--planar-reflection` reflect the sky cube (blue pavements, white road
  blotches: consider the planar pass by default, measure its cost), trees 2–4× too dark, 18:30 cascade fit,
  materials' own reflection cubes, a second distant matched place. Matched shots of the original:
  `ref/shots/{fairhaven,fence}-<hour>/` (with every shader constant at the moment).
- **Post open**: 22:00 frames blown out (upstream: exposure / lighting), the composite's motion blur (small),
  exposure metering from the 80×44 level, the sun lens flare (no reader found).
- **Map open**: tabs (PgUp / PgDn, LB / RB) and filters, the Most Wanted list panel, pad glyphs in prompts, the
  pause menu's QUIT item.
- The user's original-game save was rewritten by the game on load in several runs; the agents restored it or kept
  backups in `ref/savebackup-2026-10-14-*` (the post agent did not restore: its pre-run copy is
  `ref/savebackup-2026-10-14-post/`).

Older handoffs (session 8 and before): [handoffs-archive.md](handoffs-archive.md).
