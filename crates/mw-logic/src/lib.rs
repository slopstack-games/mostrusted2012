//! The original's game-logic interpreters, engine-free (no Bevy) and wasm-compatible (pure Rust, no threads):
//! the game's flow and UI are data run by these (CLAUDE.md "Fidelity rule", PLAN.md M8):
//!
//! - [`binding`]: the binding path language (`Mode.ShowLoadingScreen`, `Widget.X`, operators such as EQUALS /
//!   BETWEEN / NOT), parsed and evaluated like the original's evaluator over view models the host provides.
//! - [`hsm`]: the HSMData interpreter (node lifecycle, transitions, behaviour types).
//! - [`natives`]: one name → function registry shared by HSM hooks and Lua natives, under the original names.
//!
//! The Lua 5.1 VM for the UI's bytecode is its own crate (`mw-lua`).

pub mod binding;
pub mod hsm;
pub mod natives;
