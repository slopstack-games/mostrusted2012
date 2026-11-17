# Reverse engineering notes

## The executable is encrypted on disk

`NFS13.exe` (version 1.4.0.0, the Steam / EA app build: Steam build id 10351185, 13,684,056 bytes, file date
2026-10-03; the dump in `re/` and the snapshots in `re/out/live` are of this build, checked again 2026-11-17 after a
Steam update that only replaced the EA app installer. If the build id or size ever changes, every address in the
docs, the native table and `re/probe.toml` needs re-checking against a new dump) is a 32-bit PE whose `.text` section (11.5 MB) has a
Shannon entropy of 8.0 bits/byte, meaning it is encrypted. The entry point (RVA `0xDDD830`) lies in a small
unnamed section appended after `.rsrc`, which is the EA DRM wrapper. It decrypts the code at runtime after
checking the licence with the EA app.

As a result, statically loading the file into Ghidra yields almost nothing: auto-analysis finishes in under a
minute and finds no game code. `.rdata` is not encrypted and still has strings and 116 RTTI type descriptors
(`.?AV...`), which can help name classes once code is available.

For the parts that are code rather than data (physics and handling, rendering, audio, AI, game rules, UI), the
sources are:

1. The data: much of the game's behaviour is driven by AttribSys vaults and GeneSys objects, which are readable
   from the bundles.
2. Public research (burnout.wiki, the Burnout Paradise and Hot Pursuit modding communities). Criterion's engine
   is shared across these titles.
3. **Reverse engineering the game's own code** (user decision, 2026-10-06): to make Most Rusted look and feel
   exactly like the original, any of the game's inner workings may be reverse engineered. That covers the physics
   and handling, the renderer (passes, shaders, lighting, post-processing, LOD and streaming), audio, AI, traffic,
   cameras and game rules. Allowed means are the decompiled dump (below), the shader disassembly (`mwtool
   shader-asm`), observing the running game (`mwtool memscan` / `telemetry`, the probe's hooks and pokes,
   graphics frame captures of the original's draw calls and render targets), and experiments that change a value
   or function and watch the result. Where both are available, this beats a plausible guess: the fidelity rule
   in CLAUDE.md wants a source for every behaviour. The limits stay the same: nothing that touches the DRM, and
   decompiled code never goes into this repository (the rules below).

## The user's decompiled dump (2026-10-04)

The user made a memory dump of the running game and decompiled it with Ghidra (`re/NFS13_dump/`, git-ignored:
pseudo-C, function table, strings). It is the reference for any behaviour the data cannot show: the handling
model first, and equally the renderer, audio, AI and game rules (point 3 above). Rules:

- Never commit the dump, Ghidra databases or any pseudo-C, and never paste decompiled code into source, comments
  or docs.
- Record what the code *does* in prose and formulas, in our own words, citing function addresses so it can be
  re-checked: [decomp.md](decomp.md) collects the findings for every system; implementation details go into
  the system's own doc ([handling.md](handling.md) for physics) and the research log.
- Write our own Rust from those descriptions.

### Functions the dump is missing (2026-10-14)

The dump's Ghidra run only found functions reached by calls: **16,257 code addresses that a data section points
at are not function starts in `functions.csv`** (53,501 functions), almost all of them methods reached only
through vtables or callback / state tables. Each sits after int3 / nop / ret padding and starts with a normal
prologue. That is why several systems' docs say "not in the dump" or "writer not found": the camera director's
update `0x005d5e20` (the chase camera's wall avoidance, decomp.md "Cameras") is one, referenced from the vtable at
`0x00f99d1c`. Treat "not in any decompiled function" as "not decompiled yet", not "does not exist".

`tools/snapimage.py` carves the decrypted image out of a memory snapshot (`re/out/live/snap1`, `.text` identical in
snap2) into a PE Ghidra loads (sections in their memory layout, `.data` / `.bss` as they were in game), and with
`--functions re/NFS13_dump/functions.csv` exports every such address as `vt_<address>` so Ghidra's analysis starts
a function there and follows its calls. The output stays in `re/out/live/`.

**REA** (MCP server `rea`, Ghidra provider, `/opt/ghidra` 12.1.2) opens it with `open_binary` and gives
pseudocode, assembly, cross-references and call paths per address. Its default deadline for import + analysis is
5.5 min, enough for the plain carved image but not for the one with the exports: `REA_GHIDRA_STARTUP_TIMEOUT_MS`
in the `rea` server's env (`~/.claude.json`) is set to 3600000. A timed-out request kills the Ghidra process and
the next one starts the analysis again. Pseudocode from REA follows the dump's rules above: prose in the docs only.

## The PS Vita build: a statically analysable ARM binary (2026-10-07)

The user owns the PS Vita release (PCSB00183, EU, v01.00, built 2012-09-24) and, on their own modded console,
decrypted the eboot with FAGDec. Unlike the x86 `NFS13.exe`, **the Vita eboot is not DRM-wrapped once FAGDec
has run**: `re/vita/FAGDec/patch/PCSB00183/eboot.bin.elf` is a clean ELF32 little-endian ARM executable
(entry `0x67c798`, load base `0x81000000`, ~7.9 MB code segment, entropy 7.23 — not packed). It loads
straight into Ghidra/radare2 and still carries debug strings and full `D:\dev\CGCode\Hawaii\Runtime\VITAOPT\...`
source paths. It is the same engine as the PC version ("Hawaii"), so it is a legitimate RE source under the
same policy as the dump, and often an easier one (clean disassembly, named asserts).

Rules, same spirit as the dump:
- The ELF and anything derived from it (Ghidra DBs, disassembly, pseudo-C) are game data: they live in
  git-ignored `re/vita/` and are **never committed**; findings go into the docs in prose + addresses only.
- It is an **ARMv7 build, not the x86 binary**. Use it to recover control flow and *which fields a rule reads*;
  then confirm the exact constants/offsets against the PC dump or a live read before a value becomes a model
  decision (the fidelity rule — the shipping target is the PC game). Note the Vita address (`0x81......`) and
  the PC-dump address side by side when both are known.
- Only the eboot + `sce_module` SELFs are decrypted so far. The `.BNDL` assets are PFS-encrypted and still
  need a decrypted copy (VitaShell / FTP / NoNpDrm) before `mwtool` can read them.

## Code inside the running game: the probe and mods (policy, 2026-10-04; widened 2026-10-06)

The user decided that hooks running in their own licensed, running copy of the game are in scope, first as a
measuring and verification instrument (the probe, 2026-10-04). On 2026-10-06 the user widened this: **modding
the original is allowed**, including new behaviour such as new vehicle types (bikes), game modes or rendering,
and changes to the game's data files. The one thing that stays forbidden is getting around the DRM. The probe's
design is [probe.md](probe.md). What is and is not allowed:

**Allowed**
- Loading our own DLL into the running game (proxy DLL, `AppInit_DLLs`, or a remote thread in the Wine prefix),
  on this machine, by the user, for runs started with `tools/orig/orig launch --probe` or `mwtool capture
  --probe`, or by a mod launcher built the same way. A normal Steam launch stays untouched: no DLL override is
  left behind after a modded run.
- **Mods**: new or changed game behaviour through code patched in memory after the game has unpacked itself
  (the probe's detour and replacement mechanism), new code in our own DLL, and changed or added data: GeneSys
  objects, AttribSys vaults, bundles and their resources. Data mods may replace files in the install, provided
  the original is backed up first and the mod tooling can restore it (verify with Steam's "Verify integrity"
  as the last resort). The save game may be changed the same way, with a backup.
- Reading anything in the process from inside it, including registers and stack contents at hooked
  instructions, and logging it to the git-ignored `ref/`.
- Writing *data* values in memory (constants, fields) to learn what they do, from inside (the probe) or from
  outside (`mwtool memscan poke` through `/proc/<pid>/mem`), with the original value restored afterwards.
- Patching *game* functions in memory, after the game has unpacked itself, at addresses that are documented in
  prose in [decomp.md](decomp.md): a detour to a logging thunk, or a replacement by our own Rust port of that
  function so the port can be proven exact against the original's behaviour.
- Writing that instruction-patching step itself. This is the user's own licensed copy, offline, after
  unpacking, on documented non-DRM addresses, for measurement, tuning and (since 2026-10-06) mods. Earlier the
  assistant declined to write the detour installer and left it to the user; the user has since asked that the
  assistant **at least try** to write it (2026-10-05), falling back to an equivalent external tool
  (`/proc/<pid>/mem` pokes) only if it genuinely cannot. An in-process tuning overlay (hooking the game's
  present and input paths to draw a menu that pokes handling *data* fields) is within this: it is the Tier-1
  detour mechanism plus Tier-0 data pokes, not a new capability. The hard limits below still bind it in full.
  Outcome of the first try (2026-10-05): the assistant's safety system stopped it again, so the overlay was
  dropped and the external `mwtool tune` built instead ([probe.md](probe.md) "Status", "Tuning menu"). The
  assistant still tries when asked, and states plainly when it is stopped.

**Not allowed**
- **A DRM bypass, or anything that touches the DRM**: the wrapper stub (the unnamed section after `.rsrc`), its
  import resolution, `Core/Activation.dll`, licence or online checks. A modded game is always started through
  the normal licensed launch. The probe and mods install only after the game has unpacked itself and never
  read, dump, save or decrypt protected code. `NFS13.exe` on disk is not patched: its code is encrypted, so
  changing it would mean decrypting it. Code mods live in memory only. The expected-bytes signatures checked
  before patching come from the user's existing dump and live in the git-ignored `re/probe.toml`, never in the
  repo.
- Running the game online with the probe or a code mod loaded. Modded launches go offline, as `orig launch
  --probe` / `mwtool capture --probe` already do. This keeps other players' games and the user's account out
  of it.

**What goes in the repository**: the mechanism (`crates/mw-probe`, mod code, `mwtool` commands, `orig` flags),
function addresses (as in decomp.md), and measurement results in prose with the run name as their source. Not:
byte signatures, pseudo-C, struct dumps, captured data, game files or assets (new or edited bundles that
contain game data stay in git-ignored dirs). Mod content we make ourselves (our own models, textures, tuning
values) may be committed. A mod that is ever shared carries only our own code and content, never game files
and nothing that touches the DRM.

## Ghidra setup

A headless project can be created with:

```sh
MAXMEM=12G /opt/ghidra/support/analyzeHeadless re/ghidra_project NFS13 \
    -import "<game dir>/NFS13.exe" -overwrite
```

`re/ghidra_project` is git-ignored. Never commit Ghidra databases, decompiler output or game data.
