//! `Material` (0x2), `Shader` (0x53) and `Model` (0x51).

use mw_bundle::{Reader, Resource, ResourceId};

use crate::Result;

/// Material: a shader plus per-sampler texture and sampler-state bindings
/// and a block of float4 constants.
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x06 | u16  | offset of import table |
/// | 0x08 | ptr  | shader (import) |
/// | 0x0C | ptr  | parameter indices (i8[n]) |
/// | 0x10 | ptr  | parameter flags (i8[n]) |
/// | 0x14 | ptr  | parameter name hashes (u32[n]) |
/// | 0x18 | ptr  | parameter data (float4[]) |
/// | 0x1C | u8   | parameter count |
/// | 0x1D | u8   | parameters with data |
/// | 0x20 | u8   | sampler count |
/// | 0x24 | ptr  | sampler keys (u16[s]), match `Shader::samplers[].key` |
/// | 0x28 | ptr  | sampler states (imports, ptr[s]) |
/// | 0x2C | ptr  | textures (imports, ptr[s]) |
#[derive(Debug, Clone)]
pub struct Material {
    pub id: ResourceId,
    pub shader: Option<ResourceId>,
    pub parameters: Vec<MaterialParameter>,
    pub samplers: Vec<MaterialSampler>,
}

#[derive(Debug, Clone)]
pub struct MaterialParameter {
    pub name_hash: u32,
    pub value: Option<[f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct MaterialSampler {
    pub key: u16,
    pub texture: Option<ResourceId>,
    pub sampler_state: Option<ResourceId>,
}

impl Material {
    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let n_params = r.u8(0x1C)? as usize;
        let mut parameters = Vec::with_capacity(n_params);
        if n_params > 0 {
            let (idx, hashes, data) = (r.u32(0x0C)? as usize, r.u32(0x14)? as usize, r.u32(0x18)? as usize);
            for i in 0..n_params {
                let slot = r.u8(idx + i)? as i8;
                let value = if slot >= 0 { Some(r.f32_array::<4>(data + slot as usize * 16)?) } else { None };
                parameters.push(MaterialParameter { name_hash: r.u32(hashes + i * 4)?, value });
            }
        }
        let n_samplers = r.u8(0x20)? as usize;
        let mut samplers = Vec::with_capacity(n_samplers);
        if n_samplers > 0 {
            let (keys, states, textures) = (r.u32(0x24)?, r.u32(0x28)?, r.u32(0x2C)?);
            for i in 0..n_samplers as u32 {
                samplers.push(MaterialSampler {
                    key: r.u16((keys + i * 2) as usize)?,
                    texture: res.import_at(textures + i * 4),
                    sampler_state: res.import_at(states + i * 4),
                });
            }
        }
        Ok(Self { id: res.id, shader: res.import_at(0x08), parameters, samplers })
    }
}

/// Shader: technique description, sampler names and the vertex layout.
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x08 | ptr  | name string |
/// | 0x14 | ptr  | parameter data slots (i8[n], -1 = no default) |
/// | 0x1C | ptr  | parameter name hashes (u32[n]) |
/// | 0x20 | ptr  | parameter defaults (float4[]) |
/// | 0x24 | u8   | parameter count |
/// | 0x28 | ptr  | parameter name strings (ptr[n]) |
/// | 0x5C | u8   | sampler count |
/// | 0x60 | ptr  | sampler keys (u16[s]) |
/// | 0x64 | ptr  | sampler channels (u8[s]) |
/// | 0x68 | ptr  | sampler name strings (ptr[s]) |
/// | 0x9C | ptr  | vertex descriptor (import) |
#[derive(Debug, Clone)]
pub struct Shader {
    pub id: ResourceId,
    pub name: String,
    pub vertex_descriptor: Option<ResourceId>,
    pub samplers: Vec<ShaderSampler>,
    pub parameters: Vec<ShaderParameter>,
}

/// A material constant the shader reads. Materials refer to it by `hash`.
#[derive(Debug, Clone)]
pub struct ShaderParameter {
    pub name: String,
    pub hash: u32,
    pub default: Option<[f32; 4]>,
}

#[derive(Debug, Clone)]
pub struct ShaderSampler {
    pub key: u16,
    pub channel: u8,
    pub name: String,
}

impl Shader {
    pub fn parse(res: &Resource) -> Result<Self> {
        let data = res.header();
        let r = Reader(data);
        let cstr = |off: usize| -> String {
            let s = data.get(off..).unwrap_or_default();
            let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            String::from_utf8_lossy(&s[..end]).into_owned()
        };
        let n = r.u8(0x5C)? as usize;
        let mut samplers = Vec::with_capacity(n);
        if n > 0 {
            let (keys, chans, names) = (r.u32(0x60)? as usize, r.u32(0x64)? as usize, r.u32(0x68)? as usize);
            for i in 0..n {
                samplers.push(ShaderSampler {
                    key: r.u16(keys + i * 2)?,
                    channel: r.u8(chans + i)?,
                    name: cstr(r.u32(names + i * 4)? as usize),
                });
            }
        }
        let n_params = r.u8(0x24)? as usize;
        let mut parameters = Vec::with_capacity(n_params);
        if n_params > 0 {
            let (slots, hashes, defaults, names) =
                (r.u32(0x14)? as usize, r.u32(0x1C)? as usize, r.u32(0x20)? as usize, r.u32(0x28)? as usize);
            for i in 0..n_params {
                let slot = r.u8(slots + i)? as i8;
                parameters.push(ShaderParameter {
                    name: cstr(r.u32(names + i * 4)? as usize),
                    hash: r.u32(hashes + i * 4)?,
                    default: if slot >= 0 { Some(r.f32_array(defaults + slot as usize * 16)?) } else { None },
                });
            }
        }
        Ok(Self {
            id: res.id,
            name: cstr(r.u32(0x08)? as usize),
            vertex_descriptor: res.import_at(0x9C),
            samplers,
            parameters,
        })
    }

    pub fn sampler_named(&self, name: &str) -> Option<&ShaderSampler> {
        self.samplers.iter().find(|s| s.name == name)
    }
}

/// Model: up to six LODs plus shadow / occlusion states, each mapping to one
/// of the imported renderables.
///
/// | off  | type | field |
/// |------|------|-------|
/// | 0x00 | ptr  | renderables (imports, ptr[n]) |
/// | 0x04 | ptr  | state → renderable index (u8[states], 0xFF = none) |
/// | 0x08 | ptr  | LOD distances squared (f32[lods]) |
/// | 0x0C | f32  | shadow distance squared |
/// | 0x14 | u8   | renderable count |
/// | 0x15 | u8   | flags |
/// | 0x16 | u8   | state count |
/// | 0x17 | u8   | version (5) |
/// | 0x18 | u8   | LOD count |
#[derive(Debug, Clone)]
pub struct Model {
    pub id: ResourceId,
    pub renderables: Vec<Option<ResourceId>>,
    /// Indexed by `EModelState`: LOD0..LOD5, SHADOW, OCCLUSION.
    pub state_renderables: Vec<Option<u8>>,
    pub lod_distances: Vec<f32>,
    pub flags: u8,
}

impl Model {
    pub const STATE_LOD0: usize = 0;
    pub const STATE_SHADOW: usize = 6;

    pub fn parse(res: &Resource) -> Result<Self> {
        let r = Reader(res.header());
        let (rend, states, lods) = (r.u32(0x00)?, r.u32(0x04)? as usize, r.u32(0x08)? as usize);
        let n_rend = r.u8(0x14)? as u32;
        let n_states = r.u8(0x16)? as usize;
        let n_lods = r.u8(0x18)? as usize;
        Ok(Self {
            id: res.id,
            renderables: (0..n_rend).map(|i| res.import_at(rend + i * 4)).collect(),
            state_renderables: (0..n_states)
                .map(|i| r.u8(states + i).map(|v| (v != 0xFF).then_some(v)))
                .collect::<mw_bundle::Result<_>>()?,
            lod_distances: (0..n_lods).map(|i| r.f32(lods + i * 4).map(f32::sqrt)).collect::<mw_bundle::Result<_>>()?,
            flags: r.u8(0x15)?,
        })
    }

    /// Renderable for a given LOD (0 = highest detail), falling back to the
    /// nearest available lower-detail LOD.
    pub fn renderable_for_lod(&self, lod: usize) -> Option<ResourceId> {
        self.state_renderables
            .iter()
            .take(6)
            .skip(lod)
            .flatten()
            .chain(self.state_renderables.iter().take(6).flatten())
            .find_map(|&i| self.renderables.get(i as usize).copied().flatten())
    }
}
