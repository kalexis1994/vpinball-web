// What happens after the table is drawn.
//
// The scene is rendered into a floating-point buffer with no ceiling on it, so
// a lit insert or the specular off the ball comes out at four or ten rather
// than being clipped at one. This file is where that extra light goes: the part
// above the threshold is cut out, blurred wide, and added back over the top,
// which is what makes a lamp spill past its own outline instead of stopping
// dead at the edge of its geometry. Only then is the whole thing tone mapped,
// once, on the way to the screen.
//
// It is a port of `FBShader.hlsl` — `bloom_cutoff` and `ps_main_fb_bloom` at
// line 305, the separable blurs at 714 and 839, `ReinhardToneMap` at
// `FBShader.fxh:42` — and of `Renderer::UpdateBloom` (`Renderer.cpp:2043`),
// which is the code that strings them together.
//
// # Why the tone mapping is here and not in the material shader
//
// It used to be in `material.wgsl`, and the comment there said it amounted to
// the same thing "as long as there is no bloom". That was true and is no longer
// true. The original's tone mapper carries the note `// overflow is handled by
// bloom` on all three of its overloads: it is written on the assumption that
// something downstream is catching what it lets through. Tone mapping before
// the bloom pass throws that light away, and then there is nothing left to
// bloom.

struct Post {
    // xy = one texel of the source, zw unused
    texel    : vec4<f32>,
    // x = bloom strength, y = exposure, z = whether bloom is live
    params   : vec4<f32>,
};

@group(0) @binding(0) var<uniform> post : Post;
@group(0) @binding(1) var source : texture_2d<f32>;
@group(0) @binding(2) var samp : sampler;
// Only the composite binds this one; the others get the same texture again,
// which costs nothing and keeps one bind group layout for every pass.
@group(0) @binding(3) var overlay : texture_2d<f32>;

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0)       uv   : vec2<f32>,
};

// One triangle covering the screen, from the vertex index alone. Cheaper than a
// quad and, more to the point, no vertex buffer to keep alive.
@vertex
fn vs_main(@builtin(vertex_index) i : u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out : VsOut;
    out.uv = uv;
    out.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return out;
}

// Reinhard, `FBShader.fxh:42`.
//
// The luminance weight is the Y row of the CIE RGB to XYZ matrix, and 0.25 is
// `BURN_HIGHLIGHTS` — how far a highlight is allowed to burn before it stops
// getting brighter. The cap keeps an infinity from propagating; the original
// notes it saw those drawn as black blobs on NVidia hardware.
fn tone_map(color : vec3<f32>) -> vec3<f32> {
    let l = min(dot(color, vec3<f32>(0.176204, 0.812985, 0.0108109)), 1000.0);
    return color * ((l * 0.25 + 1.0) / (l + 1.0));
}

// What counts as bright enough to bloom. `FBShader.hlsl:305`.
//
// Not a hard threshold: below 2.5 nothing blooms, above 2.5 everything above it
// does, and in the band between there is a quadratic knee so a highlight
// creeping up to the threshold fades in rather than snapping on. The tiny
// constant is the smallest normal 16-bit float, guarding the divisions.
fn cutoff(c : vec3<f32>) -> vec3<f32> {
    let threshold = 2.5;
    let knee = threshold * 1.0;
    let brightness = max(c.r, max(c.g, c.b));
    var soft = brightness - (threshold - knee);
    soft = clamp(soft, 0.0, 2.0 * knee);
    soft = soft * soft * (1.0 / (4.0 * knee + 0.00006103515625));
    let contribution = max(soft, brightness - threshold) / max(brightness, 0.00006103515625);
    return c * contribution;
}

// The bright pass, at a quarter of the resolution in each direction.
//
// Four taps on the diagonals — a box filter, which the original is careful to
// say is the right choice here and a gaussian is not: this is a downsample, and
// a gaussian downsample loses energy the box keeps.
@fragment
fn cut_off(in : VsOut) -> @location(0) vec4<f32> {
    let d = post.texel.xy;
    let s = (textureSample(source, samp, in.uv - d).rgb
           + textureSample(source, samp, in.uv + d).rgb
           + textureSample(source, samp, in.uv + vec2<f32>( d.x, -d.y)).rgb
           + textureSample(source, samp, in.uv + vec2<f32>(-d.x,  d.y)).rgb) * 0.25;
    let lit = cutoff(tone_map(s * post.params.y));
    return vec4<f32>(max(lit, vec3<f32>(0.0)) * post.params.x, 1.0);
}

// The separable blurs.
//
// Both are gaussians sampled between texels, so each tap costs one fetch and
// covers two: ten taps a side reach nineteen texels, five reach nine. The
// offsets are not integers for exactly that reason, and there is no centre tap
// — it falls out of the first pair.

// 39 x 39, for the bloom. `FBShader.hlsl:839`.
const WIDE_OFFSET = array<f32, 10>(
     0.66063,  2.46625,  4.43946,  6.41301,  8.38706,
    10.36173, 12.33715, 14.31341, 16.29062, 18.26884
);
const WIDE_WEIGHT = array<f32, 10>(
    0.13669, 0.15600, 0.10738, 0.05971, 0.02682,
    0.00973, 0.00285, 0.00067, 0.00013, 0.00002
);

// 19 x 19, for the light that comes through the plastics. `FBShader.hlsl:714`.
const NARROW_OFFSET = array<f32, 5>(0.65323, 2.42572, 4.36847, 6.31470, 8.26547);
const NARROW_WEIGHT = array<f32, 5>(0.19923, 0.18937, 0.08396, 0.02337, 0.00408);

fn blur_wide(uv : vec2<f32>, axis : vec2<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    for (var i = 0; i < 10; i++) {
        let d = axis * post.texel.xy * WIDE_OFFSET[i];
        result += (textureSample(source, samp, uv + d).rgb
                 + textureSample(source, samp, uv - d).rgb) * WIDE_WEIGHT[i];
    }
    return result;
}

fn blur_narrow(uv : vec2<f32>, axis : vec2<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    for (var i = 0; i < 5; i++) {
        let d = axis * post.texel.xy * NARROW_OFFSET[i];
        result += (textureSample(source, samp, uv + d).rgb
                 + textureSample(source, samp, uv - d).rgb) * NARROW_WEIGHT[i];
    }
    return result;
}

@fragment
fn wide_h(in : VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_wide(in.uv, vec2<f32>(1.0, 0.0)), 1.0);
}

@fragment
fn wide_v(in : VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_wide(in.uv, vec2<f32>(0.0, 1.0)), 1.0);
}

@fragment
fn narrow_h(in : VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_narrow(in.uv, vec2<f32>(1.0, 0.0)), 1.0);
}

@fragment
fn narrow_v(in : VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur_narrow(in.uv, vec2<f32>(0.0, 1.0)), 1.0);
}

// Scene plus bloom, tone mapped, out to the screen.
// `ps_main_fb_rhtonemap`, `FBShader.hlsl:354`.
//
// The result is written linear: the swap chain is viewed through an sRGB
// format, so the hardware does the encode. The original applies its own gamma
// here because it is writing to a plain buffer.
@fragment
fn composite(in : VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(source, samp, in.uv).rgb;
    if (post.params.z != 0.0) {
        c += textureSample(overlay, samp, in.uv).rgb;
    }
    return vec4<f32>(tone_map(c * post.params.y), 1.0);
}
