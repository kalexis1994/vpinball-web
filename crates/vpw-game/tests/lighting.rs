//! The playfield's lighting, on a real table with a real ROM.
//!
//! Everything here is about one failure that took a long time to find: the
//! whole playfield blinked, several times a second, and it sounded a relay
//! every time it did. It was not the ROM, and it was not the table's script.
//! It was the two places where the emulation turns something continuous into a
//! yes or a no — the lamp matrix and the solenoid drivers — doing it by
//! sampling rather than by asking the thing that is actually lit.
//!
//! These are slow tests: each one loads F-14, boots the ROM through its power-on
//! self test, puts a coin in and starts a game, which is a couple of million
//! instructions. They are worth it. Nothing smaller reproduces this, because it
//! only appears when a real ROM's real timing meets a real table's script.

use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::items::Kind;
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

/// The table, or nothing if it is not here.
///
/// It is not in the repository — a real one is over a hundred megabytes, and it
/// is somebody else's work — so every test that needs it says so and steps
/// aside. Panicking instead turns "you have not put a table in
/// `web/debug-assets/`" into a red build on every machine but one, which is
/// what it did until a CI runner tried it.
fn table_bytes() -> Option<Vec<u8>> {
    const PATH: &str = "../../web/debug-assets/f14.vpx";
    match std::fs::read(PATH) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipped: {PATH} is not there");
            None
        }
    }
}

/// F-14, with a game in progress and a ball waiting to be plunged.
fn playing() -> Option<Game> {
    let bytes = table_bytes()?;
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(
        std::path::Path::new("../../../vpinball/scripts").into(),
    ));
    let roms: Rc<dyn RomSource> = Rc::new(RomDir("../../web/debug-assets/roms".into()));
    let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries).with_roms(roms))
        .expect("the table should load");
    game.start().expect("the script should start");

    // Let the board finish booting, then a coin and the start button. The holds
    // are long enough for the ROM to see the switch close on one of its own
    // sweeps, and the waits long enough for it to act on it.
    run(&mut game, 4000);
    for (key, hold, after) in [("Digit5", 60, 2500), ("Digit1", 60, 3000)] {
        game.key(key, true);
        run(&mut game, hold);
        game.key(key, false);
        run(&mut game, after);
    }
    Some(game)
}

/// A frame's worth of steps, at sixty frames a second against a kilohertz
/// physics loop.
const FRAME: usize = 16;

/// The end of a rendered frame.
///
/// Two of the script's timer events belong to a frame and not to the clock:
/// `core.vbs` polls the ROM controller from `PinMameTimer`, whose
/// `TimerInterval` is `-2`, and Visual Pinball fires that once a frame from
/// `FireTimers(-2)`. A test that only calls `step` runs a table whose script
/// never talks to its board — the flippers move but nothing plays their sound,
/// because the callback that would is on the far side of that poll.
fn frame(game: &mut Game) {
    game.game_sync();
    game.new_frame();
}

/// One millisecond of table time, with a frame every sixteen of them.
fn tick(game: &mut Game, t: usize) {
    game.step();
    if t % FRAME == FRAME - 1 {
        frame(game);
    }
}

/// Runs `ms` milliseconds of table time, rendering as it goes.
fn run(game: &mut Game, ms: usize) {
    for t in 0..ms {
        tick(game, t);
    }
}

/// Which lights the script has on, by name.
fn lit(game: &Game) -> Vec<Rc<str>> {
    game.items()
        .iter()
        .filter(|item| matches!(item.kind, Kind::Light) && item.light_level() > 0.0)
        .map(|item| item.name.clone())
        .collect()
}

/// The general illumination is a string of bulbs that is either on or off for
/// a whole ball. It must not be seen to move.
///
/// On F-14 the table hangs it off solenoid 14, which is the A/C mux relay
/// (`s11games.c:372`). The ROM sets that relay once and leaves it — measured,
/// the drive register reads `0x2000` on every frame without exception. What
/// made it blink was our own accounting: the solenoid accumulator was cleared
/// at the end of each frame, so a frame containing no write to the latch saw no
/// coil, and the script was told the relay had dropped out. The original
/// restarts the accumulator from what is *held* rather than from nothing
/// (`s11.c:219`).
#[test]
fn the_general_illumination_holds_still() {
    let Some(mut game) = playing() else { return };
    let mut changes = 0;
    let mut previous: Option<Vec<Rc<str>>> = None;

    for _ in 0..120 {
        for _ in 0..FRAME {
            game.step();
        }
        frame(&mut game);
        let now = lit(&game);
        if let Some(before) = &previous {
            let moved = now
                .iter()
                .filter(|name| name.starts_with("GI"))
                .count()
                .abs_diff(before.iter().filter(|name| name.starts_with("GI")).count());
            changes += moved;
        }
        previous = Some(now);
    }

    assert_eq!(
        changes, 0,
        "the general illumination changed {changes} times over two seconds"
    );
}

/// And the lamps that the ROM *is* flashing keep flashing.
///
/// The failure and its fix are both about making things hold still, and the way
/// to get that wrong is to make everything hold still. F-14 chases its inserts
/// during a game; if this stops finding movement, the lamps have been smoothed
/// into a photograph.
#[test]
fn the_inserts_still_flash() {
    let Some(mut game) = playing() else { return };
    let mut moved = 0;
    let mut previous: Option<Vec<Rc<str>>> = None;

    for _ in 0..120 {
        for _ in 0..FRAME {
            game.step();
        }
        frame(&mut game);
        let now = lit(&game);
        if previous.as_ref().is_some_and(|before| &now != before) {
            moved += 1;
        }
        previous = Some(now);
    }

    assert!(
        moved > 4,
        "the playfield only changed {moved} times in two seconds, which is not a pinball"
    );
}

/// No lamp should be seen switching every single frame.
///
/// A ROM flashes a lamp at a few hertz. Anything at frame rate is the emulation
/// beating against the strobe rather than the game doing something, which is
/// what the filament model is there to prevent.
#[test]
fn nothing_blinks_at_frame_rate() {
    let Some(mut game) = playing() else { return };
    let mut history: std::collections::HashMap<Rc<str>, Vec<bool>> =
        std::collections::HashMap::new();

    for _ in 0..60 {
        for _ in 0..FRAME {
            game.step();
        }
        frame(&mut game);
        let on = lit(&game);
        for item in game.items().iter() {
            if matches!(item.kind, Kind::Light) {
                history
                    .entry(item.name.clone())
                    .or_default()
                    .push(on.contains(&item.name));
            }
        }
    }

    for (name, frames) in &history {
        let flips = frames.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            flips < frames.len() / 2,
            "{name} changed on {flips} of {} frames, which is a strobe rather than a flash",
            frames.len()
        );
    }
}

#[test]
fn pressing_a_flipper_button_sounds_the_flipper() {
    // A System 11 does not drive its flippers from the CPU: they are wired
    // straight from the cabinet buttons through the flipper relay, so the ROM
    // never sees them. PinMAME invents four solenoids for them anyway, from the
    // buttons, for exactly the games whose flippers the CPU does not drive
    // (`core.c:1746-1753`) — and a table's script is written entirely in terms
    // of those:
    //
    //     Const sLRFlipper = 46          ' core.vbs:2849
    //     SolCallback(sLLFlipper) = "SolLFlipper"
    //
    // `SolLFlipper` is what plays `fx_Flipperup` and calls
    // `LeftFlipper.RotateToEnd`. Without the invented solenoids the callback
    // never runs: the flippers move, because the port drives them from the key
    // as well, and they do it in silence.
    let Some(mut game) = playing() else { return };
    game.take_sounds();

    for (key, up) in [("KeyZ", "left"), ("KeyM", "right")] {
        game.key(key, true);
        let mut heard = false;
        for t in 0..400 {
            tick(&mut game, t);
            if game
                .take_sounds()
                .iter()
                .any(|s| s.to_ascii_lowercase().contains("flipperup"))
            {
                heard = true;
                break;
            }
        }
        assert!(heard, "the {up} flipper went up without a sound");

        game.key(key, false);
        let mut down = false;
        for t in 0..400 {
            tick(&mut game, t);
            if game
                .take_sounds()
                .iter()
                .any(|s| s.to_ascii_lowercase().contains("flipperdown"))
            {
                down = true;
                break;
            }
        }
        assert!(down, "the {up} flipper came back without a sound");
    }
}

#[test]
fn the_score_row_says_ball_the_only_way_seven_segments_can() {
    // A seven-segment digit cannot show an uppercase `B`: it would need the top
    // bar and the upper right as well, and with all seven lit the digit is an
    // `8`. So every character generator ever written draws it as a lowercase
    // `b`, and F-14's ROM is no exception — it writes 0x7C, which is the tall
    // left side, the bottom, the lower right and the middle bar.
    //
    // Worth pinning down because it looks like a bug and is not. There is no
    // font anywhere between the ROM and the glass: the display lights the
    // strokes it is sent, and a machine on the floor says "bALL 1" too.
    let Some(mut game) = playing() else { return };

    for t in 0..40000 {
        tick(&mut game, t);
        let m = game.machine();
        let (_, lower) = m.displays();
        if !lower.contains("ALL") {
            continue;
        }

        assert!(
            lower.contains("bALL"),
            "the score row should read it the only way it can: {lower:?}"
        );

        // And the bits behind it, so this fails if the reading of them ever
        // drifts rather than only if the text does.
        let segments = m.segments();
        let lower_row = &segments[segments.len() / 2..];
        let b = lower_row
            .iter()
            .position(|&s| s != 0)
            .map(|i| lower_row[i])
            .expect("something is lit");
        assert_eq!(
            b, 0x007C,
            "the ROM writes a lowercase b: bottom, both left strokes, lower              right and the middle bar"
        );
        return;
    }
    panic!("forty seconds in, the machine never announced a ball");
}
