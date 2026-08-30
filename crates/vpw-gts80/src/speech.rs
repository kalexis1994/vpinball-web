//! The Sound & Speech board, the third and largest of Gottlieb's System 80
//! cards.
//!
//! About forty of these games have one: Mars, Volcano, Black Hole, Haunted
//! House, Devil's Dare. It is a whole computer again — a 6502 and a 6532, the
//! same pair as the game board — with two digital-to-analog converters and a
//! [`Votrax`] speech chip on it.
//!
//! The two converters do quite different jobs. One is the loudspeaker, written
//! sample by sample the way the smaller cards write theirs. The other is not
//! audio at all: **it sets the speech chip's clock**, which is how these games
//! do the falling "TILT" — the same word, spoken slower and deeper each time
//! (`gts80s.c:392`).
//!
//! The card's two voices are summed here: the converter at half and the speech
//! chip at full, which is the balance PinMAME mixes them at.
//!
//! # The memory map (`gts80s.c:483`)
//!
//! | From | To | What |
//! |------|-----|------|
//! | `$0000` | `$01FF` | the 6532's RAM, mirrored every 128 bytes |
//! | `$0200` | `$0FFF` | the 6532's registers |
//! | `$1000` | `$1FFF` | write: the loudspeaker's converter |
//! | `$2000` | `$2FFF` | write: a phoneme for the speech chip |
//! | `$3000` | `$3FFF` | write: the speech chip's clock |
//! | `$4000` | `$6FFF` | three expansion boards nobody fitted |
//! | `$7000` | `$7FFF` | the ROM — and a write here mutes the speech |
//!
//! and the whole of it again from `$8000`, because address line 15 does not
//! reach anything. That is what puts the ROM under the processor's vectors at
//! `$FFFA`, and a map that does not fold cannot boot.
//!
//! The write to the ROM window is not a mistake in the firmware: it uses those
//! writes as a delay loop, and the board turns each one into "stop talking".

use vpw_audio::{Dac, DcBlocker, SampleClock};
use vpw_bus::Bus;
use vpw_m6502::Cpu;
use vpw_riot::Riot;

use crate::votrax::Votrax;

/// The processor's clock: 3.579545 MHz over four, the same crystal division
/// the game board uses, and measured on real hardware (`gts80s.c:690`).
pub const CLOCK: u32 = 894_886;

/// Where the two 2 KB images land, and how much room they have.
const ROM_BASE: u16 = 0x7000;
const ROM_SIZE: usize = 0x1000;

/// The card's own dip switches, as the operator left them.
///
/// Six of them reach port B and are read *inverted* (`gts80s.c:357`), so all
/// off — which is where VPinMAME leaves them — reads back as all ones.
const DIPS: u8 = 0x00;

/// How loud the converter sits under the voice.
///
/// PinMAME gives the card's audio stream a level of 50 out of 100 and the
/// speech chip's 100 (`gts80s.c:657` and `:672`), so the effects and music sit
/// at half the voice. That is the balance a table author has been hearing.
const DAC_LEVEL: f32 = 0.5;

/// The corner of the network on the converter's output, in Hz.
///
/// Gottlieb's own is an RC pair across the converter; PinMAME's commented-out
/// one names 270 kΩ and 15 kΩ (`gts80s.c:658`). This is the same idea and the
/// same job: without something here what comes out is the staircase.
const RECONSTRUCTION_HZ: f32 = 6_000.0;

/// Everything the sound processor can reach.
pub struct Speech {
    pub riot: Riot,
    rom: Vec<u8>,

    /// The loudspeaker's converter.
    pub dac: Dac,
    /// The speech chip.
    pub votrax: Votrax,

    /// Whether the card's own test button is being held.
    diag: bool,
    /// Whether the speech chip's busy line was high last time anybody looked,
    /// for spotting the edge that interrupts the processor.
    was_busy: bool,
}

impl Speech {
    fn new(rom: Vec<u8>, rate: u32) -> Self {
        Self {
            riot: Riot::new(),
            rom,
            dac: Dac::new(),
            votrax: Votrax::new(rate),
            diag: false,
            was_busy: false,
        }
    }

    /// What port B reads: the speech chip's busy line, the test button, and
    /// the dips (`gts80s.c:355`).
    fn port_b_input(&self) -> u8 {
        let busy = u8::from(self.votrax.busy()) << 7;
        let test = if self.diag { 0x00 } else { 0x40 };
        busy | test | (DIPS ^ 0x3F)
    }
}

impl Bus for Speech {
    fn read(&mut self, addr: u16) -> u8 {
        // Address line 15 reaches nothing, so the card's whole map answers
        // twice — which is what puts the ROM under the vectors at `$FFFA`.
        let a = addr & 0x7FFF;
        match a {
            0x0000..=0x01FF => self.riot.ram[usize::from(a) % 128],
            0x0200..=0x0FFF => {
                self.riot.in_b = self.port_b_input();
                self.riot.read(a)
            }
            ROM_BASE..=0x7FFF => self
                .rom
                .get(usize::from(a - ROM_BASE))
                .copied()
                .unwrap_or(0xFF),
            // Three expansion boards the card has connectors for and almost no
            // machine ever had. An empty connector reads as an open bus.
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        let a = addr & 0x7FFF;
        match a {
            0x0000..=0x01FF => self.riot.ram[usize::from(a) % 128] = value,
            0x0200..=0x0FFF => self.riot.write(a, value),
            0x1000..=0x1FFF => self.dac.write(value),
            // The phoneme bits reach the chip inverted (`gts80s.c:450`), and a
            // phoneme is also what takes the mute off.
            0x2000..=0x2FFF => {
                self.votrax.set_muted(false);
                self.votrax.write(value ^ 0x3F);
            }
            // The converter drives the chip's clock straight: `$7E` is the
            // middle of the range and gives the 720 kHz of ordinary speech,
            // and `$46` the 398 kHz of a finished TILT. Both points were
            // measured on real hardware with a frequency meter
            // (`gts80s.c:392`).
            0x3000..=0x3FFF => self
                .votrax
                .set_clock((u32::from(value) * 5750).saturating_sub(4500)),
            // A write into the ROM window: the firmware's delay loop, and the
            // card's off switch for the voice.
            ROM_BASE..=0x7FFF => self.votrax.set_muted(true),
            _ => {}
        }
    }
}

/// The card, and the processor on it.
pub struct SpeechBoard {
    pub cpu: Cpu,
    pub mem: Speech,

    clock: SampleClock,
    dc: DcBlocker,
    smoothed: f32,
    smoothing: f32,

    audio: Vec<f32>,
    rate: u32,
    produced: u64,
    owed: f64,
}

impl SpeechBoard {
    /// Builds a card from the two 2 KB images a set carries.
    pub fn new(first: &[u8], second: &[u8], rate: u32) -> Self {
        let mut rom = vec![0xFF; ROM_SIZE];
        for (image, at) in [(first, 0usize), (second, 0x800)] {
            let n = image.len().min(ROM_SIZE - at);
            rom[at..at + n].copy_from_slice(&image[..n]);
        }

        let mut mem = Speech::new(rom, rate);
        let mut cpu = Cpu::new();
        cpu.reset(&mut mem);
        let mut board = Self {
            cpu,
            mem,
            clock: SampleClock::new(CLOCK, rate),
            dc: DcBlocker::new(rate),
            smoothed: 0.0,
            smoothing: 0.0,
            audio: Vec::new(),
            rate,
            produced: 0,
            owed: 0.0,
        };
        board.set_rate(rate);
        board
    }

    /// Points the output at a different rate, throwing away what was made at
    /// the old one — the filters are built from the rate they run at.
    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate;
        self.clock = SampleClock::new(CLOCK, rate);
        self.dc = DcBlocker::new(rate);
        self.mem.votrax.set_host_rate(rate);
        let k = 1.0 - (-std::f32::consts::TAU * RECONSTRUCTION_HZ / rate as f32).exp();
        self.smoothing = k.clamp(0.0, 1.0);
        self.smoothed = 0.0;
        self.audio.clear();
    }

    /// Hands the card a command from the game board.
    ///
    /// Six bits of it reach port A, and the card sets bit 7 itself when the
    /// low nibble is not zero — which is how the firmware tells "say nothing"
    /// from "say sound zero" (`gts80s.c:646`).
    pub fn command(&mut self, value: u8) {
        let data = value & 0x3F;
        self.mem.riot.in_a = data | if data & 0x0F != 0 { 0x80 } else { 0x00 };
    }

    /// The card's own test button, which port B reads.
    pub fn set_diagnostic(&mut self, held: bool) {
        self.mem.diag = held;
    }

    pub fn produced(&self) -> u64 {
        self.produced
    }

    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        if rate != self.rate {
            self.set_rate(rate);
        }
        std::mem::take(&mut self.audio)
    }

    /// Runs one instruction and samples both converters for as long as it took.
    pub fn step(&mut self) -> u32 {
        // The 6532's timer holds the line down until it is read, so this one is
        // a level and not a pulse.
        self.cpu.irq = self.mem.riot.irq();

        let cycles = self.cpu.step(&mut self.mem);
        self.mem.riot.tick(cycles);

        for _ in 0..self.clock.advance(cycles) {
            let raw = self.mem.dac.sample();
            self.smoothed += self.smoothing * (raw - self.smoothed);
            let voice = self.mem.votrax.sample();
            self.audio
                .push(self.dc.process(self.smoothed * DAC_LEVEL + voice));
            self.produced += 1;
        }

        // The speech chip's busy line is the processor's interrupt, and it is
        // the *falling* edge that matters: the chip has finished a phoneme and
        // wants the next one (`gts80s.c:340`).
        let busy = self.mem.votrax.busy();
        if self.mem.was_busy && !busy {
            self.cpu.nmi = true;
        }
        self.mem.was_busy = busy;

        cycles
    }

    /// Runs for a number of the card's own clocks.
    pub fn run(&mut self, clocks: f64) {
        self.owed += clocks;
        while self.owed > 0.0 {
            self.owed -= f64::from(self.step());
        }
    }
}
