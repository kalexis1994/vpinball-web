// Everything the table shader is, except the vertex stage.
//
// The original has an ubershader (`shaders/hlsl_glsl/BasicShader.hlsl` plus
// `Material.fxh`) with dozens of techniques selected by defines. Here the
// structure is the opposite: small, specialised pipelines, chosen when the
// table is loaded and not per frame.
//
// The **lighting does follow the original**, because it is what gives tables
// the look they have. It is `lightLoop` from `Material.fxh:186`, with its three
// parts: energy conservation, the two point scene lights (`DoPointLight`, with
// its ashikhmin/blinn BRDF and its range attenuation), and the contribution of
// the environment.
//
// # Why this file is not a whole shader
//
// There are two vertex stages, not one. The static geometry of a table arrives
// already in world space —its transform is baked once at load time— while the
// dozen pieces that move need a model matrix per draw. Everything downstream of
// the vertex stage is identical for both, and the lighting is by far the
// largest and the most delicate part of it.
//
// So this file holds the shared part and `table_vs.wgsl` / `dynamic_vs.wgsl`
// hold one vertex stage each; `TablePipeline` concatenates this file with one
// of them to build a shader module. The alternative — putting a model matrix on
// the static path too, set to the identity — would add a matrix multiply to
// every one of a table's eighty thousand baked vertices in order to serve about
// a dozen.
//
// The one liberty we take is the environment: the original does lookups into an
// equirectangular envmap with the mip picked by roughness (`DoEnvmapDiffuse` /
// `DoEnvmapGlossy`). Here there is no envmap yet, so it is replaced by a
// constant color. It is the smallest possible substitution and it leaves the
// rest of the formula untouched.

struct Frame {
    view_proj  : mat4x4<f32>,
    eye        : vec4<f32>,
    // rgb = ambient color, a = range of the lights
    ambient    : vec4<f32>,
    // xyz = position of scene light 0 / 1
    light0     : vec4<f32>,
    light1     : vec4<f32>,
    // rgb = emission of the lights, a = how much the environment contributes
    emission   : vec4<f32>,
    // x = mip levels of the environment map, y = height, z = exposure
    env        : vec4<f32>,
    // xy = one screen pixel in UV, which is how a fragment finds its own place
    // in the transmitted-light buffer. The original's `w_h_height`.
    screen     : vec4<f32>,
    // The plane below which a fragment is thrown away: xyz normal, w distance.
    // Zero except while the reflection probe is being drawn.
    clip       : vec4<f32>,
    // xyz = the normal of the surface that reflects, w = how strongly.
    mirror     : vec4<f32>,
};

// The same values `Shader::SetMaterial` hands to the original's ubershader
// (`renderer/Shader.cpp:790-855`), already resolved on the Rust side.
struct MaterialData {
    // rgb = base color, a = alpha already resolved by opacity_active
    base_color : vec4<f32>,
    // rgb = specular color (without the 0.08), a = specular exponent
    glossy     : vec4<f32>,
    // rgb = clearcoat layer already multiplied by 0.08, a = edge weight
    clearcoat  : vec4<f32>,
    // x = has texture, y = is metal, z = wrap lighting, w = edge
    flags      : vec4<f32>,
    // x = specular image lerp, y = thickness
    extra      : vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame : Frame;
// The environment map with its mips, the already convolved irradiance, and the
// sampler they share.
@group(0) @binding(1) var env_radiance   : texture_2d<f32>;
@group(0) @binding(2) var env_irradiance : texture_2d<f32>;
@group(0) @binding(3) var env_samp       : sampler;

@group(1) @binding(0) var<uniform> material : MaterialData;
@group(1) @binding(1) var tex : texture_2d<f32>;
@group(1) @binding(2) var samp : sampler;

struct VsIn {
    @location(0) pos    : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) uv     : vec2<f32>,
};

// Light coming from behind a translucent surface.
//
// The lights on the table are drawn on their own into this buffer before
// anything else, and it is blurred. What it holds at a given pixel is how much
// lamp light is arriving there — so a plastic insert cover, which is
// translucent, can add the glow of the bulb underneath it and of the bulbs
// beside it. `Renderer::DrawBulbLightBuffer` fills it; `BasicShader.hlsl:337`
// reads it.
@group(0) @binding(4) var transmitted : texture_2d<f32>;
@group(0) @binding(5) var transmitted_samp : sampler;

// The playfield's reflection of what stands on it.
//
// Drawn before the table, from a camera flipped through the playfield, with
// everything at or below the playfield clipped away. Because the camera is the
// mirror image of the real one, a point on the playfield sees its own
// reflection at its own place on the screen — so this is sampled in screen
// space with no maths at all, which is the whole trick.
// `RenderProbe::DoRenderReflectionProbe`, `RenderProbe.cpp:404`.
@group(0) @binding(6) var reflected : texture_2d<f32>;

struct VsOut {
    @builtin(position) clip   : vec4<f32>,
    @location(0)       world  : vec3<f32>,
    @location(1)       normal : vec3<f32>,
    @location(2)       uv     : vec2<f32>,
};


// `FresnelSchlick`, `Material.fxh:102`.
fn fresnel_schlick(spec : vec3<f32>, ldoth : f32, edge : f32) -> vec3<f32> {
    return spec + (vec3<f32>(edge) - spec) * pow(1.0 - ldoth, 5.0);
}

fn max_component(v : vec3<f32>) -> f32 {
    return max(v.x, max(v.y, v.z));
}

// `GeometricOpacity`, `Material.fxh:91`. Makes the edge of a translucent part
// more opaque: seen edge-on it looks more solid than seen head-on.
fn geometric_opacity(ndotv : f32, alpha : f32, blending : f32, thickness : f32) -> f32 {
    let x = abs(ndotv);
    let g = blending * (1.0 - (x / (x * (1.0 - thickness) + thickness)));
    return mix(alpha, 1.0, g);
}

const PI : f32 = 3.14159265;

// `ray_to_equirectangular_uv`, `Helpers.fxh:251`. The polar angle is measured
// from +Z, which in Visual Pinball is the axis pointing up.
fn env_uv(ray : vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        0.5 + atan2(ray.y, ray.x) / (2.0 * PI),
        acos(clamp(ray.z, -1.0, 1.0)) / PI
    );
}

// `DoEnvmapDiffuse`, `Material.fxh:150`. The irradiance map already comes
// divided by PI, so it is a lookup and nothing more.
fn env_diffuse(n : vec3<f32>, diffuse : vec3<f32>) -> vec3<f32> {
    let e = textureSampleLevel(env_irradiance, env_samp, env_uv(n), 0.0).rgb;
    return diffuse * e * frame.emission.a;
}

// `DoEnvmapGlossy`, `Material.fxh:158`. Picking the mip by roughness is, in the
// original's own words, a "very very crude approximation by abusing miplevels".
fn env_glossy(r : vec3<f32>, glossy : vec3<f32>, glossy_power : f32) -> vec3<f32> {
    // The original uses the **height** of the map, not the width, even though
    // the uniform is called `TexWidth`.
    let log_h = log2(frame.env.y);
    let mip = min(
        log_h + log2(sqrt(3.0)) - 0.5 * log2(glossy_power + 1.0),
        log_h - 1.0
    );
    let e = textureSampleLevel(env_radiance, env_samp, env_uv(r), clamp(mip, 0.0, frame.env.x - 1.0)).rgb;
    return glossy * e * frame.emission.a;
}

// `DoPointLight`, `Material.fxh:109`.
fn point_light(
    light_pos    : vec3<f32>,
    pos          : vec3<f32>,
    n            : vec3<f32>,
    v            : vec3<f32>,
    diffuse      : vec3<f32>,
    glossy       : vec3<f32>,
    edge         : f32,
    glossy_power : f32,
    wrap         : f32,
    is_metal     : bool,
) -> vec3<f32> {
    let light_dir = light_pos - pos;
    let l = normalize(light_dir);
    let ndl = dot(n, l);
    var out = vec3<f32>(0.0);

    // Lambertian diffuse with the optional wrap term, normalised so that
    // wrapping the light around does not add energy.
    if (!is_metal && (ndl + wrap) > 0.0) {
        out = diffuse * ((ndl + wrap) / ((1.0 + wrap) * (1.0 + wrap)));
    }

    if (ndl > 0.0) {
        let h = normalize(l + v);
        let ndh = dot(n, h);
        let ldh = dot(l, h);
        let vdh = dot(v, h);
        if (ndh > 0.0 && ldh > 0.0 && vdh > 0.0) {
            out = out + fresnel_schlick(glossy, ldh, edge)
                * (((glossy_power + 1.0) / (8.0 * vdh)) * pow(ndh, glossy_power));
        }
    }

    // Attenuation with a range, not the physical 1/d². It is what allows having
    // "ranged" lights instead of them lighting the whole table.
    let d2 = dot(light_dir, light_dir);
    let range = frame.ambient.a;
    let r4 = range * range * range * range;
    var aten = saturate(1.0 - (d2 * d2) / max(r4, 1.0));
    aten = aten * aten / (d2 + 1.0);

    var ambient = glossy;
    if (!is_metal) {
        ambient = ambient + diffuse;
    }

    return out * frame.emission.rgb * aten + ambient * frame.ambient.rgb;
}

@fragment
fn fs_main(in : VsOut) -> @location(0) vec4<f32> {
    // The clip plane, which WebGPU has no fixed-function equivalent of: the
    // original sets one on the device (`RenderProbe.cpp:418`) and here the
    // fragment has to throw itself away.
    if (frame.clip.w != 0.0 || any(frame.clip.xyz != vec3<f32>(0.0))) {
        if (dot(in.world, frame.clip.xyz) + frame.clip.w < 0.0) {
            discard;
        }
    }

    // The original's two routes: `ps_main` without texture
    // (`BasicShader.hlsl:320-323`) and `ps_main_texture` with one (`:371-374`).
    let has_texture = material.flags.x > 0.5;
    let is_metal = material.flags.y > 0.5;
    let lerp_img = material.extra.x;

    var texel = vec4<f32>(1.0);
    if (has_texture) {
        texel = textureSample(tex, samp, in.uv);
        // The original clamps the texel because it may come from an HDR
        // texture.
        texel = vec4<f32>(saturate(texel.rgb), texel.a);
    }

    // `pixel.a *= cBase_Alpha.a` (`BasicShader.hlsl:368`).
    var alpha = material.base_color.a * texel.a;
    let albedo = vec4<f32>(material.base_color.rgb * texel.rgb, alpha);

    var n = normalize(in.normal);
    let v = normalize(frame.eye.xyz - in.world);

    // Tables are full of meshes with their normals flipped. The original flips
    // them unconditionally since 10.8 and explains why in `Material.fxh:194`:
    // "quite a lot of tables feature wrong normals".
    if (dot(n, v) < 0.0) {
        n = -n;
    }

    let wrap = material.flags.z;
    let edge = material.flags.w;

    var diffuse = albedo.rgb;

    // The specular is **not** the material color as it stands: the original
    // multiplies it by 0.08 before feeding it to the BRDF
    // (`BasicShader.hlsl:322`). It is the typical F0 of a dielectric. Passing
    // it raw leaves a grey floor over the whole table that kills the playfield
    // texture.
    var glossy : vec3<f32>;
    if (is_metal) {
        // On metal the base color acts as the specular.
        glossy = diffuse;
    } else if (has_texture) {
        let tint = texel.rgb * lerp_img + vec3<f32>(1.0 - lerp_img);
        glossy = tint * material.glossy.rgb * 0.08;
    } else {
        glossy = material.glossy.rgb * 0.08;
    }

    // The clearcoat layer already comes multiplied by 0.08 from Rust.
    let specular = material.clearcoat.rgb;

    // Energy conservation (`Material.fxh:200-208`): if diffuse plus glossy go
    // past one, both get scaled. Without this a large matte surface like the
    // playfield receives full specular over its entire extent.
    let sum = max_component(diffuse) + max_component(glossy);
    if (sum > 1.0) {
        diffuse = diffuse / sum;
        glossy = glossy / sum;
    }

    // The exponent already comes mapped from 0..1 to 2..2048 (`Shader.cpp:799`).
    let glossy_power = material.glossy.a;

    var color = vec3<f32>(0.0);
    if ((!is_metal && max_component(diffuse) > 0.0) || max_component(glossy) > 0.0) {
        color = color + point_light(frame.light0.xyz, in.world, n, v, diffuse, glossy, edge, glossy_power, wrap, is_metal);
        color = color + point_light(frame.light1.xyz, in.world, n, v, diffuse, glossy, edge, glossy_power, wrap, is_metal);
    }

    // Environment (IBL). For many tables this is the term that rules: they
    // leave the two scene lights black and all the light comes from here.
    let ndv = min(max(dot(n, v), 0.0), 1.0);
    if (!is_metal && max_component(diffuse) > 0.0) {
        color = color + env_diffuse(n, diffuse);
    }
    if (max_component(glossy) > 0.0 || max_component(specular) > 0.0) {
        let refl = (2.0 * ndv) * n - v;
        if (max_component(glossy) > 0.0) {
            color = color + env_glossy(refl, glossy, glossy_power);
        }
        // `DoEnvmap2ndLayer`, `Material.fxh:168`: the clearcoat layer mixes the
        // result with the environment according to the Fresnel.
        if (max_component(specular) > 0.0) {
            let w = fresnel_schlick(specular, ndv, edge);
            let e = textureSampleLevel(env_radiance, env_samp, env_uv(refl), 0.0).rgb
                * frame.emission.a;
            color = mix(color, e, w);
        }
    }

    // Geometric opacity: the edge of a translucent part looks more solid, and
    // a translucent one lets through whatever the lamps behind it are doing.
    if (alpha < 1.0) {
        alpha = geometric_opacity(ndv, alpha, material.clearcoat.a, material.extra.y);
        // `BasicShader.hlsl:337`. The square root is the original's, marked
        // there as magic; the alpha is in it because a surface that is barely
        // there transmits barely anything.
        let uv = in.clip.xy * frame.screen.xy;
        let through = textureSampleLevel(transmitted, transmitted_samp, uv, 0.0).rgb;
        color = color + sqrt(diffuse) * through * alpha;
    }

    // What the playfield is mirroring, if this surface faces it.
    //
    // `compute_reflection`, `BasicShader.hlsl:199`. Two things about it are
    // worth keeping as they are. It is **added** rather than mixed in by the
    // Fresnel term, which the original notes is a simplification and which
    // stops a strong reflection from hiding the artwork at a grazing angle. And
    // the smoothstep is what selects: a surface square to the probe takes the
    // whole reflection, one tilted past sixty degrees takes none, so the
    // playfield and the flat tops of the things on it mirror and the walls do
    // not. The original calls those two numbers magic and says they came from
    // looking at it.
    if (frame.mirror.w > 0.0) {
        // Half a texel off, so the hardware's own filtering blurs it a little —
        // the original's trick, and cheaper than blurring.
        //
        // Sampled with an explicit level and outside any branch that depends on
        // this fragment. `textureSample` picks its mip from the derivatives of
        // neighbouring fragments, so it may only be called where every fragment
        // in a quad agrees on reaching it; `facing` is per-fragment and putting
        // the sample behind it is a uniformity violation. Native wgpu let it
        // through and the browser did not — a black canvas with the sound still
        // playing, which is what a shader that failed to compile looks like.
        let uv = in.clip.xy * frame.screen.xy + 0.5 * frame.screen.xy;
        let mirrored = textureSampleLevel(reflected, transmitted_samp, uv, 0.0).rgb;
        let facing = smoothstep(0.5, 0.9, dot(frame.mirror.xyz, n));
        color = color + facing * frame.mirror.w * mirrored;
    }

    // Linear, and deliberately with no ceiling on it. The tone mapping happens
    // once at the end, in `post.wgsl`, after the bloom pass has had a chance to
    // catch whatever overshoots — which is the order the original works in and
    // the reason its own tone mapper carries the note that overflow is handled
    // by bloom.
    return vec4<f32>(color, alpha);
}
