//! The alphanumeric displays of System 80B, and the two processors that draw
//! them.
//!
//! A System 80 or 80A shows its score on BCD latches: four bits in, one digit
//! out, and nothing it can spell (see [`crate::display`]). The 80B replaced
//! that with **two rows of twenty sixteen-segment characters**, and put a
//! **Rockwell 10939** display processor in front of each row so the game
//! processor no longer has to walk the digits itself. Write a character and it
//! lands in the next column; write a control word and the chip changes how it
//! is drawing.
//!
//! That is why an 80B can say `BOUNTY HUNTER` where a Black Hole can only
//! count.
//!
//! # How the game board talks to them
//!
//! Awkwardly, because RIOT 1 has eight bits of port A and eight of port B and
//! the job needs more. A byte reaches the processors a nibble at a time:
//!
//! - the low nibble of port B is the data;
//! - the **falling** edge of port A bit 4 latches it as the low nibble;
//! - the **rising** edge of port A bit 5 latches it as the high nibble;
//! - and then a rising edge on port B bit 4 or bit 5 hands the assembled byte
//!   to the first or the second processor.
//!
//! # About where this comes from
//!
//! The wiring is Gottlieb's, read off `gts80.c` — a description of a circuit
//! board rather than a program. The chip's own command set is Rockwell's, and
//! is written here from its description rather than ported: `gts80.c` carries
//! no BSD licence line, unlike the speech chip next door in [`crate::votrax`].

/// How many characters one processor drives, and so how many are in a row.
pub const COLUMNS: usize = 20;

/// Where the segments of a sixteen-segment character sit in a word.
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
///
/// The order is VPinMAME's, which is the order a table's own segment lights
/// are numbered in — the same one [`vpw_s11::display::seg`] uses, and a test
/// keeps the two honest.
pub mod seg {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const C: u16 = 1 << 2;
    pub const D: u16 = 1 << 3;
    pub const E: u16 = 1 << 4;
    pub const F: u16 = 1 << 5;
    /// Left half of the middle bar.
    pub const G: u16 = 1 << 6;
    pub const COMMA: u16 = 1 << 7;
    /// Upper left diagonal.
    pub const H: u16 = 1 << 8;
    /// Upper centre vertical.
    pub const J: u16 = 1 << 9;
    /// Upper right diagonal.
    pub const K: u16 = 1 << 10;
    /// Right half of the middle bar.
    pub const M: u16 = 1 << 11;
    /// Lower right diagonal.
    pub const N: u16 = 1 << 12;
    /// Lower centre vertical.
    pub const P: u16 = 1 << 13;
    /// Lower left diagonal.
    pub const R: u16 = 1 << 14;
    pub const DOT: u16 = 1 << 15;
}

use seg::*;

/// The sixteen-segment font.
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
    ('O', A | B | C | D | E | F),
    ('P', A | B | E | F | G | M),
    ('Q', A | B | C | D | E | F | N),
    ('R', A | B | E | F | G | M | N),
    ('S', A | F | G | M | C | D),
    ('T', A | J | P),
    ('U', B | C | D | E | F),
    ('V', E | F | K | R),
    ('W', B | C | E | F | N | R),
    ('X', H | K | N | R),
    ('Y', H | K | P),
    ('Z', A | D | K | R),

    (' ', 0),
    ('-', G | M),
    ('_', D),
    ('=', D | G | M),
    ('+', G | J | M | P),
    ('*', G | H | J | K | M | N | P | R),
    ('/', K | N),
    ('\\', H | R),
    ('|', J | P),
    ('!', B | C | DOT),
    ('\'', J),
    ('"', F | J),
    ('(', K | N),
    (')', H | R),
    ('[', A | D | E | F),
    (']', A | B | C | D),
    ('<', K | N),
    ('>', H | R),
    ('?', A | B | M | P),
    ('@', A | B | C | D | E | F | J | M),
    (':', J | P),
    (';', J | P | COMMA),
    (',', COMMA),
    ('.', DOT),
    ('#', B | C | G | J | M | P),
    ('$', A | C | D | F | G | J | M | P),
    ('%', F | H | K | M | N | G | R | C),
    ('&', A | D | E | F | G | H | M | N),
    ('^', H | K),
];

/// The character a code draws, and the segments that draw it.
///
/// The 10939 has its own character set rather than plain ASCII, because the
/// display can show punctuation *attached to* a digit — a comma or a decimal
/// point inside the same character cell, which is what a score with thousands
/// separators needs:
///
/// | Codes | What |
/// |-------|------|
/// | `$00`, `$01` | blank |
/// | `$02`–`$0B` | `0`–`9` with a comma |
/// | `$0C`–`$15` | `0`–`9` with a decimal point |
/// | `$16`–`$1F` | `0`–`9` with a semicolon |
/// | `$20`–`$5F` | plain ASCII, space to underscore |
/// | `$60`–`$7F` | the test patterns, which light segments one at a time |
///
/// and **bit 7 is a comma** on top of any of them. Genesis draws its score as
/// `$4F` with `$CF` at the thousands, which is where that was found: PinMAME
/// masks the bit away and loses the separators.
///
/// The character comes back beside the segments because it is worth keeping:
/// a host that wants to *read* the display should not have to work backwards
/// from the strokes, and two characters can draw the same thing.
pub fn glyph(code: u8) -> (char, u16) {
    // The top bit hangs a comma on whatever the other seven draw.
    let comma = if code & 0x80 != 0 { COMMA } else { 0 };
    let code = code & 0x7F;
    let (c, bits) = plain(code);
    (c, bits | comma)
}

/// The seven-bit part of a code: the character it draws and the strokes it is
/// drawn with.
fn plain(code: u8) -> (char, u16) {
    match code {
        0x00 | 0x01 => (' ', 0),
        // Ten digits three times over, once for each piece of punctuation the
        // cell can carry with them.
        0x02..=0x1F => {
            let digit = (code - 0x02) % 10;
            let extra = match (code - 0x02) / 10 {
                0 => COMMA,
                1 => DOT,
                _ => COMMA | DOT,
            };
            let c = char::from(b'0' + digit);
            (c, segments(c) | extra)
        }
        // The test patterns: the low half lights one segment at a time, the
        // high half lights them one more at a time. Neither is a character.
        0x60..=0x6F => ('\u{0}', 1 << (code - 0x60)),
        0x70..=0x7F => ('\u{0}', (1u16 << (code - 0x70)).wrapping_sub(1) | 1),
        _ => {
            let c = char::from(code);
            (c, segments(c))
        }
    }
}

/// What a set of strokes reads as.
///
/// The punctuation is ignored: it is not part of the glyph. Where two
/// characters draw the same thing — `0` and `O`, `5` and `S` — the digit wins,
/// because these displays spend their lives showing scores. That is the same
/// rule the rest of this port reads its displays by.
pub fn reading(strokes: u16) -> char {
    let glyph = strokes & !(COMMA | DOT);
    if glyph == 0 {
        return ' ';
    }
    FONT.iter()
        .find(|&&(_, bits)| bits == glyph)
        .map_or('?', |&(c, _)| c)
}

/// The strokes a character is drawn with, or nothing if the font has no such
/// character.
pub fn segments(c: char) -> u16 {
    let upper = c.to_ascii_uppercase();
    FONT.iter()
        .find(|&&(f, _)| f == upper)
        .map_or(0, |&(_, bits)| bits)
}

/// One Rockwell 10939 display processor.
///
/// It holds a row of characters and draws them by itself: the game board hands
/// it one byte at a time and the chip does the multiplexing, which is the
/// whole reason it is there. A board has two, one per row, and the second is
/// a slave — it starts refreshing when the first is told to.
#[derive(Debug, Clone)]
pub struct Rockwell10939 {
    /// What each column is showing, as the code the game wrote.
    codes: [u8; COLUMNS],
    /// Whether the chip is still halted. It powers up that way and starts on
    /// the command that says so.
    halted: bool,
    /// Whether the next byte is a control word rather than a character.
    control_next: bool,
    /// How many columns the chip is driving. Zero until the game says.
    digits: u8,
    /// Where the next character will land.
    column: u8,
    /// Clocks per digit: sixteen, thirty-two or sixty-four. Sixty-four at
    /// power-on.
    digit_cycle: u8,
    /// How much of each digit's slot the chip lights it for, which is the
    /// brightness. `None` until a game has said, and a chip nobody has told
    /// runs at full.
    duty: Option<u8>,
}

impl Default for Rockwell10939 {
    fn default() -> Self {
        Self::new()
    }
}

impl Rockwell10939 {
    pub fn new() -> Self {
        Self {
            codes: [0; COLUMNS],
            halted: true,
            control_next: false,
            digits: 0,
            column: 0,
            digit_cycle: 64,
            duty: None,
        }
    }

    /// Hands the chip one byte.
    ///
    /// A `$01` says the *next* byte is a command; anything else is a character
    /// for the next column, and the column advances by itself. That is the
    /// whole protocol, and it is why a game can spell a word by writing its
    /// letters one after another and nothing else.
    pub fn load(&mut self, data: u8) {
        if !self.control_next && data == 0x01 {
            self.control_next = true;
            return;
        }
        if self.control_next && data != 0x01 {
            self.control_next = false;
            self.command(data);
            return;
        }
        // A chip that has not been told how many digits it drives is not
        // driving any, so a character has nowhere to go.
        if self.digits == 0 {
            return;
        }
        self.codes[usize::from(self.column) % COLUMNS] = data;
        self.column = (self.column + 1) % self.digits;
    }

    fn command(&mut self, data: u8) {
        match data {
            0x05 => self.digit_cycle = 16,
            0x06 => self.digit_cycle = 32,
            0x07 => self.digit_cycle = 64,
            // Cursor control, which no Gottlieb uses.
            0x08..=0x0A => {}
            0x0E => self.halted = false,
            // Six bits of value under a two-bit opcode, so the register
            // holds nought to sixty-three slots out of the digit's own count.
            0x40..=0x7F => self.duty = Some(data & 0x3F),
            // How many columns to drive. The code for thirty-two is `$80`
            // rather than the zero the field would otherwise hold.
            0x80..=0x9F => self.digits = if data == 0x80 { 32 } else { data & 0x1F },
            0xC0..=0xD3 => self.column = data & 0x1F,
            _ => {}
        }
    }

    /// Whether the chip has been started.
    pub fn running(&self) -> bool {
        !self.halted
    }

    /// Starts it, which is what the master beside it does to the slave.
    pub fn start(&mut self) {
        self.halted = false;
    }

    /// How brightly it is lighting the row, nought to one.
    ///
    /// The chip gives each character a slot of `digit_cycle` clocks and lights
    /// it for `duty + 1` of them. A chip nobody has set the duty on runs at
    /// full, which is not the same as one set to its lowest.
    pub fn brightness(&self) -> f32 {
        match self.duty {
            None => 1.0,
            Some(duty) => f32::from(duty + 1) / f32::from(self.digit_cycle),
        }
    }

    /// The raw codes the game wrote, for whoever is working out why a display
    /// says something unexpected.
    pub fn codes(&self) -> &[u8] {
        &self.codes[..self.width()]
    }

    /// The row as characters.
    ///
    /// Read off the **segments** rather than off the codes, because the two do
    /// not always agree and the segments are what somebody standing at the
    /// machine sees. Genesis draws its score with `$4F`, which the chip's
    /// character set calls `O` — its `0` at `$30` is the slashed kind, and no
    /// programmer wants that in a score. On the glass it is a ring, and a ring
    /// in a score is a nought.
    pub fn text(&self) -> String {
        self.codes
            .iter()
            .take(self.width())
            .map(|&code| reading(glyph(code).1))
            .collect()
    }

    /// The row as segment words, one per character.
    pub fn segments(&self) -> Vec<u16> {
        self.codes
            .iter()
            .take(self.width())
            .map(|&code| glyph(code).1)
            .collect()
    }

    /// How many columns are worth reading: what the chip was told to drive,
    /// and never more than the row has.
    fn width(&self) -> usize {
        if self.digits == 0 {
            COLUMNS
        } else {
            usize::from(self.digits).min(COLUMNS)
        }
    }
}

/// Both rows, and the nibble latch the game board fills them through.
#[derive(Debug, Clone, Default)]
pub struct Alphanumeric {
    /// The two processors: the top row and the bottom one.
    pub rows: [Rockwell10939; 2],
    /// The byte being assembled, a nibble at a time.
    data: u8,
    /// The last values seen on the two ports, for spotting the edges.
    last_a: u8,
    last_b: u8,
}

impl Alphanumeric {
    pub fn new() -> Self {
        Self::default()
    }

    /// RIOT 1's ports moved.
    ///
    /// Both are looked at together because the byte is built from one and
    /// strobed by the other, and which edge came first decides what the
    /// processors are handed.
    pub fn ports(&mut self, a: u8, b: u8) {
        // Port A assembles the byte out of port B's low nibble. The two halves
        // are latched on opposite edges, which is the board making one wire do
        // the work of two.
        if self.last_a & 0x10 != 0 && a & 0x10 == 0 {
            self.data = (self.data & 0xF0) | (b & 0x0F);
        }
        if self.last_a & 0x20 == 0 && a & 0x20 != 0 {
            self.data = (self.data & 0x0F) | ((b & 0x0F) << 4);
        }
        self.last_a = a;

        // And port B hands it over: one strobe per processor.
        for (i, strobe) in [0x10u8, 0x20].into_iter().enumerate() {
            if self.last_b & strobe == 0 && b & strobe != 0 {
                self.rows[i].load(self.data);
            }
        }
        self.last_b = b;

        // The second processor is a slave: it is the *first* one being told to
        // start that starts them both, and a game only ever says it once.
        if self.rows[0].running() && !self.rows[1].running() {
            self.rows[1].start();
        }
    }

    /// The two rows as text.
    pub fn text(&self) -> (String, String) {
        (self.rows[0].text(), self.rows[1].text())
    }

    /// Both rows as segment words, the top one first.
    pub fn segments(&self) -> Vec<u16> {
        let mut out = self.rows[0].segments();
        out.extend(self.rows[1].segments());
        out
    }
}
