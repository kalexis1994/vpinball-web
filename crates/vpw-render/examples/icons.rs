//! Draws the application icons.
//!
//!     cargo run --release -p vpw-render --example icons -- web/public
//!
//! Run once and commit what comes out: the page's build must not depend on a
//! Rust toolchain being present, and an icon that never changes has no business
//! being regenerated on every deploy.
//!
//! # Why it is drawn rather than photographed
//!
//! The obvious icon for this is a photograph of a table, and at 192 pixels a
//! photograph of a table is a smudge: a real playfield is a dense collage with
//! no single shape in it, and shrinking one keeps the density and loses the
//! shapes. What survives at that size is one silhouette with strong contrast,
//! so this is a ball and two flippers and nothing else — the two shapes nobody
//! mistakes for anything but pinball.
//!
//! Everything is drawn into a buffer four times too big and then averaged down,
//! which is the cheapest antialiasing there is and enough for shapes this
//! simple.

use std::path::PathBuf;

/// How much bigger the working buffer is than the icon.
const SS: u32 = 4;

/// The fraction of a maskable icon that is guaranteed to survive being cropped.
///
/// Android may cut a maskable icon to a circle, a squircle or a rounded square,
/// and only the middle 80% of it is promised to come through — the rest is
/// bleed. So the maskable variant draws the same picture smaller and lets the
/// background run to the edges.
const SAFE: f32 = 0.8;

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "web/public".into())
        .into();
    std::fs::create_dir_all(&dir).expect("could not make the output directory");

    for (name, size, scale) in [
        ("icon-192.png", 192u32, 1.0f32),
        ("icon-512.png", 512, 1.0),
        // Drawn small enough that a circular crop keeps all of it.
        ("icon-maskable-512.png", 512, SAFE),
        // iOS puts its own rounded corners on and does not crop, so this one is
        // the plain drawing at the size Apple asks for.
        ("apple-touch-icon.png", 180, 1.0),
    ] {
        let image = draw(size, scale);
        let path = dir.join(name);
        image.save(&path).expect("could not write the icon");
        println!("{}  {size}x{size}", path.display());
    }
}

/// One icon. `scale` shrinks the artwork without shrinking the background.
fn draw(size: u32, scale: f32) -> image::RgbaImage {
    let big = size * SS;
    let mut buf = image::RgbaImage::new(big, big);
    let n = big as f32;

    for y in 0..big {
        for x in 0..big {
            // -1..1 across the icon, y up.
            let px = (x as f32 + 0.5) / n * 2.0 - 1.0;
            let py = 1.0 - (y as f32 + 0.5) / n * 2.0;
            buf.put_pixel(x, y, image::Rgba(shade(px, py, scale)));
        }
    }

    image::imageops::resize(&buf, size, size, image::imageops::FilterType::Lanczos3)
}

/// The colour at one point of the icon, in -1..1 space.
fn shade(x: f32, y: f32, scale: f32) -> [u8; 4] {
    // The cabinet: a dark blue that goes darker towards the bottom, so the
    // silver of the ball has something to sit against at any size.
    let t = (1.0 - y) * 0.5;
    let mut c = [
        mix(0.055, 0.020, t),
        mix(0.075, 0.028, t),
        mix(0.145, 0.055, t),
    ];

    // Artwork coordinates, so the maskable variant is the same picture smaller.
    let (x, y) = (x / scale, y / scale);

    // A faint glow behind the ball, the way an insert lights the plastic
    // around it. It is what stops the background reading as flat black.
    let glow = (1.0 - (x * x + (y - 0.18) * (y - 0.18)).sqrt() / 0.85).max(0.0);
    let glow = glow * glow * 0.12;
    c = [c[0] + glow * 0.3, c[1] + glow * 0.5, c[2] + glow];

    // The flippers: two capsules pointing up and outwards, in the red a
    // Williams cabinet uses.
    for side in [-1.0f32, 1.0] {
        let a = capsule(x, y, side * 0.13, -0.42, side * 0.52, -0.10, 0.085);
        if a > 0.0 {
            // Lit along the top edge, so it reads as a solid object and not a
            // sticker.
            let lit = ((y + 0.42) * 0.9).clamp(0.0, 1.0);
            let red = [
                mix(0.55, 0.95, lit),
                mix(0.06, 0.20, lit),
                mix(0.09, 0.22, lit),
            ];
            c = over(c, red, a);
        }
    }

    // The ball, with the highlight up and to the left, matching the glow.
    let r = (x * x + (y - 0.22) * (y - 0.22)).sqrt();
    let ball = 0.27;
    let a = coverage(ball - r);
    if a > 0.0 {
        // A sphere lit from the upper left: the shade follows the surface
        // normal rather than the flat distance, or it looks like a disc.
        let nx = x / ball;
        let ny = (y - 0.22) / ball;
        let nz = (1.0 - (nx * nx + ny * ny)).max(0.0).sqrt();
        let l = (nx * -0.45 + ny * 0.55 + nz * 0.70).max(0.0);
        let base = 0.28 + 0.62 * l;
        // Chrome takes a hard specular on top of the diffuse.
        let spec = l.powf(28.0) * 0.9;
        let v = (base + spec).min(1.0);
        c = over(c, [v * 0.98, v * 0.99, v], a);
    }

    [to_srgb(c[0]), to_srgb(c[1]), to_srgb(c[2]), 255]
}

/// How much of a pixel a shape covers, from its signed distance.
///
/// A hard step would alias even at four times the resolution, because the
/// downsample only sees whole samples; a one-sample-wide ramp gives the
/// resampler something to average.
fn coverage(d: f32) -> f32 {
    (d / 0.006 + 0.5).clamp(0.0, 1.0)
}

/// Coverage of a capsule: a segment from `(x0, y0)` to `(x1, y1)`, radius `r`.
fn capsule(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (x0 + dx * t, y0 + dy * t);
    coverage(r - ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn over(under: [f32; 3], top: [f32; 3], a: f32) -> [f32; 3] {
    [
        mix(under[0], top[0], a),
        mix(under[1], top[1], a),
        mix(under[2], top[2], a),
    ]
}

/// Linear to sRGB. The shading above is done in linear light, which is the only
/// way the sphere comes out looking round.
fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}
