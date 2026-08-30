//! The Votrax SC-01, the chip that gives these machines a voice.
//!
//! It is not a sampler and there is no recorded speech anywhere in the
//! machine. The SC-01 is a **formant synthesiser** on a single die: you write
//! one of sixty-four phonemes, it pulls a busy line and takes a tenth of a
//! second or so to say it, and when the line drops it wants the next one. The
//! sound board spells words out of them. That is why a Gottlieb says "BLACK
//! HOLE" in the voice it does — nobody recorded it, it was spelled.
//!
//! # Where this comes from
//!
//! A port of PinMAME's `src/sound/votrax.c`, which carries
//! `// license:BSD-3-Clause` and so may be used here, unlike the CPU cores
//! next door. The simulation core is Olivier Galibert's, reconstructed from
//! the die itself: [`SC01`] and [`SC01A`] below are the chip's own five
//! hundred and twelve bytes of phoneme ROM, and the filters are the
//! switched-capacitor networks it is built from, worked out from the areas of
//! their capacitors. That is why this sounds like an SC-01 and not merely like
//! a speech synthesiser.
//!
//! > Copyright the MAME and PinMAME authors — Olivier Galibert for this core,
//! > Mike Coates and Tom Haukap for the one before it. Redistribution and use
//! > in source and binary forms, with or without modification, are permitted
//! > provided that the conditions of the BSD-3-Clause licence are met. This
//! > software is provided by the copyright holders "as is" and any express or
//! > implied warranties are disclaimed.
//!
//! # How it makes a sound
//!
//! Two excitations and five filters. A glottal wave — a staircase built from a
//! transistor ladder, which is why it has nine steps and not a smooth shape —
//! and a shaped hiss from a fifteen-bit shift register. They pass through F1,
//! then F2 (which the noise is injected into by a path of its own), then F3,
//! then a fixed F4, then a final low-pass. Every one is a switched-capacitor
//! filter whose cutoff is set by *which capacitors the phoneme ROM switches
//! in*, and the ROM values are interpolated an eighth at a time, so the
//! formants slide rather than jump.
//!
//! Two bugs on the die are kept, because they are what the chip does: the
//! formant update writes `fc` where it means `va` and the other update does
//! the reverse, and two ROM fields have their bits reversed to compensate for
//! a comparator the prototype miswired.
//!
//! The clock is the board's business — nominally 720 kHz, and Gottlieb's Sound
//! & Speech board can move it with a converter, which is how these games say
//! "TILT" more slowly and more deeply each time.

/// The nominal clock, in Hz: what a board sets for ordinary speech.
pub const NOMINAL_CLOCK: u32 = 720_000;

/// Which of the two chips. They differ in a handful of ROM bytes, and only the
/// Mars prototype has the older one (`gts80games.c:346`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Chip {
    /// The original SC-01.
    Sc01,
    /// The SC-01-A, which is what almost every machine has.
    #[default]
    Sc01A,
}

/// The names Votrax gives the phonemes, which is what a sound ROM's author
/// would have been typing.
#[rustfmt::skip]
pub const NAMES: [&str; 64] = [
    "EH3", "EH2", "EH1", "PA0", "DT",  "A1",  "A2",  "ZH",
    "AH2", "I3",  "I2",  "I1",  "M",   "N",   "B",   "V",
    "CH",  "SH",  "Z",   "AW1", "NG",  "AH1", "OO1", "OO",
    "L",   "K",   "J",   "H",   "G",   "F",   "D",   "S",
    "A",   "AY",  "Y1",  "UH3", "AH",  "P",   "O",   "I",
    "U",   "Y",   "T",   "R",   "E",   "W",   "AE",  "AE1",
    "AW2", "UH2", "UH1", "UH",  "O2",  "O1",  "IU",  "U1",
    "THV", "TH",  "ER",  "EH",  "E1",  "AW",  "PA1", "STOP",
];

/// The code for a phoneme's name, for whoever is spelling by hand.
pub fn code_of(name: &str) -> Option<u8> {
    NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
        .map(|i| i as u8)
}

/// The phoneme ROM of the original SC-01, sixty-four entries of eight bytes.
const SC01: [u8; 512] = [
    0xA4, 0x50, 0xA0, 0xF0, 0xE0, 0x00, 0x00, 0x03, 0xA4, 0x50, 0xA0, 0x00, 0x23, 0x0A, 0x00, 0x3E,
    0xA4, 0x58, 0xA0, 0x30, 0xF0, 0x00, 0x00, 0x3F, 0xA3, 0x81, 0x69, 0xB0, 0xC1, 0x0C, 0x00, 0x3D,
    0x26, 0xD3, 0x49, 0x90, 0xA1, 0x09, 0x00, 0x3C, 0x27, 0x81, 0x68, 0x94, 0x21, 0x0A, 0x00, 0x3B,
    0x82, 0xC3, 0x48, 0x24, 0xA1, 0x08, 0x00, 0x3A, 0xA4, 0x00, 0x38, 0x18, 0x68, 0x01, 0x00, 0x39,
    0x20, 0x52, 0xE1, 0x88, 0x63, 0x0A, 0x00, 0x38, 0x22, 0xC1, 0xE8, 0x90, 0x61, 0x04, 0x00, 0x37,
    0xA2, 0x83, 0x60, 0x10, 0x66, 0x03, 0x00, 0x36, 0xA2, 0xC1, 0xE8, 0x80, 0xA1, 0x09, 0x00, 0x35,
    0xA2, 0xC1, 0xE8, 0x34, 0x61, 0x0A, 0x00, 0x34, 0xA3, 0x81, 0xC9, 0xB4, 0x21, 0x0A, 0x00, 0x33,
    0xA3, 0x81, 0xC9, 0xE4, 0xA1, 0x07, 0x00, 0x32, 0xA3, 0x81, 0xC9, 0x54, 0x63, 0x01, 0x00, 0x31,
    0xA3, 0x81, 0x69, 0x60, 0x61, 0x04, 0x00, 0x30, 0xA7, 0x81, 0xE8, 0x74, 0xA0, 0x07, 0x00, 0x2F,
    0xA7, 0x81, 0xE8, 0x74, 0x20, 0x0A, 0x00, 0x2E, 0x22, 0xC1, 0x60, 0x14, 0x66, 0x0A, 0x00, 0x2D,
    0x26, 0xD3, 0x49, 0x70, 0x20, 0x0A, 0x00, 0x2C, 0x82, 0x43, 0x08, 0x54, 0x63, 0x04, 0x00, 0x2B,
    0xE0, 0x32, 0x11, 0xE8, 0x72, 0x01, 0x00, 0x2A, 0x26, 0x53, 0x01, 0x64, 0xA1, 0x07, 0x00, 0x29,
    0x22, 0xC1, 0xE8, 0x80, 0x21, 0x0A, 0x00, 0x28, 0xA6, 0x91, 0x61, 0x80, 0x21, 0x0A, 0x00, 0x27,
    0xA2, 0xC1, 0xE8, 0x84, 0x21, 0x0A, 0x00, 0x26, 0xA8, 0x24, 0x13, 0x63, 0xB2, 0x07, 0x00, 0x25,
    0xA3, 0xC1, 0xE9, 0x84, 0xC1, 0x0C, 0x00, 0x24, 0xA3, 0x81, 0xC9, 0x54, 0xE3, 0x00, 0x00, 0x23,
    0x26, 0x12, 0xA0, 0x64, 0x61, 0x0A, 0x00, 0x22, 0x26, 0xD3, 0x69, 0x70, 0x61, 0x05, 0x00, 0x21,
    0xA6, 0xC1, 0xC9, 0x84, 0x21, 0x0A, 0x00, 0x20, 0xE0, 0x32, 0x91, 0x48, 0x68, 0x04, 0x00, 0x1F,
    0x26, 0x91, 0xE8, 0x00, 0x7C, 0x0B, 0x00, 0x1E, 0xA8, 0x2C, 0x83, 0x65, 0xA2, 0x07, 0x00, 0x1D,
    0x26, 0xC1, 0x41, 0xE0, 0x73, 0x01, 0x00, 0x1C, 0xAC, 0x04, 0x22, 0xFD, 0x62, 0x01, 0x00, 0x1B,
    0x2C, 0x34, 0x7B, 0xDB, 0xE8, 0x00, 0x00, 0x1A, 0x2C, 0x64, 0x23, 0x11, 0x72, 0x0A, 0x00, 0x19,
    0xA2, 0xD0, 0x09, 0xF4, 0xA1, 0x07, 0x00, 0x18, 0x23, 0x81, 0x49, 0x20, 0x21, 0x0A, 0x00, 0x17,
    0x23, 0x81, 0x49, 0x30, 0xA1, 0x07, 0x00, 0x16, 0xA3, 0xC1, 0xE9, 0x84, 0xA1, 0x08, 0x00, 0x15,
    0x36, 0x4B, 0x08, 0xD4, 0xA0, 0x09, 0x00, 0x14, 0xA3, 0x81, 0x69, 0x70, 0xA0, 0x08, 0x00, 0x13,
    0x60, 0x58, 0xD1, 0x9C, 0x63, 0x01, 0x00, 0x12, 0x6C, 0x54, 0x8B, 0xFB, 0xA2, 0x09, 0x00, 0x11,
    0x6C, 0x54, 0x8B, 0xFB, 0x63, 0x01, 0x00, 0x10, 0x28, 0x64, 0xD3, 0xF7, 0x63, 0x01, 0x00, 0x0F,
    0x22, 0x91, 0xE1, 0x90, 0x73, 0x01, 0x00, 0x0E, 0x36, 0x19, 0x24, 0xE6, 0x61, 0x0A, 0x00, 0x0D,
    0x32, 0x88, 0xA5, 0x66, 0xA3, 0x07, 0x00, 0x0C, 0xA6, 0x91, 0x61, 0x90, 0xA1, 0x09, 0x00, 0x0B,
    0xA6, 0x91, 0x61, 0x90, 0x61, 0x0A, 0x00, 0x0A, 0xA6, 0x91, 0x61, 0x80, 0x61, 0x0B, 0x00, 0x09,
    0xA3, 0xC1, 0xE9, 0xC4, 0x61, 0x01, 0x00, 0x08, 0x6C, 0x54, 0xCB, 0xF3, 0x63, 0x04, 0x00, 0x07,
    0xA6, 0xC1, 0xC9, 0x34, 0xA1, 0x07, 0x00, 0x06, 0xA6, 0xC1, 0xC9, 0x64, 0x61, 0x01, 0x00, 0x05,
    0xE8, 0x16, 0x03, 0x61, 0xFB, 0x00, 0x00, 0x04, 0x27, 0x81, 0x68, 0xC4, 0xA1, 0x09, 0x00, 0x02,
    0x27, 0x81, 0x68, 0xD4, 0x61, 0x01, 0x00, 0x01, 0x27, 0x81, 0x68, 0x74, 0x61, 0x03, 0x00, 0x00,
];

/// The phoneme ROM of the SC-01-A, sixty-four entries of eight bytes.
const SC01A: [u8; 512] = [
    0xA4, 0x50, 0xA0, 0xF0, 0xE0, 0x00, 0x00, 0x03, 0xA4, 0x50, 0xA0, 0x00, 0x23, 0x0A, 0x00, 0x3E,
    0xA4, 0x58, 0xA0, 0x30, 0xF0, 0x00, 0x00, 0x3F, 0xA3, 0x80, 0x69, 0xB0, 0xC1, 0x0C, 0x00, 0x3D,
    0x26, 0xD3, 0x49, 0x90, 0xA1, 0x09, 0x00, 0x3C, 0x27, 0x81, 0x68, 0x94, 0x21, 0x0A, 0x00, 0x3B,
    0x82, 0xC3, 0x48, 0x24, 0xA1, 0x08, 0x00, 0x3A, 0xA4, 0x00, 0x38, 0x18, 0x68, 0x01, 0x00, 0x39,
    0x20, 0x52, 0xE1, 0x88, 0x63, 0x0A, 0x00, 0x38, 0x22, 0xC1, 0xE8, 0x90, 0x61, 0x04, 0x00, 0x37,
    0xA2, 0x83, 0x60, 0x10, 0x66, 0x03, 0x00, 0x36, 0xA2, 0xC1, 0xE8, 0x80, 0xA1, 0x09, 0x00, 0x35,
    0xA2, 0xC1, 0xE8, 0x34, 0x61, 0x0A, 0x00, 0x34, 0xA3, 0x81, 0x89, 0xB4, 0x21, 0x0A, 0x00, 0x33,
    0xA3, 0x81, 0x89, 0xE4, 0xA1, 0x07, 0x00, 0x32, 0xA3, 0x81, 0x89, 0x54, 0x63, 0x01, 0x00, 0x31,
    0xA3, 0x80, 0x69, 0x60, 0x61, 0x04, 0x00, 0x30, 0xA7, 0x80, 0xE8, 0x74, 0xA0, 0x07, 0x00, 0x2F,
    0xA7, 0x80, 0xE8, 0x74, 0x20, 0x0A, 0x00, 0x2E, 0x22, 0xC1, 0x60, 0x14, 0x66, 0x0A, 0x00, 0x2D,
    0x26, 0xD3, 0x49, 0x70, 0x20, 0x0A, 0x00, 0x2C, 0x82, 0x43, 0x08, 0x54, 0x63, 0x04, 0x00, 0x2B,
    0xE0, 0x32, 0x11, 0xE8, 0x72, 0x01, 0x00, 0x2A, 0x26, 0x53, 0x01, 0x64, 0xA1, 0x07, 0x00, 0x29,
    0x22, 0xC1, 0xE8, 0x80, 0x21, 0x0A, 0x00, 0x28, 0xA6, 0x91, 0x61, 0x80, 0x21, 0x0A, 0x00, 0x27,
    0xA2, 0xC1, 0xE8, 0x84, 0x21, 0x0A, 0x00, 0x26, 0xA8, 0x24, 0x13, 0x63, 0xB2, 0x07, 0x00, 0x25,
    0xA3, 0x40, 0xE9, 0x84, 0xC1, 0x0C, 0x00, 0x24, 0xA3, 0x81, 0x89, 0x54, 0xE3, 0x00, 0x00, 0x23,
    0x26, 0x12, 0xA0, 0x64, 0x61, 0x0A, 0x00, 0x22, 0x26, 0xD3, 0x69, 0x70, 0x61, 0x05, 0x00, 0x21,
    0xA6, 0xC1, 0xC9, 0x84, 0x21, 0x0A, 0x00, 0x20, 0xE0, 0x32, 0x91, 0x48, 0x68, 0x04, 0x00, 0x1F,
    0x26, 0x91, 0xE8, 0x00, 0x7C, 0x0B, 0x00, 0x1E, 0xA8, 0x2C, 0x83, 0x65, 0xA2, 0x07, 0x00, 0x1D,
    0x26, 0xC1, 0x41, 0xE0, 0x73, 0x01, 0x00, 0x1C, 0xAC, 0x04, 0x22, 0xFD, 0x62, 0x01, 0x00, 0x1B,
    0x2C, 0x34, 0x7B, 0xDB, 0xE8, 0x00, 0x00, 0x1A, 0x2C, 0x64, 0x23, 0x11, 0x72, 0x0A, 0x00, 0x19,
    0xA2, 0xD0, 0x09, 0xF4, 0xA1, 0x07, 0x00, 0x18, 0x23, 0x81, 0x49, 0x20, 0x21, 0x0A, 0x00, 0x17,
    0x23, 0x81, 0x49, 0x30, 0xA1, 0x07, 0x00, 0x16, 0xA3, 0x40, 0xE9, 0x84, 0xA1, 0x08, 0x00, 0x15,
    0x36, 0x4B, 0x08, 0xD4, 0xA0, 0x09, 0x00, 0x14, 0xA3, 0x80, 0x69, 0x70, 0xA0, 0x08, 0x00, 0x13,
    0x60, 0x58, 0xD1, 0x9C, 0x63, 0x01, 0x00, 0x12, 0x6C, 0x54, 0x8B, 0xFB, 0xA2, 0x09, 0x00, 0x11,
    0x6C, 0x54, 0x8B, 0xFB, 0x63, 0x01, 0x00, 0x10, 0x28, 0x64, 0xD3, 0xF7, 0x63, 0x01, 0x00, 0x0F,
    0x22, 0x91, 0xE1, 0x90, 0x73, 0x01, 0x00, 0x0E, 0x36, 0x19, 0x24, 0xE6, 0x61, 0x0A, 0x00, 0x0D,
    0x32, 0x88, 0xA5, 0x66, 0xA3, 0x07, 0x00, 0x0C, 0xA6, 0x91, 0x61, 0x90, 0xA1, 0x09, 0x00, 0x0B,
    0xA6, 0x91, 0x61, 0x90, 0x61, 0x0A, 0x00, 0x0A, 0xA6, 0x91, 0x61, 0x80, 0x61, 0x0B, 0x00, 0x09,
    0xA3, 0x40, 0xE9, 0xC4, 0x61, 0x01, 0x00, 0x08, 0x6C, 0x54, 0xCB, 0xF3, 0x63, 0x04, 0x00, 0x07,
    0xA6, 0xC1, 0xC9, 0x34, 0xA1, 0x07, 0x00, 0x06, 0xA6, 0xC1, 0xC9, 0x64, 0x61, 0x01, 0x00, 0x05,
    0xE8, 0x16, 0x03, 0x61, 0xFB, 0x00, 0x00, 0x04, 0x27, 0x81, 0x68, 0xC4, 0xA1, 0x09, 0x00, 0x02,
    0x27, 0x81, 0x68, 0xD4, 0x61, 0x01, 0x00, 0x01, 0x27, 0x81, 0x68, 0x74, 0x61, 0x03, 0x00, 0x00,
];

/// The glottal wave, as the die draws it.
///
/// Built from a ladder of transistors rather than from a table, which is why
/// it is this shape: one step to ground, seven quarter-sized steps down the
/// ladder, then the middle value again. Rescaled so the middle is zero and the
/// top is one.
#[allow(clippy::eq_op)] // The sevenths are the ladder, and read as a ladder.
const GLOTTAL_WAVE: [f64; 9] = [
    0.0,
    -4.0 / 7.0,
    7.0 / 7.0,
    6.0 / 7.0,
    5.0 / 7.0,
    4.0 / 7.0,
    3.0 / 7.0,
    2.0 / 7.0,
    1.0 / 7.0,
];

/// The capacitor banks each formant's ROM bits switch in.
const F1_CAPS: [u32; 4] = [2546, 4973, 9861, 19724];
const F2V1_CAPS: [u32; 4] = [1390, 2965, 5875, 11297];
const F2V2_CAPS: [u32; 5] = [833, 1663, 3164, 6327, 12654];
const F2N1_CAPS: [u32; 4] = [1390, 2965, 5875, 11297];
const F2N2_CAPS: [u32; 5] = [833, 1663, 3164, 6327, 12654];
const F3_CAPS: [u32; 4] = [2226, 4485, 9056, 18111];

/// What the chip's own timer is waiting to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// The strobe has been seen: latch the phoneme's ROM values.
    Commit,
    /// The phoneme is over: let the busy line go.
    End,
}

/// One Votrax SC-01.
pub struct Votrax {
    chip: Chip,
    /// The chip's clock, and the two it divides down to: the sample clock at
    /// an eighteenth of it, and the capacitor switching clock at a
    /// thirty-sixth.
    mainclock: f64,
    sclock: f64,
    cclock: f64,

    inflection: u8,
    phone: u8,

    /// The acknowledge/request line. High is idle; the board reads it inverted
    /// as "busy".
    ar_state: bool,
    /// The chip's own timer, counted in main-clock ticks.
    timer: Option<(Pending, f64)>,

    // The ROM values of the phoneme in progress.
    rom_duration: u8,
    rom_vd: u8,
    rom_cld: u8,
    rom_fa: u8,
    rom_fc: u8,
    rom_va: u8,
    rom_f1: u8,
    rom_f2: u8,
    rom_f2q: u8,
    rom_f3: u8,
    rom_closure: bool,
    rom_pause: bool,

    // Where the interpolation has got to, eight bits each.
    cur_fa: u8,
    cur_fc: u8,
    cur_va: u8,
    cur_f1: u8,
    cur_f2: u8,
    cur_f2q: u8,
    cur_f3: u8,

    // And what the filters were last built from.
    filt_fa: u8,
    filt_fc: u8,
    filt_va: u8,
    filt_f1: u8,
    filt_f2: u8,
    filt_f2q: u8,
    filt_f3: u8,

    // The counters. Only the first two are reset on a phoneme change.
    phonetick: u16,
    ticks: u8,
    pitch: u8,
    closure: u8,
    update_counter: u8,

    cur_closure: bool,
    noise: u16,
    cur_noise: bool,
    sample_count: u32,

    // The filter histories.
    voice_1: [f64; 4],
    voice_2: [f64; 4],
    voice_3: [f64; 4],
    noise_1: [f64; 3],
    noise_2: [f64; 3],
    noise_3: [f64; 2],
    noise_4: [f64; 2],
    vn_1: [f64; 4],
    vn_2: [f64; 4],
    vn_3: [f64; 4],
    vn_4: [f64; 4],
    vn_5: [f64; 2],
    vn_6: [f64; 2],

    // And their coefficients.
    f1_a: [f64; 4],
    f1_b: [f64; 4],
    f2v_a: [f64; 4],
    f2v_b: [f64; 4],
    f2n_a: [f64; 2],
    f2n_b: [f64; 2],
    f3_a: [f64; 4],
    f3_b: [f64; 4],
    f4_a: [f64; 4],
    f4_b: [f64; 4],
    fx_a: [f64; 1],
    fx_b: [f64; 2],
    fn_a: [f64; 3],
    fn_b: [f64; 3],

    /// Whether the board has muted the output.
    muted: bool,

    // Getting the chip's own sample rate to the host's, which are not
    // multiples of each other and one of which moves.
    host_rate: f64,
    owed: f64,
    last: f64,
    sum: f64,
    taken: u32,
}

fn bit(value: u64, n: u32) -> u8 {
    ((value >> n) & 1) as u8
}

fn bitswap4(value: u64, b3: u32, b2: u32, b1: u32, b0: u32) -> u8 {
    (bit(value, b3) << 3) | (bit(value, b2) << 2) | (bit(value, b1) << 1) | bit(value, b0)
}

#[allow(clippy::too_many_arguments)]
fn bitswap7(value: u64, b6: u32, b5: u32, b4: u32, b3: u32, b2: u32, b1: u32, b0: u32) -> u8 {
    (bit(value, b6) << 6)
        | (bit(value, b5) << 5)
        | (bit(value, b4) << 4)
        | (bit(value, b3) << 3)
        | (bit(value, b2) << 2)
        | (bit(value, b1) << 1)
        | bit(value, b0)
}

/// The total capacitance a set of ROM bits switches in.
fn bits_to_caps(mut value: u8, caps: &[u32]) -> u32 {
    let mut total = 0;
    for c in caps {
        if value & 1 != 0 {
            total += c;
        }
        value >>= 1;
    }
    total
}

/// Shifts a history along and puts the new value at the front.
fn shift(value: f64, hist: &mut [f64]) {
    for i in (1..hist.len()).rev() {
        hist[i] = hist[i - 1];
    }
    hist[0] = value;
}

/// One pass of a filter: `a` over the inputs, `b` over the outputs.
fn apply(x: &[f64], y: &[f64], a: &[f64], b: &[f64]) -> f64 {
    let mut total = 0.0;
    for (xi, ai) in x.iter().zip(a) {
        total += xi * ai;
    }
    for (yi, bi) in y.iter().zip(&b[1..]) {
        total -= yi * bi;
    }
    total
}

impl Default for Votrax {
    fn default() -> Self {
        Self::new(48_000)
    }
}

impl Votrax {
    pub fn new(host_rate: u32) -> Self {
        let clock = f64::from(NOMINAL_CLOCK);
        let mut chip = Self {
            chip: Chip::Sc01A,
            mainclock: clock,
            sclock: clock / 18.0,
            cclock: clock / 36.0,
            inflection: 0,
            // There is no reset on this chip and its state at power-on is
            // random. Idle and silent is a sane place to start it.
            phone: 0x3F,
            ar_state: true,
            timer: None,
            rom_duration: 0,
            rom_vd: 0,
            rom_cld: 0,
            rom_fa: 0,
            rom_fc: 0,
            rom_va: 0,
            rom_f1: 0,
            rom_f2: 0,
            rom_f2q: 0,
            rom_f3: 0,
            rom_closure: false,
            rom_pause: false,
            cur_fa: 0,
            cur_fc: 0,
            cur_va: 0,
            cur_f1: 0,
            cur_f2: 0,
            cur_f2q: 0,
            cur_f3: 0,
            filt_fa: 0,
            filt_fc: 0,
            filt_va: 0,
            filt_f1: 0,
            filt_f2: 0,
            filt_f2q: 0,
            filt_f3: 0,
            phonetick: 0,
            ticks: 0,
            pitch: 0,
            closure: 0,
            update_counter: 0,
            cur_closure: true,
            noise: 0,
            cur_noise: false,
            sample_count: 0,
            voice_1: [0.0; 4],
            voice_2: [0.0; 4],
            voice_3: [0.0; 4],
            noise_1: [0.0; 3],
            noise_2: [0.0; 3],
            noise_3: [0.0; 2],
            noise_4: [0.0; 2],
            vn_1: [0.0; 4],
            vn_2: [0.0; 4],
            vn_3: [0.0; 4],
            vn_4: [0.0; 4],
            vn_5: [0.0; 2],
            vn_6: [0.0; 2],
            f1_a: [0.0; 4],
            f1_b: [0.0; 4],
            f2v_a: [0.0; 4],
            f2v_b: [0.0; 4],
            f2n_a: [0.0; 2],
            f2n_b: [0.0; 2],
            f3_a: [0.0; 4],
            f3_b: [0.0; 4],
            f4_a: [0.0; 4],
            f4_b: [0.0; 4],
            fx_a: [0.0; 1],
            fx_b: [0.0; 2],
            fn_a: [0.0; 3],
            fn_b: [0.0; 3],
            muted: false,
            host_rate: f64::from(host_rate),
            owed: 0.0,
            last: 0.0,
            sum: 0.0,
            taken: 0,
        };
        chip.phone_commit();
        chip.filters_commit(true);
        chip
    }

    /// Which of the two chips this is.
    pub fn set_chip(&mut self, which: Chip) {
        self.chip = which;
        self.phone_commit();
        self.filters_commit(true);
    }

    pub fn set_host_rate(&mut self, rate: u32) {
        self.host_rate = f64::from(rate);
    }

    /// The clock, which on Gottlieb's board comes from a converter.
    pub fn set_clock(&mut self, hz: u32) {
        let hz = f64::from(hz.max(1));
        if (hz - self.mainclock).abs() < f64::EPSILON {
            return;
        }
        self.mainclock = hz;
        self.sclock = hz / 18.0;
        self.cclock = hz / 36.0;
        self.filters_commit(true);
    }

    pub fn clock(&self) -> u32 {
        self.mainclock as u32
    }

    /// The busy line the board reads and interrupts on.
    pub fn busy(&self) -> bool {
        !self.ar_state
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The name of the phoneme in progress.
    pub fn speaking(&self) -> &'static str {
        NAMES[usize::from(self.phone & 0x3F)]
    }

    /// Writes a phoneme: six bits of it, and two of inflection above them.
    pub fn write(&mut self, value: u8) {
        self.inflection = (value >> 6) & 0x03;
        self.phone = value & 0x3F;
        self.ar_state = false;

        // A commit is scheduled about a tenth of a millisecond out — one phi1
        // transition and the ROM's extra state — unless one is already on its
        // way. It overrides a pending end-of-phoneme, which is what the strobe
        // does on the chip anyway.
        if !matches!(self.timer, Some((Pending::Commit, _))) {
            self.timer = Some((Pending::Commit, 72.0));
        }
    }

    /// Latches the phoneme's own ROM values.
    ///
    /// On the die the ROM is re-read continuously. It is immutable, so there is
    /// no point in not caching it.
    fn phone_commit(&mut self) {
        // Only these two are reset on a phoneme change. The rest run free.
        self.phonetick = 0;
        self.ticks = 0;

        let rom: &[u8; 512] = match self.chip {
            Chip::Sc01 => &SC01,
            Chip::Sc01A => &SC01A,
        };
        for entry in rom.as_chunks::<8>().0 {
            let val = u64::from_le_bytes(*entry);
            if self.phone != ((val >> 56) & 0x3F) as u8 {
                continue;
            }
            self.rom_f1 = bitswap4(val, 0, 7, 14, 21);
            self.rom_va = bitswap4(val, 1, 8, 15, 22);
            self.rom_f2 = bitswap4(val, 2, 9, 16, 23);
            self.rom_fc = bitswap4(val, 3, 10, 17, 24);
            self.rom_f2q = bitswap4(val, 4, 11, 18, 25);
            self.rom_f3 = bitswap4(val, 5, 12, 19, 26);
            self.rom_fa = bitswap4(val, 6, 13, 20, 27);

            // These two have their bits the other way round from everything
            // else: the prototype miswired the comparator against the tick
            // count, and they compensated in the ROM rather than in silicon.
            self.rom_cld = bitswap4(val, 34, 32, 30, 28);
            self.rom_vd = bitswap4(val, 35, 33, 31, 29);

            self.rom_closure = bit(val, 36) != 0;
            self.rom_duration = bitswap7(!val, 37, 38, 39, 40, 41, 42, 43);

            // Hard-wired on the die rather than held in the ROM: PA0 and PA1
            // are the two silences.
            self.rom_pause = self.phone == 0x03 || self.phone == 0x3E;

            if self.rom_cld == 0 {
                self.cur_closure = self.rom_closure;
            }
            return;
        }
    }

    /// One step of interpolation: an eighth of the way to the target.
    fn interpolate(reg: &mut u8, target: u8) {
        *reg = reg.wrapping_sub(*reg >> 3).wrapping_add(target << 1);
    }

    /// One tick of the chip's digital half, at the capacitor clock.
    fn chip_update(&mut self) {
        // The phoneme tick counter, stopped once the ticks reach sixteen. The
        // counter itself keeps going; it is the comparator that is disabled.
        if self.ticks != 0x10 {
            self.phonetick += 1;
            // Compared against the duration times four, with one tick of delay
            // in the path.
            if self.phonetick == ((u16::from(self.rom_duration) << 2) | 1) {
                self.phonetick = 0;
                self.ticks += 1;
                if self.ticks == self.rom_cld {
                    self.cur_closure = self.rom_closure;
                }
            }
        }

        // Two update counters, one dividing by sixteen and one by forty-eight,
        // phased so the slower ticks exactly between two of the faster.
        self.update_counter += 1;
        if self.update_counter == 0x30 {
            self.update_counter = 0;
        }
        let tick_625 = self.update_counter & 0x0F == 0;
        let tick_208 = self.update_counter == 0x28;

        // The formants. There is a bug on the die here — `fc` is updated where
        // `va` should be — and it is kept, because it is what the chip does.
        // On a pause the formants freeze unless both volumes are already zero.
        if tick_208 && (!self.rom_pause || (self.filt_fa == 0 && self.filt_va == 0)) {
            Self::interpolate(&mut self.cur_fc, self.rom_fc);
            Self::interpolate(&mut self.cur_f1, self.rom_f1);
            Self::interpolate(&mut self.cur_f2, self.rom_f2);
            Self::interpolate(&mut self.cur_f2q, self.rom_f2q);
            Self::interpolate(&mut self.cur_f3, self.rom_f3);
        }

        // And everything else, with the other half of the same die bug.
        if tick_625 {
            if self.ticks >= self.rom_vd {
                Self::interpolate(&mut self.cur_fa, self.rom_fa);
            }
            if self.ticks >= self.rom_cld {
                Self::interpolate(&mut self.cur_va, self.rom_va);
            }
        }

        // The glottal closure counter. Its level reaches the analog path at
        // once — there is no synchronising it to the pitch.
        if !self.cur_closure && (self.filt_fa != 0 || self.filt_va != 0) {
            self.closure = 0;
        } else if self.closure != (7 << 2) {
            self.closure += 1;
        }

        // The pitch counter. An equality comparison, so it can be made to miss
        // by moving the inflection inputs — but it wraps. The +2 is the delay
        // in the path.
        self.pitch = self.pitch.wrapping_add(1);
        if self.pitch == (0xE0u8 ^ (self.inflection << 5) ^ (self.filt_f1 << 1)).wrapping_add(2) {
            self.pitch = 0;
        }

        // The filters are rebuilt at index one of the pitch wave, which does
        // indeed mean four times in a row.
        if self.pitch & 0xF9 == 0x08 {
            self.filters_commit(false);
        }

        // The noise register: fifteen bits, with an exclusive-nor on the top
        // two for the loop.
        let inp = u16::from(self.cur_noise && self.noise != 0x7FFF);
        self.noise = ((self.noise << 1) & 0x7FFE) | inp;
        self.cur_noise = ((self.noise >> 14) ^ (self.noise >> 13)) & 1 == 0;
    }

    /// Produces one sample at the host's rate.
    ///
    /// The chip's own rate is an eighteenth of its clock, which the board
    /// moves about, so the two never line up: chip samples are accumulated and
    /// averaged into each host one, which decimates when the chip is running
    /// faster than the host and holds when it is running slower.
    pub fn sample(&mut self) -> f32 {
        self.owed += self.sclock / self.host_rate;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            self.advance();
            self.sample_count = self.sample_count.wrapping_add(1);
            if self.sample_count & 1 != 0 {
                self.chip_update();
            }
            self.last = self.analog_calc();
            self.sum += self.last;
            self.taken += 1;
        }
        let out = if self.taken > 0 {
            self.sum / f64::from(self.taken)
        } else {
            self.last
        };
        self.sum = 0.0;
        self.taken = 0;
        if self.muted { 0.0 } else { out as f32 }
    }

    /// Runs the chip's own timer forward by one of its samples.
    ///
    /// Two events, and between them the whole handshake: a write pulls the
    /// busy line down and schedules the commit, the commit latches the ROM and
    /// schedules the end, and the end lets the line go — which is what
    /// interrupts the board and asks for the next phoneme.
    fn advance(&mut self) {
        let Some((what, left)) = self.timer else {
            return;
        };
        let left = left - 18.0;
        if left > 0.0 {
            self.timer = Some((what, left));
            return;
        }
        match what {
            Pending::Commit => {
                self.phone_commit();
                let ticks = 16.0 * (f64::from(self.rom_duration) * 4.0 + 1.0) * 4.0 * 9.0 + 2.0;
                self.timer = Some((Pending::End, ticks));
            }
            Pending::End => {
                self.ar_state = true;
                self.timer = None;
            }
        }
    }
}

impl Votrax {
    /// A filter in the shape most of the chip's are: two op-amps, a switched
    /// capacitor pair on the input and another over the first amplifier.
    ///
    /// The capacitor values appear on both sides of the division, which is
    /// what says the capacitance per unit area is not needed — only the areas
    /// are, and those are what came off the die.
    #[allow(clippy::too_many_arguments)]
    fn build_standard_filter(
        &self,
        a: &mut [f64; 4],
        b: &mut [f64; 4],
        c1t: u32, // unswitched, input, top
        c1b: u32, // switched, input, bottom
        c2t: u32, // unswitched, over the first amp, top
        c2b: u32, // switched, over the first amp, bottom
        c3: u32,  // between the two amps
        c4: u32,  // over the second amp
    ) {
        let (cclock, sclock) = (self.cclock, self.sclock);
        let k0 = f64::from(c1t) / (cclock * f64::from(c1b));
        let k1 = (u64::from(c4) * u64::from(c2t)) as f64
            / (cclock * (u64::from(c1b) * u64::from(c3)) as f64);
        let k2 = (u64::from(c4) * u64::from(c2b)) as f64
            / (cclock * cclock * (u64::from(c1b) * u64::from(c3)) as f64);

        // Radians from the peak frequency.
        let num =
            i64::from(c3) * (i64::from(c1t) * i64::from(c2t) - i64::from(c1b) * i64::from(c2b));
        let den = (u64::from(c2b) * u64::from(c4) * u64::from(c2b)) as f64;
        let mut w0 = (cclock * cclock * (num as f64) / den).abs().sqrt();

        // Keep it inside -pi/T .. pi/T.
        let wd_max = std::f64::consts::PI * sclock;
        if w0 > wd_max {
            w0 -= 2.0 * wd_max;
        }

        // Pre-warp, then the z-transform.
        let zc = w0 / (w0 / (2.0 * sclock)).tan();
        let (m0, m1, m2) = (zc * k0, zc * k1, zc * zc * k2);
        *a = [1.0 + m0, 3.0 + m0, 3.0 - m0, 1.0 - m0];
        *b = [
            1.0 + (m1 + m2),
            3.0 + (m1 - m2),
            3.0 - (m1 + m2),
            1.0 - (m1 - m2),
        ];
        normalise4(a, b);
    }

    /// The much simpler one used once at the end: a resistor and a capacitor
    /// over a single amplifier.
    fn build_lowpass_filter(&self, a: &mut [f64; 1], b: &mut [f64; 2], c1t: u32, c1b: u32) {
        // The capacitors put the corner near 150 Hz, which is no good;
        // recordings of the chip want it around 4 kHz, so it is fudged by that
        // ratio. Galibert's fudge, kept, because his is the voice that matches
        // the recordings.
        let k = f64::from(c1b) / (self.cclock * f64::from(c1t)) * (150.0 / 4000.0);
        let mut w0 = 1.0 / k;
        let wd_max = std::f64::consts::PI * self.sclock;
        if w0 > wd_max {
            w0 -= 2.0 * wd_max;
        }
        let zc = w0 / (w0 / (2.0 * self.sclock)).tan();
        let m = zc * k;
        a[0] = 1.0;
        *b = [1.0 + m, 1.0 - m];
        a[0] /= b[0];
        b[1] /= b[0];
        b[0] = 1.0;
    }

    /// What shapes the white noise before it is let into the voice.
    #[allow(clippy::too_many_arguments)]
    fn build_noise_shaper_filter(
        &self,
        a: &mut [f64; 3],
        b: &mut [f64; 3],
        c1: u32,  // over the first amp
        c2t: u32, // unswitched, between the amps, input, top
        c2b: u32, // switched, between the amps, input, bottom
        c3: u32,  // over the second amp
        c4: u32,  // switched, after the second amp
    ) {
        let k0 = (u64::from(c2t) * u64::from(c3) * u64::from(c2b)) as f64 / f64::from(c4);
        let k1 = (u64::from(c2t) * u64::from(c2b)) as f64 * self.cclock;
        let k2 =
            (u64::from(c1) * u64::from(c2t) * u64::from(c3)) as f64 / (self.cclock * f64::from(c4));

        let mut w0 = k2.powf(-0.5);
        let wd_max = std::f64::consts::PI * self.sclock;
        if w0 > wd_max {
            w0 -= 2.0 * wd_max;
        }
        let zc = w0 / (w0 / (2.0 * self.sclock)).tan();
        let (m0, m1, m2) = (zc * k0, zc * k1, zc * zc * k2);
        *a = [m0, 0.0, -m0];
        *b = [1.0 + m1 + m2, 2.0 - 2.0 * m2, 1.0 - m1 + m2];
        let d = b[0];
        for x in a.iter_mut() {
            *x /= d;
        }
        b[1] /= d;
        b[2] /= d;
        b[0] = 1.0;
    }

    /// How the noise is let into F2, which is a path of its own.
    ///
    /// `c4` is a capacitor of the circuit that factors out of the transfer
    /// function; it is named so the call reads like the schematic.
    #[allow(clippy::too_many_arguments)]
    fn build_injection_filter(
        &self,
        a: &mut [f64; 2],
        b: &mut [f64; 2],
        c1b: u32,
        c2t: u32,
        c2b: u32,
        c3: u32,
        _c4: u32,
    ) {
        let k0 = self.cclock * f64::from(c2t);
        let k1 = self.cclock
            * ((i64::from(c1b) * i64::from(c3) - i64::from(c2t) * i64::from(c2t)) as f64
                / f64::from(c2t));
        let k2 = f64::from(c2b);

        // No pre-warping here.
        let zc = 2.0 * self.sclock;
        let m = zc * k2;
        *a = [k0 + m, k0 - m];
        *b = [k1 + m, k1 - m];
        a[0] /= b[0];
        a[1] /= b[0];
        b[1] /= b[0];
        b[0] = 1.0;
    }

    /// Rebuilds whichever filters the interpolated ROM values have moved.
    fn filters_commit(&mut self, force: bool) {
        self.filt_fa = self.cur_fa >> 4;
        self.filt_fc = self.cur_fc >> 4;
        self.filt_va = self.cur_va >> 4;

        if force || self.filt_f1 != self.cur_f1 >> 4 {
            self.filt_f1 = self.cur_f1 >> 4;
            let (mut a, mut b) = (self.f1_a, self.f1_b);
            self.build_standard_filter(
                &mut a,
                &mut b,
                11247,
                11797,
                949,
                52067,
                2280 + bits_to_caps(self.filt_f1, &F1_CAPS),
                166272,
            );
            self.f1_a = a;
            self.f1_b = b;
        }

        if force || self.filt_f2 != self.cur_f2 >> 3 || self.filt_f2q != self.cur_f2q >> 4 {
            self.filt_f2 = self.cur_f2 >> 3;
            self.filt_f2q = self.cur_f2q >> 4;

            let (mut a, mut b) = (self.f2v_a, self.f2v_b);
            self.build_standard_filter(
                &mut a,
                &mut b,
                24840,
                29154,
                829 + bits_to_caps(self.filt_f2q, &F2V1_CAPS),
                38180,
                2352 + bits_to_caps(self.filt_f2, &F2V2_CAPS),
                34270,
            );
            self.f2v_a = a;
            self.f2v_b = b;

            let (mut a, mut b) = (self.f2n_a, self.f2n_b);
            self.build_injection_filter(
                &mut a,
                &mut b,
                29154,
                829 + bits_to_caps(self.filt_f2q, &F2N1_CAPS),
                38180,
                2352 + bits_to_caps(self.filt_f2, &F2N2_CAPS),
                34270,
            );
            self.f2n_a = a;
            self.f2n_b = b;
        }

        if force || self.filt_f3 != self.cur_f3 >> 4 {
            self.filt_f3 = self.cur_f3 >> 4;
            let (mut a, mut b) = (self.f3_a, self.f3_b);
            self.build_standard_filter(
                &mut a,
                &mut b,
                0,
                17594,
                868,
                18828,
                8480 + bits_to_caps(self.filt_f3, &F3_CAPS),
                50019,
            );
            self.f3_a = a;
            self.f3_b = b;
        }

        if force {
            let (mut a, mut b) = (self.f4_a, self.f4_b);
            self.build_standard_filter(&mut a, &mut b, 0, 28810, 1165, 21457, 8558, 7289);
            self.f4_a = a;
            self.f4_b = b;

            let (mut a, mut b) = (self.fx_a, self.fx_b);
            self.build_lowpass_filter(&mut a, &mut b, 1122, 23131);
            self.fx_a = a;
            self.fx_b = b;

            let (mut a, mut b) = (self.fn_a, self.fn_b);
            self.build_noise_shaper_filter(&mut a, &mut b, 15500, 14854, 8450, 9523, 14083);
            self.fn_a = a;
            self.fn_b = b;
        }
    }

    /// One sample of the chip's analog half.
    fn analog_calc(&mut self) -> f64 {
        // The voice on its own.
        // 1. Pick the pitch wave up.
        let mut v = if self.pitch >= (9 << 3) {
            0.0
        } else {
            GLOTTAL_WAVE[usize::from(self.pitch >> 3)]
        };

        // 2. Through the first amplifier, which is linear on the die even
        //    though the patent does not say so.
        v *= f64::from(self.filt_va) * (1.0 / 15.0);
        shift(v, &mut self.voice_1);

        // 3. F1.
        v = apply(&self.voice_1, &self.voice_2, &self.f1_a, &self.f1_b);
        shift(v, &mut self.voice_2);

        // 4. F2, the voice half of it.
        v = apply(&self.voice_2, &self.voice_3, &self.f2v_a, &self.f2v_b);
        shift(v, &mut self.voice_3);

        // The noise on its own.
        // 5. Its pitch, at a linear amplitude.
        let gate = self.pitch & 0x40 != 0 && self.cur_noise;
        let mut n = 1e4 * if gate { 1.0 } else { -1.0 };
        n *= f64::from(self.filt_fa) * (1.0 / 15.0);
        shift(n, &mut self.noise_1);

        // 6. The noise shaper.
        n = apply(&self.noise_1, &self.noise_2, &self.fn_a, &self.fn_b);
        shift(n, &mut self.noise_2);

        // 7. Scaled by what F2 will take of it.
        let mut n2 = n * f64::from(self.filt_fc) * (1.0 / 15.0);
        shift(n2, &mut self.noise_3);

        // 8. F2, the noise half.
        n2 = apply(&self.noise_3, &self.noise_4, &self.f2n_a, &self.f2n_b);
        shift(n2, &mut self.noise_4);

        // And the two together from here.
        // 9. Add the two halves of F2.
        let mut vn = v + n2;
        shift(vn, &mut self.vn_1);

        // 10. F3.
        vn = apply(&self.vn_1, &self.vn_2, &self.f3_a, &self.f3_b);
        shift(vn, &mut self.vn_2);

        // 11. A second helping of noise.
        vn += n * f64::from(5 + (15 ^ self.filt_fc)) * (1.0 / 20.0);
        shift(vn, &mut self.vn_3);

        // 12. F4, which is fixed.
        vn = apply(&self.vn_3, &self.vn_4, &self.f4_a, &self.f4_b);
        shift(vn, &mut self.vn_4);

        // 13. The glottal closure amplitude, linear again.
        vn *= f64::from(7 ^ (self.closure >> 2)) * (1.0 / 7.0);
        shift(vn, &mut self.vn_5);

        // 14. And the fixed filter on the way out.
        vn = apply(&self.vn_5, &self.vn_6, &self.fx_a, &self.fx_b);
        shift(vn, &mut self.vn_6);

        // Room to spare, so the loudest phonemes do not clip.
        vn * (1.0 / 2.4)
    }
}

/// Divides a four-tap pair through by `b[0]`, which is how every one of these
/// filters is left.
fn normalise4(a: &mut [f64; 4], b: &mut [f64; 4]) {
    let d = b[0];
    for x in a.iter_mut() {
        *x /= d;
    }
    b[1] /= d;
    b[2] /= d;
    b[3] /= d;
    b[0] = 1.0;
}
