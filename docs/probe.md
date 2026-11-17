# The probe: running our code inside the original game

Design for measuring, and later experimenting on, the running original with code that executes inside its
process. Written 2026-10-04 after the user decided that hooks in their own licensed, running copy are in scope
(policy: [reverse-engineering.md](reverse-engineering.md), "Code inside the running game"). Status is tracked
in [PLAN.md](PLAN.md) (M3.2 and "Testing").

## Status (2026-10-05; 2026-10-19)

- **2026-10-19: hooks without code patching.** The probe now runs code on the game's own thread through two
  vtable slots (pointer swaps in `.rdata`, checked and restored; "Capture helpers" below): the race car's tick
  and the free-drive behaviour's frame. That covers what the capture helpers need (teleport, car swap, scripted
  inputs, fixed conditions, a per-tick log); the instruction-detour installer of Tier 1 was not needed and is
  still not written. Hooks inside functions without a vtable entry would still need it.

- **Tier 0 built and verified**: `mwtool memscan` has `poke`, `unpoke`, `pokes`; `telemetry` and `capture`
  take `--poke name=value` (restored after the run). Writes go through `/proc/<pid>/mem` and are refused outside
  writable, non-executable mappings below 4 GiB (a poke at the tick function is refused; the drag constant at
  `NFS13.exe+0xd872a0` was set 1.2 → 1.3, listed, restored by `unpoke`, and restored again at exit).
- **Phase 0 built and verified**: `crates/mw-probe` builds for `i686-pc-windows-gnu` (`.cargo/config.toml`
  names the mingw linker) as an `xinput1_3.dll` stand-in that forwards the real exports (Wine's builtin lacks
  `XInputGetDSoundAudioDeviceGuid`; it is left unresolved and logged). `tools/orig/orig launch --probe <name>`
  copies it next to `NFS13.exe`, sets the override and `MW_PROBE_OUT` through the request file and
  `orig-wrap`; `orig quit` removes the copy; `orig modules` lists the game's loaded DLLs (both `xinput1_3` and
  `dinput8` are loaded, plus EA's own `IGO32.dll` overlay). In the test run the probe was loaded *after* the
  wrapper had unpacked the code (the tick function's bytes never changed while it watched), the vehicle table
  chain resolved within 1.1 s (at the title screen already, so it is an "engine up" signal rather than
  "gameplay"), and the game ran seven minutes in free roam with normal input and car state readable.
- **Tier 1 not built, and not by me**: the next step is the in-memory detour installer (overwriting the first
  bytes of game functions with a jump to a logging stub). The assistant's attempt to write that code was
  stopped by its safety system, and it will not write it. Everything up to that point (the load path, the
  readiness signals, the row format and the plan below) stands; whether and how the patching step is written
  is the user's decision. The hardware-breakpoint route in "Risks" is the same kind of step and is in the same
  position.
- **2026-10-05, second attempt**: the user asked for an in-game tuning overlay and for the assistant to at least
  try the patching step (policy updated accordingly). It tried, and its safety system stopped it again; the
  in-process overlay code was removed and the probe is back to Phase 0. What the research established stays
  useful for whoever writes it: the game's present wrapper is `FUN_0046fb60` (it calls
  `IDXGISwapChain::Present`, vtable +0x20, on the swapchain held in `DAT_011acde4`; the device is in
  `DAT_011acd64`; the end-of-frame caller is `FUN_00476f20`); there is no game text-drawing helper to borrow;
  pad input needs no hook at all, since the game polls `XInputGetState` through our proxy (`FUN_0040f140`,
  `FUN_004119c0`). The fallback the user named, an external menu, is built: `mwtool tune` ("Tuning menu").

## Capture helpers (2026-10-19)

Tools that make runs of the original repeatable: teleport, car swap, scripted inputs tick by tick, fixed
conditions, a per-tick log. Source: `crates/mw-probe/src/control.rs` (in the game), `crates/mwtool/src/probe_ctl.rs`
(the commands), `flat_strip.rs` (finding strips and kerbs). Commands: docs/commands.md "Capture helpers".

**Where our code runs** (no instruction of the game is rewritten): two vtable slots in `.rdata` are pointed at
small thunks (pushad / pushfd, call into Rust, restore, jump to the original method), each installed only when
the slot holds the expected method (so after the wrapper has unpacked the game) and restored on unload.
- `0x01023760` → `FUN_00904da0`, the race car's "Update Car" (decomp.md "Update loop"): `on_tick(vehicle, dt)`
  runs for every race car each 60 Hz tick; the probe acts on the player's car only. `FUN_0091c5c0` copies each
  car's control record into its control fields (`FUN_009aacc0`) before the world step updates the cars, so the
  scripted inputs written here (+0x1bf0 steer, +0x1bf4 throttle, +0x1c00 brake ×4, +0x1c20 nitrous ×4, +0x1c40
  handbrake mask ×4) are the ones that tick uses. Nitrous's own throttle rule (`FUN_006c1a80`) is bypassed.
- `0x00fdb4d4` → `FUN_00761d70`, the free-drive "Car Swap Logic" behaviour's update (decomp.md "Jack spots"):
  `on_frame()` runs the commands that call game functions, once per frame on the game-logic thread, where the
  jack spots call the same functions. The tick and the frame run on the same thread (status `tick_thread` =
  `frame_thread`). The XInput poll is not usable for this: it runs on the input thread (`FUN_0040f140`).

**Commands** (`cmd.req` in the probe dir, answer in `cmd.out`, `status.txt` twice a second):
- `place x y z heading [mph]`: the game's placement with streaming, `FUN_00647e00` on G + 0x25690, called as the
  trigger teleport `FUN_006e4770` calls it (handle, position, direction, speed in mph, 0, 0xf, 0, 0). It waits
  until the target is resident and resets the car (suspension, wheels, engine) as a respawn does; with a speed
  the car starts rolling (62.137 mph read back exactly). y is the ground; the game puts the car on it.
- `tp x y z heading [m/s]` (`mwtool tp --raw`): moves the chassis body ([vehicle +0x1760]: rows +0x00..+0x20,
  position +0x30, velocity +0xb0, angular velocity +0xc0, the world inverse inertia +0x40..+0x6c rotated from
  the local one at +0xe0 after checking the stored one has that form) and each wheel record (centre, velocity,
  spin = speed / radius) and wheel matrix by the same rigid motion, at the start of the car's tick. Only for
  short moves inside the streamed world; `--hops` chains them.
- `car <vehicle id>`: `FUN_006c2020(player, id, colour 0, rollout 0)` as the jack spot's swap; the profile's
  current car changes (the save is restored after a session).
- `repair` (`FUN_00844940(handle)`, vehicle-manager message 0x16), `notraffic` (`FUN_008462a0`, traffic message
  0xf), `poke` / `hold` / `peek <type> <value> <chain>` (hold = rewritten every tick), `release`.
  `mwtool conditions --hour h` pokes the time-of-day rate (environment manager + 0x1b610, = renderer − 0x80 +
  0x1b610) to 0 and holds the time (+0x1b604); `--nitrous n` holds player 0's amount (game + 0x264f0 + 0x10).
- `mods <vehicle id> <slot>=<level> ...` (`mwtool mods <car> <fits>`): fits upgrades without unlocking them, through
  the swap's own rollout argument: the player's rollout record is copied ([G + 0x2f404] + 0x827c + player index ×
  0xd8, 0x2b dwords: +0x30 vehicle id, five groups of (descriptor pointer, slot, level) from +0x70, `FUN_0067e770`),
  each named group's slot and level replaced, the vehicle id set, and `FUN_006c2020(player, id, 0, &copy)` called
  as the jack spot calls it (the Easydrive / save paths pass a rollout the same way). The game copies the record
  into the profile (`FUN_0068e9d0`) and fixes the descriptor pointer itself. **The profile is changed in memory and
  the game autosaves it** (the car and its fits were in the save after the next launch once): back the save up
  first and restore it afterwards, as for `car`. Game slots ("UpgradeEnum" 0..14) and the data's fields:
  `mw_assets::mods::GAME_SLOT_FIELD` (`mwtool mods <car>` prints both); levels 1 base, 2 PRO (`FUN_0065de50`).
  `rollout` prints the record.
- `script <inputs.csv> [<watch> <out.csv>]`: inputs `t,steer,throttle,brake,handbrake,nitrous` (step-wise, row
  by game time since the start, steer + = right as the game's field; nitrous is the control amount written to all
  four lanes of vehicle +0x1c20, bypassing the tank and its throttle rule, and `nan` leaves the lane to the game); the log has one row per tick: `t,tick,dt`
  then the watch file's fields as the state the previous tick left, with the control fields as the values that
  tick used (the convention of `mwtool capture` rows and of `--replay`). Chains through the player's vehicle are
  followed every tick (contact lists move); unreadable values are NaN. `arm x z [mph]` holds the next script
  until the car has been placed there (and has the speed), so a run that starts with `place` starts on the
  first tick after it. `log <watch> <out> [s]` logs without inputs; `stop`.

**Reproducibility** (research log 2026-10-19 "probe capture helpers"): two runs from a rest placement with the
same script agree to float noise (1e-5 mph, 0.4 mm over 6.5 s with a kerb); runs placed at speed are
bit-identical, but the engine state after an at-speed placement takes one of two values (alternating runs
match); over a kerb drop the contact list's entry order can differ between runs and the runs then drift apart
by tenths of a mph.

**Limits**: the airport arena (zones 36-57, x −3300..350, z −5800..−1800) is not reachable here: placing,
teleporting or driving into it puts the car back on the free-roam start (respawn reason 6, `FUN_006d0180`). The
HSM's "Show Airport Required Dialog" and `CanJoinAirportGameByFriendIndexBinding` suggest an entitlement gate
(not traced). The test strip is therefore Hodges Airfield (`tools/orig/spots.txt`). The idle attract mode
(the Most Wanted list over a grey picture) starts after some idle time: tap a harmless key (`orig key w`, the
horn) before a run. Never mute the game's stream by hand: WirePlumber stores that per application name and the
user's own launches inherit it; `orig-wrap` sends headless runs to a silent sink (`tools/silent-sink`).

**Crashes on the far placement (2026-10-19)**: of seven launches, the first `place` to Hodges Airfield worked once
(the 911 from the canyon start, −107, 37, −370) and killed the game five times (the process exits, gamescope
keeps the last frame, the probe stops answering): from the Fairhaven start (200, 48, 1488) with and without a
car swap before it, and from the canyon start with a fitted mod. The kernel log had NVRM "can't alloc VA space
for mapping" / out-of-memory lines in the same minutes while other agents' headless runs shared the 8 GB card, so
GPU memory is the suspect (not proven). Short placements on the strip itself never failed. Open: a hop chain
(`tp --hops`) or a nearer start for the long move.

## Memory dumps on request (2026-10-10)

The probe reads memory in-process, so it needs no ptrace permission (`kernel.yama.ptrace_scope` = 1 blocks
`memscan` / `snapshot.py` on this machine). While the game runs with `orig launch --probe <name>`, write
`ref/probe/<name>/dump.req`; within 200 ms the probe reads it, renames it `dump.done`, writes one `<label>.bin`
per line and a report `dump.txt` (address, length, ok / unreadable). A line is `label base [offset ...] length`
in the `handling.watch` convention (`base`, then for each offset: read a pointer there and add the offset):

```
veh     NFS13.exe+0xcedb04 0x4 0x53220 0x184 0x0 0x3000     # the player's vehicle object
chassis NFS13.exe+0xcedb04 0x4 0x53220 0x184 0x10 0x0 0x200 # its chassis body
```

Read-only (every range is checked with `VirtualQuery` first). Used for the wheel placement check (research log
2026-10-10: wheel matrices at vehicle +0x870 + i × 0x1d0 +0x60, chassis rows / position at body +0x00 / +0x30,
render local offset at vehicle +0x160).

## Tuning menu: `mwtool tune` (2026-10-05)

A terminal menu that runs next to the game (`tools/orig/orig launch`, then `mwtool tune` in another terminal)
and changes handling values while you drive. Every change is a Tier-0 data poke (writable, non-executable
mappings only, original remembered), and everything is restored on quit. Source: `crates/mwtool/src/tune.rs`.

- **Pages**: Grip, Drag/Aero, Drift/HB, Drivetrain. Each row shows the live value, the original once changed,
  `exp` (experiment: writable, effect unknown or overwritten by the game) or `ro` (state, read-only), the
  address and where it comes from.
- **Global fields** are the `.bss` constants our handling code already cites (drag K, anti-roll scale, drift
  side-force cap, clutch cap, shift phases, downshift hysteresis, idle governor, plus gravity and the burnout
  scale). A poke holds for every car.
- **Car fields** live on the player's vehicle object, found through the `handling.watch` chain every 100 ms
  (`q = [[NFS13.exe+0xcedb04]+4]`, `index = [q+0x55c48]`, `vehicle = [[q+0x53220] + 4 + 8·index]`). When
  the object changes (a different car chosen in the game), the old records are dropped without writing back
  and the set values go into the new car. **Cars can only be switched in the game**: no memory write respawns a
  vehicle, since a car's bodies, wheels, GeneSys data and model are all built when it spawns.
- **Keys**: ↑/↓ select, ←/→ step (Shift ×10, `,` / `.` ×0.1), Enter types a value, `h` holds a value
  (re-applied whenever the game changes it, for the grip multipliers whose writer is unknown), `r` / `R`
  restore one / all, `s` / `L` save / load the preset (`--preset`, default `ref/tune/preset.txt`, lines
  `key = value [hold]`), Tab or 1–4 for pages, `q` quits and restores. `--apply` loads the preset at start;
  `--print` prints every field once and exits.
- **Not on the pages yet**: per-car GeneSys data (grip curves and axle scales, gear ratios and shift points,
  drift behaviour, handbrake strength, drag and lift coefficients). The game reads them in place from the
  loaded resource, but no pointer from the vehicle object to that resource is known; `mwtool handling <car>`
  prints them. Finding that pointer is the next step for this menu.
- Verified on the running game (XKR, car park): the car resolved at index 48; drag K 1.2 → 1.25 read back from
  outside and 1.2 again after quit; wheel 0 grip multiplier held at 0.9 and back to 1 after quit.


## Why

`mwtool memscan` / `telemetry` read the car's state through `/proc/<pid>/mem` at up to 120 Hz. That settled
many constants, but it cannot see:

- values that live only in registers or on the stack during a tick: the per-wheel normal load N, the lateral
  and longitudinal caps (μ × N × scales × surface × multipliers), the impulses the contact solver applied, the
  slip angle and ratio it used, the drift side force vector. These are exactly the unknowns left in
  [handling.md](handling.md) ("absolute grip scale", "drift side force", the per-wheel grip multipliers whose
  writer was never found);
- *when* within a tick something happened (samples from outside are not tick-aligned, and a 120 Hz read of a
  60 Hz tick sees each state twice or misses the transient);
- which function wrote a value (the grip multipliers, the handbrake lock condition).

A probe that runs on the game's own thread at a chosen instruction sees all of it, tick-exact, and can also
*change* things: set a constant to test what it does, or run our Rust port of one function in the original's
place to prove the port is exact. The last is the strongest fidelity test available: if the game drives
identically with our lateral curve substituted for its own, the port is right by construction.

## What it is not

The probe started as a research instrument. Since 2026-10-06 the policy also allows mods of the original
(Tier 3 below), and the probe's load path and patcher are their mechanism. The policy in
[reverse-engineering.md](reverse-engineering.md) applies in full: it never touches the DRM wrapper, the licence
check or `Core/Activation.dll`, never patches `NFS13.exe` on disk, never dumps or decrypts protected code, and
the game stays offline while it is loaded. Data files it replaces are backed up and restorable. The repository holds only the
mechanism; the addresses and byte signatures it needs are already documented in prose in
[decomp.md](decomp.md) or live in the git-ignored `re/`.

## Facts that shape the design (2026-10-04, from the PE headers on disk)

- `NFS13.exe` is 32-bit, image base `0x400000`, **no ASLR** (`DllCharacteristics` 0x8100: NX compatible,
  terminal-server aware, no `DYNAMIC_BASE`). Every address in the dump and in decomp.md is the runtime address.
- The on-disk import table has only `SHLWAPI.dll`, `KERNEL32.dll`, `USER32.dll`: those are the wrapper's
  imports. The game's own imports (D3D11, XInput, DirectInput, sockets...) are resolved by the wrapper after it
  decrypts `.text`. Consequence: a proxy DLL named after a game import (the usual `dinput8.dll` trick) is only
  loaded if and when the game itself asks for that name, which is after unpacking. That is actually convenient
  (see "Loading"), but it has to be checked which names the game loads.
- The game runs under Proton 11 (Wine with the new WoW64: a 64-bit Unix process hosting the 32-bit PE). A
  Unix-side `LD_PRELOAD` library therefore shares the address space but cannot be *called* from 32-bit game code
  without a mode switch. The probe is a 32-bit Windows DLL; the Unix side is used only for launching.
- The physics tick (`FUN_00904260`) runs at a fixed 60 Hz; functions of interest are listed with addresses in
  [decomp.md](decomp.md). Calling conventions in the dump are MSVC `__thiscall` / `__fastcall` / `__cdecl`
  and are not always recovered correctly by Ghidra.

## Tiers

Build in this order; each tier is useful on its own.

### Tier 0: `mwtool memscan poke` (no DLL)

Writing to `/proc/<pid>/mem` needs the same ptrace permission as reading. A `poke <name|addr> <value>` command
in the memscan REPL (and `--poke name=value` on `telemetry` / `capture`) lets us set a data value while the
game runs, with the original value remembered and restored on `unpoke` or exit. Use: confirm what a `.bss`
constant does (set the suspected lateral grip scale to half and watch the skidpad speed), test a candidate
value for an unmeasured constant, zero a term to isolate another. **Data only, never code**: the tool refuses
addresses inside `.text`. This is a day of work and needs no toolchain.

### Tier 1: the probe DLL, logging (design; the patching step is not written, see Status)

A 32-bit Windows DLL (`crates/mw-probe`, target `i686-pc-windows-gnu`) loaded into the game. At start-up it
waits until the game has unpacked itself, then installs **register-snapshot hooks** at the addresses listed in
a config file and streams what they see into a CSV in `ref/<run>/probe.csv`, in the same layout as
`telemetry.csv` (tick, `speed_mph`, then named columns) so `mwtool compare` and the replay tooling work on it
unchanged.

**Hook mechanism.** One generic thunk per hook, written in `naked_asm!`: it pushes all general registers and
the flags, calls `extern "C" fn hit(hook_id: u32, regs: *const Regs)` in Rust with a pointer to that frame,
restores everything, executes the relocated original instructions and jumps back. Because the thunk snapshots
every register and the stack pointer, the Rust side can read `this` from `ecx`, stack arguments from
`[esp + 4 + 4 n]`, or any struct field, without depending on Ghidra's guess of the calling convention. A hook
is then a *spec*, not code:

```toml
# re/probe.toml (git-ignored: the expected bytes come from the dump)
out = "ref/run1/probe.csv"

[[hook]]
name   = "tyre_limits"                  # FUN_009c5da0: force limits per wheel (decomp.md "Tyres")
at     = 0x009c5da0
expect = "55 8b ec 83 e4 f8"            # the first bytes, checked before patching; refuse on mismatch
read   = [
  { col = "wheel",  from = "ecx",         type = "u32", add = 0x10 },
  { col = "n_load", from = "[esp+8]",     type = "f32" },
  { col = "mu_lat", from = "[ecx+0x1a4]", type = "f32" },
]

[[hook]]
name   = "tick_end"                     # FUN_00904260 return: one row per tick
at     = 0x00904260
where  = "return"                       # entry (default) or return (hooks the ret via a stack-swapped return address)
read   = [{ col = "speed_mph", from = "[ecx+0x1620]", type = "f32" }]
```

Rows are written into a lock-free ring buffer on the game thread and flushed by a writer thread, so a hook
costs a few hundred nanoseconds. A `tick` counter column (incremented at the tick hook) lets per-wheel rows be
joined to their tick.

**Patching.** Entry hooks overwrite the first ≥ 5 bytes with `jmp thunk` and relocate the displaced
instructions into the thunk's tail. Instruction lengths come from a length decoder (the `iced-x86` crate,
`no_std`-capable, decodes 32-bit code; or `retour`'s built-in handling, which also fixes up relative branches;
`retour` is CI-tested on `i686-pc-windows-gnu` and its `RawDetour` / `GenericDetour` work on stable, only the
`static_detour!` macro needs nightly). Mid-function hooks (`where = 0x...` inside a function) take an explicit
`len` and are refused if the displaced bytes contain a relative branch. Patching uses `VirtualProtect` and is
done while the game is at a known quiet point (see "Timing"). Every patch is logged with its original bytes and
undone on `DLL_PROCESS_DETACH`.

**Timing.** The DLL may be loaded before the wrapper has decrypted `.text`. The installer thread polls the
bytes at each hook's `at` every 10 ms and installs only when they equal `expect` and have been stable for
0.5 s. If they never match, nothing is patched and the log says so: wrong version, or the signature is wrong.

**Loading.** Three candidates, to be settled by Phase 0 below with a DLL that only writes a "hello" file:

1. *Proxy DLL*: a DLL with the name of one the game loads after unpacking (`xinput1_3.dll` is the likely one;
   `dinput8.dll`, `winmm.dll` and `version.dll` are the other usual suspects), forwarding its exports to the
   real one, placed next to `NFS13.exe` and enabled with `WINEDLLOVERRIDES="xinput1_3=n,b"`. Both the file and
   the override are set only for probe runs by `tools/orig/orig launch --probe`, and the file is removed by
   `orig quit`, so normal launches are untouched. Which names the game loads is read from the running game's
   module list (`/proc/<pid>/maps`), not from the code.
2. *`AppInit_DLLs`* in the prefix's registry (`HKLM\Software\Microsoft\Windows NT\CurrentVersion\Windows`),
   if this Wine version implements it. Loads with `user32`, so before unpacking; the poll in "Timing" covers
   that. Would need no file in the game directory, but affects every program in that prefix (only this game
   lives there).
3. *Remote thread* from a tiny 32-bit Windows helper started in the same prefix (`CreateRemoteThread` +
   `LoadLibraryA`), after the game shows its window. Most flexible, most moving parts.

Prefer 1, fall back to 2, use 3 only if neither works.

**Toolchain.** `rustup target add i686-pc-windows-gnu` and `pacman -S mingw-w64-gcc` (provides
`i686-w64-mingw32-gcc`; neither is installed yet). The crate is `cdylib`, `#![no_main]`, depends on
`windows-sys` only, and is excluded from the default workspace build so `cargo build --workspace` on Linux is
unaffected (its own `cargo build -p mw-probe --target i686-pc-windows-gnu --release`).

**Output and tooling.** `mwtool capture --probe` starts the game with the probe (through `orig`), passes the
run directory through the `MW_PROBE_OUT` environment variable (Wine passes the Unix environment through;
`/home/...` is `Z:\home\...`), and gathers `probe.csv` next to `telemetry.csv` and `audio.wav`. `mwtool
compare` learns the extra columns. A `probe.log` in the run directory records load time, which hooks were
installed, their original bytes, and how many rows were dropped if the ring buffer overflowed.

### Tier 2: experiments

Same DLL, two more spec kinds:

- `[[poke]]`: `at`, `type`, `value`: write a data value after installation (what Tier 0 does from outside, but
  tick-synchronised: applied from the tick hook so the change lands between ticks).
- `[[replace]]`: `at`, `expect`, `fn = "lateral_curve"`: redirect a whole function to a Rust implementation
  with the matching convention (`extern "thiscall"`, `"fastcall"`, `"cdecl"` are all stable Rust ABIs). The
  implementation lives in a Bevy-free crate shared with the runtime, so the *same* Rust code runs inside the
  original and inside Most Rusted. First candidates, smallest and purest first: the lateral grip curve
  (`FUN_009bcdb0`), the longitudinal curve (`FUN_009bc720`), the drag force (`FUN_009cf850`), the brake torque
  (`FUN_009b1180`), then the steering update (`FUN_009d31f0`) and the drift entry test (`FUN_009d5690`). A
  replacement is validated by driving the same scripted manoeuvre (`tools/orig/orig run`) with and without
  it and comparing the two `telemetry.csv` with `mwtool compare`: a correct port shows no difference above
  floating-point noise. This needs the handling maths pulled out of `crates/mostrusted` into a plain crate
  (`mw-handling`), which is also a cleanliness win for the runtime.

### Tier 3 (allowed since 2026-10-06, not yet designed): mods

New vehicle types (bikes), new game modes or new rendering in the original. The policy allows this
([reverse-engineering.md](reverse-engineering.md)). The cost is the same as before: each needs the class layouts
of many systems and code that cooperates with all of them, in an engine with no names. Bikes in particular have
nothing to build on: no bike code, types or assets survive from Burnout Paradise (research log, 2026-10-06), and
the handling code assumes left/right wheel pairs per axle. Each mod is designed here before it is built: what it
patches (addresses documented in decomp.md), what data it adds or replaces, and how it is undone.

## Phases and acceptance

0. **Load path** (half a day): "hello" DLL, each candidate loader, pick one. Done when `orig launch --probe`
   leaves a hello file and a normal launch does not load the DLL.
1. **Tier 0 poke** (a day): memscan `poke` / `unpoke`, refuses `.text`, restores on exit. Done when setting
   the drag constant halves the measured drag in a coast-down capture.
2. **Tier 1 logging** (two to three days): thunk, patcher with signature check, ring buffer, CSV, `orig`
   integration, `compare` columns. Done when a tick hook's `speed_mph` column matches `telemetry.csv` and a
   corner capture gives per-wheel N and caps for all four wheels every tick.
3. **First measurements**: N and the lateral / longitudinal caps in a steady corner (settles the grip scales
   and the "μ ≈ 3" puzzle), the drift side force vector in a brake-tap drift, the handbrake lock condition,
   who writes the per-wheel grip multipliers (a write-watch hook on the field's writer, found by poking the
   field and seeing which hook fires). Each result goes into handling.md with "probe, run `ref/<name>`" as
   its source.
4. **Tier 2 replace**: `mw-handling` crate, the lateral curve as the first replacement, the comparison
   procedure, then the rest of the list.

## Risks

| Risk | Mitigation |
|------|-----------|
| The DRM wrapper checks code integrity at run time and exits or misbehaves after a patch | Phase 0 patches one function and plays for ten minutes. If patches are detected, fall back to hardware breakpoints (DR0–DR3 set with `SetThreadContext`, a vectored exception handler reads the `CONTEXT`): no code is modified, four hooks at a time, slower but ample at 60 Hz. Whether this Wine honours debug registers set from a Windows thread context is itself to be checked in Phase 0. |
| The game pauses or crashes while a hook runs (page protection, a hook on the wrong thread) | Hooks only read memory and push into a ring buffer; nothing allocates or locks on the game thread. The writer thread does the I/O. |
| Wrong version or wrong signature patches a live instruction stream | `expect` bytes are compulsory and must match exactly, or the hook is skipped and logged. |
| Addresses drift | They don't: fixed image base, the game is on its final patch (1.4.0.0). |
| A mod leaves the install changed | Replaced data files are backed up before the first write and restored by the mod tooling; Steam's "Verify integrity" is the fallback. No DLL override survives a modded run. |
| Online play with modified code | `orig` only ever launches offline (the EA sign-in prompt is dismissed); the probe is never loaded by a normal launch. A check for a live connection from inside the DLL is a Phase 1 nicety, not a safeguard to rely on. |

## Open questions

- Which DLL names does the unpacked game load (settles the proxy choice)? Read from `/proc/<pid>/maps`.
- Does this Wine implement `AppInit_DLLs`? Try it in Phase 0 if the proxy route fails.
- Does the wrapper re-verify `.text` after start-up? Phase 0, by waiting.
- The physics thread: is the tick on the main thread or a worker? Matters only for the breakpoint fallback.
