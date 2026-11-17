//! The original's managers, as far as the natives read and write them (one sub-struct per manager, in the file of
//! the area that owns it; fields name their offset in the original, as decomp.md does).

/// The logic state the natives act on.
#[derive(Debug, Default)]
pub struct LogicState {
    /// What the natives asked the engine to do, in call order; the runtime drains it every frame.
    pub requests: Vec<Request>,
    /// Calls of natives that are known but not ported yet (`"hook Name"` / `"lua Name"`), for logs and tools.
    pub unported: Vec<String>,
    /// What bindings read ([`ViewModels`](super::ViewModels)).
    pub view_models: super::ViewModels,
    /// What the runtime reports about the engine ([`Engine`]).
    pub engine: Engine,
}

/// Engine facts the logic reads, set by the runtime (or a tool) before the logic runs.
#[derive(Debug)]
pub struct Engine {
    /// `FUN_00897960` on `*0x010edafc`: the graphics are prepared (`BWAIT4GFX`).
    pub graphics_prepared: bool,
    /// The loading-screen manager's request mask (UI manager +0x2d204) is nonzero and +0x2d200 clear
    /// (`HSB_WAIT4LS` / `HSB_WAIT4NL`).
    pub loading_screen_showing: bool,
    /// The UI clock (`*0x010edacc` → +4 → +0x18): ticks at 3,000,000 per second (i64 at +8) and the u32 at +0,
    /// advanced by [`LogicHost::frame`](super::LogicHost::frame).
    pub ui_ticks: i64,
    pub ui_counter: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Engine { graphics_prepared: true, loading_screen_showing: false, ui_ticks: 0, ui_counter: 0 }
    }
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {}
