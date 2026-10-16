//! Which backdrop geometry to draw when every track unit is loaded at once.
//!
//! A track unit's `InstanceList` entries past the "always loaded" count are its **backdrop**: low-detail
//! world-space geometry (terrain tiles, distant building blocks, LOD stand-ins, water planes, landmark
//! towers) that the game draws while the player is *in that unit*, to fill in everything that is not
//! streamed in at full detail. Each unit lists its own potentially visible set at the level of detail that
//! suits its distance, so the same hillside exists as a 22,747-triangle tile in the backdrop of the units
//! on it and as a 2,342-triangle tile in the backdrop of units 2 km away. The game only ever draws one
//! unit's backdrop (plus the main geometry of the loaded units around the player). Drawing the union of
//! all 169 backdrops stacks every tier of every tile on top of each other and on top of the detailed main
//! geometry: coarse terrain pokes through roads ("mountains on the street") and building-block stand-ins
//! stand where the detailed buildings and streets are.
//!
//! With the whole map loaded, [`BackdropSelector`] composes one backdrop for every place, from the view
//! of the zone that owns it (zones come from the `ZoneList` in `HAWAII/PVS.BNDL`; zone ID = unit number):
//!
//! - Inside a zone, a backdrop triangle is drawn when that zone lists its model: exactly what the game
//!   shows there while the player stands in that zone, so the tile tier fits the detailed geometry around
//!   it and stand-ins for buildings elsewhere drop out.
//! - Outside every zone (distant hills, sea, horizon), of the backdrop models covering a spot the ones
//!   listed by the zone nearest to it are drawn: the finest tier anyone uses there.
//! - Water surfaces (`Water_*` shaders) follow the same rule inside a zone: a zone draws only the water it
//!   lists. Far zones list coarse water planes (a 2 x 1 km rectangle at y 13.1 over the river gorge whose
//!   water the gorge's own zones list at y -36.8, a sloped sea plane at y 30.4 over the coastal industrial
//!   area at y 6-20): drawn inside a zone that does not list them they flood it. Outside the zones, water is
//!   composed separately from solid geometry, so a lake bed listed by a near zone does not hide the water
//!   plane a farther zone lists over it, except in the gaps between zone polygons: within `WATER_GAP` (128 m) of a
//!   zone that lists water, only that zone's water is drawn (research log 2026-10-16).
//!
//! Long triangles are split before their centroid decides, so tiles are cut along zone borders. When only
//! some track units are streamed ([`BackdropSelector::set_loaded`]), the other zones count as outside and
//! are filled from the nearest loaded zone's view.

use std::collections::HashMap;
use std::sync::Mutex;

use mw_bundle::{ResourceId, ResourceType};
use mw_formats as fmt;

use crate::{AssetStore, Error, Result};

/// Triangles with an edge longer than this are split before their centroid decides.
const MAX_EDGE: f32 = 48.0;
/// Triangles longer than this are always split (inside the indexed region); shorter ones only where the
/// decision changes across them.
const SPLIT_ALWAYS: f32 = 512.0;
/// Split depth limit (water planes have triangles 40 km long: 40 km / 2^12 = 10 m).
const MAX_DEPTH: u32 = 12;
/// Cell size of the coverage index and of the distance cache.
const CELL: f32 = 128.0;
/// How far beyond the zones the coverage index reaches.
const MARGIN: f32 = 6000.0;
/// Models whose listing zones are this much farther away than the best still count as tied.
const TIE: f32 = 1.0;
/// Water outside every zone but this close to the nearest loaded zone lies in a gap between zone polygons
/// (the river gorge runs along such gaps): if that zone lists water, it owns the spot. Model decision: every
/// floating water sample outside the zones was within 65 m of one (`mwtool water`, 2026-10-16).
const WATER_GAP: f32 = 128.0;

impl AssetStore {
    /// The world's trigger volumes (`TriggerData` in `HAWAII/TRIGGERS.BNDL`, loaded by `FUN_0053c2f0`):
    /// speed cameras, billboards, security gates, jack spots and the other region kinds.
    pub fn trigger_data(&self) -> Result<fmt::TriggerData> {
        let bundle = self.bundle("HAWAII/TRIGGERS.BNDL").ok_or(Error::Missing(ResourceId(0)))?;
        let entry = bundle.entries_of_type(ResourceType::TriggerData).next().ok_or(Error::Missing(ResourceId(0)))?;
        Ok(fmt::TriggerData::parse(&bundle.load(entry)?)?)
    }

    /// The streaming zones (`ZoneList` in `HAWAII/PVS.BNDL`).
    pub fn zone_list(&self) -> Result<fmt::ZoneList> {
        let bundle = self.bundle("HAWAII/PVS.BNDL").ok_or(Error::Missing(ResourceId(0)))?;
        let entry = bundle.entries_of_type(ResourceType::ZoneList).next().ok_or(Error::Missing(ResourceId(0)))?;
        Ok(fmt::ZoneList::parse(&bundle.load(entry)?)?)
    }

    /// The traffic lane network (`TrafficLaneData` in `HAWAII/TRAFFIC.BNDL`); query it with
    /// [`fmt::TrafficLaneData::lanes_near`].
    pub fn traffic_lanes(&self) -> Result<fmt::TrafficLaneData> {
        let bundle = self.bundle("HAWAII/TRAFFIC.BNDL").ok_or(Error::Missing(ResourceId(0)))?;
        let entry =
            bundle.entries_of_type(ResourceType::TrafficLaneData).next().ok_or(Error::Missing(ResourceId(0)))?;
        Ok(fmt::TrafficLaneData::parse(&bundle.load(entry)?)?)
    }

    /// The AI's road graph (`AIData` in `HAWAII/AI/AIDATA.BNDL`): road centre-line nodes with widths.
    pub fn ai_data(&self) -> Result<fmt::AiData> {
        let bundle = self.bundle("HAWAII/AI/AIDATA.BNDL").ok_or(Error::Missing(ResourceId(0)))?;
        let entry = bundle.entries_of_type(ResourceType::AIData).next().ok_or(Error::Missing(ResourceId(0)))?;
        Ok(fmt::AiData::parse(&bundle.load(entry)?)?)
    }

    /// The gameplay road network (`RoadData` in `HAWAII/GAMELOGIC/ROADDATA.BNDL`): roads, sections and points
    /// with the oncoming-lane bounds the nitrous refill reads.
    pub fn road_data(&self) -> Result<fmt::RoadData> {
        let bundle = self.bundle("HAWAII/GAMELOGIC/ROADDATA.BNDL").ok_or(Error::Missing(ResourceId(0)))?;
        let entry = bundle.entries_of_type(ResourceType::RoadData).next().ok_or(Error::Missing(ResourceId(0)))?;
        Ok(fmt::RoadData::parse(&bundle.load(entry)?)?)
    }

    /// A [`BackdropSelector`] for the given backdrop listings `(track unit, model, transform)`: loads the
    /// zones and indexes the drawn meshes of every listed model.
    pub fn backdrop_selector<'a>(
        &self,
        listings: impl IntoIterator<Item = (u32, ResourceId, &'a [[f32; 4]; 4])>,
    ) -> Result<BackdropSelector> {
        let mut sel = BackdropSelector::new(self.zone_list()?);
        let mut placed: HashMap<ResourceId, [[f32; 4]; 4]> = HashMap::new();
        for (unit, model, transform) in listings {
            sel.add_listing(unit, model);
            placed.entry(model).or_insert(*transform);
        }
        let mut models: Vec<_> = placed.into_iter().collect();
        models.sort_by_key(|(m, _)| *m);
        for (model, transform) in models {
            let Ok((meshes, _)) = self.model_meshes(model, 0) else { continue };
            for sm in meshes.iter().filter(|sm| !crate::is_hidden_shader(&sm.shader_name)) {
                sel.add_mesh(model, sm.mesh_index, is_water_shader(&sm.shader_name), &sm.mesh, &transform);
            }
        }
        sel.build_index();
        Ok(sel)
    }
}

/// Water surface shaders (`Water_Opaque_Singlesided`, `Water_Proto_Cheap`).
pub fn is_water_shader(name: &str) -> bool {
    name.starts_with("Water_")
}

/// The track unit number of a `TRK_UNIT<n>.BNDL` path.
pub fn track_unit_number(path: &str) -> Option<u32> {
    let name = path.rsplit(['/', '\\']).next()?;
    let upper = name.to_ascii_uppercase();
    upper.strip_prefix("TRK_UNIT")?.strip_suffix(".BNDL")?.parse().ok()
}

/// One indexed backdrop mesh: its triangles projected on the ground plane.
struct Coverage {
    model: ResourceId,
    water: bool,
    /// `[ax, az, bx, bz, cx, cz]` per triangle.
    triangles: Vec<[f32; 6]>,
}

/// Decides which backdrop triangles to draw (see the module docs).
pub struct BackdropSelector {
    zones: fmt::ZoneList,
    /// Zone index by zone ID (track unit number).
    zone_index: HashMap<u32, usize>,
    /// Zone indices listing each model in their backdrop.
    listed: HashMap<ResourceId, Vec<usize>>,
    meshes: Vec<Coverage>,
    /// Index into `meshes` by (model, mesh index).
    mesh_of: HashMap<(ResourceId, usize), usize>,
    /// Grid origin (x, z) and size in cells.
    origin: [f32; 2],
    size: [usize; 2],
    /// Per grid cell: (mesh, triangle) pairs whose bounding box overlaps the cell.
    cells: Vec<Vec<(u32, u32)>>,
    /// Distance from a cell centre to the nearest loaded zone listing a model, by (model, cell).
    distances: Mutex<HashMap<(ResourceId, i32, i32), f32>>,
    /// Zones whose main geometry is drawn (all by default).
    loaded: Vec<bool>,
    /// Zones that list at least one water mesh (filled by [`Self::build_index`]).
    lists_water: Vec<bool>,
}

impl BackdropSelector {
    pub fn new(zones: fmt::ZoneList) -> Self {
        let zone_index = zones.zones.iter().enumerate().map(|(i, z)| (z.id, i)).collect();
        let zones_len = zones.zones.len();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for z in &zones.zones {
            lo = [lo[0].min(z.bounds[0]), lo[1].min(z.bounds[1])];
            hi = [hi[0].max(z.bounds[2]), hi[1].max(z.bounds[3])];
        }
        let (origin, size) = if lo[0] <= hi[0] {
            let origin = [lo[0] - MARGIN, lo[1] - MARGIN];
            let size = [0, 1].map(|i| ((hi[i] - lo[i] + 2.0 * MARGIN) / CELL).ceil() as usize);
            (origin, size)
        } else {
            ([0.0; 2], [0; 2])
        };
        Self {
            zones,
            zone_index,
            listed: HashMap::new(),
            meshes: Vec::new(),
            mesh_of: HashMap::new(),
            origin,
            size,
            cells: Vec::new(),
            distances: Mutex::new(HashMap::new()),
            loaded: vec![true; zones_len],
            lists_water: vec![false; zones_len],
        }
    }

    /// Only these track units' main geometry is drawn: the area of any other zone is filled from the
    /// view of the nearest loaded zone (its backdrop has the stand-ins for what is not streamed).
    pub fn set_loaded(&mut self, units: impl IntoIterator<Item = u32>) {
        self.loaded = vec![false; self.zones.zones.len()];
        for unit in units {
            if let Some(&z) = self.zone_index.get(&unit) {
                self.loaded[z] = true;
            }
        }
        self.distances.lock().unwrap().clear();
    }

    pub fn zones(&self) -> &fmt::ZoneList {
        &self.zones
    }

    /// Record that track unit `unit` has `model` in its backdrop. Units without a zone are ignored.
    pub fn add_listing(&mut self, unit: u32, model: ResourceId) {
        if let Some(&z) = self.zone_index.get(&unit) {
            let list = self.listed.entry(model).or_default();
            if !list.contains(&z) {
                list.push(z);
            }
        }
    }

    /// Index a mesh of a listed model for coverage queries (call [`Self::build_index`] afterwards).
    pub fn add_mesh(
        &mut self,
        model: ResourceId,
        mesh_index: usize,
        water: bool,
        mesh: &fmt::DecodedMesh,
        transform: &[[f32; 4]; 4],
    ) {
        let p: Vec<[f32; 3]> = mesh.positions.iter().map(|&v| apply(transform, v)).collect();
        let triangles = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|t| {
                let [a, b, c] = t.map(|i| p.get(i as usize).copied());
                let (a, b, c) = (a?, b?, c?);
                Some([a[0], a[2], b[0], b[2], c[0], c[2]])
            })
            .collect();
        self.mesh_of.insert((model, mesh_index), self.meshes.len());
        self.meshes.push(Coverage { model, water, triangles });
    }

    /// Bin the indexed triangles into the coverage grid.
    pub fn build_index(&mut self) {
        self.lists_water = vec![false; self.zones.zones.len()];
        for cov in self.meshes.iter().filter(|c| c.water) {
            for &zone in self.listed.get(&cov.model).into_iter().flatten() {
                self.lists_water[zone] = true;
            }
        }
        let [w, h] = self.size;
        let mut cells = vec![Vec::new(); w * h];
        for (m, cov) in self.meshes.iter().enumerate() {
            for (t, tri) in cov.triangles.iter().enumerate() {
                let (x0, x1) = (tri[0].min(tri[2]).min(tri[4]), tri[0].max(tri[2]).max(tri[4]));
                let (z0, z1) = (tri[1].min(tri[3]).min(tri[5]), tri[1].max(tri[3]).max(tri[5]));
                let (Some((i0, j0)), Some((i1, j1))) = (self.cell_clamped(x0, z0), self.cell_clamped(x1, z1)) else {
                    continue;
                };
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        cells[j * w + i].push((m as u32, t as u32));
                    }
                }
            }
        }
        self.cells = cells;
    }

    /// Grid cell of a position, clamped to the grid (None for an empty grid).
    fn cell_clamped(&self, x: f32, z: f32) -> Option<(usize, usize)> {
        let [w, h] = self.size;
        if w == 0 || h == 0 {
            return None;
        }
        let i = ((x - self.origin[0]) / CELL).floor().clamp(0.0, (w - 1) as f32) as usize;
        let j = ((z - self.origin[1]) / CELL).floor().clamp(0.0, (h - 1) as f32) as usize;
        Some((i, j))
    }

    /// Indexed meshes covering `(x, z)` (each once).
    fn covering(&self, x: f32, z: f32) -> Vec<usize> {
        let [w, h] = self.size;
        let i = ((x - self.origin[0]) / CELL).floor();
        let j = ((z - self.origin[1]) / CELL).floor();
        if self.cells.is_empty() || i < 0.0 || j < 0.0 || i >= w as f32 || j >= h as f32 {
            return Vec::new();
        }
        let mut out: Vec<usize> = Vec::new();
        for &(m, t) in &self.cells[j as usize * w + i as usize] {
            let m = m as usize;
            if !out.contains(&m) && point_in_triangle(x, z, &self.meshes[m].triangles[t as usize]) {
                out.push(m);
            }
        }
        out
    }

    /// Distance from `(x, z)` to the nearest loaded zone that lists `model` (cached per grid cell).
    fn listing_distance(&self, model: ResourceId, x: f32, z: f32) -> f32 {
        let (i, j) = (((x - self.origin[0]) / CELL).floor() as i32, ((z - self.origin[1]) / CELL).floor() as i32);
        let mut cache = self.distances.lock().unwrap();
        *cache.entry((model, i, j)).or_insert_with(|| {
            let (cx, cz) = (self.origin[0] + (i as f32 + 0.5) * CELL, self.origin[1] + (j as f32 + 0.5) * CELL);
            self.listed
                .get(&model)
                .into_iter()
                .flatten()
                .filter(|&&zone| self.loaded[zone])
                .map(|&zone| self.zones.zones[zone].distance(cx, cz))
                .fold(f32::INFINITY, f32::min)
        })
    }

    /// The nearest loaded zone within `range` of `(x, z)` and its distance.
    fn nearest_loaded_zone(&self, x: f32, z: f32, range: f32) -> Option<(usize, f32)> {
        self.zones
            .zones
            .iter()
            .enumerate()
            .filter(|&(i, zone)| {
                let [x0, z0, x1, z1] = zone.bounds;
                self.loaded[i] && x > x0 - range && x < x1 + range && z > z0 - range && z < z1 + range
            })
            .map(|(i, zone)| (i, zone.distance(x, z)))
            .filter(|&(_, d)| d < range)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Whether mesh `mesh_index` of backdrop `model` is drawn at ground position `(x, z)`.
    pub fn keeps(&self, model: ResourceId, mesh_index: usize, x: f32, z: f32) -> bool {
        let Some(listing) = self.listed.get(&model) else { return false };
        let this = self.mesh_of.get(&(model, mesh_index)).copied();
        // Inside a loaded zone exactly what that zone lists is drawn, water included.
        let zone = self.zones.zones.iter().position(|zone| zone.contains(x, z));
        if let Some(zone) = zone
            && self.loaded[zone]
        {
            return listing.contains(&zone);
        }
        // Elsewhere water surfaces and solid geometry are composed separately: a lake bed listed by a near
        // zone does not hide the water plane a farther zone lists over it.
        let water = this.is_some_and(|m| self.meshes[m].water);
        if water
            && zone.is_none()
            && let Some((owner, _)) = self.nearest_loaded_zone(x, z, WATER_GAP)
            && self.lists_water[owner]
        {
            return listing.contains(&owner);
        }
        let mut cands: Vec<usize> =
            self.covering(x, z).into_iter().filter(|&m| self.meshes[m].water == water).collect();
        if let Some(m) = this
            && !cands.contains(&m)
        {
            cands.push(m);
        }
        // A model listed by most zones (the whole-map horizon model in GLOBALRESOURCES, 144 of 169) is
        // always nearest; it only fills in where no regional model covers the spot.
        let global = |m: ResourceId| self.listed.get(&m).is_some_and(|l| 2 * l.len() > self.zones.zones.len());
        let nearest_listing = || {
            let regional: Vec<usize> = cands.iter().copied().filter(|&m| !global(self.meshes[m].model)).collect();
            if global(model) {
                return regional.is_empty();
            }
            let mine = self.listing_distance(model, x, z);
            if mine.is_infinite() {
                return false;
            }
            let best = regional
                .iter()
                .map(|&m| self.listing_distance(self.meshes[m].model, x, z))
                .fold(f32::INFINITY, f32::min);
            mine <= best + TIE
        };
        // Outside the zones, and in a zone that is not streamed (seen from outside), the nearest listing wins.
        nearest_listing()
    }

    /// The part of mesh `mesh_index` of backdrop `model` that [`Self::keeps`] at each triangle's centroid,
    /// after splitting long triangles. `transform` places the mesh in the world (row-major, `v * M`).
    /// Returns `None` when nothing is kept, an unchanged copy when everything is.
    pub fn filter_mesh(
        &self,
        model: ResourceId,
        mesh_index: usize,
        mesh: &fmt::DecodedMesh,
        transform: &[[f32; 4]; 4],
    ) -> Option<fmt::DecodedMesh> {
        let mut b = MeshBuilder::new(mesh);
        let mut keep_all = true;
        let mut kept = Vec::with_capacity(mesh.indices.len());
        let keeps = |x: f32, z: f32| self.keeps(model, mesh_index, x, z);
        let region = [
            self.origin[0],
            self.origin[1],
            self.origin[0] + self.size[0] as f32 * CELL,
            self.origin[1] + self.size[1] as f32 * CELL,
        ];
        for t in mesh.indices.as_chunks::<3>().0 {
            if t.iter().any(|&i| i as usize >= mesh.positions.len()) {
                keep_all = false;
                continue;
            }
            let before = kept.len();
            split(&keeps, region, &mut b, *t, 0, transform, &mut kept);
            keep_all &= kept.len() - before == 3 && kept[before..] == t[..];
        }
        if keep_all {
            return Some(mesh.clone());
        }
        if kept.is_empty() {
            return None;
        }
        Some(b.finish(&kept))
    }
}

fn apply(m: &[[f32; 4]; 4], p: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
}

/// Point-in-triangle test on the ground plane, edges inclusive.
fn point_in_triangle(x: f32, z: f32, t: &[f32; 6]) -> bool {
    let d = |ax: f32, az: f32, bx: f32, bz: f32| (bx - ax) * (z - az) - (bz - az) * (x - ax);
    let (d1, d2, d3) = (d(t[0], t[1], t[2], t[3]), d(t[2], t[3], t[4], t[5]), d(t[4], t[5], t[0], t[1]));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Keep (sub-)triangle `t` where `keeps` says so at its centroid, splitting long triangles first.
/// `region` is the area where decisions can vary (`[min_x, min_z, max_x, max_z]`); beyond it a
/// triangle is decided as a whole.
fn split(
    keeps: &dyn Fn(f32, f32) -> bool,
    region: [f32; 4],
    b: &mut MeshBuilder,
    t: [u32; 3],
    depth: u32,
    transform: &[[f32; 4]; 4],
    out: &mut Vec<u32>,
) {
    let p = t.map(|i| apply(transform, b.position(i)));
    let edge = |a: [f32; 3], c: [f32; 3]| ((a[0] - c[0]).powi(2) + (a[2] - c[2]).powi(2)).sqrt();
    let longest = edge(p[0], p[1]).max(edge(p[1], p[2])).max(edge(p[2], p[0]));
    let centre = |i: usize| (p[0][i] + p[1][i] + p[2][i]) / 3.0;
    let outside = p.iter().all(|v| v[0] < region[0])
        || p.iter().all(|v| v[0] > region[2])
        || p.iter().all(|v| v[2] < region[1])
        || p.iter().all(|v| v[2] > region[3]);
    let decide = |out: &mut Vec<u32>, keep: bool| {
        if keep {
            out.extend_from_slice(&t);
        }
    };
    if longest <= MAX_EDGE || depth >= MAX_DEPTH || outside {
        return decide(out, keeps(centre(0), centre(2)));
    }
    // Below SPLIT_ALWAYS a triangle is decided as a whole when its corners and centre agree, otherwise
    // it is split into four at the edge midpoints.
    if longest <= SPLIT_ALWAYS {
        let votes =
            [keeps(centre(0), centre(2)), keeps(p[0][0], p[0][2]), keeps(p[1][0], p[1][2]), keeps(p[2][0], p[2][2])];
        if votes.iter().all(|&v| v == votes[0]) {
            return decide(out, votes[0]);
        }
    }
    let [a, c, d] = t;
    let (ac, cd, da) = (b.midpoint(a, c), b.midpoint(c, d), b.midpoint(d, a));
    for sub in [[a, ac, da], [ac, c, cd], [da, cd, d], [ac, cd, da]] {
        split(keeps, region, b, sub, depth + 1, transform, out);
    }
}

/// A copy of a [`fmt::DecodedMesh`] that can grow edge-midpoint vertices.
struct MeshBuilder {
    mesh: fmt::DecodedMesh,
    midpoints: HashMap<(u32, u32), u32>,
}

impl MeshBuilder {
    fn new(mesh: &fmt::DecodedMesh) -> Self {
        Self { mesh: mesh.clone(), midpoints: HashMap::new() }
    }

    fn position(&self, i: u32) -> [f32; 4] {
        self.mesh.positions[i as usize]
    }

    fn midpoint(&mut self, a: u32, b: u32) -> u32 {
        let key = (a.min(b), a.max(b));
        if let Some(&m) = self.midpoints.get(&key) {
            return m;
        }
        let (a, b) = (a as usize, b as usize);
        let mix = |v: &[[f32; 4]]| -> [f32; 4] { std::array::from_fn(|c| (v[a][c] + v[b][c]) * 0.5) };
        let unit = |v: [f32; 4]| -> [f32; 4] {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l, v[3]] } else { v }
        };
        let m = &mut self.mesh;
        let index = m.positions.len() as u32;
        let p = mix(&m.positions);
        m.positions.push(p);
        for v in [&mut m.normals, &mut m.tangents, &mut m.binormals].into_iter().flatten() {
            // Keep the sign stored in w (bitangent sign) from the first corner.
            let x = mix(v);
            let x = unit([x[0], x[1], x[2], v[a][3]]);
            v.push(x);
        }
        for v in m.colors.iter_mut().chain(m.texcoords.iter_mut().flatten()) {
            let x = mix(v);
            v.push(x);
        }
        self.midpoints.insert(key, index);
        index
    }

    /// The mesh reduced to the given triangles, with unused vertices dropped.
    fn finish(self, triangles: &[u32]) -> fmt::DecodedMesh {
        let src = self.mesh;
        let mut remap = vec![u32::MAX; src.positions.len()];
        let mut order = Vec::new();
        let indices = triangles
            .iter()
            .map(|&i| {
                let slot = &mut remap[i as usize];
                if *slot == u32::MAX {
                    *slot = order.len() as u32;
                    order.push(i as usize);
                }
                *slot
            })
            .collect();
        let pick = |v: &Vec<[f32; 4]>| order.iter().map(|&i| v[i]).collect::<Vec<_>>();
        fmt::DecodedMesh {
            positions: pick(&src.positions),
            normals: src.normals.as_ref().map(pick),
            tangents: src.tangents.as_ref().map(pick),
            binormals: src.binormals.as_ref().map(pick),
            colors: src.colors.as_ref().map(pick),
            texcoords: src.texcoords.iter().map(|t| t.as_ref().map(pick)).collect(),
            indices,
            // Backdrop geometry is static world scenery: never skinned.
            joints: None,
            weights: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(id: u32, x0: f32, x1: f32) -> fmt::Zone {
        fmt::Zone {
            id,
            district: 0,
            arena: 0,
            flag: 0,
            points: vec![[x0, 0.0], [x1, 0.0], [x1, 100.0], [x0, 100.0]],
            bounds: [x0, 0.0, x1, 100.0],
            neighbours: vec![],
        }
    }

    /// A flat strip from x0 to x1 (z 10..90), two triangles.
    fn strip(x0: f32, x1: f32) -> fmt::DecodedMesh {
        fmt::DecodedMesh {
            positions: vec![[x0, 0.0, 10.0, 1.0], [x1, 0.0, 10.0, 1.0], [x1, 0.0, 90.0, 1.0], [x0, 0.0, 90.0, 1.0]],
            normals: Some(vec![[0.0, 1.0, 0.0, 0.0]; 4]),
            tangents: None,
            binormals: None,
            colors: None,
            texcoords: vec![Some(vec![
                [0.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
            ])],
            indices: vec![0, 1, 2, 0, 2, 3],
            joints: None,
            weights: None,
        }
    }

    const FINE_A: ResourceId = ResourceId(0xA);
    const FINE_B: ResourceId = ResourceId(0xB);
    const COARSE: ResourceId = ResourceId(0xC);
    const WATER: ResourceId = ResourceId(0xD);
    const FAR: ResourceId = ResourceId(0xE);
    const COARSE2: ResourceId = ResourceId(0xF);

    /// Zones 1 (x 0..100), 2 (x 100..200) and 3 (x 1000..1100). Zones 1 and 2 list their own fine tile
    /// over both; zone 3 lists a coarse tile over both, a water plane over everything and a coarse tile far
    /// out, where zone 1 lists a finer one.
    fn selector() -> BackdropSelector {
        let zones = vec![zone(1, 0.0, 100.0), zone(2, 100.0, 200.0), zone(3, 1000.0, 1100.0)];
        let mut s = BackdropSelector::new(fmt::ZoneList { zones });
        let id = crate::IDENTITY;
        for (unit, model, mesh, water) in [
            (1, FINE_A, strip(0.0, 200.0), false),
            (2, FINE_B, strip(0.0, 200.0), false),
            (3, COARSE, strip(0.0, 200.0), false),
            (3, WATER, strip(-500.0, 500.0), true),
            (1, FAR, strip(-400.0, -200.0), false),
            (3, COARSE2, strip(-450.0, -150.0), false),
        ] {
            s.add_listing(unit, model);
            s.add_mesh(model, 0, water, &mesh, &id);
        }
        s.build_index();
        s
    }

    #[test]
    fn inside_a_zone_its_own_listing_wins() {
        let s = selector();
        assert!(s.keeps(FINE_A, 0, 50.0, 50.0));
        assert!(!s.keeps(FINE_A, 0, 150.0, 50.0));
        assert!(s.keeps(FINE_B, 0, 150.0, 50.0));
        assert!(!s.keeps(COARSE, 0, 50.0, 50.0));
        // Zone 1 lists no water: a farther zone's water plane does not flood it.
        assert!(!s.keeps(WATER, 0, 50.0, 50.0));
    }

    #[test]
    fn outside_the_zones_the_nearest_listing_wins() {
        let s = selector();
        // At x = -300 FAR is listed by zone 1, COARSE2 by zone 3: zone 1 is nearer, so the
        // tier it uses wins.
        assert!(s.keeps(FAR, 0, -300.0, 50.0));
        assert!(!s.keeps(COARSE2, 0, -300.0, 50.0));
        // Water is composed on its own: the only water plane there stays.
        assert!(s.keeps(WATER, 0, -300.0, 50.0));
        assert!(s.keeps(WATER, 0, -100.0, 50.0));
    }

    #[test]
    fn water_in_a_gap_between_zones_belongs_to_the_nearest_zone() {
        const RIVER: ResourceId = ResourceId(0x10);
        let zones = vec![zone(1, 0.0, 100.0), zone(2, 100.0, 200.0), zone(3, 1000.0, 1100.0)];
        let mut s = BackdropSelector::new(fmt::ZoneList { zones });
        let id = crate::IDENTITY;
        for (unit, model, mesh) in [(3, WATER, strip(-500.0, 500.0)), (2, RIVER, strip(150.0, 260.0))] {
            s.add_listing(unit, model);
            s.add_mesh(model, 0, true, &mesh, &id);
        }
        s.build_index();
        // 30 m past zone 2, which lists water of its own: only that water is drawn there.
        assert!(s.keeps(RIVER, 0, 230.0, 50.0));
        assert!(!s.keeps(WATER, 0, 230.0, 50.0));
        // Zone 2 itself lists only its river: the far plane does not flood it.
        assert!(!s.keeps(WATER, 0, 150.0, 50.0));
        // Next to zone 1 (no water listed) and far from every zone, the nearest listing still fills in.
        assert!(s.keeps(WATER, 0, -30.0, 50.0));
        assert!(s.keeps(WATER, 0, 400.0, 50.0));
    }

    #[test]
    fn unloaded_zones_are_seen_from_the_nearest_loaded_one() {
        let mut s = selector();
        s.set_loaded([1, 3]);
        // Zone 2 is not streamed: its own fine tile no longer counts, zone 1's view fills it.
        assert!(!s.keeps(FINE_B, 0, 150.0, 50.0));
        assert!(s.keeps(FINE_A, 0, 150.0, 50.0));
        assert!(!s.keeps(COARSE, 0, 150.0, 50.0));
    }

    #[test]
    fn mesh_is_cut_at_the_zone_border() {
        let s = selector();
        let a = s.filter_mesh(FINE_A, 0, &strip(0.0, 200.0), &crate::IDENTITY).expect("part of it is kept");
        assert!(a.indices.iter().all(|&i| a.positions[i as usize][0] <= 100.0 + 1e-3));
        assert_eq!(a.texcoords[0].as_ref().unwrap().len(), a.positions.len());
        let b = s.filter_mesh(FINE_B, 0, &strip(0.0, 200.0), &crate::IDENTITY).unwrap();
        assert!(b.indices.iter().all(|&i| b.positions[i as usize][0] >= 100.0 - 1e-3));
        assert!(s.filter_mesh(COARSE, 0, &strip(0.0, 200.0), &crate::IDENTITY).is_none());
        assert!(s.filter_mesh(ResourceId(0x99), 0, &strip(0.0, 200.0), &crate::IDENTITY).is_none());
    }

    #[test]
    fn unit_numbers() {
        assert_eq!(track_unit_number("HAWAII/TRK_UNIT1343.BNDL"), Some(1343));
        assert_eq!(track_unit_number("HAWAII/trk_unit7.bndl"), Some(7));
        assert_eq!(track_unit_number("HAWAII/PVS.BNDL"), None);
    }
}
