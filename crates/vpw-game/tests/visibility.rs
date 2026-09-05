//! What the script switches off before anybody sees the table.
//!
//! A table's first lines are very often a fork on `Table.ShowDT`: one set of
//! parts for a desktop and another for a cabinet, with the set that does not
//! belong switched off by name. The file stores *both* sets visible, because
//! the file is what the script edits — so a port that reads the file and never
//! asks the script ends up drawing both.
//!
//! The Sopranos is where this was found: its cabinet walls stood in the middle
//! of the playfield as a black slab.

use std::rc::Rc;

use vpin::vpx::gameitem::GameItemEnum;
use vpin::vpx::gameitem::dragpoint::DragPoint;
use vpin::vpx::gameitem::wall::Wall;
use vpw_game::{Game, NoLibraries, Resources, ScriptLibrary};

/// A table with one wall on it, and whatever script is given.
///
/// A wall rather than a primitive because a wall carries its whole shape in
/// the file — four corners is a mesh — where a primitive without stored
/// vertices is not drawn at all and would prove nothing.
fn table(script: &str) -> (vpw_table::geometry::Scene, Game) {
    table_with(true, script)
}

/// The same table, with the wall shown or hidden in the file.
fn table_with(visible: bool, script: &str) -> (vpw_table::geometry::Scene, Game) {
    let corner = |x: f32, y: f32| DragPoint {
        x,
        y,
        ..DragPoint::default()
    };
    let mut vpx = vpin::vpx::VPX::default();
    vpx.gameitems.push(GameItemEnum::Wall(Wall {
        name: "Wallsprim".into(),
        is_top_bottom_visible: visible,
        is_side_visible: visible,
        drag_points: vec![
            corner(100.0, 100.0),
            corner(400.0, 100.0),
            corner(400.0, 400.0),
            corner(100.0, 400.0),
        ],
        ..Wall::default()
    }));
    vpx.gamedata.code = script.as_bytes().into();

    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(NoLibraries);
    let game = Game::load(&vpx, &mut scene, Resources::new(libraries))
        .expect("the table should have loaded");
    (scene, game)
}

/// Whether any of that part's meshes is still going to be drawn.
///
/// One part becomes several meshes and the extra ones say what they are in
/// brackets — a wall is `Wallsprim (top)` and `Wallsprim (side)` — so the
/// match is on the part the mesh belongs to rather than on the mesh's own
/// name.
fn drawn(scene: &vpw_table::geometry::Scene, part: &str) -> bool {
    scene.meshes.iter().any(|m| {
        m.visible
            && (m.name.eq_ignore_ascii_case(part)
                || m.name
                    .to_ascii_lowercase()
                    .starts_with(&format!("{} (", part.to_lowercase())))
    })
}

#[test]
fn the_file_alone_leaves_the_wall_standing() {
    // The control. Without a script saying otherwise the wall is drawn, which
    // is what makes the next test mean something.
    let (scene, _game) = table("");
    assert!(drawn(&scene, "Wallsprim"), "the wall is in the file");
}

#[test]
fn a_part_the_script_hides_at_load_is_not_drawn() {
    // The shape a real table writes it in: a fork on the desktop flag, with
    // the parts that belong to the other mode switched off by name.
    let (scene, _game) = table(
        "If Table.ShowDT Then\n\
         \tWallsprim.visible = false\n\
         End If\n",
    );
    // `Visible` is the top of a wall (`surface.cpp:1594`); the side has its
    // own flag and the file's one still stands.
    assert!(
        !drawn(&scene, "Wallsprim (top)"),
        "the script switched it off before the scene was uploaded"
    );
    assert!(drawn(&scene, "Wallsprim (side)"), "the side was never hidden");
}

/// The other direction: a part the file hides and the script shows.
///
/// Circus keeps its desktop rails as two ramps the file hides, and its `Init`
/// turns them on for a desktop — which is the original's order too: the
/// static picture is taken after `Init` has run (`player.cpp:739`) from each
/// part's live flag (`ramp.cpp:862`). Without this the whole bottom of the
/// table was missing.
#[test]
fn a_part_the_script_shows_at_init_is_drawn() {
    let (mut scene, mut game) = table_with(
        false,
        "Sub Table1_Init\n\
         \tWallsprim.visible = true\n\
         \tWallsprim.sidevisible = true\n\
         End Sub\n",
    );
    assert!(!drawn(&scene, "Wallsprim"), "the file hides it");
    game.start().expect("Init should run");
    game.apply_visibility_changes(&mut scene);
    assert!(drawn(&scene, "Wallsprim"), "Init showed it");
}

/// A wall has two flags, and showing the top must not show the side.
#[test]
fn a_walls_side_has_its_own_flag() {
    let (mut scene, mut game) = table_with(
        false,
        "Sub Table1_Init\n\
         \tWallsprim.visible = true\n\
         End Sub\n",
    );
    game.start().expect("Init should run");
    game.apply_visibility_changes(&mut scene);
    let shown: Vec<&str> = scene
        .meshes
        .iter()
        .filter(|m| m.visible && m.name.starts_with("Wallsprim"))
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(shown, ["Wallsprim (top)"]);
}

#[test]
fn a_part_the_script_leaves_alone_is_still_drawn() {
    // The other branch of the same fork: switching parts *on* must not switch
    // anything off, or a cabinet build loses its walls instead of gaining
    // them.
    let (scene, _game) = table("Wallsprim.visible = true\n");
    assert!(drawn(&scene, "Wallsprim"));
}

// --- the table's own name ----------------------------------------------------

/// A table whose object is not called `Table1` still gets its keys.
///
/// Visual Pinball's editor calls the table object `Table1` and most tables
/// keep it, so looking only for `Table1_KeyDown` works nearly always — and the
/// times it does not are silent and total. The Sopranos calls its table
/// `Table`: the flippers still moved, because the physics answers the keyboard
/// itself, and nothing that had to go through the machine did. No coin, no
/// start, no game.
#[test]
fn the_key_handler_is_named_after_the_table_object() {
    for (object, script) in [
        ("Table", "Sub Table_KeyDown(k) : seen = seen + 1 : End Sub"),
        (
            "Table1",
            "Sub Table1_KeyDown(k) : seen = seen + 1 : End Sub",
        ),
        // The file says one thing and the script another, which is a table
        // that should still play.
        (
            "Whatever",
            "Sub Table1_KeyDown(k) : seen = seen + 1 : End Sub",
        ),
    ] {
        let mut vpx = vpin::vpx::VPX::default();
        vpx.gamedata.name = object.into();
        vpx.gamedata.code = format!("Dim seen : seen = 0\n{script}\n").as_bytes().into();

        let mut scene = vpw_table::geometry::extract(&vpx);
        let libraries: Rc<dyn ScriptLibrary> = Rc::new(NoLibraries);
        let mut game = Game::load(&vpx, &mut scene, Resources::new(libraries))
            .expect("the table should have loaded");

        game.key("Digit5", true);
        assert_eq!(
            game.script()
                .get_global("seen")
                .and_then(|v| v.to_number().ok()),
            Some(1.0),
            "the key never reached {object}'s handler"
        );
    }
}

// --- what the table puts away for itself ------------------------------------

/// `LoadValue` answers "" for something never saved, and `SaveValue` comes
/// back through it.
///
/// Both matter and the first one more: `If LoadValue(...) = "" Then` is how
/// every table asks whether anybody has played it before, so a `LoadValue`
/// that raised instead of answering stopped Ice Fever on the first line of its
/// `Init` — and a table that does not start is a table with nothing on the
/// screen.
#[test]
fn a_table_keeps_its_own_values() {
    let mut vpx = vpin::vpx::VPX::default();
    vpx.gamedata.name = "Table1".into();
    vpx.gamedata.code = "Dim before, after\n\
         before = LoadValue(\"Ice\", \"HighScore\")\n\
         SaveValue \"Ice\", \"HighScore\", 12345\n\
         after = LoadValue(\"Ice\", \"HighScore\")\n"
        .as_bytes()
        .into();

    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(NoLibraries);
    let game =
        Game::load(&vpx, &mut scene, Resources::new(libraries)).expect("the table should load");

    let text = |name: &str| {
        game.script()
            .get_global(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    assert_eq!(text("before").as_deref(), Some(""), "never saved is empty");
    assert_eq!(text("after").as_deref(), Some("12345"));

    // And it comes out as a flat list for the page to keep, keyed by the
    // table and the name folded, so a reload can put it back.
    let pairs = game.table_values();
    assert_eq!(
        pairs,
        vec!["ice/highscore".to_string(), "12345".to_string()]
    );
}

/// A previous session's values are there before the table's `Init` looks.
#[test]
fn values_handed_over_are_there_from_the_start() {
    let mut vpx = vpin::vpx::VPX::default();
    vpx.gamedata.name = "Table1".into();
    vpx.gamedata.code = "Dim seen\n".as_bytes().into();

    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(NoLibraries);
    let mut game =
        Game::load(&vpx, &mut scene, Resources::new(libraries)).expect("the table should load");
    game.restore_table_values(&["ice/highscore".into(), "999".into()]);

    game.script_mut()
        .load("seen = LoadValue(\"Ice\", \"HighScore\")")
        .expect("the snippet should run");
    assert_eq!(
        game.script()
            .get_global("seen")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .as_deref(),
        Some("999")
    );
}
