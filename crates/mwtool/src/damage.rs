//! `mwtool damage`: inspect a car's deformation data, check the skinning, and preview damage poses.

use std::path::Path;

use anyhow::{Context, Result};
use mw_assets::AssetStore;
use mw_assets::damage::{VehicleDamage, Zone};

use crate::preview::{Tri, render_views};

pub struct Options<'a> {
    pub car: &'a str,
    /// `zone=level` pairs (zone name or clip index), or `all=level`.
    pub levels: &'a [String],
    pub preview: Option<&'a Path>,
    /// Print each clip's last-frame keys.
    pub keys: bool,
}

fn bundle_for(store: &AssetStore, car: &str) -> Result<String> {
    if car.to_ascii_uppercase().ends_with(".BNDL") {
        return Ok(car.to_string());
    }
    let v = store.find_vehicle(car)?.with_context(|| format!("no car matches {car}"))?;
    println!("{} ({}, id {})", v.name, v.year, v.id);
    v.hi_bundle.context("car has no bundle")
}

pub fn run(store: &AssetStore, o: &Options) -> Result<()> {
    if o.car == "all" {
        return survey(store);
    }
    let path = bundle_for(store, o.car)?;
    let b = store.bundle(&path).with_context(|| format!("bundle not found: {path}"))?;
    let dmg = store.vehicle_damage(&b)?.context("no damage data (skeleton or VehicleDamageBehaviour)")?;
    println!("skeleton: {} bones", dmg.skeleton.bones.len());
    for (i, bone) in dmg.skeleton.bones.iter().enumerate() {
        let p = bone.position;
        let q = bone.rotation;
        let rot = if q[3].abs() < 0.9999 { format!("  bind rotation {q:.3?}") } else { String::new() };
        println!(
            "  bone {i:2}  parent {:>2}  pos ({:6.3} {:6.3} {:6.3})  id {:08x}{rot}",
            bone.parent.map_or("-".into(), |p| p.to_string()),
            p[0],
            p[1],
            p[2],
            dmg.skeleton.ids[i]
        );
    }
    print_animations(&dmg, o.keys);
    for (i, c) in dmg.clips.iter().enumerate() {
        let a = &c.animation;
        let last = a.frames.len().saturating_sub(1);
        // Frame at which each bone breaks off (scale reaches 0).
        let detach: Vec<String> = a
            .bones
            .iter()
            .enumerate()
            .filter_map(|(k, &bone)| {
                let f = a.frames.iter().position(|keys| keys[k].scale <= 0.0)?;
                Some(format!("{bone}@{f}"))
            })
            .collect();
        println!(
            "clip {i}: {:<11} (bones toward {:<11})  id {:#x}  {} frames  bones {:?}  detach {:?}",
            c.zone.name,
            Zone::sector(c.bones_direction),
            a.id,
            a.frames.len(),
            a.bones,
            detach
        );
        // Largest displacement of a surviving bone at the last frame.
        let pose = dmg.pose(&(0..dmg.clips.len()).map(|j| if j == i { 1.0 } else { 0.0 }).collect::<Vec<_>>());
        let worst = a
            .bones
            .iter()
            .filter(|&&bone| pose[bone].scale > 0.0)
            .map(|&bone| {
                let (p, q) = (pose[bone].translation, dmg.bind[bone].translation);
                (bone, (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>().sqrt())
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((bone, d)) = worst {
            println!("        at frame {last}: bone {bone} moved {:.3} m", d);
        }
        if o.keys {
            // Bones whose first-frame key is not the identity (the clip moves them before any damage).
            let moved0: Vec<usize> = a
                .bones
                .iter()
                .enumerate()
                .filter(|(k, _)| {
                    let key = &a.frames[0][*k];
                    key.translation.iter().any(|t| t.abs() > 1e-4) || key.rotation[3] < 0.9999 || key.scale != 1.0
                })
                .map(|(_, &b)| b)
                .collect();
            println!("          not identity at frame 0: bones {moved0:?}");
            let frames = [0, 1, last];
            for (k, &bone) in a.bones.iter().enumerate() {
                for f in frames {
                    if f != last && !moved0.contains(&bone) {
                        continue;
                    }
                    let key = &a.frames[f][k];
                    println!("          f{f:<2} bone {bone:2}: {}", key_text(key));
                }
            }
        }
    }
    for (i, p) in dmg.bodyparts.iter().enumerate() {
        println!(
            "bodypart {i}: hinge ({:5.2} {:5.2} {:5.2})  {:.0} kg  area {}  limits {:?}",
            p.position[0], p.position[1], p.position[2], p.mass, p.area, p.limits
        );
    }
    println!("impact graph: {:?}", store.impact_graph());

    // Skinning check: every body vertex should use valid bones and sit near the bones it follows.
    let meshes = body_meshes(store, &b)?;
    let n_bones = dmg.skeleton.bones.len();
    let (mut verts, mut bad) = (0usize, 0usize);
    let mut near = vec![(0usize, [0.0f32; 3]); n_bones];
    for m in &meshes {
        let (Some(j), Some(w)) = (&m.joints, &m.weights) else { continue };
        for ((p, j), w) in m.positions.iter().zip(j).zip(w) {
            verts += 1;
            if j.iter().zip(w).any(|(&j, &w)| w > 0.0 && j as usize >= n_bones) {
                bad += 1;
                continue;
            }
            let k = (0..4).max_by(|&a, &b| w[a].total_cmp(&w[b])).unwrap_or(0);
            let e = &mut near[j[k] as usize];
            e.0 += 1;
            for (acc, v) in e.1.iter_mut().zip(p) {
                *acc += v;
            }
        }
    }
    println!("skinned body vertices: {verts}, {bad} with out-of-range bones");
    for (bone, (n, sum)) in near.iter().enumerate().filter(|(_, (n, _))| *n > 0) {
        let c = sum.map(|s| s / *n as f32);
        let p = dmg.bind[bone].translation;
        let d = (0..3).map(|k| (c[k] - p[k]).powi(2)).sum::<f32>().sqrt();
        println!(
            "  bone {bone:2}: {n:6} vertices, centroid ({:6.2} {:6.2} {:6.2}), {d:.2} m from bone",
            c[0], c[1], c[2]
        );
    }

    let levels = parse_levels(&dmg, o.levels)?;
    if let Some(out) = o.preview {
        let pose = dmg.pose(&levels);
        let skin = dmg.skin(&pose);
        let mut tris = Vec::new();
        for m in &meshes {
            let deformed: Vec<[f32; 3]> = match (&m.joints, &m.weights) {
                (Some(j), Some(w)) => m
                    .positions
                    .iter()
                    .zip(j)
                    .zip(w)
                    .map(|((p, j), w)| {
                        let mut out = [0.0; 3];
                        for k in 0..4 {
                            if w[k] <= 0.0 {
                                continue;
                            }
                            let s = skin.get(j[k] as usize).copied().unwrap_or(mw_assets::damage::Similarity::IDENTITY);
                            let q = s.apply([p[0], p[1], p[2]]);
                            for c in 0..3 {
                                out[c] += q[c] * w[k];
                            }
                        }
                        out
                    })
                    .collect(),
                _ => m.positions.iter().map(|p| [p[0], p[1], p[2]]).collect(),
            };
            for t in m.indices.as_chunks::<3>().0 {
                let p = [deformed[t[0] as usize], deformed[t[1] as usize], deformed[t[2] as usize]];
                // Collapsed triangles are detached parts.
                if p[0] == p[1] && p[1] == p[2] {
                    continue;
                }
                tris.push(Tri { p, color: [200, 200, 210] });
            }
        }
        render_views(&tris, 512).save(out)?;
        println!("levels {levels:?} -> {}", out.display());
    }
    Ok(())
}

fn parse_levels(dmg: &VehicleDamage, specs: &[String]) -> Result<Vec<f32>> {
    let mut levels = vec![0.0; dmg.clips.len()];
    for s in specs {
        let (zone, level) = s.split_once('=').with_context(|| format!("expected zone=level, got {s}"))?;
        let level: f32 = level.parse().with_context(|| format!("bad level in {s}"))?;
        if zone == "all" {
            levels.iter_mut().for_each(|l| *l = level);
            continue;
        }
        let i = zone
            .parse::<usize>()
            .ok()
            .or_else(|| dmg.clips.iter().position(|c| c.zone.name == zone))
            .filter(|&i| i < levels.len())
            .with_context(|| format!("no clip {zone}"))?;
        levels[i] = level;
    }
    Ok(levels)
}

/// The decoded LOD0 meshes of a car's body part models.
fn body_meshes(store: &AssetStore, b: &mw_bundle::Bundle) -> Result<Vec<mw_formats::DecodedMesh>> {
    let e = b.entries_of_type(mw_bundle::ResourceType::VehicleGraphicsSpec).next().context("no VehicleGraphicsSpec")?;
    let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e)?)?;
    let mut out = Vec::new();
    for &m in spec.parts.iter().flatten() {
        let (meshes, _) = store.model_meshes(m, 0)?;
        out.extend(meshes.into_iter().map(|sm| sm.mesh));
    }
    Ok(out)
}

/// Every car: does its damage data load, and what zones do its clips cover?
fn survey(store: &AssetStore) -> Result<()> {
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE)?;
    let mut seen = std::collections::HashSet::new();
    let (mut ok, mut missing, mut failed) = (0, 0, 0);
    for v in store.vehicles(&lang)? {
        let Some(path) = v.hi_bundle else { continue };
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(b) = store.bundle(&path) else { continue };
        match store.vehicle_damage(&b) {
            Ok(Some(d)) => {
                ok += 1;
                // Clips whose moved bones don't point toward their zone.
                let off: Vec<String> = d
                    .clips
                    .iter()
                    .filter(|c| Zone::sector(c.bones_direction) != c.zone.name)
                    .map(|c| format!("{}~{}", c.zone.name, Zone::sector(c.bones_direction)))
                    .collect();
                let frames: std::collections::BTreeSet<usize> =
                    d.clips.iter().map(|c| c.animation.frames.len()).collect();
                // Bones whose first-frame key in a damage clip is not the identity (`pose_with` counts damage
                // keys from frame 0), with the largest such offset.
                let mut rest = std::collections::BTreeSet::new();
                let mut rest_max = 0.0f32;
                for a in d.clips.iter().map(|c| &c.animation) {
                    let Some(f0) = a.frames.first() else { continue };
                    for (&bone, key) in a.bones.iter().zip(f0) {
                        let t = key.translation.iter().map(|t| t * t).sum::<f32>().sqrt();
                        if t > 1e-3 || key.rotation[3] < 0.9999 || key.scale != 1.0 {
                            rest.insert(bone);
                            rest_max = rest_max.max(t);
                        }
                    }
                }
                // Blend data the game's pose blend reads: reference frames, bone masks (distinct values over
                // all clips, damage and parts) and bones with a non-identity bind rotation.
                let parts = d.animations.iter().flat_map(|a| a.clips.iter().map(|c| &c.animation));
                let all: Vec<&mw_formats::Animation> = d.clips.iter().map(|c| &c.animation).chain(parts).collect();
                let refs: std::collections::BTreeSet<usize> = all.iter().map(|a| a.reference_frame).collect();
                let mut masks: Vec<String> = all
                    .iter()
                    .map(|a| {
                        if a.bone_mask.is_empty() {
                            return "none".to_string();
                        }
                        let mut v: Vec<f32> = a.bone_mask.clone();
                        v.sort_by(f32::total_cmp);
                        v.dedup();
                        format!("{v:?}")
                    })
                    .collect();
                masks.sort();
                masks.dedup();
                let bind_rot = d.skeleton.bones.iter().filter(|b| b.rotation[3].abs() < 0.9999).count();
                let rest = format!(
                    "{rest_text}  ref {refs:?}  masks {masks:?}  rotated binds {bind_rot}",
                    rest_text = if rest.is_empty() {
                        String::new()
                    } else {
                        format!("  frame-0 offsets on bones {rest:?} (up to {rest_max:.3} m)")
                    }
                );
                println!(
                    "{:<40} {:2} bones  {} clips  frames {:?}  {:2} bodyparts  off-zone {:?}{rest}",
                    v.name,
                    d.skeleton.bones.len(),
                    d.clips.len(),
                    frames,
                    d.bodyparts.len(),
                    off
                );
            }
            Ok(None) => {
                missing += 1;
                println!("{:<40} no damage data", v.name);
            }
            Err(e) => {
                failed += 1;
                println!("{:<40} ERROR {e}", v.name);
            }
        }
        store.evict_all();
    }
    println!("{ok} cars with damage data, {missing} without, {failed} failed");
    Ok(())
}

/// Animated body parts (active aero) of the car: clips, the bones they move, the state machine's
/// variables and the speed rule when it is the simple form (`mw_assets::car_anim`).
fn print_animations(dmg: &mw_assets::damage::VehicleDamage, keys: bool) {
    let Some(a) = &dmg.animations else {
        println!("animated parts: none");
        return;
    };
    println!(
        "animated parts: {:?}  hsm {}  variables {:?}  rule {:?}",
        a.label,
        a.hsm.map_or("-".into(), |id| id.to_string()),
        a.variables,
        a.speed_rule
    );
    for (i, c) in a.clips.iter().enumerate() {
        println!(
            "  anim {i}: {}  {} frames at {:.3}/s ({:.2} s)  bones {:?}",
            c.id,
            c.animation.frames.len(),
            c.animation.frame_rate,
            c.duration(),
            c.animation.bones
        );
        if keys {
            for (f, frame) in c.animation.frames.iter().enumerate() {
                for (&bone, key) in c.animation.bones.iter().zip(frame) {
                    println!("          f{f:<2} bone {bone:2}: {}", key_text(key));
                }
            }
        }
    }
    let names: Vec<&str> =
        a.hsm_strings.iter().filter(|s| s.len() > 3 || s.parse::<f32>().is_ok()).map(String::as_str).collect();
    println!("  hsm strings: {}", names.join(" | "));
}

/// A clip key as offset, rotation angle about its axis, and scale.
fn key_text(key: &mw_formats::BoneKey) -> String {
    let [x, y, z, w] = key.rotation;
    let angle = 2.0 * w.clamp(-1.0, 1.0).acos().to_degrees();
    let s = (1.0 - w * w).max(1e-12).sqrt();
    let t = key.translation;
    format!(
        "offset ({:6.3} {:6.3} {:6.3})  rot {angle:6.1}° about ({:5.2} {:5.2} {:5.2})  scale {:.2}",
        t[0],
        t[1],
        t[2],
        x / s,
        y / s,
        z / s,
        key.scale
    )
}
