//! The lamps, the switches and the coils, as the Whitestar board wires them.
//!
//! All three are plain memory-mapped ports rather than the PIAs a System 11
//! uses, which makes this the simplest of the three boards this project
//! emulates: there is no handshaking and no control registers, just latches the
//! CPU writes and a matrix it strobes.

/// How many lamp columns the board drives.
///
/// Sixteen, in two halves. The low eight come from the lamp strobe at `$2008`
/// and the high eight from the auxiliary board at `$2009`, and together with
/// eight rows that is the hundred and twenty-eight lamps a Whitestar game has.
pub const LAMP_COLUMNS: usize = 16;

/// The lamp matrix.
///
/// A lamp is on while its column is strobed and its row driven, which on a real
/// board is for a fraction of a sweep. Nothing here averages that: the level a
/// lamp is showing is whether it was last seen driven. The bulb model that
/// System 11 uses could be brought over, and should be — the original
/// oversamples the state twice per frame *specifically because of this game*:
///
/// > (at least) LOTR requires some kind of oversampling of the lamp state,
/// > otherwise the flickering lamps during modes are always on!
#[derive(Debug, Default, Clone)]
pub struct Lamps {
    /// Which columns are being strobed, low eight then high eight.
    column: u16,
    /// Which rows are driven, already un-reversed.
    row: u8,
    /// What has been seen lit during the sweep going on now.
    pending: [u8; LAMP_COLUMNS],
    /// The last sweep that finished, which is what anybody reading gets.
    state: [u8; LAMP_COLUMNS],
}

impl Lamps {
    /// `lampstrb_w`, `$2008`: the low half of the column select.
    pub fn set_column_low(&mut self, data: u8) {
        self.column = (self.column & 0xff00) | u16::from(data);
        self.latch();
    }

    /// `auxlamp_w`, `$2009`: the high half.
    pub fn set_column_high(&mut self, data: u8) {
        self.column = (self.column & 0x00ff) | (u16::from(data) << 8);
        self.latch();
    }

    /// `lampdriv_w`, `$200a`: the rows.
    ///
    /// The board wires the row driver backwards, so the byte is reversed on the
    /// way in — the original spells it `core_revbyte` (`se.c:610`). Getting this
    /// wrong lights the right number of lamps in the wrong places, which is
    /// harder to notice than lighting none.
    pub fn set_rows(&mut self, data: u8) {
        self.row = data.reverse_bits();
        self.latch();
    }

    pub fn column(&self) -> u16 {
        self.column
    }

    pub fn rows(&self) -> u8 {
        self.row
    }

    fn latch(&mut self) {
        for (i, slot) in self.pending.iter_mut().enumerate() {
            if self.column & (1 << i) != 0 {
                *slot |= self.row;
            }
        }
    }

    /// Whether lamp `number` is lit, numbered as the game's script does:
    /// column times eight plus row, from one.
    pub fn is_lit(&self, number: u8) -> bool {
        if number == 0 {
            return false;
        }
        let i = usize::from(number - 1);
        let (col, row) = (i / 8, i % 8);
        col < LAMP_COLUMNS && self.state[col] & (1 << row) != 0
    }

    /// The matrix as it stands, one byte per column.
    pub fn matrix(&self) -> [u8; LAMP_COLUMNS] {
        self.state
    }

    /// Closes the sweep: what was seen during it becomes what is showing.
    ///
    /// Two buffers rather than one, and the difference is the whole reason a
    /// lamp reads as lit at all. A lamp is strobed for a fraction of a sweep,
    /// so a matrix that is cleared and refilled in place is empty for most of
    /// the time anybody might look at it — a script polling sixty times a
    /// second lands in the gap and sees a dark table. What it gets instead is
    /// the last sweep that finished, complete.
    pub fn end_sweep(&mut self) {
        self.state = self.pending;
        self.pending = [0; LAMP_COLUMNS];
    }
}

/// The switch matrix.
///
/// The CPU writes a column to `$3300` and reads the rows back from `$3400`.
/// The rows come back **inverted**: a closed switch pulls its line low, so the
/// board answers `~column` (`se.c:635`).
#[derive(Debug, Default, Clone)]
pub struct Switches {
    column: u8,
    /// One byte per column, a set bit meaning closed.
    ///
    /// Nine columns, not eight. Column **zero** is the dedicated one — the
    /// coin door, the service buttons and the flipper switches — which the CPU
    /// reads at `$3000` instead of strobing. A table's script addresses it the
    /// same way it addresses the rest, as a switch number below ten, and
    /// dropping those was why a coin never became a credit: `sega.vbs` puts the
    /// three coin slots on switches 4, 5 and 6.
    closed: [u8; 9],
}

impl Switches {
    /// `switch_w`, `$3300`.
    pub fn strobe(&mut self, column: u8) {
        self.column = column;
    }

    /// Which column the board is strobing right now.
    pub fn column(&self) -> u8 {
        self.column
    }

    /// `switch_r`, `$3400`.
    pub fn read(&self) -> u8 {
        let mut rows = 0u8;
        for (i, byte) in self.closed[1..].iter().enumerate() {
            if self.column & (1 << i) != 0 {
                rows |= byte;
            }
        }
        !rows
    }

    /// Opens or closes a switch by its number, `column * 10 + row` — the
    /// numbering a table's script uses.
    ///
    /// A number below ten is column zero, which is the dedicated column and not
    /// part of the strobed matrix; see [`Switches::closed`]. Returns false for
    /// a number that is not a switch at all.
    pub fn set(&mut self, number: u8, closed: bool) -> bool {
        let Some((col, row)) = Self::position(number) else {
            return false;
        };
        if closed {
            self.closed[col] |= 1 << row;
        } else {
            self.closed[col] &= !(1 << row);
        }
        true
    }

    pub fn is_closed(&self, number: u8) -> bool {
        Self::position(number).is_some_and(|(c, r)| self.closed[c] & (1 << r) != 0)
    }

    /// `dedswitch_r`, `$3000`. Active low, so a closed switch is a zero bit.
    pub fn dedicated(&self) -> u8 {
        !self.closed[0]
    }

    /// Closes one of the dedicated switches by its bit rather than its number.
    pub fn set_dedicated(&mut self, bit: u8, closed: bool) {
        if bit < 8 {
            if closed {
                self.closed[0] |= 1 << bit;
            } else {
                self.closed[0] &= !(1 << bit);
            }
        }
    }

    pub fn clear(&mut self) {
        self.closed = [0; 9];
    }

    /// Where a switch number lands in the matrix.
    ///
    /// Sega and Stern number their switches **straight through**, one to
    /// sixty-four, and not as a column and a row the way Williams does. It is
    /// the single most consequential line in this file. Read as Williams
    /// numbering, `sega.vbs`'s three coin slots — switches 4, 5 and 6 — fall
    /// into a column that is not part of the matrix at all and a coin never
    /// becomes a credit; the start button, switch 54, lands six columns away
    /// from the button; and the ball trough reports the wrong four switches.
    /// All of it silently.
    ///
    /// The original spells the convention out where it copies the cabinet
    /// buttons in (`se.c:252`): "Copy Start, Tilt, and Slam Tilt to proper
    /// position in Matrix: Switches 54,55,56", into column 7 at bits 5, 6 and
    /// 7. Which is `1 + (54 - 1) / 8` and `(54 - 1) % 8`.
    fn position(number: u8) -> Option<(usize, usize)> {
        if !(1..=64).contains(&number) {
            return None;
        }
        let i = usize::from(number - 1);
        Some((1 + i / 8, i % 8))
    }
}

/// The solenoid drivers.
///
/// Four bytes at `$2000`, which is thirty-two coils — twice what a System 11
/// has and with none of its multiplexing. The board also drives eight more
/// through the auxiliary port at `$2006`.
#[derive(Debug, Default, Clone)]
pub struct Solenoids {
    /// Bit *n* is coil *n+1*.
    live: u32,
    aux: u8,
    /// Everything seen live since the last sweep. A coil pulse is shorter than
    /// a frame and would otherwise be missed by anything that samples.
    seen: u32,
}

impl Solenoids {
    /// Where each of the four bytes lands in the thirty-two coils.
    ///
    /// Not in address order. `se.c:662` spells it out as
    /// `{ 8, 0, 16, 24 }`: the byte at `$2000` is coils 9 to 16 and the byte at
    /// `$2001` is coils 1 to 8, and the two are the wrong way round from what
    /// anyone would assume. Assuming it costs nothing at boot and everything
    /// afterwards, because both banks are real coils and the machine simply
    /// fires the wrong ones.
    const SHIFT: [u32; 4] = [8, 0, 16, 24];

    /// `se_solenoid_w`, `$2000` to `$2003`.
    pub fn write(&mut self, offset: u8, data: u8) {
        let shift = Self::SHIFT[usize::from(offset & 3)];
        self.live = (self.live & !(0xff << shift)) | (u32::from(data) << shift);
        self.seen |= self.live;
    }

    pub fn read(&self, offset: u8) -> u8 {
        ((self.live >> Self::SHIFT[usize::from(offset & 3)]) & 0xff) as u8
    }

    /// `auxboard_w`, `$2006`.
    pub fn set_aux(&mut self, data: u8) {
        self.aux = data;
    }

    pub fn aux(&self) -> u8 {
        self.aux
    }

    /// The coils driven right now.
    pub fn live(&self) -> u32 {
        self.live
    }

    /// Everything driven since this was last asked, which is what a host that
    /// samples once a frame has to read or it will miss a pulse.
    pub fn take_seen(&mut self) -> u32 {
        std::mem::replace(&mut self.seen, self.live)
    }

    pub fn is_on(&self, number: u8) -> bool {
        (1..=32).contains(&number) && self.live & (1 << (number - 1)) != 0
    }
}
