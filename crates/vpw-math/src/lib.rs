//! Visual Pinball math types and unit conversions.
//!
//! The original engine defines its own `Vertex2D` / `Vertex3Ds` / `Matrix3D`
//! (`src/math/` in the C++ repo). We do not reimplement them here: we use
//! `glam`, which is already vectorised and battle-tested, and we only supply
//! what is specific to VP: the unit system.

pub use glam;
pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec3A, Vec4};

/// Visual Pinball units.
///
/// Ported from the note in `src/physics/physconst.h`:
///
/// - **Length (VPU):** 50 VPU are 1"1/16, the diameter of a standard ball.
///   Therefore `1 VPU = 1.0625/50 inches = 0.53975 mm`.
/// - **Time (VPT):** for historical reasons `1 VPT = 10 ms`.
/// - **Mass:** 1 VP mass unit = the mass of a standard ball = 80 g.
pub mod units {
    /// Meters per VP length unit (5.3975e-4).
    pub const METERS_PER_VPU: f32 = 1.0625 / 50.0 * 0.0254;
    /// VP units per meter (~1852.71).
    pub const VPU_PER_METER: f32 = 1.0 / METERS_PER_VPU;
    /// Seconds per VP time unit.
    pub const SECONDS_PER_VPT: f32 = 0.01;
    /// VP time units per second.
    pub const VPT_PER_SECOND: f32 = 100.0;
    /// Kilograms per VP mass unit (a standard ball weighs 80 g).
    pub const KG_PER_VPM: f32 = 0.08;

    #[inline]
    pub fn vpu_to_meters(vpu: f32) -> f32 {
        vpu * METERS_PER_VPU
    }

    #[inline]
    pub fn meters_to_vpu(m: f32) -> f32 {
        m * VPU_PER_METER
    }

    #[inline]
    pub fn vpt_to_seconds(vpt: f32) -> f32 {
        vpt * SECONDS_PER_VPT
    }

    #[inline]
    pub fn seconds_to_vpt(s: f32) -> f32 {
        s * VPT_PER_SECOND
    }
}

#[cfg(test)]
mod tests {
    use super::units::*;

    #[test]
    fn length_conversion_matches_the_original_engine() {
        // physconst.h: 1 U = .53975 mm
        assert!((vpu_to_meters(1.0) - 5.3975e-4).abs() < 1e-9);
        // physconst.h: 1 m ~= 1852.71 U
        assert!((meters_to_vpu(1.0) - 1852.71).abs() < 0.01);
    }

    #[test]
    fn earth_gravity_in_vp_units() {
        // physconst.h: g = 9.81 m/s^2 = 1.81751 U/T^2
        let g_vp = meters_to_vpu(9.81) / (VPT_PER_SECOND * VPT_PER_SECOND);
        assert!((g_vp - 1.81751).abs() < 1e-4, "got {g_vp}");
    }
}
