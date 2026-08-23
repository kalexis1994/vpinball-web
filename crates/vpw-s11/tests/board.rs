//! Tests for the System 11 memory map and timing.

use vpw_s11::{
    Bus, IRQ_HOLD_CYCLES, IRQ_PERIOD_CYCLES, PIA_BASES, ROM_START, SOLENOID_LATCH, System11, pia,
};

/// Builds a machine with a program at `$4000` and the reset vector pointing
/// there. A System 11 ROM starts right at `$4000`.
fn boot(program: &[u8]) -> System11 {
    let mut machine = System11::new();

    let mut rom = vec![0x01; 0xC000]; // NOP by default
    rom[..program.len()].copy_from_slice(program);
    // RESET vector to the start of the ROM.
    let reset = (0xFFFE - ROM_START) as usize;
    rom[reset] = 0x40;
    rom[reset + 1] = 0x00;

    machine
        .board
        .load_rom(ROM_START, &rom)
        .expect("the ROM fits");
    machine.reset();
    machine
}

// --- Memory map --------------------------------------------------------------

#[test]
fn the_ram_runs_from_0000_to_1fff() {
    let mut machine = System11::new();

    machine.board.write(0x0000, 0xAA);
    machine.board.write(0x1FFF, 0xBB);

    assert_eq!(machine.board.read(0x0000), 0xAA);
    assert_eq!(machine.board.read(0x1FFF), 0xBB);
    assert_eq!(machine.board.last_unmapped, None);
}

#[test]
fn the_rom_is_read_only() {
    let mut machine = System11::new();
    machine.board.load_rom(ROM_START, &[0x12, 0x34]).unwrap();

    assert_eq!(machine.board.read(0x4000), 0x12);
    assert_eq!(machine.board.read(0x4001), 0x34);

    // Writing to it does nothing and does not count as an access outside the
    // map: there is code that sweeps memory without discriminating.
    machine.board.write(0x4000, 0xFF);
    assert_eq!(machine.board.read(0x4000), 0x12);
    assert_eq!(machine.board.last_unmapped, None);
}

#[test]
fn the_48k_rom_loads_at_4000_and_8000() {
    let mut machine = System11::new();
    let u26 = vec![0xA1; 0x4000]; // 16 KB
    let u27 = vec![0xB2; 0x8000]; // 32 KB

    machine.load_rom_48(&u26, &u27).unwrap();

    assert_eq!(machine.board.read(0x4000), 0xA1);
    assert_eq!(machine.board.read(0x7FFF), 0xA1, "u26 reaches up to $7FFF");
    assert_eq!(machine.board.read(0x8000), 0xB2, "u27 starts at $8000");
    assert_eq!(machine.board.read(0xFFFF), 0xB2, "and reaches the end");
}

#[test]
fn a_rom_that_does_not_fit_is_an_error() {
    let mut machine = System11::new();
    assert!(
        machine.board.load_rom(0x2000, &[0; 4]).is_err(),
        "below the ROM"
    );
    assert!(
        machine.board.load_rom(0xF000, &[0; 0x2000]).is_err(),
        "runs past the end"
    );
}

#[test]
fn the_six_pias_respond_at_their_address() {
    let mut machine = System11::new();

    for (index, &base) in PIA_BASES.iter().enumerate() {
        // Port B all output, with a different pattern per PIA.
        let pattern = 0x11 * (index as u8 + 1);
        machine.board.write(base + 3, 0x00); // CRB -> direction
        machine.board.write(base + 2, 0xFF); // DDRB
        machine.board.write(base + 3, 0x04); // CRB -> data
        machine.board.write(base + 2, pattern);

        assert_eq!(
            machine.board.pias[index].port_b_output(),
            pattern,
            "PIA {index} at {base:#06X} did not receive its own"
        );
    }

    assert_eq!(machine.board.last_unmapped, None);
}

#[test]
fn each_pia_decodes_only_four_addresses() {
    let mut machine = System11::new();

    // $2104 is already outside PIA 0's window and nobody decodes it.
    machine.board.read(0x2104);
    let miss = machine
        .board
        .last_unmapped
        .expect("it should have been recorded");
    assert_eq!(miss.address, 0x2104);
    assert!(!miss.write);
}

#[test]
fn the_solenoid_latch_is_at_2200() {
    let mut machine = System11::new();

    machine.board.write(SOLENOID_LATCH, 0b1010_0101);
    assert_eq!(
        machine.board.solenoids.active(),
        0b1010_0101,
        "instantaneous state"
    );

    // The firings are published when the sweep closes: 0b1010_0101 is
    // solenoids 1, 3, 6 and 8.
    machine.board.refresh_outputs();
    for number in [1, 3, 6, 8] {
        assert!(
            machine.board.solenoids.did_fire(number),
            "solenoid {number}"
        );
    }
    assert!(!machine.board.solenoids.did_fire(2));

    // It is write only: reading it falls outside the map.
    machine.board.read(SOLENOID_LATCH);
    assert_eq!(
        machine.board.last_unmapped.map(|a| a.address),
        Some(SOLENOID_LATCH)
    );
}

// --- Timing ------------------------------------------------------------------

#[test]
fn the_periodic_interrupt_comes_every_928_cycles() {
    let mut machine = boot(&[0x01]); // NOP in a loop

    assert!(machine.periodic_irq(), "at reset it starts asserted");

    machine.run_cycles(IRQ_HOLD_CYCLES);
    assert!(!machine.periodic_irq(), "after 32 cycles it is released");

    // And it does not come back until the period completes.
    machine.run_cycles(IRQ_PERIOD_CYCLES - IRQ_HOLD_CYCLES - 8);
    assert!(!machine.periodic_irq());

    machine.run_cycles(16);
    assert!(machine.periodic_irq(), "the next period started");
}

#[test]
fn the_interrupt_frequency_is_about_1078_hz() {
    // S11_IRQFREQ = 1000000/928
    let expected = 1_000_000.0 / IRQ_PERIOD_CYCLES as f64;
    assert!((expected - 1077.6).abs() < 0.1, "got {expected} Hz");
}

#[test]
fn the_irq_is_the_or_of_the_periodic_one_and_the_pias() {
    let mut machine = boot(&[0x01]);

    // Get it out of the periodic interrupt's window.
    machine.run_cycles(IRQ_HOLD_CYCLES + 4);
    assert!(!machine.irq_line(), "at rest nobody is asking");

    // One PIA asking is enough to raise the line.
    let base = PIA_BASES[pia::SWITCHES];
    machine.board.write(base + 1, 0x05); // CRA: data + CA1 IRQ enabled
    machine.board.pias[pia::SWITCHES].set_ca1(true);
    machine.board.pias[pia::SWITCHES].set_ca1(false);

    assert!(machine.board.pia_irq());
    assert!(machine.irq_line());
}

// --- Peripherals ---------------------------------------------------------------

#[test]
fn the_switch_matrix_is_multiplexed() {
    let mut machine = System11::new();
    let base = PIA_BASES[pia::SWITCHES];

    // PB as output (columns), PA stays input (rows).
    machine.board.write(base + 3, 0x00);
    machine.board.write(base + 2, 0xFF);
    machine.board.write(base + 3, 0x04);
    machine.board.write(base + 1, 0x04); // CRA -> data

    // Switch 30: the sixth of column 4, since the numbering runs straight
    // through eight at a time.
    machine.board.switches.set(30, true);

    // With column 1 driven nothing is seen.
    machine.board.write(base + 2, 1 << 0);
    machine.board.refresh_inputs();
    assert_eq!(machine.board.read(base), 0x00);

    // With column 4 driven row 6 shows up.
    machine.board.write(base + 2, 1 << 3);
    machine.board.refresh_inputs();
    assert_eq!(machine.board.read(base), 1 << 5);
}

#[test]
fn the_diagnostic_led_is_bit_4_of_pia2() {
    let mut machine = System11::new();
    let base = PIA_BASES[pia::DISPLAY_SELECT];

    machine.board.write(base + 1, 0x00); // CRA -> direction
    machine.board.write(base, 0xFF); // DDRA
    machine.board.write(base + 1, 0x04); // CRA -> data

    machine.board.write(base, 0x00);
    assert!(!machine.board.diagnostic_led());

    machine.board.write(base, 0x10);
    assert!(machine.board.diagnostic_led());
}

#[test]
fn the_sound_latch_and_the_high_solenoids_come_out_of_pia0() {
    let mut machine = System11::new();
    let base = PIA_BASES[pia::SOUND_SOLENOIDS];

    for (control, data) in [(base + 1, base), (base + 3, base + 2)] {
        machine.board.write(control, 0x00);
        machine.board.write(data, 0xFF);
        machine.board.write(control, 0x04);
    }

    machine.board.write(base, 0x42); // sound code
    machine.board.write(base + 2, 0x81); // solenoids 9 and 16

    machine.board.refresh_outputs();
    assert_eq!(machine.board.sound_latch(), 0x42);
    assert_eq!(
        machine.board.solenoids.active() >> 8,
        0x81,
        "solenoids 9-16"
    );
    assert!(machine.board.solenoids.did_fire(9));
    assert!(machine.board.solenoids.did_fire(16));
}

// --- The CPU running on the board -----------------------------------------------

#[test]
fn the_6808_runs_from_the_rom_and_writes_to_the_ram() {
    // 4000: LDS  #$1FFF
    // 4003: LDAA #$7E
    // 4005: STAA $0100
    // 4008: BRA  -2
    let machine_program = [
        0x8E, 0x1F, 0xFF, //
        0x86, 0x7E, //
        0xB7, 0x01, 0x00, //
        0x20, 0xFE,
    ];
    let mut machine = boot(&machine_program);

    assert_eq!(machine.cpu.pc, 0x4000, "it started from the reset vector");

    for _ in 0..4 {
        machine.step();
    }

    assert_eq!(machine.board.ram()[0x0100], 0x7E);
    assert_eq!(machine.cpu.last_illegal, None);
    assert_eq!(machine.board.last_unmapped, None);
}

#[test]
fn the_simulated_time_advances_at_1_mhz() {
    let mut machine = boot(&[0x20, 0xFE]); // BRA -2, 4 cycles per pass
    machine.run_cycles(1_000_000);

    assert!(
        (machine.elapsed_seconds() - 1.0).abs() < 0.001,
        "got {} s",
        machine.elapsed_seconds()
    );
}

// --- Display wiring by generation ----------------------------------------------

#[test]
fn an_s11b_sends_the_segments_inverted() {
    // It is the difference that makes reading an 11B with an 11A's
    // configuration return garbage instead of text: on the 11B a zero lights.
    use vpw_s11::DisplayConfig;

    let zero_pattern = 0b0011_1111u8; // segments a-f: a "0"

    let mut a = System11::new();
    a.set_display_config(DisplayConfig::S11A);
    let base = PIA_BASES[pia::DISPLAY_DATA];
    let sel = PIA_BASES[pia::DISPLAY_SELECT];
    for m in [&mut a] {
        // Data ports as output.
        for (control, data) in [(base + 1, base), (base + 3, base + 2), (sel + 1, sel)] {
            m.board.write(control, 0x00);
            m.board.write(data, 0xFF);
            m.board.write(control, 0x04);
        }
        m.board.write(sel, 1); // digit 1
    }
    a.board.write(base + 2, zero_pattern);
    a.board.displays.refresh();
    assert_eq!(
        &a.upper_display()[0..1],
        "0",
        "the 11A takes the data as it is"
    );

    let mut b = System11::new();
    b.set_display_config(DisplayConfig::S11B);
    for (control, data) in [(base + 1, base), (base + 3, base + 2), (sel + 1, sel)] {
        b.board.write(control, 0x00);
        b.board.write(data, 0xFF);
        b.board.write(control, 0x04);
    }
    b.board.write(sel, 1);
    b.board.write(base + 2, !zero_pattern);
    b.board.displays.refresh();
    assert_eq!(&b.upper_display()[1..2], "0", "the 11B inverts it");
}

#[test]
fn the_s11a_has_14_characters_and_the_s11b_16() {
    use vpw_s11::DisplayConfig;

    let mut machine = System11::new();
    machine.set_display_config(DisplayConfig::S11A);
    assert_eq!(machine.upper_display().chars().count(), 14);
    assert_eq!(machine.lower_display().chars().count(), 14);

    machine.set_display_config(DisplayConfig::S11B);
    assert_eq!(machine.upper_display().chars().count(), 16);
    assert_eq!(machine.lower_display().chars().count(), 16);
}

#[test]
fn on_an_s11b_the_bottom_row_is_alphanumeric() {
    // On the 11A the bottom row is seven-segment and its high byte does not
    // exist; on the 11B it is alphanumeric and that byte arrives through PIA
    // 5's port A.
    use vpw_s11::DisplayConfig;

    let mut machine = System11::new();
    machine.set_display_config(DisplayConfig::S11B);

    let sel = PIA_BASES[pia::DISPLAY_SELECT];
    let widget = PIA_BASES[pia::WIDGET];
    for (control, data) in [(sel + 1, sel), (sel + 3, sel + 2), (widget + 1, widget)] {
        machine.board.write(control, 0x00);
        machine.board.write(data, 0xFF);
        machine.board.write(control, 0x04);
    }
    machine.board.write(sel, 0); // digit 0

    // An "I" needs segments a, d, j and p: two in each byte.
    let low = 0b0000_1001u8; // a and d
    let high = 0b0010_0010u8; // j and p
    machine.board.write(sel + 2, !low);
    machine.board.write(widget, !high);
    machine.board.displays.refresh();

    assert_eq!(
        &machine.lower_display()[0..1],
        "I",
        "the high byte has to arrive through PIA 5"
    );
}
