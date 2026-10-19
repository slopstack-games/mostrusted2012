//! `mwtool placement-check --floating`: placements (trees, alpha cards, props, compounds, dynamics,
//! groundcover) whose base stands above every **drawn** surface under it, i.e. what floats on screen.
//!
//! Every surface over a placement's column (vertical line through the base's x, z) is recorded with its source:
//! - main: the non-backdrop instances of a track unit (LOD0, no hidden or water shader, not a tree / card model),
//!   flagged by whether the main view draws the model (`Model::in_main_view`; the others are the pass-mask
//!   stand-ins, research log 2026-10-16);
//! - backdrop: every backdrop model whole (with the zones that list it), and the composed backdrop
//!   (`mw_assets::backdrop`, all units loaded) that the runtime draws by default;
//! - collision: the track units' `PolygonSoupList`s.
//!
//! A placement floats when the nearest drawn surface in its column lies more than `MIN_GAP` below its base (a
//! surface closer above the base means a sunk trunk or a buried root, not a float).
//!
//! Two views are judged. **Static** (`--composed-backdrop`): drawn = every unit's main geometry + the composed
//! backdrop. **Camera zone** (what the game draws with the camera in zone Z): drawn = the main geometry of Z
//! and its `ZoneList` neighbours + the backdrop models Z lists; the placements shown are those of Z and its
//! neighbours and the groundcover collections they list (variant G: only those Z lists). `--floating` prints the static clusters and, summed over
//! camera zones, how many placements within `NEAR` of the zone float under several drawing rules (A is the
//! game's, decomp.md "Streaming zone and backdrop swap"; G adds its groundcover rule), plus road samples with
//! drawn terrain 5 cm to 3 m above them.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use mw_assets::AssetStore;
use mw_assets::backdrop::track_unit_number;
use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;

const CELL: f32 = 16.0;
const MIN_GAP: f32 = 1.0;
const UP_TOL: f32 = 0.3;
/// Surfaces farther than this above or below a base are not recorded.
const REACH: f32 = 150.0;
/// Camera-zone judgement: placements within this of the zone polygon.
const NEAR: f32 = 600.0;
/// Road sample spacing (metres).
const ROAD_STEP: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Kind {
    Tree,
    Card,
    Prop,
    Compound,
    Dynamic,
    Groundcover,
    /// A sample of a road mesh (for the terrain-above-road count).
    Road,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Src {
    Main {
        unit: u32,
        model: ResourceId,
        view: bool,
    },
    /// A backdrop model, whole.
    Backdrop(ResourceId),
    /// A backdrop model as the static composition keeps it.
    Composed(ResourceId),
    Collision,
}

struct Query {
    base: [f32; 3],
    kind: Kind,
    /// Track unit (0 for groundcover).
    unit: u32,
    /// Model, or the groundcover collection.
    model: ResourceId,
    /// (surface y, index into `sources`).
    hits: Vec<(f32, u32)>,
}

impl Query {
    /// Gap between the base and the nearest accepted surface below it, when the placement floats.
    fn floats(&self, accept: impl Fn(u32) -> bool) -> Option<f32> {
        let b = self.base[1];
        let (mut below, mut above) = (None::<f32>, None::<f32>);
        for &(y, _) in self.hits.iter().filter(|(_, s)| accept(*s)) {
            if y <= b + UP_TOL {
                below = Some(below.map_or(y, |h| h.max(y)));
            } else {
                above = Some(above.map_or(y, |h| h.min(y)));
            }
        }
        let gap = below.map_or(f32::INFINITY, |y| b - y);
        (gap > MIN_GAP && above.is_none_or(|y| y - b > gap)).then_some(gap)
    }

    /// Road sample under drawn ground: an accepted terrain / plot surface of another model 5 cm to 3 m above it.
    fn pokes(&self, accept: impl Fn(u32) -> bool, sources: &[(Src, String)]) -> bool {
        self.hits.iter().any(|&(y, s)| {
            let (src, shader) = &sources[s as usize];
            let own = matches!(src, Src::Main { model, .. } if *model == self.model);
            let dy = y - self.base[1];
            accept(s)
                && !own
                && dy > 0.05
                && dy <= 3.0
                && (shader.starts_with("TerrainPBR") || shader.starts_with("PlotPBR"))
        })
    }

    /// Whether an accepted surface lies within `MIN_GAP` of the base.
    fn near(&self, accept: impl Fn(u32) -> bool) -> bool {
        self.hits.iter().any(|&(y, s)| accept(s) && (y - self.base[1]).abs() < MIN_GAP)
    }
}

struct Columns {
    q: Vec<Query>,
    grid: HashMap<(i32, i32), Vec<u32>>,
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

fn world(m: &[[f32; 4]; 4], q: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|k| q[0] * m[0][k] + q[1] * m[1][k] + q[2] * m[2][k] + m[3][k])
}

impl Columns {
    fn tri(&mut self, t: [[f32; 3]; 3], src: u32) {
        let c = |v: f32| (v / CELL).floor() as i32;
        let (x0, x1) =
            (t.iter().map(|v| v[0]).fold(f32::MAX, f32::min), t.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
        let (z0, z1) =
            (t.iter().map(|v| v[2]).fold(f32::MAX, f32::min), t.iter().map(|v| v[2]).fold(f32::MIN, f32::max));
        if !(x0.is_finite() && x1.is_finite() && z0.is_finite() && z1.is_finite()) {
            return;
        }
        if (c(x1) - c(x0)) as i64 * (c(z1) - c(z0)) as i64 > 1 << 16 {
            return;
        }
        for cx in c(x0)..=c(x1) {
            for cz in c(z0)..=c(z1) {
                let Some(list) = self.grid.get(&(cx, cz)) else { continue };
                for &i in list {
                    let q = &mut self.q[i as usize];
                    let Some(y) = height_at(&t, q.base[0], q.base[2]) else { continue };
                    if (y - q.base[1]).abs() <= REACH && !q.hits.contains(&(y, src)) {
                        q.hits.push((y, src));
                    }
                }
            }
        }
    }

    fn mesh(&mut self, mesh: &fmt::DecodedMesh, m: &[[f32; 4]; 4], src: u32) {
        let pos: Vec<[f32; 3]> = mesh.positions.iter().map(|&q| world(m, q)).collect();
        for t in mesh.indices.as_chunks::<3>().0 {
            let (Some(&a), Some(&b), Some(&c)) =
                (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize))
            else {
                continue;
            };
            self.tri([a, b, c], src);
        }
    }
}

/// Model class for the query set: tree (a `Tree_*` shader), card (another alpha-tested `1Bit` shader), or a
/// surface model; plus the lowest vertex y (model space).
fn classify(store: &AssetStore, model: ResourceId) -> (Option<Kind>, f32) {
    let Ok((meshes, _)) = store.model_meshes(model, 0) else { return (None, 0.0) };
    let min_y = meshes.iter().flat_map(|m| &m.mesh.positions).map(|v| v[1]).fold(f32::MAX, f32::min);
    let min_y = if min_y.is_finite() && min_y < f32::MAX { min_y } else { 0.0 };
    let visible: Vec<&str> =
        meshes.iter().map(|m| m.shader_name.as_str()).filter(|s| !mw_assets::is_hidden_shader(s)).collect();
    if visible.iter().any(|s| s.starts_with("Tree_")) {
        (Some(Kind::Tree), min_y)
    } else if !visible.is_empty() && visible.iter().all(|s| s.contains("1Bit")) {
        (Some(Kind::Card), min_y)
    } else {
        (None, min_y)
    }
}

fn surface_shader(s: &str) -> bool {
    !mw_assets::is_hidden_shader(s) && !mw_assets::backdrop::is_water_shader(s)
}

pub fn run(store: &AssetStore, top: usize) -> Result<()> {
    let units: Vec<String> = store
        .bundle_paths()
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("TRK_UNIT")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // Pass 1: the queries (each groundcover collection once, with the units that list it).
    let mut q: Vec<Query> = Vec::new();
    let mut class: HashMap<ResourceId, (Option<Kind>, f32)> = HashMap::new();
    let mut main_view: HashMap<ResourceId, bool> = HashMap::new();
    let mut gc_units: HashMap<ResourceId, Vec<u32>> = HashMap::new();
    let push = |q: &mut Vec<Query>, base: [f32; 3], kind, unit, model| {
        q.push(Query { base, kind, unit, model, hits: Vec::new() })
    };
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = track_unit_number(name).unwrap_or(0);
        let Ok(wl) = store.bundle_layers(&b) else { continue };
        for p in wl.instances.iter().filter(|p| !p.backdrop) {
            let (kind, min_y) = *class.entry(p.model).or_insert_with(|| classify(store, p.model));
            let shown =
                *main_view.entry(p.model).or_insert_with(|| store.model(p.model).is_ok_and(|m| m.in_main_view()));
            if let (Some(kind), true) = (kind, shown) {
                let t = p.transform;
                push(&mut q, [t[3][0], t[3][1] + min_y * t[1][1], t[3][2]], kind, unit, p.model);
            }
            // Road samples on a ROAD_STEP grid (one per cell and metre of height).
            if kind.is_none() && shown {
                let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
                let mut done = HashSet::new();
                for sm in meshes.iter().filter(|sm| sm.shader_name.starts_with("DriveableSurface")) {
                    let pos: Vec<[f32; 3]> = sm.mesh.positions.iter().map(|&v| world(&p.transform, v)).collect();
                    for t in sm.mesh.indices.as_chunks::<3>().0 {
                        let (Some(&a), Some(&b), Some(&c)) =
                            (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize))
                        else {
                            continue;
                        };
                        let tri = [a, b, c];
                        let g = |v: f32| (v / ROAD_STEP).floor() as i32;
                        let (x0, x1) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
                        let (z0, z1) = (a[2].min(b[2]).min(c[2]), a[2].max(b[2]).max(c[2]));
                        if g(x1) - g(x0) > 64 || g(z1) - g(z0) > 64 {
                            continue;
                        }
                        for i in g(x0)..=g(x1) {
                            for j in g(z0)..=g(z1) {
                                let (x, z) = ((i as f32 + 0.5) * ROAD_STEP, (j as f32 + 0.5) * ROAD_STEP);
                                let Some(y) = height_at(&tri, x, z) else { continue };
                                if done.insert((i, j, y.floor() as i32)) {
                                    push(&mut q, [x, y, z], Kind::Road, unit, p.model);
                                }
                            }
                        }
                    }
                }
            }
        }
        for (kind, list) in [(Kind::Prop, &wl.props), (Kind::Compound, &wl.compounds), (Kind::Dynamic, &wl.dynamics)] {
            for p in list {
                let t = p.transform;
                push(&mut q, [t[3][0], t[3][1], t[3][2]], kind, unit, p.model);
            }
        }
        for e in b.entries_of_type(ResourceType::GroundcoverCollection) {
            let listed = gc_units.entry(e.id).or_default();
            listed.push(unit);
            if listed.len() > 1 {
                continue;
            }
            let Ok(gc) = store.groundcover(e.id) else { continue };
            for i in &gc.instances {
                push(&mut q, i.position, Kind::Groundcover, 0, gc.id);
            }
        }
        store.evict_all();
    }
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, x) in q.iter().enumerate() {
        grid.entry(((x.base[0] / CELL).floor() as i32, (x.base[2] / CELL).floor() as i32)).or_default().push(i as u32);
    }
    eprintln!("{} placements to check", q.len());
    let mut cols = Columns { q, grid };

    // Pass 2: surfaces.
    let mut sources: Vec<(Src, String)> = Vec::new();
    let mut src_index: HashMap<(Src, String), u32> = HashMap::new();
    let mut source = |src: Src, shader: &str| -> u32 {
        *src_index.entry((src, shader.to_string())).or_insert_with(|| {
            sources.push((src, shader.to_string()));
            sources.len() as u32 - 1
        })
    };
    let mut listings: Vec<(u32, ResourceId, [[f32; 4]; 4])> = Vec::new();
    let collision = source(Src::Collision, "");
    for name in &units {
        let Some(b) = store.bundle(name) else { continue };
        let unit = track_unit_number(name).unwrap_or(0);
        for e in b.entries_of_type(ResourceType::PolygonSoupList) {
            if let Ok(list) = fmt::PolygonSoupList::parse(&b.load(e)?) {
                let (verts, tris) = list.triangles();
                for t in &tris {
                    cols.tri(t.map(|i| verts[i as usize]), collision);
                }
            }
        }
        let Ok(wl) = store.bundle_layers(&b) else { continue };
        for p in &wl.instances {
            if p.backdrop {
                listings.push((unit, p.model, p.transform));
                continue;
            }
            if class.get(&p.model).is_some_and(|c| c.0.is_some()) {
                continue;
            }
            let view = store.model(p.model).is_ok_and(|m| m.in_main_view());
            let Ok((meshes, _)) = store.model_meshes(p.model, 0) else { continue };
            for sm in meshes.iter().filter(|sm| surface_shader(&sm.shader_name)) {
                let src = source(Src::Main { unit, model: p.model, view }, &sm.shader_name);
                cols.mesh(&sm.mesh, &p.transform, src);
            }
        }
        store.evict_all();
    }
    let sel = store.backdrop_selector(listings.iter().map(|(u, m, t)| (*u, *m, t)))?;
    let mut listed_by: HashMap<ResourceId, HashSet<u32>> = HashMap::new();
    let mut sent = HashSet::new();
    let mut sizes = [0usize; 4];
    for (u, model, t) in &listings {
        listed_by.entry(*model).or_default().insert(*u);
        let bits: [u32; 16] = std::array::from_fn(|i| t[i / 4][i % 4].to_bits());
        if !sent.insert((*model, bits)) {
            continue;
        }
        let Ok((meshes, _)) = store.model_meshes(*model, 0) else { continue };
        for sm in meshes.iter().filter(|sm| surface_shader(&sm.shader_name)) {
            let src = source(Src::Backdrop(*model), &sm.shader_name);
            cols.mesh(&sm.mesh, t, src);
            sizes[0] += sm.mesh.positions.len();
            sizes[1] += sm.mesh.indices.len() / 3;
            let Some(kept) = sel.filter_mesh(*model, sm.mesh_index, &sm.mesh, t) else { continue };
            sizes[2] += kept.positions.len();
            sizes[3] += kept.indices.len() / 3;
            let src = source(Src::Composed(*model), &sm.shader_name);
            cols.mesh(&kept, t, src);
        }
    }
    store.evict_all();
    eprintln!(
        "backdrop solids: whole {} vertices / {} triangles, composed {} / {}",
        sizes[0], sizes[1], sizes[2], sizes[3]
    );
    let cols = cols;

    // Static view.
    let drawn_static = |s: u32| matches!(sources[s as usize].0, Src::Main { view: true, .. } | Src::Composed(_));
    let standin = |s: u32| matches!(sources[s as usize].0, Src::Main { view: false, .. });
    let coll = |s: u32| s == collision;
    let floating: Vec<(u32, f32)> = cols
        .q
        .iter()
        .enumerate()
        .filter(|(_, x)| x.kind != Kind::Road)
        .filter_map(|(i, x)| Some((i as u32, x.floats(drawn_static)?)))
        .collect();
    let gap_of: HashMap<u32, f32> = floating.iter().copied().collect();
    let mut per_kind: BTreeMap<Kind, [usize; 4]> = BTreeMap::new();
    for x in &cols.q {
        per_kind.entry(x.kind).or_default()[0] += 1;
    }
    for &(i, _) in &floating {
        let x = &cols.q[i as usize];
        let e = per_kind.entry(x.kind).or_default();
        e[1] += 1;
        e[2] += x.near(standin) as usize;
        e[3] += x.near(coll) as usize;
    }
    println!(
        "== static view (all main geometry + composed backdrop): nearest drawn surface more than {MIN_GAP} m below the base"
    );
    println!("kind          total   floating  stand-in-under  collision-under");
    for (k, v) in &per_kind {
        println!("{:<12} {:7} {:9} {:14} {:16}", format!("{k:?}"), v[0], v[1], v[2], v[3]);
    }
    if let Some(path) = std::env::var_os("MW_FLOAT_CSV") {
        use std::fmt::Write as _;
        let mut out = String::from("kind,model,unit,x,y,z,gap,standin,collision\n");
        for &(i, gap) in &floating {
            let x = &cols.q[i as usize];
            let _ = writeln!(
                out,
                "{:?},{},{},{:.2},{:.2},{:.2},{:.2},{},{}",
                x.kind,
                x.model,
                x.unit,
                x.base[0],
                x.base[1],
                x.base[2],
                gap.min(999.0),
                x.near(standin),
                x.near(coll)
            );
        }
        std::fs::write(path, out)?;
    }
    // Cluster on a 32 m grid (flood over neighbours).
    let key = |p: [f32; 3]| ((p[0] / 32.0).floor() as i32, (p[2] / 32.0).floor() as i32);
    let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for &(i, _) in &floating {
        cells.entry(key(cols.q[i as usize].base)).or_default().push(i);
    }
    let mut seen = HashSet::new();
    let mut clusters: Vec<Vec<u32>> = Vec::new();
    let mut starts: Vec<_> = cells.keys().copied().collect();
    starts.sort();
    for s in starts {
        if !seen.insert(s) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![s];
        while let Some(k) = stack.pop() {
            members.extend(&cells[&k]);
            for d in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                let n = (k.0 + d.0, k.1 + d.1);
                if cells.contains_key(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
        clusters.push(members);
    }
    // Rank: non-groundcover placements count 10x (a floating tree shows more than a tuft).
    let weight = |c: &Vec<u32>| -> usize {
        c.iter().map(|&i| if cols.q[i as usize].kind == Kind::Groundcover { 1 } else { 10 }).sum()
    };
    clusters.sort_by_key(|c| std::cmp::Reverse(weight(c)));
    let zones = store.zone_list()?;
    println!("== static view: top {top} clusters (32 m cells): count by kind, worst member and its gap");
    for c in clusters.iter().take(top) {
        let mut kinds: BTreeMap<Kind, usize> = BTreeMap::new();
        for &i in c {
            *kinds.entry(cols.q[i as usize].kind).or_default() += 1;
        }
        let w = *c
            .iter()
            .max_by(|&&a, &&b| {
                let k = |i: u32| ((cols.q[i as usize].kind != Kind::Groundcover) as u8, gap_of[&i].min(999.0));
                k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let x = &cols.q[w as usize];
        let stand = c.iter().filter(|&&i| cols.q[i as usize].near(standin)).count();
        let col = c.iter().filter(|&&i| cols.q[i as usize].near(coll)).count();
        println!(
            "{:6} {kinds:?} stand-in under {stand}, collision under {col}; worst {:?} {} unit {} at {:.1},{:.1},{:.1} gap {:.1} zone {:?}",
            c.len(),
            x.kind,
            x.model,
            x.unit,
            x.base[0],
            x.base[1],
            x.base[2],
            gap_of[&w].min(999.0),
            zones.zone_at(x.base[0], x.base[2]).map(|z| z.id),
        );
    }

    // Camera-zone views, per zone: the placements shown with the camera in Z (non-groundcover of Z and its
    // neighbours, groundcover listed by them) judged against several rules for what is drawn.
    let near: f32 = std::env::var("MW_FLOAT_NEAR").ok().and_then(|v| v.parse().ok()).unwrap_or(NEAR);
    const VARIANTS: [&str; 7] = [
        "S static: all main + composed backdrop",
        "A main of Z+nbrs + Z's backdrop list",
        "C main of Z+nbrs+nbrs' nbrs + Z's list",
        "D all main + Z's list",
        "E main of Z+nbrs + lists of Z+nbrs",
        "F all main + composed + Z's list",
        "G = A, groundcover only of collections Z lists",
    ];
    println!("== per camera zone Z: placements within {near} m of Z that float, by drawing rule (non-gc / gc)");
    let per_zone = std::env::var_os("MW_FLOAT_ZONES").is_some();
    let diff_zone: Option<u32> = std::env::var("MW_FLOAT_DIFF").ok().and_then(|v| v.parse().ok());
    let mut totals = [[0usize; 3]; 8];
    for zone in &zones.zones {
        let nbrs: HashSet<u32> = zone.neighbours.iter().map(|&(i, _)| zones.zones[i].id).chain([zone.id]).collect();
        let nbrs2: HashSet<u32> = zone
            .neighbours
            .iter()
            .flat_map(|&(i, _)| zones.zones[i].neighbours.iter().map(|&(j, _)| zones.zones[j].id))
            .chain(nbrs.iter().copied())
            .collect();
        let lists =
            |m: &ResourceId, by: &dyn Fn(u32) -> bool| listed_by.get(m).is_some_and(|s| s.iter().any(|&u| by(u)));
        let rule = |v: usize, s: u32| -> bool {
            let z = zone.id;
            match (v, sources[s as usize].0) {
                (0, src) => drawn_static(s) && !matches!(src, Src::Backdrop(_)),
                (1 | 4 | 6, Src::Main { unit, view: true, .. }) => nbrs.contains(&unit),
                (2, Src::Main { unit, view: true, .. }) => nbrs2.contains(&unit),
                (3 | 5, Src::Main { view: true, .. }) => true,
                (1..=3 | 5 | 6, Src::Backdrop(m)) => lists(&m, &|u| u == z),
                (4, Src::Backdrop(m)) => lists(&m, &|u| nbrs.contains(&u)),
                (5, Src::Composed(_)) => true,
                _ => false,
            }
        };
        let mut row = [[0usize; 3]; 8];
        let [x0, z0, x1, z1] = zone.bounds;
        let c = |v: f32| (v / CELL).floor() as i32;
        for cx in c(x0 - near)..=c(x1 + near) {
            for cz in c(z0 - near)..=c(z1 + near) {
                for &i in cols.grid.get(&(cx, cz)).into_iter().flatten() {
                    let x = &cols.q[i as usize];
                    if zone.distance(x.base[0], x.base[2]) > near {
                        continue;
                    }
                    let gc = x.kind == Kind::Groundcover;
                    let shown = if gc {
                        gc_units.get(&x.model).is_some_and(|us| us.iter().any(|u| nbrs.contains(u)))
                    } else {
                        nbrs.contains(&x.unit)
                    };
                    if !shown {
                        continue;
                    }
                    let k = if x.kind == Kind::Road { 2 } else { gc as usize };
                    row[0][k] += 1;
                    let own_gc = !gc || gc_units.get(&x.model).is_some_and(|us| us.contains(&zone.id));
                    for v in 0..VARIANTS.len() {
                        if v == 6 && !own_gc {
                            continue;
                        }
                        row[1 + v][k] +=
                            if k == 2 { x.pokes(|s| rule(v, s), &sources) } else { x.floats(|s| rule(v, s)).is_some() }
                                as usize;
                    }
                    // `MW_FLOAT_DIFF=<zone>`: placements that float with the camera-zone rule (A) but not
                    // in the static view.
                    if k == 0
                        && diff_zone == Some(zone.id)
                        && let Some(gap) = x.floats(|s| rule(1, s))
                        && x.floats(|s| rule(0, s)).is_none()
                    {
                        let under: Vec<String> = x
                            .hits
                            .iter()
                            .map(|&(y, s)| format!("{:.1} {:?} {}", y, sources[s as usize].0, &sources[s as usize].1))
                            .collect();
                        println!(
                            "  A-only {:?} {} unit {} at {:.1},{:.1},{:.1} gap {gap:.1}: {}",
                            x.kind,
                            x.model,
                            x.unit,
                            x.base[0],
                            x.base[1],
                            x.base[2],
                            under.join(" | ")
                        );
                    }
                }
            }
        }
        for (t, r) in totals.iter_mut().zip(row) {
            for k in 0..3 {
                t[k] += r[k];
            }
        }
        if per_zone {
            let cells: Vec<String> = row.iter().map(|r| format!("{}/{}/{}", r[0], r[1], r[2])).collect();
            println!(
                "zone {:<5} shown {}  S A C D E F G (non-gc/gc/road): {}",
                zone.id,
                cells[0],
                cells[1..].join("  ")
            );
        }
    }
    println!(
        "shown (non-gc / gc / road samples every {ROAD_STEP} m): {}/{}/{}",
        totals[0][0], totals[0][1], totals[0][2]
    );
    println!("rule                                         floating non-gc / gc, road samples under ground");
    for (v, name) in VARIANTS.iter().enumerate() {
        println!("{name:<44} {:>7}/{}/{}", totals[1 + v][0], totals[1 + v][1], totals[1 + v][2]);
    }
    Ok(())
}
