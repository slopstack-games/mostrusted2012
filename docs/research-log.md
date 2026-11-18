# Research log

Dated findings and open questions about the game's formats and behaviour. Add new entries at the top. When a
question is answered, move the answer into the parser's doc comment and [formats.md](formats.md), and note
the date here.

Install used for verification: Steam build, `VERSION.TXT` = 1.4.0.0, 2,020 bundles.

Older entries (before 2026-11, and the early topical findings) are in
[research-log-archive.md](research-log-archive.md); move a month there once the next one has begun.

---

### 2026-11-18: event starts and jack spots run on the ported managers (`event_starts.rs`, `jackspots.rs`)

The two modules no longer hold their own rules. **event_starts.rs**: `fill_inputs` fills `EventStartInputs` (every
`OfflineEvent` as a `StartEvent` with start trigger = `SpawnPosition`, allowed cars, mode; the car id, speed,
throttle / brake, the player as reference point, pursuit = chase or cooldown, `GameLogicValues`); `apply_outcomes`
draws `EventStartState::shown`, shows the prompt when GMM+0x2c70 is `PromptEvent`, and in plain play turns
`state.chosen` (set by the manager's burnout hold, `triggered` exits the state in `world_logic::step_plain`) into
`PlayEvent`; an event the race code cannot run clears `awaiting_event`. Removed: the list build, the 10 m refresh, the
radius / speed / hold rules. **jackspots.rs**: `fill_inputs` fills `CarSwapInputs` (spots with `TriggerBox`es, owned /
store by bundle and DLC lock, parked reports from the `Parked` states, the player's car box (origin), the press, the
engine waits true: covered, exchange ready, sequence done; placing until the loader's car is there); `carry_out`
does `Spawn` / `Remove` (parked-car pipeline), `Exchange` (garage drive), `Place` (`garage.place_at`, left car's
pose), `Found`, `Finish` (the left car loaded again). Removed: the 300 m / 20 m standing rule, distances, prompt,
latch, immediate swap, reset of the last spot, car-change reset. Model decisions: the profile car given to the logic
is frozen during a swap (else its car-change reset rebuilds the slots on arrival); the player's colour is passed as a
flagged paint index; heat 0; no swap movie. See the commit for the test lines.
### 2026-11-18: the runtime's police run on `mw-police` (`police.rs`)

`police.rs` now drives `mw_police::Police::tick` from `police_tick` (FixedUpdate) and carries the decisions out in
`cop_drive`; `pursuit.rs`, `cop_sight.rs`, `cop_driver.rs`, `cop_patrol.rs` became re-exports / the traffic adapter
(wiring spec: decomp.md "Police and pursuit (ported)"). Removed from the runtime: the coordinator pass (count, kinds,
removal), role switches, low-speed / box-in, wanted and perception calls, the hand-written pursuit manager.
Crate additions: `hold_spawns`, `spawn_distance_override` (tests), `cancel_order`, `clear_orders`, `was_pursuit` pub.
Behaviour changes versus the old runtime (all from the crate): GoTo goes to the player's position (nearest cop to the
player), bust meter holds when the nearest chasing cop crashed, wrecked-by-AI only in a chase, chase start variants.
MODEL decisions kept: GoTo "arrived" = within 20 m of the point; contact box test; takedown = crash within 30 m;
spawn placement. Open: roadblocks (no roadblock props in the runtime, task output dropped), `wrecked_by_ai`, bust
camera event (bust sets state at once). Captures that would confirm: a free-roam pursuit with a GoTo cop in the
cooldown (which cop goes where), the heat step at a chase start.

### 2026-11-17: police and pursuit ported into `crates/mw-police` (engine-free; WIP, runtime not switched)

New crate `mw-police` (glam 0.32, wasm-clean, in `tools/wasm-check`): `pursuit` (pursuit manager `FUN_0066ae60`),
`sight` (perception / wanted, moved from `cop_sight.rs`), `coordinator` (sequencer, roadblock task, count, spawn
kinds, removal, roles, search legs), `driver` (moved from `cop_driver.rs`, plus the coordinator's state switch),
`patrol` (moved from `cop_patrol.rs`, traffic behind a `PatrolTraffic` trait), `score` (ScoringConfig slots),
`Police::tick` (inputs → events / spawns / removals / roadblock / roles / legs). 15 tests (scenarios: slow pass, spotted
→ chase, out of sight → cooldown → escaped, stopped → busted in 4 s, meter drains, heat rate and level-up, cap,
roadblock resumes cooldown, cop order, sequencer, roadblock trigger). `mw-assets::police::Behaviour` gained
`duration`, `roadblock_delay`, `roadblock_repeat`, `fallback_after`; `HeatLevel.cycle`.

Read in the code this session (new or corrected versus the runtime's `pursuit.rs` / `police.rs`):
- Chase start variants (`FUN_0066ae60` case 0): mode without cooldown → +0x54 at once; touching a roadblock car
  → +0x48; a chasing cop / takedown → spotter (`FUN_00514aa0`) none → +0x54, coordinator +0x448 → +0x48, spotter
  wrecked (+0x23e; ECX = spotter's player, disassembly 0x66b0b7) → +0x4c, else +0x50; cues `FUN_006f3f30` 0xc / 0xd.
  ScoringConfig offsets → actions mapped (score.rs): +0x2c 1289377, +0x58 604021, +0x90 / +0x94 sight lost /
  regained (`FUN_0065c740`), +0xc4 cooldown, +0xe4 / +0xe8 heat down / up, +0x1a8 escaped (1377906).
- `FUN_0065c880` (wrecked by AI → busted) acts only in a chase (runtime also busted in cooldown).
- Bust meter: no chasing cop → drains; nearest chasing cop crashed → **holds** (runtime drained). Candidates skip
  `FUN_006bed90` (+0x1f0 / +0x1f4, not identified). Bar range hysteresis + cue 1 after 3 s (0x00fb77ec);
  `FUN_006f0f00(2)` floor −0.9 (0x00fb8678). The bar at −1 posts camera event 0x1a only; state 2 comes from
  `FUN_00658140` (HSM action 0xf, link inferred; `Police::manual_bust`).
- Level-up message only for a 1-level step and not the first one when GMM +0x2e8d (`FUN_0065d080`); level's
  own action once per level (`FUN_0065c5e0`, bitmask rec +0x858); heat cap = highest *unlocked* row (`FUN_006673a0`).
- `FUN_00657ee0` identified: the target within 100 m of a **roadblock** (world +0x2f484 = roadblock manager, records
  0xff0, count +0x2050, max 2) → the cop "sees" (was MODEL false).
- GoTo (`FUN_0051a4e0` / `FUN_0051a3f0`): armed on the chase's rising edge (`FUN_00519040`), in the cooldown's join
  window (`FUN_00658160`: < 7 s, 0x00fb74ec, or level > 1) the cop **nearest the player** goes to the player's
  position (runtime: nearest to last-seen, to last-seen); Search when arrived (`FUN_004fbfa0`) or after `H`+0xc0.
- Sequencer `FUN_0051f010` / `FUN_00514e40` / `FUN_00514d90` (coordinator +0x6b4): main for `4fea2957` s, then
  cycle items, fallback after `f58d1d54` events (counter writer not traced), restart on a level change.
- Roadblocks: task `FUN_00526150` (first after `ae2d8f7d`, then `cbbeb3e0` after each), `FUN_005246f0`: route
  segments starting 300–600 m (`AIConfig` +0x1ac / +0x1a8) from the player, kind-0x12 triggers (`FUN_0051bd50`)
  whose bounding radius (`FUN_0042c0b0`: box size × 0.5 length → box size is full extents) touches the segment →
  `FUN_005226b0` (world message 7 → `FUN_006ab390` → `FUN_0083f030` layout, `FUN_00841040` add). Spike strips are
  roadblock props only; no helicopter logic found.
- Count `FUN_00526e30` re-read: per AIPlayerType, pending tasks + living uncrashed, ≤ 3 per pass (matches).

Open / where I stopped: decomp.md section "Police and pursuit (ported)" with the runtime wiring spec and the PLAN
item were **not written** (budget); the runtime still uses its own modules. Still MODEL: alert flag, ram strength,
hidden = no cop knows, the cooldown's stale `TimeTilEvaded` at entry (as the game), the box-in, spawn placement.
### 2026-11-17: AI racer driving ported into an engine-free crate (`crates/mw-ai`), runtime not switched yet

Every function of the racer's tick re-read in the dump and ported with its name and sourced constants
(`AIConfig` fields by offset / hash in `mw_ai::AiConfig`; `.rdata` / `.data` constants named in the doc comments).
API: `Driver::tick(cfg, path, RacerInput) -> TickOutput` (command block → `InputRecord` (`AIPlayer::FillInput`
0x006c0890, read from the disassembly) → `VehicleControls` = the +0x1bf0 / +0x1bf4 / +0x1c00 / +0x1c40 / +0x1c20
record). Ported: `FUN_004e75f0` (clear, state, reset timer +0x5854), state 8 `FUN_0050da90` (lane blend
`FUN_0050b000`, crash check `FUN_004fb9f0`, command builder `FUN_0050a740` / `FUN_00508d50`, look time
`FUN_00503880`, steering `FUN_00506c00` / `FUN_00505e60` / PID `FUN_00436920`, Hermite path `FUN_004e8190`,
`FUN_004e3ee0`, node step `FUN_00512b70`, nitrous `FUN_004ff620` / `FUN_004e4eb0` / `FUN_004fdb60`, racer
interaction `FUN_005050d0` with `FUN_00501860` / `FUN_004fda60` / `FUN_004fd940` and the state's LCG ring
(Enter 0x00506260), emergency brake `FUN_00508070`, pedals `FUN_004fa0e0`), state 11 `FUN_004eeba0` (Enter
0x004e0460, `FUN_004f9e80`, `FUN_004e3540`), path tangents / turns `FUN_004e6160` / `FUN_004e6a10`, the rubber
band `FUN_0063ef30` / `FUN_006432a0` / `FUN_006433f0` (time correction now ported) / `FUN_00642da0` / manager
`FUN_00645c90`. 21 unit tests on synthetic tracks (straight, right-hand corner, schedule braking, emergency
brake, recovery, reset, nitrous, shunt, rubber band); clippy and `tools/wasm-check` clean (crate added to it).

Differences from `ai_driver.rs` / `ai.rs` (found in the code): the recovery **starts in reverse** (Enter sets +0x34);
its probes are ±25° 2 m, forward 4 m, back 0.25 m (ours 6 m); only the back probe or a new stuck flips direction;
its steering is −4 × (left · direction to the next node) (ours 4 × the angle); its pedal is never negative and its
1..4 mph band ramps (ours 0); it exits above 5 mph on |v| with the heading within 45° and no car in the 8 m ahead.
The crash check adds the free time into the stuck time (a slow contact after > 1 s free triggers at once) and has
no route-end exclusion in state 8. The look-ahead distance runs from the current node, not the car; its cap is
`AIConfig` 0x1f0. The PIDs start with previous error 0 (a first-tick derivative kick). Nitrous memory is the
driver's own last-tick flag, not the car's burn state. `+0x68` is the player handle, not a race position: mode 2
holds 2 m **ahead** of a close faster-than-80-mph car with a lower handle (not "ram a better-placed car"). The
corner speed rule stays uncalled (no pointers to 0x004ff480 / 0x004ff550 / 0x004e5270 in the exe).

Open: runtime switch (spec in decomp.md "AI racers (ported)"); not ported: lane planner `FUN_00502e20`, racing line
`FUN_004e70e0`, obstacle sweep geometry `FUN_004ff270` (input), path check `FUN_00506d60`, `FUN_00661eb0` (route
factor, closure), virtual progress; +0x50 / +0x58 / +0x74 / flag bits meaning not traced; the corner test's toy
car cuts the 60 m arc by ~12 m (not checked against the real physics). Captures that would confirm: a live read of
D+0x5808 / pedals in a race, and a stuck AI's first recovery direction.
### 2026-11-18: logic Requests carried out by the runtime (traffic, music, UI sounds, camera snap; `logic.rs`)
Now carried out: `TrafficMessage` (kind 6 / 8 / 0xa -> `traffic::TrafficControl`: stop freezes the traffic tick and
spawns, reset removes every car once, start resumes), `TrafficDensity` (density when on, else 0, replaces the
`--traffic-density` scale in the density multiplier; `FUN_00845fb0`), `StopMusic` (song list stops, as
`StopSongList`), `StopSounds` kind 0 (`ui_sound::UiSounds::stop_all`), `CameraSnapBehind` (`camera::SnapCamera`,
the camera's own re-seat behind the car). MODEL decisions: the traffic messages' handlers (`FUN_0084e350` /
`FUN_0084e4d0` / `FUN_0084e550`) are unread, so stop = freeze is a stand-in; StopMusic is mapped onto the song
list rather than the music manager (`FUN_007908e0`).
Left unlogged-as-unhandled (no runtime system to carry them): CameraOverhead / CameraFinishLine / CameraFov (camera.rs
has no such modes), StopSounds kinds 2 / 4 (speech / gameplay stream have no separate voice groups), StopUnwantedSound,
ClearFeedbackQueues, RaceCarFlag (no invulnerable / damage-allowed switch per competitor in damage.rs),
RumbleEnable (no rumble), StartPhotoMode, ActivateSpotting, SwapGameMode, SetCarLocked (input lock lives in the
logic's mask only), WeaponsAllowed, PauseAi (ai.rs is another agent's).

### 2026-11-18: the handbrake ported from the code: the rear tyres' lock flag, the standstill hold, the steering range

**What was missing** (the 2026-11-17 reading below): `FUN_00905910` (the tyre inputs, after the driving update) sets
the tyre object's lock flag (+0x38 = constraint +0x2a8) on wheels 2 / 3 while the handbrake is on (virtual +0x44 =
vehicle +0x2a40, this tick's state). `FUN_009bc0c0` then takes the brake hold as FLT_MAX: the tyre holds the tread
against the chassis alone and never turns the wheel, so only the brake (max(handbrake strength, service)) slows
the spin and the rears lock. Standing: `FUN_009d6370` (standstill) launches only with the handbrake mask clear and
throttle ≥ 0.3 or brake ≥ 0.2 (`.bss` 0x01187230 / 0x01186ec0); otherwise the clutch stays open (the throttle
only revs the engine) and below 0.5 mph (0x01186f60) or with the handbrake it forces the control block's
handbrake mask on and sets the front lock lanes (gearbox +0x120 = +0x1b90, cleared each tick by `FUN_009da600`).
`FUN_009d17d0` (launch) drops back to standstill on the handbrake mask. `FUN_009d6170` releases the gearbox brake
pedal with the handbrake below 1 mph. `FUN_009d27b0`'s state machine writes a steering-input range (+0x50, virtual
+0x4c at 0x00920840) that `FUN_009d31f0` clamps the input to: [−1, 1], one side only for 1 s (0x011870f0) after a
pull that began moving forwards ends stopped / rolling back; idle again when the released turn's yaw < 0.1 rad/s
(0x01187070). `.bss` values read from the memory snapshot `re/out/live/snap1`.

**Port**: `mw_handling::handbrake` (state machine, tests), drivetrain standstill / launch gate and `parked`,
drive.rs (rear tyre lock flag, front lock lanes from last tick's `parked`, pedal release, steering clamp).

**Replays** (synthetic inputs in the worktree's `scratch/hb/`, 911, flat track): standing + handbrake + throttle:
before drove off on the fronts to 23 mph; now 0 mph, rpm 7800 (limiter), clutch 0, launches on release.
85 mph + handbrake: before rear ω = v / r; now rear ω 0 within ~0.4 s, κ −1, 89.8 → 45.8 mph in 2 s.
Tests before (unchanged code): accel 0–100 km/h 3.73 s, top 307.4; brake 38.1 m / 2.62 s; corner 2.57 g; drift
peak 37.3°, held 4.75 s; handbrake slip 1.1°, yaw 59°/s.

**Open**: tests run later the same day (handling.md; replays lack ref captures); decomp.md
"Handbrake and burnout" and handling.md not yet rewritten; model decision: the clutch opens when the handbrake
sends a launch back to standstill (the code leaves it); the engine-rpm launch condition (attribute +0x14 via
gearbox +0x180) not identified, taken as met; the crashed lock lanes (+0x1774) not ported; captures to confirm.

### 2026-11-18: HSM behaviours, batch of 4 (`hsm/behaviours_misc.rs`, second part)

Same method (string -> name stub -> factory -> vtable, slots from `text.asm`, `.bss` keys from snapshot 1 with a small
reader over `snap1/index.tsv`). Ported (4): `ROLLOUTACHVM` 0x77e720 / vtable 0xfdac50 (update 0x73ba10) and
`CARCOLCORRCT` 0x77e450 / 0xfdab0c (update 0x73c5e0): both only store the key at 0x117a970 (= `EXIT`) as the exit;
`HSM.STARTPHO` 0x77b150 / 0xfd935c (enter 0x740f20 `FUN_0083e710` on G+0x2f474, update 0x740f40 EXIT);
`HSM.INPUTDET` 0x77e1d0 / 0xfda9ec (update 0x73d0b0: local player's input-device record byte +0x499 set -> exit key
0x117a018 = `EXIT`). New Request: StartPhotoMode; Query: InputDeviceDetected.
Skipped, and why: `HSM.SELECTEA` (Easy Drive menu: keys EZ_RACES / EZ_MODS / EZ_CARS / EZ_M_WANTED / EZ_RECOMM /
EZ_MULTIPLAY / EZ_RETRY / EZ_LEAVE / EZ_SPEC_MOD by the UI state G+0x2f480 +0x33608; task says skip Easy Drive),
`HSM.ENABLEDE` / `HSM.CHECKFOR` (debug variable only: enter registers, exit 0x73c960 / 0x73cdc0 unregisters),
`DEBUGUNLOCKS`, `DBGSTARTFRID`, `DBGAUTOBIND`, `WRITETODATAS` (update 0x747310: key / type / value writes through
unnamed `FUN_009f8a00` helpers), `JOINEDLEFT` (enter 0x751c80 / update 0x75ef50: per-player join / leave bitmask,
`FUN_00745ff0` / `FUN_0075ce90` unread), `BINDCONTROLL` (update 0x741fd0 input polling `FUN_0047aab0`),
`HSM.UPLOADON` (Blaze job state +0x50), `RCDONEVNTWNR` (enter 0x74fa90, per-player event-result tables, update EXIT
0x73ec00), `ONLINECARSWA`, `CONSMUNLOCKS`, `MMPICONSONL`, `SHOWMEETUPPO`, `PLAYFEEDBACK`, `TELEPORTTOME`,
`CARJACKERACT`, `HSMSHAKEDOWN`, `HSMDODRIFTS`, `DOONEVTSTRT`, `DOSMSHNGRB`, `CHKPNTSONLIN`: 100+ instruction bodies
over unnamed engine functions, not read in full (no guessing).

### 2026-11-18: HSM behaviours, batch of 10 (`hsm/behaviours_misc.rs`)

Method as the earlier batches: name string address from strings.csv, the `push` / `mov edx` pair of the name stub gives
the factory, its vtable (slot order enter, enter update, running, update, exit update, exit begin, exit; default stubs
0x55cd90..0x55ce00), slots read from the dump or `text.asm`, `.bss` parameter keys decoded from snapshot 1.
Ported (10): `ACTIVATESPOT` 0x77f080 / vtable 0xfdb064 (enter 0x73afa0, exit 0x73afd0 -> `FUN_0063e2d0`), `CACHESCORE`
0x77e860 / 0xfdace0 (0x73b2b0, update EXIT 0x73b2e0), `EXITWHENGAME` 0x77ea30 / 0xfdad94 (enter 0x755ba0 `TIME` x 3e6,
update 0x74b8e0 compares with GMM+0x2d28 under GMM+0x2e74; the compared constant is `fldz`), `ACCT_INVITE` 0x77d440 /
0xfda3b4 (exit update 0x73af10, `FUN_006bad40`), `SWAPGAMEMODE` 0x77efe0 / 0xfdb01c (update 0x746100: `EventBinding`
value, else `Event`; `FUN_006e74f0`(GMM, id, 1)), `WRT_PR_SESSN` 0x77eef0 / 0xfdafb0 (enter 0x7704b0 + kind 0x76fcd0:
`IS_BOOL` / `IS_REF`; keys NAME, BOOL_VALUE, REF_VALUE decoded), `SYNCRESSTART` 0x77dec0 / 0xfda8a8 (enter 0x746210:
1.5e6 / 1.5e7 tick constants 0xfb8660 / 0xfb2894; update 0x741050), `ENDSETTINGSS` 0x77cd70 / 0xfda020 (exit 0x73c5c0
`FUN_0071c1c0`), `SHOULDSHOWCA` 0x77cc30 / 0xfd9f90 (update 0x7407c0: story mode -> FALSE else TRUE), `RUNCHALL`
0x77e400 / 0xfdaae8 (enter 0x74fc30 `FUN_00789240`, exit 0x73ed30 `FUN_00785d90`).
New Requests: ActivateSpotting, CachePlayerScore, AcceptInvite, SwapGameMode, WriteAutologBool, WriteAutologInt,
EndSettingsScreen, StartChallenge, StopChallenge; Queries GameModeClockRunning, GameModeElapsedTicks, ClockTicks,
ResultsSyncDelay. Not ported, open: the engine side of those Requests (the runtime ignores them); `TRYTOPLAYSPE`
(x19, 0x77cac0 / 0xfd9f00: speech-event state machine over `FUN_00ad82c0` / `FUN_00adaef0` and the speech system, read
only in part), `WAYPOINTS` (world drawing), `LAUNCH` / `RESETGAME` / `ENDGAME` (Blaze game helpers `FUN_004c27e0`,
`FUN_004c4b30`, ... unnamed), `PICKNEXTFDE`, `CARJACKERACT`, `KILLCAMSEQ`, `WAIT4ALLSYN`. Trivial left: `ROLLOUTACHVM`
(update EXIT 0x73ba10), `CARCOLCORRCT` (update EXIT 0x73c5e0).

### 2026-11-17: handbrake rear-wheel lock (user report), no capture yet, no model change

User report: "the handbrake doesn't lock up the rear wheels like the original; with brake-to-drift you can test".

**The original could not be captured.** Two launches (probe, save backed up to the worktree's
`ref/savebackup-2026-11-17-handbrake/`, md5 in `…handbrake.md5`, restored byte-identical after each, proxy DLL
removed, game quit): (1) the EA app updated itself on the first launch (13.827.2.6342) and then relaunched
`NFS13.exe` itself through Steam; `orig-wrap` had already consumed its request file, so that copy started on the
desktop display (`DISPLAY=:0`, no probe) and was killed within a minute. (2) Headless with the probe, title and
offline prompt passed, free roam reached (Raptor), but `mwtool car 911` (issued once while the world was still
loading on a first try, once ~20 s into free roam on a second) answered "old handle 0x0" and left the player
with no car: the probe's vehicle went to 0x0, its tick hook stopped (tick frozen at 1473 / 1568), the world
stayed carless. Open: why the swap finds no handle now (it worked 2026-10-19), and `orig-wrap` should keep the
request until `orig quit` (or a guard should kill a desktop `NFS13.exe`) so an EA-app relaunch stays headless.

**Code trace** (decomp.md "Handbrake and burnout", "Brake path in detail"): the handbrake budget is
max(strength, service × pedal²) per rear wheel in N m, the same units as the service brake; `FUN_009cdfd0`
applies min(budget, I |ω| / dt) to the spin at once (third argument 1/dt, checked in `FUN_00904da0`), keeps
the rest for the solver's brake step / tyre hold. No handbrake lock: the lock lanes are the crashed flag
(+0x1774) and the gearbox's engine-off hold of the front wheels (+0x1b90 = gearbox +0x120, `FUN_009ce3d0` /
`FUN_009cd620`). The task premise "no `Handbrake.*` speed band on the 911" is wrong: the block holds
`Brake_strength_at_low_speed` 350, `_at_high_speed` 1200, `Low_speed_mph` 30, `High_speed_mph` 135,
`Minimum_handbrake_time` 0.1 (hashes checked) and `Handbrake::new` reads them, so at 85 mph the strength is
≈ 800 N m, at 69 mph ≈ 670. The 911's rear tyre at its static load (4340 N in ours) and r 0.343 m carries
1.8 × N × r ≈ 2680 N m at the braking peak (κ −0.05) and 1.4 × N × r ≈ 2080 N m fully sliding: by the code as
read the rears cannot lock in a straight line. Also found, not ported: a per-wheel damage brake drag (wheel
+0x15c × the wheel-damage attribute +0x10, `FUN_009140b0`), and the engine-off front hold (check `ignition.rs`).

**Ours on the synthetic inputs** (spawn hodges-strip at 85 mph, `scratch/synth.py` turns the `mwtool inputs`
rows below into a 60 Hz replay CSV; outputs in the worktree's `ref/hb/*/telemetry.ours.csv`):
- straight (handbrake 0.5–2.5 s): 89.0 → 66 mph (≈ 5 m/s², rear handbrake ≈ 2.1–2.3 kN per wheel plus engine
  braking on the fronts, which take the whole drive with the handbrake on); rear ω = v / r throughout; pre-pass
  κ −0.05 at μ 1.80 (`MW_TYRE_DEBUG`): the wheel slips at the tyre's peak, it does not lock;
- turn (steer 0.5 with the handbrake): yaw ≤ 0.73 rad/s, rear slip angle ≤ 4.7°, body slip ≤ 3.4°, rear ω 2 %
  under v / r: the car barely rotates;
- brake-to-drift (brake 1 + steer 0.6 for 0.25 s, then throttle): no drift (rear slip ≤ 5.2° against
  `Slip_angle_to_enter_drift` 7°, the tap's floor `0dce38f7` 2° recovering at `b5290695` 10°/s), body slip ≤ 4.2°.
  If the original swings out here, the difference is in the drift entry / rear lateral grip, not a lock rule.

Tests (911, unchanged, no model change): accel 0–100 km/h 3.73 s, top 307.4 km/h; brake 100–0 38.1 m / 2.62 s;
corner 2.57 g; drift peak 37.3°, held 4.75 s, mean 32.3°; handbrake slip 1.1°, yaw 59°/s.

**The capture to take on an idle machine** (one agent; back up the save; wait until the probe's `tick`
advances in free roam before `mwtool car 911`): watch file = `ref/kerb.watch` up to `body_pz` plus, per wheel i,
the spin inertia `[vehicle +0xa08 + 0x1d0 i] + 0x54` and the brake object `vehicle +0xfb0 + 0x40 i` (+0x00
torque, +0x04 left over, +0x08 pedal², +0x10 lock lanes u32, +0x20 handbrake lanes u32), plus +0x1774 (u8),
+0x1b90 (u32), +0x2a20 (handbrake countdown), +0x1aa8 (hold input); then
`mwtool conditions --hour 13 --no-traffic --repair --nitrous 0` and
- `mwtool inputs ref/hb_straight/inputs.csv 0:0,1,0,0,0 0.5:0,0,0,1,0 2.5:0,0,0,0,0 3.5:0,0,0,0,0`
- `mwtool inputs ref/hb_turn/inputs.csv 0:0,1,0,0,0 0.5:0.5,0,0,1,0 2.5:0,0,0,0,0 3.5:0,0,0,0,0`
- `mwtool inputs ref/hb_bdrift/inputs.csv 0:0,1,0,0,0 0.5:0.6,0,1,0,0 0.75:0.6,1,0,0,0 2.75:0,0,0,0,0 3.5:0,0,0,0,0`
- each `mwtool capture <name> --probe --watch handbrake.watch --script ref/<name>/inputs.csv --spot hodges-strip
  --kmh 137`, then `mostrusted --drive 911 --replay ref/<name>/telemetry.csv --spawn=-2199.00,102.40,1192.90,139.50`
  and `mwtool compare`. Decisive columns: `w2_omega` / `w3_omega` against v / r, `b2_left` / `b3_left` (a
  locked wheel keeps budget left over), `b2_lock`, `w2_kappa`, `drifting`.

## Dated findings

### 2026-11-18: UI port (widget-tree renderer): survey only, stopped before code (budget)

Stopped at the survey (session budget); nothing in the runtime changed. What the next attempt can start from:
- **Widget types decide what draws.** The HUD screen 371621's layouts all sit in `Widget_ShowLayouts` widgets
  (`HUD`, `AdditionalHUD` 7 layouts, `MapOverlay`, `EngineOffHud`, `Prompt`, `EasyDrive`, the five `Nitrous`,
  `HackEasyDrive`, `CurrentRollout`, `EasyDrivePanel`); which of their layouts show is data: instance visibility
  (`Players.LocalPlayer.IsInStaticBust` on 581357), element visibility (`IsInChase` / `IsInCooldown`,
  `Mode.PromptString`, `CurrentRoad.Name`, `Recommends.*`, `Widget.Prompt.New`) and instance timelines
  (`Widget.EngineOffHud.*`, `Widget.EasyDrive.HideForMusic`). So a generic renderer draws every layout of every
  active group's widgets (`mw_assets::AssetStore::ui_screen_layouts` gives them with `widget_index`; `Screen`
  gives each widget's `group` and `layer`), in `LayoutLayer` order (0 below maps .. 6 overlay; order across
  groups not traced: model decision), with `UiScene::draw_with` and a lookup chain runtime feed (`hud.rs`
  `lookup`, `HudState`) → logic (`ui_sequence::logic_lookup`). The minimap quad (`Quad::map`) is the
  `Widget_PointOfInterest` / native `POIDisplayScript`: keep `hud.rs`'s mosaic + icons as its port.
- **Logic side.** `mw_logic::natives::pages::Pages` knows the contexts' pages and active groups but exposes only
  `current_page` / `active_groups`; the renderer needs (screen, group) in activation order: a small accessor
  plus one hook in `logic.rs` `tick` (after the HSM update) publishing it as a resource.
- **Feeds the logic lacks.** `Players.LocalPlayer.CurrentNitrousType` (the HSM's "Choose Nitrous") reads
  `players::Player::nitrous`, which the runtime never sets: set it from the fitted mod (`hud::NitrousType::of`'s
  slot rule) or the nitrous HUD always picks NONE.
- **Menu letterbox.** The flag `FUN_00794700` tests (UI manager +0x2ecf4) is read with +0x2ecf0 in
  `FUN_0061f8xx` (UI camera: +0x2ecf0 an override camera, else the default at +0x2eea0; getter per layer
  0x791480): the letterbox belongs to a UI camera override, not to a screen flag. No direct writer found in
  `text.asm` (written through another base). Until traced: a screen whose layout instances bind
  `UIConfig.HudAspectCorrection` is a HUD screen, else letterboxed (model decision).
- **The pause menu 431082**: `Menu` is a `Widget_MenuData` (layer 3, layouts 431094 / 1695971 / 1695911); its
  `Widget.Menu.Items` / `SelectedIndex` come from the menu widget's native code (filled from the group's
  `Data` / `StdData` MenuData widgets): that native has to be ported before pause.rs can go.
- **Blocker to decide (integrator)**: the logic runs only under `--hsm`; a HUD drawn only from the logic's
  groups disappears from plain / `--drive` runs unless `--hsm` becomes the default (`--drive 911 --hsm` was not
  run: `logic.rs` treats a run without the front end as booted, so it should reach free drive).
### 2026-11-17: the runtime's races run on the ported checkpoint manager (`world_logic.rs`, race.rs); event starts / jack spots not yet

Work in progress (stopped on the session budget). **Done**: `world_logic.rs` (`WorldLogic`, NonSend): the
`LogicState` the managers of `natives::{checkpoints, event_starts, car_swap}` run on in every mode: under `--hsm` the
logic host's (logic.rs hands it over in `receive`; `RuntimeHost::request` routes `Request::Checkpoint` / `EventStart`
/ `CarSwap` into `WorldLogic::outbox`), otherwise one owned there, on which `step_plain` issues the free drive's
"Do Event Starts" (node 816214's parameters read from the HSM data, `EventStartParams::read`, shared with the
behaviour) and "Car Swap Logic" requests; sets `Inputs` → `Step` → `Outcomes` around `LogicTick`. **race.rs**: the
hand-written gate overlap / lap / position / finish rules are gone; `fill_checkpoints` fills `CheckpointInputs` (route
entries with `TriggerBox::from_quat` gate boxes, the event's `CheckpointInfo` (new in `mw_assets::events::Event`),
car boxes, reset flag (crash / `AiCrash`), `FUN_00661da0` answer (`RaceState::past_target`), distances to the finish,
the race clock into GMM+0x2d28, the AI racers as PM slots 12 + grid slot, `GameMode` +0x6e markers, the marker keys);
plain play enters / places at go, steps per fixed tick, leaves on the finish (`checkpoints_plain`); `--hsm` fills
before `LogicTick` and reads after (`checkpoints_inputs` / `checkpoints_outcomes`); `apply_checkpoints` turns the
counters, lap, position, finish (`FUN_006b9d00` place, GMM+0x2e78) and `AiDone` into `RacerProgress`, messages and
results; the gate markers / map icons now draw the manager's `Marker` outcomes (`RaceState::markers`,
`gate_marker::shown_gates`). logic.rs no longer answers `CheckpointCount` / `Loaded` / `Finished` (the logic's own
answers); an event the logic's event starts chose gets its set-up from the runtime's `PlayEvent` without the Easy
Drive choice, and its `LoadEventData` waits for it. **Data**: `StartConfig::logic` (`GameLogicValues`): the
`CheckPointsConfig` / `MapPoiConfig` marker keys (+0x28 / +0x24 / Type 1–4 at +0x0c / +0x30 / +0x1c / +0x20; finish
+0x28 / +0x2c), +0x244, +0x284, +0x160 / +0x168 / +0x16c, +0x1ec, +0x8c, +0x104 (offsets from `mwtool genesys-types`).
mw-logic: `EventStartState::chosen`, `TriggerBox::from_quat` (+ test).

Model decisions kept / new: the car box centred half its height above the origin; "resetting" = the crash sequence
active / `AiCrash`; GMM+0x2e48, G+0x2f458 +0x85c, `FUN_00656220` reroute not read (false); AI finishing times and
places kept for the results board; the AI board order by the position relation; lap times derived from the counter.

**Open (next session)**: `event_starts.rs` still runs its own list / prompt / burnout rules: fill
`EventStartInputs` (events with start trigger = `SpawnPosition`, allowed lists, car id, speed, throttle / brake,
pursuit = chase only, `GameLogicValues`), draw `EventStartState::shown`, prompt from GMM+0x2c70, start on
`state.chosen` (plain: `PlayEvent` + `PlainFlow::awaiting_event`, cleared for unplayable events). `jackspots.rs`
still runs its own standing / prompt / immediate swap: fill `CarSwapInputs` (spots with `TriggerBox` location boxes,
owned = bundle and not DLC, store = DLC, parked reports from the runtime's `Parked` states, `started` = true as a
model, the engine waits true), carry out `Spawn` / `Remove` / `Exchange` (garage `Drive` + `place_at`) / `Place` /
`Finish` / `Found`; `inputs.rebuild` once when the spots arrive. Verification not finished: only
`--race 462174 --test race` was run after the change (see the commit); sprint 906146, `--hsm-event`, accel to do.

### 2026-11-17: the checkpoints, event starts and car swap run in the logic (`natives/{checkpoints,event_starts,car_swap}.rs`)

The per-frame work of `CHECKPTS`, `EVENTSTARTS` and `HSMCARSWAPLO` (until now one `Request` each) is ported from the
code into mw-logic, engine-free: the host runs it on the behaviours' requests (`LogicHost::apply`) and answers
`Query::CheckpointCount` / `CheckpointsLoaded` / `CheckpointsFinished` / `EventStartTriggered` / `CarSwapActive`
from it. What the runtime must fill each frame is a documented input struct per manager (`CheckpointInputs`,
`EventStartInputs`, `CarSwapInputs`: route entries and gate boxes, car boxes, distances to the finish, profile
answers, engine waits); the outcomes are logic state (counter, lap, position, finish, shown starts, prompt GMM+0x2c70,
slot states, swap phase) and `Request::Checkpoint` / `EventStart` / `CarSwap` for the engine. "In a trigger" is the
hit-list rule of `FUN_00536f70` over a box test (`natives/trigger_boxes.rs`). Sources: decomp.md "The checkpoint
manager in the logic", "Event starts", "Jack spots" (new paragraphs); the enters / exits not in the dump were read
from the snapshot's disassembly (`re/out/live/text.asm`: 0x749670, 0x75d8d0, 0x75d960, 0x754f30).
`mwtool hsm --run --event 462174` now needs no checkpoint answer: a synthetic lap (route from the event's gates, the
car driven straight at the logic's target gate at 60 m/s) passes 27/27 gates over 3 laps, `CHECKPTS` exits at
frame 4186, the outro plays and free drive resumes (sprint 906146: 10/10 at frame 1984). Tests: 8 new (the box rule, 3 gates in
order, a missed gate and a reset, position with an AI racer, which starts show, a burnout start, a parked car's
20–300 m rule, a swap's seven phases), 209 mw-logic tests pass.

Found on the way (corrections): the `CHECKPTS` score event (ScoringConfig +0x60) goes for every gate but the finish
while GameMode +0x6e is set (not "but the last two"); a jack spot's "CAR JACK USED" score is sent only for a swap
during a pursuit (`FUN_006c2410` on the player at 0x76239a, first time per spot) and with it `FUN_00639b60` on
G+0x2a890; position's start term is PM+0xa694 at enter (racers removed since count as ahead); the lead timer
(+0x48) is enabled by GMM+0x2e8b; `FUN_006e0520` returns 0 (constant); the event-start radius +0x1c0 is
`ACTIVERADIUS` squared; the free-drive machine runs a `CHECKPTS` too (no route: it exits at once).

Differences of the runtime's hand-written code (to fix when the runtime feeds the logic):
- **race.rs**: no reset advance (`FUN_00661da0`) for player or AI; mid-race position ignores racers removed since the
  start (+0x5c) and finished ones get their finish order; AI racers pass gates by the player's rules (sequence flag
  +0x12, no time in the original; ours keeps AI times); lap times instead of `eb1ecc26` splits; no lead events
  (took / lost / held 1.5 s), no gate score event, no GMM+0x2e7d; the finish does not wait for G+0x2f458 +0x85c ≠ 3;
  no trigger switching or GPS segment list from the counter (gps.rs builds its own); `TargetSpeed` / `TargetTime`
  in the won rule belong to `FUN_006f46e0` (`Query::ModeWon`), not to `CHECKPTS`.
- **event_starts.rs**: rebuilds the list from scratch (the original keeps entries and swap-removes, so the order
  and the 20 cap differ after a rebuild); ignores `RSPCTRADIUS`, GMM+0x2e83, the story-mode rebuild, the offline /
  free-drive / unlocked / available / intro filters and recommendations; adds the hold once per frame (the original
  per start in reach, only while the clock runs); counts the pursuit cooldown as a pursuit; starts the event at once
  (the original sets the next event, shows hint 0xe / 0xf / 0x11 / 0x12 and exits once UI manager +0x12568 is
  idle); no `TRACKEDONLY`, GMM+0x2e9c, swap gate GMM+0x252f or nearest-start prefetch; speed from |velocity|.
- **jackspots.rs**: origin in the location box (logic: box overlap); every car available (logic: `owned` / `store`
  answers); immediate swap (logic: the seven phases with their engine waits, input block 2 bit 7, GMM+0x252f);
  no parked-car spawn states, eligibility round robin, Lua `JackIt`, intro rules, pursuit / heat gate, nearest
  standing car, first-use effects (`FUN_0083c220`, heat bump) or spot counters (+0x74b0, +0xe6, +0x74c6).

Model decisions: the box test for the hit list (narrow phase not read); the prompt cleared at the start of each
logic frame (its clearer is not in the dump); the location query's distance is to the trigger position within
300 m (its sphere test against the shape not read); the mwtool driver's straight-line 60 m/s car and its race clock.
Open (runtime): fill the three input structs from race.rs / event_starts.rs / jackspots.rs and carry out the
`Checkpoint` / `EventStart` / `CarSwap` requests, then drop the runtime's own rules and its `CheckpointCount` /
`Loaded` / `Finished` answers in logic.rs; who sets `HSMCARSWAPLO` +0x74c8 / +0x74c4; `FUN_00656220`'s reroute rule;
the `GameLogicConfig` / `CheckpointConfig` values the tool and runtime should read instead of the documented
defaults.

### 2026-11-17: the last 7 HSM hooks (187 of 187) and 31 online HSM behaviour types (`hsm/behaviours_online.rs`)

Hooks (`natives/hooks_playlist.rs`, test `the_last_seven`; `lpcValue` hash 0xc6d9cf16 confirmed for `MutePlayer` from snapshot 1): HACKPlayFirstCustomPlaylist 0x676570 (`FUN_006f2730`(0), = `PlayCustomPlaylist` index 0);
RestartGamesComShowFloorFlow 0x680630 (`FUN_0067ff80`: a session object's +0x230 / +0x234 / +0x238 = 0 / -1 / 0, playlist manager byte +0x2e7f = 0, `FUN_006ce6f0` releases the player objects: `Request::RestartShowFloorFlow`);
ProcessPlaylistEndMilestones 0x676da0 (`FUN_00704b30` on G+0x2f418: ranking messages 0x1c / 0, 0x1c / 0xf for the last in the vote order, 0x12 / 0: `Request::ProcessPlaylistEndMilestones`);
GetMostWantedListFromServer 0x7eb4b0 (`FUN_0071dfe0` on the "CachedMWL" object: `Request::GetMostWantedList`); MutePlayer / UnmutePlayer 0x7eea30 / 0x7eea70 (`lpcValue` evaluated, index - 1, bit in the network layer's mask +0x80ec / +0x80f0: `Request::MutePlayer` / `UnmutePlayer`);
OnWallPostOrComment 0x7eb7d0 (`FUN_00639a90`, a bare `ret`: EXIT). All return EXIT. Model decisions: the mute string is the number itself (no binding expression), an index below 0 does nothing (the original writes out of the mask).
The "unported hook" example of the `mod.rs` / `host.rs` tests is gone (every hook is ported): they assert that none is left. Count: 187 of 187.
Behaviours (method as the earlier batches: factory = `mov edx` after the name's push, vtable from the factory, slots and keys from text.asm / snapshot 1): `mwtool hsm --codes` (new) lists the codes GAMEMODES.BNDL uses (199) with their uses and whether they are registered: 81 were unported, now 50.
Ported (31): UPLOADRESULT, UPLDCONCOURS, HSB_NET_CRE / JOI / LEA / QJO, CREATEPLAYGR, LEAVEPLAYGRO, DISCONNECTFR, "RECONNECT AL" (the data's code, 12 characters), INVITEFAILED, ISINVITEPEND, SEND_INVITE, JOINPARTYUID, DEL_PR_SESS, VALID_CNX_MP, SETFAILLOGIN, SETSUCCESSLO, BROWSER_IN, and from the same flows HAS_INV_ACCT, ENUM_ENTMNTS, SYSERROR, HOSTSETGAMEM, STOPMOVIE, ONLINESCORIN, SYNCGAMESTAT, UPLDCHALSTE, IsPRCtxtOpen, ONLNSCRACT, REBOOT, SYNCTIME (vtables, slots and messages: decomp.md "Behaviours ported, online flows").
Found: the request objects (+0x48; state +8: 0 idle / 1 pending / 2 done, error +0xc) are pumped by `FUN_004c4b30`; messages are `FUN_004c59d0` with an id (0x18 create playgroup, 0x19 / 0x1a join party by user / invite, 0x1c leave playgroup, 0x1d create game, 0x1e quick-join search, 0x20 / 0x38 / 0x39 join game / friend / invite, 0x21 leave game, 0x34 go to matchmaking, 0x37 disconnect, 0x45 concours); the join errors 45..51 map to NOT_EXIST / GAME_FULL / FAILED / INV_VER / NO_PRIV / FAILED / UNENTITLED; `VALID_CNX_MP` is a PC stub (always VALID); `SEND_INVITE` takes the id the hook `SetFriendInviteIdEasyDrive` stored (G+0x2f498); `FUN_00047a380` (the local player's net id) is `xor eax,eax` on the PC.
Offline behaviour (query defaults: no connection, no session, request objects done with an error): `HSB_NET_CRE` / `HSB_NET_QJO` never leave their enter update (the network layer is never ready), `UPLOADRESULT` / `LEAVEPLAYGRO` / `HSB_NET_LEA` / `DISCONNECTFR` finish at once, `HSB_NET_JOI` / `JOINPARTYUID` exit FAILED, `SYNCGAMESTAT` / `REBOOT` / `ONLINESCORIN` hold.
Requests (appended): NetSend { slot, message: NetMessage } (+ NetReset / NetAck / NetPump / NetCancel), UploadResultXp, ClearConcoursPending, ApplyConcoursResult, RestoreUiMessages, SetGameWish, ClearCreateGameDesired, ClearQuickJoinDesired, PublishQuickJoinInfo, ClearJoinWish, ClearInvite, ClearPartyWish, RefreshPartyUi, AutologReconnect, ClearFriendInviteId, SendInvite, DeleteSessionEntry, SetLoginState, BrowserParse, BrowserCommandDone, EnumerateEntitlements, HostSetGameMode, StopMovie, SyncGameState, UploadChallengeState, ApplyChallengeResult, OnlineScoreActionsUpdate, PostQueueMessage; Queries: NetBlazeUp, NetReady, NetInGame, NetLobbyA / B, NetRequestState / Error, ConcoursValue / Synced, InviteAccepted / Sent, JoinWish, PartyInvite, FriendInviteId, QuickJoinInfo, MatchState, QuickJoinSessionFlag, BrowserBlocked / Command, SessionHost, and the rest (behaviours.rs). One test per type at the end of `flow_tests.rs` (31).
Model decisions: the message payloads the original builds from game objects (the race result of UPLOADRESULT, the quick-join search, the ids of the join wishes) are the engine's: `NetMessage` carries what the handler decides (message id, visibility, bring-playgroup, concours value); the quick-join info publication, `BROWSER_IN`'s argument parsing (`FUN_0076f460`) and the three-state UPLDCONCOURS refresh are single Requests; `QuickJoinInfo` default "absent"; query names for the invite object's vtable slots (+0 pending / +4 accepted / +0x10 sent) are inferred from their uses; the REFRESHENT note "`FUN_004c2050` = not up" is probably inverted (it is "state 0xb", authenticated).
Verified: `mwtool hsm --run --steps 600 --answer 1222788=DONE` (boot -> "Freedrive Update" + "Freedrive HUD") and `--event 462174` (race route) still run, `tools/wasm-check` ok, clippy 0 warnings for mw-logic / mwtool, 232 mw-logic tests.
Open: the runtime carries out none of the new Requests; 50 used codes remain unported (`mwtool hsm --codes`): the event / checkpoint / car-swap types (agent C: SWAPGAMEMODE, CHKPNTSONLIN, DOONEVTSTRT, TELEPORTTOME, ...), TRYTOPLAYSPE (19 uses), the Easy Drive / car-select / debug ones (DBGAUTOBIND, DBGSTARTFRID, DEBUGUNLOCKS, SHOWCARSELEC, STARTCARSELE, PICKNEXTFDE, ...) and WRT_PR_SESSN / WRITETODATAS / ACCT_INVITE / JOINEDLEFT / WAIT4ALLSYN / SYNCRESSTART / ONLINECARSWA / MMPICONSONL / LAUNCH / ENDGAME / RESETGAME / EXITWHENGAME.
### 2026-11-17: one UI sequence player; the logic's sequences drawn, the game recap shows (`ui_sequence.rs`)

Under `--hsm` the logic's own sequences (conductors' `StartSequence` other than the title, `RUNGAMELOGIC`
`PlaySequence` other than the event's intro / outro) ran headless. Now they play through a general player,
crates/mostrusted/src/ui_sequence.rs: the logic keeps each playback's clock (its jumps are what the HSM waits on) and
hands the player (key, sequence, clock) every frame (`UiSequencePlayer::show`, after `logic::LogicTick`); the
player loads the sequence's layouts / textures / fonts on a thread (as `frontend.rs` does), steps conducted children
on their own clocks with their jump controllers, and plays: layouts (`UiScene::draw_with`, letterboxed, GlobalZIndex
19 under the front end's 20), black quads, movies, wave items (`music::SequenceWaves::player`, mixed with the front
end's), ICE camera takes on the gameplay camera (`camera_take::CameraTake::camera` with the player's car as subject,
blended back by the take's `GAME_CAM_BLEND`, as cutscene.rs), HideHud (`HudSettings::hidden_by_sequence` and the
logic's `UISequences.HideHudRequests`). Bindings (items' `enabled`, conducted jumps, layout elements) resolve through
the logic's binding host (`LogicState`'s roots, registered as `ui_sequence::LogicView`): the UI binding's path and
`[argument]` are re-joined, parsed with `mw_logic::binding` and looked up; a path that does not resolve is logged once.
logic.rs's `step_playbacks` now also sets / clears `GeneralTriggerSequenceItem`s of its playbacks
(`set_general_trigger` / `clear_general_trigger` when the clock enters / leaves the item).

**The recap 1689073** (data, `mwtool frontend --sequence 1689073`, which now names item kinds and camera takes):
the ICE take 1689072 "Game" (20.35 s, `Player` space, no bindings: shots around the player's car, lens 22–60 mm,
`GAME_CAM_BLEND` 0 → 1 over 13.68–20.35 s), HideHud 0–18.9 s, layout 1689155 (the player name strip and logo,
`Players.LocalPlayer.Name`), 1689182 (statistics: speed points `ScoringViewModel.CurrentSpeedPoints`, events and
Most Wanted completed `Mode.*`, completion per pack `Mode.GameCompletePercentage[n]` with the DLC packs'
`EntitlementManager.IsPdlcNPurchasable`, `Milestone.Value[...]`, time played, the Most Wanted list `SequenceMwl`),
the conducted 1532921 (layout 1532919) and 275950 at 15.9 s (the car's mods: `Rollouts.SelectedRollout.Data.*`),
the music bed `01000000001b0a3e` (type 4, 0–21.4 s) and the stingers of 275950. In the runtime (continued profile,
`--front-end --hsm`): the recap starts when the world is loaded (`PlaySequence` at HSM frame ~600–1100), under the
boot transition's last part; the camera circles the car, the strips and statistics panels slide in and out on
their timelines, the HUD is hidden until 18.9 s, then the camera blends to the chase camera and free roam goes on.
Screenshots (git-ignored): `scratch/recap/1689073-{01,04,08,12,16,20}s.png`, `scratch/fe/*.png`. The waves play
(`MW_MUSIC=1 MW_MUSIC_LOG=1`: the bed from 0.21 s, the 275950 stingers at 15.9 / 17.4 s).

Model decisions: as the other two players, the layouts draw in the letterboxed 1280 × 720 frame with its bars, a
movie fills that frame under the layouts, `UISequences.Params[n].TextN` reads "", unported shaders' elements are left
out; the conducted children's clocks run on real time (capped at 0.1 s per frame as `frontend.rs`). Not played:
`SetUICamera`, `AdapterHost`, `ImagePaletteSequenceItem`, the bus mixer channels (waves at their own gain),
`RaceCarEffectSequenceItem` (9.8–20.9 s), `ListenerSequenceItem`, the recap's `EventTrigger` at 15.9 s, streamed
images, the take's depth of field and motion blur.

Not moved: `frontend.rs` (title, boot transition) and `event_card.rs` keep their own players. Both hand state out of
their sequences that the general player does not model (the title's "BOOT" trigger and key press, the transition's
and the card's picture slots and tips, the card's monitor jump, race start and "TRANSITION" hand-over); moving them is
not low-risk and was left.

Open:
- The recap's statistics are empty: `Players.LocalPlayer.Name`, `ScoringViewModel.CurrentSpeedPoints`,
  `Mode.GameCompletePercentage[n]`, `Mode.SPToNextBlacklist`, `Mode.UniqueEventsComplete`, `Mode.BlackListComplete`,
  `Milestone.Value[...]`, `DriverDetails.General.TotalTimePlayed`, `SequenceMwl.*` (and `#index` in its list),
  `Players.LocalPlayer.GetVehicleInfo.ShortName`, `Rollouts.SelectedRollout.*` do not resolve in the logic yet.
- Captures that would confirm: the original's recap on a continue (when it starts relative to the transition, the
  camera framing at 1 / 8 / 12 s, the statistics shown, the music level against the bus mixer items).

### 2026-11-18: vehicle bank skid / impact voices against the code: reset on a new vehicle ported (unfinished)

Task: how the original drives the vehicle bank's skid and impact voices, and what it resets.
- **Skids** (`FUN_009d1bf0`, already ported in `vehicle_driver.rs`): the three intensities are the only inputs the
  skid sequences follow. Their reset is `FUN_009d1120` (from `FUN_009d83e0` <- `FUN_009d91a0` <- the vehicle's
  construction `FUN_00904260`): +0xd0 / +0xd4 / +0xd8 = 1.0 (`.rdata` 0x01026060..6c, read), so a new vehicle's
  skids start full and fall at their fall rates (0.1 / 0.2 / 0.5 per tick). Ours started at 0 and kept the old
  car's smoothed state across a car change. Ported: `VehicleDriver::reset` (start state 1.0), called by
  `sfx.rs` when the player car entity changes. A teleport / placement is not a construction: no reset (the
  original's `FUN_009d1120` is not on that path as far as traced).
- **Hardening (model)**: the smoothing lerp keeps a NaN for good; non-finite wheel inputs / dt are now zeroed
  (the audio thread already zeroes a non-finite smoothed value, which would be silence, not the rail).
- **Impacts**: in the original they are one-shot sequences started from `FUN_0057e920` / `FUN_0057caf0`
  (decomp.md "Traffic horns, crashes and skids"), not bindings of the vehicle bank; ours are `player_sfx.rs`
  one-shots (`VehicleDriver::impact` is only used by mwtool). So no lasting impact state exists in the bank.
- Not found: a state in the vehicle bank that survives a teleport / pause. The rail source is still open (the
  engine's sub-harmoniser is the remaining suspect, see the entry below).

### 2026-11-17: skid "loud and distorted for good" after a race start: not reproduced, underruns ruled out (unfinished)

Real-time headless runs recorded from the null sink (`scratch/live.sh` in the worktree, `MW_INPUT_SCRIPT`,
`--hsm-event 462174 --auto-start 1`, free roam on the flat area `--spawn=-2197,103.5,1194,140`), with the new
`MW_AUDIO_METER` line for the player's vehicle bank and `MW_AUDIO_STALL` (commands.md).
- **The user's recording** (36 s, 44.1 kHz, master 0.35): the loud bursts are one centred source pinned at its
  soft limiter (rails at ±0.21 = tanh × channel 0.6 × master 0.35) and flipping sign every 2–30 ms (a "telegraph"
  wave: the sign of a signal far over full scale): that is where the sub-25 Hz share and the DC wander come from.
  Up to 61 % of a second's samples sit above 0.18. Ours never: at most 21 %, and periodic (the engine at full
  throttle, which renders at −5 dBFS after its limiter in the tap); the vehicle bank's level before its limiter
  peaks at 0.4–1.5 in slides (3.6 once in a crash).
- **Underruns ruled out as a lasting cause**: three 150 ms audio-thread holds at the race start's first teleport
  (`MW_AUDIO_STALL=teleport,150,3,0.3`) give three real `Buffer underrun` errors, and the race after it measures the
  same as without (vehicle bank −22 to −23 dBFS at 70 mph, the mix within 1 dB, no rail). Our runs also underrun
  without a load burst: race2 logged 7 in a race grinding against AI cars (8 AI engine voices at ~4 % audio-thread
  cpu each, the vehicle bank 10–19 %): the audio thread is near its budget in races.
- No lasting state found in the vehicle bank (sfx_player smoothing guards non-finite values, ModValue clamps, the
  bank's Distortion channel `…73483` receives nothing) or in its driver (`VehicleDriver::approach` is stable).
- Open: the source of the rail. Next: a slide before and after the race start at the same speed (the script needs a
  clock from the race's go; `MW_INPUT_SCRIPT_START` only knows `playing`), and the engine bank's state (its
  SubHarmoniser toggles a square every 2 zero crossings, a telegraph by construction) after the teleport; the
  audio-thread budget in races (AI voice cost is ~4 %, not the 0.1–0.3 % noted before).

### 2026-11-17: skids louder after a pause, very loud at a race start: two causes fixed

User reports: skids get louder at a pause and stay that loud after it; very loud and distorted at a race start.
- **Pause** (`settings.rs` `pause_world_sound`): the world's sounds are driven by parameters written on the game
  clock, but their sources play in real time; while the clock was paused (pause menu, map, the logic's sim pause at
  a race start) they played on with frozen values (a slide's skid sequences retriggering) and what built up stayed.
  Now every engine / effects sink but the menus' (`MenuSound`: the UI sounds) pauses with `Time<Virtual>` and goes
  on where it was. Live A/B from the null sink (slide, pause 4 s, resume; `MW_PAUSE_WORLD_SOUND=0` = old): old
  plays on through the pause at −24 dBFS, new is silent and resumes at the pre-pause level. Model decision: the
  original's sim pause (`FUN_00845c60`) is taken to stop the world's sound; its receiver was not traced.
- **Race start** (`cutscene.rs` teleports): the intro's `TeleportPlayerSequenceItem`s set the body to 55–70 mph but
  left the wheels' spin, so the tyres slid until they caught up: the Spin skid at full intensity. Now the game's
  placement of a moving car (`Car::place_moving`, `FUN_008f8190`: wheels at road speed, gearbox placed) and on the
  ground (`FUN_00647e00`). `--race 462174 --test race`: spin in the first 4 s 0.07 (was ~1.0), vehicle channel
  −22.7 dBFS (was −17.6), 27/27 gates. Open: the landing input reads 1.96 m/s in the teleport frame (the wheels'
  contact velocity over the jump): should ignore the teleport.

### 2026-11-17: ninth 20 HSM hooks verified and enabled (180 of 187; `natives/hooks_playlist.rs`)

Handlers read from `re/out/live/text.asm`, callees from the dump. All return EXIT except `CanJoinAirportGameByFriendIndexBinding`
(TRUE 0x1177fe0 / FALSE 0x1177ee0, equal to the table's copies in the snapshot). New member hash: `liPlaylistIndex` = 0x25a408da (`DAT_01177b10`, found by name guess with the field hash).
Mapping: LoadLastOnlineRollout 0x693c50 (`FUN_00692800` on G+0x2f404: Request); UploadLocalPlayerXP 0x676370 (local player +0x3fc > 0 -> stat upload `FUN_00734f60`(type 4), +0x3fc = 0 either way; `Player::online_xp`);
ProcessOnlinePostEventDrivingTests 0x680560 (online event and the race state's byte +0x71 -> `FUN_00704bf0` G+0x2f418: ranking messages 6/7, 6/8);
SetupPlaylistsVote 0x676430 (`FUN_006f60f0` on the playlist manager (G+0x2f3fc)+0x14); PlayCriterionPlaylist 0x676450 (`FUN_006e9a30` = a current playlist with state; if none `FUN_006f6040`);
PlayCustomPlaylist 0x676480 (`liPlaylistIndex` -> `FUN_006f2730` = `FUN_006eae00`: creates the playlist session when +0x3c == -1); EZPlayCustomPlaylist 0x676520 (`atoi(lpcValue)` - 1, same call);
HostCancelPlaylist / SoloPlayerCancelPlaylist 0x6764c0 / 0x6764f0 (`FUN_006f5f00`(0 / 1): needs +0x8c == -1, a current playlist and `FUN_004c22b0`); MatchmakePlaygroup 0x6765c0 (`FUN_006747d0`: network request);
QuickJoinGameDesired 0x6765e0 / CreateGameDesired 0x676610 (G+0x2f490 word / G+0x2f708 byte = 1, then `FUN_006e74f0`(0, 0) on (G+0x2f3fc): wished game cleared, +0x2d10/14 = `DAT_01179528/2c` = 0 in the snapshot);
SetRolloutSweetRevengeFromEasyDrive 0x68d330 (`atoi` -> (G+0x2f404)+PM+0xa6b4 x 0xd8 + 0x82b8, then `FUN_0068ba20`(0)); SetFriendInviteIdEasyDrive 0x67a800 (`FUN_006797f0`: the string evaluated by the binding expression parser `FUN_00a50650`, `strtoll` -> G+0x2f498);
SetJoinFriendIdByIndexBinding 0x676640 (`FUN_006749a0`: the string evaluated to an index, -1, in the friend list G+0x2f3e0 (`FUN_0086f2f0`, 0x68-byte entries) -> G+0x2f4ac = 1, id +0x38/+0x3c stored at +0x2f4b0 and +0x2f4b8);
CanJoinAirportGameByFriendIndexBinding 0x676680 (`FUN_006748d0`: `FUN_00643c90` = airport pack owned -> TRUE, else friend entry byte +0x63 == 0 -> TRUE, else FALSE); OnMidGameJoin 0x6766d0 (`FUN_006e0c10`: byte +0x84 = 0, +4 = G+0x2a9e0);
Activate / DeactivateFriendCarouselViewModel 0x676cc0 / 0x676ce0 ((G+0x2f3e0)+0x14 word = 1 / 0); UnlockMwlCache 0x676df0 (`FUN_0071cad0`: lock byte +0x36 = 0, the inverse of CacheMwlAndLock). Test `the_ninth_twenty`; `the_twenty_ported_hooks` counts 180; `mod.rs` / `host.rs` tests use `MutePlayer` as the unported example.
Model decisions: the hooks that take an expression string (SetFriendInviteId, the two friend-index ones) read it as the number itself (no binding expression is evaluated); the cancel hooks are guarded only by "a playlist is current" (`HookPlaylistFields::playlist_active`), the other two guards are the runtime's;
`SetRolloutSweetRevenge` with a local player outside slots 0..12 stores nothing (the original writes out of range); the friend list is empty offline, so the friend hooks answer FALSE / do nothing.
Open: the runtime handles none of the new `Request`s (LoadLastOnlineRollout, UploadLocalPlayerXp, ProcessOnlineDrivingTests, SetupPlaylistsVote, PlayCriterionPlaylist, PlayCustomPlaylist, CancelPlaylist, MatchmakePlaygroup, ApplyLocalRollout); the `FUN_00a50650` expression evaluation is not checked against a live binding.
Remaining unported (7): HACKPlayFirstCustomPlaylist (0x676570, debug), RestartGamesComShowFloorFlow (0x680630, debug), ProcessPlaylistEndMilestones (0x676da0), GetMostWantedListFromServer (0x7eb4b0), MutePlayer (0x7eea30), UnmutePlayer (0x7eea70), OnWallPostOrComment (0x7eb7d0).

### 2026-11-17: 10 race HUD / pause menu / Autolog behaviour types (`hsm/behaviours_racehud.rs`; 120 types)

Survey: `mwtool hsm --run --steps 3000 --event 462174` (race) and with `--pause-at 300` enter no unported type at all
(every behaviour code in both traces is registered; `HSMPAUSEINTR` is ported since the dialog batch). The static walk
(`mwtool hsm <id> --depth 30` of 730003, the HUD machine 604141, the pause menu 1135398 / 1135406) finds 29
unported codes, all online / Autolog / dialog branches; the ten ported are the ones with offline logic. Method as
the earlier batches, with one correction to the notes: the factory is the `mov edx` that follows the name's push in
the registration table (0x777xxx..0x778xxx), not a function before it. Types (factory -> vtable): `DWNLDPROG`
0x77e8f0 -> 0xfdad04, `RFRSHALMANIF` 0x77ed10 -> 0xfdaed8, `RUNSTOREENUM` 0x77d530 -> 0xfda420, `PAUSEAUTOLOG`
0x77ec70 -> 0xfdae90, `SHOWPENDINGO` 0x77e940 -> 0xfdad28, `TAKEPPIC` 0x77be70 -> 0xfd9944, `SHOWVITASYST` 0x779d40 ->
0xfd8f30, `REFRESHENT` 0x77d800 -> 0xfda590, `SET_GAME_ERR` 0x779cf0 -> 0xfd8f0c, `GAMEERROR` 0x77ceb0 -> 0xfda0c0
(slots and effects: decomp.md "Behaviours ported, race HUD / pause menu / Autolog").
Found: `SET_GAME_ERR`'s parameter key `DAT_0117a378` is "ERROR STRING" (a NUL-padded space; the dump shows only
`ERROR`); `TAKEPPIC` / `SHOWVITASYST` do nothing on the PC but exit `EXIT` (TAKEPPIC's exit unregisters a named
handler at G+0xe8); `DWNLDPROG` exits through `FUN_00774dc0` (only taken when the parent handles the exit point).
Requests (new, appended): DownloadProgressionBegin / Apply / Finish, AutologRefreshManifest, StoreEnumerate,
AutologPause, ClearPendingDialog, UnregisterHandler, RefreshEntitlements, PostGameMessage, ShowGameError. Queries
(default): DownloadProgState (0), DownloadProgValid (false), DownloadProgBlocked (false), DownloadProgReady
(false), AutologManifestIdle (true), AutologBusy (false), PendingDialogId (0), GameErrorHold (false). One test per
type at the end of `flow_tests.rs`.
Model decisions: offline `DWNLDPROG` fails at once (`DownloadProgValid` default false; the real answer needs
`FUN_007daf20`'s network layer, not measured); `GameErrorHold` (the clock object's word +0xc4c, meaning unknown)
defaults to 0; `SET_GAME_ERR` / `GAMEERROR` messages have no consumer in the runtime yet; the runtime does not
act on any new Request (the Autolog / store / entitlement objects do not exist there). Verified: free roam ->
race -> free roam still runs through the logic (`--event 462174`), `tools/wasm-check` ok.
Next unported types in these trees (online): `UPLOADRESULT`, `UPLDCONCOURS`, `HSB_NET_CRE / JOI / LEA / QJO`,
`CREATEPLAYGR`, `LEAVEPLAYGRO`, `DISCONNECTFR`, `RECONNECT`, `INVITEFAILED`, `ISINVITEPEND`, `SEND_INVITE`,
`JOINPARTYUID`, `DEL_PR_SESS`, `VALID_CNX_MP`, `SETFAILLOGIN`, `SETSUCCESSLO`, `BROWSER_IN`.
### 2026-11-17: body shops respray the same car in place (`paint_wear::Respray`, `materials::repaint`)

- **What the game does** (decomp.md "Body shops", `bodyshop.rs` doc): `FUN_00748ff0` posts vehicle-manager message
  0xc (`FUN_0058bc60`: next palette colour; `FUN_0058edd0` loads its values into the render instance's paint block)
  for the same vehicle; `FUN_008e9c00` writes that block into the paint shaders' per-object constants every frame.
  Nothing is rebuilt. Liveries are the meshes' own `DiffuseTextureSampler` (research log 2026-10-06), so a respray
  changes only the paint constants on livery cars too.
- **Ours before**: the respray sent `GarageAction::Drive` and the loader rebuilt the car (its body meshes are baked
  per paint, `loader.rs` `paint_key`): a new entity, the nitrous tank carried over by hand, the race progress
  stashed in `race::attach_player`; with `--units 12` the reloaded car found no ground and the race test stalled
  after gate 3 (shop 1496594).
- **Now**: `CarSpec::paints` carries all of the car's `VehiclePaint`s; the car entity gets `paint_wear::CarPaints`.
  `GarageAction::Respray { paint_index }` (body shops; the car menu when it picks another paint of the car being
  driven) sets `Garage::current.paint_index` and puts `paint_wear::Respray(i)` on the player's car; `paint_wear`
  writes the paint into the car's own copies of its body-paint materials (`materials::repaint`: the constants
  `Look::with_paint` writes, p0, p1, p2.y, p3.rgb, p10..p13, p19.rgb, p20.xy, quantised the same way, plus the
  base colour and clear coat; p9.xy, the dust / dirt levels, stay and are rewritten by `write_materials` with the
  new paint's `DustColour.w` / `DirtColour.w`). Body-paint parts outside the body's `CarPaint` (wheels, if any)
  get their own copy first so other cars sharing the model are untouched. `CrashVfxCar::paint_index` follows.
  The race progress stash is gone (the entity stays). `--test bodyshop` checks the same entity and now holds the
  handbrake once stopped (the brake alone reversed the car out of the shot).
- **Verified**: `--race 462174 --test race --units 12` 27/27 (three resprays in place), without `--units` 27/27;
  `--test bodyshop` PASSED with the 911 (plain paint, red → blue) and the F-150 SVT Raptor (livery bundle,
  black → silver), same entity, repaired, washed, nitrous 100; 911 accel unchanged (3.58 s, 11.60 s).
- **Model decision**: the respray is applied in the frame after the pass (garage message → component); the game's
  message-queue timing was not read.
- **Open**: `tools/t` lost some stdout lines of `--test bodyshop` (the `before:` and verdict lines) in two runs;
  `tools/hl ... > out 2> err` shows them all (the two FIFO `cat`s of `tools/hl` writing one log?). Capture that
  would confirm: a body-shop pass in the original (`mwtool capture`) to see whether anything else (a fade, a
  sound) goes with the colour change.
### 2026-11-17: the logic's load requests carried out under `--hsm` (`logic_loads.rs`)

The boot's load waits passed in the frame they were asked because the queries' defaults say "loaded". Now the
runtime carries out the loads and answers from them (crates/mostrusted/src/logic_loads.rs, logic.rs module doc):
- `LoadFeedbackGroup` / `UnloadFeedbackGroup` (`LOAD_FDBK_GR` → `FUN_007982e0` / `FUN_00798370`): the group's
  bundle `EN_US/FEEDBACKGROUPS/<id>.BNDL` read and its `FeedbackMessageGroup` decoded on a thread, counted;
  `FeedbackGroupLoaded` (`FUN_00792590`) true once read while counted. Front-end run: 97039, 97037, 457707,
  604180, 1560226 (the recap's group, "Wait for Recap FMG" waits on it) each 1–3 frames.
- `PrefetchScreen` / `ReleasePrefetchedScreen`: the screen's bundle `UI/SCREENS2/<id>.BNDL`; 371621, 431082 1 frame.
- `RunLayout` / `DeactivateLayout` / `ReleaseLayout`: the layout decoded (`AssetStore::ui_layout`), `LayoutReady`
  once decoded (100143 "AlwaysAvailable Notifications", 1 frame); not drawn.
- `LoadProfile`: our profile file and `settings.cfg` read on a thread (`ProfileLoaded` after 1 frame, `ProfileReady`
  while no load runs); `ApplyProfileSettings` applies the read volumes (`--volume` on top) to `AudioSettings`.
- `PlaySequence` of anything but the event's intro / outro runs headless like a conductor's sequence (its clock
  with the jump controllers): `SequencePlaying` / `SequencePlayerBusy` while it runs, `StopPlayback` ends it. The
  continue's "Run Recap Sequence" 1560255 (`CalculateShouldShowRecapSequence` sets GMM+0x2e88 on PC) plays the game
  recap 1689073 (24 s, player 2) and now waits for it: frames 572 → 2013 in the race run; the free-drive states after
  it ("Freedrive Update", the song list, the Easy Drive prompt) start 24 s later than before. The recap's camera,
  layouts and music are not drawn / played (open).
- `RemovePlayers` / `CreateLocalPlayer` (`HSB_P1BIND`): the logic's player slots emptied / the local slot filled
  (`LocalPlayerReady`). Not the car entity: our car spawns only with the world (~15–30 s after the press), the
  original's player record exists at once and the boot transition would hang on it.
- `LoadWorld`, `LoadAll`, `AutoSelectWorld`: mapped to the runtime's loader, which streams the one world from
  start-up without a gate (`Mode.WorldLoaded` was already `FrontEnd::ready`). Open: the original starts the world
  load at `LOADWORLD` (after the key press); gating ours on it would move the world loading behind the transition.
- `LoadEventData` was already done (race link).

Who loads the groups the HSM waits for without loading them (97039 at 286011 "Wait For Required Resources" before
1560411 loads it, 97037 at 457754, 457707 at 457757, 1560603 at "Wait for Recap FMG")? Not the HSM data (only
`LOAD_FDBK_GR` 1560411 names 97039; none names 97037 / 457707); `FUN_007982e0` has no caller in the dump (its HSM
caller 0x743ee0 is outside it) and no group id is a code constant. `UIConfig` 0x172ab's
`LocalisedFeedbackMessageGroupList` (field 0x2c69cde5, id 2083002) lists 1560603, 1909255, 1909289, 457707,
1532857, 1560675: likely the engine's own preloads. Model decision: a group the logic never asked for keeps the
default answer (loaded). Also model decisions: a failed read counts as loaded (logged); nothing leaves the asset
store's cache at count 0; `CreateLocalPlayer` is immediate.

Fix on the way: `--hsm-event` is dropped when the runtime already chose an event (in one race-test run the car's
burnout at the event-start marker chose 462174 during the recap; the pending `--hsm-event` then started the race a
second time after it).

Captures that would confirm: the original's frame count from the key press to "Freedrive Update" on a continue
(the recap's 24 s), and whether the recap shows on every continue.

### 2026-11-17: eighth 20 HSM hooks verified and enabled (160 of 187; `natives/hooks_online.rs`)

Handlers read from `re/out/live/text.asm`, callees from the dump; member hashes read from the snapshot `.bss`
(`mem.bin` + `index.tsv`: 0x1177acc = `lpcValue`). All return EXIT unless noted. Mapping:
PopThankYou 0x676840 (`FUN_0063cdb0` on the entitlement manager G+0x29280: `ThankYouCount` +0x9d4 - 1, queue shifts, first entry released);
StopAutologAlert 0x676860 (`FUN_0071d190`: +0xd0 = 0, `FUN_00a43150` on the handle +0x2e8 if set);
SetDlcMessageIndicator 0x677110 (int member 0x298c2ce3 -> `FUN_0063a500`: profile value 0x11 and the "last_played" row; connected to `Entitlements::dlc_message_indicator`);
OpenStoreOverlay 0x677150 (`FUN_00655fa0` on G+0x2f438: platform overlay call, +0x10 = 1 on success);
RemoveOnlineSeenUnlocksMenu 0x676d70 (one unused member lookup, then `FUN_006f44e0` = ClearOnlineEasyDrive's request);
RefreshTopOfSpeedwallViewModel 0x6768f0 (`FUN_007357f0`(0) on +0x6c8c); ResetPlaylistScoreState 0x676ac0 (`FUN_006e32a0` playlist scores, `FUN_006bf460` clears byte +0x44b of 12 player records);
InvalidateKinectSelectedClass 0x676d00 ((G+0x2f404)+0x8f9c = -1); IsOriginIgoAvailable 0x7eb760 (platform value for key 2, hash compared with the hash of "true";
returns copies of TRUE 0x117f048 / FALSE 0x117ef88, checked equal to the table's in the snapshot; the EXIT copy of the 0x7eb... handlers is 0x117efa0, also equal);
SignInStarted / SilentSignInStarted / SignInFinished 0x7eb520 / 0x7eb550 / 0x7eb580 (`FUN_00881f10`(1) / (0) and `FUN_00886f00`: the Lua natives' code, now `lua_menus::start_sign_in(flag)` / `finish_sign_in`);
SetTargetRecommendedEvent 0x675bb0 (`FUN_00725130`(`strtol(lpcValue)`, 0, 0) on (G+0x2f454)+0x5a3c: needs the "Recommends" document and "HasRecommendations"; TRUE / FALSE);
SetCurrentRecommendedEvent 0x675c20 (`FUN_00726c80`: +0x9c = +0xa0, `FUN_00725130`(+0xa0, +0xa4, 0));
SetCarListForRecommendedEvent 0x675c50 (index > 0 and `FUN_00723300`: `Items[n].EventId` -> `FUN_00871330`; TRUE / FALSE);
SelectRecommendationDomByIndexString 0x675d10 (`FUN_00724ef0`(index, 0): nothing without recommendations or for -1);
SelectRecommendationDomByEventIdString 0x675d70 (`FUN_00723b10` finds the item by "EventId" -> `FUN_00724ef0`, else `FUN_0071d270` releases the three DOM objects);
RefreshRecommendations 0x676810 (`FUN_007355b0`(0, 0)); and the two unported offline / online hooks taken from `mwtool natives --unported`:
GenerateRecomendationPostEventString 0x676890 (`FUN_0071d040`: +0x2e4 = flag (G+0x2f3fc)+0x2ca5 and `+0x1974 < +0x1978`) and
CacheMwlAndLock 0x676dc0 (`FUN_0071caa0`: publishes "CachedMWL" as "OldCachedMWL" via `FUN_00820840`, lock byte +0x36 = 1). Test `the_eighth_twenty`; `the_twenty_ported_hooks` counts 160.
Model decisions: the DLC indicator member name is unknown (hash 0x298c2ce3; ~2000 name combinations tried), so the test cannot set it; offline there is no
"Recommends" document, so the recommendation hooks do nothing and answer FALSE where they return it (`HookOnlineFields::has_recommendations`, `event_ids` are filled by the runtime);
`FUN_006e0440` (special event 0x1d21f2 lock) is called without ecx set in the handler, so its object is unknown: a field, false; `FUN_006e5ea0` (event already running) is a set the runtime fills;
PopThankYou does nothing at a count of 0 (the original would underflow). `mod.rs` / `host.rs` tests now use LoadLastOnlineRollout as the "unported" example.
Open: `FUN_00723b10` hands `FUN_00724ef0` the item position counted from 1 (the counter is incremented before the store) while `Items[%d]` elsewhere looks 0-based: check against a live "Recommends" document;
the runtime handles none of the new `Request`s (SetRecommendedEvent, SetCarListForEvent, SelectRecommendationDom, ReleaseRecommendationDom, RefreshRecommendations, RefreshTopOfSpeedwall, ResetPlaylistScoreState, PopThankYou, StopAutologAlert, OpenStoreOverlay, CacheMwl).
Next 20 candidates (unported, online first by table order): LoadLastOnlineRollout (0x693c50), UploadLocalPlayerXP (0x676370), ProcessOnlinePostEventDrivingTests (0x680560),
SetupPlaylistsVote (0x676430), PlayCriterionPlaylist (0x676450), PlayCustomPlaylist (0x676480), HostCancelPlaylist (0x6764c0), SoloPlayerCancelPlaylist (0x6764f0),
EZPlayCustomPlaylist (0x676520), MatchmakePlaygroup (0x6765c0), QuickJoinGameDesired (0x6765e0), CreateGameDesired (0x676610), SetRolloutSweetRevengeFromEasyDrive (0x68d330),
SetFriendInviteIdEasyDrive (0x67a800), SetJoinFriendIdByIndexBinding (0x676640), CanJoinAirportGameByFriendIndexBinding (0x676680), OnMidGameJoin (0x6766d0),
ActivateFriendCarouselViewModel (0x676cc0, writes OM+0x14 = 1: trivial), DeactivateFriendCarouselViewModel (0x676ce0, = 0), UnlockMwlCache (0x676df0, +0x36 = 0: trivial).

### 2026-11-17: the sign-in family, WRITESESSION and LOADSTRSET (`hsm/behaviours_signin.rs`; 110 types)

`HSM_SERV_OP` (33 uses), `SIGNIN_CNTRL` (11), `SI_EDT_EMPT` (6), `SI_SHOW_ERR` / `SI_ADD_CNTRL` (3 each), `HSM_KEYB_ENT` (2), `SI_FIRST_PRT`, `WRITESESSION` (47), `LOADSTRSET` (5).
Method as the earlier batches: factories 0x77ca70 / 0x77c720 / 0x77c8b0 / 0x77fbf0 / 0x77fb60 / 0x77c7c0 / 0x77ca00 / 0x77d5d0 / 0x77f9f0 (the function before each name stub's `mov edx`),
vtables 0xfd9edc / 0xfd9d48 / 0xfd9dfc / 0xfdc100 / 0xfdc0cc / 0xfd9d90 / 0xfd9eb8 / 0xfda484 / 0xfdbe3c, bodies from `text.asm` (the dump lacks most of them; REA's Ghidra provider is unavailable here:
Ghidra 12.2 installed, 12.1.x wanted), keys from snapshot 1 (OPERATION, DATACONTEXT, ENTER_NEXT / PREV, EXIT_NEXT / PREV, ISPASSWORD / ISEMAIL / ISPRODKEY / ISGAMERTAG, VARNAME / VARVALUE,
NO_PERSONA 0x117a4e0, CREATE 0x117a468, PC_LOGIN 0x117a7a8, SHOW_TOS 0x117a178, WRONG_PWD 0x117a0c8, ENTITLE_GAM 0x117a1c0).
`HSM_SERV_OP` is a 26-entry table at 0xfd5208 (20 B: name, name string, start function +8, error handler +0xc, success handler +0x10; `Connect`, `Login`, `PCLogin` ... `OnlinePassKillSwitch`);
enter 0x750120 finds the `OPERATION` (unknown = state 26, byte +0x90 = 0 so the first update exits `FAILURE`), update 0x73f3c0 pumps the request object (+0x50; +0x58 state 2 = done, +0x5c error),
runs the error handler of the requested op or the success handler of the *current* state (+0x88), exit `SUCCESS` (+0x48 default) once the state is 26. The login starts (0x750210 / 0x7502d0) send `Connect`
(0x7501a0, state 0) when the network layer (`*0x010edb58`) is not up; its success handler 0x7400b0 then starts the requested op. Error handlers: generic 0x73f4d0 (0x11 `NO_PERSONA`, else show the error, `FAILURE`),
logins 0x73f620 (0x10 CREATE, 0x11 NO_PERSONA, 0xe PC_LOGIN, 0x13 SHOW_TOS, 0x15 WRONG_PWD, 0x16 ENTITLE_GAM, else `FAILURE`); the other eleven handlers are answered like the generic one.
Other types: `SIGNIN_CNTRL` (shows / hides the form control `DATACONTEXT` on `G+0x2f3e0 +0x10c8`; leaves when one of the control's two binding expressions is false: `ENTER_PREV` -> `EXIT_PREV`, else `EXIT_NEXT`),
`SI_EDT_EMPT` (edit text empty -> `TRUE` / `FALSE`), `HSM_KEYB_ENT` (on-screen keyboard: flags 4 password > 2 e-mail > 0x20 product key > 8 gamertag; result 3 accepted, 2 cancelled; `EXIT`),
`SI_FIRST_PRT` (`FUN_004c2030` is a constant true on the PC: `SUCCESS` at once), `SI_ADD_CNTRL` / `SI_SHOW_ERR` (string-set load of the lists, form / error screen built from the parameters, removed on exit;
the control building and the error screen's button focus are one Request each), `WRITESESSION` (enter sets the session variable, update exits `EXIT` when the parent handles it, `FUN_00774dc0`),
`LOADSTRSET` (loads `UI\LANGUAGE\<language>\<STRINGSET>.BNDL` in the enter update, unloads in the exit update; update 0x73dde0 = 0).
Requests: ServerOpStart / Ack / Cancel / Report / Handle / TermsRequired, WriteSession, LoadStringSet / ActivateStringSet / UnloadStringSet, SignInFormActive / EditEmptyFlag / LoadStrings / UnloadStrings /
Build / Remove, OpenKeyboard, KeyboardStore, CloseKeyboard, SignInErrorShow / Frame / Close. Queries (default): ServerOpState (2 done), ServerOpError (1), ServerOpStarted (true), ServerOpNetReady (false),
ServerOpNextState (26), StringSetBusy (false), SignInControlActive / Visible (true), SignInEditEmpty (true), FirstPartySignedIn (true), KeyboardResult (0), SignInFormStringSets (0), SignInFormBusy (false),
SignInErrorChoice (0). One test per type at the end of `flow_tests.rs`.
Result: `mwtool hsm --run --steps 600 --answer 1222788=DONE` (no `IsChatRestricted` answer any more) runs the silent sign-in: `PCLogin` sends `Connect`, fails with the generic error, `FAILURE`; then
`CancelLogin` fails the same way and the boot reaches "Freedrive Update" and "Freedrive HUD". The runtime's hand-written FAILURE stub for `HSM_SERV_OP` in `logic.rs` is not needed any more (the type is ported, so
its registry skips it).
Model decisions: offline error code 1 (a "no server" error; the real code of a dead Blaze server is unmeasured, every handler maps unknown codes to `FAILURE` anyway); the success handlers of states other than
`Connect` are `Request::ServerOpHandle` + `Query::ServerOpNextState` (online only); after the exit update's acknowledge the request object counts as idle; which of `SIGNIN_CNTRL`'s two expressions is "active"
and which "visible" is not settled (names are a guess, both are binding expressions at control +0xc).
Open: the engine side of all new Requests / Queries (the sign-in form object G+0x2f3e0 +0x10c8, the keyboard, the session store) is unported; slot 9 of `HSM_KEYB_ENT` (0x772880, form `FLOW`) and `SI_SHOW_ERR`'s
enter-time layer-set save (+0xfc / +0x100) are read but not ported; the remaining 11 `HSM_SERV_OP` error handlers and all success handlers.
### 2026-11-17: the `Players`, `Event`, `Transitions`, `GameLogicState` binding roots and `Mode.DisablePauseMenu`

Done like the earlier roots (`natives/players_view.rs`, `event_view.rs`, `transitions_view.rs`, `game_logic_view.rs`;
one `ROOTS` line each; tests per root). The race route no longer needs the driver's `Players`, `Transitions`,
`Event.Data.EventIntro` and `GameLogicState` answers.

**`Players`** (`FUN_006d4350` registers "PlayerManager", 0xcc60 B; the root is `FUN_006d3c30`'s `FUN_00a4d2a0`;
player records are the types "PlayerBase" / "Player", 230 members, 0x7f0 B inline from PM+0x10). Ported: the
manager's `LocalPlayer` / `GetLocalCompetitor` (0x6dbef0), `CameraTarget` (0x6dbf20), `Player[n]` / `GetCompetitor` /
`PlayerBase` / `AIPlayer` (0x6dbe60 / 0x6dbe30 / 0x6dbec0), `IsPlayerActive`, `Racer[n]` / `Cop[n]` (lists at +0xa664 /
+0xa630), `RacerCount`, `CopCount`, `NumNonLocalPlayers` (+0xa6a4 - 1), `NumNonMidGameJoinPlayers` (0x6bef60),
`NumTeamPlayers`, `NumOpponents`, `HasAnyPlayerFinishedCurrentEvent`; and the record members the HSM reads:
`ResetInProgress` (0x6bc410: byte +0x2c1, or the world's per-vehicle flag G+0x25694[vehicle]), `Wrecked` /
`Crashed` / `IsActive` (+0x23e / +0x23f / +0x23d), `WasBusted` (+0x434), `IsTeleporting` (+0x2c8), `IsFallingOffCliff`
(+0x2f6), `IsInBuilding` (+0x285), `IsMidGameJoining` (+0x474), `IsEngineOn` (`FUN_006d8ab0`), `IsInWaitForZone`
(0x6d8c20), `IsWreckSuicide` (0x6d8b90), `HasChaseStarted` (0x6db740), `CurrentNitrousType` (`FUN_006bde60`: NONE /
BURN / DUMP / JUMP and the `_PRO` names from the class +0x84 and kind +0x80 of the vehicle record), `PlayerIndex`,
`RaceCarIndex`, `LocalPlayer`, `IsCop`, `GetVehicleInfo.ID` / `.GamePackID`. State the engine owns is a documented
`Player` field the runtime sets (reset pending, engine on, in wait-for-zone, chase started, nitrous, wrecked, ...).
Not ported (they need physics, pursuit or rollout): speeds, RPM, heat, chase / position / score members,
transforms, `SelectedRollout`; they do not resolve (a published "Players" definition still answers). A missing
local player is a null object (`ResetInProgress(NOT)` then reads false-NOT = true, like any failed binding).
`Mode.LeaveGameRequested` now reads the local player's `IsTeleporting` (+0x2c8) as the code does.
Model decisions: racers / cops are the AI-driven (+0x9c = 2) non-cop / cop slots in slot order (the original keeps
explicit lists); `NumNonLocalPlayers` counts the occupied slots - 1.

**`Event`** (`FUN_006dfbc0` / `FUN_006e6bf0`, type "EventDataManager" 0x7ec B, the GMM's two event data managers
swap on a load): registered fields `Data` (.rdata "Data", +0xf4, the event's GeneSys object), `GameMode` (+0xec),
`HasDescription2` (+0xe9). `Event.Data.<field>` is the current event's GeneSys field by hash (`mw_assets::Events::
event_fields` -> `EventInfo::data`, filled by `mwtool hsm` and the runtime like `Mode.Data`), `Event.GameMode.<field>`
the current mode's record; `Id`, `Name2`, `IsRecommends`, `CustomItems`, `speedwall.Items`, ... are published data
definitions and fall through. Model decisions: the loaded event is the GMM's current event (+0x2ce0);
`HasDescription2` false. **Finding:** the old driver kept `Event.Data.EventIntro` set after the race, so the return
to free roam wrongly played an intro; with the root it is 0 again ("Does Event Intro Exist" FALSE) as in the data.
The checkpoint count / finished answers (`Query.CheckpointCount` / `CheckpointsFinished`) belong to the checkpoint
manager (G+0x2f47c), which is not a binding root; the driver still stands in for it.

**`Transitions`** is not a `FUN_00a4d2a0` root: the screen-transition manager (G+0x2f458 +0x4b0) answers through its
property getter (factory 0x7edbd0), which compares the property hash with globals the static initialisers fill (the
code is beyond the text dump; found in the memory snapshot at file offset 0xb67261). `Transitioning` = state +0x3ac ==
3; `Image1` / `Image2` (+0x394 / +0x398), `Tip[n]` (+0x374 + (n & 7) * 4), `AllowInput` (+0x3c5). The state machine is
`FUN_007f4760` (0 idle, 1 loading, 2 ready, 3 playing, 4 finishing; the same word is `Query::TransitionState`).
The runtime owns it: `LogicState::transitions.set_playing(bool)` while its transition plays (the runtime's own
`Transitions.Transitioning(NOT)` override in `mostrusted/src/logic.rs` can go, agent C's file); idle (offline tools) =
not transitioning.

**`GameLogicState`** (`FUN_006a38c0`): `IsAccessingProfile` (0x6ad680: `(+0x4941 && +0x4935) || +0x4942` of G+0x2f454;
`LogicState::profile_io`), `HasSaveGameDeviceBeenUnmounted` (always false), `IsControlGroupEnabled["NAME"]` (0x6b1770:
the input manager's group table, built in its constructor: CAR_ONLY {1}, MENU {2}, DIALOG {3}, DEBUG {4},
UNIVERSAL {5}, EASYDRIVE {6}, CAR {1, 6}; enabled when every group's block count byte G+0x2f0c0+g is 0, i.e.
[`InputBlocks::counts`]). The pause test no longer needs an answer.

**`Mode.DisablePauseMenu`**: `FUN_006fe520` registers it as the byte at manager +0x252b ("Is it safe to open the pause
menu?", VM +0x250b, cleared by the VM constructor; no other writer in the text dump: the engine sets it);
`GameModeManager::disable_pause_menu`. The same function registers more fields (`TargetTime`, `Speedwall*`, medals,
`PersonalBest`, ...) not ported yet.

**Result.** `mwtool hsm --run --steps 1500 --event 462174 ...` (free roam -> race -> free roam) needs
`SignInState.Params.IsChatRestricted`, `1222788=DONE`, `Query.CheckpointsFinished` and `1560270=FALSE` only; the
driver's local player now has a running car (`engine_on`, `vehicle_alive`: a stand-in for the engine).
Captures that would confirm: a live read of the local player's +0x2c1 around a respawn and of the transition
manager's +0x3ac during an event intro.

### 2026-11-17: 9 dialog / pause / photo HSM behaviour types (`hsm/behaviours_dialog.rs`; 94 types)

`SHOWDIALOG` (73 uses), `HSMPAUSEINTR`, `SUPPRESSPREF`, `RUNPRCONTEXT`, `TAKEPHOTO`, `PHOTO_LISTEN` / `PHOTO_COMPL` / `PHOTO_INPUT`, `LIMITEDEDSP`.
Method as the earlier batches: factory = the `mov edx` of the name stub (0x77df10, 0x77ea80, 0x77f170, 0x77edb0, 0x77d4e0, 0x77b1a0 / 1f0 / 240,
0x7800c0), vtable from the factory (0xfda8cc, 0xfdadb8, 0xfdb0d0, 0xfdaf20, 0xfda3fc, 0xfd9380 / 93a4 / 93c8, 0xfdc234), slot bodies from `text.asm`
(the dump lacks several), keys from snapshot 1 (POS_EXIT 0x117a6e0, NEG_EXIT 0x117a0d8, DIALOG 0x117a4c8, DEF_PG_NAME 0x117ae90, SUSPEND 0x117ae88,
RESTOREGRP 0x117ae60). `SHOWDIALOG` runs on the same `Dialogs` state as the Lua natives `RequestDialog` / `GetDialogResult` / `ClearDialog`:
`LogicHost::request` applies `ShowDialog` / `CloseDialog` (filling `last`) / `TakeDialogResult` to `LogicState.screen.dialogs`; `Query::DialogResult`
(0xff = none) / `DialogOpen` read it; `Dialogs::set_result` (the runtime) answers it.
`SUPPRESSPREF`: enter 0x740f70, enter update 0x740f90 (byte +0x6dc clear), exit 0x740fb0 (object G+0x2f458 +0x1940, 0x7edab0 / 0x7edb10).
`HSMPAUSEINTR`: idle watches the UI manager's timeline for an item of type 0x5f6d2 (0x74d040), active holds > 15 frames until released.
`RUNPRCONTEXT` = `RUN_PG_RNNR2`'s sibling on the context manager G+0x2ed74 with 0x83b580 / 0x83b590 / 0x83b5a0 / 0x83b5b0 / 0x83d630 (own Requests / Queries).
`LIMITEDEDSP` (0x75aff0, 0x762650, jump tables 0x7629a0 / 0x7629bc): seven states over four entitlements (0x2045b9, 0x2195ac / ad / ae) after the first
check 0x1721a; offline the enter update never passes (player byte +0x42f), and with nothing owned the update walks 0 -> 3 -> 6 four times to `SUCCESS`.
Requests: TakeDialogResult, PauseInterruptStart / Begin / End, SuppressPrefs, ReleasePrefs, StartPhotoCapture, PhotoInputBegin / End, PostPhotoSnapEvent,
PushPrContext, LeavePrContext, LimitedEdSend. Queries (default): DialogResult (0xff), DialogOpen (false), PauseInterruptTriggered (false),
PauseInterruptReleased (true), PrefsSuppressPending (false), PhotoCaptured (true), PhotoIdle (true), PhotoButtonHeld (false), PhotoTriggered (false),
PhotoMode (0), PrContextExists (true), PrContextExit (0), PrContextBusy (false), LimitedEdReady (false), EntitlementOwned (false; `LogicHost` answers it
from `Entitlements::has`, so the DLC lock holds), LimitedEdPlayerFlag / PlayerBit (false), LimitedEdRequest (0 = pending). One test per type at the end of `flow_tests.rs`.
Result: `mwtool hsm --run --steps 600` (free roam command) still reaches "Freedrive Update" and "Freedrive HUD".
Model decisions: the pause interrupt's UI-state scan (timeline items, `FUN_00827400`) and release test (`FUN_005470e0` + screen state) are one Query each;
the photo telemetry events ("GAME / EVNT / SNAP" and the photo-input one, `FUN_00872590`) are one Request each (the runtime posts only for a valid player
vehicle); the photo object's states are two bool Queries (6 captured, 1 idle), default true so an engine without photo mode never stalls; the
limited-edition server request is `LimitedEdSend` plus the poll Query.
Open: the runtime answers none of the new Requests / Queries except the dialog and entitlement ones; photo mode itself and the pause-interrupt timeline
items are unported; the sign-in family is the bulk of the remaining machines.
### 2026-11-17: races run through the logic in the runtime (`--hsm`, logic.rs, `--hsm-event`)

Under `--hsm` a race now goes the logic's route (decomp.md "Route free roam → race") and race.rs / event_card.rs /
cutscene.rs run "driven" (logic.rs module doc lists what is answered and carried out). Without `--hsm` nothing
changes (`RaceDriven` absent, `CutsceneState::logic_driven` false).
- **Event choice**: any runtime start (map START RACE, Easy Drive, pause menu, world markers: `PlayEvent`) is the
  logic's request through the same native path as `mwtool hsm --event`: the free-roam prompt's input wait 908987
  exits `EZ_RIGHT`, "Easy Drive Menu" 1135029 calls `StartEventFromEasyDrive` (`lpcValue`) and exits `TO_EVENT`
  (the runtime's menus stand in for the logic's Easy Drive widgets). `--hsm-event <id>` chooses once 908987 waits.
- **Event change**: `LoadEventData` is ready when the runtime holds the event's `RaceSetup` (free drive at once).
- **Race**: `RequestRespawnAtEventStart` (`Respawn` of the event) → `race::StartRace` (car held at the start until
  resident; `Players.LocalPlayer.ResetInProgress` follows `RaceState::placing`); `Mode.HasIntroLoadCompleted` once
  placed with the opponents; the event card is the chosen menu's screen transition: `Transitions.Transitioning`
  while it shows, its "TRANSITION" item sets the logic's general trigger, so "Screen Transition Voodoo" waits for
  it and `RUNGAMELOGIC` of `Event.Data.EventIntro` plays the intro at the same moment as the hand-written flow did;
  `Query::SequencePlaying` = the cutscene plays; `CHECKPTS` = race.rs's gates (`CheckpointCount` /
  `CheckpointsLoaded` / `CheckpointsFinished`); the outro on `RUNGAMELOGIC` of `EventOutro`, its playback held
  through the results; `StartGameMode` of free drive after "On Event End" → race.rs ends the event.
- Found on the way: without the card as a transition, Voodoo's first child (`Transitions.Transitioning(NOT)`)
  ended it at once and the intro was requested 7 s before the card's trigger.

Verified: `tools/t --hsm-event 462174 --auto-start 1 --test race --mute`: `hsm:` log free roam (machines 1 / 2) →
`START_EVENT` → `LoadEventData 462174` → machine 3 at 730003 → `Respawn` → `PlaySequence 1439074` (intro) → 27/27
gates → `PlaySequence 1689795` (outro) → machine 3 finished → `StartGameMode 390703` → "event over, back to free
roam" → machines 5 / 6 (free drive again); `TEST RESULT race ... passed=27` and `TEST RESULT race after
event_over=true`. `MW_HSM_SHOTS=<dir>` saves card / intro / race / outro / free-roam screenshots.
`--race 462174 --test race --units 12` (no `--hsm`) stalls after gate 3 on master too: the kinematic test car
passes body shop 1496594, the respray reloads the car, and with only 12 units loaded there is no ground collision
under it, so the new car is never placed (0 player cars). Without `--units` (streamed world) the same route passes.
Model decisions: listed in logic.rs ("Race stand-ins"): intro load = placed with opponents; "TRANSITION" kept set
until the event ends; `WasBusted` false; the outro's playback held through the hand-written results. Open: the
logic's own Easy Drive / map widgets (their Lua) instead of the runtime's menus; `PauseAi`, `GameModeTimer`,
`AddAllAIPlayers` still logged only (race.rs / ai.rs time the go and the AI); a second race per run is still refused
(`RaceOver`); captures: the original's moment of `RequestRespawnAtEventStart` vs the card's clock.

### 2026-11-17: the route free roam → race, and 7 race-route behaviour types (`hsm/behaviours_race.rs`; 92 types)

`mwtool hsm --run --event <id>` (docs/commands.md) plays the player's choice of an offline event from free roam
and follows the logic on: the Easy Drive menu's list item calls `StartEventFromEasyDrive` (the hook of the map's / list's
START RACE items; `StartEventFromBinding` is the map's variant), the free-drive machine ends its mode (HUD machine
exit `START_EVENT`), the GMM finishes it, loads the next event's data and starts "Race Game Mode" 730003 (circuit
462174 and sprint 906146 are the same machine); the race runs to the checkpoints, then outro, "On Event End", and
the GMM goes back to free roam by itself. State ids and transitions: decomp.md "Route free roam → race".
Found on the way (all fixed): (1) free roam ended at frame 4 since the event-flow batch: `MODEENDCHECK` reads
`FUN_006f46e0`, which returns GMM+0x2e78; that byte is *1* in a running mode (GMM constructor 0x70c603 and the
end-of-event reset 0x70b37d set it; `SETENDREASON` / `SetGameModeWon` clear it), so the old name "won" with default
false made every mode end at once. `Query::ModeWon` is now the byte (offline too), set by `LogicHost::new`
together with +0x2e7a `StoryMode` and +0x2e7b (also constructor values). (2) `FUN_0070d580` after the machine
finished: `natives::mode_machine::update` now ports the event change (`FUN_006f1720` 0 → 1 `FUN_006f06d0`(requested)
→ 2 → 3 when (GMM+8)+0xdc == 4, then `FUN_0070b1e0`: story mode and a request → pending event = requested, current
event / mode / +0x2e7f = +0x2e80 / +0x2e8a; else the hard reset `FUN_006e6af0`) and `FUN_0070caa0`'s next-event part
(record +0x8c, else in story mode the free-drive event with flag 1: the way back; free roam clears the request).
New `Request::LoadEventData`, `GameModeManager::transition_state`, `EventInfo::next_event` (the +0x8c field is not
named in the assets yet; the runtime fills it).
Types, in the order the route enters them (method as the earlier batches: factory = function before the name stub,
vtable from its `mov [eax], vtable`, slots from text.asm, constants from snapshot 1 / `exe-const`): `PAUSESIM`
0x77d990 → 0xfda650 (the same pause as `BPAUSED`), `ENABLEALERTS` 0x77dc60 → 0xfda7a4 (every slot is the base:
nothing; its effect, if any, is somebody else reading that the state is active: open), `CLRSELEVENT` 0x77b6d0 →
0xfd95c0 (update 0x73baa0), `HSB_NOSCORES` 0x77b380 → 0xfd9458 (scoring manager +0x124 counter), `CRASHESCAPE`
0x77b290 → 0xfd93ec (update 0x748150: 0.2 s period, 24 cool-downs, 15 m / dot 0.1, 4 s / 5 s, constants read from
`.rdata`), and two next-nearest ones off the offline route: `CHECK_ENT` 0x77d300 → 0xfda318 (always `HAVE`),
`HSB_TOG_LS` 0x77b970 → 0xfd96f8 (loading screen show / hide). Requests: ClearSelectedEvent, AdjustScoringDisabled,
CrashEscape, ToggleLoadingScreen, LoadEventData. Queries (defaults): CrashLocalOk (false), CrashPlayerCount (0),
CrashPlayerState (0), CrashPlayerNear (false). One test per type at the end of `flow_tests.rs`, four in
`mode_machine.rs`.
How far the route gets: the whole race loop with the stand-ins below; the only type the offline route still holds on
is `HSMPAUSEINTR` (agent B's: it is held, the intro sequence exits anyway). Everything else the race machine, its HUD
machine and the pause menu enter is ported; the rest of the unported types in these trees (about 40, 600+ states)
are online / sign-in / dialog / photo (`SHOWDIALOG`, `SUPPRESSPREF`, `WRITESESSION`, `RUNPRCONTEXT`, `LOADSTRSET`,
`HSM_SERV_OP`, `SIGNIN_CNTRL`, ...: dialog, nav-tab and credits parts of the pause menu are next).
Model decisions (driver stand-ins, not the original's): the headless run has no widgets or input, so `--event`
replaces state 1135029 "Easy Drive Menu" by the list item (hook call + exit `TO_EVENT`) and answers the first input
of 908987 with `EZ_RIGHT`; the `Players` / `Event` / `Transitions` / `GameLogicState` roots are not ported, so
`Players.LocalPlayer.ResetInProgress(NOT)` and `Transitions.Transitioning(NOT)` read TRUE, `Event.Data.EventIntro`
is the event's intro sequence, the "TRANSITION" general trigger fires with the intro, `Mode.HasIntroLoadCompleted`
is set at once, `Query.CheckpointCount` is the event's gate count and `CheckpointsFinished` false (answer it TRUE to
see the finish and the outro; `--answer 1560270=FALSE` stands for `Players.LocalPlayer.WasBusted`).
`--pause-at <frame>` holds the pause button (`Query.InputExit=PAUSE`, with
`--answer 'GameLogicState.IsControlGroupEnabled["MENU"]=TRUE'`): the pause menu's machine starts ("Run PageRunner2
Context - PauseMenuScreen"); `Mode.DisablePauseMenu` is missing from the `Mode` view model (the state exits through
an empty exit point). `RACE_MODE` (+0x170 of `GameLogicConfig`) is not read (field hash unknown), so
`IsRaceEvent` / `LastEventWasRace` stay with the runtime. Captures that would confirm: GMM+0x2d0c, +0x2ce4, +0x2e7e /
+0x2e80 at the moment a race is chosen in free roam (probe), and +0x2e78 in free roam (expected 1).
### 2026-11-17: the HSM's requests drive music, UI sounds, sim pause, world, mute, chevrons, triggers (`logic.rs`)

Under `--hsm` the runtime now carries out these Requests through the system that already does the job (logic.rs
module doc lists them):
- `PlaySongList` / `StopSongList` → `music::SongListRequest` replaces `songs.rs`'s hand-written list choice. "Play
  Freedrive Song List" (`USEDEFAULT`) asks `Query::DefaultEventSongList`; the runtime answers with the current mode's
  `MusicPlaylist` (1622455). `free_drive` = GMM+0x2e90.
- `PlayUiSound { field }` → `ui_sound.rs`. The field is an offset into the object behind `[[DAT_010edb08]+4]+0xb278`.
  That handle is filled by 0x4b80fe from the `GlobalSound` root (`SOUND/GLOBAL.BNDL` 0x79e7, loaded at 0x4b8033): its
  +0x14 handle `MiscSounds`, next to +0x1c `VehicleSounds` (+0xb270, already confirmed). The `MiscSound.UI` struct
  (`cfac01f3`, +0x40) holds 8 UniqueIds at +0x4c..+0x68: 1836407 / 1836401 / 1836405 / 1836335 / 1836058 / 1836333 /
  1836331 / 1836060 (`mw_assets::ui_sounds`, test `tests/ui_sounds.rs`). Each is a UI sequence of wave items. It plays
  on the effects channel, on the game clock (the sim-timer player UIM+0x47748).
- `SimPause` → `Time<Virtual>` paused / resumed, as the pause menu does. Only the pauser resumes. The HSM now steps on
  the frame clock (real time, capped at the game clock's 2/60 s) with `Clock::game` None while paused; before, it
  stopped with the game clock, and the logic could not have resumed it.
- `ShowWorld` → `FrontEnd::world_shown`. In driven mode `Screen.IsActive` (the transition's hold) = world loaded and
  shown: the original's HUD screen is pushed by the HUD machine inside "Show World Freedrive".
- `MuteSound` → counts per reason; any count mutes the world's sound (`FrontEnd::logic_mute`, ORed with the front
  end's own mute). `MuteRumble`, `SetFullRate` → kept in the host (`Query::FullRate`). `AdjustShowWorldCount` is the
  logic's own count.
- `HideChevron` → `chevron::ChevronsHidden` (route markers). `EnableTriggers` → `freeroam::TriggersEnabled` (trigger
  hits; off until "Show World States" enables them).
- Answers: `UIMessages.NISRunning` (cutscene.rs), `CollectablesReady` (freeroam.rs), `FullRate`,
  `DefaultEventSongList`. Queries answered by default are logged once each.

**Why the boot did not reach free drive under `--hsm` any more:**
- The `SignInState` root (same day) made `IsChatRestricted` false, so "Get Online Silently" runs "ServerOperation
  PCLogin" (`HSM_SERV_OP`, unported, held). Runtime stand-in: server operations exit FAILURE (no server). The chain
  then goes CancelLogin → FAILED → ClearMatchmakingState and on.
- With a continued profile `Mode.ShowRecap` is true. "Show Game Recap" waits on `UIMessages.NISRunning(NOT)`, now
  answered.
- `MODEENDCHECK` (update 0x757af0): when `FUN_006f46e0` returns 0 (`je 0x757b60`) it goes straight to the end check,
  so free drive ended in its first frame (`Query::ModeWon` default false). Runtime stand-in: true in free drive.

Verified (`tools/t --front-end --hsm --front-end-shots scratch/fe --mute`, `MW_MUSIC=1 MW_MUSIC_LOG=1`):
- Title at 0.88 s, key at 11.18 s, transition at 11.20 s, playing at 16.00 s (baseline 15.86 s).
- Title / transition / playing shots look the same as before.
- The HSM stays in "Freedrive Update". It plays list 1622455 (song 1622458 starts after the transition, the clock
  having been paused by "Offline Pause" while `Transitions.Transitioning`, frames 618–712).
- `hsm:` shows SimPause, ShowWorld, EnableTriggers, MuteSound, HideChevron, PlaySongList, MuteRumble, SetFullRate
  carried out.
- `--drive 911 --test accel` unchanged (3.58 s / 307.4 km/h).

Model decisions:
- Server operations fail.
- `ModeWon` true in free drive.
- `FUN_007983b0` read as "the sequence with that UniqueId".
- The UI sound items play whole, without fades.
- `HIDECHEV`'s message 0x1b and `ENABLETRIGS`' message on `DAT_010edb48`: receivers not traced, mapped to the route
  markers / free-roam trigger hits.
- No rumble or frame-rate change in the runtime.

Open:
- `PlayUiSound` not yet seen in a run (EasyDrive opens only through "Open EasyDrive Request").
- Races started by `race.rs` don't go through the HSM, so the intro race's list (2026345) and `HideChevron` in event
  intros aren't reached.
- `FUN_006f46e0`'s offline derivation.
- 46 Requests still logged only (world / profile / feedback-group loads, `RunLayout`, `PlaySequence`, players).

### 2026-11-17: the DLC lock in the runtime (`dlc.rs`)

Every DLC pack counts as not owned (user decision; decomp.md "DLC and entitlements" for the rules and the
runtime's coverage). Checked:
- `--drive 2196000` (Porsche 918 Spyder, Terminal Velocity) and `--race 2165035` (SIDEWINDER) stop with "… is part
  of the … PACK, which is not owned"; the 911 drives as before (accel test unchanged).
- Jack spots: 60 of the 183 hold a DLC car and show the store prompt; `--test jackspot` still swaps into the base
  game's 918 Spyder Concept.
- The airport: the barrier stands across the tunnel east of zone 43 (`PRESS E TO PURCHASE AIRPORT`, text 2211224,
  screenshot at (175, 48, −2010) facing west); a spawn on the runway or at (100, 48, −2010) (zone 43) is put back
  on the free-roam start. The arena lies west of the barrier: (120, −2010) is zone 700, (100, −2010) zone 43.
- `ZoneList` zones overlap (`zone_at` returns the first match); the airport test uses zone ids 36–57 (probe.md
  "Limits", observed in the original); arena 527914 holds only 7 of them.

### 2026-11-17: ownership check: a retail executable (`mw_gamedir::GameDir::ownership`)

User decision: the base game counts as owned when `NFS13.exe` is a known retail build; DLC is never inferred from
the files (the Steam depot ships every pack; the DLC lock is the logic's entitlements, agent round of the same day).
- `GameDir::ownership` hashes `NFS13.exe` (SHA-256, ~14 MB) against `RETAIL_BUILDS` (release, hash). Known: Steam
  1.4.0.0 (app 1262560). The EA app build is not known yet: add its hash once a copy is confirmed.
- `mostrusted` refuses to start on an unknown executable, before indexing, in both the native and browser builds
  (`run`). The message gives the hash to report. Tools (`mwtool`) do not check.
- Hashes are the one exception to "nothing derived from game data" (CLAUDE.md), and they hold no content.
- Tests: `mw_gamedir::ownership_tests` (the Steam install is retail; a fake executable is refused).
### 2026-11-17: seventh 20 HSM hooks verified and enabled (140 of 187; `natives/hooks_misc.rs`)

Handlers read from `re/out/live/text.asm`, callees from the dump. Mapping (all return EXIT unless noted; TRUE / FALSE as named):
CalculateBlacklistDescription 0x676a70 (`strtol(lpcValue)` - 1 -> `FUN_006f9380` on (G+0x2f3fc)+0x20: the flags +0x2c48..+0x2c4a are cleared, then
it picks the text (game pack lock "gamepack", vehicle not owned "VehicleName", not yet raced) and posts the HUD message: `Request`);
ResolveBoolBinding 0x680160 (member `Path` = 0x50617468, a four-byte name hashes to its bytes; `FUN_0067f4e0` = the Lua native's code, TRUE / FALSE);
SetVehicleAndRolloutFromSaveGameByVehicleID 0x680680 (`liId`, int) and ...String 0x687a30 (`strtol(lpcValue)`): `FUN_006800a0`, vehicle > 0 only
(existing `Request::SetVehicleAndRolloutFromSaveGame`); UpdateAwardProgressBindings 0x7eb4e0 (`FUN_00724680` + `FUN_00724690` on G+0x2f3f8 as
`UpdateProgressAwardsStrings`; returns the table's EXIT copy 0x117efa0 = same value); IncrementSetting 0x7eb610 / DecrementSetting 0x7eb680
(`lpcValue` = category, `DAT_0117ec1c` = index, both `strtol`; `FUN_007e8900` / `FUN_007e8970`: value < max -> value + 1 / min < value -> value - 1 through
the settings manager's slots; TRUE when changed, else FALSE; `SettingsManager::increment` / `decrement`); ExitWidgetGroup 0x7f24f0 (`DAT_0117ed40` text ->
`FUN_007eef40`, the Lua native's `exit_group_via`); UpdateDeferredViewModelSilent 0x7f9940 (`lpcViewModelName` = 0x1a5058d7 -> `FUN_007f72f0`(SM+0x280, name, silent = 1, &out):
load kind 2 instead of 1); HasScreenRegisterdDeferredViewModel 0x7f9990 (`FUN_007f7510`: FALSE while the page changes (+0x1b4 or +0x21c's +0x1904 not 0), TRUE
when a widget group's deferred list names it, byte comparison); SetJumpToStoreFlag 0x6770d0 (`FUN_00675580`: (G+0x2f3fc)+0x2f27 = 1, the binding text
`DAT_01177d18` read as an int (0 when unresolved) -> entitlement manager (G+0x2f3e0 +0x4ce0) +0xdbc); ClearEntitlementJumpID 0x6770a0 (that +0xdbc = 0);
ClearNetworkedTimer 0x68ae20 (message 6 into the queue `*0x010edb0c`+0x10); ClearOnlineEasyDrive 0x676d50 (`FUN_006f44e0` on G+0x14c08) and
RefreshOnlineEasyDrive 0x676d30 (`FUN_00706ec0` = clear + `FUN_00706c90` refill); SetGameVisibilityPublic / Joinable / InviteOnly 0x6762d0 / 0x6762f0 /
0x676310 (G+0x2a9e4 = 4 / 3 / 2); SetOnlineRaceCarFrozen / Unfrozen 0x68aea0 / 0x68af10 (`FUN_006bf330` +0xa0 != -1: engine message 0x37 via `FUN_00844f70`
on G+0x2ed70 with 1 / 0, then message 2 with the flag into the queue `*0x010edb18`+0x10). Test `the_seventh_twenty`; `the_twenty_ported_hooks` counts 140.
Model decisions: three member names are not known (hashes read from the snapshot `.bss`: setting index 0x2f348b52, ExitWidgetGroup text 0x83959c02, jump binding
0x9e943e15; several hundred thousand `lpc` / `li` / `lb` + word combinations tried, only `lpcViewModelName` hit), so the test cannot set them (missing member reads as ""/0);
`hook_misc.registered_deferred_view_models` and `screen_changing` are filled by the runtime (the page's widget groups are not in the logic); the entitlement jump id and the visibility are
plain fields (no reader in the logic yet).
Open: the runtime handles none of the new `Request`s (CalculateBlacklistDescription, UpdateDeferredViewModelSilent, ClearNetworkedTimer, ClearOnlineEasyDrive,
RefreshOnlineEasyDrive, SetRaceCarFrozen; the saved-vehicle and progress-awards ones were already open); `mod.rs` / `host.rs` tests now use
SetTargetRecommendedEvent as their "unported" example.
Next 20 candidates (all unported; offline-relevant first): PopThankYou (0x676840), StopAutologAlert (0x676860), SetDlcMessageIndicator (0x677110),
OpenStoreOverlay (0x677150), RemoveOnlineSeenUnlocksMenu (0x676d70), RefreshTopOfSpeedwallViewModel (0x6768f0), ResetPlaylistScoreState (0x676ac0),
InvalidateKinectSelectedClass (0x676d00), IsOriginIgoAvailable (0x7eb760), SignInStarted (0x7eb520), SilentSignInStarted (0x7eb550), SignInFinished
(0x7eb580), SetTargetRecommendedEvent (0x675bb0), SetCurrentRecommendedEvent (0x675c20), SetCarListForRecommendedEvent (0x675c50),
SelectRecommendationDomByIndexString (0x675d10), SelectRecommendationDomByEventIdString (0x675d70), RefreshRecommendations (0x676810),
GenerateRecomendationPostEventString (0x676890), CacheMwlAndLock (0x676dc0).

### 2026-11-17: sixth 20 HSM hooks verified and enabled (120 of 187; `natives/hooks_select.rs`, `hooks_flow.rs`)

Handlers read from `re/out/live/text.asm`, callees from the dump. Argument names found by hash search over the
snapshot's `.bss`: `liOptionId` (`DAT_01177a98` = 0x74b4f5f4), `liSelectedWorldId` (`DAT_01177bd8` = 0x4ee0ce22);
`lpcValue` strings go through `strtol(s, 0, 10)` (`FUN_00d751e0`) or `atol`. Mapping (all return EXIT unless noted):
CancelMilestoneTracking 0x675e50 (`FUN_0071d1e0` on profile db +0x5a3c: clears the tracking fields) ; HasEasyDriveOptionBeenSeen 0x676920
(`FUN_0072eb90`: seen byte of the "ez_options" row, TRUE/FALSE); SetEasyDriveOptionSeen 0x676970 (`FUN_00674d20`: ez_options row via
`FUN_007291b0`, or the "ez_mods" row via `FUN_007316d0` when `FUN_006ecae0` knows the id); SetEasyDriveOptionSeenToTrue 0x6769d0
(`FUN_00674d90`: online flag (GMM+4)+0xe8 and id != 0 -> `FUN_00703c10` only, else the rows with 1 then `FUN_007040b0`); SetSelectedWorld
0x675ad0 (`FUN_00645780` on G+0x2f468); SetAvailableVehicleListEvent 0x675cc0 (existing Request); CalculateDistanceToEventByIdString 0x675e00
(`FUN_0078ff50`: UIM+0x1f8f0 +0xcff0 = n, +0xd544 = 1); SetRolloutFromVehicleIdString 0x675ee0 (n > 0: `FUN_006c7950`(local player, n, 0));
SetHighlightedRolloutByIdString 0x675f40 (rollout manager +0x1c4 = n); SetHighlightedVehicleByIdString 0x680390 (`FUN_0067ef40`: +0x1c8 = n when
changed, record copied to +0x1cc or cleared); IsTargetScoreHit 0x6803e0 (local player +0x418 >= event record +0xcc, TRUE/FALSE);
ShowTargetScoreReminderMessage 0x680440 (`FUN_0067f7f0`: message 0x5f5ee "score" = target - score); SelectPaintPackFromEasyDrive 0x68d3a0
(`FUN_0068d200`); UpdatePendingRollout_EasyDrive 0x6903a0 (`FUN_00690030`) and _Stock 0x68ae60 (`FUN_0068aa40`: UI sound +0x64, then
`FUN_006830a0`(id, 0, 0)); SetFinalEventData 0x6766f0 (`FUN_006b9d00`: local player +0x5b8 = GMM+0x2ca3 + 1, +0x5c8 / +0x5cc = GMM+0x2d28 / +0x2d2c, +0x5d0 = 0);
StartEventFromBinding 0x676790 (`FUN_00674be0`: `lpcValue` is a binding read as int, out value starts at -1; not -1: `FUN_006e74f0`(id, 0));
SetWaypointToEventFromBinding 0x6767d0 (`FUN_00674c60`: same, then `FUN_00792390`, `FUN_00796ff0`(id), UIM+0x2c5e0 = -1);
UpdateProgressAwardsStrings 0x6768c0 (`FUN_0071fd20`(-1, +0x10e8) and (6, +0x12e8) on G+0x2f3f8); SetFMNMenuShowingVITAExclusiveEvents
0x676a30 (`FUN_006e0470`(1): GMM+0x2e85; was parked, now enabled). Test `the_sixth_twenty`; `the_twenty_ported_hooks` counts 120.
Model decisions: the "ez_options" seen flags are a mirror in `SelectFields` (the ez_mods branch is not mirrored; the profile db row write is the
`Request`); `event_target_score` (the event record +0xcc) and the player's score are filled by the runtime; the meaning of GMM+0x2ca3 / +0x2d28 / +0x2d2c is open;
a binding that does not resolve reads as -1 (assumed: the visitor leaves the preset out value).
Open: the runtime handles none of the new `Request`s (CancelMilestoneTracking, SetEasyDriveOptionSeen*, SetSelectedWorld, SetRolloutFromVehicle,
LoadHighlightedVehicle, ShowTargetScoreReminder, SelectPaintPack, UpdatePendingRollout, SetWaypointToEvent, UpdateProgressAwardsStrings).
Next 20 candidates (all unported; the ten Offline ones first): CalculateBlacklistDescription (0x676a70), ResolveBoolBinding (0x680160),
SetVehicleAndRolloutFromSaveGameByVehicleID (0x680680), SetVehicleAndRolloutFromSaveGameByVehicleIDString (0x687a30), UpdateAwardProgressBindings
(0x7eb4e0), IncrementSetting (0x7eb610), DecrementSetting (0x7eb680), ExitWidgetGroup (0x7f24f0), UpdateDeferredViewModelSilent (0x7f9940),
HasScreenRegisterdDeferredViewModel (0x7f9990); then small Online ones: SetJumpToStoreFlag, ClearEntitlementJumpID, ClearNetworkedTimer,
ClearOnlineEasyDrive, RefreshOnlineEasyDrive, SetGameVisibilityInviteOnly, SetGameVisibilityJoinable, SetGameVisibilityPublic,
SetOnlineRaceCarFrozen, SetOnlineRaceCarUnfrozen.
### 2026-11-17: 10 HUD / pursuit HSM behaviour types (`hsm/behaviours_hud.rs`; 85 types)

Route check: `mwtool hsm --run --steps 900` (docs/commands.md, free roam) enters four still-placeholder types:
`HORN_PRESS`, `WAITSYSMENU`, `COSTSTATESC`, `WAITFORCHASE` (all under "Controlling Vehicle Offline"; the rest of the route's 64 codes
were ported). Static walk of the machines 390148 / 371633 / 371635 added the next candidates in tree order: `DISABLETIME`,
`DISABLEWORLD`, `HOSTEZGUIDE`, `CLEARUIPOSTF`, then `SETPENDINGON`, `OVERRIDEPAGE` (all ported). Method as the earlier batches (factory = `mov edx`
of the name stub, vtable from the factory, slots from `text.asm`, parameter / exit keys decoded from snapshot 1; addresses in decomp.md).
Requests: CostStatePrune, CostStateScan, DisableStatsTimer, SetWorldRendering, DisableAudio, HostEasyGuide, ReleaseEasyGuide, ClearUiPostFx,
SetPendingDialog, OverridePage. Queries (default): HornPaused (false), HornDown (false), SystemMenuActive (false), CostStateActive (false),
LocalPursuitState (0), WorldRendering (true), EasyGuideShown (true), EasyGuideClosed (true). One test per type in `flow_tests.rs`.
Result: the free-roam command still reaches "Freedrive Update" and "Freedrive HUD" with an identical state trace (the four route types held before
and hold now, waiting for engine answers: system menu, pursuit state, horn input).
Model decisions: `COSTSTATESC`'s scan (0x7548e0 from 0x754989, the cost-record walk) is one Request, not ported; `HORN_PRESS`'s timer has no reader in the machine and
is kept in the instance, the input checks are two Queries; the clock for both timers is the game clock (`FUN_0046ee10`, step x rate; a stopped
clock = 0). `HOSTEZGUIDE`'s object fields are Queries / Requests (+0x54 state, +0x58 flag). `SETPENDINGON` has no known meaning for G+0x2f70c (named after the parameter).
Open, next unported on the machines: `SHOWDIALOG` (73 uses; enter 0x745e10 / update 0x740a20 / exit 0x740a80 read: DIALOG, POS_EXIT, NEG_EXIT via UI dialog
object G480+0x47740), `HSMPAUSEINTR` (0x73d100 / 0x74d040 / 0x74cfe0, scans UI objects), `LIMITEDEDSP` (0x75aff0 / 0x762650: 7-state online request),
`SUPPRESSPREF` (0x740f70 / 0x740f90 / 0x740fb0), `TAKEPHOTO` / `PHOTO_*`, `RUNPRCONTEXT`, `CLEARUIPOSTF` siblings; the sign-in family (`SIGNIN_CNTRL`, `SI_*`, `HSM_KEYB_ENT`, `HSM_SERV_OP`) is the bulk (offline-unreachable).
### 2026-11-17: the `UIConfig`, `SignInState`, `UISequences` and `GeneralSequence` binding roots

Registered like `Mode` (`FUN_00a4d2a0` root / type / object, `FUN_00a4e2f0` fields, `FUN_00a4e350` getters). Code:
`natives/ui_config_view.rs`, `sign_in_view.rs`, `sequence_view.rs`; one `ROOTS` line each. Tests per root.

**`UIConfig`** (`FUN_007a03c0`, `UIConfigManager`, 0x41c). Constructor values: `Platform` (+0x208) = the `.rdata`
string "PC" (0xfde864), `IsPlatformPC` 1, X360 / PS3 / VITA 0, `IsHighDef` 1, `NumberOfMonitors` 1. Language
getters read the UI language id at +0xb4 (0 until set): `IsEnglish` 0x78ec50 (1 or 2), German 6, Russian 7, Japanese
0xb, Traditional Chinese 0xc, Arabic 0xe (same codes as the language menu). `ShouldShowSubtitleOptions` (0x7a9df0) and
`IsLeftToRightLanguage` (0x7a9e20) answer true for language 0, else read a byte of the UI language table record
(`FUN_00794f30`), not ported. Model decisions: LTR false only for Arabic, subtitle options shown. Open: the runtime
does not set the language (`screen.language` stays 0, so `IsEnglish` is false until something sets it).
`HudAspectCorrection.*` / `Data.*` stay with the UI data.

**`SignInState`** (`FUN_00888d40`, `SignInViewModel` 0x3148, `Params` = `SignInParamsViewModel` at +8). Read from the
getters: `IsChatRestricted` (0x889bf0) stores 0, `IsMultiplayerAllowed` (0x889be0) stores 1, `IsAccountDeactivated`
(0x881610: error code +0x135c == 0x1b), `AuthenticatedWithBlaze` (0x889c10 -> 0x4c2070 -> 0xa82f20: the Blaze
connection object is null), `IsReadyForToS` (0x881650: byte +0x1589), plain bytes for `AccountCreated`,
`FirstUseInTitle`, `EMailHasEAAccount`, ...; all false / empty offline. **Finding:** the original's
`IsChatRestricted` is false, not true; the boot's "Get Online Silently" then goes to the silent sign-in (state
1597850 / `ServerOperation PCLogin`, the Blaze login, not ported: an offline run would wait for it). The earlier
`--answer SignInState.Params.IsChatRestricted=TRUE` is therefore an online stand-in (model decision: skip the
login), not a value of the code, and it stays in the documented commands. `SignInState.Error` is not a registered
member (does not resolve).

**`UISequences`** (`UISequenceService` 0x3258) and **`GeneralSequence`** (`GeneralSequenceService` 0x98). Getters:
`HideHudRequests` +0x3254, `IsAnimationTriggerSet[n]` (0x82bcb0, bit set at +0x20), `IsActionHostUsed["X"]`
(`FUN_0082fbe0`: byte 1 of the entry at +0x34 + 2 * i; unknown = false; set by `FUN_00829b20`), `UseDefaultHud` /
`UseThermalHud` / `UseNightHud` (0x82c690 / 0x82c720 / 0x82c7a0: mode = first nonzero request counter of +8 (none
match), +0x10 (thermal 2), +0x14 (night 3), +0x18 / +0x1c (neither), else 1 = default), `Params[n].Text1..3` (16 slots
of 0x320 B, valid byte +0x4c; invalid = null). General triggers: the service's bits at +0x88 (`IsTriggerSet[n]`,
0x82e950) and up to five named ones (`Triggers["NAME"]` = `FUN_0082fb00`: sorted table at +0xc, bits from +0x20, a
missing name is a null object; `Trigger.IsTriggerSet` 0x82c650). Setter `FUN_00829d30(name, n)` sets bit n - 1 (name 0:
the service's; a new name is added unless five exist), clearer `FUN_00829810` (an emptied name leaves the table,
`FUN_0082ea50`). `hash("BOOT")` is 1112493908, the `GeneralTriggerSequenceItem` name field the assets keep.
**The settable path:** `LogicState::set_general_trigger(name_hash, index)` / `clear_general_trigger` (and
`state.sequences` for HUD requests, action hosts, params, animation bits); the runtime calls them when the sequence
item fires / ends. `mostrusted/src/logic.rs` now does that for "BOOT" instead of answering the binding itself
(not compiled in this session: engine-free task; the integrator's build checks it); `mwtool hsm --run` sets BOOT
at start. Model decision: the named table is keyed by the binding's name hash (the getter looks up hash + 1, the
setter the value as given; the callers' packing is not traced).

**Result.** The documented free-roam command needs two answers instead of nine (`IsChatRestricted=TRUE`,
`1222788=DONE`; the `Mode.*`, `UIConfig.Platform`, BOOT and `Transitions` answers are gone) and still reaches
"Freedrive Update" and machine 371633; the 90-step boot still reaches "Playing". Captures that would confirm:
a live read of UIConfig +0xb4 and the SignIn VM bytes at the title.
### 2026-11-17: DLC and entitlements: the `EntitlementManager` root and the original's DLC gates

Worktree agent. Spec in decomp.md "DLC and entitlements". **User decision (2026-11-17): our game must not unlock
DLC cars or areas; the Steam depot ships every pack, so until a real entitlement source exists the base game counts
as owned and every DLC pack as not owned** (`natives/entitlements.rs`, offline default: an empty owned set; not a
model value).

**Sources.** `FUN_006401b0` (owned = in EM+0x22c, count +0x32c; the online pass special case), the registration
`FUN_00649eb0` (10 getters / 1 field, read from text.asm), the id globals' static initialisers (`.text` 0xf56530..,
0xf5bf10.., disassembled from `re/out/live/NFS13_snap1_vt.exe`: the dump's text.asm stops at 0xb81fff), the data
(`GAMEPLAY.BNDL` `Entitlement` / `GamePack` / `GameUnlockList` / `NucleusGrantMappingsList`, `GLOBALCONFIG.BNDL`
`GamePackConfig`, `GameLogicConfig` +0xac / +0x1f0 / +0x58 / +0x2a8), the vehicle list +0x2c (`VehicleListEntry`
`GamePackID`, `FUN_0042ade0`), the event `GamePack` (+0x20). `GameChangerID` is the name of the universal id
field 0x655d3f5f (it hashes to it; `FUN_005dbb20` reads it by name).

**Ported (engine-free).** `Entitlements` in `LogicState` (`has` = `FUN_006401b0`, `add`, `has_online_pass`,
`has_pdlc`, the GPM helpers `FUN_00643b40` / `643ae0` / `643c10` / `643c50` / `647780` / `647700` / `0072fcc0`,
`pdlc_purchasable` = `FUN_00647680`, `unlock_blocked` = `FUN_00686010`, `owns_any` = `FUN_00640210`,
`pack_content_available` (`FUN_006f3830`), `mod_slot_available` (`FUN_00678e60`), `airport_pack_owned`
(`FUN_00643c90`)); the root `EntitlementManager` (`HasOnlinePass`, `HasPdlc1..4`, `IsPdlc3/4Purchasable`,
`ThankYouCount`, `ShowDirectDownloadInfo`; `natives/entitlement_view.rs`); `Mode.CurrentVehicleGamepack` (the
local player's new `vehicle_id` → pack). `mw_assets::packs` (`pack_data`), `VehicleEntry::game_pack`,
`VehicleInfo::game_pack`, `Event::game_pack`, `mwtool packs`; `mwtool hsm --run` fills the packs (the boot run is
unchanged). Tests: `entitlements`, `entitlement_view`, `mw-assets/tests/packs.rs` (install).

**The four packs** (pack `GameChangerID`, release, entitlement): Ultimate Speed 2115003 / 1 / 2115001, Terminal
Velocity 2196330 / 2 / 2201004 (the airport: `GamePackConfig` +0x10; mod slots 11 / 13), Movie Legends 2196660 /
3 / 2201005, NFS Heroes 2196661 / 4 / 2201006. The base pack is 17127. In the install 20 cars and 46 offline events
belong to them (`mwtool packs`); online events / challenges of packs 2..4 also exist (34 / 19).

| pack | cars (`VehicleList` id) | offline events (id; † = granted by the entitlement's `GameUnlock`) |
|------|------|------|
| Ultimate Speed | 2076005 Bugatti Veyron Vitesse, 2076070 McLaren F1 LM, 2076137 Hennessey Venom GT Spyder, 2076203 Lamborghini Aventador J, 2076266 Pagani Zonda R | 2090001 DOWNFALL, 2090016 TERMINAL VELOCITY, 2090031 STEEL CITY, 2090046 DRIVE HARD, 2090062 CRITICAL PATH, 2090092 GREEN ZONE, 2090107 BURNING BRIDGES, 2090123 AROUND THE WORLD, 2090137† MOST WANTED VENOM, 2090152 THE DESCENT, 2109068 THE ENFORCER, 2109288 LIVE FOR SPEED, 2109323 FULL OVERDRIVE, 2109324 FULL THROTTLE |
| Terminal Velocity | 2196000 Porsche 918 Spyder, 2196065 BMW 1 Series M Coupe, 2196131 Audi RS3 Sportback, 2196197 Ford Fiesta ST, 2196263 Alfa Romeo Mito QV | 2165035 SIDEWINDER, 2165052 EASY DRIFTER, 2193062 AERIAL AGGRESSION, 2193106 ROUND TRIP, 2193221 POWER SLIDE, 2193406 DUTY FREE, 2204045† MOST WANTED 918 SPYDER, 2216000 DRIFT CITY, 2216641 AIR BRAKE, 2254089 AIR RAID, 2254100 GREEN BELT, 2254497 SHIP TO SHORE, 2254586 DESTRUCTION SITE, 2254919 JAILBIRD |
| Movie Legends | 2196334 Shelby GT500, 2196399 Pontiac Firebird T/A, 2196466 Dodge Charger R/T, 2196529 Aston Martin DB5, 2196595 Aston Martin DBS | 2191016 LICENCE TO THRILL, 2191171 BOOTLEGGING, 2191269† MOST WANTED GT500, 2191413 SUDDEN IMPACT, 2191469 BULLET TRAIN, 2191507 MAYHEM, 2191553 DEADLINE, 2191594 LIVE AND LET DRIVE, 2191651 PULP FRICTION |
| NFS Heroes | 393276 BMW M3 GTR, 2076339 Lamborghini Diablo SV, 2076399 Porsche 911 GT2, 2076462 Nissan 350Z, 2076524 Nissan Skyline GT-R (R34) | 2189981 GET TO CALLAHAN, 2191208† MOST WANTED M3 GTR, 2191848 BRAKING TRACTION, 2191871 RUNNING WILD, 2254660 HIGH STAKES, 2254708 FUELLED, 2254712 BRING THE NOISE, 2254765 SPEED DEMON, 2282050 INNER CITY PRESSURE |

**Not ported / open.** `DlcMessageIndicator` (profile value 0x11) stays unresolved: the HSM reads it as
`DlcMessageIndicator(GTEQ:n)` through `HSB_SEL_EX_B`, i.e. the string visitor, whose int32 slot (0xa49bf0, slot 12
of 0x00fd8368) prints `%d` without the comparison; answering 0 would stall "Is DLC Messaging Required?" (2241062)
on exit "0", so either property type 1 reaches another slot (`FUN_00a4eb00`) or the comparison happens elsewhere:
not traced. With it the original would advertise the packs to a new profile (not seen → not bought → TRUE). The
unlock manager (`IsAssetUnlocked` → `FUN_0068cc30` / `FUN_00688fa0`) is not ported: its entitlement test is
(`unlock_blocked`). The offline airport "put back to the start" (probe.md "Limits", reason 6) was not found in the
dump: the gates seen offline are the barrier chevron (`FUN_006e68d0`) and the store prompt near it
(`FUN_006cfd40`). Captures that would confirm: the original with a save that has no DLC at the airport road
(x 136, z −2010): the barrier and the prompt text 2211224; the garage with a DLC car selected; a DLC jack spot's
prompt. Runtime work (garage, jack spots, free roam / map events, mods, airport) is specified in decomp.md.

### 2026-11-17: fifth 20 HSM hooks verified and enabled (100 of 187; `natives/hooks_{flow,ui}.rs`)

All 20 were already ported but parked; handlers re-read from `re/out/live/text.asm`, now enabled in `Registry::original`.
Easydrive sounds (OnOpenEasyDrive 0x6877b0 / OnStartEvent 0x687830 / OnSelectEasyDriveOption 0x6878b0 /
OnOpenEasyDriveMods 0x687930 / OnCloseEasyDriveMods 0x6879b0): identical bodies, the UniqueId at `[obj+0xb27c + [obj+0xb278]+0x28 + f]`
(obj = `[DAT_010edb08]+4`) with f = 0x50 / 0x60 / 0x58 / 0x54 / 0x4c; if non-zero `FUN_007983b0`(UIM+0x2f480... +0x32858) maps it and
`FUN_00a45070` starts it on UIM+0x47748 (`Request::PlayUiSound`), then EXIT. GMM (= `[G+0x2f3fc]`) byte flags: +0x2caa
(PlayEndOfIntroSequenceInEasyDrive sets 1, Clear sets 0, Should tests), +0x2f28 (SetHasIntroEasyDriveBeenActive / Has...), +0x2e9b
(SwitchToGameplayLoadWorld), +0x2e9a (LeaveGame), +0x2e9d (ExitForMultiplayerMatchMaking), all set 1 and return EXIT; the tests
return TRUE / FALSE. EXIT-only bodies: ClearE3MultiplayerProgression (0x676360), ShowPartyInvitePopup (0x7eb730), OnPhotoSharedToWall
(0x7eb7c0), ShowX360GamerCard / ...EasyDrive / ...ByPlayerIndex (0x7eb6f0 / 0x7eb710 / 0x7eb740: a `FUN_009f16b0` hash lookup, result
dropped). The three PC handlers return `DAT_0117efa0`, not 0x1177ef0; both hold the same 8-byte key in `snap1/mem.bin`
(`00000068 a1614568`), so EXIT. The 20th, ForceKeyboardShutdown (0x677060): `FUN_00845dc0` sets bit 3 of `*DAT_010edb0c`, posts
{0xf, 0} through `FUN_0084dcc0`, clears the bit (`Request::ForceKeyboardShutdown`). Test `the_fifth_twenty`; `the_twenty_ported_hooks` counts 100.
Model decisions: none new. Open: the runtime does not handle `Request::ForceKeyboardShutdown` / `PlayUiSound` yet.
Next candidates (offline, unported): SetFMNMenuShowingVITAExclusiveEvents (parked), CancelMilestoneTracking (0x675e50:
`FUN_0071d1e0` on profile db +0x5a3c), HasEasyDriveOptionBeenSeen, SetEasyDriveOptionSeen, SetEasyDriveOptionSeenToTrue,
SetSelectedWorld, SetAvailableVehicleListEvent, CalculateDistanceToEventByIdString, SetRolloutFromVehicleIdString,
SetHighlightedRolloutByIdString, SetHighlightedVehicleByIdString, IsTargetScoreHit, ShowTargetScoreReminderMessage,
SelectPaintPackFromEasyDrive, UpdatePendingRollout_EasyDrive, UpdatePendingRollout_EasyDrive_Stock, SetFinalEventData,
StartEventFromBinding, SetWaypointToEventFromBinding, UpdateProgressAwardsStrings.
### 2026-11-17: 9 more HSM behaviour types of the free-drive machines (`hsm/behaviours_mode2.rs`)

`MUTEALLRUMBL`, `PAUSEMSGS`, `PLAYSONGLIST`, `WEAPONSOK`, `MINIMAPICONS`, `SYNCPROFILEW`, `HIDECHEV`, `PAUSEAI`,
`DOBODYSHOP` (67 types in all; addresses and slots: decomp.md "The HSM interpreter", fourth batch). Method as the
earlier batches: factory from the name registration (string push, `mov edx, factory` near 0x778xxx), vtable from the
factory, slot bodies from `text.asm`, parameter keys from snapshot 1. Requests: MuteRumble, PauseMessages,
ForcePauseMessageQueues, PlaySongList, StopSongList, WeaponsAllowed, ResetPlayerWeapons, UpdateMinimapIcons,
SetProfileSyncFlag, HideChevron, PauseAi, UpdateBodyShop. Queries (default): CurrentEventSongList (0),
DefaultEventSongList (0), EventWeaponsAllowed (false), GmmFlag2e80 (false), ProfileServiceConnected (false),
ProfileSyncFlag (false). One test per type in `hsm/flow_tests.rs`; `mwtool hsm --run` (docs/commands.md) still
reaches "Freedrive Update" with "Do Body Shop" and "Mute All Rumble (Paused)" now running real behaviours.
Model decisions: `SYNCPROFILEW`'s connected states 1..14 (the online profile sync) are not ported, connected is
treated as offline; `DOBODYSHOP`'s update and `MINIMAPICONS`' update are single per-frame Requests (the work is
engine state; the body shop's zone-list reads `FUN_005372f0` / `FUN_005378d0` and the minimap visibility check
0x74e0b0 are not ported); `EventWeaponsAllowed` defaults to false (the free-drive record's byte +0xf2 was not read);
`PAUSEMSGS` keeps queue names, not pointers. Open: the host side of these 12 Requests (message system, song
player, rumble); `ResetPlayerWeapons` = `FUN_006c8e90` only summarised; the free-roam route still holds
`HSMCARSWAPLO` etc. (agent B2). Captures that would confirm: G+0x2ef50..60 and G480+0x325e8 in free roam.
### 2026-11-17: 8 event-flow HSM behaviour types (`hsm/behaviours_event.rs`)

`SETENDREASON`, `MODEENDCHECK`, `HSB_WAIT4IN`, `SET_LAY_SET`, `DOBLACKLISTS`, `CHECKPTS`, `EVENTSTARTS`, `HSMCARSWAPLO` (66 types
in all). Method as the entry below, with one correction: the factory of a name is the function *before* the name stub
(the stub's `mov edx, <factory>`), not the one after it; its `mov [eax], vtable` gives the vtable. Entry-point and exit
constants (`ENDED` 0x117a2d8, `RETRY` 0x117a668, `EXIT`, `TIME_OUT` 0x117a978, `CREDITS` 0x117a0e8) decode with the
packed-name alphabet from snapshot 1. Details per type: decomp.md "The HSM interpreter".
Model decisions: `SETENDREASON` / `MODEENDCHECK` write GMM+0x2e78 / +0x2d08 through `Request::SetGameModeWon` /
`SetEndReason`, which `natives::mode_machine::apply` applies to `GameModeManager::won` / `end_reason` (the `Mode`
binding root reads them); `FUN_006f46e0` (won, offline: medal / time maths on event data), `FUN_006f1530` (limit
reached), `FUN_0074e790`'s list scan and `FUN_006e27e0` (display time left; its `FUN_00eda646` is `_ftol` of 0.0, read
from the asm) are `Query`s the runtime answers (the host answers only the online `won`). `HSB_WAIT4IN`'s candidate
exits are `Query::InputExit { index }` (first one the parent handles wins, as the original's chain; the two cues
`FUN_0079fb40` are not ported). `DOBLACKLISTS`: phases 0, 2, 3, 6 and the credits tail ported; the steps of phases
1 / 4 / 5 are engine requests whose effect on the phase is a query; the view model `BlacklistShutDown` is published
with defaults only. `CHECKPTS`, `EVENTSTARTS`, `HSMCARSWAPLO` (0x200, 0x1f0, 0x74d0 bytes of instance state): lifecycle,
exit conditions and parameters ported, the per-frame work is `Request`s (checkpoint passing, event-start markers,
the 7-phase car swap) for the runtime: open, with "Rules that are C++ ... are ported from the code" still to do.
Test: 11 in `hsm/flow_tests.rs`; `mwtool hsm --run` still reaches "Freedrive Update".

### 2026-11-17: skid / crash sound glitch: not reproduced headless (user: "extremely loud, distorted")

- **Original vs ours** (`ref/xkr_launch`: the original's own audio, the inputs replayed with the tap, levels
  relative to the full-throttle phase): hard braking is +1.8 dB in the original and +1.0 dB in ours (our skid
  channel −7.8 dB). Our car decelerates more slowly under the same brake input (20–30 mph where the original has
  stopped), a handling difference.
- **Live output = tap**: real-time drives recorded from the null sink's monitor (`parecord`; `MW_INPUT_SCRIPT`
  with `MW_INPUT_SCRIPT_START=playing`): a handbrake turn, a 58 mph crash, then wheelspin against the obstacle
  (spin 1.0 at 0 mph and 7800 rpm in first gear while the throttle is held, the `Spin` set at −40 dB). Peaks 0.25
  at the user's master 0.35, no clipping, no step over 0.2. The live mix matches the tap's two channels × 0.6 ×
  0.35 within a dB.
- **Race intro**: the intro's teleports give ~1 m/s landings, which start the `Spin` / landing sequences; the
  vehicle channel there is ~8 dB over a free-roam drift.
- Not the cause: the volume channels (equal bases 0.6), a doubled vehicle bank (built once), the engine
  normalisation missing from the tap (it is in the bank's master gain), the device format (48 kHz stereo like
  the null sink).
- The browser runs at master 1.0 (no settings file): +9 dB against the desktop's saved 0.35.
- Open: the user's exact situation; a capture of the original's drift and crash with audio.

### 2026-11-17: the `ModeVM`, `Net`, `JoinGameState` / `CreateMatchState` / `MatchMakeState` roots

Root dispatch: `LogicState`'s `binding::Host` now looks the root hash up in a table (`state.rs` `ROOTS`), one module
per root (`natives/<root>_view.rs`); a member a module does not answer falls through to published definitions.

**`ModeVM`** (`modevm_view.rs`). `GameModeViewModel` is registered by `FUN_006fe520` with ECX = the view model
embedded in the game-mode manager at manager +0x20 (`FUN_0070be40` calls `FUN_006e7670` on `esi+0x20` at 0x70c5de,
then `FUN_0070a120` on the manager with the stack argument `param_2 + 0xe8`, the binding manager the registrations go
to). So a VM offset is a manager offset minus 0x20: `ForceOpenEasyDrive` VM +0x2c89 = manager +0x2ca9 (the hook's
byte), `ModsAppliedInOutro` VM +0x24fc = manager +0x251c (`GarageFields::mod_flag`); the VM keeps back-pointers to
the manager (+0x1914) and the game logic (+0x1910). Getters ported: `CanSkipIntro` (0x711d90, always true),
`ShowSkipPrompt` (0x711dc0: manager +0x2510 float > 0.0, the constant at 0xf8f344), `ShowPositionFromCheckpoint`
(0x716e50: manager +0x2526 `HasTargetSpeed` clear and the current mode id is not 0x212851). The 6fe520 block also
registers more `GameModeManager`-typed (`Mode`) fields (`HasTargetSpeed`, `DisablePauseMenu`, `PersonalBest`, ...):
not ported as `Mode` members (unresolved members read false / 0 as before). Not ported: the blacklist getters,
`IsWaitingForBustRetry` (0x6e2d10: +0x2e90 clear and `UISequences.IsActionHostUsed["RETRY"](NOT)`),
`WasBlacklistBeatenAndNotShutdown` (0x6e8860), `GetFinishedString` (0x6e85c0), `ShouldShowInCarPosition`
(0x716ec0, a UI global), `CurrentContext`.

**`Net`** (`net_view.rs`; `FUN_004c4740`, 28 getters, all `LAB_`, read from text.asm). Offline answers from the
getters' no-connection branches: `IsPlayerConnectedToFirstyParty` true (0x4cf2a0 constant), `AreAllPlayersConnected`
true (0xa83320: empty slots count as fine), `IsInX360Party` false (constant); `IsInOnlineGame`, `IsLoggedInToServer`,
`IsConnectedToServer`, `IsHost`, `IsGameAdmin`, `IsInLobby`, `IsInPlaygroup`, `AreWeHostMigrating`, ... false; counts 0.
Model decision: the Blaze / session objects never exist offline (not ported). The indexed members do not resolve.

**`JoinGameState`, `CreateMatchState`, `MatchMakeState`** (`online_state_view.rs`): `IsValid` (0x64d820 / 0x64d870 /
0x64d850: a validity field of the zero-initialised object) and `DoRematchmaking` (0x651660, needs the playgroup
pointer): false (model decision).

**Result.** `mwtool hsm --run` (docs/commands.md "Freedrive Update") needs no `Net.IsLoggedInToServer` answer any
more and still reaches "Freedrive Update". "Open EasyDrive Request" (1560481) takes FALSE through `ModeVM.ForceOpenEasyDrive`
and 1532918 ("Is In Online Game") takes FALSE: both exits handled. Tests `modevm_view`, `net_view`,
`online_state_view`.

Open: entitlement getters (`HasPdlc1..4`: `FUN_006401b0` on global ids), `EntitlementManager.DlcMessageIndicator`
and `ThankYouCount` (0x651480, 0x6514c0); `UIConfig` / `SignInState` / `Autolog` / `PlayDirector` / `StoreViewModel`
and the other HSM-read roots (listing: `mwtool bindings` rows with source `hsm:`) are unported; `Recommends`,
`Screen`, `Transitions`, `Demo`, `ActiveGameMode` are not registered roots (published data or the engine).

### 2026-11-17: the `Mode` binding root (`natives/mode_view.rs`)

**Registration.** `FUN_0070a120` (`ECX` = the game-mode manager) registers the root "Mode" (`.rdata` 0x00fc7ef0)
of type "GameModeManager":
- 34 fields (`FUN_00a4e2f0`: name, offset into the manager, count, property type).
- 86 getters (`FUN_00a4e350`: name, getter, count, property type). The full table with offsets and getter
  addresses is in the dump at 0x70a120.
- Property types: 9 bool, 0 / 1 / 2 integers, 8 float, 0x11 time, 0xf / 0x10 strings, 0x1a object.
- It then calls `FUN_006fe520`, which registers "ModeVM" (`GameModeViewModel`) on its argument (the caller
  `FUN_0070be40` passes param_2 + 0xe8). ModeVM is not ported.

**Port.**
- `LogicState` is now the binding host: `Mode` first, then the view models. A `Mode` member the table does not
  answer falls through to a data definition published as "Mode" (`Mode.Time`, `Mode.DisablePauseMenu`).
- The resolve call sites moved from `view_models.resolve_*` to `LogicState::resolve_*`.
- Fields: every manager byte the logic keeps. New `GameModeManager` fields: +0x2e77 world loaded, +0x2e76,
  +0x2ea8, +0x2e91 / +0x2e92, +0x2d08, +0x2e64 / +0x2e68, and the event-kind flags.
- `Mode.Data` = the current `GameMode` object. Its members are the GeneSys fields by hash
  (`Events::game_mode_fields`, `mode_view::data_fields`).
- Getters read from `text.asm`:

  | Getter | Address | Reads |
  |---|---|---|
  | `WorldLoaded` | 0x712010 | an event and +0x2e77 |
  | `ShowLoadingScreen` | `FUN_006f49c0` | +0x2e81, +0x2e7f, event, online / +0x2e76, +0x2e77, then the two bits of +0x2ea8 |
  | `HasIntroLoadCompleted` | 0x70da10 | +0x2e82 |
  | `IsInGameIntro` / `…Freedrive` | 0x711e90 / 0x711e70 | +0x2e91 or +0x2e92 / +0x2e91 |
  | `HudHsm` | 0x716f40 | the mode's `GameMode` +0x20 |
  | `IsPauseAllowed` | `FUN_006e62a0` | |
  | `RetryLastEventEnabled` | 0x711ee0 | +0x2eac |
  | `LeaveGameRequested` | `FUN_00711f30` | |
  | `ShowRecommendsIntro` / `RecommendsEventActive` | 0x7120c0 | G+0x2f6d0..d8: none offline |

- The online session getters are false offline.
- The runtime sets +0x2e77 from `FrontEnd::ready`. The `mwtool hsm --run` driver sets it at once.

**Result.**
- Boot to free roam no longer needs the six `Mode` answers. `mwtool hsm --run` with only the boot's answers and
  `Transitions` reaches "Freedrive Update", and the HUD machine 371633 comes from the mode's data.
- "Is Recommends" takes FALSE.
- In the runtime the mode machine now takes the recap branch. `Mode.ShowRecap` (+0x2e88) is set by the ported
  `CalculateShouldShowRecapSequence` for the continued profile, so "Show Game Recap" pushes page 1560241 and waits
  for its group (`mwtool hsm --run` uses a fresh logic state).
- Test `mode_view::tests::mode_root`.

Model decisions:
- The local player's +0x2c8 (`LeaveGameRequested`) is taken as clear.
- Members not ported (times, online / Autolog / recommendation getters, career statistics) do not resolve.

Open:
- `ModeVM` (`ForceOpenEasyDrive` at +0x2c89 of its object: the EasyDrive prompt's "Open EasyDrive Request").
- `Net.IsInOnlineGame`.
- Whether the original shows the recap on a continued profile (a capture of the original's boot with a save).
- The recap page in the runtime.

### 2026-11-17: widget input and conductors (`natives/pages.rs`, `logic.rs`, `input.rs`)

**The input mapping table** (`FUN_007e7910`, `.rdata` 0x00fe7260, 33 entries of {name, id, directional flag}; an
unknown name gives 0x60):
- The documented 42–61, then `to_main_menu` 62, `play_it` 63, `fullscreen_photo` 64, `friend_recommends` 65,
  `hitbox` 66, `dbg_win_event` 67, `dbg_switch_progression` 68, `easy_left` / `right` / `up` / `down` 69–72,
  `universal_accept` 73, `universal_alt` 74. The flag is 1 for 42–51 only.
- Default bindings: keyboard `FUN_00483470` (Windows virtual-key codes), pad `FUN_00482250` (pad codes
  cross-checked against 46–49 and 52–55):

  | Action | Keyboard | Pad |
  |---|---|---|
  | `easy_*` 69–72 | Numpad 4 / 6 / 8 / 2 | d-pad (codes 8 / 9 / 6 / 7, plus an argument `DAT_00fa1db8` not read) |
  | `universal_accept` 73 | Enter | A |
  | `universal_alt` 74 | Backspace | X |
  | `to_main_menu` 62 | F1 | Start |
  | `scroll_up` / `down` 50–51 | Numpad 8 / 2 | right stick Y |
  | `play_it` 63, 65 | A | — |
  | `fullscreen_photo` 64 | Right Ctrl | — |
  | `dbg_*` 67 / 68 | F10 / F11 | — |

**Action definitions:** `UserActionDefinition` = GeneSys object `0x01000000_<id>` in the screen bundles:
`InputMapping`, `Analogue`, field `2da735e3` = repeat (`AssetStore::ui_action_definition`).

**Widget input** (`LogicHost::input`, every frame before the HSM update):
- Each active widget's actions fire per `FUN_007f50e0`: analogue every frame; repeat on the press or the
  auto-repeat tick, directional ones only above 0.05; otherwise on the press.
- Routing follows `FUN_007f40d0`:
  - Cross-widget `Widget::Function`: offered to the target's scripts until one handles it.
  - Lua: every script of the widget out of its 0.15 s cooldown; a refusal by `ActiveBinding` also starts it.
  - Native: `ExitGroupVia` → the group exit; other names → the HSM hook.
- `input.rs` `MenuAction` gained 50, 51, 60, 62, 69–74 and `MenuInput::by_id`.
- Test `tests/pages.rs::calibration_actions` (boot screen group 1532543): accept → NEXT, cancel → BACK, the Lua
  `IncrementSetting` runs.

**Conductors:**
- The title's conductor "PressStart" is the front end's title: `StopSequence` / `DestroyConductor` stop it.
- Any other `StartSequence` runs headless (`logic.rs` `Playback`): the sequence loaded on a thread, its clock
  stepped with its jump controllers (bindings through the logic's view models), and the conductor set idle at
  the end, so `HasSequenceFinished` / `IsConductorIdle` keep the original's timing.
- A binding that gives no sequence id makes the conductor idle at once.

Model decisions:
- Every active widget takes input (the focus / modal rules were not traced).
- An analogue value reaches a Lua callback as an extra argument.
- A native callback calls its hook without arguments.
- No success / failure sounds yet (the definition's handles `7866a8e2` / `1cb2a09a` were not read).
- `MW_INPUT_SCRIPT_START=playing` lets a script run in free roam.

Open:
- Drawing conductors' sequences: a generic sequence player (frontend.rs and event_card.rs each draw their own
  today). `PlaySequence` (Lua) is on the boot, pause (431082) and two more screens; the native `PlaySequenceFast`
  is on 44 screens and is not ported.
- In free roam nothing fireable is active yet. The HUD's `ShowMap` (Tab) is gated by `Mode.IsInGameIntro(NOT)`, and
  EasyDrive's group 1135023 opens only through "Open EasyDrive Request". Both need the `Mode` roots.

### 2026-11-17: front-end screens from the HSM: page contexts, widget groups, the boot page (`natives/pages.rs`, `logic.rs`)

**Page contexts and widget groups** (`mw_logic::natives::pages`, inside `LogicHost`; `LogicHost::apply` carries out
the logic's part of a request for every host).
- `PushPageContext` pushes {name, page} and asks the host for the page's screen (`Pages::wanted` →
  `pages::load_screen` → `Pages::provide`; feature `assets`). `PageReady` is true once the screen is loaded.
- `ActivateWidgetGroup` builds each widget of the group as decomp.md "Widget definitions (JSON) and widget scripts"
  specifies: one Lua state per widget with the natives bound, the chunk run, `Construct(this)`,
  `SetConfigurationItem` per scalar `Script` entry, then `OnActivated`.
- `PreInputUpdate(dt)` runs every frame. `DeactivateWidgetGroup` / `LeavePageContext` call `OnDeactivating` +
  `Destruct`.
- The widgets' Lua states live in `LogicHost::pages`, outside the `RefCell` the natives borrow.

Model decisions:
- The `Widget` root is one published JSON object of the active widgets' typed fields. The original resolves it
  through the script's view model, slot 8. Adapters are not published yet.
- Every native is bound, not only `LuaHooks`.
- `WidgetExit` (page +0x208) is read from WM+0x488.
- Native screen scripts are not run.

**The boot page, from the original's data** (`mostrusted --hsm`; `frontend.rs` "driven" mode):
- "Run PageRunner2 Context - Boot Flow" pushes 491012 (12 widgets). "Activate Press Start Widget Group" activates
  491032. Its `Background` widget's `PlaySequence` script creates the conductor "PressStart" and starts
  `Widget.Background.Sequence`, which resolves to 1022357, and `frontend.rs` plays the title.
- The key press only binds the primary user (`HSB_P1BIND`). The HSM then asks for the group's transition
  (`PlayWidgetGroupTransition` 491032 → the boot transition 1836020).
- `TransitionFinished` follows the front end's transition. `TransitionState` stays at its default 2, so the group
  deactivates after 10 frames (the original's cap) while the transition holds over the load, and the script stops
  its sequence.
- The boot then leaves the page and, in free drive, pushes the HUD screen 371621 (98 widgets) and activates its
  groups.
- Headless timings: title started by the logic at 0.58 s, key at 10.85 s, transition at 10.87 s, playing at
  15.65 s. Without `--hsm` the flow is unchanged (key 10.76 s, playing 15.56 s).
- Test: `mw-logic/tests/pages.rs` (needs the install).

Open:
- `CreateConductor` / `StopSequence` / `DestroyConductor` are still logged only. The front end's sequence player
  should become the conductor.
- The other group transitions (data: each group's `ScreenTransition`) and `TransitionState`.
- The HUD screen's widgets run their scripts, but nothing draws from them yet (`hud.rs` is still hand-written).
- `Mode.*` roots: `Mode.ShowRecommendsIntro` and two others leave "Select HUD" / the EasyDrive prompt with an
  unhandled exit.

### 2026-11-17: the original's HSM runs inside the runtime, shadow mode (`--hsm`, `mostrusted/src/logic.rs`)

`mostrusted --hsm` runs "Top State" (20527) every frame on `LogicHost`, next to the hand-written flows. The
game-mode manager's update (`mode_machine::update`, `FUN_006abc10` order) runs after each HSM update. Requests are
logged (`hsm:` lines, the first of each kind) and applied to the logic state (`mode_machine::apply`); the runtime
does not carry any of them out yet. Hook-queued requests are drained right after the hook, so the log keeps the
original's call order. Answers from runtime state: the title's "BOOT" trigger (`FrontEnd::boot_trigger`),
`PrimaryUserBound` (the title key press while `ListenForPrimaryUser` listens), `NewProfile` (`--new-game` or no
saved profile), the loading screen (`Phase::Loading`), `Mode.WorldLoaded` (`FrontEnd::ready`) and
`Transitions.Transitioning` (`Phase::Transition`). The other answers are stand-ins (model decisions), the same as
`mwtool hsm --run`: platform PC, offline, chat restricted, `ConnectToAutolog` exits DONE, snapshot 1's `Mode`
bindings (HudHsm 371633, FeedbackGroupID 2, ...), events available when listed, weapon lists loaded at once,
unported behaviours hold and unported hooks answer EXIT.

Headless front-end run (`tools/t --front-end --hsm --front-end-shots scratch/fe --mute`):
- The machine waits on the title until BOOT (frame ~550, ≈ 9.3 s, as the title sequence sets it), then in
  "Press Start" until the key press.
- Then, in two frames, it goes boot → profile ("Set Up Continue Game" for a saved profile) → "Playing" →
  "Run Game Mode" → machine 390148 → "Wait for Loading" → "Show World Freedrive" / "Freedrive Update", and the
  free-drive HUD machine starts.

"Wait for Loading" passed at once because our world was already loaded (10.3 s, before the press). Every other
engine wait answers "done" at once because the runtime does not carry out the Requests yet: profile, world and
feedback-group loads.

Open: carry out the Requests one by one and drop the hand-written flow each replaces: `PushPageContext` /
`ActivateWidgetGroup` (front-end screens), `LoadWorld`, `LoadProfile`, `StartGameMode`, the HUD machine's requests.
`Mode` / `Event` binding roots (the GMM's binding table) instead of the stand-ins.

### 2026-11-17: fourth 20 HSM hooks verified and enabled (80 of 187; `natives/hooks_{flow,event,ui}.rs`)

Handlers read from `re/out/live/text.asm`, callees with `fn`. Already ported but parked, now verified byte for byte and
enabled: SetSafehouseFromProfile (0x676070) and ApplySelectedVehicleRollout (0x6762c0) return EXIT only;
ClearHudInstantQueue (0x67a7b0: frees UIM+0x126e8 through `FUN_00cc4780`, +0x126e8 = 0, +0x126e0 = 0);
StopAllSequences (0x680480: `FUN_0067f8c0` = ClearHudMessageQueues, then `FUN_00a44dd0` on UIM+0x47748 / +0x4774c);
StopSpeech (0x676100: `FUN_008423a0` on G+0x2c3c0; the Lua native of the same name is a separate handler 0x6784e0);
SkipCurrentTrack (0x676b40: `FUN_0079f760` on UIM+0x30370); ToggleLoadingScreenOff (0x677010: UIM+0x2d204 &= ~0x20,
+0x2d21c = 0); ClearForceOpenEasyDriveFlag (GMM+0x2ca9 = 0), ClearCreditsTransition (GMM+0x2f26 = 0),
ClearJumpToStoreFlag (0x677080: GMM+0x2f27 = 0; the table lists it as Online but the PC body is this one store),
SetFMNMenuShowingCommonEvents (`FUN_006e0470`(GMM, 0) = GMM+0x2e85 = 0).
Fixed and enabled: DeactivateDamage / ReactivateDamage (`FUN_0068acb0` / `FUN_0068ad50`, identical but for a flag
byte 1 / 0): only with the local slot occupied and the car loaded (`FUN_006ba2f0`), post message 0x1f {vehicle (+0xa0 or
-1), flag} through `FUN_0069a5e0` (`Request::VehicleDamage` now carries both). CleanAndRepairVehicle (`FUN_006926b0`):
the local record's rollout copy (vtable +0x18, 0x2b dwords, two zeroed) applied by `FUN_0068e9d0`(+0x98, copy, 2), then
`FUN_00844940`(vehicle) when +0xa0 != -1 (`Request::CleanAndRepairVehicle { vehicle }`).
New: ClearPendingResets (0x676080: `FUN_0063d410`(G+0x25690, i) for i in 0..16; per slot it clears the loading-screen bit
0x1000 / UIM+0x2d238 when the slot is the local car's and resets the entry: `Request::ClearPendingResets`);
TransferCurrentHeatToVehicle (0x6761c0: the local vehicle id = (record vtable +0x18)+0x30 via `FUN_006ba320`; if non-zero
`FUN_0072a280`(profile db G+0x2f454+0x4964, id, 0) writes the "garage" row's "heat" column = 0; `Request::TransferCurrentHeatToVehicle`);
IsSpeedTrapEvent (0x680530 = `FUN_0067fe30`: online event, else the event type record's byte +0x10 (via
`FUN_0053c020` on the handle (GMM+4)+0xf4) is 0x34 or 0x47, `FUN_006e1720`; `GameModeManager::current_event_kind`);
SetValidProgressionData (0x687750: `lbValid` = 0xce68ae4f from `.bss` 0x1177a6c; `FUN_00686e70` zeroes 11 entries of
G+0x2f3e4, then its +0xa4 = flag and G+0x2f3e8 +0x70 = 0: `Request::SetValidProgressionData`);
SetCollectablesNotInWorld (0x676330: `FUN_0070d260`, `FUN_007000d0` on G+0x14e40); SetRollout (0x68d2f0: `liRolloutId`
= 0xd04318ef from `.bss` 0x1177c78, `FUN_0068bbf0`(G+0x2f404, id): copies the rollout with that id into the local slot's
record and applies it). The `.bss` hashes were read from `re/out/live/snap1/mem.bin` and matched by `field_hash`.
Test `the_fourth_twenty`; `the_twenty_ported_hooks` now counts 80.

Model decisions: no local player = nothing happens (the original dereferences `FUN_006bf330` unchecked);
`current_event_kind` is filled by the runtime (the type record behind the current event handle is not modelled).
Open: what G+0x2f3e4 / G+0x2f3e8 are (SetValidProgressionData); whether the local vehicle id of
TransferCurrentHeatToVehicle (+0x30 of the vtable +0x18 object) equals the garage "vehicleid".
Next candidates (parked, offline or trivial; verify the same way): OnOpenEasyDrive, OnStartEvent, OnSelectEasyDriveOption,
OnOpenEasyDriveMods, OnCloseEasyDriveMods, PlayEndOfIntroSequenceInEasyDrive, ShouldPlayEndOfIntroSequenceInEasyDrive,
ClearPlayEndOfIntroSequenceInEasyDrive, SetHasIntroEasyDriveBeenActive, HasIntroEasyDriveBeenActive,
SwitchToGameplayLoadWorld, LeaveGame, ExitForMultiplayerMatchMaking, ClearE3MultiplayerProgression, ShowPartyInvitePopup,
OnPhotoSharedToWall, ShowX360GamerCard, ShowX360GamerCardEasyDrive, ShowX360GamerCardByPlayerIndex,
SetFMNMenuShowingVITAExclusiveEvents.
### 2026-11-17: HSM boot placeholders ported, the game-mode manager starts free drive (`hsm/behaviours_mode.rs`, `natives/mode_machine.rs`)

13 more behaviour types (58 in all; details decomp.md "The HSM interpreter", third batch): the boot's three
placeholders `PRELOADCOPS` (streams `VEHICLES\VEH_<id>_HI/LO.BNDL` for the player's car, the default cop 0x1df59
and the cop table), `HSB_RNLAYOUT`, `LIST_INVITE` (waits for ever offline), then the free-drive mode's
`REQTRAFRSTRT`, `GETSPEEDWALL`, `PREFETCHSCRE`, `ALLOWSIMPAUS`, `SETSIMPAUSES`, `BPAUSED`, `HSB_RUNNING`,
`ENABLETRIGS`, `HSB_DISCONGP` and `RUNHUDSUBNET` (the same vtable slots as `RUNSUBNET`: one class under two names).
**What `StartGameMode` does** (from `FUN_00701cb0` / `FUN_0070d580`, values read in both snapshots): the event is
G+0x2f6f8, on the boot the free-drive event 390703 that `SetNewGame` picks; the GMM starts the event's `GameMode`
+0x1c instance (390147 "Freedrive Game Mode") once the event data and the mode's `WeaponList` are loaded. Ported
engine-free as `natives::mode_machine` (`start_game_mode` applied by `LogicHost::request`, `update` called by the
`Hsm` owner after `Hsm::update`, queries `NextEvent` / `GameModeStarted` / `GameModeEvent` / `GameModeMachine` /
`FreedriveEvent` answered from `LogicState` by `LogicHost::query`); mw-assets reads `GameMode` +0x1c / +0x20 / +0x40
and `GameLogicConfig` ids (`StartConfig::ids`). `mwtool hsm --run` plays the GMM and reaches "Freedrive Update" with
the "Freedrive HUD" machine at frame 3 (route in decomp.md) with six binding answers on top of the boot's five
(`Mode.HasTransitionedEvents`, `Mode.Data.FeedbackGroupID`, `Mode.WorldLoaded`, `Mode.NeedTheMapScreen`,
`Mode.HudHsm`, `Transitions.Transitioning(NOT)`; values as snapshot 1 reads them): the `Mode` binding root is the
GMM's binding table, not ported.
Model decisions: the driver marks an event available when it is in GAMEPLAY's `EventList` (stands in for
`FUN_006e58a0`'s world list lookup, which picks 390703 as the snapshots show) and loads event data and weapon lists at
once; `HSB_RNLAYOUT`'s slot is named by the layout id in Requests (the original keeps a slot handle); the
`.bss` qword 0x0117ac30 it zeroes is not ported; `HSB_DISCONGP`'s per-frame re-block is a Request every update (the
logic host applies it to `InputBlocks`); online paths of `REQTRAFRSTRT` / the GMM are ported as read but untested.
Open: the runtime host (`mostrusted/src/logic.rs`) should call `mode_machine::apply` in `request`, fall back to
`LogicHost::query`, and call `mode_machine::update` after `Hsm::update`; `FUN_0070b1e0` (end of event),
`FUN_007090f0`, `FUN_0070caa0`; `RUNSUBNET`'s cross-resource load. Next candidate types on the free-roam route (held
as placeholders): `MUTEALLRUMBL`, `PAUSEMSGS`, `PLAYSONGLIST`, `WEAPONSOK`, `MINIMAPICONS`, `SYNCPROFILEW`,
`HIDECHEV`, `PAUSEAI`, `DOBODYSHOP`, `HSMCARSWAPLO`, `DOBLACKLISTS`, `MODEENDCHECK`, `CHECKPTS`, `EVENTSTARTS`,
`HSB_WAIT4IN`, `SET_LAY_SET`, `SETENDREASON`. Captures that would confirm: G+0x2f6f8 and GMM+0x2cfc right after
the boot of a fresh profile; the `PRELOADCOPS` instance's lists (+0x98 / +0xcc) in free roam.

### 2026-11-17: third 20 HSM hooks verified and enabled (60 of 187; `natives/hooks_{flow,event}.rs`)

Handlers read from `re/out/live/text.asm` (vtable-only), helpers with `fn`: RemoveAllCops 0x67a740 (`FUN_006795c0`, per
occupied slot `FUN_006bf190` = kind +0x9c == 2 and (+0x578)+0xb5, then `FUN_006ce180`(slot, 3)), AreAllAIPlayersLoaded,
RequestRespawn / ...ToLastSwapSpot (`FUN_006cd340` / `FUN_006cd3a0`: +0x2c4 = 0, (+0x435 = 1), `FUN_006c9fb0`(0, 0)),
HasLocalPlayerSetRollout, ResetChaseStats (`FUN_0065b830`(pursuit, 1) then heat +0x3d8 / +0x3dc / +0x3e8 = 0),
ResetLocalPlayerHeat, TransferVehicleHeatToPlayer (+0x3dc only), ResetLocalPlayerDamage (`FUN_006ba2a0` then
GMM+0x2e7f = 0 then message 0x16), ClearNitrous (`FUN_0065de30` = thunk to `FUN_00658c90`, arg PM+0xa6b4),
ShouldShowEscapeSequence (`FUN_006e03d0` = GMM+0x2e90, or the mode record (GMM+0x2cf4 / +0x2cf8)+0x18 == config +0x60),
FixUpRecommendationRetry (`FUN_006e04d0`: only when GMM+0x2e90, `FUN_00726ca0`(`last_played` value via `FUN_00732650`)),
SyncTimeOfDay (`FUN_006f4960`), ClearPostEventAwards (`FUN_006e6670` + `FUN_006e28c0`, all 25 offsets match),
RemoveRoadblocks (`FUN_008456d0`), ResetProps (`FUN_00846830`, message 0x15), HACKSuspend / Resume (G+0x2f0c8 = 3.0, `.rdata`
0xf92eb0 = 0x40400000; `FUN_0083c930` group 1 bit 6), SetSeenEventsForVehicleToTrue (`FUN_006e0480`, GMM+0x2e86),
ForceOpenEasyDrive (GMM+0x2ca9). All exit through EXIT except the three bool tests (TRUE 0x1177fe0 / FALSE 0x1177ee0).
All 20 were already right; one fix: AreAllAIPlayersLoaded now needs a vehicle (+0xa0 != -1) as well as `loaded`
(`FUN_006ba2f0` returns false without one). Test `the_third_twenty` (through `Registry::original`).

Model decision: AreAllAIPlayersLoaded in the original walks the records from PM+0x5f50 with a stride of 0x5e0
(the records are 0x7f0 apart), so slots 13..23 read shifted memory; the port reads each slot's own record. Open: confirm
with a probe read whether slots 13+ ever matter (they would only differ with AI racers).
Next candidates (still unported, offline): SetSafehouseFromProfile, ClearHudInstantQueue, StopAllSequences, StopSpeech,
CleanAndRepairVehicle, DeactivateDamage, ReactivateDamage, ApplySelectedVehicleRollout, ClearPendingResets, TransferCurrentHeatToVehicle,
IsSpeedTrapEvent, SetValidProgressionData, SetCollectablesNotInWorld, SetRollout, SkipCurrentTrack, ToggleLoadingScreenOff,
ClearForceOpenEasyDriveFlag, SetFMNMenuShowingCommonEvents, ClearJumpToStoreFlag, ClearCreditsTransition.
### 2026-11-17: Lua natives, fourth batch: the last 15 the UI's chunks read (100 of 385; `natives/lua_garage.rs`)

The intersection of `mwtool natives --area offline --unported` with the native names in `mwtool lua --out` column 2
is now empty: every offline Lua native a shipped UI chunk names is ported. Handlers from the dump, `re/out/live/text.asm`
for 0x67a6b0 (AddNewGarageItem), 0x7e9c30 (ReleaseAllImages), 0x692740, 0x690270, 0x680340 (the dump has no function
there; `fn` does not resolve register `this`, the asm does).
- Rank natives: the `this` of `FUN_006ec470` / `FUN_006ec4f0` / `FUN_006ec5a0` is **G+0x14c08**, the profile manager
  (PPM); the rank list is at PPM+0x94 (count +0x98, (record, offset) pairs, rank at `*(rec+0x28)+0x18+off`, threshold key at
  +0xc). GetMaxRank = max rank (0 empty). GetRankForSP(sp) = `FUN_006ec4f0`(sp - local player record +0x408), where the
  record is `FUN_006bf330` of PM (G+0x2f3dc): rollout `(*(G+0x2f3e4)+0x44) == 4` → the rank with the nearest threshold at
  or below the argument (`FUN_006893c0` = smallest positive matching row, else 0), else 1. GetSPForRank(rank) =
  `FUN_006ec5a0`(rank) + the same +0x408: rank 0 → 1; above the max → `FUN_006ec230` (largest value over the listed ranks);
  else that rank's threshold in mode 4, 0 otherwise.
- AddNewGarageItem(kind, id, flag): `FUN_006fe380` on G+0x14c08 (sic, same object): `FUN_006f8950` validity (item-type tree),
  then a sorted id → byte map gets the flag. HasEarnedAward(awardId, uniqueId): `FUN_007322a0`, query "OfflineAward" on
  the progression database (G+0x2f454 +0x4964), true when a row exists. SelectEasyGuideCategory(id): `FUN_00792fc0` on
  UIM+0x47640 (find the category with that id among the current page's children, push the current page, descend).
  SetFocusVehicleId: `FUN_0086bbf0`(id, 0) on OM+0x4c90. SetLocalPlayerLicenseRegistration: `FUN_00679760` → `FUN_006ee6f0`
  (8 chars into the local player's 0x1c-byte record, +4). SetRolloutFromProfile / SetSinglePlayerRollout: the same
  requests as their hooks. SetVehicleAndRolloutFromSaveGameByVehicleID (Lua 0x680340 = `FUN_006800a0`, > 0 only). Requests
  (engine-only): ApplyModUpgrade (`FUN_0068ab10`, also sets GMM+0x251c), ReleaseAllImages (`FUN_007e1340`: image
  slots in state 2 → 3), ResetNeutalVaues (flag 8 of `*0x010edb18`, message {8,0} queued), SetAvailableVehicleListEvent
  (`FUN_00871330` on OM+0x5ee0), SetVehicleAndRolloutFromSaveGame. Tests in `lua_garage.rs`; `mwtool lua`: 115 scripts run, 0 errors.

Model decisions: the rank table (`GarageFields::ranks`, thresholds precomputed from `FUN_006893c0`'s data lookups), `sp_base`,
`rank_mode`, the garage validity set, earned awards and the guide's per-page `children` are filled by the runtime, not read
from memory. GetRankForSP with no threshold at or below the argument: the original dereferences a null record; we return 1.
Open: filling those fields from the install (the rank/threshold data, OfflineAward rows); the engine side of the five new requests.

Next candidates (offline Lua natives still unported, 163 minus these; none is named by a UI chunk, so they are reached
from the HSM behaviours/hooks, GeneSys sequences or the in-race feedback/event scripts, which this session did not
trace; by family): camera (TransitionCameraTo*, Get/SetCameraFov, ICE camera, overhead / finish-line camera), feedback
messages and sequences (ShowFeedbackMessage, StartFeedbackSequence, Load/UnloadFeedbackGroup, loading-screen natives),
sound (PlaySound, PlayMusic, PlaySpeech, mix snapshots), race/competitor queries (GetCompetitor*, GetRaceDistance,
GetDistanceToFinish, IsInTrigger*, GetTrigger*), vehicle control and queries (SpawnVehicle, TeleportVehicle,
SetCarLocked/Frozen, GetVehicle*, speed getters), traffic (StopTraffic, SetTrafficDensity), nitrous, and the rollout /
route ones (SetEventFromAvailableVehicles, SelectDefaultRollout, SetRolloutFromVehicleID, SetRouteFromPlayerTo*,
SetSelectedWorld, GetSelectedVehicleId / ColourId, IsAssetUnlocked, SaveControllerConfiguration). The rollout/route and
selected-id ones are the cheapest next (state fields like this batch); the camera / sound / traffic / vehicle ones are
mostly engine requests plus a few getters that need runtime-filled fields.

### 2026-11-17: Lua natives, third batch: 15 more (85 of 385; `natives/lua_misc.rs`)

Each read from its handler (dump; `re/out/live/text.asm` for the six the dump lacks: ReleaseSettings 0x7eb170,
PrepareControlSettings 0x7eb140, GoBackEasyGuideCategory 0x675910, Enable/DisableDreamshotsCameraControlGroup
0x7ea120 / 0x7ea150, StartViewingEasyDrivePages 0x680300). Natives: UpdateDeferredViewModel (0x7f96a0, FUN_007f72f0),
ClearViewModel (FUN_0081ed30 / FUN_0081ec00: the entry swapped with the last, downloads of that name removed or
their +2 cleared), GetCurrentFreedriveEvent (FUN_0068fa10, shared with the hook), GetNextEventForProgression
(FUN_0068fba0: other-mode events that are available, listed at G+0x29204 and pass the rollout check; pass 1 skips
`EventInfo::completed` = FUN_006ec6a0, pass 2 takes any, else the free-drive pick), StartActionBind (FUN_00878790),
SetGameMode / SetDistrict (read their argument and do nothing), Prepare/ReleaseControlSettings (FUN_00879010 /
FUN_00879c70), GoBackEasyGuideCategory (FUN_00790a60), StartViewingEasyDrivePages (+FUN_007997f0), Enable/Disable
DreamshotsCameraControlGroup (bit 4 of group 0 of the input-block set G+0x2f0a4: FUN_0083d030 clears, FUN_0083d000
sets, each counting at +0x2f0c0; `InputBlocks::set` added), SetLocalPlayerLicensePlate (FUN_0067fa20),
ChangeVehicleOnCustomRollout (FUN_00691110). Tests in `lua_misc.rs`; `Download` records now carry name and flags.

Model decisions: UpdateDeferredViewModel pushes `MiscFields::deferred_ids[name]` (else -1): the original's out
value (set by FUN_0081d1f0 inside FUN_00820550) was not traced; the load is `Request::UpdateDeferredViewModel`.
SetLocalPlayerLicensePlate's telemetry event (FUN_004c3890 "GAME EVNT LICE") is not ported. The `this` of
FUN_006e4d10 / FUN_006ee760 (plate list) is not identified; plates are filled by the runtime.
Open (closed in the fourth batch): GetRankForSP's `this` is G+0x14c08.
### 2026-11-17: 11 more HSM behaviour types, the boot needs no behaviour answers (`hsm/behaviours_game.rs`)

The 6 types removed from the first batch were verified against `text.asm` (factories are not in the dump: vtable
from the factory's `mov [esi],vtable`, slots from `.rdata`, bodies from `text.asm` plus `fn` for callees) and
ported: DBAUTOSELWD, SELECT_PROF, WAIT4FEEDBK, WAITFORVEHIC, ISNEWPROFILE, ISAUTOTESTIN, plus HSB_STRSCRPT (only
slot 1 own, returns 1). Then the unported types the boot still ran as placeholders: REBOOT_REQ, ENABLETIME,
COLLECTABLES, LOAD_FDBK_GR (7 + 4 = 11 ported in all). Details and addresses: decomp.md "The HSM
interpreter" (second batch). Findings: SELECT_PROF names the profile `%llu` of the persona id (`FUN_00428860` into
`DAT_010ecc10`) and posts message 0xc on exit; the `.bss` parameter keys read from snapshot 1 are `GROUP`,
`GROUPBINDING` (shared by WAIT4FEEDBK and LOAD_FDBK_GR) and `UNLOADONEXIT`; the earlier WIP's readings were all
confirmed (no corrections). Name stubs (`FUN_00779e10` etc.) return (hash, factory) in edx: that is how a name
maps to its factory.
Request: AutoSelectWorld, SetProfileUser, ProfileUserSelected, StartCollectables, StopCollectables,
LoadFeedbackGroup, UnloadFeedbackGroup, AdjustTimeEnabled. Query (default): AutoSelectableWorld (true), UserId (1:
model decision, offline has no persona), FeedbackGroupLoaded (true), VehicleListLoaded (true), NewProfile (true),
RebootRequested (false), CollectablesReady (true). Tests in `hsm/flow_tests.rs`, one per type.
`mwtool hsm --run` reaches "Playing" with 5 answers instead of 11; each of the 5 is needed: the BOOT trigger,
`UIConfig.Platform`, `Net.IsLoggedInToServer`, `SignInState.Params.IsChatRestricted` (engine bindings / online
services) and state 1222788 `ConnectToAutolog` (online Autolog sign-in; its subtree "PC Login" waits for a
sign-in screen). None is a behaviour answer. Still placeholders on the boot path:
PRELOADCOPS (0x744c50 / 0x75c1c0 / 0x758030, resource streaming), HSB_RNLAYOUT (0x77f5f0, 7 slots), LIST_INVITE
(0x74dd80, online invites) are the next candidates. ENABLETIME's counter meaning (G+0x25624) is inferred from the
name.

### 2026-11-17: second 20 HSM hooks verified and enabled (`natives/hooks_{flow,event,ui}.rs`)

Handlers read from `re/out/live/text.asm` (TRUE = `DAT_01177fe0`, FALSE = `DAT_01177ee0`, EXIT = `DAT_01177ef0`).
All 20 matched the earlier port, no Rust fix needed; they moved from `register_parked` to `register` (now 40 of 187
hooks). Verified: SetFreedriveEventToPlay (0x690310: `FUN_0068fa10` pick, then `FUN_0063e380`(G+0x2f4c8, id, 0)
= pending +0x230 / aux +0x234 = -1 / flag +0x238), SetIntroLoadComplete (GMM+0x2e82), ShouldShowSafehouseExitSequence
(`FUN_006e0520` is `xor al,al; ret`, so TRUE), ShutdownGame (`DAT_011a2470`), SetStoryMode / SetGameModeWon /
SetAllowLoadingScreen / SetRetryAtEnd (bool member tag 5 from `StoryMode` / `Won` / `lbEnable` / `Retry` into
GMM+0x2e7a / 0x2e78 / 0x2e81 / 0x2ead), IsChallengeActive (GMM+0x2ce0 ≠ 0), RetryLastEvent (`FUN_00732650`(PPM, 1)
= `last_played`[1], then `FUN_006e74f0`(id, 0)), StartEventFromEasyDrive (`lpcValue` tag 2, strtol base 10, skip on
-1), RequestRespawnAtEventStart (`FUN_006cd380`: player +0x2c4 = G+0x2f6f8, `FUN_006c9fb0`(0, 0)),
AwardPostEventAwards (`FUN_006e65e0`: amount GMM+0x2e70 > 0 → `FUN_00736a90`(+0x2ce8, event, amount); each award
+0x38a0[..+0x38b4]: `FUN_00736ab0` if `FUN_0071f7e0` else `FUN_006792b0`), ResetCurrentEventStats (`FUN_0067f700`:
`FUN_006600d0`(event +0x2ce0, mode (+0x2cf4/+0x2cf8)+0x18, online)), RemoveAllRaceCars (`FUN_006d1310`: vtable +0x20 of
all 24 slot pointers, `FUN_006ce180`(slot, 3) for occupied 12..23, PM+0xa660 / 0xa694 / 0xa698 = 0),
ClearHudMessageQueues (`FUN_0067f8c0`: UIM queues +0x7230 / +0xa6f0, records +0x12458 (stride 0x7c) and +0x12468 /
+0x124f8 / +0x12574 / +0x126e8). Engine-only bodies (a `Request`, the hook itself just calls with no argument and
exits EXIT): SetEventFromAvailableVehicles (`FUN_00673d70`), SetTimeOfDayFromEvent (`FUN_0067f570`),
PreparePostEventAwards (`FUN_006fbb60`, 330 lines), AddAllAIPlayers (`FUN_00686f30`).
Model decision: the runtime must fill `LogicState` where the `Request` doc says so (PreparePostEventAwards writes
GMM+0x2eb4 / 0x2e70 / 0x2ce8 / 0x38a0 awards, which `AwardPostEventAwards` reads); until it does, those stay at
their defaults. Test: `hooks_tests.rs::the_second_twenty` (through `Registry::original`).
Next candidates (still parked, same files): RemoveAllCops, AreAllAIPlayersLoaded, RequestRespawn,
RequestRespawnToLastSwapSpot, ResetChaseStats, ResetLocalPlayerHeat, TransferVehicleHeatToPlayer,
HasLocalPlayerSetRollout, ResetLocalPlayerDamage, ClearNitrous, ShouldShowEscapeSequence, FixUpRecommendationRetry,
SyncTimeOfDay, ClearPostEventAwards, RemoveRoadblocks, ResetProps, HACKSuspend/ResumeCarControlForIntro.
### 2026-11-17: 15 HSM behaviour types of the boot flow (`hsm/behaviours_game.rs`, `behaviours_ui.rs`)

The stopped agent's WIP (22 types, built from `fn` and `text.asm`) was checked against the dump function by function (factory → vtable → each method; the C bodies of 0x76f7f0 / 0x76eac0 / 0x76eb50 / 0x76e4e0 / 0x76e5e0 / 0x73e730 / 0x744810 / 0x7448e0 / 0x7450e0 / 0x73ee50 / 0x73eed0 / 0x75c870 / 0x75e6f0 / 0x73d130 / 0x73d1d0 / 0x73d3d0 / 0x73e220 / 0x73b980; the vtable-only ones 0x76ffd0, 0x73ddf0, 0x73da30 / 0x73da60 / 0x744040, 0x73e7e0 / 0x73e930, 0x740f00, 0x745b10, 0x73b9e0, 0x73e2a0 from the WIP's reading of `text.asm`, not re-read). Ported (15): RUN_PG_RNNR2, ACTI_WDGT_GR, HSB_P1BIND, SETBLACKBACK, CHANGEVSYNCS, MUTE_SOUND, LOADWORLD, LOADPROFILE, IN_ON_GAME, RUNGAMEMODE, GAMEMODE (partial: DISCONNECT only), RUNGAMELOGIC, LOAD_ALL, BINGAME, HSM_PLAYING. Tests: `hsm/flow_tests.rs`, one or two per type. Request variants (`natives/state.rs`): PushPageContext, LeavePageContext, RestoreFocusGroup, ActivateWidgetGroup, PlayWidgetGroupTransition, DeactivateWidgetGroup, ClearWidgetExit, SetBlackBackground, SetFullRate, MuteSound, LoadWorld, LoadProfile, ApplyProfileSettings, ReleasePrimaryUser, ListenForPrimaryUser, RemovePlayers, CreateLocalPlayer, StartGameMode, StopGameMode, PlaySequence, StopPlayback (the WIP's `StopSequence` clashed with the Lua native's), LoadAll, EnterGame, LeaveGame, StartPlaying, StopPlaying, SessionLost; `Host::request` / `Host::query` with `behaviours::Query` (defaults named per variant; `mwtool hsm --run --answer Query.<Name>=<v>` overrides, `PrimaryUserBound` follows the driver's press).
Removed, not verified in this pass (next candidates, code is in the git history of branch worktree-agent-ad67203ead166fb17): DBAUTOSELWD 0x7702e0, SELECT_PROF (0x73f270 / 0x75cd00), WAIT4FEEDBK 0x746a50, WAITFORVEHIC 0x741b70, ISNEWPROFILE 0x73d350, ISAUTOTESTIN 0x76e8a0, HSB_STRSCRPT (all slots base).
Model decisions: engine waits default to "ready" (a missing engine never blocks); GAMEMODE's ActiveGameMode upkeep, Autolog refreshes and the GAMEEVNTSTRT / GAMESESSSTRT telemetry are not ported; the playback ids of `PlaySequence` are a process counter (the original's handle comes from the sequence player). `mwtool hsm --run` reaches "Playing" with the old answer set and with HSB_P1BIND, LOAD_ALL, LOADPROFILE, LOADWORLD no longer answered (docs/commands.md); the remaining unported behaviours of that run: SELECT_PROF, DBAUTOSELWD, WAIT4FEEDBK, WAITFORVEHIC, ISNEWPROFILE, ISAUTOTESTIN.
### 2026-11-17: Lua natives verified against their handlers (60), 10 more ported (`natives/lua_menus.rs`)

Method: each handler read from the dump (`fn <addr>`) or, where the dump lacks it (15 of them), `re/out/live/text.asm`; argument conversion, pushed values, state offsets and callee compared. Verdicts: **58 verified, 2 fixed, 0 unregistered**.
Fixed: `ChangePostFXSetting` (0x007e9f80) reads the key with `fistp` at the default rounding (`ROUND`), not truncation (delta stays `_ftol2`); `StartTextEntry` (0x007f2380) reads the 6th argument (flag bit) the same way. New `lua_args::round_int`. Comment fix: `SetOfflineEventToPlay`'s `FUN_0063e380` runs on the object at G+0x2f4c8 (fields +0x230 / +0x234 / +0x238), not G.
Verified (field, offset and conversion match): SetOfflineEventToPlay, SetTargetOfflineEvent, SetStoryMode (GMM+0x2e7a), GetChallengeVehicleSet, SelectDrivingTestListGroup, SetRequestedEasyGuideContent, StopViewingEasyDrivePages (UIM+0x47698 byte, +0x47730), GetCurrentSafeHouse, SetSelectedVehicleId / SetSafeHouse / SetTargetRouteFromSafehouse / ChangePerkOnCustomRollout (argument read, nothing else), print, the five Resolve*Binding (FUN_0067f4e0 / 6794e0 / 679550 / 7e7cc0 / 7ecac0), ExitWidgetGroup (FUN_007eef40: the `{...}` text = chars 1..len-2), the Set{Bool,Int,Double,String,UniqueId}Variable and DeleteVariable (optional prefix when more than 3 / 2 arguments; 64-byte path buffer), SetModalMode, GetPlatformName ("PC"), SmoothStep (x87 maths and clamp, NaN passes through), SetLanguage / GetInitialLanguageSelectLanguage (maps are inverse), Create/Destroy/Start/Stop conductor and sequence natives, HasSequenceFinished, IsConductorIdle, GetTextEntryState (+0x48), GetEnteredText (+0x38fc), RequestDialog, GetDialogResult (-1 when none, entry removed), ClearDialog, DoRpc, all settings natives (category by id, vtable slots 2..12 as in lua_settings.rs; Min = slot 7, Max = slot 6; unsigned pushes), SaveSettings / RevertSettings / RevertCategorySettings, RegisterPostFXSetting (record layout and key 3 start value), ResetPostFXSettings.
Model decisions found while verifying (marked in the docs, not wrong): `StartSequence` / `HasSequenceFinished` do not parse the sequence binding (the original does nothing / answers true when it does not parse); `RequestDialog` opens the id without looking for a definition; `StartTextEntry` does not clear the keyboard object's +0x44 (the runtime owns +0x48); `SaveSettings` skips the category's slot 5 (after-save hook); `IsSettingValid` asks "does the index name a setting" instead of the category's slot 2; unknown conductor / category ids dereference null in the original and are idle / 0 here.
New (10, all offline, `natives/lua_menus.rs`, state `LogicState.menus`): SetNewGame (0x006902a0, the hook's `hooks_flow::set_new_game`), SelectChallengeVehicleSet (0x00679cd0: G+0x2f418+0x334, `Request::RecountChallengeVehicles` = FUN_006f3f90), SelectVehicleCategory (0x006902d0: G+0x2f404+0x1bc, `RebuildVehicleList` = FUN_0068b7c0), SetSelectedVehicleColour (0x00679ff0 -> FUN_00657b50), SetVehicleModifier (0x00679f20 -> FUN_00657d10), SignInStarted (0x007eac10 -> FUN_00881f10(1): OM+0x10d0 fields +0x158a/+0x1588 = 1, +0x135c = 0, +0x1360 = -1, +0x139c/+0x136d = 0), SignInFinished (0x007eac40 -> FUN_00886f00: both fetched lists emptied, +0x158a/+0x1588 = 0), SelectPack (0x007ea480 -> FUN_008739f0/FUN_00642a90, +0xdce = 1; index past the packs selects none, model), IsViewModelDownloadComplete (0x007ee5d0 -> FUN_0081d1b0: false only while a download record with that id exists; empty offline), UpdateAwardProgressBindings (0x007e9f40 -> FUN_00724680 / FUN_00724690 = FUN_0071fd20(-1 / 6)). SignInFinished is now ported, so `ViewLegal.OnCancel` completes with nothing unported (the end-to-end test asserts `unported.is_empty()`). `mwtool natives`: 70 / 385 Lua natives. Not ported from the list: UpdateCachedMostWantedList (online: notification centre request), GetCurrentFreedriveEvent (FUN_0068fa10 walks the world's event tables), UpdateDeferredViewModel (FUN_007f72f0 searches the screen's view-model definitions). Next candidates: those three, the camera / feedback / sound natives of `LUA` (Offline area), the competitor getters (GetCompetitor*).

### 2026-11-17: 20 HSM hooks ported (`natives/hooks_flow.rs`, `hooks_event.rs`)

Handlers are not in `functions.csv` (vtable-only); read from `re/out/live/text.asm` (awk from the address to the next int3) with the helpers' bodies from the dump. Ported (all offline): CalculateShouldShowRecapSequence (GMM+0x2e88 := 1), ClearRecapFlag, ClearMatchmakingState (FUN_00675470), PopulateSettings / SaveSettings (FUN_0086e0a0 / FUN_0086fe20 on the settings at +0x2f3e0 of the object behind 0x117e9f4+0x2020, not G), SetNewGame (FUN_0068fd50), SetSinglePlayerRollout, SetRolloutFromProfile, IsNewGame, ClearNewGameFlag, ShouldPlayNewGameIntro, SetNewGameIntroCompleted, SetIntroEventNext, IsIntroRace, IsProgressionReady, SaveProfile, StartGameModeTimer, StopGameModeTimer, ResetGameMode, IsOnlineEvent. Each returns EXIT (0x1177ef0) or TRUE / FALSE (0x1177fe0 / 0x1177ee0). State: LogicState.game / config / game_modes / players / progression (offsets in field docs); engine work as Request variants. About 70 further hooks from a stopped pass are in register_parked (unverified, not in Registry::original(); original_with_parked() for tests). Model decisions: missing / mistyped arguments read as zero; ResetGameMode sends no telemetry record. mwtool hsm --run stands in a loaded profile, so IsProgressionReady / ShouldPlayNewGameIntro are no longer answered by hand; the boot trace is identical. Open: verify the parked hooks one by one.
### 2026-11-17: Lua natives, first 60 ported (M8 step 3; `natives/lua_{args,widgets,settings,career}.rs`)

Merged the stopped agent's WIP onto the wired registry (`Registry::set_lua`, `LogicHost::install_lua`) and
verified it. Sources: each handler in the dump (addresses in the doc comments), Lua 5.1 `lapi.c` as linked in the
exe (`lua_args.rs`: `tolstring` NULL gives `"<invalid>"`, `tonumber` + `_ftol2` for ints). Ported (60 of 385, mwtool
natives): bindings (`Resolve{Bool,Int,Double,String,WideString}Binding`), `ExitWidgetGroup`, `Set{Bool,Int,Double,
String,UniqueId}Variable`, `DeleteVariable`, `DoRpc`, `SetModalMode`, `print`, `GetPlatformName` (`"PC"`),
`SmoothStep`, `SetLanguage`, `GetInitialLanguageSelectLanguage`, conductors / sequences (6), dialogs (3), text
entry (3), settings (14 + 3 post-FX), career fields (12). State: `LogicState::{screen, widget_scripts, text_entry,
settings, post_fx, game, game_modes, ui_manager, challenges}`; engine-only effects are `Request::{WidgetRpc,
SetLanguage, CreateConductor, DestroyConductor, StartSequence, StopSequence, ShowDialog, CloseDialog,
StartTextEntry, SaveSettings}`.
- **Tests**: `natives/lua_tests.rs` (a test per area, every native called as a Lua global); `tests/lua_natives.rs`
  (shipped chunks `ViewLegal`, `Breadcrumb` through `LogicHost`). `ViewLegal.OnCancel` calls the unported
  `SignInFinished` then `ExitWidgetGroup("CANCEL")`; `ViewLegal.OnAction` without a configured binding errors in
  the script on a chunk-level nil local (context, not a native).
- **Model decisions**: `SetUniqueIdVariable` stores an int (the typed JSON has no tag 7); the script variables
  are republished as the single root `Widget` (one member per widget; the bindings' root is one name hash, the
  original resolves `Widget.<w>.<..>` through slot 8 of the script objects); `GetCurrentSafeHouse` answers 0
  and the `SetSafeHouse`-like natives only read their argument until the career state exists.
- **Open**: `SignInFinished` / `SignInStarted` (online, trivial), the profile / career natives that rebuild lists.

### 2026-11-17: native registries wired (PLAN M8 step 3, `mw_logic::natives`)

- `Registry::original()` lists the 187 HSM hooks (`FUN_007763b0`) and 385 Lua natives (`FUN_009f7e50`) with handler
  addresses; ported ones carry a fn, the rest record `"hook Name"` / `"lua Name"` in `LogicState::unported`.
  `Area` classification of the entries is still `unknown` (later).
- `LogicHost` (one `Registry` + `LogicState`) implements `hsm::Host` (hooks through `Registry::call_hook`, binding
  reads and publish / retract through `state.view_models`) and `install_lua` binds every Lua native as a global.
  `ViewModels` = published data-definition roots plus runtime-registered root providers.
- HSM data definitions are read with `mw_formats::json::Json` (engine reader, key order, int / double tags).
- `mwtool natives [--area a] [--unported]`; `mwtool hsm --run` and `mwtool lua` run on the logic host. Results
  unchanged: boot reaches "Playing" with the answer set, 115 Lua scripts run clean.

### 2026-11-17: browser audio: it plays, matches native's level, drops out while streaming
- **Start**: browsers start an `AudioContext` suspended until a gesture; the game (cpal via Bevy/rodio) creates
  its context while starting. `tools/web/index.html` keeps every context and resumes them on the first
  click / key / touch (picking the install folder counts). Headless: `--autoplay-policy=no-user-gesture-required`.
- **Measurement** (no listening): the page's `?record_audio=<s>` routes everything cpal connects to the
  speakers through a tap into a `MediaRecorder` (`audio/webm;codecs=pcm`, lossless, off the main thread) and
  POSTs it to `serve.py` (`/__audio`); `tools/web/headless <name> '<flags>' <timeout> <seconds>` runs it.
  Native reference: the same drive in `tools/hl env MW_MUTE=0 ...` (null sink only) recorded with `parecord` from
  the sink's monitor. Analysis: RMS per 0.1 s, exact-zero runs (dropouts), clicks, spectral centroid per second.
- **Result** (`--drive 911 --units 12 --auto-throttle 0.6`, 34 s after the car is placed): level and pitch
  match: median RMS 0.17 browser vs 0.059 native = the user's saved master 0.35 (the browser has no settings
  file: defaults, master 1.0); centroid 1.4–2.4 kHz in both. **Dropouts**: 42 (913 ms, longest 328 ms, 13
  clicks) in the browser, 0 natively, clustered in the first ~8 s after placement and around 32 s, i.e. while
  units stream in: cpal's WebAudio host refills two 2048-frame buffers (~93 ms) from main-thread timeouts, so
  any main-thread stall over ~46 ms (GPU uploads, mesh / image preparation, a main-thread game file read) gaps.
- **Main-thread file reads** (`WebSource` now warns on reads ≥ 16 ms): the startup index is 4,040 small reads
  (~4.4 s total, the text path); while driving only GLOBALEFFECTS (91 ms), ENVIRONMENT (102 ms) and a feedback
  group (41 ms): not the main cause of the gaps.
- **Fixed the same day** (user: patch both): `third_party/` carries rodio 0.22.2 and bevy_audio 0.19.1 through
  `[patch.crates-io]` (third_party/README.md): bevy_audio opens its sink on cpal's AudioWorklet host on wasm32 with
  atomics (falling back to WebAudio), the sink wrapped in `send_wrapper` (the worklet stream holds the page's
  `AudioContext`, not `Send`; Bevy's systems run on the main thread there); rodio's 32 lock sites on the callback
  path go through `lock_spin` (spins on `try_lock` in the threaded browser build, the ordinary lock natively).
  Same drive, 31 s after placement: **0 dropouts, 0 clicks** (was 42 / 913 ms), median RMS 0.168 (was 0.17);
  log line "audio: AudioWorklet output". Native unchanged: median RMS 0.0590 (0.0586 before), 0 dropouts;
  243 tests, accel and race tests as before. Original plan, for the record: take the mixer off the main thread with cpal's `audioworklet` host (the callback
  runs on the audio rendering thread over shared memory). Bevy's `AudioOutput` is crate-private and opens
  cpal's *default* host (always WebAudio), so this needs a patched `bevy_audio` (or our own output plugin), and
  rodio's `Player` takes plain `Mutex::lock`s on its controls (volume / speed / seek / position) every few
  samples while the main thread sets them: a contended lock traps on the worklet thread (no `Atomics.wait`
  there) as on the main thread; those have to become try-locks or atomics. Bigger buffers alone would trade the
  gaps for ~0.4 s of latency.
- Headless setup: `brave --headless=new` with `--use-angle=vulkan --disable-vulkan-surface` presented frames
  only with a profile left by an earlier run; with fresh profiles it gets no WebGPU adapter, and without
  `--disable-vulkan-surface` the swap chain's shared images fail (device lost). Brave inside `tools/hl`'s
  headless gamescope (X11, `--enable-features=Vulkan`) works from a fresh profile: `tools/web/headless`.
  Killing the launcher leaves Brave's processes running (they restored old tabs into later runs): the runner
  kills by profile.

### 2026-11-17: Binding evaluator (M8 step 1, `mw_logic::binding`, `mwtool bindings`)
The binding language's parser and evaluator read from the code (decomp.md "Binding evaluator": functions,
visitor vtables, constants); ported as `mw_logic::binding` (engine-free, wasm-checked): `Binding::parse`
(`FUN_00a50650`, HSM / Lua / most UI) and `parse_rooted` (`FUN_00a503e0`), `compile` with `{binding}` splices,
`eval_bool` / `eval_int` / `eval_float` / `eval_double` / `eval_string` / `eval_text` over a `Host` (or `Roots`:
root name → `Provider`). Sources: the dump (parsers, arithmetic, comparisons, LERP) and the snapshot's
disassembly `re/out/live/text.asm` for the vtable-only visitor methods; REA could not run (installed Ghidra 12.2,
the `rea` server accepts 12.1.x only).
- **Found in the code, not in the data**: 22 operators (ADD MINUS MULT DIV POW FLOOR_TO CEIL_TO ROUND_TO AXES MOD
  MIN MAX EQUALS GTEQ LTEQ BETWEEN HASHEQUALS NOT UPPERCASE POSTFIX IS_EMPTY LERP); LERP / BETWEEN take their
  arguments from the following op records by position (`MINVALUE` etc. are not in the exe); indices are 1-based
  (none = 1), `["key"]` hashes; the operators that apply depend on the *result type* (bool: comparisons, NOT /
  IS_EMPTY once; float / double: LERP then arithmetic; int: arithmetic; string: UPPERCASE / POSTFIX only); a
  missing value is false and only IS_EMPTY inverts it; the bool of an int is its *original* value ≠ 0 (the
  arithmetic only feeds comparisons); EQUALS on a string compares hash(upper(first 23 bytes)); op tokens are
  upper-cased and at most 23 bytes; from the first `(` on the text is one segment. `~` in a widget path means
  *absolute* (`FUN_007cb300`), not "relative to the list item" as `mw_formats::binding` assumed.
- **Verification**: `mwtool bindings` collects 5,024 distinct binding texts (JSON 2,544, GeneSys 3,503 string
  fields, HSM parameters 263, Lua constants 239; 156 whole-value `{binding}`s, 37 Lua concatenation fragments
  skipped): **all parse**. 10 unit tests on the operators against the semantics above (hashes checked against
  the AXES table in `.bss`). The warnings are data quirks the original has too: `Signals.GetRandomNumber[100]
  (LTEQ,50)` (GeneSys `EnabledBinding`: LTEQ without `:` compares with 0), `Players.LocalPlayer.GetTeam(EQUALS, 0)`
  / `(EQUALS, 1)` (both compare with 0), `LocalPlayer.TimeTilEvaded(BETWEEN(10, 19))` and
  `...IsRankingUp(IS_EMPTY(NOT))` (nested parens: no operator), `(CAPS)` ×6 (not an operator or formatter: a
  no-op), `MAINRANGE` ×4 (typo, harmless: positional).
- **Model decisions**: op records past the count read 0 (the original reads stale memory); i64 DIV / MOD by 0
  leave the value (the original faults); nesting of `{binding}` string values capped at 8; the int result of a
  string or object is 0 (the original leaves the caller's initial value); `Value::List(n)` answers `_length` in
  `Roots` (the original registers `_length` members on its list types, static initialisers at 0x00f593xx–
  0x00f797xx); the 64-bit fixed-point time type (slot 10) and vectors (only `axes()`) are not values.
- **Open**: the text widget's formatters (`SPEED`, `TIME`, `DIST`, `WITHORDINAL`, ... `.rdata` 0x00f9e484, vtable
  0x00fd82f8 first half, 0x0046xxxx) still live in `mw_formats::binding` (`hud.rs`); the root registry's
  contents (which roots are static types vs dynamic providers, the `Signals` functions); `%name%` widget
  variables (`FUN_007c83f0`); binding splices of JSON adapters (`FUN_00a4f410`, `BindingSplices`); which of the
  7 rooted-form call sites serve which data; `EnabledBindingEvaluationMode` (`Constantly` / `On Entry`, 13 JSON
  widgets) is widget-level. `mw_formats::binding` (used by `hud.rs`) differs from the original in `~`, NOT /
  IS_EMPTY counting, named LERP arguments and string EQUALS; move `hud.rs` onto `mw_logic::binding`.
### 2026-11-17: game logic: the HSM interpreter (PLAN M8 step 2)
Worktree agent. The HSMData parser completed and the original's interpreter ported (decomp.md "The HSM
interpreter", formats.md "HSMData").
- **Parser** (`mw_formats::hsm`): all official structures (burnout.wiki names): top-level states with instance
  ids, entry points with spawn entry ids, child exit points with parent exit points, typed parameters incl.
  nested STRUCTUREs, data definitions (path + JSON text), behaviour interfaces (entry / exit point ids,
  parameter defaults), fallback byte. All 37 HSMData of the install parse (`mwtool hsm --bundle all`).
- **Found**: parameter ids are not always canonical packed names: 484 `BINDING` parameters (and the exe's
  constant for EXITBINDTRUE / HSB_SEL_EX_B / HSM_B_TRUE / HSB_B_NOT_0 / HSB_EX_BI_CH) are "BINDING" NUL "PATH"
  (0x54a48941b5db2e54), while BINDINGTIMED / RUNSUBNET use the plain `BINDING`; lookups compare all 64 bits, so
  each behaviour uses its exact id. Hooks return the exit point; a missing hook gives `EXITINVALID` (also every
  instance's initial exit point). The fallback byte only matters for unregistered ids (RUN_CHILDREN →
  HSB_RUNCHILD, else HSB_PLCHLDR = exit through EXIT); a top-level state always falls back to HSB_PLCHLDR.
  Children are not ticked while their parent is entering; instant states chain within one frame (later passes
  tick the children started in the same call). The boot's instance id 20527 is G+0x2f710 (live). 5 top-level
  states name states GAMEMODES lacks (RUNSUBNET's enter requests a load when the state is missing: other HSM
  resources may hold them; not seen in the install).
- **Interpreter** (`mw_logic::hsm`): 12 machine slots, results ring, the 4-phase tick, children passes,
  transitions, entry points, subtree exit requests, data definitions published / retracted as view models, hook
  arguments from the JSON (`serde_json`). Behaviour types are a registry of factories by behaviour id; 19 ported
  (list in decomp.md); `Host` trait for bindings (typed bool / text / int), hooks, view models, graphics /
  loading-screen queries; `Clock` with the two clocks the behaviours read. 10 lifecycle tests.
- **Verified** (`mwtool hsm --run`, answers in docs/commands.md): the boot runs Top State → Boot ("Wait For
  Graphics Prepare") + Run Always Active Systems + Enable Black Background + the boot page (Language Select: the
  PC locale check exits at once → Press Start: widget group 491032, "Exit When Intro Sequence Binding Is True" →
  Press Start) + Switch to 60Hz → Post Controller Bind Boot (Autoselect and Load World: DEBUG Autoselect World →
  Load World via ENTRY; Do Profile Load: Start Loading All Data → Select Profile → Load Profile → PopulateSettings;
  Platform Selector PC → Get Online Silently → ConnectToAutolog) → InGame (Wait For Required Resources: feedback
  group 97039, vehicle list, IsProgressionReady, CalculateShouldShowRecapSequence → Setup Start Point: Is Brand
  New Profile YES → SetNewGame, SetSinglePlayerRollout → TOGAME → Should Play New Game Intro FALSE) → "Playing"
  (Wait For Message Groups, Load Message Groups, Preload Cop Cars, Switch to 30Hz mode, Listen for Invite Flow).
  Same order as decomp.md "Boot, loading screens and the free-roam start".
- **Model decisions**: missing child exit records / entry points are reported (`Trace::Missing`) and ignored
  (the original dereferences null); 10,000 passes per children update (the original has no guard); a binding
  counts as parsed when its path is non-empty (until agent A's evaluator is connected); `WAIT4TIME`'s
  `CurrentHSMTimer` view model is not published; the driver holds unported behaviours and answers unknown hooks
  with EXIT (driver only).
- **Open**: the ~180 used behaviour types left (RUN_PG_RNNR2 / ACTI_WDGT_GR / HSB_P1BIND / RUNGAMEMODE /
  GAMEMODE / RUNGAMELOGIC / LOADWORLD / LOADPROFILE / MUTE_SOUND / SETBLACKBACK / CHANGEVSYNCS first); which
  engine clocks +0x18 / +0x38 of the timer manager are; the byte at UI manager +0x2d200; the data-definition
  `adapters` member (hash 0xb21d9989) and binding writes into published view models; connecting the binding
  evaluator (typed reads `FUN_00a4f160` bool, `FUN_00a4f2e0` text, `FUN_00a4f200` int, the parse context
  machine +0xe8) and the native registry for hooks. A probe hook on `FUN_00776470` logging (state id, phase) in
  the original would confirm the boot path frame by frame.
### 2026-11-17: Lua 5.1 VM (`mw-lua`): the UI's 115 chunks run (PLAN M8 step 5)
Worktree agent. C = confirmed (dump / data / differential test), I = inferred.
- **Decision: our own interpreter, not `rilua`.** rilua 0.1.27 (MIT / Apache, single author, the WoW-addon
  variant of 5.1.1, ~49k lines with compiler, GC, io / os, taint): its undump hard-codes an 8-byte `size_t` (the
  game's chunks have 4), host functions are plain `fn(&mut LuaState)` pointers (no captured state, so no
  name → closure natives), MSRV 1.92, and table order / number output are not the game's. Ours is ~4k lines with
  no compiler (everything ships as bytecode) and owns the details fidelity needs. The chunks use little: no
  SELF, 3 TFORLOOP, 1 POW; `string.sub/gsub/find/len`, `table.insert/remove`, `math.floor/ceil/sin/pi`,
  `tostring`, `tonumber`, `pairs`, `print`, `error` (C, opcode / GETGLOBAL scan).
- **What follows the original** (crate docs): `lundump.c` (size_t 4 or 8 accepted), the 38 opcodes of
  `lvm.c`, `ltable.c` ported (main positions, `luaS_hash`, `hashnum`, rehash sizes, re-insertion order,
  `luaH_getn`, keys created by `t[k] = nil`), so `pairs` order matches; `lstrlib.c` matcher and `format`;
  `ltablib.c` quicksort; 5.1.3 message texts and `ldebug.c` variable names (`?:0: attempt to call global 'X'
  (a nil value)`: stripped chunks, source `=?`, line 0); `luaL_where` positions only with line info (never in
  the game's chunks). Natives are global closures receiving their registered name (`FUN_009f86d0`).
- **The game's CRT and libs** (C, `.rdata` of the dump): `"%.14g"` at 0x01084474; pre-2015 MSVC CRT strings
  `1#INF` / `1#QNAN` / `1#IND` at 0x0109f3c8.. → numbers print as `1e+020`, `1.#INF`, `-1.#IND`;
  `"table contains non-strings"` at 0x01084574 is the 5.1.3 text (5.1.5 changed it; the host-diff test avoids
  it). `lua_tointeger` rounds to nearest-even (luaconf.h's `fistp` path for `_MSC_VER` x86, I: build flags not
  seen), `%d` truncates, out of range = 0x80000000; `math.random` = MSVC `rand()` (its state is shared with the
  game's C++, so the draw order is not reproducible).
- **How the game calls scripts** (C, dump): `FUN_009ef230` = `lua_getfield(globals, <script>)`,
  `lua_getfield(-1, <callback>)`, flag "is a function"; `FUN_009ef2e0` = `lua_pcall(L, n, MULTRET, 0)` and on
  error **pops the message without logging it**: script errors are silent. `FUN_007ec680`: `Construct(<light
  userdata = the script object>)`, then per JSON config item `SetConfigurationItem(name, value)` with value a
  string, an integer (`FUN_00ec0380` = `lua_pushinteger`), a boolean, or a double pushed as `(double)(float)v`
  (a float round trip). `mw_lua::Lua::call_field` does the lookup + call and returns the error (the host should
  drop it like the game).
- **Natives**: `print` is a native (`FUN_007e9920`: `lua_tolstring(L, 1)`, nothing else, no error for any
  argument), registered after the libraries, so it replaces base `print`. `ExitWidgetGroup` (`FUN_007f22e0`,
  argument 1 as a string, `"<invalid>"` otherwise) is registered with `FUN_009f7e50` too but missing from the
  385-name list (`scratch/scripting/reg_9f7e50.json`); the dump's 216 `FUN_009f7e50("…")` calls add only it:
  **386 natives**. The chunks read **171** of them (most used: `print` 70 scripts, `ResolveIntBinding` 69,
  `ExitWidgetGroup` 55, `ResolveBoolBinding` 48, `ResolveStringBinding` 43, `SetBoolVariable` 36,
  `SetIntVariable` 35, `SetStringVariable` 28, `DoRpc` 22, `WriteStringToOnlineSession` 19,
  `UpdateDeferredViewModel` 17). Read but not registered (nil in the game): `ToggleOfflineEventFilter`,
  `MarkAutologAlertAsSeen`, `GoBackEasyGuide`, `SaveCustomRollouts`, `SelectCustomRollout`, `SetRolloutColour`,
  `Change{Weapon,Package,UpgradePackage}OnCustomRollout`, `SetNo{EMail,Password}AsLastError`, plus script
  variables (`lsAction`, `liNumItems`, `liRecommendIndex`, `lsCategoryBinding`) and two other scripts' tables.
- **Scripts share a state** (I): `CarSelectMW` calls `CarSelectMenu.Back(...)` and `EditSpeedlists` reads
  `Breadcrumb`, both other scripts' global tables, so at least a screen's scripts live in one Lua state.
- **Verified**: `cargo test -p mw-lua`: 9 differential scripts compiled by the host `luac5.1` give the same
  output as `lua5.1` (numbers, coercion, patterns, `format`, `pairs` order with deletions and rehashes, table
  library and `sort`, closures / varargs / the `arg` table, error texts with variable names, metatables); MSVC
  formatting unit tests; on the install, all 115 chunks load, their names hash to their ids, main + `Construct`
  run, and `ViewLegal.OnAction` calls `ResolveStringBinding` then `ExitWidgetGroup("TOS" / "PP")`.
  `mwtool lua --natives <386 names>`: main + `Construct` of all 115 without error (9 natives called);
  `--call all` (every callback, no arguments) reaches 109 natives, every error comes from the missing context
  (nil arguments, globals another callback sets, the unregistered names above), none from the VM.
  Per-script table: `mwtool lua --natives <list> --call all --out <tsv>` (script, native, calls, first
  arguments; `nil` rows = not registered); the list is the union of `reg_9f7e50.json` and the dump's calls.
- **Model decisions**: tables / functions hash and print (`table: 0000002A`) by a per-state counter, not an
  address; at most 200 nested calls (LUAI_MAXCCALLS; the game allows deeper Lua recursion); memory is
  reference counted, cycles live until the state is dropped (the game stops its collector,
  `lua_gc(STOP)`); `collectgarbage` answers 0. Not provided (no chunk uses them): coroutines, `io`, `os`,
  `package`, `load*` / `dofile`, `getfenv` / `setfenv`, `string.dump`, weak tables.
- **Open**: one state for all scripts or one per screen (the state list of `FUN_009f86d0`), and when the game
  steps the collector; the arguments of `OnActivated`, `OnDeactivating`, `PreInputUpdate` and action callbacks
  (`OnAction` gets something `ResolveStringBinding` resolves); porting the 171 natives (mw-logic registry).
### 2026-11-17: JSON widget definitions and screen scripts, typed (M8 step 4, data side)
Worktree agent. Code: `mw_formats::json` (the engine's JSON reader), `mw_formats::widget` (typed definitions,
naming, the native screen-script table), `mw_assets::widgets` (screen → widget groups → widgets → JSON → scripts;
adapter hosts), `mwtool widgets`. Spec: decomp.md "Widget definitions (JSON) and widget scripts"; format:
formats.md "Widget definitions (JSON `TextFile`s)".
- **Result** (`mwtool widgets`, exit 0): 859 unique JSON files; 846 named by the 94 screens' `Widget_*` structs
  (CRC-32 of `"<Name>_<UniqueId>.json"`), all parse; the other 13 (`EN_US/FEEDBACKGROUPS`) are the adapters of
  `SequenceItems.AdapterHost` sequence items (referenced by handle). Scripts: 419 Lua uses of 115 chunks (every
  chunk of the install, none missing), 181 native uses of 22 names, 0 unresolved; the ScreenScript objects'
  `LuaAsset` equals the JSON's everywhere; `LayoutLayer` / `ModalMode` names map 1:1 to the GeneSys enum values on
  all 1,876 screen widgets (7 layers: `MODAL_BACKGROUND` 4 was not known).
- **Keys = GeneSys field names** (C): the JSON keys hash (GeneSys field hash) to the twin struct's field hashes,
  so the 400 distinct keys name 400 GeneSys fields (269 not in `genesys_names.txt`). Not added to the names
  file (it holds names found by dictionary attack; a bulk copy of the data's keys is closer to extracted data):
  open whether to read them from the user's install at run time.
- **Untyped (listed set)**: 18 widget types keep their own fields in `WidgetDef::fields` (online / Autolog / login /
  report / store and social blades / driver details / notification tabs / player points); `mwtool widgets`
  prints them per type. Everything else is typed, with a hard error on any unknown key inside typed records.
- **Native screen scripts** (C, snapshot `snap1`): factory table at manager +0x308 (count +0x408 = 24), the
  manager at 0x24473560 in that snapshot; each entry {`field_hash(name)`, factory}; each factory sits next to a
  registration stub returning (hash of the name string, factory). 24 = the 22 used + `RearViewMirror` and
  `MWL_VITA`. Names, factories and sizes in `NATIVE_SCREEN_SCRIPTS`.
- **Corrections to the research entry below**: `LuaHooks` *is* used at run time: `FUN_007f84c0` binds each listed
  name into the widget's Lua state (`FUN_009f1c30`; non-natives other than `tonumber` / `tostring` / `error` /
  `pairs` / `mfpFunction` set the error flag); `LuaModulesIncluded` picks "open the listed `LuaModules`" vs "open
  all five". One Lua state per widget (widget +0x84). Actions are read from the GeneSys `UserActionInstance`, not
  the JSON (the JSON key hashes for them are never referenced). `IsCallbackInLua` = "offer to the widget's
  scripts" (native scripts too); `"Widget::Function"` callbacks go to another widget's scripts.
- **Model decisions**: none in behaviour (no runtime yet). Parser choices: the JSON reader is our own port of the
  engine's (file-ordered objects, int vs double tags, first-match lookup), not serde_json, because
  `SetConfigurationItem` order and the int / double split are observable by the scripts; `lua_configuration()`
  rounds doubles through f32 as `FUN_007ec680` does.
- **Open**: the Lua init's two unnamed keys (hashes `67d01e38` bool, `cc2e0dc5` binding; absent from the data);
  vtable slots 7, 9–11; callers of `OnActivated` / `OnDeactivating` (`FUN_007ebc80` / `FUN_007ebcb0`); where
  `Widget::Function` is split (record +0x104); the release-build effect of the error flag +0x2028 (39 `LuaHooks`
  names are not natives); when the widget's Lua state is created; per-native behaviour (24 C++ classes);
  `RearViewMirror` / `MWL_VITA` never used by the shipped screens.

### 2026-11-17: game logic: scripting, flow, AI (survey of the install, the exe and prior work)
Research agent; full tables, registries and URLs in the main checkout's git-ignored
`scratch/engine-scripting-research.md` (raw outputs and scripts in `scratch/scripting/`). Whole install scanned
(2,020 bundles, 353,223 resources). C = confirmed, I = inferred.
- **One script VM: PUC-Rio Lua 5.1.3** (version string at 0x01083c3e), standard 38 opcodes, 32-bit size_t,
  double numbers, debug info stripped (C). No Squirrel, Scaleform / ActionScript or RAGE-style custom VM; the
  other "VMs" are C++ interpreters of data: HSM, bindings, the sequence player, PageRunner.
- **Lua = UI only**: 115 unique chunks (229 copies, ~25k instructions, 1,103 functions), `TextFile` (0x70) in
  `UI/SCREENS2`; id = CRC-32 of the lower-cased `"<ScriptName>.lua"` (all 115 match). Menus, map, car select,
  settings, online / store. No game-mode, AI or physics Lua (C).
- **JSON** `TextFile`s: 859 unique widget definitions (439 Lua callbacks, 224 native actions, binding adapters,
  script configuration); id = CRC-32 of `"<WidgetName>_<UniqueId>.json"` lower-cased (`FUN_007edd50`,
  `FUN_007e0850`) (C).
- **HSMData**: 17 unique machines: GAMEMODES' game-logic machine (2,706 nodes, 36 entry points, 199 behaviour
  types) and 16 active-aero machines (C). Node +0x14 is a data definition, not Lua (formats.md corrected).
- **GeneSys**: 1,138 type names, 83,655 objects; 8,485 `Sequence`s using 73 `*SequenceItem` types; 243
  `*_ScreenScript` objects (`LuaAsset`, `LuaHooks`, `LuaModules`, `LuaModulesIncluded` cracked); 24 types with
  no chunk = native C++ screen scripts (C). `LuaCode`, `LuaData`, AttribSys, BinaryFile, FlaptFile: 0 resources.
- **Bindings**: ≥2,701 distinct paths in JSON, Lua and HSM, ~100 roots (`Widget`, `Signals`, `Mode`,
  `Players`, …); operators seen EQUALS, GTEQ, LTEQ, MINUS, ADD, MIN, BETWEEN, NOT (C syntax, I full grammar).
- **Lua host** `FUN_009f86d0`: base + optional package / table / io / string / math (module table 0x0102a1fc),
  GC pause 100 / stepmul 200 then **GC stopped**; every native registered as a global closure with its name as
  upvalue (`LuaHooks` is not a runtime whitelist, I). Natives: `FUN_009f7e50` registers **385** Lua natives;
  `FUN_007763b0` **187** HSM hooks keyed by name hash (35 names in both). Widget scripts: factory
  `FUN_007f2850` (Lua script object vtable 0x00fe7f78, or one of up to 32 native screen scripts);
  `FUN_007ec680` calls `Construct` then `SetConfigurationItem` per item; other callbacks by name (I).
- **HSM never calls Lua**: it calls natives through `CALLHSMHOOK` / `CALLHSMHOOKS` / `HSMHOOKEXIT` (121 hook
  names used), arguments from the node's data definition. The exe registers 268 behaviour types (203 used).
- **Flow vs rules**: flow = the HSM; content = GeneSys (111 `OfflineEvent`, 12 `GameMode`, `EventList`,
  `AIPlayerInformation`, `ScoringAction`); cutscenes = sequences run by `RUNGAMELOGIC` nodes; menus = JSON
  widgets + Lua + native screen scripts; rules (checkpoints, positions, timers, scoring, rewards, unlocks) are
  C++ reached through hooks (`StartGameModeTimer`, `AwardPostEventAwards`, `RemoveAllCops`, …).
- **AI**: racers, cops, traffic are C++ state machines; no HSM / Lua / behaviour-tree data for AI (C); tuned by
  `AIConfig`, 200 `AIPlayerType` (`Behaviour` picks the state), 40 `HeatLevel` → `PursuitBehaviour` →
  `CopBehaviour`, `CopType`, `RoadblockRules`, `TrafficManoeuvreConfig`. HSM only pauses / preloads / removes AI.
- **Prior work**: burnout.wiki "HSM Data/NFS Most Wanted" (official field names; high value); BNDL Explorer
  (github.com/FrankHUN88453/BNDLExplorer: MW2012 editor, JSON / Lua text editors, HSM viewer; the PS3
  prototype's debug data names ~3/4 of GeneSys field hashes; licence to check, format knowledge only); DGIorio's
  tools (bundles, `HashBreaker`); nothing from GDC; no other reimplementation found.
- About 15 functions the scripts call do not exist in the exe (`ToggleOfflineEventFilter`,
  `SaveCustomRollouts`): dead or online paths (I).

Consequence (user, 2026-11-17: a 1:1 rewrite): CLAUDE.md "Fidelity rule" and PLAN.md M8: the logic data is
executed by ports of these interpreters. Open: what `HSB_STRSCRPT` runs; Lua state per screen or per script, GC
and error handling; the second HSM-style registry at 0x54d (RUNSEQUENCE / EXITBINDTRUE / EnableCompound);
`NavigationMesh` and `GameplayLinkData`; the full binding grammar from the evaluator; the offline / online split
of the natives.

### 2026-11-17: browser port, step 4: the runtime renders in a browser (WebGPU, free camera)
`mostrusted` builds for wasm32 with threads (`tools/web/build`: nightly, std with atomics, wasm-bindgen 0.2.129;
153 MB module without debug info, not yet optimised with wasm-opt) and runs in headless Brave on WebGPU:
`--camera=-1131,27,1627,-1190,15,1720 --units 4 --hour 13` loads the same 4 track units, 2881 models, 690 textures,
3788 instances as natively and renders the road at Hawaii's bridge (frame POSTed by `--screenshot`; matches the
native frame up to the canvas framing). What it took:
- `main()` split: native `main` locates the install and calls `run(cli, game)`; in a browser `web.rs` exports
  `start_with_files(FileList, args)` (the folder picked on the page) and `start_with_listing(base, listing, args)`
  (a served install: `tools/web/serve.py` serves the player's own install over loopback with HTTP ranges, the
  build, and the COOP / COEP headers). Flags are the native ones, from the page's `?args=` (`+` = space).
  `WebSource::from_urls` serves both; the base URL must be absolute (worker threads run from `blob:` scripts).
- Compiled out of the browser build: hidapi / `steam_controller.rs` (the Gamepad API covers pads), the SDL
  gamepad-mapping environment write (`set_var` panics there). Window = the page's `#mostrusted` canvas.
  `--screenshot` POSTs a PNG to the server (`platform::save_screenshot`). Bevy's `webgpu` feature.
- **`Layered` material within WebGPU's 16 samplers per stage**: its 12 texture / sampler pairs were visible to
  vertex and fragment (the vertex stage counted 18 with StandardMaterial's 6), and the fragment stage had 18.
  Now fragment-only, and two samplers: every game texture already had the same sampler
  (`convert::game_sampler`), bound once on `tex0` (a missing first texture is a white 1×1 image carrying that
  sampler, as Bevy's fallback is white); the planar reflection reads with the sky cube's sampler (both linear,
  clamp). Native frames unchanged: before vs after differs no more than two runs of the same build (2.5-3.3 %
  vs 3.0 % of pixels, traffic / particles).
- Settings, profile and saves: no config directory in a browser, so defaults and nothing saved (as before
  with no `$HOME`); browser storage is open.

Later the same day: the worker panic was the loader's `Receiver::recv_timeout` (std reads `Instant` inside):
`platform::recv_timeout` polls in a browser (`try_recv`, 1 ms worker sleeps = atomic waits, no clock); our
threads now set `Error.stackTraceLimit` (platform's `Builder` wraps wasm_thread's), which named it. Clean run
after that; **driving works in the browser** (`--drive 911 --units 12 --auto-throttle 0.4`: car, HUD, Easydrive,
minimap, streaming; slower per frame than native: 35 vs 47 km/h at frame 400). Open: audio needs a user gesture (headless runs are muted);
`AssetStore::new` indexes 2020 TOCs on the page's main thread (text path, a few seconds); the window size on the
page vs `--units` / memory (4 GB cap) for larger worlds; driving (step 5).

### 2026-11-17: browser port, step 3: threads as Web Workers, a main thread that never blocks
Decision: the browser build uses **shared-memory threads** (wasm32 with `atomics`, nightly `-Z build-std`, COOP /
COEP headers). Without threads every loader would have to run in a separate worker instance and send its results
as messages (the browser has no synchronous file reads on the page's main thread), a rewrite of every load path.
Bevy 0.19 supports atomics builds (bevy_render wraps its wgpu handles with `send_wrapper` there); its own task
pools stay single-threaded on the web, our loader threads don't.

- `mostrusted/src/platform.rs`: `thread::{spawn, Builder}` = `std::thread` natively, `wasm_thread` (0.3.3, same
  API, each thread a Web Worker on the shared memory) in a browser; all 26 `spawn` and 16 `Builder` sites use it
  (`steam_controller.rs` and `movie.rs` stay `std`: native-only, hidapi / ffmpeg). `PollLock::poll_lock` (a
  `try_lock`; busy = "not yet") for every main-thread read of a worker's result slot (24 `take()`s, the
  `is_some` / `clone` / `retain` polls in sequence_fx, world_coronas, music, world_sfx): a contended lock **traps**
  on a browser's main thread (`memory.atomic.wait32` is not allowed there). `StreamFocus`: the focus is polled
  (a busy lock skips one frame), releases go through an mpsc channel (a skipped release would be lost).
  `Instant` = `bevy::platform::time::Instant` (`std::time::Instant::now` panics in a browser; same type
  natively), seeds from `platform::time_seed` (`Date.now()` there). The bake pool falls back to the global
  rayon pool when threads can't be made; rayon-core 1.13's global pool falls back to the calling thread itself.
- `WebSource` changed from `FileReaderSync` (worker-only, `File` handles tied to one thread, `unsafe impl Send`)
  to **blob URLs + synchronous range `XMLHttpRequest`s**: the source is plain strings, `Send + Sync` without
  unsafe, readable from any worker; on the page's main thread binary sync requests are not allowed, so it reads
  `x-user-defined` text and takes the low byte of each char (works, slower).
- Builds: `tools/wasm-check --threads` (nightly, rust-src; the linker flags `--shared-memory --import-memory
  --max-memory=4294967296` and the `__wasm_init_tls` / `__tls_*` exports are required: without them the memory is
  not shared and posting it to a worker fails with `DataCloneError: #<Memory> could not be cloned`).

Verified natively: clippy 0, 201 tests, 911 accel unchanged (0-60 mph 3.58 s, quarter mile 11.60 s),
`--race 462174 --test race` 27/27 gates. In headless Brave (scratch wasm-bindgen module, `--target web`,
COOP / COEP served, `crossOriginIsolated` true): `WebSource` made on the page from 7 install files, `GameDir`
validated on the main thread (VERSION.TXT and a 1 MiB range of VEH_122769_HI through the text path, 23 ms); a
`wasm_thread` loader worker read the same range (identical FNV hash), built the `AssetStore` (4406 resources,
9 ms), then four workers loaded and parsed GLOBALEFFECTS / VEH_122769_HI / GLOBALTEXTUREDICTIONARY / SHADERS
at the same time through the shared store with the native counts and 0 errors (16-58 ms each); the page polled
the results with `try_lock` only.

Open for step 4: `mostrusted` itself on wasm32 (hidapi and ffmpeg compiled out, settings / profile / saves into
browser storage, the debug trace files off, the page / worker bootstrap); main-thread locks in the engine-free
crates (`AssetStore`'s bundle cache) are still blocking if the main thread loads: loads belong on workers.

### 2026-11-17: browser port, step 2: game files read through `mw_gamedir::Source`
Every game-data read in the engine-free crates now goes through `GameDir` (`read`, `read_at`, `len`, `resolve`)
over a `Source` trait (`mw-gamedir/src/source.rs`): `DirSource` (disk, native), `WebSource` (wasm32: the folder
the player picks with `<input webkitdirectory>`, `File` handles read on demand with `FileReaderSync`), `MemSource`
(tests). Moved onto it: `AssetStore::new` (TOCs via the new `Bundle::read_toc_with`), the store's bundle opens,
`instance_positions` (traffic_lamps.rs), SPS music streams (music.rs; the index now holds game-relative paths) and
the police speech container (`SpeechData::container` = the `GameDir` and the SWC0's path, read with `read_at`).
`GameDir::path` / `root` stay for native users that hand paths to other programs (mwtool, the ffmpeg movie
decoder): on a browser source they return `None` / an empty path, so movies are off there. The install root
inside a picked folder is the shallowest folder holding `SHADERS.BNDL` (picking the install or a parent works).

Constraint: `FileReaderSync` exists only in Web Workers, so in the browser the loads must run in a worker
(the main thread gets errors). `WebSource` is `Send + Sync` only without the `atomics` target feature (one
thread); a threaded build (step 3) has to keep the handles in the loading worker. The other `std::fs` users in
`mostrusted` are settings / profile / save files and debug traces: browser storage, step 4.

Verified: native unchanged (clippy 0, 201 tests, `mwtool verify` 0 failures over 2020 bundles, 911 accel
0-60 mph 3.58 s / quarter mile 11.60 s, `mwtool speech 1186352` decodes its 144 lines, `mwtool music --wave
010000000005dc99` 44.38 s). In a browser: a scratch wasm-bindgen 0.2.129 module in a dedicated worker in headless
Brave, fed `File`s of 7 install files under `Need for Speed(TM) Most Wanted/` plus one outside it: `GameDir`
validates (version 1.4.0.0), any-case resolve and `read_at` work (EOF error past the end), `AssetStore::new`
indexes 4406 resources (rayon falls back to the calling thread), and GLOBALEFFECTS / VEH_122769_HI /
GLOBALTEXTUREDICTIONARY load and parse with the native counts (521/129/12, 831/28/21, 33/33/0), 0 errors.
`FileReaderSync` is absent on the page's main thread, as expected.

### 2026-11-17: browser port, step 1: the engine-free crates build and run as wasm32
Question from the user: can the game run in a browser? Plan agreed: (1) engine-free crates on
`wasm32-unknown-unknown`, (2) file reads behind a trait (native `std::fs` / browser `File` handles from a
user-picked install folder: the data is never hosted), (3) the ~17 `std::thread::spawn` sites and rayon onto a
task-pool helper, (4) `mostrusted` on WebGPU behind a `web` feature (free camera first, no audio), (5) driving,
then audio. The native build stays the reference for captures and comparisons.

Step 1 result: `mw-gamedir`, `mw-bundle`, `mw-formats`, `mw-audio`, `mw-assets`, `mw-handling` compile for
wasm32 with no source changes (`tools/wasm-check`). A throwaway raw-ABI module (scratchpad, not committed) ran in
Node 24 on real bundles: `Bundle::from_bytes`, `load` of every resource (decompression via flate2/miniz),
`Texture::parse` and `Renderable::parse`. Results identical to the same code built natively: GLOBALEFFECTS 521
resources / 129 textures / 12 renderables, VEH_122769_HI 831 / 28 / 21, GLOBALTEXTUREDICTIONARY 33 / 33 / 0,
SHADERS 3099, HAWAII/GLOBALRESOURCES 549 / 261 / 101; zero load or parse errors. Speed: wasm 1.4-1.6x the native
time (GLOBALRESOURCES 607 ms against 390 ms, single thread both).

Compiles but fails at runtime on wasm32-unknown-unknown (input for step 2): every `std::fs` call
(`mw-gamedir` install detection and `read_dir` case lookup, `Bundle::open` / `read_toc`, `mw-assets` `music.rs`
and `police_speech.rs` partial reads with `File::open` + seek). `std::env::var*` returns nothing there (the
`MW_*` switches are simply off). `AssetStore::new` uses rayon `par_iter` over every bundle's TOC: rayon-core 1.13
falls back to the calling thread on wasm. Runtime-only blockers kept for later steps: threads in `mostrusted`,
the 4 GB wasm32 memory cap (the free camera still loads the whole world, ~4.5 GB GPU), `hidapi` (Steam
Controller) to be compiled out, storage buffers in `game_lighting.wgsl` need WebGPU (no WebGL2).

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
  shows the gap.) Answered 2026-10-16: zone 13 has no river (unmodelled block interiors); every zone lists the
  water it shows.
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
  `Coronas`. Contact-driven ones (impact, contact and spot effects, lights off) are ported since 2026-10-16
  (`prop_vfx.rs`); ambient ones that are on from the start (power-plant smoke, "AmbFX Dummy") are not. The
  exhaust effects are the cars' spot effects (2026-10-09).

---

### 2026-11-18: Lua natives, fifth batch: state-field getters and setters (115 of 385; `natives/lua_sim.rs`)
Handlers are not in the decompiled dump (`fn` says "not a function start"); read from `re/out/live/text.asm` at the
address `mwtool natives` lists. Ported: GetFrameCount, GetSimTime, GetSimTimeStep (the sim clock record =
`FUN_0046ee10` on `*0x010edacc`, `*(+4)+0x38`: int +0, i64 +8 x the float 3.33333e-7 (.rdata 0xf9f608), floats
+0x14 x +0x10 while byte +0x1c), GetLocalPlayerCompetitorIndex (PM+0xa6b4), IsCompetitorActive (PM+0xa5d0[i] != 0),
GetCompetitorTeam (record i x 0x7f0 + 0x268; new `Player::team`), GetCameraTarget (`FUN_006bf100`: +0xa6bc when set
and occupied, else +0xa6b4), ResetCameraTarget (`FUN_006cab10`(PM, -1): +0xa6bc = -1), GetUseOverheadCamera (camera
manager +0x36de), GetCameraFov (+0x150 x 57.2958), SetCameraFov / ResetCameraFov (camera message 0xf via
`FUN_00843ca0`(0, set, fov): `Request::CameraFov`), AreUnitsMetric (UIM+0x2cf28), ShouldLoadingScreenBeShowing
(UIM+0x2d204 != 0 && +0x2d200 == 0 = `Engine::loading_screen_showing`), SetCarFrozen (PM+0xcba8 = 1.0 / 0.0; new
`PlayerManager::car_frozen`). Model decisions: out-of-range competitor indexes read as empty (the original indexes
raw memory); the clock, camera and units fields are set by the runtime (`SimFields`), not yet fed.
Skipped (need a callee not yet read): GetCompetitorName (`FUN_006bf2d0` + string copy), GetCompetitorType (table
0x10c5414 by player kind), IsLoadingScreenShowing (`FUN_00799d90`), IsHudMessageQueueEmpty (vtable call),
SetUseOverheadCamera (message 0xd, unverified argument order).

### 2026-11-18: Lua natives, sixth batch: camera messages, buttons, sound stops (135 of 385; `natives/lua_sim.rs`)
Handlers read from `re/out/live/text.asm`. Ported: SetMinimumCollisionDamage (argument check only, no effect),
GetCompetitorType (player +0x9c indexes the string table 0x010c5414: 0 LocalPlayer, 1 Network, 2 AI; later entries
are unrelated strings, model decision: empty), SetUseOverheadCamera (camera message 0xd: `Request::CameraOverhead`),
SetUseFinishLineCamera (message 0xe, id = arg 2 when on, else -1), SnapCameraToBehindCar (message 1),
AllowCameraSwitching (camera system +8), EnableLocalRaceCarRumble (message 9), IsExitLoadingScreenButtonPressed /
IsSkipCutsceneButtonPressed (input state word +0x1b4 == 2), IsSpectatorCameraButtonPressed (+0x19c == 2),
SetCarLocked / DisableMenuControls (bit 1 of mask word 1 / 2 at G+0x2f0a4 by `FUN_0083c960` / `FUN_0083c930`, the
byte +0x1c+word counts the changed words), EnableNitrousBalancing (G+0x291f8, true without an argument),
StopAllSounds / StopSpeech / StopGameplayStream (`FUN_007a2140` on UIM+0x2d2f0 with kind 0 / 2 / 4;
`Request::StopSounds`), StopMusic, StopUnwantedSound, ClearAllFeedbackQueues / ClearAllNonBackgroundFeedbackQueues
(`FUN_007b3410`, then `FUN_007b1120` / `FUN_007b1150`(~2) on UIM+0x30840; `Request::ClearFeedbackQueues`). The
requests only record the call; the sound, feedback and camera systems behind them are the runtime's.
Skipped: GetCompetitorName (string copy), IsLoadingScreenShowing (`FUN_00792090` + vtable +0x14),
IsHudMessageQueueEmpty (vtable +0x10), ClearLoadingScreenLayout (needs the UIM+0x2d204 mask, not modelled),
StopUsingIceCamera / SetIceCameraPaused (guarded messages via `FUN_0084c110`), GetIceTakeDuration (`FUN_0083b730`),
IsVehicleCop (`FUN_0083be20`), InvalidateRaceTarget (`FUN_00843170`), ClearAllChevrons (`FUN_006465a0` + three
messages), SetCarStill.

### 2026-11-18: Lua natives, seventh batch: no-ops, traffic messages, flags, sign-in errors (155 of 385; `natives/lua_sim.rs`)
Handlers read from `re/out/live/text.asm`. Ported: eight argument-check-only handlers (SetHeadlightsState,
ShowX360FriendsPopup, ShowPartyInvitePopup, OnPhotoSharedToWall, UpdateSpammableFriends, ShowX360GamerCard,
ShowX360GamerCardEasyDrive, ShowX360GamerCardByPlayerIndex), IsFriendsUiOnScreen (pushes false), StopTraffic /
ResetTraffic / StartTraffic (`FUN_008461b0` / `FUN_00846240` / `FUN_00846270`: message 6 / 8 / 0xa to the object at
`*0x010edb30`+0x10; `Request::TrafficMessage`), IsStoryMode (GMM+0x2e7a), IsChallengeActive (GMM+0x2ce0 != 0),
SetRearViewMirrorRender (G+0x2efb5), SetTOSAsLastError / SetCancelAsLastError / SetPWMisMatchAsLastError /
SetEmailMisMatchAsLastError (`FUN_00883690` / `8836c0` / `8836f0` / `883720` on the sign-in VM+0x10d0: code
+0x135c = 2 / 3 / 0x1a / 0x1e, +0x1360 = -1, bytes +0x139c and +0x136d cleared; `SimFields::sign_in_error`),
GetSignInError (the code, VM +0x242c). The traffic system and the sign-in flow are not ported, so the records only
hold what the natives write.
Skipped: SpamFriend, ShowX360KinectTunerUi (same shape as the no-ops, not needed yet), ClearEntitlementJumpID
(`FUN_0086d610`: field +0xdbc of the object at *(G+0x2f3e0)+0x4ce0, not modelled), StoreLoaded / StoreUnloaded
(`FUN_0086f7f0` unread), OnWallPostOrComment (`FUN_00639a90`), SendAddedRecoFriendTelemetry (`FUN_007e8d00`),
HasFinishedStreamingCameraWorld (`FUN_005470e0` streaming state), the nitrous natives (`FUN_00658ad0` and a
competitor-to-slot lookup unread).

### 2026-11-18: Lua natives, eighth batch: network timer, online flags, race-car messages (175 of 385; `natives/lua_sim.rs`)
Handlers read from `re/out/live/text.asm`. Ported: six argument-check-only handlers (FlagUseMaxTriCacheSize,
ToggleUnlockAllVehicles, ViewInvites, SetUseDebugCamera, SetMatchMakingState, DebugPrint), IsRemoteCompetitor (slot
kind +0x9c == 1, `FUN_006c3750`), SetRaceCarInvulnerable / SetRaceCarDamageAllowed (message 0x1f / 0x1e with
competitor and flag to `*0x010edb2c`+0x10; `Request::RaceCarFlag`), EnableNitrousEarning (G+0x291f0),
SetSpammable (sign-in VM bytes +0x2438 / +0x2439 / +0x243a = args 1 / 2 / 3), IsNetworkTimerSyncing / Synced
(clock manager +4 -> +0x6c == 1 / 2), IsOnline (online manager +4 -> +0x6a20 != 0), IsInOnlineEvent (byte +0x6b of
the current event record), GetPlayerVehicleIndex (arg when exactly one, else the local slot; player +0xa0),
SetPreEventSpeedwallPos (G'+0x2f6fc), SetTrafficDensity (`Request::TrafficDensity`, float read only when on),
RefreshGarageItemsCache, UnregisterSpeechCommand (requests). Model decisions: an empty slot reads false / -1;
the online / timer / event fields are set by the runtime (`SimFields`), not fed yet.
Skipped (callee unread or state not modelled): IsVehicleActive / IsVehiclePhysical / IsVehicleLoaded (world vehicle
tables `FUN_00573390`, `FUN_0083bde0`), SetEngineState, SetSirenState, SetInfiniteNitrous (`FUN_00658c90`),
IsCarWrecked (`FUN_006ba280`), SetCarStill (the false branch unread), GetNumOnlinePlayers (`FUN_00a83640`),
SetEMail / SetOriginID (string copy), IsSomethingDownloading, ClearTargetRoute (`FUN_00792390` x3), DisconnectFromServer
(`FUN_0083dc00`), SetFriendInviteId / SetJoinPlayerId (string -> i64 `FUN_00d753c0`), SetRecommendationTimeToBeat,
SetRolloutSweetRevenge (`FUN_0068ba20`), SaveControllerConfiguration, HasEasyDriveOptionBeenSeen, SetExitPointID.
