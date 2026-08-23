//! The parser, against every script Visual Pinball ships.
//!
//! Seventy files and twelve thousand lines of VBScript written by other people
//! over twenty years, including `core.vbs` — the three-thousand-line library
//! that every table loads. It is the only test here that can say the parser
//! handles *real* VBScript rather than the VBScript I happened to think of.
//!
//! The scripts are not in this repository: they belong to Visual Pinball, which
//! is cloned next to it as the port's reference. If it is not there the tests
//! skip themselves, the same way the tests that need a real ROM do.
//!
//! ```text
//! git clone --filter=blob:none --sparse https://github.com/vpinball/vpinball ../vpinball
//! git -C ../vpinball sparse-checkout add /scripts
//! ```

use std::path::{Path, PathBuf};

/// Where the reference clone lives, relative to this crate.
const SCRIPTS: &str = "../../../vpinball/scripts";

fn scripts() -> Option<Vec<PathBuf>> {
    let dir = Path::new(SCRIPTS);
    if !dir.is_dir() {
        eprintln!("skipped: {SCRIPTS} is not there");
        return None;
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vbs")))
        .collect();
    files.sort();
    if files.is_empty() {
        eprintln!("skipped: no .vbs files in {SCRIPTS}");
        return None;
    }
    Some(files)
}

/// Reads a script. They are not all UTF-8 — some carry a stray byte in a
/// comment — and the real engine does not care, so neither does this.
fn read(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("could not read the script");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn every_script_visual_pinball_ships_parses() {
    let Some(files) = scripts() else { return };

    let mut failures = Vec::new();
    for path in &files {
        if let Err(e) = vpw_vbscript::parser::parse(&read(path)) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            failures.push(format!("{name}: {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} scripts failed to parse:\n  {}",
        failures.len(),
        files.len(),
        failures.join("\n  ")
    );
}

#[test]
fn the_corpus_is_big_enough_to_mean_something() {
    // A guard against the test above quietly passing on an empty or truncated
    // checkout and looking like coverage it does not have.
    let Some(files) = scripts() else { return };
    let lines: usize = files.iter().map(|p| read(p).lines().count()).sum();
    assert!(
        files.len() >= 50 && lines >= 10_000,
        "the corpus looks truncated: {} files, {lines} lines",
        files.len()
    );
}

#[test]
fn core_vbs_parses_and_declares_what_tables_expect() {
    // `core.vbs` is the library every table loads. If it parses but comes out
    // empty, the test above would still pass, so this one checks that the tree
    // actually has the things in it that tables reach for.
    let Some(files) = scripts() else { return };
    let Some(core) = files.iter().find(|p| {
        p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("core.vbs"))
    }) else {
        eprintln!("skipped: core.vbs is not in the checkout");
        return;
    };

    let program = vpw_vbscript::parser::parse(&read(core)).expect("core.vbs has to parse");

    let mut classes = Vec::new();
    let mut procs = Vec::new();
    for s in &program.body {
        match &s.kind {
            vpw_vbscript::ast::StmtKind::Class(c) => classes.push(c.name.to_string()),
            vpw_vbscript::ast::StmtKind::Proc(p) => procs.push(p.name.to_string()),
            _ => {}
        }
    }

    // The classes tables build their rules on.
    for wanted in ["cvpmTimer", "cvpmBallStack", "cvpmDropTarget"] {
        assert!(
            classes.iter().any(|c| c.eq_ignore_ascii_case(wanted)),
            "core.vbs should declare class {wanted}; found {classes:?}"
        );
    }
    // And the entry point every table calls.
    assert!(
        procs.iter().any(|p| p.eq_ignore_ascii_case("vpmInit")),
        "core.vbs should declare vpmInit"
    );
    assert!(
        classes.len() >= 10,
        "core.vbs declares more than {} classes",
        classes.len()
    );
}

/// A published table, if one has been put where the other tests look for it.
///
/// The scripts Visual Pinball ships are written by its own maintainers and are
/// tidier than what is out in the wild. A real table's script is written by
/// whoever made the table, over years, with the editor of the day — which is
/// how we found that Terminator 2's is stored with bare carriage returns as
/// line endings, and that treating those as whitespace collapses the file onto
/// one line.
const TABLE: &str = "../../web/debug-assets/f14.vpx";

#[test]
fn a_real_tables_script_parses() {
    let path = std::path::Path::new(TABLE);
    if !path.is_file() {
        eprintln!("skipped: {TABLE} is not there");
        return;
    }
    let bytes = std::fs::read(path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let code = vpx.gamedata.code.string;

    let program = match vpw_vbscript::parser::parse(&code) {
        Ok(p) => p,
        Err(e) => panic!("the table's script failed to parse: {e}"),
    };

    // And it has to contain what a table's script always contains: the
    // handlers the player is going to call.
    let handlers: Vec<String> = program
        .body
        .iter()
        .filter_map(|s| match &s.kind {
            vpw_vbscript::ast::StmtKind::Proc(p) => Some(p.name.to_string()),
            _ => None,
        })
        .collect();
    for wanted in ["Table1_Init", "Table1_KeyDown", "Table1_KeyUp"] {
        assert!(
            handlers.iter().any(|h| h.eq_ignore_ascii_case(wanted)),
            "a table's script should define {wanted}"
        );
    }
    assert!(
        handlers.len() > 50,
        "only {} procedures; that does not look like a real table",
        handlers.len()
    );
}
