//! Parses the VBScript inside a `.vpx` and reports what happened.
//!
//! ```text
//! cargo run --release -p vpw-vbscript --example parse_table -- table.vpx
//! ```
//!
//! The point is the real thing: a published table's script, with whatever its
//! author wrote in it, rather than a snippet chosen because it parses.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: parse_table <table.vpx>");
    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let code = vpx.gamedata.code.string;

    println!(
        "table   {}",
        vpx.info.table_name.clone().unwrap_or_default()
    );
    println!(
        "script  {} lines, {} bytes",
        code.lines().count(),
        code.len()
    );

    match vpw_vbscript::parser::parse(&code) {
        Err(e) => {
            println!("PARSE FAILED: {e}");
            if let Some(line) = e.line {
                let from = line.saturating_sub(3).max(1);
                for (n, text) in code.lines().enumerate() {
                    let n = n as u32 + 1;
                    if n >= from && n <= line + 2 {
                        let mark = if n == line { ">>" } else { "  " };
                        println!("{mark}{n:>6}: {text}");
                    }
                }
            }
            std::process::exit(1);
        }
        Ok(program) => {
            let mut subs = 0;
            let mut functions = 0;
            let mut classes = 0;
            let mut handlers = Vec::new();
            for s in &program.body {
                match &s.kind {
                    vpw_vbscript::ast::StmtKind::Proc(p) => {
                        if p.is_function {
                            functions += 1;
                        } else {
                            subs += 1;
                        }
                        // A handler is a procedure named `Object_Event`, which
                        // is how a table says what it wants to be told about.
                        if p.name.contains('_') {
                            handlers.push(p.name.to_string());
                        }
                    }
                    vpw_vbscript::ast::StmtKind::Class(_) => classes += 1,
                    _ => {}
                }
            }
            println!("parsed  ok");
            println!("  subs       {subs}");
            println!("  functions  {functions}");
            println!("  classes    {classes}");
            println!("  handlers   {}", handlers.len());
            for h in handlers.iter().take(25) {
                println!("    {h}");
            }
            if handlers.len() > 25 {
                println!("    ... and {} more", handlers.len() - 25);
            }
        }
    }
}
