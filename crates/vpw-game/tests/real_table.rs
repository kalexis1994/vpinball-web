//! A published table, loaded and played.
//!
//! This is the test the whole crate exists for. Everything below it can be
//! right in isolation and still not add up to a game: the physics can simulate,
//! the interpreter can interpret, and the table still does nothing because a
//! name did not resolve or an event went to a handler that is spelled
//! differently.
//!
//! It needs a real `.vpx`, which is a hundred megabytes and is not in the
//! repository, and Visual Pinball's script library next to it. Without either
//! the tests skip themselves, the same way the ones that need a ROM do.
//!
//! ```text
//! # a table, from wherever you keep yours
//! cp "F-14 Tomcat (Williams 1987) 1.0.vpx" web/debug-assets/f14.vpx
//! # and the library every table shares
//! git -C ../vpinball sparse-checkout add /scripts
//! ```

use std::path::Path;

use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

const TABLE: &str = "../../web/debug-assets/f14.vpx";
const SCRIPTS: &str = "../../../vpinball/scripts";
const ROMS: &str = "../../web/debug-assets/roms";

struct Loaded {
    game: Game,
    scene: vpw_table::geometry::Scene,
}

fn load() -> Option<Loaded> {
    if !Path::new(TABLE).is_file() {
        eprintln!("skipped: {TABLE} is not there");
        return None;
    }
    let bytes = std::fs::read(TABLE).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: std::rc::Rc<dyn ScriptLibrary> = if Path::new(SCRIPTS).is_dir() {
        std::rc::Rc::new(LibraryDir(Path::new(SCRIPTS).into()))
    } else {
        eprintln!("note: {SCRIPTS} is not there, so the table runs without core.vbs");
        std::rc::Rc::new(vpw_game::NoLibraries)
    };

    // The ROM, if it is there. Without it F-14 loads and plays, but its score
    // never moves: the rules are in the firmware, not the script.
    let mut resources = Resources::new(libraries);
    let roms = Path::new(ROMS);
    if roms.is_dir() {
        resources =
            resources.with_roms(std::rc::Rc::new(vpw_game::controller::RomDir(roms.into())));
    } else {
        eprintln!("note: {ROMS} is not there, so the table runs without its ROM");
    }

    let game = match Game::load(&vpx, &mut scene, resources) {
        Ok(g) => g,
        Err(e) => panic!("the table failed to load: {e}"),
    };
    Some(Loaded { game, scene })
}

#[test]
fn a_real_table_loads_with_its_script() {
    let Some(l) = load() else { return };

    // The parts the script is going to reach for by name.
    let items = l.game.items();
    assert!(
        items.len() > 100,
        "a real table has hundreds of parts, not {}",
        items.len()
    );
    for wanted in ["LeftFlipper", "RightFlipper", "Drain", "Plunger"] {
        assert!(
            items.get(wanted).is_some(),
            "the table should have a part called {wanted}"
        );
    }
    // And names resolve without regard to case, because scripts do not care.
    assert!(items.get("leftflipper").is_some());
}

#[test]
fn the_script_starts_the_table_without_complaining() {
    let Some(mut l) = load() else { return };
    l.game.start().expect("Table1_Init should run");

    // Silence is the result worth asserting. A table's start-up is a gauntlet
    // of `On Error Resume Next` blocks that each end in a `MsgBox` when
    // something the host should have provided is missing — a version number, a
    // settings key, a library. Every one of those messages was a real gap in
    // the binding at some point, and the absence of all of them is what says
    // the table got through its own start-up the way it was written to.
    let messages = l.game.take_messages();
    assert!(
        messages.is_empty(),
        "the table complained on start-up: {messages:?}"
    );
}

#[test]
fn the_table_gets_a_controller() {
    // F-14 is a ROM table: its whole game lives in the ROM, and its script
    // talks to it through `Controller`. Until PinMAME is wired in there is a
    // stand-in behind that name, but the name has to resolve — a table whose
    // `Controller` is `Empty` stops at the first switch it tries to report.
    let Some(mut l) = load() else { return };
    l.game.start().ok();
    l.game
        .script()
        .load("Dim t : t = TypeName(Controller)")
        .expect("Controller should be an object by now");
    let t = l.game.script().get_global("t").unwrap();
    assert_eq!(&*t.to_str().unwrap(), "Controller");
}

#[test]
fn a_key_reaches_both_the_flipper_and_the_script() {
    let Some(mut l) = load() else { return };
    l.game.start().ok();

    let flipper = l
        .game
        .items()
        .get("LeftFlipper")
        .expect("F-14 has a left flipper");
    let shape = *flipper.shapes.first().expect("it has a collision shape");

    let angle = |g: &Game| match g.engine.borrow().shapes().get(shape) {
        Some(vpw_physics::engine::Shape::Flipper(f)) => f.angle_cur,
        _ => panic!("that shape is not a flipper"),
    };

    let at_rest = angle(&l.game);
    l.game.key("KeyZ", true);
    for _ in 0..60 {
        l.game.step();
    }
    let raised = angle(&l.game);
    assert!(
        (raised - at_rest).abs() > 0.05,
        "the flipper did not move: {at_rest} -> {raised}"
    );
}

#[test]
fn the_table_runs_for_a_while_without_falling_over() {
    // Ten seconds of table time with a ball in play, every event dispatched
    // into the script and every timer run. What this catches is not one bug in
    // particular: it is the class of bug where a handler panics, a borrow is
    // held across a call, or a timer schedules itself into an infinite loop.
    let Some(mut l) = load() else { return };
    l.game.start().ok();
    l.game.new_ball();

    for _ in 0..10_000 {
        l.game.step();
    }

    assert!(
        !l.game.engine.borrow().balls.is_empty(),
        "the ball should still be on the table"
    );
}

#[test]
fn the_moving_parts_come_out_of_the_baked_scene() {
    // If they do not, every one of them is drawn twice: once frozen at its
    // rest pose and once following the physics.
    let Some(l) = load() else { return };
    let names: Vec<&str> = l.scene.meshes.iter().map(|m| m.name.as_str()).collect();
    for part in l.game.parts() {
        assert!(
            !names.contains(&&*part.mesh.name),
            "{} is both animated and baked into the scene",
            part.mesh.name
        );
    }
    assert!(!l.game.parts().is_empty(), "a table has moving parts");
}

#[test]
fn a_script_can_drive_the_flippers_it_names() {
    // A table's key handler calls `RotateToEnd` by name, and that has to reach
    // the physics. This is the whole binding in one line of VBScript.
    let Some(l) = load() else { return };

    let flipper = l.game.items().get("LeftFlipper").unwrap();
    let shape = *flipper.shapes.first().unwrap();
    let solenoid = |g: &Game| match g.engine.borrow().shapes().get(shape) {
        Some(vpw_physics::engine::Shape::Flipper(f)) => f.solenoid,
        _ => panic!("not a flipper"),
    };

    assert!(!solenoid(&l.game));
    l.game
        .script()
        .load("LeftFlipper.RotateToEnd")
        .expect("the script should reach the flipper");
    assert!(solenoid(&l.game), "RotateToEnd did not reach the physics");

    l.game.script().load("LeftFlipper.RotateToStart").unwrap();
    assert!(!solenoid(&l.game));
}

#[test]
fn the_script_sees_the_flippers_angle_in_degrees() {
    // Visual Pinball reports angles in degrees and the physics keeps radians.
    // A table that compares `CurrentAngle` against a number gets nonsense if
    // the two are confused.
    let Some(l) = load() else { return };
    // `Dim` because the table's own script turned `Option Explicit` on, and
    // anything loaded afterwards is held to it.
    l.game
        .script()
        .load("Dim a : a = LeftFlipper.CurrentAngle")
        .unwrap();
    let a = l
        .game
        .script()
        .get_global("a")
        .unwrap()
        .to_number()
        .unwrap();
    assert!(
        a.abs() <= 180.0,
        "{a} does not look like an angle in degrees"
    );
}

/// The flipper you see on F-14 is not the flipper part.
///
/// The script hides the built-in bat (`LeftFlipper.visible = 0`) and shows a
/// primitive in its place, turning it from a timer with
/// `batleft.ObjRotZ = LeftFlipper.CurrentAngle + 1`. A port that bakes
/// primitives into the static scene draws a flipper that never moves, with the
/// hidden one poking out from under it — which is exactly how this was found.
#[test]
fn the_primitive_the_script_turns_is_drawn_where_the_script_puts_it() {
    let Some(mut l) = load() else { return };
    l.game.start().ok();

    let bat = l
        .game
        .parts()
        .iter()
        .position(|p| p.mesh.name.eq_ignore_ascii_case("batleft"));
    let Some(bat) = bat else {
        eprintln!("skipped: this table has no primitive flipper bats");
        return;
    };

    // The built-in flipper is hidden and the bat is shown. Both are moving
    // parts; the difference is which one the script wants drawn.
    for _ in 0..200 {
        l.game.step();
    }
    let flipper = l
        .game
        .parts()
        .iter()
        .position(|p| p.mesh.name.eq_ignore_ascii_case("LeftFlipper"))
        .expect("the table has a LeftFlipper");
    assert!(
        !l.game.part_visible(flipper),
        "the script hides the built-in flipper"
    );
    assert!(l.game.part_visible(bat), "and shows the primitive bat");

    let at_rest = l.game.part_transform(bat);

    l.game.key("KeyZ", true);
    for _ in 0..200 {
        l.game.step();
    }
    let raised = l.game.part_transform(bat);

    assert_ne!(
        at_rest, raised,
        "the bat should have turned with the flipper"
    );
    // And it turned rather than moved: the pivot stays put.
    assert_eq!(
        at_rest.w_axis, raised.w_axis,
        "a flipper turns about its pivot; it does not travel"
    );
}

#[test]
fn the_clock_a_script_reads_actually_runs() {
    // A name that resolves is remembered, so `LeftFlipper` costs one lookup for
    // the whole game. Right for a part, quietly fatal for a clock: `GameTime`
    // went through the same door and the first read froze it at zero for good.
    //
    // What that costs is not subtle. `core.vbs` schedules a delayed solenoid
    // with `If GameTime >= FlipAt(aIdx) + RomControlDelay` (line 2208), which
    // with a frozen zero is `0 >= 0 + delay` — never. And it refreshes the
    // playfield with `UpdateVisual = (GameTime - LastPinMameVisualSync > 10)`
    // (line 2411), which is `0 - 0 > 10` — never either.
    let Some(mut l) = load() else { return };
    l.game.start().ok();

    let read = |g: &mut Game, name: &str| -> f64 {
        g.script()
            .load(&format!("Dim probe : probe = {name}"))
            .unwrap_or_else(|e| panic!("{name} should be readable: {e}"));
        g.script().get_global("probe").unwrap().to_number().unwrap()
    };

    let before = read(&mut l.game, "GameTime");
    for _ in 0..2500 {
        l.game.step();
    }
    let after = read(&mut l.game, "GameTime");
    assert!(
        after - before >= 2000.0,
        "two and a half seconds in, GameTime went from {before} to {after}"
    );

    // And `PreciseGameTime` is the same clock in **seconds**: it is
    // `m_time_sec` (`ScriptGlobalTable.cpp:691`), and `core.vbs` treats
    // `GameTime / 1000.0` as its replacement (line 2401). Handing back
    // milliseconds is a thousandfold error in whatever reads it.
    let precise = read(&mut l.game, "PreciseGameTime");
    let millis = read(&mut l.game, "GameTime");
    assert!(
        (precise - millis / 1000.0).abs() < 0.05,
        "PreciseGameTime is in seconds: {precise} against {millis} ms"
    );
}
