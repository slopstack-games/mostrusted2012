// Corona sprites (`coronas.rs`): the game's `Vfx_Corona` (glows, beams) and `Vfx_CoronaFlare` (flares).
// The CPU builds every sprite as a world-space quad with its colour in game HDR units.
//
// `Vfx_Corona`: the vertex program pulls the sprite's depth `w` metres along the view axis (−0.5: towards the
// camera), blended in by sat((biased view depth − 0.01) / 2); the pixel program writes colour × texture ×
// (alpha × texture alpha), added (ONE, INV_SRC_ALPHA with alpha 0), depth-tested without writing.
// `Vfx_CoronaFlare`: no depth test; the alpha is scaled by the corona's visibility, the fraction of a 7 × 7
// grid of depth samples one pixel apart around the test point that lies behind it (`Vfx_CoronaVisibilityTest`;
// the game renders that into a texture the flares read, here it is evaluated per pixel from the depth prepass).

#import bevy_pbr::mesh_view_bindings::view
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct CoronaParams {
    // x = cd/m² per game unit, y = tap spacing (pixels), z = flare (visibility test, no depth bias)
    misc: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CoronaParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // glows / beams: w = depth bias (m); flares: xyz = visibility test point (world)
    @location(4) extra: vec4<f32>,
    @location(5) colour: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) uv: vec2<f32>,
    // flares: xy = the test point in pixels, z = its depth (reverse-Z NDC), w = on screen
    @location(2) test: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = vec4<f32>(v.position, 1.0);
    out.clip = view.clip_from_world * world;
    out.colour = v.colour;
    out.uv = v.uv;
    out.test = vec4<f32>(0.0);
    if params.misc.z > 0.5 {
        let c = view.clip_from_world * vec4<f32>(v.extra.xyz, 1.0);
        let ndc = c.xyz / max(c.w, 1e-6);
        let px = (ndc.xy * vec2<f32>(0.5, -0.5) + 0.5) * view.viewport.zw + view.viewport.xy;
        out.test = vec4<f32>(px, ndc.z, select(0.0, 1.0, c.w > 0.0));
    } else if v.extra.w != 0.0 {
        // Bevy's view space looks down −z: the game's z + w is z − w here.
        var vp = view.view_from_world * world;
        let vz = -vp.z - v.extra.w;
        vp.z = -vz;
        let b = view.clip_from_view * vp;
        if b.w > 0.01 {
            let t = saturate((b.w - 0.01) * 0.5);
            out.clip.z = mix(out.clip.z, b.z / b.w * out.clip.w, t);
        }
    }
    return out;
}

fn visibility(c: vec4<f32>) -> f32 {
#ifdef DEPTH_PREPASS
    if c.w < 0.5 {
        return 0.0;
    }
    let r = params.misc.y;
    let lo = vec2<f32>(4.0) + view.viewport.xy;
    let hi = view.viewport.xy + view.viewport.zw - vec2<f32>(4.0);
    let centre = clamp(c.xy, lo, hi);
    var seen = 0.0;
    for (var y = -3; y <= 3; y++) {
        for (var x = -3; x <= 3; x++) {
            let p = centre + vec2<f32>(f32(x), f32(y)) * r;
            // Reverse-Z: the corona is seen where the scene is not nearer than it.
            let scene = prepass_depth(vec4<f32>(p, 0.0, 1.0), 0u);
            seen += select(0.0, 1.0, scene <= c.z);
        }
    }
    return seen / 49.0;
#else
    return 1.0;
#endif
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(diffuse, diffuse_sampler, in.uv);
    var a = in.colour.a * t.a;
    if params.misc.z > 0.5 {
        a *= visibility(in.test);
    }
    if a <= 0.0 {
        discard;
    }
    return vec4<f32>(in.colour.rgb * t.rgb * a * params.misc.x * view.exposure, 0.0);
}
