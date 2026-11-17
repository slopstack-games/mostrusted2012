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

/// The folder the player picked in a browser (`<input type="file" webkitdirectory>`): `File` handles, read on
/// demand with `FileReaderSync`. That API exists only in Web Workers, so the loads must run in one (the main
/// thread gets an error from every read).
#[cfg(target_arch = "wasm32")]
pub struct WebSource {
    index: Index,
    files: HashMap<String, web_sys::File>,
    label: String,
}

// SAFETY: without the `atomics` target feature a wasm module has one thread, so the `File` handles (JS objects,
// tied to the thread that created them) are never touched from another one. A threaded build (the browser port's
// step 3) must not compile this impl: it would need the handles moved to the loading worker instead.
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Send for WebSource {}
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Sync for WebSource {}

#[cfg(target_arch = "wasm32")]
impl std::fmt::Debug for WebSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSource").field("label", &self.label).field("files", &self.files.len()).finish()
    }
}

#[cfg(target_arch = "wasm32")]
impl WebSource {
    /// From a directory picker's `FileList` (each `File` carries its `webkitRelativePath`).
    pub fn from_file_list(list: &web_sys::FileList) -> Self {
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
    pub fn from_files(files: impl IntoIterator<Item = (String, web_sys::File)>) -> Self {
        let (paths, handles): (Vec<String>, Vec<web_sys::File>) = files.into_iter().unzip();
        let label = paths.first().and_then(|p| p.split(['/', '\\']).next()).unwrap_or("<picked folder>").to_string();
        let mut out = Self { index: Index::default(), files: HashMap::new(), label };
        for (rel, file) in install_relative_paths(&paths).into_iter().zip(handles) {
            if let Some(rel) = rel {
                out.index.insert(rel.clone());
                out.files.insert(rel, file);
            }
        }
        out
    }

    fn get(&self, canonical: &str) -> io::Result<&web_sys::File> {
        self.files.get(canonical).ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

#[cfg(target_arch = "wasm32")]
fn js_err(e: wasm_bindgen::JsValue) -> io::Error {
    io::Error::other(format!("{e:?}"))
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
        Ok(self.get(canonical)?.size() as u64)
    }
    fn read_at(&self, canonical: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let file = self.get(canonical)?;
        let end = offset + len as u64;
        if end as f64 > file.size() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let blob = file.slice_with_f64_and_f64(offset as f64, end as f64).map_err(js_err)?;
        let buf = web_sys::FileReaderSync::new().map_err(js_err)?.read_as_array_buffer(&blob).map_err(js_err)?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
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
