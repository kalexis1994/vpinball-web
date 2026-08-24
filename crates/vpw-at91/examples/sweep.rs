//! Asks the sound board for one sound after another and says which ones it plays.
//!
//!     cargo run --release -p vpw-at91 --example sweep -- lotr.zip [first] [last] [seconds]
//!
//! The board takes a while to boot — it flashes its LED eight times, a second
//! each — so the sweep starts after that and gives every command a fifth of a
//! second to make a noise. What comes out is a list of which command numbers
//! this game's firmware answers to, which is the thing that says whether the
//! command path works at all.

use std::io::Read;

use vpw_arm7::Cpu;
use vpw_at91::Sound;

const CLOCK: u64 = vpw_at91::board::CLOCK_HZ as u64;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: sweep <rom.zip> [first] [last]");
    let first: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let last: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(255);
    // A fifth of a second is enough to tell whether a command does anything;
    // hearing what it does takes longer, so it is asked for.
    let each: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.2);

    let mut board = Box::new(Sound::new());
    let roms = read_zip(&path);
    let want = |part: &str| -> Vec<u8> {
        roms.iter()
            .find(|(n, _)| n.to_ascii_lowercase().contains(part))
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| panic!("no image matching {part:?}"))
    };
    board.load_bios(&want("bios.u8"));
    board.load_u7(&want("u7"));
    for (i, part) in ["u17", "u21", "u36", "u37"].iter().enumerate() {
        board.load_samples(i, &want(part));
    }

    let mut cpu = Cpu::new();
    cpu.reset();
    let mut cycles = 0u64;
    // Drained as it goes, because the board only keeps a third of a second and
    // throws away what a host has not collected.
    let run = |cpu: &mut Cpu, board: &mut Sound, until: u64, cycles: &mut u64| {
        let mut heard = Vec::new();
        while *cycles < until {
            let n = cpu.step(board);
            *cycles += u64::from(n);
            cpu.irq = board.tick(n);
            cpu.fiq = board.fiq();
            if board.queued() > 4096 {
                heard.append(&mut board.take_audio());
            }
        }
        heard.append(&mut board.take_audio());
        heard
    };

    // Past the eight flashes of the boot test and its startup tone.
    let settled = CLOCK * 11 / 2;
    run(&mut cpu, &mut board, settled, &mut cycles);
    println!("booted; sweeping {first:#04x} to {last:#04x}\n");

    // Long enough to be worth listening to, for whichever turns out loudest.
    let slice = (CLOCK as f64 * each) as u64;
    let mut answered = 0;
    let mut loudest: (i32, u32, Vec<(i16, i16)>) = (0, 0, Vec::new());
    for command in first..=last.min(255) {
        board.send(command as u8);
        let until = cycles + slice;
        let audio = run(&mut cpu, &mut board, until, &mut cycles);
        let peak = audio
            .iter()
            .map(|&(l, r)| i32::from(l).abs().max(i32::from(r).abs()))
            .max()
            .unwrap_or(0);
        if peak > loudest.0 {
            loudest = (peak, command, audio.clone());
        }
        if peak > 0 || last == first {
            answered += usize::from(peak > 0);
            println!(
                "  {command:#04x}  peak {peak:>6} of 32767  ({} samples)",
                audio.len()
            );
        }
        // Back to a byte the firmware will not mistake for a repeat.
        board.send(0x00);
        let until = cycles + slice / 4;
        run(&mut cpu, &mut board, until, &mut cycles);
    }
    println!(
        "\n{answered} of {} commands made a noise",
        last.min(255) - first + 1
    );
    if !loudest.2.is_empty() {
        let mut bytes = Vec::with_capacity(loudest.2.len() * 4);
        for (l, r) in &loudest.2 {
            bytes.extend_from_slice(&l.to_le_bytes());
            bytes.extend_from_slice(&r.to_le_bytes());
        }
        let rate = (loudest.2.len() as f64 / each) as u32;
        let name = format!("command-{:02x}.wav", loudest.1);
        write_wav(&name, &bytes, rate);
        println!("wrote {name}: the loudest one, {rate} Hz");
    }
}

/// Two channels of sixteen-bit samples, in the only container everything reads.
fn write_wav(path: &str, samples: &[u8], rate: u32) {
    let mut out = Vec::with_capacity(samples.len() + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    let _ = std::fs::write(path, out);
}

fn read_zip(path: &str) -> std::collections::BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(path).expect("could not open the zip");
    let mut zip = zip::ZipArchive::new(file).expect("not a zip");
    let mut out = std::collections::BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("entry");
        if entry.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("decompress");
        out.insert(entry.name().to_string(), data);
    }
    out
}
