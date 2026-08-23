//! The environment map.
//!
//! For most tables this is **the** light source. Plenty of them leave the two
//! scene lights black and trust everything to the environment and to the
//! shadows already baked into the playfield texture — F-14 is one of those, and
//! without the environment it draws flat and grey.
//!
//! The original does two different lookups against the same map
//! (`Material.fxh:150-165`):
//!
//! - **Diffuse**, against a version convolved with the cosine of the angle
//!   —the irradiance—, which it precomputes in `EnvmapPrecalc`
//!   (`Renderer.cpp:222`).
//! - **Specular**, against the original map, picking the mip level from the
//!   roughness. It is, in its own words, a "very very crude approximation by
//!   abusing miplevels".
//!
//! We do the same here: the mip chain is built on the CPU at load time and the
//! irradiance is convolved over a small version of the map, which is plenty
//! because the result is smooth by definition.

use std::f32::consts::PI;

/// The map that ships with Visual Pinball, packed into the binary.
pub const DEFAULT_ENVMAP: &[u8] = include_bytes!("../assets/EnvMap.webp");

/// Resolution the irradiance is convolved at. It is a smooth map: more
/// resolution adds nothing and the cost grows with the product of the two.
const IRRADIANCE_W: u32 = 32;
const IRRADIANCE_H: u32 = 16;
/// Resolution the map is shrunk to before convolving.
const SOURCE_W: u32 = 64;
const SOURCE_H: u32 = 32;

pub struct EnvMap {
    /// The map with its mip chain, for the specular term.
    pub radiance: wgpu::TextureView,
    /// The already convolved irradiance, for the diffuse term.
    pub irradiance: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// How many mip levels `radiance` has.
    pub mip_levels: u32,
    /// Height of the map. The original picks the mip for the specular term from
    /// the **height**, even though the uniform is called `TexWidth`
    /// (`Renderer.cpp:1037` passes `m_envSampler->GetHeight()`). With the width
    /// the lookup ends up one level blurrier than it should be.
    pub height: u32,
}

impl EnvMap {
    /// Loads an equirectangular map from image bytes.
    pub fn load(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return None;
        }

        let levels = 32 - w.max(h).leading_zeros();
        let radiance = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-envmap"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // The mip chain is built on the CPU. It is a small texture and it is
        // done exactly once, so a blit pass on the GPU is not worth it.
        let mut level = img.clone();
        for i in 0..levels {
            if i > 0 {
                let (nw, nh) = ((level.width() / 2).max(1), (level.height() / 2).max(1));
                level =
                    image::imageops::resize(&level, nw, nh, image::imageops::FilterType::Triangle);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &radiance,
                    mip_level: i,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &level,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level.width() * 4),
                    rows_per_image: Some(level.height()),
                },
                wgpu::Extent3d {
                    width: level.width(),
                    height: level.height(),
                    depth_or_array_layers: 1,
                },
            );
        }

        let irradiance = convolve(&img);
        let irr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-irradiance"),
            size: wgpu::Extent3d {
                width: IRRADIANCE_W,
                height: IRRADIANCE_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Linear, not sRGB: the irradiance was already computed in linear.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            irr.as_image_copy(),
            &irradiance,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(IRRADIANCE_W * 4),
                rows_per_image: Some(IRRADIANCE_H),
            },
            wgpu::Extent3d {
                width: IRRADIANCE_W,
                height: IRRADIANCE_H,
                depth_or_array_layers: 1,
            },
        );

        Some(Self {
            radiance: radiance.create_view(&wgpu::TextureViewDescriptor::default()),
            irradiance: irr.create_view(&wgpu::TextureViewDescriptor::default()),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("vpw-env-sampler"),
                // Horizontally it wraps all the way around; vertically it cuts
                // off at the poles and there we have to clamp.
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            mip_levels: levels,
            height: h,
        })
    }

    /// Loads the map that ships with Visual Pinball.
    pub fn default_map(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        Self::load(device, queue, DEFAULT_ENVMAP)
    }
}

/// Direction of texel `(x, y)` of an equirectangular map.
///
/// It is the inverse of `ray_to_equirectangular_uv` (`Helpers.fxh:251`): the
/// horizontal coordinate is the azimuth and the vertical one the polar angle
/// measured from `+Z`, which in Visual Pinball is the axis pointing up.
fn direction(x: u32, y: u32, w: u32, h: u32) -> [f32; 3] {
    let u = (x as f32 + 0.5) / w as f32;
    let v = (y as f32 + 0.5) / h as f32;
    let phi = (u - 0.5) * 2.0 * PI;
    let theta = v * PI;
    let (st, ct) = theta.sin_cos();
    [st * phi.cos(), st * phi.sin(), ct]
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Convolves the map with the cosine of the angle to get the irradiance.
///
/// The original does this in `EnvmapPrecalc` and points out that the result
/// already comes divided by PI ("does /PI-corrected lookup/final color
/// already", `Material.fxh:149`), so it does here too.
fn convolve(src: &image::RgbaImage) -> Vec<u8> {
    let small = image::imageops::resize(
        src,
        SOURCE_W,
        SOURCE_H,
        image::imageops::FilterType::Triangle,
    );

    // Direction and weight of every source texel. The weight is the solid angle
    // it covers, which in equirectangular goes with the sine of the polar
    // angle.
    let mut samples = Vec::with_capacity((SOURCE_W * SOURCE_H) as usize);
    for y in 0..SOURCE_H {
        let theta = (y as f32 + 0.5) / SOURCE_H as f32 * PI;
        let area = theta.sin();
        for x in 0..SOURCE_W {
            let p = small.get_pixel(x, y);
            samples.push((
                direction(x, y, SOURCE_W, SOURCE_H),
                [
                    srgb_to_linear(p[0]),
                    srgb_to_linear(p[1]),
                    srgb_to_linear(p[2]),
                ],
                area,
            ));
        }
    }
    let mut output = Vec::with_capacity((IRRADIANCE_W * IRRADIANCE_H * 4) as usize);
    for y in 0..IRRADIANCE_H {
        for x in 0..IRRADIANCE_W {
            let n = direction(x, y, IRRADIANCE_W, IRRADIANCE_H);
            let mut accum = [0.0f32; 3];
            let mut weight = 0.0f32;
            for (d, color, area) in &samples {
                let ndl = n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
                if ndl > 0.0 {
                    let w = ndl * area;
                    weight += w;
                    for (acc, c) in accum.iter_mut().zip(color) {
                        *acc += c * w;
                    }
                }
            }
            // Normalising by the weight gives the cosine-weighted average of
            // the radiance, which is exactly what the original integrates
            // (`FBShader.hlsl:886-905`, Monte Carlo over a hemisphere with
            // cosine sampling). It carries no extra factor: it already comes
            // PI-corrected.
            let scale = if weight > 0.0 { 1.0 / weight } else { 0.0 };
            for c in accum {
                output.push(((c * scale).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            output.push(255);
        }
    }
    output
}
