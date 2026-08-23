//! Boots many WPC sets and reports which ones run clean.
//!
//!     cargo run --release -p vpw-wpc --example wpc_batch -- <list.txt> <dir> [seconds]
//!
//! The list is the TSV at `tests/data/rom_sets.tsv`, with the exact image name
//! of each set. The name is needed and "the biggest one in the zip" is not
//! enough: several games ship one-megabyte sound ROMs next to a half-megabyte
//! game, and grabbing the wrong one makes the CPU derail in very unobvious
//! places.

use std::io::Read;
use vpw_wpc::Wpc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: wpc_batch <list.txt> <directory> [seconds]");
        std::process::exit(2);
    }
    let list = std::fs::read_to_string(&args[1]).expect("could not read the list");
    let dir = std::path::Path::new(&args[2]);
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let target = (seconds * 2_000_000.0) as u64;

    let (mut clean, mut tested) = (0usize, 0usize);
    let mut with_dmd = 0usize;
    let mut problems: Vec<(String, String)> = Vec::new();

    for line in list.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 2 {
            continue;
        }
        let (set, file_name) = (fields[0], fields[1]);

        let path = dir.join(format!("{set}.zip"));
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };

        let mut image: Option<Vec<u8>> = None;
        for i in 0..zip.len() {
            let Ok(mut e) = zip.by_index(i) else { continue };
            if !e.is_file() || e.name().to_ascii_lowercase() != file_name {
                continue;
            }
            let mut data = Vec::new();
            if e.read_to_end(&mut data).is_ok() {
                image = Some(data);
            }
        }
        let Some(image) = image else { continue };

        let mut wpc = Wpc::new();
        if wpc.load_rom(&image).is_err() {
            continue;
        }
        wpc.reset();

        let mut seen = std::collections::HashSet::new();
        while wpc.cycle() < target {
            wpc.step();
            seen.insert(wpc.cpu.pc);
            if wpc.cpu.last_illegal.is_some() {
                break;
            }
        }

        tested += 1;
        if wpc.bus.dmd.lit_pixels() > 0 {
            with_dmd += 1;
        }

        if let Some((pc, op)) = wpc.cpu.last_illegal {
            problems.push((set.to_string(), format!("opcode {op:#04X} at {pc:#06X}")));
        } else if seen.len() < 500 {
            problems.push((set.to_string(), format!("only {} addresses", seen.len())));
        } else {
            clean += 1;
        }
    }

    println!("{clean} of {tested} WPC sets booted with no undefined opcodes");
    println!("{with_dmd} of {tested} drew something on the dot matrix display");

    if !problems.is_empty() {
        println!();
        println!("with problems ({}):", problems.len());
        for (set, reason) in problems.iter().take(20) {
            println!("  {set:<14} {reason}");
        }
        if problems.len() > 20 {
            println!("  ... and {} more", problems.len() - 20);
        }
    }
}
