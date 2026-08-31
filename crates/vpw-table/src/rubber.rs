//! Rubbers.
//!
//! A rubber is a **tube** extruded along a closed outline of control points. In
//! F-14 there are twenty-three: the ones going round the posts and the ones on
//! the slingshots.
//!
//! Port of `Rubber::GenerateMesh` (`parts/rubber.cpp:1237`) and of
//! `Rubber::UpdateRubber` (`:1381`), which is the one that positions it.
//!
//! # The frame that travels along the curve
//!
//! Extruding a tube needs a set of axes at every point of the path. Taking them
//! from scratch at each ring makes the tube **twist on itself** around the
//! curves and the texture come out crooked. The original carries the previous
//! ring's binormal forward (`rubber.cpp:1285-1286`), which is a hand-made
//! parallel transport frame; the same thing goes here.

use crate::dragpoint::{self, Point};
use crate::geometry::{Mesh, MeshKind, Vertex};
use vpw_math::{Mat4, Quat, Vec3};

/// How many sides the tube has.
///
/// The original takes it from the table's detail level and, for the rubbers that
/// go into the static buffer, always uses the maximum: `10 * 1.3 = 13`
/// (`rubber.cpp:1248-1249`). Since we bake everything at load time, we are
/// always in that case.
const SEGMENTS: usize = 13;

pub fn build(rubber: &vpin::vpx::gameitem::rubber::Rubber) -> Option<Mesh> {
    if !rubber.is_visible {
        return None;
    }
    mesh(rubber)
}

/// The same geometry, whether or not the table starts with it showing.
///
/// The two are not the same question. [`build`] is for the static scene, where
/// a rubber that is hidden at load time is simply not baked. A rubber the
/// script animates is built either way: `RSling1` and `RSling2` on F-14 both
/// start invisible and are precisely the frames of the slingshot's arm, so a
/// port that skipped them would have nothing to show when the script asks.
pub fn mesh(rubber: &vpin::vpx::gameitem::rubber::Rubber) -> Option<Mesh> {
    // The center curve, seen from above. Rubbers are always closed.
    let curve = dragpoint::expand(
        &dragpoint::from_vpin(&rubber.drag_points),
        true,
        dragpoint::ACCURACY,
    );
    if curve.len() < 3 {
        return None;
    }

    // `GetSplineVertex` closes the ring by repeating the first point at the end
    // and returns `cvertex + 1`, so there are as many rings as points.
    let rings = curve.len();
    let radius = rubber.thickness as f32 * 0.5;
    // The mesh is generated at the hit height and then translated to the drawing
    // one; see the matrix further down.
    let height = rubber.hit_height.unwrap_or(rubber.height);

    let (vertices, indices) = tube(&curve, rings, radius, height);
    let mut vertices = vertices;
    normals(&mut vertices, &indices);

    Some(Mesh {
        name: rubber.name.clone(),
        vertices,
        indices,
        transform: transform(rubber, &curve, radius, height),
        image: rubber.image.clone(),
        material: rubber.material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Rubber,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    })
}

/// The tube's rings, with their carried-along frame.
fn tube(curve: &[Point], rings: usize, radius: f32, height: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(rings * SEGMENTS);
    let mut prev_binormal = Vec3::ZERO;
    let inv_rings = 1.0 / rings as f32;
    let inv_segments = 1.0 / SEGMENTS as f32;

    for i in 0..rings {
        let i2 = if i == rings - 1 { 0 } else { i + 1 };
        let center = curve[i].pos;
        let next = curve[i2].pos;
        let tangent = Vec3::new(next.x - center.x, next.y - center.y, 0.0);

        // The first ring has nowhere to carry the frame from, so it builds it
        // against an auxiliary vector (`rubber.cpp:1279-1281`).
        let (normal, binormal) = if i == 0 {
            let up = Vec3::new(center.x + next.x, center.y + next.y, height * 2.0);
            let n = tangent.cross(up);
            (n, tangent.cross(n))
        } else {
            let n = prev_binormal.cross(tangent);
            (n, tangent.cross(n))
        };
        let normal = normal.normalize_or_zero();
        prev_binormal = binormal.normalize_or_zero();

        let u = i as f32 * inv_rings;
        let axis = tangent.normalize_or_zero();
        for j in 0..SEGMENTS {
            let v = (j as f32 + u) * inv_segments;
            let angle = (j as f32 * 360.0 * inv_segments).to_radians();
            let radial = if axis == Vec3::ZERO {
                normal * radius
            } else {
                Quat::from_axis_angle(axis, angle) * normal * radius
            };
            vertices.push(Vertex {
                pos: [center.x + radial.x, center.y + radial.y, height + radial.z],
                normal: [0.0, 0.0, 0.0],
                uv: [u, v],
            });
        }
    }

    let mut indices = Vec::with_capacity(rings * SEGMENTS * 6);
    for i in 0..rings {
        for j in 0..SEGMENTS {
            let q0 = i * SEGMENTS + j;
            let q1 = i * SEGMENTS + if j != SEGMENTS - 1 { j + 1 } else { 0 };
            let (q2, q3) = if i != rings - 1 {
                (
                    (i + 1) * SEGMENTS + j,
                    (i + 1) * SEGMENTS + if j != SEGMENTS - 1 { j + 1 } else { 0 },
                )
            } else {
                (j, if j != SEGMENTS - 1 { j + 1 } else { 0 })
            };
            // The order of the six indices is the original's
            // (`rubber.cpp:1339-1344`): 0,1,2 and then 3,2,1.
            indices.extend([
                q0 as u32, q1 as u32, q2 as u32, q3 as u32, q2 as u32, q1 as u32,
            ]);
        }
    }

    (vertices, indices)
}

/// Normals by averaging the faces that touch each vertex.
///
/// It is what `ComputeNormals` does in the original (`rubber.cpp:1346`).
pub fn normals(vertices: &mut [Vertex], indices: &[u32]) {
    let mut accum = vec![Vec3::ZERO; vertices.len()];
    for t in indices.as_chunks::<3>().0 {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (va, vb, vc) = (
            Vec3::from_array(vertices[a].pos),
            Vec3::from_array(vertices[b].pos),
            Vec3::from_array(vertices[c].pos),
        );
        // Unnormalized: that way each face weighs by its area, which is what you
        // want on a tube with rings of different sizes.
        let n = (vb - va).cross(vc - va);
        accum[a] += n;
        accum[b] += n;
        accum[c] += n;
    }
    for (v, n) in vertices.iter_mut().zip(accum) {
        v.normal = n.normalize_or_zero().to_array();
    }
}

/// The matrix that takes the tube where it belongs.
///
/// `Rubber::UpdateRubber` (`rubber.cpp:1381-1388`) rotates around the center of
/// the bounding box and then translates to the drawing height. Since the
/// original uses row vectors, in `glam` the chain goes **backwards**.
fn transform(
    rubber: &vpin::vpx::gameitem::rubber::Rubber,
    curve: &[Point],
    radius: f32,
    height: f32,
) -> Mat4 {
    // The center of the bounding box of the already generated mesh. It can be
    // worked out from the curve plus the radius, without walking the vertices.
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in curve {
        min = min.min(p.pos);
        max = max.max(p.pos);
    }
    let center = Vec3::new(
        (min.x + max.x) * 0.5,
        (min.y + max.y) * 0.5,
        // In z the mesh goes from `height - radius` to `height + radius`.
        height,
    );
    let _ = radius;

    Mat4::from_translation(Vec3::new(center.x, center.y, rubber.height))
        * Mat4::from_rotation_x(rubber.rot_x.to_radians())
        * Mat4::from_rotation_y(rubber.rot_y.to_radians())
        * Mat4::from_rotation_z(rubber.rot_z.to_radians())
        * Mat4::from_translation(-center)
}
