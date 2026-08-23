//! Draws a segment display to a PNG, to look at it.
//!
//! ```text
//! cargo run --release -p vpw-render --example segments -- out.png
//! ```
//!
//! It writes what a System 11 says at power-up, which is the one string that
//! exercises both rows: letters on the top and a score on the bottom.

use vpw_render::segments::{Glyph, Row, Style, draw};

/// PinMAME's own table of characters to segment bitmasks (`core.c:187`), for
/// the entries this needs. Turning text into segments is the emulator's job in
/// the real thing; here it is just how the sample is written.
fn seg16(c: char) -> u16 {
    match c {
        '0' => 0x443F,
        '1' => 0x2200,
        '2' => 0x085B,
        '3' => 0x084F,
        '4' => 0x0866,
        '5' => 0x086D,
        '6' => 0x087D,
        '7' => 0x0007,
        '8' => 0x087F,
        '9' => 0x086F,
        'A' => 0x0877,
        'B' => 0x2A0F,
        'C' => 0x0039,
        'E' => 0x0079,
        'F' => 0x0071,
        'H' => 0x0876,
        'I' => 0x2209,
        'J' => 0x001E,
        'L' => 0x0038,
        'M' => 0x0536,
        'N' => 0x1136,
        'O' => 0x003F,
        'P' => 0x0873,
        'R' => 0x1873,
        'S' => 0x086D,
        'T' => 0x2201,
        'U' => 0x003E,
        'W' => 0x5036,
        'Y' => 0x2500,
        '-' => 0x0840,
        _ => 0x0000,
    }
}

/// The seven-segment digits.
///
/// Almost the low seven bits of the same table, and `1` is the exception that
/// says why "almost" matters: a fourteen-segment `1` is the centre stem, which
/// a seven-segment display does not have, so masking it down gives a blank.
fn seg7(c: char) -> u16 {
    match c {
        '1' => 0x06,
        _ => seg16(c) & 0x7F,
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "segments.png".into());

    let text = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "  F-14 TOMCAT ".into());
    let score = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "40000003800000".into());
    let top: Vec<u16> = text.chars().map(seg16).collect();
    let bottom: Vec<u16> = score.chars().map(seg7).collect();

    let cell: u32 = std::env::args()
        .nth(4)
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let columns = top.len().max(bottom.len());
    let style = Style {
        columns,
        ..Style::default()
    };
    // Two rows, each a cell wide and two and a bit tall.
    let size = (cell * columns as u32, cell * 5);
    let r = draw(
        &[
            Row {
                segments: &top,
                glyph: Glyph::Alphanumeric,
            },
            Row {
                segments: &bottom,
                glyph: Glyph::Numeric,
            },
        ],
        size,
        style,
    );

    // Composited on the dark the display actually sits against. Looking at it
    // over white makes the unlit strokes read as black lines and the whole
    // thing as unreadable, which is a fact about the paper and not about the
    // display.
    let mut flat = Vec::with_capacity(r.rgba.len());
    for px in r.rgba.as_chunks::<4>().0 {
        let a = f32::from(px[3]) / 255.0;
        for c in 0..3 {
            let back = [12u8, 10, 14][c];
            flat.push((f32::from(px[c]) * a + f32::from(back) * (1.0 - a)) as u8);
        }
        flat.push(255);
    }

    image::save_buffer(&out, &flat, r.width, r.height, image::ColorType::Rgba8)
        .expect("could not write the png");
    println!("{}x{} -> {out}", r.width, r.height);
}
