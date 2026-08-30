//! The 6532, one behaviour at a time.
//!
//! Weighted towards the timer, because that is the part a machine's whole
//! rhythm hangs off and the part with a second half — what it does *after* it
//! passes zero — that a model can plausibly leave out and not notice until a
//! ROM measures it.

use vpw_riot::Riot;

/// The register addresses, written the way the chip decodes them.
const DRA: u16 = 0x00;
const DDRA: u16 = 0x01;
const DRB: u16 = 0x02;
const DDRB: u16 = 0x03;
/// Read the timer, leaving its interrupt off.
const READ_TIMER: u16 = 0x04;
/// Read the timer and enable its interrupt.
const READ_TIMER_IRQ: u16 = 0x0C;
const READ_FLAGS: u16 = 0x05;
/// Write the timer: `0x14 + n` picks the divider, `+8` enables the interrupt.
const WRITE_TIMER_1: u16 = 0x14;
const WRITE_TIMER_8: u16 = 0x15;
const WRITE_TIMER_1_IRQ: u16 = 0x1C;

#[test]
fn a_port_reads_its_pins_and_not_its_latch() {
    // The bits programmed as outputs read back what the chip is driving; the
    // rest read what the outside world is. A machine's switch matrix is
    // exactly this: strobe on the output half, sense on the input half, one
    // port doing both.
    let mut riot = Riot::new();
    riot.write(DDRA, 0x0F); // low nibble out, high nibble in
    riot.write(DRA, 0xAA);
    riot.in_a = 0x5F;

    assert_eq!(riot.port_a_output(), 0x0A);
    assert_eq!(
        riot.read(DRA),
        0x5A,
        "high from the pins, low from the latch"
    );
}

#[test]
fn the_direction_registers_are_readable() {
    let mut riot = Riot::new();
    riot.write(DDRB, 0x3C);
    assert_eq!(riot.read(DDRB), 0x3C);
    riot.write(DRB, 0xFF);
    assert_eq!(riot.port_b_output(), 0x3C, "only where it drives");
}

#[test]
fn the_timer_counts_down_at_the_rate_it_was_given() {
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_1, 10);
    riot.tick(4);
    assert_eq!(riot.read(READ_TIMER), 6);

    // The same count at the /8 rate takes eight times as long.
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_8, 10);
    riot.tick(4);
    assert_eq!(riot.read(READ_TIMER), 10, "not one tick yet");
    riot.tick(4);
    assert_eq!(riot.read(READ_TIMER), 9);
}

#[test]
fn past_zero_the_timer_keeps_counting_at_one_a_clock() {
    // The half that is easy to leave out. When it passes zero the flag sets
    // *and the divider goes to one*, so what a program reads afterwards is
    // how long ago the timeout was. Stopping at zero looks fine until a ROM
    // uses it that way.
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_8, 1);
    riot.tick(8); // 1 -> 0
    assert_eq!(riot.read(READ_TIMER), 0);
    assert_eq!(riot.read(READ_FLAGS) & 0x80, 0, "not yet");

    riot.tick(8); // 0 -> past
    assert!(riot.read(READ_FLAGS) & 0x80 != 0, "the timeout happened");
    // And now every clock counts, not every eighth.
    let mut riot2 = riot.clone();
    riot2.tick(3);
    assert_eq!(riot2.read(READ_TIMER), 0xFF - 3);
}

#[test]
fn reading_the_timer_clears_its_flag_and_reading_the_flags_does_not() {
    // Two registers, two different clearings, and a routine that wanted both
    // has to read both. Getting this backwards leaves a machine either
    // interrupting for ever or never.
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_1, 1);
    riot.tick(2);
    assert!(riot.read(READ_FLAGS) & 0x80 != 0);
    assert!(
        riot.read(READ_FLAGS) & 0x80 != 0,
        "still set after a flag read"
    );
    riot.read(READ_TIMER);
    assert_eq!(riot.read(READ_FLAGS) & 0x80, 0, "cleared by the timer read");
}

#[test]
fn the_address_used_to_read_the_timer_decides_whether_it_may_interrupt() {
    // The enable is not a register bit; it is which of two addresses the ROM
    // reads. That is genuinely how the chip works and it looks like a typo
    // until you know.
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_1, 1);
    riot.tick(2);
    assert!(!riot.irq(), "written from the address that leaves it off");

    riot.write(WRITE_TIMER_1_IRQ, 1);
    riot.tick(2);
    assert!(riot.irq());

    riot.read(READ_TIMER); // clears the flag *and* switches the interrupt off
    riot.tick(300);
    assert!(!riot.irq(), "read from the address that disables it");

    riot.read(READ_TIMER_IRQ);
    riot.write(WRITE_TIMER_1, 1);
    riot.tick(2);
    assert!(!riot.irq(), "the write switched it off again");
}

#[test]
fn writing_the_timer_clears_a_pending_timeout() {
    let mut riot = Riot::new();
    riot.write(WRITE_TIMER_1_IRQ, 1);
    riot.tick(2);
    assert!(riot.irq());
    riot.write(WRITE_TIMER_1_IRQ, 200);
    assert!(!riot.irq(), "a fresh count starts with a clear flag");
}

#[test]
fn the_edge_detector_watches_pa7_and_can_be_told_which_way() {
    // Falling by default. The flag is cleared by reading the flags, which is
    // the opposite of the timer's.
    let mut riot = Riot::new();
    riot.write(0x04, 0); // edge control: negative, no interrupt
    riot.in_a = 0x80;
    riot.tick(1);
    riot.in_a = 0x00;
    riot.tick(1);
    assert!(riot.read(READ_FLAGS) & 0x40 != 0, "a falling edge");
    assert_eq!(riot.read(READ_FLAGS) & 0x40, 0, "and the read cleared it");

    // Told to watch the other way, the same fall does nothing and the rise
    // does.
    let mut riot = Riot::new();
    riot.write(0x05, 0); // positive edge
    riot.in_a = 0x00;
    riot.tick(1);
    riot.read(READ_FLAGS); // settle low, and ignore however we got there
    riot.in_a = 0x00;
    riot.tick(1);
    assert_eq!(
        riot.read(READ_FLAGS) & 0x40,
        0,
        "staying low is not an edge"
    );
    riot.in_a = 0x80;
    riot.tick(1);
    assert!(riot.read(READ_FLAGS) & 0x40 != 0, "and rising is");
}

#[test]
fn the_edge_can_interrupt_on_its_own() {
    let mut riot = Riot::new();
    riot.write(0x06, 0); // negative edge, interrupt enabled
    riot.in_a = 0x80;
    riot.tick(1);
    riot.in_a = 0x00;
    riot.tick(1);
    assert!(riot.irq(), "the edge alone can ask");
    riot.read(READ_FLAGS);
    assert!(!riot.irq());
}
