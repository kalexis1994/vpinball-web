//! Tests of the flasher model: the outline, its texture coordinates, and the
//! placement matrix a script's numbers turn into.

use vpin::vpx::gameitem::dragpoint::DragPoint;
use vpin::vpx::gameitem::flasher::{Filter as VpxFilter, Flasher as VpxFlasher, RenderMode};
use vpin::vpx::gameitem::ramp_image_alignment::RampImageAlignment;
use vpw_math::{Vec2, Vec3};
use vpw_table::flasher::{Filter, build};
use vpw_table::geometry::Bounds;

/// A rectangle of control points from (x0, y0) to (x1, y1).
fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> VpxFlasher {
    let corner = |x: f32, y: f32| DragPoint {
        x,
        y,
        z: 0.0,
        smooth: false,
        ..DragPoint::default()
    };
    VpxFlasher {
        name: "F".into(),
        height: 50.0,
        alpha: 100,
        modulate_vs_add: 0.9,
        is_visible: true,
        filter_amount: 100,
        image_alignment: RampImageAlignment::Wrap,
        drag_points: vec![
            corner(x0, y0),
            corner(x0, y1),
            corner(x1, y1),
            corner(x1, y0),
        ],
        ..VpxFlasher::default()
    }
}

fn table() -> Bounds {
    Bounds {
        min: Vec3::ZERO,
        max: Vec3::new(1000.0, 2000.0, 0.0),
    }
}

#[test]
fn the_outline_lies_flat_and_turns_about_its_own_centre() {
    // `flasher.cpp:1186-1192`: the vertices are at z = 0 and the matrix takes
    // them to the box's centre, turns them and lifts them by the height. So
    // the centre of the box ends up exactly at (cx, cy, height) whatever the
    // rotation, and a corner does not.
    let f = build(&rectangle(100.0, 200.0, 300.0, 600.0), table()).expect("a rectangle");
    assert_eq!(f.center, Vec2::new(200.0, 400.0));
    let mut s = f.state.clone();
    s.rot = [-45.0, 30.0, 12.0];
    let m = f.transform(&s);
    let c = m.transform_point3(Vec3::new(200.0, 400.0, 0.0));
    assert!((c - Vec3::new(200.0, 400.0, 50.0)).length() < 1e-3, "{c}");
    let corner = m.transform_point3(Vec3::new(100.0, 200.0, 0.0));
    assert!((corner - Vec3::new(100.0, 200.0, 50.0)).length() > 10.0);
}

#[test]
fn a_flasher_stood_on_its_edge_turns_the_way_the_original_turns() {
    // The Lord of the Rings mounts its backdrop strobes at `RotX = -90`: a
    // rectangle in the plan view stood up on its centre line. Which edge goes
    // up is the sign convention of `SetRotateX` (`matrix.h:300-306`) under
    // the row-vector multiply (`matrix.h:759-765`): `z' = y·sin + z·cos`, the
    // standard right-handed turn — so at minus ninety the **far** edge
    // (small y) rises and the near edge sinks. The first draft of this test
    // asserted the opposite from intuition, and a port that agreed with it
    // would hang every one of those strobes upside down.
    let f = build(&rectangle(0.0, 0.0, 100.0, 100.0), table()).expect("a rectangle");
    let mut s = f.state.clone();
    s.rot = [-90.0, 0.0, 0.0];
    let m = f.transform(&s);
    let near = m.transform_point3(Vec3::new(50.0, 100.0, 0.0));
    let far = m.transform_point3(Vec3::new(50.0, 0.0, 0.0));
    assert!(
        (far.z - 100.0).abs() < 1e-3 && near.z.abs() < 1e-3,
        "near edge {near}, far edge {far}"
    );
    assert!(
        (near.y - 50.0).abs() < 1e-3 && (far.y - 50.0).abs() < 1e-3,
        "it stands over the centre line"
    );
}

#[test]
fn writing_x_moves_the_whole_outline() {
    // `put_X` moves every control point by the difference (`flasher.cpp:563`),
    // and with them the centre the rotations turn about. So the rotated shape
    // must be the same shape, shifted.
    let f = build(&rectangle(100.0, 200.0, 300.0, 600.0), table()).expect("a rectangle");
    let mut s = f.state.clone();
    s.rot = [0.0, 0.0, 30.0];
    let before = f.transform(&s);
    s.x += 40.0;
    s.y -= 10.0;
    let after = f.transform(&s);
    for p in [Vec3::new(100.0, 200.0, 0.0), Vec3::new(300.0, 600.0, 0.0)] {
        let d = after.transform_point3(p) - before.transform_point3(p);
        assert!((d - Vec3::new(40.0, -10.0, 0.0)).length() < 1e-3, "{d}");
    }
}

#[test]
fn wrap_stretches_the_picture_over_the_box_and_world_over_the_table() {
    // `flasher.cpp:1081-1093`.
    let wrap = build(&rectangle(100.0, 200.0, 300.0, 600.0), table()).expect("wrap");
    let uvs: Vec<[f32; 2]> = wrap.vertices.iter().map(|v| v.uv).collect();
    assert!(
        uvs.contains(&[0.0, 0.0]) && uvs.contains(&[1.0, 1.0]),
        "{uvs:?}"
    );

    let mut world = rectangle(100.0, 200.0, 300.0, 600.0);
    world.image_alignment = RampImageAlignment::World;
    let world = build(&world, table()).expect("world");
    let far_corner = world
        .vertices
        .iter()
        .find(|v| v.pos == [300.0, 600.0])
        .expect("the far corner");
    assert_eq!(far_corner.uv, [0.3, 0.3]);
}

#[test]
fn a_display_is_the_bounding_rectangle_with_the_frame_stretched_over_it() {
    // `flasher.cpp:1095-1104`: whatever the outline, a display flasher is
    // four corners and texture coordinates from zero to one.
    let mut f = rectangle(100.0, 200.0, 300.0, 600.0);
    f.render_mode = Some(RenderMode::DMD);
    // A fifth point, to prove the outline itself is not used.
    f.drag_points.insert(
        2,
        DragPoint {
            x: 250.0,
            y: 700.0,
            z: 0.0,
            smooth: false,
            ..DragPoint::default()
        },
    );
    let f = build(&f, table()).expect("a display");
    assert_eq!(f.vertices.len(), 4);
    assert_eq!(f.indices.len(), 6);
    let uvs: Vec<[f32; 2]> = f.vertices.iter().map(|v| v.uv).collect();
    assert_eq!(uvs, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
    // And it sorts ten thousand further back than a picture flasher at the
    // same height (`flasher.cpp:1343`).
    let picture = build(&rectangle(100.0, 200.0, 300.0, 600.0), table()).expect("picture");
    assert_eq!(
        picture.sort_depth(&picture.state) - f.sort_depth(&f.state),
        10000.0
    );
}

#[test]
fn the_old_dmd_flag_still_means_a_display() {
    // `IDMD` from before 10.8.1 (`flasher.cpp:507-513`).
    let mut f = rectangle(0.0, 0.0, 100.0, 100.0);
    f.is_dmd = Some(true);
    let f = build(&f, table()).expect("a display");
    assert_eq!(f.mode, vpw_table::flasher::RenderMode::Dmd);
}

#[test]
fn a_hidden_flasher_is_still_built_and_a_flat_one_is_not() {
    // Hidden ones are the game's to show (`core.vbs:2534`).
    let mut hidden = rectangle(0.0, 0.0, 100.0, 100.0);
    hidden.is_visible = false;
    let hidden = build(&hidden, table()).expect("kept");
    assert!(!hidden.state.visible && !hidden.state.is_drawn());
    // A line has no area to draw.
    assert!(build(&rectangle(0.0, 0.0, 100.0, 0.0), table()).is_none());
    // And a backdrop flasher is a 2D overlay, not part of the table.
    let mut backdrop = rectangle(0.0, 0.0, 100.0, 100.0);
    backdrop.backglass = Some(true);
    assert!(build(&backdrop, table()).is_none());
}

#[test]
fn the_shader_colour_is_alpha_times_scale_out_of_a_hundred() {
    // `flasher.cpp:1241`: convertColor(color, alpha * intensity_scale / 100).
    let f = build(&rectangle(0.0, 0.0, 100.0, 100.0), table()).expect("a rectangle");
    let mut s = f.state.clone();
    s.alpha = 20.0;
    s.intensity_scale = 0.5;
    assert!((s.shader_color()[3] - 0.1).abs() < 1e-6);
    // Black, fully transparent or scaled to nothing: not drawn at all
    // (`flasher.cpp:1179`).
    s.color = [0.0; 3];
    assert!(!s.is_drawn());
    s.color = [1.0; 3];
    s.intensity_scale = 0.0;
    assert!(!s.is_drawn());
}

#[test]
fn the_filter_names_are_the_scripts() {
    // `flasher.cpp:710-726` compares lower-cased, and an unknown name is
    // no change.
    assert_eq!(Filter::from_name("Multiply"), Some(Filter::Multiply));
    assert_eq!(Filter::from_name("SCREEN"), Some(Filter::Screen));
    assert_eq!(Filter::from_name("plaid"), None);
    let mut f = rectangle(0.0, 0.0, 100.0, 100.0);
    f.filter = VpxFilter::Additive;
    let f = build(&f, table()).expect("a rectangle");
    assert_eq!(f.state.filter, Filter::Additive);
    assert_eq!(f.state.filter as u8, 1, "the number the shader compares");
}

#[test]
fn modulate_vs_add_is_kept_off_both_ends() {
    // `flasher.cpp:1242`: zero disables the blend and one "looks not good".
    let f = build(&rectangle(0.0, 0.0, 100.0, 100.0), table()).expect("a rectangle");
    let mut s = f.state.clone();
    s.modulate_vs_add = 1.0;
    assert!(s.clamped_modulate() < 1.0);
    s.modulate_vs_add = 0.0;
    assert!(s.clamped_modulate() > 0.0);
}
