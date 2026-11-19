// The original's final post pass in free roam and its depth-of-field input (decomp.md "Post-processing";
// run by tonemap.rs after the composite):
//   dof_down:   PS 0x01012858 (`FUN_008e7530`): the composite's frame to half resolution, four taps at
//               ± half a source texel, each weighted by its circle of confusion |CoC| × 0.99 + 0.01; alpha
//               = the mean weight. CoC = max(K.x × depth + K.y, K.z) with K from the camera's depth-of-field
//               block; in view depth z that is max(x0 (1 − focus / z), 0) (`FUN_008c9e40`, K.z = 0).
//               The game then runs two 4-tap blur passes over it (`FUN_008e7900`, PS 0x010130f0 family) whose
//               taps i = 1…3 count only where the CoC exceeds i / 4; the free-roam CoC stays under x0 = 0.042,
//               so only their first tap survives and they are left out (its sub-texel offset was not traced).
//   final_pass: PS 0x01017050 (the depth of field + tint variant, index 3 of the 12 `FUN_008e41e0` builds;
//               `FUN_008e8c40` picks it from the post flags +0xd1c = 0x7f, live): lerp(frame, dof, min(15
//               |CoC|, 1)), the splatter overlay (none here), then the colour cube at rgb ×
//               ColourCubeScalesOffsets.xyz + .w with scale (c × 15/16) and offset (b + 0.5 − c / 2 + 1/32),
//               c / b the contrast / brightness (1 / 0 plus the sun-facing boost, tonemap.rs `sun_grade`).
// The result is decoded to linear for Bevy's sRGB swapchain.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

// Must match `TonemapUniform` (tonemap.rs) / `Tonemap` (tonemap.wgsl).
struct Tonemap {
    camera: vec4<f32>,
    exposure: vec4<f32>,
    setting: vec4<f32>,
    // y = 1 applies the colour cube; z = the PostFX states' cube weight (sequence_picture.rs)
    flags: vec4<f32>,
    vignette_a: vec4<f32>,
    vignette_d: vec4<f32>,
    vignette_centre_scale: vec4<f32>,
    vignette_angle: vec4<f32>,
    bloom: vec4<f32>,
    // x = contrast c, y = brightness b, z = 1 with depth of field
    grade: vec4<f32>,
    // x = near plane (reversed depth = near / z), y = CoC at infinity x0, z = focus distance
    dof: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tm: Tonemap;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var frame_sampler: sampler;
@group(0) @binding(3) var dof_texture: texture_2d<f32>;
@group(0) @binding(4) var depth: texture_depth_2d;
@group(0) @binding(5) var colour_cube: texture_3d<f32>;
@group(0) @binding(6) var cube_sampler: sampler;
// The sequences' PostFX state cube (TintBlend's state slots, FUN_008e4a90).
@group(0) @binding(7) var state_cube: texture_3d<f32>;

// max(K.x × d + K.y, K.z) of the game's device depth, written in view depth.
fn coc_at(p: vec2<i32>) -> f32 {
    let d = textureLoad(depth, p, 0);
    if d <= 0.0 {
        return tm.dof.y;
    }
    let z = tm.dof.x / d;
    return max(tm.dof.y * (1.0 - tm.dof.z / z), 0.0);
}

@fragment
fn dof_down(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(frame));
    let base = vec2<i32>(in.position.xy) * 2;
    var sum = vec3(0.0);
    var weight = 0.0;
    for (var i = 0; i < 4; i += 1) {
        let p = min(base + vec2(i & 1, i >> 1u), size - 1);
        let w = abs(coc_at(p)) * 0.99 + 0.01;
        sum += textureLoad(frame, p, 0).rgb * w;
        weight += w;
    }
    return vec4(sum / weight, weight * 0.25);
}

@fragment
fn final_pass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(frame, frame_sampler, in.uv, 0.0);
    var rgb = c.rgb;
    if tm.grade.z > 0.5 {
        let p = min(vec2<i32>(in.position.xy), vec2<i32>(textureDimensions(depth)) - 1);
        let blend = min(abs(coc_at(p)) * 15.0, 1.0);
        let dof = textureSampleLevel(dof_texture, frame_sampler, in.uv, 0.0).rgb;
        rgb = mix(rgb, dof, blend);
    }
    let contrast = tm.grade.x;
    let coord = rgb * (contrast * 15.0 / 16.0) + (tm.grade.y + 0.5 - contrast * 0.5 + 1.0 / 32.0);
    var g: vec3<f32>;
    if tm.flags.y > 0.5 {
        g = textureSampleLevel(colour_cube, cube_sampler, saturate(coord), 0.0).rgb;
    } else {
        // The identity cube under clamp-to-edge addressing.
        g = saturate((coord - 1.0 / 32.0) * (16.0 / 15.0));
    }
    // TintBlend (FUN_008c9e40): the weights above 1 in sum are scaled to 1; the environment pair weighs 1, so
    // the state cube's w gives (env + w · state) / (1 + w) (a weighted sum of entries, linear in the lookup).
    if tm.flags.z > 0.0 {
        let s = textureSampleLevel(state_cube, cube_sampler, saturate(coord), 0.0).rgb;
        g = (g + tm.flags.z * s) / (1.0 + tm.flags.z);
    }
    return vec4(srgb_to_linear(g), c.a);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
