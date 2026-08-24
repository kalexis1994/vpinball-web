//! Which of a table's images the renderer can actually decode, and why not.
//!
//!     cargo run --release -p vpw-render --example textures -- table.vpx
//!
//! Nothing about a texture going wrong is an error anywhere. `upload_texture`
//! answers `None` and the batch falls back to white; a mesh that names an image
//! the table does not have gets the same; a polygon whose texture coordinates
//! all land on one texel comes out a flat colour. All three look identical from
//! the outside — a pale rectangle where somebody's artwork should be — and this
//! tells them apart.
//!
//! Given four more numbers it also answers the question that matters once a
//! photograph has been taken: `x0 y0 x1 y1` lists everything overlapping that
//! patch of table, so a flat rectangle can be turned back into the piece that
//! drew it. That is how the sword ramp on Lord of the Rings was found.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: textures <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let scene = vpw_table::geometry::extract(&vpx);

    println!("{} images", scene.images.len());
    let (mut ok, mut bad) = (0, 0);
    for img in &scene.images {
        let outcome = match (&img.rgba, &img.encoded) {
            (Some(rgba), _) => {
                let want = img.width as usize * img.height as usize * 4;
                if rgba.len() >= want && img.width > 0 && img.height > 0 {
                    None
                } else {
                    Some(format!(
                        "raw pixels: {} bytes for {}x{}, which needs {want}",
                        rgba.len(),
                        img.width,
                        img.height
                    ))
                }
            }
            (None, Some(enc)) => {
                let format = image::guess_format(enc)
                    .map(|f| format!("{f:?}"))
                    .unwrap_or_else(|_| {
                        let head: Vec<String> =
                            enc.iter().take(8).map(|b| format!("{b:02x}")).collect();
                        format!("unrecognised, starts {}", head.join(" "))
                    });
                match image::load_from_memory(enc) {
                    Ok(_) => None,
                    Err(e) => Some(format!("{format}: {e}")),
                }
            }
            (None, None) => Some("no pixels and no encoded bytes".into()),
        };
        match outcome {
            None => ok += 1,
            Some(why) => {
                bad += 1;
                println!("  FAILS  {:<28} {why}", img.name);
            }
        }
    }
    println!();
    println!("{ok} decode, {bad} do not");

    // The other way a piece comes out white: it names an image the table does
    // not have. Nothing complains — the lookup answers `None` and the batch
    // falls back to white — so the only sign is a pale rectangle where
    // somebody's artwork should be.
    println!();
    let mut dangling: Vec<(&str, &str)> = Vec::new();
    let mut untextured = 0;
    for m in &scene.meshes {
        if m.image.is_empty() {
            untextured += 1;
            continue;
        }
        if scene.image(&m.image).is_none() {
            dangling.push((m.name.as_str(), m.image.as_str()));
        }
    }
    println!(
        "{} meshes, {untextured} with no image at all",
        scene.meshes.len()
    );
    if dangling.is_empty() {
        println!("every image a mesh asks for is in the table");
    } else {
        println!(
            "{} meshes ask for an image that is not there:",
            dangling.len()
        );
        for (mesh, image) in &dangling {
            println!("  {mesh:<28} wants {image:?}");
        }
    }

    // The big untextured pieces, which are what a flat pale rectangle in a
    // photograph turns out to be.
    println!();
    let pf = scene.playfield;
    println!(
        "playfield x {:.0}..{:.0}  y {:.0}..{:.0}",
        pf.min.x, pf.max.x, pf.min.y, pf.max.y
    );
    let mut big: Vec<(f32, &str, &str, [f32; 4])> = Vec::new();
    for m in &scene.meshes {
        if !m.image.is_empty() || !m.visible {
            continue;
        }
        let Some(b) = m.bounds() else { continue };
        let area = (b.max.x - b.min.x) * (b.max.y - b.min.y);
        big.push((
            area,
            m.name.as_str(),
            m.material.as_str(),
            [b.min.x, b.min.y, b.max.x, b.max.y],
        ));
    }
    big.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("the ten biggest pieces carrying no image:");
    for (area, name, material, b) in big.iter().take(10) {
        println!(
            "  {area:>10.0}  {name:<24} material {material:<18} x {:.0}..{:.0}  y {:.0}..{:.0}",
            b[0], b[2], b[1], b[3]
        );
    }

    // What colour an untextured piece actually comes out. A material we read
    // as white is a piece that comes out white, and there is nothing to tell it
    // apart from a texture that failed.
    println!();
    println!("materials, as read:");
    let mut names: Vec<&str> = scene.meshes.iter().map(|m| m.material.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    for n in names {
        if n.is_empty() {
            continue;
        }
        match scene.material(n) {
            Some(m) => println!("  {n:<22} {:?}", m),
            None => println!("  {n:<22} NOT IN THE TABLE"),
        }
    }

    // Anything overlapping a region named on the command line, so a flat patch
    // spotted in a photograph can be turned back into the piece that drew it.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 6 {
        let n = |i: usize| args[i].parse::<f32>().unwrap_or(0.0);
        let (x0, y0, x1, y1) = (n(2), n(3), n(4), n(5));
        println!();
        println!("pieces overlapping x {x0}..{x1}, y {y0}..{y1}:");
        for m in &scene.meshes {
            let Some(b) = m.bounds() else { continue };
            if b.max.x < x0 || b.min.x > x1 || b.max.y < y0 || b.min.y > y1 {
                continue;
            }
            println!(
                "  {:<26} kind {:?}  image {:<18} material {:<20} z {:.0}..{:.0}  visible {}",
                m.name, m.kind, m.image, m.material, b.min.z, b.max.z, m.visible
            );
        }
    }

    // The UV range of every mesh that carries an image. A range of nothing is a
    // polygon sampling one texel, which comes out as a patch of flat colour and
    // is indistinguishable from a texture that failed to load.
    println!();
    println!("meshes whose texture coordinates go nowhere:");
    let mut flat_uv = 0;
    for m in &scene.meshes {
        if m.image.is_empty() || m.vertices.len() < 3 {
            continue;
        }
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for v in &m.vertices {
            u0 = u0.min(v.uv[0]);
            u1 = u1.max(v.uv[0]);
            v0 = v0.min(v.uv[1]);
            v1 = v1.max(v.uv[1]);
        }
        if (u1 - u0).abs() < 1e-4 || (v1 - v0).abs() < 1e-4 {
            flat_uv += 1;
            println!(
                "  {:<26} image {:<18} u {u0:.3}..{u1:.3}  v {v0:.3}..{v1:.3}",
                m.name, m.image
            );
        }
    }
    if flat_uv == 0 {
        println!("  none");
    }

    // And the materials, for the same reason.
    let mut no_material: Vec<&str> = Vec::new();
    for m in &scene.meshes {
        if !m.material.is_empty() && scene.material(&m.material).is_none() {
            no_material.push(m.name.as_str());
        }
    }
    if !no_material.is_empty() {
        println!();
        println!(
            "{} meshes name a material that is not there:",
            no_material.len()
        );
        for m in no_material.iter().take(12) {
            println!("  {m}");
        }
    }
}
