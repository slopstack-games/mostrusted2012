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

    /// All files with the given extension (e.g. `"BNDL"`), relative to the root.
    pub fn files_with_extension(&self, ext: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        walk(&self.root, &mut |p| {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
                out.push(p.strip_prefix(&self.root).unwrap_or(p).to_path_buf());
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
    if cfg!(windows) {
        for base in
            ["C:\\Program Files\\EA Games", "C:\\Program Files (x86)\\Origin Games", "C:\\Program Files\\Origin Games"]
        {
            out.push(Path::new(base).join(STEAM_FOLDER_NAME));
            out.push(Path::new(base).join("Need for Speed Most Wanted"));
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
    if cfg!(windows) {
        roots.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
        roots.push(PathBuf::from("C:\\Program Files\\Steam"));
    }
    roots.retain(|p| p.is_dir());
    roots
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
