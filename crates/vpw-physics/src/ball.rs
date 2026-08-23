//! The ball.
//!
//! Port of `HitBall` (`physics/hitball.cpp` and `.h`). It is the only object
//! that moves on its own: everything else on the table is either still or
//! moved by the script.
//!
//! # The units
//!
//! Positions are in VPU and times in VPT, Visual Pinball's internal time unit.
//! A standard-sized ball measures 25 VPU of radius and gravity at a 6 degree
//! slope is worth 0.97 in those units — see [`crate::constants`].

use crate::collision::CollisionEvent;
use crate::constants::{
    C_CONTACTVEL, C_DISP_GAIN, C_DISP_LIMIT, C_EMBEDDED, C_EMBEDSHOT, C_LOWNORMVEL,
    DEFAULT_BALL_SIZE, PHYS_FACTOR, PHYS_TOUCH,
};
use vpw_math::{Mat3, Quat, Vec2, Vec3};

/// How many slots of position history a ball keeps, one per ten milliseconds
/// (`MAX_BALL_TRAIL_POS`, `hitball.h:11`). A tenth of a second.
pub const BALL_TRAIL: usize = 10;

/// State of a ball.
#[derive(Debug, Clone)]
pub struct Ball {
    pub pos: Vec3,
    pub vel: Vec3,
    /// The velocity it had a moment ago, kept only for the kicker.
    ///
    /// A ball creeping over the rim of a kicker that is not going to catch it
    /// loses almost all its speed to friction and stops dead on the lip. The
    /// original calls the workaround an ugly hack and keeps it anyway
    /// (`kicker.cpp:1134`): if the ball drops below a crawl while brushing a
    /// kicker, give it back the speed it had. Without it a table has invisible
    /// sticky spots wherever a saucer is.
    pub old_vel: Vec3,
    /// Angular momentum. The ball rolls, and that matters for friction and
    /// for drawing it spinning.
    pub angular_momentum: Vec3,
    /// Orientation, so the texture can be drawn spinning with the ball.
    ///
    /// The original keeps a 3x3 matrix and reorthonormalizes it on every step
    /// (`hitball.cpp:453`, `Matrix3::OrthoNormalize`), because integrating a
    /// matrix gradually pushes it out of being a pure rotation: the axes drift
    /// out of alignment and stop having unit length.
    ///
    /// Here we use a quaternion. It is better for three concrete reasons: it
    /// is four numbers instead of nine, normalizing is one square root instead
    /// of a Gram-Schmidt over three vectors, and above all it **cannot
    /// degenerate** —a normalized quaternion always represents a rotation,
    /// whereas a badly integrated matrix can end up shearing or mirroring the
    /// ball.
    pub orientation: Quat,
    pub radius: f32,
    pub mass: f32,
    /// Whether it is captured in a kicker. A captured ball neither moves nor
    /// receives gravity.
    pub locked: bool,
    /// Squared distance travelled since the last event, so we do not repeat
    /// script firings on the same trigger.
    pub last_event_sqr_dist: f32,
    /// Where the ball was when it last fired one.
    ///
    /// Starts far away (`hitball.cpp:19` puts it at -10000 on every axis) so
    /// that the very first event of a ball's life is never mistaken for a
    /// repeat of one that never happened.
    pub last_event_pos: Vec3,
    /// How much it moved in the last step, squared.
    pub drsq: f32,
    /// Where the ball was, one slot per ten milliseconds
    /// (`HitBall::OnPhysicStepProcessed`, `hitball.cpp:27`).
    ///
    /// A hundred milliseconds of history, and the only thing it is for is
    /// telling a ball that has stopped moving from one that is still going:
    /// see [`Ball::old_position`]. Slots start at `f32::MAX`, which is the
    /// original's "nothing written here yet" (`PhysicsEngine.cpp:698`).
    pub old_pos: [Vec3; BALL_TRAIL],
}

impl Default for Ball {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            old_vel: Vec3::ZERO,
            angular_momentum: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            radius: DEFAULT_BALL_SIZE,
            mass: 1.0,
            locked: false,
            last_event_sqr_dist: 0.0,
            last_event_pos: Vec3::splat(-10000.0),
            drsq: 0.0,
            old_pos: [Vec3::splat(f32::MAX); BALL_TRAIL],
        }
    }
}

impl Ball {
    /// A ball at rest at a point.
    pub fn new(pos: Vec3, radius: f32) -> Self {
        Self {
            pos,
            radius,
            ..Self::default()
        }
    }

    /// Moment of inertia of a solid sphere: `2/5 · m · r²` (`hitball.h:59`).
    pub fn inertia(&self) -> f32 {
        0.4 * self.radius * self.radius * self.mass
    }

    /// Remembers where the ball is, in the slot for this millisecond.
    pub fn record_position(&mut self, time_ms: u64) {
        self.old_pos[Self::slot(time_ms)] = self.pos;
    }

    /// Where the ball was at that millisecond, or `None` if that far back has
    /// not happened yet — a ball less than a tenth of a second old, or one that
    /// has just been created.
    pub fn old_position(&self, time_ms: u64) -> Option<Vec3> {
        let p = self.old_pos[Self::slot(time_ms)];
        (p.x != f32::MAX).then_some(p)
    }

    fn slot(time_ms: u64) -> usize {
        ((time_ms / 10) % BALL_TRAIL as u64) as usize
    }

    /// Angular velocity, derived from the momentum.
    pub fn angular_velocity(&self) -> Vec3 {
        self.angular_momentum / self.inertia()
    }

    /// Whether a collision here should reach the script, and remembers it if so.
    ///
    /// Port of the filter at the top of `HitObject::FireHitEvent`
    /// (`collide.cpp:19-36`). A ball that has come to rest against something
    /// keeps colliding with it, once per step, for as long as it sits there —
    /// and every one of those is the *same* hit as far as a table is concerned.
    /// Without this a resting ball pulses its switch a thousand times a second:
    /// on F-14 that is `vpmTimer.PulseSw` on a standup target the ball happens
    /// to be leaning on, scoring for ever.
    ///
    /// The two constants are the original's, and it calls the first one magic
    /// itself: half a square unit of travel is what counts as "the ball has
    /// been somewhere since", and a quarter is how far it has to have moved
    /// from where it last fired.
    pub fn fires_event(&mut self) -> bool {
        if self.last_event_sqr_dist < 0.5 {
            let moved = (self.last_event_pos - self.pos).length_squared();
            if moved <= 0.25 {
                return false;
            }
        }
        self.last_event_sqr_dist = 0.0;
        self.last_event_pos = self.pos;
        true
    }

    /// Moves the ball and spins its orientation (`hitball.cpp:444`).
    pub fn update_displacements(&mut self, dtime: f32) {
        if self.locked {
            return;
        }
        let ds = self.vel * dtime;
        self.pos += ds;

        self.last_event_sqr_dist += ds.length_squared();
        self.drsq = ds.length_squared();

        // Integrate the orientation: dq/dt = ½·ω·q, with the angular velocity
        // as a pure quaternion. Normalizing at the end is enough for it to
        // stay an exact rotation.
        let w = self.angular_velocity();
        if w.length_squared() > 0.0 {
            let omega = Quat::from_xyzw(w.x, w.y, w.z, 0.0);
            let derivative = omega * self.orientation;
            let integrated = Quat::from_xyzw(
                self.orientation.x + derivative.x * 0.5 * dtime,
                self.orientation.y + derivative.y * 0.5 * dtime,
                self.orientation.z + derivative.z * 0.5 * dtime,
                self.orientation.w + derivative.w * 0.5 * dtime,
            );
            self.orientation = integrated.normalize();
        }
    }

    /// The orientation as a matrix, which is what the renderer consumes.
    pub fn orientation_matrix(&self) -> Mat3 {
        Mat3::from_quat(self.orientation)
    }

    /// Applies gravity and the table's acceleration (`hitball.cpp:474`).
    ///
    /// `nudge` is the cabinet's acceleration, already in VP units. The original
    /// subtracts it —it does not add it— because it is a fictitious force: the
    /// cabinet moves and the ball, out of inertia, stays put.
    pub fn update_velocities(&mut self, gravity: Vec3, nudge: Vec3, slope_rad: f32) {
        if !self.locked {
            // dV = (1/m)·(ΣF)·dt, with gravity already expressed as an
            // acceleration. `PHYS_FACTOR` is the duration of one physics step
            // in VP time units.
            self.vel += gravity * PHYS_FACTOR;

            // The nudge acts in the horizontal plane of the cabinet, but the
            // frame of reference is the playfield, which is tilted: that is
            // why the `y` component gets split between `y` and `z`.
            self.vel.x -= PHYS_FACTOR * nudge.x;
            self.vel.y -= PHYS_FACTOR * nudge.y * slope_rad.cos();
            self.vel.z -= PHYS_FACTOR * nudge.y * slope_rad.sin();
        }
    }

    /// Bounding box for the broadphase.
    ///
    /// The original uses a **cubic** box the size of the velocity plus the
    /// radius (`hitball.cpp:404`), not the one of the actual path: the ball
    /// can bounce within the same step and go off in any direction.
    pub fn hit_bbox(&self) -> crate::Aabb {
        let vl = self.vel.length() + self.radius + 0.05;
        crate::Aabb {
            min: self.pos - Vec3::splat(vl),
            max: self.pos + Vec3::splat(vl),
        }
    }

    /// Whether the ball is still enough to stop spinning it.
    ///
    /// The thresholds come from `PhysicsEngine.cpp:705-711` and are scaled by
    /// gravity, so they hold the same on a table with more or less slope. The
    /// original uses them for the `C_BALL_SPIN_HACK`: a ball that has come to
    /// a stop but keeps spinning looks wrong, so it brakes the spin.
    ///
    /// Note that there is **no** ball-sleeping mechanism: the original had one
    /// (`C_DYNAMIC`) and it has been commented out for years, with the note
    /// "does not work stable enough anymore nowadays" (`physconst.h:87`).
    pub fn is_at_rest(&self, gravity_strength: f32) -> bool {
        use crate::constants::GRAVITYCONST;
        let g = gravity_strength / GRAVITYCONST;
        let mag = self.vel.x * self.vel.x + self.vel.y * self.vel.y;
        self.drsq < 8.0e-5
            && mag < 1.0e-3 * g * g
            && self.vel.z.abs() < 0.2 * g
            && self.angular_momentum.length() < 0.9 * g
    }
}

/// How much two balls bounce off each other.
///
/// It is a constant and does not come from any material: in the original it is
/// hand-written in `HitBall::Collide` (`hitball.cpp:635`). It makes sense —two
/// steel balls bounce like two steel balls, not like whatever wall happens to
/// be next to them— but it is worth making it visible that this is a decision
/// and not an oversight.
const BALL_TO_BALL_RESTITUTION: f32 = 0.8;

/// When two balls are going to touch, if they touch at all.
///
/// Port of `HitBall::HitTest` (`hitball.cpp:520`). The normal it returns
/// points **from `a` towards `b`**.
///
/// The general case is a quadratic: two spheres moving at constant velocity
/// touch when the distance between their centers equals the sum of the radii,
/// and that gives a second-degree equation in time. The other case —already
/// overlapping— is solved like a wall's: if they come in fast or are deeply
/// embedded, time zero; otherwise the time is spread out inside the contact
/// band so it does not compete with zero-time events.
///
/// The original also has a path for two balls exactly center on center that
/// has been **disabled** for years, with a comment wondering whether it was
/// needed at all. We do not port it: if it ever happened, the normal comes out
/// degenerate and the hit is discarded, which is what the original does today.
pub fn hit_test_balls(a: &Ball, b: &Ball, dtime: f32) -> Option<CollisionEvent> {
    let d = a.pos - b.pos;
    let dv = a.vel - b.vel;

    let bcddsq = d.length_squared();
    let bcdd = bcddsq.sqrt();
    if bcdd < 1.0e-8 {
        return None; // center on center: no normal to use
    }

    let bb = dv.dot(d);
    let bnv = bb / bcdd;
    if bnv > C_LOWNORMVEL {
        return None; // they are separating
    }

    let total_radius = a.radius + b.radius;
    let bnd = bcdd - total_radius; // distance between surfaces

    let hittime = if bnd <= PHYS_TOUCH {
        if bnd < b.radius * -2.0 {
            return None; // too deeply embedded: discard it
        }
        if bnv.abs() > C_CONTACTVEL || bnd <= -PHYS_TOUCH {
            0.0
        } else {
            bnd * (1.0 / (2.0 * PHYS_TOUCH)) + 0.5
        }
    } else {
        let aa = dv.length_squared();
        if aa < 1.0e-8 {
            return None; // almost no relative velocity: wait for the contact
        }
        let (t1, t2) = solve_quadratic(aa, 2.0 * bb, bcddsq - total_radius * total_radius)?;
        // The smaller of the two roots, unless one is negative: there the
        // positive one is the good one, because the negative one is a crossing
        // that already happened.
        if t1 * t2 < 0.0 {
            t1.max(t2)
        } else {
            t1.min(t2)
        }
    };

    if !hittime.is_finite() || hittime < 0.0 || hittime > dtime {
        return None;
    }

    // Where `b` ends up relative to `a` at the instant of the hit.
    let hit_pos = b.pos + dv * hittime;
    let normal = hit_pos - a.pos;
    if normal.length_squared() <= f32::MIN_POSITIVE {
        return None;
    }

    Some(CollisionEvent {
        hit_time: hittime,
        hit_normal: normal.normalize(),
        hit_distance: bnd,
        // The original has ball-to-ball contacts **turned off**
        // (`BALL_CONTACTS` in `physconst.h:67`, "not working anymore?!?").
        // Same case as `C_DYNAMIC`: we do not port something the original
        // disabled for being unstable.
        is_contact: false,
        hit_flag: false,
        hit_org_normal_velocity: 0.0,
        hit_vel: Vec2::ZERO,
    })
}

/// `at² + bt + c = 0`, with the two roots ordered the way the original returns
/// them (`SolveQuadraticEq`, `math.cpp:...`).
fn solve_quadratic(a: f32, b: f32, c: f32) -> Option<(f32, f32)> {
    let discr = b * b - 4.0 * a * c;
    if discr < 0.0 {
        return None;
    }
    let discr = discr.sqrt();
    let inv_a = -0.5 / a;
    Some(((b + discr) * inv_a, (b - discr) * inv_a))
}

/// Resolves the collision between two balls.
///
/// Port of `HitBall::Collide` (`hitball.cpp:600`). The normal points **from
/// `a` towards `b`**, the same one [`hit_test_balls`] returns.
///
/// The pair is resolved in one go and symmetrically. The original cannot do
/// that: each ball looks for its own collision separately, so when A's best
/// option is B and B's is A, the pair would be resolved twice. It avoids that
/// by discarding one of the two based on comparing their pointers, and since
/// that introduces a bias from the memory ordering, it also flips the criterion
/// on every cycle (`m_swap_ball_collision_handling`). Resolving the pair only
/// once, there is no bias to compensate for and that whole machinery is
/// unnecessary.
///
/// A ball captured in a kicker has **infinite mass**: it does not move, and the
/// other one bounces off it as off a wall.
pub fn collide_balls(a: &mut Ball, b: &mut Ball, coll: &CollisionEvent) {
    let normal = coll.hit_normal;
    let mut dot = (b.vel - a.vel).dot(normal);

    if dot >= -C_LOWNORMVEL {
        if dot > C_LOWNORMVEL {
            return; // they are separating
        }
        if coll.hit_distance < -C_EMBEDDED {
            dot = -C_EMBEDSHOT; // they ended up embedded: a shove to pull them apart
        } else {
            return;
        }
    }

    // Positional separation, for the low velocities where the impulse alone is
    // not enough. The original corrects each ball with a different value and
    // leaves a note saying the second one is noisy and should be investigated
    // (`hitball.cpp:625`). Here the same correction is split half and half,
    // which is symmetric and what the `* 0.5` on both sides meant to say.
    let mut sep = -C_DISP_GAIN * coll.hit_distance;
    if sep > 1.0e-4 {
        sep = sep.min(C_DISP_LIMIT);
        match (a.locked, b.locked) {
            (true, true) => {}
            // Against a locked ball, the moving one takes the whole shift.
            (true, false) => b.pos += normal * sep,
            (false, true) => a.pos -= normal * sep,
            (false, false) => {
                b.pos += normal * (sep * 0.5);
                a.pos -= normal * (sep * 0.5);
            }
        }
    }

    let inv_mass_a = if a.locked { 0.0 } else { 1.0 / a.mass };
    let inv_mass_b = if b.locked { 0.0 } else { 1.0 / b.mass };
    if inv_mass_a + inv_mass_b <= 0.0 {
        return; // two locked balls: there is nothing to resolve
    }

    let impulse = -(1.0 + BALL_TO_BALL_RESTITUTION) * dot / (inv_mass_a + inv_mass_b);
    a.vel -= normal * (impulse * inv_mass_a);
    b.vel += normal * (impulse * inv_mass_b);
}
