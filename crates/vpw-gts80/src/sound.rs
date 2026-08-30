//! The System 80 sound board: a second 6502 on a card of its own.
//!
//! Smaller than the game board it hangs off. A 6502, one [`Riot6530`], a
//! resistor ladder, and a socket for a ROM. The game board writes four bits to
//! it and walks away; everything after that — the pitch, the length, the
//! shape of the waveform — is this processor writing bytes to eight resistors
//! in a timed loop. There is no sound chip. **Port A of the RIOT is the
//! loudspeaker.**
//!
//! # The two cards
//!
//! Gottlieb built the same board twice.
//!
//! The **sound card** ([`Card::Sound`]) is the 1980 one: a 1 KB ROM of
//! four-bit samples at `$0400`, and the code that plays them lives in the
//! kilobyte of mask ROM on the 6530 itself — which is why a set for one of
//! these carries `6530sy80.bin` beside the game's own `.snd`. No interrupt
//! line: the processor polls the latch.
//!
//! The **piggyback card** ([`Card::Piggyback`]) replaced it around 1983 with
//! one 2 KB ROM holding both the code and the samples, and wired the
//! interrupt: a command from the game board now interrupts this one instead of
//! waiting to be noticed. Ice Fever, El Dorado and their neighbours are these.
//!
//! # Where the clock comes from, which is nowhere in particular
//!
//! Neither card has a crystal. The 6502 is clocked by a resistor and a
//! capacitor, so no two boards agree and no board agrees with itself warm;
//! PinMAME's own note on this runs to twenty lines and ends by saying real
//! ones have been measured from 675 kHz to 1 MHz (`gts80s.c:270-290`). The
//! numbers here are the ones PinMAME settled on, because that is the pitch
//! everybody who has heard these games in software has heard.

use vpw_audio::{Dac, DcBlocker, SampleClock};
use vpw_bus::Bus;
use vpw_m6502::Cpu;
use vpw_riot::Riot6530;

/// Which of the two cards a set carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    /// 1980's: a 1 KB four-bit sample ROM and the 6530's own ROM beside it.
    Sound,
    /// 1983's piggyback: one 2 KB ROM, and the interrupt line connected.
    Piggyback,
}

impl Card {
    /// The processor's clock (`gts80s.c:296` and `:307`).
    pub fn clock(self) -> u32 {
        match self {
            Card::Sound => 904_977,
            Card::Piggyback => 961_538,
        }
    }
}

/// What the dip switches on the card say when nobody has touched them.
///
/// Two of them reach port B, and both are read *inverted* — off means the bit
/// is set (`gts80s.c:173`). Off is where they leave the factory and where
/// VPinMAME leaves them, so this is what a table author has been listening to.
const DIPS: u8 = 0x90;

/// Bit 5 of port B is tied high on the card, so the latch never reads as a
/// bare command.
const TIED_HIGH: u8 = 0x20;

/// How loud this card sits in the mix.
///
/// The ladder swings the full range and the card's own amplifier does not run
/// it at unity into the cabinet: PinMAME gives the stream a mixing level of 50
/// out of 100 (`gts80s.c:255`). Without it the DAC's square edges clip against
/// everything else the player is mixing.
const LEVEL: f32 = 0.5;

/// The cutoff of the reconstruction filter, in Hz.
///
/// The ladder's output is a staircase and the card has an RC network on it to
/// take the corners off. PinMAME stands a FIR in for that at 0.15 of a 22 kHz
/// stream (`gts80s.c:184`), which is this, and without something in that place
/// what comes out is the staircase and all its images.
const RECONSTRUCTION_HZ: f32 = 3_300.0;

/// The card's memory: everything the sound processor can reach.
pub struct Sound {
    pub riot: Riot6530,
    card: Card,

    /// The game's own ROM: 2 KB on a piggyback, 1 KB of four-bit samples on a
    /// sound card.
    rom: Vec<u8>,
    /// The 6530's mask ROM, which only a sound card has and which holds the
    /// code that card runs.
    system: Vec<u8>,

    /// The resistor ladder, which is the whole audio path.
    pub dac: Dac,

    /// The last command the game board sent, four bits of it.
    command: u8,
    /// Whether the card has armed its interrupt. Port B bit 6, and only the
    /// piggyback has the wire (`gts80s.c:92`).
    irq_armed: bool,
    /// An interrupt the game board has asked for and the processor has not yet
    /// been offered.
    irq_pending: bool,
}

impl Sound {
    fn new(card: Card, rom: Vec<u8>, system: Vec<u8>) -> Self {
        let mut riot = Riot6530::new();
        riot.in_b = DIPS | TIED_HIGH;
        Self {
            riot,
            card,
            rom,
            system,
            dac: Dac::new(),
            command: 0,
            irq_armed: false,
            irq_pending: false,
        }
    }

    /// The game board has written the latch.
    ///
    /// Four bits, and the rest of the port is the dips and a pin tied high.
    /// On a piggyback a command that is not zero also interrupts, but only
    /// while the card's own firmware has said it may.
    fn command(&mut self, value: u8) {
        self.command = value & 0x0F;
        self.riot.in_b = DIPS | TIED_HIGH | self.command;
        if self.card == Card::Piggyback && self.command != 0 && self.irq_armed {
            self.irq_pending = true;
        }
    }

    /// Where an address lands. `None` is a hole, which reads as an open bus.
    fn rom_byte(&self, addr: u16) -> Option<u8> {
        let at = |image: &[u8], offset: u16| image.get(usize::from(offset)).copied();
        match self.card {
            // One 2 KB image, at `$0800` and again at `$F800` so the vectors
            // at `$FFFA` land in the top of it (`gts80s.h:30`).
            Card::Piggyback => match addr {
                0x0800..=0x0FFF => at(&self.rom, addr - 0x0800),
                0xF800..=0xFFFF => at(&self.rom, addr - 0xF800),
                _ => None,
            },
            // The game's kilobyte at `$0400`, mirrored at `$0800`, and the
            // 6530's own above it and again at `$FC00` for the vectors
            // (`gts80s.h:21`).
            Card::Sound => match addr {
                0x0400..=0x07FF => at(&self.rom, addr - 0x0400),
                0x0800..=0x0BFF => at(&self.rom, addr - 0x0800),
                0x0C00..=0x0FFF => at(&self.system, addr - 0x0C00),
                0xFC00..=0xFFFF => at(&self.system, addr - 0xFC00),
                _ => None,
            },
        }
    }
}

impl Bus for Sound {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // Sixty-four bytes of RAM, and the card decodes so little of the
            // address that they answer everywhere below the registers — and
            // again at `$1000`, which is where the firmware keeps its stack.
            0x0000..=0x01FF | 0x1000..=0x10FF => {
                self.riot.ram[usize::from(addr) % vpw_riot::r6530::RAM]
            }
            0x0200..=0x03FF => self.riot.read(addr),
            _ => self.rom_byte(addr).unwrap_or(0xFF),
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x01FF | 0x1000..=0x10FF => {
                self.riot.ram[usize::from(addr) % vpw_riot::r6530::RAM] = value;
            }
            0x0200..=0x03FF => {
                self.riot.write(addr, value);
                match addr & 0x0F {
                    // Port A is the ladder. Whatever the chip drives on the
                    // pins it has been told are outputs is the level.
                    0 | 1 => self.dac.write(self.riot.port_a_output()),
                    // And port B's bit 6 is the interrupt's own arming, on the
                    // card that has the wire.
                    2 | 3 => self.irq_armed = self.riot.port_b_output() & 0x40 != 0,
                    _ => {}
                }
            }
            // Writing to a ROM is a bug in the ROM or in us; either way it is
            // not a crash.
            _ => {}
        }
    }
}

/// The card, and the processor on it.
pub struct SoundBoard {
    pub cpu: Cpu,
    pub mem: Sound,

    clock: SampleClock,
    dc: DcBlocker,
    /// A one-pole low-pass standing in for the card's own reconstruction
    /// network. See [`RECONSTRUCTION_HZ`].
    smoothed: f32,
    smoothing: f32,

    audio: Vec<f32>,
    rate: u32,
    produced: u64,

    /// Sound-board clocks owed, because the two processors do not agree on
    /// what a second is and the remainder has to be carried.
    owed: f64,
}

impl SoundBoard {
    /// Builds a piggyback card around its single 2 KB ROM.
    pub fn piggyback(rom: Vec<u8>, rate: u32) -> Self {
        Self::new(Card::Piggyback, rom, Vec::new(), rate)
    }

    /// Builds a sound card around the game's samples and the 6530's own ROM.
    pub fn sound(rom: Vec<u8>, system: Vec<u8>, rate: u32) -> Self {
        Self::new(Card::Sound, rom, system, rate)
    }

    fn new(card: Card, rom: Vec<u8>, system: Vec<u8>, rate: u32) -> Self {
        let mut mem = Sound::new(card, rom, system);
        let mut cpu = Cpu::new();
        cpu.reset(&mut mem);
        let mut board = Self {
            cpu,
            mem,
            clock: SampleClock::new(card.clock(), rate),
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

    /// Which card this is.
    pub fn card(&self) -> Card {
        self.mem.card
    }

    /// Points the output at a different rate, throwing away what was made at
    /// the old one.
    ///
    /// The filters are built from the rate they run at, so one made for
    /// another rate is not a slightly wrong filter but a different one.
    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate;
        self.clock = SampleClock::new(self.mem.card.clock(), rate);
        self.dc = DcBlocker::new(rate);
        // One pole: `y += k (x - y)`, with `k` set so the corner lands where
        // the card's own network puts it.
        let k = 1.0 - (-std::f32::consts::TAU * RECONSTRUCTION_HZ / rate as f32).exp();
        self.smoothing = k.clamp(0.0, 1.0);
        self.smoothed = 0.0;
        self.audio.clear();
    }

    /// Hands the card a command from the game board.
    pub fn command(&mut self, value: u8) {
        self.mem.command(value);
    }

    /// The sound-board test button, which is wired to the processor's NMI.
    pub fn diagnostic(&mut self) {
        self.cpu.nmi = true;
    }

    /// How many samples the card has made since it was built.
    ///
    /// A card making nothing is firmware that has not got going; a card making
    /// forty-eight thousand a second is one doing its job with nothing to say.
    pub fn produced(&self) -> u64 {
        self.produced
    }

    /// Takes the audio away and leaves the buffer empty.
    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        if rate != self.rate {
            self.set_rate(rate);
        }
        std::mem::take(&mut self.audio)
    }

    /// Runs one instruction and samples the ladder for as long as it took.
    pub fn step(&mut self) -> u32 {
        // The interrupt is a pulse and not a level: the game board's write
        // reaches the processor for one instruction, and if the firmware has
        // interrupts masked at that moment then it missed it, which is what
        // the wire does too.
        self.cpu.irq = self.mem.irq_pending;
        self.mem.irq_pending = false;

        let cycles = self.cpu.step(&mut self.mem);
        self.mem.riot.tick(cycles);
        if self.mem.riot.irq() {
            self.mem.irq_pending = true;
        }

        for _ in 0..self.clock.advance(cycles) {
            let raw = self.mem.dac.sample();
            self.smoothed += self.smoothing * (raw - self.smoothed);
            self.audio.push(self.dc.process(self.smoothed) * LEVEL);
            self.produced += 1;
        }
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
