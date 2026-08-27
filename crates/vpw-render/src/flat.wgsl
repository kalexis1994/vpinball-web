// The flat engine's shaders: the baked table drawn as pictures.
//
// Four small stages around one idea — light is additive, so the table can be
// photographed once dark and once per lamp, and the difference of the two
// photographs *is* that lamp's light. See `flat.rs` for the whole story.
//
// Two bind group layouts share this module. The live one carries the baked
// base, its depth and the sprite atlas; the bake one carries the freshly lit
// photograph, the base to subtract, and the little uniform that maps an atlas
// slot back onto the screen.

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0)       uv   : vec2<f32>,
};

// One triangle covering the screen, the same as `post.wgsl`.
@vertex
fn vs_full(@builtin(vertex_index) i : u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out : VsOut;
    out.uv = uv;
    out.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

// ---- the live frame --------------------------------------------------------

@group(0) @binding(0) var flat_samp : sampler;
@group(0) @binding(1) var base_tex : texture_2d<f32>;
// A colour texture holding depth, not a depth texture: the WebGL2 backend
// cannot fetch from those, and this engine exists for exactly the machines
// that end up on that backend.
@group(0) @binding(2) var base_depth : texture_2d<f32>;
@group(0) @binding(3) var atlas : texture_2d_array<f32>;

struct BaseOut {
    @location(0)          color : vec4<f32>,
    @builtin(frag_depth)  depth : f32,
};

// The whole static table in one draw: its photograph, and — through
// `frag_depth` — the depth it was photographed with, so the pieces drawn live
// afterwards hide behind a post exactly as they would have behind the post's
// geometry.
@fragment
fn fs_base(in : VsOut) -> BaseOut {
    var out : BaseOut;
    out.color = textureSampleLevel(base_tex, flat_samp, in.uv, 0.0);
    out.depth = textureLoad(base_depth, vec2<i32>(in.clip.xy), 0).r;
    return out;
}

// One lamp's light, stretched from its atlas slot back over the piece of
// screen it was photographed from, scaled by how lit the lamp is right now,
// and *added* — the blend state carries the arithmetic.
struct LayerIn {
    @builtin(vertex_index) corner : u32,
    // Screen rect in clip space: (x0, y0) bottom-left, (x1, y1) top-right.
    @location(0) rect : vec4<f32>,
    // The matching atlas rect, normalised, y down.
    @location(1) uv_rect : vec4<f32>,
    // x = atlas layer, y = the lamp's level as a fraction of its bake.
    @location(2) info : vec2<f32>,
};

struct LayerOut {
    @builtin(position) clip  : vec4<f32>,
    @location(0)       uv    : vec2<f32>,
    @location(1)       layer : f32,
    @location(2)       scale : f32,
};

@vertex
fn vs_layer(in : LayerIn) -> LayerOut {
    // Two triangles from six vertex indices, no vertex buffer.
    let k = vec2<f32>(
        f32(in.corner == 1u || in.corner == 2u || in.corner == 4u),
        f32(in.corner == 2u || in.corner == 4u || in.corner == 5u),
    );
    var out : LayerOut;
    out.clip = vec4<f32>(mix(in.rect.xy, in.rect.zw, k), 0.0, 1.0);
    // Clip y goes up, texture y goes down: the top of the slot is the top of
    // the rect.
    out.uv = mix(in.uv_rect.xy, in.uv_rect.zw, vec2<f32>(k.x, 1.0 - k.y));
    out.layer = in.info.x;
    out.scale = in.info.y;
    return out;
}

@fragment
fn fs_layer(in : LayerOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(atlas, flat_samp, in.uv, i32(in.layer), 0.0);
    return vec4<f32>(c.rgb * in.scale, 1.0);
}

// ---- the bake --------------------------------------------------------------

struct Slot {
    // Screen uv of the photographed rect: origin, then size.
    origin : vec4<f32>,
    // xy = the slot's top-left in atlas pixels, zw = uv-per-atlas-pixel.
    step   : vec4<f32>,
};

// Bindings 4-7, not 0-3: the live half above already claims those, and two
// module-scope variables on one binding point — even in entry points that
// never meet — is something the browser's compiler refuses and the native
// one does not.
@group(0) @binding(4) var bake_samp : sampler;
@group(0) @binding(5) var lit_tex : texture_2d<f32>;
@group(0) @binding(6) var dark_tex : texture_2d<f32>;
@group(0) @binding(7) var<uniform> slot : Slot;

// The base photograph moved into its keeping texture. A render rather than a
// copy, because the HDR buffer it comes from was not created copyable.
@fragment
fn fs_blit(in : VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(lit_tex, bake_samp, in.uv, 0.0);
}

// The lamp itself: everything this photograph shows that the dark one does
// not. Negative where the lamp *darkens* — a modulating bulb halo does — and
// the additive replay subtracts there, which is the point of keeping the
// difference in a float texture.
@fragment
fn fs_diff(in : VsOut) -> @location(0) vec4<f32> {
    let uv = slot.origin.xy + (in.clip.xy - slot.step.xy) * slot.step.zw * slot.origin.zw;
    let lit = textureSampleLevel(lit_tex, bake_samp, uv, 0.0);
    let dark = textureSampleLevel(dark_tex, bake_samp, uv, 0.0);
    return vec4<f32>(lit.rgb - dark.rgb, 1.0);
}
