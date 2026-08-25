//! The physics on a real table.
//!
//! The tests skip themselves if the table is missing: it does not go in the repo
//! because it weighs over a hundred megabytes.
//!
//! ```text
//! cargo test --release -p vpw-table
//! ```

use vpin::vpx::gameitem::GameItemEnum;
use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Event, Shape};
use vpw_physics::trigger::Crossing;
use vpw_table::physics::Button;

fn table() -> Option<vpin::vpx::VPX> {
    let path = std::path::Path::new("../../web/debug-assets/f14.vpx");
    let bytes = std::fs::read(path).ok()?;
    vpin::vpx::from_bytes(&bytes).ok()
}

#[test]
fn a_real_table_generates_thousands_of_shapes() {
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let shapes = vpw_table::physics::build(&vpx);

    assert!(
        shapes.len() > 1000,
        "F-14 should give thousands of shapes; it gave {}",
        shapes.len()
    );

    // And of all four kinds: a wall is not just its side.
    let (mut lines, mut circles, mut points, mut polygons) = (0, 0, 0, 0);
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
            Shape::Plane(_) => {}
        }
    }
    assert!(
        lines > 0 && circles > 0 && points > 0 && polygons > 0,
        "kinds missing: {lines} walls, {circles} circles, {points} points, {polygons} polygons"
    );

    // F-14 is built around eighteen stand-up targets, and every one of them is
    // a mesh. Nothing else on this table produces a triangle, so this counts
    // them: without it the table loads, plays, and quietly has no targets.
    assert!(
        joints > 1000,
        "the seams between surfaces should be sealed by thousands of joints, \
         and there are {joints}"
    );
    assert!(
        triangles > 1000,
        "F-14's eighteen targets should be thousands of triangles, and there are {triangles}"
    );

    // Every vertex of a wall needs its vertical joint, otherwise the ball snags
    // on the corners. It is the invariant that verifies none of them went
    // ungenerated.
    //
    // Gates and spinners unbalance it on purpose and have to be discounted: a
    // one-way gate adds a rigid segment that is not part of any outline, and the
    // brackets of both add posts that are not joints. That the discount balances
    // exactly is, incidentally, the proof that the bridge for those two pieces
    // generates what it says it generates.
    let mut gate_segments = 0;
    let mut bracket_posts = 0;
    for item in &vpx.gameitems {
        match item {
            GameItemEnum::Gate(g) if g.is_collidable => {
                if !g.two_way {
                    gate_segments += 1;
                }
                if g.show_bracket {
                    bracket_posts += 2;
                }
            }
            GameItemEnum::Spinner(sp) if sp.show_bracket => bracket_posts += 2,
            _ => {}
        }
    }

    // Every wall vertex gets a vertical joint, so the two counts move together.
    // Ramps break the arithmetic — they seal their own seams with `LineZ` and
    // `Line3D` rather than circles — so their lines are taken back out. What is
    // left is the walls, rubbers and the rest, where the invariant still holds.
    let ramp_lines = ramp_line_count(&vpx);
    // The cabinet's four sides come out too. They are the one place the
    // original builds bare segments with no joints between them
    // (`PhysicsEngine.cpp:174-178`): the corners of the world are the one set
    // of corners no ball can arrive at from outside.
    const CABINET_SIDES: usize = 4;
    assert_eq!(
        lines + slingshots - gate_segments - ramp_lines - CABINET_SIDES,
        circles - bracket_posts,
        "every vertex needs its vertical joint \
         ({lines} lines - {ramp_lines} from ramps + {slingshots} slingshots \
         - {gate_segments} from gates against {circles} circles \
         - {bracket_posts} posts)"
    );

    // And the active pieces, which count themselves.
    assert_eq!(slingshots, 2, "F-14 has two slingshots");
    assert_eq!(bumpers, 1, "F-14 has one bumper");
    assert_eq!(kickers, 8, "F-14 has eight kickers");
    assert!(gates + spinners > 0, "F-14 has gates and spinners");
    // Six, not two: the two at the bottom, two up top that answer to the same
    // buttons, and two `Diverter`s moved by the ROM. All six are solid obstacles
    // even though only four are driven from the buttons.
    assert_eq!(flippers, 6, "F-14 has six flippers");
    // Two, not one: the shooter lane one and a `KickBack`, which is the mechanism
    // that returns the ball from the left outlane. Both are collision shapes;
    // which one answers to the button is decided by `player_plunger`.
    assert_eq!(plungers, 2, "F-14 has two plungers");
}

#[test]
fn f14s_flippers_go_to_the_right_buttons() {
    // F-14 is a good case because it has **six** flippers: the two at the
    // bottom, two up top that answer to the same buttons, and two `Diverter`s
    // moved by the ROM that the player never touches. Splitting them by position
    // would tie both diverters to the left button — both are to the left of the
    // axis — and they would jerk about on every shot.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let shapes = vpw_table::physics::build(&vpx);
    let assignment = vpw_table::physics::assign_flippers(&vpx, &shapes);

    assert_eq!(
        assignment.len(),
        4,
        "the four player ones have to be left, without the two diverters"
    );

    let left = assignment
        .iter()
        .filter(|(_, b)| *b == Button::Left)
        .count();
    let right = assignment
        .iter()
        .filter(|(_, b)| *b == Button::Right)
        .count();
    assert_eq!(
        (left, right),
        (2, 2),
        "two per side: the bottom one and the top one"
    );

    // And each one on the side it belongs to.
    let axis = (vpx.gamedata.left + vpx.gamedata.right) * 0.5;
    for &(i, button) in &assignment {
        let f = match &shapes[i] {
            Shape::Flipper(f) => f,
            _ => panic!("index {i} is not a flipper"),
        };
        let expected = if f.center.x < axis {
            Button::Left
        } else {
            Button::Right
        };
        assert_eq!(
            button, expected,
            "the flipper at x={:.0} ended up on the wrong button",
            f.center.x
        );
    }
}

#[test]
fn an_f14_flipper_gets_the_ball_out_of_the_bottom() {
    // The proof that the whole chain works: real table, flipper built from the
    // .vpx, ball falling towards the drain, button pressed. If it goes up, it is
    // playable.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let shapes = vpw_table::physics::build(&vpx);
    let assignment = vpw_table::physics::assign_flippers(&vpx, &shapes);

    // The bottom left flipper: the one closest to the player.
    let (index, _) = *assignment
        .iter()
        .filter(|(i, b)| *b == Button::Left && matches!(&shapes[*i], Shape::Flipper(_)))
        .max_by(|a, b| {
            let y = |i: usize| match &shapes[i] {
                Shape::Flipper(f) => f.center.y,
                _ => f32::MIN,
            };
            y(a.0).total_cmp(&y(b.0))
        })
        .expect("F-14 has a left flipper");

    let Shape::Flipper(f) = &shapes[index] else {
        unreachable!()
    };
    let (center, radius, z) = (f.center, f.flipper_radius, f.z_low);

    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut e = Engine::new(shapes.clone(), gravity);
    e.slope_rad = g.angle_tilt_max.to_radians();

    // The ball falling right onto the tip of the flipper at rest.
    let tip = e.flipper(index).unwrap().tip_center();
    e.add_ball(Ball::new(
        Vec3::new(tip.x, tip.y - radius * 0.35, z + DEFAULT_BALL_SIZE + 40.0),
        DEFAULT_BALL_SIZE,
    ));

    // Let it come down and settle.
    for _ in 0..400 {
        e.step();
    }
    let before = e.balls[0].pos;
    assert!(
        (before.x - center.x).abs() < radius * 2.0 && before.z < z + 100.0,
        "the ball should have fallen onto the flipper: {before:?}"
    );

    // And now the button.
    e.set_flipper_solenoid(index, true);
    let mut highest = before.y;
    for _ in 0..1500 {
        e.step();
        highest = highest.min(e.balls[0].pos.y);
    }

    // In VP `y` grows towards the player, so going up means the y **goes down**.
    let climbed = before.y - highest;
    eprintln!("the flipper sent it {climbed:.0} table units up");
    assert!(
        climbed > 300.0,
        "the flipper should have sent it well up the table; it only climbed {climbed:.0}"
    );
    assert!(
        e.flipper(index).unwrap().angle_cur != e.flipper(index).unwrap().angle_start,
        "and the flipper should have moved"
    );
}

#[test]
fn an_f14_trigger_reports_when_the_ball_goes_over_it() {
    // Almost all of a table's logic hangs off the triggers: counting laps,
    // lighting targets, starting a multiball. If they do not report, there is no
    // game.
    //
    // One is targeted deliberately. Dropping balls at random and hoping one goes
    // over would be a test that passes by luck: of F-14's thirteen, seven are up
    // on ramps and a ball rolling across the playfield never touches them.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let triggers = vpw_table::physics::triggers(&vpx);
    assert!(!triggers.is_empty(), "F-14 has triggers");

    // One at playfield level, which is where the ball is going to be.
    let (target, bbox) = triggers
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.bbox()))
        .find(|(_, c)| c.min.z <= 1.0)
        .expect("F-14 has triggers at playfield level");

    let center = (bbox.min + bbox.max) * 0.5;
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut e = Engine::new(vpw_table::physics::build(&vpx), gravity);
    e.slope_rad = g.angle_tilt_max.to_radians();
    for t in triggers {
        e.add_trigger(t);
    }

    // Dropped right above it, a bit higher, so it falls inside.
    e.add_ball(Ball::new(
        Vec3::new(center.x, center.y - 120.0, bbox.max.z + 60.0),
        DEFAULT_BALL_SIZE,
    ));

    let mut crossings = Vec::new();
    for _ in 0..6_000 {
        e.step();
        for ev in e.take_events() {
            // Nothing here asks for hit reports, so a trigger crossing is
            // the only kind of event that can turn up.
            let Event::Trigger {
                trigger, crossing, ..
            } = ev
            else {
                continue;
            };
            if trigger == target {
                crossings.push(crossing);
            }
        }
    }

    eprintln!("F-14's trigger {target} recorded {crossings:?}");
    assert_eq!(
        crossings.first(),
        Some(&Crossing::Entered),
        "the ball went over it and it did not report"
    );
}

#[test]
fn f14s_triggers_never_report_the_same_thing_twice() {
    // The invariant that matters: entering and leaving have to alternate. Two
    // entries in a row for the same ball would be one report too many, and in
    // the game that is a point counted twice.
    //
    // It is also what verifies that none of the three patches the original has
    // to fix its out-of-sync state are needed: here the state is recomputed on
    // every step, so it cannot end up wrong.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut reported = std::collections::HashSet::new();
    let mut total = 0usize;

    for i in 0..8 {
        let mut e = Engine::new(vpw_table::physics::build(&vpx), gravity);
        e.slope_rad = g.angle_tilt_max.to_radians();
        for t in vpw_table::physics::triggers(&vpx) {
            e.add_trigger(t);
        }

        let x = g.left + (g.right - g.left) * (0.15 + 0.7 * i as f32 / 7.0);
        e.add_ball(Ball::new(
            Vec3::new(x, g.top + (g.bottom - g.top) * 0.1, 80.0),
            DEFAULT_BALL_SIZE,
        ));

        let mut inside = vec![false; e.triggers().len()];
        for _ in 0..10_000 {
            e.step();
            for ev in e.take_events() {
                let Event::Trigger {
                    trigger,
                    ball,
                    crossing,
                } = ev
                else {
                    continue;
                };
                assert_eq!(ball, 0, "there is only one ball");
                match crossing {
                    Crossing::Entered => {
                        assert!(
                            !inside[trigger],
                            "trigger {trigger} reported two entries in a row"
                        );
                        inside[trigger] = true;
                        reported.insert(trigger);
                        total += 1;
                    }
                    Crossing::Exited => {
                        assert!(
                            inside[trigger],
                            "trigger {trigger} reported an exit without an entry"
                        );
                        inside[trigger] = false;
                    }
                }
            }
        }
    }

    eprintln!(
        "{} distinct triggers reported, {total} crossings",
        reported.len()
    );
}

#[test]
fn f14s_triggers_get_drawn() {
    // Until now the triggers existed for the physics but not for the eye.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let scene = vpw_table::geometry::extract(&vpx);
    let names: Vec<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|i| match i {
            GameItemEnum::Trigger(t) if t.is_visible => Some(t.name.as_str()),
            _ => None,
        })
        .collect();

    let drawn = names
        .iter()
        .filter(|n| scene.meshes.iter().any(|m| m.name == **n))
        .count();

    eprintln!("{drawn} of {} visible triggers have a mesh", names.len());
    assert!(drawn > 0, "no F-14 trigger generated a mesh");

    // And every mesh has to have real triangles.
    for m in scene
        .meshes
        .iter()
        .filter(|m| names.contains(&m.name.as_str()))
    {
        assert!(m.triangles() > 0, "trigger {} came out empty", m.name);
    }
}

#[test]
fn a_multiball_on_a_real_table_does_not_fall_apart() {
    // Six balls at once on F-14. It is what really exercises ball-to-ball
    // collision: two balls colliding near a wall is the case where the order
    // things get resolved in matters, and where a ball pushed by another one can
    // end up on the wrong side of the geometry.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);
    let mut e = Engine::new(vpw_table::physics::build(&vpx), gravity);
    e.slope_rad = g.angle_tilt_max.to_radians();

    const BALLS: usize = 6;
    for i in 0..BALLS {
        let x = g.left + (g.right - g.left) * (0.25 + 0.5 * i as f32 / (BALLS - 1) as f32);
        let y = g.top + (g.bottom - g.top) * (0.15 + 0.1 * (i % 2) as f32);
        e.add_ball(Ball::new(Vec3::new(x, y, 80.0), DEFAULT_BALL_SIZE));
    }

    let mut worst_overlap: f32 = 0.0;
    for step in 0..10_000 {
        e.step();

        for (n, b) in e.balls.iter().enumerate() {
            let p = b.pos;
            assert!(
                p.x > g.left - 200.0
                    && p.x < g.right + 200.0
                    && p.y > g.top - 200.0
                    && p.y < g.bottom + 400.0
                    && p.z > -200.0,
                "ball {n} escaped at step {step}: {p:?}"
            );
        }

        for i in 0..BALLS {
            for j in (i + 1)..BALLS {
                let d = (e.balls[i].pos - e.balls[j].pos).length();
                worst_overlap = worst_overlap.max(2.0 * DEFAULT_BALL_SIZE - d);
            }
        }
    }

    // No ball can end up half a ball inside another one. There is always some
    // overlap — the engine resolves at the instant of contact and then
    // integrates — but it has to be small.
    eprintln!("worst overlap between balls: {worst_overlap:.1} units");
    assert!(
        worst_overlap < DEFAULT_BALL_SIZE * 0.5,
        "they ended up too far into each other: {worst_overlap:.1} out of {DEFAULT_BALL_SIZE} of radius"
    );
}

#[test]
fn no_ball_escapes_from_a_real_table() {
    // **The** test of this module. Balls are dropped all over the table and it is
    // checked that none of them goes through the floor or the walls. A single
    // hole in the collision geometry — an inverted normal, a missing joint —
    // shows up here.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let shapes = vpw_table::physics::build(&vpx);
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);

    let mut dropped = 0;
    let mut arrived = 0;

    for i in 0..6 {
        for j in 0..6 {
            let x = g.left + (g.right - g.left) * (0.15 + 0.7 * i as f32 / 5.0);
            let y = g.top + (g.bottom - g.top) * (0.10 + 0.5 * j as f32 / 5.0);

            let mut e = Engine::new(shapes.clone(), gravity);
            e.slope_rad = g.angle_tilt_max.to_radians();
            e.add_ball(Ball::new(Vec3::new(x, y, 60.0), DEFAULT_BALL_SIZE));
            dropped += 1;

            for step in 0..10_000 {
                e.step();
                let p = e.balls[0].pos;
                assert!(
                    p.x > g.left - 200.0
                        && p.x < g.right + 200.0
                        && p.y > g.top - 200.0
                        && p.y < g.bottom + 400.0
                        && p.z > -200.0,
                    "the ball dropped at ({x:.0}, {y:.0}) escaped at step {step}: {p:?}"
                );
            }

            // Down at the drain, or come to rest up on a ramp. The second is
            // not a failure: a ramp is a surface, and a ball that lands on one
            // stays there. Before ramps had any collision every ball fell
            // through them to the playfield, which is why this used to be a
            // simpler question.
            let p = e.balls[0].pos;
            let resting = e.balls[0].vel.length() < 0.5;
            if p.y > g.bottom * 0.8 || (p.z > 45.0 && resting) {
                arrived += 1;
            } else if std::env::var("VPW_WHERE").is_ok() {
                eprintln!(
                    "  dropped at ({x:.0},{y:.0}) ended at ({:.0},{:.0},{:.0}) |v|={:.2}",
                    p.x,
                    p.y,
                    p.z,
                    e.balls[0].vel.length()
                );
            }
        }
    }

    // Most of them have to end up somewhere a ball can be. The ones that do not
    // get stuck against some wall: F-14 has guides drawn as outlines that fold
    // back on themselves, thinner than the ball itself, and a ball that ends up
    // inside one of those does not know how to get out. It is a known problem
    // and it is written down; the threshold is set so that **making it worse**
    // breaks the test.
    //
    // It moved from 60% to 55% when ramps got collision, and that is the number
    // getting more honest rather than worse: a ball dropped from above used to
    // fall straight through every ramp on the table and roll to the drain.
    // Now it lands on them, rolls off the side of them, and sometimes ends up
    // underneath one. Five of the misses come to rest at y≈1708 in the left
    // outlane — a hair above the line drawn here, and nobody's idea of stuck.
    let percentage = arrived * 100 / dropped;
    eprintln!("{arrived} of {dropped} balls ended up somewhere sensible ({percentage}%)");
    assert!(
        percentage >= 55,
        "only {percentage}% ended up somewhere sensible; something broke"
    );
}

#[test]
fn the_physics_of_a_real_table_runs_much_faster_than_real_time() {
    // It is the point of the port. With 3000 shapes and the broadphase, one
    // second of table has to cost far less than one second of CPU — otherwise
    // there is no headroom for the rest of the frame on a modest device.
    //
    // The number it prints is a **floor**, not the exact measurement: cargo runs
    // the tests in parallel and the other two in this file each simulate
    // thirty-six balls, so this one competes with them for CPU. On its own it
    // gives close to twice that.
    //
    // Even so the threshold has to bite. One version of this module walked the
    // table's three thousand shapes on every sub-cycle just to move eight
    // rotating pieces, and cost twelve times more; with the old threshold of 20x
    // it passed without complaining. The current one leaves a factor of five of
    // margin for slower machines and still detects a regression of that size.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };
    let g = &vpx.gamedata;
    let shapes = vpw_table::physics::build(&vpx);
    let gravity = Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY);

    let start = Vec3::new((g.left + g.right) * 0.5, (g.top + g.bottom) * 0.3, 100.0);

    // The **best** of several runs, not the average and not one run.
    //
    // This measures what the machine can do, and a machine that is also doing
    // something else answers a different question. Under a parallel build the
    // same loop here drops from 1200x to 230x, and on a shared CI runner it
    // would do worse — so a single sample makes this a test that fails for
    // reasons that have nothing to do with the physics, which is the kind that
    // teaches people to ignore a red build.
    //
    // The fastest sample is the honest one: nothing can make the loop run
    // faster than it is able to, so a real regression still shows up in it.
    let steps = 10_000; // ten seconds of table
    let simulated = steps as f64 / 1000.0;

    // Measured against a control, and **paired in time with it**.
    //
    // An absolute threshold measures the machine and not the code, and on a
    // phone that is not a small effect: the same binary answers anywhere from
    // 129x to 942x here depending on what the governor feels like, so a fixed
    // floor fails about a third of the time for reasons that have nothing to
    // do with the physics. That is the kind of red build people learn to
    // ignore, which is worse than no test at all.
    //
    // So what has to hold is a **ratio**: how much table time one unit of
    // arithmetic buys. The two have to be measured next to each other, not one
    // after all the others — a control taken at the end samples whatever the
    // governor is doing by then, which is how a first attempt at this still
    // spread three to one. Timed back to back inside the same round, a machine
    // that is slow is slow for both and it cancels.
    //
    // Best of five, for the reason above: nothing can make either loop run
    // faster than it is able to, so the fastest round is the least polluted
    // one and a real regression still shows up in it.
    let mut best: f64 = 0.0;
    for _ in 0..5 {
        let at = std::time::Instant::now();
        let mut acc = 0u64;
        for i in 0..20_000_000u64 {
            acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i);
        }
        std::hint::black_box(acc);
        let control = at.elapsed().as_secs_f64();

        // A fresh ball each round, or the later ones are timing a ball that has
        // already come to rest and a cheaper loop than the first. The engine is
        // rebuilt with it: the shapes are cloned rather than read again, so this
        // is the ball's cost and not the parser's.
        let mut e = Engine::new(shapes.clone(), gravity);
        e.slope_rad = g.angle_tilt_max.to_radians();
        e.add_ball(Ball::new(start, DEFAULT_BALL_SIZE));

        let at = std::time::Instant::now();
        for _ in 0..steps {
            e.step();
        }
        let real = at.elapsed().as_secs_f64();
        best = best.max(simulated / real.max(1e-9) * control);
    }

    eprintln!("{simulated}s of table: {best:.0}x real time per control second");
    // A debug build is not the thing being measured. Its physics and its
    // control loop slow down by different factors — bounds checks and no
    // inlining land on the one and barely touch the other — so the ratio
    // means nothing there and the same code reads 8 on one run and 14 on the
    // next. The floor below is calibrated on release, where an interleaved
    // A/B of a change that halved the ramp triangles read 20 to 54 against
    // 20 to 21 for the old code, and it is only asked of release.
    if cfg!(debug_assertions) {
        eprintln!("debug build: reported, not asserted");
        return;
    }
    // The regression this was written for — a version that walked the table's
    // three thousand shapes on every sub-cycle to move eight rotating pieces —
    // cost twelve times more. Half of the lowest reading seen on a good machine
    // still catches that with room to spare.
    assert!(
        best > 12.0,
        "only {best:.1}x real time per control second; the loop got more \
         expensive relative to this machine"
    );
}

/// How many plain line segments the ramps contribute.
///
/// Counted rather than predicted: the original subdivides a ramp's wall
/// wherever it climbs faster than the collision skin, so the number depends on
/// the shape of the ramp and not on anything you could work out by hand.
fn ramp_line_count(vpx: &vpin::vpx::VPX) -> usize {
    use vpin::vpx::gameitem::GameItemEnum;
    let collision = vpw_table::physics::build_with_owners(vpx);
    collision
        .shapes
        .iter()
        .zip(&collision.owners)
        .filter(|(shape, owner)| {
            matches!(shape, Shape::Line(_))
                && owner
                    .is_some_and(|i| matches!(vpx.gameitems.get(i), Some(GameItemEnum::Ramp(_))))
        })
        .count()
}

#[test]
fn a_rubber_is_a_tube_and_not_a_line() {
    // `Rubber::PhysicSetup` (`rubber.cpp:528`) sweeps a hexagon of radius
    // thickness/2 along the spline and hands the physics its triangles, its
    // edges and its vertices. Two things follow that a ring of segments on the
    // centreline does not have, and both are what a rubber is for:
    //
    // It stands off the centreline, so a ball is turned away half a rubber
    // earlier than it would be by a line through the middle of one. And it is
    // closed, so it stops a ball from inside its own loop as well as outside —
    // which is the side a ball that has got somewhere it should not be is
    // trying to come back through.
    //
    // The z band is the tell that is easiest to get wrong: it is centred on the
    // hit height, not hung below it. Off by half a thickness in the same
    // direction every time looks entirely plausible on screen.
    let Some(vpx) = table() else { return };
    let collision = vpw_table::physics::build_with_owners(&vpx);

    let mut checked = 0;
    for (index, item) in vpx.gameitems.iter().enumerate() {
        let GameItemEnum::Rubber(r) = item else {
            continue;
        };
        if !r.is_collidable {
            continue;
        }
        let owned: Vec<usize> = collision.shapes_of(index).collect();
        if owned.is_empty() {
            continue;
        }
        checked += 1;

        let faces = owned
            .iter()
            .filter(|i| matches!(collision.shapes[**i], Shape::Triangle(_)))
            .count();
        assert!(
            faces > 0,
            "{} is made of {} shapes and not one of them is a face",
            r.name,
            owned.len()
        );

        // Every shape of it lies inside the tube's own box.
        let height = r.hit_height.unwrap_or(r.height);
        let half = r.thickness as f32 * 0.5;
        let (mut lowest, mut highest) = (f32::MAX, f32::MIN);
        for i in &owned {
            if let Some(b) = collision.shapes[*i].bbox() {
                lowest = lowest.min(b.min.z);
                highest = highest.max(b.max.z);
            }
        }
        // Centred on the hit height, and no wider than the tube. It is not
        // exactly a full thickness tall: `rubber.cpp:1299` squashes two of the
        // six vertices to sixty per cent so the collision section comes out
        // more of a rectangle than a circle, and those two sit at the extremes
        // of z. Being *centred* is the part that matters — the mistake this
        // guards against hung the whole band below the hit height, which looks
        // entirely plausible until a ball goes under a rubber.
        let centre = (lowest + highest) * 0.5;
        assert!(
            (centre - height).abs() < 0.5,
            "{} spans z {lowest:.1}..{highest:.1}, centred on {centre:.1}, and its hit height \
             is {height}: a tube hangs around that height, it does not sit above it",
            r.name
        );
        assert!(
            highest > lowest && highest - lowest <= half * 2.0 + 0.5,
            "{} is {:.1} tall and a tube of thickness {} is at most {:.1}",
            r.name,
            highest - lowest,
            r.thickness,
            half * 2.0
        );
    }
    assert!(checked > 0, "the table has no collidable rubber to check");
}

#[test]
fn a_ramps_right_edge_is_the_one_the_original_calls_right() {
    // The original offsets its first half of vertices along `+vnormal`
    // (`ramp.cpp:484`) and then calls that half the right wall
    // (`ramp.cpp:585`); the `-vnormal` half is the left (`ramp.cpp:605`).
    //
    // Naming them the other way round is invisible on a ramp walled the same
    // on both sides — which is most of them — and puts the wall on the wrong
    // side of every ramp that is not. F-14 has one: a ramp with a 62-unit
    // guide down one side and nothing down the other, and the guide was
    // standing on the wrong edge.
    //
    // `vnormal` is worked out inside the module, so what is checked here is its
    // sign, which is all a swap can change: rotating the direction of travel a
    // quarter turn the way `normal_at` does, the right edge is the one that
    // lands on the positive side of it.
    let Some(vpx) = table() else { return };

    let mut checked = 0;
    for item in &vpx.gameitems {
        let GameItemEnum::Ramp(r) = item else {
            continue;
        };
        if !r.is_collidable {
            continue;
        }
        let Some(c) = vpw_table::ramp::path(r, vpw_table::dragpoint::collision_accuracy()) else {
            continue;
        };
        if c.center.len() < 3 {
            continue;
        }
        for i in 1..c.center.len() {
            let (prev, mid) = (c.center[i - 1], c.center[i]);
            if (mid - prev).length() < 1.0 {
                continue; // too short to give a direction
            }
            // `normal_at`'s first term: the direction of travel, turned.
            let normal = vpw_math::Vec2::new(prev.y - mid.y, mid.x - prev.x).normalize();
            let side = (c.right[i] - mid).dot(normal);
            assert!(
                side > 0.0,
                "{}: its right edge is {side:.1} along the normal at point {i}, and the \
                 original's right edge is the positive side",
                r.name
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no ramp on the table had an edge to check");
}

#[test]
fn a_ramp_is_as_high_as_wherever_you_ask_it() {
    // A wall has one height and the answer does not depend on where you ask.
    // A ramp **climbs**, so "how high is the ramp" only has an answer at a
    // point, and the original forwards the question to `Ramp::GetSurfaceHeight`
    // for exactly that reason (`pintable.cpp:5171`).
    //
    // F-14's popper `sw24a` is what made this visible. It is mounted on
    // `Ramp4`, which runs from 150 at the popper end down to 112 at the other.
    // Answering with the ramp's `heighttop` put the ball the popper creates 36
    // units under the ramp's own floor, and the first step shoved it off the
    // ramp altogether.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };

    let ramp = vpx
        .gameitems
        .iter()
        .find_map(|i| match i {
            GameItemEnum::Ramp(r) if r.name.eq_ignore_ascii_case("Ramp4") => Some(r),
            _ => None,
        })
        .expect("F-14 has a Ramp4");
    assert_eq!(ramp.height_bottom, 150.0, "it starts high");
    assert_eq!(ramp.height_top, 112.0, "and comes down");

    let at = |x: f32, y: f32| vpw_table::builtin::surface_height(&vpx.gameitems, "Ramp4", x, y);

    // At the popper, which is where the ramp begins.
    let popper = at(871.4, 84.3);
    assert!(
        (popper - 148.0).abs() < 3.0,
        "at the popper the ramp is near its bottom end, 150: it says {popper:.1}"
    );

    // And somewhere further along it is genuinely lower. Not a fixed number —
    // the claim is that the answer *moves*, which is the whole point.
    let along = at(465.0, 68.0);
    assert!(
        along < popper - 5.0,
        "further down the ramp should be lower: {along:.1} against {popper:.1} at the popper"
    );
    assert!(
        (112.0..=150.0).contains(&along),
        "and still between the ramp's two ends: {along:.1}"
    );
}

#[test]
fn the_popper_puts_the_ball_on_its_ramp_and_not_in_a_corner() {
    // The end of the same story, on the real geometry. `SolPopper` creates a
    // ball in `sw24a` and `cvpmBallStack` kicks it with `InitKick sw24a, 270,
    // 28` — straight along -x, onto the habitrail that carries it across the
    // top of the table.
    //
    // With the ramp's height read at the wrong end the ball was born inside the
    // ramp floor, got thrown off it within twenty milliseconds, fell to the
    // playfield at the very top of the table and rolled into the wedge between
    // `Wall14` and the end of `Wall19` at (809, 96), where it stopped for good.
    // Telemetry from a real game shows it sitting there for the last nine and a
    // half seconds of the recording.
    let Some(vpx) = table() else {
        eprintln!("skipped: web/debug-assets/f14.vpx is missing");
        return;
    };

    let kicker = vpx
        .gameitems
        .iter()
        .find_map(|i| match i {
            GameItemEnum::Kicker(k) if k.name.eq_ignore_ascii_case("sw24a") => Some(k),
            _ => None,
        })
        .expect("F-14 has an sw24a");

    let surface = vpw_table::builtin::surface_height(
        &vpx.gameitems,
        &kicker.surface,
        kicker.center.x,
        kicker.center.y,
    );

    let g = &vpx.gamedata;
    let mut engine = Engine::new(
        vpw_table::physics::build(&vpx),
        Engine::gravity_from_slope(g.angle_tilt_max, DEFAULT_TABLE_GRAVITY),
    );
    engine.slope_rad = g.angle_tilt_max.to_radians();

    // Served and kicked the way the table does it, rather than dropped in with
    // a velocity. It matters now that a kicker knows which balls are inside it:
    // `CreateBall` puts the ball in the hole **and into the hole's volume set**
    // (`kicker.cpp:1153`, reached with `newBall` true), and `Kick` leaves it in
    // that set on the way out. A ball merely placed at the middle of a kicker
    // is one the hole has not dealt with, and it gets taken on the next step —
    // which is what the original does too (`collide.cpp:277-287`) and what this
    // test used to hide by never going through the serve.
    let shape = engine
        .shapes()
        .iter()
        .position(|s| {
            matches!(s, vpw_physics::engine::Shape::Kicker(k)
                if (k.circle.center.x - kicker.center.x).abs() < 0.01
                    && (k.circle.center.y - kicker.center.y).abs() < 0.01)
        })
        .expect("sw24a builds a kicker");

    let start = Vec3::new(
        kicker.center.x,
        kicker.center.y,
        surface + DEFAULT_BALL_SIZE,
    );
    let ball = engine.add_ball(Ball::new(start, DEFAULT_BALL_SIZE));
    engine.capture_ball(shape, ball);
    // `Kick 270, 28`: the angle is degrees from straight up the table, turning
    // clockwise, so the velocity is `(sin a, -cos a)` (`kicker.cpp:749`).
    let a: f32 = 270f32.to_radians();
    engine.release_from_kicker(shape, Vec3::new(a.sin(), -a.cos(), 0.0) * 28.0);

    for _ in 0..6000 {
        engine.step();
    }
    let end = engine.balls[0].pos;

    // The wedge it used to die in.
    let wedge = Vec3::new(808.9, 95.9, 25.0);
    assert!(
        (end - wedge).length() > 100.0,
        "six seconds on and it is back in the corner at ({:.1}, {:.1}, {:.1})",
        end.x,
        end.y,
        end.z
    );

    // And it actually went somewhere: down the ramp and across the table, not
    // a few units off the kicker.
    let travelled = (end - start).length();
    assert!(
        travelled > 500.0,
        "it should have ridden the ramp a long way; it moved {travelled:.0} units to ({:.1}, {:.1}, {:.1})",
        end.x,
        end.y,
        end.z
    );
}

/// A habitrail's channel is twenty units wider than its wires are apart.
///
/// `ramp.cpp:471-474`: for a wire ramp the collision outline is
/// `m_wireDistanceX + 20`, while the tubes are drawn at `m_wireDistanceX`.
/// That twenty is the difference between a channel a ball runs down and one
/// it wedges in: the wires on the table this was found on are thirty-eight
/// apart and a ball is fifty across.
///
/// Built by hand rather than read from F-14, which has no wire ramps.
#[test]
fn a_wire_ramps_channel_is_twenty_wider_than_its_wires() {
    use vpin::vpx::gameitem::dragpoint::DragPoint;
    use vpin::vpx::gameitem::ramp::{Ramp, RampType};

    let point = |x: f32, y: f32| DragPoint {
        x,
        y,
        ..DragPoint::default()
    };
    let ramp = |kind: RampType| Ramp {
        ramp_type: kind,
        wire_distance_x: 38.0,
        wire_diameter: 6.0,
        width_bottom: 55.0,
        width_top: 55.0,
        drag_points: vec![point(100.0, 1000.0), point(100.0, 500.0)],
        ..Ramp::default()
    };
    let widths = |r: &Ramp| {
        let seen = vpw_table::ramp::path(r, 4.0).expect("a path to draw");
        let met = vpw_table::ramp::collision_path(r, 4.0).expect("a path to run on");
        (
            (seen.right[0] - seen.left[0]).length(),
            (met.right[0] - met.left[0]).length(),
        )
    };

    let (drawn, channel) = widths(&ramp(RampType::TwoWire));
    assert!(
        (drawn - 38.0).abs() < 1e-3,
        "the tubes are drawn at the wire spacing, not {drawn}"
    );
    assert!(
        (channel - 58.0).abs() < 1e-3,
        "the channel is the spacing plus twenty, not {channel}"
    );

    // The two kinds the original leaves alone.
    let (drawn, channel) = widths(&ramp(RampType::OneWire));
    assert!(
        (channel - drawn).abs() < 1e-3,
        "a single wire gets no extra: {drawn} vs {channel}"
    );
    let (drawn, channel) = widths(&ramp(RampType::Flat));
    assert!(
        (channel - drawn).abs() < 1e-3,
        "a flat ramp gets no extra: {drawn} vs {channel}"
    );
}
