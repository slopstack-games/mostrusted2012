//! `mwtool compare`: a recording of the original (`mwtool capture`) against Most Rusted's replay of the same
//! inputs (`mostrusted --replay`), side by side.
//!
//! The original's yaw rate, lateral acceleration, body slip and path are derived from its body state (velocity,
//! angular velocity, axes, position) with the conventions Most Rusted logs: yaw + = left, body slip + = nose
//! left of the velocity, positions in each run's start frame (x = left, z = forward).

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;

const MPH: f32 = 0.44704;

/// One sample of either run, in common units.
#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    t: f32,
    mph: f32,
    yaw: f32,
    lat_g: f32,
    slip: f32,
    rpm: f32,
    gear: f32,
    drifting: f32,
    pos: [f32; 2],
}

/// One CSV row by column name.
type Row = HashMap<String, f32>;

fn read(path: &Path) -> Result<Vec<Row>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines();
    let header: Vec<String> = lines.next().unwrap_or("").split(',').map(str::to_string).collect();
    let rows = lines
        .map(|l| {
            header
                .iter()
                .zip(l.split(','))
                .filter_map(|(h, v)| v.parse::<f32>().ok().map(|v| (h.clone(), v)))
                .collect::<HashMap<_, _>>()
        })
        .filter(|r| r.contains_key("t"))
        .collect();
    Ok(rows)
}

fn v3(r: &Row, p: &str) -> Option<[f32; 3]> {
    Some([*r.get(&format!("{p}x"))?, *r.get(&format!("{p}y"))?, *r.get(&format!("{p}z"))?])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The original's recording.
fn original(rows: &[Row]) -> Result<Vec<Sample>> {
    let t0 = rows.first().and_then(|r| r.get("t")).copied().unwrap_or(0.0);
    let first = rows.iter().find(|r| v3(r, "body_fwd").is_some()).context("no body axes in the recording")?;
    let (f0, p0) = (v3(first, "body_fwd").unwrap(), v3(first, "body_p").unwrap_or([0.0; 3]));
    let left0 = v3(first, "body_right").unwrap_or([f0[2], 0.0, -f0[0]]);
    let mut out = Vec::new();
    for r in rows {
        let (Some(v), Some(w), Some(up), Some(fwd), Some(side)) =
            (v3(r, "body_v"), v3(r, "body_w"), v3(r, "body_up"), v3(r, "body_fwd"), v3(r, "body_right"))
        else {
            continue;
        };
        let yaw = dot(w, up);
        let speed = dot(v, v).sqrt();
        let p = v3(r, "body_p").unwrap_or([0.0; 3]);
        let d = [p[0] - p0[0], p[1] - p0[1], p[2] - p0[2]];
        out.push(Sample {
            t: r["t"] - t0,
            mph: r.get("speed_mph").copied().unwrap_or(speed / MPH),
            yaw,
            lat_g: yaw * dot(v, fwd) / 9.81,
            // Body +X ("right" in the watch file) is the car's left in game coordinates.
            slip: -dot(v, side).atan2(dot(v, fwd).abs().max(0.5)).to_degrees(),
            rpm: r.get("rpm").copied().unwrap_or(0.0),
            gear: r.get("gear").copied().unwrap_or(0.0),
            drifting: r.get("drifting").map_or(0.0, |&d| if (0.5..2.0).contains(&d) { 1.0 } else { 0.0 }),
            pos: [dot(d, left0), dot(d, f0)],
        });
    }
    Ok(out)
}

/// Most Rusted's replay log.
fn ours(rows: &[Row]) -> Vec<Sample> {
    let p = |r: &Row| [r.get("px").copied().unwrap_or(0.0), r.get("pz").copied().unwrap_or(0.0)];
    let p0 = rows.first().map(p).unwrap_or([0.0; 2]);
    // Start heading: the direction of the first metre travelled (the replay starts driving straight).
    let p1 = rows.iter().map(p).find(|q| (q[0] - p0[0]).hypot(q[1] - p0[1]) > 1.0).unwrap_or([p0[0], p0[1] + 1.0]);
    let (dx, dz) = (p1[0] - p0[0], p1[1] - p0[1]);
    let n = (dx * dx + dz * dz).sqrt().max(1e-3);
    let (fx, fz) = (dx / n, dz / n);
    // Ours is Y up, Z forward, X left: left = (fz, -fx) rotated.
    let (lx, lz) = (fz, -fx);
    rows.iter()
        .map(|r| {
            let g = |k: &str| r.get(k).copied().unwrap_or(0.0);
            let d = [g("px") - p0[0], g("pz") - p0[1]];
            Sample {
                t: g("t"),
                mph: g("speed_mph"),
                yaw: g("yaw_rate"),
                lat_g: g("lat_accel") / 9.81,
                slip: g("body_slip_deg"),
                rpm: g("rpm"),
                gear: g("gear"),
                drifting: g("drifting"),
                pos: [d[0] * lx + d[1] * lz, d[0] * fx + d[1] * fz],
            }
        })
        .collect()
}

fn at(s: &[Sample], t: f32) -> Option<Sample> {
    let i = s.partition_point(|x| x.t < t);
    s.get(i).or(s.last()).copied().filter(|x| (x.t - t).abs() < 0.2)
}

pub fn compare(original_csv: &Path, ours_csv: &Path, every: f32) -> Result<()> {
    let a = read(original_csv)?;
    let b = read(ours_csv)?;
    let a = original(&a)?;
    let b = ours(&b);
    if a.is_empty() || b.is_empty() {
        bail!("one of the runs is empty");
    }
    let end = a.last().unwrap().t.min(b.last().unwrap().t);
    println!("original = {}, ours = {}", original_csv.display(), ours_csv.display());
    println!(
        "{:>6} | {:>13} | {:>13} | {:>11} | {:>13} | {:>11} | {:>5} | {:>5} | path x/z (m) orig / ours",
        "t", "mph o/u", "yaw °/s o/u", "lat g o/u", "slip ° o/u", "rpm o/u", "gear", "drift"
    );
    let mut t = 0.0;
    let mut err = [0.0f64; 4];
    let mut n = 0;
    while t <= end {
        if let (Some(x), Some(y)) = (at(&a, t), at(&b, t)) {
            println!(
                "{t:6.1} | {:6.1} {:6.1} | {:6.1} {:6.1} | {:5.2} {:5.2} | {:6.1} {:6.1} | {:5.0} {:5.0} | {:2.0} {:2.0} | {:2.0} {:2.0} | {:6.1} {:6.1} / {:6.1} {:6.1}",
                x.mph,
                y.mph,
                x.yaw.to_degrees(),
                y.yaw.to_degrees(),
                x.lat_g,
                y.lat_g,
                x.slip,
                y.slip,
                x.rpm,
                y.rpm,
                x.gear,
                y.gear,
                x.drifting,
                y.drifting,
                x.pos[0],
                x.pos[1],
                y.pos[0],
                y.pos[1]
            );
        }
        t += every;
    }
    // RMS differences over every sample of ours.
    for y in &b {
        if let Some(x) = at(&a, y.t) {
            let d = [x.mph - y.mph, (x.yaw - y.yaw).to_degrees(), x.slip - y.slip, x.lat_g - y.lat_g];
            for (e, d) in err.iter_mut().zip(d) {
                *e += (d * d) as f64;
            }
            n += 1;
        }
    }
    let rms = |i: usize| (err[i] / n.max(1) as f64).sqrt();
    println!(
        "RMS difference over {n} samples: speed {:.1} mph, yaw rate {:.1} °/s, body slip {:.1}°, lateral {:.2} g",
        rms(0),
        rms(1),
        rms(2),
        rms(3)
    );
    let (xa, xb) = (a.iter().rfind(|s| s.t <= end).unwrap(), b.last().unwrap());
    println!(
        "end of path: original ({:.1}, {:.1}) m, ours ({:.1}, {:.1}) m (x left, z forward of the start)",
        xa.pos[0], xa.pos[1], xb.pos[0], xb.pos[1]
    );
    Ok(())
}
