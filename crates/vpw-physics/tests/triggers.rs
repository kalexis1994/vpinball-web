//! Tests for the triggers.
//!
//! A trigger does not touch the ball: the only thing it produces is events. So
//! what gets verified is that it reports **exactly once** per entry and once
//! per exit, and that it never moves anything.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, PHYS_FACTOR};
use vpw_physics::engine::{Engine, Event};
use vpw_physics::trigger::{Crossing, Trigger, Zone};

const R: f32 = DEFAULT_BALL_SIZE;

fn circle(radius: f32) -> Trigger {
    Trigger::new(
        Zone::Circle {
            center: Vec2::ZERO,
            radius,
        },
        0.0,
        50.0,
    )
}

/// A square of side 200 centered on the origin, given counter-clockwise.
fn square() -> Trigger {
    Trigger::new(
        Zone::Polygon {
            vertices: vec![
                Vec2::new(-100.0, -100.0),
                Vec2::new(100.0, -100.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(-100.0, 100.0),
            ],
        },
        0.0,
        50.0,
    )
}

fn ball_at(x: f32, y: f32, z: f32) -> Ball {
    Ball::new(Vec3::new(x, y, z), R)
}

// ------------------------------------------------------------- the zone ---

#[test]
fn a_circle_knows_who_is_inside() {
    let t = circle(100.0);
    assert!(t.contains(&ball_at(0.0, 0.0, 25.0)));
    assert!(t.contains(&ball_at(99.0, 0.0, 25.0)));
    assert!(!t.contains(&ball_at(101.0, 0.0, 25.0)));
}

#[test]
fn it_looks_at_the_center_of_the_ball_and_not_its_edge() {
    // The original tests the sides of the trigger with `lateral = false`, that
    // is, against the center. A trigger fires when the ball passes **over the
    // top**, not when it grazes it with its side.
    let t = circle(100.0);
    let grazing = ball_at(100.0 + R * 0.5, 0.0, 25.0);

    assert!(
        !t.contains(&grazing),
        "the edge of the ball touches it, but its center is outside"
    );
}

#[test]
fn a_ball_up_above_does_not_fire_the_trigger() {
    // The zone is finite in height. A ball on a ramp passing over the top does
    // not count.
    let t = circle(100.0);
    assert!(!t.contains(&ball_at(0.0, 0.0, 500.0)));
}

#[test]
fn a_disabled_trigger_contains_nobody() {
    let mut t = circle(100.0);
    t.enabled = false;
    assert!(!t.contains(&ball_at(0.0, 0.0, 25.0)));
}

#[test]
fn a_polygon_knows_who_is_inside() {
    let t = square();
    assert!(t.contains(&ball_at(0.0, 0.0, 25.0)));
    assert!(t.contains(&ball_at(-99.0, 99.0, 25.0)));
    assert!(!t.contains(&ball_at(150.0, 0.0, 25.0)));
    assert!(!t.contains(&ball_at(0.0, -150.0, 25.0)));
}

#[test]
fn a_concave_polygon_does_not_catch_the_gap() {
    // A wire trigger is drawn by hand and can have any shape at all. Counting
    // crossings solves it; comparing against a box or assuming convexity does
    // not.
    //
    // A "U": the gap in the middle is outside even though it sits between the
    // arms.
    let t = Trigger::new(
        Zone::Polygon {
            vertices: vec![
                Vec2::new(-100.0, -100.0),
                Vec2::new(-50.0, -100.0),
                Vec2::new(-50.0, 50.0),
                Vec2::new(50.0, 50.0),
                Vec2::new(50.0, -100.0),
                Vec2::new(100.0, -100.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(-100.0, 100.0),
            ],
        },
        0.0,
        50.0,
    );

    assert!(t.contains(&ball_at(0.0, 80.0, 25.0)), "the bottom of the U");
    assert!(t.contains(&ball_at(-75.0, 0.0, 25.0)), "an arm");
    assert!(
        !t.contains(&ball_at(0.0, -20.0, 25.0)),
        "the gap is outside"
    );
}

#[test]
fn the_winding_of_the_outline_does_not_matter() {
    // The drag points of a table come in any order.
    let clockwise = Trigger::new(
        Zone::Polygon {
            vertices: vec![
                Vec2::new(-100.0, 100.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(100.0, -100.0),
                Vec2::new(-100.0, -100.0),
            ],
        },
        0.0,
        50.0,
    );
    let b = ball_at(10.0, -30.0, 25.0);

    assert_eq!(square().contains(&b), clockwise.contains(&b));
    assert!(clockwise.contains(&b));
}

// -------------------------------------------------------- the crossings ---

#[test]
fn entering_reports_only_once() {
    let mut t = circle(100.0);
    let inside = ball_at(0.0, 0.0, 25.0);

    assert_eq!(t.update(0, &inside), Some(Crossing::Entered));
    assert_eq!(t.update(0, &inside), None, "it does not report again");
    assert_eq!(t.update(0, &inside), None);
}

#[test]
fn leaving_reports_only_once() {
    let mut t = circle(100.0);
    t.update(0, &ball_at(0.0, 0.0, 25.0));

    let outside = ball_at(500.0, 0.0, 25.0);
    assert_eq!(t.update(0, &outside), Some(Crossing::Exited));
    assert_eq!(t.update(0, &outside), None);
}

#[test]
fn each_ball_keeps_its_own_count() {
    // In a multiball, one ball inside cannot mask another one's entry.
    let mut t = circle(100.0);
    let inside = ball_at(0.0, 0.0, 25.0);
    let outside = ball_at(500.0, 0.0, 25.0);

    assert_eq!(t.update(0, &inside), Some(Crossing::Entered));
    assert_eq!(
        t.update(1, &inside),
        Some(Crossing::Entered),
        "the second one too"
    );
    assert_eq!(t.update(0, &outside), Some(Crossing::Exited));
    assert!(t.has_ball(1), "and the second one is still inside");
    assert!(!t.has_ball(0));
}

#[test]
fn taking_a_ball_out_fires_nothing() {
    // A ball that drains has to leave the record without reporting; otherwise
    // the trigger keeps believing it is inside and the next ball to take that
    // index does not fire the entry.
    let mut t = circle(100.0);
    let inside = ball_at(0.0, 0.0, 25.0);
    t.update(0, &inside);

    t.renumber_after_removing(0);

    assert!(!t.has_ball(0));
    assert_eq!(
        t.update(0, &inside),
        Some(Crossing::Entered),
        "a new ball with that index has to fire again"
    );
}

#[test]
fn the_balls_after_the_one_that_drained_move_down_with_it() {
    // The engine removes the drained ball from its list, so every ball after it
    // changes index. This bitset is keyed by index and has to move with them.
    //
    // Clear the one bit and leave the rest alone — the obvious thing to write —
    // and everything above the gap answers for its neighbour: the ball that
    // inherits the freed index reads as outside and fires an entry it never
    // made. With one ball in play nothing sits above the gap and it never
    // shows. In a multiball, one drain scrambles every trigger on the table.
    let mut t = circle(100.0);
    let inside = ball_at(0.0, 0.0, 25.0);
    let outside = ball_at(500.0, 0.0, 25.0);

    // Balls 0 and 2 are in the trigger, ball 1 is not.
    t.update(0, &inside);
    t.update(1, &outside);
    t.update(2, &inside);
    assert!(t.has_ball(0) && !t.has_ball(1) && t.has_ball(2));

    // Ball 0 drains. What was 1 is now 0, and what was 2 is now 1.
    t.renumber_after_removing(0);

    assert!(!t.has_ball(0), "the ball that was outside still is");
    assert!(t.has_ball(1), "and the one that was inside still is");
    assert!(!t.has_ball(2), "with nothing left in the slot they vacated");

    // The one that is still inside says so, rather than announcing itself.
    assert_eq!(t.update(1, &inside), None);
    // And the one that is outside is not credited with an exit it never made.
    assert_eq!(t.update(0, &outside), None);
}

#[test]
fn the_last_slot_can_be_taken_out_too() {
    // Sixty-four is where the bitset ends, and shifting a `u64` by 64 is not a
    // shift at all — it is a panic in debug and a no-op in release.
    let mut t = circle(100.0);
    let inside = ball_at(0.0, 0.0, 25.0);
    t.update(63, &inside);
    assert!(t.has_ball(63));

    t.renumber_after_removing(63);

    assert!(!t.has_ball(63));
}

// -------------------------------------------------------- in the engine ---

fn engine_with_trigger(t: Trigger) -> Engine {
    let mut e = Engine::new(Vec::new(), Vec3::ZERO);
    e.add_trigger(t);
    e
}

#[test]
fn a_ball_crossing_reports_an_entry_and_an_exit() {
    let mut e = engine_with_trigger(circle(100.0));
    let mut b = ball_at(-400.0, 0.0, 25.0);
    b.vel = Vec3::new(20.0, 0.0, 0.0);
    e.add_ball(b);

    let mut crossings = Vec::new();
    for _ in 0..500 {
        e.step();
        crossings.extend(e.take_events().filter_map(|ev| match ev {
            Event::Trigger { crossing, .. } => Some(crossing),
            // Nothing here asks for hit reports, so there are none of those
            // and none from a slingshot either.
            Event::Hit { .. } | Event::Slingshot { .. } => None,
        }));
    }

    assert_eq!(
        crossings,
        vec![Crossing::Entered, Crossing::Exited],
        "one entry and one exit, in that order"
    );
}

#[test]
fn the_trigger_does_not_deflect_the_ball() {
    // What sets it apart from everything else in the engine. The original also
    // pushes the ball forward a touch when it fires, so the crossing is not
    // detected again; here that is not needed and it is not done.
    let mut e = engine_with_trigger(circle(100.0));
    let mut b = ball_at(-400.0, 0.0, 25.0);
    b.vel = Vec3::new(20.0, 3.0, 0.0);
    e.add_ball(b);
    let initial_vel = e.balls[0].vel;

    let mut path = Vec::new();
    for _ in 0..500 {
        e.step();
        path.push(e.balls[0].pos);
    }

    assert_eq!(e.balls[0].vel, initial_vel, "the velocity is not touched");

    // And the trajectory has to be a perfect straight line.
    let expected =
        |i: usize| Vec3::new(-400.0, 0.0, 25.0) + initial_vel * (PHYS_FACTOR * (i + 1) as f32);
    for (i, p) in path.iter().enumerate() {
        assert!(
            (*p - expected(i)).length() < 1e-2,
            "on step {i} the ball came off the line: {p:?} instead of {:?}",
            expected(i)
        );
    }
}

#[test]
fn the_event_says_which_trigger_and_which_ball() {
    let mut e = Engine::new(Vec::new(), Vec3::ZERO);
    e.add_trigger(Trigger::new(
        Zone::Circle {
            center: Vec2::new(-300.0, 0.0),
            radius: 60.0,
        },
        0.0,
        50.0,
    ));
    e.add_trigger(Trigger::new(
        Zone::Circle {
            center: Vec2::new(300.0, 0.0),
            radius: 60.0,
        },
        0.0,
        50.0,
    ));

    e.add_ball(ball_at(-300.0, 0.0, 25.0)); // starts in the first one
    e.add_ball(ball_at(300.0, 0.0, 25.0)); // and this one in the second

    e.step();
    let mut events: Vec<(usize, usize)> = e
        .take_events()
        .filter_map(|ev| match ev {
            Event::Trigger { trigger, ball, .. } => Some((trigger, ball)),
            Event::Hit { .. } | Event::Slingshot { .. } => None,
        })
        .collect();
    events.sort_unstable();

    assert_eq!(events, vec![(0, 0), (1, 1)], "each ball in its own trigger");
}

#[test]
fn a_fast_ball_does_not_go_through_a_trigger_without_reporting() {
    // **The** risk of looking at the position on every step instead of finding
    // the exact crossing: if the ball advanced further than the width of the
    // trigger in one step, it would come in and go out with nobody noticing.
    //
    // Whether that happens also depends on the **phase** —on where the steps
    // land relative to the trigger—, so testing a single trajectory measures
    // nothing: it might go right through the center out of sheer luck. Twenty
    // phases are swept per velocity and we look for the first one where any of
    // them fails.
    //
    // And we use the smallest trigger a table carries: a button of radius 25,
    // fifty units from end to end.
    const RADIUS: f32 = 25.0;

    let misses = |speed: f32, phase: f32| {
        let mut e = engine_with_trigger(circle(RADIUS));
        let mut b = ball_at(-500.0 + phase, 0.0, 25.0);
        b.vel = Vec3::new(speed, 0.0, 0.0);
        e.add_ball(b);

        let mut entries = 0;
        for _ in 0..2000 {
            e.step();
            entries += e
                .take_events()
                .filter(|ev| {
                    matches!(
                        ev,
                        Event::Trigger {
                            crossing: Crossing::Entered,
                            ..
                        }
                    )
                })
                .count();
            if e.balls[0].pos.x > 500.0 {
                break;
            }
        }
        entries == 0
    };

    let mut first_that_misses = None;
    for n in 1..=60 {
        let speed = n as f32 * 20.0;
        // The advance per step is `speed * PHYS_FACTOR`; the phases are spread
        // out inside that advance.
        let advance = speed * PHYS_FACTOR;
        let has_gap = (0..20).any(|k| misses(speed, advance * k as f32 / 20.0));
        if has_gap {
            first_that_misses = Some(speed);
            break;
        }
    }

    let limit = first_that_misses.expect("it did not fail even at a velocity of 1200");
    eprintln!(
        "the trigger of radius {RADIUS} is only skipped at a velocity of {limit} \
         ({:.0} units per step)",
        limit * PHYS_FACTOR
    );

    // A pinball does not go beyond some sixty VP units per unit of time. The
    // margin has to be several times that.
    assert!(
        limit > 60.0 * 4.0,
        "the margin is very small: it already skips at {limit}, and a fast ball runs at about 60"
    );
}

// ------------------------------------------------------------ animation ---

/// A trigger that sinks 32 units at one per millisecond.
fn animated() -> Trigger {
    circle(100.0).with_animation(32.0, 1.0)
}

#[test]
fn the_trigger_sinks_when_the_ball_comes_in() {
    let mut t = animated();
    t.animate_crossing(Crossing::Entered);

    assert!(t.animating(), "it has to start moving");
    for _ in 0..10 {
        t.animate(1.0);
    }
    assert!(
        (t.anim_offset + 10.0).abs() < 1e-4,
        "ten milliseconds at one unit per ms: {}",
        t.anim_offset
    );

    for _ in 0..100 {
        t.animate(1.0);
    }
    assert_eq!(t.anim_offset, -32.0, "and it stops at the limit");
    assert!(!t.animating(), "and stops animating");
}

#[test]
fn the_trigger_comes_back_animating_and_not_in_one_jump() {
    // Here is the difference from the original: `Trigger::UpdateAnimation`, on
    // receiving the exit, sets the offset to **plus** the limit and then goes
    // up, and the first check is `offset >= 0`, so it cuts off on the very same
    // frame. All the machinery for coming up gradually that it has around it
    // is never used. It is a missing minus sign.
    let mut t = animated();
    t.animate_crossing(Crossing::Entered);
    for _ in 0..100 {
        t.animate(1.0);
    }
    assert_eq!(t.anim_offset, -32.0, "fully sunk");

    t.animate_crossing(Crossing::Exited);
    t.animate(1.0);
    assert!(
        t.anim_offset < 0.0,
        "one millisecond later it still has to be coming up, not at zero"
    );
    assert!(t.animating());

    for _ in 0..100 {
        t.animate(1.0);
    }
    assert_eq!(t.anim_offset, 0.0, "and it ends up at rest");
    assert!(!t.animating());
}

#[test]
fn an_exit_halfway_down_comes_up_from_where_it_is() {
    // The original resets the offset to zero on entering, so an interrupted
    // animation makes a visible jump. Here it carries on from where it was.
    let mut t = animated();
    t.animate_crossing(Crossing::Entered);
    for _ in 0..5 {
        t.animate(1.0);
    }
    let halfway = t.anim_offset;
    assert!(halfway < 0.0 && halfway > -32.0, "halfway down");

    t.animate_crossing(Crossing::Exited);
    t.animate(1.0);

    assert!(
        (t.anim_offset - (halfway + 1.0)).abs() < 1e-4,
        "it should carry on from {halfway}, it ended at {}",
        t.anim_offset
    );
}

#[test]
fn a_still_trigger_costs_nothing() {
    let mut t = animated();
    for _ in 0..1000 {
        t.animate(1.0);
    }
    assert_eq!(t.anim_offset, 0.0);
    assert!(!t.animating());
}

#[test]
fn the_animation_does_not_depend_on_how_many_frames_per_second_there_are() {
    // The original ties it to the video frame, so the same trigger sinks
    // faster on a machine that runs at more frames. Tied to the table's clock,
    // the result is the same.
    let mut fine = animated();
    let mut coarse = animated();
    fine.animate_crossing(Crossing::Entered);
    coarse.animate_crossing(Crossing::Entered);

    for _ in 0..80 {
        fine.animate(0.25); // twenty milliseconds in small steps
    }
    for _ in 0..4 {
        coarse.animate(5.0); // the same twenty, in big steps
    }

    assert!(
        (fine.anim_offset - coarse.anim_offset).abs() < 1e-4,
        "{} against {}",
        fine.anim_offset,
        coarse.anim_offset
    );
}

#[test]
fn the_engine_sinks_the_trigger_when_the_ball_goes_over_it() {
    let mut e = engine_with_trigger(animated());
    let mut b = ball_at(-400.0, 0.0, 25.0);
    b.vel = Vec3::new(20.0, 0.0, 0.0);
    e.add_ball(b);

    let mut deepest: f32 = 0.0;
    for _ in 0..500 {
        e.step();
        let _ = e.take_events().count();
        deepest = deepest.min(e.triggers()[0].anim_offset);
    }

    assert!(
        deepest < -20.0,
        "the wire should have sunk, it got to {deepest}"
    );
    assert_eq!(
        e.triggers()[0].anim_offset,
        0.0,
        "and after the ball left, come back to rest"
    );
}

#[test]
fn the_animation_finishes_even_if_the_trigger_is_turned_off() {
    // If it gets turned off with the wire sunk, it still has to finish coming
    // back; otherwise it stays pinned down there forever.
    let mut e = engine_with_trigger(animated());
    e.add_ball(ball_at(0.0, 0.0, 25.0));
    for _ in 0..100 {
        e.step();
        let _ = e.take_events().count();
    }
    assert!(e.triggers()[0].anim_offset < 0.0, "sunk");

    e.set_trigger_enabled(0, false);
    for _ in 0..200 {
        e.step();
        let _ = e.take_events().count();
    }

    assert_eq!(
        e.triggers()[0].anim_offset,
        0.0,
        "it should have finished coming back"
    );
}
