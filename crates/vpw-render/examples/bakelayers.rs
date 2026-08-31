//! What a baked table's layers actually hold, measured off the atlas.
//!
//! ```text
//! cargo run --release -p vpw-render --example bakelayers -- table.vpx [surface]
//! ```
//!
//! For one question and it is the one that decides everything else: when the
//! lamps are on, does adding a surface's `LM_*` layers to its `BM_*` base give
//! back a lit table, or does it give back white? Reasoning about it has been
//! wrong three times; this reads the texels.
//!
//! What it prints per layer, over that layer's own island of the atlas:
//!
//! - **covered** — the share of the island whose alpha is not zero. A lightmap
//!   is mostly nothing; that is what makes it a lamp and not a copy of the
//!   table.
//! - **adds** — the mean of `rgb × a` over the *whole* island, which is
//!   exactly what the additive pass contributes there: the shader returns
//!   `staticColor × texel` and the blend weighs it by its own alpha
//!   (`BasicShader.hlsl:432`, `RenderDevice.cpp:2497`).
//!
//! The sum of `adds` across a surface's layers is what gets added to its base.
//! If that sum is near the base's own brightness, the composition is the one
//! the bake intends. If it is several times it, no amount of blending will
//! save it and the layers are not meant to be added to *this* base.

use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args.next().expect("usage: bakelayers table.vpx [surface]");
    // Which family to look at: the meshes whose name ends in this.
    let surface = args.next().unwrap_or_else(|| "_Playfield".to_string());

    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let scene = vpw_table::geometry::extract(&vpx);

    // The atlases, decoded once.
    let mut atlas: BTreeMap<String, image::RgbaImage> = BTreeMap::new();
    for i in &scene.images {
        if i.name.starts_with("VLM") {
            println!(
                "atlas {:<14} {}x{}  raw={:?} encoded={:?} alpha={}",
                i.name,
                i.width,
                i.height,
                i.rgba.as_ref().map(Vec::len),
                i.encoded.as_ref().map(|e| (e.len(), &e[..8.min(e.len())])),
                i.has_alpha
            );
        }
        let decoded = match (&i.rgba, &i.encoded) {
            (Some(raw), _) => image::RgbaImage::from_raw(i.width, i.height, raw.clone()),
            (None, Some(enc)) => {
                // The guard rails down, the same as the renderer does.
                let mut reader = image::ImageReader::new(std::io::Cursor::new(enc.as_slice()))
                    .with_guessed_format()
                    .expect("a readable texture");
                reader.limits(image::Limits::no_limits());
                match reader.decode() {
                    Ok(d) => Some(d.to_rgba8()),
                    Err(e) => {
                        println!("  {} would not decode: {e}", i.name);
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(d) = decoded {
            atlas.insert(i.name.clone(), d);
        }
    }

    // sRGB, because that is how the atlas is uploaded and so how the shader
    // reads it. Comparing bytes would compare the wrong numbers.
    let linear = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };

    let mut base_light = 0.0f32;
    let mut layer_total = 0.0f32;

    for m in &scene.meshes {
        let is_base = m.name.starts_with("BM_") && m.name.ends_with(&surface);
        let is_layer = m.name.starts_with("LM_") && m.name.ends_with(&surface);
        if !is_base && !is_layer {
            continue;
        }
        let Some(img) = atlas.get(&m.image) else {
            println!("{:<38} no atlas called {}", m.name, m.image);
            continue;
        };
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &m.vertices {
            for c in 0..2 {
                lo[c] = lo[c].min(v.uv[c]);
                hi[c] = hi[c].max(v.uv[c]);
            }
        }
        let px =
            |u: f32, size: u32| ((u * size as f32).round() as i64).clamp(0, size as i64 - 1) as u32;
        let (x0, x1) = (px(lo[0], img.width()), px(hi[0], img.width()));
        let (y0, y1) = (px(lo[1], img.height()), px(hi[1], img.height()));

        let (mut sum, mut covered, mut count) = (0.0f32, 0u64, 0u64);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = img.get_pixel(x, y).0;
                let a = f32::from(p[3]) / 255.0;
                let l = (linear(p[0]) + linear(p[1]) + linear(p[2])) / 3.0;
                sum += l * a;
                if p[3] > 0 {
                    covered += 1;
                }
                count += 1;
            }
        }
        let mean = sum / count.max(1) as f32;
        println!(
            "   atlas {} is {}x{}; uv {:.4}..{:.4} , {:.4}..{:.4} -> x {}..{} y {}..{}",
            m.image,
            img.width(),
            img.height(),
            lo[0],
            hi[0],
            lo[1],
            hi[1],
            x0,
            x1,
            y0,
            y1
        );
        println!(
            "{:<38} {:>5}x{:<5} covered {:>5.1}%   adds {:.4}",
            m.name,
            x1 - x0 + 1,
            y1 - y0 + 1,
            100.0 * covered as f32 / count.max(1) as f32,
            mean
        );
        if is_base {
            base_light += mean;
        } else {
            layer_total += mean;
        }
    }

    println!();
    println!("the base holds     {base_light:.4}");
    println!("its layers add     {layer_total:.4}");
    if base_light > 0.0 {
        println!(
            "which is           {:.1}x the base",
            layer_total / base_light
        );
    }
}
