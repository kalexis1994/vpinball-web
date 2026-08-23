//! Inventory of a table's game items, to know what has to be drawn.
use std::collections::BTreeMap;
use vpin::vpx::gameitem::GameItemEnum as G;

fn main() {
    let path = std::env::args().nth(1).expect("usage: items <table.vpx>");
    let bytes = std::fs::read(&path).unwrap();
    let vpx = vpin::vpx::from_bytes(&bytes).unwrap();

    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    let mut with_mesh = 0usize;
    let mut total_vertices = 0usize;
    let mut total_triangles = 0usize;

    for item in &vpx.gameitems {
        let name = match item {
            G::Wall(_) => "Wall",
            G::Flipper(_) => "Flipper",
            G::Timer(_) => "Timer",
            G::Plunger(_) => "Plunger",
            G::TextBox(_) => "TextBox",
            G::Bumper(_) => "Bumper",
            G::Trigger(_) => "Trigger",
            G::Light(_) => "Light",
            G::Kicker(_) => "Kicker",
            G::Decal(_) => "Decal",
            G::Gate(_) => "Gate",
            G::Spinner(_) => "Spinner",
            G::Ramp(_) => "Ramp",
            G::Reel(_) => "Reel",
            G::LightSequencer(_) => "LightSeq",
            G::Primitive(_) => "Primitive",
            G::Flasher(_) => "Flasher",
            G::Rubber(_) => "Rubber",
            G::HitTarget(_) => "HitTarget",
            G::Ball(_) => "Ball",
            G::PartGroup(_) => "PartGroup",
            G::Generic(..) => "Generic",
        };
        *count.entry(name).or_default() += 1;

        if let G::Primitive(p) = item
            && let Ok(Some(m)) = p.read_mesh()
        {
            with_mesh += 1;
            total_vertices += m.vertices.len();
            total_triangles += m.indices.len();
        }
    }

    println!(
        "table           {}",
        vpx.info.table_name.unwrap_or_default()
    );
    println!("items           {}", vpx.gameitems.len());
    println!("images          {}", vpx.images.len());
    println!(
        "materials       {}",
        vpx.gamedata.materials.as_ref().map_or(0, |m| m.len())
    );
    println!();
    for (k, v) in &count {
        println!("{k:12} {v:5}");
    }
    println!();
    println!("primitives with mesh  {with_mesh}");
    println!("vertices              {total_vertices}");
    println!("triangles             {total_triangles}");
    let g = &vpx.gamedata;
    println!();
    println!(
        "playfield  x {}..{}  y {}..{}",
        g.left, g.right, g.top, g.bottom
    );
    println!("pf image   {:?}", g.image);
}
