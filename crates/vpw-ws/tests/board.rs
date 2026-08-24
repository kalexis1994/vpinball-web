//! The Whitestar board, without needing a ROM.
//!
//! Everything here is about the wiring: which byte lands in which latch, which
//! way round the bits go, and what the CPU sees when it reads back. None of it
//! needs a game, which is the point — a matrix that is wired backwards is far
//! easier to find here than in a photograph of a table with the wrong lamps on.

use vpw_bus::Bus;
use vpw_ws::Board;
use vpw_ws::io::LAMP_COLUMNS;

#[test]
fn ram_is_ram() {
    let mut b = Board::new();
    b.write(0x0000, 0x5a);
    b.write(0x1fff, 0xa5);
    assert_eq!(b.read(0x0000), 0x5a);
    assert_eq!(b.read(0x1fff), 0xa5);
}

#[test]
fn the_fixed_half_comes_from_the_end_of_the_image() {
    // `ROM_COPY(SE_ROMREGION, 0x18000, 0x8000, 0x8000)` — the CPU's `$8000` is
    // byte `0x18000` of the region, which for a 128 KB game is the last 32 KB
    // of its image. This is the line that decides whether the processor starts
    // on the game's reset vector or on somebody else's.
    let mut image = vec![0u8; 0x2_0000];
    image[0x1_8000] = 0xc0;
    image[0x1_ffff] = 0x0d;
    let mut b = Board::new();
    b.load_rom(&image).expect("128 KB fits");
    assert_eq!(b.read(0x8000), 0xc0);
    assert_eq!(b.read(0xffff), 0x0d);
}

#[test]
fn a_small_image_is_mirrored_to_fill_the_region() {
    // The region is half a megabyte whatever the game is, and the original
    // fills it by loading the image over and over (`se.h:82`). It is not a
    // convenience: it is what makes the top bank bits harmless.
    let mut image = vec![0u8; 0x2_0000];
    image[0] = 0x11;
    let mut b = Board::new();
    b.load_rom(&image).expect("128 KB fits");
    for bank in [0u8, 8, 16, 24] {
        b.write(0x3200, bank);
        assert_eq!(b.read(0x4000), 0x11, "bank {bank} should see the image");
    }
}

#[test]
fn the_bank_register_pages_the_window_at_4000() {
    let mut image = vec![0u8; 0x8_0000];
    for bank in 0..32usize {
        image[bank * 0x4000] = bank as u8;
    }
    let mut b = Board::new();
    b.load_rom(&image).expect("512 KB fits");
    for bank in 0..32u8 {
        b.write(0x3200, bank);
        assert_eq!(b.read(0x4000), bank, "bank {bank}");
        assert_eq!(b.bank(), bank);
    }
}

#[test]
fn the_top_bit_of_the_bank_register_is_the_diagnostic_led() {
    // And it is inverted: `selocals.diagnosticLed = (data>>7)?0:1` (`se.c:568`).
    let mut b = Board::new();
    b.write(0x3200, 0x00);
    assert!(b.diagnostic_led);
    b.write(0x3200, 0x80);
    assert!(!b.diagnostic_led);
}

#[test]
fn the_switch_matrix_answers_the_column_it_is_strobed_with() {
    // Numbered straight through: 1 to 8 are column 1, 9 to 16 column 2, and so
    // on. See `switches_are_numbered_straight_through_and_not_by_column`.
    let mut b = Board::new();
    assert!(b.switches.set(1, true));
    assert!(b.switches.set(11, true));

    b.write(0x3300, 0b0000_0001); // column 1
    assert_eq!(b.read(0x3400), !0b0000_0001, "switch 1 is column 1 bit 0");

    b.write(0x3300, 0b0000_0010); // column 2
    assert_eq!(b.read(0x3400), !0b0000_0100, "switch 11 is column 2 bit 2");

    b.write(0x3300, 0b0000_0100); // column 3, nothing on it
    assert_eq!(b.read(0x3400), 0xff);
}

#[test]
fn a_closed_switch_reads_as_a_zero() {
    // Active low, which is the sort of thing that works perfectly in reverse:
    // a game whose switches are all inverted boots, scans, and thinks every
    // target is permanently hit.
    let mut b = Board::new();
    b.write(0x3300, 0xff);
    assert_eq!(b.read(0x3400), 0xff, "nothing closed reads as all ones");
    b.switches.set(1, true);
    assert_eq!(b.read(0x3400) & 1, 0);
}

#[test]
fn the_lamp_rows_are_wired_backwards() {
    // `core_revbyte` (`se.c:610`). Getting it wrong lights the right number of
    // lamps in the wrong places, which is harder to spot than lighting none.
    let mut b = Board::new();
    b.write(0x200a, 0b1000_0000);
    assert_eq!(b.lamps.rows(), 0b0000_0001);
    b.write(0x200a, 0b0000_0001);
    assert_eq!(b.lamps.rows(), 0b1000_0000);
}

#[test]
fn a_lamp_is_lit_by_its_column_and_its_row_together() {
    let mut b = Board::new();
    b.write(0x200a, 0b1000_0000); // reversed: row 1
    b.write(0x2008, 0b0000_0001); // column 1
    // A lamp has to be seen in **both** windows of a pair before it shows, so
    // the strobe is repeated and two windows are closed. See `io::Lamps`.
    b.lamps.end_window();
    b.write(0x2008, 0b0000_0001);
    b.lamps.end_window();
    assert!(b.lamps.is_lit(1), "column 1 row 1 is lamp 1");
    assert!(!b.lamps.is_lit(2));
    assert!(!b.lamps.is_lit(9), "column 2 was never strobed");
}

#[test]
fn the_high_columns_come_from_the_auxiliary_port() {
    // Sixteen columns in two halves, `$2008` and `$2009`. Without the second
    // the top sixty-four lamps of every game are dark.
    let mut b = Board::new();
    b.write(0x200a, 0b1000_0000);
    b.write(0x2009, 0b0000_0001); // column 9
    b.lamps.end_window();
    b.write(0x2009, 0b0000_0001);
    b.lamps.end_window();
    assert!(b.lamps.is_lit(65), "column 9 row 1 is lamp 65");
    assert_eq!(b.lamps.matrix().len(), LAMP_COLUMNS);
}

#[test]
fn the_coils_are_four_bytes_of_them() {
    let mut b = Board::new();
    b.write(0x2001, 0b0000_0001);
    b.write(0x2003, 0b1000_0000);
    assert!(b.solenoids.is_on(1));
    assert!(b.solenoids.is_on(32));
    assert!(!b.solenoids.is_on(2));
    assert_eq!(b.read(0x2001), 0b0000_0001);
}

#[test]
fn a_pulse_shorter_than_a_frame_is_still_reported() {
    // A coil fires for thirty milliseconds and a host samples once a frame, so
    // the live word alone loses pulses. Everything seen since the last ask is
    // what a host has to read.
    let mut b = Board::new();
    b.write(0x2001, 0b0000_0010);
    b.write(0x2001, 0b0000_0000);
    assert_eq!(b.solenoids.live(), 0, "it is over");
    assert_eq!(b.solenoids.take_seen() & 0b10, 0b10, "it still happened");
    assert_eq!(b.solenoids.take_seen(), 0, "and only once");
}

#[test]
fn nothing_in_the_map_is_a_hole() {
    // Every address the board answers is one the original answers too. An
    // address that is quietly nothing is how a board runs for twenty minutes
    // and then does something inexplicable.
    let mut b = Board::new();
    for addr in [
        0x0000u16, 0x1fff, 0x2000, 0x2003, 0x2007, 0x2008, 0x2009, 0x200a, 0x3000, 0x3100, 0x3400,
        0x3406, 0x3407, 0x3500, 0x3700, 0x4000, 0x7fff, 0x8000, 0xffff,
    ] {
        b.last_unmapped = None;
        let _ = b.read(addr);
        assert!(
            b.last_unmapped.is_none(),
            "read of {addr:04x} fell off the map"
        );
    }
    for addr in [
        0x0000u16, 0x2000, 0x2006, 0x2008, 0x2009, 0x200a, 0x200b, 0x3200, 0x3300, 0x3406, 0x3500,
        0x3600, 0x3601, 0x3800, 0x3801,
    ] {
        b.last_unmapped = None;
        b.write(addr, 0);
        assert!(
            b.last_unmapped.is_none(),
            "write to {addr:04x} fell off the map"
        );
    }
}

// --------------------------------------------------- how switches are numbered ---
//
// The single most consequential thing in the board, and the one that is silent
// when wrong: Sega and Stern number their switches straight through, one to
// sixty-four, where Williams numbers them by column and row. Read with the
// wrong convention the board still boots, still scans, and still answers — it
// just answers about different switches, so a coin is never a credit, the start
// button is six columns from the start button, and the ball trough reports
// somebody else's switches.

#[test]
fn switches_are_numbered_straight_through_and_not_by_column() {
    // `se.c:252` puts Start, Tilt and Slam Tilt — "Switches 54,55,56" — into
    // column 7 at bits 5, 6 and 7. That is the whole convention in one line.
    let mut b = Board::new();
    b.switches.set(54, true);
    b.write(0x3300, 1 << 6); // column 7 is the seventh strobe bit
    assert_eq!(
        b.read(0x3400),
        !(1 << 5),
        "switch 54 is column 7 bit 5, not column 5 row 4"
    );
}

#[test]
fn the_coin_slots_land_where_the_script_library_says() {
    // `sega.vbs:32` names switches 4, 5 and 6 as the three coin slots, and the
    // original puts the coin switches in column 1 starting at the fourth bit.
    // The two only agree under this numbering.
    let mut b = Board::new();
    for (number, bit) in [(4u8, 3u8), (5, 4), (6, 5)] {
        let mut fresh = Board::new();
        fresh.switches.set(number, true);
        fresh.write(0x3300, 1);
        assert_eq!(
            fresh.read(0x3400),
            !(1 << bit),
            "coin switch {number} should be column 1 bit {bit}"
        );
    }
    // And nothing about it collides with the ball trough, which the table puts
    // on 11 to 14 — the next column along.
    b.switches.set(11, true);
    b.write(0x3300, 1 << 1);
    assert_eq!(b.read(0x3400), !(1 << 2), "switch 11 is column 2 bit 2");
}

#[test]
fn sixty_four_switches_and_no_more() {
    let mut b = Board::new();
    assert!(b.switches.set(1, true));
    assert!(b.switches.set(64, true));
    assert!(!b.switches.set(0, true), "there is no switch zero");
    assert!(!b.switches.set(65, true), "the matrix stops at sixty-four");
    assert!(b.switches.is_closed(1));
    assert!(b.switches.is_closed(64));
}

#[test]
fn the_first_and_last_switch_are_the_corners_of_the_matrix() {
    let mut b = Board::new();
    b.switches.set(1, true);
    b.write(0x3300, 1);
    assert_eq!(b.read(0x3400), !1, "switch 1 is column 1 bit 0");

    let mut b = Board::new();
    b.switches.set(64, true);
    b.write(0x3300, 1 << 7);
    assert_eq!(b.read(0x3400), !(1 << 7), "switch 64 is column 8 bit 7");
}

#[test]
fn the_solenoid_bytes_are_not_in_address_order() {
    // `se.c:662`: `{ 8, 0, 16, 24 }`. The byte at `$2000` is coils 9 to 16 and
    // the byte at `$2001` is coils 1 to 8, which is the wrong way round from
    // what anyone would assume. It costs nothing at boot and everything
    // afterwards: both banks are real coils, so the machine fires the wrong
    // ones rather than none.
    let mut b = Board::new();
    b.write(0x2001, 1);
    assert!(b.solenoids.is_on(1), "$2001 bit 0 is coil 1");
    assert!(!b.solenoids.is_on(9));

    let mut b = Board::new();
    b.write(0x2000, 1);
    assert!(b.solenoids.is_on(9), "$2000 bit 0 is coil 9");
    assert!(!b.solenoids.is_on(1));
}

/// The rule the original spells out and this game needs: a lamp shows only if
/// it was driven in both windows of the pair. One is a lamp the game is
/// flickering as part of its artwork, and flickering is not on.
#[test]
fn a_lamp_seen_in_only_one_window_of_a_pair_stays_dark() {
    let mut b = Board::new();
    b.write(0x200a, 0b1000_0000); // row 1
    b.write(0x2008, 0b0000_0001); // column 1
    b.lamps.end_window();
    // Nothing drives it during the second window.
    b.write(0x200a, 0);
    b.lamps.end_window();
    assert!(!b.lamps.is_lit(1), "seen once is not lit");

    // Driven through both, it shows.
    for _ in 0..2 {
        b.write(0x200a, 0b1000_0000);
        b.write(0x2008, 0b0000_0001);
        b.lamps.end_window();
    }
    assert!(b.lamps.is_lit(1));
}

/// The expander board's columns are latched, not strobed: they are set once
/// and stay, where a strobed lamp that is not driven again goes out.
#[test]
fn an_expander_boards_lamps_are_not_swept() {
    let mut b = Board::new();
    b.boards = vpw_ws::board::Boards {
        aux_solenoids: true,
        leds: true,
    };
    // The data latch, then a falling edge of ASTB to clock the low byte and a
    // rising one to clock the high byte, whose top bits pick the row.
    b.write(0x2006, 0b0000_0101);
    b.write(0x200b, 0x80);
    b.write(0x200b, 0x00); // falling: eight LEDs
    b.write(0x2006, 0b0100_0000);
    b.write(0x200b, 0x80); // rising: the row select
    assert!(b.lamps.is_lit(81), "column 10 row 1 is lamp 81");
    assert!(b.lamps.is_lit(83));
    // And it is still lit after a pair of windows nobody drove it in.
    b.lamps.end_window();
    b.lamps.end_window();
    assert!(
        b.lamps.is_lit(81),
        "a latched lamp does not need re-driving"
    );
}

/// The relay is wired the other way up from what anyone would guess, and
/// guessing costs a playfield that is dark whenever the game wants it lit.
#[test]
fn the_general_illumination_relay_is_on_when_the_bit_is_low() {
    let mut b = Board::new();
    b.write(0x200b, 0x00);
    assert!(b.gi_relay, "bit low is the relay closed");
    b.write(0x200b, 0x01);
    assert!(!b.gi_relay);
}
