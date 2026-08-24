//! The sound board, as the CPU board sees it.
//!
//! A thin thing on purpose: the board itself is [`vpw_at91`], and all this
//! does is run it alongside the game and pass the commands along. The two run
//! on different clocks — the game's 6809 at 2 MHz, the sound board's ARM at
//! 40 MHz — so the sound board is caught up in board time rather than
//! interleaved, which is what the real pair do anyway: the CPU board writes a
//! byte and gets on with the game.

use vpw_arm7::Cpu;
use vpw_at91::Sound;
use vpw_at91::board::CLOCK_HZ as ARM_HZ;

use crate::CPU_CLOCK_HZ as GAME_HZ;

/// The sound board and its processor.
pub struct SoundBoard {
    cpu: Cpu,
    board: Box<Sound>,
    /// Cycles of the ARM's own clock.
    cycle: u64,
    /// Samples produced, for a host that wants to know whether it is working.
    pub produced: u64,
    /// Where the output rate is between the last two of the board's samples,
    /// and the one before, for [`SoundBoard::take_audio_at`].
    phase: f64,
    previous: f32,
    /// How many times the firmware has taken the fast interrupt, which is how
    /// it hears about a command. See [`vpw_at91::Sound::send`].
    pub commands_taken: u64,
}

impl SoundBoard {
    pub fn new() -> Self {
        let mut cpu = Cpu::new();
        cpu.reset();
        Self {
            cpu,
            board: Box::new(Sound::new()),
            cycle: 0,
            produced: 0,
            phase: 0.0,
            previous: 0.0,
            commands_taken: 0,
        }
    }

    pub fn board_mut(&mut self) -> &mut Sound {
        &mut self.board
    }

    /// A sound command from the CPU board.
    pub fn send(&mut self, command: u8) {
        self.board.send(command);
    }

    /// Runs the sound board up to the game's cycle count.
    ///
    /// The ARM runs twenty times faster than the game's processor, so the
    /// target is scaled rather than shared.
    pub fn catch_up(&mut self, game_cycle: u64) {
        let target = game_cycle * u64::from(ARM_HZ) / u64::from(GAME_HZ);
        // A bound, because a host that has not called this for a while would
        // otherwise be made to emulate minutes of ARM in one go.
        let most = u64::from(ARM_HZ) / 10;
        let target = target.min(self.cycle + most);
        while self.cycle < target {
            let n = self.cpu.step(&mut *self.board);
            self.cycle += u64::from(n);
            self.cpu.irq = self.board.tick(n);
            let asking = self.board.fiq();
            if asking && !self.cpu.fiq {
                self.commands_taken += 1;
            }
            self.cpu.fiq = asking;
        }
    }

    /// What the board has been up to: fast interrupts asked for, and the four
    /// read counters — command bytes, command words, sample bytes, sample words.
    pub fn diagnostics(&self) -> (u64, [u64; 4], u32, u32) {
        (
            self.commands_taken,
            self.board.reads,
            self.board.aic.enabled,
            self.cpu.r[15],
        )
    }

    /// The stereo samples the board has made, at its own rate.
    pub fn take_audio(&mut self) -> Vec<(i16, i16)> {
        let out = self.board.take_audio();
        self.produced += out.len() as u64;
        out
    }

    /// The same audio as one mono stream at `rate`, ready for a mixer.
    ///
    /// The board runs at whatever its firmware programmed — a shade over
    /// twenty-four kilohertz on the games seen so far — and the mixer runs at
    /// forty-eight, so this resamples. Linear interpolation is enough for
    /// close to a doubling: the error it makes is at half the output rate and
    /// there is nothing up there to alias down. It is also the only kind of
    /// resampling that survives a rate that changes underneath it, which this
    /// one can, because a firmware is free to reprogram its timer.
    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        let from = self.board.sample_rate();
        let taken = self.board.take_audio();
        if from <= 0.0 || taken.is_empty() {
            return Vec::new();
        }
        // How far the output walks along the input for each output sample.
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

impl Default for SoundBoard {
    fn default() -> Self {
        Self::new()
    }
}
