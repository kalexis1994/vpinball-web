//! Loads a table, plays it for a while, and says what happened.
//!
//! ```text
//! cargo run --release -p vpw-game --example play -- table.vpx [scripts-dir] [seconds] [out.wav] [roms-dir]
//! ```
//!
//! It is the diagnostic that answers "why does this table not do anything":
//! what the script said, which handlers actually fired, which members it asked
//! for that this port does not model.
//!
//! Given a fourth argument it also writes what the table *sounded* like to a
//! WAV. That is the only honest way to check audio: a mixer with a sign error
//! or a decoder reading 8-bit as signed still produces a signal, still passes
//! a test that measures its level, and is obviously wrong the moment you hear
//! it.

use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{AUDIO_RATE, Game, LibraryDir, NoLibraries, Resources, ScriptLibrary};

fn main() {
    env_logger();

    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: play <table.vpx> [scripts-dir] [seconds]");
    let scripts = args.next();
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let wav = args.next();
    let roms = args.next();

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = match &scripts {
        Some(dir) => Rc::new(LibraryDir(dir.into())),
        None => Rc::new(NoLibraries),
    };

    println!(
        "table    {}",
        vpx.info.table_name.clone().unwrap_or_default()
    );

    let mut resources = Resources::new(libraries);
    if let Some(dir) = &roms {
        let source: Rc<dyn RomSource> = Rc::new(RomDir(dir.into()));
        resources = resources.with_roms(source);
    }

    let mut game = match Game::load(&vpx, &mut scene, resources) {
        Ok(g) => g,
        Err(e) => {
            println!("LOAD FAILED: {e}");
            std::process::exit(1);
        }
    };
    println!("items    {}", game.items().len());
    println!("parts    {}", game.parts().len());

    match game.start() {
        Ok(()) => println!("init     ok"),
        Err(e) => println!("init     FAILED: {e}"),
    }
    for m in game.take_messages() {
        println!("  says: {m}");
    }

    println!("sounds   {} in the table", game.audio().bank.len());

    game.new_ball();
    // Shoot it, or the ball sits in the lane and the table has nothing to say.
    game.key("Space", true);

    // Audio is rendered in step with the physics rather than all at the end:
    // the mixer only advances when it is asked to, and a sound that was started
    // and finished between two renders would never be heard.
    let steps = (seconds * 1000.0) as u32;
    let mut recording: Vec<f32> = Vec::new();
    let chunk = AUDIO_RATE as usize / 100; // 10 ms
    let mut buffer = vec![0.0; chunk * 2];
    for i in 0..steps {
        if i == 600 {
            game.key("Space", false);
        }
        game.step();
        if wav.is_some() && i % 10 == 9 {
            game.render_audio(&mut buffer);
            recording.extend_from_slice(&buffer);
        }
    }

    if let Some(path) = &wav {
        match write_wav(path, &recording, AUDIO_RATE) {
            Ok(()) => println!(
                "wrote    {path} ({:.1}s, peak {:.2})",
                recording.len() as f32 / 2.0 / AUDIO_RATE as f32,
                recording.iter().fold(0.0f32, |a, s| a.max(s.abs()))
            ),
            Err(e) => println!("WAV FAILED: {e}"),
        }
    }

    println!();
    println!("after {seconds}s of table time:");
    println!("  balls in play  {}", game.engine.borrow().balls.len());
    let sounds = game.take_sounds();
    println!("  sounds asked   {}", sounds.len());
    for s in sounds.iter().take(6) {
        println!("    {s}");
    }
    for m in game.take_messages() {
        println!("  says: {m}");
    }

    // What the table asked for that this port does not model. It is the
    // shopping list for whoever works on the binding next.
    let mut wanted: Vec<String> = game.items().iter().flat_map(|i| i.unmodelled()).collect();
    wanted.sort();
    wanted.dedup();
    if !wanted.is_empty() {
        println!(
            "  members not modelled ({}): {}",
            wanted.len(),
            wanted.join(", ")
        );
    }
}

/// Logging without pulling in a logger crate: `log` needs somewhere to go, and
/// for a diagnostic tool that somewhere is stderr.
fn env_logger() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// Writes interleaved stereo `f32` as a 16-bit WAV.
///
/// Hand-rolled because it is forty lines and a dependency here would be one
/// more thing between a change and hearing it.
fn write_wav(path: &str, pcm: &[f32], rate: u32) -> std::io::Result<()> {
    use std::io::Write;

    let channels = 2u16;
    let bits = 16u16;
    let block_align = channels * bits / 8;
    let data_len = (pcm.len() * 2) as u32;

    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        // 32767 and not 32768: rounding a full-scale +1.0 up wraps to the most
        // negative sample, which is a click on the loudest peak of every sound.
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&out)
}
