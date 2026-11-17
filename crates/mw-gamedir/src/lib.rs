//! Finds the user's Need for Speed: Most Wanted (2012) installation.
//!
//! Most Rusted ships no game data. Every asset is read at runtime from a
//! legitimately installed copy, located in this order:
//!
//! 1. an explicit path (e.g. `--game-dir`),
//! 2. the `MW2012_DIR` environment variable,
//! 3. every Steam library (app 1262560), including Flatpak and Windows layouts,
//! 4. the default EA app / Origin install locations.
//!
//! In a browser there is no filesystem: the player picks the install folder and [`WebSource`] serves it
//! ([`GameDir::from_source`]).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod source;
#[cfg(target_arch = "wasm32")]
pub use source::WebSource;
pub use source::{DirSource, MemSource, Source};

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

/// The retail builds of `NFS13.exe` the ownership check accepts: (release, SHA-256). Hashes only, no game content
/// (CLAUDE.md "Hard rules" allows them). Add a build when a player's copy is confirmed retail.
pub const RETAIL_BUILDS: &[(&str, &str)] =
    &[("Steam 1.4.0.0 (app 1262560)", "3af4dc5f206d3b41d2dcf46ef8ef41575e56834ae73de826708a2a44c44fe971")];

/// The result of [`GameDir::ownership`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    /// A known retail build.
    Retail { release: &'static str },
    /// Not a known build (or no executable): its hash, for adding a confirmed release to [`RETAIL_BUILDS`].
    Unknown { sha256: String },
}

/// A validated game installation: its files come from a [`Source`] (a directory on disk, or the folder picked in
/// a browser), and every read of game data goes through [`GameDir::read`] / [`GameDir::read_at`].
#[derive(Debug, Clone)]
pub struct GameDir {
    source: Arc<dyn Source>,
    /// The install directory (empty when the files are not on disk).
    root: PathBuf,
    version: Option<String>,
}

impl GameDir {
    /// Validate a specific directory.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Error> {
        Self::from_source(Arc::new(DirSource::new(path)))
    }

    /// Validate an install served by any [`Source`].
    pub fn from_source(source: Arc<dyn Source>) -> Result<Self, Error> {
        for f in REQUIRED_FILES {
            if source.resolve(f).is_none() {
                return Err(Error::Invalid { path: PathBuf::from(source.label()), missing: f.to_string() });
            }
        }
        let version = source
            .resolve("VERSION.TXT")
            .and_then(|p| source.read(&p).ok())
            .map(|b| String::from_utf8_lossy(&b).trim().to_string());
        let root = source.dir().map(Path::to_path_buf).unwrap_or_default();
        Ok(Self { source, root, version })
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

    /// The install directory; empty when the files are not on disk (a browser's picked folder).
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn source(&self) -> &Arc<dyn Source> {
        &self.source
    }

    /// Whether this is a retail copy of the game the player owns (user decision, 2026-11-17: the base game counts
    /// as owned when its executable is a known retail build; DLC is never inferred from the files, which ship every
    /// pack: see `mw_logic`'s entitlements). Reads and hashes `NFS13.exe` (~14 MB).
    pub fn ownership(&self) -> Ownership {
        let Ok(exe) = self.read(EXECUTABLE) else { return Ownership::Unknown { sha256: String::new() } };
        use sha2::Digest;
        let digest = sha2::Sha256::digest(&exe);
        let sha256: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        match RETAIL_BUILDS.iter().find(|(_, h)| *h == sha256) {
            Some((release, _)) => Ownership::Retail { release },
            None => Ownership::Unknown { sha256 },
        }
    }

    /// Contents of `VERSION.TXT` (e.g. `1.4.0.0`).
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The canonical form (`/`-separated, the case on disk) of a game-relative path, matched case-insensitively.
    pub fn resolve(&self, relative: impl AsRef<Path>) -> Option<String> {
        self.source.resolve(&relative.as_ref().to_string_lossy())
    }

    /// Resolve a game-relative path such as `VEHICLES/VEH_1085007_HI.BNDL` to a path on disk, matching
    /// case-insensitively. `None` when the install is not a directory: native-only users (tools, ffmpeg) only;
    /// game data is read with [`GameDir::read`].
    pub fn path(&self, relative: &str) -> Option<PathBuf> {
        let dir = self.source.dir()?;
        Some(dir.join(self.source.resolve(relative)?))
    }

    /// A whole file, by game-relative path (any case).
    pub fn read(&self, relative: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        self.source.read(&self.canonical(relative.as_ref())?)
    }

    /// `len` bytes of a file from `offset`, by game-relative path (any case).
    pub fn read_at(&self, relative: impl AsRef<Path>, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        self.source.read_at(&self.canonical(relative.as_ref())?, offset, len)
    }

    /// Size of a file in bytes, by game-relative path (any case).
    pub fn len(&self, relative: impl AsRef<Path>) -> io::Result<u64> {
        self.source.len(&self.canonical(relative.as_ref())?)
    }

    fn canonical(&self, relative: &Path) -> io::Result<String> {
        let rel = relative.to_string_lossy();
        self.source
            .resolve(&rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{rel} is not in the install")))
    }

    /// All files with the given extension (e.g. `"BNDL"`), relative to the root, sorted. The paths use `/` on
    /// every platform, so they compare equal to game-relative strings such as `VEHICLES/VEH_1085007_HI.BNDL`.
    pub fn files_with_extension(&self, ext: &str) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .source
            .files()
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
            .collect();
        out.sort();
        out
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

#[cfg(test)]
mod ownership_tests {
    use super::*;

    /// The install on this machine is the Steam retail build (skipped without an install).
    #[test]
    fn steam_install_is_retail() {
        let Ok(g) = GameDir::locate(None) else { return };
        if g.version() != Some("1.4.0.0") {
            return;
        }
        assert!(matches!(g.ownership(), Ownership::Retail { .. }), "{:?}", g.ownership());
    }

    /// An install whose executable is not a known build is refused, with the hash to report.
    #[test]
    fn unknown_executable_is_not_owned() {
        let dir = std::env::temp_dir().join(format!("mw-gamedir-ownership-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in REQUIRED_FILES {
            std::fs::write(dir.join(f), b"not the game").unwrap();
        }
        let g = GameDir::open(&dir).unwrap();
        let o = g.ownership();
        std::fs::remove_dir_all(&dir).ok();
        match o {
            Ownership::Unknown { sha256 } => assert_eq!(sha256.len(), 64),
            other => panic!("{other:?}"),
        }
    }
}
