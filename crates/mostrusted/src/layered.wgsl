// Albedo for the game's multi-texture shader families, ported from their
// DXBC (see `mwtool shader-asm`). Lighting is still Bevy's PBR.
//
// mode 0: plain StandardMaterial
// mode 1: DriveableSurface   colour(uv0) * detail(world.xz * DetailMapUvScale) * 2
// mode 2: TerrainPBR         mask/noise driven blend of three tiling layers
// mode 3: PlotPBR            mask-weighted tinted tiling layers + decal
// mode 4: road line          road (mode 1) with a tinted line texture over it

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct Layered {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    // p[0]: Tiling_Ratios_X (terrain: noise ratio in w)
    // p[1]: Tiling_Ratios_Y; roads: DetailMapUvScale in xy
    // p[2]: plot: maskTilingRatios.xy, decalTilingRatios.zw; line: Line_Diffuse
    // p[3..5]: plot: tiling1..3Diffuse tints
    // p[6]: plot: decal_Diffuse (w = decal strength)
    p: array<vec4<f32>, 7>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> layered: Layered;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var tex0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var samp0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var tex1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var samp1: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var tex2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var samp2: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var tex3: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var samp3: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var tex4: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var samp4: sampler;

// TerrainPBR layer weights: overlay of the mask channels with a noise texture.
fn terrain_weights(m: vec4<f32>, n: f32) -> vec3<f32> {
    var w: vec3<f32>;
    w.x = select(2.0 * n * m.r, 1.0 - 2.0 * (1.0 - m.r) * (1.0 - n), m.r > 0.5);
    w.z = select(2.0 * (1.0 - n) * m.b, 1.0 - 2.0 * n * (1.0 - m.b), m.b > 0.5);
    w.y = 1.0 - (m.r + m.b);
    w = saturate(w);
    return w / max(dot(w, vec3(1.0)), 1e-4);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    var uv_b = uv;
#ifdef VERTEX_UVS_B
    uv_b = in.uv_b;
#endif

    let tx = layered.p[0];
    let ty = layered.p[1];
    if layered.mode == 1u || layered.mode == 4u {
        let colour = textureSample(tex0, samp0, uv).rgb;
        // The road vertex shader projects the detail map in world space.
        let detail = textureSample(tex1, samp1, in.world_position.xz * ty.xy).rgb;
        var albedo = colour * detail * 2.0;
        if layered.mode == 4u {
            let line = textureSample(tex2, samp2, uv_b);
            albedo = mix(albedo, line.rgb * layered.p[2].rgb, line.a);
        }
        pbr_input.material.base_color = vec4(albedo * pbr_input.material.base_color.rgb, 1.0);
    } else if layered.mode == 2u {
        let m = textureSample(tex0, samp0, uv);
        let n = textureSample(tex1, samp1, uv_b * tx.w).g;
        let w = terrain_weights(m, n);
        let l1 = textureSample(tex2, samp2, uv_b * vec2(tx.x, ty.x)).rgb;
        let l2 = textureSample(tex3, samp3, uv_b * vec2(tx.y, ty.y)).rgb;
        let l3 = textureSample(tex4, samp4, uv_b * vec2(tx.z, ty.z)).rgb;
        let albedo = (l1 * w.x + l2 * w.y + l3 * w.z) * (2.0 * m.g);
        pbr_input.material.base_color = vec4(albedo, 1.0);
    } else if layered.mode == 3u {
        let md = layered.p[2];
        let m = textureSample(tex0, samp0, uv * md.xy).rgb;
        let w = m / max(dot(m, vec3(1.0)), 1e-4);
        let l1 = textureSample(tex1, samp1, uv * vec2(tx.x, ty.x)).rgb * layered.p[3].rgb;
        let l2 = textureSample(tex2, samp2, uv * vec2(tx.y, ty.y)).rgb * layered.p[4].rgb;
        let l3 = textureSample(tex3, samp3, uv * vec2(tx.z, ty.z)).rgb * layered.p[5].rgb;
        var albedo = l1 * w.x + l2 * w.y + l3 * w.z;
        let d = textureSample(tex4, samp4, uv * md.zw);
        let dt = layered.p[6];
        albedo = mix(albedo, d.x * dt.rgb, d.y * dt.w);
        pbr_input.material.base_color = vec4(albedo, 1.0);
    }

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
