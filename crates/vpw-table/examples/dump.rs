//! Dumps the summary of one or more `.vpx` files. A development tool.
//!
//!     cargo run -p vpw-table --example dump -- table.vpx [other.vpx ...]

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: dump <file.vpx> [...]");
        std::process::exit(2);
    }

    for path in paths {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        println!("=== {name} ===");

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("  could not open: {e}\n");
                continue;
            }
        };

        match vpw_table::summarize(&bytes) {
            Ok(t) => {
                println!("  name         {:?}", t.table_name);
                println!("  author       {:?}", t.author);
                println!("  version      {:?}", t.version);
                println!("  format       {}", t.file_version);
                println!("  items        {}", t.gameitem_count);
                println!("  images       {}", t.image_count);
                println!("  sounds       {}", t.sound_count);
                println!("  script       {} bytes", t.script_len);
                println!(
                    "  screenshot   {}",
                    t.screenshot
                        .map_or("no".into(), |s| format!("yes, {} bytes", s.len()))
                );
                match &t.rom {
                    vpw_table::RomRequirement::NotNeeded => println!("  ROM          not needed"),
                    vpw_table::RomRequirement::Required {
                        game_name,
                        alternates,
                    } => {
                        println!("  ROM          {game_name}  -> {game_name}.zip");
                        if !alternates.is_empty() {
                            println!("  alternates   {alternates:?}");
                        }
                    }
                    vpw_table::RomRequirement::RequiredUnknown => {
                        println!("  ROM          uses VPinMAME, name not detected")
                    }
                }
            }
            Err(e) => println!("  ERROR: {e}"),
        }
        println!();
    }
}
