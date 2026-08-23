//! Motorola MC6800 / MC6808 emulator.
//!
//! Williams System 11 —the hardware behind F-14 Tomcat— uses an M6808 at 1 MHz
//! for the game and another one for the sound board (`s11.c:1320` in PinMAME).
//! The 6808 is a 6800 with the oscillator built in: same instruction set, same
//! timings.
//!
//! # About where this code comes from
//!
//! This is **not** a port of the MAME core. `src/cpu/m6800/` in PinMAME is
//! still under the old MAME licence, which is non-commercial and therefore
//! incompatible with this project's GPLv3. The 6800's instruction set, its
//! addressing modes, its flag behaviour and its timings are public Motorola
//! specification, so this implementation was written from there.
//!
//! # Verification
//!
//! Execution timings are a fact about the chip, not a design decision, so the
//! cycle table is checked against PinMAME's
//! (`src/cpu/m6800/m6800.c`, `cycles_6800[]`) with
//! `cargo run -p vpw-m6800 --example cycles`. Of the 203 opcodes with a
//! defined timing, **197 match**. The remaining 6 diverge on purpose:
//!
//! | Opcode | PinMAME | Here | Why |
//! |--------|---------|-----|---------|
//! | `9D`, `DD` | 6 cycles (direct `JSR`) | `HCF` | Direct `JSR` was introduced by the **6801**, replacing HCF. On a 6800/6808 these opcodes hang the CPU. |
//! | `87`, `C7` | 3 cycles (immediate `STAA`/`STAB`) | illegal | There is nowhere to store to in immediate mode. The 6800 does not define them. |
//! | `8F`, `CF` | 4 cycles (immediate `STS`/`STX`) | illegal | Likewise. |
//!
//! Undefined opcodes are not executed silently: they get recorded in
//! [`Cpu::last_illegal`].
//!
//! # Usage
//!
//! ```
//! use vpw_m6800::{Bus, Cpu, FlatMemory, vectors};
//!
//! let mut mem = FlatMemory::new();
//! mem.load(0x0100, &[0x86, 0x2A]); // LDAA #$2A
//! mem.set_vector(vectors::RESET, 0x0100);
//!
//! let mut cpu = Cpu::new();
//! cpu.reset(&mut mem);
//! let cycles = cpu.step(&mut mem);
//!
//! assert_eq!(cpu.a, 0x2A);
//! assert_eq!(cycles, 2);
//! ```

mod ccr;
mod exec;

pub use ccr::{CCR_ALWAYS_SET, Ccr};
pub use vpw_bus::{Bus, FlatMemory};

/// Exception vectors, at the top of the memory map.
pub mod vectors {
    /// Maskable interrupt request.
    pub const IRQ: u16 = 0xFFF8;
    /// Software interrupt (the `SWI` instruction).
    pub const SWI: u16 = 0xFFFA;
    /// Non-maskable interrupt.
    pub const NMI: u16 = 0xFFFC;
    /// Reset.
    pub const RESET: u16 = 0xFFFE;
}

/// CPU state.
#[derive(Debug, Clone)]
pub struct Cpu {
    /// Accumulator A.
    pub a: u8,
    /// Accumulator B.
    pub b: u8,
    /// Index register.
    pub x: u16,
    /// Stack pointer. Points at the next free byte and grows downwards.
    pub sp: u16,
    /// Program counter.
    pub pc: u16,
    /// Condition codes.
    pub cc: Ccr,

    /// Last undefined opcode that was executed, with the PC it sat at.
    /// If this stops being `None` while a real ROM is running, something went
    /// off the rails: either something is left to implement or the control
    /// flow strayed out of its bed.
    pub last_illegal: Option<(u16, u8)>,

    /// The CPU swallowed an `HCF` and is unusable until the next reset.
    pub(crate) hcf: bool,

    /// The CPU is halted in `WAI` waiting for an interrupt.
    pub(crate) waiting: bool,
    /// The state has already been pushed (`WAI` does it before halting), so
    /// the interrupt that wakes the CPU up does not have to push it again.
    pub(crate) context_pushed: bool,

    /// Current level of the IRQ line. It is level sensitive: while it stays
    /// high and `I` is 0, the interrupt fires again.
    irq_line: bool,
    /// Current level of the NMI line, so we can detect the edge.
    nmi_line: bool,
    /// NMI edge already detected and still unserviced.
    nmi_pending: bool,
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
            b: 0,
            x: 0,
            sp: 0,
            pc: 0,
            // After a reset the interrupt mask is left set.
            cc: Ccr {
                i: true,
                ..Ccr::default()
            },
            last_illegal: None,
            hcf: false,
            waiting: false,
            context_pushed: false,
            irq_line: false,
            nmi_line: false,
            nmi_pending: false,
        }
    }

    /// Reset sequence: masks the IRQ and jumps to the RESET vector.
    ///
    /// The accumulators, the index and the stack pointer are left undefined by
    /// the real hardware; here we leave them as they were.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.cc.i = true;
        self.waiting = false;
        self.context_pushed = false;
        self.nmi_pending = false;
        self.hcf = false;
        self.pc = bus.read_u16(vectors::RESET);
    }

    /// Sets the level of the IRQ line. It is level sensitive.
    pub fn set_irq(&mut self, level: bool) {
        self.irq_line = level;
    }

    /// Sets the level of the NMI line. It triggers on the rising edge.
    pub fn set_nmi(&mut self, level: bool) {
        if level && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = level;
    }

    /// The CPU is halted in `WAI`.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// The CPU executed an `HCF` and only a reset gets it out of there.
    pub fn is_hcf(&self) -> bool {
        self.hcf
    }

    /// Executes one instruction (or services a pending interrupt) and returns
    /// the cycles consumed.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.hcf {
            // "Catch fire": it walks the memory map incrementing the PC and no
            // longer responds to interrupts.
            self.pc = self.pc.wrapping_add(1);
            return 1;
        }

        if let Some(cycles) = self.service_interrupts(bus) {
            return cycles;
        }

        if self.waiting {
            // Halted in WAI with nothing to service: the clock keeps running.
            return 1;
        }

        let opcode = self.fetch_u8(bus);
        self.execute(bus, opcode)
    }

    /// Runs until it has consumed at least `cycles` cycles and returns how
    /// many it actually spent (always >= `cycles`, because it does not split
    /// instructions).
    pub fn run(&mut self, bus: &mut impl Bus, cycles: u32) -> u32 {
        let mut spent = 0;
        while spent < cycles {
            spent += self.step(bus);
        }
        spent
    }

    // --- Interrupts ----------------------------------------------------------

    /// Services NMI or IRQ if there is one to service.
    fn service_interrupts(&mut self, bus: &mut impl Bus) -> Option<u32> {
        if self.nmi_pending {
            self.nmi_pending = false;
            return Some(self.enter_interrupt(bus, vectors::NMI));
        }

        // The IRQ only gets in if the mask is clear.
        if self.irq_line && !self.cc.i {
            return Some(self.enter_interrupt(bus, vectors::IRQ));
        }

        None
    }

    /// Pushes the context (if needed), masks the IRQ and jumps to the vector.
    fn enter_interrupt(&mut self, bus: &mut impl Bus, vector: u16) -> u32 {
        // `WAI` already left the context on the stack; pushing it again would
        // corrupt the stack.
        let cycles = if self.context_pushed {
            4
        } else {
            self.push_context(bus);
            12
        };

        self.waiting = false;
        self.context_pushed = false;
        self.cc.i = true;
        self.pc = bus.read_u16(vector);
        cycles
    }

    /// The 6800's push order: PCL, PCH, IXL, IXH, ACCA, ACCB, CCR.
    pub(crate) fn push_context(&mut self, bus: &mut impl Bus) {
        let [pch, pcl] = self.pc.to_be_bytes();
        let [xh, xl] = self.x.to_be_bytes();
        self.push_u8(bus, pcl);
        self.push_u8(bus, pch);
        self.push_u8(bus, xl);
        self.push_u8(bus, xh);
        self.push_u8(bus, self.a);
        self.push_u8(bus, self.b);
        self.push_u8(bus, self.cc.bits());
    }

    /// Pulls the context back in the reverse order (what `RTI` does).
    pub(crate) fn pull_context(&mut self, bus: &mut impl Bus) {
        self.cc = Ccr::from_bits(self.pull_u8(bus));
        self.b = self.pull_u8(bus);
        self.a = self.pull_u8(bus);
        let xh = self.pull_u8(bus);
        let xl = self.pull_u8(bus);
        self.x = u16::from_be_bytes([xh, xl]);
        let pch = self.pull_u8(bus);
        let pcl = self.pull_u8(bus);
        self.pc = u16::from_be_bytes([pch, pcl]);
    }

    // --- Stack and fetch primitives -----------------------------------------

    #[inline]
    pub(crate) fn push_u8(&mut self, bus: &mut impl Bus, value: u8) {
        bus.write(self.sp, value);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline]
    pub(crate) fn pull_u8(&mut self, bus: &mut impl Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(self.sp)
    }

    #[inline]
    pub(crate) fn fetch_u8(&mut self, bus: &mut impl Bus) -> u8 {
        let value = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    #[inline]
    pub(crate) fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
        let hi = self.fetch_u8(bus);
        let lo = self.fetch_u8(bus);
        u16::from_be_bytes([hi, lo])
    }
}
