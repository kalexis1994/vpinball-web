// A light's halo, and the insert it lights.
//
// Port of `ClassicLightShader.hlsl` — the halo of `PS_LightWithoutTexel`
// (`:89-117`) and the lit artwork of `PS_LightWithTexel` (`:52-87`) — and of
// the bulb halo of `LightShader.hlsl:34`. The shape of the light —its outline—
// already arrives triangulated; what this shader does is paint a halo over it
// that fades out with the distance to the center, and add it to what is
// already drawn.
//
// The color is not one single color: there is one for the center and another
// for the edge, and it interpolates between the two by the **square root** of
// the distance, not by the distance — that is what makes the core look more
// concentrated.
//
// # This file is not a whole shader
//
// It is concatenated after `material.wgsl`, the way the table's vertex stages
// are, and takes from it the frame (`frame`, group 0), the material and its
// texture (`material`, `tex`, `samp`, group 1), the `VsOut` it fills in, and
// the lighting — `point_light`, `env_diffuse`, `env_glossy`,
// `fresnel_schlick`. The lit insert is the reason: the original lights the
// insert's artwork with the very `lightLoop` its playfield uses
// (`ClassicLightShader.hlsl:70`), and an insert lit by a second, slightly
// different loop would sit on the playfield in a slightly different light, at
// every intensity, for no reason a player could name. The light's own data is
// at group 2 so that it never collides with the material's.

struct LightData {
    // xyz = center in VPU, w = 1 / range
    center     : vec4<f32>,
    // rgb = center color, a = intensity
    color      : vec4<f32>,
    // rgb = edge color, a = falloff exponent
    color2     : vec4<f32>,
    // x = how much the halo modulates rather than adds; see `fs_bulb`.
    // y = transmission scale; see `fs_transmitted`.
    // z = image mode: the artwork as it is, not lit; see `fs_texel`.
    blend      : vec4<f32>,
};

@group(2) @binding(0) var<uniform> light : LightData;

// `vs_light_main`, `ClassicLightShader.hlsl:33-50`. The mesh is already in
// world space, and its normal is straight up — `light.cpp:546-548` writes
// `(0, 0, 1)` into every vertex of the lightmap, insert and halo alike — so
// there is nothing to carry per vertex but the place and the texture
// coordinate.
@vertex
fn vs_light(@location(0) pos : vec3<f32>, @location(1) uv : vec2<f32>) -> VsOut {
    var out : VsOut;
    out.clip = frame.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

/// The halo at a point: its colour, and how much of it lands there.
///
/// `rgb` is the walk from the edge colour to the centre colour, `a` the
/// attenuation times the lamp's intensity — the `atten*lightColor_intensity.w`
/// that both of the original's light shaders build before they part ways over
/// how to blend it.
fn halo(world : vec3<f32>) -> vec4<f32> {
    let len = length(light.center.xyz - world) * light.center.w;
    let atten = pow(1.0 - saturate(len), light.color2.a);
    let lcolor = mix(light.color2.rgb, light.color.rgb, sqrt(len));
    return vec4<f32>(lcolor, atten * light.color.a);
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

/// The same halo, into the transmitted-light buffer.
///
/// That buffer answers one question — how much lamp light is arriving at this
/// point of the playfield — and a lamp gets to answer it only as far as its own
/// `TransmissionScale` says. The original multiplies the intensity by it for
/// that pass and for no other (`light.cpp:801`); the pass itself refuses a lamp
/// whose scale is zero outright (`light.cpp:600`), which is why the draw loop
/// skips those rather than relying on a multiply by nothing.
///
/// Without the scale every insert on the table bleeds its full brightness onto
/// the ball and through every translucent plastic above it, which is a table
/// that glows from underneath everywhere at once.
@fragment
fn fs_transmitted(in : VsOut) -> @location(0) vec4<f32> {
    let lit = halo(in.world);
    let contribution = lit.a * light.blend.y;
    return vec4<f32>(lit.rgb * contribution, saturate(contribution));
}

/// The classic halo with no picture: `PS_LightWithoutTexel`,
/// `ClassicLightShader.hlsl:89-117`, less the lit surface colour it adds
/// underneath — that term is the surface's own material lit in the lamp's
/// colour, and on a flat additive disc it is what the playfield already shows.
@fragment
fn fs_classic(in : VsOut) -> @location(0) vec4<f32> {
    let lit = halo(in.world);
    let lcolor = lit.rgb;
    let contribution = lit.a;

    // Linear and uncapped, like everything else the table pass writes. A halo
    // is the brightest thing on a playfield and the first to overshoot, which
    // is exactly what the bloom pass is looking for; tone mapping it here would
    // flatten it before that pass ever saw it.
    return vec4<f32>(lcolor * contribution, saturate(contribution));
}

// `OverlayHDR`, `Helpers.fxh:499-519`. Photoshop's overlay — multiply below
// half, screen above — with the case picked per channel by `step(0.5, base)`
// and the result clamped at zero, because both sides may be past one here.
// The `float4` version, alpha included, is the one the light shader calls.
fn overlay_hdr(base : vec4<f32>, blend : vec4<f32>) -> vec4<f32> {
    let pick = step(vec4<f32>(0.5), base);
    return max(
        mix(base * blend * 2.0, 1.0 - 2.0 * (1.0 - base) * (1.0 - blend), pick),
        vec4<f32>(0.0)
    );
}

// `ScreenHDR`, `Helpers.fxh:467-470`.
fn screen_hdr(base : vec4<f32>, blend : vec4<f32>) -> vec4<f32> {
    return max(1.0 - (1.0 - base) * (1.0 - blend), vec4<f32>(0.0));
}

// `lightLoop`, `Material.fxh:186-247`, as the insert's texel goes through it.
//
// The same three parts the table shader runs — energy conservation, the two
// scene point lights, the environment — on the same helpers from
// `material.wgsl`. It is written out here rather than shared with `fs_main`
// there because that one has the loop inlined among its texture, alpha-test,
// transmission and reflection concerns, none of which an insert has; the
// original keeps `lightLoop` as one function and this is that function.
fn light_loop(
    pos       : vec3<f32>,
    normal    : vec3<f32>,
    v         : vec3<f32>,
    diffuse_in: vec3<f32>,
    glossy_in : vec3<f32>,
    specular  : vec3<f32>,
    edge      : f32,
    is_metal  : bool,
) -> vec3<f32> {
    var n = normal;
    var ndv = dot(n, v);
    // "quite a lot of tables feature wrong normals" (`Material.fxh:194`): a
    // backside is lit as a front. For an insert the normal is straight up and
    // the camera is above the table, so this only ever matters for the
    // reflection probe, which looks up from underneath.
    if (ndv < 0.0) {
        n = -n;
        ndv = -ndv;
    }
    ndv = min(ndv, 1.0);

    var diffuse = diffuse_in;
    var glossy = glossy_in;
    let diffuse_max = max_component(diffuse);
    let glossy_max = max_component(glossy);
    let specular_max = max_component(specular);
    // Energy conservation (`Material.fxh:200-208`).
    let sum = diffuse_max + glossy_max;
    if (sum > 1.0) {
        diffuse = diffuse / sum;
        glossy = glossy / sum;
    }

    let glossy_power = material.glossy.a;
    let wrap = material.flags.z;
    var color = vec3<f32>(0.0);
    if ((!is_metal && diffuse_max > 0.0) || glossy_max > 0.0) {
        color = color + point_light(frame.light0.xyz, pos, n, v, diffuse, glossy, edge, glossy_power, wrap, is_metal);
        color = color + point_light(frame.light1.xyz, pos, n, v, diffuse, glossy, edge, glossy_power, wrap, is_metal);
    }
    if (!is_metal && diffuse_max > 0.0) {
        color = color + env_diffuse(n, diffuse);
    }
    if (glossy_max > 0.0 || specular_max > 0.0) {
        let r = (2.0 * ndv) * n - v;
        if (glossy_max > 0.0) {
            color = color + env_glossy(r, glossy, glossy_power);
        }
        // `DoEnvmap2ndLayer`, `Material.fxh:167-172`. Its Fresnel takes the
        // *material's* edge, not the `edge` handed in — which for a metal is
        // one — and so does this.
        if (specular_max > 0.0) {
            let w = fresnel_schlick(specular, ndv, material.flags.w);
            let e = textureSampleLevel(env_radiance, env_samp, env_uv(r), 0.0).rgb
                * frame.emission.a;
            color = mix(color, e, w);
        }
    }
    return color;
}

/// The lit insert: `PS_LightWithTexel`, `ClassicLightShader.hlsl:52-87`.
///
/// Three steps, in the original's order. The texel of the insert's picture —
/// a picture of the whole playfield, sampled at this point's own place on it —
/// is **lit through the surface's material**: it is the base colour, the
/// glossy colour is the texel tinted by the material's, the clearcoat is the
/// material's, and all of it goes through the same light loop as the playfield
/// (`:64-71`). In image mode ("passthrough", `lightingOff`) the texel is taken
/// as it is (`:60-61`). Then the halo is **added** to that, alpha and all
/// (`:80-81`), and the sum is folded back into the texel with an overlay and a
/// screen (`:82-83`) — so the halo's colour brightens the artwork's own colours
/// rather than washing over them, and the words on the insert stay legible at
/// full brightness. Blended `ONE`/`ONE` onto what is already drawn
/// (`light.cpp:815-817`).
///
/// Below zero intensity there is no halo and the artwork is drawn dark, which
/// is what an insert that is switched off looks like: `light.cpp:713-718` only
/// leaves early when the picture *is* the surface's own.
@fragment
fn fs_texel(in : VsOut) -> @location(0) vec4<f32> {
    var pixel = textureSample(tex, samp, in.uv);

    var color : vec4<f32>;
    let lighting_off = light.blend.z > 0.5;
    if (lighting_off) {
        color = pixel;
    } else {
        // "could be HDR" (`:64`).
        pixel = vec4<f32>(saturate(pixel.rgb), pixel.a);
        let is_metal = material.flags.y > 0.5;
        let diffuse = pixel.rgb * material.base_color.rgb;
        // The texel straight into the glossy colour, with the material's 0.08
        // — and **not** the glossy-image lerp the table shader applies: the
        // classic light shader never had it (`:66`).
        var glossy = pixel.rgb * material.glossy.rgb * 0.08;
        var edge = material.flags.w;
        if (is_metal) {
            glossy = diffuse;
            edge = 1.0;
        }
        // The clearcoat already carries its 0.08 from the Rust side (`:67`).
        let specular = material.clearcoat.rgb;
        let v = normalize(frame.eye.xyz - in.world);
        color = vec4<f32>(
            light_loop(in.world, normalize(in.normal), v, diffuse, glossy, specular, edge, is_metal),
            pixel.a
        );
    }
    color.a = color.a * material.base_color.a;

    if (light.color.a != 0.0) {
        let lit = halo(in.world);
        color = color + vec4<f32>(lit.rgb * lit.a, saturate(lit.a));
        color = overlay_hdr(pixel, color);
        color = screen_hdr(pixel, color);
    }

    return color;
}
