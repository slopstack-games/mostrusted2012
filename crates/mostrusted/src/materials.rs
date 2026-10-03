//! Materials: approximating the game's shader families on top of Bevy PBR.
//!
//! A [`Look`] is a hashable description derived from a game mesh's shader,
//! textures and constants. It becomes a [`GameMaterial`]: a `StandardMaterial`
//! plus the [`Layered`] extension, whose fragment shader (`layered.wgsl`)
//! reproduces the albedo maths of the multi-texture families (roads, terrain),
//! ported from the game's DXBC. Other families still use one base texture.

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
}

/// Hashable description of a material. Floats are quantised so that
/// identical looks share one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Look {
    pub mode: Mode,
    /// Standard: [diffuse]. Road: [colour, detail]. Terrain: [mask, noise, layer1..3].
    pub textures: [Option<TexKey>; 5],
    /// Which game TEXCOORD feeds Bevy's second UV channel.
    pub uv_b: Option<u8>,
    tint: [u16; 4],
    roughness: u16,
    /// Shader constants, quantised: 7 vec4s (`Layered::params.p`).
    vals: [i32; 28],
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

/// Pack up to seven vec4 constants.
fn pack(v: &[[f32; 4]]) -> [i32; 28] {
    let mut out = [0; 28];
    for (i, c) in v.iter().take(7).enumerate() {
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
            textures: [sm.diffuse.map(|t| (t, true)), None, None, None, None],
            uv_b: None,
            tint: [1.0, 1.0, 1.0, 1.0].map(q),
            roughness: q(sm.parameter("PbrMaterialRoughness").map_or(0.85, |r| r[0].clamp(0.05, 1.0))),
            vals: [0; 28],
            blend: Blend::Opaque,
            double_sided: name.contains("doublesided"),
            paint: name.contains("bodypaint"),
        };

        // Line / decal / paint variants draw their own texture on top of the road.
        let road_overlay = ["line", "decal", "retroreflective"].iter().any(|k| name.contains(k));
        if name.starts_with("driveablesurface")
            && !road_overlay
            && let (Some(colour), Some(detail)) = (srgb("ColourMap_Sampler"), srgb("DetailMap_Diffuse_Sampler"))
        {
            let s = sm.parameter("DetailMapUvScale").unwrap_or([1.0, 1.0, 0.0, 0.0]);
            look.mode = Mode::Road;
            look.textures = [Some(colour), Some(detail), None, None, None];
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
            look.mode = Mode::RoadLine;
            look.textures = [Some(colour), Some(detail), Some(line), None, None];
            look.uv_b = Some(2);
            look.vals = pack(&[[0.0; 4], [s[0], s[1], 0.0, 0.0], tint]);
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
            look.mode = Mode::Plot;
            look.textures = [Some(mask), Some(l1), srgb("Tiling2TextureSampler"), srgb("Tiling3TextureSampler"), decal];
            look.vals = pack(&[
                x,
                y,
                [m[0], m[1], d[0], d[1]],
                tint("tiling1Diffuse"),
                tint("tiling2Diffuse"),
                tint("tiling3Diffuse"),
                // No decal texture: zero weight.
                if decal.is_some() { tint("decal_Diffuse") } else { [0.0; 4] },
            ]);
        } else if name.starts_with("terrainpbr")
            && let (Some(mask), Some(noise)) = (linear("MaskTextureSampler"), linear("NoiseTextureSampler"))
            && has_uv(1)
        {
            let x = sm.parameter("Tiling_Ratios_X").unwrap_or([1.0; 4]);
            let y = sm.parameter("Tiling_Ratios_Y").unwrap_or([1.0; 4]);
            let n = sm.parameter("Tiling_Ratio_Noise").unwrap_or([1.0; 4]);
            look.mode = Mode::Terrain;
            look.textures = [
                Some(mask),
                Some(noise),
                srgb("Tiling1TextureSampler"),
                srgb("Tiling2TextureSampler"),
                srgb("Tiling3TextureSampler"),
            ];
            look.uv_b = Some(1);
            look.vals = pack(&[[x[0], x[1], x[2], n[0]], [y[0], y[1], y[2], 0.0]]);
        }

        if name.starts_with("cable") {
            // Power lines: the gradient texture isn't bound as diffuse.
            look.textures = [None; 5];
            look.tint = [0.04, 0.04, 0.045, 1.0].map(q);
        } else if name.starts_with("water") {
            // Placeholder until the water shader is ported: dark, glossy.
            look.textures = [None; 5];
            look.tint = [0.03, 0.07, 0.08, 1.0].map(q);
            look.roughness = q(0.06);
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

    /// Drop texture slots that failed to load, falling back to the standard
    /// mode if a layered mode lost an essential input.
    pub fn retain_textures(&mut self, loaded: impl Fn(&TexKey) -> bool) {
        for t in &mut self.textures {
            if t.is_some_and(|k| !loaded(&k)) {
                *t = None;
            }
        }
        // Missing textures bind as opaque white, so a layered mode needs its
        // essential inputs or it falls back to the plain material.
        let essential = match self.mode {
            Mode::Standard => 0,
            Mode::Road | Mode::Terrain | Mode::Plot => 2,
            Mode::RoadLine => 3,
        };
        if self.textures[..essential].iter().any(Option::is_none) {
            self.mode = Mode::Standard;
            self.uv_b = None;
            self.textures = [None; 5];
        }
        if self.mode == Mode::Plot && self.textures[4].is_none() {
            // No decal: zero its strength.
            self.vals[24..28].fill(0);
        }
    }

    pub fn material(&self, tex: impl Fn(Option<TexKey>) -> Option<Handle<Image>>, paint: Color) -> GameMaterial {
        let [r, g, b, a] = self.tint.map(dq);
        // Bevy's clearcoat made these meshes disappear (missing tangents?),
        // so paint is a plain glossy material for now.
        let (base_color, roughness) =
            if self.paint { (paint, 0.35) } else { (Color::linear_rgba(r, g, b, a), dq(self.roughness)) };
        let v = self.vals.map(|x| x as f32 / 1000.0);
        let p = std::array::from_fn(|i| Vec4::new(v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]));
        GameMaterial {
            base: StandardMaterial {
                base_color,
                base_color_texture: if self.mode == Mode::Standard { tex(self.textures[0]) } else { None },
                perceptual_roughness: roughness,
                alpha_mode: match self.blend {
                    Blend::Opaque => AlphaMode::Opaque,
                    Blend::Mask => AlphaMode::Mask(0.5),
                    Blend::Translucent => AlphaMode::Blend,
                },
                double_sided: self.double_sided,
                cull_mode: None,
                ..default()
            },
            extension: Layered {
                params: LayeredParams { mode: self.mode as u32, p, ..default() },
                tex0: tex(self.textures[0]),
                tex1: tex(self.textures[1]),
                tex2: tex(self.textures[2]),
                tex3: tex(self.textures[3]),
                tex4: tex(self.textures[4]),
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
    pub p: [Vec4; 7],
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
}

impl MaterialExtension for Layered {
    fn fragment_shader() -> ShaderRef {
        "embedded://mostrusted/layered.wgsl".into()
    }
}
