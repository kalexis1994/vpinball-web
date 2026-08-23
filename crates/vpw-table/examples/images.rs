//! What images a table carries, and what the file names its backdrop.
fn main() {
    let path = std::env::args().nth(1).expect("usage: images <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let g = &vpx.gamedata;
    println!("playfield image   {:?}", g.image);
    println!("view modes        desktop {:?}", g.bg_view_mode_desktop);
    println!(
        "  rotation {} inclination {} fov {} layback {}",
        g.bg_rotation_desktop, g.bg_inclination_desktop, g.bg_fov_desktop, g.bg_layback_desktop
    );
    println!(
        "  offset ({}, {}, {})  scale ({}, {}, {})",
        g.bg_offset_x_desktop,
        g.bg_offset_y_desktop,
        g.bg_offset_z_desktop,
        g.bg_scale_x_desktop,
        g.bg_scale_y_desktop,
        g.bg_scale_z_desktop
    );
    println!();
    // Who uses each image: an image nobody references is dead weight, and one
    // used by a primitive is a surface that already exists in the file.
    use vpin::vpx::gameitem::GameItemEnum as G;
    let want = std::env::args().nth(2);
    for item in &vpx.gameitems {
        let (kind, name, image, pos) = match item {
            G::Primitive(p) => (
                "primitive",
                p.name.clone(),
                p.image.clone(),
                format!(
                    "({:.0},{:.0},{:.0}) size ({:.0},{:.0},{:.0})",
                    p.position.x, p.position.y, p.position.z, p.size.x, p.size.y, p.size.z
                ),
            ),
            G::Flasher(f) => (
                "flasher",
                f.name.clone(),
                f.image_a.clone(),
                format!("({:.0},{:.0},{:.0})", f.pos_x, f.pos_y, f.height),
            ),
            _ => continue,
        };
        if image.is_empty() {
            continue;
        }
        if let Some(w) = &want
            && !image.to_lowercase().contains(&w.to_lowercase())
        {
            continue;
        }
        println!("  {kind:10} {name:22} image {image:20} at {pos}");
    }
}
