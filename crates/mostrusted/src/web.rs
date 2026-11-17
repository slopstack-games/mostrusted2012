//! The browser entry point. The page (`tools/web/index.html`) loads the module, then hands over the install and
//! the command line: either the folder the player picked (`start_with_files`) or a listing of a local server that
//! serves the player's own install (`start_with_listing`, `tools/web/serve.py`). The game data never leaves the
//! player's machine. Flags are the native ones (`--camera=…`, `--units`, `--screenshot`, …).

use std::sync::Arc;

use bevy::prelude::*;
use clap::Parser;
use mw_gamedir::{GameDir, WebSource};
use wasm_bindgen::prelude::*;

fn start(source: WebSource, args: Vec<String>) -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("panic: {info}").into());
    }));
    let cli = crate::Cli::try_parse_from(std::iter::once("mostrusted".to_string()).chain(args))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
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
