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
