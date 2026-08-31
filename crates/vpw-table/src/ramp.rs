//! Ramps.
//!
//! A ramp is an **open** path of control points that climbs from one height to
//! another, and that widens or narrows along the way. There are two families,
//! with different geometries (`ramp.h`, `RampType`):
//!
//! - **Flat**: a floor of quad strips plus two side walls, each with its own
//!   height (`Ramp::GenerateRampMesh`, `ramp.cpp:2121`).
//! - **Wire**: one, two, three or four tubes following the path
//!   (`Ramp::GenerateWireMesh`, `ramp.cpp:1071`). These are the *habitrails*.
//!
//! Computing the outline is shared by both, and it is `Ramp::GetRampVertex`
//! (`ramp.cpp:331`).

use crate::dragpoint::{self, Point};
use crate::geometry::{Bounds, Mesh, MeshKind, Vertex};
use vpin::vpx::gameitem::ramp::{Ramp, RampType};
use vpw_math::{Mat4, Quat, Vec2, Vec3};

/// Sides of each tube, same as on the rubbers.
const SEGMENTS: usize = 13;

/// The ramp's outline, already resolved.
pub struct Outline {
    /// Points of the center curve.
    pub center: Vec<Vec2>,
    /// Left and right edge at the height of each point.
    pub left: Vec<Vec2>,
    pub right: Vec<Vec2>,
    /// Height of each point.
    pub height: Vec<f32>,
    /// How much of the path has been covered, from 1 to 0. The original uses it
    /// as a texture coordinate.
    pub ratio: Vec<f32>,
}

/// The ramp's two edges and the height along it — `Ramp::GetRampVertex`.
///
/// One function, two accuracies. The shape is the same either way — the same
/// control points, the same spline — but the collision code walks it far more
/// coarsely than the renderer does, which is the original's choice and not an
/// accident: `Ramp::PhysicSetup` (`ramp.cpp:545`) passes the hit-shape detail
/// level where the renderer passes the table's.
///
/// Pass [`dragpoint::collision_accuracy`] for the shapes the ball meets and
/// [`dragpoint::ACCURACY`] for the ones it sees.
pub fn path(ramp: &Ramp, accuracy: f32) -> Option<Outline> {
    // Ramps are not loops: the path has a beginning and an end.
    let curve = dragpoint::expand(&dragpoint::from_vpin(&ramp.drag_points), false, accuracy);
    if curve.len() < 2 {
        return None;
    }
    Some(outline(ramp, &curve, edge_width(ramp)))
}

/// The path the **ball** meets, which is not quite the one it sees.
///
/// For a habitrail the original widens the collision outline by twenty units
/// over the wire spacing — `widthcur = m_wireDistanceX; if (inc_width)
/// widthcur += 20.0f;` (`ramp.cpp:471-474`), and `PhysicSetup` is the one
/// caller that passes `inc_width` as true (`ramp.cpp:545`). The tubes are drawn
/// at the spacing; the channel the ball runs in is the spacing plus twenty.
///
/// Leaving the twenty out is not a small error. A two-wire ramp with its wires
/// thirty-eight apart draws correctly and gets a thirty-eight unit channel
/// with a wall on each side, and a ball is fifty across: it wedges between the
/// walls and stops dead, anywhere along the ramp, for ever.
pub fn collision_path(ramp: &Ramp, accuracy: f32) -> Option<Outline> {
    let curve = dragpoint::expand(&dragpoint::from_vpin(&ramp.drag_points), false, accuracy);
    if curve.len() < 2 {
        return None;
    }
    let extra = match ramp.ramp_type {
        RampType::Flat | RampType::OneWire => 0.0,
        _ => 20.0,
    };
    Some(outline(ramp, &curve, edge_width(ramp) + extra))
}

/// How far apart the two edges run: a fixed distance for a wire ramp, and
/// `NAN` for a flat one, where it is interpolated point by point.
fn edge_width(ramp: &Ramp) -> f32 {
    if ramp.ramp_type == RampType::Flat {
        return f32::NAN;
    }
    if ramp.ramp_type == RampType::OneWire {
        // A single wire runs down the center: the "width" is its diameter.
        ramp.wire_diameter
    } else {
        ramp.wire_distance_x
    }
}

pub fn build(ramp: &Ramp, playfield: Bounds) -> Vec<Mesh> {
    if !ramp.is_visible {
        return Vec::new();
    }
    let Some(c) = path(ramp, dragpoint::ACCURACY) else {
        return Vec::new();
    };

    let is_wire = ramp.ramp_type != RampType::Flat;
    if is_wire {
        wires(ramp, &c)
    } else {
        flat(ramp, &c, playfield)
    }
}

/// `Ramp::GetRampVertex`, `ramp.cpp:331`.
///
/// Width and height are interpolated by **how much of the path has been
/// covered**, not by the point's index: a ramp with badly spread control points
/// still climbs evenly.
fn outline(ramp: &Ramp, curve: &[Point], fixed_width: f32) -> Outline {
    let n = curve.len();

    // Total length, summing the segments in the plane.
    let total: f32 = (0..n - 1)
        .map(|i| (curve[i].pos.truncate() - curve[i + 1].pos.truncate()).length())
        .sum::<f32>()
        .max(1e-6);

    let mut c = Outline {
        center: Vec::with_capacity(n),
        left: Vec::with_capacity(n),
        right: Vec::with_capacity(n),
        height: Vec::with_capacity(n),
        ratio: Vec::with_capacity(n),
    };

    let mut covered = 0.0f32;
    for i in 0..n {
        // The ends do not wrap around: a ramp is not a loop.
        let prev = curve[if i > 0 { i - 1 } else { i }].pos.truncate();
        let next = curve[if i < n - 1 { i + 1 } else { i }].pos.truncate();
        let mid = curve[i].pos.truncate();

        let normal = normal_at(prev, mid, next, i, n);

        covered += (prev - mid).length();
        let fraction = covered / total;

        let width = if fixed_width.is_nan() {
            fraction * (ramp.width_top - ramp.width_bottom) + ramp.width_bottom
        } else {
            fixed_width
        };

        c.center.push(mid);
        // Which side is which, and it is the normal that decides: the original
        // puts `+vnormal` into the first half of its vertex array
        // (`ramp.cpp:484`) and then calls that half the **right** wall
        // (`ramp.cpp:585`, `pv1 = &rgvLocal[i]`), with the `-vnormal` half the
        // left (`ramp.cpp:605`, `rgvLocal[cvertex + i]`).
        //
        // Naming them the other way round is invisible on a ramp walled the
        // same on both sides, which is most of them, and puts the wall on the
        // wrong side of every ramp that is not — a one-wire ramp with a single
        // guide, or either of the three-wire types, whose two heights differ by
        // fifty units.
        c.right.push(mid + normal * (width * 0.5));
        c.left.push(mid - normal * (width * 0.5));
        c.height.push(
            curve[i].pos.z + fraction * (ramp.height_top - ramp.height_bottom) + ramp.height_bottom,
        );
        c.ratio.push(1.0 - fraction);
    }
    c
}

/// The outward normal at a point of the path.
///
/// At the ends the one of the single segment there is used; in the middle, the
/// intersection of the two offset lines, which is what keeps the edge from
/// narrowing on tight curves (`ramp.cpp:387-441`).
fn normal_at(prev: Vec2, mid: Vec2, next: Vec2, i: usize, n: usize) -> Vec2 {
    let v1 = Vec2::new(prev.y - mid.y, mid.x - prev.x);
    let v2 = Vec2::new(mid.y - next.y, next.x - mid.x);

    if i == n - 1 {
        return v1.normalize_or_zero();
    }
    if i == 0 {
        return v2.normalize_or_zero();
    }

    let v1 = v1.normalize_or_zero();
    let v2 = v2.normalize_or_zero();
    if (v1.x - v2.x).abs() < 1e-4 && (v1.y - v2.y).abs() < 1e-4 {
        return v1;
    }

    // Two lines offset along their normals; their intersection gives the
    // vertex's normal.
    let (a, b) = (prev.y - mid.y, mid.x - prev.x);
    let cc = a * (v1.x - prev.x) + b * (v1.y - prev.y);
    // `ramp.cpp:428`: `D = vnext.y - vmiddle.y`, not the other way round. With
    // the sign flipped the determinant is wrong and the intersection of two
    // nearly-parallel edges — which is most of a long straight run, once the
    // curve has been subdivided — lands thousands of units away. The ramp's
    // edge then flies off the table, and with it the floor the ball rides on.
    let (d, e) = (next.y - mid.y, mid.x - next.x);
    let f = d * (v2.x - next.x) + e * (v2.y - next.y);

    let det = a * e - b * d;
    if det == 0.0 {
        return v1;
    }
    let inv = 1.0 / det;
    Vec2::new(
        mid.x - (b * f - e * cc) * inv,
        mid.y - (cc * d - a * f) * inv,
    )
}

/// Flat ramp: the floor plus the two walls.
fn flat(ramp: &Ramp, c: &Outline, playfield: Bounds) -> Vec<Mesh> {
    let world_uv = ramp.image_alignment
        == vpin::vpx::gameitem::ramp_image_alignment::RampImageAlignment::World;
    let inv_width = 1.0 / (playfield.max.x - playfield.min.x).max(1e-6);
    let inv_length = 1.0 / (playfield.max.y - playfield.min.y).max(1e-6);

    // Floor: one strip between the two edges, and the **right** edge first.
    //
    // That order is the texture's, not the geometry's. `strip` gives its first
    // edge `u = 1` and its second `u = 0`, which is the original's own
    // assignment (`ramp.cpp:2165`) — and the original hands it
    // `rgvLocal[i]`, the `+vnormal` side, which is the one we call `right`
    // (`ramp.cpp:2148`, and see the note in `outline`). Passing `left` first
    // put `u = 1` on the wrong edge and printed every ramp's artwork
    // backwards.
    //
    // Invisible on the ramps most tables have, whose textures are a plain
    // surface or symmetric enough not to say. Not invisible on The Sopranos,
    // whose apron is a two-triangle ramp with the whole apron printed on it:
    // it came out mirrored, and beside the flasher carrying the same artwork
    // the right way round it read as a second, backwards apron.
    let mut meshes = vec![strip(
        format!("{} (floor)", ramp.name),
        c,
        |i| (c.right[i], c.height[i]),
        |i| (c.left[i], c.height[i]),
        world_uv,
        (inv_width, inv_length),
        ramp,
    )];

    // Walls: the same strip, but from the floor's height to the wall's.
    if ramp.right_wall_height_visible > 0.0 {
        meshes.push(strip(
            format!("{} (right wall)", ramp.name),
            c,
            |i| (c.right[i], c.height[i]),
            |i| (c.right[i], c.height[i] + ramp.right_wall_height_visible),
            world_uv,
            (inv_width, inv_length),
            ramp,
        ));
    }
    if ramp.left_wall_height_visible > 0.0 {
        meshes.push(strip(
            format!("{} (left wall)", ramp.name),
            c,
            |i| (c.left[i], c.height[i]),
            |i| (c.left[i], c.height[i] + ramp.left_wall_height_visible),
            world_uv,
            (inv_width, inv_length),
            ramp,
        ));
    }
    meshes
}

/// A strip of quads between two edges running along the path.
fn strip(
    name: String,
    c: &Outline,
    edge_a: impl Fn(usize) -> (Vec2, f32),
    edge_b: impl Fn(usize) -> (Vec2, f32),
    world_uv: bool,
    inv: (f32, f32),
    ramp: &Ramp,
) -> Mesh {
    let n = c.center.len();
    let mut vertices = Vec::with_capacity(n * 2);
    let mut indices = Vec::with_capacity((n - 1) * 6);

    for i in 0..n {
        let (pa, za) = edge_a(i);
        let (pb, zb) = edge_b(i);
        let uv = |p: Vec2, u: f32| {
            if world_uv {
                [p.x * inv.0, p.y * inv.1]
            } else {
                [u, c.ratio[i]]
            }
        };
        vertices.push(Vertex {
            pos: [pa.x, pa.y, za],
            normal: [0.0; 3],
            uv: uv(pa, 1.0),
        });
        vertices.push(Vertex {
            pos: [pb.x, pb.y, zb],
            normal: [0.0; 3],
            uv: uv(pb, 0.0),
        });

        if i + 1 < n {
            let b = (i * 2) as u32;
            indices.extend([b, b + 1, b + 3, b, b + 3, b + 2]);
        }
    }

    crate::rubber::normals(&mut vertices, &indices);
    Mesh {
        name,
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        image: ramp.image.clone(),
        material: ramp.material.clone(),
        visible: true,
        // The original picks the sampler from the ramp's own image alignment
        // (`ramp.cpp:895`): an image wrapped *along* the ramp is clamped, and
        // one tiled by world coordinates repeats. Getting it wrong is not
        // subtle — The Sopranos' apron is a two-triangle ramp with the apron
        // printed on it, and repeating that laid a second apron, mirrored,
        // across the cabinet beside the real one.
        clamp: !world_uv,
        scenery: false,
        kind: MeshKind::Ramp,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    }
}

/// Wire ramp: one or several tubes following the path.
///
/// How many and at what height comes from `ramp.cpp:2002-2070`. The `+3` on the
/// upper tubes is written just like that, literally, in the original.
fn wires(ramp: &Ramp, c: &Outline) -> Vec<Mesh> {
    let radius = ramp.wire_diameter * 0.5;
    let half_y = ramp.wire_distance_y * 0.5;

    // (path, offset in z)
    let paths: Vec<(&[Vec2], f32)> = match ramp.ramp_type {
        RampType::OneWire => vec![(&c.center[..], 0.0)],
        RampType::TwoWire => vec![(&c.left[..], 3.0), (&c.right[..], 3.0)],
        RampType::FourWire => vec![
            (&c.left[..], half_y),
            (&c.right[..], half_y),
            (&c.left[..], 3.0),
            (&c.right[..], 3.0),
        ],
        RampType::ThreeWireLeft => vec![
            (&c.left[..], half_y),
            (&c.left[..], 3.0),
            (&c.right[..], 3.0),
        ],
        RampType::ThreeWireRight => vec![
            (&c.right[..], half_y),
            (&c.left[..], 3.0),
            (&c.right[..], 3.0),
        ],
        RampType::Flat => Vec::new(),
    };

    paths
        .into_iter()
        .enumerate()
        .filter_map(|(k, (path, dz))| {
            let (mut vertices, indices) = tube(path, &c.height, radius, dz);
            if indices.is_empty() {
                return None;
            }
            crate::rubber::normals(&mut vertices, &indices);
            Some(Mesh {
                name: format!("{} (wire {})", ramp.name, k + 1),
                vertices,
                indices,
                transform: Mat4::IDENTITY,
                image: ramp.image.clone(),
                material: ramp.material.clone(),
                visible: true,
                clamp: false,
                scenery: false,
                kind: MeshKind::Ramp,
                additive: None,
                depth_bias: 0.0,
                disable_lighting: 0.0,
            })
        })
        .collect()
}

/// `Ramp::CreateWire`, `ramp.cpp:1017`.
///
/// Same as the rubbers' tube except for two things: the tangent includes the
/// **height**, because a ramp climbs, and the last ring reuses the previous
/// one's tangent — otherwise the wire ends one control point too early, as the
/// original's comment says.
fn tube(path: &[Vec2], height: &[f32], radius: f32, dz: f32) -> (Vec<Vertex>, Vec<u32>) {
    let rings = path.len();
    if rings < 2 {
        return (Vec::new(), Vec::new());
    }

    let mut vertices = Vec::with_capacity(rings * SEGMENTS);
    let mut prev_binormal = Vec3::ZERO;
    let inv_rings = 1.0 / rings as f32;
    let inv_segments = 1.0 / SEGMENTS as f32;

    for i in 0..rings {
        let i2 = if i == rings - 1 { i } else { i + 1 };
        let z = height[i];

        let tangent = if i == rings - 1 {
            let p = path[i] - path[i - 1];
            Vec3::new(p.x, p.y, height[i2] - z)
        } else {
            Vec3::new(
                path[i2].x - path[i].x,
                path[i2].y - path[i].y,
                height[i2] - z,
            )
        };

        let (normal, binormal) = if i == 0 {
            let up = Vec3::new(
                path[i2].x + path[i].x,
                path[i2].y + path[i].y,
                height[i2] - z,
            );
            let n = tangent.cross(up);
            (n, tangent.cross(n))
        } else {
            let n = prev_binormal.cross(tangent);
            (n, tangent.cross(n))
        };
        let normal = normal.normalize_or_zero();
        prev_binormal = binormal.normalize_or_zero();

        let axis = tangent.normalize_or_zero();
        let u = i as f32 * inv_rings;
        for j in 0..SEGMENTS {
            let v = (j as f32 + u) * inv_segments;
            let angle = (j as f32 * 360.0 * inv_segments).to_radians();
            let radial = if axis == Vec3::ZERO {
                normal * radius
            } else {
                Quat::from_axis_angle(axis, angle) * normal * radius
            };
            vertices.push(Vertex {
                pos: [
                    path[i].x + radial.x,
                    path[i].y + radial.y,
                    z + dz + radial.z,
                ],
                normal: [0.0; 3],
                uv: [u, v],
            });
        }
    }

    // Unlike a rubber, a wire **does not close**: there is one ring of quads
    // fewer than there are rings.
    let mut indices = Vec::with_capacity((rings - 1) * SEGMENTS * 6);
    for i in 0..rings - 1 {
        for j in 0..SEGMENTS {
            let next = if j != SEGMENTS - 1 { j + 1 } else { 0 };
            let q0 = (i * SEGMENTS + j) as u32;
            let q1 = (i * SEGMENTS + next) as u32;
            let q2 = ((i + 1) * SEGMENTS + j) as u32;
            let q3 = ((i + 1) * SEGMENTS + next) as u32;
            indices.extend([q0, q1, q2, q3, q2, q1]);
        }
    }

    (vertices, indices)
}
