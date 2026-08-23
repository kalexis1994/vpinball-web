//! Loads a `.vpx` and draws a photo of the table to a PNG.
//!
//! ```text
//! cargo run --release -p vpw-render --example shot -- table.vpx output.png [width] [height]
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: shot <table.vpx> [output.png] [width] [height]");
    let output = args.next().unwrap_or_else(|| "table.png".into());
    let width: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(720);
    let height: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1280);

    let t0 = std::time::Instant::now();
    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let read_time = t0.elapsed();

    let t1 = std::time::Instant::now();
    let mut scene = vpw_table::geometry::extract(&vpx);
    // VPW_LIT=1 turns every lamp on. A table's lamps belong to the game, and
    // without a ROM running almost none of them are lit — which is right, and
    // useless for looking at what the lighting does.
    // VPW_REFLECT=<strength> overrides how strongly the playfield mirrors, so
    // the pass can be photographed with and without.
    if let Ok(v) = std::env::var("VPW_REFLECT")
        && let Ok(strength) = v.parse()
    {
        scene.lighting.reflection_strength = strength;
    }
    if std::env::var("VPW_LIT").is_ok() {
        for light in &mut scene.lights {
            light.state = 1.0;
        }
    }
    let extract_time = t1.elapsed();

    let mut gpu = pollster::block_on(vpw_render::offscreen::Offscreen::new(width, height))
        .expect("could not initialise wgpu");

    // VPW_BLOOM=0 turns the bloom pass off, which is how you find out what it
    // is contributing: two photos of the same table, one with and one without.
    if let Ok(v) = std::env::var("VPW_BLOOM") {
        gpu.set_bloom(v.parse().unwrap_or(1.8));
    }

    let t2 = std::time::Instant::now();
    let gpu_scene = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    let upload_time = t2.elapsed();

    // Something on the machine's head, so a photograph shows the display the
    // way a player would see it. VPW_SAY=text picks what it says.
    {
        use vpw_render::segments::{Glyph, Row, Style};
        let text = std::env::var("VPW_SAY").unwrap_or_else(|_| "  F-14 TOMCAT ".into());
        let score = std::env::var("VPW_SCORE").unwrap_or_else(|_| "40000003800000".into());
        let (rows, columns) = vpw_table::backbox::DISPLAY_GRID;
        let _ = rows;
        let top: Vec<u16> = text.chars().map(seg16).collect();
        let bottom: Vec<u16> = score.chars().map(seg7).collect();
        let raster = vpw_render::segments::draw(
            &[
                Row {
                    segments: &top,
                    glyph: Glyph::Alphanumeric,
                },
                Row {
                    segments: &bottom,
                    glyph: Glyph::Numeric,
                },
            ],
            vpw_table::backbox::DISPLAY_PIXELS,
            Style {
                columns,
                ..Default::default()
            },
        );
        gpu.set_display(&gpu_scene, &raster);
    }

    let aspect = width as f32 / height as f32;
    // VPW_VIEW=front|overhead takes the shot from one of the named places a
    // player looks from; without it, the framing that fits whatever is there.
    let mut camera = match std::env::var("VPW_VIEW").ok().as_deref() {
        Some(name) => {
            let view = match name {
                "overhead" | "cenital" => vpw_render::camera::View::Overhead,
                _ => vpw_render::camera::View::Front,
            };
            let head = vpw_table::backbox::Backbox::for_playfield(scene.playfield).bounds();
            vpw_render::Camera::for_view(
                view,
                (scene.playfield.min, scene.playfield.max),
                (head.min, head.max),
                aspect,
            )
        }
        None => {
            let (min, max) = gpu_scene.bounds;
            let mut c = vpw_render::Camera::framing(min, max, aspect);
            // Look a little above the center: the lower part of the table is
            // the one that matters while playing.
            c.target.y += (max.y - min.y) * 0.05;
            c
        }
    };
    let _ = &mut camera;

    // VPW_ONLY=name draws only the batches whose material or image contains
    // that text. Useful for working out who covers whom.
    let only = std::env::var("VPW_ONLY").ok().map(|s| s.to_lowercase());
    let except = std::env::var("VPW_EXCEPT").ok().map(|s| s.to_lowercase());
    let t3 = std::time::Instant::now();
    let pixels = gpu.render_filtered(&gpu_scene, &camera, |b| {
        let matches = |f: &String| {
            b.material.to_lowercase().contains(f.as_str())
                || b.image.to_lowercase().contains(f.as_str())
        };
        if let Some(f) = &except
            && matches(f)
        {
            return false;
        }
        only.as_ref().is_none_or(matches)
    });
    let draw_time = t3.elapsed();

    let s = gpu_scene.stats;
    println!("adapter        {}", gpu.adapter);
    println!(
        "table          {}",
        vpx.info.table_name.clone().unwrap_or_default()
    );
    println!();
    println!("meshes         {}", s.meshes);
    println!("vertices       {}", s.vertices);
    println!("triangles      {}", s.triangles);
    println!("textures       {}", s.textures);
    println!(
        "lights         {} lit of {}",
        scene.lights.len(),
        gpu.lights.len()
    );
    println!(
        "draw calls     {} (one per mesh would be {}, {:.1}x fewer)",
        s.draw_calls,
        s.draw_calls_naive,
        s.draw_calls_naive as f32 / s.draw_calls.max(1) as f32
    );
    println!();
    println!("read .vpx      {:?}", read_time);
    println!("extract        {:?}", extract_time);
    println!("upload to GPU  {:?}", upload_time);
    println!("draw           {:?}", draw_time);

    if std::env::var("VPW_BATCHES").is_ok() {
        println!();
        println!(
            "{:<26} {:<26} {:>8} {:>7}  tex",
            "material", "image", "triangles", "meshes"
        );
        for b in &gpu_scene.batches {
            println!(
                "{:<26} {:<26} {:>8} {:>7}  {}",
                if b.material.is_empty() {
                    "-"
                } else {
                    &b.material
                },
                if b.image.is_empty() { "-" } else { &b.image },
                b.index_count / 3,
                b.merged,
                if b.textured { "yes" } else { "NO" }
            );
        }
    }

    let not_background = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 12 || p[1] > 12 || p[2] > 16)
        .count();
    println!();
    println!(
        "pixels with table  {} of {} ({:.1}%)",
        not_background,
        width * height,
        100.0 * not_background as f32 / (width * height) as f32
    );

    image::save_buffer(&output, &pixels, width, height, image::ColorType::Rgba8)
        .expect("could not save the PNG");
    println!("saved to       {output}");
}

/// PinMAME's characters-to-segments table (`core.c:187`), for the sample text.
fn seg16(c: char) -> u16 {
    match c {
        '0' => 0x443F,
        '1' => 0x2200,
        '2' => 0x085B,
        '3' => 0x084F,
        '4' => 0x0866,
        '5' => 0x086D,
        '6' => 0x087D,
        '7' => 0x0007,
        '8' => 0x087F,
        '9' => 0x086F,
        'A' => 0x0877,
        'C' => 0x0039,
        'E' => 0x0079,
        'F' => 0x0071,
        'H' => 0x0876,
        'I' => 0x2209,
        'L' => 0x0038,
        'M' => 0x0536,
        'N' => 0x1136,
        'O' => 0x003F,
        'P' => 0x0873,
        'R' => 0x1873,
        'S' => 0x086D,
        'T' => 0x2201,
        'U' => 0x003E,
        '-' => 0x0840,
        _ => 0x0000,
    }
}

/// The seven-segment digits. `1` is the exception: a fourteen-segment `1` is
/// the centre stem, which a seven-segment display does not have.
fn seg7(c: char) -> u16 {
    match c {
        '1' => 0x06,
        _ => seg16(c) & 0x7F,
    }
}
