//! Taking the DC out of a sound board's output.
//!
//! Both of a System 11's sound boards idle at a level that is not zero: the XS
//! board sits around -0.31 and the CS board at a flat -0.29, because their DACs
//! and their CVSD decoders idle at half scale and the mixing that follows does
//! not centre anything. On the real machine that does not matter, because the
//! board's output reaches the amplifier through a coupling capacitor and a
//! capacitor does not pass DC.
//!
//! Nothing here had one, and the offset is not harmless. It is inaudible on its
//! own — a constant is not a sound — but it eats a third of the headroom before
//! a single note is played, so the mix clips early; and every time the audio
//! starts, stops or is cut between buffers it steps from zero to a third of
//! full scale, which is a loud click.
//!
//! So: a one-pole high-pass, which is what a coupling capacitor is.

/// A one-pole DC blocker: `y[n] = x[n] - x[n-1] + r * y[n-1]`.
///
/// `r` sets the corner frequency. It has to be close enough to 1 that nothing
/// audible is touched — a pinball's lowest useful content is around 40 Hz — and
/// far enough from it that a real offset is gone within a fraction of a second.
#[derive(Debug, Clone)]
pub struct DcBlocker {
    r: f32,
    last_in: f32,
    last_out: f32,
}

/// Where the filter turns over, in hertz. Two octaves below anything a cabinet
/// speaker reproduces, so the only thing it removes is the offset.
pub const CORNER_HZ: f32 = 12.0;

impl DcBlocker {
    pub fn new(sample_rate: u32) -> Self {
        // The usual first-order mapping. At 48 kHz and 12 Hz this is 0.99843.
        let r = 1.0 - (std::f32::consts::TAU * CORNER_HZ / sample_rate.max(1) as f32);
        Self {
            r: r.clamp(0.0, 0.99999),
            last_in: 0.0,
            last_out: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.last_in = 0.0;
        self.last_out = 0.0;
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.last_in + self.r * self.last_out;
        self.last_in = x;
        self.last_out = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_fades_to_nothing() {
        // The whole point: the board's idle level must not reach the mix.
        let mut f = DcBlocker::new(48_000);
        let mut last = 0.0;
        for _ in 0..48_000 {
            last = f.process(-0.3);
        }
        assert!(last.abs() < 0.001, "the offset survived: {last}");
    }

    #[test]
    fn it_settles_fast_enough_to_not_be_heard_as_a_swell() {
        // A quarter of a second is the budget: longer and the start of a table
        // has an audible rise in it.
        let mut f = DcBlocker::new(48_000);
        let mut out = 0.0;
        for _ in 0..12_000 {
            out = f.process(-0.3);
        }
        assert!(out.abs() < 0.05, "still {out} after a quarter second");
    }

    #[test]
    fn it_leaves_the_signal_alone() {
        // A 440 Hz tone has to come through at its own level. A filter with the
        // corner in the wrong place would thin out everything a table plays.
        let mut f = DcBlocker::new(48_000);
        let mut peak: f32 = 0.0;
        for n in 0..48_000 {
            let x = (std::f32::consts::TAU * 440.0 * n as f32 / 48_000.0).sin();
            let y = f.process(x);
            if n > 4_800 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak > 0.99, "a 440 Hz tone came out at {peak}");
    }

    #[test]
    fn a_tone_riding_on_an_offset_keeps_its_shape() {
        let mut f = DcBlocker::new(48_000);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for n in 0..48_000 {
            let x = 0.5 * (std::f32::consts::TAU * 200.0 * n as f32 / 48_000.0).sin() - 0.3;
            let y = f.process(x);
            if n > 9_600 {
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
        // Centred on zero and half a unit either way, not offset by 0.3.
        assert!((hi + lo).abs() < 0.02, "not centred: {lo}..{hi}");
        assert!(
            (hi - lo - 1.0).abs() < 0.02,
            "amplitude changed: {lo}..{hi}"
        );
    }
}
