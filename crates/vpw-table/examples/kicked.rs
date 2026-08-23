//! Kicks a ball out of a named kicker and follows it, on the real geometry.
//!
//! ```text
//! cargo run --release -p vpw-table --example kicked -- table.vpx sw24a 270 28 [seconds]
//! ```
//!
//! The angle is Visual Pinball's: degrees from straight up the table, turning
//! clockwise, so the velocity is `(sin a, -cos a)` (`kicker.cpp:749`).

use vpin::vpx::gameitem::GameItemEnum as G;
use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::Engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: kicked <table.vpx> <kicker> <angle> <force> [s]");
    let want = args.next().expect("kicker name");
    let angle: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let force: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let seconds: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(3.0);

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let g = &vpx.gamedata;

    let mut at = None;
    for item in &vpx.gameitems {
        if let G::Kicker(k) = item
            && k.name.eq_ignore_ascii_case(&want)
        {
            at = Some((
                k.center.x,
                k.center.y,
                vpw_table::builtin::surface_height(
                    &vpx.gameitems,
                    &k.surface,
                    k.center.x,
                    k.center.y,
                ),
                k.kicker_type.clone(),
            ));
        }
    }
    let (x, y, surface, kind) = at.unwrap_or_else(|| panic!("no kicker called {want}"));
    println!("kicker {want}: centre ({x:.1}, {y:.1}) on a surface at z={surface} type {kind:?}");

    let a = angle.to_radians();
    let vel = Vec3::new(a.sin(), -a.cos(), 0.0) * force;
    println!(
        "kick   {angle} degrees at {force}  ->  v = ({:.2}, {:.2}, {:.2})",
        vel.x, vel.y, vel.z
    );

    let collision = vpw_table::physics::build_with_owners(&vpx);
    let owners = collision.owners.clone();
    let shapes = collision.shapes;
    let names: Vec<String> = owners
        .iter()
        .map(|o| match o.and_then(|i| vpx.gameitems.get(i)) {
            Some(g) => g.name().to_string(),
            None => "the table itself".to_string(),
        })
        .collect();
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut engine = Engine::new(shapes, gravity);
    engine.slope_rad = g.angle_tilt_max.to_radians();

    // `Kicker::CreateSizedBallWithMass` puts the ball's **centre** at the
    // surface height (`kicker.cpp:661`), so it sits sunk into the saucer. An
    // extra argument raises it, to see what a different height would do.
    let lift: f32 = std::env::args()
        .nth(6)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let start = Vec3::new(x, y, surface + lift);
    let mut ball = Ball::new(start, DEFAULT_BALL_SIZE);
    ball.vel = vel;
    engine.add_ball(ball);
    println!(
        "start  ({:.1}, {:.1}, {:.1})   (lift {lift})",
        start.x, start.y, start.z
    );
    println!();

    let steps = (seconds * 1000.0) as usize;
    let mut was = vel;
    for i in 0..steps {
        // What is in front of it *before* the step: after one, whatever it hit
        // is already behind and reports nothing.
        let before = engine.balls[0].clone();
        let mut hit = Vec::new();
        for (j, sh) in engine.shapes().iter().enumerate() {
            if sh.hit_test(&before, 0.001).is_some() {
                hit.push(format!("{} #{j}", names[j]));
            }
        }

        engine.step();

        // A sudden change of velocity is a bounce. Say what off.
        let now = engine.balls[0].vel;
        if (now - was).length() > 3.0 {
            let b = engine.balls[0].clone();
            println!(
                "{:5} ms  BOUNCE at ({:7.1},{:7.1},{:6.1})  v {:?} -> {:?}   touching: {}",
                i + 1,
                b.pos.x,
                b.pos.y,
                b.pos.z,
                (was.x.round(), was.y.round(), was.z.round()),
                (now.x.round(), now.y.round(), now.z.round()),
                if hit.is_empty() {
                    "nothing".to_string()
                } else {
                    hit.join(", ")
                }
            );
        }
        was = now;

        if i % 20 == 0 || i + 1 == steps {
            let b = &engine.balls[0];
            println!(
                "{:5} ms  ({:7.1},{:7.1},{:6.1})  v=({:7.2},{:7.2},{:7.2})",
                i + 1,
                b.pos.x,
                b.pos.y,
                b.pos.z,
                b.vel.x,
                b.vel.y,
                b.vel.z
            );
        }
    }
}
