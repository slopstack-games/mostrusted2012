// UI quads placed in the world (world_ui.rs): drawn after the tonemap, depth-tested against the scene, as
// the game's `UIDepthTestedShader` (texture x colour, the scene depth compared by hand in the UI pass).

struct View {
    clip_from_world: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.clip = view.clip_from_world * vec4<f32>(v.position, 1.0);
    out.uv = v.uv;
    out.colour = v.colour;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, tex_sampler, in.uv) * in.colour;
}
