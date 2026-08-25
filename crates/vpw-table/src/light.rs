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
//! # An insert is artwork, not a disc
//!
//! A classic light with an image (`IMG1`) is not a coloured halo: it is the
//! insert's artwork, sampled at **table-space** UVs — the whole playfield maps
//! to 0..1, `light.cpp:519-520` — lit through the material of whatever it lies
//! on, and *then* the halo is folded into it with an overlay and a screen
//! (`ClassicLightShader.hlsl:52-87`). Drawing such a light as a halo alone is
//! the single most visible way a port of this renderer reads as wrong: every
//! insert on every table becomes a flat coloured spot where the player expects
//! to see the words on it light up. This module carries the image name, the
//! mode and the UVs; the shader does the rest.
//!
//! # Almost all of them start out off
//!
//! The initial state comes from the file, and in a typical table nearly
//! everything is at zero: of F-14's 443 lights, 24 start on. The rest are turned
//! on by the game as it goes. Without a script engine, drawing only the ones the
//! file declares as on is the right thing — even if it means the table looks a
//! lot dimmer than a real game.
//!
//! # A lamp is not a switch
//!
//! [`Lamp`] is the other half of this file: the part of a light that changes
//! between frames. `Light::UpdateAnimation` (`light.cpp:299-357`) ramps the
//! intensity toward its target instead of assigning it, walks the blink
//! pattern, and — for the incandescent fader — runs a tungsten filament's
//! thermal model, so a lamp warms up orange and dies away red. Writing the
//! level straight to the GPU instead gives a table where every insert snaps
//! like a light switch, which is the most visible way a port of this renderer
//! reads as fake.

use crate::dragpoint;
use crate::geometry::Bounds;
use std::sync::OnceLock;
use vpin::vpx::VPX;
use vpin::vpx::gameitem::GameItemEnum;
use vpw_math::{Vec2, Vec3};

/// The `m_inPlayState` value that means "run the blink pattern"
/// (`LightStateBlinking`, `light.h:19`). It is a float and not a flag in the
/// original because from 10.8 a state may be any level between off and on, and
/// two is simply the value past the top of that range.
pub const BLINKING: f32 = 2.0;

/// How a lamp travels from the level it is showing to the level it was asked
/// for (`FADE`, `light.h:23`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fader {
    /// Straight there. What a switch does.
    None,
    /// A ramp of [`Light::fade_up`] / [`Light::fade_down`] units of intensity
    /// per millisecond (`light.cpp:322-334`).
    ///
    /// The default, and not as the safe choice: the original's `LightData`
    /// initialises `m_fader` to `FADER_LINEAR` (`light.h:67`), so a file with no
    /// `FADE` chunk — which is every pre-10.8 table — fades linearly.
    #[default]
    Linear,
    /// A tungsten filament heating up and cooling down (`light.cpp:336-353`).
    ///
    /// Slower off the mark than the linear ramp and much slower to die, and it
    /// tints the lamp by the filament's colour temperature on the way, which is
    /// what makes a real bulb go orange as it fades out.
    Incandescent,
}

impl From<&vpin::vpx::gameitem::light::Fader> for Fader {
    fn from(f: &vpin::vpx::gameitem::light::Fader) -> Self {
        use vpin::vpx::gameitem::light::Fader as F;
        match f {
            F::None => Fader::None,
            F::Linear => Fader::Linear,
            F::Incandescent => Fader::Incandescent,
        }
    }
}

/// A light that is on, ready to draw.
#[derive(Debug, Clone)]
pub struct Light {
    pub name: String,
    /// The outline, already triangulated, in world space.
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// One texture coordinate per vertex, in **table space**: the whole
    /// playfield maps to 0..1 (`light.cpp:519-520`). Empty when there is
    /// nothing to sample — a bulb, or a classic light with no image.
    ///
    /// Table space and not the outline's own: the insert's image is a picture
    /// of the *whole playfield* with that insert lit, and the light's shape is
    /// a window cut into it. Mapping the outline to 0..1 instead would stretch
    /// the entire playfield into one insert.
    pub uvs: Vec<[f32; 2]>,
    /// `IMG1`: the insert's artwork, by image name, or empty. Only a classic
    /// light carries one — the original throws a bulb's away before it looks
    /// at it (`light.cpp:708`: `offTexel` is null for a bulb light). Whether
    /// the name resolves to a picture is the renderer's question; a name that
    /// does not is drawn as if there were none (`light.cpp:823`).
    pub image: String,
    /// `IMMO` ("passthrough"): the artwork is shown as it is, not lit through
    /// the surface material (`ClassicLightShader.hlsl:60-61`, `lightingOff`).
    /// Meaningless without [`image`](Light::image).
    pub image_mode: bool,
    /// The material of what the light lies on — the playfield's, or the wall's
    /// top (`light.cpp:373`). The insert's artwork is lit *through it*
    /// (`ClassicLightShader.hlsl:65-70`): the shader takes the texel as the
    /// base colour and the surface's glossiness and clearcoat as its own.
    pub surface_material: String,
    /// The image of what the light lies on (`light.cpp:374`). It decides
    /// whether an **unlit** insert is drawn at all: one whose artwork is the
    /// surface's own picture adds nothing the playfield does not already show
    /// and is skipped (`light.cpp:714`), one with a picture of its own is drawn
    /// dark, artwork and all, because the playfield underneath does not have
    /// it. See [`Light::drawn_when_off`].
    pub surface_image: String,
    /// Center of the halo, in VPU. Its **z is the falloff's centre**, which is
    /// not where the outline sits: see [`build`].
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
    ///
    /// A lamp the file declares as *blinking* has this at 1 and [`blinking`]
    /// set: the level it shows is then the blink pattern's business, and one is
    /// where the pattern starts if nothing ever ticks it.
    ///
    /// [`blinking`]: Light::blinking
    pub state: f32,
    /// Whether the file's state was `LightStateBlinking` (`light.cpp:315`).
    pub blinking: bool,
    /// Whether this is a bulb — a halo floating over the surface — rather than
    /// a classic insert lying on it. It decides the blend, the halo's height
    /// and whether the light reaches the transmitted-light buffer at all.
    pub is_bulb: bool,
    /// `TRMS`: how much of this lamp reaches the transmitted-light buffer, the
    /// one a translucent part reads to find the light arriving underneath it
    /// (`light.cpp:801`). Zero keeps it out of that buffer entirely
    /// (`light.cpp:600`).
    pub transmission_scale: f32,
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
    /// How this one fades. See [`Fader`].
    pub fader: Fader,
    /// `FASP` / `FASD`: intensity per millisecond, up and down
    /// (`light.cpp:325`, `:330`).
    pub fade_up: f32,
    pub fade_down: f32,
    /// `BPAT`, one entry per character: true where the pattern says lit
    /// (`light.cpp:315`, which compares the character to `'1'`).
    pub blink: Vec<bool>,
    /// `BINT`: milliseconds one character of the pattern lasts
    /// (`light.h:319`).
    pub blink_interval: f32,
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

/// What a light lies on, and the table it lies in.
///
/// `Light::RenderSetup` resolves the surface a light names (`SURF`) into a
/// height, a material and an image (`light.cpp:371-374`); the height is what
/// every part standing on something takes, and the other two belong to the
/// classic light alone. The playfield's own when the name is empty, and a
/// wall's top or a ramp's otherwise (`PinTable::GetSurfaceMaterial` and
/// `GetSurfaceImage`, `pintable.cpp:5178-5207`); a name that matches nothing
/// falls back to the playfield the way the original does, with a logged error
/// and no other consequence.
///
/// The table's extent is here because the insert's texture coordinates are in
/// **table space** (`light.cpp:499-500`), and a light cannot know that on its
/// own.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    /// The playfield's extent in VPU.
    pub table: Bounds,
    /// See [`Light::surface_material`].
    pub material: String,
    /// See [`Light::surface_image`].
    pub image: String,
}

impl Site {
    /// The playfield itself, with what the table says it is made of.
    pub fn playfield(vpx: &VPX, table: Bounds) -> Self {
        let g = &vpx.gamedata;
        Site {
            table,
            material: g.playfield_material.clone(),
            image: g.image.clone(),
        }
    }

    /// The surface a light names, or the playfield when it names none or
    /// names something that is not there (`pintable.cpp:5178-5207`).
    pub fn resolve(vpx: &VPX, surface: &str, table: Bounds) -> Self {
        if surface.is_empty() {
            return Self::playfield(vpx, table);
        }
        vpx.gameitems
            .iter()
            .find_map(|item| match item {
                // A wall's *top* material (`pintable.cpp:5187`): the light lies
                // on top of it, and the side is what the ball hits.
                GameItemEnum::Wall(w) if w.name.eq_ignore_ascii_case(surface) => Some(Site {
                    table,
                    material: w.top_material.clone(),
                    image: w.image.clone(),
                }),
                GameItemEnum::Ramp(r) if r.name.eq_ignore_ascii_case(surface) => Some(Site {
                    table,
                    material: r.material.clone(),
                    image: r.image.clone(),
                }),
                _ => None,
            })
            .unwrap_or_else(|| Self::playfield(vpx, table))
    }
}

/// Converts a light from the file.
///
/// **Including the ones that start out off.** A table's lamps are almost all
/// off in the file — they are the game's lamps, and the game turns them on —
/// so dropping them here leaves a port with nothing to light up and a playfield
/// that stays dark however well the rest works. What is dropped is a light with
/// no intensity at all, which is a light that could never do anything, and a
/// light the author hid.
pub fn build(l: &vpin::vpx::gameitem::light::Light, surface_z: f32, site: &Site) -> Option<Light> {
    // `VSBL`. The original gates the whole lightmap on it (`light.cpp:700`) and
    // returns before drawing anything at all in the editor pass
    // (`light.cpp:562`), so a hidden light contributes nowhere. Tables use this
    // for lamps that exist only for the script to hold state on, and for GI
    // lights whose halo the author did not want; drawing them puts coloured
    // discs on the playfield that no real game has. Absent means shown: the
    // chunk is from 10.8 and the constructor's default is true
    // (`light.h:94`).
    if !l.visible.unwrap_or(true) {
        return None;
    }

    // `state` is from 10.8; old tables carry `state_u32`. Reading the float
    // alone and falling back to zero turns every lamp a pre-10.8 file declares
    // as lit off at load.
    let raw = l.state.unwrap_or(l.state_u32 as f32);
    let blinking = raw >= BLINKING;
    // A blinking lamp is carried at full brightness with the flag beside it:
    // the pattern decides what it actually shows, and one is where the pattern
    // starts. Showing it off would be just as arbitrary and looks worse.
    let state = if blinking { 1.0 } else { raw };
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

    // Where the outline sits. A bulb's halo **floats**: the original adds
    // `m_bulbHaloHeight` before the 0.1 that keeps it from fighting the surface
    // over depth (`light.cpp:490-494`, `:515`). A GI light with a halo height
    // of 28 whose mesh is left on the playfield lies flat under every ramp and
    // plastic that crosses it, and is occluded by them, instead of hanging
    // above the lot — which is the whole reason the field exists.
    let halo_z = surface_z
        + if l.is_bulb_light {
            l.bulb_halo_height
        } else {
            0.0
        }
        + 0.1;

    // And where the *falloff* is centred, which is a different height again:
    // `GetCurrentHeight()` is `m_initSurfaceHeight + m_d.m_height`
    // (`light.h:143`), fed to the shader as `center_range.z`
    // (`light.cpp:758`). `HGHT` lifts the source of the light without moving
    // the disc it is painted on, so a lamp raised above the playfield spreads
    // its falloff wider — pinning it to the surface makes every raised lamp a
    // tight spot instead of a wash.
    let center_z = surface_z + l.height.unwrap_or(0.0);

    // The insert's picture. A bulb has none whatever the file says
    // (`light.cpp:708`), and a classic light with no name has none either; the
    // rest sample a picture of the whole playfield at the point's own place on
    // it (`light.cpp:519-520`):
    //
    //     buf[t].tu = pv0->x * inv_tablewidth;
    //     buf[t].tv = pv0->y * inv_tableheight;
    //
    // Divided by the extent and **not** offset by the table's origin — the
    // original does not subtract `m_left`/`m_top`, and every table's artwork
    // was authored against that mapping. A table whose origin is not zero is
    // vanishingly rare, but were one to exist, "correcting" this would shift
    // its inserts off their windows.
    let image = if l.is_bulb_light {
        String::new()
    } else {
        l.image.clone()
    };
    let uvs = if image.is_empty() {
        Vec::new()
    } else {
        let inv_w = 1.0 / (site.table.max.x - site.table.min.x);
        let inv_h = 1.0 / (site.table.max.y - site.table.min.y);
        flat.iter().map(|p| [p.x * inv_w, p.y * inv_h]).collect()
    };

    Some(Light {
        name: l.name.clone(),
        vertices: flat.iter().map(|p| [p.x, p.y, halo_z]).collect(),
        indices,
        uvs,
        image,
        image_mode: l.is_image_mode,
        surface_material: site.material.clone(),
        surface_image: site.image.clone(),
        center: Vec3::new(l.center.x, l.center.y, center_z),
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
        blinking,
        is_bulb: l.is_bulb_light,
        // Clamped at zero the way `put_TransmissionScale` clamps it
        // (`light.cpp:1322`); a negative one would subtract light from the ball.
        transmission_scale: l.transmission_scale.max(0.0),
        // A classic light has no bulb blend at all, and the original clamps a
        // bulb's away from both ends: zero disables the blend outright and one
        // "looks not good with day-night changes" (`light.cpp:830`).
        modulate: if l.is_bulb_light {
            l.bulb_modulate_vs_add.clamp(0.0001, 0.9999)
        } else {
            0.0
        },
        fader: l.fader.as_ref().map_or(Fader::default(), Fader::from),
        fade_up: l.fade_speed_up,
        fade_down: l.fade_speed_down,
        // The original replaces an empty pattern with a single off frame
        // (`light.cpp:1258-1259`) — and it has to, because `UpdateAnimation`
        // indexes the string unconditionally.
        blink: if l.blink_pattern.is_empty() {
            vec![false]
        } else {
            l.blink_pattern.chars().map(|c| c == '1').collect()
        },
        // `Load` seeds the interval at 125 ms *before* reading `BINT`
        // (`light.cpp:926`), so a file without the chunk blinks eight times a
        // second rather than once per frame, which is what a zero would give.
        blink_interval: if l.blink_interval == 0 {
            125.0
        } else {
            l.blink_interval as f32
        },
    })
}

impl Light {
    /// Whether the light is drawn while it is off.
    ///
    /// The original leaves `Light::Render` at zero intensity only for a bulb
    /// and for a classic light whose picture *is* its surface's picture and
    /// which is not in image mode (`light.cpp:713-718`) — "assumes/requires
    /// that the light in this kind of state is basically -exactly- the same as
    /// the static/(un)lit playfield". Everything else is drawn dark: a classic
    /// light with a picture of its own is drawn in that picture, lit by the
    /// scene but not by itself, because the playfield underneath does not show
    /// it and the table was authored counting on it being there.
    ///
    /// Skipping those the way the halo-only ones are skipped makes every such
    /// insert *vanish* when the game turns it off, artwork included, instead
    /// of going dark.
    pub fn drawn_when_off(&self) -> bool {
        !self.is_bulb
            && !self.image.is_empty()
            && (self.image_mode || !self.image.eq_ignore_ascii_case(&self.surface_image))
    }
}

/// A lamp's colour, the way the original converts one: a divide by 255 and no
/// gamma decode (`convertColor`, `utils/color.h:22`, fed to a light at
/// `light.cpp:711-712`). See `geometry::color` for why the asymmetry with
/// textures is deliberate.
fn color(c: &vpin::vpx::color::Color) -> [f32; 3] {
    [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
}

// ---------------------------------------------------------------------------
// The running lamp
// ---------------------------------------------------------------------------

/// What a light is doing right now, as opposed to what the file says it is.
///
/// One per light, owned by whoever draws them. `Light::UpdateAnimation`
/// (`light.cpp:299-357`) in one object: the blink pattern's cursor, the level
/// the lamp is actually showing, and — for [`Fader::Incandescent`] — the
/// temperature of its filament.
#[derive(Debug, Clone)]
pub struct Lamp {
    fader: Fader,
    fade_up: f32,
    fade_down: f32,
    /// `m_d.m_intensity`: the lamp at full power, before state and scale.
    intensity: f32,
    blink: Vec<bool>,
    blink_interval: f32,
    /// `m_inPlayState`: 0 off, 1 on, [`BLINKING`] for the pattern, or any
    /// level in between from a 10.8 table.
    in_play: f32,
    /// `m_currentIntensity`: what is on the screen.
    current: f32,
    /// `m_currentFilamentTemperature`, in kelvin. Room temperature is dark.
    temperature: f64,
    /// `m_iblinkframe`.
    frame: usize,
    /// The lamp's own clock. The original reads the player's `m_time_msec`;
    /// here the caller hands over how much table time each frame was worth and
    /// this accumulates it, which asks less of the caller and answers the only
    /// question the blinker has — how long since the last frame of the pattern.
    now_ms: f32,
    /// `m_timenextblink`.
    next_blink_ms: f32,
}

impl Lamp {
    /// A lamp showing exactly what the file says it shows.
    pub fn new(l: &Light) -> Self {
        let in_play = if l.blinking { BLINKING } else { l.state };
        let mut lamp = Lamp {
            fader: l.fader,
            fade_up: l.fade_up,
            fade_down: l.fade_down,
            intensity: l.intensity,
            blink: l.blink.clone(),
            blink_interval: l.blink_interval,
            in_play,
            current: 0.0,
            temperature: ROOM_T,
            frame: 0,
            now_ms: 0.0,
            // `RenderSetup` starts the pattern one interval out
            // (`light.cpp:381` into `RestartBlinker`, `light.h:323`).
            next_blink_ms: l.blink_interval,
        };
        // Not faded in from black: the file's state is where the table starts,
        // and a lamp the author lit should be lit on the first frame rather
        // than ramping up out of nothing while the player watches.
        lamp.snap(in_play, 1.0);
        lamp
    }

    /// The level to draw, in the same units as [`Light::intensity`].
    ///
    /// `m_currentIntensity`: what the lamp is showing *now*, which is not the
    /// `intensity` the file gives it — that one is the lamp at full power and
    /// is only where a fade is heading.
    pub fn level(&self) -> f32 {
        self.current
    }

    /// What to multiply the lamp's two colours by.
    ///
    /// One for every fader but [`Fader::Incandescent`], which tints by the
    /// filament's temperature relative to a 2700 K reference
    /// (`light.cpp:723-735`). That ratio is what turns a dying bulb orange:
    /// the emission alone would only make it dimmer.
    pub fn tint(&self) -> [f32; 3] {
        if self.fader != Fader::Incandescent {
            return [1.0; 3];
        }
        let now = filament_tint(self.temperature as f32);
        let reference = filament_tint(2700.0);
        [
            now[0] / reference[0],
            now[1] / reference[1],
            now[2] / reference[2],
        ]
    }

    /// The level the lamp is heading for: `light.cpp:315-316`.
    fn target(&self, scale: f32) -> f32 {
        let state = if self.in_play == BLINKING {
            f32::from(self.blink[self.frame.min(self.blink.len() - 1)])
        } else {
            self.in_play
        };
        self.intensity * scale * state
    }

    /// Puts the lamp where it was asked to be with no fade at all, the way
    /// [`Fader::None`] would.
    ///
    /// For a caller with no clock. It is not the same as an `update` with a
    /// large step: the filament model would still take the scenic route.
    pub fn snap(&mut self, state: f32, scale: f32) -> bool {
        self.set_state(state);
        let target = self.target(scale);
        let changed = self.current != target;
        self.current = target;
        // Kept consistent so a later `update` does not have to fade from
        // whatever temperature the filament happened to be left at.
        self.temperature = if self.intensity * scale > 0.0 {
            emission_to_temperature(target / (self.intensity * scale))
        } else {
            ROOM_T
        };
        changed
    }

    /// `setInPlayState`, `light.cpp:1533`: changing to blinking restarts the
    /// pattern from its first frame, right away.
    fn set_state(&mut self, state: f32) {
        if state == self.in_play {
            return;
        }
        self.in_play = state;
        if state == BLINKING {
            self.frame = 0;
            self.next_blink_ms = self.now_ms;
        }
    }

    /// One frame of `Light::UpdateAnimation` (`light.cpp:299-357`).
    ///
    /// `state` is the original's `m_inPlayState` — 0 off, 1 on, [`BLINKING`],
    /// or a level in between — and `scale` is `IntensityScale`, the dimmer a
    /// script writes every frame while it fades a lamp by hand. `dt_ms` is how
    /// much *table* time this frame was worth, not wall clock: a fade that
    /// keeps running while the physics is paused would drift away from the
    /// game it belongs to.
    ///
    /// Answers whether anything the renderer cares about moved.
    pub fn update(&mut self, state: f32, scale: f32, dt_ms: f32) -> bool {
        self.now_ms += dt_ms;
        self.set_state(state);

        // `UpdateBlinker`, `light.h:311`. One frame of the pattern per call and
        // not a catch-up loop: that is the original's, and it means a pattern
        // whose interval is shorter than a frame runs at the frame rate rather
        // than skipping characters — the fast attract-mode blink of a table
        // like The Lord of the Rings stays a blink instead of becoming a blur.
        if self.in_play == BLINKING && self.next_blink_ms <= self.now_ms {
            self.frame += 1;
            if self.frame >= self.blink.len() {
                self.frame = 0;
            }
            self.next_blink_ms += self.blink_interval;
        }

        let target = self.target(scale);
        let was = (self.current, self.temperature);
        if self.current != target {
            match self.fader {
                Fader::None => self.current = target,
                Fader::Linear => self.linear(target, dt_ms),
                Fader::Incandescent => self.incandescent(target, scale, dt_ms),
            }
        }
        (self.current, self.temperature) != was
    }

    /// `light.cpp:322-334`: a straight ramp, clamped so it cannot overshoot.
    fn linear(&mut self, target: f32, dt_ms: f32) {
        let speed = if self.current < target {
            self.fade_up
        } else {
            self.fade_down
        };
        // The original does not guard this and does not have to: its own
        // default is 0.2 per millisecond. A file that stores zero — or one of
        // the infinities `FASP` is known to carry — would leave the lamp
        // frozen at whatever it was showing for the rest of the game, so a
        // speed that cannot make progress is treated as no fader at all.
        if !speed.is_finite() || speed <= 0.0 {
            self.current = target;
            return;
        }
        if self.current < target {
            self.current = (self.current + speed * dt_ms).min(target);
        } else {
            self.current = (self.current - speed * dt_ms).max(target);
        }
    }

    /// `light.cpp:336-353`: a #44 bulb's filament, heated and left to cool.
    fn incandescent(&mut self, target: f32, scale: f32, dt_ms: f32) {
        let full = self.intensity * scale;
        // The original's own guard: with no power on offer there is no model to
        // run and the lamp keeps whatever it was showing (`light.cpp:338`).
        if full == 0.0 {
            return;
        }
        let state = if full > 0.0 { target / full } else { 0.0 };
        let speed = if self.current < target {
            self.fade_up
        } else {
            self.fade_down
        };
        // Same reasoning as `linear`, one step removed: this is `1.0 / (fade
        // speed in ms)` in the original's words, and a zero here stalls the
        // filament rather than the ramp.
        if !speed.is_finite() || speed <= 0.0 {
            self.current = target;
            self.temperature = emission_to_temperature(state);
            return;
        }
        let inv_fade_speed = speed / full;
        // "a bulb with this characteristics reaches full power between 30 and
        // 40ms so we modulate around this", clamped at 500 ms because past that
        // the filament has settled anyway and the integration loses precision.
        let remaining = (dt_ms * 0.001 * 40.0 * inv_fade_speed).min(0.5);
        self.temperature = if state != 0.0 {
            // 6.3 V is the #44's rating, modulated by the fourth root of the
            // asked-for emission: "not fully correct (ignoring visible/non
            // visible wavelengths) but an acceptable approximation".
            let u = 6.3 * state.sqrt().sqrt();
            heat_up(self.temperature, remaining, u)
        } else {
            cool_down(self.temperature, remaining)
        };
        self.current = filament_emission(self.temperature) * full;
    }
}

// ---------------------------------------------------------------------------
// The filament
// ---------------------------------------------------------------------------
//
// A port of `utils/bulb.cpp` for the one bulb the lights use, `BULB_44`
// (`light.cpp:345`). It is a physical model and not a curve fit: the filament
// has a mass and a surface, electric power heats it, Stefan-Boltzmann radiation
// cools it, and the light that comes out is the visible part of a black body at
// whatever temperature the thing has reached. That is why an incandescent lamp
// fades the way it does — fast up, slow down, and orange on the way out — and
// none of it falls out of a linear ramp.

/// Room temperature, and the floor of the model (`bulb.cpp:126`).
const ROOM_T: f64 = 293.0;
/// The top of the lookup tables — past this a tungsten filament melts
/// (`bulb.h:16`).
const BULB_T_MAX: usize = 3400;
/// The #44: 6.3 V, 250 mA, stable at 2710 K, with the filament surface (m²) and
/// mass (kg) the original fits to those ratings (`bulb.cpp:82`).
const RATING_U: f64 = 6.3;
const RATING_I: f64 = 0.250;
const RATING_T: usize = 2710;
const FILAMENT_SURFACE: f64 = 2.219_161_565_4e-6;
const FILAMENT_MASS: f64 = 0.311_786_646_6e-6;

/// Linear RGB tint of a black body from 1500 K to 3000 K in steps of 100,
/// normalised to a relative luminance of one (`bulb.cpp:92`).
const TINT: [[f32; 3]; 16] = [
    [3.253_114, 0.431_191, 0.000_001],
    [3.074_21, 0.484_372, 0.000_001],
    [2.914_679, 0.531_794, 0.000_001],
    [2.769_808, 0.574_859, 0.000_001],
    [2.643_605, 0.612_374, 0.000_001],
    [2.523_686, 0.645_953, 0.020_487],
    [2.414_433, 0.676_211, 0.042_456],
    [2.316_033, 0.703_137, 0.065_485],
    [2.225_598, 0.727_599, 0.089_456],
    [2.144_543, 0.749_200, 0.114_156],
    [2.070_389, 0.768_694, 0.139_412],
    [1.997_974, 0.787_618, 0.165_180],
    [1.935_725, 0.803_465, 0.191_508],
    [1.876_871, 0.818_242, 0.218_429],
    [1.821_461, 0.832_006, 0.245_241],
    [1.772_554, 0.843_853, 0.271_900],
];

/// The tables `bulb_init` precomputes (`bulb.cpp:120`), for the #44 alone.
struct Filament {
    /// Visible emission, in lumen per steradian per m², for 1500 K upward.
    emission: Vec<f64>,
    /// The inverse, quantised the way the original quantises it: 512 buckets
    /// over twice the emission at 2700 K.
    temperature: Vec<f64>,
    /// Kelvin per second of cooling, and per second per volt squared of
    /// heating, at each whole kelvin.
    cool: Vec<f64>,
    heat: Vec<f64>,
}

/// Resistance of the filament at `t`, from its resistance at room temperature.
///
/// `BULB_R`, `bulb.cpp:70`. The 2024 rewrite dropped the `(T/T0)^1.215` model
/// for this straight line because it is the one the bulb characteristics above
/// were fitted with.
fn resistance(r0: f64, t: f64) -> f64 {
    r0 * (1.0 + 0.0045 * (t - ROOM_T))
}

fn filament() -> &'static Filament {
    static FILAMENT: OnceLock<Filament> = OnceLock::new();
    FILAMENT.get_or_init(|| {
        // Resistance at room temperature, back-computed from the U/I/T ratings.
        let r0 = (RATING_U / RATING_I) / resistance(1.0, RATING_T as f64);

        // Filament temperature to visible emission, by Coblentz and Emerson's
        // formula, converted from W.sr-1.cm-2 to lumen.sr-1.m-2.
        let emission: Vec<f64> = (0..=BULB_T_MAX - 1500)
            .map(|i| {
                let t = 1500.0 + i as f64;
                let p = 1.247 / (1.0 + 129.05 / t).powf(204.0)
                    + 0.0678 / (1.0 + 78.85 / t).powf(404.0)
                    + 0.0489 / (1.0 + 23.52 / t).powf(1004.0)
                    + 0.0406 / (1.0 + 13.67 / t).powf(2004.0);
                p * 68493.150685
            })
            .collect();

        // And back again. The original walks the table rather than inverting
        // the formula, and so does this.
        let p2700 = emission[2700 - 1500];
        let mut pos = 0;
        let temperature: Vec<f64> = (0..512)
            .map(|i| {
                let p = f64::from(i) * (p2700 / 255.0);
                while pos + 1 < emission.len() && emission[pos] < p {
                    pos += 1;
                }
                1500.0 + pos as f64
            })
            .collect();

        let (mut cool, mut heat) = (Vec::new(), Vec::new());
        for i in 0..=BULB_T_MAX {
            let t = i as f64;
            // Tungsten's specific heat, from Agrawal's "Heating-times of
            // tungsten filament incandescent lamps": 45.2268 J.kg-1.K-1 is the
            // gas constant for tungsten, 310 K its Debye temperature.
            let specific_heat = 3.0 * 45.2268 * (1.0 - (310.0 * 310.0) / (20.0 * t * t))
                + (2.0 * 4.5549e-3 * t)
                + (4.0 * 5.77874e-10 * t * t * t);
            // Emissivity over all wavelengths, cut by the coil factor because
            // part of what a coiled filament radiates lands back on itself.
            let emissivity = 0.6865 * 0.0000689 * t.powf(1.0748);
            // Radiated power by Planck's law...
            let mut c =
                -5.670_374_419e-8 * FILAMENT_SURFACE * emissivity * (t.powi(4) - ROOM_T.powi(4));
            // ...plus what the base and wires carry away by convection.
            c += -0.07 * ((t - ROOM_T) / RATING_T as f64) / resistance(r0, t);
            // Electric power is U²/R; the U² is left out so it can be
            // modulated per call.
            let h = 1.0 / resistance(r0, t);
            cool.push(c / (FILAMENT_MASS * specific_heat));
            heat.push(h / (FILAMENT_MASS * specific_heat));
        }

        Filament {
            emission,
            temperature,
            cool,
            heat,
        }
    })
}

/// Visible emission at `t`, relative to the emission at the bulb's rated
/// temperature — so one is a lamp at full power (`bulb.cpp:185`).
///
/// A departure, and a small one: at exactly `BULB_T_MAX` the original hands
/// back the raw lumen figure instead of the ratio, which is some fifty thousand
/// times too large. It is unreachable at 6.3 V — the filament settles near
/// 2710 K — but it is a cliff edge, and normalising it costs nothing.
fn filament_emission(t: f64) -> f32 {
    if t < 1500.0 {
        return 0.0;
    }
    let f = filament();
    let i = (t as usize).min(BULB_T_MAX) - 1500;
    (f.emission[i] / f.emission[RATING_T - 1500]) as f32
}

/// The temperature a filament would have to be at to emit `p` (`bulb.cpp:230`).
fn emission_to_temperature(p: f32) -> f64 {
    let f = filament();
    let v = (p * 255.0) as isize;
    f.temperature[v.clamp(0, 511) as usize]
}

/// Linear RGB tint of the filament at `t` (`bulb.cpp:198`).
fn filament_tint(t: f32) -> [f32; 3] {
    if t < 1500.0 {
        return TINT[0];
    }
    if t >= 2999.0 {
        return TINT[15];
    }
    let scaled = (t - 1500.0) / 100.0;
    let lower = scaled as usize;
    let alpha = scaled - lower as f32;
    let (a, b) = (TINT[lower], TINT[lower + 1]);
    [
        (1.0 - alpha) * a[0] + alpha * b[0],
        (1.0 - alpha) * a[1] + alpha * b[1],
        (1.0 - alpha) * a[2] + alpha * b[2],
    ]
}

/// The filament under `u` volts for `duration` **seconds** (`bulb.cpp:265`).
///
/// The serial resistance the original takes is always zero from a light
/// (`light.cpp:345` passes `0.0f`), so it is left out rather than carried
/// unused.
fn heat_up(mut t: f64, mut duration: f32, u: f32) -> f64 {
    let f = filament();
    while duration > 0.0 {
        // Kept inside the tables: below room temperature and above the melting
        // point there is nothing to look up.
        t = t.clamp(ROOM_T, BULB_T_MAX as f64);
        let i = t as usize;
        let energy = f64::from(u) * f64::from(u) * f.heat[i] + f.cool[i];
        // Electric heating and radiated cooling have met: the filament is not
        // going anywhere, however long is left.
        if -10.0 < energy && energy < 10.0 {
            return t;
        }
        // Half a millisecond through the initial current surge, because the
        // resistance climbs fast enough with temperature to change the answer
        // within one; a millisecond after that.
        let dt = duration.min(if energy > 1000e3 { 0.0005 } else { 0.001 });
        t += f64::from(dt) * energy;
        let before = duration;
        duration -= dt;
        // The original asserts here instead. A step too small to subtract is a
        // loop that never ends, and a lamp is not worth hanging the frame over.
        if duration == before {
            break;
        }
    }
    t
}

/// The filament with the power off, for `duration` seconds (`bulb.cpp:246`).
fn cool_down(mut t: f64, mut duration: f32) -> f64 {
    let f = filament();
    while duration > 0.0 {
        let dt = duration.min(0.001);
        t += f64::from(dt) * f.cool[(t as usize).min(BULB_T_MAX)];
        if t <= 294.0 {
            return ROOM_T;
        }
        let before = duration;
        duration -= dt;
        if duration == before {
            break;
        }
    }
    t
}
