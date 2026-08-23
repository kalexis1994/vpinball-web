//! Two-pole filter sections, and the analogue stages the sound boards built
//! out of them.
//!
//! A CVSD decoder puts out a staircase: it moves by one step per clock and the
//! steps are square, so what comes off it carries a great deal of energy well
//! above anything a voice contains. On the real board that never reaches the
//! cabinet — the chip's output goes through two op-amp stages before the
//! amplifier, and those are what make the speech sound like speech instead of
//! like a buzz with words in it. PinMAME says so in as many words while
//! explaining an unrelated timing bug: the harshness was "part of the reason
//! that the HC55516 output sounded so terrible" (`wmssnd.c:242`).
//!
//! Ported from PinMAME's `src/sound/filter.c`, BSD-3-Clause and so compatible
//! with this project's GPLv3.
//!
//! # Why the coefficients come from resistors
//!
//! Because the filter is a circuit, and the circuit is on the schematic. Each
//! `*_setup` below takes the actual component values and turns the analogue
//! transfer function into a digital one, so the cutoff is not a number someone
//! liked the sound of — it is where the board puts it.

/// A biquad in direct form 1, carried in double precision.
///
/// Form 1 and not form 2 because it is what the reference implementation uses
/// (`filter.h:63`, which says as much and keeps four history registers rather
/// than two on purpose).
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
    a1: f64,
    a2: f64,
    b0: f64,
    b1: f64,
    b2: f64,
}

impl Biquad {
    /// Builds the section from the canonical continuous-time form
    ///
    /// ```text
    ///              1
    /// H(s) = -----------------
    ///        1 + c1*s + c2*s^2
    /// ```
    ///
    /// by the bilinear transform, pre-warped so the response at `f0` is the
    /// analogue one (`filter.c:383`).
    ///
    /// The DC gain is dropped on the way: every analogue stage here inverts and
    /// has a gain of `-R3/R1`, and carrying that through would leave the level
    /// to the schematic when the level belongs to the mixer. The reference does
    /// the same and says why (`filter.c:481`).
    pub fn from_canonical(f0: f64, c1: f64, c2: f64, sample_rate: f64) -> Self {
        let mut w0 = f0 * (2.0 * std::f64::consts::PI);

        // Keep it inside -pi/T..pi/T. Above that the bilinear transform has
        // nowhere to put the frequency, and these stages sit near the limit
        // whenever the host runs at a rate the board's designers never saw.
        let wd_max = std::f64::consts::PI * sample_rate;
        if w0 > wd_max {
            w0 -= 2.0 * wd_max;
        }

        // The time-step factor, pre-warped to f0.
        let gn = w0 / (w0 / (2.0 * sample_rate)).tan();

        let cc = 1.0 + gn * c1 + gn * gn * c2;
        Self {
            b0: 1.0 / cc,
            b1: 2.0 / cc,
            b2: 1.0 / cc,
            a1: 2.0 * (1.0 - gn * gn * c2) / cc,
            a2: (1.0 - gn * c1 + gn * gn * c2) / cc,
            ..Self::default()
        }
    }

    /// One sample through (`filter.h:89`).
    pub fn step(&mut self, x0: f64) -> f64 {
        let y0 = (self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2)
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }

    /// Forgets its history, leaving the coefficients alone.
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Single-pole active low pass, from its components (`filter.c:511`).
///
/// ```text
///            -1/(R1*R2*C1)
/// H(s) = ---------------------------------------
///        (1/R1 + 1/R2 + 1/R3)*s + 1/(R2*R3*C1)
/// ```
pub fn active_lp(r1: f64, r2: f64, r3: f64, c1: f64, sample_rate: f64) -> Biquad {
    let cc1 = r2 * r3 * c1 * (1.0 / r1 + 1.0 / r2 + 1.0 / r3);
    let fc = 1.0 / ((2.0 * std::f64::consts::PI) * cc1);
    Biquad::from_canonical(fc, cc1, 0.0, sample_rate)
}

/// Multiple-feedback active low pass, from its components (`filter.c:464`).
///
/// ```text
///                    -1/(C1*C2*R1*R2)
/// H(s) = ------------------------------------------------------
///        s^2 + (1/C1)*(1/R1 + 1/R2 + 1/R3)*s + 1/(C1*C2*R2*R3)
/// ```
pub fn mf_lp(r1: f64, r2: f64, r3: f64, c1: f64, c2: f64, sample_rate: f64) -> Biquad {
    let cc1 = c2 * r2 * r3 * (1.0 / r1 + 1.0 / r2 + 1.0 / r3);
    let cc2 = c1 * c2 * r2 * r3;
    let fc = 1.0 / ((2.0 * std::f64::consts::PI) * cc2.sqrt());
    Biquad::from_canonical(fc, cc1, cc2, sample_rate)
}

/// Which sound board's output stages to use (`hc55516.h:28-30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    /// Williams speech board type 2, the C-8228: two multiple-feedback stages.
    /// Systems 3, 6, 7 and 9.
    C8228,
    /// Williams System 11: a single-pole active stage, then a
    /// multiple-feedback one.
    System11,
    /// Pre-DCS WPC, from the WPC89 schematics. The default in the reference.
    Wpc89,
}

/// The two stages a speech chip's output goes through on its way out of the
/// cabinet.
///
/// Which two depends on the board, and that is not something to guess at: the
/// machine driver names the board and the board names the circuit
/// (`hc55516.c:547`).
#[derive(Debug, Clone, Copy)]
pub struct OutputFilter {
    first: Biquad,
    second: Biquad,
}

impl OutputFilter {
    /// Builds the pair for a board at a given output rate (`hc55516.c:556`).
    pub fn new(board: Board, sample_rate: f64) -> Self {
        let (first, second) = match board {
            Board::C8228 => (
                mf_lp(43000.0, 36000.0, 180000.0, 1800e-12, 180e-12, sample_rate),
                mf_lp(27000.0, 15000.0, 27000.0, 4700e-12, 1200e-12, sample_rate),
            ),
            Board::System11 => (
                active_lp(43000.0, 36000.0, 180000.0, 180e-12, sample_rate),
                mf_lp(47000.0, 22000.0, 15000.0, 1800e-12, 330e-12, sample_rate),
            ),
            Board::Wpc89 => (
                mf_lp(150000.0, 22000.0, 150000.0, 1800e-12, 330e-12, sample_rate),
                mf_lp(47000.0, 22000.0, 150000.0, 1800e-12, 330e-12, sample_rate),
            ),
        };
        Self { first, second }
    }

    /// One sample through both stages (`hc55516.c:899`).
    pub fn step(&mut self, sample: f32) -> f32 {
        self.second.step(self.first.step(f64::from(sample))) as f32
    }

    pub fn reset(&mut self) {
        self.first.reset();
        self.second.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// How much of a sine at `hz` survives, as a fraction of its amplitude.
    fn response(board: Board, hz: f64) -> f32 {
        let mut f = OutputFilter::new(board, RATE);
        let mut peak = 0.0f32;
        let n = (RATE * 0.2) as usize;
        for i in 0..n {
            let t = i as f64 / RATE;
            let out = f.step((t * hz * 2.0 * std::f64::consts::PI).sin() as f32);
            // Only the second half, once the filter has settled.
            if i > n / 2 {
                peak = peak.max(out.abs());
            }
        }
        peak
    }

    #[test]
    fn a_voice_goes_through_and_the_hash_above_it_does_not() {
        // The point of the thing. A CVSD decoder puts out a staircase clocked
        // at tens of kilohertz, and without these two stages all of that
        // reaches the speaker along with the words.
        let voice = response(Board::System11, 1000.0);
        let hash = response(Board::System11, 20_000.0);
        assert!(voice > 0.9, "a voice should come through: {voice:.3}");
        assert!(hash < voice / 10.0, "and the hash should not: {hash:.3}");
    }

    #[test]
    fn direct_current_comes_out_at_unit_gain() {
        // The analogue stages invert and have a gain of -R3/R1; the digital
        // ones drop it, because the level belongs to the mixer and not to the
        // schematic (`filter.c:481`). So a constant has to come out as itself,
        // right way up.
        for board in [Board::C8228, Board::System11, Board::Wpc89] {
            let mut f = OutputFilter::new(board, RATE);
            let mut out = 0.0;
            for _ in 0..4000 {
                out = f.step(1.0);
            }
            assert!(
                (out - 1.0).abs() < 1e-3,
                "{board:?} settled at {out} instead of 1"
            );
        }
    }

    #[test]
    fn the_boards_do_not_all_filter_the_same() {
        // Otherwise there would be no reason for the machine driver to name
        // one, and picking the wrong one would cost nothing.
        let s11 = response(Board::System11, 10_000.0);
        let c8228 = response(Board::C8228, 10_000.0);
        assert!(
            (s11 - c8228).abs() > 0.05,
            "System 11 {s11:.3} against C-8228 {c8228:.3}"
        );
    }

    #[test]
    fn resetting_forgets_the_signal_and_keeps_the_filter() {
        let mut f = OutputFilter::new(Board::System11, RATE);
        for _ in 0..100 {
            f.step(1.0);
        }
        f.reset();
        assert_eq!(f.step(0.0), 0.0, "nothing left over");

        let mut fresh = OutputFilter::new(Board::System11, RATE);
        fresh.step(0.0);
        for _ in 0..50 {
            assert!(
                (f.step(1.0) - fresh.step(1.0)).abs() < 1e-12,
                "and it still filters the same as a new one"
            );
        }
    }
}
