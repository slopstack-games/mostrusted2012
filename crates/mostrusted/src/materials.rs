//! Materials: approximating the game's shader families on top of Bevy PBR.
//!
//! A [`Look`] is a hashable description derived from a game mesh's shader,
//! textures and constants. It becomes a [`GameMaterial`]: a `StandardMaterial`
//! plus the [`Layered`] extension, whose fragment shader (`layered.wgsl`)
//! reproduces the albedo maths of the multi-texture families (roads, terrain),
//! ported from the game's DXBC. Other families still use one base texture.

use bevy::material::OpaqueRendererMethod;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use mw_assets::SceneMesh;
use mw_bundle::ResourceId;

pub type GameMaterial = ExtendedMaterial<StandardMaterial, Layered>;

/// A texture as the GPU should see it: game ID plus colour space (true = sRGB).
pub type TexKey = (ResourceId, bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blend {
    Opaque,
    Mask,
    Translucent,
    /// Premultiplied colour, `src + dst × (1 - alpha)` (vehicle glass).
    Premultiplied,
    /// `src × dst` (vehicle lens filters).
    Multiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Standard = 0,
    Road = 1,
    Terrain = 2,
    /// PlotPBR: mask-blended tinted tiling layers plus a decal.
    Plot = 3,
    /// DriveableSurface line variants: road plus a tinted line texture (UV2).
    RoadLine = 4,
    /// WorldPBR: diffuse (Bevy base colour) and normal map (Bevy normal map) plus specular
    /// (x = F0 in gamma space, y = roughness) and the per-instance AO map (UV1, green).
    WorldPbr = 5,
    /// VehicleNFS13 body / wheel parts: albedo (texture or `PbrMaterialDiffuseColour`), F0 (specular
    /// texture rgb or `PbrMaterialSpecularColour`), roughness (normal map alpha or
    /// `PbrMaterialRoughness`), vertex colour AO / specular occlusion (via UV_1) and a clear coat.
    Vehicle = 6,
    /// Tree_Translucent: premultiplied leaves, albedo = (texel / alpha)^2 in gamma space, AO from the
    /// vertex colour's y (via UV_1) times the instance AO, no specular.
    Tree = 7,
    /// Water_Opaque / Water_Proto_Cheap: two scrolling wave normal maps, F0 / roughness from
    /// `specularAndRoughness`, algae-tinted albedo (black for the cheap variant).
    Water = 8,
    /// VehicleNFS13_Refraction lamp interiors: parallax-offset internal normal / colour / AO, LED
    /// emissive channels switched by the light state, the reflector's sharp sky reflection.
    Refraction = 9,
    /// VehicleNFS13_Glass*: forward, premultiplied: tinted volume colour lit diffusely, Fresnel-weighted
    /// sky reflection, optional lightmapped running light.
    Glass = 10,
    /// VehicleNFS13_Glass_Colourise: multiplies what is behind it by MaterialColour_SimpleMultiply.
    Colourise = 11,
}

/// Per-placement inputs the world shaders take from the instance rather than the material: the world
/// matrix's w column (xyz = albedo tint, w = AO scale, with `diffuseTintColour` from the instance's
/// override block multiplied in) and an `AOMapTextureSampler` override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceLook {
    tint: [u16; 3],
    ao: u16,
    pub ao_map: Option<ResourceId>,
}

impl Default for InstanceLook {
    fn default() -> Self {
        Self { tint: [q(1.0); 3], ao: q(1.0), ao_map: None }
    }
}

impl InstanceLook {
    pub fn from_placement(p: &mw_assets::Placement) -> Self {
        let w = p.transform.map(|r| r[3]);
        let t = p.overrides.as_deref().and_then(|o| o.constant("diffuseTintColour")).unwrap_or([1.0; 4]);
        // Prop / compound / dynamic placements come with the column forced to (0, 0, 0, 1): their
        // tint and AO live in a separate per-instance block that is not decoded yet.
        let w = if w[..3] == [0.0; 3] { [1.0; 4] } else { w };
        Self {
            tint: [w[0] * t[0], w[1] * t[1], w[2] * t[2]].map(q),
            ao: q(w[3]),
            ao_map: p.overrides.as_deref().and_then(|o| o.texture("AOMapTextureSampler")),
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Hashable description of a material. Floats are quantised so that
/// identical looks share one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Look {
    pub mode: Mode,
    /// Standard: [diffuse]. Road: [colour, detail]. Terrain: [mask, noise, layer1..3].
    /// WorldPbr: [diffuse, specular, AO map]. Vehicle: [diffuse, specular, normal, lightmapped lights].
    /// Refraction: [colour, external normal, emissive, internal normal, displacement]. Glass:
    /// [diffuse, lightmap].
    pub textures: [Option<TexKey>; N_TEX],
    /// Tangent-space normal map (Bevy's `normal_map_texture`; needs mesh tangents).
    pub normal: Option<TexKey>,
    /// Which game TEXCOORD feeds Bevy's second UV channel.
    pub uv_b: Option<u8>,
    tint: [u16; 4],
    roughness: u16,
    /// Shader constants, quantised: `N_VEC` vec4s (`Layered::params.p`).
    vals: [i32; N_VEC * 4],
    pub blend: Blend,
    pub double_sided: bool,
    pub paint: bool,
    /// The mesh keeps its vertex colour as Bevy's COLOR attribute (vehicle shaders read it raw).
    pub vertex_colour: bool,
}

fn q(v: f32) -> u16 {
    (v.clamp(0.0, 65.0) * 1000.0).round() as u16
}

fn dq(v: u16) -> f32 {
    v as f32 / 1000.0
}

fn qi(v: f32) -> i32 {
    (v.clamp(-1.0e5, 1.0e5) * 1000.0).round() as i32
}

/// Extension texture slots and constant vec4s (keep in sync with `layered.wgsl`).
const N_TEX: usize = 10;
const N_VEC: usize = 14;

/// Fill the texture slots in order.
fn slots(v: &[Option<TexKey>]) -> [Option<TexKey>; N_TEX] {
    std::array::from_fn(|i| v.get(i).copied().flatten())
}

fn flag(b: bool) -> f32 {
    b as u8 as f32
}

/// Pack up to `N_VEC` vec4 constants.
fn pack(v: &[[f32; 4]]) -> [i32; N_VEC * 4] {
    let mut out = [0; N_VEC * 4];
    for (i, c) in v.iter().take(N_VEC).enumerate() {
        for (j, x) in c.iter().enumerate() {
            out[i * 4 + j] = qi(*x);
        }
    }
    out
}

/// Car dirt and dust amounts (the paint shaders' `GenericVector4_A.xy`), as f32 bits; set by `--dirt`.
/// The game's rule that drives them at runtime is not traced yet, so they default to clean.
static DIRT: [std::sync::atomic::AtomicU32; 2] = [const { std::sync::atomic::AtomicU32::new(0) }; 2];

pub fn set_dirt_level(dirt: f32, dust: f32) {
    use std::sync::atomic::Ordering::Relaxed;
    DIRT[0].store(dirt.clamp(0.0, 1.0).to_bits(), Relaxed);
    DIRT[1].store(dust.clamp(0.0, 1.0).to_bits(), Relaxed);
}

fn dirt_level() -> [f32; 2] {
    DIRT.each_ref().map(|a| f32::from_bits(a.load(std::sync::atomic::Ordering::Relaxed)))
}

/// The lightmapped lights of a vehicle body / paint shader (`LightmapLightsTextureSampler`).
struct VehicleLightmap {
    texture: Option<TexKey>,
    /// 0 none; 1 four channels (`EmissiveFourChannel`): sat(texel² × mask.xyzy) × multiplier against
    /// the red / green / blue / alpha channel colours; 2 green only: sat(texel.y² × mask.y) × multiplier
    /// × green colour; 3 like 2 but also times the surface albedo (`Lightmap` variants).
    kind: f32,
    multiplier: f32,
    /// Red, green, blue and alpha channel colours (`LightmappedLights*ChannelColour`).
    channels: [[f32; 4]; 4],
    /// The lightmap is read with TEXCOORD3 (else TEXCOORD0).
    uv3: bool,
}

fn vehicle_lightmap(sm: &SceneMesh, name: &str) -> VehicleLightmap {
    let texture = sm.texture("LightmapLightsTextureSampler").map(|t| (t, false));
    let kind = match texture {
        None => 0.0,
        Some(_) if name.contains("emissivefourchannel") => 1.0,
        Some(_) if name.contains("_emissive_") => 2.0,
        Some(_) if name.contains("lightmap") => 3.0,
        Some(_) => 0.0,
    };
    let c = |n: &str| sm.parameter(&format!("LightmappedLights{n}ChannelColour")).unwrap_or([0.0; 4]);
    VehicleLightmap {
        texture: texture.filter(|_| kind > 0.0),
        kind,
        multiplier: sm.parameter("mSelfIlluminationMultiplier").map_or(1.0, |v| v[0]),
        channels: [c("Red"), c("Green"), c("Blue"), c("Alpha")],
        uv3: kind >= 2.0 && sm.mesh.texcoord(3).is_some(),
    }
}

impl Look {
    /// Derive a look from a decoded game mesh.
    pub fn from_scene_mesh(sm: &SceneMesh) -> Self {
        let name = sm.shader_name.to_ascii_lowercase();
        let srgb = |s: &str| sm.texture(s).map(|t| (t, true));
        let linear = |s: &str| sm.texture(s).map(|t| (t, false));
        let has_uv = |i: usize| sm.mesh.texcoord(i).is_some();

        let mut look = Self {
            mode: Mode::Standard,
            textures: slots(&[sm.diffuse.map(|t| (t, true))]),
            normal: None,
            uv_b: None,
            tint: [1.0, 1.0, 1.0, 1.0].map(q),
            roughness: q(sm.parameter("PbrMaterialRoughness").map_or(0.85, |r| r[0].clamp(0.05, 1.0))),
            vals: [0; N_VEC * 4],
            blend: Blend::Opaque,
            double_sided: name.contains("doublesided"),
            paint: name.contains("bodypaint"),
            vertex_colour: false,
        };

        // Road normal maps and F0 (SpecColour green); they need the decoded tangent frame.
        let framed = sm.mesh.tangents.is_some();
        let normal_map = linear("Normal_Sampler").filter(|_| framed);
        let detail_normal = linear("DetailMap_Normal_Sampler").filter(|_| framed);
        let spec_colour = linear("SpecColour_Sampler");
        // The wet path: the material's RoadDepth map and the engine's global puddle map (slots 7, 8),
        // WetScaleParams (p4), the vertex wetness mask (TEXCOORD5.x, `convert::add_road_wetness`).
        let road_depth = linear("RoadDepth_Sampler");
        let puddle = Some((mw_assets::environment::PUDDLE_MAP, false));
        let wet_scale = [WET_SCALE_PARAMS[0], WET_SCALE_PARAMS[1], WET_SCALE_PARAMS[2], 0.0];
        // Line and Decal variants draw their own texture (TEXCOORD2) over the road. FloatingDecal(_1Bit) and
        // RetroreflectivePaint_Lightmap write the plain road in their G-buffer pass (colour and spec at uv0,
        // detail at world xz × DetailMapUvScale; the paint one never samples its Line_* maps there), so they
        // take the road path; FloatingDecal_1Bit also discards where EdgeAlphaPlusAoMap(uv0).y < 0.5.
        let floating = name.contains("floatingdecal");
        let road_overlay = name.contains("line") || (name.contains("decal") && !floating);
        if name.starts_with("driveablesurface")
            && !road_overlay
            && let (Some(colour), Some(detail)) = (srgb("ColourMap_Sampler"), srgb("DetailMap_Diffuse_Sampler"))
        {
            let s = sm.parameter("DetailMapUvScale").unwrap_or([1.0, 1.0, 0.0, 0.0]);
            let edge = linear("EdgeAlphaPlusAoMap_Sampler").filter(|_| floating);
            look.mode = Mode::Road;
            look.textures = slots(&[
                Some(colour),
                Some(detail),
                edge,
                normal_map,
                detail_normal,
                spec_colour,
                None,
                road_depth,
                puddle,
            ]);
            // p2.x: discard where tex2(uv0).y < 0.5 (FloatingDecal_1Bit).
            let edge_test = if edge.is_some() { 1.0 } else { 0.0 };
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0], [edge_test, 0.0, 0.0, 0.0], [0.0; 4], wet_scale]);
        } else if name.starts_with("driveablesurface")
            && name.contains("decal")
            && let (Some(colour), Some(detail), Some(decal)) =
                (srgb("ColourMap_Sampler"), srgb("DetailMap_Diffuse_Sampler"), srgb("Decal_Diffuse_Sampler"))
            && has_uv(2)
        {
            // DriveableSurface_Decal(_Lightmap): the road with Decal_Diffuse / Decal_NormalPlusSpec at
            // TEXCOORD2 lerped over it by the decal alpha (albedo, normal, F0, roughness, Fresnel from
            // Decal_Roughness_Specular_Fresnel), written opaque like a line, so it shares the line path
            // with an untinted overlay.
            let s = sm.parameter("DetailMapUvScale").unwrap_or([1.0, 1.0, 0.0, 0.0]);
            let rsf = sm.parameter("Decal_Roughness_Specular_Fresnel").unwrap_or([0.5, 0.04, 1.0, 0.0]);
            look.mode = Mode::RoadLine;
            look.textures = slots(&[
                Some(colour),
                Some(detail),
                Some(decal),
                normal_map,
                detail_normal,
                spec_colour,
                linear("Decal_NormalPlusSpec_Sampler"),
                road_depth,
                puddle,
            ]);
            look.uv_b = Some(2);
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0], [1.0; 4], rsf, wet_scale]);
        } else if name.starts_with("driveablesurface")
            && name.contains("line")
            && let (Some(colour), Some(detail), Some(line)) =
                (srgb("ColourMap_Sampler"), srgb("DetailMap_Diffuse_Sampler"), srgb("Line_Diffuse_Sampler"))
            && has_uv(2)
        {
            // Lines use TEXCOORD2 and blend over the road by the line alpha.
            let s = sm.parameter("DetailMapUvScale").unwrap_or([1.0, 1.0, 0.0, 0.0]);
            let tint = sm.parameter("Line_Diffuse").unwrap_or([1.0; 4]);
            let rsf = sm.parameter("Line_Roughness_Specular_Fresnel").unwrap_or([0.5, 0.04, 1.0, 0.0]);
            look.mode = Mode::RoadLine;
            look.textures = slots(&[
                Some(colour),
                Some(detail),
                Some(line),
                normal_map,
                detail_normal,
                spec_colour,
                linear("Line_NormalPlusSpec_Sampler"),
                road_depth,
                puddle,
            ]);
            look.uv_b = Some(2);
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0], tint, rsf, wet_scale]);
        } else if name.starts_with("plotpbr")
            && let (Some(mask), Some(l1)) = (linear("MaskTextureSampler"), srgb("Tiling1TextureSampler"))
        {
            // Layers tile UV0 by Tiling_Ratios_X/Y; the mask and decal tile UV0
            // by maskTilingRatios / decalTilingRatios.
            let x = sm.parameter("Tiling_Ratios_X").unwrap_or([1.0; 4]);
            let y = sm.parameter("Tiling_Ratios_Y").unwrap_or([1.0; 4]);
            let m = sm.parameter("maskTilingRatios").unwrap_or([1.0; 4]);
            let d = sm.parameter("decalTilingRatios").unwrap_or([1.0; 4]);
            let tint = |n: &str| sm.parameter(n).unwrap_or([1.0; 4]);
            let decal = srgb("DecalTextureSampler");
            let rsf = |n: &str| sm.parameter(n).unwrap_or([1.0, 0.04, 1.0, 0.0]);
            let framed = sm.mesh.tangents.is_some();
            look.mode = Mode::Plot;
            look.textures = slots(&[
                Some(mask),
                Some(l1),
                srgb("Tiling2TextureSampler"),
                srgb("Tiling3TextureSampler"),
                decal,
                linear("Tiling1NormalSampler").filter(|_| framed),
                linear("Tiling2NormalSampler").filter(|_| framed),
                linear("Tiling3NormalSampler").filter(|_| framed),
            ]);
            if has_uv(1) {
                look.uv_b = Some(1);
            }
            look.vals = pack(&[
                x,
                y,
                [m[0], m[1], d[0], d[1]],
                tint("tiling1Diffuse"),
                tint("tiling2Diffuse"),
                tint("tiling3Diffuse"),
                // No decal texture: zero weight.
                if decal.is_some() { tint("decal_Diffuse") } else { [0.0; 4] },
                rsf("Layer1_Roughness_Specular_Fresnel"),
                rsf("Layer2_Roughness_Specular_Fresnel"),
                rsf("Layer3_Roughness_Specular_Fresnel"),
                // Flags (has tiling normals, has AO map, instance AO, has decal), set in `retain_textures`.
                [0.0, 0.0, 1.0, 0.0],
            ]);
        } else if name.starts_with("terrainpbr")
            && let (Some(mask), Some(noise)) = (linear("MaskTextureSampler"), linear("NoiseTextureSampler"))
            && has_uv(1)
        {
            let x = sm.parameter("Tiling_Ratios_X").unwrap_or([1.0; 4]);
            let y = sm.parameter("Tiling_Ratios_Y").unwrap_or([1.0; 4]);
            let n = sm.parameter("Tiling_Ratio_Noise").unwrap_or([1.0; 4]);
            let rsf = |n: &str| sm.parameter(n).unwrap_or([1.0, 0.04, 1.0, 0.0]);
            let framed = sm.mesh.normals.is_some() && sm.mesh.tangents.is_some();
            // The plain `Normal` variant's macro normal map is world space (Y up); the
            // `TangentSpaceNormal` one needs a second packed frame, not ported: vertex normal instead.
            let macro_normal = linear("NormalTextureSampler").filter(|_| framed && !name.contains("tangentspace"));
            look.mode = Mode::Terrain;
            look.textures = slots(&[
                Some(mask),
                Some(noise),
                srgb("Tiling1TextureSampler"),
                srgb("Tiling2TextureSampler"),
                srgb("Tiling3TextureSampler"),
                macro_normal,
                linear("Tiling1NormalSampler").filter(|_| framed),
                linear("Tiling2NormalSampler").filter(|_| framed),
                linear("Tiling3NormalSampler").filter(|_| framed),
            ]);
            look.uv_b = Some(1);
            look.vals = pack(&[
                [x[0], x[1], x[2], n[0]],
                [y[0], y[1], y[2], 0.0],
                rsf("Layer1_Roughness_Specular_Fresnel"),
                rsf("Layer2_Roughness_Specular_Fresnel"),
                rsf("Layer3_Roughness_Specular_Fresnel"),
                // Flags (has macro normal, has tiling normals), set in `retain_textures`.
                [0.0; 4],
            ]);
        } else if (name.contains("worldpbr_") || name.starts_with("world_diffuse_specular"))
            && let (Some(diffuse), Some(spec)) = (srgb("DiffuseTextureSampler"), linear("SpecularTextureSampler"))
        {
            // Every WorldPBR variant (buildings, vertex-lit, LOD2 windows, ...) and World_Diffuse_Specular
            // writes the same G-buffer inputs; variants without a normal map use the vertex normal.
            let c = sm.parameter("MaterialDiffuse").unwrap_or([1.0; 4]);
            // ColouredSpecular: F0 = SpecularColour(uv0).rgb * MaterialDiffuse (linear, no cubic).
            let coloured = linear("SpecularColourTextureSampler").filter(|_| name.contains("colouredspecular"));
            // The AO map sits in sampler slot 1: `OverlayTextureSampler` in the material, or a
            // per-instance `AOMapTextureSampler` override (see `with_instance`).
            let ao = linear("OverlayTextureSampler").filter(|_| has_uv(1));
            look.mode = Mode::WorldPbr;
            look.textures = slots(&[Some(diffuse), Some(spec), ao, coloured]);
            look.normal = linear("NormalTextureSampler").filter(|_| sm.mesh.tangents.is_some());
            look.tint = [c[0], c[1], c[2], 1.0].map(q);
            look.vals = pack(&[
                // w: planar-reflection variants have no cube reflection (only the wet planar one).
                [flag(coloured.is_some()), 1.0, flag(ao.is_some()), flag(name.contains("planarreflection"))],
                [c[0], c[1], c[2], 0.0],
            ]);
            if has_uv(1) {
                look.uv_b = Some(1);
            }
        }

        let vehicle = name.starts_with("vehiclenfs13_");
        if vehicle && !["paint", "glass", "refraction"].iter().any(|k| name.contains(k)) {
            look.mode = Mode::Vehicle;
            let diffuse = srgb("DiffuseTextureSampler");
            let spec = linear("SpecularTextureSampler");
            let normal_tex = linear("NormalTextureSampler");
            // Only the `NormalMap` variants perturb the normal; the single-channel `Emissive` one and
            // the `Wheel_Textured_Roughness` one keep the vertex normal and read just roughness there.
            let single_emissive = name.contains("_emissive_");
            let normal_mapped = name.contains("normalmap") && !single_emissive;
            let normal = normal_tex.filter(|_| normal_mapped && sm.mesh.tangents.is_some());
            let lm = vehicle_lightmap(sm, &name);
            look.textures = slots(&[diffuse, spec, normal_tex, lm.texture]);
            look.normal = normal;
            let tyre = name.contains("tyre");
            let c = if tyre {
                // The tyre's diffuse / specular / normal slots are unbound (they read 0): black rubber
                // that only the dust layer lightens.
                let d = sm.parameter("PbrMaterialDustColour").unwrap_or([0.05; 4]);
                [d[0] * 0.5, d[1] * 0.5, d[2] * 0.5, 1.0]
            } else if diffuse.is_some() {
                [1.0; 4]
            } else {
                sm.parameter("PbrMaterialDiffuseColour").unwrap_or([1.0; 4])
            };
            look.tint = [c[0], c[1], c[2], 1.0].map(q);
            let f0 = if tyre { [0.0; 4] } else { sm.parameter("PbrMaterialSpecularColour").unwrap_or([0.04; 4]) };
            // Roughness: the `PbrMaterialRoughness` constant where the shader has one (Body, Lightmap),
            // else the normal map's alpha, or its green channel in the single-channel Emissive variant.
            let rough_const = sm.parameter("PbrMaterialRoughness").map(|r| r[0]);
            let rough_source = match (rough_const, normal_tex) {
                (None, Some(_)) if single_emissive => 2.0,
                (None, Some(_)) => 1.0,
                _ => 0.0,
            };
            let rough = if tyre { 0.9 } else { rough_const.unwrap_or(0.5) };
            let cc_spec = sm.parameter("PbrMaterialClearcoatSpecular").map_or(0.0, |v| v[0]);
            let cc_fresnel = sm.parameter("PbrMaterialClearcoatFresnel").map_or(0.0, |v| v[0]);
            look.vals = pack(&[
                [f0[0], f0[1], f0[2], rough],
                [flag(spec.is_some()), rough_source, cc_spec, cc_fresnel],
                [flag(sm.mesh.colors.is_some()), flag(diffuse.is_some()), lm.kind, lm.multiplier],
                [c[0], c[1], c[2], flag(lm.uv3)],
                lm.channels[0],
                lm.channels[1],
                lm.channels[2],
                lm.channels[3],
            ]);
            look.vertex_colour = sm.mesh.colors.is_some();
            if lm.uv3 {
                look.uv_b = Some(3);
            }
        } else if vehicle && name.contains("paint") {
            // Body paint: `with_paint` fills in the paint; only the lightmapped lights are set here.
            let lm = vehicle_lightmap(sm, &name);
            // `_Livery`: DiffuseTextureSampler at TEXCOORD0 is a colour over the paint by its alpha;
            // `_TwoPaintMask`: its green selects the secondary paint (research log 2026-10-06, session 5).
            let kind = if name.contains("twopaintmask") {
                2.0
            } else if name.contains("livery") {
                1.0
            } else {
                0.0
            };
            let livery = sm.texture("DiffuseTextureSampler").filter(|_| kind > 0.0).map(|t| (t, kind < 1.5));
            // Dirt / dust / scratches read EffectsTextureSampler at TEXCOORD2 (UV_1 here), unless the
            // lightmapped lights already take UV_1 for TEXCOORD3. Only with `--dirt` until the game's
            // dirt build-up rule is ported (research log 2026-10-06, session 5 round 4).
            let effects = linear("EffectsTextureSampler").filter(|_| has_uv(2) && !lm.uv3 && dirt_level() != [0.0; 2]);
            look.textures = slots(&[livery, effects, None, lm.texture]);
            let two_paint = sm.parameter("mPaintColourIndex").map_or(0.0, |v| v[0]);
            look.vals = pack(&[
                [0.0; 4],
                [0.0; 4],
                [flag(sm.mesh.colors.is_some()), 0.0, lm.kind, lm.multiplier],
                [1.0, 1.0, 1.0, flag(lm.uv3)],
                lm.channels[0],
                lm.channels[1],
                lm.channels[2],
                lm.channels[3],
                [if livery.is_some() { kind } else { 0.0 }, two_paint, flag(effects.is_some()), 0.0],
            ]);
            look.vertex_colour = sm.mesh.colors.is_some();
            if lm.uv3 {
                look.uv_b = Some(3);
            } else if effects.is_some() {
                look.uv_b = Some(2);
            }
        } else if vehicle && name.contains("refraction") && sm.mesh.tangents.is_some() {
            // Lamp interiors: parallax-offset reflector with LED emissive channels (see `layered.wgsl`).
            let p = |n: &str, d: [f32; 4]| sm.parameter(n).unwrap_or(d);
            look.mode = Mode::Refraction;
            look.textures = slots(&[
                linear("ColourSampler"),
                linear("ExternalNormalTextureSampler"),
                linear("EmissiveTextureSampler"),
                linear("InternalNormalTextureSampler"),
                linear("DisplacementSampler"),
            ]);
            look.vals = pack(&[
                p("ParallaxConstants", [0.0; 4]),
                [
                    p("EmissiveLuminance", [0.0; 4])[0],
                    p("PbrMaterialClearcoatSpecular", [0.0; 4])[0],
                    p("PbrMaterialFresnel", [0.0; 4])[0],
                    0.0,
                ],
                p("PbrMaterialDiffuseColour", [0.25; 4]),
                p("mExternalGlassColour", [1.0; 4]),
                p("BrakeColour", [0.0; 4]),
                p("HeadlightColour", [0.0; 4]),
                p("ReversingColour", [0.0; 4]),
                p("TaillightColour", [0.0; 4]),
            ]);
        } else if vehicle && name.contains("glass_colourise") {
            // A multiplicative filter over what is behind it (light lenses): MaterialColour_SimpleMultiply.
            let c = sm.parameter("MaterialColour_SimpleMultiply").unwrap_or([1.0; 4]);
            look.mode = Mode::Colourise;
            look.textures = [None; N_TEX];
            look.tint = [c[0], c[1], c[2], 1.0].map(q);
            look.vals = pack(&[[c[0], c[1], c[2], 1.0]]);
        } else if vehicle && name.contains("glass") {
            let p = |n: &str, d: [f32; 4]| sm.parameter(n).unwrap_or(d);
            let lm = linear("LightmapTextureSampler").filter(|_| name.contains("lightmap"));
            let diffuse = srgb("DiffuseTextureSampler").filter(|_| name.contains("textured"));
            look.mode = Mode::Glass;
            look.textures = slots(&[diffuse, lm]);
            look.vals = pack(&[
                [
                    p("OpacityMin", [0.1; 4])[0],
                    p("FresnelFactor", [1.0; 4])[0],
                    p("SurfaceSoftness", [0.0; 4])[0],
                    p("mSelfIlluminationMultiplier", [0.0; 4])[0],
                ],
                p("DebugOverride_GlassVolumeColour", [0.0; 4]),
                p("RunningColour", [0.0; 4]),
                [flag(diffuse.is_some()), flag(lm.is_some()), flag(sm.mesh.colors.is_some()), 0.0],
            ]);
            look.vertex_colour = sm.mesh.colors.is_some();
        }

        if name.starts_with("tree_") && look.textures[0].is_some() {
            look.mode = Mode::Tree;
            look.vals = pack(&[[sm.mesh.colors.is_some() as u8 as f32, 1.0, 0.0, 0.0]]);
            if sm.mesh.colors.is_some() {
                look.uv_b = Some(crate::convert::UV_FROM_COLOUR);
            }
        }

        if name.starts_with("cable") {
            // Power lines: the gradient texture isn't bound as diffuse.
            look.textures = [None; N_TEX];
            look.tint = [0.04, 0.04, 0.045, 1.0].map(q);
        } else if name.starts_with("water") && sm.texture("NormalTextureSampler").is_some() {
            // Water_Opaque_Singlesided's vertex program: waves, surf over TEXCOORD1; diffuse, spec-depth and
            // surf mask over TEXCOORD0; river floor and the scrolling refraction lookup over TEXCOORD2
            // (carried in the COLOR attribute, `convert::WATER_UVS`); caustics over world xz / causticTile.
            // Water_Proto_Cheap: waves over TEXCOORD0, nothing else.
            let opaque = !name.contains("cheap");
            let p = |n: &str, d: [f32; 4]| sm.parameter(n).unwrap_or(d);
            let t1 = p("normalMapTile1", [1.0; 4]);
            let t2 = p("normalMapTile2", [1.0; 4]);
            let speed = p("waveAnimationSpeeds", [20.0; 4]);
            let sr = p("specularAndRoughness", [0.02, 0.1, 0.0, 0.0]);
            let algae = p("algaeColour", [0.0; 4]);
            let surf = p("surfTile", [1.0; 4]);
            let surf_speed = p("surfAnimationSpeed", [1.0; 4]);
            let caustic = p("causticTile", [1.0; 4]);
            let tex = |s: &str, srgb_: bool| sm.texture(s).filter(|_| opaque).map(|t| (t, srgb_));
            look.mode = Mode::Water;
            look.textures = slots(&[
                None,
                linear("NormalTextureSampler"),
                tex("DiffuseAndCausticsTextureSampler", true),
                tex("SpecDepthAndRefractionTextureSampler", false),
                tex("RiverFloorTextureSampler", true),
                tex("SurfTextureSampler", false),
                tex("SurfMaskTextureSampler", false),
                tex("SurfNormalTextureSampler", false),
            ]);
            look.uv_b = Some(if opaque && has_uv(1) && has_uv(2) {
                crate::convert::WATER_UVS
            } else if opaque && has_uv(1) {
                1
            } else {
                0
            });
            look.vals = pack(&[
                [t1[0], t1[1], t2[0], t2[1]],
                [speed[0], speed[1], sr[0], sr[1]],
                algae,
                [0.0; 4],
                [surf[0], surf[1], surf_speed[0], 0.0],
                [caustic[0], caustic[1], 0.0, 0.0],
            ]);
            if std::env::var_os("MW_WATER_DEBUG").is_some() {
                let n = sm.mesh.positions.len().max(1) as f32;
                let c = sm.mesh.positions.iter().fold([0.0f32; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]);
                let names: Vec<String> = sm
                    .textures
                    .iter()
                    .map(|(s, t)| format!("{}={:016x}", s.trim_end_matches("TextureSampler"), t.0))
                    .collect();
                eprintln!(
                    "water {} renderable {:016x} centre {:.0} {:.0} {:.0} uv2 {} textures {:?}",
                    sm.shader_name,
                    sm.renderable.0,
                    c[0] / n,
                    c[1] / n,
                    c[2] / n,
                    has_uv(2),
                    names
                );
            }
        } else if name.starts_with("water") {
            look.textures = [None; N_TEX];
            look.tint = [0.0, 0.0, 0.0, 1.0].map(q);
            look.roughness = q(0.1);
        } else if look.mode == Mode::Standard && look.textures[0].is_none() {
            let c = ["PbrMaterialDiffuseColour", "MaterialDiffuse", "materialDiffuse"]
                .iter()
                .find_map(|n| sm.parameter(n))
                .unwrap_or([0.6, 0.6, 0.62, 1.0]);
            look.tint = [c[0], c[1], c[2], 1.0].map(q);
        }

        look.blend = if look.mode == Mode::Glass {
            Blend::Premultiplied
        } else if look.mode == Mode::Colourise {
            Blend::Multiply
        } else if look.mode == Mode::Refraction {
            // Despite the name, the lamp interiors write the G-buffer.
            Blend::Opaque
        } else if look.mode == Mode::RoadLine {
            // Lines and DriveableSurface_Decal write the whole road, the overlay lerped in by its alpha.
            Blend::Opaque
        } else if name.contains("1bit") || name.contains("alphatest") || name.contains("foliage") {
            Blend::Mask
        } else if name.contains("glass")
            || name.contains("alphablend")
            || name.contains("decal")
            || name.contains("refraction")
            || name.contains("gbufferblend")
        {
            Blend::Translucent
        } else {
            Blend::Opaque
        };
        if look.blend == Blend::Translucent && look.textures[0].is_none() {
            let [r, g, b, _] = look.tint.map(dq);
            look.tint = [r * 0.3, g * 0.3, b * 0.35, 0.35].map(q);
        }
        look
    }

    /// Every texture this look needs.
    pub fn all_textures(&self) -> impl Iterator<Item = TexKey> + '_ {
        self.textures.iter().chain(std::iter::once(&self.normal)).flatten().copied()
    }

    /// Drop texture slots that failed to load, falling back to the standard
    /// mode if a layered mode lost an essential input.
    pub fn retain_textures(&mut self, loaded: impl Fn(&TexKey) -> bool) {
        for t in self.textures.iter_mut().chain(std::iter::once(&mut self.normal)) {
            if t.is_some_and(|k| !loaded(&k)) {
                *t = None;
            }
        }
        // Missing textures bind as opaque white, so a layered mode needs its
        // essential inputs or it falls back to the plain material.
        let essential = match self.mode {
            Mode::Standard | Mode::Vehicle | Mode::Glass | Mode::Colourise => 0,
            Mode::Refraction => 5,
            Mode::Tree => 1,
            Mode::Water => 0,
            Mode::Road | Mode::Terrain | Mode::Plot | Mode::WorldPbr => 2,
            Mode::RoadLine => 3,
        };
        if self.textures[..essential].iter().any(Option::is_none) {
            // WorldPBR keeps its diffuse texture as a plain material.
            let keep = if self.mode == Mode::WorldPbr { self.textures[0] } else { None };
            self.mode = Mode::Standard;
            self.uv_b = None;
            self.textures = slots(&[keep]);
            self.normal = None;
        }
        if self.mode == Mode::Plot && self.textures[4].is_none() {
            // No decal: zero its strength.
            self.vals[24..28].fill(0);
        }
        let has = |i: usize| flag(self.textures[i].is_some());
        match self.mode {
            Mode::Road | Mode::RoadLine => {
                self.vals[0] = qi(flag(self.textures[3].is_some() && self.textures[4].is_some()));
                self.vals[1] = qi(has(5));
                self.vals[2] = qi(has(6));
                // p0.w: 0 = no wet path (no puddle map), 1 = puddle map only (a missing RoadDepth map
                // reads 0 in the game: no flattening), 2 = both.
                self.vals[3] = qi(has(8) * (1.0 + has(7)));
            }
            Mode::Terrain => {
                self.vals[20] = qi(has(5));
                self.vals[21] = qi(flag(self.textures[6..9].iter().all(Option::is_some)));
            }
            Mode::Water => {
                self.vals[12] = qi(flag(self.textures[2].is_some() && self.textures[3].is_some()));
                self.vals[13] = qi(has(1));
                // p3.z: river floor + TEXCOORD2 present; p3.w: surf (texture, mask and normal) present.
                self.vals[14] = qi(flag(self.uv_b == Some(crate::convert::WATER_UVS) && self.textures[4].is_some()));
                self.vals[15] = qi(flag(self.textures[5..8].iter().all(Option::is_some)));
            }
            Mode::Plot => {
                self.vals[40] = qi(flag(self.textures[5..8].iter().all(Option::is_some)));
                self.vals[41] = qi(has(8));
                self.vals[43] = qi(has(4));
            }
            _ => {}
        }
        if self.mode == Mode::WorldPbr {
            self.vals[0] = qi(self.textures[3].is_some() as u8 as f32);
            self.vals[2] = qi(self.textures[2].is_some() as u8 as f32);
        }
        if self.paint {
            // Body paint keeps the flags `from_scene_mesh` / `with_paint` wrote: its slot 0 is the livery or
            // two-paint mask (p8.x) and slot 1 the effects texture (p8.z), not a diffuse and a spec map; read
            // as the latter, the effects map became F0 at TEXCOORD0 and tinted the dirt pink / cyan (research
            // log 2026-10-06, session 5 round 4).
            if self.textures[0].is_none() {
                self.vals[32] = 0;
            }
            self.vals[34] = qi(has(1));
            if self.textures[3].is_none() {
                self.vals[10] = 0;
            }
        } else if self.mode == Mode::Vehicle {
            // p1.x has spec map, p1.y roughness source (needs the normal texture), p2.y has diffuse,
            // p2.z lightmapped-lights kind (needs the lightmap).
            self.vals[4] = qi(has(1));
            if self.textures[2].is_none() {
                self.vals[5] = 0;
            }
            self.vals[9] = qi(has(0));
            if self.textures[3].is_none() {
                self.vals[10] = 0;
            }
        }
        if self.mode == Mode::Glass {
            self.vals[12] = qi(has(0));
            self.vals[13] = qi(has(1));
        }
    }

    /// Apply a placement's tint, AO scale and AO map (world shaders only).
    pub fn with_instance(mut self, inst: &InstanceLook) -> Self {
        if self.mode == Mode::Plot {
            self.vals[42] = qi(dq(inst.ao));
            if let Some(ao) = inst.ao_map
                && self.uv_b == Some(1)
            {
                self.textures[8] = Some((ao, false));
            }
            return self;
        }
        if self.mode == Mode::Tree {
            self.vals[1] = qi(dq(inst.ao));
            return self;
        }
        if self.mode != Mode::WorldPbr {
            return self;
        }
        for (c, t) in self.tint.iter_mut().zip(inst.tint) {
            *c = q(dq(*c) * dq(t));
        }
        self.vals[1] = qi(dq(inst.ao));
        if let Some(ao) = inst.ao_map
            && self.uv_b == Some(1)
        {
            self.textures[2] = Some((ao, false));
            self.vals[2] = qi(1.0);
        }
        self
    }

    /// Body-paint meshes take the car's paint: the vehicle mode with the paint's colour, F0 and coat.
    /// The livery / two-paint mask and the dirt amounts (`--dirt`) are applied in `layered.wgsl`.
    pub fn with_paint(mut self, paint: Option<&mw_assets::paint::VehiclePaint>) -> Self {
        if !self.paint {
            return self;
        }
        let p = paint.copied().unwrap_or(mw_assets::paint::VehiclePaint {
            diffuse: [0.42, 0.02, 0.025],
            specular: [0.0025; 3],
            specular_roughness: 0.0,
            clearcoat_specular: 0.03,
            clearcoat_fresnel: 0.5,
            clearcoat_roughness: 0.0,
            clearcoat: true,
            pearlescent: [0.0; 3],
            dirt_colour: [0.0; 3],
            dust_colour: [0.0; 3],
            dirt_scale: 1.0,
            dust_scale: 1.0,
            secondary: None,
            colour_index: -1,
        });
        let (cc_spec, cc_fresnel) = if p.clearcoat { (p.clearcoat_specular, p.clearcoat_fresnel) } else { (0.0, 0.0) };
        self.mode = Mode::Vehicle;
        // Slot 0 keeps the livery / two-paint mask, 1 the effects (dirt) texture, 3 the lightmapped
        // lights of the `_Lightmap` paint variants; p8 (livery kind, mPaintColourIndex, has effects) is
        // kept from `from_scene_mesh`. GenericVector4_A.xy = (dirt, dust) amounts, B / C = the paint's
        // DirtColour / DustColour (research log 2026-10-06, session 5).
        // `FUN_008e9c00` writes A = (dirt level × DirtColour.w, dust level × DustColour.w, ..), B = DirtColour,
        // C = DustColour, and slots 0x30 / 0x31 = the secondary paint's diffuse / specular minus the primary's,
        // which the `_TwoPaint*` programs add × (mPaintColourIndex or the mask's green): p12 / p13 here
        // (research log 2026-10-06, session 5 round 4).
        let [dirt, dust] = dirt_level();
        let (dc, uc) = (p.dirt_colour, p.dust_colour);
        self.vals[36..48].copy_from_slice(
            &pack(&[
                [dirt * p.dirt_scale, dust * p.dust_scale, 0.0, 0.0],
                [dc[0], dc[1], dc[2], 0.0],
                [uc[0], uc[1], uc[2], 0.0],
            ])[..12],
        );
        let (sd, ss) = p.secondary.unwrap_or((p.diffuse, p.specular));
        let d = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], 0.0];
        self.vals[48..56].copy_from_slice(&pack(&[d(sd, p.diffuse), d(ss, p.specular)])[..8]);
        self.textures = slots(&[self.textures[0], self.textures[1], None, self.textures[3]]);
        self.normal = None;
        self.tint = [p.diffuse[0], p.diffuse[1], p.diffuse[2], 1.0].map(q);
        self.vals[..8].copy_from_slice(
            &pack(&[
                [p.specular[0], p.specular[1], p.specular[2], p.specular_roughness],
                [0.0, 0.0, cc_spec, cc_fresnel],
            ])[..8],
        );
        self.vals[9] = 0;
        for (i, c) in p.diffuse.iter().enumerate() {
            self.vals[12 + i] = qi(*c);
        }
        self
    }

    /// Clear coat strength for Bevy: the coat's reflectance at grazing angles,
    /// `lerp(F0, 1, fresnel)` (Bevy's coat rises from 0.04 to 1 times this).
    fn clearcoat(&self) -> f32 {
        if self.mode != Mode::Vehicle {
            return 0.0;
        }
        let (spec, fresnel) = (self.vals[6] as f32 / 1000.0, self.vals[7] as f32 / 1000.0);
        let c = spec + (1.0 - spec) * fresnel;
        if c < 0.02 { 0.0 } else { c.min(1.0) }
    }

    /// Whether the mesh needs the road wetness mask (`convert::add_road_wetness`).
    pub fn wet_road(&self) -> bool {
        matches!(self.mode, Mode::Road | Mode::RoadLine)
    }

    /// Whether the material reads the vehicle light state.
    pub fn uses_lights(&self) -> bool {
        match self.mode {
            Mode::Vehicle => self.vals[10] != 0,
            Mode::Refraction => true,
            Mode::Glass => self.vals[13] != 0,
            _ => false,
        }
    }

    pub fn material(
        &self,
        tex: impl Fn(Option<TexKey>) -> Option<Handle<Image>>,
        paint: Option<Color>,
        env: &GameEnvironment,
        lights: &VehicleLights,
    ) -> GameMaterial {
        let [r, g, b, a] = self.tint.map(dq);
        let base_color = match paint {
            Some(c) if self.paint => c,
            _ => Color::linear_rgba(r, g, b, a),
        };
        let roughness = dq(self.roughness);
        let v = self.vals.map(|x| x as f32 / 1000.0);
        let mut p: [Vec4; N_VEC] =
            std::array::from_fn(|i| Vec4::new(v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]));
        if self.mode == Mode::Vehicle
            && self.paint
            && let Some(c) = paint
        {
            // `--paint`: the vehicle mode's albedo is p3.rgb.
            let c = c.to_linear();
            p[3] = Vec4::new(c.red, c.green, c.blue, p[3].w);
        }
        GameMaterial {
            base: StandardMaterial {
                base_color,
                base_color_texture: if matches!(self.mode, Mode::Standard | Mode::WorldPbr | Mode::Vehicle | Mode::Tree)
                {
                    tex(self.textures[0])
                } else {
                    None
                },
                normal_map_texture: tex(self.normal),
                perceptual_roughness: roughness,
                clearcoat: self.clearcoat(),
                clearcoat_perceptual_roughness: 0.03,
                // Opaque and alpha-tested looks write the game's G-buffer in the deferred prepass;
                // translucent ones are drawn forward.
                opaque_render_method: OpaqueRendererMethod::Deferred,
                alpha_mode: match self.blend {
                    Blend::Opaque => AlphaMode::Opaque,
                    // The 1-bit and tree shaders discard below 129/255.
                    Blend::Mask => AlphaMode::Mask(0.505882),
                    Blend::Translucent => AlphaMode::Blend,
                    Blend::Premultiplied => AlphaMode::Premultiplied,
                    Blend::Multiply => AlphaMode::Multiply,
                },
                double_sided: self.double_sided,
                cull_mode: None,
                ..default()
            },
            extension: Layered {
                params: LayeredParams {
                    mode: self.mode as u32,
                    p,
                    env: Vec4::new(
                        env.sky_brightness,
                        env.reflection_mips as f32,
                        env.road_wetness,
                        flag(env.planar.is_some()),
                    ),
                    lights: lights.mask,
                    ..default()
                },
                sky: env.reflection.clone(),
                planar: env.planar.clone(),
                tex0: tex(self.textures[0]),
                tex1: tex(self.textures[1]),
                tex2: tex(self.textures[2]),
                tex3: tex(self.textures[3]),
                tex4: tex(self.textures[4]),
                tex5: tex(self.textures[5]),
                tex6: tex(self.textures[6]),
                tex7: tex(self.textures[7]),
                tex8: tex(self.textures[8]),
                tex9: tex(self.textures[9]),
            },
        }
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, Reflect)]
pub struct LayeredParams {
    pub mode: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
    /// Mode-specific constants; see `layered.wgsl`.
    pub p: [Vec4; N_VEC],
    /// x = sky brightness (cd/m² per panorama unit), y = reflection cube mip count.
    pub env: Vec4,
    /// Vehicle light state ([`VehicleLights::mask`]).
    pub lights: Vec4,
}

/// Extension bindings start at 100 to stay clear of `StandardMaterial`'s.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct Layered {
    #[uniform(100)]
    pub params: LayeredParams,
    #[texture(101)]
    #[sampler(102)]
    pub tex0: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub tex1: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub tex2: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub tex3: Option<Handle<Image>>,
    #[texture(109)]
    #[sampler(110)]
    pub tex4: Option<Handle<Image>>,
    #[texture(111)]
    #[sampler(112)]
    pub tex5: Option<Handle<Image>>,
    #[texture(113)]
    #[sampler(114)]
    pub tex6: Option<Handle<Image>>,
    #[texture(115)]
    #[sampler(116)]
    pub tex7: Option<Handle<Image>>,
    #[texture(117)]
    #[sampler(118)]
    pub tex8: Option<Handle<Image>>,
    #[texture(119)]
    #[sampler(120)]
    pub tex9: Option<Handle<Image>>,
    /// Prefiltered sky reflection cube (`sky::reflection_cube`).
    #[texture(121, dimension = "cube")]
    #[sampler(122)]
    pub sky: Option<Handle<Image>>,
    /// The planar reflection (`reflection.rs`), read by the roads' wet path.
    #[texture(123)]
    #[sampler(124)]
    pub planar: Option<Handle<Image>>,
}

impl MaterialExtension for Layered {
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/layered.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://mostrusted/layered.wgsl".into()
    }

    /// The planar reflection camera is the only view with a normal prepass: its pipelines discard what
    /// lies below the reflection plane (`reflection.rs`).
    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if key.mesh_key.contains(bevy::pbr::MeshPipelineKey::NORMAL_PREPASS)
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            fragment.shader_defs.push("PLANAR_REFLECTION_VIEW".into());
        }
        Ok(())
    }
}

/// The vehicle light state, the game's per-object `g_selfIlluminationMask`: x = brake, y = headlights /
/// running lights (also the lightmapped tail glow), z = reversing, w = tail lights.
#[derive(Resource, Clone, Copy, Debug)]
pub struct VehicleLights {
    /// What is switched on regardless of the pedals (`--lights`).
    pub base: Vec4,
    pub mask: Vec4,
}

impl VehicleLights {
    pub fn from_slice(v: &[f32]) -> Self {
        let g = |i: usize, d: f32| v.get(i).copied().unwrap_or(d).clamp(0.0, 1.0);
        let base = Vec4::new(g(0, 0.0), g(1, 1.0), g(2, 0.0), g(3, 1.0));
        Self { base, mask: base }
    }
}

impl Default for VehicleLights {
    fn default() -> Self {
        Self::from_slice(&[])
    }
}

/// Scene-wide inputs of the game's material passes: the sky reflection cube and its brightness.
#[derive(Resource, Clone)]
pub struct GameEnvironment {
    pub reflection: Option<Handle<Image>>,
    pub reflection_mips: usize,
    pub sky_brightness: f32,
    /// Scales the road shader's wet amount (puddle map w × TEXCOORD5.x). 1 is the game (whose road
    /// shader has no global wetness input: roads are always as wet as the puddle map says); 0 dries the
    /// roads, for comparison (`--road-wetness`).
    pub road_wetness: f32,
    /// The planar reflection image (`reflection.rs`); without it the wet roads reflect the sky cube.
    pub planar: Option<Handle<Image>>,
}

impl Default for GameEnvironment {
    fn default() -> Self {
        Self { reflection: None, reflection_mips: 0, sky_brightness: 0.0, road_wetness: 1.0, planar: None }
    }
}

/// The road shader's `WetScaleParams` (global, set by the engine; values provisional): x = puddle map
/// tiling (world xz × x), y = how far the normal's deviation shifts the planar reflection lookup, z =
/// reflection blur (mip = roughness × z).
pub const WET_SCALE_PARAMS: [f32; 3] = [1.0 / 24.0, 0.08, 7.0];
