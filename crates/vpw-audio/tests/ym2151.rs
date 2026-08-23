//! YM2151 tests.
//!
//! The question to answer is not "does it compile" but "does it synthesise":
//! if the phase generator, the envelope and the algorithm routing do their
//! job, the chip has to put out a wave with the right frequency, the right
//! shape and the right duration.

use vpw_audio::Ym2151;

/// Clock of the CS board: the NTSC colour crystal.
const CLOCK: u32 = 3_579_545;
/// The chip divides its clock by 64 to put out a sample.
const SAMPLE_RATE: u32 = CLOCK / 64;

/// Programs a channel with algorithm 7 (the four operators in parallel, with
/// no modulation) and only the last operator audible.
///
/// It is the simplest possible patch: a lone sine wave, which is what you need
/// to measure frequency without the modulation dirtying up the spectrum.
fn sine_patch(chip: &mut Ym2151, channel: u8, kc: u8) {
    let write = |chip: &mut Ym2151, reg: u8, value: u8| {
        chip.write_address(reg);
        chip.write_data(value);
    };

    // Algorithm 7, no feedback, sounding out of both channels.
    write(chip, 0x20 + channel, 0xC0 | 7);
    // Note and fraction.
    write(chip, 0x28 + channel, kc);
    write(chip, 0x30 + channel, 0x00);

    for slot in 0..4u8 {
        let op = channel + slot * 8;
        // Multiplier 1, no detune.
        write(chip, 0x40 + op, 0x01);
        // Only the last operator is heard; the others go silent.
        write(chip, 0x60 + op, if slot == 3 { 0x00 } else { 0x7F });
        // Instantaneous attack.
        write(chip, 0x80 + op, 0x1F);
        // No decay: it stays up while the key is held down.
        write(chip, 0xA0 + op, 0x00);
        write(chip, 0xC0 + op, 0x00);
        // Maximum sustain level and fast release.
        write(chip, 0xE0 + op, 0x0F);
    }
}

fn key_on(chip: &mut Ym2151, channel: u8) {
    chip.write_address(0x08);
    chip.write_data(0x78 | channel); // all four operators
}

fn key_off(chip: &mut Ym2151, channel: u8) {
    chip.write_address(0x08);
    chip.write_data(channel);
}

/// Generates `n` mono samples.
fn render(chip: &mut Ym2151, n: usize) -> Vec<f32> {
    (0..n).map(|_| chip.next_sample_mono()).collect()
}

/// Estimates the frequency by counting rising zero crossings.
fn frequency(samples: &[f32]) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f32 * SAMPLE_RATE as f32 / samples.len() as f32
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

// --- The basics --------------------------------------------------------------

#[test]
fn on_reset_it_is_silent() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    let samples = render(&mut chip, 1000);

    assert_eq!(
        peak(&samples),
        0.0,
        "without touching anything it must not sound"
    );
    assert_eq!(chip.active_operators(), 0);
}

#[test]
fn a_note_sounds() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);
    key_on(&mut chip, 0);

    let samples = render(&mut chip, 2000);
    assert!(
        peak(&samples) > 0.05,
        "it should sound, peak {}",
        peak(&samples)
    );
    assert_eq!(chip.active_operators(), 4);
}

#[test]
fn releasing_the_key_turns_it_off() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);
    key_on(&mut chip, 0);
    let sounding = peak(&render(&mut chip, 2000));
    assert!(sounding > 0.05);

    key_off(&mut chip, 0);
    // Give the release time to finish.
    render(&mut chip, 20_000);
    let after = peak(&render(&mut chip, 2000));

    assert!(
        after < sounding / 20.0,
        "after releasing it should go quiet (from {sounding} to {after})"
    );
}

// --- The phase generator -------------------------------------------------------

#[test]
fn going_up_an_octave_doubles_the_frequency() {
    // It is the strongest test of the phase generator and it does not depend
    // on knowing the absolute note mapping: whatever the frequency of a note
    // is, the same note one octave up has to give exactly twice that.
    let measure = |kc: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, kc);
        key_on(&mut chip, 0);
        render(&mut chip, 500); // skip the attack
        // Long window: counting zero crossings has a resolution of one cycle,
        // so with few cycles the measurement error swamps what you are trying
        // to measure.
        frequency(&render(&mut chip, 400_000))
    };

    // The key code carries the octave in bits 6-4.
    let low = measure(0x2A);
    let high = measure(0x3A);

    let ratio = high / low;
    assert!(
        (ratio - 2.0).abs() < 0.02,
        "an octave has to be exactly double: {low:.1} Hz -> {high:.1} Hz (ratio {ratio:.4})"
    );
}

#[test]
fn the_twelve_notes_of_an_octave_rise_in_order() {
    // The key code encodes the note in four bits but only twelve are valid:
    // the values 3, 7, 11 and 15 are skipped. Walking the twelve good ones has
    // to give strictly increasing frequencies.
    let valid_notes = [0u8, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14];
    let mut previous = 0.0f32;

    for note in valid_notes {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x40 | note);
        key_on(&mut chip, 0);
        render(&mut chip, 500);
        let f = frequency(&render(&mut chip, 20_000));

        assert!(
            f > previous,
            "note {note:#x} should be higher than the previous one \
             ({f:.1} Hz against {previous:.1} Hz)"
        );
        previous = f;
    }
}

#[test]
fn the_frequency_multiplier_multiplies() {
    // Register $40 carries a multiplier: it is what lets you build harmonics
    // without changing the note.
    let measure = |mul: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x3A);
        // The audible operator is slot 3 of channel 0.
        chip.write_address(0x40 + 3 * 8);
        chip.write_data(mul);
        key_on(&mut chip, 0);
        render(&mut chip, 500);
        frequency(&render(&mut chip, 20_000))
    };

    let base = measure(1);
    let double = measure(2);
    let quadruple = measure(4);

    assert!(
        (double / base - 2.0).abs() < 0.02,
        "x2: {base:.1} -> {double:.1}"
    );
    assert!(
        (quadruple / base - 4.0).abs() < 0.05,
        "x4: {base:.1} -> {quadruple:.1}"
    );
}

// --- The envelope ------------------------------------------------------------------

#[test]
fn the_total_level_lowers_the_volume() {
    let measure = |tl: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        chip.write_address(0x60 + 3 * 8);
        chip.write_data(tl);
        key_on(&mut chip, 0);
        render(&mut chip, 500);
        peak(&render(&mut chip, 5000))
    };

    let loud = measure(0x00);
    let medium = measure(0x10);
    let quiet = measure(0x20);

    assert!(loud > medium, "TL 0 louder than TL 16 ({loud} vs {medium})");
    assert!(
        medium > quiet,
        "TL 16 louder than TL 32 ({medium} vs {quiet})"
    );
    // The total level is 7 bits spread over 96 dB: each unit is 0.75 dB, so
    // 32 units are 24 dB, that is, a ratio of 16.
    let ratio = loud / quiet;
    assert!(
        (12.0..=20.0).contains(&ratio),
        "32 units of TL are 24 dB, that is, a ratio near 16 (got {ratio:.2})"
    );
}

#[test]
fn the_decay_falls_to_the_sustain() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);

    // Fast decay down to a sustain at half height.
    chip.write_address(0xA0 + 3 * 8);
    chip.write_data(0x1F);
    chip.write_address(0xE0 + 3 * 8);
    chip.write_data(0x4F);

    key_on(&mut chip, 0);
    let attack = peak(&render(&mut chip, 500));
    render(&mut chip, 20_000);
    let sustain = peak(&render(&mut chip, 2000));

    assert!(
        sustain < attack && sustain > 0.0,
        "it should come down and stay there (attack {attack}, sustain {sustain})"
    );
}

// --- The algorithms -----------------------------------------------------------------

#[test]
fn phase_modulation_enriches_the_spectrum() {
    // It is the whole idea of FM synthesis: a lone sine wave is poor, and
    // modulated with another one it generates sidebands. You can tell because
    // the wave stops crossing zero twice per cycle.
    let build = |algorithm: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        chip.write_address(0x20);
        chip.write_data(0xC0 | algorithm);
        // In a chain the intermediate operators have to have volume: if they
        // are muted they do not carry the modulation and the last one ends up
        // sounding like a lone sine wave.
        for slot in 0..4u8 {
            chip.write_address(0x60 + slot * 8);
            chip.write_data(0x00);
        }
        key_on(&mut chip, 0);
        render(&mut chip, 1000);
        render(&mut chip, 20_000)
    };

    let parallel = build(7);
    let chain = build(0);

    let crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();

    assert!(
        peak(&chain) > 0.01,
        "the chain should sound, peak {}",
        peak(&chain)
    );
    assert_ne!(
        crossings(&parallel),
        crossings(&chain),
        "modulating should change the shape of the wave"
    );
}

#[test]
fn all_eight_algorithms_sound() {
    for algorithm in 0..8u8 {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        chip.write_address(0x20);
        chip.write_data(0xC0 | algorithm);
        // Give all four volume so that any routing can be heard.
        for slot in 0..4u8 {
            chip.write_address(0x60 + slot * 8);
            chip.write_data(0x10);
        }
        key_on(&mut chip, 0);
        render(&mut chip, 1000);

        let p = peak(&render(&mut chip, 5000));
        assert!(
            p > 0.001,
            "algorithm {algorithm} put out nothing (peak {p})"
        );
    }
}

#[test]
fn the_eight_channels_are_independent() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    for channel in 0..8u8 {
        sine_patch(&mut chip, channel, 0x30 + channel);
    }

    // Turn them on one at a time and see that each one adds up.
    let mut previous = 0;
    for channel in 0..8u8 {
        key_on(&mut chip, channel);
        render(&mut chip, 200);
        let active = chip.active_operators();
        assert_eq!(active, previous + 4, "channel {channel} did not latch");
        previous = active;
    }
    assert_eq!(chip.active_operators(), 32, "all 32 operators sounding");
}

// --- Panning ----------------------------------------------------------------------------

#[test]
fn panning_splits_between_left_and_right() {
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);

    // Left only: bit 6.
    chip.write_address(0x20);
    chip.write_data(0x40 | 7);
    key_on(&mut chip, 0);
    render(&mut chip, 500);

    let (mut left, mut right) = (0.0f32, 0.0f32);
    for _ in 0..5000 {
        let (l, r) = chip.next_sample();
        left = left.max(l.abs());
        right = right.max(r.abs());
    }

    assert!(left > 0.01, "it should sound on the left ({left})");
    assert_eq!(right, 0.0, "and nothing on the right");
}

// --- The low frequency oscillator -------------------------------------------------

/// Programs the LFO: waveform, frequency and depths.
fn patch_lfo(chip: &mut Ym2151, wave: u8, freq: u8, amd: u8, pmd: u8) {
    let mut w = |r: u8, v: u8| {
        chip.write_address(r);
        chip.write_data(v);
    };
    w(0x1B, wave);
    w(0x18, freq);
    w(0x19, amd & 0x7F); // bit 7 at 0: amplitude depth
    w(0x19, 0x80 | (pmd & 0x7F)); // bit 7 at 1: phase depth
}

#[test]
fn vibrato_moves_the_frequency() {
    // Without the LFO the note has a fixed frequency; with vibrato it
    // oscillates around it, so the zero crossing count changes.
    let measure = |with_vibrato: bool| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        if with_vibrato {
            patch_lfo(&mut chip, 2, 0x40, 0, 0x7F); // triangle, maximum depth
            // Phase sensitivity of the channel, at maximum.
            chip.write_address(0x38);
            chip.write_data(0x70);
        }
        key_on(&mut chip, 0);
        render(&mut chip, 500);
        render(&mut chip, 100_000)
    };

    let flat = measure(false);
    let vibrated = measure(true);

    let crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert_ne!(
        crossings(&flat),
        crossings(&vibrated),
        "the vibrato should shift the frequency"
    );
    assert!(peak(&vibrated) > 0.05, "and the note has to keep sounding");
}

#[test]
fn tremolo_moves_the_amplitude() {
    // The tremolo adds attenuation, so the envelope of the sound stops being
    // flat: between the peak and the trough there has to be a difference.
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);
    // The high nibble of $18 picks the LFO's frequency range. It has to end up
    // a good deal slower than the measurement window: if each window covers a
    // whole cycle, the peak always comes out similar and the tremolo becomes
    // invisible.
    patch_lfo(&mut chip, 2, 0xC0, 0x7F, 0);
    // Enable the tremolo on the audible operator (bit 7 of register $A0).
    chip.write_address(0xA0 + 3 * 8);
    chip.write_data(0x80);
    // Amplitude sensitivity of the channel. At maximum, the attenuation the
    // LFO adds turns the operator off completely.
    chip.write_address(0x38);
    chip.write_data(0x02);
    key_on(&mut chip, 0);
    render(&mut chip, 1000);

    // Measure the peak in short windows across several LFO cycles.
    let mut peaks: Vec<f32> = Vec::new();
    for _ in 0..48 {
        peaks.push(peak(&render(&mut chip, 1000)));
    }

    let highest = peaks.iter().fold(0.0f32, |a, &b| a.max(b));
    let lowest = peaks.iter().fold(1.0f32, |a, &b| a.min(b));
    assert!(
        highest > lowest * 1.5,
        "the tremolo should make the volume breathe (from {lowest} to {highest})"
    );
}

#[test]
fn without_enabling_it_the_tremolo_has_no_effect() {
    // Bit 7 of register $A0 is per operator: if it is not set, the LFO does
    // not touch its amplitude even if it is configured.
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);
    patch_lfo(&mut chip, 2, 0xC0, 0x7F, 0);
    chip.write_address(0x38);
    chip.write_data(0x02);
    key_on(&mut chip, 0);
    render(&mut chip, 1000);

    let mut peaks: Vec<f32> = Vec::new();
    for _ in 0..48 {
        peaks.push(peak(&render(&mut chip, 1000)));
    }
    let highest = peaks.iter().fold(0.0f32, |a, &b| a.max(b));
    let lowest = peaks.iter().fold(1.0f32, |a, &b| a.min(b));

    assert!(
        highest < lowest * 1.05,
        "without enabling it, the volume should stay put ({lowest} to {highest})"
    );
}

#[test]
fn the_four_waveforms_give_different_results() {
    let measure = |wave: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        patch_lfo(&mut chip, wave, 0xC0, 0x7F, 0);
        chip.write_address(0xA0 + 3 * 8);
        chip.write_data(0x80);
        chip.write_address(0x38);
        chip.write_data(0x02);
        key_on(&mut chip, 0);
        render(&mut chip, 1000);
        // Energy sum: each waveform spreads the attenuation differently.
        render(&mut chip, 60_000).iter().map(|s| s * s).sum::<f32>()
    };

    let energies: Vec<f32> = (0..4).map(measure).collect();
    for (i, a) in energies.iter().enumerate() {
        for (j, b) in energies.iter().enumerate().skip(i + 1) {
            assert!(
                (a - b).abs() > a * 0.01,
                "waveforms {i} and {j} give almost the same thing ({a} and {b})"
            );
        }
    }
}

#[test]
fn with_zero_depth_the_lfo_does_nothing() {
    let measure = |amd: u8, pmd: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 0, 0x4A);
        patch_lfo(&mut chip, 2, 0x20, amd, pmd);
        chip.write_address(0xA0 + 3 * 8);
        chip.write_data(0x80);
        chip.write_address(0x38);
        chip.write_data(0x73);
        key_on(&mut chip, 0);
        render(&mut chip, 1000);
        render(&mut chip, 20_000)
    };

    let without_lfo = measure(0, 0);
    let with_lfo = measure(0x7F, 0x7F);

    assert_ne!(
        without_lfo.iter().map(|s| s * s).sum::<f32>().to_bits(),
        with_lfo.iter().map(|s| s * s).sum::<f32>().to_bits(),
        "with depth something should change"
    );
    // With zero depth the output has to be identical to having no LFO at all.
    let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
    sine_patch(&mut chip, 0, 0x4A);
    key_on(&mut chip, 0);
    render(&mut chip, 1000);
    let never_configured = render(&mut chip, 20_000);

    assert_eq!(
        without_lfo, never_configured,
        "zero depth has to be the same as having no LFO"
    );
}

// --- The noise generator --------------------------------------------------------

#[test]
fn the_noise_only_affects_channel_8() {
    // It is the only place in the chip where the noise generator gets in: it
    // replaces the output of the last operator of channel 8.
    let measure = |channel: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, channel, 0x4A);
        // Enable the noise with the shortest period, which is the one that
        // strays furthest from a tone.
        chip.write_address(0x0F);
        chip.write_data(0x9F);
        key_on(&mut chip, channel);
        render(&mut chip, 500);

        let s = render(&mut chip, 20_000);
        // The noise crosses zero far more often than a sine wave.
        s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    };

    let channel_0 = measure(0);
    let channel_7 = measure(7);

    assert!(
        channel_7 > channel_0 * 5,
        "channel 8 should sound like noise and channel 1 like a tone \
         ({channel_7} crossings against {channel_0})"
    );
}

#[test]
fn without_enabling_it_channel_8_sounds_normal() {
    let crossings = |enabled: bool| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 7, 0x4A);
        if enabled {
            chip.write_address(0x0F);
            chip.write_data(0x9F);
        }
        key_on(&mut chip, 7);
        render(&mut chip, 500);
        let s = render(&mut chip, 20_000);
        s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    };

    assert!(
        crossings(true) > crossings(false) * 3,
        "bit 7 of register $0F is what turns it on ({} against {})",
        crossings(true),
        crossings(false)
    );
}

#[test]
fn the_noise_period_changes_its_colour() {
    // A short period gives brighter noise: more transitions per second.
    let crossings = |period: u8| {
        let mut chip = Ym2151::new(CLOCK, SAMPLE_RATE);
        sine_patch(&mut chip, 7, 0x4A);
        chip.write_address(0x0F);
        chip.write_data(0x80 | period);
        key_on(&mut chip, 7);
        render(&mut chip, 500);
        let s = render(&mut chip, 20_000);
        s.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    };

    let bright = crossings(0x1F); // minimum period
    let dark = crossings(0x00); // maximum period
    assert!(
        bright > dark,
        "a short period should give brighter noise ({bright} against {dark})"
    );
}
