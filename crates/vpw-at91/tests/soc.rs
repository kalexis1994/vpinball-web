//! The parts of the chip a sound board leans on: the timer that sets the
//! sample rate and the controller that decides which interrupt is taken.

use vpw_at91::soc::{Aic, TC_CPCS, Timer, irq};

/// A timer set up the way firmware sets up a sample clock: divide by two,
/// compare at 825, restart on the compare. Forty megahertz over two over 825
/// is a shade over twenty-four kilohertz, which is what these boards run at.
fn sample_clock() -> Timer {
    Timer {
        mode: 0x4000, // restart on the RC compare
        rc: 825,
        running: true,
        ..Timer::default()
    }
}

#[test]
fn the_compare_fires_at_the_rate_it_was_set_to() {
    let mut timer = sample_clock();
    let mut fired = 0;
    for _ in 0..40_000_000u32 / 1000 {
        if timer.advance(1000) {
            fired += 1;
        }
    }
    // Within one of the arithmetic: 40e6 / 2 / 825.
    assert!((24_240..=24_243).contains(&fired), "fired {fired} times");
}

/// The whole point of jumping to the next event rather than counting: it has
/// to leave the timer in the state that counting would have left it in.
#[test]
fn advancing_in_one_jump_matches_advancing_a_cycle_at_a_time() {
    let (mut jumped, mut counted) = (sample_clock(), sample_clock());
    let mut jumped_fired = 0;
    let mut counted_fired = 0;
    for _ in 0..37 {
        if jumped.advance(4321) {
            jumped_fired += 1;
        }
        let mut any = false;
        for _ in 0..4321 {
            any |= counted.advance(1);
        }
        if any {
            counted_fired += 1;
        }
        assert_eq!(jumped.counter, counted.counter, "the count");
        assert_eq!(jumped.remainder, counted.remainder, "the leftover cycles");
        assert_eq!(jumped.status, counted.status, "the events");
        jumped.status = 0;
        counted.status = 0;
    }
    assert_eq!(jumped_fired, counted_fired);
}

#[test]
fn a_stopped_timer_does_not_count() {
    let mut timer = sample_clock();
    timer.running = false;
    assert!(!timer.advance(1_000_000));
    assert_eq!(timer.counter, 0);
}

/// The bound a host uses to skip work has to be a bound: never later than the
/// event, or the event is missed.
#[test]
fn the_quiet_time_never_runs_past_the_compare() {
    let mut timer = sample_clock();
    for _ in 0..200 {
        let quiet = timer.until_event();
        assert!(quiet > 0, "a bound of zero would never make progress");
        // One cycle short of the bound cannot fire.
        if quiet > 1 {
            assert!(!timer.advance(quiet - 1), "fired early, at {quiet}");
        }
        assert!(
            timer.advance(1),
            "did not fire when the bound said it would"
        );
        timer.status = 0;
    }
}

#[test]
fn without_the_restart_bit_the_counter_runs_on_to_its_top() {
    let mut timer = sample_clock();
    timer.mode = 0; // no restart
    timer.advance(825 * 2);
    assert_eq!(timer.counter, 825, "stopped at the compare, not reset");
    assert!(timer.status & TC_CPCS != 0);
    // And from there the next event is the sixteen-bit overflow, which is not
    // a compare and so is reported in the status rather than the return.
    timer.status = 0;
    timer.advance((0x1_0000 - 825) * 2 - 2);
    assert_eq!(timer.counter, 0xffff, "one tick short of the top");
    assert_eq!(timer.status, 0, "and nothing has happened yet");
    timer.advance(2);
    assert_eq!(timer.counter, 0);
    assert!(timer.status & vpw_at91::soc::TC_COVFS != 0, "it went round");
}

#[test]
fn the_controller_takes_the_highest_priority_source_that_is_enabled() {
    let mut aic = Aic::default();
    aic.modes[irq::TC0 as usize] = 3;
    aic.modes[irq::TC1 as usize] = 5;
    aic.vectors[irq::TC0 as usize] = 0x1000;
    aic.vectors[irq::TC1 as usize] = 0x2000;
    aic.enabled = (1 << irq::TC0) | (1 << irq::TC1);

    aic.raise(irq::TC0);
    assert!(aic.asserted());
    aic.raise(irq::TC1);
    assert_eq!(aic.acknowledge(), 0x2000, "the higher priority one");

    // And while it is in service, the lower one does not interrupt it.
    assert!(!aic.asserted());
    aic.end_of_interrupt();
    assert!(aic.asserted());
    assert_eq!(aic.acknowledge(), 0x1000);
}

#[test]
fn a_source_that_is_not_enabled_is_not_taken() {
    let mut aic = Aic::default();
    aic.raise(irq::TC2);
    assert!(!aic.asserted());
    aic.enabled = 1 << irq::TC2;
    assert!(aic.asserted());
}

/// The fast interrupt has a line of its own and never comes out of the
/// priority scheme, which is what makes it fast.
#[test]
fn the_fast_interrupt_is_kept_apart_from_the_rest() {
    let mut aic = Aic::default();
    aic.enabled = 1 << irq::FIQ;
    aic.raise(irq::FIQ);
    assert!(aic.fast_asserted());
    assert!(!aic.asserted(), "it is not one of the vectored sources");
}
