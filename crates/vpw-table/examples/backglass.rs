//! Writes a head's artwork to a PNG, so it can be looked at.
//!
//! The one thing a test cannot tell you about a picture is whether it is any
//! good. The tests hold the parts that have a right answer — the frame is
//! grey, the score window is dark, the palette is the table's colours and not
//! its plywood — and this is for the part that does not.
//!
//! ```text
//! cargo run -p vpw-table --example backglass -- out.png [table.vpx]
//! ```
//!
//! With a table it pulls the finished sheet straight out of that table's
//! scene, which is the same picture the player will hang on the machine.
//! Without one it paints the house palette.

use vpw_table::backglass::{BACKGLASS_IMAGE, BACKGLASS_PIXELS, Palette, paint};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "backglass.png".into());

    let px = match args.next() {
        Some(path) => {
            let bytes = std::fs::read(&path).expect("read the table");
            let vpx = vpin::vpx::from_bytes(&bytes).expect("parse the table");
            let scene = vpw_table::geometry::extract(&vpx);
            scene
                .images
                .iter()
                .find(|i| i.name == BACKGLASS_IMAGE)
                .and_then(|i| i.rgba.clone())
                .expect("the scene carries a painted backglass")
        }
        None => {
            let t = std::time::Instant::now();
            let px = paint(&Palette::fallback());
            println!("painted in {:.1} ms", t.elapsed().as_secs_f32() * 1000.0);
            px
        }
    };

    image::save_buffer(
        &out,
        &px,
        BACKGLASS_PIXELS.0,
        BACKGLASS_PIXELS.1,
        image::ColorType::Rgba8,
    )
    .expect("write the picture");
    println!("wrote {out}");
}
