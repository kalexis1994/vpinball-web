//! Drops a ball onto a real table and reports what happens.
//!
//! ```text
//! cargo run --release -p vpw-table --example drop -- table.vpx [seconds]
//! ```
//!
//! It is the end-to-end check of the physics: no hand-built shapes, everything
//! comes out of the `.vpx`.

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Shape};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: drop <table.vpx> [seconds]");
    let seconds: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(10.0);

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let g = &vpx.gamedata;

    let t0 = std::time::Instant::now();
    let shapes = vpw_table::physics::build(&vpx);
    let building = t0.elapsed();

    let (mut lines, mut circles, mut planes, mut points, mut polygons) = (0, 0, 0, 0, 0);
    let mut slingshots = 0;
    let (mut bumpers, mut kickers) = (0, 0);
    let (mut gates, mut spinners) = (0, 0);
    let mut flippers = 0;
    let mut plungers = 0;
    let mut joints = 0;
    let mut triangles = 0;
    for s in &shapes {
        match s {
            Shape::Line(_) => lines += 1,
            Shape::Circle(_) => circles += 1,
            Shape::Plane(_) => planes += 1,
            Shape::Point(_) => points += 1,
            Shape::Poly(_) => polygons += 1,
            Shape::Slingshot(_) => slingshots += 1,
            Shape::Bumper(_) => bumpers += 1,
            Shape::Kicker(_) => kickers += 1,
            Shape::Gate(_) => gates += 1,
            Shape::Spinner(_) => spinners += 1,
            Shape::Flipper(_) => flippers += 1,
            Shape::Plunger(_) => plungers += 1,
            // The poles that seal the seams of a ramp. There are thousands.
            Shape::LineZ(_) | Shape::Line3D(_) => joints += 1,
            // The mesh of a target. Counted apart from the polygons because it
            // is a different test — see `HitTriangle`.
            Shape::Triangle(_) => triangles += 1,
        }
    }

    println!(
        "table          {}",
        vpx.info.table_name.clone().unwrap_or_default()
    );
    println!(
        "playfield      {:.0} x {:.0}",
        g.right - g.left,
        g.bottom - g.top
    );
    println!("slope          {:.1} degrees", g.angle_tilt_max);
    println!();
    println!("shapes         {} in total", shapes.len());
    println!("  walls        {lines}");
    println!("  circles      {circles}");
    println!("  points       {points}");
    println!("  polygons     {polygons}");
    println!("  slingshots   {slingshots}");
    println!("  bumpers      {bumpers}");
    println!("  kickers      {kickers}");
    println!("  gates        {gates}");
    println!("  spinners     {spinners}");
    println!("  flippers     {flippers}");
    println!("  plungers     {plungers}");
    println!("  planes       {planes}");
    println!("  triangles    {triangles}");
    println!("  joints       {joints} (the seams of ramps and rubbers)");
    println!(
        "  triggers     {} (outside the tree)",
        vpw_table::physics::triggers(&vpx).len()
    );
    println!("building       {building:?}");

    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut engine = Engine::new(shapes, gravity);
    engine.slope_rad = g.angle_tilt_max.to_radians();

    // Drop the ball right at the top, in the middle of the table.
    let start = Vec3::new((g.left + g.right) * 0.5, (g.top + g.bottom) * 0.25, 200.0);
    engine.add_ball(Ball::new(start, DEFAULT_BALL_SIZE));
    println!();
    println!(
        "dropped at     ({:.0}, {:.0}, {:.0})",
        start.x, start.y, start.z
    );

    let steps = (seconds * 1000.0) as usize;
    let t1 = std::time::Instant::now();
    let (mut z_min, mut z_max) = (f32::MAX, f32::MIN);
    let mut travelled = 0.0f32;
    let mut previous = start;
    let mut escaped = None;

    for i in 0..steps {
        engine.step();
        let p = engine.balls[0].pos;
        z_min = z_min.min(p.z);
        z_max = z_max.max(p.z);
        travelled += (p - previous).length();
        previous = p;

        // Did it leave the table?
        let margin = 200.0;
        if escaped.is_none()
            && (p.x < g.left - margin
                || p.x > g.right + margin
                || p.y < g.top - margin
                || p.y > g.bottom + margin
                || p.z < -margin)
        {
            escaped = Some((i, p));
        }
    }
    let simulated = t1.elapsed();

    let b = &engine.balls[0];
    println!(
        "ended at       ({:.0}, {:.0}, {:.0})",
        b.pos.x, b.pos.y, b.pos.z
    );
    println!("speed          {:.2}", b.vel.length());
    println!("travelled      {travelled:.0} units");
    println!("height         between {z_min:.0} and {z_max:.0}");
    println!();
    match escaped {
        Some((step, p)) => println!(
            "IT ESCAPED at step {step} ({:.0}, {:.0}, {:.0})",
            p.x, p.y, p.z
        ),
        None => println!("it did not escape from the table"),
    }

    let simulated_ms = steps as f64;
    let real_ms = simulated.as_secs_f64() * 1000.0;
    println!();
    println!(
        "simulating     {simulated:?} for {seconds}s of table ({:.0}x real time)",
        simulated_ms / real_ms.max(0.001)
    );
}
