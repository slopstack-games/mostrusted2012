// Debug probe for sudden brightness jumps (`MW_FLASH_LOG`, tonemap.rs): one texel per cell of a 32 × 32
// grid over the frame, read back to the CPU every frame. Not part of the game's pipeline.
//   x: mean displayed luma of the cell (the tonemapped image, linear)
//   y: pixels of the HDR target with a non-finite channel (NaN / Inf)
//   z: brightest finite HDR luma in the cell (game units)
//   w: the adapted exposure (log2) this frame

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var shown: texture_2d<f32>;
@group(0) @binding(2) var state: texture_2d<f32>;

const GRID: u32 = 32u;

fn finite(c: vec3<f32>) -> bool {
    let e = bitcast<vec3<u32>>(c) & vec3(0x7f800000u);
    return all(e != vec3(0x7f800000u));
}

@fragment
fn probe(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let cell = vec2<u32>(in.position.xy);
    let size = textureDimensions(hdr);
    let lo = cell * size / GRID;
    let hi = max((cell + 1u) * size / GRID, lo + 1u);
    var bad = 0.0;
    var peak = 0.0;
    var sum = 0.0;
    var n = 0.0;
    for (var y = lo.y; y < hi.y; y += 1u) {
        for (var x = lo.x; x < hi.x; x += 1u) {
            let c = textureLoad(hdr, vec2(x, y), 0).rgb;
            if finite(c) {
                peak = max(peak, dot(c, vec3(0.2126, 0.7152, 0.0722)));
            } else {
                bad += 1.0;
            }
            sum += dot(textureLoad(shown, vec2(x, y), 0).rgb, vec3(0.2126, 0.7152, 0.0722));
            n += 1.0;
        }
    }
    return vec4(sum / n, bad, peak, textureLoad(state, vec2(0), 0).x);
}
