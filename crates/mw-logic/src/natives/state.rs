//! The original's managers, as far as the natives read and write them (one sub-struct per manager, in the file of
//! the area that owns it; fields name their offset in the original, as decomp.md does).

/// The logic state the natives act on.
#[derive(Debug, Default)]
pub struct LogicState {
    /// What the natives asked the engine to do, in call order; the runtime drains it every frame.
    pub requests: Vec<Request>,
    /// Calls of natives that are known but not ported yet (`"hook Name"` / `"lua Name"`), for logs and tools.
    pub unported: Vec<String>,
}

/// A request to the engine (the runtime in `mostrusted`, or a tool's stub).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Request {}
