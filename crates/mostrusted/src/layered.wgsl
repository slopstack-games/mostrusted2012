// The game's shader families, ported from their DXBC G-buffer passes (see `mwtool shader-asm`): albedo,
// normal, roughness, F0, Fresnel strength, AO and the indirect term as the game writes them.
//
// Opaque materials run in the deferred prepass and write the game's four G-buffer targets packed into
// Bevy's 128-bit deferred texel (`game_gbuffer`); `deferred.rs` lights them with the port of the game's
// `LightBuffer_KeylightAndAmbient` pass. Translucent materials run the forward branch, which still uses
// Bevy's PBR lighting.
//
// mode 0: plain StandardMaterial
// mode 1: DriveableSurface   colour(uv0) * detail(world.xz * DetailMapUvScale) * 2, slope-space normals
// mode 2: TerrainPBR         mask/noise driven blend of three tiling layers with their normal maps
// mode 3: PlotPBR            mask-weighted tinted tiling layers + decal, blended tiling normals, AO map
// mode 4: road line          road (mode 1) with a tinted line texture over it
// mode 5: WorldPBR           Bevy base colour + normal map, specular map -> roughness / F0, AO map
// mode 6: vehicle parts      F0 / roughness from textures or constants, vertex AO (UV_1), clear coat
// mode 7: trees              premultiplied texel: albedo = (rgb / a)^2, vertex AO, no specular
// mode 8: water              two scrolling wave normal maps, algae albedo, near-mirror reflection
// mode 9: vehicle lamp interior (Refraction): parallax-offset reflector, LED emissive channels
// mode 10: vehicle glass     forward, premultiplied: tinted volume + Fresnel sky reflection
// mode 11: vehicle lens filter (Glass_Colourise): multiply blend

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    pbr_types::PbrInput,
    mesh_view_bindings::view,
}
#import bevy_render::globals::Globals

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{VertexOutput, FragmentOutput}
#ifdef MOTION_VECTOR_PREPASS
#import bevy_pbr::pbr_prepass_functions::calculate_motion_vector
#endif
// The prepass view layout has the globals at binding 1 (the main pass has them at 11).
@group(0) @binding(1) var<uniform> prepass_globals: Globals;
fn time() -> f32 {
    return prepass_globals.time;
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::globals,
}
fn time() -> f32 {
    return globals.time;
}
#endif

struct Layered {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    // road:    p0 = (has road + detail normal maps, has SpecColour, has line normal, 0),
    //          p1.xy = DetailMapUvScale, p2 = Line_Diffuse, p3 = Line_Roughness_Specular_Fresnel
    //          textures: colour, detail, line, normal, detail normal, SpecColour, line normal+spec
    // terrain: p0 = Tiling_Ratios_X (w = noise ratio), p1 = Tiling_Ratios_Y, p2..p4 = Layer RSF,
    //          p5 = (has world-space macro normal, has tiling normals)
    //          textures: mask, noise, layer 1..3, macro normal, tiling normal 1..3
    // plot:    p0/p1 = Tiling_Ratios_X/Y, p2 = (maskTilingRatios.xy, decalTilingRatios.xy),
    //          p3..p5 = tiling1..3Diffuse, p6 = decal_Diffuse (w = strength), p7..p9 = Layer RSF,
    //          p10 = (has tiling normals, has AO map, instance AO, has decal)
    //          textures: mask, layer 1..3, decal, tiling normal 1..3, AO map
    // WorldPBR: p0 = (has SpecularColour map, instance AO scale, has AO map, planar only), p1 = MaterialDiffuse
    // vehicle: p0 = (F0 rgb, roughness), p1 = (has spec map, roughness source: 0 constant, 1 normal
    //          alpha, 2 normal green, coat F0, coat fresnel), p2 = (has vertex colour (x = AO,
    //          y = specular occlusion), has diffuse map, lightmapped-lights kind, multiplier),
    //          p3 = (albedo rgb, lightmap reads UV_1 = TEXCOORD3), p4..p7 = red / green / blue / alpha
    //          channel colours; textures: diffuse, specular, normal, lightmap lights
    // refraction: p0 = ParallaxConstants, p1 = (EmissiveLuminance, ClearcoatSpecular, PbrMaterialFresnel),
    //          p2 = PbrMaterialDiffuseColour, p3 = mExternalGlassColour, p4..p7 = brake / headlight /
    //          reversing / taillight colours; textures: colour, external normal, emissive, internal
    //          normal, displacement
    // glass:   p0 = (OpacityMin, FresnelFactor, SurfaceSoftness, mSelfIlluminationMultiplier),
    //          p1 = DebugOverride_GlassVolumeColour, p2 = RunningColour, p3 = (has diffuse, has lightmap,
    //          has vertex colour); textures: diffuse, lightmap
    // colourise: p0 = MaterialColour_SimpleMultiply
    // tree:    p0 = (has vertex colour, instance AO, 0, 0)
    // water:   p0 = (normalMapTile1.xy, normalMapTile2.xy), p1 = (waveAnimationSpeeds.xy, F0, roughness),
    //          p2 = algaeColour, p3 = (opaque variant, has normal map); textures: -, normal,
    //          DiffuseAndCaustics, SpecDepthAndRefraction
    // RSF = Roughness_Specular_Fresnel (x = roughness scale, y = F0, z = Fresnel strength)
    p: array<vec4<f32>, 12>,
    // x = sky brightness (cd/m² per panorama unit), y = reflection cube mip count
    env: vec4<f32>,
    // vehicle light state (g_selfIlluminationMask): x = brake, y = headlights / running, z = reverse, w = tail
    lights: vec4<f32>,
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
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var tex5: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var samp5: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var tex6: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(114) var samp6: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(115) var tex7: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(116) var samp7: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(117) var tex8: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(118) var samp8: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(119) var tex9: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var samp9: sampler;
// The prefiltered sky reflection cube (mip = roughness × 5.5), as the game's G-buffer passes read it.
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var sky_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var sky_samp: sampler;
// The planar reflection (`reflection.rs`): the scene mirrored in y = view.color_grading.hue, HDR game units.
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var planar_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(124) var planar_samp: sampler;

// The wet roads' reflection at a road point: the planar reflection at the point's mirrored projection,
// shifted by the normal's deviation from the vertex normal (WetScaleParams.y), blurred by roughness (the
// game reads a mip chain at roughness × WetScaleParams.z; four taps stand in for it). Without a planar
// reflection (car viewer, `--no-reflection`) the sky cube stands in. cd/m².
fn wet_reflection(p: vec3<f32>, n: vec3<f32>, nv: vec3<f32>, v: vec3<f32>, rough: f32, ws: vec4<f32>) -> vec3<f32> {
#ifndef PLANAR_REFLECTION_VIEW
    if layered.env.w > 0.5 {
        let h = view.color_grading.hue;
        let clip = view.clip_from_world * vec4(p.x, 2.0 * h - p.y, p.z, 1.0);
        let uv = clip.xy / clip.w * vec2(0.5, -0.5) + 0.5 + (n - nv).xz * ws.y;
        let r = rough * 0.02;
        var c = vec3(0.0);
        c += textureSampleLevel(planar_tex, planar_samp, uv + vec2(r, 0.5 * r), 0.0).rgb;
        c += textureSampleLevel(planar_tex, planar_samp, uv + vec2(-0.5 * r, r), 0.0).rgb;
        c += textureSampleLevel(planar_tex, planar_samp, uv + vec2(-r, -0.5 * r), 0.0).rgb;
        c += textureSampleLevel(planar_tex, planar_samp, uv + vec2(0.5 * r, -r), 0.0).rgb;
        return c * 0.25 * layered.env.x;
    }
#endif
    let lod = min(rough * ws.z, layered.env.y - 1.0);
    return textureSampleLevel(sky_cube, sky_samp, reflect(-v, n), lod).rgb * layered.env.x;
}

// The G-buffer pass turns the specular map's gamma-space reflectance into linear F0 with this cubic.
fn gamma_to_f0(s: vec3<f32>) -> vec3<f32> {
    return saturate(((0.307841 * s + 0.686033) * s + 0.006163) * s + 0.001527);
}

// Bevy derives F0 as 0.16 * reflectance^2.
fn reflectance_for_f0(f0: vec3<f32>) -> vec3<f32> {
    return sqrt(f0 / 0.16);
}

fn set_specular(pbr_input: ptr<function, PbrInput>, f0: vec3<f32>, rough: f32) {
    (*pbr_input).material.reflectance = reflectance_for_f0(f0);
    (*pbr_input).material.perceptual_roughness = clamp(rough, 0.045, 1.0);
    (*pbr_input).material.metallic = 0.0;
}

fn set_ao(pbr_input: ptr<function, PbrInput>, ao: f32) {
    (*pbr_input).diffuse_occlusion *= vec3(ao);
    (*pbr_input).specular_occlusion *= ao * ao;
}

// The game's env-BRDF LUT (sampled at N·V, roughness, F0) approximated analytically (Karis 2014):
// returns (scale on F0, bias); the game's form is lut.x * (F0 + (1 - F0) * fresnel * lut.y).
fn env_brdf(rough: f32, nv: f32) -> vec2<f32> {
    let c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    return vec2(-1.04, 1.04) * a004 + r.zw;
}

// Sky reflection as the G-buffer passes compute it into RT3: cube(R, roughness × 5.5) × env-BRDF.
fn sky_reflection(n: vec3<f32>, v: vec3<f32>, rough: f32, f0: vec3<f32>, fresnel: f32) -> vec3<f32> {
    let r = reflect(-v, n);
    let lod = min(rough * 5.5, layered.env.y - 1.0);
    let sky = textureSampleLevel(sky_cube, sky_samp, r, lod).rgb * layered.env.x;
    let ab = env_brdf(rough, saturate(dot(n, v)));
    return sky * (f0 * ab.x + fresnel * ab.y);
}

// TerrainPBR layer weights: overlay of the mask channels with a noise texture.
fn terrain_weights(m: vec4<f32>, n: f32) -> vec3<f32> {
    var w: vec3<f32>;
    w.x = select(2.0 * n * m.r, 1.0 - 2.0 * (1.0 - m.r) * (1.0 - n), m.r > 0.5);
    w.z = select(2.0 * (1.0 - n) * m.b, 1.0 - 2.0 * n * (1.0 - m.b), m.b > 0.5);
    w.y = 1.0 - (m.r + m.b);
    w = saturate(w);
    return w / max(dot(w, vec3(1.0)), 1e-4);
}

// DriveableSurface normal maths: a tangent-space texel turned into a slope (dx/dy, dz/dy) relative to
// a horizontal road, using only the horizontal parts of the vertex tangent and bitangent.
fn road_slope(texel: vec3<f32>, t: vec3<f32>, b: vec3<f32>) -> vec2<f32> {
    let c = 2.0 * (texel - 0.5);
    let v = c.y * vec3(b.x, 0.0, b.z) + c.z * vec3(0.0, 1.0, 0.0) + c.x * vec3(t.x, 0.0, t.z);
    return v.xz / max(v.y, 0.05);
}

// The detail normal map is projected in world space with its texel swizzled to (x, z, -y).
fn detail_slope(texel: vec3<f32>) -> vec2<f32> {
    let c = 2.0 * (texel - 0.5);
    return vec2(c.x, -c.y) / max(c.z, 0.05);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef PLANAR_REFLECTION_VIEW
    // The mirrored view only shows what is above the reflection plane (`reflection.rs`).
    if in.world_position.y < view.color_grading.hue - 0.05 {
        discard;
    }
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    var uv_b = uv;
#ifdef VERTEX_UVS_B
    uv_b = in.uv_b;
#endif
    // Vertex frame (world space): normal, tangent, bitangent = sign * cross(N, T).
    let nv = normalize(pbr_input.world_normal);
    var tv = vec3(1.0, 0.0, 0.0);
    var bv = vec3(0.0, 0.0, 1.0);
#ifdef VERTEX_TANGENTS
    tv = normalize(in.world_tangent.xyz);
    bv = cross(nv, tv) * select(-1.0, 1.0, in.world_tangent.w >= 0.0);
#endif

    let v_dir = normalize(view.world_position.xyz - in.world_position.xyz);
    // G-buffer extras the PBR input has no slot for: RT2.w Fresnel strength, RT3.w vehicle flag (selects
    // the lighting pass's vehicle diffuse model), and indirect light beyond the sky reflection.
    var fresnel = 1.0;
    var car = 0.0;
    var extra = vec3(0.0);

    let tx = layered.p[0];
    let ty = layered.p[1];
    if layered.mode == 1u || layered.mode == 4u {
        let world_uv = in.world_position.xz * ty.xy;
        // FloatingDecal_1Bit: discard where EdgeAlphaPlusAoMap(uv0).y < 0.5.
        if layered.mode == 1u && layered.p[2].x > 0.5 && textureSample(tex2, samp2, uv).y < 0.5 {
            discard;
        }
        let colour = textureSample(tex0, samp0, uv).rgb;
        let detail = textureSample(tex1, samp1, world_uv).rgb;
        var albedo = colour * detail * 2.0;
        let flags = layered.p[0];
        var rough = 1.0;
        var f0 = 0.02;
        var line_a = 0.0;
        if layered.mode == 4u {
            let line = textureSample(tex2, samp2, uv_b) * layered.p[2];
            line_a = line.a;
            albedo = mix(albedo, line.rgb, line_a);
        }
        var n_road = nv;
        if flags.x > 0.5 {
            // Slope sum of road map, detail map and vertex normal, then (p.x, 1, p.y) normalised.
            let road = textureSample(tex3, samp3, uv);
            let det = textureSample(tex4, samp4, world_uv);
            var p = road_slope(road.rgb, tv, bv) + detail_slope(det.rgb) + nv.xz / max(nv.y, 0.05);
            if flags.z > 0.5 {
                p += road_slope(textureSample(tex6, samp6, uv_b).rgb, tv, bv) * line_a;
            }
            n_road = normalize(vec3(p.x, 1.0, p.y));
            rough = road.a * det.a;
        }
        if flags.y > 0.5 {
            f0 = textureSample(tex5, samp5, uv).g;
        }
        if layered.mode == 4u {
            let rsf = layered.p[3];
            rough = mix(rough, rsf.x, line_a);
            f0 = mix(f0, rsf.y, line_a);
            fresnel = mix(1.0, rsf.z, line_a);
        }
        // The wet path (DriveableSurface main pass): wet = puddle.w × saturate(TEXCOORD5.x); in a puddle
        // the normal falls back to the vertex normal by RoadDepth.y × puddle.x, roughness scales by
        // puddle.y, albedo darkens by puddle.z and F0 goes to water's 0.02. The only indirect light on a
        // road is the planar reflection × env-BRDF × wet (none on dry roads); the sky cube stands in for
        // the planar reflection texture.
        var wet_in = 0.0;
        var ao = 1.0;
#ifdef VERTEX_COLORS
        wet_in = in.color.x;
        ao = in.color.y;
#endif
        var n = n_road;
        if flags.w > 0.5 {
            let ws = layered.p[4];
            let puddle = textureSample(tex8, samp8, in.world_position.xz * ws.x);
            var depth = 0.0;
            if flags.w > 1.5 {
                depth = textureSample(tex7, samp7, uv).y;
            }
            let wet = saturate(puddle.w * wet_in * layered.env.z);
            let k = mix(vec3(1.0), vec3(depth * puddle.x, puddle.y, puddle.z), wet);
            n = normalize(nv + k.x * (n_road - nv));
            rough *= k.y;
            albedo *= k.z;
            f0 = mix(0.02, f0, k.x);
            let ab = env_brdf(rough, saturate(dot(n, v_dir)));
            let strength = (f0 * ab.x + ab.y) * wet;
            extra += wet_reflection(in.world_position.xyz, n, nv, v_dir, rough, ws) * strength;
        }
        pbr_input.N = n;
        pbr_input.material.base_color = vec4(albedo, 1.0);
        set_specular(&pbr_input, vec3(f0), rough);
        pbr_input.diffuse_occlusion = vec3(ao);
        // No sky-cube reflection on roads (only the wet planar term in `extra`).
        pbr_input.specular_occlusion = 0.0;
    } else if layered.mode == 2u {
        let m = textureSample(tex0, samp0, uv);
        let n = textureSample(tex1, samp1, uv_b * tx.w).g;
        let w = terrain_weights(m, n);
        let uv1 = uv_b * vec2(tx.x, ty.x);
        let uv2 = uv_b * vec2(tx.y, ty.y);
        let uv3 = uv_b * vec2(tx.z, ty.z);
        let l1 = textureSample(tex2, samp2, uv1).rgb;
        let l2 = textureSample(tex3, samp3, uv2).rgb;
        let l3 = textureSample(tex4, samp4, uv3).rgb;
        let albedo = (l1 * w.x + l2 * w.y + l3 * w.z) * (2.0 * m.g);
        pbr_input.material.base_color = vec4(albedo, 1.0);
        // Layer constants blended by the same weights: x = roughness scale, y = F0.
        let c = layered.p[2] * w.x + layered.p[3] * w.y + layered.p[4] * w.z;
        let flags = layered.p[5];
        var rough = c.x;
        if flags.y > 0.5 {
            // Tiling normals: x, y and roughness in w; z is reconstructed.
            let nt = textureSample(tex6, samp6, uv1).xyw * w.x + textureSample(tex7, samp7, uv2).xyw * w.y
                + textureSample(tex8, samp8, uv3).xyw * w.z;
            let t2 = 2.0 * (nt.xy - 0.5);
            let tz = sqrt(max(0.0, 1.0 - dot(t2, t2)));
            // Macro frame: the world-space macro normal (texel swizzled to x, z, -y) or the vertex normal.
            var nm = nv;
            if flags.x > 0.5 {
                let mt = 2.0 * (textureSample(tex5, samp5, uv).rgb - 0.5);
                nm = normalize(vec3(mt.x, mt.z, -mt.y));
            }
            let bm = normalize(cross(nm, tv));
            let tm = normalize(cross(bm, nm));
            pbr_input.N = normalize(t2.y * bm + tz * nm + t2.x * tm);
            rough *= nt.z;
        }
        set_specular(&pbr_input, vec3(c.y), rough);
        set_ao(&pbr_input, saturate(2.0 * m.g));
        fresnel = c.z;
    } else if layered.mode == 3u {
        let md = layered.p[2];
        let m = textureSample(tex0, samp0, uv * md.xy).rgb;
        let w = m / max(dot(m, vec3(1.0)), 1e-4);
        let uv1 = uv * vec2(tx.x, ty.x);
        let uv2 = uv * vec2(tx.y, ty.y);
        let uv3 = uv * vec2(tx.z, ty.z);
        let l1 = textureSample(tex1, samp1, uv1).rgb * layered.p[3].rgb;
        let l2 = textureSample(tex2, samp2, uv2).rgb * layered.p[4].rgb;
        let l3 = textureSample(tex3, samp3, uv3).rgb * layered.p[5].rgb;
        var albedo = l1 * w.x + l2 * w.y + l3 * w.z;
        let flags = layered.p[10];
        // A missing decal reads 0 in the game (unbound slot).
        var d = vec4(0.0);
        if flags.w > 0.5 {
            d = textureSample(tex4, samp4, uv * md.zw);
        }
        let dt = layered.p[6];
        albedo = mix(albedo, d.x * dt.rgb, d.y * dt.w);
        pbr_input.material.base_color = vec4(albedo, 1.0);
        let c = layered.p[7] * w.x + layered.p[8] * w.y + layered.p[9] * w.z;
        var nt_a = 1.0;
        if flags.x > 0.5 {
            let nt = textureSample(tex5, samp5, uv1) * w.x + textureSample(tex6, samp6, uv2) * w.y
                + textureSample(tex7, samp7, uv3) * w.z;
            let t = 2.0 * (nt.rgb - 0.5);
            pbr_input.N = normalize(t.x * tv + t.y * bv + t.z * nv);
            nt_a = nt.a;
        }
        // Roughness = lerp(decal.z, 1, X * normal alpha), F0 = blended layer specular.
        let k = c.x * nt_a;
        set_specular(&pbr_input, vec3(c.y), mix(d.z, 1.0, k));
        var ao = flags.z;
        if flags.y > 0.5 {
            ao *= textureSample(tex8, samp8, uv_b).y;
        }
        set_ao(&pbr_input, ao);
        // Like roads, plots have no cube reflection, only the wet planar one.
        pbr_input.specular_occlusion = 0.0;
    } else if layered.mode == 5u {
        // WorldPBR G-buffer pass: o0.w = spec.y (roughness; the reflection mip is spec.y * 5.5),
        // o2 = F0 = cubic(spec.x), or SpecularColour.rgb * MaterialDiffuse for ColouredSpecular,
        // o1.w = AO map(uv1).y * instance AO.
        let p = layered.p[0];
        let spec = textureSample(tex1, samp1, uv);
        var f0 = gamma_to_f0(spec.xxx);
        if p.x > 0.5 {
            f0 = textureSample(tex3, samp3, uv).rgb * layered.p[1].rgb;
        }
        set_specular(&pbr_input, f0, spec.y);
        var ao = p.y;
        if p.z > 0.5 {
            ao *= textureSample(tex2, samp2, uv_b).y;
        }
        set_ao(&pbr_input, ao);
        if p.w > 0.5 {
            pbr_input.specular_occlusion = 0.0;
        }
    } else if layered.mode == 6u {
        // VehicleNFS13 G-buffer pass: o1 = albedo (diffuse texture or PbrMaterialDiffuseColour),
        // o2 = F0 (spec map rgb or PbrMaterialSpecularColour) * vertex y, o0.w = roughness
        // (PbrMaterialRoughness, else the normal map's alpha, or its green in the single-channel
        // Emissive variant), o1.w = vertex x (AO). The coat reflects with F0 = ClearcoatSpecular, rising
        // towards 1 at grazing angles by ClearcoatFresnel. The vertex colour is read raw here (Bevy has
        // multiplied it into its base colour), so the albedo is rebuilt from the material.
        let p0 = layered.p[0];
        let p1 = layered.p[1];
        let p2 = layered.p[2];
        let p3 = layered.p[3];
        var albedo = vec4(p3.rgb, 1.0);
        if p2.y > 0.5 {
            albedo = textureSample(tex0, samp0, uv) * vec4(p3.rgb, 1.0);
        }
        pbr_input.material.base_color = albedo;
        var f0 = p0.rgb;
        var rough = p0.w;
        if p1.x > 0.5 {
            f0 = textureSample(tex1, samp1, uv).rgb;
        }
        if p1.y > 0.5 {
            let nt = textureSample(tex2, samp2, uv);
            rough = select(nt.a, nt.y, p1.y > 1.5);
        }
        var ao = vec2(1.0);
#ifdef VERTEX_COLORS
        if p2.x > 0.5 {
            ao = in.color.xy;
        }
#endif
        set_specular(&pbr_input, f0 * ao.y, rough);
        pbr_input.diffuse_occlusion *= vec3(ao.x);
        pbr_input.specular_occlusion *= ao.y;
        car = 1.0;
#ifdef PREPASS_PIPELINE
        // Clear coat: reflectance lerp(F0, F0 + (1 - F0) * fresnel, (1 - N·V)^5) × vertex y; the base layer
        // is scaled by (1 - coat) and the coat's sharp sky reflection goes into the indirect term.
        let nv = saturate(dot(pbr_input.N, v_dir));
        let schlick = pow(1.0 - nv, 5.0);
        let grazing = p1.z + (1.0 - p1.z) * p1.w;
        let coat = mix(p1.z, grazing, schlick) * ao.y;
        pbr_input.material.base_color = vec4(pbr_input.material.base_color.rgb * (1.0 - coat), pbr_input.material.base_color.a);
        pbr_input.material.reflectance *= sqrt(1.0 - coat);
        let r = reflect(-v_dir, pbr_input.N);
        extra += textureSampleLevel(sky_cube, sky_samp, r, 0.0).rgb * layered.env.x * coat;
#endif
        // Lightmapped lights (LightmapLightsTextureSampler), switched by the light state mask:
        // four channels sat(texel² × mask.xyzy) × multiplier against the red / green / blue / alpha
        // colours, or the green channel alone (the `Lightmap` variants also tint it by the albedo).
        if p2.z > 0.5 {
            let t = textureSample(tex3, samp3, select(uv, uv_b, p3.w > 0.5));
            let m = layered.lights;
            var emit: vec3<f32>;
            if p2.z < 1.5 {
                let e = saturate(t * t * m.xyzy) * p2.w;
                emit = e.x * layered.p[4].rgb + e.y * layered.p[5].rgb + e.z * layered.p[6].rgb
                    + e.w * layered.p[7].rgb;
            } else {
                emit = saturate(t.y * t.y * m.y) * p2.w * layered.p[5].rgb;
                if p2.z > 2.5 {
                    emit *= pbr_input.material.base_color.rgb;
                }
            }
            emit *= layered.env.x;
            extra += emit;
            pbr_input.material.emissive = vec4(emit, 1.0);
        }
    } else if layered.mode == 9u {
        // VehicleNFS13_Refraction (lamp interiors), a G-buffer pass despite the name. The UV is offset
        // along the eye ray in tangent space, bent towards the external lens normal by
        // ParallaxConstants.z, by a height of displacement.x × P.x + P.y. At the offset UV: internal
        // normal map → normal, displacement.z → roughness, .y → AO, Colour → F0, Fresnel =
        // lerp(colour.x, 1, PbrMaterialFresnel), albedo = PbrMaterialDiffuseColour. RT3 holds only
        // (sharp sky reflection of the eye ray off the reflector × ClearcoatSpecular + LED emission) ×
        // lerp(colour, mExternalGlassColour, its w) × AO, without the vehicle flag.
        let pc = layered.p[0];
        let k = layered.p[1];
        let i_w = -v_dir;
        let i_ts = vec3(dot(i_w, tv), dot(i_w, bv), dot(i_w, nv));
        let height = textureSample(tex4, samp4, uv).x * pc.x + pc.y;
        let ext = 2.0 * textureSample(tex1, samp1, uv).rgb - 1.0;
        let d = normalize(mix(i_ts, -ext, pc.z));
        let uv2 = uv + vec2(d.x, -d.y) * height;
        let ni = 2.0 * (textureSample(tex3, samp3, uv2).rgb - 0.5);
        let n = normalize(ni.x * tv + ni.y * bv + ni.z * nv);
        let disp = textureSample(tex4, samp4, uv2);
        let colour = textureSample(tex0, samp0, uv2);
        pbr_input.N = n;
        pbr_input.material.base_color = vec4(layered.p[2].rgb, 1.0);
        set_specular(&pbr_input, colour.rgb, disp.z);
        fresnel = mix(colour.x, 1.0, k.z);
        pbr_input.diffuse_occlusion = vec3(disp.y);
        pbr_input.specular_occlusion = 0.0;
        // Emissive channels: brake, headlight, reversing, tail.
        let e = saturate(textureSample(tex2, samp2, uv2) * layered.lights) * k.x;
        let emit = e.x * layered.p[4].rgb + e.y * layered.p[5].rgb + e.z * layered.p[6].rgb + e.w * layered.p[7].rgb;
        let dw = normalize(d.x * tv + d.y * bv + d.z * nv);
        let refl = textureSampleLevel(sky_cube, sky_samp, reflect(dw, n), 0.0).rgb;
        let g = layered.p[3];
        extra = (refl * k.y + emit) * layered.env.x * mix(colour.rgb, g.rgb, g.w) * disp.y;
        pbr_input.material.emissive = vec4(extra, 1.0);
    } else if layered.mode == 10u {
        // VehicleNFS13_Glass: the volume colour is the vertex colour (Textured: lerp(vertex colour,
        // diffuse, diffuse.a)), replaced by DebugOverride_GlassVolumeColour when its w > 0. Composited
        // in the forward branch below.
        let flags = layered.p[3];
        var vol = vec4(0.0);
#ifdef VERTEX_COLORS
        if flags.z > 0.5 {
            vol = in.color;
        }
#endif
        if flags.x > 0.5 {
            let dt = textureSample(tex0, samp0, uv);
            vol = mix(vol, dt, dt.a);
        }
        if layered.p[1].w > 0.0 {
            vol = layered.p[1];
        }
        pbr_input.material.base_color = vol;
        set_specular(&pbr_input, vec3(0.0), 1.0);
        pbr_input.specular_occlusion = 0.0;
        if flags.y > 0.5 {
            // Running light: lightmap.y × mask.y × mSelfIlluminationMultiplier × RunningColour.
            let lm = textureSample(tex1, samp1, uv).y;
            extra = lm * layered.lights.y * layered.p[0].w * layered.p[2].rgb * layered.env.x;
        }
    } else if layered.mode == 7u {
        // The G-buffer stores rgb / a without the sqrt, so albedo = (rgb / a)^2 with rgb in gamma
        // space; Bevy has already decoded rgb to linear (~ rgb^2), so divide by a^2.
        let base = pbr_input.material.base_color;
        let a = max(base.a, 0.5);
        pbr_input.material.base_color = vec4(min(base.rgb / (a * a), vec3(1.0)), base.a);
        set_specular(&pbr_input, vec3(0.0), 1.0);
        var ao = layered.p[0].y;
        if layered.p[0].x > 0.5 {
            ao *= uv_b.y;
        }
        pbr_input.diffuse_occlusion *= vec3(ao);
        pbr_input.specular_occlusion *= ao;
    }

    else if layered.mode == 8u {
        let t = time();
        let tiles = layered.p[0];
        let p1 = layered.p[1];
        let flags = layered.p[3];
        let w1 = uv_b * tiles.xy + vec2(0.0, t / p1.x);
        let w2 = uv_b * tiles.zw + vec2(0.0, t / p1.y);
        if flags.y > 0.5 {
            let nt = 0.5 * (textureSample(tex1, samp1, w1).rgb + textureSample(tex1, samp1, w2).rgb);
            let c = 2.0 * (nt - 0.5);
            pbr_input.N = normalize(c.x * tv + c.y * bv + c.z * nv);
        }
        var albedo = vec3(0.0);
        var rough = p1.w;
        var refl = 1.0;
        if flags.x > 0.5 {
            // s: x = roughness scale, y = depth (floor visibility, damps the reflection), z = algae.
            let s = textureSample(tex3, samp3, uv);
            let base = textureSample(tex2, samp2, uv).rgb;
            // The river floor (TEXCOORD2) is not available here: a mid grey stands in for it.
            albedo = mix(saturate(vec3(0.35) * s.y) * base, layered.p[2].rgb, s.z);
            rough *= s.x;
            refl = 1.0 - s.y;
        }
        pbr_input.material.base_color = vec4(albedo, 1.0);
        // The game's reflection Fresnel lookup reads dot(-V, N) (negative on visible water), so it
        // always takes the grazing-angle value: a near-mirror. A high F0 stands in for that.
        set_specular(&pbr_input, vec3(max(p1.z, 0.6)), rough);
        pbr_input.specular_occlusion *= refl;
    }

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
#ifdef PREPASS_PIPELINE
    out.deferred = game_gbuffer(pbr_input, v_dir, fresnel, car, extra);
    out.deferred_lighting_pass_id = GAME_LIGHTING_PASS_ID;
#ifdef NORMAL_PREPASS
    out.normal = vec4(in.world_normal * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
#else
    if layered.mode == 11u {
        // VehicleNFS13_Glass_Colourise: multiply blend by lerp(1, MaterialColour, fog.w = 1).
        out.color = vec4(layered.p[0].rgb, 1.0);
    } else if layered.mode == 10u {
        out.color = main_pass_post_lighting_processing(pbr_input, vehicle_glass(pbr_input, v_dir, extra));
    } else {
        out.color = apply_pbr_lighting(pbr_input);
        out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    }
#endif
    return out;
}

#ifndef PREPASS_PIPELINE
// VehicleNFS13_Glass forward pass (clean, dry, uncracked path), premultiplied:
//   F = OpacityMin + (1 - OpacityMin) × FresnelFactor × (1 - N·V)^5
//   colour = volume.rgb × volume.a × diffuse light × (1 - F) + lerp(cube(R), blurred cube(R), SurfaceSoftness) × F
//   alpha = volume.a × (1 - F) + F, plus the running light added on top.
// The game's diffuse light is key × N·L × shadow / π + irradiance cube; Bevy's lighting stands in for it.
fn vehicle_glass(pbr_input: PbrInput, v: vec3<f32>, emit: vec3<f32>) -> vec4<f32> {
    let g = layered.p[0];
    let vol = pbr_input.material.base_color;
    var lit_input = pbr_input;
    lit_input.material.base_color = vec4(vol.rgb, 1.0);
    let lit = apply_pbr_lighting(lit_input).rgb;
    let n = normalize(pbr_input.N);
    let nv = saturate(dot(n, v));
    let f = g.x + (1.0 - g.x) * g.y * pow(1.0 - nv, 5.0);
    let r = reflect(-v, n);
    let sharp = textureSampleLevel(sky_cube, sky_samp, r, 0.0).rgb;
    let soft = textureSampleLevel(sky_cube, sky_samp, r, min(3.0, layered.env.y - 1.0)).rgb;
    let refl = mix(sharp, soft, g.z) * layered.env.x * view.exposure;
    let colour = lit * vol.a * (1.0 - f) + refl * f + emit * view.exposure;
    return vec4(colour, vol.a * (1.0 - f) + f);
}
#endif

// Deferred lighting pass ID of the game's lighting (`deferred.rs`); Bevy's own pass handles ID 1.
const GAME_LIGHTING_PASS_ID: u32 = 2u;

// The game's G-buffer, one RGBA8 target per word:
//   RT0 = (N * 0.5 + 0.5, roughness)  RT1 = (sqrt(albedo), AO)  RT2 = (F0, Fresnel strength)
//   RT3 = (sqrt(indirect × exposure), vehicle flag)
// The indirect term is the sky reflection × specular occlusion (AO², or 0 on dry roads) plus `extra`.
fn game_gbuffer(pbr_input: PbrInput, v: vec3<f32>, fresnel: f32, car: f32, extra: vec3<f32>) -> vec4<u32> {
    let n = normalize(pbr_input.N);
    let rough = pbr_input.material.perceptual_roughness;
    let f0 = 0.16 * pbr_input.material.reflectance * pbr_input.material.reflectance;
    let ao = pbr_input.diffuse_occlusion.x;
    let indirect = sky_reflection(n, v, rough, f0, fresnel) * pbr_input.specular_occlusion + extra;
    return vec4(
        pack4x8unorm(vec4(n * 0.5 + 0.5, rough)),
        pack4x8unorm(vec4(sqrt(saturate(pbr_input.material.base_color.rgb)), ao)),
        pack4x8unorm(vec4(saturate(f0), fresnel)),
        pack4x8unorm(vec4(sqrt(saturate(indirect * view.exposure)), car)),
    );
}
