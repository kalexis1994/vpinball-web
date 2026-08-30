//! Camera.
//!
//! The original solves this in `ViewSetup::ComputeMVP` (`ViewSetup.cpp:331`),
//! which drags along three layout modes (desktop, FSS, cabinet), an iterative
//! framing adjustment, stereo and *layback* — a shear that has been there since
//! before the engine was really 3D. Here there is a perspective camera and
//! nothing else; parity with the layout modes comes later.

use vpw_math::{Mat4, Vec3};

/// How the camera turns the world into a picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lens {
    /// A vertical field of view, in degrees. Parallel lines converge, which is
    /// what a photograph of a machine looks like and what Visual Pinball's own
    /// camera always is (`MatrixPerspectiveFovLH`, `math/matrix.h:582`).
    Perspective { fov: f32 },
    /// Half the height of the slab in shot, in VPU. Nothing converges.
    ///
    /// This is the overhead view, and it is not a stylistic choice. The screen
    /// is standing in for the sheet of glass over the playfield, and a sheet of
    /// glass shows you, at every point of it, the point of the table directly
    /// underneath — which is the definition of an orthographic projection. The
    /// perspective a player sees comes from their own eye looking at the phone,
    /// and putting a second one inside the picture is what makes a top-down
    /// view feel like a photograph of a table rather than a table.
    ///
    /// It pays for itself in framing too. Under perspective, anything standing
    /// on the playfield is nearer the eye than the playfield and projects
    /// wider than its footprint, so the camera has to retreat until the tallest
    /// of it fits. F-14's side walls are 220 units tall and run along the very
    /// edge of the table, so there is no framing trick that avoids them: at
    /// forty-five degrees they cost nine per cent of the screen and at twelve
    /// they still cost two. Orthographically a wall projects onto its own
    /// footprint, exactly as it does under real glass, and the cost is zero.
    Orthographic { half_height: f32 },
}

impl Lens {
    /// The field of view, for a perspective lens.
    pub fn fov(self) -> Option<f32> {
        match self {
            Lens::Perspective { fov } => Some(fov),
            Lens::Orthographic { .. } => None,
        }
    }
}

/// Camera looking at the table from the front and above.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// The point it looks at, in VPU.
    pub target: Vec3,
    /// Distance to the target.
    pub distance: f32,
    /// Inclination above the plane of the playfield, in degrees. 90 is looking
    /// straight down; typical values for a desktop table are around 50.
    pub inclination: f32,
    /// Rotation around the table's vertical axis, in degrees.
    pub azimuth: f32,
    /// How it projects. See [`Lens`].
    pub lens: Lens,
    /// The backward skew the original applies before the camera, in degrees.
    ///
    /// See [`Self::layback_shear`]. Zero for everything but the cabinet view.
    pub layback: f32,
    pub near: f32,
    pub far: f32,
}

/// The named places a player looks at the machine from.
///
/// Not a free camera with presets: each of these is a *state*, and the things
/// around it change with it. What the camera frames is only the first of them —
/// where the score goes is the other, and that belongs to whoever is drawing
/// the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Standing in front of the machine: the whole thing, head and all.
    Front,
    /// Straight down on the playfield.
    ///
    /// What a table is easiest to actually *play* from — nothing is foreshortened
    /// and nothing near the flippers is hidden behind a ramp — and what the head
    /// has no place in, so it is left out of the framing entirely.
    Overhead,
    /// Standing at the machine, the way a cabinet stands you: low down, a long
    /// lens, looking along the table rather than at it.
    ///
    /// The original's "fullscreen" view setup, which every table carries and
    /// almost nobody sees, because it is the one a three-screen cabinet uses.
    /// It matters here for a reason that has nothing to do with cabinets: the
    /// tables that model a room or a whole machine around themselves model it
    /// *for this camera*. From the desktop view that scenery hangs over the
    /// playfield; from here you are inside it, which is what its author built.
    ///
    /// It is the one view with layback, and on these tables layback is not a
    /// detail — sixty degrees of it, against zero on the desktop.
    Cabinet,
}

impl View {
    /// How high above the playfield the eye sits, in degrees.
    ///
    /// Forty-five for the front view is not invented: a table carries its own
    /// desktop inclination in the file and F-14's is 44.6. Reading each table's
    /// own is the refinement; this is the value they cluster around.
    fn inclination(self) -> f32 {
        match self {
            // Thirty-four rather than the forty-five the files cluster
            // around: lower is where a person's eyes actually are at a
            // machine, the near field grows and the head recedes, and the
            // picture reads as standing at the cabinet instead of hovering
            // over it. Chosen by photographing the same two tables down a
            // ladder of angles and looking.
            View::Front => 34.0,
            View::Overhead => 90.0,
            // Never reached in practice: the cabinet view is only offered on
            // a table that carries its own numbers. This is the original's
            // own default for the mode.
            View::Cabinet => 22.0,
        }
    }

    /// Vertical field of view, in degrees.
    ///
    /// The front view keeps the forty-five degrees a photograph of a machine
    /// wants. The overhead view is not a photograph of a machine: it is the
    /// sheet of glass over the playfield, and the player is looking through it.
    /// A wide lens is what stops it being that, in two ways that turn out to be
    /// the same way.
    ///
    /// It costs screen. Looking straight down, anything standing on the
    /// playfield is *nearer the eye than the playfield is*, so it projects
    /// outward — and the camera has to retreat until the worst of it fits,
    /// taking the playfield with it. F-14's tallest ramp is 235 units and at
    /// forty-five degrees it overhangs the table's edge by nine per cent, so
    /// nine per cent of the screen goes to holding something that is not the
    /// table. At twelve degrees the same ramp overhangs by two.
    ///
    /// And it costs the illusion. A long lens converges on an orthographic
    /// view, and an orthographic view is what a sheet of glass over a flat
    /// table actually looks like: no vanishing point, no fan of ramps leaning
    /// away from the middle, the edges of the playfield parallel to the edges
    /// of the screen. The framing and the feel improve together, which is the
    /// sign that the number was wrong rather than merely untuned.
    fn lens(self) -> Lens {
        match self {
            // A touch wider than the classic forty-five, to match the closer
            // stance the lower inclination takes.
            View::Front => Lens::Perspective { fov: 47.0 },
            View::Cabinet => Lens::Perspective { fov: 20.0 },
            // The half-height is what the framing works out; this is only a
            // statement that the overhead view does not converge.
            View::Overhead => Lens::Orthographic { half_height: 1.0 },
        }
    }

    /// How much room to leave around what is framed, as a multiplier on the
    /// distance.
    ///
    /// The front view stands back a little, the way somebody photographing a
    /// machine does. The overhead view does not: the whole point is that the
    /// edge of the playfield is the edge of the screen, and a four per cent
    /// gap is four per cent of a phone.
    fn margin(self) -> f32 {
        match self {
            View::Front => 1.04,
            View::Overhead => 1.0,
            // None: a cabinet has no polite distance to stand at. You are at
            // the machine.
            View::Cabinet => 1.0,
        }
    }

    /// Whether the machine's head is part of what this view has to fit.
    pub fn shows_backbox(self) -> bool {
        matches!(self, View::Front | View::Cabinet)
    }
}

/// How much sky the front view leaves over the machine's crown, as a fraction
/// of the head's own height.
///
/// Enough to read as a photograph of a machine rather than a machine pressed
/// against the ceiling, and little enough that the machine still fills the
/// frame.
const HEAD_ROOM: f32 = 0.10;

/// How high above the playfield the overhead view stops looking, in VPU.
///
/// Visual Pinball's own upper window border, which is what a cabinet's glass
/// is measured against: 20 cm above the playfield, or about 371 units
/// (`ViewSetup.h`, and the `WTZ0` field of every table written since 10.8).
/// The glass is the natural ceiling for a view that is looking through it.
const GLASS_CEILING_VPU: f32 = 371.0;

/// The eight corners of an axis-aligned box.
pub fn box_corners(min: Vec3, max: Vec3) -> [Vec3; 8] {
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 2000.0,
            inclination: 50.0,
            azimuth: 0.0,
            lens: Lens::Perspective { fov: 45.0 },
            layback: 0.0,
            near: 10.0,
            far: 20_000.0,
        }
    }
}

impl Camera {
    /// The camera for a named view, framing what that view has to show.
    ///
    /// The two boxes are given separately because the answer differs: the front
    /// view has to fit the machine's head as well as its playfield, and the
    /// overhead view has to fit the playfield and nothing else — including it
    /// would push the table away from the camera to make room for something
    /// that is not even in shot.
    pub fn for_view(
        view: View,
        playfield: (Vec3, Vec3),
        backbox: (Vec3, Vec3),
        aspect: f32,
    ) -> Self {
        Self::for_view_of(view, playfield, backbox, aspect, &[])
    }

    /// The same, fitting the table's **real** geometry rather than one box
    /// around all of it.
    ///
    /// The difference is worth a couple of per cent of a phone screen, and the
    /// reason is worth more than that. Looking straight down, the constraint is
    /// `|x| / (d - z)`: a thing that is both far from the middle *and* tall is
    /// what pushes the camera back. One box around the table claims there is
    /// something as tall as the tallest ramp standing in each of its four
    /// corners, and there never is — F-14's tallest ramp is well inland, and
    /// what is actually at the corners is the flat playfield. Fitting the boxes
    /// the meshes really occupy asks the true question, and the answer is that
    /// the playfield can reach the top and bottom of the screen after all.
    ///
    /// `occupied` is corners of whatever must stay in shot. The playfield's own
    /// box is always included, so passing nothing is the same as framing the
    /// box and passing junk cannot crop the table.
    pub fn for_view_of(
        view: View,
        playfield: (Vec3, Vec3),
        backbox: (Vec3, Vec3),
        aspect: f32,
        occupied: &[Vec3],
    ) -> Self {
        Self::for_authored_view(view, playfield, backbox, aspect, occupied, None)
    }

    /// The same, standing where the table's author put the camera.
    ///
    /// Every `.vpx` carries a view setup per mode and the desktop one is what
    /// a player at a keyboard sees in Visual Pinball. Its author tuned those
    /// numbers until the machine looked the way they wanted, so using them is
    /// the difference between showing the table somebody made and showing our
    /// idea of it — and on a table that models a room around itself it is the
    /// difference between standing inside that room and standing outside it,
    /// looking at the back of its lid.
    ///
    /// Only the two numbers that decide the *look* are taken: how high the eye
    /// sits and how wide the lens is. Where to stand is still worked out here,
    /// by the same fit as always, because our framing has to hold a head the
    /// original never draws in this scene and a screen shape the original
    /// never had. The author's layback and scene scale are read and not
    /// applied; both are projection shears rather than camera moves, and
    /// almost every table leaves them alone.
    ///
    /// The overhead view takes none of it. It is not a view the original has,
    /// so there is nothing authored to honour.
    pub fn for_authored_view(
        view: View,
        playfield: (Vec3, Vec3),
        backbox: (Vec3, Vec3),
        aspect: f32,
        occupied: &[Vec3],
        authored: Option<vpw_table::geometry::AuthoredView>,
    ) -> Self {
        let authored = authored.filter(|_| matches!(view, View::Front | View::Cabinet));
        let (mut min, mut max) = playfield;
        if view.shows_backbox() {
            // The whole head, crown and all, and a little sky over it.
            //
            // This used to frame up to the *display* and let the blank strip
            // above it crop, on the reasoning that blank cabinet is not worth
            // screen. It buys less than it costs: the machine ends up with its
            // backglass jammed against the top edge, and on a squat window —
            // where the width binds and the machine is tall — the display
            // itself starts going with it. A photograph of a machine has air
            // above the machine.
            // Only for the view that photographs the machine. The cabinet
            // view is framed by its author's own numbers, and adding sky to
            // those is second-guessing them.
            let head_room = if matches!(view, View::Front) {
                (backbox.1.z - backbox.0.z) * HEAD_ROOM
            } else {
                0.0
            };
            let mut head_max = backbox.1;
            head_max.z += head_room;
            min = min.min(backbox.0);
            max = max.max(head_max);
        }
        let camera = Self {
            target: (min + max) * 0.5,
            inclination: authored.map_or_else(|| view.inclination(), |a| a.inclination),
            lens: match authored {
                Some(a) => Lens::Perspective { fov: a.fov },
                None => view.lens(),
            },
            // Only where the original applies it. Its desktop stack carries a
            // layback too, but every table leaves that one at zero or a
            // fraction of a degree, and shearing the world for a fraction of a
            // degree is a cost with nothing on the other side of it.
            layback: match (view, authored) {
                (View::Cabinet, Some(a)) => a.layback,
                _ => 0.0,
            },
            ..Default::default()
        };
        let mut corners = box_corners(min, max).to_vec();
        corners.extend_from_slice(occupied);
        let mut camera = Self::fit(camera, &corners, aspect, view.margin());
        if let Some(a) = authored {
            // The author's framing, by the original's own arithmetic. See
            // [`Self::fit_legacy`].
            camera = Self::fit_legacy(camera, &corners, aspect);
            // The author's own nudge, applied to the distance the way the
            // original applies it: `FitCameraToVertices` adds `mViewZ` to
            // whatever the fit worked out (`ViewSetup.cpp:678`), so a negative
            // one steps the camera closer than the fit alone would. That is
            // how a table asks to be seen from inside its own scenery rather
            // than from a polite distance outside it — The Sopranos asks for
            // two hundred units closer, and two hundred units is the
            // difference between looking down its playfield and looking at
            // the underside of the room it is standing in.
            //
            // Never all the way to nothing: a table with a wild offset in it
            // should be framed oddly, not turned inside out.
            camera.distance = (camera.distance + a.offset.z).max(camera.distance * 0.25);
            camera.bracket_depth(&corners);
        }
        if matches!(view, View::Overhead) {
            camera.clip_above(playfield.0.z + GLASS_CEILING_VPU);
        }
        camera
    }

    /// Puts the near plane at the front of what is being framed.
    ///
    /// The front view frames the table, and anything between the eye and the
    /// table is not the table. Normally there is nothing there and the plane
    /// costs nothing; on a machine that models a room around itself there is
    /// the room, and its lid hangs over the playfield from every angle a front
    /// view can take. The Sopranos' reaches 1215 units up over the whole
    /// table — the far two thirds of its playfield are behind it.
    ///
    /// The corners handed over are the ones the original frames on (see
    /// `Scene::legacy_bounds`), so the plane lands at the near edge of the
    /// playfield at the height of its tallest wall or ramp. A machine's head
    /// is at the *far* end and is never cut by this; what is cut is scenery
    /// standing between you and the glass, which is the whole of what it is
    /// for.
    pub fn start_at(&mut self, corners: &[Vec3]) {
        if corners.is_empty() {
            return;
        }
        let eye = self.eye();
        let forward = (self.target - eye).normalize_or_zero();
        let nearest = corners
            .iter()
            .map(|p| (*p - eye).dot(forward))
            .fold(f32::MAX, f32::min);
        if nearest > 1.0 && nearest < self.far {
            self.near = nearest;
        }
    }

    /// Puts the near plane at a height, for the view that looks straight down.
    ///
    /// Looking down from above, anything higher than the glass is between the
    /// camera and the table and can only ever be in the way. Most tables have
    /// nothing up there and never notice; the ones that do are the ones with a
    /// room modelled around the machine — a backdrop, a ceiling, a lit box the
    /// author built for a front view — and from directly overhead you are
    /// inside it, looking at the underside of its lid. The Sopranos is one:
    /// a single primitive reaching 1215 units up, which came out as a dark
    /// smear over half the playfield.
    ///
    /// Nothing on a playfield is anywhere near this high — a tall ramp is two
    /// or three hundred — so the plane costs real geometry nothing, and the
    /// view whose whole idea is that the screen *is* the glass is exactly the
    /// one that should start at it.
    fn clip_above(&mut self, ceiling: f32) {
        // The camera looks straight down, so the distance from the eye to a
        // height is the difference of the two. Only ever pushed further out,
        // never pulled in: the bracket already worked out how much precision
        // the depth buffer needs, and this is not the place to spend it.
        let near = (self.eye().z - ceiling).max(self.near);
        if near < self.far {
            self.near = near;
        }
    }

    /// Frames a whole box: picks target and distance so that it fits.
    ///
    /// Dividing the size by the tangent of the field of view is not enough.
    /// The table is viewed at an angle, so its near edge ends up much closer to
    /// the eye than the far one and grows: with the straightforward arithmetic
    /// the two bottom corners fall out of frame.
    ///
    /// The original has the same problem and solves it by **searching** for the
    /// position by bisection (`ViewSetup.cpp:120-153`). Here we do the same on
    /// the distance, which is simpler and converges in a few steps.
    pub fn framing(min: Vec3, max: Vec3, aspect: f32) -> Self {
        let camera = Self {
            target: (min + max) * 0.5,
            ..Default::default()
        };
        Self::fit(camera, &box_corners(min, max), aspect, 1.04)
    }

    /// The distance search, for a camera that already knows where it is looking
    /// from.
    ///
    /// Split out from [`Camera::framing`] because the search only answers for
    /// the inclination it was run at: move the eye afterwards and the distance
    /// it found is for a shot nobody is taking. A named view picks its angle
    /// first and searches second.
    fn fit(mut camera: Self, corners: &[Vec3], aspect: f32, margin: f32) -> Self {
        if corners.is_empty() {
            return camera;
        }
        // An orthographic lens does not need the search at all. Nothing
        // converges, so what fits does not depend on how far away the camera
        // stands: the answer is just how far off the axis the furthest corner
        // is, and it can be read straight off.
        if matches!(camera.lens, Lens::Orthographic { .. }) {
            return Self::fit_orthographic(camera, corners, aspect, margin);
        }

        // How far out of frame the worst offender goes, at a given distance.
        // One is exactly on the edge.
        let overflow = |c: &Self| {
            let vp = c.view_projection(aspect);
            corners
                .iter()
                .map(|p| {
                    let clip = vp * p.extend(1.0);
                    if clip.w <= 0.0 {
                        return f32::INFINITY;
                    }
                    (clip.x / clip.w).abs().max((clip.y / clip.w).abs())
                })
                .fold(0.0f32, f32::max)
        };

        // First a rough estimate, then double until it fits.
        let mut lo = corners[0];
        let mut hi = corners[0];
        for p in corners {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let fov = camera.lens.fov().unwrap_or(45.0);
        camera.distance = ((hi - lo).length() * 0.5 / (fov.to_radians() * 0.5).tan()).max(1.0);
        let mut far = camera.distance;
        for _ in 0..32 {
            camera.distance = far;
            if overflow(&camera) <= 1.0 {
                break;
            }
            far *= 1.5;
        }

        // And now get as close as possible without anything falling out.
        let mut near = 0.0f32;
        for _ in 0..40 {
            let mid = 0.5 * (near + far);
            camera.distance = mid;
            if overflow(&camera) <= 1.0 {
                far = mid;
            } else {
                near = mid;
            }
        }
        camera.distance = far;

        // Centre the *picture*, not the box. The search only moves the eye
        // along its axis, so whichever corner binds first pins its edge of
        // the frame and all the slack collects on the other side — a head
        // with a strip of empty screen above it while the flippers touch the
        // bottom. Sliding the target across the view until the projected
        // content sits centred spends that slack evenly, and each slide earns
        // another (cheaper) distance search, since a centred subject usually
        // lets the camera come a little closer still.
        for _ in 0..3 {
            let vp = camera.view_projection(aspect);
            let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
            let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
            for p in corners {
                let clip = vp * p.extend(1.0);
                if clip.w <= 0.0 {
                    continue;
                }
                let (x, y) = (clip.x / clip.w, clip.y / clip.w);
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
            let (mid_x, mid_y) = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
            if mid_x.abs() < 0.005 && mid_y.abs() < 0.005 {
                break;
            }
            let forward = (camera.target - camera.eye()).normalize_or_zero();
            let up = camera.up();
            let right = forward.cross(up);
            let half_h =
                (camera.lens.fov().unwrap_or(45.0).to_radians() * 0.5).tan() * camera.distance;
            camera.target += right * (mid_x * half_h * aspect) + up * (mid_y * half_h);

            // The subject moved on screen; the closest fit for it is new.
            let mut lo = camera.distance * 0.3;
            let mut hi = camera.distance * 2.0;
            for _ in 0..30 {
                let mid = 0.5 * (lo + hi);
                camera.distance = mid;
                if overflow(&camera) <= 1.0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            camera.distance = hi;
        }

        camera.distance *= margin;
        camera.bracket_depth(corners);
        camera
    }

    /// The orthographic fit, which is arithmetic rather than a search.
    ///
    /// Measure every corner sideways and upwards from the view axis; the box
    /// has to be at least as tall as the tallest and at least as wide as the
    /// widest, and the screen's shape decides which of the two is binding. The
    /// distance is then only a question of standing far enough back that the
    /// near plane is in front of the table, because moving an orthographic
    /// camera along its own axis changes nothing about the picture.
    fn fit_orthographic(mut camera: Self, corners: &[Vec3], aspect: f32, margin: f32) -> Self {
        let forward = (camera.target - camera.eye()).normalize_or_zero();
        let up = camera.up();
        let right = forward.cross(up);

        // Both edges of the content, not the furthest one: taking the largest
        // absolute offset makes a box that *contains* everything but is
        // centred on the target, so a table whose parts reach further to one
        // side than the other comes out with the slack all on the other —
        // visibly off-centre on screen. Measuring each side and moving the
        // target to the middle of them costs nothing and is exactly centred.
        let (mut min_w, mut max_w) = (f32::MAX, f32::MIN);
        let (mut min_h, mut max_h) = (f32::MAX, f32::MIN);
        let mut reach = 0.0f32;
        for p in corners {
            let v = *p - camera.target;
            let (x, y) = (v.dot(right), v.dot(up));
            min_w = min_w.min(x);
            max_w = max_w.max(x);
            min_h = min_h.min(y);
            max_h = max_h.max(y);
            reach = reach.max(v.dot(forward).abs());
        }
        // Moving the target along the view's own axes moves the eye with it,
        // so the direction — and with it `right` and `up` — does not change.
        camera.target += right * ((min_w + max_w) * 0.5) + up * ((min_h + max_h) * 0.5);
        let (half_w, half_h) = ((max_w - min_w) * 0.5, (max_h - min_h) * 0.5);
        // Whichever of the two the screen cannot hold is the one that decides.
        let half_height = half_h.max(half_w / aspect.max(0.01)).max(1e-3) * margin;
        camera.lens = Lens::Orthographic { half_height };
        // Far enough back that everything is in front of the camera, with room
        // for the near plane. Nothing about the picture depends on this.
        camera.distance = (reach * 2.0).max(half_height) + 100.0;
        camera.bracket_depth(corners);
        camera
    }

    /// Where the original's legacy camera stands, in our own frame.
    ///
    /// Port of `ViewSetup::FitCameraToVertices` (`ViewSetup.cpp:625`), which
    /// is how every table written before 10.8 was framed and what the numbers
    /// still stored in these files were tuned against. Rather than search for
    /// a distance, it extends the frustum's slopes back from every corner and
    /// takes where they meet: for a symmetric frustum that is exact, and it
    /// is the distance at which the extremes touch the edges of the screen
    /// with nothing to spare.
    ///
    /// Nothing to spare is the point. Our own fit stands back a little, the
    /// way somebody photographing a machine does, and on a table that models a
    /// room around itself that polite step backwards is a step out through the
    /// room's own opening.
    ///
    /// The original works in its own axes and this works in ours — right, up
    /// and forward from the camera — which is the same arithmetic without the
    /// sign conventions of a coordinate system we do not use. Layback is left
    /// out: it is a shear applied in the projection, not a place to stand.
    fn fit_legacy(mut camera: Self, corners: &[Vec3], aspect: f32) -> Self {
        let Lens::Perspective { fov } = camera.lens else {
            return camera;
        };
        if corners.is_empty() {
            return camera;
        }
        // Half the field of view, because the slope is from the middle of the
        // screen to the edge and the field of view spans both.
        let slope_up = (0.5 * fov.to_radians()).tan();
        let slope_right = slope_up * aspect.max(0.01);

        let forward = (camera.target - camera.eye()).normalize_or_zero();
        let up = camera.up();
        let right = forward.cross(up);

        let (mut max_u, mut min_u) = (f32::MIN, f32::MAX);
        let (mut max_r, mut min_r) = (f32::MIN, f32::MAX);
        // How far the nearest corner reaches back towards the camera, so the
        // eye can be kept outside what it is looking at.
        let mut nearest = 0.0f32;
        let shear = camera.layback_shear();
        for p in corners {
            // Through the same shear the drawing goes through, or the fit
            // frames a table nobody is looking at.
            let leaned = shear.transform_point3(*p);
            let v = leaned - camera.target;
            let (r, u, f) = (v.dot(right), v.dot(up), v.dot(forward));
            // Where the frustum's edge, extended back from this corner, cuts
            // the plane through the target: the corner is on screen from any
            // camera beyond that.
            max_u = max_u.max(u - slope_up * f);
            min_u = min_u.min(u + slope_up * f);
            max_r = max_r.max(r - slope_right * f);
            min_r = min_r.min(r + slope_right * f);
            nearest = nearest.max(-f);
        }
        let up_dist = (max_u - min_u) / (slope_up * 2.0);
        let right_dist = (max_r - min_r) / (slope_right * 2.0);

        // The centre of what was measured, so the table is in the middle of
        // the picture rather than merely inside it.
        camera.target += right * ((max_r + min_r) * 0.5) + up * ((max_u + min_u) * 0.5);
        // Never inside the machine. A long lens on a wide screen has width to
        // spare and only the table's length to fit, and the fit answers that
        // by walking forward until the length fills the height — which on a
        // cabinet's fifteen degree lens is well past the front of the cabinet
        // and out the other side of the apron. The original never meets this,
        // because its cabinet mode is only ever on a screen turned upright;
        // ours is on whatever the window happens to be.
        //
        // A tenth past the nearest corner, so there is a machine in front of
        // you rather than around you.
        camera.distance = up_dist.max(right_dist).max(nearest * 1.1).max(1.0);
        camera
    }

    /// Pulls the near and far planes in around what is being looked at.
    ///
    /// A depth buffer spends almost all of its precision just in front of the
    /// near plane, so what matters is the *ratio* between the two planes and
    /// not the gap. The defaults here are ten and twenty thousand — a ratio of
    /// two thousand — which is survivable at the two thousand units a
    /// forty-five degree lens stands at and not survivable at all at the ten
    /// thousand a twelve degree lens needs: everything of interest lands in the
    /// last thousandth of the range and coplanar surfaces start fighting.
    ///
    /// Bracketing the table instead takes that ratio to about four. The margins
    /// are wide on purpose — half the near distance, double the far — because
    /// what is framed is not everything that is drawn, and a plane that clips
    /// scenery is a worse bug than a plane that wastes precision.
    fn bracket_depth(&mut self, corners: &[Vec3]) {
        let eye = self.eye();
        // Along the view axis, which is what the planes actually cut: a corner
        // far off to the side is further from the eye than it is deep, and
        // bracketing on the straight-line distance would put the near plane
        // through the front of the table.
        let forward = (self.target - eye).normalize_or_zero();
        let (mut nearest, mut farthest) = (f32::MAX, 0.0f32);
        for p in corners {
            let d = (*p - eye).dot(forward);
            nearest = nearest.min(d);
            farthest = farthest.max(d);
        }
        if nearest > farthest {
            return;
        }
        self.near = (nearest * 0.5).max(1.0);
        self.far = (farthest * 2.0).max(self.near + 1.0);
    }

    /// Eye position in table coordinates.
    ///
    /// In Visual Pinball `+Y` points towards the player and `+Z` points up, so
    /// the camera pulls back along `+Y` and rises along `+Z`.
    pub fn eye(&self) -> Vec3 {
        let inc = self.inclination.to_radians();
        let az = self.azimuth.to_radians();
        let horizontal = self.distance * inc.cos();
        self.target
            + Vec3::new(
                horizontal * az.sin(),
                horizontal * az.cos(),
                self.distance * inc.sin(),
            )
    }

    /// View matrix, **left-handed**.
    ///
    /// This is not a matter of taste: Visual Pinball builds its matrices with
    /// `MatrixLookAtLH` and `MatrixPerspectiveFovLH` (`math/matrix.h:541,582`).
    /// With a right-handed system the table comes out **mirrored** — you notice
    /// it immediately on the apron, where the game title reads backwards.
    /// Leans the world back, before the camera looks at it.
    ///
    /// Layback is the oldest trick in this file's ancestry: it shears the
    /// world so that everything standing on the playfield leans away from the
    /// player, which lets a camera sit almost level with the table — where a
    /// cabinet player's eyes are — and still see the far end. Without it a
    /// fifteen degree lens at twenty-two degrees looks at the back of the
    /// slingshots.
    ///
    /// `y += -tan(layback / 2) · z`, in table space, before the view — which
    /// is where the original puts it (`ViewSetup.cpp:446`, and again inside
    /// its camera fit at `:648`, because a fit that ignored the shear would
    /// frame a table that is not the one being drawn).
    ///
    /// It is a shear, so it is not a rigid motion, and the original's own
    /// comment says as much: it "slightly breaks lighting, reflection and
    /// stereo". It does the same here and for the same reason — the lighting
    /// runs on world positions and the eye, neither of which this touches, so
    /// what shifts is where things land on screen and nothing else.
    fn layback_shear(&self) -> Mat4 {
        if self.layback.abs() < 1e-3 {
            return Mat4::IDENTITY;
        }
        let k = -(0.5 * self.layback.to_radians()).tan();
        let mut m = Mat4::IDENTITY;
        // Column-major: the z column's y row, which is what adds z into y.
        m.z_axis.y = k;
        m
    }

    pub fn view(&self) -> Mat4 {
        vpw_math::glam::camera::lh::view::look_at_mat4(self.eye(), self.target, self.up())
            * self.layback_shear()
    }

    /// Which way is up on screen.
    ///
    /// Up is the table's own up, `+Z`, with the part of it that points along
    /// the view taken out. Looking straight down that leaves nothing: `+Z`
    /// **is** the view direction, and a look-at built with the two parallel
    /// collapses — the table vanishes rather than coming out sideways, which is
    /// the sort of failure that reads as "the overhead view is broken".
    ///
    /// So the straight-down case names its own answer, and it is `-Y`: the far
    /// end of the table is the top of the screen, which puts the flippers at
    /// the bottom where a player expects them. Every inclination short of
    /// straight down already converges on it, so switching between them does
    /// not flip the picture over.
    fn up(&self) -> Vec3 {
        let forward = self.target - self.eye();
        let len = forward.length();
        if len < 1e-6 {
            return Vec3::Z;
        }
        let forward = forward / len;
        let up = Vec3::Z - forward * Vec3::Z.dot(forward);
        if up.length_squared() < 1e-6 {
            -Vec3::Y
        } else {
            up.normalize()
        }
    }

    /// Projection over the **0..1** depth range, which is what WebGPU expects.
    /// The OpenGL variant gives -1..1 and leaves half the world on the wrong
    /// side of the clip plane.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        // DirectX convention: depth 0..1 with Y up, which is exactly WebGPU's
        // NDC. Vulkan's has Y flipped and OpenGL's uses -1..1; neither of the
        // two is any use here.
        use vpw_math::glam::camera::lh::proj::directx;
        match self.lens {
            Lens::Perspective { fov } => {
                directx::perspective(fov.to_radians(), aspect.max(0.01), self.near, self.far)
            }
            Lens::Orthographic { half_height } => {
                let h = half_height.max(1e-3);
                let w = h * aspect.max(0.01);
                directx::orthographic(-w, w, -h, h, self.near, self.far)
            }
        }
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }
}
