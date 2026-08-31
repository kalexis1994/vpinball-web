//! Walls (in the editor, "surfaces").
//!
//! They are the plastics, the side rails and the slingshot walls: a closed
//! outline of control points, extruded between two heights. F-14 has forty of
//! them.
//!
//! Port of `Surface::GenerateMesh` (`parts/surface.cpp:485`). The wall is drawn
//! in two parts which the original treats separately and which can be visible
//! independently: the **side**, a ring of quads, and the **top**, the outline
//! polygon triangulated.

use crate::dragpoint::{self, Point};
use crate::geometry::{Bounds, Mesh, MeshKind, Vertex};
use vpw_math::{Mat4, Vec2, Vec3};

/// Generates a wall's meshes: side and top.
pub fn build(wall: &vpin::vpx::gameitem::wall::Wall, playfield: Bounds) -> Vec<Mesh> {
    let points = dragpoint::expand(
        &dragpoint::from_vpin(&wall.drag_points),
        true,
        dragpoint::ACCURACY,
    );
    if points.len() < 3 {
        return Vec::new();
    }

    let mut meshes = Vec::new();
    if wall.is_side_visible {
        meshes.push(side(wall, &points));
    }
    if wall.is_top_bottom_visible
        && let Some(m) = top(wall, &points, playfield)
    {
        meshes.push(m);
    }
    meshes
}

/// Normal of each segment, in the plane of the table.
///
/// It comes from `surface.cpp:498-509`. Note that it is **not** the usual
/// perpendicular: the original builds `(dy, dx)` without negating either of
/// them.
fn segment_normals(points: &[Point]) -> Vec<Vec2> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let p1 = points[i].pos;
            let p2 = points[if i < n - 1 { i + 1 } else { 0 }].pos;
            let d = Vec2::new(p1.x - p2.x, p1.y - p2.y);
            let inv = 1.0 / d.length().max(1e-9);
            Vec2::new(d.y * inv, d.x * inv)
        })
        .collect()
}

/// The ring of quads between the two heights.
fn side(wall: &vpin::vpx::gameitem::wall::Wall, points: &[Point]) -> Mesh {
    let n = points.len();
    let normals = segment_normals(points);
    let (bottom, top) = (wall.height_bottom, wall.height_top);

    let mut vertices = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);

    for i in 0..n {
        let p1 = points[i].pos;
        let p2 = points[if i < n - 1 { i + 1 } else { 0 }].pos;
        let a = if i == 0 { n - 1 } else { i - 1 };
        let c = if i < n - 1 { i + 1 } else { 0 };

        // On a smooth vertex the normal is averaged with the neighboring
        // segment's; on a corner the segment's own is used and that is that.
        let n0 = if points[i].smooth {
            ((normals[a] + normals[i]) * 0.5).normalize_or_zero()
        } else {
            normals[i]
        };
        let n1 = if points[c].smooth {
            ((normals[i] + normals[c]) * 0.5).normalize_or_zero()
        } else {
            normals[i]
        };

        let base = vertices.len() as u32;
        // The order of the four vertices is the original's: bottom-top on the
        // first point, top-bottom on the second.
        vertices.push(vertex(p1, bottom, n0, 0.0, 1.0));
        vertices.push(vertex(p1, top, n0, 0.0, 0.0));
        vertices.push(vertex(p2, top, n1, 1.0, 0.0));
        vertices.push(vertex(p2, bottom, n1, 1.0, 1.0));
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    Mesh {
        name: format!("{} (side)", wall.name),
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        image: wall.side_image.clone(),
        material: wall.side_material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Wall,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    }
}

/// One vertex of a wall's side.
///
/// The y of the normal is **negated**, which the original does on all four
/// vertices of every side quad and does not explain (`surface.cpp:574`, and
/// again at `:578`, `:582`, `:586`). Whatever the reason — the outline's
/// winding, or the y axis running toward the player — leaving it out is not a
/// silent difference: every wall on the table takes its specular highlight
/// from the wrong side, so a light in front of a wall lights the back of it.
fn vertex(p: Vec3, z: f32, n: Vec2, u: f32, v: f32) -> Vertex {
    Vertex {
        pos: [p.x, p.y, z],
        normal: [n.x, -n.y, 0.0],
        uv: [u, v],
    }
}

/// The top: the outline triangulated, at the upper height.
///
/// The UV is taken from the position over the whole table
/// (`surface.cpp:638-651`), not from the outline: that way the top's texture
/// lines up with the playfield's.
fn top(
    wall: &vpin::vpx::gameitem::wall::Wall,
    points: &[Point],
    playfield: Bounds,
) -> Option<Mesh> {
    let outline: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.pos.x, p.pos.y)).collect();
    let indices = crate::triangulate::polygon(&outline);
    if indices.is_empty() {
        return None;
    }

    let inv_width = 1.0 / (playfield.max.x - playfield.min.x).max(1e-6);
    let inv_length = 1.0 / (playfield.max.y - playfield.min.y).max(1e-6);

    let vertices = points
        .iter()
        .map(|p| Vertex {
            pos: [p.pos.x, p.pos.y, wall.height_top],
            normal: [0.0, 0.0, 1.0],
            uv: [p.pos.x * inv_width, p.pos.y * inv_length],
        })
        .collect();

    Some(Mesh {
        name: format!("{} (top)", wall.name),
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        image: wall.image.clone(),
        material: wall.top_material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Wall,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    })
}
