//! The bridge between the physics and the matrices that draw it.
//!
//! What is checked here is that the state the engine computes actually reaches
//! the transform of each piece — that pressing a button moves a flipper's
//! vertices, that pulling the plunger stretches its shaft, that a ball crossing
//! a trigger sinks the wire.
//!
//! It is built out of hand-made shapes instead of a real `.vpx` on purpose: a
//! table is a hundred megabytes and is not in the repo, and a flipper that
//! rotates is a flipper that rotates whatever file it came from.

use vpw_math::{Mat4, Vec2, Vec3};
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY};
use vpw_physics::engine::{Engine, Shape};
use vpw_physics::flipper::{Flipper, FlipperParams};
use vpw_physics::parts::{Bumper, PivotAxis, Spinner};
use vpw_physics::plunger::{Plunger, PlungerParams};
use vpw_physics::shapes::HitCircle;
use vpw_physics::trigger::{Trigger, Zone};
use vpw_table::animation::{AnimatedPart, Animation};

/// Where a piece's transform sends a given local point.
fn place(part: &AnimatedPart, engine: &Engine, local: Vec3) -> Vec3 {
    part.transform(engine).transform_point3(local)
}

fn engine_with(shapes: Vec<Shape>) -> Engine {
    let mut e = Engine::new(
        shapes,
        Engine::gravity_from_slope(6.0, DEFAULT_TABLE_GRAVITY),
    );
    e.slope_rad = 6.0f32.to_radians();
    e
}

// --------------------------------------------------------------- flipper ---

fn flipper_part() -> (Engine, AnimatedPart) {
    let engine = engine_with(vec![Shape::Flipper(Flipper::new(FlipperParams {
        center: Vec2::new(300.0, 1800.0),
        angle_start: 0.0,
        angle_end: (-60.0f32).to_radians(),
        ..FlipperParams::default()
    }))]);

    // The same decomposition `animation::flippers` builds: the placement goes
    // in `base`, the height scale and the half-turn in `local`, and the angle
    // is what sits in between.
    let base = Mat4::from_translation(Vec3::new(300.0, 1800.0, 0.0));
    let local =
        Mat4::from_scale(Vec3::new(1.0, 1.0, 50.0)) * Mat4::from_rotation_z(std::f32::consts::PI);
    let part = AnimatedPart {
        mesh: vpw_table::ball::mesh(),
        base,
        local,
        anim: Animation::Flipper { shape: 0 },
    };
    (engine, part)
}

#[test]
fn a_flipper_that_is_not_pressed_does_not_move() {
    let (mut engine, part) = flipper_part();
    let before = place(&part, &engine, Vec3::Y);
    for _ in 0..50 {
        engine.step();
    }
    let after = place(&part, &engine, Vec3::Y);
    assert!((before - after).length() < 1e-3, "{before:?} vs {after:?}");
}

#[test]
fn pressing_the_button_rotates_the_flipper_mesh() {
    let (mut engine, part) = flipper_part();
    let at_rest = place(&part, &engine, Vec3::Y);

    engine.set_flipper_solenoid(0, true);
    for _ in 0..60 {
        engine.step();
    }
    let raised = place(&part, &engine, Vec3::Y);

    assert!(
        (at_rest - raised).length() > 0.05,
        "the flipper's angle never reached its matrix: {at_rest:?} vs {raised:?}"
    );

    // And it goes back down when the button is released.
    engine.set_flipper_solenoid(0, false);
    for _ in 0..600 {
        engine.step();
    }
    let back = place(&part, &engine, Vec3::Y);
    assert!(
        (at_rest - back).length() < (at_rest - raised).length(),
        "the flipper did not come back down"
    );
}

#[test]
fn the_flipper_turns_around_its_own_pivot() {
    // Whatever the angle, the point on the axis stays put. If `base` and
    // `local` were the wrong way round it would orbit instead.
    let (mut engine, part) = flipper_part();
    let pivot_at_rest = place(&part, &engine, Vec3::ZERO);

    engine.set_flipper_solenoid(0, true);
    for _ in 0..60 {
        engine.step();
    }
    let pivot_raised = place(&part, &engine, Vec3::ZERO);

    assert!(
        (pivot_at_rest - pivot_raised).length() < 1e-3,
        "the pivot moved: {pivot_at_rest:?} vs {pivot_raised:?}"
    );
    assert!((pivot_at_rest - Vec3::new(300.0, 1800.0, 0.0)).length() < 1e-3);
}

// -------------------------------------------------------------- plunger ---

/// A plunger like a real table's: 80 units of travel, parked at the back.
fn plunger_parts() -> (Engine, AnimatedPart, AnimatedPart) {
    let center_y = 2000.0f32;
    let stroke = 80.0f32;
    let height = 20.0f32;
    let width = 25.0f32;

    let engine = engine_with(vec![Shape::Plunger(Plunger::new(PlungerParams {
        x: -width,
        x2: width,
        y: center_y + height,
        z_low: 0.0,
        frame_end: center_y - stroke,
        frame_start: center_y,
        rest_pos: 0.5,
        speed_pull: 0.5,
        speed_fire: 80.0,
        scatter_velocity: 0.0,
        momentum_xfer: 1.0,
        auto_plunger: false,
        material: Default::default(),
    }))]);

    let base = Mat4::from_translation(Vec3::new(0.0, 0.0, width));
    let head = AnimatedPart {
        mesh: vpw_table::plunger::head_mesh("P", width, String::new()),
        base,
        local: Mat4::IDENTITY,
        anim: Animation::PlungerHead { shape: 0 },
    };
    let shaft = AnimatedPart {
        mesh: vpw_table::plunger::shaft_mesh("P (shaft)", width, String::new()),
        base,
        local: Mat4::IDENTITY,
        anim: Animation::PlungerShaft {
            shape: 0,
            shoulder_y: vpw_table::plunger::SHOULDER_Y,
            back_y: center_y + height,
        },
    };
    (engine, head, shaft)
}

#[test]
fn the_plunger_head_sits_where_the_physics_says() {
    let (engine, head, _) = plunger_parts();
    let Some(Shape::Plunger(p)) = engine.shapes().first() else {
        unreachable!()
    };
    // The tip of the mesh is the local origin.
    let tip = place(&head, &engine, Vec3::ZERO);
    assert!(
        (tip.y - p.pos).abs() < 1e-3,
        "tip at {} for pos {}",
        tip.y,
        p.pos
    );
}

#[test]
fn pulling_the_plunger_moves_the_head_backwards() {
    let (mut engine, head, _) = plunger_parts();
    let before = place(&head, &engine, Vec3::ZERO).y;

    engine.pull_plunger(0);
    for _ in 0..200 {
        engine.step();
    }
    let after = place(&head, &engine, Vec3::ZERO).y;

    // In Visual Pinball `y` grows towards the player, so pulling it back means
    // a bigger `y`.
    assert!(
        after > before + 1.0,
        "the plunger did not come back: {before} -> {after}"
    );
}

#[test]
fn the_shaft_stretches_instead_of_sliding() {
    // This is the whole reason the rod is two pieces. The back of the shaft is
    // pinned to the barrel; only its length changes.
    let (mut engine, _, shaft) = plunger_parts();

    let back = |engine: &Engine| place(&shaft, engine, Vec3::Y).y;
    let front = |engine: &Engine| place(&shaft, engine, Vec3::ZERO).y;

    let back_before = back(&engine);
    let front_before = front(&engine);

    engine.pull_plunger(0);
    for _ in 0..200 {
        engine.step();
    }

    assert!(
        (back(&engine) - back_before).abs() < 1e-3,
        "the back of the shaft moved: {back_before} -> {}",
        back(&engine)
    );
    let stretched = back(&engine) - front(&engine);
    let original = back_before - front_before;
    assert!(
        stretched < original - 1.0,
        "the shaft did not change length: {original} -> {stretched}"
    );
}

#[test]
fn the_shaft_never_turns_inside_out() {
    // A plunger driven past its own barrel would give the scale a negative
    // length and flip every normal on the rod.
    let (engine, _, shaft) = plunger_parts();
    let broken = AnimatedPart {
        anim: Animation::PlungerShaft {
            shape: 0,
            shoulder_y: vpw_table::plunger::SHOULDER_Y,
            // A back end in front of the shoulder: impossible, but a table with
            // a negative `height` would produce it.
            back_y: -10_000.0,
        },
        ..shaft
    };
    let length = place(&broken, &engine, Vec3::Y).y - place(&broken, &engine, Vec3::ZERO).y;
    assert!(length > 0.0, "the shaft came out inverted: {length}");
}

// -------------------------------------------------------------- trigger ---

#[test]
fn a_ball_over_a_trigger_sinks_the_wire() {
    let mut engine = engine_with(Vec::new());
    engine.add_trigger(
        Trigger::new(
            Zone::Circle {
                center: Vec2::new(0.0, 0.0),
                radius: 100.0,
            },
            0.0,
            60.0,
        )
        .with_animation(32.0, 0.4),
    );
    let part = AnimatedPart {
        mesh: vpw_table::ball::mesh(),
        base: Mat4::IDENTITY,
        local: Mat4::IDENTITY,
        anim: Animation::Trigger { trigger: 0 },
    };

    let at_rest = place(&part, &engine, Vec3::ZERO);

    // A ball parked right on top of it. It is `locked` so gravity leaves it
    // alone: what is being tested is the animation, not the fall.
    let mut ball = Ball::new(Vec3::new(0.0, 0.0, 25.0), DEFAULT_BALL_SIZE);
    ball.locked = true;
    engine.add_ball(ball);

    for _ in 0..200 {
        engine.step();
    }
    let sunk = place(&part, &engine, Vec3::ZERO);

    assert!(
        sunk.z < at_rest.z - 1.0,
        "the wire did not sink: {} -> {}",
        at_rest.z,
        sunk.z
    );
    assert!(
        sunk.z >= at_rest.z - 32.0 - 1e-3,
        "it sank past its own limit: {}",
        sunk.z
    );
}

// --------------------------------------------------------- bumper, spinner ---

#[test]
fn a_bumper_that_fires_drops_its_ring() {
    let mut bumper = Bumper::new(
        HitCircle {
            center: Vec2::ZERO,
            radius: 45.0,
            z_low: 0.0,
            z_high: 50.0,
            material: Default::default(),
        },
        1.0,
        1.0,
    );
    bumper.ring_drop = 25.0;
    bumper.ring_speed = 0.5;
    // What `Bumper::collide` sets when the hit is hard enough.
    bumper.hit = true;

    let mut engine = engine_with(vec![Shape::Bumper(bumper)]);
    let part = AnimatedPart {
        mesh: vpw_table::ball::mesh(),
        base: Mat4::IDENTITY,
        local: Mat4::IDENTITY,
        anim: Animation::BumperRing { shape: 0 },
    };

    let at_rest = place(&part, &engine, Vec3::ZERO);
    for _ in 0..20 {
        engine.step();
    }
    let dropped = place(&part, &engine, Vec3::ZERO);
    assert!(dropped.z < at_rest.z - 1.0, "the ring did not drop");

    // And it comes back on its own, without anyone telling it to.
    for _ in 0..200 {
        engine.step();
    }
    let back = place(&part, &engine, Vec3::ZERO);
    assert!(
        (back.z - at_rest.z).abs() < 1e-3,
        "the ring did not come back: {} vs {}",
        at_rest.z,
        back.z
    );
}

#[test]
fn a_spinner_that_is_spinning_turns_its_plate() {
    let mut spinner = Spinner::new(
        PivotAxis {
            center: Vec2::ZERO,
            length: 80.0,
            rotation_deg: 0.0,
            height: 30.0,
            z_low: 0.0,
            angle_min: 0.0,
            angle_max: 0.0,
            damping: 0.99,
        },
        0.3,
    );
    spinner.angle_speed = 0.5;

    let mut engine = engine_with(vec![Shape::Spinner(spinner)]);
    let part = AnimatedPart {
        mesh: vpw_table::ball::mesh(),
        base: Mat4::IDENTITY,
        local: Mat4::IDENTITY,
        anim: Animation::Spinner { shape: 0 },
    };

    // A point off the axis, so a rotation actually shows.
    let before = place(&part, &engine, Vec3::Z);
    for _ in 0..30 {
        engine.step();
    }
    let after = place(&part, &engine, Vec3::Z);
    assert!(
        (before - after).length() > 0.05,
        "the plate did not turn: {before:?} vs {after:?}"
    );
}

// ------------------------------------------------------------- the scene ---

#[test]
fn the_moving_pieces_leave_the_baked_scene() {
    // If they stay in it, each one gets drawn twice: once frozen at its rest
    // pose and once following the physics.
    let mut scene = vpw_table::geometry::Scene {
        meshes: vec![
            named_mesh("LeftFlipper"),
            named_mesh("Wall1"),
            named_mesh("Plunger (shaft)"),
        ],
        materials: Vec::new(),
        images: Vec::new(),
        playfield: vpw_table::geometry::Bounds {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        },
        playfield_image: String::new(),
        playfield_material: String::new(),
        lighting: vpw_table::geometry::Lighting {
            lights: [Vec3::ZERO; 2],
            emission: [0.0; 3],
            ambient: [0.0; 3],
            range: 1.0,
            env_scale: 0.0,
            exposure: 1.0,
            reflection_strength: 0.0,
        },
        lights: Vec::new(),
    };

    scene.remove(&["LeftFlipper".to_string(), "Plunger (shaft)".to_string()]);

    let left: Vec<&str> = scene.meshes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(left, ["Wall1"]);
}

fn named_mesh(name: &str) -> vpw_table::geometry::Mesh {
    let mut m = vpw_table::ball::mesh();
    m.name = name.into();
    m
}
