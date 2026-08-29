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
    let corner = |x: f32, y: f32| DragPoint {
        x,
        y,
        ..DragPoint::default()
    };
    let mut vpx = vpin::vpx::VPX::default();
    vpx.gameitems.push(GameItemEnum::Wall(Wall {
        name: "Wallsprim".into(),
        is_top_bottom_visible: true,
        is_side_visible: true,
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
    assert!(
        !drawn(&scene, "Wallsprim"),
        "the script switched it off before the scene was uploaded"
    );
}

#[test]
fn a_part_the_script_leaves_alone_is_still_drawn() {
    // The other branch of the same fork: switching parts *on* must not switch
    // anything off, or a cabinet build loses its walls instead of gaining
    // them.
    let (scene, _game) = table("Wallsprim.visible = true\n");
    assert!(drawn(&scene, "Wallsprim"));
}
