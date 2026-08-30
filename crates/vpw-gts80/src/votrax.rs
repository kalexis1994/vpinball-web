//! The Votrax SC-01, the chip that gives these machines a voice.
//!
//! It is not a sampler and there is no recorded speech anywhere in the
//! machine. The SC-01 is a **formant synthesiser**: sixty-four phonemes, and
//! the sound board spells words out of them one at a time. You write a
//! phoneme, the chip raises its busy line and takes about a tenth of a second
//! to say it, and when the line drops it wants the next one. That is the whole
//! interface, and it is why a Gottlieb says "BLACK HOLE" in the voice it does:
//! nobody recorded it, it was spelled.
//!
//! # What is measured here and what is modelled
//!
//! Worth being straight about, because the two are not the same.
//!
//! **Measured**, from Gottlieb's board and the chip's own documentation: the
//! sixty-four phoneme codes and their order; that the low six bits are the
//! phoneme and the top two an inflection; that the board writes them inverted;
//! that the chip runs from a clock the board sets with a second DAC, nominally
//! 720 kHz, and that lowering it slows and deepens the voice — which is how
//! these games do the falling "TILT" (`gts80s.c:392`).
//!
//! **Modelled**: the sound itself. A three-resonator formant synthesiser,
//! excited by a pulse train for the voiced phonemes and by noise for the rest,
//! with the formant frequencies taken from the ordinary published values for
//! English phones and the durations grouped by class rather than given one by
//! one. That is the same *kind* of machine the SC-01 is, and it speaks with
//! the same phonemes at the same pace, but it is a reconstruction from the
//! outside and not a copy of a particular chip: expect a robot from 1980,
//! rather than *this* robot from 1980.

/// The nominal clock, in Hz — what the board sets when it wants ordinary
/// speech (`gts80s.c:395`, measured on real hardware by flipprojets).
pub const NOMINAL_CLOCK: f32 = 720_000.0;

/// What a phoneme is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A vowel or a glide: the cords are working and the formants carry it.
    Voiced,
    /// A nasal — voiced, but quieter and duller, because the sound is coming
    /// out of the nose.
    Nasal,
    /// A hiss with no voice behind it.
    Fricative,
    /// A hiss with voice behind it.
    VoicedFricative,
    /// A burst after a silence.
    Stop,
    /// The same with the cords running.
    VoicedStop,
    /// Silence, which is a phoneme here: words need gaps and the board asks
    /// for them by name.
    Pause,
}

/// One of the sixty-four.
struct Phoneme {
    /// The name Votrax gives it, which is what the sound ROM's author typed.
    name: &'static str,
    kind: Kind,
    /// The first three formants, in Hz at the nominal clock.
    formants: [f32; 3],
    /// How long it lasts at the nominal clock, in milliseconds.
    ms: f32,
}

const fn p(name: &'static str, kind: Kind, f1: f32, f2: f32, f3: f32, ms: f32) -> Phoneme {
    Phoneme {
        name,
        kind,
        formants: [f1, f2, f3],
        ms,
    }
}

use Kind::{Fricative, Nasal, Pause, Stop, Voiced, VoicedFricative, VoicedStop};

/// The phoneme set, in the chip's own code order.
///
/// The order is the chip's and the names are Votrax's. A sound ROM is written
/// against these numbers, so this table is the one thing here that has to be
/// exactly right — a shuffled table does not sound slightly wrong, it says
/// different words.
#[rustfmt::skip]
const PHONEMES: [Phoneme; 64] = [
    /* 00 */ p("EH3", Voiced,          570.0, 1900.0, 2500.0,  59.0),
    /* 01 */ p("EH2", Voiced,          570.0, 1900.0, 2500.0,  71.0),
    /* 02 */ p("EH1", Voiced,          570.0, 1900.0, 2500.0, 121.0),
    /* 03 */ p("PA0", Pause,             0.0,    0.0,    0.0,  47.0),
    /* 04 */ p("DT",  VoicedStop,      300.0, 1700.0, 2600.0,  47.0),
    /* 05 */ p("A2",  Voiced,          660.0, 1700.0, 2400.0,  71.0),
    /* 06 */ p("A1",  Voiced,          660.0, 1700.0, 2400.0, 103.0),
    /* 07 */ p("ZH",  VoicedFricative, 300.0, 1800.0, 2500.0,  90.0),
    /* 08 */ p("AH2", Voiced,          730.0, 1100.0, 2450.0,  71.0),
    /* 09 */ p("I3",  Voiced,          390.0, 1990.0, 2550.0,  55.0),
    /* 0A */ p("I2",  Voiced,          390.0, 1990.0, 2550.0,  80.0),
    /* 0B */ p("I1",  Voiced,          390.0, 1990.0, 2550.0, 121.0),
    /* 0C */ p("M",   Nasal,           250.0, 1100.0, 2200.0, 103.0),
    /* 0D */ p("N",   Nasal,           250.0, 1700.0, 2600.0,  80.0),
    /* 0E */ p("B",   VoicedStop,      250.0, 1100.0, 2300.0,  71.0),
    /* 0F */ p("V",   VoicedFricative, 300.0, 1100.0, 2400.0,  71.0),
    /* 10 */ p("CH",  Fricative,       350.0, 1800.0, 2600.0,  71.0),
    /* 11 */ p("SH",  Fricative,       350.0, 1800.0, 2600.0, 121.0),
    /* 12 */ p("Z",   VoicedFricative, 300.0, 1700.0, 2600.0,  71.0),
    /* 13 */ p("AW1", Voiced,          570.0,  840.0, 2410.0, 146.0),
    /* 14 */ p("NG",  Nasal,           250.0, 2000.0, 2600.0, 121.0),
    /* 15 */ p("AH1", Voiced,          730.0, 1100.0, 2450.0, 146.0),
    /* 16 */ p("OO1", Voiced,          440.0, 1020.0, 2240.0, 103.0),
    /* 17 */ p("OO",  Voiced,          300.0,  870.0, 2240.0, 185.0),
    /* 18 */ p("L",   Voiced,          360.0, 1100.0, 2600.0,  80.0),
    /* 19 */ p("K",   Stop,            300.0, 1990.0, 2850.0,  80.0),
    /* 1A */ p("J",   VoicedFricative, 300.0, 1800.0, 2500.0,  47.0),
    /* 1B */ p("H",   Fricative,       490.0, 1480.0, 2500.0,  71.0),
    /* 1C */ p("G",   VoicedStop,      250.0, 1990.0, 2850.0,  71.0),
    /* 1D */ p("F",   Fricative,       300.0, 1100.0, 2400.0, 103.0),
    /* 1E */ p("D",   VoicedStop,      300.0, 1700.0, 2600.0,  55.0),
    /* 1F */ p("S",   Fricative,       320.0, 1700.0, 2900.0,  90.0),
    /* 20 */ p("A",   Voiced,          660.0, 1700.0, 2400.0, 185.0),
    /* 21 */ p("AY",  Voiced,          530.0, 1840.0, 2480.0, 65.0),
    /* 22 */ p("Y1",  Voiced,          300.0, 2200.0, 3000.0,  80.0),
    /* 23 */ p("UH3", Voiced,          520.0, 1190.0, 2390.0,  47.0),
    /* 24 */ p("AH",  Voiced,          730.0, 1100.0, 2450.0, 250.0),
    /* 25 */ p("P",   Stop,            250.0, 1100.0, 2300.0, 103.0),
    /* 26 */ p("O",   Voiced,          570.0,  840.0, 2410.0, 185.0),
    /* 27 */ p("I",   Voiced,          390.0, 1990.0, 2550.0, 185.0),
    /* 28 */ p("U",   Voiced,          440.0, 1020.0, 2240.0, 185.0),
    /* 29 */ p("Y",   Voiced,          300.0, 2200.0, 3000.0, 103.0),
    /* 2A */ p("T",   Stop,            300.0, 1700.0, 2600.0,  71.0),
    /* 2B */ p("R",   Voiced,          420.0, 1300.0, 1600.0,  90.0),
    /* 2C */ p("E",   Voiced,          390.0, 2300.0, 3000.0, 185.0),
    /* 2D */ p("W",   Voiced,          300.0,  610.0, 2200.0,  80.0),
    /* 2E */ p("AE",  Voiced,          660.0, 1720.0, 2410.0, 185.0),
    /* 2F */ p("AE1", Voiced,          660.0, 1720.0, 2410.0, 103.0),
    /* 30 */ p("AW2", Voiced,          570.0,  840.0, 2410.0,  80.0),
    /* 31 */ p("UH2", Voiced,          520.0, 1190.0, 2390.0,  71.0),
    /* 32 */ p("UH1", Voiced,          520.0, 1190.0, 2390.0, 121.0),
    /* 33 */ p("UH",  Voiced,          640.0, 1190.0, 2390.0, 185.0),
    /* 34 */ p("O2",  Voiced,          570.0,  840.0, 2410.0,  80.0),
    /* 35 */ p("O1",  Voiced,          570.0,  840.0, 2410.0, 121.0),
    /* 36 */ p("IU",  Voiced,          350.0, 1300.0, 2300.0,  59.0),
    /* 37 */ p("U1",  Voiced,          440.0, 1020.0, 2240.0,  90.0),
    /* 38 */ p("THV", VoicedFricative, 300.0, 1400.0, 2600.0,  80.0),
    /* 39 */ p("TH",  Fricative,       300.0, 1400.0, 2600.0,  71.0),
    /* 3A */ p("ER",  Voiced,          490.0, 1350.0, 1690.0, 146.0),
    /* 3B */ p("EH",  Voiced,          570.0, 1900.0, 2500.0, 185.0),
    /* 3C */ p("E1",  Voiced,          390.0, 2300.0, 3000.0,  55.0),
    /* 3D */ p("AW",  Voiced,          570.0,  840.0, 2410.0, 250.0),
    /* 3E */ p("PA1", Pause,             0.0,    0.0,    0.0, 121.0),
    /* 3F */ p("STOP", Pause,            0.0,    0.0,    0.0,  47.0),
];

/// The code for a phoneme's name, for whoever is spelling by hand.
///
/// The names are Votrax's own, which is what a sound ROM's author would have
/// been typing: `B L AE K` is "black".
pub fn code_of(name: &str) -> Option<u8> {
    PHONEMES
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))
        .map(|i| i as u8)
}

/// How much of each glottal cycle the cords are open for.
const OPEN_QUOTIENT: f32 = 0.4;

/// What the three resonators together do to the excitation's level.
///
/// Each formant is a peak of about its own frequency over its bandwidth, and
/// three in a row multiply, so the voice needs bringing back down to somewhere
/// a loudspeaker can take. Measured on the phoneme set rather than derived:
/// this is where an ordinary vowel peaks at about half scale.
const GAIN: f32 = 0.30;

/// The voice's own pitch at the nominal clock, in Hz.
///
/// The SC-01 is a man with a cold and it is not a wide range: the two
/// inflection bits move it a little, and the board's clock DAC moves it a lot,
/// which is what makes a slowed "TILT" also a deeper one.
const PITCH: f32 = 104.0;

/// A two-pole resonator: one formant.
#[derive(Debug, Clone, Copy, Default)]
struct Resonator {
    a1: f32,
    a2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    /// Tunes it to a frequency and a bandwidth at a sample rate.
    ///
    /// The gain is set so the filter passes direct current unchanged, which is
    /// the ordinary way to normalise one of these. It does *not* make the
    /// resonance unity — a formant is a peak, and three of them in a row
    /// multiply — so the voice has an overall gain of its own further down.
    fn tune(&mut self, hz: f32, bandwidth: f32, rate: f32) {
        let r = (-std::f32::consts::PI * bandwidth / rate).exp();
        let theta = std::f32::consts::TAU * hz.min(rate * 0.45) / rate;
        self.a1 = 2.0 * r * theta.cos();
        self.a2 = -r * r;
        self.gain = 1.0 - self.a1 - self.a2;
    }

    fn step(&mut self, x: f32) -> f32 {
        let y = self.gain * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One Votrax SC-01.
pub struct Votrax {
    /// What the board's second DAC has set the chip's clock to.
    clock: f32,
    /// The output rate this is being sampled at.
    rate: f32,

    /// The phoneme being spoken, and how long is left of it in seconds.
    phoneme: u8,
    inflection: u8,
    remaining: f32,
    /// Whether the chip is busy, which is the line that drives the board's
    /// interrupt.
    busy: bool,
    /// Whether the board has muted it. A write anywhere in the ROM window
    /// does that — the firmware uses those writes as a delay loop and as an
    /// off switch both (`gts80s.c:480`).
    muted: bool,

    /// Where the glottal pulse train has got to, and the last flow through it
    /// — the excitation is the *rate of change* of that flow, which is what
    /// puts the sharp edge at the moment the cords close.
    phase: f32,
    flow: f32,
    /// The noise source, a maximal-length shift register.
    noise: u32,

    formants: [Resonator; 3],
    /// Where the formants are now and where the current phoneme wants them.
    at: [f32; 3],
    to: [f32; 3],
    /// The same for loudness, so phonemes join rather than click.
    level: f32,
    level_to: f32,
    /// How much of the excitation is noise rather than voice, 0 to 1.
    hiss: f32,
    hiss_to: f32,
    /// Whether the phoneme in progress is a stop, whose burst is at the front.
    burst: bool,
}

impl Votrax {
    pub fn new(rate: u32) -> Self {
        let mut chip = Self {
            clock: NOMINAL_CLOCK,
            rate: rate as f32,
            phoneme: 0x3F,
            inflection: 0,
            remaining: 0.0,
            busy: false,
            muted: false,
            phase: 0.0,
            flow: 0.0,
            noise: 0x1234_5678,
            formants: [Resonator::default(); 3],
            at: [500.0, 1500.0, 2500.0],
            to: [500.0, 1500.0, 2500.0],
            level: 0.0,
            level_to: 0.0,
            hiss: 0.0,
            hiss_to: 0.0,
            burst: false,
        };
        chip.set_rate(rate);
        chip
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate as f32;
        for r in &mut self.formants {
            *r = Resonator::default();
        }
    }

    /// The clock the board's second DAC is feeding the chip.
    ///
    /// `$7E` is the middle of the range and gives 720 kHz, which is where it
    /// sits for every message except the falling "TILT" (`gts80s.c:392`).
    pub fn set_clock(&mut self, latch: u8) {
        self.clock = (f32::from(latch) * 5750.0 - 4500.0).max(1.0);
    }

    /// The busy line, which is the board's interrupt.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The clock the chip is running at, in Hz.
    pub fn clock(&self) -> f32 {
        self.clock
    }

    /// Mutes or unmutes the chip's output.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The name of the phoneme being spoken, for whoever is debugging a voice.
    pub fn speaking(&self) -> &'static str {
        PHONEMES[usize::from(self.phoneme & 0x3F)].name
    }

    /// Writes a phoneme. The low six bits pick it, the top two inflect it.
    pub fn write(&mut self, value: u8) {
        self.phoneme = value & 0x3F;
        self.inflection = value >> 6;

        let sound = &PHONEMES[usize::from(self.phoneme)];
        // Every duration in the table is at the nominal clock, so a board that
        // slows the clock stretches all of them together.
        self.remaining = sound.ms * 0.001 * NOMINAL_CLOCK / self.clock;
        self.busy = self.remaining > 0.0;
        self.burst = matches!(sound.kind, Stop | VoicedStop);

        let (level, hiss) = match sound.kind {
            Voiced => (1.0, 0.0),
            Nasal => (0.6, 0.0),
            VoicedFricative => (0.5, 0.5),
            Fricative => (0.45, 1.0),
            VoicedStop => (0.8, 0.35),
            Stop => (0.6, 1.0),
            Pause => (0.0, 0.0),
        };
        self.level_to = level;
        self.hiss_to = hiss;
        if sound.kind != Pause {
            self.to = sound.formants;
        }
    }

    /// Produces one sample and advances the chip by one of them.
    pub fn sample(&mut self) -> f32 {
        let dt = 1.0 / self.rate;
        if self.busy {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.busy = false;
                // Nothing is holding the mouth open any more.
                self.level_to = 0.0;
                self.hiss_to = 0.0;
            }
        }

        // Formants and loudness slide rather than jump: a mouth takes about
        // fifteen milliseconds to move, and without that every phoneme
        // boundary is a click.
        let glide = (dt / 0.015).min(1.0);
        for i in 0..3 {
            self.at[i] += (self.to[i] - self.at[i]) * glide;
        }
        self.level += (self.level_to - self.level) * glide;
        self.hiss += (self.hiss_to - self.hiss) * glide;

        if self.muted || self.level < 0.001 {
            return 0.0;
        }

        // The cords, at a pitch the inflection bits nudge and the board's
        // clock drags along with it.
        let pitch =
            PITCH * (1.0 + 0.06 * f32::from(self.inflection)) * (self.clock / NOMINAL_CLOCK);
        self.phase += pitch * dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // The glottis opens for the first part of each cycle and snaps shut.
        // Taking the difference of the flow gives the excitation, and the snap
        // is the edge all the formants ring from; a bare impulse train instead
        // is far too bright and buzzes.
        let flow = if self.phase < OPEN_QUOTIENT {
            0.5 * (1.0 - (std::f32::consts::PI * self.phase / OPEN_QUOTIENT).cos())
        } else {
            0.0
        };
        let pulse = (flow - self.flow) * 8.0;
        self.flow = flow;

        // And the hiss, from a shift register.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        let noise = ((self.noise & 0xFFFF) as f32 / 32768.0 - 1.0) * 0.4;

        // A stop is a burst at the front of the phoneme and quiet behind it.
        let shaped = if self.burst {
            let total = PHONEMES[usize::from(self.phoneme)].ms * 0.001 * NOMINAL_CLOCK / self.clock;
            let gone = (total - self.remaining) / total.max(1e-6);
            (1.0 - gone * 4.0).max(0.0)
        } else {
            1.0
        };

        let excitation = pulse * (1.0 - self.hiss) + noise * self.hiss;
        let mut out = excitation * self.level * shaped;

        let bandwidths = [90.0, 110.0, 180.0];
        for (i, formant) in self.formants.iter_mut().enumerate() {
            formant.tune(self.at[i], bandwidths[i], self.rate);
            out = formant.step(out);
        }
        (out * GAIN).clamp(-1.0, 1.0)
    }
}
