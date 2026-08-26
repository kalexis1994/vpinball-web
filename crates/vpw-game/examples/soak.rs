//! Plays a table badly for a long time, on purpose.
//!
//!     cargo run --release -p vpw-game --example soak -- table.vpx roms/ [seconds]
//!
//! A crash that needs "a few seconds of play" to appear is a crash no single
//! frame test will ever meet: it wants a ball in motion, flippers firing at
//! the wrong moments, drains, replays, the machine's timers all running. This
//! example is that player — coin, start, plunge, mash — for as many table
//! seconds as asked, and a panic anywhere in the step comes out with a native
//! backtrace instead of a wedged canvas.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

const SCRIPTS: &str = "../../../vpinball/scripts";

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: soak <table.vpx> <roms/> [seconds]");
    let roms = args
        .next()
        .expect("usage: soak <table.vpx> <roms/> [seconds]");
    let seconds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);

    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(Path::new(SCRIPTS).into()));
    let source: Rc<dyn RomSource> = Rc::new(RomDir(PathBuf::from(&roms)));
    let mut game = Game::load(
        &vpx,
        &mut scene,
        Resources::new(libraries).with_roms(source),
    )
    .unwrap_or_else(|e| panic!("the table failed to load: {e}"));
    game.start().unwrap_or_else(|e| panic!("Table1_Init: {e}"));
    println!("board running   {}", game.machine().is_running());

    // A held key is a (code, down-at, up-at) triple; the player below is all
    // schedule and no skill.
    let mut audio = vec![0.0f32; 2 * 735];
    for t in 0..seconds * 1000 {
        // Coin, start, and a fresh ball straight in front of the plunger.
        if t == 2_000 {
            game.key("Digit5", true);
        }
        if t == 2_060 {
            game.key("Digit5", false);
        }
        if t == 4_000 {
            game.key("Digit1", true);
        }
        if t == 4_060 {
            game.key("Digit1", false);
        }
        // A ball every eight seconds, plunged hard: a second's pull, then let
        // go. Whatever the last ball was doing, tough — that is what a player
        // hammering the new-ball key does too.
        let cycle = t % 8_000;
        if t >= 6_000 {
            if cycle == 0 {
                game.new_ball();
            }
            if cycle == 200 {
                game.key("Space", true);
            }
            if cycle == 1_200 {
                game.key("Space", false);
            }
            // Both flippers, out of phase, faster than any ball deserves.
            if cycle % 700 == 300 {
                game.key("ShiftLeft", true);
            }
            if cycle % 700 == 500 {
                game.key("ShiftLeft", false);
            }
            if cycle % 900 == 400 {
                game.key("ShiftRight", true);
            }
            if cycle % 900 == 650 {
                game.key("ShiftRight", false);
            }
        }
        game.step();
        if t % 17 == 16 {
            game.game_sync();
            game.new_frame();
        }
        // The audio pump too: in the browser it runs beside the frame loop
        // and its borrow is part of the same story.
        if t % 16 == 0 {
            game.render_audio(&mut audio);
        }
        if t % 10_000 == 0 && t > 0 {
            let balls = game.engine.borrow().balls.len();
            let lit = game
                .machine()
                .segments()
                .iter()
                .filter(|s| **s != 0)
                .count();
            println!("{:>3}s  balls {balls}  lit segments {lit}", t / 1000);
        }
    }
    println!("survived {seconds}s of bad pinball");
}
