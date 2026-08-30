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

    let mut game = Game::load(&vpx, &mut scene, resources).expect("the table should load");
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
}
