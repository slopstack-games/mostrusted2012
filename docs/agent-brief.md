# Brief for worktree agents

Given to every worktree agent together with its task, so task prompts stay short. The prompt names the
integration branch (`<branch>` below) and what the other agents are working on. You don't need anything else
from the integrator's conversation.

## Setup, in order

1. `git merge --ff-only <branch>` in your worktree (it may start from an older commit).
2. `.claude/tools/copy-deps.sh <main checkout> <your worktree>`: real copies of the finished dependency build,
   never hard links. The main checkout is the first line of `git worktree list`; its target is a fresh build of
   `<branch>`.
3. Build only with `tools/b` (= `nice -n 19 cargo build --release -p mostrusted -p mwtool`, filtered to
   warnings / errors). Another package set rebuilds Bevy. Don't run clippy, tests or `--workspace` builds: the
   integrator runs them once on the merged tree. Run `cargo fmt` before committing.

## Engine-free tasks

When the task is in the engine-free crates (mw-formats, mw-assets, mw-logic, mw-lua, mw-audio, mw-handling,
mwtool): skip copy-deps and `tools/b` (no Bevy build); build and test only the crates you touch
(`nice -n 19 cargo build --release -p <crate> -p mwtool`, `nice -n 19 cargo test -q --release -p <crate>`),
keep them wasm32-compatible (`tools/wasm-check`), and clippy clean for them.

## Token habits (context is the budget)

- Big docs: `tools/doc <doc> [heading text] [--head N]` prints one section (no text = headings with sizes).
  Start from the pointers in the task; don't survey docs you weren't pointed to.
- Dump: `fn <addr> --head 60` first; the rest of a function only if needed. One function at a time.
- Long tool output into a file, then grep it; tests through `grep -E "test result|FAILED|panicked"`.
- Don't re-read files you just edited or wrote.
- Your report: at most ~400 words. Lists, tables and logs go into the research log or a git-ignored file.

## Rules

- **Fidelity** (CLAUDE.md): every behaviour or look change names its source: a data field, a function in the
  dump (`re/` of the main checkout), an `.rdata` constant, a capture. No tuning by feel; a value without a
  source is a model decision and is marked as one in the docs. Dump helpers in the main checkout's git-ignored
  `.claude/tools/`: `fn <addr>`, `callers <addr>`, `exe-const <va> [n] [f32|u32|str]`. REA (MCP `rea`) opens
  `re/out/live/NFS13_snap1_vt.exe` for functions the dump misses (reverse-engineering.md "Functions the dump is
  missing"; slow first query; other agents may use it at the same time).
- **Never commit** game data, exports or decompiled code; findings go into the docs as prose. That includes disassembly or `fn` output saved to
  files: keep those in the git-ignored `scratch/`, never at the repo root, and check `git status` before committing.
- **/tmp and the scratchpad are RAM**: screenshots, traces and binaries go into the worktree's git-ignored
  `extracted/`, `ref/` or `scratch/`.
- **The original game** (`tools/orig`): only when the task says so (one agent at a time; it is silent by
  itself, never mute its stream by hand (WirePlumber stores that per application); quit it after, back up the save). Otherwise list the captures that would confirm your change in the research
  log.
- **No windows on the user's desktop**: run `mostrusted` only through `tools/t <args>` or `tools/hl
  ./target/release/mostrusted <args>` (headless gamescope, 1280 × 720 like the normal window; silent: `MW_MUTE=1`
  and a null sink, `tools/silent-sink`). Nothing may be heard either.
- **Verify** with `tools/t <args>` (TEST RESULT lines, exit code) and `--screenshot` runs (through `tools/hl`); look at the PNGs that
  decide something, A/B stacked with `python3 tools/stack.py out.png a.png b.png`.
- **Merging**: other agents work in parallel (the prompt says on what). Keep edits to files outside your area
  small; prefer new modules plus small hooks.
- **Context**: CLAUDE.md "Working economically" and "Token habits" above.

## When done

1. Add a dated research-log entry (newest first, under the header) with sources, model decisions and open items.
   Update decomp.md / formats.md / PLAN.md where relevant. Add new flags and env vars to docs/commands.md (one
   line each), not CLAUDE.md.
2. Commit on your worktree branch. End the message with the co-author line the integrator gives you.
3. Report briefly. The report is all the integrator sees: the branch and commit, what changed, how it was
   verified (test lines, screenshot paths), the model decisions and what is still open. No step-by-step
   narration; long tables and logs go into the research log or a git-ignored file that the report points to.
