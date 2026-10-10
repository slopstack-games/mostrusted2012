//! Finds the user's Need for Speed: Most Wanted (2012) installation.
//!
//! Most Rusted ships no game data. Every asset is read at runtime from a
//! legitimately installed copy, located in this order:
//!
//! 1. an explicit path (e.g. `--game-dir`),
//! 2. the `MW2012_DIR` environment variable,
//! 3. every Steam library (app 1262560), including Flatpak and Windows layouts,
//! 4. the default EA app / Origin install locations.

use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "MW2012_DIR";
pub const STEAM_APP_ID: u32 = 1262560;
pub const STEAM_FOLDER_NAME: &str = "Need for Speed(TM) Most Wanted";
pub const EXECUTABLE: &str = "NFS13.exe";

/// Files that must exist for a directory to count as a valid install.
const REQUIRED_FILES: &[&str] = &[EXECUTABLE, "GLOBALTEXTUREDICTIONARY.BNDL", "SHADERS.BNDL", "WORLDLIST.BNDL"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path} is not a Most Wanted (2012) install: missing {missing}")]
    Invalid { path: PathBuf, missing: String },
    #[error(
        "could not find Need for Speed: Most Wanted (2012).\n\
         Install it through Steam or the EA app, or point to it with --game-dir \
         or the {ENV_VAR} environment variable."
    )]
    NotFound,
}

/// A validated game installation.
#[derive(Debug, Clone)]
pub struct GameDir {
    root: PathBuf,
    version: Option<String>,
}

impl GameDir {
    /// Validate a specific directory.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Error> {
        let root: PathBuf = path.into();
        for f in REQUIRED_FILES {
            if resolve_case_insensitive(&root, f).is_none() {
                return Err(Error::Invalid { path: root, missing: f.to_string() });
            }
        }
        let version = resolve_case_insensitive(&root, "VERSION.TXT")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string());
        Ok(Self { root, version })
    }

    /// Locate the game using the explicit path, env var, then auto-detection.
    pub fn locate(explicit: Option<&Path>) -> Result<Self, Error> {
        if let Some(p) = explicit {
            return Self::open(p);
        }
        if let Some(p) = std::env::var_os(ENV_VAR) {
            return Self::open(PathBuf::from(p));
        }
        candidates().into_iter().find_map(|p| Self::open(p).ok()).ok_or(Error::NotFound)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Contents of `VERSION.TXT` (e.g. `1.4.0.0`).
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Resolve a game-relative path such as `VEHICLES/VEH_1085007_HI.BNDL`,
    /// matching case-insensitively so it works on case-sensitive filesystems.
    pub fn path(&self, relative: &str) -> Option<PathBuf> {
        resolve_case_insensitive(&self.root, relative)
    }

    /// All files with the given extension (e.g. `"BNDL"`), relative to the root. The paths use `/` on every
    /// platform, so they compare equal to game-relative strings such as `VEHICLES/VEH_1085007_HI.BNDL`.
    pub fn files_with_extension(&self, ext: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        walk(&self.root, &mut |p| {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
                let rel = p.strip_prefix(&self.root).unwrap_or(p);
                let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
                out.push(PathBuf::from(parts.join("/")));
            }
        });
        out.sort();
        out
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
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

/// Resolve `relative` (with `/` or `\` separators) under `root`, ignoring case
/// per component.
pub fn resolve_case_insensitive(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in relative.split(['/', '\\']).filter(|c| !c.is_empty()) {
        let direct = cur.join(comp);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let found = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))?;
        cur = found.path();
    }
    cur.exists().then_some(cur)
}

/// Candidate install directories, most likely first.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for steam in steam_roots() {
        for lib in steam_libraries(&steam) {
            out.push(lib.join("steamapps").join("common").join(STEAM_FOLDER_NAME));
        }
    }
    // The EA app / Origin record where they installed the game.
    #[cfg(windows)]
    out.extend(registry::string(
        registry::HKEY_LOCAL_MACHINE,
        "SOFTWARE\\EA Games\\Need for Speed(TM) Most Wanted",
        "Install Dir",
    ));
    if cfg!(windows) {
        for base in program_files() {
            for vendor in ["EA Games", "Origin Games"] {
                out.push(base.join(vendor).join(STEAM_FOLDER_NAME));
                out.push(base.join(vendor).join("Need for Speed Most Wanted"));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".steam/root"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        roots.push(home.join("Library/Application Support/Steam"));
    }
    #[cfg(windows)]
    {
        // Where Steam says it is installed, then the default locations.
        roots.extend(registry::string(registry::HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath"));
        roots.extend(registry::string(registry::HKEY_LOCAL_MACHINE, "SOFTWARE\\Valve\\Steam", "InstallPath"));
        roots.extend(program_files().into_iter().map(|p| p.join("Steam")));
    }
    roots.retain(|p| p.is_dir());
    roots
}

/// The `Program Files` directories (Windows), 32-bit first: Steam and the EA app install there by default.
fn program_files() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    dirs.push(PathBuf::from("C:\\Program Files (x86)"));
    dirs.push(PathBuf::from("C:\\Program Files"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|p| seen.insert(p.clone()));
    dirs
}

/// Reading a string from the registry, declared by hand (no dependency on the windows crates).
#[cfg(windows)]
mod registry {
    use std::ffi::c_void;
    use std::path::PathBuf;

    // Predefined keys are sign-extended 32-bit values.
    pub const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    pub const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x2;
    /// Steam is a 32-bit program: its machine-wide keys are in the 32-bit view.
    const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            ty: *mut u32,
            data: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }

    pub fn string(root: isize, subkey: &str, value: &str) -> Option<PathBuf> {
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (subkey, value) = (wide(subkey), wide(value));
        let mut buf = [0u16; 1024];
        let mut len = std::mem::size_of_val(&buf) as u32;
        // SAFETY: NUL-terminated names, and an output buffer passed with its size in bytes.
        let status = unsafe {
            RegGetValueW(
                root as *mut c_void,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if status != 0 {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..end]);
        (!text.is_empty()).then(|| PathBuf::from(text))
    }
}

/// The Steam root itself plus every library listed in `libraryfolders.vdf`.
fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(vdf) {
        libs.extend(parse_library_paths(&text));
    }
    libs
}

/// Extract every `"path" "<dir>"` value from a `libraryfolders.vdf`.
fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
            (parts.next()? == "path").then(|| parts.next()).flatten()
        })
        .map(|p| PathBuf::from(p.replace("\\\\", "\\")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            parse_library_paths(vdf),
            vec![PathBuf::from("/home/u/.local/share/Steam"), PathBuf::from("D:\\SteamLibrary")]
        );
    }
}
