//! Physics engine constants, ported 1:1 from `src/physics/physconst.h`.
//!
//! The values are kept identical to the original on purpose: any deviation
//! changes the behaviour of existing tables. If something gets touched for
//! performance, it has to be the *algorithm*, not these numbers.

/// Microseconds between physics steps (physics at 1000 Hz).
pub const PHYSICS_STEPTIME_US: u64 = 1000;
/// Physics step in seconds.
pub const PHYSICS_STEPTIME_S: f64 = PHYSICS_STEPTIME_US as f64 * 1e-6;

/// Default step in microseconds (1 VPT = 10 ms, a historical unit).
pub const DEFAULT_STEPTIME_US: u64 = 10_000;
/// Default step in seconds.
pub const DEFAULT_STEPTIME_S: f64 = 0.01;

/// Physics step expressed in Visual Pinball Time (VPT).
pub const PHYS_FACTOR: f32 = (PHYSICS_STEPTIME_S / DEFAULT_STEPTIME_S) as f32;

// --- Table defaults ----------------------------------------------------------

/// What a table's gravity setting defaults to, as a **multiple of Earth's**.
///
/// `m_Gravity = 0.97f * GRAVITYCONST` (`pintable.cpp:3833`), and the file
/// stores the product, not this. So this number is not an acceleration and
/// must never be handed to anything that wants one — see
/// [`DEFAULT_TABLE_GRAVITY_VPU`].
pub const DEFAULT_TABLE_GRAVITY: f32 = 0.97;

/// The same, as the acceleration the ball actually falls at, in VPU per VP
/// time unit squared.
///
/// The original's `SetGravity(slope, strength)` (`PhysicsEngine.cpp:104`) takes
/// the product and nothing else. Handing it the bare 0.97 instead — which is
/// what this port did — makes every table on earth 1.82 times too floaty, and
/// the arithmetic says so plainly: a ball dropped fifty VP units, which is one
/// ball diameter, or twenty-seven millimetres, takes 101 ms at 0.97 and 75 ms
/// at this. Seventy-five is what a real one takes.
///
/// A table's own number is read from the file and used in preference; this is
/// only what a table that does not say gets.
pub const DEFAULT_TABLE_GRAVITY_VPU: f32 = DEFAULT_TABLE_GRAVITY * GRAVITYCONST;
pub const DEFAULT_TABLE_CONTACTFRICTION: f32 = 0.075;
pub const DEFAULT_TABLE_SCATTERANGLE: f32 = 0.5;
pub const DEFAULT_TABLE_ELASTICITY: f32 = 0.25;
pub const DEFAULT_TABLE_ELASTICITY_FALLOFF: f32 = 0.0;
pub const DEFAULT_TABLE_PFSCATTERANGLE: f32 = 0.0;
pub const DEFAULT_TABLE_MIN_SLOPE: f32 = 6.0;
pub const DEFAULT_TABLE_MAX_SLOPE: f32 = 6.0;

/// Static level of detail with which ramps and rubbers are approximated for
/// the collision code.
pub const HIT_SHAPE_DETAIL_LEVEL: f32 = 7.0;

/// Earth's gravity expressed in VP units (U/T^2).
pub const GRAVITYCONST: f32 = 1.81751;

// --- Collisions --------------------------------------------------------------

/// "Almost zero" threshold in well-ported linear conditions.
pub const C_PRECISION: f32 = 0.01;
/// Tolerance for collisions against the endpoints of a segment.
pub const C_TOL_ENDPNTS: f32 = 0.0;
/// Tolerance for collisions against point radii.
pub const C_TOL_RADIUS: f32 = 0.005;

/// Positive contact skin: every collision inside this skin reports time zero.
/// Beyond it objects pass through each other.
pub const PHYS_SKIN: f32 = 25.0;
/// Default ball diameter, in VP units.
pub const DEFAULT_BALL_SIZE: f32 = 25.0;
/// Skin around the object that grows its size in order to measure contacts.
pub const PHYS_TOUCH: f32 = 0.05;

/// Below this normal velocity the collision is treated as a sustained contact
/// instead of as an impulse.
pub const C_LOWNORMVEL: f32 = 0.0001;
pub const C_CONTACTVEL: f32 = 0.099;

// --- Fixups of the "old physics" path ----------------------------------------
// The original has two branches (`NEW_PHYSICS` vs. legacy) and compiles the
// legacy one by default. We port that one, which is what every published
// table runs.

pub const C_EMBEDDED: f32 = 0.0;
pub const C_EMBEDSHOT: f32 = 0.05;
/// Gain in the displacement correction for rigid contacts (steel against hard
/// plastic or hard wood).
pub const C_DISP_GAIN: f32 = 0.9875;
pub const C_DISP_LIMIT: f32 = 5.0;

// --- Trigger/kicker hysteresis -----------------------------------------------

/// Minimum time/intersection difference allowed in the simulation.
pub const STATICTIME: f32 = 0.02;
/// Number of intersections within `STATICTIME` from which the clamp is
/// forced. 0 = always clamp.
pub const STATICCNTS: u32 = 10;

/// Precision level and cycle count for the iterative flipper calculations.
pub const C_INTERATIONS: u32 = 20;

/// Velocity threshold considered to be zero (plumb / tilt).
pub const VELOCITY_EPSILON: f32 = 0.05;

// --- Real-world units --------------------------------------------------------
// `src/core/def.h:610-627`. Almost the whole engine lives in VPU and VPT and
// does not need to know what they really measure, but the cabinet and the
// plumb are modelled in metres and seconds —they are real mechanics, with
// masses in kilos and a gravity of 9.8— so we need to convert back and forth.
//
// 50 VPU are 1.0625 inches, which is the diameter of a standard ball.

/// How many VPU a metre measures.
pub const VPU_PER_METER: f32 = 50.0 / (0.0254 * 1.0625);
/// How many VPT a second lasts (1 VPT = 10 ms).
pub const VPT_PER_SECOND: f32 = 100.0;
/// From m/s² to VPU/VPT², which is what the ball works in.
pub const MS2_TO_VPUVPT2: f32 = VPU_PER_METER / (VPT_PER_SECOND * VPT_PER_SECOND);
/// Real gravity, in m/s². Not the table's: that one is in
/// [`DEFAULT_TABLE_GRAVITY`] and already comes tilted and in VP units.
pub const GRAVITY_MS2: f32 = 9.80665;
