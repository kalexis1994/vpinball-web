//! The score, from the board to the glass.
//!
//! Nothing in the player draws a display. A table carries a light for every
//! segment of every digit — F-14 has three hundred and twenty-two of them,
//! named `a00` through `ad9` for the alphanumeric row and `LED150` upwards for
//! the numeric one — and a handler that turns them on and off. All the machine
//! has to do is answer `Controller.ChangedLEDs`, and the score appears where
//! the table's author drew it, in the table's own artwork.
//!
//! Two things had to be right for any of it to show, and both were wrong.
//!
//! The numbering is not the hardware's. The board multiplexes sixteen digit
//! slots and a System 11A fills fourteen of them, in two groups of seven with a
//! gap; what a table sees is those packed with the gaps closed, top row then
//! bottom row, nought upwards. The original does that packing where it draws
//! (`core.c:1496`) and `ChangedLEDs` reports positions in the packed array.
//!
//! And F-14 hangs its whole display on `Table1.ShowDT`, which we answered as a
//! global and not as a member of the table. Read as `Empty` it is neither true
//! nor false, the branch never runs, and the table plays with a blank backbox.

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

fn loaded() -> Option<Game> {
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
    Some(game)
}

/// Every light the table uses as a display segment.
fn segment_lights(game: &Game) -> Vec<Rc<str>> {
    game.items()
        .iter()
        .filter(|item| matches!(item.kind, Kind::Light))
        .filter(|item| item.name.starts_with('a') || item.name.starts_with("LED"))
        .map(|item| item.name.clone())
        .collect()
}

fn lit(game: &Game) -> usize {
    game.items()
        .iter()
        .filter(|item| matches!(item.kind, Kind::Light))
        .filter(|item| item.name.starts_with('a') || item.name.starts_with("LED"))
        .filter(|item| item.light_level() > 0.0)
        .count()
}

/// The whole point: a score on the board becomes a score on the table.
#[test]
fn the_score_reaches_the_tables_own_lights() {
    let Some(mut game) = loaded() else { return };
    assert!(
        segment_lights(&game).len() > 300,
        "F-14 carries a light per segment and there should be hundreds"
    );

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

    // Watched over a while rather than sampled: F-14 flashes the score at the
    // start of a ball, so any one instant can legitimately be dark.
    let mut most = 0;
    for _ in 0..80 {
        for _ in 0..100 {
            game.step();
        }
        most = most.max(lit(&game));
    }
    assert!(
        most > 15,
        "the display never lit more than {most} segments; the score is not getting through"
    );
}

/// And it says what the machine says.
///
/// The board reports `bALL 1` on the bottom row, which is six characters of
/// seven-segment. Sixteen or so lights is what that costs, and finding fewer
/// than the top row alone needs would mean only half the display is wired.
#[test]
fn both_rows_of_the_display_draw() {
    let Some(mut game) = loaded() else { return };
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

    let mut seen_alphanumeric = false;
    let mut seen_numeric = false;
    for _ in 0..80 {
        for _ in 0..100 {
            game.step();
        }
        for item in game.items().iter() {
            if item.light_level() <= 0.0 || !matches!(item.kind, Kind::Light) {
                continue;
            }
            // The top row's segments are named `a` and a hex pair; the bottom
            // row's are `LED` and three digits.
            if item.name.starts_with("LED") {
                seen_numeric = true;
            } else if item.name.starts_with('a') && item.name.len() == 3 {
                seen_alphanumeric = true;
            }
        }
    }
    assert!(seen_alphanumeric, "the top row never lit a segment");
    assert!(seen_numeric, "the bottom row never lit a segment");
}

/// `ChangedLEDs` reports changes, not state.
///
/// Asking twice in a row has to give nothing the second time, or a table
/// redraws the whole display on every one of its twenty-five polls a second.
#[test]
fn asking_twice_reports_nothing_the_second_time() {
    let Some(mut game) = loaded() else { return };
    for _ in 0..4000 {
        game.step();
    }

    // The table polls this itself, so the first answer here is only whatever
    // has changed since its last poll. Take it and ask again straight away.
    let _ = game.machine().changed_leds(!0, 0);
    let again = game.machine().changed_leds(!0, 0);
    assert!(
        again.is_empty(),
        "nothing ran in between and it reported {} changes",
        again.len()
    );
}

/// The mask is honoured, and it is a mask over the packed numbering.
#[test]
fn the_mask_selects_which_digits_are_reported() {
    let Some(mut game) = loaded() else { return };
    for _ in 0..4000 {
        game.step();
    }
    let _ = game.machine().changed_leds(!0, 0);

    // Run until something on the top row moves, asking only about the top row.
    let mut top = Vec::new();
    for _ in 0..4000 {
        game.step();
        top = game.machine().changed_leds(0x3FFF, 0);
        if !top.is_empty() {
            break;
        }
    }
    assert!(!top.is_empty(), "the top row never changed in four seconds");
    assert!(
        top.iter().all(|&(digit, _, _)| digit < 14),
        "a masked-out digit was reported: {top:?}"
    );
}

/// The table has to be able to answer the questions that are also globals.
///
/// This is the one that kept the display dark. `Table1.ShowDT` is how F-14
/// decides whether to draw anything for a player at a desk, and everything
/// behind it — the score, the flippers' status report, the desktop backdrop —
/// is invisible if it reads as `Empty`.
#[test]
fn the_table_answers_showdt() {
    use vpw_vbscript::Value;
    let Some(game) = loaded() else { return };
    let shown = game
        .script()
        .get_global("DesktopMode")
        .expect("the script sets DesktopMode from Table1.ShowDT");
    assert!(
        matches!(shown, Value::Bool(true)),
        "DesktopMode came out as {shown:?}, so the table never drew its desktop half"
    );
}
