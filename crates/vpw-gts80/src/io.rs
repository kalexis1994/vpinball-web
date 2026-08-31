//! What the RIOTs' ports are wired to.
//!
//! Everything a System 80 does to the outside world goes through six 8-bit
//! ports, and this is what each one carries. The wiring is Gottlieb's, read
//! off `gts80.c` — a description of a circuit board rather than a program.
//!
//! # The numbering, which is the whole difficulty
//!
//! A System 80's switch numbers look like the two-digit positions every other
//! family uses and are **not** read the same way. Gottlieb wired the matrix
//! with its rows and columns swapped relative to everyone else, so PinMAME
//! converts (`gts80.c:112`, whose comment is exactly "row and column is
//! swapped"): a switch numbered `n` is found at
//!
//! ```text
//! k = n + 1;  return line k % 10,  strobe line k / 10
//! ```
//!
//! which is why the start button is 47 and the first coin is 17 — both on
//! return line 8, strobes 4 and 1 — and why every table written against
//! `sys80.vbs` uses those numbers and no others. Getting this wrong does not
//! look like a numbering bug from outside: the game runs, the display counts,
//! and no switch a player closes ever reaches the ROM.

/// How many bytes of switch matrix there are.
///
/// Eight of them are the hardware's return lines and are read by the strobe.
/// Index 0 is not read that way — it is the coin door's slam switch, on a gate
/// of its own — and the rest are where a table's flipper switches (112 to 115
/// in `sys80.vbs`) land: numbers VPinMAME accepts and the ROM never sees.
pub const SWITCH_GROUPS: usize = 16;

/// Lamp columns. Port B of RIOT 2 selects one of twelve latches with its top
/// nibble and drives four lamps with its bottom one.
pub const LAMP_COLUMNS: usize = 12;

/// Where the slam switch sits in the matrix: `swSlamTilt = -1`.
const SLAM: u8 = 0x80;

/// The state of everything the board drives and everything it reads.
///
/// Kept apart from the board so a host can look at it, change a switch and put
/// it back without going through the chips — which is what a table's script
/// does on every hit.
#[derive(Debug, Clone)]
pub struct Io {
    /// The switch matrix, in PinMAME's own layout: `switches[return]`, bit =
    /// strobe. See the note at the top of this file for why that is the way
    /// round it is.
    pub switches: [u8; SWITCH_GROUPS],

    /// The lamp matrix: four lamps per latch, twelve latches, indexed by the
    /// column the hardware selects.
    ///
    /// Column 0 is not lamps at all: its bit 0 is the **game-on** relay and
    /// bit 1 the **tilt** relay, which is how a System 80 tells the playfield
    /// it is live. A table reads them as solenoids 10 and 11 — `sys80.vbs:20`
    /// names solenoid 10 `GameOnSolenoid`, and `vpmFlips` will not move a
    /// flipper without it.
    pub lamps: [u8; LAMP_COLUMNS],

    /// What the solenoid drivers are being told this instant.
    ///
    /// Nine coils, a bit each: the first eight are two banks of four selected
    /// by a pair of enable lines, which is why a System 80 can only fire one
    /// coil from each bank at a time, and the ninth has a pin to itself.
    pub live: u16,
    /// Everything driven since the last sweep.
    seen: u16,
    /// What the last sweep concluded, which is what anybody asking gets, with
    /// the two relays above it.
    reported: u16,

    /// Which of the coin door's switches are wired **normally closed**, as a
    /// mask of column 0.
    ///
    /// Only one bit is ever used and only by the last System 80B boards: the
    /// slam switch, which they wired the other way up from every machine
    /// before them (`gts80games.c`, the last argument of `INITGAME`). A game
    /// on the wrong side of this does not misbehave subtly — it puts
    /// `SLAM SWITCH CLOSED` on the glass and refuses to do anything at all.
    pub inverted: u8,

    /// The four banks of dip switches, as the operator set them.
    ///
    /// The settings a System 80 leaves the factory with, which is what the
    /// reference ships (`gts80.h`, `GTS80_DIPS`): S2, S9, S10, S16, S17, S18,
    /// S20, S21, S23, S24 and the whole of S25–S32.
    ///
    /// These used to be thirty-two zeros, on the reasoning that VPinMAME
    /// defaults its per-game settings that way and that the only thing a dip
    /// changes is how many coins a credit costs. The second half of that was
    /// wrong, and expensively: **with every switch off, Circus plays a whole
    /// game in silence.** The board never raises the sound strobe — bit 4 of
    /// the port it shares with the solenoids, the only bit of that port the
    /// firmware never touched — and the card is never asked for anything. Set
    /// them as the factory did and the same game sends command 0x05 and the
    /// card plays.
    ///
    /// Which is worth remembering the shape of: the machine booted, lit its
    /// displays, took a coin, started a game and ran a ball, and was silent,
    /// and nothing in the sound path was wrong.
    pub dips: [u8; 4],

    /// The last command written to the sound board, and whether it is new.
    pub sound_command: u8,
    /// Every bit the game board has ever put on the port the sound command
    /// shares, OR'd together. For asking whether firmware ever raises the
    /// strobe at all: a machine that plays nothing and a machine that is never
    /// asked look the same from the card.
    pub sound_port_seen: u8,
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
            switches: [0; SWITCH_GROUPS],
            lamps: [0; LAMP_COLUMNS],
            live: 0,
            seen: 0,
            reported: 0,
            inverted: 0,
            // S1-S8, S9-S16, S17-S24, S25-S32, each bank the way the port
            // reads it. See the field.
            dips: [0x02, 0x83, 0xDB, 0xFF],
            sound_command: 0,
            sound_port_seen: 0,
            sound_pending: false,
        }
    }

    /// Where a switch number lands in the matrix, as `(return line, strobe)`.
    ///
    /// `GTS80_sw2m` (`gts80.c:112`), which is three cases: the ordinary
    /// two-digit numbers with their digits swapped, the negative ones for the
    /// coin door, and the ones above 96 that VPinMAME invented for the
    /// flippers and reads the plain way round.
    fn place(number: i32) -> Option<(usize, u8)> {
        let m = if number >= 96 {
            (number / 10) * 8 + (number % 10 - 1)
        } else if number < 0 {
            number + 8
        } else {
            let k = number + 1;
            (k % 10) * 8 + k / 10
        };
        let m = usize::try_from(m).ok()?;
        (m / 8 < SWITCH_GROUPS).then_some((m / 8, 1u8 << (m % 8)))
    }

    /// Opens or closes one switch, by the number a table uses.
    pub fn set_switch(&mut self, number: i32, closed: bool) {
        let Some((group, bit)) = Self::place(number) else {
            return;
        };
        if closed {
            self.switches[group] |= bit;
        } else {
            self.switches[group] &= !bit;
        }
    }

    pub fn switch_closed(&self, number: i32) -> bool {
        Self::place(number).is_some_and(|(group, bit)| self.switches[group] & bit != 0)
    }

    /// Whether somebody is kicking the machine hard enough to trip the slam
    /// switch, which is wired to RIOT 1's port A and to nothing else.
    pub fn slammed(&self) -> bool {
        self.switches[0] & SLAM != 0
    }

    /// What the coin door's own switches put on RIOT 1's port A.
    ///
    /// All ones for a machine nobody is kicking — except on the boards that
    /// wired one of them normally closed, where that bit reads the other way
    /// (`gts80.c:141`).
    pub fn door_input(&self) -> u8 {
        if self.slammed() {
            self.inverted
        } else {
            self.inverted ^ 0xFF
        }
    }

    /// Whether a lamp is lit, by the number a table uses.
    ///
    /// `GTS80_lamp2m` is `no + 8` (`gts80.c:126`), and the board packs four
    /// lamps into each column — so lamp 0 is the *third* column's first bit,
    /// and the two columns before it are the relays and their neighbours.
    pub fn lamp_lit(&self, number: usize) -> bool {
        let m = number + 8;
        m / 4 < LAMP_COLUMNS && self.lamps[m / 4] & (1 << (m % 4)) != 0
    }

    /// Whether the game-on relay is pulled in, which is a System 80's way of
    /// saying a game is in progress.
    pub fn game_on(&self) -> bool {
        self.relays() & 0x01 != 0
    }

    /// And the tilt relay beside it.
    pub fn tilted(&self) -> bool {
        self.lamps[0] & 0x02 != 0
    }

    /// The two relays as the board reports them, which is not quite as it
    /// holds them.
    ///
    /// `GTS80_vblank` (`gts80.c:93`) folds tilt into game-on for a System 80
    /// and 80A: `gameOn &= (gameOn ^ 0x06) >> 1`, which reads as "if the tilt
    /// relay is in, the game-on relay is not". That is the wire that kills the
    /// flippers on a tilt, because `vpmFlips` polls solenoid 10.
    fn relays(&self) -> u16 {
        let both = u16::from(self.lamps[0] & 0x03);
        both & ((both ^ 0x06) >> 1)
    }

    /// The returns the switch matrix drives for a given strobe.
    ///
    /// One bit per return line, assembled from line 8 downwards because that
    /// is the order the port's bits are wired in. Return line 0 is not in the
    /// loop: it is the slam switch's gate and never reaches this port.
    pub(crate) fn switch_returns(&self, strobe: u8) -> u8 {
        let mut bits = 0u8;
        for line in (1..=8).rev() {
            bits <<= 1;
            if self.switches[line] & strobe != 0 {
                bits |= 1;
            }
        }
        bits
    }

    /// What RIOT 2's port A just put on the coil drivers.
    pub(crate) fn drive_solenoids(&mut self, coils: u16) {
        self.live = coils;
        self.seen |= coils;
    }

    /// Closes one sweep of the solenoid drivers.
    ///
    /// The board pulses a coil for a few milliseconds and a host looks once a
    /// frame, so what a host is told is not the instant but the window:
    /// everything driven since the last sweep, plus the two relays, which are
    /// levels rather than pulses and are read fresh (`gts80.c:91`).
    pub(crate) fn sweep(&mut self) {
        self.reported = self.seen | (self.relays() << 9);
        self.seen = self.live;
        self.live = 0;
    }

    /// Whether a coil was driven during the last sweep, numbered from 1.
    ///
    /// Nine coils and the two relays above them, which a table reads as
    /// solenoids 10 and 11.
    pub fn solenoid_on(&self, number: u8) -> bool {
        (1..=11).contains(&number) && self.reported & (1 << (number - 1)) != 0
    }

    /// Every coil the last sweep saw, as a bit set.
    pub fn solenoids(&self) -> u16 {
        self.reported
    }
}
