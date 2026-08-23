//! Tests of the control point expansion and of the triangulation.

use vpw_math::{Vec2, Vec3};
use vpw_table::dragpoint::{
    ACCURACY, ControlPoint, HIT_SHAPE_DETAIL_LEVEL, accuracy_for, collision_accuracy, expand,
};
use vpw_table::triangulate;

fn point(x: f32, y: f32, smooth: bool) -> ControlPoint {
    ControlPoint {
        pos: Vec3::new(x, y, 0.0),
        smooth,
    }
}

#[test]
fn a_corner_generates_no_extra_points() {
    // With every point marked as a corner, the curve is not subdivided: the
    // outline has to come out exactly as it was, one vertex per control point.
    let square = [
        point(0.0, 0.0, false),
        point(100.0, 0.0, false),
        point(100.0, 100.0, false),
        point(0.0, 100.0, false),
    ];
    let out = expand(&square, true, ACCURACY);
    assert_eq!(out.len(), 4, "a square of corners is four vertices");
    assert!(
        out.iter().all(|p| p.control),
        "all of them are the author's points"
    );
}

#[test]
fn a_smooth_point_rounds_the_corner() {
    // Marking the points as smooth, the spline passes through curving and
    // intermediate vertices are needed to approximate it.
    let corners = [
        point(0.0, 0.0, false),
        point(100.0, 0.0, false),
        point(100.0, 100.0, false),
        point(0.0, 100.0, false),
    ];
    let smooth = [
        point(0.0, 0.0, true),
        point(100.0, 0.0, true),
        point(100.0, 100.0, true),
        point(0.0, 100.0, true),
    ];
    let hard = expand(&corners, true, ACCURACY);
    let curved = expand(&smooth, true, ACCURACY);
    assert!(
        curved.len() > hard.len() * 4,
        "the smooth version has to generate many more points: {} vs {}",
        curved.len(),
        hard.len()
    );
    assert!(
        curved.iter().any(|p| !p.control),
        "the intermediate points are not the author's"
    );
}

#[test]
fn an_open_path_ends_on_the_last_point() {
    // On an open path — a ramp — the last point is added by the function by
    // hand, because the loop only emits the first point of each stretch.
    let path = [
        point(0.0, 0.0, false),
        point(50.0, 0.0, false),
        point(100.0, 0.0, false),
    ];
    let out = expand(&path, false, ACCURACY);
    let last = out.last().unwrap();
    assert!(
        (last.pos - Vec3::new(100.0, 0.0, 0.0)).length() < 1e-3,
        "it has to end on the last control point; it gave {:?}",
        last.pos
    );
}

#[test]
fn repeated_points_do_not_break_anything() {
    let with_repeat = [
        point(0.0, 0.0, false),
        point(0.0, 0.0, false),
        point(100.0, 0.0, false),
        point(100.0, 100.0, false),
    ];
    let out = expand(&with_repeat, true, ACCURACY);
    assert!(!out.is_empty());
    assert!(out.iter().all(|p| p.pos.is_finite()), "no NaNs");
}

#[test]
fn the_height_is_interpolated_along_the_stretch() {
    // The original builds the spline **only with x and y** (`MeshUtils.h:76`),
    // so the height goes linearly between the two ends of the stretch.
    let ramp = [
        ControlPoint {
            pos: Vec3::new(0.0, 0.0, 0.0),
            smooth: false,
        },
        ControlPoint {
            pos: Vec3::new(200.0, 0.0, 100.0),
            smooth: false,
        },
    ];
    let out = expand(&ramp, false, ACCURACY);
    assert!((out[0].pos.z - 0.0).abs() < 1e-3);
    assert!(
        (out.last().unwrap().pos.z - 100.0).abs() < 1e-3,
        "it has to reach the final height; it gave {}",
        out.last().unwrap().pos.z
    );
}

#[test]
fn a_square_splits_into_two_triangles() {
    let square = [
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ];
    assert_eq!(triangulate::polygon(&square).len(), 6);
}

#[test]
fn the_winding_of_the_outline_does_not_matter() {
    let clockwise = [
        Vec2::new(0.0, 10.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(0.0, 0.0),
    ];
    assert_eq!(
        triangulate::polygon(&clockwise).len(),
        6,
        "an outline the other way round has to triangulate just the same"
    );
}

#[test]
fn a_concave_polygon_is_triangulated_completely() {
    // An "L": six vertices, four triangles.
    let ell = [
        Vec2::new(0.0, 0.0),
        Vec2::new(30.0, 0.0),
        Vec2::new(30.0, 10.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(10.0, 30.0),
        Vec2::new(0.0, 30.0),
    ];
    let idx = triangulate::polygon(&ell);
    assert_eq!(idx.len(), 12, "six vertices give four triangles");
    assert!(
        idx.iter().all(|&i| (i as usize) < ell.len()),
        "indices in range"
    );
}

#[test]
fn a_degenerate_polygon_does_not_hang() {
    assert!(triangulate::polygon(&[]).is_empty());
    assert!(triangulate::polygon(&[Vec2::ZERO, Vec2::ONE]).is_empty());
    // All the points on the same line: there is no area to triangulate, but it
    // cannot go round in circles either.
    let collinear = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(3.0, 0.0),
    ];
    let _ = triangulate::polygon(&collinear);
}

#[test]
fn averaged_normals_come_out_unit_length() {
    // Rubbers and wire ramps compute their normals by averaging the faces that
    // touch each vertex, like the original's `ComputeNormals`. What matters is
    // that they come out unit length and point outwards.
    use vpw_table::geometry::Vertex;

    // A pyramid with a square base and the tip on top.
    let mut vertices = vec![
        Vertex {
            pos: [-1.0, -1.0, 0.0],
            normal: [0.0; 3],
            uv: [0.0, 0.0],
        },
        Vertex {
            pos: [1.0, -1.0, 0.0],
            normal: [0.0; 3],
            uv: [0.0, 0.0],
        },
        Vertex {
            pos: [1.0, 1.0, 0.0],
            normal: [0.0; 3],
            uv: [0.0, 0.0],
        },
        Vertex {
            pos: [-1.0, 1.0, 0.0],
            normal: [0.0; 3],
            uv: [0.0, 0.0],
        },
        Vertex {
            pos: [0.0, 0.0, 2.0],
            normal: [0.0; 3],
            uv: [0.0, 0.0],
        },
    ];
    let indices = vec![0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4];

    vpw_table::rubber::normals(&mut vertices, &indices);

    for v in &vertices {
        let n = Vec3::from_array(v.normal);
        assert!((n.length() - 1.0).abs() < 1e-4, "non-unit normal: {n:?}");
    }
    // The tip of the pyramid points straight up.
    let tip = Vec3::from_array(vertices[4].normal);
    assert!(tip.z > 0.9, "the tip has to point upwards; it gave {tip:?}");
}

// ------------------------------------------- drawn fine, collided coarse ---

#[test]
fn a_detail_level_becomes_the_accuracy_the_original_computes() {
    // `ramp.h:173` and `rubber.cpp:162`: 4 * 10^((10 - level) / 1.5). The two
    // numbers that matter are the ends of the range the original actually uses.
    assert!(
        (accuracy_for(10.0) - 4.0).abs() < 1e-3,
        "level ten is the finest"
    );
    assert!(
        (accuracy_for(HIT_SHAPE_DETAIL_LEVEL) - 400.0).abs() < 1e-2,
        "the hit-shape level is four hundred, got {}",
        accuracy_for(HIT_SHAPE_DETAIL_LEVEL)
    );
}

#[test]
fn what_is_drawn_is_finer_than_what_is_collided() {
    // Larger is coarser — the comment beside the original's formula calls 4 the
    // "highest accuracy". Getting the sense of this number backwards would
    // silently swap the two curves, and both would still look plausible.
    assert!(
        collision_accuracy() > ACCURACY,
        "collision {} should be coarser than drawing {ACCURACY}",
        collision_accuracy()
    );

    // And it really does produce fewer points on the same curve.
    let arc = [
        point(0.0, 0.0, true),
        point(200.0, 300.0, true),
        point(400.0, 0.0, true),
    ];
    let fine = expand(&arc, false, ACCURACY).len();
    let coarse = expand(&arc, false, collision_accuracy()).len();
    assert!(
        coarse < fine,
        "the coarse curve has {coarse} points and the fine one {fine}"
    );
}
