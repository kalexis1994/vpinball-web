//! Prints how long each of the chip's sixty-four phonemes lasts and how loud
//! it is, straight off the phoneme ROM.
//!
//! ```text
//! cargo run -p vpw-gts80 --example phonemes
//! ```
//!
//! Nothing here is a table anybody typed in: the durations come out of the
//! chip's own ROM through its own counters, which is why they are the answer
//! to "how long is a `PA1`" rather than a guess at it.

use vpw_gts80::votrax::{NAMES, NOMINAL_CLOCK, Votrax};

const RATE: u32 = 48_000;

fn main() {
    let mut chip = Votrax::new(RATE);
    chip.set_clock(NOMINAL_CLOCK);

    println!(" code name  ms    peak");
    for (code, name) in NAMES.iter().enumerate() {
        chip.write(code as u8);
        let mut samples = 0u32;
        let mut peak = 0.0f32;
        while chip.busy() && samples < RATE {
            peak = peak.max(chip.sample().abs());
            samples += 1;
        }
        let ms = f64::from(samples) * 1000.0 / f64::from(RATE);
        println!("  {code:02X}  {name:<4} {ms:6.1} {peak:7.4}");
    }
}
