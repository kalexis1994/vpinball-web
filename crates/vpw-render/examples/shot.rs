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
    if std::env::var("VPW_RAW_LIGHTS").is_ok() {
        use vpin::vpx::gameitem::GameItemEnum;
        let mut vals: Vec<(String, f32, f32, f32, bool)> = Vec::new();
        for item in &vpx.gameitems {
            if let GameItemEnum::Light(l) = item {
                vals.push((
                    l.name.clone(),
                    l.intensity,
                    l.falloff_radius,
                    l.bulb_modulate_vs_add,
                    l.is_bulb_light,
                ));
            }
        }
        vals.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        println!("{} lights in the file", vals.len());
        for (n, i, f, m, b) in vals.iter().take(3).chain(vals.iter().rev().take(3)) {
            println!("  {n:<16} intensity {i:>8.2}  falloff {f:>6.1}  modulate {m:.4}  bulb {b}");
        }
        let bulbs = vals.iter().filter(|v| v.4).count();
        println!("  bulb lights {bulbs} of {}", vals.len());
    }
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
    // VPW_LIGHTS=<file> takes the lamp states from a game that actually ran,
    // one `name<TAB>level` per line, which is what `vpw-game`'s `table`
    // example writes. Photographing what the ROM is doing is the only way to
    // see what a player sees; a table with every lamp forced on and a table
    // with none look equally unlike it.
    if let Ok(path) = std::env::var("VPW_LIGHTS") {
        let text = std::fs::read_to_string(&path).expect("could not read the light dump");
        let mut levels = std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((name, level)) = line.split_once('\t') {
                levels.insert(
                    name.to_ascii_lowercase(),
                    level.parse::<f32>().unwrap_or(0.0),
                );
            }
        }
        let (mut found, mut lit) = (0, 0);
        for light in &mut scene.lights {
            if let Some(&level) = levels.get(&light.name.to_ascii_lowercase()) {
                found += 1;
                light.state = level;
                if level > 0.0 {
                    lit += 1;
                }
            }
        }
        println!(
            "lights: {found} of {} named in the dump, {lit} lit",
            scene.lights.len()
        );
    }
    if std::env::var("VPW_LIGHT_STATS").is_ok() {
        let n = scene.lights.len().max(1);
        let mean = |f: fn(&vpw_table::light::Light) -> f32| {
            scene.lights.iter().map(f).sum::<f32>() / n as f32
        };
        println!("lights            {}", scene.lights.len());
        println!(
            "  intensity       mean {:.4}, max {:.4}, min {:.4}",
            mean(|l| l.intensity),
            scene
                .lights
                .iter()
                .map(|l| l.intensity)
                .fold(0.0f32, f32::max),
            scene
                .lights
                .iter()
                .map(|l| l.intensity)
                .fold(f32::MAX, f32::min)
        );
        println!("  falloff radius  mean {:.1}", mean(|l| l.falloff_radius));
        println!("  falloff power   mean {:.2}", mean(|l| l.falloff_power));
        println!("  modulate        mean {:.2}", mean(|l| l.modulate));
        println!(
            "  vertices        mean {:.0}",
            mean(|l| l.vertices.len() as f32)
        );
        println!("scene lighting");
        println!("  ambient         {:?}", scene.lighting.ambient);
        println!("  emission        {:?}", scene.lighting.emission);
        println!("  exposure        {}", scene.lighting.exposure);
        println!("  bloom strength  {}", scene.lighting.bloom_strength);
        println!("  physics         {:?}", scene.physics);
        println!("  env scale       {}", scene.lighting.env_scale);
        println!("  light range     {}", scene.lighting.range);
        println!("  global emission {}", vpx.gamedata.global_emission_scale);
        println!(
            "  overwrite day/night {:?}",
            vpx.gamedata.overwrite_global_day_night
        );
        println!("  light 0 at      {:?}", scene.lighting.lights[0]);
        for l in scene.lights.iter().take(4) {
            println!(
                "  e.g. {:<20} i {:.4} r {:.0} p {:.2} c {:?}",
                l.name, l.intensity, l.falloff_radius, l.falloff_power, l.color
            );
        }
    }
    // VPW_NO_SCENE_LIGHTS=1 turns the two point lights off, to find out whether
    // they are contributing anything at all.
    if std::env::var("VPW_NO_SCENE_LIGHTS").is_ok() {
        scene.lighting.emission = [0.0; 3];
    }
    // VPW_INTENSITY=<f> scales every light, to find out how far off the scale
    // is rather than arguing about it.
    if let Ok(v) = std::env::var("VPW_INTENSITY")
        && let Ok(f) = v.parse::<f32>()
    {
        for light in &mut scene.lights {
            light.intensity *= f;
        }
    }
    // VPW_FLAT_LIGHTS=1 forces every light onto the plain additive path, to
    // tell "the halo is not being drawn" apart from "the halo is drawn and its
    // blend contributes nothing".
    if std::env::var("VPW_FLAT_LIGHTS").is_ok() {
        for light in &mut scene.lights {
            light.modulate = 0.0;
        }
    }
    let extract_time = t1.elapsed();

    // How bright the playfield's own artwork is before anything lights it.
    // The number to compare a photograph against: a render much darker than
    // the texture is a lighting fault, and one about as dark is a table that
    // was drawn dark.
    if std::env::var("VPW_ART").is_ok() {
        let want = scene.playfield_image.to_ascii_lowercase();
        for img in &vpx.images {
            if img.name.to_ascii_lowercase() != want {
                continue;
            }
            let Some(data) = img.jpeg.as_ref().map(|j| j.data.clone()) else {
                continue;
            };
            if let Ok(decoded) = image::load_from_memory(&data) {
                let rgb = decoded.to_rgb8();
                let n = rgb.pixels().len().max(1) as f64;
                let mean: f64 = rgb
                    .pixels()
                    .map(|p| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) as f64 / 3.0)
                    .sum::<f64>()
                    / n;
                println!("playfield art  {} is {mean:.1}/255 mean", img.name);
            }
        }
    }

    let mut gpu = pollster::block_on(vpw_render::offscreen::Offscreen::new(width, height))
        .expect("could not initialise wgpu");

    // VPW_BLOOM=0 turns the bloom pass off, which is how you find out what it
    // is contributing: two photos of the same table, one with and one without.
    // The table's own, unless a photograph is being taken with and without.
    gpu.set_bloom(scene.lighting.bloom_strength);
    if let Ok(v) = std::env::var("VPW_BLOOM") {
        gpu.set_bloom(v.parse().unwrap_or(1.8));
    }

    let t2 = std::time::Instant::now();
    let gpu_scene = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    // VPW_NO_FLASHERS=1 leaves them out, for a photograph of what they add.
    if std::env::var("VPW_NO_FLASHERS").is_err() {
        gpu.upload_flashers(&scene);
    }
    let upload_time = t2.elapsed();

    // VPW_FLASHERS=1 lists them, with the state the file leaves them in.
    if std::env::var("VPW_FLASHERS").is_ok() {
        println!("{} flashers", scene.flashers.len());
        for f in &scene.flashers {
            let s = &f.state;
            println!(
                "  {:<16} {:?} at ({:.0}, {:.0}) h {:.0} rot {:?} alpha {:.0} {}{} A {:?} B {:?} {:?} {:.0}%",
                f.name,
                f.mode,
                s.x,
                s.y,
                s.height,
                s.rot,
                s.alpha,
                if s.visible { "shown" } else { "hidden" },
                if s.add_blend { " additive" } else { "" },
                s.image_a,
                s.image_b,
                s.filter,
                s.filter_amount
            );
        }
    }

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
            // The same box the player frames on, not the bare rectangle: the
            // playfield in the file is a flat sheet and what has to fit is the
            // sheet plus whatever stands on it. Framing the sheet here and the
            // box there made this photograph flatter than the thing it was
            // supposed to be a photograph of.
            let pf = scene.playfield;
            vpw_render::Camera::for_view_of(
                view,
                (
                    vpw_math::Vec3::new(pf.min.x, pf.min.y, 0.0),
                    vpw_math::Vec3::new(pf.max.x, pf.max.y, 0.0),
                ),
                (head.min, head.max),
                aspect,
                &scene.occupied(),
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

    // Where the edges of the playfield actually land, as a fraction of the
    // screen. This is the number the framing is judged on: with the screen as
    // the glass over the table, anything short of the edge is a black bar and
    // anything past it is a crop.
    {
        let pf = scene.playfield;
        let vp = camera.view_projection(aspect);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for cx in [pf.min.x, pf.max.x] {
            for cy in [pf.min.y, pf.max.y] {
                let clip = vp * vpw_math::Vec3::new(cx, cy, 0.0).extend(1.0);
                x = x.max((clip.x / clip.w).abs());
                y = y.max((clip.y / clip.w).abs());
            }
        }
        println!();
        println!(
            "playfield fills  {:.1}% wide, {:.1}% tall",
            x * 100.0,
            y * 100.0
        );
        let table = (pf.max.x - pf.min.x) / (pf.max.y - pf.min.y);
        println!(
            "table {table:.4} vs screen {aspect:.4}: {:.1}% of bar is unavoidable",
            (1.0 - (table / aspect).min(aspect / table)) * 100.0
        );
    }

    // VPW_ONLY=name draws only the batches whose material or image contains
    // that text. Useful for working out who covers whom.
    let only = std::env::var("VPW_ONLY").ok().map(|s| s.to_lowercase());
    let except = std::env::var("VPW_EXCEPT").ok().map(|s| s.to_lowercase());
    let t3 = std::time::Instant::now();
    // The head, on the same terms the player uses it: a view that does not
    // show it does not draw it.
    let head = !matches!(
        std::env::var("VPW_VIEW").ok().as_deref(),
        Some("overhead") | Some("cenital")
    );
    let pixels = gpu.render_filtered(&gpu_scene, &camera, |b| {
        if !head && b.backbox {
            return false;
        }
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
