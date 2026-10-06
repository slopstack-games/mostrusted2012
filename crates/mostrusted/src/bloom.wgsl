// The game's bloom chain (decomp.md "Post-processing", programs built into NFS13.exe), run by tonemap.rs
// between the exposure adaptation and the composite:
//   down_hdr:  PS 0x010104c0 (VS 0x0100fc50): 4 bilinear taps at ±1 source texel on both diagonals, × the
//              tonemap scale T.w / 128, stored as RGBM (M = max channel, at most 1, rounded up to 1/255).
//              Decoded = average × T.w, so each channel clips at 32 in exposure-scaled units.
//   down_rgbm: PS 0x0100ffa0: the same 4 taps on an RGBM level, decoded × 32, re-encoded.
//   blur_h:    PS 0x01010d38 (VS 0x010109c8): 5 taps at 0, ±1.407333, ±3.294215 texels along x, weights
//              6.533238, 9.728171, 3.005209 (sum 32, the RGBM range), ÷ T.w: HDR game units, float target.
//   add:       PS 0x010117c0 (VS 0x01011280): the same kernel along y (weights normalised), × kColorAddWeight.z,
//              plus the coarser combined level × .w. Its alpha (the vignette weight) is computed per pixel in
//              tonemap.wgsl instead.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Bloom {
    // x = kColorAddWeight.z (this level), y = .w (the coarser result), z = the exposure bias
    weights: vec4<f32>,
}

@group(0) @binding(0) var<uniform> bloom: Bloom;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;
@group(0) @binding(3) var coarser: texture_2d<f32>;
@group(0) @binding(4) var state: texture_2d<f32>;

// T.w, the 1×1 tonemap texture's w: log2(e) × 2^(exposure + bias) (`_BuildConstant`, research log
// 2026-10-04); tonemap.wgsl's exp(-x · 2^(e + bias)) is the game's 2^(-x · T.w).
fn tonemap_scale() -> f32 {
    return 1.442695 * exp2(textureLoad(state, vec2(0), 0).x + bloom.weights.z);
}

fn encode_rgbm(c: vec3<f32>) -> vec4<f32> {
    let v = c * 0.0078125;
    let m = ceil(min(max(max(v.r, v.g), max(v.b, 0.000001)), 1.0) * 255.0) / 255.0;
    return vec4(saturate(v / m), m);
}

fn decode(c: vec4<f32>) -> vec3<f32> {
    return c.rgb * c.a;
}

fn four_taps(uv: vec2<f32>) -> array<vec4<f32>, 4> {
    let t = 1.0 / vec2<f32>(textureDimensions(source));
    return array(
        textureSampleLevel(source, source_sampler, uv + vec2(-t.x, -t.y), 0.0),
        textureSampleLevel(source, source_sampler, uv + vec2(-t.x, t.y), 0.0),
        textureSampleLevel(source, source_sampler, uv + vec2(t.x, t.y), 0.0),
        textureSampleLevel(source, source_sampler, uv + vec2(t.x, -t.y), 0.0),
    );
}

@fragment
fn down_hdr(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let s = four_taps(in.uv);
    return encode_rgbm((s[0].rgb + s[1].rgb + s[2].rgb + s[3].rgb) * tonemap_scale());
}

@fragment
fn down_rgbm(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let s = four_taps(in.uv);
    return encode_rgbm((decode(s[0]) + decode(s[1]) + decode(s[2]) + decode(s[3])) * 32.0);
}

const OFFSET_1: f32 = 1.407333;
const OFFSET_2: f32 = 3.294215;
const WEIGHT_0: f32 = 6.533238;
const WEIGHT_1: f32 = 9.728171;
const WEIGHT_2: f32 = 3.005209;

@fragment
fn blur_h(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / f32(textureDimensions(source).x);
    var sum = decode(textureSampleLevel(source, source_sampler, in.uv, 0.0)) * WEIGHT_0;
    for (var i = -1.0; i <= 1.0; i += 2.0) {
        let a = textureSampleLevel(source, source_sampler, in.uv + vec2(i * OFFSET_1 * t, 0.0), 0.0);
        let b = textureSampleLevel(source, source_sampler, in.uv + vec2(i * OFFSET_2 * t, 0.0), 0.0);
        sum += decode(a) * WEIGHT_1 + decode(b) * WEIGHT_2;
    }
    return vec4(sum / tonemap_scale(), 1.0);
}

@fragment
fn add(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / f32(textureDimensions(source).y);
    var sum = textureSampleLevel(source, source_sampler, in.uv, 0.0).rgb * (WEIGHT_0 / 32.0);
    for (var i = -1.0; i <= 1.0; i += 2.0) {
        let a = textureSampleLevel(source, source_sampler, in.uv + vec2(0.0, i * OFFSET_1 * t), 0.0).rgb;
        let b = textureSampleLevel(source, source_sampler, in.uv + vec2(0.0, i * OFFSET_2 * t), 0.0).rgb;
        sum += a * (WEIGHT_1 / 32.0) + b * (WEIGHT_2 / 32.0);
    }
    let c = textureSampleLevel(coarser, source_sampler, in.uv, 0.0).rgb;
    return vec4(sum * bloom.weights.x + c * bloom.weights.y, 1.0);
}
