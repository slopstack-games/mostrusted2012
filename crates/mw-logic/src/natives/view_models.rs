//! The view models bindings read: the roots the game registers (`Mode`, `Players`, `Signals`, ...: the root
//! registry `FUN_00a520d0`, records of 0x14 at +0x12968 of the binding manager, and the dynamic providers
//! `FUN_00a58f70`) and the data-definition view models HSM states publish (`FUN_00a4ead0`) and retract
//! (`FUN_00a4de20`). Natives resolve bindings through this (`ResolveBoolBinding`, ...), the HSM host reads its
//! conditions here, widgets bind to it.
//!
//! The API below is the contract between the registry and the natives; the bodies are filled by the registry
//! work (PLAN.md M8 step 3).

use mw_formats::json::Json;

/// Every view model bindings can read.
#[derive(Debug, Default)]
pub struct ViewModels {
    /// Published data-definition view models by path, in publish order.
    pub published: Vec<(String, Json)>,
}

impl ViewModels {
    /// `ResolveBoolBinding` / an HSM condition: the binding text evaluated as a bool (`None` = not resolved).
    pub fn resolve_bool(&self, _binding: &str) -> Option<bool> {
        None
    }
    /// `ResolveIntBinding`.
    pub fn resolve_int(&self, _binding: &str) -> Option<i64> {
        None
    }
    /// `ResolveDoubleBinding`.
    pub fn resolve_double(&self, _binding: &str) -> Option<f64> {
        None
    }
    /// `ResolveStringBinding` (the to-text visitor; "" on error in the original).
    pub fn resolve_string(&self, _binding: &str) -> Option<String> {
        None
    }
    /// An HSM state with a data definition became active.
    pub fn publish(&mut self, path: &str, data: Json) {
        self.retract(path);
        self.published.push((path.to_string(), data));
    }
    /// ... and finished.
    pub fn retract(&mut self, path: &str) {
        self.published.retain(|(p, _)| p != path);
    }
}
