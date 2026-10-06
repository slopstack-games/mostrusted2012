// The game's auto exposure and tonemap (research log "the game's tonemap"), three passes a frame:
//   meter:   `Blit2d_AutomaticExposureMeter`: displayed luma of the frame under the current exposure,
//            on a small grid (the game meters a downsampled copy).
//   adapt:   `Blit2d_GeneratePCTonemapConstants_AverageLumaFinal` + `_BuildConstant`: average the
//            metered luma and step the log2 exposure towards a target; the 1×1 state holds
//            (exposure, smoothed luma) and carries over to the next frame.
//   tonemap: `GBufferComposite` without motion blur: the bloom term (bloom.wgsl builds its texture;
//            `MW_NO_BLOOM=1` turns it off), the vignette (A × v + D from the default
//            PostFX keyframe, `MW_NO_VIGNETTE=1` turns it off), then 1 - exp(-hdr × E), E = 2^(exposure + bias), gamma-encoded
//            with the game's polynomial fit of sRGB. Then the grade of the final post pass built into
//            NFS13.exe (pixel shader at 0x01016148, research log 2026-10-06): the "TintBlend" volume
//            sampled at rgb × ColourCubeScalesOffsets.xyz + .w (15/16, 1/32), here the keyframe's
//            ColourCube alone. The result is decoded back to linear for Bevy's sRGB swapchain.
//            Bloom off is the game's own off path: `FUN_0089d5c0` then sets B (slot 0x9d) to
//            (1, 1, 1, 0), and B.w = 0 removes the bloom term. The TintBlend slot weights
//            (`FUN_008c9e40`) wait for live reads (decomp.md "Post-processing").
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
    // Vignette: A (rgb = outer − inner, w = the weight offset 1 − k), D (rgb = inner, w = k),
    // centre xy + scale × k, angle (x, radians).
    vignette_a: vec4<f32>,
    vignette_d: vec4<f32>,
    vignette_centre_scale: vec4<f32>,
    vignette_angle: vec4<f32>,
    // B: BloomColour.rgb, bloom intensity (0 = bloom off)
    bloom: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tm: Tonemap;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var hdr_sampler: sampler;
@group(0) @binding(3) var meter_texture: texture_2d<f32>;
@group(0) @binding(4) var state: texture_2d<f32>;
@group(0) @binding(5) var colour_cube: texture_3d<f32>;
@group(0) @binding(6) var cube_sampler: sampler;
@group(0) @binding(7) var bloom_texture: texture_2d<f32>;

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

// The vignette weight of the bloom add pass (exe programs VS 0x01011e18, PS 0x010117c0): the texture
// coordinate (y down, as Bevy's uv) mapped to [-1, 1], rotated by the angle, offset by the centre and
// scaled; v = smoothstep(saturate(|p| + A.w)). The game computes it at the bloom resolution and stores it in
// the bloom target's alpha; it is smooth, so evaluating it per pixel matches up to the bilinear fetch.
fn vignette_weight(uv: vec2<f32>) -> f32 {
    let q = uv * 2.0 - 1.0;
    let sc = vec2(sin(tm.vignette_angle.x), cos(tm.vignette_angle.x));
    let r = vec2(q.x * sc.y + q.y * sc.x, q.y * sc.y - q.x * sc.x);
    let p = (r - tm.vignette_centre_scale.xy * 2.0 + 1.0) * tm.vignette_centre_scale.zw;
    return smoothstep(0.0, 1.0, saturate(length(p) + tm.vignette_a.w));
}

@fragment
fn tonemap(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(hdr, hdr_sampler, in.uv, 0.0);
    let k = exposure_scale(current_exposure());
    // GBufferComposite: (hdr + bloom × B.rgb × saturate(bloom × T.w) × B.w) × (A.rgb × v + D.rgb) × T.w
    // (decomp.md "Post-processing"); T.w = k / ln 2, the curve below is 2^(-x · T.w) = exp(-x · k).
    let bloom = textureSampleLevel(bloom_texture, hdr_sampler, in.uv, 0.0).rgb;
    let lit = c.rgb + bloom * tm.bloom.rgb * saturate(bloom * k * 1.442695) * tm.bloom.w;
    let vignette = tm.vignette_a.rgb * vignette_weight(in.uv) + tm.vignette_d.rgb;
    var g = gamma_fit(max(1.0 - exp(-lit * vignette * k), vec3(0.0)));
    if tm.flags.y > 0.5 {
        g = textureSampleLevel(colour_cube, cube_sampler, saturate(g) * (15.0 / 16.0) + 1.0 / 32.0, 0.0).rgb;
    }
    return vec4(srgb_to_linear(g), c.a);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
