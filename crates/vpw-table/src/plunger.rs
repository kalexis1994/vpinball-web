//! The plunger's rod.
//!
//! Port of `Plunger::RenderSetup` (`plunger.cpp:250`) for the `PlungerTypeModern`
//! shape, which is what a table gets when it does not define a custom tip.
//!
//! The plunger was the only piece of the table that had physics and no drawing:
//! `vpw-physics` had been moving it from the start and there was nothing on
//! screen.
//!
//! # How the original builds it
//!
//! A lathe: a list of `(radius, distance from the tip)` points spun around the
//! rod's axis in 24 steps (`modernCoords`, `plunger.cpp:201`). The radius is a
//! fraction of the plunger's nominal half-width; the distance is in table units
//! already. So a wide plunger is a fat one, not a long one.
//!
//! # The detail that matters: the rod **stretches**
//!
//! The obvious reading —the rod slides inside the barrel and only the visible
//! part changes— is wrong, and the original says so in its own geometry. The
//! last lathe point is pinned to `rody`, the back of the rod, which does not
//! move; every other point travels with the tip (`plunger.cpp:509-518`). Pull
//! the plunger and the shaft gets *longer*, it does not slide.
//!
//! Visual Pinball can afford that because it pre-computes twenty-five whole
//! meshes, one per animation frame, and picks one. We draw with a single matrix
//! per piece, so instead the rod comes in **two pieces**:
//!
//! - the **head**, lathe points 0..=5, rigid, translated by the tip position;
//! - the **shaft**, a plain cylinder, translated to the head's shoulder and
//!   scaled along `y` to reach `rody`.
//!
//! Points 5 and 6 of the descriptor share the same radius, so the seam between
//! the two is exact and invisible. Two matrices reproduce what the original
//! needed twenty-five meshes for.
//!
//! # What is not drawn
//!
//! The **coil spring** behind the rod, which the original lathes from the
//! table's `springLoops` / `springGauge` / `springDiam`. It is a framing call,
//! not laziness: from the play camera the spring sits on the far side of the
//! cabinet wall, and all the work it costs shows up in zero pixels.
//!
//! There is also no fallback material. A plunger that declares none gets the
//! engine's default, flat grey, exactly like any other part of the table that
//! declares none — the ball is the one piece that brings its own, because the
//! ball is the one piece that is in no `.vpx`.

use crate::geometry::{Mesh, MeshKind, Vertex};
use std::f32::consts::TAU;
use vpw_math::{Mat4, Vec3};

/// How many vertices each ring of the lathe has (`circlePoints`,
/// `plunger.cpp:262`).
const CIRCLE_POINTS: usize = 24;

/// One point of the lathe (`PlungerCoord`, `plunger.h`).
///
/// The two coordinates are in **different units**, which is easy to miss and
/// ugly when you do: `r` is a fraction of the plunger's nominal half-width, so
/// a wide plunger is a fat one, while `y` is in table units already, so a wide
/// plunger is not a long one. A real plunger works exactly like that — they all
/// stick out the same amount whatever the barrel.
struct Coord {
    /// Radius at this point, as a fraction of the nominal half-width.
    r: f32,
    /// Position along the axis, in table units, with the tip at 0.
    y: f32,
    /// Texture `v` of the ring.
    tv: f32,
    /// Profile normal: `nx` is the radial component, `ny` the axial one.
    nx: f32,
    ny: f32,
}

/// `modernCoords` (`plunger.cpp:201`), the modern plunger added by rascal.
///
/// Points 0..3 are the tip, 4 is the ring that stops it against the barrel, and
/// 5..6 are the shaft. Note that 3 and 4 share a `y`: that is a flat step, a
/// disc, and it is why the normals matter more than the radii here.
const MODERN: [Coord; 7] = [
    Coord {
        r: 0.20,
        y: 0.0,
        tv: 0.00,
        nx: 1.0,
        ny: 0.0,
    },
    Coord {
        r: 0.30,
        y: 3.0,
        tv: 0.11,
        nx: 1.0,
        ny: 0.0,
    },
    Coord {
        r: 0.35,
        y: 5.0,
        tv: 0.14,
        nx: 1.0,
        ny: 0.0,
    },
    Coord {
        r: 0.35,
        y: 23.0,
        tv: 0.19,
        nx: 1.0,
        ny: 0.0,
    },
    Coord {
        r: 0.45,
        y: 23.0,
        tv: 0.21,
        nx: 0.8,
        ny: 0.0,
    },
    Coord {
        r: 0.25,
        y: 24.0,
        tv: 0.25,
        nx: 0.3,
        ny: 0.0,
    },
    Coord {
        r: 0.25,
        y: 100.0,
        tv: 1.00,
        nx: 0.3,
        ny: 0.0,
    },
];

/// Where the head ends and the shaft begins: the index of the last lathe point
/// that belongs to the head.
const SHOULDER: usize = 5;

/// How far along the axis the shoulder sits, in table units.
pub const SHOULDER_Y: f32 = MODERN[SHOULDER].y;

/// The head: the tip, its ring and the first stretch of shaft.
///
/// Local frame with the tip at the origin, pointing towards `+y` — the same
/// convention as the descriptor. `half_width` is the `.vpx`'s `width`, which is
/// a half-width and not a width.
pub fn head_mesh(name: &str, half_width: f32, material: String) -> Mesh {
    let (vertices, indices) = lathe(&MODERN[..=SHOULDER], half_width);
    Mesh {
        name: name.into(),
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        image: String::new(),
        material,
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Builtin,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    }
}

/// The shaft: a cylinder one unit long, to be scaled along `y`.
///
/// It is a unit so that the animation can stretch it with a plain scale. Its
/// radius is that of lathe point 5, which is the same as point 6's — that is
/// what makes the seam with the head exact.
pub fn shaft_mesh(name: &str, half_width: f32, material: String) -> Mesh {
    let r = MODERN[SHOULDER].r;
    let cylinder = [
        Coord {
            r,
            y: 0.0,
            tv: MODERN[SHOULDER].tv,
            nx: MODERN[SHOULDER].nx,
            ny: 0.0,
        },
        Coord {
            r,
            y: 1.0,
            tv: MODERN[6].tv,
            nx: MODERN[6].nx,
            ny: 0.0,
        },
    ];
    let (vertices, indices) = lathe(&cylinder, half_width);
    Mesh {
        name: name.into(),
        vertices,
        indices,
        transform: Mat4::IDENTITY,
        image: String::new(),
        material,
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Builtin,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    }
}

/// Spins a profile around the `y` axis (`plunger.cpp:498-547`).
///
/// The original walks the circle first and the profile second, so its vertices
/// come out column-major; the order does not matter as long as the indices
/// agree, and going row-major here makes the index arithmetic obvious.
///
/// The last column of vertices is a duplicate of the first, with `u = 1`
/// instead of `u = 0`. Without it the triangle that closes the loop interpolates
/// `u` from 0.96 back to 0 and squeezes the whole texture into one strip.
fn lathe(profile: &[Coord], half_width: f32) -> (Vec<Vertex>, Vec<u32>) {
    let w = half_width;
    let rows = profile.len();
    let mut vertices = Vec::with_capacity(rows * (CIRCLE_POINTS + 1));

    for c in profile {
        for i in 0..=CIRCLE_POINTS {
            let angle = i as f32 / CIRCLE_POINTS as f32 * TAU;
            let (sn, cs) = angle.sin_cos();

            // `plunger.cpp:539-541`, minus the table position: this mesh stays
            // in a local frame and the placement lives in the matrix. Only the
            // radius scales with the width; `y` is already in table units.
            let pos = Vec3::new(c.r * sn * w, c.y, c.r * cs * w);

            // The original writes `nz = -c->nx * cs` while the radial direction
            // is `(sn, 0, +cs)`, i.e. a normal mirrored in z. It does not show
            // there because the shader normalizes and flips whatever faces away
            // from the eye, but it is wrong, so this uses `+cs`.
            let mut normal = Vec3::new(c.nx * sn, c.ny, c.nx * cs).normalize_or_zero();
            if normal == Vec3::ZERO {
                normal = Vec3::Y;
            }

            vertices.push(Vertex {
                pos: pos.to_array(),
                normal: normal.to_array(),
                // The original starts the wrap at 0.51 so that the centreline
                // of the texture lands on the top of the cylinder.
                uv: [0.51 + i as f32 / CIRCLE_POINTS as f32, c.tv],
            });
        }
    }

    let row = (CIRCLE_POINTS + 1) as u32;
    let mut indices = Vec::with_capacity((rows - 1) * CIRCLE_POINTS * 6);
    for j in 0..(rows - 1) as u32 {
        for i in 0..CIRCLE_POINTS as u32 {
            let a = j * row + i;
            let b = a + row;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpw_math::Vec2;

    const W: f32 = 25.0;

    fn radius_at(m: &Mesh, y: f32) -> f32 {
        m.vertices
            .iter()
            .filter(|v| (v.pos[1] - y).abs() < 0.01)
            .map(|v| Vec2::new(v.pos[0], v.pos[2]).length())
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_head_follows_the_original_descriptor() {
        let m = head_mesh("Plunger", W, String::new());
        assert_eq!(m.vertices.len(), 6 * (CIRCLE_POINTS + 1));
        // The tip is not a point: the original starts the lathe at r = 0.20,
        // so the very front of the plunger is a small flat disc.
        assert!((radius_at(&m, 0.0) - 0.20 * W).abs() < 1e-3);
        assert!((radius_at(&m, 24.0) - 0.25 * W).abs() < 1e-3);
        // The ring at y = 23 is the fattest thing on the plunger.
        assert!((radius_at(&m, 23.0) - 0.45 * W).abs() < 1e-3);
    }

    #[test]
    fn only_the_radius_scales_with_the_width() {
        // `r` is a fraction of the half-width and `y` is already in table
        // units, so a plunger twice as wide is twice as fat and exactly as
        // long. Getting this backwards makes wide plungers grow out of the
        // shooter lane.
        let narrow = head_mesh("a", W, String::new());
        let wide = head_mesh("b", W * 2.0, String::new());
        let length = |m: &Mesh| m.vertices.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!((length(&narrow) - length(&wide)).abs() < 1e-4);
        assert!((radius_at(&wide, 23.0) - 2.0 * radius_at(&narrow, 23.0)).abs() < 1e-3);
    }

    #[test]
    fn the_ring_is_fatter_than_the_shaft() {
        let m = head_mesh("Plunger", W, String::new());
        assert!(radius_at(&m, 23.0) > radius_at(&m, 24.0));
    }

    #[test]
    fn the_shaft_is_one_unit_long_and_matches_the_head() {
        let head = head_mesh("Plunger", W, String::new());
        let shaft = shaft_mesh("Plunger shaft", W, String::new());
        let far = shaft
            .vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::MIN, f32::max);
        assert!(
            (far - 1.0).abs() < 1e-4,
            "the shaft has to be a unit long: {far}"
        );
        // Same radius at the seam, so the join is invisible.
        assert!((radius_at(&shaft, 0.0) - radius_at(&head, 24.0)).abs() < 1e-3);
    }

    #[test]
    fn the_shoulder_is_where_the_head_ends() {
        let head = head_mesh("Plunger", W, String::new());
        let furthest = head
            .vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::MIN, f32::max);
        assert!((furthest - SHOULDER_Y).abs() < 1e-3);
    }

    #[test]
    fn the_normals_are_unit_length() {
        for m in [
            head_mesh("a", W, String::new()),
            shaft_mesh("b", W, String::new()),
        ] {
            for v in &m.vertices {
                let n = Vec3::from_array(v.normal).length();
                assert!((n - 1.0).abs() < 1e-4, "unnormalized normal: {n}");
            }
        }
    }

    #[test]
    fn the_normals_point_out_of_the_solid() {
        // Along the shaft the normal has to be purely radial and point away
        // from the axis, or the chrome lights up inside out.
        let m = shaft_mesh("Plunger shaft", W, String::new());
        for v in &m.vertices {
            let radial = Vec2::new(v.pos[0], v.pos[2]).normalize_or_zero();
            let n = Vec2::new(v.normal[0], v.normal[2]).normalize_or_zero();
            assert!(
                radial.dot(n) > 0.99,
                "normal facing inwards: {:?}",
                v.normal
            );
        }
    }

    #[test]
    fn the_seam_does_not_reuse_the_first_column() {
        let m = head_mesh("Plunger", W, String::new());
        let first = m.vertices[0];
        let last = m.vertices[CIRCLE_POINTS];
        assert!((Vec3::from_array(first.pos) - Vec3::from_array(last.pos)).length() < 1e-4);
        assert!((last.uv[0] - first.uv[0] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_indices_fall_inside_the_mesh() {
        for m in [
            head_mesh("a", W, String::new()),
            shaft_mesh("b", W, String::new()),
        ] {
            let n = m.vertices.len() as u32;
            assert!(m.indices.iter().all(|&i| i < n));
            assert_eq!(m.indices.len() % 3, 0);
        }
    }
}
