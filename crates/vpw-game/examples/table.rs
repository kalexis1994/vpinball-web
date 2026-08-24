//! Runs a real table with its ROM, headless, and says what the machine is doing.
//!
//!     cargo run --release -p vpw-game --example table -- table.vpx roms/ [seconds]
//!
//! The browser is a bad place to find out why a table is quiet: there is no
//! way to look inside it. This drives the same code the browser drives — the
//! script, the physics, the board, the frame timers — and then prints the two
//! things that say what state the machine is in: what the display says, and
//! which switches the table is reporting.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

const SCRIPTS: &str = "../../../vpinball/scripts";

/// Two channels of sixteen-bit samples, in the only container everything reads.
fn write_wav(path: &str, samples: &[u8], rate: u32) {
    let mut out = Vec::with_capacity(samples.len() + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    let _ = std::fs::write(path, out);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: table <table.vpx> <roms/> [seconds]");
    let roms = args
        .next()
        .expect("usage: table <table.vpx> <roms/> [seconds]");
    let seconds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);

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
    println!("game name       {:?}", game.machine().game_name());
    println!();

    // Coin, then start, the way a player does: hold the key for a moment and
    // let it go. Both go through the script, not the board, because that is
    // the path a browser takes.
    let press = |game: &mut Game, code: &str, at: u32, t: u32| {
        if t == at {
            game.key(code, true);
        }
        if t == at + 60 {
            game.key(code, false);
        }
    };

    // What the board drives, watched rather than taken: `changed_lamps` and
    // its friends are consumed by whoever asks first, and the table's own
    // script is asking. A probe that drains them is a probe that stops the
    // table working while it looks at it.
    let mut lamps_ever = [false; 128];
    let mut solenoids_ever = 0u32;
    let mut lit_now = 0usize;
    // And what the *table* has: the board's lamps only matter once the script
    // has copied them onto the pieces the renderer draws, and that is a
    // different set of numbers from the board's.
    let mut table_lit_ever = 0usize;
    let mut table_lit_now = 0usize;
    let mut table_lights = 0usize;
    let mut ever: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut heard = 0usize;
    let mut loud = 0usize;
    let mut peak = 0.0f32;
    let mut energy = 0.0f64;
    let mut wav: Vec<u8> = Vec::new();
    let mut silent_runs = Vec::new();
    let mut run_of_silence = 0usize;
    for t in 0..seconds * 1000 {
        // Two seconds to settle, a coin, then start.
        press(&mut game, "Digit5", 2_000, t);
        press(&mut game, "Digit1", 4_000, t);
        game.step();
        if t % 17 == 16 {
            game.game_sync();
            game.new_frame();
        }
        if t % 16 == 0 {
            let m = game.machine();
            solenoids_ever |= m.solenoids_active();
            lit_now = 0;
            for n in 1..=128u8 {
                if m.lamp_lit(n) {
                    lamps_ever[usize::from(n) - 1] = true;
                    lit_now += 1;
                }
            }
        }
        if t % 16 == 0 {
            table_lit_now = 0;
            table_lights = 0;
            for item in game.items().iter() {
                if item.kind != vpw_game::items::Kind::Light {
                    continue;
                }
                table_lights += 1;
                if item.light_level() > 0.0 {
                    table_lit_now += 1;
                    ever.insert(item.name.to_string());
                }
            }
            table_lit_ever = ever.len();
        }
        // The mixer is where the board's audio ends up; ask it for the same
        // millisecond of sound a host would have asked for.
        let mut out = [0.0f32; 96];
        game.render_audio(&mut out);
        heard += out.len() / 2;
        for s in out {
            peak = peak.max(s.abs());
            energy += f64::from(s) * f64::from(s);
            wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let quiet = out.iter().all(|s| s.abs() < 1.0e-4);
        if quiet {
            run_of_silence += 1;
        } else {
            loud += 1;
            if run_of_silence > 0 {
                silent_runs.push(run_of_silence);
                run_of_silence = 0;
            }
        }
    }

    println!("after {seconds}s of table time:");
    println!("  handlers fired  {}", game.handlers_fired());
    println!("  switches closed {:016x}", game.machine().switch_matrix());
    let closed: Vec<u8> = (1..=64u8)
        .filter(|&n| game.machine().switch_closed(n))
        .collect();
    println!("  which are       {closed:?}");
    println!(
        "  lamps ever lit  {} of 128, {lit_now} lit at the end",
        lamps_ever.iter().filter(|&&b| b).count()
    );
    println!(
        "  table lights    {table_lit_ever} of {table_lights} ever on, {table_lit_now} on at the end"
    );
    println!("  solenoids ever  {solenoids_ever:032b}");
    println!(
        "  solenoids now   {:032b}",
        game.machine().solenoids_active()
    );
    println!("  sound latch     {:02x}", game.machine().sound_latch());
    println!(
        "  audio           {loud} of {} ms had sound in them",
        seconds * 1000
    );
    silent_runs.sort_unstable();
    if let Some(longest) = silent_runs.last() {
        println!("  longest gap     {longest} ms");
    }
    println!("  frames rendered {heard} sample frames asked for");
    println!(
        "  loudest sample  {peak:.4} of 1.0, rms {:.4}",
        (energy / (heard.max(1) * 2) as f64).sqrt()
    );
    write_wav("table.wav", &wav, vpw_game::AUDIO_RATE);
    println!("  wrote table.wav");

    // What the game has lit, by name, for a renderer to photograph. Numbers
    // about lamps have been agreeing with each other all day while the picture
    // stayed dark, so the picture is the thing to look at.
    if let Ok(path) = std::env::var("VPW_DUMP_LIGHTS") {
        let mut out = String::new();
        for item in game.items().iter() {
            if item.kind == vpw_game::items::Kind::Light {
                out.push_str(&format!("{}\t{:.4}\n", item.name, item.light_level()));
            }
        }
        std::fs::write(&path, out).expect("could not write the light dump");
        println!("  wrote {path}");
    }

    let (dots, w, h) = game.machine().dmd();
    if !dots.is_empty() {
        println!("\nthe display says:");
        for y in 0..h {
            let row: String = (0..w)
                .map(|x| match dots[y * w + x] {
                    0 => ' ',
                    1 => '.',
                    2 => '+',
                    _ => '#',
                })
                .collect();
            println!("  |{}|", row.trim_end());
        }
    }
}
