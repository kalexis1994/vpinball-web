//! Tests for the ball.
//!
//! The physics is deterministic, so here we can really verify things: we
//! simulate, we measure, and we compare against what mechanics says. That is a
//! big difference from the renderer, where the only thing we could do was look
//! at the result.

use vpw_math::{Mat3, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::constants::{
    DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY, GRAVITYCONST, PHYSICS_STEPTIME_S,
};

/// The gravity of a table, as `PhysicsEngine::SetGravity` builds it
/// (`PhysicsEngine.cpp:104`): the slope splits the force between the axis that
/// runs along the table and the one that goes through it.
fn gravity(slope_deg: f32, strength: f32) -> Vec3 {
    let r = slope_deg.to_radians();
    Vec3::new(0.0, r.sin() * strength, -r.cos() * strength)
}

/// One physics step, with no collisions.
fn step(b: &mut Ball, g: Vec3) {
    b.update_velocities(g, Vec3::ZERO, 6.0f32.to_radians());
    b.update_displacements(1.0);
}

#[test]
fn gravity_is_split_according_to_the_slope() {
    // At zero degrees the table is horizontal: all of gravity goes downwards.
    let g = gravity(0.0, DEFAULT_TABLE_GRAVITY);
    assert!(
        g.y.abs() < 1e-6,
        "with no slope there is no lengthwise component"
    );
    assert!((g.z + DEFAULT_TABLE_GRAVITY).abs() < 1e-6);

    // At ninety, all of gravity runs along the table.
    let g = gravity(90.0, DEFAULT_TABLE_GRAVITY);
    assert!((g.y - DEFAULT_TABLE_GRAVITY).abs() < 1e-5);
    assert!(g.z.abs() < 1e-5);

    // And at the 6 degrees of a real table, the lengthwise component is small
    // but it is the one that brings the ball down.
    let g = gravity(6.0, DEFAULT_TABLE_GRAVITY);
    assert!(g.y > 0.0, "the ball has to fall towards the player");
    assert!(g.y < g.z.abs(), "and by much less than the vertical one");
}

#[test]
fn a_ball_in_free_fall_follows_the_formula() {
    // With no collisions, the ball has to satisfy v = g·t and d = ½·g·t².
    //
    // Careful with the units: the physics step lasts `PHYS_FACTOR` in VP time,
    // and `update_displacements` receives that same dt. Since `PHYS_FACTOR` is
    // worth 0.1, a hundred steps are ten VP time units.
    let g = Vec3::new(0.0, 0.0, -1.0);
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);

    let steps = 100;
    for _ in 0..steps {
        b.update_velocities(g, Vec3::ZERO, 0.0);
        b.update_displacements(vpw_physics::constants::PHYS_FACTOR);
    }

    let t = steps as f32 * vpw_physics::constants::PHYS_FACTOR;
    let expected_v = -t;
    assert!(
        (b.vel.z - expected_v).abs() < 1e-3,
        "velocity: got {} and expected {expected_v}",
        b.vel.z
    );

    // The integration is forward Euler, so the position ends up half a step
    // ahead of the continuous formula. The error has to be of that order and
    // no bigger.
    let expected_d = -0.5 * t * t;
    let error = (b.pos.z - expected_d).abs();
    let half_step = 0.5 * vpw_physics::constants::PHYS_FACTOR * expected_v.abs();
    assert!(
        error <= half_step * 1.1,
        "position: got {}, continuous formula {expected_d}, error {error} > half step {half_step}",
        b.pos.z
    );
}

#[test]
fn a_captured_ball_does_not_move() {
    let mut b = Ball::new(Vec3::new(100.0, 200.0, 50.0), DEFAULT_BALL_SIZE);
    b.locked = true;
    let before = b.pos;

    for _ in 0..100 {
        step(&mut b, gravity(6.0, DEFAULT_TABLE_GRAVITY));
    }

    assert_eq!(b.pos, before, "a ball captured in a kicker does not fall");
    assert_eq!(b.vel, Vec3::ZERO, "nor does it build up velocity");
}

#[test]
fn the_nudge_acts_opposite_to_how_the_cabinet_moves() {
    // It is a fictitious force: the cabinet moves and the ball, out of
    // inertia, stays put. Seen from the table, the ball goes the other way.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.update_velocities(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.0);
    assert!(
        b.vel.x < 0.0,
        "nudging in +X sends the ball to -X; got {}",
        b.vel.x
    );
}

#[test]
fn the_lengthwise_nudge_is_split_between_two_axes() {
    // The nudge acts in the horizontal plane of the cabinet, but the ball
    // lives on the playfield, which is tilted. The lengthwise component has to
    // be split between the table's axis and the vertical.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    let slope = 6.0f32.to_radians();
    b.update_velocities(Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0), slope);

    assert!(b.vel.y < 0.0, "it has to push lengthwise");
    assert!(b.vel.z < 0.0, "and some in the vertical");
    // The ratio between the two is the tangent of the slope.
    let ratio = b.vel.z / b.vel.y;
    assert!(
        (ratio - slope.tan()).abs() < 1e-4,
        "the ratio has to be the tangent; got {ratio}"
    );
}

#[test]
fn the_moment_of_inertia_is_that_of_a_solid_sphere() {
    // 2/5·m·r², `hitball.h:59`.
    let b = Ball {
        radius: 10.0,
        mass: 2.0,
        ..Ball::default()
    };
    assert!((b.inertia() - 0.4 * 100.0 * 2.0).abs() < 1e-4);
}

#[test]
fn the_orientation_stays_orthonormal() {
    // The original integrates a 3x3 matrix and reorthonormalizes it on every
    // step. Here the orientation is a quaternion, which cannot degenerate; the
    // test still verifies the property that matters, because the renderer
    // consumes a matrix and that matrix has to still be a pure rotation after
    // thousands of steps.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    // A hard spin about some arbitrary axis.
    b.angular_momentum = Vec3::new(1.0, 2.0, 3.0).normalize() * b.inertia() * 5.0;

    for _ in 0..2000 {
        b.update_displacements(0.01);
    }

    let m = b.orientation_matrix();
    for (name, axis) in [("x", m.x_axis), ("y", m.y_axis), ("z", m.z_axis)] {
        assert!(
            (axis.length() - 1.0).abs() < 1e-3,
            "the {name} axis stopped being a unit vector: it measures {}",
            axis.length()
        );
    }
    assert!(
        m.x_axis.dot(m.y_axis).abs() < 1e-3,
        "the axes stopped being perpendicular"
    );
    assert!(m.y_axis.dot(m.z_axis).abs() < 1e-3);
    // And it is still a rotation, not a reflection.
    assert!(
        m.determinant() > 0.9,
        "the determinant came out {}",
        m.determinant()
    );
}

#[test]
fn a_still_ball_does_not_spin_its_orientation() {
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.update_displacements(1.0);
    assert!(
        (b.orientation_matrix() - Mat3::IDENTITY)
            .to_cols_array()
            .iter()
            .all(|v| v.abs() < 1e-6),
        "with no angular momentum the orientation does not change"
    );
}

#[test]
fn the_bounding_box_grows_with_the_velocity() {
    // The broadphase uses a cubic box the size of the velocity plus the
    // radius, not the one of the actual path: the ball can bounce within the
    // same step and go off in any direction (`hitball.cpp:404`).
    let mut b = Ball::new(Vec3::ZERO, 25.0);
    let still = b.hit_bbox();
    assert!(
        (still.max.x - 25.05).abs() < 1e-3,
        "at rest, the box is the radius"
    );

    b.vel = Vec3::new(100.0, 0.0, 0.0);
    let fast = b.hit_bbox();
    assert!(fast.max.x > still.max.x, "when moving, the box grows");
    // And it grows **in every direction**, not just towards where it is going.
    assert!(
        (fast.min.x + fast.max.x).abs() < 1e-3,
        "the box has to stay centered on the ball"
    );
    assert!(fast.max.z > still.max.z, "it also grows in z");
}

#[test]
fn rest_is_measured_relative_to_gravity() {
    // The thresholds are scaled by gravity, so a table with more slope does
    // not change what counts as "still".
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.drsq = 0.0;
    assert!(
        b.is_at_rest(GRAVITYCONST),
        "a ball with no velocity is at rest"
    );

    b.vel = Vec3::new(1.0, 0.0, 0.0);
    assert!(!b.is_at_rest(GRAVITYCONST), "moving, it is not");

    // And a ball spinning fast does not count as still either.
    b.vel = Vec3::ZERO;
    b.angular_momentum = Vec3::new(0.0, 0.0, 100.0);
    assert!(!b.is_at_rest(GRAVITYCONST), "spinning, it is not either");
}

#[test]
fn the_step_time_is_one_millisecond() {
    // It is the constant that pins down everything else: the original's
    // `PHYSICS_STEPTIME` is worth 1000 microseconds.
    assert!((PHYSICS_STEPTIME_S - 0.001).abs() < 1e-9);
}

#[test]
fn the_orientation_quaternion_does_not_denormalize() {
    // The reason for having swapped the original's 3x3 matrix for a
    // quaternion. A matrix integrated step by step gets deformed and has to be
    // reorthonormalized with Gram-Schmidt; a quaternion only needs one square
    // root, and on top of that it cannot end up representing something that is
    // not a rotation.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.angular_momentum = Vec3::new(3.0, -1.0, 2.0).normalize() * b.inertia() * 40.0;

    for _ in 0..50_000 {
        b.update_displacements(0.01);
    }

    assert!(
        (b.orientation.length() - 1.0).abs() < 1e-4,
        "the quaternion stopped being a unit one: it measures {}",
        b.orientation.length()
    );
    // And the matrix that comes out of it is still a rotation, not a
    // reflection nor a deformation.
    let m = b.orientation_matrix();
    assert!(
        (m.determinant() - 1.0).abs() < 1e-3,
        "the determinant came out {} and it has to be one",
        m.determinant()
    );
}

#[test]
fn the_ball_spins_about_the_axis_of_its_angular_momentum() {
    // A check that the sign of the integration is the right one: with angular
    // momentum in +z, a point at +x has to go towards +y.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.angular_momentum = Vec3::new(0.0, 0.0, 1.0) * b.inertia();

    // A quarter turn: w = 1 rad per unit of time.
    let target = std::f32::consts::FRAC_PI_2;
    let dt = 0.001;
    for _ in 0..(target / dt) as usize {
        b.update_displacements(dt);
    }

    let rotated = b.orientation * Vec3::X;
    assert!(
        (rotated - Vec3::Y).length() < 0.01,
        "after a quarter turn in +z, +x has to end up at +y; got {rotated:?}"
    );
}

// ------------------------------------------------- the same hit, twice ---

/// `HitObject::FireHitEvent` (`collide.cpp:19-36`).
///
/// A ball resting against something collides with it once per step, for as
/// long as it sits there, and every one of those is the same hit as far as the
/// table is concerned. The original filters them by asking whether the ball has
/// actually been anywhere since the last one.
#[test]
fn a_ball_that_has_not_moved_does_not_fire_the_same_event_again() {
    let mut b = Ball::new(Vec3::new(100.0, 100.0, 25.0), DEFAULT_BALL_SIZE);

    assert!(
        b.fires_event(),
        "the first hit of a ball's life always counts"
    );
    assert!(
        !b.fires_event(),
        "and the second, from the same spot, does not"
    );
    assert!(!b.fires_event());
}

#[test]
fn a_ball_that_has_travelled_fires_again() {
    let mut b = Ball::new(Vec3::new(100.0, 100.0, 25.0), DEFAULT_BALL_SIZE);
    assert!(b.fires_event());

    // The original's threshold is a *squared* distance of 0.25, so it is half
    // a unit of real travel, and the comparison excludes the boundary. Move
    // further than that.
    b.pos.x += 0.6;

    assert!(b.fires_event(), "it went somewhere, so this is a new hit");
}

#[test]
fn travelling_and_coming_back_still_counts_as_somewhere() {
    // The other half of the filter: the accumulated path matters even when the
    // ball ends up back where it started, which is what stops a ball rattling
    // in place from being mistaken for one at rest.
    let mut b = Ball::new(Vec3::new(100.0, 100.0, 25.0), DEFAULT_BALL_SIZE);
    assert!(b.fires_event());

    b.vel = Vec3::new(10.0, 0.0, 0.0);
    b.update_displacements(0.1);
    b.vel = Vec3::new(-10.0, 0.0, 0.0);
    b.update_displacements(0.1);

    assert!(
        b.fires_event(),
        "it travelled two units even though it is back where it was"
    );
}

// ------------------------------------------------- the spin of a resting ball ---

#[test]
fn a_ball_remembers_where_it_was_a_tenth_of_a_second_ago() {
    // Ten slots, one per ten milliseconds (`hitball.h:11`).
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    assert!(
        b.old_position(0).is_none(),
        "a new ball has no history to look back on"
    );

    b.pos = Vec3::new(10.0, 0.0, 25.0);
    b.record_position(0);
    b.pos = Vec3::new(20.0, 0.0, 25.0);
    b.record_position(50);

    assert_eq!(b.old_position(0), Some(Vec3::new(10.0, 0.0, 25.0)));
    assert_eq!(
        b.old_position(9),
        Some(Vec3::new(10.0, 0.0, 25.0)),
        "same slot"
    );
    assert_eq!(b.old_position(50), Some(Vec3::new(20.0, 0.0, 25.0)));
    // A hundred milliseconds later the ring has come round to the same slot.
    b.pos = Vec3::new(30.0, 0.0, 25.0);
    b.record_position(100);
    assert_eq!(b.old_position(0), Some(Vec3::new(30.0, 0.0, 25.0)));
}
