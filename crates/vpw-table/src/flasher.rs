//! Flashers: the strobes, beacons and flash domes of a table.
//!
//! A flasher is a flat polygon of control points, lifted to a height and
//! turned by three angles, with one or two pictures on it and a colour. It is
//! not lit by anything — it *is* the light — and it is drawn blended over what
//! is already there, every frame, because a script switches it on and off,
//! fades it and moves it as the game goes.
//!
//! Port of `Flasher::RenderSetup` (`flasher.cpp:1030-1125`) for the mesh and
//! of the placement matrix in `Flasher::Render` (`flasher.cpp:1182-1210`). What
//! the shader does with the numbers is in `vpw_render::flashers`.
//!
//! # Two halves
//!
//! [`Flasher`] is what the file fixes: the outline, its texture coordinates,
//! where it turns about, how it sorts. [`State`] is everything a script may
//! change — and that is nearly everything else: a table writes `Visible`,
//! `Opacity`, `IntensityScale`, `Color`, both images and all three rotations
//! from its script, and `core.vbs` toggles `Visible` on every flasher wired to
//! a solenoid (`core.vbs:2534`). Keeping the two apart is what lets the
//! renderer upload the shape once and push a small block of numbers a frame.
//!
//! # A flasher on 10.8 is also the machine's display
//!
//! From 10.8 a flasher whose `RenderMode` is [`RenderMode::Dmd`] shows the dot
//! matrix rather than a picture, and that is how a modern table places the
//! display on the playfield or the head. The mesh is then the outline's
//! bounding rectangle with texture coordinates from zero to one
//! (`flasher.cpp:1095-1104`), and what goes on it is the machine's frame.

use crate::dragpoint;
use vpw_math::{Mat4, Vec2, Vec3};

/// What the flasher shows: a picture, or one of the machine's displays.
///
/// `FlasherData::RenderMode` (`flasher.h:21-28`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// The blended images. What every table before 10.8 has.
    #[default]
    Flasher,
    /// The dot matrix display.
    Dmd,
    /// A screen — CRT or LCD — fed by a plugin. Not drawn here.
    Display,
    /// An alphanumeric segment display fed by a plugin. Not drawn here.
    AlphaSeg,
    /// One of the ancillary windows. Not drawn here.
    External,
}

/// How two images on a flasher are combined (`Filters`, `vpinball.idl:56`).
///
/// The number each one carries is what the shader compares against
/// (`fs_flasher.sc:11-15`), so it is fixed here rather than left to the
/// compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    None = 0,
    Additive = 1,
    /// The default (`Settings_properties.inl:978`).
    #[default]
    Overlay = 2,
    Multiply = 3,
    Screen = 4,
}

impl Filter {
    /// The name a script writes to `Filter`, compared the way
    /// `Flasher::put_Filter` compares it — lower-cased, and anything it does
    /// not know leaves the value alone (`flasher.cpp:710-726`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(Filter::None),
            "additive" => Some(Filter::Additive),
            "overlay" => Some(Filter::Overlay),
            "multiply" => Some(Filter::Multiply),
            "screen" => Some(Filter::Screen),
            _ => None,
        }
    }

    /// The name `Flasher::get_Filter` hands back (`flasher.cpp:692-708`).
    pub fn name(self) -> &'static str {
        match self {
            Filter::None => "None",
            Filter::Additive => "Additive",
            Filter::Overlay => "Overlay",
            Filter::Multiply => "Multiply",
            Filter::Screen => "Screen",
        }
    }

    fn from_vpin(f: &vpin::vpx::gameitem::flasher::Filter) -> Self {
        use vpin::vpx::gameitem::flasher::Filter as F;
        match f {
            F::None => Filter::None,
            F::Additive => Filter::Additive,
            F::Overlay => Filter::Overlay,
            F::Multiply => Filter::Multiply,
            F::Screen => Filter::Screen,
        }
    }
}

/// A corner of the polygon: where it is, flat on the table, and which texel
/// goes there. The 3D position is not here because the rotations and the
/// height are the script's to change; see [`Flasher::transform`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlasherVertex {
    /// In table space, at `z = 0`.
    pub pos: [f32; 2],
    pub uv: [f32; 2],
}

/// Everything about a flasher that a script cannot change.
#[derive(Debug, Clone)]
pub struct Flasher {
    pub name: String,
    /// The outline, expanded from its control points and triangulated, lying
    /// flat on the table. `transform` lifts and turns it.
    pub vertices: Vec<FlasherVertex>,
    pub indices: Vec<u32>,
    /// The centre of the outline's bounding box, which is what the rotations
    /// turn about (`flasher.cpp:1186-1188`) and what the script reads back as
    /// `X` and `Y`.
    pub center: Vec2,
    /// `FLDB`. Only a sort key: the original's `DrawMesh` folds it into the
    /// depth it orders transparent draws by and never touches the depth
    /// buffer with it (`RenderDevice.cpp:2708`).
    pub depth_bias: f32,
    pub mode: RenderMode,
    /// `LMAP`: the lamp this flasher follows, if any. A flasher bound to a
    /// light scales its own alpha by that light's current level
    /// (`flasher.cpp:1171-1177`), which is how a 10.8 table paints a
    /// pre-rendered lightmap over the playfield and has it fade with the bulb.
    pub light_map: Option<String>,
    /// Where everything the script can write starts out.
    pub state: State,
}

/// What a flasher is doing right now: the members a script writes.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub visible: bool,
    /// The centre as the script sees it. `put_X` moves every control point by
    /// the difference (`flasher.cpp:563-570`), so a moved flasher is the same
    /// outline translated, and the renderer does exactly that.
    pub x: f32,
    pub y: f32,
    /// `FHEI`: how far above the playfield it floats.
    pub height: f32,
    /// `FROX`, `FROY`, `FROZ`, in degrees.
    pub rot: [f32; 3],
    /// `COLR`, as the original converts it: divided by 255 and not decoded
    /// (`convertColor`, `utils/color.h:22`; see `geometry::color`).
    pub color: [f32; 3],
    /// `FALP`, the script's `Opacity`: **out of a hundred**, and never below
    /// zero (`flasher.cpp:502`).
    pub alpha: f32,
    /// The script's `IntensityScale`. Starts at one (`flasher.cpp:138`).
    pub intensity_scale: f32,
    /// `MOVA`. How much an additive flasher modulates what is under it rather
    /// than adding to it; and, on a display flasher, its opacity
    /// (`flasher.cpp:1338`). The default is 0.9 (`Settings_properties.inl:971`).
    pub modulate_vs_add: f32,
    /// `ADDB`: whether it adds light or paints over.
    pub add_blend: bool,
    /// `FILT`, and `FIAM` **out of a hundred** — the shader wants a fraction
    /// (`flasher.cpp:1294`), and tables run to several thousand per cent.
    pub filter: Filter,
    pub filter_amount: f32,
    /// `IMAG` and `IMAB`. On a display flasher `image_a` is the glass over the
    /// dots, which is not drawn here.
    pub image_a: String,
    pub image_b: String,
}

impl State {
    /// The colour and alpha the shader is handed, `staticColor_Alpha`
    /// (`flasher.cpp:1241`): `convertColor(color, alpha * intensity_scale /
    /// 100)`.
    pub fn shader_color(&self) -> [f32; 4] {
        [
            self.color[0],
            self.color[1],
            self.color[2],
            self.alpha * self.intensity_scale / 100.0,
        ]
    }

    /// Whether there is anything to draw at all, before the lightmap is
    /// applied: hidden, black, fully transparent or scaled to nothing are all
    /// an early return in the original (`flasher.cpp:1162`, `:1179`).
    pub fn is_drawn(&self) -> bool {
        self.visible && self.color != [0.0; 3] && self.alpha != 0.0 && self.intensity_scale != 0.0
    }

    /// `modulate_vs_add` as the shader gets it: "avoid 0, as it disables the
    /// blend and avoid 1 as it looks not good with day->night changes"
    /// (`flasher.cpp:1242`). The bulb light clamps its own the same way.
    pub fn clamped_modulate(&self) -> f32 {
        self.modulate_vs_add.clamp(0.00001, 0.9999)
    }
}

impl Flasher {
    /// Where the polygon stands, with the script's numbers.
    ///
    /// `flasher.cpp:1188-1192` builds it as
    ///
    /// ```text
    /// translate(-cx, -cy, 0) · RotZ · RotY · RotX · translate(cx, cy, height)
    /// ```
    ///
    /// in the original's row-vector order, which is the order the steps are
    /// applied: to the origin, turn about z then y then x, back to the centre
    /// and up to the height. `glam` multiplies the other way round, so the
    /// chain is written backwards — see the note at the top of
    /// `geometry.rs`. Getting the rotation order wrong is invisible on the
    /// tables that only tilt about one axis, and every LOTR flasher does.
    ///
    /// A script that writes `X` or `Y` has moved every control point by the
    /// difference, and with it the centre the rotations turn about; that is a
    /// translation on the outside of the whole thing.
    pub fn transform(&self, s: &State) -> Mat4 {
        let c = self.center;
        Mat4::from_translation(Vec3::new(s.x - c.x, s.y - c.y, 0.0))
            * Mat4::from_translation(Vec3::new(c.x, c.y, s.height))
            * Mat4::from_rotation_x(s.rot[0].to_radians())
            * Mat4::from_rotation_y(s.rot[1].to_radians())
            * Mat4::from_rotation_z(s.rot[2].to_radians())
            * Mat4::from_translation(Vec3::new(-c.x, -c.y, 0.0))
    }

    /// The depth the original sorts a transparent draw by: `depthBias -
    /// center.z` (`RenderDevice.cpp:2708`), with the centre at the flasher's
    /// height (`flasher.cpp:1212`). Larger is further back and is drawn first
    /// (`RenderPass.cpp:118-121`).
    ///
    /// A display flasher is pushed ten thousand further back so it lands
    /// "after opaques and before transparents" (`flasher.cpp:1341-1343`,
    /// `RenderPass.cpp:86`).
    pub fn sort_depth(&self, s: &State) -> f32 {
        let bias = match self.mode {
            RenderMode::Flasher => self.depth_bias,
            _ => self.depth_bias - 10000.0,
        };
        bias - s.height
    }
}

/// The outline as the original draws it, and its bounding box.
///
/// `GetRgVertex` with the defaults: a closed loop at full accuracy
/// (`dragpoint.h:156`). The box is of the **curve**, not of the control
/// points — `UpdateCenter` (`flasher.cpp:73-100`) walks the expanded vertices
/// — and on an outline with smooth points the two differ, which would put
/// the script's `X` a few units from where the renderer turns the polygon.
///
/// `None` for an outline with fewer than three points or no area, which the
/// original leaves without a mesh (`flasher.cpp:1044-1049`, `:1114-1118`).
pub fn outline(f: &vpin::vpx::gameitem::flasher::Flasher) -> Option<(Vec<Vec2>, Vec2, Vec2)> {
    let expanded = dragpoint::expand(
        &dragpoint::from_vpin(&f.drag_points),
        true,
        dragpoint::ACCURACY,
    );
    if expanded.len() < 3 {
        return None;
    }
    let flat: Vec<Vec2> = expanded
        .iter()
        .map(|p| Vec2::new(p.pos.x, p.pos.y))
        .collect();
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in &flat {
        min = min.min(*p);
        max = max.max(*p);
    }
    if max.x <= min.x || max.y <= min.y {
        return None;
    }
    Some((flat, min, max))
}

/// Where the script's `X` and `Y` start: the centre of the outline's box
/// (`flasher.cpp:98-99`). For whoever holds the script's numbers, so they
/// begin where the renderer's centre is.
pub fn center(f: &vpin::vpx::gameitem::flasher::Flasher) -> Option<Vec2> {
    outline(f).map(|(_, min, max)| (min + max) * 0.5)
}

/// Converts a flasher from the file.
///
/// **Including the ones that start out hidden**: `core.vbs` shows and hides
/// flashers off the solenoids (`vpmToggleObj`, `core.vbs:2534`), so a strobe
/// saved off is precisely the one the game is about to switch on. What is
/// dropped is a flasher with no outline to draw.
///
/// `playfield` is the table's extent, which the world-aligned texture mapping
/// needs. `flasher.cpp:1079` divides by `m_right - m_left`, not by the right
/// edge, though on every table seen the left is zero.
pub fn build(
    f: &vpin::vpx::gameitem::flasher::Flasher,
    playfield: crate::geometry::Bounds,
) -> Option<Flasher> {
    use vpin::vpx::gameitem::flasher::RenderMode as M;

    // `BGLS`: a flasher that lives on the desktop backdrop is a 2D overlay
    // drawn in screen space over the backglass picture (`flasher.cpp:1200-1205`
    // stretches it to the render target), not something standing on the
    // table. There is no backdrop here, and placing it in the scene by its
    // table coordinates would hang a stray rectangle somewhere near the top
    // of the playfield.
    if f.backglass == Some(true) {
        return None;
    }

    // `RDMD` from 10.8.1; before that `IDMD`, a bool the loader maps onto the
    // same two modes (`flasher.cpp:507-514`).
    let mode = match &f.render_mode {
        Some(M::Flasher) => RenderMode::Flasher,
        Some(M::DMD) => RenderMode::Dmd,
        Some(M::Display) => RenderMode::Display,
        Some(M::AlphaSeg) => RenderMode::AlphaSeg,
        Some(M::ExtRender) => RenderMode::External,
        None if f.is_dmd == Some(true) => RenderMode::Dmd,
        None => RenderMode::Flasher,
    };

    let (flat, min, max) = outline(f)?;
    let size = max - min;
    let center = (min + max) * 0.5;

    let (vertices, indices) = if mode == RenderMode::Flasher {
        let indices = crate::triangulate::polygon(&flat);
        if indices.is_empty() {
            return None;
        }
        // `flasher.cpp:1081-1093`. Wrap stretches the picture over the
        // outline's box; world lays it over the whole table so several
        // flashers can share one playfield-sized overlay.
        let wrap = f.image_alignment
            == vpin::vpx::gameitem::ramp_image_alignment::RampImageAlignment::Wrap;
        let table = playfield.max - playfield.min;
        let vertices = flat
            .iter()
            .map(|p| FlasherVertex {
                pos: [p.x, p.y],
                uv: if wrap {
                    [(p.x - min.x) / size.x, (p.y - min.y) / size.y]
                } else {
                    [p.x / table.x, p.y / table.y]
                },
            })
            .collect();
        (vertices, indices)
    } else {
        // A display is the outline's bounding rectangle with the frame
        // stretched over it, whatever shape the author drew
        // (`flasher.cpp:1095-1104`). The original triangulates the first four
        // control points and applies that to the rectangle's corners; for the
        // rectangle every table draws that is the same two triangles this
        // gives, and for anything else it is the two triangles that were
        // meant.
        let corner = |x: f32, y: f32, u: f32, v: f32| FlasherVertex {
            pos: [x, y],
            uv: [u, v],
        };
        (
            vec![
                corner(min.x, min.y, 0.0, 0.0),
                corner(min.x, max.y, 0.0, 1.0),
                corner(max.x, max.y, 1.0, 1.0),
                corner(max.x, min.y, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    };

    Some(Flasher {
        name: f.name.clone(),
        vertices,
        indices,
        center,
        depth_bias: f.depth_bias,
        mode,
        light_map: f.light_map.clone().filter(|n| !n.is_empty()),
        state: State {
            visible: f.is_visible,
            x: center.x,
            y: center.y,
            height: f.height,
            rot: [f.rot_x, f.rot_y, f.rot_z],
            color: [f.color.r, f.color.g, f.color.b].map(|v| f32::from(v) / 255.0),
            // `max(0, ...)`, `flasher.cpp:502`.
            alpha: f.alpha.max(0) as f32,
            intensity_scale: 1.0,
            modulate_vs_add: f.modulate_vs_add,
            add_blend: f.add_blend,
            filter: Filter::from_vpin(&f.filter),
            filter_amount: f.filter_amount as f32,
            image_a: f.image_a.clone(),
            image_b: f.image_b.clone(),
        },
    })
}
