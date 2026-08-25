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

/// How many of them the board strobes itself.
///
/// Ten: eight from the lamp strobe and two from the auxiliary board. Anything
/// above that belongs to an expander board, which latches its lamps rather
/// than strobing them — so those columns are not averaged over a window and
/// not cleared at the end of one. The original draws the same line, clearing
/// and smoothing exactly the first ten columns and copying the rest straight
/// through (`se.c:161`).
const STROBED_COLUMNS: usize = 10;

/// The lamp matrix.
///
/// A lamp is on while its column is strobed and its row driven, which on a real
/// board is for a fraction of a sweep, so what a reader gets has to be an
/// average over a window rather than an instant.
///
/// The window and the rule for it are the original's (`se.c:130`), and both
/// halves matter. The window is a **sixtieth of a second**, which has to be at
/// least as long as a full sweep of the columns or every reading misses
/// whichever columns fell outside it — a playfield that is dark in patches
/// that move. And the rule is that a lamp counts as lit only if it was driven
/// in **both** of the last two windows, which is what this game needs:
///
/// > (at least) LOTR requires some kind of oversampling of the lamp state,
/// > otherwise the flickering lamps during modes are always on!
///
/// So the state a script reads is decided thirty times a second out of two
/// sixtieth-of-a-second windows. A lamp the game is holding on is seen in both
/// and reads lit; one the game is flickering as part of its artwork is seen in
/// one and reads dark, which is the point.
#[derive(Debug, Clone)]
pub struct Lamps {
    /// Which columns are being strobed, low eight then high eight.
    column: u16,
    /// Which rows are driven, already un-reversed.
    row: u8,
    /// What has been seen driven during the window going on now.
    pending: [u8; LAMP_COLUMNS],
    /// How many of the current pair of windows each lamp was seen in.
    seen: [u8; LAMP_COLUMNS * 8],
    /// The decision from the last pair, which is what anybody reading gets.
    state: [u8; LAMP_COLUMNS],
    /// Windows closed, to tell the first of a pair from the second.
    windows: u32,
}

impl Default for Lamps {
    fn default() -> Self {
        Self {
            column: 0,
            row: 0,
            pending: [0; LAMP_COLUMNS],
            seen: [0; LAMP_COLUMNS * 8],
            state: [0; LAMP_COLUMNS],
            windows: 0,
        }
    }
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

    /// A column an expander board drives directly. See [`STROBED_COLUMNS`].
    pub fn set_latched(&mut self, column: usize, value: u8) {
        if (STROBED_COLUMNS..LAMP_COLUMNS).contains(&column) {
            self.state[column] = value;
        }
    }

    /// Closes one window. Called sixty times a second; see [`Lamps`].
    ///
    /// Every second window it decides, out of the pair, what is showing.
    pub fn end_window(&mut self) {
        for (col, driven) in self.pending.iter().enumerate().take(STROBED_COLUMNS) {
            for row in 0..8 {
                if driven & (1 << row) != 0 {
                    self.seen[col * 8 + row] += 1;
                }
            }
        }
        self.pending = [0; LAMP_COLUMNS];
        self.windows += 1;
        if !self.windows.is_multiple_of(2) {
            return;
        }
        for (col, showing) in self.state.iter_mut().enumerate().take(STROBED_COLUMNS) {
            let mut bits = 0u8;
            for row in 0..8 {
                // Both windows, not either: `lampstate[...] > 1` in the
                // original, with the comment that it is what makes this game's
                // lamps blink rather than sit on.
                if self.seen[col * 8 + row] > 1 {
                    bits |= 1 << row;
                }
            }
            *showing = bits;
        }
        self.seen = [0; LAMP_COLUMNS * 8];
    }
}

/// The switch matrix.
///
/// The CPU writes a column to `$3300` and reads the rows back from `$3400`.
/// The rows come back **inverted**: a closed switch pulls its line low, so the
/// board answers `~column` (`se.c:635`).
#[derive(Debug, Clone)]
pub struct Switches {
    column: u8,
    /// One byte per column, a set bit meaning closed. See [`SWITCH_COLUMNS`].
    closed: [u8; SWITCH_COLUMNS],
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            column: 0,
            closed: [0; SWITCH_COLUMNS],
        }
    }
}

/// How many columns of switch the board's numbering reaches.
///
/// Twelve, `CORE_STDSWCOLS` (`core.h:335`), and only eight of them are the
/// playfield matrix. A script's switch number lands in them through
/// `core_swSeq2m(n) = n + 7` (`core.c:2108`) — column `(n+7)/8`, bit `(n+7)%8`
/// — and that one expression is the whole convention:
///
/// ```text
/// column  0    n = -3..0   the coin door: memory protect, then the red,
///                          green and black buttons (`se.h:70`)
/// columns 1-8  n = 1..64   the playfield matrix, strobed from `$3300`
/// column  11   n = 81..88  the cabinet's flipper buttons and their EOS
///                          switches, `CORE_FLIPPERSWCOL` (`core.h:334`)
/// ```
///
/// Dropping either end of that is not a quiet loss: the flipper buttons are
/// switches 81 to 84 and the coin door's three buttons are 0, -1 and -2, so a
/// matrix that only holds 1 to 64 is a machine you cannot flip, cannot give a
/// service credit and cannot get into the service menu.
///
/// The gaps are kept rather than rejected, because `core_swSeq2m` is an offset
/// and not a range check and the original does the same. -7 to -4 land in the
/// low half of column 0, which nothing on this board reads; 65 to 80 are
/// columns 9 and 10, which no Whitestar game strobes. Both are accepted and
/// go nowhere, which is what a real board does with a switch that is not
/// wired.
pub const SWITCH_COLUMNS: usize = 12;

/// Where the cabinet's flipper buttons are. `CORE_FLIPPERSWCOL` (`core.h:334`).
pub const FLIPPER_COLUMN: usize = 11;

/// The black Begin Test button, `SE_SWBLACK` (`se.h:70`).
pub const SW_BLACK: i32 = 0;
/// The green Service Credit button, `SE_SWGREEN`.
pub const SW_GREEN: i32 = -1;
/// The red Volume button, `SE_SWRED`.
pub const SW_RED: i32 = -2;
/// The coin door's memory protect switch, `SE_SWMEMORYPROTECT`.
pub const SW_MEMORY_PROTECT: i32 = -3;

/// Reverses the low nybble of a byte. `core_revnyb` (`core.h:670`).
fn revnyb(x: u8) -> u8 {
    (x & 0x0f).reverse_bits() >> 4
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
        !self.strobed()
    }

    /// The one column the strobe selects.
    ///
    /// **One**, not the OR of all of them. `core_getSwCol` (`core.c:2122`)
    /// walks up from the bottom to the lowest set bit and returns that column
    /// alone, and answers **column 1** when the strobe is zero rather than
    /// "nothing is closed". Both halves matter to a running game: the ROM
    /// strobes one column at a time, so the OR is usually the same answer, but
    /// during the switch test it walks the columns with overlapping writes and
    /// an ORed matrix reports switches on columns the ROM is not looking at —
    /// which reads as targets hitting themselves.
    fn strobed(&self) -> u8 {
        let mut column = 1usize;
        let mut bits = self.column;
        if bits != 0 {
            while bits & 1 == 0 {
                bits >>= 1;
                column += 1;
            }
        }
        self.closed[column]
    }

    /// Opens or closes a switch by the number a table's script uses.
    ///
    /// Returns false for a number that is not a switch on this board at all.
    /// See [`SWITCH_COLUMNS`] for what the numbering reaches.
    pub fn set(&mut self, number: impl Into<i32>, closed: bool) -> bool {
        let Some((col, row)) = Self::position(number.into()) else {
            return false;
        };
        if closed {
            self.closed[col] |= 1 << row;
        } else {
            self.closed[col] &= !(1 << row);
        }
        true
    }

    pub fn is_closed(&self, number: impl Into<i32>) -> bool {
        Self::position(number.into()).is_some_and(|(c, r)| self.closed[c] & (1 << r) != 0)
    }

    /// `dedswitch_r`, `$3000`. Active low, so a closed switch is a zero bit.
    ///
    /// Two different columns arrive here, which is why this is not just a byte
    /// somewhere (`se.c:648`):
    ///
    /// ```text
    /// D0 left flipper   D1 left EOS    D2 right flipper  D3 right EOS
    /// D4 unused         D5 Volume      D6 Service Credit D7 Begin Test
    /// ```
    ///
    /// The top three bits are column 0 — the coin door's buttons — and the
    /// bottom five come from the flipper column, whose low nybble is
    /// **reversed** on the way (`core_revnyb`) because the core orders those
    /// four as right-EOS, right, left-EOS, left and the board wants them the
    /// other way up. Get the reversal wrong and the left flipper button works
    /// the right flipper's EOS input, which the ROM reads as a flipper that is
    /// permanently at the end of its stroke.
    pub fn dedicated(&self) -> u8 {
        let cds = self.closed[0] & 0xe0;
        let fls = self.closed[FLIPPER_COLUMN];
        let fls = revnyb(fls) | ((fls & 0x80) >> 3);
        !(cds | fls)
    }

    /// Closes one of the coin door's switches by its bit in `$3000` rather
    /// than its number. Bits 4 to 7 are switches -3, -2, -1 and 0.
    pub fn set_dedicated(&mut self, bit: u8, closed: bool) {
        if bit < 8 {
            if closed {
                self.closed[0] |= 1 << bit;
            } else {
                self.closed[0] &= !(1 << bit);
            }
        }
    }

    /// Whether the coin door's memory protect switch is set, which is what
    /// stops the CPU writing the top of its battery-backed RAM (`se.c:589`).
    pub fn memory_protect(&self) -> bool {
        self.is_closed(SW_MEMORY_PROTECT)
    }

    pub fn clear(&mut self) {
        self.closed = [0; SWITCH_COLUMNS];
    }

    /// Where a switch number lands in the matrix.
    ///
    /// `core_swSeq2m(n) = n + 7` (`core.c:2108`), and the column is that over
    /// eight. Sega and Stern number their switches **straight through** rather
    /// than as a column and a row the way Williams does, and the offset of
    /// seven is what leaves room below switch 1 for the coin door.
    ///
    /// It is the single most consequential line in this file. Read as Williams
    /// numbering, `sega.vbs`'s three coin slots — switches 4, 5 and 6 — fall
    /// into a column that is not part of the matrix at all and a coin never
    /// becomes a credit; the start button, switch 54, lands six columns away
    /// from the button; and the ball trough reports the wrong four switches.
    /// All of it silently.
    ///
    /// The original spells the convention out where it copies the cabinet
    /// buttons in (`se.c:252`): "Copy Start, Tilt, and Slam Tilt to proper
    /// position in Matrix: Switches 54,55,56", into column 7 at bits 5, 6 and
    /// 7. Which is `(54 + 7) / 8` and `(54 + 7) % 8`.
    fn position(number: i32) -> Option<(usize, usize)> {
        let i = number + 7;
        if !(0..(SWITCH_COLUMNS * 8) as i32).contains(&i) {
            return None;
        }
        let i = i as usize;
        Some((i / 8, i % 8))
    }
}

/// The solenoid drivers.
///
/// Four bytes at `$2000`, which is thirty-two coils — twice what a System 11
/// has and with none of its multiplexing. The board also drives eight more
/// through the auxiliary port at `$2006`.
///
/// # Why there are three copies of the state here
///
/// A coil pulse is a few tens of milliseconds and a script polls at whatever
/// its timer runs at, so what the script reads cannot be the live latch. The
/// original keeps three things and reports them on two different clocks
/// (`se_vblank`, `se.c:177`):
///
/// - `pulsedSolState`, the drivers as they stand this instant, which is
///   [`Solenoids::live`];
/// - `selocals.solenoids`, the OR of everything written since the last sweep,
///   copied into what the script sees every eight vblanks — **15 Hz**;
/// - `selocals.flipsol`, the two flipper power outputs, which get their own
///   sweep every two vblanks — **60 Hz** — because a flipper that answers a
///   sixteenth of a second late is a flipper nobody can aim.
#[derive(Debug, Default, Clone)]
pub struct Solenoids {
    /// Bit *n* is coil *n+1*. `coreGlobals.pulsedSolState`.
    live: u32,
    aux: u8,
    /// The three outputs of the expander board, if the game has one.
    latched: u8,
    /// Everything seen live since the last sweep. `selocals.solenoids`.
    seen: u32,
    /// What the last sweep decided, which is what anybody asking gets.
    /// `coreGlobals.solenoids`.
    reported: u32,
    /// The flipper power outputs since the last flipper sweep, and the last
    /// pair written. `selocals.flipsol` and `selocals.flipsolPulse`.
    flip: u8,
    flip_pulse: u8,
    /// What the last flipper sweep decided: the low nybble of
    /// `coreGlobals.solenoids2`.
    flip_state: u8,
}

/// The first of the four flipper solenoids a script knows about.
///
/// `CORE_FIRSTLFLIPSOL` (`core.h:303`), and the four are power and hold for
/// each lower flipper: 45 right power, 46 right, 47 left power, 48 left. A
/// table's script watches 46 and 48 (`core.vbs:2849`) and the board only ever
/// drives the two power outputs, so 46 answers for 45 and 48 for 47 —
/// `core_getSol` masks them together (`core.c:2209`).
pub const FIRST_FLIPPER_SOL: u8 = 45;

/// The solenoid the fast-flips flag is reported on.
///
/// Fifteen, and it is not a coil: the board's left flipper power output *is*
/// coil 15, and because that gets remapped to 47 the number is free. The
/// original reuses it for "the game has the flippers live", read straight out
/// of a byte of CPU RAM (`se.c:185`), and `sega.vbs:23` calls it
/// `GameOnSolenoid` — which is what `vpmFlips` polls to decide whether a
/// flipper key does anything at all (`core.vbs:2132`).
pub const FAST_FLIP_SOL: u8 = 15;

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
    ///
    /// The byte at `$2000` is not quite eight coils: its top two bits are the
    /// flipper *power* outputs, coils 16 and 15, and the original pulls them
    /// out of the word entirely — `sols &= 0xffff3fff` — and moves them to 45
    /// and 47 (`se.c:666`). They have to move because a table's script drives
    /// its flippers from those numbers and would otherwise be watching two
    /// coils that mean nothing to it; and they have to be masked off because
    /// 15 is then reused for the fast-flips flag. See [`FAST_FLIP_SOL`].
    pub fn write(&mut self, offset: u8, data: u8) {
        let offset = offset & 3;
        let shift = Self::SHIFT[usize::from(offset)];
        let mut sols = u32::from(data) << shift;
        if offset == 0 {
            // Bit 7 is coil 16, the right flipper's power, and goes to
            // solenoid 45 — the low bit of the nybble. Bit 6 is coil 15, the
            // left flipper's, and goes to 47 — two bits up.
            self.flip_pulse = ((data & 0x80) >> 7) | ((data & 0x40) >> 4);
            self.flip |= self.flip_pulse;
            sols &= 0xffff_3fff;
        }
        self.live = (self.live & !(0xff << shift)) | sols;
        self.seen |= sols;
    }

    /// `solenoid_r`. Some Whitestar II ROMs read the drivers back.
    ///
    /// The two flipper power bits are put back where the CPU wrote them
    /// (`se.c:680`): they are not in the word any more, so reading `$2000`
    /// without them would tell the ROM it never drove a flipper.
    pub fn read(&self, offset: u8) -> u8 {
        let offset = offset & 3;
        let mut data = ((self.live >> Self::SHIFT[usize::from(offset)]) & 0xff) as u8;
        if offset == 0 {
            data &= 0x3f;
            data |= (self.flip_pulse & 0x01) << 7;
            data |= (self.flip_pulse & 0x04) << 4;
        }
        data
    }

    /// `auxboard_w`, `$2006`.
    pub fn set_aux(&mut self, data: u8) {
        self.aux = data;
    }

    pub fn aux(&self) -> u8 {
        self.aux
    }

    /// The coils driven right now, unsmoothed. `coreGlobals.pulsedSolState`.
    pub fn live(&self) -> u32 {
        self.live
    }

    /// Everything driven since this was last asked, which is what a host that
    /// samples once a frame has to read or it will miss a pulse.
    pub fn take_seen(&mut self) -> u32 {
        std::mem::replace(&mut self.seen, self.live)
    }

    /// Closes one sweep of the flipper power outputs, at 60 Hz.
    ///
    /// `se_vblank` at `% VBLANK` (`se.c:177`): what the last window saw becomes
    /// what the script sees, and the window restarts holding whatever the ROM
    /// last wrote rather than nothing — so a flipper that is being *held* does
    /// not blink off between sweeps.
    pub fn end_flipper_window(&mut self) {
        self.flip_state = self.flip;
        self.flip = self.flip_pulse;
    }

    /// Closes one sweep of the thirty-two drivers, at 15 Hz.
    ///
    /// `se_vblank` at `% (VBLANK*SE_SOLSMOOTH)` (`se.c:181`). `fast_flips` is
    /// the byte of CPU RAM the game keeps its "flippers are live" flag in; see
    /// [`FAST_FLIP_SOL`].
    pub fn end_sweep(&mut self, fast_flips: bool) {
        self.reported = self.take_seen();
        if fast_flips {
            self.reported |= 1 << (FAST_FLIP_SOL - 1);
        }
    }

    /// The three outputs of the 520-5068-01 expander board, numbered 33 to 35.
    ///
    /// Latched rather than driven: the board holds what it was last clocked
    /// with, so unlike the thirty-two real drivers these do not need to be
    /// re-asserted to stay on.
    pub fn set_latched(&mut self, data: u8) {
        self.latched = data & 0x07;
    }

    pub fn latched(&self) -> u8 {
        self.latched
    }

    /// Whether a solenoid is on, as a script reading `Controller.Solenoid(n)`
    /// or `ChangedSolenoids` sees it — which is the last sweep's answer and not
    /// the live latch. See [`Solenoids`].
    pub fn is_on(&self, number: u8) -> bool {
        match number {
            1..=32 => self.reported & (1 << (number - 1)) != 0,
            33..=35 => self.latched & (1 << (number - 33)) != 0,
            // 45 and 47 are the two power outputs and 46 and 48 are the holds
            // the board does not have, so each hold answers for its power
            // output — `core_getSol` masks the pair together (`core.c:2209`).
            45..=48 => {
                let mask = match number {
                    45 => 0x01,
                    46 => 0x03,
                    47 => 0x04,
                    _ => 0x0c,
                };
                self.flip_state & mask != 0
            }
            _ => false,
        }
    }
}
