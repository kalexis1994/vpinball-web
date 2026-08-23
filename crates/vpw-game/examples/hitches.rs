//! Times every physics step of a real game and reports the slow ones.
//!
//! ```text
//! cargo run --release -p vpw-game --example hitches -- table.vpx scripts-dir roms-dir [seconds]
//! ```
//!
//! The loop runs at a thousand steps a second, so a step has a millisecond of
//! real time to spend and no more. What matters is not the average — that has
//! been comfortable for a long time — but the **tail**: one step in a thousand
//! that takes fifty milliseconds is invisible in an average and is exactly what
//! a player feels as a hitch.
//!
//! It reports the distribution, the worst offenders and, for anything periodic,
//! the gap between them: a stall that arrives on a fixed beat is somebody's
//! timer, and the gap names it.

use std::rc::Rc;
use std::time::Instant;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: hitches <table.vpx> <scripts-dir> <roms-dir> [seconds]");
    let scripts = args.next().expect("scripts dir");
    let roms = args.next().expect("roms dir");
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30.0);

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(scripts.into()));
    let rom_source: Rc<dyn RomSource> = Rc::new(RomDir(roms.into()));
    let mut game = Game::load(
        &vpx,
        &mut scene,
        Resources::new(libraries).with_roms(rom_source),
    )
    .expect("the table should load");
    game.start().expect("the script should start");

    // Boot the board, put a coin in and start a game, so the measurement is of
    // a table that is actually running its rules and not of an idle one.
    for _ in 0..4000 {
        game.step();
    }
    for (key, hold, after) in [("Digit5", 60, 2500), ("Digit1", 60, 3000)] {
        game.key(key, true);
        for _ in 0..hold {
            game.step();
        }
        game.key(key, false);
        for _ in 0..after {
            game.step();
        }
    }
    game.new_ball();

    let steps = (seconds * 1000.0) as usize;
    let mut us = vec![0u32; steps];
    let mut context: Vec<(usize, u32, usize, u64, Vec<String>)> = Vec::new();
    let mut handlers = game.handlers_fired();
    for (i, slot) in us.iter_mut().enumerate() {
        // On purpose: **not** draining, because the browser player does not
        // drain either. If a queue grows without bound this is where it shows.
        let t = Instant::now();
        game.step();
        *slot = t.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;

        let fired = game.handlers_fired();
        if *slot > 900 {
            context.push((
                i,
                *slot,
                game.engine.borrow().balls.len(),
                fired - handlers,
                game.take_sounds(),
            ));
        }
        handlers = fired;
    }

    if !context.is_empty() {
        println!("what the slow steps were doing:");
        for (i, us, balls, fired, sounds) in &context {
            println!(
                "  step {i:7}  {us:7} us   {balls} balls, {fired} handlers, sounds {sounds:?}"
            );
        }
        println!();
    }

    println!(
        "queues nobody drained: {} sounds, {} messages",
        game.take_sounds().len(),
        game.take_messages().len()
    );

    // What the host does once a frame, on top of the steps: it asks the table
    // for the level of every light and the pose of every moving part, and it
    // asks by **name**.
    let names: Vec<String> = game
        .items()
        .iter()
        .filter(|it| matches!(it.kind, vpw_game::items::Kind::Light))
        .map(|it| it.name.to_string())
        .collect();
    let parts: Vec<String> = game.parts().iter().map(|p| p.mesh.name.clone()).collect();
    let mut worst_frame = 0u128;
    let mut total_frame = 0u128;
    const FRAMES: usize = 600;
    for _ in 0..FRAMES {
        let t = Instant::now();
        let mut acc = 0.0f32;
        for n in &names {
            acc += game.items().get(n).map_or(1.0, |it| it.light_level());
        }
        for n in &parts {
            acc += f32::from(game.items().get(n).is_some_and(|it| it.visible()));
        }
        std::hint::black_box(acc);
        let e = t.elapsed().as_micros();
        worst_frame = worst_frame.max(e);
        total_frame += e;
    }
    println!();
    println!(
        "looking up {} lights and {} parts by name, per frame: mean {:.0} us, worst {} us",
        names.len(),
        parts.len(),
        total_frame as f64 / FRAMES as f64,
        worst_frame
    );

    // A frame the way the page really runs it: sixteen milliseconds of physics
    // and then the audio for the time that just passed. The audio pump aims to
    // keep a tenth of a second queued and will render up to a quarter of a
    // second in one go if it has fallen behind, so both sizes are worth timing.
    let rate = vpw_game::AUDIO_RATE as usize;
    for (label, frames) in [
        ("16 ms", rate / 60),
        ("0.1 s", rate / 10),
        ("0.25 s", rate / 4),
    ] {
        let mut buf = vec![0.0f32; frames * 2];
        let mut worst = 0u128;
        let mut total = 0u128;
        for _ in 0..120 {
            for _ in 0..16 {
                game.step();
            }
            let t = Instant::now();
            game.render_audio(&mut buf);
            let e = t.elapsed().as_micros();
            worst = worst.max(e);
            total += e;
        }
        println!(
            "rendering {label} of audio in one go: mean {:.0} us, worst {worst} us",
            total as f64 / 120.0
        );
    }

    // How many lights actually change from one frame to the next. Each one is
    // a separate write to the GPU in `sync`, and the ones that do not change
    // are skipped, so this is the real per-frame cost of the lighting.
    {
        let mut last: Vec<f32> = names
            .iter()
            .map(|n| game.items().get(n).map_or(1.0, |it| it.light_level()))
            .collect();
        let mut changes = Vec::new();
        for _ in 0..300 {
            for _ in 0..16 {
                game.step();
            }
            let mut n = 0;
            for (i, name) in names.iter().enumerate() {
                let level = game.items().get(name).map_or(1.0, |it| it.light_level());
                if (level - last[i]).abs() >= 1e-4 {
                    last[i] = level;
                    n += 1;
                }
            }
            changes.push(n);
        }
        let busy: Vec<(usize, usize)> = changes
            .iter()
            .copied()
            .enumerate()
            .filter(|&(_, n)| n > 60)
            .collect();
        let mut sorted = changes.clone();
        sorted.sort_unstable();
        println!(
            "lights that change per frame: median {}, p90 {}, worst {} (of {})",
            sorted[sorted.len() / 2],
            sorted[sorted.len() * 9 / 10],
            sorted[sorted.len() - 1],
            names.len()
        );
        println!(
            "  frames with more than 60 changing, out of {}: {:?}",
            changes.len(),
            busy
        );
    }

    // How many moving parts actually move from one frame to the next. Every
    // one is a separate write to the GPU in `sync`, and most of a table's
    // "moving" parts stand still most of the time.
    {
        let mut last: Vec<vpw_math::Mat4> = (0..game.parts().len())
            .map(|i| game.part_transform(i))
            .collect();
        let mut changes = Vec::new();
        for _ in 0..300 {
            for _ in 0..16 {
                game.step();
            }
            let mut n = 0;
            for (i, was) in last.iter_mut().enumerate() {
                let m = game.part_transform(i);
                if m != *was {
                    *was = m;
                    n += 1;
                }
            }
            changes.push(n);
        }
        changes.sort_unstable();
        println!(
            "parts that move per frame: median {}, p90 {}, worst {} (of {})",
            changes[changes.len() / 2],
            changes[changes.len() * 9 / 10],
            changes[changes.len() - 1],
            last.len()
        );
    }

    let total: u64 = us.iter().map(|&v| u64::from(v)).sum();
    let mut sorted = us.clone();
    sorted.sort_unstable();
    let pct = |p: f64| sorted[((sorted.len() - 1) as f64 * p) as usize];

    println!("{steps} steps of a game in progress");
    println!("  mean     {:8.1} us", total as f64 / steps as f64);
    println!("  median   {:8} us", pct(0.5));
    println!("  p99      {:8} us", pct(0.99));
    println!("  p99.9    {:8} us", pct(0.999));
    println!("  worst    {:8} us", sorted[sorted.len() - 1]);
    println!(
        "  over 1 ms: {} steps ({:.3}%)",
        us.iter().filter(|&&v| v > 1000).count(),
        us.iter().filter(|&&v| v > 1000).count() as f64 * 100.0 / steps as f64
    );

    // The worst ones, and how far apart they are: a stall on a fixed beat is
    // somebody's timer, and the beat is what names it.
    let mut worst: Vec<(usize, u32)> = us.iter().copied().enumerate().collect();
    worst.sort_by_key(|&(_, v)| std::cmp::Reverse(v));
    worst.truncate(20);
    worst.sort_by_key(|&(i, _)| i);
    println!();
    println!("the twenty slowest steps:");
    let mut last = None;
    for (i, v) in worst {
        match last {
            Some(p) => println!("  step {i:7}  {v:7} us   ({} ms after the last)", i - p),
            None => println!("  step {i:7}  {v:7} us"),
        }
        last = Some(i);
    }
}
