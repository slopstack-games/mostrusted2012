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
    pub use web::{Builder, spawn};

    /// `wasm_thread` with one addition: each worker reports full stacks (`Error.stackTraceLimit`), so a panic on a
    /// loader thread names its caller (the default 10 frames end inside the panic machinery).
    #[cfg(target_arch = "wasm32")]
    mod web {
        fn full_stacks() {
            if let Ok(error) = js_sys::Reflect::get(&js_sys::global(), &"Error".into()) {
                let _ = js_sys::Reflect::set(&error, &"stackTraceLimit".into(), &200.into());
            }
        }

        pub struct Builder(wasm_thread::Builder);

        impl Builder {
            pub fn new() -> Self {
                Self(wasm_thread::Builder::new())
            }
            pub fn name(self, name: String) -> Self {
                Self(self.0.name(name))
            }
            pub fn spawn<F, T>(self, f: F) -> std::io::Result<wasm_thread::JoinHandle<T>>
            where
                F: FnOnce() -> T + Send + 'static,
                T: Send + 'static,
            {
                self.0.spawn(move || {
                    full_stacks();
                    f()
                })
            }
        }

        pub fn spawn<F, T>(f: F) -> wasm_thread::JoinHandle<T>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            Builder::new().spawn(f).expect("spawn a worker thread")
        }
    }
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

/// Save the captured frame: to the file natively; in a browser it is POSTed to the page's server
/// (`/__screenshot?name=<file name>`, `tools/web/serve.py`), the only place a headless browser can put it.
pub fn save_screenshot(
    path: impl AsRef<std::path::Path>,
) -> impl FnMut(bevy::prelude::On<bevy::render::view::screenshot::ScreenshotCaptured>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        bevy::render::view::screenshot::save_to_disk(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let path = path.as_ref().to_owned();
        move |shot| crate::web::post_screenshot(&path, &shot.image)
    }
}

/// `Receiver::recv_timeout` for worker threads. In a browser it reads `std::time::Instant` (which panics there),
/// so it polls instead: `try_recv`, then 1 ms sleeps (a worker's sleep is an atomic wait, no clock) until the
/// timeout's worth of slices has passed; a message is seen within ~1 ms.
pub fn recv_timeout<T>(
    rx: &std::sync::mpsc::Receiver<T>,
    timeout: std::time::Duration,
) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rx.recv_timeout(timeout)
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
        let slices = timeout.as_millis().max(1);
        for i in 0..slices {
            match rx.try_recv() {
                Ok(v) => return Ok(v),
                Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
                Err(TryRecvError::Empty) if i + 1 < slices => std::thread::sleep(std::time::Duration::from_millis(1)),
                Err(TryRecvError::Empty) => {}
            }
        }
        Err(RecvTimeoutError::Timeout)
    }
}
