//! The plunger.
//!
//! Port of `PlungerMoverObject` and `HitPlunger` (`physics/hitplunger.cpp`).
//!
//! It is the only piece the player **charges up**: you pull it back, force
//! builds up, you let go and it shoots out. Everything else on the table
//! reacts to the ball; this one reacts to a button.
//!
//! # How it moves
//!
//! It has a position `pos` in table units, between `frame_end` (all the way
//! forward) and `frame_start` (all the way back), and a rest position in
//! between. While pulling, a constant force takes it backwards. On release it
//! enters **fire mode**: the exit velocity is computed in one go from how far
//! it was pulled, and it travels at that velocity until it bounces off the
//! spring of the barrel.
//!
//! # What is not ported
//!
//! Three whole paths of the original are left out because they are about
//! cabinets and not keyboards:
//!
//! - The **mechanical plunger**: a hardware sensor that reports the real
//!   position of a physical plunger. The original models the on-screen one as
//!   if it were joined to the real one by a spring, so it does not teleport.
//! - The **auto-fire**: detecting that the player pulled and released the
//!   physical plunger on a table that has no plunger, and turning that into a
//!   launch button.
//! - The **retract**: `PullBackandRetract`, which makes the plunger go back
//!   and forth on its own. The script uses it, not the player.
//!
//! `auto_plunger` is there: it is a property of the table —the ones that are
//! launched with a button instead of a spring— and it always fires from all
//! the way back.

use crate::Aabb;
use crate::ball::Ball;
use crate::collision::{CollisionEvent, Material};
use crate::constants::{C_DISP_GAIN, C_DISP_LIMIT, C_EMBEDDED, C_EMBEDSHOT, C_LOWNORMVEL};
use crate::shapes::{HitCircle, LineSeg};
use vpw_math::{Vec2, Vec3};

/// Height of the plunger, in table units (`hitplunger.cpp:9`).
pub const PLUNGER_HEIGHT: f32 = 50.0;

/// The mass of the plunger (`hitplunger.cpp:11`).
///
/// It is an engine constant and not a property of the table: every plunger
/// weighs the same, and what tables tune is the force.
pub const PLUNGER_MASS: f32 = 30.0;

/// How many steps fire mode lasts (`PlungerMoverObject::Fire`).
///
/// It is a safety cutoff: past that time we leave the mode even if the plunger
/// has not finished settling, so as not to get stuck there.
const FIRE_STEPS: u32 = 200;

/// The plunger.
#[derive(Debug, Clone)]
pub struct Plunger {
    // The fixed box, in the plane.
    x: f32,
    x2: f32,
    y: f32,
    z_low: f32,

    /// All the way forward, all the way back, and the distance between the
    /// two.
    pub frame_end: f32,
    pub frame_start: f32,
    pub frame_len: f32,
    /// Where it sits at rest, between 0 and 1.
    pub rest_pos: f32,

    /// Where it is now, in table units.
    pub pos: f32,
    pub speed: f32,

    /// Pull force while the button is held down.
    pull_force: f32,
    /// Fixed velocity during fire mode, and how much of it is left.
    fire_speed: f32,
    fire_bounce: f32,
    fire_timer: u32,
    /// What the ball gives back to it when it hits it.
    reverse_impulse: f32,
    /// Temporary limit on advancing, so it does not run over the ball.
    travel_limit: f32,

    /// Properties of the table.
    pub speed_pull: f32,
    pub speed_fire: f32,
    pub scatter_velocity: f32,
    pub momentum_xfer: f32,
    pub auto_plunger: bool,

    material: Material,
}

impl Plunger {
    /// Builds the plunger (`HitPlunger::HitPlunger`, `hitplunger.cpp:14`).
    pub fn new(d: PlungerParams) -> Self {
        let frame_len = d.frame_start - d.frame_end;
        Self {
            x: d.x,
            x2: d.x2,
            y: d.y,
            z_low: d.z_low,
            frame_end: d.frame_end,
            frame_start: d.frame_start,
            frame_len,
            rest_pos: d.rest_pos,
            pos: d.frame_end + d.rest_pos * frame_len,
            speed: 0.0,
            pull_force: 0.0,
            fire_speed: 0.0,
            fire_bounce: 0.0,
            fire_timer: 0,
            reverse_impulse: 0.0,
            travel_limit: d.frame_end,
            speed_pull: d.speed_pull,
            speed_fire: d.speed_fire,
            scatter_velocity: d.scatter_velocity,
            momentum_xfer: d.momentum_xfer,
            auto_plunger: d.auto_plunger,
            material: d.material,
        }
    }

    /// Where it is, between 0 (all the way forward) and 1 (all the way back).
    pub fn relative_position(&self) -> f32 {
        (self.pos - self.frame_end) / self.frame_len
    }

    /// How far it has been drawn back from where it rests, from 0 to 1.
    ///
    /// Not the same number as [`Plunger::relative_position`], and the
    /// difference is the whole reason this exists. A plunger parks part of the
    /// way along its frame — the table says where, and F-14 parks at 0.17 —
    /// so the raw position reads as already drawn back when nobody has touched
    /// it. What a shooter drawn on screen has to show is the *travel*: nothing
    /// at rest, everything at the back of the frame.
    ///
    /// Firing throws it forward past the park position, which comes out as 0
    /// rather than as a negative: there is no such thing as less than not
    /// drawn back.
    pub fn travel(&self) -> f32 {
        let room = 1.0 - self.rest_pos;
        if room <= 1e-6 {
            return 0.0;
        }
        ((self.relative_position() - self.rest_pos) / room).clamp(0.0, 1.0)
    }

    /// Starts pulling backwards (`PlungerMoverObject::PullBack`).
    pub fn pull(&mut self) {
        self.speed = 0.0;
        self.pull_force = self.speed_pull;
    }

    /// Lets go and fires (`PlungerMoverObject::Fire`, `hitplunger.cpp:239`).
    ///
    /// The exit velocity comes **in one go** from how far it had been pulled,
    /// and not from integrating a spring force. It is simpler and gives a
    /// plunger that responds predictably, which is what a player wants.
    pub fn release(&mut self) {
        // A button table always fires at full power.
        let from = if self.auto_plunger {
            1.0
        } else {
            self.relative_position()
        };
        self.fire_from(from);
    }

    fn fire_from(&mut self, start_pos: f32) {
        self.pull_force = 0.0;
        let start_pos = start_pos.max(self.rest_pos);
        self.pos = self.frame_end + start_pos * self.frame_len;

        // Negative because the movement goes forwards, that is, towards
        // decreasing y. The `100/13` comes from the original.
        let dx = start_pos - self.rest_pos;
        self.fire_speed =
            -(self.speed_fire / 100.0) * self.frame_len * (dx * (100.0 / 13.0)) / PLUNGER_MASS;

        // The bounce off the barrel's spring: proportional to how far it was
        // pulled, and past half the travel it bounces as much as it can.
        const MAX_PULL: f32 = 0.5;
        let bounce = if dx < MAX_PULL { dx / MAX_PULL } else { 1.0 };
        self.fire_bounce = -bounce * self.rest_pos;
        self.fire_timer = FIRE_STEPS;
    }

    /// Updates the velocity (`PlungerMoverObject::UpdateVelocities`).
    pub fn update_velocities(&mut self) {
        if self.fire_timer > 0 {
            // In fire mode it travels at a fixed velocity: it was already
            // computed on release.
            self.speed = self.fire_speed;
            self.fire_timer -= 1;
        } else if self.pull_force != 0.0 {
            // The pull force is in units where the time step is worth one, so
            // no `dt` shows up here.
            self.speed += self.pull_force / PLUNGER_MASS;

            if self.pos > self.frame_start {
                self.speed = 0.0;
                self.pos = self.frame_start;
            }
            let min_pos = self.frame_end + self.rest_pos * self.frame_len;
            if self.pos < min_pos {
                self.speed = 0.0;
                self.pos = min_pos;
            }
        }

        // What the ball gave back reaches the rod only on a table with a
        // mechanical plunger. In the original the line that applies it is the
        // last one *inside* `else if (isMech)` (`hitplunger.cpp:470`) and only
        // the clearing is outside, so a keyboard plunger never feels the ball.
        //
        // Applying it here regardless is not a rounding error, because nothing
        // in this mode ever puts the rod back: with no fire timer and no pull
        // force neither branch above touches the speed, so the kick is still
        // there next step and every step after. Measured, a ball coming back
        // down the lane at a rod that is still bouncing from a shot sends it to
        // the fully retracted stop —149 units from rest— where it stays, in
        // full view, for the rest of the ball.
        //
        // The mechanical plunger is not ported: there is no analogue cabinet
        // input in a browser to drive it. So the impulse is collected and
        // dropped, which is what the original does with it in this mode.
        self.reverse_impulse = 0.0;
    }

    /// Moves it (`PlungerMoverObject::UpdateDisplacements`).
    pub fn update_displacements(&mut self, dtime: f32) {
        self.pos += dtime * self.speed;
        self.pos = self.pos.max(self.travel_limit);

        // In fire mode, on crossing the bounce position it turns around with
        // less force. That is the barrel's spring.
        let rel = self.relative_position();
        let bounce = self.rest_pos + self.fire_bounce;
        let crossed = if self.fire_speed < 0.0 {
            rel <= bounce
        } else {
            rel >= bounce
        };
        if self.fire_timer != 0 && dtime != 0.0 && crossed {
            self.pos = self.frame_end + bounce * self.frame_len;
            self.fire_speed = -self.fire_speed * 0.4;
            self.fire_bounce *= -0.4;
        }
        self.pos = self.pos.max(self.travel_limit);

        if dtime != 0.0 {
            if self.pos < self.frame_end {
                self.speed = 0.0;
                self.pos = self.frame_end;
            } else if self.pos > self.frame_start {
                self.speed = 0.0;
                self.pos = self.frame_start;
            }
            self.pos = self.pos.max(self.travel_limit);
        }

        // The limit is good for one step only.
        self.travel_limit = self.frame_end;
    }

    // ------------------------------------------------------------ geometry ---

    /// The base: the bottom of the slot, which does not move.
    fn base(&self) -> LineSeg {
        LineSeg::new(
            Vec2::new(self.x, self.y),
            Vec2::new(self.x2, self.y),
            self.z_low,
            self.z_low + PLUNGER_HEIGHT,
            self.material,
        )
    }

    /// The two sides of the slot. They reach as far as the tip is.
    ///
    /// That extra `0.0001` comes from the original: without it, the two
    /// endpoints of the segment would fall on the same x and the normal would
    /// come out undefined.
    fn sides(&self) -> [LineSeg; 2] {
        [
            LineSeg::new(
                Vec2::new(self.x + 0.0001, self.pos),
                Vec2::new(self.x, self.y),
                self.z_low,
                self.z_low + PLUNGER_HEIGHT,
                self.material,
            ),
            LineSeg::new(
                Vec2::new(self.x2, self.y),
                Vec2::new(self.x2 + 0.0001, self.pos),
                self.z_low,
                self.z_low + PLUNGER_HEIGHT,
                self.material,
            ),
        ]
    }

    /// The tip: the part that moves and that hits the ball.
    fn tip(&self) -> LineSeg {
        LineSeg::new(
            Vec2::new(self.x2, self.pos),
            Vec2::new(self.x, self.pos),
            self.z_low,
            self.z_low + PLUNGER_HEIGHT,
            self.material,
        )
    }

    fn joint(&self, x: f32, y: f32) -> HitCircle {
        HitCircle {
            center: Vec2::new(x, y),
            radius: 0.0,
            z_low: self.z_low,
            z_high: self.z_low + PLUNGER_HEIGHT,
            material: self.material,
        }
    }

    /// The box, with the plunger at **any** position of its travel.
    pub fn bbox(&self) -> Aabb {
        let (y0, y1) = (
            self.frame_end.min(self.frame_start).min(self.y),
            self.frame_end.max(self.frame_start).max(self.y),
        );
        Aabb {
            min: Vec3::new(self.x.min(self.x2), y0, self.z_low),
            max: Vec3::new(
                self.x.max(self.x2) + 0.0001,
                y1,
                self.z_low + PLUNGER_HEIGHT,
            ),
        }
    }

    // ---------------------------------------------------------- collisions ---

    /// When it is going to hit the ball (`HitPlunger::HitTest`).
    ///
    /// The fixed parts —the bottom and the sides— are tested like any other
    /// wall. The tip **moves**, and there the original uses a trick: instead
    /// of moving the segment, it looks at the hit from a frame where the tip
    /// is still, subtracting its velocity from the ball. A change of reference
    /// frame is cheaper and more exact than moving the geometry.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        // How much velocity it passes to the ball. A heavier ball takes away
        // less velocity for the same momentum (p = mv, that is, v = p/m).
        let ball_mass = ball.mass.max(0.05);
        let impulse = self.speed * (self.momentum_xfer / ball_mass);

        // The tip is looked at from a frame where it is still.
        let mut relative_ball = ball.clone();
        relative_ball.vel.y -= self.speed;

        let [left, right] = self.sides();
        let still = [self.base(), left, right];
        let still_joints = [self.joint(self.x, self.y), self.joint(self.x2, self.y)];
        let moving_joints = [self.joint(self.x, self.pos), self.joint(self.x2, self.pos)];

        // The original keeps shortening the time limit with each test. Here
        // they are all tested against the whole time and the earliest one is
        // picked: it comes out the same —the minimum is the minimum— and there
        // are seven segments.
        let candidates = still
            .iter()
            .filter_map(|l| l.hit_test(ball, dtime))
            .chain(still_joints.iter().filter_map(|j| j.hit_test(ball, dtime)))
            .map(|c| (c, 0.0))
            .chain(
                std::iter::once(self.tip())
                    .filter_map(|l| l.hit_test(&relative_ball, dtime))
                    .chain(
                        moving_joints
                            .iter()
                            .filter_map(|j| j.hit_test(&relative_ball, dtime)),
                    )
                    .map(|c| (c, impulse)),
            )
            .filter(|(c, _)| c.hit_time >= 0.0 && c.hit_time <= dtime);

        let (mut c, vel_y) = candidates.min_by(|a, b| a.0.hit_time.total_cmp(&b.0.hit_time))?;
        c.hit_vel = Vec2::new(0.0, vel_y);

        // If they ended up overlapping, it gets just enough of a shove to pull
        // them apart.
        if c.hit_distance <= 0.0 && c.hit_vel.y == impulse && impulse.abs() < c.hit_distance.abs() {
            c.hit_vel.y = -c.hit_distance.abs();
        }
        Some(c)
    }

    /// Freezes it where it is for one step only, because it has just hit the
    /// ball.
    ///
    /// Without this, a fast plunger against a heavy ball catches up with it
    /// and runs it over. And there is something worse: the physics loop
    /// advances to the next hit, so two bodies in continuous contact give a
    /// time of zero forever and time stops running. The limit breaks the
    /// contact just enough for the ball to get clear.
    ///
    /// It is separate from [`Plunger::hit_test`] for a reason of ordering: the
    /// limit has to be the position from **before** the moving pieces advance,
    /// and by the time the hit is resolved the plunger has already moved. The
    /// original writes the field from inside `HitTest` with a `const_cast`;
    /// here the engine calls it at the right moment.
    pub fn mark_travel_limit(&mut self) {
        if self.travel_limit < self.pos {
            self.travel_limit = self.pos;
        }
    }

    /// Resolves the hit (`HitPlunger::Collide`, `hitplunger.cpp:715`).
    pub fn collide(&mut self, ball: &mut Ball, coll: &CollisionEvent, scatter: f32) {
        let n = coll.hit_normal;
        let mut dot = (ball.vel.x - coll.hit_vel.x) * n.x + (ball.vel.y - coll.hit_vel.y) * n.y;

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

        let hdist = -C_DISP_GAIN * coll.hit_distance;
        if hdist > 1.0e-4 {
            ball.pos += n * hdist.min(C_DISP_LIMIT);
        }

        // The `-1.45` comes from the original: it does not come from any
        // material, it is the hardness of the plunger.
        let impulse = dot * -1.45 / (1.0 + 1.0 / PLUNGER_MASS);

        // Hitting something takes away its appetite for bouncing. A plunger
        // fired into thin air bounces quite a bit; one that hits the ball, far
        // less, because the momentum went off with it.
        self.fire_bounce *= 0.6;

        if coll.hit_vel.y != 0.0 {
            // The tip hit the ball, or the other way around. Some of the
            // ball's momentum comes back to the plunger.
            //
            // The original calls the 0.22 a "fudge factor" and admits it is
            // there to make it look right; it justifies it as the spring
            // tension and the friction, which are not modelled.
            const FUDGE: f32 = 0.22;
            self.reverse_impulse = ball.vel.y * impulse * (ball.mass / PLUNGER_MASS) * FUDGE;
        }

        ball.vel += n * impulse;
        ball.vel *= 0.999; // friction, on all three axes

        if self.scatter_velocity > 0.0 && ball.vel.y.abs() > self.scatter_velocity {
            // Same distribution as in the rest of the engine: `s(1-s²)`, which
            // vanishes at the extremes and concentrates the values near zero.
            let s = scatter * (1.0 - scatter * scatter) * 2.59808 * self.scatter_velocity;
            ball.vel.y += s;
        }
    }
}

/// The data the table builds a plunger from.
#[derive(Debug, Clone)]
pub struct PlungerParams {
    pub x: f32,
    pub x2: f32,
    pub y: f32,
    pub z_low: f32,
    /// All the way forward (smaller y) and all the way back (larger y).
    pub frame_end: f32,
    pub frame_start: f32,
    pub rest_pos: f32,
    pub speed_pull: f32,
    pub speed_fire: f32,
    pub scatter_velocity: f32,
    pub momentum_xfer: f32,
    pub auto_plunger: bool,
    pub material: Material,
}
