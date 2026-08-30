//! Loads a table and evaluates snippets against its running script.
//!
//! For reading an error message backwards: a table says "division by zero" at
//! a line, and the question is always which of the names on that line is not
//! what the table thinks it is.
//!
//! ```text
//! cargo run -p vpw-game --example probe -- table.vpx "LeftFlipper.Return" "x.y"
//! ```

use std::path::Path;
use std::rc::Rc;

use vpw_game::{Game, LibraryDir, NoLibraries, Resources, ScriptLibrary};

const SCRIPTS: &str = "../vpinball/scripts";
const ROMS: &str = "web/debug-assets/roms";

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args.next().expect("usage: probe <table.vpx> <expr>...");
    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = if Path::new(SCRIPTS).is_dir() {
        Rc::new(LibraryDir(Path::new(SCRIPTS).into()))
    } else {
        Rc::new(NoLibraries)
    };
    let mut resources = Resources::new(libraries);
    if Path::new(ROMS).is_dir() {
        resources = resources.with_roms(Rc::new(vpw_game::controller::RomDir(ROMS.into())));
    }

    let count = |scene: &vpw_table::geometry::Scene| {
        let shown = scene.meshes.iter().filter(|m| m.visible).count();
        let tris: usize = scene
            .meshes
            .iter()
            .filter(|m| m.visible)
            .map(|m| m.indices.len() / 3)
            .sum();
        (scene.meshes.len(), shown, tris)
    };
    let before = count(&scene);

    let mut game = Game::load(&vpx, &mut scene, resources).expect("the table should load");
    let after = count(&scene);
    for m in &scene.meshes {
        println!(
            "   static: {:<28} {:>7} tris  visible={}  material={:?} image={:?}",
            m.name,
            m.indices.len() / 3,
            m.visible,
            m.material,
            m.image
        );
    }
    {
        use std::collections::BTreeMap;
        let mut by: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
        for pt in game.parts() {
            let k = if pt.mesh.name.starts_with("LM_") {
                "LM_*".to_string()
            } else if pt.mesh.name.starts_with("BM_") {
                format!("BM: {}", pt.mesh.name)
            } else {
                "other".to_string()
            };
            let e = by.entry(k).or_default();
            e.0 += 1;
            e.1 += pt.mesh.indices.len() / 3;
            if pt.mesh.visible {
                e.2 += 1;
            }
        }
        for (k, (n, t, v)) in &by {
            println!("   part {k:<28} {n:>4} meshes {t:>8} tris, {v} visible");
        }
        // Do the materials and textures these meshes name actually exist?
        let mut missing_mat: BTreeMap<String, usize> = BTreeMap::new();
        let mut missing_img: BTreeMap<String, usize> = BTreeMap::new();
        let mut check = |m: &vpw_table::geometry::Mesh| {
            if !m.material.is_empty() && scene.material(&m.material).is_none() {
                *missing_mat.entry(m.material.clone()).or_default() += 1;
            }
            if !m.image.is_empty() && scene.image(&m.image).is_none() {
                *missing_img.entry(m.image.clone()).or_default() += 1;
            }
        };
        for pt in game.parts() {
            check(&pt.mesh);
        }
        for m in &scene.meshes {
            check(m);
        }
        println!("   materials named but not in the table: {missing_mat:?}");
        println!("   images named but not in the table:    {missing_img:?}");
        // What one lightmap looks like, since there is no code that knows what
        // one is.
        if let Some(lm) = game.parts().iter().find(|p| p.mesh.name.starts_with("LM_")) {
            println!(
                "   a lightmap: {} material={:?} image={:?}",
                lm.mesh.name, lm.mesh.material, lm.mesh.image
            );
        }
        println!("   images in the table:");
        for i in &scene.images {
            if i.name.to_lowercase().contains("nestmap") {
                println!(
                    "     {} {}x{} alpha={}",
                    i.name, i.width, i.height, i.has_alpha
                );
            }
        }
    }
    let shown_parts = game.parts().iter().filter(|p| p.mesh.visible).count();
    let part_tris: usize = game
        .parts()
        .iter()
        .filter(|p| p.mesh.visible)
        .map(|p| p.mesh.indices.len() / 3)
        .sum();
    println!(
        "   parts: {shown_parts} visible of {}, {part_tris} triangles",
        game.parts().len()
    );
    println!(
        "-- scene: {} meshes ({} visible, {} triangles) -> {} meshes ({} visible, {} triangles), {} moving parts --",
        before.0,
        before.1,
        before.2,
        after.0,
        after.1,
        after.2,
        game.parts().len()
    );
    match game.start() {
        Ok(()) => println!("-- the script started --"),
        Err(e) => println!("-- the script failed to start: {e} --"),
    }

    println!(
        "-- {} sounds in the table --",
        vpw_table::sound::extract(&vpx).len()
    );

    for (i, expr) in args.enumerate() {
        // A statement rather than a value: anything with a space in it that is
        // not an expression — `PlaySound "x"` — is run as it stands.
        if expr.contains(' ') && !expr.contains('=') {
            match game.script().load(&expr) {
                Ok(()) => println!("{expr} -- ran"),
                Err(e) => println!("{expr} -> {e}"),
            }
            continue;
        }
        let name = format!("__probe{i}");
        let src = format!("Dim {name} : {name} = {expr}");
        match game.script().load(&src) {
            Ok(()) => {
                let v = game.script().get_global(&name);
                println!("{expr} = {v:?}");
            }
            Err(e) => println!("{expr} -> {e}"),
        }
    }

    // What comes out of the mixer over the next second of table time, with
    // whatever the expressions above set going.
    let mut peak = 0.0f32;
    let mut out = vec![0.0f32; 1024];
    for _ in 0..60 {
        for _ in 0..17 {
            game.step();
        }
        game.render_audio(&mut out);
        peak = peak.max(out.iter().fold(0.0f32, |a, b| a.max(b.abs())));
    }
    println!("-- mixer peak over a second: {peak:.4} --");

    // The same questions again, now that the table has been running: a lamp
    // that fades up takes a second or two to get there.
    for (i, expr) in std::env::args().skip(2).enumerate() {
        if expr.contains(' ') && !expr.contains('=') {
            continue;
        }
        let name = format!("__after{i}");
        if game
            .script()
            .load(&format!("Dim {name} : {name} = {expr}"))
            .is_ok()
        {
            println!(
                "after a second: {expr} = {:?}",
                game.script().get_global(&name)
            );
        }
    }
}
