//! Tests for the collision shapes.
//!
//! Every shape answers *when* the ball hits and *against which normal*. Both
//! can be verified against elementary geometry, so the cases in here are
//! computable by hand.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::collision::Material;
use vpw_physics::constants::DEFAULT_BALL_SIZE;
use vpw_physics::shapes::{HitCircle, HitPlane, HitPoint, HitPoly, LineSeg, solve_quadratic};

const R: f32 = 25.0;

fn ball(pos: Vec3, vel: Vec3) -> Ball {
    let mut b = Ball::new(pos, R);
    b.vel = vel;
    b
}

/// A vertical wall at x = 0, running along y and **facing +X**.
///
/// The order of the two points defines which way it faces: with `v1` up top
/// and `v2` down below, the normal comes out towards +X. The other way round,
/// the wall is passable from this side.
fn wall() -> LineSeg {
    LineSeg::new(
        Vec2::new(0.0, 100.0),
        Vec2::new(0.0, -100.0),
        0.0,
        50.0,
        Material::default(),
    )
}

#[test]
fn the_order_of_the_points_defines_which_way_the_wall_faces() {
    // It is the detail that makes a wall stop things on one side and be
    // passable on the other. Flipping the order flips the normal.
    let a = LineSeg::new(
        Vec2::new(0.0, 100.0),
        Vec2::new(0.0, -100.0),
        0.0,
        50.0,
        Material::default(),
    );
    let b = LineSeg::new(
        Vec2::new(0.0, -100.0),
        Vec2::new(0.0, 100.0),
        0.0,
        50.0,
        Material::default(),
    );
    assert!(
        (a.normal - Vec2::X).length() < 1e-5,
        "one faces +X; got {:?}",
        a.normal
    );
    assert!(
        (b.normal + Vec2::X).length() < 1e-5,
        "the other -X; got {:?}",
        b.normal
    );

    // And a ball coming from +X only hits the first one.
    let ball_from_x = ball(Vec3::new(100.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));
    assert!(
        a.hit_test(&ball_from_x, 100.0).is_some(),
        "it hits the front face"
    );
    assert!(
        b.hit_test(&ball_from_x, 100.0).is_none(),
        "and goes through the back one"
    );
}

#[test]
fn a_walls_normal_is_perpendicular_and_a_unit_vector() {
    let p = wall();
    assert!((p.normal.length() - 1.0).abs() < 1e-5);
    let direction = (p.v2 - p.v1).normalize();
    assert!(
        p.normal.dot(direction).abs() < 1e-5,
        "the normal has to be perpendicular to the segment"
    );
    assert!((p.length - 200.0).abs() < 1e-4);
}

#[test]
fn a_ball_coming_straight_on_hits_when_it_should() {
    // The ball starts 100 from the wall, with radius 25: its edge is at 75.
    // At velocity 10, it takes 7.5 units of time to touch it.
    let p = wall();
    let b = ball(Vec3::new(100.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));

    let c = p.hit_test(&b, 100.0).expect("it has to hit");
    assert!(
        (c.hit_time - 7.5).abs() < 1e-3,
        "it should have hit at 7.5; got {}",
        c.hit_time
    );
    assert!(
        (c.hit_normal - Vec3::X).length() < 1e-5,
        "the normal points towards the ball; got {:?}",
        c.hit_normal
    );
}

#[test]
fn a_ball_moving_away_does_not_hit() {
    let p = wall();
    let b = ball(Vec3::new(100.0, 0.0, 25.0), Vec3::new(10.0, 0.0, 0.0));
    assert!(p.hit_test(&b, 100.0).is_none());
}

#[test]
fn a_hit_outside_the_step_does_not_count() {
    // The ball is going to hit, but not for another 75 units of time, and the
    // step lasts 1: it is not this step's business.
    let p = wall();
    let b = ball(Vec3::new(100.0, 0.0, 25.0), Vec3::new(-1.0, 0.0, 0.0));
    assert!(
        p.hit_test(&b, 1.0).is_none(),
        "the hit falls outside the step"
    );
    assert!(p.hit_test(&b, 100.0).is_some(), "with more time it does");
}

#[test]
fn a_ball_passing_beyond_the_segment_does_not_hit() {
    // The segment goes from y=-100 to y=100. A ball at y=500 is aimed at the
    // plane that contains the wall, but not at the wall.
    let p = wall();
    let b = ball(Vec3::new(100.0, 500.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));
    assert!(
        p.hit_test(&b, 100.0).is_none(),
        "it runs past the end of the segment"
    );
}

#[test]
fn a_ball_flying_over_the_wall_does_not_hit() {
    // The wall reaches up to z=50; the ball goes past at z=200.
    let p = wall();
    let b = ball(Vec3::new(100.0, 0.0, 200.0), Vec3::new(-10.0, 0.0, 0.0));
    assert!(p.hit_test(&b, 100.0).is_none(), "it passes over the top");
}

#[test]
fn a_ball_resting_on_the_wall_gives_a_contact_and_not_a_hit() {
    // Practically touching and with almost no normal velocity: it is a
    // sustained contact, not an impact.
    let p = wall();
    let b = ball(Vec3::new(25.01, 0.0, 25.0), Vec3::new(-0.01, 0.0, 0.0));

    let c = p.hit_test(&b, 1.0).expect("it has to detect something");
    assert!(c.is_contact, "it has to be a contact, not a hit");
    assert!(
        (c.hit_org_normal_velocity - (-0.01)).abs() < 1e-4,
        "the contact keeps the normal velocity in order to hold the ball up"
    );

    // The time is **not** zero, and that is on purpose: inside the contact
    // band the original spreads the time out with `bnd·10 + 0.5`
    // (`collide.cpp:96`) so these cases do not compete with the real zero-time
    // events, which are the actual hits.
    let expected = 0.01 * (1.0 / (2.0 * vpw_physics::constants::PHYS_TOUCH)) + 0.5;
    assert!(
        (c.hit_time - expected).abs() < 1e-3,
        "got {} and expected {expected}",
        c.hit_time
    );
    assert!(
        c.hit_time > 0.0 && c.hit_time < 1.0,
        "and it falls inside the band"
    );
}

#[test]
fn a_ball_too_deeply_sunk_is_discarded() {
    // A guard against balls that slipped in through an earlier bug: if it is
    // further in than its own radius, the hit is not resolved.
    let p = wall();
    let b = ball(Vec3::new(-50.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));
    assert!(p.hit_test(&b, 100.0).is_none());
}

#[test]
fn the_circle_hits_at_the_distance_of_the_two_radii() {
    // A circle of radius 30 at the origin, a ball of radius 25 at a distance
    // of 155. They touch when the distance between centers is 55, that is,
    // after travelling 100. At velocity 10, that is 10 units of time.
    let c = HitCircle {
        center: Vec2::ZERO,
        radius: 30.0,
        z_low: 0.0,
        z_high: 50.0,
        material: Material::default(),
    };
    let b = ball(Vec3::new(155.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));

    let e = c.hit_test(&b, 100.0).expect("it has to hit");
    assert!((e.hit_time - 10.0).abs() < 1e-2, "got {}", e.hit_time);
    assert!(
        (e.hit_normal - Vec3::X).length() < 1e-4,
        "the normal goes from the center towards the ball; got {:?}",
        e.hit_normal
    );
}

#[test]
fn the_circle_does_not_hit_if_the_ball_runs_past() {
    let c = HitCircle {
        center: Vec2::ZERO,
        radius: 30.0,
        z_low: 0.0,
        z_high: 50.0,
        material: Material::default(),
    };
    // It goes past at a lateral distance of 200: they never get close enough.
    let b = ball(Vec3::new(155.0, 200.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));
    assert!(c.hit_test(&b, 100.0).is_none());
}

#[test]
fn the_circles_normal_always_points_outwards() {
    let c = HitCircle {
        center: Vec2::ZERO,
        radius: 30.0,
        z_low: 0.0,
        z_high: 50.0,
        material: Material::default(),
    };
    // From several directions, the normal has to go from the center to the
    // ball.
    for angle in [0.0f32, 1.0, 2.5, 4.0, 5.5] {
        let (s, co) = angle.sin_cos();
        let dir = Vec3::new(co, s, 0.0);
        let b = ball(dir * 155.0 + Vec3::new(0.0, 0.0, 25.0), dir * -10.0);
        let e = c.hit_test(&b, 100.0).expect("it has to hit");
        assert!(
            (e.hit_normal - dir).length() < 1e-3,
            "at angle {angle} the normal gave {:?} and {dir:?} was expected",
            e.hit_normal
        );
    }
}

#[test]
fn the_playfield_plane_stops_the_fall() {
    // The playfield is the plane z=0 with the normal pointing up.
    let p = HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material: Material::default(),
    };
    // A ball at height 125, radius 25: its edge is 100 from the plane.
    let b = ball(Vec3::new(0.0, 0.0, 125.0), Vec3::new(0.0, 0.0, -10.0));

    let e = p.hit_test(&b, 100.0).expect("it has to hit");
    assert!((e.hit_time - 10.0).abs() < 1e-3, "got {}", e.hit_time);
    assert!((e.hit_normal - Vec3::Z).length() < 1e-6);
}

#[test]
fn a_ball_resting_on_the_playfield_gives_a_contact() {
    let p = HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material: Material::default(),
    };
    let b = ball(Vec3::new(0.0, 0.0, 25.0), Vec3::new(0.0, 0.0, -0.01));

    let e = p.hit_test(&b, 1.0).expect("it has to detect something");
    assert!(e.is_contact, "resting is a contact");
    assert!(e.hit_distance.abs() < 1e-3, "and at distance zero");
}

#[test]
fn the_point_catches_the_ball_that_goes_through_the_corner() {
    // Without the points, a ball that arrives right at the seam between two
    // walls slips through. The point is a sphere of radius zero.
    let p = HitPoint {
        p: Vec3::new(0.0, 0.0, 25.0),
        material: Material::default(),
    };
    let b = ball(Vec3::new(125.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));

    let e = p.hit_test(&b, 100.0).expect("it has to hit");
    assert!((e.hit_time - 10.0).abs() < 1e-2, "got {}", e.hit_time);
    assert!((e.hit_normal - Vec3::X).length() < 1e-3);
}

#[test]
fn the_quadratic_returns_the_two_roots_in_order() {
    // t² - 3t + 2 = 0  ->  t = 1 and t = 2
    let (t1, t2) = solve_quadratic(1.0, -3.0, 2.0).expect("it has roots");
    assert!((t1 - 1.0).abs() < 1e-5, "the first one gave {t1}");
    assert!((t2 - 2.0).abs() < 1e-5, "the second one gave {t2}");
    assert!(t1 <= t2, "they have to come in order");
}

#[test]
fn the_quadratic_with_no_real_roots_returns_nothing() {
    // t² + 1 = 0 has no real solution: the ball never gets to touch.
    assert!(solve_quadratic(1.0, 0.0, 1.0).is_none());
}

#[test]
fn a_captured_ball_does_not_hit_anything() {
    let mut b = ball(Vec3::new(100.0, 0.0, 25.0), Vec3::new(-10.0, 0.0, 0.0));
    b.locked = true;

    assert!(wall().hit_test(&b, 100.0).is_none());
    let c = HitCircle {
        center: Vec2::ZERO,
        radius: 30.0,
        z_low: 0.0,
        z_high: 50.0,
        material: Material::default(),
    };
    assert!(c.hit_test(&b, 100.0).is_none());
    let p = HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material: Material::default(),
    };
    assert!(p.hit_test(&b, 100.0).is_none());
}

#[test]
fn the_bounding_boxes_contain_the_shape() {
    let p = wall();
    let c = p.bbox();
    assert!(c.min.x <= 0.0 && c.max.x >= 0.0);
    assert!(c.min.y <= -100.0 && c.max.y >= 100.0);
    assert!(c.min.z <= 0.0 && c.max.z >= 50.0);

    let ci = HitCircle {
        center: Vec2::new(10.0, 20.0),
        radius: 30.0,
        z_low: 5.0,
        z_high: 45.0,
        material: Material::default(),
    };
    let c = ci.bbox();
    assert!((c.min.x - (-20.0)).abs() < 1e-5);
    assert!((c.max.y - 50.0).abs() < 1e-5);
    assert!((c.min.z - 5.0).abs() < 1e-5);
}

// ------------------------------------------------------------------ lids ---

/// A flat quad wound so that its face points up, which is what a wall's top
/// lid has to do.
///
/// `Hit3DPoly::Init` (`collideex.cpp:494-511`) runs Newell's method and then
/// **negates** it, so the normal is `(v2-v0) x (v1-v0)` — the opposite of the
/// cross product most code reaches for. Clockwise seen from above is therefore
/// the winding that faces upward, and it is the winding a `.vpx`'s wall
/// outlines arrive in.
fn lid(z: f32) -> HitPoly {
    HitPoly::new(
        vec![
            Vec3::new(0.0, 0.0, z),
            Vec3::new(0.0, 100.0, z),
            Vec3::new(100.0, 100.0, z),
            Vec3::new(100.0, 0.0, z),
        ],
        Material {
            elasticity: 0.3,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        },
    )
    .expect("four corners are not degenerate")
}

#[test]
fn the_normal_is_the_originals_and_not_its_opposite() {
    // `collideex.cpp:509`, "NOTE: normal is flipped!". Take the cross product
    // the other way round and every face built from this blocks from the wrong
    // side: measured on F-14, all thirty-one collidable walls let a ball
    // dropped on them fall straight through.
    let verts = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(0.0, 10.0, 0.0),
    ];
    let p = HitPoly::new(
        verts.to_vec(),
        Material {
            elasticity: 0.3,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        },
    )
    .expect("not degenerate");
    let expected = (verts[2] - verts[0]).cross(verts[1] - verts[0]).normalize();
    assert!((p.normal - expected).length() < 1e-5, "got {:?}", p.normal);
}

#[test]
fn a_lid_faces_up_and_catches_a_ball_falling_onto_it() {
    // The whole point of a wall's top. Face it the other way and the ball goes
    // straight through — which is what every one of F-14's thirty-one walls did
    // before the normal was put the right way round.
    let p = lid(50.0);
    assert!(
        p.normal.z > 0.9,
        "the normal points up, not down: {:?}",
        p.normal
    );

    let mut b = Ball::new(Vec3::new(50.0, 50.0, 90.0), DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -10.0); // falling

    let hit = p.hit_test(&b, 10.0).expect("it has to catch the ball");
    assert!(hit.hit_normal.z > 0.9, "and push it back up");
}

#[test]
fn a_lid_ignores_a_ball_coming_from_underneath() {
    // One-sidedness is the other half: a wall's top is not a ceiling.
    let p = lid(50.0);
    let mut b = Ball::new(Vec3::new(50.0, 50.0, 10.0), DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, 10.0); // rising towards it from below

    assert!(p.hit_test(&b, 10.0).is_none());
}

#[test]
fn a_lid_only_catches_what_is_over_it() {
    let p = lid(50.0);
    let mut b = Ball::new(Vec3::new(500.0, 500.0, 90.0), DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -10.0);

    assert!(
        p.hit_test(&b, 10.0).is_none(),
        "it is nowhere near the quad"
    );
}

#[test]
fn a_seam_catches_a_ball_that_is_only_creeping() {
    // `collide.cpp:529` rejects a `HitPoint` at `C_CONTACTVEL`, not at
    // `C_LOWNORMVEL`. The gap between the two is where a ball rolling slowly
    // into the join between two wall segments lives, and losing it there is how
    // a ball sinks into a corner.
    let p = HitPoint {
        p: Vec3::ZERO,
        material: Material {
            elasticity: 0.3,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        },
    };
    let mut b = Ball::new(
        Vec3::new(DEFAULT_BALL_SIZE + 0.02, 0.0, 0.0),
        DEFAULT_BALL_SIZE,
    );
    // Creeping *away* at a speed between the two thresholds: 0.0001 < v < 0.099.
    b.vel = Vec3::new(0.05, 0.0, 0.0);

    assert!(
        p.hit_test(&b, 1.0).is_some(),
        "a ball this slow still has to be in contact with the seam"
    );
}
