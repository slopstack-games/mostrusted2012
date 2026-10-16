//! `mwtool car-lights <car|all>`: the light sources a car carries (`VehicleVfxBehaviour.Lights`,
//! `mw_assets::car_lights`): per entry its group, gate, definition type, colour, intensity, threshold, cone or
//! projector, and its locators (car space). `all` surveys every car bundle: the distinct (group, type,
//! definition) combinations and how many cars use each.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_assets::car_lights::CarLightSource;
use mw_assets::lights::LightShape;

fn kind(l: &CarLightSource) -> String {
    if l.projector.is_some() {
        return "projected".into();
    }
    match l.def.shape {
        LightShape::Point => "point".into(),
        LightShape::Cone { angle } => format!("cone {angle:.1}"),
        LightShape::Spot { angle, inner } => format!("spot {angle:.1}/{inner:.1}"),
        LightShape::Line { .. } => "line".into(),
        LightShape::Other => "custom".into(),
    }
}

fn car_sources(store: &AssetStore, bundle: &str) -> Result<Vec<CarLightSource>> {
    let b = store.bundle(bundle).context("bundle not found")?;
    let e = b.entries_of_type(mw_bundle::ResourceType::VehicleGraphicsSpec).next().context("no VehicleGraphicsSpec")?;
    let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
    Ok(store.car_light_sources(&b, &spec))
}

pub fn run(store: &AssetStore, car: &str) -> Result<()> {
    if car == "all" {
        return survey(store);
    }
    let v = store.find_vehicle(car)?.context("no such car")?;
    let bundle = v.hi_bundle.context("no bundle")?;
    let lights = car_sources(store, &bundle)?;
    println!("{} ({bundle}): {} light sources", v.name, lights.len());
    for l in &lights {
        println!(
            "  group {:2} gate {} {:16} def {:016x} colour {:.3?} x {:.3?} intensity {} x {} threshold {}{}",
            l.group,
            l.gate,
            kind(l),
            l.def.id.0,
            l.def.colour,
            &l.colour[..3],
            l.def.intensity,
            l.scale,
            l.def.attenuation_threshold,
            if l.animation.is_some() { " animated" } else { "" }
        );
        if let Some(p) = &l.projector {
            println!(
                "    projector fov scale {:?} attenuation {:?} window {:?} material {:?} texture {:?}",
                p.fov_scale,
                p.attenuation,
                p.window,
                p.material.map(|m| format!("{:016x}", m.0)),
                p.texture.map(|t| format!("{:016x}", t.0))
            );
        }
        for loc in &l.locators {
            let [x, y, z, w] = loc.rotation;
            // Rows of the row-vector rotation matrix (the game's quaternion expansion in `FUN_005818f0`).
            let rx = [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)];
            let ry = [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)];
            let rz = [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)];
            println!(
                "    locator at {:.3?} |q| {:.4} x {:.3?} y {:.3?} z {:.3?}",
                loc.position,
                (x * x + y * y + z * z + w * w).sqrt(),
                rx,
                ry,
                rz
            );
        }
    }
    Ok(())
}

fn survey(store: &AssetStore) -> Result<()> {
    let mut seen: BTreeMap<(u8, u8, String, u64), (usize, String)> = BTreeMap::new();
    let mut textures: BTreeMap<String, usize> = BTreeMap::new();
    let mut cars = 0;
    let bundles: Vec<String> = store
        .bundle_paths()
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| {
            let u = p.to_ascii_uppercase();
            u.starts_with("VEHICLES/VEH_") && (u.ends_with("_HI.BNDL") || u.ends_with("_TR.BNDL"))
        })
        .collect();
    for bundle in bundles {
        let Ok(lights) = car_sources(store, &bundle) else { continue };
        cars += 1;
        for l in &lights {
            let e = seen.entry((l.group, l.gate, kind(l), l.def.id.0)).or_insert((0, bundle.clone()));
            e.0 += 1;
            if let Some(p) = &l.projector {
                *textures.entry(format!("{:?} / {:?}", p.material, p.texture)).or_default() += 1;
                for loc in &l.locators {
                    let [x, y, z, w] = loc.rotation;
                    if (w.abs() - 1.0).abs() > 1e-4 {
                        println!("  {bundle}: group {} projector locator rotation {:?}", l.group, [x, y, z, w]);
                    }
                }
            }
        }
    }
    println!("{cars} cars");
    for ((group, gate, kind, def), (n, example)) in &seen {
        println!("  group {group:2} gate {gate} {kind:16} def {def:016x}: {n} cars (e.g. {example})");
    }
    for (t, n) in textures {
        println!("  projector material / texture {t}: {n}");
    }
    Ok(())
}
