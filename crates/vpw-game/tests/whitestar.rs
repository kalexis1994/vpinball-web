//! A Whitestar machine, through the bridge the table's script sees.
//!
//! The board tests in `vpw-ws` say the silicon works. This says the game gets
//! it: that a set name a table asks for finds three processors' worth of
//! images in one zip, that the game board runs, and — the part that is easy to
//! declare finished and hard to be sure about — that the sound board's output
//! comes back out of the machine at the rate the mixer wants.
//!
//! Needs the ROM, which is copyrighted firmware and is not in the repository.
//! Put `lotr.zip` in `web/debug-assets/roms/` (already ignored by git) or point
//! `VPW_ROMS` at wherever you keep yours; without it these skip themselves.

use std::path::PathBuf;

use vpw_game::AUDIO_RATE;
use vpw_game::controller::Machine;

/// The Lord of the Rings, as its script spells it.
const SET: &str = "lotr";

/// The six trough switches, found by closing candidates until the display
/// stopped saying `LOOKING FOR PINBALLS`.
const TROUGH: [u8; 6] = [9, 10, 11, 12, 13, 14];

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
    // Six balls in the trough. Without them the game never gets past looking
    // for them: it says so on the display, keeps every lamp dark, and pulses
    // the coils hunting for a ball that is not there.
    for number in TROUGH {
        machine.set_switch(number, true);
    }
    Some(machine)
}

#[test]
fn the_zip_holds_three_boards_and_all_of_them_load() {
    let Some(machine) = booted() else { return };
    assert!(machine.is_running());
    assert_eq!(machine.game_name(), Some(SET));
}

#[test]
fn the_game_board_drives_the_playfield() {
    let Some(machine) = booted() else { return };
    for _ in 0..5000 {
        machine.advance(0.001);
    }
    assert!(
        !machine.changed_lamps().is_empty(),
        "after five seconds the game should have driven a lamp"
    );
    // And it should know about its balls, which is what got it this far.
    assert!(TROUGH.iter().all(|&n| machine.switch_closed(n)));
}

/// The one that matters: the sound board is a processor running its own
/// firmware at twenty times the game's clock, and its audio has to arrive
/// here resampled to the rate the mixer runs at.
///
/// The board spends its first seconds flashing its LED — eight times, a second
/// each — and then plays a startup tone, so the test has to wait for it. That
/// wait is the board's, not this emulator's: the original patches those delays
/// out precisely because they are there.
#[test]
fn the_sound_board_produces_audio_at_the_mixers_rate() {
    let Some(machine) = booted() else { return };

    let mut heard = 0usize;
    let mut peak = 0.0f32;
    for _ in 0..7000 {
        machine.advance(0.001);
        for sample in machine.take_audio() {
            heard += 1;
            peak = peak.max(sample.abs());
            assert!(
                sample.abs() <= 1.0,
                "a sample outside the range a mixer takes: {sample}"
            );
        }
    }

    // Seven seconds of machine time, at the mixer's rate, allowing for the
    // board's own clock not dividing into it evenly and for the last few
    // samples still being in flight.
    let want = AUDIO_RATE as usize * 7;
    assert!(
        heard > want * 9 / 10,
        "expected about {want} samples and got {heard}"
    );
    assert!(
        peak > 0.0,
        "the board ran for seven seconds without a sound"
    );
}
