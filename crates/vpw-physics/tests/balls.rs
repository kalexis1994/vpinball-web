//! Tests for the collision between balls.
//!
//! Up to here everything that collided was still. Two balls is the first case
//! where both sides move, and where there are conservation laws that work as
//! an independent check on the port: if the impulse is wrong, momentum is not
//! conserved, and that does not depend on having read the original correctly.

use vpw_math::Vec3;
use vpw_physics::ball::{Ball, collide_balls, hit_test_balls};
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Shape};
use vpw_physics::shapes::HitPlane;

const R: f32 = DEFAULT_BALL_SIZE;

/// Restitution between balls: the constant the original has hand-written.
const E: f32 = 0.8;

fn ball(x: f32, vx: f32) -> Ball {
    let mut b = Ball::new(Vec3::new(x, 0.0, R), R);
    b.vel = Vec3::new(vx, 0.0, 0.0);
    b
}

fn momentum(balls: &[&Ball]) -> Vec3 {
    balls.iter().fold(Vec3::ZERO, |acc, b| acc + b.vel * b.mass)
}

// ------------------------------------------------------------ detection ---

#[test]
fn two_balls_coming_closer_are_going_to_touch() {
    let a = ball(0.0, 10.0);
    let b = ball(200.0, -10.0);

    let c = hit_test_balls(&a, &b, 100.0).expect("they have to touch");

    // They approach at 20 and they are 200 - 50 = 150 apart surface to
    // surface.
    assert!(
        (c.hit_time - 7.5).abs() < 1e-3,
        "the time has to come out of the quadratic: {}",
        c.hit_time
    );
    assert!(
        (c.hit_normal - Vec3::X).length() < 1e-4,
        "the normal goes from the first to the second: {:?}",
        c.hit_normal
    );
}

#[test]
fn two_balls_moving_apart_do_not_hit() {
    let a = ball(0.0, -10.0);
    let b = ball(200.0, 10.0);

    assert!(hit_test_balls(&a, &b, 100.0).is_none());
}

#[test]
fn two_balls_passing_side_by_side_do_not_hit() {
    // The case that tells a real test apart from comparing distances: the
    // centers come closer, but never close enough.
    let mut a = ball(0.0, 10.0);
    let mut b = ball(200.0, -10.0);
    a.pos.y = -60.0;
    b.pos.y = 60.0; // 120 of lateral separation, and the radii add up to 50

    assert!(hit_test_balls(&a, &b, 100.0).is_none());
}

#[test]
fn a_slow_ball_does_not_fire_the_hit_ahead_of_time() {
    // Without the minimum relative velocity check, two nearly still balls
    // generate an event per step and the loop does not advance.
    let a = ball(0.0, 1e-6);
    let b = ball(200.0, 0.0);

    assert!(hit_test_balls(&a, &b, 100.0).is_none());
}

#[test]
fn two_balls_already_overlapping_hit_right_now() {
    let a = ball(0.0, 10.0);
    let b = ball(2.0 * R - 5.0, -10.0); // 5 units embedded

    let c = hit_test_balls(&a, &b, 100.0).expect("they are overlapping");
    assert_eq!(c.hit_time, 0.0, "it has to be resolved right now");
    assert!(c.hit_distance < 0.0, "and the distance has to be negative");
}

#[test]
fn two_balls_center_on_center_are_discarded() {
    // There is no normal to use. The original has a patch for this that has
    // been **disabled** for years; we do the same thing it does today.
    let a = ball(0.0, 10.0);
    let b = ball(0.0, -10.0);

    assert!(hit_test_balls(&a, &b, 100.0).is_none());
}

// ----------------------------------------------------------- resolution ---

#[test]
fn a_head_on_hit_sends_them_backwards() {
    let mut a = ball(0.0, 10.0);
    let mut b = ball(2.0 * R, -10.0);
    let c = hit_test_balls(&a, &b, 1.0).expect("they touch");

    collide_balls(&mut a, &mut b, &c);

    // Equal masses and head on: each one comes out with the other's velocity,
    // scaled by the restitution.
    assert!(
        (a.vel.x + 10.0 * E).abs() < 1e-3,
        "the first one should come out at {}, it came out at {}",
        -10.0 * E,
        a.vel.x
    );
    assert!(
        (b.vel.x - 10.0 * E).abs() < 1e-3,
        "and the second one the other way"
    );
}

#[test]
fn momentum_is_conserved() {
    // The independent check: it does not depend on having read the original
    // correctly, it comes from the physics.
    let mut a = ball(0.0, 37.0);
    let mut b = ball(2.0 * R, -11.0);
    let before = momentum(&[&a, &b]);

    let c = hit_test_balls(&a, &b, 1.0).expect("they touch");
    collide_balls(&mut a, &mut b, &c);

    let after = momentum(&[&a, &b]);
    assert!(
        (after - before).length() < 1e-3,
        "the momentum changed: {before:?} -> {after:?}"
    );
}

#[test]
fn the_hit_does_not_create_energy() {
    let mut a = ball(0.0, 37.0);
    let mut b = ball(2.0 * R, -11.0);
    let before = 0.5 * a.mass * a.vel.length_squared() + 0.5 * b.mass * b.vel.length_squared();

    let c = hit_test_balls(&a, &b, 1.0).expect("they touch");
    collide_balls(&mut a, &mut b, &c);

    let after = 0.5 * a.mass * a.vel.length_squared() + 0.5 * b.mass * b.vel.length_squared();
    assert!(
        after <= before + 1e-3,
        "with restitution {E} it has to lose energy: {before} -> {after}"
    );
    assert!(after > 0.0, "but not all of it");
}

#[test]
fn the_order_of_the_two_balls_changes_nothing() {
    // The original resolves the pair from only one of the two and picks which
    // one by comparing **pointers**; since that introduces a bias from the
    // memory ordering, it also flips the criterion on every cycle to
    // compensate. Here the pair is resolved once and symmetrically, so there
    // is no bias. This pins that down.
    let (mut a1, mut b1) = (ball(0.0, 23.0), ball(2.0 * R, -7.0));
    let c1 = hit_test_balls(&a1, &b1, 1.0).expect("they touch");
    collide_balls(&mut a1, &mut b1, &c1);

    let (mut a2, mut b2) = (ball(0.0, 23.0), ball(2.0 * R, -7.0));
    let mut c2 = hit_test_balls(&b2, &a2, 1.0).expect("they touch");
    c2.hit_normal = -c2.hit_normal; // the normal goes from the first to the second
    collide_balls(&mut a2, &mut b2, &c2);

    assert!(
        (a1.vel - a2.vel).length() < 1e-4 && (b1.vel - b2.vel).length() < 1e-4,
        "it came out different depending on the order: {:?}/{:?} against {:?}/{:?}",
        a1.vel,
        b1.vel,
        a2.vel,
        b2.vel
    );
}

#[test]
fn a_captured_ball_weighs_infinity() {
    // A ball locked in a kicker does not move, and the other one bounces off
    // it as off a wall.
    let mut captured = ball(0.0, 0.0);
    captured.locked = true;
    let mut loose = ball(2.0 * R, -10.0);

    let c = hit_test_balls(&captured, &loose, 1.0).expect("they touch");
    collide_balls(&mut captured, &mut loose, &c);

    assert_eq!(captured.vel, Vec3::ZERO, "the captured one does not move");
    assert!(
        loose.vel.x > 0.0,
        "and the other one has to bounce: {:?}",
        loose.vel
    );
    assert!(
        (loose.vel.x - 10.0 * E).abs() < 1e-3,
        "with the whole restitution, not split: {}",
        loose.vel.x
    );
}

#[test]
fn a_glancing_hit_deflects_both() {
    // A graze does not send them back the way they came: it opens them up.
    let mut a = ball(0.0, 20.0);
    let mut b = ball(2.0 * R - 1.0, 0.0);
    a.pos.y = -20.0;
    b.pos.y = 20.0;

    let c = hit_test_balls(&a, &b, 1.0).expect("they graze");
    collide_balls(&mut a, &mut b, &c);

    assert!(a.vel.y < 0.0, "one goes one way: {:?}", a.vel);
    assert!(
        b.vel.y > 0.0,
        "and the other one the other way: {:?}",
        b.vel
    );
    assert!(a.vel.x > 0.0, "and both keep moving forward");
}

// ---------------------------------------------------- inside the engine ---

/// An empty table with a floor, to test the whole loop.
fn empty_engine() -> Engine {
    let floor = HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material: Default::default(),
    };
    Engine::new(
        vec![Shape::Plane(floor)],
        Vec3::new(0.0, 0.0, -DEFAULT_TABLE_GRAVITY),
    )
}

#[test]
fn the_engine_makes_two_balls_hit() {
    // Without this the balls go through each other, which is what used to
    // happen before porting this.
    let mut e = empty_engine();
    e.add_ball(ball(0.0, 30.0));
    e.add_ball(ball(600.0, -30.0));

    for _ in 0..200 {
        e.step();
    }

    assert!(
        e.balls[0].vel.x < 0.0 && e.balls[1].vel.x > 0.0,
        "they should have bounced: {:?} and {:?}",
        e.balls[0].vel,
        e.balls[1].vel
    );
    assert!(
        e.balls[0].pos.x < e.balls[1].pos.x,
        "and not have crossed over: {} and {}",
        e.balls[0].pos.x,
        e.balls[1].pos.x
    );
}

#[test]
fn the_balls_never_end_up_overlapping() {
    // The invariant that matters in the game: in a multiball you must not be
    // able to see one ball stuck inside another.
    let mut e = empty_engine();
    for i in 0..6 {
        let mut b = ball(i as f32 * 120.0, if i % 2 == 0 { 25.0 } else { -25.0 });
        b.pos.y = (i as f32) * 7.0;
        e.add_ball(b);
    }

    let mut worst: f32 = 0.0;
    for _ in 0..3000 {
        e.step();
        for i in 0..e.balls.len() {
            for j in (i + 1)..e.balls.len() {
                let d = (e.balls[i].pos - e.balls[j].pos).length();
                worst = worst.max(2.0 * R - d);
            }
        }
    }

    // There is always some overlap: the engine resolves at the instant of the
    // contact and then integrates. Half a ball inside another one, no.
    assert!(
        worst < R * 0.5,
        "they ended up too far inside each other: {worst:.2} out of {R} of radius"
    );
}

#[test]
fn momentum_is_conserved_in_a_multiball() {
    // With no gravity and no floor: just the balls among themselves. If the
    // loop resolved a pair twice, or skipped one, it shows up here.
    let mut e = Engine::new(Vec::new(), Vec3::ZERO);
    for i in 0..5 {
        let mut b = ball(i as f32 * 130.0, if i % 2 == 0 { 18.0 } else { -18.0 });
        b.pos.y = (i as f32) * 11.0;
        e.add_ball(b);
    }
    let before = momentum(&e.balls.iter().collect::<Vec<_>>());
    let initial_velocities: Vec<Vec3> = e.balls.iter().map(|b| b.vel).collect();

    for _ in 0..2000 {
        e.step();
    }

    // First: that there were hits at all. With no gravity and no walls,
    // momentum is conserved even if nobody touches, and the test would be
    // vacuous.
    let hits = e
        .balls
        .iter()
        .zip(&initial_velocities)
        .filter(|(b, v)| (b.vel - **v).length() > 1.0)
        .count();
    assert!(hits >= 4, "only {hits} balls changed velocity");

    let after = momentum(&e.balls.iter().collect::<Vec<_>>());
    assert!(
        (after - before).length() < 1e-2,
        "the total momentum changed: {before:?} -> {after:?}"
    );
}
