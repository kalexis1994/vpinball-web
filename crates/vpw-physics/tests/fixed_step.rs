//! The clock that turns real time into physics ticks.
//!
//! The whole job is not losing time. A tick that is not run is a millisecond of
//! the game that did not happen, and nobody is told: the ball is simply
//! somewhere else than it should be, and a player who was aiming feels it
//! without being able to say what went wrong.

use vpw_physics::constants::PHYSICS_STEPTIME_S;
use vpw_physics::{FixedStep, MAX_CATCH_UP_S};

/// The clock the player uses.
fn clock() -> FixedStep {
    FixedStep::default()
}

#[test]
fn an_ordinary_frame_loses_nothing_over_a_long_run() {
    // A frame is not a whole number of ticks — at 120 Hz it is eight and a
    // third — so every frame leaves a remainder. Dropping it instead of
    // carrying it costs a sixth of the game and looks like the loop failing to
    // keep up.
    let mut c = clock();
    let frame = 1.0 / 120.0;
    let frames = 1200;

    let mut ticks = 0u64;
    for _ in 0..frames {
        ticks += u64::from(c.advance(frame));
    }

    let wanted = (f64::from(frames) * frame / PHYSICS_STEPTIME_S) as u64;
    assert!(
        ticks.abs_diff(wanted) <= 1,
        "ten seconds at 120 fps should be {wanted} ticks, and it ran {ticks}"
    );
}

#[test]
fn a_stall_the_clock_will_make_up_for_is_made_up_for_in_full() {
    // The failure this is here for. The caller clamps the elapsed time it hands
    // in and the clock caps the ticks it hands back, and those two used to be
    // separate numbers in separate crates — a quarter of a second against a
    // hundred ticks. Everything in between was thrown away without anyone
    // deciding to: a 250 ms browser stall asked for 250 ticks, got 100, and a
    // hundred and fifty milliseconds of the game did not happen.
    let mut c = clock();
    let stall = MAX_CATCH_UP_S;

    let ticks = c.advance(stall);
    let wanted = (stall / PHYSICS_STEPTIME_S) as u32;
    assert_eq!(
        ticks, wanted,
        "a stall of exactly what the clock says it will make up for has to be \
         made up for whole"
    );
}

#[test]
fn the_caller_and_the_clock_agree_on_where_the_line_is() {
    // The invariant that keeps the two from drifting apart again: whatever a
    // caller clamps to, it has to be this, and this has to be reachable.
    let c = clock();
    assert_eq!(
        c.max_catch_up_seconds(),
        MAX_CATCH_UP_S,
        "the number a caller is told to clamp to is the number this enforces"
    );

    // And it is not so long that catching up is its own freeze. Measured in a
    // browser, a step costs about 107 microseconds; a quarter of a second of
    // them is around 27 ms, which is one dropped frame. Checked at compile
    // time, because it is a fact about a constant and a test that cannot fail
    // at run time should not pretend it might.
    const _: () = assert!(MAX_CATCH_UP_S <= 0.3);
}

#[test]
fn past_the_line_the_time_is_gone_rather_than_owed() {
    // A tab that spent a minute in the background is not owed a minute of
    // pinball: catching up would be a minute of frozen picture. Slow motion is
    // the better failure, and the remainder has to be dropped rather than kept
    // for next frame — kept, it never clears and every frame from then on is a
    // catch-up frame.
    let mut c = clock();
    let ticks = c.advance(60.0);
    assert!(
        f64::from(ticks) * PHYSICS_STEPTIME_S <= MAX_CATCH_UP_S + 1e-9,
        "a minute in the background ran {ticks} ticks"
    );

    // The next ordinary frame is ordinary again.
    let after = c.advance(1.0 / 120.0);
    assert!(
        after <= 10,
        "the debt should have been dropped, not carried: {after} ticks"
    );
}
