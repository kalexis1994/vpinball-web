//! Boots many System 11 ROM sets and reports which ones run clean.
//!
//!     cargo run --release -p vpw-s11 --example batch -- <manifest.tsv> <dir> [seconds]
//!
//! The manifest is the TSV in `data/rom_sets.tsv`, with the columns
//! `set`, `generacion`, `display`, `carga`, `imagen1`, `imagen2`.
//!
//! It is the broadest test the emulator can be put through: if the memory map,
//! the CPU and the timing are right, hundreds of different games have to boot
//! without touching anything outside the map. It covers the three families
//! that share this hardware: System 9, System 11 and Data East.

use std::collections::BTreeMap;
use std::io::Read;

use vpw_s11::{DisplayConfig, System11};

struct Outcome {
    set: String,
    family: String,
    ok: bool,
    detail: String,
    display: String,
    addresses: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: batch <manifest.tsv> <rom directory> [seconds]");
        std::process::exit(2);
    }
    let manifest = std::fs::read_to_string(&args[1]).expect("could not read the manifest");
    let dir = std::path::Path::new(&args[2]);
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8.0);

    let mut outcomes = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in manifest.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 4 {
            continue;
        }
        if line.starts_with('#') || fields.len() < 5 {
            continue;
        }
        let (set, family, display_col, layout) = (fields[0], fields[1], fields[2], fields[3]);
        let (image1, image2) = (fields[4], fields.get(5).copied().unwrap_or(""));
        let display = match display_col {
            "s11b" => DisplayConfig::S11B,
            _ => DisplayConfig::S11A,
        };
        if !seen.insert(set.to_string()) {
            continue; // the manifest may repeat a set
        }

        let path = dir.join(format!("{set}.zip"));
        if !path.exists() {
            continue;
        }

        outcomes.push(try_set(
            set, family, &path, layout, image1, image2, display, seconds,
        ));
    }

    // --- Report ------------------------------------------------------------
    if std::env::args().any(|a| a == "detail") {
        println!("set          | family  | addr | display          | outcome");
        println!("-------------+---------+------+------------------+-------------------------");
        for r in &outcomes {
            let mark = if r.ok { "ok" } else { "FAIL" };
            println!(
                "{:<12} | {:<7} | {:>4} | {:<16} | {mark} {}",
                r.set, r.family, r.addresses, r.display, r.detail
            );
        }
        println!();
    }

    // Summary by family.
    let mut by_family: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for r in &outcomes {
        let e = by_family.entry(r.family.as_str()).or_default();
        e.1 += 1;
        if r.ok {
            e.0 += 1;
        }
    }
    println!("family  | clean   |   tested");
    println!("--------+---------+---------");
    for (family, (ok, total)) in &by_family {
        println!("{family:<7} | {ok:>7} | {total:>8}");
    }

    let clean = outcomes.iter().filter(|r| r.ok).count();
    println!();
    println!(
        "{clean} of {} sets booted with no undefined opcodes and no accesses outside the map",
        outcomes.len()
    );

    // What failed and why.
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for r in outcomes.iter().filter(|r| !r.ok) {
        let reason = r
            .detail
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        *reasons.entry(reason).or_default() += 1;
    }
    if !reasons.is_empty() {
        println!();
        println!("failure reasons:");
        let mut order: Vec<_> = reasons.into_iter().collect();
        order.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (reason, n) in order {
            println!("  {n:>3}  {reason}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_set(
    set: &str,
    family: &str,
    path: &std::path::Path,
    layout: &str,
    image1: &str,
    image2: &str,
    display: DisplayConfig,
    seconds: f64,
) -> Outcome {
    let failure = |detail: &str| Outcome {
        set: set.to_string(),
        family: family.to_string(),
        ok: false,
        detail: detail.to_string(),
        display: String::new(),
        addresses: 0,
    };

    let roms = match read_zip(path) {
        Ok(r) => r,
        Err(e) => return failure(&format!("could not open the zip: {e}")),
    };

    let Some(first) = roms.get(image1) else {
        return failure(&format!("{image1} is missing"));
    };
    let second = if image2.is_empty() {
        None
    } else {
        roms.get(image2)
    };
    if !image2.is_empty() && second.is_none() {
        return failure(&format!("{image2} is missing"));
    }

    // Each family builds its 64 KB region differently. The manifest's `carga`
    // column says which of the five layouts this set uses.
    let mut images: Vec<(usize, &[u8])> = Vec::new();
    match layout {
        "16400+32-8000" => {
            images.push((0x4000, first));
            images.push((0x8000, second.unwrap()));
        }
        "8x4000+32-8000" => {
            // The small image is mirrored to cover the gap at $6000.
            images.push((0x4000, first));
            images.push((0x6000, first));
            images.push((0x8000, second.unwrap()));
        }
        "32-0000+32-8000" => {
            images.push((0x0000, first));
            images.push((0x8000, second.unwrap()));
        }
        "32-8000" => images.push((0x8000, first)),
        "64-0000" => images.push((0x0000, first)),
        other => return failure(&format!("unknown layout: {other}")),
    }

    let mut machine = System11::new();
    if let Err(e) = machine.load_cpu_region(&images) {
        return failure(&format!("does not fit in the map: {e}"));
    }
    machine.set_display_config(display);
    machine.reset();

    let mut addresses = std::collections::HashSet::new();
    let target = (seconds * 1_000_000.0) as u64;
    while machine.cycle() < target {
        machine.step();
        addresses.insert(machine.cpu.pc);
        if machine.cpu.is_hcf() {
            break;
        }
    }

    let display = machine.upper_display().trim().to_string();
    let mut detail = String::new();
    let mut ok = true;

    if machine.cpu.is_hcf() {
        detail = format!("HCF at {:#06X}", machine.cpu.pc);
        ok = false;
    } else if let Some((pc, opcode)) = machine.cpu.last_illegal {
        detail = format!("opcode {opcode:#04X} at {pc:#06X}");
        ok = false;
    } else if let Some(access) = machine.board.last_unmapped {
        detail = format!("outside the map at {:#06X}", access.address);
        ok = false;
    } else if addresses.len() < 200 {
        detail = format!("only {} addresses: it did not boot", addresses.len());
        ok = false;
    }

    Outcome {
        set: set.to_string(),
        family: family.to_string(),
        ok,
        detail,
        display,
        addresses: addresses.len(),
    }
}

fn read_zip(path: &std::path::Path) -> std::io::Result<BTreeMap<String, Vec<u8>>> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        if entry.is_file() {
            let name = entry.name().to_ascii_lowercase();
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            out.insert(name, data);
        }
    }
    Ok(out)
}
