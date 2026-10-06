// The game's auto exposure and tonemap (research log "the game's tonemap"), three passes a frame:
//   meter:   `Blit2d_AutomaticExposureMeter`: displayed luma of the frame under the current exposure,
//            on a small grid (the game meters a downsampled copy).
//   adapt:   `Blit2d_GeneratePCTonemapConstants_AverageLumaFinal` + `_BuildConstant`: average the
//            metered luma and step the log2 exposure towards a target; the 1×1 state holds
//            (exposure, smoothed luma) and carries over to the next frame.
//   tonemap: `GBufferComposite` without motion blur, bloom or the vignette (A × vignette + D, neutral
//            in the default PostFX keyframe): 1 - exp(-hdr × E), E = 2^(exposure + bias), gamma-encoded
//            with the game's polynomial fit of sRGB. Then the grade of the final post pass built into
//            NFS13.exe (pixel shader at 0x01016148, research log 2026-10-06): the "TintBlend" volume
//            sampled at rgb × ColourCubeScalesOffsets.xyz + .w (15/16, 1/32), here the keyframe's
//            ColourCube alone. The result is decoded back to linear for Bevy's sRGB swapchain.
// The light buffer holds the game's HDR units: the camera's exposure is set so that Bevy's
// `view.exposure` is 1 / GAME_UNIT (`main.rs`).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Tonemap {
    // TonemapCameraConstants: min, max exposure, bias, manual (exposure fixed at min)
    camera: vec4<f32>,
    // TonemapExposureConstants: -, luma dead band, speed (stops / s), frame time (s)
    exposure: vec4<f32>,
    // TonemapExposureSettingConstants: target luma high, low; scene luminance high, low
    setting: vec4<f32>,
    // x = 1 restarts adaptation from the middle of the range, y = 1 applies the colour cube
    flags: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tm: Tonemap;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler: sampler;
@group(0) @binding(3) var meter_texture: texture_2d<f32>;
@group(0) @binding(4) var state: texture_2d<f32>;
@group(0) @binding(5) var colour_cube: texture_3d<f32>;
@group(0) @binding(6) var cube_sampler: sampler;

// Must match `METER_SIZE` in tonemap.rs.
const METER: u32 = 32u;

fn exposure_scale(e: f32) -> f32 {
    return exp2(e + tm.camera.z);
}

fn current_exposure() -> f32 {
    return textureLoad(state, vec2(0), 0).x;
}

// The game's gamma encode: a polynomial fit of linear → sRGB.
fn gamma_fit(c: vec3<f32>) -> vec3<f32> {
    let p = (c * 0.070706 - 0.317854) * c - 0.022711;
    return saturate(sqrt(c) * 1.271696 + p);
}

@fragment
fn meter(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // 4 × 4 bilinear taps over this texel's share of the frame.
    let k = exposure_scale(current_exposure());
    var sum = vec3(0.0);
    for (var y = 0u; y < 4u; y += 1u) {
        for (var x = 0u; x < 4u; x += 1u) {
            let o = (vec2(f32(x), f32(y)) - 1.5) / (4.0 * f32(METER));
            sum += textureSampleLevel(hdr, hdr_sampler, in.uv + o, 0.0).rgb;
        }
    }
    let c = max(1.0 - exp(-sum / 16.0 * k), vec3(0.0));
    let luma = dot(gamma_fit(c), vec3(0.299, 0.587, 0.114));
    return vec4(luma);
}

@fragment
fn adapt(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var sum = 0.0;
    for (var y = 0u; y < METER; y += 1u) {
        for (var x = 0u; x < METER; x += 1u) {
            sum += textureLoad(meter_texture, vec2(x, y), 0).x;
        }
    }
    let luma = sum / f32(METER * METER);
    var prev = textureLoad(state, vec2(0), 0).xy;
    if tm.flags.x > 0.5 {
        prev = vec2(0.5 * (tm.camera.x + tm.camera.y), luma);
    }
    let dt = tm.exposure.w;
    let band = max(tm.exposure.y, 1e-4);
    // Smoothed luma: prev + 2^(-7.213475 |dt|) (luma - prev), as the shader has it.
    let smoothed = prev.y + exp2(-7.213475 * abs(dt)) * (luma - prev.y);
    // Scene luminance by inverting the curve at the mean luma, mapped onto the target range.
    let lin = pow(max(luma, 1e-6), 2.2);
    let scene = log2(max(1.0 - lin, 1e-6)) / -exp2(prev.x);
    let s = tm.setting;
    let t = saturate((scene - s.w) / max(s.z - s.w, 1e-4));
    let goal = t * (s.x - s.y) + s.y;
    let step = abs(dt * tm.exposure.z);
    var e: f32;
    if smoothed < goal - band {
        e = prev.x + step;
    } else if goal + band < smoothed {
        e = prev.x - step;
    } else {
        e = prev.x + step * (goal - smoothed) / band;
    }
    e = clamp(e, tm.camera.x, tm.camera.y);
    if tm.camera.w != 0.0 {
        return vec4(tm.camera.x, luma, 0.0, 0.0);
    }
    return vec4(e, smoothed, 0.0, 0.0);
}

@fragment
fn tonemap(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(hdr, hdr_sampler, in.uv, 0.0);
    let k = exposure_scale(current_exposure());
    var g = gamma_fit(max(1.0 - exp(-c.rgb * k), vec3(0.0)));
    if tm.flags.y > 0.5 {
        g = textureSampleLevel(colour_cube, cube_sampler, saturate(g) * (15.0 / 16.0) + 1.0 / 32.0, 0.0).rgb;
    }
    return vec4(srgb_to_linear(g), c.a);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
