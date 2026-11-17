//! Where the install's files come from: a directory on disk (native), the folder the player picked in a browser
//! (wasm32), or memory (tests). Every game-file read in the engine-free crates goes through [`Source`], so the
//! same loaders run on both targets.
//!
//! Paths given to a source are *canonical*: relative to the install root, `/`-separated, in the case the files
//! have on disk ([`Source::resolve`] turns any spelling into one).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// A read-only view of the install's files.
pub trait Source: Send + Sync + std::fmt::Debug {
    /// The canonical form of a game-relative path (`/` or `\` separators, any case), if the file exists.
    fn resolve(&self, relative: &str) -> Option<String>;
    /// Every file of the install, canonical.
    fn files(&self) -> Vec<String>;
    /// Size of a file in bytes.
    fn len(&self, canonical: &str) -> io::Result<u64>;
    /// `len` bytes from `offset`; an error if the file is shorter.
    fn read_at(&self, canonical: &str, offset: u64, len: usize) -> io::Result<Vec<u8>>;
    /// The whole file.
    fn read(&self, canonical: &str) -> io::Result<Vec<u8>> {
        let len = self.len(canonical)?;
        self.read_at(canonical, 0, len as usize)
    }
    /// The install directory when the files are on disk (native tools that hand paths to other programs).
    fn dir(&self) -> Option<&Path> {
        None
    }
    /// How the source is named in errors (the directory, or what the browser picked).
    fn label(&self) -> String;
}

/// The install as a directory on disk.
#[derive(Debug, Clone)]
pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Source for DirSource {
    fn resolve(&self, relative: &str) -> Option<String> {
        let full = crate::resolve_case_insensitive(&self.root, relative)?;
        let rel = full.strip_prefix(&self.root).ok()?;
        Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
    }

    fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.root, &mut |p| {
            if let Ok(rel) = p.strip_prefix(&self.root) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        });
        out
    }

    fn len(&self, canonical: &str) -> io::Result<u64> {
        Ok(std::fs::metadata(self.root.join(canonical))?.len())
    }

    fn read_at(&self, canonical: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(self.root.join(canonical))?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0; len];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read(&self, canonical: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.root.join(canonical))
    }

    fn dir(&self) -> Option<&Path> {
        Some(&self.root)
    }

    fn label(&self) -> String {
        self.root.display().to_string()
    }
}

/// On wasm32 there is no filesystem: a directory source holds nothing (the browser uses [`WebSource`]).
#[cfg(target_arch = "wasm32")]
impl Source for DirSource {
    fn resolve(&self, _: &str) -> Option<String> {
        None
    }
    fn files(&self) -> Vec<String> {
        Vec::new()
    }
    fn len(&self, _: &str) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn read_at(&self, _: &str, _: u64, _: usize) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn label(&self) -> String {
        self.root.display().to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

/// Case-insensitive index of canonical paths, for sources that list their files up front.
#[derive(Debug, Default, Clone)]
struct Index(HashMap<String, String>);

impl Index {
    fn key(relative: &str) -> String {
        relative.split(['/', '\\']).filter(|c| !c.is_empty()).collect::<Vec<_>>().join("/").to_ascii_lowercase()
    }
    fn insert(&mut self, canonical: String) {
        self.0.insert(Self::key(&canonical), canonical);
    }
    fn resolve(&self, relative: &str) -> Option<String> {
        self.0.get(&Self::key(relative)).cloned()
    }
}

/// Files held in memory (tests, and small installs assembled by hand).
#[derive(Debug, Default, Clone)]
pub struct MemSource {
    index: Index,
    files: HashMap<String, Vec<u8>>,
}

impl MemSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file under its game-relative path (`/`-separated, in the case it should keep).
    pub fn insert(&mut self, path: &str, data: Vec<u8>) {
        let canonical = path.split(['/', '\\']).filter(|c| !c.is_empty()).collect::<Vec<_>>().join("/");
        self.index.insert(canonical.clone());
        self.files.insert(canonical, data);
    }

    fn get(&self, canonical: &str) -> io::Result<&[u8]> {
        self.files.get(canonical).map(Vec::as_slice).ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

impl Source for MemSource {
    fn resolve(&self, relative: &str) -> Option<String> {
        self.index.resolve(relative)
    }
    fn files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn len(&self, canonical: &str) -> io::Result<u64> {
        Ok(self.get(canonical)?.len() as u64)
    }
    fn read_at(&self, canonical: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let data = self.get(canonical)?;
        usize::try_from(offset)
            .ok()
            .and_then(|o| data.get(o..o.checked_add(len)?))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
    }
    fn label(&self) -> String {
        "<memory>".into()
    }
}

/// Split the picked files' relative paths into the install's: the install root is the shallowest folder holding
/// `SHADERS.BNDL`, so picking the install folder itself or a parent of it both work. Returns, per input in
/// order, the canonical path under that root (`None` for files outside it).
pub fn install_relative_paths(picked: &[String]) -> Vec<Option<String>> {
    let parts = |p: &str| p.split(['/', '\\']).filter(|c| !c.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let prefix = picked
        .iter()
        .map(|p| parts(p))
        .filter(|c| c.last().is_some_and(|n| n.eq_ignore_ascii_case("SHADERS.BNDL")))
        .min_by_key(Vec::len)
        .map(|mut c| {
            c.pop();
            c
        });
    let Some(prefix) = prefix else { return vec![None; picked.len()] };
    picked
        .iter()
        .map(|p| {
            let c = parts(p);
            (c.len() > prefix.len() && c[..prefix.len()] == prefix[..]).then(|| c[prefix.len()..].join("/"))
        })
        .collect()
}

/// The folder the player picked in a browser (`<input type="file" webkitdirectory>`). Each `File` becomes a blob
/// URL (a plain string, so the source is `Send + Sync` and any thread can read it); a read is a synchronous
/// `XMLHttpRequest` for that byte range. In a Web Worker the bytes come back as an `ArrayBuffer`; on the page's
/// main thread (where synchronous requests can't return binary) as `x-user-defined` text, decoded byte by byte:
/// it works, but loads belong on a worker thread.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub struct WebSource {
    index: Index,
    /// Canonical path → (blob or http URL, size in bytes).
    files: HashMap<String, (String, u64)>,
    label: String,
}

#[cfg(target_arch = "wasm32")]
impl WebSource {
    /// From a directory picker's `FileList` (each `File` carries its `webkitRelativePath`). Must run on the
    /// thread that owns the list (the page); the source can then move anywhere.
    pub fn from_file_list(list: &web_sys::FileList) -> io::Result<Self> {
        let files: Vec<web_sys::File> = (0..list.length()).filter_map(|i| list.get(i)).collect();
        let paths = files
            .iter()
            .map(|f| {
                js_sys::Reflect::get(f, &"webkitRelativePath".into())
                    .ok()
                    .and_then(|v| v.as_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| f.name())
            })
            .collect::<Vec<_>>();
        Self::from_files(paths.into_iter().zip(files))
    }

    /// From `(path as picked, File)` pairs; paths are made install-relative with [`install_relative_paths`].
    pub fn from_files(files: impl IntoIterator<Item = (String, web_sys::File)>) -> io::Result<Self> {
        let mut entries = Vec::new();
        for (path, file) in files {
            let url = web_sys::Url::create_object_url_with_blob(&file).map_err(js_err)?;
            entries.push((path, url, file.size() as u64));
        }
        Ok(Self::from_urls(entries))
    }

    /// From `(path, URL, size)` triples: any URL a synchronous range request can read, e.g. a local server
    /// serving the player's own install (`tools/web/serve.py`). Paths are made install-relative with
    /// [`install_relative_paths`].
    pub fn from_urls(entries: impl IntoIterator<Item = (String, String, u64)>) -> Self {
        let entries: Vec<(String, String, u64)> = entries.into_iter().collect();
        let paths: Vec<String> = entries.iter().map(|(p, _, _)| p.clone()).collect();
        let label = paths.first().and_then(|p| p.split(['/', '\\']).next()).unwrap_or("<picked folder>").to_string();
        let mut out = Self { index: Index::default(), files: HashMap::new(), label };
        for (rel, (_, url, size)) in install_relative_paths(&paths).into_iter().zip(entries) {
            if let Some(rel) = rel {
                out.index.insert(rel.clone());
                out.files.insert(rel, (url, size));
            }
        }
        out
    }

    fn get(&self, canonical: &str) -> io::Result<&(String, u64)> {
        self.files.get(canonical).ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

#[cfg(target_arch = "wasm32")]
fn js_err(e: wasm_bindgen::JsValue) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

/// `len` bytes from `offset` of a blob URL, by a synchronous range request.
#[cfg(target_arch = "wasm32")]
fn blob_range(url: &str, offset: u64, len: usize, size: u64) -> io::Result<Vec<u8>> {
    use wasm_bindgen::JsCast;
    let xhr = web_sys::XmlHttpRequest::new().map_err(js_err)?;
    xhr.open_with_async("GET", url, false).map_err(js_err)?;
    let whole = offset == 0 && len as u64 == size;
    if !whole {
        xhr.set_request_header("Range", &format!("bytes={}-{}", offset, offset + len as u64 - 1)).map_err(js_err)?;
    }
    let on_page = js_sys::global().dyn_ref::<web_sys::Window>().is_some();
    let started = on_page.then(js_sys::Date::now);
    if on_page {
        xhr.override_mime_type("text/plain; charset=x-user-defined").map_err(js_err)?;
    } else {
        xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    }
    xhr.send().map_err(js_err)?;
    let status = xhr.status().map_err(js_err)?;
    let bytes = if on_page {
        let text = xhr.response_text().map_err(js_err)?.unwrap_or_default();
        text.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>()
    } else {
        js_sys::Uint8Array::new(&xhr.response().map_err(js_err)?).to_vec()
    };
    // A read on the page's main thread stalls rendering and the audio callback (cpal's WebAudio callbacks run
    // there): report the ones that take a frame or more.
    if let Some(t) = started {
        let ms = js_sys::Date::now() - t;
        if ms >= 16.0 {
            web_sys::console::warn_1(&format!("main-thread game file read: {url} {offset}+{len} in {ms:.0} ms").into());
        }
    }
    match status {
        206 if bytes.len() == len => Ok(bytes),
        // A browser that ignores the range sends the whole file.
        200 if bytes.len() as u64 == size => Ok(bytes[offset as usize..offset as usize + len].to_vec()),
        _ => Err(io::Error::other(format!("blob read {offset}+{len}: status {status}, {} bytes", bytes.len()))),
    }
}

#[cfg(target_arch = "wasm32")]
impl Source for WebSource {
    fn resolve(&self, relative: &str) -> Option<String> {
        self.index.resolve(relative)
    }
    fn files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn len(&self, canonical: &str) -> io::Result<u64> {
        Ok(self.get(canonical)?.1)
    }
    fn read_at(&self, canonical: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let (url, size) = self.get(canonical)?;
        if offset.checked_add(len as u64).is_none_or(|end| end > *size) {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        blob_range(url, offset, len, *size)
    }
    fn label(&self) -> String {
        self.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_root_is_the_shallowest_shaders_folder() {
        let picked: Vec<String> = [
            "Games/NFS/SHADERS.BNDL",
            "Games/NFS/VEHICLES/VEH_1_HI.BNDL",
            "Games/NFS/Support/SHADERS.BNDL",
            "Games/other.txt",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            install_relative_paths(&picked),
            vec![
                Some("SHADERS.BNDL".into()),
                Some("VEHICLES/VEH_1_HI.BNDL".into()),
                Some("Support/SHADERS.BNDL".into()),
                None
            ]
        );
    }

    #[test]
    fn memory_source_resolves_any_case_and_reads_ranges() {
        let mut m = MemSource::new();
        m.insert("VEHICLES/Veh_1.BNDL", (0..10).collect());
        assert_eq!(m.resolve("vehicles\\VEH_1.bndl").as_deref(), Some("VEHICLES/Veh_1.BNDL"));
        assert_eq!(m.read_at("VEHICLES/Veh_1.BNDL", 3, 4).unwrap(), vec![3, 4, 5, 6]);
        assert!(m.read_at("VEHICLES/Veh_1.BNDL", 8, 4).is_err());
        assert_eq!(m.read("VEHICLES/Veh_1.BNDL").unwrap().len(), 10);
    }
}
