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
    let mut b = Board::new();
    assert!(b.switches.set(11, true), "11 is column 1 row 1");
    assert!(b.switches.set(23, true));

    b.write(0x3300, 0b0000_0001); // column 1
    assert_eq!(b.read(0x3400), !0b0000_0001, "row 1 of column 1 is closed");

    b.write(0x3300, 0b0000_0010); // column 2
    assert_eq!(b.read(0x3400), !0b0000_0100, "row 3 of column 2 is closed");

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
    b.switches.set(11, true);
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
    assert!(b.lamps.is_lit(65), "column 9 row 1 is lamp 65");
    assert_eq!(b.lamps.matrix().len(), LAMP_COLUMNS);
}

#[test]
fn the_coils_are_four_bytes_of_them() {
    let mut b = Board::new();
    b.write(0x2000, 0b0000_0001);
    b.write(0x2003, 0b1000_0000);
    assert!(b.solenoids.is_on(1));
    assert!(b.solenoids.is_on(32));
    assert!(!b.solenoids.is_on(2));
    assert_eq!(b.read(0x2000), 0b0000_0001);
}

#[test]
fn a_pulse_shorter_than_a_frame_is_still_reported() {
    // A coil fires for thirty milliseconds and a host samples once a frame, so
    // the live word alone loses pulses. Everything seen since the last ask is
    // what a host has to read.
    let mut b = Board::new();
    b.write(0x2000, 0b0000_0010);
    b.write(0x2000, 0b0000_0000);
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
