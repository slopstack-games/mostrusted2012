//! High-level helpers that turn Models into renderer-ready meshes.

use std::collections::HashMap;

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

/// Shaders that are not meant to be drawn as plain geometry (shadow casters, occluders, collision).
pub fn is_hidden_shader(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["shadow", "occlusion", "occluder", "invisible", "collision"].iter().any(|k| name.contains(k))
}

/// A placed model (e.g. from an `InstanceList`).
#[derive(Debug, Clone)]
pub struct Placement {
    pub model: ResourceId,
    pub transform: [[f32; 4]; 4],
    /// InstanceList entries past the "always loaded" count are backdrop.
    pub backdrop: bool,
    /// Per-instance material overrides (`InstanceList` only): tint constants, AO maps, lightmaps.
    pub overrides: Option<std::sync::Arc<fmt::InstanceOverrides>>,
    /// Prop / compound / dynamic placements: the object definition (physics, `crate::props`).
    pub object: Option<(ResourceId, crate::props::ObjectKind)>,
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

    /// All model placements in a bundle: `InstanceList` entries plus props,
    /// compound objects and dynamic objects (see [`WorldLayers`]).
    pub fn bundle_placements(&self, bundle: &Bundle) -> Result<Vec<Placement>> {
        Ok(self.bundle_layers(bundle)?.into_placements())
    }

    /// Placements of every drawable world layer in a bundle, kept apart.
    /// Objects whose definition or model cannot be resolved are skipped and
    /// counted in `unresolved`.
    pub fn bundle_layers(&self, bundle: &Bundle) -> Result<WorldLayers> {
        let mut out = WorldLayers::default();
        for e in bundle.entries_of_type(ResourceType::InstanceList) {
            let list = fmt::InstanceList::parse(&bundle.load(e)?)?;
            out.instances.extend(list.instances.into_iter().filter_map(|i| {
                let overrides = (!i.overrides.constants.is_empty() || !i.overrides.textures.is_empty())
                    .then(|| std::sync::Arc::new(i.overrides));
                Some(Placement {
                    model: i.model?,
                    transform: i.transform,
                    backdrop: !i.always_loaded,
                    overrides,
                    object: None,
                })
            }));
        }
        type ParseDef = fn(&mw_bundle::Resource) -> fmt::Result<fmt::ObjectDef>;
        let mut defs: HashMap<ResourceId, Option<ResourceId>> = HashMap::new();
        let mut unresolved = 0;
        use crate::props::ObjectKind;
        let layers: [(ResourceType, ParseDef, ObjectKind, &mut Vec<Placement>); 3] = [
            (ResourceType::PropInstanceList, fmt::ObjectDef::parse_prop, ObjectKind::Prop, &mut out.props),
            (
                ResourceType::CompoundInstanceList,
                fmt::ObjectDef::parse_compound,
                ObjectKind::Compound,
                &mut out.compounds,
            ),
            (
                ResourceType::DynamicInstanceList,
                fmt::ObjectDef::parse_world_object,
                ObjectKind::Dynamic,
                &mut out.dynamics,
            ),
        ];
        for (ty, parse_def, kind, dest) in layers {
            for e in bundle.entries_of_type(ty) {
                let list = fmt::ObjectInstanceList::parse(&bundle.load(e)?)?;
                for inst in list.instances {
                    let Some(obj) = inst.object else { continue };
                    let model = *defs.entry(obj).or_insert_with(|| {
                        let res = self.load(obj).ok()?;
                        parse_def(&res).ok()?.model()
                    });
                    match model {
                        Some(model) => dest.push(Placement {
                            model,
                            transform: inst.transform,
                            backdrop: false,
                            overrides: None,
                            object: Some((obj, kind)),
                        }),
                        None => unresolved += 1,
                    }
                }
            }
        }
        out.unresolved = unresolved;
        for e in bundle.entries_of_type(ResourceType::LightInstanceList) {
            out.lights += fmt::ObjectInstanceList::parse(&bundle.load(e)?)?.instances.len();
        }
        for e in bundle.entries_of_type(ResourceType::GroundcoverCollection) {
            let col = fmt::GroundcoverCollection::parse(&bundle.load(e)?)?;
            out.groundcover.push((col.id, col.layers.iter().map(|l| l.total).sum()));
        }
        Ok(out)
    }
}

/// A texture-atlas sprite class of a groundcover collection.
#[derive(Debug, Clone)]
pub struct GroundcoverMaterial {
    pub texture: Option<ResourceId>,
    /// `[offset.x, offset.y, scale.x, scale.y]` per sprite index.
    pub regions: Vec<[f32; 4]>,
}

/// All instances of one groundcover collection with its materials resolved.
#[derive(Debug, Clone)]
pub struct Groundcover {
    pub id: ResourceId,
    pub materials: Vec<GroundcoverMaterial>,
    pub instances: Vec<fmt::GroundcoverInstance>,
}

impl AssetStore {
    /// Load and decode a groundcover collection (all layers).
    pub fn groundcover(&self, id: ResourceId) -> Result<Groundcover> {
        let col = fmt::GroundcoverCollection::parse(&self.load(id)?)?;
        let blobs = col
            .blobs
            .iter()
            .map(|&b| Ok(fmt::GroundcoverInstances::parse(&self.load(b)?)?))
            .collect::<Result<Vec<_>>>()?;
        let mut instances = Vec::with_capacity(col.layers.iter().map(|l| l.total as usize).sum());
        for layer in 0..col.layers.len() {
            instances.extend(col.instances(layer, &blobs)?);
        }
        let materials = col
            .materials
            .iter()
            .map(|m| GroundcoverMaterial {
                texture: m
                    .material
                    .and_then(|id| self.material(id).ok())
                    .and_then(|mat| mat.samplers.iter().find_map(|s| s.texture)),
                regions: m.regions.clone(),
            })
            .collect();
        Ok(Groundcover { id, materials, instances })
    }
}

/// The world layers of one bundle that become model placements.
#[derive(Debug, Clone, Default)]
pub struct WorldLayers {
    /// `InstanceList`: buildings, terrain, roads.
    pub instances: Vec<Placement>,
    /// `PropInstanceList` via `PropObject`: street furniture.
    pub props: Vec<Placement>,
    /// `CompoundInstanceList` via `CompoundObject`.
    pub compounds: Vec<Placement>,
    /// `DynamicInstanceList` via `WorldObject`.
    pub dynamics: Vec<Placement>,
    /// `LightInstanceList` entries (parsed but not rendered).
    pub lights: usize,
    /// Groundcover collections the bundle references, with their instance counts. Collections are
    /// shared between bundles, so deduplicate by ID when adding up.
    pub groundcover: Vec<(ResourceId, u32)>,
    /// Prop/compound/dynamic entries whose object or model could not be found.
    pub unresolved: usize,
}

impl WorldLayers {
    pub fn into_placements(self) -> Vec<Placement> {
        let mut all = self.instances;
        all.extend(self.props);
        all.extend(self.compounds);
        all.extend(self.dynamics);
        all
    }

    pub fn add(&mut self, other: WorldLayers) {
        self.instances.extend(other.instances);
        self.props.extend(other.props);
        self.compounds.extend(other.compounds);
        self.dynamics.extend(other.dynamics);
        self.lights += other.lights;
        self.groundcover.extend(other.groundcover);
        self.unresolved += other.unresolved;
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
            .map(|&model| Placement { model, transform: IDENTITY, backdrop: false, overrides: None, object: None })
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
            out.extend(wheel.parts.iter().filter_map(|p| {
                Some(Placement { model: p.model?, transform, backdrop: false, overrides: None, object: None })
            }));
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
