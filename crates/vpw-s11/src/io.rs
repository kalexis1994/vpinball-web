//! Board inputs and outputs: switches, lamps and solenoids.
//!
//! # How things are numbered
//!
//! Switches and lamps are numbered **1 to 64, straight through**, in the order
//! the CPU sweeps them: 1 is column 1 row 1, 9 is column 2 row 1, 64 is column
//! 8 row 8. That is what a machine's manual prints, what PinMAME's game
//! definitions use — F-14's flipper buttons are 63 and 15 — and what a table's
//! script writes.
//!
//! It is worth being explicit, because the obvious alternative is wrong in a
//! way that half works. Williams' service documentation lays the matrix out as
//! a grid and positions get written as two digits, so it is easy to read a
//! number as column-and-row and land on 11 through 88. Tables disprove it on
//! their own: F-14 has switches 20, 30, 49, 50, 59 and 60 — rows 0 and 9 in a
//! matrix with eight — and it maps its lights `NFadeL 1, l1` through
//! `NFadeL 64, l64`. Read the wrong way every lamp still lights and every
//! switch still closes; they are just not the ones the table meant, which is
//! the kind of wrong that survives a long time.
//!
//! The first column is the coin door: tilt, ball-roll tilt, start, the three
//! coin chutes, slam tilt and high-score reset, in that order — exactly the
//! list at the top of `s11.vbs`. It is strobed like any other column.

use crate::bulb::{Drive, Filament};

/// Translates a switch or lamp number into `(column index, row mask)`.
///
/// Returns `None` outside 1 to 64 — a number that does not exist, rather than
/// one that quietly means something else.
pub fn matrix_position(number: u8) -> Option<(usize, u8)> {
    if !(1..=64).contains(&number) {
        return None;
    }
    let i = usize::from(number - 1);
    Some((i / 8, 1 << (i % 8)))
}

/// Switches, which reach further than lamps do.
///
/// The first sixty-four are the playfield matrix and are numbered exactly like
/// the lamps. Past those, the original's `swMatrix` keeps columns the CPU never
/// strobes, for switches that are not on the playfield at all, and the same
/// `no+7` arithmetic addresses them (`core_swSeq2m`, `core.c:2108`). One of
/// those columns matters here: number eleven is the flipper buttons
/// (`CORE_FLIPPERSWCOL`, `core.h:334`), which is where `s11.vbs` puts them —
///
/// ```vbs
/// Const swLRFlip = 82              ' s11.vbs:37
/// Const swLLFlip = 84
/// Case LeftFlipperKey
///     .Switch(swLLFlip) = True : vpmKeyDown = False : vpmFlips.FlipL True
/// ```
///
/// — and rejecting eighty-four as "not a switch" drops the flipper buttons on
/// the floor. See [`FLIPPER_COLUMN`].
pub fn switch_position(number: u8) -> Option<(usize, u8)> {
    if !(1..=SWITCH_COUNT as u8).contains(&number) {
        return None;
    }
    let i = usize::from(number - 1);
    Some((i / 8, 1 << (i % 8)))
}

/// Columns of the switch matrix the port keeps, one to eleven.
///
/// Eight of them are the playfield and the CPU strobes those; the rest exist
/// because the original's array does, and because a script writes into one of
/// them. See [`switch_position`].
pub const SWITCH_COLUMNS: usize = 11;

/// The highest switch number there is, eleven columns of eight.
pub const SWITCH_COUNT: usize = SWITCH_COLUMNS * 8;

/// The column the flipper buttons live in, zero-based, and the two switches in
/// it. `CORE_FLIPPERSWCOL` is 11 one-based (`core.h:334`).
pub const FLIPPER_COLUMN: usize = 10;

/// Known switch numbers.
pub mod switches {
    /// Left flipper button on F-14 Tomcat (`s11games.c:407`,
    /// `FLIP_SWNO(63,15)`).
    ///
    /// This is where the *board* sees it, on the playfield matrix. It is not
    /// where a script writes it — see [`FLIPPER_BUTTON_LEFT`].
    pub const F14_FLIPPER_LEFT: u8 = 63;
    /// Right flipper button on F-14 Tomcat.
    pub const F14_FLIPPER_RIGHT: u8 = 15;

    /// Where a **script** reports the flipper buttons: `swLLFlip` and
    /// `swLRFlip` (`s11.vbs:37-38`), which are bits 3 and 1 of the flipper
    /// column and therefore numbers 84 and 82.
    ///
    /// Every System 11 has them there; which playfield switch they in turn
    /// drive is the game's own business and comes from its `FLIP_SWNO`.
    pub const FLIPPER_BUTTON_LEFT: u8 = 84;
    pub const FLIPPER_BUTTON_RIGHT: u8 = 82;

    /// Switches that fire F-14's special solenoids
    /// (the `ss17`-`ss22` parameters of its `INITGAMEFULL`).
    pub const F14_SPECIAL_SOLENOID_TRIGGERS: [u8; 3] = [57, 58, 28];
}

/// The playfield's switch matrix.
///
/// PIA 4 drives the columns through port B and reads the rows through port A.
/// Closing a switch connects a row to a column, so the CPU only sees it while
/// it is strobing that switch's column.
#[derive(Debug, Clone, Default)]
pub struct SwitchMatrix {
    /// `columns[i]` are the closed switches of column `i + 1`.
    ///
    /// Eleven of them, not eight: the CPU only ever strobes the first eight,
    /// but the original's array is longer and a script writes into the eleventh
    /// (see [`switch_position`]).
    columns: [u8; SWITCH_COLUMNS],
}

impl SwitchMatrix {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens or closes a switch by its number, 1 to 88.
    ///
    /// Returns `false` if the number does not exist in the matrix.
    pub fn set(&mut self, number: u8, closed: bool) -> bool {
        let Some((column, mask)) = switch_position(number) else {
            return false;
        };
        if closed {
            self.columns[column] |= mask;
        } else {
            self.columns[column] &= !mask;
        }
        true
    }

    /// Closes a switch and opens it again. That is what a ball does when it
    /// goes over a rollover, which is most of the cases.
    ///
    /// Careful: for the ROM to see it, the machine has to run between the
    /// close and the open. This is good for setting up the state, not for
    /// simulating the whole pulse.
    pub fn is_closed(&self, number: u8) -> bool {
        switch_position(number).is_some_and(|(column, mask)| self.columns[column] & mask != 0)
    }

    /// Closes or opens a switch by its physical place in the matrix, without
    /// going through the numbering. For working out what the numbering is.
    pub fn set_raw(&mut self, column: usize, bit: u8, closed: bool) {
        if column >= self.columns.len() || bit > 7 {
            return;
        }
        if closed {
            self.columns[column] |= 1 << bit;
        } else {
            self.columns[column] &= !(1 << bit);
        }
    }

    /// Opens every switch.
    pub fn clear(&mut self) {
        self.columns = [0; SWITCH_COLUMNS];
    }

    /// Rows the CPU reads with the given columns driven.
    pub fn rows_for_strobe(&self, strobe: u8) -> u8 {
        let mut rows = 0;
        // Only the eight the CPU can address: the strobe is one byte, one bit
        // per column, and the columns past those are not on the playfield at
        // all — nothing drives them and the CPU never asks. See
        // [`switch_position`].
        for (i, closed) in self.columns.iter().take(8).enumerate() {
            if strobe & (1 << i) != 0 {
                rows |= closed;
            }
        }
        rows
    }

    /// Raw state, by column.
    pub fn columns(&self) -> &[u8; SWITCH_COLUMNS] {
        &self.columns
    }
}

/// How brightly a lamp has to glow to count as on, where the answer has to be
/// yes or no. `core.c:3315`.
const LIT_THRESHOLD: f32 = 0.1;

/// The lamp matrix.
///
/// Two things make this more than a grid of bits. It is **multiplexed**: PIA 1
/// strobes one column at a time, so at any instant seven eighths of the lamps
/// are off. And it is **modulated**: the ROM dims a lamp by driving it on some
/// passes and skipping others, which is how a System 11 gets more than two
/// brightnesses out of a bulb.
///
/// A real bulb turns both of those into a steady glow, because a filament
/// integrates. Reading the matrix as a boolean does not, at any sampling rate:
/// too fast and you catch the strobe, too slow and you beat against it, and
/// either way the whole playfield flickers.
///
/// Measured on F-14, the ROM walks the matrix about 67 times a second and dims
/// a lamp by driving it on one pass and skipping the next. There is a bit over
/// one pass per display frame, so there is nothing to average **within** a
/// frame: whatever the frame timing, a boolean answer alternates. What settles
/// it is the filament, whose glow follows the drive with a lag of a few tens of
/// milliseconds — so a lamp driven every other pass sits at half brightness
/// instead of blinking at thirty hertz.
///
/// So this does not sample the matrix at all. It keeps sixty-four [`Filament`]s
/// and drives them, and what comes out is a **level**: how bright each bulb
/// actually is. That is also where the original ends up — it integrates a bulb
/// per lamp and reports a modulated level that `core.vbs` scales by `pwmScale`
/// — and it is the only reading under which "driven one pass in two" means half
/// brightness rather than a thirty-hertz flicker.
#[derive(Debug, Clone)]
pub struct LampMatrix {
    /// One filament per lamp, which is what actually answers the question.
    bulbs: [[Filament; 8]; 8],
    /// Which lamps the ROM is driving this instant.
    driving: [u8; 8],
}

impl Default for LampMatrix {
    fn default() -> Self {
        LampMatrix {
            bulbs: [[Filament::new(Drive::LAMP_MATRIX); 8]; 8],
            driving: [0; 8],
        }
    }
}

impl LampMatrix {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what PIA 1 is driving at `now` seconds: `rows` on port A and
    /// `strobe` (the columns) on port B.
    ///
    /// Both writes come here, as they do in the original (`s11.c:363`), which
    /// means the microseconds between the ROM setting the rows and moving the
    /// strobe are seen as a moment of the wrong lamps being driven. That is
    /// what really happens on the machine, and because the filaments are
    /// integrated over time rather than sampled, a few microseconds of it
    /// contribute a few microseconds' worth of heat and nothing more.
    pub fn drive(&mut self, now: f32, rows: u8, strobe: u8) {
        for (column, driving) in self.driving.iter_mut().enumerate() {
            *driving = if strobe & (1 << column) != 0 { rows } else { 0 };
        }
        // After the update, not before: what a filament is told is the drive
        // from here on, and it remembers the one it has been under since it was
        // last touched. Integrating first would hand every bulb the state it
        // had one write ago, and the whole matrix would lag a column behind.
        self.integrate(now);
    }

    /// Brings every filament up to `now` under the drive it has had.
    pub fn integrate(&mut self, now: f32) {
        for (column, bulbs) in self.bulbs.iter_mut().enumerate() {
            for (row, bulb) in bulbs.iter_mut().enumerate() {
                bulb.integrate(now, self.driving[column] & (1 << row) != 0);
            }
        }
    }

    /// How brightly a lamp is showing, from 0 to 1.
    pub fn level(&self, number: u8) -> f32 {
        match matrix_position(number) {
            Some((column, mask)) => self.bulbs[column][mask.trailing_zeros() as usize].brightness(),
            None => 0.0,
        }
    }

    /// Whether a lamp counts as showing.
    ///
    /// A tenth, which is the threshold the original uses when it turns its own
    /// integrated levels back into a bitmap (`core.c:3315`).
    pub fn is_lit(&self, number: u8) -> bool {
        self.level(number) >= LIT_THRESHOLD
    }

    /// State of the last frame, by column, as a bitmap of the lamps showing.
    pub fn columns(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        for (column, bits) in out.iter_mut().enumerate() {
            for row in 0..8 {
                if self.bulbs[column][row].brightness() >= LIT_THRESHOLD {
                    *bits |= 1 << row;
                }
            }
        }
        out
    }

    /// How many lamps are showing.
    pub fn lit_count(&self) -> u32 {
        self.columns().iter().map(|c| c.count_ones()).sum()
    }
}

/// Over how many frames a solenoid's state is held.
///
/// `S11_SOLSMOOTH` in the original (`s11.c:205`), where the published state is
/// `solsmooth[0] | solsmooth[1]` — on if it was on in either of the last two
/// frames. Without it a coil the ROM holds down reads as flapping whenever a
/// frame happens to catch none of its drive, and on F-14 the coil in question
/// is the general-illumination relay: the script toggles the whole playfield's
/// lighting on every flap, and clacks the relay sound each time.
const SOLENOID_SMOOTHING: usize = 2;

/// State of the solenoids.
///
/// System 11 drives them in two groups: 1 through 8 from a latch at `$2200`
/// and 9 through 16 from PIA 0's port B. Solenoids pulse for a few
/// milliseconds, so looking at the instantaneous state misses almost every
/// firing; that is why which ones went active during the sweep is accumulated
/// as well.
#[derive(Debug, Clone, Default)]
pub struct Solenoids {
    /// Solenoids 1-8, from the `$2200` latch.
    low: u8,
    /// Solenoids 9-16, from PIA 0's port B.
    high: u8,
    /// Special solenoids 17-22, in bits 0 to 5. Whatever their control line or
    /// their switch has most recently said, before [`Solenoids::ss_enabled`]
    /// gets a vote. `locals.switchedSol`, `s11.c:542`.
    switched: u8,
    /// PIA 0's CB2, inverted: whether the special solenoids are live at all.
    /// `locals.ssEn`, `s11.c:612`.
    ss_enabled: bool,
    /// The four invented flipper solenoids, 45 to 48, as the bits
    /// `coreGlobals.solenoids2` keeps them in. See [`FIRST_FLIPPER`].
    flippers: u8,
    /// The solenoid that drives the mux relay, if this game has one.
    /// See [`Solenoids::set_mux`].
    mux: Option<u8>,
    /// The ones that went active during the frame in progress.
    fired_accum: u32,
    /// The last two frames' worth, kept separately so the answer can be the
    /// two of them together. See [`Solenoids::refresh`].
    frames: [u32; SOLENOID_SMOOTHING],
    /// Which of the two is next to be overwritten.
    frame: usize,
    /// The outputs that are not coils but bulbs, and the filament each one is
    /// really driving. See [`Solenoids::set_bulb`].
    bulbs: [Option<Filament>; SOLENOID_COUNT],
    /// The drive each bulb has had since the filaments were last brought up to
    /// date, so a pulse shorter than a frame is not lost.
    bulb_driven: u32,
}

/// How many solenoid outputs a System 11 board has, counting the special ones
/// and the second bank the mux relay reaches.
///
/// Thirty-two, laid out as: 1-8 from the `$2200` latch, 9-16 from PIA 0's port
/// B, 17-22 the special ones, 23 "game on", 24 unused, and 25-32 the same
/// eight drivers as 1-8 seen through the mux relay. `s11.c:577`.
pub const SOLENOID_COUNT: usize = 32;

/// The first of the six special solenoids. `CORE_FIRSTSSSOL`, `core.h:308`.
const FIRST_SPECIAL: u8 = 17;

/// The first of the four flipper solenoids. `CORE_FIRSTLFLIPSOL`, `core.h:303`.
///
/// They are not driver outputs and no ROM writes them. On a System 11 the
/// flippers are wired straight from the cabinet buttons through the flipper
/// relay, so the CPU never sees them at all — and yet a table's script is
/// written entirely in terms of them:
///
/// ```vbs
/// Const sLRFlipper = 46            ' core.vbs:2849
/// Const sLLFlipper = 48
/// SolCallback(sLLFlipper) = "SolLFlipper"
/// ```
///
/// and `SolLFlipper` is what plays `fx_Flipperup` **and** calls
/// `LeftFlipper.RotateToEnd`. So PinMAME invents them, from the buttons, for
/// exactly the games whose flippers the CPU does not drive
/// (`core.c:1746-1753`). Without them a table's flippers are silent and, on a
/// faithful port, would not move at all.
pub const FIRST_FLIPPER: u8 = 45;

/// The two the right button raises, and the two the left one does
/// (`CORE_LRFLIPSOLBITS` and `CORE_LLFLIPSOLBITS`, `core.h:324-325`). Each
/// button raises a pair: the "power" winding and the "hold" one, which is why
/// there are four solenoids for two flippers.
const RIGHT_FLIPPER_BITS: u8 = 0x03;
const LEFT_FLIPPER_BITS: u8 = 0x0c;

/// Where each of the six control lines lands among the special solenoids.
///
/// The lines arrive in the order PIA 1 CA2, PIA 1 CB2, PIA 3 CA2, PIA 3 CB2,
/// PIA 4 CA2, PIA 4 CB2, and the solenoids they drive are **not** in that
/// order. This is the Williams half of `ssSolNo`, `s11.c:539`; Data East wired
/// the same six lines differently again.
const SPECIAL_ORDER: [u8; 6] = [5, 4, 1, 2, 0, 3];

/// The output that says a game is in progress. `S11_GAMEONSOL`.
const GAME_ON: u8 = 23;

impl Solenoids {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that a solenoid output is wired to a bulb rather than to a coil.
    ///
    /// This is not a detail. Several of the sixteen outputs on a System 11 do
    /// not drive a coil at all: they switch a general illumination string or a
    /// flasher, and the game modulates them — chopping the GI to dim it, ramping
    /// a flasher to fade it. Asked as a yes-or-no question at frame rate, a
    /// chopped GI string reads as flapping, and a table's script dutifully
    /// switches the whole playfield's lighting on and off and clacks a relay
    /// sound every time. The bulb is what makes the answer steady, exactly as
    /// it does for the lamp matrix.
    ///
    /// The original keeps the same table, per game, in `s11.c:895` onwards; the
    /// entries for a given ROM set live in [`crate::games`].
    pub fn set_bulb(&mut self, number: u8, drive: Drive) {
        if let Some(i) = Self::index(number) {
            self.bulbs[i] = Some(Filament::new(drive));
        }
    }

    /// Which solenoid, if any, throws the mux relay on this game.
    ///
    /// A System 11 board has eight low-side drivers and some games want
    /// sixteen, so a relay switches the same eight between two banks of coils.
    /// While it is pulled in, everything the ROM writes to solenoids 1-8 comes
    /// out of solenoids 25-32 instead — the "C side" — and the A side is dead.
    /// The relay itself is an ordinary solenoid, and which one it is depends on
    /// the game: it is the fourth argument of the original's `INITGAMEFULL`
    /// (`s11games.c:38`), and 14 on F-14.
    pub fn set_mux(&mut self, number: u8) {
        self.mux = Some(number);
    }

    /// Brings the bulbs up to `now`, under the drive they have had since the
    /// last call.
    pub fn integrate(&mut self, now: f32) {
        for (i, bulb) in self.bulbs.iter_mut().enumerate() {
            if let Some(bulb) = bulb {
                bulb.integrate(now, self.bulb_driven & (1u32 << i) != 0);
            }
        }
    }

    /// How brightly a bulb output is showing, or `None` if that output is a
    /// coil and the question does not apply.
    pub fn level(&self, number: u8) -> Option<f32> {
        self.bulbs[Self::index(number)?].map(|b| b.brightness())
    }

    /// A solenoid number as an index, or `None` if there is no such solenoid.
    fn index(number: u8) -> Option<usize> {
        (1..=SOLENOID_COUNT as u8)
            .contains(&number)
            .then(|| usize::from(number - 1))
    }

    /// Write to the `$2200` latch: solenoids 1-8.
    pub fn set_low(&mut self, now: f32, value: u8) {
        self.low = value;
        self.publish(now);
    }

    /// PIA 0's port B: solenoids 9-16.
    pub fn set_high(&mut self, now: f32, value: u8) {
        self.high = value;
        self.publish(now);
    }

    /// One of the six control lines that drive a special solenoid, from PIA 1,
    /// 3 or 4's CA2 or CB2. `setSSSol`, `s11.c:537`.
    ///
    /// `line` is 0 to 5 in the order the original hooks them up, and `level` is
    /// the pin: **low means on**, as it does on all of these.
    pub fn set_special_line(&mut self, now: f32, line: usize, level: bool) {
        let Some(&offset) = SPECIAL_ORDER.get(line) else {
            return;
        };
        let bit = 1u8 << offset;
        if level {
            self.switched &= !bit;
        } else {
            self.switched |= bit;
        }
        self.publish(now);
    }

    /// A special solenoid wired straight to a switch, closed or opened.
    ///
    /// Six of the outputs can be driven by a playfield switch rather than by
    /// the CPU, and on the machine that wiring is the point: a slingshot or a
    /// pop bumper fires from its own switch, through the hardware, without
    /// waiting for the ROM to notice. The original reads these once a frame in
    /// its fake VBLANK (`s11.c:193`); which switch drives which is per game,
    /// and lives in [`crate::games`].
    ///
    /// `offset` is 0 to 5, for solenoids 17 to 22.
    pub fn set_special_switch(&mut self, now: f32, offset: usize, closed: bool) {
        if offset >= 6 {
            return;
        }
        let bit = 1u8 << offset;
        if closed {
            self.switched |= bit;
        } else {
            self.switched &= !bit;
        }
        self.publish(now);
    }

    /// PIA 0's CB2, which is inverted: the pin low is the special solenoids
    /// enabled. `locals.ssEn = !data`, `s11.c:612`.
    pub fn set_special_enabled(&mut self, now: f32, enabled: bool) {
        self.ss_enabled = enabled;
        // The gate opening or closing changes the state of seven outputs at
        // once, so it publishes like any other write. The original does the
        // same thing from `pia0cb2_w` (`s11.c:612`), where it pushes game-on
        // and the whole switched-solenoid group in the same breath.
        self.publish(now);
    }

    pub fn special_enabled(&self) -> bool {
        self.ss_enabled
    }

    /// Raises or drops the invented flipper solenoids from the buttons.
    ///
    /// `core.c:1746-1753`, and the gate is the same `ssEn` that governs the
    /// special solenoids — `core_updateSw(locals.ssEn)` (`s11.c:358`). That is
    /// not incidental: the flipper relay and the switched solenoids are the
    /// same relay, so between balls, in the operator's menu and on a tilt the
    /// flippers go dead together with the slingshots, which is what the machine
    /// does.
    pub fn set_flipper_buttons(&mut self, now: f32, left: bool, right: bool) {
        let mut bits = 0;
        if self.ss_enabled {
            if left {
                bits |= LEFT_FLIPPER_BITS;
            }
            if right {
                bits |= RIGHT_FLIPPER_BITS;
            }
        }
        if bits == self.flippers {
            return;
        }
        self.flippers = bits;
        self.publish(now);
    }

    /// Whether one of the four flipper solenoids is up right now.
    fn flipper_active(&self, number: u8) -> bool {
        let Some(bit) = number.checked_sub(FIRST_FLIPPER) else {
            return false;
        };
        bit < 4 && self.flippers & (1 << bit) != 0
    }

    /// Instantaneous state of all thirty-two, as a bitmap where bit 0 is
    /// solenoid 1. `updsol`, `s11.c:558`.
    pub fn active(&self) -> u32 {
        let mut state = u32::from(self.low) | (u32::from(self.high) << 8);

        // The special solenoids, and "game on" with them: all seven are dead
        // while PIA 0's CB2 says so, whatever their own lines say.
        if self.ss_enabled {
            state |= u32::from(self.switched) << (FIRST_SPECIAL - 1);
            state |= 1 << (GAME_ON - 1);
        }

        // The mux relay, last, because what it switches is the A side and it is
        // not itself on the A side.
        if self.mux.is_some_and(|m| state & (1 << (m - 1)) != 0) {
            state = (state & 0x00FF_FF00) | (state << 24);
        }
        state
    }

    /// Records the current drive and brings the bulbs up to date.
    fn publish(&mut self, now: f32) {
        let state = self.active();
        self.fired_accum |= state;
        self.bulb_driven = state;
        self.integrate(now);
    }

    /// The ones that went active at some point during the last sweep.
    ///
    /// This is what to look at to find out whether a coil fired: a pulse a few
    /// milliseconds long does not show up in `active()` except by luck.
    pub fn fired(&self) -> u32 {
        self.frames.iter().fold(0, |a, f| a | f)
    }

    /// Whether a solenoid (1-32) fired during the last couple of frames.
    pub fn did_fire(&self, number: u8) -> bool {
        // The flipper solenoids are not driver outputs and have no pulse to
        // accumulate: they are up for exactly as long as the button is held,
        // so "did it fire this frame" and "is it up" are the same question.
        if number >= FIRST_FLIPPER {
            return self.flipper_active(number);
        }
        Self::index(number).is_some_and(|i| self.fired() & (1 << i) != 0)
    }

    /// Closes the frame.
    ///
    /// The accumulator restarts from whatever is **held**, not from nothing
    /// (`locals.solenoids = coreGlobals.pulsedSolState`, `s11.c:219`). This is
    /// the whole difference between a coil that is on and a coil that was
    /// switched on: the ROM writes the latch once and leaves it, so a frame
    /// that happens to contain no write to it sees no pulse — and starting from
    /// zero turns a relay the game is holding down into one that flaps at frame
    /// rate. On F-14 that relay is the A/C mux, which the table has wired to the
    /// playfield lighting, so the whole playfield blinked and the relay sound
    /// clacked along with it.
    pub fn refresh(&mut self) {
        self.frames[self.frame] = self.fired_accum;
        self.frame = (self.frame + 1) % SOLENOID_SMOOTHING;
        self.fired_accum = self.active();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbering_runs_straight_through() {
        // 1 to 64 in sweep order, not column-and-row. F-14's flipper buttons
        // are 63 and 15, and its lights are `l1` through `l64`.
        assert_eq!(matrix_position(1), Some((0, 0b0000_0001)));
        assert_eq!(matrix_position(8), Some((0, 0b1000_0000)));
        assert_eq!(matrix_position(9), Some((1, 0b0000_0001)));
        assert_eq!(matrix_position(63), Some((7, 0b0100_0000)));
        assert_eq!(matrix_position(64), Some((7, 0b1000_0000)));
    }

    #[test]
    fn numbers_outside_the_matrix_are_rejected() {
        for number in [0, 65, 88, 100, 255] {
            assert_eq!(matrix_position(number), None, "{number} does not exist");
        }
    }

    #[test]
    fn the_coin_door_is_the_first_column() {
        // `s11.vbs:24`: tilt, ball roll tilt, start, coin 3, coin 2, coin 1,
        // slam, high score reset. All eight in one column, which is how a
        // coin reaches the CPU at all.
        for number in 1..=8 {
            assert_eq!(matrix_position(number).unwrap().0, 0, "switch {number}");
        }
    }

    #[test]
    fn a_switch_is_only_seen_with_its_column_strobed() {
        let mut m = SwitchMatrix::new();
        assert!(m.set(switches::F14_FLIPPER_LEFT, true));
        assert!(m.is_closed(63));

        // Switch 63 is the seventh of column 8.
        assert_eq!(m.rows_for_strobe(0b0000_0001), 0, "column 1: not seen");
        assert_eq!(
            m.rows_for_strobe(0b1000_0000),
            0b0100_0000,
            "column 8, row 7"
        );
        assert_eq!(m.rows_for_strobe(0xFF), 0b0100_0000, "every column");
    }

    #[test]
    fn an_invalid_number_breaks_nothing() {
        let mut m = SwitchMatrix::new();
        assert!(!m.set(99, true), "99 does not exist");
        assert!(!m.is_closed(99));
        assert_eq!(m.rows_for_strobe(0xFF), 0);
    }

    #[test]
    fn the_two_f14_flippers_are_in_different_columns() {
        let mut m = SwitchMatrix::new();
        m.set(switches::F14_FLIPPER_LEFT, true);
        m.set(switches::F14_FLIPPER_RIGHT, true);

        // 63 is column 8 row 7; 15 is column 2 row 7. The two buttons are on
        // different columns, so a sweep sees one at a time — which is the
        // reason the matrix has to be swept at all.
        assert_eq!(m.rows_for_strobe(0b1000_0000), 0b0100_0000);
        assert_eq!(m.rows_for_strobe(0b0000_0010), 0b0100_0000);
        assert_eq!(m.rows_for_strobe(0b0000_0001), 0, "neither is in column 1");
    }

    /// A sweep of the matrix, in real time: eight columns at a millisecond
    /// each, which is about how fast F-14's ROM goes round. `rows` is what is
    /// driven in column 1; the rest of the columns are dark.
    ///
    /// Time is what the filaments run on, so a test that does not advance it
    /// tests nothing.
    fn sweep(l: &mut LampMatrix, t: &mut f32, rows: u8) {
        for column in 0..8u8 {
            let driven = if column == 0 { rows } else { 0 };
            l.drive(*t, driven, 1 << column);
            *t += 0.001;
        }
        l.integrate(*t);
    }

    #[test]
    fn a_lamp_driven_every_other_sweep_glows_half() {
        // Which is how a System 11 dims a lamp: there is no brightness control,
        // only the choice of skipping a pass.
        let (mut l, mut t) = (LampMatrix::new(), 0.0);
        for _ in 0..60 {
            sweep(&mut l, &mut t, 0b0000_0001);
            sweep(&mut l, &mut t, 0b0000_0000);
        }
        let dimmed = l.level(1);
        assert!(
            (0.1..0.6).contains(&dimmed),
            "a lamp driven half the time glows part way, not {dimmed}"
        );
        assert!(l.is_lit(1), "and it is on, not flickering");

        // And it stays there rather than swinging: sampled all through the two
        // sweeps it is meant to be a steady half, not a square wave.
        let (mut low, mut high) = (f32::MAX, 0.0f32);
        for _ in 0..10 {
            sweep(&mut l, &mut t, 0b0000_0001);
            low = low.min(l.level(1));
            high = high.max(l.level(1));
            sweep(&mut l, &mut t, 0b0000_0000);
            low = low.min(l.level(1));
            high = high.max(l.level(1));
        }
        assert!(high - low < 0.05, "steady, not blinking: {low}..{high}");
    }

    #[test]
    fn a_lamp_driven_every_sweep_is_simply_on() {
        let (mut l, mut t) = (LampMatrix::new(), 0.0);
        for _ in 0..60 {
            sweep(&mut l, &mut t, 0b0000_0001);
        }
        assert!(l.level(1) > 0.6, "on: {}", l.level(1));
        assert!(l.is_lit(1));
        assert!(!l.is_lit(2), "and its neighbour is not");

        // And once the ROM stops driving it, it goes out — over a few tens of
        // milliseconds, because that is how long a filament takes to cool.
        for _ in 0..30 {
            sweep(&mut l, &mut t, 0b0000_0000);
        }
        assert!(l.level(1) < 0.01, "out: {}", l.level(1));
        assert!(!l.is_lit(1));
    }

    #[test]
    fn a_lamp_does_not_go_out_between_two_strobes_of_its_column() {
        // Seven eighths of every pass, a lit lamp has nothing across it. If
        // that showed, the whole playfield would strobe.
        let (mut l, mut t) = (LampMatrix::new(), 0.0);
        for _ in 0..60 {
            sweep(&mut l, &mut t, 0b0000_0001);
        }
        let mut low = f32::MAX;
        for _ in 0..8 {
            for column in 0..8u8 {
                let driven = if column == 0 { 0b0000_0001 } else { 0 };
                l.drive(t, driven, 1 << column);
                t += 0.001;
                l.integrate(t);
                low = low.min(l.level(1));
            }
        }
        assert!(low > 0.6, "it never dips: {low}");
    }

    #[test]
    fn the_moment_between_the_two_port_writes_lights_nothing() {
        // The ROM sets the rows and then moves the strobe, so for the handful
        // of microseconds in between, the new rows are driven into the *old*
        // column. Reading the matrix as a bitmap makes those lamps come on for
        // a whole frame; charging a filament for the microseconds it lasts does
        // not warm it enough to see.
        let (mut l, mut t) = (LampMatrix::new(), 0.0);
        let gap = 0.000_006; // a few 6808 cycles at 1 MHz
        for _ in 0..200 {
            for column in 0..8u8 {
                let driven = if column == 0 { 0b0000_0001 } else { 0 };
                // Rows first, still on the previous column.
                l.drive(
                    t,
                    driven,
                    if column == 0 {
                        0b1000_0000
                    } else {
                        1 << (column - 1)
                    },
                );
                t += gap;
                // Then the strobe moves.
                l.drive(t, driven, 1 << column);
                t += 0.001 - gap;
            }
        }
        assert!(l.is_lit(1), "the lamp that is really driven is on");
        let ghost = l.level(9); // column 2 row 1, driven only during the gaps
        assert!(ghost < 0.01, "and the one that is not stays dark: {ghost}");
    }

    /// The mux relay: while it is pulled in, the same eight drivers come out
    /// somewhere else entirely.
    #[test]
    fn the_mux_relay_moves_the_first_eight_to_the_second_bank() {
        let mut s = Solenoids::new();
        s.set_mux(14);

        // Relay out: solenoid 1 is solenoid 1.
        s.set_low(0.0, 0b0000_0001);
        assert_eq!(s.active(), 0b1, "solenoid 1 on the A side");

        // Pull the relay in — it is solenoid 14, so on the high byte.
        s.set_high(0.0, 1 << 5);
        let state = s.active();
        assert_eq!(
            state & 0xFF,
            0,
            "the A side goes dead while the relay is in: {state:#010x}"
        );
        assert_eq!(
            state & 0xFF00_0000,
            1 << 24,
            "and comes out as solenoid 25: {state:#010x}"
        );
        assert!(
            s.did_fire(25) || {
                s.refresh();
                s.did_fire(25)
            }
        );
        assert_eq!(
            state & 0x00FF_FF00,
            1 << 13,
            "the relay itself is untouched, and it is on the part it does not switch"
        );
    }

    /// A game with no mux relay never routes anything.
    #[test]
    fn without_a_mux_the_first_eight_stay_where_they_are() {
        let mut s = Solenoids::new();
        s.set_low(0.0, 0b0000_0001);
        s.set_high(0.0, 0xFF);
        assert_eq!(s.active() & 0xFF, 1, "solenoid 1 is still solenoid 1");
        assert_eq!(s.active() & 0xFF00_0000, 0, "and there is no second bank");
    }

    /// The six control lines do not reach the six special solenoids in order.
    ///
    /// The lines arrive PIA 1 CA2, PIA 1 CB2, PIA 3 CA2, PIA 3 CB2, PIA 4 CA2,
    /// PIA 4 CB2, and they land on solenoids 22, 21, 18, 19, 17, 20. Getting
    /// this wrong swaps a table's diverters for its slingshots, which is the
    /// kind of thing that looks like a physics bug.
    #[test]
    fn the_control_lines_land_where_the_wiring_puts_them() {
        for (line, expected) in [(0, 22), (1, 21), (2, 18), (3, 19), (4, 17), (5, 20)] {
            let mut s = Solenoids::new();
            s.set_special_enabled(0.0, true);
            // The lines are active low.
            s.set_special_line(0.0, line, false);
            assert!(
                s.did_fire(expected) || {
                    s.refresh();
                    s.did_fire(expected)
                },
                "line {line} should drive solenoid {expected}, and the state is {:#010x}",
                s.active()
            );
        }
    }

    /// A slingshot fires from its own switch, without the CPU.
    #[test]
    fn a_switch_can_fire_a_special_solenoid_on_its_own() {
        let mut s = Solenoids::new();
        s.set_special_enabled(0.0, true);
        s.set_special_switch(0.0, 0, true);
        assert!(s.active() & (1 << 16) != 0, "solenoid 17 from its switch");
        s.set_special_switch(0.0, 0, false);
        assert_eq!(s.active() & (1 << 16), 0, "and it lets go with the switch");
    }

    /// And none of them do anything while the board says the game is off.
    ///
    /// PIA 0's CB2 gates all seven — the six special solenoids and "game on" —
    /// which is what keeps a slingshot from firing at a ball rolling around in
    /// attract mode.
    #[test]
    fn nothing_special_fires_with_the_game_off() {
        let mut s = Solenoids::new();
        s.set_special_enabled(0.0, false);
        s.set_special_switch(0.0, 0, true);
        s.set_special_line(0.0, 4, false);
        assert_eq!(
            s.active(),
            0,
            "no special solenoid and no game-on while CB2 says off"
        );

        // And they all come to life together the moment it says on.
        s.set_special_enabled(0.0, true);
        assert!(
            s.did_fire(17) || {
                s.refresh();
                s.did_fire(17)
            }
        );
    }

    #[test]
    fn the_solenoids_remember_the_pulses() {
        let mut s = Solenoids::new();

        // A short pulse: it goes on and off within the same sweep.
        s.set_low(0.0, 0b0000_0100); // solenoid 3
        s.set_low(0.0, 0b0000_0000);

        assert_eq!(
            s.active(),
            0,
            "instantaneously there is nothing on any more"
        );
        s.refresh();
        assert!(s.did_fire(3), "but the firing was recorded");
        assert!(!s.did_fire(4));

        s.refresh();
        assert!(
            s.did_fire(3),
            "it is held for one more frame, so a coil the ROM keeps down does \
             not read as flapping when a frame catches none of its drive"
        );

        s.refresh();
        assert!(!s.did_fire(3), "and then it is gone");
    }

    #[test]
    fn the_high_solenoids_start_at_nine() {
        let mut s = Solenoids::new();
        s.set_high(0.0, 0b0000_0001); // first bit of the high group
        s.refresh();

        assert!(s.did_fire(9));
        assert!(!s.did_fire(1));
        assert_eq!(s.active(), 1 << 8);
    }
}
