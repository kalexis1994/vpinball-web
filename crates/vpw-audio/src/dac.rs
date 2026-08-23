//! 8-bit digital-to-analog converter.
//!
//! It is the simplest thing on the sound board: a one-byte latch whose value
//! is directly the level of the signal. The sound CPU writes samples to it by
//! hand, in a timed loop, and that is where the effects come from.

/// An unsigned 8-bit DAC.
///
/// The idle value is `$80`, half scale: the codes below it are the negative
/// half of the waveform and the ones above it the positive half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dac {
    level: u8,
}

/// Silence code: half scale.
pub const SILENCE: u8 = 0x80;

impl Default for Dac {
    fn default() -> Self {
        Self::new()
    }
}

impl Dac {
    pub fn new() -> Self {
        Self { level: SILENCE }
    }

    /// Back to idle.
    pub fn reset(&mut self) {
        self.level = SILENCE;
    }

    /// Writes a new code. The level is held until the next write, just like on
    /// the chip.
    pub fn write(&mut self, value: u8) {
        self.level = value;
    }

    /// The raw code currently loaded.
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Sample normalised to `-1.0..1.0`.
    pub fn sample(&self) -> f32 {
        (f32::from(self.level) - f32::from(SILENCE)) / 128.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_idle_it_is_silent() {
        assert_eq!(Dac::new().sample(), 0.0);
    }

    #[test]
    fn the_extremes_give_the_full_scale() {
        let mut dac = Dac::new();

        dac.write(0xFF);
        assert!(
            (dac.sample() - 0.9921875).abs() < 1e-6,
            "almost the maximum"
        );

        dac.write(0x00);
        assert_eq!(dac.sample(), -1.0, "the exact minimum");
    }

    #[test]
    fn the_level_is_held_between_writes() {
        let mut dac = Dac::new();
        dac.write(0xC0);
        assert_eq!(dac.sample(), dac.sample(), "reading it does not consume it");
        assert_eq!(dac.level(), 0xC0);
    }

    #[test]
    fn it_never_goes_out_of_range() {
        let mut dac = Dac::new();
        for value in 0..=255u8 {
            dac.write(value);
            assert!(
                (-1.0..=1.0).contains(&dac.sample()),
                "{value} gave {}",
                dac.sample()
            );
        }
    }
}
