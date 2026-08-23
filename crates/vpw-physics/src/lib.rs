//! Visual Pinball's physics engine, ported to Rust.
//!
//! Counterpart of `src/physics/` in the C++ repo (~11k LOC). It is the most
//! self-contained piece of the original engine: it does not touch the OS, it
//! does not touch the renderer and it is deterministic, so it is the best
//! starting point for the port and also where most of the performance wins
//! are.
//!
//! Status: only the constants are ported. See `docs/porting-plan.md`.

pub mod ball;
pub mod collision;
pub mod constants;
pub mod engine;
pub mod flipper;
pub mod nudge;
pub mod parts;
pub mod plunger;
pub mod quadtree;
pub mod shapes;
pub mod trigger;

use constants::PHYSICS_STEPTIME_S;

/// Fixed-step accumulator for the physics loop.
///
/// The original engine runs the physics at 1000 Hz, decoupled from the frame
/// rate (`PhysicsEngine::UpdatePhysics`). We reproduce that contract: the
/// renderer asks for `advance(dt)` and consumes however many ticks come out.
#[derive(Debug, Clone)]
pub struct FixedStep {
    accumulator: f64,
    step: f64,
    /// Cap on ticks per frame, so we do not enter a death spiral if the tab
    /// was in the background. Derived from [`FixedStep::max_catch_up_seconds`]
    /// rather than chosen: see there for why it cannot be its own number.
    max_steps_per_frame: u32,
}

/// The longest stretch of real time the loop will make up for.
///
/// Past this the time is gone: a tab that spent a minute in the background is
/// not owed a minute of pinball. Under it, every millisecond is paid.
///
/// It has to be **one** number. It was two — the caller clamped the elapsed
/// time to a quarter of a second and this capped the ticks at a hundred, in
/// different crates, neither knowing about the other — and everything between
/// them was thrown away without anyone deciding to. A browser stall of 250 ms
/// asked for 250 ticks, got 100, and a hundred and fifty milliseconds of the
/// game simply did not happen. That is what a physics rate of 820 Hz on a
/// 120 Hz display was: not the loop failing to keep up, the loop being told to
/// skip.
///
/// A quarter of a second of catch-up is about 27 ms of work at the rate the
/// step actually runs at in a browser — one dropped frame. Cheaper than losing
/// a sixth of a second of a ball.
pub const MAX_CATCH_UP_S: f64 = 0.25;

impl Default for FixedStep {
    fn default() -> Self {
        Self::new(PHYSICS_STEPTIME_S)
    }
}

impl FixedStep {
    pub fn new(step_seconds: f64) -> Self {
        debug_assert!(step_seconds > 0.0);
        Self {
            accumulator: 0.0,
            step: step_seconds,
            max_steps_per_frame: (MAX_CATCH_UP_S / step_seconds).ceil() as u32,
        }
    }

    /// The longest stretch of real time this will make up for, in seconds.
    ///
    /// A caller that clamps the elapsed time it hands in —and it should, or a
    /// tab returning from the background asks for an hour— has to clamp it to
    /// **this**. Clamping to anything larger throws the difference away; to
    /// anything smaller leaves catch-up this was willing to do undone.
    pub fn max_catch_up_seconds(&self) -> f64 {
        self.max_steps_per_frame as f64 * self.step
    }

    /// Duration of one tick, in seconds.
    #[inline]
    pub fn step_seconds(&self) -> f64 {
        self.step
    }

    /// Adds the elapsed time and returns how many ticks have to be simulated.
    pub fn advance(&mut self, delta_seconds: f64) -> u32 {
        self.accumulator += delta_seconds;
        let mut steps = (self.accumulator / self.step) as u32;
        if steps > self.max_steps_per_frame {
            steps = self.max_steps_per_frame;
            // Past the limit the time is gone rather than owed: catching up on
            // a minute in the background would be a minute of frozen picture,
            // and slow motion is the better failure. A caller that clamps what
            // it hands in to [`FixedStep::max_catch_up_seconds`] never gets
            // here at all, which is the point of saying so out loud.
            self.accumulator = 0.0;
        } else {
            self.accumulator -= steps as f64 * self.step;
        }
        steps
    }

    /// Fraction of the next tick already elapsed, to interpolate the render.
    #[inline]
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.step) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_runs_at_1000_hz() {
        use constants::DEFAULT_STEPTIME_S;
        assert!((PHYSICS_STEPTIME_S - 0.001).abs() < f64::EPSILON);
        assert!((DEFAULT_STEPTIME_S / PHYSICS_STEPTIME_S - 10.0).abs() < 1e-9);
    }

    #[test]
    fn accumulates_ticks_without_losing_time() {
        let mut fs = FixedStep::default();
        // One frame at 60 fps is 16.67 physics ticks.
        assert_eq!(fs.advance(1.0 / 60.0), 16);
        assert_eq!(fs.advance(1.0 / 60.0), 17); // the remainder carries over
    }

    #[test]
    fn does_not_enter_a_death_spiral() {
        // Half a minute in the background is not owed half a minute of pinball:
        // it is owed [`MAX_CATCH_UP_S`] and the rest is gone. Derived rather
        // than written down, because the whole point of that constant is that
        // it is the only place the number lives — it used to be written down
        // here and clamped to something else in the caller, and everything
        // between the two was thrown away without anyone deciding to.
        let mut fs = FixedStep::default();
        let most = (MAX_CATCH_UP_S / fs.step_seconds()) as u32;
        assert_eq!(fs.advance(30.0), most);
        assert_eq!(fs.advance(0.0), 0, "and the debt is dropped, not carried");
    }
}

/// Axis-aligned box. This is what the broadphase compares before spending
/// time on a real collision test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: vpw_math::Vec3,
    pub max: vpw_math::Vec3,
}

impl Aabb {
    /// The box that contains nothing, ready to start absorbing points.
    pub fn empty() -> Self {
        Self {
            min: vpw_math::Vec3::splat(f32::MAX),
            max: vpw_math::Vec3::splat(f32::MIN),
        }
    }

    pub fn absorb(&mut self, p: vpw_math::Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Whether two boxes touch. This is what the broadphase discards with.
    pub fn intersects(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    pub fn center(&self) -> vpw_math::Vec3 {
        (self.min + self.max) * 0.5
    }
}

/// The box that contains every one of the given points.
pub fn aabb_of_points(points: &[vpw_math::Vec3]) -> Aabb {
    let mut b = Aabb::empty();
    for p in points {
        b.absorb(*p);
    }
    b
}
