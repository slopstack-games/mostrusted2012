//! Repair for cars whose wheel data doesn't match their body.
//!
//! One car ships with front wheels that sit nowhere near its front arches: the Hennessey Venom GT Spyder
//! (Ultimate Speed Pack DLC) has its front wheels at z = 0.125 in every source (VehicleGraphicsSpec, the
//! physics RigidBodies, SuspensionWheel, HI and LO bundles), a 1.63 m wheelbase on a 4.66 m car, while its
//! body's front arches are centred at z ≈ 1.3. Every other car's overhangs are 0.46–1.36 m (survey in the
//! research log, 2026-10-04 "wheel layout").
//!
//! The arches are found from the body mesh: along the car, the lowest point of the outer body surface jumps
//! up over each wheel (the arch). An axle whose overhang is implausible is moved to the arch nearest to
//! where a normal overhang would put it. Cars with plausible data are never touched.

use crate::SceneMesh;

/// Overhangs beyond this (metres from the body's end to the axle) are treated as broken data.
const MAX_OVERHANG: f32 = 1.6;
/// Expected overhang when searching for the real arch.
const TYPICAL_OVERHANG: f32 = 0.95;
const BIN: f32 = 0.05;
/// An arch raises the outer body's lowest point by at least this much over the sills.
const ARCH_RISE: f32 = 0.25;
/// Arches are about a wheel diameter long.
const MIN_ARCH_LENGTH: f32 = 0.4;

/// Centres (z) of the wheel arches in a body mesh, front to back.
pub fn arch_centres(meshes: &[SceneMesh]) -> Vec<f32> {
    let points = || meshes.iter().flat_map(|m| m.mesh.positions.iter());
    let half = points().map(|p| p[0].abs()).fold(0.0f32, f32::max);
    let (zmin, zmax) = points().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[2]), b.max(p[2])));
    if half <= 0.0 || zmax <= zmin {
        return Vec::new();
    }
    let n = ((zmax - zmin) / BIN) as usize + 1;
    let mut low = vec![f32::NAN; n];
    for p in points().filter(|p| p[0].abs() > 0.75 * half) {
        let i = (((p[2] - zmin) / BIN) as usize).min(n - 1);
        low[i] = if low[i].is_nan() { p[1] } else { low[i].min(p[1]) };
    }
    let mut sorted: Vec<f32> = low.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return Vec::new();
    }
    sorted.sort_by(f32::total_cmp);
    let sill = sorted[sorted.len() / 4];
    // Runs of raised bins, bridging gaps of up to two bins (vents and trim inside an arch).
    let raised: Vec<bool> = low.iter().map(|&v| !v.is_nan() && v > sill + ARCH_RISE).collect();
    let mut arches = Vec::new();
    let mut i = 0;
    while i < n {
        if !raised[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut j = i + 1;
        while j < n && j <= end + 3 {
            if raised[j] {
                end = j;
            }
            j += 1;
        }
        if (end - start + 1) as f32 * BIN >= MIN_ARCH_LENGTH {
            arches.push(zmin + (start + end + 1) as f32 * 0.5 * BIN);
        }
        i = end + 1;
    }
    arches.sort_by(|a, b| b.total_cmp(a));
    arches
}

/// Corrected z for the front and rear axles, if either is implausible for this body: `(front, rear)`, each
/// `None` when the data is kept. `front_z` / `rear_z` are the data's axle positions (+z is forward).
pub fn axle_corrections(meshes: &[SceneMesh], front_z: f32, rear_z: f32) -> (Option<f32>, Option<f32>) {
    let (zmin, zmax) = meshes
        .iter()
        .flat_map(|m| m.mesh.positions.iter())
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[2]), b.max(p[2])));
    let front_bad = zmax - front_z > MAX_OVERHANG || front_z > zmax;
    let rear_bad = rear_z - zmin > MAX_OVERHANG || rear_z < zmin;
    if !front_bad && !rear_bad {
        return (None, None);
    }
    let arches = arch_centres(meshes);
    let nearest = |want: f32| arches.iter().copied().min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()));
    let fix = |bad: bool, want: f32| {
        if !bad {
            return None;
        }
        // Only accept an arch close to where a normal overhang puts the axle.
        nearest(want).filter(|z| (z - want).abs() < 0.6)
    };
    (fix(front_bad, zmax - TYPICAL_OVERHANG), fix(rear_bad, zmin + TYPICAL_OVERHANG))
}

impl crate::AssetStore {
    /// Move a car's axles onto its body's arches where the data is implausible (see the module docs).
    /// Returns the moves as `(old z, new z)`, empty for cars with sane data.
    pub fn repair_wheel_layout(&self, spec: &mut mw_formats::VehicleGraphicsSpec) -> Vec<(f32, f32)> {
        let Some(body) = spec.parts.iter().flatten().next() else { return Vec::new() };
        let Ok(model) = self.model(*body) else { return Vec::new() };
        let Some(Some(rid)) = model.renderables.first() else { return Vec::new() };
        let Ok((meshes, _)) = self.renderable_meshes(*rid) else { return Vec::new() };
        let zs: Vec<f32> = spec.wheels.iter().map(|w| w.position[2]).collect();
        if zs.is_empty() {
            return Vec::new();
        }
        // Axles by position (some specs swap the Front / Rear labels).
        let mid = zs.iter().sum::<f32>() / zs.len() as f32;
        let front = zs.iter().copied().filter(|z| *z > mid).fold(f32::MIN, f32::max);
        let rear = zs.iter().copied().filter(|z| *z <= mid).fold(f32::MAX, f32::min);
        if front == f32::MIN || rear == f32::MAX {
            return Vec::new();
        }
        let (fix_front, fix_rear) = axle_corrections(&meshes, front, rear);
        let mut moves = Vec::new();
        for (old, new) in [(front, fix_front), (rear, fix_rear)] {
            let Some(new) = new else { continue };
            for w in spec.wheels.iter_mut().filter(|w| (w.position[2] - old).abs() < 0.05) {
                w.position[2] += new - old;
            }
            moves.push((old, new));
        }
        moves
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box body 4.6 m long with raised outer-edge points over two "arches" at z = 1.3 and -1.5.
    fn body() -> Vec<SceneMesh> {
        let mut positions = Vec::new();
        let mut z = -2.3;
        while z <= 2.3 {
            let over_arch = (z - 1.3f32).abs() < 0.3 || (z + 1.5f32).abs() < 0.3;
            let low = if over_arch { 0.65 } else { 0.12 };
            for x in [-0.95, 0.95] {
                positions.push([x, low, z, 1.0]);
                positions.push([x, 1.1, z, 1.0]);
            }
            positions.push([0.0, 0.1, z, 1.0]);
            z += 0.02;
        }
        let mesh = mw_formats::DecodedMesh {
            positions,
            normals: None,
            tangents: None,
            binormals: None,
            colors: None,
            texcoords: Vec::new(),
            indices: Vec::new(),
            joints: None,
            weights: None,
        };
        vec![SceneMesh {
            renderable: mw_bundle::ResourceId(0),
            mesh_index: 0,
            material: None,
            shader_name: String::new(),
            diffuse: None,
            normal_map: None,
            textures: Vec::new(),
            sampler_states: Vec::new(),
            parameters: Vec::new(),
            mesh,
        }]
    }

    #[test]
    fn finds_arches() {
        let a = arch_centres(&body());
        assert_eq!(a.len(), 2, "{a:?}");
        assert!((a[0] - 1.3).abs() < 0.06 && (a[1] + 1.5).abs() < 0.06, "{a:?}");
    }

    #[test]
    fn repairs_only_broken_axles() {
        // The Venom's data: front axle at 0.125 (2.2 m overhang), rear fine.
        assert_eq!(axle_corrections(&body(), 0.125, -1.505).1, None);
        let front = axle_corrections(&body(), 0.125, -1.505).0.unwrap();
        assert!((front - 1.3).abs() < 0.06, "{front}");
        // Plausible data is never touched.
        assert_eq!(axle_corrections(&body(), 1.25, -1.45), (None, None));
    }
}
