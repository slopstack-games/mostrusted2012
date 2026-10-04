// Camera-facing particle quads: a port of the game's `VfxParticles_Diffuse*` programs
// (`mwtool shader-asm VfxParticles_Diffuse_SubUV`). The CPU (`particles.rs`) expands each particle into
// a world-space quad as the game's Bear renderers do; the vertex program lights it and picks the atlas
// frames, the pixel program blends two frames and writes premultiplied colour.
//
// Game vertex inputs and where they come from here:
//   COLOR       rgba                      -> @location(5) colour (linear rgb, alpha)
//   TEXCOORD.xy corner uv, .z SubUV (age) -> @location(2) uv, @location(3) frame.x
//   TEXCOORD1.zw translucency, key light as ambient -> @location(4) light.xy
//   TEXCOORD3.xyz lighting normal, .w wrap          -> @location(1) normal, light.z
// Lighting (vertex program): ambient = irradiance(n) x 1.3 + light.y x KeyLightColour;
// key = saturate((n.(-L) + wrap) / (1 + wrap)), lerped to 1 by the translucency, x (1 - light.y), x the
// draw's opacity (gVfxConstants.z, here light.w) x the vertex shadow test (not ported). The game's
// irradiance quadric is the lighting pass's irradiance cube here; the exposure-texture and LDR terms of the
// program are left out (they are off in the HDR path).
// Pixel program: lerp(frame0, frame1, blend) x colour; discard below 1/255; rgb x a, alpha x opacity
// (blend ONE, INV_SRC_ALPHA). The soft variants multiply alpha by the depth fade.

#import bevy_pbr::mesh_view_bindings::view
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct ParticleParams {
    // x, y = atlas tiles (NumTiles), z = EnableLooping, w = frame blending on (1) / off (0)
    atlas: vec4<f32>,
    // rgb = key light colour (cd/m2), w = sky brightness (cd/m2 per irradiance-cube unit)
    key_colour: vec4<f32>,
    // xyz = direction the key light travels
    key_direction: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ParticleParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var irradiance: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var irradiance_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // x = SubUV (particle age 0..1), y = soft-particle fade distance (m, 0 = off)
    @location(3) frame: vec2<f32>,
    // x = translucency, y = key light as ambient, z = wrap, w = opacity
    @location(4) light: vec4<f32>,
    @location(5) colour: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) uv0: vec2<f32>,
    // xy = second frame, z = blend between the frames
    @location(2) uv1: vec3<f32>,
    // x = fade distance, y = opacity
    @location(3) misc: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.clip = view.clip_from_world * vec4<f32>(v.position, 1.0);

    let n = normalize(v.normal);
    let irr = textureSampleLevel(irradiance, irradiance_sampler, n, 0.0).rgb * params.key_colour.w * view.exposure;
    let key_colour = params.key_colour.rgb * view.exposure;
    let wrap = v.light.z;
    var key = saturate((dot(n, -params.key_direction.xyz) + wrap) / (1.0 + wrap));
    key = mix(key, 1.0, v.light.x) * (1.0 - v.light.y);
    let ambient = max(irr, vec3<f32>(0.0)) * 1.3 + v.light.y * key_colour;
    let lit = ambient + key_colour * key * v.light.w;
    out.colour = vec4<f32>(v.colour.rgb * lit, v.colour.a);

    // Atlas frames, as the game's program computes them (row index from frame / tiles.y).
    let tiles = max(params.atlas.xy, vec2<f32>(1.0));
    let count = tiles.x * tiles.y;
    let f = v.frame.x * count;
    var frames = vec4<f32>(f, f, f + 1.0, f + 1.0);
    if params.atlas.z < 0.5 {
        let last = count - 0.999;
        frames = vec4<f32>(min(last, f), min(last, f), min(last, f + 1.0), min(last, f + 1.0));
    }
    let inv = 1.0 / tiles;
    let scaled = frames * vec4<f32>(1.0, inv.y, 1.0, inv.y);
    let offsets = fract(vec4<f32>(inv, inv) * floor(scaled));
    out.uv0 = v.uv * inv + offsets.xy;
    out.uv1 = vec3<f32>(v.uv * inv + offsets.zw, fract(frames.y) * params.atlas.w);
    out.misc = vec2<f32>(v.frame.y, v.light.w);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = textureSample(diffuse, diffuse_sampler, in.uv0);
    let b = textureSample(diffuse, diffuse_sampler, in.uv1.xy);
    let c = mix(a, b, in.uv1.z) * in.colour;
    var alpha = c.a;
#ifdef DEPTH_PREPASS
    if in.misc.x > 0.0 {
        // Linear depths from reverse-Z: view z = near / ndc z.
        let near = view.clip_from_view[3][2];
        let scene = near / max(prepass_depth(in.clip, 0u), 1e-7);
        let particle = near / max(in.clip.z, 1e-7);
        alpha *= saturate((scene - particle) / in.misc.x);
    }
#endif
    if alpha * in.misc.y < 0.003922 {
        discard;
    }
    return vec4<f32>(c.rgb * alpha, alpha * in.misc.y);
}
