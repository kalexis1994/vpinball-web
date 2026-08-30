//! The System 80B sound cards: **two** more 6502s, and a different set of
//! chips in each of three generations.
//!
//! The older cards had one processor writing a resistor ladder by hand. This
//! one splits the job in two and gives each half a processor of its own:
//!
//! - the **Y-CPU** makes the music, on two [`Ay8910`]s or one [`Ym2151`];
//! - the **D-CPU** does nothing but feed a converter, which is where the
//!   drums and the crashes come from.
//!
//! They share a latch. A command from the game board goes into it and
//! interrupts both, and each decides on its own what to do about it. That is
//! why these games can hold a tune under a sound effect where a Black Hole
//! has to choose.
//!
//! # The three generations
//!
//! | | Y-CPU | Chips | Games |
//! |---|-------|-------|-------|
//! | 1 | 1 MHz | two AY-3-8913 and an [`Sp0250`] | Rock, Raven, Genesis, Gold Wings |
//! | 2 | 2 MHz | two AY-3-8913 | Victory, Diamond Lady, TX-Sector |
//! | 3 | 2 MHz | one YM2151 | Bad Girls, Hot Shots, Bone Busters |
//!
//! The first two are told apart by their ROMs — 8 KB images against 32 KB —
//! and the last two are not: they carry the same shaped ROMs and differ only
//! in which chip is soldered on. See [`crate::games`], which keeps the short
//! list of the games that have the FM chip.
//!
//! # The programmable interrupt
//!
//! Neither processor runs on a fixed timer. The Y-CPU has a **rate register**
//! that sets its own interrupt: two nibbles, each dividing a 976 Hz clock, so
//! it can be anything from four times a second to a thousand. That is the
//! clock the music is sequenced on, and a game changes it mid-tune.
//!
//! # About where this comes from
//!
//! The wiring is Gottlieb's, read off `gts80s.c`, which is BSD-3-Clause. The
//! AY was written from its data sheet because PinMAME's is old-MAME; the
//! speech chip is a port, and the FM chip was already here.

use vpw_audio::{Ay8910, DcBlocker, SampleClock, Sp0250, Ym2151};
use vpw_bus::Bus;
use vpw_m6502::{Cpu, flags};

/// Which of the three cards this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    /// 1985: two AY-3-8913 and an SP0250, and the processors at 1 MHz.
    One,
    /// 1986: the speech chip gone, the processors at 2 MHz.
    Two,
    /// 1988: a YM2151 where the AYs were.
    Three,
}

impl Generation {
    /// Both processors run at the same clock, and it doubled with the second
    /// card (`gts80s.c:1356` and `:1373`).
    pub fn clock(self) -> u32 {
        match self {
            Generation::One => 1_000_000,
            _ => 2_000_000,
        }
    }
}

/// How much of a second the interrupt rate register divides.
///
/// A quarter of a megahertz over two hundred and fifty-six, which the two
/// nibbles then divide again (`gts80s.c:770`).
const RATE_CLOCK: f64 = (250_000 >> 8) as f64;

/// The clocks the two music chips run at.
const AY_CLOCK: u32 = 2_000_000;
const YM_CLOCK: u32 = 4_000_000;

/// How the four sources are balanced, out of a hundred, as PinMAME mixes them
/// (`gts80s.c:1088`-`:1110`).
const AY_LEVEL: f32 = 0.25;
const DAC_LEVEL: f32 = 0.50;
const SPEECH_LEVEL: f32 = 0.50;
const FM_LEVEL: f32 = 0.75;

/// Everything on the card that is not one of the two processors.
pub struct Card {
    generation: Generation,

    /// Sixty-four kilobytes of address space each, which is how the images are
    /// laid out: a game's ROM lands at the top of it.
    rom_y: Vec<u8>,
    rom_d: Vec<u8>,
    ram_y: [u8; 0x800],
    ram_d: [u8; 0x800],

    /// The command from the game board, and whether the first one has gone by.
    latch: u8,
    heard_one: bool,
    /// The interrupt lines, which are held until each processor takes them.
    y_irq: bool,
    d_irq: bool,
    /// A non-maskable interrupt the Y-CPU has asked for on the D-CPU's behalf.
    d_nmi: bool,

    /// The Y-CPU's own interrupt rate, and whether it is switched on.
    nmi_rate: u8,
    nmi_enable: bool,
    /// Y-CPU clocks until the next one.
    to_nmi: f64,

    /// The two music chips, the byte on their bus, and which register each has
    /// selected.
    pub ay: [Ay8910; 2],
    ay_latch: u8,
    ay_reg: [u8; 2],
    /// The last value written to the control register, for the edges.
    control: u8,

    /// The speech chip, on a first-generation card.
    pub speech: Option<Sp0250>,
    speech_latch: u8,

    /// The FM chip, on a third-generation one.
    pub fm: Option<Ym2151>,
    fm_port: u8,

    /// The D-CPU's converter: a level and a volume, multiplied.
    dac_data: u8,
    dac_volume: u8,

    /// The card's own dip switches, which only the test one is read for.
    dips: u8,
}

impl Card {
    fn new(generation: Generation, rom_y: Vec<u8>, rom_d: Vec<u8>, rate: u32) -> Self {
        Self {
            generation,
            rom_y,
            rom_d,
            ram_y: [0; 0x800],
            ram_d: [0; 0x800],
            latch: 0,
            heard_one: false,
            y_irq: false,
            d_irq: false,
            d_nmi: false,
            nmi_rate: 0,
            nmi_enable: false,
            to_nmi: 0.0,
            ay: [Ay8910::new(AY_CLOCK, rate), Ay8910::new(AY_CLOCK, rate)],
            ay_latch: 0,
            ay_reg: [0; 2],
            control: 0,
            speech: (generation == Generation::One).then(|| Sp0250::new(rate)),
            speech_latch: 0,
            fm: (generation == Generation::Three).then(|| Ym2151::new(YM_CLOCK, rate)),
            fm_port: 0,
            dac_data: 0,
            dac_volume: 0,
            dips: 0,
        }
    }

    /// What the Y-CPU reads from its input port.
    ///
    /// Four bits of dips, the card's test switch, and — on a first-generation
    /// card — the speech chip saying it wants another byte (`gts80s.c:868`).
    fn input(&self) -> u8 {
        let mut data = self.dips;
        // The test switch pulls its bit down, and a card whose switch is out
        // reads it high.
        data |= 0x40;
        if self.speech.as_ref().is_some_and(Sp0250::wants_data) {
            data |= 0x80;
        }
        data
    }

    /// The Y-CPU's sound control register.
    ///
    /// Bit 0 switches its own interrupt on. On the first two cards the rest is
    /// the bus to the music chips and the speech chip: bit 2 is the AYs'
    /// direction pin, bit 3 picks which of the two, bit 4 is their other bus
    /// pin, and bit 6 tells the speech chip a byte is waiting. On the third it
    /// is bit 7, which picks the FM chip's register or its data.
    fn control(&mut self, data: u8) {
        self.nmi_enable = data & 0x01 != 0;
        if self.generation == Generation::Three {
            self.fm_port = u8::from(data & 0x80 != 0);
            self.control = data;
            return;
        }

        // A write happens on the *falling* edge of the direction pin, which is
        // the chip latching what is on its bus.
        if self.control & 0x04 != 0 && data & 0x04 == 0 {
            let chip = usize::from(data & 0x08 == 0);
            if data & 0x10 != 0 {
                self.ay_reg[chip] = self.ay_latch & 0x0F;
            } else {
                let reg = self.ay_reg[chip];
                self.ay[chip].write(reg, self.ay_latch);
            }
        }

        // And the speech chip takes its byte on the falling edge of its own.
        if self.control & 0x40 != 0 && data & 0x40 == 0 {
            let byte = self.speech_latch;
            if let Some(speech) = &mut self.speech {
                speech.write(byte);
            }
        }

        self.control = data;
    }

    /// One sample of everything the card is making, at the host's rate.
    pub fn sample(&mut self) -> f32 {
        let music = if let Some(fm) = &mut self.fm {
            fm.next_sample_mono() * FM_LEVEL
        } else {
            (self.ay[0].sample() + self.ay[1].sample()) * AY_LEVEL
        };
        let speech = match &mut self.speech {
            Some(chip) => chip.sample() * SPEECH_LEVEL,
            None => 0.0,
        };
        // The converter is a level multiplied by a volume, both eight bits,
        // and what comes out is unsigned — the card's own coupling takes the
        // offset off, and so does the one downstream.
        let dac = f32::from(self.dac_data) * f32::from(self.dac_volume) / 65025.0;
        music + speech + dac * DAC_LEVEL
    }
}

/// The Y-CPU's view of the card.
struct YBus<'a>(&'a mut Card);

impl Bus for YBus<'_> {
    fn read(&mut self, addr: u16) -> u8 {
        let card = &mut *self.0;
        if addr < 0x0800 {
            return card.ram_y[usize::from(addr)];
        }
        match (card.generation, addr) {
            (Generation::One, 0x6000) => card.input(),
            (Generation::One, 0xA800) => card.latch,
            (Generation::One, 0xB000) => {
                card.d_nmi = true;
                0
            }
            (Generation::Two, 0x4000) => card.input(),
            (Generation::Two | Generation::Three, 0x6800) => card.latch,
            (Generation::Two | Generation::Three, 0x7000) => {
                card.d_nmi = true;
                0
            }
            _ => byte(&card.rom_y, addr),
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        let card = &mut *self.0;
        if addr < 0x0800 {
            card.ram_y[usize::from(addr)] = value;
            return;
        }
        match (card.generation, addr) {
            (Generation::One, 0x2000) => card.speech_latch = value,
            (Generation::One, 0x4000) => card.control(value),
            (Generation::One, 0x8000) => card.ay_latch = value,
            (Generation::One, 0xA000) => card.nmi_rate = value,
            (Generation::One, 0xB000) => card.d_nmi = true,

            (Generation::Two, 0x6000) => card.nmi_rate = value,
            (Generation::Two, 0x8000) => card.ay_latch = value,
            (Generation::Two, 0xA000) => card.control(value),

            (Generation::Three, 0x4000) => {
                let port = card.fm_port;
                if let Some(fm) = &mut card.fm {
                    if port == 0 {
                        fm.write_address(value);
                    } else {
                        fm.write_data(value);
                    }
                }
            }
            (Generation::Three, 0x6000) => card.nmi_rate = value,
            (Generation::Three, 0xA000) => card.control(value),
            _ => {}
        }
    }
}

/// The D-CPU's view, which is a great deal smaller: it reads the latch and it
/// writes a converter.
struct DBus<'a>(&'a mut Card);

impl Bus for DBus<'_> {
    fn read(&mut self, addr: u16) -> u8 {
        let card = &mut *self.0;
        if addr < 0x0800 {
            return card.ram_d[usize::from(addr)];
        }
        let latch_at = match card.generation {
            Generation::One => 0x8000,
            _ => 0x4000,
        };
        if addr == latch_at {
            return card.latch;
        }
        byte(&card.rom_d, addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        let card = &mut *self.0;
        if addr < 0x0800 {
            card.ram_d[usize::from(addr)] = value;
            return;
        }
        let dac_at = match card.generation {
            Generation::One => 0x4000,
            _ => 0x8000,
        };
        if addr == dac_at {
            card.dac_volume = value;
        } else if addr == dac_at + 1 {
            card.dac_data = value;
        }
    }
}

fn byte(rom: &[u8], at: u16) -> u8 {
    rom.get(usize::from(at)).copied().unwrap_or(0)
}

/// The card, and the two processors on it.
pub struct SoundCard80B {
    pub y_cpu: Cpu,
    pub d_cpu: Cpu,
    pub card: Card,

    clock: SampleClock,
    dc: DcBlocker,
    audio: Vec<f32>,
    rate: u32,
    produced: u64,
    owed: f64,
}

impl SoundCard80B {
    /// Builds a card from the two processors' ROM images, each already laid
    /// out in its own sixty-four kilobytes.
    pub fn new(generation: Generation, rom_y: Vec<u8>, rom_d: Vec<u8>, rate: u32) -> Self {
        let mut card = Card::new(generation, rom_y, rom_d, rate);
        let mut y_cpu = Cpu::new();
        let mut d_cpu = Cpu::new();
        y_cpu.reset(&mut YBus(&mut card));
        d_cpu.reset(&mut DBus(&mut card));
        let mut board = Self {
            y_cpu,
            d_cpu,
            card,
            clock: SampleClock::new(generation.clock(), rate),
            dc: DcBlocker::new(rate),
            audio: Vec::new(),
            rate,
            produced: 0,
            owed: 0.0,
        };
        board.set_rate(rate);
        board
    }

    pub fn generation(&self) -> Generation {
        self.card.generation
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate;
        self.clock = SampleClock::new(self.card.generation.clock(), rate);
        self.dc = DcBlocker::new(rate);
        for ay in &mut self.card.ay {
            ay.set_rate(rate);
        }
        if let Some(speech) = &mut self.card.speech {
            speech.set_rate(rate);
        }
        if let Some(fm) = &mut self.card.fm {
            fm.set_rates(YM_CLOCK, rate);
        }
        self.audio.clear();
    }

    /// Hands the card a command from the game board.
    ///
    /// Two things happen to it on the way. The very first is dropped, because
    /// it arrives before the card is listening. And the rest are **inverted**
    /// — the game board drives the bus the other way up — after which `$FF`
    /// means nothing was said (`gts80s.c:971`).
    pub fn command(&mut self, value: u8) {
        if !self.card.heard_one {
            self.card.heard_one = true;
            return;
        }
        let data = value ^ 0xFF;
        if data == 0xFF {
            return;
        }
        self.card.latch = data;
        self.card.y_irq = true;
        self.card.d_irq = true;
    }

    /// The card's own test switch.
    pub fn set_diagnostic(&mut self, held: bool) {
        // Bit 0 of the dips is read inverted into the test bit.
        self.card.dips = if held { 0x01 } else { 0x00 };
    }

    pub fn produced(&self) -> u64 {
        self.produced
    }

    /// A byte of either processor's scratch RAM.
    ///
    /// For a host looking at what the firmware is doing — a diagnostic, or a
    /// test — rather than for anything the card needs.
    pub fn ram_y(&self, addr: u16) -> u8 {
        self.card.ram_y[usize::from(addr) & 0x7FF]
    }

    pub fn ram_d(&self, addr: u16) -> u8 {
        self.card.ram_d[usize::from(addr) & 0x7FF]
    }

    /// Writes where one of the processors would, which is how a host can work
    /// a card the firmware has not got to yet.
    pub fn write_y(&mut self, addr: u16, value: u8) {
        YBus(&mut self.card).write(addr, value);
    }

    pub fn write_d(&mut self, addr: u16, value: u8) {
        DBus(&mut self.card).write(addr, value);
    }

    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        if rate != self.rate {
            self.set_rate(rate);
        }
        std::mem::take(&mut self.audio)
    }

    /// How long the Y-CPU's own interrupt is between firings, in its clocks.
    ///
    /// Two nibbles, each dividing a 976 Hz clock, and neither can be zero
    /// because the register is subtracted from sixteen.
    fn nmi_period(&self) -> f64 {
        let rate = self.card.nmi_rate;
        let lo = f64::from(16 - (rate & 0x0F));
        let hi = f64::from(16 - (rate >> 4));
        f64::from(self.card.generation.clock()) * lo * hi / RATE_CLOCK
    }

    /// Runs one instruction on each processor and samples what came out.
    ///
    /// They share a clock, so one instruction each is close enough to
    /// lockstep: what matters is that neither runs a whole tune while the
    /// other is stopped.
    pub fn step(&mut self) -> u32 {
        // Both interrupt lines are held until the processor takes them, which
        // is what the board does: it does not pulse them.
        self.y_cpu.irq = self.card.y_irq;
        let y_takes = self.card.y_irq && self.y_cpu.p & flags::IRQ_DISABLE == 0;
        let cycles = self.y_cpu.step(&mut YBus(&mut self.card));
        if y_takes {
            self.card.y_irq = false;
        }

        self.d_cpu.irq = self.card.d_irq;
        let d_takes = self.card.d_irq && self.d_cpu.p & flags::IRQ_DISABLE == 0;
        if self.card.d_nmi {
            self.card.d_nmi = false;
            self.d_cpu.nmi = true;
        }
        self.d_cpu.step(&mut DBus(&mut self.card));
        if d_takes {
            self.card.d_irq = false;
        }

        // The Y-CPU's own interrupt. The timer runs whether or not it is
        // allowed to fire, because the register only gates the pin.
        self.card.to_nmi -= f64::from(cycles);
        while self.card.to_nmi <= 0.0 {
            self.card.to_nmi += self.nmi_period();
            if self.card.nmi_enable {
                self.y_cpu.nmi = true;
            }
        }

        for _ in 0..self.clock.advance(cycles) {
            let mixed = self.card.sample();
            self.audio.push(self.dc.process(mixed).clamp(-1.0, 1.0));
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
