//! The Sound & Speech board, against a firmware written here.
//!
//! No System 80 set with one of these was to hand, so what a real ROM would
//! prove — that it boots and talks — is not proved. What these prove is
//! everything a real ROM would then depend on: that the two images land side
//! by side and the vectors point into them, that the first converter is the
//! loudspeaker, that a command arrives on port A with the strobe the card adds
//! itself, that the second converter drags the speech chip's clock and with it
//! the length of a word, and that the busy line interrupts when a phoneme
//! ends — which is the wire the whole of speech is sequenced on.

use vpw_bus::Bus;
use vpw_gts80::speech::SpeechBoard;

/// Where the test firmware sits in the 4 KB window, and where it runs from —
/// the copy above `$8000`, because that is the one the vectors point into.
const ENTRY: u16 = 0xF100;
const HANDLER: u16 = 0xF180;
/// The byte the handler counts in.
const COUNTER: usize = 0x20;

/// Builds the two 2 KB images a set carries, holding a firmware that slams
/// the loudspeaker's converter and counts interrupts.
fn firmware() -> (Vec<u8>, Vec<u8>) {
    let mut rom = vec![0xEA; 0x1000];
    let put = |rom: &mut Vec<u8>, at: usize, bytes: &[u8]| {
        rom[at..at + bytes.len()].copy_from_slice(bytes);
    };

    #[rustfmt::skip]
    let program: &[u8] = &[
        0xA2, 0x00,             // LDX #$00
        0x8E, COUNTER as u8, 0x00, // STX $0020   clear the count
        // The loop, at ENTRY + 5.
        0xA9, 0xFF,             // LDA #$FF
        0x8D, 0x00, 0x10,       // STA $1000   the loudspeaker, hard over
        0xA9, 0x00,             // LDA #$00
        0x8D, 0x00, 0x10,       // STA $1000   and hard back
        0x4C, (ENTRY + 5) as u8, ((ENTRY + 5) >> 8) as u8,
    ];
    put(&mut rom, 0x100, program);
    put(&mut rom, 0x180, &[0xEE, COUNTER as u8, 0x00, 0x40]); // INC / RTI

    // The vectors, in the top of the second image.
    put(&mut rom, 0xFFA, &[HANDLER as u8, (HANDLER >> 8) as u8]);
    put(&mut rom, 0xFFC, &[ENTRY as u8, (ENTRY >> 8) as u8]);
    put(&mut rom, 0xFFE, &[HANDLER as u8, (HANDLER >> 8) as u8]);

    let second = rom.split_off(0x800);
    (rom, second)
}

fn board() -> SpeechBoard {
    let (first, second) = firmware();
    SpeechBoard::new(&first, &second, 48_000)
}

/// How long the chip takes to say one phoneme, in samples.
fn spoken(board: &mut SpeechBoard, phoneme: u8) -> usize {
    // The board's own wiring inverts the phoneme bits on the way to the chip,
    // so a test that wants a particular one has to invert them too.
    board.mem.write(0x2000, phoneme ^ 0x3F);
    let mut samples = 0;
    while board.mem.votrax.busy() && samples < 48_000 {
        board.mem.votrax.sample();
        samples += 1;
    }
    samples
}

#[test]
fn the_two_images_land_side_by_side_under_the_vectors() {
    let board = board();
    assert_eq!(board.cpu.pc, ENTRY, "reset should land in the ROM's mirror");
    // The same four kilobytes answer below `$8000` too, because address line
    // 15 reaches nothing.
    let mut board = board;
    assert_eq!(board.mem.read(0x7100), board.mem.read(0xF100));
    assert_eq!(board.mem.read(0x7FFC), board.mem.read(0xFFFC));
}

#[test]
fn the_first_converter_is_the_loudspeaker() {
    let mut board = board();
    board.run(20_000.0);
    let audio = board.take_audio_at(48_000);
    assert!(!audio.is_empty());
    assert!(
        audio.iter().any(|s| s.abs() > 0.1),
        "a firmware slamming the converter end to end should be audible"
    );
    assert_eq!(board.produced(), audio.len() as u64);
}

#[test]
fn a_command_arrives_on_port_a_with_the_strobe_the_card_adds() {
    let mut board = board();
    board.command(0x05);
    assert_eq!(board.mem.riot.in_a, 0x85, "five, and the card's own bit 7");

    // Six bits reach the port, and the strobe is the *low nibble* being
    // something — which is how the firmware tells "say nothing" from "say
    // sound zero".
    board.command(0x30);
    assert_eq!(board.mem.riot.in_a, 0x30);
    board.command(0xFF);
    assert_eq!(board.mem.riot.in_a, 0xBF);
}

#[test]
fn the_second_converter_drags_the_speech_chips_clock() {
    let mut board = board();
    // `$7E` is the middle of the range and is what every message but one uses.
    board.mem.write(0x3000, 0x7E);
    assert_eq!(board.mem.votrax.clock(), 720_000);
    let ordinary = spoken(&mut board, 0x24); // AH, the longest vowel

    // And `$46` is where the falling "TILT" ends up, at about 398 kHz — the
    // two points flipprojets measured with a frequency meter.
    board.mem.write(0x3000, 0x46);
    assert!(board.mem.votrax.clock().abs_diff(398_000) < 3_000);
    let slowed = spoken(&mut board, 0x24);

    assert!(
        slowed > ordinary * 3 / 2,
        "a word at 398 kHz should take about 1.8 times as long: \
         {ordinary} samples became {slowed}"
    );
}

#[test]
fn the_busy_line_interrupts_when_a_phoneme_ends() {
    let mut board = board();
    board.run(20_000.0);
    let before = board.mem.riot.ram[COUNTER];

    // PA1, a pause, which is a phoneme like any other. Its length is not a
    // number anybody typed in: it comes out of the chip's own ROM through its
    // own counters, and lands at 186 ms.
    board.mem.write(0x2000, 0x3E ^ 0x3F);
    assert!(board.mem.votrax.busy(), "the chip should be speaking");

    // Half of it: still busy, and nothing has interrupted.
    board.run(f64::from(vpw_gts80::speech::CLOCK) * 0.09);
    assert!(board.mem.votrax.busy());
    assert_eq!(board.mem.riot.ram[COUNTER], before, "not yet");

    // And past the end of it.
    board.run(f64::from(vpw_gts80::speech::CLOCK) * 0.15);
    assert!(!board.mem.votrax.busy(), "the chip should have finished");
    assert!(
        board.mem.riot.ram[COUNTER] > before,
        "the falling busy line should have interrupted the processor"
    );
}

#[test]
fn port_b_carries_the_busy_line_and_the_test_button() {
    let mut board = board();
    // Nothing pressed, nothing speaking: the test button reads high and the
    // dips read back inverted, which with all of them off is all ones.
    assert_eq!(board.mem.read(0x0202), 0x7F);

    board.set_diagnostic(true);
    assert_eq!(board.mem.read(0x0202) & 0x40, 0, "the button pulls it down");
    board.set_diagnostic(false);

    board.mem.write(0x2000, 0x24 ^ 0x3F);
    assert_eq!(board.mem.read(0x0202) & 0x80, 0x80, "busy");
}

#[test]
fn a_write_into_the_rom_window_stops_the_voice() {
    let mut board = board();
    board.mem.write(0x3000, 0x7E);
    board.mem.write(0x2000, 0x24 ^ 0x3F); // AH, which the ROM holds for 244 ms
    // Long enough for the formants to have slid up: they move an eighth of the
    // way at a time, so the front of a phoneme is quiet.
    let loud = (0..6000)
        .map(|_| board.mem.votrax.sample().abs())
        .fold(0.0f32, f32::max);
    assert!(loud > 0.01, "the chip should be saying something");

    // The firmware's delay loop writes into its own ROM, and the card turns
    // each of those into "stop talking".
    board.mem.write(0x7123, 0x00);
    let quiet = (0..2000)
        .map(|_| board.mem.votrax.sample().abs())
        .fold(0.0f32, f32::max);
    assert_eq!(quiet, 0.0, "and it should have stopped");
}
