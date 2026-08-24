//! An ARM7TDMI, because a pinball machine from 2003 has one making the noise.
//!
//! Stern's third-generation sound board is an Atmel AT91 — an ARM7TDMI — and a
//! Xilinx FPGA. The interesting part is that **the sound chip is not a chip**:
//! the board emulates a BSMT2000 in software on the ARM, which is why there is
//! no sample player to write here and no shortcut past the processor. The
//! samples are in the ROMs, the code that turns a command into a voice is in
//! the ROMs, and the only way to hear any of it is to run the ROMs.
//!
//! # What this is
//!
//! The whole user-mode and privileged instruction set of an ARM7TDMI in ARM
//! state: data processing with the barrel shifter, the two multiply forms,
//! single and block transfers, halfword and signed transfers, the swap, the
//! status-register transfers, and the exceptions. Thumb state is in
//! [`thumb`].
//!
//! # The one thing that trips every ARM emulator
//!
//! `R15` reads as the instruction's address **plus eight** in ARM state, and
//! plus four in Thumb. That is not a quirk to paper over: it is the pipeline,
//! it is architectural, and code relies on it — `add r0, pc, #4` is how a
//! compiler of that era addressed a literal. Every read of R15 in here goes
//! through [`Cpu::reg`] for that reason, and nothing reads `self.r[15]`
//! directly except the fetch.

pub mod thumb;

/// What the processor talks to. Addresses are 32 bits and so is the data path.
///
/// Separate from `vpw_bus::Bus`, which is a 16-bit address and a byte: the two
/// have nothing in common but the name.
pub trait Bus {
    fn read32(&mut self, addr: u32) -> u32;
    fn write32(&mut self, addr: u32, value: u32);
    fn read8(&mut self, addr: u32) -> u8;
    fn write8(&mut self, addr: u32, value: u8);

    /// Halfwords, for the `LDRH`/`STRH` family.
    fn read16(&mut self, addr: u32) -> u16 {
        u16::from(self.read8(addr)) | (u16::from(self.read8(addr.wrapping_add(1))) << 8)
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.write8(addr, value as u8);
        self.write8(addr.wrapping_add(1), (value >> 8) as u8);
    }
}

/// The processor's modes. The number is the bottom five bits of the CPSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x1b,
    System = 0x1f,
}

impl Mode {
    fn from_bits(bits: u32) -> Self {
        match bits & 0x1f {
            0x11 => Mode::Fiq,
            0x12 => Mode::Irq,
            0x13 => Mode::Supervisor,
            0x17 => Mode::Abort,
            0x1b => Mode::Undefined,
            0x1f => Mode::System,
            // Anything else is treated as user, which is what a real part does
            // with a reserved encoding rather than catching fire.
            _ => Mode::User,
        }
    }

    /// Which bank of registers this mode sees, or `None` for the two that use
    /// the user bank.
    fn bank(self) -> Option<usize> {
        match self {
            Mode::User | Mode::System => None,
            Mode::Fiq => Some(0),
            Mode::Irq => Some(1),
            Mode::Supervisor => Some(2),
            Mode::Abort => Some(3),
            Mode::Undefined => Some(4),
        }
    }
}

/// Condition-code bits of the CPSR.
const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;
/// Interrupt disables, and the Thumb bit.
const I: u32 = 1 << 7;
const F: u32 = 1 << 6;
const T: u32 = 1 << 5;

/// Where each exception sends the processor.
mod vectors {
    pub const RESET: u32 = 0x00;
    pub const UNDEFINED: u32 = 0x04;
    pub const SWI: u32 = 0x08;
    /// Neither is taken by this core. The AT91 in a sound board has no memory
    /// management and nothing to abort on, and a core that invented aborts
    /// would send the firmware somewhere it never expects to go.
    #[expect(dead_code, reason = "named so the vector table is complete")]
    pub const PREFETCH_ABORT: u32 = 0x0c;
    #[expect(dead_code, reason = "named so the vector table is complete")]
    pub const DATA_ABORT: u32 = 0x10;
    pub const IRQ: u32 = 0x18;
    pub const FIQ: u32 = 0x1c;
}

/// An ARM7TDMI.
pub struct Cpu {
    /// The sixteen visible registers. `r[15]` is the program counter and holds
    /// the address of the instruction being fetched, **not** what a read of
    /// R15 answers; see [`Cpu::reg`].
    pub r: [u32; 16],
    pub cpsr: u32,

    /// The banked registers, by [`Mode::bank`].
    ///
    /// FIQ banks r8 to r14 and everything else banks only r13 and r14, so the
    /// wide slot is kept for all of them and the narrow modes use the top two.
    /// Simpler than two shapes, and the cost is twenty words.
    banked: [[u32; 7]; 5],
    /// The user-mode r8 to r14, parked while FIQ has the bank.
    user: [u32; 7],
    spsr: [u32; 5],

    /// The lines, as levels. Sampled between instructions.
    pub irq: bool,
    pub fiq: bool,

    /// Set when the processor has executed a `SWI` this step, for a host that
    /// wants to answer one itself.
    pub swi: Option<u32>,

    /// Whether the instruction just executed wrote the program counter.
    ///
    /// Not something that can be worked out by comparing: `b .` is a spin loop
    /// and a perfectly ordinary thing for firmware to sit in, and it writes the
    /// program counter with the value it already had. So the write is recorded
    /// rather than inferred.
    branched: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            // Reset leaves it in supervisor with both interrupts masked.
            cpsr: Mode::Supervisor as u32 | I | F,
            banked: [[0; 7]; 5],
            user: [0; 7],
            spsr: [0; 5],
            irq: false,
            fiq: false,
            swi: None,
            branched: false,
        }
    }

    /// Puts it back where a power-on leaves it.
    pub fn reset(&mut self) {
        self.cpsr = Mode::Supervisor as u32 | I | F;
        self.r[15] = vectors::RESET;
    }

    pub fn mode(&self) -> Mode {
        Mode::from_bits(self.cpsr)
    }

    pub fn thumb(&self) -> bool {
        self.cpsr & T != 0
    }

    /// Reads a register **as an instruction sees it**.
    ///
    /// Which for R15 is the pipeline: the address of the instruction plus
    /// eight in ARM state and plus four in Thumb. Code of this era depends on
    /// it — `add r0, pc, #4` is how a literal was addressed — so this is not a
    /// detail to round off.
    pub fn reg(&self, n: usize) -> u32 {
        if n == 15 {
            self.r[15].wrapping_add(if self.thumb() { 4 } else { 8 })
        } else {
            self.r[n]
        }
    }

    fn set_reg(&mut self, n: usize, v: u32) {
        if n == 15 {
            self.r[15] = v & !1;
            self.branched = true;
        } else {
            self.r[n] = v;
        }
    }

    /// The user-mode view of a register, for the transfers that ask for it —
    /// `LDM`/`STM` with the `^` and `MRS`/`MSR` in a banked mode.
    fn user_reg(&self, n: usize) -> u32 {
        match (self.mode().bank(), n) {
            (None, _) | (_, 0..=7) | (_, 15) => self.reg(n),
            (Some(_), _) => self.user[n - 8],
        }
    }

    fn set_user_reg(&mut self, n: usize, v: u32) {
        match (self.mode().bank(), n) {
            (None, _) | (_, 0..=7) | (_, 15) => self.set_reg(n, v),
            (Some(_), _) => self.user[n - 8] = v,
        }
    }

    /// Moves to another mode, swapping the banked registers over.
    ///
    /// The awkward one is FIQ, which banks r8 to r14 while every other mode
    /// banks only r13 and r14. Coming out of FIQ has to put r8 to r12 back
    /// where user mode left them, and coming out of anything else must not.
    pub fn set_mode(&mut self, to: Mode) {
        let from = self.mode();
        if from == to {
            self.cpsr = (self.cpsr & !0x1f) | to as u32;
            return;
        }

        // Park what the old mode was using.
        let wide = from == Mode::Fiq;
        match from.bank() {
            Some(b) => {
                let lo = if wide { 0 } else { 5 };
                for i in lo..7 {
                    self.banked[b][i] = self.r[8 + i];
                }
                if !wide {
                    for i in 0..5 {
                        self.user[i] = self.r[8 + i];
                    }
                }
            }
            None => {
                for i in 0..7 {
                    self.user[i] = self.r[8 + i];
                }
            }
        }

        // And bring in what the new one uses.
        let wide = to == Mode::Fiq;
        match to.bank() {
            Some(b) => {
                let lo = if wide { 0 } else { 5 };
                for i in 0..lo {
                    self.r[8 + i] = self.user[i];
                }
                for i in lo..7 {
                    self.r[8 + i] = self.banked[b][i];
                }
            }
            None => {
                for i in 0..7 {
                    self.r[8 + i] = self.user[i];
                }
            }
        }

        self.cpsr = (self.cpsr & !0x1f) | to as u32;
    }

    pub fn spsr(&self) -> u32 {
        match self.mode().bank() {
            Some(b) => self.spsr[b],
            // User and system have no saved status. Reading it gives the
            // current one, which is what the architecture says is
            // unpredictable and what every part actually does.
            None => self.cpsr,
        }
    }

    fn set_spsr(&mut self, v: u32) {
        if let Some(b) = self.mode().bank() {
            self.spsr[b] = v;
        }
    }

    /// Whether a condition code holds.
    fn passes(&self, cond: u32) -> bool {
        let (n, z, c, v) = (
            self.cpsr & N != 0,
            self.cpsr & Z != 0,
            self.cpsr & C != 0,
            self.cpsr & V != 0,
        );
        match cond {
            0x0 => z,
            0x1 => !z,
            0x2 => c,
            0x3 => !c,
            0x4 => n,
            0x5 => !n,
            0x6 => v,
            0x7 => !v,
            0x8 => c && !z,
            0x9 => !c || z,
            0xa => n == v,
            0xb => n != v,
            0xc => !z && n == v,
            0xd => z || n != v,
            _ => true,
        }
    }

    fn set_flag(&mut self, bit: u32, on: bool) {
        if on {
            self.cpsr |= bit;
        } else {
            self.cpsr &= !bit;
        }
    }

    fn set_nz(&mut self, v: u32) {
        self.set_flag(N, v & 0x8000_0000 != 0);
        self.set_flag(Z, v == 0);
    }

    /// Enters an exception: banks the return address and the status, switches
    /// mode, masks what has to be masked, and jumps to the vector.
    fn enter(&mut self, vector: u32, mode: Mode, return_from: u32, mask_fiq: bool) {
        let cpsr = self.cpsr;
        self.set_mode(mode);
        self.set_spsr(cpsr);
        self.r[14] = return_from;
        self.cpsr &= !T;
        self.cpsr |= I;
        if mask_fiq {
            self.cpsr |= F;
        }
        self.set_reg(15, vector);
    }

    /// Takes an interrupt if one is asserted and not masked.
    ///
    /// Between instructions, which is where a real part samples them.
    fn interrupts(&mut self) -> bool {
        // The return address is the instruction after the one that would have
        // run, plus the pipeline: four in ARM, and four in Thumb as well
        // because the return is through `SUBS pc, lr, #4`.
        let step = if self.thumb() { 2 } else { 4 };
        if self.fiq && self.cpsr & F == 0 {
            let ret = self.r[15].wrapping_add(step).wrapping_add(4);
            self.enter(vectors::FIQ, Mode::Fiq, ret, true);
            return true;
        }
        if self.irq && self.cpsr & I == 0 {
            let ret = self.r[15].wrapping_add(step).wrapping_add(4);
            self.enter(vectors::IRQ, Mode::Irq, ret, false);
            return true;
        }
        let _ = step;
        false
    }

    /// Runs one instruction and returns the cycles it took.
    ///
    /// The count is an approximation: this core charges the sequential and
    /// non-sequential cycles the architecture manual gives for each class,
    /// which is what matters for a sound board whose job is to produce samples
    /// at a rate, and does not model the memory system's wait states.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        self.swi = None;
        if self.interrupts() {
            return 3;
        }
        if self.thumb() {
            thumb::step(self, bus)
        } else {
            self.step_arm(bus)
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

mod arm;
