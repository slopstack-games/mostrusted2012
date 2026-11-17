//! What differs between the desktop and the browser build (wasm32 with shared-memory threads), so the rest of
//! the runtime is written once:
//!
//! - [`thread`]: background loads start here. Native = `std::thread`; in a browser each thread is a Web Worker
//!   sharing the module's memory (`wasm_thread`, same API).
//! - [`PollLock`]: how the main thread reads what a worker filled in. In a browser the page's main thread may not
//!   block (a contended lock traps there), so it polls: a busy lock reads as "not yet", tried again next frame.
//! - [`Instant`]: `std::time::Instant` panics in a browser; Bevy's is the same type natively.
//! - [`time_seed`]: wall-clock nanoseconds for seeding, from `Date.now()` in a browser.

pub use bevy::platform::time::Instant;

pub mod thread {
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::thread::{Builder, spawn};
    #[cfg(target_arch = "wasm32")]
    pub use wasm_thread::{Builder, spawn};
}

/// A non-blocking lock for the main thread.
pub trait PollLock<T> {
    /// The guard, or `None` while another thread holds the lock (a poisoned lock is still returned: the value a
    /// panicked worker left is as good as any).
    fn poll_lock(&self) -> Option<std::sync::MutexGuard<'_, T>>;
}

impl<T> PollLock<T> for std::sync::Mutex<T> {
    fn poll_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> {
        match self.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

/// Nanoseconds since the Unix epoch (a seed), 1 if the clock is unavailable.
pub fn time_seed() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() * 1e6) as u64
    }
}
