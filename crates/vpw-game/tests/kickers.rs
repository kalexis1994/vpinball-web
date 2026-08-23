//! What a table gets back when it serves itself a ball.
//!
//! A saucer is the one place where the script, the physics and the ROM all have
//! to agree about a single ball at the same instant, and where disagreeing is
//! invisible: the ball is in the hole either way, and the difference only shows
//! up as a machine that never gives it back.
//!
//! Both tests here are regressions from a real capture. F-14's bumper popper
//! is animated by the table rather than by the physics:
//!
//! ```vbscript
//! Set popperBall = sw24a.Createball
//! popperBall.Z = 0
//! ...
//! sw24a.TimerInterval = 2 : sw24a.TimerEnabled = 1
//! ```
//!
//! Answering `Createball` with `Empty` made the second line raise, which took
//! the rest of the sub with it — including the timer that would have destroyed
//! the ball and told the stack it had gone. The ROM re-served the popper every
//! second and a half, and because a kicker would take a second ball on top of
//! the one it already held, each retry left another one locked in the hole with
//! nothing pointing at it. Eleven balls in thirty seconds.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};
use vpw_physics::engine::Shape;

const TABLE: &str = "../../web/debug-assets/f14.vpx";
const SCRIPTS: &str = "../../../vpinball/scripts";

/// The table and Visual Pinball's libraries, or `None` if either is missing.
///
/// Neither is in the repository — the table because it is a hundred megabytes,
/// the libraries because they belong to Visual Pinball — so these skip rather
/// than fail on a machine that does not have them.
fn loaded() -> Option<Game> {
    let table = Path::new(TABLE);
    let scripts = PathBuf::from(SCRIPTS);
    if !table.is_file() || !scripts.is_dir() {
        eprintln!("skipped: needs {TABLE} and {SCRIPTS}");
        return None;
    }
    let bytes = std::fs::read(table).expect("the table will not open");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("the table will not parse");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(scripts));
    Game::load(&vpx, &mut scene, Resources::new(libraries)).ok()
}

/// How many balls the given kicker is holding, by name: 0 or 1.
fn holding(game: &Game, kicker: &str) -> usize {
    let engine = game.engine.borrow();
    engine
        .shapes()
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            matches!(s, Shape::Kicker(k) if k.captured.is_some())
                && game
                    .items()
                    .by_shape(*i)
                    .is_some_and(|item| item.name.eq_ignore_ascii_case(kicker))
        })
        .count()
}

#[test]
fn createball_hands_the_ball_back_to_the_script() {
    let Some(mut game) = loaded() else { return };

    // The two lines from `SolPopper`, and a read back so the assertion is about
    // the ball the script is holding and not about the call not raising.
    game.script_mut()
        .load(
            r#"
            Dim popperBall, zBack
            Set popperBall = sw24a.CreateBall
            popperBall.Z = 42
            zBack = popperBall.Z
            "#,
        )
        .expect("the script raised");

    let z = game
        .script()
        .get_global("zBack")
        .expect("zBack was never assigned");
    assert_eq!(
        z.to_number().expect("z is not a number"),
        42.0,
        "the script has to get a real ball back, not Empty"
    );
}

#[test]
fn a_kicker_that_has_a_ball_does_not_take_another() {
    // `kicker.cpp:1113`. The second ball is created — the original creates it
    // too — but it is left free to roll out, and the first one keeps its place.
    // Overwriting would strand the first: a locked ball nothing points at is
    // skipped by every hit test and by the simulation loop, so it can never be
    // reached again.
    let Some(mut game) = loaded() else { return };

    game.script_mut()
        .load("Dim a : Set a = sw24a.CreateBall")
        .expect("the first CreateBall raised");
    assert_eq!(game.engine.borrow().balls.len(), 1);
    assert_eq!(holding(&game, "sw24a"), 1, "it should hold the first");

    game.script_mut()
        .load("Dim b : Set b = sw24a.CreateBall")
        .expect("the second CreateBall raised");

    assert_eq!(game.engine.borrow().balls.len(), 2, "both balls exist");
    assert_eq!(
        holding(&game, "sw24a"),
        1,
        "and the kicker still holds exactly one of them"
    );
    // The first ball is the one it kept, and it is still where it was put.
    let engine = game.engine.borrow();
    assert!(engine.balls[0].locked, "the first is still held");
    assert!(
        !engine.balls[1].locked,
        "the second is free, and rolls out instead of stacking"
    );
}
