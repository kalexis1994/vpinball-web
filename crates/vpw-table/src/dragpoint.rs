//! Control points to polyline.
//!
//! Walls, ramps and rubbers do not store a mesh: they store a handful of control
//! points and a flag per point saying whether the curve passes smoothly through
//! it or makes a corner. The geometry is generated at load time.
//!
//! The original solves this in `DragPointObject::GetRgVertex`
//! (`parts/dragpoint.h:156`) with **centripetal Catmull-Rom splines** — the
//! variant that avoids the cusps the uniform ones produce — subdivided
//! recursively until each stretch is flat enough (`math/MeshUtils.h:168`).

use vpw_math::Vec3;

/// A control point exactly as it comes out of the file.
#[derive(Debug, Clone, Copy)]
pub struct ControlPoint {
    pub pos: Vec3,
    /// Whether the curve passes smoothly through here or makes a corner.
    pub smooth: bool,
}

/// A point of the polyline, already expanded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub pos: Vec3,
    /// Whether the vertex is smoothed with its neighbors when computing
    /// normals.
    pub smooth: bool,
    /// Whether it is one of the points the author placed, and not a generated
    /// one.
    pub control: bool,
}

/// Subdivision accuracy for anything that is drawn, and for surfaces.
///
/// The original uses 4 and comments it as "maximum precision that we allow
/// for" (`dragpoint.h:156`). It is [`accuracy_for`] at detail level ten, which
/// is what a ramp with an opaque material asks for (`ramp.h:167`).
pub const ACCURACY: f32 = 4.0;

/// The detail level the original approximates ramps and rubbers at **for the
/// collision code**, as against for drawing (`HIT_SHAPE_DETAIL_LEVEL`,
/// `physconst.h:25`).
pub const HIT_SHAPE_DETAIL_LEVEL: f32 = 7.0;

/// A detail level turned into a subdivision accuracy, the original's way.
///
/// `ramp.h:173` and `rubber.cpp:162` both spell out the same expression, and
/// the comment beside it gives the sense of the number: *min = 4 (highest
/// accuracy/detail level)*. **Larger is coarser.**
///
/// It is a function rather than a constant so the two levels cannot drift
/// apart: change [`HIT_SHAPE_DETAIL_LEVEL`] and the accuracy follows.
pub fn accuracy_for(detail_level: f32) -> f32 {
    4.0 * 10f32.powf((10.0 - detail_level) / 1.5)
}

/// Subdivision accuracy for collision geometry on ramps and rubbers.
///
/// A hundred times coarser than what the same curve is drawn with, and that is
/// deliberate on the original's part: `Ramp::PhysicSetup` (`ramp.cpp:545`) and
/// the three collision paths in `rubber.cpp` all pass the hit-shape level,
/// while the renderer passes the table's.
///
/// Sharing one curve between the two is tempting — a floor the ball rolls on
/// that does not line up with the floor it sees is a real problem — but the
/// cost is not free either: measured on F-14, the fine curve doubles a ramp's
/// collision shapes and costs a fifth of every physics step, for eight per cent
/// of the table's geometry. The original chose the coarse one for collision and
/// so does this.
pub fn collision_accuracy() -> f32 {
    accuracy_for(HIT_SHAPE_DETAIL_LEVEL)
}

/// Coefficients of a cubic stretch, per axis.
struct Cubic {
    c: [[f32; 4]; 3],
}

impl Cubic {
    /// `InitCubicSplineCoeffs`, `MeshUtils.h:15`.
    fn spline(x0: f32, x1: f32, t0: f32, t1: f32) -> [f32; 4] {
        [
            x0,
            t0,
            -3.0 * x0 + 3.0 * x1 - 2.0 * t0 - t1,
            2.0 * x0 - 2.0 * x1 + t0 + t1,
        ]
    }

    /// `InitNonuniformCatmullCoeffs`, `MeshUtils.h:37`.
    fn nonuniform(x: [f32; 4], dt0: f32, dt1: f32, dt2: f32) -> [f32; 4] {
        let (x0, x1, x2, x3) = (x[0], x[1], x[2], x[3]);
        let mut t1 = (x1 - x0) / dt0 - (x2 - x0) / (dt0 + dt1) + (x2 - x1) / dt1;
        let mut t2 = (x2 - x1) / dt1 - (x3 - x1) / (dt1 + dt2) + (x3 - x2) / dt2;
        // Rescale the tangents to parameterize over [0,1].
        t1 *= dt1;
        t2 *= dt1;
        Self::spline(x1, x2, t1, t2)
    }

    /// `CatmullCurve::SetCurve`, `MeshUtils.h:59`.
    ///
    /// Careful: the original builds the curve **only with x and y** — the 3D
    /// version delegates to the 2D one (`MeshUtils.h:76`). A ramp's height is
    /// interpolated separately, not by the spline.
    fn new(v0: Vec3, v1: Vec3, v2: Vec3, v3: Vec3) -> Self {
        let mut dt0 = (v1 - v0).truncate().length().sqrt();
        let mut dt1 = (v2 - v1).truncate().length().sqrt();
        let mut dt2 = (v3 - v2).truncate().length().sqrt();

        // Repeated control points.
        if dt1 < 1e-4 {
            dt1 = 1.0;
        }
        if dt0 < 1e-4 {
            dt0 = dt1;
        }
        if dt2 < 1e-4 {
            dt2 = dt1;
        }

        Self {
            c: [
                Self::nonuniform([v0.x, v1.x, v2.x, v3.x], dt0, dt1, dt2),
                Self::nonuniform([v0.y, v1.y, v2.y, v3.y], dt0, dt1, dt2),
                // z goes linearly between the two points of the stretch, which
                // is what the original does by keeping only `xy`.
                [v1.z, v2.z - v1.z, 0.0, 0.0],
            ],
        }
    }

    fn at(&self, t: f32) -> Vec3 {
        let (t2, t3) = (t * t, t * t * t);
        let axis = |c: &[f32; 4]| c[3] * t3 + c[2] * t2 + c[1] * t + c[0];
        Vec3::new(axis(&self.c[0]), axis(&self.c[1]), axis(&self.c[2]))
    }
}

/// `FlatWithAccuracy`, `MeshUtils.h:346`: twice the signed area of the triangle,
/// squared.
fn flat(v1: Vec3, v2: Vec3, mid: Vec3, accuracy: f32) -> bool {
    let double_area = (mid.x - v1.x) * (v2.y - v1.y) - (v2.x - v1.x) * (mid.y - v1.y);
    double_area * double_area < accuracy
}

/// State of a subdivision in progress.
struct Subdivision<'a> {
    cc: &'a Cubic,
    accuracy: f32,
    out: &'a mut Vec<Point>,
}

impl Subdivision<'_> {
    /// `RecurseSmoothLine`, `MeshUtils.h:168`.
    ///
    /// The depth limit is not in the original: its data comes from the editor,
    /// and we read third-party files where a degenerate curve cannot be allowed
    /// to hang the load.
    fn recurse(&mut self, t1: f32, t2: f32, v1: Point, v2: Point, depth: usize) {
        const MAX: usize = 20;

        let t_mid = (t1 + t2) * 0.5;
        let mid = Point {
            pos: self.cc.at(t_mid),
            // Generated points are always smooth: they are part of the curve.
            smooth: true,
            control: false,
        };

        if depth >= MAX || flat(v1.pos, v2.pos, mid.pos, self.accuracy) {
            // This loop never adds the last point: the next round adds it as its
            // own first point.
            self.out.push(v1);
        } else {
            self.recurse(t1, t_mid, v1, mid, depth + 1);
            self.recurse(t_mid, t2, mid, v2, depth + 1);
        }
    }
}

/// Expands the control points into a polyline.
///
/// `closed` tells an outline — a wall, which comes back on itself — apart from
/// an open path, like a ramp.
pub fn expand(points: &[ControlPoint], closed: bool, accuracy: f32) -> Vec<Point> {
    let n = points.len();
    if n < 2 {
        return points
            .iter()
            .map(|p| Point {
                pos: p.pos,
                smooth: p.smooth,
                control: true,
            })
            .collect();
    }

    let end = if closed { n } else { n - 1 };
    let mut out = Vec::new();
    let mut last = Point {
        pos: points[n - 1].pos,
        smooth: true,
        control: false,
    };

    for i in 0..end {
        let p1 = points[i];
        let p2 = points[if i < n - 1 { i + 1 } else { 0 }];

        // Two points that coincide: there is no stretch.
        if (p1.pos - p2.pos).length_squared() == 0.0 {
            continue;
        }

        // The neighbors that define the tangents. If the point is not smooth,
        // the point itself is used, and there the curve makes a corner.
        let iprev = if p1.smooth {
            if i == 0 {
                if closed { n - 1 } else { 0 }
            } else {
                i - 1
            }
        } else {
            i
        };
        let inext = {
            let j = if p2.smooth { i + 2 } else { i + 1 };
            if j >= n {
                if closed { j - n } else { n - 1 }
            } else {
                j
            }
        };

        let cc = Cubic::new(points[iprev].pos, p1.pos, p2.pos, points[inext].pos);
        let v1 = Point {
            pos: p1.pos,
            smooth: p1.smooth,
            control: true,
        };
        let v2 = Point {
            pos: p2.pos,
            smooth: p2.smooth,
            control: true,
        };
        last = v2;

        Subdivision {
            cc: &cc,
            accuracy,
            out: &mut out,
        }
        .recurse(0.0, 1.0, v1, v2, 0);
    }

    if !closed {
        // Nobody else adds the last point.
        out.push(Point {
            pos: last.pos,
            smooth: true,
            control: false,
        });
    }
    out
}

/// Converts `vpin`'s control points.
pub fn from_vpin(points: &[vpin::vpx::gameitem::dragpoint::DragPoint]) -> Vec<ControlPoint> {
    points
        .iter()
        .map(|p| ControlPoint {
            pos: Vec3::new(p.x, p.y, p.z),
            smooth: p.smooth,
        })
        .collect()
}
