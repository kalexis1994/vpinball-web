//! Tests for the simulation loop.
//!
//! Here we can already simulate end to end: build a table, drop a ball and
//! look at where it ends up. It is the part of the project where verification
//! works best, because the answer comes from mechanics and not from the eye.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::collision::Material;
use vpw_physics::constants::DEFAULT_TABLE_GRAVITY;
use vpw_physics::engine::{Engine, Shape};
use vpw_physics::shapes::{HitCircle, HitPlane, LineSeg};

const R: f32 = 25.0;

/// The playfield: the plane z = 0.
fn playfield(material: Material) -> Shape {
    Shape::Plane(HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material,
    })
}

/// A box of walls that encloses the table, **facing inwards**.
///
/// The direction the outline is traversed in defines which way the walls face,
/// and a wall only stops things on its front face. Traversed the other way,
/// this same box holds nothing in: the ball goes through it as if it were not
/// there. It is worth verifying, and there is a test below that does.
fn walls(width: f32, length: f32, height: f32, material: Material) -> Vec<Shape> {
    let e = [
        (Vec2::new(0.0, 0.0), Vec2::new(width, 0.0)), // bottom, faces +Y
        (Vec2::new(width, 0.0), Vec2::new(width, length)), // right, faces -X
        (Vec2::new(width, length), Vec2::new(0.0, length)), // top, faces -Y
        (Vec2::new(0.0, length), Vec2::new(0.0, 0.0)), // left, faces +X
    ];
    e.into_iter()
        .map(|(a, b)| Shape::Line(LineSeg::new(a, b, 0.0, height, material)))
        .collect()
}

#[test]
fn the_box_walls_face_inwards() {
    // This pins down the premise of every test below. If the outline is
    // traversed the other way, the walls face outwards and the table holds
    // nothing in — and the tests would fail for a reason that has nothing to
    // do with what they mean to prove.
    let center = Vec2::new(500.0, 1000.0);
    for shape in walls(1000.0, 2000.0, 100.0, Material::default()) {
        let Shape::Line(l) = shape else {
            unreachable!()
        };
        let mid = (l.v1 + l.v2) * 0.5;
        let towards_center = (center - mid).normalize();
        assert!(
            l.normal.dot(towards_center) > 0.9,
            "the wall {:?}-{:?} faces outwards: normal {:?}",
            l.v1,
            l.v2,
            l.normal
        );
    }
}

fn table(material: Material) -> Engine {
    let mut shapes = vec![playfield(material)];
    shapes.extend(walls(1000.0, 2000.0, 100.0, material));
    Engine::new(
        shapes,
        Engine::gravity_from_slope(6.0, DEFAULT_TABLE_GRAVITY),
    )
}

/// Runs `n` physics steps, that is, `n` table milliseconds.
fn run(e: &mut Engine, n: usize) {
    for _ in 0..n {
        e.step();
    }
}

#[test]
fn a_dropped_ball_falls_to_the_playfield_and_stays() {
    let mut e = table(Material {
        elasticity: 0.0,
        friction: 0.3,
        ..Material::default()
    });
    e.add_ball(Ball::new(Vec3::new(500.0, 1000.0, 400.0), R));

    run(&mut e, 3000);

    let b = &e.balls[0];
    assert!(
        (b.pos.z - R).abs() < 2.0,
        "it should come to rest at the height of its radius; it ended at z={}",
        b.pos.z
    );
    assert!(b.pos.z > 0.0, "and not go through the playfield");
}

#[test]
fn a_ball_does_not_go_through_the_playfield_even_falling_very_fast() {
    // This is the proof that the loop looks for the hit before moving: with a
    // fixed step, a ball at this velocity would run straight past.
    let mut e = table(Material::default());
    let mut b = Ball::new(Vec3::new(500.0, 1000.0, 500.0), R);
    b.vel = Vec3::new(0.0, 0.0, -5000.0);
    e.add_ball(b);

    run(&mut e, 100);

    assert!(
        e.balls[0].pos.z > 0.0,
        "it slipped through the playfield: it ended at z={}",
        e.balls[0].pos.z
    );
}

#[test]
fn a_fast_ball_does_not_go_through_a_wall() {
    let mut e = table(Material::default());
    let mut b = Ball::new(Vec3::new(500.0, 1000.0, 50.0), R);
    b.vel = Vec3::new(8000.0, 0.0, 0.0); // straight at the wall at x=1000
    e.add_ball(b);

    run(&mut e, 200);

    assert!(
        e.balls[0].pos.x < 1000.0,
        "it went through the wall: it ended at x={}",
        e.balls[0].pos.x
    );
}

#[test]
fn the_ball_rolls_down_the_slope() {
    // The table is tilted six degrees: a dropped ball has to head towards the
    // player, that is, towards increasing `y`.
    let mut e = table(Material {
        elasticity: 0.0,
        friction: 0.05,
        ..Material::default()
    });
    e.add_ball(Ball::new(Vec3::new(500.0, 1000.0, R), R));
    let y0 = e.balls[0].pos.y;

    run(&mut e, 2000);

    assert!(
        e.balls[0].pos.y > y0 + 10.0,
        "it should have come down; it went from y={y0} to y={}",
        e.balls[0].pos.y
    );
}

#[test]
fn the_ball_ends_up_against_the_bottom_wall() {
    // Left to run, the slope takes it all the way down and there it stays.
    let mut e = table(Material {
        elasticity: 0.1,
        friction: 0.1,
        ..Material::default()
    });
    e.add_ball(Ball::new(Vec3::new(500.0, 200.0, R), R));

    run(&mut e, 20_000);

    let b = &e.balls[0];
    assert!(
        b.pos.y > 1900.0,
        "it should be against the far wall; it ended at y={}",
        b.pos.y
    );
    assert!(b.pos.y < 2000.0, "but on this side of the wall");
}

#[test]
fn a_ball_shut_in_never_escapes() {
    // The hardest test of the lot: it is thrown hard in some arbitrary
    // direction and left to bounce for a long time. Any sign error in a
    // normal, or a hit that gets missed, ends with the ball outside.
    let mut e = table(Material {
        elasticity: 0.9,
        friction: 0.1,
        ..Material::default()
    });
    let mut b = Ball::new(Vec3::new(500.0, 1000.0, 50.0), R);
    b.vel = Vec3::new(900.0, 700.0, 0.0);
    e.add_ball(b);

    for _ in 0..30_000 {
        e.step();
        let p = e.balls[0].pos;
        assert!(
            p.x > -50.0 && p.x < 1050.0 && p.y > -50.0 && p.y < 2050.0 && p.z > -50.0,
            "the ball escaped from the table: {p:?}"
        );
    }
}

#[test]
fn the_ball_bounces_off_a_bumper() {
    // A circle in the middle of the table: the ball has to get flung back.
    let mut shapes = vec![playfield(Material::default())];
    shapes.extend(walls(1000.0, 2000.0, 100.0, Material::default()));
    shapes.push(Shape::Circle(HitCircle {
        center: Vec2::new(500.0, 1000.0),
        radius: 40.0,
        z_low: 0.0,
        z_high: 100.0,
        material: Material {
            elasticity: 0.9,
            ..Material::default()
        },
    }));
    let mut e = Engine::new(
        shapes,
        Engine::gravity_from_slope(0.0, DEFAULT_TABLE_GRAVITY),
    );

    let mut b = Ball::new(Vec3::new(200.0, 1000.0, 50.0), R);
    b.vel = Vec3::new(300.0, 0.0, 0.0);
    e.add_ball(b);

    run(&mut e, 500);

    assert!(
        e.balls[0].vel.x < 0.0,
        "it should come back bounced; its velocity in x is {}",
        e.balls[0].vel.x
    );
    // And never have got inside the bumper.
    let d = (e.balls[0].pos.truncate() - Vec2::new(500.0, 1000.0)).length();
    assert!(d > 40.0, "it ended up inside the bumper: distance {d}");
}

#[test]
fn the_simulation_is_reproducible() {
    // It is the property that makes it possible to have regression tests over
    // trajectories. The scatter generator is our own and starts from the same
    // seed, so two identical runs give the same thing bit for bit.
    let build = || {
        let mut e = table(Material {
            elasticity: 0.8,
            friction: 0.2,
            scatter: 0.5,
            elasticity_falloff: 0.0,
        });
        let mut b = Ball::new(Vec3::new(500.0, 1000.0, 50.0), R);
        b.vel = Vec3::new(400.0, 300.0, 0.0);
        e.add_ball(b);
        e
    };

    let (mut a, mut b) = (build(), build());
    run(&mut a, 5000);
    run(&mut b, 5000);

    assert_eq!(
        a.balls[0].pos, b.balls[0].pos,
        "two identical runs have to match"
    );
    assert_eq!(a.balls[0].vel, b.balls[0].vel);
}

#[test]
fn a_captured_ball_stays_where_it_is() {
    let mut e = table(Material::default());
    let mut b = Ball::new(Vec3::new(500.0, 1000.0, 200.0), R);
    b.locked = true;
    e.add_ball(b);
    let before = e.balls[0].pos;

    run(&mut e, 2000);

    assert_eq!(e.balls[0].pos, before, "a captured ball does not move");
}

#[test]
fn several_balls_are_simulated_at_once() {
    let mut e = table(Material {
        elasticity: 0.5,
        friction: 0.2,
        ..Material::default()
    });
    for i in 0..8 {
        let x = 200.0 + i as f32 * 80.0;
        e.add_ball(Ball::new(Vec3::new(x, 500.0, 300.0), R));
    }

    run(&mut e, 3000);

    for (i, b) in e.balls.iter().enumerate() {
        assert!(b.pos.z > 0.0, "ball {i} went through the playfield");
        assert!(b.pos.z < 400.0, "ball {i} flew off: z={}", b.pos.z);
    }
}

#[test]
fn the_loop_does_not_hang_with_simultaneous_hits() {
    // A ball wedged right into a corner generates several hits at the same
    // instant. Without the counter that forces an advance, the loop would sit
    // there resolving zero-time events forever.
    let mut e = table(Material {
        elasticity: 0.0,
        friction: 0.5,
        ..Material::default()
    });
    e.add_ball(Ball::new(Vec3::new(R + 0.001, R + 0.001, R), R));

    // If this finishes, the cutoff works.
    run(&mut e, 2000);

    let p = e.balls[0].pos;
    assert!(p.x > 0.0 && p.y > 0.0 && p.z > 0.0, "it ended at {p:?}");
}

#[test]
fn a_ball_with_elasticity_one_keeps_the_bounce_height() {
    // With no friction and a perfect bounce, the ball has to come back roughly
    // to the height it left from. It is a proof that the loop is neither
    // losing nor inventing energy.
    let shapes = vec![playfield(Material {
        elasticity: 1.0,
        elasticity_falloff: 0.0,
        friction: 0.0,
        scatter: 0.0,
    })];
    // A horizontal table so the slope does not get involved.
    let mut e = Engine::new(shapes, Vec3::new(0.0, 0.0, -DEFAULT_TABLE_GRAVITY));
    let start_height = 300.0;
    e.add_ball(Ball::new(Vec3::new(0.0, 0.0, start_height), R));

    // Track the maximum height it reaches after bouncing.
    let mut max_after_bounce: f32 = 0.0;
    let mut bounced = false;
    for _ in 0..20_000 {
        e.step();
        let b = &e.balls[0];
        if !bounced && b.vel.z > 0.0 {
            bounced = true;
        }
        if bounced {
            max_after_bounce = max_after_bounce.max(b.pos.z);
        }
    }

    assert!(bounced, "the ball never bounced");
    assert!(
        max_after_bounce > start_height * 0.8,
        "it lost too much energy: it went up to {max_after_bounce} out of {start_height}"
    );
    assert!(
        max_after_bounce < start_height * 1.2,
        "it gained energy out of nowhere: it went up to {max_after_bounce} out of {start_height}"
    );
}

#[test]
fn a_ball_that_is_going_nowhere_loses_its_spin() {
    // `PhysicsEngine.cpp:690-730`, the block the original guards with
    // `C_BALL_SPIN_HACK`. Its own comment on the damping factor says what it is
    // for: *do not kill spin completely, otherwise stuck balls will happen
    // during regular gameplay*. A ball wedged against something can hold itself
    // there on angular momentum alone, and this is what shakes it loose.
    //
    // The test is a ratio of spin to travel, so the ball has to be spinning
    // hard and barely moving — which is exactly the state being chased.
    let mut engine = Engine::new(
        vec![Shape::Plane(HitPlane {
            normal: Vec3::Z,
            d: 0.0,
            material: Material {
                elasticity: 0.25,
                elasticity_falloff: 0.0,
                friction: 0.075,
                scatter: 0.0,
            },
        })],
        Vec3::new(0.0, 0.0, -DEFAULT_TABLE_GRAVITY),
    );
    let mut b = Ball::new(Vec3::new(0.0, 0.0, R), R);
    // Spinning about the playfield's plane, which is what the ratio measures,
    // and hardly going anywhere.
    b.angular_momentum = Vec3::new(400.0, 400.0, 0.0);
    engine.add_ball(b);

    let before = engine.balls[0].angular_momentum.length();
    // Past the hundred milliseconds of history the test needs.
    for _ in 0..300 {
        engine.step();
    }
    let after = engine.balls[0].angular_momentum.length();

    // Friction alone takes about a tenth of it off over this many steps, so
    // the threshold has to be well past that to mean anything: measured, the
    // damping leaves 0.04% of the spin and friction alone leaves 90%.
    //
    // The clamp at 0.23 in the original is per application, not cumulative —
    // over three hundred milliseconds of resting contact it compounds, and a
    // ball that has been sitting still that long really has stopped spinning.
    assert!(
        after < before * 0.5,
        "the spin has to come down, and not just by what friction takes:          {before:.1} -> {after:.1}"
    );
}
