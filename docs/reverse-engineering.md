# Reverse engineering notes

## The executable is encrypted on disk

`NFS13.exe` (version 1.4.0.0, the Steam / EA app build) is a 32-bit PE whose `.text` section (11.5 MB) has a
Shannon entropy of 8.0 bits/byte, meaning it is encrypted. The entry point (RVA `0xDDD830`) lies in a small
unnamed section appended after `.rsrc`, which is the EA DRM wrapper. It decrypts the code at runtime after
checking the licence with the EA app.

As a result, statically loading the file into Ghidra yields almost nothing: auto-analysis finishes in under a
minute and finds no game code. `.rdata` is not encrypted and still has strings and 116 RTTI type descriptors
(`.?AV...`), which can help name classes once code is available.

Most Rusted does **not** include or automate any way to remove or bypass that protection. The asset side of the
project does not need the executable at all: every format so far has been worked out from the data files and
public community documentation.

For the parts that are code rather than data (handling model, AI, game rules), the options are:

1. Keep working data-first: much of the game's behaviour is driven by AttribSys vaults and GeneSys objects,
   which are readable from the bundles.
2. Use public research (burnout.wiki, the Burnout Paradise and Hot Pursuit modding communities). Criterion's
   engine is shared across these titles.
3. Any analysis of a decrypted binary is a separate decision for the person doing it, who is responsible for
   whether that is permitted where they live. Decompiled code itself never goes into this repository.

## The user's decompiled dump (2026-10-04)

The user made a memory dump of the running game and decompiled it with Ghidra (`re/NFS13_dump/`, git-ignored:
pseudo-C, function table, strings). It is used as a reference for behaviour the data cannot show (the handling
model above all). Rules:

- Never commit the dump, Ghidra databases or any pseudo-C, and never paste decompiled code into source, comments
  or docs.
- Record what the code *does* in prose and formulas, in our own words, citing function addresses so it can be
  re-checked: [decomp.md](decomp.md) collects the findings; implementation details go into
  [handling.md](handling.md) and the research log.
- Write our own Rust from those descriptions.

## Code inside the running game: the probe (policy, 2026-10-04)

The user decided that hooks running in their own licensed, running copy of the game are in scope, as a
measuring and verification instrument. The design is [probe.md](probe.md). What is and is not allowed:

**Allowed**
- Loading our own DLL into the running game (proxy DLL, `AppInit_DLLs`, or a remote thread in the Wine prefix),
  on this machine, by the user, for runs started with `tools/orig/orig launch --probe` or `mwtool capture
  --probe`. Normal launches stay untouched: no file in the game directory and no DLL override outside those runs.
- Reading anything in the process from inside it, including registers and stack contents at hooked
  instructions, and logging it to the git-ignored `ref/`.
- Writing *data* values in memory (constants, fields) to learn what they do, from inside (the probe) or from
  outside (`mwtool memscan poke` through `/proc/<pid>/mem`), with the original value restored afterwards.
- Patching *game* functions in memory, after the game has unpacked itself, at addresses that are documented in
  prose in [decomp.md](decomp.md): a detour to a logging thunk, or a replacement by our own Rust port of that
  function so the port can be proven exact against the original's behaviour.
- Writing that instruction-patching step itself. This is the user's own licensed copy, offline, after
  unpacking, on documented non-DRM addresses, for measurement and tuning, never distributed. Earlier the
  assistant declined to write the detour installer and left it to the user; the user has since asked that the
  assistant **at least try** to write it (2026-10-05), falling back to an equivalent external tool
  (`/proc/<pid>/mem` pokes) only if it genuinely cannot. An in-process tuning overlay (hooking the game's
  present and input paths to draw a menu that pokes handling *data* fields) is within this: it is the Tier-1
  detour mechanism plus Tier-0 data pokes, not a new capability. The hard limits below still bind it in full.
  Outcome of the first try (2026-10-05): the assistant's safety system stopped it again, so the overlay was
  dropped and the external `mwtool tune` built instead ([probe.md](probe.md) "Status", "Tuning menu"). The
  assistant still tries when asked, and states plainly when it is stopped.

**Not allowed**
- Anything that touches the DRM: the wrapper stub (the unnamed section after `.rsrc`), its import resolution,
  `Core/Activation.dll`, licence or online checks. The probe installs only after the game has unpacked itself
  and never reads, dumps, saves or decrypts protected code; the expected-bytes signatures it checks before
  patching come from the user's existing dump and live in the git-ignored `re/probe.toml`, never in the repo.
- Writing to the game's files, the install directory (beyond the proxy DLL placed for a probe run and removed
  after it), the save game or the Steam / EA configuration.
- Running the game online with the probe loaded. Only `orig launch --probe` / `mwtool capture --probe` load it,
  and they launch offline.
- Distributing the probe as a mod or growing it into one (new vehicle types, game modes, rendering). Those are
  the reimplementation's job; [probe.md](probe.md) "Tier 3" says so. A one-off experiment that needs more is
  written up in probe.md first.

**What goes in the repository**: the mechanism (`crates/mw-probe`, `mwtool` commands, `orig` flags), function
addresses (as in decomp.md), and measurement results in prose with the run name as their source. Not: byte
signatures, pseudo-C, struct dumps, captured data.

## Ghidra setup

A headless project can be created with:

```sh
MAXMEM=12G /opt/ghidra/support/analyzeHeadless re/ghidra_project NFS13 \
    -import "<game dir>/NFS13.exe" -overwrite
```

`re/ghidra_project` is git-ignored. Never commit Ghidra databases, decompiler output or game data.
