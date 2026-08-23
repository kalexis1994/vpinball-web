// A light's halo.
//
// Port of `ClassicLightShader.hlsl:75-82`. The shape of the light —its outline—
// already arrives triangulated; what this shader does is paint a halo over it
// that fades out with the distance to the center, and add it to what is already
// drawn.
//
// The color is not one single color: there is one for the center and another
// for the edge, and it interpolates between the two by the **square root** of
// the distance, not by the distance — that is what makes the core look more
// concentrated.

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

struct LightData {
    // xyz = center in VPU, w = 1 / range
    center     : vec4<f32>,
    // rgb = center color, a = intensity
    color      : vec4<f32>,
    // rgb = edge color, a = falloff exponent
    color2     : vec4<f32>,
    // x = how much the halo modulates rather than adds; see `fs_bulb`.
    blend      : vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame : Frame;
@group(1) @binding(0) var<uniform> light : LightData;

struct VsOut {
    @builtin(position) clip  : vec4<f32>,
    @location(0)       world : vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos : vec3<f32>) -> VsOut {
    var out : VsOut;
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    return out;
}

/// The bulb halo, which meets what is under it in two ways at once.
///
/// `PS_BulbLight`, `LightShader.hlsl:34`. The shape of the light is the same as
/// the classic one — the same attenuation, the same square-root walk from the
/// edge colour to the centre colour — and what differs is entirely how it is
/// blended.
///
/// The blend is reverse subtraction with the destination scaled by one minus
/// the source colour and the source scaled by its own alpha, which reads as
/// nonsense until it is written out. With `C` the light's contribution and `m`
/// its modulate-versus-add setting, the shader hands back a **negative** colour
/// `-m·C` and an alpha of `1/m - 1`, and the hardware computes
///
/// ```text
///   dst · (1 - src) - src · srcAlpha  =  dst · (1 + m·C)  +  (1 - m)·C
/// ```
///
/// At `m = 0` that is `dst + C`: the flat additive disc the classic light
/// draws. At `m = 1` it is `dst · (1 + C)`: the pixel underneath multiplied,
/// so a lit insert brightens the artwork it sits on rather than painting its
/// own colour over it. The original calls it "a very crude approximation of
/// real lighting" and it is most of what makes a bulb light look lit.
@fragment
fn fs_bulb(in : VsOut) -> @location(0) vec4<f32> {
    let len = length(light.center.xyz - in.world) * light.center.w;
    // `max` rather than `saturate`: the original keeps a sliver here because a
    // hard zero turns the blend off, and the blend is doing the work.
    let atten = pow(max(1.0 - len, 0.0001), light.color2.a);
    let lcolor = mix(light.color2.rgb, light.color.rgb, sqrt(len));
    let m = light.blend.x;
    return vec4<f32>(lcolor * (-m * atten * light.color.a), 1.0 / m - 1.0);
}

@fragment
fn fs_main(in : VsOut) -> @location(0) vec4<f32> {
    let len = length(light.center.xyz - in.world) * light.center.w;
    let atten = pow(1.0 - saturate(len), light.color2.a);
    let lcolor = mix(light.color2.rgb, light.color.rgb, sqrt(len));
    let contribution = atten * light.color.a;

    // Linear and uncapped, like everything else the table pass writes. A halo
    // is the brightest thing on a playfield and the first to overshoot, which
    // is exactly what the bloom pass is looking for; tone mapping it here would
    // flatten it before that pass ever saw it.
    return vec4<f32>(lcolor * contribution, saturate(contribution));
}
