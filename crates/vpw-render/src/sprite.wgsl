// A picture on the screen: a textured rectangle with no depth.
//
// Two things are drawn with it. The table's backdrop, which the original
// draws as a full-screen sprite with the depth test off before a single part
// of the table (`Renderer::DrawBackground`, `Renderer.cpp:921`) — it is how a
// table dresses what is *around* its playfield. And the score, in the windows
// the backdrop has for it, which the original leaves to a second window and
// this port draws in place. See `crate::overlay`.
//
// The rectangle and the part of the picture it shows are a uniform, so the
// same six vertices serve every sprite and no vertex buffer is needed.

struct Sprite {
    // Left, top, width, height, as fractions of the screen, y down.
    rect : vec4<f32>,
    // The same for the part of the texture to show.
    uv : vec4<f32>,
};

@group(0) @binding(0) var picture : texture_2d<f32>;
@group(0) @binding(1) var picture_samp : sampler;
@group(0) @binding(2) var<uniform> sprite : Sprite;

struct Out {
    @builtin(position) clip : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i : u32) -> Out {
    // Two triangles over the unit square, corner k at (k & 1, k >> 1).
    var order = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let k = order[i];
    let corner = vec2<f32>(f32(k & 1u), f32(k >> 1u));
    let at = sprite.rect.xy + corner * sprite.rect.zw;
    var out : Out;
    // Screen fractions run y down; clip space runs y up.
    out.clip = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = sprite.uv.xy + corner * sprite.uv.zw;
    return out;
}

@fragment
fn fs_main(in : Out) -> @location(0) vec4<f32> {
    return textureSampleLevel(picture, picture_samp, in.uv, 0.0);
}
