# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

---

## Dated findings

### 2026-10-16: police sirens, light bars and radio chatter
User report (play test): "cops have no sounds, lights etc.". Read from the dump, REA on the decrypted image and
the data (three research sub-agents plus checks); details in decomp.md "Police sirens, light bars and radio",
layouts in formats.md "Police sounds, lights and speech". Ported in `police_fx.rs` (state, the interface),
`police_audio.rs`, `police_lightbar.rs`, `police_speech.rs`; data `mw_assets::police_{sound,lights,speech}`.
- **Sirens (found)**: `CopSoundSet` `…23013` (`VEHICLES/WAVES.BNDL`, loaded by `FUN_005776a0`); a car's set is
  its `VehicleSound` +0x40 or else `9f467b30[slot % 2]` (`FUN_00593ac0`). Siren level byte (entity +0x186c,
  0 off / 1–3 = `Sirens[level − 1].Wave[slot % n]`, looped), blip (request 5: random `Blips` for 0.8 s),
  crossfade 0.2 s / stop 0.15 s (`.bss` 0x0110d020 / 0x0110c940 read in snap1), gain 1.6, on the snapshot channel
  `Sirens_Competitor` (gain 0.501, 4–400 m, doppler c = 680 m/s) or `Player_Vehicle_Sirens` (`FUN_0058bdc0`,
  `FUN_00574510`, `FUN_00571f30`, `FUN_00572070`, `FUN_0058d770`). Requests: `SetSirenState` (message 0xd,
  `FUN_00844300` → `FUN_00572d50`: siren level 1 + bar together), the horn of a siren car (`FUN_00587720`: off →
  1 → 2 → 3 → off; also the **player's horn runtime**, the old "not found"), cutscene cars
  (`AnimatedRaceCarBehaviour` +0x54 / +0x5a, `FUN_005b4750`). Dying siren `89ecdfc4` (`…b3883`) once when a car
  with its siren on is wrecked. Waves (pitch tracks): wail `…2301e` (5 s, 656 → 1641 → 680 Hz), yelps `…23016` /
  `…23022` / `…23023` (600–1700 Hz, ~3 sweeps / s), hi-lo `…2301b` / `…2301c` (1031 / 1559 Hz).
- **Light bars (found)**: effect group 3 (entity +0x1c27); `VehicleVfxBehaviour` `…_COP_Coronas` (19 coronas
  with `FlashAnimation`s) and two `Light_Custom`s (blue / red, intensity 35). Flash: `FUN_0049ef00` (segment by
  `frac(t / ΣDuration)`, sample at `(Frequency · t + Phase) · n`, linear), per-car rate
  `frac(slot × 0.39 + 0.5) × 0.2 + 0.9`. The bar mesh's `_LocalEmissive` program: 16 local lights (slot 0x48 /
  0x49) from the coronas' values ^10 at camera-facing locators (`FUN_0057fcf0`), R = 222.767. The bar's world
  light uses `LightBuffer_Cop` (diffuse only, vertical fade). Cops' head / tail lights are always on (+0x1a8c).
- **Radio (found)**: `EN_US/SPEECH/SPEECHEVENTSEQUENCES.BNDL` (110 events) + the SWC0 container (7,665 SPS
  streams, `WaveContainerTable` index), one speech at a time by priority (`FUN_008408a0`, `FUN_008406a0`,
  `FUN_0083d270`), triggers from the scoring actions' `GameplayTrigger`s and the HSM "Speech Manager" (983342).
- **Model decisions**: (1) what switches an AI cop's siren in a pursuit was not found (only the paths above write
  the requests): chasing / searching cops in a pursuit or cooldown have `SetSirenState(on)`, others off; (2) the
  three nearest sirens sound (`Sirens_Competitor.1653b84c` = 3 read as a voice limit); (3) speech conditions
  whose binding we lack read 0; lines play on the effects volume; (4) the busted lines 1186445 / 1186447
  (priority 0, play-once, no trigger found).
- **Checked**: `mwtool siren out.wav` (set `…23039`: off, levels 1–3, blip, level 1: peaks follow the waves,
  no clicks at the switches: max |Δ| 0.25 vs p99.99 0.13); `MW_SIREN_PRESS=5,10,15,20,25,30` with the cop car
  as the player's: levels 1 → 2 → 3 → off → 1, the voice at −12 dBFS at 7.6 m, silent when off;
  `--test pursuit` with `MW_SIREN_LOG=1 MW_AUDIO_METER=1 MW_SPEECH_LOG=1`: each chasing cop's siren starts with
  the chase (sets by slot parity), placed (gain −7..−12 dB at 7–60 m, doppler 0.95–1.05), a wrecked cop's
  dying siren, the chase-start radio event, chatter every 5 s after a line; speech lines decode band-limited
  (99.4–99.9 % of the energy in 300–3400 Hz, peaks −0.3 dBFS, no discontinuities). Night screenshots of the bar
  (`MW_LIGHTBAR_SHOTS`, 1/8 s apart): blue / red hotspots alternating on the bar and the rear deck, the bar's
  blue / red light on the road and a sign.
- **Open / captures wanted**: (a) a probe watch on entity +0x1950 / +0x186c during a chase (who turns the AI
  sirens on); (b) a capture of a chase at night (light bar look, the bar's light on the road and signs: ours may
  be too strong on the retroreflective signs; the coronas are not drawn: `FUN_004a5790` / `FUN_004ab450` not
  decoded); (c) the siren's pitch wobble after impacts (`FUN_00aabc10`) and its factor B+0x28; (d) the speech's
  radio clicks and static bed (`AudioSettings` +0x18 / +0x1c), vocabulary slots (car model / colour), narrator
  one-offs, the loudhailer's 3D placement, the busted lines, and whether a line item waits for the previous line
  (ours: items start at their `StartTime` and may overlap); (e) the `Racer_Cop_Radio` parameters (no reader).

### 2026-10-15: navigation (SET DESTINATION, the GPS route) from the code
User play test: "Navigation (SET DESTINATION route / GPS) does not work yet". Details in decomp.md "Navigation: SET
DESTINATION and the GPS route".
- **Found**: `Map:OnSetWaypoint` is dispatched by its name hash (static initialiser 0x00f68cf0 → `.bss` 0x0117f49c)
  in the map's action handler (0x008188e0) to `FUN_0080eb60`: events route to their start trigger, collectables
  to their trigger. The route lives in `UIMapManager` (G+0x2f480) +0x1f8f0: three route objects (waypoint, the
  race's GPS line, the shown one). The search is the AI's A* (path request type 9, `FUN_00843240`, both route
  options on); when moving, the start is moved two road nodes ahead along the velocity. Line: ≤ 512 points
  with the node's drivable width, u along the path and side vectors. Every 0.5 s the car is projected onto the
  path; 15 m off → re-route; progress refined per frame within ±50 m; arrival within 5 m clears all routes.
  The A* turn penalty is 5·max(0, 0.98 − cos Δ)³ (the trig was open: it is a cosine), multiplied by the × 5 / × 100
  route-option factors; the exit-angle reference is still open.
- **Ported** (`gps.rs`): SET DESTINATION on the map for events and (new) every collectable's option menu, the
  route on a worker thread, the line, re-routing, progress, arrival, and drawing on the world map and the minimap.
  `--map-waypoint <event id>` chooses SET DESTINATION headlessly (with `--show-map` the map stays open);
  `MW_GPS_LOG=1` logs requests, routes, the 0.5 s checks and re-routes.
- **Checked**: CONTINENTAL DRIFT (1.2 km) and HARBOR RUN (6.4 km) from the default spawn: the map screen draws
  the route along the roads to the event, the minimap shows it from the car; with the car moving up the road
  the progress follows (logged every 0.5 s: along / off / left). At the default spawn the car is 22 m from the
  first route's nearest point (it starts at a node behind), so the first check re-routes, as the code would.
  Unit tests: line decimation / u / side, nearest point and progress.
- **Open**: (1) how the original draws the route (colour, width, texture, whether the driven part is hidden):
  the renderer was not found in the decompiled set; **capture wanted**: a Steam F12 screenshot of the original's
  minimap and map with a destination set (e.g. HARBOR RUN from the start point). (2) The A* turn penalty's angle
  reference (data check: AIData junction exit angles against the exits' geometric headings), then the penalty and
  the route-option factors can be ported; without them our routes may make U-turns the original avoids (seen once
  after the start-point re-route). (3) The path expansion `FUN_00520a50` (ours: the AI's spline samples). (4) The
  race GPS line (second route, `FUN_007a0e40`, next min(8, perLap − 1) gates) and its chaining into the shown
  route are not ported. (5) Lua hooks `SetRouteFromPlayerToEvent` / `ToSafehouse` / `ClearTargetRoute` /
  `SetWaypointToJackspot` not read.

### 2026-10-15: traffic reactions: the headlight flash, the hit horn and car alarms (manoeuvre 1) ported
User report (play test 2026-10-14): traffic sounds and effects mostly missing, "horns and headlight flashing when
you drive towards them". Read from the dump and the disassembly; details in decomp.md "Traffic reactions
(2026-10-15)"; ported in `traffic_reaction.rs` (engine-free, 3 unit tests), `traffic.rs` (`crash_hit_cars`,
`reaction_lamps`) and `car_sfx.rs` (hit horn, alarm).
- **Manoeuvres** are only for physical traffic: a car that gets a world vehicle (`FUN_0061cf50` →
  `FUN_00609b70`) with a *request type* scores 11 manoeuvres (`FUN_006071d0` / `FUN_005eac20`). The 11 objects at
  `world+0x339d0` (vtables set by `FUN_00420720`) map index → update: 0 `FUN_00604290`, 1 `FUN_005f5c40`, 2
  `FUN_005f77e0` (swerve), 3 `FUN_005f6150`, 4 `FUN_005f6e30`, 5 `FUN_005f65a0` (jack-knife), 6 `FUN_005ffc10`, 7
  `FUN_005f6aa0`, 8 `FUN_005f73a0`, 9 `FUN_005ebca0`, 10 `FUN_005e9260` (wrecked). Car byte +0x6f holds the index.
- **Who reacts how**: a contact turns a lane follower physical (`FUN_0060c3a0`'s loop) with request 4 or 6
  (`FUN_005f2eb0`): 4 when the contact closes faster than 30 mph, the traffic car moves faster than 40 mph and the
  contact lies in its box zone 5–9 (side / rear, `FUN_00929da0`); else 6. Request 6 (and 2: every parked car, and 0
  for a car with a trailer) picks **manoeuvre 1**; request 4 picks 6 / 2 / 4 by an input at +0xd0 (> 0.66 / > 0.33 /
  else; input not traced). The arbiter's option 0 queues request 0 → manoeuvre 0 (no flash). So the flash and the
  "hit horn" belong to cars bumped from the front or slowly, not to cars merely approached; approaching cars only
  get the existing "stop for obstacle" horn (40 %, once per 55 s; ported 2026-10-09).
- **Manoeuvre 1** (`FUN_005f5c40`): brakes / steers away; the car's horn timer is reset and it wants its horn (bit
  0) when its random +0x5c ≤ `TrafficParamConfig` +0x20 (0.4); a parked car that is moving sets the alarm bit
  0x10 instead. It ends when the car's physical time (+0x200, += dt in `FUN_005ffa10`) passes 5 s (`.rdata`
  0x00fb278c) for a lane car (→ manoeuvre 0, or 10 when the damage bar is full); parked cars stay in it.
- **Headlight flash**: while +0x6f = 1, `FUN_00ebe8c0` starts `FUN_00614620` (pattern = LCG high word % 3), restarted
  whenever a pattern ends. Patterns (`.rdata` 0x00fb2168, seconds; even steps dark, odd steps lit, −1 = end):
  0.15 0.30 0.13 0.25 0.14 0.25 0.18 | 0.12 0.20 0.10 0.20 0.09 0.16 0.21 0.20 0.18 (holds dark at step 8) | 0.16
  0.75 0.22 0.25 0.16 0.32 0.13. `FUN_005f53b0` ramps the brightness byte +0x64 to 255 / 0 by
  `TrafficVehicleConfig` +0x28 (5000 /s) × dt. The car's light sources are drawn (`FUN_005fd190`) at intensity
  `0.7 + 0.5 × b/255` (0 for parked cars and manoeuvre 10), per car × `0.9 + 0.2 × random`. Our renderer draws no
  car light sources (world_lights.rs: `VehicleVfxBehaviour.Lights` not drawn), so the flash needs those first.
- **Car alarm** (`FUN_006042c0`): bit 0x10 (or a proximity test `FUN_005f2740`, 10.5 m, not traced) plays
  `MiscSound.c1109b6e` when the car's `TrafficVehicle` +0x3c flag (`172df888`, true on 7 of 11 types) is set and
  `(car index % 10) × 0.1 ≤ MiscSound` +0xe0 (0.4), then hazard-blinks (0x20: bit 8 toggles every 0.4 s between
  0.5 s and 15 s of +0x4c).
- **Indicators** (same functions): lane car +0x72 / +0x8 / +0xc pick left (0x40 → bit 2) / right (0x80 → bit 4),
  brightness +0x65; the blink period reads `TrafficVehicleConfig` +0x80 / +0xac (0.012 / 0.022, too short for
  seconds: unit open).
- **Ported**: a crashed lane car gets its request from the hit's zone on its box and the closing speed; requests
  2 / 6 run manoeuvre 1 (5 s for lane cars, for ever for parked ones): the flash patterns and the brightness
  ramp as read, the hit horn through the two horn slots, the alarm (8 s, `(slot % 10) × 0.1 ≤ 0.4`, types with
  the flag, once per car). Model decisions: the random +0x5c and the pattern draws come from one second LCG with
  our seed; the contact speed is the closing speed along the contact normal; "moving" is the 0.5 m/s standing
  test without the direction term; the behaviour's 150 m radius is not applied to the lights; request 4 gets no
  reaction (manoeuvres 2 / 4 / 6 not ported); and, since car light sources are not drawn, the flash scales the
  lamps' `HeadlightColour` emission by the game's intensity ratio `(0.7 + 0.5 b/255) / 0.7` (up to 1.71) on the
  flashing car's own material copies.
- **Checks**: unit tests (pattern 0 = three lit phases ending dark, zones, request rule, manoeuvre 1 ends at 5 s).
  `--test crash-traffic`: head-on, zone 3, closing 128 km/h, request 6, the pickup's 3 lamp materials flash (its
  random 0.62 > 0.4: no hit horn). Free roam at full throttle (`--spawn=800,42,1688,71 --auto-throttle 1
  --bench 75`): five hits (zones 2, 4, 4, 5, 7; all request 6, the zone-5 one closed at 3 km/h), every car
  flashes; the two with random 0.10 / 0.13 honk right after (horn at 16 m / 32 m, gain 0 dB). Alarm data loads
  (`MiscSound.c1109b6e`, share 0.4); no parked car was hit in these runs (alarm not seen in game).
- Open: the per-player request list (`world+0x36120 + p·0x680`, entries at +0x6b0, count +0x770) consumed by
  `FUN_0060e030` (its writer, and so request types 1 / 3 / 5, not found: possibly the near-miss / panic reactions to
  an approaching player); message 0x10 requests (`FUN_005efdd0`); manoeuvre 0's behaviour; how +0x64 / bit 8 reach
  the lamp materials (hazard lights of parked cars); the input +0xd0; the hazard
  blink and indicators; car light sources (then the flash drives their intensity instead of the lamp glow). A
  capture of the original (bump a stopped car head-on at night, F12 burst) would confirm the flash look.

### 2026-10-15: surface effects and tyre marks from the code (smoke, kick-up, marks per wheel surface)

The play test said grass and dirt behave like road for the effects and the tyre marks look off. Cause: every
effect used surface 0 / the first surface with marks, although the wheels already know their surface (the
collision tag). Now traced in the dump (docs/decomp.md "Tyre marks and tyre smoke") and ported:

- **Which effect** (`FUN_00589e20`, the race car's per-wheel effect update; `FUN_0054aaa0` looks up the
  surface's `SurfaceVfx`, dry block, the wet one in the wet state): smoke = `PlayerWheelEffects` (+0xa0; AI cars
  `AIWheelEffects` +0x40) `DefaultTyre` (`OnRims` on rims, the `61ac1863` variant when `FUN_006a15c0(…, 0xb) ==
  2`); a second kick-up = `WheelVfx` (+0x138; AI `4cf3cf8f` +0x140); tyre marks = `TyreMarksMaterials` by the
  car's tyre class (0 `Offroad`, 1 `eff16a09`, 2 `Road`, 3 `Sport`, 4 `Track`; class from car +0x1a88 through a
  per-car short at +0x6c / +0x6e / +0x70), `RimMarksMaterial` (+0xe8) on rims. `SurfaceVfx` field offsets
  (handles from +0x40, 8 bytes each, sorted by name) are in formats.md.
- **When** (same function): smoke while the wheel is on the ground and any of its three smoke ramps is above
  0.01 (`.rdata` 0x00fad910); the ramps go to the effect as its `WheelParams`. Tyre marks are laid by **every
  wheel on the ground**, all the time; the strip's per-point weights are Slip = the `Skid_Marks` lateral ramp,
  Spin = max(spin ramp, lock ramp). What shows is decided by the material: tarmac's `Run` layer is fully
  transparent (marks only from slip / spin), grass and dirt use `…117f02` (`Run` transparent too, brown `Slip`
  `…f58ca` / `Spin` `…117f26` layers).
- **The ramps** (`FUN_009cef90`, per wheel and tick, record at vehicle +0x2c90 + 0x30 i; lanes 0..2 from the
  axle's `TyreVfxBehaviour.Smoke`, lanes 4, 5, 7 from `Skid_Marks`, identified by which reader uses which
  lanes): spin = clamp((s − `1281cf25`) / (`6a55fbd2` − `1281cf25`))^`Curve_Power` × `35caad30`, s = |tread −
  ground| mph; lateral = the same over |α| (`6a83c966` → `53b71e27`) × a window on the car's mph (`26028ef5` →
  `2c72e92a`); lock (`64222c3f`) = the spin form, counted only while wheel +0x108 < −0.1 (spin only at or above
  it; `.data` 0x011870d0, all lanes −0.1); each smoothed by `6274d1ad` (rising) / `27c5dd70` (falling) × dt × 60
  of the gap. `LatParams` / `LongParams` field roles are from these offsets (names beyond `Curve_Power` not
  cracked). The dirt build-up (`FUN_009d19c0`) reads the smoke lanes.
- **Other vehicles** (`FUN_00606210` over up to 431 slots, likely traffic): marks only when `FUN_009dfd40` says
  so: m = max(wheel +0x130, clamp((+0x118 − 0.5) / 0.5) × 0.55, clamp((+0x134 − 0.1) / 0.9)), and on polygons
  without the `ROAD` collision flag (bit 1 of the tag) at least max(+0x130, 0.5); laid while m > 0.1 and the wheel
  is not on rims; material = `TyreMarksMaterials.Road` (+0x28) always. Its kick-up is `SurfaceVfx` +0x118
  (`01d1b4fd`) or +0x120 (`f967c1ce`) into the shared `SurfaceFx` wheel-contact list (`FUN_0049ba00`). Inputs
  +0x118 / +0x130 / +0x134 (from wheel +0xd8, +0x110, +0xdc) are not identified. Not ported (we draw marks for
  the player only).
- **Built**: `mw_handling::tyre_vfx` (the ramps, unit tests on the 911's numbers), `mw_assets::handling`
  reads `TyreVfxBehaviour` (one per vehicle bundle, 188 over the install, 93 of them `…68fe4`), `drive.rs` carries surface index, ramps,
  tyre width and lateral axis per contact, `particles.rs` / `tyre_marks.rs` / `paint_wear.rs` use them.
  Model decisions: wheel +0x108 read as κ (the sign test against −0.1 splits spin from lock); the effect's
  `Intensity` = max of the three smoke ramps (the spawner's binding slots are not decoded); a strip point every
  0.25 m; the strip's across vector = the tyre's lateral axis × the tyre width (2 × the wheel body's half
  extent; the game passes wheel +0xbc with each point and its vertex builder was not found), so the drawn width
  is `WidthScale` (0.65) × the width (0.19 m on a 911 rear instead of 0.65 m); strips break on a change of
  material; zero-weight points on a transparent-`Run` material are not stored; ramps of a wheel in the air head
  for 0; impacts still use the default surface.
- **Checked**: `--test drift --track-surface 8`: marks from launch wheelspin (spin 0.15) and through the drift
  (rear slip 0.6, spin 0.4–0.5), dense white smoke; `--track-surface 4` (grass): brown dust clumps
  (`…a39d7`) and brown strips; surface 0 (the default track): smoke, no marks (its `SurfaceVfx` has none, so a
  test needing marks wants `--track-surface 8`); world launch on surface 2 (paving) gives its dusty smoke.
  Screenshots in `ref/fx/` (not committed). Drift test result unchanged (peak 36.4°, held 4.73 s).
- Open: the tarmac smoke looks very dense next to the original (compare with a capture); the grass marks' colour
  against real grass; the tyre-mark vertex builder (consumer of the 0x50-byte records queued by
  `FUN_004af040`: the meaning of wheel +0xbc, U, the age in POSITION.w and the `Animation` fade); the tyre
  class → mark slot table (+0x6c..0x70 of the object at car +0xd0); `SurfaceVfx` +0x100 (`bb02333c`, started for
  the local player's rear wheels with a camera-space transform: maybe lens dirt) and +0x58 (`5830a883`, copied to
  the car at +0x3c0). **Live read wanted** (not done: the original was not to be launched this session): wheel
  +0x108 / +0x110 / +0xd8 / +0xdc and the ramp record at vehicle +0x2c90 while launching, locking and drifting,
  to confirm κ and the inputs of `FUN_009dfd40`.

### 2026-10-15: map and minimap icons: item types, filters, Most Wanted races, jack spots, cops
User play test: "many map icons are missing: jack spots, race locations, etc.", "Most Wanted races are shown like
normal races on the map", "cops are not shown as cops on the map / minimap". Details in decomp.md "The world map's
items and filters; the minimap's free-roam icons".
- **Where the map's item kinds live**: not in the exe and not in `MainMapConfig`: the screen script
  `LiveMapManager_ScreenScript` 1135185 (found by scanning a dump of every GenesysObject for the look key
  1135494) has `MapItemTypes` (icon and card key per item type), `CollectablesName`, `AICopPOI` / `AIPlayerPOI` /
  `BlackListDriver` / `CheckpointPOI` / `FinishLinePOI` / `SelectionPOI`; `FUN_0080ec90` reads them by these names
  (strings in the pseudo-C; the hashes match the dump's field hashes). New: `AssetStore::map_manager_config`,
  `map_filters`, `mwtool ui map-items`.
- **Most Wanted races**: `FUN_008154f0` files an event as `MapItemTypes[9]` (key 955947, the red diamond `0e962f`)
  when its +0xe9 (`c8f26580`, first place only) is set, else `[1]` (the green diamond). The 2026-10-10 reading
  ("red = beaten", the `MAP_EVENT_DONE` step) was wrong for the capture's red icons: they are the Most Wanted
  races' own look. Fixed: our map drew every event green.
- **Filters**: the map shows one list at a time: RACES, JACK SPOTS, RECOMMENDS, SPEED CAMERAS, BILLBOARDS (group
  1135638), Z / A (LT / RT) step through them (`FUN_00818550`); each filter's list from `FUN_008181b0` (trigger
  kinds per filter). Ported; jack spots appear once found (the icon's `JackSpots.IsSwapSpotFound[#]`).
- **Minimap**: free-roam collectables get a point of interest within 400 m (removed beyond 450 m; `FUN_006ffd90`,
  `FUN_006ffba0`): body shops (the wrench, always) and jack spots (the wheel, once found) have map icons; speed
  cameras and billboards have world looks only. Ported. Cops were drawn with the racers' orange arrow because
  `Players.PlayerBase[#].IsCop` was forced false; the look's own cop elements (blue / red) now show.
- **Checked** (headless, screenshots in the worktree's git-ignored `ref/map1/`, looked at): the map's RACES
  (green and red diamonds; the cops of a `MW_PURSUIT_DEMO` chase beside the player), JACK SPOTS (the three found
  spots of a test profile), SPEED CAMERAS (74 orange dots; a flick selects one and shows its card "SPEEDWALL /
  SPEED CAMERA" with SELECT), BILLBOARDS (216); the minimap at (310, 420): the body shop's wrench under the car, a
  found jack spot's wheel 170 m away, the patrol cop's red arrow. `mwtool ui map-items` prints the table.
- **Dead ends**: the HUD screens' event looks with world layouts (1013512, 955952, 955955, 1013520, 1132924) are
  named by no code (image scan for the immediates) and no GenesysObject (full dump scan): no event icons on the
  minimap. `FUN_00ac54c0(0x5f3f3)` is a type test (the event's type chain contains 390131), not a game flag.
- **Model decisions**: the `Collect3` table (`FUN_00728b20`, the collectables the map lists) is not read: every
  trigger of the kinds counts (74 speed cameras and 216 billboards include the packs' kinds; the base game has
  fewer); `FUN_006fcf90` (a profile list by the spot's +0x24; puts a spot into item 14 on the map and hides it
  from the HUD) never holds; jack spots not found are not selectable; RECOMMENDS is empty; the filter index is not
  saved; collectables' option menus (`SpeedCamera`, `Jump`, `Jackspot`) are not wired; events keep the measured
  card 1481372 rather than item type 1's 1013511.
- **Open / captures wanted from the original**: (1) the map's SPEED CAMERAS and BILLBOARDS lists on a known save
  (count and which, to settle `Collect3` and the packs' kinds); (2) JACK SPOTS on a save with found and swapped
  spots (what item 14 / `FUN_006fcf90` is: is a found spot shown before its car is driven); (3) the minimap near a
  body shop and a found jack spot (icon size and the 400 m range); (4) the minimap and map in a pursuit (cop
  arrow: blue / red alternation; whether patrolling cops show outside a pursuit); (5) the selected speed camera's
  card and its options; (6) where the in-world jack spot marker (look 1013508's world layouts 186088 / 195581) and
  the speed camera / billboard world looks (1013522, 1013491) are drawn: not ported.

### 2026-10-15: title prompt pair, music by situation (user play test: front end, audio)

- **Prompt overlap**: layout 1274392's labels 1597779 ("PRESS START", hidden on
  `Controls.IsXInputControllerConnected(NOT)`) and 1274432 (`ComplianceTerm.Value["PressStart"]` = "PRESS ANY
  KEY", hidden on `Controls.IsXInputControllerConnected`) hide each other with an instant `OUT` idle step on a
  triggered behaviour; our tween applied the element's own looping pulse after it, whose `IN` undid the hide, so
  both showed. Triggered idles now apply last (our reading of the order: the tween code is not traced). Checked
  with front-end shots with and without a pad (`MW_XINPUT_CONNECTED=0|1` overrides the check; no pad was
  connected to this machine during the runs).
- **Music**: decomp.md "Music" has the code (song manager states, start index, play order, the situation bias of
  the song float `bc7081c6`, `PLAYSONGLIST`, `MUTE_SOUND` = the world bus only, `MUSIC_ADVANCE` = M, the
  DynamicMusicManager does nothing for pursuits); formats.md "Audio" the data (`SongList`, `Song`, Wave prefetch +
  streamed file, the music sequence items). Before: every `UI/SONGS/*.SPS` shuffled (UI loops, credits and the
  Picture Paradise list included), all sound muted on the front end. Now: the title's own waves, the
  transition's whoosh and looping bed, free roam and events on list 1622455 with the game's ordering, M / RB
  skips, the in-game menu fades the music, music stays on over loading screens.
- **Verified**: `mwtool music` (lists, game modes' `MusicPlaylist`), wave decodes (title intro 45.39 s = its
  45.385 s item, loop 26.10 s with the `H` loop flag, no silent seconds; prefetch + file concatenated), a unit
  test of the ordering (a pass plays every song once; free drive puts the f < 0 songs first, events the f > 0),
  runs with `MW_MUSIC_LOG=1`: front end (title intro -> transition whoosh + bed -> list 1622455, first song index
  1 on a new profile), `--drive --show-pause` with a scripted M tap (skipped; a 1.5 s hold did not), the pause
  menu fading the level to 0 and back.
- **Open / wanted from the original**: an audio capture of the boot (title -> transition -> first song) and of a
  pause / resume to confirm levels and the fade back; whether a new profile's first song is list index 1 (Calvin
  Harris) as the code reads; profile option 5 (shuffle) default (a live read of the profile options); event
  +0x68 (an event's own song list); the "now playing" popup (FeedbackMessage 30567) is not shown; the Autolog
  player's music (116490 / sequence 1402320), credits and cutscene `SetSongList` items are not played; the
  in-game loading screen's 0.1 s mute ramp.

### 2026-10-15: car sounds: idle, nitrous, backfires, engine start / stop (play test "Car sounds and effects")
- **Backfires never happened in normal driving** (cause found, fixed): the pop rule `FUN_009b20b0` reads the
  sound's audible RPM +0x1f00 = `Idle + (Max − Idle) × +0x1f10`, and +0x1f10 already holds the RPM offsets the
  engine sequences post (effect property 1, `FUN_009aea50`). Our rule used the physics curve without them. The 911's
  upshift variant `…1d1a22` kicks the RPM +0.2 at engagement (shift clock 1..1.25) and drops it from ~0.13 to 0 in
  one tick at the item's end, which is the drop that backfires (`1 − 0.13 · r · 10 < 0.1` for r > 0.69). Now the
  player's rule gets the engine driver's offset and load (`Exhaust::set_sound`). Full throttle on the test track
  (20 s, 911): 0 → 1 backfire, 40 pops; the kick's frame shows the normalised RPM 0.796 → 0.660. Every third
  911 upshift has the kick (round robin), so ~1 in 9 upshifts backfires with its 0.1; the BMW M3 GTR (0.65) and
  the McLarens far more. `--test exhaust` is muted (no engine sound, so no offsets): still 0 backfires there.
- **Pops / crackles were silent** (fixed): the 911 has no `Pop_Sequence` (101 of 283 banks lack one; `dacf5fd5`,
  between `Ignition_Sequence` and `Pop_Sequence`, is never set). Its pops are the `Gurgle_Sequence` (`…1d1c96`,
  bound to `GetPopProbability`: pop waves 0.75..1, crackles 0.05..0.85), which the engine sound starts with the
  bank's `Sequences` at sound start (the sound's state 2 → 3 code in `00580000.c` after `FUN_00575340`: `FUN_005866f0` of
  `EngineSound2` +0x14, kept at +0x1ac). Its items have `Trigger_Probability` curves that start at 0, and the
  player read every one-shot's probability (and gain / pitch) at item position 0, so they never fired. Now read
  at the item position (`sequence::Player::start_voice`, same in `sfx_player`). Off-throttle at 6500 rpm the
  crackle channels play (−30 / −34 dB average) ~33 dB under the engine; the retrigger rate (12/s × value) is
  still our model. `6001d10d` on wave items (1 on every item with a probability curve, 2 / 3 elsewhere) is unread.
- **Nitrous sound** (new): `HeatLevelSoundSet.NitrousLocal` (`…181ffc` in `GAMELOGIC/HL_31208.BNDL`, the only
  heat-level sound bundle; `FUN_0065fee0` builds `HL_%d`): a 12 s timeline, burn wave `…1b0ab3` (1..8.885 s,
  gain fading in over ~0.9 s) and tail `…1b0ab2` (8.7..10.594) on channel `…7533`, ducking the engine channels
  (`…b381c` ×0.67, harmonic EQs, comb rasp, whine), +0.03 RPM, PostFX items, and eight timeline controllers on
  `Players.LocalPlayer.NitrousState.UsingNitrous`. `JumpTimelineController.b6a51260` = **DestinationTime**
  (cracked; sorts before `TriggerTime`). `2d0e4348` (bool, not cracked) read as "monitor": fire on the binding
  turning true at any time past `TriggerTime`, else only when the clock crosses it; the only reading under which
  all eight make sense (loop the burn 8.75 → 10.25 → 1 while held, quit on a tap under 1.5 s, tail on release
  after 2 s, restart on a new press). Merged into the player's engine bank; started on the rising edge of the
  nitrous input (its starter is not in the dump: model). Offline (`MW_NITROUS_AT=t0,t1 mwtool engine-sound`):
  a 1 s use plays and quits by +1.25 s; a 4 s use plays the burn and the tail ~1 s after release.
  `9b519cdb` (cracks as `NitrousDump`, unchecked) holds `…0b3a2a`, a 3 s timeline (wave `…1b0a36`); not used.
- **Engine start / stop** (new, `ignition.rs`): the gearbox's start-up states 0 (off) and 1 (cranking)
  (`FUN_009ce3d0`, `FUN_009cd7a0`): off → cranking after 0.5 s off (`.bss` 0x01186ef0 ← `.rdata` 0x01026050) on
  throttle / brake; cranking → running when `Ignition_Sequence` posts property 7 (+0x1be5, `FUN_008f5490` case 7)
  or has ended (+0x1be4 ← sound +0x1c8, `FUN_00900120`); `FUN_0058c930` starts the ignition in state 1 and stops
  it in state 0. `EngineOn` (+0x1ef0, the engine buses' gain binding) moves 0.05 per tick (`.bss` 0x011866c0 ←
  `.rdata` 0x01025370) towards 1 while running, 0 otherwise (`FUN_009b3030`). Catch times over the 283 banks:
  0.25..1.5 s (the 911 0.25). The ignition wave plays as a one-shot on its snapshot channel `Player Vehicle
  Ignition` like the horn (`AssetStore::player_ignition`); in the bank it reached the master at −48 dBFS, under
  the idle. Checked: runtime log "player ignition" started at spawn, out −42 dBFS at 6 m, "engine running" 0.24 s
  later. Model decisions: the player's car spawns cranking in play (running in `--test` / `--replay`); IGNITION
  (action 17: I / L3) turns the engine off and starts it again (its reader was not found; ENGINE_START / OFF
  39 / 40 are unbound); while off / cranking the throttle is held at 0 (the clutch never closes there) and the
  physics engine idles silently. Message 0x1e (`FUN_00587490`, script `FUN_0068b070`) forces state 0 with
  +0x1be3 (the "held off" flag); not wired.
- **Idle** (measured, not changed): the idle loop plays (911: `…1d1c7a` / `…1d1c7b` at 1790..2290 audible rpm,
  gain 0.3, faded out over 500 rpm), but in game the load sits at ~0.19 (the idle governor) and the whole engine
  is −39.5 dBFS at idle against −7.6 at full throttle (bus inputs −15 vs +12 dB: the data's own balance). With
  the engine volume (0.6) and music at −28 dBFS the idle is ~16 dB under the music, which is likely what reads as
  "missing". Open: the engine-vs-music mix and the `SendDspPlugIn.Bus_Routing` (0 / 1 / 3) buses (we sum only the
  first send to "none" into the master; `472497`, routing 1, is muted between load 0.2 and 0.54 by the load
  curves); a capture of the original standing still (audio) settles it.
- **Nitrous flames** (looked at, not changed): `--test nitrous` shows the flame effect `…24f34` at both pipes, but
  sitting low and short behind the tips; the original's cone emitter axis (+Z inferred, 2026-10-09) and the
  Bear emitter maths are still unread. Needs a screenshot of the original under nitrous at a known spot.
- **Also found**: `FUN_0058d1c0` plays a random sequence of `EngineSound2` `c99783d2` (+0x60) when the throttle
  pedal (entity +0x192c) rises by more than 0.3 in a frame and `6fd79c8d` (+0x5c) when it falls by more than 0.3
  (`.rdata` 0x00fac27c / 0x00fae1f8), in gears above 1 (`FUN_0090d460`): driveline shunt clunks; not ported.
- **Open / live reads wanted**: (1) entity +0x1a84 (requested engine state 0 / 1 / 2) and vehicle +0x1bd8 right
  after a car spawn and after a jack-spot swap, to confirm the spawn state; (2) the IGNITION key's effect in the
  original (does I stop the engine?); (3) audio capture at standstill for the idle level; (4) a nitrous
  screenshot for the flames.

### 2026-10-15: billboards' three looks and props without an AdditionalInfo (play-test items)

Read from the dump and the data (no live reads; the original was not run). Details in decomp.md "Compound
models: standing, found, broken".

- **Billboards showed their broken model.** A `CompoundObject` carries three models: +0x04 (skinned to the
  parts, the billboard's torn-poster panels), +0x24 and sometimes +0x28. The standing object draws its +0x10,
  which `FUN_005396d0` (stream-in) / `FUN_00533a00` set to +0x24 while the object's flag +0x10f is clear and
  to +0x28 when it is set (each falling back to +0x04 when empty); `FUN_00532430` (the break, +0x10d = 1)
  switches to +0x04. We drew +0x04 standing. Now every standing compound draws +0x24 under its own LODs
  (so street lamps etc. too: +0x24 is not a distance LOD as guessed on 2026-09; it is the standing model) and
  the pieces only after the smash. Checked: screenshots of billboard 1496888 (BioWare advert), the
  `--smash-set compounds` run (lamps stand, the billboard 0xac10f734 shows its DICE/BioWare advert and breaks
  into torn panels), `--test freeroam` results unchanged.
- **+0x10f is "found"**: `FUN_00554230`'s last argument is a binary search of the instance id (+0x4c, the
  trigger id) in the list `FUN_006a0400` returns; that list (`+0x22c8`, count `+0x26b0`) is filled from the
  profile's `Collect3` table ("TriggerID" column, `FUN_00728b20`) with billboard kinds ('2' 'A' 'D' 'J' 'M')
  only. So a found billboard stands as +0x28: a "WANTED / Fairhaven Police Department" poster whose picture is
  `World_SmashedBillboard_InstanceDiffuse_Specular_Singlesided` (G-buffer program identical to
  World_Diffuse_Specular's) with its diffuse from the **instance**: every billboard's `CompoundInstanceList`
  record has an override block at +0x54 (same layout as `InstanceList`'s) binding `DiffuseSampler` = texture
  `0100000000141784`, a silhouette on a torn poster (216 of 216; no other compound has a block). That is what
  the game draws offline / without Autolog friends; online pictures come through the `BillboardImages`
  allocator and the `billboards` / `advert_contentid` online settings (not traced). Done in `freeroam.rs`
  (`billboard_faces`): found = in the profile's billboard list. Model decision: the game picks the face at
  stream-in only; ours picks it when the list is first known and while the compound is broken (so a newly
  found billboard shows the poster once it is put back).
- **Indestructible trash bags**: 19 prop and 3 dynamic definitions have no `AdditionalInfo` (the trash bag
  `PropObject` 03102fec, 10 kg, 353 placements; crates of 0.2-15 kg; two 35 kg 2.6 m objects; one 2500 kg).
  We treated them as never knockable (a model decision). `FUN_0054dfe0` clears the pinned bit when the
  definition's info pointer (+0x30) is null, and `FUN_0054e570` / `FUN_0054bf70` skip their info reads: they
  are loose (asleep) dynamic bodies from the start. Fixed (`PropPhysics::break_force` 0); `--smash-set bags`
  drives through the bag and two crates: all three knocked, 0.6 km/h lost. (The set also fixed the smash test
  waiting for 5 props when a set places fewer.)
- Open: the `World_GbufferBlend` frame mesh over the poster's picture draws as our generic translucent
  material (a bright frame; its blend program is not ported). Whether 0x546ff0's callback (re-picks every
  compound's model, registered through `FUN_00adb5e0`) fires on a profile change was not resolved. A capture
  of the original at a found and an unfound billboard would confirm the faces.

### 2026-10-14: the post chain against the original's code: bloom sizes, final pass, depth of field, grade

The user: "postfx [is still off]". Compared our chain with the original's, stage by stage, from the full `.text`
(`re/out/live/text.asm`), the dump, the exe's DXBC and live reads: the 20 renderer dumps of `ref/live1/ren` and
snap1, where the renderer is at 0x2404ce00 (found from its depth-of-field block; the static pointer to it is
`.bss` 0x01183168). Details in decomp.md "Final pass, depth of field and grade (2026-10-14)".

- **What the original runs in free roam**: post flags (renderer +0xaa9c) = 0x7f: depth of field, bloom,
  vignette, tint, fisheye (identity values), motion blur, cube scale / offset. The final pass is variant 3,
  `0x01017050` (depth of field + tint); its camera motion blur variants are never chosen while the depth of
  field is on. The composite (`GBufferComposite`) has its own motion blur; it is on, but its vectors are
  0.053 × a one-frame reprojection: ≈ 0.5 px at 51 mph, ≤ 2 px at 200 km/h. No sharpen, film grain, chromatic
  aberration or FXAA program exists among the exe's 74 post programs; distortion / splatter are empty in clear
  free roam; no lens-flare string or program was found (`LensFlareColour` in the keyframes has no reader yet).
- **Bloom levels were 4–6× too small.** The original's are the renderer targets 0x50…0x52: 320 × 176, 80 × 44,
  20 × 11, fixed (the same at 1920 × 1080 in snap1 and at 1280 × 720 in a probe dump this session; ours were
  1/4, 1/8, 1/16 of the screen). Ported as fixed sizes. Reach of the blur's outer taps (±3.29 texels) at 1080p:
  ±20 / ±79 / ±316 px (was ±13 / ±26 / ±53), at 720p ±13 / ±53 / ±211 px; with the day weights the bloom is
  the mean of the two coarse levels, so its glow now spreads over a sixth of the frame. Coastal-road view
  (1280 × 720, noon, linear light added by the bloom): dark centre 0.0117 → 0.0143, frame mean 0.0297 → 0.0286.
  (The first fix this session used 1/6 of the screen rounded to 16, which matched 1080p only; the probe dump
  at 720p showed the sizes are fixed.)
- **Depth of field (new)**: CoC = max(x0 (1 − 90 m / z), 0), x0 = 0.042 (camera block A = 73.448, focus 90, f =
  0.10283 m, live in every read, source not found); a half-resolution CoC-weighted copy is blended in with
  min(15 CoC, 1): nothing within 90 m, 63 % at infinity. The two blur passes reduce to one tap under this CoC
  and are left out (offset untraced). Coastal view, high-frequency energy of the far band (0.3–0.5 h), all
  changes together: 29.2 → 28.1.
- **Brightness / contrast (new)**: `ColourCubeScalesOffsets` = (c · 15/16, b + 0.5 − c/2 + 1/32), c = 1 +
  `e49dc52c` × (1 − t), b = `d1fd3a11` × (1 − t), t = (1 + forward · flat to-sun) / 2 (options at 50). Reproduces
  the live constants to 4 digits (c = 1.0178 / 1.0063 in dumps 0 / 10; this session's probe dump at another
  keyframe, env 0.032 / 0.05, t = 0.8915: c = 1.00543, offset 0.032007, live 1.00543 / 0.032010). Looking away from the sun at noon: c =
  1.0295, b = 0.0148, frame +4 levels, luma std 53.1 → 55.3; into the sun nothing changes.
- **Vignette solved**: the writer is `FUN_008e8c40` (post object +0xc00): k = 1 + 500 (0.5 + 0.5 Sharpness)^16
  (1.82445), s = the contrast c. Both were a measured constant / model decision before.
- **TintBlend slots read**: base identity (0), the two keyframes' cubes with the timeline weights (1 − t, t),
  the Alternate pair at 0, two PostFX-state cubes (heat `…789d`) at 0. Ported: the bake blends the pair
  (before: the earlier keyframe's cube switched at t = 0.5).
- **Dead ends / open**: the writer of the composite blur matrix (+0xb1f0) and of the camera's DOF block; the
  bloom targets' creation code; the DOF blur's tap offset (`FUN_008ee250` for modes 8 / 12); the
  meter's input (the original meters L2, 80 × 44, through `FUN_008e36d0` / `FUN_008e2f80`; ours a 32² grid of
  the frame); the sun lens flare.
- **Probe read (this session)**: `.bss` 0x01183168 points at the renderer (used through the probe's dump.req;
  `ref/shots/fairhaven-post-probe/`). The user's save was rewritten by the game on load (as in earlier runs);
  the copy from before is `ref/savebackup-2026-10-14-post/`.
- New switches: `MW_NO_DOF`, `MW_NO_SUN_CONTRAST`, `MW_NO_CUBE_BLEND`, `MW_BLOOM_DIVISORS=4,8,16`, `MW_POST_LOG`.

### 2026-10-14: the map screen's controls and menu input, from the code
User report: "I cannot navigate around the map (in the pause menu) properly, especially with the controller."
Cause: our map read only `just_pressed` arrows and d-pad and stepped between icons by our own ±60° rule; the left
stick did nothing, and the pause menu had no stick and no auto-repeat. The original's rules are now read and
ported (decomp.md "Menu input and the map screen's controls"):
- **Where the rules are**: the screen data (`mwtool genesys UI/SCREENS2/899991.BNDL --max-items 1000`; the
  default 16-item cap hid 77 of the 93 widget groups) gives each widget's `UserActionInstance`s: the map's
  `MapManager` widget takes `MainMapNav_*` (analogue `menu_*`) as `OnMenuUp` / … of the native `LiveMapManager`
  script; accept → `TO_OPTIONS`, cancel and start → `TO_GAME`, tabs `tab_left` / `right`, filters `trigger_left` /
  `right`. The screen's Lua scripts are compiled Lua 5.1 chunks (`TextFile` resources, 4-byte length + `\x1bLuaQ`,
  4-byte `size_t`): re-encoding `size_t` to 8 bytes lets the host's `luac5.1 -l -l` list them. `MainMapControls`
  only calls `MapManager:OnControlsActivate` / `Deactivate`; `MainMapZoom` holds the zoom states (below).
- **The code** (REA on the decrypted image; none of it is in the dump): `OnMenu*` dispatch `FUN_00803ce0`, the
  per-frame update `FUN_008170d0` (dead zone 0.4, hold 0.1 s before the cursor moves at 1500 m/s, release ≤ 0.4 s
  = flick `FUN_00814f60` within 30° then 45°, else snap `FUN_00813e60` within 0.4 screen heights), the glide
  `FUN_00807a60` with the ease `FUN_00630980` (0.8 s), the cursor = `SelectedPosition` = camera look-at. Its
  tuning is the GeneSys `MainMapConfig` 1135320 (`UI/UICONFIG.BNDL`), found by its field offsets in the type
  table (`mwtool genesys-types`). Input mapping names → action ids (`FUN_007e7910`), the `_alt` (d-pad) twin
  rule (`FUN_0047a3f0`), firing and auto-repeat 0.2 / 0.1 s (`FUN_007f50e0`, `FUN_0047c3xx`).
- **No zoom or pan on PC**: `MainMapZoom.ToggleZoomState` (states distance 8000 / 3400) has no caller in the data
  or the exe (it is the Vita's pinch zoom); the right stick (`scroll_*`) is not used by the map. The map "pans"
  only by moving the cursor, which the camera follows.
- **Back from the map goes to the game**, not to the pause menu: `Back_MapScreen` (cancel) exits `TO_GAME`, which
  ends the in-game menu (HSM 1695290 → 469008). Our map sent Backspace to the pause menu (a `NAV_BACK` guess).
- **Port**: `input::MenuInput` (the 13 menu actions with the original's keyboard / pad defaults, the d-pad twin
  rule, pressed / repeat), used by `pause.rs` and `map_screen.rs`; `map_screen::nav_update` ports the update,
  snap, flick and glide with the data's `MainMapConfig`. New `MW_INPUT_SCRIPT` (keys and a virtual pad sending
  raw gamepad messages, screenshots; `input_script.rs`) and `MW_MAP_LOG`.
- **Checked** (headless, `ref/mapnav/run1.sh`, `run2.sh`; screenshots in the worktree's `ref/mapnav/r1`, `r2`):
  pad stick tap right → the next event right of the player; stick held up 1 s → free cursor (no card), release
  → snapped to the nearest event; d-pad left → the event 64 px left; A → options, d-pad down → START RACE, B →
  back to the map, B → driving. Pause menu: stick held down 0.55 s → three rows (CONTINUE → MULTIPLAYER, as
  the original with the key held as long), d-pad up one row, keyboard Esc / arrows / Enter into MAP; keyboard tap → flick, hold → free
  cursor then snap, Backspace → driving.
- **Checked on the original** (`tools/orig`, keyboard and the virtual pad, which the game did see this time:
  its prompts switched to pad glyphs; save backed up to `ref/savebackup-2026-10-14-mapnav/`, the game rewrote
  `MUD.29.NFS13Save` on load and it was restored; shots in the worktree's `ref/mapnav/orig/`): tap Right on
  the map → the next event to the right (flick); Up held 0.8 s → free cursor without card, the map scrolling
  ≈ 221 px/s (ours 1500 m/s ÷ 6.29 m/px = 238 px/s; the key press latency is a few frames), release → snapped
  to the event under the cursor; pad d-pad left → flick left; pad left stick right 0.6 s → free cursor, then a
  snap; **Backspace on the map → driving** (TO_GAME). Pause menu: wraps (Up on CONTINUE → QUIT); Down held
  0.45 / 0.55 / 1.0 / 1.25 s moved 3 / 3 / 6 / 7 rows = **a repeat every 0.2 s**, not 0.2 then 0.1 as
  `FUN_0047c360` reads; the measurement is used. The original's offline pause list also has QUIT (ours lists 7
  items) and switches its key-box prompts to pad glyphs after pad input (ours always shows keys).
- **Model decisions / open**: the cursor moves along our flat map's screen axes (the camera rows at view +0x2f370
  and the up axis's foreshortening are not read); `FUN_0078f1d0` (a gate before moving) taken as true; the view's
  +0x2f5a8 (0.2 / 0.5 s written on moves / flicks) has no reader found, so the camera follows the cursor without
  lag; the failed left flick's `TO_MWL` (threshold `DAT_0117f578` in .bss) snaps instead; tabs and filters are
  inert; accept on the player opens nothing (the `LocalPlayer` options list is not identified).

### 2026-10-14: lighting against the original: time of day found, sun path, NewSky constants, fences
- **Time of day in memory**: environment manager M (= frame object G + 0x2bc400 = renderer − 0x80) + 0x1b604 =
  time of day in seconds (`FUN_008b3f10`, this = M + 0x18b80, field +0x2a84), followed by 86400.0, a float, the
  rate (+0x1b610, 24 game s per real s in free roam) and the hour as an int. Signature scan + poke:
  `scratch/todscan.py` (git-ignored, in the lighting worktree). The `ref/live1/ren` dumps were at 14:13–14:34,
  not ~12:15 as the old sun model said.
- **Matched captures** of the original: `ref/shots/{fairhaven,fence}-<hour>/` (9.5, 13, 18.5, 21.5, 1 h; time
  frozen by poking rate 0), with `slots.tsv` (all shader constants), `camera.txt` (chase camera; its matrix
  row 2 is the view direction, not "back"), `notes.txt`. Keep the original out of its idle attract mode by
  tapping a key. `mostrusted --camera=… --fov 70` (new flag, horizontal degrees) renders the same view.
- **Sun path** (`FUN_008a8480`, ported in `SunMoonPath`): bearing = BearingCentre + Variation × cos(2π((t −
  14 h + 6 h)/24 h − PhaseOffset)); two elevation curves cos²p·(cos p·h + h + (noon − horizon)) + horizon with
  the day phase p of `FUN_00893620` (08:00–20:00 → π/2…3π/2): the key light (50° at 14:00, 28° at 08/20, 40° at
  02:00; `0bb64ed2`/`c915f96d`/`1dc20d63`) and the sky's sun (35/0/25°; `ac4fe424`/`138500fe`/`ffbdcf8b`) used by
  `KeyLightDirAndXZLength` (NewSky glow) and `SunCoronaDirection` (fog). `a5caac0f`/`644383ee` are the day's
  hours 8 / 20 (not moonrise/set). Matches every live read (unit test). The old model was up to 18° off in bearing.
- **NewSky constants** (live 14:13 vs data, exact): `SunColour` = (SunColour.rgb × SunColour.w, `Sky.2ec8df83`
  exponent 32…1027, was used as 0.65); `BaseCloudColour` = (Clouds.LightColour, `Sky.6acb8a54`²);
  `DomeRanges.w` = `Sky.2ec8df82`; `CloudParamsPS` = (t, 1 − t, 1, 1): the two keyframes' cloud textures cross-
  fade by threshold; `CloudParamsPS2.y` = BaseCloudIntensity (Opacity / Feathering are not these). Gradient B
  (`FUN_008a5a90`/`FUN_008a5560`, 64 texels at i/64) = gradient A with the two lowest stops replaced by the third
  and the first three positions × 0.01 (renderer flag +0xbd80 = 1). Keyframe blends are linear between
  timeline entries and the sky gradient blends stop by stop (positions too). All ported in `sky.rs`.
- **Ambient**: the deferred pass's irradiance cube is GPU-filtered (exe program `0x01004eb8`, normalised
  cos-power average, stored sqrt(x/4)) from a runtime cube; `IrradianceQuadric` (slot 0x18) is built on the
  CPU in `FUN_008c0f30` → `FUN_008b4d90` from the LightRig fills (× |state +0x2474|) as directional lights in L2
  SH (`FUN_008a7bf0`), plus terms not traced. Live 14:13 quadric: up (0.80, 0.94, 1.04), down 0.20, towards the
  sun's azimuth (0.63, 0.50, 0.48), away (0.14, 0.26, 0.32). The rig alone does not fit it (best permutation
  rms 0.09). Model now: cosine convolution of the ported sky + a ground of albedo 0.1 (model value); it gives
  up ≈ 0.30–0.51, i.e. still about half the quadric's. Old rig + sky-average cube: up (1.27, 1.88, 2.63).
  `MW_RIG_AMBIENT=1` = the old cube; `MW_ENV_REPORT=1` logs the bake's constants and irradiance at the rig axes.
- **Shadows**: live `ShadowMap_Constants.w` 6.5 / `Constants2.w` 0.02 at every hour → shadows to 275 m fading out
  by 325 m (ours was 180 / 30; changed). `Constants2.y` (value beyond the fade) = 1 / 0.75 / 0.7 / 0 at 9.5 / 13 /
  18.5 / night: source not found. At 18:30 the original's car park is sunlit where ours is shadowed by the
  buildings behind the camera: the game's cascade fit (not located) may not include those casters. Open.
- **Fences** (`Fence_GreyScale_Doublesided`) drew as opaque black panels: the forward program alpha-tests at
  0.588235 and writes a flat unlit `materialDiffuse`. Ported as mode 13; railings now see-through, bar shadows.
- **Wet stand-in**: without `--planar-reflection` wet roads / plots reflect the sky cube; this turns pavements
  blue and roads white-blotched where the original (always planar) is warm and dry-looking. With
  `--planar-reflection` or `--road-wetness 0` our fence-13 pavement matches the original's warmth. Not
  changed (cost of a default planar pass not measured yet).
- **Numbers** (sRGB region means, `scratch/regions.py`): fairhaven 13 h shaded facade orig (54,78,88) / before
  (105,145,155) / now (72,88,96); road (129,130,122) / (184,188,172) / (118,131,137); night 01:00 pavement Y
  0.021 / 0.201 / 0.021; fence 01:00 most regions 19–30× too bright before, 1–3× now. Trees are 2–4× too dark
  by day in ours (open: tree lighting), night facades show a warm lit area the original lacks (open).
- Not done: the materials' data reflection cubes (`ReflectionTextureSampler`, BC3 cubes; the game reflects
  cube.a × SkyReflectionAverageColour + cube.rgb × SkyReflectionAverageLight, we reflect the HDR sky), the
  secondary cloud layer, the cloud dome coordinates, the shadow-far value, `--weather` pairs (what selects the
  Alternates in the original is unknown), a second, distant place.

### 2026-10-14: a third of the game's functions are missing from the dump

- **Found**: scanning `.rdata` / `.data` of the decrypted image for pointers into `.text` that land after padding
  (int3 / nop / ret) gives 25,319 targets; 16,257 of them are not function starts in the dump's `functions.csv`
  (53,501). They are methods reached only through vtables and tables, which the dump's Ghidra run never turned
  into functions, so their code was never decompiled. The camera director's update `0x005d5e20` is one
  (vtable slot `0x00f99d1c`; the bytes there are `push ecx; push esi; mov esi, ecx` after int3 padding).
- **Tooling**: `tools/snapimage.py` (carve the image from a snapshot, memory layout, optional exports for those
  starts); REA (MCP, Ghidra provider) opens the result. reverse-engineering.md "Functions the dump is missing".
- **First use, the camera director** (`0x005d5e20`, 232 bytes): with it, every function the chase camera runs per
  frame is read, and none queries the world (decomp.md "Avoidance: the original has none"). The chase camera
  has no wall avoidance; our push-in is off (`MW_CAMERA_PUSH_IN=1` for A/B). Open: a capture of the original
  backed into a wall, to rule out a render-side step.
- **The steering's q (vehicle +0x2c70), searched again, not found**: an objdump sweep of the whole `.text`
  (`re/out/live/text.asm`, so the 16k missing functions are included) has eight stores with displacement
  +0x2c70, all in non-vehicle code (game mode, `FUN_00422640`, `FUN_008a7590`), and no 16-byte store near it;
  the field is written through another base or an out-parameter. The constants q would need (35° = 0.6109 rad,
  1/35) occur once each: 1/35 at `0x00fa92f0` (read only in `0x004fbe27`, not vehicle code) and 35° at `.data`
  `0x010cf610`, read only by **`FUN_008fcd60`** (in the dump, undocumented): not q, but an orientation
  controller on a rigid body (inputs at +0x240..+0x24c times `DAT_010cf604..614`, impulses with 37.5, 20, 170,
  80, 40 and 30; possibly air control or a scripted pose; lead for later). q = slip / `Maximum_steering_angle`
  (35° on the XKR) would need no constant, which fits; a second car's capture still tells.
- **Status**: the plain carved image analyses within REA's default 5.5 min; the one with 16,257 exports does not,
  so the deadline is raised (`REA_GHIDRA_STARTUP_TIMEOUT_MS`), with it the image with the exports analyses (2026-10-14). Not yet read: the other
  "not in the dump" / "writer not found" items: `0x0118a060`'s writer (decomp.md, drift), the `LAB_005a22e0` wave
  gain, `AIPlayer::FillInput` 0x006c0890, the constraint +0x210 writer, the `world+0xa2bee` start-over flag.

### 2026-10-10: the Steam Controller (2026) as a pad
The controller (here on its wireless puck, `28DE:1304`) is a mouse, a keyboard and a vendor-defined HID
interface to the OS, so gilrs never lists it; only Steam Input makes an XInput pad of it, for games started
through Steam. `steam_controller.rs` reads the vendor interface on a thread (`hidapi`, pure-Rust backends) and
writes the raw gamepad messages `bevy_gilrs` writes, so `input.rs` and the menus need no change.
- Protocol from SDL's `SDL_hidapi_steam_triton.c` / `steam/controller_structs.h` (layout knowledge only): state
  reports 0x42 / 0x45 / 0x47 (buttons `u32`, triggers and sticks `i16`), puck connect / disconnect reports
  0x79 / 0x46, the lizard-mode-off feature report every 3 s. The puck's interfaces 2..=5 are one slot each; on
  Windows each splits into mouse / keyboard / vendor collections and only the vendor one (usage page 0xFF00) is
  opened.
- Checked on Windows with Steam running: the pad connects as "Steam Controller" next to an Xbox pad, and triggers,
  both sticks and A arrive from it (`--gamepad-debug`, which now prints the pad's entity).
- Open: with Steam running and the game not started through it, Steam's desktop layout still sends keys and mouse
  moves from the controller (README has the ways around it); Linux is not tried (hidraw needs Steam's udev
  rules); USB and Bluetooth product ids 0x1302 / 0x1303 are from SDL's list, not seen here; trackpads, gyro, back
  grips and rumble are not used. The reader steps aside when `SteamVirtualGamepadInfo` is set (assumed to be how
  Steam Input announces its virtual pad, as SDL reads it; not checked with a non-Steam shortcut).

### 2026-10-10: Speed Points found; collectables run (speed cameras, gates, billboards)
Third pass at Speed Points, from the data first. Details in decomp.md "Speed Points"; runtime `freeroam.rs`,
data `mw_assets::scoring`, tool `mwtool scoring`.

- **Found.** `mwtool genesys-types` has `ScoringAction` / `ScoringConfig`; `genesys all --type ScoringAction`
  finds nothing because the 223 actions are structs inside one untyped list object of `GAMEPLAY.BNDL`
  (390077). `ScoringConfig`'s field offsets are exactly the "event ids at +0x18 / +0x1c / +0x260 / +0x288 /
  +0x28c / +0x294 of the config at global +0x44" that the hit handler `FUN_007059b0` sends, so those events
  are scoring actions. The action's array at +0x34 (`8c1feb6d`, count field cracked as `SPCount`) is the SP:
  the titles read "Billboard +{sp}". Code: `FUN_006a8050` → `FUN_006a75f0` → `FUN_006d08a0` (offline: SP =
  `SP[0]`) → `FUN_00735ca0` → `FUN_00734f60` → `FUN_00734400` (total += SP). Values: speed camera 500,
  billboard 500, security gate 250, jack spot car 350, each on the first find.
- **Names cracked** (`genesys-crack` with a scoring word list, 3 words): 23 `ScoringConfig` fields
  (`BillboardDiscovered`, `SpeedCamDiscovered`, `SmashGateDiscovered`, `SecurityGateHit`,
  `SpeedCameraTriggered`, `CarSwapDiscovered`, `Takedown` …), all sorting into their fields' places, and
  `SPCount` / `ScoreCount` / `OnlineFeedbackCount`. The SP array's own name was not found ("SP" does not hash
  to it).
- **Billboards**: a compound instance's word at +0x4c is its trigger's id (216 of 216, 0.0 m apart); and the
  volume alone counts too (`FUN_006e42f0` passes billboard kinds). Gates have no linked prop.
- **Correction**: the "+10 nitrous per smashed gate in free roam" (`FUN_0074cf40`, logged 2026-10-06) is the
  Smash-and-Grab game mode's item scoring; free-roam gates give no nitrous. Not wired.
- **HUD from data**: the SP total (layout 979928) is shown by the HSM "SP Visibility" (node 1695370: 8 s at the
  start and after each change of `ScoringViewModel.CurrentSpeedPoints`); the "+SP" message is sequence 995814;
  the gate's panel "SPEEDWALL / SECURITY GATES n / 152" is sequence 995322 from `SecurityGateHit`'s
  `GameplayTrigger`. The speed camera's plate (sequence 1107082) is a world-space layout (instance scale
  0.005) and is not drawn yet; nor is its PostFX flash.
- **Test** (`--test freeroam`, 911, camera 1025449 / billboard 1496888 / gate 1274961): 37 mph pass not
  counted; 84.7 mph pass counted, first find, +500; 97.7 mph pass again, +0, best raised; billboard +500
  (handled by its volume, then by its prop); gate +250; total 1250; the profile reads back equal. A second run
  on that profile awards nothing (the found gate is dropped at the firing rule, the found billboard only sends
  the 0-point action).
- **Model decisions** (marked in the code): the overlap's narrow phase (box against box by separating axes,
  the car's box centred on its origin; `FUN_00da2e50`'s job was not read); "the prop breaks" = our prop coming
  loose; totals = the triggers of each kind in `TriggerData` (74 / 152 / 216); a `GameplayTrigger` fires its
  sequences whenever its action scores; the total's backing trigger read as "just changed"; `SPEED` prints mph
  or km/h without a unit; no speedwall rows (Autolog friends); the HUD message slides into the total's plate
  as our behaviour-step reading places it (y 22 after its steps, authored at 52).
- **Dead ends / open**: `upload_sp_buckets`, `SP_%d_%d` and `SPEEDPOINTS` are upload / formatter strings
  (no rule behind them); `CurrentSpeedPoints` is the view-model field only. Not traced: the SP of offline
  events (strings `EventAward`, `ProgressionAwardPoints`; the `ScoringAction` position tables are online),
  what triggers the takedown / near-miss style actions, the hints (`FUN_006ef070`, `FUN_006f3f30`), the
  billboard's jump-distance record (`FUN_006ed900`, `FUN_0073cef0`), the `+0x2e74` flag in free roam, whether
  the real collectable totals leave some triggers out. A capture of the original at a billboard, a gate and a
  camera (HUD, 0–8 s) would settle the message placement, the plate and the totals.

### 2026-10-10: jack spots (parked cars, the swap) and body shops

From the dump's pseudo-C, the game data and `.rdata` / `.data` constants of the exe on disk; no live reads (the
original was off-limits). Details in decomp.md "Jack spots: the parked cars and the swap" and "Body shops".

- **Found.** The HSM's "Freedrive States" (469936) start "Do Body Shop" (979770, `DOBODYSHOP`) and "Car Swap
  Logic" (979767 = `HSMBehaviourCarSwapLogic`, script object `JackSpots`). The swap behaviour's functions are
  in the decompiled set: slots `FUN_00747aa0`, idle / prompt / input `FUN_007615f0`, per-slot car
  `FUN_0075d290`, the swap's state machine `FUN_00761d70` with `FUN_0075d590` / `FUN_00758f00`, resets
  `FUN_0075d800` / `FUN_007596b0`. The list at collectables manager +0x41b0 is the `CarSwapSpot` list
  (confirmed by the slot builder reading +0x10 `ColourId`, +0x1c `e36e9d9d`, +0x20 `Placement`, +0x24
  `VehicleId`, +0x28 `0e3bcabe`).
- **Rules with their constants**: location triggers within 300 m, the nearest five get a distance; a car is
  requested from 20 m to 300 m and stays inside 300 m (`.data` 0x010c5ca4 = 90000, 0x010c5cac = 400); swap
  while in the location trigger at ≤ 70 mph (`GameLogicConfig.ff4aef6f`), prompt = language 955092
  (`GameLogicConfig.a1e83efa`) in `Mode.PromptString` (HUD layout 604307), input SWAP_CARS (action 14: the
  per-action state ints sit at 0x1c + 12 × action in the player's input block, checked against LOOK_BACK 37 at
  +0x1d8 and the dialog actions 90 / 91 at +0x454 / +0x460).
- **`CarSwapSpot.e36e9d9d`** is a second parking place: in all 182 spots that have one it lies 2.3–3.8 m
  beside the car trigger (one 6.5 m ahead), facing the same way (two reversed) (`MW_JACKSPOT_LOG=1` prints
  them). The car the player came in is left there, the next one on the car trigger, alternating. `ColourId` is
  a `VehiclePaint` UniqueId = the low half of the paint's resource id, an entry of the car's
  `VehicleColourPalette` (`16671e74` list, or `ColourGroups`' `PrimaryColour`).
- **Body shop**: handler `FUN_00748ff0` = next palette colour (message 0xc → `FUN_0058bc60`, cyclic, not
  random), repair + wash (message 0x16 → `FUN_0058f060`, now a direct link through `FUN_00596880`), nitrous
  +100, stats, two score events, pursuit calls.
- **Runtime**: `jackspots.rs`, `bodyshop.rs`, `--test jackspot`, `--test bodyshop`; finds in the profile
  (`jackspots=`, `jackcars=`, `bodyshops=`, `swapspot=` lines; older files load unchanged).
  Bug found on the way: a drivable car's model and the same model loaded without a skeleton shared one loader
  key, so a parked copy of a car already driven drew skinned meshes without a skin (a wgpu validation error);
  parked cars are their own variant now (`Worker::unskinned`).
- **The body shop's update** `FUN_00749220` (0x00749220–0x00749569) is missing from the pseudo-C (no caller
  of `FUN_00748ff0` there); disassembled from the dump image (`rest/NFS13_dump.exe`, capstone; a call scan
  finds the one caller at 0x007493df). Rule: a pass on entering a kind-6 trigger, each shop at most once per
  5 s (`.rdata` 0x00fd2120), no damage or speed test.
- **Dead ends.** `FUN_00748e50` / `FUN_00748900` next to it belong to another behaviour (a list with
  priorities at +0x2a).
  The `JackSpots` variable getters and the function registering them are not decompiled either (their strings
  have one reference each, from undecompiled code). `mwtool hsm` shows no parameters on either node.
- **Open.** (1) The tip text 1444769 says body shops repair "or" respray; the code does both on every pass. A
  capture of the original driving through damaged would confirm the reading. (2) The swap's transition (manufacturer movie,
  sequences 995872 / 1016424, who sets B+0x74c4). (3) Which cars the profile has (`FUN_0068f190`): Most Wanted
  cars and DLC spots are shown here although the original hides or locks them. (4) The jack spot's map / HUD
  icon (`FUN_006a2480`, the badge ids). (5) Whether input state 2 is "pressed" or "held". (6) The score events
  955227 / 955328 / 955329 belong to the Speed Points work.

### 2026-10-10: Police, pursuits and heat traced; first playable pursuit
- **Where the rules live**: a pursuit manager with a 0x8b0-byte record per player at `*(world + 0x2f410)`
  (`FUN_0066ae60`), fed by the AI cop coordinator (`FUN_00526c30`); all tuning is data: `GameLogicConfig`
  (bust 20 m / 25 mph / 4 s), 40 `HeatLevel` rows (7 for free roam, heat thresholds 0 / 2 / 8 / 16 / 26 / 38 /
  52), `Gameplay.CopType` → `AIPlayerType` → `Rollout` → car, and a heat value on every `ScoringAction`
  (decomp.md "Police and pursuits"; `mwtool police`).
- **Findings that differ from the expectation**: there is no heat "meter" filled by infractions; heat is one
  float raised by scoring actions (chase start 0.5, takedown 1, ram 0.2 …) and by time while chased, and the
  cooldown runs the *heat* back down to 0: escaping means cooling to zero, not waiting out a timer. A pursuit
  starts when a cop *sees* the player above the heat level's speed limit (110 mph at level 0), is rammed, or
  after collisions in its view; speed cameras are not involved. Busting is a 4 s meter that fills near a cop
  below 12.5 mph (25 mph when touching), not "stopped".
- **Method notes**: the script-binding registrations (`FUN_00a4e350("Player", "IsInChase", …)`) map every HUD
  binding to a field or getter and were the way in; getters left as `LAB_` in the pseudo-C are five-instruction
  stubs readable with capstone from the dumped image. `GlobalConfig` (`DAT_010edac0`) +0x24 / +0x44 are
  `GameLogicConfig` / `ScoringConfig`, so `+ 0x288` style constants in the pursuit code are their field offsets.
- **Dead ends**: the cooldown's × 4 factor tests vehicle +0x1a84 == 0, written by several vehicle functions
  (0, 2, 0x80000000) without a readable meaning; `mwtool genesys all --type HeatLevel` finds nothing because
  the rows sit inside an untyped list object (dump `GAMELOGIC/GAMEPLAY.BNDL` instead); `Helicopter` is 0 on all
  rows and no cop spike-strip code was found.
- **Port**: `pursuit.rs`, `police.rs`, HUD heat / BUSTING bars; `--test pursuit` (escape and bust). Cops lose a
  911 from a standing start (the Interceptor is slow and our cops follow the road centre without the game's
  chase path), which is why the test caps the player at 60 mph.

### 2026-10-10 (later): night lighting settled from the decrypted image
With `opencode_extract/rest/NFS13_dump.exe` (capstone scans for struct displacements and call sites; scratch
scripts under `target/night/`) the open items of the entry below resolved (decomp.md "World lights" has the
rules and addresses):
- **Per-hour switch**: day = 8 h < t < 20 h (`SunMoonPath`'s two hours, not `DayNightTransition`'s 8.5 / 20);
  night level = hours to the nearer boundary / `LightTransitionDuration`, clamped (`FUN_008a8480`); a light's
  factor from `OnDuringDay` / `OnDuringNight` (`FUN_0089b400`), staggered per owner for object lights.
  `OnDuringTwilight` is not read by that function. The earlier model ramp (lights off 08:30 to 09:30) was
  wrong: they go off 07:00 to 08:00.
- **Slot 0x11** = (night level, …) (`FUN_008c2080`, through `FUN_00a621a0`): the modelled night level for lit
  windows was right in kind and now uses the traced value.
- **`LightMapTextureSampler`** is bound by the **Model** (+0x10 override block, missed by the parser; also by
  instance overrides): 2,557 world models name one, 117 distinct textures, the default `…0957a7` a 16×16 black
  one, the others baked warm lamp light on facades (128², BC1 / RGBA8). The same block binds
  `AOMapTextureSampler` (2,441 models, 422 textures), so many models had no AO map in the port until now.
- **`FUN_008df590`** = sort by distance, keep the nearest (sizes not read); **`FUN_00ede840`** = ceil (the cone
  edge was rounded down before; now up).
- **Specular power** is 2^(11.0001 − 11 roughness): confirmed in the listing (`exp` after the `mad`, and the
  power itself applied as `exp(log(N·H) × power)`, both base 2 in DXBC). Fixed for the key light and the lights.
- **Flash patterns** decoded (`FUN_00547470`); the port draws those lights steady.
- **Roads**: plain `DriveableSurface` adds albedo × COLOR.rgb² × 0.318 × night level (ported: the baked colour
  rides in UV_1.xy + COLOR.z); the `_Lightmap` road variants add LightMap(uv1).rgb × `lightMapIntensity` ×
  albedo with no night factor (ported).
- Still open: the light lists' sizes; what the second hour pair (8.5 / 20) drives; `PlotPBR` lightmaps (same
  kind of term, `lightMapIntensity`, sampler 13; not ported); line lights; coronas; the "SetLightsOn" trigger.

### 2026-10-10: night lighting: world lights, street lamps, lit windows
Task: the original at dusk downtown has lit windows and lamps casting pools of light; ours had neither.
- **Light definitions cracked** (`genesys-crack`, a lighting word list; every name sorts into its field's
  place): `AttenuationThreshold` (13b601dc), `OnDuringDay` / `OnDuringNight` / `OnDuringRain` /
  `OnDuringTwilight` / `OnDuringWet` (the five bools, in that order), `ConeInnerAngle` (bf16f152);
  `EnvironmentSettings.DayNightTransition`: `LightTransitionDuration` 1, `LightTransitionOffset` 0,
  `ShadowTransitionDuration` 1, `ShadowTransitionOffset` 0, and two hours 8.5 / 20 (`1ab46d30` / `0e2be130`,
  not cracked; they sort between the Light… and Shadow… names, like `Night…End` / `Night…Start`).
- **The light model** (decomp.md "World lights"): colour = `Colour` × `Intensity`, falling off as
  saturate(1 / d² − threshold / max channel), so a light ends at sqrt(max / threshold) metres (a street lamp:
  62 × (1, 0.85, 0.38), threshold 0.3: 14.4 m); spot cone = saturate((cos − cos edge) / (cos inner − cos edge))
  with the edge quantised to 2°; the key light's BRDF, no shadows.
- **Counts** (`mwtool lights`): 3,628 in `LightInstanceList`s (2,454 spot, 1,108 point, 66 line), 4,921
  carried by compound / world objects through `WCVfxBehaviour.Lights` (3,628 spot, 1,293 point). 602 have a
  flash pattern. 2,368 are also `OnDuringDay`.
- **Street lamps are object lights**: a compound object's `mLocators` (+0x10: count, ids, `{n, ptr, 0}`; quat +
  position per locator, the car locator layout) holds a group per lamp head (double lamp `…621c484f`: two
  locators at (±2.94, 8.76, 0), rotated so +z points down), and its "SetLightsOn" `WCVfxBehaviour` names the
  light definition and that group (plus two `Corona_definition`s on the same group).
- **Lit windows**: emissive × albedo into RT3. Building variants: spec.w² × AO map.w × `LitWindowColour` ×
  `AdditiveIntensities.y`; LOD2: diffuse.w² instead of the two masks; Illuminance / Emissive signs:
  Illuminance(uv0).y × `LightmapColour` × `AdditiveIntensities.x`; all × `GlobalLightMapIntensities.x` except
  the `Persistent` variants. `InstanceIntensity` variants read `AdditiveIntensities` from the instance block.
- **Implemented**: `mw_assets::lights`, `world_lights.rs`, the light-volume programs in `game_lighting.wgsl`
  (instanced cubes after the key light pass, additive, fogged by the fog's transmittance), mode 5 of
  `layered.wgsl`. Bench (downtown spawn, 01:30, 30 units, 2,171 lights on): 7.25 → 7.89 ms mean, on a machine
  shared with three other builds.
- **Model decisions**: the day / night ramp (linear over `LightTransitionDuration` from each boundary hour;
  `OnDuringTwilight` holds a light on through it); `GlobalLightMapIntensities.x` = that night level (0…1);
  object lights are on whenever their definition's flags say so (the "SetLightsOn" trigger is not traced, and
  a knocked-over lamp keeps its light); lights with a flash pattern stay off; no cap on the number of lights;
  the cone table index rounds down (`FUN_00ede840` taken as floor).
- **Dead ends / open**: the writer of a light record's intensity and of constant slot 0x11 (decomp.md);
  `LightMapTextureSampler` is still unbound: the `_Lightmap` programs sample it at TEXCOORD1 and scale it by
  `AdditiveIntensities.x` (roads: `lightMapIntensity`), but no material, instance override or `ZoneHeader`
  slot names a texture for it, and the dump search for its binder was not done (a frame capture of the
  original would show what is bound in slot 9). Plain `DriveableSurface` adds albedo × COLOR.rgb² × 0.318 ×
  `GlobalLightMapIntensities.x` (baked vertex light on roads): decoded, not drawn (the road path repacks the
  vertex colour). Line lights (66), coronas (`Vfx_Corona`, the lamps' `Corona_definition`s) and car lights
  are not drawn. Seen but not changed: the key light program's specular power is `exp(11.0001 − 11 rough)` in
  DXBC, where `exp` is 2^x; `game_lighting.wgsl` uses e^x for it (both the key light and the new lights).
  At `--hour 21` the downtown view is washed out by bloom / exposure, before and after this work.

### 2026-10-10: Windows host; first live read of the original from Windows (chase-camera `.bss`)
The workspace builds and runs on Windows (MSVC): `mwtool memscan` / `telemetry` / `tune` / `capture` reach the
game through `ReadProcessMemory` (`memscan/windows.rs`), the install is found through the EA app's registry key
(`C:\EA\...` here: the EA app build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles, `mwtool verify` 0 failures) or
Steam's. The 911's accel test gives the Linux numbers (0-100 km/h 3.73 s). `tools/orig` is not ported: there is
no input / screenshot harness for the original on Windows yet, so gameplay captures need a person driving.

Live read with the game started from the EA app and left at its start-up screens (`NFS13.exe` at 0x00400000;
no elevation needed). Values already known read the same as on Linux / Proton: 0x01186b74 = 9.81,
0x01186bf0 = 0x01186b10 = 0.1, 0x01186630 = 1.8, 0x01186f60 = 0.5, 0x0118a018 = 0.44704, 0x0118a060 =
0x0118a090 = 2.23694, 0x0110f590 = 3600, `_DAT_0110e8f0` = 45. **New** (the chase camera's `.bss` constants, decomp.md "Cameras"
listed them as not read):

| Address | Value | Where it is used (decomp.md) |
|---|---|---|
| `DAT_0110e770` | 22.352 (= 50 mph in m/s) | normaliser and clamp of the longitudinal acceleration `a_long` |
| `DAT_0110e4d0` | 0.75 | the step clamp of `target - cam[+0x588]` |
| `DAT_0110e680` | 4.0 | the offset clamp |
| `DAT_0110e610` | 0.01 | squared-length threshold of the local vector |
| `DAT_0110e640` | 5.0 | speed threshold `S` of the 1.85 term |

Each is the first lane of a vector of four equal values. Applied the same day in `camera.rs`: the
acceleration normaliser (was a model value of 20 mph/s), the 0.75 m step clamp and the 4 m offset clamp. Also read: 0x0110f940 holds
a pointer (0x1a199970), 0x0110f944 = 0 and 0x010eac98 = 0x001ae534 at this state (objects; their fields need
gameplay).

**Telemetry works too**: a `handling.watch` rebuilt from the offsets in `mwtool tune` and decomp.md "Update
loop" (vehicle = `[[[[NFS13.exe+0xcedb04]+4]+0x53220]+0x184]`, index 48; git-ignored
`ref/win-live1/handling.watch`) sampled the idling free-roam car: 1,010-1,020 rpm, gear 1, wheel loads 3,950 /
3,959 / 3,805 / 3,796 N, grip 1. It has the car, drift, input (+0x1bf0 / +0x1bf4 / +0x1c00) and wheel-load
columns; the body pose / velocity, wheel contact, spin and slip columns that `--replay` and `mwtool compare`
need were then found live: the rigid body is `[vehicle + 0x1760]` with right / up / forward at +0x00 / +0x10 /
+0x20, position at +0x30, velocity at +0xb0 and angular velocity at +0xc0 (1.2-1.5 rad/s about y in a left
turn at 45 mph, 0 at rest). Wheel contact, spin and slip columns are still missing.

**Driving the original from Windows** (`tools/orig/orig.ps1`: focus, scancode keys through `SendInput`, a
screenshot of the game's client area; keys are only sent while the game is the foreground window). The
keyboard drives without changing the controller setting: A / Left / Z show up as `in_throttle` 1, `in_steer`
-1 (left is negative), `in_brake` 1. First recorded run (`ref/win-live1/drive1.csv`, the Aston V12 Vantage on
grass then brick downtown, so not a clean reference): 0 to 47 mph in 4 s through 1st and 2nd, then a wall.

**Matched screenshots**: the chase camera's matrix (camera +0x10) and FOV (+0x90) read at the moment of a
screenshot give the same view in ours (`--camera=<position>,<position + 20 x third row>`). The third row
points **forward** (from the eye to the car), not back as the 2026-10-06 entry says. First pair, downtown
(camera 2221.45, 21.43, 2611.26; FOV 70; the original's hour is not known, an overcast dusk or dawn): in ours
the road beyond the lawn is a bright tan band (the wet road's sky reflection at a grazing angle; the
original's is dark grey), the brick paving is bright and dry-looking (the original's is dark and glossy), no
lit windows or street lamps. Open: where the original keeps its time of day, so pairs can be made at the same
hour.

### 2026-10-10: wheel placement: the game builds its wheels from VehicleInfo's axles, not the spec

- **Report**: "wheel placement of some cars is still off". `mwtool wheels all [--png <dir>]` (new) prints every
  car's spec wheel position next to where the game puts it, and draws side views (spec position as a blue ring).
  Over all 104 car bundles (73 named, cop variants, 11 traffic `_TR`): no spec wheel has a rotation or a scale
  other than 1, and the physics `RigidBody` wheel transforms equal the spec everywhere. But the spec's wheel y
  is a placeholder on many cars: 0.311 front / 0.316 rear on the Challenger, Crown Victoria, Impreza Cosworth,
  Caterham, M3 GTR (whatever the tyre radius, 0.27–0.37 m), 0.325 on every traffic car (tyres 0.31–0.44 m: the
  box truck's sank 11 cm into the road), and the Venom's front axle is at z 0.125 (the old arch heuristic).
- **The rule (dump, sub-agent trace, confidence high)**: the vehicle build `FUN_009b40c0` (from `FUN_009b4e60`)
  loops over `VehicleInfo.Axles` (+0x98, 0x28 each; `VehicleInfo` = the physical definition's `AdditionalInfo`)
  and each axle's `SuspensionWheel` handles (`1048818a`) and **overwrites** the wheel body's position with
  x = `SuspensionWheel.dcbe38af`, y = `VehicleInfo.caac331c` + R − `Axle.2f5fb582`, z = `Axle.eaee225f`, R = the
  wheel body's cylinder `Radius` (the body named by `SuspensionWheel.Child`). The spring is then built at that
  geometry (`FUN_009b29b0`: rest length = distance + W / k, so the wheel rests there). Drawing: the spawn
  matrix `FUN_00593ac0` takes the spec position and scale once; each frame `FUN_005798a0` replaces it with the
  physics wheel's world matrix (vehicle +0x870 + i × 0x1d0, rows +0x60.., translation +0x90, built by
  `FUN_00902930` from the wheel record's centre, no offsets) times the inverse of the car's render matrix
  (vehicle +0x160 local offset × chassis). The spec quaternion is never read and the spec scale only reaches
  the spawn matrix. Kinematic traffic (`FUN_005ee7f0`, from `FUN_005f7cf0` / `FUN_005fcbd0`): the same axles,
  every wheel at the front-left radius, R·((x, R_FL, z) − T) with `VehicleInfo.509931cd` (rows R, translation T).
- **Live check (probe, 2026-10-10)**: `mw-probe` now serves memory dump requests (`dump.req`, docs/probe.md).
  The player's F-150 Raptor in free roam, parked and again after driving and stopping: drawn wheel centres
  (wheel matrix × inverse render matrix) (±0.9275, 0.353 / 0.356, 1.675) and (±0.933, 0.324 / 0.325, −1.725);
  the rule gives (±0.925, 0.354, 1.675) / (±0.93, 0.324, −1.725), the spec (±0.935, 0.436, 1.68 / −1.72). The
  wheel object's +0xc0 (the built rest position in the chassis frame) is the rule's minus the chassis body
  origin (0, 0.75, 0.203). Screenshots of the Raptor from the side (camera rotated with Delete / Page Down)
  show the original's lower wheels in the arches. The XKR snapshot `snap1` (2026-10-06) has the rule at +0xc0
  as well (its live wheels were 3–5 cm higher: that car was not at rest).
- **The ground**: at rest the tyre's bottom is at y = `caac331c` − `2f5fb582` ≈ −T.y in model space (Raptor −0.08 /
  −0.11, T.y 0.097; Challenger +0.028, T.y −0.033). So the player's body height comes out of the physics as
  before; for traffic we lift the model by T.y above the lane (inferred from the same relation; the traffic car
  frame's writer was not traced).
- **Mirroring settled**: the game turns right wheels half a turn about Y (`FUN_00593ac0`, `FUN_00902930` negate
  rows 0 and 2) and draws them with the wheel shaders' flipped technique, whose vertex programs scale the
  position by (10, 10, −10) instead of (10, 10, 10) with front-face culling (`mwtool shader-asm
  VehicleNFS13_Wheel_Textured_Roughness`; `g_flipUvsOnFlippedTechnique`): together a mirror on X, as ours.
- **Port**: `mw_assets::wheel_layout` (the rule; `place_spec_wheels` moves spec wheels by side and axle).
  `VehicleHandling` wheel bodies take the rule's positions (physics and drawing, `loader.rs`), the static viewer
  (`vehicle_placements`) too, traffic gets the kinematic pose and `ride_height` (`traffic.rs`). The arch
  heuristic `mw_assets::wheel_arches` (2026-10-04, a model decision for the Venom) is removed: the Venom's
  VehicleInfo puts its front axle at z 1.3.
- **Per car (rule − spec)**: Venom front z +1.175; Focus ST y +0.16 (wheels hung below the body); Raptor y −0.08 /
  −0.11; M3 GTR (and its 4 variants) z ±0.10, y +0.03; box truck 392322 y +0.10, traffic 122821 −0.08,
  122822 +0.07, 122824 (pickup) +0.06, 122819 (van) −0.055; 911 GT2 x −0.09, y −0.02..−0.05; A1 clubsport y −0.05..−0.09;
  Challenger y +0.08, z ±0.02; Evoque y −0.06..−0.07; Diablo, 350Z, Maserati, Marussia, Charger R/T, Firebird
  4–6 cm; most others under 3 cm (911 Carrera S 5–8 mm, Veyrons ~1 cm). Images: `extracted/vis/*_{before,after}.png`
  (worktree, git-ignored).
- **Handling tests unchanged** (before = master build): 911 accel 3.58 s 0–60 / 307.6 → 307.4 km/h top, brake
  38.1 m, corner 2.57 g; Focus ST and Raptor identical but for 0.02–0.08 s at 0–200 km/h (the grip load is the
  static corner load, not the spring's).
- **Open**: the traffic frame's height (T.y lift) is inferred; what wheel record +0x3c and traffic car +0x1fc
  are; the names of `caac331c` (a body reference height) and `2f5fb582` (an axle's ground offset); a per-axle
  scale on the strut-top-to-wheel distance in `FUN_009b40c0` (1 by default, `FUN_008f9120`; likely an upgrade
  hook) is not applied.

### 2026-10-10: model LODs and draw distances (the game's rule, `lod.rs`)
Performance item "fewer draws" (handoff of 2026-10-09): every world placement was drawn at LOD0 at any distance.
- **The rule, traced** (decomp.md "Model LOD and draw distance"): `FUN_008a02f0` draws one placement:
  `d = max(0, |M·c − eye| − r)` from the LOD0 renderable's bounding sphere, `v = d² / (s² · S²)`, nothing past
  the model's last LOD distance (`Model` +0x08, squared, count +0x18), else the first LOD with `v ≤ lod[i]`
  (at most four); the shadow pass also stops past `Model` +0x0C and draws the SHADOW state when there is one.
  `s` is per placement: `InstanceList` +0x04 (was "unknown"), the prop list's +0x4C, compounds +0x50,
  dynamics +0x4C (the float after the instance id; `FUN_00550020` passes them). `S` is the view's
  (`FUN_008b5fe0`): `max(L · 0.75 / tan(fov_h / 2), 1)`, L from the "LodScaling" option (0 → 1, 1 → 1.5,
  2 → pixels / 1280×720 capped at 1.5; `.rdata` 0.5, 0.75, 1.0, 1.5, 1/720, 1/1280 read from the exe); this
  install's `config.NFS13Save` has `LodScaling=1` (also the game's default on a fast machine, `FUN_00414b40`).
  Cars have a separate rule (`FUN_008aa970`, 15 / 30 / 60 / 150 m × S), not ported.
- **Data** (new `mwtool lods [--at x,y,z] [--scale S]`): 3668 models placed in the world; LOD counts 1: 820,
  2: 224, 3: 2623, 4: 1; LOD1 / LOD2 have their own renderables in 2848 / 2624 models; 1835 have a SHADOW
  renderable. Common tables 100/200/400, 200, 80/150/200, 200/400, 80/150/2000 m. Placement scales: 1.0 for
  31 774 instances, 12 201 props, 4257 compounds, 4364 dynamics; the rest 0.01–10 (instances 0.5–8 mostly).
  From (815, 60, 1760) at S 1.5 the game draws 159 of the 12 290 props and 3437 of 33 524 non-backdrop
  instances (all units loaded, before frustum culling).
- **Ported** (`lod.rs`; `mw_formats::Model::lod_for_distance`, `Instance::lod_scale`, `ObjectInstance::lod_scale`,
  `Placement::lod_scale`): the loader decodes each placement's LOD1..3 renderables as their own model variants and
  sends the LOD table; every placement's mesh children carry the LODs they belong to; `choose_lods` (PostUpdate,
  before visibility propagation) applies the rule each frame from the main camera with S from its horizontal
  FOV, changing visibility only when the choice changes (≈0.4 ms a frame for ≈11 400 placements, `MW_LOD_LOG=1`).
  Shadows: casters past the shadow distance are dropped in `shadow_cull` (`LodShadow`). A/B: `MW_NO_LOD=1`;
  `MW_LOD_SCALING=0|1|2`.
- **Model decisions**: the SHADOW renderable is not drawn (the visible LOD casts); the shadow decision uses the
  main camera's position; breakable compounds keep their LOD0 pieces at every LOD (cull only); backdrops keep the
  zone composition; `--merge-static` merges LOD0 without the rule; L = 1.
- **Bench** (same binary, `MW_NO_LOD=1` → default, interleaved; machine shared with other agents, load 5–13):
  `--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --mute --bench 10`, two rounds: frame mean
  15.71 / 16.73 → 10.57 / 8.98 ms, p95 23.1 / 22.5 → 17.1 / 11.7 ms, p99 28.2 / 26.8 → 24.1 / 16.1 ms, render
  graph 11.0 / 8.2 → 7.1 / 5.9 ms, visible mesh entities 4789 → 1688 (props 1207 → 124, world 2682 → 958, alpha
  590 → 295), distinct visible materials 507 → 372, outer shadow cascade 404 → 356 casters. `--spawn=800,42,1688,62
  --auto-throttle 0.4 --bench 20`: visible 7289 → 2338 (props 2509 → 348), outer cascade 881 → 740, render graph
  9.9 / 11.7 → 8.3 / 9.4 ms, frame mean 16.8 / 18.3 → 16.8 / 14.8 ms (load 8–13: noisy).
  **Costs**: mesh entities 25 800 → 54 400 (the LOD meshes, hidden until chosen) and meshes 8.5k → 15.7k; the
  main-world schedule did not drop (5.9–9.4 ms either way, noisy), RSS too noisy to compare (1.25–2.05 GB).
- **Pictures**: `--camera=-1131,27,1627,-1190,15,1720 --units 20`: MAD 0.58/255, 1.8 % of pixels > 8/255, all in
  distant trees and shrubs (their LOD1/2 now), nothing missing near the camera; downtown drive start
  (815, 60, 1760): MAD 0.86, the difference is foliage shimmer and exhaust vapour only.
- **Open**: the SHADOW state's renderables (a shadow-only entity per placement; Bevy has no per-view mesh swap);
  swapping mesh handles on one set of entities instead of an entity per LOD (needs the AABB updated with the
  mesh); car LODs (`FUN_008aa970`) for traffic / AI; the `Model` +0x19 pass mask; the shadow views' eye position.

### 2026-10-10: frame time: the 2/60 s cap, the sky filter, pre-warm, single-threaded main world, CPU clustering
Follow-up of the 2026-10-09 list ("Next, by the numbers"); model LOD / draw distances / placement culling were
another agent's and are not touched here. Method: every A/B interleaved (same command back to back, 2–3 rounds),
load average given per number. The machine was shared (load 3–28); for "under load" numbers 16 niced busy
loops (`burn.py`, nice 19) ran beside the game. Drive = `--drive 911 --units 30 --spawn=815,60,1760,270
--auto-throttle 0.3 --mute --bench 10`, race = `--race 462174 --units 30 --mute --bench 20`. One chrome trace
(`--features bevy/trace_chrome`, race, 4.2 GB) was aggregated with a streaming script (span totals per name,
per parent, per window).
- **Reading the numbers**: the bench's frame mean is bimodal on this desktop: Wayland paces some runs at the
  display (16.67 ms) and leaves others unthrottled (~7.5–8.5 ms) in the same configuration, so means are only
  comparable within a mode; the per-segment times (main world, render prepare / graph, ms per fixed step) are
  what the A/Bs compare. In the frame log, frames with `r_spec` ≈ 10.5 ms are the swapchain acquire waiting for
  the display (`prepare_windows` 10.4 ms in the trace), not CPU work.
- **1. The 2/60 s cap (committed)**: `Time<Virtual>` max delta 250 ms → 2/60 s (decomp.md "Update loop"); Bevy
  applies it to real time before the relative speed, so `--test` (4x) gets 8 steps. At most two fixed steps per
  frame at normal speed: race start (load 10–15) 9 / 7 / 5 steps after the slow frames before, 2 after; drive
  under load up to 4 → 2. Steady-state frame times unchanged (60 fps the cap never acts). `--test
  accel|brake|corner|drift` lines identical. Below 30 fps the game now runs slower than real time, as the
  original does. `MW_UNCAPPED_DELTA=1` = before.
- **2. The sky's environment light (committed)**: Bevy's `GeneratedEnvironmentMapLight` re-ran its whole filter
  every frame (copy, SPD downsample, GGX radiance for each of the 512² cube's 10 mips with 32–512 samples, 1024-
  sample irradiance: ~75 M filtered samples a frame, plus ~15 bind groups and uniform buffers created per frame).
  The camera now keeps it until the filter has dispatched 4 times on the current sky (counted in the render
  world after `filtering_system`), then drops it; the `EnvironmentMapLight` it made keeps the filtered cubes. An
  `AssetEvent` on the sky image (a bake) puts it back. Pictures: road 13 h new vs old 0.14 MAD (two runs of the
  old mode differ by 0.53: foliage sway), wet night + planar reflection 0.20, a run-time rebake 13 h → 19 h 0.06
  against per-frame filtering. GPU utilisation at a vsync-paced 60 fps (road view, 8 s sampled after loading,
  load 13–18): 64 % vs 73 % (≈ 1.5 ms of GPU a frame); whole drive set below 56 → 43 %. Its passes never showed
  in `MW_BENCH_PASSES` (they are recorded outside the diagnostics Bevy prints). `MW_SKY_FILTER_EVERY_FRAME=1` =
  before.
- **Found on the way (not fixed)**: after a run-time rebake (13 h → 19 h on a fixed camera) the road looks
  different from a fresh start at 19 h (MAD 13: bright, yellow road; same with per-frame sky filtering, so
  older than this change): something the road samples is not refreshed by `store_images` (reflection cube or
  wetness inputs in a material bind group?). And `--vehicle 911 --screenshot` (car viewer) crashes at exit on
  master too ("Failed to send screenshot: sending on a closed channel", then SIGSEGV / abort), no file written.
- **3. Pre-warm (committed, `prewarm.rs`)**: Bevy specialises a mesh's pipeline per view (main, deferred prepass,
  each cascade) the first time it is visible there and keeps it for the entity's life
  (`SpecializedMaterialPipelineCache`, `specialize_shadows`: entries go only on a mesh / material change), so a
  camera cut pays for every entity that view has not seen. The race-start cut (trace): `specialize_shadows`
  21.9 ms + `queue_shadows` 11.3 ms; frame log (load 3–5, three rounds): specialise 13–21 ms + queue 8–14 ms, a
  34–44 ms frame (190 ms under heavy load on 2026-10-09). Pre-warm: when loading is done, and only where the
  frame is hidden (a race: the intro now waits for it; a front-end title / transition / loading screen), the 3D
  cameras get an all-pass frustum (half-spaces (0, 0, 0, 1)) and every visible caster goes into every cascade
  for 3 frames. Cost: one 104–117 ms frame at the loading view (specialise ~50, queue ~28, graph ~27 ms), 133–153
  ms in all. After it the race-start cut specialises ~0.5 ms (queue 7–11 ms stays: binning newly visible
  entities is per visibility change) and no frame after loading exceeds 30 ms (before: the 34–44 ms cut). Plain
  free roam (`--drive`) had no start hitch (no frame > 34 ms after the first second, two rounds) and is left
  alone: there `Loaded::done` can arrive while driving. Race test (`--test race`) unchanged. Screenshots equal
  (road 0.16, wet 0.20 MAD). Model decision: the 3-frame count. `MW_NO_PREWARM=1` = before.
- **4. Physics step cost**: the trace (race, ~60 steps/s): `PhysicsSchedule` 2.85 ms a step, `SubstepSchedule`
  6 × 0.29 ms. Avian's schedules are already single-threaded executors; the cost is its queries' `par_iter`
  (`integrate_positions` / `_velocities`, `update_solver_body_angular_inertia`, `prepare_solver_bodies`,
  `writeback_solver_bodies`, the AABB updates, `propagate_collider_transforms`): each call fans ~9 tasks of
  1–2 µs out to the compute pool and waits, ~65 µs wall each, and on a busy machine much more: the main thread
  sat idle 2.96 ms in one `integrate_velocities` while a worker was busy elsewhere, and inside other scopes it
  ran render-world tasks (a 4.4 ms shadow pass inside `clear_velocity_increments`). Our own fixed systems are
  small (`car_physics` 0.18 ms, the rest < 0.01 ms).
  - **Tried, not committed: serial avian.** A copy of avian3d 0.7.0 with its 10 `par_iter` calls made `iter`
    (git-ignored `[patch.crates-io]`, removed again): handling tests identical; quiet (load 5) 0.96–1.08 →
    0.91–0.93 ms a step; with 16 busy loops 5.4–5.7 → 4.4–4.7 ms a step, frame 20.4 → 19.7 ms. Worth it only
    with a maintained fork / vendored avian (a project decision: ~52 k lines).
  - **Tried, no effect: fewer compute threads** (`TaskPoolOptions.compute.max_threads` 4 instead of 8): equal
    within noise under load.
- **5. Single-threaded main world (committed)**: all main-world schedules (First … Last, the fixed ones) on
  `SingleThreadedExecutor`; the render world and queries' own `par_iter` stay parallel. Idle machine: equal
  (drive main world 3.7–4.3 ms either way); race at load 8–22: main world 4.02 / 3.55 → 3.32 / 3.30 ms, step
  1.19 → 1.03 ms. With 16 busy loops (three rounds): main world 16.7 → 12.8 ms, step 5.1 → 3.6 ms, frame mean
  19.3 → 17.4 ms, p95 26 → 22.6 ms. Handling and race tests identical. `MW_MAIN_MULTI=1` = before.
- **6. CPU light clustering (committed)**: no point / spot / probe lights exist, but GPU clustering rebuilt
  per-view buffers and bind groups and encoded its passes every frame (`prepare_clusters_for_gpu_clustering`
  0.59 ms + `cluster_on_gpu` 0.25 ms traced). `GlobalClusterSettings.gpu_clustering = None` at startup (Bevy's
  CPU fallback; the render world copies the setting each frame). `ClusterConfig::None` on the cameras instead
  is a wgpu validation error with GPU clustering (zero-sized buffers). Drive, load 3–5: render prepare 1.93 →
  1.66 ms, graph 5.05 → 4.77 ms; road 0.04 / wet 0.01 MAD. `MW_GPU_CLUSTERING=1` = before.
- **All together** (drive, master-equivalent switches vs new, three interleaved rounds): load 3–4: main world
  4.04 → 3.62 ms, render prepare 2.05 → 1.69 ms, ms per step 1.02 → 0.80, GPU 56 → 43 % at 60 fps; with 16 busy
  loops (load 5–22): frame mean 20.3 → 17.0 ms (49 → 59 fps), p95 26.7 → 22.2 ms, main world 17.3 → 12.3 ms,
  render prepare 6.7 → 5.4 ms, step 5.3 → 3.5 ms.
- **What is left, by cost** (race trace, ~53 fps, ms per frame with tracing overhead): render-world draw encoding
  `render_system` 9.7 (deferred prepass 2.6, shadow cascades 1.6, submit 1.0: draw counts, the LOD / culling
  work; bindless materials would merge draws), per-mesh-entity render bookkeeping ~1.5 (`write_batched_
  instance_buffers` 0.6, `write_indirect_parameters_buffers` 0.5, `collect_meshes_for_gpu_building` 0.4,
  `extract_meshes_for_gpu_building` 0.25: scale with mesh entities), physics ~2.9 a step (above), transform
  propagation 0.35 × 2 (PostUpdate and the fixed loop), visibility 0.33 + 0.27 (cascades), `prepare_skins` 0.24,
  the four small material types' `check_entities_needing_specialization` ~0.1 each (Chevron, tyre marks,
  particle quads, sparks: per-type `AssetChanged` scans; merging them into one material type would remove three),
  our tonemap / bloom node 0.27 (12 bind groups created per frame; cacheable), `cull_casters` 0.13.

### 2026-10-10: the wheels are the game's point masses (spring, damper and contact passes ported)
Handoff item 3 ("Kerbs part 2"). Read from the disassembly of the snapshot's decrypted code (`re/out/live/snap1`),
constants from NFS13.exe on disk (`exe-const`), checked against the snapshot's objects; details in decomp.md
"Suspension, wheels, aerodynamics".
- **Spring `FUN_0092f510`** (read in full): along anchor (element +0x20) → wheel centre, x = length − rest length
  (+0x10), λ = −k x h through the chassis's effective mass at the anchor plus the wheel's 1/m, on both; while
  extended the result may close at most |x| / dt (the ±1 is `.rdata` 0x01024570 = −1). **Damper `FUN_0092f160`**:
  λ = −coef × v_rel × 0.5 (0x010245b0) × h, rebound slot (+0x18) while extending, cut at v_rel = 0 where it would
  change sign; the wheel's share is masked by wheel record +0x0c = 1.0, which is the w of the wheel centre (a point;
  1.0 in all 124 records of snap1): **the damper acts on the chassis alone**, and its × 0.5 halves the damping the
  data gives. Same element list: all dampers, then all springs, then the bars (`FUN_00932f90`).
- **Wheel contact**: `FUN_009c5a90` raises the wheel's velocity by its own predicted penetration / dt (the record
  is activated by `FUN_009c52d0` only when d ≥ 0; the strut stop runs only then), `FUN_009c2670` stops the wheel
  approaching the ground (the bias term is × constraint +0x0c = 0 in snap1). Positions are integrated by
  `FUN_009324f0` **between** the contact passes and the sub-steps (bodies and wheel centres; `FUN_009328a0`,
  taken for it before, is something else). Wheel gravity (`FUN_009282d0`) is +9.81 h² **up** per sub-step: the
  wheels fall only through their springs.
- **Checks in snap1**: the XKR's four spring elements (at 0x07bc7880 + i × 0x80) hold k 41603 / 41784 / 47896 /
  47663, c 3524 / 3540 / 4013 / 3993, rebound 6344 / 6371 / 6420 / 6389: ours from the data k 41671 / 47750, c 3530
  / 4000, rebound 6354 / 6401 (0.3 %). The wheel records' velocities along their contact normals, −2.279 / −2.307 /
  −1.791 / −1.813 m/s, equal one sub-step's spring kick k |x| h / m with the compressions read from the same
  snapshot (2.283 / 2.301 / 1.792 / 1.820) only with h = 1/120: the sub-step dt is dt / 2. The step's context is
  at 0x07b72b80 (the earlier note's 0x07b72a80 is 0x100 off): +0x08 wheel records, +0x20 constraints, +0x1b0 /
  +0x1b4 = 2 / 2, +0x1bc = 1/60.
- **Ported** (`mw_handling::suspension`, 4 unit tests; drive.rs): each wheel a point mass of the data's wheel mass
  moving along the car's up axis; per tick two sub-steps (contact impulse, strut-stop share, dampers, springs,
  anti-roll bars, wheel gravity), the tyres (acting where the contact carried a normal impulse), then the two
  contact passes (wheel + strut stop) and the wheels' integration, which open the game's next tick; the 4 ×
  corner-weight cap and the spring's clamp at 0 are gone. `strut_stops` is replaced; `anti_roll` runs per sub-step
  inside the same pass. `MW_WHEEL_TRACE=<file>` logs the wheels per tick. First try without the strut joints'
  reaction: a pitched car's springs pushed it back along its tilted up axis (0–200 km/h 11.12 → 11.38 s, top
  307.6 → 306.4 km/h, brake 38.1 → 38.8 m); with the across-axis part of the ground's impulse on the wheel passed to
  the chassis (model decision, handling.md) all three are back.
- **911 tests, before → after**: accel 0–100 3.73 / 0–200 11.12 s unchanged, top 307.6 → 307.3 km/h; brake
  38.1 m, corner 2.57 g, drift (peak 37.6°, held 4.75 s) unchanged. Landing deepest 0.104 → 0.125 m, overtravel
  0.051 → 0.071 m, body contact 0.82 s, settle 2.72 → 3.50 s; climb deepest 0.098 → 0.104 m, overtravel 0.069 →
  0.171 m, body contact 1.70 → 0.78 s, settle 4.72 → 5.00 s; kerb (50 km/h, 0.168 m, 30°) vy peak 1.36 → 1.78 m/s,
  pitch rate 41.4 → 36.3 °/s, roll rate 105.5 → 85.1 °/s, pitch −4.11..0.37 → −2.80..0.57°, roll −4.27..2.48 →
  −3.49..2.21°, rise 0.185 → 0.181 m, a wheel off 0.383 → 0.467 s, all four never, stop 3022 → 3116 N s; jitter at
  100 km/h pitch 0.81 → 0.92° p-p (1 Hz), roll 0.0092 → 0.0003°, vy 0.021 → 0.027 m/s p-p. The slower settles are
  the damper's × 0.5.
- **`ref/live1` at 66.5 s** (XKR, 118 km/h): it is a **drop-off only** (`mwtool surfaces --near 1572.5,551,6
  --steps`: the 0.153 m face at z ≈ 553–554, normal (0.18–0.20, 0, 0.98), lies across the path; the 0.122 m face
  listed near it is 4 m to the side). Aligned at the front axle's middle crossing it (t 66.532): the +X front wheel
  is 0.72 m past, the −X one 0.12 m short (32° to the axle); body heading −21.6°, velocity −10.4° (drifting).
  `--test kerb` got `MW_TEST_KERB_UP` (here 0: the car comes on a 0.153 m plateau and drops off) and the same
  binary geometry was built on master's physics for the "before": `MW_TEST_KERB_H=0.153 MW_TEST_KERB_UP=0
  MW_TEST_KERB_ANGLE=-31 MW_TEST_KMH=118 --drive xkr --test kerb`. Before → after: vy peak 1.18 → 1.54 m/s, pitch
  rate 10.5 → 12.8 °/s, roll rate 17.4 → 22.1 °/s, pitch −0.11..0.58 → −0.24..0.71°, roll −1.29..0.21 →
  −1.33..0.18°, a wheel off 0.167 → 0.133 s (each wheel 1–3 ticks), stop 26 → 1372 N s (the body lands on the
  stops); −21° (the velocity's angle) gives the same within 1 °/s. The original over ticks 0–35: pitch rate up
  to 16.8 °/s, roll rate swings −33 / +44 / −46 °/s, and its on-ground bytes (`w*_contact`) stay 1 on all four.
  Not a clean reference: the original was drifting (yaw rate ≈ −55 °/s), flicked the steering (0.99 → 0 at
  66.46 s, back to 0.94 at 66.66 s) and came over a crest (vy +1.5 → 0 m/s in the 0.3 s before the edge); its roll
  rate swings ±40–49 °/s again 0.5 s later on open road. So: pitch rates of the same order, our roll response to
  the drop smaller than the original's total, and the original keeps its on-ground flags where ours lose a wheel
  for a few ticks; what that byte means is not traced (its writer was not found; over all of `ref/live1` it drops
  for one 120 Hz sample about 650 times at speed, a mid-tick read, and for longer only 20 times).
- **Model decisions** (handling.md): the wheel moves along the car's up axis only, the strut joints' reaction
  stands in for what holds the game's wheel on its strut; the spring's anchor is our strut end (only the sag
  matters along the strut); a contact record exists where the ray finds ground within 0.3 m below full droop;
  wheels resync to rest beyond 1 m; steep ground's normal change is taken along up / max(n · up, 0.5); the in-air
  extra gravity reaches the wheels once per tick.
- **Open**: a clean capture of the original over a kerb or drop at a known speed, straight, with the wheel records
  (+0x00 centre, +0x40 velocity) and the spring elements' +0x3c (spring + damper force) in the watch list, to
  compare tick by tick; the on-ground byte's writer; the strut joints; the cylinder's ride-up on a kerb edge (our
  ray meets the step at once); the original's landing settle against ours (3.50 s now).

### 2026-10-10: chase camera: frame-by-frame against the original, and the missing stages ported
User report: "the camera still isn't like the original" (lag / swing, distance / height; "the original feels more
physics based"; "stiffly attached, while the original kind of looks ahead sometimes and moves a lot more over
jumps"). Method: a replay harness that runs **our camera code** over **the original's recorded car motion**, so
the motion is identical and only the camera rules differ:
`tools/camcompare.py inputs ref/live1 <in.csv>` (one line per camera update: the car frame cam + 0x500 / the
body's rotation / the wheel-plane frame cam + 0x4d0, body velocity, air time, the car's id) →
`mostrusted --camera-replay <in.csv>,<out.csv>` (`camera::replay_recording`, the same `CameraState::update` /
`finish` the game runs, no window) → `tools/camcompare.py compare ref/live1 <out.csv>` (both cameras in the car's
frame; look-around / look-back updates (cam + 0x6f0 ≠ 0, + 1 s), 41 torn records and impact-shake updates (random
noise in both) left out; 11 668 moving updates).

- **`ref/live1` holds three cars**, not one: Jaguar XKR 1399602 (0–20 s, 1762 kg), McLaren MP4-12C 122765
  (20–145 s, 1350 kg), Mercedes SL 65 AMG 122769 (145–258 s, 1807 kg), told apart by the body's mass (+0x7c) and
  confirmed by state + 0x44 / + 0x60 (the data eye / look-at) and the centre of mass (body − cam + 0x500, exactly
  the data's `centre_of_mass` each). Earlier single-car fits (round 8-11: eye (0, 1.575, −4.6), WL_RATIO 0.425)
  mixed them. The followed point cam + 0x500 is the model origin (body position − R · centre of mass), the same
  point as our car transform.
- **`.bss` values read from the memory snapshot `re/out/live/snap2`** (it holds the exe's whole data and bss; no
  need to run the game): `DAT_0110e770` 22.352 (= 50 mph in m/s), `0110e4d0` 0.75, `0110e680` 4, `0110e610`
  0.01, `0110e640` 5, `0110e8f0` 45, `0110e4e0` 0.9848 (cos 10°), `0118a060` 2.23694, `01189f60` π,
  `0110e550` 1, `0110e790` 0.05; `.rdata` `00fb0a20` 0.12, `00fafb8c` 0.3, `00fb0840` 2.6, `00fb0850` 0.4.
- **Before → after, RMS difference ours − original (original's std)**: eye x (swing) 0.139 → 0.042 m (0.90),
  eye height 0.130 → 0.023 m (0.17; correlation 0.69 → 0.99), eye z 0.361 → 0.038 m (0.54; 0.83 → 0.998),
  distance 0.366 → 0.030 m, view yaw relative to the car 0.64° → 0.47° (8.4°; 0.997 → 0.999), **view pitch
  relative to the car 2.22° → 0.23°** (1.67°; correlation 0.20 → 0.99), world pitch 2.22° → 0.23°, roll 0.19°
  → 0.19° (0.66°), **FOV 2.31° → 0.23°**. No lag between the two in any component (cross-correlation delay 0).
  Responses (original / before / after): eye z while accelerating > 4 m/s² −5.63 / −5.92 / −5.65 m; view pitch
  while braking < −6 m/s² −1.76 / −3.98 / −1.77°; eye swing per rad/s of yaw rate 1.337 / 1.172 / 1.330 m;
  at rest per car (eye height / view pitch): Jaguar 1.398 m 0.19° / 1.667 m −4.43° / 1.402 m 0.11° (it was
  parked on a slope: the gradient stage), McLaren −2.60° / −3.36° / −2.52°. **Jumps** (the recording's four,
  0.8–1.7 s in the air): range of the world pitch over take-off … landing + 1 s, original 21.5°, before 34.0°,
  after 19.7°; pitch difference 6.7–10.1° RMS before, 0.4–1.4° after. **Look-ahead**: the original's view turns
  towards the direction of travel by 13.2° at 10–30° slip (mean 17°); ours 12.0 → 12.2°; per rad/s of yaw rate
  −11.7 / −11.3°. It is the yaw lag (no separate look-ahead term exists); the "stiff" feel came from the pitch:
  ours followed the sprung body's pitch and the decel pitch 2.2× too strongly, and lacked the slope stage.
- **What was wrong / missing, each from the code** (decomp.md "Cameras", round 12):
  - the followed frame is vehicle + 0x2db0, the **wheels' plane** (`FUN_0090a5f0`), not the body: 3° RMS less
    roll, no squat / dive; its forward's height is low-passed (0.12 per update, within ±0.3) and the frame
    rebuilt round the world's up (fit of cam + 0x4f4: 1.6e−4). Ours followed the body's pitch.
  - the **composition** (`FUN_005c2130`): the look-at point rides in that frame, not in the smoothed one; the
    eye's offsets (spring, z) and the pivot are scaled by |cos(yaw lag)| (the range-reduced polynomial is a
    cosine); rebuilt from the fields it gives the recorded eye to 1–2 cm and the view to 0.15° / 0.035°.
  - **`FUN_005c7a80`** (not read before): the **deceleration pitch** (a look-direction turn, deceleration in
    m/s² along the velocity, only within 10° of the nose, a 2/s lag; ours used mph/s and a camera rotation), the
    **gradient adjustments** (slope ±10°: look turned back by 0.5 × slope, eye raised 4 × slope uphill / lowered
    1.5 × slope downhill, eased 0.1 per update, held in the air) and the look-at point following **half the eye's
    vertical spring** (0.4 in the air, 0.4 + 2.6u while a landing squashes it). Replayed: cam + 0x670 0.0003 rad,
    0x680 0.0007, 0x694 0.005 m (correlations 0.999); the look-at height to 0.03 m.
  - the z offset's constants (above): 22.352 m/s² full scale (we had 8.94: our offset was 2.5× too big), the gap
    clamp 0.75 m, ±4 m, and the airborne 1.85 m/s term. Replay of cam + 0x588: 0.011 m RMS (0.389 with ours).
  - the **FOV**: `powf(speed fraction, max / min)` (the decompiler dropped the call; from the disassembly), then the
    acceleration term's factor is exactly 1 per 60 Hz update (FOV RMS 0.44° in the field replay).
  - the **yaw stiffness**: `K1 + (YawDriftSpring − K1) · clamp(|slip°| / 45)`: 0.00035 RMS against cam + 0x6d4
    (0.0013 K1 alone); best-fit m 0.0603 = the McLaren's `YawDriftSpring` 0.06. Our drift switch and the 0.5
    drift lean (model decisions) are gone.
  - airborne: K_pitch eases to 0.1 (recorded 0.10–0.34 in the air); the landing shake request is
    `clamp((|a_y| − 10) · 0.00204) · 6.5` with a_y the vertical acceleration in m/s² (the four landings: the
    recorded level is the request after one update's decay); cam + 0x574's "landing height offset" is dead (it
    is overwritten with 0x554 before any use: 99.7 % of updates equal).
- **Still different / model decisions**: the yaw stiffness's standing value below 1.15 m/s (fit: the recorded
  switch lies between 1.0 and 1.4 m/s; `FUN_005c89c0`'s reset flag car + 0x3c not read); the z rate's
  `g · k60 · (1 − w)` term (vehicle + 0x2b20, a value in [−1, 1], 0 on 94 % of updates; 0.028 m RMS without it,
  0.011 with); the bob `FUN_005c8880` (recorded output std 7 mm: left out); the side-on ratio's box (ours the
  body box); the impact / high-speed shake noise (random in both); our wall push-in (no world query in the
  original's chase code). The wheel plane in our game is built from the four hubs (the original's four points
  at vehicle + 0x14..+0x20 are not identified; their behaviour fits the wheels). In the replay the landing
  request fires one update earlier than the original's (air time vs contacts; 1 of 4 landings missed its shake).
- Our game (911): `--test landing` level 0.87 at touch-down, eye dips 1.1 m on the spring, pitch held in the air;
  `--test brake` eye z −1.54 → +0.06 m; `--test drift` yaw lag up to 23° (no lean), FOV up to 94.8° from the
  acceleration term (|a| includes the lateral part, as in the code).

### 2026-10-10: the player's horn, impacts, grind and damage sounds (`player_sfx.rs`)
Request: the player's horn, and the scrape / grind sound against walls and other cars with its level rule.
Everything with its function and constant is in decomp.md "The player's horn, impacts and scrapes"; data
`mw_assets::car_sfx::PlayerCarSfx`, runtime `crates/mostrusted/src/player_sfx.rs`.
- **Horn**: HORN = action 5, W / pad B (`FUN_00483470` / `FUN_00482250`, already in decomp.md), now
  `input::defaults::HORN`. The sound is the first sequence of the car's `<id>_VEHICLESOUND` horn list (+0x60 /
  +0x64; the 911's `…1d1c69`: one wave item 0..5 s, gain 1.75, snapshot channel `Player Vehicle Horn`, gain 0.7 in
  `Mix_Main`). Dead ends for the runtime: no code reads a player horn list; `HORN_SOUND` is two 64-bit ids hashed at
  start-up (0x0117a4f0, 0x0117beb8) with no reader in the dump, `HORN_PRESS` an HSM node type. MODEL: plays while
  held, 0.1 s release.
- **There is no wall-scrape loop in the game.** The research log's "scrape branch `FUN_00929da0`" was wrong:
  `FUN_00929da0` classifies a point against the car's box, used by `FUN_0057caf0`'s *crash* branch (entity
  vtable +0x58 = byte +0x1c2b, the crash state) to play tumbling sounds by zone (+0x154..+0x164; not ported).
  What the player hears scraping a wall is `FUN_0057caf0`'s impact waves (at most every 0.1 s above 2.235 m/s)
  plus the damage sounds; the continuous grind (`FUN_0057f2e0`) is only against vehicles (race cars, traffic,
  parked cars): MetalCar pool `97b6a045` on `Player Vehicle Scraping`, waves chained 0.278 s before their end, gain
  `1.09^(−t)` from the grind's start (`powf` = `FUN_00ed9e1b`, base `.bss` 0x0110ce70 = 1.09), a 0.1 s fade after
  the last contact, then a 0.05 s release; pitch `R^(2x−1)` with R = 1 (constant). A new grind plays a `Sparks`
  wave (pitch 1..1.3, ≥ 0.1 s apart). The old `sfx.rs` scrape loop (looped `Scratch` waves by sliding speed, sparks
  above 5 m/s) had no source and was removed.
- **Damage sounds** (`FUN_00585030` → `FUN_00576220`): the damage grid (`paint_wear::damage_grid`) drives them: a
  cell's zone level first passing its threshold (`.bss` 0x0110c990: 0.01 at the four light corners) plays a
  `1ae351a5` wave on `Player Vehicle Glass Smash` (light glass breaking); with a race car in contact, a cell's
  scratch level rising by > 0.106 in a frame plays a `Scratch` wave (`Player Vehicle Debris`, gain 0.196).
- **Impacts** now use the traced player slot table (light / medium / heavy × front / side / rear, heavy adds a
  +0x108 wave) and the surface wave on `Player Vehicle World Collision`; `sfx.rs`'s loudness-sorted cues remain only
  for landing thumps (a model). The surface sets' player slot +0xf4 is a `RaceCarEffectSequenceItem` timeline (no
  sound). The contacts come from every physics step (`particles::ImpactEvent`, now with the other body): reading
  the last step's `Collisions` once per frame missed the impact step at 24 fps (a 14 km/h wall tap played nothing).
- **Offline** (`mwtool traffic-sound player out.wav`, before placement): horn 5.09 s, −23.4 dBFS RMS (stopped after
  1 s: 1.46 s with the release); player impact slots 2.5–2.85 s, −29.4..−34.8 dBFS; heavy extra −27.8; world waves
  0.65–1.48 s, −28..−34 dBFS (gain 0.714 at 10 m/s, 1 at 50 m/s); grind waves 0.68–1.13 s, −24.8..−27.8 dBFS;
  sparks 0.51–0.91 s, −43..−44 dBFS; glass 0.66–0.92 s, −29..−32 dBFS; scratch 3.0–3.2 s, −45 dBFS.
- **In game** (`MW_AUDIO_METER=1 MW_CAR_SFX_LOG=1`, `--volume master=0`; out = rendered RMS before the volume):
  - `--drive 911 --units 30 --spawn=800,42,1688,71 --auto-throttle 0.6 --bench 45 MW_HORN_AT=6,7.5`: the horn at
    6 m, gain −0.3 dB, out −23.6 dBFS, released at 7.5 s; ramming traffic car 25 at 9.6 s: a player car impact
    (light front, −26.4 dBFS), the grind and sparks together; the 911 keeps pushing the car: 98 grind waves in
    35 s (one every ~0.45 s: 0.68–1.13 s waves handed over 0.278 s early), out −26.1 dBFS at the start falling to
    −37.7 dBFS after 14 s (1.09^−14 = −10.5 dB, predicted −36.6), 4 car impacts, 2 sparks.
  - into a wall at 14 km/h (`--spawn=-1045,25,1598,0 --auto-throttle 1`): front-left zone 2 % > 0.01 → one glass
    wave, −26.2 dBFS at 3 m; another wall run: one world impact wave.
- MODEL decisions: horn held / released, the contact value read as "the pair pushed this step", closing speed for
  the impact level, the default surface, voices placed at the car, 12 one-shot voices, the grind's x from speed /
  top speed. Open: the horn's runtime, the crashing car's zone sounds, landing sounds (`FUN_00576b40`, the surface
  set's +0x124 / +0x158 pools, `Landings_Large`), the AI racers' contacts still read once per frame
  (`car_sfx.rs`), the `VfxGlobals.VehicleScrapes` effects for car-to-car contacts (the code that picks one of
  its 36 slots was not found; our scrape sparks stay the surface's side-impact effect).

### 2026-10-10: parked cars (static vehicle records) placed and spawned as the game does
Request: spawn the parked cars where the game places them, with the right models and paints, as loose bodies.
Details in decomp.md "Traffic runtime" → "Parked cars", the record in formats.md (`mw_formats::TrafficStaticVehicle`).
- **Records**: 392 over the hulls, all with chance 100, 34 pursuit-flagged, the 12-byte tail always 0; each flow
  type (6–22) names one vehicle type. `mwtool traffic` prints the survey, `mwtool traffic --at x,z,r` lists them.
- **Placement** (`FUN_0060db90`, with the hull's lanes): an LCG draw per record against the chance, the flow type's
  vehicle (`FUN_00608c80`), the type's next paint (round robin, `FUN_006080d0` with no colour), 150 slots
  (`world+0x2bbd8`). **Spawn** (`FUN_0060b2f0`) at the record's matrix; when the area starts over, records within
  30 m of a race car are skipped (`world+0x35f94` = 900, read in `snap2`).
- **Snapshots**: the live record table read in both. `snap2`: the same active hulls as ours, 77 live against our 79
  (missing in the original: hull 232's records 3 and 4, both pursuit-flagged; open). `snap1`: 66 placed (60 spawned,
  6 skipped by the 30 m rule on the car park's upper decks), ours 72 (missing: hull 255's ground-floor records 5–7,
  13, 15, 16; open: `FUN_005f09a0` → `FUN_005ec6b0` releases records flagged 4, its condition not read).
- **Runtime** (`traffic.rs`): kinematic bodies with the traffic models (same loader requests, 250 m draw range),
  removed with their hull; a dynamic body's touch turns one into a crashed car (as hit traffic, and the impact goes
  to `car_impacts.rs`). Checked: `--test crash-parked` (new) at (925, 24, −101) puts the 911 10 m in front of the
  nearest parked car standing on collision: "parked car 0 (hull 213 record 0, VEH_122819_TR) hit, now crashing", the
  van pushed across the lot (screenshot). Screenshots of the lot at (−981, 28, 1626): four cars in their paints
  sitting on the yard (`--camera=-990,38,1598,-982,28,1627`).
- **Found**: many records stand where there is no collision (yards, behind walls, car-park decks: the hull 131 yard
  has none at the cars' height, its only nearby surface is at 30.7 m); they are scenery the player cannot reach.
- MODEL decisions: the start-over flag (`world+0xa2bee`, writer not found) read as the first area after loading,
  the clearance against the player only, no pursuits.

### 2026-10-10: the map screen (event selection), the event-loading card, key-box prompts
Handoff item 4 (front end). New: `map_screen.rs` (screen 899991, MAP tab), `event_card.rs` (the PLAY_EVENT
transition), races started at runtime now get their intro / outro (`cutscene::run_event_cutscenes`).
- **How the original reaches the map** (HSM, `mwtool hsm <id> --parents`, new): the pause screen's MAP item
  exits `SHOW_MAP` → "In Game Menu" 469008 → "Suppress Prefetched Screens -> Run Map Screen" 1695290 → "Run PauseMap
  Screen" 1695291 → "Show Main Map" → "Run PageRunner2 Context - Main Map" 899996 (`DEF_PAGE_REF` 899991) → "Run
  Map Screen" 1607028 (`Screen.Session.NavTabName` = MAP) → "Run Map Options" 1676747: widget groups 899993
  (`MapControls` + `TabActions`), 1676900 / 1676899 (persistent map, map layouts 1695072), 2012629 (map title).
  `MapControls` exits `TO_OPTIONS` → group 1222205 (`Options`, script `MainMapOptions`), whose list for an event
  is the `Event` menu data 1222202: "SET DESTINATION" (`Map:OnSetWaypoint`), "START RACE"
  (`Event:OnJumpToEvent:PLAY_EVENT`). Group 1222205's `ScreenTransition`s: `CAR_SELECT` → 1524151 (sequence 1524152),
  `PLAY_EVENT` → 1453803 (sequence 1014321). Its other exits: `EVENT_CARSEL` → "Event Car Select" (group 1695023:
  menus 1695026 / 1695037 "SELECT CAR", `CarStats`, `CarImages`; `PLAY_EVENT` there runs
  `SetEventFromAvailableVehicles`), `TO_JKSPOT`, `NEW_GAME`, `QUIT`. The multiplayer garage reaches screen 1571371
  ("Customise Rollouts Multiplayer": manufacturer → model → mod type → mod groups 1571372 / 1571382 / 1571389 /
  1571390); 1135879 was not reached from these trees.
- **Capture of the original** (`tools/orig`, audio muted, save backed up to `ref/savebackup-2026-10-10/` and
  restored byte-identical afterwards; screenshots kept in `ref/frontend-2026-10-10/`): pause menu (o4), map with
  the player selected (o5), an event selected (o6, "MOST WANTED 8": a transient red glow on every road), its options
  (o7: SET DESTINATION / START RACE replace the card's SELECT row), START RACE → the event card (o8–o10: transition
  pictures, the Most Wanted list on the left, then the speedwall, the event strip "MOST WANTED 8" with "BEAT THE
  MERCEDES-BENZ SL 65 AMG", the tip strip) → the intro (o11). The map's left panel is the Most Wanted list (group
  899992, `MwlLayouts` 1470720), the bottom the navigation tabs (PGUP, five icon tabs, MAP selected, PGDN) and the
  Autolog ticker, top right the player's header, bottom right the filter "RACES" between Z and A.
- **What the RACES filter shows** (fitted, see below): the current car's events in green (the F-150 Raptor's TURF
  WAR, LIBERTY PARK (an ambush), ROLLING STOCK matched icons) and the Most Wanted races in red. The icon is the
  `POI` look 1222133 (key 1135497) = layout 816139, the green diamond `01010000000f789a`, with a triggered step
  `TO,MAP_EVENT_DONE` on `~LiveMapManager.ShowEventComplete[#]`: red = beaten (the user's save had beaten every
  Most Wanted race). The selected event's card is look 1135809 (key 1135808) = layout 1481372 (type, name,
  difficulty, `Events.Loading.Difficulty`, route length, the SELECT row on `LiveMapManager.SelectedIndex(EQUALS:#)`,
  ten elements `OUT` on `~Widget.Options.IsActive`); the cursor look 1135304 (key 1135303) = layout 1470763 (three
  additive rings while `MapControls` is active, and the option list: a stack over `~Widget.Options.Items` at
  y = `~Widget.Options.MainMapOptions.OptionHeight`); the player's arrow look 1222137 = layout 371789 (`rot: Rot`).
  The Most Wanted events' own card (the capture's, with the rival car and the speedwall) is probably 1481445 /
  1481350 (keys 1135735 / 1135737); not used.
- **The map image**: the `POI` widget's map layout 1222145 draws `01010000002195ca` (1024², BC1: terrain, roads,
  city blocks) as a 2000 × 2412 map-unit rectangle. Registered against the `UIMapGrid` tiles (whose world
  placement is data: origin (−4129, −4920), 1036 m tiles, 8 × 10) by maximising the correlation of their road lines:
  it covers x −4000…3990, z −4581…5406 (registration, error ≲ 10 m).
- **Camera** (`UIMapCameraConfig` of the screen's `POI`: look-at `Widget.MapManager.SelectedPosition`, offset
  `LiveMapManager.ScreenLookAtOffset` (0.25), distance 7000, FOV / tilt from `Widget.POI.MainMapZoom.Fov` /
  `Widget.POI.ZoomAngle`, `LookAtRotation` 0): the selection sits at (836, 360) of the 1280×720 frame in both o5
  and o7. A least-squares fit of eight icons of o7 (the green and red diamonds) against the events' start points,
  with Most Wanted 8 at the anchor: world +X points 94.4° clockwise from screen-right (≈ down), +Z ≈ left, 6.29 m per
  reference pixel, residuals 1–15 px (one Most Wanted icon 36 px off: its icon may not sit at its start). A
  pure 90° turn fits clearly worse (cost 3268 against 1322). Image registration of the screenshot against the
  map image failed (the original's road rendering differs; several spurious mirrored optima) — the icon fit is the
  measurement.
- **Event-loading card** (sequence 1014321, 8.5 s): black shape 26458 and the conducted card 1014274 (0–8.25 s),
  black 1689864 (8.25–8.5 s); controllers: 0.067 → 0.017 while `Screen.TransitionsLoaded(NOT)`,
  **`JumpMonitorTimelineController` 0.15 → 8.0 with no binding**, 8.233 → 8.017 while `Screen.IsActive(NOT)`.
  Our reading of the monitor: the outer clock waits at 0.15 s until the conducted card has played out (a plain jump
  would cut the card at 0.15 s). The card 1014274 (8.5 s): picture sequence 1899171 (as in the boot transition),
  black quad, tip strip 1899089, header strips 1675179, the Most Wanted list (conducted 2041453, 0.75–4.5 s,
  `SequenceMwl.Items`), the event panel (1675921, or 2041418 for `Event.Data.IsBlacklistEvent` = Most Wanted
  events), rewards 1014298 (`Events.Loading.Reward` / `SecondReward` / `ThirdReward`, `TargetSpeed[n]`), the
  speedwall (conducted 1675879, 4.5–8.5 s), holds 7.733 → 7.517 while `Screen.IsActive(NOT)` (8.0 → 7.767 on
  `UIMessages.NISStalled` / `System.IsSimulationPausedForStreaming`) and the general trigger 3361386002 at 7.75 s:
  the EventIntro context's "Screen Transition Voodoo" waits for `GeneralSequence.Triggers["TRANSITION"]` before "Run
  intro sequence", so the intro starts under the card's last 0.75 s.
- **Element stacks run across too**: `UIElement_ElementStack.ae583a9a` (+236, a 2-byte enum) is 1 on every list
  (pause, HUD, Easydrive, map) and 0 on the navigation tabs: our reading, 0 = cells left to right (formats.md).
- **Key-box prompts**: the keyboard's button prompts are `UIListItems` 113577 (plain names: 113578):
  `U+F092 U+F040 U+E047 U+E048 ESC U+E049 U+F012 U+F093`; the font's `U+E047` / `U+E048` / `U+E049` are the box's
  left cap, fill and right cap; Return is `U+E06B`. `Font::layout` now stretches the fill under the name; the box is
  drawn light and the name dark (the `U+F0xx` style codes are not decoded).
- **Runtime races**: the cutscene systems run while a `Setup` resource exists (`--race`, or inserted by the
  card), so races started from the map get their intro and outro; `CutsceneState.hold_intro` keeps the intro
  until "TRANSITION". Checked: KEYS TO THE CITY from the map, card 0–7.75 s, held for the opponents and the start
  area (0.1 s with the start area loaded, ~18 s while the full world streamed), TRANSITION, intro 1912702 plays
  (`MW_CARD_SHOTS`). A full-world run on the shared GPU (other agents' 5 GB) quit with a wgpu out-of-memory error
  as the intro began; with `--units 20` near the start it runs through.
- **Model decisions**: the flat orthographic camera with the fitted turn and scale and our easing (8/s); arrows
  pick the nearest item within ±60° of the direction (the `LiveMapManager` Lua script is not decoded); no
  completion state (all green); the Most Wanted list, the player header and card, the speedwall rows, rewards,
  the road glow (layouts 1222163 / 1222558) and the tab icons are left out; SET DESTINATION, ambushes / speed runs
  and the other filters are inactive; the option list is moved off screen while the options group is inactive;
  the race is started 0.25 s into the card; the card's difficulty is the event's first allowed car's; key-box inks.
- **Open**: car select for events the car may not enter (`EVENT_CARSEL`, group 1695023) and the customise screens
  (1571371, 1135879); the event-intro card for Easydrive's START RACE (does the original play 1014321 there too?);
  `MAP_EVENT_DONE` from the save's progression; the Most Wanted card / speedwall; why the map turns 94.4° (the
  map camera's `LookAtRotation` is 0; tilt?); the map's 3D projection (`ProjectionAngle`), zoom; the AI placing both
  opponents of KEYS TO THE CITY at one point (seen in the runtime race, `ai.rs`).

### 2026-10-09: frame drops: the render thread's multi-draw path, physics catch-up, bakes on every core
User report: "optimize this game, it is weirdly dropping frames sometimes". Measured with a new per-frame log
(`MW_FRAME_LOG=<file>`, `frame_log.rs`: main schedule segments First / PreUpdate / fixed loop / Update /
PostUpdate / Last, the gap to the next frame, the render thread's segments assets / specialise / queue /
prepare / graph, fixed steps, loader messages, entity / mesh / image / material counts), `--bench` now printing
p95 / p99 / max / hitches > 50 ms plus `BENCH MESHES` (visible / total mesh entities by kind) and `BENCH
MATERIALS` (distinct visible materials), and chrome traces (`--features bevy/trace_chrome`, one build).
**Caveat**: the machine was shared all session (load 8–27: another agent running the original game, other
builds, and for the first hour ~11 GB of this session's binaries / traces on the RAM-backed /tmp, which pushed
the machine into swap); every A/B below is interleaved, same command, back to back, and the absolute numbers
are high for that reason. Drive = `--drive 911 --units 30 --spawn=800,42,1688,62 --auto-throttle 0.4 --mute`.

- **Where the time went** (drive trace, master): the render thread set the frame time (45.6 ms of render
  schedule per frame against 22.9 ms main). Its biggest items were Bevy's multi-draw-indirect bookkeeping:
  `write_binned_instance_buffers<Opaque3dDeferred>` 6.2 ms, `<Shadow>` 3.8, `prepare_preprocess_bind_groups`
  5.9, the `unpack_bins` dispatch encoding 3.8, `queue_submit` 6.8. That path keeps, per **batch set** (one per
  material bind group) and per view, two GPU buffers written with their own `write_buffer`, a bind group and a
  compute dispatch; downtown **959 distinct materials** are visible (10 300 visible mesh entities: world 5 134,
  props 3 465, alpha-tested 1 156, traffic ~300, backdrop 170), in the main view and three cascades.
- **Fix 1 (committed): direct draws.** `NoIndirectDrawing` on the main and the planar-reflection cameras (the
  shadow views inherit it): GPU mesh preprocessing stays, the per-batch-set machinery goes. `MW_INDIRECT=1`
  restores the old path. Drive A/B (master → new, three rounds of 30 s at load 17–27): frame mean 108 / 63 /
  51 → 33 / 41 / 18 ms; render prepare 52 / 29 / 24 → 7.1 / 9.4 / 3.9 ms; render graph 53 / 32 / 26 → 24 / 30 /
  13 ms. Earlier rounds (20 s, load 9–15): 47 / 50 / 70 → 31 / 26 / 25 ms, frames > 50 ms per 20 s: 0–62 vs
  108+ on master. Race (`--race 462174 --units 30`, 40 s): 79 → 21 ms (load 10–19), 38 → 39 ms (load 14,
  disturbed). Free camera over the full map (city overview, 3 846 visible): 23.6 / 21.9 → 28.1 / 16.8 ms (noise;
  fewer visible materials there). GPU utilisation with direct draws 34–79 % (median ~50 %): still CPU-bound.
  **Pictures**: road, night + planar reflection, grass, city, car within run-to-run noise (road 0.28 MAD vs
  0.50 between two master runs); one car shot differed in the glass, a rerun of the same mode matched master
  bit for bit (a slow first frame, not the mode). Drive / drift shots are not deterministic.
- **Dead end: CPU batching** (`PbrPlugin { use_gpu_instance_buffer_builder: false }`): looked 3× faster but
  drew no world geometry at all (only sky and the car) with no error; not used.
- **Fix 2 (committed): physics stalls on the async pool.** Avian optimises its collider trees on Bevy's async
  compute pool and `block_on`s the result later in the same step; that pool also compiles pipelines. Under load
  `block_on_optimize_trees` averaged 13.7 ms per frame (normally 0.01). Now inline (`ColliderTreeOptimization {
  use_async_tasks: false }`, ~0.03–0.09 ms a step; `MW_AVIAN_ASYNC=1` = before). `--test accel|brake|corner|
  drift` TEST RESULT lines identical to master.
- **Fix 3 (committed): environment bakes off the frame's cores.** A bake (painted timeline change while driving,
  F5–F7) built its cube maps on the global rayon pool: 108 ms with every core busy (the main and render threads
  starved for that time). Now on a 2-thread pool of its own: 234 ms in the background, same texels
  (`MW_BAKE_THREADS=n`, 0 = global pool). Bakes now log their duration.
- **Small (committed)**: particle batches that stay empty are no longer re-uploaded each frame (every mesh
  change makes Bevy walk all mesh entities in `mark_3d_meshes_as_changed_if_their_assets_changed`).
- **The spiral**: fixed steps per frame follow the frame time (drive on master: 2–3 steps normally, up to 15
  after a slow frame, ~3–8 ms each), so one slow frame makes the next ones slower; most clusters of > 50 ms
  frames in the logs are this plus machine contention (main fixed loop and render graph slow together, no
  loader messages). The game itself advances at most 2/60 s per frame (decomp.md "Update loop": one 2/60
  catch-up step, even after a 4.7 s stall); Bevy catches up to 250 ms. A clamp of `Time<Virtual>` max delta to
  2/60 s (before `--test`'s speed-up) was written but **not committed** (not measured yet; it changes play below
  30 fps, deliberately, toward the original).
- **Streaming events**: traffic model arrivals (+11–24 meshes, +1–10 materials, 3–16 loader messages) cost
  ~5 ms of uploads in the render thread; not a hitch on their own. Race start: one 190 ms frame (render
  specialise 60 ms + queue 49 ms) when the race's many new entities first become visible.
- **Next, by the numbers** (direct draws, quiet-ish machine: render ~14–19 ms, main 8–11 ms): (1) the render
  graph's draw encoding (deferred prepass ~5 ms + 2.4 apply, shadows ~5 ms over three cascades): fewer draws:
  the game's model LOD / draw distances (`Model.lod_distances` unused: every placement is drawn at LOD0 at any
  distance; 3 465 visible prop meshes downtown) need tracing in the decomp first; bindless `GameMaterial` would
  let Bevy merge draws across materials (a `layered.wgsl` rewrite); (2) commit the 2/60 s clamp after an A/B
  and the handling tests; (3) `GeneratedEnvironmentMapLight` re-filters the sky every frame for the forward
  materials (~0.8 ms render CPU plus GPU): a static prefiltered map from the bake would do; (4) pre-warm the
  pipelines and specialisations of the race's start area during loading; (5) per-step physics cost (~2.5–3.5
  ms a step under load, mostly executor overhead of avian's 6 substeps).

### 2026-10-09: kerbs threw the car (the strut stop's sidewall slack ported)
User report: "the curbs are making the car way too jumpy."
- **Kerbs** (new `mwtool surfaces --near x,z[,r] --steps`: low steep faces, nearest first): downtown kerbs are
  vertical faces of paving surface 9, 0.15–0.23 m high, mostly 0.168 m (around 800, 1688). No surface roughness
  is involved (paving: 12 mm over ~3 m; vertical faces are never under a wheel ray), and the flat track shows the
  same, so the surface bumps of today are not the cause: it is the suspension's step response, older than them.
- **Reproduced** with the new `--test kerb` (flat track, a 0.168 m slab 3 m deep with vertical faces, edge 30°
  off square, 50 km/h held; `MW_TEST_KERB_H` / `_DEPTH` / `_ANGLE`, `MW_TEST_KMH`; `MW_SUSP_TRACE` logs pitch,
  roll, rates and the stop impulse per tick). 911 before: the front-right wheel meets the step, the ground is
  0.108 m past its strut end, and our hard stop put 2161 N s into the chassis in one tick (roll rate −190 °/s,
  pitch −84 °/s); three wheels left the ground and the body rose 0.227 m over a 0.168 m kerb.
- **The game's stop** (`FUN_009c5a90`, read in full, decomp.md): the chassis takes max(d − min(d, slack, 0.1),
  0.1 d) of the predicted penetration per contact pass, slack = constraint +0x470 = **the tyre sidewall**, wheel
  radius − `WheelSizes.337afbd9` (rim radius; `FUN_0095b4d0`), 0.074 / 0.073 m on the XKR in the snapshot
  (0.065 / 0.068 on the 911). Ours took all of it (the model decision "slack 0"). The wheel itself follows the
  ground (its velocity raised by d / dt) and its spring keeps compressing.
- **Ported**: `strut_stops` (2 contact passes with the slack rule at the strut end, then 2 impulse sub-steps
  scaled by the share), `VehicleHandling::rim_radius` (per axle through `VehicleInfo.Axles`; the 911's front
  axle points at the higher-id `WheelSizes`, so not by id order), the spring no longer clamps at the travel.
- **After** (911, `--test kerb`, 50 km/h, 30°): stop impulse on the first hit 2161 → 993 N s, peak roll rate 190 →
  106 °/s, pitch rate 84 → 41 °/s, peak |vy| 1.96 → 1.36 m/s, roll swing −6.3..4.0° → −4.3..2.5°, pitch
  −4.9..1.7° → −4.1..0.4°, rise 0.227 → 0.185 m, time with a wheel off 0.53 → 0.38 s (all four never).
- **911 tests**: accel (0–200 km/h 11.12 s, top 307.5 → 307.6 km/h), brake (38.1 m), corner (2.57 g) and drift
  (peak 37.6°, held 4.75 s) unchanged. Landing: deepest 0.056 → 0.104 m, overtravel 0 → 0.051 m, body contact
  0.82 s and settle 2.73 → 2.72 s; climb: deepest 0.049 → 0.098 m, body contact 0.78 → 1.70 s, settle 4.33 → 4.72 s.
  Both as expected from the rule: within the sidewall the stop only takes a tenth per pass, so the body sinks
  about the sidewall (0.065 m) further onto the stop before the rim holds it; the extra body contact on the climb's
  kink is the body box touching the slope while sunk. Not checked against the original's landing yet.
- **The original** (`ref/live1`, XKR): at 65.8–66.8 s it crosses a 0.153 m kerb face at (1571, 554) at 117 km/h,
  ~27° off square, dropping off the pavement: pitch rate up to 21 °/s, roll rate ±45 °/s, vy −0.96 → +0.73 m/s, all
  four contacts kept. Same order as ours now; a like-for-like run (our XKR over a drop-off at that speed) is next.
- **Open**: the game's damper acts on the wheel record's velocity (a 25 kg point mass the contact pass drives
  onto the ground), so in the original a step also kicks through the damper and the wheel's momentum; ours has
  a massless ray wheel. Porting the wheel as a point mass (spring `FUN_0092f510`, damper `FUN_0092f160`, contact
  pass) is the next step for kerbs. The strut top is −0.077 m chassis-local on both XKR axles (bump travel 0.056 /
  0.067 m above rest; ours 0.06 for every car): its data source not found. A ray wheel still meets a step at
  once where the game's cylinder rides up over √(2rh) ≈ 0.3 m (slow speeds).

### 2026-10-09: car badges and decals cut out (vehicle `_Alpha1bit` / `_Alpha_` shaders)
User report: the 911's rear "PORSCHE" letters drew as solid rounded blocks and "911 Carrera S" sat on an opaque
silver plate. An old bug: every car's badges, the traffic cars' grille logos and hub caps were opaque quads.
- **Which material**: the badges are `VehicleNFS13_Body_Alpha1bit_NormalMap_Textured_NoDamage` (the 911: body
  renderable `b5cdc877` mesh 2, 28 triangles, diffuse `01000000001026c2`, BC3, an atlas whose **alpha** holds the
  letter shapes, rgb a chrome gradient). Over all `VEHICLES/*.BNDL` (`mwtool models --verbose`): Body_Alpha1bit
  Textured 176 meshes, untextured Body_Alpha1bit 11, Wheel_Alpha1bit(_Textured) 1616, Body_Alpha_ 32,
  Wheel_Alpha_* 808 (traffic cars mostly `Wheel_Alpha_Textured_Normalmap` / `_Blurred_Doublesided_PixelAO`).
- **DXBC** (`mwtool shader-asm`): the `_Alpha1bit` G-buffer programs (4 render targets) start with `sample t0;
  add r0.w, -0.5; lt; discard`: discard where the mask's alpha < 0.5. The mask is `DiffuseTextureSampler` in
  the `_Textured` variants and `NormalTextureSampler` (its alpha) in `Body_Alpha1bit_NormalMap` /
  `Wheel_Alpha1bit_Normalmap`. Their shadow / depth programs discard below 0.501961 (128/255). The `_Alpha_`
  (blended) variants have **no G-buffer pass**: their main programs are forward (one target, lit with
  `IrradianceQuadric`, `KeyLight*`, the shadow map and `g_PerVehicleFog`), o0.w = the diffuse alpha (untouched
  by the fog), and `Wheel_Alpha_Textured_Normalmap` discards below 0.125 first; `_BlurFade` multiplies the
  alpha by 1 − TEXCOORD2.z = 1 − `g_wheelConstants.x` (vertex program) and discards where that is < 0.125.
- **Material states** (`shader-asm` now prints the `MaterialState` of each pass slot): the `_Alpha1bit` G-buffer
  passes use the opaque G-buffer state `01000006d979a79a` (blend off, like `Body_Textured_NormalMap_NoDamage`):
  no alpha-to-coverage, the cut-out is the discard alone. The `_Alpha_` forward passes use `010000063eb1486e`:
  rt0 `7908498b` = colour src alpha / inv src alpha, add; alpha zero / zero; write mask rgba (alpha-over).
  The rasterizer block's second word is the D3D11 cull mode (3 back on bodies, 2 front on wheels, 1 none on
  `_Doublesided`), and the header's third word (+0x14) is the rasterizer state's hash (`dbe57716` / `2a3f72bc`
  / `e3207a03`). formats.md's blend-block layout is corrected: the 8 packed targets start at the block offset.
- **Cause**: our vehicle branch of `layered.wgsl` forced the albedo alpha to 1 (`vec4(albedo.rgb + ..., 1.0)`,
  and the texture alpha was never the coverage), so `AlphaMode::Mask` never discarded; the `_Alpha_` variants
  were `Blend::Opaque` (their names hold neither "alphablend" nor "1bit").
- **Fix**: `materials.rs` sets p8.w = alpha source (1 diffuse, 2 normal map, by the rule above) and p9.z = the
  forward discard threshold; `layered.wgsl` keeps that alpha in the base colour. `_Alpha1bit` vehicle looks use
  `AlphaMode::Mask(0.5)` (G-buffer, the deferred prepass discards; the forward AlphaMask3d copy is discarded
  as for trees), `_Alpha_` ones `AlphaMode::Blend`. Checked before / after (rear, front, wheel close-ups): 911
  (PORSCHE / 911 Carrera S cut out, crest), XKR (JAGUAR letters), M3, Veyron (wheel centre text), Aventador
  (Lamborghini script), R34 (TE37 spoke label, `_BlurFade`), Diablo SV (Diablo / SV badges), Police Interceptor,
  traffic van `VEH_122819_TR` and hatchback `VEH_122816_TR` (grille rings and hub caps no longer on dark plates).
- **Model decisions / open**: the `_Alpha_` parts are lit by Bevy's forward PBR, not a port of their forward
  program (and they write no depth, where the game's depth state is the opaque one); `g_wheelConstants.x` (the
  `_BlurFade` fade, presumably wheel spin blur) is taken as 0, the wheel at rest: its writer is not traced. The
  untextured `_Alpha1bit` meshes do not cut their shadows (Bevy's standard prepass reads the base colour texture
  only).

### 2026-10-09: the Jaguar XKR's "broken spoiler" after contact (the game's damage pose blend ported)
User report: after contact in a race (XKR, CONTINENTAL DRIFT) a large dark triangle stood up from the boot like a
broken spoiler. Reproduced with `--drive xkr --test crash` (a frontal hit: front 28 %, front-left / front-right 11 %)
and `mwtool damage xkr --level all=0.5 --preview` (the boot lid flies up and sideways); a single zone
(`--level left=1`) looked almost right, which is why the earlier `rear=0.5` preview hid it.
- **Cause (data + our blend)**: `mwtool damage xkr --keys` shows bones 6, 7, 8 (boot lid hinge, 556 vertices; the
  two tail-light halves, 1,865 / 1,843 vertices) with the same non-identity key in **every frame of all 8 clips**:
  offsets (−0.117, 0.014, −0.004), (0.133, 0.048, 0.013), (0.101, 0.048, 0.010) (the clips were made on a rig whose
  bone 6 sat 12 cm further right). We added every damaged zone's key to the bind pose, so the constant went in once
  per damaged zone (3 times after a frontal hit, plus the rear clip's rotation of the same bones), and one zone at
  1 % already shifted the boot lid 12 cm.
- **The game's rule (decomp.md "Vehicle damage pose", confirmed in the dump)**: `FUN_005b62f0` applies the clips in
  index order through the additive blend `FUN_009f3250`, each key as its change from the clip's **reference frame**
  (Anim +0x36, 0 in vehicles; our parser had it as "not applied"): t += T[f] − T[r], q = (Q[f] Q[r]⁻¹) q, but scale
  absolute (the last clip that scales a bone wins), only on the channels a clip animates. Levels round to whole
  percent and snap down to whole frames (no blend between frames), capped at 0.99 while not wrecked, skipped at 0.
  Model space and skinning use bind positions only (`FUN_009f4560`, `FUN_0059fa70`): the bind rotations are ignored.
  So the constant keys cancel in the original.
- **Ported** in `VehicleDamage::pose_with` (`Animation` now carries `reference_frame`, `bone_mask`, `channels`).
  Part clips (active aero) go through the same blend after the damage; their player is not identified
  (candidate `FUN_0047bd00`, which calls the same additive blend), and the Veyron's transition clips start from a
  deployed pose, so the HSM probably layers them; open.
- **Other cars** (`mwtool damage all`, new column): 30 of 72 cars have frame-0 offsets on damage-clip bones; most
  under 3 cm, but R34 Skyline 0.78 m (bone 16 = the steering wheel, 389 vertices: the front-left clip moved it 77 cm
  to the left-hand-drive position on any front-left hit), Viper GTS 0.57 m (bones 9–12, 35), McLaren F1 LM 0.38 m,
  MP4-12C 0.21 m, SLS 0.18 m, Evoque 0.14 m, XKR 0.14 m, Camaro ZL1 0.13 m, Veyron Vitesse 0.07 m, Agera R 0.06 m,
  Impreza 0.05 m, 911 Turbo 0.05 m (M3 Coupé 3 cm). Every car changes slightly too: whole frames instead of
  interpolated ones, the 0.99 cap (frame 19 of 20 at most until wrecked), last-wins scale (a part detached by one
  zone's clip comes back if a later clip animates its scale), and tilted bind bones (every car has at least one,
  the steering wheel; 18 cars have 2–5, e.g. Huayra flaps) now rotate about model axes. All 72 cars use reference
  frame 0; bone masks are all 1.0 except on 9 cars, where some clips mask bones with 0.
- **After**: `--test crash` and `--damage all=0.5` show an intact boot; 911 damage and `--anim all=1` spoiler as
  before. Open: the distance fade (k at `.data` 0x0110d53c) for AI cars, the wrecked state (lifts the 0.99 cap),
  the aero player.
- **Tools**: `mwtool damage <car> --keys` (each clip's keys at frames 0, 1 and last; every frame of part clips;
  bind rotations in the bone list), `damage all` lists frame-0 offsets, reference frames, masks and rotated binds;
  `mostrusted --damage zone=level,...` starts the player car damaged.

### 2026-10-09: race cinematic cameras settled by captures (axes, FOV, DUTCH, outro binding)
User report: intro shots rolled ~40° or framing sky / trees, the outro framing an opponent top-down. Captured the
original (`tools/orig/orig`, headless, muted; the 4C at Hughes Park, Easydrive → Races → CONTINENTAL DRIFT →
Start race, screenshots every ~0.6 s through the intro), then compared shot by shot with ours
(`MW_CUTSCENE_SHOT=intro:<t>`).
- **The original's 462174 intro is upside down** from its second shot (3 s) to the teleport: road and buildings
  hang from the top of the frame, the sun at the bottom; the tall-building shot looks up at the towers upside
  down, the car passes the KNOCKING BROTHERS sign upside down. The first shot (an aerial over the park dome)
  rolls by 175° over its 3 s: the barrel roll that turns the intro over. So the camera bone's key is the camera
  matrix as stored: look +Z, up +Y, roll kept; the 49 "+Y down" clips are authored that way. Our earlier
  per-clip flip (up = whichever of ±Y is upward on average) made those shots upright and the first shot's roll
  meaningless; removed (`AnimatedCamera::y_down` gone). With the rule every one of the eight shots frames what
  the original frames (dome, upside-down road with the sun, curved overpass, overpass with the car, towers,
  arch, sign with the car, sky with a lamp post).
- **Subject survey** (all 741 camera items of the 111 events, `MW_EVENTS_KEYS` dumps): in 244 of 251 clips that
  run while an animated race car is on screen (or are bound to the player), the car is within 40° of the bone's
  +Z (7 on −Z). The skeleton's bind rotation for bone 1 (180° about Y; formats.md said "identity") would turn the
  look to −Z if applied, so it is not.
- **FOVs are horizontal**: the chase view of the 4C at rest (FOV 70, `MinFieldOfView`) frames the car ≈ 405 px
  wide in the original at 1280×720, ours 400 px with 70° horizontal and 220 px with 70° vertical (A/B with
  `MW_FOV_VERTICAL=1`); the intro's 19–35 mm lens shots match in size the same way (arch 260 / 280 px). The lens
  formula's 18 is half of a 36 mm frame's width. Converted in `camera::projection_fov` for chase, bonnet, crash
  and cinematic cameras (handling.md "Model decisions" corrected).
- **`DUTCH`** (code): × 65536 (`.rdata` 0x0106f9d0) → 16-bit angle → × 2π / 65536 (`FUN_00e22760`), so it is
  in turns and ±0.25 is ±90°; `FUN_00e230c0` pre-multiplies the camera rows (left' = c·left + s·up), as
  `camera_take` does. The 3 s intro takes `South`, `Trail`, `To`, `Get` sweep 0.25 → −0.25 → 0.25 (barrel rolls,
  the same style as 462174's first shot); `Default_No_Race_Event_Outro` holds 90° for 10 s (not captured).
- **The outro freezes where the car finished** (code `FUN_0049a910` + the take's `0bd6ff75` flags, and now
  captures): the race outro's top-down camera stays over the finish spot; finishing at 59 mph (the event's route
  entry count at DTF+0x510 cut to 1 in memory, restored by quitting) the car is at the left edge 0.5 s later and
  gone by 1 s, the rest of the outro shows only asphalt; finishing at rest the car sits at the left edge, as in
  ours. So "the outro frames road or an opponent" is the original's behaviour; no binding change.
- Captures: the scratch copies were not kept in the repo (game imagery). Harness notes: Easydrive's Start Race
  only appears for discovered events (Turf War offered only Set Destination); the event's `Laps` (+0xf6) set to 0
  in memory gives a zero-lap race that ends at once with the car at rest.
- **Open**: 1119174's first shot (0–3 s) puts the camera 0.4 m above the trigger height looking up 87°; ours draws
  the road's back faces (all world materials are double-sided), so it shows asphalt; whether the original
  culls them there was not captured. The animated cameras' `GetTransform` binding is still read live.

### 2026-10-09: the start-up flow: title, boot transition, loading screen, free-roam start, pause menu
User request: "work on getting the car spawns in, loading screens and menus". Before: the world assembled in
view, the car dropped at a `--spawn` / hand-picked point as soon as its car loaded (on whatever collision had
streamed in: planters, or nothing).
- **`HSMData` decoded** (formats.md "HSMData", `mw_formats::hsm`, `mwtool hsm`): nodes, `ENTER` events, children
  with exit-code transitions, parameters; keys base-40 packed (a sub-agent traced the interpreter: `FUN_00775ed0`,
  `FUN_007760b0`, behaviour registry 0x776000–0x778300 keyed by `~crc32(code name)`). The flow is now read, not
  guessed: "Top State" 21597 → boot page 491012 → "Press Start" (group 491032) → screen transition → "Post
  Controller Bind Boot" (load world, profile) → "InGame": "Wait For Required Resources" → "Setup Start Point"
  (new / continue) → "Playing"; the loading screen is "Manage Loading Screen" toggling layout 108177 on
  `Mode.ShowLoadingScreen` (decomp.md "Boot, loading screens and the free-roam start" has the code side: the
  world-loaded flag = world ready + Resident(camera, 10 m) + traffic or 1.5 s; placement waits for Resident(target,
  max(200 m, 3 s)); start = last / nearest swap spot of the car or `GameLogicConfig.beab0bc2` = trigger 1156487).
- **What the loading screens are** (data): the boot transition 1836020 (black, car pictures from
  `LoadingTipsConfig.TransitionImages` cross-fading every ~10 s, the strip with "LOADING..." and the tip ticker
  `Transitions.Tip`, holding at 4.05–4.25 s while `Screen.IsActive(NOT)`); in game, `UIConfig`'s lists hold only
  108177 (black, spinner 360°/s from its idle behaviour). The `Events.Loading.*` layouts are the event-intro cards.
- **New-game car**: `GameLogicConfig.DefaultRollout` 523000 → Aston Martin V12 Vantage (122672); start trigger
  1156487 = the Freedrive event "FAIRHAVEN" start (the intro event "WELCOME TO FAIRHAVEN" ends next to it).
- **Bevy UI bug found**: `ImageNode` with the default `NodeImageMode::Auto` keeps the texture's aspect inside the
  node: solid full-screen quads (a 1×1 image) came out square, the 2:1 pictures letterboxed. UI quads now use
  `Stretch` (`ui_quads.rs`; the HUD and Easydrive draw through it too: race HUD and Easydrive checked unchanged).
- **UI runtime additions** (`mw_assets::ui`): idle spins (type 8), idle fades looping over their span, triggered
  idle steps (the prompt pair's instant `OUT`), image bindings answered with a texture id (`#` = the instance's
  `Binding_Replacement_Index`), nested stacks over `{parent}`, `#parent_index#` in template bindings, scrolling
  texts' strings / data source. `mw_assets::sequence` reads movie, black quad, streamed image, general trigger and
  conductor items (kept nested with their own jump controllers and enabled bindings).
- **Timings** (release build, other agents building on the machine; `--front-end-shots`, `FRONTEND TIMING`): new
  game: front-end screens 0.17 s, car placed and area resident 4.97 s, world loaded 6.67 s, "PRESS ANY KEY" 11.4 s
  (the title sequence's own 9.3 s), key 12.5 s, playing 18.1 s (the 4.8 s transition). `--drive 911 --show-loading`:
  resident 5.3 s, loading screen off 6.9 s. Continue (McLaren at its swap spot): resident 7.3 s, ready 9.3 s. The
  full 169-unit world still takes ~40 s in the background.
- **Model decisions**: title backdrop and movies drawn full screen, movies through ffmpeg (no VP6 decoder), no title
  music / transition sounds (muted), four random pictures and eight random offline tips per run, the `ImageNReady`
  holds never wait (polarity not traced), the strip's tip not scrolled, `UIRoadRibbonShader` streaks left out,
  "ready" uses the 1.5 s timeout without the traffic check, our profile file, the pause menu's MAP page (a list of
  the offline races), inert online items, pad A / B in menus, "any key" although the node says `ANYBUTTON=False`.
- **Open**: the map screen 899991 (event selection on the map), car select / customise screens (1571371,
  1135879), the new-game intro event (1560354), event-loading cards, the streaming stall, a capture of the
  original's boot to compare the transition timing and what the screen shows between the transition and play.

### 2026-10-09: groundcover sprites face the camera (the `Groundcover` vertex program ported)
User report: since the sprite-size fix (byte / 127 × 2^e m, so the e = 6 / 8 groups are 10–60 m tree cards) the
cards showed as thin vertical slivers when seen edge-on: we drew each sprite as two crossed static quads.
- **Ported** (`groundcover.wgsl`, from `mwtool shader-asm Groundcover`, the `vs_4_0` program): per corner
  (u, v), sway = sin(Time × `speed` + p.x) × `amount`; d = normalize(p.xz − ViewPosition.xz), right = (−d.z,
  0, d.x); vertex = p + right × (sway × v + width × (u − 0.5)) + up × height × v, width / height =
  BINORMAL.xy × `GroundcoverConstants.x` (= byte / 127 × 2^e); uv = region.xy + region.zw × (u, 1 − v) with
  region = `GroundcoverTextureRegions[round(A × 255)]`; normal = normalize(sign(width) × right × (u − 0.5) +
  (0, v, 0) − 0.1 (d.x, 0, d.y)) + max(8 − height, 0.75) × (ground normal + (0, 0.25, 0)); colour sqrt(2 tint)
  (we keep 2 tint for the layered shader's sqrt-albedo path, as before).
- **The CPU builder's per-sprite step** (`FUN_00eb9500`), done in the same vertex shader: a sprite is emitted
  only if (p − eye) · forward ≥ 0 and |p − eye|² ≤ R², R = ((w >> 1 & 63) + (h >> 1 & 63)) × 2^e / 127 × 125;
  its width and height bytes are multiplied by min(5 (1 − d² / R²), 1) and truncated (`(char)(int)`) before
  the / 127. Positions are 3D distances (`param_6` = eye). The other per-sprite test, `FUN_00eb8290`, is a
  box-against-frustum-planes test (the rasteriser's clipping does that here).
- **How**: one mesh per group as before, but 4 vertices per sprite all at its base, with the sprite's size
  bytes, 2^e and corner index in a custom attribute (`materials::ATTRIBUTE_GROUNDCOVER_SPRITE`);
  `Layered::specialize` swaps in `groundcover.wgsl` as the vertex shader (forward, deferred prepass, the
  planar view's normal prepass) for meshes that carry it. The entities get explicit bounds (`NoAutoAabb`) that
  cover any turn and the sway. Wind constants come from the collection material's `amount` / `speed`
  (material value or shader default): (0.025, 0.8) on 5461 groups, (0.24, 0.8) on 4923, (0.1, 0.1) on 4845, a
  few others up to 0.35.
- **Side find**: atlas region 2 of many grass materials has scale.x = −0.5 (a mirrored cell); our old mesh
  builder treated any non-positive scale as missing and mapped the whole atlas onto those 41 k sprites. No
  sprite uses an all-zero region or an index ≥ 16. The regions are now used as the program uses them.
- **Look** (`--camera=-610,56,2310,-420,90,2390`): the slivers are gone, the cards turn with the camera
  (orbit views from the west and south-west of the park); tufts at the `grass-tufts` viewpoint unchanged.
- **Cost**: vertices per sprite 8 → 4 (two quads → one), 384 → 256 bytes. Interleaved A/B at the park view
  (`MW_BENCH_PASSES=1`, machine at load 13–24 from parallel builds, so the CPU-side frame times, 76–160 ms,
  are noise): deferred prepass GPU 2.33–2.38 ms old vs 2.33–2.43 ms new, its fragment invocations 4.35 M →
  3.82 M (−12 %, one card instead of two crossed). The frame is CPU-bound (render-world prepare and
  main-world schedule, 178 k entities); groundcover's 15 k group entities are ~14 % of the mesh entities but
  only ~19 are visible there.
- **Model decisions**: `Time` taken as seconds (Bevy's `globals.time`, wrapping at 1 h; the game's value not
  read live); the builder's caps (20000 sprites a frame, `DAT_010c6f70`, and 300 draw records) are not
  applied, so where the game would run out (tree traversal order) we draw more; the facing and range use the
  view being drawn (the planar reflection view uses its own mirrored eye; the game builds once per frame for
  the main camera); motion vectors ignore the sway.

### 2026-10-09: compound objects (street lamps, panels) break into their parts
User request: lamps should come apart as in the original. Rules and functions in decomp.md "Props".
- **Data**: `CompoundObject` +0x08 is a `Skeleton` (not a GeneSys handle); the model is skinned to it, one
  bone per part (`mwtool props -v` prints the triangles per bone: every compound model is fully skinned).
  Part record +0x60 = bone index, +0x64 / +0x65 / +0x66 = replaces / next / first replacement (0xFF = none),
  +0x68 / +0x6c = linked part indices. Behaviour entries are 0x14 bytes with the type id at **+0x0C** (the
  import at +0x00, its id again at +0x08; first read at +0x04, which gave 0 everywhere: dead end). Survey of
  all 52 compounds: no joints (+0x3c = 0 everywhere), no `EnableCompoundBehaviour` with +0x1a set, 10
  definitions (the single-arm street lamps, ~2600 placements) with a `SmashProxyBehaviour` part (the whole
  pole, 100000 N, replaced by its lower and upper halves), the 11-panel structures with pinned linked panels.
- **Common street lamp** (`fed64b97`, 934 placements): 150 kg compound, knocked at 5000 kg m/s, smashes after
  0.5 s loose above 10000 N into arm (bone 1, 20 kg), head (bone 2, 15 kg) and pole (bone 5, 120 kg); parts
  start loose, at rest (`FUN_005341a0` zeroes their velocity).
- **Runtime**: `props.rs` `smash_compounds` / `spawn_part` / `prop_world_sounds`; loader splits models by bone
  (`DecodedMesh::subset`, `Variant::bones`); `--test smash --smash-set compounds` (six compounds, camera beside
  the last knock or smash, `-smash<n>` screenshots).
- **Results** (`--test smash --smash-set compounds`, 60 km/h): all 6 smash: lamp after 1.25 s loose (12173 N),
  double lamp 1.55 s (31924 N), `e547f377` 0.90 s, `f3afb6cd` 1.23 s (101801 N, 6 parts), `1e63a9d5` 1.18 s,
  panels at once (smash force 0): 31 part bodies, no pole split (100000 N never reached; also not at
  120 km/h: 5 smashes before the end, 26 parts). Default `--test smash` unchanged apart from the lamp, which
  now smashes 1.75 s after the knock into 3 parts (speed losses 0.15 / 0.24 / 0.85 / 0.36 km/h as before).
  World drive (`--spawn=800,42,1688,62 --no-traffic --auto-throttle 0.7`): the lamp knocked at 89 km/h smashes
  1.0 s later on landing (80087 N); arm, head and pole lie on the pavement ~12 m on (screenshots looked at).
- Open: the code that compares `SmashProxyBehaviour.ForceToSmash` (getter 0x5b9800, called through a runtime
  table not found); what plays the compound's "Collision FX" effect; the world-contact sound threshold (audio
  object +0x94 at `[DAT_010edb08+4]+0xb270`); the damaged model swap (`PropObject` +8) still unported.

### 2026-10-09: traffic horns, traffic crash and skid sounds, AI racers' crash sounds
Request: "add traffic horns and traffic crash and skid sounds as the original has them". Every function and
constant in decomp.md "Traffic horns, crashes and skids"; data `mw_assets::car_sfx`, runtime `car_sfx.rs`.
- **Horns**: the "55" timer is seconds of behaviour time and starts at FLT_MAX (`snap2`: FLT_MAX on every
  unchecked car); a car stopping for an obstacle (mode 0) honks when its personality ≤ `TrafficParamConfig
  .831be501` = 0.4, then not again for 55 s. The open "TrafficConfig +0x20 / +0x24" were `TrafficParamConfig`
  (0.4, and 0.7 for the swerve / jack-knife manoeuvres' horn). Two horn slots, `horns[car index % count]` within
  60 m. Six horn timelines; four branch on `Signals.GetRandomNumber[100](LTEQ,50)` through a
  `JumpTimelineController`, now run by `sfx_player` (with the comma form). Traffic `JumpTimelineController`s are
  time triggers (the skid loop 4 → 1 s, the wreck loop 12 → 10.5 s).
- **Crashes**: the 31 `CollisionSoundSet` slots are now identified for 18 of them: (competitor | player) ×
  (light / medium / heavy) × (front / rear / side by 30°), `FUN_0057e920`; traffic car contacts always use the
  competitor light front slot (`333a28fc`). World impacts play a wave of the surface's `Collision_Sounds` pool
  `7e01f1e6`, gain 0.714..1 over 30..100 mph. Thresholds in the global `VehicleSound` (2.235 m/s, 14, 30, 0.1 s).
  Items without a mixer channel play on a mix-snapshot channel named by `Snapshot_Property` (`Mix_Main`, gains
  0.26 / 0.40 / 0.63 small / medium / big); 38 channel names added to `genesys_names.txt` from `FUN_004b5f40`.
- **Skids**: six traffic skid slots, started for the nearest physical car whose wheel 0 marks and that moves
  faster than 5 m/s, stopped with a 0.1 s release. AI racers' skids: a Drift + (Brake | Spin) state machine with
  0.15 s swaps (`FUN_0058f6d0`), switched by an unidentified vehicle field (+0x2d58); not ported.
- **Offline** (`mwtool traffic-sound sfx out.wav`, levels before placement): horns 0.95–4.94 s, −27..−31 dBFS
  RMS (branches seen: 1822f8 2.12 / 3.53 s, 1822ff 3.50 / 3.83 s); competitor impacts 2.5–3 s, light −40..−42,
  medium −35..−36, heavy −31..−34 dBFS; world wave −34.5 / −32.8 / −31.6 dBFS at 10 / 30 / 50 m/s; skid loop
  −25 dBFS, stopped at 6 s it ends at 6.51 s.
- **In game** (`MW_AUDIO_METER=1 MW_CAR_SFX_LOG=1`, voice out = rendered RMS before the effects volume):
  - free roam at 0.3 throttle (`--drive 911 --units 30 --spawn=800,42,1688,71 --auto-throttle 0.3 --bench 75`):
    the 911 rams car 25 and stays pushing it; 13–23 horn timers armed, 1 horn (46.6 m, full gain).
  - the same at full throttle (60 s, two runs): 3 horns each (35–48 m at start, out −28..−31 dBFS at 10–40 m, gain
    0 dB: the horn falloff's peak gain 5 holds full level to ~45 m), 5–6 traffic skids (pushed crashed car, 11–12 m,
    gain −2.3 dB, out −28..−44 dBFS), 1 traffic car impact (11.2 m, −1.1 dB, out −34.7 dBFS), 1–3 traffic world
    impacts (11–12 m, −1.0 dB, out −32..−34 dBFS); traffic engines at the same distances −32..−45 dBFS.
  - race (`--race 462174 --no-cutscenes --bench 90`): 11 AI car impacts, 52 AI world impacts (+2 with the set's
    slot), 3 horns, 4 traffic skids, 1 traffic world impact. AI impact gain by distance: −3 dB at 10–40 m, −10 at
    40–60, −17 at 60–100 (out −35..−39 dBFS within 40 m, −60 at 60–100); world waves −5 dB to 60 m, −13 at 60–100,
    −26 beyond 100 m; doppler 0.955..1.05. AI engines at 23–48 m: −26..−28 dBFS.
  - first race run, with the game's own car-car test (any value above 0): AI cars leaning on each other filled
    the impact voices (624 impacts skipped in 90 s); with the 2.235 m/s threshold none were skipped.
- Model decisions (decomp.md lists them): impact records → per-step contacts with the 2.235 m/s threshold and
  0.1 s spacing for every impact; closing speed as the impact value; the default surface; no scrapes; crashed
  traffic "skids" when touching the world above 5 m/s; 8 impact voices; impacts at zero gain not started; one
  placement per voice; the random draw's range.
- Open: the player's horn runtime (the player's horn list, `Player Vehicle Horn`), the player's impacts by the
  traced slot table (`sfx.rs` still sorts by loudness), the scrape branch (`FUN_00929da0`), the AI skid state
  machine's field (+0x2d58), contact surfaces for impact waves, wrecked-traffic sounds and car alarms (no damage
  bar on traffic, no parked cars), manoeuvre horns, a capture of the original with traffic to set these levels
  against the player's engine.

### 2026-10-09: traffic density is the event's (1.0 in free roam, not 4.78); car-against-car crashes ported
User reports: "scale back the traffic" (more traffic than the original) and "collisions with traffic and other cars
can result in crashes with the crash camera". Details: decomp.md "Traffic rules" (density, load budget, counts)
and "Crashes" → "Car against car".
- **The density base is set by the event**, not fixed at `TrafficSystemConfig` 0x80 (4.78): `FUN_0070caa0`
  (event set-up) posts traffic message 4 with the `OfflineEvent`'s `ce4961e7` (+0xb0) when `90f52617` (+0xf1)
  is set, else the hour array (the open "hour array reader": `FUN_0070caa0` / `FUN_006e6380`, reached through
  `AIConfig` handles); `TrafficEnabled` off posts message 0xf (no traffic). The posting wrappers were found by
  scanning `snap2`'s decrypted `.text` for the queue pointer `DAT_010edb30` (the decompiler hides the calls).
  Free drive's events all hold 1.0; the snapshots read base 1.0 at `world+0xa2b74` (the 4.78 sits at +0xa2b70,
  the reset copy). The port used 4.78: 4.78× too dense, the 256-car cap reached at once.
- **The load budget counts only crashing traffic** (the AND with the crash-record mask `+0x1afe8`); the
  measuring point `world+0x36110` is the player's car (snapshot, within 0.3 m).
- **Counts** (live lane cars around the player, snapshot vehicle table vs ours standing at the same spot after
  25 s): snap1 (1814, 1649) original 96 total, 12 / 27 / 34 / 56 / 86 within 100 / 200 / 250 / 400 / 600 m; ours
  now 103, 10 / 28 / 37 / 61 / 87 (before: 255, 17 / 71 / 86 / 132 / 216). snap2 (925, −101) original 108,
  1 / 5 / 9 / 31 / 75; ours now 126–135, 3–5 / 8–10 / 10–14 / 30–38 / 85–90 (ours stands: ×1.2 slow-speed
  ramp against the original's ×1.11 at 38 mph); before 256 (cap), 14–23 / 34–37 / 54 / 128–135 / 211–216.
- **Car against car**: the game's handlers ported (impact zone, the two `ImpactDamageGraphs` curves, the
  `Vehicle_Gameplay.DamageStats` strength factors, the race-car share split, the glancing cap, landing on a car,
  the rule set per pair). Checks (`--test crash-traffic`, `--test crash-ai --ai 911 --no-traffic`, spawn
  925,23,-101,10): head-on into a traffic car at 63 mph against its 18 (closing 80.7 mph, front zones): player
  damage 20.1 > `TrafficRules` 20 → crash camera, slow motion, Freeway / car shots, reset rolling at 44 mph; the
  traffic car takes 50.3 > its `PlayerRules` 50 → wrecked. T-bone of a nearly stationary AI 911 at 70 mph:
  AI damage 73.7 > `PlayerRules` 50 → AI crash, reset 4.5 s later rolling at 30 mph; the player takes 0.2
  (the struck car's weight is its speed). An 81 mph closing head-on is right at the player's threshold: hits on
  traffic below about 80 mph closing do not crash the player, as in the game's numbers.
- **Dead ends / open**: the writer of the per-pair gate (vehicle +0x1640 + 4·i); vehicle +0x1630 (landing
  rule); `FUN_0092e7b0`'s inputs (+0x1bf0…+0x1c20); the per-location table +0x16a0; the damage bar
  (`FUN_009b2530`); how AI racers reset after a crash in the game (`FUN_006d39d0` skips players with kind +0x9c ≠
  0; ours uses the crash camera's length); the parked-car slots (60–77 live in the snapshots) are still not ported.

### 2026-10-09: the "flashbang": the impact shake's noise spring diverged at low frame rates
User report: "sometimes it randomly seems to flashbang" (the screen goes white for a moment while driving).
- **Detector** (`flash_probe.rs` / `flash_probe.wgsl`, debug only): `MW_FLASH_LOG=1|<file>` reduces each
  tonemapped frame onto a 32 × 32 grid after the tonemap (mean displayed luma, non-finite HDR pixels,
  brightest HDR value, adapted exposure), reads it back every frame, logs one line per frame (with the camera
  pose, timeline, hour, exposure range) and warns `FLASH` when the mean luma jumps over the last second's
  median; `MW_FLASH_SHOTS=<dir>` saves a screenshot of each.
- **Reproduced**: in `--race 462174 --test race` the whole frame went to a flat displayed luma 0.91 (all
  1024 cells, HDR 1.877 everywhere: the fog colour) for long stretches, with the camera's **rotation NaN** and
  its position finite. No non-finite HDR pixel, no exposure jump (pinned at the keyframe minimum all along),
  no bloom involvement. A check in `follow_car` showed the impact shake's spring state (`ImpactShakeState`
  x / v) at ±1e37…inf / NaN with the shake level 0: 0 × inf = NaN angles.
- **Cause**: the noise stage of `FUN_005dfe90` (decomp.md "Camera effects") was stepped with the frame time:
  `v += kick + (−x − v)·1; x += v·30·dt`, i.e. `x ← (1 − 30·dt)·x + 30·dt·kick`, which grows geometrically when a
  frame takes longer than 1/15 s. In an ordinary 120 s `--drive 911 --units 30 --auto-throttle 1 --nitrous` run
  (normal speed, frames of 50–110 ms while smoke filled the screen) x reached 1e30. While the level is 0
  this is invisible; after any contact the level stays above 0 for 10–20 s (decay 0.835 per update until it
  underflows), and the view was then turned by 2 × level × x degrees: an arbitrary orientation each frame, and
  NaN (white) once x overflowed, until the camera state was reset. The 4× test runs hit it within seconds.
- **The original** steps it once per 1/60 s tick: on `ref/live1` h = 0.5 on every clean update, also right
  after a 4.7 s stall (decomp.md "The noise stage's time step"). **Fixed** (`camera.rs` `ImpactShakeState`): the
  noise runs in 1/60 s updates (at most 8 per frame, the rest dropped, as the original drops time behind its
  catch-up step); the level's decay was already per-update exact.
- **Checked after**: the race test and the 120 s nitrous drive keep a finite camera with no full-white frames
  (probe log: 0 frames with a NaN camera, before 93 of 1052 in the same race test); `--test crash` still shakes (level 4, 2.5° RMS pitch
  at level > 1, the recording had 3.6 / 3.0 / 3.2° at mean level 2.5).
- **Not the cause, seen on the way**: (1) a standing burnout (car pushed against a wall at full throttle) fills
  the whole view with tyre smoke at displayed luma ~0.9 (`FLASH` lines with up to 1022 / 1024 cells white,
  HDR only 4–5 units, exposure at its −0.2 minimum): plausible for white smoke in noon sun, but not compared
  with a capture of the original (open). (2) The eye's vertical spring (`HeightSpring`) is also stepped with
  the frame time and is unstable above ~0.2 s frames, but its limits clamp it (bounded bob, no NaN); not
  changed.

### 2026-10-09: surfaces: what grass and dirt do, the grip fade, the drift factor and the bumps
User report: "surfaces don't change the grip yet, grass and dirt drive like normal road."
- **Measured first** (`MW_TYRE_DEBUG=1`, park grass at 1117, 56 and −751, 2147, found with the new
  `mwtool surfaces --near x,z[,r] [--find <index>]`): the surface **was** resolved per wheel (tag → list index →
  `SurfacePhysics` 283005 → 911 link grip 1.3 → lateral 0.835 / longitudinal 0.88, rolling 0.03 × N, slide drag
  2.25 × mph × 1…12 over 0–20° of slip). No step lost the surface; the triangle lookup (`surface_at` re-casts
  against the trimesh, whose triangle order `TrimeshFlags::FIX_INTERNAL_EDGES` keeps) and the tag decode are
  right. On the flat track (new `--track-surface <index>`) the steady state already differed before this work:
  911 0–200 km/h 11.1 s / 14.0 s / 13.7 s and top 307 / 258 / 260 km/h on asphalt / grass / dirt, skidpad 2.54 /
  1.98 / 2.14 g. So the grip numbers were in; what was missing is what makes grass *feel* different.
- **Read in full** (`FUN_009d8bd0` and everything it calls; decomp.md "Tyres"): (1) **bumps**: the player's car
  rides on Perlin noise of the contact position, amplitude `Surface_roughness.7cb58933` mm, frequency
  `8193fd9b` per metre (grass ±8 cm over ~10 m, dirt ±2 cm over 2 m, paving ±1.2 cm over 3 m, asphalt 0.1 mm),
  normal rebuilt from samples 3 cm away; the game's noise is exactly Ken Perlin's 2002 improved noise (its
  512-entry table at 0x010cfcb0 is the reference permutation twice). (2) **grip fade**: leaving a surface its
  grip fades towards the new link grip over (old link grip − lower old grip) × 3 × min(time on it, 2 s) × 0.5 and
  the lower applies (grass → asphalt: 1.4 s). (3) the **drift's surface factor** (`FUN_009cede0`, assumed 1
  before) is the mean effective lateral grip of the four wheels: the drift's yaw torque and side force shrink on
  grass. (4) the tyre class (`RoadOverrides` = class 2 at car creation) and the `TyreSpecificOverrides` names
  `OffRoadOverrides`, `SportOverrides`, `TrackOverrides` (cracked; `08b66463` not). `Surface_speed` is not read by
  the car code; the wet block names the same physics on the main surfaces.
- **Ported** (`mw_handling::surface`: `SurfaceGrip`, `drift_factor`, `roughness`, `perlin`; `drive.rs`;
  `DriftInput::surface`). Unit tests (run standalone with `rustc --test`): Perlin gives the reference 0.13692 at
  (3.14, 42, 7), the fade takes 1.395 s.
- **After** (911, flat track): the steady state hardly moves (the bumps cost grass 0.14 s to 200 km/h and 1.5 km/h
  of top speed, dirt 0.17 s and 2.5 km/h; skidpad within 0.01 g); the grass run's car height now moves ±3 cm and the front / rear compression ±2–3 cm over the run (before: constant to
  1 mm); the scripted drift on grass no longer spins (peak 67.9° → 28.7°, exit 11 → 64 km/h), on dirt 45.8° →
  30.3°, asphalt unchanged (36.5°). In the world (park, grass → paving): the rears' grip read 0.83 / 0.89 on the
  paving and climbed back to 1 over ~1.4 s.
- Open: whether the `TyreUpgrade` mods set the tyre class; the roughness flag's player record +0x9c (read it live
  to confirm "human player"); `FUN_009d5270`, the other reader of the front wheels' lateral grip; the wet block's
  condition.

### 2026-10-09: cars get dirty and scratched (dirt / dust build-up, damage scratches)
User report: "the cars also don't get scratches and dirt on them". Ported from the code, the data and the paint
shader; rules in decomp.md "Vehicle paint, liveries and dirt", runtime `crates/mostrusted/src/paint_wear.rs`.
- **Dead end first**: the 2026-10-06 lead (`FUN_008e1280`'s callers) is a view-constant block, not the car's
  record. The records come from `FUN_00583290`, which reads the dust / dirt from render instance words 0x6f6 /
  0x6f4; `FUN_00591ac0` copies them from vehicle +0x2c80 / +0x2c84 each frame. No function in the dump writes
  those: scanning the decrypted `.text` of a memory snapshot (`re/out/live/snap1`) for the 32-bit displacement
  0x2c80 found three `lea ecx, [reg+0x2c80]` before thiscalls, i.e. a sub-object whose methods use `this`:
  `FUN_009d1980` (reset), `FUN_009ce7b0` (spawn: start levels from the instance), `FUN_009d6e30` → **`FUN_009d19c0`
  (the build-up)**, earlier mislabelled "surface rumble". Useful technique for other "no writer" offsets
  (vehicle +0x2c70, the steering offset, sits in the same block). Scanner kept in the session scratchpad
  (`dispscan.py`: objdump at each hit, function from `functions.csv`).
- **Build-up**: per wheel on the ground, from the surface's `SurfaceVehicleEffects` (new type: `Dirt` / `Dust`
  = (rate, time), speed ramp 30 → 120 mph): dirt += max(skid values, ramp) × dt / max(time, dt) × rate / 4, dust
  += ramp × dt / max(time, dt) × rate / 4, clamped 0..1. Grass / dirt: 0.06 / s each at full speed; dry asphalt
  only dust (0.01 / s at 120 mph); two water surfaces wash (−0.5). The repair message (0x12) zeroes them.
- **Dust and dirt were swapped**: the snapshot's paint block holds `DustColour` at +0x60 (→ `GenericVector4_B`,
  the e.w² mask: light streaks low on the body) and `DirtColour` at +0x70 (→ `_C`, the e.y² × e(uv × 7).x
  splatter). The record scales dust × 2 and dirt × 3 before the colours' w.
- **Scratches**: the paint's vertex program samples a 3 × 5 grid (`gavDamageVolumeValues`) over the car's
  bounding box at the bind-pose x / z; `FUN_00585030` fills it from the eight zone levels (vehicle +0x112c…,
  ring order front-left … left) × `VehicleInfo.76bfd668` (10; read through the snapshot) capped at 1. The pixel
  program's scratch term (effects map z bands) lerps to `PbrScratchColour`, `PbrScratchRoughness` and
  `0802d981` (scratch F0, matched in the snapshot). A 10 % zone hit scratches that end of the car fully.
- **Built**: `CarGrime` (dust, dirt) per `Car` (player and AI) built up in `FixedPostUpdate`; each car gets its
  own copies of its paint materials, updated when the levels or the grid move by 1/512; the paint meshes carry
  the bind-pose x / z in COLOR.zw (AO stays in xy); lightmapped paints (`_Lightmap`, 31 cars) read the effects
  map from UV_0, so they get dirt and scratches too (`_Livery_Lightmap` still not: three UV sets). `--dirt`
  now sets the starting levels; R (reset) repairs and washes. `mwtool surfaces` prints each surface's build-up and
  `--find <index>` lists flat places with that surface; `--track-surface <index>` (merged in place of `MW_TEST_SURFACE`) makes the test track that
  surface; `MW_DIRT_LOG=1` logs the player's levels.
- **Checks** (session scratchpad `pw/`): `static_compare.png` (911 `--dirt 1,0` splatter, `0,1` dust streaks,
  silver `0.6,0.6`), `m3_compare.png` (BMW M3, lightmapped paint, clean / dirty), `track_dirt_1500.png` vs
  `track_clean_1500.png` (`--track-surface 7 --test-track --auto-throttle 1`: dust 0.60, dirt 0.62 after ~25 s;
  rear bumper and sills covered), `crash_compare.png` / `m3crash_compare.png` (`--test crash`, front 44 % /
  26 %: scratch bands over the front wings and bumper fading onto the doors). `--test drift` on dirt: dirt
  0.34 after the 6 s drift.
- **Model decisions**: (1) the per-wheel skid values that feed the dirt rate (`FUN_009cef90`, smoothed, three
  per wheel) are stood in for by the skid sound's lateral / spin ramps per wheel, unsmoothed; (2) the dry weather
  block is always used (the game takes the wet one when the surface manager's state is ≥ 1, not identified;
  grip and sounds also use dry here); (3) the grid box is the body meshes' vertex box (the game's: the model's
  mesh boxes); (4) the grid is sampled per fragment from interpolated x / z (the game per vertex); (5) our zone
  levels (the damage clips' positions) stand for the vehicle's zone values, and the 0.99 cap is applied always
  (no wrecked state here); (6) traffic cars stay clean (their own spawn path; whether the game builds up their
  dirt is not checked); (7) the clear coat keeps our merged form, now × (1 − s) as well.
- **Open**: the crumple normal map / AO (TEXCOORD1, needs a third UV path), rain drops on the paint
  (`GenericSamplerB`, `A.zw`), the wet block's selector, `FUN_009cef90`, gas stations repairing (the handler is
  reached through a vtable), and a capture of the original's dirty car for colour (ours reads blue-grey under
  the test track's sky).

### 2026-10-09: prop collisions on by default: the game's car-against-object response and break-free rule
Traced in the dump (details and addresses in decomp.md "Props"); runtime in `props.rs`, on by default since
this entry (`--no-prop-physics` opts out; `--prop-physics` is gone).
- **Why the car wedged**: ours solved a knocked lamp like a wall. The game solves a race car's contact with a
  dynamic object like any contact but adds only a share of the impulse to each body: `FUN_00960f20` picks the
  pair handler by category (`FUN_00929350`: 0 player, 1 AI, 2 crashing race car, 3 traffic, 4 anything
  else), `FUN_00941170` (player vs object) fills a response record from
  `CollisionResponses_RaceCar.RaceCarVsDynamic` (`c7d59471`, +0x40c), `FUN_009bb640` copies it into the
  contact constraint, and the contact passes (`FUN_009be420`, friction `FUN_009bed70`) add `impulse × scale` to
  each body's linear / angular accumulator (+0x90 / +0x94 / +0x98 / +0x9c). Data: the player's car takes
  **0.1** of the linear and **0.05** of the angular impulse, the object 1 and 0.4 (fields `e75eb28b`, `207258c8`,
  `357a73bd`, `1caa1e4f`), restitution 0; AI racers 0.01 / 0.01 (`2e3c3d30`); traffic 1 / 0.55
  (`CollisionResponses_Global.TrafficVsDynamic`); a crashing race car gets the default record (all 1). The
  contact friction is the object's rigid-body `Friction` (record +0x00, read from the definition's main body).
  Bodies with flag +0xf4 bit 2 (pinned) take no impulse at all.
- **Break-free** (sub-agent trace, confirmed from the disassembly): `67f0233c` is a **momentum** threshold:
  `FUN_0054e570` unpins when the hitter's `|mass × velocity|` (its whole velocity, body +0x70.w × +0xb0)
  exceeds it, run from the bounding-box overlap pass `FUN_00554d80` before the contact solve. `5586d09d` = may
  come loose, `94c94bab` = starts pinned (true exactly where the threshold is above 0; cones start loose).
  A 1.5 t car knocks a street lamp (5000) over above 3.3 m/s, a 750 sign above 0.5 m/s. The earlier reading
  (impulse along the normal) is replaced.
- **Reset**: `FUN_0055ae50` every frame, no timer: a moving prop goes back (or is removed) once both the camera
  and the player's car are beyond `mfResetDistanceSquared`. Moving props live in a 64-slot pool (the farthest
  from the camera is reset when full); physics objects in a 48-slot pool, evicted by camera distance and
  frustum.
- **Effects**: `ForceToDamage` (+0x18) is a contact force (N, impulse / dt): above it, once per knocked
  lifetime, the prop's damaged state is set and, for a vehicle hitter, its impact effect (`PropObject` +0x14,
  a `BearEffect`) starts (`FUN_0054bf70`). `PropObject` +0x1c is a `WCPlaySoundBehaviour` with
  `Collision_Waves`; the world-contact sound waits 0.35 s between plays (`FUN_0054c680`). Compounds smash when
  the contact force passes `ForceToSmash` (+0x30) after `08c87eeb` (+0x34, seconds) of being loose
  (`FUN_00555c90`); not ported (no part models).
- **Runtime**: knocked props are `GameLayer::LooseProp`, outside the race cars' avian layers; `prop_contacts`
  (before each step) tests standing props against the car's momentum and solves the car–prop contacts itself
  with the scales above (two passes, as the game's two sub-steps), accumulated and clamped impulses, friction
  within μ × normal. A standing prop that holds stays in avian's solver and stops the car. Props get a
  collider within 30 m (+ their radius) of a race car or crashed (dynamic) traffic, kept to 45 m (grid of
  32 m cells); lane-following traffic does not activate props (it never knocks them loose). Prop
  impact sounds play (player only), impact effects start on `ForceToDamage` (props only; compounds carry no
  effect handle).
- **Results** (`--test smash`, 911 at a held speed; car speed lost to each prop's contacts, which includes
  pushing it along afterwards): 60 km/h: cone 0.15 km/h, sign (threshold 500) 0.24, lamp 0.93, fence
  1.55 (pushed into the wall at the end); 120 km/h: 0.10 / 0.36 / 1.13 / 0.49; at 10 km/h the lamp holds (car
  momentum 4.2k < 5000) and the car stops against it; largest one-step loss 0.57–0.77 %. The rules predict for
  a single inelastic exchange at 60 km/h 0.02 (cone), 0.2 (sign, fence), 0.55 km/h (lamp). World drive
  (`--spawn=800,42,1688,62 --no-traffic`, 0.7 throttle): a lamp knocked at 89 km/h costs 0.7 km/h over two
  steps and flies off spinning (screenshot looked at); the speed trace matches `--no-prop-physics` within
  1 km/h; the stop at x 990 is a world wall in both runs, and the suggested heading 71 stops behind a traffic
  van in both.
- **Performance** (`--bench 10` at the CLAUDE.md spot, shared machine at load 7–14, so noisy): props with a
  collider 5 (master's 120 m streaming around the player: 69; 30 m around every traffic car too: ~180). In
  the quieter rounds the frame median was 22.1–22.9 ms with props against 21.0–21.5 ms with
  `--no-prop-physics` (main-world schedule +0.2–0.6 ms); master's `--prop-physics` 21.6 ms against this
  branch's 23.1 ms in one pair, the other pairs within noise either way.
- **Model decisions**: the impulse is computed with the unscaled effective mass (the contact prestep that
  writes the point's effective mass, point +0x2c, was not read); overlap push-out 0.2 of the depth per step,
  capped at 4 m/s, 1 cm slop (the game's targets, point +0x30.., not traced); speculative contacts for gaps
  up to one step's travel; props that start loose stand static until the first touch (the game creates them
  asleep: same behaviour); objects without `AdditionalInfo` never come loose; the activation radius (the game
  activates on bounding-box overlap with an awake body); prop sound volume ∝ closing speed / 12 m/s and a
  2 m/s threshold for car contacts (the game's threshold is in an audio object, not read); traffic and loose
  props hitting standing props still break them through the contact hook (closing contact, not box overlap).
- Open: compound parts and smash; the damaged model swap (`PropObject` +8, the same model in every instance
  seen); the world-contact sound of falling props and "Player_Vehicle_Prop_Collision" (only looked up by
  name); the prop pools' limits; knocked props that come to rest keep their body (the game drops it and keeps
  the static proxy where it lies); `vehicle+0x1777` switches the player's scales to 0.4 / 0.01 (not ported).
- Dead ends: name guesses for `67f0233c`, `5586d09d`, `94c94bab`, `08c87eeb` (about 45 words); the prop manager
  in the snapshots (its global is unknown).

### 2026-10-09: traffic and AI racers make sound: competitor engines, the traffic engine pool, 3D falloff and doppler
User report: "traffic sounds are also missing" (only the player's car had engine / tyre sounds). Details and
every function in decomp.md "Other cars' engine sounds".
- **Data**: each car has `<id>_VEHICLESOUND` (the player's) and `<id>_COMPETITORSOUND` `VehicleSound` resources
  (type 0x207, id = CRC-32 of the name, checked: `1085007_competitorsound` = `1d6e64d2`, `122816_…` = `090c3b09`);
  +0x48 is the `EngineSound2`, +0x60 / +0x64 the horn list. AI racers use the competitor bank (one Ginsu set, a
  listener-distance mixer). Traffic loads `<component>_COMPETITORSOUND` from `TRAFFICATTRIBS.BNDL`: four distinct
  two-sequence banks (wave loops on `Traffic.EngineSounds[#].RPM`, road / tyre layers on `NormalisedSpeed`).
- **The traffic RPM is in the sound data**: the `NormalisedSpeed` sequence's `TrafficEffectSequenceItem` maps the
  speed onto the RPM sequence's own axis through a curve with gear-change dips (`mwtool traffic-sound all`:
  `…18231b` 2000 2363 2670 3249 3829 3539 4143 3947 4275 4525 4631 rpm at 0, 0.1 .. 1). Inferred: the four banks'
  output ranges equal their RPM axes, and the live instances in `snap2` hold 2081 / 2961 rpm where the wheel-spin
  fallback of the getter gives 2726.
- **The pool** (`FUN_0060bc20`, all constants read from `.rdata` or `snap2`): five instances, re-chosen every 0.3 s
  among the traffic cars within 85 m of the player's car by `min(v²/400, 1)·0.4 + (0.5 + 0.1 in view)·(1 −
  min(d²/85², 1))`; beyond 85 m an instance fades out over 1.5 s; when more top-five cars find no slot than
  instances are fading, the oldest is released. `snap2` had two playing, one releasing.
- **3D maths** (`FUN_00ab9690`, `FUN_00ab7260`; `pow` = `FUN_00ed9e1b` from the disassembly): gain = clamp(emitter
  directivity × listener focus × `SoundDistanceFalloff` curve, 0, 1), doppler = |c / ((v_e − v_l)·u + c)| with c
  = 680 / 340 / 170 m/s by the channel's mode. Traffic engines: −1.8 dB at 10 m, −5.0 at 20, −16.4 at 50, −35.1 at
  85 m (`mwtool traffic-sound`); competitor engines: full to ~39 m (peak gain 2, clamped), −16 dB at 100 m, silent
  at 250 m. Voice pools: 10 / 32 / 6 / 16 voices (`Voice3dShared`, `Voice3dMonoShared`, `Voice3dDefragShared`,
  `VoiceMonoShared`).
- **Measured** (`MW_AUDIO_METER=1`, summarised by distance band; out = the voice's rendered RMS before the engine
  volume):
  - pass-by render (`mwtool traffic-sound 122819 out.wav --passby 40`, listener 4 m from the lane): −48.5 dB at
    100 m, −16.7 at 46 m, −3.4 at 11 m, −0.2 at 4.3 m; doppler 1.055 approaching and 0.950 receding (= 340 / (340
    ∓ 17.9)); the balance swings from left to right through the pass (L −30.3 / R −39.3 dBFS 0.25 s before the
    closest point, L −41.0 / R −29.9 after).
  - free roam (`--drive 911 --units 30 --spawn=800,42,1688,71 --auto-throttle 0.3`, 60 s): five traffic voices,
    40 started and 32 taken over in 59 s; gain median −2.5 dB under 10 m, −4.9 (10–20 m), −8.6 (20–40), −17.7
    (40–60), −27.6 (60–85), −51.4 (85–120, fading); out −33 dBFS under 10 m to −53 at 60–85 m; doppler 0.95..1.08.
  - race (`--race 462174 --no-cutscenes`, the player held on the grid, 148 s): 8 AI engines + 8 AI tyre voices + 5
    traffic; AI engine gain 0 dB to 40 m, −1.8 (40–60), −6.9 (60–85), −16.0 (85–120), −29.4 (120–180), −84 (180–250)
    and inaudible beyond; out −28 to −34 dBFS within 60 m (−30 at idle on the grid); AI skids (spin 0.9 at the
    start) −32..−35 dBFS within 60 m (gain −2 to −6 dB: their channel's falloff, min 4 m, max 280 m, curve 3.5);
    doppler 0.89..1.17.
  - cost: audio thread per voice (median, of one core) AI engine 1.9 %, AI tyres 0.36 %, traffic engine 0.9..1.3 %;
    the race's 21 voices ≈ 25 % of a core, voices beyond their falloff's range are not rendered. Game thread 65 µs
    per frame with 8 AI + 5 traffic, 17 µs with traffic only.
  - the player's engine is unchanged: `mwtool engine-sound 911` renders a byte-identical file before and after.
- **Levels**: the competitor banks' raw mix is 24..29 dB below full scale at full throttle (911 −26.3, XKR −26.0,
  Veyron −24.8, Aventador −24.1, 1085698 −28.7, 122682 −24.8 dBFS), close to the traffic banks (−22..−37 over a
  speed sweep), while the player's banks are −2..−11 and are normalised to −10.5. So the other cars play at the
  data's level (no normalisation, no skid mix gains): AI engines sit ~18 dB under the player's at full throttle,
  traffic a further ~10–17 dB down. Model relation: the game's bus levels are outside the bundles.
- **Model decisions** (decomp.md lists them): listener = camera; one placement per voice (first channel of the
  bank's list); our linear-balance pan; no distance filter / occlusion (3D slots 3–6); `Random` = the lane car's
  personality; `IsWetSurface` = bad-weather keyframes; RPM fallback = idle; linear 1.5 s release ramp; virtual
  voices at zero gain; one engine + one tyre voice per AI car (no competitor limit found); AI gear-change clock =
  the player's (the competitor banks bind `LocalPlayer.GetPlayerNormalisedGearChangeTimeElapsed`); other cars at
  the data level.
- **Open**: horns (`FUN_006042c0`: two slots, `horns[car % count]` within 60 m; the trigger's timer at car +0x48
  against 55 and `TrafficConfig` +0x20 / +0x24 not settled; our player lacks `JumpTimelineController` and the
  `(LTEQ,50)` comma form the horn timelines use), traffic collision and skid sounds, `PassbySequenceBehaviour`
  "TrafficPassby" (no reader found), occlusion (+0x44's writer), the 3D slots 3–6 consumers, the panner
  (`FUN_00ab5a60` azimuth), the competitor skids' start code, AI impacts, and a capture of the original with
  traffic and racers passing to set the levels against the player's engine.

### 2026-10-09: engine sound "stuck in gear": the audible RPM range is the bank's Ginsu span; load, shift clock, effects
User report after a race: "the sound in general is pretty fucked but mostly the engine still sounds as if it is
stuck in gear". The physics shifts (911 `--test accel`: 7800 → 5100 rpm on 1→2); the sound did not follow.
- **Cause**: `GetPlayerAudibleRPM` was `1000 + 7000 × normalised RPM` (the `RaceCarEngineSound` constructor's
  range, "never overwritten" in the 2026-10-04 reading). `FUN_00575340`, run when the sound starts, overwrites it
  with the lowest `StartTime` and highest `EndTime` of the Ginsu items in `Sequences` (decomp.md "Engine sound").
  Over the 73 engine banks that span runs 1000..2770 at the bottom (median 1720) and 4400..10000 at the top
  (median 6400), so with the fixed range the sequences clamped the top of every gear to one grain: the top 19 %
  of the rev range on the median car (up to 45 %, 34 cars over 20 %; estimated with `a` = 0.8), plus the
  bottom on the idle loop alone. In a race (race gearbox, shifts near `Max_RPM`) that is where the engine spends
  its time, so the pitch sat still through most of each gear and barely dropped on the shift.
- **Evidence** (`ref/xkr_launch`, the original's own audio; pitch tracked as the RPM whose cycle harmonics stand
  out, 50 ms hops; the audio lags the telemetry by about 0.25 s): the recorded pitch follows `1750 + 4250 × n` of
  the telemetry rpm (4764 → 4196 tracked against 4220 predicted, 5101 → 4550 / 4511, 5776 → 5215 / 5108, 6427 →
  5826 / 5700); `1000 + 7000 × n` would pin everything above ~5000 rpm. Replaying its inputs (`--replay`) and
  rendering our sound from the run: before, the tracked pitch sat at the 5424 top for 71 % of the full-throttle
  time and the upshifts dropped it 7 and 251 cents; now 45 % and 354 / 539 cents, and after the 1→2 shift ours
  falls to 4196 at +0.83 s against the original's 4196 at +0.85 s. The 911 (`--test accel`): pinned 23 % → 3 %
  (its span 1790..7500 is close to the old range, so it was the lesser case).
- **Also from the code** (all in decomp.md "Engine sound", ported in `mw_assets::engine_driver`): the
  normalised RPM ignores the bottom 5 % of the range (`.bss` 0x01186680, read live); the engine load follows the
  engine's lagged throttle (+0x1d14, which an upshift cuts to 0), not the pedal, with the speed term scaled by
  the clutch and a rate of 60 × `99d4b91e` per second (0.5 per tick, `.bss` 0x011865e0 read live), and no peak
  hold; the gear-change sequences start when the shift starts (the clutch opening) and run on time since the
  shift started / its shift time (`FUN_00911650`), not on a gear-index change with an assumed 0.25 s; every
  variant plays round robin and the cruising gearbox has its own lists; their `RaceCarEffectSequenceItem`s post
  back to the engine model (`FUN_008f5490`): an RPM kick at engagement, an LFO shunt, a power cut of the load and
  a load floor on downshifts; the rev limiter plays only in the last two gears above 0.99 (ours fired whenever
  the rpm touched the redline, so at most upshifts); the engine and tyre sounds read the player car's control
  block, and AI cars no longer overwrite the Bear effects' engine load.
- **Load through the 2→3 shift** (original's +0x1f20 per tick from the declutch tick / ours on the replay):
  0.99 / 1.00, 0.40 / 0.53, 0.11 / 0.28, 0.02 / 0.15, 0.010 / 0.08, 0.005 / 0.04, then with the clutch coming in
  0.17 / 0.32, 0.34 / 0.45, 0.51 / 0.60, 0.52 / 0.75, and about 0.95 in both ten ticks after the clutch-in; the original's load rises
  past 1 (1.04) after the shift, which is the shunt's ×1.2 multiplier. Ours leads by one tick (the game computes
  the load before the engine's tick). Before, the sound used the pedal: full load through the declutch and neutral
  phases, cut only after the gear index changed.
- **Level through a shift** (911, mean of 5 upshifts, 20 ms RMS relative to the 100 ms before): before −0.1..−0.4
  dB for 140 ms, then −3 dB at 200 ms; now +2.5 dB at 20–60 ms (the clunk wave, gain 1.3) and −4.8 dB at 100 ms
  (the power cut). The original XKR dips 3–5 dB for about 0.2 s (its recording).
- **Mix** (defaults, `settings.rs` bases × volumes; model values, the game's buses are outside the data): engine
  full-throttle run −12.4 dBFS RMS, peak −4.5; vehicle sounds (`mwtool vehicle-sound`) −19.7 / −5.6; music −28.2
  / −13.6. Nothing clips on its own; the engine source peaks at 0.996 (its tanh limiter, 0.003 % of samples), and
  engine, impact and music peaks together can pass full scale (no master limiter). No traffic or AI sound reaches
  the player's banks (impacts are filtered to the player car; only the player has an engine sound).
- **Model decisions**: the `LfoSequenceItem` properties are read as 0 = rate (Hz) and 1 = depth of `Lfo`, or of
  `ShuntLfo` when the sequence's effects read it (the writer is not in the dump), depth 0 when none is active,
  property 2 ignored; effect items bound to values we do not supply (suspension load +0x1f28, traction load
  `FUN_009ab130`, pitch) are left out; effects reach the next update (frame rate, the game's next tick); the rev
  limiter restarts once per cycle at `6e6be213`..`9529a29e` cycles per second, without its RPM / load dip (the
  LFO depth +0xd8 has no writer found); `RevLimitGain` = 1 while limiting; the round-robin index starts at 0;
  traces without the shift clock assume 0.15 s per shift; the engine-damage factor of `FUN_009ae930` is 1.
- **Open**: whether a sequence clamps its binding at `Binding_Max` or its items drop out above `EndTime` (the
  XKR's first Ginsu set ends at 5424, the span at 6000: ours holds set A's top grain above ~6100 rpm and the
  tracker still finds 45 % pinned, while the original's pitch keeps rising to ~5800; the position comes through
  binding controllers' virtual calls, `FUN_00a42140`); property 5 (+0x1e94); the engine's throttle override
  list; the Burnout / Drift / In_air / Reversing lists (their RPM offsets and load multipliers); a capture of the
  original with +0x1f10, +0x1f20 and the shift clock (+0x1b20 / +0x1b24) in the watch to check the effects tick
  by tick. `MW_SOUND_TRACE=<file>` (game and `mwtool engine-sound`) logs load, normalised / audible RPM, the shift
  clock and the effects; `MW_ENGINE_TRACE` now carries the drivetrain columns the sound reads; `MW_AUDIO_NO_FX=1`
  drops the effect items.

### 2026-10-09: race checkpoint markers found (points of interest), minimap turned the wrong way
The user's reports from a race: "checkpoints aren't visible", "the minimap is rotated the wrong way around".
- **Where the gate visual is**: the ids the race code picks (`NormalPoi` 1013519, finish 1013510, ...) are the
  **`9a5a94d3` key** of `PointOfInterestVisuals` objects, not their UniqueIds (dead end of the first search:
  matching `655d3f5f`). The objects sit in the UI screens (393 instances over the HUD screens; the race HUD
  604153's `Widget_PointOfInterest` lists 32). Each has a map icon and **world layouts**: the checkpoint marker is
  a UI panel drawn in the gate's plane, not a model or effect. Formats and the four gate looks: formats.md
  "Points of interest and the minimap".
- **The code**: `FUN_00759860` marks the target gate and the one after it (`FUN_007545b0` look keys, the finish
  from `MapPoiConfig`); `FUN_00749930` the matrix (gate rotation, box bottom); the bindings `Transform` / `Pos` /
  `Rot` / `Scale` / `DistanceToCamera` / `MapOrigin` / `MapRot` / `TileTex` / `TileSize` were located by their
  name hashes (static initialisers at 0xf699b0..0xf69bf0 store `hash("Pos")` etc. in `.bss` 0x0117fxxx, found by
  scanning the memory snapshot for the string addresses) → evaluators `FUN_00805870` (world layouts) and
  `FUN_0080ba00` (map tiles). `UIDepthTestedShader` samples the deferred depth itself: world layouts are drawn in
  the UI pass after post-processing. Ported in `gate_marker.rs` + `world_ui.rs` (a pass after the game tonemap,
  depth-tested against the main depth buffer); decisions in decomp.md "Races" → "Port and model decisions".
- **Looks**: target = a thin white pole with "CHECK" and the race time on glow strips at its top, 0.1 m per
  layout pixel × 0.5…8 with camera distance 20…500 m (it stays readable far away), fading in from 20 to 60 m;
  next gate = a faint bar and "CHECK" that fades out within 10 m; finish = "FINISH". On gate 0 of 462174 the box
  bottom (the marker's origin) is 4 m under the road: the data's, kept.
- **Minimap cause**: `GetPlayerDirection` (`FUN_006be9b0`) reads the vehicle matrix's row 2 = local **+Z**; the
  HUD rendered the map with Bevy's `forward()` (local −Z), so the map showed the road behind the car at the top
  and turned the wrong way. Fixed (map and the binding). Checked the other suspect, mirroring: the stitched
  `UIMapGrid` tiles overlay the traffic lanes exactly with column = +x, row = +z, and the heading 0 / 90
  screenshots match the mosaic rotated 180° / 90° (screen right = (−f.z, f.x), as the 3-D view).
- **Minimap icons added** from the same looks: player arrow (1013507), target / next gate bars, finish, the other
  racers' arrows (1013521). Icon turn, size and off-map handling are model decisions (decomp.md "Minimap
  orientation and icons").
- Open: a capture of the original at a gate (marker size and the text side; the 3D UI's axis convention, which
  taken literally from the map tile layout would hang the panel below the gate); the map camera's scale (600 m,
  FOV 90°) and the additive tiles; which `MapPoiConfig` fields the code reads for player / racer icons; icon
  clamping at the map edge; the per-Type checkpoint looks (+0x0c, +0x30, +0x1c, +0x20; unused by the data).
- `--spawn` now overrides the event start of `--race` (for looking at a gate:
  `--race 462174 --no-cutscenes --spawn=-531.5,58,2113.3,144.3`); `MW_GATE_LOG=1` logs the shown gates.

### 2026-10-09: nitrous tank and refill traced (FUN_00669ab0, FUN_00667c10); RoadData decoded
- **Found through** the `NitrousViewModel` registration (`FUN_00658ee0`: field offsets into a per-player copy) and
  the `ClearNitrous` script command (→ `FUN_00658c90`, the per-player reset): a 200-byte record per player at
  game +0x264f0. The game-logic tick `FUN_0066ad70` → `FUN_00669ab0` runs the refill events (`FUN_00667c10`, a
  13-way switch reading `NitrousParameters` by offset) and the burn; `FUN_00663270` the near misses. Full rules in
  decomp.md "Nitrous refill"; ported in `mw_handling::nitrous` (`Nitrous::update`, `NitrousRules`) and fed by
  `mostrusted/src/nitrous.rs`.
- **Surprises**: the tank starts empty; `NitrousUsage.845c3879` is a 1 s ramp of the control amount (not "the
  amount"), `7ebf362a` a 0.1 s minimum burn (not a start threshold); burning forces the throttle to 1
  (`FUN_006c1a80`); every car's offline default `Nitrous_Offline` has nitrous disabled (you win the mod first);
  the punch's minimum-burn word is its 50 / 100 start amount, so a punch cannot be cut short; MinSpeed and
  TakingShortcut never award (`aeba495e` false in every variant, and no road carries the shortcut bit);
  near misses award once per 0.5 s window and only above 80 mph; CrashEscape is a police car crashing near the
  player (inferred), FatalHit a takedown.
- **Field names cracked by use**: `fc79a57e` = `Enabled` (the four usages), `aeba495e` = the event awards,
  `d629c55f` = the event's speed threshold (mph), `d63cff99` = its rate per second or one-off amount,
  `21948e6c` = HighSpeed's percentage of the top speed, `efb8084e` = Slipstreaming's percentage, `1732c75e` =
  a contact / lane delay, `d6eeeb05` = the oncoming grace, `02318b5f` = Donutting's speed limit, `959a0ec3` =
  its delay, `c8e9863d` = CatchingAir's time, `fd8a5ed5` = the punch's start amount, `c441f502` / `d5b487c7` =
  the start amount / speed. `4d907940` is in every event and read by nothing found.
- **RoadData (0x203)** decoded (`mw_formats::road_data`): 147 roads, 342 sections, 1494 Hermite points with a
  capture radius and per-direction oncoming-lane bounds in metres (i8 × 4). The player's road position is
  refreshed every 24th tick (`FUN_00668650` cycles the 24 player slots).
- **Near-miss query** (`FUN_00665f30` → `FUN_004b71a0` → `FUN_00aa0440`): a 5 m sphere at the car against the
  broadphase spheres of category 8 (traffic only), one tick of latency; touching traffic (`FUN_006564c0`)
  cancels the window until an empty answer. Vehicle +0x100 is the forward axis, +0x120 the velocity
  (decomp.md's trigger section had +0x100 as a velocity: corrected).
- **Slipstream** (`FUN_0063c790`): the AI port lacked it; now computed for the player against every other car.
- **Checks**: `--test drift` (911, `MW_NITROUS_LOG=1`): nothing for 0.75 s, then 6.1 per second (the Online
  variant's 6/s); high speed adds 1.5/s above 66 mph. `--test donut`: 2.0/s at 25 mph after 0.75 s.
  `--test ai --ai-seconds 90 --traffic-density 2` (the AI drives the player through traffic): oncoming lane
  1.2/s, high speed 1.5/s, air 24/s, an oncoming near miss +6 at 92 mph, the cancel while stuck against a
  crashed car; same-direction near misses and the competitor hit did not occur in that run. Unit tests in
  `mw_handling::nitrous` and `mw_formats::road_data`.
- **Open**: the input action states (2 = pressed is inferred); speedbreaker / jump modes; the broadphase
  sphere source; the slipstream ray's end points; the AI racers' own nitrous variant (they run the default
  rules plus the AIConfig refill); race-start fills (no writer of the amount other than the awards, the reset and the infinite flag was found); wiring the
  gate smash (+10) and gas stations (+100).

### 2026-10-09: "floating grass" at the CONTINENTAL DRIFT start: groundcover residency and sprite size

User report: dozens of small yellow-green tufts hang in the sky and in front of the hills at the start of
event 462174 (camera −610, 56, 2310 looking east / north-east over the park fence at a smooth beige hill).
- **What they were**: groundcover sprites of collection `5cfd7148` (e = 2 / 4 grass and bush sprites at
  y ≈ 120 around (−380, 2345), ~240 m away). `mwtool placement-check --column=-380,2345` (new): the only
  surface there is the backdrop terrain tile `322fcc77` at y 122.5, listed by units 86, 152, 153 and 936,
  which are 2.5–3 km away (x ≈ 1400–2500, z ≈ 3100–3800). The place lies in zone 133 (the camera's zone),
  whose view has no terrain between x −470 and −390 at all and draws the hill as unit 133's main terrain
  `6ddfd1b1`, which ends at a lower ridge. Collection `5cfd7148` is listed by the same four far units: it is
  their groundcover for that hill, placed on their backdrop tier. We loaded every unit's collections
  everywhere, so it stood on terrain that the composed backdrop does not draw there.
- **Which collections the game draws** (decomp.md "Groundcover drawing"): a list of 64 collection slots,
  filled from the loaded units (the filler is not traced). `mwtool placement-check --owners` (new): 400
  collections, 1–34 per unit (mean 12.2), one shared by 144 units; the zone + all its neighbours list at most
  63 distinct collections in every zone (median 36), zone + edge neighbours at most 50. So the slots hold the
  PVS neighbourhood. **Fix** (`crates/mostrusted/src/groundcover.rs`): a group is drawn only when its
  collection is listed by the camera's zone (the nearest zone outside every polygon) or one of its
  neighbours, rebuilt when the camera changes zone, on top of the 125·r distance test. A static per-place
  rule was ruled out: of the groups whose place is in a zone not listing them, 39 % stand on collision (good
  tufts near zone borders listed only by units farther away).
- **Sprite size** (also fixed): `FUN_008c9aa0` writes `(float)(1 << e)` into `GroundcoverConstants`
  (slot 0x7f) per draw record, so a sprite is byte / 127 × 2^e m, not × 4. e = 4 sprites were 4× too small,
  e = 6 16×, e = 8 64×: the e = 6 / 8 groups are 10–60 m tree cards. With them the park hills are forested
  (`--camera=-610,56,2310,-520,54,2310`): the "sand" hill is the forest floor (`6ddfd1b1`,
  `TerrainPBR_Normal`), which the cards hide; its plain look was not looked into further. The live reads of
  2026-10-06 (4 in most dumps, 256 in two) are e = 2 and e = 8 draws.
- Not groundcover: the dark specks against the sky in `--camera=-650,260,1850,-1000,60,2150 --hour 12` stay with
  `--no-groundcover` (another system; not identified).
- Debug: `MW_GC_E=<e>` loads only the groups of one size exponent; `mwtool placement-check --view=<eye,target>`
  lists the sprites in a 30° cone (`MW_VIEW_HIGH=1`: highest above the horizon first), `--column=x,z` every
  placed mesh over a point with its zone (`MW_COLUMN_ZONES=133,86` prints zones, `MW_COLUMN_GC=1` the
  collections per unit).
- Open: camera-facing sprites (two crossed quads still stand in; with the real sizes the edge-on quad of a
  tree card shows as a thin vertical sliver), the per-sprite range and shrink (merged group meshes draw every
  sprite of a visible group at full size), who fills the 64-slot list (is it the player's zone or the
  camera's, and with which neighbours), the layer flags `DAT_010c6f78`.

### 2026-10-09: race cutscenes: event intro / outro sequences, ICE camera takes, no offline countdown
- **Where they are**: `OfflineEvent` fields cracked: `EventIntro`, `EventOutro`, `AROutro`, `IntroMessageGroup`,
  `FeedbackMessageGroup`; `GameMode`: `AROutro`, `CountdownSequence`, `HUD_HSM`, `Intro`, `ResultsSequence`,
  `FeedbackGroupID`. The values are `Sequence` UniqueIds; the intro sits in `EN_US/FEEDBACKGROUPS/<IntroMessageGroup>`,
  the outros in the game mode's group (604180 for races, from the HSM's "Select Game Mode Feedback Group"
  state). All 108 intros load (13–54 s bespoke cinematics); the outros are shared per event type.
- **No countdown offline**: `CountdownSequence` is 0 for every offline mode and the HSM's countdown / false-start
  states are in the online flow. The intro ends on a rolling start (teleports at `Speed_MPH` 70 / 55, AI drives
  the player, slow motion ramps up, the camera blends to the chase camera) and an event trigger, on which we
  start the race. The `GAMEMODEINTR` 10 s is still in the code; where it sits in the offline flow is open.
- **Camera clips**: `AnimatedCameraSequenceItem` clips animate bone 1 of a two-bone camera skeleton; FOV =
  2 atan(18 / `FocalLength`) (`FUN_005c6c10`). The look axis can't be read off one clip (a pure pitch about X
  fits several conventions). 462174's clips hold the bone's +Y down and "+Z forward, −Y up" frames its animated
  car; 575163's hold +Y up and the same mapping turns them upside down. Survey of all 753 intro camera clips: X
  always level, +Y up in 704, down in 49. Rule now: look along +Z, up = the clip's upward ±Y (model decision);
  for the +Y-up clips "look −Z" (the usual exporter camera) also gives good shots in 575163: settle with a
  capture.
- **ICE takes decoded** (`CameraTake`, 45 in CAMERAS.BNDL; `mw_formats::camera_take`, `mwtool camera-take`): the
  whole ICE player read from the dump's ICE library (layout, bit packing, fixed point, Hermite keys with the
  game's tangent rule, spaces / avatars, lens FOV 2 atan(15.96 / lens)); all 45 parse to their exact end. Every
  camera of the race intros / outros is a take. Binding flags mean "sample once" (the outro freezes the car's frame).
  Doubt: `DUTCH` at its +0.25 maximum gives a 90° roll in 11 takes (the code says turns × 65536 as a 16-bit
  angle); check against a capture of the original.
- **Runtime** (`cutscene.rs`): intro before the race (`RaceState::cutscene`, `rolling_start`), outro on the
  player's finish, the skip (Esc / Back after `TriggerTime`, 462174: 1 → 19.5 s), letterbox shared with the crash
  camera. `--no-cutscenes`, `--cutscene outro`, `MW_CUTSCENE_TRACE`, `MW_CUTSCENE_SHOT=[intro:|outro:]<s>`.
  Checked with screenshots at intro 1.5 / 4 / 11 / 15 / 20 / 22 / 23 / 25 s and outro 2 / 6 / 14 s, and
  `--test race` (skips the intro, waits for the outro).
- Open: a capture of the original's intro and outro to settle the camera-bone axes, the FOV convention and
  `DUTCH`; the item handlers (vtables) for the teleport speed and the AI drive's style; nested timeline
  controllers; the outro's UI layouts, audio and post-FX items.

### 2026-10-09: race HUD, countdown, results, UI blend states, Easydrive race start
- **Nitrous "dark box" bug: UI blend states are data.** A `UIMaterial`'s `Material` names a render `Material`
  (GLOBALMATERIALDICTIONARY) that only imports its `Shader`; the shader imports a `MaterialState` (type 0x6,
  e.g. `01000006905172cc`): three pointers (+0 blend block, +4 depth-stencil, +8 rasterizer), the blend block a
  hash / key word then one packed u32 per render target (8): bit 0 enable, 1–5 src, 6–10 dst, 11–13 op, 14–18 src
  alpha, 19–23 dst alpha, 24–26 op alpha, 27–30 write mask (D3D11 enum values). `FlaptGenericShader` (259
  elements of the HUD screens) = `0x7931498b` = src alpha / inv src alpha (alpha-over); `UIAdditiveShader`
  (40) = `0x7911488b` = **src alpha / one**; the unused default is `0x7931498a` (disabled). Flares, glints and
  streaks are authored on black (e.g. `0101000000160c92` is opaque, mean value 11/255) and were drawn alpha-over,
  so the 639×216 nitrous flare of layout 1670359 covered the speedometer. Fixed for every element through a
  Bevy `UiMaterial` with that blend (`ui_quads.rs`); `mwtool ui render` blends them additively too. Not
  decoded: the depth-stencil / rasterizer blocks; the other UI shaders' states (`UIStretched`, `UIRibbonShader`,
  `UINoiseShader`, `UIParticleShader`, `UIDepthTestedShader`) are drawn alpha-over (unchecked).
- **Which screens the race uses** (HSM strings of `GAMELOGIC/GAMEMODES.BNDL`, the number after each "Activate
  ... Widget Group"): 371670 free drive ("CostToState HUD"), 371678 time trial, 371720 online free drive,
  371725 online team DM, 1019407 online speed test, 582390 online results, 1377068 `ArcResults` (the front
  end's results / summary menus), 1014257 `EventIntro`, "Race HUD" names 371726 / 604152. Screen **604153** is
  the one whose widgets bind the race: `AdditionalHUD` 1470173 (POSITION `EventPosition(WITHORDINAL)`, LAP
  `{Lap} / {Event.Data.Laps}` on circuits, DISTANCE `DistanceLeft(DIST)` / `(DIST, LONG)` on sprints, the
  short one shown below 500 m by triggered behaviours), 1470152 (TIME `Mode.Time`), `HUD` 1106106 (WRONG WAY /
  CHECKPOINT MISSED banners on `DistanceToFinishManager.*`), 979749 (EVENT ENDING IN, `Mode.DisplayTimeout`).
- **Countdown and results are `Sequence`s**, not screens: `LayoutSequenceItem`s (`Layout`, `InstanceParams`,
  `StartTime` / `EndTime`, `EnabledBinding`, texts `bf1ce9a1` (Text1) / `Text2` / `Text3` read by the layout as
  `UISequences.Params[#].TextN`). Online race mode 1019777: `4d1a6bfa` = 275829 (bound to `Mode.IntroTime`,
  0–15): "GET READY..." 0–0.9 s, the countdown card ("RACE" / "STARTING IN..." / `{Mode.IntroTime(INT)}`, GO when
  `IntroTime ≤ 0`) 1–5 s, beeps at 0.99 / 1.99 / 2.99 s, "FALSE START" 8–11 s; `ResultsSequence` 1019828:
  leaderboard 1335088 (stacks over `ActiveGameMode.RaceLeaderboard`, columns TIME / TKDN / WRCK / SP, DNF when
  `FinalTime` is 0) and summary 1645120 (`Widget.HUD.SummaryHeadline` / `SummaryDetail`, which 582390's string
  palettes fill: "YOU FINISHED {FinishPosition(WITHORDINAL)}", "TIME {LocalPlayerTime(TIME)}") at 0.25–8.25 s.
  **The offline race mode 730000 has 0 in `4d1a6bfa`, `c9ccb25c` and `ResultsSequence`**; the offline events'
  own intros (e.g. 1439074 for 462174) are camera / teleport / drive-with-AI sequences with no layouts, so the
  offline countdown and results layouts were not found. Ours uses the online race mode's (model decision).
  Reading of the countdown clock: the HSM's "Wait For Countdown Start (Exit When Binding Is True)" with
  `Mode.IntroTime(LTEQ:3)` then "Play countdown (Run Gamelogic Sequence)": the sequence runs on its own clock
  from there (this order makes "GET READY" first, digits 2 and 1 with the beeps, GO at the start).
- **Behaviour vocabulary** (screens 604153 / 371670, counts): triggered `in [OUT] / out [IN]` (22),
  `TO,INVISIBLE` (24), approach offsets `384,0@0+0; -384,0@100+450` (35, also `-1000,0` with duration 0 before an
  `IN`), `TO,WHITE`, `TO,<palette entry>` (HUD_PURSUIT_HEATFILL, HUD_FRIEND), four-number steps on masks
  (`100,100,100,200`, under `UsingNitrous`) and bare ids (sounds). Elements also carry their own
  transition-in (the countdown card's 2954×1794 white flare flashes for 300 ms then `TO,INVISIBLE`). Played now
  in `UiScene::draw_with` (`UiAnim`); our reading of each step is listed in decomp.md.
- **Smaller fixes**: the free-roam HUD drew the odometer five times (every `Nitrous` widget variant places
  1806698); a `DisplayText` binding now wins over the authored string ("PLACEHOLDER"); text that is not a number
  no longer compares equal to 0 in bindings (`FinalTime(EQUALS:0)` showed DNF for a time).
- **Easydrive races**: the RACES tier (`TO_FMN`, item type `FMNEAREST` with callbacks
  `CalculateDistanceToEventByIdString`, `SelectRecommendationDomByEventIdString`, `ExitTierVia(TO_FMN_OPT)`) and
  its ACTIONS tier (`TO_FMN_OPT`: `WaypointItem`, `StartEventItem`) are generated in code; the labels "SET
  DESTINATION" / "START RACE" are the `TO_BL_OPT` hook items' (`StartEventFromBinding`). Wired to a runtime
  `race::StartRace`.
- **Open**: the offline countdown / results / event-intro card; which screen the HUD widgets pick per mode
  (`Widget_HudSelection`); racer names (results show the car names); masks with textures are rectangle clips;
  transition-out of disappearing elements, idle behaviours, colour steps; the depth / rasterizer state blocks.

### 2026-10-09: AI racers: same physics, the racer state ported, AIData decoded, the race hookup
- **Multi-car physics**: `car_physics` runs every `Car` from its own `CarControls` (the game writes all 16
  race-car slots' controls with `FUN_009aacc0` from per-player input records, `FUN_0091c5c0`); the player's come
  from `CarInput`, single-car systems follow a `Player` marker, cars collide with each other and a car contact
  sets the impact yaw damping's car timer. The 911's accel / brake / corner / drift test lines are identical
  before and after (3.73 s, 38.1 m, 2.57 g, drift 37.6°).
- **The AI driver is read from the dump** (decomp.md "AI racers"): AI racers run the full handling model; the
  racer state (8) steers with a PID on the look-ahead lateral error (per-car gains from
  `Vehicle_Gameplay.PIDController`), integrates a pedal from a speed PID, fires nitrous on straights below its
  target, brakes on a 1.5 s time to contact, and recovers / resets after sticking. Ported in `ai_driver.rs`,
  `AIConfig` read from `GLOBALCONFIG`.
- **No corner speed for a physical racer**: its target is the rubber band (a speed cap from
  `RaceBalancingData`), lowered only by obstacles; every reader of the path's corner speed byte is the virtual
  progress of a car-less racer or an uncalled helper. Without a race (`--test ai`, no cap) cars crash where a
  corner comes too fast (the Veyron at 280 km/h, ~800 m into the test route), recover and reset; with the
  event's rubber band (125 mph × 0.5–1) all 8 opponents of race 462174 finish its 3 laps (lap times 87–115 s,
  the player standing at the start, so the AI runs its "ahead" factors). A live read of the target speed during
  a race should confirm this. The game's corner speed rule is ported for reference (`MW_AI_CORNER_SPEED=1`
  applies the uncalled braking look-ahead; at 1.8 × √(gμR) with μ ≈ 3 it limits little).
- **AIData (0x200) decoded** (formats.md): the AI's road graph, 9153 nodes with drivable widths in a 125 m grid,
  1160 junctions, 284 jump links; no speeds or racing lines. The game A*-searches its junctions for routes and
  relaxes a racing line inside the road width (not ported: we follow the centre-line spline).
- **Pace on the same path** (`--test ai`, the player's 911 driven by the same driver as an AI 911 starting
  20 m ahead): splits to 1300 m 25.8 s and 25.6 s (same car, same law, same physics), before both meet the
  corner they take too fast without a rubber band.
- Discrepancy to settle live: the rubber-band search found `FUN_00514590` copying the schedule speeds to driver
  +0x5668; the path search listed only the init and the traffic substates as its writers.
- Open: the racing line, the lane planner's obstacle scoring (`FUN_004febe0`), racer interaction (side swipe /
  ram, `FUN_005050d0`), the opponents' own cars (`AIPlayerInformation` / `AIPlayerType` rollout), the rubber
  band's time correction, AI engine sounds and tyre marks (both follow the player only).

### 2026-10-09: events decoded, race rules traced, races playable (`--race`)
- **Where the events are**: not one resource each. `GAMELOGIC/GAMEMODES.BNDL` has one 240 KB GenesysObject
  (`0100000000045d5b`) whose polymorphic `Item` array holds the 111 `OfflineEvent`s next to the online events,
  the 12 `GameMode`s, arenas and balancing data; `GAMELOGIC/GAMEPLAY.BNDL` holds the `EventList`s (career: 109
  ids), `AllowedVehicleList`s and `AIPlayerType`s the same way. A first install-wide scan by type name found no
  `OfflineEvent` object, because the scan only listed top-level objects (dead end worth remembering: look
  inside `Item` containers). References are UniqueIds, matched against items' `655d3f5f`. Layout and the event
  kinds in formats.md "Events"; `mwtool events [<id> --png]`.
- **Checkpoints and starts are TriggerData records**: every `Checkpoints` id is a kind-0 box (2 m deep, 36 m
  tall, as wide as the road or junction), every `SpawnPosition` / `Placement` a kind-4 point with a heading.
  All 111 events resolve (0 unresolved ids). The earlier decomp note that no `GameMode` instance exists on PC
  was wrong: they are container items too (`09d6656d` = 10 s, the crash reset delay).
- **Race rules** traced from the dump (decomp.md "Races"): the intro is GameMode `15f07dbf` (10 s for races,
  0 for speed runs) with the 3-2-1 in its last 3 s; cars are frozen by the handbrake only (throttle revs);
  only the target gate is tested, as the car box's overlap with the trigger box, in order; laps and finish by
  the route-entry count; position = 1 + racers finished or strictly closer to the finish along the route; a
  wrong-way / missed-checkpoint state machine (1 s / 2 s / √10 m / 1 s); won = top three (first only on the Most
  Wanted races); a race respawn faces the target gate (corrects "nearest race-route point" in "Crashes").
- **Route markers** found by following `GameLogicConfig.CheckpointConfig` to two materials in
  `HAWAII/GLOBALRESOURCES.BNDL`: shader `Chevron` (green HDR arrows, `materialDiffuse` (0, 5.6, 0.036), scrolling
  by frac(Time × speed)) and `ChevronBlockRoad` (red crosses fading over 500 m). Drawn along the events'
  `CustomChevron` polylines (`chevron.rs`); they look like the original's holographic arrows. The strip builder
  is not in the dump: board height, UV scale and which flag selects the crosses are model decisions.
- **Runtime**: `--race <id>` (event car, or `--drive`), `--test race` (kinematic run through every gate; circuit
  462174: 27 / 27 gates, 3 laps, P1, wrong way seen on a backwards start; sprint 983491, speed run 1186771, Most
  Wanted 1399902 with `--drive 911` also pass). Open: the gate POI visual (id 1013519 not found as a top-level
  `PointOfInterestVisuals`), the road-network route segments (ours are straight), what advances the race clock,
  the countdown sequence id, the AI start hold, the outro / results flow, `CheckpointInfo` counts that exceed the
  checkpoint count on some events (the dynamic-array count rule may be off there).

### 2026-10-09: traffic at runtime (spawn, lanes, lights, following, hits)
Code findings are in decomp.md "Traffic runtime"; the runtime is `crates/mostrusted/src/traffic.rs`.
- **Traffic cars are the `VEHICLES/VEH_<id>_TR.BNDL` bundles** (11, one per `TrafficVehicle.VehicleComponent`):
  one LOD, a `VehicleGraphicsSpec` like the drivable cars, no paints inside. Their paints are the
  `VehicleColourPalette` (`16671e74` handle array) of the `VehicleComponent` in `VEHICLES/TRAFFICATTRIBS.BNDL`,
  1–7 per car, used round-robin per type (`FUN_00608720`). Box and mass come from `RaceCarPhysicalDefinition`
  `0x01000002_<component>` (1135–2633 kg, the bus 392322 7000 kg). No trailers in the PC data.
- **The fuzzy option choice is ported exactly**: the scorer's SIMD block (`world+0x69810`) was dumped from
  `snap2` (traffic object at `0x23f4e780`, 16-aligned, so the research log's `+0xa2b74` is `+0xa2b70` from it).
  Its lanes are `TrafficFuzzyLogicConfig` envelopes stored as `(a, d, 1/(b−a), −1/(d−c))`, matched field by
  field; the cruise option's 0.2 at block `+0x510` has no data source found (live read).
- **Traffic lights**: junction records decoded (formats.md); phase masks are "held" bits (red). Checked on
  every junction: all 189 stop lines are referenced by light groups and owned by sections.
- **Hull population**: newly active cells are filled along their lanes at the flow's density (one car per
  `60 / (4.78 · Density)` s of lane at lane speed), no car within 50 m of the player; the 256-slot cap is
  reached at once in the downtown area (9 cells active), after which cars only appear as others leave.
- **Checks** (headless, `MW_TRAFFIC_LOG=1`): a car stops exactly 10 m behind the stationary player (mode 0,
  `prm[0x9c]`), the next one queues behind it (mode 4) while the neighbouring lanes pass at 33 / 42 mph (lane
  speeds); 3–13 cars wait at red lights at any time; hitting traffic at 30–100 km/h turns the car into a crashed
  body that spins away (screenshots in the session scratchpad); the tick costs ~0.2 ms for 256 cars.
- **Performance** (`--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --bench 10`, other builds
  running, load average ~4): back to back 15.9 ms mean with `--no-traffic`, 17.3 ms with traffic (main world
  4.7 → 5.7 ms, render prepare 7.6 → 8.8 ms, far-cascade shadow casters 879 → 1332). Drawing every car instead
  of those within 250 m cost 25.3 ms against 13.3 on a quieter machine (the draw range is a model decision;
  most of the cost is the extra mesh entities in the render world).
- **Dead ends / open**: the game's switch to a physical car (the crash entry for traffic; `FUN_005fcbd0` is the
  kinematic wheel setup), the manoeuvres (option 0), the point the load budget measures from (`world+0x36110`),
  the 24-slot vehicle range, the hour array's reader, per-car brake / indicator lights (our light state is one
  global per material).

### 2026-10-09: exhaust effects: the game's pop rule, the car spot effects, and the Bear bindings decoded
- **Start**: no exhaust / backfire / nitrous effect name in the data, but `strings.csv` has the sound getter
  `GetPopProbability` ("the current exhaust pop probability") on `RaceCarEntity` and `GetPrimaryExhaustTransform`.
  The getter reads vehicle +0x1eb4; its writer `FUN_009b20b0` is the pop rule (decomp.md "Exhaust pops and
  backfires"), run every tick by the engine-sound update. It reads `Engine.Sound` (+0x70 `38d82e04` = backfire,
  +0x7c `99a16939` = pop; the struct was found live: vehicle +0x4b0 → the `Engine` struct, idle 1000 / max 6200
  on the snapshot car) and sets two flags: +0x1f42 starts `Backfire_Sequence` and holds effect group 14 for two
  updates, +0x1f41 starts `Pop_Sequence`. Constants: 4000 audible rpm, × 10, 0.8 (`.bss` 0x01186810, read live).
- **Effects**: `VehicleVfxBehaviour.SpotEffects` (every car) name `BearEffect`s by `Group`, a bit of the car's
  effect mask that `FUN_0057a5e0` builds (nitrous 8, backfire 14, engine running 19; formats.md table). The
  positions are `VehicleGraphicsSpec` locator groups (+0x1C table, decoded now: the 911's group 137267 = its four
  pipes). `FUN_005b28d0` starts each effect at each locator on the group's rising edge and passes `Intensity` = 1
  (`FUN_0049bc60`). So the exhaust effects are `…24f2f` (smoke at idle, below 6 m/s), `…24f34` (nitrous flame)
  and `…24f2d` (backfire: orange glow, blue and yellow flames, a light).
- **Bear runtime**: the action kinds are an index into the game's action table, whose names (`.rdata` 0x00f9d664)
  name every kind (formats.md). Read from the code and ported: `UpdateBindings` (0x33; the "controller": curves
  over chosen inputs, combine ops, binding slots; one input is the player car's engine load, a
  `BearGlobalParameters` field), `EmitterConePlus` (0x00: windows on time, camera distance and speed, rate /
  burst / speed scaled by binding slots), emitter-space layers (0x14 + 0x1A / 0x1B: the flames ride on the pipes;
  before they were left behind the car as a faint trail), and `ParametricSetColourGradient` (0x1C): a piecewise
  ramp, not a polynomial, with rgb × the fourth column (brightness 2 → 8 on the nitrous flame) instead of alpha.
  The colour gradient change applies to every effect; the tyre smoke of `--test donut` looks the same as before.
- **Result**: 911 `--test exhaust` (full throttle through the gears, a lift, nitrous): 22 pops, 0 backfires; its
  upshifts drop the audible rpm by at most 9 % of the rev range in a tick, the backfire needs 9 % × 1/r. The BMW
  M3 GTR (0.65) needs 3.5 %: its run had 0 by chance (P ≈ 0.3); `MW_EXHAUST_LOG=1` prints every drop and event.
  The shift phases are ported from the game (decomp.md "Drivetrain and brakes", checked on `ref/xkr_launch`), so
  the 911 rarely backfiring follows from its data; the M3 GTR, MP4-12C, F1 LM and the SWAT van carry the high
  values.
- **Dead ends / open**: the +0x1f43 flag (no reader); who starts `Gurgle_Sequence` (bound to the pop probability;
  played by our retrigger model); the nitrous condition's second half (`FUN_006a0020` / `FUN_0069ff20`, taken as
  "nitrous burns"); groups 9 / 20 (nitrous modes 1 / 3, not run) and 12 (wreck); `da6ac384` = 2's world flag
  +0x1b934; the cone emitter's angle maths (+Z axis inferred from the pipe locators); the backfire light
  (`InitialiseAndSubmitLightLocalSpace`, 0x0F) is not drawn; `UpdateParametricFadeOverDistance` (0x38) is still
  the old near-fade guess, which dims the flames closer than 8 m; the `UVDistortion` offset of the smoke; the
  other `BearGlobalParameters` inputs (+0x60, +0x68, +0x6c, +0x70, +0x78, bytes +0x7c / +0x7e). No capture of
  the original was made: flame size, colour and frequency are unchecked against it.

### 2026-10-09: no generic angular damping on a driving car; the chassis inertia is ours; post-impact yaw damping ported
- Searched for the source of our unsourced `AngularDamping(0.4)` through the callers of the angular-damping
  routine `FUN_00907b50` (decomp.md "Angular damping"). The vehicle's damping constraint (`FUN_009aa700`, record
  at vehicle +0xe0) comes from `PHYSICS/DAMPINGCONSTRAINTSLIST.BNDL`, whose `DrivingVehicle` entry is empty; live
  in both snapshots the record holds zeros and FLT_MAX thresholds. The only other per-tick damping of the
  player car outside a drift is `FUN_008f7950`: yaw only, 0.05 × a timer that a collision sets to 1 and that
  runs out in 1 s (world, car) or 0.3 s (object). Ported (drive.rs `ImpactYawDamping`); fires on both impacts
  of `--test crash`; the 911's accel / brake / corner / handbrake / drift / landing results are unchanged.
- **Chassis inertia confirmed live**: the body record's local inverse inertia (+0xe0) is a solid box with the
  chassis half extents about the centre of mass × `Inertia_Scale`, to four digits on both snapshot cars
  (1399602, 1762 kg; 122769, 1807 kg). Ours already does this. The MP4's four hulls do not enter (the lead in
  the 2026-10-07 entry is closed).
- So the original has no yaw resistance we lack in the body: on `ref/live1` 98.98 s (MP4-12C, the brake-tap
  entry) ours with the original's zero damping over-rotates more (yaw RMS 8.6 → 14.0 °/s, speed RMS 7.7 → 11.4
  mph, slip 2.8 → 5.9°). The 0.4 stays, listed as a model decision (handling.md), until the real cause is found.
- Where the runs part (0.4 kept): the drift's yaw torque, scale and side force match tick by tick, the yaw
  rates match up to 1.5 s. Under the full brake in the slide (1.75–2.25 s) the original slows 111.4 → 89.4 mph
  (≈2.0 g), ours 106.6 → 74.5 (≈2.9 g); from 2 s our front / rear slip angles run 8° above the original's
  (2.25 s: front 47 / 39°, rear 25.5 / 17.6°) and the speed hold then pushes twice as hard to catch up (20 kN
  against 10 kN). Next: the tyres' combined braking and side force at 20–45° of slip.
- **Correction, same day: most of that was the replay, not the physics.** (1) Our replay logged `speed_mph` as
  the forward component of the velocity, the game's +0x1620 is the ground speed signed by direction; in a
  20–30° slide ours read 6–13 % low. (2) The approach eased off near the start speed and the MP4 ran into the
  60 s limit at 125.8 mph (recording 128.7). (3) It started in whatever gear the approach left (4th at 8100
  rpm; the original was in 5th at 6400). Fixed in replay.rs (the game's speed, full throttle to the start
  speed, the recording's first gear engaged with the clutch closed: `Drivetrain::put_in_gear`). On 98.98 s:
  speed RMS 7.7 → 3.1 mph; the braking in the slide is no longer far off.
- **The 98.98 s window has a terrain event**: at 2.04 s the original's front-right wheel loses contact for one
  tick (`w1_contact` 0), the next tick's drift yaw torque is 0 (it needs every wheel down) and the yaw rate
  steps from −63 to −49 °/s in two ticks. Our flat-track replay cannot have it; the run is only comparable up to
  2.03 s. Up to there the residual yaw moment (Iy dω/dt − the drift's torque, Iy = 2697 from the box) agrees
  with ours within ~1 kN m; ours still peaks higher (−72 against −62 °/s with the 0.4, −79 without).
- Five brake-tap windows of `ref/live1` (MP4), each cut at its first wheel-contact loss, yaw RMS against the
  original with our 0.4 / with 0: 65.5 s 15.6 / 19.9, 69.0 s 5.5 / 7.6, 98.98 s (to 2.03 s) 6.4 / 7.8, 107.5 s
  4.8 / 12.9 °/s; 146.0 and 156.2 s are poor either way (26 / 24 °/s: they start mid-manoeuvre, the original
  already yawing). So the original does resist yaw more than our undamped car in these slides; the code puts
  no damping there, so the cause is in the tyre or chassis model. Ruled out: the inertia (above), the blend
  scale (below), the effective mass (we already solve wheel after wheel with the chassis's effective mass, as
  the game), extra wheel bodies (the island's body list in the snapshot holds the chassis, other cars and props;
  the wheels are spin-only records).
- **Settled from the snapshots**: the wheel constraints are an array of 0x500-byte records at island +0x20
  (the island object at 0x07b72a80 in both snapshots: bodies +0x00, 0x100 each; wheel records +0x08, 0x80 each;
  found by scanning for that pointer pattern). The player's four hold +0x210 = (1, 0, 0, 0) and +0x4aa = 1:
  the blend-input scale is 1 (`FUN_009c5da0` broadcasts lane 0), and the full tyre solve runs. Wheel records:
  spin inertia (+0x54) 1.06 front, 2.125 rear on the SL 65 = ½ m r² from the data (19 kg, 0.334 m; 23 kg,
  0.352 m) + 0.70 on the driven wheels, the same as ours. The constraint's +0x100 solve state does not hold the
  caps between ticks (impulses are written over them), so the caps cannot be read from a snapshot.

### 2026-10-07: the anti-roll bar's solve found (`FUN_0092ee70`); `FUN_00932af0` is the sleep pass; 2 sub-steps
- Prompted by the Vita cross-check ("Update constraints" ⊇ the island step). `FUN_00932af0`, which Ghidra could
  not decompile and which was taken for the bar's solve, is the island **sleep** pass (disassembled from
  snap2; details in decomp.md "Suspension, wheels, aerodynamics"). The bar is solved by **`FUN_0092ee70`**
  through `FUN_0093f200` → `FUN_00932f90`: torque k·atan(Δy/Δx) − c·(chassis ω · forward) per axle from the
  two wheel centres in the chassis frame, ±F along the up axis at the wheel centres, capped per wheel at
  |Δy| / (4 W dt²), chassis only, explicit, at dt / 2 in each of the 2 sub-steps. Checked by hand at
  0x92f00e..0x92f031 and the zeroed term at 0x92ee86 / 0x92eec5. The earlier note that no decompiled function
  reads element +0x20 / +0x30 was wrong: `FUN_0092ee70` does.
- The island step's sub-step counts are world +0x460 = 2 (`FUN_00932190`) and +0x464 = 2 (`FUN_0093f200`),
  from the constructor `FUN_009330d0` and in both snapshots: `STOP_ITERATIONS` 4 → 2.
- Ported (drive.rs `anti_roll`, replacing `anti_roll_loads`, which added a compression-difference load to each
  wheel's spring force and switched off for a wheel in the air). 911 before → after: jitter roll p-p
  0.0121° → 0.0092°, roll-rate p-p 0.084 → 0.057°/s; climb settle 4.20 → 4.13 s; landing, corner (2.57 g),
  brake (38.5 m) and handbrake unchanged. The steady-corner roll is 0.0° before and after: our tyre forces carry
  no roll moment, so the bar mostly acts over bumps and landings.
- **Damper slots settled**: `FUN_0092f160` (the damper element) uses +0x18 = `0c5f89b3` × c while the strut
  extends (wheel moving away from the chassis anchor) and +0x2c = c while it compresses, which is what we
  already assumed (handling.md "Model decisions"); now sourced.
- Leads left: the spring `FUN_0092f510`; then our springs and dampers could move into the same sub-stepped
  element pass (they act on the wheel-vs-anchor velocity, ours on the mount's velocity along the ground
  normal). What wheel record +0x0c is (1 in every read).

### 2026-10-07: wall scrape and the drift's collision exits ported; `--test drift` has not drifted since a9a73f6
- **Wall scrape** (`FUN_008f7ab0`, the "Update wall scrape" profiler scope, step 9 of decomp.md "Update loop"):
  read from the disassembly (the pseudo-C garbles the shuffles). A world contact above 6 (`.rdata` 0x01021ff0)
  zeroes the steering fraction +0x1c80 and the tyres' angle +0x1c8c, keeping +0x1c84 and the range +0x1c88.
  Ported in drive.rs after the drift (`car.steer = 0`). Seen firing in `--test crash` on both impacts (closing
  speed 31.8 and 17.9 m/s; `RUST_LOG=mostrusted::drive=debug` logs it), silent otherwise.
- **Drift collision exits** (`FUN_009d83e0` codes 1 and 3): world contact above 6 (`.data` 0x010cfb40), object
  contact above 6 (`.rdata` 0x01026230, from a per-car list of 0x90-byte entries); precedence as the game's (later
  codes overwrite: 1 < 3 < 5 < 7 < 8 < 9). Unit test `hard_contacts_end_a_drift_and_soft_ones_do_not`.
- The contact value: the game's comes from the collision table (world record +0x1c; the same field the camera's
  impact shake reads with range 5..60), whose writer and unit were not traced. Ours is the largest normal closing
  speed (m/s) of the body collider's world contacts, and of the body's and bumper box's other contacts for the
  object exit: the same stand-in the camera already uses (handling.md "Model decisions").
- 911 tests unchanged: corner 2.57 g, 0–100 km/h 3.73 s, landing ok, smash and crash fine.
- **Found while testing**: `--test drift` gave no drift (peak 4.5°), on master too; bisected to a9a73f6 ("Tyres from
  the code", the game's μ × N caps replacing LAT_GRIP 0.4 / LONG_GRIP 0.7). **Not a fault of the model**: replays of
  `ref/live1` show the original would not drift with that script either. At 166.42 s the driver did what the
  script did (straight at 77 mph, a 0.2 s full brake tap, full lock during it): the original's rear slip peaked at
  2.5° against the tap threshold recovering from 2.17° at 10°/s, no drift; our replay of that window also stays out
  (1.96°). All six brake-tap entries in the recording follow yaw already built, by a flick the other way (65.98,
  69.52, 98.98, 107.97, 156.70) or full lock on the throttle (146.00); replayed, ours enters on the same tick
  (within one 120 Hz row) with the rear slip within 0.1–0.2° RMS. The 0.6 s tap window is confirmed (presses up to
  0.48 s lowered the threshold, one of 0.68 s did not). The live1 car for 58.4–178 s is the **McLaren MP4-12C**
  (id 122765; 1350 kg, corner loads 2827 / 3800 N, thresholds 8.0 / 2.17 / 5.5); 0–53.7 s the XKR; from 178.4 s an
  unidentified 1807 kg car.
- **Fixed: κ taken before the brake step.** `FUN_009c75b0` runs the brake step `FUN_009bbff0` before the tyre pass
  `FUN_009c6f00`, whose caps (`FUN_009c5da0`) take κ from the stepped wheel; ours took κ first, so a rolling
  wheel's braking cap came out near 0 and the wheel stayed slowed. Replay of 98.98 (κ RMS against the original):
  braking before the entry front 0.076 → 0.006, rear 0.027 → 0.006; braking in the drift front 0.174 → 0.041, rear
  0.091 → 0.031. 911 brake 100–0 km/h 38.5 → 38.1 m; other tests unchanged. (Detail not verified: `FUN_009c5da0`
  reads the spin at +0x4c where `FUN_009bbff0` writes +0x2c; the decompiler garbles the arguments.)
- `--test drift` now plays the recorded 98.98 s entry (full lock one way 0.2 s, centre 0.1 s, full brake with full
  lock the other way 0.45 s, then throttle with the lock held): 911 drift 5.25 s, peak 37.6°; MP4-12C 5.25 s, 18.2°.
- `--replay` now pairs each row with the inputs that produced it (our rows were one tick late) and reads speed, yaw
  rate and body slip after the physics step; new columns `wheel_angle`, `tyre_yaw_front/rear`, `drift_yaw_tq`,
  `drift_side_f`, `drift_hold_f`, `drift_yaw_scale`. In the recording, `drift_side_f` x/y/z hold the same value.
- Open: ours over-rotates once in a drift (98.98: yaw −84°/s against −54°/s at 2.0 s, more speed lost) although the
  drift forces match the original's within ~2 % (side force exactly). Our `AngularDamping(0.4)` has no source; with
  it at 0 the over-rotation gets much worse (yaw RMS 13.8 → 21.9°/s), so the original has yaw damping or more yaw
  inertia on the ground that is not traced. Lead: the MP4's chassis is four convex hulls, ours a solid box.

### 2026-10-07: what the Vita eboot can and cannot give (scope of the ARM build)
- The decrypted Vita eboot (below) is self-relocated and analysable (27,867 absolute `0x81......` code
  pointers; r2 `aaa` built 14,350 functions), so real code/data xref works. **But the handling/physics layer
  is as anonymous in it as in the PC dump**, for two confirmed reasons:
  1. Physics reflection type/field names and the game's own source-path asserts were **compiled out of this
     retail build**. Present: gameplay/AI/UI/Lua command names (`AwardNitrous`, `SetInfiniteNitrous`,
     `GetSteering`, ~40 `*HSM Behaviour` state names) and SDK source paths only (Blaze, EAAudioCore,
     rwfilesystem). Absent (0 string hits): `NitrousParameters`, `NitrousStrength`, `HandlingBehaviour`,
     `SteeringBehaviour`, `DriftBehaviour`, `Maximum_steering_angle`, `*_speed_steering_range`,
     `Power_curve`, and the type tokens `Gearbox`/`Gen.Vehicle`/`GenReflection`/`AttribSys`.
  2. The physics *simulation* names don't exist at all (compiled out), and the surviving gameplay/UI command
     names (`AwardNitrous`, `UsingNitrous`, profiler-scope labels) that the sim would use are **hash-interned
     and have no code reference** — so "find string → xref → name the handling function" does not work for the
     physics layer. (Reflection/telemetry *property-registration* strings like `GetPlayerSuspensionLoad` ARE
     referenced — see below — but only from the ID-registration table, not from the solver.)
  3. The **PC GeneSys field hashes are not Vita code immediates** (different intern/hash scheme): `/v4` for
     `0xe9de413e` (Nitrous_strength), `0xd00887ca` (engine +0xf0), `0x0798d257` (punch) = 0 each, vs 1931
     hits for `0x3f800000`=1.0f as a control. Portable float constants don't discriminate either (10.0f = 68
     hits). So PC offsets/hashes cannot pin a Vita function.
- **Tooling correction (important):** r2's `aaa` recorded code→code call xrefs but **NOT data xrefs from
  MOVW/MOVT split-immediate loads** (the usual ARM way to materialise a string/data address), so `axt`/`/v4`
  on a string vaddr wrongly returns nothing even for strings that ARE referenced. Confirm string→code with a
  MOVW/MOVT+LDR-literal scanner (scratch `movtscan.py`), not r2. Verified: `GetPlayerSuspensionLoad`
  (0x816d1464) is loaded by a MOVW/MOVT pair at 0x811ba082 — a real reference r2 missed. (r2 also mis-decodes
  some literal pools as Thumb branches, compounding the problem.)
- **Net**: the Vita build is a good structural reference (the physics-tick profiler tree below) and a second
  binary to confirm *behaviour*, but it is **not a shortcut for the anonymous handling internals**. Tracing a
  specific handling rule in it costs about the same as the PC searches that already failed, with ARM/Thumb on
  top. For the remaining data-reader dead-ends (nitrous multiplier reader, steering neutral-offset +0x2c70
  writer, etc.) a live read/probe on the running PC game is the more promising route, not more static Vita work.
- Dead-ends this confirms are NOT closable from the Vita build as-is (all three delegated searches ended
  negative): nitrous multiplier reader (handling.md ~L444), steering neutral-offset q writer (~L472), the
  anti-roll bar per-tick solve (PC `FUN_00932af0`, ~L467-468), strut-stop slack (wheel constraint +0x470,
  ~L477), and damper rebound-vs-bump application (spring +0x18/+0x2c, ~L466). All remain
  telemetry/assumption-backed; the better route for them is a **live read/probe on the running PC game**, as
  handling.md "Model decisions" already anticipates for `FUN_00932af0`.
- Two concrete Vita entry points found, if the physics-data subsystem is ever walked by call graph (not names):
  - **Damping-constraints bundle loader at Vita 0x81380f70** — references the string
    `PHYSICS/DAMPINGCONSTRAINTSLIST.BNDL` (vaddr 0x816f4ae0) via MOVW/MOVT; the asset-ingest side of the
    per-car spring/damper definitions (not the per-tick solve).
  - **Telemetry/reflection property registrar**: `fcn.811b88ac` (~6.4 KB) builds a property table binding
    name+description+integer-ID and calling registrar `fcn.8140ee9a`. `GetPlayerSuspensionLoad` (0x816d1464,
    ref at 0x811ba082) and "Current level of Suspension Load" (0x816d147c) are registered here as IDs, NOT
    function pointers — the getter is dispatched elsewhere by ID. This is the UI/telemetry surface, not the
    per-wheel load solver (PC `FUN_00905910`: N = wheel +0x104 static corner load + ½ axle downforce).

### 2026-10-07: the Vita tick tree is CONFIRMED identical in the PC dump
- The same profiler-scope label block exists in the PC dump (`re/NFS13_dump/strings.csv`) at
  0x01022d38..0x01022f10, **byte-for-byte identical to the Vita block in labels, indentation and order** —
  e.g. `"Update Race Car Physics"` (0x01022d38), `"  Apply handling forces"` (0x01022d64),
  `"      Update Handbrake"` (0x01022e00), `"    Update steering"` (0x01022e8c), `"  Update wheel fr input"`
  (0x01022ee4), `UpdatePreSolveVel`/`UpdatePostPhysics` (0x01022efc/0x01022f10). Same engine, same source.
  So the tick tree below is not just the Vita's — it IS the PC game's per-vehicle physics tick structure.
- Like on Vita, these label strings have **xrefs=0** in Ghidra (the profiler computes the address), so they
  don't name functions by themselves on either side — the FUN_ mapping must come from the known decomp
  anchors + the call tree + RTTI class names (`?RaceCarPhysicsObject`, `BSimpleRaceCarPhysicsObject`,
  `?DynamicPhysicsObject`, `APhysicsObject`, all in the PC dump).

### 2026-10-07: physics-tick structure from the Vita profiler labels
- The Vita eboot carries a contiguous indented profiler-scope label table at vaddr 0x816f4590..0x816f4ac0;
  indentation = call depth. It maps the per-vehicle physics tick (the Vita equivalent of decomp.md "Update
  loop"), even though the labels themselves aren't pointer-xref'd to their functions:
  - Frame **Update Race Car Physics**: Attribsys Update → **Apply handling forces** → Update physics object
    → LineTests → Update network state → Update RaceCarPhysics { Update RoadVehiclePhysics; Update Car
    { Handbrake → Engine → Driving { wall scrape → steering } → sound } → steering } → Update vehicle info →
    Update constraints → Update freezing → Update wheel fr input. Then UpdatePreSolveVel, UpdatePostPhysics.
  - Enclosing frame **Update Module** (sim frame): voice pools, PhObjM create/destroy race cars, inputs,
    network extrapolation, PhObjM Update_Pre/PostCarnage (traffic create), tri-cache, gameplay manager,
    update solver inputs, carnage, collision analyzer (Move Points To Hull / Handle Race Car Wall / Non Race
    Car World / Object Object), collision rep mgr, damage.
  - Behaviour/order correspondence to PC `FUN_` (hypotheses, not address-confirmed): Handbrake↔FUN_009d27b0,
    Driving→steering↔FUN_009d73f0→FUN_009d31f0, wall scrape↔FUN_008f7ab0/turn-around FUN_008f8360,
    LineTests↔suspension ray casts, Update wheel fr input↔tyre inputs FUN_00905910, Update constraints↔island
    solver FUN_0093f5a0. Drift has no own scope (folded into Driving/steering).

### 2026-10-07: PS Vita eboot DECRYPTED — statically analysable ARM build in `re/vita/FAGDec`
- FAGDec (run on the modded Vita, base + patch) decrypted `eboot.bin` and the `sce_module/*.suprx` to ELF.
  Output in `re/vita/FAGDec/{app,patch}/PCSB00183/`. The **`patch` eboot is the one that runs** (8,050,440 B
  vs `app` 8,049,928 B — the game has an update). `.elf` and `.elf.temp` are identical; `.sha256` +
  `self_auth.bin` are FAGDec bookkeeping.
- `eboot.bin.elf` is a genuine **ELF32, little-endian, Machine=ARM, Type=0xfe00 (SCE exec)**, entry
  `0x67c798`. Whole-file entropy **7.226** (normal ARM code + data, NOT packed — contrast the x86
  `NFS13.exe` at 8.0). **This is the first statically analysable copy of the game's code.**
- Segments: LOAD `0x81000000` R+E, FileSiz `0x78a3a8` (~7.9 MB `.text`+rodata); LOAD `0x8178b000` RW
  (data/bss, MemSiz `0x12a0d8`); a SceModuleInfo segment (PT type `LOOS+0xfffff01`). **Load base
  `0x81000000`** — use when disassembling.
- Retains debug strings and **full source paths**: `D:\dev\CGCode\Hawaii\Runtime\VITAOPT\Code\...` ("Hawaii"
  = engine codename, matches the PC `HAWAII/` bundles). Handling leads found as plaintext: the string
  `"  Apply handling forces"`, `AwardNitrous`, `GetPlayerSuspensionLoad`, `GetChassisModifications*`,
  `Player_Vehicle_Suspension_Hits`. `Blaze::GameReporting::NFS::*` C++ symbol names present.
- Caveat: ARMv7 build of the same engine, not the x86 PC binary — structurally the same logic, but
  constants/layout may differ by platform. Use it to recover *control flow and which fields are read* for a
  handling rule, then confirm the exact constants against the PC dump / live reads per the fidelity rule.
- The `.BNDL` assets were NOT decrypted by FAGDec (SELF layer only) and **getting them decrypted was tried
  and abandoned** (2026-10-07, user decision). What was ruled out, so it isn't re-attempted:
  - VitaShell **copy** from `ux0:/app/PCSB00183` serves ciphertext (FTP-pulled `PHYSICSCONFIG.BNDL`
    byte-identical to the dump, SHA1 0e3880b7…); `ux0:` is aliased to the microSD `uma0:`, so neither path
    helps.
  - VitaShell **direct open** (hex view) of a `.BNDL` also shows ciphertext (magic `3e 59 86 8d`, not
    `bnd2`) — this installed title has no live PFS mount through VitaShell, so VitaShell cannot decrypt it.
  - VitaShell FTP (192.168.0.39:1337) reads raw encrypted files and rejects `DELE` (550).
  - `psvpfsparser` (offline PC) needs a klicensee/zRIF **and** F00D service access — multi-step, likely
    dead-ends on F00D. `MaiDumpTool` would decrypt on-console but is a 2016 unvetted binary (not used).
  - **Why it's not worth it:** the Vita bundles use a different GeneSys hash scheme than the PC build (PC
    field hashes aren't Vita immediates, confirmed above), so decrypted Vita data wouldn't cross-reference
    cleanly with the PC bundles we already read natively. The value was always the code, which we have.
  - If ever needed: a PFS dumper that uses the console's reF00D (the clean on-Vita route) into `ux0:/dump`,
    then FTP in. The eboot + modules are already decrypted in `re/vita/FAGDec/`.

### 2026-10-07: PS Vita build (`re/vita/PCSB00183`) — fully encrypted, needs on-Vita decryption
- The dump is the EU retail Vita game: `param.sfo` (plaintext, outside the SELF) says
  `EP0006-PCSB00183_00-NFSMW000VITA00EU`, version `01.00`, `c_date=20120924`, `sdk_ver=01800000`. 2.5 GB,
  3108 files, dated 2022-02-17.
- **Nothing in the dump is decrypted.** `eboot.bin` has no `SCE\0` magic, entropy 8.000 (pure random);
  `eboot.elf` is just a renamed copy of the same still-encrypted eboot (not a converted ELF). `.suprx`
  modules and every `.BNDL` are encrypted too (no `bnd2` magic → `mwtool list` rejects them). This is a raw
  PFS-encrypted image: `sce_pfs/{unicv.db,files.db}` present, NPDRM key at
  `sce_sys/package/work.bin` (1024 B).
- **The PSVita-RE-tools cannot decrypt this on the PC.** `vita-unmake-fself` (the only PC tool) explicitly
  cannot handle NPDRM/System-encrypted SELF (its README). `FAGDec` — the tool that can — is a PS Vita
  homebrew `.vpk`; it runs on a modded console, not here. There is no offline PC key path in that repo.
- Two encryption layers to undo, both console-side: (1) **PFS** on the whole filesystem (gives plaintext
  `.BNDL` assets — then `mwtool` reads them like the PC bundles, which are `bnd2` v5 **platform 1 (PC)**; the
  Vita bundles may differ in platform/endianness, TBD once readable); (2) **SELF** on `eboot.bin` + `.suprx`
  (FAGDec → ELF, an ARMv7 binary statically analysable in Ghidra/radare2, unlike the encrypted x86
  `NFS13.exe`). The Vita eboot is a legitimate second reference for handling/physics per the fidelity rule.
- Decrypt procedure recorded for when a modded Vita is available: use FAGDec's `app_em` PATH_ID mode —
  place the game under `ux0:/app_em/PCSB00183/` with `sce_sys/package/work.bin` in place, then decrypt
  `eboot.bin` and the `sce_module/*.suprx` to ELF; PFS-decrypt the `.BNDL` set separately (FAGDec app_em, or
  a NoNpDrm/`psvpfsparser` dump). Drop the decrypted ELFs + bundles into `re/vita/` (git-ignored, like the
  PC dump) for analysis.

## Open questions

### Textures
- Header flags at 0x20: values 0x00/0x10/0x20/0x30. Is one of them sRGB? (Currently all colour textures are
  treated as sRGB.)
- Byte 0x2C ranges 0–10 and does not mean missing mips (full chains are present). Streaming LOD bias?

### Renderable
- Mesh bytes 0x24–0x27 (sub-part code, then 1, 0, 4 in samples).
- The compressed OBB at mesh+0x00: the exporter documents its quantisation. Decode it for culling.
- Buffer descriptor words `{0, 1, 3|2}`: D3D11 bind flags?
- Is the SNORM position scale always 10.0, or does it ever vary per mesh/renderable?

### Vertex data
- `Color` (RGBA8): roads = baked light (rgb² × GlobalLightMapIntensities.x × 0.318) + AO (a); vehicles = AO (x)
  and specular occlusion (y); trees = AO (y). (Answered 2026-10-04 for these families.)
- World layouts often have Tangent + Binormal but no Normal (cables): is the normal `cross(T, B)`, and with which
  sign?

### Materials / shaders
- Lightmap shaders sample `LightMapTextureSampler` at TEXCOORD1, but no material or instance override binds it.
  Where does the texture come from (a separate resource per track unit, GeneSys, runtime)?
- Bevy's clear coat now renders on vehicles (they have tangents since 2026-10-04).

### World
- InstanceList instance `unknown_04` (f32) and `unknown_08` (u32).
- ZoneHeader (0x206, one per track unit, version 9): import slots at 0x04 InstanceList, 0x0C
  DynamicInstanceList, 0x14 LightInstanceList, 0x1C CompoundInstanceList, 0x24 PropInstanceList, 0x2C one
  GroundcoverCollection, then `u32 count` at 0x34 and `ptr` at 0x38 to more GroundcoverCollection imports
  (2026-10-04, TRK_UNIT1). No visibility data in it. Not parsed yet.
- How the game draws near water (city rivers, the airfield lake): no zone lists a water mesh over the river
  next to downtown (unit 13), and no main geometry covers it. A separate water system? (`--backdrop-zone 13`
  shows the gap.)
- ZoneList zone byte 0x40 (0 or 1) and the header u32 at 0x18 (3).
- What does `WORLDLIST.BNDL` contain, and how does it relate to `HAWAII/MAPS/*.BNDL`?

### Data
- GeneSys field names: the hash is known (2026-10-04). About 1,200 of 5,520 distinct field hashes are named;
  the rest need better word lists. `655d3f5f` (the `CgsCore.UniqueId` at +4 of every object) is unnamed.
- The 64-bit type hash at type+0x10: check it against `field_hash` of the type name (and a 64-bit variant);
  it may also be a layout fingerprint.
- GeneSys: three objects per vehicle bundle still fail to decode (pointers read as huge offsets).
- AttribSys: no `AttribSysSchema` / `AttribSysVault` resources exist in the install (checked 2026-10-04).
- Drift (2026-10-06, from the code): every field's role is settled (decomp.md "Drift"). Still open: the speed
  hold's `.bss` constant 0x01187250, the counter-steer `powf` exponent (taken as `Max_counter_steering_power`),
  and whether the outward side force reproduces the original's drifts (needs a recording).
- Tyre data not yet understood: `Lateral/Long_Slip_Factors`, `Grip_factor` / `Sliding_grip_factor`,
  `Available_grip`, and the lateral curve's fourth field (0.5 everywhere). Five more `Tyres` floats per axle
  (400, 1000, 2) are unnamed.
- VehicleList entry fields beyond ID / names / year (floats look like stats; u16 near 0x6A may be RPM).

### Environment and weather
- What switches to the `Alternate` keyframes (7 hazy twins, one per time-of-day band; no timeline lists
  them). A gameplay state (events, pursuit), a random weather roll, or a menu option? Nothing in GeneSys
  references them except the keyframes.
- `Weather.Thunderstorm_Intensity` (0.23 on night keyframes): how the game shows it (no lightning shader,
  texture, effect or sound was found). `Weather` `0141d842` (0.25 day / 0.15 evening, night and Alternates)
  and `e8eb5d48` (0 everywhere) are unnamed.
- Who reads the wind (`WindBearing` 130° ± `WindBearingVariability` 25°, `WindSpeedMin/Max` 1 / 3.5)? The
  tree shader's sway is per material and only time-driven; cloud offsets are CPU-side (`Clouds.Speed` 5).
- `WetScaleParams` values (puddle tiling, reflection distortion, reflection blur): CPU-set; ours are guesses
  (`materials::WET_SCALE_PARAMS`). The planar reflection plane height and its "Modified" projection.
- When surfaces use `SurfaceType.Wet_Conditions` (wet tyre marks, spray, sounds) instead of `Dry_Conditions`.
- `EnvironmentSettings.DayNightTransition` (`1ab46d30` 8.5, `0e2be130` 20, two (1, 0) pairs) and the
  `Rendering` block (sky material handle, `MoonDiscAlpha`, `SunDiscAlpha`, `31c63004` 180, 3.5 × 2).
- `WorldPaintMap` header u32 at 0x24 (4878 / 1926 / 94).

### Vehicle damage
- How the game turns an impact into zone damage: which `ImpactDamageGraphs` curve (two of them; the second
  has its low point the other way round, `(0, 6 %)`?), what units (mph assumed), how damage is split between
  zones (we use the nearest zone plus a 60° falloff), and what `CrashingRules` / `CollisionInfoDamageProfile`
  / `DamageBarProfile.ImpactLocationDamageScale` / `Physics.LandingDamage` add. Global objects in
  `PHYSICS/PHYSICSCONFIG.BNDL` and `GAMEPLAY.BNDL`.
- `VehicleDamageBehaviour.Bodypart`: the hinged parts (doors, bonnet, bumpers, mirrors) presumably swing
  physically once their `DamageThreshold` looseness exceeds 0. Which bone does each hinge drive? (Hinge
  positions don't coincide with bone positions; bone names are unknown.) What are the three spring-like
  Vector3s (`99e38802`, `19beda43`, `13ac399e`), `9a601fda` (rear bumper `[-3, -5, 0.35]`) and the
  `DamageThreshold` value `31ad3690`?
- `ControlMesh` (GeneSys, one per car, five Vector4s like `[2, 5, 0.2]`, `[0.01, 0.75, 0.75, 0.85]`): a
  separate resource type 0x210 exists in the type list; is it the crumple / damage-volume tuning? The
  car's `0x108e7a` GeneSys object (truncated decode) may be related.
- `WheelDamage` (four per car: steering angle −20..30°, camber-like `b44cbfcf` −10..−20): damaged-wheel
  alignment, not applied yet.
- Damage volume grid (3 × 5 crumple values per car) for the crumple normal map: the shader side is known;
  how the game fills it from the zone levels is not.
### Particles (BearEffect)
- The emission controllers: 0x33 / 0x34 decoded 2026-10-09 (`Bindings`); 0x42 (`UpdateWheelBindings`, 4 inputs)
  is not, and the binding-slot bytes of the wheel and collision spawners (0x05, 0x07, 0x01) are not: those still
  use curve 0 over `Intensity`.
- What the game writes into `WheelParams` (0x40 bytes), `VehicleParams` (0x30), `CollisionMeshParams` and
  `WheelContact`, and what it means by `Velocity` / `Intensity` for each effect (code).
- Spawner fields: the three angle ranges of the wheel spawner (0x05: `180,180 | 0,180 | 0,360`), its
  `0.9..1.2` range, the 0x07 `0.25 ×3` triple, the u32 flags at the end of every spawner.
- `Bounce` / `Integrate` drag (0–0.999) and the two trailing floats; how bounce planes come from
  `VehicleParams` / `WheelContact`.
- `ViewFade` (0x38): its 12 leading floats (all 1 but one 0.6462) and the `20 51` / `20 40` pair.
- Renderer words: the first float (0.01–4, read as a soft-particle fade distance), the rgb(a) after it,
  the trailing u32 flags; the mesh renderers (0x1E/0x1F/0x22/0x23) and their Model imports; 0x4D
  (`VfxSplatter_Dynamic`).
- `VfxVehicleCollisions` (36 handles: 2 empty + 16 effects, twice) field names, and which collision picks
  which slot. `SurfaceType` weather conditions `f395b019` / `7348be25` (dry / wet by their contents), the
  `VfxWheelEffects` fields `61ac1863` / `e7a7150c`, and the rest of `SurfaceVfx`.
- `gVfxSparkBrightness` (spark shader global) and the particle VS's exposure term
  (`lerp(1, exposure.w, gVfxConstants.x)`): `VfxGlobals.Rendering` has `ExposureScale` 0.4,
  `ExposureScaleCoronas` 0.4, `ExposureGamma` 2.2 (cracked) and unnamed `dacaf024` 0.4, `a77107f8` 10.
- Collision surface ids (`PolygonSoupList` tags) → `Surfaces.Surfaces` index, so dirt / grass get their dust.
- World effects: `mwtool genesys all --type WCVfxBehaviour` finds 3,931 objects (3,911 in `TRK_UNIT*`): 2,302
  have `Spot_effects` (`Spot_effect` handle + `Locator_group` id; e.g. `…24f3d`, `…eee1b`, `…af136`), 1,095 an
  `ImpactEffect` (`…24f3d` 613×, `…1621e7` 410×: prop smashes), most have `Lights` (`LightDefinition`) or
  `Coronas`. How a world locator group resolves to positions (compound / world object parts?) is open; a car's
  resolve in its `VehicleGraphicsSpec` (2026-10-09). The exhaust effects are the cars' spot effects (2026-10-09).

---

## Findings

### 2026-10-07: the PS Vita build (PCSB00183, EU, 1.00): encrypted copy, not analysed

The user copied their Vita install (`re/vita/PCSB00183`, git-ignored) as a second reference for the physics
code. The copy is the encrypted on-console form: `eboot.bin`, the `.suprx` modules, every `.BNDL` and even
`sce_sys/icon0.png` have 8.0 bits/byte entropy and no magic (no `SCE\0`, `\x7fELF`, `bnd2` or PNG header). Only
`param.sfo` is plain (title ID PCSB00183, content ID EP0006-PCSB00183_00-NFSMW000VITA00EU, APP_VER 01.00).
Decrypting it would mean getting around the console's DRM, which the project does not do
([reverse-engineering.md](reverse-engineering.md)), so nothing was read from it. It becomes useful only as a
plain ARM ELF supplied by the user, the same way as the PC dump. Why it might help then: a second compiler's
output of the same handling code (different inlining, maybe leftover symbols or asserts) could name or confirm
functions the PC dump leaves open, such as the writer of the steering's neutral offset.
A later `eboot.elf` (4,472,368 bytes, 4 KiB shorter than `eboot.bin`) is still ciphertext: 8.0 bits/byte across
the whole file, no ELF or SCE header anywhere, no readable strings. It is byte for byte `eboot.bin` with its
first 4096 bytes cut off. The tools on gamebrew's "PSVita RE Tools" page fall into two groups. FAGDec,
vita-unmake-fself and the kernel dumpers do the decryption step, which is the DRM step, so they are not used here.
VitaDecompilerMod, prxtool and the psvitalibdoc NID tables do the analysis, but only on a plain ELF.

### 2026-10-07: chase camera weight: the eye's vertical spring and the roll ported, yaw stiffness read

- **Method**: the decompiler drops arguments and locals on the camera helpers, so `FUN_005d12b0`'s vertical
  stage, `FUN_005dc270` / `FUN_005dbbc0` / `FUN_005bd470`, `FUN_005c9380` and `FUN_005c90a0`'s tail were read
  from `disas.py snap2`, constants with `exe-const`, and every rule replayed on `ref/live1` before porting
  (decomp.md "The eye's vertical spring, roll and yaw stiffness (round 11)").
- **Vertical spring** (cam + 0x550): a clamped damped spring (K 20, ζ 0.8 / 0.75, m 1) kicked by the shaped
  vertical acceleration on the ground and by the vertical speed in the air, with a 0.3 s landing window
  (cam + 0x716 / 0x6b0, written at the end of `FUN_005d4970`). It moves the eye only, not the look-at point.
  Replay 5.8 mm RMS, correlation 0.999 (recorded std 0.14 m). The `.bss` 0x0118a060 (2.23694) only turns the
  horizontal speed into mph for the airborne gain.
- **Roll** (cam + 0x660): lateral acceleration is m/s² (the range 10-30 is 1-3 g, so the yaw-rate term does
  most of the work), the rate is intensity × π/180 × 20 = 0.70/s, and the camera leans out of the turn.
  Replay 0.023° RMS with the 911's `Camera_Roll`, so the recorded car shares it.
- **Sign trap**: the game's car and camera row 0 is forward × up (the car's right as rendered), i.e. Bevy's
  local −X for our proper rotations; the camera matrix's row 0 is screen-left. Check signs on the recording
  (yaw rate vs heading change) before trusting a row's name.
- **Yaw stiffness**: the moving part of the old fit is code (K1 = lerp(`63fb2207`, `5a74d44d`, min(cam + 0x6d8,
  speed fraction))); a further blend `s = clamp(|vtable+0x48 vector| / _DAT_0110e8f0)` towards
  `FUN_00574870(0x29)` is unresolved: no motion signal tried (slip, lateral speed, yaw rate, world velocity)
  explains the 0.002 residual. Dead end for now; a live read of `_DAT_0110e8f0` and the vtable's +0x48 target
  would settle it.
- Not done: the deceleration pitch (`Deceleration_Pitch_Change`), the standing yaw value's producer.

### 2026-10-07: bloom ported

- **Passes** (decomp.md "Bloom passes (ported)"): three RGBM levels (exe downsample programs, 4 taps at ±1
  source texel on both diagonals, `kInvWidthHeight` from `FUN_008e6470` = the source's texel size), 5-tap
  horizontal blur into a float target ÷ T.w, the add pass (vertical blur × z + coarser × w), coarse to fine
  with the `FUN_008e6f40` weights (minimum `DAT_00f9d474` = 0.01 from `.rdata`), and the `GBufferComposite`
  term with B = (`BloomColour`, `99436e68`) of the time-blended environment keyframe. T.w in our tonemap is
  1.442695 × 2^(e + bias) (`_BuildConstant`), which both the downsample and the composite's saturate use.
- **Level sizes not traced** (model decision 1/4, 1/8, 1/16): the targets come through `*(renderer + 0x220)`
  +0x130…+0x148 (call at `0x008cc828`, edx = renderer +0x220, or +0x254 in a mode tested at `0x008cc66b`).
  Dead end: in the renderer dumps that pointer is 0x04ca25f0, but `snap2` is another run (the address holds
  other data there), and a byte-signature search for the renderer in `snap2` (40 bytes of static pointers
  from the dump) matched hundreds of places. A live read of the struct settles it.
- **Check** (no capture of the original exists, so only plausibility): `--camera=-1131,27,1627,…`, mean
  displayed linear luminance below the text lines, on / off: noon (12.3 h) 0.272 / 0.245 (+11 %), 1.5 h
  0.089 / 0.081 (+10 %); bright clouds and the sky behind the trees glow, nothing clips to white. The
  exposure is metered before the bloom, so it is the same in both. At 1.5 h this view shows no lit street
  lamps, so the night glow of point lights was not seen. Frame time (`--bench 10`, RTX 3070) 16.61 ms on vs
  16.68 ms off: within vsync noise; the passes don't record GPU spans, so per-pass cost was not measured.

### 2026-10-07: the chase camera's "speed shake" split into its sources; the high-speed shake ported

- `ref/live1` decomposed (decomp.md "High-speed shake and the eye's vertical spring"): with the impact
  shake idle, the view's pitch residual against the followed frame is a matrix at cam + 0x360 (correlation
  0.98), the eye's height residual is cam + 0x550 (`FUN_005d12b0`'s vertical spring, 0.995). Body pitch passes
  only weakly (the followed frame's pitch is 0.05° RMS high-passed, the body's 0.15-0.2°).
- `High_Speed_Shake` read in `FUN_005d0b90` → `FUN_005e34e0` → `FUN_005e2a70` / `FUN_009e7a30`: a 199-sample
  quaternion curve (`.bss` 0x01188300, ICE `SHAKE_DATA`, filled at run time) ping-ponged at 30 samples/s ×
  Frequency, scaled by Amplitude × f^Power / 10, plus a pitch bias, turning the view about a point 10 m
  behind the eye. Ported with a stand-in curve of the recorded statistics (model decision); the replay
  matches the recorded std per speed band (0.030° / 0.068-0.071° at 40-80 / 80-140 mph).
- Open: the curve's own samples (where the table is filled: not in the dump; the ICE strings `SHAKE_DATA`,
  `SHAKE_QUAT_*`, `SHAKE_POS_*`, `Shake Type` at 0x0107c4xx..ccxx are the lead); the speed input S (the recorded
  scale is ~30 % below the data law under 60 mph); why the random walk at cam + 0x440 never moves; the
  vertical spring's input and `.bss` scale (needs `disas.py` on `FUN_005d12b0`). Roll (`FUN_005c9380`), the
  deceleration pitch and the yaw-stiffness producer were not reached this round.

### 2026-10-07: the chase camera's impact shake read and ported

- `FUN_005dfe90` read: a noise spring (game LCG, ±`WobbleMagnitude` kicks, step `Maximum_frequency_multiplier`
  × dt) plus a ±`ShakeMagnitude` jitter, times `Maximum_amplitude` × level, as view angles in degrees.
  `FUN_005cd040`'s arguments mapped to the `Impact_shake` fields through the call in `FUN_005d0b90`; the
  decompiler drops one argument there (the body), so the mapping was settled with the disassembly of the call
  site and by matching the recording (decay 0.835 / frame, x autocorrelation 0.5, x std 0.167, jitter uniform
  ±1). Details: decomp.md "Camera effects".
- Ported (camera.rs `ImpactShakeState`); the guessed `CameraGameplayShakeEffect` springs are gone.
- Open: contact + 0xc's unit (taken as m/s normal speed), the landing request's |v| units (`FUN_005c8e40`, not
  ported), `FUN_005d9740`'s Euler order and the rotation pivot; the speed shake (the level at speed is ~0.01,
  so the impact path does not explain the 0.2-0.3° wobble at 40-80 mph; not found); the strings search for
  other camera effects was not done.

### 2026-10-07: free-roam bloom comes from the environment keyframe, not PostFX

- **Source of the renderer's bloom block** (+0xac74, decomp.md "Post-processing" has the chain): the copy's
  source is `FUN_008ce250`'s local filled from the PostFX manager +0x1300…+0x1330 (`FUN_008e3b50`; found by
  counting the pushes, the decompiled argument lists of both calls are off by one). `FUN_008ecf80` blends
  the PostFX keyframes there and then, when the environment override flag +0x15e0 is set, overwrites every
  word but 7 from +0x15f0…+0x1628, which `FUN_008e3e30` fills from the blended environment state
  (`FUN_008c2080` / `FUN_008c0f30`, +0x24d0…+0x2510).
- **Value match**: live colour (0.292, 0.474, 0.604) = `EnvironmentKeyframe.Camera.BloomColour` of 29
  keyframes (the noon set, e.g. `…124575`); B.w = `99436e68` (0.387…0.599 by day, live 0.595 / 0.510);
  words 9–11 = `9dd29928` (0.009) and 1, 1 (`2b7c4634`, `e59ff4ec`). The research log's earlier note that
  the PostFX `Bloom` unpack order puts floats at integer slots is resolved: in free roam those words are not
  the PostFX keyframe's at all. The integer words (4, 5, 5, 1) have no float Camera field: runtime values.
- **Conflict to check**: `e59ff4ec` is also used as the exposure adaptation speed (`CameraExposure.speed`,
  role inferred from the day curve). If it is a bloom weight, the speed field is something else.
- **Dead end**: the minute-long drift of the colour and weights is not a lerp between two keyframes of the
  noon timeline (colour ≈ 25 % towards the orange keyframes, B.w and weights inconsistent with that); more
  inputs are blended (regions, weather). Not followed further.
- **Level combine** traced (`FUN_008e6f40`): C3 = blur(L3), C2 = (w10·blur(L2) + w11·C3) / (w10 + w11),
  C1 = (w9·blur(L1) + (w10 + w11)·C2) / Σ; the add program `0x010117c0` blurs t0 vertically with the 5-tap
  kernel and adds t1 × `kColorAddWeight.w`. Which weight is z or w is inferred from the first call.
- Data parsed and blended (`CameraExposure.bloom_*`); render passes not ported this session (budget).

### 2026-10-06: the live-read `.bss` values applied to the handling code

Each site re-read in the dump with the value in hand:
- **Clutch k** 0x011873e0 = 2π/60: `FUN_009d7ac0`'s Δω is engine rpm (+0x20) minus the driveline's rpm
  (`FUN_009d55f0` multiplies by 60/2π), so k converts it to rad/s. Ours already worked in rad/s: no change.
- **Standstill hold** 0x01186e60 = −2 (mph, the scale 0x0118a060): the hold acts only when rolling backwards
  faster than 2 mph, never at rest. Ours was 0; now −2.
- **Downshift gate** 0x01187580 = 1 s on the ground; 0x01186630 = 1.8 is a speed (body |v| in m/s,
  `FUN_009ae840`): the ground timer resets only with no wheel down *and* the body faster than 1.8 m/s. Ported.
- **Clutch-in throttle**: the disassembly (the decompiler lost the `pow` arguments) shows the exponent is a
  different vector, 0x01187470 = 0.5, and 0x011874d0 = 0.7 is a scale: upshift (`FUN_009d6af0`) throttle =
  0.7 √progress with no pedal term, downshift (`FUN_009d6ca0`) = pedal × 0.7 √progress. Ported (was pedal ×
  progress).
- **Stall** 0x011874a0 = 0.9: rpm < 0.9 `Idle_RPM` drops to standstill (state 2) in the launch (`FUN_009d17d0`)
  and in `FUN_009d6880`, `FUN_009d95a0`, `FUN_009d6af0`, `FUN_009d69e0`, `FUN_009d6ca0`, `FUN_009da1d0` (not the
  downshift declutch `FUN_009ce490`). Ported.
- **Kick-down window** 0x011871e0 = 0.2 s (was 1 s). **Punch** 0x0118a018 = mph → m/s (`Punch.0798d257` is
  mph/s); `nitrous::PUNCH_SCALE`, modes 1 / 3 still not run. 0x01187250 = 60 confirms the time bases.
- **Tyre brake hold** `FUN_009bc0c0`: the vector 0x01186b70 is 9.81 in every lane; only its y lane is used: the
  slip speed is taken from the contact velocity with `param_8 × 9.81` off its vertical component, i.e. the
  velocity after this tick's gravity if `param_8` is dt (the 4th argument of `FUN_009c6f00`, not traced). Not
  ported.
- **Strut stop**: share to the chassis = max(d − min(+0x470, 0.1), 0.1 d) / d (0x01186bf0, 0x01186b10 = 0.1).
  Equal to ours when the constraint's +0x470 is 0. Dead end: the chain `10edb04,4,53220,184,0` lands on the
  vehicle (vtable 0x01023758), and the wheel constraint (+0x210 blend-input scale, +0x470) was not located from
  it in the time available; both stay open.
- Skid sound: 0x0118a060 confirms mph; 0x011870d0 = −0.1 is compared with +0x108 of an unidentified wheel
  object in `FUN_009d1bf0` (a signed value, likely travel / contact distance). 0x01186f60 (0.5) not looked at.
- Telemetry `ref/live1`: the upshift clutch-in lasts 3–4 ticks (clutch 0.25 → 1); the sound load (lagged throttle)
  stays below 0.7 √progress during it and passes 0.7 only after it, consistent with the 0.7 cap but not a proof
  (the load is lagged).
- 911 tests: accel, brake, corner, handbrake, drift, engine unchanged within 0.05 s; nitrous 0–200 km/h
  10.33 → 10.87 s (the upshift clutch-in throttle is now 0.7 √progress); landing / climb fine (climb
  overtravel 0.119 → 0.040 m).

### 2026-10-06 (session 5, late): vignette ported; bloom's live block does not fit the keyframe

From the 20 renderer dumps of `ref/live1/ren` (3 s apart) and the snapshot `snap2`; details in decomp.md
"Post-processing".

- **Vignette, solved except two scalars.** D = (inner × s, k), A = ((outer − inner) × s, 1 − k), the vertex
  shader's constant = (centre, `Scale` × k), all from `PostFxKeyframe.Vignette` of the default keyframe
  `…78dc` (A.rgb / D.rgb = −0.54921 = 0.45078585 − 1 in every dump; centre (0.5, 0.3), `Scale` × 1.8245 =
  (1.2771, 1.0126)). k = 1.8245 is constant over all dumps, including one under another PostFX state; s moves
  slowly between 1.0025 and 1.0385 (2.81 in that state). Neither is explained: `Sharpness` = 0.34 fits no
  simple formula tried (1 / (1 − x), (1 + x)², …), and no renderer instruction stores to +0xa980 / +0xa990 by a
  literal displacement (byte search of the snapshot's code, 0x880000…0x900000), so the writer uses another
  base. The add pass's alpha is v = smoothstep(saturate(|p| + A.w)) (PS `0x010117c0`), the VS (`0x01011e18`)
  takes p = (rotate(2·uv − 1, angle) − 2·centre + 1) × scale with uv the D3D texture coordinate (y down, so
  the centre sits 30 % from the top and the bottom corners darken most). **Ported** in `tonemap.rs` /
  `tonemap.wgsl` with k as a measured constant and s = 1 (model decision); `MW_NO_VIGNETTE=1` turns it off.
  Check at the `--camera=-1131,27,1627,…` view, noon (displayed linear luminance, on / off): centre 1.00,
  top-left 0.78, top-right 0.63, bottom-left 0.51, bottom-right 0.31 (after the tonemap curve and the colour
  cube, so not the raw factor; the raw factor is 1 at the centre, 0.45 where v = 1, i.e. both bottom
  corners, and ≈ 0.66 at the top corners).
- **Bloom, not ported.** The copy into +0xac74 is at 0x008ccbdc (`FUN_008c9e40`, x87 load/store of 13 words,
  sets bit 1 of +0xaa9c). Four of the words are integer bit patterns (4, 5, 5, 1, sliding to 1, 1, 1, 0 in a
  transition), the colour drifts linearly over the minute (0.342 → 0.417 red) and B.w (+0xac84) 0.595 → 0.510;
  the default keyframe has a grey 0.402 colour and floats at those slots. So free roam blends other PostFX
  keyframes over time (the selector, presumably a PostFX timeline, is not known), and the unpack order of
  `FUN_008e51b0` noted below puts float fields where the live block holds integers: the field mapping is not
  confirmed. No PostFxKeyframe has a Colour near the live one. Next: disassemble the source of that copy (its
  `edi`) and the PostFX state's keyframe selection from the snapshot.
- The colour-cube slot weights were not reached.

### 2026-10-06 (session 6): fog, groundcover size, top mip and the sun from the live dumps

Read across all 20 `ref/live1/ren` dumps (slot records per dump, car positions from `ren.log`).
- **Hour of the run**: slot 0x0d (key light direction) goes from (0.411, −0.765, −0.495) to (0.441,
  −0.760, −0.477). With `BearingCentre` −232.37 / `BearingVariation` 35.06 / `BearingPhaseOffset` −0.5 the
  bearing atan2(z, x) of the to-sun vector (129.7° → 132.8°) gives **~12:13 → ~12:34**, rising as it should.
  Our old convention (bearing from +Z towards +X) puts that vector at −39.6°, outside the reachable
  92.6°–162.7°; the two other sign variants either are unreachable or run backwards in time. Fixed in
  `SunMoonPath::key_light_direction` (x = cos b, z = sin b): looking from (887, 27, −555) towards +X/−Z
  at 12:00 the sun moved from in front of the camera (glare on the road) to behind it. The elevation (49.9°
  → 49.5°, falling) means the arc peaks near 12:00; our sine arc between 05:00 and 21:00 peaks at 13:00
  (open, not changed: no code source for rise / set yet).
- **Fog `ScattCoeffs`**: computed in `FUN_008c0f30` with a constant scale of 1 (decomp.md "Fog"), so near /
  far are **not** scaled. Live (0.0007576, 0.06061, 2) at (865, −578) = 80–1400 m, Power 2 = exactly what
  our timeline `1245c8` gives there from 10:00 to 14:00. Driving north (dumps 13–16) the slot slid to
  80–1171 m, Power 1.52 over ~10 s: the original blends between painted timelines in space / time, we switch.
  So the "flat fog beyond 500 m" view is a timeline whose keyframe really reads 500 m; what keeps the
  original's hills visible must be the gradient alpha or the layer term, not near / far. Dead end: renderer
  +0x1ef80 and +0x2110 in snap2 hold no fog values (the `this` of `FUN_008c0f30` is another object).
- **`LayerFogCoeffs`**: formula traced (decomp.md); w = z / (1 + 2^y) holds in every dump. Not ported: the
  entry weights and the density factor (0.53 and rising at the run's place) are untraced, and the shader
  term needs them.
- **Groundcover**: the vertex program reads only `cb1[0].x`, size = BINORMAL.xy (byte / 127) × it, the quad
  turned about Y to face the camera. Slot 0x7f = 4 in 15 dumps, 0 in 4, 256 in 2 (other passes reuse the
  slot). Size now byte / 127 × 4 (was byte / 32, 0.8 % larger).
- **Top mip**: `FUN_00409510` passes the "start at mip 1" flag only when `DAT_010eac98` is set and
  `FUN_00414a50` (+0x113) returns **0**; the earlier note had it inverted. `FUN_0086cf50` / `FUN_0086cf80`
  map +0x113 to a settings option (1 = option 1, reload on change). The live value 1 therefore means full
  resolution: the original does not drop the top mip at the user's settings, ours already loads mip 0.
  The "scaled wrong" road report needs another explanation.

### 2026-10-06 (session 5, late): live-read session against the running original

The user ran the original (free roam, about five minutes: standing, then driving up to 134 mph, then standing);
everything below was read from its memory, read-only. Reads through `/proc/<pid>/mem` worked with
`kernel.yama.ptrace_scope` = 1 on this machine (no sysctl needed this time).

**What was saved** (git-ignored; raw memory never leaves `re/out` / `ref`):

- `re/out/live/snap1`, `snap2`: full snapshots of the process's readable memory below 4 GiB (1.4 GB each;
  `mem.bin` + `index.tsv`), standing and at speed. They hold the unpacked code as well, so call sites the
  decompiler lost (`this` in ECX) can be disassembled, and every `.bss` value and object can still be followed
  after the game has quit. Helpers in the git-ignored `.claude/tools/`: `live.py [-s snap] <va|chain> [n]`
  (chain = `10edb04,4,53220`: deref then add), `disas.py <snap> <va> <len>`, `slots.py [-s snap]`,
  `snapshot.py`, the recorders `camrec.py` / `renrec.py`, the loader `camload.py`.
- `ref/live1/telemetry.csv`: the handling watch list at 120 Hz (35,610 rows).
- `ref/live1/cam.bin` (+ `cam.json`): 30,983 records at ~120 Hz of the whole camera director (0x3050 bytes), the
  player vehicle object +0x1600…+0x2d00, its rigid body block (0x100) and the per-player camera state (0x200).
- `ref/live1/ren/ren_NNNN.bin`: 20 dumps, 3 s apart, of the renderer object (0x20000 bytes from renderer − 0x80)
  and every shader constant slot; `ren.log` has the car position and speed of each.

**Objects found** (addresses are this run's; the route is what matters):

- The **frame object** `G` (`this` of `FUN_0046f730`, called from `FUN_00476f20`): found by scanning for
  `[G+0x2db10c] == 7` and `[G+0x623afc] ∈ {1, 2}`; no global pointer to it is known yet. The camera module
  (`DAT_010edaf4`) lies inside it.
- The **renderer** (`this` of `FUN_008c9e40` and of `FUN_0089d5c0`, i.e. their `param_1`) = `G + 0x2bc400 + 0x80`
  (disassembly: `FUN_0046f730` passes `G + 0x2bc400` to `FUN_008ce250`, which passes `this + 0x80` on). Its
  render mode at +0x1ed0c − 0x80 read 7.
- The **shader constant context** (`this` of `FUN_00a5d660`, in ESI in `FUN_0089d5c0`) = `*(renderer + 0x2e50)`:
  `ctx[slot]` points at the slot's float4s (count per slot in the table at `0x0118cec8`). Slots are rewritten
  by each pass, so a read shows what the last writer left (a mid-frame snapshot shows another pass's values).
- The **chase camera** is where decomp.md said: director = `*(*0x010edaf4 + 4) + 0x6a0`, chase camera at
  +0x8d0 (vtable `0xf999b4` confirmed). Camera matrix at camera +0x10 (rows right / up / back, position at +0x40),
  the followed car's frame at +0x4d0 (position +0x500), **field of view in degrees at +0x90** (70 at rest,
  102.7 at 134 mph in this run).

**Static values** (`.bss`, initialised at start-up; vectors repeat the value four times):

| address | value | what the code uses it for (decomp.md / handling.md) | we assumed |
|---|---|---|---|
| 0x011873e0 | 0.10472 (= 2π/60) | factor in the clutch torque of `FUN_009d7ac0`: an rpm → rad/s conversion, not a tuning scale | 1 |
| 0x01186e60 | −2.0 | standstill hold speed limit | 0 |
| 0x01187580 | 1.0 | seconds on the ground before a downshift is allowed | 0 |
| 0x01187250 | 60 | per-tick → per-second factor | 60 |
| 0x011874d0 | 0.7 | clutch-in throttle exponent | - |
| 0x011874a0 | 0.9 | stall fraction | - |
| 0x01186b74 | 9.81 | the y term in `FUN_009bc0c0` (gravity) | not modelled |
| 0x0118a060, 0x0118a090 | 2.23694 | m/s → mph | - |
| 0x011870d0 | −0.1 | load floor of the tyre spin sound ramp | - |
| 0x01186630 | 1.8 | unidentified (drivetrain entry) | - |
| 0x01186f60 | 0.5 | unidentified (drivetrain entry) | - |
| 0x01186bf0, 0x01186b10 | 0.1, 0.1 | strut stop slack | 0 |
| 0x0118a018 | 0.44704 | mph → m/s (the nitrous punch "scale" is a unit conversion) | - |
| 0x011871e0 | 0.2 | kick-down time window, seconds | 1 |
| 0x0110f590 | 3600 | traffic cull radius squared (60 m) | - |
| 0x0110e8f0 | 45 | camera: `_DAT_0110e8f0` of the yaw spring (`FUN_005c90a0`) | - |
| 0x0110e4e0 | 0.9848 (cos 10°) | camera: `_DAT_0110e4e0` of `FUN_005c7a80` | - |

**Renderer values while driving in daylight** (renderer offsets as in decomp.md "Post-processing"):

- **Bloom is on in free roam**: the flag bytes at +0x9834…+0x9837 are all 1 and bit 1 of +0xaa9c is set. The
  set at +0xac74: colour (0.292, 0.474, 0.604), then 4 (integer), 0.5, 5, 5 (integers), 1.0, 1 (integer),
  0.009, 1.0, 1.0, 0.
- **Vignette**: D (+0xa980) = (1.0065, 1.0065, 1.0065, 1.8245), A (+0xa990) = (−0.5528, −0.5528, −0.5528,
  −0.8245). The composite multiplies by A·v + D, so the factor is 1.0065 where the vignette weight is 0 and
  0.4537 where it is 1 (the keyframe's inner 1 and outer 0.45, stretched by about 1 %); the alpha pair gives
  1.8245 → 1.0. The neighbours +0xa9a0…+0xa9bc read (0, nan, nan, nan) and (0.5, 0.3, 1.277, 1.013).
- **Constant slots** at one moment (car at (887, 24.5, −555)): 0x23 `ScattCoeffs` = (0.000895, 0.0716, 1.502,
  0.4); 0x24 = (0.2808, 0.4115, 0.4959, 0.25); 0x27 = (0.96915, 0.00412, 0, 0); 0x83 `LayerFogCoeffs` =
  (0.1512, −6.888, 0.0826, 0.0819); 0x7f `GroundcoverConstants` = (4, 4, 4, 4); 0x0d key light direction =
  (0.397, −0.766, −0.505). The other 15 dumps hold the rest of the run.

**Other one-off reads**:

- The object at `0x010eac98` (pointer `0x014de500`), byte +0x113 (what `FUN_00414a50` returns) = **1**.
- The GeneSys object at `DAT_0110f940`: type word `0x00069444`/`0x00069445` near its header, data
  (`*(obj + 0x28)`) floats from +0x10: 4, 50, 10, 0.25, 0.4, 0.7, 2, 80, 6, 25, 0.6, 0.649, 0.3, 8, **0.24
  (+0x48)**, 4, −20, 15, 20, 14, 25, −8, 20, 2, 0.05, 5, 0.05, 0.5, 0.15, 0.5, 2, 2, 0, 8, 4, 10.
  `DAT_0110f944` = 0.
- The player index was 48 again (vehicle table slot offset 0x184).

**Traffic, from the snapshots** (after the game had quit):

- `DAT_0110f940` **is `TrafficParamConfig`** (`0100000000069444` in `GLOBALCONFIG.BNDL`): all 36 floats read
  live equal the data file's fields in order (field +0x48 = 0.24). The inference in decomp.md is confirmed.
- The traffic system object (`world` in decomp.md's traffic section) was found by searching for the copy of
  `TrafficSystemConfig`'s 4.78 outside the resource: at `world + 0xa2b74` it reads **4.78** in both snapshots
  (the base, unmodified), followed by 1.0, an empty marker, then **1.0 standing / 1.1106 at speed** at +0xa2b80
  and 0 at +0xa2b84. The word at +0xa2b40 is 0xd and the one at +0xa2b44 is 1, so the mode decomp.md puts at
  +0xa2b40 is probably at +0xa2b44. +0xa2bfc does not hold a float (bytes 01 01 ff 7f); the float after it
  (+0xa2c00) is 1.0, so the runtime speed scale is very likely that one and was 1.0 in free roam. The point at
  +0x36114 is not the player's position: (1814, 23.7, 1650) while the car stood at (1400, 34, 1178), and
  (925, 22.4, −101) while it drove at (887, 24.5, −555), so it runs several hundred metres ahead of or away from
  the car (a streaming or look-ahead point; not identified).

Not read (no route to the object yet): pad dead zones, the free-roam state bytes, the colour-cube frame state.
All of them can still be followed in the snapshots.

### 2026-10-06 (session 5, round 6): the shadow pass's cost

**Tooling** (`shadow_cull.rs`, a PostUpdate system after Bevy's `check_dir_light_mesh_visibility` that filters
each cascade's `CascadesVisibleEntities` before extraction). Env switches: `MW_SHADOW_CASCADE_OFF=0,2`,
`MW_SHADOW_CLASS_OFF=alpha,world,backdrop,prop,car,other` (mesh entities carry a `ShadowClass`: world placements
split into opaque `world` / alpha-tested `alpha` / `backdrop` = model keys with bit 62 or 63, `prop` from
`props::spawn_prop`, `car` from `drive.rs`, `other` = no class), `MW_SHADOW_MIN_TEXELS=k`,
`MW_SHADOW_MAP_SIZE`, `MW_SHADOW_DISTANCE`, `MW_SHADOW_MODEL_RANGE=1`, `MW_SHADOW_FOOTPRINT=0`. The bench line now
prints each cascade's texel size.

**Measurement caveat**: the machine was not quiet (another agent's rustc and a browser; load average 4.4-8,
given per run below). Frame means also clamp at 16.6-16.8 ms in many runs (60 Hz) although the line says
"X11": the same city view read 12.4 ms in one run and 16.67 ms in the next with identical settings, so
`AutoNoVsync` under XWayland is paced at 60 Hz part of the time. Below ~16.7 ms the frame mean is not a cost
signal; the render-world CPU (prepare + graph) is used alongside. Not fixed (open: force `Immediate` or run
the bench in gamescope / a bare X server).

**Attribution**, drive view (`--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3 --bench 8`),
two runs each (run 1 at load 5-8, run 2 at load 5-7); baseline casters 147 / 316 / 1586 per cascade, texel
0.028 / 0.084 / 0.251 m:

| switch | casters | frame mean ms | render world ms (prep + graph) | saves |
|---|---|---|---|---|
| baseline | 147/316/1586 | 26.3 (disturbed) / 18.4 | 25.4 / 17.7 | |
| `--no-shadows` | - | 16.6 / 16.7 (capped) | 12.9 / 12.4 | ≥ 1.7 frame, 5 ms render world |
| cascade 0 off | 0/316/1587 | 19.0 / 18.7 | 18.3 / 18.0 | nothing |
| cascade 1 off | 147/0/1587 | 18.2 / 19.6 | 17.5 / 18.8 | nothing |
| cascade 2 (outer) off | 147/316/0 | 16.6 / 16.8 (capped) | 14.3 / 14.5 | ≥ 1.6 frame, ~3.3 ms render world |
| alpha-tested off | 128/259/1351 | 19.6 / 19.0 | 18.9 / 18.3 | nothing |
| opaque world placements off | 83/123/307 | 16.7 / 16.9 (capped) | 14.2 / 14.0 | ≥ 1.6 frame, ~3.6 ms render world |
| backdrop off | 143/310/1574 | 19.6 / 19.7 | 18.9 / 19.0 | nothing |
| props + car off | 102/271/1541 | 17.8 / 19.1 | 17.2 / 18.4 | within noise |
| other off | 132/301/1571 | 18.3 / 19.1 | 17.6 / 18.5 | nothing |
| map 1024 / 4096 | same | 19.2, 21.0 / 20.6, 19.0 | 18.5-20.3 | nothing (not GPU raster) |
| min 1 texel | 147/312/1571 | 18.5 / 18.6 | 17.8 / 18.0 | nothing (16 casters) |

The cost is **CPU per caster per cascade view** in Bevy's render world (specialise / queue / batch), not
rasterisation: map size does not matter, and the outer cascade's ~1 250 opaque world placements are most of it
(about 1-2 µs each). A sub-texel size cull cannot help: the outer texel is 0.25 m and only 16 casters (85 at
4 texels) are smaller.

**Default-on change: receiver-footprint cull** (`shadow_cull.rs` module docs). Our lighting pass samples a
cascade only for visible points whose light-space xy is inside it, and fades the shadow to nothing at the last
far bound (180 m). Bevy keeps every caster inside each cascade's square box, which bounds a sphere around the
frustum slice. Criterion: project the caster's world AABB corners and the view frustum's corners (from
`minimum_distance` to the last far bound, the whole shadow range, not just the cascade's slice) with the cascade's
`clip_from_world`; drop the caster from that cascade if the two xy rectangles do not overlap after widening the
frustum's by 4 texels (2×2 PCF tap and the receiver's normal offset). Casters toward the sun are not culled by
depth, so off-screen casters whose shadow reaches the view stay.

| view | casters before -> after | frame mean ms before / after (two rounds) | render world ms |
|---|---|---|---|
| drive | 147/316/1586 -> 138/277/853 | 18.98, 19.35 -> 18.20, 18.58 (load 6-7) | 18.3, 18.7 -> 17.5, 17.9 |
| city overview | 28/29/108 -> 28/28/56 | 13.10, 13.18 -> 12.55 (second run disturbed: load spike, 24.6 ms) | 12.4, 12.4 -> 11.8 |

About **0.8 ms per frame** in the drive view. Image check (`--hour 12`, standing car, PIL): drive view before vs
after MAD 0.45/255, 0.9 % of pixels > 8/255, and the amplified difference shows only the idling tachometer and
one-pixel edge shimmer of the car and tiles, no shadow change; city overview MAD 0.04/255, 0.25 % > 8/255.

**Left behind flags (picture changes)**:
- `MW_SHADOW_MODEL_RANGE=1`: the model's own **shadow distance** (`Model` +0x0C, stored squared, now parsed as
  `Model::shadow_distance`; survey of 12 679 models: 100, 200, 400, 800, 1000 m and 81 other values). A caster
  is dropped while the camera is farther than that from its AABB centre (the comparison point is a model
  decision; the field's use is not yet traced in the decomp). Drive: outer cascade 1586 -> 1090 alone,
  frame 18.98 / 18.88 -> 17.83 / 17.42 ms; with the footprint cull 853 -> 648, 18.20 / 18.58 -> 17.74 /
  17.78 ms. Picture: 0.6 % of pixels > 8/255, upper half (far buildings' shadows). Fidelity-sourced data,
  so it should become the default once the decomp confirms the meaning and the distance origin.
- `MW_SHADOW_DISTANCE=120` (instead of 180): 147/271/774 casters, 17.1 ms (one clean run; the other disturbed).
- `MW_SHADOW_MAP_SIZE`: no gain either way.

**LODs for shadows** (step 3, not done): the loader only ever loads LOD0 (`AssetStore::model_meshes(id, 0)`).
The measured cost is per caster (draw), not per vertex, so a coarser LOD in the shadow views would not cut it;
the `SHADOW` state (`Model::STATE_SHADOW`, index 6) could map a model to a dedicated shadow renderable, which
would need a second mesh entity per placement (Bevy cannot swap meshes per view) and so add entities. Distance
LOD selection for the main view (from `lod_distances`) is the fidelity task; it would need `VisibilityRange`
per LOD entity, which Bevy's directional-light culling also honours per view.

### 2026-10-06 (session 5, round 6): the chase camera's position springs and avoidance

Goal: read `FUN_005d12b0` and the other "weight" helpers, resolve the yaw spring's value, locate avoidance.
Result: **`FUN_005d12b0` partly read; its speed band, heights and longitudinal (z) offset ported**;
avoidance not in the decompiled code (decomp.md "Speed and acceleration movement", "Avoidance: where the
search ended").

- `FUN_005d12b0`: `h = clamp((v − b6010ee1) / (53e4b4f9 − b6010ee1), 0, 1)` (mph); eye / look-at heights go
  to `7d4ec535` / `b028673b` linearly in `h`, unsmoothed (the round-4 guesses were right). The eye's z offset
  is a **first-order lag** (rate `fdd25ee3` = 2/s accelerating, `285f2260` = 2.5/s decelerating) towards
  `0a78c15f · h + (bd3a7cfc | 7c9039e2) · |a_long| / a_max`, `a_long` the acceleration projected on the
  velocity. So the braking offset is `7c9039e2` = 0.5 m, not `8e1310f6` = 1.5 m (that one feeds a vertical
  stage at cam + 0x554 whose locals the decompiler lost). `.bss` limits not read: `DAT_0110e770` (a_max and
  clamp), `DAT_0110e4d0`, `DAT_0110e680`, `DAT_0110e610`, `DAT_0110e640`; `a_max` = 20 mph/s is a model value.
- Yaw spring: nothing more resolvable from the pseudo-C; stays a model decision.
- Avoidance: the director's update `FUN_005d5e20` is not a function in the dump; every remaining helper of
  the chase update and the manager `FUN_005e23c0` calls only camera-range maths, leaves and the CRT (callee
  lists in decomp.md). No world / car / prop query exists in the decompiled camera code. Next: disassemble
  0x005d5e20 from the unpacked exe (probe), or watch cam + 0x580..0x58c / 0x5a0 live against a wall.
- Not read this round (budget): `FUN_005d0b90`, `FUN_005c7a80`, `FUN_005c9380`.
- **Traces, 911, before → after** (boom = horizontal eye distance in car space, m; accel_z = z offset):
  accel: boom peak 6.83 → 7.19, settle 5.60 → 5.60, z offset peak −1.33 → −2.64 (it now includes the speed
  term: −1.00 at top speed); brake (100 → 0 km/h): eye forward 1.50 → 0.32 m, boom at stop 3.10 → 4.28;
  drift: boom 6.79 / 5.12 → 7.10 / 5.38; corner: 6.77 / 5.51 → 7.10 / 5.52; crash: boom min 4.08 → 4.60,
  settle 4.60 both; landing: boom 6.83 / 6.08 → 7.10 / 6.26, eye height 1.32…2.00 → 1.32…2.03. Yaw lag,
  lateral, FOV and pitch unchanged (rules not touched).

### 2026-10-06 (session 5, round 5): the chase camera ported from FUN_005d4970

- **Structure of the update** (decomp.md "The followed frame"): the "sway" is a per-axis smoothing of the
  car's orientation as Euler angles (pitch, yaw, roll), `angle += (1 − retain) · wrap(target − angle)`,
  `retain = 1 − clamp(K · dt · 60, 0, 1)` (`FUN_005e14d0` → `FUN_005c89c0` → `FUN_005c75b0`), stiffnesses at
  cam + 0x6d0 / 0x6d4 / 0x6dc; the pitch's retain × `min(|speed| · 0.02, 1)`; the pitch stiffness eases 0.1 per
  update to a per-player default (`FUN_005c8e40`, with an airborne branch and a landing shake). No lag clamp.
  Ported (`camera.rs`); our 0.6 rad yaw-lag cap and the "up vector at 2/s" slope following are gone.
- **Yaw spring field map settled** (stored order via `mwtool genesys`): y10 `Decay_rate` 0.01, y14 `78527964`
  10, y18 `High_speed` 100, y1c `63fb2207` 0.05, y20 `397fce82` 10, y24 `Low_speed` 0, y28 `5a74d44d` 0.075.
  The yaw-rate term divides by y14 − y20 = 0, so it is inert in the shipped data. The stiffness formula
  `lerp(lerp(y28, y18, f), mods(0x29), s)` stays unresolved (`f` unshown by the decompiler, `mods(0x29)` and
  `_DAT_0110e8f0` need a live read); our speed blend 0.075 → 0.05 is kept and marked as ours.
- **Weight**: `FUN_005c8880` is a vertical bob, a 4-sample mean of `0.5 (v2 + v3)` read at `obj + 0x1f0 + n·4`
  (likely the rear wheels) × state + 0x94, Y only; not ported. `FUN_005d12b0` (3500 bytes, writes
  cam + 0x550..0x58c) is the likely speed / acceleration movement; not read.
- **Avoidance**: no world or collision query anywhere in the call tree of `FUN_005c2130`, `FUN_005d12b0`,
  `FUN_005dfe90` (all leaves are maths). Remaining candidates listed in decomp.md.
- **Traces, 911, before (round-4 baseline) → after** (peak / settle; lateral and boom in car space):
  drift yaw lag −15.5 / −14.7° → −14.6 / −14.0°, lateral 1.32 / 1.27 → 1.29 / 1.21 m, boom 6.80 / 5.0 m
  unchanged, height 1.74 / 1.69 → 1.74 / 1.68 m, FOV 73.1 / 71.6 → 73.1 / 71.2°; corner yaw lag −15.7 / −15.4° →
  −13.8 / −13.6°, lateral 1.49 / 1.47 → 1.32 / 1.29 m, boom 6.79 / 5.5 → 6.77 / 5.51 m; accel unchanged (yaw 0,
  boom 6.80 / 5.60 m, FOV 70 → 95°); crash: yaw lag 1.1° peak, boom min 4.10 m (4.05), settle 5.47 m. The
  smaller lag is the linear-in-dt gain (stiffer than `1 − e^(−r·dt)` at the tests' 4× time step); at 60 Hz
  real time the two are identical. The visible change is the pitch: rigid with the car at standstill and
  following faster below 50 m/s; the car's body roll, followed before through the eased up vector, is now
  its own smoothed angle (`K_ROLL`, a model value).
- Still needed for the rest of the port, from a live read (the original was not run this round): state + 0x70,
  + 0x74, + 0x94, cam + 0x6dc, `_DAT_0110e8f0`, `_DAT_01189f60`, and the stack `f` of `FUN_005c90a0`.

### 2026-10-06 (session 5, round 5): static mesh merging

**What was built** (`loader.rs` `Worker::send_merged`, `convert::merge`; opt-in `--merge-static`): per track
unit, static placements (no physics, not a backdrop, not groundcover, not the car) whose model is placed
**once** in the unit and whose parts are all opaque or alpha-tested, unpainted, with a plain attribute set,
are moved to world space and merged per (`Look` after instance tint / AO, attribute set) into meshes of at
most 2^18 vertices drawn at the origin. The vertex transform reproduces Bevy's mesh vertex stage: positions
by the matrix, normals by the inverse transpose (normalised), tangents by the matrix (normalised) with w
times the determinant's sign. The winding is not reversed for mirroring transforms: Bevy 0.19 does not flip
the front face for a negative determinant either (only `meshlet` does), all world materials have
`cull_mode: None`, and `double_sided` ones flip back-face normals, so keeping the winding keeps the picture.

**Merging repeated models runs out of GPU memory**: the first version merged every static placement; full-map
loads then failed with wgpu "Out of Memory" (8 GB RTX 3070, others' runs sharing it), because instanced
models (trees, lamps, kerbs) were copied into every merged mesh. Repeated models already share one bin (one
per mesh x material), so merging them saves nothing; only single-use models are merged now.

**Result: no measurable gain, so it is off by default.** Back-to-back A/B on one binary (`--no-merge` then
merged, two rounds, load average 6-11 from other agents' builds and runs), frame mean ms:

| view | unmerged | merged | mesh entities (visible) unmerged -> merged | shadow casters per cascade |
|---|---|---|---|---|
| cliff | 23.6 / 14.2 | 17.9 / 16.9 | 107 869 (3 258) -> 98 702 (2 945) | 79/162/655 -> 84/160/642 |
| city | 16.7 / 18.2 | 21.8 / 16.6 | 107 869 (3 652) -> 98 702 (3 233) | 28/29/108 -> 50/53/128 |
| drive | 22.4 / 20.1 | 20.4 / 20.1 | 20 756 (3 846) -> 18 791 (3 484) | 147/316/1586 -> 161/304/1446 |

Render-world CPU (prepare + graph) moved with the frame time, within the noise (cliff 22.4 / 13.4 vs
16.3 / 14.2 ms; drive 21.6 / 19.4 vs 19.8 / 19.4 ms). RSS 1.2-2.4 GB in both, dominated by noise; load wall
time unchanged within noise (merging is cheap next to decoding). Earlier baseline runs on master at load 8-15:
cliff 20.7 / 17.9 ms, city 18.7 ms, drive 48.5 / 39.7 ms (other agents' games and rustc running).

Why it does not help: merging cuts only ~10 % of the visible mesh entities. Bins are per mesh x material and
world materials are almost all specific to one model (their textures), so single-use models rarely share a
`Look` with another model in the same unit; repeated models are already one bin. To cut bins substantially
the materials would have to be shared (texture arrays / atlases, bindless), which is a renderer change.

Images (`--screenshot`, PIL, mean abs diff per channel, pixels off by > 8/255): cliff 0.81/0.47/0.26 and
2.6 %, city 0.37/0.24/0.12 and 0.6 %, night (20 units, hour 1.5) 0.10 and 0.1 %. Two unmerged runs of the
cliff differ by more (1.22/0.79/0.44, 2.9 %): the scrolling sea and the swaying tree canopies; no
difference outside them.

Next by the numbers: shadows (2-3 ms cliff, ~7 ms drive in round 4): fewer casters per cascade (far or tiny
objects out of the outer cascade; the drive's third cascade has ~1 450 casters) and fewer shadow views, after
measuring the game's own shadow distance and split.

### 2026-10-06 (session 5, round 5): the chase camera, second attempt

Goal: find the chase camera's update from round 4's leads and port its sway, weight and avoidance.
Result: **the code is found; nothing ported yet** (the budget went into the search; decomp.md "## Cameras"
has the map).

- Lead 1 (the 0x3050 objects) worked, indirectly: the object at camera manager + 0x6a0 is a 0x3050-byte
  camera director built by `FUN_00422640`, which stores 15 camera sub-objects' vtables. Their update methods
  (vtable +8, read from `.rdata`) were checked for the car's speed and the camera data: the sub-camera at
  director + 0x8d0 (vtable 0xf999b4) updates through **`FUN_005d4970`**, which reads the speed and calls about
  20 helpers, of which `FUN_005c90a0` (yaw spring), `FUN_005c9380` (roll), `FUN_005c7a80` (gradient
  adjustments) and `FUN_005d0b90` (speed-based movement, impact shake, final write) read
  `CameraGameplayExternal`.
- Why round 4 missed it: the camera reads its data through a per-player state at camera + 0x290
  (resource at state + 0x98, instance offset at state + 0x9c), so the read is
  `*(*(state + 0x98) + 0x28) + idx + OFF`, usually with a sub-struct base first (`+ 0xbc`, then `+0x10..`).
- First rule read (`FUN_005c90a0`): the yaw spring is chosen by **the car's yaw rate in car space**, not only
  by speed: `t = clamp(1 − (|ω_y| − y20) / (y14 − y20), 0, 1)`, rate-limited by `y10` per call into
  cam + 0x6d8, and blended with a speed factor `clamp(|v| / v_ref, 0, 1)` (`v_ref` is `.bss` `_DAT_0110e8f0`)
  and the car's mods-profile value 0x29. The y-names are sub-offsets of `Yaw_spring_modification`; which hash
  is `Low_speed` / `High_speed` / `Decay_rate` is open, and so is the last blend (one unread local).
- Lead 3: `CameraGameplayExternalGlobals` is only impact shake + lens + a shake effect. Lead 4: no
  `ProcessNode.Camera*` instances exist in any bundle; the chase camera is plain code, not a node graph.
- Avoidance is not located: `FUN_005dfe90` (shared with the scripted cameras, the impact shake and the final
  writer) looks like transform composition; the unread helpers listed in decomp.md are the candidates.
- No camera rule changed, so the round-4 baseline traces still describe our camera.

### 2026-10-06 (session 5, round 5): groundcover look

- **Why the tufts were black**: the groundcover quads used the Standard look with Bevy's double-sided
  material, which flips the world normal on back faces. Our sprite normal is the instance's ground normal
  (pointing up), so every back-facing quad got a downward normal and the deferred pass left it unlit. Test
  (park lawn, `--camera=-760,45,1960,-900,30,2080 --hour 12`): with the new groundcover mode but Bevy's
  flipped normal kept, the tufts stay solid black exactly as before; with the vertex normal used unflipped
  they read as olive grass clumps in the lawn's light. The game never meets the case: its sprites always
  face the camera.
- **The programs** (`mwtool shader-asm Groundcover`; facts in formats.md "Sprite vertices"): one texture
  (`DiffuseSampler`, an atlas), cell from the COLOR alpha byte (= the instance's region byte) via
  `GroundcoverTextureRegions`, `uv = region.xy + region.zw × (u, 1 − v)`; alpha test at 0.5; RT1 = texel.rgb /
  texel.a × sqrt(2 tint), so albedo = (rgb / a)² × 2 tint, like the trees' premultiplied texels; RT0 = the
  normalised vertex normal; RT2 / RT3 untouched (no specular); no lighting in the shader. The vertex normal
  is mostly the ground normal: `normalize(sprite vector) + max(8 − height, 0.75)·(n + (0, 0.25, 0))`.
- **The CPU vertices** (`FUN_00eb9500`): position, NORMAL = bytes +8..+10 / 127, BINORMAL = (+6, +7, +11) /
  127 (width and height shrunk by the range fade), COLOR = bytes +12..+15 as RGBA (tint, region), UV halfs
  (0,0), (1,0), (1,1), (0,1). Constant `DAT_01081004` = 1/127 (.rdata).
- **Sprite size, still open, narrowed**: `gObjectCB` is not a separate buffer: the draw record points at the
  collection's own material record (`collection +0x30 + material × 0x110`, 0x110 = the cbuffer's 272
  bytes), so `GroundcoverConstants` = the record's first 16 bytes. On disk those hold the material import
  (all zero in `TRK_UNIT1`'s collection 03c936db), so whatever resolves the import must also write the size
  scale there; the 0x20F load / fix-up function was not found this round (next: the resource-type handler
  for 0x20F, or a live read of the record in the running game). Sizes stay at byte / 32 (model decision).
- **Done**: `Mode::Groundcover` (materials.rs / layered.wgsl): albedo = (texel / a)² × 2 tint clamped to 1,
  unflipped vertex normal, no specular. The quads' vertex normals follow the game's formula without the
  camera term (−0.1·d), since the crossed quads are static; the tint is no longer clamped per factor.
  Camera-facing sprites need a vertex-shader path in the layered material (not cheap): still crossed quads.

### 2026-10-06 (session 5, round 4): the chase camera from the code

Goal: port the chase camera's sway, weight and avoidance from the code that reads `CameraGameplayExternal`.
Result: **not found; nothing ported** (decomp.md "## Cameras" lists every search and candidate).

- Searched every function for reads of the External camera's field offsets (eye 0x148, look-at 0x130, pivot
  0x13c, acceleration 0x64..0x8c, roll 0xa0..0xb8, yaw spring 0xcc..0xe4, pitch 0xf8..0x108, top level
  0x160..0x170) in the GeneSys read form `*(res + 0x28) + OFF + index` (both orders), as `*(float *)(base + OFF)`
  and as `base[OFF / 4]`, and for the container's four camera handles. Only false positives (render-state
  setup at `FUN_008be4e0` / `FUN_008be1c0` / `FUN_008bdc50`, matrix maths, an unrelated global GeneSys resource).
  Conclusion: the camera does not read its data in any of the forms the handling code uses; it probably copies
  it once into its own object, or reaches it through a node graph.
- Leads: `FUN_005d4270` registers `CameraEntityModuleState` with getters for `GameplayExternalCameraTransform`
  and `GameplayExternalFOV` (`LAB_005dae70`, `LAB_005daeb0`): labels, not decompiled; their bodies (the probe or a
  disassembly of the unpacked code) give the field that holds the Gameplay External camera. `FUN_005e23c0` keeps
  eight-deep histories of a car's speed (+0x1620), the time step, the body's velocity and a transform (a
  target filter, by its shape), next to the camera module; its caller's caller `FUN_005d2cd0` looks like the
  camera manager's update (loads `CAMERAS.BNDL`, updates that history, then calls virtual method +8 of a
  0x3050-byte object in an array at manager + 0x6a0): the vtable of those objects is the next thing to find. GeneSys has camera process nodes
  (`CameraTransformTracker`, `VelocityLerp`, `Collider`, `EntityAvoider`, `ImpactShake`, `FOVAutoZoom`), which
  suggests the avoidance is more than a push-in along the boom; whether the gameplay camera uses them is open.
- Baseline traces of the current (data-only) camera, 911, for the next attempt to compare against (peak /
  settle over the last second; yaw lag = camera yaw − car yaw; lateral, boom = |(x, z)|, height in car space):
  drift: yaw lag −15.5° / −14.7°, lateral 1.32 / 1.27 m, boom 4.6 → 6.80 (launch) / 5.0 m, height 1.74 / 1.69 m,
  FOV 73.1 / 71.6°; accel: yaw 0, boom 6.81 at 4.5 s / 5.6 m at 308 km/h, height 1.73 / 1.70, FOV 70 → 95°;
  corner (skidpad, 74 km/h): yaw lag −15.7 / −15.4°, lateral 1.49 / 1.47 m, boom 6.79 / 5.5 m, FOV 71.5°.
  crash (114 km/h into the wall at 7.05 s): yaw lag ±0.7°, lateral ±0.06 m; the boom jumps 6.60 → 5.44 m in
  one frame at the hit and bottoms at 4.05 m, settling at 5.6 m; height dips to 1.34 m; impact pitch 10.5°.
  Nothing in our camera steers round the wall or other cars: the only world response is the push-in along the
  ray from the look-at point.

### 2026-10-06 (session 5, round 4): the anti-roll bar and roll damping from the code

The jitter (roll 0.45° p-p at 11.2 Hz at 100 km/h, 0.06° at rest, gone without the bar's damper) came from our
bar damping, which used `Front_` / `Rear_roll_damping` (1.2) as a ratio of critical against the scaled stiffness.

- **Where the game builds the bar**: `FUN_008fda70` (vehicle build) reads `BodyMovement` +0x68
  (`Front_roll_damping`) and +0x7c (`Rear_roll_damping`) and calls `FUN_00933a60` once per axle with the axle's
  two wheel objects, the chassis body, a stiffness (vehicle slots +0x7fc / +0x800), the ratio and an inertia: the
  reciprocal of lane z of the chassis's +0xe0 vector (inverse inertia → roll inertia, inferred). `FUN_00933a60`
  allocates a type-2 constraint element (types registered in `FUN_009333b0`; springs are type 0 via
  `FUN_00933930`) and stores k × 60 (`.bss` 0x01185d70, read live earlier) and 2 (`.rdata` 0x010245d0) × ζ ×
  √(k × I), with k **unscaled** inside the root. So the effective ratio is ζ / √60 ≈ 0.155: ours was 7.7× too
  damped, and with last tick's compression rates the explicit damper rang at 11 Hz.
- **Not found** (searched for about half the budget): the writer of the stiffness slots +0x7fc / +0x800 (they
  are copied, not read from `BodyMovement` directly in any decompiled function; `782f05c1` / `2bb0f6a0` at +0x64 /
  +0x78 is from the field order and earlier sessions), and the per-tick solve of the type-2 element: no
  decompiled function reads element +0x20 / +0x30; the island step `FUN_0093f5a0` calls `FUN_00932af0` (1104
  bytes, no callees), which has no body in the dump: likely the force-element pass (bars and spring dampers).
  Searched: readers of `BodyMovement` +0x64 / +0x78 in all files, the pool at world +0x3ac / +0x3c4, `switch` on
  the element type in 0092–009C, functions loading +0x20 and +0x30 vectors with `sqrt`. So the bar's input
  (compression difference vs. body roll against the axle), where it acts and its integration form stay ours.
- **Ported**: the damping coefficient (chassis.rs `roll_damping` = 2 ζ √k_data, × √I in `drive::anti_roll_loads`).
  Jitter at 100 km/h 0.449° @ 11.2 Hz → 0.011° @ 1.0 Hz, at rest 0.060° → 0.000°. Accel, corner (2.57 g),
  handbrake, drift unchanged; brake 38.5 → 38.6 m; landing deepest 0.055 → 0.058 m; climb deepest 0.101 →
  0.056 m, 133 → 126 km/h (handling.md "Measurements").
- **Open**: the stiffness unit (per radian in our model), the solve (`FUN_00932af0` needs a disassembly or a
  live trace: a probe write-watch on the element's +0x20 lanes would name the reader).

### 2026-10-06 (session 5, round 4): dirt colours, two-tone paint

**Why the dirt drew pink / cyan.** Not the maths, the data or the colour space: our `layered.wgsl` port of
`VehicleNFS13_BodyPaint`'s G-buffer program matches the DXBC line by line (the `gObjectCB` listing in
`mwtool shader-asm` confirms `GenericVector4_A/B/C` at cb1[0..2], `PbrPaintDiffuseColour` cb1[4], scratch cb1[5..6],
`PbrPaintSpecularColour` cb1[7]; effects texture `t1` at `v3.zw` = vertex TEXCOORD2). Debug frames (albedo forced to
the raw effects texture, to its alpha, to `p[10]`, to the UV) showed that even a flat `DirtColour` albedo came out
pink and cyan, so the tint was added after the albedo. Cause: `Look::retain_textures` re-derived the generic vehicle
flags for paint meshes after `with_paint` had written them: p1.x = "has spec map" = slot 1, which on paint meshes is
the effects texture, so F0 became the effects map's rgb at TEXCOORD0 (and p2.y = "has diffuse" multiplied the livery
into the paint before the livery blend). Paint meshes now keep their own flags there (only clearing a slot whose
texture failed to load). After the fix the red and silver 911 at `--dirt 0.6,0.3` show light dust patches and darker
streaks low on the doors and behind the wheels, no hue shift; a flat `DirtColour` albedo renders brown-grey.
The effects texture stays linear: the program squares `e.w` and `e.y` itself (a gamma-2 decode), which it would
not do on an sRGB view. Note from the asm: partial dust darkens (lerp(paint, d·C, d), the same formula as ours),
and covered areas lose F0 and the clear coat, so dust on a white car reads as grey specks.

**The writer of the paint constants** is `FUN_008e9c00` (008E0000.c, the per-object constant block written
through `FUN_00a621a0(slot)`, not `FUN_00a5d660`, which is why the earlier search found no writer). For a render
record (stride 0x2b0 at `param_1 + 0x1a0`) with a paint block `P` at record word 0xa2:
- 0x2a = `P+0x10` (primary diffuse), 0x2b = `P+0x20` (primary specular), 0x2e = `P+0x30` (scratch colour);
- 0x30 = `P+0x40 − P+0x10`, 0x31 = `P+0x50 − P+0x20`: the **secondary** diffuse / specular **minus the primary**,
  so `_TwoPaint*`'s `mad(Secondary, index, Primary)` is a lerp to the secondary paint;
- 0x9d (`GenericVector4_B`) = `P+0x60..0x6c` = DirtColour, 0x9e (`_C`) = `P+0x70..0x7c` = DustColour;
- 0x9c (`_A`) = (`DirtColour.w` × record word 0x28, `DustColour.w` × word 0x29, word 0x2a, word 0x2b).
  So the shader's dirt amount is the car's dirt level × `DirtColour.w` (0.83 on the cars seen) and the dust
  amount its dust level × `DustColour.w` (0.85). Now applied to `--dirt`.

**Two-tone paint.** The second paint comes from the car's `Genesys.Gen.VehicleColourPalette`: field `Colours`
(`16671e74`, a list of paint handles) and `ColourGroups`, each a `ColourGroup` struct of `PrimaryColour`
(`2968e77a`), `SecondaryColour` (`750cf08e`) and `WheelColour` (names cracked with `mwtool genesys-crack`; the
sort order by name matches). The secondary paint's own `PaintColour` is 1 (the primary's 0). Two more palette
fields, `17acabdb` and `db17b169`, are 1 on the police cars seen (unnamed). Bound now: `vehicle_paints` gives a
paint the (diffuse, specular) of the `SecondaryColour` of the group whose `PrimaryColour` it is, and the shader
adds (secondary − primary) × (`mPaintColourIndex` or the mask's green). Check: Ford Police Interceptor
(122713) at `--paint-index 2` (its group's primary, a mid blue) draws its `mPaintColourIndex = 1` roof mesh white.
**Open:** the paint the palette's `Colours` list names (1099e2, dark blue, index 0, our default) is not the group's
`PrimaryColour` (18cf62) on that car, so by default the roof stays blue; on the Corvette Z06 police (122699)
`Colours` and the group agree but both `_TwoPaint_Livery` meshes have index 0. How the game picks the group for a
car (cop cars may always use group 0) needs the code that fills the paint block `P` (word 0xa2 of the render
record; `FUN_008e1280` initialises the record, called from `FUN_008b7a20` and `FUN_008cffa0`).

**Dirt build-up: where it stopped.** The dirt / dust levels are words 0x28 / 0x29 of the render record, copied
as one float4 (with 0x2a / 0x2b) from a parameter of `FUN_008e1280` (record init); its callers `FUN_008b7a20`
and `FUN_008cffa0` were not read, so the source (surface, speed, distance, car washes) is not traced. Dirt stays
off unless `--dirt` is given.

### 2026-10-06 (session 5, round 4): water shaders

User report: water "still looks very weird". Where it is (`MW_WATER_DEBUG=1` on any run logs each water
mesh: shader, renderable, vertex centroid (local for instanced meshes), whether it has TEXCOORD2, and its
textures by sampler): 16 `Water_Opaque_Singlesided` meshes (rivers and ponds: e.g. near 1647,13,2976,
2837,13,4777, 7454,13,4356, 1559,14,−212, −2269,17,−4193; the park pond near −850,·,2000 and the river under
the road-cliff bridge are instanced), 9 `Water_Proto_Cheap` (sea / far water: 6419,19,951, −385,17,5343,
2919,18,−5855, 6259,10,2539, …). Viewpoints `water-river`, `water-pond` (viewpoints.md).

**Before** (pond, noon): a flat slate-blue surface, no waves, no floor, no reflection structure. Causes,
term by term against the DXBC (formats.md "Shaders", Water):
- Pass: the family is opaque in our materials (deferred, our lighting + Fog pass) and in the game (4 G-buffer
  targets): already right; not the Bevy forward-pass bug the trees had.
- F0: ours forced max(specularAndRoughness.x, 0.6) to fake the mirror, so the deferred sun specular and the
  sky term used a wrong F0. The game writes specularAndRoughness.x and gets the mirror from the LUT read at a
  negative N·V instead. Fixed: F0 from the material; RT3 now computed as the program does (reflect(−V, N),
  horizon clamp of R.y, LOD roughness × 5.5, env-BRDF at N·V = 0, × (1 − s.y)) into the RT3 extra, with the
  generic sky term switched off (specular occlusion 0) so it is not counted twice.
- River floor: ours used a constant 0.35 grey because Bevy has two UV sets. TEXCOORD2 now rides in the COLOR
  attribute (`convert::WATER_UVS`; the water layout has no vertex colour), and the floor, its refraction offset
  (SpecDepthAndRefraction.w at 2 × TEXCOORD2 scrolling Time / 2) and the caustics (DiffuseAndCaustics.w at
  world xz / causticTile) are ported.
- Surf (foam): not drawn before; now SurfNormal / Surf / SurfMask as in the program (white albedo, normal and
  roughness lerped by Surf.y × SurfMask.y).
- Waves: coordinates and scroll already matched (TEXCOORD1 × normalMapTile, v + Time / waveAnimationSpeeds).
- `Water_Proto_Cheap`: albedo 0, roughness .y, same reflection: matches after the reflection fix.

**After**: pond shows the floor and algae near the banks and wave structure; the river close up shows the
floor with caustic nets in the shallow part and the sky reflection in the deep part, also at night (floor and
caustics are albedo, so night ambient lights them).

**Open**: (1) the material's `ReflectionTextureSampler` cube (`…157de9`, 128² BC3: grey mountain silhouette in
rgb, alpha 1 above it) multiplies the sky reflection by (rgb + a); we have no cube slot per material, so it is
left out (factor 1): reflected mountains are missing and the reflection below the silhouette is too bright.
(2) The env-BRDF LUT's address mode (AmbientSpecSampler, s14) is assumed clamp; with wrap a negative N·V
would read the head-on column (reflection ≈ F0) instead of the mirror. (3) A straight diagonal edge between
a floor-visible and a deep region at `water-river`: data (two meshes / SpecDepth) or ours, not checked.
(4) `PlotPBR_AO_StandingWater_Opaque_Lightmap_Singlesided` (28 meshes) not ported. (5) No capture of the
original's water to compare against.

### 2026-10-06 (session 5, round 4): Easydrive and HUD from the game's layouts

**Differences found** (our frames at 1280×720 against the layout data, `mwtool ui show`; largest first):

1. **Easydrive panel** was our own (dark box at 4 % / 28 %, orange highlight, Bevy-sized rows, 8 rows). The game's
   is the `EasyDrive` widget's layout 1773380: a single `UIElement_ElementStack` at (3, 43) whose rows are
   further stacks and item layouts (formats.md "UI element stacks"). Element stacks were not parsed at all, so
   nothing the game draws through them (Easydrive, the `EasyDrivePanel`, any list UI) could be drawn.
2. **Speed digits faint**: the speed label's own colour is grey with alpha 0.45 (`1670378/9`, font 581007,
   size 38), so faint digits are what the data says for that label; why the original reads bright is not
   known (a material / blend on the label, or a code-set colour). Not changed.
3. **Odometer ". 0 KM"**: the layout splits the reading: `{Widget.Nitrous.Odometer(DIST,LONG)}` right-justified
   (118×21, anchor 5, font 581007 size 19, alpha 0.56) and a separate "." label at (1209, 696) (font 1206239).
   Our `DIST` format prints `0.0`, which collides with that dot; the game's `DIST,LONG` formatter (code) must
   print the digits around the separate dot (probably integer part, gap, tenths). Not changed: needs the formatter.
4. **Minimap**: drawn as an opaque black square with grey roads; whether the original's map is masked / tinted
   by the frame layout (1206246) was not checked this round.
5. Nitrous bar and "KM/H" / "KM" unit labels: placed and coloured as the data says (unit labels alpha 0.51).

**Easydrive now** (`crates/mostrusted/src/easydrive.rs`): layout 1773380 drawn through `UiScene` (as the HUD)
with the `Widget.EasyDrive.*` bindings filled from the menu state: `Items._length` / `Items[i].Name` (visible
rows), `SelectedIndex` (the selected row template), `CurrentItem.Name`, `TierHistoryLength` /
`TierHistory[i].Name` (header: the titles of the open tiers), `EasyDrive.Length` (footer arrows), item flags
(`Locked`, `New`, `PositionNotNull`) false, `Shutdown` empty (picks the plain name label), `Widget.<TYPE>.IsActive`
false (the generic row). Element stacks are generic in `UiScene::stack`: header, one cell per entry (first true
template, else default), footer; inside a cell `#` / `#parent_index#` are substituted and dot-less bindings are
fields of the entry. Before: our dark box with orange bar; after: the game's white selected bar with its arrow,
translucent dark row strips, game font, at the layout's place (top left).

**Still ours / open**: row pitch = the lowest element bottom of the cell layout (26 px; the `EasyDrivePanel`
moves by 26.42 per item, so the stack's real cell size rule is not decoded: `CellSizeDefinition`, `Padding`
unread); 10 rows shown (from the footer's `Length >= 11` threshold; `NumItemsShown` is code); the six glow images
of the selected row (`01010000001942a0`) and every layout's APPROACH / FADE transitions and idle behaviours are
not animated; the header at the root tier is empty (root has no title); the closed-menu prompt (1805584) and the
`EasyDrivePanel` side panel (descriptions, 1902272) are not drawn (which shows when is widget-adapter code).
HUD layouts not drawn at all: pursuit / cooldown widgets, event prompt (`Prompt`), messages (`UIMessages.*`),
Most Wanted / AdditionalHUD layouts beyond the minimap and road name: each needs its game state (heat, events,
message queue) first.

### 2026-10-06 (session 5, round 4): mods applied, chosen through Easydrive

- 25 of the 55 profile indices are identified from `FUN_00574870`'s call sites (decomp.md "Nitrous and
  upgrades"); `mwtool mods` prints their field names. The rest keep `p<n>`.
- Applying (`mw_assets::mods::apply_to`, `crates/mostrusted/src/mods_menu.rs`): the stock handling is cloned and
  each identified index goes through value × Π + Σ / replace; a `NitrousUpgrade` sets the tank burn to its
  variant's `Nitrous.053c5a6a`. `--mods slot=category` (indices as `mwtool mods` prints them); the spawn logs
  `mods: …`. A selection change rebuilds `CarParams` live.
- 911: no mods = baseline (accel 3.72 s / 11.58 s; nitrous 3.47 s, tank 4.17 s). `12=0` (LONG GEARS PRO: torque
  ×1.05, final drive ×0.85, gear-up speed ×1.175): 0–200 km/h 11.05 → 10.72 s, top 307.6 → 309.1 km/h. `7=0`
  (POWERSHOT NITROUS PRO, burn 24 → 18): tank empty 4.17 → 5.57 s, 0–200 10.33 → 8.93 s.
- Open: the Easydrive "Customize car" pages are not wired yet; the unidentified indices (incl. all body / damage
  ones); `TyreUpgrade` (its `a316f8e3` is a `Tyres` handle, not applied); the top-gear lock that long gears lift.

### 2026-10-06 (session 5, round 4): tyre marks

- **Look** (`Vfx_TyreMarksNew`, G-buffer pass = the `ps_4_0` with three targets): vertex = POSITION.xyz strip centre
  (.w an age-like value), TEXCOORD.xyz the vector across the strip (.w = U along it), COLOR = (Slip weight, Spin
  weight, side 0/1 = V, opacity). Position = centre + across x `WidthScale` x (V - 0.5) + 0.03 m along the up
  vector it derives from the across vector, then 0.35 - 0.33 x saturate(dot(to_eye, up)) m towards the eye (depth
  bias). Opacity out = (1 - `Animation`.x x POSITION.w) x COLOR.w. Pixel: `RunDiffuse` premultiplied, `SlipDiffuse`
  "over" it with weight Slip.a x COLOR.x, `SpinDiffuse` likewise with COLOR.y, all x saturate(opacity), discard
  below 1/2550; RT1 = colour (premultiplied), RT0 = (0,0,0,a), RT2 = `Specular` x a. Details in formats.md "Shaders".
- **Data**: `SurfaceVfx.TyreMarksMaterials` (field `75cb5de9`, inline struct) holds one material per tyre class:
  `eff16a09`, `Offroad`, `Road`, `Sport`, `Track`; also `SurfaceVfx.RimMarksMaterial`. 48 of 65 surfaces have one;
  surfaces 0 and 1 (the `Surfaces` list head) have none. Tarmac `Road` material `…117f16`: Run `…7a932` (8x8, fully
  transparent: no marks from rolling), Slip `…7a92a` (32x32 black, alpha fading at the strip edges), Spin
  `…117f0e`; `WidthScale` 0.65, `Animation` 0, `Specular` 0. `Offroad` = `…117f14`, `Track` = `…117f23`.
- Per car, `TyreVfxBehaviour` (`Front_Tyres` / `Rear_Tyres` -> `Skid_Marks` / `Smoke`, each `Lat_Slip`
  (`LatParams`: `Curve_Power` 0.35 and seven unnamed floats, e.g. 0.1, 60, 50, 0.45, 23.5, 0, 0.2 on `…68fe4`),
  `Long_Spin` and `64222c3f` (`LongParams`)): the intensity curves of the marks. Field names beyond `Curve_Power`
  are not cracked and **the code reading them was not found**: no string in strings.csv names tyre marks, skid
  marks or `TyreVfx`, the field hashes and the struct stride (0x130 = `FrontRearParams`) do not appear as
  constants, and `* 8 + 0x18` (`TyreMarksMaterials` indexed by tyre class) has no hit. Open.
- **Built** (`crates/mostrusted/src/tyre_marks.rs`, `tyre_marks.wgsl`): the shader port above (lit in place,
  blend ONE / INV_SRC_ALPHA), strips per wheel of the driven car. Model decisions (until the rule is traced):
  the Slip / Spin weights are the per-wheel lateral / spin intensities of the skid **sound** rule
  (`FUN_009d1bf0`, decomp.md); a mark is laid while max(lat, spin) >= 0.05; a strip point every 0.25 m; U = metres
  of strip; the across vector is a unit vector (width = `WidthScale` = 0.65 m, wide for a 911: the game may scale
  it by the tyre width); fade linear over 60 s on the CPU instead of `Animation`; at most 4000 segments (oldest
  dropped); the first surface with a material stands in for every contact (surface types not decoded), `Road`
  tyres, dry only. `MW_TYRE_MARKS_LOG=1` logs live segments (drift test: ~100 per second while sliding).
- A mark mesh with the same vertex layout as the skinned car meshes made Bevy's multi-draw batching fail
  (`skinned_mesh_layout` validation error); an extra unused `UV_1` attribute avoids it.

### 2026-10-06 (session 5, round 4): performance baseline and first optimisations

**Benchmark** (`bench.rs`): `--bench <s>` on any headless run waits for `Loaded::done`, warms up 4 s, measures
for `<s>` seconds and prints one `BENCH RESULT` line: frames, mean / median / 1 %-low frame time (Bevy's
`FrameTimeDiagnosticsPlugin`), the main-world schedule time (First..Last), the render-world CPU time split into
prepare (extract .. bind groups, which includes the swapchain acquire and so any GPU wait) and graph (the
`Render` set: encoding, submit, present), shadow casters per cascade, entity / mesh-entity / visible counts,
mesh and image asset counts at start and end (both constant: no streaming during the measurement), RSS and
GPU / backend. `MW_BENCH_PASSES=1` adds Bevy's `RenderDiagnosticsPlugin` (per-pass CPU / GPU timestamps and
pipeline statistics; Bevy's shadow passes and our own lighting / sky / tonemap nodes are not instrumented).
`--no-shadows` turns the key light's shadow maps off (attribution only).

Vsync: on Wayland (this machine: KWin, NVIDIA) wgpu offers no `Immediate`, and `AutoNoVsync` falls back to
`Mailbox`, which the compositor paces at 60 Hz: every run read 59-60 fps, even with one track unit. Under
XWayland `AutoNoVsync` gets `Immediate` (a 1-unit scene went from 16.9 to 11.1 ms), so `--bench` drops
`WAYLAND_DISPLAY` at startup when `DISPLAY` is set; the result line says "X11" or "Wayland, vsync-paced".
`--frame-stats` numbers taken on Wayland before today were capped the same way.

All runs: RTX 3070, Vulkan, 1280x720 window, release build, while other agents were compiling (load average
6-21 on 16 threads). Run-to-run noise is large (up to 2x): each configuration was run twice, the table gives
both. Commands: `--camera=-1131,27,1627,-1190,15,1720` (cliff, full map), `--camera=-650,260,1850,-1000,60,2150`
(city overview, full map), `--drive 911 --units 30 --spawn=815,60,1760,270 --auto-throttle 0.3` (drive), each
with `--mute --bench 10`.

| view | frame mean ms | median | 1 % low | main world ms | entities / mesh / visible | RSS |
|---|---|---|---|---|---|---|
| cliff | 28.0 / 19.4 (later 17.6) | 26.6 / 17.4 | 59 / 45 | 18.9 / 10.1 | 178 811 / 107 868 / 3 257 | 1.3-1.8 GB |
| city | 18.7 / 18.5 | 17.3 / 16.9 | 41 / 69 | 10.1 / 9.9 | 178 811 / 107 868 / 3 651 | 1.9 GB |
| drive | 29.0 / 35.2 (later 20.3 / 18.3) | 28.8 / 31.8 | 60 / 90 | 9.2 / 12.4 | 34 394 / 20 755 / 3 845 | 1.6-1.9 GB |

**Attribution** (same viewpoint, two runs; "saves" against the quieter baseline runs):

| switch | cliff frame ms | drive frame ms | saves |
|---|---|---|---|
| baseline | 19.4 / 18.7 | 20.3 / 18.3 | |
| `--units 20` (23 341 entities) | 19.6 / 19.1 | | nothing: the 150 unloaded-from-view units cost no frame time |
| `--no-groundcover` | 44 (busy) / 16.0, main 8.1 ms | | ~2 ms main world (15 000 group entities, visibility) |
| `--no-backdrop` | | 21.2 / 29.9 | nothing measurable |
| `--no-shadows` | 21.7 / 16.6, main 6.5 ms | 11.2 / 13.1, main 4.0 / 3.7 ms | 2-3 ms cliff, ~7 ms drive: the largest single cost |

GPU utilisation during the drive bench (nvidia-smi, 1 Hz) was 25-40 % at 21-50 fps: the runtime is **CPU
bound**, and mostly in the **render world**: render-world CPU (prepare + graph) 13.5-19 ms against a main
world of 5-8 ms, so with pipelined rendering the render world sets the frame time. GPU timestamps of the
instrumented passes are small (deferred G-buffer 0.4-0.5 ms, bin unpacking 0.5-1.7 ms).

A chrome trace of the drive bench (`--features bevy/trace_chrome`, one build; the trace itself slows frames
~2x, so read proportions, not absolutes) puts the render world's CPU in Bevy's GPU-driven batching, per view:
`write_binned_instance_buffers<Shadow>` 2.4 ms and `<Opaque3dDeferred>` 1.8 ms, `prepare_preprocess_bind_groups`
2.5, `queue_submit` 1.9, `unpack_bins` 1.6, `update_sparse_buffers` 1.3, shadow pass encoding 1.2 ms per frame.
These scale with views (the 3 shadow cascades are 3 more views) and with bins (one per distinct mesh x
material visible in a view; world geometry is almost all unique meshes, so bins ~ visible entities), not with
the loaded entity count. Main world: avian physics 2.75 ms per frame (1.27 fixed steps per frame, 6 substeps),
visibility propagation and frustum tests ~2.5 ms of CPU across threads, `calculate_bounds` 0.7 ms. Our own
systems are small: `hud::draw_hud` 0.4 ms, everything else under 0.1 ms (`groundcover_distance` included).

The floor: one track unit with `--no-shadows` (3 647 entities) ran at 11.1 ms (main world 3.5 ms) on a
quieter machine and 19.4 ms (main world 12.4 ms) at load average 15, so on this shared machine CPU contention
alone moves every number by up to 8 ms, and the full map adds only ~0-8 ms over that floor. Some runs landed
on exactly 600 frames in 10 s (60.0 fps) on the X11 path too; an unloaded machine is needed to rule out a
remaining pacing cap before comparing small changes. A drive pair run back to back: `--no-shadows` 16.7 ms
(main 4.2, render 6.6 + 7.7 ms) against 36.8 ms with shadows (main 14.5, render 17.3 + 18.1 ms).

So the candidate list's distance culling, unloading far units and merging groundcover would not move the
frame time at these viewpoints: the work is in what is visible, per view. What would: fewer bins per view
(merging static world meshes per material within a track unit, which also cuts shadow bins), fewer shadow
views or casters (the cascade split and the 180 m shadow distance in `deferred.rs` are provisional, not from
the game: measuring the game's `ShadowMap_ScaleOffset` / distance first is the fidelity-safe order), and
physics on its own thread budget. No optimisation was landed in this round: the measured costs did not
match a change that keeps the frame identical and fits the round.

### 2026-10-06 (session 5, round 3): vehicle liveries, paint masks and dirt

- **Paint variants** across all car bundles (`mwtool models <bundle> --verbose`): plain `BodyPaint` (53 cars),
  `_Lightmap` 31, `_Livery` 10, `_Livery_Lightmap` 5, `_TwoPaint` 6, `_TwoPaint_Livery` 5 (the police cars
  122699, 122706, 122713, 510072, 2076005), `_TwoPaintMask` 9, `_TwoPaintMask_Lightmap` 2, `_NormalMap_NoDamage` 4.
- **Liveries are material data, not paint options.** The livery is the material's `DiffuseTextureSampler`
  texture (BC3, alpha 0 where the paint shows); `Genesys.Gen.VehiclePaint` has no texture field (its 21 fields:
  `PbrDiffuse/Specular/Pearlescent/ScratchColour`, `DirtColour`, `DustColour`, clear coat and scratch floats,
  `Display_Texture` = the menu swatch, `Type`, `PaintColour`). So `--paint-index` and liveries are independent;
  no `--livery` flag.
- **Paint pixel program** (longest `ps_4_0`, object CB `gObjectCB`): `GenericVector4_A/B/C`, then
  `PbrPaintClearcoatSettings`, `PbrPaintDiffuseColour`, `PbrPaintScratchColour`, `PbrPaintScratchSettings`,
  (`PbrPaintSecondaryDiffuseColour`, `PbrPaintSecondarySpecularColour` in the two-paint variants),
  `PbrPaintSpecularColour`, `VehicleID`; material CB `mPaintColourIndex` (0 / 1 per mesh). Maths:
  `_Livery`: paint = lerp(paint, livery.rgb, livery.a), livery at TEXCOORD0. `_TwoPaint`: diffuse =
  `PbrPaintDiffuseColour + mPaintColourIndex × PbrPaintSecondaryDiffuseColour` (specular likewise);
  `_TwoPaintMask`: the same with the DiffuseTextureSampler's green at TEXCOORD0 in place of the index.
- **Dirt / dust** (all paint variants): EffectsTextureSampler (material-bound, shared) at TEXCOORD2:
  dirt a = sat(e.w² × A.x), dust d = sat(2 × sat(e.y² × A.y) × e(uv×7).x); coverage = a + d(1 - a);
  colour lerps towards a·B(1 - d) + d·C (B, C = GenericVector4_B / _C, whose data are the paint's
  `DirtColour` / `DustColour`); roughness lerps to 1, F0 × (1 - coverage), clear-coat reflection ×
  (1 - coverage)⁴. **Scratches**: e(uv×(2,3)).z band × sat(2·e.z - 1) × TEXCOORD5.w (the damage weight from the
  vertex program) lerp the colour to `PbrPaintScratchColour` and roughness / F0 to `PbrPaintScratchSettings.xy`.
- **Code**: the object-constant names are registered with slot numbers in one table function in the
  0x8c4xxx range (calls to `FUN_00a5d490`): `PbrPaintDiffuseColour` 0x2a, `PbrPaintScratchColour` 0x2e,
  `ScratchSettings` 0x2f, `SecondaryDiffuse` 0x30, `SecondarySpecular` 0x31, `GenericVector4_A` 0x9c. Who writes
  these slots for the car (the per-object setter, not `FUN_00a5d660`) was not found yet.
- **Built**: liveries drawn (`materials.rs` paint branch, `layered.wgsl` mode 6). Police Corvette
  (`--vehicle "corvette z06 police"`) shows its POLICE / FAIRHAVEN CITY livery over the dark-blue paint.
  `--dirt dirt[,dust]` enables the dirt port.
- **Open**: (1) the secondary paint's source (no VehiclePaint field names it; two-paint meshes with index 1
  draw the primary paint); (2) the dirt port tinted clean paint pink with orange blotches even at zero
  amounts in the first test, so it is only enabled by `--dirt` until that is found (suspect: UV_1 = TEXCOORD2
  on these meshes or the p9–p11 uniforms); (3) the dirt accumulation rule and who sets GenericVector4_A;
  (4) scratches (need the damage weight); (5) whether the mask texture is sRGB (read linear here).

### 2026-10-06 (session 5, round 3): fog as the game computes it; fog on trees

- **Why trees floated unfogged** (measured): with the Fog/lighting pass forced to output red, every pixel
  turned red except the trees. Bevy 0.19 queues `AlphaMode::Mask` materials in the forward `AlphaMask3d`
  phase even when their render method is deferred (`bevy_pbr` material.rs: only the opaque branch checks
  `OpaqueRendererMethod::Deferred`), so trees (`Tree_Translucent_1Bit_*`), and every other 1-bit look, were
  written to the G-buffer, lit and fogged by our pass, and then drawn again on top with Bevy's forward PBR and
  no fog. In the game the tree programs only write the G-buffer (two render targets, `discard` below the alpha
  reference, no fog constants) and the Fog pass fogs them with the terrain. Fix: layered.wgsl's forward
  branch discards when `DEFERRED_PREPASS` and `MAY_DISCARD` are both defined (views without a deferred prepass,
  such as the planar reflection, keep forward shading). After: trees and the hills under them are fogged alike
  from the high and low cameras; at road level the trees are now lit by the game's key light and ambient.
- **The Fog pass** (`mwtool shader-asm Fog`, ps 01010008ce7671f3; port in `apply_fog`): distance = Euclidean
  camera → pixel in world metres (view-space reconstruction from depth); t = saturate(d × ScattCoeffs.x −
  ScattCoeffs.y)^ScattCoeffs.z; t' = t × gradient alpha; in-scattering t' × saturate(ScatteringSettings.y /
  (1 + ScatteringSettings.x × cos)²) × ScatteringColor (we clamped at 64 instead of 1); a height-layer term
  exp(−|d × (C.z / (1 + exp(C.x Δy + C.y)) − C.w) / Δy|) from `LayerFogCoeffs`; sky pixels (depth 1) get only
  the layer term and the scatter. The fog colour is a 1D texture sampled at the screen u.
- **CPU side** (docs/decomp.md "Fog"): `ScatteringSettings` = (g, (1 − g²) × strength / 4π); now applied, so
  in-scattering is on again (0.0039 / (1 + 0.971 cos)² at noon: ≤ 0.001 away from the sun, 1 into it).
- **Still open**: whether near / far are scaled on the CPU (the shader's form matches 1 / (far − near),
  near / (far − near), Power, so 80–500 m stays; the mountains beyond 500 m stay fully fogged); the gradient
  texture builder (rgb approximated, alpha taken as 1: if its alpha is below 1 at noon the hills keep detail,
  which is the most likely reason the original's hills are hazy rather than flat); the `LayerFogCoeffs`
  source; the sky-pixel fog. A live read of slots 0x23, 0x27, 0x83 and of the gradient texture in the running
  game would settle all four. Groundcover goes through layered.wgsl masks too; forward glass, particles and
  sparks still get no fog (they need the fog uniform in their bind groups).

### 2026-10-06 (session 5, round 3): landings, steep climbs, jitter

- **Reproduced** with new test-track tests (`--test landing`, `--test climb`, `--test jitter`; handling.md
  "Measurements"). Before: the 911 lands from a 3 m drop at 150 km/h with −8.6 m/s; 0.14 m of travel and the
  load cap (4 × corner weight) cannot stop that, the wheels end 0.34 m past full compression and the body box
  rests on the ground; the climb (120 km/h into 30°) ends STUCK at 0 km/h, sunk 0.478 m.
- **Why it stayed stuck**: once the wheel mount is under the ground the wheel ray starts inside it: against the
  test track's solid box the hit is at distance 0 with a zero normal (spring force × 0), against a trimesh (the
  world) the ray misses the back face (wheel airborne, in-air gravity on). Nothing lifts the car off its body box.
- **The game**: the strut's end is a hard contact on the chassis (`FUN_009c5a90` → `FUN_009c2670`, decomp.md).
  Ported: the speculative non-negative impulse at each wheel contact once the ground reaches the strut's end;
  the ray now starts a wheel radius above the mount (stopgap). After: landing sink 0.055 m, no overtravel,
  settled 0.45 s after touchdown; the climb completes. Regression tests unchanged.
- **Jitter** is a roll buzz at 11.2 Hz: 0.45° p-p (37°/s) at 100 km/h, 0.06° at rest. Turning the anti-roll bar
  off removes it (0.03°), and so does its damper alone (0.008°): the bar damper (`Front_` / `Rear_roll_damping`
  × critical against the roll inertia, an interpretation) is explicit and uses the previous tick's compression
  rates, which rings. Not fixed: the bar's unit and damping are not traced in the code (decomp.md "Unknown").
  Pitch 0.75° p-p at ~1 Hz at 100 km/h is the test's throttle controller, not jitter.
- The body collider reports world contacts on the 5.7° kicker without touching (speculative contacts in
  `collisions_with`), which also feeds `wall_time` for the turn-around assist; worth checking.
- Open: the strut travel (+0x460, from the builder's caller), the slack constants (`.bss` 0x01186bf0, 0x01186b10:
  a live read), the sub-step count of `FUN_0093f5a0`.

### 2026-10-06 (session 5, round 3): gearbox modes and part-throttle shifting

User report: "sometimes it kind of sounds like the vehicles are stuck in a gear". Reproduced on the test
track with `--auto-throttle` (log once a second, km/h : gear : rpm). Before: the 911 at 0.3 held first gear
for 17 s, 1100 → 7650 rpm, shifting at 72 km/h; at 0.5 it shifted 1→2 at 68 km/h, 7391 rpm; the XKR at 0.5
held first to 6499 rpm (52 km/h) and every gear to ~6500; the Veyron at 0.7 shifted at 6300–6600 rpm in every
gear. Cause: we only had the race gearbox (911: `Gear_up_RPM` 7700 in every gear, `Gear_up_speed` 25 / 40 /
80 / 120 / 150 / 175 mph), whose shift points sit near the limiter whatever the pedal.

The code (decomp.md "Gearbox modes"): `FUN_009ce630` switches to the `CruisingGearbox` after 3 s of
0 < pedal < 0.95 with no brake, and back after 0.05 s above 0.95; `FUN_00900660` applies it (tables reloaded,
gear clamped on the way back); `FUN_009dc3c0` takes the cruising `Gear_change_time` just after a kick-down.
The 911's cruising box: 5500 rpm, 15 / 45 / 70 / 85 / 105 / 135 mph, `Gear_down_RPM` 2000, 0.9 s per shift,
4th / 6th / 7th 1.30 / 0.88 / 0.71 instead of 1.225 / 0.92 / 0.77, top gear not locked.

After (same runs): 911 at 0.3: cruising box from 3 s, 2nd at 68 km/h, 3rd at 88 km/h (5477 rpm); at 0.7:
2nd at 67, 3rd at 81, 4th at 128, 5th at 154 km/h, all near 5500 rpm, then 4500–5400 rpm cruising; XKR at
0.5: 1 → 3 at 56 km/h (the target skips gears as `FUN_009d4c80` does), 4th at 100 km/h, 3000–4000 rpm;
Veyron at 0.7: shifts at 4100–4500 rpm, 7th (cruising box) at 247 km/h. Full throttle never qualifies, so
`--test accel` (911: 0–100 km/h 3.72 s, quarter mile 11.58 s, top 307.6 km/h) and the `ref/xkr_launch` replay
are unchanged (compare output identical).

Still open: (1) the first gear at part throttle still runs to ~0.95 × the race `Gear_up_RPM` (911: 7262 rpm at
68 km/h at 0.3): the launch state `FUN_009d17d0` keeps the clutch slipping until max(throttle, clutch, rpm /
+0x130) ≥ 0.95, and +0x130 was set from the race box at standstill. That is the game's rule, but our clutch at
partial engagement is almost rigid, so the rpm only gets there with the road speed; the game's clutch
(`FUN_009d7ac0`) may let the engine flare ahead and close sooner. Port the clutch's slip torque to settle it.
(2) `.bss` 0x011871e0 (kick-down window, assumed 1 s), gearbox +0xb8 (added to the shift time) and the
vehicle's vtable +0x34 flag (blocks the cruising box) are unread. (3) The sound side (limiter and gear-change
sequences of the bank) was not examined: the engine sound still follows the physics rpm.

### 2026-10-06 (session 5, round 3): nitrous and the upgrade profile

- `FUN_009d2aa0` decoded (decomp.md "Nitrous and upgrades"): mode 0 is a torque-multiplier lerp to
  `Engine.Nitrous_strength.e9de413e` (1.9 on the 911); modes 1 / 3 (punch, jump) add a forward force × mass
  faded from `Punch/Jump` fade-start mph to vehicle +0x7e8. Ported as `mw_handling::nitrous` with the gearbox's
  nitrous rules; `--test nitrous` on the 911: 0–100 3.47 s (3.72 without), 0–200 10.33 s (11.05), quarter
  11.25 s (11.58).
- `FUN_00574870` decoded: a 50-entry profile (operand, op, index, 64-bit active mask), multiply / add /
  replace. The mods are `Vehicles.PerformanceUpgrades` (two categories of slots; ModifierUpgrade /
  NitrousUpgrade / TyreUpgrade); `mw_assets::mods` loads them, `mwtool mods <car>` prints slot → choice →
  `p<index> x|+|= value`. Open: the index → handling-field map (call sites with literal 0x00..0x36), which
  category is pro, the slot names (field hashes), and applying a selection (`--mods`) in the runtime.
- Open: the multiplier's reader (assumed engine torque), the tank code and its refill rules (the data is
  `NitrousParameters`, five variants per car), `.bss` 0x0118a018 (punch / jump force scale), vehicle +0x30e8.

### 2026-10-06 (session 5, round 3): car animations, active aero

- **Where active aero lives**: `Genesys.Vehicles.AnimationStateBehaviour` → an `HSMData` state machine + its
  `Animation` clips on the damage skeleton (layout in formats.md "Vehicle animations"; 16 models).
  `PhysicsDrivenVehicleAnimationBehaviour` / `AnimatedBodyPart` turned out to be the exposed suspension and
  wheel-angle bones of four open-wheel cars, not aero (dead end for aero; useful later for suspension).
  `AnimatedRaceCarBehaviour` has no vehicle instances checked yet.
- **HSM conditions are in the data as text**: the 911's machine reads `speedinmph`, condition `Speed above
  70mph`, operands `70` and `40`; LFA `70` / `50`; the SLS / SL65 labels give 74 / 75 mph but no bare operand
  string (probably stored binary). Veyron, McLaren and Venom read `brake` too (`Brake ON and Speed above
  80mph`, `Wait 0.4s`); the Huayra reads `steering` (`Speed between 50 and 137mph and Turning Left`); the
  Agera R reads `gas`. Unknown operands per state: `False` (reverse play?) and `0.25` (blend time?).
- **Code**: the variable name strings (`.rdata` 0x00fae7e0..0x00fae7f4) have no references in the dump;
  the condition evaluator and the variables' producers are not found (docs/decomp.md "Vehicle animations").
  Next step: decode the HSMData property tree so every machine can run, and look
  for the HSM interpreter through the `HSM.AnimationCondition` type string.
- **Built**: `mw_assets::car_anim` (loader, simple speed rule), clips composed on top of the damage pose
  (`VehicleDamage::pose_with`), `car_anim.rs` (`--anim all=1` / `0=0.5` pins, otherwise the rule in
  `--drive`), `mwtool damage <car>` lists the parts. 911 `--test accel`: deploys at 70.2 mph, fully out
  1.6 s later. The `--vehicle` viewer draws static instances without the skeleton, so `--anim` needs `--drive`.

### 2026-10-06 (session 5, round 2): traffic rules in the code

Details and formulas are in decomp.md "Traffic rules (session 5, round 2)".

- **Found (confirmed by reading the code).**
  - Frame function `FUN_0060ebc0`; its local-game tick is `FUN_0060eab0`.
  - Active area = the player's hull cell plus that cell's listed neighbours (`FUN_005f0270`). The union and
    diff of that set is in `FUN_0060c9f0`.
  - Emitters are lane sections at the edge of the active area or with no predecessor (`FUN_0061c780`).
  - Spawn interval `60 / (m·Density)` with a random factor (`FUN_005fb120`): **`TrafficFlow.Density` is cars
    per minute per lane**.
  - A new car is skipped unless the gap to the last car on the section is at least `Speed × 2 s`
    (`FUN_0060e5e0`). The 2 s is read from the `DAT_0110f940` object.
  - Vehicle pick by `Proportion` from the flow type that section byte `+0x25` selects (`FUN_00608c80`). This
    settles the byte: it is the `TrafficFlowType` index.
  - Density multiplier = `TrafficSystemConfig` 0x80 (4.78), ×1.2 when the player is slow (20 → 60 mph ramp),
    ×`PursuitScale` 0.5 in a pursuit (`FUN_005f0780`).
  - Load budget 200 within a `.bss` radius: 20 per moving car, 5 per slow or stopped car, 30 per physical car.
    No spawning at 200 or above; above it the farthest car is removed every 0.5 s (`FUN_005fd6a0`).
  - The game's random source is a 64-bit LCG (`0x5851f42d4c957f2d`) feeding an 8-slot delay ring.
- **Inferred.**
  - The `DAT_0110f940` object is `TrafficParamConfig`: the values at every offset read fit (0x10 = 4 m
    standstill gap, 0x60 = 25 m minimum look-ahead, 0x6c = 2 s spawn headway, 0x88 = 2 s look-ahead time,
    0xac = 3).
  - Car following in `FUN_005fc220`: look-ahead `1.5 × max(2 s·v, 25 m)` (`FUN_005f1380` scans for the car
    ahead), and gap = `dist − back − 4 m − 3·x`.
  - World vehicle slots 256-405 / 406-429 may be parked cars.
- **Dead ends.**
  - The 24-entry hour array of `TrafficSystemConfig` has no direct reader among the users of `world+0xb17b0`.
    The only other way the base density changes is message 4 in `FUN_0060a380` (the sender is not traced).
  - `sys` 0x74 / 0x78 (1500 / 205) are read only by the setup function `FUN_005ef0f0`, into SIMD constant
    blocks at `world+0x35ff0..`. Their use is not traced.
  - `fn` cannot show the start value of the random lerp in `FUN_005fb120`: x87 temporaries were lost in
    decompilation.
- **Stopped at.**
  - `FUN_00617d00`: read only up to branch 1. It is a four-way behaviour arbiter: `FUN_005ea6a0` scores four
    options, the best one wins, and the cruise target is `Speed·mph·world+0xa2bfc`. The scorer and the
    acceleration limits (`MinAcceleration` −20 is in the same object) are not traced.
  - The rung advance and pose: `FUN_005f0d90`, `FUN_005f7cf0`, `FUN_005fcbd0` (the switch to a physical car).
  - The other tick steps: `FUN_00605d30`, `FUN_00602ae0` (it calls the "Crashing Traffic" network helpers
    `FUN_006180d0` / `FUN_0061a550`), `FUN_005fbbf0`, `FUN_0060ba30`, `FUN_0060b9a0`.
  - Junction records: reached through hull lane data `+0x28`. `FUN_0060c9f0` walks a `u8` at junction `+0x35`
    with entries near `+0x56`.
  - The player-stopped-for-30 s action `FUN_005fd3f0`.
- **`.bss` values needed (live read).**
  - `DAT_0110f940` / `DAT_0110f944`: which GeneSys object, and fields 0x48 and 0x6c.
  - `DAT_0118a090` (a rate scale in `FUN_00617d00`) and `world+0xa2bfc` (the global cruise-speed scale).
  - `DAT_0110f590`: the squared radius of the load budget.
  - The base density `world+0xa2b74` during free roam at several hours, to see whether the hour array reaches
    it.

### 2026-10-06 (session 5, round 2): free-roam scoring, trigger hits, jack spots

Read from the dump only (no live reads); details and formulas in decomp.md "Trigger hits" and "Billboards,
gates and other kinds".

- Settled: the header +0x1c index is per-250 m-cell lists of trigger indices; the streamer `FUN_0053bc70`
  activates the 2 × 2 cells around each player car and keeps at most 300 triggers live (`FUN_00539e50`, the
  "300 entries" of the loader). Box size (+0x24) is the **full** extent (the shape gets size × 0.5); points
  get no shape and are never overlap-tested. Flags bit 0 makes a box **one-way** (`FUN_00536f70`: counts only
  if `dot(vehicle vector, box Z axis) ≥ 0`); bit 1 is not read by the trigger code (meaning still open).
- Firing (`FUN_00707650`): on entry only (diff with last frame's per-vehicle list), already-found triggers
  dropped except speed cameras, then a ring buffer drained one hit per 4 s (`DAT_00fcb050`) into
  `FUN_007059b0`. The overlap test itself runs in the `"TriggerUpdate"` job (`FUN_00536a10` builds per-vehicle
  queries from the car's half-extent box); its narrow phase was not read, so "car box vs trigger shape" is
  inferred, not confirmed. Our runtime's origin-in-box test stays a model decision until then.
- Billboards: counted when the **prop breaks** (`FUN_005b5340`, the only poster of game-logic message 0x14,
  checks the smashed object's linked trigger id at +0x90 and the billboard kind). Gates: overlap path, plus a
  helper kind 7 / 'I' that hits the nearest '0'/'H' gate (`FUN_00537600`).
- Speed camera result goes to the Autolog leaderboard request in `FUN_008749e0` (`"eventId"`, board
  `"RoadRules"`); offline `FUN_00874830(id)`.
- Jack spots: a GeneSys list at manager +0x41b0 pairs two trigger ids (+0x20, +0x28) with a key (+0x24);
  `FUN_007061d0(key)` marks a pair found. Probably `CarSwapSpot` (44 bytes, eight UniqueIds), not confirmed:
  `mwtool genesys all --type CarSwapSpot` printed nothing, so the instances are embedded or in a bundle the
  tool does not scan.
- Dead ends: `BillboardWorth` / `ItemWorth` / `HighValueWorth` are Smash-and-Grab mode variables
  (`HSMBehaviourHSMDoSmashAndGrabScoring`), not free-roam points. No other poster of message 0x14 exists (all
  functions using the game-logic queue `DAT_010edb10` checked).
- Stopped at: the job's narrow-phase test (find the function `FUN_00da2e50` dispatches for "TriggerUpdate";
  `FUN_00536f70` is its per-hit callback), the body shop tail of `FUN_007059b0`, Speed Points (strings
  `CurrentSpeedPoints`, `SPEEDPOINTS`, `upload_sp_buckets`; listeners in `FUN_006a8050`), jack spot car
  spawning and the swap input (the `JackSpots` script and `JackspotScript_ScreenScript`), kinds 11
  (`FUN_006e4770`) and those of `FUN_006e16c0` / `FUN_006e1690`. Live reads wanted: game-state bytes
  `+0x2e90`, `+0x2e91`, `+0x2e95` (jack spot and speed-camera modes) during free roam.

### 2026-10-06 (session 5, round 2): braking instability and the steering's neutral offset

Question: in `ref/xkr_launch` (Jaguar XKR, rows at 60 Hz) the original turns and stops from 64 mph in 1.3 s
under straight braking (6.0–7.1 s, brake 1, steer 0); our replay brakes straight and is at 29 mph at 7.5 s.

- **The original hits something twice; the tyres do not start it.** Between two rows (6.017 → 6.033 s) the body
  velocity jumps from (28.4, 0.0, −1.83) to (21.0, 0.53, −4.91) m/s with a vertical kick and the yaw rate from 0
  to 61°/s: a 7.4 m/s change in one 1/60 s tick (≈ 45 g), at (1834.4, 38.8, 1983.1). The second one at
  6.600 → 6.617 s, (1845.2, 38.8, 1981.6): the velocity turns 34° in one tick (15.5 m/s straight on → 8.9 forward,
  5.8 sideways) and the speed drops from 34.8 to 25.6 mph. Before either the wheels show nothing: κ ≈ 0 on all
  four at 6.00 s, α < 0.2°. After the second the car slides at 30° body slip with the fronts locked (ω 0, κ −1 from
  6.78 s) and stops at 2.3 g (sliding sideways, possibly still in contact). The wheel `N` stays 4580–4750 front,
  4050–4300 rear throughout.
- **Between the hits the braking matches ours.** 6.08 → 6.58 s the original loses 5.6 m/s (1.13 g, slip within
  ±2°, the drift flag on from 6.07 s with `steer_norm` creeping to +0.26 from the drift's remap); our replay
  brakes at 1.1 g over its first 0.5 s and 1.4–1.6 g later. So the mismatch in the window is the collisions,
  which the flat test track of `--replay` cannot reproduce (our car is also 5 mph faster at the brake point:
  68.8 against 63.7 mph, an acceleration difference). This run is not usable for brakes or tyres after 6.0 s.
- **Open lead (brakes):** in the clean stretch the original's rear wheels show κ 0 … +0.07 under full brake with
  the clutch in (gear 3, 4300–4600 rpm), the fronts −0.05 → −0.51: the rears are not visibly braked although
  the XKR has 2000 N·m rear brake torque. Ours shows rear κ −0.1 … −0.3. Needs a clean straight-line brake capture.
- **q (vehicle +0x2c70) is the body slip / 35°.** The probe's `steer_q` equals atan2(v · right, |v · forward|) /
  35° to four digits from 0.5° to 33° (both events), times speed / 10 mph below 10 mph (q / slip / speed constant
  over 1–4 m/s). Outside the drift `steer_norm` follows −0.3 q at the steering rate (−0.20 at q 0.63, −0.09 at
  q 0.21), and that steers the fronts towards the velocity (their slip angle is the body slip plus yaw minus the
  wheel angle): a counter-steer assist. Writer still not found: every `0x2c70` in the dump (0x004–0x011) is either
  a read (`FUN_00904260`, `FUN_00902930`, `FUN_008f8cd0` as `[0xb1c]`) or a write to the game-mode object at
  +0x2f3fc (`FUN_006e0580` adds to it, 0x006c / 0x0074 / 0x0076 set it); the 4-float vector +0x2c70..+0x2c7c is
  never stored by offset, so it is likely written through a pointer to the block. 35° equals the XKR's
  `Maximum_steering_angle`: a capture of a car with another maximum would tell a constant from the field.
- Ported (drive.rs `steer_towards`, `steer_slip_fraction`): the offset q × `Maximum_steering_angle` ×
  `553bc073` (0.3; 1 in reverse; 0 in a drift), its side-limit extension `a40a6b17` (1) / `16fd6022` (0), and the
  centring test against the offset (decomp.md "Steering"). The replay is unchanged (RMS speed 15.1 mph, yaw rate
  7.8°/s, slip 5.5°: our car never slips there); 911: 0–100 km/h 3.72 s, 100–0 km/h 38.5 m, skidpad 2.57 g, all
  unchanged; handbrake test slip 1.2° → 1.1° (yaw 59°/s), drift test peak 4.5° (no drift) both ways.
- Not reached: the tyre +0x210 combined-slip scale and the brake torque split in the code (the run gave no
  clean evidence on them).

### 2026-10-06 (session 5, round 2): groundcover draw distance and sprites

Found the CPU side of groundcover: `FUN_00eb9500` (and a twin, `FUN_00eb8390`) walks a group's instances
and writes four 0x2C-byte vertices per visible sprite into a dynamic buffer (the `Groundcover` vertex
layout: position, normal, binormal, colour, half-float uv), then records the draw with the material at
`collection+0x30 + material·0x110` (found by grepping for that stride) and the group's byte +0x30. So the
GPU never sees the packed instances. What it says:

- **Group test**: distance from the camera (`param_6`) to the nearest point of the group box ≤ group+0x3C ×
  `DAT_010c6f74`. `DAT_010c6f74` is .data, 125.0 on disk (could be changed at runtime; nothing seen writing it).
  group+0x3C is the box margin `r`, so a group with 1 m sprites is drawn to 125 m.
- **Group byte +0x30 is a size exponent e, not a quad count**: the test scales size bytes by
  `(1 << (e & 0x1f)) × 1/127` (`DAT_01081004`). Values 2/4/6/8. Our old "1 to 4 crossed quads" was a guess;
  the game draws one quad per instance.
- **Instance +6 / +7 are width / height** (signed `char`), not yaw / size: BINORMAL = (w·f/127, h·f/127,
  byte 11/127). The shader uses BINORMAL.x × `cb1[0].x` as the width and .y × `cb1[0].x` as the height, and
  the sign of the width for its normal. Where the two bytes are equal (the "yaw equals size" note) the
  sprite is square.
- **Per-sprite test and fade**: drawn if in front of the camera plane (`(p − eye)·forward ≥ 0`, `param_7`)
  and |p − eye|² ≤ R², R = ((w>>1 & 63) + (h>>1 & 63)) × 2^e/127 × 125; then f = min((1 − d²/R²) ×
  `DAT_01081000` (5.0), 1) multiplies the width and height bytes (truncated to an integer). Sprites shrink
  to nothing over the last fifth of their range in squared distance; no alpha fade (the pixel programs
  only `discard` alpha < 0.5). Cap `DAT_010c6f70` = 20000 sprites per frame and 300 draws (`param_11+0x12c0`).
- **Vertex program**: right = (−d.z, 0, d.x) with d = normalize(instance.xz − ViewPosition.xz): yaw-only,
  turns about world up; vertex = p + right × (width × (u − 0.5) + sway × v) + up × height × v, sway =
  sin(Time.x × `speed` + p.x) × `amount` (material parameters), so only the top sways, sideways. Normal for
  lighting = sign(width)·right·(u − 0.5) + up·v − 0.1·d, plus max(8 − height, 0.75) × (normal + (0, 0.25, 0)).
  Texture region = `GroundcoverTextureRegions[colour.a × 255]`, uv (u, 1 − v) × scale + offset.
- **Not found**: who fills `GroundcoverConstants` (slot 0x7F; no `FUN_00a5d660(0x7f)` call). The draw record
  carries e, so `cb1[0].x` is probably derived from it; with 2^e it would be 2^e/127 m per byte, which is
  the old byte/32 only for e = 2 and up to 2 m per byte for e = 8 (implausible for grass, unchecked). The
  size stays byte/32 (model decision, `GroundcoverInstance::size_m`). `mwtool placement-check --groups -v`
  now prints e, r and the largest width / height bytes per group (`MW_GC_VERBOSE=1`). Result: **r is in
  2^e/127-per-byte units**, r ≈ ((w + h) / 2) × 2^e/127 of the largest sprite (e 2: 62/62 → 1.95 vs r 1.974;
  e 4: 47/47 → 5.92 vs 5.938, 37/18 → 3.46 vs 3.504; e 6: 21/87 → 27.2 vs 27.6; e 8: 7/32 → 19.7 × 2.016 =
  39.7 vs 40.9), so 125·r is the largest per-sprite range of the group. e = 8 groups are drawn to ~5 km;
  with byte/32 they are 1 m tufts (the far specks the audit saw), with 2^e/127 they would be 14 x 64 m
  sprites (far-vegetation cards?). Which is right needs a look at the original or the 0x7F writer; this
  session could not run screenshots (the agent sandbox refused to launch `mostrusted`), so sizes stay byte/32.
- **Unit visibility**: the builder only sees groups handed to it (by the `GroundcoverJob`, string at
  0x1002028 with no code reference in the dump); not traced whether a group needs its track unit loaded.

Implemented: one mesh per group, hidden by `groundcover_distance` (main.rs) when the camera is farther than
125·r from its box. Not done: the per-sprite range / shrink (needs a vertex-shader path; merged group meshes
draw every sprite of a visible group at full size) and the camera-facing turn (two crossed quads stand in).

### 2026-10-06 (session 5, round 2): ground texture filtering, sampler states, WorldPBR ground variants

Follow-up to the "wrong scale on road and terrain" report, after the UV audit below found the UV maths
matching. Five candidates checked; only the anisotropy differed, and fixing it does not remove the coarse
asphalt grain, so the cause is still open.

- **Mip maps: match.** `mwtool sampler-usage --textures` over all track units: every texture bound to a
  ground shader is BC1 / BC3 with a full chain down to 1×1. The only exceptions are 4×4 placeholders with a
  single level and 7 textures from other families. The chain is stored mip 0 first and tightly packed:
  on 154 BC textures of `TRK_UNIT1`, mip 1's block-endpoint image correlates with mip 0's downsampled 2×2
  at a median of 0.92. Our BC upload path (`convert::image`) passes every level (`mip_level_count`), with
  trilinear filtering. Only non-BC or non-4-aligned textures fall back to a single decoded level, and no
  ground texture does.
- **Sampler states: decoded** (docs/formats.md "SamplerState"; layout from the function that fills the
  `D3D11_SAMPLER_DESC`). Every ground colour, detail and normal map is wrap / 4× anisotropic / bias 0. Ours
  was one sampler for everything: wrap, 8× anisotropic, bias 0. Changed it to 4× (convert.rs). The
  effect is small: in the fixed `road-close` view, the mean horizontal gradient of the mid-distance road
  fell from 9.6 to 8.5, and the near road did not change.
  Still open: slots on trilinear / MipLODBias −0.9 (RoadDepth, Line, Decal normal/spec, SpecColour, some
  Plot/Terrain tiling maps; sharper in the game than ours) and the 37 mirror / clamp states on non-ground
  slots. Both need a per-slot sampler: `TexKey` would carry the state, and the shader would call
  `textureSampleBias`, because a wgpu sampler has no LOD bias.
- **Address modes: match** for the ground (all wrap).
- **`WorldPBR_Horizontal_*`: match.** In the vertex program, TEXCOORD0 and TEXCOORD1 pass through unscaled
  (`o1.xy = v1.xy`, `o1.zw = v2.xy`), and only the wet map gets world XZ × `cb0[6].x`. In the pixel program,
  albedo = Diffuse(uv0) × vertex colour × `MaterialDiffuse` × wet darkening, and AO = Overlay(uv1).y. Our
  `Mode::WorldPbr` does the same: diffuse on uv0 as the base colour, AO on `uv_b` = uv1.
- **Vertex UV formats: match.** Every TEXCOORD stream that the ground shader layouts carry for UVs is
  R16G16Float. Our half decoder (`dxgi::f16_to_f32`) is exact, subnormals included.
- **Detail map strength: matches.** The DriveableSurface G-buffer pixel program computes
  `ColourMap(v1.xy) × DetailMap(v1.zw) × 2 × wet darkening` with no distance fade and no strength
  parameter.
- **Still unexplained:** the high-contrast grain shows right behind the car, where mip 0 is sampled, so it
  comes from the detail texture itself at its authored tiling, not from filtering. Next checks:
  1. **Lead: the Texture header byte at 0x2C.** We read it as `most_detailed_mip`. It is 1 on every
     mipped texture and 0 on the single-level ones (survey of 20000 textures). The upload loop
     (0x00d8ee30) writes every level with `UpdateSubresource`, mip 0 included. The function that creates
     the shader-resource view (0x00d8e670) sets `MostDetailedMip` to this byte only when its flag argument
     is set, and to 0 otherwise. `MipLevels` is −1. Its single caller (0x00409510) passes 1 when the
     global at 0x010eac98 is non-zero and 0x00414a50 returns **false** (corrected 2026-10-06; the live
     value is true, so the top mip is kept). A texture-quality setting.
     With the flag set, the game draws every mipped texture one level down: half the resolution, texels
     twice as large and softer. That fits "scaled wrong" and the harsh grain of ours.
     Next: read 0x010eac98 live (memscan / probe) with the user's settings, and find what 0x00414a50 tests.
     If both hold, start our views at `most_detailed_mip` too (wgpu `TextureViewDescriptor::base_mip_level`,
     or drop level 0 at upload).
  2. Take a Steam F12 capture of the original at the `road-close` viewpoint.

### 2026-10-06 (session 5): ground texture scale audit

Report: "some textures, especially on the road and terrain are still scaled wrong" (no location). Every UV path
of the ground families with the most world triangles (`mwtool shader-usage`) was read from the DXBC
(`mwtool shader-asm`, vertex program outputs, the pixel program's `sample` coordinates, and the `gMaterialCB`
offsets in the reflection header) and compared with `materials.rs` / `layered.wgsl`.

- **DriveableSurface** (+ `_Lightmap`, `_AlphaMask` by name): VS `o1.xy = TEXCOORD0` (raw), `o1.zw = world.xz ×
  DetailMapUvScale.xy` (world = position × gObjectCB world, so instance transforms included), `o5.xy = world.xz
  × gGlobalCB[6].x` (puddle map). PS: ColourMap, Normal, SpecColour, RoadDepth at uv0; DetailMap_Diffuse and
  DetailMap_Normal at the world coordinate. **Ours already matched.**
- **DriveableSurface_DEPRECATED_Line_DirectionV** (1643 meshes): uv0 as above, detail at world xz ×
  DetailMapUvScale, Line_Diffuse and Line_NormalPlusSpec at **raw TEXCOORD2** (`o9.zw`). TEXCOORD4 (float3)
  and TEXCOORD2.x outside (−0.3, 1.3) only feed the retroreflective emission flag, not coordinates. **Matched.**
- **TerrainPBR** (`_Normal_`, `_TangentSpaceNormal_`): gMaterialCB packs `Tiling_Ratio_Noise` (float, offset 0),
  `Tiling_Ratios_X` (float3, offset 4), `Tiling_Ratios_Y` (float3, offset 16); the material stores each as a
  float4, used from .x / .xyz. VS: mask and macro normal at raw uv0; layer i (diffuse + normal) at
  `uv1 × (X[i], Y[i])`; noise at `uv1 × Noise`. **Matched.** Values over TRK_UNIT1 (`mwtool models --verbose`)
  are plausible: X/Y 1–300 per layer, Noise 50–200; no identity / zero / missing values seen.
- **PlotPBR** (`DriveableGrass_…`, `AO_Normal_Specular_…`): layers i at `uv0 × (X[i], Y[i])` (VS), mask at
  `uv0 × maskTilingRatios.xy`, decal at `uv0 × decalTilingRatios.xy` (PS), AO map and lightmap at raw uv1.
  **Matched.**
- **Mismatches found (fixed):** the road variants whose names contain "decal" or "retroreflective" fell to the
  plain material with whichever texture `DIFFUSE_SAMPLERS` picked, on uv0:
  - `DriveableSurface_Decal(_Lightmap)` (343 meshes): the DXBC writes the whole road (colour × detail × 2 as
    above) and lerps `Decal_Diffuse` / `Decal_NormalPlusSpec` sampled at **TEXCOORD2** over it by the decal
    alpha (albedo, normal slope, F0 to `Decal_Roughness_Specular_Fresnel.y`, roughness to .x, Fresnel to .z);
    written opaque. Ours drew Decal_Diffuse on uv0, translucent. Now the line path (Mode::RoadLine, untinted).
  - `DriveableSurface_FloatingDecal(_1Bit)` (65 meshes): the plain road maths (detail at world xz ×
    DetailMapUvScale); `_1Bit` discards where `EdgeAlphaPlusAoMap(uv0).y < 0.5`. Ours drew the **detail map on
    uv0** (wrong scale). Now Mode::Road, with the edge-alpha discard (slot 2, p2.x flag).
  - `DriveableSurface_DEPRECATED_RetroreflectivePaint_Lightmap` (57 meshes): its G-buffer pass never samples
    the Line_* maps (road at uv0 / world xz, AO map and lightmap at uv1). Ours drew Line_Diffuse on uv0. Now
    Mode::Road.
  Screenshot at `--camera=-1150,22,1700,-1160,20.5,1716`: before, faint smeared streaks on the asphalt; after,
  distinct oil / tyre stains at their own places; lane lines unchanged (~0.1–0.15 m).
- **Open:** the WorldPBR `Horizontal_*` ground variants (≈1200 meshes) were not audited; per-mesh vertex
  descriptors were not surveyed (all these shaders declare TEXCOORD0/1/2 as R16G16Float); sampler address
  modes not checked; non-1Bit FloatingDecal blending (still Translucent by name) not read from the DXBC; the
  decal / line normal is added to the road slope in ours where the DXBC lerps it by the overlay alpha. If the
  user's report persists, it is more likely a WorldPBR ground variant or texture filtering than these families.

### 2026-10-06 (session 5): placement audit (groundcover, trees, props)

User report: "foliage etc sometimes is in the wrong place" (no location). New checker `mwtool placement-check
[--bundle <unit>] [--top N] [--groups]` (`crates/mwtool/src/placement.rs`): loads the collision of every track
unit (1,652,030 triangles, 8 m grid), drops each placement onto the surface under its origin (vertical line,
nearest surface in y) and reports per layer, per model, per unit (groundcover) and the largest offsets. Trees
(models with a `Tree_*` shader) are their own layer and are checked at their lowest vertex (model min y times
the matrix's y row), not the origin. `--groups` rates every groundcover group (good = median |dy| < 0.1 m) by
layer / blob / start / quads; `MW_GC_VARIANTS=1` adds alternative decodes of the u16 fractions.

Numbers (dy = placement y - ground y; share of placements that have ground under them):

| layer | n | no ground | median | abs(dy) < 0.1 | < 0.5 | > 2 up | > 2 down |
|---|---|---|---|---|---|---|---|
| prop | 12,290 | 21 | +0.01 | 85.4% | 96.0% | 0.1% | 0.5% |
| compound | 4,301 | 37 | +0.01 | 77.6% | 88.2% | 5.5% | 1.2% |
| dynamic | 4,477 | 65 | +0.01 | 40.2% | 64.7% | 11.9% | 5.1% |
| instance (non-tree, origin) | 23,904 | 1,149 | +0.01 | 28.5% | 45.1% | 17.4% | 14.0% |
| instance tree (lowest vertex) | 9,620 | 234 | -0.64 | 10.2% | 37.9% | 2.4% | 26.5% |
| groundcover q1 / q2 / q3 / q4 | 5.09 M / 2.91 M / 0.29 M / 0.51 M | 37-77% | +0.01 | 42 / 32 / 45 / 26% | 64 / 55 / 61 / 41% | 8-25% | 10-16% |

(Groundcover n counts a collection once per unit that lists it; collections are shared between units.
Backdrop `InstanceList` entries have no collision under their origin: 1,907 of 1,907, as expected for far LODs.)

Results, no decode error found:
- **Groundcover position decode is right.** Per group, 50.2% of 11,579 groups (55% of instances) sit within
  0.1 m of collision with the current decode; every alternative is far worse (x/z swapped 8.8%, x flipped 9.4%,
  z flipped 9.7%, x and z flipped 4.7%, y flipped 3.5%). Good groups are spread evenly over blob index
  (43-57%), start = 0 or not (53 / 50%) and group order, so the group -> blob/start mapping is not the problem.
  Layer 0 (small tufts) 61% good, layers 1-2 (bushes, 2-4 crossed quads) 28%: they grow on terrain that
  collision covers coarsely or not at all. The worst unit (TRK_UNIT165, median +5.1 m) was checked visually at
  (1562, 22.5, 1456): the tufts stand on a raised lawn planter that has render geometry but no collision.
- **Groundcover anchor matches the shader.** `Groundcover` vs_4_0: vertex = instance position + width x (u - 0.5)
  along the camera-perpendicular horizontal + height x v up (`mad r0.xyz, r1.w, l(0,1,0,0), r0.xyz` with
  r1.w = v4.y x size x `cb1[0].x`), so the sprite stands on its base, as the loader draws it. The billboard is
  cylindrical: the quad's horizontal axis is perpendicular to (position - camera).xz, not the stored yaw (our
  static crossed quads use yaw; open). `cb1[0].x` is still unknown; the dump registers the cbuffer
  `GroundcoverConstants` (slot 0x7F, 16 bytes) and `GroundcoverTextureRegions` (0x80, 16 x 16 bytes) in
  `FUN_008c34b0`; the writer of slot 0x7F was not traced.
- **Trees are planted, not floating**: per-model medians of the lowest vertex are -0.1 to -0.9 m (trunks sunk
  into the ground), 2.4% more than 2 m up. Outliers: `a97b9688` (1,545 placements, lowest vertex median -5.8 m,
  origin -2.0 m: the model reaches 3.8 m below its origin; looks like a deliberate deep root, not checked
  visually) and `f86cfc31` (12, +2.2 m, probably on planters without collision).
- **Tree vertex programs do not move vertices** beyond wind sway (`Tree_Translucent_1Bit_Doublesided`: xz +=
  sin(time x speed + phase) x amplitude x weight, phase / weight from a vertex attribute), so the leaf cards are
  meant to be where the mesh puts them.
- Props, compounds: placed on the ground. Dynamics with large offsets are a handful of models (`34280415`
  +43 m x 33, `cdf4b833` +20 m x 40, `42d010d9` -18 m x 23); not identified (no debug names), probably objects
  on roofs / under bridges where the nearest collision is another level.
- Screenshot `lamp-tree` (-1112,24,1640 -> -1130,20,1630): lamp, trees, hedges and kerb stand correctly. In the
  same shot, dark groundcover specks hang against the sky in front of the cliff on the left.

Most likely cause of the report (not fixed, open): **groundcover is drawn at every distance.** The game presumably
draws groundcover only near the camera (unverified; open point "fade distance"), but we draw all collections everywhere, so tufts of
far collections (e.g. `e557779c`, shared by units 36-57, instances up to y = 422 over the mountains) are drawn
over far terrain that we render with a different / coarser backdrop LOD or not at all; they appear as floating
specks. Next step: find the fade distance (the GroundcoverJob, string at 0x1002028, culls groups per frame; or a
live read) and cull groups beyond it in the loader / renderer. Also open: camera-facing billboards instead of
yaw-oriented crossed quads.

### 2026-10-06 (session 5): tyres: longitudinal constraint and the launch

- Read `FUN_009c6f00` (the per-wheel tyre pass called from `FUN_009c75b0`), `FUN_009bc340`, `FUN_009bc0c0`,
  `FUN_009bbff0` and the clutch's torque line in `FUN_009d7ac0` (decomp.md "Tyres", step 3). `FUN_009bc340` is
  bookkeeping (a copy of the solve state and two contact anchors), not a force. `FUN_009bc0c0` is the
  longitudinal constraint: a brake hold first (the brake budget left after the spin step, / r, against the
  chassis alone, the spin untouched), then the slip that remains with the coupled mass 1/m + r²/I, both within
  the one cap; only the coupled part turns the wheel. The drive torque never enters the tyre pass: the clutch
  puts it into the driveline before. κ for the caps is taken from the spin after the clutch and the brake step.
- Ours already matched the coupled part, the order (brake step → caps → longitudinal → lateral) and the κ
  definition. It lacked the brake hold: a wheel the brake had stopped was spun back up by the coupled pass and
  stopped again next tick, so braking moved through the wheel's inertia and the rears slipped. Ported
  (drive.rs); the old "locked wheel stays at 0" special case now falls out of it.
- `ref/xkr_launch` replay (XKR) with the current model, before → after: RMS speed 15.2 → 15.1 mph, yaw 7.8 →
  7.8 °/s; during the braking the rear κ goes −0.14 / −0.26 → −0.03 / −0.02 (original +0.01 / −0.03) and the
  front κ −0.07 / −0.02 → −0.11 / −0.15 (original −0.13 / −0.54). 911 tests unchanged: 0–100 km/h 3.72 s,
  100–0 km/h 38.5 m / 2.63 s, skidpad 2.57 g.
- The "revs hang at ~6400 after the 1→2 shift, κ ≈ 0.6 in 2nd" no longer reproduces with the merged model: ours
  now reads κ 0.42 / 0.33 in 1st, 0.49 just after the shift and 0 from 3.5 s (original 0.10–0.12 in 1st, 0.39
  after the shift, 0 from 3.75 s). What is left: ours spins more in 1st and reaches the limiter (6750 against
  6120 rpm), and it leaves ~0.3 s early (launch timing, 3612 against 1568 rpm at 1.5 s).
- Open: the clutch torque is multiplied by a `.bss` vector at 0x011873e0 (one lane per entry; no writer in the
  dump), taken as 1. Below 1 the clutch is softer, the engine runs further above the wheels and less torque
  reaches them in 1st: the first live read to make. The relative velocity in `FUN_009bc0c0` also subtracts
  a `.bss` scalar 0x01186b74 × an input lane from its y component (probably gravity × dt; not modelled).
- The +0x210 blend-input scale: two leads, not followed to the end (decomp.md "Tyres" → Unknown): the
  constraint builder `FUN_009c24b0` zeroes it, and `FUN_009a56b0` (from `FUN_0095b100`) writes a vector there
  on some object with a flag at +0x4aa. If only the builder writes it, the blends get 0 (pure curves), not 1.

### 2026-10-06 (session 5): drivetrain: standstill, downshift gate, launch delay

- Standstill hold found and ported (decomp.md "Drivetrain and brakes"): `FUN_009d6170` builds the input at
  vehicle +0x1aa8 (brake, or throttle in a forward gear, below a signed speed threshold), `FUN_009b1180` turns it
  into a body velocity change of `99407bbc` m/s² against the direction of travel. The threshold (`.bss`
  0x01186e60) is unread; any positive value would hold the XKR's launch (6 m/s² < 15), which the reference does
  not show, so it is set to 0 pending a live read.
- Downshift gate: vehicle +0x1ed0 is the time on the ground (`FUN_009ae840`); ported with an assumed limit 0.
- Launch delay: the 0.45 s idle wait matches the forced-brake mask (vehicle +0x1c30) of the control block, whose
  setter is not found; the standstill timers serve reverse selection. Dead end: the gearbox states 0/1 and
  `FUN_009262a0` were not reached this session.

### 2026-10-06 (session 5): post-processing: composite constants, bloom, vignette, tint

Traced from the dump and the exe programs (details in decomp.md "Post-processing and colour grade"). No
rendering change this session: every stage still missing has at least one value that could not be traced
without the running game, and the brief was to leave a stage out rather than guess it.

- **The constant names are not in the code.** `kHdrToneMapping`, `kColorAddWeight`, `kVignette…`,
  `ColourCubeScalesOffsets`, `BloomColour` only occur inside the DXBC blobs (RDEF); no function references them.
  `GBufferComposite`'s pixel constants are `GenericVector4_A` / `_B` / `_D` (gObjectCB), set through the global
  constant table (`FUN_00a5d660(index)` returns the slot): A = 0x9c, B = 0x9d, C = 0x9e, D = 0x9f.
- **B and C** (`FUN_0089d5c0`, branch for material type 0x36): when bloom is on (a flag at +0x1518 of the view
  settings object and the renderer's +0x9836, both runtime), B = (Bloom.Colour.rgb, Bloom `ff564907`) and C =
  (1 / Bloom `76826c13`, Bloom `da328699`, 0, 0), with C = (1, 1) when `76826c13` ≤ 0.001 (`0x00f8bb34`).
  When bloom is off, B = (1, 1, 1, 0) (`.rdata 0x0100cc10`) and C = (1, 1, 0, 0) (`0x0100caf0`): B.w = 0
  removes the bloom term, so our composite without bloom equals the game's with bloom off.
- **PostFxKeyframe.Bloom** is unpacked by `FUN_008e51b0` into 13 floats (the order the renderer keeps them at
  +0xac74…): Colour.xyz, `da328699` (DarkBloomWeight), `ff564907` (Threshold?), `b303e3e6`, `41e5a069`,
  `3efef892` (Luminance), `76826c13` (DarkBloomWhitePoint), `c9f605e2`, `a3d336c1`, `cf7d9668`, 0. Field
  names are the cracked ones `mwtool genesys` prints (unconfirmed). `29c5d30d` (Saturation) is not read.
  `FUN_008e9610` blends the current and next keyframe's set (lerp by the transition) and then towards the
  state by its weight; `FUN_008eccb0` runs it for each PostFX state whose flag at +0xf4 is set.
  Bloom set-up `FUN_008eaaf0` receives `c9f605e2`, `a3d336c1`, `cf7d9668` (default keyframe `…78d9`: 1, 0, 0)
  plus three vignette vectors; that they become `kColorAddWeight` is likely but not followed to the end.
- **Bloom programs** (exe): downsample `0x0100ffa0` = 4 RGBM taps × 32, × 1/128 (the average), re-encoded
  as RGBM with M = max channel (≥ 1e-6, ≤ 1) rounded up to 1/255: RGBM covers 0…32. Blur VS `0x010109c8`
  places taps at 0, ±1.407333, ±3.294215 × `kScale` along one axis; blur PS `0x01010d38` weights them 6.533238
  (centre), 9.728171, 3.005209 (sum 32, i.e. normalised for the RGBM range: a 5-tap linear-sampled Gaussian)
  and divides by the 1×1 tonemap texture's w (the exposure) back to HDR units.
- **Vignette A and D**: A = renderer +0xa990, D = +0xa980, each multiplied per component by a vec4 at
  +0x15d0 of the view settings (set by `FUN_008e3e30`, an override setter). Nothing in the dump writes +0xa980 /
  +0xa990 with a literal offset; the writer was not found. The keyframe data (`Vignette.5e9c4fa6` = inner
  (1, 1, 1, 1), `5f000214` = outer colour, (0.04, 0.18, 0.27) in `…78d9`) suggest D = inner and A = outer −
  inner, but that is not traced, so the vignette stays out.
- **TintBlend weights** (`FUN_008c9e40`, the frame's post set-up, before `FUN_008e0930`): seven (resource,
  offset) cube slots from the frame state at +0x2c, +0x34, +0x3c, +0x44, +0x4c, +0x64, +0x6c with weights
  +0x54, +0x58, +0x5c, +0x60, +0x74, +0x78 (the last two replaced by +0x1560 / +0x1564 and the first two by
  +0x1568 / +0x156c when the view's +0x155e override is set). A missing slot falls back to the base cube
  (+0x2c). If the six weights sum above 1 they are scaled to sum 1; the base cube gets 1 − sum. The PostFX
  state's two `ColourCubeSettings` cubes and their [0, 1]-clamped weights (`FUN_008ecf80` → state +0x12e8…
  +0x12fc) are very likely the last two slots. Where the EnvironmentKeyframe's `ColourCube` enters (one of
  the base / first four slots) was not found.
- **EnvironmentKeyframe.Camera** carries the same two field hashes as the bloom (`da328699` at +68,
  `76826c13` at +72) next to the colour vector `96a4b35b` at +16: an environment bloom override, reader not
  found.

Open, needs the running game (`.bss` / heap reads, not possible this session): the bloom-enabled flag
(view settings +0x1518, renderer +0x9836); renderer +0xa980 / +0xa990 (vignette D / A) and +0xac74…+0xaca4
(the bloom set) on a frame; the seven TintBlend slots and weights in the frame state (+0x2c…+0x78), which
answers which slot holds the environment cube; which PostFxKeyframe is active in free roam (`…78d9` vs
`…78dc`). With those, bloom (traced kernel above), the vignette and the cube blend can be ported without
model decisions; the bloom level weights (`kColorAddWeight.z/.w`) also need a live read of the combine
program's constant buffer.

Note for the yellow-tint question: the default keyframe's bloom colour is blue (0.29, 0.42, 0.60) and its
vignette outer colour is a dark blue; both push the image away from yellow, so their absence is a plausible
part of the remaining warm cast (not measured).

### 2026-10-06 (session 5): sky, sun path and shadow cascades

- **The near-black upper sky** (noon, camera looking up) was the cloud layer decoded as sRGB (next item): with
  that fixed the patches read teal-blue. Still open: the clear sky's gradient (top stop ≈ (0, 0.11, 0.21)) runs
  at scale 1 under an exposure set by the noon key light (~11 units). The game scales the gradient by `BaseCloudColour.w` = s²
  (`FUN_008b27e0`; decomp.md "Sky, sun and shadows"), s a runtime environment-state float whose keyframe field
  is not identified, so the scale stays 1 (open). Dead end: fitting `SkyReflectionAverageColour` as the
  hemisphere average of gradient × s² + glow for the unnamed Sky fields `e570557e` (1.80 in the first
  keyframe), `d2f5bcb1` (1.07), `9f2051a4` and `BaseCloudIntensity` gives per-channel ratios 0.4–1.1 with a
  spread of 0.3–0.9 over 175 keyframes: none of them is the scale on its own (the average probably includes
  the clouds).
- **Clouds were decoded as sRGB**: the cloud textures (`…3f34`, `…3f3c`, …) are `BC3_UNORM`, and NewSky
  multiplies the sample straight into its HDR sum. Ours raised the RGB to 2.2, darkening the cloud colour
  ~2.5× (0.65 → 0.39 in the thick parts). Fixed: the texels are used as stored.
- **Dome mapping matches**: `FUN_008bca20` builds the dome (`Sky vertex buffer`, 0x7e vertices) as unit
  vectors (radius `DAT_00f8a820` = 1.0), so the gradient coordinate |POSITION.y| is our |dir.y|. Its
  `Sky texcoord buffer` holds x = the distance along the vertex direction to a sphere (centre and radius at
  `DAT_010cf400..0c`, .bss: the cloud shell, used by the horizon fade) and y, z = the cloud layers' planar
  coordinates (built from angles of the dome grid: constants π, 2π, π/2 at `0x00ffc024..2c`). Ours maps the
  cloud texture by azimuth relative to the sun and by elevation: not the game's (open, needs the grid maths).
- **Key light**: `KeyLightDirection` (constant slot 0xd) is copied from renderer +0x2920 in `FUN_008ad7d0`;
  the writer of +0x2920 (a struct copy, no direct store in the dump) and the reader of
  `EnvironmentSettings.SunMoonPath` (offset 0xf0, fields at 0x100–0x128) were not found. The provisional
  `SunMoonPath::key_light_direction` model is unchanged (open).
- **Shadow cascades**: `FUN_004a40a0` loads `ShadowMap_Constants{,2,3,4}` (slots 0x1c–0x1f) from environment
  state +0xf00..0xf3f; those are packed by `FUN_008a4910` (called from `FUN_008c8b00` with renderer +0x16f10, the
  shadow object; it reads its matrices at +0x90.., +0x220.., +0x3b0..). The fit that fills them (split
  distances, cascade extents) is not located yet, so our splits stay provisional (open).

### 2026-10-06 (session 5): input bindings

- Found from the action-name strings (`STEERLEFT` …, `.rdata` 0x00f9fd48 on): their pointer table at `.data`
  0x010c3540 (96 actions), the default builders `FUN_00482250` (XInput pad), `FUN_00483470` (keyboard, Windows
  virtual-key codes, not DirectInput scan codes) and `FUN_004843b0` (mouse), all calling `FUN_00ac72a0`, and
  the ini reader `FUN_0047cf90`. Tables and the mapping are in decomp.md "Input and control bindings".
- The Settings-screen reading is confirmed (A / Z / arrows / Space; RT / LT / left stick / X; Esc and Start
  pause; PgUp / PgDn tabs; R Ctrl = AUTOLOG_SETTINGS). New: nitrous = Left Shift / A, camera = C / LB, horn =
  W / B, look around = Delete, Page Down, Home, End / right stick, change vehicle = Q / Back, Easydrive = Num 4 /
  6 / 8 / 2 / d-pad (min 0.8, so only a near-full press counts). LOOK_BACK and LOOK_AROUND exist as actions but
  neither table binds them: how the original looks back by default (right stick down = ROTATECAMERADOWN?) is
  open; trace the camera code that reads action 10 / 37.
- Mapping: per binding `max(0, (raw − min) / (max − min))` (`FUN_00ac6850`); pad sticks per axis with dead zone
  0.2 rescaled, triggers 0.1 rescaled, `|v| / 32768`, `byte / 255` (`FUN_004119c0`). Keys are 0 / 1: no ramp
  before the controls are handed to `FUN_009aacc0` was seen on this path (the ramps, if any, are downstream:
  decomp.md "Update loop").
- Runtime: `crates/mostrusted/src/input.rs` holds the driving defaults and the mapping (unit tests); drive.rs,
  camera.rs and menu.rs read through it. Ours: steer arrows / left stick, accelerate A / RT, brake Z / LT,
  handbrake Space / X, camera C / LB, look around Delete / PgDn / Home / End / right stick (End or the stick
  fully back looks back, our rule), car menu also on Esc / Start / Back (PAUSE / CHANGE_VEHICLE). W / S / B and
  pad A / Y / R3 lost their old driving roles. No nitrous in the runtime yet.
- Open: the dead zones are `.rdata` initial values; a live read of the pad object (+0x140 / +0x144) would show
  whether the options change them. Whether the saved ini replaces or adds to the defaults.

### 2026-10-06 (session 5): Easydrive and the front end

- **Where the menu lives**: not in standalone GeneSys objects (`mwtool genesys all --type
  Widget_EasyDriveTierDefinition` finds none) but inline in the driving screens' `Screen2` objects
  (`UI/SCREENS2/371621.BNDL` free roam with the full tree; 1019407, 1019814, 1560359, 2173025, 2210073, 371670,
  371678, 371720, 371725, 604153 carry variants). The screen's widget groups (`WidgetGroup`, children in field
  `c7413006`) are the unit: one group holds the menu widget (`Widget_ShowLayouts` "EasyDrive": 3 layouts
  `01000000001b0f44`, `01000000001b8d10`, `01000000001a24e2`, scripts `Menu_Fast`, `EasyDrive_ScreenScript`,
  `EasyDriveSpeech`, `ExitOnIdleScript`) plus every item type (`Widget_EasyDriveTypeDefinition`); every other
  group with a `Widget_EasyDriveTierDefinition` is one tier with its item lists. Loader `mw_assets::easydrive`,
  listing `mwtool ui easydrive [screen]`. Layout in formats.md "Easydrive".
- **Tree** (labels from the English table): root `TO_BASE` (no title): RETRY LAST EVENT (`TO_RTY_CNF`, only
  with `Mode.RetryLastEventEnabled`), AUTOLOG RECOMMENDS (`TO_AR`), RACES (`TO_FMN`), EXCLUSIVE RACES (VITA only),
  CUSTOMIZE CAR (`TO_MOD`), CHANGE CAR (`TO_JACK`), MOST WANTED CARS (`TO_BLACKLIST`), MULTIPLAYER (`TO_MP`),
  STORE. Change car = MANUFACTURERS (`JSMANUFACT` source) -> MODELS (`TO_JS_MOD`, `JSMODEL`) -> LOCATIONS
  (`TO_JS_LOC`, jack spots) -> OPTIONS. Customise = MOD CLASS (+ LIVERIES -> `TO_PAINT`, `PAINTITEM`) -> MOD TYPE
  -> MOD STANDARD. Confirm tiers (retry, store, leave single player) hold NO / YES and the speech variants.
- **Behaviour from the data**: link items select through `LINKITEM`'s callback `ExitTierVia(Token)` (enabled
  by `Unlocked`), so a link token is the HSM transition naming the next tier. Generated tiers name their code
  callbacks: `EZStateJack::SelectManufacturer(Id)`, `EZStateJack::SelectModel`, `JACKSPOT::JackIt(Placement)`,
  `SelectPaintPackFromEasyDrive(PaintId)`, `UpdatePendingRollout_EasyDrive(UpgradeEnum)`,
  `SetVehicleAndRolloutFromSaveGameByVehicleIDString(Id)`. Inputs: the menu widget's user actions bind
  `EasyUp`/`easy_up` -> `EasyDrive::EasyDriveOnPrevious`, `EasyDown`/`easy_down` -> `OnNext`, `EasyLeft` ->
  `GoBackTier`, `EasyRight` -> `SelectCurrentItem` (only if `NumItemsShown >= 1`), `Hack_EasyDriveQuit`/`cancel`
  -> `QuickClose`; the HackEasyDrive widget opens it with `Hack_EasyRight`/`easy_right`. Idle close:
  `ExitOnIdleScript` fires `TIMEOUT` on `ScreenTimer` "EasyDriveTimer", `Delay: 1000` (unit not checked).
- **Code**: the strings (`ExitTierVia`, `EasyDriveOnNext`, `easy_right`, `SelectPaintPackFromEasyDrive` ...)
  are in `strings.csv` but `callers` finds no reference to any of them (registered through tables the dump has no
  xrefs for); no function found yet. Open: which `ConstantItems` / `ConstantItems2` adapter entries come from
  which list (the adapters are code), how `Signals.GetIntValue` sources are filled, the PC keys behind the
  `easy_*` mappings, the layouts' look.
- **Runtime first version** (`crates/mostrusted/src/easydrive.rs`, `--show-easydrive`): tree, titles and labels
  from the data; link items open their tier; the CHANGE CAR tier lists the catalogue's makes, then models, and
  drives the car via `GarageAction::Drive` (skipping the jack-spot location step); everything else is shown and
  inert. **Model decisions**: panel place / colours / sizes (fonts are the game's body and title fonts, the
  layouts are not read yet), 8 rows shown, keyboard `E` to open and Esc / gamepad B for `cancel` (navigation uses the original's EASY_* bindings, Num 4 / 6 /
  8 / 2 and the d-pad, since the merge with the input work; the arrows steer),
  all list items of a tier shown regardless of their source's condition (so confirm tiers show four rows and the
  root shows retry / VITA / store), paint not wired.

### 2026-10-06 (session 5): free-roam rules

- `TriggerData` (one instance, `HAWAII/TRIGGERS.BNDL`, 437680 bytes) decoded: header with three record arrays
  (6404 boxes, 53 spheres, 2585 points) found by reading `FUN_0053c2f0`, which indexes the three as one list
  (0x30 / 0x18 / 0x20 strides). Not Burnout Paradise's layout (no landmarks / generic-region split, no
  start position). Parser `mw_formats::TriggerData`, listing `mwtool freeroam`.
- Collectable kind codes come from `FUN_006e4920` (telemetry names). Base-game counts: 216 Billboard,
  152 SecGate, 74 SpeedCam, 183 JackspotLocation, 185 JackspotCar, 22 BodyShop. All are distinct places (no two
  of a kind within 5 m except one gate pair); every jack spot location has a car box beside it (2 extra cars).
  Heights (20–90 m) are road / car-park heights for those parts of the map; a check against the road
  collision is not done.
- Counts vs the game's totals: no "x of N" found in the data yet. `CollectablesConfig` (GLOBALCONFIG) has five
  id lists with counts 4, 35, 101, 35, 35 at its u16 fields 0x68..0x70; the 35/101 lists hold jack spot location
  and car ids (checked against TriggerData), probably per content pack; the 4-item list is not in TriggerData.
- Speed camera rule from `FUN_006ed0a0`: counts at |speed| ≥ `CollectablesConfig.d89eeda4` = 50 mph, records
  `(int)(100 × mph)`. Jack spots need the script flag `JackSpots.IsSwapSpotVehicleSpawned[n]` (`FUN_006e44f0`).
  Details in decomp.md "Free-roam rules".
- Dead ends: the strings "SpeedCam", "SecGate" … are referenced only from code (`FUN_006e4920`), no data
  tables point at them; `FUN_006f3f30(n)` is a show-once hint, not scoring; `GameLogicConfig.FirstSeenScores`
  holds ids, not point values.
- Open: (1) the spatial index (header +0x1c: 45 (ptr, count) pairs then 39 more at 0x5fa78, u32 trigger indices;
  +0x20/+0x24 = 45 × 39 cells, +0x28 origin (-4794.6, -4303.2), +0x30 cell 250 × 250 m, all inferred); (2) box
  `size`: full or half extents (full fits the objects: 10.5 m billboards, 19–35 m road-wide camera boxes);
  (3) record flags byte (+0x12): 2 on speed cams, jack spot locations, body shops, 0 on gates / billboards /
  jack spot cars; (4) the other ~55 kind codes (kind 4 points = 2518, probably spawn / roaming locations;
  kind 29 spheres = 41); (5) the overlap test that fires a trigger and the smash requirement for gates and
  billboards; (6) Speed Points per event; (7) `.bss` values to read live: `0x0118a060` (already open).

### 2026-10-06 (session 5): traffic

- `TrafficLaneData` (0x701): a single instance, `HAWAII/TRAFFIC.BNDL` (2.3 MB, 57 imports), next to 35
  `TrafficFlow`, 23 `TrafficFlowType`, 11 `TrafficVehicle` and one `TrafficVehicleTraits` GeneSys objects.
  Decoded and parsed (`mw_formats::TrafficLaneData`, `mwtool traffic [--png]`): 294 hull cells (~400 m quads,
  130 with lanes), 6758 lane sections, 52 607 rungs, 1005 spans, 5512 neighbour links, 36 junctions, 189 stop
  lines, 392 static vehicles, 374 km of lanes. Ground truth: the lane map drawn top-down is Fairhaven's road
  network (freeway loop, downtown grid, mountain roads), and 99.6 % of each hull's rungs lie in its cell.
- Verified across all hulls: record sizes (section 0x30, rung 0x20, span 12, neighbour 4, static vehicle 0x50,
  junction 0x150), every section's rung range within its hull, neighbour ranges within the list, forward /
  backward hull links below the hull count, section length = last cumulative rung length (cumulative lengths
  restart per section), span `f32` pairs are value and reciprocal. Section byte 0x24 = `TrafficFlow` index:
  confirmed in the code (`FUN_005f4d10` reads the flow's `Speed` through it, scaled by 0.44704 → mph).
- Dead end: hull header byte 0x04 read as u16 gave impossible neighbour counts; it is u8 neighbours + u8 static
  vehicles (static-record region sizes are multiples of 0x50 matching byte 0x05).
- Open (format): junction record (0x150: an id, 150 / 20 pairs, light masks), stop line (2 bytes, `00 01`),
  static vehicle record (matrix + 16 bytes?), section byte 0x25 (0..21, probably a `TrafficFlowType` index),
  span `u16` at +8 (a hull index?), hull header u16 0x08 / 0x0A (equal but once: traffic light range?), header
  0x2C-0x34 (three pointers at the flow import list), the vehicle-type records at 0x38 / 0x3C (0.65, 1200).
- Open (code): spawn / despawn distances, car count, per-flow density, hour-of-day density use
  (`TrafficSystemConfig` array), junction behaviour, reactions to the player, switch to a physical car.
  Leads: readers of `world+0x697fc`, `FUN_005e9350`, the "Crashing Traffic" net message in `00610000.c`.
  `.bss` value needed from a live read: GeneSys object at `DAT_0110f940` field 0x48 (scales
  `TrafficVehicle` field 0x10 in `FUN_005f4d10`).

### 2026-10-06: no Burnout Paradise bike code survives (dead end)

- Dump strings (23,963): no `bike`, `motorcycle`, `rider`, `wheelie` or `stoppie`. The only vehicle-type enum
  strings are `VEHICLE_TYPE_RACER` / `VEHICLE_TYPE_COP`, which sit among the Blaze `REQUEST_TYPE_*` matchmaking
  strings. They are an Autolog racer-or-cop flag, not a chassis type.
- GeneSys (`mwtool genesys-types`, 12,410 lines): no bike, rider or lean type. `BodyMovementBehaviour`
  (`Take_off_behaviour`, `Angle_control`) is the car body's movement, the source of the anti-roll bar values.
  It is not a helicopter or bike type.
- Install: no bike-named files. `VEHICLES/` holds only `VEH_*` car bundles and the shared lists.
- `VehicleInfo.Axle` stores a list of wheel handles plus a u16 count (2 on every car checked), so the data
  format could describe one wheel per axle. The handling code assumes left/right pairs, though: per-axle
  differentials (`FUN_009d8140`) and per-axle anti-roll bars (decomp.md).
- Caveat: engine functions are unnamed and have no RTTI, so a nameless dead function cannot be ruled out. With no
  strings and no data types left, though, nothing of the bike code is usable.

### 2026-10-06: the drivetrain from the code; the "2nd-gear clutch bug" is the tyres

- **Read from the dump** (details and addresses in decomp.md "Drivetrain and brakes"): the gearbox is a
  14-state machine (`FUN_009da340`); the engine (`FUN_009d9960`) and clutch (`FUN_009d7ac0`) run after it each
  tick. Corrections to our model: the shift phases are declutch 10 % / **neutral 50 %** / **clutch-in 40 %**
  (we had 40 / 50), with the gear changing at the end of neutral; the ±50 rpm random offset only feeds the
  launch clutch, not the upshift point; downshifts blip the engine for the whole shift; the rpm downshift rule
  is not gated by the throttle (its gate, vehicle +0x1ed0 against `.bss` 0x01187580, is unidentified; the
  original downshifts 3 → 2 → 1 while braking with no throttle); in gear the clutch opens below 20 mph with
  neither pedal, or below idle. The declutched engine still takes the throttle (torque = B + T × (full − B),
  B the resistance faded by the governor); ours ignored it, which is why our idle sat at a flat 1000 / 969.
  The throttle is peak-held (`c6deb7a6` per tick) and lagged (`36676b9c` per tick), bypassed during upshifts;
  a value-noise torque (rpm / 120 lattice points per second, amplitude from a second `Power_curve` three-point
  curve, 0 at idle on the XKR) reaches the rpm. Clutch-in brings the throttle back as pedal × progress^k.
- **The idle**: with the hold / lag time base 60 the governor's limit cycle gives 1003–1031 rpm, mean 1016,
  alternating tick to tick, against the original's 1002–1033, mean ~1015 (`ref/xkr_launch`); 1 gives 987–1029,
  an instant lag 968–1091. The `.bss` vector 0x01187250 that holds it is not read yet (handling.md "Model
  decisions").
- **The 1→2 shift** in `ref/xkr_launch`, tick by tick: one declutch tick (clutch 1 → 0), five neutral ticks,
  gear 2 on the fifth, then 0.25 / 0.5 / 0.75 / 1: T ≈ 0.167 s from the XKR's 0.15 s × (1 + 0.15 × r). Ours now
  does the same.
- **The original's clutch never locks under load.** Its engine runs ×1.9 / ×1.3 / ×1.14 above the driven
  wheels in 1st / 2nd / 3rd at full throttle (rpm against w2_omega × ratio × final drive) while the rear tyres
  sit at κ ≈ 0: the clutch is one speed-matching impulse per tick before the tyre solve, and the tyres pull the
  wheels back to road speed, so it slips with T ≈ I_eff × Δω / dt (≈ 250 / ratio² N m per rad/s, consistent
  across the three gears). Our clutch is the same impulse; the difference after the shift is the tyres: ours
  spin the rear tyres (κ 0.6 in 2nd, κ 0.2–0.3 with `MW_LONG_GRIP=1`) instead of holding them, so the engine
  sits on the tyres' slip instead of the clutch's. With `MW_LONG_GRIP=1` the 2nd-gear rpm takes the original's
  shape (4600 → 4430 → 6460) but the car is 0.5 s quicker to 60 mph: the longitudinal tyre cap at small slip,
  not the drivetrain, is the gap (handed on to the tyre work).
- **Replay** (`--replay ref/xkr_launch`, before → after): speed RMS over the launch (1–6 s) 1.86 → 0.83 mph;
  speed at 1 / 1.5 / 2 / 3 s after the throttle 4.8 / 13.1 / 20.4 / 36.3 → 6.6 / 14.8 / 21.3 / 38.0 mph
  (original 6.6 / 15.4 / 22.4 / 39.3); 60 mph after 4.55 → 4.58 s (original 4.63); 1→2 at 2.02 → 1.92 s
  (original 1.87); 2nd-gear rpm 6250–6680 → 5770–6720 (original 4750–6750; RMS 1105 → 636); idle 1000 flat →
  1003–1031. Worse: the first second, where the replay holds the handbrake (the original was parked in gear
  index 0 with the rear wheels locked): our handbrake takes the driven wheels off the clutch (`drive.rs`), so the
  free engine now revs to the limiter on the throttle (the original's rose at +109 rpm per tick with the clutch
  closed, i.e. loaded by ~150–190 N m, which fits the clutch formula with the driveline still connected, best
  with the front axle: `FUN_009d5520` / `FUN_009d7ac0` force the front share to 1 when vehicle vfunc +0x44 is
  true). Left for the handbrake work; 1st-gear rpm RMS 568 → 2493 because of it.
- **911 tests**: `--test accel` 0–60 mph 3.72 → 3.82 s, 0–100 km/h 3.87 → 3.98 s, 0–200 km/h 11.12 → 11.40 s,
  quarter mile 11.68 → 11.82 s, top 307.4 → 307.6 km/h (the clutch-in throttle ramp and the throttle lag at
  launch); `--test engine` idles 1002–1046 rpm (was 1000), revs and shifts as before, and now downshifts 3 → 2
  on the brake. `--test brake` 100–0 km/h 40.1 → 42.3 m (downshift blips while braking).
- Open: the launch reaches full torque in its first tick in the original (+140 rpm per tick = the full-load
  torque at 1028–1312 rpm) while the lag needs ~10 ticks; the 0.45 s the original waits between throttle and
  revs (parked, gear index 0); the constants 0x01187250, 0x011874d0, 0x011874a0, 0x01187580 and vehicle
  +0x1ed0. Procedure: with the game in free roam, `mwtool memscan`, `dump NFS13.exe+0xd87250` (and +0xd874d0,
  +0xd874a0, +0xd87580); add `engine_lag f32 … 0x184 0x1d14` and `gearbox_state i32 … 0x184 0x1bd8` and
  `x1ed0 f32 … 0x184 0x1ed0` to `handling.watch` and re-capture the launch and a braking run.

### 2026-10-06: the grip caps traced to the end; N is the static load; surfaces; no calibrated grip scale

- **No hidden factor.** `FUN_009c5da0`'s caps are μ × N × camber × (surface grip × axle `*_scale` × the per-wheel
  grip multipliers); `FUN_00905910` builds every input per wheel each tick. The multipliers (wheel +0x140) are 1
  outside a timed gameplay effect (`FUN_00900120`, lanes 0 / 1) and a post-event ramp (`FUN_009b2bb0`, lanes 2 / 3);
  the axle scales come from `Tyres` (`FUN_009d4ce0`); the surface grip is `Surface_Grip` blended by the car's
  `VehicleSurfaceProfile` link (`FUN_009cec40`), 1 on asphalt for every car. `LAT_GRIP` / `LONG_GRIP` (0.4 /
  0.7, fitted to real-world 911 figures) are gone. Details in decomp.md "Tyres".
- **N is not the spring force**: static corner load + ½ × the axle's downforce share (Tyre `5f032abb` would blend
  in the strut force; 0 on all 197 cars). `ref/xkr_launch` shows it: `w*_load_in` is 4576 / 4596 / 4071 / 4051 N
  throughout (= 1762 kg × g by the centre of mass), `w*_N` adds |`Lift_coefficient`| × |v_fwd| |v| (1.00 × v² in
  all) split 42.1 % front at body z `b821de16` and 47.6 % at `d21c15d0` with the brake ramp at 1. The body's lift
  force keeps the extra `ca386558` factor; the tyres do not. The lateral curve's exponent is × static / N.
- **The reference run agrees with the literal scale**: braking with only the fronts biting (κ ≈ −0.2) the XKR
  slows at 0.95–1.0 g = 1.74 × front static load / weight; sliding at 20–30° with locked fronts, a steady 1.95 g.
- **Its κ field is the post-solve slip**: the original's rear tyres read κ ≈ 0 while pulling 0.6 g in 2nd
  (the clutch slips instead); ours, read after the tyre pass, does the same (0.40 vs 0.42 at the 1→2 shift, 0
  from 3.5 s); before the pass ours reads 0.2–0.6. Telemetry now records the post-pass slip ratio. Tested with the
  drivetrain branch (04533ea) cherry-picked: 2nd-gear revs 4913 / 5278 / 5975 vs the original's 4831 / 5101 /
  5776 at 3.5 / 4 / 4.5 s; speed ~2 mph ahead; 60 mph 0.26 s early; 1st gear still spins more than the original
  (post-pass κ 0.7 vs 0.1 at 2 s).
- **Solver order and mass** (`FUN_009289f0` → `FUN_009c75b0`): one pass per tick per wheel, wheel after wheel,
  after the contact iterations; brake budget on the wheel's spin first, then the tyre; the force goes into the
  **chassis** (the 0x100-byte body records; the wheels are compact 0x80-byte spin records) with the chassis's
  effective mass at the contact (`FUN_009ca610`). Ported: brake-then-one-pass (the 6 sequential passes are gone),
  impulses applied wheel by wheel so each sees the velocity the previous left, chassis effective mass.
- **Surfaces and rolling resistance** (`FUN_009d8bd0`, `FUN_0054d750`, `FUN_009d4a10`): each wheel's surface
  (collision tag) → `SurfacePhysics` `RoadOverrides`: `Surface_Grip`, `5c2a9f18` (rolling, 0.01 on asphalt) and
  `7585f9cb` (slide-drag scale); the car's link adds |mph| × a drag ramping with the slip angle (0 → 4 N per mph
  over 15° on asphalt). Ported per wheel (`WheelSurface`), replacing our Crr 0.015; the flat test track counts as
  the game's `Default_surface` (rolling 0.01, grip 1, no slide drag). Not ported: the fade when the surface changes.
- **Side effects of the true grip**: the 911 and Veyron hold 2.6 g on the skidpad, the scripted brake-tap drift no
  longer reaches the 7° rear slip (results in handling.md), and braking below ~30 km/h tails off exponentially
  (the tyre pass is no longer saturated); the game's standstill hold (`BrakeBehaviour.99407bbc` = 15 m/s²,
  `FUN_009b1180`, scaled by vehicle +0x1aa8 whose meaning is unknown) is what should finish a stop: not ported.
- **Standstill**: the slip angle's floor in the game is a physics epsilon (`DAT_01188270`, compared against
  throughout the solver), so slip angles at rest are noise in both games; with `Minimum_speed_to_enter_drift` 0
  (XKR) our drift state now flickers on for a tick at 0 mph in the replay. The gate belongs in drift.rs (the
  original's bodies come to rest exactly). Our rolling resistance is clamped to what stops the contact (ours).
- **Open**: the tyre constraint's +0x210 scale on the slip-blend inputs (no writer in the dump; if it is large,
  any brake or wheelspin collapses the lateral grip, which is how a brake tap could start a drift); which state
  moves the downforce point to `35d32b7d`; vehicle +0x1aa8; the strut travel (`WheelSuspensionConstraint`
  `7fc0dc41`?) and the rebound damper slot (no reader found).

### 2026-10-06: update loop, steering, drift and handbrake re-read from the code

Every rule in the tick from the steering to the drift, checked against the dump line by line (decomp.md "Update
loop", "Low-speed turn-around assist", "Steering", "Drift", "Handbrake and burnout" have the details and
addresses). Changes to the model:
- **Order**: the steering runs after the tyres (`FUN_00904260` step 7), so each tick's tyres use the previous
  tick's wheel angle; ours now does too. The full-rate test (0.99) reads the raw stick, not the in-drift remap.
- **Drift side force pushes outwards.** It acts along the body's side axis with the sign of the drift scale S,
  and the yaw torque with −sign(S) about the up axis (`FUN_009ce240`, `FUN_009d0860`). The torque must turn the
  car into the drift (it is the donut torque, largest at full lock into the drift), so the force points away from
  the turn's centre. Ours pushed inwards; that was why the cap of 5 gave a 3 g "grip circle". The cap (`.bss`
  0x01187380) applies to the dimensionless factor, so the force can reach 5 × `09832a93` = 70 m/s²; it is only
  applied with every wheel down, while torque, scale and speed hold run whenever drifting.
- **Damping time base K = 60** is in the code: `(1 − c)^(dt × K)` in `FUN_00907b50` / `FUN_0091dad0` with K at
  `.bss` 0x01184cc0 / 0x01185640 (both read live as 60). Damping now applies before the forces and on the exit
  tick, as in `FUN_009d91a0`.
- **Steering in a drift**: the early counter-steer limit counts from `Slip_angle_to_enter_drift` (not 0) to
  `fb0656a3`; the counter-steer opening is raised to a power (`powf`, exponent not visible; taken as
  `Max_counter_steering_power`); the wheels themselves are clamped to the limit; the counter-steer rate blends in
  with body slip from the entry angle to `5277ec9e`, only while counter-steering against the rotation. The yaw
  boost of the side force is gated by the same counter-steer flag (ours had it inverted).
- **Handbrake** (`FUN_009d27b0`): a countdown reset to `Minimum_handbrake_time` while pulled, so it stays on
  that long after release. While on: rear brake torque = max(handbrake strength, service) (no forced lock: the
  game's lock mask comes from other states), the front axle takes all of the drive (`FUN_009d5520`: also on
  rear-driven cars; matches the drivetrain agent's live reading of `ref/xkr_launch`), no drift can start, and a
  drift ends (code 9). A drift that starts on the tick after release gets the handbrake's scale. The state is read
  through the vehicle's virtual +0x44 (vtable 0x01023758, entry 0x00920810, not in the dump), identified by
  these uses.
- **Burnout** (20 mph live): releases the front brakes on front-driven cars, else the rear (AWD keeps its front
  brakes); blends the driveline split towards that axle; blocks drift entry.
- **Ported**: the low-speed turn-around assist (`FUN_008f8360`, wall contacts from avian), the drift's exit
  steering scale `ca545e03`, the counter-steer torque gate (`c901c1e8` / `e40fa397`, 0 everywhere), the
  `de67c45d` forward hold (0 everywhere). Body movement has no per-tick stage (`BodyMovementBehaviour` is read at
  setup); `Angle_control` exists as a type only. `VehicleInfo.40634c09` (the drift torque's per-car scalar) is
  (1, 1, 1) on all 197 cars.
- **Results** (handling.md "Measurements"): the 911's brake-tap drift now spins (peak 72°, down to 10 km/h)
  instead of holding 22°; the MP4-12C holds 33° for 5.2 s (was 12°); the F-150's skidpad falls from 2.0 to 1.2 g;
  handbrake taps give 3–4° (F-150 2.7°, was 13.9°). Not tuned; needs the original's runs.
- **Open**: the speed-hold constant at `.bss` 0x01187250 (60 assumed; `memscan dump NFS13.exe+0xd87250`); the
  counter-steer exponent's source; the steering neutral offset's input (vehicle +0x2c70: no writer found); the
  collision exits and wall-hit steering reset (a contact value > 6, unit unknown); the handbrake state machine's
  `.bss` thresholds 0x011870f0 / 0x01187070 (no reader of its outputs found); the per-wheel surface factor.
- Dead ends: `FUN_00920810` (vtable +0x44) and the drift's `powf` exponent are not recoverable from the dump;
  `0x006e…` writers of a +0x2c70 belong to another class; `FUN_009d4f20` reads a curve, not `BodyMovement`.

### 2026-10-06: the tyre screech "horn" was a decoder bug; skid intensities from the game's code

- **Cause of the horn**: every 32 kHz `Wave` decoded to just its 47-sample PCM prefix. `mw_audio::sps` rebuilds
  EALayer3 granules into free-format MPEG frames of the nominal 640 kbps size, capped at 2304 bytes; at 32 kHz
  the nominal size is 2880, so frames were exactly 2304 bytes, and minimp3 (nanomp3) only searches free-format
  sizes *below* 2304 (`k < MAX_FREE_FORMAT_FRAME_SIZE`). It found no frame, and the 47 samples were played as a
  loop: a 681 Hz buzz (32000 / 47) with all its harmonics, times the item's pitch (1.1 → 749 Hz, 1.3 → 885,
  1.4 → 953: the "peak ≈ 950 Hz" of the 2026-10-04 measurement). Fix: cap at 2303. Affected: 18 of 281 waves
  in `HAWAII/SURFACESOUNDS.BNDL`, 9 of 119 in `SOUND/METALCAR.BNDL`, 16 of 141 in `SOUND/GLOBAL.BNDL`; all now
  decode to their header's sample count (e.g. `…f34e3` 65014 samples, 2.03 s), with no discontinuity at
  granule edges (|diff|/rms 0.14–0.19 inside and at edges).
- **The default surface's `Drift` sequence** (`…f4eae`, bound to `LatSlipIntensityForSkidSound` 0..1) has six
  wave layers: [0] 0.03–0.40 `…f34e3` pitch 1.3, gain 1–1.5 by speed 40–80 mph; [1] 0.065–0.33 `…f4ead` /
  `…f4eac` (48 kHz) pitch 0.6–0.7 by `Jitter`; [3] 0.18–0.44 `…f4eb4` (48 kHz) gain 2; [4] 0.33–0.83 `…f4eb2` /
  `…f4eb1` pitch 1.1 gain 2.5; [5] 0.62–1 `…f4eb5` / `…f4eb6` pitch 1.1 gain 2.5; [6] 0.43–1 `…f34e3` pitch
  1.4, gain 3 → 1 as speed rises 0 → 100 mph. Layers 0, 4, 5 and 6 are the 32 kHz waves, so above 0.44 the
  drift was nothing but the buzz. Decoded, they are tyre squeal like the others: lines at most +8 to +14 dB
  over their third-octave (the recordings' own squeal), energy mostly at 500 Hz–2 kHz.
- **Truly tonal layers by design**: the `Brake` sequence's items [2] (`…d4a7e`, a 5.24 kHz tone, pitch 0.9 →
  4.7 kHz) and [3] (`…d4a7a`, a 2.75 kHz sine) on channel `…d4a85` (gain 0.36), with curves peaking at 0.31 /
  0.19 between braking intensity 0.03 and 0.63. In a braking sweep they hold −25 to −28 dB of the total: faint,
  as the data sets them.
- **Measured** (`mwtool vehicle-sound --sweep lat --mph 40`, 13 s, intensity 0 → 1; "lines" = energy in bins
  more than 20 dB above their third-octave median): before, lines held −0.7 to −1.6 dB of the total from
  intensity 0.5 up (≈ 70–85 % of the sound was the buzz, peaks +33 to +41 dB at 750 / 1500 / 2244 / 3744 Hz);
  after, no such bins at any intensity, strongest line +8 to +15 dB. Level at full drift −16.2 → −11.9 dBFS.
  Scripted 12 s render: drift segment −16.7 → −12.5 dBFS, peak prominence +45 → +10 dB.
- **Skid intensities, from `FUN_009d1bf0`** (decomp.md "Tyre skid sound intensities"): one reference wheel
  (the first rear wheel, or the first front one on front-drive cars); lateral = slip angle 0.05° → 40° times
  speed 0 → 10 mph; spin = |tread − ground| 2 → 20 mph (a locked wheel counts); braking = pedal 0.3 → 1 times
  speed 0 → 40 mph, halved, or the speed factor alone under a vehicle predicate (assumed: handbrake); per-tick
  rise / fall lerps 0.5 / 0.1, 0.12 / 0.2, 0.05 / 0.5. The values come from `TyreSoundParams` (`…1b0ca7`,
  `AssetStore::tyre_sound_params`). Ours had: lateral full at 12° (so a 15° slide sat on the drift's top
  layers, the ones that were broken), spin on/off, braking from locked wheels or 0.45 × pedal, max over all
  wheels, 80 ms smoothing. In `--test drift` the lateral intensity now peaks at 0.75 (5.5 s above 0.1, 3.7 s
  in 0.6–0.8); `--test brake` locks the wheels, so the spin sequence plays (as the game's formula does) and
  braking reaches 0.48.
- **Open**: the runtime constants `0x0118a060` (slip-speed scale, read as mph) and `0x011870d0` (wheel-load
  threshold for spin) and what vehicle +0x1b80 and the virtual method +0x44 are (memscan dumps / the probe
  would settle them); `GetPlayerSuspensionLoad` (`FUN_006d92d0` → vehicle +0x1f28, smoothed per tick by
  `FUN_009aae90` towards the mean of two axle ratios) drives the drift / brake channels' gains (0.4 → 1, exp 2)
  and is not fed yet (0: those channels sit at 0.4 / 0.5); `BusMixer` / `Lfo` sequence items are ignored; our
  ×4 / ×3.5 / ×2.5 skid mix gains are still assumed. Confirming capture of the original: see PLAN M5.4.

### 2026-10-06: the yellow tint: ColourCube channels were swapped; the post chain lives in NFS13.exe
- **Where the grade happens**: the earlier entry ("tonemap, auto exposure and colour cube") found no PC post
  shader in `SHADERS.BNDL` that samples a cube except the rear-view mirror and assumed the main view does the
  same. The real final pass is one of 74 DXBC programs in `NFS13.exe`'s `.rdata`, which is not encrypted
  (`mwtool shader-asm --exe list`, new). `0x01016148` and its 11 variants (created in `FUN_008e41e0`):
  distortion offset, optional DOF, splatter overlay, then `out = Sampler3dTint(rgb ×
  ColourCubeScalesOffsets.xyz + .w)`, the last operation on the frame. The whole chain is in
  [decomp.md](decomp.md) "Post-processing and colour grade".
- **The grade volume is built on the CPU** ("TintBlend", `FUN_008e5f80` / `FUN_008e0930` / `FUN_008e0ac0`):
  a job blends up to seven ColourCubes with weights into a 16³ four-byte buffer, which is copied into the
  texture row by row. The kernels (`0x00eb56d0`… via the table at `0x010e8f70`, indexed by cube count) are not
  in the dump.
- **Cause of the yellow**: the cube bytes were read as "blue fastest, stored R, G, B". A straight blend + row
  copy keeps the stored order, and the shader samples x = red, so the stored data is the texture's own layout:
  red fastest, each entry B, G, R (+ alpha → B8G8R8A8). That is the same bytes with red and blue swapped in
  both the index and the output. The identity cube `…3e93` reads as the identity either way, which is why the
  old reading passed its test. With the fix, the noon cube `…4e220` turns mid grey 102 into (99, 112, 116):
  it cools the mid-tones against the orange key light (11.0, 7.95, 5.21), where the old reading warmed them to
  (116, 112, 99) on top of it. Supporting evidence from the PostFX states' bindings: the cooldown cube
  (`Status.IsInCooldown`, `…7898`) now reads blue-steel (grey 102 → (77, 102, 118)) instead of sepia, and the
  noon / dawn cubes give teal shade under neutral highlights. **Inferred, not confirmed**: the kernels that
  would prove it are missing from the dump; a screenshot of the original settles it (procedure below).
- **Change**: `mw_formats::ColourCube` now reads the layout above (`at(r, g, b)` returns the graded RGB for
  input (r, g, b)); `tonemap.rs` builds the texture with x = red as before.
- **Measured** (road-cliff viewpoint, `--units 20`, sRGB means; warm = (R + G)/2 − B):

  | hour | region | before RGB / warm | after RGB / warm |
  |------|--------|-------------------|------------------|
  | 13 | whole frame | 114 110 79 / +33.0 | 105 110 95 / +12.7 |
  | 13 | road | 70 63 58 / +8.4 | 57 63 71 / −10.6 |
  | 13 | concrete post (shade) | 90 93 94 / −2.7 | 73 93 108 / −24.3 |
  | 13 | white cliff rock (sun) | 171 169 154 / +15.6 | 165 169 164 / +3.1 |
  | 13 (downtown plaza) | whole frame | 176 171 148 / +26.2 | 174 171 163 / +9.5 |
  | 9 | whole frame | 118 117 98 / +19.6 | 131 126 102 / +26.3 |
  | 18 | whole frame | 116 115 105 / +10.6 | 126 122 108 / +16.3 |
  | 22 | whole frame | 147 132 115 / +24.0 | 169 151 125 / +34.5 |

  Noon (cube `…4e220`) loses its cast. 09:00 (`…1244fc`) and 18:00 (`…0f0862`) are close to neutral cubes, and
  22:00 uses the identity cube, so their warmth comes from elsewhere: the key light and fills, the missing
  TintColour (21–23 h: (0.745, 0.776, 1.0), blue), the missing bloom (noon BloomColour (0.29, 0.47, 0.6), blue)
  and vignette. The foliage stays golden: it is the textures' autumn colour.
- **Now questionable**: the provisional skylight in `sky::irradiance_cube` (SKY_FILL) was added to stop the
  grade turning shade olive, which was the swapped cube. With the fix, shade in the sun-lit noon frame reads
  blue (post −24). For comparison, the same noon frame with the skylight off (SKY_FILL = 0, not committed):
  road 48 48 49 (−1.5), post 37 36 37 (−0.5), whole frame +18.8: neutral greys, but the shade is much darker.
  Left in place until the original's shade colour is measured.
- **Open**: the TintBlend weights (does the environment cube replace or blend with the PostFX states'
  identity cube, and are keyframe pairs blended or the nearest taken, as we do?); who fills the composite's A,
  B, D (`EnvironmentKeyframe.Camera.TintColour`, `BloomColour`, the `PostFxKeyframe` vignette / bloom fields);
  bloom; the night warmth.
- **Confirming screenshot of the original** (not taken: needs the user): free roam, pause or park on the
  coastal road at (−1131, 27, 1627) looking towards (−1190, 15, 1720), clear weather, 13:00, no pursuit /
  nitrous (those states add their own cubes). `tools/orig/orig launch`, drive there, `tools/orig/orig shot
  <scratch>/orig_h13.png`; ours: `mostrusted --camera=-1131,27,1627,-1190,15,1720 --units 20 --hour 13
  --screenshot <scratch>/ours_h13.png`. Compare the asphalt and the concrete post's (R + G)/2 − B: the old
  reading predicts warm (+8 road), the new one cool (−11 road). The time of day in the original must be read
  from its clock (`GameTimeOfDay`), since the hour is not settable from the menu.

### 2026-10-06: crash rules and the crash camera (M3.3)
- **Rules** (`Physics.CrashingRules`, 14 instances: 10 in GAMELOGIC/GAMEPLAY, 4 in PHYSICS/PHYSICSCONFIG): ten
  `ImpactRules` arrays (named so far: `LandingRules`, `PlayerRules`, `RoadblockRules`, `TrafficRules`,
  `WorldRules`) with their counts; each rule has three float-array thresholds and an optional
  `DamageOverride`. `FUN_009a9fd0` is the test: damage over `1617a710`, closing speed (mph) over `2414aa68`, a
  third value over `d6c0e908` (empty everywhere). The player's rules come through its damage bar profile
  (`DamageBarProfiles.PlayerOffline` → `CrashingRules` handle at +0x34). Offline player: world 30 damage /
  70 mph. Write-up: decomp.md "Crashes"; values: handling.md "Crash camera".
- **Crash camera** = `Sequences2[1]` of `FeedbackMessageGroup` 457707 (`0100000000068b43`): slow motion
  (1 → 0.1, 0.14, 0.22), two `CameraSequenceItem`s with `BystanderCameraParams`, `HideHudSequenceItem`, a
  letterbox `UILayout`, a `SceneAnalysis` and mixer automation, 5.5 s on the real-time clock. Identified by
  content only: the code that plays it on a crash was not found (the item handlers sit behind vtables).
- **Names cracked** (`genesys-crack`, camera vocabulary): `Camera_Transform_Binding`,
  `Target_Transform_Binding`, `X/Y/Z_Offset`, `Target_X/Y/Z_Offset`, `Screen_Offset_X/Y`, `Maximum_FOV`,
  `Minimum_FOV`, `Use_Zoom`. Unnamed: `86980811` (10 / 1000), `cc8312fe` (50 / 5.4), `005c8fc7` (bool),
  `72b09fef` (10 / 3.07), `c6b768dd`, `8b9e63bb`, `af29af91`; 4-word searches only gave nonsense.
  `SAS.Analyser["Freeway"]`: analyser names are field hashes of the string (`Freeway` = 892525534,
  `Landing` = 1890182743).
- **Reset after a crash** (`FUN_006d0f40`, `FUN_006c9fb0`, `FUN_006d0180`): the last pre-crash pose, rolling at
  `min(max(30, 0.7 × mph), 50)` (`GameLogicConfig` `75d82dd5`, `a44eb2f3`, `74521df4`). The wait is
  `GameMode+0x44`; no `GameMode` object is in the PC bundles (`mwtool genesys all --type GameMode` finds none,
  and `SmashAndGrabConfig.GameMode` 2210000 is not a resource).
- **Dead ends**: the world impact's damage value and closing speed are computed inside the contact solver
  (`FUN_0093bc40` copies the thresholds into the response block; `FUN_008fab30` and `FUN_009bb640` move
  flags but the comparison was not found); the `DAT_0118a090` scale in the closing-speed test is `.bss`
  (read it live: `mwtool memscan`, `dump NFS13.exe+0xd8a090`; 2.2369 expected); the bystander and analyser
  maths (no in-place reads of `BystanderCameraParams`; analysers built by `FUN_007d9840`…).
- Implemented: `mw_assets::crash` (rules, sequence, bystander params, analyser, letterbox, reset speeds; all
  rules and the sequence verified by `crates/mw-assets/tests/crash.rs`), `crates/mostrusted/src/crash.rs`
  (detection, slow motion via `Time<Virtual>`, shots, HUD, letterbox, reset), `--crash-wall-width`,
  `MW_CRASH_TRACE`.
- Open: confirm against a capture of the original's crash cam (procedure in the session report: a
  `tools/orig` run into a wall at > 70 mph closing, screenshots at 0.3 / 1.25 / 3.25 / 5 s, and the bystander
  camera's position and FOV read with `memscan`); the analyser's y = −8; the shot FOV with zoom off; whether
  free roam resets rolling.

### 2026-10-05: `mwtool tune`; present and input paths of the original

- Read live through the tuning menu (XKR, car park, idle): the player car at table index 48; idle 1012 rpm;
  wheel loads N 4576 / 4596 / 4071 / 4051 (front heavier, sum ≈ 17.3 kN ≈ the car's weight); grip multipliers
  all 1; the `.bss` constants as in `consts.txt`. The `drift_threshold` field (vehicle +0x2c10) holds FLT_MAX
  while not drifting, so it is state set on drift entry, not a tunable threshold.
- From the dump (research for an in-game overlay that was not built; probe.md "Status"): the present wrapper is
  `FUN_0046fb60`, which calls `IDXGISwapChain::Present` (vtable +0x20) on the swapchain in `DAT_011acde4`
  (device in `DAT_011acd64`, created in `FUN_00d9bb20`); the frame end is `FUN_00476f20`. The pad is polled
  on an input thread (`FUN_0040f140`, also `FUN_004119c0`) through `XInputGetState`; DirectInput 8 is created in
  `FUN_0040fa40` for keyboard and mouse. Dead ends: `GetAsyncKeyState` is imported but never called;
  `DebugPrint` is a script binding to the debugger log, not on-screen text; no game text-drawing helper.
- Open: the pointer from the vehicle object to its loaded GeneSys handling resource, which would put grip curves,
  gear ratios and drift behaviour on the tuning menu; whether the duplicate value next to each `.bss` constant
  (e.g. 1.2 at both 0x011872a0 and 0x011872a4) is a second copy the code also reads.

### 2026-10-05: the probe loads (Phase 0); memscan poke (Tier 0)

- The unpacked game loads `xinput1_3.dll` and `dinput8.dll` by name (`orig modules`, from `/proc/<pid>/maps`),
  both from Proton's builtin directory by default, and EA's `IGO32.dll` overlay injects itself from the EA
  Desktop install in the prefix. A `xinput1_3.dll` next to `NFS13.exe` plus `WINEDLLOVERRIDES=xinput1_3=n,b`
  (Proton appends its own overrides to ours) loads our DLL; `LoadLibraryW` of the full `system32` path then
  gets Wine's builtin behind its placeholder file, so forwarding works. Wine's `xinput1_3` has no
  `XInputGetDSoundAudioDeviceGuid` export.
- The DLL is loaded after the wrapper has unpacked `.text` (the game asks for XInput during its own init), so no
  waiting was needed. The vehicle-table chain `[[[NFS13.exe+0xcedb04]+4]+0x53220]` resolves ~1 s after that,
  on the title screen already; `player_index` = 48 is the gameplay signal. Seven minutes of free roam with the
  DLL loaded: no change in behaviour, input or readable state.
- `memscan poke` verified: a code address is refused, a `.bss` constant can be changed and is restored by
  `unpoke` and at exit. The detour step of Tier 1 is not written (probe.md "Status").

### 2026-10-04 (session 3, end): the exe's headers; the probe designed

- `NFS13.exe` on disk (PE headers, not code): image base `0x400000` with **no `DYNAMIC_BASE`** (DllCharacteristics
  0x8100 = NX compatible + terminal-server aware), so the game is never relocated and every address in the dump
  and in decomp.md is the live address. The import table names only `SHLWAPI.dll`, `KERNEL32.dll` and
  `USER32.dll`: those are the wrapper's; the game's imports are bound after it unpacks. `.text` is 0xb80e81
  bytes at `0x401000`; the wrapper is the unnamed 0x12f6-byte code section at `0x11dd000`.
- The game runs in Proton 11 (new WoW64: 32-bit PE in a 64-bit Unix process). The prefix has Wine's builtin
  `dinput8`, `winmm`, `version` and `xinput1_1..1_4` in `system32`. Toolchain for a 32-bit Windows DLL is not
  installed (no `i686-w64-mingw32-gcc`, no `i686-pc-windows-gnu` target).
- Design for running our own code inside the running game written up in [probe.md](probe.md); policy added to
  reverse-engineering.md. Open: which DLL names the unpacked game loads (`/proc/<pid>/maps`), whether this Wine
  implements `AppInit_DLLs`, whether the wrapper re-checks `.text` after start-up.

### 2026-10-04 (session 3, end): driving the original ourselves; the first reference run

- **`tools/orig/orig`** runs the original headless inside gamescope (Steam launch option `tools/orig/orig-wrap
  %command%`; normal launches unaffected) and drives it: XTest keys into gamescope's own X display, screenshots
  via `gamescopectl screenshot`, memory reads from `handling.watch`, scripted steps (`waitfor speed_mph > 60`,
  `capture`). Getting keys in: headless gamescope never sends WM_TAKE_FOCUS, so Wine ignored all keys until the
  tool sends it to Wine's top-level window itself (`activate()`). The menus then take keys; driving reads only the
  device selected in Settings > Gameplay > Controller, so automated driving needs "Keyboard" there (restore the
  user's "Xbox 360 Controller for Windows" afterwards, as done this session). The virtual uinput pad is not seen
  while Steam Input is on for the game (Steam takes controllers through hidraw; nothing opened the device).
- **The original's controls** (this install): keyboard Left / Right steer, A accelerate, Z brake, Space
  handbrake; pad left stick, RT, LT, X handbrake.
- **First reference run** (`ref/xkr_launch`, Jaguar XKR, keyboard full throttle 5 s from rest, then full brake;
  replayed with `--replay`, compared with `mwtool compare`): the launch matches closely (original / ours at
  3, 4, 5, 6 s: 22.7 / 20.3, 39.5 / 36.6, 53.4 / 53.4, 63.7 / 65.3 mph; distance at 6 s 68.9 / 66.4 m; shifts
  at the same times). Ours trails by ~2 mph off the line. **Bug found: in 2nd gear our revs sit at ~6400 while
  the original's rise 4830 → 6430 with speed: our clutch does not lock after the 1→2 shift (XKR).** The braking
  half is unusable: the original yawed at −34°/s with ~1 g sideways and entered the drift state (it hit or spun
  on something in the car park). The original idles at 1004–1027 rpm (ours: a flat 1000 or 969).

### 2026-10-04 (session 3, later): constants read from the running game; the engine's 0.74 s steps

- **memscan fix**: `/proc/<pid>/maps` paths with spaces ("Need for Speed(TM) Most Wanted") were cut at the first
  space, so `NFS13.exe+…` addresses never resolved (the first `consts.txt` / `porsch.csv` were empty).
- **Live vehicle**: `handling.watch` (git-ignored) reads the player's car through
  `[[[[NFS13.exe+0xcedb04]+4]+0x53220]+4+8·i]` with i = 48 in free roam (`player_index`). Checked against the
  speedometer and the body velocity. First readings (911, accelerating in 2nd): tyre loads front ~2460 N, rear
  ~4000 N (sum = the car's weight), per-wheel grip multipliers all 1, drift entry threshold 7°.
- **`.bss` constants** (docs/decomp.md, read with `dump`): drag K 1.2; anti-roll scale 60; drift damping time
  bases 60 / 60; side-force cap 5; burnout speed 20 mph; clutch cap 5000 N m; shift phases 10 % declutch / 40 %
  neutral; downshifts × 0.5 time; downshift hysteresis 2 mph; idle governor 20 rpm, 0.05 per rpm, up to 0.5
  throttle; downshift blip × 2.5; declutched below 0.1; the steering's full-rate threshold 0.99 (inputs below it
  steer outwards at the raw `High_speed_steer_rate`); the slip-ratio stop threshold 0.01; g 9.81; and 60/2π at
  0x01187310 (the check value). Applied to the model.
- **Not consistent yet**: with the game's side-force cap (5) our side force reaches ~3 g and the brake-tap drift
  turns into a tight grip circle, so our reading of its size, boost or direction is off; the model keeps a cap of
  1 until the side force (`drift_side_f*` in the watch file) is recorded in a drift. With the 0.99 steering
  threshold the handbrake test's half-stick tap gives 4° (17.9° with the earlier guesses); whether that matches
  the original needs a recording.
- **Engine "steps" in game** (user: "a constant rpm held for about a second, then the next"): recorded the game's
  own audio stream (`parecord --monitor-stream`) and tracked its pitch: plateaus of ~0.7 s while the rpm rose
  smoothly (27 rpm per tick). The offline replay of the same rpm trace was smooth. Cause: `PlaybackSettings::LOOP`
  makes bevy_audio wrap the source in rodio's `repeat_infinite`, which buffers it 32768 samples (0.743 s) at a
  time, each rendered with the parameters of the moment it starts. The engine and SFX sources never end, so they
  now play with `ONCE`: the recorded pitch rises continuously. Dead ends on the way: an adaptive per-parameter
  ramp in `sequence::Player` and interpolating the rpm between physics ticks (both kept; neither was the cause).

### 2026-10-04 (session 3): handling rebuilt from the game's code

The user supplied a decompilation of the game (`re/NFS13_dump/`, git-ignored; rules in reverse-engineering.md).
Findings per subsystem are in [decomp.md](decomp.md); the model built from them is in handling.md "The model".
- **Method**: GeneSys fields are read in place as `*(res + 0x28) + FIELD_OFFSET + instance`, so the offsets from
  `mwtool genesys-types` identify the handling code; `.rdata` / `.data` constants come from the unencrypted parts
  of `NFS13.exe` on disk. About 30 tuning constants live in `.bss` with no initialiser in the dump; the memory
  dump the pseudo-C came from (`NFS13_dump.exe`, not on this machine) or `mwtool memscan` would give them.
- **Settled**: 60 Hz tick; the steering range / rate rules and the in-drift input remap; the drift state machine
  (rear-tyre slip entry, brake-tap threshold, signed drift scale, Bézier yaw torque with the donut top, body-axis
  side force, peak-latched damping, speed hold); engine as its own inertia with a friction curve, clutch, no
  efficiency loss, shift on rpm and speed, top-gear lock (911: 6 gears); brakes pedal² with a ramp, no ABS / TC;
  spring k = load / sag, asymmetric damper, anti-roll bars; per-axis drag with an over-speed ramp and lift
  (`Down_force` unused, which explains the parked downforce branch's regressions: wrong fields, ~7× too strong);
  the tyre as a capped velocity constraint with the game's curve shapes and slip blends (no friction circle).
- **Field names corrected**: `0c5f89b3` is a damper ratio (not a frequency), `41543f85` the static sag (not
  travel), `Engine_resistance` the declutched deceleration (not engine braking), `35e18207` / `6ce0fda7` the
  handbrake's drift scale (not grip), `0dce38f7` / `8f151151` / `b5290695` drift-entry angles and rate.
- **Results** (911): 0–100 3.88 s, top 311 km/h with no drag fit (data 304; drag K = 1), 100–0 40 m, skidpad
  1.01 g, brake-tap drift 22° held 4.5 s. Calibrated (model values): lateral / longitudinal grip scales 0.4 /
  0.7, clutch cap 1.6 × peak torque.
- **Bugs met on the way**: braking with brake and tyre as separate steps made locked wheels cycle (the tyre spun
  them back up each tick) — now solved together in sequential-impulse passes, as the game does; clamping the
  revs after integrating ate the drive near the limiter (the game clamps first); the speed hold's gain is per
  tick (the game logs it × 60); a degenerate tyre frame after a crash gave a NaN camber.
- **Open**: the absolute grip scale and per-wheel grip multipliers (measure N with memscan); the handbrake's lock
  mask condition (inferred: the handbrake); the steering neutral offset's input; the full-throttle donut (the 911
  no longer spins from full lock and throttle at rest; the rear slip stays under the 7° entry); the clutch cap
  and shift-phase constants.

### 2026-10-04 (engine sound from the game code): audible RPM, load, harmonics, the Ginsu player, the EQs

The user supplied a Ghidra decompilation of the exe (`re/NFS13_dump`, git-ignored). It settles several of the
"engine ground truth" entry's open questions; where the code is missing (not decompiled) this says so.
Function addresses are given so the reading can be rechecked; no decompiled code is copied.

- **Audible RPM is not the physics RPM** (reverses the previous entry's change). `RaceCarEngineSound` holds
  a fixed RPM range of 1000..8000 (constructor `FUN_0058c3f0`; never overwritten) and `GetAudibleRPM` /
  `Players.*.GetPlayerAudibleRPM` (`FUN_0083eb60`) return `min + (max − min) × normalised RPM`. The
  normalised RPM (vehicle `+0x1f10`, `FUN_009aea50` ← `FUN_009ae930`) is the idle..max fraction of the engine
  RPM (`Engine.Power_curve.Idle_RPM` / `Max_RPM`, `+0x2c` / `+0x38`) raised to `2 − a`, where `a` is an
  `EngineSound2` term crossfaded by engine load between `cb44d82d` (off load) and `a33d84fe` (full load), or
  `913bb900` when a vehicle flag (`+0x1f45`) is set. The 911 has 0.7 for all three: exponent 1.3, so 4400
  physics rpm (half the range) plays as 1000 + 7000 × 0.5^1.3 = 3840 audible rpm, and the idle loop alone
  (sequence axis 1790..7500) covers physics revs up to about 2300. The pow's argument order is read from the
  register use around the SSE `powf` call; the small offset `_DAT_01186680` and the second branch of
  `FUN_009ae930` (a term in a global tuning field `+0xd0`) were not resolved. A sum of a short history buffer
  is added before the clamp (gear-change blips?, not reproduced). Implemented in
  `engine_driver::normalised_rpm` / `audible_rpm`; `MW_AUDIO_RPM_LINEAR=1` restores audible = physics RPM for
  A/B.
- **Engine load** (`GetEngineLoad` → vehicle `+0x1f20`, `FUN_009b1f70`): a one-pole follower, rate =
  `EngineSound2.99d4b91e` (0.5) × an unresolved constant, towards `s + (1 − s) × h`, with `s = (clamp01(speed /
  reference) × Engine.937338e7 (0.75) × ?)²` and `h` a 0..1 value at vehicle `+0x1d14` whose writer was not
  found (read as the pedal demand); the output is the maximum of the instant blend and the last six smoothed
  values (peak hold). Implemented as `engine_driver::LoadFilter` with a 100 ms response and the top speed as
  the reference; the hold keeps the on-load layers for ~100 ms after a lift.
- **Tracked harmonics** (`FUN_00586330`, `FUN_00574ff0`): firing frequency = audible RPM / 120 × cylinders
  (`EngineSound2.ed754b99`, 6 on the 911); `TrackedHarmonic1Frequency` = firing × `7cbc11ca` (1),
  `TrackedHarmonic2Frequency` = firing × `7cbc11c9` (3; was 2 × harmonic 1 here), `SubharmoniserFrequency` =
  firing × `283e07de` (1), tunnel harmonic × `dbcddcfc` (6) with gain `11a77b36` and Q `64cbe1ff` (not
  bound in the 911's data). `CombRaspDelayTime` = 1 / (firing × `f0291dd3`). The LFO bindings are
  `0.5 × sin` of a phase times an amplitude (`FUN_00aabc30`), `SkidLfo` a sawtooth; `Jitter` = two member
  sums. `EngineOn` is vehicle `+0x1ef0`, `EngineGainDueToPower` `+0x1d1c` (writers not found).
  `GetNormalisedRPM` and `GetAudibleRPM` themselves are thunks the dump lacks.
- **Ginsu player** (`GinsuPlayer`, process `FUN_00aaf0a0`; helpers `FUN_00aad240` grain index at a
  position, `FUN_00aacf60` position for an RPM, `FUN_00aad040` grain start, `FUN_00aad0f0` grain length,
  `FUN_00aad3c0` copy / linear resample): the recording plays at its native rate (the copy is called with
  equal in / out counts: pitch comes from the grain choice alone, not resampling); every other mixer block
  (256 samples, the block counter at mixer `+0xc480`) the player may jump: it picks a grain within a window
  around the RPM's grain (window = parameter 1 of the plug-in, default unknown), favouring staying or stepping
  back one or two cycles at random, and jumps by whole grains from the fractional current position (phase
  preserving) with a 0.25 ms linear crossfade (12 samples at 48 kHz). The RPM parameter glides linearly over
  33 ms. Our overlap-add synth already keeps phase; adopted the 33 ms glide (`sequence::GinsuGroup`,
  replacing the 10 ms one-pole). Chunking keeps each copy ≤ 64 input samples (two XAS blocks).
- **DSP**: `PeakingIir2` (`FUN_00d01e60` / coefficients `FUN_00ced070`) is the RBJ peaking EQ with
  `A = √Gain` (Gain linear, 1 = bypass) and Q clamped to 0.2..20, which is what `dsp::Biquad::peaking` does.
  `HighPassButterworth` (`FUN_00cff950` → `FUN_00cde880`) and `BandPassIir2` (`FUN_00ce65b0`) are standard
  designs with the frequency clamped to (0.001π, 0.999π). **`Distortion` ('Dst0') and `SubHarmoniser`
  ('SUBH') are registered (`FUN_004b2cc0`, descriptors at `010d1930` / `010d2410`) but their code lies in
  gaps Ghidra did not decompile (`00ab0e30..`, `00aad940..`), so they stay stand-ins.** Their parameter
  tables give ranges: Distortion default 1, 1e-5..1000 (a drive), Model −100..100; SubHarmoniser Interval
  default 2, 0..25. Stand-in changes: the distortion is now `tanh(d·x)/d` (unity for small signals, so the
  drive-4 strain bus no longer rivals the clean path; `MW_AUDIO_DIST=raw` keeps `tanh(d·x)`), the octave
  divider outputs at the input's RMS with 2 ms edges and tracks nothing above 1.5 kHz.
- **Per-car `DspParam` overrides**: the index ↔ runtime parameter mapping is still not in the dump. Two
  readings are consistent across the data and now applied: `Parameter_name` 1 on a filter = Frequency (the
  911's global 6.5 kHz EQs 472213 / 472218 become 400 Hz × 2 and 1 kHz × 3; HighPass 472227 before one
  sub-harmoniser becomes 6810 Hz, i.e. that bus is switched off), 0 on a Send = its gain (472306 = 0 mutes
  the comb-rasp delay path). Delay "4 = 0.2" and BandPass "5 = 40" exceed the plug-ins' parameter counts
  and are left alone. Send gains (`SendDspPlugIn.Gain`, all 1 in the 911) are now applied.
- **Measurements** (`mwtool engine-sound 911`, scripts in the session scratch `engine/`): steady 4000 rpm
  full load, bands in dB relative to the 500 Hz band, raw set-A recording 20 Hz −23 / 60 Hz −6 / 120 Hz +6 /
  250 Hz +8 / 2 kHz −9 / 4 kHz −15. Wet before: −12 / −0.5 / +6 / +3 / −10 / −14 (the +11 dB sub-bass).
  Wet now: −24 / +0.5 / +7 / +9 / −12 / −19 (within 2 dB of the recording up to 1 kHz; 60–120 Hz +6 dB of
  which the sub-harmoniser stand-in is 5 dB; the top end 3–6 dB darker since the 6.5 kHz EQs moved).
  Harmonic / total energy 0.98 (before 0.87, recording 0.97). Off load the 100 Hz EQs (× 4 Q 2, × 5 Q 0.5) still
  put the 60–120 Hz bands 7–13 dB over the off-load recordings: that is what the data's EQ does. Default
  sweep: f0 tracks the game mapping within −41 cents median (the glide lag), local pitch spread p90 124
  cents (before 224); dry spectral centroid 422 Hz (before 469: the curve lowers mid-sweep pitch).
- **A/B switches**: `MW_AUDIO_DRY=1` (all plug-ins off), `MW_AUDIO_NO_DIST`, `MW_AUDIO_NO_SUB`,
  `MW_AUDIO_NO_EQ`, `MW_AUDIO_DIST=raw`, `MW_AUDIO_RPM_LINEAR=1`; `MW_AUDIO_METER=1` with `mwtool
  engine-sound` prints each channel's level and plug-in chain with the applied parameters.
- **Dead ends**: the `GetNormalisedRPM` / `GetAudibleRPM` thunk targets (`LAB_005a2190` / `LAB_005a2240`)
  are not in the dump; vehicle `+0x1d14` (load source) has no scalar writer in the decompiled files; the class
  descriptors' parameter tables carry no names; the `DspParam` reader (`EngineSound2 +0xd8..+0xe4`) was not
  located (the +0xd8 hits in `0057/0058` are other GeneSys types).
- **Open**: listen to `after2_911.wav` vs `before_911.wav` (scratch) and in game with / without
  `MW_AUDIO_RPM_LINEAR`; the Distortion model and the SubHarmoniser law need the two undecompiled
  functions (re-run Ghidra over `00aad940..00aade60` and `00ab0e30..00ab1110`); the Ginsu window parameter
  default; the `DspParam` indices 4 / 5; `EngineGainDueToPower`; the state lists.

### 2026-10-04 (session 2): "everything is yellow": skylight in the ambient
- User report: the frame looks yellow. Causes: the noon key light is warm (11.0, 7.95, 5.21), the ambient rig
  is small (up fill 0.065 × 2.6), and the noon ColourCube warms shadows and mid-tones (grey 102 → (116, 112,
  99), 68 → (61, 55, 49); highlights stay neutral). With no blue skylight in the shade, the grade turns it
  olive (the car's shadow on the grey test track).
- The data's own balance: SkyReflectionAverageColour (1.12, 1.69, 2.44) is strongly blue. Warm sun + that sky
  on a horizontal surface sums to about neutral white, which looks like how the keyframe is meant to balance.
- Change (provisional, `sky::irradiance_cube`): the irradiance cube = the rig's fills + the upper hemisphere
  at SkyReflectionAverageColour, cosine-weighted ((1 + n.y)/2). Shade is now blue-grey, white stone white.
- Found on the way: the `Diffuse_1Bit_*` forward pass lights with `IrradianceQuadric` (7 × vec4 in
  gGlobalCB: second-order SH as a quadric), the CPU-built ambient; its G-buffer pass writes only RT0 / RT1
  (albedo = texture × materialDiffuse, roughness 1). The golden foliage is the textures' own colour
  (Fairhaven's autumn trees), not a shading error.
- To settle the ambient: a reference screenshot from the original (shade colour next to sunlit grey).

### 2026-10-04 (audio session, later): tyre screech, road, wind, impacts and scrapes from the sound data
- **Tyres** come from the surface data, not from `TyreSound` (its waves in `VEHICLES/WAVES.BNDL` are all the
  silent 0.1 s placeholder `…79ff`). `HAWAII/SURFACELIST.BNDL`: `Surfaces { Default_surface, Chevron_surface,
  Surfaces [65] }` → `SurfaceType { two condition sets (f395b019 / 7348be25, read as dry / wet) { Flats, Normal,
  Offroad, Rims, Track: { Surface_Physics, Surface_Rumble, Surface_Sounds } } }` → `SurfaceSounds { Brake,
  Drift, Spin (+ _Competitor), Reflection, two speed-bound road-noise sequences (4648873a, 1376f03b), Bumps,
  Transitions, TyreParams }`. Default surface, dry, Normal → `…3d399`. The skid sequences bind
  `RaceCars.Entities["Id"].LatSlipIntensityForSkidSound` / `SpinIntensityForSkidSound` /
  `SpinBrakingIntensityForSkidSound` (0..1) and layer loops and one-shots across that range (the drift: 15
  items; it swaps layers rather than getting louder). The game computes the intensities from `TyreSoundParams`
  (fields unnamed; Lateral 0.4 / 1.3 / 25 / 20 / 18 / 80 mph...). Ours (assumed): lateral = slip angle 4° → 12°
  (= 0.4 → 1.3 × a ~9° peak angle) faded in from 2 to 8 m/s; spin = a wheel spinning under power; braking =
  a locked / sliding wheel, or 0.45 × pedal under hard ABS braking. Inputs from `Car::wheel_contacts()` (slip
  split along the car's heading).
- **Wind**: `VehicleSound.Player_Sequences` (`SOUND/GLOBAL.BNDL`; 12 entries once the handle-array fix
  applies) has a speed-bound sequence (`…87ea6`, 0–180 mph) with wind layers crossfaded 45–90 / 70–138 /
  100–250 and 19 mixer items (bus ducking by speed: those target the engine's buses, which live in the other
  player, so they do nothing here). The other player sequences are wall reflections (`WallTracker...`), cop
  distance, dust clouds and damage; not played.
- **Impacts**: every car's `Vehicle_Sound.Collision_Sounds` (UniqueId 140990) is the `CollisionSoundSet` in
  `SOUND/METALCAR.BNDL` (`…226be`): 31 unnamed sequence slots (timelines of crunch and debris waves, random
  pick per item) and 24 wave pools (named: `Scratch`, `Sparks`, `Landing` (empty)). Bindings inside: 
  `RaceCars.GetMostRecentPlayerImpactClosingVelocity` (ranges 2–60, read as mph), `GetMassForLastCrash`,
  `MostWantedCarInLastCrash`. Which slot is which collision type is unknown: the 21 timeline slots are sorted
  by loudness into light / medium / heavy cues (closing speed < 4, < 12, ≥ 12 m/s) at 0.4 / 0.8 / 1.4 of the
  impact level. `Scratch` (two 3 s one-shots) becomes a loop (crossfaded seam) bound to the body's sliding
  speed; `Sparks` a cue on hard hits and scrape starts. Landings after ≥ 0.25 s airborne use the impact cue at
  the touchdown speed. Body contacts come from `particles::ImpactEvent` (closing and sliding speed).
- **Playback**: one more sequence player (`crates/mostrusted/src/sfx.rs`), separate from the engine's; it uses
  copies of the engine's player and loader (`mw_audio::sfx_player`, `sfx_dsp`, `mw_assets::sfx_loader`, no
  Ginsu) while the engine path is reworked, with a 20 ms release on stopped voices. Our mix (the bank ends in
  buses outside the bundles, like the engine's): master 1.4, skid sequences ×4 / ×3.5 / ×2.5, impacts ×3,
  scrape ×2.5 (applied to the items' gain values).
- **Measurements** (`mwtool vehicle-sound`, scripted 12 s; `tests/vehicle_sfx.rs`): road + wind at 50–60 mph
  −24 to −26 dBFS, drift −16, locked brakes −15 to −16, wheelspin −17, heavy impact −6, scrape −21, wind at
  120–160 mph −22; no limiter hits. Drift spectral flatness 0.000 (a tonal squeal, peak ≈ 950 Hz, centroid
  820 Hz), locked brakes 0.05 (centroid 1.3 kHz), scrape 0.30 (noisy, centroid 3.7 kHz), road and wind < 0.01
  (rumble, centroid 170–290 Hz). Discontinuity peaks (3.0) are in the scrape and match the raw `Scratch` wave's
  own (3.0). In game (`MW_SFX_TRACE`): `--test drift` lateral intensity above 0.3 for 5.9 s of the 5.8 s drift,
  braking only during the 0.36 s brake tap; `--test handbrake` locked 3.8 s; `--test brake` ABS squeal 1.9 s;
  `--test crash` one heavy impact at 31.9 m/s; `--test accel` no wheelspin (traction control).
- Open: surface types (collision tags) to pick other surfaces' sounds and their bumps / transitions; which
  collision slot is which (car vs wall vs traffic, front / side / rear?); `TyreSoundParams` field names;
  ambience (`AmbienceSequences` loop with `JumpTimelineController`s, not implemented), wall reflections, horn
  (`Vehicle_Sound.Horn` UniqueId 143356, no input), turbo / blow-off (EngineSound2 bindings, engine path).

### 2026-10-04 (engine ground truth): what the recordings say, status and next steps

Session cut short (user paused). The user hears the engine as "still wrong"; this session measured the synth
against the raw recordings instead of against its own metrics. Scripts are in the session scratch dir
(`/tmp/claude-1000/engine-scratch/*.py`: `ana.py` helpers, `grains.py`, `grain_phase.py`, `hold.py`,
`resynth_compare.py`, `graph.py`, `compare_bands.py`); regenerate the wavs with the commands below.

- **Decoder is clean** (911 main rev-up `01000000001d1c7c`): spectral flatness 0.0035 (tonal, no hiss), |diff|
  at XAS frame boundaries / inside frames 0.99, no DC, no clipping. Not the problem.
- **Grains are phase-aligned engine cycles.** Grain length = `120 · sr / rpm` within 0.2 % of the RPM table
  (median ratio 0.9985) on every recording, and the cross-correlation of consecutive grains (time-normalised)
  peaks at lag 0 for 95 % of pairs (p90 0.005 cycle; the off-load sweep 78 %). So concatenating grains at their
  boundaries is phase-continuous and jumps between nearby grains need no phase alignment. Consecutive cycles
  correlate only 0.74 (median): the engine itself varies cycle to cycle.
- **The single-layer synth is faithful.** New `mwtool ginsu-resynth <bundle> <index> out.wav [--hold rpm]`
  (`GinsuSynth::render`) drives one recording with its own RPM curve. Against the original: level within
  ±0.8 dB (p10/p90), per-50 ms spectral distance over 1/6-octave bands median 3.5 dB, p90 5.3, which equals
  the recording's own cycle-to-cycle floor (next cycle: median 3.6 dB, p90 5.0). Steady holds at 2,500 and
  4,000 rpm match the raw grains at that RPM within ~2 dB per order (order 3 dominant, order 6 +4 dB high at
  4,000). Only blemish: pitch spread p90 71 cents vs 45 for the original (the 1-in-8 random jumps).
- **The RPM axis is real engine RPM** (changed: `EngineDriver::audible_rpm` now passes the physics RPM
  through; the sequences clamp it to 1790..7500). Evidence: the idle loop `01000000001d1c7a` has its
  autocorrelation peak at 40.0 Hz and spectral peaks at 119 / 159 Hz, exactly the first grains of the Ginsu
  sweep (~1580 rpm: lag len/3 = 40.5 Hz, orders 4.5 / 6 = 119 / 159 Hz), and the sweep ends at 7814 rpm = the
  handling's `Max_RPM` 7800. The old remap (idle 1000 → 1790, redline → 7500) compressed the pitch range to
  0.84× and made 4,000 rpm sound like 4,300. Below 1790 rpm the car now plays the idle loop only, as the data
  says. Not verified by ear.
- **Mix findings, unchanged** (steady 4,000 rpm hold, `hold.py`): the dry six-layer bank is close to the sum of
  both microphone sets' recordings (harmonic / total energy 0.99, order 3 dominant). The wet chain adds +5 dB
  at 60 Hz and +11 dB at 20–60 Hz over the recordings (full load) and +11/+13/+10 dB at 60/120/250 Hz off load,
  and drops the harmonic ratio to 0.87 (added inharmonic content). Channel meter on the hold: bus B `0b381c`
  input +5.3 dB RMS (Peaking 100 Hz ×4 Q2 and ×5 Q0.5 are +12/+14 dB), the set-A strain path `073483` fed at
  −16.5 dB RMS into `tanh(4·x)`, i.e. a near-square wave about as loud as the clean path. These stand-ins are
  the prime suspects for "wrong"; the data cannot settle the distortion law or the sub-harmoniser level.
- **Data details**: plug-in field `2a740b25` is the output channel count (1 until the `Pan2D`, then 6). The
  911's `DspParam` overrides: Peaking 472213 / 472218 param 1 = 400 / 1000 (reads as Frequency: the global
  6.5 kHz EQs become 400 Hz / 1 kHz for this car), Delay 472308 param 4 = 0.2 (Feedback) and its Send 472306
  param 0 = 0 (the delay path is muted), HighPass 472227 param 1 = 6810 (does not read as a frequency before a
  sub-harmoniser), BandPass 750019 param 5 = 40 (bandwidth?). Field order in the types does not give these
  indices; still not applied. `GinsuDspPlugIn` carries no playback parameters (`a4831678` 1 / 0 only).
- **A/B wavs** (scratch dir, regenerate): `911/01000000001d1c7c_ginsu.wav` raw on-load sweep vs
  `resynth_on.wav` (same sweep through the synth), `resynth_hold4000.wav` vs the raw at 4,000 rpm,
  `hold4000_on.wav` / `hold4000_on_dry.wav` (bank wet / dry at 4,000 rpm full load, old RPM mapping),
  `synth_wet.wav` / `synth_dry.wav` / `synth_legacy.wav` (default rev-up and coast, old mapping),
  `game.trace` (in-game RPM from `--test engine`).
- **Next**: (1) listen to raw vs `resynth_hold4000` vs `hold4000_on_dry` vs `hold4000_on` to place the fault
  (synth, mix or effects); (2) if the wet chain is at fault, scale the unknown effects so they never dominate
  the recorded signal (distortion drive relative to the input level, sub-harmoniser at the band's level, EQ
  gains as given); (3) the 1-in-8 random jump: test following the recording strictly with jumps only when the
  walk leaves the window; (4) decode the `DspParam` index ↔ runtime parameter mapping; (5) check the audio
  callback cost (rodio underruns after the SFX merge).

### 2026-10-04 (session 2): wheel layout, the Venom's front axle
- User report: wheels off on some cars (Hennessey Venom GT Spyder). Survey over every car
  (`cargo test --release -p mw-assets --test wheel_survey -- --ignored --nocapture`): body z extent vs spec
  axle positions. Every car has front / rear overhangs of 0.46–1.36 m except the Venom: front axle at z = 0.125
  (2.20 m overhang, 1.63 m wheelbase on a 4.66 m body). The Veyron and one Caterham entry have swapped
  Front / Rear labels (already handled: the loader classifies axles by position).
- The 0.125 is in every source: VehicleGraphicsSpec wheels, the physics `RigidBody` transforms (`0d0c29b4`),
  `SuspensionWheel`, and the LO bundle. No other vector in the car's GeneSys data sits at a front-axle
  position. The Venom is Ultimate Speed Pack DLC. How the original shows it correctly is unknown (dead end:
  no override found in the bundles).
- Arches from the mesh: along z, the lowest point of the outer 25 % of the body's half-width jumps from the
  sill (~0.1–0.2 m) to ~0.55–0.73 m over each wheel. 911: arches centred at +1.25 / −1.25 (data +1.24 /
  −1.21); Venom: front arch +1.05…+1.55 (centre +1.30), rear around −1.5 (data −1.505).
- Fix (`mw_assets::wheel_arches`): an axle whose overhang exceeds 1.6 m moves to the arch nearest to a
  0.95 m overhang (within 0.6 m); applied to the spec wheels (viewer and drive) and the physics wheels. Only
  the Venom triggers it. (Superseded 2026-10-10: the game takes wheel positions from `VehicleInfo.Axles`, whose
  front axle on the Venom is at z 1.3; the heuristic is removed, see "wheel placement".)
- Unrelated: the dark rectangle near the horizon in `--vehicle` shots is in the sky / cloud panorama (it
  shows behind the 911 too).
### 2026-10-04 (particles): BearEffect decoded, tyre smoke, kick-up and sparks
- **Where**: all 214 `BearEffect` (0x301) resources are in `GLOBALEFFECTS.BNDL` (26 are empty 0x30-byte
  placeholders), with 102 materials, 129 textures, 12 models (debris meshes) and one `BearGlobalParameters`.
  No other bundle has any; the vehicle and world bundles reference effects only through GeneSys handles.
- **Format** (layout tables in `mw-formats/src/bear.rs`, burnout.wiki "Bear Effect/NFS Most Wanted" for the
  layer header): effect header (version `0x269987a1`, layer count, action-group mask 3), a parameter table at
  0x20 (`u8 n, 0, u16 size, u32, u8 kind[n]`, then names; the wiki's table offset field points at a copy) with
  a handle array `{u16 offset | 1, u16 global id}`; per layer a 0x20 header, a name (`<number>_Test`), a
  channel table (`u8 n, u8 external, u16 bucket bytes, u8 kind[n]`, names), an action table (`u16 offset[n]`
  then `u8 kind[n]`) and four action-group offsets. A group is `u8 sequences, primaries, secondaries, pad`,
  `{u8 primaries, u8 per-bucket}` per sequence, `{u8 action, u8 secondaries}` per primary, `u8 action` per
  secondary: verified on all 1,113 groups (counts add up, indices in range). Group 0 = update (spawners are
  primaries whose secondaries initialise new particles), group 1 = render; 2 and 3 never occur.
- **Operands**: action payloads are 4-byte words; channel references are `u16 lo = bucket offset | scratch
  bit, u16 hi = 4 × channel index` (the scratch copy holds `Radius*`, `Colour*`, unpacked `Velocity*`),
  parameter references `hi = 0x40 + 2 × index`, engine references `hi = 0xEB40` (layer state; `.1` written by
  the controllers), `0xEB42` (view, `.a0` in every renderer), `0xEB44` once. Material / model imports sit
  inside the renderers (0x24 +72, 0x25 / 0x29 +32, 0x27 +80, 0x28 +112, 0x1F +56, 0x22 / 0x23 +24).
- **Action kinds** (52 seen; by which channels they touch, which group / role they have and their value
  ranges): spawners 0x00 (transform: box extent, LOD distance range, angle ranges, speed range, rate /s at
  +0x60, per metre +0x64, burst count u32 +0x6C; rate × lifetime ≈ max particles across layers), 0x01
  (collision mesh: speed, two angle ranges, LOD, rate u32 +0x54 (2000 while scraping), burst u32 +0x58
  (128)), 0x05 (wheel: LOD, speed, three angle ranges, lateral −1..1, height, per metre? +0x70, rate +0x74),
  0x07 (wheel contact), 0x06 (contact list), 0x2B (super-emitter), 0x4C (splatter); initialisers 0x0C
  lifetime (age = 0, 1 / rand(min, max)), 0x12 radius, 0x0A sprite rotation (turns) + spin, 0x0B 3-D
  rotation, 0x0D alpha, 0x13 bounce plane, 0x14 / 0x15 place in the emitter frame (quaternion + offset), 0x16
  inherit a velocity parameter × rand(min, max), 0x47 seed, 0x0F / 0x11 light / collision radius; updates 0x2F
  age (normalised), 0x3D integrate (gravity −9.8 for debris, +0.4 for smoke, drag), 0x3E integrate + bounce
  (restitution and friction ranges), 0x2C / 0x2D rotation, 0x40 rigid debris, 0x44 bounds, 0x49 / 0x4A / 0x4B
  velocity pack / unpack (`PackedVelocity`); render group 0x3C radius curve (four Bézier control points over
  age), 0x1C colour (rows 0–4 = polynomial coefficients in age for r, g, b and a brightness/alpha column;
  a 5-key u8 alpha curve at +144 with a scale at +156; sparks: `1 − t²`, `0.41 + 0.14t − 0.54t²`, … orange
  to black, column 3 = 12 − 4t − 7t²; smoke: blue-grey, alpha curve peaking at 0.31), 0x17 alpha × colour,
  0x48 seeded brightness, 0x38 camera fades (near fade 1–10 m), 0x1D atlas frame from age, 0x45 view;
  renderers 0x24 quads, 0x27 quads with atlas frames, 0x28 seeded quads, 0x25 / 0x29 motion-blurred sprites,
  0x1E / 0x1F / 0x22 / 0x23 meshes, 0x4D splatter, 0x30 lights. 5-key curves: `u8 values[5], u8 times[5]`
  (`ff ff ff ff ff | 00 fc fd fe ff` = 1).
- **Which effect where**: `HAWAII/SURFACELIST.BNDL` → `Surfaces.Surfaces` (65 `SurfaceType`, 0 = default
  tarmac) → two weather conditions (dry / wet) → `Surface_vfx` → `SurfaceVfx`: `WheelVfx` (kick-up, 0x07
  spawner on `WheelContact`), `PlayerWheelEffects` / `AIWheelEffects` → `VfxWheelEffects` (`DefaultTyre` =
  tyre smoke `…49247`, 7 layers, `WheelParams` input; `OnRims` = rim sparks), ten directional impact effects,
  tyre-mark materials, `RimMarksMaterial`. Tarmac: impacts front / rear / bottom = `…a394b` (two spark bursts
  of 128), left / right = `…7bd97` (continuous sparks at 2000/s: scraping). `VfxGlobals` →
  `VehicleImpacts` / `VehicleScrapes` (`VfxVehicleCollisions`), `Misc` → `BurstTyreWheelEffects`,
  `SpikeStripSparks`. Names cracked with direction / surface / effect vocabularies (added to
  `genesys_names.txt`). Dead ends: no hits for the 36 `VfxVehicleCollisions` fields with 4-word direction ×
  severity × impact vocabularies; `PuddlePuncturedBurnoutHit`, `RoofWheelsHeavyDebris`,
  `ContinuousCarCrashTrails`, `TrailsMeshVfxGlancing` were false positives on float fields.
- **Shaders**: `VfxParticles_Diffuse*` take CPU-expanded quads (world positions, COLOR, TEXCOORD.xy corner /
  .z age, TEXCOORD1.zw translucency / key-as-ambient, TEXCOORD3 lighting normal + wrap); the VS lights per
  vertex (irradiance quadric × 1.3 + wrap-lit key light), picks two atlas frames from `NumTiles` /
  `EnableLooping` (row index from frame / tiles.y), the PS blends them, alpha-tests at 1/255 and writes
  premultiplied colour with alpha × `gVfxConstants.z` (one blend state for normal and additive);
  `_SoftParticles` variants fade by scene depth. `VfxParticles_MotionBlurSpriteUntex` integrates a moving
  disc's (1 − r²) profile over the time each pixel is covered (analytic coverage from the two roots), colour
  × `Overbright` (6) × `gVfxSparkBrightness`, additive.
- **Implemented** (`crates/mostrusted/src/particles.rs`, `particles.wgsl`, `sparks.wgsl`): a CPU runtime that
  runs each layer's groups in stored order with the decoded actions, batches per material, and draws with the
  two ported shaders (forward, premultiplied / additive). Tyre smoke per wheel from the contact patch's
  sliding speed (`drive::WheelContact`, minimal read-only addition to `drive.rs`), kick-up per rolling wheel,
  impact bursts and scrape sparks from avian contacts of the car body (`ImpactEvent`). `--screenshot-time`,
  `--screenshot-after-impact`, `MW_VFX_DEMO`, `MW_VFX_DEBUG` for checks.
- **Stand-ins / guesses** (each marked in the code): emission = spawner rate × controller curve 0 over
  `Intensity` (sliding speed / 15 m/s) × camera-distance LOD + per metre of sliding; quadratic drag
  (`v /= 1 + drag·|v|·dt`: smoke stops within metres; a linear reading carried it 25 m with the car); spawn
  directions as azimuth / elevation from the first two angle ranges; collision sparks spread over ±0.3 m
  (the game spreads them over the collision mesh); `SPARK_BRIGHTNESS` 0.15 for `gVfxSparkBrightness`
  (without it sparks tonemap to flat white); streaks over 1/30 s; the irradiance cube for the quadric;
  always the dry tarmac surface.
- **Pitfalls**: avian reports one contact pair per track-unit trimesh that stays touching (speculative), so
  `collision_started` never fires on impact; impacts are detected by closing speed (> 3 m/s, 0.25 s cooldown).
  The spark quads are built in y-down pixel space and were back-face culled (culling is off for particles).
  Empty meshes upset Bevy's mesh allocator ("Use-after-free … unallocated key"): batches keep one invisible
  quad.
- **Verification**: every effect parses (`mwtool verify`, corpus test `bear_effects_decode`: 214 effects, 572
  layers, 415 quad / spark renderers); `mw-assets` resolves every table entry and material. Screenshots:
  handbrake and drift tests on the test track (smoke clouds behind the rear tyres), a crash into the guard rail
  west of downtown (`--auto-throttle 1 --screenshot-after-impact 0.04`: spark spray and dust),
  `MW_VFX_DEMO=010000000007bd97,0.25,15` at street level (orange specks and a white-hot streak).

### 2026-10-04 (audio session): the engine plays the car's whole sound bank
- **Bindings are strings.** `CgsCore.StringBase` = `{i32 offset from the field, u32 length incl. NUL}`; the
  "raw" binding values in earlier dumps were these. Sequences bind named game values:
  `Players.LocalPlayer.GetPlayerAudibleRPM`, `GetPlayerEngineLoad`, `RaceCars.EngineSound[#].EngineOn`,
  `GetPopProbability`, `RevLimitGain`, `TrackedHarmonic1Frequency`, `SubharmoniserFrequency`,
  `GetPlayerNormalisedGearChangeTimeElapsed`, `LatSlipIntensityForSkidSound`, `GetSpeedMph`, ... with optional
  tests (`(NOT)`, `(GTEQ:100)`, `(BETWEEN,20,60)`).
- **Field names cracked** (python CRC brute force over sound vocabulary, each checked against the values; added
  to `genesys_names.txt`): `StartTime` / `EndTime` (item range on the sequence axis), `Fade_Out`,
  `Binding_Max`, `Binding_Exponent`, `Binding_Invert`, `Binding_Evaluate_Once`, `Binding_Range_Min/Max`,
  `Output_Value_Min/Max`, `Binding_Modulation_Type`, `Animation_Modulation_Type`, `AnimPointCount`,
  `TangentIn/OutX/Y`, `WavesCount`, `Trigger_Probability(Count)`, `Double_ParamsCount`, and in EngineSound2
  `Ignition_Sequence`, `Pop_Sequence`, `Backfire_Sequence`, `Gurgle_Sequence`. Dead ends: TyreSoundParams
  fields (5,000-word 3-word search and `genesys-crack` at 4 words found nothing trustworthy); 3-word hits with
  suffixes are mostly CRC collisions (`Down_Lift_Soft_Chance`).
- **Two array-count bugs in the decoder**: a WaveSequenceItem's `Waves` showed 0 items (their count is the
  second trailing u8, `WavesCount`, confirmed by array spacing), and EngineSound2's twelve handle lists were
  read with overlapping lengths (Burnout 18, Drift 18, Sequences 3). Handle arrays sit back to back, every slot
  with a handle import, so the decoder now takes such a run up to the next array as the length: Burnout 1,
  Drift 2, In_air 4, **Sequences 6**, channels 14. VehicleSound's lists change the same way (Player_Sequences
  12). The engine had been playing 3 of its 6 always-on sequences.
- **Sequence semantics (from the data; formats.md)**: items fade in/out *inside* their range (wind layers
  45–90 / 70–138 / 100–250 mph with 20–40 mph fades crossfade exactly); a modulating value maps its binding
  (or, without one, the item's normalised position) through its curve onto `Output_Value_Min..Max`, else is
  `Value`. Load curves therefore scale up to 2× and 4× (the earlier code read the curve y as the gain).
  Assumed: the two modulation-type enums are ignored (frequency automations with `Value` 24000 and either type
  only make sense as "output replaces"); curves are piecewise linear.
- **The engine's mixer graph is data.** Each `MixerChannel` has a gain, a mixing group and a plug-in chain;
  `SendDspPlugIn.Voice_Routing` sends to another channel, a `SubmixDspPlugIn` collects channels of the groups it
  lists (the idle loop reaches the set-A bus this way). 911: set A → submix → {EQ (100 Hz ×5, 6.5 kHz ×2), EQ +
  distortion 4} → vehicle bus; set B → submix → {EQ 2/6 kHz, band-pass + sub-harmoniser + low-pass, delay} →
  {EQ, EQ + distortion 1–4 by load} → second bus + sub-harmoniser bus. `MixerChannelSequenceItem` parameters:
  `PlugIn` 0 + `Parameter_Name` 0 + `Automated_Property` 0 = channel gain; a plug-in unique id + a field hash
  (`Frequency` = 0x06096291) + 9 = that plug-in's field; `PlugIn` 0 + field hash + 9 = the channel's first
  plug-in with that field (the load curves drive the strain channels' 1 kHz peak gain and distortion amount).
  The two vehicle buses send to each other; the player drops sends that close a loop.
- **New runtime** (`mw_audio::sequence` + `dsp`, built by `mw_assets::sound`, driven by
  `mw_assets::engine_driver`): every sequence of the bank, Ginsu items in one shared period schedule, wave items
  (loops vs one-shots by the SPS loop flag), the graph with RBJ biquads, `tanh` distortion, a flip-flop octave
  divider, delay. Engine bank = `Sequences` (RPM ×2 with idle loops, load crossfades, speed, normalised RPM bus
  automation), Gurgle (pops, retriggered at a rate ∝ pop probability), Backfire / Ignition / rev-limiter cues,
  one upshift and one downshift gear-change group (by content: the upshift group has the LFO shunt and power
  cut). Our rules (assumed): audible RPM = physics idle..redline mapped onto the RPM sequence's axis
  (1790..7500 on the 911, so idle plays the recorded idle loop and the Ginsu fades in over 100 rpm); load =
  throttle, cut for one normalised shift time (0.25 s); pops armed by lifting off above 45 % revs, fading over
  0.6 s; backfires on 35 % of lifts above 75 % revs and half of full-throttle upshifts above 60 %; tracked
  harmonic = audible rpm × the recording's strongest half-order (Goertzel over grains: 911 3, Mustang 4, GT-R
  3, Veyron 4); per-car loudness normalised (raw bank levels spanned 24 dB, Mito −31.5 to GT-R −7.6 dBFS on one
  sweep, because the mixes end in buses outside the vehicle bundle; now −16.7 to −11.2).
- **Measurements** (911, `mwtool engine-sound`; scripts in the session scratchpad): 4 s rev + 4 s coast:
  pitch vs the order-3 firing frequency median −12 cents, p90 72 (old six-layer mixer: 98); peak |diff|/RMS per
  2 ms ≤ 1.0 (old 1.2); limiter 0.002 % of samples; max 10 ms loudness step 6.3 dB. Off load is now ~8 dB
  below full load (old mixer: same level), idle ~15 dB below full throttle (the data's idle loop gain 0.3). The
  plug-ins add +4.9 dB at 63 Hz and +2.7 dB at 500 Hz and −1.3 dB at 250 Hz against `--dry`. Recorded game
  trace (`--test engine`): steady segments 6–47 cents p90; worst windows are the idle loop (fixed pitch by
  design) and shifts. Corpus test `mw-assets/tests/engine_banks.rs`: all 73 engine banks render finite,
  audible and without hitting the limiter. Discontinuity peaks left are real transients (ignition starter,
  backfires).
- Open: the state lists (Burnout, Drift, In_air, Reversing) and which gear-change list is which; per-car
  DspParam overrides (parameter indices unknown); `BusMixerChannelSequenceItem`, `LfoSequenceItem`,
  `RaceCarEffectSequenceItem` (camera shake?) not played; the true plug-in formulas; the game's audible-RPM and
  engine-load definitions.
### 2026-10-04 (camera session): VehicleCameraContainer decoded, camera.rs
- **Camera types** (`mwtool genesys-types | grep -i camera`): the container holds `CameraGameplayExternal`
  (392 bytes: six sub-blocks, four vec3s, FOVs, a handle to `CameraGameplayShakeEffect` and pointers to
  `CameraGameplayExternalGlobals` (`Impact_shake`, `LensProperties`, a shake effect) and
  `CameraGameplayGradientAdjustments`), `CameraGameplayBonnet` / `Bumper` (128 bytes, same layout) and
  `CameraRearView` (+ `CameraRearViewGlobals`). All 79 containers in the install read
  (`crates/mw-assets/tests/cameras.rs`); 30 of ~150 fields vary between cars (eye, look-at, speed movement,
  roll and pitch amounts, shake speed, in-car eyes), the rest are global constants.
- **Field order is alphabetical** within a size group (case-insensitive): every named camera field is in
  order (`Maximum_Roll_Angle` < `Minimum_Angular_Velocity_for_Roll` < `Minimum_Lateral_…` <
  `Roll_Angle_Change_Intensity`; `Amplitude` < `Damping` < `Maximum_Angle` < `Minimum_Angle` <
  `Spring_Coefficient`). That bounds an unnamed field's name and was used to accept or reject hits.
- **Names cracked** (python templates with mixed capitalisation, plus `mwtool genesys-crack`, which gained a
  `title-small` style: Title_Snake with `for`/`to`/`at`… in lower case, the style of
  `Minimum_Lateral_Acceleration_for_Roll`): pitch `Maximum_` / `Minimum_deceleration_for_pitch_change` (12 / 2),
  `Pitch_change_curve_power`, `Pitch_change_intensity`; roll `Minimum_Lateral_Acceleration_for_Roll` (10),
  `Minimum_Angular_Velocity_for_Roll` (0.2), `Roll_Angle_Change_Intensity`; shake `Minimum_` /
  `Maximum_Speed_for_Shake` (0 / 180 mph, the 911's top speed is 189); shake springs `Spring_Coefficient`,
  `Minimum_` / `Maximum_Angle`, `Lower_` / `Upper_Translation_Limit`; `Maximum_amplitude`,
  `Maximum_frequency_multiplier`, `YawDriftSpring`, `Maximum_Gradient`; mirror `Horizontal_FOV`,
  `Aspect_ratio`, `Near_` / `Far_clip_plane`, `X_axis_rotation`. Plausible without a confirming sibling:
  `Camera_Roll_Factor_Power`, `ShakeMagnitude`, `WobbleMagnitude`, `AccelerationResponse`, `FocalDistance`.
  Also added the names handling.rs already hashed but genesys_names.txt lacked (`MinFieldOfView`, …), so the
  dumps now show them.
- **Units**: the thresholds only make sense in mph/s: `Minimum_Lateral_Acceleration_for_Roll` = 10 m/s² would
  mean no roll below 1 g; as mph/s the roll range is 0.46–1.37 g and the pitch range 0.09–0.55 g.
- **Dead ends**: (1) the two roll maxima (`02dd11bc` 2.5–3, `3f8ced67` 30–40) track the minima's meaning and
  sort right, but `Maximum_Angular_Velocity_for_Roll`, `Maximum_Lateral_Acceleration_for_Roll`, all one- and
  two-character edits of both (13.9 M candidates), `…_for_Maximum_Roll`, `Camera_Roll_Maximum_…` and Max/Full/Peak
  variants miss. (2) No `Acceleration_movement`, `Speed_based_movement`, `Yaw_spring_modification`,
  `Speed_based_height_change` or bonnet field cracked with 4–5-word vocabularies (snake, Title, title-small,
  Camel); only nonsense hits (`Vertical_Speed_Impact_Mph`, `Low_at_Min_Z_Lateral`, …). (3) No look-around /
  free-look fields (Look, Free, Orbit, Return, Recentre, Yaw, Pitch, Limit, Speed templates). (4)
  `CAMERAS.BNDL` holds 45 `CameraTake`s named after event outros, intros, busted and "FinalCam" shots
  (`Sprint_Race_Event_Outro_ICE`, `Busted`, `EMP_Zoom_Hit`): cinematic cameras, not gameplay. (5) A web search
  found no community notes on these types.
- **Bonnet / bumper**: the bumper eye (0, 0.425, 1.5) is inside the 911's nose (its bumper is at z ≈ 2.2), so the
  car must not be drawn in that view; their FOVs (95→105, 95→120) look very wide when read as vertical.
- Implemented in `crates/mostrusted/src/camera.rs` (moved out of drive.rs): yaw spring per 60 Hz tick from
  the speed band (and `YawDriftSpring` with a lean towards the velocity while drifting), speed-based eye /
  look-at movement, an acceleration spring (eye trails by a / ω², clamped to −2 / +1.5 m), roll and pitch from
  the named thresholds and curve powers, high-speed shake, impact springs kicked by body-collider contacts,
  a ray against the world so the eye stays in front of walls, look-back, look-around (right stick, right
  mouse drag) that springs back, the bonnet and bumper modes (C / gamepad Y), `--camera-mode`,
  `MW_CAMERA_TRACE`. Table and results in handling.md ("Cameras").
- Open: the in-car cameras' 19 floats (they are rigid for now), the remaining acceleration and yaw-spring
  fields, `Speed_based_height_change`, `Gradient_Adjustments`, whether the FOVs are vertical (compare
  with the original; `mwtool memscan` could find the camera's position and FOV in the running game).
### 2026-10-04 (weather): status / next steps
- Works: runtime environment (`weather.rs`: painted timeline under the camera, fractional hours, F5/F6/F7,
  `--weather`, `--timeline`), road wet path (`--road-wetness`), opt-in planar reflection
  (`--planar-reflection`), `mwtool paint-map`. fmt / clippy (0 warnings) / tests / `mwtool verify` (0
  failures) pass.
- Integration note: the planar reflection adds a second `Camera3d`; every main-camera query must use
  `crate::MainCamera` (done in main, drive, garage, damage, weather). New queries (e.g. the camera agent's
  `camera.rs`) need it too before the reflection can be on by default.
- Pre-existing, not from this work: the daytime sky renders black in the main view (the mirrored view,
  which skips the game tonemap, shows it blue), so the game tonemap / grade likely crushes it.
- Next: make the planar reflection default once camera queries are merged; give it a mip chain; tune
  `WetScaleParams`; port `PlotPBR_AO_StandingWater` and `WorldPBR_Horizontal_PlanarReflection`; tree sway
  (needs a `Layered` vertex shader); blend when the painted timeline changes; LayerFog from its paint map.

### 2026-10-04 (weather): what the data has for weather, timeline paint map, wet roads
- **No rain.** No precipitation exists in the PC data: no GeneSys type or field with rain / precipitation /
  wetness vocabulary (12,410 type rows searched), no rain / drop / splash / lightning shader (`mwtool shaders`),
  no such texture in `ENVIRONMENT.BNDL`, `GLOBALEFFECTS.BNDL`, `GLOBALTEXTUREDICTIONARY.BNDL`,
  `HAWAII/GLOBALRESOURCES.BNDL`, no rain string in `GLOBALEFFECTS` effects, and the `Weather` block holds only
  floats (no effect or texture handle). A rain effect would be an invention, so none is drawn.
- **`Weather` block** (cracked with weather vocabularies + `/usr/share/dict` words, accepted only where the
  values fit): `WindBearing` 130, `WindBearingVariability` 25, `WindSpeedMin` 1, `WindSpeedMax` 3.5 (all
  175 keyframes), `Thunderstorm_Intensity` (0.23 on the 42 night keyframes, 00:00–04:00, else 0), `0141d842`
  (0.25 on daytime keyframes, 0.15 in the evening, at night and on every Alternate), `e8eb5d48` (0).
  Also cracked: `SparkExposure` (Vfx: −3 night, 0 morning / evening, 0.75 noon), `DistanceBlend` (HeatHaze,
  100), and five array counts that equal their arrays' lengths (`ColourSunCount` 5, `SkyColorGradientCount`
  30, `ScatteringColourCount` 3, `TintColourCount` 4, `LensFlareColourCount` 5). Rejected as flukes
  (two-word hits that don't read as names or don't fit): `NearPlane`, `CapSun`, `AnimateSpeed`,
  `ExposureDefault`, `IntensityStandard`, `BrightnessAdjustment`, `BrightnessOverride`.
  `655d3f5f` (`CgsCore.UniqueId`) is the object's own ID low bits at top level and 0 in sub-structs.
- **Alternate keyframes**: 154 of 175 keyframes name one of only 7 Alternates (the earlier "seven keyframes
  have Alternates" was the other way round): night 00–04 (`…124683`), dawn 05–06 (`…1d3c66`, `…12466e`),
  morning 07–10 (`…1d3c62`), noon 12–15 (`…1d3c65`), evening 17–23 (`…1d3c63`), and `…f083a` for the
  front-end timeline `…9efca`. Hour 04 keyframes have none. No timeline lists an Alternate. They are hazier:
  fog far 550–1200 m (vs 1500+), key light ×0.6–0.8, brighter shadow fill, their own colour cube, cloud
  `LightColour` 1.72 → 1.0. Nothing in the data selects them; `--weather` / F7 switch to them.
- **Which timeline lights a place: `WorldPaintMap` (0x91).** `EnvironmentSettings` fields `409f546f` /
  `ae067627` hold the low bits of `…2823d` / `…285cb` in `ENVIRONMENT.BNDL`: quadtree maps over 1 km cells
  painting the 25 timelines and the 19 `LayerFog` objects (formats.md). `VfxGlobals` `5e608031` names a third
  (`GLOBALEFFECTS.BNDL` `…285cc`, one `ParticleLighting`). Cell = floor(x / 1000) + 3, floor(z / 1000) + 5 for
  the timeline map: checked by drawing the 169 zone outlines over it (`mwtool paint-map … --png`): region
  borders follow zone edges and the river, and every zone is painted. The drive spawn is under `…1245c8`,
  downtown under `…1245c6`, the north-west under the default `…69cbb`. Timelines repeat a keyframe over
  consecutive hours (00–03, 05–06, ...), which only makes sense with interpolation, so hours now blend
  linearly between entries (provisional).
- **Wet roads are always on.** `DriveableSurface` main pass decoded: t5 = RoadDepth (material), t6 =
  `puddleMapSampler` (engine-bound), t7 = planar reflection, t8 = env-BRDF LUT; `WetScaleParams` (global CB)
  x = puddle tiling of world xz, y = reflection distortion by (N − N_vertex).xz, z = reflection mip per
  roughness. wet = puddle.w × saturate(TEXCOORD5.x); k = lerp(1, (RoadDepth.y × puddle.x, puddle.y, puddle.z),
  wet); N = N_v + k.x (N − N_v); roughness × k.y; albedo × k.z; F0 = lerp(0.02, F0, k.x); indirect = albedo ×
  baked vertex light + planar(mirrored projection + distortion, mip) × envBRDF(N·V, roughness, F0) × wet. There
  is no global wetness input, so roads are as wet as the puddle map says, day and night. TEXCOORD5.x is a
  per-vertex mask: 0 on 22 % of TRK_UNIT1's road vertices, ≥ 1 on 78 % (max 3.9, saturated).
- **The puddle map** is `GLOBALTEXTUREDICTIONARY.BNDL` `…7811` (256², no material binds it): x ≈ 0.9 around /
  low in puddles (normal kept), y ≈ 0.07 / 0 (roughness scale), z ≈ 0.8 / 0.45 (darkening), w ≈ 0.55 / 1
  (wet amount): identified by these channels matching the shader's use (it is the only texture with them).
- `PlotPBR_AO_StandingWater` (28 meshes) runs the same wet maths with its own `PuddleTextureSampler` at
  TEXCOORD1 and no RoadDepth (not ported).
- **`SurfaceType`** has `Dry_Conditions` / `Wet_Conditions` blocks (cracked): 57 of 65 surface types give the
  wet one other tyre-mark materials, `Surface_vfx`, sounds or `VehicleEffects`. What makes a surface wet is
  unknown.
- Implemented: `weather.rs` (environment rebuilt at runtime on a background thread: F5 / F6 hour, F7
  Alternate, timeline from the paint map under the camera, `--hour` fractions, `--timeline`), the road wet
  path in `layered.wgsl` (puddle map, RoadDepth, vertex wetness mask in COLOR, vertex AO), and an optional
  planar reflection (`reflection.rs`, `--planar-reflection`: a mirrored camera through the same deferred
  pipeline, clipped below the plane; the sky cube stands in without it). Not done: rain (not in the data),
  tree sway (decoded, needs a vertex shader for the Layered material), cloud scroll, thunderstorms.
- Dead end: the first keyframe survey script overwrote entries and reported 7 keyframes *with* Alternates;
  `mwtool genesys` truncates arrays at 16 items by default, which hid timeline hours 19–23 (use
  `--max-items 40`).
### 2026-10-04 (props session): prop physics, collision surface tags — status / next steps
- **Prop physics data**: `PropObject` matches burnout.wiki's `Game::World::PropObject`: 0x0C
  `mPhysicalDefinitionHandle`, 0x14 impact effect, 0x1C impact sound, 0x24 `mfResetDistanceSquared`
  (10000), 0x28 `mbRemoveInsteadOfReset`. `WorldObject` / `CompoundObject` have the physical definition at
  0x1C; compound parts (ptr 0x30, count 0x38, 0x70 bytes each) are embedded WorldObjects with their own
  physical definitions (+0x1C) and part links at +0x64. All in `ObjectDef` (world.rs).
- GeneSys `PhysicalDefinition`: bounds (`62dbd683` centre, `98e902ed` half extents), `AdditionalInfo`,
  `RigidBodies` (transform `0d0c29b4`, `ComOffset`, `Inertia_Scale`, `Mass`, `Friction`, `44ac37c1` probably
  restitution, `LinearDrag`, `AngularDrag`, `BoxVolumes` (transform `d084ac7c`, half extents `3727fead`),
  `CylinderVolumes` (`Radius`, `HalfLength`, axis = local Z), capsule / sphere / convex-hull lists).
  New names (genesys-crack, single candidates): `HalfLength`, `BoundingConvexHull`, `ComOffset`,
  `ForceToDamage`, `ForceToSmash`. World objects use only boxes (422) and cylinders (12). `mwtool props`.
- `DynamicAdditionalInfo` / `CompoundAdditionalInfo`: `ForceToDamage`, `67f0233c` (unnamed; 0 cones/bins,
  500–2500 signs/fences, 5000 street lamps, 20000 gantries, 100000 on mass-0 objects), `Strength`, `Score`,
  compound `ForceToSmash` (10000 on street lamps). Read `67f0233c` as the break-free threshold, units
  assumed impulse N s (m_hitter × approach speed: lamps break above ~12 km/h for a 1.5 t car). Hypothesis.
  Dead end: guessing `ForceToMove/Break/Detach/Topple/...` for `67f0233c` (no hit with ~150-word lists).
- 112 prop, 52 compound, 271 dynamic definitions (55 dynamics with physics; 3120 dynamic placements have
  none). Mass 0 objects have default-looking values (friction 0.1, drag 0.1) → treated as immovable.
- **Collision tags** (burnout.wiki `muCollisionTag`, but the PC bit order is reversed vs the wiki's PS3
  table): as LE u32, flags bits 0–9 (DRIVABLE 1, ROAD 2, KERB 4, ..., FATAL 0x40), surface index bits 13–19,
  track unit bits 20–31 (all 1,652,030 triangles carry their own unit number). Surface index → the
  65-entry `Surfaces` list in `HAWAII/SURFACELIST.BNDL` (`SurfaceType`: debug `Colour`, per-contact
  `Surface_Physics` with `Surface_Grip` 1.0 asphalt / 0.75 dirt / 0.7 grass, sounds, vfx). No surface
  names in the data. First read the wiki's order literally: surface indices up to 90 (dead end).
  `mwtool surfaces`, `mw_assets::surfaces`, test `crates/mw-assets/tests/props.rs`.
- **Runtime** (`crates/mostrusted/src/props.rs`): props/compounds/dynamics with physics get a compound
  collider (loader thread), pinned static until a contact hook sees impulse ≥ break force, then dynamic with
  the inelastic momentum exchange; reset at 100 m. Layers: World / Car / CarBumper (low box that only meets
  props) / Prop; wheel rays see only the world. Damage scales loose-prop contacts by m_prop/(m_car+m_prop).
  Colliders are streamed: only props within 120 m of the car carry a body (removed past 160 m).
  `CarSurface` component = surface under the car (via `WorldSurfaces::surface_at`).
- Verified: `--test smash` at 60 km/h knocks cone, street lamp and fence, car stops at the immovable wall
  (z 167.8 vs wall 170); at 8 km/h the lamp holds and the car stops against it. World drive from the
  default spawn knocks a street lamp and the roadside crash barrels / tyres; screenshots looked at.
- Measurements (noisy, shared machine, load average 5–14): full map 17,948 placements with physics; load
  24.6 s vs 18.5 s with `--no-prop-physics` (the decode is per definition, so most of the gap is probably
  noise, re-measure); frame time with all 18k static bodies ~27–29 ms vs 13–27 ms without — streaming (50
  active) did not clearly change it under that load. Needs a clean A/B (`--frame-stats`), ideally timing the
  physics schedule alone.
- Next: clean perf A/B; compound parts as separate breakable bodies (need part→mesh mapping, maybe the
  skeleton / locators); `67f0233c` name; use `ForceToDamage` / damaged model; hook surface grip into the
  tyre model and skid audio; a knocked lamp leaning on a slope can wedge the car (realistic, but check).
- Merge (2026-10-04, integration branch): prop physics is **opt-in** (`--prop-physics`; `--test smash` turns it
  on). On master's physics the default world drive (`--drive 911 --units 12 --auto-throttle 0.4` or 0.6) knocks
  the street lamp at z 1714, then stops dead against it and the crash barrels at z ~1722 (0 km/h at 0.6
  throttle). `--test smash` at 60 km/h now pushes the cone and the knocked lamp ahead of the car and stops at
  the fence (z 117.7, fence standing: its contact is car → lamp → fence, so the hook never sees the car's
  impulse); on the props branch alone the car was deflected after the lamp and drove past the fence. The
  "Verified" line above no longer holds on either. Frame time on the full map (free camera, vsync-capped):
  16.6 ms mean with and without prop physics, a few more spikes with it. The chase camera's wall ray and the
  planar reflection's ground ray now use the World layer only (the bumper box would otherwise block them).

### 2026-10-04 (session 2): NewSky shader read
- Main PS (`…79648d36`): glow = pow(saturate(dir · KeyLightDir), SunColour.w) × SunColour.rgb (exponent 0.65 at
  noon: a broad glow over the sun's half, not a disc); gradient = lerp(B(|y|), A(|y|), w) × BaseCloudColour.w
  with w = saturate(dot(norm(sun.xz), norm(dir.xz)) + DomeRanges.w) from the VS; four cloud layers, each
  lerp(sky, cloud.rgb × BaseCloudColour.rgb, saturate((cloud.a − CloudParamsPS[i]) × CloudParamsPS2.y)); output
  sqrt(colour / 4). VS: per-layer uv = (−TEXCOORD.y × s + o, −TEXCOORD.z × s' + o' + 1) from
  `BaseCloud{A..D}ParamsVS`; horizon fade 1 − ((TEXCOORD.x − DomeRanges.x) × DomeRanges.y)^DomeRanges.z.
- Ported: the glow (the old power-500 × 1.067 glow is gone; `2ec8df83` / `2ec8df82` are probably the sun
  disc, a separate draw). Not decodable from shaders: gradient B, the gradient scale BaseCloudColour.w and the
  per-layer cloud parameters (CPU-side). Dead end: `SkyReflectionAverageColour` / gradient hemisphere average
  ranges 0.3–5.6 over the day, so it is not a fixed multiple of the gradient.

### 2026-10-04 (session 2): sun and moon path
- `EnvironmentSettings` (`…69cad`) `SunMoonPath`: `BearingCentre` −232.37 (cracked), `BearingVariation` 35.06,
  `BearingPhaseOffset` −0.5; unnamed `0bb64ed2` 50, `c915f96d` 40, `1dc20d63` 28, `ffbdcf8b` 25,
  `ac4fe424` 35, `138500fe` 0, `a5caac0f` 8, `644383ee` 20 (crack with sun/moon/elevation/time vocabularies:
  no further hits). `CarSelect` blocks hold KeyLightBearing / KeyLightElevation for the garage scenes;
  `DayNightTransition` `1ab46d30` 8.5, `0e2be130` 20.
- Model (`SunMoonPath::key_light_direction`, provisional): bearing = centre + variation × sin(2π(h/24 +
  offset)) (centre at 12:00, ±35° over the day); sun elevation a sine arc 05:00–21:00 peaking at `0bb64ed2`
  (the default timeline's key light switches on at 05:00 and peaks at 13:00); moon from `644383ee` (20) to
  `a5caac0f` (8) peaking at `c915f96d`. **Unknown: the bearing's reference axis and direction**; assumed
  clockwise from +Z towards +X (noon: towards +X −Z). Needs one reference screenshot from the original
  (shadow direction at a known place and hour) to settle.
- The sky model's darkening away from the sun (gradient position, research log "deferred pipeline") is now
  visible with the sun behind the road-cliff camera.

### 2026-10-04 (session 2): the game's tonemap, auto exposure and colour cube
- **Light buffer units**: `LightBuffer_KeylightAndAmbient` adds RT3 × HDRConstants.y / Tonemap.w, undoing the
  exposure the G-buffer pass applied, so the light accumulation buffer holds plain game HDR units.
- **Tonemap** (`GBufferComposite`, both variants): hdr (6-tap motion blur) + bloom × B.w × B.rgb ×
  saturate(bloom × T.w), × (A.rgb × bloom.a + D.rgb), × T.w, then 1 − 2^(−x) and a polynomial sRGB fit
  (√c·1.271696 + (0.070706c − 0.317854)c − 0.022711). T.w (1×1 tonemap texture) = log2(e) × 2^(e + C0.z)
  (`_BuildConstant`), so display = sRGB(1 − exp(−hdr × 2^(e + bias))). DXBC `exp` / `log` are base 2.
- **Auto exposure** (`Blit2d_AutomaticExposureMeter`, `_AverageLuma`, `_AverageLumaFinal`): meter = luma
  (0.299, 0.587, 0.114) of the displayed colour under the current exposure; 16-tap averages down to one
  value L. State (e, L̄): L̄ = prev + 2^(−7.213475|C1.w|)(L − prev); scene = log2(1 − L^2.2) / −2^e;
  target = lerp(C2.y, C2.x, saturate((scene − C2.w)/(C2.z − C2.w))); e steps by |C1.w·C1.z| towards the
  target outside a ±C1.y band (proportionally inside), clamped to [C0.x, C0.y]; C0.w ≠ 0 fixes e = C0.x.
- **Keyframe Camera fields** (survey of 188 keyframes + the default timeline by hour): exposure range
  `3bec5831` / `3bec5027` (−8…8; noon −0.2…3, night 2.7…3.5), `ExposureCompensationBias` (cracked; 3 at
  night, else 0), scene-luminance range `4b48cdce` / `ec890b00`, luma targets `9355acc0` / `8357903e`, band
  `0abf68c5`, speed `e59ff4ec`. min ≤ max holds on every keyframe for both pairs. Dead end: the first reading
  (`4b48cdce`/`ec890b00` as the exposure range, pinned at 2 at noon) gave E = 4 and washed-out frames; with
  the S-curve grade on top it is clearly too hot. Also cracked: `DarkBloomWeight`, `DarkBloomWhitePoint`.
- **ColourCube (0x52)**: 16-byte header (N = 16, N, 0, 0) + N³ RGB8, blue fastest; `…3e93` is the identity.
  13 in ENVIRONMENT.BNDL, 8 in POSTFX.BNDL. Gamma in, gamma out. No PC post shader samples it except
  `UIRearViewMirrorShader` / `UIPersistentImage` (cube(saturate(rgb × ColourTintAndBloomHeadroom) × scale +
  offset)); the main view's use is assumed to be the same. The noon cube is a contrast S-curve (grey 51 → 41,
  153 → 192) with a warm cast.
- Implemented (`tonemap.rs`, `tonemap.wgsl`): meter (32² grid, 4×4 taps each), adapt (ping-ponged 1×1 state),
  tonemap + grade, replacing Bevy's tonemapping; the camera exposure makes the HDR target hold game units.
  Not yet: bloom, the composite tint (A, D), motion blur, PostAA, ColourTintAndBloomHeadroom.

### 2026-10-04 (session 2): key-light shadows (CSM lookup ported)
- `LightBuffer_KeylightAndAmbient` shadow lookup decoded (`gGlobalCB`: `ShadowMap_Constants{,2}`,
  `ShadowMap_ScaleOffset[3]`, `ShadowMap_WorldToLight0`, comparison sampler s15 on `shadowMap0` t15):
  - One atlas, three cascades stacked vertically: cascade i at uv = ls.xy × (0.5, −1/6) + (0.5, (2i + 1)/6).
  - Cascade 0 is `WorldToLight0`; cascades 1 and 2 reuse it with an xy scale + offset (`ScaleOffset[0].xy` /
    `.zw`, `[1].xy` / `[2].xy`) and a z offset only (`[1].z`, `[2].z`): same orientation and depth scale.
  - Selection: the first cascade with max(|x|, |y|) < 0.99; outside all three the reference depth is 0 (lit).
  - One `sample_c_lz` tap (2×2 hardware PCF), then lerp(C2.y, s, fade) with fade = saturate(C.w − view depth ×
    C2.w)², then squared, then × saturate(1 − 0.8 (1 − AO)).
  - Casters: `WorldShadow_Opaque_{Single,Double}sided` (white PS, position-only VS with
    `ViewProjectionModified`) are shadow-only proxy meshes; the loader still skips them (`skip_shader`). Vehicle
    and character shaders carry a `MaterialShadowMapBias` parameter (caster-side bias).
- Implemented: Bevy renders three directional cascades (2048², splits 20 m / … / 180 m, fade over the last 30 m:
  provisional, the game sets them from code) and `game_lighting.wgsl::game_shadow` samples them with the game's
  selection, single tap, fade and square. Bevy's receiver normal / depth bias stands in for the caster bias.
  Groundcover casts no shadows.
- Observation: shaded areas are much darker than in the game. Noon rig: key 11 units vs up fill 0.065 × 2.6;
  with only the rig cube as ambient the ratio is ~20:1. The game's RT3 indirect term also carries lightmaps and
  vertex light, which are not bound yet (M1.4.7): probably the missing sky light in shade.
### 2026-10-04 (vehicle lights and glass): VehicleNFS13 light, lens and glass shaders ported
- **Usage survey** (`mwtool models <car> --verbose` over all 74 `VEH_*_HI` bundles): besides wheels and
  body, the light / glass families are `Glass` (140 meshes), `Glass_Colourise` (118),
  `Body_Textured_NormalMap_Lightmap` (77) + `_Licenseplate` (73), `Glass_Textured_Lightmap` (73),
  `_Emissive_NoDamage_NoEffects` (72), `_EmissiveFourChannel_NoDamage_NoEffects` (71), `Refraction` (62),
  `BodyPaint_Lightmap` (31), `Body_Lightmap` (7), `LocalEmissive` (5, not ported).
- **`g_selfIlluminationMask`** (gObjectCB, set per car at runtime) switches the lights. In `Refraction` it
  multiplies the emissive texture's rgba against Brake / Headlight / Reversing / Taillight colours, so
  x = brake, y = headlights, z = reversing, w = tail. The lightmapped shaders use `mask.xyzy` against the
  Red / Green / Blue / Alpha channel colours: red = brake (911: 909 × 10, a huge HDR value), green =
  headlight glow, blue = reversing, alpha = tail glow, which also follows y. Ported as `--lights
  brake,headlights,reverse,tail` (default `0,1,0,1`); driving sets x from the brake pedal and z in reverse.
- **Lightmapped lights**: `EmissiveFourChannel`: sat(LM(uv0)² × mask.xyzy) × mSelfIlluminationMultiplier,
  dotted with the four channel colours, added to RT3. Its albedo is `PbrMaterialDiffuseColour` (no diffuse
  sampler), roughness the normal map's alpha. `_Emissive_`: sat(LM(TEXCOORD3).y² × mask.y) × multiplier ×
  green colour. It does **not** normal-map (o0.xyz = vertex normal) and its roughness is the normal texture's
  **green**. `Lightmap` variants (body, licence plate, paint): the same green term × the surface albedo after
  the coat. Roughness = `PbrMaterialRoughness`, F0 = `PbrMaterialSpecularColour` (the normal map's alpha is
  not used, which the old code got wrong). The lightmap UV is TEXCOORD3 for all of these except
  EmissiveFourChannel (TEXCOORD0). Emission is in the game's HDR units, so it is scaled by `GAME_UNIT` like
  the sky.
- **`Refraction` is an opaque G-buffer pass** (the old code drew it translucent grey): it renders the lamp
  interior. The eye ray in tangent space (VS output from TANGENT / BINORMAL / NORMAL), bent towards
  −(external normal map) by ParallaxConstants.z, offsets the UV by height = displacement.x × P.x + P.y (uv +
  (d.x, −d.y) × h). At the offset UV: internal normal map → RT0 normal, displacement.z → roughness,
  displacement.y → AO, `ColourSampler` → F0, Fresnel = lerp(colour.x, 1, PbrMaterialFresnel), albedo =
  `PbrMaterialDiffuseColour`. RT3 holds only (sharp cube(reflect(eye ray, n)) × ClearcoatSpecular + LED
  emission) × lerp(colour, mExternalGlassColour, its w) × AO, without the vehicle flag. On the 911,
  mExternalGlassColour = 14.1 × white, so the reflector is bright. The tangent sign for these float-frame
  meshes now comes from the stored binormal (`convert::mesh`).
- **`Glass_Colourise`** is a multiply-blend lens filter: the VS outputs lerp(1, MaterialColour_SimpleMultiply,
  g_PerVehicleFog.w) and the PS writes it with alpha 1. This is what makes tail lights red (911: (0.83, 0,
  0.009)). Ported with `AlphaMode::Multiply`.
- **`Glass*`** (forward, premultiplied, decoded for the clean / dry / uncracked path; `GenericVector4_A` =
  dirt, dust and wetness amounts at runtime): the volume colour is the **vertex colour** (Textured:
  lerp(vertex colour, diffuse, diffuse.a)), replaced by `DebugOverride_GlassVolumeColour` when its w > 0
  (artists use this: the 911's lamp glass has w = 0.05, an R8 piece w = 1). F = OpacityMin + (1 − OpacityMin)
  × FresnelFactor × (1 − N·V)⁵. colour = volume.rgb × volume.a × (key × N·L × shadow / π + irradiance) ×
  (1 − F) + lerp(cube(R), EnvironmentSpecular(R), SurfaceSoftness) × F, alpha = volume.a × (1 − F) + F,
  plus the running light LM(uv0).y × mask.y × multiplier × RunningColour (not squared here). Windows have
  vertex alpha ≈ 0.7 with near-black rgb (dark tint); headlight covers have alpha 0 (clear but for Fresnel).
  The diffuse term uses Bevy's forward lighting for now.
- Dirt / dust / scuffs are runtime-driven (`GenericVector4_A/B/C` × Effects texture). With them at 0 the
  body shaders reduce to the formulas above. Wheels: `Wheel_Textured_Roughness` keeps the vertex normal and
  reads roughness from the normal map's alpha.
- The tyre shader samples Diffuse / Specular / Normal and their Blur variants, but tyre materials bind only
  Effects / BlurEffects. Those textures probably come from somewhere at runtime; unresolved.
- Vehicle vertex colours now reach the shader as Bevy's COLOR attribute (UV_1 carries the lightmap UV), so
  the vehicle branch rebuilds its albedo from p3 × diffuse texture instead of Bevy's base colour.
- Checked visually: 911 (tail outline glow, red brake LEDs with `--lights 1,1,1,1`, projector headlight),
  Audi R8 GT (red tail clusters, LED DRL dots glowing at night), Veyron (round red tail lamps, LED bars).
### 2026-10-04 (damage): car deformation is skeletal animation, one clip per impact zone
- Started from PLAN M6.4's guess of a `DeformationModel`-like resource (Burnout Paradise's 0x10004). **Dead
  end:** vehicle bundles have no such type. Their resource types: Renderable, Model, Material, Texture,
  GeneSys, Wave/Ginsu, ConvexHull, PolygonSoupList, VehicleGraphicsSpec, VehicleSound, **Skeleton,
  AnimationList (×2), Animation, HSMData**.
- GeneSys types with damage in their name: `VehicleDamageBehaviour` (+ `.Bodypart`, `.DamageSequence`,
  `.SpotEffect`), `ControlMesh`, `WheelDamage`, global `ImpactDamageGraphs`, `CrashingRules`,
  `CollisionInfoDamageProfile`, `DamageBarProfile`, `LandingDamage`. `DamageSequence` handles point at audio
  / VFX `Sequence`s (sounds and spot effects when parts break), not geometry.
- **The vehicle vertex shader** (`mwtool shader-asm VehicleNFS13_BodyPaint`) is a 4-bone skinning shader:
  `gBoneMatrixArray` (48 × 3 rows) indexed by `BLENDINDICES × 3`, first weight `1 − (w.y + w.z + w.w)`.
  The `_NoDamage` shader variants drop `TEXCOORD1` and the crumple sampler; one variant reads
  `gavDamageVolumeValues` (3 × 5 bilinear grid over model XZ) for the crumple normal-map blend.
- `VehicleDamageBehaviour.AnimationList` → an `AnimationList` with 8 clips over the car's 37 bones
  (911). burnout.wiki documents `AnimationCollection` / `Anim` / `AnimSkel` for NFS MW (field names from
  PS3 debug symbols); the codec details were worked out here: rotations are 32-bit smallest-three (`0xDFF7FDFF`
  = identity; decoded angles grow smoothly frame by frame, which a wrong bit layout would not do), scales
  are one float per bone. Frame 0 is all zeros / identity, so translations are offsets from the bind pose.
- Which clip is which zone: from the bones each clip moves (bones cluster on the body: `mwtool damage 911`
  prints each bone's vertex centroid). On 50 of the 72 cars the moved bones point exactly at front, rear,
  left, right, FL, FR, RL, RR in clip order; on the rest a few clips are one 45° sector off (bones placed
  off-centre), never mirrored, so the order is fixed and the clip index is the zone.
- Posing the 911 at full front + left + front-left damage in the software preview: the bonnet buckles
  upward at its crease, the front bumper and plate disappear, the left mirror drops off. The bonnet rising
  (rather than sinking into the engine bay) also confirms the rotation sign.
- Skeleton bone IDs are not `field_hash`, zlib CRC-32 or BZIP2 CRC-32 of 1–3-word combinations of ~80 car part
  words in three casings (dead end for now; the hinge names `BonnetHinge` etc. don't match either).
- Runtime (`crates/mostrusted/src/damage.rs`): Bevy GPU skinning with one joint entity per bone; the
  body collider's contacts give the impact (avian's pre-solve `normal_speed`, speed-weighted contact
  centroid, near-vertical normals ignored as landings) → `ImpactDamageGraphs` curve → zone levels → clip
  frames. `--test crash`: the 911 into a wall at 115 km/h gets 29 % front damage (bonnet creased, plate and
  bumper pieces gone); at 172 km/h 56 %; a 35° glancing hit at 141 km/h gives 39 % front-left and 14 % left.
### 2026-10-04 (night): backdrop LOD tiers drawn on top of each other ("buildings on streets, mountains")
- **Report**: buildings randomly on streets and mountains on top of the map. **Cause**: every track unit's
  InstanceList backdrop (entries past the always-loaded count, 1,907 placements, 231 distinct models) was drawn
  for all 169 units at once. A unit's backdrop is what the game draws *while the player is in that unit*: its
  potentially visible set outside the loaded area, at the LOD that suits the distance. The same hillside is a
  22,747-triangle tile (`b2fe2fa7`) in the backdrop of units 49-53/137 and a 2,342-triangle tile
  (`28a73b07`) in that of 26 units farther away (same 1.9 km footprint, heights differ by up to 7 m); building
  blocks have a full and a LOD2 twin (`37852c3c` / `462b7b99`); 144 units list the whole-map horizon model
  `9eaa3c27` from GLOBALRESOURCES. Union = every tier stacked: coarse terrain pokes through roads and lawns,
  a beige slab covered the road at the road-cliff viewpoint, building-block stand-ins overlapped streets.
  The 2026-10-03 LOD2 strip only removed one symptom.
- **Evidence the game draws one unit's backdrop**: rasterised all main and backdrop geometry on an 8 m grid
  (temporary `examples/backdrop_raster.rs`, not committed). Only 12 % of a unit's backdrop cells fall on the main
  geometry of that unit and its ZoneList neighbours (2 % on its own), i.e. the backdrop fills the area that
  is *not* streamed. Tier pairs are listed by disjoint zone sets, and zone 137 lists the fine tile while its
  edge neighbour 48 lists the coarse one, so the game cannot draw neighbours' backdrops together.
  `--backdrop-zone 6` (only unit 6's list) at the city-lod-check viewpoint: roads clean, distant hills that
  unit 6 cannot see are missing (PVS).
- **ZoneList decoded** (`HAWAII/PVS.BNDL`, `mw_formats::ZoneList`, layout in formats.md): 169 zones = the 169
  track units (zone ID = unit number), each a 4-12 point polygon on the ground plane with district and
  arena IDs and a neighbour list (flag 3 = shared edge, mostly mutual; flag 1 = nearby, often one-way).
  `TRK_UNIT1298-1390` (district 152518) form a 3x3 grid east of the map with almost nothing in them.
- **Fix** (`mw_assets::backdrop`, default `BackdropMode::Composed`): compose one backdrop per place from the
  view of the zone that owns it. Inside a zone a backdrop triangle is drawn when that zone lists its model;
  outside all zones the models listed by the nearest listing zone win (the whole-map model only fills in);
  water meshes are composed separately (a near zone's lake bed must not hide a farther zone's water plane),
  and where a zone lists no water the nearest listing zone's water fills in. Triangles are split (adaptive,
  48 m, always above 512 m, depth 12) so tiles are cut at zone borders. Each backdrop model is sent once
  instead of once per listing unit (54,712 instead of 56,434 instances). `--all-backdrops` restores the old
  union, `--backdrop-zone N` shows one unit's list, `--no-backdrop` none. With `--units N` the zones that
  are not streamed count as outside and are filled from the nearest loaded zone's view (their stand-ins).
- Before/after (same cameras, `--all-backdrops` vs default): road-cliff: rocky coarse tile over the lawn and a
  flat slab over the road are gone; city-lod-check: terrain no longer smears over the curved road and the
  rails; south view: the lighter coarse patch on the front ridge is gone.
- Dead ends: (1) extra track units: all 169 bundles are in the zone list, none is a variant. (2) Per-model
  keep/drop: 49 of 231 models are only partly right (big terrain tiles span their own zone and far areas), so
  the decision has to be per triangle. (3) Nearest zone as the owner outside zones left holes where the
  nearest zone does not see a far tile; (4) letting water compete with terrain dropped lake water where a
  near zone lists the lake bed; (5) a 6-level split decided 600 m water pieces by one centroid (holes in the
  airfield lake).
- Remaining: cut faces where one zone's tile meets an area another zone represents differently (a mountain
  north of the airfield, `eec26d7c`, listed by 36/37/42, is cut into an arch over zones 46/55 that do not
  list it); trees in a few far places float where no zone lists ground under them (also with
  `--all-backdrops`); the river beside downtown is filled by a cheap far water plane (y ≈ 29); a few small
  triangle gaps where loaded and unloaded zones meet with `--units`. Full map load 19.4 s -> 23.9 s
  (indexing 1.7 s, the rest is triangle splitting on the loader thread).
### 2026-10-04 (drift session): tick-exact tests, drift model from `DriftBehaviour`
- **Handling tests are now deterministic.** `--test` scripting ran in `Update` at 4× time, so the input changed
  on frame boundaries, not physics ticks: 0–100 km/h on the 911 measured 3.93, 4.33 and 4.91 s on different
  runs. The script now runs in `FixedUpdate` before `car_physics`, counts physics ticks from the car's first one,
  and `car_physics` applies the scripted input itself. Two runs of every test print identical `TEST RESULT`
  lines (accel 3.92 s, brake 37.1 m, corner 1.01 g, drift, handbrake, donut).
- **Field names cracked** (one-word templates, prefix × subject × suffix, ~155k candidates against the 44
  unnamed `DriftBehaviour` hashes, so about 10⁻³ false hits expected): `Slip_angle_to_enter_drift` (8e4032c0),
  `Minimum_speed_to_enter_drift` (d71a857c), `Drift_angle_to_exit_drift` (4971b7ae), `Drift_angle_for_side_force`
  (3ca074df), `Minimum_drift_angle_for_side_force` (f6c50d59), `Drift_scale_for_maximum_side_force` (aadfc7b1),
  `Speed_for_maximum_side_force` (9e6f2943), `Drift_scale_from_counter_steering` (005c9352),
  `Low_slip_angle_for_counter_steering` (0e2a4c75), and `Mid_yaw_torque` (e324fa82, confirmed). Handbrake:
  `Brake_strength_at_low_speed` (b0bb7eaa), `Brake_strength_at_high_speed` (4af55f8a, formerly "handbrake
  torque"), `Low_speed_mph`, `High_speed_mph`, `Minimum_handbrake_time`, and the unused
  `Handbrake_grip_values.{Front,Rear}_wheel_grip_factor`. Free 5- and 6-word searches over the same vocabulary
  gave only nonsense beyond these (as expected from their false-hit rates); still unnamed: the yaw-curve
  breakpoints `2ef426dd` / `70528afa` / `a5c9496f`, trigger `8f151151` / `0dce38f7` / `b5290695`, and
  most of `Other`.
- **Survey facts** (79 `Drift` blocks): `Slip_angle_to_enter_drift` ≥ `8f151151` on every car; drift-happy cars
  have the lowest entry angles (Lancia 5.5°, 350Z 6°, M3 6.25°), AWD supercars the highest (8.5–9°). The two
  Ford Police Interceptors have `Maximum_drift_angle` 180, `0dce38f7` 90, `b5290695` 100 and `35e937b8` 50:
  thresholds pushed out of reach, i.e. drifting off. `Max_yaw_torque` relative to weight varies 3× (911 high,
  F-150 low).
- **Drift model** (`crates/mostrusted/src/drift.rs`, details and per-field status in handling.md): Off →
  Entering (drift scale over `0dce38f7`, or handbrake, while steering) → Drifting (slip over `8f151151`, or
  directly over `Slip_angle_to_enter_drift`) → Off (under `Drift_angle_to_exit_drift`). Yaw torque from the
  Min/Mid/Max curve against steering, faded towards `Maximum_drift_angle`; `Drift_angular_damping` on the yaw
  rate; inward side force; automatic counter-steer; a drift level that decays below `Drift_angle_for_decay`.
  Donuts from `Donut_yaw_torque`, scaled by `Speed_influence` × mph.
- **Dead ends while calibrating** (911 unless noted, steering 0.8 after a brake tap at 110 km/h):
  - Torque and damping only (no fade): the car spins at once (82°, down to 10 km/h). The data's damping read at
    60 Hz (3.1 /s) gives ω ≈ τ / (3.1 I) ≈ 2 rad/s at 0.8 steering; at 120 Hz it holds 20–33° but still drifts
    up slowly, at 240 Hz the car stays under the side-force decay angle and never really drifts.
  - The tyre moment hardly changes with the drift angle once the rear saturates, so something must make the
    torque depend on the angle; a linear fade to `Maximum_drift_angle` worked except on full lock and throttle
    (spin), power 2 is stable everywhere but shallow (12–28°), 1.5 gives 14–30°.
  - Damping the yaw rate relative to the path's turn rate instead of the absolute yaw rate: spins at 0.8.
  - Front wheels following the player's lock in a drift: everything over-rotates and scrubs speed (911 down to
    24 km/h) but the heavy cars drift; blending the lock in below `Low_slip_angle_for_counter_steering` + 10°
    lets the F-150 and Veyron hold 11–14°, but the 911 spins on full lock and throttle and the F-150 crabs
    sideways with no yaw. Kept the full counter-steer.
  - Side force pointing outward instead of inward changes the angles by only 2–4°.
  - Donut torque without a speed factor: the car pirouettes at 200–270°/s on the spot.
- Open: the yaw-curve breakpoints, what `Default_steering` (0.2–0.4) does, the side force's units, the
  per-tick time base, and the heavy cars (they barely drift here; if they drift in the original, its rear
  grip in a drift is lower than this model's). Measuring drift angles in the original game would settle most of
  these (handling.md, "Matching the original game").

### 2026-10-04 (evening): engine sound rewritten from the game's audio data
- User report after the resampling fix: the engine "still plays snippets in full and isn't blending them".
  Cause: grains (whole engine cycles, up to 77 ms) were butt-joined with a 2 ms cross-fade while
  alternating between two grains, and only two of a car's six recordings played.
- **The game's mix is data** (`Genesys.Gen.EngineSound2` in the vehicle bundle, `mw_assets::engine_sound`):
  `Sequences` → `Sequence` objects. Two hold three `GinsuSequenceItem`s each (Ginsu + `MixerChannel` +
  RPM range 1790–7500 on the 911), all playing at once: per microphone set a full-load rev-up, a part-load
  rev-up and an off-load fall. A third sequence (0..1 binding: engine load) has `MixerChannelSequenceItem`s
  whose gain curves (`AnimPoints`: x `12d3a8aa`, y `12d3a8ab`, four tangent fields not decoded) crossfade
  the channels. 911 set A: full 0 → 1 from load 0.53 to 1, part a triangle peaking at 0.47, off 0.86 → 0 by
  0.5; set B similar. Channel `Gain` 0.709 / 0.709 / 1.0; `MasterGain` 1.25. Bus channels carry EQ /
  distortion / sub-harmoniser / filter curves (not used yet). A second `EngineSound2` with one sequence uses
  the short Ginsu copies (probably distant / low quality). Veyron, GT-R, Mustang and Focus also give six
  layers.
- The two microphone sets are one recording run: equal grain counts (506 / 506), boundaries offset by a
  constant trim (~1,200 samples), cross-correlation 0.2. Independent synths put them out of phase (sweep
  pitch-tracking p90 150 cents vs ~65 for one layer); a shared period schedule and shared randomness fixed
  that (p90 63 cents).
- **New synth** (`mw_audio::ginsu`): overlap-add of engine cycles. Each output period blends the current
  cycle out and the next grain's preceding cycle in with cos² / sin² windows, both resampled to the period
  of the current RPM (pitch follows RPM continuously). Consecutive grains share samples, so straight playback
  is seamless; the walk takes the next cycle 7 periods in 8 within ±2 grains of the RPM's grain, else a
  random grain in that window. All layers share one schedule (the first layer sets the period) and one random
  stream. Load is smoothed over 50 ms, RPM over 10 ms; the sum goes through `tanh`.
- Measurements (911, `mwtool engine-sound`): 2,000 → 7,500 rpm sweep p90 pitch error 63 cents (tracker floor
  ~65 for a single layer), worst discontinuity ratio 0.71 (was 0.82); 2-20 Hz envelope modulation on steady
  holds 0.008–0.038 for most rpm / load cells, close to the raw recordings' 0.04–0.05; exceptions full load
  at 2,500 rpm (0.098: long low-rpm cycles make any small window loop at a few Hz) and off load at 4,500 rpm
  (0.082; the off-load recording has only ~41 grains per 1,000 rpm). Strategies compared: strict walk with
  jump-back (loops), random grain every period (more modulation), wider windows (more modulation).
- Assumed / not decoded: the load binding is fed with throttle (reverse: brake); the sequences' binding raw
  values, curve tangents, whether RPM is clamped to the items' 1790–7500 range, the bus effects, engine
  states (Burnout / Drift / In_air / Reversing lists).

### 2026-10-04 (later): the game's deferred pipeline in Bevy, environment timeline
- **Pipeline**: opaque game materials now run in Bevy's deferred prepass and write the game's four G-buffer
  targets (RGBA8 each, packed into Bevy's 128-bit deferred texel) with deferred-lighting ID 2. A full-screen
  system in the `Core3d` schedule (`deferred.rs`, `game_lighting.wgsl`) ports `LightBuffer_KeylightAndAmbient`
  and the `Fog` pass and lights those pixels; it runs before Bevy's own deferred lighting (which only touches
  ID 1) and stores the ID depth, because Bevy's pass discards it. RT3 (indirect) is computed in the G-buffer
  pass as the game does: sky reflection cube at mip = roughness × 5.5 × env-BRDF (Karis' analytic fit stands
  in for the game's 3-D LUT) × AO², plus the vehicle clear coat's sharp reflection. Translucent materials stay
  forward with Bevy lighting. Verified: tinting the pass output shows it draws every opaque pixel.
- **Environment timeline decoded** (a Sonnet agent surveyed and cracked the names; formats.md has the
  layout): 25 timelines × 21 hourly keyframes; default timeline `…69cbb`, noon keyframe `…124619` (key light
  (11, 7.95, 5.21), ambient scale 2.6, fog near 80 / far 1525 / power 1.5, cloud texture `…3f46`). 29 names
  added to `genesys_names.txt`, each checked against its hash. Seven `Alternate` keyframes are bad-weather
  twins (dimmer key light, nearer fog); nothing found yet selects a timeline for free roam. (Superseded on
  2026-10-04, weather entry: 154 keyframes share 7 Alternates; a WorldPaintMap selects the timeline.)
- The renderer now takes key light, ambient rig (→ irradiance cube), sky gradient + sun colour + cloud layer
  (→ skybox and reflection cube) and fog from the keyframe at `--hour` (default 13). Game HDR units × 1200 =
  cd/m² / lux (`GAME_UNIT`) until the game's exposure is ported.
- **Dead end / correction**: the two HDR panoramas used earlier this session are a sunrise capture (glow 3°
  above the horizon) that no keyframe references; no longer used.
- Provisional (marked in code): sun direction (hand-picked; SunMoonPath fields mostly unnamed), sun glow
  exponent 500 and scale 1.067 (unnamed sky fields `2ec8df83` / `2ec8df82`), cloud threshold = 1 − Opacity
  and sharpness = 1 / Feathering, cloud dome mapping, gradient position = sin(elevation), the fog colour's sun
  gradient (pow 4 of the azimuth cosine), no height-layer fog, in-scattering disabled (the raw phase 0.971
  in strength/(1 + phase·cosθ)² floods the view; the CPU-side mapping to ScatteringSettings is unknown).
- Remaining Bevy stand-ins: Bevy's tonemapping and default exposure (the game has auto exposure, a tonemap
  pass and a ColourCube grade), `GeneratedEnvironmentMapLight` for forward materials, no shadows (the game
  uses a cascaded shadow map), the env-BRDF fit.

### 2026-10-04 (later): engine rev-up staircase fixed
- `GinsuSynth` now resamples every grain by `rpm / recorded rpm` (Catmull-Rom read position, clamped to
  0.7–1.4) and fades into the old grain's continuation at the same rate. Each grain's recorded rpm comes from
  inverting the RPM table at its centre (`Ginsu::rpm_at_position`); on the 911 it matches the grain-length
  estimate `120 · sr / len` within 1 % from 3,000 rpm up (1.7 % on the first grain).
- The mixer's RPM smoothing went from 30 ms to 10 ms: with resampled grains the 30 ms filter was the main
  remaining lag (20–40 cents on a 2,200 rpm/s sweep). 10 ms still bridges the 64 Hz updates.
- Measured on a 2,000 → 7,500 rpm sweep in 2.5 s, 30 ms windows, harmonic-sum f0 at rpm/20 (±25 %), scored
  against the rpm delayed by each version's control latency: median −30 → −5 cents, p90 |error| 84 → 59
  cents, p95 window-to-window step 131 → 91 cents. Discontinuities (peak |diff| / local RMS per 2 ms block):
  max 0.88 → 0.76. Steady holds: envelope modulation unchanged (0.020 / 0.035 / 0.003 at 2,500 / 4,500 /
  6,500 rpm off load); on-load holds sit within 3 cents of the expected pitch.
- Metric caveats (dead ends): below ~3,000 rpm an engine cycle (48 ms at 2,500) is longer than the window, so
  per-window errors there are tracker noise; around 4,900–5,300 rpm the 911's recording has partials that
  pull a 5-harmonic comb to −200 cents on steady holds with both synths; the off-load recording has little
  energy at the firing frequency at 4,500 rpm (the tracker hits its bound). A comb on the engine-cycle
  fundamental (rpm/120, 30 harmonics, ±10 %) failed even on a clean synthetic sweep.

### 2026-10-04 (shaders): G-buffer decodes, tangent frames, instance overrides, paint, sky
- **G-buffer semantics** (from the main pass of each family; t-registers mapped through sampler slots of the
  programs that keep RDEF names): RT0 normal + roughness, RT1 sqrt(albedo) + AO, RT2 F0 + Fresnel strength,
  RT3 sqrt(indirect × exposure). Reflection = cube.rgb × SkyReflectionAverageLight + cube.a ×
  SkyReflectionAverageColour, times an env-BRDF LUT sampled at (N·V, roughness, F0) × 0.9375 + 0.03125.
  Exposure = Tonemap(0.5, 0.5).w × HDRConstants.x. A full per-family write-up (13 groups, ~330 lines) was made
  in the session scratchpad; the ported maths is in `layered.wgsl` with comments. Highlights:
  - WorldPBR: F0 = cubic(spec.x) (sRGB→linear approximation), roughness = spec.y, AO = AO map(uv1).y ×
    instance w. ColouredSpecular: F0 = SpecularColour(uv0).rgb × MaterialDiffuse, no cubic. PersistentLightmap
    = Lightmap without the `DynamicLighting_GlobalLightMapIntensities` factor. Lit windows: emissive =
    spec.w² × AOMap.w × LitWindowColour × AdditiveIntensities.y × albedo (night only, not drawn).
  - Terrain: tiling normals hold x, y and roughness (w), z reconstructed; the `Normal` variant's macro normal
    map is world space (texel swizzled x, z, −y); AO = 2·mask.g; F0 / roughness / Fresnel per layer from
    `Layer{1,2,3}_Roughness_Specular_Fresnel`.
  - Roads: normals blended in slope space (road map + world-space detail map + vertex normal, then
    normalize(p.x, 1, p.y)); roughness = Normal.a × Detail.a; F0 = SpecColour.g; wetness from a puddle map,
    RoadDepth and vertex TEXCOORD5.x; reflection only planar × wet mask, so dry roads have no indirect
    specular. Road vertex colour: rgb = baked light, a = AO.
  - Plots: blended tiling normals (rgb), roughness = lerp(decal.z, 1, layer.x × normal.a), F0 = layer.y.
  - Trees: G-buffer albedo = (texel / alpha)² (premultiplied, gamma space), AO = vertex colour.y × instance w,
    wind = sin(time·speed + uv.y) × amplitude × uv.x on x and z, no specular.
  - Water: two wave normal maps scrolling by time / waveAnimationSpeeds; the reflection's Fresnel lookup uses
    dot(−V, N), negative on visible water, so it always takes the grazing value (near-mirror water).
  - Vehicles: textured parts F0 = spec rgb, roughness = normal-map alpha, AO / specular occlusion = vertex
    colour x / y; clear coat F0 = PbrMaterialClearcoatSpecular rising to 1 at grazing angles by
    PbrMaterialClearcoatFresnel; base scaled by (1 − coat). The tyre material binds no diffuse / spec / normal
    textures although its pass reads them (D3D11 unbound = 0): black rubber plus the dust layer.
- **Packed tangent frames decoded** (world RGBA8, vehicle SNORM16): see formats.md. Test
  `mw-assets/tests/tangent_frames.rs`: normals agree with face normals on 98.8 % (TRK_UNIT1) / 97.3 % (911)
  of triangles; tangents follow +U on 82 % / 68 % (vehicle normal maps also use other UV sets).
- **InstanceList per-instance overrides** (+0x10) and the world matrix's w column (tint / AO): formats.md.
  Prop / compound instances have a similar block at +0x50 (`InstanceAdditiveAndAO`), not decoded; their tint is
  treated as 1.
- **VehiclePaint field names** cracked with the paint shader's constant names as vocabulary (14 names, in
  `genesys_names.txt`). `a1d177bd` (0.019–0.05) is read as the coat F0 by range and position; unconfirmed.
- **Sky**: `ENVIRONMENT.BNDL` panoramas and cloud layers (formats.md). One daytime pair (`…3f39` HDR,
  `…3f3a` clouds) is used as skybox and environment-light source until the timeline is decoded.
- **Bevy stand-ins** at the time (superseded by the deferred pipeline entry above): `StandardMaterial`
  lighting fed with the ported G-buffer values; `GeneratedEnvironmentMapLight` from the sky cube instead of the
  game's reflection cubes + LUT + irradiance quadric (needs Bevy features `dfg_lut` and `bluenoise_texture`;
  without `dfg_lut` the environment specular silently does nothing); `DistanceFog`; Bevy's clear coat (F0
  fixed at 0.04, strength set to the coat's grazing reflectance); water near-mirror approximated with F0 0.6;
  directional light at a hand-picked angle (sun azimuth aligned with the panorama's glow); no shadows.
- Open: lightmap texture source (the lightmap shaders read `LightMapTextureSampler`, which neither the
  material nor the instance overrides bind); road vertex AO and baked light; water river floor (third UV set),
  foam, caustics; paint flakes / pearlescent / livery; vehicle glass and lights; TerrainPBR TangentSpaceNormal
  macro frame.

### 2026-10-04 (late night): jumpy engine sound
- **Ginsu RPM tables are signed.** The short recordings start at about -2832 samples, so as u32 their tables
  looked like they were falling. They are in fact *rising*. They are lower-quality copies of the main rev-ups:
  same RPM range, same first and last grain sizes, fewer grains.
- **Recording layout per car**: two sets of three long recordings, in resource order (likely engine and
  exhaust): the main rev-up under load, a second rising sweep from lower RPM, and a falling (off-load) sweep.
  Then come the short copies of the two main rev-ups. Grains are one engine cycle, i.e. two crank turns (911:
  3684 samples at 1575 rpm, 734 at 7814 rpm, at 48 kHz). `pick_engine` used to take the rising recording with
  the most grains and, separately, the "falling" one with the most grains. On the 911 that paired set A's
  rev-up with set B's off-load sweep, so timbre, loudness and RPM range changed at every throttle change. It now
  takes the rev-up that has a short copy, plus the falling sweep of the same set (nearest in resource order).
  92 of 93 cars get such a pair; one car has no Ginsu at all.
- **RPM signal** (`MW_ENGINE_TRACE=<file>` with `--test engine`, rendered by `mwtool engine-sound --trace`):
  1. The launch-clutch slip applied in every gear below about 5600 rpm, so a throttle tap in 3rd at 115 km/h
     moved the revs about 600 rpm in one tick. The slip now applies only in 1st and reverse.
  2. The race gearbox's downshift points (911: 3700–6000 rpm) also applied off the throttle, so coasting and
     braking kicked down with jumps of about 1800 rpm. These are now kick-down points: off throttle the box uses
     the cruising gearbox's points (2000 rpm), blended by pedal position.
- The on/off-load mix is now an equal-power cross-fade (the recordings are uncorrelated, so a linear fade
  dips 3 dB).
- Measured (911, 10 ms loudness steps and 20 ms spectral-centroid steps): the off-load sweep through the
  synth is now smoother than its raw recording (centroid steps over 3 semitones in 6% vs 9% of windows), and
  loudness steps over the human-like pedal trace fell from 4.0 to 3.0 dB (p95).
- Dead end: picking the next grain at random around the target instead of alternating made held-RPM
  modulation worse (0.069 vs 0.018 at 2500 rpm off load).
- **Rev-up staircase**: grains play at their recorded pitch and change only at grain ends, so under
  acceleration the pitch steps (2000→7500 rpm in 2.5 s: up to 118 cents behind, steps up to 370 cents per 5 ms).
  Fix planned: resample grains by current rpm / grain rpm (see next-session.md).
- Still open: a steady 4500 rpm off load warbles at about 18 Hz (two dissimilar neighbouring grains
  alternate), and the second sweep and second set are not used yet (layering, `EngineSound2` data).

### 2026-10-04: world layers
- Prop, Compound, Dynamic and Light instance lists share one record layout (see formats.md): 0x60 bytes, matrix
  at +0x00 (InstanceList has it at +0x20), object import at +0x40. Whole install: 507 lists, 21068 objects,
  3424 object definitions, 3628 lights, all parse and every object resolves to a model
  (`crates/mw-assets/tests/world_layers.rs`). Map placements 35431 -> 56499 (props 12290, compounds 4301,
  dynamics 4477).
- Visual check (unit 1 road, spawn area): tyres, barrels, pallet, road sign and street lamps stand upright on
  the ground, not mirrored. No axis fix needed.
- CompoundObject model slot 0x24 is a lower-poly twin of slot 0x04 (same bbox, ~1/3 to 1/2 the triangles);
  we draw slot 0x04. Guess: 0x24 is a distance LOD. Unverified.
- Translation-row w of the 3x4 matrices is 0.94 for props (240/255 as float?) and 1.0/0 elsewhere. Unknown.
- **Groundcover** (0x20F collection + 0x214 instance blobs), decoded from scratch (layout in the doc comment of
  `crates/mw-formats/src/groundcover.rs`): the game has no mesh for it, the `Groundcover` vertex shader expands
  16-byte instances into camera-facing wind-swayed sprites. Per collection: up to 5 materials (one atlas
  texture each with 16 texture regions), 3 layers, a bounding-volume tree of 0x60-byte nodes whose leaves
  point at 0x40-byte groups (box, material index, quads, blob + start + count). Instance x/y/z are u16
  fractions of the group box. All 400 collections / 1,364,853 instances in the install parse, group counts add
  up to the layer totals, and every blob resolves (`all_groundcover_parses`).
  - Dead ends: first thought `start` was a global index (it is `start << 16 | blob`, blobs hold <= ~8190
    instances), and that material = layer (it is the group's +0x24; a collection with 1 material uses it for
    all layers). The first sub-record fields were mistaken for leaf headers.
  - Verification: neighbour instances within 1.5 m differ in y by 0.0 (median) when the axes are (x, y, z) and by
    0.6 when permuted; against collision triangles ~25% of sampled instances sit within 5 mm of the surface
    (the rest are over bridges/water/other layers). Visually small grass tufts stand on the lawn at
    (-1135, 24.8, 1606).
  - Guesses: size byte / 32 = height in metres (the box margin r equals the largest size byte / 32); byte 6
    is yaw (random in some groups, equal to the size in others); tint = albedo * 2 (the shader outputs
    sqrt(2 * tint)); sprites per instance = `kind / 2` crossed quads. Instance density is low (1.36 M for the
    whole map) so verges are mostly bare; some dark tufts are dark because the tint is.
  - Runtime: merged into one mesh per collection and material on the loader thread (+2.7 s load of ~32 s,
    peak RSS +23 MB), static crossed quads instead of the shader's billboards, `--no-groundcover` to skip.
- The big leaf-card quads sticking out of tree canopies in the road viewpoint predate the new layers (identical
  with only `InstanceList` placements and `--no-groundcover`); not investigated.
- Open: groundcover wind/billboarding and fade distance, `cb1[0].x` size constant, light definitions
  (GeneSys, not decoded); CompoundObject parts (0x14/0x18 arrays of 0x30-byte sub-transforms, probably
  breakable pieces) unexplored; ZoneHeader is just a list of imports to the above (not needed to place things).

### 2026-10-04 (night): collision holes, engine note, chase camera
- **PolygonSoupList quads are in triangle-strip order** (the fourth vertex is opposite the first): split them as
  (a, b, c) + (b, d, c). The fan split made every quad a bow-tie with a triangular hole, which caused the gaps in
  the floor and phantom surfaces (the "drive-911" viewpoint car used to stop on one). Checked by normals: all
  225,442 collision quads in the install give two same-facing triangles with the strip split (1 exception);
  `crates/mw-assets/tests/corpus.rs`.
- **Engine note**: the synth was faithful (rendered pitch follows the recording; the autocorrelation f0 readings
  of rpm/40 above ~4,500 rpm on throttle are octave errors, because that recording has a strong every-other-cycle
  pattern). The fault was the RPM signal: at launch it held the clutch rpm flat for ~1.5 s. Now it rises with the
  wheels, and `Engine_inertia` limits rev changes. `mwtool engine-sound --trace` renders a recorded trace
  through the same `EngineMixer` the game uses.
- **Chase camera**: `VehicleCameraContainer.External` gives the eye and look-at points, the FOV range and
  roll/pitch/shake amounts (table in [handling.md](handling.md)). The FOV is vertical judging by the framing.
  Physics runs at 64 Hz; the car now uses avian's `TransformInterpolation` + Hermite easing, and the camera
  smooths car-relative offsets only (smoothing its world position made it trail by v/6 m).

### 2026-10-04 (later): GeneSys field-name hash cracked, handling from named data
- **The field-name hash is CRC-32** (polynomial 0x04C11DB7, MSB first, not reflected), computed with the
  shift-register ("non-augmented") algorithm: the register starts as the complement of the first four bytes,
  each later byte is shifted in at the bottom while the byte shifted out goes through the table, and the result
  is complemented. Names of four bytes therefore hash to themselves (`Mass` = 0x4d617373), which was the clue.
  `mw_formats::genesys::field_hash`.
- How it was found: nested types are named `Parent.Child` (`Engine.Sound`, `DriftBehaviour.Side_force`), and
  the field holding one is named `Child`, which gave known (name, hash) pairs. Two fields in one type differed
  by XOR 0x0816 = `"in" ^ "ax"` (…min / …max): the hash is XOR-linear with one byte per 8 bits, i.e. polynomial
  division. XORing two same-length names cancels the unknown init, and dividing their difference by small
  quotients gave the polynomial (0x04C11DB7 from every pair); the per-length constant then fixed init/xorout.
  Earlier attempts failed because they hashed with the standard (augmented) CRC.
- Recovering names: `mwtool genesys-crack` builds names from word lists in the game's styles
  (`Capital_snake_case` for designer data, `CamelCase` and `Title_Snake` elsewhere). The exe's strings are
  encrypted, so word lists come from type-name components and English. False positives are real
  (`High_race_angle_efficiency` collides with `Mass`; `Typed_Visual` with `655d3f5f`), so names were
  accepted when they read naturally, fit their sibling fields and fit the values (`Top_speed_MPH` = 189).
- Handling names that settled earlier guesses: `RaceGearbox` vs `CruisingGearbox` (the race box is the
  player's), `Gear_change_time` (0.15 vs 0.9 s), `Differential` = final drive, `Power_To_Front`,
  `Engine_resistance`, `Idle_RPM` / `Max_RPM`.
- **Tyres**: seven curves per axle: `Pure_Lateral_Grip` / `Brake_Slip_Lateral_Grip` /
  `Traction_Slip_Lateral_Grip` (`Gradient`, `Peak_Force_Coefficient`, `Peak_Slip_Angle`) and
  `{Brake,Traction}_{Pure,Slip}_Long_Grip` (`Peak_Force_Coefficient` at `Peak_Slip_Ratio`,
  `Max_Slip_Force_Coefficient` at `Max_Slip_Ratio`). On most cars `Gradient = 3 × peak / angle`: the curve is a
  Hermite from 0 to the peak with a flat end tangent. Details in [handling.md](handling.md).
- **Steering**: per speed band (`Medium_speed` / `High_speed`, mph) a wheel-angle range and separate rates for
  steering in, across and back to centre.
- `RigidBody` vector `62dbd683` is the bounding-box centre (the physical definition's value minus the body
  offset gives the body's), so the body transform's translation is indeed the centre of mass.
- `VehicleCameraContainer`, `VehiclePaint` and `WheelSizes` objects sit in every vehicle bundle: likely sources
  for chase-camera offsets and paint colours.

### 2026-10-04: texture scaling, audio
- `mwtool shader-usage` ranks shaders by world triangles. Every top family was checked in its vertex shader:
  WorldPBR / World_* use UV0 unscaled (correct already). PlotPBR scales UV0 per layer by
  `Tiling_Ratios_X/Y`, its mask by `maskTilingRatios` and its decal by `decalTilingRatios`, tints each layer
  by `tiling{1,2,3}Diffuse`, and blends `decal.x * decal_Diffuse.rgb` by `decal.y * decal_Diffuse.w`. Road
  line variants draw `Line_Diffuse_Sampler` at TEXCOORD2, tinted by `Line_Diffuse`, over the normal road
  albedo by the line alpha. `World_GbufferBlend` meshes are decals (alpha blended). Cables use a gradient
  texture, not a diffuse texture. Vehicle shaders use UV0.
- **Audio codecs**: every `.SPS` and `Wave` resource is EAAudioCore v1 with codec 5 = EALayer3 v1 (mono to
  5.1, 16–48 kHz). 135 `.SPS` files start with a `D` block (headerless; their headers live elsewhere, not
  decoded yet). Engine sounds are Ginsu (`Gnsu30`) with EA-XAS v0.
- **Ginsu layout**: vgmstream's audio offset is 4 bytes short for MW: there is a u32 (0) at 0x20, the RPM
  table starts at 0x24. Evidence: at vgmstream's offset XAS frame starts jump ~3× more than frame interiors;
  4 bytes later the ratio is 0.99. The RPM table holds recording *positions* (samples) at evenly spaced RPMs
  (rising for on-load sweeps, falling for off-load); the grain table holds grain boundaries.
- Each car has eight Ginsu recordings (two sets of three sweeps plus two short ones); which is engine /
  exhaust / interior is not decoded yet. On-load = rising RPM table, off-load = falling.
- Synth check without listening: a rendered rev sweep's spectral centroid rises 715 → 1400 Hz and falls back
  on the coast; grain switches must cross-fade with the old grain's *continuation*, not its already-played
  tail (that clicked every ~40 ms).

### 2026-10-03 (night): GeneSys decoded, handling, LOD stand-ins, controller
- **GeneSys decoder works** (`mw-formats/src/genesys.rs`, `mwtool genesys`). Field flags: low byte = alignment,
  `0x100` pointer (in-chunk sub-object; with an import on the field = external ref), `0x200` fixed array,
  `0x800` dynamic array, `0x1000` array of pointers. Array lengths are trailing small unsigned fields, assigned
  to arrays sorted by (alignment descending, offset): RigidBody's bool array is laid out first but counted last.
  Every struct starts with a type import at +0.
- Field names of four characters or fewer are stored verbatim (`Mass` = `0x4d617373`); longer names are hashed
  with the still-unknown function.
- All vehicle handling data is in GeneSys; see [handling.md](handling.md) for the mappings and evidence.
- Three objects per vehicle bundle fail to decode (pointers read as huge offsets). Their types are not needed
  for handling; still open.
- **Wheel labels can lie**: the Veyron's spec names its rear wheels "Front". All cars face +Z, so wheels are now
  classified by position (front = z above the midpoint; mirror when x < 0).
- **Backdrop instances**: InstanceList entries past the "always loaded" count are world-space geometry at the
  identity transform (terrain, water, roads, horizon). They also include LOD2 stand-in buildings
  (shaders containing `LOD2`) that overlap the detailed buildings; those meshes are now stripped from backdrop
  placements (`--keep-lod-proxies` brings them back). This was the "misplaced assets" report.
  (Superseded on 2026-10-04: the backdrop is a per-unit LOD view; see the backdrop composition entry.)
- **Controller**: gilrs's bundled SDL mapping DB lacked the 8BitDo Ultimate 2 (`03000000c82d00000b31…`), so its
  triggers were unmapped. The current SDL GameControllerDB is bundled and passed via `SDL_GAMECONTROLLERCONFIG`;
  `--gamepad-debug` logs raw input.
- Physics pitfalls: avian `LinearDamping` acts like ~1.7 kN of drag at 240 km/h (now 0); suspension damping
  must use velocity along the ground normal, not the car's up axis (porpoising at speed).

### 2026-10-03 (evening): collision and driving
- **PolygonSoupList** decoded (layout in `mw-formats/src/collision.rs`): world-space, vertices `u16 ×
  500/32768 + i8 multiplier × 500 m`, quads before triangles. One list per track unit (TRK_UNIT1: 158 soups,
  6.9k triangles). It includes invisible walls / barriers.
- GenesysType kinds: 0 int, 1 uint, 2 float, 3 bool, 4 char, 5 enum, 6 compound value (Vector3, StringBase),
  7 class/struct, 8 resource handle. The type name is at `*(*(type+0x20))`, size at 0x18, fields inline from
  0x24 (0x1C each: type import, type-hash low32, 0, name hash, size, offset, u16 count, u16 align). Objects
  start with the base type's fields (0xC bytes for the common base).
- Drivable prototype: avian3d 0.7. Pitfalls hit: (1) wheel rays must exclude the body's child collider entity;
  (2) suspension damping from finite differences launches the car on first contact, so use the body velocity at
  the mount; (3) explicit `Mass` alone leaves collider-derived (density 1) inertia, so set `AngularInertia` and
  `NoAuto*` explicitly.

### 2026-10-03 (later): vehicles, names, shaders
- **Imports**: the high bit of the import offset word is a kind flag. `0` = pointer slot, `0x8xxxxxxx` = handle
  (the ID is stored in place). GenesysObject, ZoneHeader, the VehicleGraphicsSpec locator tables and others use
  handles. `mw_bundle::ImportKind`.
- **VehicleGraphicsSpec** decoded (layout in `mw-formats/src/vehicle.rs`): body part models, skeleton, collision
  PolygonSoupList, and four 0x90-byte wheels (pos, quat, scale, part models + spin mask + name). Wheel models
  are authored for the left (+X) side; right wheels are mirrored (scale.x = -1). Verified visually on the 911 and
  the Veyron.
- Vehicle IDs: `VEH_<id>_HI/LO.BNDL` uses the VehicleList entry ID (field 0x00). The GraphicsSpec resource ID
  is a GameChanger ID that also carries it (e.g. `0x0101000100108e4f` → 1085007).
- **Language**: `{u32 index, u32 count, ptr ids, ptr (offset,len) pairs}`, strings UTF-16LE.
  `UI/LANGUAGE/0001.BNDL` is English.
- **VehicleList**: 110 × 0x70-byte entries; 0x08/0x10 Language IDs, 0x6C year.
- **Shader parameters**: shaders store parameter names, hashes and defaults (0x14 / 0x1C / 0x20 / 0x24 /
  0x28), so material parameter hashes resolve to names (`PbrMaterialDiffuseColour`, `Tiling_Ratios_X`, ...).
- **ShaderProgramBuffer = DXBC** (SM4.0, with RDEF reflection, so names survive). Pixel programs are imported by
  the Shader; vertex programs sit behind VertexProgramState → ShaderProgramBuffer. `mwtool shader-asm <name>`.
  The renderer is deferred (4 G-buffer targets; albedo stored as `sqrt`).
- HLSL `TEXCOORDn` == VertexDescriptor `TexCoord(n)` (raw semantic 5+n).
- **DriveableSurface** albedo = `ColourMap(uv0) × DetailMap_Diffuse(world.xz × DetailMapUvScale) × 2`. Line,
  decal and paint variants are overlays.
- **TerrainPBR** albedo = Σ layer_i(uv1 × (Tiling_Ratios_X[i], Tiling_Ratios_Y[i])) × w_i × 2·mask.g, where
  `w.x = overlay(mask.r, noise)`, `w.z = overlay(mask.b, 1-noise)`, `w.y = 1 - mask.r - mask.b`, normalised;
  noise = `NoiseTexture(uv1 × Tiling_Ratio_Noise).g`.
- Vehicle bodies: `VehicleNFS13_BodyPaint` has no parameters (paint colour comes from elsewhere, likely player /
  GeneSys data); trim uses `VehicleNFS13_Body` with `PbrMaterialDiffuseColour`.
- GeneSys (started): `GenesysType` = reflected type. Header: `u8 2, u8 kind, u16 field count`, base-type import
  at 0x04, size at 0x0C, name hash at 0x10, name string near the end ("Genesys.Gen.NitrousParameters.TrafficNearMiss",
  "CgsCore.UniqueId", "uint16_t"...). Fields are 0x1C bytes: `{type import, type hash, 0, name hash, size,
  offset, u16 count, u16 align}`. Name hashes are not CRC32 of the full type name; the hash is still unknown.
- The `--screenshot` pose was being moved by gamepad stick drift; the camera controller is now disabled in
  screenshot mode.

### 2026-10-03: initial survey
- All bundles are `bnd2` v5, platform 1 (PC). Resource entries are 0x48 bytes (0x44 of data, padded for
  the u64). Every resource in the install decompresses to its declared size.
- `NFS13.exe` `.text` is encrypted (entropy 8.0); the entry point is in an appended DRM stub. See
  [reverse-engineering.md](reverse-engineering.md).
- Vertex attribute and texture formats are raw `DXGI_FORMAT` values.
- Vehicle/character positions: `R16G16B16A16_SNORM` × 10.0 (Porsche 911 = 4.49 × 1.97 × 1.30 m).
- Shaders import their VertexDescriptor at 0x9C (all 373 shaders).
- Material sampler `u16` keys match Shader sampler keys; shader sampler names identify the role.
- InstanceList matrices sometimes have `w = 1.0` in rows 0–2; ignore the w column.
- World: 169 `HAWAII/TRK_UNIT*.BNDL`, 35,424 InstanceList placements, ≈26M LOD0 triangles, 0 decode errors.
- Config bundles (`VEHICLELIST`, `GLOBALCONFIG`, `PHYSICSCONFIG`, `GAMEPLAY`) are almost entirely GeneSys
  types and objects.
- Audio: `.SPS` streams start with an EA SNR/SPS block header (`0x48`). Movies: `.VP6` with EA `MVhd`
  container.
- No signage is mirrored when the game's coordinates are used as-is (Y up, metres).
