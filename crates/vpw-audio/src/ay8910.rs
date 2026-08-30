//! The General Instrument AY-3-8910, and the AY-3-8913 that is the same chip
//! in a smaller package.
//!
//! Three square waves, a noise source and one envelope shared between them.
//! It is the sound of a 1980s arcade and of half the home computers of the
//! decade, and Gottlieb put **two** of them on the System 80B sound card —
//! six voices, which is what those games use for their music.
//!
//! # About where this code comes from
//!
//! Written from General Instrument's data sheet. PinMAME's `sound/ay8910.c`
//! carries no BSD licence line, so it is still under the old MAME licence —
//! non-commercial, and incompatible with this project's GPLv3 — and none of it
//! was read for this. What *was* read is Gottlieb's own wiring in `gts80s.c`,
//! which is BSD and is a description of a circuit board besides.
//!
//! # The registers
//!
//! | Register | What |
//! |----------|------|
//! | 0, 1 | channel A tone period: eight fine bits and four coarse |
//! | 2, 3 | channel B |
//! | 4, 5 | channel C |
//! | 6 | noise period, five bits |
//! | 7 | the mixer: three tone enables, three noise enables, and the ports |
//! | 8, 9, 10 | channel A, B and C level — four bits, or bit 4 to follow the envelope |
//! | 11, 12 | envelope period, sixteen bits |
//! | 13 | envelope shape |
//! | 14, 15 | the two I/O ports, which an 8913 does not have |
//!
//! The enables in register 7 are **active low**: a bit set is a voice
//! switched off. That is the one thing about this chip everybody gets wrong
//! once.

/// How many voices, which is also how many tone generators there are.
const CHANNELS: usize = 3;

/// What each of the sixteen amplitude codes is worth.
///
/// The data sheet gives the ladder as **3 dB a step**, which is a factor of
/// the square root of two in voltage, and zero at the bottom. A real chip's
/// bottom few steps wander off that curve; this is the curve it was specified
/// to.
fn ladder(level: u8) -> f32 {
    if level == 0 {
        return 0.0;
    }
    2.0f32.powf((f32::from(level) - 15.0) / 2.0)
}

/// One AY-3-8910.
#[derive(Debug, Clone)]
pub struct Ay8910 {
    regs: [u8; 16],

    /// Where each tone generator's counter has got to, and which way its
    /// square wave is pointing.
    counter: [u16; CHANNELS],
    high: [bool; CHANNELS],

    /// The noise generator: a counter and the shift register it clocks.
    noise_counter: u8,
    noise: u32,

    /// The envelope: its own counter, which of the sixteen steps it is on,
    /// whether it is climbing, and whether it has stopped.
    env_counter: u32,
    env_step: u8,
    env_climbing: bool,
    env_held: bool,

    /// The chip's clock, and the rate a host wants samples at.
    clock: f64,
    rate: f64,
    /// Chip steps owed to the host's next sample, and what has been made
    /// towards it.
    owed: f64,
    sum: f32,
    taken: u32,
    last: f32,
}

impl Ay8910 {
    /// `clock` is the pin, not the internal rate: everything inside runs from
    /// a sixteenth of it.
    pub fn new(clock: u32, rate: u32) -> Self {
        Self {
            regs: [0; 16],
            counter: [0; CHANNELS],
            high: [false; CHANNELS],
            noise_counter: 0,
            // Anything but zero. A shift register of zeroes is a shift
            // register that never comes out of it.
            noise: 1,
            env_counter: 0,
            env_step: 0,
            env_climbing: false,
            env_held: false,
            clock: f64::from(clock),
            rate: f64::from(rate),
            owed: 0.0,
            sum: 0.0,
            taken: 0,
            last: 0.0,
        }
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.rate = f64::from(rate);
        self.owed = 0.0;
    }

    /// What a register holds.
    pub fn read(&self, reg: u8) -> u8 {
        self.regs[usize::from(reg & 0x0F)]
    }

    /// Writes a register.
    pub fn write(&mut self, reg: u8, value: u8) {
        let reg = usize::from(reg & 0x0F);
        // The registers that are not eight bits wide ignore what is above
        // them, and a host that reads one back should see what the chip sees.
        let value = match reg {
            1 | 3 | 5 | 13 => value & 0x0F,
            6 => value & 0x1F,
            8..=10 => value & 0x1F,
            _ => value,
        };
        self.regs[reg] = value;

        // Writing the shape register restarts the envelope, whatever it was
        // doing and even if the value did not change. A game uses that: the
        // same shape written again is how it retriggers a note.
        if reg == 13 {
            self.env_step = 0;
            self.env_held = false;
            self.env_climbing = value & 0x04 != 0;
            self.env_counter = 0;
        }
    }

    /// The period of one tone generator, in internal steps.
    fn tone_period(&self, channel: usize) -> u16 {
        let fine = u16::from(self.regs[channel * 2]);
        let coarse = u16::from(self.regs[channel * 2 + 1]) & 0x0F;
        (coarse << 8 | fine).max(1)
    }

    fn noise_period(&self) -> u8 {
        (self.regs[6] & 0x1F).max(1)
    }

    fn envelope_period(&self) -> u32 {
        let period = u32::from(self.regs[12]) << 8 | u32::from(self.regs[11]);
        period.max(1)
    }

    /// The level the envelope is at, nought to fifteen.
    fn envelope(&self) -> u8 {
        if self.env_climbing {
            self.env_step
        } else {
            15 - self.env_step
        }
    }

    /// One step of the chip's internal clock, a sixteenth of the pin.
    fn step(&mut self) {
        for channel in 0..CHANNELS {
            self.counter[channel] += 1;
            if self.counter[channel] >= self.tone_period(channel) {
                self.counter[channel] = 0;
                self.high[channel] = !self.high[channel];
            }
        }

        self.noise_counter += 1;
        if self.noise_counter >= self.noise_period() {
            self.noise_counter = 0;
            // Seventeen bits, tapped at the first and the fourth.
            let feedback = ((self.noise ^ (self.noise >> 3)) & 1) << 16;
            self.noise = (self.noise >> 1) | feedback;
        }

        // The envelope has a divider of its own on top of the sixteen, so it
        // runs at the pin over two hundred and fifty-six.
        if !self.env_held {
            self.env_counter += 1;
            if self.env_counter >= self.envelope_period() * 16 {
                self.env_counter = 0;
                self.env_advance();
            }
        }
    }

    /// One step of the envelope, and what happens when it runs off the end.
    ///
    /// Four bits decide: `CONT` says whether there is anything after the first
    /// pass, `ATT` which way that pass goes, `ALT` whether each pass turns
    /// round, and `HOLD` whether it stops after one.
    fn env_advance(&mut self) {
        if self.env_step < 15 {
            self.env_step += 1;
            return;
        }

        let shape = self.regs[13];
        let (cont, alt, hold) = (shape & 0x08 != 0, shape & 0x02 != 0, shape & 0x01 != 0);

        if !cont {
            // One pass and then silence, whichever way that pass went.
            self.env_climbing = false;
            self.env_step = 15;
            self.env_held = true;
        } else if hold {
            if alt {
                self.env_climbing = !self.env_climbing;
            }
            self.env_step = 15;
            self.env_held = true;
        } else {
            if alt {
                self.env_climbing = !self.env_climbing;
            }
            self.env_step = 0;
        }
    }

    /// What the three channels add up to at this instant, in `-1.0..1.0`.
    fn level(&self) -> f32 {
        let mixer = self.regs[7];
        let noise = self.noise & 1 != 0;
        let mut total = 0.0;
        for channel in 0..CHANNELS {
            // Active low: a bit set in the mixer is a voice switched *off*,
            // which is the thing about this chip everybody gets wrong once.
            let tone_off = mixer & (1 << channel) != 0;
            let noise_off = mixer & (1 << (channel + 3)) != 0;
            let open = (self.high[channel] || tone_off) && (noise || noise_off);
            if !open {
                continue;
            }
            let amplitude = self.regs[8 + channel];
            let level = if amplitude & 0x10 != 0 {
                self.envelope()
            } else {
                amplitude & 0x0F
            };
            total += ladder(level);
        }
        // Three voices into one pin, and the chip does not add them at unity.
        total / CHANNELS as f32
    }

    /// One sample at the host's rate.
    pub fn sample(&mut self) -> f32 {
        // Everything inside runs at a sixteenth of the pin, and that is still
        // faster than the host wants samples, so what falls between two of
        // them is averaged rather than dropped.
        self.owed += self.clock / 16.0 / self.rate;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            self.step();
            self.last = self.level();
            self.sum += self.last;
            self.taken += 1;
        }
        let out = if self.taken > 0 {
            self.sum / self.taken as f32
        } else {
            self.last
        };
        self.sum = 0.0;
        self.taken = 0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the chip for a stretch and says how loud it was and how often the
    /// output changed direction, which is enough to tell a tone from silence
    /// and a high note from a low one.
    fn listen(chip: &mut Ay8910, samples: usize, rate: u32) -> (f32, f32) {
        let mut peak = 0.0f32;
        let mut crossings = 0;
        let mid = 0.05;
        // Seeded from the first sample, so a level that starts high is not
        // counted as having got there.
        let mut previous = chip.sample();
        peak = peak.max(previous);
        for _ in 1..samples {
            let s = chip.sample();
            peak = peak.max(s);
            if (s > mid) != (previous > mid) {
                crossings += 1;
            }
            previous = s;
        }
        let seconds = samples as f32 / rate as f32;
        (peak, crossings as f32 / 2.0 / seconds)
    }

    #[test]
    fn a_channel_makes_a_square_wave_at_the_period_it_was_given() {
        let mut chip = Ay8910::new(2_000_000, 48_000);
        // A period of 250 at two megahertz over sixteen: 125000 / 250 / 2,
        // which is 250 Hz, because it takes two counts to make a cycle.
        chip.write(0, 250);
        chip.write(1, 0);
        chip.write(7, 0b0011_1110); // channel A tone on, everything else off
        chip.write(8, 15);

        let (peak, hz) = listen(&mut chip, 48_000, 48_000);
        assert!(peak > 0.3, "it should be loud, and peaked at {peak}");
        assert!(
            (hz - 250.0).abs() < 5.0,
            "it should be 250 Hz, and was {hz}"
        );
    }

    /// The enables are active low, and switching *both* of a channel's off
    /// does not silence it: it leaves the amplitude sitting on the pin as a
    /// steady level. That is not a quirk to tidy away — it is how these chips
    /// get used as a crude converter, and a System 80B does exactly that.
    #[test]
    fn a_channel_with_nothing_enabled_holds_a_steady_level() {
        let mut chip = Ay8910::new(2_000_000, 48_000);
        chip.write(0, 250);
        chip.write(8, 15);

        chip.write(7, 0xFF);
        let (peak, hz) = listen(&mut chip, 4_800, 48_000);
        assert!(peak > 0.3, "the amplitude is still on the pin");
        assert_eq!(hz, 0.0, "but nothing is moving it");

        // Turn the amplitude down and it really is silent.
        chip.write(8, 0);
        let (peak, _) = listen(&mut chip, 4_800, 48_000);
        assert_eq!(peak, 0.0);

        // And clearing one bit of the mixer lets one voice out again.
        chip.write(8, 15);
        chip.write(7, 0b0011_1110);
        let (peak, hz) = listen(&mut chip, 4_800, 48_000);
        assert!(peak > 0.3);
        assert!(hz > 100.0, "moving this time");
    }

    #[test]
    fn the_volume_ladder_is_three_decibels_a_step() {
        // Each step is the square root of two, so two steps double it.
        assert_eq!(ladder(0), 0.0);
        assert!((ladder(15) - 1.0).abs() < 1e-6);
        assert!((ladder(13) / ladder(15) - 0.5).abs() < 1e-6);
        assert!((ladder(14) / ladder(15) - 0.5f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn the_envelope_shapes_end_where_the_data_sheet_says() {
        let ending = |shape: u8| {
            let mut chip = Ay8910::new(2_000_000, 48_000);
            chip.write(11, 1);
            chip.write(12, 0);
            chip.write(13, shape);
            // Well past one pass of sixteen steps.
            for _ in 0..2_000 {
                chip.step();
            }
            chip.envelope()
        };

        // Without `CONT` every shape decays to nothing, whichever way it went.
        for shape in 0x00..=0x07 {
            assert_eq!(ending(shape), 0, "shape {shape:#04X} should end silent");
        }
        // `CONT` and `HOLD` together stop at whichever end they reached.
        assert_eq!(ending(0x09), 0, "decay, then hold at the bottom");
        assert_eq!(ending(0x0B), 15, "decay, turn round, hold at the top");
        assert_eq!(ending(0x0D), 15, "attack, then hold at the top");
        assert_eq!(ending(0x0F), 0, "attack, turn round, hold at the bottom");
    }

    #[test]
    fn writing_the_shape_again_restarts_the_envelope() {
        let mut chip = Ay8910::new(2_000_000, 48_000);
        chip.write(11, 1);
        chip.write(12, 0);
        chip.write(13, 0x09); // decay once and hold
        for _ in 0..2_000 {
            chip.step();
        }
        assert_eq!(chip.envelope(), 0);

        // The same value again, which is how a game retriggers a note.
        chip.write(13, 0x09);
        assert_eq!(chip.envelope(), 15, "back at the top");
    }

    #[test]
    fn the_noise_register_is_not_stuck() {
        let mut chip = Ay8910::new(2_000_000, 48_000);
        chip.write(6, 1);
        chip.write(7, 0b0011_0111); // channel A noise only
        chip.write(8, 15);
        let (peak, _) = listen(&mut chip, 4_800, 48_000);
        assert!(peak > 0.2, "noise should be audible, and peaked at {peak}");

        // And it should not repeat inside a seventeen-bit period.
        let mut chip = Ay8910::new(2_000_000, 48_000);
        chip.write(6, 1);
        let first: Vec<u32> = (0..64)
            .map(|_| {
                chip.step();
                chip.noise & 1
            })
            .collect();
        assert!(first.contains(&0) && first.contains(&1));
    }
}
