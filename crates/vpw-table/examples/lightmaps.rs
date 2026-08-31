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
        if !lm && !p.name.starts_with("BM_") {
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
        println!(
            "     x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}   uv {:.3}..{:.3} , {:.3}..{:.3}",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], uv_lo[0], uv_hi[0], uv_lo[1], uv_hi[1]
        );
    }
}
