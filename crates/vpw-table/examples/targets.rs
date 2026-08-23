//! What a table's targets turn into, and where they are.
//!
//! ```text
//! cargo run --release -p vpw-table --example targets -- table.vpx
//! ```

use vpin::vpx::gameitem::GameItemEnum;
use vpw_physics::engine::Shape;

fn main() {
    let path = std::env::args().nth(1).expect("usage: targets <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");

    for item in vpx.gameitems.iter() {
        if let GameItemEnum::HitTarget(t) = item {
            println!(
                "{:16} {:?} at ({:.0},{:.0},{:.0}) size ({:.2},{:.2},{:.2}) rotZ {:.0} \
                 collidable {} legacy {} hitEvent {} dropped {}",
                t.name,
                t.target_type,
                t.position.x,
                t.position.y,
                t.position.z,
                t.size.x,
                t.size.y,
                t.size.z,
                t.rot_z,
                t.is_collidable,
                t.is_legacy,
                t.use_hit_event,
                t.is_dropped,
            );
        }
    }

    let collision = vpw_table::physics::build_with_owners(&vpx);
    let mut tris = 0;
    let mut on_targets = 0;
    for (i, s) in collision.shapes.iter().enumerate() {
        if matches!(s, Shape::Triangle(_)) {
            tris += 1;
            if let Some(Some(owner)) = collision.owners.get(i)
                && matches!(vpx.gameitems.get(*owner), Some(GameItemEnum::HitTarget(_)))
            {
                on_targets += 1;
            }
        }
    }
    println!(
        "\n{} shapes in all, {tris} triangles, {on_targets} of them on targets",
        collision.shapes.len()
    );
}
