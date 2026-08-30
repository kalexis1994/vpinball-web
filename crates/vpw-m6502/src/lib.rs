//! MOS Technology 6502 emulator.
//!
//! Gottlieb's System 80 — the hardware behind Ice Fever, Black Hole, Haunted
//! House and about fifty more — runs a 6502 at 3.579545 MHz divided by four,
//! which is 894 kHz (`gts80.c:708` in PinMAME). Its sound boards run one too.
//!
//! # About where this code comes from
//!
//! This is **not** a port of the MAME core. `src/cpu/m6502/` in PinMAME is
//! still under the old MAME licence, which is non-commercial and therefore
//! incompatible with this project's GPLv3. The 6502's instruction set, its
//! addressing modes, its flag behaviour and its timings are public MOS
//! specification — the chip is fifty years old and its datasheet has been in
//! print the whole time — so this implementation was written from there.
//!
//! # What is and is not here
//!
//! The 151 documented opcodes, all thirteen addressing modes, the three
//! interrupts and `BRK`. The undocumented opcodes are **not** implemented: a
//! game ROM written by Gottlieb's engineers against Rockwell's own assembler
//! has no reason to contain one, and executing a guess is worse than saying so
//! — an unknown opcode is recorded in [`Cpu::last_illegal`] and treated as a
//! two-cycle `NOP`.
//!
//! Decimal mode **is** here, because pinball firmware is full of it: a score
//! is a string of BCD digits and every machine of this era adds to it with
//! `SED`/`ADC`/`CLD`.
//!
//! # How far it is verified
//!
//! Sixteen tests, each naming a fact about the chip rather than a function of
//! ours, and weighted towards the facts that are easy to get wrong in exactly
//! one direction: the zero page wrapping, the `JMP` bug, what `RTS` adds back,
//! which flag `BRK` pushes, when an indexed access pays for a page and when it
//! does not.
//!
//! What they cannot do is prove the other hundred and forty opcodes right, and
//! there is no point pretending otherwise: the test that settles this core is
//! a System 80 ROM booting on it, which is the next thing to build.
//!
//! # Two things that are easy to get wrong
//!
//! The stack lives at `$0100 + S` and wraps within that page: pushing with
//! `S = 0` writes `$0100` and leaves `S = $FF`. And the `JMP ($xxFF)` bug is
//! real hardware and is emulated — the high byte comes from `$xx00`, not from
//! the next page — because a ROM assembled in 1985 was tested against a chip
//! that did that.
//!
//! # Usage
//!
//! ```
//! use vpw_m6502::{Bus, Cpu, FlatMemory, vectors};
//!
//! let mut mem = FlatMemory::new();
//! mem.load(0x0100, &[0xA9, 0x2A]); // LDA #$2A
//! mem.bytes[vectors::RESET as usize] = 0x00;
//! mem.bytes[vectors::RESET as usize + 1] = 0x01;
//!
//! let mut cpu = Cpu::new();
//! cpu.reset(&mut mem);
//! let cycles = cpu.step(&mut mem);
//!
//! assert_eq!(cpu.a, 0x2A);
//! assert_eq!(cycles, 2);
//! ```

mod exec;

pub use vpw_bus::{Bus, FlatMemory};

/// Interrupt and reset vectors, at the top of the map.
pub mod vectors {
    /// Non-maskable interrupt.
    pub const NMI: u16 = 0xFFFA;
    /// Reset.
    pub const RESET: u16 = 0xFFFC;
    /// Maskable interrupt request, and where `BRK` goes.
    pub const IRQ: u16 = 0xFFFE;
}

/// The status register's bits.
pub mod flags {
    pub const CARRY: u8 = 0x01;
    pub const ZERO: u8 = 0x02;
    /// Interrupts disabled.
    pub const IRQ_DISABLE: u8 = 0x04;
    pub const DECIMAL: u8 = 0x08;
    /// Set in the copy `BRK` pushes, clear in the one an interrupt pushes.
    /// It is not a real bit of the register — nothing can test it — but what
    /// is on the stack is how a handler tells the two apart.
    pub const BREAK: u8 = 0x10;
    /// Bit 5 has no meaning and reads as one.
    pub const UNUSED: u8 = 0x20;
    pub const OVERFLOW: u8 = 0x40;
    pub const NEGATIVE: u8 = 0x80;
}

/// CPU state.
#[derive(Debug, Clone)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    /// Stack pointer. The stack is at `$0100 + s`, growing downwards.
    pub s: u8,
    pub pc: u16,
    /// Status register. Bit 5 always reads as one.
    pub p: u8,

    /// Whether an IRQ line is being held low. Level-triggered: as long as this
    /// is set and the disable flag is clear, the CPU keeps taking interrupts.
    pub irq: bool,
    /// Whether an NMI is pending. Edge-triggered: set it and the CPU takes one
    /// interrupt, whatever the disable flag says.
    pub nmi: bool,
    /// Whether the CPU is stopped at a `WAI`-like halt. The 6502 has no such
    /// instruction; this is set when an unknown opcode is executed so a caller
    /// can notice rather than watch the machine wander.
    pub last_illegal: Option<(u16, u8)>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFD,
            pc: 0,
            p: flags::IRQ_DISABLE | flags::UNUSED,
            irq: false,
            nmi: false,
            last_illegal: None,
        }
    }

    /// Pulls reset: interrupts off, and the PC from `$FFFC`.
    ///
    /// The stack pointer is left at `$FD`. Real hardware does not load it —
    /// reset runs the same microcode as an interrupt and decrements it three
    /// times without writing — so from a power-on `$00` it ends at `$FD`, and
    /// that is the value every ROM of this era is written against.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.s = 0xFD;
        self.p = flags::IRQ_DISABLE | flags::UNUSED;
        self.pc = read_u16(bus, vectors::RESET);
        self.irq = false;
        self.nmi = false;
        self.last_illegal = None;
    }

    /// Runs one instruction, or takes a pending interrupt. Returns the cycles.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        // An NMI cannot be masked and is taken first. Edge-triggered: the
        // request is consumed here, so a line held low does not stack up.
        if self.nmi {
            self.nmi = false;
            return self.interrupt(bus, vectors::NMI);
        }
        if self.irq && self.p & flags::IRQ_DISABLE == 0 {
            return self.interrupt(bus, vectors::IRQ);
        }
        exec::step(self, bus)
    }

    /// Pushes the state and jumps to a vector. Seven cycles, as on the chip.
    fn interrupt(&mut self, bus: &mut impl Bus, vector: u16) -> u32 {
        let pc = self.pc;
        self.push(bus, (pc >> 8) as u8);
        self.push(bus, pc as u8);
        // Without `BREAK`: that bit is how the handler tells a real interrupt
        // from a `BRK`, and it is the only difference between the two.
        self.push(bus, (self.p | flags::UNUSED) & !flags::BREAK);
        self.p |= flags::IRQ_DISABLE;
        self.pc = read_u16(bus, vector);
        7
    }

    fn push(&mut self, bus: &mut impl Bus, value: u8) {
        bus.write(0x0100 | u16::from(self.s), value);
        self.s = self.s.wrapping_sub(1);
    }

    fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.s = self.s.wrapping_add(1);
        bus.read(0x0100 | u16::from(self.s))
    }

    fn set_zn(&mut self, value: u8) {
        self.set_flag(flags::ZERO, value == 0);
        self.set_flag(flags::NEGATIVE, value & 0x80 != 0);
    }

    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.p |= flag;
        } else {
            self.p &= !flag;
        }
    }

    fn flag(&self, flag: u8) -> bool {
        self.p & flag != 0
    }
}

/// A 16-bit read, **little-endian**, which is the 6502's order.
///
/// Not [`Bus::read_u16`]: that one is big-endian because it was written for
/// the 68xx family, and the two conventions are exactly the trap this note
/// exists to keep somebody out of.
pub(crate) fn read_u16(bus: &mut impl Bus, addr: u16) -> u16 {
    let lo = bus.read(addr);
    let hi = bus.read(addr.wrapping_add(1));
    u16::from_le_bytes([lo, hi])
}
