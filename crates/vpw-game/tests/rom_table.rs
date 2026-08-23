//! A ROM table, running its own firmware.
//!
//! On F-14 the table's script holds no rules at all: it reports switches to the
//! board and copies back lamps and solenoids. So "the table loads" and "the
//! table plays" are two different claims, and only this test makes the second
//! one — the board has to boot, get past its self-test, and start driving the
//! playfield before anything on it means anything.
//!
//! It needs the ROM, which is copyrighted firmware and is not in the
//! repository. Put `f14_l1.zip` in `web/debug-assets/roms/` (already ignored by
//! git) or point `VPW_ROMS` at wherever you keep yours; without it the tests
//! skip themselves.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::controller::{Machine, RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

const TABLE: &str = "../../web/debug-assets/f14.vpx";
const SCRIPTS: &str = "../../../vpinball/scripts";
const ROMS: &str = "../../web/debug-assets/roms";

/// F-14's set, as its script spells it: `Const cGameName = "f14_l1"`.
const SET: &str = "f14_l1";

fn rom_dir() -> Option<PathBuf> {
    let dir = match std::env::var("VPW_ROMS") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from(ROMS),
    };
    if dir.join(format!("{SET}.zip")).is_file() {
        Some(dir)
    } else {
        eprintln!("skipped: no {SET}.zip in {}", dir.display());
        None
    }
}

/// Boots the board on its own, with no table around it.
///
/// Worth having separately from the full test below: if this passes and that
/// one fails, the emulator is fine and the bridge is not.
fn booted() -> Option<Machine> {
    let dir = rom_dir()?;
    let zip = std::fs::read(dir.join(format!("{SET}.zip"))).expect("the ROM will not open");
    let machine = Machine::new();
    machine
        .load(SET, &zip, None)
        .unwrap_or_else(|e| panic!("the ROM did not load: {e}"));
    Some(machine)
}

#[test]
fn the_rom_zip_decompresses_and_the_board_takes_it() {
    let Some(machine) = booted() else { return };
    assert!(machine.is_running());
    assert_eq!(machine.game_name(), Some(SET));
}

#[test]
fn the_board_drives_the_playfield_once_it_has_booted() {
    let Some(machine) = booted() else { return };

    // A System 11 spends its first moments on a memory test and only then
    // starts scanning the matrices. Two seconds is generous; the interesting
    // assertion is that it happens at all.
    for _ in 0..2000 {
        machine.advance(0.001);
    }

    // `ChangedLamps` is the whole contract: what the ROM has done to the lamps
    // since the script last looked. If this is empty the board is not running
    // the game — it is stuck in a reset loop, and every lamp on the table stays
    // dark no matter what the script does.
    let changed = machine.changed_lamps();
    assert!(
        !changed.is_empty(),
        "after two seconds the ROM should have driven at least one lamp"
    );

    // And reading clears the record: a table polls this sixty times a second
    // and expects each change once. Reporting a change twice is how a port ends
    // up flickering; never clearing is how lamps latch on forever.
    let again = machine.changed_lamps();
    assert!(
        again.len() < changed.len(),
        "reading ChangedLamps should have consumed the changes, not repeated {} of them",
        again.len()
    );
}

/// The same table as `real_table.rs`, but with its ROM.
fn load_with_rom() -> Option<Game> {
    let dir = rom_dir()?;
    if !Path::new(TABLE).is_file() {
        eprintln!("skipped: {TABLE} is not there");
        return None;
    }
    if !Path::new(SCRIPTS).is_dir() {
        // Without `core.vbs` the script never gets as far as `Controller.Run`,
        // so there is nothing for this test to look at.
        eprintln!("skipped: {SCRIPTS} is not there");
        return None;
    }

    let bytes = std::fs::read(TABLE).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(Path::new(SCRIPTS).into()));
    let roms: Rc<dyn RomSource> = Rc::new(RomDir(dir));
    let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries).with_roms(roms))
        .unwrap_or_else(|e| panic!("the table failed to load: {e}"));
    game.start().unwrap_or_else(|e| panic!("Table1_Init: {e}"));
    Some(game)
}

#[test]
fn the_table_starts_its_own_rom() {
    let Some(mut game) = load_with_rom() else {
        return;
    };

    // `Table1_Init` calls `LoadVPM`, which ends in `Controller.Run`. If the
    // board is running, the whole chain worked: the script found `core.vbs` and
    // `s11.vbs`, read `cGameName`, asked the host for a controller, and the
    // host found the ROM and built the machine.
    assert!(
        game.machine().is_running(),
        "the table's script should have started the ROM"
    );
    assert_eq!(game.machine().game_name(), Some(SET));

    // Five seconds of table time, which is also five seconds of board time.
    for _ in 0..5000 {
        game.step();
    }

    // The script polls the board from a timer, so by now it has seen lamps
    // change and pushed them onto the table. What is being tested is the round
    // trip: ROM to controller to script to playfield.
    assert!(
        game.handlers_fired() > 0,
        "the script's timers should have run"
    );

    let (upper, lower) = game.machine().displays();
    let shown = format!("{upper}{lower}");
    assert!(
        shown.chars().any(|c| c.is_ascii_digit()),
        "the score displays should show something, not {shown:?}"
    );
}

/// Presses a key for `down_ms` and then lets the table run for `after_ms`.
fn press(game: &mut Game, code: &str, down_ms: u32, after_ms: u32) {
    game.key(code, true);
    for _ in 0..down_ms {
        game.step();
    }
    game.key(code, false);
    for _ in 0..after_ms {
        game.step();
    }
}

#[test]
fn a_coin_buys_a_game_and_the_machine_serves_a_ball() {
    let Some(mut game) = load_with_rom() else {
        return;
    };

    // Settle into attract mode.
    for _ in 0..4000 {
        game.step();
    }

    // The trough has to report its balls before any of this means anything: a
    // machine that thinks it is empty will not start a game. It comes from
    // `bsTrough.Balls = 4` in the table's own `Table1_Init`, but the switches
    // are not written there and then — `core.vbs` marks the stack as needing an
    // update and pushes it on the next tick of its own timer, which is why this
    // is asserted after the table has run rather than before.
    let trough: Vec<u8> = (11..=14)
        .filter(|&n| game.machine().switch_closed(n))
        .collect();
    assert_eq!(
        trough,
        vec![11, 12, 13, 14],
        "the trough should be reporting four balls"
    );

    // The coin. The script does not report it straight away: `s11.vbs` queues
    // `vpmTimer.AddTimer 750, "vpmTimer.PulseSw swCoin1"`, because a real coin
    // takes a moment to fall past the switch. So the wait is not padding.
    let attract = game.machine().displays().0;
    press(&mut game, "Digit5", 60, 2500);
    let credited = game.machine().displays().0;
    assert!(
        credited.contains("CREDIT"),
        "a coin should have bought a credit; the display says {credited:?} \
         (it said {attract:?} before)"
    );

    // And the start button takes it: the machine announces the ball, fires the
    // coil under the trough, and a ball appears in the shooter lane. That last
    // one is the whole chain in one assertion — a keypress reached the script,
    // the script reached the board, the board's own program decided to start a
    // game, and the coil it fired came back through the script as a kick.
    assert!(
        !game.machine().solenoid_fired(2),
        "the trough is quiet so far"
    );
    press(&mut game, "Digit1", 60, 500);

    let announced = game.machine().displays().1;
    assert!(
        announced.contains("bALL"),
        "the machine should have announced a ball: {announced:?}"
    );
    assert_eq!(
        game.engine.borrow().balls.len(),
        1,
        "and put one on the playfield"
    );
    eprintln!("attract {attract:?} -> credited {credited:?} -> {announced:?}");

    // The ball stays: a machine that served one and then lost track of it goes
    // into a ball search and ends the game.
    for _ in 0..5000 {
        game.step();
    }
    assert!(
        game.machine().displays().1.contains("bALL"),
        "the game ended: {:?}",
        game.machine().displays()
    );
}

/// A full plunge puts the ball into play.
///
/// The shooter lane is not a flat corridor: `Ramp3` lifts the ball from the
/// playfield to 61 units over a wall that is 45 tall, and `Ramp5` carries it
/// round the top. Without collision on ramps the ball ran into that wall at
/// full speed and came straight back, and there was no way to get a ball into
/// play at all.
#[test]
fn a_full_plunge_carries_the_ball_up_the_lane() {
    let Some(mut game) = load_with_rom() else {
        return;
    };

    for _ in 0..4000 {
        game.step();
    }
    press(&mut game, "Digit5", 60, 2500);
    press(&mut game, "Digit1", 60, 3000);
    assert_eq!(
        game.engine.borrow().balls.len(),
        1,
        "the machine should have served a ball"
    );

    let start = game.engine.borrow().balls[0].pos;
    assert!(
        start.y > 1800.0,
        "it should be in the shooter lane, not at {start:?}"
    );

    // Hold the plunger back all the way, then let go.
    press(&mut game, "Space", 900, 0);

    let (mut highest, mut tallest) = (f32::MAX, f32::MIN);
    for _ in 0..1200 {
        game.step();
        let Some(ball) = game.engine.borrow().balls.first().map(|b| b.pos) else {
            break;
        };
        highest = highest.min(ball.y);
        tallest = tallest.max(ball.z);
    }

    // Up the ramp — which means off the playfield, in `z` — and past the wall
    // that used to stop it at y≈1264.
    assert!(
        tallest > 60.0,
        "the ball never got off the playfield: highest z was {tallest:.0}"
    );
    assert!(
        highest < 600.0,
        "the ball did not reach the top of the table: it got to y={highest:.0}"
    );
}

/// The kickback saucer catches the ball and the machine puts it into play.
///
/// The catch is the fiddly half: F-14's saucers are *legacy* kickers, and
/// a legacy kicker grabs whatever touches it however high the ball is riding
/// (`kicker.cpp:1128`). Ask the height question anyway and the ball rolls into
/// the hole, stops on top of it, and stays there — nothing catches it and
/// nothing tells the script it arrived.
///
/// The ball is dropped straight into the saucer rather than plunged and left to
/// find its way there. It used to be plunged, and that worked until the special
/// solenoids started working: F-14 holds a diverter across the habitrail for
/// the first five seconds of a ball, so the route the old version of this test
/// relied on is one the machine now closes on purpose. Dropping the ball in is
/// also a straighter question — the old version could fail for any of a dozen
/// reasons on the way, and only one of them was the one it was asking about.
#[test]
fn the_saucer_catches_the_ball_and_the_machine_puts_it_into_play() {
    let Some(mut game) = load_with_rom() else {
        return;
    };

    for _ in 0..4000 {
        game.step();
    }
    press(&mut game, "Digit5", 60, 2500);
    press(&mut game, "Digit1", 60, 3000);
    // Take the served ball out of the way: this test is about one saucer and
    // one ball, and a second ball rolling around the lane muddies both halves.
    game.engine.borrow_mut().balls.clear();

    let saucer = kicker_position(&game, "sw55").expect("F-14 has a saucer called sw55");
    {
        let mut engine = game.engine.borrow_mut();
        // A little above it and falling, which is how a ball arrives.
        let from = saucer + vpw_math::Vec3::new(0.0, 0.0, 30.0);
        let ball = engine.add_ball(vpw_physics::ball::Ball::new(from, 25.0));
        engine.balls[ball].vel = vpw_math::Vec3::new(0.0, -2.0, -5.0);
    }

    let mut caught_at = None;
    let mut fastest_after: f32 = 0.0;
    for t in 0..8000 {
        game.step();
        if caught_at.is_none() && game.take_sounds().iter().any(|s| s == "popper_ball") {
            caught_at = Some(t);
        }
        if caught_at.is_some()
            && let Some(ball) = game.engine.borrow().balls.first()
        {
            fastest_after = fastest_after.max(ball.vel.length());
        }
    }
    let caught = caught_at.expect("the saucer should have caught the ball");
    eprintln!("caught after {caught} ms, then left at {fastest_after:.1}");
    assert!(
        fastest_after > 15.0,
        "the machine should have ejected the ball; the fastest it moved was {fastest_after:.1}"
    );
}

/// Where a named kicker sits, from the physics rather than from the file.
fn kicker_position(game: &Game, name: &str) -> Option<vpw_math::Vec3> {
    let item = game.items().get(name)?;
    let engine = game.engine.borrow();
    item.shapes
        .iter()
        .find_map(|&s| match engine.shapes().get(s) {
            Some(vpw_physics::engine::Shape::Kicker(k)) => Some(vpw_math::Vec3::new(
                k.circle.center.x,
                k.circle.center.y,
                k.circle.z_low,
            )),
            _ => None,
        })
}

#[test]
fn the_rom_drives_all_sixty_four_lamps() {
    // A System 11 numbers its lamps straight through from 1 to 64
    // (`core_m2swSeq`, `core.c:2109`), not by column and row the way WPC does.
    // F-14's script agrees: it fades `l1` through `l64`.
    //
    // With WPC's numbering the last two columns came out as 71 to 88, numbers
    // the board does not answer for. They read dark forever, so they never
    // *changed*, so the script was never told about them: sixteen bulbs off for
    // the whole game, and the other forty-eight lighting under a number that
    // belongs to a different one.
    let Some(machine) = booted() else { return };

    let mut seen: std::collections::BTreeSet<u8> = Default::default();
    for _ in 0..20_000 {
        machine.advance(0.001);
        for (number, _) in machine.changed_lamps() {
            seen.insert(number);
        }
    }

    let missing: Vec<u8> = (1..=64u8).filter(|n| !seen.contains(n)).collect();
    assert!(
        missing.is_empty(),
        "twenty seconds in, the board never reported these lamps: {missing:?}"
    );
}
