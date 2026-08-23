//! The flipper.
//!
//! It is the most complex piece of the engine and the one that defines how the
//! game feels. Unlike everything else, it **moves on its own**: it has a
//! solenoid that applies torque to it, a stop it brakes against, and an
//! angular momentum of its own that it exchanges with the ball.
//!
//! Port of `physics/hitflipper.cpp`.
//!
//! # The shape
//!
//! A flipper is two circles —the base, fixed, and the tip, which rotates
//! around the base— joined by two flat faces. The ball can hit any of the four
//! things.
//!
//! # The detail that makes it feel right
//!
//! When the flipper is resting against its stop and the ball pushes it **in
//! the same direction the solenoid is pushing**, the original does not
//! transfer kinetic energy to the flipper (`hitflipper.cpp:908-927`). Without
//! that, a shot against a flipper at rest comes out dead: the ball "lends"
//! energy to something that cannot move.

use crate::Aabb;
use crate::ball::Ball;
use crate::collision::{CollisionEvent, Material, elasticity_with_falloff};
use crate::constants::{
    C_CONTACTVEL, C_DISP_GAIN, C_DISP_LIMIT, C_EMBEDDED, C_EMBEDSHOT, C_LOWNORMVEL, C_PRECISION,
    PHYS_FACTOR, PHYS_TOUCH,
};
use vpw_math::{Vec2, Vec3};

/// The moving state of a flipper (`FlipperMoverObject`).
#[derive(Debug, Clone)]
pub struct Flipper {
    /// Center of the base, which is the axis of rotation.
    pub center: Vec2,
    pub base_radius: f32,
    pub end_radius: f32,
    /// Distance between the two centers.
    pub flipper_radius: f32,
    pub z_low: f32,
    pub z_high: f32,

    /// Rest and shot angles, in radians.
    pub angle_start: f32,
    pub angle_end: f32,
    /// Current angle.
    pub angle_cur: f32,

    pub angular_momentum: f32,
    pub angle_speed: f32,
    pub inertia: f32,

    /// Force of the solenoid.
    pub strength: f32,
    /// What fraction of the force the return has.
    pub return_ratio: f32,
    /// How long the coil takes to reach full force.
    pub ramp_up: f32,
    /// Damping near the stop, and from which angle it kicks in.
    ///
    /// The angle is in **radians**, like every other angle here. A `.vpx`
    /// stores it in degrees and the original converts on the way in
    /// (`ANGTORAD`, `hitflipper.cpp:369`); leaving it in degrees makes the
    /// comparison below true for the whole stroke, since a flipper's entire
    /// travel is about 1.2 radians and the default is 6. The flipper then runs
    /// at the hold-coil's strength from the moment it starts moving — about
    /// three quarters of what the table asked for, every flip.
    pub torque_damping: f32,
    pub torque_damping_angle: f32,

    /// Whether the button is pressed.
    pub solenoid: bool,
    /// Angular acceleration from the coil this step (`hitflipper.cpp:420`).
    ///
    /// Only a contact needs it: a ball resting on a flipper has to be told how
    /// hard the arm underneath it is accelerating, which is a different
    /// question from how fast it is already turning.
    pub angular_acceleration: f32,
    /// Accumulated torque of the coil.
    cur_torque: f32,
    /// Whether it is resting against a stop, and with how much torque.
    pub in_contact: bool,
    contact_torque: f32,

    pub material: Material,

    /// The normal of the face with the flipper at angle zero.
    ///
    /// Since the base and the tip have different radii, the two faces that
    /// join them are **not parallel to the arm**: they are slanted by the
    /// angle whose sine is `(base - tip) / length`. This vector is that normal
    /// already worked out (`hitflipper.cpp:72`).
    zero_ang_norm: Vec2,
}

/// The data a flipper is built from.
///
/// They go in a struct because there are fourteen of them, almost all `f32`,
/// and several look alike —three different radii, two angles, two dampings—.
/// As positional arguments, swapping two of them compiles just the same and
/// gives a flipper that moves wrong without anything warning you.
#[derive(Debug, Clone)]
pub struct FlipperParams {
    pub center: Vec2,
    /// Radius of the big circle, the one at the axis.
    pub base_radius: f32,
    /// Radius of the small circle, the one at the tip.
    pub end_radius: f32,
    /// Distance between the two centers. It is **not** the total length.
    pub flipper_radius: f32,
    pub z_low: f32,
    pub z_high: f32,
    /// In radians. In the `.vpx` they come in degrees.
    pub angle_start: f32,
    pub angle_end: f32,
    pub mass: f32,
    /// Force of the solenoid going up.
    pub strength: f32,
    /// What fraction of the force it uses to come back. Going down is much
    /// gentler than going up.
    pub return_ratio: f32,
    /// How long the solenoid takes to give all of its force. It is what keeps
    /// a flipper from hitting at full power the instant the button is pressed.
    pub ramp_up: f32,
    /// Braking on reaching the stop, and from which angle it starts to apply.
    pub torque_damping: f32,
    pub torque_damping_angle: f32,
    pub material: Material,
}

impl Default for FlipperParams {
    /// The original's default values, for the fields old tables do not carry.
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            base_radius: 21.5,
            end_radius: 13.0,
            flipper_radius: 130.0,
            z_low: 0.0,
            z_high: 50.0,
            angle_start: 0.0,
            angle_end: std::f32::consts::FRAC_PI_3,
            mass: 1.0,
            strength: 2200.0,
            return_ratio: 0.09,
            ramp_up: 3.0,
            torque_damping: 0.75,
            // Six degrees, the original's default (`Settings_properties.inl`).
            torque_damping_angle: 6.0f32.to_radians(),
            material: Material::default(),
        }
    }
}

impl Flipper {
    /// Builds the flipper.
    ///
    /// Port of `FlipperMoverObject::FlipperMoverObject` (`hitflipper.cpp:29`)
    /// and of what `Flipper::PhysicSetup` (`flipper.cpp:...`) passes to it.
    pub fn new(d: FlipperParams) -> Self {
        // The radii cannot be zero: the original clamps them to 0.01.
        let base_radius = d.base_radius.max(0.01);
        let end_radius = d.end_radius.max(0.01);
        let flipper_radius = d.flipper_radius.max(0.01);

        // If the two angles coincide the flipper has no travel and the
        // collision loop hangs looking for a crossing that never comes. The
        // original adds a ten-thousandth to it; it is ugly but it is what
        // there is.
        let angle_end = if d.angle_end == d.angle_start {
            d.angle_end + 0.0001
        } else {
            d.angle_end
        };

        // The moment of inertia of a bar rotating about one end.
        let inertia = (1.0 / 3.0) * d.mass * flipper_radius * flipper_radius;
        let ratio = (base_radius - end_radius) / flipper_radius;
        let zero_ang_norm = Vec2::new((1.0 - ratio * ratio).max(0.0).sqrt(), -ratio);

        Self {
            center: d.center,
            base_radius,
            end_radius,
            flipper_radius,
            z_low: d.z_low,
            z_high: d.z_high,
            angle_start: d.angle_start,
            angle_end,
            angle_cur: d.angle_start,
            angular_momentum: 0.0,
            angle_speed: 0.0,
            inertia,
            strength: d.strength,
            return_ratio: d.return_ratio,
            ramp_up: d.ramp_up,
            torque_damping: d.torque_damping,
            torque_damping_angle: d.torque_damping_angle,
            solenoid: false,
            angular_acceleration: 0.0,
            cur_torque: 0.0,
            in_contact: false,
            contact_torque: 0.0,
            material: d.material,
            zero_ang_norm,
        }
    }

    /// The box that contains it, with the flipper in **any** position.
    ///
    /// It is the whole circle the tip sweeps, and not the sector it actually
    /// travels. The original has the version fitted to the sector written down
    /// and **disabled**, with the reason next to it: it does not cope with a
    /// script changing the angles mid-game (`hitflipper.cpp:CalcHitBBox`).
    ///
    /// Here it weighs even more: the tree is built once when the table loads,
    /// so a box that depended on the angles would be a time bomb —the day a
    /// flipper changes its travel, the tree loses it and the ball goes through
    /// it—. And the saving would be a handful of candidates over two flippers.
    pub fn bbox(&self) -> Aabb {
        let reach = self.flipper_radius + self.end_radius + 0.1;
        Aabb {
            min: Vec3::new(self.center.x - reach, self.center.y - reach, self.z_low),
            max: Vec3::new(self.center.x + reach, self.center.y + reach, self.z_high),
        }
    }

    /// Where the center of the tip is right now.
    pub fn tip_center(&self) -> Vec2 {
        self.center + Vec2::new(self.angle_cur.sin(), -self.angle_cur.cos()) * self.flipper_radius
    }

    /// Velocity of a point rigidly attached to the flipper.
    ///
    /// It only rotates about `z`, so the velocity is the cross product of the
    /// angular velocity with the arm.
    pub fn surface_velocity(&self, r: Vec3) -> Vec3 {
        Vec3::new(0.0, 0.0, self.angle_speed).cross(r)
    }

    /// Acceleration of a point rigidly attached to the flipper
    /// (`hitflipper.cpp:459`): the tangential part from the coil, plus the
    /// centripetal part from already turning.
    pub fn surface_acceleration(&self, r: Vec3) -> Vec3 {
        let tangential = Vec3::new(0.0, 0.0, self.angular_acceleration).cross(r);
        let av2 = self.angle_speed * self.angle_speed;
        tangential + Vec3::new(-av2 * r.x, -av2 * r.y, 0.0)
    }

    /// A ball **resting on** the flipper (`HitFlipper::Contact`,
    /// `hitflipper.cpp:1012`).
    ///
    /// A contact is not a hit. The generic one treats whatever the ball is
    /// lying on as an immovable wall and cancels gravity against it, which is
    /// right for a plastic and wrong for the one part of the table that is
    /// itself accelerating. Without this the arm does not lift a cradled ball
    /// as it begins to move — it only reaches it once the geometry has swept
    /// through where the ball was — and the ball's weight never reaches the
    /// arm, so a held flipper does not sag under it.
    ///
    /// The solve is the original's: find the contact force that stops the two
    /// accelerating into each other, apply it to both, then do the same for
    /// friction along the surface.
    pub fn contact(&mut self, ball: &mut Ball, coll: &CollisionEvent, dtime: f32, gravity: Vec3) {
        let normal = coll.hit_normal;

        // Magic, and the original says so: without it balls push each other
        // through a flipper that is just sitting there.
        if coll.hit_distance < -C_EMBEDDED {
            ball.vel += normal * 0.1;
        }

        let r_b = -ball.radius * normal;
        let hit_pos = ball.pos + r_b;
        // In the ball's own z plane, so the contact is planar.
        let c_f = Vec3::new(self.center.x, self.center.y, ball.pos.z);
        let r_f = hit_pos - c_f;

        let v_b = ball.surface_velocity(r_b);
        let v_f = self.surface_velocity(r_f);
        let vrel = v_b - v_f;
        let norm_vel = vrel.dot(normal);

        // Something else may already have taken the ball away this step.
        if norm_vel > C_CONTACTVEL {
            return;
        }

        let a_b = ball.surface_acceleration(r_b, gravity);
        let a_f = self.surface_acceleration(r_f);
        let arel = a_b - a_f;

        // The normal turns with the flipper, so its own rate of change is part
        // of the relative acceleration along it.
        let normal_deriv = Vec3::new(0.0, 0.0, self.angle_speed).cross(normal);
        let norm_acc = arel.dot(normal) + 2.0 * normal_deriv.dot(vrel);
        if norm_acc >= 0.0 {
            return; // already separating
        }

        let inv_mass = 1.0 / ball.mass;
        let a_bc = normal * inv_mass;
        let cross = r_f.cross(-normal);
        let a_fc = (cross / self.inertia).cross(r_f);
        let contact_force_acc = normal.dot(a_bc - a_fc);
        if contact_force_acc <= 0.0 {
            return;
        }

        let j = -norm_acc / contact_force_acc;
        ball.vel += normal * ((j * dtime) * inv_mass - coll.hit_org_normal_velocity);
        self.apply_impulse(cross * (j * dtime));

        // Friction along the surface, static or dynamic depending on whether
        // the two are already sliding.
        let max_fric = j * self.material.friction;
        let slip = vrel - normal * norm_vel;
        let slip_speed = slip.length();
        let (slip_dir, numer, denom_f) = if slip_speed < C_PRECISION {
            let slip_acc = arel - normal * arel.dot(normal);
            if slip_acc.length_squared() < 1e-6 {
                return; // neither sliding nor about to
            }
            let dir = slip_acc.normalize();
            let cross_f = r_f.cross(dir);
            (
                dir,
                -dir.dot(arel),
                dir.dot((cross_f / -self.inertia).cross(r_f)),
            )
        } else {
            let dir = slip / slip_speed;
            let cross_f = r_f.cross(dir);
            (
                dir,
                -dir.dot(vrel),
                dir.dot((cross_f / self.inertia).cross(r_f)),
            )
        };

        let cross_b = r_b.cross(slip_dir);
        let denom_b = inv_mass + slip_dir.dot((cross_b / ball.inertia()).cross(r_b));
        let fric = (numer / (denom_b + denom_f)).clamp(-max_fric.abs(), max_fric.abs());

        ball.apply_surface_impulse(cross_b * (dtime * fric), slip_dir * (dtime * fric));
        self.apply_impulse(r_f.cross(slip_dir) * -(dtime * fric));
    }

    /// Applies an angular impulse (`hitflipper.cpp:424`).
    pub fn apply_impulse(&mut self, rot_i: Vec3) {
        self.angular_momentum += rot_i.z;
        self.angle_speed = self.angular_momentum / self.inertia;
    }

    /// One step of the coil (`FlipperMoverObject::UpdateVelocities`,
    /// `hitflipper.cpp:357`).
    pub fn update_velocities(&mut self) {
        // Without the button pressed, the spring brings it back with a force
        // far smaller than the shot's.
        let mut desired = self.strength;
        if !self.solenoid {
            desired *= -self.return_ratio;
        }

        // Near the stop the torque is damped, so it does not land hard.
        let dist_to_stop = (self.angle_cur - self.angle_end).abs();
        if dist_to_stop < self.torque_damping_angle {
            let t = dist_to_stop / self.torque_damping_angle;
            let lerp = (t * t) * (t * t);
            desired *= lerp + self.torque_damping * (1.0 - lerp);
        }

        if !self.positive_direction() {
            desired = -desired;
        }

        // The coil does not reach full force all at once: it ramps up.
        let ramp = if self.ramp_up <= 0.0 {
            1e6
        } else {
            (self.strength / self.ramp_up).min(1e6)
        };
        self.cur_torque = if desired >= self.cur_torque {
            (self.cur_torque + ramp * PHYS_FACTOR).min(desired)
        } else {
            (self.cur_torque - ramp * PHYS_FACTOR).max(desired)
        };

        let mut torque = self.cur_torque;
        self.in_contact = false;

        // Is it resting against a stop?
        if self.angle_speed.abs() <= 1e-2 {
            let (min, max) = self.range();
            if self.angle_cur >= max - 1e-2 && torque > 0.0 {
                self.angle_cur = max;
                self.in_contact = true;
                self.contact_torque = torque;
                self.angular_momentum = 0.0;
                torque = 0.0;
            } else if self.angle_cur <= min + 1e-2 && torque < 0.0 {
                self.angle_cur = min;
                self.in_contact = true;
                self.contact_torque = torque;
                self.angular_momentum = 0.0;
                torque = 0.0;
            }
        }

        self.angular_acceleration = torque / self.inertia;
        self.angular_momentum += PHYS_FACTOR * torque;
        self.angle_speed = self.angular_momentum / self.inertia;
    }

    /// Moves the flipper (`FlipperMoverObject::UpdateDisplacements`).
    pub fn update_displacements(&mut self, dtime: f32) {
        self.angle_cur += self.angle_speed * dtime;
        let (min, max) = self.range();
        self.angle_cur = self.angle_cur.clamp(min, max);

        // On reaching the stop, the flipper brakes dead.
        if self.angle_speed.abs() < 0.0005 {
            return;
        }
        if (self.angle_cur == max && self.angle_speed > 0.0)
            || (self.angle_cur == min && self.angle_speed < 0.0)
        {
            self.angular_momentum = 0.0;
            self.angle_speed = 0.0;
        }
    }

    fn range(&self) -> (f32, f32) {
        (
            self.angle_start.min(self.angle_end),
            self.angle_start.max(self.angle_end),
        )
    }

    fn positive_direction(&self) -> bool {
        self.angle_end > self.angle_start
    }

    /// When and against what the ball hits.
    ///
    /// A flipper is four things: the two circles —the base, fixed, and the
    /// tip, which rotates— and the two flat faces that join them. All of them
    /// are tested and whichever hits first wins.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if ball.locked {
            return None;
        }

        let mut best: Option<CollisionEvent> = None;
        for c in [
            self.test_circle(ball, dtime, self.center, self.base_radius),
            self.test_circle(ball, dtime, self.tip_center(), self.end_radius),
            self.test_face(ball, dtime, true),
            self.test_face(ball, dtime, false),
        ] {
            if let Some(hit) = c
                && best.as_ref().is_none_or(|m| hit.hit_time < m.hit_time)
            {
                best = Some(hit);
            }
        }
        best
    }

    /// Hit against one of the flipper's two flat faces.
    ///
    /// Port of `HitTestFlipperFace` (`hitflipper.cpp:683`).
    ///
    /// # Why iterating is necessary
    ///
    /// Against a still wall, the time of the hit comes out of a division. With
    /// a face that **rotates**, the distance from the ball to the face is not
    /// linear in time: the sine and cosine of the angle show up. There is no
    /// closed formula, so the original looks for the root by **modified false
    /// position**, which converges fast and needs no derivatives.
    fn test_face(&self, ball: &Ball, dtime: f32, left_face: bool) -> Option<CollisionEvent> {
        use crate::constants::{C_INTERATIONS, C_PRECISION};

        let (angle_min, angle_max) = self.range();
        let anglespeed = self.angle_speed;

        // The normal of the face at angle zero. The two faces share the `y`
        // component and differ in the sign of the `x` one.
        let ffnx = if left_face {
            -self.zero_ang_norm.x
        } else {
            self.zero_ang_norm.x
        };
        let ffny = self.zero_ang_norm.y;
        // The point where the face starts, on the circle of the base.
        let vp = Vec2::new(self.base_radius * ffnx, self.base_radius * ffny);

        let mut t = 0.0f32;
        let (mut t0, mut t1) = (0.0f32, dtime);
        let (mut d0, mut d1, mut dp) = (0.0f32, 0.0f32, 0.0f32);
        let mut f = Vec2::ZERO;
        let (mut ballvtx, mut ballvty) = (0.0f32, 0.0f32);
        let mut bffnd = 0.0f32;
        let mut k = 1u32;
        let mut touching = false;

        while k <= C_INTERATIONS {
            let contact_ang = (self.angle_cur + anglespeed * t).clamp(angle_min, angle_max);
            let (rs, rc) = contact_ang.sin_cos();

            // Green's transform: rotate the normal and the starting point to
            // the angle the flipper would have at instant `t`.
            f = Vec2::new(ffnx * rc - ffny * rs, ffny * rc + ffnx * rs);
            let vt = Vec2::new(
                vp.x * rc - vp.y * rs + self.center.x,
                vp.y * rc + vp.x * rs + self.center.y,
            );

            ballvtx = ball.pos.x + ball.vel.x * t - vt.x;
            ballvty = ball.pos.y + ball.vel.y * t - vt.y;
            bffnd = ballvtx * f.x + ballvty * f.y - ball.radius;

            if bffnd.abs() <= C_PRECISION {
                break;
            }

            if k == 1 {
                if bffnd < -(ball.radius + self.end_radius) {
                    return None; // on the wrong side, or too far inside
                }
                if bffnd <= PHYS_TOUCH {
                    touching = true;
                    break;
                }
                t0 = 0.0;
                d0 = bffnd;
                t1 = dtime;
                d1 = bffnd;
                dp = bffnd;
                t = dtime;
                k += 1;
                continue;
            }

            // Modified false position: if the root falls on the same side
            // twice in a row, the endpoint that did not move is halved. It is
            // what avoids the stalling of the classic method.
            if bffnd * d0 <= 0.0 {
                t1 = t;
                d1 = bffnd;
                if dp * bffnd > 0.0 {
                    d0 *= 0.5;
                }
            } else {
                t0 = t;
                d0 = bffnd;
                if dp * bffnd > 0.0 {
                    d1 *= 0.5;
                }
            }

            if (d1 - d0).abs() < 1e-12 {
                return None;
            }
            t = t0 - d0 * (t1 - t0) / (d1 - d0);
            dp = bffnd;
            k += 1;
        }

        // The original accepts a "close" solution if it did not fully
        // converge, as long as the error is less than a quarter of the ball's
        // radius.
        if !t.is_finite()
            || t < 0.0
            || t > dtime
            || (k > C_INTERATIONS && bffnd.abs() > ball.radius * 0.25)
        {
            return None;
        }
        let _ = touching;

        // Does the contact fall within the length of the face?
        let tangent = if left_face {
            Vec2::new(-f.y, f.x)
        } else {
            Vec2::new(f.y, -f.x)
        };
        let bfftd = ballvtx * tangent.x + ballvty * tangent.y;
        let length = self.flipper_radius * self.zero_ang_norm.x;
        if bfftd < 0.0 || bfftd > length {
            return None;
        }

        let hitz = ball.pos.z + ball.vel.z * t;
        if hitz + ball.radius * 0.5 < self.z_low || hitz - ball.radius * 0.5 > self.z_high {
            return None;
        }

        let bnv = ball.vel.x * f.x + ball.vel.y * f.y;
        let is_contact = bnv.abs() <= C_CONTACTVEL && bffnd.abs() <= PHYS_TOUCH;

        Some(CollisionEvent {
            hit_time: t,
            hit_normal: Vec3::new(f.x, f.y, 0.0),
            hit_distance: bffnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }

    /// Hit against one of the flipper's two circles.
    ///
    /// Unlike a fixed circle, the velocity that matters is the **relative**
    /// one: the flipper is moving.
    fn test_circle(
        &self,
        ball: &Ball,
        dtime: f32,
        center: Vec2,
        radius: f32,
    ) -> Option<CollisionEvent> {
        let dist = Vec2::new(ball.pos.x - center.x, ball.pos.y - center.y);
        let bcdd = dist.length();
        if bcdd <= 1.0e-6 {
            return None;
        }

        let target = radius + ball.radius;
        let bnd = bcdd - target;

        // The velocity of the point of the flipper closest to the ball.
        let r_f = Vec3::new(center.x - self.center.x, center.y - self.center.y, 0.0);
        let v_f = self.surface_velocity(r_f);
        let v_rel = Vec2::new(ball.vel.x - v_f.x, ball.vel.y - v_f.y);

        let normal = dist / bcdd;
        let bnv = v_rel.dot(normal);
        if bnv > C_LOWNORMVEL && bnd > PHYS_TOUCH {
            return None; // moving away and not touching
        }

        let mut is_contact = false;
        let hittime = if bnd < PHYS_TOUCH {
            if bnd < -ball.radius {
                return None;
            }
            if bnv.abs() <= C_CONTACTVEL {
                is_contact = true;
                0.0
            } else {
                (-bnd / bnv).max(0.0)
            }
        } else {
            let a = v_rel.length_squared();
            if a < 1.0e-8 {
                return None;
            }
            let b = dist.dot(v_rel);
            let (t1, t2) =
                crate::shapes::solve_quadratic(a, 2.0 * b, bcdd * bcdd - target * target)?;
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

        Some(CollisionEvent {
            hit_time: hittime,
            hit_normal: Vec3::new(normal.x, normal.y, 0.0),
            hit_distance: bnd,
            is_contact,
            hit_flag: false,
            hit_org_normal_velocity: if is_contact { bnv } else { 0.0 },
            hit_vel: Vec2::ZERO,
        })
    }

    /// The hit: exchanges momentum between the ball and the flipper
    /// (`HitFlipper::Collide`, `hitflipper.cpp:849`).
    pub fn collide(&mut self, ball: &mut Ball, coll: &CollisionEvent) {
        let normal = coll.hit_normal;
        let r_b = normal * -ball.radius;
        let hit_pos = ball.pos + r_b;

        // The hit is resolved in the plane the ball is in.
        let c_f = Vec3::new(self.center.x, self.center.y, ball.pos.z);
        let r_f = hit_pos - c_f;

        let v_b = ball.surface_velocity(r_b);
        let v_f = self.surface_velocity(r_f);
        let v_rel = v_b - v_f;
        let mut bnv = normal.dot(v_rel);

        if bnv >= -C_LOWNORMVEL {
            if bnv > C_LOWNORMVEL {
                return;
            }
            if coll.hit_distance < -C_EMBEDDED {
                bnv = -C_EMBEDSHOT;
            } else {
                return;
            }
        }

        // Get the ball out from inside the flipper.
        let mut hdist = -C_DISP_GAIN * coll.hit_distance;
        if hdist > 1.0e-4 {
            if hdist > C_DISP_LIMIT {
                hdist = C_DISP_LIMIT;
            }
            ball.pos += normal * hdist;
        }

        let mut ang_resp = r_f.cross(normal);

        // **The detail that makes a shot feel alive.** If the flipper is
        // resting against its stop and the impulse pushes it into the stop, no
        // energy is transferred to it: it has nowhere to move. Without this,
        // hitting a flipper at rest gives back a dead bounce.
        let ang_imp = -ang_resp.z;
        let mut flipper_scale = 1.0;
        if self.in_contact && self.contact_torque * ang_imp >= 0.0 {
            ang_resp = Vec3::ZERO;
            flipper_scale = 0.5;
        }

        let epsilon = elasticity_with_falloff(
            self.material.elasticity,
            self.material.elasticity_falloff,
            bnv,
        );

        let inv_mass = 1.0 / ball.mass;
        let denom = inv_mass + normal.dot((ang_resp / self.inertia).cross(r_f));
        let mut impulse = -(1.0 + epsilon) * bnv / denom;
        let mut flipper_imp = normal * -(impulse * flipper_scale);
        let mut rot_i = r_f.cross(flipper_imp);

        if self.in_contact && rot_i.z * self.contact_torque < 0.0 {
            // The ball is pushing against the solenoid. If the flipper can
            // recover quickly, it is better to treat it as if it were fixed:
            // that way the bounce comes out complete instead of damped.
            let recoil_time = -rot_i.z / self.contact_torque;
            let bnv_after = bnv + impulse * inv_mass;
            if recoil_time <= 0.5 || bnv_after > 0.0 {
                impulse = -(1.0 + epsilon) * bnv * ball.mass;
                flipper_imp = Vec3::ZERO;
                rot_i = Vec3::ZERO;
            }
        }
        let _ = flipper_imp;

        ball.vel += normal * (impulse * inv_mass);
        self.apply_impulse(rot_i);

        // Friction, which is what gives the ball its spin.
        let mut tangent = v_rel - normal * v_rel.dot(normal);
        let tangent_sp_sq = tangent.length_squared();
        if tangent_sp_sq > 1e-6 {
            tangent /= tangent_sp_sq.sqrt();
            let vt = v_rel.dot(tangent);

            let cross_b = r_b.cross(tangent);
            let mut kt = inv_mass + tangent.dot((cross_b / ball.inertia()).cross(r_b));
            let cross_f = r_f.cross(tangent);
            kt += tangent.dot((cross_f / self.inertia).cross(r_f));

            let max_fric = (self.material.friction * impulse).abs();
            let jt = (-vt / kt).clamp(-max_fric, max_fric);

            if jt.is_finite() {
                ball.apply_surface_impulse(cross_b * jt, tangent * jt);
                self.apply_impulse(cross_f * -jt);
            }
        }
    }
}
