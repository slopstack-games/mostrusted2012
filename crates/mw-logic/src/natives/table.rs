//! Every native of the original's two registries: name, handler address in NFS13.exe, area. Filled from the dump
//! (`FUN_007763b0` for the hooks, `FUN_009f7e50` for Lua; decomp.md "Native registries"). Names and addresses
//! only, no code.

use super::Area;

/// HSM hooks (`FUN_007763b0`).
pub const HOOKS: &[(&str, u32, Area)] = &[];

/// Lua natives (`FUN_009f7e50`).
pub const LUA: &[(&str, u32, Area)] = &[];
