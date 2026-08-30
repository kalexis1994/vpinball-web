//! The audio chips of the pinball sound boards.
//!
//! The 8-bit DAC and the HC55516 CVSD decoder of the XS board, the YM2151 —
//! the FM synthesiser of the CS board — and the AY-3-8910, which is what a
//! Gottlieb System 80B makes its music with.
//!
//! # How you get audio out of this
//!
//! The chips do not produce samples: they produce a **level** that is held
//! between writes. The sound CPU keeps changing it by hand, in timed loops
//! with the cycles counted. To record that you have to sample the level at a
//! fixed rate, and that is what [`SampleClock`] takes care of:
//!
//! ```
//! use vpw_audio::{mix, Dac, SampleClock};
//!
//! let mut dac = Dac::new();
//! let mut clock = SampleClock::new(1_000_000, 48_000);
//! let mut output = Vec::new();
//!
//! // The CPU writes the DAC and runs a few cycles...
//! dac.write(0xC0);
//! for _ in 0..clock.advance(100) {
//!     output.push(mix(&[dac.sample()]));
//! }
//!
//! assert!(!output.is_empty());
//! ```

pub mod ay8910;
pub mod biquad;
pub mod bsmt2000;
mod cvsd;
mod dac;
mod dcblock;
pub mod mixer;
pub mod sp0250;
mod ym2151;

pub use ay8910::Ay8910;
pub use biquad::{Biquad, OutputFilter};
pub use cvsd::Cvsd;
pub use dac::{Dac, SILENCE};
pub use dcblock::DcBlocker;
pub use mixer::{Mixer, Play};
pub use sp0250::Sp0250;
pub use ym2151::{LfoWave, Ym2151};

/// Decides when to take a sample while the CPU clock runs.
///
/// The CPU clock and the audio clock are not multiples of each other, so the
/// remainder is carried over: without that the output rate would drift and the
/// audio would change pitch.
#[derive(Debug, Clone)]
pub struct SampleClock {
    cycles_per_sample: f64,
    accumulator: f64,
}

impl SampleClock {
    /// `cpu_hz` is the sound CPU clock and `sample_rate` the desired output
    /// rate.
    pub fn new(cpu_hz: u32, sample_rate: u32) -> Self {
        debug_assert!(cpu_hz > 0 && sample_rate > 0);
        Self {
            cycles_per_sample: f64::from(cpu_hz) / f64::from(sample_rate),
            accumulator: 0.0,
        }
    }

    /// Adds CPU cycles and returns how many samples have to be taken.
    pub fn advance(&mut self, cycles: u32) -> u32 {
        self.accumulator += f64::from(cycles);
        let samples = (self.accumulator / self.cycles_per_sample) as u32;
        self.accumulator -= f64::from(samples) * self.cycles_per_sample;
        samples
    }

    /// How many CPU cycles one sample lasts.
    pub fn cycles_per_sample(&self) -> f64 {
        self.cycles_per_sample
    }

    pub fn reset(&mut self) {
        self.accumulator = 0.0;
    }
}

/// Sums several sources into a single sample.
///
/// It divides by the number of sources instead of clipping: a sound board with
/// the DAC and the CVSD both playing loud would saturate horribly, and on the
/// real board the two signals go through an analog summer that does not add
/// them at unity gain either.
pub fn mix(sources: &[f32]) -> f32 {
    if sources.is_empty() {
        return 0.0;
    }
    let sum: f32 = sources.iter().sum();
    (sum / sources.len() as f32).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_clock_does_not_drift() {
        // At 1 MHz and 48 kHz a sample falls every 20.833 cycles. If the
        // remainder were not carried over, the rate would end up at 1/20 and
        // the audio would come out sharp.
        let mut clock = SampleClock::new(1_000_000, 48_000);
        let mut total = 0;
        for _ in 0..1000 {
            total += clock.advance(1_000); // a million cycles in total
        }
        assert_eq!(total, 48_000, "one second of CPU is 48000 samples");
    }

    #[test]
    fn the_clock_copes_with_irregular_advances() {
        // Instructions take different times, so the advances are not even.
        let mut clock = SampleClock::new(1_000_000, 48_000);
        let mut total = 0;
        let mut cycles = 0u32;
        let mut n = 2u32;
        while cycles < 1_000_000 {
            let step = n.min(1_000_000 - cycles);
            total += clock.advance(step);
            cycles += step;
            n = if n >= 12 { 2 } else { n + 1 };
        }
        assert_eq!(total, 48_000);
    }

    #[test]
    fn mixing_averages_and_clips() {
        assert_eq!(mix(&[]), 0.0);
        assert_eq!(mix(&[0.5]), 0.5);
        assert_eq!(mix(&[1.0, 0.0]), 0.5);
        assert_eq!(mix(&[1.0, 1.0]), 1.0, "two at full scale do not overshoot");
        assert_eq!(mix(&[-1.0, -1.0]), -1.0);
    }
}
