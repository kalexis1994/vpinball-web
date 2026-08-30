//! The score displays: BCD in, seven segments out.
//!
//! A System 80 has no display processor. RIOT 1 puts a digit's *value* on four
//! bits of port B and a *position* on four bits of port A, and three latches —
//! strobed by three more bits of port A — hold one digit each for the three
//! banks of display. The board walks all sixteen positions many times a second
//! and the tube does the rest.
//!
//! So there is nothing to read back: what a digit shows is whatever was last
//! latched into it, and a host that wants a score reads [`Displays::digits`].
//!
//! # Why the segments and not the digits
//!
//! The value that reaches the latch is BCD, so the obvious thing to keep is
//! the number. It is the wrong thing: the machine writes `$0F` to blank a
//! digit and `$0A`–`$0E` for a handful of symbols, and a leading-zero blank is
//! most of what a score display does. Keeping segments keeps all of it, and a
//! caller that wants a number reads it back off them.

/// Seven segments, `a` in bit 0 through `g` in bit 6.
pub type Segments = u8;

/// Digits `0`–`9`, then the codes a System 80 blanks with.
///
/// `$0F` is the blank the ROM writes to suppress a leading zero, which is why
/// this table has to run to sixteen rather than ten.
const BCD_TO_SEGMENTS: [Segments; 16] = [
    0x3F, // 0
    0x06, // 1
    0x5B, // 2
    0x4F, // 3
    0x66, // 4
    0x6D, // 5
    0x7D, // 6
    0x07, // 7
    0x7F, // 8
    0x6F, // 9
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Where a digit select lands on the panel.
///
/// The board's sixteen positions are not the sixteen digits in order: the
/// displays are wired for the shortest run of copper rather than for the
/// convenience of whoever reads them later. This is the same permutation
/// PinMAME applies (`gts80.c`'s `reorder` in the BCD path), and without it a
/// score reads back with its digits shuffled.
const REORDER: [usize; 16] = [8, 0, 1, 15, 9, 10, 11, 12, 13, 14, 2, 3, 4, 5, 6, 7];

/// The three banks a System 80 drives.
///
/// # Why there are two copies of everything
///
/// The display is multiplexed: one position of sixteen is lit at a time, the
/// board blanks it before it moves on, and the eye does the rest. Keeping only
/// what is lit *now* keeps one digit and fifteen blanks, which is what the
/// tube would look like to a camera with a fast enough shutter and is not what
/// anybody means by a score.
///
/// So the segments are accumulated over a whole refresh and handed over as a
/// frame — `segments[pos].w |= …` and a `memset` every other vertical blank
/// (`gts80.c:269` and `:104`). What a host reads is the last complete sweep,
/// which is a score.
#[derive(Debug, Clone)]
pub struct Displays {
    /// Sixteen positions per bank, as segments: the last complete sweep.
    ///
    /// Bank 0 and bank 1 are the four player scores — six digits each on a
    /// System 80, seven on an 80A — and bank 2 is the ball-and-credit strip
    /// along the bottom.
    pub digits: [[Segments; 16]; 3],
    /// The sweep in progress.
    frame: [[Segments; 16]; 3],

    /// The three latches, as last loaded.
    latch: [Segments; 3],
    /// The last value seen on port A, for spotting the strobes' rising edges.
    last_a: u8,
}

impl Default for Displays {
    fn default() -> Self {
        Self::new()
    }
}

impl Displays {
    pub fn new() -> Self {
        Self {
            digits: [[0; 16]; 3],
            frame: [[0; 16]; 3],
            latch: [0; 3],
            last_a: 0,
        }
    }

    /// RIOT 1's port A moved: the digit select and the three latch strobes.
    ///
    /// Each latch loads on the **rising edge** of its own bit, taking whatever
    /// port B has on its low nibble at that moment. Then the digit the select
    /// names is set from all three latches at once, because all three banks
    /// share one select.
    pub fn port_a(&mut self, a: u8, b: u8) {
        let value = BCD_TO_SEGMENTS[usize::from(b & 0x0F)];
        for (i, strobe) in [0x10u8, 0x20, 0x40].into_iter().enumerate() {
            if a & strobe != 0 && self.last_a & strobe == 0 {
                self.latch[i] = value;
            }
        }
        self.last_a = a;

        let select = usize::from(a & 0x0F);
        let pos = REORDER[15 - select];
        self.frame[0][pos] |= self.latch[0];
        self.frame[1][pos] |= self.latch[1];
        // The status strip is not reordered: it is wired straight through, and
        // counted from the other end.
        self.frame[2][15 - select] |= self.latch[2];
    }

    /// Closes one sweep: what has been lit becomes what is shown.
    pub(crate) fn sweep(&mut self) {
        self.digits = std::mem::take(&mut self.frame);
    }

    /// Reads a bank back as the number it is showing, for whoever wants the
    /// score rather than the glass.
    ///
    /// Blank digits are skipped rather than read as zero, which is what makes
    /// a six-digit display showing `  1230` come back as 1230.
    /// One bank as text, sixteen characters, blanks for blanks.
    ///
    /// For a host that wants to *show* the score rather than know it — a
    /// diagnostic panel, a log line. A position showing something that is not
    /// a digit comes out as `?`, because on the glass it is something, and a
    /// space would say the display was dark when it is not.
    pub fn text(&self, bank: usize) -> String {
        self.digits[bank]
            .iter()
            .map(|s| match digit_of(*s) {
                Some(d) => char::from(b'0' + d),
                None if *s == 0 => ' ',
                None => '?',
            })
            .collect()
    }

    pub fn number(&self, bank: usize, from: usize, len: usize) -> u64 {
        let mut out = 0u64;
        for i in from..(from + len).min(16) {
            if let Some(d) = digit_of(self.digits[bank][i]) {
                out = out * 10 + u64::from(d);
            }
        }
        out
    }
}

/// The digit a set of segments is showing, or `None` for a blank.
fn digit_of(segments: Segments) -> Option<u8> {
    BCD_TO_SEGMENTS[..10]
        .iter()
        .position(|s| *s == segments)
        .map(|d| d as u8)
}
