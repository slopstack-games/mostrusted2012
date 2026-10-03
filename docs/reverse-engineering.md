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
   whether that is permitted where they live. Nothing derived from it, including decompiled code, belongs in
   this repository.

## Ghidra setup

A headless project can be created with:

```sh
MAXMEM=12G /opt/ghidra/support/analyzeHeadless re/ghidra_project NFS13 \
    -import "<game dir>/NFS13.exe" -overwrite
```

`re/ghidra_project` is git-ignored. Never commit Ghidra databases, decompiler output or game data.
