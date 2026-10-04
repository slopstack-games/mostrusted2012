// Motion-blurred sparks: a port of the game's `VfxParticles_MotionBlurSpriteUntex`
// (`mwtool shader-asm VfxParticles_MotionBlurSpriteUntex`). Each spark is a disc that moved from `end` to
// `start` during the frame; the pixel program integrates the disc's (1 - r^2) profile over the time the
// pixel spends inside it, so a fast spark becomes a soft streak. The game's CPU renderer hands the
// vertex program screen-space quads; here the vertex program builds them from the two world positions.
//
// Game pixel program: p = pixel relative to the start disc, d = displacement (both in disc radii);
// t0, t1 = saturate(roots of |p - t d| = 1); coverage = (t1 - t0)(1 - |p|^2) + (t1^2 - t0^2) p.d
// - (t1^3 - t0^3)|d|^2 / 3; alpha = coverage x colour.a (discard below 1e-4); rgb = colour
// x Overbright x gVfxSparkBrightness. Blend SRC_ALPHA, ONE (additive).

#import bevy_pbr::mesh_view_bindings::view

struct SparkParams {
    // x = Overbright x spark brightness
    scale: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SparkParams;

struct Vertex {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    // corner: x = -1 at the end disc, +1 at the start disc; y = -1 / +1 across
    @location(2) corner: vec2<f32>,
    // x = radius (m)
    @location(3) size: vec2<f32>,
    @location(5) colour: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
    // xy = pixel relative to the start disc, zw = end - start (disc radii)
    @location(1) disc: vec4<f32>,
};

fn to_pixels(c: vec4<f32>, size: vec2<f32>) -> vec2<f32> {
    return (c.xy / c.w * vec2<f32>(0.5, -0.5) + 0.5) * size;
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let cs = view.clip_from_world * vec4<f32>(v.start, 1.0);
    var ce = view.clip_from_world * vec4<f32>(v.end, 1.0);
    if ce.w < 0.05 {
        ce = cs;
    }
    // Fade sparks out within a metre of the camera (not in the game's program: its sparks rarely get
    // this close; here the chase camera can sit at the contact point).
    out.colour = vec4<f32>(v.colour.rgb * params.scale.x, v.colour.a * saturate(cs.w - 0.5));
    if cs.w < 0.05 {
        // Behind the camera: outside the depth range, clipped.
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        out.disc = vec4<f32>(0.0);
        return out;
    }
    let size = view.viewport.zw;
    // The start disc is the head of the streak.
    let ps = to_pixels(cs, size);
    let pe = to_pixels(ce, size);
    let r = max(v.size.x * view.clip_from_view[1][1] * 0.5 * size.y / cs.w, 0.75);
    let axis = ps - pe;
    let len = length(axis);
    var dir = vec2<f32>(1.0, 0.0);
    if len > 1e-3 {
        dir = axis / len;
    }
    let perp = vec2<f32>(-dir.y, dir.x);
    var base = pe - dir * r;
    if v.corner.x > 0.0 {
        base = ps + dir * r;
    }
    let px = base + perp * v.corner.y * r;
    let ndc = (px / size - 0.5) * vec2<f32>(2.0, -2.0);
    out.clip = vec4<f32>(ndc * cs.w, cs.z, cs.w);
    out.disc = vec4<f32>((px - ps) / r, (pe - ps) / r);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.disc.xy;
    let d = in.disc.zw;
    let pp = dot(p, p) - 1.0;
    let dd = max(dot(d, d), 1e-6);
    let pd = dot(p, d);
    let disc = sqrt(max(pd * pd - pp * dd, 0.0));
    let t = saturate(vec2<f32>(-disc - pd, disc - pd) / dd);
    let t0 = t.x;
    let t1 = t.y;
    var cover = (t0 - t1) * pp + pd * (t0 * t0 - t1 * t1);
    cover = cover + (t0 * t0 * t0 - t1 * t1 * t1) * dd / 3.0;
    // The integral runs over the frame's time [0, 1]; a still spark (d = 0) is a plain disc.
    if dot(in.disc.zw, in.disc.zw) < 1e-6 {
        cover = max(-pp, 0.0);
    }
    if cover < 0.000392 {
        discard;
    }
    return vec4<f32>(in.colour.rgb, max(cover, 0.0) * in.colour.a);
}
