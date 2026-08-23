//! Motorola HC55516 CVSD decoder.
//!
//! It is the chip that does the speech on the 80s machines — the spoken
//! "F-14 Tomcat" the table says at power-up comes out of here.
//!
//! # How it works
//!
//! CVSD is *continuously variable slope delta modulation*: the audio does not
//! travel as samples but as **one bit per clock cycle**, and each bit says
//! nothing more than whether the signal goes up or down. All the work is in
//! deciding *how much* it goes up or down, and that is what the syllabic
//! filter takes care of:
//!
//! - A shift register holds the last three bits.
//! - If all three are the same, the decoder has been falling short: the real
//!   signal is changing faster than the step can keep up with, so the step
//!   **grows**.
//! - If they are not all the same, it is tracking well and the step **decays**.
//! - The integrator accumulates that step, signed according to the bit, and
//!   leaks charge little by little so it does not drift.
//!
//! That automatic adjustment of the step is what lets it fit intelligible
//! speech into a single bit per sample.
//!
//! # Origin
//!
//! Ported from PinMAME's `src/sound/hc55516.c`, which is under BSD-3-Clause
//! and therefore compatible with this project's GPLv3. The filter constants
//! come from the analysis of the real chip that file documents.

// Both filters are carried in double precision, like the reference
// implementation: the constants come from the analysis of the real chip and in
// `f32` they would lose digits that do matter. The conversion to `f32` happens
// only at the output.

/// Syllabic filter: charge constant, 4 ms at 16 kHz.
const CHARGE: f64 = 0.984_496_437_005_408_5;
/// Estimator filter (the integrator leak): 1 ms at 16 kHz.
const DECAY: f64 = 0.939_413_062_813_475_8;

/// The HC55516 carries a **three**-bit shift register. Its bigger brothers
/// (HC55532/36/64) use four.
const SHIFT_MASK: u8 = 0x07;

/// Voltage on the high input of the syllabic filter. The datasheet gives
/// 1.2 V RMS at 0 dB, which is 1.7 V peak.
const V_HIGH: f64 = 1.7;
/// The low input is ground.
const V_LOW: f64 = 0.0;

/// One HC55516.
#[derive(Debug, Clone)]
pub struct Cvsd {
    /// The last three bits received.
    shift_register: u8,
    /// Simulated voltage on the integrator: the chip's output.
    integrator: f64,
    /// Simulated voltage on the syllabic filter: the step size.
    syllabic: f64,
}

impl Default for Cvsd {
    fn default() -> Self {
        Self::new()
    }
}

impl Cvsd {
    pub fn new() -> Self {
        Self {
            shift_register: 0,
            integrator: 0.0,
            syllabic: 0.0,
        }
    }

    /// Back to the initial state, silent.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Feeds in one bit of the stream and returns the resulting sample,
    /// normalised to `-1.0..1.0`.
    pub fn clock(&mut self, bit: bool) -> f32 {
        // The current step comes from the level of the syllabic filter.
        let step = (1.0 - DECAY) * self.syllabic;
        if bit {
            self.integrator += step;
        } else {
            self.integrator -= step;
        }

        // Integrator leak: without this the signal drifts and goes out of range.
        self.integrator *= DECAY;

        self.shift_register = ((self.shift_register << 1) | u8::from(bit)) & SHIFT_MASK;

        // If the last three bits coincide, the decoder is lagging behind and
        // the step has to grow. If not, it is left to decay.
        let coincidence = self.shift_register == 0 || self.shift_register == SHIFT_MASK;
        self.syllabic *= CHARGE;
        self.syllabic += (1.0 - CHARGE) * if coincidence { V_HIGH } else { V_LOW };

        self.sample()
    }

    /// Current sample, normalised to `-1.0..1.0`.
    ///
    /// The integrator simulates a voltage that can reach `V_HIGH`, so that is
    /// what it is scaled by. It is clamped in case the filter overshoots.
    pub fn sample(&self) -> f32 {
        (self.integrator / V_HIGH).clamp(-1.0, 1.0) as f32
    }

    /// Raw integrator voltage, unnormalised.
    pub fn integrator(&self) -> f64 {
        self.integrator
    }

    /// Level of the syllabic filter: how big the step is right now.
    pub fn step_size(&self) -> f64 {
        self.syllabic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds in `n` identical bits and returns the last sample.
    fn clock_run(cvsd: &mut Cvsd, bit: bool, n: usize) -> f32 {
        let mut last = 0.0;
        for _ in 0..n {
            last = cvsd.clock(bit);
        }
        last
    }

    #[test]
    fn starts_silent() {
        let cvsd = Cvsd::new();
        assert_eq!(cvsd.sample(), 0.0);
        assert_eq!(cvsd.step_size(), 0.0);
    }

    #[test]
    fn a_run_of_ones_grows_the_step() {
        let mut cvsd = Cvsd::new();

        // The first bits have not filled the three-bit register yet.
        cvsd.clock(true);
        cvsd.clock(true);
        let initial_step = cvsd.step_size();

        clock_run(&mut cvsd, true, 50);
        assert!(
            cvsd.step_size() > initial_step * 5.0,
            "sustained coincidence has to grow the step \
             (it was {initial_step}, it ended at {})",
            cvsd.step_size()
        );
    }

    #[test]
    fn a_run_of_ones_drives_the_signal_up() {
        let mut cvsd = Cvsd::new();
        let sample = clock_run(&mut cvsd, true, 100);
        assert!(sample > 0.5, "it should be well up, it gave {sample}");
    }

    #[test]
    fn a_run_of_zeros_drives_the_signal_down() {
        let mut cvsd = Cvsd::new();
        let sample = clock_run(&mut cvsd, false, 100);
        assert!(sample < -0.5, "it should be well down, it gave {sample}");
    }

    #[test]
    fn alternating_bits_keep_the_step_small() {
        // Without coincidence the syllabic filter decays: this is the "I am
        // tracking the signal just fine, no need to floor it" case.
        let mut cvsd = Cvsd::new();
        for i in 0..200 {
            cvsd.clock(i % 2 == 0);
        }

        assert!(
            cvsd.step_size() < 0.01,
            "alternating there is no coincidence, the step decays (it ended at {})",
            cvsd.step_size()
        );
        assert!(
            cvsd.sample().abs() < 0.05,
            "and the signal stays close to zero (it gave {})",
            cvsd.sample()
        );
    }

    #[test]
    fn the_integrator_leaks_its_charge() {
        // After driving the signal up, alternating brings it back towards
        // zero: that is the leak of the estimator filter.
        let mut cvsd = Cvsd::new();
        let high = clock_run(&mut cvsd, true, 100);

        for i in 0..300 {
            cvsd.clock(i % 2 == 0);
        }

        assert!(
            cvsd.sample().abs() < high.abs() / 10.0,
            "the leak should have brought it back to zero (from {high} to {})",
            cvsd.sample()
        );
    }

    #[test]
    fn the_output_never_goes_out_of_range() {
        let mut cvsd = Cvsd::new();
        for _ in 0..10_000 {
            let sample = cvsd.clock(true);
            assert!(
                (-1.0..=1.0).contains(&sample),
                "it went out of range: {sample}"
            );
        }
    }

    #[test]
    fn reset_brings_back_the_silence() {
        let mut cvsd = Cvsd::new();
        clock_run(&mut cvsd, true, 100);
        assert!(cvsd.sample() > 0.5);

        cvsd.reset();
        assert_eq!(cvsd.sample(), 0.0);
        assert_eq!(cvsd.step_size(), 0.0);
    }
}
