//! Where the score goes on a table's desktop backdrop.
//!
//! In desktop mode the original draws a table's backdrop picture over the
//! whole window and no head at all (`Renderer::DrawBackground`,
//! `Renderer.cpp:921`): the picture *is* the backglass, composed by the author
//! in screen space with the playfield standing over its middle. Spider-Man's
//! is the machine's own glass twice over, side by side, so that the windows
//! for players one and three come out left of the table and two and four to
//! its right. The score itself is PinMAME's own window, which the player
//! drags over whatever the artwork painted for it.
//!
//! This port has no second window, so the display is drawn *on* the picture,
//! and the question is where. Two answers, in order:
//!
//! 1. **The table says.** A textbox or flasher flagged DMD is the original's
//!    own way of putting the score on the backdrop: it draws in a flat
//!    1000×750 space stretched over the window (`EDITOR_BG_WIDTH`,
//!    `stdafx.h:43`; `Textbox::Render`, `textbox.cpp:270`). Exact, and rare.
//! 2. **The paint says.** A backglass painted for a machine with score
//!    windows has them: flat-coloured, framed, wider than tall, the size a
//!    six-digit display is. Found by looking, which is a guess — a good one on
//!    Spider-Man, where the four windows come out to the pixel, and an empty
//!    one on F-14 or Circus, whose backdrops paint no windows. See
//!    [`painted`].
//!
//! Neither answer says which window is whose, and the paint may say more
//! places than there are players: Spider-Man's glass is painted twice, and
//! half of its eight windows are behind the playfield from any camera. So
//! what leaves here is every candidate, and the renderer — which knows where
//! the table lands on the screen — sets the covered ones aside and hands the
//! rest out by corner, the way a four-player backglass has been laid out
//! since the seventies: player one top left, two top right, three and four
//! beneath. See [`by_corner`].
//!
//! And when neither has anything to say, the display goes in a strip near
//! the top, wherever the picture has the least going on and the table is not.
//! See [`FALLBACK_WINDOWS`].

use vpin::vpx::VPX;
use vpin::vpx::gameitem::GameItemEnum;

use crate::geometry::Image;

/// A rectangle on the screen, as `[left, top, width, height]` in fractions
/// of it. The backdrop is stretched over the whole window, so a fraction of
/// the picture is the same fraction of the screen.
pub type Rect = [f32; 4];

/// Where the playfield lands on the screen: its four corners, in the same
/// fractions, going round. A trapezoid from any camera but the overhead
/// one — the far end narrow, the near end wide — and its bounding box would
/// claim the whole top of the screen that the far end leaves free.
pub type Quad = [[f32; 2]; 4];

/// The original's flat backdrop space (`stdafx.h:43`).
pub const EDITOR_BG: (f32, f32) = (1000.0, 750.0);

/// Where the score goes when nothing says: one of these strips, the one with
/// the least art under it that the table does not cover. Top centre first,
/// because on a backdrop composed the usual way that is over the table's far
/// end and nothing else; then the corners.
pub const FALLBACK_WINDOWS: [Rect; 5] = [
    [0.37, 0.03, 0.26, 0.12],
    [0.02, 0.03, 0.26, 0.12],
    [0.72, 0.03, 0.26, 0.12],
    [0.02, 0.85, 0.26, 0.12],
    [0.72, 0.85, 0.26, 0.12],
];

/// Where the score may be drawn: every candidate, in no particular order.
/// The renderer chooses among them once it knows where the table stands on
/// the screen; see [`by_corner`] and [`pick_one`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScoreWindows {
    pub rects: Vec<Rect>,
    /// Whether the candidates are alternatives for **one** window rather
    /// than windows for several players. The fallback strips are; the paint's
    /// windows are not.
    pub pick_one: bool,
    /// Whether the score goes *over* the table rather than under it.
    ///
    /// Painted windows belong to the picture and are drawn with it, under
    /// the table, so the playfield covers whichever ones it stands on. A
    /// display the table declared is drawn over everything, which is where
    /// the original puts it (`textbox.cpp:317`, depth −10000) — and so is the
    /// strip the score falls back to, which is nowhere in the picture and
    /// would be under the table's far end if it were drawn with it.
    pub over: bool,
}

/// Where a table's score goes on its backdrop. Empty for a table with no
/// backdrop picture; never empty with one.
pub fn score_windows(vpx: &VPX, images: &[Image], backdrop: &str) -> ScoreWindows {
    if backdrop.is_empty() {
        return ScoreWindows::default();
    }
    if let Some(rect) = declared(vpx) {
        log::debug!("the table puts its score at {rect:?} on the backdrop");
        return ScoreWindows {
            rects: vec![rect],
            pick_one: false,
            over: true,
        };
    }
    let picture = images
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case(backdrop))
        .and_then(decoded);
    let found = picture
        .as_ref()
        .map(|(rgba, w, h)| painted(rgba, *w, *h))
        .unwrap_or_default();
    if found.is_empty() {
        log::debug!("no score windows on the backdrop; the score goes in a strip");
        ScoreWindows {
            rects: match &picture {
                Some((rgba, w, h)) => quietest_first(rgba, *w, *h),
                None => FALLBACK_WINDOWS.to_vec(),
            },
            pick_one: true,
            over: true,
        }
    } else {
        log::debug!("{} score windows painted on the backdrop: {found:?}", found.len());
        ScoreWindows {
            rects: found,
            pick_one: false,
            over: false,
        }
    }
}

/// The fallback strips, the one with the least art under it first.
///
/// "Art" is any pixel that is not near black: a backdrop that is mostly
/// black with a logo and a figure on it — Circus — should get its score in
/// the black, not across the logo. Ties keep [`FALLBACK_WINDOWS`]' order.
fn quietest_first(rgba: &[u8], width: u32, height: u32) -> Vec<Rect> {
    let busy = |r: Rect| -> f32 {
        let x0 = (r[0] * width as f32) as u32;
        let x1 = ((r[0] + r[2]) * width as f32).min(width as f32) as u32;
        let y0 = (r[1] * height as f32) as u32;
        let y1 = ((r[1] + r[3]) * height as f32).min(height as f32) as u32;
        let (mut lit, mut all) = (0u32, 0u32);
        // Every fourth pixel each way is plenty for a fraction.
        for y in (y0..y1).step_by(4) {
            for x in (x0..x1).step_by(4) {
                let i = ((y * width + x) * 4) as usize;
                if rgba.len() >= i + 3 {
                    all += 1;
                    if rgba[i].max(rgba[i + 1]).max(rgba[i + 2]) > 40 {
                        lit += 1;
                    }
                }
            }
        }
        if all == 0 { 0.0 } else { lit as f32 / all as f32 }
    };
    let mut strips: Vec<(f32, usize, Rect)> = FALLBACK_WINDOWS
        .iter()
        .enumerate()
        .map(|(i, &r)| (busy(r), i, r))
        .collect();
    strips.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    strips.into_iter().map(|(_, _, r)| r).collect()
}

/// The table's own placement of its display, if it made one.
///
/// A textbox flagged DMD, or — the 10.0 form the original still honours
/// (`textbox.cpp:270`) — one whose text says "DMD"; failing that, a flasher
/// flagged DMD on the backglass. Both in the 1000×750 space.
pub fn declared(vpx: &VPX) -> Option<Rect> {
    let norm = |x0: f32, y0: f32, x1: f32, y1: f32| -> Option<Rect> {
        let (l, t) = (x0.min(x1) / EDITOR_BG.0, y0.min(y1) / EDITOR_BG.1);
        let (w, h) = ((x1 - x0).abs() / EDITOR_BG.0, (y1 - y0).abs() / EDITOR_BG.1);
        (w > 0.0 && h > 0.0).then_some([l, t, w, h])
    };
    for item in &vpx.gameitems {
        match item {
            GameItemEnum::TextBox(t)
                if t.is_dmd.unwrap_or(false) || t.text.to_ascii_lowercase().contains("dmd") =>
            {
                if let Some(r) = norm(t.ver1.x, t.ver1.y, t.ver2.x, t.ver2.y) {
                    return Some(r);
                }
            }
            GameItemEnum::Flasher(f)
                if f.is_dmd.unwrap_or(false) && f.backglass.unwrap_or(false) && f.is_visible =>
            {
                let xs = f.drag_points.iter().map(|p| p.x);
                let ys = f.drag_points.iter().map(|p| p.y);
                let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
                let (y0, y1) = (ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max));
                if let Some(r) = norm(x0, y0, x1, y1) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

/// The picture's pixels, whatever form the file keeps them in.
pub fn decoded(image: &Image) -> Option<(Vec<u8>, u32, u32)> {
    if let Some(rgba) = &image.rgba {
        return Some((rgba.clone(), image.width, image.height));
    }
    let bytes = image.encoded.as_ref()?;
    match image::load_from_memory(bytes) {
        Ok(decoded) => {
            let rgba = decoded.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            Some((rgba.into_raw(), w, h))
        }
        Err(e) => {
            log::warn!("the backdrop \"{}\" would not decode: {e}", image.name);
            None
        }
    }
}

/// How wide the picture is looked at. Coarse on purpose: a window is a
/// feature a hundred pixels across, and the brush strokes inside the art are
/// the thing to blur away.
const LOOK_WIDTH: u32 = 400;
/// How much two neighbouring pixels' colours may differ, as chromaticity
/// (each channel's share of the sum), and still be one flat patch.
const CHROMA_TOL: f32 = 0.045;
/// And how much their brightness may differ. Loose: a painted window has a
/// highlight across it, which changes how bright it is and not what colour.
const LUMA_TOL: f32 = 60.0;
/// A window is at least this much of its box, and shaped like this.
const MIN_FILL: f32 = 0.80;
const ASPECT: (f32, f32) = (2.2, 10.0);
/// As fractions of the picture: a six-digit display, not a pixel and not a
/// wall.
const WIDTH: (f32, f32) = (0.05, 0.45);
const HEIGHT: (f32, f32) = (0.015, 0.15);

/// The score windows painted on a backglass, as fractions of the picture, in
/// no particular order.
///
/// A window is a patch of one flat colour whose bounding box it fills, wider
/// than it is tall, of the size a display is. Flatness is judged on the
/// colour's *chromaticity* rather than its value, because a painted window
/// has a painted highlight — Spider-Man's are teal with a lighter band across
/// the top — and that is the same paint at two brightnesses. The frame round
/// a window is what stops the patch at its edge; the art outside it is
/// anything at all and no assumption is made about it.
pub fn painted(rgba: &[u8], width: u32, height: u32) -> Vec<Rect> {
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return Vec::new();
    }
    // Box-filtered down to a size where a window is a few dozen pixels.
    let (small, w, h) = downscale(rgba, width, height, LOOK_WIDTH);
    let (w, h) = (w as usize, h as usize);
    // Chromaticity and brightness of each pixel.
    let chroma: Vec<[f32; 4]> = small
        .chunks(4)
        .map(|c| {
            let c = [f32::from(c[0]), f32::from(c[1]), f32::from(c[2])];
            let s = c[0] + c[1] + c[2] + 1.0;
            [c[0] / s, c[1] / s, c[2] / s, s / 3.0]
        })
        .collect();
    let same = |a: [f32; 4], b: [f32; 4]| {
        (a[0] - b[0]).abs() <= CHROMA_TOL
            && (a[1] - b[1]).abs() <= CHROMA_TOL
            && (a[2] - b[2]).abs() <= CHROMA_TOL
            && (a[3] - b[3]).abs() <= LUMA_TOL
    };
    // A pixel is flat when it matches all four neighbours. The border is not.
    let mut flat = vec![false; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let c = chroma[y * w + x];
            flat[y * w + x] = same(c, chroma[y * w + x - 1])
                && same(c, chroma[y * w + x + 1])
                && same(c, chroma[(y - 1) * w + x])
                && same(c, chroma[(y + 1) * w + x]);
        }
    }
    // Connected patches of flat pixels and their boxes.
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for start in 0..w * h {
        if !flat[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
        let mut count = 0usize;
        while let Some(i) = stack.pop() {
            count += 1;
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            for j in [i.wrapping_sub(1), i + 1, i.wrapping_sub(w), i + w] {
                if j < w * h && flat[j] && !seen[j] && (j / w == y || j % w == x) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        let fill = count as f32 / (bw * bh);
        let aspect = bw / bh;
        let (wf, hf) = (bw / w as f32, bh / h as f32);
        if fill >= MIN_FILL
            && (ASPECT.0..=ASPECT.1).contains(&aspect)
            && (WIDTH.0..=WIDTH.1).contains(&wf)
            && (HEIGHT.0..=HEIGHT.1).contains(&hf)
        {
            out.push([x0 as f32 / w as f32, y0 as f32 / h as f32, wf, hf]);
        }
    }
    out
}

/// A picture box-filtered down to at most `max_width` across, or left as it
/// is when it is already no wider. Alpha comes along.
pub fn downscale(rgba: &[u8], width: u32, height: u32, max_width: u32) -> (Vec<u8>, u32, u32) {
    if width <= max_width || width == 0 || height == 0 {
        return (rgba.to_vec(), width, height);
    }
    let scale = width as f32 / max_width as f32;
    let w = max_width;
    let h = (height as f32 / scale).round().max(1.0) as u32;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let y0 = (y as f32 * scale) as u32;
        let y1 = (((y + 1) as f32 * scale) as u32).clamp(y0 + 1, height);
        for x in 0..w {
            let x0 = (x as f32 * scale) as u32;
            let x1 = (((x + 1) as f32 * scale) as u32).clamp(x0 + 1, width);
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * width + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u32::from(rgba[i + c]);
                    }
                    n += 1;
                }
            }
            let o = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    (out, w, h)
}

/// How wide one copy is, when a picture is the same thing twice side by
/// side — a backdrop painted with the glass once for each side of the
/// playfield. `None` when it is not.
///
/// Not the halves: Spider-Man's second copy is cropped at the edge, so the
/// repeat is at sixty-two per cent of the width and the halves differ
/// everywhere. Instead every shift from a third to two thirds of the width
/// is tried on a coarse copy and the one that lines the picture up with
/// itself is the copy's width, if it lines up closely — and only on a
/// picture with something in it: a black backdrop lines up with itself at
/// every shift and is not two of anything.
pub fn repeat_width(rgba: &[u8], width: u32, height: u32) -> Option<u32> {
    const LOOK: u32 = 128;
    const CLOSE: f32 = 10.0;
    const CONTRAST: f32 = 20.0;
    if width < 8 || height == 0 {
        return None;
    }
    let (small, w, h) = downscale(rgba, width, height, LOOK);
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize, c: usize| f32::from(small[(y * w + x) * 4 + c]);
    let every = || (0..h).flat_map(|y| (0..w).flat_map(move |x| (0..3).map(move |c| (x, y, c))));
    // Contrast: how far the picture strays from its own mean.
    let n = (w * h * 3) as f32;
    let mean = every().map(|(x, y, c)| at(x, y, c)).sum::<f32>() / n;
    let contrast = every().map(|(x, y, c)| (at(x, y, c) - mean).abs()).sum::<f32>() / n;
    if contrast < CONTRAST {
        return None;
    }
    let mut best: Option<(f32, usize)> = None;
    for shift in (w / 3)..(w * 2 / 3) {
        let mut diff = 0.0f32;
        let mut count = 0.0f32;
        for y in 0..h {
            for x in 0..(w - shift) {
                for c in 0..3 {
                    diff += (at(x, y, c) - at(x + shift, y, c)).abs();
                    count += 1.0;
                }
            }
        }
        let d = diff / count.max(1.0);
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, shift));
        }
    }
    let (d, shift) = best?;
    (d < CLOSE).then(|| (shift as f32 * width as f32 / w as f32).round() as u32)
}

/// The leftmost `copy` pixels of every row of a picture, less any margin:
/// a repeat runs from the start of one copy to the start of the next, and
/// the author may have left a band of nothing between them. Spider-Man's is
/// a hundred columns of pure white. Columns of one flat colour are trimmed
/// off both edges. See [`flat_columns`].
pub fn crop_left(rgba: &[u8], width: u32, height: u32, copy: u32) -> (Vec<u8>, u32, u32) {
    let copy = copy.clamp(1, width);
    let (skip, keep) = flat_columns(rgba, width, height, copy);
    let mut out = Vec::with_capacity((keep * height * 4) as usize);
    for y in 0..height {
        let start = ((y * width + skip) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (keep * 4) as usize]);
    }
    (out, keep, height)
}

/// The columns of a picture's first `copy` columns that are not margin, as
/// `(first column to keep, columns to keep)`.
///
/// A margin is a run of columns of one flat colour — every pixel within a
/// few levels of the column's top one — sitting at either edge of the copy,
/// or near it: the copy's width comes from a coarse look at the picture and
/// may land a few columns into the next copy, past the margin. So a run that
/// ends within a sixteenth of the width of the cut counts, and everything
/// from its start is dropped. At least one column is always kept.
fn flat_columns(rgba: &[u8], width: u32, height: u32, copy: u32) -> (u32, u32) {
    const TOL: u8 = 4;
    const MIN_RUN: u32 = 3;
    let flat = |x: u32| {
        let top = [rgba[(x * 4) as usize], rgba[(x * 4) as usize + 1], rgba[(x * 4) as usize + 2]];
        (0..height).all(|y| {
            let i = ((y * width + x) * 4) as usize;
            (0..3).all(|c| rgba[i + c].abs_diff(top[c]) <= TOL)
        })
    };
    let near = (copy / 16).max(1);
    // The runs of flat columns, as (start, end).
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut x = 0;
    while x < copy {
        if flat(x) {
            let start = x;
            while x < copy && flat(x) {
                x += 1;
            }
            if x - start >= MIN_RUN {
                runs.push((start, x));
            }
        } else {
            x += 1;
        }
    }
    let left = runs
        .iter()
        .filter(|(start, _)| *start <= near)
        .map(|(_, end)| *end)
        .max()
        .unwrap_or(0);
    let right = runs
        .iter()
        .filter(|(_, end)| copy - *end <= near)
        .map(|(start, _)| *start)
        .min()
        .unwrap_or(copy);
    if right > left {
        (left, right - left)
    } else {
        (0, copy)
    }
}

/// The windows by player: top left is player one, top right two, bottom
/// left three, bottom right four — the way a four-player backglass has
/// always been laid out. Each window goes to the corner it is nearest, and
/// where two claim the same corner the nearer wins; a player whose corner
/// has no window gets `None`. Trailing `None`s are dropped, so a single
/// window anywhere comes back as `[Some(window)]`: player one, wherever it
/// is.
///
/// `table` is where the playfield stands on the screen, when known: a window
/// it covers more than half of is no use and is set aside, unless that
/// leaves nothing, in which case the covered ones are better than none.
pub fn by_corner(windows: &[Rect], table: Option<Quad>) -> Vec<Option<Rect>> {
    let clear = clear_of(windows, table);
    if clear.len() == 1 {
        return vec![Some(clear[0])];
    }
    let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let mut out: Vec<Option<(f32, Rect)>> = vec![None; 4];
    for r in clear {
        let (cx, cy) = (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5);
        let (d, at) = corners
            .iter()
            .enumerate()
            .map(|(i, c)| ((cx - c[0]).powi(2) + (cy - c[1]).powi(2), i))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or((0.0, 0));
        if out[at].is_none_or(|(had, _)| d < had) {
            out[at] = Some((d, r));
        }
    }
    let mut out: Vec<Option<Rect>> = out.into_iter().map(|o| o.map(|(_, r)| r)).collect();
    while out.last().is_some_and(Option::is_none) {
        out.pop();
    }
    out
}

/// One window out of several alternatives: the one the table covers least,
/// and of those it covers equally, the first.
pub fn pick_one(windows: &[Rect], table: Option<Quad>) -> Option<Rect> {
    let Some(t) = table else {
        return windows.first().copied();
    };
    windows
        .iter()
        .enumerate()
        .map(|(i, r)| (covered(*r, t), i, *r))
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, _, r)| r)
}

/// The windows the table does not stand on, or all of them if it stands on
/// every one.
fn clear_of(windows: &[Rect], table: Option<Quad>) -> Vec<Rect> {
    let clear: Vec<Rect> = match table {
        Some(t) => windows
            .iter()
            .copied()
            .filter(|w| covered(*w, t) <= 0.5)
            .collect(),
        None => windows.to_vec(),
    };
    if clear.is_empty() {
        windows.to_vec()
    } else {
        clear
    }
}

/// How much of `window` lies under `table`, as a fraction of the window:
/// a grid of points across the window, and the share of them inside the
/// table's outline.
fn covered(window: Rect, table: Quad) -> f32 {
    const ACROSS: usize = 8;
    const DOWN: usize = 4;
    let mut inside = 0;
    for j in 0..DOWN {
        for i in 0..ACROSS {
            let x = window[0] + window[2] * (i as f32 + 0.5) / ACROSS as f32;
            let y = window[1] + window[3] * (j as f32 + 0.5) / DOWN as f32;
            if in_quad([x, y], table) {
                inside += 1;
            }
        }
    }
    inside as f32 / (ACROSS * DOWN) as f32
}

/// Whether a point is inside a convex quadrilateral, whichever way round
/// its corners go: on the same side of all four edges.
fn in_quad(p: [f32; 2], q: Quad) -> bool {
    let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let s: [f32; 4] = std::array::from_fn(|i| side(q[i], q[(i + 1) % 4]));
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// The outline of a rectangle, for a table that is one.
pub fn quad_of(rect: Rect) -> Quad {
    [
        [rect[0], rect[1]],
        [rect[0] + rect[2], rect[1]],
        [rect[0] + rect[2], rect[1] + rect[3]],
        [rect[0], rect[1] + rect[3]],
    ]
}
