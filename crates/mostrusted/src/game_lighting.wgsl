// The game's deferred key light and ambient: a port of `LightBuffer_KeylightAndAmbient` (see
// `mwtool shader-asm LightBuffer_KeylightAndAmbient`). It reads the game G-buffer that `layered.wgsl`
// packs into Bevy's deferred texture:
//   RT0 = (N * 0.5 + 0.5, roughness)  RT1 = (sqrt(albedo), AO)  RT2 = (F0, Fresnel strength)
//   RT3 = (sqrt(indirect × exposure), vehicle flag)
// It also applies the game's Fog pass (`mwtool shader-asm Fog`) to the lit geometry.
// Divergences from the game, for now: no cascaded shadow map (shadow = 1); one irradiance cube for both
// of the game's (it blends a second, local one for vehicles by distance); no height-layer fog; the fog
// colour's sun gradient is approximated.

#import bevy_pbr::{
    mesh_view_bindings::{view, deferred_prepass_texture},
    prepass_utils,
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc},
}

struct GameLighting {
    // Key light colour × illuminance (lux), and the direction it travels.
    key_colour: vec4<f32>,
    key_direction: vec4<f32>,
    // x = sky brightness (cd/m² per irradiance-cube unit)
    params: vec4<f32>,
    fog_colour: vec4<f32>,
    fog_colour_sun: vec4<f32>,
    // 1 / (far - near), near / (far - near), power
    fog_coeffs: vec4<f32>,
    // rgb, w = strength
    scattering: vec4<f32>,
    // x = phase
    scattering_params: vec4<f32>,
}

@group(2) @binding(0) var<uniform> lighting: GameLighting;
@group(2) @binding(1) var irradiance: texture_cube<f32>;
@group(2) @binding(2) var irradiance_sampler: sampler;

// Must match `GAME_LIGHTING_PASS_ID` in layered.wgsl: the pass draws at this depth and only touches
// pixels whose deferred lighting ID matches.
const PASS_ID: f32 = 2.0;

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    let uv = vec2<f32>(f32(vertex_index >> 1u), f32(vertex_index & 1u)) * 2.0;
    let clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), PASS_ID / 255.0, 1.0);
    return FullscreenVertexOutput(clip, uv);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy);
    let g = textureLoad(deferred_prepass_texture, px, 0);
    let rt0 = unpack4x8unorm(g.x);
    let rt1 = unpack4x8unorm(g.y);
    let rt2 = unpack4x8unorm(g.z);
    let rt3 = unpack4x8unorm(g.w);

    let depth = prepass_utils::prepass_depth(in.position, 0u);
    let frag = vec4(in.position.xy, depth, 1.0);
    let world = position_ndc_to_world(frag_coord_to_ndc(frag));
    let v = normalize(view.world_position.xyz - world);

    let n = normalize(rt0.xyz * 2.0 - 1.0);
    let rough = rt0.w;
    let albedo = rt1.rgb * rt1.rgb;
    let ao = rt1.w;
    let f0 = rt2.rgb;
    let fresnel_strength = rt2.w;
    let car = select(0.0, 1.0, rt3.w > 0.992157);
    let one_minus_f0 = 1.0 - f0;

    // Key light, dimmed in occluded areas: the CSM term (1 here) × (1 - 0.8 × (1 - AO)).
    let l = -lighting.key_direction.xyz;
    let shadow = 1.0;
    let key = lighting.key_colour.rgb * view.exposure * shadow * saturate(1.0 - 0.8 * (1.0 - ao));
    let nl_raw = dot(n, l);
    let nl = saturate(nl_raw);

    // Diffuse: Lambert for the world; vehicles use the Ashikhmin-Shirley term
    // 28/(23π) (1 - F0) (1 - (1 - N·L/2)^5) (1 - (1 - N·V/2)^5).
    let lambert = albedo * one_minus_f0 * nl * 0.31831;
    let nv = saturate(dot(n, v));
    let as_v = 1.0 - pow(1.0 - 0.5 * nv, 5.0);
    let as_l = 1.0 - pow(1.0 - 0.5 * nl_raw, 5.0);
    let ashikhmin = albedo * one_minus_f0 * (as_v * as_l * 0.387508) * nl;
    var colour = key * mix(lambert, ashikhmin, car);

    // Ambient: irradiance cube around the normal × AO × (1 - F0) × albedo.
    let irr = textureSampleLevel(irradiance, irradiance_sampler, n, 0.0).rgb * lighting.params.x * view.exposure;
    colour += irr * ao * one_minus_f0 * albedo;

    // Specular: normalised Blinn-Phong with power exp(11 - 11 roughness), a max(N·L, N·V) × V·H visibility
    // term and Schlick Fresnel whose (1 - F0) part is scaled by the Fresnel strength.
    let power = exp(11.0001 - 11.0 * rough);
    let h = normalize(v + l);
    let nh = min(saturate(dot(n, h)) + 0.0001, 1.0);
    let vh = dot(v, h);
    let d = (power + 1.0) * 0.039789 * pow(nh, power);
    let vis = 1.0 / (saturate(max(nv, nl_raw) * vh) + 0.0013);
    let f = f0 + one_minus_f0 * fresnel_strength * pow(saturate(1.0 - vh), 5.0);
    colour += key * d * vis * f * nl;

    // Indirect light from the G-buffer pass (sky reflection, lightmaps), already exposed.
    colour += rt3.rgb * rt3.rgb;

    return vec4(apply_fog(colour, world, l), 1.0);
}

// The game's Fog pass: amount t = saturate(d / (far - near) - near / (far - near))^power, blended as
// scene × (1 - t) + colour × t, plus in-scattering t × ScatteringColour × strength / (1 + phase cosθ)²
// with θ measured from the sun's corona direction (opposite the sun, so looking at the sun gives the
// peak). The fog colour comes from a gradient between Colour and ColourSun that the game rebuilds per
// frame; here it is blended by how close the view azimuth is to the sun's (approximation).
fn apply_fog(colour: vec3<f32>, world: vec3<f32>, to_sun: vec3<f32>) -> vec3<f32> {
    let rel = world - view.world_position.xyz;
    let d = length(rel);
    let dir = rel / max(d, 1e-4);
    let c = lighting.fog_coeffs;
    let t = pow(saturate(d * c.x - c.y), c.z);
    let toward = pow(saturate(dot(normalize(vec3(dir.x, 0.0, dir.z)), normalize(vec3(to_sun.x, 0.0, to_sun.z)))), 4.0);
    let fog = mix(lighting.fog_colour.rgb, lighting.fog_colour_sun.rgb, toward) * view.exposure;
    let cos_corona = dot(dir, -to_sun);
    let phase = lighting.scattering.w / pow(1.0 + lighting.scattering_params.x * cos_corona, 2.0);
    let scatter = t * min(phase, 64.0) * lighting.scattering.rgb * view.exposure;
    return colour * (1.0 - t) + fog * t + scatter;
}
