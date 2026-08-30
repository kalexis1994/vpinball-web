//! The General Instrument SP0250, the speech chip of a first-generation
//! System 80B sound card.
//!
//! Where the Votrax next door spells words out of sixty-four fixed phonemes,
//! this one is told everything: it is an **LPC synthesiser**, and the game
//! feeds it fifteen bytes at a time describing a vocal tract — six filter
//! stages, an amplitude, a pitch, and how many frames to hold them for. Speech
//! is a stream of those frames. It says whatever it is given, in whatever
//! voice the frames describe, which is why a Gottlieb from 1986 sounds less
//! like a robot than one from 1981.
//!
//! # Where this comes from
//!
//! A port of PinMAME's `src/sound/sp0250.c`, which carries
//! `// license:BSD-3-Clause` and names Olivier Galibert as its author. The
//! coefficient table below is the chip's own internal ROM.
//!
//! > Copyright the MAME and PinMAME authors, Olivier Galibert for this
//! > implementation. Redistribution and use in source and binary forms, with
//! > or without modification, are permitted provided that the conditions of
//! > the BSD-3-Clause licence are met. This software is provided by the
//! > copyright holders "as is" and any express or implied warranties are
//! > disclaimed.

/// How many bytes make one frame of speech.
const FRAME: usize = 15;

/// How many filter stages the vocal tract is modelled with.
const STAGES: usize = 6;

/// The chip's clock and what it divides down to.
///
/// Three point one two megahertz in, and one sample out every three hundred
/// and twelve of them: ten thousand a second, which is the rate the original
/// speech was sampled at.
const CLOCK: u32 = 3_120_000;
const DIVIDER: u32 = 312;

/// The chip's own coefficient ROM, read by the filter parameters.
#[rustfmt::skip]
const COEFFICIENTS: [u16; 128] = [
      0,   9,  17,  25,  33,  41,  49,  57,  65,  73,  81,  89,  97, 105, 113, 121,
    129, 137, 145, 153, 161, 169, 177, 185, 193, 201, 209, 217, 225, 233, 241, 249,
    257, 265, 273, 281, 289, 297, 301, 305, 309, 313, 317, 321, 325, 329, 333, 337,
    341, 345, 349, 353, 357, 361, 365, 369, 373, 377, 381, 385, 389, 393, 397, 401,
    405, 409, 413, 417, 421, 425, 427, 429, 431, 433, 435, 437, 439, 441, 443, 445,
    447, 449, 451, 453, 455, 457, 459, 461, 463, 465, 467, 469, 471, 473, 475, 477,
    479, 481, 482, 483, 484, 485, 486, 487, 488, 489, 490, 491, 492, 493, 494, 495,
    496, 497, 498, 499, 500, 501, 502, 503, 504, 505, 506, 507, 508, 509, 510, 511,
];

/// An amplitude, which the chip stores as five bits and a shift.
fn amplitude(v: u8) -> i16 {
    (u16::from(v & 0x1F) << (v >> 5)) as i16
}

/// A filter coefficient: an index into the ROM, and a sign in the top bit.
fn coefficient(v: u8) -> i16 {
    let value = COEFFICIENTS[usize::from(v & 0x7F)] as i16;
    if v & 0x80 == 0 { -value } else { value }
}

/// One stage of the vocal tract.
#[derive(Debug, Clone, Copy, Default)]
struct Stage {
    f: i16,
    b: i16,
    z1: i16,
    z2: i16,
}

/// One SP0250.
#[derive(Debug, Clone)]
pub struct Sp0250 {
    amp: i16,
    pitch: u8,
    repeat: u8,
    pcount: i32,
    rcount: i32,
    lfsr: u16,
    voiced: bool,

    fifo: [u8; FRAME],
    fifo_pos: usize,

    filter: [Stage; STAGES],

    /// The host's rate, and where the interpolation between two of the chip's
    /// own samples has got to.
    rate: f64,
    phase: f64,
    previous: f32,
    next: f32,
}

impl Sp0250 {
    pub fn new(rate: u32) -> Self {
        let mut chip = Self {
            amp: 0,
            pitch: 0,
            repeat: 0,
            pcount: 0,
            rcount: 0,
            lfsr: 0x7FFF,
            voiced: false,
            fifo: [0; FRAME],
            fifo_pos: 0,
            filter: [Stage::default(); STAGES],
            rate: f64::from(rate),
            phase: 0.0,
            previous: 0.0,
            next: 0.0,
        };
        chip.next = chip.step();
        chip
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.rate = f64::from(rate);
        self.phase = 0.0;
    }

    /// The chip's own sample rate: ten thousand a second.
    pub fn native_rate() -> u32 {
        CLOCK / DIVIDER
    }

    /// Whether the chip wants another byte, which is the DATA REQUEST pin.
    ///
    /// The board reads it in the top bit of a port and feeds the chip fifteen
    /// bytes whenever it is high. It goes low when the frame is full and comes
    /// back the moment the chip takes it.
    pub fn wants_data(&self) -> bool {
        self.fifo_pos != FRAME
    }

    /// Hands the chip one byte of a frame. Bytes past the fifteenth are
    /// dropped, which is what the pin above is for.
    pub fn write(&mut self, data: u8) {
        if self.fifo_pos != FRAME {
            self.fifo[self.fifo_pos] = data;
            self.fifo_pos += 1;
        }
    }

    /// Takes the fifteen bytes and becomes the vocal tract they describe.
    fn load(&mut self) {
        self.filter[0].b = coefficient(self.fifo[0]);
        self.filter[0].f = coefficient(self.fifo[1]);
        self.amp = amplitude(self.fifo[2]);
        self.filter[1].b = coefficient(self.fifo[3]);
        self.filter[1].f = coefficient(self.fifo[4]);
        self.pitch = self.fifo[5];
        self.filter[2].b = coefficient(self.fifo[6]);
        self.filter[2].f = coefficient(self.fifo[7]);
        self.repeat = self.fifo[8] & 0x3F;
        self.voiced = self.fifo[8] & 0x40 != 0;
        self.filter[3].b = coefficient(self.fifo[9]);
        self.filter[3].f = coefficient(self.fifo[10]);
        self.filter[4].b = coefficient(self.fifo[11]);
        self.filter[4].f = coefficient(self.fifo[12]);
        self.filter[5].b = coefficient(self.fifo[13]);
        self.filter[5].f = coefficient(self.fifo[14]);

        self.fifo_pos = 0;
        self.pcount = 0;
        self.rcount = 0;
        for stage in &mut self.filter {
            stage.z1 = 0;
            stage.z2 = 0;
        }
    }

    /// One sample at the chip's own ten thousand a second.
    fn step(&mut self) -> f32 {
        if self.rcount >= i32::from(self.repeat) {
            if self.fifo_pos == FRAME {
                self.load();
            } else {
                // Nothing to say yet. The chip idles on frames of one, at
                // whatever pitch it last had, rather than stopping.
                self.repeat = 1;
                self.pcount = 0;
                self.rcount = 0;
            }
        }

        // Fifteen bits, and it runs whether the frame is voiced or not.
        self.lfsr ^= (self.lfsr ^ (self.lfsr >> 1)) << 15;
        self.lfsr >>= 1;

        // A voiced frame is a click once a pitch period; an unvoiced one is
        // the noise at full amplitude either way up.
        let mut z0 = if self.voiced {
            if self.pcount == 0 { self.amp } else { 0 }
        } else if self.lfsr & 1 != 0 {
            self.amp
        } else {
            -self.amp
        };

        // Six stages in a row, each feeding the next. The arithmetic is the
        // chip's: sixteen bits wide, and what runs off the top runs off.
        for stage in &mut self.filter {
            let acc = i32::from(z0)
                + ((i32::from(stage.z1) * i32::from(stage.f)) >> 8)
                + ((i32::from(stage.z2) * i32::from(stage.b)) >> 9);
            z0 = acc as i16;
            stage.z2 = stage.z1;
            stage.z1 = z0;
        }

        self.pcount += 1;
        if self.pcount - 1 == i32::from(self.pitch) {
            self.pcount = 0;
            self.rcount += 1;
        }

        // Seven bits of real resolution, pushed up to fill the room there is.
        let dac = (i32::from(z0) << 3).clamp(-32768, 32767);
        dac as f32 / 32768.0
    }

    /// One sample at the host's rate.
    ///
    /// Ten thousand a second is well under what a host wants, so this is
    /// interpolation rather than decimation — and the chip's own output is
    /// band-limited to five kilohertz, so a straight line between two of its
    /// samples is close to the right answer.
    pub fn sample(&mut self) -> f32 {
        self.phase += f64::from(Self::native_rate()) / self.rate;
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.previous = self.next;
            self.next = self.step();
        }
        let t = self.phase as f32;
        self.previous + (self.next - self.previous) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame the chip will accept: an amplitude, a pitch, a repeat count and
    /// six stages of filter.
    fn frame(amp: u8, pitch: u8, repeat: u8, voiced: bool) -> [u8; FRAME] {
        let mut f = [0x80u8; FRAME];
        f[2] = amp;
        f[5] = pitch;
        f[8] = repeat | if voiced { 0x40 } else { 0 };
        f
    }

    #[test]
    fn it_asks_for_a_frame_and_stops_asking_when_it_has_one() {
        let mut chip = Sp0250::new(48_000);
        assert!(chip.wants_data(), "it wants bytes from the start");
        for byte in frame(0x9F, 40, 4, true) {
            assert!(chip.wants_data());
            chip.write(byte);
        }
        assert!(!chip.wants_data(), "and stops once the frame is full");

        // It takes the frame on its next sample and asks for the next one.
        // The chip runs at ten thousand a second and the host at forty-eight,
        // so a handful of the host's go by before one of the chip's does.
        for _ in 0..50 {
            chip.sample();
        }
        assert!(chip.wants_data());
    }

    #[test]
    fn a_frame_makes_a_noise_and_an_empty_chip_does_not() {
        let mut chip = Sp0250::new(48_000);
        let quiet: f32 = (0..4_800).map(|_| chip.sample().abs()).fold(0.0, f32::max);
        assert_eq!(quiet, 0.0, "nothing said is nothing heard");

        for byte in frame(0x9F, 40, 20, false) {
            chip.write(byte);
        }
        let loud: f32 = (0..4_800).map(|_| chip.sample().abs()).fold(0.0, f32::max);
        assert!(
            loud > 0.01,
            "an unvoiced frame should hiss, and peaked at {loud}"
        );
    }

    #[test]
    fn a_voiced_frame_clicks_once_a_pitch_period() {
        let mut chip = Sp0250::new(Sp0250::native_rate());
        for byte in frame(0x9F, 50, 60, true) {
            chip.write(byte);
        }
        // Count the peaks: at ten thousand samples a second and a pitch of
        // fifty-one, that is about two hundred a second.
        let mut peaks = 0;
        let mut previous = 0.0f32;
        let mut rising = false;
        for _ in 0..10_000 {
            let s = chip.sample();
            if s > previous + 0.001 {
                rising = true;
            } else if rising && s < previous {
                peaks += 1;
                rising = false;
            }
            previous = s;
        }
        assert!(
            peaks > 50,
            "a voiced frame should be periodic, and had {peaks} peaks"
        );
    }
}
