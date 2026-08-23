//! Tests for the plunger.
//!
//! It is the only piece the player **charges up**: you pull, it builds up, you
//! let go. What has to be verified is that the force comes from how far it was
//! pulled and not from something else, because that is what makes it feel like
//! a plunger.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::collision::Material;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, PHYS_FACTOR};
use vpw_physics::engine::{Engine, Shape};
use vpw_physics::plunger::{Plunger, PlungerParams};

const R: f32 = DEFAULT_BALL_SIZE;

/// Where the rod sits with nobody touching it: `frame_end + rest_pos * frame_len`.
const REST: f32 = 1910.0 + 0.17 * 180.0;

/// A plunger with measurements close to those of a real table: the lane at
/// x 880..920, with 180 units of travel ending at y = 2090.
fn params() -> PlungerParams {
    PlungerParams {
        x: 885.0,
        x2: 925.0,
        y: 2090.0,
        z_low: 0.0,
        frame_end: 1910.0,
        frame_start: 2090.0,
        rest_pos: 0.17,
        speed_pull: 10.0,
        speed_fire: 110.0,
        // The original's default.
        mech_strength: 85.0,
        scatter_velocity: 0.0,
        momentum_xfer: 1.0,
        auto_plunger: false,
        material: Material::default(),
    }
}

fn plunger() -> Plunger {
    Plunger::new(params())
}

/// One complete step, the way the engine does it.
fn step(p: &mut Plunger) {
    p.update_velocities();
    p.update_displacements(PHYS_FACTOR);
}

/// The ball in the lane, resting against the tip.
fn ball_in_the_lane(p: &Plunger) -> Ball {
    Ball::new(Vec3::new(905.0, p.pos - R - 1.0, R), R)
}

// ------------------------------------------------------------- the pull ---

#[test]
fn it_starts_at_rest() {
    let p = plunger();
    assert!(
        (p.relative_position() - 0.17).abs() < 1e-4,
        "it should start at the parked position: {}",
        p.relative_position()
    );
    assert_eq!(p.speed, 0.0);
}

#[test]
fn pulling_takes_it_backwards() {
    let mut p = plunger();
    let start = p.pos;
    p.pull();
    for _ in 0..100 {
        step(&mut p);
    }

    assert!(
        p.pos > start,
        "in VP y grows towards the player, so pulling means increasing y: {} -> {}",
        start,
        p.pos
    );
}

#[test]
fn the_pull_stops_at_the_back() {
    let mut p = plunger();
    p.pull();
    for _ in 0..5000 {
        step(&mut p);
        assert!(
            p.pos <= p.frame_start + 1e-3,
            "it went past the back: {} against {}",
            p.pos,
            p.frame_start
        );
    }
    assert!(
        (p.relative_position() - 1.0).abs() < 1e-3,
        "it should end up all the way back: {}",
        p.relative_position()
    );
}

#[test]
fn pulling_twice_does_not_send_it_further_back() {
    // The browser repeats the `keydown` while the key is held down.
    let mut p = plunger();
    p.pull();
    for _ in 0..5000 {
        step(&mut p);
    }
    let limit = p.pos;

    p.pull();
    for _ in 0..100 {
        step(&mut p);
    }
    assert!((p.pos - limit).abs() < 1e-3, "{} against {}", p.pos, limit);
}

// ------------------------------------------------------------- the shot ---

/// Pulls for `steps` and releases; returns the velocity right afterwards.
fn pull_and_release(steps: usize) -> (Plunger, f32) {
    let mut p = plunger();
    p.pull();
    for _ in 0..steps {
        step(&mut p);
    }
    p.release();
    p.update_velocities();
    let v = p.speed;
    (p, v)
}

#[test]
fn releasing_fires_it_forwards() {
    let (_, v) = pull_and_release(1000);
    assert!(v < 0.0, "forwards is decreasing y: {v}");
}

#[test]
fn the_further_it_is_pulled_the_harder_it_comes_out() {
    // What makes a plunger feel like a plunger.
    //
    // The three pulls are short on purpose: with the pull force of a real
    // table the plunger reaches the back in about ninety milliseconds, so any
    // larger value always gives the maximum and the test would measure
    // nothing.
    let (_, little) = pull_and_release(30);
    let (_, medium) = pull_and_release(60);
    let (_, lots) = pull_and_release(200);

    assert!(
        lots.abs() > medium.abs() && medium.abs() > little.abs(),
        "it should grow: {little} -> {medium} -> {lots}"
    );
}

#[test]
fn releasing_from_rest_does_almost_nothing() {
    let mut p = plunger();
    p.release();
    p.update_velocities();
    assert!(
        p.speed.abs() < 1e-3,
        "with no pull there is no energy to let go of: {}",
        p.speed
    );
}

#[test]
fn the_shot_ends_on_its_own() {
    // Fire mode has a safety cutoff: without it, the plunger would keep
    // travelling at a fixed velocity forever.
    let (mut p, _) = pull_and_release(5000);
    for _ in 0..1000 {
        step(&mut p);
    }
    assert!(p.speed.abs() < 1.0, "it should have settled: {}", p.speed);
    assert!(
        p.relative_position() >= -1e-3 && p.relative_position() <= 1.0 + 1e-3,
        "and end up inside its travel: {}",
        p.relative_position()
    );
}

#[test]
fn the_plunger_bounces_off_the_spring() {
    // When fired into thin air, a real plunger bounces. It shows up as the
    // velocity changing direction before settling.
    let (mut p, _) = pull_and_release(5000);
    let mut changes = 0;
    let mut sign = p.speed.signum();
    for _ in 0..600 {
        step(&mut p);
        if p.speed != 0.0 && p.speed.signum() != sign {
            changes += 1;
            sign = p.speed.signum();
        }
    }
    assert!(changes >= 1, "it should have bounced at least once");
}

#[test]
fn a_button_plunger_always_fires_at_full_power() {
    // Tables that are launched with a button have no spring: the force is
    // constant, like a solenoid.
    let mut auto = Plunger::new(PlungerParams {
        auto_plunger: true,
        ..params()
    });
    auto.release();
    auto.update_velocities();
    let without_pulling = auto.speed;

    let mut auto2 = Plunger::new(PlungerParams {
        auto_plunger: true,
        ..params()
    });
    auto2.pull();
    for _ in 0..5000 {
        step(&mut auto2);
    }
    auto2.release();
    auto2.update_velocities();

    assert!(
        (without_pulling - auto2.speed).abs() < 1e-3,
        "always equally hard"
    );
    assert!(without_pulling < 0.0, "and forwards");
}

// ------------------------------------------------------------- the ball ---

fn engine_with_plunger(p: Plunger) -> Engine {
    Engine::new(vec![Shape::Plunger(p)], Vec3::ZERO)
}

#[test]
fn the_plunger_launches_the_ball() {
    let mut p = plunger();
    p.pull();
    for _ in 0..5000 {
        step(&mut p);
    }
    let ball = ball_in_the_lane(&p);
    p.release();

    let mut e = engine_with_plunger(p);
    e.add_ball(ball);
    let start_y = e.balls[0].pos.y;

    let i = e.plunger_indices().next().unwrap();
    e.release_plunger(i);
    for _ in 0..500 {
        e.step();
    }

    let travelled = start_y - e.balls[0].pos.y;
    eprintln!("the ball went up {travelled:.0} units");
    assert!(
        e.balls[0].vel.y < -1.0,
        "the ball should come out upwards: {:?}",
        e.balls[0].vel
    );
    assert!(travelled > 100.0, "and really have moved: {travelled:.0}");
}

#[test]
fn a_short_pull_sends_the_ball_out_slower() {
    // The proof that the force gets all the way to the ball and does not get
    // lost along the way.
    let speed_for = |steps: usize| {
        let mut p = plunger();
        p.pull();
        for _ in 0..steps {
            step(&mut p);
        }
        let ball = ball_in_the_lane(&p);
        let mut e = engine_with_plunger(p);
        e.add_ball(ball);
        let i = e.plunger_indices().next().unwrap();
        e.release_plunger(i);

        let mut fastest: f32 = 0.0;
        for _ in 0..500 {
            e.step();
            fastest = fastest.max(-e.balls[0].vel.y);
        }
        fastest
    };

    let soft = speed_for(35);
    let hard = speed_for(200);
    eprintln!("short pull: {soft:.1}, long pull: {hard:.1}");
    assert!(
        hard > soft * 1.5,
        "a long pull has to hit quite a bit harder: {soft} against {hard}"
    );
}

#[test]
fn the_ball_does_not_go_through_the_plunger() {
    // The walls of the lane and the bottom have to stop it.
    let mut e = engine_with_plunger(plunger());
    let mut b = Ball::new(Vec3::new(905.0, 1950.0, R), R);
    b.vel = Vec3::new(0.0, 40.0, 0.0); // towards the back, hard
    e.add_ball(b);

    for _ in 0..2000 {
        e.step();
        assert!(
            e.balls[0].pos.y < 2200.0,
            "it went out the back of the plunger: {:?}",
            e.balls[0].pos
        );
    }
}

#[test]
fn a_ball_resting_against_it_does_not_send_the_rod_away() {
    // A ball pressing on the tip gives the plunger a shove back
    // (`hitplunger.cpp:747`), and on a table with a mechanical plunger the rod
    // feels it. On a keyboard one it does not: the line that applies it is the
    // last one inside the `else if (isMech)` branch (`hitplunger.cpp:470`).
    //
    // Applying it anyway is not a small error, because nothing here puts the
    // rod back. With no fire timer and no pull force, no branch touches the
    // speed again, so whatever the ball gave it is still there next step and
    // every step after: the rod creeps off on its own and parks against a
    // stop, in full view, for the rest of the ball.
    let mut p = plunger();
    let resting = p.pos;
    let mut b = ball_in_the_lane(&p);

    // A ball leaning on the tip, the way one waiting to be launched does.
    for _ in 0..200 {
        b.vel = Vec3::new(0.0, 2.0, 0.0); // pressed against the rod
        if let Some(coll) = p.hit_test(&b, PHYS_FACTOR) {
            p.collide(&mut b, &coll, 0.0);
        }
        step(&mut p);
    }

    assert!(
        (p.pos - resting).abs() < 1.0,
        "the rod started at {resting:.1} and has wandered to {:.1}",
        p.pos
    );
}

#[test]
fn a_ball_hitting_a_moving_rod_does_not_send_it_to_the_back_stop() {
    // A ball that hits the tip gives the plunger a shove back
    // (`hitplunger.cpp:747`), but on a keyboard plunger the rod never feels it:
    // the line that applies it is the last one inside `else if (isMech)`
    // (`hitplunger.cpp:470`).
    //
    // Applying it anyway sticks, because in this mode no branch puts the rod
    // back — so it does not wobble, it leaves. Fired and still bouncing, with a
    // ball coming back down the lane at it, the rod ends up against the fully
    // retracted stop and stays there for the rest of the ball.
    let mut p = plunger();
    p.pull();
    for _ in 0..200 {
        step(&mut p);
    }
    let back_stop = p.pos;
    p.release();

    let mut e = Engine::new(vec![Shape::Plunger(p)], Vec3::ZERO);
    let i = e.plunger_indices().next().unwrap();
    e.release_plunger(i);

    for n in 0..6000 {
        // A ball arriving from up the lane, again and again.
        if n % 300 == 0 {
            while !e.balls.is_empty() {
                e.remove_ball(0);
            }
            let tip = e.plunger(i).unwrap().pos;
            let mut b = Ball::new(Vec3::new(905.0, tip - R - 30.0, R), R);
            b.vel = Vec3::new(0.0, 30.0, 0.0);
            e.add_ball(b);
        }
        e.step();
    }

    let rod = e.plunger(i).unwrap().pos;
    assert!(
        (rod - REST).abs() < 5.0,
        "the rod should have stayed near {REST:.1}, and it is at {rod:.1}          (the back stop is {back_stop:.1})"
    );
}

// --------------------------------------------------- what a screen shows ---
//
// A shooter drawn on screen is drawn from `travel`, not from `relative_position`
// and not from how far a finger moved. These pin down the difference.

#[test]
fn a_parked_plunger_has_not_been_drawn_back_at_all() {
    // The raw position reads 0.17 here — the park position the table asked for
    // — and a spring drawn from that number is a spring already squashed by a
    // sixth before anybody has touched the control.
    let p = plunger();
    assert!(
        p.relative_position() > 0.1,
        "the fixture should park part way"
    );
    assert!(
        p.travel() < 1e-4,
        "at rest it has travelled nothing: {}",
        p.travel()
    );
}

#[test]
fn holding_the_button_draws_it_all_the_way_back() {
    let mut p = plunger();
    p.pull();
    let mut seen: Vec<f32> = Vec::new();
    for _ in 0..1000 {
        p.update_velocities();
        p.update_displacements(0.001);
        seen.push(p.travel());
    }

    // It gets there, it never leaves the range, and it only ever goes one way
    // while the button is held — a picture that jitters backwards is a picture
    // that reads as a bug.
    assert!(
        p.travel() > 0.99,
        "held down it should end up fully drawn: {}",
        p.travel()
    );
    assert!(
        seen.iter().all(|t| (0.0..=1.0).contains(t)),
        "travel left the 0..1 range"
    );
    assert!(
        seen.windows(2).all(|w| w[1] >= w[0] - 1e-6),
        "it moved backwards while the button was held"
    );
}

#[test]
fn firing_forward_never_reads_as_less_than_parked() {
    // Released, it overshoots the park position going forward. That is a real
    // movement of a real rod and it is still not "drawn back by a negative
    // amount", which is what a naive subtraction would say and what would make
    // a spring drawn from it turn inside out.
    let mut p = plunger();
    p.pull();
    for _ in 0..1000 {
        p.update_velocities();
        p.update_displacements(0.001);
    }
    p.release();
    for _ in 0..2000 {
        p.update_velocities();
        p.update_displacements(0.001);
        let t = p.travel();
        assert!(
            (0.0..=1.0).contains(&t),
            "travel left the 0..1 range while firing: {t}"
        );
    }
}

// ------------------------------------------------- pulled by a finger ---
//
// The plunger key cannot say *how far*: a key is down or it is not, so held
// down it draws the rod back on its own. A finger on a screen can, and that is
// what a plunger actually is — you pull it as far as you mean to. These cover
// that path, which is the original's mechanical-plunger one.

/// Runs the rod for `ms` milliseconds with nothing else going on.
fn settle(p: &mut Plunger, ms: u32) {
    for _ in 0..ms {
        p.update_velocities();
        p.update_displacements(0.001);
    }
}

#[test]
fn holding_it_part_way_takes_it_part_way() {
    let mut p = plunger();
    p.hold_at(0.5);
    settle(&mut p, 500);
    assert!(
        (p.travel() - 0.5).abs() < 0.02,
        "held at the halfway point it should get there: {}",
        p.travel()
    );
}

#[test]
fn the_rod_does_not_jump_to_the_finger() {
    // The whole reason the original attaches the two with a spring instead of
    // assigning the position: a rod that teleports moves at infinite speed
    // between two frames, and nothing the collision code does can see it pass.
    let mut p = plunger();
    let before = p.travel();
    p.hold_at(1.0);
    p.update_velocities();
    p.update_displacements(0.001);
    assert!(
        p.travel() - before < 0.2,
        "one millisecond took it from {before} to {}",
        p.travel()
    );
}

#[test]
fn a_short_pull_of_the_finger_is_a_softer_shot() {
    // The point of the control. Pull it a little, it goes a little.
    let speed_after = |travel: f32| {
        let mut p = plunger();
        p.hold_at(travel);
        settle(&mut p, 600);
        p.let_go();
        p.update_velocities();
        p.speed.abs()
    };
    let (soft, hard) = (speed_after(0.25), speed_after(1.0));
    assert!(
        hard > soft * 1.5,
        "a full pull should be much harder than a quarter one: {soft} vs {hard}"
    );
}

#[test]
fn letting_go_from_rest_does_almost_nothing_either() {
    // Touching the control and letting go without dragging is not a shot.
    let mut p = plunger();
    p.hold_at(0.0);
    settle(&mut p, 100);
    p.let_go();
    p.update_velocities();
    assert!(
        p.speed.abs() < 20.0,
        "a touch with no pull should not launch anything: {}",
        p.speed
    );
}

#[test]
fn the_key_still_works_the_way_a_key_has_to() {
    // Adding the finger must not have taken the keyboard away: held down, the
    // key still draws the rod back on its own.
    let mut p = plunger();
    p.hold_at(0.6);
    settle(&mut p, 200);
    p.pull();
    settle(&mut p, 1000);
    assert!(
        p.travel() > 0.99,
        "the key should still draw it all the way back: {}",
        p.travel()
    );
}

#[test]
fn it_follows_the_finger_quickly_and_settles() {
    // How the control feels, pinned. Tracking a finger has to be fast enough
    // to look attached to it — about fifty milliseconds for the length of the
    // frame — and the overshoot afterwards is the rod's own weight against the
    // spring, which the original models and which is worth keeping. What is
    // not worth keeping is an unbounded one, so it is bounded here.
    let mut p = plunger();
    p.hold_at(0.5);

    let mut peak = 0.0f32;
    let mut arrived = None;
    for ms in 0..600 {
        p.update_velocities();
        p.update_displacements(0.001);
        peak = peak.max(p.travel());
        if arrived.is_none() && p.travel() >= 0.5 {
            arrived = Some(ms);
        }
    }

    let arrived = arrived.expect("it never reached the finger");
    assert!(
        arrived < 100,
        "it took {arrived} ms to reach the finger, which reads as lag"
    );
    assert!(
        peak < 0.62,
        "it overshot to {peak}, which reads as the control being loose"
    );
    assert!(
        (p.travel() - 0.5).abs() < 0.02,
        "it settled at {} instead of where the finger is",
        p.travel()
    );
}
