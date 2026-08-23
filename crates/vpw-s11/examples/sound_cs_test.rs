//! Boots the CS sound board and sends it sound numbers.
//!
//!     cargo run -p vpw-s11 --example sound_cs_test -- f14_l1.zip [command]

use std::collections::BTreeMap;
use std::io::Read;

use vpw_s11::CsSoundBoard;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: sound_cs_test <rom.zip> [command]");
        std::process::exit(2);
    };
    let roms = read_zip(&path);
    let u4 = find(&roms, "u4").expect("u4 is missing");
    let u19 = find(&roms, "u19").expect("u19 is missing");
    println!("u4: {} bytes, u19: {} bytes", u4.len(), u19.len());

    let mut board = CsSoundBoard::new();
    assert!(board.load_roms(&[u4, u19]), "the images are not 32 KB");
    board.reset();
    println!("reset vector -> {:#06X}", board.cpu.pc);
    println!();

    let only: Option<u8> = std::env::args().nth(2).and_then(|s| s.parse().ok());

    // A dry boot.
    board.run_until(500_000);
    println!("after booting:");
    println!("  PC                {:#06X}", board.cpu.pc);
    println!("  undefined opcode  {:?}", board.cpu.last_illegal);
    println!("  outside the map   {:?}", board.bus.last_unmapped);
    println!("  DAC writes        {}", board.bus.dac_writes());
    println!("  speech bits       {}", board.bus.cvsd_bits());
    println!("  FM operators      {}", board.bus.ym.active_operators());
    println!();

    let commands: Vec<u8> = match only {
        Some(c) => vec![c],
        None => (0..=255).collect(),
    };

    println!("command | DAC  | speech| FM operators  | RMS    | LFO");
    println!("--------+------+-------+---------------+--------+-----");
    for command in commands {
        let mut board = CsSoundBoard::new();
        board.load_roms(&[u4, u19]);
        board.reset();
        board.run_until(1_000_000);

        let before = (board.bus.dac_writes(), board.bus.cvsd_bits());
        board.send_command(command);

        let mut fm_max = 0;
        let mut sum = 0.0f64;
        let mut n = 0u64;
        let target = board.cycle() + 1_000_000;
        while board.cycle() < target {
            board.step();
            fm_max = fm_max.max(board.bus.ym.active_operators());
            let m = f64::from(board.sample());
            sum += m * m;
            n += 1;
        }

        let dac = board.bus.dac_writes() - before.0;
        let speech = board.bus.cvsd_bits() - before.1;
        let rms = (sum / n as f64).sqrt();
        if dac > 100 || speech > 100 || fm_max > 0 {
            let lfo = board.bus.ym.register(0x1B) & 3;
            let amd = board.bus.ym.register(0x19);
            println!(
                "   {command:>3}  | {dac:>4} | {speech:>5} | {fm_max:>13} | {rms:.4} | LFO wave {lfo} depth {amd:#04X}"
            );
        }
    }
}

fn read_zip(path: &str) -> BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(path).expect("could not open the zip");
    let mut archive = zip::ZipArchive::new(file).expect("not a valid zip");
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        if entry.is_file() {
            let name = entry.name().to_ascii_lowercase();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            out.insert(name, data);
        }
    }
    out
}

fn find<'a>(roms: &'a BTreeMap<String, Vec<u8>>, needle: &str) -> Option<&'a [u8]> {
    roms.iter()
        .find(|(n, _)| n.contains(needle))
        .map(|(_, d)| d.as_slice())
}
