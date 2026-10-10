// The game's deferred key light and ambient: a port of `LightBuffer_KeylightAndAmbient` (see
// `mwtool shader-asm LightBuffer_KeylightAndAmbient`). It reads the game G-buffer that `layered.wgsl`
// packs into Bevy's deferred texture:
//   RT0 = (N * 0.5 + 0.5, roughness)  RT1 = (sqrt(albedo), AO)  RT2 = (F0, Fresnel strength)
//   RT3 = (sqrt(indirect × exposure), vehicle flag)
// It also applies the game's Fog pass (`mwtool shader-asm Fog`) to the lit geometry.
// Divergences from the game, for now: the shadow cascades are Bevy's (fit, split distances, caster bias
// differ from the game's CPU-side setup; see `game_shadow`); one irradiance cube for both
// of the game's (it blends a second, local one for vehicles by distance); no height-layer fog; the fog
// colour's sun gradient is approximated.

#import bevy_pbr::{
    mesh_view_bindings::{
        view, deferred_prepass_texture, lights, directional_shadow_textures,
        directional_shadow_textures_comparison_sampler,
    },
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    prepass_utils,
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc},
}

struct GameLighting {
    // Key light colour × illuminance (lux), and the direction it travels.
    key_colour: vec4<f32>,
    key_direction: vec4<f32>,
    // x = sky brightness (cd/m² per irradiance-cube unit); shadow fade: y = 1 / fade length,
    // z = shadow distance / fade length, w = shadow term beyond the fade
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

// A world light as the game's `LightBuffer_Point` / `_Cone` / `_Spot` programs take it (gObjectCB
// `deferred_lightPosition`, `deferred_lightColour`, `deferred_lightDirection`; built on the CPU by the
// game's FUN_008b0940 / FUN_008af930 / FUN_008afee0, here by `world_lights.rs`).
struct WorldLight {
    // xyz = position, w = range in metres (the radius the game scales the light's volume mesh by)
    position: vec4<f32>,
    // rgb = Colour × intensity (× cd/m² per game unit), w = AttenuationThreshold / max(rgb in game units)
    colour: vec4<f32>,
    // xyz = towards the light along its axis (−beam direction), w = cos(cone edge)
    direction: vec4<f32>,
    // x = 1 / (cos(inner) − cos(edge)); y = 1 for cone and spot lights, 0 for point lights
    cone: vec4<f32>,
}
// Only the light-volume pipeline binds this.
@group(2) @binding(3) var<storage, read> world_lights: array<WorldLight>;

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

// The G-buffer at a pixel, unpacked.
struct Surface {
    world: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    rough: f32,
    albedo: vec3<f32>,
    ao: f32,
    f0: vec3<f32>,
    fresnel_strength: f32,
    car: f32,
    indirect: vec3<f32>,
}

fn read_surface(position: vec4<f32>) -> Surface {
    let px = vec2<i32>(position.xy);
    let g = textureLoad(deferred_prepass_texture, px, 0);
    let rt0 = unpack4x8unorm(g.x);
    let rt1 = unpack4x8unorm(g.y);
    let rt2 = unpack4x8unorm(g.z);
    let rt3 = unpack4x8unorm(g.w);
    let depth = prepass_utils::prepass_depth(position, 0u);
    let frag = vec4(position.xy, depth, 1.0);
    var s: Surface;
    s.world = position_ndc_to_world(frag_coord_to_ndc(frag));
    s.v = normalize(view.world_position.xyz - s.world);
    s.n = normalize(rt0.xyz * 2.0 - 1.0);
    s.rough = rt0.w;
    s.albedo = rt1.rgb * rt1.rgb;
    s.ao = rt1.w;
    s.f0 = rt2.rgb;
    s.fresnel_strength = rt2.w;
    s.car = select(0.0, 1.0, rt3.w > 0.992157);
    s.indirect = rt3.rgb * rt3.rgb;
    return s;
}

// What a surface sends to the eye per unit of light arriving from `l`: the BRDF × N·L that the key light
// program and the point / cone / spot light programs share (the same instructions in each).
//   Diffuse: Lambert for the world, albedo (1 − F0) N·L / π; vehicles (RT3.w = 1) use the
//   Ashikhmin-Shirley term 28/(23π) albedo (1 − F0) (1 − (1 − N·L/2)^5) (1 − (1 − N·V/2)^5) N·L.
//   Specular: normalised Blinn-Phong, D = (power + 1) × 0.039789 × N·H^power, a
//   1 / (saturate(max(N·V, N·L) × V·H) + 0.0013) visibility term and Schlick Fresnel whose (1 − F0) part
//   is scaled by the Fresnel strength. The power is `exp(11.0001 − 11 roughness)` in the disassembly.
fn surface_response(s: Surface, l: vec3<f32>) -> vec3<f32> {
    let one_minus_f0 = 1.0 - s.f0;
    let nl_raw = dot(s.n, l);
    let nl = saturate(nl_raw);
    let nv = saturate(dot(s.n, s.v));
    let lambert = s.albedo * one_minus_f0 * nl * 0.31831;
    let as_v = 1.0 - pow(1.0 - 0.5 * nv, 5.0);
    let as_l = 1.0 - pow(1.0 - 0.5 * nl_raw, 5.0);
    let ashikhmin = s.albedo * one_minus_f0 * (as_v * as_l * 0.387508) * nl;
    let power = exp(11.0001 - 11.0 * s.rough);
    let h = normalize(s.v + l);
    let nh = min(saturate(dot(s.n, h)) + 0.0001, 1.0);
    let vh = dot(s.v, h);
    let d = (power + 1.0) * 0.039789 * pow(nh, power);
    let vis = 1.0 / (saturate(max(nv, nl_raw) * vh) + 0.0013);
    let f = s.f0 + one_minus_f0 * s.fresnel_strength * pow(saturate(1.0 - vh), 5.0);
    return mix(lambert, ashikhmin, s.car) + d * vis * f * nl;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let s = read_surface(in.position);

    // Key light, dimmed in occluded areas: the CSM term × (1 - 0.8 × (1 - AO)).
    let l = -lighting.key_direction.xyz;
    let shadow = game_shadow(s.world, s.n, l);
    let key = lighting.key_colour.rgb * view.exposure * shadow * saturate(1.0 - 0.8 * (1.0 - s.ao));
    var colour = key * surface_response(s, l);

    // Ambient: irradiance cube around the normal × AO × (1 - F0) × albedo.
    let irr = textureSampleLevel(irradiance, irradiance_sampler, s.n, 0.0).rgb * lighting.params.x * view.exposure;
    colour += irr * s.ao * (1.0 - s.f0) * s.albedo;

    // Indirect light from the G-buffer pass (sky reflection, lightmaps, lit windows), already exposed.
    colour += s.indirect;

    return vec4(apply_fog(colour, s.world, l), 1.0);
}

// The game's point, cone and spot lights (`mwtool shader-asm LightBuffer_Point` / `_Cone` / `_Spot`, the
// programs with the specular term): additive passes over the light buffer (blend ONE, ONE), one light
// volume each. Per pixel:
//   L     = light position − pixel, d² = |L|² + 0.001
//   atten = saturate(1 / d² − deferred_lightColour.w)
//   cone  = saturate((L/|L| · deferred_lightDirection.xyz − deferred_lightPosition.w) × deferred_lightDirection.w)
//           (cone and spot lights only)
//   out   = deferred_lightColour.rgb × atten × cone × surface_response
// No shadow, no AO. Here every light is one instance of a cube around its range, drawn with the back
// faces only (so the camera may be inside) at the depth of the lighting pass ID, as the full-screen pass.
// The game draws its Fog pass after the lights; here the key-light pass has applied the fog already, so
// a light's contribution is scaled by the fog's transmittance (1 − t), which is what the Fog pass's
// `scene × (1 − t)` does to it.
struct LightVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

@vertex
fn light_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> LightVertexOutput {
    // Cube corners (bit 0 = x, 1 = y, 2 = z), two triangles per face, counter-clockwise from outside.
    var corners = array<u32, 36>(
        1u, 3u, 7u, 1u, 7u, 5u, 0u, 4u, 6u, 0u, 6u, 2u, 2u, 6u, 7u, 2u, 7u, 3u,
        0u, 1u, 5u, 0u, 5u, 4u, 4u, 5u, 7u, 4u, 7u, 6u, 0u, 2u, 3u, 0u, 3u, 1u,
    );
    let c = corners[vertex_index];
    let corner = vec3<f32>(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u)) * 2.0 - 1.0;
    let light = world_lights[instance];
    let world = light.position.xyz + corner * light.position.w;
    var clip = view.clip_from_world * vec4(world, 1.0);
    clip.z = PASS_ID / 255.0 * clip.w;
    return LightVertexOutput(clip, instance);
}

@fragment
fn light_fragment(in: LightVertexOutput) -> @location(0) vec4<f32> {
    let light = world_lights[in.index];
    let s = read_surface(in.position);
    let to_light = light.position.xyz - s.world;
    let d2 = dot(to_light, to_light) + 0.001;
    var atten = saturate(1.0 / d2 - light.colour.w);
    let l = to_light / sqrt(d2);
    if light.cone.y > 0.5 {
        atten *= saturate((dot(l, light.direction.xyz) - light.direction.w) * light.cone.x);
    }
    if atten <= 0.0 {
        discard;
    }
    let colour = light.colour.rgb * atten * view.exposure * surface_response(s, l);
    return vec4(colour * (1.0 - fog_amount(s.world)), 0.0);
}

// The game's Fog pass (`mwtool shader-asm Fog`, ps_4_0 of 01010008ce7671f3), a full-screen quad over the
// lit scene with premultiplied output (rgb = gradient × alpha + scatter, alpha = coverage):
//   d     = |camera → pixel| in metres (the VS hands over a ray scaled by the projection's z terms, the
//           PS divides it by (depth − P22): view-space reconstruction, so world metres)
//   t     = saturate(d × ScattCoeffs.x − ScattCoeffs.y)^ScattCoeffs.z, ScattCoeffs = (1 / (far − near),
//           near / (far − near), Power) from the keyframe's NearDistance / FarDistance (metres) / Power
//   t'    = t × GradientTextureA.a (a 1D texture sampled at the screen u; its alpha caps the opacity)
//   phase = saturate(ScatteringSettings.y / (1 + ScatteringSettings.x × dot(dir, SunCoronaDirection))²)
//   out   = scene × (1 − t') × layer + gradient.rgb × (1 − (1 − t') × layer) + t' × phase × ScatteringColor
// ScatteringSettings = (g, (1 − g²) × strength / 4π) and SunCoronaDirection = −(state +0xeb0), written per
// frame in the light-buffer setup (docs/decomp.md "Fog"). Not ported: `layer` = exp(−|d × (C.z / (1 +
// exp(C.x × Δy + C.y)) − C.w) / Δy|) with C = LayerFogCoeffs (Δy = the ray's height change), whose CPU
// source is not traced (1 here, as with LayerFogCoeffs = 0); the gradient texture, whose builder is not
// traced: its rgb is approximated by blending Colour → ColourSun with the view azimuth's closeness to
// the sun's, its alpha is taken as 1 (Colour.w and ColourSun[3] are 1 in the data).
fn fog_amount(world: vec3<f32>) -> f32 {
    let d = length(world - view.world_position.xyz);
    let c = lighting.fog_coeffs;
    return pow(saturate(d * c.x - c.y), c.z);
}

fn apply_fog(colour: vec3<f32>, world: vec3<f32>, to_sun: vec3<f32>) -> vec3<f32> {
    let rel = world - view.world_position.xyz;
    let d = length(rel);
    let dir = rel / max(d, 1e-4);
    let t = fog_amount(world);
    let toward = pow(saturate(dot(normalize(vec3(dir.x, 0.0, dir.z)), normalize(vec3(to_sun.x, 0.0, to_sun.z)))), 4.0);
    let fog = mix(lighting.fog_colour.rgb, lighting.fog_colour_sun.rgb, toward) * view.exposure;
    let cos_corona = dot(dir, -to_sun);
    let phase = saturate(lighting.scattering.w / pow(1.0 + lighting.scattering_params.x * cos_corona, 2.0));
    let scatter = t * phase * lighting.scattering.rgb * view.exposure;
    return colour * (1.0 - t) + fog * t + scatter;
}

// The game's cascaded shadow lookup (`LightBuffer_KeylightAndAmbient`): the first cascade whose light-space
// xy lies inside ±0.99 is sampled with one hardware 2×2 PCF tap (`sample_c_lz`); points outside every
// cascade compare against depth 0 and are lit. The result fades to `params.w` with view depth as
// saturate(C.w - depth × C2.w)², then the blended value is squared.
// Bevy renders the cascades (one array layer each instead of the game's vertical atlas). The game biases
// casters per material (`MaterialShadowMapBias`); Bevy's normal and depth bias on the receiver stand in.
fn game_shadow(world: vec3<f32>, n: vec3<f32>, to_light: vec3<f32>) -> f32 {
    if lights.n_directional_lights == 0u {
        return 1.0;
    }
    let light = &lights.directional_lights[0];
    if ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    var s = 1.0;
    for (var i = 0u; i < (*light).num_cascades; i += 1u) {
        let cascade = &(*light).cascades[i];
        let offset = (*light).shadow_normal_bias * (*cascade).texel_size * n + (*light).shadow_depth_bias * to_light;
        let clip = (*cascade).clip_from_world * vec4(world + offset, 1.0);
        let ndc = clip.xyz / clip.w;
        if max(abs(ndc.x), abs(ndc.y)) < 0.99 && ndc.z >= 0.0 && ndc.z <= 1.0 {
            let uv = ndc.xy * vec2(0.5, -0.5) + 0.5;
            let layer = i32((*light).depth_texture_base_index + i);
            s = textureSampleCompareLevel(
                directional_shadow_textures, directional_shadow_textures_comparison_sampler, uv, layer, ndc.z
            );
            break;
        }
    }
    let forward = -view.world_from_view[2].xyz;
    let depth = dot(world - view.world_position.xyz, forward);
    var fade = saturate(lighting.params.z - depth * lighting.params.y);
    fade *= fade;
    let blended = mix(lighting.params.w, s, fade);
    return blended * blended;
}
