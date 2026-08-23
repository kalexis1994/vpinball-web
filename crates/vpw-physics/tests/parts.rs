//! Tests for bumpers, gates, spinners and kickers.
//!
//! What sets these pieces apart from a wall is that they **react**: they kick
//! the ball, they open, they spin or they capture it. That is what gets
//! verified.

use vpw_math::{Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::collision::{CollisionEvent, Material};
use vpw_physics::constants::DEFAULT_BALL_SIZE;
use vpw_physics::parts::{Bumper, Gate, Kicker, PivotAxis, Spinner};
use vpw_physics::shapes::HitCircle;

const R: f32 = DEFAULT_BALL_SIZE;

fn circle(radius: f32, elasticity: f32) -> HitCircle {
    HitCircle {
        center: Vec2::ZERO,
        radius,
        z_low: 0.0,
        z_high: 60.0,
        material: Material {
            elasticity,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        },
    }
}

fn incoming_ball(vx: f32) -> Ball {
    let mut b = Ball::new(Vec3::new(70.0, 0.0, 25.0), R);
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

// ---------------------------------------------------------------- bumper ---

#[test]
fn a_bumper_kicks_the_ball() {
    let mut bp = Bumper::new(circle(45.0, 0.5), 20.0, 1.0);
    let mut b = incoming_ball(-30.0);
    let before = b.vel.length();

    let fired = bp.collide(&mut b, &contact(), 0.0);

    assert!(fired, "at that velocity it should have fired");
    assert!(
        b.vel.length() > before,
        "it has to come out faster: {before:.1} -> {:.1}",
        b.vel.length()
    );
    assert!(b.vel.x > 0.0, "and the other way");
    assert!(bp.hit, "and report that it fired, for the animation");
}

#[test]
fn a_graze_does_not_fire_the_bumper() {
    // The threshold keeps the table from firing on its own with slow balls.
    let mut bp = Bumper::new(circle(45.0, 0.5), 20.0, 10.0);
    let mut b = incoming_ball(-2.0);
    let before = b.vel.length();

    let fired = bp.collide(&mut b, &contact(), 0.0);

    assert!(!fired);
    assert!(b.vel.length() < before, "it only bounces");
    assert!(!bp.hit);
}

#[test]
fn the_threshold_is_measured_against_the_incoming_velocity() {
    // The order matters: if it were measured after the bounce, the threshold
    // would be comparing against a velocity that has already changed sign and
    // magnitude.
    let mut strong = Bumper::new(circle(45.0, 0.9), 20.0, 25.0);
    let mut b = incoming_ball(-30.0);
    assert!(
        strong.collide(&mut b, &contact(), 0.0),
        "it came in at 30 with a threshold of 25: it has to fire"
    );

    let mut tight = Bumper::new(circle(45.0, 0.9), 20.0, 35.0);
    let mut b2 = incoming_ball(-30.0);
    assert!(
        !tight.collide(&mut b2, &contact(), 0.0),
        "it came in at 30 with a threshold of 35: it must not fire"
    );
}

#[test]
fn a_disabled_bumper_does_not_stop_the_ball() {
    let bp = Bumper::new(circle(45.0, 0.5), 20.0, 1.0);
    let mut disabled = bp.clone();
    disabled.enabled = false;
    let b = incoming_ball(-30.0);

    assert!(bp.hit_test(&b, 1.0).is_some(), "enabled it does hit");
    assert!(disabled.hit_test(&b, 1.0).is_none(), "disabled it does not");
}

// ------------------------------------------------------------------ gate ---

fn pivot_axis() -> PivotAxis {
    PivotAxis {
        center: Vec2::ZERO,
        length: 50.0,
        rotation_deg: 0.0,
        height: 40.0,
        z_low: 0.0,
        angle_min: 0.0,
        angle_max: std::f32::consts::FRAC_PI_2,
        damping: 0.985,
    }
}

fn gate(two_way: bool) -> Gate {
    Gate::new(pivot_axis(), 0.25, two_way)
}

/// A ball crossing the gate's axis, coming from +y towards -y.
fn crossing_ball(vy: f32) -> Ball {
    let mut b = Ball::new(Vec3::new(0.0, -vy.signum() * 30.0, 25.0), R);
    b.vel = Vec3::new(0.0, vy, 0.0);
    b
}

#[test]
fn the_gate_detects_the_ball_on_both_faces() {
    let g = gate(false);
    let from_one_side = g.hit_test(&crossing_ball(-40.0), 1.0);
    let from_the_other = g.hit_test(&crossing_ball(40.0), 1.0);

    assert!(
        from_one_side.is_some(),
        "it has to see it coming from one side"
    );
    assert!(from_the_other.is_some(), "and from the other one too");
    assert_ne!(
        from_one_side.unwrap().hit_flag,
        from_the_other.unwrap().hit_flag,
        "and tell which of the two faces it came in through"
    );
}

#[test]
fn the_ball_goes_through_the_gate_without_being_deflected() {
    // The gate does not stop it: it is a pass-through line. What stops the
    // ball on the wrong side is a separate rigid segment the table adds.
    let mut g = gate(false);
    let b = crossing_ball(-40.0);
    let before = b.vel;

    let c = g.hit_test(&b, 1.0).expect("it should have detected it");
    g.collide(&b, &c);

    assert_eq!(b.vel, before, "the ball carries on unchanged");
}

#[test]
fn a_ball_that_does_not_reach_the_axis_does_not_touch_it() {
    let g = gate(false);
    let mut far = Ball::new(Vec3::new(0.0, -400.0, 25.0), R);
    far.vel = Vec3::new(0.0, -1.0, 0.0); // moving away

    assert!(g.hit_test(&far, 1.0).is_none());
}

#[test]
fn a_ball_flying_over_the_top_does_not_touch_the_gate() {
    // The axis is finite in z. A ball on a ramp above does not move it.
    let g = gate(false);
    let mut high = crossing_ball(-40.0);
    high.pos.z = 300.0;

    assert!(g.hit_test(&high, 1.0).is_none());
}

#[test]
fn a_ball_opens_the_gate_by_pushing_it() {
    let mut g = gate(false);
    let b = crossing_ball(-40.0);
    let c = g.hit_test(&b, 1.0).expect("it should have detected it");

    g.collide(&b, &c);

    assert!(g.angle_speed.abs() > 0.0, "it should start to open");
}

#[test]
fn the_one_way_gate_barely_gives_way_from_behind() {
    // It is what keeps the ball from coming back the way it came. The original
    // leaves it a bounce of 1/50 so the door does not look completely rigid.
    let mut from_front = gate(false);
    let mut from_behind = gate(false);
    let b = incoming_ball(-30.0);

    let mut c = contact();
    c.hit_flag = true;
    from_front.collide(&b, &c);

    c.hit_flag = false;
    from_behind.collide(&b, &c);

    assert!(
        from_behind.angle_speed.abs() < from_front.angle_speed.abs() / 10.0,
        "from behind it has to barely give way: {} vs {}",
        from_behind.angle_speed,
        from_front.angle_speed
    );
}

#[test]
fn a_two_way_gate_gives_way_to_both_sides() {
    let mut a = gate(true);
    let mut b_gate = gate(true);
    let b = incoming_ball(-30.0);

    let mut c = contact();
    c.hit_flag = false;
    a.collide(&b, &c);
    c.hit_flag = true;
    b_gate.collide(&b, &c);

    assert!(a.angle_speed.abs() > 0.1, "from one side it does");
    assert!(b_gate.angle_speed.abs() > 0.1, "from the other one too");
    assert!(
        a.angle_speed * b_gate.angle_speed < 0.0,
        "and towards opposite sides: {} and {}",
        a.angle_speed,
        b_gate.angle_speed
    );
}

#[test]
fn hitting_the_gate_low_makes_it_spin_more() {
    // The angular velocity comes from the linear one divided by the height of
    // the axis. It is what happens with a real door: more leverage down low.
    let mut tall = Gate::new(
        PivotAxis {
            height: 200.0,
            ..pivot_axis()
        },
        0.25,
        false,
    );
    let mut short = Gate::new(
        PivotAxis {
            height: 20.0,
            ..pivot_axis()
        },
        0.25,
        false,
    );

    let b = incoming_ball(-30.0);
    let mut c = contact();
    c.hit_flag = true;
    tall.collide(&b, &c);
    short.collide(&b, &c);

    assert!(
        short.angle_speed > tall.angle_speed,
        "a lower axis spins more: {} vs {}",
        short.angle_speed,
        tall.angle_speed
    );
}

#[test]
fn the_gate_closes_on_its_own() {
    // Without the gravity term the door would stay open forever. It is the
    // difference between a door and a hole.
    let mut g = gate(false);
    g.angle = 1.0;
    g.angle_speed = 0.0001;

    let initial = g.angle_speed;
    g.update_velocities();

    assert!(
        g.angle_speed < initial,
        "gravity has to be braking it: {} -> {}",
        initial,
        g.angle_speed
    );
}

#[test]
fn the_gate_returns_to_rest_and_stays_still() {
    // Without the cutoff, a slow ball leaves it animating forever.
    let mut g = gate(false);
    g.angle = 0.005;
    g.angle_speed = 0.005;

    g.update_velocities();

    assert_eq!(g.angle, g.angle_min, "it has to settle");
    assert_eq!(g.angle_speed, 0.0, "and stay still");
}

#[test]
fn the_gate_swings_open_and_then_closes_on_its_own() {
    // The full cycle of a door: the ball pushes it, the door opens, it hits
    // the stop, and gravity brings it back to rest. It is what separates a
    // door from a hole in the table.
    //
    // The original clamps with the angle from the **previous** round and only
    // then integrates, so the door spends one step outside its range before it
    // gets stopped. That is not a bug: it is the original's ordering, and the
    // margin below allows for it.
    let mut g = gate(false);
    let b = crossing_ball(-60.0);
    let c = g.hit_test(&b, 1.0).expect("it should have detected it");
    g.collide(&b, &c);
    assert!(g.angle_speed.abs() > 0.0, "the ball has to open it");

    let mut max_opening: f32 = 0.0;
    for _ in 0..5_000 {
        g.update_velocities();
        g.update_displacements(0.1);
        max_opening = max_opening.max(g.angle.abs());

        let margin = g.angle_speed.abs() * 0.1;
        assert!(
            g.angle.abs() <= g.angle_max + margin + 1e-3,
            "it escaped from its range: {} (max {}, margin {margin})",
            g.angle,
            g.angle_max
        );
    }

    assert!(
        max_opening > 0.1,
        "it should really have opened, it got to {max_opening}"
    );
    assert_eq!(g.angle, g.angle_min, "and end up closed");
    assert_eq!(g.angle_speed, 0.0, "and still");
}

// --------------------------------------------------------------- spinner ---

fn spinner(limited: bool) -> Spinner {
    Spinner::new(
        PivotAxis {
            center: Vec2::ZERO,
            length: 80.0,
            rotation_deg: 0.0,
            height: 40.0,
            z_low: 0.0,
            angle_min: 0.0,
            angle_max: if limited { 1.0 } else { 0.0 },
            damping: 0.9879,
        },
        0.3,
    )
}

#[test]
fn a_ball_makes_the_spinner_spin() {
    let mut s = spinner(false);
    let b = crossing_ball(-40.0);
    let c = s.hit_test(&b, 1.0).expect("it should have detected it");

    let spun = s.collide(&b, &c);

    assert!(spun, "from the front it has to spin");
    assert!(s.angle_speed.abs() > 0.0);
}

#[test]
fn the_spinner_does_not_spin_if_it_is_hit_from_behind() {
    let mut s = spinner(false);
    let mut b = incoming_ball(0.0);
    b.vel = Vec3::new(-40.0, 0.0, 0.0); // against the normal

    assert!(!s.collide(&b, &contact()), "from behind it does not count");
    assert_eq!(s.angle_speed, 0.0);
}

#[test]
fn the_spinner_marks_the_face_the_other_way_round_from_the_gate() {
    // It is not a cosmetic detail: the sign of the spin comes from it.
    // Changing it makes the spinner turn the opposite way from the one the
    // ball went through.
    let g = gate(false);
    let s = spinner(false);
    let b = crossing_ball(-40.0);

    let cg = g.hit_test(&b, 1.0).expect("gate");
    let cs = s.hit_test(&b, 1.0).expect("spinner");

    assert_ne!(cg.hit_flag, cs.hit_flag);
}

#[test]
fn the_free_spinner_goes_all_the_way_around() {
    let mut s = spinner(false);
    s.angle_speed = 5.0;
    for _ in 0..2000 {
        s.update_displacements(0.1);
        assert!(
            (0.0..=std::f32::consts::TAU + 1e-3).contains(&s.angle),
            "the angle has to wrap: {}",
            s.angle
        );
    }
}

#[test]
fn the_limited_spinner_bounces_at_its_limits() {
    let mut s = spinner(true);
    s.angle_speed = 20.0;
    for _ in 0..500 {
        s.update_displacements(0.1);
        assert!(
            s.angle >= s.angle_min - 1e-3 && s.angle <= s.angle_max + 1e-3,
            "it went past the stops: {}",
            s.angle
        );
    }
}

#[test]
fn the_spinner_ends_up_hanging_downwards() {
    // Without the gravity term it would stop in any position, and a vane left
    // stopped horizontally looks wrong and on top of that blocks the way.
    let mut s = spinner(false);
    s.angle = 2.0;
    s.angle_speed = 3.0;
    for _ in 0..20_000 {
        s.update_velocities();
        s.update_displacements(0.1);
    }

    let vertical = s.angle.sin().abs();
    assert!(
        vertical < 0.1,
        "it should end up nearly vertical, it ended at {} rad (sin={})",
        s.angle,
        vertical
    );
    assert!(
        s.angle_speed.abs() < 0.1,
        "and nearly still: {}",
        s.angle_speed
    );
}

// ---------------------------------------------------------------- kicker ---

#[test]
fn a_kicker_captures_a_ball_that_falls_inside() {
    let mut k = Kicker::new(circle(30.0, 0.3), 0.7, false);
    let mut b = Ball::new(Vec3::new(5.0, 5.0, 10.0), R); // nicely sunk in
    b.vel = Vec3::new(3.0, 0.0, -2.0);

    let grabbed = k.try_capture(&mut b, 0);

    assert!(grabbed, "it should capture it");
    assert!(b.locked, "and the ball comes to a stop");
    assert_eq!(b.vel, Vec3::ZERO);
    assert_eq!(k.captured, Some(0));
    // Centered in the hole.
    assert!((b.pos.x - k.circle.center.x).abs() < 1e-4);
}

#[test]
fn a_ball_that_passes_over_the_top_is_not_captured() {
    // It is what keeps every ball that grazes a kicker from being captured: if
    // it comes in high, it runs straight past over the edge.
    let mut k = Kicker::new(circle(30.0, 0.3), 0.7, false);
    let mut b = Ball::new(Vec3::new(0.0, 0.0, 200.0), R);
    b.vel = Vec3::new(50.0, 0.0, 0.0);

    assert!(!k.try_capture(&mut b, 0), "it passes over the top");
    assert!(!b.locked);
}

#[test]
fn a_kicker_with_a_ball_inside_does_not_capture_another() {
    let mut k = Kicker::new(circle(30.0, 0.3), 0.7, false);
    let mut a = Ball::new(Vec3::new(0.0, 0.0, 10.0), R);
    let mut b = Ball::new(Vec3::new(0.0, 0.0, 10.0), R);

    assert!(k.try_capture(&mut a, 0));
    assert!(!k.try_capture(&mut b, 1), "it already has one inside");
    assert!(!b.locked);
    // And it stops detecting hits while it is busy.
    assert!(k.hit_test(&b, 1.0).is_none());
}

#[test]
fn releasing_puts_the_ball_back_in_play() {
    let mut k = Kicker::new(circle(30.0, 0.3), 0.7, false);
    let mut b = Ball::new(Vec3::new(0.0, 0.0, 10.0), R);
    k.try_capture(&mut b, 0);

    k.release(&mut b, Vec3::new(0.0, -100.0, 0.0));

    assert!(!b.locked, "it moves again");
    assert!(b.vel.y < 0.0, "with the velocity it was given");
    assert_eq!(k.captured, None, "and the kicker is free again");
}
