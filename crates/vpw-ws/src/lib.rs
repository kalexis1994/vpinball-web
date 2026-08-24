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
pub mod dmd;
pub mod games;
pub mod io;
pub mod sound;

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

/// How many times a second the board closes a window of the lamp sweep.
///
/// Sixty, and it is a floor rather than a preference: the board strobes ten
/// columns off a 976 Hz interrupt, so a full sweep takes about ten
/// milliseconds, and a window shorter than that misses whichever columns fall
/// outside it every single time. Two of these windows make one decision, at
/// thirty times a second. See [`io::Lamps`].
pub const WINDOW_HZ: u32 = 60;

/// A Whitestar machine.
pub struct Whitestar {
    pub cpu: Cpu,
    pub board: Board,
    /// The display, if this game's board was given one. See [`dmd::Dmd`].
    pub dmd: Option<Box<dmd::Dmd>>,
    /// The sound board, if this game's was given its ROMs. See [`sound`].
    pub sound: Option<Box<sound::SoundBoard>>,
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
            dmd: None,
            sound: None,
            cycle: 0,
            next_firq: u64::from(CPU_CLOCK_HZ / FIRQ_HZ),
            firq_until: 0,
            next_sweep: u64::from(CPU_CLOCK_HZ / WINDOW_HZ),
            firqs: 0,
        }
    }

    /// Loads a game's CPU image and starts the processor on it.
    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), String> {
        self.board.load_rom(image)?;
        self.reset();
        Ok(())
    }

    /// Gives the machine its display board.
    pub fn load_display_rom(&mut self, image: &[u8]) -> Result<(), String> {
        let mut dmd = dmd::Dmd::new();
        dmd.load_rom(image)?;
        self.dmd = Some(Box::new(dmd));
        Ok(())
    }

    /// Gives the machine its sound board.
    ///
    /// Four ROMs and a BIOS: the two-megabyte `bios.u8` every one of these
    /// boards carries, the game's own sixty-four kilobyte `u7`, and four
    /// megabytes of samples.
    pub fn load_sound(&mut self, bios: &[u8], u7: &[u8], samples: [&[u8]; 4]) {
        let mut board = sound::SoundBoard::new();
        board.board_mut().load_bios(bios);
        board.board_mut().load_u7(u7);
        for (i, image) in samples.iter().enumerate() {
            board.board_mut().load_samples(i, image);
        }
        self.sound = Some(Box::new(board));
    }

    /// The stereo samples the sound board has made since this was last asked.
    pub fn take_audio(&mut self) -> Vec<(i16, i16)> {
        match &mut self.sound {
            Some(s) => s.take_audio(),
            None => Vec::new(),
        }
    }

    /// Whether the sound board's images were all there, and how many samples
    /// it has made since it was built.
    ///
    /// The one pair of numbers that tells a player why a machine is quiet: no
    /// board at all is a missing image in the zip, a board making nothing is a
    /// firmware that has not got going, and a board making twenty-four thousand
    /// a second is one doing its job with nothing to say.
    pub fn sound_stats(&self) -> (bool, u64) {
        match &self.sound {
            Some(s) => (true, s.produced),
            None => (false, 0),
        }
    }

    /// The same, as mono at the rate a mixer wants. See
    /// [`sound::SoundBoard::take_audio_at`].
    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        match &mut self.sound {
            Some(s) => s.take_audio_at(rate),
            None => Vec::new(),
        }
    }

    /// The dot matrix, one byte per dot from 0 to 3, or nothing if this
    /// machine has no display board loaded.
    pub fn dmd_frame(&self) -> Option<&[u8]> {
        self.dmd.as_ref().map(|d| &d.frame()[..])
    }

    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.board);
        self.cycle = 0;
        self.next_firq = u64::from(CPU_CLOCK_HZ / FIRQ_HZ);
        self.firq_until = 0;
        self.next_sweep = u64::from(CPU_CLOCK_HZ / WINDOW_HZ);
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

        // The sound board hears about a command the moment the CPU board
        // writes one, and otherwise runs on its own clock alongside.
        if let Some(sound) = &mut self.sound {
            if std::mem::take(&mut self.board.sound_pending) {
                sound.send(self.board.sound_latch);
            }
            sound.catch_up(self.cycle);
        }

        // The display board runs on its own 2 MHz clock, which is this one's,
        // and takes its commands through the latch at `$3600`. It is caught up
        // rather than interleaved instruction by instruction: it answers the
        // CPU board only through a busy line and a status byte, and both of
        // those are levels that survive being read a few microseconds late.
        if let Some(display) = &mut self.dmd {
            if std::mem::take(&mut self.board.dmd_data_pending) {
                display.set_data(self.board.dmd_latch);
                // The CPU board strobes it in with a low-then-high pulse, which
                // is what `dmdlatch_w` does (`se.c:725`).
                display.set_ctrl(self.board.dmd_ctrl & !1);
                display.set_ctrl(self.board.dmd_ctrl | 1);
                self.board.dmd_ctrl |= 1;
            }
            if let Some(ctrl) = self.board.dmd_ctrl_pending.take() {
                display.set_ctrl(ctrl);
                self.board.dmd_ctrl = ctrl;
            }
            display.run_until(self.cycle);
            self.board.dmd_status = (u8::from(display.busy()) << 7) | (display.status() << 3);
        }

        // Closing the window is what makes a lamp that is strobed for a
        // fraction of it read as lit at all. See [`WINDOW_HZ`].
        if self.cycle >= self.next_sweep {
            self.board.lamps.end_window();
            self.next_sweep += u64::from(CPU_CLOCK_HZ / WINDOW_HZ);
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
