//! Boots a real System 11 ROM and reports how far it gets.
//!
//!     cargo run -p vpw-s11 --example boot -- f14_l1.zip
//!
//! It expects a zip in PinMAME's format. It looks up the two main-CPU images
//! by name (`*u26*` and `*u27*`), which is the `S11_ROMSTART48` layout: 16 KB
//! at `$4000` and 32 KB at `$8000`.

use std::collections::BTreeMap;
use std::io::Read;

use vpw_s11::{System11, pia};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: boot <rom.zip> [seconds]");
        std::process::exit(2);
    };
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    // With `advance`, it presses the door's diagnostic button every two
    // simulated seconds. A real board that was just plugged in, with a blank
    // CMOS, sits waiting for exactly that.
    let press_advance = std::env::args().any(|a| a == "advance");

    let roms = read_zip(&path);
    println!("=== {path} ===");
    for (name, data) in &roms {
        println!("  {name:<16} {:>7} bytes", data.len());
    }
    println!();

    let u26 = find(&roms, "u26").expect("the u26 image is missing");
    let u27 = find(&roms, "u27").expect("the u27 image is missing");
    println!("loading u26 ({} KB) at $4000", u26.len() / 1024);
    println!("loading u27 ({} KB) at $8000", u27.len() / 1024);

    let mut machine = System11::new();
    machine
        .load_rom_48(u26, u27)
        .expect("the images fit in the map");

    // The sound board has its own CPU and its own ROMs.
    match (find(&roms, "u21"), find(&roms, "u22")) {
        (Some(u21), Some(u22)) => {
            if machine.load_sound_roms(u21, u22) {
                println!("XS sound board: u21 and u22 loaded");
            } else {
                println!("XS sound board: the images are not 32 KB");
            }
        }
        _ => println!("XS sound board: no images in the zip"),
    }
    match (find(&roms, "u4"), find(&roms, "u19")) {
        (Some(u4), Some(u19)) => {
            if machine.load_cs_sound_roms(&[u4, u19]) {
                println!("CS sound board: u4 and u19 loaded");
            } else {
                println!("CS sound board: the images are not 32 KB");
            }
        }
        _ => println!("CS sound board: no images in the zip"),
    }

    machine.reset();

    println!("reset vector -> {:#06X}", machine.cpu.pc);
    println!();

    if let Some(sound) = &machine.sound {
        println!("XS reset vector -> {:#06X}", sound.cpu.pc);
    }
    if let Some(sound) = &machine.sound_cs {
        println!("CS reset vector -> {:#06X}", sound.cpu.pc);
    }

    // Run it and see how far it gets.
    let mut audio_max = 0.0f32;
    let mut fm_max = 0usize;
    let mut speech_max = 0.0f32;
    let mut dac_max = 0.0f32;
    let mut nonzero_audio = 0u64;
    let mut led_changes = 0u32;
    let mut max_lamps = 0u32;
    let mut solenoids_seen = 0u32;
    let mut previous_led = machine.board.diagnostic_led();
    let mut visited: BTreeMap<u16, u64> = BTreeMap::new();

    let target = (seconds * 1_000_000.0) as u64;
    let mut next_pulse = 2_000_000u64;
    while machine.cycle() < target {
        if press_advance && machine.cycle() >= next_pulse {
            // A button pulse: press and release.
            machine.board.diagnostic_advance = !machine.board.diagnostic_advance;
            next_pulse += 250_000;
        }
        machine.step();
        *visited.entry(machine.cpu.pc).or_default() += 1;

        if let Some(cs) = &machine.sound_cs {
            fm_max = fm_max.max(cs.bus.ym.active_operators());
        }
        let sample = machine.audio_sample();
        audio_max = audio_max.max(sample.abs());
        if sample != 0.0 {
            nonzero_audio += 1;
        }
        if let Some(sound) = &machine.sound {
            speech_max = speech_max.max(sound.bus.cvsd.sample().abs());
            dac_max = dac_max.max(sound.bus.dac.sample().abs());
        }
        max_lamps = max_lamps.max(machine.board.lamps.lit_count());
        solenoids_seen |= machine.board.solenoids.fired();

        let led = machine.board.diagnostic_led();
        if led != previous_led {
            led_changes += 1;
            previous_led = led;
        }

        if machine.cpu.is_hcf() {
            println!("!! the CPU executed HCF at {:#06X}", machine.cpu.pc);
            break;
        }
        if let Some((pc, opcode)) = machine.cpu.last_illegal {
            println!("!! undefined opcode {opcode:#04X} at {pc:#06X}");
            break;
        }
    }

    println!("--- after {:.3} s simulated ---", machine.elapsed_seconds());
    println!("  cycles            {}", machine.cycle());
    println!("  PC                {:#06X}", machine.cpu.pc);
    println!("  SP                {:#06X}", machine.cpu.sp);
    println!(
        "  A / B / X         {:#04X} / {:#04X} / {:#06X}",
        machine.cpu.a, machine.cpu.b, machine.cpu.x
    );
    println!("  addresses seen    {}", visited.len());
    println!();
    println!("  undefined opcode  {:?}", machine.cpu.last_illegal);
    println!("  outside the map   {:?}", machine.board.last_unmapped);
    println!();
    println!("  ┌────────────────┐");
    println!(
        "  │ {} │  <- alphanumeric, 16 segments",
        machine.upper_display()
    );
    println!(
        "  │ {} │  <- score displays, 7 segments",
        machine.lower_display()
    );
    println!("  └────────────────┘");
    if std::env::args().any(|a| a == "segments") {
        println!("  raw segments of the bottom row:");
        for (slot, bits) in machine.board.displays.lower_segments().iter().enumerate() {
            if *bits != 0 {
                println!(
                    "    slot {slot:>2}  {bits:#018b}  -> {:?}",
                    vpw_s11::segments_to_char_7seg(*bits)
                );
            }
        }
        println!("  raw segments of the top row:");
        for (slot, bits) in machine.board.displays.upper_segments().iter().enumerate() {
            if *bits != 0 {
                println!(
                    "    slot {slot:>2}  {bits:#018b}  -> {:?}",
                    vpw_s11::segments_to_char(*bits)
                );
            }
        }
    }
    println!();
    println!(
        "  diagnostic LED    {} (changed {led_changes} times)",
        machine.board.diagnostic_led()
    );
    println!("  sound latch       {:#04X}", machine.board.sound_latch());
    println!(
        "  active solenoids  {:#018b}",
        machine.board.solenoids.active()
    );
    println!(
        "  solenoids that fired {:#018b}",
        machine.board.solenoids.fired()
    );
    println!(
        "  lamps lit         {} of 64 (peak seen: {max_lamps})",
        machine.board.lamps.lit_count()
    );
    println!("  solenoids that fired at some point {solenoids_seen:#018b}");
    println!();

    if let Some(sound) = &machine.sound {
        println!("  --- sound board ---");
        println!("  PC                 {:#06X}", sound.cpu.pc);
        println!("  cycles             {}", sound.cycle());
        println!("  undefined opcode   {:?}", sound.cpu.last_illegal);
        println!("  outside the map    {:?}", sound.bus.last_unmapped);
        println!("  DAC                {:#04X}", sound.bus.dac.level());
        println!("  audio peak         {audio_max:.4}  (DAC {dac_max:.4}, speech {speech_max:.4})");
        println!("  speech bits        {}", sound.bus.cvsd_bits());
        println!(
            "  sound PIA          DDRA {:#04X} DDRB {:#04X}  CA2 {:?} CB2 {:?}",
            sound.bus.pia.port_a_direction(),
            sound.bus.pia.port_b_direction(),
            sound.bus.pia.ca2_mode(),
            sound.bus.pia.cb2_mode()
        );
        println!("  samples with signal {nonzero_audio}");
        println!();
    }

    if let Some(cs) = &machine.sound_cs {
        println!("  --- CS sound board ---");
        println!("  PC                 {:#06X}", cs.cpu.pc);
        println!(
            "  cycles             {} (should be twice as many)",
            cs.cycle()
        );
        println!("  undefined opcode   {:?}", cs.cpu.last_illegal);
        println!("  outside the map    {:?}", cs.bus.last_unmapped);
        println!("  DAC writes         {}", cs.bus.dac_writes());
        println!("  speech bits        {}", cs.bus.cvsd_bits());
        println!("  FM operators       peak {fm_max}");
        println!();
    }
    println!(
        "  accumulated audio  {} samples ({:.2} s at 48 kHz)",
        machine.pending_audio(),
        machine.pending_audio() as f64 / 48_000.0
    );
    println!();

    println!("  PIA   DDRA DDRB  PA   PB");
    for (index, p) in machine.board.pias.iter().enumerate() {
        println!(
            "  {index} {:<12} {:#04X} {:#04X}  {:#04X} {:#04X}",
            pia_name(index),
            p.port_a_direction(),
            p.port_b_direction(),
            p.port_a_output(),
            p.port_b_output()
        );
    }

    // How much RAM it touched: if the POST ran, it should have written a fair
    // amount.
    let ram_used = machine.board.ram().iter().filter(|&&b| b != 0).count();
    println!();
    println!(
        "  RAM bytes other than zero: {ram_used} / {}",
        machine.board.ram().len()
    );

    // Where the time goes: if it is stuck, the loop stands out.
    let mut hot: Vec<_> = visited.iter().map(|(&pc, &n)| (n, pc)).collect();
    hot.sort_unstable_by(|a, b| b.cmp(a));
    println!();
    println!("  most executed addresses:");
    for (n, pc) in hot.iter().take(12) {
        println!("    {pc:#06X}  {n:>8}x");
    }
}

fn pia_name(index: usize) -> &'static str {
    match index {
        pia::SOUND_SOLENOIDS => "sound/sol",
        pia::LAMPS => "lamps",
        pia::DISPLAY_SELECT => "digit/diag",
        pia::DISPLAY_DATA => "display",
        pia::SWITCHES => "switches",
        _ => "widget",
    }
}

fn read_zip(path: &str) -> BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(path).expect("could not open the zip");
    let mut archive = zip::ZipArchive::new(file).expect("not a valid zip");
    let mut out = BTreeMap::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("unreadable entry");
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_ascii_lowercase();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("could not decompress");
        out.insert(name, data);
    }
    out
}

fn find<'a>(roms: &'a BTreeMap<String, Vec<u8>>, needle: &str) -> Option<&'a Vec<u8>> {
    roms.iter()
        .find(|(name, _)| name.contains(needle))
        .map(|(_, data)| data)
}
