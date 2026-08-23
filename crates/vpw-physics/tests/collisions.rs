//! Tests for the collision response.
//!
//! Every case is checked against known mechanics —conservation, signs,
//! limits— and not against "whatever came out the first time". Where the
//! original makes a decision that does not follow from the physics, the test
//! cites the line.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::collision::{CollisionEvent, Material, elasticity_with_falloff};
use vpw_physics::constants::DEFAULT_BALL_SIZE;

/// A horizontal wall: the ball falls onto it, the normal points up.
fn floor() -> Vec3 {
    Vec3::Z
}

fn contact(distance: f32) -> CollisionEvent {
    CollisionEvent {
        hit_distance: distance,
        hit_normal: floor(),
        ..CollisionEvent::default()
    }
}

/// With no friction and no scatter, to isolate the bounce.
fn smooth(elasticity: f32) -> Material {
    Material {
        elasticity,
        elasticity_falloff: 0.0,
        friction: 0.0,
        scatter: 0.0,
    }
}

#[test]
fn a_perfectly_elastic_bounce_keeps_the_velocity() {
    let mut b = Ball::new(Vec3::new(0.0, 0.0, 100.0), DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -10.0);

    b.collide_3d_wall(floor(), &smooth(1.0), &contact(0.0), 0.0);

    assert!(
        (b.vel.z - 10.0).abs() < 1e-4,
        "with elasticity 1 it has to come out at the same velocity; got {}",
        b.vel.z
    );
}

#[test]
fn a_bounce_with_no_elasticity_leaves_the_ball_resting() {
    let mut b = Ball::new(Vec3::new(0.0, 0.0, 100.0), DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -10.0);

    b.collide_3d_wall(floor(), &smooth(0.0), &contact(0.0), 0.0);

    assert!(
        b.vel.z.abs() < 1e-4,
        "with no elasticity it does not bounce; got {}",
        b.vel.z
    );
}

#[test]
fn intermediate_elasticity_gives_back_the_right_fraction() {
    for e in [0.25f32, 0.5, 0.75] {
        let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
        b.vel = Vec3::new(0.0, 0.0, -10.0);
        b.collide_3d_wall(floor(), &smooth(e), &contact(0.0), 0.0);
        assert!(
            (b.vel.z - 10.0 * e).abs() < 1e-3,
            "with elasticity {e} it should have come out at {}, got {}",
            10.0 * e,
            b.vel.z
        );
    }
}

#[test]
fn the_bounce_does_not_touch_the_tangential_velocity() {
    // The impulse goes along the normal, so whatever runs parallel to the wall
    // stays the same. With no friction, of course.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(7.0, 3.0, -10.0);

    b.collide_3d_wall(floor(), &smooth(0.5), &contact(0.0), 0.0);

    assert!(
        (b.vel.x - 7.0).abs() < 1e-4,
        "the x component is not touched"
    );
    assert!((b.vel.y - 3.0).abs() < 1e-4, "nor the y one");
}

#[test]
fn a_ball_moving_away_does_not_bounce_twice() {
    // It is the guard that keeps the ball from getting stuck to the wall
    // bouncing off it on successive steps.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, 5.0); // already going up
    let before = b.vel;

    b.collide_3d_wall(floor(), &smooth(1.0), &contact(0.0), 0.0);

    assert_eq!(b.vel, before, "a ball moving away is not touched");
}

#[test]
fn an_embedded_ball_gets_a_shove() {
    // If it ended up inside the wall, the original gives it a kick to get it
    // out instead of leaving it in there (`hitball.cpp:51-52`).
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::ZERO; // still but inside
    let sunk = contact(-1.0);

    b.collide_3d_wall(floor(), &smooth(0.0), &sunk, 0.0);

    assert!(b.vel.z > 0.0, "it has to come out shoved; got {}", b.vel.z);
    assert!(
        b.pos.z > 0.0,
        "and on top of that it gets pulled out of the wall"
    );
}

#[test]
fn the_shove_out_has_a_cap() {
    // When crossing a ramp the measured distance jumps around; without a cap
    // the ball would make a visible hop (`hitball.cpp:62-63`, C_DISP_LIMIT).
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -1.0);

    b.collide_3d_wall(floor(), &smooth(0.0), &contact(-1000.0), 0.0);

    assert!(
        b.pos.z <= vpw_physics::constants::C_DISP_LIMIT + 1e-3,
        "the shove cannot go past the limit; got {}",
        b.pos.z
    );
}

#[test]
fn friction_sets_the_ball_spinning() {
    // A ball that arrives sliding leaves spinning: it is what makes it roll
    // instead of skidding all the way across the table.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(20.0, 0.0, -10.0);
    let rough = Material {
        elasticity: 0.5,
        friction: 0.5,
        ..Material::default()
    };

    assert_eq!(b.angular_momentum, Vec3::ZERO);
    b.collide_3d_wall(floor(), &rough, &contact(0.0), 0.0);

    assert!(
        b.angular_momentum.length() > 0.0,
        "friction has to generate spin"
    );
    // Moving in +x over a floor, the ball rolls about +y.
    assert!(
        b.angular_momentum.y > 0.0,
        "the spin has to go in the rolling direction; got {:?}",
        b.angular_momentum
    );
    assert!(b.vel.x < 20.0, "and brake it a little");
}

#[test]
fn a_ball_already_rolling_gets_no_more_friction() {
    // Rolling without slipping: the velocity of the contact point is zero, so
    // friction has nothing to act against.
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    let v = 20.0;
    b.vel = Vec3::new(v, 0.0, -10.0);
    // Rolling: w = v / r, about +y.
    b.angular_momentum = Vec3::new(0.0, v / b.radius, 0.0) * b.inertia();

    let vx_before = b.vel.x;
    let rough = Material {
        elasticity: 0.5,
        friction: 0.5,
        ..Material::default()
    };
    b.collide_3d_wall(floor(), &rough, &contact(0.0), 0.0);

    assert!(
        (b.vel.x - vx_before).abs() < 0.5,
        "rolling it should barely brake at all; it went from {vx_before} to {}",
        b.vel.x
    );
}

#[test]
fn elasticity_drops_with_the_impact_velocity() {
    // `collide.h:56`. A harder hit bounces less.
    let soft = elasticity_with_falloff(0.8, 0.5, 1.0);
    let hard = elasticity_with_falloff(0.8, 0.5, 100.0);
    assert!(
        hard < soft,
        "the faster it is, the less it bounces: {soft} vs {hard}"
    );

    // With no falloff, the elasticity is the same at any velocity.
    assert_eq!(elasticity_with_falloff(0.8, 0.0, 1.0), 0.8);
    assert_eq!(elasticity_with_falloff(0.8, 0.0, 100.0), 0.8);
}

#[test]
fn scatter_does_not_act_at_low_velocity() {
    // At low velocity it would mess up the ball coming to rest, so the
    // original turns it off when the change in velocity is small
    // (`hitball.cpp:105`).
    //
    // The 0.5 below is not just any number: the distribution is `s·(1−s²)`,
    // which **is zero at s = ±1**. Passing 1.0 never scatters, by design.
    let with_scatter = Material {
        elasticity: 0.5,
        scatter: 0.5,
        ..Material::default()
    };

    // The scatter **rotates the horizontal velocity**, so a ball falling
    // perfectly straight down must not turn: there is no direction to deflect.
    // That is why both balls carry a component in `y`.
    let mut slow = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    slow.vel = Vec3::new(0.0, 5.0, -0.5);
    slow.collide_3d_wall(floor(), &with_scatter, &contact(0.0), 0.5);
    assert!(
        slow.vel.x.abs() < 1e-6,
        "at low velocity it does not scatter"
    );

    let mut fast = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    fast.vel = Vec3::new(0.0, 5.0, -50.0);
    fast.collide_3d_wall(floor(), &with_scatter, &contact(0.0), 0.5);
    assert!(fast.vel.x.abs() > 1e-6, "at high velocity it does");
}

#[test]
fn a_ball_falling_straight_down_is_never_deflected() {
    // A corollary of the previous one, and worth pinning down: the scatter
    // rotates the horizontal vector, so with no horizontal component it does
    // nothing no matter how much scatter the material has.
    let lots = Material {
        elasticity: 0.5,
        scatter: 2.0,
        ..Material::default()
    };
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(0.0, 0.0, -80.0);
    b.collide_3d_wall(floor(), &lots, &contact(0.0), 0.5);
    assert!(
        b.vel.x.abs() < 1e-6 && b.vel.y.abs() < 1e-6,
        "got {:?}",
        b.vel
    );
}

#[test]
fn scatter_vanishes_at_the_ends_of_the_range() {
    // The shape `s·(1−s²)` is chosen so that most of the draws land near zero:
    // the ends of the range deflect nothing. Without this, a table with high
    // scatter would send balls off at absurd angles.
    let with_scatter = Material {
        elasticity: 0.5,
        scatter: 0.5,
        ..Material::default()
    };
    for extreme in [-1.0f32, 0.0, 1.0] {
        let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
        b.vel = Vec3::new(0.0, 5.0, -50.0);
        b.collide_3d_wall(floor(), &with_scatter, &contact(0.0), extreme);
        assert!(
            b.vel.x.abs() < 1e-6,
            "with s={extreme} it must not deflect; got {}",
            b.vel.x
        );
    }
}

#[test]
fn the_contact_holds_the_ball_up_against_gravity() {
    // A ball resting on the playfield does not sink: the surface gives back
    // exactly what gravity puts into it on this step.
    let g = Vec3::new(0.0, 0.0, -1.0);
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    let dtime = 0.1;

    // Simulates what the loop does: gravity first, then the contact.
    b.vel += g * dtime;
    let before = b.vel.z;
    assert!(before < 0.0, "gravity pushes it downwards");

    b.handle_static_contact(&contact(0.0), 0.0, dtime, g);

    assert!(
        b.vel.z.abs() < 1e-5,
        "the contact has to cancel the fall; it ended at {}",
        b.vel.z
    );
}

#[test]
fn the_contact_does_not_pull_the_ball_downwards() {
    // The normal force is never negative: a surface pushes, it does not pull
    // (`hitball.cpp:296`, the max(0, ...)).
    let g = Vec3::new(0.0, 0.0, -1.0);
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    // A ball that is already coming up slowly, resting.
    b.vel = Vec3::new(0.0, 0.0, 0.05);
    let before = b.vel.z;

    b.handle_static_contact(&contact(0.0), 0.0, 0.1, g);

    assert!(
        b.vel.z >= before - 1e-6,
        "the contact cannot brake it downwards: {before} -> {}",
        b.vel.z
    );
}

#[test]
fn a_ball_resting_on_a_slope_does_not_slide_on_its_own() {
    // This is the static friction case: the ball is still on the tilted
    // playfield and friction opposes the **acceleration**, not the velocity.
    // Without that case, every still ball would slide.
    let slope = 6.0f32.to_radians();
    let g = Vec3::new(0.0, slope.sin(), -slope.cos()) * 0.97;

    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    let normal = Vec3::Z;
    let dtime = 0.1;

    // With a lot of friction, the impulse has to oppose the fall.
    b.apply_friction(normal, dtime, 5.0, g);

    assert!(
        b.vel.y <= 1e-6,
        "static friction cannot push it down the slope; got {}",
        b.vel.y
    );
    assert!(
        b.angular_momentum.length() > 0.0 || b.vel.length() > 0.0,
        "something has to have acted"
    );
}

#[test]
fn friction_has_a_cap() {
    // It cannot go past the Coulomb cone: coefficient times normal force.
    //
    // The ball has to be leaving the surface as well as sliding along it, or
    // this never reaches the branch being tested: a ball whose normal velocity
    // is nothing goes to the static one instead (`hitball.cpp:335`). Leaving
    // while sliding is not a contrived case — it is the one the original names,
    // a ball a rubber has just pushed upwards.
    let g = Vec3::new(0.0, 0.0, -1.0);
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(1000.0, 0.0, 1.0);

    let before = b.vel.x;
    b.apply_friction(Vec3::Z, 0.1, 0.3, g);
    let braking = before - b.vel.x;

    // The maximum impulse in one step is coef . m . g . dt.
    let cap = 0.3 * b.mass * 1.0 * 0.1;
    assert!(
        braking <= cap * 1.5,
        "it braked {braking}, which goes past the cap of {cap}"
    );
    assert!(braking > 0.0, "but something has to brake");
}

#[test]
fn a_ball_skidding_along_what_it_rests_on_is_not_braked_like_a_slider() {
    // The half of the condition at `hitball.cpp:336` that is easy to lose:
    // `normVel <= 0.025` sends a ball that is *resting on* a surface to the
    // static branch even when it is sliding fast along it.
    //
    // Take that away and the ball goes to the dynamic branch, where friction
    // opposes the slip velocity up to the full Coulomb cap and the skid is gone
    // within a few milliseconds. Every shot off a flipper starts as a skid, so
    // which branch it takes is where the ball ends up.
    let g = Vec3::new(0.0, 0.0, -1.0);
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);
    b.vel = Vec3::new(1000.0, 0.0, 0.0); // resting on it, and sliding hard

    let before = b.vel.x;
    b.apply_friction(Vec3::Z, 0.1, 0.3, g);

    assert_eq!(
        b.vel.x, before,
        "on a flat surface there is nothing pulling the ball along it, so the          static branch has nothing to oppose and the skid survives"
    );
}

#[test]
fn a_ball_resting_on_a_slope_is_held_by_friction_against_it() {
    // The other side of the same branch: what the static case is *for*. On a
    // slope the tangential part of gravity is what the ball would slide down,
    // and friction opposes that acceleration rather than any velocity. It is
    // the rolling constraint, and it is why a ball rolls down a table instead
    // of skating down it.
    let slope = 6.0f32.to_radians();
    let g = Vec3::new(0.0, slope.sin(), -slope.cos());
    let mut b = Ball::new(Vec3::ZERO, DEFAULT_BALL_SIZE);

    b.apply_friction(Vec3::Z, 0.1, 0.3, g);

    assert!(
        b.angular_momentum.length() > 0.0,
        "friction against the slope has to start the ball turning"
    );
    assert!(
        b.vel.y < 0.0,
        "and push back against the way gravity is pulling it, got {}",
        b.vel.y
    );
}
