//! Parses every `.vbs` in a directory and reports what failed.
//!
//! ```text
//! cargo run -p vpw-vbscript --example parse_corpus -- ../vpinball/scripts
//! ```
fn main() {
    let dir = std::env::args().nth(1).expect("usage: parse_corpus <dir>");
    let mut ok = 0;
    let mut failed = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("could not read the directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vbs")))
        .collect();
    entries.sort();

    for path in &entries {
        let src = std::fs::read(path).expect("could not read the file");
        let src = String::from_utf8_lossy(&src);
        match vpw_vbscript::parser::parse(&src) {
            Ok(_) => ok += 1,
            Err(e) => failed.push((path.clone(), e)),
        }
    }
    println!("parsed {ok} of {}", entries.len());
    for (p, e) in &failed {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        println!("  {name}: {e}");
    }
}
