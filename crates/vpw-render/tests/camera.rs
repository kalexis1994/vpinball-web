//! Tests for the camera and the environment map. None of this needs a GPU.

use vpw_math::Vec3;
use vpw_render::Camera;
use vpw_render::camera::View;

/// Projects a point and returns its position on screen, with `x` to the right
/// and `y` up, both in the -1..1 range.
fn to_screen(c: &Camera, p: Vec3, aspect: f32) -> (f32, f32, f32) {
    let clip = c.view_projection(aspect) * p.extend(1.0);
    (clip.x / clip.w, clip.y / clip.w, clip.z / clip.w)
}

fn table_camera() -> Camera {
    // A typical table: 950 wide by 2100 long.
    Camera::framing(Vec3::ZERO, Vec3::new(950.0, 2100.0, 200.0), 0.5625)
}

#[test]
fn the_x_axis_goes_to_the_right_of_the_screen() {
    // This is **the** camera test. Visual Pinball builds its matrices with
    // `MatrixLookAtLH` and `MatrixPerspectiveFovLH` (`math/matrix.h:541,582`),
    // that is, a left-handed system. With a right-handed one the table comes
    // out mirrored and you only notice once some text shows up on screen.
    let c = table_camera();
    let center = c.target;
    let (left, _, _) = to_screen(&c, center - Vec3::new(300.0, 0.0, 0.0), 0.5625);
    let (right, _, _) = to_screen(&c, center + Vec3::new(300.0, 0.0, 0.0), 0.5625);
    assert!(
        left < right,
        "+X has to fall to the right; got left={left} right={right}"
    );
}

#[test]
fn the_back_of_the_table_ends_up_at_the_top() {
    // In Visual Pinball `y` grows towards the player, so small `y` —the back of
    // the table— has to show up at the top of the screen.
    let c = table_camera();
    let center = c.target;
    let (_, back, _) = to_screen(&c, center - Vec3::new(0.0, 600.0, 0.0), 0.5625);
    let (_, front, _) = to_screen(&c, center + Vec3::new(0.0, 600.0, 0.0), 0.5625);
    assert!(
        back > front,
        "the back has to end up at the top; got {back} vs {front}"
    );
}

#[test]
fn depth_goes_from_zero_to_one() {
    // WebGPU expects NDC with z in 0..1, like DirectX. The OpenGL convention
    // gives -1..1 and leaves half the world on the wrong side of the clip
    // plane.
    let c = table_camera();
    let near = c.eye() + (c.target - c.eye()).normalize() * (c.near + 1.0);
    let (_, _, z_near) = to_screen(&c, near, 0.5625);
    let (_, _, z_far) = to_screen(&c, c.target, 0.5625);
    assert!(
        (0.0..=1.0).contains(&z_near),
        "near z out of range: {z_near}"
    );
    assert!((0.0..=1.0).contains(&z_far), "far z out of range: {z_far}");
    assert!(z_near < z_far, "the closer thing has to have the smaller z");
}

#[test]
fn the_eye_ends_up_on_the_players_side_and_above() {
    let c = table_camera();
    let eye = c.eye();
    assert!(
        eye.y > c.target.y,
        "the camera looks from the player's side"
    );
    assert!(eye.z > 0.0, "and from above the playfield");
}

#[test]
fn the_framing_fits_the_whole_table_in_frame() {
    let (min, max) = (Vec3::ZERO, Vec3::new(950.0, 2100.0, 200.0));
    let aspect = 0.5625;
    let c = Camera::framing(min, max, aspect);
    for corner in [
        Vec3::new(min.x, min.y, 0.0),
        Vec3::new(max.x, min.y, 0.0),
        Vec3::new(min.x, max.y, 0.0),
        Vec3::new(max.x, max.y, 0.0),
    ] {
        let (x, y, z) = to_screen(&c, corner, aspect);
        assert!(x.abs() <= 1.0, "corner {corner:?} falls outside on x ({x})");
        assert!(y.abs() <= 1.0, "corner {corner:?} falls outside on y ({y})");
        assert!(
            (0.0..=1.0).contains(&z),
            "corner {corner:?} falls outside on z ({z})"
        );
    }
}

#[test]
fn the_default_environment_map_is_a_valid_image() {
    // It is the `EnvMap.webp` that ships with Visual Pinball. If it does not
    // decode, every table is left without its main light source.
    let img = image::load_from_memory(vpw_render::env::DEFAULT_ENVMAP)
        .expect("the environment map has to decode");
    assert_eq!(
        (img.width(), img.height()),
        (512, 256),
        "equirectangular 2:1"
    );
}

// ----------------------------------------------------------- named views ---

/// A standard body, and the head that stands behind it.
fn machine() -> ((Vec3, Vec3), (Vec3, Vec3)) {
    let playfield = (Vec3::ZERO, Vec3::new(964.0, 2162.0, 0.0));
    let b = vpw_table::backbox::Backbox::for_playfield(vpw_table::geometry::Bounds {
        min: playfield.0,
        max: playfield.1,
    });
    let bounds = b.bounds();
    (playfield, (bounds.min, bounds.max))
}

/// Whether a point lands inside the picture.
fn on_screen(c: &Camera, p: Vec3, aspect: f32) -> bool {
    let clip = c.view_projection(aspect) * p.extend(1.0);
    clip.w > 0.0 && (clip.x / clip.w).abs() <= 1.0 && (clip.y / clip.w).abs() <= 1.0
}

#[test]
fn the_front_view_shows_the_head_and_the_overhead_one_does_not() {
    // The two views differ in what they are *for*. Standing in front you are
    // looking at a machine, and a machine has a head; from straight above you
    // are reading a playfield, and the head is behind you.
    let (playfield, backbox) = machine();
    let aspect = 16.0 / 9.0;

    let front = Camera::for_view(View::Front, playfield, backbox, aspect);
    let top_of_head = Vec3::new(482.0, backbox.0.y, backbox.1.z);
    assert!(
        on_screen(&front, top_of_head, aspect),
        "the front view has to fit the head: {top_of_head:?}"
    );

    // And the overhead one frames the playfield without paying for it.
    let over = Camera::for_view(View::Overhead, playfield, backbox, aspect);
    assert!(
        over.distance < front.distance,
        "framing less should not cost more: {} against {}",
        over.distance,
        front.distance
    );
}

#[test]
fn both_views_fit_the_whole_playfield() {
    // Whatever else a view does, the part you play on has to be in it. All four
    // corners, not the middle: the corners are what a naive framing loses.
    let (playfield, backbox) = machine();
    for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0] {
        for view in [View::Front, View::Overhead] {
            let c = Camera::for_view(view, playfield, backbox, aspect);
            for x in [playfield.0.x, playfield.1.x] {
                for y in [playfield.0.y, playfield.1.y] {
                    let corner = Vec3::new(x, y, 0.0);
                    assert!(
                        on_screen(&c, corner, aspect),
                        "{view:?} at {aspect:.2} lost {corner:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn looking_straight_down_does_not_collapse_the_picture() {
    // Up is the table's `+Z` with the part along the view taken out, and
    // looking straight down there is none left: a look-at built from two
    // parallel vectors collapses and the table disappears. The straight-down
    // case has to name its own answer.
    let (playfield, backbox) = machine();
    let aspect = 16.0 / 9.0;
    let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
    assert_eq!(c.inclination, 90.0, "this is the degenerate one");

    let m = c.view_projection(aspect);
    assert!(
        m.to_cols_array().iter().all(|v| v.is_finite()),
        "the matrix came out with holes in it: {m:?}"
    );

    // And the table is the right way up: the far end above the near one on
    // screen, which is what puts the flippers at the bottom.
    let far = m * Vec3::new(482.0, playfield.0.y, 0.0).extend(1.0);
    let near = m * Vec3::new(482.0, playfield.1.y, 0.0).extend(1.0);
    assert!(
        far.y / far.w > near.y / near.w,
        "the far end should be the higher one on screen"
    );
}

/// The same projection the renderer does to place the score on the head.
fn head_rect(view: View, aspect: f32) -> Option<[f32; 4]> {
    let (playfield, backbox) = machine();
    if !view.shows_backbox() {
        return None;
    }
    let head = vpw_table::backbox::Backbox::for_playfield(vpw_table::geometry::Bounds {
        min: playfield.0,
        max: playfield.1,
    });
    let c = Camera::for_view(view, playfield, backbox, aspect);
    let vp = c.view_projection(aspect);

    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for corner in head.corners() {
        let clip = vp * corner.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let x = (clip.x / clip.w + 1.0) * 0.5;
        let y = (1.0 - clip.y / clip.w) * 0.5;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    Some([min_x, min_y, max_x - min_x, max_y - min_y])
}

#[test]
fn the_head_lands_in_the_upper_part_of_the_front_view() {
    // The score is drawn on the head, and where the head is cannot be guessed
    // from the top of the window: it moves with the aspect ratio, and the two
    // views frame completely different things. So the renderer projects the
    // four corners and says.
    for aspect in [16.0 / 9.0, 4.0 / 3.0, 1.0] {
        let [left, top, width, height] =
            head_rect(View::Front, aspect).expect("the head is in shot from the front");

        assert!(
            (0.0..1.0).contains(&left) && (0.0..1.0).contains(&top),
            "at {aspect:.2} it landed off screen: {left} {top}"
        );
        assert!(
            left + width <= 1.0 && top + height <= 1.0,
            "and has to fit: {left}+{width}, {top}+{height}"
        );
        // Above the middle: it is the head of the machine, not its apron.
        assert!(
            top + height < 0.6,
            "the head should be in the upper part of the shot, and ends at {}",
            top + height
        );
        // And roughly centred, because a machine is symmetric about its length.
        let centre = left + width * 0.5;
        assert!(
            (centre - 0.5).abs() < 0.05,
            "off centre at {aspect:.2}: {centre}"
        );
    }
}

#[test]
fn there_is_no_head_to_put_the_score_on_from_above() {
    // Looking straight down, the head is edge-on and behind the camera's
    // interest entirely. Answering with a rectangle anyway would put the score
    // over the playfield, which is where the ball is.
    assert!(head_rect(View::Overhead, 16.0 / 9.0).is_none());
}
