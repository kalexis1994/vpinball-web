//! What a baked table's lightmap primitives actually say about themselves.

use vpin::vpx::gameitem::GameItemEnum as G;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: lightmaps <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut shown = 0;
    for i in &vpx.gameitems {
        let G::Primitive(p) = i else { continue };
        let lm = vpw_table::geometry::is_lightmap(&p.name, &p.image);
        let wanted = std::env::var("ONLY").unwrap_or_default();
        if !wanted.is_empty() {
            if !p.name.to_lowercase().contains(&wanted.to_lowercase()) {
                continue;
            }
        } else if !lm && !p.name.starts_with("BM_") {
            continue;
        }
        if lm {
            shown += 1;
            if shown > 6 {
                continue;
            }
        }
        println!(
            "{:<34} lightmap={lm} visible={} add_blend={:?} alpha={:?} \
             dlt={:?} dlb={:?} depth_bias={} static={} toy={} collidable={} material={:?}",
            p.name,
            p.is_visible,
            p.add_blend,
            p.alpha,
            p.disable_lighting_top,
            p.disable_lighting_below,
            p.depth_bias,
            p.static_rendering,
            p.is_toy,
            p.is_collidable,
            p.material,
        );
        println!("      light_map={:?} color={:?}", p.light_map, p.color);
        println!(
            "      position=({:.1},{:.1},{:.1}) size=({:.3},{:.3},{:.3}) rot_tra={:?}",
            p.position.x,
            p.position.y,
            p.position.z,
            p.size.x,
            p.size.y,
            p.size.z,
            p.rot_and_tra.map(|v| (v * 100.0).round() / 100.0)
        );
    }

    if std::env::var("ONLY").is_ok() {
        // A different question: which meshes sit at the bottom edge of the
        // table, where the apron is.
        let scene = vpw_table::geometry::extract(&vpx);
        let bottom = vpx.gamedata.bottom;
        let mut rows: Vec<(f32, String)> = Vec::new();
        for m in &scene.meshes {
            let (mut ylo, mut yhi, mut zlo, mut zhi) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for v in &m.vertices {
                let w = m
                    .transform
                    .transform_point3(vpw_math::Vec3::new(v.pos[0], v.pos[1], v.pos[2]));
                ylo = ylo.min(w.y);
                yhi = yhi.max(w.y);
                zlo = zlo.min(w.z);
                zhi = zhi.max(w.z);
            }
            let want = std::env::var("ONLY").unwrap_or_default();
            let by_name = want != "bottom" && m.name.to_lowercase().contains(&want.to_lowercase());
            if by_name || (want == "bottom" && yhi > bottom * 0.86) {
                rows.push((
                    ylo,
                    format!(
                        "{:<30} y {:>7.0}..{:<7.0} z {:>6.1}..{:<6.1} visible={} tris={}",
                        m.name,
                        ylo,
                        yhi,
                        zlo,
                        zhi,
                        m.visible,
                        m.indices.len() / 3
                    ),
                ));
            }
        }
        // And the raw items in that band, whether or not we built a mesh for
        // one: a piece we drop entirely would never show up above.
        use vpin::vpx::gameitem::GameItemEnum as G;
        println!(
            "backdrop: desktop={:?} fullscreen={:?} colour={:?} render_decals={:?}",
            vpx.gamedata.backglass_image_full_desktop,
            vpx.gamedata.backglass_image_full_fullscreen,
            vpx.gamedata.backdrop_color,
            vpx.gamedata.render_decals
        );
        println!("who uses the desktop images:");
        for m in &scene.meshes {
            if m.image.to_lowercase().contains("dtcircus")
                || m.image.to_lowercase().contains("backdrop")
            {
                println!(
                    "  {:<30} image={:<24} visible={}",
                    m.name, m.image, m.visible
                );
            }
        }
        for it in &vpx.gameitems {
            if let vpin::vpx::gameitem::GameItemEnum::Primitive(p) = it
                && (p.image.to_lowercase().contains("dtcircus")
                    || p.image.to_lowercase().contains("backdrop"))
            {
                {
                    println!(
                        "  raw primitive {:<24} image={:<20} visible={}",
                        p.name, p.image, p.is_visible
                    );
                }
            }
        }
        println!("images the table carries:");
        for i in &scene.images {
            println!("  {:<32} {}x{}", i.name, i.width, i.height);
        }
        println!("raw items near the bottom:");
        for it in &vpx.gameitems {
            let (kind, name, y) = match it {
                G::Wall(w) => (
                    "wall",
                    w.name.clone(),
                    w.drag_points.iter().map(|d| d.y).fold(f32::MIN, f32::max),
                ),
                G::Ramp(r) => (
                    "ramp",
                    r.name.clone(),
                    r.drag_points.iter().map(|d| d.y).fold(f32::MIN, f32::max),
                ),
                G::Rubber(r) => (
                    "rubber",
                    r.name.clone(),
                    r.drag_points.iter().map(|d| d.y).fold(f32::MIN, f32::max),
                ),
                G::Primitive(p) => ("primitive", p.name.clone(), p.position.y),
                G::Decal(d) => ("decal", d.name.clone(), d.center.y),
                G::Flasher(f) => ("flasher", f.name.clone(), f.pos_y),
                _ => continue,
            };
            if y > 1950.0 {
                // Walls and ramps come out under derived names — "Wall3
                // (top)", "instcard (floor)" — so match the stem.
                let built = scene
                    .meshes
                    .iter()
                    .filter(|m| m.name.starts_with(&name))
                    .count();
                println!("  {kind:<10} {name:<28} y~{y:.0}  meshes={built}");
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        println!("the table's bottom edge is y = {bottom:.0}");
        for (_, line) in &rows {
            println!("  {line}");
        }
        return;
    }
    let mut gi = 0;
    for i in &vpx.gameitems {
        let G::Light(l) = i else { continue };
        if l.name.starts_with("gi_") || gi < 3 {
            gi += 1;
            if gi < 8 {
                println!(
                    "light {:<12} intensity={} state={:?} visible={:?} bulb={:?}",
                    l.name, l.intensity, l.state, l.visible, l.is_bulb_light
                );
            }
        }
    }
    let scene = vpw_table::geometry::extract(&vpx);
    println!("lights in the scene: {}", scene.lights.len());
    let has: Vec<&str> = scene
        .lights
        .iter()
        .map(|l| l.name.as_str())
        .filter(|n| n.starts_with("gi_"))
        .collect();
    println!("gi_* that survived extraction: {:?}", has);

    let names: Vec<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|i| match i {
            G::Light(l) => Some(l.name.as_str()),
            _ => None,
        })
        .filter(|n| n.contains("001") || n.contains("gi_"))
        .collect();
    println!("light names with gi_ or 001: {names:?}");
    let wanted: Vec<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|i| match i {
            G::Primitive(p) => p.light_map.as_deref(),
            _ => None,
        })
        .collect();
    let mut uniq: Vec<&str> = wanted.clone();
    uniq.sort_unstable();
    uniq.dedup();
    println!("light_map values the lightmaps ask for: {uniq:?}");

    // Which pass each bake mesh lands in, which is what decides whether its
    // depth bias means anything at all.
    let scene = vpw_table::geometry::extract(&vpx);
    for m in &scene.meshes {
        if !m.name.starts_with("BM_") {
            continue;
        }
        let with_alpha = scene.image(&m.image).is_some_and(|i| i.has_alpha);
        let transparent = scene
            .material(&m.material)
            .is_some_and(|mat| mat.is_transparent(with_alpha));
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut uv_lo = [f32::MAX; 2];
        let mut uv_hi = [f32::MIN; 2];
        for v in &m.vertices {
            for c in 0..3 {
                lo[c] = lo[c].min(v.pos[c]);
                hi[c] = hi[c].max(v.pos[c]);
            }
            for c in 0..2 {
                uv_lo[c] = uv_lo[c].min(v.uv[c]);
                uv_hi[c] = uv_hi[c].max(v.uv[c]);
            }
        }
        println!(
            "{:<24} bias={:>6} transparent={transparent} tris={:>6}",
            m.name,
            m.depth_bias,
            m.indices.len() / 3
        );
        // In world space, which is the only space in which two meshes can be
        // said to be over or under one another.
        let mut wlo = [f32::MAX; 3];
        let mut whi = [f32::MIN; 3];
        for v in &m.vertices {
            let w = m
                .transform
                .transform_point3(vpw_math::Vec3::new(v.pos[0], v.pos[1], v.pos[2]));
            for (c, value) in [w.x, w.y, w.z].into_iter().enumerate() {
                wlo[c] = wlo[c].min(value);
                whi[c] = whi[c].max(value);
            }
        }
        println!(
            "     world x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.1}..{:.1}",
            wlo[0], whi[0], wlo[1], whi[1], wlo[2], whi[2]
        );
        println!(
            "     x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}   uv {:.3}..{:.3} , {:.3}..{:.3}",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], uv_lo[0], uv_hi[0], uv_lo[1], uv_hi[1]
        );
    }

    for m in &scene.materials {
        if !m.name.starts_with("VLM") {
            continue;
        }
        println!(
            "material {:<18} opacity={:.3} opacity_active={} base={:?} metal={}",
            m.name, m.opacity, m.opacity_active, m.base_color, m.is_metal
        );
    }
    // And what the file itself says, before we read it.
    if let Some(mats) = &vpx.gamedata.materials {
        for m in mats {
            if !m.name.starts_with("VLM") {
                continue;
            }
            println!(
                "   file: {:<18} opacity={:.3} opacity_active={} type={:?}",
                m.name, m.opacity, m.opacity_active, m.type_
            );
        }
    }

    // The base and three of its layers, side by side: same geometry, and the
    // question is whether they read the same texels.
    for m in &scene.meshes {
        let want = m.name == "BM_Playfield"
            || (m.name.starts_with("LM_") && m.name.ends_with("_Playfield"));
        if !want {
            continue;
        }
        let (mut ulo, mut uhi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &m.vertices {
            for c in 0..2 {
                ulo[c] = ulo[c].min(v.uv[c]);
                uhi[c] = uhi[c].max(v.uv[c]);
            }
        }
        println!(
            "{:<36} verts={:<6} image={:<14} uv {:.4}..{:.4} , {:.4}..{:.4}",
            m.name,
            m.vertices.len(),
            m.image,
            ulo[0],
            uhi[0],
            ulo[1],
            uhi[1]
        );
    }
}
