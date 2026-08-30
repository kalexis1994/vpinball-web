//! What a table carries against what comes out of the extractor.
//!
//! For the complaint that never has a line number: "it looks like something is
//! missing". Counting both sides says which kind, and whether it was dropped on
//! the way in or never drawn.

use std::collections::BTreeMap;

use vpin::vpx::gameitem::GameItemEnum as G;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: inventory <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");

    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut hidden: BTreeMap<&str, usize> = BTreeMap::new();
    for i in &vpx.gameitems {
        let (k, shown) = match i {
            G::Wall(w) => ("wall", w.is_top_bottom_visible),
            G::Flipper(f) => ("flipper", f.is_visible),
            G::Timer(_) => ("timer", false),
            G::Plunger(p) => ("plunger", p.is_visible),
            G::TextBox(_) => ("textbox", false),
            G::Bumper(_) => ("bumper", true),
            G::Trigger(t) => ("trigger", t.is_visible),
            G::Light(_) => ("light", true),
            G::Kicker(_) => ("kicker", true),
            G::Decal(_) => ("decal", true),
            G::Gate(g) => ("gate", g.is_visible),
            G::Spinner(s) => ("spinner", s.is_visible),
            G::Ramp(r) => ("ramp", r.is_visible),
            G::Reel(_) => ("reel", true),
            G::LightSequencer(_) => ("lightseq", false),
            G::Primitive(p) => ("primitive", p.is_visible),
            G::Flasher(f) => ("flasher", f.is_visible),
            G::Rubber(r) => ("rubber", r.is_visible),
            G::HitTarget(h) => ("hittarget", h.is_visible),
            _ => ("other", false),
        };
        *kinds.entry(k).or_default() += 1;
        if !shown {
            *hidden.entry(k).or_default() += 1;
        }
    }

    println!("in the file:");
    for (k, n) in &kinds {
        let h = hidden.get(k).copied().unwrap_or(0);
        println!("  {k:<10} {n:>5}   {h} not visible");
    }

    let scene = vpw_table::geometry::extract(&vpx);
    let tris: usize = scene.meshes.iter().map(|m| m.indices.len() / 3).sum();
    println!(
        "\nout of the extractor: {} meshes, {tris} triangles, {} lights, {} flashers",
        scene.meshes.len(),
        scene.lights.len(),
        scene.flashers.len()
    );

    // And which of them are hidden, which is not the same as absent: a VLM
    // table bakes its lighting into new meshes and hides the originals.
    let shown = scene.meshes.iter().filter(|m| m.visible).count();
    let shown_tris: usize = scene
        .meshes
        .iter()
        .filter(|m| m.visible)
        .map(|m| m.indices.len() / 3)
        .sum();
    println!("  of those, {shown} visible with {shown_tris} triangles");

    let mut by_prefix: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for m in &scene.meshes {
        let key = if m.name.starts_with("LM_") {
            "LM_* (lightmaps)".to_string()
        } else if m.name.starts_with("BM_") {
            "BM_* (baked)".to_string()
        } else {
            "everything else".to_string()
        };
        let e = by_prefix.entry(key).or_default();
        e.0 += 1;
        e.1 += m.indices.len() / 3;
    }
    println!();
    for (k, (n, t)) in &by_prefix {
        println!("  {k:<18} {n:>4} meshes, {t:>8} triangles");
    }

    // Which primitives are asking the physics for work, and how much.
    let mut heavy: Vec<(usize, String, bool, bool)> = Vec::new();
    for i in &vpx.gameitems {
        let G::Primitive(p) = i else { continue };
        let tris = p.read_mesh().ok().flatten().map_or(0, |m| m.indices.len());
        if p.is_collidable && !p.is_toy && tris > 0 {
            heavy.push((tris, p.name.clone(), p.is_collidable, p.is_toy));
        }
    }
    heavy.sort_unstable_by_key(|h| std::cmp::Reverse(h.0));
    let total: usize = heavy.iter().map(|h| h.0).sum();
    println!(
        "
collidable primitives: {} of them, {total} triangles of collision",
        heavy.len()
    );
    for (t, n, _, _) in heavy.iter().take(10) {
        println!("  {n:<32} {t:>8}");
    }

    // And where the collision shapes actually come from, which is not always
    // where the triangles are.
    let c = vpw_table::physics::build_with_owners(&vpx);
    let mut per: BTreeMap<usize, usize> = BTreeMap::new();
    for i in c.owners.iter().flatten() {
        *per.entry(*i).or_default() += 1;
    }
    let mut rows: Vec<(usize, String)> = per
        .iter()
        .map(|(i, n)| {
            let name = match &vpx.gameitems[*i] {
                G::Wall(w) => format!("wall {}", w.name),
                G::Ramp(r) => format!("ramp {}", r.name),
                G::Rubber(r) => format!("rubber {}", r.name),
                G::Primitive(p) => format!("primitive {}", p.name),
                G::Trigger(t) => format!("trigger {}", t.name),
                other => format!("{other:?}").chars().take(24).collect(),
            };
            (*n, name)
        })
        .collect();
    rows.sort_unstable_by_key(|r| std::cmp::Reverse(r.0));
    println!(
        "
collision shapes: {} in all",
        c.shapes.len()
    );
    for (n, name) in rows.iter().take(10) {
        println!("  {name:<40} {n:>8}");
    }
}
