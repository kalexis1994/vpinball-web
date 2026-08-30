//! The System 80B alphanumeric displays.
//!
//! No 80B set was to hand, so what a real ROM would prove — that a game spells
//! `BOUNTY HUNTER` across the glass — is not proved. What these prove is
//! everything it would rest on: that the byte the game assembles a nibble at a
//! time comes out whole, that the Rockwell 10939 takes the commands it takes
//! and fills its columns in order, that its character set is the chip's and
//! not ASCII, and that the segments come out in the order the rest of this
//! port draws them in.

use vpw_bus::Bus;
use vpw_gts80::alpha::{Alphanumeric, COLUMNS, Rockwell10939, glyph, seg};
use vpw_gts80::board::Generation;
use vpw_gts80::{Board, System80};

/// Sets a chip up the way a game does and spells something into it.
fn spelling(text: &str) -> Rockwell10939 {
    let mut chip = Rockwell10939::new();
    // Twenty columns, then start.
    chip.load(0x01);
    chip.load(0x80 | COLUMNS as u8);
    chip.load(0x01);
    chip.load(0x0E);
    for byte in text.bytes() {
        chip.load(byte);
    }
    chip
}

#[test]
fn a_control_word_is_announced_by_the_byte_before_it() {
    let mut chip = Rockwell10939::new();
    assert!(!chip.running(), "it powers up halted");

    // A bare `$0E` is the letter it happens to be, not the start command.
    chip.load(0x01);
    chip.load(0x80 | 20);
    chip.load(0x0E);
    assert!(!chip.running());

    // Announced, it is the command.
    chip.load(0x01);
    chip.load(0x0E);
    assert!(chip.running(), "and now it is drawing");
}

#[test]
fn characters_fill_the_columns_in_order_and_wrap() {
    let chip = spelling("BOUNTY HUNTER");
    assert_eq!(chip.text(), "BOUNTY HUNTER       ");

    // Twenty columns and twenty-one characters: the last one lands back at the
    // front, because the column counter is taken modulo the digit count.
    let chip = spelling("ABCDEFGHIJKLMNOPQRSTU");
    assert_eq!(chip.text(), "UBCDEFGHIJKLMNOPQRST");
}

#[test]
fn a_chip_that_has_not_been_told_its_width_writes_nothing() {
    // The digit counter powers up at zero and the chip drives nothing until a
    // game says otherwise, so a character has nowhere to land.
    let mut chip = Rockwell10939::new();
    for byte in b"HELLO" {
        chip.load(*byte);
    }
    assert_eq!(chip.text().trim(), "");
}

#[test]
fn the_write_column_can_be_moved() {
    let mut chip = spelling("");
    chip.load(0x01);
    chip.load(0xC0 | 5); // next write at column five
    for byte in b"HI" {
        chip.load(*byte);
    }
    assert_eq!(chip.text(), "     HI             ");
}

#[test]
fn brightness_is_the_slot_the_duty_fills() {
    let mut chip = spelling("");
    // Sixty-four clocks a digit at power-on, and a duty of zero is the chip's
    // own full brightness rather than darkness.
    assert_eq!(chip.brightness(), 1.0);

    chip.load(0x01);
    chip.load(0x40 | 31); // half of the sixty-four
    assert!((chip.brightness() - 0.5).abs() < 0.01);

    chip.load(0x01);
    chip.load(0x05); // sixteen clocks a digit, so the same duty is twice as
    assert!(
        (chip.brightness() - 2.0).abs() < 0.01,
        "long a share of a shorter slot"
    );
}

/// The character set is the chip's own, not ASCII: a score with thousands
/// separators needs the comma inside the digit's own cell.
#[test]
fn the_digits_come_in_three_flavours_of_punctuation() {
    for (code, expect) in [
        (0x02u8, seg::COMMA),
        (0x0C, seg::DOT),
        (0x16, seg::COMMA | seg::DOT),
    ] {
        let (c, bits) = glyph(code);
        assert_eq!(c, '0', "code {code:#04X} draws a nought");
        assert_eq!(bits & (seg::COMMA | seg::DOT), expect);
        // And the nought itself is there under the punctuation.
        assert_eq!(bits & !(seg::COMMA | seg::DOT), glyph(b'0').1);
    }
    // Nine, three times over.
    assert_eq!(glyph(0x0B).0, '9');
    assert_eq!(glyph(0x15).0, '9');
    assert_eq!(glyph(0x1F).0, '9');

    assert_eq!(glyph(0x00), (' ', 0));
    assert_eq!(glyph(b'A').0, 'A');
}

/// The segment order has to be the one the rest of this port draws in, because
/// a table's own segment lights are numbered by it. Nothing here would fail if
/// it drifted — the display would simply come out as nonsense strokes.
#[test]
fn the_segments_are_in_the_same_order_as_every_other_machine_here() {
    use vpw_s11::display::seg as s11;
    assert_eq!(seg::A, s11::A);
    assert_eq!(seg::B, s11::B);
    assert_eq!(seg::C, s11::C);
    assert_eq!(seg::D, s11::D);
    assert_eq!(seg::E, s11::E);
    assert_eq!(seg::F, s11::F);
    assert_eq!(seg::G, s11::G);
    assert_eq!(seg::COMMA, s11::COMMA);
    assert_eq!(seg::H, s11::H);
    assert_eq!(seg::J, s11::J);
    assert_eq!(seg::K, s11::K);
    assert_eq!(seg::M, s11::M);
    assert_eq!(seg::N, s11::N);
    assert_eq!(seg::P, s11::P);
    assert_eq!(seg::R, s11::R);
    assert_eq!(seg::DOT, s11::DOT);

    // And the letters have to draw the same way, or the same word would read
    // differently on a Gottlieb and a Williams.
    for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ -".chars() {
        let ours = vpw_gts80::alpha::segments(c);
        let theirs = vpw_s11::display::segments_to_char(ours);
        // Two pairs light exactly the same strokes, so the information needed
        // to tell them apart does not exist on either display. The digit wins,
        // because these things spend their lives showing scores.
        let expect = match c {
            'O' => '0',
            'S' => '5',
            other => other,
        };
        assert_eq!(
            theirs, expect,
            "{c} draws something the other reads as {theirs}"
        );
    }
}

/// The awkward part: the byte reaches the chips a nibble at a time, on two
/// different edges of two different ports.
#[test]
fn the_board_assembles_a_byte_out_of_two_nibbles() {
    let mut alpha = Alphanumeric::new();

    let mut send = |value: u8| {
        // Low nibble on the falling edge of port A bit 4.
        alpha.ports(0x10, value & 0x0F);
        alpha.ports(0x00, value & 0x0F);
        // High nibble on the rising edge of bit 5.
        alpha.ports(0x00, value >> 4);
        alpha.ports(0x20, value >> 4);
        // And a rising edge on port B bit 4 hands it to the first chip.
        alpha.ports(0x20, 0x00);
        alpha.ports(0x20, 0x10);
    };

    send(0x01);
    send(0x80 | 20);
    send(0x01);
    send(0x0E);
    for byte in b"HI" {
        send(*byte);
    }
    assert_eq!(alpha.text().0, "HI                  ");
    assert_eq!(alpha.text().1.trim(), "", "the other chip heard nothing");
}

#[test]
fn the_two_strobes_go_to_the_two_rows() {
    let mut alpha = Alphanumeric::new();
    let mut send = |value: u8, strobe: u8| {
        alpha.ports(0x10, value & 0x0F);
        alpha.ports(0x00, value & 0x0F);
        alpha.ports(0x00, value >> 4);
        alpha.ports(0x20, value >> 4);
        alpha.ports(0x20, 0x00);
        alpha.ports(0x20, strobe);
    };
    for (strobe, text) in [(0x10u8, b"TOP"), (0x20, b"LOW")] {
        send(0x01, strobe);
        send(0x80 | 20, strobe);
        send(0x01, strobe);
        send(0x0E, strobe);
        for byte in text {
            send(*byte, strobe);
        }
    }
    let (top, bottom) = alpha.text();
    assert_eq!(top.trim_end(), "TOP");
    assert_eq!(bottom.trim_end(), "LOW");
}

/// A board built for an 80B has the alphanumeric screen and no BCD latches,
/// and one 8 KB system ROM where the older boards have two of 4 KB.
#[test]
fn an_80b_board_has_the_alphanumeric_screen() {
    let mut mem = System80::new_80b(vec![0xEA; 2048], vec![0x11; 8192]);
    assert_eq!(mem.generation, Generation::Alphanumeric);
    assert!(mem.alpha().is_some());
    assert!(
        mem.displays().is_none(),
        "there are no BCD latches on an 80B"
    );

    // The 8 KB lands where the two 4 KB images did, vectors and all.
    assert_eq!(mem.read(0x2000), 0x11);
    assert_eq!(mem.read(0x3FFF), 0x11);
    assert_eq!(mem.read(0xFFFC), 0x11, "the reset vector is still in there");
}

/// A 4 KB game ROM does not fit the 2 KB window, so the board puts its halves
/// in the two copies of the map that address line 15 tells apart — the one
/// place on any of these boards where that line is decoded.
#[test]
fn a_four_kilobyte_game_rom_is_read_in_two_halves() {
    let mut game = vec![0xAA; 4096];
    game[0x800..].fill(0xBB);
    let mut mem = System80::new_80b(game, vec![0x11; 8192]);

    assert_eq!(mem.read(0x1000), 0xAA, "the first half, low");
    assert_eq!(mem.read(0x5000), 0xAA, "and again where A14 repeats it");
    assert_eq!(mem.read(0x9000), 0xBB, "the second half, above A15");
    assert_eq!(mem.read(0xD000), 0xBB);

    // A 2 KB game ROM has no second half and repeats everywhere, as before.
    let mut small = System80::new_80b(vec![0xAA; 2048], vec![0x11; 8192]);
    assert_eq!(small.read(0x1000), 0xAA);
    assert_eq!(small.read(0x9000), 0xAA);
}

#[test]
fn a_board_reports_whichever_display_it_has() {
    let bcd = Board::new(vec![0xEA; 2048], vec![0x11; 4096], vec![0x22; 4096]);
    assert_eq!(bcd.segments().len(), 32, "two banks of sixteen BCD digits");

    let alpha = Board::new_80b(vec![0xEA; 2048], vec![0x11; 8192]);
    assert_eq!(alpha.segments().len(), 40, "two rows of twenty characters");
    let (top, bottom) = alpha.text();
    assert_eq!(top.len(), 20);
    assert_eq!(bottom.len(), 20);
}
