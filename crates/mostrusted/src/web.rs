//! The browser entry point. The page (`tools/web/index.html`) loads the module, then hands over the install and
//! the command line: either the folder the player picked (`start_with_files`) or a listing of a local server that
//! serves the player's own install (`start_with_listing`, `tools/web/serve.py`). The game data never leaves the
//! player's machine. Flags are the native ones (`--camera=…`, `--units`, `--screenshot`, …).

use std::sync::Arc;

use bevy::prelude::*;
use clap::Parser;
use mw_gamedir::{GameDir, WebSource};
use wasm_bindgen::prelude::*;

/// A panic message from a thread that cannot reach the page's hooks (a worker): [`report`] hands it over from the
/// main thread, which polls each frame.
static WORKER_PANIC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Call the page's hook `name(text, flag)` (`index.html`: `__mwStage`, `__mwError`). Silent where the page has none
/// (workers have no such global).
fn page_hook(name: &str, text: &str, flag: bool) {
    let global = js_sys::global();
    if let Ok(f) = js_sys::Reflect::get(&global, &name.into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call2(&global, &text.into(), &flag.into());
    }
}

/// What the runtime tells the loading overlay of the page, once a frame (main thread): the loader's current stage,
/// whether the world is loaded (the page then hides the overlay), and a panic a worker thread left behind.
pub fn report(status: &str, done: bool) {
    page_hook("__mwStage", status, done);
    if let Ok(mut p) = WORKER_PANIC.try_lock()
        && let Some(msg) = p.take()
    {
        page_hook("__mwError", &msg, true);
    }
    // Settings and progress written since the last frame (by any thread) go to `localStorage` here.
    crate::platform::flush_text_store();
}

/// Debug switch (`?fail=panic`): panics, to see the page show it.
#[wasm_bindgen]
pub fn debug_panic() {
    panic!("debug panic requested by ?fail=panic");
}

/// Debug hook for page tests: write `text` as the text file `name` (`platform::write_text`: the in-memory store,
/// then `localStorage` entry `mostrusted/<name>`).
#[wasm_bindgen]
pub fn debug_store_write(name: String, text: String) -> bool {
    crate::platform::write_text(std::path::Path::new(&name), &text).is_ok()
}

/// Debug hook for page tests: the text file `name` as the game would read it; `reload` first re-reads
/// `localStorage` (what a new page load does).
#[wasm_bindgen]
pub fn debug_store_read(name: String, reload: bool) -> Option<String> {
    if reload {
        crate::platform::load_text_store();
    }
    crate::platform::read_text(std::path::Path::new(&name))
}

/// The panic hook that shows a panic on the page, wrapping whatever hook is installed (Bevy's, which logs to the
/// console). Called right before `App::run`, after the plugins have set theirs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let msg = format!("panic: {info}");
        // On the main thread the page hook shows it at once; a worker leaves it for the next frame's `report`.
        let g = js_sys::global();
        if js_sys::Reflect::get(&g, &"__mwError".into()).is_ok_and(|f| f.is_function()) {
            page_hook("__mwError", &msg, true);
        } else if let Ok(mut p) = WORKER_PANIC.lock() {
            p.get_or_insert(msg);
        }
    }));
}

fn start(source: WebSource, args: Vec<String>) -> Result<(), JsValue> {
    let cli = crate::Cli::try_parse_from(std::iter::once("mostrusted".to_string()).chain(args))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    crate::platform::load_text_store();
    let game = GameDir::from_source(Arc::new(source)).map_err(|e| JsValue::from_str(&e.to_string()))?;
    crate::run(cli, game).map_err(|e| JsValue::from_str(&format!("{e:#}")))
}

/// The folder the player picked (`<input type="file" webkitdirectory>`).
#[wasm_bindgen]
pub fn start_with_files(files: web_sys::FileList, args: Vec<String>) -> Result<(), JsValue> {
    let source = WebSource::from_file_list(&files).map_err(|e| JsValue::from_str(&e.to_string()))?;
    start(source, args)
}

/// A served install: `listing` holds one `<size>\t<path>` line per file, each read from `<base><path>`. `base` must
/// be absolute: the worker threads run from `blob:` scripts, against which a relative URL is invalid.
#[wasm_bindgen]
pub fn start_with_listing(base: String, listing: String, args: Vec<String>) -> Result<(), JsValue> {
    let entries = listing.lines().filter_map(|l| {
        let (size, path) = l.split_once('\t')?;
        let url = format!(
            "{base}{}",
            path.split('/').map(js_sys::encode_uri_component).map(String::from).collect::<Vec<_>>().join("/")
        );
        Some((path.to_string(), url, size.parse().ok()?))
    });
    start(WebSource::from_urls(entries), args)
}

/// [`crate::platform::save_screenshot`] in a browser: PNG, POSTed to `/__screenshot?name=<file name>`.
pub fn post_screenshot(path: &std::path::Path, image: &Image) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "shot.png".into());
    let png = match image.clone().try_into_dynamic() {
        Ok(img) => {
            let mut out = std::io::Cursor::new(Vec::new());
            match img.to_rgb8().write_to(&mut out, image::ImageFormat::Png) {
                Ok(()) => out.into_inner(),
                Err(e) => return error!("screenshot: {e}"),
            }
        }
        Err(e) => return error!("screenshot: {e:?}"),
    };
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&js_sys::Uint8Array::from(&png[..]));
    let url = format!("/__screenshot?name={}", js_sys::encode_uri_component(&name));
    if let Some(w) = web_sys::window() {
        let _ = w.fetch_with_str_and_init(&url, &init);
        info!("Screenshot posted as {name} ({} bytes)", png.len());
    }
}
