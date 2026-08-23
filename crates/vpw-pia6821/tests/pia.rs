//! MC6821 tests, against the Motorola datasheet.

use vpw_pia6821::{C2Mode, Pia};

/// Register indices (RS1:RS0).
const DATA_A: u8 = 0;
const CTRL_A: u8 = 1;
const DATA_B: u8 = 2;
const CTRL_B: u8 = 3;

/// Bit 2 of the control register: points the access at the data register
/// instead of the direction one.
const DDR_ACCESS: u8 = 0x00;
const DATA_ACCESS: u8 = 0x04;

/// Leaves port A with the given direction and the access pointing at data.
fn setup_port_a(pia: &mut Pia, ddr: u8) {
    pia.write(CTRL_A, DDR_ACCESS);
    pia.write(DATA_A, ddr);
    pia.write(CTRL_A, DATA_ACCESS);
}

fn setup_port_b(pia: &mut Pia, ddr: u8) {
    pia.write(CTRL_B, DDR_ACCESS);
    pia.write(DATA_B, ddr);
    pia.write(CTRL_B, DATA_ACCESS);
}

// --- Registers ------------------------------------------------------------------

#[test]
fn control_bit_2_picks_between_data_and_direction() {
    let mut pia = Pia::new();

    // With bit 2 at 0, the port access goes to the direction register.
    pia.write(CTRL_A, DDR_ACCESS);
    pia.write(DATA_A, 0xF0);
    assert_eq!(pia.port_a_direction(), 0xF0);
    assert_eq!(pia.read(DATA_A), 0xF0, "the direction is what gets read");

    // With bit 2 at 1, it goes to the data register.
    pia.write(CTRL_A, DATA_ACCESS);
    pia.write(DATA_A, 0xFF);
    assert_eq!(pia.port_a_direction(), 0xF0, "the direction did not change");
    assert_eq!(pia.port_a_output(), 0xF0, "only the output bits come out");
}

#[test]
fn the_select_ignores_the_high_bits() {
    // The chip only has two select lines; the board decodes the rest of the
    // address.
    let mut pia = Pia::new();
    setup_port_b(&mut pia, 0xFF);
    pia.write(DATA_B | 0xFC, 0x5A);
    assert_eq!(pia.port_b_output(), 0x5A);
}

#[test]
fn the_input_bits_read_the_outside_world() {
    let mut pia = Pia::new();
    setup_port_a(&mut pia, 0x0F); // low nibble output, high nibble input

    pia.write(DATA_A, 0x0A);
    pia.set_port_a_input(0xF0);

    assert_eq!(pia.read(DATA_A), 0xFA, "0xF0 of input + 0x0A of output");
    assert_eq!(
        pia.port_a_output(),
        0x0A,
        "outwards it only drives the low nibble"
    );
}

#[test]
fn control_bits_6_and_7_are_read_only() {
    let mut pia = Pia::new();
    pia.write(CTRL_A, 0xFF);
    // Bits 0-5 were written; the flags cannot be forced by software.
    assert_eq!(pia.read(CTRL_A) & 0xC0, 0x00);
    assert_eq!(pia.read(CTRL_A) & 0x3F, 0x3F);
}

#[test]
fn reset_leaves_everything_at_zero() {
    let mut pia = Pia::new();
    setup_port_a(&mut pia, 0xFF);
    pia.write(DATA_A, 0xAA);
    pia.write(CTRL_B, 0x3F);

    pia.reset();

    assert_eq!(pia.port_a_direction(), 0);
    assert_eq!(pia.port_a_output(), 0);
    assert_eq!(pia.read(CTRL_A), 0);
    assert_eq!(pia.read(CTRL_B), 0);
}

// --- Interrupts from C1 --------------------------------------------------------

#[test]
fn ca1_sets_the_flag_on_the_configured_edge() {
    let mut pia = Pia::new();

    // Bit 1 = 0: falling edge. Bit 0 = 1: interrupt enabled.
    pia.write(CTRL_A, DATA_ACCESS | 0x01);

    pia.set_ca1(true);
    assert!(!pia.irq_a(), "the rise is not the active transition");

    pia.set_ca1(false);
    assert!(pia.irq_a(), "the fall is");
    assert_eq!(pia.read(CTRL_A) & 0x80, 0x80, "the flag is in bit 7");
}

#[test]
fn ca1_can_trigger_on_a_rising_edge() {
    let mut pia = Pia::new();
    // Bit 1 = 1: rising edge.
    pia.write(CTRL_A, DATA_ACCESS | 0x03);

    pia.set_ca1(false);
    assert!(!pia.irq_a());

    pia.set_ca1(true);
    assert!(pia.irq_a());
}

#[test]
fn the_flag_is_set_even_when_the_interrupt_is_disabled() {
    let mut pia = Pia::new();
    // Bit 0 = 0: interrupt disabled.
    pia.write(CTRL_A, DATA_ACCESS);

    pia.set_ca1(true);
    pia.set_ca1(false);

    assert!(!pia.irq_a(), "the interrupt line does not go active");
    assert!(pia.irq_flags_a().0, "but the flag did get set");
    assert_eq!(
        pia.read(CTRL_A) & 0x80,
        0x80,
        "and it can be polled by software"
    );
}

#[test]
fn reading_the_port_clears_the_flags() {
    let mut pia = Pia::new();
    pia.write(CTRL_A, DATA_ACCESS | 0x01);
    pia.set_ca1(true);
    pia.set_ca1(false);
    assert!(pia.irq_a());

    pia.read(DATA_A);

    assert!(!pia.irq_a(), "reading the data port drops the interrupt");
    assert_eq!(pia.read(CTRL_A) & 0xC0, 0x00);
}

#[test]
fn reading_the_control_does_not_clear_the_flags() {
    let mut pia = Pia::new();
    pia.write(CTRL_A, DATA_ACCESS | 0x01);
    pia.set_ca1(true);
    pia.set_ca1(false);

    pia.read(CTRL_A);
    assert!(pia.irq_a(), "only the data port clears them");
}

#[test]
fn reading_one_port_does_not_clear_the_other_ones_flags() {
    let mut pia = Pia::new();
    pia.write(CTRL_A, DATA_ACCESS | 0x01);
    pia.write(CTRL_B, DATA_ACCESS | 0x01);
    pia.set_ca1(true);
    pia.set_ca1(false);
    pia.set_cb1(true);
    pia.set_cb1(false);

    pia.read(DATA_A);

    assert!(!pia.irq_a());
    assert!(pia.irq_b(), "half B is independent");
}

// --- Interrupts from C2 as an input ---------------------------------------------

#[test]
fn ca2_as_an_input_sets_its_own_flag() {
    let mut pia = Pia::new();
    // Bit 5 = 0: CA2 is an input. Bit 4 = 0: falling edge.
    // Bit 3 = 1: interrupt enabled.
    pia.write(CTRL_A, DATA_ACCESS | 0x08);
    assert!(matches!(pia.ca2_mode(), C2Mode::Input { .. }));

    pia.set_ca2(true);
    pia.set_ca2(false);

    assert!(pia.irq_a());
    assert_eq!(pia.read(CTRL_A) & 0x40, 0x40, "C2's flag is bit 6");
}

#[test]
fn ca2_as_an_output_raises_no_interrupts() {
    let mut pia = Pia::new();
    // Bit 5 = 1: CA2 is an output.
    pia.write(CTRL_A, DATA_ACCESS | 0x30);

    pia.set_ca2(true);
    pia.set_ca2(false);

    assert!(!pia.irq_a());
    assert!(!pia.irq_flags_a().1, "the flag does not even get set");
}

// --- C2 as an output -------------------------------------------------------------------

#[test]
fn ca2_manual_follows_bit_3() {
    let mut pia = Pia::new();

    // Bits 5 and 4 at 1: manual mode.
    pia.write(CTRL_A, DATA_ACCESS | 0x38); // bit 3 = 1
    assert_eq!(pia.ca2_mode(), C2Mode::Manual);
    assert!(pia.ca2_output());

    pia.write(CTRL_A, DATA_ACCESS | 0x30); // bit 3 = 0
    assert!(!pia.ca2_output());
}

#[test]
fn ca2_handshake_goes_down_on_reading_port_a() {
    let mut pia = Pia::new();
    // Bit 5 = 1, bit 4 = 0, bit 3 = 0: handshake.
    pia.write(CTRL_A, DATA_ACCESS | 0x20);
    assert_eq!(pia.ca2_mode(), C2Mode::Handshake);

    pia.read(DATA_A);
    assert!(!pia.ca2_output(), "the read pulls CA2 down");

    // It comes back up on the next active transition of CA1.
    pia.set_ca1(true);
    pia.set_ca1(false); // falling edge = active by default
    assert!(pia.ca2_output(), "CA1's active transition puts it back up");
}

#[test]
fn cb2_handshake_fires_on_a_write_not_on_a_read() {
    // This is the chip's asymmetry: CA2 goes with the read of port A,
    // CB2 with the write to port B.
    let mut pia = Pia::new();
    setup_port_b(&mut pia, 0xFF);
    pia.write(CTRL_B, DATA_ACCESS | 0x20); // handshake

    pia.read(DATA_B);
    assert!(
        !pia.cb2_output(),
        "it starts down, the read did not move it"
    );

    pia.set_cb1(true);
    pia.set_cb1(false);
    assert!(pia.cb2_output(), "CB1 brings it up");

    pia.read(DATA_B);
    assert!(pia.cb2_output(), "reading port B does not pull CB2 down");

    pia.write(DATA_B, 0x55);
    assert!(!pia.cb2_output(), "writing port B does pull it down");
}

#[test]
fn ca2_pulse_is_exposed_as_an_event() {
    let mut pia = Pia::new();
    // Bit 5 = 1, bit 4 = 0, bit 3 = 1: pulse mode.
    pia.write(CTRL_A, DATA_ACCESS | 0x28);
    assert_eq!(pia.ca2_mode(), C2Mode::Pulse);

    assert!(!pia.take_ca2_pulse(), "there has been no pulse yet");

    pia.read(DATA_A);
    assert!(pia.take_ca2_pulse(), "the read pulsed CA2");
    assert!(!pia.take_ca2_pulse(), "the event is consumed exactly once");
}

#[test]
fn cb2_pulse_goes_with_the_write() {
    let mut pia = Pia::new();
    setup_port_b(&mut pia, 0xFF);
    pia.write(CTRL_B, DATA_ACCESS | 0x28); // pulse

    pia.read(DATA_B);
    assert!(!pia.take_cb2_pulse(), "the read does not pulse CB2");

    pia.write(DATA_B, 0x01);
    assert!(pia.take_cb2_pulse());
}

// --- System 11 scenario -----------------------------------------------------------

#[test]
fn lamp_matrix_the_way_system_11_wires_it() {
    // System 11's PIA 1: PA = the lamp matrix return rows,
    // PB = the strobe columns. Both as outputs.
    let mut pia = Pia::new();
    setup_port_a(&mut pia, 0xFF);
    setup_port_b(&mut pia, 0xFF);

    pia.write(DATA_A, 0b0000_1001); // rows 0 and 3
    pia.write(DATA_B, 0b0000_0100); // column 2

    assert_eq!(pia.port_a_output(), 0b0000_1001);
    assert_eq!(pia.port_b_output(), 0b0000_0100);
}

#[test]
fn sound_handshake_the_way_system_11_uses_it() {
    // PIA 0: PA0-PA7 is the sound latch and CA2 the handshake towards the board.
    let mut pia = Pia::new();
    setup_port_a(&mut pia, 0xFF);

    // CA2 in manual mode, which is how System 11 drives it by hand.
    pia.write(CTRL_A, DATA_ACCESS | 0x38);
    assert!(pia.ca2_output(), "handshake at rest");

    pia.write(DATA_A, 0x42); // sound code
    pia.write(CTRL_A, DATA_ACCESS | 0x30); // pulls the strobe down

    assert_eq!(pia.port_a_output(), 0x42);
    assert!(!pia.ca2_output());

    pia.write(CTRL_A, DATA_ACCESS | 0x38); // brings it back up
    assert!(pia.ca2_output());
}

#[test]
fn six_pias_share_a_single_irq_line() {
    // On System 11 the twelve interrupt lines (A and B of each PIA) are wired
    // together to the 6808's IRQ.
    let mut pias: Vec<Pia> = (0..6).map(|_| Pia::new()).collect();
    for pia in &mut pias {
        pia.write(CTRL_A, DATA_ACCESS | 0x01);
    }

    assert!(!pias.iter().any(Pia::irq), "at rest, nobody is asking");

    pias[4].set_ca1(true);
    pias[4].set_ca1(false);
    assert!(pias.iter().any(Pia::irq), "one asking is enough");

    pias[4].read(DATA_A);
    assert!(!pias.iter().any(Pia::irq), "servicing it drops the line");
}

#[test]
fn the_c2_flag_is_hidden_when_c2_becomes_an_output() {
    // The datasheet: bit 6 reads zero while C2 is an output. The flag stays
    // stored, but stops being visible.
    let mut pia = Pia::new();

    // CA2 as an input, falling edge, interrupt enabled.
    pia.write(CTRL_A, DATA_ACCESS | 0x08);
    pia.set_ca2(true);
    pia.set_ca2(false);
    assert_eq!(
        pia.read(CTRL_A) & 0x40,
        0x40,
        "with CA2 as an input the flag is visible"
    );

    // Reconfiguring CA2 as a manual output hides it.
    pia.write(CTRL_A, DATA_ACCESS | 0x38);
    assert_eq!(
        pia.read(CTRL_A) & 0x40,
        0x00,
        "with CA2 as an output bit 6 reads zero"
    );
    assert!(!pia.irq_a(), "and it does not ask for an interrupt either");

    // Back as an input, it reappears.
    pia.write(CTRL_A, DATA_ACCESS | 0x08);
    assert_eq!(
        pia.read(CTRL_A) & 0x40,
        0x40,
        "the flag had never been cleared"
    );
}
