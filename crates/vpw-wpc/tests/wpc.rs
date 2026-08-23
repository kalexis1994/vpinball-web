//! Tests for the WPC board.

use vpw_wpc::{Bus, DMD_HEIGHT, DMD_WIDTH, Wpc, WpcBus, registers as r};

/// `BRA -2` for the 6809: a tight loop.
const LOOP: [u8; 2] = [0x20, 0xFE];

/// A ROM image of the given size, with the loop at `$8000` and the reset
/// vector pointing there.
///
/// The last 32 KB of the image are the ones that stay fixed at `$8000`.
fn rom(size: usize) -> Vec<u8> {
    let mut image = vec![0x12u8; size]; // NOP
    let fixed = size - 0x8000;
    image[fixed..fixed + 2].copy_from_slice(&LOOP);
    image[size - 2] = 0x80; // reset vector -> $8000
    image[size - 1] = 0x00;
    // Mark every 16 KB bank with its number, so they can be told apart.
    for bank in 0..size / 0x4000 {
        image[bank * 0x4000 + 0x100] = bank as u8;
    }
    image
}

fn machine(size: usize) -> Wpc {
    let mut wpc = Wpc::new();
    wpc.load_rom(&rom(size)).expect("the image is valid");
    wpc.reset();
    wpc
}

// --- Memory map ---------------------------------------------------------

#[test]
fn ram_reaches_up_to_3faf() {
    let mut bus = WpcBus::new();
    bus.write(0x0000, 0x11);
    bus.write(0x2FFF, 0x22);
    bus.write(0x3FAF, 0x33);

    assert_eq!(bus.read(0x0000), 0x11);
    assert_eq!(bus.read(0x2FFF), 0x22);
    assert_eq!(bus.read(0x3FAF), 0x33);
}

#[test]
fn the_last_32k_stay_fixed_at_8000() {
    let mut wpc = machine(0x8_0000);
    assert_eq!(
        wpc.cpu.pc, 0x8000,
        "the reset vector comes from the fixed part"
    );

    // The fixed bank is the second to last of the image: 512 KB are 32 banks,
    // so $8000 shows bank 30.
    assert_eq!(wpc.bus.read(0x8100), 30);
}

#[test]
fn the_4000_window_switches_banks() {
    let mut wpc = machine(0x8_0000);

    for bank in [0u8, 1, 7, 15, 31] {
        wpc.bus.write(0x3FB0 + r::ROMBANK as u16, bank);
        assert_eq!(wpc.bus.read(0x4100), bank, "bank {bank} did not show up");
    }
}

#[test]
fn the_page_mask_comes_from_the_image_size() {
    // A 256 KB image has 16 banks, so the register only uses four bits:
    // asking for bank 16 gives back bank 0.
    let mut wpc = machine(0x4_0000);
    wpc.bus.write(0x3FB0 + r::ROMBANK as u16, 16);
    assert_eq!(wpc.bus.read(0x4100), 0, "the extra bit is discarded");

    wpc.bus.write(0x3FB0 + r::ROMBANK as u16, 15);
    assert_eq!(wpc.bus.read(0x4100), 15);
}

#[test]
fn an_image_of_invalid_size_is_rejected() {
    let mut wpc = Wpc::new();
    assert!(wpc.load_rom(&[0u8; 1000]).is_err(), "too small");
    assert!(
        wpc.load_rom(&[0u8; 0x3_0000]).is_err(),
        "not a power of two"
    );
    assert!(wpc.load_rom(&rom(0x2_0000)).is_ok());
}

// --- The shift accelerator -----------------------------------------

#[test]
fn the_accelerator_computes_address_and_mask() {
    // The CPU leaves a base address and a bit number, and the chip gives back
    // the already shifted address and the bit mask. It saves the 6809 the
    // game's hottest loop: walking bit tables.
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    bus.write(base + r::SHIFTADRH as u16, 0x12);
    bus.write(base + r::SHIFTADRL as u16, 0x34);
    bus.write(base + r::SHIFTBIT as u16, 0x00);

    assert_eq!(bus.read(base + r::SHIFTADRH as u16), 0x12);
    assert_eq!(bus.read(base + r::SHIFTADRL as u16), 0x34);
    assert_eq!(bus.read(base + r::SHIFTBIT as u16), 0x01, "bit 0");

    // Bit 8 advances one byte and wraps back to bit 0.
    bus.write(base + r::SHIFTBIT as u16, 8);
    assert_eq!(bus.read(base + r::SHIFTADRL as u16), 0x35);
    assert_eq!(bus.read(base + r::SHIFTBIT as u16), 0x01);

    // Bit 5 does not move the address but it does shift the mask.
    bus.write(base + r::SHIFTBIT as u16, 5);
    assert_eq!(bus.read(base + r::SHIFTADRL as u16), 0x34);
    assert_eq!(bus.read(base + r::SHIFTBIT as u16), 0x20);
}

#[test]
fn the_accelerator_carries_into_the_high_byte() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    bus.write(base + r::SHIFTADRH as u16, 0x12);
    bus.write(base + r::SHIFTADRL as u16, 0xFF);
    bus.write(base + r::SHIFTBIT as u16, 8); // advance one byte

    assert_eq!(bus.read(base + r::SHIFTADRL as u16), 0x00);
    assert_eq!(
        bus.read(base + r::SHIFTADRH as u16),
        0x13,
        "it crossed a page"
    );
}

// --- Switches and lamps --------------------------------------------------------

#[test]
fn the_switch_matrix_is_multiplexed() {
    let mut wpc = machine(0x8_0000);
    let base = 0x3FB0u16;

    assert!(wpc.set_switch(46, true), "column 4, row 6");

    wpc.bus.write(base + r::SWCOLSELECT as u16, 1 << 0);
    assert_eq!(
        wpc.bus.read(base + r::SWROWREAD as u16),
        0,
        "column 1: nothing"
    );

    wpc.bus.write(base + r::SWCOLSELECT as u16, 1 << 3);
    assert_eq!(
        wpc.bus.read(base + r::SWROWREAD as u16),
        1 << 5,
        "column 4: row 6 shows up"
    );
}

#[test]
fn the_flipper_switches_arrive_inverted() {
    // They come in on their own line, outside the matrix, with negative logic.
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    assert_eq!(bus.read(base + r::FLIPPERS as u16), 0xFF, "none pressed");

    bus.asic.flipper_switches = 0b0000_0011;
    assert_eq!(bus.read(base + r::FLIPPERS as u16), 0b1111_1100);
}

#[test]
fn the_lamps_accumulate_over_the_strobe_pass() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    // The row and the column can be written in any order.
    bus.write(base + r::LAMPCOLUMN as u16, 1 << 0);
    bus.write(base + r::LAMPROW as u16, 0b0000_0011);
    bus.write(base + r::LAMPROW as u16, 0b0001_0000);
    bus.write(base + r::LAMPCOLUMN as u16, 1 << 1);

    bus.asic.refresh_lamps();
    assert!(bus.asic.lamp_lit(11), "column 1, row 1");
    assert!(bus.asic.lamp_lit(12));
    assert!(bus.asic.lamp_lit(25), "column 2, row 5");
    assert!(!bus.asic.lamp_lit(13));
}

#[test]
fn the_solenoids_are_split_into_four_groups() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    bus.write(base + r::SOLENOID2 as u16, 0x01); // solenoid 1
    bus.write(base + r::SOLENOID4 as u16, 0x01); // solenoid 9
    bus.write(base + r::SOLENOID3 as u16, 0x01); // solenoid 17
    bus.write(base + r::SOLENOID1 as u16, 0x01); // solenoid 25

    let s = bus.asic.solenoids();
    for bit in [0, 8, 16, 24] {
        assert!(s & (1 << bit) != 0, "bit {bit} is missing");
    }
}

// --- Interrupts ---------------------------------------------------------------

#[test]
fn the_periodic_interrupt_starts_off() {
    // At power-on the game has not enabled it yet: the boot sequence runs
    // without interrupts until the ROM decides to turn them on.
    let mut wpc = machine(0x8_0000);
    assert!(!wpc.bus.asic.periodic_irq_enabled());

    wpc.run_cycles(vpw_wpc::IRQ_PERIOD_CYCLES * 4);
    assert!(!wpc.bus.asic.irq(), "without enabling it, it does not fire");

    // Bit 2 of the zero cross register is the one that turns it on.
    wpc.bus.write(0x3FB0 + r::ZC_IRQ_ACK as u16, 0x04);
    assert!(wpc.bus.asic.periodic_irq_enabled());

    wpc.run_cycles(vpw_wpc::IRQ_PERIOD_CYCLES * 2);
    assert!(wpc.bus.asic.irq(), "now it does");
}

#[test]
fn writing_bit_7_acknowledges_the_interrupt() {
    let mut bus = WpcBus::new();
    bus.asic.raise_irq();
    assert!(bus.asic.irq());

    bus.write(0x3FB0 + r::ZC_IRQ_ACK as u16, 0x80);
    assert!(!bus.asic.irq(), "bit 7 clears it");
}

#[test]
fn the_zero_crossing_is_consumed_when_read() {
    let mut bus = WpcBus::new();
    bus.asic.tick_zero_cross();

    assert_eq!(bus.read(0x3FB0 + r::ZC_IRQ_ACK as u16) & 0x80, 0x80);
    assert_eq!(
        bus.read(0x3FB0 + r::ZC_IRQ_ACK as u16) & 0x80,
        0x00,
        "reading it consumes it"
    );
}

#[test]
fn the_interrupt_frequency_is_about_976_hz() {
    assert!((vpw_wpc::IRQ_FREQ_HZ - 976.5625).abs() < 0.01);
    assert_eq!(vpw_wpc::IRQ_PERIOD_CYCLES, 2048);
}

// --- The dot matrix display ----------------------------------------------------------

#[test]
fn the_cpu_sees_the_dmd_through_two_512_byte_windows() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    // Write into page 0 through the low window.
    bus.write(base + r::DMD_PAGE3800 as u16, 0);
    bus.write(0x3800, 0xAA);

    // And into page 1 through the high one.
    bus.write(base + r::DMD_PAGE3A00 as u16, 1);
    bus.write(0x3A00, 0x55);

    // Each window sees its own.
    assert_eq!(bus.read(0x3800), 0xAA);
    assert_eq!(bus.read(0x3A00), 0x55);

    // And with both pointing at the same page, they see the same thing.
    bus.write(base + r::DMD_PAGE3A00 as u16, 0);
    assert_eq!(bus.read(0x3A00), 0xAA, "both windows over page 0");
}

#[test]
fn the_dmd_draws_where_it_should() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;

    bus.write(base + r::DMD_PAGE3800 as u16, 0);
    bus.write(base + r::DMD_SHOWPAGE as u16, 0);

    // Bit 0 of the first byte is the top left dot.
    bus.write(0x3800, 0x01);
    assert!(bus.dmd.pixel(0, 0));
    assert!(!bus.dmd.pixel(1, 0));

    // Byte 16 starts the second row: 128 dots are 16 bytes.
    bus.write(0x3800 + 16, 0x01);
    assert!(bus.dmd.pixel(0, 1));
    assert_eq!(bus.dmd.lit_pixels(), 2);
}

#[test]
fn changing_the_visible_page_counts_a_frame() {
    let mut bus = WpcBus::new();
    let base = 0x3FB0u16;
    assert_eq!(bus.dmd.frames(), 0);

    bus.write(base + r::DMD_SHOWPAGE as u16, 1);
    bus.write(base + r::DMD_SHOWPAGE as u16, 2);
    assert_eq!(bus.dmd.frames(), 2);

    // Repeating the same page does not count.
    bus.write(base + r::DMD_SHOWPAGE as u16, 2);
    assert_eq!(bus.dmd.frames(), 2);
}

#[test]
fn the_display_is_128_by_32() {
    assert_eq!(DMD_WIDTH, 128);
    assert_eq!(DMD_HEIGHT, 32);
    let bus = WpcBus::new();
    assert!(
        !bus.dmd.pixel(DMD_WIDTH, 0),
        "out of range does not blow up"
    );
    assert!(!bus.dmd.pixel(0, DMD_HEIGHT));
    assert_eq!(bus.dmd.render_text().lines().count(), DMD_HEIGHT);
}

// --- The machine running ------------------------------------------------------------

#[test]
fn the_6809_runs_from_the_fixed_rom() {
    let mut wpc = machine(0x8_0000);
    wpc.run_cycles(10_000);

    assert_eq!(wpc.cpu.pc, 0x8000, "still in its loop");
    assert_eq!(wpc.cpu.last_illegal, None);
}

#[test]
fn simulated_time_advances_at_2_mhz() {
    let mut wpc = machine(0x8_0000);
    wpc.run_cycles(2_000_000);
    assert!((wpc.elapsed_seconds() - 1.0).abs() < 0.001);
}
