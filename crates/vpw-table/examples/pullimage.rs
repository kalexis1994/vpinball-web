//! Writes one of a table's images out as a file, to look at it.
//!
//! ```text
//! cargo run -p vpw-table --example pullimage -- table.vpx <image name> out.<ext>
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let (table, name, out) = (
        args.next().expect("table.vpx"),
        args.next().expect("image name"),
        args.next().expect("output path"),
    );
    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let scene = vpw_table::geometry::extract(&vpx);
    let img = scene
        .image(&name)
        .unwrap_or_else(|| panic!("no image called {name}"));
    match (&img.encoded, &img.rgba) {
        (Some(enc), _) => {
            std::fs::write(&out, enc).expect("could not write");
            println!("wrote {} bytes of the encoded picture ({}x{})", enc.len(), img.width, img.height);
        }
        (None, Some(raw)) => {
            println!("raw RGBA {}x{}, {} bytes — not written as-is", img.width, img.height, raw.len());
        }
        _ => println!("the image holds no pixels"),
    }
}
