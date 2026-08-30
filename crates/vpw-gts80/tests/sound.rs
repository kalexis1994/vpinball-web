//! The sound card, checked against a firmware written here rather than one
//! from a machine.
//!
//! A real ROM proves the card plays; these prove the four things a real ROM
//! cannot tell you apart when it goes quiet — that the image is where the card
//! looks for it, that port A reaches the loudspeaker, that the game board's
//! command arrives beside the dip switches, and that the interrupt only fires
//! on the card that has the wire and only once the firmware has armed it.

use vpw_gts80::sound::{Card, SoundBoard};

/// Where the test firmware sits inside whichever image holds it.
const ENTRY: u16 = 0x0100;
/// And its interrupt handler.
const HANDLER: u16 = 0x0180;
/// The byte the handler counts in, in the sixty-four the card has.
const COUNTER: usize = 0x10;

/// Builds an image with a firmware that squares port A and counts interrupts.
///
/// `base` is where the image's *top* copy lands, because that is the copy the
/// vectors point into and so the one the processor runs from.
fn firmware(size: usize, base: u16) -> Vec<u8> {
    let mut rom = vec![0xEA; size];
    let start = base.wrapping_add(ENTRY);
    let put = |rom: &mut Vec<u8>, at: usize, bytes: &[u8]| {
        rom[at..at + bytes.len()].copy_from_slice(bytes);
    };

    #[rustfmt::skip]
    let program: &[u8] = &[
        0xA9, 0xFF,             // LDA #$FF
        0x8D, 0x01, 0x02,       // STA $0201   direction A: all output
        0xA9, 0x40,             // LDA #$40
        0x8D, 0x03, 0x02,       // STA $0203   direction B: bit 6 out
        0x8D, 0x02, 0x02,       // STA $0202   port B bit 6: arm the interrupt
        0x58,                   // CLI
        // The loop, at ENTRY + 13.
        0xA9, 0xFF,             // LDA #$FF
        0x8D, 0x00, 0x02,       // STA $0200   the ladder, hard over
        0xA9, 0x00,             // LDA #$00
        0x8D, 0x00, 0x02,       // STA $0200   and hard back
        0x4C, (start.wrapping_add(13)) as u8, (start.wrapping_add(13) >> 8) as u8,
    ];
    put(&mut rom, usize::from(ENTRY), program);
    put(
        &mut rom,
        usize::from(HANDLER),
        &[0xEE, COUNTER as u8, 0x00, 0x40],
    );

    // The vectors, in the top six bytes of the image.
    let handler = base.wrapping_add(HANDLER);
    put(&mut rom, size - 6, &[start as u8, (start >> 8) as u8]); // NMI, unused
    put(&mut rom, size - 4, &[start as u8, (start >> 8) as u8]); // RESET
    put(&mut rom, size - 2, &[handler as u8, (handler >> 8) as u8]);
    rom
}

/// A piggyback card: one 2 KB image, whose top copy is at `$F800`.
fn piggyback() -> SoundBoard {
    SoundBoard::piggyback(firmware(2048, 0xF800), 48_000)
}

#[test]
fn the_piggybacks_image_answers_where_the_vectors_point() {
    let board = piggyback();
    assert_eq!(board.cpu.pc, 0xF900, "reset should land in the top copy");
    assert_eq!(board.card(), Card::Piggyback);
}

#[test]
fn port_a_is_the_loudspeaker() {
    let mut board = piggyback();
    board.run(20_000.0);
    let audio = board.take_audio_at(48_000);
    assert!(!audio.is_empty(), "the card should have made samples");
    assert!(
        audio.iter().any(|s| s.abs() > 0.1),
        "a firmware slamming port A from end to end should be audible"
    );
    assert_eq!(board.produced(), audio.len() as u64);
}

#[test]
fn the_command_arrives_beside_the_dip_switches() {
    let mut board = piggyback();
    board.run(20_000.0);
    board.command(0x05);
    // Four bits of command, the two dips that reach the port, and the pin the
    // card ties high — so a bare command is never what the firmware reads.
    assert_eq!(board.mem.riot.in_b, 0xB5);

    // And only four bits of it: the fifth the game board borrows from a lamp
    // latch is for the speech card, which is not this one.
    board.command(0x1A);
    assert_eq!(board.mem.riot.in_b, 0xBA);
}

#[test]
fn a_command_interrupts_the_card_that_has_the_wire() {
    let mut board = piggyback();
    // Long enough for the firmware to have armed the interrupt.
    board.run(20_000.0);
    assert_eq!(board.mem.riot.ram[COUNTER], 0, "nothing yet");

    board.command(0x03);
    board.run(20_000.0);
    assert!(
        board.mem.riot.ram[COUNTER] > 0,
        "the handler should have run"
    );

    // A command of zero is the game board saying nothing, and says nothing.
    let so_far = board.mem.riot.ram[COUNTER];
    board.command(0x00);
    board.run(20_000.0);
    assert_eq!(board.mem.riot.ram[COUNTER], so_far);
}

#[test]
fn the_older_card_finds_its_two_images_and_polls_for_its_commands() {
    // A kilobyte of samples the firmware never reads, and the 6530's own
    // kilobyte above it holding the code — which is the copy at `$FC00`.
    let mut board = SoundBoard::sound(vec![0x00; 1024], firmware(1024, 0xFC00), 48_000);
    assert_eq!(board.cpu.pc, 0xFD00, "reset should land in the 6530's ROM");
    assert_eq!(board.card(), Card::Sound);

    board.run(20_000.0);
    assert!(board.produced() > 0);

    // This card has no interrupt line: the command waits on port B until the
    // firmware next looks at it.
    board.command(0x03);
    board.run(20_000.0);
    assert_eq!(board.mem.riot.in_b, 0xB3);
    assert_eq!(board.mem.riot.ram[COUNTER], 0, "no wire, no interrupt");
}
