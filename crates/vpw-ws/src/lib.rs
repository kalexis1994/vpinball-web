//! Sega and Stern's **Whitestar** board, the one behind Lord of the Rings.
//!
//! Sega bought Data East's pinball division in 1994 and Whitestar is what it
//! made of that design; Stern bought it from Sega in 1999 and kept building it
//! until S.A.M. replaced it in 2006. PinMAME calls the whole line `se`
//! (`src/wpc/se.c`) and the generation `GEN_WS`.
//!
//! # What is here and what is not
//!
//! The **CPU board**: a 6809 at 2 MHz, its memory map, the banked ROM, the lamp
//! matrix, the switch matrix and the coils. That is what makes a game run — a
//! coin credits, a start button starts, targets score, the flippers get their
//! solenoids — and it is what a table's script talks to.
//!
//! Not here yet, and both are separate boards with their own processors:
//!
//! - The **display**, a 128 by 32 dot matrix driven by a second 6809 with its
//!   own ROM. The CPU board only talks to it through a latch at `$3600` and a
//!   status byte at `$3700`, so its absence costs the score display and
//!   nothing else.
//! - The **sound**. For a game of this vintage that is an Atmel AT91 — an ARM7
//!   — with an FPGA standing in for a BSMT2000, which is an entire second
//!   instruction set. The CPU board only ever *writes* to it (`$3800`), so
//!   leaving it disconnected costs the sound and cannot stall the game.
//!
//! # The two clocks
//!
//! A Whitestar has no periodic interrupt from a PIA the way a System 11 does.
//! It takes a FIRQ from a timer at a fixed rate and does its bookkeeping on the
//! video blank, and the original is explicit that the second of those has to
//! happen twice a frame *because of this game*:
//!
//! > (at least) LOTR requires some kind of oversampling of the lamp state,
//! > otherwise the flickering lamps during modes are always on!

pub mod board;
pub mod games;
pub mod io;

use vpw_m6809::Cpu;

pub use board::Board;

/// The main CPU's clock. `MDRV_CPU_ADD(M6809, 2000000)`, an MC6809E.
pub const CPU_CLOCK_HZ: u32 = 2_000_000;

/// How often the board takes a FIRQ.
///
/// Nine hundred and seventy-six hertz, and the original notes where the number
/// comes from: "FIRQ Frequency according to Theory of Operation" (`se.c:46`).
pub const FIRQ_HZ: u32 = 976;

/// How long the FIRQ line is held. The original pulses it, which in its CPU
/// core means asserted for one instruction.
const FIRQ_HOLD_CYCLES: u64 = 4;

/// How many times a second the board closes a sweep of the lamps.
///
/// Twice per frame at sixty frames, which is the original's `VBLANK` of 2 and
/// the comment that goes with it: a game whose lamps flicker as part of its
/// artwork reads as permanently on if the sweep is closed any slower.
pub const SWEEP_HZ: u32 = 120;

/// A Whitestar machine.
pub struct Whitestar {
    pub cpu: Cpu,
    pub board: Board,
    /// Cycles since reset.
    cycle: u64,
    next_firq: u64,
    firq_until: u64,
    next_sweep: u64,
    /// How many FIRQs have been raised, for finding out whether the board is
    /// taking them at all.
    pub firqs: u64,
}

impl Whitestar {
    pub fn new() -> Self {
        let mut board = Board::new();
        let mut cpu = Cpu::new();
        cpu.reset(&mut board);
        Self {
            cpu,
            board,
            cycle: 0,
            next_firq: u64::from(CPU_CLOCK_HZ / FIRQ_HZ),
            firq_until: 0,
            next_sweep: u64::from(CPU_CLOCK_HZ / SWEEP_HZ),
            firqs: 0,
        }
    }

    /// Loads a game's CPU image and starts the processor on it.
    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), String> {
        self.board.load_rom(image)?;
        self.reset();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.board);
        self.cycle = 0;
        self.next_firq = u64::from(CPU_CLOCK_HZ / FIRQ_HZ);
        self.firq_until = 0;
        self.next_sweep = u64::from(CPU_CLOCK_HZ / SWEEP_HZ);
    }

    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// One instruction, with the board's clocks kept up to date around it.
    pub fn step(&mut self) -> u32 {
        if self.cycle >= self.next_firq {
            self.cpu.set_firq(true);
            self.firq_until = self.cycle + FIRQ_HOLD_CYCLES;
            self.firqs += 1;
            self.next_firq += u64::from(CPU_CLOCK_HZ / FIRQ_HZ);
        }
        if self.firq_until != 0 && self.cycle >= self.firq_until {
            self.cpu.set_firq(false);
            self.firq_until = 0;
        }

        let cycles = self.cpu.step(&mut self.board);
        self.cycle += u64::from(cycles);

        // Closing the sweep is what makes a lamp that is strobed for a fraction
        // of it read as lit for the whole of it. See [`SWEEP_HZ`].
        if self.cycle >= self.next_sweep {
            self.board.lamps.end_sweep();
            self.next_sweep += u64::from(CPU_CLOCK_HZ / SWEEP_HZ);
        }

        cycles
    }

    /// Runs at least `cycles` cycles.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let target = self.cycle + cycles;
        while self.cycle < target {
            self.step();
        }
        self.cycle
    }

    /// Runs the equivalent of `seconds` of real time.
    pub fn run_seconds(&mut self, seconds: f64) -> u64 {
        self.run_cycles((seconds * f64::from(CPU_CLOCK_HZ)) as u64)
    }
}

impl Default for Whitestar {
    fn default() -> Self {
        Self::new()
    }
}
