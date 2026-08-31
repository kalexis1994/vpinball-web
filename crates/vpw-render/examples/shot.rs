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

    // VPW_HIDE=name leaves out every mesh whose name contains that text,
    // before anything is uploaded. `VPW_ONLY` and `VPW_EXCEPT` work on
    // batches, which are a material and a texture; this works on parts, which
    // is what has a name and stands somewhere. Between the two, "what is that
    // slab" takes a couple of minutes instead of an afternoon.
    // VPW_SHOW=name is the other way round: everything else goes, which is how
    // you find out what the thing in the way actually looks like.
    if let Ok(show) = std::env::var("VPW_SHOW") {
        let show = show.to_lowercase();
        for m in &mut scene.meshes {
            if !m.name.to_lowercase().contains(&show) {
                m.visible = false;
            }
        }
    }
    if let Ok(hide) = std::env::var("VPW_HIDE") {
        let hide = hide.to_lowercase();
        let mut gone = 0;
        for m in &mut scene.meshes {
            if m.visible && m.name.to_lowercase().contains(&hide) {
                m.visible = false;
                gone += 1;
            }
        }
        println!("hidden {gone} meshes matching '{hide}'");
    }

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
    // VPW_MODULATE=<0..1> overrides every halo's modulate-vs-add, to see how
    // much of a dark field is the halos multiplying darkness.
    if let Ok(v) = std::env::var("VPW_MODULATE")
        && let Ok(m) = v.parse()
    {
        for light in &mut scene.lights {
            light.modulate = m;
        }
    }
    // VPW_GLOBAL=<scale> overrides the table's day/night, to separate "the
    // file asks for a dark room" from "the room is darker than asked".
    if let Ok(v) = std::env::var("VPW_GLOBAL")
        && let Ok(g) = v.parse::<f32>()
    {
        let old = scene.lighting.env_scale;
        scene.lighting.env_scale = old * g;
        scene.lighting.ambient = scene.lighting.ambient.map(|c| c * g);
        scene.lighting.emission = scene.lighting.emission.map(|c| c * g);
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
    // VPW_ENVMAP=default lights the table by the map that ships with Visual
    // Pinball instead of its own, which is how you find out what the table's
    // own map is doing: two photos, one under each.
    if std::env::var("VPW_ENVMAP").is_ok_and(|v| v == "default") {
        scene.env_image.clear();
    }
    // VPW_FLAT_LIGHTS=1 forces every light onto the plain additive path, to
    // tell "the halo is not being drawn" apart from "the halo is drawn and its
    // blend contributes nothing".
    if std::env::var("VPW_FLAT_LIGHTS").is_ok() {
        for light in &mut scene.lights {
            light.modulate = 0.0;
        }
    }
    // VPW_HALO_ONLY=1 takes every insert's picture away, so it is drawn as the
    // coloured halo it was before the lit-insert technique existed — the
    // "before" of a before-and-after.
    if std::env::var("VPW_HALO_ONLY").is_ok() {
        for light in &mut scene.lights {
            light.image.clear();
            light.uvs.clear();
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
    // The pieces with a matrix of their own, on request. A baked table keeps
    // nearly everything here — its `BM_*` meshes and its `LM_*` layers of
    // light — so a photo without them is a photo of one mesh.
    //
    // `VPW_NOADD=1` leaves the layers of light out, which is how you find out
    // whether a table that comes out white is being lit twice or added to
    // ninety-six times.
    if std::env::var("VPW_PARTS").is_ok() {
        // The real list: everything the player would animate, physics
        // pieces included, so the photo has flippers in it.
        let collision = vpw_table::physics::build(&vpx);
        let engine =
            vpw_physics::engine::Engine::new(collision, vpw_math::Vec3::new(0.0, 0.0, -1.0));
        let mut parts = vpw_table::animation::animated_parts(&vpx, &engine);
        // What the file hides stays hidden. The player asks the live item
        // every frame — a script shows and hides these constantly — but a
        // photograph has no script.
        parts.retain(|p| p.mesh.visible);
        if std::env::var("VPW_NOADD").is_ok() {
            parts.retain(|p| p.mesh.additive.is_none());
        }
        // VPW_DROP=name leaves out the moving pieces whose name contains that
        // text. `VPW_HIDE` works on the static meshes; this is its opposite
        // number for the pieces with a matrix, which on a baked table is
        // nearly all of them. Rendering with and without one piece and
        // comparing is how you find out whether it was ever being drawn.
        // VPW_LIGHTMAPS=1 puts a bake's `LM_*` layers back, which the real
        // path leaves out. For asking again whether they compose now that the
        // bake is drawn unlit and the coverage alpha is honoured.
        if std::env::var("VPW_LIGHTMAPS").is_ok() {
            let mut added = 0;
            for (index, item) in vpx.gameitems.iter().enumerate() {
                let vpin::vpx::gameitem::GameItemEnum::Primitive(p) = item else {
                    continue;
                };
                if !vpw_table::geometry::is_lightmap(&p.name, &p.image) || !p.is_visible {
                    continue;
                }
                let Some(mesh) = vpw_table::geometry::primitive_part(p) else {
                    continue;
                };
                parts.push(vpw_table::animation::AnimatedPart {
                    mesh,
                    base: vpw_math::Mat4::IDENTITY,
                    local: vpw_math::Mat4::IDENTITY,
                    anim: vpw_table::animation::Animation::Primitive { index },
                });
                added += 1;
            }
            eprintln!("put back {added} lightmap layers");
        }
        // VPW_KEEP=name is the other way round: only the moving pieces whose
        // name contains that text, which is how you look at one of them alone.
        if let Ok(keep) = std::env::var("VPW_KEEP") {
            parts.retain(|p| p.mesh.name.contains(&keep));
            eprintln!("kept {} pieces matching '{keep}'", parts.len());
        }
        if let Ok(drop) = std::env::var("VPW_DROP") {
            let before = parts.len();
            let wanted: Vec<&str> = drop.split(',').filter(|s| !s.is_empty()).collect();
            parts.retain(|p| !wanted.iter().any(|w| p.mesh.name.contains(w)));
            eprintln!("dropped {} pieces matching '{drop}'", before - parts.len());
        }
        eprintln!("parts uploaded: {}", parts.len());
        // VPW_PARTLIST=1 lists every moving piece with the room it takes up
        // and which pass it lands in — for finding the one that is standing
        // over something else.
        if std::env::var("VPW_PARTLIST").is_ok() {
            let mut rows: Vec<(f32, String)> = Vec::new();
            for pt in &parts {
                let alpha = scene.image(&pt.mesh.image).is_some_and(|i| i.has_alpha);
                let clear = scene
                    .material(&pt.mesh.material)
                    .is_some_and(|m| m.is_transparent(alpha));
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in &pt.mesh.vertices {
                    let w = pt
                        .mesh
                        .transform
                        .transform_point3(vpw_math::Vec3::new(v.pos[0], v.pos[1], v.pos[2]));
                    for (c, value) in [w.x, w.y, w.z].into_iter().enumerate() {
                        lo[c] = lo[c].min(value);
                        hi[c] = hi[c].max(value);
                    }
                }
                let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
                rows.push((
                    area,
                    format!(
                        "{:<26} {:<11} bias={:>6} area={:>10.0}  z {:>7.1}..{:<7.1}",
                        pt.mesh.name,
                        if clear { "see-through" } else { "opaque" },
                        pt.mesh.depth_bias,
                        area,
                        lo[2],
                        hi[2]
                    ),
                ));
            }
            rows.sort_by(|a, b| b.0.total_cmp(&a.0));
            eprintln!("the biggest pieces:");
            for (_, line) in rows.iter().take(14) {
                eprintln!("   {line}");
            }
        }
        // VPW_ORDER=1 lists the see-through pieces in the order they will be
        // drawn — by depth bias, more negative last — which is the only way to
        // see who ends up painting over whom.
        if std::env::var("VPW_ORDER").is_ok() {
            let mut blended: Vec<(&str, f32)> = parts
                .iter()
                .filter(|p| {
                    let alpha = scene.image(&p.mesh.image).is_some_and(|i| i.has_alpha);
                    scene
                        .material(&p.mesh.material)
                        .is_some_and(|m| m.is_transparent(alpha))
                })
                .map(|p| (p.mesh.name.as_str(), p.mesh.depth_bias))
                .collect();
            blended.sort_by(|a, b| b.1.total_cmp(&a.1));
            eprintln!(
                "see-through pieces, first drawn to last ({}):",
                blended.len()
            );
            for (name, bias) in &blended {
                eprintln!("   {bias:>8} {name}");
            }
        }
        gpu.upload_dynamic(&scene, &parts);
    }
    let gpu_scene = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    // VPW_BAKE=1 traces the GI relay's lightmap first — shadows and all —
    // which is the difference between light that respects the posts and light
    // that shines through them.
    if std::env::var("VPW_BAKE").is_ok() {
        let t = std::time::Instant::now();
        // VPW_GROUPS=<file> bakes the groups a machine was observed switching
        // (the `table` example's VPW_DUMP_GROUPS writes it) instead of the
        // guessed ones.
        let n = if let Ok(path) = std::env::var("VPW_GROUPS") {
            let text = std::fs::read_to_string(&path).expect("could not read the groups");
            let observed: Vec<Vec<String>> = text
                .lines()
                .map(|l| l.split('\t').map(str::to_string).collect())
                .collect();
            let groups = vpw_render::bake::gi_groups_from_names(&scene, &observed);
            let bake =
                vpw_render::bake::bake_gi_set(&scene, &groups, vpw_render::bake::INDIRECT_SAMPLES);
            let names: Vec<Vec<String>> = groups.iter().map(|g| g.names.clone()).collect();
            gpu.apply_gi_bake(&bake, &names);
            groups.len()
        } else {
            gpu.bake_gi(&scene)
        };
        println!("baked {n} GI groups in {:.1?}", t.elapsed());
    }
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
                "cabinet" | "gabinete" => vpw_render::camera::View::Cabinet,
                _ => vpw_render::camera::View::Front,
            };
            let head = vpw_table::backbox::Backbox::for_playfield(scene.playfield).bounds();
            // A head the table built for itself is not ours to frame, the same
            // way the player treats it.
            let head = if scene.built_head {
                (head.min, head.max)
            } else {
                let pf = scene.playfield;
                (
                    vpw_math::Vec3::new(pf.min.x, pf.min.y, 0.0),
                    vpw_math::Vec3::new(pf.max.x, pf.max.y, 0.0),
                )
            };
            // The same box the player frames on, not the bare rectangle: the
            // playfield in the file is a flat sheet and what has to fit is the
            // sheet plus whatever stands on it. Framing the sheet here and the
            // box there made this photograph flatter than the thing it was
            // supposed to be a photograph of.
            let pf = scene.playfield;
            let mut camera = vpw_render::Camera::for_authored_view(
                view,
                (
                    vpw_math::Vec3::new(pf.min.x, pf.min.y, 0.0),
                    vpw_math::Vec3::new(pf.max.x, pf.max.y, 0.0),
                ),
                head,
                aspect,
                // The same two sets the player uses: the original's for the
                // view the original has, ours for the one it does not.
                &match view {
                    vpw_render::camera::View::Overhead => scene.occupied(),
                    _ => scene.legacy_bounds(),
                },
                Some(match view {
                    vpw_render::camera::View::Cabinet => scene.cabinet,
                    _ => scene.view,
                }),
            );
            // The same start the player uses.
            if matches!(view, vpw_render::camera::View::Front) {
                camera.start_at(&scene.legacy_bounds());
            }
            camera
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

    // VPW_FIT=1 says what the camera is actually fitted to, which is the
    // question behind every "why is it standing there".
    if std::env::var("VPW_FIT").is_ok() {
        let legacy = scene.legacy_bounds();
        let (mut lo, mut hi) = (
            vpw_math::Vec3::splat(f32::MAX),
            vpw_math::Vec3::splat(f32::MIN),
        );
        for p in &legacy {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let kinds = |k| {
            scene
                .meshes
                .iter()
                .filter(|m| m.visible && m.kind == k)
                .count()
        };
        println!(
            "legacy fit: {} corners from {} walls, {} ramps, {} rubbers -> x {:.0}..{:.0} y {:.0}..{:.0} z {:.0}..{:.0}",
            legacy.len(),
            kinds(vpw_table::geometry::MeshKind::Wall),
            kinds(vpw_table::geometry::MeshKind::Ramp),
            kinds(vpw_table::geometry::MeshKind::Rubber),
            lo.x,
            hi.x,
            lo.y,
            hi.y,
            lo.z,
            hi.z
        );
    }

    // VPW_MESHES=1 lists every mesh with where it stands, biggest first.
    //
    // The question this answers is "what is that black slab covering half the
    // playfield", and it is not answerable from the batch list: a batch is a
    // material and a texture, and the thing in the way is a *part*, with a
    // name and a position.
    if std::env::var("VPW_MESHES").is_ok() {
        let mut meshes: Vec<_> = scene
            .meshes
            .iter()
            .filter_map(|m| m.bounds().map(|b| (m, b)))
            .collect();
        // By footprint on the playfield, since what hides a table is what is
        // wide, not what has the most triangles.
        meshes.sort_by(|a, b| {
            let area = |x: &vpw_table::geometry::Bounds| (x.max.x - x.min.x) * (x.max.y - x.min.y);
            area(&b.1).total_cmp(&area(&a.1))
        });
        println!(
            "{:<28} {:>7} {:<22} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
            "mesh", "tris", "image", "x0", "x1", "y0", "y1", "z0", "z1"
        );
        let show: usize = std::env::var("VPW_MESHES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        for (m, b) in meshes.iter().take(show) {
            println!(
                "{:<28} {:>7} {:<22} {:>6.0} {:>6.0} {:>6.0} {:>6.0} {:>6.0} {:>6.0}{}",
                m.name,
                m.triangles(),
                m.image,
                b.min.x,
                b.max.x,
                b.min.y,
                b.max.y,
                b.min.z,
                b.max.z,
                if m.visible { "" } else { "  hidden" }
            );
        }
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
    let room = matches!(
        std::env::var("VPW_VIEW").ok().as_deref(),
        Some("cabinet") | Some("gabinete")
    );
    gpu.room = room;
    let pixels = gpu.render_filtered(&gpu_scene, &camera, |b| {
        if !room && b.scenery {
            return false;
        }
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
    // Which map the table was lit by. On a table with no other light this
    // is the whole exposure, so a photo that says nothing about it cannot
    // be compared with another.
    println!(
        "environment    {} (table asks for {})",
        gpu.pipeline.envmap.source,
        if scene.env_image.is_empty() {
            "nothing"
        } else {
            &scene.env_image
        }
    );
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

    let brightness =
        |p: &[u8; 4]| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) as f64 / 3.0;
    let texels = pixels.as_chunks::<4>().0;
    let not_background = texels
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
    // The number to put next to `playfield art` above: how bright the
    // picture came out. Over every pixel, black bars included, because the
    // set has to be the same in the two photographs being compared; a mean
    // over "the pixels that are not black" moves its own goalposts when the
    // table gets darker.
    println!(
        "rendered mean      {:.1}/255 over the whole picture",
        texels.iter().map(brightness).sum::<f64>() / texels.len().max(1) as f64
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
