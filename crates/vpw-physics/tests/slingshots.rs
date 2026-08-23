//! Tests for the slingshot.
//!
//! It is a wall with a solenoid: if the ball hits it hard, it shoves it back.
//! Without them, the ball sits there dead in the triangle above the flippers
//! and the table stops playing.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::collision::{CollisionEvent, Material};
use vpw_physics::constants::DEFAULT_BALL_SIZE;
use vpw_physics::shapes::{LineSeg, Slingshot};

const R: f32 = DEFAULT_BALL_SIZE;

/// A vertical slingshot at x = 0, facing +X.
///
/// The elasticity is a rubber's, not a wall's: a slingshot **is** a rubber
/// band, and with low elasticity the solenoid's kick gets eaten up by the
/// bounce and the ball comes out slower than it went in.
fn sling(force: f32, threshold: f32) -> Slingshot {
    let seg = LineSeg::new(
        Vec2::new(0.0, 100.0),
        Vec2::new(0.0, -100.0),
        0.0,
        50.0,
        Material {
            elasticity: 0.85,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        },
    );
    Slingshot::new(seg, force, threshold)
}

fn ball(y: f32, vx: f32) -> Ball {
    let mut b = Ball::new(Vec3::new(R + 0.01, y, 25.0), R);
    b.vel = Vec3::new(vx, 0.0, 0.0);
    b
}

fn contact() -> CollisionEvent {
    CollisionEvent {
        hit_normal: Vec3::X,
        hit_distance: 0.0,
        ..CollisionEvent::default()
    }
}

#[test]
fn a_hard_hit_fires_the_solenoid() {
    let s = sling(80.0, 5.0);
    let mut b = ball(0.0, -30.0); // hitting hard
    let before = b.vel.length();

    let fired = s.collide(&mut b, &contact(), 0.0);

    assert!(fired, "at that velocity it should have fired");
    assert!(
        b.vel.length() > before,
        "the ball has to come out faster than it went in: {before:.1} -> {:.1}",
        b.vel.length()
    );
    assert!(b.vel.x > 0.0, "and the other way");
}

#[test]
fn a_gentle_graze_does_not_fire() {
    // The threshold is there so a ball that barely grazes the slingshot does
    // not set it off: otherwise the table would go mad on its own.
    let s = sling(80.0, 5.0);
    let mut b = ball(0.0, -1.0);
    let before = b.vel.length();

    let fired = s.collide(&mut b, &contact(), 0.0);

    assert!(!fired, "it should not have fired");
    assert!(
        b.vel.length() < before,
        "with no kick, the bounce brakes the ball: {before:.2} -> {:.2}",
        b.vel.length()
    );
}

#[test]
fn the_kick_is_strongest_in_the_middle() {
    // The real mechanism pushes from the center of the rubber, so the force
    // has the shape of a parabola: maximum in the middle and zero at the ends.
    let s = sling(80.0, 1.0);

    let middle = s.kick_force(&ball(0.0, -30.0), Vec3::X);
    let near_the_edge = s.kick_force(&ball(80.0, -30.0), Vec3::X);
    let edge = s.kick_force(&ball(100.0, -30.0), Vec3::X);

    assert!(
        middle > near_the_edge,
        "the middle hits harder: {middle:.1} vs {near_the_edge:.1}"
    );
    assert!(
        near_the_edge > edge,
        "and the edge almost nothing: {near_the_edge:.1} vs {edge:.1}"
    );
    assert!(
        edge.abs() < 1.0,
        "at the very end the kick dies off; got {edge:.3}"
    );
}

#[test]
fn the_maximum_of_the_kick_is_half_the_force() {
    // It is not a mistake of ours: the original does it this way and comments
    // on it itself —"maximum value 0.5 ...I think this should have been
    // 1.0...oh well" (`collideex.cpp:82`). Changing it would alter the force
    // of every slingshot on every table, so we leave it as it is. The test is
    // here so it is written down that this is deliberate.
    let s = sling(80.0, 1.0);
    let middle = s.kick_force(&ball(0.0, -30.0), Vec3::X);
    assert!(
        (middle - 40.0).abs() < 0.5,
        "the maximum should be half of 80; got {middle:.2}"
    );
}

#[test]
fn a_disabled_slingshot_does_not_fire() {
    // The script can turn them off, and then they behave like an ordinary
    // wall.
    let mut s = sling(80.0, 1.0);
    s.disabled = true;
    let mut b = ball(0.0, -50.0);
    let before = b.vel.length();

    let fired = s.collide(&mut b, &contact(), 0.0);

    assert!(!fired);
    assert!(
        b.vel.length() < before,
        "disabled, it only bounces: {before:.1} -> {:.1}",
        b.vel.length()
    );
}

#[test]
fn the_kick_grows_with_the_configured_force() {
    let weak = sling(20.0, 1.0);
    let strong = sling(200.0, 1.0);

    let mut a = ball(0.0, -30.0);
    let mut b = ball(0.0, -30.0);
    weak.collide(&mut a, &contact(), 0.0);
    strong.collide(&mut b, &contact(), 0.0);

    assert!(
        b.vel.length() > a.vel.length() * 2.0,
        "a strong slingshot has to hit much harder: {:.1} vs {:.1}",
        a.vel.length(),
        b.vel.length()
    );
}

#[test]
fn the_slingshot_is_still_a_wall() {
    // Even if it does not fire, it has to stop the ball: it is a wall.
    let s = sling(80.0, 1000.0); // sky-high threshold: it never fires
    let mut b = ball(0.0, -50.0);

    let fired = s.collide(&mut b, &contact(), 0.0);

    assert!(!fired);
    assert!(b.vel.x > 0.0, "it has to bounce all the same");
}

#[test]
fn a_ball_moving_away_does_not_set_it_off() {
    let s = sling(80.0, 5.0);
    let mut b = ball(0.0, 30.0); // moving away
    let before = b.vel;

    let fired = s.collide(&mut b, &contact(), 0.0);

    assert!(!fired, "moving away it does not fire");
    assert_eq!(b.vel, before, "nor does it touch it");
}
