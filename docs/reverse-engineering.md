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

## Ghidra setup

A headless project can be created with:

```sh
MAXMEM=12G /opt/ghidra/support/analyzeHeadless re/ghidra_project NFS13 \
    -import "<game dir>/NFS13.exe" -overwrite
```

`re/ghidra_project` is git-ignored. Never commit Ghidra databases, decompiler output or game data.
