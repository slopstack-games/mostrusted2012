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
    /// WorldPbr: [diffuse, specular, AO map]. Vehicle: [diffuse, specular, normal (alpha = roughness)].
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
const N_VEC: usize = 12;

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
        };

        // Road normal maps and F0 (SpecColour green); they need the decoded tangent frame.
        let framed = sm.mesh.tangents.is_some();
        let normal_map = linear("Normal_Sampler").filter(|_| framed);
        let detail_normal = linear("DetailMap_Normal_Sampler").filter(|_| framed);
        let spec_colour = linear("SpecColour_Sampler");
        // Line / decal / paint variants draw their own texture on top of the road.
        let road_overlay = ["line", "decal", "retroreflective"].iter().any(|k| name.contains(k));
        if name.starts_with("driveablesurface")
            && !road_overlay
            && let (Some(colour), Some(detail)) = (srgb("ColourMap_Sampler"), srgb("DetailMap_Diffuse_Sampler"))
        {
            let s = sm.parameter("DetailMapUvScale").unwrap_or([1.0, 1.0, 0.0, 0.0]);
            look.mode = Mode::Road;
            look.textures = slots(&[Some(colour), Some(detail), None, normal_map, detail_normal, spec_colour]);
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0]]);
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
            ]);
            look.uv_b = Some(2);
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0], tint, rsf]);
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

        if name.starts_with("vehiclenfs13_") && !["paint", "glass", "refraction"].iter().any(|k| name.contains(k)) {
            look.mode = Mode::Vehicle;
            let diffuse = srgb("DiffuseTextureSampler");
            let spec = linear("SpecularTextureSampler");
            let normal = linear("NormalTextureSampler").filter(|_| sm.mesh.tangents.is_some());
            look.textures = slots(&[diffuse, spec, normal]);
            look.normal = normal;
            let tyre = name.contains("tyre");
            let c = if tyre {
                // The tyre's diffuse / specular / normal slots are unbound (they read 0): black rubber
                // that only the dust layer lightens.
                let d = sm.parameter("PbrMaterialDustColour").unwrap_or([0.05; 4]);
                [d[0] * 0.5, d[1] * 0.5, d[2] * 0.5, 1.0]
            } else {
                sm.parameter("PbrMaterialDiffuseColour").unwrap_or([1.0; 4])
            };
            if diffuse.is_none() {
                look.tint = [c[0], c[1], c[2], 1.0].map(q);
            }
            let f0 = if tyre { [0.0; 4] } else { sm.parameter("PbrMaterialSpecularColour").unwrap_or([0.04; 4]) };
            let rough = if tyre { 0.9 } else { sm.parameter("PbrMaterialRoughness").map_or(0.5, |r| r[0]) };
            let cc_spec = sm.parameter("PbrMaterialClearcoatSpecular").map_or(0.0, |v| v[0]);
            let cc_fresnel = sm.parameter("PbrMaterialClearcoatFresnel").map_or(0.0, |v| v[0]);
            look.vals = pack(&[
                [f0[0], f0[1], f0[2], rough],
                [spec.is_some() as u8 as f32, normal.is_some() as u8 as f32, cc_spec, cc_fresnel],
                [sm.mesh.colors.is_some() as u8 as f32, 0.0, 0.0, 0.0],
            ]);
            if sm.mesh.colors.is_some() {
                look.uv_b = Some(crate::convert::UV_FROM_COLOUR);
            }
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
            // Waves scroll over TEXCOORD1 (opaque) or TEXCOORD0 (cheap); the rest uses TEXCOORD0.
            let opaque = !name.contains("cheap");
            let t1 = sm.parameter("normalMapTile1").unwrap_or([1.0; 4]);
            let t2 = sm.parameter("normalMapTile2").unwrap_or([1.0; 4]);
            let speed = sm.parameter("waveAnimationSpeeds").unwrap_or([20.0; 4]);
            let sr = sm.parameter("specularAndRoughness").unwrap_or([0.02, 0.1, 0.0, 0.0]);
            let algae = sm.parameter("algaeColour").unwrap_or([0.0; 4]);
            look.mode = Mode::Water;
            look.textures = slots(&[
                None,
                linear("NormalTextureSampler"),
                srgb("DiffuseAndCausticsTextureSampler").filter(|_| opaque),
                linear("SpecDepthAndRefractionTextureSampler").filter(|_| opaque),
            ]);
            look.uv_b = Some(if opaque && has_uv(1) { 1 } else { 0 });
            look.vals = pack(&[[t1[0], t1[1], t2[0], t2[1]], [speed[0], speed[1], sr[0], sr[1]], algae, [0.0; 4]]);
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

        look.blend = if name.contains("1bit") || name.contains("alphatest") || name.contains("foliage") {
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
            Mode::Standard | Mode::Vehicle => 0,
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
            }
            Mode::Terrain => {
                self.vals[20] = qi(has(5));
                self.vals[21] = qi(flag(self.textures[6..9].iter().all(Option::is_some)));
            }
            Mode::Water => {
                self.vals[12] = qi(flag(self.textures[2].is_some() && self.textures[3].is_some()));
                self.vals[13] = qi(has(1));
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
        if self.mode == Mode::Vehicle {
            self.vals[4] = qi(self.textures[1].is_some() as u8 as f32);
            self.vals[5] = qi(self.textures[2].is_some() as u8 as f32);
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
            colour_index: -1,
        });
        let (cc_spec, cc_fresnel) = if p.clearcoat { (p.clearcoat_specular, p.clearcoat_fresnel) } else { (0.0, 0.0) };
        self.mode = Mode::Vehicle;
        // Livery / two-paint masks are not applied yet: plain paint.
        self.textures = [None; N_TEX];
        self.normal = None;
        self.tint = [p.diffuse[0], p.diffuse[1], p.diffuse[2], 1.0].map(q);
        self.vals[..8].copy_from_slice(
            &pack(&[
                [p.specular[0], p.specular[1], p.specular[2], p.specular_roughness],
                [0.0, 0.0, cc_spec, cc_fresnel],
            ])[..8],
        );
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

    pub fn material(
        &self,
        tex: impl Fn(Option<TexKey>) -> Option<Handle<Image>>,
        paint: Option<Color>,
        env: &GameEnvironment,
    ) -> GameMaterial {
        let [r, g, b, a] = self.tint.map(dq);
        let base_color = match paint {
            Some(c) if self.paint => c,
            _ => Color::linear_rgba(r, g, b, a),
        };
        let roughness = dq(self.roughness);
        let v = self.vals.map(|x| x as f32 / 1000.0);
        let p = std::array::from_fn(|i| Vec4::new(v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]));
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
                },
                double_sided: self.double_sided,
                cull_mode: None,
                ..default()
            },
            extension: Layered {
                params: LayeredParams {
                    mode: self.mode as u32,
                    p,
                    env: Vec4::new(env.sky_brightness, env.reflection_mips as f32, 0.0, 0.0),
                    ..default()
                },
                sky: env.reflection.clone(),
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
}

impl MaterialExtension for Layered {
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/layered.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://mostrusted/layered.wgsl".into()
    }
}

/// Scene-wide inputs of the game's material passes: the sky reflection cube and its brightness.
#[derive(Resource, Clone, Default)]
pub struct GameEnvironment {
    pub reflection: Option<Handle<Image>>,
    pub reflection_mips: usize,
    pub sky_brightness: f32,
}
