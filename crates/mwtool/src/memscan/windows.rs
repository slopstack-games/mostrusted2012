//! The Windows side of `memscan`: another process's memory through `ReadProcessMemory` /
//! `WriteProcessMemory`, its mappings from `VirtualQueryEx`. The Win32 calls are declared by hand, as in
//! `mw-probe` (no dependency on the windows crates).
//!
//! A 64-bit `mwtool` reads the 32-bit game without trouble. The handle is opened for reading only; the write
//! handle is opened the first time something is poked.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::sync::Mutex;

use anyhow::{Result, bail};

use super::Region;

type HANDLE = *mut c_void;
type BOOL = i32;

#[repr(C)]
struct MEMORY_BASIC_INFORMATION {
    BaseAddress: *mut c_void,
    AllocationBase: *mut c_void,
    AllocationProtect: u32,
    #[cfg(target_pointer_width = "64")]
    PartitionId: u16,
    RegionSize: usize,
    State: u32,
    Protect: u32,
    Type: u32,
}

#[repr(C)]
struct PROCESSENTRY32W {
    dwSize: u32,
    cntUsage: u32,
    th32ProcessID: u32,
    th32DefaultHeapID: usize,
    th32ModuleID: u32,
    cntThreads: u32,
    th32ParentProcessID: u32,
    pcPriClassBase: i32,
    dwFlags: u32,
    szExeFile: [u16; 260],
}

#[repr(C)]
struct PROCESS_MEMORY_COUNTERS {
    cb: u32,
    PageFaultCount: u32,
    PeakWorkingSetSize: usize,
    WorkingSetSize: usize,
    QuotaPeakPagedPoolUsage: usize,
    QuotaPagedPoolUsage: usize,
    QuotaPeakNonPagedPoolUsage: usize,
    QuotaNonPagedPoolUsage: usize,
    PagefileUsage: usize,
    PeakPagefileUsage: usize,
}

const PROCESS_VM_OPERATION: u32 = 0x0008;
const PROCESS_VM_READ: u32 = 0x0010;
const PROCESS_VM_WRITE: u32 = 0x0020;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TH32CS_SNAPPROCESS: u32 = 0x2;
const INVALID_HANDLE_VALUE: HANDLE = usize::MAX as HANDLE;
const STILL_ACTIVE: u32 = 259;
const ERROR_ACCESS_DENIED: i32 = 5;

const MEM_COMMIT: u32 = 0x1000;
const MEM_PRIVATE: u32 = 0x2_0000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_WRITECOPY: u32 = 0x08;
const PAGE_EXECUTE: u32 = 0x10;
const PAGE_EXECUTE_READ: u32 = 0x20;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
const PAGE_GUARD: u32 = 0x100;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn ReadProcessMemory(process: HANDLE, addr: *const c_void, buf: *mut c_void, len: usize, read: *mut usize) -> BOOL;
    fn WriteProcessMemory(
        process: HANDLE,
        addr: *const c_void,
        buf: *const c_void,
        len: usize,
        written: *mut usize,
    ) -> BOOL;
    fn VirtualQueryEx(process: HANDLE, addr: *const c_void, out: *mut MEMORY_BASIC_INFORMATION, len: usize) -> usize;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Process32FirstW(snapshot: HANDLE, entry: *mut PROCESSENTRY32W) -> BOOL;
    fn Process32NextW(snapshot: HANDLE, entry: *mut PROCESSENTRY32W) -> BOOL;
    fn K32GetMappedFileNameW(process: HANDLE, addr: *const c_void, out: *mut u16, len: u32) -> u32;
    fn K32GetProcessMemoryInfo(process: HANDLE, out: *mut PROCESS_MEMORY_COUNTERS, len: u32) -> BOOL;
}

/// An owned process or snapshot handle.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and still open.
        unsafe { CloseHandle(self.0) };
    }
}

// SAFETY: a kernel handle is a plain token; the calls made on it here are thread-safe.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

fn open_process(access: u32, pid: u32) -> std::io::Result<Handle> {
    // SAFETY: plain call; a null result is the error.
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(Handle(h)) }
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub struct Memory {
    pid: u32,
    process: Handle,
    /// A second handle with write access, only once something is poked.
    process_rw: Mutex<Option<Handle>>,
}

impl Memory {
    pub fn open(pid: u32) -> Result<Self> {
        let process = open_process(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, pid).map_err(|e| {
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                anyhow::anyhow!(
                    "access denied opening process {pid}. If the game runs elevated, run mwtool from an \
                     administrator terminal too"
                )
            } else {
                anyhow::Error::new(e).context(format!("opening process {pid}"))
            }
        })?;
        Ok(Self { pid, process, process_rw: Mutex::new(None) })
    }

    /// The pid of the process whose executable is `name` (case-insensitive); with several, the one using the
    /// most memory (the game rather than a launcher stub).
    pub fn find(name: &str) -> Result<u32> {
        // SAFETY: plain call; INVALID_HANDLE_VALUE is the error.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(anyhow::Error::new(std::io::Error::last_os_error()).context("listing processes"));
        }
        let snapshot = Handle(snapshot);
        // SAFETY: all-zero is a valid value of this plain struct.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut best: Option<(usize, u32)> = None;
        // SAFETY: `entry` is correctly sized and `dwSize` says so.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            let exe = from_wide(&entry.szExeFile);
            if exe.rsplit(['/', '\\']).next().is_some_and(|b| b.eq_ignore_ascii_case(name)) {
                let pid = entry.th32ProcessID;
                let size = working_set(pid);
                if best.is_none_or(|(s, _)| size > s) {
                    best = Some((size, pid));
                }
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        let Some((_, pid)) = best else { bail!("no running process named {name} (is the game running?)") };
        Ok(pid)
    }

    /// Every committed region. Guard pages are left out (touching one would trip it in the game).
    pub fn regions(&self) -> Result<Vec<Region>> {
        let mut out = Vec::new();
        // The file behind an allocation is the same for all of its regions.
        let mut last_file: Option<(usize, String)> = None;
        let mut at = 0usize;
        loop {
            // SAFETY: all-zero is a valid value of this plain struct.
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: a correctly sized out-struct; the address is only a query key.
            let n = unsafe {
                VirtualQueryEx(
                    self.process.0,
                    at as *const c_void,
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if n == 0 || info.RegionSize == 0 {
                break;
            }
            let start = info.BaseAddress as usize;
            let Some(end) = start.checked_add(info.RegionSize) else { break };
            if info.State == MEM_COMMIT && info.Protect & PAGE_GUARD == 0 {
                let protect = info.Protect & 0xff;
                let alloc = info.AllocationBase as usize;
                let path = if info.Type == MEM_PRIVATE {
                    String::new()
                } else {
                    if last_file.as_ref().is_none_or(|(a, _)| *a != alloc) {
                        last_file = Some((alloc, self.mapped_file(start)));
                    }
                    last_file.as_ref().map(|(_, p)| p.clone()).unwrap_or_default()
                };
                out.push(Region {
                    start: start as u64,
                    end: end as u64,
                    writable: matches!(
                        protect,
                        PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
                    ),
                    executable: matches!(
                        protect,
                        PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
                    ),
                    path,
                });
            }
            if end <= at {
                break;
            }
            at = end;
        }
        if out.is_empty() {
            bail!("no memory regions of process {} could be listed (has it exited?)", self.pid);
        }
        Ok(out)
    }

    /// The NT device path of the file mapped at `addr` (`\Device\HarddiskVolume3\...\NFS13.exe`), or empty.
    fn mapped_file(&self, addr: usize) -> String {
        let mut buf = [0u16; 1024];
        // SAFETY: a correctly sized output buffer.
        let n =
            unsafe { K32GetMappedFileNameW(self.process.0, addr as *const c_void, buf.as_mut_ptr(), buf.len() as u32) };
        from_wide(&buf[..(n as usize).min(buf.len())])
    }

    pub fn alive(&self) -> bool {
        let mut code = 0u32;
        // SAFETY: a valid handle and out-pointer.
        unsafe { GetExitCodeProcess(self.process.0, &mut code) != 0 && code == STILL_ACTIVE }
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        let Ok(addr) = usize::try_from(addr) else { return false };
        let mut read = 0usize;
        // SAFETY: the buffer is ours and `len` is its length; a bad remote address only fails the call.
        let ok = unsafe {
            ReadProcessMemory(self.process.0, addr as *const c_void, buf.as_mut_ptr().cast(), buf.len(), &mut read)
        };
        ok != 0 && read == buf.len()
    }

    /// Writes anywhere the page protection allows: `Process::write` decides where that may be.
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<()> {
        let addr = usize::try_from(addr)?;
        let mut guard = self.process_rw.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let access = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION;
            let h = open_process(access, self.pid)
                .map_err(|e| anyhow::Error::new(e).context("opening the process memory for writing"))?;
            *guard = Some(h);
        }
        let mut written = 0usize;
        // SAFETY: the buffer is ours and `len` is its length; a bad remote address only fails the call.
        let ok = unsafe {
            WriteProcessMemory(
                guard.as_ref().unwrap().0,
                addr as *const c_void,
                data.as_ptr().cast(),
                data.len(),
                &mut written,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if written != data.len() {
            bail!("short write ({written} of {} bytes)", data.len());
        }
        Ok(())
    }
}

/// Working-set size of a process in bytes (0 when it cannot be opened).
fn working_set(pid: u32) -> usize {
    let Ok(h) = open_process(PROCESS_QUERY_LIMITED_INFORMATION, pid) else { return 0 };
    // SAFETY: all-zero is a valid value of this plain struct.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: a correctly sized out-struct.
    if unsafe { K32GetProcessMemoryInfo(h.0, &mut counters, counters.cb) } != 0 { counters.WorkingSetSize } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test process finds itself by name, maps its own executable, and a write lands.
    #[test]
    fn own_process() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        let pid = Memory::find(&name).unwrap();
        assert_eq!(pid, std::process::id());
        let m = Memory::open(pid).unwrap();
        assert!(m.alive());
        let regions = m.regions().unwrap();
        assert!(regions.iter().any(|r| r.file_name().eq_ignore_ascii_case(&name) && r.executable && !r.writable));

        let cell = Box::new(0x1122_3344u32);
        let addr = &*cell as *const u32 as u64;
        assert!(regions.iter().any(|r| r.writable && !r.executable && (r.start..r.end).contains(&addr)));
        m.write(addr, &0x5566_7788u32.to_le_bytes()).unwrap();
        let mut b = [0u8; 4];
        assert!(m.read(addr, &mut b));
        assert_eq!(u32::from_le_bytes(b), 0x5566_7788);
        // SAFETY: a live allocation of ours; volatile because the write came from outside the compiler's view.
        assert_eq!(unsafe { std::ptr::read_volatile(&*cell) }, 0x5566_7788);
        assert!(!m.read(8, &mut b));
    }
}
