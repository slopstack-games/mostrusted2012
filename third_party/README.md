# Patched upstream crates

Used through `[patch.crates-io]` in the root `Cargo.toml`. Each is the published crate's `src`, manifest,
licences and README, unchanged except for the patches listed here (search for "mostrusted patch"). Drop a crate
from here once upstream covers the change.

## rodio 0.22.2 (MIT / Apache-2.0)

- `src/lib.rs`: `LockSpin::lock_spin`, and every `Mutex::lock().unwrap()` in `player.rs`, `queue.rs`,
  `spatial_player.rs` and `source/buffered.rs` (32 sites) goes through it. In the threaded browser build the
  mixer runs on the AudioWorklet thread while the game sets volumes, positions and queues sounds from the
  page's main thread; a contended `Mutex::lock` waits with `memory.atomic.wait32`, which traps on both threads.
  The critical sections are single assignments or queue operations, so `lock_spin` spins on `try_lock` there.
  Natively it is the ordinary lock.
- `#![allow(warnings)]`: path dependencies are not lint-capped like registry ones.
- `Cargo.toml`: the `[[example]]` / `[[bench]]` / `[[test]]` targets removed (their files are not copied).

## bevy_audio 0.19.1 (MIT / Apache-2.0)

- `src/audio_output.rs` `open_sink`: on wasm32 with atomics the output opens on cpal's AudioWorklet host (the
  callback runs on the audio rendering thread over shared memory), falling back to the default WebAudio host,
  whose callbacks run on the main thread and gap whenever a frame stalls (research-log 2026-11-17 "browser
  audio"). Elsewhere unchanged.
- `Cargo.toml`: cpal with the `audioworklet` feature on wasm32. `#![allow(warnings)]` as above.
