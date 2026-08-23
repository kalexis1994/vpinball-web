//! Primitives as collision, on a real table.
//!
//! A primitive is an arbitrary mesh, and on any table built in the last decade
//! it is how nearly everything solid is made — the plastics, the metal guides,
//! the toys, the habitrails. F-14 is old enough that its playfield is mostly
//! walls and ramps, which is why this was the last of the collision gaps rather
//! than the first, but it still has thirty-odd primitives that stop a ball.
//!
//! The interesting half is what is *not* built. Three flags on a primitive mean
//! three different things, and only one of them is about drawing:
//!
//! - `is_visible` is drawing, and says nothing about collision. A hidden
//!   primitive still stops the ball, in the original as here.
//! - `is_collidable` is the switch a script throws.
//! - `is_toy` is the author saying the part is scenery, and it wins over
//!   `is_collidable` (`primitive.cpp:187`).
//!
//! On F-14 that last flag does most of the work: a hundred and thirteen of a
//! hundred and forty-six primitives are toys.

use vpin::vpx::gameitem::GameItemEnum;
use vpw_physics::engine::Shape;

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

fn table() -> Option<vpin::vpx::VPX> {
    let bytes = table_bytes()?;
    Some(vpin::vpx::from_bytes(&bytes).expect("a readable .vpx"))
}

/// How many shapes each game item produced, by index.
fn shapes_per_item(vpx: &vpin::vpx::VPX) -> Vec<usize> {
    let collision = vpw_table::physics::build_with_owners(vpx);
    let mut counts = vec![0usize; vpx.gameitems.len()];
    for owner in collision.owners.iter().flatten() {
        counts[*owner] += 1;
    }
    counts
}

/// The primitives that should collide do, and the ones that should not, do not.
///
/// One test rather than three, because the three answers are only meaningful
/// against each other: "some primitives produced shapes" is satisfied by a
/// build that collides with all of them, and "the toys produced none" by one
/// that collides with nothing.
#[test]
fn only_the_solid_primitives_become_shapes() {
    let Some(vpx) = table() else { return };
    let counts = shapes_per_item(&vpx);

    let (mut solid, mut toys, mut inert) = (0, 0, 0);
    for (i, item) in vpx.gameitems.iter().enumerate() {
        let GameItemEnum::Primitive(p) = item else {
            continue;
        };
        // A primitive with no mesh in the file cannot collide whatever its
        // flags say, and a few on this table have none.
        if p.read_mesh().ok().flatten().is_none() {
            continue;
        }
        if p.is_toy {
            assert_eq!(counts[i], 0, "{} is a toy and produced shapes", p.name);
            toys += 1;
        } else if !p.is_collidable {
            assert_eq!(
                counts[i], 0,
                "{} is not collidable and produced shapes",
                p.name
            );
            inert += 1;
        } else {
            assert!(
                counts[i] > 0,
                "{} should collide and produced nothing",
                p.name
            );
            solid += 1;
        }
    }

    assert!(solid > 20, "only {solid} primitives collide on F-14");
    assert!(toys > 50, "only {toys} toys, which is not this table");
    assert!(
        inert > 0,
        "no primitive is switched off, which is suspicious"
    );
}

/// A primitive's mesh becomes the same three kinds of shape a target's does.
///
/// Triangles alone leave seams: three faces meeting at a corner have a gap a
/// ball fits through, and the edges and the corners seal it.
#[test]
fn a_primitive_gets_triangles_edges_and_corners() {
    let Some(vpx) = table() else { return };
    let collision = vpw_table::physics::build_with_owners(&vpx);

    // The largest collidable primitive on the table, so the counts are big
    // enough to mean something.
    let biggest = vpx
        .gameitems
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match item {
            GameItemEnum::Primitive(p) if p.is_collidable && !p.is_toy => {
                Some((i, p.read_mesh().ok().flatten()?.indices.len()))
            }
            _ => None,
        })
        .max_by_key(|&(_, n)| n)
        .expect("F-14 has collidable primitives");

    let (mut tris, mut edges, mut points) = (0, 0, 0);
    for (i, shape) in collision.shapes.iter().enumerate() {
        if collision.owners.get(i).copied().flatten() != Some(biggest.0) {
            continue;
        }
        match shape {
            Shape::Triangle(_) => tris += 1,
            Shape::Line3D(_) => edges += 1,
            Shape::Point(_) => points += 1,
            other => panic!("a primitive produced a {other:?}"),
        }
    }

    // One triangle per face, less the slivers. `HitTriangle` throws away a
    // face whose un-normalised normal is shorter than a tenth, because below
    // that the original does not trust its own arithmetic — and a real mesh has
    // some: this one loses about a tenth of its faces that way, and the
    // original loses the same ones.
    let faces = biggest.1;
    assert!(
        tris <= faces && tris > faces * 3 / 4,
        "{tris} triangles from {faces} faces: too many gone to be slivers"
    );
    assert!(edges > 0 && points > 0, "the seams are not sealed");
    assert!(
        edges < tris * 3,
        "{edges} edges for {tris} triangles means none are shared"
    );
}

/// Every triangle a primitive produces points somewhere.
///
/// `HitTriangle` refuses a sliver rather than producing one that answers hit
/// tests wrongly, so a mesh full of slivers would be a piece of scenery with
/// holes in it. This is the check that a real table's meshes survive the test.
#[test]
fn a_primitives_triangles_all_have_a_direction() {
    let Some(vpx) = table() else { return };
    let collision = vpw_table::physics::build_with_owners(&vpx);
    let mut checked = 0;
    for shape in &collision.shapes {
        if let Shape::Triangle(t) = shape {
            let length = t.normal.length();
            assert!(
                (length - 1.0).abs() < 1e-3,
                "a triangle's normal should be a unit vector and one is {length}"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 5000,
        "only {checked} triangles on the whole table"
    );
}
