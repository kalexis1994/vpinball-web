//! CVSD round trip: encode a signal and see whether the decoder reconstructs
//! it.
//!
//! This is the test that really says whether the decoder works. A CVSD encoder
//! **contains** a decoder: at every step it compares the input signal against
//! what the decoder has reconstructed so far and emits a bit according to
//! whether it has to go up or down. So if [`Cvsd`]'s decoder is correct, it is
//! enough to drive it from the outside with that rule to have an encoder, and
//! what comes out has to look like the original.
//!
//! If the syllabic filter were wrong —if the step did not grow on
//! coincidence— the encoder could not follow the slopes and the error would go
//! through the roof.

use vpw_audio::Cvsd;

/// Encodes `signal` and returns the bit stream along with what the decoder
/// reconstructs.
fn encode_decode(signal: &[f32]) -> (Vec<bool>, Vec<f32>) {
    let mut cvsd = Cvsd::new();
    let mut bits = Vec::with_capacity(signal.len());
    let mut decoded = Vec::with_capacity(signal.len());

    for &target in signal {
        // The bit says whether to go up or down relative to what has been
        // reconstructed.
        let bit = target > cvsd.sample();
        bits.push(bit);
        decoded.push(cvsd.clock(bit));
    }

    (bits, decoded)
}

/// Root-mean-square error between two signals, skipping the start-up: the
/// integrator and the syllabic filter begin at zero and need a few samples to
/// lock on.
fn rms_error(a: &[f32], b: &[f32], skip: usize) -> f32 {
    let n = a.len() - skip;
    let sum: f32 = a[skip..]
        .iter()
        .zip(&b[skip..])
        .map(|(x, y)| (x - y) * (x - y))
        .sum();
    (sum / n as f32).sqrt()
}

fn sine(samples: usize, cycles: f32, amplitude: f32) -> Vec<f32> {
    (0..samples)
        .map(|i| {
            let phase = i as f32 / samples as f32 * cycles * std::f32::consts::TAU;
            phase.sin() * amplitude
        })
        .collect()
}

#[test]
fn it_reconstructs_a_sine() {
    // 40 cycles in 8000 samples: at 16 kHz that would be 80 Hz, well inside
    // the range of the voice.
    let original = sine(8000, 40.0, 0.6);
    let (_, decoded) = encode_decode(&original);

    let error = rms_error(&original, &decoded, 500);
    assert!(
        error < 0.1,
        "the decoder should follow the sine closely, error {error}"
    );
}

#[test]
fn it_tracks_a_signal_that_changes_amplitude() {
    // Real speech: a carrier with its amplitude modulated. It is exactly the
    // case the syllabic filter exists to solve.
    let original: Vec<f32> = (0..8000)
        .map(|i| {
            let t = i as f32 / 8000.0;
            let envelope = (t * std::f32::consts::TAU * 3.0).sin().abs();
            let carrier = (t * std::f32::consts::TAU * 60.0).sin();
            carrier * envelope * 0.7
        })
        .collect();

    let (_, decoded) = encode_decode(&original);
    let error = rms_error(&original, &decoded, 500);
    assert!(error < 0.12, "error {error}");
}

#[test]
fn silence_is_encoded_by_alternating() {
    // With the signal at zero the encoder oscillates around zero, so there is
    // no coincidence and the step stays small. It is the property that makes
    // silence sound like silence and not like noise.
    let original = vec![0.0f32; 2000];
    let (bits, decoded) = encode_decode(&original);

    let changes = bits.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        changes > bits.len() / 3,
        "in silence the stream should alternate a lot ({changes} changes in {})",
        bits.len()
    );

    let error = rms_error(&original, &decoded, 200);
    assert!(error < 0.02, "silence has to stay quiet, error {error}");
}

#[test]
fn a_loud_signal_grows_the_step_and_a_quiet_one_does_not() {
    // The step adapts to the signal: that is what puts the "variable slope" in
    // the name of the format.
    let mut loud = Cvsd::new();
    for &target in &sine(4000, 40.0, 0.9) {
        let bit = target > loud.sample();
        loud.clock(bit);
    }

    let mut quiet = Cvsd::new();
    for &target in &sine(4000, 40.0, 0.05) {
        let bit = target > quiet.sample();
        quiet.clock(bit);
    }

    assert!(
        loud.step_size() > quiet.step_size() * 2.0,
        "the loud signal should be working with a bigger step \
         (loud {}, quiet {})",
        loud.step_size(),
        quiet.step_size()
    );
}

#[test]
fn it_does_not_diverge_on_any_old_stream() {
    // A stream of garbage must not push the output out of range nor leave it
    // stuck at one extreme.
    let mut cvsd = Cvsd::new();
    let mut seed = 0x1234_5678u32;
    let mut sum = 0.0f32;

    for _ in 0..20_000 {
        // Some congruential generator, just to get varied bits.
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let sample = cvsd.clock(seed & 0x8000_0000 != 0);
        assert!((-1.0..=1.0).contains(&sample));
        sum += sample;
    }

    let average = sum / 20_000.0;
    assert!(
        average.abs() < 0.1,
        "with random bits the output should stay centred, it gave {average}"
    );
}
