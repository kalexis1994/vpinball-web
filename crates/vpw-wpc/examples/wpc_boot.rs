//! Boots a WPC ROM and reports how far it gets.
//!
//!     cargo run --release -p vpw-wpc --example wpc_boot -- tz_92.zip [seconds]

use std::io::Read;
use vpw_wpc::Wpc;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: wpc_boot <rom.zip> [seconds]");
        std::process::exit(2);
    };
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);

    let file = std::fs::File::open(&path).expect("could not open the zip");
    let mut zip = zip::ZipArchive::new(file).expect("not a valid zip");

    // The game ROM is the largest image that is a power of two.
    let mut best: Option<(String, Vec<u8>)> = None;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        if !e.is_file() {
            continue;
        }
        let name = e.name().to_string();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        if data.len().is_power_of_two()
            && data.len() >= 0x2_0000
            && best.as_ref().is_none_or(|(_, d)| data.len() > d.len())
        {
            best = Some((name, data));
        }
    }
    let (name, image) = best.expect("there is no usable ROM image");
    println!("{path}");
    println!("  image: {name} ({} KB)", image.len() / 1024);

    let mut wpc = Wpc::new();
    wpc.load_rom(&image).expect("the image is not valid");
    wpc.reset();
    println!("  reset vector -> {:#06X}", wpc.cpu.pc);
    println!();

    let mut seen = std::collections::HashSet::new();
    let mut led_changes = 0u32;
    let mut led = false;
    let mut irq_enabled_at = None;

    let target = (seconds * 2_000_000.0) as u64;
    while wpc.cycle() < target {
        wpc.step();
        seen.insert(wpc.cpu.pc);
        if irq_enabled_at.is_none() && wpc.bus.asic.periodic_irq_enabled() {
            irq_enabled_at = Some(wpc.cycle());
        }
        let now = wpc.bus.asic.diagnostic_led();
        if now != led {
            led_changes += 1;
            led = now;
        }
        // The 6809 has no HCF; the equivalent is derailing into an opcode that
        // does not exist.
        if wpc.cpu.last_illegal.is_some() {
            break;
        }
    }

    println!("--- after {:.2} s simulated ---", wpc.elapsed_seconds());
    println!("  PC                 {:#06X}", wpc.cpu.pc);
    println!("  addresses seen     {}", seen.len());
    println!("  undefined opcode   {:?}", wpc.cpu.last_illegal);
    println!(
        "  ROM bank           {:#04X}",
        wpc.bus.asic.register(vpw_wpc::registers::ROMBANK)
    );
    println!(
        "  periodic IRQ       {}",
        match irq_enabled_at {
            Some(c) => format!("enabled at cycle {c}"),
            None => "never enabled".into(),
        }
    );
    println!("  diagnostic LED changed {led_changes} times");
    println!("  solenoids          {:#034b}", wpc.bus.asic.solenoids());
    println!("  DMD frames         {}", wpc.bus.dmd.frames());
    println!("  lit dots           {} of 4096", wpc.bus.dmd.lit_pixels());

    if std::env::args().any(|a| a == "dmd") {
        println!();
        println!("+{}+", "-".repeat(128));
        for line in wpc.bus.dmd.render_text().lines() {
            println!("|{line}|");
        }
        println!("+{}+", "-".repeat(128));
    }
}
