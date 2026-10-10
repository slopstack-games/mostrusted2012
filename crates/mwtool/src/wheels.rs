//! `mwtool wheels`: where each car's wheels are, by the spec and by the game's rule (docs/formats.md
//! "Wheel placement", `mw_assets::wheel_layout`).
//!
//! Per wheel: the `VehicleGraphicsSpec` position, the position the game builds the wheel at (player cars: the
//! vehicle build's axle rule; `_TR` bundles: the kinematic traffic pose, model space), the difference, the
//! wheel models' radius and the physics wheel body's, and the tyre's bottom at each position (the ground at
//! rest is at y = −T.y in model space, `T` = `VehicleInfo.509931cd` translation, also printed).
//! `--png <dir>` writes a side view per car (body grey, tyre red, other wheel parts yellow; the spec's wheel
//! position as a blue outline, the ground at rest green).

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_bundle::ResourceType;

pub struct WheelCheck {
    pub name: String,
    pub spec: [f32; 3],
    /// Where the game puts the wheel (`None` without the axle data).
    pub game: Option<[f32; 3]>,
    /// Radius of the wheel's models (largest distance from the axle).
    pub model_radius: f32,
    /// Cylinder radius of the physics wheel body.
    pub body_radius: Option<f32>,
}

pub struct CarCheck {
    pub wheels: Vec<WheelCheck>,
    /// `VehicleInfo.509931cd` translation y: the model origin's height above the road at rest.
    pub ride_height: Option<f32>,
}

/// The wheel part models of a spec wheel, transformed into car space at `position` (right wheels turned half
/// a turn about Y, as the game does: `FUN_00593ac0`, `FUN_00902930`).
fn wheel_points(
    store: &AssetStore,
    w: &mw_formats::Wheel,
    position: [f32; 3],
) -> Vec<(usize, Vec<[f32; 3]>, Vec<u32>)> {
    let turn = position[0] < 0.0;
    let mut out = Vec::new();
    for (i, p) in w.parts.iter().enumerate() {
        let Some(m) = p.model else { continue };
        let Ok((meshes, _)) = store.model_meshes(m, 0) else { continue };
        for sm in meshes {
            let pts = sm
                .mesh
                .positions
                .iter()
                .map(|v| {
                    let (x, z) = if turn { (-v[0], -v[2]) } else { (v[0], v[2]) };
                    [x + position[0], v[1] + position[1], z + position[2]]
                })
                .collect();
            out.push((i, pts, sm.mesh.indices.clone()));
        }
    }
    out
}

/// Measure one car (a `_HI` or `_TR` bundle path).
pub fn check(store: &AssetStore, bundle: &str) -> Result<CarCheck> {
    let b = store.bundle(bundle).with_context(|| format!("no bundle {bundle}"))?;
    let e = b.entries_of_type(ResourceType::VehicleGraphicsSpec).next().context("no VehicleGraphicsSpec")?;
    let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
    let traffic = bundle.to_ascii_uppercase().ends_with("_TR.BNDL");
    let layout = store.vehicle_wheel_layout(&b);
    let mut placed = spec.clone();
    if let Some(l) = &layout {
        let positions: Vec<[f32; 3]> =
            if traffic { l.traffic_positions() } else { l.wheels.iter().map(|w| w.position).collect() };
        mw_assets::wheel_layout::place_spec_wheels(&mut placed, &positions);
    }
    let wheels = spec
        .wheels
        .iter()
        .zip(&placed.wheels)
        .map(|(w, p)| {
            let radius = wheel_points(store, w, [0.0; 3])
                .iter()
                .flat_map(|(_, pts, _)| pts.iter().map(|v| (v[1] * v[1] + v[2] * v[2]).sqrt()))
                .fold(0.0f32, f32::max);
            let body_radius = layout.as_ref().and_then(|l| {
                let side = |x: f32| x < 0.0;
                l.wheels
                    .iter()
                    .find(|r| {
                        side(r.position[0]) == side(p.position[0]) && (r.position[2] - p.position[2]).abs() < 1e-4
                    })
                    .map(|r| r.radius)
            });
            WheelCheck {
                name: w.name.clone(),
                spec: w.position,
                game: layout.is_some().then_some(p.position),
                model_radius: radius,
                body_radius,
            }
        })
        .collect();
    Ok(CarCheck { wheels, ride_height: layout.map(|l| l.traffic_ride_height()) })
}

/// Side view of the car with its wheels where the game puts them, viewed from the right (−X).
pub fn side_view(store: &AssetStore, bundle: &str, out: &std::path::Path) -> Result<()> {
    use crate::preview::Tri;
    let b = store.bundle(bundle).with_context(|| format!("no bundle {bundle}"))?;
    let e = b.entries_of_type(ResourceType::VehicleGraphicsSpec).next().context("no VehicleGraphicsSpec")?;
    let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
    let car = check(store, bundle)?;
    let mut tris = Vec::new();
    for &m in spec.parts.iter().flatten() {
        let (meshes, _) = store.model_meshes(m, 0)?;
        for sm in meshes {
            let p = &sm.mesh.positions;
            for t in sm.mesh.indices.as_chunks::<3>().0 {
                tris.push(Tri {
                    p: t.map(|i| [p[i as usize][0], p[i as usize][1], p[i as usize][2]]),
                    color: [150, 150, 160],
                });
            }
        }
    }
    let ring = |tris: &mut Vec<Tri>, c: [f32; 3], r: f32, colour: [u8; 3]| {
        // A thin ring in the wheel's plane, outside the car so it draws on top in the side view.
        let n = 48;
        for k in 0..n {
            let (a0, a1) =
                (k as f32 / n as f32 * std::f32::consts::TAU, (k + 1) as f32 / n as f32 * std::f32::consts::TAU);
            let p = |a: f32, rr: f32| [-1.5, c[1] + rr * a.sin(), c[2] + rr * a.cos()];
            tris.push(Tri { p: [p(a0, r), p(a1, r), p(a1, r - 0.012)], color: colour });
            tris.push(Tri { p: [p(a0, r), p(a1, r - 0.012), p(a0, r - 0.012)], color: colour });
        }
    };
    for (w, c) in spec.wheels.iter().zip(&car.wheels) {
        if w.position[0] > 0.0 {
            continue;
        }
        for (i, pts, idx) in wheel_points(store, w, c.game.unwrap_or(c.spec)) {
            let colour = if i == 0 { [220, 60, 50] } else { [240, 200, 60] };
            for t in idx.as_chunks::<3>().0 {
                tris.push(Tri { p: t.map(|i| pts[i as usize]), color: colour });
            }
        }
        ring(&mut tris, c.spec, c.model_radius, [70, 120, 255]);
    }
    // The ground at rest: y = −T.y.
    let g = -car.ride_height.unwrap_or(0.0);
    tris.push(Tri {
        p: [[-1.5, g - 0.005, -3.0], [-1.5, g + 0.005, -3.0], [-1.5, g + 0.005, 3.0]],
        color: [60, 200, 90],
    });
    tris.push(Tri {
        p: [[-1.5, g - 0.005, -3.0], [-1.5, g + 0.005, 3.0], [-1.5, g - 0.005, 3.0]],
        color: [60, 200, 90],
    });
    let size = 600;
    let img = crate::preview::render_views(&tris, size);
    image::imageops::crop_imm(&img, size, 0, size, size).to_image().save(out)?;
    Ok(())
}

pub fn run(store: &AssetStore, car: &str, png: Option<&std::path::Path>) -> Result<()> {
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE)?;
    let cars: Vec<(String, String)> = if car == "all" {
        // Every car bundle: the catalogue's (named), then the rest (cop / traffic `_TR` variants).
        let mut seen = std::collections::HashSet::new();
        let mut cars: Vec<(String, String)> = store
            .vehicles(&lang)?
            .into_iter()
            .filter_map(|v| Some((v.name, v.hi_bundle?.to_ascii_uppercase())))
            .filter(|(_, b)| seen.insert(b.clone()))
            .collect();
        for p in store.bundle_paths() {
            let path = p.to_string_lossy().replace('\\', "/").to_ascii_uppercase();
            let Some(file) = path.strip_prefix("VEHICLES/VEH_") else { continue };
            if (file.ends_with("_HI.BNDL") || file.ends_with("_TR.BNDL")) && seen.insert(path.clone()) {
                cars.push((file.trim_end_matches(".BNDL").to_string(), path));
            }
        }
        cars
    } else if car.to_ascii_uppercase().ends_with(".BNDL") {
        vec![(car.to_string(), car.to_string())]
    } else {
        let v = store.find_vehicle(car)?.with_context(|| format!("no car matches {car}"))?;
        vec![(v.name, v.hi_bundle.context("car has no bundle")?)]
    };
    println!(
        "{:<30} {:<10} {:>20} {:>20} {:>7} {:>7} {:>6} {:>6} {:>7} {:>7} {:>6}",
        "car", "wheel", "spec x y z", "game x y z", "dy", "dz", "r", "r_body", "bot_sp", "bot_gm", "T.y"
    );
    for (name, bundle) in cars {
        let car = match check(store, &bundle) {
            Ok(c) => c,
            Err(e) => {
                println!("{name:<30} {bundle}: {e:#}");
                continue;
            }
        };
        if let Some(dir) = png {
            std::fs::create_dir_all(dir)?;
            let file: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            side_view(store, &bundle, &dir.join(format!("{file}_{}.png", bundle.replace(['/', '.'], "_"))))?;
        }
        let short: String = name.chars().take(30).collect();
        let v3 = |p: [f32; 3]| format!("{:6.3} {:6.3} {:6.3}", p[0], p[1], p[2]);
        for w in car.wheels {
            let game = w.game.unwrap_or([f32::NAN; 3]);
            println!(
                "{:<30} {:<10} {:>20} {:>20} {:7.3} {:7.3} {:6.3} {:6.3} {:7.3} {:7.3} {:6.3}",
                short,
                w.name,
                v3(w.spec),
                v3(game),
                game[1] - w.spec[1],
                game[2] - w.spec[2],
                w.model_radius,
                w.body_radius.unwrap_or(f32::NAN),
                w.spec[1] - w.model_radius,
                game[1] - w.model_radius,
                car.ride_height.unwrap_or(f32::NAN),
            );
        }
    }
    Ok(())
}
