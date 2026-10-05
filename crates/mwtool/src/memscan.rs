//! Memory scanner, telemetry recorder and data poker for the running original game (Linux / Proton).
//!
//! This watches the game's *data* while it runs (car speed, drift state, ...) to measure behaviour the
//! handling model has to reproduce, through `/proc/<pid>/mem` of a process the user owns. Scanning and
//! telemetry only read. `poke` writes a data value (to learn what a constant does; docs/probe.md "Tier 0")
//! and remembers the original, which `unpoke` or exit restores; it refuses addresses outside writable,
//! non-executable mappings, so code is never touched (docs/reverse-engineering.md, "Code inside the running
//! game"). Code is never read or disassembled here.
//!
//! Reading another process needs ptrace permission. With `kernel.yama.ptrace_scope = 1` (the usual default)
//! either run `sudo sysctl kernel.yama.ptrace_scope=0` for the session, or give the binary the capability:
//! `sudo setcap cap_sys_ptrace+ep target/release/mwtool` (redo after each build).
//!
//! `memscan` is a line-based REPL (interactive, or commands piped on stdin); `help` lists the commands.
//! Found values are saved to a watch file that `telemetry` samples into CSV. Watch file lines:
//!
//! ```text
//! # name   type  address [offset ...]
//! speed    f32   0x1a2b3c40
//! drift    f32   NFS13.exe+0x9f1234 0x10 0x48     # [[NFS13.exe+0x9f1234] + 0x10] + 0x48
//! ```
//!
//! With offsets, each step reads a 32-bit pointer at the current address and adds the next offset (the game
//! is a 32-bit process), so the chain survives the heap moving between runs.

use std::fs::File;
use std::io::{BufRead, BufWriter, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// One mapping from `/proc/<pid>/maps`.
#[derive(Debug, Clone)]
pub struct Region {
    pub start: u64,
    pub end: u64,
    pub writable: bool,
    pub executable: bool,
    pub path: String,
}

pub struct Process {
    pub pid: u32,
    mem: File,
    /// `/proc/<pid>/mem` opened for writing, only once something is poked.
    mem_rw: std::sync::Mutex<Option<File>>,
}

impl Process {
    pub fn open(pid: u32) -> Result<Self> {
        let path = format!("/proc/{pid}/mem");
        let mem = File::open(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::anyhow!(
                    "permission denied reading {path}. Allow ptrace for this session with \
                     `sudo sysctl kernel.yama.ptrace_scope=0`, or `sudo setcap cap_sys_ptrace+ep <mwtool binary>`"
                )
            } else {
                anyhow::Error::new(e).context(path.clone())
            }
        })?;
        Ok(Self { pid, mem, mem_rw: std::sync::Mutex::new(None) })
    }

    /// The process whose command line names `name` (case-insensitive basename match); with several, the
    /// one using the most memory (the game rather than a launcher stub).
    pub fn find(name: &str) -> Result<Self> {
        let want = name.to_ascii_lowercase();
        let mut best: Option<(u64, u32)> = None;
        for entry in std::fs::read_dir("/proc")? {
            let Ok(pid) = entry?.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
            let hit = cmdline.split(|&b| b == 0).any(|arg| {
                let arg = String::from_utf8_lossy(arg).to_ascii_lowercase();
                arg.rsplit(['/', '\\']).next() == Some(want.as_str())
            });
            if !hit {
                continue;
            }
            let rss = std::fs::read_to_string(format!("/proc/{pid}/statm"))
                .ok()
                .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
                .unwrap_or(0);
            if best.is_none_or(|(r, _)| rss > r) {
                best = Some((rss, pid));
            }
        }
        let Some((_, pid)) = best else { bail!("no running process named {name} (is the game running?)") };
        Self::open(pid)
    }

    pub fn regions(&self) -> Result<Vec<Region>> {
        let maps = std::fs::read_to_string(format!("/proc/{}/maps", self.pid))?;
        Ok(maps.lines().filter_map(parse_maps_line).collect())
    }

    /// Writable regions worth scanning: below 4 GiB (the 32-bit game's address space) unless `all`, and
    /// not device or special mappings.
    pub fn scan_regions(&self, all: bool) -> Result<Vec<Region>> {
        Ok(self
            .regions()?
            .into_iter()
            .filter(|r| r.writable && (all || r.end <= 1 << 32))
            .filter(|r| !(r.path.starts_with("/dev/") || r.path.starts_with("[v") || r.path.starts_with("/memfd:")))
            .collect())
    }

    /// Lowest mapped address of a file whose basename matches `module` (case-insensitive).
    pub fn module_base(&self, module: &str) -> Result<u64> {
        let want = module.to_ascii_lowercase();
        self.regions()?
            .iter()
            .filter(|r| r.path.rsplit('/').next().is_some_and(|b| b.to_ascii_lowercase() == want))
            .map(|r| r.start)
            .min()
            .with_context(|| format!("module {module} is not mapped"))
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        self.mem.read_exact_at(buf, addr).is_ok()
    }

    pub fn read_u32(&self, addr: u64) -> Option<u32> {
        let mut b = [0; 4];
        self.read(addr, &mut b).then(|| u32::from_le_bytes(b))
    }

    pub fn read_value(&self, ty: Ty, addr: u64) -> Option<f64> {
        let mut b = [0; 4];
        let n = ty.size();
        self.read(addr, &mut b[..n]).then(|| ty.decode(&b[..n]))
    }

    /// Write `data` at `addr`, only inside a writable, non-executable mapping below 4 GiB (the game's data;
    /// never its code, never host libraries). `/proc/<pid>/mem` itself would write anywhere, so this check is
    /// the rule.
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<()> {
        let end = addr + data.len() as u64;
        let ok = self
            .regions()?
            .iter()
            .any(|r| r.writable && !r.executable && r.end <= 1 << 32 && addr >= r.start && end <= r.end);
        if !ok {
            bail!("{addr:#010x} is not in a writable, non-executable data mapping of the game: refusing to write");
        }
        let mut guard = self.mem_rw.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/proc/{}/mem", self.pid))
                .context("opening the process memory for writing")?;
            *guard = Some(f);
        }
        guard.as_ref().unwrap().write_all_at(data, addr).with_context(|| format!("writing {addr:#010x}"))
    }
}

/// Data values written into the game with their original bytes, so they can be put back.
#[derive(Default)]
pub struct Pokes(Vec<(u64, Ty, Vec<u8>)>);

impl Pokes {
    /// Write `value` as `ty` at `addr`; the first poke of an address remembers what was there.
    pub fn poke(&mut self, p: &Process, addr: u64, ty: Ty, value: f64) -> Result<f64> {
        let n = ty.size();
        let mut old = vec![0u8; n];
        if !p.read(addr, &mut old) {
            bail!("cannot read {addr:#010x}");
        }
        let new = ty.encode(value);
        p.write(addr, &new[..n])?;
        let before = ty.decode(&old);
        if !self.0.iter().any(|(a, _, _)| *a == addr) {
            self.0.push((addr, ty, old));
        }
        Ok(before)
    }

    /// Restore one address (`Some`) or all of them (`None`); returns how many were restored.
    pub fn restore(&mut self, p: &Process, addr: Option<u64>) -> usize {
        let mut n = 0;
        self.0.retain(|(a, _, old)| {
            if addr.is_some_and(|x| x != *a) {
                return true;
            }
            match p.write(*a, old) {
                Ok(()) => n += 1,
                Err(e) => eprintln!("could not restore {a:#010x}: {e}"),
            }
            false
        });
        n
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn list(&self) -> impl Iterator<Item = (u64, Ty, f64)> + '_ {
        self.0.iter().map(|(a, ty, old)| (*a, *ty, ty.decode(old)))
    }

    /// Drop the record of a poked address without writing anything back: for a field of an object the game
    /// has since freed (the player's car was replaced), where restoring would write into reused memory.
    pub fn forget(&mut self, addr: u64) -> bool {
        let n = self.0.len();
        self.0.retain(|(a, _, _)| *a != addr);
        self.0.len() != n
    }

    /// The value an address held before its first poke, if it was poked.
    pub fn original(&self, addr: u64) -> Option<f64> {
        self.0.iter().find(|(a, _, _)| *a == addr).map(|(_, ty, old)| ty.decode(old))
    }
}

fn parse_maps_line(line: &str) -> Option<Region> {
    // address perms offset dev inode [path]; the path may contain spaces ("Need for Speed(TM) Most Wanted").
    let mut parts = line.splitn(6, ' ');
    let (start, end) = parts.next()?.split_once('-')?;
    let perms = parts.next()?;
    let path = parts.nth(3).unwrap_or("").trim().to_string();
    Some(Region {
        start: u64::from_str_radix(start, 16).ok()?,
        end: u64::from_str_radix(end, 16).ok()?,
        writable: perms.as_bytes().get(1) == Some(&b'w'),
        executable: perms.as_bytes().get(2) == Some(&b'x'),
        path,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    F32,
    I32,
    U8,
}

impl Ty {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "f32" | "float" => Ty::F32,
            "i32" | "int" | "u32" => Ty::I32,
            "u8" | "byte" => Ty::U8,
            _ => bail!("unknown type {s} (f32, i32, u8)"),
        })
    }

    pub fn size(self) -> usize {
        match self {
            Ty::F32 | Ty::I32 => 4,
            Ty::U8 => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Ty::F32 => "f32",
            Ty::I32 => "i32",
            Ty::U8 => "u8",
        }
    }

    pub fn decode(self, b: &[u8]) -> f64 {
        match self {
            Ty::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Ty::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Ty::U8 => b[0] as f64,
        }
    }

    /// Little-endian bytes of `v` as this type (the first `size()` bytes are meaningful).
    pub fn encode(self, v: f64) -> [u8; 4] {
        match self {
            Ty::F32 => (v as f32).to_le_bytes(),
            Ty::I32 => (v as i32).to_le_bytes(),
            Ty::U8 => [v as u8, 0, 0, 0],
        }
    }

    /// Values an unknown-value scan keeps: finite and of sane magnitude for floats.
    fn plausible(self, v: f64) -> bool {
        match self {
            Ty::F32 => v.is_finite() && v.abs() < 1e7 && (v == 0.0 || v.abs() > 1e-20),
            _ => true,
        }
    }
}

/// How a value compares with the previous snapshot (or a constant).
#[derive(Debug, Clone, Copy)]
pub enum Filter {
    Any,
    Changed,
    Unchanged,
    Increased,
    Decreased,
    Eq(f64, f64),
    Range(f64, f64),
    Gt(f64),
    Lt(f64),
}

impl Filter {
    fn keep(self, prev: f64, now: f64) -> bool {
        match self {
            Filter::Any => true,
            Filter::Changed => now != prev,
            Filter::Unchanged => now == prev,
            Filter::Increased => now > prev,
            Filter::Decreased => now < prev,
            Filter::Eq(v, tol) => (now - v).abs() <= tol,
            Filter::Range(a, b) => now >= a && now <= b,
            Filter::Gt(v) => now > v,
            Filter::Lt(v) => now < v,
        }
    }
}

/// Snapshot of one region plus a bitset of candidate slots (one slot per value of the scan type).
struct Block {
    start: u64,
    data: Vec<u8>,
    alive: Vec<u64>,
    count: usize,
}

impl Block {
    fn kill(&mut self, i: usize) {
        self.alive[i / 64] &= !(1 << (i % 64));
    }
    fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.alive.iter().enumerate().flat_map(|(w, &bits)| {
            let mut bits = bits;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(w * 64 + b)
            })
        })
    }
}

/// Candidates below this are re-read value by value instead of re-reading whole regions.
const SPARSE: usize = 50_000;

pub struct Scan {
    pub ty: Ty,
    blocks: Vec<Block>,
    undo: Vec<Vec<(Vec<u64>, usize)>>,
}

impl Scan {
    /// Snapshot every scan region and keep the slots that pass `first` (compared with themselves).
    pub fn new(p: &Process, ty: Ty, first: Filter, all_regions: bool) -> Result<Self> {
        let n = ty.size();
        let mut blocks = Vec::new();
        for r in p.scan_regions(all_regions)? {
            let len = (r.end - r.start) as usize;
            let mut data = vec![0u8; len];
            if !p.read(r.start, &mut data) {
                continue;
            }
            let slots = len / n;
            let mut b = Block { start: r.start, alive: vec![0; slots.div_ceil(64)], data, count: 0 };
            for i in 0..slots {
                let v = ty.decode(&b.data[i * n..i * n + n]);
                let keep = match first {
                    Filter::Any => ty.plausible(v),
                    f => f.keep(v, v),
                };
                if keep {
                    b.alive[i / 64] |= 1 << (i % 64);
                    b.count += 1;
                }
            }
            if b.count > 0 {
                blocks.push(b);
            }
        }
        Ok(Self { ty, blocks, undo: Vec::new() })
    }

    pub fn count(&self) -> usize {
        self.blocks.iter().map(|b| b.count).sum()
    }

    /// Re-read the candidates and keep those that pass `f` against their previous value.
    pub fn refine(&mut self, p: &Process, f: Filter) {
        self.undo.push(self.blocks.iter().map(|b| (b.alive.clone(), b.count)).collect());
        let n = self.ty.size();
        let sparse = self.count() < SPARSE;
        for b in &mut self.blocks {
            if b.count == 0 {
                continue;
            }
            if sparse {
                let slots: Vec<usize> = b.slots().collect();
                for i in slots {
                    let mut buf = [0u8; 4];
                    let ok = p.read(b.start + (i * n) as u64, &mut buf[..n]);
                    let prev = self.ty.decode(&b.data[i * n..i * n + n]);
                    let now = self.ty.decode(&buf[..n]);
                    if ok && f.keep(prev, now) {
                        b.data[i * n..i * n + n].copy_from_slice(&buf[..n]);
                    } else {
                        b.kill(i);
                        b.count -= 1;
                    }
                }
                continue;
            }
            let mut fresh = vec![0u8; b.data.len()];
            if !p.read(b.start, &mut fresh) {
                b.alive.iter_mut().for_each(|w| *w = 0);
                b.count = 0;
                continue;
            }
            let slots: Vec<usize> = b.slots().collect();
            for i in slots {
                let prev = self.ty.decode(&b.data[i * n..i * n + n]);
                let now = self.ty.decode(&fresh[i * n..i * n + n]);
                if !f.keep(prev, now) {
                    b.kill(i);
                    b.count -= 1;
                }
            }
            b.data = fresh;
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        for (b, (alive, count)) in self.blocks.iter_mut().zip(prev) {
            b.alive = alive;
            b.count = count;
        }
        true
    }

    /// The first `limit` candidates as (address, last snapshot value).
    pub fn candidates(&self, limit: usize) -> Vec<(u64, f64)> {
        let n = self.ty.size();
        self.blocks
            .iter()
            .flat_map(|b| b.slots().map(move |i| (b.start + (i * n) as u64, self.ty.decode(&b.data[i * n..i * n + n]))))
            .take(limit)
            .collect()
    }
}

/// A named value to sample: an address, optionally reached through a pointer chain.
#[derive(Debug, Clone)]
pub struct Watch {
    pub name: String,
    pub ty: Ty,
    pub base: Base,
    pub offsets: Vec<u64>,
}

#[derive(Debug, Clone)]
pub enum Base {
    Absolute(u64),
    Module(String, u64),
}

fn parse_num(s: &str) -> Result<u64> {
    let s = s.trim();
    let v = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    };
    v.with_context(|| format!("bad number {s}"))
}

impl Watch {
    /// `name type address [offset ...]`, address = `0x…` or `module+0x…`.
    pub fn parse(line: &str) -> Result<Option<Self>> {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            return Ok(None);
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(ty), Some(addr)) = (parts.next(), parts.next(), parts.next()) else {
            bail!("expected `name type address [offset ...]`: {line}");
        };
        Ok(Some(Self {
            name: name.to_string(),
            ty: Ty::parse(ty)?,
            base: parse_base(addr)?,
            offsets: parts.map(parse_num).collect::<Result<_>>()?,
        }))
    }

    pub fn resolve(&self, p: &Process, modules: &mut ModuleCache) -> Option<u64> {
        let mut addr = match &self.base {
            Base::Absolute(a) => *a,
            Base::Module(m, off) => modules.base(p, m)? + off,
        };
        for off in &self.offsets {
            addr = p.read_u32(addr)? as u64 + off;
        }
        Some(addr)
    }

    fn to_line(&self) -> String {
        let base = match &self.base {
            Base::Absolute(a) => format!("{a:#x}"),
            Base::Module(m, off) => format!("{m}+{off:#x}"),
        };
        let offs: String = self.offsets.iter().map(|o| format!(" {o:#x}")).collect();
        format!("{} {} {base}{offs}", self.name, self.ty.name())
    }
}

fn parse_base(s: &str) -> Result<Base> {
    Ok(match s.split_once('+') {
        Some((m, off)) => Base::Module(m.to_string(), parse_num(off)?),
        None => Base::Absolute(parse_num(s)?),
    })
}

#[derive(Default)]
pub struct ModuleCache(Vec<(String, u64)>);

impl ModuleCache {
    fn base(&mut self, p: &Process, module: &str) -> Option<u64> {
        if let Some((_, b)) = self.0.iter().find(|(m, _)| m.eq_ignore_ascii_case(module)) {
            return Some(*b);
        }
        let b = p.module_base(module).ok()?;
        self.0.push((module.to_string(), b));
        Some(b)
    }
}

pub fn read_watch_file(path: &Path) -> Result<Vec<Watch>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.lines().filter_map(|l| Watch::parse(l).transpose()).collect()
}

/// Apply `--poke` specs (`watch-name=value`, `addr=value`, `addr:i32=value`) and return them for restoring.
pub fn apply_pokes(p: &Process, watches: &[Watch], specs: &[String]) -> Result<Pokes> {
    let mut pokes = Pokes::default();
    let mut modules = ModuleCache::default();
    for spec in specs {
        let (target, value) = spec.split_once('=').with_context(|| format!("--poke {spec}: expected target=value"))?;
        let value: f64 = value.trim().parse().with_context(|| format!("--poke {spec}: bad value"))?;
        let (addr, ty) = match watches.iter().find(|w| w.name == target.trim()) {
            Some(w) => (w.resolve(p, &mut modules).with_context(|| format!("{} does not resolve", w.name))?, w.ty),
            None => {
                let (addr, ty) = match target.split_once(':') {
                    Some((a, t)) => (a, Ty::parse(t.trim())?),
                    None => (target, Ty::F32),
                };
                let addr = match parse_base(addr.trim())? {
                    Base::Absolute(a) => a,
                    Base::Module(m, off) => p.module_base(&m)? + off,
                };
                (addr, ty)
            }
        };
        let before = pokes.poke(p, addr, ty, value)?;
        eprintln!("poked {addr:#010x}: {before} -> {value} (restored at the end)");
    }
    Ok(pokes)
}

/// Describe an address: `module+0x…` when it lies in a mapped file, else the absolute address.
fn describe(regions: &[Region], addr: u64) -> String {
    match regions.iter().find(|r| (r.start..r.end).contains(&addr) && !r.path.is_empty()) {
        Some(r) => {
            let name = r.path.rsplit('/').next().unwrap_or(&r.path);
            let base = regions.iter().filter(|x| x.path == r.path).map(|x| x.start).min().unwrap_or(r.start);
            format!("{addr:#010x} ({name}+{:#x})", addr - base)
        }
        None => format!("{addr:#010x}"),
    }
}

const HELP: &str = "\
scan <f32|i32|u8> [unknown | eq V [tol] | range A B | gt V | lt V]   start a new scan
changed | unchanged | inc | dec                  keep candidates that changed / ... since the last step
eq V [tol] | range A B | gt V | lt V             keep candidates whose current value matches
undo                                             revert the last step
count | list [N]                                 number of candidates / the first N with current values
watch <addr|#i> ... [secs]                       print values every 100 ms (default 5 s); #i = list index
dump <addr> [words]                              words around an address as hex / i32 / f32
ptr <addr> [max_offset]                          find 32-bit pointers to [addr - max_offset, addr]
save <name> <type> <addr|#i|module+off> [offsets...]   append a value to the watch file
poke <addr|#i|module+off|watch-name> <value> [type]     write a data value (type: the watch's, else f32);
                                                 the original is restored by unpoke or at exit
unpoke [addr|all]                                restore poked values; `pokes` lists them
help | quit";

/// The interactive scanner.
pub fn repl(p: &Process, watch_file: &Path, all_regions: bool) -> Result<()> {
    println!("attached to pid {} (read-only); watch file {}; `help` for commands", p.pid, watch_file.display());
    let mut scan: Option<Scan> = None;
    let mut listed: Vec<u64> = Vec::new();
    let mut pokes = Pokes::default();
    let stdin = std::io::stdin();
    let result = loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break Ok(());
        }
        let args: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = args.first() else { continue };
        let mut st = ReplState { scan: &mut scan, listed: &mut listed, pokes: &mut pokes, watch_file, all_regions };
        if let Err(e) = repl_command(p, cmd, &args[1..], &mut st) {
            if e.to_string() == "quit" {
                break Ok(());
            }
            println!("error: {e:#}");
        }
    };
    if !pokes.is_empty() {
        let n = pokes.restore(p, None);
        println!("restored {n} poked value(s)");
    }
    result
}

struct ReplState<'a> {
    scan: &'a mut Option<Scan>,
    listed: &'a mut Vec<u64>,
    pokes: &'a mut Pokes,
    watch_file: &'a Path,
    all_regions: bool,
}

/// `poke` target: a watch name (type from the watch file) or an address.
fn poke_target(arg: &str, ty: Option<&str>, st: &ReplState, p: &Process) -> Result<(u64, Ty)> {
    if let Ok(watches) = read_watch_file(st.watch_file)
        && let Some(w) = watches.iter().find(|w| w.name == arg)
    {
        let addr = w.resolve(p, &mut ModuleCache::default()).with_context(|| format!("{arg} does not resolve"))?;
        return Ok((addr, ty.map(Ty::parse).transpose()?.unwrap_or(w.ty)));
    }
    let ty = match ty {
        Some(t) => Ty::parse(t)?,
        None => st.scan.as_ref().map_or(Ty::F32, |s| s.ty),
    };
    Ok((resolve_addr(arg, st.listed, p)?, ty))
}

fn parse_filter(cmd: &str, args: &[&str], ty: Ty) -> Result<Option<Filter>> {
    let num = |i: usize| -> Result<f64> {
        args.get(i).context("missing value")?.parse::<f64>().with_context(|| format!("bad value {:?}", args.get(i)))
    };
    Ok(Some(match cmd {
        "unknown" => Filter::Any,
        "changed" => Filter::Changed,
        "unchanged" => Filter::Unchanged,
        "inc" => Filter::Increased,
        "dec" => Filter::Decreased,
        "eq" => {
            let v = num(0)?;
            let default_tol = if ty == Ty::F32 { 1e-3 * v.abs().max(1.0) } else { 0.0 };
            Filter::Eq(v, args.get(1).map(|_| num(1)).transpose()?.unwrap_or(default_tol))
        }
        "range" => Filter::Range(num(0)?, num(1)?),
        "gt" => Filter::Gt(num(0)?),
        "lt" => Filter::Lt(num(0)?),
        _ => return Ok(None),
    }))
}

fn resolve_addr(arg: &str, listed: &[u64], p: &Process) -> Result<u64> {
    if let Some(i) = arg.strip_prefix('#') {
        let i: usize = i.parse()?;
        return listed.get(i).copied().with_context(|| format!("no list entry #{i}"));
    }
    match parse_base(arg)? {
        Base::Absolute(a) => Ok(a),
        Base::Module(m, off) => Ok(p.module_base(&m)? + off),
    }
}

fn repl_command(p: &Process, cmd: &str, args: &[&str], st: &mut ReplState) -> Result<()> {
    let ReplState { scan, listed, watch_file, all_regions, .. } = st;
    let (scan, listed, watch_file, all_regions) = (&mut **scan, &mut **listed, *watch_file, *all_regions);
    match cmd {
        "help" | "?" => println!("{HELP}"),
        "quit" | "exit" | "q" => bail!("quit"),
        "poke" => {
            let [target, value, ty @ ..] = args else { bail!("poke <addr|#i|module+off|watch-name> <value> [type]") };
            let value: f64 = value.parse().with_context(|| format!("bad value {value}"))?;
            let (addr, ty) = poke_target(target, ty.first().copied(), st, p)?;
            let before = st.pokes.poke(p, addr, ty, value)?;
            let now = p.read_value(ty, addr).unwrap_or(f64::NAN);
            println!("poked {}: {before} -> {now}", describe(&p.regions()?, addr));
        }
        "unpoke" => {
            let which = match args.first() {
                None | Some(&"all") => None,
                Some(a) => Some(resolve_addr(a, listed, p)?),
            };
            let n = st.pokes.restore(p, which);
            println!("restored {n} value(s)");
        }
        "pokes" => {
            for (a, ty, orig) in st.pokes.list() {
                let now = p.read_value(ty, a).unwrap_or(f64::NAN);
                println!("{}  original {orig}  now {now}", describe(&p.regions()?, a));
            }
        }
        "scan" => {
            let ty = Ty::parse(args.first().context("scan <type> ...")?)?;
            let first = match args.get(1) {
                None => Filter::Any,
                Some(c) => parse_filter(c, &args[2..], ty)?.context("unknown scan condition")?,
            };
            if matches!(first, Filter::Changed | Filter::Unchanged | Filter::Increased | Filter::Decreased) {
                bail!("a new scan needs unknown / eq / range / gt / lt");
            }
            let t = Instant::now();
            let s = Scan::new(p, ty, first, all_regions)?;
            println!("{} candidates ({:.1} s)", s.count(), t.elapsed().as_secs_f32());
            *scan = Some(s);
        }
        "undo" => {
            let s = scan.as_mut().context("no scan")?;
            if s.undo() {
                println!("{} candidates", s.count());
            } else {
                println!("nothing to undo");
            }
        }
        "count" => println!("{} candidates", scan.as_ref().context("no scan")?.count()),
        "list" => {
            let s = scan.as_ref().context("no scan")?;
            let n: usize = args.first().map(|a| a.parse()).transpose()?.unwrap_or(20);
            let regions = p.regions()?;
            listed.clear();
            for (i, (addr, prev)) in s.candidates(n).into_iter().enumerate() {
                let now = p.read_value(s.ty, addr).map_or("?".into(), |v| format!("{v}"));
                println!("#{i:<3} {}  now {now}  (last {prev})", describe(&regions, addr));
                listed.push(addr);
            }
            if s.count() > n {
                println!("... {} more", s.count() - n);
            }
        }
        "watch" => {
            let ty = scan.as_ref().map_or(Ty::F32, |s| s.ty);
            let mut secs = 5.0;
            let mut addrs = Vec::new();
            for a in args {
                if a.starts_with('#') || a.starts_with("0x") || a.contains('+') {
                    addrs.push(resolve_addr(a, listed, p)?);
                } else {
                    secs = a.parse()?;
                }
            }
            let end = Instant::now() + Duration::from_secs_f64(secs);
            while Instant::now() < end {
                let vals: Vec<String> =
                    addrs.iter().map(|&a| p.read_value(ty, a).map_or("?".into(), |v| format!("{v:>12.4}"))).collect();
                println!("{}", vals.join(" "));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        "dump" => {
            let addr = resolve_addr(args.first().context("dump <addr> [words]")?, listed, p)?;
            let words: u64 = args.get(1).map(|a| a.parse()).transpose()?.unwrap_or(16);
            let start = addr.saturating_sub(words / 2 * 4) & !3;
            for i in 0..words {
                let a = start + i * 4;
                let Some(w) = p.read_u32(a) else {
                    println!("{a:#010x}  unreadable");
                    continue;
                };
                let mark = if a == addr { "<" } else { " " };
                println!("{a:#010x}{mark} {w:08x}  {:>12}  {:>14.5}", w as i32, f32::from_bits(w));
            }
        }
        "ptr" => {
            let addr = resolve_addr(args.first().context("ptr <addr> [max_offset]")?, listed, p)?;
            let max_off = args.get(1).map(|a| parse_num(a)).transpose()?.unwrap_or(0x1000);
            let lo = addr.saturating_sub(max_off);
            let regions = p.regions()?;
            let mut hits = 0;
            for r in p.scan_regions(all_regions)? {
                let mut data = vec![0u8; (r.end - r.start) as usize];
                if !p.read(r.start, &mut data) {
                    continue;
                }
                for (i, w) in data.as_chunks::<4>().0.iter().enumerate() {
                    let v = u32::from_le_bytes(*w) as u64;
                    if (lo..=addr).contains(&v) {
                        let at = r.start + i as u64 * 4;
                        println!("{} -> {v:#010x} + {:#x}", describe(&regions, at), addr - v);
                        hits += 1;
                        if hits >= 200 {
                            println!("(stopped at 200)");
                            return Ok(());
                        }
                    }
                }
            }
            println!("{hits} pointers");
        }
        "save" => {
            let [name, ty, addr, offsets @ ..] = args else { bail!("save <name> <type> <addr> [offsets...]") };
            let base =
                if addr.starts_with('#') { Base::Absolute(resolve_addr(addr, listed, p)?) } else { parse_base(addr)? };
            let w = Watch {
                name: name.to_string(),
                ty: Ty::parse(ty)?,
                base,
                offsets: offsets.iter().map(|o| parse_num(o)).collect::<Result<_>>()?,
            };
            let now = w.resolve(p, &mut ModuleCache::default()).and_then(|a| p.read_value(w.ty, a));
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(watch_file)?;
            writeln!(f, "{}", w.to_line())?;
            println!("saved {} (now {now:?})", w.to_line());
        }
        other => {
            let s = scan.as_mut().context("no scan; start with `scan <type> ...`")?;
            let f = parse_filter(other, args, s.ty)?.with_context(|| format!("unknown command {other}; try help"))?;
            let t = Instant::now();
            s.refine(p, f);
            println!("{} candidates ({:.2} s)", s.count(), t.elapsed().as_secs_f32());
        }
    }
    Ok(())
}

/// Sample every watch at `hz` into CSV (`t` in seconds, then one column per watch) until `seconds` pass
/// or the process exits. Unreadable values are left empty.
pub fn telemetry(p: &Process, watches: &[Watch], out: &PathBuf, hz: f64, seconds: Option<f64>) -> Result<()> {
    telemetry_until(p, watches, out, hz, seconds, None)
}

/// [`telemetry`], also stopping when `stop` is set (another thread watching for Enter).
pub fn telemetry_until(
    p: &Process,
    watches: &[Watch],
    out: &PathBuf,
    hz: f64,
    seconds: Option<f64>,
    stop: Option<&std::sync::atomic::AtomicBool>,
) -> Result<()> {
    if watches.is_empty() {
        bail!("the watch file has no entries");
    }
    let mut w = BufWriter::new(File::create(out)?);
    let names: Vec<&str> = watches.iter().map(|w| w.name.as_str()).collect();
    writeln!(w, "t,{}", names.join(","))?;
    let mut modules = ModuleCache::default();
    let period = Duration::from_secs_f64(1.0 / hz.max(1.0));
    let start = Instant::now();
    let mut next = start;
    let mut rows = 0u64;
    eprintln!("recording {} values at {hz} Hz to {} (Ctrl-C to stop)", watches.len(), out.display());
    loop {
        let t = start.elapsed().as_secs_f64();
        let stopped = stop.is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed));
        if stopped || seconds.is_some_and(|s| t >= s) || !Path::new(&format!("/proc/{}", p.pid)).exists() {
            break;
        }
        let vals: Vec<String> = watches
            .iter()
            .map(|wt| {
                wt.resolve(p, &mut modules)
                    .and_then(|a| p.read_value(wt.ty, a))
                    .map_or(String::new(), |v| format!("{v}"))
            })
            .collect();
        writeln!(w, "{t:.4},{}", vals.join(","))?;
        rows += 1;
        if rows.is_multiple_of(hz.max(1.0) as u64) {
            w.flush()?;
        }
        next += period;
        if let Some(d) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        } else {
            next = Instant::now();
        }
    }
    w.flush()?;
    eprintln!("{rows} rows");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scan this test process for a known float, follow it as it changes, and resolve a pointer chain.
    #[test]
    fn scans_own_process() {
        let p = Process::open(std::process::id()).unwrap();
        let mut target = Box::new([0.0f32; 4]);
        target[1] = 1234.5678;
        let addr = &target[1] as *const f32 as u64;
        let mut s = Scan::new(&p, Ty::F32, Filter::Eq(1234.5678, 1e-3), true).unwrap();
        assert!(s.candidates(usize::MAX).iter().any(|(a, _)| *a == addr));
        target[1] = 2000.25;
        std::hint::black_box(&target);
        s.refine(&p, Filter::Increased);
        s.refine(&p, Filter::Eq(2000.25, 1e-3));
        assert!(s.candidates(usize::MAX).iter().any(|(a, _)| *a == addr));
        assert_eq!(p.read_value(Ty::F32, addr), Some(2000.25));

        // A watch through a pointer: [ptr_cell] + 4 lands on target[1]. Only valid when the heap address
        // fits in 32 bits, as it does in the 32-bit game; otherwise check the absolute form only.
        let w = Watch::parse(&format!("x f32 {addr:#x}")).unwrap().unwrap();
        assert_eq!(w.resolve(&p, &mut ModuleCache::default()), Some(addr));
        let base = &target[0] as *const f32 as u64;
        if base < 1 << 32 {
            let cell = Box::new(base as u32);
            let cell_addr = &*cell as *const u32 as u64;
            let w = Watch::parse(&format!("x f32 {cell_addr:#x} 0x4")).unwrap().unwrap();
            assert_eq!(w.resolve(&p, &mut ModuleCache::default()), Some(addr));
        }
    }

    #[test]
    fn watch_lines() {
        let r = parse_maps_line(
            "00401000-00f82000 r-xp 00001000 103:03 78521981                          /home/t/Need for Speed(TM) Most Wanted/NFS13.exe",
        )
        .unwrap();
        assert_eq!(r.path, "/home/t/Need for Speed(TM) Most Wanted/NFS13.exe");
        assert_eq!(parse_maps_line("010eb000-011be000 rw-p 00000000 00:00 0 ").unwrap().path, "");
        let w = Watch::parse("drift f32 NFS13.exe+0x9f1234 0x10 0x48  # comment").unwrap().unwrap();
        assert_eq!(w.to_line(), "drift f32 NFS13.exe+0x9f1234 0x10 0x48");
        assert!(Watch::parse("   # only a comment").unwrap().is_none());
    }
}
