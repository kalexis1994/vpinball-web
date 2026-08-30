//! The speech chip, checked against the one thing about it that was published.
//!
//! Everything in [`vpw_gts80::votrax`] is decoded out of the chip's own ROM by
//! its own counters, which is a lot of bit-swapping to get right and no way to
//! tell from the outside when it is wrong: a mangled field does not fail, it
//! just says something else.
//!
//! Votrax's data sheet prints one number per phoneme — how long it lasts — and
//! that is the cross-check. If the duration field is decoded correctly then so
//! is the rest of the packing around it, because they share the same
//! sixty-four rows and the same bit order.

use vpw_gts80::votrax::{Chip, NAMES, NOMINAL_CLOCK, Votrax};

const RATE: u32 = 48_000;

/// The durations Votrax publishes, in milliseconds, in the chip's code order.
#[rustfmt::skip]
const PUBLISHED_MS: [f64; 64] = [
     59.0,  71.0, 121.0,  47.0,  47.0,  71.0, 103.0,  90.0,
     71.0,  55.0,  80.0, 121.0, 103.0,  80.0,  71.0,  71.0,
     71.0, 121.0,  71.0, 146.0, 121.0, 146.0, 103.0, 185.0,
    103.0,  80.0,  47.0,  71.0,  71.0, 103.0,  55.0,  90.0,
    185.0,  65.0,  80.0,  47.0, 250.0, 103.0, 185.0, 185.0,
    185.0, 103.0,  71.0,  90.0, 185.0,  80.0, 185.0, 103.0,
     90.0,  71.0, 103.0, 185.0,  80.0, 121.0,  59.0,  90.0,
     80.0,  71.0, 146.0, 185.0, 121.0, 250.0, 185.0,  47.0,
];

/// How long the chip holds its busy line for one phoneme, in milliseconds.
fn spoken_ms(chip: &mut Votrax, code: u8) -> f64 {
    chip.write(code);
    let mut samples = 0u32;
    while chip.busy() && samples < RATE {
        chip.sample();
        samples += 1;
    }
    f64::from(samples) * 1000.0 / f64::from(RATE)
}

#[test]
fn every_phoneme_lasts_as_long_as_the_data_sheet_says() {
    let mut chip = Votrax::new(RATE);
    chip.set_clock(NOMINAL_CLOCK);
    for (code, (name, published)) in NAMES.iter().zip(PUBLISHED_MS).enumerate() {
        let measured = spoken_ms(&mut chip, code as u8);
        let off = (measured - published).abs() / published;
        assert!(
            off < 0.08,
            "{name} ({code:#04X}): the data sheet says {published} ms and the \
             ROM gives {measured:.1}"
        );
    }
}

#[test]
fn the_two_chips_are_not_quite_the_same() {
    // They share most of the ROM. The SC-01-A changed twelve rows, all of them
    // the open vowels — `AH`, `AW`, `AE`, `UH` and their variants — which is
    // the whole difference between the Mars prototype and every machine after
    // it.
    let say = |which: Chip, code: u8| -> Vec<f32> {
        let mut chip = Votrax::new(RATE);
        chip.set_chip(which);
        chip.set_clock(NOMINAL_CLOCK);
        chip.write(code);
        (0..8_000).map(|_| chip.sample()).collect()
    };

    // `AH`, one of the twelve.
    assert_ne!(say(Chip::Sc01, 0x24), say(Chip::Sc01A, 0x24));
    // And `S`, which is not: the two chips say it the same way.
    assert_eq!(say(Chip::Sc01, 0x1F), say(Chip::Sc01A, 0x1F));
}

#[test]
fn the_busy_line_is_the_whole_handshake() {
    let mut chip = Votrax::new(RATE);
    chip.set_clock(NOMINAL_CLOCK);
    assert!(!chip.busy(), "idle at power-on");

    // AH, the longest vowel the chip has.
    chip.write(0x24);
    assert!(chip.busy(), "busy the instant the strobe is seen");
    assert_eq!(chip.speaking(), "AH");

    let mut samples = 0;
    while chip.busy() && samples < RATE {
        chip.sample();
        samples += 1;
    }
    assert!(!chip.busy(), "and it lets go at the end");
    assert!(samples > RATE / 10, "AH is a quarter of a second");
}

#[test]
fn the_silences_are_silent_and_the_vowels_are_not() {
    let mut chip = Votrax::new(RATE);
    chip.set_clock(NOMINAL_CLOCK);

    // A vowel, from cold: the formants slide an eighth of the way at a time,
    // so it takes a moment to come up.
    chip.write(0x24); // AH
    let loud = (0..12_000)
        .map(|_| chip.sample().abs())
        .fold(0.0f32, f32::max);
    assert!(loud > 0.1, "a vowel should be loud, and peaked at {loud}");

    // `STOP` closes the mouth. Give the filters time to ring out.
    chip.write(0x3F);
    for _ in 0..12_000 {
        chip.sample();
    }
    let quiet = (0..2_000)
        .map(|_| chip.sample().abs())
        .fold(0.0f32, f32::max);
    assert!(quiet < 0.01, "and silence should be silent, not {quiet}");
}

#[test]
fn a_slower_clock_makes_a_longer_word() {
    let mut chip = Votrax::new(RATE);
    chip.set_clock(NOMINAL_CLOCK);
    let ordinary = spoken_ms(&mut chip, 0x24);

    // Where a finished "TILT" ends up: 398 kHz, measured on real hardware.
    chip.set_clock(398_000);
    let slowed = spoken_ms(&mut chip, 0x24);

    let ratio = slowed / ordinary;
    assert!(
        (ratio - f64::from(NOMINAL_CLOCK) / 398_000.0).abs() < 0.1,
        "the word should stretch with the clock, and stretched by {ratio:.2}"
    );
}
