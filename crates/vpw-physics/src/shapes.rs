//! The shapes the ball collides against.
//!
//! They all answer the same question: *given the state of the ball and an
//! available time, when and against which normal is it going to hit?* They
//! return the time of the hit, or `None` if there is none inside the step.
//!
//! Port of `physics/collide.cpp`.
//!
//! # The normal distance is at the heart of everything
//!
//! Every shape computes `bnd`, the distance from the ball to the surface
//! measured along the normal, and `bnv`, the velocity in that same direction.
//! Everything is decided with those two numbers: whether there is a hit
//! (`bnd / -bnv` gives the time), whether there is a sustained contact (both
//! almost zero), or whether the ball ended up embedded (`bnd` very negative).

use crate::ball::Ball;
use crate::collision::{CollisionEvent, Material};
use crate::constants::{C_CONTACTVEL, C_LOWNORMVEL, C_TOL_ENDPNTS, C_TOL_RADIUS, PHYS_TOUCH};
use crate::{Aabb, aabb_of_points};
use vpw_math::{Mat3, Quat, Vec2, Vec3};

/// A wall: a vertical segment between two heights.
///
/// It is the most used shape of all — the walls, the sides of the ramps and
/// the edges of the plastics are all this.
#[derive(Debug, Clone)]
pub struct LineSeg {
    pub v1: Vec2,
    pub v2: Vec2,
    /// In-plane normal, pointing outwards.
    pub normal: Vec2,
    pub length: f32,
    pub z_low: f32,
    pub z_high: f32,
    pub material: Material,
}

impl LineSeg {
    /// Builds a segment.
    ///
    /// The normal comes out of `v1 - v2` rotated, **in that order**
    /// (`LineSeg::CalcNormalAndLength`, `collide.cpp:176`). The order matters:
    /// flipping it gives the opposite normal, and since a wall **only collides
    /// on its front face**, it would end up passable exactly on the side it
    /// was supposed to stop. The direction the outline is traversed in is what
    /// defines which side is the outside.
    pub fn new(v1: Vec2, v2: Vec2, z_low: f32, z_high: f32, material: Material) -> Self {
        let vt = v1 - v2;
        let length = vt.length();
        let normal = if length > 1e-9 {
            Vec2::new(vt.y / length, -vt.x / length)
        } else {
            Vec2::Y
        };
        Self {
            v1,
            v2,
            normal,
            length,
            z_low,
            z_high,
            material,
        }
    }

    pub fn bbox(&self) -> Aabb {
        aabb_of_points(&[
            Vec3::new(self.v1.x, self.v1.y, self.z_low),
            Vec3::new(self.v2.x, self.v2.y, self.z_high),
        ])
    }

    /// `LineSeg::HitTestBasic` (`collide.cpp:53`), with the three flags set to
    /// `true`: normal face, lateral and rigid, which is what a wall uses.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        // Normal velocity: positive means it is moving away.
        let bnv = ball.vel.x * self.normal.x + ball.vel.y * self.normal.y;
        if bnv > C_LOWNORMVEL {
            return None; // clearly moving away
        }

        // Distance from the **edge** of the ball to the wall.
        let bcpd =
            (ball.pos.x - self.v1.x) * self.normal.x + (ball.pos.y - self.v1.y) * self.normal.y;
        let bnd = bcpd - ball.radius;

        let inside = bnd <= 0.0;

        // Excessive penetration: the hit is discarded. It is a guard against
        // balls that got in through an earlier bug.
        if bnd < -ball.radius || bcpd < 0.0 {
            return None;
        }

        let hittime = if bnd <= PHYS_TOUCH {
            if inside || bnv.abs() > C_CONTACTVEL || bnd <= -PHYS_TOUCH {
                0.0 // slow but embedded, or fast: resolve right now
            } else {
                // Spread the time out inside the contact band so it does not
                // compete with zero-time events.
                bnd * (1.0 / (2.0 * PHYS_TOUCH)) + 0.5
            }
        } else if bnv.abs() > C_LOWNORMVEL {
            bnd / -bnv
        } else {
            return None; // too slow: wait until they touch
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        // Does the contact fall inside the segment, or does it run past the
        // endpoints?
        let btv = ball.vel.x * self.normal.y - ball.vel.y * self.normal.x;
        let btd = (ball.pos.x - self.v1.x) * self.normal.y
            - (ball.pos.y - self.v1.y) * self.normal.x
            + btv * hittime;
        if btd < -C_TOL_ENDPNTS || btd > self.length + C_TOL_ENDPNTS {
            return None;
        }

        // And at the right height? The wall has a z range.
        let hitz = ball.pos.z + ball.vel.z * hittime;
        if hitz + ball.radius * 0.5 < self.z_low || hitz - ball.radius * 0.5 > self.z_high {
            return None;
        }

        let is_contact = bnv.abs() <= C_CONTACTVEL && bnd.abs() <= PHYS_TOUCH;
        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: Vec3::new(self.normal.x, self.normal.y, 0.0),
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }

    /// The same segment seen from the other side.
    ///
    /// The original keeps **two** opposite `LineSeg`s inside each gate and each
    /// spinner (`collideex.cpp:HitGate::HitGate`) so it can detect the crossing
    /// in both directions. It is the same line: it is enough to flip it when
    /// testing it, without duplicating the data.
    pub fn flipped(&self) -> Self {
        Self {
            v1: self.v2,
            v2: self.v1,
            normal: -self.normal,
            ..*self
        }
    }

    /// Tests the segment as a line the ball **goes through**.
    ///
    /// This is `HitTestBasic` with `direction=false, lateral=true,
    /// rigid=false`, which is how gates and spinners call it. Three
    /// differences with the rigid path, and none of them is cosmetic:
    ///
    /// 1. The ball does not bounce, so there is no contact band and no
    ///    penetration check: either it crosses the plane within the remaining
    ///    time, or it does not.
    /// 2. The radius is **added** instead of subtracted. Without that the ball
    ///    reaches halfway into the flipper before it notices and starts to
    ///    move (comment from the original at `collide.cpp:76`).
    /// 3. The ball moving away is accepted: we have to detect both the entry
    ///    and the exit.
    pub fn hit_test_through(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let bnv = ball.vel.x * self.normal.x + ball.vel.y * self.normal.y;
        let bnd = (ball.pos.x - self.v1.x) * self.normal.x
            + (ball.pos.y - self.v1.y) * self.normal.y
            + ball.radius;

        // Outside and moving away, or inside and coming closer: in both cases
        // there is no crossing. It is the same sign, which is why the product
        // is enough.
        if bnv * bnd >= 0.0 {
            return None;
        }
        let hittime = bnd / -bnv;
        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        let btv = ball.vel.x * self.normal.y - ball.vel.y * self.normal.x;
        let btd = (ball.pos.x - self.v1.x) * self.normal.y
            - (ball.pos.y - self.v1.y) * self.normal.x
            + btv * hittime;
        if btd < -C_TOL_ENDPNTS || btd > self.length + C_TOL_ENDPNTS {
            return None;
        }

        let hitz = ball.pos.z + ball.vel.z * hittime;
        if hitz + ball.radius * 0.5 < self.z_low || hitz - ball.radius * 0.5 > self.z_high {
            return None;
        }

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: Vec3::new(self.normal.x, self.normal.y, 0.0),
            hit_distance: bnd,
            is_contact: false,
            hit_flag: bnv > C_LOWNORMVEL,
            hit_org_normal_velocity: 0.0,
            hit_vel: Vec2::ZERO,
        })
    }
}

/// A circle seen from above, extruded in height.
///
/// The bumpers, the flipper bases, the posts and the kickers are this.
#[derive(Debug, Clone)]
pub struct HitCircle {
    pub center: Vec2,
    pub radius: f32,
    pub z_low: f32,
    pub z_high: f32,
    pub material: Material,
}

impl HitCircle {
    pub fn bbox(&self) -> Aabb {
        Aabb {
            min: Vec3::new(
                self.center.x - self.radius,
                self.center.y - self.radius,
                self.z_low,
            ),
            max: Vec3::new(
                self.center.x + self.radius,
                self.center.y + self.radius,
                self.z_high,
            ),
        }
    }

    /// `HitCircle::HitTestBasicRadius` with the flags of a rigid object:
    /// bumper, flipper base, gate, spinner (`collide.cpp:209`).
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        self.hit_test_with(ball, dtime, true, true, true)
    }

    /// The same, with the flags of a kicker or a trigger: **all three off**.
    ///
    /// The original writes the two cases out on one line (`collide.cpp:209`):
    /// "all of these true = bumper/flipperbase/gate/spinner, all false =
    /// kicker/trigger". Reusing the rigid version for a kicker turns it into a
    /// solid post, and an invisible one — because each flag does something
    /// different and all three point the same way:
    ///
    /// - `lateral` adds the ball's radius to the target, so a twenty-five unit
    ///   saucer becomes a fifty unit obstacle.
    /// - `rigid` allows a **contact**, which the engine then resolves with
    ///   friction. A kicker never produces one in the original; the assignment
    ///   is inside the rigid branch (`collide.cpp:264`).
    /// - `direction` throws away a hit where the ball is receding, which is
    ///   exactly the case a kicker has to keep — the original's own comment is
    ///   "commonly hit while receding" (`collide.cpp:255`).
    ///
    /// What that looks like on a table: a ball kicked up an outlane runs into
    /// a rigid disc twice the size of the saucer it just left, loses its speed
    /// to friction, and drops back in to be kicked again, for ever.
    ///
    /// **`direction` is left on here, and that is a departure.** The original
    /// turns it off so that a ball already inside and on its way out still
    /// reports — and then decides what to do about it from the set of volumes
    /// it remembers each ball being inside of (`collide.cpp:276`). This engine
    /// deliberately does not keep that set: it derives inside-ness from the
    /// position on each step instead, which is what `trigger.rs` explains at
    /// length and why three of the original's self-repair branches are not
    /// needed here. Without the set there is nothing to tell a ball leaving
    /// from a ball arriving, so a receding hit would let a saucer grab the ball
    /// it has just ejected. Off by one flag and honest about it beats faithful
    /// to a branch whose state does not exist.
    pub fn hit_test_as_volume(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        self.hit_test_with(ball, dtime, true, false, false)
    }

    /// `HitCircle::HitTestBasicRadius` (`collide.cpp:208`).
    ///
    /// Not implemented: the original's `capsule3D` branch for a ball riding
    /// above the shape's top, and the volume-set bookkeeping that decides
    /// whether a trigger reports entering or leaving. Both need state this
    /// engine does not keep yet.
    fn hit_test_with(
        &self,
        ball: &Ball,
        dtime: f32,
        direction: bool,
        lateral: bool,
        rigid: bool,
    ) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        // A circle is a planar problem: the vertical component is ignored.
        let dist = Vec2::new(ball.pos.x - self.center.x, ball.pos.y - self.center.y);
        let dv = Vec2::new(ball.vel.x, ball.vel.y);

        // The ball's radius counts only for a lateral hit.
        let target_radius = match lateral {
            true => self.radius + ball.radius,
            false => self.radius,
        };

        let bcddsq = dist.length_squared();
        let bcdd = bcddsq.sqrt();
        if bcdd <= 1.0e-6 {
            return None; // right on the center: there is no normal
        }

        let b = dist.dot(dv);
        let bnv = b / bcdd;
        if direction && bnv > C_LOWNORMVEL {
            return None; // clearly receding
        }

        let bnd = bcdd - target_radius;
        let a = dv.length_squared();

        let mut is_contact = false;
        let hittime = if rigid && bnd < PHYS_TOUCH {
            if bnd < -ball.radius {
                return None; // too far inside
            }
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else {
                // It can come out negative if the ball was going so fast that
                // it already got in; the check below discards that.
                (-bnd / bnv).max(0.0)
            }
        } else {
            // Outside and receding, or inside and approaching: no hit.
            if (!rigid && bnd * bnv > 0.0) || a < 1.0e-8 {
                return None;
            }
            let (t1, t2) = solve_quadratic(a, 2.0 * b, bcddsq - target_radius * target_radius)?;
            // If the two times have different signs, the ball is already
            // inside and what matters is when it comes out.
            if t1 * t2 < 0.0 {
                t1.max(t2)
            } else {
                t1.min(t2)
            }
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        let hitz = ball.pos.z + ball.vel.z * hittime;
        if hitz + ball.radius * 0.5 < self.z_low || hitz - ball.radius * 0.5 > self.z_high {
            return None;
        }

        let hitx = ball.pos.x + ball.vel.x * hittime;
        let hity = ball.pos.y + ball.vel.y * hittime;
        let d = Vec2::new(hitx - self.center.x, hity - self.center.y);
        let sqrlen = d.length_squared();

        // If the impact lands exactly on the center there is no defined
        // normal, and the original makes one up: any direction will do.
        let hit_normal = if sqrlen > 1.0e-8 {
            let inv = 1.0 / sqrlen.sqrt();
            Vec3::new(d.x * inv, d.y * inv, 0.0)
        } else {
            Vec3::Y
        };

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal,
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }
}

/// An infinite plane. It is the playfield and the glass above it.
#[derive(Debug, Clone)]
pub struct HitPlane {
    pub normal: Vec3,
    /// Distance from the plane to the origin along the normal.
    pub d: f32,
    pub material: Material,
}

impl HitPlane {
    /// `HitPlane::HitTest` (`collide.cpp:1075`).
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let bnv = self.normal.dot(ball.vel);
        if bnv > C_CONTACTVEL {
            return None; // moving away
        }

        let bnd = self.normal.dot(ball.pos) - ball.radius - self.d;

        let mut is_contact = false;
        let hittime = if bnd < PHYS_TOUCH {
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else if bnd <= 0.0 {
                0.0 // already penetrated: resolve right now
            } else {
                bnd / -bnv
            }
        } else if bnv.abs() > C_LOWNORMVEL {
            bnd / -bnv
        } else {
            return None;
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: self.normal,
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }
}

/// A point: the corners where two walls meet.
///
/// Without these, a ball that arrives right at the vertex between two segments
/// slips through the seam.
#[derive(Debug, Clone)]
pub struct HitPoint {
    pub p: Vec3,
    pub material: Material,
}

impl HitPoint {
    pub fn bbox(&self) -> Aabb {
        Aabb {
            min: self.p,
            max: self.p,
        }
    }

    /// `HitPoint::HitTest` (`collide.cpp:1120`): the ball against a sphere of
    /// radius zero.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let dist = ball.pos - self.p;
        let bcddsq = dist.length_squared();
        let bcdd = bcddsq.sqrt();
        if bcdd <= 1.0e-6 {
            return None;
        }

        let b = dist.dot(ball.vel);
        let bnv = b / bcdd;
        // `collide.cpp:529` rejects at `C_CONTACTVEL`, not at `C_LOWNORMVEL` —
        // a thousand times looser. The difference is the whole job of this
        // shape: a `HitPoint` exists to seal the seam where two wall segments
        // meet, and a ball creeping into that seam does so at exactly the crawl
        // speed the tighter threshold throws away. Reject it there and the ball
        // sinks into the corner until the penetration correction shoves it back
        // out, which it then does over and over.
        if bnv > C_CONTACTVEL {
            return None;
        }

        let bnd = bcdd - ball.radius;
        let a = ball.vel.length_squared();

        let mut is_contact = false;
        let hittime = if bnd < PHYS_TOUCH {
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else {
                (-bnd / bnv).max(0.0)
            }
        } else {
            if a < 1.0e-8 {
                return None;
            }
            let (t1, t2) = solve_quadratic(a, 2.0 * b, bcddsq - ball.radius * ball.radius)?;
            if t1 * t2 < 0.0 {
                t1.max(t2)
            } else {
                t1.min(t2)
            }
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        let hit_pos = ball.pos + ball.vel * hittime;
        let hit_normal = (hit_pos - self.p).normalize_or(Vec3::Y);

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal,
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }
}

/// Solves `a·t² + b·t + c = 0`, keeping the two roots in order.
///
/// The original calls it `SolveQuadraticEq`. Returns `None` if there are no
/// real roots, that is, if the ball never gets to touch.
pub fn solve_quadratic(a: f32, b: f32, c: f32) -> Option<(f32, f32)> {
    let discr = b * b - 4.0 * a * c;
    if discr < 0.0 {
        return None;
    }
    let discr = discr.sqrt();
    let inv_2a = 1.0 / (2.0 * a);
    let t1 = (-b - discr) * inv_2a;
    let t2 = (-b + discr) * inv_2a;
    Some((t1, t2))
}

/// The radius a non-lateral test uses. It exists so that the rolling tests do
/// not depend on the real radius of the ball.
pub const ROLLING_RADIUS: f32 = C_TOL_RADIUS;

/// A polygon on a plane: the top face of a wall, the surface of a ramp.
///
/// It is what lets a ball stay **resting on a plastic** instead of going
/// through it. Without this, everything that is raised is a shell with walls
/// but no lid.
///
/// Port of `Hit3DPoly` (`collideex.cpp:548`).
#[derive(Debug, Clone)]
pub struct HitPoly {
    /// The vertices, in order around the outline.
    pub verts: Vec<Vec3>,
    pub normal: Vec3,
    pub material: Material,
}

impl HitPoly {
    /// Builds the polygon, computing its normal from the first three
    /// vertices. Returns `None` if they are collinear.
    pub fn new(verts: Vec<Vec3>, material: Material) -> Option<Self> {
        if verts.len() < 3 {
            return None;
        }
        // `Hit3DPoly::Init` (`collideex.cpp:494-511`) builds the normal with
        // Newell's method and then multiplies by **minus** one over the length,
        // with the comment "NOTE: normal is flipped! Thus we need vertices in
        // CCW order". For a triangle Newell's sum is `(v1-v0) x (v2-v0)`, so
        // the normal the original ends up with is that cross product negated —
        // the same convention its own `HitTriangle` uses.
        //
        // Taking the un-negated one instead points every one of these faces the
        // other way, and a face only blocks from the side its normal is on. On
        // a table that means every wall's top and bottom lid faces away from
        // the ball: measured on F-14, all thirty-one collidable walls let a
        // ball dropped on them fall straight through, and a wall that is solid
        // underneath then catches it from above so it ends up riding around
        // inside the wall's shell.
        let n = (verts[2] - verts[0]).cross(verts[1] - verts[0]);
        if n.length_squared() < 1e-12 {
            return None;
        }
        Some(Self {
            verts,
            normal: n.normalize(),
            material,
        })
    }

    pub fn bbox(&self) -> Aabb {
        aabb_of_points(&self.verts)
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let bnv = self.normal.dot(ball.vel);
        if bnv > C_LOWNORMVEL {
            return None; // moving away
        }

        // The point of the ball closest to the plane.
        let mut hit_pos = ball.pos - self.normal * ball.radius;
        let bnd = self.normal.dot(hit_pos - self.verts[0]);
        let inside = bnd <= 0.0;

        if bnd < -ball.radius {
            return None; // excessive penetration
        }

        let hittime = if bnd <= PHYS_TOUCH {
            if inside || bnv.abs() > C_CONTACTVEL || bnd <= -PHYS_TOUCH {
                0.0
            } else {
                bnd * (1.0 / (2.0 * PHYS_TOUCH)) + 0.5
            }
        } else if bnv.abs() > C_LOWNORMVEL {
            bnd / -bnv
        } else {
            return None;
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        // Advance the contact point and ask whether it falls inside the
        // outline. The original solves it by projecting onto the xy plane and
        // counting crossings; that works because these polygons are never
        // vertical.
        hit_pos += ball.vel * hittime;
        if !point_in_polygon(&self.verts, hit_pos) {
            return None;
        }

        let is_contact = bnv.abs() <= C_CONTACTVEL && bnd.abs() <= PHYS_TOUCH;
        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: self.normal,
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }
}

/// One triangle of a mesh. `HitTriangle`, `collideex.cpp:797`.
///
/// # Why this is not [`HitPoly`] with three vertices
///
/// [`HitPoly`] is a port of `Hit3DPoly`, and the two are not
/// interchangeable even though the geometry is. `Hit3DPoly` decides whether the
/// contact point is inside the outline by projecting onto the xy plane and
/// counting crossings, which its own comment in the original explains away with
/// "these polygons are never vertical". The triangles of a mesh are frequently
/// vertical — the face of a drop target is a wall — and projected onto the
/// playfield a vertical triangle is a line with no inside at all.
///
/// So this one does what the original's triangle does: barycentric coordinates,
/// in three dimensions, with no assumption about which way the face points.
///
/// The two also differ in three smaller ways that all matter on a mesh:
///
/// - A triangle counts as **degenerate** far sooner than a polygon does, which
///   is the difference that matters on a mesh; the normal convention is the
///   same in both, `(v2-v0) x (v1-v0)`.
/// - A triangle counts as **degenerate** far sooner: the original rejects one
///   whose un-normalised normal is shorter than a tenth, and notes that below
///   that the precision of the test is not good enough to trust. A mesh has
///   hundreds of triangles and some of them are slivers.
/// - The moment of contact is worked out the same way a wall's is, rather than
///   through `Hit3DPoly`'s interpolation, which exists for the flat lids of
///   ramps and surfaces.
#[derive(Debug, Clone)]
pub struct HitTriangle {
    /// Counter-clockwise, as the original requires.
    pub verts: [Vec3; 3],
    pub normal: Vec3,
    pub material: Material,
}

/// How short the un-normalised normal may be before the triangle is thrown
/// away. `collideex.cpp:812`, where the original notes it is a precision guard
/// and that it depends on the unit system.
const DEGENERATE: f32 = 0.01;

impl HitTriangle {
    /// Builds the triangle, or `None` if it is too thin to test reliably.
    pub fn new(verts: [Vec3; 3], material: Material) -> Option<Self> {
        let n = (verts[2] - verts[0]).cross(verts[1] - verts[0]);
        let ls = n.length_squared();
        if ls < DEGENERATE {
            return None;
        }
        Some(Self {
            verts,
            normal: n / ls.sqrt(),
            material,
        })
    }

    /// The same, from the three vertices in the order the renderer has them.
    ///
    /// Every caller building colliders out of a mesh needs this, and every one
    /// of them in the original writes out the same swap by hand with the same
    /// comment attached (`hittarget.cpp:243`, `primitive.cpp:249`).
    pub fn from_render_order(v0: Vec3, v1: Vec3, v2: Vec3, material: Material) -> Option<Self> {
        Self::new([v0, v2, v1], material)
    }

    pub fn bbox(&self) -> Aabb {
        aabb_of_points(&self.verts)
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let bnv = self.normal.dot(ball.vel);
        if bnv > C_CONTACTVEL {
            return None; // clearly receding
        }

        // The point of the ball that will touch the face, if any does.
        let mut hit_pos = ball.pos - self.normal * ball.radius;
        let bnd = self.normal.dot(hit_pos - self.verts[0]);

        if bnd < -ball.radius {
            return None; // already far through it: not a collision
        }

        let mut is_contact = false;
        let hittime = if bnd <= PHYS_TOUCH {
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else if bnd <= 0.0 {
                0.0 // already touching, and moving in: no time to wait
            } else {
                bnd / -bnv
            }
        } else if bnv.abs() > C_LOWNORMVEL {
            bnd / -bnv
        } else {
            return None; // too slow to be worth predicting; wait for contact
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        hit_pos += ball.vel * hittime;

        // Barycentric, `collideex.cpp:866`. `u` and `v` are the contact point's
        // coordinates in the triangle's own two edges, so it is inside exactly
        // when both are positive and together no more than one.
        let (e0, e1) = (self.verts[2] - self.verts[0], self.verts[1] - self.verts[0]);
        let d = hit_pos - self.verts[0];
        let (d00, d01, d02) = (e0.dot(e0), e0.dot(e1), e0.dot(d));
        let (d11, d12) = (e1.dot(e1), e1.dot(d));
        let inv = 1.0 / (d00 * d11 - d01 * d01);
        let u = (d11 * d02 - d01 * d12) * inv;
        let v = (d00 * d12 - d01 * d02) * inv;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            return None;
        }

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: self.normal,
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }
}

/// Crossing count over the xy plane (`collideex.cpp:620-650`).
fn point_in_polygon(verts: &[Vec3], p: Vec3) -> bool {
    let mut inside = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (verts[i], verts[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let t = (p.y - a.y) / (b.y - a.y);
            if p.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// A segment that kicks: the slingshot.
///
/// It is an ordinary wall except for one thing: if the ball hits it hard
/// enough, a solenoid shoves it back. It is what keeps the ball from sitting
/// there dead in the triangle above the flippers.
///
/// Port of `LineSegSlingshot::Collide` (`collideex.cpp:61`).
#[derive(Debug, Clone)]
pub struct Slingshot {
    pub seg: LineSeg,
    /// Force of the solenoid.
    pub force: f32,
    /// How much normal velocity it has to be hit with in order to fire.
    pub threshold: f32,
    /// Whether the script turned it off.
    pub disabled: bool,
}

impl Slingshot {
    pub fn new(seg: LineSeg, force: f32, threshold: f32) -> Self {
        Self {
            seg,
            force,
            threshold,
            disabled: false,
        }
    }

    pub fn bbox(&self) -> Aabb {
        self.seg.bbox()
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        self.seg.hit_test(ball, dtime)
    }

    /// How hard the solenoid pushes, given where the ball hit.
    ///
    /// The kick is **strongest in the middle** of the segment and dies off
    /// towards the ends, in the shape of a parabola. It makes physical sense:
    /// the real mechanism pushes from the center of the rubber.
    ///
    /// The maximum of that parabola is 0.5, not 1, and the original comments on
    /// it itself: "maximum value 0.5 ...I think this should have been 1.0...oh
    /// well" (`collideex.cpp:82`). We leave it as it is: changing it would
    /// alter the force of every slingshot on every table.
    pub fn kick_force(&self, ball: &Ball, hit_normal: Vec3) -> f32 {
        let point = Vec2::new(
            ball.pos.x - hit_normal.x * ball.radius,
            ball.pos.y - hit_normal.y * ball.radius,
        );
        let d = self.seg.v2 - self.seg.v1;
        let length = d.x * hit_normal.y - d.y * hit_normal.x;
        let btd =
            (point.x - self.seg.v1.x) * hit_normal.y - (point.y - self.seg.v1.y) * hit_normal.x;

        // -1 at one end, +1 at the other, 0 in the middle.
        let t = if length.abs() > 1.0e-6 {
            (btd + btd) / length - 1.0
        } else {
            -1.0
        };
        0.5 * (1.0 - t * t) * self.force
    }

    /// Resolves the hit. Returns `true` if the solenoid fired.
    pub fn collide(&self, ball: &mut Ball, coll: &CollisionEvent, scatter_extra: f32) -> bool {
        let dot = coll.hit_normal.dot(ball.vel);
        let fires = !self.disabled && dot <= -self.threshold;

        if fires {
            // The shove is applied **before** the bounce, against the normal:
            // the ball comes in faster than it arrived and gets flung out.
            let force = self.kick_force(ball, coll.hit_normal);
            ball.vel -= coll.hit_normal * force;
        }

        ball.collide_3d_wall(coll.hit_normal, &self.seg.material, coll, scatter_extra);
        fires
    }
}

/// A vertical line: a pole of zero radius, between two heights.
///
/// `HitLineZ` in the original (`collide.cpp:406`). It exists to seal the seam
/// where two surfaces meet at an angle. A ball arriving exactly at the join of
/// two walls is, to each of them, already past the edge — neither claims it,
/// and it goes through. A pole at the join catches it.
///
/// It is the same test as [`HitPoint`] with the third dimension left out, plus
/// a check that the hit lands between the two heights.
#[derive(Debug, Clone)]
pub struct HitLineZ {
    pub xy: Vec2,
    pub z_low: f32,
    pub z_high: f32,
    pub material: Material,
}

impl HitLineZ {
    pub fn bbox(&self) -> Aabb {
        Aabb {
            min: Vec3::new(self.xy.x, self.xy.y, self.z_low),
            max: Vec3::new(self.xy.x, self.xy.y, self.z_high),
        }
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let dist = Vec2::new(ball.pos.x - self.xy.x, ball.pos.y - self.xy.y);
        let dv = Vec2::new(ball.vel.x, ball.vel.y);

        let bcddsq = dist.length_squared();
        let bcdd = bcddsq.sqrt();
        // Dead centre has no direction to push in.
        if bcdd <= 1.0e-6 {
            return None;
        }

        let b = dist.dot(dv);
        let bnv = b / bcdd;
        if bnv > C_CONTACTVEL {
            return None; // receding
        }

        let bnd = bcdd - ball.radius;
        let a = dv.length_squared();

        let mut is_contact = false;
        let hittime = if bnd < PHYS_TOUCH {
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else {
                -bnd / bnv
            }
        } else {
            if a < 1.0e-8 {
                return None;
            }
            let (t1, t2) = solve_quadratic(a, 2.0 * b, bcddsq - ball.radius * ball.radius)?;
            if t1 * t2 < 0.0 {
                t1.max(t2)
            } else {
                t1.min(t2)
            }
        };

        if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
            return None;
        }

        // Only the stretch between the two heights is solid.
        let hit_z = ball.pos.z + hittime * ball.vel.z;
        if hit_z < self.z_low || hit_z > self.z_high {
            return None;
        }

        let hit_x = ball.pos.x + hittime * ball.vel.x;
        let hit_y = ball.pos.y + hittime * ball.vel.y;
        let normal = Vec2::new(hit_x - self.xy.x, hit_y - self.xy.y).normalize_or_zero();

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: Vec3::new(normal.x, normal.y, 0.0),
            hit_distance: bnd,
            is_contact,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            ..CollisionEvent::default()
        })
    }
}

/// A line segment in space: the same pole as [`HitLineZ`], leaning.
///
/// `HitLine3D` (`collideex.cpp:1052`). The original's trick, and it is a good
/// one: rather than write a second collision routine, it rotates the world so
/// that the segment stands up along `z`, runs the vertical test, and turns the
/// answer back. One routine, and no chance of the two drifting apart.
///
/// These are the joints along the edges of a ramp's floor, where one triangle
/// meets the next. Without them a ball rolling up a ramp falls through the
/// seams.
#[derive(Debug, Clone)]
pub struct HitLine3D {
    /// The rotation that stands the segment up along `z`.
    to_upright: Mat3,
    upright: HitLineZ,
    bbox: Aabb,
    /// The pass direction this seam inherits, when it seals one-sided floors.
    ///
    /// A ramp's floor answers only from its front: a ball rising from
    /// underneath passes through and lands on top, and vertical up-kickers
    /// are built on exactly that (`collideex.cpp:838`; the ramp builder says
    /// where). The joints that seal the seams between those triangles exist
    /// for the ball rolling **on** them — a fold must not be a gap — but as
    /// plain cylinders they answer from every side, so the one route the
    /// floors leave open is fenced by their own stitching: South Park's
    /// SuperVUK fires the ball up under the wireform's mouth and it clatters
    /// off the entry seam instead of rising through. A seam carrying its
    /// floors' normal refuses only what the floors refuse.
    pass_up: Option<Vec3>,
}

impl HitLine3D {
    /// The segment's two ends, back in world space. For whoever has to name a
    /// mystery wall.
    pub fn endpoints(&self) -> (Vec3, Vec3) {
        let back = self.to_upright.transpose();
        (
            back * Vec3::new(self.upright.xy.x, self.upright.xy.y, self.upright.z_low),
            back * Vec3::new(self.upright.xy.x, self.upright.xy.y, self.upright.z_high),
        )
    }

    /// Returns `None` for a segment with no length, which has no direction to
    /// stand up.
    pub fn new(v1: Vec3, v2: Vec3, material: Material) -> Option<Self> {
        let line = v2 - v1;
        if line.length_squared() < 1e-12 {
            return None;
        }
        // Straight to the rotation that takes the segment to `+z`. The original
        // builds it by hand from an axis and an angle; this says the same thing
        // and cannot be got wrong by a sign.
        let to_upright = Mat3::from_quat(glam_rotation_arc(line.normalize(), Vec3::Z));

        let a = to_upright * v1;
        let b = to_upright * v2;
        Some(Self {
            to_upright,
            upright: HitLineZ {
                xy: Vec2::new(a.x, a.y),
                z_low: a.z.min(b.z),
                z_high: a.z.max(b.z),
                material,
            },
            bbox: Aabb {
                min: v1.min(v2),
                max: v1.max(v2),
            },
            pass_up: None,
        })
    }

    /// Marks this line as the seam of one-sided floors facing `up`.
    pub fn sealing(mut self, up: Vec3) -> Self {
        self.pass_up = Some(up);
        self
    }

    pub fn material(&self) -> &Material {
        &self.upright.material
    }

    pub fn bbox(&self) -> Aabb {
        self.bbox
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        // A seam is passable exactly where its floors are: a triangle takes
        // no ball moving along its normal (`collideex.cpp:838`, the receding
        // test), so neither does the stitching between two of them.
        if let Some(up) = self.pass_up
            && up.dot(ball.vel) > C_CONTACTVEL
        {
            return None;
        }
        let mut upright_ball = ball.clone();
        upright_ball.pos = self.to_upright * ball.pos;
        upright_ball.vel = self.to_upright * ball.vel;

        let mut hit = self.upright.hit_test(&upright_ball, dtime)?;
        // The normal comes back in the rotated frame; the transpose of a
        // rotation is its inverse.
        hit.hit_normal = self.to_upright.transpose() * hit.hit_normal;
        Some(hit)
    }
}

/// The shortest rotation taking `from` to `to`, both unit vectors.
///
/// Written out rather than taken from `glam` so the antiparallel case is
/// explicit: a ramp that doubles back on itself produces segments pointing
/// straight down, and `from_rotation_arc` is documented as unspecified there.
fn glam_rotation_arc(from: Vec3, to: Vec3) -> Quat {
    let dot = from.dot(to).clamp(-1.0, 1.0);
    if dot > 1.0 - 1e-6 {
        return Quat::IDENTITY;
    }
    if dot < -1.0 + 1e-6 {
        // Half a turn about any axis square to `from`.
        let axis = if from.x.abs() > 0.9 {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let axis = from.cross(axis).normalize();
        return Quat::from_axis_angle(axis, std::f32::consts::PI);
    }
    let axis = from.cross(to).normalize();
    Quat::from_axis_angle(axis, dot.acos())
}
