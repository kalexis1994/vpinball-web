//! Lights: the playfield inserts and the general illumination.
//!
//! A light is not geometry with a material: it is a **shape** — an outline of
//! control points, sitting just above the surface — onto which a halo is painted
//! that fades with the distance to the center. It is drawn additively over
//! whatever is already there.
//!
//! Port of `Light::RenderSetup` (`light.cpp:434-460`) for the shape and of
//! `ClassicLightShader.hlsl:75-82` for the halo.
//!
//! # Almost all of them start out off
//!
//! The initial state comes from the file, and in a typical table nearly
//! everything is at zero: of F-14's 443 lights, 24 start on. The rest are turned
//! on by the game as it goes. Without a script engine, drawing only the ones the
//! file declares as on is the right thing — even if it means the table looks a
//! lot dimmer than a real game.

use crate::dragpoint;
use vpw_math::{Vec2, Vec3};

/// A light that is on, ready to draw.
#[derive(Debug, Clone)]
pub struct Light {
    pub name: String,
    /// The outline, already triangulated, in world space.
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Center of the halo, in VPU.
    pub center: Vec3,
    /// Range. Beyond this the light contributes nothing.
    pub falloff_radius: f32,
    /// Exponent of the falloff.
    pub falloff_power: f32,
    /// Intensity **already scaled** by the original's factor. See
    /// [`INTENSITY_FACTOR`].
    pub intensity: f32,
    /// Color near the center and color at the edge. The shader interpolates.
    pub color: [f32; 3],
    pub color2: [f32; 3],
    /// 0 off, 1 on; the values in between are half measures.
    pub state: f32,
    /// How the halo meets what is under it, from 0 to 1.
    ///
    /// At 0 the light is simply **added**, which is what a flat coloured disc
    /// over the artwork looks like. At 1 it **modulates**: the pixel underneath
    /// is multiplied by one plus the light's contribution, so a lit insert
    /// brightens the artwork it sits on instead of painting over it. The
    /// original calls the blend "a very crude approximation of real lighting"
    /// (`light.cpp:828`) and it is the difference between a lamp that looks
    /// painted on and one that looks lit.
    ///
    /// Only a bulb light has one; the classic kind is always additive.
    pub modulate: f32,
}

/// What the original scales a light's intensity by **for the bulb mesh alone**
/// (`light.cpp:776`):
///
/// ```text
/// lightColor_intensity.w = m_currentIntensity * 0.02f; //!! make configurable?
/// ```
///
/// Twenty-two lines further down, for the halo — the disc that is actually the
/// lit insert — it is the plain intensity (`light.cpp:798`):
///
/// ```text
/// lightColor_intensity.w = m_currentIntensity;
/// ```
///
/// The two are different draws of different meshes and only the first is
/// scaled. Applying it to the halo as well makes every lamp on the table fifty
/// times too dim, and the way that fails is worth writing down, because it does
/// not look like dimness. A bulb halo does not paint its colour on: it
/// multiplies what is under it by one plus its contribution. Divide the
/// contribution by fifty and the multiplier is one, so a lit insert is
/// pixel-for-pixel identical to an unlit one — and the table reads as a table
/// whose lamps are not wired up rather than as a table that is too dark.
/// Measured on a real game: turning on all hundred and forty-seven of a
/// playfield's lamps moved its average brightness by 0.6 out of 255.
///
/// There is no bulb mesh in this renderer yet. When there is, this is its
/// factor and not the halo's.
pub const INTENSITY_FACTOR: f32 = 0.02;

/// Converts a light from the file.
///
/// **Including the ones that start out off.** A table's lamps are almost all
/// off in the file — they are the game's lamps, and the game turns them on —
/// so dropping them here leaves a port with nothing to light up and a playfield
/// that stays dark however well the rest works. What is dropped is a light with
/// no intensity at all, which is a light that could never do anything.
pub fn build(l: &vpin::vpx::gameitem::light::Light, surface_z: f32) -> Option<Light> {
    // `state` is from 10.8; old tables carry `state_u32`. The value 2 is
    // "blinking", which without a script is treated as on.
    let state = l.state.unwrap_or(l.state_u32 as f32);
    let state = if state >= 2.0 { 1.0 } else { state };
    if l.intensity <= 0.0 {
        return None;
    }

    let outline = dragpoint::expand(
        &dragpoint::from_vpin(&l.drag_points),
        true,
        dragpoint::ACCURACY,
    );
    if outline.len() < 3 {
        return None;
    }

    let flat: Vec<Vec2> = outline
        .iter()
        .map(|p| Vec2::new(p.pos.x, p.pos.y))
        .collect();
    let indices = crate::triangulate::polygon(&flat);
    if indices.is_empty() {
        return None;
    }

    // Just above the surface, so it does not fight with it over depth
    // (`light.cpp:515`).
    let z = surface_z + 0.1;

    Some(Light {
        name: l.name.clone(),
        vertices: flat.iter().map(|p| [p.x, p.y, z]).collect(),
        indices,
        center: Vec3::new(l.center.x, l.center.y, surface_z),
        falloff_radius: l.falloff_radius.max(1.0),
        falloff_power: l.falloff_power,
        // Full brightness. `light.cpp:316` has
        // `targetIntensity = intensity * state`, and the scaling by state
        // happens where the state can change — at draw time — rather than here,
        // where it would be frozen at whatever the file happened to say.
        // The halo's own intensity, unscaled. See [`INTENSITY_FACTOR`] for the
        // draw that is scaled and why applying it here hides every lamp.
        intensity: l.intensity,
        color: color(&l.color),
        color2: color(&l.color2),
        state,
        // A classic light has no bulb blend at all, and the original clamps a
        // bulb's away from both ends: zero disables the blend outright and one
        // "looks not good with day-night changes" (`light.cpp:830`).
        modulate: if l.is_bulb_light {
            l.bulb_modulate_vs_add.clamp(0.0001, 0.9999)
        } else {
            0.0
        },
    })
}

/// A lamp's colour, the way the original converts one: a divide by 255 and no
/// gamma decode (`convertColor`, `utils/color.h:22`, fed to a light at
/// `light.cpp:711-712`). See `geometry::color` for why the asymmetry with
/// textures is deliberate.
fn color(c: &vpin::vpx::color::Color) -> [f32; 3] {
    [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
}
