//! The active pieces of the table: bumpers, gates, spinners and kickers.
//!
//! They all share one idea: stopping the ball is not enough, they have to
//! **react**. The bumper kicks it, the gate opens and lets it through, the
//! spinner spins, the kicker captures it.
//!
//! Port of `physics/collideex.cpp` and of `parts/kicker.cpp`.

use crate::ball::Ball;
use crate::collision::{CollisionEvent, Material};
use crate::constants::{PHYS_FACTOR, PHYS_SKIN};
use crate::shapes::{HitCircle, LineSeg};
use vpw_math::{Vec2, Vec3};

/// A bumper: a circle that kicks.
///
/// Port of `BumperHitCircle::Collide` (`collideex.cpp:24`).
#[derive(Debug, Clone)]
pub struct Bumper {
    pub circle: HitCircle,
    /// How much velocity it adds to the ball.
    pub force: f32,
    /// How much velocity it has to be hit with in order to fire.
    pub threshold: f32,
    pub enabled: bool,
    /// Whether it has just fired, so the animation knows about it.
    pub hit: bool,

    /// How far the ring dropped from its place, in VP units. Always negative
    /// or zero.
    pub ring_offset: f32,
    /// How far the ring moves per table millisecond.
    pub ring_speed: f32,
    /// How far down it goes.
    pub ring_drop: f32,
    /// Whether it is going down; when it reaches the bottom, it comes up.
    ring_dropping: bool,
    ring_animating: bool,
}

impl Bumper {
    pub fn new(circle: HitCircle, force: f32, threshold: f32) -> Self {
        Self {
            circle,
            force,
            threshold,
            enabled: true,
            hit: false,
            ring_offset: 0.0,
            ring_speed: 0.5,
            ring_drop: 0.0,
            ring_dropping: false,
            ring_animating: false,
        }
    }

    /// Moves the ring. `dt_ms` is in table milliseconds, not video ones.
    ///
    /// The ring drops all at once when the bumper fires and comes back on its
    /// own. It is the only piece that animates **by itself**: the others
    /// follow an angle or a position the physics already computes, and this
    /// one has neither — the bumper is a fixed circle, and the ring that goes
    /// up and down is pure decoration.
    ///
    /// Like the trigger, the animation is tied to the table's clock and not to
    /// the video frame: that way the ring takes the same time on a machine
    /// running at 30 frames per second as on one running at 144.
    pub fn animate(&mut self, dt_ms: f32) {
        if self.ring_drop <= 0.0 {
            return;
        }
        if self.hit {
            self.hit = false;
            self.ring_animating = true;
            self.ring_dropping = true;
        }
        if !self.ring_animating {
            return;
        }

        let step = if self.ring_dropping {
            -self.ring_speed
        } else {
            self.ring_speed
        };
        self.ring_offset += step * dt_ms;

        if self.ring_dropping {
            if self.ring_offset <= -self.ring_drop {
                self.ring_offset = -self.ring_drop;
                self.ring_dropping = false;
            }
        } else if self.ring_offset >= 0.0 {
            self.ring_offset = 0.0;
            self.ring_animating = false;
        }
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        self.circle.hit_test(ball, dtime)
    }

    /// Resolves the hit. Returns `true` if the bumper fired.
    ///
    /// The order matters: first the incoming velocity is computed, then the
    /// bounce happens, and **only after that** the kick is added. The other
    /// way around, the threshold would be measured against a velocity that has
    /// already changed.
    pub fn collide(&mut self, ball: &mut Ball, coll: &CollisionEvent, scatter: f32) -> bool {
        let dot = coll.hit_normal.dot(ball.vel);
        ball.collide_3d_wall(coll.hit_normal, &self.circle.material, coll, scatter);

        let fires = dot <= -self.threshold;
        if fires {
            ball.vel += coll.hit_normal * self.force;
            self.hit = true;
        }
        fires
    }
}

/// A gate: a one-way door.
///
/// The ball pushes it and goes through; from the other side, it bounces. It is
/// what keeps a ball from coming back the way it came.
///
/// The ball is **not** deflected when it crosses: the gate is a pass-through
/// line, and what stops it on the wrong side is a separate rigid `LineSeg` the
/// table adds only when the door is one-way (`gate.cpp:PhysicSetup`).
///
/// Port of `HitGate` and `GateMoverObject` (`collideex.cpp:192` and `:303`).
#[derive(Debug, Clone)]
pub struct Gate {
    /// The axis of the door, as a pass-through line. It is tested on both
    /// faces.
    pub seg: LineSeg,
    /// Height of the door. It divides the velocity, so hitting it low makes it
    /// spin more — like a real door, more leverage down low.
    pub height: f32,

    pub angle: f32,
    pub angle_speed: f32,
    pub angle_min: f32,
    pub angle_max: f32,
    /// Already raised to `PHYS_FACTOR`: it is applied once per physics step.
    pub damping: f32,
    /// How hard gravity pulls the door back towards rest.
    pub gravity_factor: f32,

    pub two_way: bool,
    /// Whether the script locked it open.
    pub open: bool,
    /// Which way it opened. It defines the sign of its limits.
    pub hit_direction: bool,
    /// Whether it is the script moving it and not a ball.
    pub forced_move: bool,
    pub enabled: bool,
}

/// The data the table builds a gate or a spinner from.
///
/// They go together because they describe the same thing —an axis that
/// rotates— and because passing them loose would be nine positional
/// arguments, all `f32`, impossible to read at the call site.
#[derive(Debug, Clone, Copy)]
pub struct PivotAxis {
    pub center: Vec2,
    pub length: f32,
    pub rotation_deg: f32,
    /// Height of the piece above its base. It divides the velocity of the hit.
    pub height: f32,
    pub z_low: f32,
    pub angle_min: f32,
    pub angle_max: f32,
    /// Not raised to `PHYS_FACTOR`: that is fixed up when the piece is built.
    pub damping: f32,
}

impl Gate {
    /// Builds the door from its axis.
    pub fn new(pivot: PivotAxis, gravity_factor: f32, two_way: bool) -> Self {
        let (min, max) = sort_limits(pivot.angle_min, pivot.angle_max);
        Self {
            seg: pivot_segment(&pivot),
            height: pivot.height,
            angle: min,
            angle_speed: 0.0,
            angle_min: min,
            angle_max: max,
            damping: pivot.damping.powf(PHYS_FACTOR),
            gravity_factor,
            two_way,
            open: false,
            hit_direction: false,
            forced_move: false,
            enabled: true,
        }
    }

    /// Tests both faces (`HitGate::HitTest`, `collideex.cpp:246`).
    ///
    /// `hit_flag` encodes **which face** the ball came in through, and from
    /// that we later work out whether the door gives way or only bounces.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        if let Some(mut c) = self.seg.hit_test_through(ball, dtime) {
            c.hit_flag = false;
            return Some(c);
        }
        if let Some(mut c) = self.seg.flipped().hit_test_through(ball, dtime) {
            c.hit_flag = true;
            return Some(c);
        }
        None
    }

    /// The hit (`collideex.cpp:192`).
    pub fn collide(&mut self, ball: &Ball, coll: &CollisionEvent) {
        let dot = coll.hit_normal.dot(ball.vel);
        let h = self.height * 0.5;

        let mut speed = dot.abs();
        if h.abs() > 1.0 {
            speed /= h;
        }
        self.angle_speed = speed;

        // Hit from behind and it is not a two-way one: it does not go through,
        // it only bounces a little. That `1/50` comes from the original, so it
        // does not look rigid.
        if !coll.hit_flag && !self.two_way {
            self.hit_direction = dot > 0.0;
            self.angle_speed *= 1.0 / 50.0;
            return;
        }

        self.hit_direction = false;
        if coll.hit_flag && self.two_way {
            self.angle_speed = -self.angle_speed;
        }
    }

    /// Moves the door (`GateMoverObject::UpdateDisplacements`).
    ///
    /// The order is the original's and it is not arbitrary: **first** it
    /// clamps against the limits with the angle from the previous round, and
    /// **then** it integrates.
    pub fn update_displacements(&mut self, dtime: f32) {
        if self.two_way {
            if self.angle.abs() > self.angle_max {
                self.angle = self.angle_max.copysign(self.angle);
                self.hit_stop(true);
            }
            if self.angle.abs() < self.angle_min {
                self.angle = self.angle_min.copysign(self.angle);
                self.hit_stop(false);
            }
        } else {
            // On a one-way one, the door opens towards the side it was pushed
            // from.
            let direction = if self.hit_direction { -1.0 } else { 1.0 };
            if direction * self.angle > self.angle_max {
                self.angle = direction * self.angle_max;
                self.hit_stop(true);
            }
            if direction * self.angle < self.angle_min {
                self.angle = direction * self.angle_min;
                self.hit_stop(false);
            }
        }

        if self.angle_speed == 0.0 {
            self.forced_move = false;
        }
        self.angle += self.angle_speed * dtime;
    }

    /// Bounce against a stop. `opening` tells the opening one from the closing
    /// one.
    fn hit_stop(&mut self, opening: bool) {
        if !self.forced_move {
            // The extra `0.8` comes from the original: it brakes a bit faster
            // so the door does not sit there rattling against the stop.
            self.angle_speed = -self.angle_speed * self.damping * 0.8;
        } else if (opening && self.angle_speed > 0.0) || (!opening && self.angle_speed < 0.0) {
            self.angle_speed = 0.0;
        }
    }

    /// Damps the door and lets it fall back to rest
    /// (`GateMoverObject::UpdateVelocities`, `collideex.cpp:303`).
    ///
    /// The `sin(angle)` is what closes it on its own: the center of gravity
    /// pulls downwards, and it pulls harder the further open it is. Without
    /// that term the door stays open forever.
    pub fn update_velocities(&mut self) {
        if self.open {
            return;
        }
        if self.angle.abs() < self.angle_min + 0.01 && self.angle_speed.abs() < 0.01 {
            // It brakes a little early so it does not end up animating forever
            // with a slow ball.
            self.angle = self.angle_min;
            self.angle_speed = 0.0;
        }
        if self.angle_speed != 0.0 && self.angle != self.angle_min {
            self.angle_speed -= self.angle.sin() * self.gravity_factor * (PHYS_FACTOR / 100.0);
            self.angle_speed *= self.damping;
        }
    }
}

/// A spinner: the vane that spins when the ball goes under it.
///
/// Port of `HitSpinner` and `SpinnerMoverObject` (`collideex.cpp:390`).
#[derive(Debug, Clone)]
pub struct Spinner {
    pub seg: LineSeg,
    pub height: f32,

    pub angle: f32,
    pub angle_speed: f32,
    /// If the two are equal, the vane spins free; otherwise it is limited.
    pub angle_min: f32,
    pub angle_max: f32,
    /// Already raised to `PHYS_FACTOR`.
    pub damping: f32,
    pub elasticity: f32,
    pub enabled: bool,
}

impl Spinner {
    pub fn new(pivot: PivotAxis, elasticity: f32) -> Self {
        let (min, max) = sort_limits(pivot.angle_min, pivot.angle_max);
        Self {
            seg: pivot_segment(&pivot),
            height: pivot.height,
            angle: 0.0f32.clamp(min, max),
            angle_speed: 0.0,
            angle_min: min,
            angle_max: max,
            damping: pivot.damping.powf(PHYS_FACTOR),
            elasticity,
            enabled: true,
        }
    }

    /// Tests both faces (`HitSpinner::HitTest`, `collideex.cpp:428`).
    ///
    /// Careful: the spinner sets `hit_flag` **the other way round** from the
    /// gate — the face that is the front one for one is the back one for the
    /// other.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        if let Some(mut c) = self.seg.hit_test_through(ball, dtime) {
            c.hit_flag = true;
            return Some(c);
        }
        if let Some(mut c) = self.seg.flipped().hit_test_through(ball, dtime) {
            c.hit_flag = false;
            return Some(c);
        }
        None
    }

    /// The hit (`collideex.cpp:390`).
    ///
    /// Unlike the gate, the spinner **only counts from the front**: hit from
    /// behind it does not spin. Returns `true` if it spun.
    pub fn collide(&mut self, ball: &Ball, coll: &CollisionEvent) -> bool {
        let dot = coll.hit_normal.dot(ball.vel);
        if dot < 0.0 {
            return false; // from behind it does not count
        }

        let h = self.height * 0.5;
        let mut speed = dot.abs();
        if h.abs() > 1.0 {
            speed /= h;
        }
        speed *= self.damping;

        self.angle_speed = if coll.hit_flag { -speed } else { speed };
        true
    }

    /// Spins the vane (`SpinnerMoverObject::UpdateDisplacements`).
    pub fn update_displacements(&mut self, dtime: f32) {
        use std::f32::consts::TAU;

        self.angle += self.angle_speed * dtime;

        if self.angle_min != self.angle_max {
            // Limited vane: it bounces against its limits.
            if self.angle > self.angle_max {
                self.angle = self.angle_max;
                if self.angle_speed > 0.0 {
                    self.angle_speed *= -0.005 - self.elasticity;
                }
            }
            if self.angle < self.angle_min {
                self.angle = self.angle_min;
                if self.angle_speed < 0.0 {
                    self.angle_speed *= -0.005 - self.elasticity;
                }
            }
        } else {
            // Free vane: it goes all the way around and the angle wraps.
            while self.angle > TAU {
                self.angle -= TAU;
            }
            while self.angle < 0.0 {
                self.angle += TAU;
            }
        }
    }

    /// Brakes the vane and leaves it hanging downwards
    /// (`SpinnerMoverObject::UpdateVelocities`).
    ///
    /// Just as on the gate, the `sin(angle)` is gravity acting on the center
    /// of mass: it is what makes the vane end up still and vertical instead of
    /// stopping in any old position.
    pub fn update_velocities(&mut self) {
        self.angle_speed -= self.angle.sin() * (0.0025 * PHYS_FACTOR);
        self.angle_speed *= self.damping;
    }
}

/// The .vpx can bring the limits the wrong way round; the original sorts them
/// when building.
fn sort_limits(a: f32, b: f32) -> (f32, f32) {
    (a.min(b), a.max(b))
}

/// The axis of a gate or of a spinner, as a pass-through line.
///
/// It overshoots by `PHYS_SKIN` on each side so the ball does not slip through
/// the ends, and it is finite in z: one ball high, because what matters is
/// whether the ball goes **underneath** the vane and not whether it flies over
/// it.
fn pivot_segment(d: &PivotAxis) -> LineSeg {
    let (sn, cs) = d.rotation_deg.to_radians().sin_cos();
    let half = d.length * 0.5 + PHYS_SKIN;
    let tip = Vec2::new(cs * half, sn * half);
    LineSeg::new(
        d.center - tip,
        d.center + tip,
        d.z_low,
        d.z_low + 2.0 * PHYS_SKIN,
        Material::default(),
    )
}

/// A kicker: the hole that captures the ball.
///
/// Port of `KickerHitCircle::DoCollide` (`kicker.cpp:1111`).
///
/// With no script engine there is nobody to release it, so capturing a ball
/// leaves it there. That is the right thing: in the real game the table
/// releases it, not the physics.
#[derive(Debug, Clone)]
pub struct Kicker {
    pub circle: HitCircle,
    /// What relative height has to be reached for it to grab.
    pub hit_accuracy: f32,
    /// Whether it behaves the way kickers did before the height test existed.
    ///
    /// A legacy kicker grabs whatever touches it, however high the ball is
    /// riding (`kicker.cpp:1128`). Most tables' saucers are legacy — F-14's
    /// launch-lane saucer is — so leaving this out means a ball rolls into the
    /// hole, comes to a stop on top of it and stays there, because nothing
    /// catches it and nothing tells the script it arrived.
    pub legacy_mode: bool,
    pub enabled: bool,
    /// Which ball it has inside, if it has one.
    pub captured: Option<usize>,
}

impl Kicker {
    pub fn new(circle: HitCircle, hit_accuracy: f32, legacy_mode: bool) -> Self {
        Self {
            circle,
            hit_accuracy,
            legacy_mode,
            enabled: true,
            captured: None,
        }
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled || self.captured.is_some() {
            return None;
        }
        // A kicker is not a wall. See `HitCircle::hit_test_as_volume`.
        self.circle.hit_test_as_volume(ball, dtime)
    }

    /// What height the ball has to be at for the kicker to grab it.
    pub fn grab_height(&self, ball: &Ball) -> f32 {
        (self.circle.z_low + ball.radius) * self.hit_accuracy
    }

    /// Tries to capture the ball. Returns `true` if it grabbed it.
    ///
    /// The ball has to be sunk far enough into the hole: if it comes in fast
    /// and high, it runs straight past over the edge. That is what keeps every
    /// ball that grazes a kicker from being captured.
    pub fn try_capture(&mut self, ball: &mut Ball, index: usize) -> bool {
        if self.captured.is_some() || !self.enabled {
            return false;
        }
        // `kicker.cpp:1128`. A legacy kicker does not ask how high the ball is.
        if !self.legacy_mode && ball.pos.z >= self.grab_height(ball) {
            return false; // it passes over the top
        }

        // It comes to rest in the center of the hole.
        ball.pos = Vec3::new(
            self.circle.center.x,
            self.circle.center.y,
            self.circle.z_low + ball.radius,
        );
        ball.vel = Vec3::ZERO;
        ball.angular_momentum = Vec3::ZERO;
        ball.locked = true;
        self.captured = Some(index);
        true
    }

    /// Releases the ball with a given velocity.
    ///
    /// In the game this is fired by the script; here it is exposed so the
    /// engine can do it when one exists.
    pub fn release(&mut self, ball: &mut Ball, vel: Vec3) {
        if self.captured.is_none() {
            return;
        }
        ball.locked = false;
        ball.vel = vel;
        // A ball comes out of a saucer with the spin it went in with, and that
        // spin is nobody's business now: the original clears it on the way out
        // (`m_angularmomentum.SetZero()`, `kicker.cpp:738`). Leaving it means
        // the ball lands somewhere down the table still turning the way it was
        // when it arrived, and friction steers it off the line the kick aimed
        // it along.
        ball.angular_momentum = Vec3::ZERO;
        self.captured = None;
    }
}
