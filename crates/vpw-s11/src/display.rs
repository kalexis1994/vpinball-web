//! Williams System 11 alphanumeric displays.
//!
//! A System 11A —the F-14 Tomcat hardware— has two rows of 14 characters: the
//! top one 16-segment (alphanumeric) and the bottom one 7-segment (the score
//! displays). Both are multiplexed: the CPU picks a digit slot, writes its
//! segments, and repeats. At any given instant there is a single digit lit;
//! what one "sees" is the persistence of the display and of the eye.
//!
//! That is why a whole sweep's worth of segments is accumulated here and only
//! published afterwards. Without that, reading the display at any moment would
//! return a single character and thirteen spaces.
//!
//! # Wiring
//!
//! From the comments in the original driver (`s11.c:719-741`):
//!
//! - PIA 2, port A, bits 0-3: **digit select** (0-15).
//! - PIA 3, port B: segments `a b c d e f g` + comma of the top row.
//! - PIA 3, port A: segments `h j k m n p r` + dot of the top row.
//! - PIA 2, port B: the 7 segments of the bottom row.
//!
//! Of the 16 digit slots only 1-7 and 9-15 are shown (`DISP_SEG_7` in
//! `core.h:217` applied to `s11_dispS11a`): two groups of seven per row. Slots
//! 0 and 8 exist in the sweep but have no display behind them.

/// Digit slots of the multiplexing.
pub const DIGIT_SLOTS: usize = 16;

/// How many characters are actually visible per row.
pub const VISIBLE_CHARS: usize = 14;

/// The slots that have a physical character behind them on a System 11A: two
/// groups of seven, skipping 0 and 8.
const SLOTS_S11A: [usize; VISIBLE_CHARS] = [1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15];

/// The digit slots a machine actually shows, in the order a table numbers them.
///
/// The board multiplexes sixteen slots but a System 11A only puts characters in
/// fourteen of them, in two groups of seven with a gap; a System 11B fills all
/// sixteen. What a table sees is neither — it is the visible ones **packed**,
/// so the top row is digits 0 upwards with no gaps and the bottom row carries
/// straight on from there.
///
/// That packing is the original's, and it happens where it draws
/// (`core_seg_video_update`, `core.c:1496`): it walks the display layout entry
/// by entry, reads `coreGlobals.segments[layout->start + n]`, and writes to
/// `coreGlobals.drawSeg[segPos++]`. `segPos` never skips, so the gaps in the
/// hardware's numbering close up. `ChangedLEDs` then reports positions in
/// `drawSeg`, which is why F-14's script can say `Dim Digits(28)` and index
/// them nought to twenty-seven.
pub fn visible_slots(config: DisplayConfig) -> &'static [usize] {
    const ALL: [usize; DIGIT_SLOTS] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    if config.full_width { &ALL } else { &SLOTS_S11A }
}

/// How this machine's display is wired.
///
/// Not every System 11 builds it the same way, and the differences are not
/// cosmetic: reading an 11B with an 11A's configuration returns garbage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayConfig {
    /// The segment data arrives **inverted** (`S11_DISPINV` in the driver). A
    /// zero lights the segment.
    pub inverted: bool,
    /// The bottom row is 16-segment alphanumeric instead of 7-segment numeric
    /// (`S11_LOWALPHA`), and its high byte arrives through PIA 5's port A.
    pub alphanumeric_lower: bool,
    /// All 16 digit slots have a character, instead of two groups of seven
    /// with gaps.
    pub full_width: bool,
}

impl DisplayConfig {
    /// System 11 and 11A, like F-14 Tomcat: fourteen alphanumeric characters
    /// on top, fourteen seven-segment digits below, data the right way up.
    pub const S11A: Self = Self {
        inverted: false,
        alphanumeric_lower: false,
        full_width: false,
    };

    /// System 11B and 11C, like Taxi or Diner: both rows alphanumeric, sixteen
    /// characters, and the data inverted.
    pub const S11B: Self = Self {
        inverted: true,
        alphanumeric_lower: true,
        full_width: true,
    };
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self::S11A
    }
}

/// Segment bits, exactly as the board wires them.
///
/// ```text
///       -- a --
///      |\  |  /|
///      f h j k b
///      |  \|/  |
///       -g- -m-
///      |  /|\  |
///      e r p n c
///      |/  |  \|
///       -- d --
/// ```
pub mod seg {
    /// Top bar.
    pub const A: u16 = 1 << 0;
    /// Upper right vertical.
    pub const B: u16 = 1 << 1;
    /// Lower right vertical.
    pub const C: u16 = 1 << 2;
    /// Bottom bar.
    pub const D: u16 = 1 << 3;
    /// Lower left vertical.
    pub const E: u16 = 1 << 4;
    /// Upper left vertical.
    pub const F: u16 = 1 << 5;
    /// Left half of the middle bar.
    pub const G: u16 = 1 << 6;
    /// Comma.
    pub const COMMA: u16 = 1 << 7;
    /// Upper left diagonal.
    pub const H: u16 = 1 << 8;
    /// Upper centre vertical.
    pub const J: u16 = 1 << 9;
    /// Upper right diagonal.
    pub const K: u16 = 1 << 10;
    /// Right half of the middle bar.
    pub const M: u16 = 1 << 11;
    /// Lower right diagonal (centre -> lower right corner).
    pub const N: u16 = 1 << 12;
    /// Lower centre vertical.
    pub const P: u16 = 1 << 13;
    /// Lower left diagonal (centre -> lower left corner).
    pub const R: u16 = 1 << 14;
    /// Decimal point.
    pub const DOT: u16 = 1 << 15;

    /// The two punctuation bits, which are not part of the glyph.
    pub const PUNCTUATION: u16 = COMMA | DOT;
}

use seg::*;

/// 14-segment font.
///
/// The glyphs that show up in F-14's boot messages are verified against the
/// patterns the real ROM draws; the rest follow the standard convention for
/// these displays. If a `?` shows up in a message, the most likely cause is a
/// missing fix to an entry in this table, not a broken emulation: it is worth
/// looking at the raw segments with
/// `cargo run -p vpw-s11 --example boot -- rom.zip 10 segments`.
#[rustfmt::skip]
const FONT: &[(char, u16)] = &[
    ('0', A | B | C | D | E | F),
    ('1', B | C),
    ('2', A | B | G | M | E | D),
    ('3', A | B | C | D | G | M),
    ('4', F | G | M | B | C),
    ('5', A | F | G | M | C | D),
    ('6', A | F | E | D | C | G | M),
    ('7', A | B | C),
    ('8', A | B | C | D | E | F | G | M),
    ('9', A | B | C | D | F | G | M),

    ('A', A | B | C | E | F | G | M),
    ('B', A | B | C | D | J | P | M),
    ('C', A | D | E | F),
    ('D', A | B | C | D | J | P),
    ('E', A | D | E | F | G),
    ('F', A | E | F | G),
    ('G', A | C | D | E | F | M),
    ('H', B | C | E | F | G | M),
    ('I', A | D | J | P),
    ('J', B | C | D | E),
    ('K', E | F | G | K | N),
    ('L', D | E | F),
    ('M', B | C | E | F | H | K),
    ('N', B | C | E | F | H | N),
    ('P', A | B | E | F | G | M),
    ('Q', A | B | C | D | E | F | N),
    ('R', A | B | E | F | G | M | N),
    ('T', A | J | P),
    ('U', B | C | D | E | F),
    ('V', E | F | K | R),
    ('W', B | C | E | F | N | R),
    ('X', H | K | N | R),
    ('Y', H | K | P),
    ('Z', A | D | K | R),

    ('-', G | M),
    ('_', D),
    ('=', D | G | M),
    ('+', G | M | J | P),
    ('*', H | J | K | G | M | N | P | R),
    ('/', K | N),
    ('\\', H | R),
    ('|', J | P),
    ('\'', J),
    ('"', F | J),
];

/// Translates a segment pattern into the character it draws.
///
/// The comma and the dot are ignored: they are punctuation, not part of the
/// glyph. A pattern that does not correspond to any known character returns
/// `'?'`; lots of those usually means the sweep was read at the wrong moment.
///
/// # Indistinguishable glyphs
///
/// In 14 segments there are characters that are **the same drawing**, so the
/// information needed to tell them apart simply does not exist:
///
/// | Light the same thing | Decoded as |
/// |----------------------|------------|
/// | `0` and `O`          | `0`        |
/// | `5` and `S`          | `5`        |
/// | `/`, `(` and `<`     | `/`        |
/// | `\`, `)` and `>`     | `\`        |
///
/// The digit is chosen because the display spends its life showing scores. A
/// `GAME OVER` reads `GAME 0VER` and a `SCORE` reads `5CORE`: that is not a
/// decoding error, it is what the display physically shows.
pub fn segments_to_char(pattern: u16) -> char {
    let glyph = pattern & !PUNCTUATION;
    if glyph == 0 {
        return ' ';
    }
    // The bottom display is 7-segment: if the three centre segments of an `8`
    // lit up without diagonals, the exact lookup still resolves it.
    FONT.iter()
        .find(|&&(_, bits)| bits == glyph)
        .map_or('?', |&(c, _)| c)
}

/// 7-segment font, for the bottom row.
///
/// Reusing the 14-segment one is not enough: on a 7-segment display the middle
/// bar is **a single one** (the `g` segment), while on the 14-segment one it is
/// split into two halves (`g` and `m`). A 7-segment `2` lights `a b g e d`, and
/// that same `2` on the top display also lights `m`. Decoding the bottom row
/// with the 14-segment table left everything as `?` except `0` and `1`, which
/// are the only ones that do not use the middle bar.
#[rustfmt::skip]
const FONT_7SEG: &[(char, u16)] = &[
    ('0', A | B | C | D | E | F),
    ('1', B | C),
    ('2', A | B | G | E | D),
    ('3', A | B | C | D | G),
    ('4', F | G | B | C),
    ('5', A | F | G | C | D),
    ('6', A | F | E | D | C | G),
    ('7', A | B | C),
    ('8', A | B | C | D | E | F | G),
    ('9', A | B | C | D | F | G),

    ('A', A | B | C | E | F | G),
    ('b', C | D | E | F | G),
    ('C', A | D | E | F),
    ('d', B | C | D | E | G),
    ('E', A | D | E | F | G),
    ('F', A | E | F | G),
    ('H', B | C | E | F | G),
    ('G', A | C | D | E | F),
    ('J', B | C | D | E),
    ('L', D | E | F),
    ('N', A | B | C | E | F),
    ('P', A | B | E | F | G),
    ('U', B | C | D | E | F),
    ('n', C | E | G),
    ('o', C | D | E | G),
    ('r', E | G),
    ('t', D | E | F | G),
    ('-', G),
    ('_', D),
];

/// Translates a pattern from the bottom row, which is 7-segment.
pub fn segments_to_char_7seg(pattern: u16) -> char {
    let glyph = pattern & !PUNCTUATION;
    if glyph == 0 {
        return ' ';
    }
    FONT_7SEG
        .iter()
        .find(|&&(_, bits)| bits == glyph)
        .map_or('?', |&(c, _)| c)
}

/// The two displays on the backbox.
pub struct Displays {
    /// Digit slot selected by the multiplexing (0-15).
    digit_select: u8,

    /// Accumulated segments of the sweep in progress.
    upper_accum: [u16; DIGIT_SLOTS],
    lower_accum: [u16; DIGIT_SLOTS],
    /// Last complete sweep: what is visible.
    upper: [u16; DIGIT_SLOTS],
    lower: [u16; DIGIT_SLOTS],

    /// State of the anti-flicker filter, one per half of the top row.
    flicker_high: FlickerFilter,
    flicker_low: FlickerFilter,

    /// How this machine's display is wired.
    config: DisplayConfig,
}

/// Filters out the "all segments" blip that some ROMs do.
///
/// To show an error message without the normal routine, several System 11 ROMs
/// write the correct segments, burn cycles in an empty loop, turn **every**
/// segment of the digit on and turn them off two instructions later
/// (`s11.c:444-461`). On the real machine the inertia of the display and of the
/// eye hide it; in an emulation that accumulates by OR, that `0xFF` would cover
/// up the character.
///
/// The rule: a `0xFF` is never latched at the moment it arrives. If a `0x00`
/// comes afterwards and more than 16 cycles went by in between, it was a
/// genuine lighting (the display test) and only then is the `0xFF` latched.
#[derive(Debug, Clone, Copy, Default)]
struct FlickerFilter {
    previous: u8,
    previous_cycle: u64,
}

impl FlickerFilter {
    /// Returns the value that has to be latched, or `None` if it must be
    /// ignored.
    fn filter(&mut self, data: u8, cycle: u64) -> Option<u8> {
        let latch = if data == 0xFF {
            None
        } else if data == 0x00 && self.previous == 0xFF && cycle - self.previous_cycle > 16 {
            Some(0xFF)
        } else {
            Some(data)
        };

        self.previous = data;
        self.previous_cycle = cycle;
        latch
    }
}

impl Default for Displays {
    fn default() -> Self {
        Self::new()
    }
}

impl Displays {
    pub fn new() -> Self {
        Self {
            digit_select: 0,
            upper_accum: [0; DIGIT_SLOTS],
            lower_accum: [0; DIGIT_SLOTS],
            upper: [0; DIGIT_SLOTS],
            lower: [0; DIGIT_SLOTS],
            flicker_high: FlickerFilter::default(),
            flicker_low: FlickerFilter::default(),
            config: DisplayConfig::S11A,
        }
    }

    /// Changes the display wiring. Emptying what was accumulated avoids mixing
    /// data read with one configuration and with the other.
    pub fn set_config(&mut self, config: DisplayConfig) {
        self.config = config;
        self.upper_accum = [0; DIGIT_SLOTS];
        self.lower_accum = [0; DIGIT_SLOTS];
        self.upper = [0; DIGIT_SLOTS];
        self.lower = [0; DIGIT_SLOTS];
    }

    pub fn config(&self) -> DisplayConfig {
        self.config
    }

    /// Applies the data inversion if this machine uses it.
    #[inline]
    fn normalize(&self, data: u8) -> u8 {
        if self.config.inverted { !data } else { data }
    }

    /// Digit slot the multiplexing currently has selected.
    pub fn digit_select(&self) -> u8 {
        self.digit_select
    }

    /// PIA 2, port A: the four low bits pick the digit.
    pub fn select_digit(&mut self, port_a: u8) {
        self.digit_select = port_a & 0x0F;
    }

    /// PIA 3, port B: segments `a`-`g` and comma of the top row.
    pub fn write_upper_low(&mut self, data: u8, cycle: u64) {
        let data = self.normalize(data);
        if let Some(value) = self.flicker_low.filter(data, cycle) {
            self.upper_accum[self.digit_select as usize] |= u16::from(value);
        }
    }

    /// PIA 3, port A: segments `h`-`r` and dot of the top row.
    pub fn write_upper_high(&mut self, data: u8, cycle: u64) {
        let data = self.normalize(data);
        if let Some(value) = self.flicker_high.filter(data, cycle) {
            self.upper_accum[self.digit_select as usize] |= u16::from(value) << 8;
        }
    }

    /// PIA 2, port B: the low byte of the bottom row.
    pub fn write_lower(&mut self, data: u8) {
        let data = self.normalize(data);
        self.lower_accum[self.digit_select as usize] |= u16::from(data);
    }

    /// PIA 5, port A: the **high** byte of the bottom row.
    ///
    /// Only the machines with an alphanumeric bottom row use it; on the rest
    /// that port does something else and it has to be ignored.
    pub fn write_lower_high(&mut self, data: u8) {
        if !self.config.alphanumeric_lower {
            return;
        }
        let data = self.normalize(data);
        self.lower_accum[self.digit_select as usize] |= u16::from(data) << 8;
    }

    /// Closes the sweep: what was accumulated becomes what is visible.
    pub fn refresh(&mut self) {
        self.upper = self.upper_accum;
        self.lower = self.lower_accum;
        self.upper_accum = [0; DIGIT_SLOTS];
        self.lower_accum = [0; DIGIT_SLOTS];
    }

    /// Raw segments of the top row, by digit slot.
    pub fn upper_segments(&self) -> &[u16; DIGIT_SLOTS] {
        &self.upper
    }

    pub fn lower_segments(&self) -> &[u16; DIGIT_SLOTS] {
        &self.lower
    }

    /// The top row as text.
    ///
    /// It is 14 characters on a System 11A and 16 on an 11B.
    pub fn upper_text(&self) -> String {
        self.row_text(&self.upper, segments_to_char)
    }

    /// The bottom row as text.
    pub fn lower_text(&self) -> String {
        let decode = if self.config.alphanumeric_lower {
            segments_to_char
        } else {
            segments_to_char_7seg
        };
        self.row_text(&self.lower, decode)
    }

    fn row_text(&self, row: &[u16; DIGIT_SLOTS], decode: fn(u16) -> char) -> String {
        visible_slots(self.config)
            .iter()
            .map(|&slot| decode(row[slot]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digits_decode() {
        for (expected, bits) in FONT.iter().take(10) {
            assert_eq!(segments_to_char(*bits), *expected);
        }
    }

    #[test]
    fn punctuation_does_not_change_the_glyph() {
        let five = FONT.iter().find(|(c, _)| *c == '5').unwrap().1;
        assert_eq!(segments_to_char(five), '5');
        assert_eq!(segments_to_char(five | COMMA), '5');
        assert_eq!(segments_to_char(five | DOT), '5');
    }

    #[test]
    fn no_segments_is_a_space() {
        assert_eq!(segments_to_char(0), ' ');
        assert_eq!(
            segments_to_char(COMMA),
            ' ',
            "just a comma is still a space"
        );
    }

    #[test]
    fn an_unknown_pattern_does_not_lie() {
        // Every segment lit is not any character.
        assert_eq!(segments_to_char(0x7FFF), '?');
    }

    #[test]
    fn the_font_has_no_repeated_patterns() {
        // Two characters with the same pattern would make the decoding
        // ambiguous and dependent on the order of the table.
        for (i, (char_a, bits_a)) in FONT.iter().enumerate() {
            for (char_b, bits_b) in FONT.iter().skip(i + 1) {
                if bits_a == bits_b {
                    // A few pairs are legitimately identical on a 14-segment
                    // display; the rest would be a mistake in the table.
                    let allowed = matches!(
                        (char_a, char_b),
                        ('0', 'O')
                            | ('/', '(')
                            | ('/', '<')
                            | ('\\', ')')
                            | ('\\', '>')
                            | ('(', '<')
                            | (')', '>')
                    );
                    assert!(allowed, "{char_a} and {char_b} share a pattern");
                }
            }
        }
    }

    #[test]
    fn the_filter_ignores_the_all_segments_blip() {
        let mut f = FlickerFilter::default();

        assert_eq!(
            f.filter(0x3F, 100),
            Some(0x3F),
            "a normal character is latched"
        );
        assert_eq!(
            f.filter(0xFF, 102),
            None,
            "the 0xFF is not latched at the time"
        );
        // Two instructions later: it was flicker, not a real lighting.
        assert_eq!(f.filter(0x00, 104), Some(0x00));
    }

    #[test]
    fn the_filter_lets_the_display_test_through() {
        let mut f = FlickerFilter::default();

        assert_eq!(f.filter(0xFF, 1000), None);
        // More than 16 cycles went by: the lighting was genuine.
        assert_eq!(f.filter(0x00, 1100), Some(0xFF));
    }

    #[test]
    fn the_bottom_row_uses_the_7_segment_table() {
        // The 7-segment `2` does not carry the right half of the middle bar,
        // so the 14-segment table does not recognise it.
        let two_7seg = A | B | G | E | D;
        assert_eq!(segments_to_char_7seg(two_7seg), '2');
        assert_eq!(
            segments_to_char(two_7seg),
            '?',
            "it does not match the 14-segment table"
        );

        // `0` and `1` are the same in both: they do not use the middle bar.
        for bits in [A | B | C | D | E | F, B | C] {
            assert_eq!(segments_to_char(bits), segments_to_char_7seg(bits));
        }
    }

    #[test]
    fn the_7_segment_font_does_not_repeat_patterns_either() {
        for (i, (char_a, bits_a)) in FONT_7SEG.iter().enumerate() {
            for (char_b, bits_b) in FONT_7SEG.iter().skip(i + 1) {
                assert_ne!(bits_a, bits_b, "{char_a} and {char_b} share a pattern");
            }
        }
    }

    #[test]
    fn the_sweep_accumulates_and_is_published() {
        let mut d = Displays::new();

        // A "5" in slot 1 and a "0" in slot 2. The "5" needs segment M, which
        // travels on the high port: that is two writes.
        let five = A | F | G | M | C | D;
        d.select_digit(1);
        d.write_upper_low(five as u8, 10);
        d.write_upper_high((five >> 8) as u8, 12);
        d.select_digit(2);
        d.write_upper_low((A | B | C | D | E | F) as u8, 20);

        assert_eq!(
            d.upper_text(),
            " ".repeat(14),
            "nothing is visible until the refresh"
        );

        d.refresh();
        assert_eq!(d.upper_text(), "50            ");

        // The next sweep starts clean.
        d.refresh();
        assert_eq!(d.upper_text(), " ".repeat(14));
    }
}
