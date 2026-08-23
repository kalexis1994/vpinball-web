//! Tests for the CS sound board.

use vpw_s11::{Bus, CsSoundBoard, CsSoundBus};

/// A ROM region with each bank marked with its own number, so it is possible
/// to see which one ended up in the window.
fn bus_with_roms() -> CsSoundBus {
    let mut bus = CsSoundBus::new();
    let images: Vec<Vec<u8>> = (0..4).map(|i| vec![0xA0 + i as u8; 0x8000]).collect();
    let refs: Vec<&[u8]> = images.iter().map(Vec::as_slice).collect();
    assert!(bus.load_roms(&refs));
    bus
}

// --- Memory map --------------------------------------------------------------

#[test]
fn the_ram_takes_up_the_first_8k() {
    let mut bus = CsSoundBus::new();
    bus.write(0x0000, 0x11);
    bus.write(0x1FFF, 0x22);

    assert_eq!(bus.read(0x0000), 0x11);
    assert_eq!(bus.read(0x1FFF), 0x22);
    assert_eq!(bus.last_unmapped, None);
}

#[test]
fn the_rom_bank_is_picked_from_7800() {
    let mut bus = bus_with_roms();

    // At start-up bank 0 is visible.
    assert_eq!(bus.read(0x8000), 0xA0);

    // Bit 2 picks the 32 KB half inside the 64 KB bank, and since each image
    // is mirrored, the same one is still visible.
    bus.write(0x7800, 0x04);
    assert_eq!(bus.read(0x8000), 0xA0);

    // Bits 0 and 1 jump between banks.
    bus.write(0x7800, 0x01);
    assert_eq!(bus.read(0x8000), 0xA1);
    bus.write(0x7800, 0x02);
    assert_eq!(bus.read(0x8000), 0xA2);
    bus.write(0x7800, 0x03);
    assert_eq!(bus.read(0x8000), 0xA3);
}

#[test]
fn an_image_of_the_wrong_size_is_rejected() {
    let mut bus = CsSoundBus::new();
    assert!(!bus.load_roms(&[&[0u8; 0x4000]]));
    assert!(bus.load_roms(&[&[0u8; 0x8000]]));
}

#[test]
fn the_gaps_in_the_map_are_recorded() {
    let mut bus = CsSoundBus::new();
    bus.read(0x3000);
    assert_eq!(bus.last_unmapped, Some(0x3000));
}

// --- The audio chips -------------------------------------------------------------

#[test]
fn the_cvsd_goes_through_two_different_addresses() {
    // Unlike the XS board, here the speech does not go through the PIA:
    // `$6000` sets the bit and lowers the clock, `$6800` raises it and that is
    // when it goes in.
    let mut bus = CsSoundBus::new();
    assert_eq!(bus.cvsd_bits(), 0);

    for _ in 0..100 {
        bus.write(0x6000, 0x01); // bit at 1, clock down
        bus.write(0x6800, 0x00); // clock up: it shifts
    }

    assert_eq!(bus.cvsd_bits(), 100);
    assert!(
        bus.cvsd.sample() > 0.5,
        "a hundred ones should raise the speech, it gave {}",
        bus.cvsd.sample()
    );
}

#[test]
fn the_cvsd_does_not_shift_without_an_edge() {
    let mut bus = CsSoundBus::new();
    bus.write(0x6000, 0x01);
    bus.write(0x6800, 0x00);
    assert_eq!(bus.cvsd_bits(), 1);

    // Raising a clock that was already up does not shift anything.
    for _ in 0..50 {
        bus.write(0x6800, 0x00);
    }
    assert_eq!(bus.cvsd_bits(), 1);
}

#[test]
fn the_ym2151_is_addressed_by_parity() {
    // The even address picks the register and the odd one carries the data.
    let mut bus = CsSoundBus::new();

    bus.write(0x2000, 0x20); // register $20: channel 0's algorithm
    bus.write(0x2001, 0xC7);
    assert_eq!(bus.ym.register(0x20), 0xC7);

    // The chip repeats itself across the whole window.
    bus.write(0x2010, 0x28);
    bus.write(0x2011, 0x4A);
    assert_eq!(bus.ym.register(0x28), 0x4A);
}

#[test]
fn the_ym_status_is_read_at_the_odd_address() {
    let mut bus = CsSoundBus::new();
    // Timer A at maximum speed, with its interrupt enabled.
    bus.write(0x2000, 0x10);
    bus.write(0x2001, 0xFF);
    bus.write(0x2000, 0x14);
    bus.write(0x2001, 0x05); // start A + enable its interrupt

    for _ in 0..100 {
        bus.ym.next_sample_mono();
    }

    assert_eq!(bus.read(0x2001) & 0x01, 0x01, "timer A's flag");
    assert!(bus.ym.irq());
}

#[test]
fn the_board_mixes_the_three_sources_with_different_gains() {
    // On the real board the three signals go through a summing amplifier with
    // different resistors: the speech is in charge and the FM sits further
    // back.
    let mut bus = CsSoundBus::new();
    assert_eq!(bus.sample(), 0.0, "at rest, silence");

    // Raise the speech to the maximum.
    for _ in 0..200 {
        bus.write(0x6000, 0x01);
        bus.write(0x6800, 0x00);
    }
    let speech_only = bus.sample();
    assert!(
        speech_only > 0.3,
        "the speech alone already weighs a fair bit ({speech_only})"
    );
}

// --- The YM's timer ------------------------------------------------------------------

#[test]
fn the_ym_timer_keeps_the_beat() {
    // Without timers the music sequencer does not advance: it is the time base
    // of the whole driver.
    let mut bus = CsSoundBus::new();

    // Timer B, which is the slow one.
    bus.write(0x2000, 0x12);
    bus.write(0x2001, 0xF0);
    bus.write(0x2000, 0x14);
    bus.write(0x2001, 0x0A); // start B + enable its interrupt

    assert!(!bus.ym.irq(), "it has not expired yet");

    // B's period with index $F0 is 16*(256-240) = 256 samples.
    for _ in 0..300 {
        bus.ym.next_sample_mono();
    }
    assert!(bus.ym.irq(), "it should have expired");

    // Clearing the flag disarms it until the next expiry.
    bus.write(0x2000, 0x14);
    bus.write(0x2001, 0x2A); // bit 5: clear B's flag
    assert!(!bus.ym.irq(), "clearing the flag drops the line");
}

// --- The board's CPU ---------------------------------------------------------------------

#[test]
fn the_6809_starts_from_the_banked_window() {
    // The reset vector lives at $FFFE, inside the ROM window.
    let mut image = vec![0x12u8; 0x8000]; // NOP
    image[0x7FFE] = 0x90;
    image[0x7FFF] = 0x00;

    let mut board = CsSoundBoard::new();
    assert!(board.load_roms(&[&image]));
    board.reset();

    assert_eq!(board.cpu.pc, 0x9000);
    board.run_cycles(1000);
    assert_eq!(board.cpu.last_illegal, None);
    assert_eq!(board.bus.last_unmapped, None);
}

#[test]
fn run_until_does_not_accumulate_the_overshoot() {
    let mut image = vec![0x12u8; 0x8000]; // NOP, 2 cycles
    image[0x7FFE] = 0x90;
    image[0x7FFF] = 0x00;

    let mut board = CsSoundBoard::new();
    board.load_roms(&[&image]);
    board.reset();

    for i in 1..=1000u64 {
        board.run_until(i * 7);
    }
    assert!(
        board.cycle().abs_diff(7000) <= 2,
        "it should stay glued to the target, it gave {}",
        board.cycle()
    );
}

#[test]
fn the_commands_arrive_on_the_nmi_and_not_on_the_irq() {
    // It is the wiring this had wrong and that left the board mute: the PIA's
    // B side goes to the 6809's NMI, not to its IRQ. A program that only has
    // an NMI routine has to wake up all the same.
    //
    //   9000: LDA #$01
    //   9002: STA $0100      ; marks "I ran the main program"
    //   9005: BRA -2
    //   9100: (NMI) INC $0101 ; marks "a command reached me"
    //   9104: RTI
    let mut image = vec![0x12u8; 0x8000];
    image[0x1000..0x1008].copy_from_slice(&[
        0x86, 0x01, // LDA #$01
        0xB7, 0x01, 0x00, // STA $0100
        0x20, 0xFE, // BRA -2
        0x12,
    ]);
    image[0x1100..0x1105].copy_from_slice(&[
        0x7C, 0x01, 0x01, // INC $0101
        0x3B, // RTI
        0x12,
    ]);
    image[0x7FFE] = 0x90; // RESET -> $9000
    image[0x7FFF] = 0x00;
    image[0x7FFC] = 0x91; // NMI -> $9100
    image[0x7FFD] = 0x00;

    let mut board = CsSoundBoard::new();
    board.load_roms(&[&image]);
    board.reset();

    // Enable the CB1 interrupt on the PIA (bit 0 of control register B) and
    // leave the access on data.
    board.bus.write(0x4003, 0x05);
    board.run_cycles(200);
    assert_eq!(board.bus.read(0x0100), 0x01, "the main program ran");
    assert_eq!(board.bus.read(0x0101), 0x00, "nothing has arrived yet");

    board.send_command(0x42);
    board.run_cycles(200);

    assert_eq!(
        board.bus.read(0x0101),
        0x01,
        "the command should have come in on the NMI"
    );
    assert_eq!(board.cpu.last_illegal, None);
}
