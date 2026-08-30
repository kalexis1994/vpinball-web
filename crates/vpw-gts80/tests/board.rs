//! The board's wiring, checked without a ROM.
//!
//! What a ROM proves is that the whole thing runs; what these prove is that
//! each wire goes where the schematic says, which is the part that is easy to
//! get subtly wrong and hard to see from a machine that boots anyway.

use vpw_bus::Bus;
use vpw_gts80::board::{NVRAM, System80};

fn board() -> System80 {
    System80::new(vec![0xEA; 2048], vec![0x11; 4096], vec![0x22; 4096])
}

#[test]
fn the_map_folds_because_two_address_lines_are_missing() {
    // A14 and A15 are not connected, so everything repeats four times. It is
    // not a curiosity: the CPU fetches its vectors from $FFFA, and with those
    // two lines gone that is the top of the U3 ROM. A map that does not fold
    // cannot boot.
    let mut mem = board();
    assert_eq!(mem.read(0x2000), 0x11, "U2");
    assert_eq!(mem.read(0x3000), 0x22, "U3");
    assert_eq!(mem.read(0xFFFC), 0x22, "the reset vector lives in U3");
    assert_eq!(mem.read(0x7000), 0x22, "and the whole map repeats");
}

#[test]
fn the_battery_ram_is_mirrored_eight_times() {
    // Two hundred and fifty-six bytes across two kilobytes of map, because
    // only eight address lines reach the chip.
    let mut mem = board();
    mem.write(0x1800, 0x5A);
    assert_eq!(mem.read(0x1800), 0x5A);
    assert_eq!(mem.read(0x1900), 0x5A, "the second copy");
    assert_eq!(mem.read(0x1F00), 0x5A, "and the eighth");
    mem.write(0x1FFF, 0x99);
    assert_eq!(mem.nvram[NVRAM - 1], 0x99);
}

#[test]
fn each_riot_has_its_own_hundred_and_twenty_eight_bytes() {
    let mut mem = board();
    mem.write(0x0000, 1);
    mem.write(0x0080, 2);
    mem.write(0x0100, 3);
    assert_eq!(mem.riot[0].ram[0], 1);
    assert_eq!(mem.riot[1].ram[0], 2);
    assert_eq!(mem.riot[2].ram[0], 3);
}

#[test]
fn the_switch_matrix_answers_the_strobe_it_is_given() {
    let mut mem = board();
    // Switch 34, which is not column 3 row 4: Gottlieb swapped the two, so it
    // is strobe 3 and return line 5. See the note in `io.rs`.
    mem.io.set_switch(34, true);
    assert!(mem.io.switch_closed(34));

    // Port B of RIOT 0 strobes, port A senses. Strobing another line finds
    // nothing; strobing this one answers on return 5.
    mem.write(0x0201, 0xFF); // DDRA: all output, so nothing reads back
    mem.write(0x0203, 0xFF); // DDRB: all output, which is the strobe
    mem.write(0x0201, 0x00); // and port A back to input
    mem.write(0x0202, 1 << 3);
    assert_eq!(mem.read(0x0200), 1 << 4, "return line 5 of the byte");

    mem.write(0x0202, 1 << 5);
    assert_eq!(mem.read(0x0200), 0, "a strobe with nothing closed on it");
}

/// The numbers a table actually uses, which are the ones worth being sure of:
/// a matrix built the other way round runs a game nobody can put a coin in.
#[test]
fn the_coin_door_is_where_sys80_vbs_says_it_is() {
    let mut mem = board();
    mem.write(0x0201, 0x00);
    mem.write(0x0203, 0xFF);

    // `swTest` 7, `swCoin1` 17, `swStartButton` 47, `swTilt` 57: all four on
    // return line 8, on strobes 0, 1, 4 and 5 (`sys80.vbs:23`).
    for (number, strobe) in [(7, 0), (17, 1), (47, 4), (57, 5)] {
        mem.io.set_switch(number, true);
        mem.write(0x0202, 1 << strobe);
        assert_eq!(mem.read(0x0200), 0x80, "switch {number}");
        mem.io.set_switch(number, false);
        assert_eq!(mem.read(0x0200), 0x00, "switch {number}, released");
    }

    // And the slam switch is number -1, which is not on any strobe at all: it
    // reaches RIOT 1's port A through a gate of its own.
    mem.io.set_switch(-1, true);
    assert!(mem.io.slammed());
    for strobe in 0..8 {
        mem.write(0x0202, 1 << strobe);
        assert_eq!(
            mem.read(0x0200),
            0x00,
            "the slam switch is not in the matrix"
        );
    }
}

#[test]
fn the_dips_take_the_matrixs_place_when_the_board_asks() {
    // There are more things to read than pins to read them with, so a bit of
    // RIOT 1's port B swaps the switch matrix for a bank of dips, and two bits
    // of RIOT 2's pick which bank.
    let mut mem = board();
    mem.io.dips = [0x01, 0x02, 0x04, 0x08];
    mem.write(0x0201, 0x00); // port A of RIOT 0 is an input

    mem.write(0x0283, 0xFF); // RIOT 1 port B: output
    mem.write(0x0282, 0x80); // and ask for dips
    mem.write(0x0303, 0xFF); // RIOT 2 port B: output
    mem.write(0x0302, 0x10); // bank 1

    // Reversed, because the bank is wired to the port the other way up.
    assert_eq!(mem.read(0x0200), 0x02u8.reverse_bits());
}

#[test]
fn the_lamp_latches_are_a_column_and_four_lamps() {
    let mut mem = board();
    mem.write(0x0303, 0xFF); // port B all output
    // Latch 3 — column 2, the first one a table can name — lamps 1 and 3.
    mem.write(0x0302, 0x30 | 0x05);
    assert_eq!(mem.io.lamps[2], 0x05);
    assert!(mem.io.lamp_lit(0), "the first lamp a table can reach");
    assert!(mem.io.lamp_lit(2));
    assert!(!mem.io.lamp_lit(1));

    // Latch zero is not a lamp at all: it is the game-on and tilt relays.
    mem.write(0x0302, 0x10 | 0x01);
    assert!(mem.io.game_on());
    assert!(!mem.io.tilted());
}

#[test]
fn the_solenoids_are_two_banks_of_four_and_a_ninth_on_its_own() {
    // The port drives them through inverting buffers, so the byte is read
    // upside down. Each bank has an enable, which is why only one coil per
    // bank can fire at a time — a real limit of the board and not a
    // simplification.
    let mut mem = board();
    mem.write(0x0301, 0xFF); // port A all output

    // Bank one enabled, coil 3 of it: enable is PA5, index in PA0-1.
    mem.write(0x0300, !(0x20 | 0x02));
    assert_eq!(mem.io.live & 0x0F, 0b0100, "solenoid 3");

    // Bank two, enabled by PA6, index in PA2-3.
    mem.write(0x0300, !(0x40 | 0x04));
    assert_eq!(mem.io.live & 0xF0, 0b0010_0000, "solenoid 6");

    // And the ninth has a pin to itself.
    mem.write(0x0300, !0x80);
    assert_eq!(mem.io.live, 0x100);
}

/// What a host reads is the window and not the instant, because a coil is
/// driven for a few milliseconds and a host looks once a frame.
#[test]
fn a_coil_that_fires_between_two_looks_still_happened() {
    let mut mem = board();
    mem.write(0x0301, 0xFF);

    mem.write(0x0300, !(0x20 | 0x02)); // solenoid 3 on
    mem.write(0x0300, !0x00); // and off again, all within one frame
    assert_eq!(mem.io.live, 0, "nothing is being driven this instant");

    // A quarter of a second is four sweeps; one is enough.
    mem.tick(vpw_gts80::board::CLOCK / 10);
    assert!(mem.io.solenoid_on(3), "the pulse is still reported");
}

/// The game-on and tilt relays are lamp latch zero, and a table reads them as
/// solenoids 10 and 11 — `sys80.vbs:20` calls solenoid 10 `GameOnSolenoid`,
/// and `vpmFlips` will not move a flipper without it.
#[test]
fn the_relays_are_reported_as_the_two_solenoids_above_the_nine() {
    let mut mem = board();
    mem.write(0x0303, 0xFF);

    mem.write(0x0302, 0x10 | 0x01); // latch 1: game on
    mem.tick(vpw_gts80::board::CLOCK / 10);
    assert!(mem.io.solenoid_on(10), "game on");
    assert!(!mem.io.solenoid_on(11));

    // Tilt takes game-on down with it, which is the wire that kills the
    // flippers on a tilt.
    mem.write(0x0302, 0x10 | 0x03);
    mem.tick(vpw_gts80::board::CLOCK / 10);
    assert!(!mem.io.solenoid_on(10), "tilted");
    assert!(mem.io.solenoid_on(11));
}

#[test]
fn the_sound_command_is_four_bits_and_a_borrowed_fifth() {
    // Sixteen sounds on the port, and a lamp latch's bit lends the board a
    // seventeenth to thirty-second.
    let mut mem = board();
    mem.write(0x0301, 0xFF);
    mem.write(0x0300, !(0x10 | 0x07));
    assert_eq!(mem.io.sound_command, 0x07);

    mem.io.lamps[2] = 0x02;
    mem.write(0x0300, !(0x10 | 0x05));
    assert_eq!(mem.io.sound_command, 0x15, "the lamp lent the high bit");
}

#[test]
fn the_displays_latch_on_a_rising_strobe_and_write_where_the_select_points() {
    let mut mem = board();
    mem.write(0x0281, 0xFF); // RIOT 1 port A: output
    mem.write(0x0283, 0xFF); // and port B

    // Put a 7 on port B, then raise the first latch's strobe with the select
    // at 0.
    mem.write(0x0282, 0x07);
    mem.write(0x0280, 0x00);
    mem.write(0x0280, 0x10);

    // What a host reads is the last complete sweep, so exactly one has to
    // end: the display is multiplexed, and a snapshot of it mid-sweep is one
    // lit digit and fifteen blanks. A second sweep would publish the empty
    // frame after this one, which on a running board is the score blinking.
    mem.tick(vpw_gts80::board::CLOCK / 25);

    let lit = mem.displays.digits[0].iter().filter(|d| **d != 0).count();
    assert_eq!(lit, 1, "one digit, where the select pointed");
    assert_eq!(mem.displays.number(0, 0, 16), 7);
}
