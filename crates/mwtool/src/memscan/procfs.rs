//! The Linux side of `memscan`: another process's memory through `/proc/<pid>/mem`, its mappings from
//! `/proc/<pid>/maps`.

use std::fs::File;
use std::os::unix::fs::FileExt;

use anyhow::{Context, Result, bail};

use super::Region;

pub struct Memory {
    pid: u32,
    mem: File,
    /// `/proc/<pid>/mem` opened for writing, only once something is poked.
    mem_rw: std::sync::Mutex<Option<File>>,
}

impl Memory {
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

    /// The pid of the process whose command line names `name` (case-insensitive basename match); with
    /// several, the one using the most memory (the game rather than a launcher stub).
    pub fn find(name: &str) -> Result<u32> {
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
        Ok(pid)
    }

    pub fn regions(&self) -> Result<Vec<Region>> {
        let maps = std::fs::read_to_string(format!("/proc/{}/maps", self.pid))?;
        Ok(maps.lines().filter_map(parse_maps_line).collect())
    }

    pub fn alive(&self) -> bool {
        std::path::Path::new(&format!("/proc/{}", self.pid)).exists()
    }

    pub fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        self.mem.read_exact_at(buf, addr).is_ok()
    }

    /// Writes anywhere the kernel allows: `Process::write` decides where that may be.
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<()> {
        let mut guard = self.mem_rw.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/proc/{}/mem", self.pid))
                .context("opening the process memory for writing")?;
            *guard = Some(f);
        }
        Ok(guard.as_ref().unwrap().write_all_at(data, addr)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lines() {
        let r = parse_maps_line(
            "00401000-00f82000 r-xp 00001000 103:03 78521981                          /home/t/Need for Speed(TM) Most Wanted/NFS13.exe",
        )
        .unwrap();
        assert_eq!(r.path, "/home/t/Need for Speed(TM) Most Wanted/NFS13.exe");
        assert_eq!(r.file_name(), "NFS13.exe");
        assert_eq!(parse_maps_line("010eb000-011be000 rw-p 00000000 00:00 0 ").unwrap().path, "");
    }
}
