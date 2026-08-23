//! Measures the frequency the YM2151 puts out for each note.
//!
//!     cargo run -p vpw-audio --example ym2151_notes

use vpw_audio::Ym2151;

const CLOCK: u32 = 3_579_545;
const SAMPLE_RATE: u32 = CLOCK / 64;

fn measure(kc: u8) -> f32 {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    let mut w = |r: u8, v: u8| {
        chip.write_address(r);
        chip.write_data(v);
    };
    w(0x20, 0xC7); // algorithm 7, stereo
    w(0x28, kc);
    w(0x30, 0x00);
    for slot in 0..4u8 {
        w(0x40 + slot * 8, 0x01);
        w(0x60 + slot * 8, if slot == 3 { 0x00 } else { 0x7F });
        w(0x80 + slot * 8, 0x1F);
        w(0xA0 + slot * 8, 0x00);
        w(0xC0 + slot * 8, 0x00);
        w(0xE0 + slot * 8, 0x0F);
    }
    w(0x08, 0x78);

    for _ in 0..500 {
        chip.next_sample_mono();
    }
    let n = 400_000;
    let samples: Vec<f32> = (0..n).map(|_| chip.next_sample_mono()).collect();
    let crossings = samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f32 * SAMPLE_RATE as f32 / n as f32
}

fn main() {
    // The twelve valid note codes of an octave. The chip uses four codes for
    // every group of three semitones, so one in every four does not exist.
    let notes = [
        (0x0u8, "C#"),
        (0x1, "D"),
        (0x2, "D#"),
        (0x4, "E"),
        (0x5, "F"),
        (0x6, "F#"),
        (0x8, "G"),
        (0x9, "G#"),
        (0xA, "A"),
        (0xC, "A#"),
        (0xD, "B"),
        (0xE, "C"),
    ];

    println!("octave 4 (KC $4x)");
    println!("note | KC   | measured | tempered  | error");
    println!("-----+------+----------+-----------+-------");

    // Reference: A of octave 4 should give 440 Hz.
    let a4 = measure(0x4A);
    for (code, name) in notes {
        let f = measure(0x40 | code);
        // Semitones relative to A within the group of twelve.
        let index = notes.iter().position(|(c, _)| *c == code).unwrap() as i32;
        let index_a = notes.iter().position(|(c, _)| *c == 0x0A).unwrap() as i32;
        let expected = a4 * 2f32.powf((index - index_a) as f32 / 12.0);
        let error = (f / expected - 1.0) * 100.0;
        println!(
            "  {name:<2} | ${:02X}  | {f:>7.2} Hz | {expected:>7.2} Hz | {error:>+5.2}%",
            0x40 | code
        );
    }
    println!();
    println!("A of octave 4: {a4:.2} Hz (the standard is 440)");
}
