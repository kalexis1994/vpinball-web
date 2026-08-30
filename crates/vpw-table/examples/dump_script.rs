//! Prints a table's script, so a line number in an error message can be read.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_script <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    print!("{}", vpx.gamedata.code.string);
}
