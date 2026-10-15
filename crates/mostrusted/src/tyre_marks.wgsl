// Tyre marks: a port of the game's `Vfx_TyreMarksNew` G-buffer pass (`mwtool shader-asm Vfx_TyreMarksNew`, the
// pass with three render targets) and its vertex program.
//
// Vertex program: position = centre + side x WidthScale x (v - 0.5) + up x 0.03 m, then pulled towards the
// eye by 0.35 - 0.33 x saturate(dot(to_eye, up)) m (0.02 m seen from above, 0.35 m at grazing angles).
// Pixel program: the Run diffuse layer (premultiplied by its own alpha), then the Slip layer over it with
// weight Slip.a x COLOR.x and the Spin layer with weight Spin.a x COLOR.y (each "over": c = c (1 - w) + layer
// w); everything x saturate(opacity); discard below 1/2550; written premultiplied to the albedo target (RT1,
// which holds sqrt(albedo); here the sRGB-decoded texture is taken as linear albedo), RT0 = (0, 0, 0, a), RT2 = Specular x a. Here the albedo is lit in place with the
// key light and the irradiance cube (the game lights it in the deferred pass) and blended ONE, INV_SRC_ALPHA.

#import bevy_pbr::mesh_view_bindings::view

struct MarkParams {
    // x = WidthScale, y = Specular
    material: vec4<f32>,
    // rgb = key light colour (cd/m2), w = sky brightness (cd/m2 per irradiance-cube unit)
    key_colour: vec4<f32>,
    // xyz = direction the key light travels
    key_direction: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: MarkParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var run_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var run_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var slip_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var slip_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var spin_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var spin_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var irradiance: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var irradiance_sampler: sampler;

struct Vertex {
    // centre of the strip at this point
    @location(0) position: vec3<f32>,
    // ground normal at the contact
    @location(1) normal: vec3<f32>,
    // x = along the strip (game TEXCOORD.w), y = side 0 / 1 (game COLOR.z)
    @location(2) uv: vec2<f32>,
    // vector across the strip, the tyre's width long (game TEXCOORD.xyz)
    @location(3) side: vec4<f32>,
    // x = slip weight, y = spin weight, w = opacity (game COLOR.xyw)
    @location(5) colour: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) weights: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let up = normalize(v.normal);
    var p = v.position + v.side.xyz * params.material.x * (v.uv.y - 0.5) + up * 0.03;
    let to_eye = normalize(view.world_position - p);
    p = p + to_eye * (0.35 - 0.33 * saturate(dot(to_eye, up)));
    out.clip = view.clip_from_world * vec4<f32>(p, 1.0);
    out.weights = v.colour;
    out.uv = v.uv;
    out.normal = up;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let run = textureSample(run_tex, run_sampler, in.uv);
    var c = vec4<f32>(run.rgb * run.a, run.a);
    let slip = textureSample(slip_tex, slip_sampler, in.uv);
    let ws = slip.a * in.weights.x;
    c = c * (1.0 - ws) + vec4<f32>(slip.rgb * ws, ws);
    let spin = textureSample(spin_tex, spin_sampler, in.uv);
    let wp = spin.a * in.weights.y;
    c = c * (1.0 - wp) + vec4<f32>(spin.rgb * wp, wp);
    let opacity = saturate(in.weights.w);
    c = c * opacity;
    if c.a - 0.000392 < 0.0 {
        discard;
    }
    // The textures are sampled as sRGB (linear albedo), as the particle textures are.
    let albedo = c.rgb / max(c.a, 1e-4);
    let n = normalize(in.normal);
    let irr = textureSampleLevel(irradiance, irradiance_sampler, n, 0.0).rgb * params.key_colour.w;
    let key = params.key_colour.rgb * saturate(dot(n, -params.key_direction.xyz));
    let lit = albedo * (max(irr, vec3<f32>(0.0)) + key) * view.exposure;
    return vec4<f32>(lit * c.a, c.a);
}
