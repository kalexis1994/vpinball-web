//! The environment map.
//!
//! For most tables this is **the** light source. Plenty of them leave the two
//! scene lights black and trust everything to the environment and to the
//! shadows already baked into the playfield texture — F-14 is one of those, and
//! without the environment it draws flat and grey.
//!
//! Which map is a question the table answers. `EIMG` names one of its own
//! images (`pintable.cpp:2415`), the original looks that up first and only
//! falls back to the `EnvMap.webp` it ships when the table has none
//! (`Renderer.cpp:208-210`). On a table lit by nothing else, every playfield
//! pixel is `texel × base × irradiance × envScale`, so drawing it under a map
//! it did not ask for scales the whole picture by the ratio of the two maps'
//! irradiance. F-14 asks for `shinyenvironment3blur4`.
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
use vpw_table::geometry::{Image, Scene};

/// The map that ships with Visual Pinball, packed into the binary.
pub const DEFAULT_ENVMAP: &[u8] = include_bytes!("../assets/EnvMap.webp");
/// What [`EnvMap::source`] says when the map is the shipped one.
pub const DEFAULT_SOURCE: &str = "EnvMap.webp";

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
    /// Where the map came from: the name of the table image, or
    /// [`DEFAULT_SOURCE`] for the shipped one. So a photograph can say which
    /// map it was taken under.
    pub source: String,
}

impl EnvMap {
    /// Loads an equirectangular map from image bytes.
    pub fn load(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        Self::from_rgba(device, queue, img, DEFAULT_SOURCE)
    }

    /// Loads the map that ships with Visual Pinball.
    pub fn default_map(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        Self::load(device, queue, DEFAULT_ENVMAP)
    }

    /// Loads the map from one of the table's own images.
    ///
    /// The two ways in are the ones the scene's textures take (`scene.rs:527`):
    /// a raw BMP is already RGBA, the rest go through the decoder. What the
    /// decoder knows is what the crate was built with — PNG, JPG, WebP, BMP —
    /// and an `.exr` or `.hdr` is not among them, so a table that carries a
    /// floating-point environment gets `None` here and the shipped map from
    /// [`Self::for_table`]. The original keeps such a map in FP16/FP32
    /// (`Renderer.cpp:546`); this port's radiance texture is 8-bit sRGB, so
    /// supporting one is a format change and not only a decoder.
    pub fn from_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> Option<Self> {
        let img = match (&image.rgba, &image.encoded) {
            (Some(rgba), _) => image::RgbaImage::from_raw(image.width, image.height, rgba.clone())?,
            (None, Some(bytes)) => image::load_from_memory(bytes).ok()?.to_rgba8(),
            (None, None) => return None,
        };
        Self::from_rgba(device, queue, img, &image.name)
    }

    /// The map a table asks for, or the shipped one when it asks for none.
    ///
    /// `Renderer.cpp:208-210`: `GetImage(m_envImage)` first, `EnvMap.webp`
    /// only when that comes back empty. A name the table does not carry, or an
    /// image that does not decode, counts as empty too: the original's
    /// `GetImage` answers null for the first and the second cannot happen there.
    /// Either way the fallback is logged, because a table drawn under the wrong
    /// map looks plausible and only measures wrong.
    pub fn for_table(device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) -> Self {
        if !scene.env_image.is_empty() {
            match scene.image(&scene.env_image) {
                Some(img) => match Self::from_image(device, queue, img) {
                    Some(map) => {
                        log::info!("environment map: table image {:?}", img.name);
                        return map;
                    }
                    None => log::warn!(
                        "environment map: table image {:?} does not decode, using {DEFAULT_SOURCE}",
                        img.name
                    ),
                },
                None => log::warn!(
                    "environment map: table names {:?} but does not carry it, using {DEFAULT_SOURCE}",
                    scene.env_image
                ),
            }
        } else {
            log::info!("environment map: table names none, using {DEFAULT_SOURCE}");
        }
        Self::default_map(device, queue).expect("the default environment map has to be loadable")
    }

    fn from_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: image::RgbaImage,
        source: &str,
    ) -> Option<Self> {
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
        // done once per table, so a blit pass on the GPU is not worth it.
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
            // Half floats and not bytes, as in the original (`Renderer.cpp:227`
            // allocates RGBA16F and `:773-785` writes it unclamped). With an
            // 8-bit source nothing ever goes above one, so what this buys is
            // the darks: a linear byte's first step is 1/255, and a dim room
            // sits below it.
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            irr.as_image_copy(),
            bytemuck::cast_slice(&irradiance),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(IRRADIANCE_W * 8),
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
            source: source.to_owned(),
        })
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

/// Convolves the map with the cosine of the angle to get the irradiance, as
/// RGBA half floats.
///
/// The original does this in `EnvmapPrecalc` and points out that the result
/// already comes divided by PI ("does /PI-corrected lookup/final color
/// already", `Material.fxh:149`), so it does here too.
///
/// It also gauss-blurs the source first (`Renderer.cpp:553-634`), but only a
/// floating-point one wider than 64 texels (`:556`), and says why: it samples
/// the hemisphere 4k times and a sun-sized spot would need 64k. This one does
/// not sample — it integrates every texel of a 64×32 shrink, which is its own
/// low-pass — and the 8-bit maps it can load were never blurred there either.
fn convolve(src: &image::RgbaImage) -> Vec<u16> {
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
            // PI-corrected. And it is not clamped: the original does not
            // either (`Renderer.cpp:773-785`), and the texture can hold it.
            let scale = if weight > 0.0 { 1.0 / weight } else { 0.0 };
            for c in accum {
                output.push(half::f16::from_f32(c * scale).to_bits());
            }
            output.push(half::f16::ONE.to_bits());
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(v: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(128, 64, image::Rgba([v, v, v, 255]))
    }

    fn texel(irr: &[u16], x: u32, y: u32) -> [f32; 3] {
        let i = ((y * IRRADIANCE_W + x) * 4) as usize;
        [
            half::f16::from_bits(irr[i]).to_f32(),
            half::f16::from_bits(irr[i + 1]).to_f32(),
            half::f16::from_bits(irr[i + 2]).to_f32(),
        ]
    }

    #[test]
    fn a_uniform_environment_lights_every_direction_the_same() {
        // The cosine-weighted average of a constant is the constant, already
        // PI-corrected: no factor sneaks in on the way to the texture.
        let irr = convolve(&uniform(255));
        for y in 0..IRRADIANCE_H {
            for x in 0..IRRADIANCE_W {
                for c in texel(&irr, x, y) {
                    assert!((c - 1.0).abs() < 1e-3, "({x},{y}) = {c}");
                }
            }
        }
    }

    #[test]
    fn a_dim_environment_keeps_its_darkness() {
        // sRGB 20 is linear 0.0069, below the first step of a linear byte. A
        // byte texture rounded that to 2/255 = 0.0078, thirteen per cent
        // brighter than the room; a half float keeps it.
        let irr = convolve(&uniform(20));
        let want = srgb_to_linear(20);
        let [r, _, _] = texel(&irr, 0, 0);
        assert!((r - want).abs() < want * 0.01, "{r} vs {want}");
    }
}
