//! Triangulation of simple polygons.
//!
//! Wall tops are arbitrary closed outlines — concave, curved, sometimes with the
//! vertices going either way round — and they have to be split into triangles.
//!
//! The original uses `PolygonToTriangles` (`math/MeshUtils.h`), which is ear
//! clipping. The same thing goes here: it is the right algorithm for simple
//! polygons and it needs no dependencies.

use vpw_math::Vec2;

/// Twice the signed area. Positive if the outline runs counter-clockwise.
fn double_area(p: &[Vec2]) -> f32 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let a = p[i];
            let b = p[(i + 1) % n];
            a.x * b.y - b.x * a.y
        })
        .sum()
}

fn inside(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> bool {
    let sign = |u: Vec2, v: Vec2, w: Vec2| (v.x - u.x) * (w.y - u.y) - (w.x - u.x) * (v.y - u.y);
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Triangulates the outline and returns indices into the input array.
///
/// Returns empty if the polygon is degenerate.
pub fn polygon(points: &[Vec2]) -> Vec<u32> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }

    // Always working counter-clockwise simplifies the ear test; if the outline
    // came the other way round, it is walked in reverse.
    let ccw = double_area(points) > 0.0;
    let mut remaining: Vec<usize> = if ccw {
        (0..n).collect()
    } else {
        (0..n).rev().collect()
    };

    let mut indices = Vec::with_capacity((n - 2) * 3);
    // Each clipped ear removes a vertex; the counter keeps us from looping
    // forever on an outline that crosses itself.
    let mut no_progress = 0;
    let mut i = 0usize;

    while remaining.len() > 3 {
        if no_progress > remaining.len() {
            break;
        }
        let len = remaining.len();
        let (ia, ib, ic) = (
            remaining[i % len],
            remaining[(i + 1) % len],
            remaining[(i + 2) % len],
        );
        let (a, b, c) = (points[ia], points[ib], points[ic]);

        let convex = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y) > 0.0;
        let free = convex
            && !remaining
                .iter()
                .any(|&j| j != ia && j != ib && j != ic && inside(a, b, c, points[j]));

        if free {
            indices.extend([ia as u32, ib as u32, ic as u32]);
            remaining.remove((i + 1) % len);
            no_progress = 0;
        } else {
            i += 1;
            no_progress += 1;
        }
    }

    if remaining.len() == 3 {
        indices.extend(remaining.iter().map(|&j| j as u32));
    }
    indices
}
