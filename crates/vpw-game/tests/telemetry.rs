//! Tests for the rolling record.
//!
//! What matters about a ring is what it does at the edges: that it is bounded,
//! that the oldest thing really does go, that a window asks for the *end* of
//! the history and not the beginning, and that nothing is recorded at all when
//! nobody asked for it. The contents are just numbers; those are the
//! properties a recorder is trusted for.

use vpw_game::telemetry::{BallSample, Event, MAX_BALLS, SAMPLE_MS, Sample, Telemetry, WINDOW_MS};

fn sample(t_ms: u32) -> Sample {
    Sample {
        t_ms,
        balls: [BallSample::default(); MAX_BALLS],
        ball_count: 0,
        locked: 0,
        flipper_left: 0.0,
        flipper_right: 0.0,
        solenoids: 0,
        switches: 0,
    }
}

/// A recorder that is on and past its first sample, which is when it starts
/// treating changes as edges.
fn running() -> Telemetry {
    let mut t = Telemetry::new();
    t.set_enabled(true);
    t.sample(sample(0));
    t
}

#[test]
fn nothing_is_recorded_until_somebody_asks() {
    let mut t = Telemetry::new();
    assert!(!t.is_enabled());

    t.sample(sample(0));
    t.event(0.0, Event::Balls(1));
    t.edges(0.0, 0xffff_ffff, u64::MAX);

    assert_eq!(t.sample_count(), 0);
    assert_eq!(t.event_count(), 0);
}

#[test]
fn turning_it_off_drops_the_history() {
    // A window that no longer ends at "now" is worse than no window: it looks
    // like a record of the present and is a record of whenever it stopped.
    let mut t = running();
    t.sample(sample(SAMPLE_MS));
    assert!(t.sample_count() > 0);

    t.set_enabled(false);

    assert_eq!(t.sample_count(), 0);
    assert_eq!(t.event_count(), 0);
}

#[test]
fn the_oldest_sample_goes_to_make_room() {
    let mut t = running();
    let capacity = (WINDOW_MS / SAMPLE_MS) as usize;

    for i in 0..capacity as u32 + 500 {
        t.sample(sample(i * SAMPLE_MS));
    }

    assert_eq!(t.sample_count(), capacity, "it stays bounded");
    // And what is left is the end of the run, not the start.
    assert!(t.latest_ms() >= WINDOW_MS);
}

#[test]
fn the_first_reading_is_not_an_edge() {
    // A machine whose switches are closed when the recorder starts has not just
    // closed them. Reporting the starting state as a burst of edges would say
    // something happened at the moment recording began, every time.
    let mut t = Telemetry::new();
    t.set_enabled(true);

    t.edges(0.0, 0b101, 0b11);

    assert_eq!(t.event_count(), 0);
}

#[test]
fn a_coil_firing_is_one_edge_on_and_one_off() {
    let mut t = running();

    t.edges(10.0, 0b10, 0);
    t.edges(40.0, 0, 0);

    let json = t.dump(60.0, "test");
    assert!(
        json.contains(r#""kind":"solenoid","n":2,"on":true"#),
        "{json}"
    );
    assert!(
        json.contains(r#""kind":"solenoid","n":2,"on":false"#),
        "{json}"
    );
    assert_eq!(
        t.event_count(),
        2,
        "and nothing for the coils that did not move"
    );
}

#[test]
fn switches_are_numbered_from_one() {
    // Bit 0 is switch 1. Off by one here and every number in a dump points at
    // the wrong thing on the playfield, which is the kind of mistake that is
    // only found by someone chasing the wrong switch for an afternoon.
    let mut t = running();

    t.edges(10.0, 0, 1 << 15);

    let json = t.dump(60.0, "test");
    assert!(
        json.contains(r#""kind":"switch","n":16,"closed":true"#),
        "{json}"
    );
}

#[test]
fn a_ball_appearing_is_an_edge_but_standing_still_is_not() {
    let mut t = running();

    t.balls(10.0, 1);
    t.balls(20.0, 1);
    t.balls(30.0, 1);
    t.balls(40.0, 0);

    assert_eq!(t.event_count(), 2, "one for the ball, one for the drain");
}

#[test]
fn the_window_takes_the_end_of_the_history() {
    let mut t = running();
    for i in 1..=100u32 {
        t.sample(sample(i * 1000));
        t.event(f64::from(i) * 1000.0, Event::Balls((i % 4) as u8));
    }

    // Ten seconds of a hundred: the last ten and nothing before them.
    let json = t.dump(10.0, "test");
    assert!(json.contains("100000"), "the newest is in it: {json}");
    assert!(!json.contains("\"t\":50000"), "the middle is not: {json}");
}

#[test]
fn a_dump_is_json_even_when_the_table_is_rude() {
    let mut t = running();
    t.event(10.0, Event::Message(r#"he said "no" \ then left"#.into()));
    t.event(20.0, Event::Sound("fx\tbell\n".into()));

    let json = t.dump(60.0, "a \"note\"");

    // The escapes are there rather than the raw characters.
    assert!(json.contains(r#"\"no\""#), "{json}");
    assert!(json.contains(r"\\"), "{json}");
    assert!(json.contains(r"\t"), "{json}");
    // And no bare control character survived into the text.
    assert!(
        !json.contains('\t'),
        "a raw tab would end the parse: {json}"
    );
}

#[test]
fn a_ball_that_is_not_there_does_not_become_a_zero() {
    // A flipper a table does not have is `NaN`, and JSON cannot write one. It
    // has to come out as `null`: a reader that gets `NaN` back either throws or,
    // worse, reads it as a flipper sitting at zero degrees.
    let mut t = running();
    let mut s = sample(1000);
    s.flipper_left = f32::NAN;
    s.flipper_right = 12.5;
    t.sample(s);

    let json = t.dump(60.0, "test");
    assert!(json.contains("[null,12.5]"), "{json}");
    assert!(!json.contains("NaN"), "{json}");
}

#[test]
fn a_held_ball_is_told_apart_from_one_merely_resting() {
    // The whole question when a saucer will not give a ball back: a ball a
    // kicker is holding and a ball sitting in the same bowl are within a
    // millimetre of each other, and only one of them is coming out.
    let mut t = running();
    let mut s = sample(1000);
    s.ball_count = 2;
    s.locked = 0b01; // the first is held, the second is not
    t.sample(s);
    t.event(
        1000.0,
        Event::Kicker {
            name: "sw21".into(),
            holding: true,
        },
    );

    let json = t.dump(60.0, "test");
    assert!(json.contains(r#""locked": [0,1]"#), "{json}");
    assert!(
        json.contains(r#""kind":"kicker","name":"sw21","holding":true"#),
        "{json}"
    );
}

#[test]
fn a_mark_carries_what_the_host_said_about_it() {
    // Everything inside is stamped in table time, which means nothing to
    // somebody opening the file later. The note is the only link to a clock.
    let t = running();

    let json = t.dump(30.0, "2026-08-22T04:19:07.000Z");

    assert!(
        json.contains(r#""note": "2026-08-22T04:19:07.000Z""#),
        "{json}"
    );
}
