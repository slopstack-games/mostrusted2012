//! The native functions the original's logic data calls, under their original names (PLAN.md M8 step 3):
//!
//! - **HSM hooks**: the registry filled by `FUN_007763b0` (187 names, keyed by name hash at the HSM manager
//!   +0x360), called by `CALLHSMHOOK` / `CALLHSMHOOKS` / `HSMHOOKEXIT` with a member of the state's data
//!   definition as arguments (`FUN_007761a0`), and by widgets' native actions (`FUN_007f2670`). A hook returns
//!   the exit point to take; [`EXITINVALID`](crate::hsm::EXITINVALID) means "not (yet)".
//! - **Lua natives**: the registry filled by `FUN_009f7e50` (385 names); the host binds each as a global closure
//!   with its name as upvalue (`FUN_009f86d0`), for the widget scripts' `LuaHooks` (decomp.md "Widget definitions
//!   (JSON) and widget scripts").
//!
//! 36 names are in both registries (35 plus `ExitWidgetGroup`); each has its own handler in the original, and its
//! own entry here.
//!
//! [`LogicHost`] owns a registry and the state and serves both interpreters (the HSM's [`Host`](crate::hsm::Host),
//! globals of a Lua state); the state is shared with the Lua closures as `Rc<RefCell<LogicState>>`.
//!
//! Natives act on [`LogicState`]: the original's managers as engine-free structs (one sub-struct per manager,
//! each in the file of the area that owns it). What only the engine can do (load a world, play a sequence,
//! mute the sound) is asked for through [`LogicState::requests`]; the runtime drains it every frame.
//!
//! Layout: [`table`] lists every native of both registries with its handler address and area (from the dump);
//! each area file (`hooks_*.rs`, `lua_*.rs`) has a `register(&mut Registry)` that installs the ones it ports.
//! [`Registry::original`] builds the table and runs every area's `register`. An entry without a function is a
//! known native that is not ported yet: calling it is recorded in [`LogicState::unported`].

pub mod game_logic;
pub mod game_modes;
pub mod hook_args;
pub mod hooks_event;
pub mod hooks_flow;
#[cfg(test)]
mod hooks_tests;
pub mod hooks_ui;
pub mod host;
pub mod players;
pub mod progression;
pub mod state;
pub mod table;
pub mod view_models;

use std::collections::HashMap;

pub use host::LogicHost;
pub use state::{Engine, LogicState, Request};
pub use view_models::ViewModels;

use crate::hsm::Key;
use mw_formats::json::Json;

/// An HSM hook: the state's data-definition member named by the behaviour (`Params`, `EnterParams`, ...) as
/// arguments; returns the exit point ([`EXITINVALID`](crate::hsm::EXITINVALID) = not done / not applicable).
pub type HookFn = fn(&mut LogicState, Option<&Json>) -> Key;

/// A Lua native: the call's arguments, its results (the original pushes them and returns their count).
pub type LuaFn = fn(&mut LogicState, &mut mw_lua::Lua, Vec<mw_lua::Value>) -> mw_lua::Result<Vec<mw_lua::Value>>;

/// What a native is for, from its handler and its callers (offline game first; online / platform ones are ported
/// last or answered with the original's offline result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    /// Single-player game, menus, profile, settings.
    Offline,
    /// Autolog, online play, the store, friends, Origin.
    Online,
    /// Console / platform specifics (PS3, X360, Vita, trophies).
    Platform,
    /// Debug and development helpers.
    Debug,
    /// Not classified yet.
    Unknown,
}

impl Area {
    pub const ALL: [Area; 5] = [Area::Offline, Area::Online, Area::Platform, Area::Debug, Area::Unknown];

    pub fn name(self) -> &'static str {
        match self {
            Area::Offline => "offline",
            Area::Online => "online",
            Area::Platform => "platform",
            Area::Debug => "debug",
            Area::Unknown => "unknown",
        }
    }

    /// By [`name`](Self::name), case-insensitive.
    pub fn parse(s: &str) -> Option<Area> {
        Self::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(s))
    }
}

/// One native of the original.
#[derive(Debug, Clone)]
pub struct Native<F> {
    pub name: &'static str,
    /// The handler's address in NFS13.exe (as in decomp.md).
    pub handler: u32,
    pub area: Area,
    /// `None`: known, not ported yet.
    pub f: Option<F>,
}

impl<F> Native<F> {
    pub fn ported(&self) -> bool {
        self.f.is_some()
    }
}

/// Both registries of the original.
#[derive(Debug, Default)]
pub struct Registry {
    hooks: Vec<Native<HookFn>>,
    lua: Vec<Native<LuaFn>>,
    hook_index: HashMap<&'static str, usize>,
    lua_index: HashMap<&'static str, usize>,
}

impl Registry {
    /// Every native of both registries ([`table`]), with the ported ones installed.
    pub fn original() -> Self {
        let mut r = Self::default();
        for &(name, handler, area) in table::HOOKS {
            r.hook_index.insert(name, r.hooks.len());
            r.hooks.push(Native { name, handler, area, f: None });
        }
        for &(name, handler, area) in table::LUA {
            r.lua_index.insert(name, r.lua.len());
            r.lua.push(Native { name, handler, area, f: None });
        }
        // Area files register here (one line each).
        hooks_flow::register(&mut r);
        hooks_event::register(&mut r);
        r
    }

    /// [`original`](Self::original) plus the hooks ported but not yet verified against the dump (hooks_*.rs
    /// `register_parked`).
    pub fn original_with_parked() -> Self {
        let mut r = Self::original();
        hooks_flow::register_parked(&mut r);
        hooks_event::register_parked(&mut r);
        hooks_ui::register_parked(&mut r);
        r
    }

    /// Install a ported hook; panics if the name is not one of the original's (a typo, or a missing table row).
    pub fn set_hook(&mut self, name: &str, f: HookFn) {
        let i = *self.hook_index.get(name).unwrap_or_else(|| panic!("{name} is not an HSM hook of the original"));
        self.hooks[i].f = Some(f);
    }

    /// Install a ported Lua native; panics if the name is not one of the original's.
    pub fn set_lua(&mut self, name: &str, f: LuaFn) {
        let i = *self.lua_index.get(name).unwrap_or_else(|| panic!("{name} is not a Lua native of the original"));
        self.lua[i].f = Some(f);
    }

    pub fn hook(&self, name: &str) -> Option<&Native<HookFn>> {
        self.hook_index.get(name).map(|&i| &self.hooks[i])
    }

    pub fn lua(&self, name: &str) -> Option<&Native<LuaFn>> {
        self.lua_index.get(name).map(|&i| &self.lua[i])
    }

    pub fn hooks(&self) -> &[Native<HookFn>] {
        &self.hooks
    }

    pub fn lua_natives(&self) -> &[Native<LuaFn>] {
        &self.lua
    }

    /// Call an HSM hook as the HSM manager does: `None` when the original has no hook of that name (the
    /// caller exits through EXITINVALID); an unported one is recorded in [`LogicState::unported`] and answers
    /// EXITINVALID.
    pub fn call_hook(&self, state: &mut LogicState, name: &str, args: Option<&Json>) -> Option<Key> {
        let native = self.hook(name)?;
        Some(match native.f {
            Some(f) => f(state, args),
            None => {
                state.unported.push(format!("hook {name}"));
                crate::hsm::EXITINVALID
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_both_registries() {
        let r = Registry::original();
        assert_eq!(r.hooks().len(), 187);
        assert_eq!(r.lua_natives().len(), 385);
        let both = r.hooks().iter().filter(|h| r.lua(h.name).is_some()).count();
        assert_eq!(both, 36);
        let mut state = LogicState::default();
        assert_eq!(r.call_hook(&mut state, "NoSuchHook", None), None);
        assert_eq!(r.call_hook(&mut state, "ResolveBoolBinding", None), Some(crate::hsm::EXITINVALID));
        assert_eq!(state.unported, ["hook ResolveBoolBinding"]);
    }
}
