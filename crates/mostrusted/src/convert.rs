//! Conversion of decoded game resources into Bevy assets.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use mw_formats::{DxgiFormat, Texture};

/// `uv_b` value that routes the vertex colour's xy into UV_1 instead of a TEXCOORD (Bevy would multiply
/// a COLOR attribute into the base colour; vehicles keep AO and specular occlusion there).
pub const UV_FROM_COLOUR: u8 = 0xC0;

/// Build a Bevy mesh from a decoded game mesh. Game TEXCOORD0 becomes UV_0
/// and `uv_b` (if any) selects the game TEXCOORD that becomes UV_1. Normals
/// are taken from the vertex data when present, otherwise computed. `colour` adds the vertex
/// colour as Bevy's COLOR attribute (the vehicle shaders read it themselves).
pub fn mesh(m: &mw_formats::DecodedMesh, uv_b: Option<u8>, colour: bool) -> Option<Mesh> {
    if m.indices.is_empty() {
        return None;
    }
    let positions: Vec<[f32; 3]> = m.positions.iter().map(|p| [p[0], p[1], p[2]]).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(m.indices.clone()));
    let uv2 = |uv: &[[f32; 4]]| uv.iter().map(|t| [t[0], t[1]]).collect::<Vec<_>>();
    if let Some(uv) = m.texcoord(0) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv2(uv));
    }
    let uv_b =
        if uv_b == Some(UV_FROM_COLOUR) { m.colors.as_deref() } else { uv_b.and_then(|i| m.texcoord(i as usize)) };
    if let Some(uv) = uv_b {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv2(uv));
    }
    if colour && let Some(c) = &m.colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c.clone());
    }
    match &m.normals {
        Some(n) => {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n.iter().map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>())
        }
        None => mesh.compute_smooth_normals(),
    }
    // Tangents feed the normal maps. Packed frames carry the bitangent sign in w; stored float
    // tangents have none: their sign comes from the stored binormal when there is one (vehicle lamp
    // interiors), else +1 (terrain).
    if let (Some(t), Some(n)) = (&m.tangents, &m.normals) {
        let t: Vec<[f32; 4]> = t
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let sign = match m.binormals.as_ref().and_then(|b| b.get(i)) {
                    Some(b) => {
                        let n = n[i];
                        let c = [n[1] * v[2] - n[2] * v[1], n[2] * v[0] - n[0] * v[2], n[0] * v[1] - n[1] * v[0]];
                        c[0] * b[0] + c[1] * b[1] + c[2] * b[2]
                    }
                    None => v[3],
                };
                [v[0], v[1], v[2], if sign < 0.0 { -1.0 } else { 1.0 }]
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, t);
    }
    Some(mesh)
}

/// Which optional attributes a mesh has (bit 0 UV_0, 1 UV_1, 2 COLOR, 3 TANGENT, 4 NORMAL). Only meshes
/// with the same set can be merged into one; `None` for anything [`merge`] does not handle (joints).
pub fn merge_signature(mesh: &Mesh) -> Option<u8> {
    use bevy::mesh::VertexAttributeValues as V;
    let mut sig = 0;
    for (id, values) in mesh.attributes() {
        let bit = match (id.id, values) {
            (i, V::Float32x3(_)) if i == Mesh::ATTRIBUTE_POSITION.id => continue,
            (i, V::Float32x2(_)) if i == Mesh::ATTRIBUTE_UV_0.id => 1,
            (i, V::Float32x2(_)) if i == Mesh::ATTRIBUTE_UV_1.id => 2,
            (i, V::Float32x4(_)) if i == Mesh::ATTRIBUTE_COLOR.id => 4,
            (i, V::Float32x4(_)) if i == Mesh::ATTRIBUTE_TANGENT.id => 8,
            (i, V::Float32x3(_)) if i == Mesh::ATTRIBUTE_NORMAL.id => 16,
            _ => return None,
        };
        sig |= bit;
    }
    matches!(mesh.indices(), Some(Indices::U32(_))).then_some(sig)
}

/// Merge meshes with the same [`merge_signature`] into one, each moved by its transform into the merged
/// mesh's space. This reproduces Bevy's mesh vertex stage: positions by the matrix, normals by its inverse
/// transpose (normalised), tangents by the matrix (normalised) with w times the sign of the determinant.
/// The winding is kept as it is under a mirroring transform too: Bevy does not flip the front face for a
/// negative determinant either (all world materials draw both faces; `double_sided` ones flip the normal
/// of back faces, so a mirrored double-sided mesh keeps the lighting it had unmerged).
pub fn merge(meshes: &[(&Mesh, bevy::math::Mat4)]) -> Mesh {
    use bevy::math::{Mat3, Vec3, Vec4Swizzles};
    use bevy::mesh::VertexAttributeValues as V;
    let n: usize = meshes.iter().map(|(m, _)| m.count_vertices()).sum();
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(n);
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    let mut tan: Vec<[f32; 4]> = Vec::new();
    let mut uv0: Vec<[f32; 2]> = Vec::new();
    let mut uv1: Vec<[f32; 2]> = Vec::new();
    let mut col: Vec<[f32; 4]> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for &(mesh, m) in meshes {
        let base = pos.len() as u32;
        let m3 = Mat3::from_mat4(m);
        let det = m3.determinant();
        let nm = m3.inverse().transpose();
        let sign = if det < 0.0 { -1.0 } else { 1.0 };
        if let Some(V::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            pos.extend(p.iter().map(|p| m.transform_point3(Vec3::from(*p)).to_array()));
        }
        if let Some(V::Float32x3(v)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            nrm.extend(v.iter().map(|v| (nm * Vec3::from(*v)).normalize_or_zero().to_array()));
        }
        if let Some(V::Float32x4(v)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            tan.extend(v.iter().map(|v| {
                let t = (m3 * bevy::math::Vec4::from(*v).xyz()).normalize_or_zero();
                [t.x, t.y, t.z, v[3] * sign]
            }));
        }
        if let Some(V::Float32x2(v)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            uv0.extend_from_slice(v);
        }
        if let Some(V::Float32x2(v)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) {
            uv1.extend_from_slice(v);
        }
        if let Some(V::Float32x4(v)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            col.extend_from_slice(v);
        }
        if let Some(Indices::U32(i)) = mesh.indices() {
            idx.extend(i.iter().map(|i| i + base));
        }
    }
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_indices(Indices::U32(idx));
    if !nrm.is_empty() {
        out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    }
    if !tan.is_empty() {
        out.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tan);
    }
    if !uv0.is_empty() {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    }
    if !uv1.is_empty() {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    }
    if !col.is_empty() {
        out.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    }
    out
}

/// Road meshes (`DriveableSurface*`): put the vertex wetness mask TEXCOORD5.x (saturated, as the vertex
/// shader does; 0 or ≥ 1 on real roads) and the vertex AO (colour alpha) in Bevy's COLOR attribute as
/// (wetness, AO, 0, 1). The road shader reads them itself; road materials have no other use for COLOR.
pub fn add_road_wetness(mesh: &mut Mesh, m: &mw_formats::DecodedMesh) {
    let n = m.positions.len();
    let wet = m.texcoord(5);
    let colour: Vec<[f32; 4]> = (0..n)
        .map(|i| {
            let w = wet.and_then(|t| t.get(i)).map_or(0.0, |t| t[0].clamp(0.0, 1.0));
            let ao = m.colors.as_ref().and_then(|c| c.get(i)).map_or(1.0, |c| c[3]);
            [w, ao, 0.0, 1.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colour);
}

/// Add the game's bone indices and weights as Bevy joint attributes (for `damage.rs`, which skins car
/// bodies to their damage skeleton). Weights on bones outside the skeleton are dropped. Only for meshes
/// that will get a `SkinnedMesh`: Bevy skins any mesh with these attributes.
pub fn add_skin(mesh: &mut Mesh, m: &mw_formats::DecodedMesh, bones: usize) {
    let (Some(joints), Some(weights)) = (&m.joints, &m.weights) else { return };
    let mut j_out = Vec::with_capacity(joints.len());
    let mut w_out = Vec::with_capacity(weights.len());
    for (j, w) in joints.iter().zip(weights) {
        let mut j = *j;
        let mut w = *w;
        for k in 0..4 {
            if j[k] as usize >= bones {
                (j[k], w[k]) = (0, 0.0);
            }
        }
        let sum: f32 = w.iter().sum();
        // A vertex left without weight follows the root.
        let w = if sum > 1e-4 { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0, 0.0] };
        j_out.push(j);
        w_out.push(w);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, bevy::mesh::VertexAttributeValues::Uint16x4(j_out));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, w_out);
}

/// Build a GPU image. Block-compressed textures are uploaded as-is when the
/// device supports BC formats; everything else is decoded to RGBA8.
pub fn image(tex: &Texture, srgb: bool, bc_supported: bool) -> Option<Image> {
    let block_aligned = tex.width.is_multiple_of(4) && tex.height.is_multiple_of(4);
    let bc = match tex.format {
        DxgiFormat::Bc1Unorm => Some(if srgb { TextureFormat::Bc1RgbaUnormSrgb } else { TextureFormat::Bc1RgbaUnorm }),
        DxgiFormat::Bc2Unorm => Some(if srgb { TextureFormat::Bc2RgbaUnormSrgb } else { TextureFormat::Bc2RgbaUnorm }),
        DxgiFormat::Bc3Unorm => Some(if srgb { TextureFormat::Bc3RgbaUnormSrgb } else { TextureFormat::Bc3RgbaUnorm }),
        _ => None,
    };
    let is_2d = tex.dimension == mw_formats::Dimension::D2;
    let mut image = match bc {
        Some(format) if bc_supported && block_aligned && is_2d => {
            let mut img = Image::new(
                Extent3d { width: tex.width, height: tex.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                tex.data[..tex.slice_size().min(tex.data.len())].to_vec(),
                format,
                RenderAssetUsages::RENDER_WORLD,
            );
            img.texture_descriptor.mip_level_count = tex.mip_count;
            img
        }
        _ => {
            let (w, h, rgba) = tex.decode_rgba8(0)?;
            Image::new(
                Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                TextureDimension::D2,
                rgba,
                if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
                RenderAssetUsages::RENDER_WORLD,
            )
        }
    };
    // One sampler for every slot, set to the game's most common anisotropic `SamplerState` (wrap, 4x
    // anisotropic, no LOD bias): the one every DriveableSurface colour, detail and normal map and most
    // TerrainPBR / PlotPBR tiling maps use (`mwtool sampler-usage`). The other common state (trilinear,
    // MipLODBias -0.9) needs a per-slot sampler and a bias that wgpu samplers lack; docs/formats.md
    // "SamplerState".
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        anisotropy_clamp: 4,
        ..ImageSamplerDescriptor::linear()
    });
    Some(image)
}
