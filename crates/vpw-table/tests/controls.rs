//! Tests of the key mapping.
//!
//! Space held pulls the plunger and fires on release; `Z` and `M` are the
//! flippers. What is checked is that the key reaches the right piece, and above
//! all that the plunger tells pressing from releasing: its strength comes from
//! how long it was held down.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Shape};
use vpw_table::controls::{Action, Controls};
use vpw_table::physics::Button;

fn table() -> Option<vpin::vpx::VPX> {
    let path = std::path::Path::new("../../web/debug-assets/f14.vpx");
    let bytes = std::fs::read(path).ok()?;
    vpin::vpx::from_bytes(&bytes).ok()
}

fn engine(vpx: &vpin::vpx::VPX) -> Engine {
    let g = &vpx.gamedata;
    let mut e = Engine::new(
        vpw_table::physics::build(vpx),
        Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY),
    );
    e.slope_rad = g.angle_tilt_max.to_radians();
    e
}

// ----------------------------------------------------------- the mapping ---

#[test]
fn the_three_keys_are_mapped() {
    assert_eq!(Action::from_key_code("KeyZ"), Some(Action::LeftFlipper));
    assert_eq!(Action::from_key_code("KeyM"), Some(Action::RightFlipper));
    assert_eq!(Action::from_key_code("Space"), Some(Action::Plunger));
}

#[test]
fn any_other_key_does_nothing() {
    assert_eq!(Action::from_key_code("KeyQ"), None);
    assert_eq!(Action::from_key_code("Enter"), None);
    // It is `code`, the physical position of the key, not `key`. Passing the
    // bare letter must not work, and if some day somebody swaps it for `key`
    // this test will say so.
    assert_eq!(Action::from_key_code("z"), None);
}

// ------------------------------------------------------- on a real table ---

#[test]
fn each_button_moves_the_flippers_on_its_side() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);

    let left = c.flippers(Button::Left).to_vec();
    let right = c.flippers(Button::Right).to_vec();
    assert_eq!(left.len(), 2, "F-14 has two flippers on the left");
    assert_eq!(right.len(), 2, "and two on the right");

    let angle = |e: &Engine, i: usize| e.flipper(i).unwrap().angle_cur;
    let rest: Vec<f32> = left.iter().map(|&i| angle(&e, i)).collect();

    c.key(&mut e, "KeyZ", true);
    for _ in 0..200 {
        e.step();
    }

    for (n, &i) in left.iter().enumerate() {
        assert_ne!(angle(&e, i), rest[n], "left flipper {i} did not move");
    }
    for &i in &right {
        assert_eq!(
            angle(&e, i),
            e.flipper(i).unwrap().angle_start,
            "right flipper {i} moved and it should not have"
        );
    }
}

#[test]
fn releasing_the_key_lowers_the_flipper() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);
    let i = c.flippers(Button::Left)[0];
    let rest = e.flipper(i).unwrap().angle_start;

    c.key(&mut e, "KeyZ", true);
    for _ in 0..200 {
        e.step();
    }
    assert_ne!(e.flipper(i).unwrap().angle_cur, rest, "up");

    c.key(&mut e, "KeyZ", false);
    for _ in 0..600 {
        e.step();
    }
    assert!(
        (e.flipper(i).unwrap().angle_cur - rest).abs() < 0.05,
        "it should have come back: {} against {rest}",
        e.flipper(i).unwrap().angle_cur
    );
}

#[test]
fn losing_focus_releases_everything() {
    // If the window loses focus, another one gets the `keyup` and the flipper
    // stays up forever.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);
    let i = c.flippers(Button::Left)[0];
    let rest = e.flipper(i).unwrap().angle_start;

    c.key(&mut e, "KeyZ", true);
    c.key(&mut e, "KeyM", true);
    for _ in 0..200 {
        e.step();
    }

    c.release_all(&mut e);
    for _ in 0..600 {
        e.step();
    }
    assert!((e.flipper(i).unwrap().angle_cur - rest).abs() < 0.05);
}

// ----------------------------------------------------------- the plunger ---

#[test]
fn the_players_plunger_is_not_the_kickback() {
    // F-14 has two plungers: the one in the shooter lane and a `KickBack`, which
    // is the mechanism that returns the ball from the left outlane and is fired
    // by the ROM. If space moved that one, the player would press and nothing
    // would happen where they are looking.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let e = engine(&vpx);
    assert_eq!(e.plunger_indices().count(), 2, "F-14 has two plungers");

    let c = Controls::new(&vpx, &e);
    let i = c.plunger().expect("it has to have picked one");
    let Shape::Plunger(p) = &e.shapes()[i] else {
        panic!("that index is not a plunger")
    };

    // The shooter lane one is all the way to the right; the KickBack, to the
    // left.
    let x = p.bbox().min.x;
    assert!(
        x > (vpx.gamedata.left + vpx.gamedata.right) * 0.5,
        "it picked the left one, which is the KickBack: x={x}"
    );
}

#[test]
fn holding_space_pulls_the_plunger() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);
    let i = c.plunger().unwrap();
    let rest = e.plunger(i).unwrap().relative_position();

    c.key(&mut e, "Space", true);
    for _ in 0..60 {
        e.step();
    }
    let halfway = e.plunger(i).unwrap().relative_position();
    assert!(
        halfway > rest,
        "it should be going backwards: {rest} -> {halfway}"
    );

    for _ in 0..200 {
        e.step();
    }
    assert!(
        e.plunger(i).unwrap().relative_position() > halfway,
        "and keep going while the key stays down"
    );
}

#[test]
fn keyboard_repeat_does_not_restart_the_pull() {
    // The browser repeats the `keydown` while the key is held. If every
    // repetition called `pull` again, the velocity would be zeroed and the
    // plunger would never reach the bottom.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);
    let i = c.plunger().unwrap();

    for _ in 0..30 {
        c.key(&mut e, "Space", true); // repetition
        for _ in 0..10 {
            e.step();
        }
    }
    let with_repeat = e.plunger(i).unwrap().relative_position();

    let mut e2 = engine(&vpx);
    let mut c2 = Controls::new(&vpx, &e2);
    c2.key(&mut e2, "Space", true);
    for _ in 0..300 {
        e2.step();
    }
    let without_repeat = e2.plunger(i).unwrap().relative_position();

    assert!(
        (with_repeat - without_repeat).abs() < 1e-3,
        "the repetition changed the result: {with_repeat} against {without_repeat}"
    );
}

#[test]
fn releasing_space_fires_the_ball() {
    // The end-to-end proof of what the player asked for: hold space, let go, and
    // watch the ball leave the lane.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let mut e = engine(&vpx);
    let mut c = Controls::new(&vpx, &e);
    let i = c.plunger().unwrap();

    let start = vpw_table::controls::ball_start_point(&e, i, DEFAULT_BALL_SIZE)
        .expect("the plunger has to say where the ball goes");
    e.add_ball(Ball::new(start, DEFAULT_BALL_SIZE));

    // Let it settle against the tip.
    for _ in 0..200 {
        e.step();
    }
    let y_before = e.balls[0].pos.y;

    c.key(&mut e, "Space", true);
    for _ in 0..300 {
        e.step();
    }
    c.key(&mut e, "Space", false);

    let mut highest = y_before;
    for _ in 0..2000 {
        e.step();
        highest = highest.min(e.balls[0].pos.y);
    }

    let climbed = y_before - highest;
    eprintln!("the ball left the lane by {climbed:.0} units");
    assert!(
        climbed > 300.0,
        "the plunger should have sent it well up the table; it climbed {climbed:.0}"
    );
}

#[test]
fn a_short_pull_sends_the_ball_less_far() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let travel = |steps_held: usize| {
        let mut e = engine(&vpx);
        let mut c = Controls::new(&vpx, &e);
        let i = c.plunger().unwrap();
        let start = vpw_table::controls::ball_start_point(&e, i, DEFAULT_BALL_SIZE).unwrap();
        e.add_ball(Ball::new(start, DEFAULT_BALL_SIZE));
        for _ in 0..200 {
            e.step();
        }
        let y = e.balls[0].pos.y;

        c.key(&mut e, "Space", true);
        for _ in 0..steps_held {
            e.step();
        }
        c.key(&mut e, "Space", false);

        let mut up = y;
        for _ in 0..2000 {
            e.step();
            up = up.min(e.balls[0].pos.y);
        }
        y - up
    };

    let soft = travel(35);
    let hard = travel(300);
    eprintln!("short pull: {soft:.0}, long pull: {hard:.0}");
    assert!(
        hard > soft,
        "a longer pull has to reach further: {soft:.0} against {hard:.0}"
    );
}

// So that `Vec3` gets used and the import is not redundant.
#[test]
fn the_ball_comes_out_of_the_plunger_lane() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let e = engine(&vpx);
    let c = Controls::new(&vpx, &e);
    let i = c.plunger().unwrap();
    let start: Vec3 = vpw_table::controls::ball_start_point(&e, i, DEFAULT_BALL_SIZE).unwrap();

    let Shape::Plunger(p) = &e.shapes()[i] else {
        panic!()
    };
    let bbox = p.bbox();
    assert!(
        start.x >= bbox.min.x && start.x <= bbox.max.x,
        "the ball has to appear inside the lane: {start:?} in {bbox:?}"
    );
    assert!(start.z > 0.0, "and resting on it, not under the floor");
}
