//! The sound board as the firmware sees it: an address space.
//!
//! These go through [`Bus`] rather than poking the peripherals directly,
//! because the bugs this file exists for were all in the decode — the right
//! peripheral behind the wrong offset. A register that answers at the wrong
//! address passes every test that talks to the struct.

use vpw_arm7::Bus;
use vpw_at91::Sound;

/// Timer channel 0's registers. `desound.c:854` maps the block at the top of
/// the address space; the channel stride is 0x40.
const TC0_CCR: u32 = 0xfffe_0000;
const TC0_CMR: u32 = 0xfffe_0004;
const TC0_CV: u32 = 0xfffe_0010;
const TC0_RC: u32 = 0xfffe_001c;
const TC0_IER: u32 = 0xfffe_0024;
const TC0_IDR: u32 = 0xfffe_0028;
const TC0_IMR: u32 = 0xfffe_002c;

/// The power-saving block: control, clock enable, clock disable, clock status
/// (`at91.c:1145-1163,1465`).
const PS_CR: u32 = 0xffff_4000;
const PS_PCER: u32 = 0xffff_4004;
const PS_PCDR: u32 = 0xffff_4008;
const PS_PCSR: u32 = 0xffff_400c;

/// The parallel port's registers (`at91.c:1414-1427`).
const PIO_PER: u32 = 0xffff_0000;
const PIO_PSR: u32 = 0xffff_0008;
const PIO_OER: u32 = 0xffff_0010;
const PIO_ODR: u32 = 0xffff_0014;
const PIO_OSR: u32 = 0xffff_0018;

/// Where the firmware writes the samples (`desound.c:871`).
const SAMPLE_PORT: u32 = 0x2040_0000;

/// The clocks a blank part comes up with (`at91.c:1683`): the three timers'
/// among them, which is what makes the misdecode below latent on boot.
const POWER_AT_RESET: u32 = 0x17c;

/// Starts timer 0 the way the firmware starts the sample clock: wave mode
/// with the restart-on-compare bit, a compare that divides forty megahertz
/// down to the sample rate, and the software trigger.
fn start_sample_clock(snd: &mut Sound) {
    snd.write32(TC0_CMR, 0x4000);
    snd.write32(TC0_RC, 825);
    snd.write32(TC0_CCR, 5);
}

#[test]
fn the_clock_enable_is_one_register_up_from_the_control() {
    let mut snd = Sound::new();
    assert_eq!(snd.read32(PS_PCSR), POWER_AT_RESET);

    // Disable at 0x08, and the status shows it.
    snd.write32(PS_PCDR, 0x70);
    assert_eq!(snd.read32(PS_PCSR), POWER_AT_RESET & !0x70);

    // The control register shares no bits with the clocks: writing it does
    // not enable anything. Decoded one register down — the bug this test
    // pins — this write would have been the enable.
    snd.write32(PS_CR, 0x70);
    assert_eq!(snd.read32(PS_PCSR), POWER_AT_RESET & !0x70);

    // Enable at 0x04. Decoded one down, this would have been the disable:
    // the firmware asks for its clock back and loses it instead.
    snd.write32(PS_PCER, 0x10);
    assert_eq!(snd.read32(PS_PCSR), (POWER_AT_RESET & !0x70) | 0x10);
}

#[test]
fn the_write_only_power_registers_do_not_read_back() {
    let mut snd = Sound::new();
    // The reference answers only the status register (`at91.c:1465`).
    assert_eq!(snd.read32(PS_CR), 0);
    assert_eq!(snd.read32(PS_PCER), 0);
    assert_eq!(snd.read32(PS_PCDR), 0);
}

/// The reason the misdecode was worth fixing: the power register is the
/// timers' clock. A timer whose clock is off must hold its count, and one
/// whose clock comes back must count again — from the same write that brings
/// it back, not from the next unrelated peripheral access.
#[test]
fn a_timer_holds_its_count_while_its_clock_is_off_and_counts_when_it_returns() {
    let mut snd = Sound::new();
    start_sample_clock(&mut snd);

    snd.write32(PS_PCDR, 0x10);
    snd.tick(100_000);
    assert_eq!(snd.read32(TC0_CV), 0, "no clock, no count");

    snd.write32(PS_PCER, 0x10);
    snd.tick(2_000);
    // Two thousand cycles at divide-by-two is a thousand ticks: the compare
    // at 825 fired and restarted the count, and 175 remain. The count moving
    // at all says the clock is back; the restart says the compare fired.
    assert_eq!(snd.read32(TC0_CV), 175, "the clock is back");
}

/// A stale bound is the failure mode of skipping work: the board runs the
/// timers lazily, and the bound on how long they may be left alone has to be
/// recomputed when a write changes what they would do. This is the interrupt
/// arriving on time with nothing else touching the peripherals in between.
#[test]
fn the_sample_interrupt_fires_without_any_other_peripheral_access() {
    let mut snd = Sound::new();
    start_sample_clock(&mut snd);
    snd.write32(TC0_IER, 1 << 4); // the RC compare
    snd.aic.enabled = 1 << vpw_at91::soc::irq::TC0;

    // 825 ticks at divide-by-two: the compare is 1650 cycles out. Nothing
    // reads or writes a peripheral from here on; only time passes.
    let mut fired = false;
    for _ in 0..2_000 {
        fired |= snd.tick(1);
    }
    assert!(fired, "the compare came and went and no interrupt fired");
}

#[test]
fn the_timer_interrupt_mask_reads_at_2c_not_at_the_disable_register() {
    let mut snd = Sound::new();
    snd.write32(TC0_IER, 0x11);
    snd.write32(TC0_IDR, 0x01);
    // The mask is at 0x2c (`at91.c:1386`); 0x28 is the write-only disable.
    assert_eq!(snd.read32(TC0_IMR), 0x10);
    assert_eq!(snd.read32(TC0_IDR), 0);
}

#[test]
fn the_pio_output_status_reads_at_18_not_at_the_enable_register() {
    let mut snd = Sound::new();
    snd.write32(PIO_OER, 0xff);
    snd.write32(PIO_ODR, 0x0f);
    // The status is at 0x18 (`at91.c:1419`); 0x10 is the write-only enable.
    assert_eq!(snd.read32(PIO_OSR), 0xf0);
    assert_eq!(snd.read32(PIO_OER), 0);
}

#[test]
fn the_pio_enable_status_reads_at_08_and_the_enable_register_does_not_answer() {
    let mut snd = Sound::new();
    snd.write32(PIO_PER, 0x3c);
    assert_eq!(snd.read32(PIO_PSR), 0x3c);
    // The reference has no case for a read of the enable register itself
    // (`at91.c:1414`), so it answers what every unmapped register answers.
    assert_eq!(snd.read32(PIO_PER), 0);
}

#[test]
fn a_halfword_store_per_channel_makes_one_stereo_sample() {
    let mut snd = Sound::new();
    // The two channels in the two halves of the word: the upper half at +2
    // is channel zero (`desound.c:576`).
    snd.write16(SAMPLE_PORT + 2, 0x1234);
    snd.write16(SAMPLE_PORT, 0x8765);
    assert_eq!(snd.queued(), 1);
    assert_eq!(snd.take_audio(), vec![(0x1234, 0x8765u16 as i16)]);
}

/// The reference's handler tests the upper half of the mask first, and a full
/// word has both halves, so a word store makes **one** sample from the upper
/// half (`desound.c:573-586`). Making two — one per half — doubles the rate of
/// whatever writes words, and desyncs the channels forever after.
#[test]
fn a_full_word_store_to_the_sample_port_makes_one_sample_not_two() {
    let mut snd = Sound::new();
    snd.write32(SAMPLE_PORT, 0x1234_5678);
    assert_eq!(snd.queued(), 0, "one channel alone does not make a pair");

    // The other channel arrives; the pair is the word's upper half and this,
    // not the word's two halves.
    snd.write16(SAMPLE_PORT, 0x0f0f);
    assert_eq!(snd.take_audio(), vec![(0x1234, 0x0f0f)]);
    assert_eq!(snd.queued(), 0, "nothing left over from the word store");
}

/// Where the firmware reads the command from the CPU board (`desound.c:871`).
const COMMAND_PORT: u32 = 0x2000_0000;

#[test]
fn a_command_is_taken_once_and_then_repeated() {
    let mut snd = Sound::new();
    snd.send(0x53);
    assert_eq!(snd.read8(COMMAND_PORT), 0x53);
    // The queue is empty now, and an empty queue repeats the last byte
    // (`desound.c:725`): the firmware polls and acts on a change, so an idle
    // port has to keep saying the same thing.
    assert_eq!(snd.read8(COMMAND_PORT), 0x53);
    assert_eq!(snd.read8(COMMAND_PORT), 0x53);
    assert_eq!(
        snd.commands_taken, 1,
        "one command, however often it is read"
    );
}

#[test]
fn reposting_the_same_byte_is_not_a_new_command() {
    let mut snd = Sound::new();
    // The game holds the latch between commands, so only a change is queued
    // (`desound.c:696`).
    snd.send(0x21);
    snd.send(0x21);
    snd.send(0x21);
    assert_eq!(snd.read8(COMMAND_PORT), 0x21);
    assert_eq!(
        snd.read8(COMMAND_PORT),
        0x21,
        "the repeat, not a second entry"
    );
    assert_eq!(snd.commands_taken, 1);
}

#[test]
fn a_burst_longer_than_the_queue_loses_its_head_and_keeps_its_tail() {
    let mut snd = Sound::new();
    for c in 1..=6u8 {
        snd.send(c);
    }
    // Four slots (`ARMSNDBUFSIZE`, `desound.c:433`), which hold three
    // commands: when the write head lands on the read head the oldest is
    // dropped (`desound.c:705`), so the slot under the write head is the
    // sacrifice. The last byte of a burst is the one that says what the
    // burst meant, and it survives.
    for expected in 4..=6u8 {
        assert_eq!(snd.read8(COMMAND_PORT), expected);
    }
    assert_eq!(snd.read8(COMMAND_PORT), 6, "and then the repeat");
    assert_eq!(snd.commands_taken, 3);
}

#[test]
fn a_word_read_of_the_command_port_takes_the_command_too() {
    let mut snd = Sound::new();
    snd.send(0x77);
    // The firmware reads the port with whatever width its compiler chose;
    // both land on the same queue.
    assert_eq!(snd.read32(COMMAND_PORT), 0x77);
    assert_eq!(snd.commands_taken, 1);
}
