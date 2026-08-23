//! Prints the lines of a `.vpx`'s VBScript that match a pattern.
//!
//!     cargo run -p vpw-table --example script -- table.vpx controller.vbs

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: script <file.vpx> [pattern]");
    let needle = args.next().unwrap_or_default().to_ascii_lowercase();

    let bytes = std::fs::read(&path).expect("could not read");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse");
    let code = vpx.gamedata.code.string;

    for (n, line) in code.lines().enumerate() {
        if needle.is_empty() || line.to_ascii_lowercase().contains(&needle) {
            println!("{:>5}: {}", n + 1, line);
        }
    }
}
