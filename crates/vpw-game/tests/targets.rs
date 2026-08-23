//! Targets, hit with a real ball on a real table.
//!
//! F-14 Tomcat is built around eighteen stand-up targets — the ALPHA through
//! FOXTROT kill targets and the two by the right ramp — and until now the ball
//! went straight through every one of them. There was no arm for `HitTarget` in
//! `physics::build` at all, so the parts existed as far as the script was
//! concerned and did not exist as far as the ball was concerned.
//!
//! These tests aim a ball at a named target and check three things in turn: that
//! it stops, that the script hears about it, and that the machine's switch
//! closes. All three have to hold for the target to be part of the game.

use std::rc::Rc;

use vpin::vpx::gameitem::GameItemEnum;
use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};
use vpw_math::Vec3;
use vpw_vbscript::Object;

/// The table, or nothing if it is not here.
///
/// It is not in the repository — a real one is over a hundred megabytes, and it
/// is somebody else's work — so every test that needs it says so and steps
/// aside. Panicking instead turns "you have not put a table in
/// `web/debug-assets/`" into a red build on every machine but one, which is
/// what it did until a CI runner tried it.
fn table_bytes() -> Option<Vec<u8>> {
    const PATH: &str = "../../web/debug-assets/f14.vpx";
    match std::fs::read(PATH) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipped: {PATH} is not there");
            None
        }
    }
}

fn table() -> Option<(vpin::vpx::VPX, vpw_table::geometry::Scene)> {
    let bytes = table_bytes()?;
    let vpx = vpin::vpx::from_bytes(&bytes).expect("a readable .vpx");
    let scene = vpw_table::geometry::extract(&vpx);
    Some((vpx, scene))
}

/// The table with its ROM, sitting in attract mode.
fn loaded() -> Option<Game> {
    let (vpx, mut scene) = table()?;
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(
        std::path::Path::new("../../../vpinball/scripts").into(),
    ));
    let roms: Rc<dyn RomSource> = Rc::new(RomDir("../../web/debug-assets/roms".into()));
    let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries).with_roms(roms))
        .expect("the table should load");
    game.start().expect("the script should start");
    Some(game)
}

/// Where a target stands, from the file rather than from the engine.
fn target_position(name: &str) -> Vec3 {
    // Only reached from a test that has already stepped aside if the table is
    // not there, so this cannot be the thing that fails.
    let (vpx, _) = table().expect("the caller checked");
    for item in vpx.gameitems.iter() {
        if let GameItemEnum::HitTarget(t) = item
            && t.name == name
        {
            return Vec3::new(t.position.x, t.position.y, t.position.z);
        }
    }
    panic!("{name} is not on this table");
}

/// How far in front of a target the ball is put down.
///
/// Short on purpose. F-14's targets stand in banks of three about forty units
/// apart, so a ball started any further back arrives at the wrong one — which
/// is how these tests found out that they work.
const RUN_UP: f32 = 55.0;

/// Where a ball has to come from to hit a target on the face.
///
/// A target faces the player: its hit plane sits at a positive `y` in the
/// target's own space (`hittarget.cpp:191`, where every vertex has y near zero
/// and the plane is offset further positive), and `rot_z` turns that facing
/// with the target. So the front is `RUN_UP` units along the target's own +y.
fn in_front_of(name: &str) -> Vec3 {
    let (vpx, _) = table().expect("the caller checked");
    for item in vpx.gameitems.iter() {
        if let GameItemEnum::HitTarget(t) = item
            && t.name == name
        {
            let a = t.rot_z.to_radians();
            return Vec3::new(
                t.position.x - a.sin() * RUN_UP,
                t.position.y + a.cos() * RUN_UP,
                t.position.z + 25.0,
            );
        }
    }
    panic!("{name} is not on this table");
}

/// Rolls a ball at a target from `from`, and says where it ended up.
fn roll_into(game: &mut Game, target: Vec3, from: Vec3, speed: f32) -> Vec3 {
    let ball = launch(game, target, from, speed);
    for _ in 0..200 {
        game.step();
    }
    game.engine.borrow().balls[ball].pos
}

/// Puts a ball down in front of the target and shoves it at it.
fn launch(game: &mut Game, target: Vec3, from: Vec3, speed: f32) -> usize {
    let mut engine = game.engine.borrow_mut();
    let ball = engine.add_ball(vpw_physics::ball::Ball::new(from, 25.0));
    engine.balls[ball].vel = (target - from).normalize() * speed;
    ball
}

/// Rolls a ball at a target and reports whether `watch` was ever true along the
/// way.
///
/// Watching **during** the roll and not after it, which matters more than it
/// sounds: `vpmTimer.PulseSw` closes a switch and opens it again two ticks of a
/// 25 Hz timer later, so the whole of a target's switch closure is forty
/// milliseconds that happen a moment after the ball arrives. Look afterwards
/// and it has already been and gone.
fn roll_and_watch(
    game: &mut Game,
    target: Vec3,
    from: Vec3,
    speed: f32,
    watch: impl Fn(&Game) -> bool,
) -> bool {
    launch(game, target, from, speed);
    for _ in 0..400 {
        game.step();
        if watch(game) {
            return true;
        }
    }
    false
}

/// The first thing: a target is solid.
#[test]
fn a_ball_does_not_pass_through_a_target() {
    let Some(mut game) = loaded() else { return };
    let target = target_position("sw25");
    let from = in_front_of("sw25");

    let landed = roll_into(&mut game, target, from, 40.0);

    // Through it and still going would put it well past, on the far side.
    assert!(
        landed.y > target.y - 20.0,
        "the ball went through sw25: target at y {:.0}, ball ended at y {:.0}",
        target.y,
        landed.y
    );
}

/// The second: the script hears it.
///
/// F-14 wires every one of these to `vpmTimer.PulseSw`, so a hit that does not
/// reach the script is a target that is solid and silent — which is worse than
/// one that does nothing, because it blocks a shot without scoring it.
#[test]
fn hitting_a_target_reaches_the_script() {
    let Some(mut game) = loaded() else { return };
    // A coin and a game, so the ROM is out of attract mode and will act on a
    // switch rather than ignoring it.
    for _ in 0..4000 {
        game.step();
    }
    for (key, hold, after) in [("Digit5", 60, 2500), ("Digit1", 60, 3000)] {
        game.key(key, true);
        for _ in 0..hold {
            game.step();
        }
        game.key(key, false);
        for _ in 0..after {
            game.step();
        }
    }

    let target = target_position("sw25");
    let closed = roll_and_watch(&mut game, target, in_front_of("sw25"), 40.0, |g| {
        g.machine().switch_closed(25)
    });

    assert!(closed, "switch 25 never closed after hitting sw25");
}

/// And a target that is nowhere near the ball stays quiet.
///
/// The mesh of one target becomes several hundred shapes, and getting the
/// transform wrong would scatter them across the playfield where they would
/// stop balls that should have gone by. This is the check that they are where
/// the target is.
#[test]
fn a_target_only_blocks_its_own_place() {
    let Some(mut game) = loaded() else { return };
    let target = target_position("sw25");
    // Alongside it rather than at it: the same approach, shifted far enough to
    // the side to clear the target and everything standing beside it.
    let from = in_front_of("sw25") + Vec3::new(-190.0, 0.0, 0.0);
    let to = Vec3::new(from.x, from.y - 200.0, target.z);

    let landed = roll_into(&mut game, to, from, 40.0);

    assert!(
        landed.y < target.y,
        "something stopped a ball that passes well clear of sw25: ended at y {:.0}, \
         target at y {:.0}",
        landed.y,
        target.y
    );
}

/// A script can take a target out of the way.
///
/// `Collidable = False` has to reach all seven hundred odd shapes the target's
/// mesh turned into. Missing any of them leaves an invisible fragment of target
/// standing where the ball is supposed to go through, which is the sort of
/// thing that reads as "the physics is wrong" rather than as a missed flag.
#[test]
fn turning_a_target_off_lets_the_ball_through() {
    let Some(mut game) = loaded() else { return };
    let target = target_position("sw25");
    let from = in_front_of("sw25");

    let blocked = roll_into(&mut game, target, from, 40.0);

    game.items()
        .get("sw25")
        .expect("sw25 is an item")
        .set("Collidable", &[], vpw_vbscript::Value::Bool(false), false)
        .expect("a target can be switched off");
    // A fresh ball, since the first one is still lying against the target.
    game.engine.borrow_mut().balls.clear();
    let through = roll_into(&mut game, target, from, 40.0);

    assert!(
        through.y < blocked.y - 20.0,
        "switching sw25 off should let the ball past: stopped at y {:.0}, \
         with it off it reached y {:.0}",
        blocked.y,
        through.y
    );
}

/// And a stand-up target does not drop.
///
/// Both kinds are `HitTarget` in the file and only the type tells them apart.
/// F-14's are all stand-ups, so every one of them being knocked flat by the
/// first ball would be a quiet disaster.
#[test]
fn a_stand_up_target_does_not_drop() {
    let Some(mut game) = loaded() else { return };
    let target = target_position("sw25");
    roll_into(&mut game, target, in_front_of("sw25"), 40.0);

    let item = game.items().get("sw25").expect("sw25 is an item");
    assert!(!item.drops(), "sw25 is a stand-up, not a drop target");
    assert!(!item.is_dropped(), "and it is still standing");
}

// ------------------------------------------------------------ slingshots ---

/// Where each slingshot segment is, and which way it faces.
fn slingshots(game: &Game) -> Vec<(String, Vec3, Vec3)> {
    let engine = game.engine.borrow();
    let mut out = Vec::new();
    for (i, shape) in engine.shapes().iter().enumerate() {
        let vpw_physics::engine::Shape::Slingshot(s) = shape else {
            continue;
        };
        let mid = (s.seg.v1 + s.seg.v2) * 0.5;
        let name = game
            .items()
            .by_shape(i)
            .map(|it| it.name.to_string())
            .unwrap_or_default();
        out.push((
            name,
            Vec3::new(mid.x, mid.y, (s.seg.z_low + s.seg.z_high) * 0.5),
            Vec3::new(s.seg.normal.x, s.seg.normal.y, 0.0),
        ));
    }
    out
}

#[test]
fn a_slingshot_announces_itself_and_not_as_an_ordinary_hit() {
    // Everything a slingshot does for the player hangs off one script handler.
    // On F-14, `LeftSlingShot_Slingshot` reports the switch, plays
    // `fx_slingshot`, swaps the three plastics that draw the arm moving and
    // starts the timer that puts them back. There is no `_Hit` to fall back on,
    // and the original would not call one anyway: `LineSegSlingshot::Collide`
    // replaces the plain segment's `Collide` outright and fires
    // `DISPID_SurfaceEvents_Slingshot` and nothing else (`collideex.cpp:61`).
    //
    // The port was throwing away the one bit that says the solenoid went off,
    // so the ball was flung out in silence off a plastic that never moved.
    let Some(mut game) = loaded() else { return };
    let slings = slingshots(&game);
    assert!(
        slings.len() >= 2,
        "F-14 has one over each flipper; the table gave {}",
        slings.len()
    );

    for (name, at, normal) in slings {
        game.take_sounds();

        // From in front of the rubber, hard enough to beat the threshold.
        let from = at + normal * 40.0;
        let mut engine = game.engine.borrow_mut();
        let ball = engine.add_ball(vpw_physics::ball::Ball::new(from, 25.0));
        engine.balls[ball].vel = -normal * 30.0;
        drop(engine);

        let mut heard = false;
        for _ in 0..80 {
            game.step();
            if game.take_sounds().iter().any(|s| s.contains("slingshot")) {
                heard = true;
                break;
            }
        }
        assert!(heard, "{name} bounced the ball without a sound");

        game.engine.borrow_mut().remove_ball(0);
    }
}

#[test]
fn a_slingshot_draws_its_arm_moving_and_puts_it_back() {
    // The sound is only half of it. F-14 draws the arm as three rubbers stacked
    // in the same place — `LSling`, `LSling1`, `LSling2` — and the handler
    // shows them in turn off a timer while the plastic under them dips on its
    // `TransZ` and comes back up.
    //
    // Two things had to be true for any of that to show, and neither was. The
    // rubbers were baked into the static scene, so changing `Visible` on one
    // changed nothing on screen: the table marks them `EnableStaticRendering =
    // false` exactly to say they are not static, which is the same flag the
    // port already honoured for primitives. And their visibility was never
    // seeded from the file, so all three read as shown and the arm was drawn in
    // all of its positions at once, permanently.
    let Some(mut game) = loaded() else { return };
    let frames = ["LSling", "LSling1", "LSling2"];

    let shown = |g: &Game| -> Vec<String> {
        g.parts()
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                g.part_visible(*i) && frames.iter().any(|n| p.mesh.name.eq_ignore_ascii_case(n))
            })
            .map(|(_, p)| p.mesh.name.clone())
            .collect()
    };
    let dip = |g: &Game| g.items().get("SLING2").map(|it| it.placement()[5]).unwrap();

    // All three are parts, and only the resting one shows.
    for frame in frames {
        assert!(
            game.parts()
                .iter()
                .any(|p| p.mesh.name.eq_ignore_ascii_case(frame)),
            "{frame} has to be a moving part; baked, the script cannot hide it"
        );
    }
    assert_eq!(shown(&game), ["LSling"], "at rest, one frame");
    assert_eq!(dip(&game), 0.0, "and the plastic is up");

    let (_, at, normal) = slingshots(&game)
        .into_iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case("LeftSlingShot"))
        .expect("F-14 has a LeftSlingShot");
    let mut engine = game.engine.borrow_mut();
    let ball = engine.add_ball(vpw_physics::ball::Ball::new(at + normal * 40.0, 25.0));
    engine.balls[ball].vel = -normal * 30.0;
    drop(engine);

    // It goes through its frames, and the plastic goes down with it.
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut lowest = 0.0f32;
    for _ in 0..1200 {
        game.step();
        let now = shown(&game);
        if seen.last() != Some(&now) {
            seen.push(now);
        }
        lowest = lowest.min(dip(&game));
    }

    assert!(
        seen.len() >= 3,
        "the arm should have moved through its frames; it showed {seen:?}"
    );
    assert!(
        lowest <= -20.0,
        "the plastic should dip; it got to {lowest}"
    );

    // And it ends where it started. A slingshot that fires and stays fired is
    // worse than one that never moves.
    assert_eq!(shown(&game), ["LSling"], "back to rest: {seen:?}");
    assert_eq!(dip(&game), 0.0, "and the plastic back up");
}
