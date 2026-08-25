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
    b.solenoids.end_sweep(false);
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

/// The half of that which was missing: the accumulator existed and nothing
/// closed a sweep on it, so what a script read was the live latch and a coil
/// pulse that fell between two polls simply never happened. Pop bumpers,
/// kickers and the trough eject that fire "sometimes" are all this.
#[test]
fn a_coil_that_pulsed_between_sweeps_is_what_the_script_reads() {
    let mut b = Board::new();
    b.write(0x2001, 0b0000_0010); // coil 2 on
    b.write(0x2001, 0b0000_0000); // and off again, in the same sweep
    assert_eq!(b.solenoids.live(), 0, "the latch has already forgotten");
    assert!(!b.solenoids.is_on(2), "and no sweep has closed yet");

    b.solenoids.end_sweep(false);
    assert!(b.solenoids.is_on(2), "the sweep saw it");
    b.solenoids.end_sweep(false);
    assert!(!b.solenoids.is_on(2), "and the next one does not repeat it");
}

/// `se_solenoid_w` (`se.c:666`) pulls the top two bits of `$2000` — coils 16
/// and 15 — out of the word entirely and reports them as solenoids 45 and 47.
/// A table's script drives its flippers from those numbers and would otherwise
/// be watching two coils that mean nothing to it.
#[test]
fn the_flipper_power_coils_are_moved_to_forty_five_and_forty_seven() {
    let mut b = Board::new();
    b.write(0x2000, 0b1100_0000);
    b.solenoids.end_flipper_window();
    b.solenoids.end_sweep(false);

    assert!(b.solenoids.is_on(45), "bit 7 is coil 16, the right flipper");
    assert!(b.solenoids.is_on(47), "bit 6 is coil 15, the left flipper");
    // And the two hold outputs the board does not have answer for their power
    // outputs, which is what `core_getSol` does (`core.c:2209`) and what the
    // script actually watches: `sLRFlipper` is 46 and `sLLFlipper` is 48.
    assert!(b.solenoids.is_on(46));
    assert!(b.solenoids.is_on(48));
    // They are gone from the thirty-two: `sols &= 0xffff3fff`.
    assert!(!b.solenoids.is_on(15));
    assert!(!b.solenoids.is_on(16));
    assert_eq!(b.solenoids.live() & 0xc000, 0);
    // But a ROM reading the port back is told what it wrote (`se.c:680`).
    assert_eq!(b.read(0x2000), 0b1100_0000);
}

/// Solenoid 15 is free because the left flipper's power output moved to 47,
/// and the original reuses it for the fast-flips flag — a byte of CPU RAM the
/// game sets while the flippers are live (`se.c:185`). `sega.vbs:23` calls it
/// `GameOnSolenoid`, and `vpmFlips` polls it to decide whether a flipper key
/// does anything at all, so a machine without it plays perfectly and cannot be
/// flipped.
#[test]
fn solenoid_fifteen_is_the_fast_flips_flag() {
    let mut b = Board::new();
    b.fast_flip_addr = Some(0x0004);
    b.write(0x0004, 0);
    assert!(!b.fast_flips());
    b.solenoids.end_sweep(b.fast_flips());
    assert!(!b.solenoids.is_on(15));

    b.write(0x0004, 192); // what the games are found to write
    assert!(b.fast_flips());
    b.solenoids.end_sweep(b.fast_flips());
    assert!(b.solenoids.is_on(15));

    // A game whose address nobody has looked up gets nothing rather than a
    // guess, which is what the original does with `fastflipaddr == 0`.
    let mut b = Board::new();
    b.write(0x0004, 192);
    assert!(!b.fast_flips());
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
fn the_numbering_reaches_below_one_and_above_sixty_four() {
    // `core_swSeq2m(n) = n + 7` (`core.c:2108`) is a plain offset, not a range
    // check, and both ends of what it reaches are switches a table uses: the
    // coin door below and the cabinet's flipper buttons above. A matrix that
    // holds only 1 to 64 drops all of them, silently, and what that costs is a
    // machine you cannot flip, cannot give a service credit and cannot get
    // into the service menu.
    let mut b = Board::new();
    assert!(b.switches.set(1, true));
    assert!(b.switches.set(64, true));
    assert!(b.switches.set(0, true), "switch zero is the black button");
    assert!(b.switches.set(-3, true), "and -3 is memory protect");
    assert!(b.switches.set(84, true), "84 is the left flipper button");
    assert!(b.switches.is_closed(1));
    assert!(b.switches.is_closed(64));
    assert!(b.switches.is_closed(0));
    assert!(b.switches.is_closed(-3));
    assert!(b.switches.is_closed(84));
    // Twelve columns of eight, offset by seven: -7 to 88 and nothing else.
    assert!(!b.switches.set(-8, true));
    assert!(!b.switches.set(89, true));
}

/// The flipper buttons, which is the item this whole numbering exists for.
///
/// They are switches 81 to 84 in the cabinet column, `CORE_FLIPPERSWCOL`
/// (`core.h:334`), and `dedswitch_r` folds that column into the low five bits
/// of `$3000` with its low nybble **reversed** (`se.c:648`).
#[test]
fn the_flipper_buttons_arrive_in_the_dedicated_byte() {
    // `sega.vbs:36`: swLRFlip 82, swLLFlip 84, swURFlip 81, swULFlip 83.
    // `se.c:641`: D0 left flipper, D1 left EOS, D2 right flipper, D3 right EOS.
    for (number, bit) in [(84, 0), (83, 1), (82, 2), (81, 3)] {
        let mut b = Board::new();
        assert!(b.switches.set(number, true), "switch {number} exists");
        assert_eq!(
            b.read(0x3000),
            !(1u8 << bit),
            "switch {number} should be bit {bit} of $3000, active low"
        );
    }
    let mut b = Board::new();
    assert_eq!(b.read(0x3000), 0xff, "nothing pressed is all ones");
}

/// The three buttons behind the coin door and the memory protect switch, which
/// `sega.vbs` numbers 0, -1, -2 and -3 (`se.h:70`). They are not a special case
/// in the board — `n + 7` puts them in column 0 at bits 7 down to 4, which is
/// exactly where `dedswitch_r` reads them.
#[test]
fn the_coin_doors_buttons_are_switches_zero_and_below() {
    for (number, bit) in [(0, 7), (-1, 6), (-2, 5)] {
        let mut b = Board::new();
        assert!(b.switches.set(number, true));
        assert_eq!(
            b.read(0x3000),
            !(1u8 << bit),
            "switch {number} should be bit {bit} of $3000"
        );
    }
    // Memory protect is the fourth of them and the one the port does not
    // report: bit 4 is "unused" in the dedicated byte (`se.c:641`).
    let mut b = Board::new();
    assert!(b.switches.set(-3, true));
    assert!(b.switches.memory_protect());
    assert_eq!(b.read(0x3000), 0xff, "it is not one of the buttons");
}

/// `core_getSwCol` (`core.c:2122`) walks to the **lowest** set bit and returns
/// that column alone. ORing every selected column instead reports switches on
/// columns the ROM is not looking at, which during the switch test reads as
/// targets hitting themselves.
#[test]
fn a_strobe_with_several_bits_selects_only_the_lowest() {
    let mut b = Board::new();
    b.switches.set(1, true); // column 1, bit 0
    b.switches.set(9, true); // column 2, bit 0
    b.write(0x3300, 0b0000_0011);
    assert_eq!(
        b.read(0x3400),
        !0b0000_0001,
        "columns 1 and 2 both selected answers column 1"
    );
    b.write(0x3300, 0b0000_0010);
    assert_eq!(b.read(0x3400), !0b0000_0001, "column 2 on its own");
}

#[test]
fn a_zero_strobe_answers_column_one_and_not_all_open() {
    // `core_getSwCol` starts its walk at column 1 and only moves off it when
    // there is a bit to move for, so a zero strobe reads column 1 rather than
    // an open matrix. It matters at power-up, before the ROM has written a
    // strobe at all.
    let mut b = Board::new();
    b.switches.set(1, true);
    b.write(0x3300, 0);
    assert_eq!(b.read(0x3400), !0b0000_0001);
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
    b.solenoids.end_sweep(false);
    assert!(b.solenoids.is_on(1), "$2001 bit 0 is coil 1");
    assert!(!b.solenoids.is_on(9));

    let mut b = Board::new();
    b.write(0x2000, 1);
    b.solenoids.end_sweep(false);
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

// ------------------------------------------------------ the rest of the ports ---

/// `dip_r` answers the **complement** of the switches (`se.c:659`). Reading
/// them straight through is invisible on a board with none set — both give
/// `0xff` — and reverses every country setting the moment one is.
#[test]
fn the_dip_port_answers_the_complement_of_the_switches() {
    let mut b = Board::new();
    assert_eq!(b.dips, 0x00, "an untouched board has none of them on");
    assert_eq!(b.read(0x3100), 0xff);
    b.dips = 0b0000_0101;
    assert_eq!(b.read(0x3100), 0b1111_1010);
}

/// `$3406` and `$3407` are lamp columns 10 and 11 — lamps 81 to 96 — whatever
/// the "general illumination" in the memory map says: `gilamp_w` writes
/// `tmpLampMatrix[10 + offset]` (`se.c:628`). Keeping them in a byte no lamp
/// reader sees leaves a game's last two banks of lamps dark for ever.
#[test]
fn the_ports_at_3406_are_lamp_columns_and_not_general_illumination() {
    let mut b = Board::new();
    b.write(0x3406, 0b0000_0101);
    b.write(0x3407, 0b1000_0000);
    assert!(b.lamps.is_lit(81), "column 10 row 1 is lamp 81");
    assert!(b.lamps.is_lit(83));
    assert!(!b.lamps.is_lit(82));
    assert!(b.lamps.is_lit(96), "column 11 row 8 is lamp 96");
    // And the ROM reads them back out of the same place (`gilamp_r`).
    assert_eq!(b.read(0x3406), 0b0000_0101);
    assert_eq!(b.read(0x3407), 0b1000_0000);
    // Latched, not strobed: the original never clears columns 10 and up
    // between sweeps (`se.c:161`).
    b.lamps.end_window();
    b.lamps.end_window();
    assert!(b.lamps.is_lit(81));
}

/// `auxboard_r` (`se.c:775`) answers `0x40` while DSTB — bit 5 of the strobe
/// latch at `$200b` — is low, which is the Tournament Serial Board's DUART
/// saying it is ready. A game with no TSIB never pulls DSTB low, so on the
/// games here this only ever shows up at power-up, when the latch is zero.
#[test]
fn the_aux_port_answers_the_duart_while_dstb_is_low() {
    let mut b = Board::new();
    b.write(0x2006, 0x5a);
    assert_eq!(b.read(0x2007), 0x40, "DSTB is low out of reset");
    b.write(0x200b, 0x20);
    assert_eq!(b.read(0x2007), 0x5a, "DSTB high is the latch");
}

/// `ram_w` refuses writes above `0x1E00` while the coin door's memory protect
/// switch is set (`se.c:589`). That is where the audits and the high scores
/// live, and the switch is what stops an operator with the door open from
/// wiping them.
#[test]
fn memory_protect_write_protects_the_top_of_the_nvram() {
    let mut b = Board::new();
    b.write(0x1e00, 0x11);
    b.write(0x1dff, 0x22);
    b.switches.set(-3, true); // swMemoryProtect
    b.write(0x1e00, 0x99);
    b.write(0x1fff, 0x99);
    b.write(0x1dff, 0x33);
    assert_eq!(b.read(0x1e00), 0x11, "protected");
    assert_eq!(b.read(0x1dff), 0x33, "below the line, still writable");
    b.switches.set(-3, false);
    b.write(0x1e00, 0x99);
    assert_eq!(b.read(0x1e00), 0x99);
}

/// `NVRAM_HANDLER(se)` fills a virgin CMOS with `0xff`, not zero
/// (`se.c:1187`). The ROM checksums this, so a board that powers up all-zeroes
/// is a board making a different decision about its own defaults than a real
/// one would.
#[test]
fn a_virgin_nvram_is_all_ones() {
    let b = Board::new();
    assert!(b.ram().iter().all(|&v| v == 0xff));
}

/// `dmdstatus_r` is one port fed by two boards (`se.c:752`): the display's busy
/// line and four status bits, and SSTO from the *sound* board in bit 1.
#[test]
fn the_status_byte_carries_the_sound_boards_ssto_line() {
    let mut b = Board::new();
    b.dmd_status = 0x80;
    assert_eq!(b.read(0x3700), 0x80);
    b.sound_sst0 = true;
    assert_eq!(b.read(0x3700), 0x82);
}

/// `$3500` is `dmdie_r`, the sound board's PLIN latch (`se.c:763`) — named for
/// a plasma display the design never had, and nothing to do with a DMD
/// interrupt enable.
#[test]
fn the_port_at_3500_is_the_sound_boards_latch() {
    let mut b = Board::new();
    assert_eq!(b.read(0x3500), 0);
    b.sound_plin = 0xa5;
    assert_eq!(b.read(0x3500), 0xa5);
}
