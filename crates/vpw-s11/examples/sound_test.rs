//! Sends sound numbers to the board and measures what comes out.
//!
//!     cargo run -p vpw-s11 --example sound_test -- f14_l1.zip [command]
//!
//! Without a command it sweeps from 0 to 255. The sound board is independent
//! of the game board, so it can be tested on its own.

use std::collections::BTreeMap;
use std::io::Read;

use vpw_s11::SoundBoard;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: sound_test <rom.zip> [command]");
        std::process::exit(2);
    };
    let roms = read_zip(&path);
    let u21 = find(&roms, "u21").expect("u21 is missing");
    let u22 = find(&roms, "u22").expect("u22 is missing");

    let only: Option<u8> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let commands: Vec<u8> = match only {
        Some(c) => vec![c],
        None => (0..=255).collect(),
    };

    // Baseline: how much the board moves if nothing is sent to it.
    let (base_dac, base_speech, base_rms) = measure(u21, u22, None);
    println!(
        "without a command: {base_dac} DAC writes, {base_speech} speech bits, RMS {base_rms:.4}"
    );
    println!();
    println!("command | DAC writes     | speech bits | audio RMS");
    println!("--------+----------------+-------------+-------------");

    for command in commands {
        let (dac, speech, rms) = measure(u21, u22, Some(command));
        // Show only what stands out from the baseline.
        if dac > base_dac * 2 + 50 || speech > base_speech * 2 + 50 {
            println!("   {command:>3}  |   {dac:>12}  |  {speech:>9}  |  {rms:.4}");
        }
    }
}

/// Runs the board for half a second and returns
/// `(DAC writes, speech bits, audio RMS)`.
fn measure(u21: &[u8], u22: &[u8], command: Option<u8>) -> (u64, u64, f32) {
    let mut board = SoundBoard::new();
    board.load_roms(u21, u22);
    board.reset();

    // Let it boot before talking to it: otherwise what gets measured is its
    // initialisation.
    board.run_until(200_000);
    let (dac_before, speech_before) = (board.bus.dac_writes(), board.bus.cvsd_bits());

    if let Some(c) = command {
        board.send_command(c);
    }

    let mut sum = 0.0f64;
    let mut n = 0u64;
    let target = board.cycle() + 500_000;
    while board.cycle() < target {
        board.step();
        let m = f64::from(board.sample());
        sum += m * m;
        n += 1;
    }

    (
        board.bus.dac_writes() - dac_before,
        board.bus.cvsd_bits() - speech_before,
        (sum / n as f64).sqrt() as f32,
    )
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

fn find<'a>(roms: &'a BTreeMap<String, Vec<u8>>, needle: &str) -> Option<&'a Vec<u8>> {
    roms.iter()
        .find(|(n, _)| n.contains(needle))
        .map(|(_, d)| d)
}
