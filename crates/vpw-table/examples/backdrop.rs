//! What a table puts on its desktop backdrop, and where.
//!
//! The original draws backglass-flagged parts in a flat 1000×750 space
//! (`EDITOR_BG_WIDTH`, `stdafx.h:43`) stretched over the whole picture — the
//! same space the backdrop image fills — so a textbox marked DMD is the
//! table's own answer to where the score goes.
//!
//! ```text
//! cargo run -p vpw-table --example backdrop -- table.vpx
//! ```

fn main() {
    let table = std::env::args().nth(1).expect("usage: backdrop <table.vpx>");
    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let g = &vpx.gamedata;
    println!("backdrop desktop=\"{}\" fullscreen=\"{}\" fss=\"{}\" render_decals={} em_reels={}",
        g.backglass_image_full_desktop, g.backglass_image_full_fullscreen,
        g.backglass_image_full_single_screen.clone().unwrap_or_default(), g.render_decals, g.render_em_reels);
    println!("  night_day={} backdrop_color={:?}", g.image_backdrop_night_day, g.backdrop_color);
    for name in [&g.backglass_image_full_desktop, &g.backglass_image_full_fullscreen] {
        if let Some(i) = vpx.images.iter().find(|i| i.name.eq_ignore_ascii_case(name)) {
            println!("  image \"{}\" {}x{} ext={}", i.name, i.width, i.height, i.ext());
        }
    }
    println!(
        "desktop camera: mode={:?} rotation={} inclination={} layback={} fov={} offset=({},{},{}) scale=({},{},{}) view_offset=({:?},{:?}) window top=({:?},{:?},{:?}) bottom=({:?},{:?},{:?})",
        g.bg_view_mode_desktop, g.bg_rotation_desktop, g.bg_inclination_desktop, g.bg_layback_desktop, g.bg_fov_desktop,
        g.bg_offset_x_desktop, g.bg_offset_y_desktop, g.bg_offset_z_desktop,
        g.bg_scale_x_desktop, g.bg_scale_y_desktop, g.bg_scale_z_desktop,
        g.bg_view_horizontal_offset_desktop, g.bg_view_vertical_offset_desktop,
        g.bg_window_top_x_offset_desktop, g.bg_window_top_y_offset_desktop, g.bg_window_top_z_offset_desktop,
        g.bg_window_bottom_x_offset_desktop, g.bg_window_bottom_y_offset_desktop, g.bg_window_bottom_z_offset_desktop,
    );
    println!("playfield: left={} top={} right={} bottom={}", g.left, g.top, g.right, g.bottom);
    // And what the scene made of it: the head, its glass and its windows.
    let scene = vpw_table::geometry::extract(&vpx);
    let glass = scene
        .images
        .iter()
        .find(|i| i.name == vpw_table::backglass::BACKGLASS_IMAGE);
    println!(
        "head: built={} glass={}x{} windows={:?}",
        scene.built_head,
        glass.map_or(0, |i| i.width),
        glass.map_or(0, |i| i.height),
        scene.head_windows
    );
    use vpin::vpx::gameitem::GameItemEnum as G;
    for item in &vpx.gameitems {
        match item {
            G::TextBox(t) => println!(
                "textbox   {:<22} dmd={:<5} at ({:.0},{:.0})-({:.0},{:.0}) text=\"{}\" transparent={}",
                t.name, t.is_dmd.unwrap_or(false), t.ver1.x, t.ver1.y, t.ver2.x, t.ver2.y, t.text, t.is_transparent),
            G::Flasher(f) if f.backglass.unwrap_or(false) || f.is_dmd.unwrap_or(false) => {
                let xs: Vec<f32> = f.drag_points.iter().map(|p| p.x).collect();
                let ys: Vec<f32> = f.drag_points.iter().map(|p| p.y).collect();
                let mn = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
                let mx = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
                println!(
                    "flasher   {:<22} backglass={:<5} dmd={:<5} visible={:<5} at ({:.0},{:.0})-({:.0},{:.0}) image=\"{}\" mode={:?}",
                    f.name, f.backglass.unwrap_or(false), f.is_dmd.unwrap_or(false), f.is_visible,
                    mn(&xs), mn(&ys), mx(&xs), mx(&ys), f.image_a, f.render_mode);
            }
            G::Light(l) if l.is_backglass => println!(
                "light     {:<22} backglass at ({:.0},{:.0})", l.name, l.center.x, l.center.y),
            G::Decal(d) if d.backglass => println!(
                "decal     {:<22} backglass at ({:.0},{:.0}) {}x{} image=\"{}\"", d.name, d.center.x, d.center.y, d.width, d.height, d.image),
            G::Reel(r) => println!(
                "reel      {:<22} at ({:.0},{:.0}) {}x{} reels={} image=\"{}\" visible={}",
                r.name, r.ver1.x, r.ver1.y, r.width, r.height, r.reel_count, r.image, r.is_visible),
            _ => {}
        }
    }
}
