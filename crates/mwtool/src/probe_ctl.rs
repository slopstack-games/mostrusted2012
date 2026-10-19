//! Commands that steer the running original through the probe DLL (docs/probe.md "Capture helpers"):
//! `mwtool probe`, `tp`, `car`, `conditions`, `inputs` and `capture --probe`.
//!
//! The probe (crates/mw-probe, `tools/orig/orig launch --probe <name>`) reads `cmd.req` in its output directory
//! (`ref/probe/<name>/`; `orig` records it in `$XDG_RUNTIME_DIR/mostrusted-orig/probe.out`) and answers in
//! `cmd.out`; `status.txt` there is refreshed twice a second.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Named teleport spots: `name x ground_y z heading_deg  # note`, committed with the tool.
pub const SPOTS: &str = "tools/orig/spots.txt";

/// The time of day (seconds) and its rate: environment manager + 0x1b604 / + 0x1b610, the manager being the
/// renderer (`[0x01183168]`) − 0x80 (research log 2026-10-14 "lighting against the original").
const TOD: &str = "0x01183168 0x1b584";
const TOD_RATE: &str = "0x01183168 0x1b590";
/// Player 0's nitrous amount (0..100): game `[0x01177868]` + 0x264f0 + 0x10 (decomp.md "Nitrous refill").
const NITROUS: &str = "0x01177868 0x26500";

pub fn probe_dir(given: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = given {
        return Ok(d.to_path_buf());
    }
    let state = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let f = state.join("mostrusted-orig").join("probe.out");
    let d = std::fs::read_to_string(&f).with_context(|| {
        format!(
            "{} not found: start the game with `tools/orig/orig launch --probe <name>` (or pass --probe-dir)",
            f.display()
        )
    })?;
    Ok(PathBuf::from(d.trim()))
}

/// Send command lines to the probe and wait for its answer.
pub fn send(dir: &Path, lines: &[String], timeout: f32) -> Result<String> {
    let id = format!(
        "id {}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
    );
    let out = dir.join("cmd.out");
    let _ = std::fs::remove_file(&out);
    let tmp = dir.join("cmd.req.tmp");
    std::fs::write(&tmp, format!("{id}\n{}\n", lines.join("\n")))?;
    std::fs::rename(&tmp, dir.join("cmd.req"))?;
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < timeout {
        if let Ok(text) = std::fs::read_to_string(&out)
            && text.starts_with(&id)
            && text.ends_with("end\n")
        {
            let body = text[id.len()..].trim_start_matches('\n').trim_end_matches("end\n").to_string();
            return Ok(body);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    bail!("no answer from the probe in {} within {timeout} s (is the game running with --probe?)", dir.display())
}

fn send_ok(dir: &Path, lines: &[String]) -> Result<String> {
    let r = send(dir, lines, 30.0)?;
    print!("{r}");
    if r.lines().any(|l| l.starts_with("error")) {
        bail!("the probe reported an error");
    }
    Ok(r)
}

pub fn status(dir: &Path) -> Result<std::collections::HashMap<String, String>> {
    let text = std::fs::read_to_string(dir.join("status.txt")).context("no status.txt from the probe yet")?;
    Ok(text.lines().filter_map(|l| l.split_once(' ')).map(|(k, v)| (k.to_string(), v.trim().to_string())).collect())
}

pub fn raw(dir: &Path, words: &[String]) -> Result<()> {
    send_ok(dir, &[words.join(" ")])?;
    Ok(())
}

/// A spot: (x, ground y, z, heading in degrees).
type Spot = [f32; 4];

fn read_spots(repo: &Path) -> Vec<(String, Spot, String)> {
    let text = std::fs::read_to_string(repo.join(SPOTS)).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let (data, note) = l.split_once('#').map_or((l, ""), |(a, b)| (a, b.trim()));
            let w: Vec<&str> = data.split_whitespace().collect();
            let v: Vec<f32> = w.get(1..5)?.iter().map(|x| x.parse().ok()).collect::<Option<_>>()?;
            Some((w[0].to_string(), [v[0], v[1], v[2], v[3]], note.to_string()))
        })
        .collect()
}

pub fn spot(name: &str) -> Result<Spot> {
    if let Ok(v) = name.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<Vec<_>, _>>() {
        return match v[..] {
            [x, y, z] => Ok([x, y, z, 0.0]),
            [x, y, z, h] => Ok([x, y, z, h]),
            _ => bail!("a spot is a name from {SPOTS} or x,ground_y,z[,heading_deg]"),
        };
    }
    read_spots(Path::new("."))
        .into_iter()
        .find(|s| s.0 == name)
        .map(|s| s.1)
        .with_context(|| format!("no spot {name} in {SPOTS} (`mwtool tp --list`)"))
}

pub fn tp(
    dir: &Path,
    target: Option<&str>,
    save: Option<&str>,
    list: bool,
    kmh: f32,
    raw: bool,
    body: bool,
) -> Result<()> {
    if list {
        for (n, s, note) in read_spots(Path::new(".")) {
            println!("{n:<20} {:>9.2} {:>7.2} {:>9.2} {:>7.1}  {note}", s[0], s[1], s[2], s[3]);
        }
        return Ok(());
    }
    if let Some(name) = save {
        let st = status(dir)?;
        let num = |k: &str| -> Result<Vec<f32>> {
            st.get(k)
                .with_context(|| format!("status has no {k} (no car?)"))?
                .split_whitespace()
                .map(|x| x.parse::<f32>().map_err(Into::into))
                .collect()
        };
        let (pos, ground, heading) = (num("pos")?, num("ground")?[0], num("heading_deg")?[0]);
        let mut text = std::fs::read_to_string(SPOTS).unwrap_or_default();
        text = text.lines().filter(|l| l.split_whitespace().next() != Some(name)).map(|l| format!("{l}\n")).collect();
        text.push_str(&format!(
            "{name} {:.3} {ground:.3} {:.3} {heading:.2}  # saved from the original\n",
            pos[0], pos[2]
        ));
        std::fs::write(SPOTS, text)?;
        println!("saved {name}: {:.3} {ground:.3} {:.3} heading {heading:.2}", pos[0], pos[2]);
        return Ok(());
    }
    let Some(target) = target else { bail!("tp <spot | x,ground_y,z[,heading]> (or --save NAME, --list)") };
    let s = spot(target)?;
    let line = if raw {
        let cmd = if body { "tpabs" } else { "tp" };
        format!("{cmd} {} {} {} {} {}", s[0], s[1], s[2], s[3], kmh / 3.6)
    } else {
        format!("place {} {} {} {} {}", s[0], s[1], s[2], s[3], kmh / 1.609_344)
    };
    send_ok(dir, &[line])?;
    Ok(())
}

/// Travel to a far spot in raw hops of `step` metres (each put on the collision mesh's ground and given 2 s to
/// stream), then a last raw teleport onto the spot. The game's own placement refuses some areas (it puts the car
/// back on the free-roam start instead: the airfield, research log 2026-10-19), and one long raw jump lands
/// outside the streamed world, which the game answers the same way.
pub fn hop(dir: &Path, store: &mw_assets::AssetStore, target: &str, step: f32) -> Result<()> {
    // Waypoints: `a;b;c`, each a spot.
    let mut targets: Vec<&str> = target.split(';').collect();
    let last = targets.pop().context("no target")?;
    for t in targets {
        hop_to(dir, store, t, step, false)?;
    }
    hop_to(dir, store, last, step, true)
}

fn hop_to(dir: &Path, store: &mw_assets::AssetStore, target: &str, step: f32, exact: bool) -> Result<()> {
    let goal = spot(target)?;
    for _ in 0..200 {
        let st = status(dir)?;
        let pos: Vec<f32> = st
            .get("pos")
            .context("status has no position (no car?)")?
            .split_whitespace()
            .map(|x| x.parse::<f32>())
            .collect::<Result<_, _>>()?;
        let (dx, dz) = (goal[0] - pos[0], goal[2] - pos[2]);
        let d = dx.hypot(dz);
        if d <= step {
            let h = if exact { goal[3] } else { dx.atan2(dz).to_degrees() };
            send_ok(dir, &[format!("tp {} {} {} {} 0", goal[0], goal[1], goal[2], h)])?;
            std::thread::sleep(Duration::from_secs(2));
            return Ok(());
        }
        let (x, z) = (pos[0] + dx / d * step, pos[2] + dz / d * step);
        let y = crate::flat_strip::ground_at(store, x, z, pos[1] + 60.0)?
            .with_context(|| format!("no ground at {x:.1}, {z:.1}"))?;
        let heading = dx.atan2(dz).to_degrees();
        send_ok(dir, &[format!("tp {x} {y} {z} {heading} 0")])?;
        std::thread::sleep(Duration::from_secs(2));
        let st = status(dir)?;
        let now: Vec<f32> =
            st.get("pos").map(|p| p.split_whitespace().filter_map(|v| v.parse().ok()).collect()).unwrap_or_default();
        if now.len() < 3 || (now[0] - x).hypot(now[2] - z) > 20.0 {
            bail!("the car did not stay at the hop ({x:.1}, {z:.1}): the game moved it back (now {now:?})");
        }
    }
    bail!("too many hops")
}

/// A car by vehicle id or (part of) its name in the vehicle list.
pub fn car_id(store: &mw_assets::AssetStore, q: &str) -> Result<u32> {
    if let Ok(id) = q.parse::<u32>() {
        return Ok(id);
    }
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE)?;
    let ql = q.to_lowercase();
    let hits: Vec<_> = store
        .vehicles(&lang)?
        .into_iter()
        .filter(|v| v.hi_bundle.is_some() && v.name.to_lowercase().contains(&ql))
        .collect();
    match hits.as_slice() {
        [v] => Ok(v.id),
        [] => bail!("no car matches {q} (`mwtool vehicles`)"),
        many => {
            bail!("{q} matches {}", many.iter().map(|v| format!("{} {}", v.id, v.name)).collect::<Vec<_>>().join(", "))
        }
    }
}

pub fn car(dir: &Path, id: u32) -> Result<()> {
    send_ok(dir, &[format!("car {id}")])?;
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct Conditions {
    pub hour: Option<f32>,
    pub no_traffic: bool,
    pub repair: bool,
    pub nitrous: Option<f32>,
}

impl Conditions {
    fn lines(&self) -> Vec<String> {
        let mut l = Vec::new();
        if let Some(h) = self.hour {
            l.push(format!("poke f32 0 {TOD_RATE}"));
            l.push(format!("hold f32 {} {TOD}", h * 3600.0));
        }
        if self.no_traffic {
            l.push("notraffic".into());
        }
        if self.repair {
            l.push("repair".into());
        }
        if let Some(n) = self.nitrous {
            l.push(format!("hold f32 {n} {NITROUS}"));
        }
        l
    }
}

pub fn conditions(dir: &Path, c: &Conditions, release: bool) -> Result<()> {
    let mut lines = c.lines();
    if release {
        lines.insert(0, "release".into());
        // The clock runs again at the free-roam rate (24 game seconds per second).
        lines.push(format!("poke f32 24 {TOD_RATE}"));
    }
    if lines.is_empty() {
        bail!("nothing to set: --hour, --no-traffic, --repair, --nitrous, --release");
    }
    send_ok(dir, &lines)?;
    Ok(())
}

/// Write an input script: each segment `seconds:steer,throttle,brake,handbrake,nitrous` holds those values for
/// that long; a last row with zero inputs marks the end.
pub fn inputs(out: &Path, segments: &[String]) -> Result<()> {
    let mut text = String::from("t,steer,throttle,brake,handbrake,nitrous\n");
    let mut t = 0.0f64;
    for s in segments {
        let (secs, vals) = s.split_once(':').context("segment: seconds:steer,throttle,brake,handbrake,nitrous")?;
        let v: Vec<f32> = vals.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
        let get = |i: usize| v.get(i).copied().unwrap_or(0.0);
        text.push_str(&format!("{t:.4},{},{},{},{},{}\n", get(0), get(1), get(2), get(3), get(4)));
        t += secs.parse::<f64>()?;
    }
    text.push_str(&format!("{t:.4},0,0,0,0,0\n"));
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(out, text)?;
    println!("wrote {} ({t:.2} s)", out.display());
    Ok(())
}

pub struct ProbeCapture<'a> {
    pub name: &'a str,
    pub watch: &'a Path,
    pub script: Option<&'a Path>,
    pub spot: Option<&'a str>,
    pub settle: f32,
    /// Start at this speed (km/h): the placement and the script start together (no settling).
    pub kmh: Option<f32>,
    pub seconds: Option<f64>,
    pub conditions: Conditions,
}

/// Record a run tick by tick from inside the game: optionally fix the conditions, teleport to a spot, wait for
/// the car to settle, then play the input script and log the watch file's fields every tick into
/// `ref/<name>/telemetry.csv`.
pub fn capture(dir: &Path, c: &ProbeCapture) -> Result<()> {
    let run = PathBuf::from("ref").join(c.name);
    std::fs::create_dir_all(&run)?;
    let run = run.canonicalize()?;
    let watch = c.watch.canonicalize().with_context(|| format!("watch file {}", c.watch.display()))?;
    let lines = c.conditions.lines();
    if !lines.is_empty() {
        send_ok(dir, &lines)?;
    }
    let spot_pose = c.spot.map(spot).transpose()?;
    if let (Some(s), None) = (spot_pose, c.kmh) {
        send_ok(dir, &[format!("place {} {} {} {} 0", s[0], s[1], s[2], s[3])])?;
        std::thread::sleep(Duration::from_secs_f32(c.settle));
    }
    let out = run.join("telemetry.csv");
    let done = run.join("telemetry.csv.done");
    let _ = std::fs::remove_file(&done);
    let _ = std::fs::remove_file(&out);
    let cmd = match c.script {
        Some(script) => {
            let inputs = run.join("inputs.csv");
            if script.canonicalize()? != inputs {
                std::fs::copy(script, &inputs)?;
            }
            format!("script {} {} {}", inputs.display(), watch.display(), out.display())
        }
        None => format!("log {} {} {}", watch.display(), out.display(), c.seconds.unwrap_or(30.0)),
    };
    match (spot_pose, c.kmh) {
        // Placement at speed: arm the script on the placed car, then place it.
        (Some(s), Some(kmh)) => send_ok(
            dir,
            &[
                format!("arm {} {} {}", s[0], s[2], kmh / 1.609_344),
                cmd,
                format!("place {} {} {} {} {}", s[0], s[1], s[2], s[3], kmh / 1.609_344),
            ],
        )?,
        _ => send_ok(dir, &[cmd])?,
    };
    std::fs::write(
        run.join("meta.txt"),
        format!(
            "watch {}\nprobe tick log (one row per game tick)\nspot {}\nstart_kmh {}\nsettle {}\nconditions {:?}\nunix_time {}\n",
            c.watch.display(),
            c.spot.unwrap_or("-"),
            c.kmh.unwrap_or(0.0),
            c.settle,
            c.conditions,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
        ),
    )?;
    let t0 = Instant::now();
    while !done.exists() {
        if t0.elapsed() > Duration::from_secs(600) {
            bail!("the run did not finish within 10 minutes");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("saved {} ({})", run.display(), std::fs::read_to_string(&done)?.trim());
    Ok(())
}
