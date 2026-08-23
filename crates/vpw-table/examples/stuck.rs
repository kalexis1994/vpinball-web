//! Puts a ball where telemetry says one got stuck, and reports what is holding
//! it there.
//!
//! ```text
//! cargo run --release -p vpw-table --example stuck -- table.vpx X Y [Z] [seconds]
//! ```
//!
//! It answers three questions in order, because they fail differently: what
//! geometry is within reach of that point, which of it the ball is actually
//! touching, and whether the ball ever leaves. A ball that is wedged reports a
//! contact every step and does not move; a ball that is merely resting reports
//! the playfield and rolls away.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Shape};

fn kind(s: &Shape) -> &'static str {
    match s {
        Shape::Line(_) => "wall",
        Shape::Circle(_) => "circle",
        Shape::Plane(_) => "playfield",
        Shape::Point(_) => "point",
        Shape::Poly(_) => "polygon",
        Shape::Triangle(_) => "triangle",
        Shape::LineZ(_) => "pole",
        Shape::Line3D(_) => "edge",
        Shape::Slingshot(_) => "slingshot",
        Shape::Bumper(_) => "bumper",
        Shape::Kicker(_) => "kicker",
        Shape::Gate(_) => "gate",
        Shape::Spinner(_) => "spinner",
        Shape::Flipper(_) => "flipper",
        Shape::Plunger(_) => "plunger",
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: stuck <table.vpx> <x> <y> [z] [seconds]");
    let x: f32 = args.next().expect("x").parse().expect("x is a number");
    let y: f32 = args.next().expect("y").parse().expect("y is a number");
    let z: f32 = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BALL_SIZE);
    let seconds: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(3.0);

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let g = &vpx.gamedata;
    let collision = vpw_table::physics::build_with_owners(&vpx);
    let shapes = collision.shapes;

    // Which game item each shape came from, by name: "a wall" is not something
    // anybody can go and look at, and `Wall272` is.
    let name_of = |i: usize| -> String {
        match collision.owners.get(i).copied().flatten() {
            Some(item) => vpx
                .gameitems
                .get(item)
                .map(|g| g.name().to_string())
                .unwrap_or_else(|| format!("item {item}")),
            None => "the table itself".to_string(),
        }
    };

    let at = Vec3::new(x, y, z);
    println!("table          {}", path);
    println!("ball at        ({x:.1}, {y:.1}, {z:.1})");
    println!("shapes         {}", shapes.len());

    // What is within reach. A ball touches at its radius, so anything whose box
    // comes within a radius and a bit is a candidate for holding it.
    let reach = DEFAULT_BALL_SIZE + 2.0;
    let mut near: Vec<(f32, usize, &'static str)> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        // A shape with no box is unbounded — the playfield plane. It is always
        // in reach and never the thing that traps a ball.
        let Some(b) = s.bbox() else { continue };
        let d = Vec3::new(
            (b.min.x - at.x).max(0.0).max(at.x - b.max.x),
            (b.min.y - at.y).max(0.0).max(at.y - b.max.y),
            (b.min.z - at.z).max(0.0).max(at.z - b.max.z),
        )
        .length();
        if d <= reach {
            near.push((d, i, kind(s)));
        }
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!();
    println!("within {reach:.0} units: {} shapes", near.len());
    for (d, i, k) in near.iter().take(25) {
        let b = shapes[*i].bbox().unwrap();
        println!(
            "  {d:6.2}  {k:10} #{i:<6} {:<14} x {:8.1}..{:<8.1} y {:8.1}..{:<8.1} z {:7.1}..{:<7.1}",
            name_of(*i),
            b.min.x,
            b.max.x,
            b.min.y,
            b.max.y,
            b.min.z,
            b.max.z
        );
    }
    if near.len() > 25 {
        println!("  ... and {} more", near.len() - 25);
    }

    // What it is actually touching, which is a different question from what is
    // nearby: a box can be within reach and the surface inside it still be out
    // of it.
    let ball = Ball::new(at, DEFAULT_BALL_SIZE);
    println!();
    println!("touching right now:");
    let mut touched = 0;
    for (_, i, k) in &near {
        if let Some(c) = shapes[*i].hit_test(&ball, 0.001) {
            touched += 1;
            println!(
                "  {k:10} #{i:<6} {:<14} distance {:8.3}  normal ({:6.3},{:6.3},{:6.3})",
                name_of(*i),
                c.hit_distance,
                c.hit_normal.x,
                c.hit_normal.y,
                c.hit_normal.z
            );
        }
    }
    if touched == 0 {
        println!("  nothing within reach reports a hit");
    }

    // Now put a ball there and see whether it leaves.
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut engine = Engine::new(shapes, gravity);
    engine.slope_rad = g.angle_tilt_max.to_radians();
    engine.add_ball(Ball::new(at, DEFAULT_BALL_SIZE));

    let steps = (seconds * 1000.0) as usize;
    let mut furthest = 0.0f32;
    for i in 0..steps {
        engine.step();
        let p = engine.balls[0].pos;
        furthest = furthest.max((p - at).length());
        if i % 250 == 0 || i + 1 == steps {
            let v = engine.balls[0].vel;
            println!(
                "{:5} ms  ({:7.1},{:7.1},{:6.1})  v=({:7.3},{:7.3},{:7.3})  |v|={:6.3}",
                i + 1,
                p.x,
                p.y,
                p.z,
                v.x,
                v.y,
                v.z,
                v.length()
            );
        }
    }

    println!();
    println!("furthest it got from where it started: {furthest:.1} units");
    if furthest < DEFAULT_BALL_SIZE {
        println!("it is stuck: it never moved as much as its own radius");
    }
}
