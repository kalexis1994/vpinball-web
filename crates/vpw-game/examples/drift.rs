//! Does the table get slower the longer it is played?
//!
//! ```text
//! cargo run --release -p vpw-game --example drift -- table.vpx scripts-dir roms-dir [minutes]
//! ```
//!
//! The question is not "is it fast" but "is it *still* as fast after twenty
//! minutes", and the two have different answers whenever something accumulates.
//! A leak that only costs memory shows up in the resident set and nowhere else;
//! a leak that costs time — a list walked every tick that never stops growing —
//! shows up as the window time climbing while the work per window stays the
//! same. Printing both side by side is what tells the two apart.
//!
//! It plays for real rather than idling: a ball is kept in play, and the
//! flippers are worked, because an idle table exercises almost none of the code
//! that could be accumulating.

use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{AUDIO_RATE, Game, LibraryDir, Resources, ScriptLibrary};

/// One window of table time, in steps. Five seconds.
const WINDOW: u32 = 5_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: drift <table.vpx> <scripts> <roms> [minutes]");
    let scripts = args.next().expect("a scripts directory");
    let roms = args.next().expect("a roms directory");
    let minutes: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(scripts.into()));
    let source: Rc<dyn RomSource> = Rc::new(RomDir(roms.into()));
    let resources = Resources::new(libraries).with_roms(source);

    let mut game = Game::load(&vpx, &mut scene, resources).expect("the table should load");
    game.start().expect("the script should start");
    game.new_ball();
    game.key("Space", true);

    // The host renders audio too, so the probe does: leaving it out would hide
    // anything accumulating in the mixer.
    let chunk = AUDIO_RATE as usize / 100;
    let mut buffer = vec![0.0; chunk * 2];

    // Which timers the script has armed, and how often each wants to run.
    // The sum of their rates is the handler load the port has to carry, and it
    // is a property of the table rather than of this code.
    for _ in 0..2000 {
        game.step();
    }
    let mut armed = game.armed_timers();
    armed.sort_by(|a, b| a.1.total_cmp(&b.1));
    println!("timers armed: {}", armed.len());
    let mut per_second = 0.0;
    for (name, interval) in &armed {
        per_second += 1000.0 / interval.max(1.0);
        println!("  {name:<24} every {interval:>7.1} ms");
    }
    println!("  => {per_second:.0} handler calls a second\n");

    let windows = (minutes * 60.0 * 1000.0 / WINDOW as f64) as u32;
    println!(
        "{:>5}  {:>9}  {:>7}  {:>7}  {:>7}  {:>7}  {:>7}  {:>9}  {:>7}  {:>8}",
        "min",
        "ms/win",
        "xreal",
        "physics",
        "board",
        "events",
        "timers",
        "stmts/k",
        "exprs/k",
        "control"
    );

    let mut plunged = false;
    for w in 0..windows {
        let started = std::time::Instant::now();
        for i in 0..WINDOW {
            let t = w * WINDOW + i;
            if t == 600 {
                game.key("Space", false);
                plunged = true;
            }
            // Work the flippers about twice a second, so the table is played
            // and not merely left running.
            if plunged {
                match t % 500 {
                    0 => {
                        game.key("LeftShift", true);
                        game.key("RightShift", true);
                    }
                    60 => {
                        game.key("LeftShift", false);
                        game.key("RightShift", false);
                    }
                    _ => {}
                }
            }
            game.step();
            // A host renders frames, and two of the timer events belong to a
            // frame rather than to the clock. Sixty a second, which is what a
            // browser gives on a phone; leaving them out would measure a table
            // that never polls its controller.
            if t % 17 == 16 {
                game.game_sync();
                game.new_frame();
            }
            if i % 10 == 9 {
                game.render_audio(&mut buffer);
            }
        }
        let elapsed = started.elapsed().as_secs_f64();

        // A fixed amount of arithmetic, identical in every window and touching
        // nothing the table owns. If this column moves, the machine underneath
        // changed speed and no amount of reading the table's code will explain
        // it. Without a control like this, a throttling phone and a leaking
        // program produce the same graph.
        let control = std::time::Instant::now();
        let mut acc = 0u64;
        for i in 0..40_000_000u64 {
            acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i);
        }
        std::hint::black_box(acc);
        let control_ms = control.elapsed().as_secs_f64() * 1000.0;

        // A drained table stops exercising anything; put another ball up.
        if game.engine.borrow().balls.is_empty() {
            game.new_ball();
            game.key("Space", true);
            for _ in 0..600 {
                game.step();
            }
            game.key("Space", false);
        }

        // The audio column is what is left over: the window minus the four
        // phases the profiler timed, which is `render_audio` and nothing else.
        let p = game.take_profile();
        let (stmts, exprs) = game.script().take_work();
        let ms = |ns: u64| ns as f64 / 1e6;
        let accounted = ms(p.physics_ns + p.board_ns + p.events_ns + p.timers_ns);
        println!(
            "{:>5.1}  {:>9.0}  {:>7.2}  {:>7.0}  {:>7.0}  {:>7.0}  {:>7.0}  {:>9}  {:>7}  {:>8.0}",
            (w + 1) as f64 * WINDOW as f64 / 60_000.0,
            elapsed * 1000.0,
            WINDOW as f64 / 1000.0 / elapsed,
            ms(p.physics_ns),
            ms(p.board_ns),
            ms(p.events_ns),
            ms(p.timers_ns),
            stmts / 1000,
            exprs / 1000,
            control_ms,
        );
        let _ = accounted;
    }
}
