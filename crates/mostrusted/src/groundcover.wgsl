// The `Groundcover` vertex program, ported from its DXBC (`mwtool shader-asm Groundcover`), plus the CPU
// builder's per-sprite range test and shrink (`FUN_00eb9500`), which the game applies when it writes the
// sprite vertices each frame. `materials::Layered::specialize` swaps this in for the meshes that carry the
// sprite attribute (`loader::groundcover_mesh`); the fragment side is `layered.wgsl` mode 12.
//
// Per vertex (all four corners of a sprite share everything but the corner index):
//   0 POSITION  the instance position (the sprite's base)
//   1 NORMAL    the ground normal (instance bytes +8..+10 / 127)
//   2 UV_0      the atlas cell corner: region.xy + region.zw × (u, 1 − v) (the program's TEXCOORD output)
//   3 COLOR     2 × tint (the program outputs sqrt(2 tint), the G-buffer holds sqrt(albedo))
//   4 sprite    (width byte, height byte, 2^e, corner 0..3): the corners are (u, v) = (0,0) (1,0) (1,1) (0,1)

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import bevy_render::globals::Globals

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
// The prepass view layout has the globals at binding 1 (as in layered.wgsl).
@group(0) @binding(1) var<uniform> prepass_globals: Globals;
fn time() -> f32 {
    return prepass_globals.time;
}
#else
#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings::globals}
fn time() -> f32 {
    return globals.time;
}
#endif

// The head of `layered.wgsl`'s material uniform: mode 12 keeps the shader's `gMaterialCB` in p0 =
// (amount, speed, 0, 0).
struct LayeredHead {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    p0: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> layered: LayeredHead;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) sprite: vec4<f32>,
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let p = (world_from_local * vec4(v.position, 1.0)).xyz;
    let eye = view.world_position;

    // CPU builder (`FUN_00eb9500`): a sprite is drawn when it is in front of the camera and within
    // R = ((w >> 1 & 63) + (h >> 1 & 63)) × 2^e / 127 × 125 of it (`DAT_010c6f74`), its width and height
    // bytes scaled by min(5 (1 − d² / R²), 1) (`DAT_01081000`) and truncated to whole bytes.
    let wb = i32(v.sprite.x);
    let hb = i32(v.sprite.y);
    let scale = v.sprite.z;
    let range = f32(((wb >> 1u) & 63) + ((hb >> 1u) & 63)) * scale * (1.0 / 127.0) * 125.0;
    let d = p - eye;
    let d2 = dot(d, d);
    let forward = -view.world_from_view[2].xyz;
    let r2 = range * range;
    let drawn = dot(d, forward) >= 0.0 && d2 <= r2 && r2 > 0.0;
    let shrink = min((1.0 - d2 / r2) * 5.0, 1.0);
    // BINORMAL.xy = byte / 127 (`DAT_01081004`), × `GroundcoverConstants.x` = 2^e in the program.
    let width = trunc(f32(wb) * shrink) * (1.0 / 127.0) * scale;
    let height = trunc(f32(hb) * shrink) * (1.0 / 127.0) * scale;

    let c = u32(v.sprite.w + 0.5);
    let u = select(0.0, 1.0, c == 1u || c == 2u);
    let vv = select(0.0, 1.0, c >= 2u);

    // The vertex program: sway the top along the sprite's right axis, turn the sprite about world up to
    // face the camera (right = perpendicular to the horizontal direction from the camera), anchor it at
    // its base.
    let sway = sin(time() * layered.p0.y + p.x) * layered.p0.x;
    let lateral = sway * vv + width * (u - 0.5);
    var dir = p.xz - eye.xz;
    dir = select(vec2(0.0, 1.0), normalize(dir), dot(dir, dir) > 1e-12);
    let right = vec3(-dir.y, 0.0, dir.x);
    let world = p + right * lateral + vec3(0.0, height * vv, 0.0);

    // Its normal: normalize(sign(w) right (u − 0.5) + up v − 0.1 dir) + max(8 − height, 0.75) × (ground
    // normal + (0, 0.25, 0)); the pixel program normalises the sum.
    let around = sign(width) * right * (u - 0.5) + vec3(0.0, vv, 0.0) - 0.1 * vec3(dir.x, 0.0, dir.y);
    let n = normalize(around) + max(8.0 - height, 0.75) * (v.normal + vec3(0.0, 0.25, 0.0));

    out.world_position = vec4(world, 1.0);
    out.position = position_world_to_clip(world);
    if !drawn {
        // Not emitted by the builder: a degenerate triangle.
        out.position = vec4(0.0, 0.0, 0.0, 1.0);
    }
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = n;
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.previous_world_position = out.world_position;
#endif
#else
    out.world_normal = n;
#endif
#ifdef VERTEX_COLORS
    out.color = v.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = v.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        v.instance_index, world_from_local[3]);
#endif
    return out;
}
