//! A System 80 machine, through the bridge the table's script sees.
//!
//! The board tests in `vpw-gts80` say each wire goes where the schematic says.
//! This says the game gets it: that a set nobody has listed is recognised by
//! its own shape, that the numbers in `sys80.vbs` reach the switches they name,
//! and — the part that decides whether the table is playable at all — that
//! putting a coin in and pressing start pulls the game-on relay in.
//!
//! Needs the ROM, which is copyrighted firmware and is not in the repository.
//! Put `icefever.zip` in `web/debug-assets/roms/` (already ignored by git) or
//! point `VPW_ROMS` at wherever you keep yours; without it these skip
//! themselves.

use std::path::PathBuf;

use vpw_game::controller::Machine;

/// Ice Fever, as its script spells it — `Const cGameName="icefever"`.
const SET: &str = "icefever";

/// The coin door, from `sys80.vbs:23`. Not a guess and not this port's
/// numbering: these are the constants every System 80 table is written
/// against.
const SW_TEST: u8 = 7;
const SW_COIN3: u8 = 37;
const SW_START: u8 = 47;

/// The outhole, from the table's own script: `bsTrough.InitSw 0,67,…`. A
/// machine that cannot find its ball will not start a game for anybody.
const SW_OUTHOLE: u8 = 67;

/// `Const GameOnSolenoid = 10` (`sys80.vbs:20`), which is the lamp latch's
/// game-on relay and the wire `vpmFlips` polls before it moves a flipper.
const GAME_ON: u8 = 10;

fn booted() -> Option<Machine> {
    let dir = match std::env::var("VPW_ROMS") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from("../../web/debug-assets/roms"),
    };
    let zip = dir.join(format!("{SET}.zip"));
    if !zip.is_file() {
        eprintln!("skipped: no {SET}.zip in {}", dir.display());
        return None;
    }
    let image = std::fs::read(&zip).expect("the ROM will not open");
    let machine = Machine::new();
    machine
        .load(SET, &image, None)
        .unwrap_or_else(|e| panic!("the ROM did not load: {e}"));
    Some(machine)
}

/// Runs the machine for a stretch, in the steps a player would.
fn run(machine: &Machine, seconds: f64) {
    let mut left = seconds;
    while left > 0.0 {
        machine.advance(0.01);
        left -= 0.01;
    }
}

#[test]
fn a_set_nobody_listed_is_recognised_by_its_shape() {
    // No table of games: a System 80 set is a 2 KB game ROM and two named
    // 4 KB system ROMs, and there are fifty-odd of them.
    let Some(machine) = booted() else { return };
    assert!(machine.is_running());
    assert_eq!(machine.game_name().as_deref(), Some(SET));
}

#[test]
fn the_board_runs_and_lights_its_display() {
    let Some(machine) = booted() else { return };
    // Gottlieb flashes its scores, so a single look can honestly find the
    // glass dark. Fifty of them over a second cannot.
    let mut lit = false;
    for _ in 0..50 {
        run(&machine, 0.02);
        lit |= machine.displays().0.chars().any(|c| c != ' ');
    }
    assert!(lit, "the attract mode should have written a digit");
}

/// The one that decides whether the table is playable: a coin and the start
/// button have to reach the ROM through the numbers the table uses.
#[test]
fn a_coin_and_the_start_button_begin_a_game() {
    let Some(machine) = booted() else { return };
    run(&machine, 2.0);
    assert!(!machine.solenoid_fired(GAME_ON), "no game before the coin");

    // A ball where the machine can find it.
    machine.set_switch(SW_OUTHOLE, true);
    run(&machine, 0.5);

    // A coin is a pulse, not a hold — `sys80.vbs` fires one through
    // `vpmTimer.PulseSw`. The third chute, because a board with nothing in
    // its memory yet gives a credit for one coin there and asks more of the
    // other two.
    for _ in 0..3 {
        machine.set_switch(SW_COIN3, true);
        run(&machine, 0.2);
        machine.set_switch(SW_COIN3, false);
        run(&machine, 0.4);
    }

    machine.set_switch(SW_START, true);
    run(&machine, 0.3);
    machine.set_switch(SW_START, false);

    // The relay comes in a few seconds later, once the machine has finished
    // resetting the playfield for a new game.
    let mut on = false;
    for _ in 0..100 {
        run(&machine, 0.1);
        on |= machine.solenoid_fired(GAME_ON);
    }
    assert!(
        on,
        "the game-on relay should be in, and without it no flipper moves"
    );
}

/// A System 80's flippers never go through the CPU: the cabinet buttons
/// drive the coils through the game-on relay. PinMAME invents solenoids 45
/// to 48 for them (`core.c:1747`) and a table written against `sys80.vbs`
/// moves and sounds its flippers from those alone — Spider-Man's flippers
/// were silent because this port did not.
#[test]
fn the_flipper_buttons_raise_the_invented_flipper_solenoids() {
    let Some(machine) = booted() else { return };
    // Buttons with the game off: nothing, the relay is out.
    machine.set_switch(114, true);
    run(&machine, 0.2);
    let _ = machine.changed_solenoids();
    assert!(!machine.solenoid_fired(48), "no game on, no flipper");
    machine.set_switch(114, false);

    // Start a game the same way as above.
    machine.set_switch(SW_OUTHOLE, true);
    run(&machine, 0.5);
    for _ in 0..3 {
        machine.set_switch(SW_COIN3, true);
        run(&machine, 0.2);
        machine.set_switch(SW_COIN3, false);
        run(&machine, 0.4);
    }
    machine.set_switch(SW_START, true);
    run(&machine, 0.3);
    machine.set_switch(SW_START, false);
    // The moment the relay comes in. With the outhole switch held closed the
    // machine drains ball after ball and the game does not last, so the
    // flipper is tried as soon as the relay is seen rather than later.
    let mut on = false;
    for _ in 0..100 {
        run(&machine, 0.1);
        if machine.solenoid_fired(GAME_ON) {
            on = true;
            break;
        }
    }
    assert!(on, "the game-on relay should be in");
    let _ = machine.changed_solenoids();

    // The left button: solenoids 47 and 48 come up, as a change the script
    // is told about, and go down with the button.
    machine.set_switch(114, true);
    run(&machine, 0.02);
    let changed = machine.changed_solenoids();
    assert!(changed.contains(&(47, true)) && changed.contains(&(48, true)), "{changed:?}");
    assert!(!changed.iter().any(|(n, _)| *n == 45 || *n == 46), "not the right one");
    machine.set_switch(114, false);
    run(&machine, 0.02);
    let changed = machine.changed_solenoids();
    assert!(changed.contains(&(48, false)), "{changed:?}");
}

/// The sound card is a second processor on a card of its own, and its audio
/// has to arrive here at the rate the mixer runs at.
#[test]
fn the_sound_card_produces_audio_at_the_mixers_rate() {
    let Some(machine) = booted() else { return };
    let (there, _) = machine.sound_stats();
    assert!(
        there,
        "the set carries a piggyback card and it should be loaded"
    );

    // A second of machine time is a second of audio, whatever the card's own
    // resistor-and-capacitor clock happens to be running at today.
    let mut samples = 0;
    for _ in 0..100 {
        machine.advance(0.01);
        samples += machine.take_audio().len();
    }
    let rate = vpw_game::AUDIO_RATE as usize;
    assert!(
        samples.abs_diff(rate) < rate / 20,
        "a second should be about {rate} samples, and was {samples}"
    );

    // And it should be a sound and not a level. An attract mode with the
    // tune switched off is genuinely silent, so the thing to listen for is a
    // coin: the game board writes a command, the card's processor is
    // interrupted, and it plays.
    let mut loud = false;
    for _ in 0..3 {
        machine.set_switch(SW_COIN3, true);
        for _ in 0..60 {
            machine.advance(0.01);
            loud |= machine.take_audio().iter().any(|s| s.abs() > 0.05);
        }
        machine.set_switch(SW_COIN3, false);
    }
    assert!(loud, "a coin should have made a noise");
}

/// The self-test button is a switch like any other on this board — there is no
/// second wire into the CPU the way a System 11 has.
#[test]
fn the_test_button_is_a_switch_in_the_matrix() {
    let Some(machine) = booted() else { return };
    machine.set_switch(SW_TEST, true);
    assert!(machine.switch_closed(SW_TEST));
    machine.set_switch(SW_TEST, false);
    assert!(!machine.switch_closed(SW_TEST));
}
