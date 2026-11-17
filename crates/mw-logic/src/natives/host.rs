//! The logic host: one [`Registry`] and one [`LogicState`] serving both interpreters, as the original's game
//! logic does (the HSM manager's hook registry at +0x360 and the Lua registry `FUN_009f7e50` act on the same
//! managers).
//!
//! - **HSM** ([`hsm::Host`]): hooks go through [`Registry::call_hook`] with the state's data-definition member;
//!   binding reads, publish and retract through [`LogicState::view_models`]; the graphics / loading-screen waits
//!   read [`LogicState::engine`], which the runtime sets.
//! - **Lua** ([`LogicHost::install_lua`]): every native of the registry becomes a global closure (the original
//!   binds each with its name as upvalue, `FUN_009f86d0`), after the libraries, so a native replaces a library
//!   function of the same name (`print`). A ported native calls its [`LuaFn`] on the shared state; an unported
//!   one records `"lua Name"` in [`LogicState::unported`] and returns no values.
//!
//! **Shared state from Lua closures**: the state is an `Rc<RefCell<LogicState>>` (mw-lua is single-threaded);
//! each native closure holds a clone and borrows the state mutably for the duration of the call. A native that
//! calls back into Lua while it holds the borrow (none of the original's does so synchronously as far as the
//! handlers show) would see a nested native fail with a Lua error ("re-entrant native call") instead of a panic.

use std::cell::RefCell;
use std::rc::Rc;

use mw_formats::json::Json;
use mw_lua::{Lua, LuaError, Value};

use super::pages::{self, Pages};
use super::{LogicState, LuaFn, Registry};
use crate::hsm::{self, Key, Trace};

/// A Lua global closure for one native.
pub type LuaNative = Box<dyn Fn(&mut Lua, &str, Vec<Value>) -> mw_lua::Result<Vec<Value>>>;

/// Receives the HSM's trace events.
pub type TraceFn = Box<dyn FnMut(&Trace)>;

/// Owns the native registry and the shared logic state; implements [`hsm::Host`].
pub struct LogicHost {
    pub registry: Registry,
    pub state: Rc<RefCell<LogicState>>,
    /// Receives the HSM's trace events (tools print them).
    pub on_trace: Option<TraceFn>,
    /// The page contexts and active widget groups (their Lua states live here, outside the shared state the
    /// natives borrow).
    pub pages: Pages,
}

impl Default for LogicHost {
    fn default() -> Self {
        Self::new(Registry::original())
    }
}

impl LogicHost {
    pub fn new(registry: Registry) -> Self {
        LogicHost { registry, state: Rc::default(), on_trace: None, pages: Pages::default() }
    }

    /// One frame of the logic's clocks: the UI clock advances by `dt` seconds and `Signals` updates
    /// (`FUN_007903c0`). Model: the UI clock's u32 is taken as elapsed milliseconds (the member descriptions of
    /// `GetMetronome` speak of milliseconds; which engine clock it is is open, decomp.md "The HSM interpreter").
    pub fn frame(&mut self, dt: f32) {
        let mut st = self.state.borrow_mut();
        st.engine.ui_ticks += (dt as f64 * 3e6).round() as i64;
        st.engine.ui_counter = (st.engine.ui_ticks / 3000) as u32;
        let (ticks, counter) = (st.engine.ui_ticks, st.engine.ui_counter);
        st.view_models.signals.update(ticks, counter);
        drop(st);
        // The active widgets' per-frame update (`FUN_007f6420`: `PreInputUpdate(dt)`).
        self.pages.frame(dt);
    }

    /// The global closure for one Lua native of the registry (`None`: not a native of the original).
    pub fn lua_native(&self, name: &str) -> Option<LuaNative> {
        let f: Option<LuaFn> = self.registry.lua(name)?.f;
        let state = self.state.clone();
        Some(Box::new(move |lua: &mut Lua, name: &str, args: Vec<Value>| {
            let Ok(mut st) = state.try_borrow_mut() else {
                return Err(LuaError::new(format!("{name}: re-entrant native call")));
            };
            match f {
                Some(f) => f(&mut st, lua, args),
                None => {
                    st.unported.push(format!("lua {name}"));
                    Ok(vec![])
                }
            }
        }))
    }

    /// Bind every Lua native of the registry as a global of `lua`.
    pub fn install_lua(&self, lua: &mut Lua) {
        for n in self.registry.lua_natives() {
            if let Some(f) = self.lua_native(n.name) {
                lua.register(n.name, f);
            }
        }
    }

    /// The part of a request the logic itself carries out before the runtime gets it: the game-mode manager's
    /// logic state (`mode_machine::apply`), the page contexts and widget groups ([`Pages`]: their scripts call
    /// natives, so no borrow of the state is held here).
    pub fn apply(&mut self, r: &super::Request) {
        super::mode_machine::apply(&mut self.state.borrow_mut(), r);
        let mut p = std::mem::take(&mut self.pages);
        pages::apply(&mut p, &self.state, &|lua| self.install_lua(lua), r);
        self.pages = p;
    }

    /// The active widgets' user actions this frame ([`Pages`]; `read` gives an input action's state): Lua
    /// callbacks run in the widgets' states, `ExitGroupVia` sets the group's exit, other native callbacks call the
    /// HSM hook of that name (without arguments: model, the `CallbackArgs` are not turned into a hook argument).
    pub fn input(&mut self, read: &dyn Fn(u8) -> pages::ActionInput) {
        let registry = &self.registry;
        self.pages.input(&self.state, read, &mut |st, name| {
            registry.call_hook(st, name, None);
        });
    }

    /// Take the unported calls recorded since the last call (for logs).
    pub fn take_unported(&self) -> Vec<String> {
        std::mem::take(&mut self.state.borrow_mut().unported)
    }
}

impl hsm::Host for LogicHost {
    /// Parse-gated (`FUN_0076f330` evaluates whenever the binding parsed): an unresolved path reads false, or
    /// true with `IS_EMPTY`.
    fn binding_bool(&mut self, path: &str) -> Option<bool> {
        self.state.borrow().view_models.resolve_bool(path)
    }
    fn binding_string(&mut self, path: &str) -> Option<String> {
        self.state.borrow().view_models.resolve_hsm_string(path)
    }
    fn binding_int(&mut self, path: &str) -> Option<i64> {
        let st = self.state.borrow();
        let b = crate::binding::Binding::parse(path).ok()?;
        let r = b.eval_int(&st.view_models);
        r.ok.then_some(r.value as i64)
    }
    fn hook(&mut self, name: &str, args: Option<&Json>) -> Option<Key> {
        self.registry.call_hook(&mut self.state.borrow_mut(), name, args)
    }
    fn graphics_prepared(&mut self) -> bool {
        self.state.borrow().engine.graphics_prepared
    }
    fn loading_screen_showing(&mut self) -> bool {
        self.state.borrow().engine.loading_screen_showing
    }
    /// [`LogicHost::apply`], then queues the request for the runtime with the natives' requests
    /// ([`LogicState::requests`]).
    fn request(&mut self, r: super::Request) {
        self.apply(&r);
        self.state.borrow_mut().requests.push(r);
    }
    /// The queries whose answer is logic state: the pending event (G+0x2f6f8), the GMM's event, mode start,
    /// machine and flags, the local player.
    fn query(&mut self, q: &hsm::behaviours::Query) -> Option<u64> {
        use hsm::behaviours::Query as Q;
        let st = self.state.borrow();
        if let Some(a) = self.pages.query(&st, q) {
            return Some(a);
        }
        let gm = &st.game_modes;
        Some(match q {
            Q::NextEvent => u64::from(st.game.pending_event),
            Q::GameModeStarted => super::mode_machine::mode_started(&st) as u64,
            Q::GameModeEvent => u64::from(gm.current_event),
            Q::GameModeMachine => gm.machine.map_or(u64::MAX, u64::from),
            Q::OnlineEvent => gm.online_event as u64,
            Q::FreedriveEvent => gm.freedrive_event as u64,
            Q::LocalPlayerReady => st.players.local_player().is_some() as u64,
            Q::EventIsFreedrive { event } => super::mode_machine::is_freedrive_event(&st, *event) as u64,
            Q::ShowWorldCount => st.game.show_world_count as u64,
            Q::RequestedEvent => u64::from(gm.requested_event),
            Q::StoryMode => gm.story_mode as u64,
            _ => return None,
        })
    }
    fn publish(&mut self, path: &str, data: &Json) {
        self.state.borrow_mut().view_models.publish(path, data.clone());
    }
    fn retract(&mut self, path: &str) {
        self.state.borrow_mut().view_models.retract(path);
    }
    fn trace(&mut self, event: &Trace) {
        if let Some(t) = &mut self.on_trace {
            t(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hsm::Host;

    #[test]
    fn hsm_side() {
        let mut h = LogicHost::default();
        let data = Json::parse(r#"{"CanShow": true}"#).unwrap();
        h.publish("SafeForDialogs", &data);
        assert_eq!(h.binding_bool("SafeForDialogs.CanShow"), Some(true));
        assert_eq!(h.binding_string("SafeForDialogs.CanShow"), Some("TRUE".into()));
        assert_eq!(h.binding_int("SafeForDialogs.Missing"), None);
        h.retract("SafeForDialogs");
        assert_eq!(h.binding_bool("SafeForDialogs.CanShow"), Some(false));
        assert_eq!(h.hook("NoSuchHook", None), None);
        assert_eq!(h.hook("SetGameVisibilityPublic", None), Some(hsm::EXITINVALID));
        assert_eq!(h.take_unported(), ["hook SetGameVisibilityPublic"]);
        assert!(h.graphics_prepared());
    }

    #[test]
    fn lua_side() {
        let h = LogicHost::default();
        let mut lua = Lua::new();
        h.install_lua(&mut lua);
        assert!(!lua.global("ResolveBoolBinding").is_nil());
        assert!(lua.global("NotANative").is_nil());
        let r = lua.call_global("ShowX360KinectTunerUi", vec![]).unwrap();
        assert!(r.is_empty());
        assert_eq!(h.take_unported(), ["lua ShowX360KinectTunerUi"]);
    }
}
