//! The probe: a 32-bit Windows DLL loaded into the user's running copy of the original game, for measurement
//! and verification (docs/probe.md; policy in docs/reverse-engineering.md "Code inside the running game").
//!
//! It stands in for a system DLL the game loads by name (`xinput1_3.dll` by default: feature `proxy-*`),
//! forwards that DLL's exports to the real one in `system32`, and runs its own thread. **Phase 0 (hello)**:
//! the thread logs when it was loaded, when the game's code became stable (the wrapper has unpacked it) and
//! when gameplay began (the vehicle table exists), into `probe.log` in the directory named by the
//! `MW_PROBE_OUT` environment variable (a Unix path; Wine maps `/` to `Z:`). Memory dumps on request
//! (`dump.req`). Capture helpers (`control.rs`, docs/probe.md "Capture helpers"): two vtable slots are pointed at
//! thunks once the game has unpacked itself (the race car's per-tick update and the free-drive "Car Swap Logic"
//! behaviour's per-frame update), for teleports, the car swap, scripted inputs, fixed conditions and a per-tick
//! log. No instruction of the game is rewritten.
//!
//! Only `tools/orig/orig launch --probe` puts this DLL next to the game and enables the DLL override for
//! that run; `orig quit` removes it. Build:
//! `cargo build --release -p mw-probe --target i686-pc-windows-gnu` (on a Windows host:
//! `--target i686-pc-windows-msvc`). Other targets, a 64-bit Windows host included, only type-check it: the
//! exports and `DllMain` exist in the 32-bit Windows build alone.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]
#![cfg_attr(not(all(windows, target_arch = "x86")), allow(dead_code, unused_imports, unused_macros))]

use std::ffi::c_void;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

mod control;

/// Base address of `NFS13.exe` (no ASLR: `DllCharacteristics` has no `DYNAMIC_BASE`).
const IMAGE_BASE: usize = 0x0040_0000;
/// The vehicle update tick (decomp.md "Update loop"); its bytes change once when the wrapper unpacks `.text`.
const TICK_FN: usize = 0x0090_4260;
/// Pointer chain to the vehicle table (`handling.watch`): `[[[NFS13.exe+0xcedb04]+4]+0x53220]`.
const VEHICLE_TABLE_CHAIN: (usize, &[usize]) = (IMAGE_BASE + 0xcedb04, &[0x4, 0x53220]);

// --- the few Win32 calls we need, declared by hand (no dependency on the windows crates) ----------------

type HMODULE = *mut c_void;
type BOOL = i32;

#[repr(C)]
struct MEMORY_BASIC_INFORMATION {
    BaseAddress: *mut c_void,
    AllocationBase: *mut c_void,
    AllocationProtect: u32,
    RegionSize: usize,
    State: u32,
    Protect: u32,
    Type: u32,
}

const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> HMODULE;
    fn GetProcAddress(module: HMODULE, name: *const u8) -> *const c_void;
    fn GetModuleFileNameW(module: HMODULE, out: *mut u16, len: u32) -> u32;
    fn GetSystemDirectoryW(out: *mut u16, len: u32) -> u32;
    fn DisableThreadLibraryCalls(module: HMODULE) -> BOOL;
    fn VirtualQuery(addr: *const c_void, out: *mut MEMORY_BASIC_INFORMATION, len: usize) -> usize;
    fn GetCurrentProcessId() -> u32;
}

#[cfg(not(windows))]
mod stubs {
    //! The crate is a workspace member so `cargo build --workspace` type-checks it on Linux too; the Win32
    //! calls do nothing there (the DLL is only ever built for `i686-pc-windows-gnu`).
    use super::*;
    pub unsafe fn LoadLibraryW(_: *const u16) -> HMODULE {
        std::ptr::null_mut()
    }
    pub unsafe fn GetProcAddress(_: HMODULE, _: *const u8) -> *const c_void {
        std::ptr::null()
    }
    pub unsafe fn GetModuleFileNameW(_: HMODULE, _: *mut u16, _: u32) -> u32 {
        0
    }
    pub unsafe fn GetSystemDirectoryW(_: *mut u16, _: u32) -> u32 {
        0
    }
    pub unsafe fn DisableThreadLibraryCalls(_: HMODULE) -> BOOL {
        0
    }
    pub unsafe fn VirtualQuery(_: *const c_void, _: *mut MEMORY_BASIC_INFORMATION, _: usize) -> usize {
        0
    }
    pub unsafe fn GetCurrentProcessId() -> u32 {
        0
    }
}
#[cfg(not(windows))]
use stubs::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Is every byte of `[addr, addr + len)` committed and readable? (Keeps the readiness poll from faulting on
/// pages the wrapper has not mapped yet.)
fn readable(addr: usize, len: usize) -> bool {
    if addr == 0 || addr.checked_add(len).is_none() {
        return false;
    }
    let mut at = addr;
    while at < addr + len {
        let mut info = MEMORY_BASIC_INFORMATION {
            BaseAddress: std::ptr::null_mut(),
            AllocationBase: std::ptr::null_mut(),
            AllocationProtect: 0,
            RegionSize: 0,
            State: 0,
            Protect: 0,
            Type: 0,
        };
        // SAFETY: plain query of our own address space with a correctly sized out-struct.
        let n =
            unsafe { VirtualQuery(at as *const c_void, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 || info.Protect == 0 {
            return false;
        }
        at = info.BaseAddress as usize + info.RegionSize;
    }
    true
}

fn read_u32(addr: usize) -> Option<u32> {
    // SAFETY: the page is committed and readable (checked), and a 32-bit load at any alignment is fine on x86.
    readable(addr, 4).then(|| unsafe { std::ptr::read_volatile(addr as *const u32) })
}

/// FNV-1a of the bytes at `addr` (what we log instead of the bytes themselves: protected code is never written
/// out, only compared).
fn hash_at(addr: usize, len: usize) -> Option<u32> {
    if !readable(addr, len) {
        return None;
    }
    let mut h: u32 = 0x811c_9dc5;
    for i in 0..len {
        // SAFETY: readable checked above.
        let b = unsafe { std::ptr::read_volatile((addr + i) as *const u8) };
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    Some(h)
}

/// Follow a `handling.watch`-style pointer chain; `None` as soon as a pointer is not readable.
fn follow(base: usize, offsets: &[usize]) -> Option<usize> {
    let mut addr = base;
    for off in offsets {
        addr = read_u32(addr)? as usize;
        if addr < 0x1_0000 {
            return None;
        }
        addr += off;
    }
    Some(addr)
}

// --- forwarding to the real system DLL ----------------------------------------------------------------

/// Each export is a naked stub that jumps through a pointer resolved in `DllMain` from the real DLL, so any
/// calling convention and argument list passes through untouched.
macro_rules! forward {
    ($dll:literal: $($name:ident)*) => {
        pub const PROXY_DLL: &str = $dll;
        pub const NAMES: &[&str] = &[$(stringify!($name)),*];
        $(
            #[allow(non_snake_case)]
            pub mod $name {
                pub static mut TARGET: usize = 0;
            }
            #[cfg(all(windows, target_arch = "x86"))]
            #[unsafe(naked)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name() {
                core::arch::naked_asm!("jmp dword ptr [{t}]", t = sym $name::TARGET)
            }
        )*
        /// Resolve every export from the real DLL; returns the names that were not found.
        pub fn resolve(real: HMODULE) -> Vec<&'static str> {
            let mut missing = Vec::new();
            $(
                // SAFETY: `real` is a loaded module; the name is NUL-terminated. The static is written once,
                // before any stub can be called (the game gets our exports only after DllMain returns).
                unsafe {
                    let p = GetProcAddress(real, concat!(stringify!($name), "\0").as_ptr());
                    if p.is_null() {
                        missing.push(stringify!($name));
                    } else {
                        $name::TARGET = p as usize;
                    }
                }
            )*
            missing
        }
    };
}

#[cfg(feature = "proxy-xinput1_3")]
mod proxy {
    use super::*;
    forward!("xinput1_3.dll":
        XInputEnable XInputGetBatteryInformation XInputGetCapabilities XInputGetDSoundAudioDeviceGuid
        XInputGetKeystroke XInputGetState XInputSetState
    );
}

#[cfg(all(feature = "proxy-dinput8", not(feature = "proxy-xinput1_3")))]
mod proxy {
    use super::*;
    forward!("dinput8.dll":
        DirectInput8Create DllCanUnloadNow DllGetClassObject DllRegisterServer DllUnregisterServer
    );
}

// --- the log ------------------------------------------------------------------------------------------

struct Log {
    file: Option<std::fs::File>,
    t0: Instant,
}

static LOG: OnceLock<std::sync::Mutex<Log>> = OnceLock::new();

/// `MW_PROBE_OUT` is a Unix path; the Wine `Z:` drive is the Unix root.
fn out_dir() -> Option<String> {
    let dir = std::env::var("MW_PROBE_OUT").ok()?;
    Some(if dir.starts_with('/') { format!("Z:{}", dir.replace('/', "\\")) } else { dir })
}

fn log(msg: &str) {
    let lock = LOG.get_or_init(|| {
        let file = out_dir().and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::OpenOptions::new().create(true).append(true).open(format!("{dir}\\probe.log")).ok()
        });
        std::sync::Mutex::new(Log { file, t0: Instant::now() })
    });
    let mut l = lock.lock().unwrap_or_else(|e| e.into_inner());
    let t = l.t0.elapsed().as_secs_f64();
    if let Some(f) = l.file.as_mut() {
        let _ = writeln!(f, "[+{t:8.3}s] {msg}");
        let _ = f.flush();
    }
}

// --- memory dumps on request ---------------------------------------------------------------------------

/// Parse an address term: `NFS13.exe+0x…` (image-relative), or a plain hex / decimal number.
fn parse_addr(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("NFS13.exe+").or_else(|| s.strip_prefix("nfs13.exe+")) {
        return parse_addr(rest).map(|a| IMAGE_BASE + a);
    }
    match s.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Serve `dump.req` in the output directory: each line `name base [offset ...] length` reads `length` bytes at
/// the address the chain gives (the `handling.watch` convention: `base`, then for each offset read a pointer
/// there and add the offset) into `name.bin`, plus a line in `dump.txt`. The request is renamed `dump.done`.
/// Read-only: game memory is only read, through the same `readable` check as everything else.
fn serve_dump_requests() {
    let Some(dir) = out_dir() else { return };
    let req = format!("{dir}\\dump.req");
    let Ok(text) = std::fs::read_to_string(&req) else { return };
    let _ = std::fs::rename(&req, format!("{dir}\\dump.done"));
    let mut report = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 3 {
            let _ = writeln!(report, "{line}: needs `name base [offset ...] length`");
            continue;
        }
        let name = words[0];
        let parsed: Option<Vec<usize>> = words[1..].iter().map(|w| parse_addr(w)).collect();
        let Some(nums) = parsed else {
            let _ = writeln!(report, "{name}: bad number in `{line}`");
            continue;
        };
        let (len, chain) = (nums[nums.len() - 1].min(1 << 22), &nums[..nums.len() - 1]);
        let addr = if chain.len() == 1 { Some(chain[0]) } else { follow(chain[0], &chain[1..]) };
        match addr {
            Some(a) if readable(a, len) => {
                let mut buf = vec![0u8; len];
                // SAFETY: the range was checked readable just above.
                unsafe { std::ptr::copy_nonoverlapping(a as *const u8, buf.as_mut_ptr(), len) };
                let ok = std::fs::write(format!("{dir}\\{name}.bin"), &buf).is_ok();
                let _ = writeln!(report, "{name} {a:#010x} {len:#x} {}", if ok { "ok" } else { "write failed" });
            }
            Some(a) => {
                let _ = writeln!(report, "{name} {a:#010x} {len:#x} unreadable");
            }
            None => {
                let _ = writeln!(report, "{name}: chain did not resolve");
            }
        }
    }
    let _ = std::fs::write(format!("{dir}\\dump.txt"), &report);
    log(&format!("served dump request: {} line(s)", report.lines().count()));
}

// --- the hello thread ---------------------------------------------------------------------------------

fn hello(real_loaded: bool, missing: Vec<&'static str>) {
    let mut exe = [0u16; 1024];
    // SAFETY: a correctly sized output buffer.
    let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), exe.as_mut_ptr(), exe.len() as u32) } as usize;
    let mut first = String::new();
    let _ = write!(
        first,
        "probe loaded as {} into pid {} ({}); real DLL {}",
        proxy::PROXY_DLL,
        unsafe { GetCurrentProcessId() },
        from_wide(&exe[..n.min(exe.len())]),
        if real_loaded { "loaded" } else { "NOT loaded" }
    );
    if !missing.is_empty() {
        let _ = write!(first, "; exports not found in the real DLL: {}", missing.join(" "));
    }
    log(&first);
    log(&format!("forwarding {} export(s): {}", proxy::NAMES.len(), proxy::NAMES.join(" ")));

    // Readiness: the tick function's bytes change once (unpacking) and then stay; gameplay exists once the
    // vehicle table pointer chain resolves.
    let start = Instant::now();
    let mut last_hash = hash_at(TICK_FN, 16);
    let mut last_change = Instant::now();
    let mut reported_stable = false;
    let mut reported_gameplay = false;
    let mut next_beat = Duration::from_secs(5);
    let mut next_request = Duration::ZERO;
    let mut next_control = Duration::ZERO;
    let mut next_status = Duration::ZERO;
    let dir = out_dir();
    log(&format!("tick fn {TICK_FN:#010x}: initial hash {last_hash:08x?}"));
    loop {
        std::thread::sleep(Duration::from_millis(10));
        let h = hash_at(TICK_FN, 16);
        if h != last_hash {
            log(&format!("tick fn bytes changed: {last_hash:08x?} -> {h:08x?}"));
            last_hash = h;
            last_change = Instant::now();
            reported_stable = false;
        } else if !reported_stable && h.is_some() && last_change.elapsed() > Duration::from_millis(500) {
            log(&format!("tick fn bytes stable for 0.5 s (hash {:08x})", h.unwrap_or(0)));
            reported_stable = true;
        }
        let table = follow(VEHICLE_TABLE_CHAIN.0, VEHICLE_TABLE_CHAIN.1);
        if !reported_gameplay && table.is_some() {
            log(&format!("gameplay: vehicle table at {:#010x}", table.unwrap_or(0)));
            reported_gameplay = true;
        } else if reported_gameplay && table.is_none() {
            log("vehicle table gone (menu / loading)");
            reported_gameplay = false;
        }
        if start.elapsed() >= next_request {
            serve_dump_requests();
            next_request = start.elapsed() + Duration::from_millis(200);
        }
        if reported_stable && start.elapsed() >= next_control {
            if let Some(dir) = &dir {
                control::service(dir);
                if start.elapsed() >= next_status {
                    control::write_status(dir);
                    control::flush_running();
                    next_status = start.elapsed() + Duration::from_millis(500);
                }
            }
            next_control = start.elapsed() + Duration::from_millis(50);
        }
        if start.elapsed() >= next_beat {
            log(&format!(
                "alive: code {}, {}",
                if reported_stable { "stable" } else { "changing" },
                if reported_gameplay { "in gameplay" } else { "no vehicle table" }
            ));
            next_beat += Duration::from_secs(if next_beat < Duration::from_secs(60) { 5 } else { 60 });
        }
    }
}

fn attach(module: HMODULE) {
    // SAFETY: standard DllMain housekeeping on our own module handle.
    unsafe { DisableThreadLibraryCalls(module) };
    let mut sysdir = [0u16; 512];
    // SAFETY: a correctly sized output buffer.
    let n = unsafe { GetSystemDirectoryW(sysdir.as_mut_ptr(), sysdir.len() as u32) } as usize;
    let real_path = format!("{}\\{}", from_wide(&sysdir[..n.min(sysdir.len())]), proxy::PROXY_DLL);
    // SAFETY: a NUL-terminated wide path. Loading by full path gets the system DLL (Wine's builtin behind its
    // placeholder file), not ourselves.
    let real = unsafe { LoadLibraryW(wide(&real_path).as_ptr()) };
    let missing = if real.is_null() { proxy::NAMES.to_vec() } else { proxy::resolve(real) };
    let real_loaded = !real.is_null();
    std::thread::spawn(move || hello(real_loaded, missing));
}

#[cfg(all(windows, target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    match reason {
        DLL_PROCESS_ATTACH => attach(module),
        DLL_PROCESS_DETACH => {
            control::uninstall_hooks();
            log("probe unloading")
        }
        _ => {}
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_path_becomes_z_drive() {
        unsafe { std::env::set_var("MW_PROBE_OUT", "/home/x/ref/run1") };
        assert_eq!(out_dir().as_deref(), Some("Z:\\home\\x\\ref\\run1"));
    }
}
