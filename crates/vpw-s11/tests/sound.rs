//! Tests for the System 11 sound board.

use vpw_s11::{Bus, ROM_START, SoundBoard, SoundBus, System11};

/// Register indices of the board's PIA, which lives at `$2000`.
const CTRL_A: u16 = 0x2001;
const DATA_B: u16 = 0x2002;
const CTRL_B: u16 = 0x2003;

/// A sound bus with both ROMs filled with a recognisable pattern.
fn bus_with_roms() -> SoundBus {
    let mut bus = SoundBus::new();
    // u21: each half with its own mark. Same for u22.
    let mut u21 = vec![0xA0; 0x8000];
    u21[0x4000..].fill(0xA1);
    let mut u22 = vec![0xB0; 0x8000];
    u22[0x4000..].fill(0xB1);
    assert!(bus.load_roms(&u21, &u22));
    bus
}

// --- Memory map --------------------------------------------------------------

#[test]
fn the_ram_takes_up_the_first_page() {
    let mut bus = SoundBus::new();
    bus.write(0x0000, 0x11);
    bus.write(0x0FFF, 0x22);

    assert_eq!(bus.read(0x0000), 0x11);
    assert_eq!(bus.read(0x0FFF), 0x22);
    assert_eq!(bus.last_unmapped, None);
}

#[test]
fn the_two_rom_windows_start_where_the_reset_needs_them() {
    // At power-up the low window sees the second half of u22 and the high one
    // the second half of u21.
    let mut bus = bus_with_roms();
    assert_eq!(bus.read(0x8000), 0xB1, "u22, second half");
    assert_eq!(bus.read(0xC000), 0xA1, "u21, second half");
}

#[test]
fn the_byte_at_1000_picks_the_banks() {
    let mut bus = bus_with_roms();

    // Bit 0 at 0: the low window moves to the first half of u22.
    // Bit 1 at 0: the high one moves to the first half of u21.
    bus.write(0x1000, 0x00);
    assert_eq!(bus.read(0x8000), 0xB0);
    assert_eq!(bus.read(0xC000), 0xA0);

    bus.write(0x1000, 0x01);
    assert_eq!(bus.read(0x8000), 0xB1, "bit 0 moved the low window");
    assert_eq!(bus.read(0xC000), 0xA0, "and it did not touch the high one");

    bus.write(0x1000, 0x02);
    assert_eq!(bus.read(0x8000), 0xB0);
    assert_eq!(bus.read(0xC000), 0xA1, "bit 1 moved the high one");
}

#[test]
fn a_rom_of_the_wrong_size_is_rejected() {
    let mut bus = SoundBus::new();
    assert!(!bus.load_roms(&[0; 0x4000], &[0; 0x8000]), "u21 too short");
    assert!(!bus.load_roms(&[0; 0x8000], &[0; 0x4000]), "u22 too short");
    assert!(bus.load_roms(&[0; 0x8000], &[0; 0x8000]));
}

#[test]
fn the_gaps_in_the_map_are_recorded() {
    let mut bus = SoundBus::new();
    bus.read(0x4000);
    assert_eq!(bus.last_unmapped, Some(0x4000));
}

// --- Audio chips ---------------------------------------------------------------

#[test]
fn port_b_drives_the_dac() {
    let mut bus = SoundBus::new();

    // Port B all output.
    bus.write(CTRL_B, 0x00);
    bus.write(DATA_B, 0xFF);
    bus.write(CTRL_B, 0x04);

    bus.write(DATA_B, 0x80);
    assert_eq!(bus.dac.sample(), 0.0, "rest is half scale");

    bus.write(DATA_B, 0x00);
    assert_eq!(bus.dac.sample(), -1.0);

    bus.write(DATA_B, 0xFF);
    assert!(bus.dac.sample() > 0.99);
}

#[test]
fn the_cvsd_is_driven_bit_by_bit_with_ca2_and_cb2() {
    // CB2 carries the data and CA2 the clock: the chip shifts on CA2's rising
    // edge.
    let mut bus = SoundBus::new();

    // A repeated 1 bit has to raise the signal.
    for _ in 0..100 {
        // CB2 high (manual mode, bit 3 of the control register).
        bus.write(CTRL_B, 0x04 | 0x38);
        // CA2 down and then up: that is when it shifts.
        bus.write(CTRL_A, 0x04 | 0x30);
        bus.write(CTRL_A, 0x04 | 0x38);
    }

    assert!(
        bus.cvsd.sample() > 0.5,
        "a hundred ones should raise the speech, it gave {}",
        bus.cvsd.sample()
    );
}

#[test]
fn the_cvsd_does_not_shift_without_a_rising_edge() {
    let mut bus = SoundBus::new();

    // Leave CA2 high and do not move it: no matter how much is written, there
    // is no edge.
    bus.write(CTRL_B, 0x04 | 0x38); // CB2 high
    bus.write(CTRL_A, 0x04 | 0x38); // CA2 up: a single shift
    let after_the_first = bus.cvsd.sample();

    for _ in 0..100 {
        bus.write(CTRL_A, 0x04 | 0x38); // still up
    }

    assert_eq!(
        bus.cvsd.sample(),
        after_the_first,
        "without an edge not one more bit goes in"
    );
}

#[test]
fn the_board_mixes_the_dac_and_the_speech() {
    let mut bus = SoundBus::new();
    assert_eq!(bus.sample(), 0.0, "at rest, silence");

    bus.write(CTRL_B, 0x00);
    bus.write(DATA_B, 0xFF);
    bus.write(CTRL_B, 0x04);
    bus.write(DATA_B, 0xFF);

    assert!(bus.sample() > 0.0, "the DAC alone is already audible");
}

// --- The sound CPU -------------------------------------------------------------

#[test]
fn the_sound_cpu_starts_from_its_rom() {
    // A program that writes the DAC, placed where the reset vector lands.
    //
    // The vector lives at $FFFE, that is, in the high window, which at
    // power-up sees u21 from its offset $4000. So $FFFE falls in u21[$7FFE].
    let mut u21 = vec![0x01u8; 0x8000]; // NOP
    let program_at = 0x4100; // -> $C100 in the map
    u21[program_at..program_at + 8].copy_from_slice(&[
        0x86, 0x40, // LDAA #$40
        0xB7, 0x20, 0x03, // STAA $2003  (CRB -> data; DDRB was left at 0)
        0x20, 0xFE, // BRA -2
        0x01,
    ]);
    u21[0x7FFE] = 0xC1;
    u21[0x7FFF] = 0x00;

    let mut board = SoundBoard::new();
    assert!(board.load_roms(&u21, &[0u8; 0x8000]));
    board.reset();

    assert_eq!(
        board.cpu.pc, 0xC100,
        "it took the vector from the high window"
    );

    board.run_cycles(100);
    assert_eq!(board.cpu.last_illegal, None);
    assert_eq!(board.bus.last_unmapped, None);
}

#[test]
fn run_until_does_not_accumulate_the_overshoot() {
    // It is the bug this had: asking for "run N more cycles" over and over
    // lets every instruction overshoot the target, and the overshoot adds up.
    // The board ended up running a third faster and the audio out of tune.
    let mut u21 = vec![0x01u8; 0x8000]; // NOP, 2 cycles
    u21[0x7FFE] = 0xC0;
    u21[0x7FFF] = 0x00;

    let mut board = SoundBoard::new();
    board.load_roms(&u21, &[0u8; 0x8000]);
    board.reset();

    // A thousand absolute targets 7 cycles apart, which is not a multiple of 2.
    for i in 1..=1000u64 {
        board.run_until(i * 7);
    }

    let expected = 7000;
    let error = board.cycle().abs_diff(expected);
    assert!(
        error <= 2,
        "it should stay glued to the target (it gave {} against {expected})",
        board.cycle()
    );
}

// --- Integrated with the main board ------------------------------------------------

#[test]
fn the_machine_runs_without_a_sound_board() {
    // It is the case that already worked: the main board does not depend on
    // the sound.
    let mut machine = System11::new();
    let mut rom = vec![0x01u8; 0xC000];
    let reset = (0xFFFE - ROM_START) as usize;
    rom[reset] = 0x40;
    rom[reset + 1] = 0x00;
    machine.board.load_rom(ROM_START, &rom).unwrap();
    machine.reset();

    machine.run_cycles(10_000);
    assert!(machine.sound.is_none());
    assert_eq!(machine.sound_sample(), 0.0, "no board, silence");
}

#[test]
fn the_two_cpus_run_in_lockstep() {
    let mut machine = System11::new();
    let mut rom = vec![0x01u8; 0xC000];
    let reset = (0xFFFE - ROM_START) as usize;
    rom[reset] = 0x40;
    rom[reset + 1] = 0x00;
    machine.board.load_rom(ROM_START, &rom).unwrap();

    let mut u21 = vec![0x01u8; 0x8000];
    u21[0x7FFE] = 0xC0;
    u21[0x7FFF] = 0x00;
    assert!(machine.load_sound_roms(&u21, &[0u8; 0x8000]));
    machine.reset();

    machine.run_cycles(1_000_000);

    let main = machine.cycle();
    let sound = machine.sound.as_ref().unwrap().cycle();
    assert!(
        main.abs_diff(sound) < 20,
        "both run at 1 MHz: main {main}, sound {sound}"
    );
}

#[test]
fn an_invalid_sound_rom_does_not_leave_the_machine_half_done() {
    let mut machine = System11::new();
    assert!(!machine.load_sound_roms(&[0; 100], &[0; 100]));
    assert!(
        machine.sound.is_none(),
        "no half-loaded board was left behind"
    );
}

#[test]
fn every_source_is_weighed_against_all_the_others_at_once() {
    // The original puts all five channels into one mixer, each with its own
    // level: two DACs at 25, two speech channels at 100 and the FM at 10
    // (`wmssnd.c:479-501`, `:731`). Mixing each board down on its own and then
    // averaging the two is a different weighting — it gives each board half the
    // total no matter what is inside it — and it is what the port was doing.
    let mut m = System11::new();
    m.sound = Some(vpw_s11::SoundBoard::new());
    m.sound_cs = Some(vpw_s11::CsSoundBoard::new());

    assert_eq!(m.audio_sample(), 0.0, "nothing driven yet");

    // The main board's effects channel alone, at full scale.
    m.sound.as_mut().unwrap().bus.dac.write(255);
    let full = m.sound.as_ref().unwrap().bus.dac.sample();
    let one_dac = m.audio_sample();

    // Its share is its own level over every level in the machine:
    // 25 / (25 + 100 + 25 + 100 + 10).
    let expected = full * 25.0 / 260.0;
    assert!(
        (one_dac - expected).abs() < 1e-6,
        "one effects channel should carry {expected:.5} of full scale, not {one_dac:.5}"
    );

    // And the sound board's effects channel is worth exactly the same, so the
    // two together are twice one.
    m.sound_cs.as_mut().unwrap().bus.dac.write(255);
    let both = m.audio_sample();
    assert!(
        (both - 2.0 * expected).abs() < 1e-6,
        "two of them should carry {:.5}, not {both:.5}",
        2.0 * expected
    );
}

#[test]
fn the_speech_goes_through_the_board_and_the_effects_do_not() {
    // The chip's output is not what reaches the speaker. On a System 11 it goes
    // through two op-amp stages first — a single-pole active low pass and then
    // a multiple-feedback one — and the machine driver names that pair
    // (`HC55516_FILTER_SYS11`, `wmssnd.c:481`). Without them the decoder's raw
    // staircase arrives with the words, which is what PinMAME blames for the
    // speech sounding "terrible" (`wmssnd.c:242`).
    //
    // The effects do not go through it: on the board they come off their own
    // output and only meet the speech at the summing amplifier.
    let mut m = System11::new();
    m.sound = Some(SoundBoard::new());

    // A run of ones takes the decoder up to a steady level.
    let cvsd = &mut m.sound.as_mut().unwrap().bus.cvsd;
    for _ in 0..200 {
        cvsd.clock(true);
    }
    let level = m.sound.as_ref().unwrap().bus.cvsd.sample();
    assert!(
        level > 0.1,
        "the decoder should be sitting somewhere: {level}"
    );

    // A filter has a memory, so the first sample out cannot already be the
    // steady answer. That is the whole difference between having one and not.
    let first = m.audio_sample();
    let settled_at = level * 100.0 / 125.0; // its level over the board's total
    assert!(
        (first - settled_at).abs() > 1e-4,
        "the speech came out unfiltered on the first sample: {first:.6}"
    );

    // And it settles on it, because these stages pass direct current at unit
    // gain: they take the hash off the top, they do not turn the speech down.
    let mut last = first;
    for _ in 0..4000 {
        last = m.audio_sample();
    }
    assert!(
        (last - settled_at).abs() < 1e-3,
        "it should settle at {settled_at:.6}, and it is at {last:.6}"
    );
}
