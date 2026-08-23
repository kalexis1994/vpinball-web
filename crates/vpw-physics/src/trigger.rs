//! The triggers: the zones that report when the ball goes through them.
//!
//! Port of `TriggerLineSeg`, `TriggerHitCircle` (`collideex.cpp`) and of
//! `Trigger::PhysicSetup` (`trigger.cpp:...`).
//!
//! A trigger does not touch the ball. All it does is report that it came in
//! and that it went out, and almost all of a table's logic hangs off that:
//! counting laps, lighting targets, starting a multiball.
//!
//! # Why this does not use the collision engine
//!
//! The original detects the entry as **surface crossings**: it builds the zone
//! out of one segment per side plus a lid, tests them with the same machinery
//! as a wall —in non-rigid mode— and keeps, for each ball, the list of volumes
//! it is inside of (`m_vpVolObjs`). Each crossing flips that list.
//!
//! Here the question is asked the other way around: on every step we look at
//! whether the center of the ball falls inside the zone, and compare it with
//! what was true on the previous step. Changed from outside to inside, there
//! is an entry; the other way around, an exit.
//!
//! The underlying difference is that the state stops being stored: it is
//! **derived** from the position. And that matters because the original's
//! stored state gets out of sync, and there are three different patches in the
//! original code that exist only to fix it up when that happens:
//!
//! - In `LineSeg::HitTestBasic` (`collide.cpp:110`), a branch that fires at
//!   time zero when the ball is less than half a radius from the line and its
//!   state does not match where it is sitting.
//! - In `HitTestBasicRadius` (`collide.cpp:280`), the case of "the ball
//!   appeared in the center of the trigger, let's assume it was created there"
//!   — for a trigger drawn on top of a kicker.
//! - In that same place, a `RemoveFromVectorSingle` for the ball that came to
//!   rest on a kicker and hits it while moving away.
//!
//! None of the three is needed here: the answer is recomputed from scratch on
//! every step, so there is nothing that can end up wrong.
//!
//! What is lost is the exact instant of the crossing —the event is pinned to
//! the one millisecond step— and the possibility of detecting a ball that
//! enters and leaves within the same step. The second one is measured in the
//! tests and does not come up: the smallest trigger on a table measures tens
//! of units and the fastest ball advances a few per step.

use crate::Aabb;
use crate::ball::Ball;
use vpw_math::{Vec2, Vec3};

/// The shape of the zone, seen from above.
#[derive(Debug, Clone)]
pub enum Zone {
    /// The star and button triggers, which the `.vpx` gives by radius.
    Circle { center: Vec2, radius: f32 },
    /// The wire ones, which come as an outline of drag points.
    Polygon { vertices: Vec<Vec2> },
}

impl Zone {
    /// Whether a point of the plane falls inside.
    pub fn contains(&self, p: Vec2) -> bool {
        match self {
            Zone::Circle { center, radius } => (p - *center).length_squared() <= radius * radius,
            Zone::Polygon { vertices } => point_in_polygon(vertices, p),
        }
    }

    fn flat_box(&self) -> (Vec2, Vec2) {
        match self {
            Zone::Circle { center, radius } => (
                *center - Vec2::splat(*radius),
                *center + Vec2::splat(*radius),
            ),
            Zone::Polygon { vertices } => {
                let mut min = Vec2::splat(f32::MAX);
                let mut max = Vec2::splat(f32::MIN);
                for v in vertices {
                    min = min.min(*v);
                    max = max.max(*v);
                }
                (min, max)
            }
        }
    }
}

/// What happens to a ball with respect to a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crossing {
    /// It has just come in.
    Entered,
    /// It has just gone out.
    Exited,
}

/// A zone that reports when the ball comes in and when it goes out.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub area: Zone,
    pub z_low: f32,
    pub z_high: f32,
    pub enabled: bool,
    /// How far it sinks when the ball goes through. It depends on the shape
    /// (`Trigger::UpdateAnimation`, `trigger.cpp:489`).
    pub anim_limit: f32,
    /// Units per millisecond.
    pub anim_speed: f32,
    /// How far down it is right now. Zero is at rest, negative is sunk.
    pub anim_offset: f32,
    /// Whether it is moving, and in which direction.
    animating: bool,
    sinking: bool,

    /// Which balls are inside right now, one per bit.
    ///
    /// A `u64` is more than enough: the table with the most simultaneous balls
    /// in existence does not reach a dozen. Balls of index 64 or higher stay
    /// outside the trigger, which is preferable to allocating memory in the
    /// hot loop.
    inside: u64,
}

impl Trigger {
    pub fn new(area: Zone, z_low: f32, z_high: f32) -> Self {
        Self {
            area,
            z_low,
            z_high,
            enabled: true,
            anim_limit: 32.0,
            anim_speed: 0.0,
            anim_offset: 0.0,
            animating: false,
            sinking: false,
            inside: 0,
        }
    }

    /// Gives it the sinking animation.
    pub fn with_animation(mut self, limit: f32, speed: f32) -> Self {
        self.anim_limit = limit;
        self.anim_speed = speed;
        self
    }

    /// The box that contains it, to discard it quickly.
    pub fn bbox(&self) -> Aabb {
        let (min, max) = self.area.flat_box();
        Aabb {
            min: Vec3::new(min.x, min.y, self.z_low),
            max: Vec3::new(max.x, max.y, self.z_high),
        }
    }

    /// Whether the ball is inside the trigger's volume right now.
    ///
    /// We look at the **center** of the ball, not its edge. It is the same
    /// thing the original does, testing the sides of the trigger with
    /// `lateral = false` (`collideex.cpp:TriggerLineSeg::HitTest`): a trigger
    /// fires when the ball passes over it, not when it grazes it.
    pub fn contains(&self, ball: &Ball) -> bool {
        self.enabled
            && ball.pos.z >= self.z_low
            && ball.pos.z <= self.z_high
            && self.area.contains(Vec2::new(ball.pos.x, ball.pos.y))
    }

    /// Whether this ball believes it is inside.
    pub fn has_ball(&self, ball_index: usize) -> bool {
        ball_index < 64 && self.inside & (1 << ball_index) != 0
    }

    /// Updates the state of a ball and returns the crossing, if there was one.
    ///
    /// The original also pushes the ball forward a touch when it fires
    /// (`pball->m_d.m_pos += STATICTIME * pball->m_d.m_vel`) so the crossing is
    /// not detected again on the next step. Here that is not needed —the state
    /// is recomputed, not accumulated— and along the way we avoid perturbing
    /// the trajectory for a reason that is not physical.
    pub fn update(&mut self, ball_index: usize, ball: &Ball) -> Option<Crossing> {
        if ball_index >= 64 {
            return None;
        }
        let bit = 1u64 << ball_index;
        let was_inside = self.inside & bit != 0;
        let is_inside = self.contains(ball);

        match (was_inside, is_inside) {
            (false, true) => {
                self.inside |= bit;
                Some(Crossing::Entered)
            }
            (true, false) => {
                self.inside &= !bit;
                Some(Crossing::Exited)
            }
            _ => None,
        }
    }

    /// Starts the sinking animation or the returning one
    /// (`TriggerAnimationHit` / `TriggerAnimationUnhit`, `trigger.cpp:396`).
    pub fn animate_crossing(&mut self, crossing: Crossing) {
        self.animating = true;
        match crossing {
            Crossing::Entered => {
                // The original also **resets** the offset to zero before
                // starting to go down. Here we carry on from wherever it is,
                // which is the same thing except when the animation was left
                // halfway: there the original makes a visible jump upwards
                // before sinking.
                self.sinking = true;
            }
            Crossing::Exited => self.sinking = false,
        }
    }

    /// Advances the animation. `dt_ms` in table milliseconds.
    ///
    /// # A sign that is missing in the original
    ///
    /// `Trigger::UpdateAnimation` (`trigger.cpp:511`), on receiving the exit,
    /// does `m_animHeightOffset = animLimit` —**positive**— and then goes up.
    /// The first check is `if (offset >= 0) { offset = 0; finish; }`, so it
    /// cuts off on the very same frame: the trigger snaps to its rest position
    /// instead of animating back.
    ///
    /// The surrounding code makes it clear that the intent was the other one:
    /// it turns on `m_doAnimation`, computes a step per unit of time and has
    /// all the machinery for coming up gradually, which with that value is
    /// never used. It is a missing minus sign.
    ///
    /// Here it goes up from wherever it is to zero, which is what that code
    /// meant to do. It is purely visual and changes nothing about the game.
    pub fn animate(&mut self, dt_ms: f32) {
        if !self.animating {
            return;
        }
        let step = dt_ms * self.anim_speed;
        if self.sinking {
            self.anim_offset -= step;
            if self.anim_offset <= -self.anim_limit {
                self.anim_offset = -self.anim_limit;
                self.animating = false;
            }
        } else {
            self.anim_offset += step;
            if self.anim_offset >= 0.0 {
                self.anim_offset = 0.0;
                self.animating = false;
            }
        }
    }

    /// Whether the animation is in motion.
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// Forgets every ball without firing anything.
    pub fn release_all(&mut self) {
        self.inside = 0;
    }

    /// Takes one ball out of the record, and moves the ones after it down.
    ///
    /// A ball that leaves the table is removed from the engine's list, and
    /// every ball after it changes index. This bitset is keyed by index, so it
    /// has to move with them — the same renumbering a kicker's captured ball
    /// gets.
    ///
    /// Clearing the one bit and leaving the rest where they are is the natural
    /// thing to write and is wrong by exactly one from the drained ball
    /// onwards: the ball that inherits the freed index reads as outside and
    /// fires an entry it never made, and its neighbours answer for each other.
    /// On one ball in play there is nothing above the removed index and it
    /// never shows; in a multiball, every drain scrambles every trigger on the
    /// table.
    ///
    /// The original cannot have this: its "which volumes am I inside" list
    /// lives on the ball (`m_vpVolObjs`, `hitball.h:31`) and dies with it.
    /// Holding the same fact the other way round is what makes the bookkeeping
    /// necessary at all.
    pub fn renumber_after_removing(&mut self, ball_index: usize) {
        if ball_index >= 64 {
            return;
        }
        let below = self.inside & ((1u64 << ball_index) - 1);
        // Nothing can sit above the last slot, and shifting by 64 is not a
        // shift.
        let above = if ball_index == 63 {
            0
        } else {
            self.inside >> (ball_index + 1)
        };
        self.inside = below | (above << ball_index);
    }
}

/// Whether a point falls inside a polygon, by crossing count.
///
/// A ray is cast off to one side and the sides it crosses are counted: odd is
/// inside. It works with concave outlines and with any winding direction,
/// which is exactly what is needed here —wire triggers are drawn by hand and
/// there is no guarantee whatsoever that they are convex or how they are
/// wound.
fn point_in_polygon(v: &[Vec2], p: Vec2) -> bool {
    if v.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = v.len() - 1;
    for i in 0..v.len() {
        // Does the side cross the height of the point?
        if (v[i].y > p.y) != (v[j].y > p.y) {
            let t = (p.y - v[i].y) / (v[j].y - v[i].y);
            if p.x < v[i].x + t * (v[j].x - v[i].x) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
