//! What the RIOTs' ports are wired to.
//!
//! Everything a System 80 does to the outside world goes through six 8-bit
//! ports, and this is what each one carries. The wiring is Gottlieb's, read
//! off `gts80.c` — a description of a circuit board rather than a program.

/// How many switch columns the matrix has.
///
/// The strobe is one port, so eight; the returns are the other port's eight
/// bits, which makes a 8×8 matrix and sixty-four switches. Gottlieb numbers
/// them from 1 with the tens digit as the row, which is why the port's bits
/// are assembled from row 8 downwards.
pub const SWITCH_COLUMNS: usize = 8;

/// Lamp columns. Port B of RIOT 2 selects one of twelve latches with its top
/// nibble and drives four lamps with its bottom one.
pub const LAMP_COLUMNS: usize = 12;

/// The state of everything the board drives and everything it reads.
///
/// Kept apart from the board so a host can look at it, change a switch and put
/// it back without going through the chips — which is what a table's script
/// does on every hit.
#[derive(Debug, Clone)]
pub struct Io {
    /// The switch matrix, a bit per switch: `switches[column]`, bit = row.
    ///
    /// Column 0 is the coin door — the slam switch, the test buttons — and is
    /// not part of the playfield.
    pub switches: [u8; SWITCH_COLUMNS],

    /// The lamp matrix: four lamps per latch, twelve latches.
    ///
    /// Latch 0 is special and the machine reads it back: its bit 0 is the
    /// **game-on** relay and bit 1 the **tilt** relay, which is how a System
    /// 80 tells the playfield it is live.
    pub lamps: [u8; LAMP_COLUMNS],

    /// Solenoids 1 to 9, a bit each, as pulsed this frame.
    ///
    /// Nine, and the ninth is on its own pin: the first eight are two banks of
    /// four selected by a pair of enable lines, which is why a System 80 can
    /// only fire one solenoid from each bank at a time.
    pub solenoids: u16,

    /// The four banks of dip switches, as the operator set them.
    pub dips: [u8; 4],

    /// The last command written to the sound board, and whether it is new.
    pub sound_command: u8,
    pub sound_pending: bool,
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}

impl Io {
    pub fn new() -> Self {
        Self {
            switches: [0; SWITCH_COLUMNS],
            lamps: [0; LAMP_COLUMNS],
            solenoids: 0,
            dips: [0; 4],
            sound_command: 0,
            sound_pending: false,
        }
    }

    /// Opens or closes one switch, by Gottlieb's own numbering.
    ///
    /// A System 80's switch numbers read as a two-digit position — 17 is
    /// column 1, row 7 — and the machine's own manual lists them that way, so
    /// a caller that has a manual in front of it can use the number on the
    /// page.
    pub fn set_switch(&mut self, number: u8, closed: bool) {
        let (col, row) = (usize::from(number / 10), number % 10);
        if col >= SWITCH_COLUMNS || row == 0 || row > 8 {
            return;
        }
        let bit = 1u8 << (row - 1);
        if closed {
            self.switches[col] |= bit;
        } else {
            self.switches[col] &= !bit;
        }
    }

    pub fn switch_closed(&self, number: u8) -> bool {
        let (col, row) = (usize::from(number / 10), number % 10);
        if col >= SWITCH_COLUMNS || row == 0 || row > 8 {
            return false;
        }
        self.switches[col] & (1 << (row - 1)) != 0
    }

    /// Whether a lamp is lit, numbered from 1 the way the manual does.
    pub fn lamp_lit(&self, number: usize) -> bool {
        if number == 0 || number > LAMP_COLUMNS * 4 {
            return false;
        }
        let n = number - 1;
        self.lamps[n / 4] & (1 << (n % 4)) != 0
    }

    /// Whether the game-on relay is pulled in, which is a System 80's way of
    /// saying a game is in progress.
    pub fn game_on(&self) -> bool {
        self.lamps[0] & 0x01 != 0
    }

    /// And the tilt relay beside it.
    pub fn tilted(&self) -> bool {
        self.lamps[0] & 0x02 != 0
    }

    /// The returns the switch matrix drives for a given strobe.
    ///
    /// The strobe is a mask of columns and the answer is a mask of rows: any
    /// switch closed in any strobed column pulls its row down. Assembled from
    /// row 8 downwards, which is the order the port's bits are wired in.
    pub(crate) fn switch_returns(&self, strobe: u8) -> u8 {
        let mut bits = 0u8;
        for row in (1..=8).rev() {
            bits <<= 1;
            let mask = 1u8 << (row - 1);
            let any = self
                .switches
                .iter()
                .enumerate()
                .any(|(col, closed)| strobe & (1 << col) != 0 && closed & mask != 0);
            if any {
                bits |= 1;
            }
        }
        bits
    }
}
