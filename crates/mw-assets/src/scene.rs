//! High-level helpers that turn Models into renderer-ready meshes.

use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats as fmt;

use crate::{AssetStore, Error, Result};

/// One drawable mesh of a model with its material bindings resolved.
#[derive(Debug, Clone)]
pub struct SceneMesh {
    pub renderable: ResourceId,
    pub mesh_index: usize,
    pub material: Option<ResourceId>,
    pub shader_name: String,
    /// Texture bound to the shader's diffuse/albedo sampler, if any.
    pub diffuse: Option<ResourceId>,
    pub normal_map: Option<ResourceId>,
    /// Bound textures by shader sampler name, e.g. `MaskTextureSampler`.
    pub textures: Vec<(String, ResourceId)>,
    /// Material constants by shader parameter name (material value, else the
    /// shader default), e.g. `PbrMaterialDiffuseColour`.
    pub parameters: Vec<(String, [f32; 4])>,
    pub mesh: fmt::DecodedMesh,
}

impl SceneMesh {
    pub fn parameter(&self, name: &str) -> Option<[f32; 4]> {
        self.parameters.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    pub fn texture(&self, sampler: &str) -> Option<ResourceId> {
        self.textures.iter().find(|(n, _)| n == sampler).map(|(_, t)| *t)
    }
}

/// Sampler names that carry base colour, in priority order. Terrain and
/// roads blend several layers in their shaders; until those are
/// reimplemented, the dominant layer stands in for the whole material.
const DIFFUSE_SAMPLERS: &[&str] = &[
    "DiffuseTextureSampler",
    "DiffuseSampler",
    "DiffuseTexture1Sampler",
    "Decal_Diffuse_Sampler",
    "Line_Diffuse_Sampler",
    "DetailMap_Diffuse_Sampler",
    "Tiling1TextureSampler",
    "DecalTextureSampler",
    "ColourMap_Sampler",
];
const NORMAL_SAMPLERS: &[&str] = &["NormalTextureSampler", "NormalTextureSampler1"];

/// A placed model (e.g. from an `InstanceList`).
#[derive(Debug, Clone)]
pub struct Placement {
    pub model: ResourceId,
    pub transform: [[f32; 4]; 4],
    /// InstanceList entries past the "always loaded" count are backdrop.
    pub backdrop: bool,
}

impl AssetStore {
    /// Decode every mesh of a model at the given LOD. Meshes that fail to
    /// decode are skipped and reported in the second return value.
    pub fn model_meshes(&self, model: ResourceId, lod: usize) -> Result<(Vec<SceneMesh>, Vec<String>)> {
        let model = self.model(model)?;
        let rid = model.renderable_for_lod(lod).ok_or(Error::Missing(model.id))?;
        self.renderable_meshes(rid)
    }

    pub fn renderable_meshes(&self, rid: ResourceId) -> Result<(Vec<SceneMesh>, Vec<String>)> {
        let rend = self.renderable(rid)?;
        let mut out = Vec::new();
        let mut errors = Vec::new();
        for (i, mesh) in rend.meshes.iter().enumerate() {
            match self.scene_mesh(&rend, i, mesh) {
                Ok(m) => out.push(m),
                Err(e) => errors.push(format!("renderable {rid} mesh {i}: {e}")),
            }
        }
        Ok((out, errors))
    }

    fn scene_mesh(&self, rend: &fmt::Renderable, index: usize, mesh: &fmt::Mesh) -> Result<SceneMesh> {
        let mat_id = mesh.material.ok_or(Error::Missing(rend.id))?;
        let material = self.material(mat_id)?;
        let (shader, layout) = self.layout_for_material(&material)?;
        let pick = |names: &[&str]| {
            names.iter().find_map(|n| {
                let key = shader.sampler_named(n)?.key;
                material.samplers.iter().find(|s| s.key == key)?.texture
            })
        };
        let parameters = shader
            .parameters
            .iter()
            .filter_map(|p| {
                let own = material.parameters.iter().find(|m| m.name_hash == p.hash).and_then(|m| m.value);
                Some((p.name.clone(), own.or(p.default)?))
            })
            .collect();
        let textures = shader
            .samplers
            .iter()
            .filter_map(|ss| Some((ss.name.clone(), material.samplers.iter().find(|m| m.key == ss.key)?.texture?)))
            .collect();
        Ok(SceneMesh {
            textures,
            parameters,
            renderable: rend.id,
            mesh_index: index,
            material: Some(mat_id),
            diffuse: pick(DIFFUSE_SAMPLERS),
            normal_map: pick(NORMAL_SAMPLERS),
            shader_name: shader.name,
            mesh: rend.decode_mesh(mesh, &layout)?,
        })
    }

    /// All model placements from every `InstanceList` in a bundle.
    pub fn bundle_placements(&self, bundle: &Bundle) -> Result<Vec<Placement>> {
        let mut out = Vec::new();
        for e in bundle.entries_of_type(ResourceType::InstanceList) {
            let list = fmt::InstanceList::parse(&bundle.load(e)?)?;
            out.extend(list.instances.into_iter().filter_map(|i| {
                Some(Placement { model: i.model?, transform: i.transform, backdrop: !i.always_loaded })
            }));
        }
        Ok(out)
    }
}

/// Row-major D3D matrix (`v * M`) from translation, quaternion (x, y, z, w)
/// and scale.
pub fn trs_matrix(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> [[f32; 4]; 4] {
    let [x, y, z, w] = q;
    // Images of the basis vectors under the rotation.
    let axes = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
        [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
        [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let row = |i: usize| [axes[i][0] * s[i], axes[i][1] * s[i], axes[i][2] * s[i], 0.0];
    [row(0), row(1), row(2), [t[0], t[1], t[2], 1.0]]
}

pub const IDENTITY: [[f32; 4]; 4] =
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

impl AssetStore {
    /// Assemble a vehicle from the `VehicleGraphicsSpec` in its bundle: body
    /// parts at the origin and every wheel part at its wheel's transform.
    /// Returns `None` if the bundle has no spec.
    pub fn vehicle_placements(&self, bundle: &Bundle) -> Result<Option<Vec<Placement>>> {
        let Some(entry) = bundle.entries_of_type(ResourceType::VehicleGraphicsSpec).next() else {
            return Ok(None);
        };
        let spec = fmt::VehicleGraphicsSpec::parse(&bundle.load(entry)?)?;
        let mut out: Vec<Placement> = spec
            .parts
            .iter()
            .flatten()
            .map(|&model| Placement { model, transform: IDENTITY, backdrop: false })
            .collect();
        for wheel in &spec.wheels {
            // Wheel models are authored for the left side (+X); right wheels
            // are the same models mirrored across the car's centre line.
            // Mirror by position (some specs label sides the other way round).
            let mut scale = wheel.scale;
            if wheel.position[0] < 0.0 {
                scale[0] = -scale[0];
            }
            let transform = trs_matrix(wheel.position, wheel.rotation, scale);
            out.extend(
                wheel.parts.iter().filter_map(|p| Some(Placement { model: p.model?, transform, backdrop: false })),
            );
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|c| p[0] * m[0][c] + p[1] * m[1][c] + p[2] * m[2][c] + m[3][c])
    }

    #[test]
    fn trs_identity_rotation_translates_and_scales() {
        let m = trs_matrix([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [-1.0, 1.0, 1.0]);
        assert_eq!(apply(&m, [0.5, 0.0, 0.0]), [0.5, 2.0, 3.0]);
    }

    #[test]
    fn trs_rotates_x_to_minus_z_about_y() {
        // 90° about +Y: x axis maps to -z.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let m = trs_matrix([0.0; 3], [0.0, h, 0.0, h], [1.0; 3]);
        let p = apply(&m, [1.0, 0.0, 0.0]);
        assert!((p[0]).abs() < 1e-6 && (p[2] + 1.0).abs() < 1e-6, "{p:?}");
    }
}
