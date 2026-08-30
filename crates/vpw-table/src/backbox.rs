//! The head of the machine: the panel that stands up behind the playfield.
//!
//! A `.vpx` does not contain one. The file describes the playfield and nothing
//! above it, because Visual Pinball draws the backglass from a separate file —
//! a `.directb2s` — or on a second monitor, and neither is part of the table.
//! So this is **built**, not loaded, from the proportions of the real cabinet.
//!
//! # Where the numbers come from
//!
//! The one hard measurement is the ball: fifty of Visual Pinball's units are
//! 1 1/16 inches, its diameter (`def.h:616`). Everything else here is expressed
//! against the playfield the file *does* describe, so a widebody gets a
//! widebody's head without anyone having to say so.
//!
//! A real machine's head is a little wider than the body it sits on, roughly
//! square, and leans back a few degrees so the glass faces the player rather
//! than the ceiling. Those three facts are what the numbers below say, and they
//! are the ones that matter for framing a shot: get them roughly right and a
//! proper model dropped in later lands in the same place.
//!
//! It is deliberately plain. Nothing here is trying to look like a machine —
//! it is standing where the machine will be, so the camera has something true
//! to frame and the score has somewhere to sit.

use crate::geometry::{Bounds, Mesh, MeshKind, Vertex};
use vpw_math::{Mat4, Vec3};

/// The name the head's face is textured with.
///
/// A reserved name rather than one from the file: nothing in a `.vpx` is called
/// this, and the head is not in a `.vpx` either. The image behind it is the
/// score display, redrawn whenever the machine's segments change, which is why
/// it is marked [`crate::geometry::Image::redrawn`] and gets a texture the
/// renderer is allowed to write to.
pub const DISPLAY_IMAGE: &str = "vpw:display";

/// How many digit cells the display is: rows, then columns.
///
/// A System 11 strobes sixteen slots per row and has two rows. A machine with a
/// different display would want different numbers here, and that is the whole
/// of what would have to change.
pub const DISPLAY_GRID: (usize, usize) = (2, 16);

/// How big the texture behind it is, in pixels.
///
/// Fixed, and that is the point: it is written in place every time the segments
/// change, and a texture that changed size would need its bind group rebuilt
/// with it. Generous enough that a digit is a few dozen pixels across, which is
/// what keeps a thin stroke from disappearing when the head is small on screen.
///
/// Sixteen columns across five hundred and twelve is thirty-two pixels a
/// digit, which is what the floating panel is drawn at and reads crisply. It
/// used to be twice that in each direction, and the four times the pixels
/// were not free: this image is redrawn and re-uploaded on *every frame* a
/// dot-matrix animation is running, and both the halo that is drawn into it
/// and the megabyte that went up to the GPU were being paid sixty times a
/// second for detail nobody could see.
pub const DISPLAY_PIXELS: (u32, u32) = (512, 128);

/// Where the display sits on the head, as fractions of the head's face:
/// left, top, width, height.
///
/// Upper middle. On a real backglass the score window is set into the artwork
/// rather than filling it, and leaving the rest of the face free is what makes
/// room for the artwork when there is any.
pub const DISPLAY_AREA: [f32; 4] = [0.12, 0.16, 0.76, 0.30];

/// How much wider the head is than the playfield.
///
/// A standard body is about 20 1/2 inches across the playfield and the head
/// above it is wider still, which is the overhang you see from the front.
const WIDTH_RATIO: f32 = 1.2;

/// How tall the head is against its own width.
///
/// Backglasses are near enough square — which is why a backglass monitor is
/// 4:3 or 5:4 and why the artwork tables carry for one is too: F-14's is
/// 1280x1024, and 1024/1280 is exactly this.
const HEIGHT_RATIO: f32 = 0.8;

/// How high the head's bottom edge sits above the playfield.
///
/// The playfield is set into the cabinet and the head stands on the cabinet
/// behind it, so there is a step up. Against the head's own height rather than
/// an absolute, so it scales with the machine.
const RISE_RATIO: f32 = 0.35;

/// How far the head leans back from vertical, in degrees.
///
/// Enough that the glass faces a standing player instead of the ceiling, and
/// little enough that it still reads as upright.
const LEAN_DEGREES: f32 = 8.0;

/// The eight corners of an axis-aligned box.
///
/// Here rather than in the renderer because the scene is what has boxes to
/// hand over and the renderer is only one of the things that reads them.
pub fn corners_of(min: Vec3, max: Vec3) -> [Vec3; 8] {
    let mut out = [Vec3::ZERO; 8];
    for (i, corner) in out.iter_mut().enumerate() {
        *corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
    }
    out
}

/// Where the head stands, in the table's own coordinates.
///
/// Built once and handed around: the renderer needs it to draw the panel, and
/// the camera needs it to decide what a view has to fit.
#[derive(Debug, Clone, Copy)]
pub struct Backbox {
    /// Centre of the panel's face.
    pub center: Vec3,
    pub width: f32,
    pub height: f32,
    /// Lean back from vertical, in radians.
    pub lean: f32,
}

impl Backbox {
    /// Works out where the head goes for a given playfield.
    ///
    /// The far end of the playfield is the **top** in table coordinates: `y`
    /// grows towards the player, so the head stands at `min.y`.
    pub fn for_playfield(playfield: Bounds) -> Self {
        let width = (playfield.max.x - playfield.min.x) * WIDTH_RATIO;
        let height = width * HEIGHT_RATIO;
        let rise = height * RISE_RATIO;
        Self {
            center: Vec3::new(
                (playfield.min.x + playfield.max.x) * 0.5,
                playfield.min.y,
                rise + height * 0.5,
            ),
            width,
            height,
            lean: LEAN_DEGREES.to_radians(),
        }
    }

    /// The eight numbers a camera needs: the box the head occupies.
    ///
    /// Approximate on the leaning axis, which is the point — a camera framing a
    /// shot wants to know the head is up there and about this big, not where
    /// each corner of it is to the millimetre.
    pub fn bounds(&self) -> Bounds {
        let half_w = self.width * 0.5;
        let half_h = self.height * 0.5;
        // Leaning back trades height for depth. Both are needed whole, since
        // the box has to contain the panel at any lean.
        let depth = half_h * self.lean.sin();
        Bounds {
            min: Vec3::new(
                self.center.x - half_w,
                self.center.y - depth,
                self.center.z - half_h,
            ),
            max: Vec3::new(
                self.center.x + half_w,
                self.center.y + depth,
                self.center.z + half_h,
            ),
        }
    }

    /// The four corners of the panel's face, anticlockwise from bottom left.
    ///
    /// The mesh is built from these and so is anything that has to know where
    /// the panel lands on screen — which is not the same question as its
    /// bounding box: the box is what a camera has to fit, and the corners are
    /// where the score goes.
    pub fn corners(&self) -> [Vec3; 4] {
        let (half_w, half_h) = (self.width * 0.5, self.height * 0.5);
        // Leaning back about the horizontal axis: the top edge goes away from
        // the player, the bottom edge towards them.
        let (sin, cos) = self.lean.sin_cos();
        let up = Vec3::new(0.0, -sin, cos);
        let right = Vec3::X;
        let at = |u: f32, v: f32| self.center + right * (half_w * u) + up * (half_h * v);
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
    }

    /// The score display, as a mesh of its own on the face of the head.
    ///
    /// Separate from the head rather than textured onto it, because the two are
    /// different things: the head is a panel and the display is a window set
    /// into it. Keeping them apart also means the head can carry artwork later
    /// without the display having to share a texture with it.
    ///
    /// Pushed a hair towards the player so it does not fight the panel behind
    /// it for the same depth — two coplanar surfaces come out as a stipple of
    /// whichever won each pixel.
    pub fn display_mesh(&self) -> Mesh {
        let [left, top, width, height] = DISPLAY_AREA;
        let normal = self.normal();
        let (sin, cos) = self.lean.sin_cos();
        let up = Vec3::new(0.0, -sin, cos);
        let right = Vec3::X;
        // The face runs -1..1 in both directions from the centre, and `top` is
        // measured from the top edge, so `v` counts down from +1.
        let at = |u: f32, v: f32| {
            self.center
                + right * (self.width * 0.5 * (left * 2.0 - 1.0 + u * width * 2.0))
                + up * (self.height * 0.5 * (1.0 - top * 2.0 - v * height * 2.0))
                + normal * (self.height * 0.002)
        };
        let corner = |u: f32, v: f32| Vertex {
            pos: at(u, v).into(),
            normal: normal.into(),
            uv: [u, v],
        };
        Mesh {
            name: "backbox display".into(),
            vertices: vec![
                corner(0.0, 1.0),
                corner(1.0, 1.0),
                corner(1.0, 0.0),
                corner(0.0, 0.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            image: DISPLAY_IMAGE.into(),
            material: String::new(),
            visible: true,
            clamp: false,
            scenery: false,
            kind: MeshKind::Backbox,
        }
    }

    /// Which way the face points: towards the player, and tipped up by the
    /// lean.
    fn normal(&self) -> Vec3 {
        let (sin, cos) = self.lean.sin_cos();
        Vec3::new(0.0, cos, sin)
    }

    /// The panel itself, as a mesh the scene can hold.
    ///
    /// One quad facing the player. Two triangles is all a stand-in needs, and
    /// keeping it to that makes it obvious in a wireframe that this is not the
    /// real thing.
    pub fn mesh(&self) -> Mesh {
        let normal = self.normal().into();
        // The same corners the score is placed against, so the two can never
        // describe different panels.
        let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let vertices = self
            .corners()
            .iter()
            .zip(uv)
            .map(|(pos, uv)| Vertex {
                pos: (*pos).into(),
                normal,
                uv,
            })
            .collect();

        Mesh {
            name: "backbox".into(),
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            // The artwork, painted from the table's own colours rather than
            // loaded: see [`crate::backglass`]. It used to be bare, which is
            // the flat white panel this port stood behind every machine.
            image: crate::backglass::BACKGLASS_IMAGE.into(),
            material: String::new(),
            visible: true,
            clamp: false,
            scenery: false,
            kind: MeshKind::Backbox,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standard body: about 20 1/2 inches across and 46 long, which is what
    /// F-14 measures.
    fn playfield() -> Bounds {
        Bounds {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(964.0, 2162.0, 0.0),
        }
    }

    #[test]
    fn the_head_stands_behind_the_playfield_and_above_it() {
        // `y` grows towards the player, so the far end — where the head goes —
        // is the smaller one. Getting this backwards puts the machine's head
        // between the player and the flippers.
        let b = Backbox::for_playfield(playfield());
        assert_eq!(b.center.y, 0.0, "at the far end, not the near one");
        assert!(
            b.center.z > 0.0,
            "and above the playfield, not level with it"
        );
        assert!(
            b.bounds().min.z > 0.0,
            "its bottom edge clears the playfield: {}",
            b.bounds().min.z
        );
    }

    #[test]
    fn the_head_is_wider_than_the_playfield_and_nearly_square() {
        // Both are facts about a real machine rather than choices: the head
        // overhangs the body, and a backglass is near enough square — which is
        // why the artwork a table carries for one is 5:4.
        let pf = playfield();
        let b = Backbox::for_playfield(pf);
        let table_width = pf.max.x - pf.min.x;

        assert!(
            b.width > table_width,
            "the head overhangs: {} against {table_width}",
            b.width
        );
        let ratio = b.height / b.width;
        assert!(
            (0.7..=1.0).contains(&ratio),
            "and is near enough square: {ratio}"
        );
    }

    #[test]
    fn a_wider_table_gets_a_wider_head() {
        // Everything is expressed against the playfield the file describes, so
        // a widebody gets a widebody's head without anyone saying so.
        let standard = Backbox::for_playfield(playfield());
        let wide = Backbox::for_playfield(Bounds {
            min: Vec3::ZERO,
            max: Vec3::new(1100.0, 2162.0, 0.0),
        });
        assert!(wide.width > standard.width);
        assert!(wide.height > standard.height);
    }

    #[test]
    fn the_panel_faces_the_player() {
        // Leaning back means the face points towards the player and upwards.
        // Pointing it the other way makes the head invisible from the one place
        // anybody looks from.
        let b = Backbox::for_playfield(playfield());
        let mesh = b.mesh();
        let normal = mesh.vertices[0].normal;
        assert!(normal[1] > 0.0, "towards the player: {normal:?}");
        assert!(normal[2] > 0.0, "and tipped up: {normal:?}");

        // The top edge leans away from the player, which is what a lean is.
        let bottom = mesh.vertices[0].pos;
        let top = mesh.vertices[3].pos;
        assert!(top[2] > bottom[2], "the top edge is the higher one");
        assert!(
            top[1] < bottom[1],
            "and the further one: {top:?} against {bottom:?}"
        );
    }
}
