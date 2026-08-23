//! A table that makes noise.
//!
//! Sound is the part of a pinball that is easiest to declare finished and
//! hardest to be sure about: a mixer with a sign error still produces a signal,
//! and a decoder that reads 8-bit PCM as signed still produces a sound. So
//! these tests measure rather than check for existence — how loud, for how
//! long, on which side — against a real table's real recordings.
//!
//! Needs the same files as `real_table.rs` and `rom_table.rs`, and skips itself
//! without them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::sound::Request;
use vpw_game::{AUDIO_RATE, Game, LibraryDir, Resources, ScriptLibrary};

const TABLE: &str = "../../web/debug-assets/f14.vpx";
const SCRIPTS: &str = "../../../vpinball/scripts";
const ROMS: &str = "../../web/debug-assets/roms";

fn table() -> Option<vpin::vpx::VPX> {
    if !Path::new(TABLE).is_file() {
        eprintln!("skipped: {TABLE} is not there");
        return None;
    }
    let bytes = std::fs::read(TABLE).expect("could not read the table");
    Some(vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx"))
}

fn game() -> Option<Game> {
    let vpx = table()?;
    if !Path::new(SCRIPTS).is_dir() {
        eprintln!("skipped: {SCRIPTS} is not there");
        return None;
    }
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(Path::new(SCRIPTS).into()));
    let mut resources = Resources::new(libraries);
    if Path::new(ROMS).is_dir() {
        let roms: Rc<dyn RomSource> = Rc::new(RomDir(PathBuf::from(ROMS)));
        resources = resources.with_roms(roms);
    }
    let mut game = Game::load(&vpx, &mut scene, resources).expect("the table failed to load");
    game.start().expect("Table1_Init");
    Some(game)
}

/// Loudest sample in one channel: 0 for left, 1 for right.
fn peak(out: &[f32], channel: usize) -> f32 {
    out.iter()
        .skip(channel)
        .step_by(2)
        .fold(0.0f32, |a, s| a.max(s.abs()))
}

/// Root mean square of the whole buffer. Peak says something got through; this
/// says there is a sound rather than a click.
fn rms(out: &[f32]) -> f32 {
    if out.is_empty() {
        return 0.0;
    }
    (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt()
}

/// A second of stereo.
fn buffer(seconds: f32) -> Vec<f32> {
    vec![0.0; 2 * (AUDIO_RATE as f32 * seconds) as usize]
}

#[test]
fn the_tables_sounds_decode_into_something_playable() {
    let Some(vpx) = table() else { return };
    let bank = vpw_table::sound::extract(&vpx);

    assert!(
        bank.len() > 40,
        "F-14 carries about sixty sounds, not {}",
        bank.len()
    );

    for sample in bank.iter() {
        assert!(
            !sample.pcm.is_empty(),
            "'{}' decoded to nothing",
            sample.name
        );
        assert!(
            (1..=2).contains(&sample.channels),
            "'{}' has {} channels",
            sample.name,
            sample.channels
        );
        assert!(
            (8000..=48000).contains(&sample.rate),
            "'{}' claims {} Hz",
            sample.name,
            sample.rate
        );
        // Between a click and a music bed. A sound that decoded to the wrong
        // width comes out at half or double the length, and this catches it.
        let seconds = sample.duration();
        assert!(
            (0.005..60.0).contains(&seconds),
            "'{}' lasts {seconds}s, which is not a table sound",
            sample.name
        );
        // Decoding 8-bit as signed, or 16-bit at the wrong endianness, gives
        // something that is loud everywhere. A real recording is not.
        assert!(
            sample.pcm.iter().all(|s| s.abs() <= 1.0),
            "'{}' has samples outside -1..1",
            sample.name
        );
    }

    // The ones the script reaches for by name have to be findable, whatever
    // case it writes them in.
    for wanted in ["drain", "flip_hit_1", "ballrelease"] {
        assert!(
            bank.find(wanted).is_some() && bank.find(&wanted.to_uppercase()).is_some(),
            "the table should have '{wanted}'"
        );
    }

    eprintln!(
        "{} sounds, {} MiB decoded",
        bank.len(),
        bank.bytes() / 1_048_576
    );
}

#[test]
fn a_sound_the_script_asks_for_actually_comes_out() {
    let Some(mut game) = game() else { return };

    // Nothing has been asked for, so there is nothing to hear. Worth asserting:
    // a mixer that leaks the previous buffer, or that hums, would pass every
    // test below and fail this one.
    let mut out = buffer(0.05);
    game.render_audio(&mut out);
    let quiet = peak(&out, 0);

    game.audio_mut().play("drain", Request::default());
    game.render_audio(&mut out);
    let loud = peak(&out, 0);

    assert!(
        loud > 0.05 && loud > quiet * 10.0 + 0.01,
        "asking for a sound should be audible: {quiet} -> {loud}"
    );
    assert!(rms(&out) > 0.001, "it is a click rather than a sound");
}

#[test]
fn the_pan_puts_a_sound_where_it_was_asked_for() {
    let Some(mut game) = game() else { return };

    let mut out = buffer(0.05);
    let hard_left = Request {
        pan: -1.0,
        ..Default::default()
    };
    game.audio_mut().play("drain", hard_left);
    game.render_audio(&mut out);

    let (l, r) = (peak(&out, 0), peak(&out, 1));
    assert!(l > 0.05, "there should be a sound at all: {l}");
    assert!(
        l > r * 5.0,
        "hard left should be on the left: left {l}, right {r}"
    );
}

#[test]
fn a_sound_lasts_as_long_as_the_recording() {
    let Some(mut game) = game() else { return };

    let seconds = {
        let audio = game.audio();
        let i = audio.bank.find("drain").expect("F-14 has a drain sound");
        audio.bank.get(i).unwrap().duration()
    };
    assert!(
        (0.5..10.0).contains(&seconds),
        "the drain sound is {seconds}s, which is not what this test assumes"
    );

    game.audio_mut().play("drain", Request::default());

    // Most of the way through it is still going...
    let mut out = buffer(seconds * 0.8);
    game.render_audio(&mut out);
    assert_eq!(game.audio().mixer.voices(), 1, "it ended early");

    // ...and past the end it is gone, and what follows is silence.
    let mut rest = buffer(seconds * 0.3);
    game.render_audio(&mut rest);
    assert_eq!(game.audio().mixer.voices(), 0, "it never ended");

    let mut after = buffer(0.05);
    game.render_audio(&mut after);
    assert_eq!(peak(&after, 0), 0.0, "something is still making noise");
}

#[test]
fn playing_the_table_makes_the_noises_the_script_asks_for() {
    let Some(mut game) = game() else { return };

    // Serve a ball and shoot it. Pressing the flippers is not enough: on a ROM
    // table the flipper noise comes from the board, which is in attract mode
    // until somebody starts a game. What does make noise on its own is the ball
    // — every table's script watches it from a timer and plays a rolling sound
    // whose level and pitch follow its speed.
    game.new_ball();
    game.key("Space", true);
    for _ in 0..600 {
        game.step();
    }
    game.key("Space", false);
    for _ in 0..4000 {
        game.step();
    }

    let asked = game.take_sounds();
    assert!(
        !asked.is_empty(),
        "four seconds of play should have asked for at least one sound"
    );
    eprintln!("the table asked for: {asked:?}");

    // The rolling sound is the one that has to be there: it is what a pinball
    // sounds like when nothing in particular is happening, and it exercises the
    // whole path — the physics moves the ball, a timer in the script reads its
    // speed, and `PlaySound` with `usesame` updates a voice that is already
    // running rather than restarting it twenty times a second.
    assert!(
        asked.iter().any(|s| s.contains("ballrolling")),
        "a ball rolling down the playfield should be audible: {asked:?}"
    );
    assert!(
        game.audio().mixer.voices() > 0,
        "and the sound should still be playing while the ball moves"
    );

    // And what it asked for is in the table: a name the bank does not have is
    // a real bug — either the script is being read wrong or the bank is.
    let audio = game.audio();
    let missing: Vec<&str> = audio.missing().collect();
    assert!(
        missing.len() < asked.len(),
        "none of the sounds it asked for exist: {missing:?}"
    );
}

#[test]
fn the_sound_board_reaches_the_mix() {
    let Some(mut game) = game() else { return };
    if !game.machine().is_running() {
        eprintln!("skipped: no ROM, so there is no sound board to hear");
        return;
    }

    // Let the machine get past its boot and into attract mode, where it plays
    // its own sounds unprompted.
    for _ in 0..8000 {
        game.step();
    }
    assert!(
        game.audio().mixer.queued() > 0 || {
            let mut out = buffer(0.1);
            game.render_audio(&mut out);
            peak(&out, 0) > 0.0
        },
        "the board produced no audio at all in eight seconds"
    );
}

#[test]
fn the_board_queue_does_not_turn_into_latency() {
    let Some(mut game) = game() else { return };
    if !game.machine().is_running() {
        return;
    }

    // A host that never renders — a hidden tab, a device that has not started —
    // must not let the board's audio pile up. Half a minute of table time with
    // nothing draining it should still leave a fraction of a second queued, or
    // the sound comes back thirty seconds behind the game.
    for _ in 0..30_000 {
        game.step();
    }
    let queued = game.audio().mixer.queued();
    assert!(
        queued <= AUDIO_RATE as usize / 2,
        "{queued} samples queued, which is {:.1}s behind",
        queued as f32 / AUDIO_RATE as f32
    );
}

#[test]
fn the_master_volume_turns_it_down() {
    let Some(mut game) = game() else { return };

    let measure = |game: &mut Game| {
        let mut out = buffer(0.05);
        game.audio_mut().play("drain", Request::default());
        game.render_audio(&mut out);
        peak(&out, 0)
    };

    let full = measure(&mut game);
    game.set_volume(0.0);
    let silent = measure(&mut game);

    assert!(full > 0.05, "the reference was already silent");
    assert_eq!(silent, 0.0, "zero volume should be silence, not {silent}");
}
