//! The nudge to the cabinet and the punishment for abusing it.
//!
//! Port of `physics/cabinet/`: `CabModelKeyboardNudge` and `CabinetPhysics`
//! for the nudge, `PlumbHandler` for the tilt. It is the model Visual Pinball
//! uses by default since 10.9 (`Settings_properties.inl:218`, mode 2).
//!
//! # How it works
//!
//! Three layers, and none of them is skipped:
//!
//! 1. **The intent.** Pressing the key does not move anything: it queues a 25
//!    ms impulse whose acceleration rises and falls with a raised cosine. It
//!    is what a hand against the wood does, and it is the difference between a
//!    nudge and a yank.
//! 2. **The cabinet.** A 113 kg piece of furniture standing on four legs that
//!    bend: one damped harmonic oscillator per axis, with frequencies measured
//!    on real machines (9.3 Hz sideways, 5.8 Hz front to back).
//! 3. **The plumb.** A ten centimetre pendulum hanging from the cabinet, with
//!    a ring around it. If it touches, that is a warning.
//!
//! What the ball feels is the **acceleration of the cabinet**, subtracted and
//! not added, because it is a fictitious force: the table moves and the ball,
//! out of inertia, stays put ([`crate::ball::Ball::update_velocities`], port
//! of `hitball.cpp:491-495`).
//!
//! # Units
//!
//! This module works in **metres, kilos and seconds**, like the original:
//! these are masses and frequencies measured on real machines and expressing
//! them in VPU would clarify nothing. The only boundary is [`Nudge::step`],
//! which returns the acceleration already converted to VPU/VPT².
//!
//! # Where we depart
//!
//! The original does not decide when the ball is over: the plumb fires the
//! `Tilt` action and **the ROM counts** the warnings. With no bridge to
//! PinMAME there is nobody to count them, so this counts them
//! ([`WARNINGS_BEFORE_TILT`]) until that bridge exists. Also left out are the
//! two old keyboard modes —VP9's push/retract and 10.8's box model— and the
//! whole cabinet sensor path, which is hardware a browser does not have.

use crate::constants::{GRAVITY_MS2, MS2_TO_VPUVPT2};
use vpw_math::{Vec2, Vec3};

/// One physics step in seconds. The original calls it `deltaTime` and has it
/// hand-written in every one of these models, always 1 ms.
const DT: f32 = 0.001;

/// The force VP's script uses as a reference (`KeyboardNudge.cpp`,
/// `coreScriptStrength`). It has no unit: it is the number tables have been
/// passing forever.
const SCRIPT_FORCE: f32 = 2.0;

/// Scale that turns that force into an acceleration: a hard nudge has to give
/// a peak of half a g.
const BASE_SCALE: f32 = 0.5 * GRAVITY_MS2 / SCRIPT_FORCE;

/// How long an impulse lasts, in milliseconds.
const IMPULSE_MS: u32 = 25;

/// Mass of a cabinet, in kilos (`CabinetPhysics.cpp:9`).
const CABINET_MASS: f32 = 113.0;

/// Frequency and damping of each axis, "calibrated on real cabinets, from
/// CFTBL to King Kong" (`CabinetPhysics.cpp:12-13`). Sideways the furniture is
/// stiffer than front to back.
const FREQ_X: f32 = 9.3;
const ZETA_X: f32 = 0.052;
const FREQ_Y: f32 = 5.8;
const ZETA_Y: f32 = 0.055;

/// Fudge factors for the visible offset (`CabinetPhysics.cpp:36-38`).
///
/// The original calls them "magic" and explains why they are needed: the model
/// only accounts for the legs bending, and a cabinet also slides, lifts a
/// little and has play in its bolts.
const OFFSET_FUDGE_X: f32 = 3.5;
const OFFSET_FUDGE_Y: f32 = 2.0;

/// Length of the plumb, in metres (`PlumbHandler.h:31`).
const PLUMB_LENGTH: f32 = 0.10;

/// Angular damping of the plumb: one linear part and another that grows with
/// the velocity (`PlumbHandler.h:47-48`). The second one is what keeps a hard
/// hit from swinging it all the way around.
const PLUMB_DAMPING_LINEAR: f32 = 1.25;
const PLUMB_DAMPING_NONLINEAR: f32 = 0.75;

/// What angle the ring sits at for the original
/// (`Settings_properties.inl:215`). It is a player setting, between 0.15 and 4
/// degrees.
pub const TILT_THRESHOLD_DEG_ORIGINAL: f32 = 1.0;

/// What angle we put it at.
///
/// Measured on this very model, a keyboard nudge takes the plumb to 53% of the
/// ring with the original's threshold, and the strongest one that comes out of
/// the scatter to 74%: it **never** touches it. Ten nudges in a row do not
/// either, at any rhythm. With the original's threshold and keyboard only, the
/// tilt does not exist, and a table where nudging is free plays differently.
///
/// In Visual Pinball that is not a problem because a real cabinet has an
/// accelerometer —which hits far harder than a key— and because the player can
/// lower the threshold from the settings. Here there is neither of those two
/// things, so the default value is the one that makes the plumb useful: at 0.6
/// degrees a gentle nudge is still free and a hard one warns.
pub const TILT_THRESHOLD_DEG: f32 = 0.6;

/// How much velocity the plumb keeps when it bounces off the ring
/// (`PlumbHandler.cpp`, "magic damping factor").
const PLUMB_BOUNCE: f32 = 0.8;

/// How many warnings are tolerated before the ball is over.
///
/// On a real machine this is **not the plumb's decision**: the plumb only
/// closes a contact and the ROM keeps the count. Three warnings with the
/// fourth one killing you is the most common setup, and it is what this does
/// as long as there is no bridge to PinMAME.
pub const WARNINGS_BEFORE_TILT: u32 = 3;

/// How long the ring is ignored after a warning, in milliseconds.
///
/// The plumb does not touch once and then hold still: it bounces off the ring
/// and touches it again for as long as its momentum lasts, and without this
/// ten nudges give twenty-five warnings. Real ROMs have the same debounce, for
/// the same reason: the contact is mechanical and it chatters.
const DEBOUNCE_MS: u32 = 500;

/// A damped harmonic oscillator (`DampedHarmonicOscillator.h`).
#[derive(Debug, Clone)]
struct DampedOscillator {
    mass: f32,
    /// Spring constant, in N/m.
    k: f32,
    /// Damping coefficient, in N·s/m.
    damping: f32,
    displacement: f32,
    velocity: f32,
    acceleration: f32,
}

impl DampedOscillator {
    /// `mass` in kilos, `frequency` in Hz —the natural one, undamped— and
    /// `zeta` the damping ratio (0 = free, 1 = critical).
    fn new(mass: f32, frequency: f32, zeta: f32) -> Self {
        let omega0 = std::f32::consts::TAU * frequency;
        Self {
            mass,
            k: mass * omega0 * omega0,
            damping: 2.0 * zeta * mass * omega0,
            displacement: 0.0,
            velocity: 0.0,
            acceleration: 0.0,
        }
    }

    /// Advances `dt` seconds under an external force in newtons.
    fn step(&mut self, force: f32, dt: f32) {
        self.acceleration =
            (force - self.damping * self.velocity - self.k * self.displacement) / self.mass;
        self.velocity += self.acceleration * dt;
        self.displacement += self.velocity * dt;
    }
}

/// The furniture (`CabinetPhysics`).
///
/// It is a 2D model with no torque, so a sideways nudge feels the same at the
/// top of the table as at the bottom. The original points that out and leaves
/// it that way.
#[derive(Debug, Clone)]
struct Cabinet {
    x: DampedOscillator,
    y: DampedOscillator,
    /// m/s².
    acceleration: Vec2,
    /// Visible offset, in metres.
    offset: Vec2,
}

impl Default for Cabinet {
    fn default() -> Self {
        Self {
            x: DampedOscillator::new(CABINET_MASS, FREQ_X, ZETA_X),
            y: DampedOscillator::new(CABINET_MASS, FREQ_Y, ZETA_Y),
            acceleration: Vec2::ZERO,
            offset: Vec2::ZERO,
        }
    }
}

impl Cabinet {
    /// One millisecond under a force in newtons.
    fn step(&mut self, force: Vec2) {
        self.x.step(force.x, DT);
        self.y.step(force.y, DT);
        self.acceleration = Vec2::new(self.x.acceleration, self.y.acceleration);
        self.offset = Vec2::new(
            self.x.displacement * OFFSET_FUDGE_X,
            self.y.displacement * OFFSET_FUDGE_Y,
        );
    }
}

/// An impulse in progress (`CabModelKeyboardNudge::Impulse`).
///
/// The acceleration rises and falls with a raised cosine, that is, it starts
/// at zero, reaches its peak halfway and comes back to zero. It is what makes
/// a nudge feel like a nudge and not like a collision.
#[derive(Debug, Clone, Copy)]
struct Impulse {
    length_ms: u32,
    elapsed_ms: u32,
    /// The peak, in m/s².
    peak: Vec2,
}

impl Impulse {
    fn in_progress(&self) -> bool {
        self.elapsed_ms <= self.length_ms
    }

    fn acceleration(&self) -> Vec2 {
        if !self.in_progress() {
            return Vec2::ZERO;
        }
        let t = self.elapsed_ms as f32 / self.length_ms as f32;
        self.peak * 0.5 * (1.0 - (std::f32::consts::TAU * t).cos())
    }
}

/// The tilt plumb (`PlumbHandler`).
///
/// It is a real pendulum: a bob at the end of a rod, with three forces on it
/// —gravity, the cabinet's shove and the rod— plus its damping. The position
/// is stored as a vector and not as an angle because the plumb moves in both
/// directions at once, and adding two angles is not the same as adding two
/// vectors.
#[derive(Debug, Clone)]
struct Plumb {
    /// Position of the bob relative to the hanging point, in metres. Its
    /// length is always [`PLUMB_LENGTH`].
    pos: Vec3,
    /// Angular velocity, in rad/s.
    omega: Vec3,
    /// What angle the ring sits at, in radians.
    threshold: f32,
    /// Whether it is touching right now.
    touching: bool,
    /// What fraction of the ring it reached, from 0 to 1 or more.
    fraction: f32,
}

impl Default for Plumb {
    fn default() -> Self {
        Self {
            pos: Vec3::new(0.0, 0.0, -PLUMB_LENGTH),
            omega: Vec3::ZERO,
            threshold: TILT_THRESHOLD_DEG.to_radians(),
            touching: false,
            fraction: 0.0,
        }
    }
}

impl Plumb {
    /// One millisecond. `acceleration` is the cabinet's, in m/s².
    ///
    /// Returns `true` on the instant it **starts** touching the ring, which is
    /// what counts as a warning. While it stays resting against it, it does
    /// not warn again.
    fn step(&mut self, acceleration: Vec2) -> bool {
        let mut axis = self.pos / PLUMB_LENGTH;

        // The change of reference frame comes in subtracted, just like on the
        // ball, and gravity pulls downwards.
        let acc = Vec3::new(-acceleration.x, -acceleration.y, -GRAVITY_MS2);

        // Torque per unit mass about the hanging point, and from there the
        // angular acceleration: for a point mass `I = m·L²`, so the mass
        // cancels out and what is left is `tau / L²`.
        let torque = self.pos.cross(acc);
        let mut alpha = torque / (PLUMB_LENGTH * PLUMB_LENGTH);

        // Nonlinear angular damping: the constant part kills the small
        // oscillations and the one that grows with the velocity keeps a hard
        // hit from spinning it around like a sling.
        let damping = PLUMB_DAMPING_LINEAR + PLUMB_DAMPING_NONLINEAR * self.omega.length();
        alpha -= self.omega * damping;

        // Integrate, taking out of the angular velocity the component along
        // the rod: that would be the bob spinning about itself, which means
        // nothing physically and only makes the number drift.
        self.omega += alpha * DT;
        self.omega -= axis * self.omega.dot(axis);

        // The bob moves with `r' = ω × r`, which preserves the length except
        // for the integration error; renormalizing corrects it.
        self.pos += self.omega.cross(self.pos) * DT;
        let length = self.pos.length();
        self.pos = if length > 1.0e-8 {
            self.pos * (PLUMB_LENGTH / length)
        } else {
            Vec3::new(0.0, 0.0, -PLUMB_LENGTH)
        };
        axis = self.pos / PLUMB_LENGTH;
        self.omega -= axis * self.omega.dot(axis);

        // How far it opened up from the vertical.
        let psi = Vec2::new(self.pos.x, self.pos.y)
            .length()
            .atan2(-self.pos.z);
        self.fraction = psi / self.threshold;

        let touches = self.fraction > 1.0;
        if touches {
            self.hit_the_ring(axis);
        }

        let edge = touches && !self.touching;
        self.touching = touches;
        edge
    }

    /// Leaves it resting right on the ring and bounces its velocity back.
    fn hit_the_ring(&mut self, axis: Vec3) {
        let limit = self.threshold - 1e-3;
        let planar = PLUMB_LENGTH * limit.sin();
        let theta = self.pos.x.atan2(self.pos.y);
        self.pos = Vec3::new(
            planar * theta.sin(),
            planar * theta.cos(),
            -PLUMB_LENGTH * limit.cos(),
        );

        // Reflects the linear velocity against the ring's normal and converts
        // it back to angular velocity.
        let v = self.omega.cross(self.pos);
        let reflected = v - axis * (2.0 * v.dot(axis));
        self.omega = self.pos.cross(reflected) / (PLUMB_LENGTH * PLUMB_LENGTH) * PLUMB_BOUNCE;
    }
}

/// The cabinet, its impulses and its plumb.
#[derive(Debug, Clone, Default)]
pub struct Nudge {
    cabinet: Cabinet,
    impulses: Vec<Impulse>,
    plumb: Plumb,
    /// How many times the plumb touched.
    pub warnings: u32,
    /// How long until it can warn again, in steps.
    debounce: u32,
    /// Whether the table is tilted. It does not come back on its own: the game
    /// resets it when the next ball starts.
    pub tilt: bool,
}

impl Nudge {
    /// Queues a nudge (`CabModelKeyboardNudge::Nudge`).
    ///
    /// `angle_deg` is in degrees, measured as in Visual Pinball: 0 is pushing
    /// the table forwards, 75 is the left nudge and 285 the right one
    /// (`InputManager.cpp:735-739`). `force` is the script's, where 2.0 is what
    /// tables use as a reference.
    ///
    /// With the table tilted the impulse is ignored: that is precisely the
    /// punishment.
    pub fn impulse(&mut self, angle_deg: f32, force: f32) {
        if self.tilt {
            return;
        }
        let a = angle_deg.to_radians();
        let scale = force * BASE_SCALE;
        self.impulses.push(Impulse {
            length_ms: IMPULSE_MS,
            elapsed_ms: 0,
            peak: Vec2::new(a.sin() * scale, -a.cos() * scale),
        });
    }

    /// One step. Returns the acceleration the ball feels, **in VPU/VPT²**.
    pub fn step(&mut self) -> Vec2 {
        let mut intent = Vec2::ZERO;
        self.impulses.retain_mut(|g| {
            g.elapsed_ms += 1;
            if !g.in_progress() {
                return false;
            }
            intent += g.acceleration();
            true
        });

        // The oscillator is driven with forces, not with accelerations.
        self.cabinet.step(intent * CABINET_MASS);

        self.debounce = self.debounce.saturating_sub(1);
        if self.plumb.step(self.cabinet.acceleration) && self.debounce == 0 {
            self.debounce = DEBOUNCE_MS;
            self.warnings += 1;
            if self.warnings > WARNINGS_BEFORE_TILT {
                self.tilt = true;
            }
        }

        self.cabinet.acceleration * MS2_TO_VPUVPT2
    }

    /// The acceleration of the last step, in 3D and in VPU/VPT²: the cabinet
    /// moves in the plane of the table and never upwards.
    pub fn acceleration(&self) -> Vec3 {
        (self.cabinet.acceleration * MS2_TO_VPUVPT2).extend(0.0)
    }

    /// How far the furniture moved, in VPU. The original uses it to shake the
    /// camera (`Renderer.cpp:2925`).
    pub fn offset(&self) -> Vec2 {
        self.cabinet.offset * crate::constants::VPU_PER_METER
    }

    /// How close to the ring the plumb is, from 0 to 1. It is what the UI can
    /// show so the player knows how much room is left.
    pub fn risk(&self) -> f32 {
        self.plumb.fraction.clamp(0.0, 1.0)
    }

    /// Whether there is any impulse still in progress.
    pub fn is_active(&self) -> bool {
        !self.impulses.is_empty()
    }

    /// Puts everything back to zero. Goes at the start of a new ball.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three nudges a keyboard has (`InputManager.cpp:735-739`), without
    /// the scatter.
    const LEFT: f32 = 75.0;
    const RIGHT: f32 = 285.0;
    const CENTER: f32 = 0.0;

    /// The average force of a keyboard nudge: `(0.6 + rand·0.8) · 2`.
    const FORCE: f32 = 2.0;

    fn run(e: &mut Nudge, steps: u32) {
        for _ in 0..steps {
            e.step();
        }
    }

    #[test]
    fn a_nudge_moves_the_table_and_dies_down() {
        let mut e = Nudge::default();
        e.impulse(LEFT, FORCE);

        // During the impulse the table really does accelerate.
        run(&mut e, 13);
        assert!(
            e.acceleration().length() > 0.1,
            "the table did not move: {:?}",
            e.acceleration()
        );

        // And a second later it is already still.
        run(&mut e, 1_000);
        assert!(!e.is_active());
        assert!(
            e.acceleration().length() < 0.05,
            "the table is still shaking: {:?}",
            e.acceleration()
        );
    }

    #[test]
    fn the_left_nudge_sends_the_ball_left() {
        // The ball subtracts the cabinet's acceleration, so pushing the table
        // towards `+x` moves it towards `-x`.
        let mut e = Nudge::default();
        e.impulse(LEFT, FORCE);
        run(&mut e, 13);
        assert!(e.acceleration().x > 0.0);

        let mut e = Nudge::default();
        e.impulse(RIGHT, FORCE);
        run(&mut e, 13);
        assert!(e.acceleration().x < 0.0);
    }

    #[test]
    fn the_forward_nudge_goes_along_the_table_axis() {
        let mut e = Nudge::default();
        e.impulse(CENTER, FORCE);
        run(&mut e, 13);
        let a = e.acceleration();
        // Towards `-y`, that is, pushing the table away from the player.
        assert!(a.y < 0.0);
        assert!(a.x.abs() < a.y.abs() * 0.01);
        assert_eq!(a.z, 0.0);
    }

    #[test]
    fn a_lone_nudge_does_not_tilt() {
        let mut e = Nudge::default();
        e.impulse(LEFT, FORCE);
        run(&mut e, 2_000);
        assert!(!e.tilt, "one nudge on its own cannot tilt");
    }

    #[test]
    fn keeping_at_it_ends_in_tilt() {
        let mut e = Nudge::default();
        for _ in 0..24 {
            e.impulse(LEFT, FORCE);
            run(&mut e, 120);
            if e.tilt {
                return;
            }
        }
        panic!(
            "keeping at it without stopping should tilt; {} warnings were left",
            e.warnings
        );
    }

    #[test]
    fn a_single_impulse_counts_a_single_warning() {
        // The plumb bounces off the ring and touches it again several times on
        // the same momentum. All of that is **one** warning.
        let mut e = Nudge::default();
        e.impulse(LEFT, 8.0);
        run(&mut e, 400);
        assert_eq!(
            e.warnings, 1,
            "one impulse alone cannot give more than one warning"
        );
    }

    #[test]
    fn a_gentle_nudge_is_still_free() {
        // The gentlest one that comes out of the original's scatter.
        let mut e = Nudge::default();
        e.impulse(LEFT, 1.2);
        run(&mut e, 2_000);
        assert_eq!(e.warnings, 0, "a gentle nudge should not warn");
    }

    #[test]
    fn with_the_table_tilted_the_nudge_does_nothing() {
        let mut e = Nudge {
            tilt: true,
            ..Default::default()
        };
        e.impulse(LEFT, 10.0);
        assert!(!e.is_active());
        assert_eq!(e.step(), Vec2::ZERO);
    }

    #[test]
    fn the_plumb_does_not_escape_the_ring() {
        // A colossal hit has to leave it resting, not spinning around.
        let mut e = Nudge::default();
        e.impulse(LEFT, 100.0);
        run(&mut e, 500);
        assert!(
            e.risk() <= 1.0,
            "the plumb ended up outside the ring: {}",
            e.risk()
        );
    }

    #[test]
    fn reset_puts_the_table_back_in_play() {
        let mut e = Nudge::default();
        for _ in 0..24 {
            e.impulse(LEFT, FORCE);
            run(&mut e, 120);
        }
        assert!(e.tilt);
        e.reset();
        assert!(!e.tilt);
        assert_eq!(e.warnings, 0);
        assert_eq!(e.risk(), 0.0);
    }

    #[test]
    fn the_unit_conversion_is_the_originals() {
        // The original's `VPUVPT2TOMS2` is worth 5.3975.
        assert!((1.0 / MS2_TO_VPUVPT2 - 5.3975).abs() < 1e-3);
    }
}
