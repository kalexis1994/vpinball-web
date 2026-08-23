//! Tests for the flipper.
//!
//! It is the piece that defines how the game feels, so what gets verified here
//! is not just "it does not blow up" but that it **hits**: that the ball comes
//! out faster than it went in, that a flipper at rest does not give back a
//! dead bounce, and that spin gets transmitted.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::collision::{CollisionEvent, Material};
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY, PHYS_FACTOR};
use vpw_physics::flipper::{Flipper, FlipperParams};

const R: f32 = DEFAULT_BALL_SIZE;

/// A flipper with measurements close to those of a real table.
fn flipper() -> Flipper {
    Flipper::new(FlipperParams {
        center: Vec2::ZERO,
        base_radius: 21.0,
        end_radius: 13.0,
        flipper_radius: 130.0,
        angle_start: (-20.0f32).to_radians(),
        angle_end: (50.0f32).to_radians(),
        z_low: 0.0,
        z_high: 50.0,
        mass: 1.0,
        strength: 2200.0,
        material: Material {
            elasticity: 0.8,
            elasticity_falloff: 0.43,
            friction: 0.6,
            scatter: 0.0,
        },
        ..FlipperParams::default()
    })
}

/// One complete step of the flipper.
fn step(f: &mut Flipper) {
    f.update_velocities();
    f.update_displacements(PHYS_FACTOR);
}

/// A ball placed **in the path of the sweep**, resting against the tip.
///
/// Where it is placed matters: the flipper rotates, so its tip moves
/// **perpendicular** to the arm. A ball right on top of the tip is in the
/// direction the arm points at, not the one it sweeps through, and there the
/// flipper runs straight past without touching it. Putting the ball there and
/// expecting a hit is a mistake in the setup, not in the engine.
fn ball_in_the_path(f: &Flipper) -> Ball {
    let tip = f.tip_center();
    let arm = tip - f.center;
    // The direction the tip moves in on the way up.
    let sweep = Vec2::new(-arm.y, arm.x).normalize();
    let p = tip + sweep * (R + f.end_radius);
    let mut b = Ball::new(Vec3::new(p.x, p.y, 25.0), R);
    // Arriving slowly against the tip.
    b.vel = Vec3::new(-sweep.x, -sweep.y, 0.0);
    b
}

#[test]
fn the_flipper_starts_at_rest_and_resting_against_its_stop() {
    let mut f = flipper();
    assert_eq!(f.angle_cur, f.angle_start);

    // Without the button pressed, the spring holds it against the lower stop.
    for _ in 0..500 {
        step(&mut f);
    }
    assert!(
        (f.angle_cur - f.angle_start).abs() < 1e-3,
        "it should still be at rest; it ended at {} degrees",
        f.angle_cur.to_degrees()
    );
    assert!(f.in_contact, "and resting against the stop");
    assert!(f.angle_speed.abs() < 1e-3, "without moving");
}

#[test]
fn pressing_the_button_raises_the_flipper() {
    let mut f = flipper();
    f.solenoid = true;

    for _ in 0..100 {
        step(&mut f);
    }

    assert!(
        (f.angle_cur - f.angle_end).abs() < 1e-2,
        "it should have reached the upper stop; it ended at {} degrees",
        f.angle_cur.to_degrees()
    );
}

#[test]
fn the_flipper_takes_as_long_as_a_real_flipper() {
    // A flipper on a real table travels its arc in some 30 to 60
    // milliseconds. Much faster feels unreal; much slower, mushy.
    let mut f = flipper();
    f.solenoid = true;

    let mut ms = 0;
    while (f.angle_cur - f.angle_end).abs() > 1e-2 && ms < 500 {
        step(&mut f);
        ms += 1;
    }

    assert!(
        (10..=120).contains(&ms),
        "the travel took {ms} ms, and a real flipper takes between 30 and 60"
    );
}

#[test]
fn releasing_the_button_brings_it_back_slower_than_it_went_up() {
    // The return is a spring, not the coil: it has a fraction of the force.
    // That is why a flipper comes down slower than it goes up.
    let mut going_up = 0;
    let mut f = flipper();
    f.solenoid = true;
    while (f.angle_cur - f.angle_end).abs() > 1e-2 && going_up < 500 {
        step(&mut f);
        going_up += 1;
    }

    f.solenoid = false;
    let mut coming_down = 0;
    while (f.angle_cur - f.angle_start).abs() > 1e-2 && coming_down < 5000 {
        step(&mut f);
        coming_down += 1;
    }

    assert!(
        coming_down > going_up * 2,
        "coming down should cost quite a bit more than going up: {going_up} vs {coming_down}"
    );
}

#[test]
fn the_flipper_does_not_go_past_its_stops() {
    let mut f = flipper();
    let (min, max) = (
        f.angle_start.min(f.angle_end),
        f.angle_start.max(f.angle_end),
    );

    // Press and release many times, at irregular intervals.
    for i in 0..2000 {
        f.solenoid = (i / 37) % 2 == 0;
        step(&mut f);
        assert!(
            f.angle_cur >= min - 1e-4 && f.angle_cur <= max + 1e-4,
            "it went past the range on step {i}: {} degrees",
            f.angle_cur.to_degrees()
        );
    }
}

#[test]
fn a_moving_flipper_hits_the_ball() {
    // **The** flipper test: the ball has to come out faster than it went in.
    // If it comes out the same or slower, the flipper does not hit.
    let mut f = flipper();
    f.solenoid = true;
    // Take it to mid-travel, which is where it has the most angular velocity.
    for _ in 0..20 {
        step(&mut f);
    }
    assert!(f.angle_speed > 0.0, "the flipper has to be going up");

    let mut b = ball_in_the_path(&f);
    let before = b.vel.length();

    let coll = f
        .hit_test(&b, 1.0)
        .expect("the ball has to be touching the flipper");
    f.collide(&mut b, &coll);

    assert!(
        b.vel.length() > before * 5.0,
        "the flipper has to kick the ball: it went in at {before:.2} and came out at {:.2}",
        b.vel.length()
    );
}

#[test]
fn a_flipper_at_rest_does_not_give_back_a_dead_bounce() {
    // This is the detail from `hitflipper.cpp:908-927`. When the flipper is
    // against its stop and the ball pushes it in the direction the solenoid is
    // already pushing, no energy is transferred to it: it has nowhere to move.
    // Without that, the ball "lends" energy to something immovable and the
    // bounce comes out dull.
    let mut f = flipper();
    // Let it settle against the rest stop.
    for _ in 0..500 {
        step(&mut f);
    }
    assert!(f.in_contact);

    let mut b = ball_in_the_path(&f);
    b.vel *= 20.0; // arriving hard against the tip
    let incoming = b.vel;
    let before = incoming.length();

    let coll = f.hit_test(&b, 1.0).expect("it has to touch");
    f.collide(&mut b, &coll);

    // With elasticity 0.8 and falloff, the bounce has to keep a good part of
    // the velocity. A dead bounce would be less than half.
    assert!(
        b.vel.length() > before * 0.5,
        "dead bounce: it went in at {before:.1} and came out at {:.1}",
        b.vel.length()
    );
    // And it has to come back, not carry straight on.
    assert!(
        b.vel.dot(incoming) < 0.0,
        "it carried straight on: it went in {incoming:?} and came out {:?}",
        b.vel
    );
}

#[test]
fn the_flipper_puts_spin_on_the_ball() {
    // The friction between the rotating tip and the ball is what puts spin on
    // it. Without that, every ball comes out without spinning and they all
    // behave the same.
    let mut f = flipper();
    f.solenoid = true;
    for _ in 0..20 {
        step(&mut f);
    }

    let mut b = ball_in_the_path(&f);
    assert_eq!(b.angular_momentum, Vec3::ZERO);

    let coll = f.hit_test(&b, 1.0).expect("it has to touch");
    f.collide(&mut b, &coll);

    assert!(
        b.angular_momentum.length() > 0.0,
        "the hit has to leave the ball spinning"
    );
}

#[test]
fn the_hit_brakes_the_flipper() {
    // Conservation: if the ball comes out with more momentum, the flipper has
    // to have lost some of its own.
    let mut f = flipper();
    f.solenoid = true;
    for _ in 0..20 {
        step(&mut f);
    }
    let momentum_before = f.angular_momentum;

    let mut b = ball_in_the_path(&f);

    let coll = f.hit_test(&b, 1.0).expect("it has to touch");
    f.collide(&mut b, &coll);

    assert!(
        f.angular_momentum < momentum_before,
        "the flipper should brake: it went from {momentum_before:.1} to {:.1}",
        f.angular_momentum
    );
}

#[test]
fn the_ball_hits_the_base_too() {
    // A flipper is two circles. The base is fixed and it also stops the ball.
    let f = flipper();
    let mut b = Ball::new(Vec3::new(0.0, 100.0, 25.0), R);
    b.vel = Vec3::new(0.0, -50.0, 0.0);

    let coll = f
        .hit_test(&b, 10.0)
        .expect("it has to hit against the base");
    assert!(coll.hit_time > 0.0);
    // The normal points from the center of the flipper towards the ball.
    assert!(coll.hit_normal.y > 0.9, "got {:?}", coll.hit_normal);
}

#[test]
fn a_ball_far_away_does_not_hit() {
    let f = flipper();
    let mut b = Ball::new(Vec3::new(500.0, 500.0, 25.0), R);
    b.vel = Vec3::new(0.0, -50.0, 0.0);
    assert!(f.hit_test(&b, 1.0).is_none());
}

#[test]
fn a_ball_above_the_flipper_does_not_hit() {
    let f = flipper();
    let tip = f.tip_center();
    let mut b = Ball::new(Vec3::new(tip.x, tip.y, 300.0), R);
    b.vel = Vec3::new(0.0, -50.0, 0.0);
    assert!(f.hit_test(&b, 1.0).is_none(), "it passes far over the top");
}

#[test]
fn the_tip_moves_with_the_angle() {
    let mut f = flipper();
    let at_rest = f.tip_center();

    f.solenoid = true;
    for _ in 0..200 {
        step(&mut f);
    }
    let up = f.tip_center();

    assert!(
        (up - at_rest).length() > 50.0,
        "the tip should have moved quite a bit; it moved {}",
        (up - at_rest).length()
    );
    // And always at the same distance from the axis.
    assert!((at_rest.length() - f.flipper_radius).abs() < 1e-3);
    assert!((up.length() - f.flipper_radius).abs() < 1e-3);
}

#[test]
fn the_middle_of_the_arm_hits_too() {
    // It is what the two flat faces contribute. Without them, a flipper is two
    // circles with a gap in between: a ball resting halfway along the arm
    // finds nothing to hit against, and the flipper passes underneath.
    let mut f = flipper();
    f.solenoid = true;
    for _ in 0..20 {
        step(&mut f);
    }

    // A point halfway between the base and the tip, shifted towards the side
    // it sweeps through.
    let tip = f.tip_center();
    let arm = tip - f.center;
    let sweep = Vec2::new(-arm.y, arm.x).normalize();
    let mid = f.center + arm * 0.5 + sweep * (R + 15.0);

    let mut b = Ball::new(Vec3::new(mid.x, mid.y, 25.0), R);
    b.vel = Vec3::new(-sweep.x, -sweep.y, 0.0) * 5.0;
    let before = b.vel.length();

    let coll = f
        .hit_test(&b, 1.0)
        .expect("the middle of the arm has to exist as far as the ball is concerned");
    f.collide(&mut b, &coll);

    assert!(
        b.vel.length() > before,
        "the middle of the arm also has to push: it went in at {before:.1}, came out at {:.1}",
        b.vel.length()
    );
}

#[test]
fn the_flippers_two_faces_point_opposite_ways() {
    // A flipper has two faces and each one stops things on its own side. A
    // ball arriving from the left and one from the right have to find opposite
    // normals.
    let f = flipper();
    let tip = f.tip_center();
    let arm = tip - f.center;
    let side = Vec2::new(-arm.y, arm.x).normalize();
    let mid = f.center + arm * 0.5;

    let mut normals = Vec::new();
    for sign in [1.0f32, -1.0] {
        let p = mid + side * (sign * (R + 15.0));
        let mut b = Ball::new(Vec3::new(p.x, p.y, 25.0), R);
        b.vel = Vec3::new(-side.x * sign, -side.y * sign, 0.0) * 20.0;
        if let Some(c) = f.hit_test(&b, 1.0) {
            normals.push(c.hit_normal);
        }
    }

    assert_eq!(normals.len(), 2, "both faces have to respond");
    assert!(
        normals[0].dot(normals[1]) < -0.5,
        "the normals have to be nearly opposite: {:?} and {:?}",
        normals[0],
        normals[1]
    );
}

/// The stroke time, in milliseconds.
fn stroke_ms() -> u32 {
    let mut f = flipper();
    f.solenoid = true;
    let mut ms = 0;
    while (f.angle_cur - f.angle_end).abs() > 1e-2 && ms < 500 {
        step(&mut f);
        ms += 1;
    }
    ms
}

#[test]
fn the_damping_angle_is_in_radians() {
    // `hitflipper.cpp:369` converts it with `ANGTORAD` before comparing it
    // against an angle in radians. Left in degrees, the default of 6 is larger
    // than a flipper's entire travel — about 1.2 radians — so the branch below
    // it is taken on every step of every stroke and the coil runs at the hold
    // torque from the instant the button goes down.
    //
    // This asserts the unit rather than a behaviour on purpose. Measured on a
    // stock flipper the two spellings differ by one millisecond of stroke and
    // not at all in the speed the ball comes off the tip, because `ramp_up`
    // caps the torque well below the point where the damping factor bites:
    // 2200 strength over a ramp of 3 is 73 units of torque per step, so
    // reaching full force takes 30 ms of a 38 ms stroke. The divergence is real
    // and the correction is not negotiable, but it shows up in how hard a held
    // flipper pushes back, not in how hard it hits.
    let f = flipper();
    assert!(
        f.torque_damping_angle < 0.5,
        "a damping angle of {} is degrees, not radians",
        f.torque_damping_angle
    );
}

#[test]
fn the_stroke_takes_as_long_as_a_real_flippers() {
    let ms = stroke_ms();
    assert!(
        (25..=70).contains(&ms),
        "the stroke took {ms} ms, and a real flipper takes between 30 and 60"
    );
}

// ------------------------------------------- a ball resting on the arm ---

/// Gravity on a table with the usual six degrees of slope: mostly down, and a
/// little towards the player, which is what presses a ball onto a flipper.
fn sloped_gravity() -> Vec3 {
    let r = 6.0f32.to_radians();
    Vec3::new(
        0.0,
        r.sin() * DEFAULT_TABLE_GRAVITY,
        -r.cos() * DEFAULT_TABLE_GRAVITY,
    )
}

#[test]
fn a_ball_leaning_on_the_arm_pushes_back_on_it() {
    // A ball resting on a flipper, held there by the table's slope. That is a
    // contact, and `HitFlipper::Contact` (`hitflipper.cpp:1012`) resolves both
    // sides of it: the arm holds the ball up, and the ball's weight reaches the
    // arm.
    //
    // The generic contact handler can only do the first half — it treats
    // whatever the ball is lying on as an immovable wall — so before this the
    // flipper never felt anything resting on it and never lifted anything as it
    // started to move.
    //
    // The collision is built by hand rather than found by `hit_test`, so the
    // contact normal is known and the test is about the solve and not about
    // where a ball happens to land on a particular flipper.
    let mut f = flipper();
    let tip = f.tip_center();

    // Uphill of the tip, touching it: the slope presses it against the arm.
    let normal = Vec3::new(0.0, -1.0, 0.0);
    let p = tip - Vec2::new(0.0, R + f.end_radius);
    let mut b = Ball::new(Vec3::new(p.x, p.y, 25.0), R);

    let coll = CollisionEvent {
        hit_time: 0.0,
        hit_normal: normal,
        // Touching, not embedded: anything below zero triggers the original's
        // kick for a ball pushed into a resting flipper (`hitflipper.cpp:1019`),
        // which by itself takes the pair past the contact threshold.
        hit_distance: 0.0,
        is_contact: true,
        hit_flag: false,
        hit_org_normal_velocity: 0.0,
        hit_vel: Vec2::ZERO,
    };

    let flipper_before = f.angular_momentum;
    f.contact(&mut b, &coll, 0.01, sloped_gravity());

    assert!(
        b.vel.length() > 0.0,
        "the arm has to hold the ball up against the slope"
    );
    assert!(
        f.angular_momentum != flipper_before,
        "and the ball leaning on it has to reach the arm"
    );
}
