//! Camera.
//!
//! The original solves this in `ViewSetup::ComputeMVP` (`ViewSetup.cpp:331`),
//! which drags along three layout modes (desktop, FSS, cabinet), an iterative
//! framing adjustment, stereo and *layback* — a shear that has been there since
//! before the engine was really 3D. Here there is a perspective camera and
//! nothing else; parity with the layout modes comes later.

use vpw_math::{Mat4, Vec3};

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
    /// Vertical field of view, in degrees.
    pub fov: f32,
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
}

impl View {
    /// How high above the playfield the eye sits, in degrees.
    ///
    /// Forty-five for the front view is not invented: a table carries its own
    /// desktop inclination in the file and F-14's is 44.6. Reading each table's
    /// own is the refinement; this is the value they cluster around.
    fn inclination(self) -> f32 {
        match self {
            View::Front => 45.0,
            View::Overhead => 90.0,
        }
    }

    /// Whether the machine's head is part of what this view has to fit.
    pub fn shows_backbox(self) -> bool {
        matches!(self, View::Front)
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 2000.0,
            inclination: 50.0,
            azimuth: 0.0,
            fov: 45.0,
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
        let (mut min, mut max) = playfield;
        if view.shows_backbox() {
            min = min.min(backbox.0);
            max = max.max(backbox.1);
        }
        let camera = Self {
            target: (min + max) * 0.5,
            inclination: view.inclination(),
            ..Default::default()
        };
        Self::fit(camera, min, max, aspect)
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
        Self::fit(camera, min, max, aspect)
    }

    /// The distance search, for a camera that already knows where it is looking
    /// from.
    ///
    /// Split out from [`Camera::framing`] because the search only answers for
    /// the inclination it was run at: move the eye afterwards and the distance
    /// it found is for a shot nobody is taking. A named view picks its angle
    /// first and searches second.
    fn fit(mut camera: Self, min: Vec3, max: Vec3, aspect: f32) -> Self {
        let corners = [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(min.x, max.y, max.z),
            Vec3::new(max.x, max.y, max.z),
        ];

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
        let size = max - min;
        camera.distance = (size.length() * 0.5 / (camera.fov.to_radians() * 0.5).tan()).max(1.0);
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
        // A little breathing room so the table does not touch the edges.
        camera.distance = far * 1.04;
        camera
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
    pub fn view(&self) -> Mat4 {
        vpw_math::glam::camera::lh::view::look_at_mat4(self.eye(), self.target, self.up())
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
        vpw_math::glam::camera::lh::proj::directx::perspective(
            self.fov.to_radians(),
            aspect.max(0.01),
            self.near,
            self.far,
        )
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }
}
