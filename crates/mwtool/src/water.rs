//! `mwtool water`: every water surface of the world (main and backdrop `InstanceList` entries whose meshes use
//! a `Water_*` or standing-water shader), with its height, extent and listing units, and where it lies above
//! the collision under it.
//!
//! The surface is sampled on a grid over its triangles. At each sample the collision column is read: the
//! highest collision surface below the water (+0.25 m) is its "bed". A bed with a wash surface (the surfaces
//! whose `VehicleEffects` wash the car) is a ford; any other bed more than 0.25 m below is "dry". Collision does
//! not cover river beds or the sea floor everywhere, so judge the drivable rows (flags 0x1 / 0x3: water over
//! a road) and the per-zone fill-in table, not single samples. For backdrop meshes the composed backdrop
//! (`mw_assets::backdrop`) says whether we draw it there; `own` counts drawn dry samples inside a zone that
//! lists the mesh itself.
//!
//! Debug environment: `MW_WATER_TRACE=zone:mesh` (samples of that pair drawn in a zone that does not list
//! it), `MW_WATER_AT=x,z;x,z` or `grid:x0,x1,z0,z1,step` (collision columns), `MW_WATER_XFORM=1` (placement
//! transforms), `MW_WATER_UNITS=1` (collision bounds per unit vs zone bounds), `MW_WATER_ALL_UNITS=1` (every
//! listing unit), `MW_WATER_DUMP=<file>` (every water triangle and zone polygon, for plotting).

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use mw_assets::AssetStore;
use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;

const CELL: f32 = 8.0;

#[derive(Default)]
struct Ground {
    tris: Vec<([[f32; 3]; 3], fmt::CollisionTag)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Ground {
    fn add(&mut self, list: &fmt::PolygonSoupList) {
        let (verts, tris) = list.triangles();
        for (t, tag) in tris.iter().zip(list.triangle_tags()) {
            let p = t.map(|i| verts[i as usize]);
            let idx = self.tris.len() as u32;
            self.tris.push((p, fmt::CollisionTag::from_u32(tag)));
            let c = |v: f32| (v / CELL).floor() as i32;
            let (x0, x1) =
                (p.iter().map(|v| v[0]).fold(f32::MAX, f32::min), p.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
            let (z0, z1) =
                (p.iter().map(|v| v[2]).fold(f32::MAX, f32::min), p.iter().map(|v| v[2]).fold(f32::MIN, f32::max));
            if c(x1) - c(x0) > 64 || c(z1) - c(z0) > 64 {
                continue;
            }
            for cx in c(x0)..=c(x1) {
                for cz in c(z0)..=c(z1) {
                    self.grid.entry((cx, cz)).or_default().push(idx);
                }
            }
        }
    }

    /// Every collision surface in the column through (x, z): (y, tag).
    fn column(&self, x: f32, z: f32) -> Vec<(f32, fmt::CollisionTag)> {
        let key = ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        let Some(list) = self.grid.get(&key) else { return Vec::new() };
        list.iter()
            .filter_map(|&i| {
                let (t, tag) = &self.tris[i as usize];
                height_at(t, x, z).map(|y| (y, *tag))
            })
            .collect()
    }
}

fn height_at(t: &[[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = t;
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-9 {
        return None;
    }
    let l0 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let l1 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let l2 = 1.0 - l0 - l1;
    const E: f32 = -1e-4;
    (l0 >= E && l1 >= E && l2 >= E).then(|| l0 * a[1] + l1 * b[1] + l2 * c[1])
}

fn is_water(shader: &str) -> bool {
    mw_assets::backdrop::is_water_shader(shader) || shader.to_ascii_lowercase().contains("standingwater")
}

/// One placed water mesh (deduplicated over the units that list the same placement).
struct WaterMesh {
    model: ResourceId,
    mesh_index: usize,
    shader: String,
    backdrop: bool,
    units: Vec<u32>,
    tris: Vec<[[f32; 3]; 3]>,
    lods: usize,
}

pub fn run(store: &AssetStore, wash: &[u8], step: f32, top: usize, composed: bool) -> Result<()> {
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut ground = Ground::default();
    let mut meshes: Vec<WaterMesh> = Vec::new();
    let mut by_key: HashMap<(ResourceId, usize, [u32; 16], bool), usize> = HashMap::new();
    let mut listings: Vec<(u32, ResourceId, [[f32; 4]; 4])> = Vec::new();
    let mut is_water_model: HashMap<ResourceId, bool> = HashMap::new();
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = mw_assets::backdrop::track_unit_number(name).unwrap_or(0);
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                ground.add(&list);
            }
        }
        let Ok(placements) = store.bundle_placements(&b) else { continue };
        for p in placements {
            if p.backdrop {
                listings.push((unit, p.model, p.transform));
            }
            let water = *is_water_model.entry(p.model).or_insert_with(|| {
                store.model_meshes(p.model, 0).is_ok_and(|(m, _)| m.iter().any(|sm| is_water(&sm.shader_name)))
            });
            if !water {
                continue;
            }
            let Ok((ms, _)) = store.model_meshes(p.model, 0) else { continue };
            let lods = store.model(p.model).map(|m| m.state_renderables.iter().take(6).flatten().count()).unwrap_or(0);
            let bits: [u32; 16] = std::array::from_fn(|i| p.transform[i / 4][i % 4].to_bits());
            for sm in ms.iter().filter(|sm| is_water(&sm.shader_name)) {
                let key = (p.model, sm.mesh_index, bits, p.backdrop);
                if let Some(&i) = by_key.get(&key) {
                    meshes[i].units.push(unit);
                    continue;
                }
                let m = p.transform;
                let world = |q: [f32; 4]| -> [f32; 3] {
                    std::array::from_fn(|k| q[0] * m[0][k] + q[1] * m[1][k] + q[2] * m[2][k] + m[3][k])
                };
                let pos: Vec<[f32; 3]> = sm.mesh.positions.iter().map(|&q| world(q)).collect();
                if std::env::var_os("MW_WATER_XFORM").is_some() {
                    let ly = sm.mesh.positions.iter().map(|v| v[1]);
                    println!(
                        "xform {} mesh {} unit {unit} backdrop {} local y {:.1}..{:.1} rows {:?}",
                        p.model,
                        sm.mesh_index,
                        p.backdrop,
                        ly.clone().fold(f32::MAX, f32::min),
                        ly.fold(f32::MIN, f32::max),
                        p.transform.map(|r| r.map(|v| (v * 1000.0).round() / 1000.0))
                    );
                }
                let tris = sm
                    .mesh
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter_map(|t| {
                        Some([*pos.get(t[0] as usize)?, *pos.get(t[1] as usize)?, *pos.get(t[2] as usize)?])
                    })
                    .collect();
                by_key.insert(key, meshes.len());
                meshes.push(WaterMesh {
                    model: p.model,
                    mesh_index: sm.mesh_index,
                    shader: sm.shader_name.clone(),
                    backdrop: p.backdrop,
                    units: vec![unit],
                    tris,
                    lods,
                });
            }
        }
        store.evict_all();
    }
    eprintln!("{} collision triangles, {} water meshes", ground.tris.len(), meshes.len());
    // MW_WATER_DUMP=<file>: every water triangle (world space) as `mesh ax ay az bx by bz cx cy cz` lines.
    if let Ok(path) = std::env::var("MW_WATER_DUMP") {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (mi, w) in meshes.iter().enumerate() {
            for t in &w.tris {
                let _ = writeln!(
                    out,
                    "{mi} {}",
                    t.iter().flatten().map(|v| format!("{v:.2}")).collect::<Vec<_>>().join(" ")
                );
            }
        }
        for z in &store.zone_list()?.zones {
            let pts: Vec<String> = z.points.iter().map(|p| format!("{:.0} {:.0}", p[0], p[1])).collect();
            let _ = writeln!(out, "zone {} {}", z.id, pts.join(" "));
        }
        std::fs::write(path, out)?;
    }
    let selector =
        if composed { Some(store.backdrop_selector(listings.iter().map(|(u, m, t)| (*u, *m, t)))?) } else { None };
    let zones = store.zone_list()?;
    // MW_WATER_AT=x,z;x,z...: the collision column at each point.
    // `grid:x0,x1,z0,z1,step` expands to a grid.
    let mut at = std::env::var("MW_WATER_AT").unwrap_or_default();
    if let Some(g) = at.strip_prefix("grid:") {
        let g: Vec<f32> = g.split(',').filter_map(|s| s.parse().ok()).collect();
        let mut pts = Vec::new();
        if g.len() == 5 {
            let mut x = g[0];
            while x <= g[1] {
                let mut z = g[2];
                while z <= g[3] {
                    pts.push(format!("{x},{z}"));
                    z += g[4];
                }
                x += g[4];
            }
        }
        at = pts.join(";");
    }
    for p in at.split(';').filter(|s| !s.is_empty()) {
        let v: Vec<f32> = p.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if v.len() == 2 {
            let mut ys: Vec<(f32, fmt::CollisionTag)> = ground.column(v[0], v[1]);
            ys.sort_by(|a, b| a.0.total_cmp(&b.0));
            let s: Vec<String> =
                ys.iter().map(|(y, t)| format!("{y:.1}/s{}/u{}/{:#x}", t.surface, t.unit, t.flags)).collect();
            println!("at {},{}: {}", v[0], v[1], s.join(" "));
        }
    }
    if std::env::var_os("MW_WATER_UNITS").is_some() {
        // Collision bounds per unit (tag) against the zone polygon's bounds.
        let mut b: BTreeMap<u16, [f32; 4]> = BTreeMap::new();
        for (t, tag) in &ground.tris {
            let e = b.entry(tag.unit).or_insert([f32::MAX, f32::MAX, f32::MIN, f32::MIN]);
            for v in t {
                *e = [e[0].min(v[0]), e[1].min(v[2]), e[2].max(v[0]), e[3].max(v[2])];
            }
        }
        for (u, e) in &b {
            let z = zones.zones.iter().find(|z| z.id == *u as u32).map(|z| z.bounds);
            println!("unit {u:<5} collision {:?} zone {:?}", e.map(|v| v.round()), z.map(|z| z.map(|v| v.round())));
        }
    }

    // Region where collision exists (the zones): sample only there.
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for z in &zones.zones {
        lo = [lo[0].min(z.bounds[0]), lo[1].min(z.bounds[1])];
        hi = [hi[0].max(z.bounds[2]), hi[1].max(z.bounds[3])];
    }
    // Floating samples per (mesh, 64 m cell): count, worst dy, sample.
    // Backdrop water drawn in a zone that does not list it (our fill-in): (zone, mesh) -> class counts.
    let mut fill: BTreeMap<(u32, usize), [usize; 4]> = BTreeMap::new();
    // MW_WATER_TRACE=zone:mesh prints some of that pair's fill-in samples with their collision column.
    let trace: Option<(u32, usize)> = std::env::var("MW_WATER_TRACE").ok().and_then(|v| {
        let (a, b) = v.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    });
    /// A sample: mesh index, water height, point, ground height, ground tag, owning zone.
    type Sample = (usize, f32, [f32; 3], f32, fmt::CollisionTag, Option<u32>);
    let mut spots: Vec<Sample> = Vec::new();
    println!(
        "{:<3} {:<18} {:>2} {:<34} {:<4} {:>3} {:>7} {:>7} {:>16} {:>16} {:>6} | {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}  units",
        "#",
        "model",
        "m",
        "shader",
        "kind",
        "lod",
        "ymin",
        "ymax",
        "x",
        "z",
        "tris",
        "n",
        "wet",
        "dry",
        "drawn",
        "own",
        "cover",
        "none"
    );
    for (mi, w) in meshes.iter().enumerate() {
        let ys = w.tris.iter().flatten().map(|v| v[1]);
        let (ymin, ymax) = (ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max));
        let xs = w.tris.iter().flatten().map(|v| v[0]);
        let zs = w.tris.iter().flatten().map(|v| v[2]);
        let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
        let (z0, z1) = (zs.clone().fold(f32::MAX, f32::min), zs.fold(f32::MIN, f32::max));
        // Grid samples inside the triangles (clipped to the zones' region).
        let (sx0, sx1) = (x0.max(lo[0]), x1.min(hi[0]));
        let (sz0, sz1) = (z0.max(lo[1]), z1.min(hi[1]));
        let [mut n, mut wet, mut dry, mut drawn, mut cover, mut none, mut own] = [0usize; 7];
        let mut cells: HashMap<(i32, i32), Sample> = HashMap::new();
        if sx0 < sx1 && sz0 < sz1 {
            let mut z = (sz0 / step).ceil() * step;
            while z <= sz1 {
                let mut x = (sx0 / step).ceil() * step;
                while x <= sx1 {
                    let wy = w.tris.iter().filter_map(|t| height_at(t, x, z)).fold(f32::MIN, f32::max);
                    if wy > f32::MIN {
                        n += 1;
                        let col = ground.column(x, z);
                        let bed = col.iter().filter(|(y, _)| *y <= wy + 0.25).max_by(|a, b| a.0.total_cmp(&b.0));
                        let above = col.iter().any(|(y, _)| *y > wy + 0.25);
                        // 0 wet, 1 dry, 2 no collision, 3 covered by collision above.
                        let class = match bed {
                            None if above => 3,
                            None => 2,
                            // A wash surface under the water is a ford or a shallow bank. (Flag 0x40 is on walls
                            // and banks too, and the 0x140 planes are kill planes in mid-air: neither marks water.)
                            Some((_, tag)) if wash.contains(&tag.surface) => 0,
                            Some((y, _)) if wy - y < 0.25 => 0,
                            Some(_) => 1,
                        };
                        *[&mut wet, &mut dry, &mut none, &mut cover][class] += 1;
                        let keep =
                            !w.backdrop || selector.as_ref().is_none_or(|s| s.keeps(w.model, w.mesh_index, x, z));
                        let zone = zones.zone_at(x, z).map(|z| z.id);
                        if keep && w.backdrop && zone.is_some_and(|z| !w.units.contains(&z)) {
                            let c = fill.entry((zone.unwrap_or(0), mi)).or_default();
                            c[class] += 1;
                            if trace == Some((zone.unwrap_or(0), mi)) && c.iter().sum::<usize>() % 37 == 1 {
                                let mut ys: Vec<String> =
                                    col.iter().map(|(y, t)| format!("{y:.1}/s{}/{:#x}", t.surface, t.flags)).collect();
                                ys.sort();
                                println!("sample {x:.0},{wy:.1},{z:.0} class {class} column {}", ys.join(" "));
                            }
                        }
                        if let (1, Some(&(y, tag))) = (class, bed)
                            && keep
                        {
                            drawn += 1;
                            if zone.is_some_and(|z| w.units.contains(&z)) {
                                own += 1;
                            }
                            let c = cells.entry(((x / 64.0).floor() as i32, (z / 64.0).floor() as i32)).or_insert((
                                0,
                                0.0,
                                [x, wy, z],
                                y,
                                tag,
                                zone,
                            ));
                            c.0 += 1;
                            if wy - y > c.1 {
                                *c = (c.0, wy - y, [x, wy, z], y, tag, c.5);
                            }
                        }
                    }
                    x += step;
                }
                z += step;
            }
        }
        let mut u = w.units.clone();
        u.sort();
        u.dedup();
        let shown: Vec<String> = u
            .iter()
            .take(if std::env::var_os("MW_WATER_ALL_UNITS").is_some() { 999 } else { 8 })
            .map(|u| u.to_string())
            .collect();
        println!(
            "{mi:<3} {} {:>2} {:<34} {:<4} {:>3} {ymin:>7.1} {ymax:>7.1} {:>16} {:>16} {:>6} | {n:>6} {wet:>6} {dry:>6} {drawn:>6} {own:>6} {cover:>6} {none:>6}  {}{}",
            w.model,
            w.mesh_index,
            w.shader.chars().take(34).collect::<String>(),
            if w.backdrop { "back" } else { "main" },
            w.lods,
            format!("{x0:.0}..{x1:.0}"),
            format!("{z0:.0}..{z1:.0}"),
            w.tris.len(),
            shown.join(","),
            if u.len() > shown.len() { format!(" (+{})", u.len() - shown.len()) } else { String::new() }
        );
        spots.extend(cells.into_values().map(|c| (mi, c.1, c.2, c.3, c.4, c.5)));
    }
    println!("== drawn water over dry ground: worst 64 m cells by height above the ground (drivable first)");
    let drivable = |t: &fmt::CollisionTag| t.has(fmt::CollisionTag::DRIVABLE) as u8;
    spots.sort_by(|a, b| drivable(&b.4).cmp(&drivable(&a.4)).then(b.1.total_cmp(&a.1)));
    let mut per_mesh: BTreeMap<usize, usize> = BTreeMap::new();
    for s in &spots {
        *per_mesh.entry(s.0).or_default() += 1;
    }
    for (mi, p, wy_at, gy, tag, zone) in spots.iter().take(top) {
        println!(
            "mesh {mi:<3} {:+6.1} m  at {:.0},{:.1},{:.0}  ground {gy:.1} surface {} flags {:#x} zone {:?}{}",
            p,
            wy_at[0],
            wy_at[1],
            wy_at[2],
            tag.surface,
            tag.flags,
            zone,
            if zone.is_none() {
                zones
                    .nearest(wy_at[0], wy_at[2])
                    .map(|z| format!(" (nearest {} at {:.0} m)", z.id, z.distance(wy_at[0], wy_at[2])))
                    .unwrap_or_default()
            } else {
                String::new()
            }
        );
    }
    println!("== fill-in (backdrop water drawn in zones that do not list it): zone mesh -> wet dry none cover");
    for ((zone, mi), c) in &fill {
        println!("zone {zone:<5} mesh {mi:<3} {:>6} {:>6} {:>6} {:>6}", c[0], c[1], c[2], c[3]);
    }
    println!("== 64 m cells with drawn floating water per mesh: {per_mesh:?}");
    Ok(())
}
