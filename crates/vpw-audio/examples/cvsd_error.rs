//! Measures how well the CVSD reconstructs different signals.
//!
//!     cargo run -p vpw-audio --example cvsd_error

use vpw_audio::Cvsd;

fn roundtrip(signal: &[f32]) -> f32 {
    let mut cvsd = Cvsd::new();
    let decoded: Vec<f32> = signal
        .iter()
        .map(|&target| cvsd.clock(target > cvsd.sample()))
        .collect();

    let skip = 500.min(signal.len() / 4);
    let n = signal.len() - skip;
    let sum: f32 = signal[skip..]
        .iter()
        .zip(&decoded[skip..])
        .map(|(a, b)| (a - b) * (a - b))
        .sum();
    (sum / n as f32).sqrt()
}

fn sine(samples: usize, cycles: f32, amplitude: f32) -> Vec<f32> {
    (0..samples)
        .map(|i| (i as f32 / samples as f32 * cycles * std::f32::consts::TAU).sin() * amplitude)
        .collect()
}

fn main() {
    println!("RMS reconstruction error (0 = perfect, 1 = full scale)");
    println!();
    for (name, signal) in [
        ("silence", vec![0.0; 8000]),
        ("low sine, amplitude 0.6", sine(8000, 20.0, 0.6)),
        ("mid sine, amplitude 0.6", sine(8000, 40.0, 0.6)),
        ("high sine, amplitude 0.6", sine(8000, 200.0, 0.6)),
        ("mid sine, amplitude 0.1", sine(8000, 40.0, 0.1)),
        ("mid sine, amplitude 0.95", sine(8000, 40.0, 0.95)),
    ] {
        println!("  {name:<28} {:.4}", roundtrip(&signal));
    }
}
