//! The artwork on the head's face, painted from the table's own colours.
//!
//! A `.vpx` has no backglass in it — see [`crate::backbox`] for why — so the
//! head has always been a blank white panel: honest, and the least convincing
//! thing on screen. A real machine's head is the loudest part of it, and it is
//! loud in a particular way: a sheet of translucent artwork with fluorescent
//! tubes behind it, so the picture does not *reflect* the room, it **glows**,
//! and the tubes show through it as broad soft pools that wash the colour out
//! towards white where they are brightest.
//!
//! That is what is built here, in four steps:
//!
//! 1. **A palette from the table itself.** Every machine has its own colours,
//!    and they are already in the file — in the playfield art, which is the
//!    one image a table always has and the one its designer chose the whole
//!    machine's look from. The dominant vivid hues are taken from it, so
//!    F-14's head comes out its reds and blues and South Park's comes out
//!    orange, without either of them being named anywhere.
//! 2. **Abstract cloud, in those colours.** Domain-warped value noise — the
//!    field is fed its own noise as a coordinate, twice — which is what turns
//!    plain fog into the stretched, folded shapes of a nebula. Nothing
//!    representational: a real backglass has a painting on it, and inventing
//!    one for a machine we cannot see would be worse than not trying.
//! 3. **The tubes behind it.** A handful of broad Gaussians, added rather than
//!    multiplied where they overpower the pigment, which is what gives the
//!    washed-out hot spot that says "lit from behind" instead of "painted
//!    bright".
//! 4. **A frame.** Grey metal down the sides and across the ends, because the
//!    glass in a real head is *held* by something, and an edge is what stops a
//!    picture from looking like a texture that ran out.
//!
//! The whole thing is a few hundred thousand pixels of arithmetic and no I/O,
//! and it is baked once, when the table is opened.

/// The name the head's face is textured with.
///
/// Reserved, like [`crate::backbox::DISPLAY_IMAGE`]: nothing in a `.vpx` is
/// called this, because nothing in a `.vpx` knows the head exists.
pub const BACKGLASS_IMAGE: &str = "vpw:backglass";

/// How big the painted sheet is, in pixels.
///
/// Five by four, which is both the head's own shape ([`crate::backbox`]'s
/// `HEIGHT_RATIO`) and the shape of every backglass image a table carries —
/// F-14's is 1280x1024. Half of that in each direction: what is on it is
/// cloud and light, the two things in the world with no fine detail, and the
/// only sharp edges are the frame and the score window, which are drawn
/// analytically at this resolution rather than sampled at any other.
pub const BACKGLASS_PIXELS: (u32, u32) = (640, 512);

/// How big the cloud is worked out before it is stretched over the sheet.
///
/// Half the picture in each direction. The cloud is smooth by construction
/// and smooth again by argument: the light behind a real backglass passes
/// through a diffuser, and a diffuser is precisely a thing that destroys
/// detail. So the expensive part — five noise fields per pixel, each five
/// octaves deep — is paid a quarter as often, and what the interpolation
/// loses was going to be lost anyway.
///
/// Half and not an eighth, which was the first try: the field's own finest
/// octave has a period of about two of its texels, so a coarser grid does not
/// blur the cloud, it *undersamples* it — and an undersampled smooth field
/// comes out as a quilt of flat squares, which is exactly what it looked
/// like.
const FIELD: (usize, usize) = (320, 256);

/// Where the tubes are, as fractions of the face: x, y, radius, brightness.
///
/// Behind the artwork and out towards the edges, the way they are fitted in a
/// real head — a tube runs up each side and along the bottom, and the middle
/// of the glass is lit by what spills inwards rather than by a lamp of its
/// own. That is also what keeps the score readable: the window sits in the
/// upper middle, and a lamp behind it would be shining through exactly the
/// part of the sheet that has to stay dark.
const TUBES: [[f32; 4]; 5] = [
    [0.08, 0.42, 0.44, 0.85],
    [0.92, 0.42, 0.44, 0.85],
    [0.30, 0.94, 0.42, 0.70],
    [0.70, 0.94, 0.42, 0.70],
    [0.50, 0.68, 0.60, 0.45],
];

/// What the sheet transmits where no tube reaches it.
///
/// Not zero: a backglass in a lit room is visible when the machine is off.
const AMBIENT: f32 = 0.34;

/// How wide the metal is down the sides, as a fraction of the width.
const SIDE_FRAME: f32 = 0.030;
/// And across the top and bottom, as a fraction of the height. Narrower, so
/// the frame reads as an upright rather than as a box.
const END_FRAME: f32 = 0.022;

/// A handful of colours, taken from a table's own art.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Vivid, and ordered by how much of the table is that colour.
    pub colours: Vec<[f32; 3]>,
}

/// How many hues the art is sorted into on the way to a palette.
const HUE_BINS: usize = 24;
/// How many of them a palette keeps.
const KEEP: usize = 4;
/// How far apart, in bins, two kept hues have to be.
///
/// Without this, a palette of "the four dominant colours" comes back as four
/// shades of one colour — which is what a photograph of anything mostly is.
const APART: usize = 3;

impl Palette {
    /// The colours of a piece of RGBA art.
    ///
    /// Pixels are weighted by `saturation² × value`, which is the whole trick:
    /// the *commonest* colour in a playfield photograph is a muddy mid-grey —
    /// wood, shadow, the average of everything — and a palette built by count
    /// alone comes back as four greys. What a machine is remembered by is its
    /// vivid colours, however little of it they cover, so vividness is what
    /// the vote is weighted by, and near-greys are not counted at all.
    ///
    /// `None` when the picture has nothing to say: a cut-out that is mostly
    /// transparent, a photograph of bare wood, a mask. The caller is expected
    /// to ask another picture rather than paint a head out of one grey.
    pub fn from_art(rgba: &[u8], width: u32, height: u32) -> Option<Self> {
        // At most this many pixels are looked at, spread evenly over the
        // picture by striding. A playfield texture is often four megapixels
        // and the answer stops moving long before that.
        const SAMPLES: usize = 24_000;
        let pixels = rgba.as_chunks::<4>().0;
        let total = (width as usize)
            .saturating_mul(height as usize)
            .min(pixels.len());
        if total == 0 {
            return None;
        }
        let stride = (total / SAMPLES).max(1);

        let mut weight = [0.0f32; HUE_BINS];
        let mut sum = [[0.0f32; 3]; HUE_BINS];
        for px in pixels.iter().step_by(stride).take(SAMPLES) {
            let c = [
                px[0] as f32 / 255.0,
                px[1] as f32 / 255.0,
                px[2] as f32 / 255.0,
            ];
            // What is not there is not a colour either. Half the textures in
            // a table are cut-outs — a plastic on a transparent background —
            // and the transparent part is stored as black, so counting it
            // hands back a picture of nothing at all. This is why the first
            // attempt painted every machine the house colours: the largest
            // image in both test tables is a cut-out.
            if px[3] < 128 {
                continue;
            }
            let (hue, sat, val) = to_hsv(c);
            // Ink and highlight: black is not a colour the machine chose, and
            // white is what its lamps do rather than what it is painted.
            if val < 0.14 || sat < 0.18 {
                continue;
            }
            let w = sat * sat * val;
            let bin = ((hue * HUE_BINS as f32) as usize).min(HUE_BINS - 1);
            weight[bin] += w;
            for k in 0..3 {
                sum[bin][k] += c[k] * w;
            }
        }

        let mut order: Vec<usize> = (0..HUE_BINS).filter(|&b| weight[b] > 0.0).collect();
        order.sort_by(|&a, &b| weight[b].total_cmp(&weight[a]));

        let mut kept: Vec<usize> = Vec::with_capacity(KEEP);
        for bin in order {
            let clash = kept.iter().any(|&k| {
                let d = k.abs_diff(bin);
                d.min(HUE_BINS - d) < APART
            });
            if !clash {
                kept.push(bin);
            }
            if kept.len() == KEEP {
                break;
            }
        }
        if kept.len() < 2 {
            return None;
        }

        let colours = kept
            .iter()
            .map(|&b| {
                let avg = [
                    sum[b][0] / weight[b],
                    sum[b][1] / weight[b],
                    sum[b][2] / weight[b],
                ];
                vivid(avg)
            })
            .collect();
        Some(Self { colours })
    }

    /// For a table whose art says nothing — no playfield texture, or one that
    /// is a photograph of plywood.
    ///
    /// The house colours, which are the decade the machine came from.
    pub fn fallback() -> Self {
        Self {
            colours: vec![
                [0.98, 0.16, 0.58],
                [0.16, 0.85, 0.95],
                [0.55, 0.25, 0.92],
                [0.99, 0.68, 0.15],
            ],
        }
    }
}

/// Pushes a colour towards the one it would be on a backglass.
///
/// Artwork on a lit sheet is printed in ink chosen to survive being shone
/// through, so it is more saturated and more even in brightness than the same
/// colour photographed on a playfield under a shadow. The hue — the part that
/// actually identifies the machine — is kept exactly.
fn vivid(c: [f32; 3]) -> [f32; 3] {
    let (hue, sat, _) = to_hsv(c);
    from_hsv(hue, sat.clamp(0.50, 0.80), 0.78)
}

fn to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let span = max - min;
    let hue = if span <= f32::EPSILON {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / span).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / span + 2.0
    } else {
        (c[0] - c[1]) / span + 4.0
    } / 6.0;
    let sat = if max <= f32::EPSILON { 0.0 } else { span / max };
    (hue.rem_euclid(1.0), sat, max)
}

fn from_hsv(hue: f32, sat: f32, val: f32) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let c = val * sat;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = val - c;
    let [r, g, b] = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    [r + m, g + m, b + m]
}

// --- the noise ------------------------------------------------------------

/// One deterministic float per lattice point.
///
/// No table and no state: the same table painted on two machines has to come
/// out the same picture, and a hash of the coordinates is the cheapest way to
/// promise that.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x9e37_79b9) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

/// Value noise: the lattice, smoothly interpolated.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    // Smoothstep rather than a straight blend, which is what stops the
    // lattice from showing as a grid of creases.
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (ix as i32, iy as i32);
    let a = hash(ix, iy, seed);
    let b = hash(ix + 1, iy, seed);
    let c = hash(ix, iy + 1, seed);
    let d = hash(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Noise at five sizes at once, each half the last and half the strength.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for octave in 0..5 {
        sum += amp * noise(x * freq, y * freq, seed.wrapping_add(octave * 131));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// The cloud, at [`FIELD`] resolution.
///
/// Domain warping: the noise is not sampled at the pixel but at a position the
/// noise itself moved, twice over. One pass gives fog; two gives the stretched
/// filaments and dark voids that read as a nebula rather than as static.
/// Inigo Quilez's construction, with the palette dropped in where his has a
/// colour ramp.
fn nebula(palette: &Palette) -> Vec<[f32; 3]> {
    let (fw, fh) = FIELD;
    let mut out = vec![[0.0f32; 3]; fw * fh];
    for y in 0..fh {
        for x in 0..fw {
            // Under two cloud-widths across the sheet, and the same scale
            // down it: the cloud must not know the picture is not square.
            // Small numbers on purpose — a fine cloud on a panel this size is
            // marbled paper, and what is wanted is weather.
            let u = (x as f32 + 0.5) / fw as f32 * 2.2;
            let v = (y as f32 + 0.5) / fh as f32 * 1.8;

            const WARP: f32 = 2.6;
            let q = [fbm(u, v, 1), fbm(u + 5.2, v + 1.3, 2)];
            let r = [
                fbm(u + WARP * q[0] + 1.7, v + WARP * q[1] + 9.2, 3),
                fbm(u + WARP * q[0] + 8.3, v + WARP * q[1] + 2.8, 4),
            ];
            let n = fbm(u + WARP * r[0], v + WARP * r[1], 5);

            // Which colour, and how much of it there is. The warp's own
            // length is the structure — it is large where the field has been
            // folded hardest, which is where a cloud is bright.
            // How much cloud there is here: the warp's own length, which is
            // large where the field has been folded hardest.
            let mut density = smoothstep(0.22, 0.92, ((r[0] + r[1]) * 0.5).clamp(0.0, 1.0));
            // And an envelope over the whole sheet, at a size nothing else
            // here works at. Without it the cloud is the same weight in every
            // square inch, which is the giveaway that a picture was generated
            // rather than composed: what a painting has and noise does not is
            // somewhere to look, and somewhere quiet beside it.
            density *= 0.45 + 0.70 * smoothstep(0.28, 0.80, fbm(u * 0.55, v * 0.55, 7));
            // Colour follows density along one ramp rather than being a
            // second field of its own. That one decision is the difference
            // between a nebula and a tie-dye: when the hue is chosen
            // independently of the brightness, two colours meet at equal
            // brightness and the join is a hard edge — marbled oil, which is
            // what the first attempt looked like. Tied to density, every
            // colour boundary is also a brightness boundary, which is what
            // the eye reads as depth. The other field is left to nudge the
            // ramp, so the same density is not always the same colour.
            let t = (density * (0.80 + 0.40 * n)).clamp(0.0, 1.0);
            out[y * fw + x] = ramp(palette, t);
        }
    }
    out
}

/// How far the cloud is smeared before it is stretched over the sheet, in
/// field texels.
///
/// The sheet is a print behind glass with light coming through it from
/// behind, and neither of those is sharp: the light passes a diffuser and the
/// ink sits on the far side of a few millimetres of acrylic. The noise that
/// makes the cloud has no idea about that — it is as crisp at its finest
/// octave as at its coarsest — so it is softened here, once, before anything
/// samples it.
const SMEAR: usize = 2;

/// Softens the cloud.
///
/// Two box passes, which is close enough to a Gaussian that nothing about a
/// nebula could tell the difference, and separable, so it costs a row and a
/// column per texel rather than the square of the radius.
fn smear(field: &mut [[f32; 3]], (w, h): (usize, usize), radius: usize) {
    if radius == 0 {
        return;
    }
    let mut tmp = vec![[0.0f32; 3]; field.len()];
    for _ in 0..2 {
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f32; 3];
                let mut n = 0.0;
                for d in x.saturating_sub(radius)..(x + radius + 1).min(w) {
                    for k in 0..3 {
                        sum[k] += field[y * w + d][k];
                    }
                    n += 1.0;
                }
                tmp[y * w + x] = [sum[0] / n, sum[1] / n, sum[2] / n];
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f32; 3];
                let mut n = 0.0;
                for d in y.saturating_sub(radius)..(y + radius + 1).min(h) {
                    for k in 0..3 {
                        sum[k] += tmp[d * w + x][k];
                    }
                    n += 1.0;
                }
                field[y * w + x] = [sum[0] / n, sum[1] / n, sum[2] / n];
            }
        }
    }
}

/// The colour of the cloud at a given density: void, body, core.
///
/// Four stops, blended smoothly. The dark end is the table's own first colour
/// taken right down rather than black — painting the thin parts of a nebula
/// black gives soot and a picture with a hole in it, and a deep version of
/// the sheet's own colour gives depth, which is what an airbrushed backglass
/// actually does. The bright end goes *past* the palette towards white, the
/// way the middle of anything glowing does.
fn ramp(palette: &Palette, t: f32) -> [f32; 3] {
    let n = palette.colours.len();
    let stops: [(f32, [f32; 3]); 4] = [
        (0.00, shade(palette.colours[0], 0.10)),
        (0.34, shade(palette.colours[0], 0.62)),
        (0.66, shade(palette.colours[1 % n], 0.88)),
        (1.00, lighten(palette.colours[2 % n], 0.30)),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let (t0, a) = pair[0];
        let (t1, b) = pair[1];
        if t <= t1 || t1 == 1.0 {
            let f = smoothstep(t0, t1, t);
            return [
                a[0] + (b[0] - a[0]) * f,
                a[1] + (b[1] - a[1]) * f,
                a[2] + (b[2] - a[2]) * f,
            ];
        }
    }
    stops[3].1
}

/// Towards white, by a fraction of what is left.
fn lighten(c: [f32; 3], amount: f32) -> [f32; 3] {
    [
        c[0] + (1.0 - c[0]) * amount,
        c[1] + (1.0 - c[1]) * amount,
        c[2] + (1.0 - c[2]) * amount,
    ]
}

/// A colour taken down to a fraction of itself, with a touch more of it left
/// in the blue: a dark that keeps a little cool in it reads as shadow, and
/// one that does not reads as dirt.
fn shade(c: [f32; 3], amount: f32) -> [f32; 3] {
    [c[0] * amount, c[1] * amount, c[2] * amount * 1.45]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Reads the cloud at a point on the sheet, smoothly.
///
/// Smoothstepped weights rather than plain bilinear: a straight blend between
/// lattice cells is continuous but its *slope* is not, and on a field this
/// stretched the slope breaks show up as faint diamonds.
fn sample(field: &[[f32; 3]], u: f32, v: f32) -> [f32; 3] {
    let (fw, fh) = FIELD;
    let x = (u * fw as f32 - 0.5).clamp(0.0, fw as f32 - 1.0);
    let y = (v * fh as f32 - 0.5).clamp(0.0, fh as f32 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(fw - 1), (y0 + 1).min(fh - 1));
    let fx = smoothstep(0.0, 1.0, x - x0 as f32);
    let fy = smoothstep(0.0, 1.0, y - y0 as f32);
    let mut c = [0.0; 3];
    for k in 0..3 {
        let top = field[y0 * fw + x0][k] + (field[y0 * fw + x1][k] - field[y0 * fw + x0][k]) * fx;
        let bot = field[y1 * fw + x0][k] + (field[y1 * fw + x1][k] - field[y1 * fw + x0][k]) * fx;
        c[k] = top + (bot - top) * fy;
    }
    c
}

// --- the sheet ------------------------------------------------------------

/// Paints the head's face: RGBA, [`BACKGLASS_PIXELS`] big, fully opaque.
pub fn paint(palette: &Palette) -> Vec<u8> {
    let (w, h) = (BACKGLASS_PIXELS.0 as usize, BACKGLASS_PIXELS.1 as usize);
    let mut field = nebula(palette);
    smear(&mut field, FIELD, SMEAR);
    let mut out = vec![255u8; w * h * 4];

    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;

            let mut c = sample(&field, u, v);
            c = lit(c, u, v);
            c = window(c, u, v);
            c = framed(c, u, v);

            let at = (y * w + x) * 4;
            for k in 0..3 {
                // A hair of noise, different per channel, against the banding
                // a gradient this smooth would otherwise show in eight bits.
                let dither = (hash(x as i32, y as i32, 90 + k as u32) - 0.5) / 255.0;
                out[at + k] = ((c[k] + dither).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// Shines the tubes through the sheet.
///
/// Two things happen to a translucent sheet with a lamp behind it, and doing
/// only the first is what makes a "backlit" texture look painted instead: the
/// artwork gets brighter, *and* once the lamp is stronger than the ink can
/// hold back, what comes through stops being the ink's colour and heads for
/// the lamp's own. That second term is the washed-out hot spot you can see on
/// any real machine, and it is why the tubes are visible as tubes.
fn lit(colour: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    // The face is wider than it is tall, so distances are measured on the
    // sheet rather than in texture coordinates: an oval pool of light is a
    // lamp with something wrong with it.
    const ASPECT: f32 = BACKGLASS_PIXELS.0 as f32 / BACKGLASS_PIXELS.1 as f32;
    let mut light = AMBIENT;
    for [tx, ty, radius, power] in TUBES {
        let (dx, dy) = ((u - tx) * ASPECT, v - ty);
        let d2 = dx * dx + dy * dy;
        light += power * (-2.4 * d2 / (radius * radius)).exp();
    }
    // The tube's own colour: the warm white of a fluorescent tube behind
    // colour print, which is a touch yellow.
    const TUBE: [f32; 3] = [1.0, 0.96, 0.88];
    let wash = (light - 1.0).max(0.0) * 0.30;
    let mut out = [0.0; 3];
    for k in 0..3 {
        out[k] = colour[k] * light + TUBE[k] * wash;
    }
    out
}

/// Darkens the sheet where the score window is set into it.
///
/// The display is a mesh of its own drawn a hair in front (see
/// [`crate::backbox::Backbox::display_mesh`]), so this is not what the score
/// is read against — it is the surround. A window cut into artwork has a dark
/// mask behind it and a trim around it, and without them the glowing panel
/// looks stuck onto the glass rather than set into it.
fn window(colour: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let [left, top, width, height] = crate::backbox::DISPLAY_AREA;
    // How far outside the display's own rectangle this pixel is, in fractions
    // of the face. Negative inside it.
    let out_x = (left - u).max(u - (left + width));
    let out_y = (top - v).max(v - (top + height));
    let outside = out_x.max(out_y);

    // A dark mask reaching a little past the panel, a bright trim around the
    // edge of that, and then the artwork.
    const MASK: f32 = 0.022;
    const TRIM: f32 = 0.010;
    let mask = 1.0 - smoothstep(MASK - 0.004, MASK + 0.004, outside);
    let trim = smoothstep(MASK - 0.002, MASK + 0.002, outside)
        * (1.0 - smoothstep(MASK + TRIM - 0.003, MASK + TRIM + 0.003, outside));

    let mut c = [0.0; 3];
    for k in 0..3 {
        // Not quite black: glass over a mask still catches something.
        let masked = colour[k] * 0.06 + 0.015;
        c[k] = colour[k] + (masked - colour[k]) * mask;
        c[k] += (0.62 - c[k]) * trim * 0.8;
    }
    c
}

/// Puts the glass in its frame.
///
/// Grey metal, and grey for a reason: the extrusion that holds a backglass is
/// anodised aluminium, and a coloured frame would fight the artwork it is
/// there to present. Wider down the sides than across the ends, which is how
/// a real one is made and what makes the head read as standing up rather than
/// lying down.
fn framed(colour: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    // Distance from the nearest edge, measured against that edge's own frame
    // width: 0 at the outside, 1 where the artwork starts.
    let side = u.min(1.0 - u) / SIDE_FRAME;
    let end = v.min(1.0 - v) / END_FRAME;
    let t = side.min(end);
    if t >= 1.0 {
        return colour;
    }

    // A slow run of light along the metal, so it reads as a bent sheet
    // catching the room rather than as a grey border — and along whichever
    // way this piece of the frame runs.
    let along = if side < end { v } else { u };
    let shine = 0.5 + 0.5 * (along * 7.3 + 0.6).sin();

    // Across the frame: a dark outer lip, the face of the metal with a
    // highlight rolled into it, then the groove the glass sits in.
    // The three hand over to each other exactly — one band's rise is the
    // next one's fall — so the frame is opaque all the way across and only
    // the innermost tenth lets the artwork through. Overlapping them
    // imperfectly leaves gaps for the picture to tint the metal, which is how
    // a grey frame ends up faintly the colour of whatever is behind it.
    let lip = 1.0 - smoothstep(0.08, 0.20, t);
    let face = smoothstep(0.08, 0.20, t) * (1.0 - smoothstep(0.62, 0.76, t));
    let groove = smoothstep(0.62, 0.76, t) * (1.0 - smoothstep(0.90, 1.0, t));

    let metal = 0.20 + 0.34 * shine + 0.26 * smoothstep(0.20, 0.45, t) * (1.0 - t);
    let mut value = 0.10 * lip + metal * face + 0.05 * groove;
    // Whatever the three bands do not claim is artwork showing through, which
    // is what keeps the inner join from being a hard line.
    let cover = (lip + face + groove).clamp(0.0, 1.0);
    value /= cover.max(1e-3);

    let mut c = [0.0; 3];
    for k in 0..3 {
        // Very slightly cool, the way brushed aluminium is.
        let grey = value * [0.99, 0.995, 1.015][k];
        c[k] = colour[k] + (grey - colour[k]) * cover;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Art that is mostly mud with some strong colour in it — which is what a
    /// playfield photograph is.
    fn art() -> (Vec<u8>, u32, u32) {
        let (w, h) = (64u32, 64u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..h {
            for x in 0..w {
                px.extend_from_slice(&if x < 8 {
                    [220, 30, 40, 255] // a red stripe
                } else if x < 14 {
                    [30, 80, 220, 255] // and a blue one
                } else {
                    [92, 84, 76, 255] // the rest is plywood
                });
            }
        }
        (px, w, h)
    }

    #[test]
    fn the_palette_is_the_tables_colours_and_not_its_mud() {
        // The commonest colour in the art above is brown by a factor of four,
        // and it is the one colour that must not come out: what a machine is
        // remembered by is the paint, not the plywood behind it.
        let (px, w, h) = art();
        let p = Palette::from_art(&px, w, h).expect("two colours in there");
        assert!(p.colours.len() >= 2, "found {:?}", p.colours);
        for c in &p.colours {
            let (_, sat, val) = to_hsv(*c);
            assert!(sat > 0.5, "washed out: {c:?}");
            assert!(val > 0.5, "muddy: {c:?}");
        }
        // Red and blue are both in there, which is what the hue spacing is
        // for: without it the four "dominant colours" are four browns.
        let hues: Vec<f32> = p.colours.iter().map(|c| to_hsv(*c).0).collect();
        assert!(
            hues.iter().any(|h| *h < 0.08 || *h > 0.92),
            "no red: {hues:?}"
        );
        assert!(
            hues.iter().any(|h| (0.5..0.75).contains(h)),
            "no blue: {hues:?}"
        );
    }

    #[test]
    fn a_picture_with_nothing_in_it_says_so() {
        // Bare dark grey, an empty buffer, and — the one that matters — a
        // cut-out whose colour is all behind a zero alpha. Each has to come
        // back empty-handed so the loader can ask another picture instead of
        // painting a head out of a mask.
        assert!(Palette::from_art(&vec![18u8; 32 * 32 * 4], 32, 32).is_none());
        assert!(Palette::from_art(&[], 0, 0).is_none());
        let cutout: Vec<u8> = std::iter::repeat_n([220u8, 30, 40, 0], 32 * 32)
            .flatten()
            .collect();
        assert!(Palette::from_art(&cutout, 32, 32).is_none());
    }

    #[test]
    fn the_sheet_is_opaque_and_the_right_size() {
        let px = paint(&Palette::fallback());
        let (w, h) = BACKGLASS_PIXELS;
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }

    /// The average of a strip of one row of the sheet, as a colour.
    fn strip(px: &[u8], y: u32, x0: u32, x1: u32) -> [f32; 3] {
        let w = BACKGLASS_PIXELS.0;
        let mut sum = [0.0f32; 3];
        for x in x0..x1 {
            let at = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                sum[k] += px[at + k] as f32 / 255.0;
            }
        }
        let n = (x1 - x0) as f32;
        [sum[0] / n, sum[1] / n, sum[2] / n]
    }

    #[test]
    fn the_edges_are_grey_metal_and_the_middle_is_not() {
        let px = paint(&Palette::fallback());
        let (w, h) = BACKGLASS_PIXELS;

        // Down the left-hand frame: no colour in it.
        let edge = strip(&px, h / 2, 1, (w as f32 * SIDE_FRAME * 0.5) as u32);
        assert!(to_hsv(edge).1 < 0.15, "the frame is painted: {edge:?}");

        // And across the sheet at its widest, well below the score window:
        // this is artwork, and artwork has colour in it.
        let art = strip(&px, (h as f32 * 0.80) as u32, w / 4, w * 3 / 4);
        assert!(to_hsv(art).1 > 0.15, "the artwork is grey: {art:?}");
    }

    #[test]
    fn the_score_window_is_dark_enough_to_read_against() {
        let px = paint(&Palette::fallback());
        let (w, h) = BACKGLASS_PIXELS;
        let [left, top, width, height] = crate::backbox::DISPLAY_AREA;
        let mid_y = ((top + height * 0.5) * h as f32) as u32;
        let inside = strip(
            &px,
            mid_y,
            ((left + width * 0.2) * w as f32) as u32,
            ((left + width * 0.8) * w as f32) as u32,
        );
        let brightest = inside[0].max(inside[1]).max(inside[2]);
        assert!(brightest < 0.14, "the window is lit: {inside:?}");

        // And the artwork beside it is not: the mask is a window, not a wash.
        let beside = strip(
            &px,
            mid_y,
            (w as f32 * 0.06) as u32,
            (w as f32 * 0.10) as u32,
        );
        assert!(
            beside[0].max(beside[1]).max(beside[2]) > brightest * 2.0,
            "the mask spread over the sheet: {beside:?}"
        );
    }

    #[test]
    fn the_same_table_is_painted_the_same_picture_twice() {
        // No state and no clock anywhere in here, which is what lets this be
        // baked on one visit and put away for the next.
        let p = Palette::fallback();
        assert_eq!(paint(&p), paint(&p));
    }
}
