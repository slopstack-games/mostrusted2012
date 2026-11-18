//! What differs between the desktop and the browser build (wasm32 with shared-memory threads), so the rest of
//! the runtime is written once:
//!
//! - [`thread`]: background loads start here. Native = `std::thread`; in a browser each thread is a Web Worker
//!   sharing the module's memory (`wasm_thread`, same API).
//! - [`PollLock`]: how the main thread reads what a worker filled in. In a browser the page's main thread may not
//!   block (a contended lock traps there), so it polls: a busy lock reads as "not yet", tried again next frame.
//! - [`Instant`]: `std::time::Instant` panics in a browser; Bevy's is the same type natively.
//! - [`time_seed`]: wall-clock nanoseconds for seeding, from `Date.now()` in a browser.
//! - [`read_text`] / [`write_text`]: the settings and profile files; `localStorage` entries in a browser.

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

/// The small text files the runtime keeps between runs (settings, the profile): [`read_text`] / [`write_text`].
/// Native = the files themselves, under `settings::config_dir()`. In a browser there are no files: each one is a
/// `localStorage` entry `mostrusted/<file name>` (see [`storage_key`]). `localStorage` only exists on the page's
/// main thread and systems run on workers too, so the browser side works on a copy in memory: [`load_text_store`]
/// fills it on the main thread before the game starts, reads and writes use the copy from any thread, and
/// [`flush_text_store`] (main thread, once a frame) writes what changed back to `localStorage`.
pub fn read_text(path: &std::path::Path) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_store::lock().get(&storage_key(path)).map(str::to_owned)
    }
}

/// Write (replace) a text file of [`read_text`], creating its directory.
pub fn write_text(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(path, text)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_store::lock().set(storage_key(path), text.to_owned());
        // On the page's main thread the entry is stored at once; from a worker, by the next frame's flush.
        flush_text_store();
        Ok(())
    }
}

/// The prefix of every `localStorage` entry of the text store.
#[cfg(any(target_arch = "wasm32", test))]
pub const STORAGE_PREFIX: &str = "mostrusted/";

/// The `localStorage` key of a text file: [`STORAGE_PREFIX`] + its file name (`mostrusted/settings.cfg`,
/// `mostrusted/profile.cfg`). Only the name counts, so the key does not depend on where a native build would keep
/// the file.
#[cfg(any(target_arch = "wasm32", test))]
pub fn storage_key(path: &std::path::Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{STORAGE_PREFIX}{name}")
}

/// The in-memory copy of the browser's text store: every entry, and the keys written since the last flush.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Default)]
struct TextStore {
    entries: std::collections::HashMap<String, String>,
    /// Keys to write to `localStorage`, oldest first, each once.
    pending: Vec<String>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl TextStore {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// An entry read from `localStorage` (not written back). A write still waiting for its flush is newer and
    /// stays.
    fn load(&mut self, key: String, text: String) {
        if !self.pending.contains(&key) {
            self.entries.insert(key, text);
        }
    }

    fn set(&mut self, key: String, text: String) {
        if self.entries.get(&key) == Some(&text) {
            return;
        }
        if !self.pending.contains(&key) {
            self.pending.push(key.clone());
        }
        self.entries.insert(key, text);
    }

    /// The entries to write out, with their current text; the queue is then empty.
    fn take_pending(&mut self) -> Vec<(String, String)> {
        let keys = std::mem::take(&mut self.pending);
        keys.into_iter().filter_map(|k| self.entries.get(&k).cloned().map(|t| (k, t))).collect()
    }

    /// Put back entries that could not be written: they are tried again at the next flush.
    fn requeue(&mut self, keys: impl IntoIterator<Item = String>) {
        for k in keys {
            if !self.pending.contains(&k) {
                self.pending.push(k);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web_store {
    use super::{PollLock, TextStore};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn store() -> &'static Mutex<TextStore> {
        static STORE: OnceLock<Mutex<TextStore>> = OnceLock::new();
        STORE.get_or_init(Default::default)
    }

    /// The store, from any thread. The page's main thread may not wait on a lock (it traps there), so a busy lock
    /// is spun on instead: every holder only touches the map (no I/O, no other lock), so it is free within
    /// microseconds.
    pub fn lock() -> MutexGuard<'static, TextStore> {
        loop {
            if let Some(g) = store().poll_lock() {
                return g;
            }
            std::hint::spin_loop();
        }
    }

    /// The store if it is free right now (the per-frame flush: a busy store waits for the next frame).
    pub fn poll() -> Option<MutexGuard<'static, TextStore>> {
        store().poll_lock()
    }
}

/// `localStorage`, on the page's main thread only (workers have none; a browser may also refuse it).
#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read every `mostrusted/…` entry of `localStorage` into the text store. Main thread, before the game starts.
#[cfg(target_arch = "wasm32")]
pub fn load_text_store() {
    let Some(storage) = local_storage() else {
        return bevy::log::warn!("no localStorage: settings and progress are not kept in this browser");
    };
    let mut store = web_store::lock();
    for i in 0..storage.length().unwrap_or(0) {
        if let Ok(Some(key)) = storage.key(i)
            && key.starts_with(STORAGE_PREFIX)
            && let Ok(Some(text)) = storage.get_item(&key)
        {
            store.load(key, text);
        }
    }
}

/// Write the entries changed since the last call to `localStorage`. Does nothing on a worker thread; the main
/// thread calls it once a frame (`web::report`). An entry the browser refuses (quota, storage disabled) is kept
/// for the next call and reported once.
#[cfg(target_arch = "wasm32")]
pub fn flush_text_store() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    let Some(mut store) = web_store::poll() else { return };
    if store.pending.is_empty() {
        return;
    }
    let Some(storage) = local_storage() else { return };
    let pending = store.take_pending();
    drop(store);
    let mut failed = Vec::new();
    for (key, text) in pending {
        if let Err(e) = storage.set_item(&key, &text) {
            if !WARNED.swap(true, Ordering::Relaxed) {
                bevy::log::warn!("could not save {key} to localStorage: {e:?}");
            }
            failed.push(key);
        }
    }
    if !failed.is_empty() {
        web_store::lock().requeue(failed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn text_files_round_trip_and_create_their_directory() {
        let dir = std::env::temp_dir().join(format!("mostrusted-text-{}-{}", std::process::id(), time_seed()));
        let path = dir.join("mostrusted").join("settings.cfg");
        assert_eq!(read_text(&path), None);
        write_text(&path, "master=0.50\n").unwrap();
        assert_eq!(read_text(&path).as_deref(), Some("master=0.50\n"));
        // The file is the plain text itself (what earlier builds wrote and read), replaced whole on a write.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "master=0.50\n");
        write_text(&path, "a=1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a=1\n");
        // A path that cannot be written is an error, not a panic.
        assert!(write_text(&path.join("below-a-file"), "x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn storage_keys_are_the_file_name_under_the_prefix() {
        assert_eq!(storage_key(Path::new("/home/u/.config/mostrusted/settings.cfg")), "mostrusted/settings.cfg");
        assert_eq!(storage_key(Path::new("mostrusted/profile.cfg")), "mostrusted/profile.cfg");
        assert_eq!(storage_key(Path::new("profile.cfg")), "mostrusted/profile.cfg");
    }

    #[test]
    fn text_store_queues_each_changed_key_once() {
        let mut s = TextStore::default();
        s.load("mostrusted/settings.cfg".into(), "master=1.00\n".into());
        assert_eq!(s.get("mostrusted/settings.cfg"), Some("master=1.00\n"));
        assert!(s.take_pending().is_empty(), "loaded entries are not written back");
        s.set("mostrusted/settings.cfg".into(), "master=1.00\n".into());
        assert!(s.take_pending().is_empty(), "an unchanged write is not queued");
        s.set("mostrusted/profile.cfg".into(), "bundle=a\n".into());
        s.set("mostrusted/profile.cfg".into(), "bundle=b\n".into());
        s.set("mostrusted/settings.cfg".into(), "master=0.25\n".into());
        assert_eq!(s.get("mostrusted/profile.cfg"), Some("bundle=b\n"));
        assert_eq!(
            s.take_pending(),
            vec![
                ("mostrusted/profile.cfg".to_string(), "bundle=b\n".to_string()),
                ("mostrusted/settings.cfg".to_string(), "master=0.25\n".to_string())
            ]
        );
        assert!(s.take_pending().is_empty());
        // A reload does not undo a write that has not reached `localStorage` yet.
        s.set("mostrusted/settings.cfg".into(), "master=0.75\n".into());
        s.load("mostrusted/settings.cfg".into(), "master=0.25\n".into());
        assert_eq!(s.take_pending(), vec![("mostrusted/settings.cfg".to_string(), "master=0.75\n".to_string())]);
        // A failed write is tried again with the text current by then.
        s.requeue(["mostrusted/profile.cfg".to_string()]);
        s.set("mostrusted/profile.cfg".into(), "bundle=c\n".into());
        assert_eq!(s.take_pending(), vec![("mostrusted/profile.cfg".to_string(), "bundle=c\n".to_string())]);
    }
}
