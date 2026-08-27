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
    // What the front view owes is the head's *display*, not its crown: the
    // strip above the display is blank cabinet, and letting it crop on a
    // wide screen is what buys the closer stance. The display's top is the
    // line that must hold.
    let head_height = backbox.1.z - backbox.0.z;
    let display_top = Vec3::new(
        482.0,
        backbox.0.y,
        backbox.1.z - head_height * vpw_table::backbox::DISPLAY_AREA[1],
    );
    assert!(
        on_screen(&front, display_top, aspect),
        "the front view has to fit the display: {display_top:?}"
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

        // The head's crown may crop off the top of a wide screen — that is
        // the front view's closer stance — but its display band, the part
        // with the score on it, has to land whole.
        let display = vpw_table::backbox::DISPLAY_AREA;
        let display_top = top + height * display[1];
        let display_bottom = top + height * (display[1] + display[3]);
        assert!(
            (0.0..1.0).contains(&left),
            "at {aspect:.2} it landed off screen: {left} {top}"
        );
        assert!(
            left + width <= 1.0 && (0.0..=1.0).contains(&display_top) && display_bottom <= 1.0,
            "and the display has to fit: {left}+{width}, display {display_top}..{display_bottom}"
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

// ---------------------------------------------------------------------------
// The overhead view as a sheet of glass
// ---------------------------------------------------------------------------
//
// The brief for this view is one sentence: the screen is the acrylic over the
// playfield. Everything below follows from taking that literally rather than
// as a figure of speech. If the screen is the glass then every point of it
// shows the point of the table directly underneath, the edges of the table sit
// on the edges of the screen, and the only thing between them is the aspect
// ratio the table happens to have.

/// Where the playfield's own edges land, as a fraction of half the screen.
/// One means touching.
fn playfield_fill(c: &Camera, playfield: (Vec3, Vec3), aspect: f32) -> (f32, f32) {
    let (min, max) = playfield;
    let vp = c.view_projection(aspect);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for cx in [min.x, max.x] {
        for cy in [min.y, max.y] {
            let clip = vp * Vec3::new(cx, cy, 0.0).extend(1.0);
            x = x.max((clip.x / clip.w).abs());
            y = y.max((clip.y / clip.w).abs());
        }
    }
    (x, y)
}

#[test]
fn the_overhead_view_does_not_converge() {
    // A perspective camera makes a thing nearer to it bigger. Under glass
    // nothing does that: a post 200 units tall covers exactly its own
    // footprint, which is why you can tell from a photograph taken through the
    // glass where a ball will actually go.
    let (playfield, backbox) = machine();
    let c = Camera::for_view(View::Overhead, playfield, backbox, 0.4615);

    // The same horizontal span, once on the playfield and once well above it.
    let span = |z: f32| {
        let a = to_screen(&c, Vec3::new(300.0, 1000.0, z), 0.4615).0;
        let b = to_screen(&c, Vec3::new(600.0, 1000.0, z), 0.4615).0;
        b - a
    };
    let (low, high) = (span(0.0), span(220.0));
    assert!(
        (low - high).abs() < 1e-4,
        "the same 300 units measured {low} on the playfield and {high} at the top \
         of a wall; an overhead view that magnifies what it is nearer to is a \
         photograph of a table, not a sheet of glass over one"
    );
}

#[test]
fn the_playfield_reaches_the_edges_of_a_phone() {
    // The point of the whole thing. A table is about twice as long as it is
    // wide and so is a phone held upright, so the length is what runs out
    // first and the length is what has to touch.
    let (playfield, backbox) = machine();
    for aspect in [0.4615, 0.5, 0.5625] {
        let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
        let (x, y) = playfield_fill(&c, playfield, aspect);
        assert!(
            y > 0.999,
            "at {aspect} the table stops {:.1}% short of the top and bottom",
            (1.0 - y) * 100.0
        );
        // And the width is the table's own shape, not slack the camera left.
        let table = (playfield.1.x - playfield.0.x) / (playfield.1.y - playfield.0.y);
        let expected = table / aspect;
        assert!(
            (x - expected).abs() < 0.01,
            "at {aspect} the table fills {x:.3} of the width where its own \
             proportions say {expected:.3}; the difference is wasted screen"
        );
    }
}

#[test]
fn which_pair_of_edges_is_touched_is_the_table_against_the_screen() {
    // Whichever of the two shapes is the *taller* decides. F-14 is 0.446 wide
    // to long, so on anything wider than that — a phone upright at 0.462, a
    // desktop at 1.78 — it is the length that runs out and the top and bottom
    // that are touched, with the slack going to the sides. Only a screen
    // narrower than the table itself swaps it over, and then the sides are
    // touched instead. There is no third case and neither one crops.
    let (playfield, backbox) = machine();
    let table = (playfield.1.x - playfield.0.x) / (playfield.1.y - playfield.0.y);

    for aspect in [0.4615, 1.0, 16.0 / 9.0] {
        let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
        let (x, y) = playfield_fill(&c, playfield, aspect);
        assert!(
            y > 0.999,
            "at {aspect} the table stops short of the top: {y}"
        );
        assert!(x <= 1.0001, "at {aspect} the table runs off the sides: {x}");
    }

    // Narrower than the table. Nothing sensible is this shape, which is the
    // reason to check it: the arithmetic has to hold at both ends or it is
    // holding by luck in the middle.
    let aspect = table * 0.75;
    let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
    let (x, y) = playfield_fill(&c, playfield, aspect);
    assert!(x > 0.999, "the table stops short of the sides: {x}");
    assert!(y <= 1.0001, "the table runs off the top and bottom: {y}");
}

#[test]
fn nothing_standing_on_the_playfield_is_cropped() {
    // "As close to the edges as possible" is only worth anything with "without
    // cropping" attached. F-14's side walls are 220 units tall and run along
    // the very edge of the table, which is the hardest case there is: under
    // perspective their tops lean outward and fall off the screen.
    let (playfield, backbox) = machine();
    let aspect = 0.4615;
    let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
    for &z in &[0.0, 100.0, 220.0] {
        for &(x, y) in &[
            (playfield.0.x, playfield.0.y),
            (playfield.1.x, playfield.0.y),
            (playfield.0.x, playfield.1.y),
            (playfield.1.x, playfield.1.y),
        ] {
            assert!(
                on_screen(&c, Vec3::new(x, y, z), aspect),
                "the corner ({x}, {y}) at height {z} is off screen"
            );
        }
    }
}

#[test]
fn the_table_is_not_stretched_to_fit() {
    // The one thing that would make every other number here come out perfect
    // and the picture come out wrong.
    let (playfield, backbox) = machine();
    for aspect in [0.4615, 0.75, 1.0, 1.7778] {
        let c = Camera::for_view(View::Overhead, playfield, backbox, aspect);
        // A square drawn on the playfield has to come out square on screen,
        // once the screen's own shape is taken out.
        let o = to_screen(&c, Vec3::new(300.0, 1000.0, 0.0), aspect);
        let dx = to_screen(&c, Vec3::new(600.0, 1000.0, 0.0), aspect).0 - o.0;
        let dy = to_screen(&c, Vec3::new(300.0, 1300.0, 0.0), aspect).1 - o.1;
        // The magnitude, not the sign: looking straight down, the table's `+y`
        // runs up the screen, so `dy` is negative and says nothing about shape.
        let ratio = ((dx * aspect) / dy).abs();
        assert!(
            (ratio - 1.0).abs() < 1e-3,
            "at {aspect} a square came out {ratio:.4} to one"
        );
    }
}

#[test]
fn the_depth_planes_stay_close_enough_to_be_useful() {
    // A long lens or an orthographic one puts the table a long way off, and the
    // defaults of ten and twenty thousand leave everything of interest in the
    // last thousandth of the depth buffer, where coplanar surfaces fight. The
    // framing brackets what it is looking at; this is the check that it did.
    let (playfield, backbox) = machine();
    for view in [View::Front, View::Overhead] {
        let c = Camera::for_view(view, playfield, backbox, 0.4615);
        assert!(
            c.far / c.near < 100.0,
            "{view:?}: near {} to far {} is a ratio of {:.0}, which is most of \
             the depth buffer spent on empty space",
            c.near,
            c.far,
            c.far / c.near
        );
    }
}
