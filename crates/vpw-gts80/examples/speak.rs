//! Spells words at the Votrax and writes what it says to a WAV.
//!
//! ```text
//! cargo run -p vpw-gts80 --example speak -- out.wav [B L AE K PA0 H O1 OO1 L]
//! ```
//!
//! No ROM needed: this drives the speech chip the way a sound board's firmware
//! would, one phoneme at a time, and is here so the voice can be listened to
//! rather than argued about.

use vpw_gts80::votrax::{NOMINAL_CLOCK, Votrax};

const RATE: u32 = 48_000;

/// "BLACK HOLE", which is what a Gottlieb from 1981 says most often.
const DEFAULT: &[&str] = &[
    "B", "L", "AE", "K", "PA0", "H", "O1", "OO1", "L", "PA1", "TH", "E", "PA0", "B", "AW1", "L",
    "PA0", "I", "Z", "PA0", "L", "AH1", "S", "T", "PA1",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("somewhere to write the wav");
    let spelled: Vec<String> = args.collect();
    let words: Vec<&str> = if spelled.is_empty() {
        DEFAULT.to_vec()
    } else {
        spelled.iter().map(String::as_str).collect()
    };

    let mut chip = Votrax::new(RATE);
    let mut samples = Vec::new();

    // The chip's own clock, which the board would be setting with its second
    // converter. `$7E` is the middle of the range and gives 720 kHz.
    chip.set_clock(0x7E);

    for name in &words {
        let Some(code) = vpw_gts80::votrax::code_of(name) else {
            eprintln!("no phoneme called {name:?}");
            continue;
        };
        chip.write(code);
        // A firmware waits on the busy line; so does this.
        while chip.busy() {
            samples.push(chip.sample());
        }
        // And the tail of it, while the mouth closes.
        for _ in 0..(RATE / 100) {
            samples.push(chip.sample());
        }
    }

    // And the same again slowed down, which is how these machines say "TILT":
    // the clock falls and the voice goes with it.
    for latch in [0x7E, 0x74, 0x6A, 0x60, 0x56, 0x4C, 0x46] {
        chip.set_clock(latch);
        for name in ["T", "I", "L", "T", "PA1"] {
            let code = vpw_gts80::votrax::code_of(name).expect("a phoneme");
            chip.write(code);
            while chip.busy() {
                samples.push(chip.sample());
            }
        }
    }

    println!(
        "{} phonemes, {:.1} s at {} Hz, nominal clock {NOMINAL_CLOCK} Hz",
        words.len(),
        samples.len() as f32 / RATE as f32,
        RATE
    );
    std::fs::write(&path, wave(&samples, RATE)).expect("the wav will not write");
    println!("wrote {path}");
}

/// Mono sixteen-bit PCM, which is the least a player will open.
fn wave(samples: &[f32], rate: u32) -> Vec<u8> {
    let bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}
