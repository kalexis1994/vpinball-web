//! The three ways a System 11 drives a solenoid, on the real machine.
//!
//! Sixteen of them come from a latch and a PIA port, and those have worked for
//! a while. The other two ways are the subject here:
//!
//! - **Six special solenoids**, 17 to 22, driven off the PIAs' control lines or
//!   straight from a playfield switch. F-14 puts its two diverters on 21 and 22
//!   and its slingshots and pop bumper on switches.
//! - **Eight muxed solenoids**, 25 to 32, which are the *same eight drivers* as
//!   1 to 8 with a relay in between. While the relay is pulled in, everything
//!   the ROM writes to the first eight comes out of the second eight.
//!
//! All of it is per game and all of it comes from one line of the original's
//! game table (`s11games.c:372` for F-14). Getting it wrong is quiet: the coils
//! fire, they are just not the ones the ROM meant.

use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
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

/// F-14 with a coin in it and a game started.
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
    Some(game)
}

/// Runs the table and returns every distinct solenoid word it passed through.
fn watch(game: &mut Game, steps: usize) -> Vec<u32> {
    let mut seen = vec![game.machine().solenoids_active()];
    for _ in 0..steps {
        game.step();
        let now = game.machine().solenoids_active();
        if seen.last() != Some(&now) {
            seen.push(now);
        }
    }
    seen
}

/// Whether any of the states had a given solenoid on.
fn ever(states: &[u32], number: u8) -> bool {
    states.iter().any(|s| s & (1 << (number - 1)) != 0)
}

/// "Game on" is solenoid 23, and it says what it means.
///
/// It is gated by PIA 0's CB2, which is inverted — the pin low is the game on.
/// Reading it the other way round leaves the six special solenoids live in
/// attract mode and dead during play, which is exactly backwards.
#[test]
fn game_on_follows_the_game() {
    let Some(bytes) = table_bytes() else { return };
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(
        std::path::Path::new("../../../vpinball/scripts").into(),
    ));
    let roms: Rc<dyn RomSource> = Rc::new(RomDir("../../web/debug-assets/roms".into()));
    let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries).with_roms(roms))
        .expect("the table should load");
    game.start().expect("the script should start");

    // Attract mode: nobody has paid.
    let attract = watch(&mut game, 8000);
    assert!(
        !ever(&attract, 23),
        "the machine says a game is on and nobody has put a coin in"
    );
    assert!(
        !game.machine().special_enabled(),
        "and the special solenoids are dead with it"
    );

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

    let playing = watch(&mut game, 2000);
    assert!(ever(&playing, 23), "a game is on and the machine says so");
    assert!(game.machine().special_enabled());
}

/// F-14's diverters are special solenoids, and the ROM uses them.
///
/// Solenoids 21 and 22 have no switch wired to them, so the only way they can
/// ever fire is off PIA 1's control lines. The script hangs `SolDiverter1` and
/// `SolDiverter2` on them, and until the control lines were hooked up the two
/// diverters on this table did not exist.
#[test]
fn the_diverters_get_driven() {
    let Some(mut game) = playing() else { return };
    // Plunge, because the diverter is thrown for the start of a ball.
    game.key("Space", true);
    for _ in 0..900 {
        game.step();
    }
    game.key("Space", false);

    let states = watch(&mut game, 6000);
    assert!(
        ever(&states, 21) || ever(&states, 22),
        "neither diverter was ever driven in six seconds of play"
    );
}

/// And it lets go of them again.
///
/// A control line is only looked at when the ROM writes the control register,
/// so a bit that gets set and never cleared reads as a diverter held across the
/// habitrail for the whole game. It comes off after about five seconds.
#[test]
fn a_diverter_is_not_held_for_ever() {
    let Some(mut game) = playing() else { return };
    game.key("Space", true);
    for _ in 0..900 {
        game.step();
    }
    game.key("Space", false);

    let states = watch(&mut game, 8000);
    let both = 1 << 20 | 1 << 21;
    assert!(
        states.iter().any(|s| s & both != 0),
        "no diverter was thrown"
    );
    assert!(
        states.iter().any(|s| s & both == 0),
        "a diverter went on and stayed on"
    );
}

/// The mux relay, watched on the machine.
///
/// F-14's is solenoid 14, and it spends most of a game pulled in. What has to
/// hold is that the ROM never drives the A side while it is: those eight
/// drivers are shared, so if it did, the trough coil and a flasher would be the
/// same wire. Measured over a whole ball, it never does — which is also why not
/// routing the mux for so long never fired a coil that should not have fired,
/// and only lost the flashers.
#[test]
fn the_rom_never_drives_both_sides_of_the_mux_at_once() {
    let Some(mut game) = playing() else { return };
    game.key("Space", true);
    for _ in 0..900 {
        game.step();
    }
    game.key("Space", false);

    let mut muxed = 0;
    let mut total = 0;
    for _ in 0..10000 {
        game.step();
        let state = game.machine().solenoids_active();
        total += 1;
        if state & (1 << 13) != 0 {
            muxed += 1;
            assert_eq!(
                state & 0xFF,
                0,
                "the A side is driven with the relay in: {state:#010x}"
            );
        }
    }
    assert!(
        muxed > total / 4,
        "the relay should be in for a good part of a ball, and it was for {muxed} of {total}"
    );
}

/// The A-side coils still fire, which is the thing the mux could most easily
/// have broken.
///
/// Solenoid 1 is F-14's trough: it is what puts a ball in the shooter lane.
/// Route it to the wrong bank and the machine serves nothing and the game is
/// over before it starts.
#[test]
fn the_trough_still_serves_a_ball() {
    let Some(bytes) = table_bytes() else { return };
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(
        std::path::Path::new("../../../vpinball/scripts").into(),
    ));
    let roms: Rc<dyn RomSource> = Rc::new(RomDir("../../web/debug-assets/roms".into()));
    let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries).with_roms(roms))
        .expect("the table should load");
    game.start().expect("the script should start");

    for _ in 0..4000 {
        game.step();
    }
    // Watched across the coin and the start, because serving the ball is the
    // first thing the machine does and it is over long before a ball is in play.
    let mut states = Vec::new();
    for (key, hold, after) in [("Digit5", 60, 2500), ("Digit1", 60, 4000)] {
        game.key(key, true);
        states.extend(watch(&mut game, hold));
        game.key(key, false);
        states.extend(watch(&mut game, after));
    }

    assert!(
        ever(&states, 1) || ever(&states, 2),
        "the trough never fired, so no ball was served"
    );
}

/// Hitting a target lights a flasher, and a flasher is a muxed solenoid.
///
/// This is the payoff, and it is not cosmetic. F-14 hangs nine callbacks on
/// solenoids 25 to 32 and those are the same eight drivers as 1 to 8 — so
/// before the relay was modelled, every flasher the game lit was reported as
/// solenoid 1 or 2 instead. On this table solenoid 1 is `bsTrough.SolIn` and 2
/// is `bsTrough.SolOut`: the machine was being told to work the ball trough
/// every time it flashed a lamp.
///
/// Nothing on a rolling ball provokes it. The flashers come with scoring, so
/// the ball is thrown at each target in turn.
#[test]
fn a_flasher_comes_out_of_the_second_bank() {
    let Some(mut game) = playing() else { return };

    let Some(bytes) = table_bytes() else { return };
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let mut everything = 0u32;

    for item in vpx.gameitems.iter() {
        let vpin::vpx::gameitem::GameItemEnum::HitTarget(t) = item else {
            continue;
        };
        // From in front of it, which is the way a target can be hit.
        let angle = t.rot_z.to_radians();
        let target = vpw_math::Vec3::new(t.position.x, t.position.y, t.position.z);
        let from = vpw_math::Vec3::new(
            t.position.x - angle.sin() * 55.0,
            t.position.y + angle.cos() * 55.0,
            t.position.z + 25.0,
        );
        {
            let mut engine = game.engine.borrow_mut();
            engine.balls.clear();
            let ball = engine.add_ball(vpw_physics::ball::Ball::new(from, 25.0));
            engine.balls[ball].vel = (target - from).normalize() * 45.0;
        }
        // Only while the ball is still on its way to the target and just after.
        // Left running longer it eventually drains, and a drain drives the
        // trough coils for a perfectly good reason — which would make the
        // assertion below about the ball's route rather than about the mux.
        for _ in 0..250 {
            game.step();
            everything |= game.machine().solenoids_active();
        }
    }

    let flashers = everything & 0xFF00_0000;
    assert!(
        flashers != 0,
        "no flasher fired in a whole bank of targets: {everything:#034b}"
    );
    // And the A side stayed out of it, which is the half that matters: those
    // drivers belong to the trough while the relay is out.
    assert_eq!(
        everything & 0b11,
        0,
        "the trough coils were driven by a flasher: {everything:#034b}"
    );
}
