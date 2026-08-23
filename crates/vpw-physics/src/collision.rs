//! Collision detection and response.
//!
//! The core is [`Ball::collide_3d_wall`]: given a contact point and a normal,
//! it decides how the ball bounces off. Every shape —walls, circles,
//! triangles, ramps— ends up calling into it.
//!
//! Port of `HitBall::Collide3DWall` (`physics/hitball.cpp:42`) and of
//! `HitObject` (`physics/collide.h`).

use crate::ball::Ball;
use crate::constants::{
    C_CONTACTVEL, C_DISP_GAIN, C_DISP_LIMIT, C_EMBEDDED, C_EMBEDSHOT, C_LOWNORMVEL, C_PRECISION,
};
use vpw_math::{Vec2, Vec3};

/// What is known about a hit that has not happened yet.
#[derive(Debug, Clone, Copy)]
pub struct CollisionEvent {
    /// When it is going to happen, in VP time from now.
    pub hit_time: f32,
    /// Normal of the surface at the contact point.
    pub hit_normal: Vec3,
    /// How far the ball is penetrating. Negative means it already got in.
    pub hit_distance: f32,
    /// Whether the ball ended up so far inside that it has to be pushed out.
    pub is_contact: bool,
    /// For objects that are not rigid —triggers— whether the ball is entering
    /// or leaving.
    pub hit_flag: bool,
    /// Normal velocity at the moment of contact, for contacts.
    pub hit_org_normal_velocity: f32,
    /// The velocity the hit **surface** is moving at.
    ///
    /// Almost everything on a table is still and this is zero. The tip of the
    /// plunger is not: it is moving when it hits the ball, and the collision
    /// has to be resolved with the relative velocity (`hitplunger.cpp:718`).
    pub hit_vel: Vec2,
}

impl Default for CollisionEvent {
    fn default() -> Self {
        Self {
            hit_time: 0.0,
            hit_normal: Vec3::ZERO,
            hit_distance: 0.0,
            is_contact: false,
            hit_flag: false,
            hit_org_normal_velocity: 0.0,
            hit_vel: Vec2::ZERO,
        }
    }
}

/// The physical properties of a surface.
#[derive(Debug, Clone, Copy)]
pub struct Material {
    pub elasticity: f32,
    /// How much the elasticity drops as the impact velocity goes up.
    pub elasticity_falloff: f32,
    pub friction: f32,
    /// Random scatter of the bounce, in radians. Negative uses the table's
    /// global value.
    pub scatter: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            elasticity: 0.3,
            elasticity_falloff: 0.0,
            friction: 0.3,
            scatter: 0.0,
        }
    }
}

/// Effective elasticity at a given velocity (`collide.h:56`).
///
/// A harder hit bounces **less**: it is what keeps a ball thrown against a
/// wall from coming back out as fast as it arrived.
pub fn elasticity_with_falloff(elasticity: f32, falloff: f32, vel: f32) -> f32 {
    if falloff > 0.0 {
        elasticity / (1.0 + falloff * vel.abs() * (1.0 / 18.53))
    } else {
        elasticity
    }
}

impl Ball {
    /// Velocity of a point on the ball's surface.
    ///
    /// It is the center's plus whatever the spin contributes. Without this
    /// term the ball always slides and never rolls (`hitball.cpp:370`).
    pub fn surface_velocity(&self, surf_p: Vec3) -> Vec3 {
        self.vel + self.angular_velocity().cross(surf_p)
    }

    /// Applies an impulse at a point on the surface (`hitball.cpp:383`).
    pub fn apply_surface_impulse(&mut self, rot_i: Vec3, impulse: Vec3) {
        self.vel += impulse / self.mass;
        self.angular_momentum += rot_i;
    }

    /// The bounce off a wall (`hitball.cpp:42`).
    ///
    /// `scatter_extra` comes from the caller's random number generator: it has
    /// to be in -1..1. It is passed in from outside on purpose, so the
    /// simulation is reproducible in the tests.
    pub fn collide_3d_wall(
        &mut self,
        hit_normal: Vec3,
        material: &Material,
        coll: &CollisionEvent,
        scatter_extra: f32,
    ) {
        let mut dot = self.vel.dot(hit_normal);

        // If the ball is already moving away there is nothing to resolve —
        // unless it ended up embedded, and then it gets a shove to get it out.
        if dot >= -C_LOWNORMVEL {
            if dot > C_LOWNORMVEL {
                return;
            }
            if coll.hit_distance < -C_EMBEDDED {
                dot = -C_EMBEDSHOT;
            } else {
                return;
            }
        }

        // Push the ball back out. The limit exists because when crossing a
        // ramp the measured distance jumps around, and without a cap the ball
        // makes a visible hop.
        let mut hdist = -C_DISP_GAIN * coll.hit_distance;
        if hdist > 1.0e-4 {
            if hdist > C_DISP_LIMIT {
                hdist = C_DISP_LIMIT;
            }
            self.pos += hit_normal * hdist;
        }

        let reaction_impulse = self.mass * dot.abs();

        let elasticity =
            elasticity_with_falloff(material.elasticity, material.elasticity_falloff, dot);
        // Careful: the original **reassigns** `dot` here, and further down
        // reuses it to decide whether there is scatter. It stops being the
        // approach velocity and becomes the total change in velocity, which is
        // positive.
        dot *= -(1.0 + elasticity);
        // The impulse goes along the normal, so it generates no torque.
        self.vel += hit_normal * dot;

        // Friction: it acts at the contact point and it **does** generate
        // torque, which is what makes the ball start to roll.
        let surf_p = hit_normal * -self.radius;
        let surf_vel = self.surface_velocity(surf_p);
        let mut tangent = surf_vel - hit_normal * surf_vel.dot(hit_normal);

        let tangent_sp_sq = tangent.length_squared();
        if tangent_sp_sq > 1e-6 {
            tangent /= tangent_sp_sq.sqrt();
            let vt = surf_vel.dot(tangent);

            let cross = surf_p.cross(tangent);
            let kt = 1.0 / self.mass + tangent.dot((cross / self.inertia()).cross(surf_p));

            // Friction cannot go beyond what the normal impulse allows: it is
            // the Coulomb cone.
            let max_fric = material.friction * reaction_impulse;
            let jt = (-vt / kt).clamp(-max_fric, max_fric);

            if jt.is_finite() {
                self.apply_surface_impulse(cross * jt, tangent * jt);
            }
        }

        // Scatter. Only if the change in velocity was large: at low velocity
        // it would mess up the ball coming to rest.
        let scatter_angle = material.scatter;
        if dot > 1.0 && scatter_angle > 1.0e-5 {
            // Quadratic distribution, as in the original: more likely near
            // zero than at the extremes.
            let s = scatter_extra;
            let scatter = s * (1.0 - s * s) * 2.59808 * scatter_angle;
            let (sin, cos) = scatter.sin_cos();
            let (vx, vy) = (self.vel.x, self.vel.y);
            self.vel.x = vx * cos - vy * sin;
            self.vel.y = vy * cos + vx * sin;
        }
    }

    /// Acceleration of a point on the surface (`hitball.cpp:375`).
    ///
    /// It is the linear one from gravity plus the centripetal one the spin
    /// contributes.
    pub fn surface_acceleration(&self, surf_p: Vec3, gravity: Vec3) -> Vec3 {
        let w = self.angular_velocity();
        gravity / self.mass + w.cross(w.cross(surf_p))
    }

    /// Resolves a **contact**: the ball resting on something, with no hit.
    ///
    /// Port of `HitBall::HandleStaticContact` (`hitball.cpp:287`). It is what
    /// holds the ball up on the playfield instead of letting it sink.
    ///
    /// The normal force is computed to cancel exactly what gravity pushes
    /// against the surface in this step, and is **never negative**: a surface
    /// pushes, it does not pull.
    pub fn handle_static_contact(
        &mut self,
        coll: &CollisionEvent,
        friction: f32,
        dtime: f32,
        gravity: Vec3,
    ) {
        // Should be zero, but only up to +/- C_CONTACTVEL.
        let norm_vel = self.vel.dot(coll.hit_normal);
        if norm_vel > C_CONTACTVEL {
            return;
        }

        let fe = gravity * self.mass;
        let dot = fe.dot(coll.hit_normal);
        let normal_force = (-(dot * dtime + coll.hit_org_normal_velocity)).max(0.0);

        self.vel += coll.hit_normal * normal_force;
        self.apply_friction(coll.hit_normal, dtime, friction, gravity);
    }

    /// Friction at a resting point (`hitball.cpp:320`).
    ///
    /// It distinguishes the two regimes the original distinguishes: with the
    /// ball sliding, friction opposes the sliding **velocity**; with the ball
    /// practically still, it opposes the **acceleration** — which is what
    /// keeps a ball resting on a slope from starting to slide on its own.
    pub fn apply_friction(&mut self, hit_normal: Vec3, dtime: f32, fric_coeff: f32, gravity: Vec3) {
        let surf_p = hit_normal * -self.radius;
        let surf_vel = self.surface_velocity(surf_p);
        let slip = surf_vel - hit_normal * surf_vel.dot(hit_normal);

        // The cap on friction comes from how hard gravity presses against this
        // surface.
        //
        // The `max(0)` is **not** in the original, and it is needed: when the
        // normal does not oppose gravity —the glass above, the underside of a
        // plastic— the expression comes out negative, and there C++'s `clamp`
        // with the minimum greater than the maximum is undefined behaviour. It
        // silently returns garbage. A negative friction does not mean
        // anything: if the surface is not pressing, there is no friction.
        let max_fric = (fric_coeff * self.mass * -gravity.dot(hit_normal)).max(0.0);

        let slip_speed = slip.length();
        // Which of the two regimes applies, and the first half of the question
        // is the one that is easy to lose (`hitball.cpp:335-341`). It sits
        // behind `C_BALL_SPIN_HACK`, which `physconst.h:89` defines, so it is
        // live in every build the original ships.
        //
        // Without it a ball that is **resting on** a surface — its normal
        // velocity nothing — and **sliding along** it goes to the dynamic
        // branch, where friction opposes the slip velocity up to the full
        // Coulomb cap and kills the skid within a few milliseconds. With it,
        // that ball goes to the static branch, where friction opposes the
        // tangential *acceleration* instead: the rolling constraint, which is
        // what turns a skid into a roll rather than stopping it.
        //
        // Every shot off a flipper starts as a skid. Which branch it takes
        // decides where it ends up.
        let normal_vel = self.vel.dot(hit_normal);
        let (slip_dir, numer) = if normal_vel <= 0.025 || slip_speed < C_PRECISION {
            // Static friction: it opposes the acceleration.
            let surf_acc = self.surface_acceleration(surf_p, gravity);
            let slip_acc = surf_acc - hit_normal * surf_acc.dot(hit_normal);
            if slip_acc.length_squared() < 1e-6 {
                return; // it neither slides nor tends to slide
            }
            let dir = slip_acc.normalize();
            (dir, -dir.dot(surf_acc))
        } else {
            // Dynamic friction: it opposes the velocity.
            let dir = slip / slip_speed;
            (dir, -dir.dot(surf_vel))
        };

        let cp = surf_p.cross(slip_dir);
        let denom = 1.0 / self.mass + slip_dir.dot((cp / self.inertia()).cross(surf_p));
        let fric = (numer / denom).clamp(-max_fric, max_fric);

        if fric.is_finite() {
            self.apply_surface_impulse(cp * (dtime * fric), slip_dir * (dtime * fric));
        }
    }
}
