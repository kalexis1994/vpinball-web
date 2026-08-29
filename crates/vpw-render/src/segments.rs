//! A segment display, drawn into an image.
//!
//! This is the whole display: nothing above it knows how a digit is shaped.
//! What comes out is a plain RGBA raster, and where that raster **goes** is
//! somebody else's decision — onto the machine's head in the scene, or into a
//! floating canvas on the page. One drawing, two destinations, so the two can
//! never disagree about what the machine is saying.
//!
//! # Where the shapes come from
//!
//! A System 11 does not send characters, it sends **segments**: sixteen bits per
//! digit, one per stroke. Which bit is which stroke is not documented anywhere
//! in prose, but PinMAME's `core_ascii2seg16` (`core.c:187`) is a table of
//! characters to bitmasks, and a few entries pin the whole layout:
//!
//! ```text
//! '/'  0x4400   bits 10 and 14      the two halves of a forward slash
//! '\'  0x1100   bits 8 and 12       the two halves of a backslash
//! '-'  0x0840   bits 6 and 11       the two halves of the middle bar
//! '+'  0x2A40   and 9 and 13        the centre stem, top and bottom
//! '0'  0x443F   bits 0-5, 10, 14    the outer box plus the slash
//! ```
//!
//! Which leaves bit 7 as the comma and bit 15 as the dot — `'0'` with a comma
//! is `0x44BF` and with a dot is `0xC43F`, and the difference in each case is
//! the one bit.
//!
//! # Two kinds of row
//!
//! The top row is alphanumeric and uses all fourteen strokes. The bottom row is
//! the score and has seven, where the middle bar is **one** stroke rather than
//! two halves — so bit 6 there means the whole bar. Drawing a score row with
//! the alphanumeric shapes leaves a gap down the middle of every `-`.

/// Which strokes a row's digits are made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Fourteen strokes: the top row of a System 11, which spells words.
    Alphanumeric,
    /// Seven: the score row, where the middle bar is a single stroke.
    Numeric,
}

/// A row of digits and what they are made of.
#[derive(Debug, Clone)]
pub struct Row<'a> {
    pub segments: &'a [u16],
    pub glyph: Glyph,
}

/// How the display is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// How many digit cells across the display is.
    ///
    /// Not "as many as the rows happen to have": the number of positions is a
    /// fact about the machine and does not move, and a row spelling a short
    /// word still occupies all of them. Fixing it is also what keeps the image
    /// the same size from one drawing to the next, which is what lets a texture
    /// be written in place instead of rebuilt.
    pub columns: usize,
    /// Colour of a lit stroke.
    pub lit: [u8; 3],
    /// Colour of one that is not.
    ///
    /// Not black: a real display's unlit segments are still *there*, a shade
    /// darker than the glass, and seeing them is most of what makes it read as
    /// a segment display rather than as text.
    pub unlit: [u8; 3],
}

impl Default for Style {
    fn default() -> Self {
        Self {
            // Sixteen slots per row, which is what a System 11 strobes: "Of the
            // 16 digit slots only 1-7 and 9-15 are shown" (`display.rs`).
            columns: 16,
            // Gas plasma, which is what these are. The colour is the gas.
            lit: [255, 176, 64],
            unlit: [34, 19, 6],
        }
    }
}

/// An RGBA image, ready to become a texture or to be put on a canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Four bytes per pixel, row major, top row first.
    pub rgba: Vec<u8>,
}

/// A stroke, as two points in a glyph box that runs 0..1 across and 0..2 down.
///
/// Two units tall because a digit is twice as tall as it is wide, and saying so
/// here keeps every coordinate a half or a whole.
type Stroke = ([f32; 2], [f32; 2]);

/// The fourteen strokes, indexed by the bit that lights them.
///
/// `None` where a bit is not a stroke: seven is the comma and fifteen the dot,
/// and both are drawn separately because they live outside the glyph box.
const STROKES: [Option<Stroke>; 16] = [
    Some(([0.0, 0.0], [1.0, 0.0])), // 0  a   top
    Some(([1.0, 0.0], [1.0, 1.0])), // 1  b   upper right
    Some(([1.0, 1.0], [1.0, 2.0])), // 2  c   lower right
    Some(([0.0, 2.0], [1.0, 2.0])), // 3  d   bottom
    Some(([0.0, 1.0], [0.0, 2.0])), // 4  e   lower left
    Some(([0.0, 0.0], [0.0, 1.0])), // 5  f   upper left
    Some(([0.0, 1.0], [0.5, 1.0])), // 6  g1  middle, left half
    None,                           // 7      comma
    Some(([0.0, 0.0], [0.5, 1.0])), // 8  h   upper-left diagonal
    Some(([0.5, 0.0], [0.5, 1.0])), // 9  i   centre stem, upper
    Some(([1.0, 0.0], [0.5, 1.0])), // 10 j   upper-right diagonal
    Some(([0.5, 1.0], [1.0, 1.0])), // 11 g2  middle, right half
    Some(([0.5, 1.0], [1.0, 2.0])), // 12 k   lower-right diagonal
    Some(([0.5, 1.0], [0.5, 2.0])), // 13 l   centre stem, lower
    Some(([0.5, 1.0], [0.0, 2.0])), // 14 m   lower-left diagonal
    None,                           // 15     dot
];

/// The whole middle bar, for a row whose bit six means all of it.
const MIDDLE_BAR: Stroke = ([0.0, 1.0], [1.0, 1.0]);

/// How thick a stroke is, against the width of the glyph.
///
/// Thin. Fourteen strokes have to fit in a box one wide and two tall without
/// running into each other, and the four diagonals all meet at the same point
/// in the middle: at anything like a fifth of the width they merge there and
/// every letter comes out a blob.
const THICKNESS: f32 = 0.09;

/// How much of a cell is padding, so neighbouring digits do not touch.
const PADDING: f32 = 0.08;

/// Draws the rows into an image of exactly the size asked for.
///
/// The size is the caller's because it is the size of something else — a
/// texture that has to stay the same from one drawing to the next, or a canvas
/// on a page. Sizing it from the content instead would mean a new texture every
/// time a word got shorter, and a new texture means a new bind group.
pub fn draw(rows: &[Row<'_>], size: (u32, u32), style: Style) -> Raster {
    let (width, height) = (size.0.max(8), size.1.max(8));
    let columns = style.columns.max(1) as u32;
    let cell = (width / columns).max(4);
    let row_h = if rows.is_empty() {
        height
    } else {
        height / rows.len() as u32
    };

    // The block of cells that is actually used, centred in the image. The
    // column count is a fact about the *display* and stays fixed — that is
    // what keeps the image a constant size and lets a texture be written in
    // place — but a machine with fewer characters than positions, or an
    // image whose width does not divide by the columns, would otherwise sit
    // hard against the left edge with the remainder as a gap on the right.
    // A System 11 has fourteen characters against a sixteen-cell grid, and
    // the two spare cells were visible as exactly that gap, on the head and
    // on the floating panel alike.
    //
    // One offset for every row, taken from the widest: the rows of a real
    // display share a left edge, and centring each on its own would slide
    // them past each other whenever they held different counts.
    let used = rows
        .iter()
        .map(|r| r.segments.len().min(columns as usize) as u32)
        .max()
        .unwrap_or(0);
    let x0 = width.saturating_sub(used * cell) / 2;

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (r, row) in rows.iter().enumerate() {
        for (c, &mask) in row.segments.iter().take(columns as usize).enumerate() {
            draw_digit(
                &mut rgba,
                (width, height),
                (x0 + c as u32 * cell, r as u32 * row_h),
                (cell, row_h),
                mask,
                row.glyph,
                style,
            );
        }
    }
    Raster {
        width,
        height,
        rgba,
    }
}

/// One digit, at a pixel offset inside the image.
fn draw_digit(
    rgba: &mut [u8],
    image: (u32, u32),
    at: (u32, u32),
    cell: (u32, u32),
    mask: u16,
    glyph: Glyph,
    style: Style,
) {
    let (cw, ch) = (cell.0 as f32, cell.1 as f32);
    let pad = cw * PADDING;
    // The glyph is one unit across and two down, and takes whichever of the two
    // the cell has less room for. A display drawn into a box that is not
    // exactly twice as tall as its cells are wide keeps its proportions rather
    // than stretching to fill it.
    let w = (cw - pad * 2.0).min((ch - pad * 2.0) * 0.5).max(1.0);
    let half = THICKNESS * w;
    // Centred in the cell, since the box may be roomier than the glyph.
    let (ox, oy) = ((cw - w) * 0.5, (ch - w * 2.0) * 0.5);

    let place = |p: [f32; 2]| [ox + p[0] * w, oy + p[1] * w];

    for bit in 0..16u16 {
        let Some(stroke) = stroke_for(bit, glyph) else {
            continue;
        };
        // A seven-segment row has no diagonals and no centre stem; lighting one
        // would draw a stroke that display does not physically have.
        let lit = mask & (1 << bit) != 0;
        let colour = if lit { style.lit } else { style.unlit };
        let (a, b) = (place(stroke.0), place(stroke.1));
        capsule(rgba, image, at, a, b, half, colour);
    }

    // The comma and the dot sit outside the glyph box, at its foot — and only
    // the alphanumeric row has them. The score row is seven strokes and seven
    // is all: its wiring is "PIA 2, port B: the 7 segments of the bottom row",
    // with nothing left over for a comma.
    if glyph == Glyph::Numeric {
        return;
    }
    if mask & (1 << 7) != 0 {
        let p = place([1.0, 2.0]);
        capsule(
            rgba,
            image,
            at,
            [p[0], p[1]],
            [p[0] - half, p[1] + half * 2.0],
            half,
            style.lit,
        );
    }
    if mask & (1 << 15) != 0 {
        let p = place([1.0, 2.0]);
        capsule(rgba, image, at, p, p, half, style.lit);
    }
}

/// Which stroke a bit lights, for a row of this kind.
fn stroke_for(bit: u16, glyph: Glyph) -> Option<Stroke> {
    match glyph {
        Glyph::Alphanumeric => STROKES[bit as usize],
        // Seven strokes, and bit six is the whole middle bar rather than its
        // left half: a score row has no seam down the centre of a `-`.
        Glyph::Numeric => match bit {
            6 => Some(MIDDLE_BAR),
            0..=5 => STROKES[bit as usize],
            _ => None,
        },
    }
}

/// Fills the pixels within `half` of the line from `a` to `b`.
///
/// A capsule rather than a rectangle because the strokes meet at angles, and
/// round ends are what makes a diagonal join a vertical without a notch.
#[allow(clippy::too_many_arguments)]
fn capsule(
    rgba: &mut [u8],
    image: (u32, u32),
    at: (u32, u32),
    a: [f32; 2],
    b: [f32; 2],
    half: f32,
    colour: [u8; 3],
) {
    // Only the pixels the capsule can reach, and never past the image: the
    // offsets are absolute, so a stroke that would run off the right-hand edge
    // has to stop rather than wrap onto the next row of pixels.
    let lo_x = ((a[0].min(b[0]) - half).floor().max(0.0)) as u32;
    let hi_x = ((a[0].max(b[0]) + half).ceil().max(0.0)) as u32;
    let lo_y = ((a[1].min(b[1]) - half).floor().max(0.0)) as u32;
    let hi_y = ((a[1].max(b[1]) + half).ceil().max(0.0)) as u32;
    let hi_x = hi_x.min(image.0.saturating_sub(at.0));
    let hi_y = hi_y.min(image.1.saturating_sub(at.1));

    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len_sq = dx * dx + dy * dy;

    for y in lo_y..hi_y {
        for x in lo_x..hi_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Closest point on the segment, clamped to its ends.
            let t = if len_sq > 0.0 {
                (((px - a[0]) * dx + (py - a[1]) * dy) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (cx, cy) = (a[0] + dx * t, a[1] + dy * t);
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            // One pixel of softening at the edge, so a stroke does not come out
            // with stairs down its diagonals.
            let cover = (half + 0.5 - d).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            put(
                rgba,
                image.0,
                at.0 + x,
                at.1 + y,
                colour,
                (cover * 255.0) as u8,
            );
        }
    }
}

/// Writes one pixel, keeping whichever of the two is brighter.
///
/// Brighter rather than blended: strokes overlap where they meet, and blending
/// an unlit one over a lit one would bite a notch out of the join.
fn put(rgba: &mut [u8], image_width: u32, x: u32, y: u32, colour: [u8; 3], alpha: u8) {
    if x >= image_width {
        return;
    }
    let i = ((y * image_width + x) * 4) as usize;
    let Some(px) = rgba.get_mut(i..i + 4) else {
        return;
    };
    let brightness = |c: &[u8]| u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]);
    let incoming = u32::from(colour[0]) + u32::from(colour[1]) + u32::from(colour[2]);
    if px[3] > 0 && brightness(px) >= incoming {
        // Already something at least as bright here.
        px[3] = px[3].max(alpha);
        return;
    }
    px[0] = colour[0];
    px[1] = colour[1];
    px[2] = colour[2];
    px[3] = px[3].max(alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every stroke of a fourteen-segment digit, comma and dot included.
    const ALL: u16 = 0xFFFF;

    fn lit_pixels(r: &Raster, style: Style) -> usize {
        r.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0 && p[0] == style.lit[0] && p[1] == style.lit[1])
            .count()
    }

    /// One digit, in a box of its own.
    fn one(mask: u16, glyph: Glyph) -> (Raster, Style) {
        let style = Style {
            columns: 1,
            ..Style::default()
        };
        let r = draw(
            &[Row {
                segments: &[mask],
                glyph,
            }],
            (CELL, CELL * 2),
            style,
        );
        (r, style)
    }

    /// A cell big enough that a stroke is several pixels across, so counting
    /// lit pixels says something.
    const CELL: u32 = 48;

    #[test]
    fn a_dark_digit_is_still_there() {
        // A real display's unlit segments do not disappear: they are a shade
        // darker than the glass, and seeing them is most of what makes this
        // read as a segment display rather than as text.
        let (r, style) = one(0, Glyph::Alphanumeric);
        assert_eq!(lit_pixels(&r, style), 0, "nothing is lit");
        let drawn = r
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count();
        assert!(drawn > 0, "but the strokes are still drawn: {drawn} pixels");
    }

    #[test]
    fn lighting_more_strokes_lights_more_pixels() {
        let style = Style::default();
        let (blank, _) = one(0, Glyph::Alphanumeric);
        let (one_stroke, _) = one(1, Glyph::Alphanumeric);
        let (everything, _) = one(ALL, Glyph::Alphanumeric);

        assert!(lit_pixels(&blank, style) < lit_pixels(&one_stroke, style));
        assert!(lit_pixels(&one_stroke, style) < lit_pixels(&everything, style));
    }

    #[test]
    fn a_score_row_has_no_seam_down_the_middle_of_a_dash() {
        // The bottom row is seven-segment and its bit six is the **whole**
        // middle bar. Drawing it with the alphanumeric shapes lights only the
        // left half, and every `-` comes out broken.
        let style = Style::default();
        let dash = 1 << 6;
        let (numeric, _) = one(dash, Glyph::Numeric);
        let (alpha, _) = one(dash, Glyph::Alphanumeric);
        assert!(
            lit_pixels(&numeric, style) > lit_pixels(&alpha, style) * 3 / 2,
            "the score row's bar should be about twice as long: {} against {}",
            lit_pixels(&numeric, style),
            lit_pixels(&alpha, style)
        );
    }

    #[test]
    fn a_score_row_has_no_diagonals_to_light() {
        // Seven strokes means seven. Lighting a bit that display does not
        // physically have would draw a stroke that is not on the glass.
        let style = Style::default();
        let (seven, _) = one(0x7F, Glyph::Numeric);
        let (asked_for_everything, _) = one(ALL, Glyph::Numeric);
        assert_eq!(
            lit_pixels(&seven, style),
            lit_pixels(&asked_for_everything, style),
            "the extra bits have nowhere to go"
        );
    }

    #[test]
    fn the_slash_and_the_backslash_cross() {
        // The bits these came from are what pin the whole layout down:
        // `'/'` is 0x4400 and `'\\'` is 0x1100 (`core.c:187`). If the four
        // diagonals were assigned any other way the two would not make an `X`.
        let style = Style::default();
        let (slash, _) = one(0x4400, Glyph::Alphanumeric);
        let (backslash, _) = one(0x1100, Glyph::Alphanumeric);
        let (cross, _) = one(0x5500, Glyph::Alphanumeric);

        assert!(lit_pixels(&slash, style) > 0);
        assert_eq!(
            lit_pixels(&slash, style),
            lit_pixels(&backslash, style),
            "the two diagonals are mirror images and cover the same area"
        );
        // The X is the two of them, less where they overlap in the middle.
        let both = lit_pixels(&slash, style) + lit_pixels(&backslash, style);
        assert!(
            lit_pixels(&cross, style) < both,
            "they have to actually cross: {} against {both}",
            lit_pixels(&cross, style)
        );
    }

    #[test]
    fn the_image_is_the_size_it_was_asked_for() {
        // Whatever the rows have in them. The size belongs to the thing being
        // drawn into — a texture that has to stay put, or a canvas — and a word
        // getting shorter cannot be allowed to resize it.
        let style = Style::default();
        for size in [(384u32, 112u32), (512, 160), (200, 60)] {
            let r = draw(
                &[
                    Row {
                        segments: &[0; 14],
                        glyph: Glyph::Alphanumeric,
                    },
                    Row {
                        segments: &[0; 3],
                        glyph: Glyph::Numeric,
                    },
                ],
                size,
                style,
            );
            assert_eq!((r.width, r.height), size);
            assert_eq!(r.rgba.len(), (r.width * r.height * 4) as usize);
        }
    }

    #[test]
    fn more_digits_than_there_are_slots_are_dropped_rather_than_wrapped() {
        // The number of slots is a fact about the machine. A row with more in
        // it than fits has to stop at the edge, not run on into the next line
        // of pixels.
        let style = Style {
            columns: 4,
            ..Style::default()
        };
        let size = (4 * CELL, CELL * 2);
        let four = draw(
            &[Row {
                segments: &[ALL; 4],
                glyph: Glyph::Alphanumeric,
            }],
            size,
            style,
        );
        let eight = draw(
            &[Row {
                segments: &[ALL; 8],
                glyph: Glyph::Alphanumeric,
            }],
            size,
            style,
        );
        assert_eq!(four, eight, "the extra four had nowhere to go");
    }

    #[test]
    fn nothing_is_drawn_outside_its_own_digit() {
        // Every stroke lit, in the left-hand cell of two. The right-hand one
        // has to come out untouched, or neighbouring digits bleed into each
        // other and the display turns into a smear at small sizes.
        let style = Style {
            columns: 2,
            ..Style::default()
        };
        let r = draw(
            &[Row {
                segments: &[ALL, 0],
                glyph: Glyph::Alphanumeric,
            }],
            (CELL * 2, CELL * 2),
            style,
        );
        for y in 0..r.height {
            for x in CELL..r.width {
                let i = ((y * r.width + x) * 4) as usize;
                let px = &r.rgba[i..i + 4];
                assert!(
                    px[0] != style.lit[0] || px[1] != style.lit[1] || px[3] == 0,
                    "a lit pixel at ({x}, {y}) belongs to the digit next door"
                );
            }
        }
    }
}
