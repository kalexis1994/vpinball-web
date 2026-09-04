// The picture a table wants behind everything.
//
// `Renderer::DrawBackground` (`Renderer.cpp:921`): the view mode's backdrop
// image, drawn as a full-screen sprite with the depth test off, before a
// single part of the table. It is how a table dresses what is *around* its
// playfield — on Circus, the whole apron end of the cabinet, which has no
// geometry at all and would otherwise be a hole.
//
// A fullscreen triangle rather than a quad: three vertices, no buffer, no
// seam down the diagonal.

@group(0) @binding(0) var backdrop : texture_2d<f32>;
@group(0) @binding(1) var backdrop_samp : sampler;

struct Out {
    @builtin(position) clip : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i : u32) -> Out {
    // (-1,-1), (3,-1), (-1,3) — one triangle covering the screen.
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var out : Out;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    // The picture is stretched to the screen, which is what a sprite drawn
    // from (0,0) to (1,1) does. Its own aspect is not kept, and the original
    // does not keep it either.
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_main(in : Out) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(backdrop, backdrop_samp, in.uv, 0.0).rgb, 1.0);
}
