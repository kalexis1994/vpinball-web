// A flasher: a lit polygon blended over the table.
//
// Port of `fs_flasher.sc` (the bgfx build of `FlasherShader.hlsl`) and, for
// the display mode, of the legacy dot-matrix path in `fs_dmd.sc`. The blend
// helpers are `common.sh:381-452` (`Helpers.fxh` in the DirectX build).
//
// There is no lighting in here at all: a flasher is not a surface, it is the
// light. What it does is pick a colour — from one picture, from two pictures
// combined, or from the flasher's own colour when it has none — and hand it to
// the blend, which is where an additive flasher and a painted one part ways.

struct Frame {
    view_proj  : mat4x4<f32>,
    eye        : vec4<f32>,
    ambient    : vec4<f32>,
    light0     : vec4<f32>,
    light1     : vec4<f32>,
    emission   : vec4<f32>,
    env        : vec4<f32>,
    screen     : vec4<f32>,
    clip       : vec4<f32>,
    mirror     : vec4<f32>,
};

struct FlasherData {
    // Table space to world space: the outline lies flat, this lifts and turns
    // it (`Flasher::transform`).
    model  : mat4x4<f32>,
    // `staticColor_Alpha`: rgb = the flasher's colour, a = opacity times
    // intensity scale, out of one.
    color  : vec4<f32>,
    // `alphaTestValueAB_filterMode_addBlend`: x, y = the alpha below which a
    // texel of A / B is thrown away (negative: never), z = the filter,
    // w = whether the blend is additive.
    tests  : vec4<f32>,
    // `amount_blend_modulate_vs_add_flasherMode`: x = filter amount, y =
    // modulate-vs-add, z = 0 one picture, 1 two, 2 none; w = the frame count,
    // which only the display path reads, for its dither.
    blend  : vec4<f32>,
    // Display mode only, `vRes_Alpha_time`: xy = dots across and down,
    // z = opacity.
    res    : vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame : Frame;
@group(1) @binding(0) var<uniform> f : FlasherData;
@group(1) @binding(1) var tex_a : texture_2d<f32>;
@group(1) @binding(2) var tex_b : texture_2d<f32>;
@group(1) @binding(3) var samp  : sampler;

struct VsOut {
    @builtin(position) clip : vec4<f32>,
    @location(0)       uv   : vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos : vec3<f32>, @location(1) uv : vec2<f32>) -> VsOut {
    var out : VsOut;
    out.clip = frame.view_proj * (f.model * vec4<f32>(pos, 1.0));
    out.uv = uv;
    return out;
}

// `common.sh:381`.
fn additive(base : vec4<f32>, blend : vec4<f32>, amount : f32) -> vec4<f32> {
    return base + blend * amount;
}

// `common.sh:406`.
fn multiply(base : vec4<f32>, blend : vec4<f32>, amount : f32) -> vec4<f32> {
    return base * blend * amount;
}

// `ScreenHDR`, `common.sh:401`: screen, floored at zero because the inputs
// may be above one.
fn screen_hdr(base : vec4<f32>, blend : vec4<f32>) -> vec4<f32> {
    return max(1.0 - (1.0 - base) * (1.0 - blend), vec4<f32>(0.0));
}

// `OverlayHDR`, `common.sh:433`: multiply where the base is dark, screen where
// it is bright, decided per channel at a half.
fn overlay_hdr(base : vec4<f32>, blend : vec4<f32>) -> vec4<f32> {
    let pick = step(vec4<f32>(0.5), base);
    let mixed = mix(base * blend * 2.0, 1.0 - 2.0 * (1.0 - base) * (1.0 - blend), pick);
    return max(mixed, vec4<f32>(0.0));
}

// The picture mode. `fs_flasher.sc:31-86`.
//
// Both textures are sampled before anything is decided. The original samples
// each behind a branch on the mode, which is uniform and legal in GLSL; a
// browser's WGSL uniformity analysis is stricter about implicit derivatives,
// and a `textureSample` it decides is in non-uniform control flow is a shader
// that silently never compiles. Two samples for a handful of flashers is
// nothing; a black table with no error is a bad afternoon.
@fragment
fn fs_flasher(in : VsOut) -> @location(0) vec4<f32> {
    let mode = f.blend.z;
    let p1 = textureSample(tex_a, samp, in.uv);
    let p2 = textureSample(tex_b, samp, in.uv);

    // The alpha test: a texel cut out of its picture contributes nothing,
    // neither colour nor alpha, so the blend leaves the pixel alone whichever
    // blend it is.
    if (mode < 2.0 && p1.a <= f.tests.x) {
        return vec4<f32>(0.0);
    }
    if (mode == 1.0 && p2.a <= f.tests.y) {
        return vec4<f32>(0.0);
    }

    // Mode 2 wires the flat colour straight through.
    var result = f.color;
    if (mode == 0.0) {
        result *= p1;
    }
    if (mode == 1.0) {
        let which = f.tests.z;
        let amount = f.blend.x;
        if (which == 2.0) {
            result *= overlay_hdr(p1, p2);
        }
        if (which == 3.0) {
            result *= multiply(p1, p2, amount);
        }
        if (which == 1.0) {
            result *= additive(p1, p2, amount);
        }
        if (which == 4.0) {
            result *= screen_hdr(p1, p2);
        }
        // `Filter_None` leaves the colour untouched by either picture, and
        // that is the original's behaviour too: nothing in its chain of
        // `if`s matches zero.
    }

    if (f.tests.w == 0.0) {
        // Painted over: plain alpha blend. "Need to clamp here or we get some
        // saturation artifacts on some tables."
        return vec4<f32>(result.rgb, saturate(result.a));
    }
    // Additive, through the same reverse-subtract blend the bulb light uses
    // (`light.wgsl`, `fs_bulb`): a negative colour and an alpha of `1/m - 1`,
    // so the hardware computes `dst · (1 + m·C) + (1 - m)·C`. At `m` near
    // zero that adds the light; at `m` near one it multiplies what is under
    // it by one plus the light, which is what a flash *does* to artwork.
    let m = f.blend.y;
    return vec4<f32>(result.rgb * (-m * result.a), 1.0 / m - 1.0);
}

// ---------------------------------------------------------------------------
// The display
// ---------------------------------------------------------------------------
//
// `fs_dmd.sc`, the legacy dot-matrix renderer, which is the one the original
// still uses for every profile but the newest ("Legacy" is style zero and the
// default). It does not draw a texture of dots: it draws the *dots*, as round
// spots inside each texel, and oversamples them thirteen times with a jittered
// triangle filter so the grid neither aliases at a distance nor turns into
// blurred squares up close.

// `hash22`, `fs_dmd.sc:51`.
fn hash22(uv : vec2<f32>) -> vec2<f32> {
    var p3 = fract(uv.xyx * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// `triangularPDF`, `fs_dmd.sc:66`: a uniform 0..1 turned into a -1..1 sample
// with a triangle distribution centred on zero.
fn triangular(r : f32) -> f32 {
    var p = 2.0 * r;
    let upper = p > 1.0;
    if (upper) {
        p = 2.0 - p;
    }
    p = 1.0 - sqrt(p);
    return select(-p, p, upper);
}

@fragment
fn fs_dmd(in : VsOut) -> @location(0) vec4<f32> {
    // "1.0..2.0 looks best (between sharp and blurry), and 1.5 matches the
    // intention of the triangle filter."
    let blur = 1.5;
    let ddxs = dpdx(in.uv) * blur;
    let ddys = dpdy(in.uv) * blur;
    // Fades the round dots to plain squares as they shrink on screen, which
    // is less aliasing than thirteen samples can otherwise hide.
    let dist_factor = clamp(1.0 - length(ddxs + ddys) * 6.66, 0.4, 1.0);
    // A new jitter every frame, so the sampling noise moves rather than sits.
    let offs = hash22(in.uv + f.blend.w);

    var color2 = vec3<f32>(0.0);
    let samples = 13.0;
    for (var i = 0; i < 13; i++) {
        let fi = f32(i);
        // Korobov / Fibonacci lattice: 1 and 8 over 13.
        let xi = vec2<f32>(fract(fi * (1.0 / samples) + offs.x), fract(fi * (8.0 / samples) + offs.y));
        let uv = in.uv + triangular(xi.x) * ddxs + triangular(xi.y) * ddys;
        let rgba = textureSampleLevel(tex_a, samp, uv, 0.0);
        // The dot inside its texel: a disc of radius one over 1.1, soft at the
        // edge.
        let dist = (fract(uv * f.res.xy) * 2.2 - 1.1) * dist_factor;
        let r2 = dist.x * dist.x + dist.y * dist.y;
        let d = smoothstep(0.0, 1.0, 1.0 - r2 * r2);
        // The frame is luminance in every channel, so `r` is the dot's level
        // whether the source was a one-channel frame or a coloured one.
        color2 += vec3<f32>(rgba.r) * d;
    }
    color2 *= f.color.rgb * ((1.0 / samples) * dist_factor * dist_factor);
    // Linear, and no gamma: the frame is decoded on the way out of the
    // texture, which is where the original says it moved that step to.
    return vec4<f32>(color2, f.res.z);
}
