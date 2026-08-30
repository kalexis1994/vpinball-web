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
    // rgb = the GI's first bounce off the glass and the plastics: flat, in
    // the lit bulbs' average colour. See `GpuFrame::gi_bounce`.
    gi_bounce  : vec4<f32>,
    // Each baked group's live level, scaling its lightmap layer. `env.z`
    // says how many layers are live.
    gi_levels  : vec4<f32>,
    // The playfield in world units: [min.x, min.y, 1/width, 1/height]. How a
    // fragment that is not the playfield finds its place in the lightmap.
    field      : vec4<f32>,
    // The general illumination: `env.w` pairs of rows, `[xyz, 1/range]` then
    // `[rgb at level and calibration, falloff_power]`. See `GpuFrame::gi`.
    gi         : array<vec4<f32>, 64>,
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

// The baked general illumination: the GI string traced against the table's
// own geometry, once, shadows and all, in playfield UV space. `gi_bounce.a`
// carries the group's live level, so the machine still owns the switch.
// See `crate::bake`.
@group(0) @binding(7) var gi_lightmap : texture_2d_array<f32>;

// The playfield's picture, for the ball's planar reflection.
@group(0) @binding(8) var field_tex : texture_2d<f32>;

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

// The general illumination, as light and not as paint.
//
// A departure from the original, which draws every bulb as a screen-space
// halo that mostly *modulates* the pixel under it — its own comment calls
// that "a very crude approximation of real lighting" (`light.cpp:826`) — so a
// playfield the table's own lighting leaves black stays black under thirty
// lit bulbs. A real machine's GI string pours real light onto the wood; in a
// dark arcade the machine glows. The brightest lit bulbs therefore also
// arrive here as point lights: the same centre, range and falloff their halos
// use, so what the author tuned keeps meaning the same thing.
fn gi_diffuse(pos : vec3<f32>, n : vec3<f32>, diffuse : vec3<f32>) -> vec3<f32> {
    // The bounce first: it reaches every corner the bulbs do not. But not the
    // head — the glass keeps the bounce inside the cabinet, and the head
    // stands behind it. The window is cabinet geometry, not the table's: the
    // field and its ramps live below 300 VPU and the head starts past it.
    let inside = 1.0 - smoothstep(300.0, 500.0, pos.z);
    var out = diffuse * frame.gi_bounce.rgb * inside;
    let count = u32(frame.env.w);
    for (var i = 0u; i < count; i = i + 1u) {
        let at    = frame.gi[2u * i];
        let color = frame.gi[2u * i + 1u];
        let to = at.xyz - pos;
        let len = length(to) * at.w;
        // A negative falloff power marks a baked lamp: it rides in the table
        // for the ball's glints only, its diffuse living in the lightmap.
        if (color.w < 0.0) {
            continue;
        }
        // The same ceiling as the bounce: the glass keeps a GI bulb's light
        // inside the cabinet, and the head stands behind it. Without this a
        // field-scale lamp paints the head's face its own colour.
        if (len < 1.0 && inside > 0.0) {
            // The halo's own attenuation, so the reach the author set is the
            // reach the light has. No cosine: a GI bulb sits a hand above the
            // wood under a plastic that scatters it everywhere, and the
            // cosine of the direct ray would darken exactly the sideways
            // reach that makes it *general* illumination. This is bounced
            // room light, not a spotlight.
            let atten = pow(1.0 - len, color.w);
            out = out + diffuse * color.rgb * (atten * inside);
        }
    }
    return out;
}

// The baked layers, sampled by world position and scaled by their groups'
// live levels — for everything that is not the playfield but stands in its
// light: above all the ball, which is steel, and steel shows the light
// around it. The same cabinet ceiling as the rest of the GI.
fn gi_baked(pos : vec3<f32>) -> vec3<f32> {
    var out = vec3<f32>(0.0);
    let layers = u32(frame.env.z);
    if (layers == 0u) {
        return out;
    }
    let uv = (pos.xy - frame.field.xy) * frame.field.zw;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return out;
    }
    let inside = 1.0 - smoothstep(300.0, 500.0, pos.z);
    for (var g = 0u; g < layers; g = g + 1u) {
        let baked = textureSampleLevel(gi_lightmap, env_samp, uv, g, 0.0).rgb;
        out = out + baked * frame.gi_levels[g];
    }
    return out * inside;
}

// `DoEnvmapGlossy`, `Material.fxh:158`. Picking the mip by roughness is, in the
// original's own words, a "very very crude approximation by abusing miplevels".
fn env_glossy(r : vec3<f32>, glossy : vec3<f32>, glossy_power : f32, mip_floor : f32) -> vec3<f32> {
    // The original uses the **height** of the map, not the width, even though
    // the uniform is called `TexWidth`.
    let log_h = log2(frame.env.y);
    let mip = min(
        log_h + log2(sqrt(3.0)) - 0.5 * log2(glossy_power + 1.0),
        log_h - 1.0
    );
    // The floor is the geometry's, not the material's: where the reflection
    // vector whips around within a pixel — a wire, a screw head, the lip of
    // a ramp — the map has to be read as blurred as that sweep, or a small
    // bright lamp lands once per mesh segment and a rail comes out as a
    // string of beads. A ball's reflection barely turns per pixel, so its
    // floor is zero and its lamps stay sharp.
    let e = textureSampleLevel(
        env_radiance,
        env_samp,
        env_uv(r),
        clamp(max(mip, mip_floor), 0.0, frame.env.x - 1.0)
    ).rgb;
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

    // An emissive surface: the texel is the light itself, not something the
    // room lights. The machine's display is the one such surface — a plasma
    // panel glows in a dark room instead of vanishing into it. Doubled so a
    // lit segment sits above the tone mapper's paper white and feeds a little
    // bloom, which is what a display does to a camera in the dark. (2.0 in
    // this slot is the playfield's baked-GI flag, not emission.)
    if (material.extra.w > 0.5 && material.extra.w < 1.5) {
        return vec4<f32>(texel.rgb * 2.0, texel.a);
    }

    // An additive layer: a copy of some geometry that carries a lamp's light
    // and nothing else, to be added to what is already drawn. Unlit, because
    // the lighting is what is painted into it; and multiplied by the colour,
    // whose alpha is how bright that lamp is *right now* against how bright it
    // is at full power. The original's `SHADER_TECHNIQUE_unshaded_with_texture`
    // with `staticColor_Alpha` (`primitive.cpp:1166-1173`).
    if (material.extra.w > 3.5 && material.extra.w < 4.5) {
        // `staticColor_Alpha * tex2D(...)`, componentwise and including the
        // alpha (`BasicShader.hlsl:432`). The alpha is not decoration here:
        // the pass blends with `SRC_ALPHA, ONE`, so the texture's own alpha is
        // the layer's **coverage** — where the atlas holds nothing for this
        // lamp, nothing is added. Dropping it adds the whole atlas everywhere
        // and the table comes out white.
        return material.base_color * texel;
    }

    // The head's artwork: also its own light — a sheet with tubes behind it —
    // but a printed one rather than a panel of plasma, so it is barely lifted
    // instead of doubled. The lighting that belongs on it is already painted
    // into the texture, which is the whole of what the bake is for.
    if (material.extra.w > 2.5 && material.extra.w < 3.5) {
        return vec4<f32>(texel.rgb * 1.25, texel.a);
    }

    // The alpha test. `BasicShader.hlsl:366`:
    //
    //     clip(pixel.a <= alphaTestValue ? -1 : 1);
    //
    // This is what makes a piece of artwork cut out of its background come out
    // as the artwork. A sword on a transparent background is an ordinary opaque
    // plastic ramp as far as its material is concerned — the material's own
    // opacity is off — so nothing about the material says the background should
    // not be drawn. What says so is the value the table's author set on the
    // *image*, and without honouring it the cut-out area is painted in whatever
    // colour it happens to carry and the piece comes out as a flat rectangle,
    // indistinguishable from a texture that failed to load.
    //
    // A negative threshold is the original's way of saying the table never
    // asked for one, and nothing is thrown away.
    let alpha_test = material.extra.z;
    if (alpha_test >= 0.0 && texel.a <= alpha_test) {
        discard;
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

    // How far the reflection vector sweeps within this pixel, in radians —
    // the curvature of the surface as the screen sees it. Derivatives must
    // sit in uniform control flow (a lesson this file already paid for once
    // with `textureSample`), so it is taken here at the top and handed down.
    // From it, the env mip that matches the sweep: a sweep of θ radians
    // crosses θ/π of the map's height, and reading anything sharper than
    // that turns each bright lamp into one bead per mesh segment.
    let refl_sweep = length(fwidth((2.0 * dot(n, v)) * n - v));
    let sweep_mip = log2(max(refl_sweep, 1e-6) * frame.env.y / 3.14159265);

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
        color = color + gi_diffuse(in.world, n, diffuse);
        // The baked half of the GI: each group traced with shadows and one
        // bounce, its layer scaled by its live level. Only the playfield
        // carries the flag — its UV spans the field, the lightmap's space.
        if (material.extra.w > 1.5 && material.extra.w < 2.5) {
            let layers = u32(frame.env.z);
            for (var g = 0u; g < layers; g = g + 1u) {
                let baked = textureSampleLevel(gi_lightmap, env_samp, in.uv, g, 0.0).rgb;
                color = color + diffuse * baked * frame.gi_levels[g];
            }
        }
    }
    // A metal has no diffuse, and the loop above leaves it out of the GI
    // entirely. A flat ambient here made the ball look like wax — a metal
    // answers direction, not averages — so the metals split: the ball gets
    // the original's dedicated treatment below, and every other metal part
    // keeps a modest ambient so rails and posts do not go black.
    // Every metal: `BallShader.hlsl`'s idea for the ball, generalised. What
    // any polished part on a pinball reflects is overwhelmingly the playfield
    // beside it, so the reflected eye ray either dives below the horizon —
    // intersect the playfield plane, sample its picture there, and light
    // that texel with the same baked GI the floor itself wears — or it
    // escapes upward into the environment map, which the metal path already
    // has. On top, the frame's GI bulbs as specular glints: the sharp
    // highlights are most of what makes chrome read as chrome. A small
    // ambient keeps the faces no reflection reaches from going black.
    //
    // Not in the reflection probe's pass (the one with a clip plane): the
    // planar trick makes no sense from under the floor, and a mirrored metal
    // computed with it comes out black — which the field then wears as a
    // dark cutout around the real part.
    let mirror_pass = frame.clip.z != 0.0;
    if (is_metal && !mirror_pass) {
        let v_dir = normalize(frame.eye.xyz - in.world);
        let r = (2.0 * dot(n, v_dir)) * n - v_dir;
        // Near the silhouette a curved metal compresses everything behind it
        // into the last sliver of outline, so almost none of it is actually
        // seen. Neither the planar sample nor a glint lobe can compress —
        // they paint the field-just-behind, or the lamp-just-behind, across
        // a wide band at full size, and that band continues the background
        // straight through the outline. That continuation *is* the glass-ball
        // illusion; where the geometry says "compressed to nothing", both
        // fade to nothing instead.
        let rim = smoothstep(0.08, 0.35, ndv);
        if (r.z < -0.02 && in.world.z > 0.0) {
            let t = in.world.z / -r.z;
            let hit = in.world + r * t;
            let uv = (hit.xy - frame.field.xy) * frame.field.zw;
            if (uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0) {
                // Steep rays only. At the sphere's rim the reflected ray
                // grazes on almost straight, lands just behind the ball, and
                // a sharp sample there continues the background through the
                // silhouette — which reads as a glass ball, not a steel one.
                // On real steel that grazing band is compressed to nothing
                // and scattered by wear; squaring the steepness is that, and
                // it leaves the belly — the field genuinely mirrored under
                // the ball — at full strength.
                let steep = clamp(-r.z, 0.0, 1.0);
                // And by the ray's travel: a reflection that flew far lands
                // far behind the part, and drawing the distant field there is
                // what reads as seeing *through* it — worst at the front
                // view, where the centre of the ball mirrors the field a
                // metre back. The original's ball keeps its reflection local
                // to the ball for the same reason; a quarter-metre half-life
                // is that locality.
                let fade = steep * steep * rim / (1.0 + t / 250.0);
                // A cross of taps rather than one: the reflection of a
                // rolling ball is soft, and one texel of a far hit is glass
                // again. The spread grows with the distance the ray flew.
                let spread = min(t * 0.002, 0.01);
                var texel = vec3<f32>(0.0);
                texel = texel + textureSampleLevel(field_tex, samp, uv + vec2<f32>(spread, 0.0), 0.0).rgb;
                texel = texel + textureSampleLevel(field_tex, samp, uv - vec2<f32>(spread, 0.0), 0.0).rgb;
                texel = texel + textureSampleLevel(field_tex, samp, uv + vec2<f32>(0.0, spread), 0.0).rgb;
                texel = texel + textureSampleLevel(field_tex, samp, uv - vec2<f32>(0.0, spread), 0.0).rgb;
                texel = texel * 0.25;
                // The picture times its light is only the field's base coat:
                // the field a player actually sees also wears its halos and
                // its lit inserts, which live in no texture this can sample.
                // The constant stands in for them, tuned until the ball's
                // reflection sits at the brightness of the wood beside it.
                // The floor just under a ball is in the ball's own shadow,
                // and its reflection is the dark core every chrome ball on a
                // surface shows at its bottom. The maps know nothing of the
                // ball, so the shadow is put back by the one thing that
                // measures "just under": the reflected ray's flight. A short
                // hop lands in the occluded ring; a long one reaches field
                // the ball never darkened. Without this the belly continues
                // the bright floor and the ball reads as glass.
                let contact = smoothstep(30.0, 180.0, t);
                let lit = texel * (gi_baked(hit) + frame.gi_bounce.rgb
                    + vec3<f32>(0.05) * frame.emission.a) * 2.5;
                // Through the wear: a scuff scatters what the mirror
                // would have returned, and the scuffs ride the mesh's UVs —
                // the physics' quaternion turns them, which is what makes a
                // rolling ball visibly roll.
                color = color + material.base_color.rgb * texel.rgb * lit * fade * contact;
            }
        }
        // The wear scatters what the mirror gave up. A scuff is not black
        // paint: the light it refuses to mirror it throws everywhere instead,
        // which is why scratches on a ball under a lit playfield read as
        // faint bright marks — and why the roll is visible at all on a dark
        // table. Fed by the same baked field light the reflections use, so a
        // ball in a dark corner stays dark, wear and all.
        // Mostly desaturated, and that is the point. Fed the field's light
        // raw, a scuff over a green insert glows insert-green — the exact
        // colour of the backdrop beside it — and the marks read as slits cut
        // through the ball. Worn steel scatters towards white in the eye
        // long before it does in the physics, and white marks read as wear.
        let worn = vec3<f32>(1.0) - texel.rgb;
        let room = gi_baked(in.world) + frame.gi_bounce.rgb + vec3<f32>(0.05) * frame.emission.a;
        let glow = mix(room, vec3<f32>(dot(room, vec3<f32>(0.334))), 0.7);
        color = color + worn * glow * 0.12;
        // The lamps, mirrored: a highlight where the reflected ray runs near
        // a bulb. The exponent is the ball's polish; the scale keeps a lamp's
        // pinpoint at the brightness its halo would show.
        let count = u32(frame.env.w);
        for (var i = 0u; i < count; i = i + 1u) {
            let at = frame.gi[2u * i];
            let lamp_color = frame.gi[2u * i + 1u];
            let to = at.xyz - in.world;
            let d = length(to);
            if (d * at.w < 1.5) {
                let glint = pow(max(dot(r, to / d), 0.0), 150.0);
                // A pow-150 lobe is about 0.12 radians wide. Where the
                // reflection sweeps further than that inside one pixel, the
                // pinpoint cannot be resolved — only sampled, one bead per
                // segment — so it fades by the ratio instead.
                let steady = 0.12 / (0.12 + refl_sweep);
                color = color + abs(lamp_color.rgb) * texel.rgb * glint * 8.0 * rim * steady;
            }
        }
    }
    if (max_component(glossy) > 0.0 || max_component(specular) > 0.0) {
        let refl = (2.0 * ndv) * n - v;
        if (max_component(glossy) > 0.0) {
            color = color + env_glossy(refl, glossy, glossy_power, sweep_mip);
        }
        // `DoEnvmap2ndLayer`, `Material.fxh:168`: the clearcoat layer mixes the
        // result with the environment according to the Fresnel.
        if (max_component(specular) > 0.0) {
            let w = fresnel_schlick(specular, ndv, edge);
            let e = textureSampleLevel(
                env_radiance,
                env_samp,
                env_uv(refl),
                clamp(sweep_mip, 0.0, frame.env.x - 1.0)
            ).rgb * frame.emission.a;
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
    // Not on metal. The probe is the floor's own reflection, sampled in
    // screen space — on the floor that is exactly right, and on the crown of
    // a ball (whose normal also faces up) it paints the mirrored field over
    // the steel at the ball's own pixels: the background, worn as a hat,
    // reading as a hole straight through the ball. A metal's reflections are
    // its own — environment, mirrored field, glints — all directional.
    if (frame.mirror.w > 0.0 && !is_metal) {
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
