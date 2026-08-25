//! A real game, end to end, on the machine the whole crate is written for.
//!
//! The board tests say each latch is wired the way the schematic says. This
//! says the wiring adds up to a game: that a button behind the coin door gives
//! a credit, that the start button starts, that the game then turns its
//! flippers on, and that pressing a flipper button drives a flipper coil. Every
//! one of those crosses a boundary the unit tests cannot — the ROM has to agree
//! that what it read was the switch we closed.
//!
//! Needs the ROM, which is copyrighted firmware and is not in the repository.
//! Put `lotr.zip` in `web/debug-assets/roms/` (already ignored by git) or point
//! `VPW_ROMS` at wherever you keep yours; without it these skip themselves.

use std::io::Read;
use std::path::PathBuf;

use vpw_ws::Whitestar;

const SET: &str = "lotr";

/// The six trough switches, and the numbers `sega.vbs` gives the buttons.
const TROUGH: [i32; 6] = [9, 10, 11, 12, 13, 14];
const SW_GREEN: i32 = -1;
const SW_START: i32 = 54;
const SW_LEFT_FLIPPER: i32 = 84;

/// The left flipper's power coil, after `se_solenoid_w` moves it off coil 15.
const SOL_LEFT_FLIPPER: u8 = 47;
/// The fast-flips flag, which is what a table polls as `GameOnSolenoid`.
const SOL_GAME_ON: u8 = 15;

fn booted() -> Option<Whitestar> {
    let dir = match std::env::var("VPW_ROMS") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from("../../web/debug-assets/roms"),
    };
    let path = dir.join(format!("{SET}.zip"));
    if !path.is_file() {
        eprintln!("skipped: no {SET}.zip in {}", dir.display());
        return None;
    }
    let file = std::fs::File::open(&path).expect("the ROM will not open");
    let mut zip = zip::ZipArchive::new(file).expect("not a zip");
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("could not read an entry");
        if entry.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("could not decompress");
        images.push((entry.name().to_ascii_lowercase(), data));
    }
    let find = |want: &str| {
        images
            .iter()
            .find(|(name, _)| name.contains(&want.to_ascii_lowercase()))
            .map(|(_, data)| data.as_slice())
    };

    let game = vpw_ws::games::find(SET).expect("lotr is a set this knows");
    let mut machine = Whitestar::new();
    machine.board.boards = game.boards;
    machine.board.fast_flip_addr = game.fast_flips;
    machine
        .load_rom(find(game.cpu).expect("the CPU image is in the zip"))
        .expect("the CPU image fits");
    if let Some(display) = find(game.display) {
        machine.load_display_rom(display).expect("the display fits");
    }
    // Balls in the trough. Without them the game never gets past looking for
    // them: it says so on the display and pulses coils hunting for a ball that
    // is not there.
    for number in TROUGH {
        assert!(machine.board.switches.set(number, true));
    }
    machine.run_seconds(3.0);
    // Nothing is in play yet, so the flag has to be clear — otherwise every
    // assertion below about it coming on would pass on a machine that never
    // touched it. The virgin RAM this board powers up with is all ones
    // (`se.c:1187`), so this is also the check that the game got far enough to
    // clear its own variables.
    assert!(
        !machine.board.fast_flips(),
        "attract mode should not have the flippers live"
    );
    Some(machine)
}

/// Holds a switch closed for a moment and lets it go, the way a finger does.
fn press(machine: &mut Whitestar, number: i32) {
    assert!(machine.board.switches.set(number, true), "switch {number}");
    machine.run_seconds(0.2);
    machine.board.switches.set(number, false);
    machine.run_seconds(0.2);
}

/// The one that says the coin door is wired at all.
///
/// The green Service Credit button is switch **-1** (`sega.vbs:26`), which
/// `core_swSeq2m` puts in matrix column 0 at bit 6, which is where
/// `dedswitch_r` reads it (`se.c:648`). A port whose switch numbering starts at
/// 1 drops it silently, and then there is no way to put a credit on the machine
/// at all — no coin, no service credit, and no way into the service menu to
/// turn free play on.
#[test]
fn the_green_button_behind_the_coin_door_gives_a_credit_and_start_starts_a_game() {
    let Some(mut machine) = booted() else { return };

    // The ROM has to see the button while it is down, so it goes down, the
    // board runs, and it comes up.
    press(&mut machine, SW_GREEN);
    press(&mut machine, SW_START);
    machine.run_seconds(6.0);

    // A game in progress sets the flag the original reads out of CPU RAM and
    // reports as solenoid 15 (`se.c:185`). It is the machine saying, in the one
    // place a table's script can hear it, that the flippers are live.
    assert!(
        machine.board.fast_flips(),
        "after a credit and a start the game should have turned the flippers on"
    );
    assert!(machine.board.solenoids.is_on(SOL_GAME_ON));
}

/// And the other end of the same wire: a flipper button is switch 84, in the
/// cabinet column, and what should come back is coil 15 — which
/// `se_solenoid_w` moves to solenoid 47 on its way out (`se.c:666`).
#[test]
fn the_left_flipper_button_drives_the_left_flipper_coil() {
    let Some(mut machine) = booted() else { return };
    press(&mut machine, SW_GREEN);
    press(&mut machine, SW_START);
    machine.run_seconds(6.0);
    assert!(machine.board.fast_flips(), "the game should be under way");

    assert!(
        !machine.board.solenoids.is_on(SOL_LEFT_FLIPPER),
        "nothing is pressing it yet"
    );
    machine.board.switches.set(SW_LEFT_FLIPPER, true);
    machine.run_seconds(0.2);
    assert!(
        machine.board.solenoids.is_on(SOL_LEFT_FLIPPER),
        "the button is switch 84 and the coil comes back as solenoid 47"
    );
    // The board also reports it as 48, which is the number a table's script
    // actually watches — `sLLFlipper` (`core.vbs:2850`).
    assert!(machine.board.solenoids.is_on(48));
}

/// Lord of the Rings drives its last three lamp columns from a nineteen-LED
/// board on the auxiliary connector, and `$3406`/`$3407` are two of those same
/// columns (`gilamp_w`, `se.c:628`). The two collide — in the original as much
/// as here — and the last writer wins. This is the check that on this game the
/// last writer is the LED board, so wiring `$3406` up to the lamp matrix where
/// it belongs did not blank the lamps that were already working.
#[test]
fn the_led_boards_lamp_columns_survive_the_ports_that_share_them() {
    let Some(mut machine) = booted() else { return };
    // The board leaves them dark until the game is through its power-up and
    // into attract mode, which takes several seconds, so this waits rather
    // than looking once.
    let mut led = [0u8; 3];
    for _ in 0..20 {
        machine.run_seconds(1.0);
        let matrix = machine.board.lamps.matrix();
        led = [matrix[10], matrix[11], matrix[12]];
        if led != [0, 0, 0] {
            break;
        }
    }
    assert_ne!(
        led,
        [0, 0, 0],
        "the LED board should be lighting something within twenty seconds"
    );
}
