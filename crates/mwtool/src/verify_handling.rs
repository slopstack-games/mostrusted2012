//! `mwtool verify-handling`: check a ported handling function against the original, offline.
//!
//! The idea (docs/probe.md, "Tier 2", offline route): record the *inputs* and *output* of a game function from
//! the running original into a CSV (per-wheel slip ratio, slip angle, camber, and the force coefficient the game
//! produced, read from memory with `mwtool telemetry` or `capture` and the right watch offsets). Then run our
//! Rust port (`mw_handling`) on the same recorded inputs and report where it disagrees. A correct port matches
//! the original to floating-point noise, with no hooks and nothing written into the game.
//!
//! Currently covers the tyre grip curves (`mw_handling::tyre::coefficients`): the per-row columns expected are,
//! for each wheel `w` (default the four `w0 w1 w2 w3` that `handling.watch` records; `w0`/`w1` are the front
//! axle, `w2`/`w3` the rear, per docs/decomp.md "Suspension"):
//!
//! ```text
//! <w>_kappa     slip ratio κ        (handling.watch, wheel +0x110; docs/decomp.md "Tyres")
//! <w>_alpha     slip angle, degrees (wheel +0x114)
//! <w>_camber    camber, degrees     (optional; 0 if absent)
//! <w>_mu_lat    the game's lateral coefficient   (the value to match, if captured)
//! <w>_mu_long   the game's longitudinal coefficient (the value to match, if captured)
//! ```
//!
//! The game stores the grip *caps* (force, Newtons), not the bare coefficients, so `<w>_mu_lat` here means the
//! captured coefficient once the user has divided a cap by its load and scales (see "Verification advice" in
//! the research log). The axle for each wheel decides which `TyreAxle` the port uses. Give the car with
//! `--vehicle` (name or `.BNDL`); `mwtool vehicles` lists names.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use mw_assets::AssetStore;
use mw_assets::handling::VehicleHandling;

type Row = HashMap<String, f32>;

fn read_csv(path: &Path) -> Result<(Vec<String>, Vec<Row>)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines();
    let header: Vec<String> = lines.next().unwrap_or("").split(',').map(|s| s.trim().to_string()).collect();
    let rows = lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            header
                .iter()
                .zip(l.split(','))
                .filter_map(|(h, v)| v.trim().parse::<f32>().ok().map(|v| (h.clone(), v)))
                .collect()
        })
        .collect();
    Ok((header, rows))
}

/// Error stats for one coefficient over all rows of all wheels.
#[derive(Default)]
struct Stats {
    n: usize,
    sum_abs: f64,
    max_abs: f64,
    sum_sq: f64,
    worst: Option<(f32, f32)>, // (ours, game) at the largest error
}

impl Stats {
    fn add(&mut self, ours: f32, game: f32) {
        let e = (ours - game) as f64;
        self.n += 1;
        self.sum_abs += e.abs();
        self.sum_sq += e * e;
        if e.abs() > self.max_abs {
            self.max_abs = e.abs();
            self.worst = Some((ours, game));
        }
    }

    fn report(&self, label: &str) {
        if self.n == 0 {
            println!("{label}: no rows (columns missing?)");
            return;
        }
        let mae = self.sum_abs / self.n as f64;
        let rms = (self.sum_sq / self.n as f64).sqrt();
        let worst = self.worst.map(|(o, g)| format!(" (ours {o:.4} vs game {g:.4})")).unwrap_or_default();
        println!("{label}: {} rows, MAE {mae:.5}, RMS {rms:.5}, max |err| {:.5}{worst}", self.n, self.max_abs);
    }
}

/// Front axle: `w0`/`w1` (handling.watch's wheel order) or a name starting with `f` (`fl`/`fr`); else rear.
fn is_front(wheel: &str) -> bool {
    matches!(wheel, "w0" | "w1") || wheel.starts_with('f')
}

pub fn verify_handling(store: &AssetStore, csv: &Path, vehicle: &str, wheels_arg: Option<&str>) -> Result<()> {
    let (header, rows) = read_csv(csv)?;
    if rows.is_empty() {
        bail!("{} has no data rows", csv.display());
    }
    let bundle = if vehicle.to_ascii_uppercase().ends_with(".BNDL") {
        vehicle.to_string()
    } else {
        let v = store.find_vehicle(vehicle)?.with_context(|| format!("no car matches {vehicle}"))?;
        v.hi_bundle.context("car has no bundle")?
    };
    let h = VehicleHandling::for_vehicle(store, &bundle)?;
    let tyres = h.tyres.context("this car has no tyre data")?;

    let wheels: Vec<String> = match wheels_arg {
        Some(list) => list.split(',').map(|s| s.trim().to_string()).collect(),
        None => ["w0", "w1", "w2", "w3"].iter().map(|s| s.to_string()).collect(),
    };
    // Keep only wheels whose columns are actually present, so a two-column capture still works.
    let present: Vec<String> = wheels
        .into_iter()
        .filter(|w| header.iter().any(|c| c == &format!("{w}_kappa") || c == &format!("{w}_alpha")))
        .collect();
    if present.is_empty() {
        bail!(
            "no per-wheel columns found. Expected e.g. `fl_kappa, fl_alpha, fl_mu_lat, fl_mu_long`. \
             Header was: {}",
            header.join(", ")
        );
    }
    println!("verifying tyre coefficients for {vehicle} ({} rows, wheels: {})", rows.len(), present.join(" "));

    let (mut lat, mut long) = (Stats::default(), Stats::default());
    for row in &rows {
        for w in &present {
            let get = |suffix: &str| row.get(&format!("{w}_{suffix}")).copied();
            let (Some(kappa), Some(alpha)) = (get("kappa"), get("alpha")) else { continue };
            let camber = get("camber").unwrap_or(0.0);
            let axle = if is_front(w) { &tyres[0] } else { &tyres[1] };
            // The game scales the lateral curve's exponent by static load / grip load (`load_in` / `N`).
            let load_ratio = match (get("load_in"), get("N")) {
                (Some(s), Some(n)) if n > 1.0 => s / n,
                _ => 1.0,
            };
            let (mu_long, mu_lat) = mw_handling::tyre::coefficients(axle, kappa, alpha, camber, load_ratio);
            if let Some(g) = get("mu_lat") {
                lat.add(mu_lat, g);
            }
            if let Some(g) = get("mu_long") {
                long.add(mu_long, g);
            }
        }
    }
    lat.report("lateral coefficient");
    long.report("longitudinal coefficient");
    if lat.n == 0 && long.n == 0 {
        bail!("no `*_mu_lat` or `*_mu_long` columns to compare against; capture the game's coefficient too");
    }
    let ok = lat.max_abs < 1e-3 && long.max_abs < 1e-3;
    println!(
        "{}",
        if ok { "PORT MATCHES (max error < 1e-3)" } else { "PORT DIVERGES: investigate the largest error above" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_track_error() {
        let mut s = Stats::default();
        s.add(1.0, 1.0);
        s.add(1.0, 1.2);
        assert_eq!(s.n, 2);
        assert!((s.max_abs - 0.2).abs() < 1e-6);
        assert_eq!(s.worst, Some((1.0, 1.2)));
    }

    #[test]
    fn front_rear_split() {
        assert!(is_front("w0"));
        assert!(is_front("w1"));
        assert!(!is_front("w2"));
        assert!(!is_front("w3"));
        assert!(is_front("fl"));
        assert!(!is_front("rr"));
    }
}
