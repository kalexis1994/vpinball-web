//! Motorola MC6809 emulator.
//!
//! It is the CPU of the Williams System 11 "CS" sound board (`wmssnd.c:738`,
//! at 2 MHz) and also that of the WPC families, that is, the tables of the 90s.
//!
//! # How it differs from the 6800
//!
//! It is not a 6800 with more instructions: it is another architecture.
//!
//! - **Two index registers** (`X` and `Y`) and **two stacks** (`S` for system
//!   and `U` for user), all four interchangeable as an addressing base.
//! - **Direct page register** (`DP`): direct mode forms the address as
//!   `DP:offset`, so "page zero" can be moved anywhere.
//! - **Indexed addressing with a postbyte**: offsets of 5, 8 or 16 bits,
//!   offsets taken from an accumulator, auto-increment and auto-decrement, and
//!   an **indirect** variant of almost everything.
//! - **Real 16-bit arithmetic**: `D` is the `A:B` pair, with `ADDD`, `SUBD`,
//!   `CMPD`, `LDD`, `STD`, `SEX` and `MUL`.
//! - **Three opcode pages**: the main one plus two prefixed by `$10` and
//!   `$11`.
//! - **Fast interrupt** (`FIRQ`), which stacks only `PC` and `CC`.
//!
//! # About where the code comes from
//!
//! Same as the 6800 core: written from the Motorola specification, not ported
//! from MAME. See the licence note in `vpw-m6800`.
//!
//! # Verification
//!
//! The cycle table of the main page is checked against PinMAME's
//! (`src/cpu/m6809/m6809.c`, `cycles1[]`) with
//! `cargo run -p vpw-m6809 --example cycles`. Of the 224 opcodes with a
//! defined timing, **220 match**. The remaining three diverge on purpose:
//!
//! | Opcode | PinMAME | Here | Why |
//! |--------|---------|-----|---------|
//! | `87` | 2 cycles (`STA` immediate) | illegal | Storing into an immediate operand makes no sense: there is nowhere to write. |
//! | `C7` | 2 cycles (`STB` immediate) | illegal | Same. |
//! | `CF` | 3 cycles (`STU` immediate) | illegal | Same. |
//!
//! It is worth noting that the reference table is not consistent with itself
//! on this: it marks `STX` immediate (`8F`) and `STD` immediate (`CD`) as
//! undefined but gives a timing to `STA`, `STB` and `STU` immediate. Here the
//! six forms are treated alike.
//!
//! # Usage
//!
//! ```
//! use vpw_m6809::{Bus, Cpu, FlatMemory, vectors};
//!
//! let mut mem = FlatMemory::new();
//! mem.load(0x0100, &[0xCC, 0x12, 0x34]); // LDD #$1234
//! mem.set_vector(vectors::RESET, 0x0100);
//!
//! let mut cpu = Cpu::new();
//! cpu.reset(&mut mem);
//! cpu.step(&mut mem);
//!
//! assert_eq!(cpu.d(), 0x1234);
//! assert_eq!(cpu.a, 0x12);
//! assert_eq!(cpu.b, 0x34);
//! ```

mod ccr;
mod exec;
mod indexed;

pub use ccr::Ccr;
pub use indexed::IndexRegister;
pub use vpw_bus::{Bus, FlatMemory};

/// Exception vectors. The 6809 has seven, against the 6800's four.
pub mod vectors {
    /// Software interrupt 3.
    pub const SWI3: u16 = 0xFFF2;
    /// Software interrupt 2.
    pub const SWI2: u16 = 0xFFF4;
    /// Fast interrupt.
    pub const FIRQ: u16 = 0xFFF6;
    /// Maskable interrupt.
    pub const IRQ: u16 = 0xFFF8;
    /// Software interrupt.
    pub const SWI: u16 = 0xFFFA;
    /// Non-maskable interrupt.
    pub const NMI: u16 = 0xFFFC;
    /// Reset.
    pub const RESET: u16 = 0xFFFE;
}

/// CPU state.
#[derive(Debug, Clone)]
pub struct Cpu {
    /// Accumulator A: the **high** byte of `D`.
    pub a: u8,
    /// Accumulator B: the low byte of `D`.
    pub b: u8,
    /// Index register X.
    pub x: u16,
    /// Index register Y.
    pub y: u16,
    /// User stack pointer.
    pub u: u16,
    /// System stack pointer. It is the one interrupts and `JSR` use.
    pub s: u16,
    /// Program counter.
    pub pc: u16,
    /// Direct page register.
    pub dp: u8,
    /// Condition codes.
    pub cc: Ccr,

    /// Last undefined opcode that executed, with the PC where it was.
    ///
    /// For opcodes of the prefixed pages the second byte is stored; the PC
    /// points at the prefix.
    pub last_illegal: Option<(u16, u8)>,

    /// Halted in `SYNC`, waiting for an interrupt line to go down.
    pub(crate) syncing: bool,
    /// Halted in `CWAI`. The state is already stacked, so the interrupt that
    /// wakes it up does not stack it again.
    pub(crate) waiting: bool,

    pub(crate) irq_line: bool,
    pub(crate) firq_line: bool,
    nmi_line: bool,
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
            y: 0,
            u: 0,
            s: 0,
            pc: 0,
            dp: 0,
            // Reset leaves both masks set.
            cc: Ccr {
                i: true,
                f: true,
                ..Ccr::default()
            },
            last_illegal: None,
            syncing: false,
            waiting: false,
            irq_line: false,
            firq_line: false,
            nmi_line: false,
            nmi_pending: false,
        }
    }

    /// The `A:B` pair seen as a 16-bit register.
    #[inline]
    pub fn d(&self) -> u16 {
        u16::from_be_bytes([self.a, self.b])
    }

    #[inline]
    pub fn set_d(&mut self, value: u16) {
        let [a, b] = value.to_be_bytes();
        self.a = a;
        self.b = b;
    }

    /// Reset: `DP` to zero, both masks set, and off to the RESET vector.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.dp = 0;
        self.cc.i = true;
        self.cc.f = true;
        self.syncing = false;
        self.waiting = false;
        self.nmi_pending = false;
        self.pc = bus.read_u16(vectors::RESET);
    }

    /// Sets the level of the IRQ line. It is level sensitive.
    pub fn set_irq(&mut self, level: bool) {
        self.irq_line = level;
    }

    /// Sets the level of the FIRQ line. Level sensitive too.
    pub fn set_firq(&mut self, level: bool) {
        self.firq_line = level;
    }

    /// Sets the level of the NMI line. Triggers on the rising edge.
    pub fn set_nmi(&mut self, level: bool) {
        if level && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = level;
    }

    /// Halted in `SYNC`.
    pub fn is_syncing(&self) -> bool {
        self.syncing
    }

    /// Halted in `CWAI`.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// Executes one instruction (or services an interrupt) and returns the
    /// cycles consumed.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if let Some(cycles) = self.service_interrupts(bus) {
            return cycles;
        }

        // `SYNC` wakes up on any active line, masked or not. If it was masked,
        // it carries straight on without servicing it.
        if self.syncing {
            if self.irq_line || self.firq_line {
                self.syncing = false;
                return 4;
            }
            return 1;
        }

        if self.waiting {
            return 1;
        }

        let opcode = self.fetch_u8(bus);
        self.execute(bus, opcode)
    }

    /// Runs until at least `cycles` cycles have been consumed.
    pub fn run(&mut self, bus: &mut impl Bus, cycles: u32) -> u32 {
        let mut spent = 0;
        while spent < cycles {
            spent += self.step(bus);
        }
        spent
    }

    // --- Interrupts -------------------------------------------------------------

    fn service_interrupts(&mut self, bus: &mut impl Bus) -> Option<u32> {
        if self.nmi_pending {
            self.nmi_pending = false;
            // The NMI stacks the entire state and masks both.
            return Some(self.enter_interrupt(bus, vectors::NMI, true, true, true));
        }

        // The FIRQ has priority over the IRQ and stacks only PC and CC, which
        // is where its name comes from.
        if self.firq_line && !self.cc.f {
            return Some(self.enter_interrupt(bus, vectors::FIRQ, false, true, true));
        }

        if self.irq_line && !self.cc.i {
            return Some(self.enter_interrupt(bus, vectors::IRQ, true, true, false));
        }

        None
    }

    /// Stacks whatever is called for, sets the masks and jumps to the vector.
    pub(crate) fn enter_interrupt(
        &mut self,
        bus: &mut impl Bus,
        vector: u16,
        entire: bool,
        mask_irq: bool,
        mask_firq: bool,
    ) -> u32 {
        let cycles = if self.waiting {
            // `CWAI` already left the complete state on the stack.
            self.waiting = false;
            4
        } else {
            self.cc.e = entire;
            if entire {
                self.push_entire_state(bus);
                if vector == vectors::FIRQ { 10 } else { 19 }
            } else {
                self.push_firq_state(bus);
                10
            }
        };

        self.syncing = false;
        if mask_irq {
            self.cc.i = true;
        }
        if mask_firq {
            self.cc.f = true;
        }
        self.pc = bus.read_u16(vector);
        cycles
    }

    /// The twelve bytes of the complete state, in the order `RTI` expects:
    /// PCL, PCH, UL, UH, YL, YH, XL, XH, DP, B, A, CC.
    pub(crate) fn push_entire_state(&mut self, bus: &mut impl Bus) {
        let [pch, pcl] = self.pc.to_be_bytes();
        let [uh, ul] = self.u.to_be_bytes();
        let [yh, yl] = self.y.to_be_bytes();
        let [xh, xl] = self.x.to_be_bytes();
        self.push_s(bus, pcl);
        self.push_s(bus, pch);
        self.push_s(bus, ul);
        self.push_s(bus, uh);
        self.push_s(bus, yl);
        self.push_s(bus, yh);
        self.push_s(bus, xl);
        self.push_s(bus, xh);
        self.push_s(bus, self.dp);
        self.push_s(bus, self.b);
        self.push_s(bus, self.a);
        self.push_s(bus, self.cc.bits());
    }

    /// What a FIRQ stacks: only `PC` and `CC`.
    fn push_firq_state(&mut self, bus: &mut impl Bus) {
        let [pch, pcl] = self.pc.to_be_bytes();
        self.push_s(bus, pcl);
        self.push_s(bus, pch);
        self.push_s(bus, self.cc.bits());
    }

    // --- Stack and fetch --------------------------------------------------------------

    /// The 6809 stack **pre-decrements**: it points at the last byte stored,
    /// not at the next hole. It is the other way round from the 6800.
    #[inline]
    pub(crate) fn push_s(&mut self, bus: &mut impl Bus, value: u8) {
        self.s = self.s.wrapping_sub(1);
        bus.write(self.s, value);
    }

    #[inline]
    pub(crate) fn pull_s(&mut self, bus: &mut impl Bus) -> u8 {
        let value = bus.read(self.s);
        self.s = self.s.wrapping_add(1);
        value
    }

    #[inline]
    pub(crate) fn push_u(&mut self, bus: &mut impl Bus, value: u8) {
        self.u = self.u.wrapping_sub(1);
        bus.write(self.u, value);
    }

    #[inline]
    pub(crate) fn pull_u(&mut self, bus: &mut impl Bus) -> u8 {
        let value = bus.read(self.u);
        self.u = self.u.wrapping_add(1);
        value
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
