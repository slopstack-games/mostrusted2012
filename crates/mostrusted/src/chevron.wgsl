// Race route markers: ports of the game's `Chevron` and `ChevronBlockRoad` programs
// (`mwtool shader-asm Chevron`, `mwtool shader-asm ChevronBlockRoad`).
//
// Chevron (scrolling arrows): u' = u - frac(Time x speed), v' = v; c = texture(u', v') x baseColour x
// materialDiffuse; out = (c.rgb x c.a, c.a).
// ChevronBlockRoad (crosses): fade = 1 - 0.002 x |world position - ViewPosition|; out = (texture.rgb x
// materialDiffuse.rgb, texture.a x fade).
// The colours are in the game's units, which is what the HDR target holds (weather.rs GAME_UNIT).

#import bevy_pbr::mesh_view_bindings::{view, globals}

struct ChevronParams {
    // baseColour x materialDiffuse
    colour: vec4<f32>,
    // x = speed, y = 1 for the block-road program
    mode: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ChevronParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tex_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fade: f32,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.clip = view.clip_from_world * vec4<f32>(v.position, 1.0);
    out.uv = v.uv;
    if params.mode.y > 0.5 {
        out.fade = 1.0 - distance(v.position, view.world_position) * 0.002;
    } else {
        out.uv.x = v.uv.x - fract(globals.time * params.mode.x);
        out.fade = 1.0;
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(tex, tex_sampler, in.uv);
    if params.mode.y > 0.5 {
        let a = t.a * saturate(in.fade);
        return vec4<f32>(t.rgb * params.colour.rgb * a, a);
    }
    let c = t * params.colour;
    return vec4<f32>(c.rgb * c.a, c.a);
}
