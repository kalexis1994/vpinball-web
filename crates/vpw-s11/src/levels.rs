//! How loud each source is against the others.
//!
//! Not a taste: the numbers are the machine driver's, and F-14 runs the `s11xs`
//! configuration — a main board with a sound section and a separate music and
//! speech board, both alive at once (`s11games.c:376-378`, which loads both
//! `S11XS_SOUNDROM88` and `S11CS_SOUNDROM88`).
//!
//! `wmssnd_s11xs` (`wmssnd.c:494`) starts from `wmssnd_s11cs` and then replaces
//! two of its devices:
//!
//! ```c
//! static struct YM2151interface s11cs_ym2151Int = {          // wmssnd.c:731
//!     1, 3579545, { YM3012_VOL(10, MIXER_PAN_CENTER, 10, MIXER_PAN_CENTER) }, ...
//! };
//! static struct DACinterface      s11xs_dacInt2     = { 2, { 25, 25 }};   // :479
//! static struct hc55516_interface s11xs_hc55516Int2 = { 2, { 100, 100 }, ... }; // :481
//! ```
//!
//! Two DACs at 25 and two speech channels at 100 — one of each per board — plus
//! the FM at 10. Five channels into **one** mixer, where the level enters
//! linearly: `(volume * mixing_level) / (100*100)` (`mixer.c:607`).
//!
//! # Why they cannot be averaged in pairs
//!
//! Because averaging is a weighting too, and a different one. Mixing each board
//! down on its own and then averaging the two gives each board half the total
//! no matter what is inside it, so a board with three sources gives each of them
//! a third of its half. The port did exactly that, and against the original it
//! came out with the effects **+8 dB** on the main board and **+3 dB** on the
//! sound board, the FM **+5 dB**, and the speech —the one channel the machine
//! actually leans on— **3 dB down** on both. Loud where it should sit back and
//! quiet where it should carry.

/// The sound effects, both boards (`s11xs_dacInt2`, `wmssnd.c:479`).
pub const DAC: f32 = 25.0;

/// The speech, both boards (`s11xs_hc55516Int2`, `wmssnd.c:481`).
pub const CVSD: f32 = 100.0;

/// The music (`s11cs_ym2151Int`, `wmssnd.c:731`).
pub const YM2151: f32 = 10.0;

/// Sums sources already multiplied by their levels, and brings the result back
/// into range.
///
/// The original does not do this last part: it scales each channel by its level
/// over ten thousand, sums, and clips whatever sticks out (`mixer.c:607`). That
/// works there because the levels are set against a known headroom. Here the
/// sum is divided by the levels that went into it, so the balance is the
/// original's and the output cannot clip on its own — which is what a browser
/// needs, since there is nothing downstream to catch it.
pub fn weighted(sources: &[(f32, f32)]) -> f32 {
    let total: f32 = sources.iter().map(|(level, _)| level).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let sum: f32 = sources.iter().map(|(level, s)| level * s).sum();
    (sum / total).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speech_carries_four_times_what_an_effect_does() {
        // The one ratio the whole thing turns on, and the reason the port's
        // pairwise averaging was audible: 100 against 25.
        assert_eq!(CVSD / DAC, 4.0);
        assert_eq!(CVSD / YM2151, 10.0);
    }

    #[test]
    fn a_single_source_comes_out_untouched() {
        // Whatever the level is. A board on its own is not quieter for having
        // been given a small number, it is only quieter next to the others.
        assert_eq!(weighted(&[(DAC, 0.5)]), 0.5);
        assert_eq!(weighted(&[(YM2151, -0.25)]), -0.25);
        assert_eq!(weighted(&[]), 0.0);
    }

    #[test]
    fn a_loud_channel_and_a_quiet_one_land_where_the_levels_say() {
        // Speech at full scale against a silent effect: 100/(100+25).
        let m = weighted(&[(CVSD, 1.0), (DAC, 0.0)]);
        assert!((m - 0.8).abs() < 1e-6, "{m}");
    }
}
