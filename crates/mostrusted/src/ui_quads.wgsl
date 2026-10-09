// UI quads drawn with the game's additive UI blend (`UIAdditiveShader`, see ui_quads.rs): the texel times the
// element's tint and opacity; the blend state (src alpha, one) does the adding.
#import bevy_ui::ui_vertex_output::UiVertexOutput

struct QuadParams {
    // Linear RGBA: tint x opacity.
    colour: vec4<f32>,
    // u0, v0, u1, v1 (flips are swapped ends).
    uv: vec4<f32>,
}

@group(1) @binding(0) var<uniform> params: QuadParams;
@group(1) @binding(1) var quad_texture: texture_2d<f32>;
@group(1) @binding(2) var quad_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let uv = mix(params.uv.xy, params.uv.zw, in.uv);
    return textureSample(quad_texture, quad_sampler, uv) * params.colour;
}
