//! Booting real WPC ROMs.
//!
//! The ROMs are not in the repo, so these tests skip themselves unless they
//! are told where the ROMs are:
//!
//! ```text
//! VPW_ROM_DIR=/path/to/the/roms cargo test --release -p vpw-wpc -- --nocapture
//! ```

use std::io::Read;
use vpw_wpc::Wpc;

/// Loads the image of a set according to the manifest.
fn load(dir: &std::path::Path, set: &str, file_name: &str) -> Option<Vec<u8>> {
    let file = std::fs::File::open(dir.join(format!("{set}.zip"))).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).ok()?;
        if e.is_file() && e.name().to_ascii_lowercase() == file_name {
            let mut data = Vec::new();
            e.read_to_end(&mut data).ok()?;
            return Some(data);
        }
    }
    None
}

fn rom_dir() -> Option<std::path::PathBuf> {
    std::env::var("VPW_ROM_DIR")
        .ok()
        .map(std::path::PathBuf::from)
}

#[test]
fn a_real_rom_draws_its_boot_message() {
    let Some(dir) = rom_dir() else {
        eprintln!("skipped: set VPW_ROM_DIR to run this test");
        return;
    };
    // Terminator 2, which is one of the most common ones to have.
    let Some(image) = load(&dir, "t2_l8", "t2_l8.rom") else {
        eprintln!("skipped: t2_l8 is not in {dir:?}");
        return;
    };

    let mut wpc = Wpc::new();
    wpc.load_rom(&image).unwrap();
    wpc.reset();
    assert_eq!(wpc.cpu.pc, 0x8C8B, "reset vector");

    wpc.run_seconds(5.0);

    assert_eq!(wpc.cpu.last_illegal, None, "it did not derail");
    assert!(
        wpc.bus.asic.periodic_irq_enabled(),
        "the ROM should have enabled the periodic interrupt"
    );
    assert!(
        wpc.bus.dmd.lit_pixels() > 100,
        "it should be drawing on the display ({} dots)",
        wpc.bus.dmd.lit_pixels()
    );

    // The message of a just-powered-on machine, drawn dot by dot.
    let screen = wpc.bus.dmd.render_text();
    assert!(
        screen.lines().filter(|l| l.contains('#')).count() >= 8,
        "the text should take up several rows of dots"
    );
}

#[test]
fn the_wpc_manifest_boots_almost_entirely() {
    let Some(dir) = rom_dir() else {
        eprintln!("skipped: set VPW_ROM_DIR to run this test");
        return;
    };
    let manifest = include_str!("data/rom_sets.tsv");

    let (mut tested, mut clean, mut with_dmd) = (0usize, 0usize, 0usize);
    let mut problems = Vec::new();

    for line in manifest.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 2 {
            continue;
        }
        let (set, file_name) = (fields[0], fields[1]);
        let Some(image) = load(&dir, set, file_name) else {
            continue;
        };

        let mut wpc = Wpc::new();
        if wpc.load_rom(&image).is_err() {
            continue;
        }
        wpc.reset();

        let mut seen = std::collections::HashSet::new();
        let target = wpc.cycle() + 8_000_000; // four seconds
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
        if wpc.cpu.last_illegal.is_none() && seen.len() >= 500 {
            clean += 1;
        } else {
            problems.push(set.to_string());
        }
    }

    if tested == 0 {
        eprintln!("skipped: there was no WPC set at all in {dir:?}");
        return;
    }

    let percentage = clean * 100 / tested;
    eprintln!(
        "{clean} of {tested} sets clean ({percentage}%), {with_dmd} with something on the display"
    );
    if !problems.is_empty() {
        eprintln!("with problems: {problems:?}");
    }

    // The ones that do not boot are modified ROMs, prototypes and third-party
    // conversions (Ultrapin, Pinball FX), not factory versions.
    assert!(
        percentage >= 95,
        "only {percentage}% booted clean; something broke ({problems:?})"
    );
}
