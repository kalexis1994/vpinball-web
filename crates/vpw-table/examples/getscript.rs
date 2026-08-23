//! Prints a table's script to standard output.
//!
//! ```text
//! cargo run -p vpw-table --example getscript -- table.vpx
//! ```
//!
//! A table's script is the half of it that is not geometry, and it is inside
//! the .vpx where no editor can reach it. Nearly every question that starts
//! "why is the table doing that" is answered by reading it.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: getscript <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    print!("{}", vpx.gamedata.code.string);
}
