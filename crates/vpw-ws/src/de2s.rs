//! The Sega/Data East sound board: a 6809 driving a BSMT2000.
//!
//! The generation before [`crate::sound`]'s AT91: same job, real silicon. A
//! 68B09E at 2 MHz runs the game's own sound program (`u7`), takes command
//! bytes from the CPU board through a latch, and answers them by programming
//! the BSMT2000's voices out of four megabytes of sample ROM. South Park and
//! every Sega Whitestar up to early Stern carried this board
//! (`SNDBRD_DE2S`, `se.c:357`); the port follows PinMAME's `desound.c`.
//!
//! # The memory map (`de2s_readmem`/`de2s_writemem`, `desound.c:246`)
//!
//! | range           | read              | write                      |
//! |-----------------|-------------------|----------------------------|
//! | `0000-1FFF`     | RAM               | RAM                        |
//! | `2002-2003`     | the command latch |                            |
//! | `2006-2007`     | `0x80`, "ready"   |                            |
//! | `2000-2001`     |                   | chip reset on bit 7 rising |
//! | `6000`          |                   | high byte of the next data |
//! | `A000-A0FF`     |                   | register `!offset` ← data  |
//! | `4000-FFFF`     | program ROM       |                            |
//!
//! The register address arriving inverted is just how the board wired the
//! bus; PinMAME undoes it the same way (`~offset & 0xff`, `desound.c:361`).

use vpw_audio::bsmt2000::Bsmt2000;
use vpw_m6809::{Bus, Cpu};

use crate::CPU_CLOCK_HZ;

/// The fixed interrupt the program's main loop runs on: "489Hz as measured
/// on a real machine" (`desound.c:267`).
const FIRQ_HZ: u32 = 489;

/// The board: processor, program, chip, and the two latches between worlds.
pub struct De2s {
    cpu: Cpu,
    bus: De2sBus,
    /// Cycles of the board's own 2 MHz clock — the same rate as the CPU
    /// board's, which spares the catch-up a conversion.
    cycle: u64,
    next_firq: u64,
    /// Samples produced, for a host that wants to know whether it is working.
    pub produced: u64,
    /// Resampling state for [`De2s::take_audio_at`].
    phase: f64,
    previous: f32,
}

struct De2sBus {
    ram: [u8; 0x2000],
    /// The program, mapped at its natural home: `rom[n]` answers address `n`,
    /// and only `4000-FFFF` is decoded.
    rom: Vec<u8>,
    chip: Bsmt2000,
    /// The byte the CPU board last wrote, and the flag the 6809 polls it by.
    latch: u8,
    /// High byte for the next BSMT register write (`de2s_bsmtcmdHi_w`).
    data_hi: u8,
    /// Last value written to the reset port, for its rising edge.
    last_reset: u8,
    /// Stereo samples the chip has rendered, waiting to be taken.
    pending: Vec<(i16, i16)>,
    /// How far sample production has kept up with the CPU, in cycles.
    rendered_to: u64,
}

impl Bus for De2sBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)],
            0x2002 | 0x2003 => self.latch,
            // "BSMT is always ready" (`de2s_bsmtready_r`).
            0x2006 | 0x2007 => 0x80,
            0x4000..=0xffff => self.rom[usize::from(addr)],
            _ => 0xff,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)] = value,
            0x2000 | 0x2001 => {
                // Bit 7 rising resets the chip, which is also when it reads
                // its mode from the last register touched.
                if self.last_reset & 0x80 == 0 && value & 0x80 != 0 {
                    self.chip.reset();
                }
                self.last_reset = value;
            }
            0x6000 => self.data_hi = value,
            0xa000..=0xa0ff => {
                let reg = !(addr as u8);
                self.chip
                    .write(reg, u16::from(self.data_hi) << 8 | u16::from(value));
            }
            _ => {}
        }
    }
}

impl De2s {
    /// Builds the board from the program ROM and the four sample images.
    ///
    /// `u7` is 64 KB with the vectors at its top; the samples go into the
    /// region in board order — u17, u21, u36, u37 at megabyte strides
    /// (`DE2S_SOUNDROM18888`, `desound.h`) — and the chip's bank remapping
    /// takes it from there.
    pub fn new(u7: &[u8], samples: [&[u8]; 4]) -> Self {
        let mut rom = vec![0xffu8; 0x10000];
        let n = u7.len().min(0x10000);
        // A short image still ends at the top, where the vectors live.
        rom[0x10000 - n..].copy_from_slice(&u7[u7.len() - n..]);

        let mut region = vec![0u8; 0x40_0000];
        for (i, image) in samples.iter().enumerate() {
            let at = i * 0x10_0000;
            let n = image.len().min(0x10_0000);
            region[at..at + n].copy_from_slice(&image[..n]);
        }

        let mut board = Self {
            cpu: Cpu::new(),
            bus: De2sBus {
                ram: [0; 0x2000],
                rom,
                chip: Bsmt2000::new(region, 11),
                latch: 0,
                data_hi: 0,
                last_reset: 0,
                pending: Vec::new(),
                rendered_to: 0,
            },
            cycle: 0,
            next_firq: u64::from(CPU_CLOCK_HZ / FIRQ_HZ),
            produced: 0,
            phase: 0.0,
            previous: 0.0,
        };
        board.cpu.reset(&mut board.bus);
        board
    }

    /// A sound command from the CPU board (`soundlatch_w`). The program
    /// polls the latch from its interrupt; no line is pulled.
    pub fn send(&mut self, command: u8) {
        self.bus.latch = command;
    }

    /// Runs the board up to the game's cycle count, rendering the chip's
    /// output along the way so register writes land between the right
    /// samples.
    pub fn catch_up(&mut self, game_cycle: u64) {
        // A bound, so a host that went quiet does not owe minutes of 6809.
        let most = u64::from(CPU_CLOCK_HZ) / 10;
        let target = game_cycle.min(self.cycle + most);
        if self.cycle < target && self.bus.rendered_to < self.cycle {
            // Whatever ran before this call has already earned its samples.
            self.render_to(self.cycle);
        }
        while self.cycle < target {
            if self.cycle >= self.next_firq {
                // HOLD_LINE semantics: the line pulses and the core takes it
                // once (`de2s_firq`, `desound.c:260`).
                self.cpu.set_firq(true);
                self.next_firq += u64::from(CPU_CLOCK_HZ / FIRQ_HZ);
            }
            let n = self.cpu.step(&mut self.bus);
            self.cpu.set_firq(false);
            self.cycle += u64::from(n);
        }
        self.render_to(self.cycle);
    }

    /// Brings the chip's output up to a moment on the board's clock.
    fn render_to(&mut self, cycle: u64) {
        let rate = self.bus.chip.sample_rate();
        let due = (cycle as f64 / f64::from(CPU_CLOCK_HZ) * rate) as u64;
        let done = (self.bus.rendered_to as f64 / f64::from(CPU_CLOCK_HZ) * rate) as u64;
        let owed = due.saturating_sub(done) as usize;
        if owed > 0 {
            let start = self.bus.pending.len();
            self.bus.pending.resize(start + owed, (0, 0));
            self.bus.chip.render(&mut self.bus.pending[start..]);
        }
        self.bus.rendered_to = cycle;
    }

    /// The stereo samples made since last asked, at the chip's own rate.
    pub fn take_audio(&mut self) -> Vec<(i16, i16)> {
        let out = std::mem::take(&mut self.bus.pending);
        self.produced += out.len() as u64;
        out
    }

    /// The same audio as one mono stream at `rate`, ready for a mixer —
    /// the very resampler [`crate::sound::SoundBoard::take_audio_at`] uses,
    /// for the very same reasons.
    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        let from = self.bus.chip.sample_rate();
        let taken = self.take_audio();
        if from <= 0.0 || taken.is_empty() {
            return Vec::new();
        }
        let step = from / f64::from(rate);
        let mut out = Vec::with_capacity((taken.len() as f64 / step) as usize + 2);
        for (l, r) in taken {
            let sample = (f32::from(l) + f32::from(r)) / 2.0 / 32768.0;
            while self.phase < 1.0 {
                let at = self.phase as f32;
                out.push(self.previous + (sample - self.previous) * at);
                self.phase += step;
            }
            self.phase -= 1.0;
            self.previous = sample;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program whose reset vector lands on an infinite loop, which is all
    /// the plumbing tests need the 6809 to do.
    fn idle_board() -> De2s {
        let mut u7 = vec![0xffu8; 0x10000];
        // BRA * at F000: 0x20 0xFE.
        u7[0xf000] = 0x20;
        u7[0xf001] = 0xfe;
        // Reset vector at FFFE/FFFF -> F000.
        u7[0xfffe] = 0xf0;
        u7[0xffff] = 0x00;
        let empty = [0u8; 0];
        De2s::new(&u7, [&empty, &empty, &empty, &empty])
    }

    #[test]
    fn the_board_runs_and_makes_samples_at_the_chip_rate() {
        let mut b = idle_board();
        // A tenth of a second of board time.
        b.catch_up(u64::from(CPU_CLOCK_HZ) / 10);
        let audio = b.take_audio();
        let expect = 24000 / 10;
        assert!(
            (audio.len() as i64 - expect).abs() < 10,
            "made {} samples, wanted about {expect}",
            audio.len()
        );
    }

    #[test]
    fn the_latch_answers_where_the_program_reads_it() {
        let mut b = idle_board();
        b.send(0x42);
        assert_eq!(b.bus.read(0x2002), 0x42);
        assert_eq!(b.bus.read(0x2006) & 0x80, 0x80, "ready must read ready");
    }

    #[test]
    fn a_register_write_reaches_the_chip_through_the_inverted_bus() {
        let mut b = idle_board();
        // Program voice 0 the way the firmware would: high byte, then the
        // low byte at the inverted register address.
        let mut reg_write = |reg: u8, val: u16| {
            b.bus.write(0x6000, (val >> 8) as u8);
            b.bus.write(0xa000 | u16::from(!reg), val as u8);
        };
        reg_write(0x37, 0); // bank 0
        reg_write(0x21, 0x4000); // loop end
        reg_write(0x16, 0x800); // rate
        // No panic and the chip took them: rendering runs the voice.
        b.catch_up(u64::from(CPU_CLOCK_HZ) / 100);
        let _ = b.take_audio();
    }
}
