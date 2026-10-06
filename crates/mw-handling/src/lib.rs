//! The game's handling maths, ported from the decompiled code (docs/decomp.md, docs/handling.md), with no
//! dependency on Bevy or avian: just `mw_assets::handling` data in, numbers out. The `mostrusted` runtime
//! drives these each tick; `mwtool` calls the same functions on inputs recorded from the running original to
//! check a port is exact (docs/probe.md, "Tier 2", offline verification).

pub mod drift;
pub mod drivetrain;
pub mod nitrous;
pub mod tyre;
