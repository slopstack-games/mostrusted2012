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
/// are taken from the vertex data when present, otherwise computed.
pub fn mesh(m: &mw_formats::DecodedMesh, uv_b: Option<u8>) -> Option<Mesh> {
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
    match &m.normals {
        Some(n) => {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n.iter().map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>())
        }
        None => mesh.compute_smooth_normals(),
    }
    // Tangents feed the normal maps. Packed frames carry the bitangent sign in w; stored float
    // tangents (terrain) have none, so they get +1.
    if let (Some(t), Some(_)) = (&m.tangents, &m.normals) {
        let t: Vec<[f32; 4]> = t.iter().map(|v| [v[0], v[1], v[2], if v[3] < 0.0 { -1.0 } else { 1.0 }]).collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, t);
    }
    Some(mesh)
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
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        anisotropy_clamp: 8,
        ..ImageSamplerDescriptor::linear()
    });
    Some(image)
}
