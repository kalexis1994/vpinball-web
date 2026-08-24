//! The bridge between a table's script and the machine's ROM.
//!
//! On a ROM table, the table's VBScript is not the game. The game is a program
//! burned into a chip in 1987, and the script is a switchboard: it reports
//! every switch the ball closes to the board, and copies back whatever the
//! board does with its lamps and its solenoids. F-14's script is eleven hundred
//! lines and almost none of it is rules — the rules are in `f14_u26.l1`.
//!
//! `vpw-s11` already emulates that board: three CPUs, six PIAs, the switch and
//! lamp matrices, the solenoids and the displays. What was missing was this —
//! the object the script calls `Controller`.
//!
//! # What the script expects
//!
//! ```vbscript
//! Set Controller = CreateObject("VPinMAME.Controller")
//! Controller.GameName = "f14_l1"
//! Controller.Run
//! ...
//! Controller.Switch(16) = 1          ' the ball crossed a rollover
//! For Each x In Controller.ChangedLamps  ' and what came back
//! ```
//!
//! `ChangedLamps` and `ChangedSolenoids` are the interesting half: they return
//! a two-dimensional array of `(number, state)` for everything that changed
//! **since the last time they were read**, and reading them clears the record.
//! A table polls them from a timer, twenty or sixty times a second. Getting the
//! "since last read" part wrong is how a port ends up with lamps that latch on
//! and never go off.
//!
//! # Where it departs from the original
//!
//! VPinMAME runs the emulated CPU on its own thread and the table polls
//! whatever state it has reached. Here it is stepped from the game loop, one
//! millisecond of board time per millisecond of table time, so the whole thing
//! stays deterministic: same inputs, same game, every time. That is worth more
//! than the parallelism on a machine that has to run the physics anyway, and it
//! is the only way the ROM can take part in a reproducible test.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use vpw_s11::System11;
use vpw_vbscript::error::{Error as VbError, Result as VbResult};
use vpw_vbscript::object::Object;
use vpw_vbscript::value::{Array, Value};

/// Where the ROM images come from.
///
/// A `.vpx` does not contain them — they are the machine's copyrighted
/// firmware and are shipped separately, as a zip named after the set. The host
/// is asked for the zip's bytes by name, which is the same shape as
/// [`crate::ScriptLibrary`] and for the same reason: in a browser there is no
/// directory to look in.
pub trait RomSource {
    /// The bytes of `<set>.zip`, or `None` if it is not available.
    fn read(&self, set: &str) -> Option<Vec<u8>>;

    /// The machine's saved settings, audits and high scores, if there are any.
    ///
    /// This lives next to the ROM because it belongs to the same thing — the
    /// machine, not the table — and because a host that can find one can find
    /// the other. Answering `None` gives the player a factory machine.
    fn cmos(&self, _set: &str) -> Option<Vec<u8>> {
        None
    }
}

/// A ROM source that has nothing. A table using it plays without its rules.
pub struct NoRoms;

impl RomSource for NoRoms {
    fn read(&self, _set: &str) -> Option<Vec<u8>> {
        None
    }
}

/// A ROM source backed by a directory on disk. Not available on wasm.
#[cfg(not(target_arch = "wasm32"))]
pub struct RomDir(pub std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl RomSource for RomDir {
    fn read(&self, set: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(format!("{set}.zip"))).ok()
    }

    /// `<set>.nv`, the same name VPinMAME saves it under.
    fn cmos(&self, set: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(format!("{set}.nv"))).ok()
    }
}

/// How many lamps a System 11 has: an eight by eight matrix.
/// The most lamps any board here drives.
///
/// A System 11 has sixty-four; a Whitestar has a hundred and twenty-eight, in
/// sixteen columns rather than eight.
const LAMPS: usize = 128;

/// How many display digits a table can ask about.
///
/// Sixty-four is the width of the low mask a script passes to `ChangedLEDs`.
/// A System 11 fills at most thirty-two of them — two rows of sixteen — and the
/// rest never change, which is the same answer the original gives.
const SEGMENTS: usize = 64;

/// Every solenoid number a script is told about, in order.
///
/// The thirty-two real driver outputs, and then the four flipper ones the
/// hardware raises from the cabinet buttons without the CPU ever seeing them
/// ([`vpw_s11::io::FIRST_FLIPPER`]). There is nothing between 33 and 44 on this
/// machine, so the run is not contiguous and the position in this sequence —
/// not the number — is what indexes the record of what the script has seen.
fn solenoid_numbers() -> impl Iterator<Item = u8> {
    (1..=SOLENOIDS as u8).chain(FLIPPER_SOLENOIDS)
}

/// The flipper solenoids, which come after the driver outputs.
const FLIPPER_SOLENOIDS: std::ops::RangeInclusive<u8> = 45..=48;

/// How many solenoid outputs it has.
///
/// Thirty-two, and the last eight are not extra hardware: 1-8 come from the
/// `$2200` latch, 9-16 from PIA 0's port B, 17-22 are the special ones driven
/// off the PIAs' control lines or straight from a switch, 23 says a game is in
/// progress, and 25-32 are those same first eight drivers seen through the
/// A/C mux relay. See [`vpw_s11::io::Solenoids`].
const SOLENOIDS: usize = vpw_s11::io::SOLENOID_COUNT;

/// The number the script knows the `i`th lamp by.
///
/// **Straight through from 1**, not by column and row. Which of the two it is
/// depends on the machine, and PinMAME settles it per driver: the number comes
/// out of `coreData->m2lamp` (`vpintf.c:96`). WPC installs its own
/// (`wpc_m2sw`, `wpc.c:187`) and gets `col*10 + row + 1`, the familiar 11 to 88
/// with gaps. A System 11 installs nothing, so it keeps the default the core
/// machine driver puts there (`core.c:3997`), which is `core_m2swSeq`:
///
/// ```text
/// int core_m2swSeq(int col, int row) { return col*8+row-7; }   // core.c:2109
/// ```
///
/// with a one-based column — that is, `i + 1`. F-14's own script agrees: it
/// fades `l1` through `l64` by those numbers and knows nothing of 11-88.
///
/// Using the WPC formula here cost the table both ends at once. The board's own
/// lookup only answers for 1 to 64 ([`vpw_s11::matrix_position`]), so the last
/// two columns —numbers 71 to 88— asked about a lamp that does not exist, came
/// back dark every time, and therefore never *changed*: sixteen bulbs the
/// script was never told about. The other forty-eight were reported under a
/// number belonging to a different bulb, and nothing under 11 was ever reported
/// at all.
fn lamp_number(i: usize) -> u8 {
    i as u8 + 1
}

/// The emulated machine, and what the script has seen of it.
/// The machine's board, whichever kind of machine it is.
///
/// Two families so far, and they have less in common than the shape of this
/// enum suggests. A System 11 talks to the world through PIAs and drives a
/// segment display; a Whitestar has no PIAs at all, twice the lamps, and a dot
/// matrix on a board of its own. What they share is what a table's script asks
/// for — lamps, switches, coils — so that is what this offers and the rest
/// answers for the family that has it.
enum Hardware {
    S11(Box<System11>),
    Whitestar(Box<vpw_ws::Whitestar>),
}

impl Hardware {
    fn run_seconds(&mut self, seconds: f64) {
        match self {
            Self::S11(b) => {
                b.run_seconds(seconds);
            }
            Self::Whitestar(b) => {
                b.run_seconds(seconds);
            }
        }
    }

    fn drain_audio(&mut self) -> Vec<f32> {
        match self {
            Self::S11(b) => b.drain_audio(),
            // The Whitestar's sound board is a processor of its own, running
            // its own firmware out of the game's ROMs, and its output comes
            // out at its rate rather than ours. See [`vpw_ws::sound`].
            Self::Whitestar(b) => b.take_audio_at(vpw_s11::DEFAULT_AUDIO_RATE),
        }
    }

    fn cmos(&self) -> Vec<u8> {
        match self {
            Self::S11(b) => b.cmos().to_vec(),
            Self::Whitestar(b) => b.board.ram().to_vec(),
        }
    }

    fn lamp_lit(&self, number: u8) -> bool {
        match self {
            Self::S11(b) => b.lamp_lit(number),
            Self::Whitestar(b) => b.board.lamps.is_lit(number),
        }
    }

    fn lamp_level(&self, number: u8) -> f32 {
        match self {
            Self::S11(b) => b.board.lamps.level(number),
            // No filament model here yet. System 11's is worth bringing over —
            // the original oversamples a Whitestar's lamps twice a frame
            // *because of* Lord of the Rings, whose modes flicker their lamps
            // as part of the artwork.
            Self::Whitestar(b) => f32::from(u8::from(b.board.lamps.is_lit(number))),
        }
    }

    fn switch_closed(&self, number: u8) -> bool {
        match self {
            Self::S11(b) => b.switch_closed(number),
            Self::Whitestar(b) => b.board.switches.is_closed(number),
        }
    }

    fn set_switch(&mut self, number: u8, closed: bool) -> bool {
        match self {
            Self::S11(b) => b.set_switch(number, closed),
            Self::Whitestar(b) => b.board.switches.set(number, closed),
        }
    }

    fn solenoids_active(&self) -> u32 {
        match self {
            Self::S11(b) => b.solenoids_active(),
            Self::Whitestar(b) => b.board.solenoids.live(),
        }
    }

    fn solenoid_fired(&self, number: u8) -> bool {
        match self {
            Self::S11(b) => b.solenoid_fired(number),
            Self::Whitestar(b) => b.board.solenoids.is_on(number),
        }
    }

    /// The two buttons behind the coin door, which every family has and each
    /// wires its own way.
    fn diagnostic_up(&self) -> bool {
        match self {
            Self::S11(b) => b.board.diagnostic_up,
            // Whitestar reads them as two of the dedicated switches rather than
            // through a PIA. Bits 5 and 6 of `$3000`, active low.
            Self::Whitestar(b) => b.board.switches.dedicated() & (1 << 6) == 0,
        }
    }

    fn diagnostic_advance(&self) -> bool {
        match self {
            Self::S11(b) => b.board.diagnostic_advance,
            Self::Whitestar(b) => b.board.switches.dedicated() & (1 << 5) == 0,
        }
    }

    fn set_diagnostic_up(&mut self, closed: bool) {
        match self {
            Self::S11(b) => b.set_diagnostic_up(closed),
            Self::Whitestar(b) => b.board.switches.set_dedicated(6, closed),
        }
    }

    fn set_diagnostic_advance(&mut self, closed: bool) {
        match self {
            Self::S11(b) => b.set_diagnostic_advance(closed),
            Self::Whitestar(b) => b.board.switches.set_dedicated(5, closed),
        }
    }

    /// The general illumination, as a level from zero to one per string.
    ///
    /// One string on a Whitestar (`se.c:314` sets `nGI = 1`), driven by bit 0
    /// of the strobe latch at `$200b` — the GI relay. A System 11 runs its
    /// general illumination off the same lamp hardware and has nothing separate
    /// to report.
    fn gi(&self) -> Vec<f32> {
        match self {
            Self::S11(_) => Vec::new(),
            Self::Whitestar(b) => vec![f32::from(u8::from(b.board.gi[2] & 1 != 0))],
        }
    }

    /// The dot matrix, one byte per dot from 0 to 3, if there is one.
    fn dmd(&self) -> Option<&[u8]> {
        match self {
            Self::S11(_) => None,
            Self::Whitestar(b) => b.dmd_frame(),
        }
    }

    fn segments(&self) -> Vec<u16> {
        match self {
            Self::S11(b) => b.segments(),
            // A Whitestar's score is a dot matrix driven by a second processor,
            // which is not here. An empty answer is the honest one: the script
            // asks what changed and nothing has.
            Self::Whitestar(_) => Vec::new(),
        }
    }
}

pub struct Machine {
    board: RefCell<Hardware>,
    /// The set that is loaded, once one is.
    /// The set that is loaded. Only its name is ever wanted, and the two
    /// families describe a game with different types, so the name is what is
    /// kept.
    game: Cell<Option<&'static str>>,
    running: Cell<bool>,
    /// What the lamps and solenoids looked like when the script last asked.
    /// The difference against the board is what `ChangedLamps` reports.
    seen_lamps: RefCell<[bool; LAMPS]>,
    /// The segments each display digit had the last time the script asked. See
    /// [`Machine::changed_leds`].
    seen_segments: RefCell<[u16; SEGMENTS]>,
    seen_solenoids: RefCell<[bool; SOLENOIDS + 4]>,
    /// What the general illumination looked like when the script last asked.
    seen_gi: RefCell<Vec<bool>>,
    /// Board time owed, in seconds. The board runs at 1 MHz and the game loop
    /// hands it a millisecond at a time.
    owed: Cell<f64>,
}

impl Machine {
    pub fn new() -> Self {
        Self {
            board: RefCell::new(Hardware::S11(Box::new(System11::new()))),
            game: Cell::new(None),
            running: Cell::new(false),
            seen_lamps: RefCell::new([false; LAMPS]),
            seen_segments: RefCell::new([0; SEGMENTS]),
            seen_solenoids: RefCell::new([false; SOLENOIDS + 4]),
            seen_gi: RefCell::new(Vec::new()),
            owed: Cell::new(0.0),
        }
    }

    /// Whether a ROM is loaded and running.
    pub fn is_running(&self) -> bool {
        self.running.get()
    }

    /// The set that is loaded.
    pub fn game_name(&self) -> Option<&'static str> {
        self.game.get()
    }

    /// Builds the machine for a named set from a zip.
    ///
    /// The layout — where each image goes in the 64 KB map — comes from the
    /// manifest, because it differs by family and getting it wrong runs the CPU
    /// on garbage rather than failing.
    pub fn load(&self, set: &str, zip: &[u8], cmos: Option<&[u8]>) -> Result<(), String> {
        // The name is checked before the zip is opened: "that is not a set
        // this emulator knows" is a better answer than "that is not a zip",
        // and for an unknown set it is also the true one.
        enum Which {
            S11(vpw_s11::games::Game),
            Whitestar(vpw_ws::games::Game),
        }
        let which = if let Some(game) = vpw_s11::games::find(set) {
            Which::S11(game)
        } else if let Some(game) = vpw_ws::games::find(set) {
            Which::Whitestar(game)
        } else {
            return Err(format!("'{set}' is not a set this emulator knows"));
        };

        let images = read_zip(zip)?;
        let name = match which {
            Which::S11(game) => self.load_s11(game, set, &images, cmos)?,
            Which::Whitestar(game) => self.load_whitestar(game, &images, cmos)?,
        };

        self.game.set(Some(name));
        self.running.set(true);
        self.owed.set(0.0);
        *self.seen_lamps.borrow_mut() = [false; LAMPS];
        *self.seen_solenoids.borrow_mut() = [false; SOLENOIDS + 4];
        *self.seen_segments.borrow_mut() = [0; SEGMENTS];
        self.seen_gi.borrow_mut().clear();
        Ok(())
    }

    /// Sega and Stern's Whitestar. See [`vpw_ws`].
    ///
    /// Simpler to build than a System 11 because there is less of it: one CPU
    /// image and no wiring table. The display and the sound are separate boards
    /// and neither is emulated, so their images are read from the zip and left
    /// there.
    fn load_whitestar(
        &self,
        game: vpw_ws::games::Game,
        images: &[(String, Vec<u8>)],
        cmos: Option<&[u8]>,
    ) -> Result<&'static str, String> {
        let cpu = pick_like(images, game.cpu)
            .ok_or_else(|| format!("'{}' is not in the zip", game.cpu))?;

        let mut machine = vpw_ws::Whitestar::new();
        machine.load_rom(cpu)?;
        // The display is a board of its own with its own processor and its own
        // half-megabyte ROM. A set without it still plays; it just has nothing
        // to say for itself.
        if let Some(display) = pick_like(images, game.display) {
            machine.load_display_rom(display)?;
        }
        // The sound board is a third processor with five images of its own. A
        // set without them plays silently rather than not at all, which is
        // what a machine with the sound board unplugged does.
        let sound = game.sound;
        let images_for = |name: &str| pick_like(images, name);
        match (
            images_for(sound.bios),
            images_for(sound.u7),
            sound.samples.map(images_for),
        ) {
            (Some(bios), Some(u7), [Some(a), Some(b), Some(c), Some(d)]) => {
                machine.load_sound(bios, u7, [a, b, c, d]);
            }
            _ => log::warn!("{}: the sound board's images are not all here", game.set),
        }
        if let Some(saved) = cmos {
            let ram = machine.board.ram_mut();
            if saved.len() != ram.len() {
                return Err(format!(
                    "the saved settings are {} bytes and the machine's memory is {}",
                    saved.len(),
                    ram.len()
                ));
            }
            ram.copy_from_slice(saved);
        }
        *self.board.borrow_mut() = Hardware::Whitestar(Box::new(machine));
        Ok(game.set)
    }

    fn load_s11(
        &self,
        game: vpw_s11::games::Game,
        set: &str,
        images: &[(String, Vec<u8>)],
        cmos: Option<&[u8]>,
    ) -> Result<&'static str, String> {
        *self.board.borrow_mut() = Hardware::S11(Box::new(System11::new()));

        let first = pick(images, game.image1)
            .ok_or_else(|| format!("'{}' is not in the zip", game.image1))?;
        let second = match game.image2 {
            Some(name) => {
                Some(pick(images, name).ok_or_else(|| format!("'{name}' is not in the zip"))?)
            }
            None => None,
        };
        let placed = game
            .layout
            .place(first, second)
            .ok_or_else(|| format!("'{set}' needs two images and the zip has one"))?;

        let mut hardware = self.board.borrow_mut();
        let Hardware::S11(board) = &mut *hardware else {
            unreachable!("just built a System 11")
        };
        board
            .load_cpu_region(&placed)
            .map_err(|e| format!("the images do not fit the map: {e}"))?;

        // The two sound boards are optional: a set without them still plays,
        // silently.
        if let (Some(u21), Some(u22)) = (pick_like(images, "u21"), pick_like(images, "u22")) {
            board.load_sound_roms(u21, u22);
        }
        if let (Some(u4), Some(u19)) = (pick_like(images, "u4"), pick_like(images, "u19")) {
            board.load_cs_sound_roms(&[u4, u19]);
        }

        board.set_display_config(game.display);
        // How this game's board is wired beyond what every System 11 shares:
        // the mux relay and the switches that fire special solenoids directly.
        let wiring = vpw_s11::games::wiring(set);
        if let Some(mux) = wiring.mux {
            board.set_mux_solenoid(mux);
        }
        board.set_special_switches(wiring.special_switches);
        board.set_flipper_buttons(wiring.flipper_buttons);

        // Which of the solenoid outputs are bulbs rather than coils.
        for &(number, drive) in vpw_s11::games::bulb_outputs(set) {
            board.set_solenoid_bulb(number, drive);
        }
        board.reset();
        if let Some(saved) = cmos {
            if !board.load_cmos(saved) {
                return Err(format!(
                    "the saved settings are {} bytes and the machine's memory is {}",
                    saved.len(),
                    board.cmos().len()
                ));
            }
        } else {
            prime(board);
        }
        drop(hardware);
        Ok(game.set)
    }

    /// The sound board's output since the last time this was asked: mono, at
    /// [`vpw_s11::DEFAULT_AUDIO_RATE`].
    ///
    /// It has to be drained, not sampled. The board writes one sample per tick
    /// of its own clock as it runs, so what is here is exactly the audio
    /// belonging to the table time that has passed — reading it late is fine,
    /// but not reading it lets the buffer grow for as long as the table runs.
    pub fn take_audio(&self) -> Vec<f32> {
        if !self.running.get() {
            return Vec::new();
        }
        self.board.borrow_mut().drain_audio()
    }

    /// The machine's battery-backed memory: settings, audits, high scores.
    ///
    /// A host should save this when the player leaves and hand it back to
    /// [`Machine::load`] next time. Without it every session starts on a
    /// factory machine with no high scores — playable, but not a pinball
    /// machine anybody would recognise as theirs.
    pub fn cmos(&self) -> Vec<u8> {
        self.board.borrow().cmos().to_vec()
    }

    /// Runs the board for a slice of table time.
    ///
    /// The board's clock and the table's are the same clock: one millisecond of
    /// table is one millisecond of board. The remainder is carried rather than
    /// dropped, or the ROM would drift slower than the table and a timed mode
    /// would last longer than it should.
    pub fn advance(&self, seconds: f64) {
        if !self.running.get() {
            return;
        }
        let owed = self.owed.get() + seconds;
        // Running less than a microsecond at a time costs more in bookkeeping
        // than it advances.
        if owed < 1e-6 {
            self.owed.set(owed);
            return;
        }
        self.owed.set(0.0);
        self.board.borrow_mut().run_seconds(owed);
    }

    /// PIA 1's port A direction register, and the raw output register.
    pub fn lamp_ddr(&self) -> (u8, u8) {
        match &*self.board.borrow() {
            Hardware::S11(b) => {
                let p = &b.board.pias[vpw_s11::pia::LAMPS];
                (p.port_a_direction(), p.port_a_output())
            }
            // A Whitestar has no PIAs at all. Nothing to report and nothing
            // that reads this cares which board it is talking to.
            Hardware::Whitestar(_) => (0, 0),
        }
    }

    /// How brightly a lamp is showing, 0 to 1. See [`vpw_s11::io::LampMatrix`]:
    /// the ROM dims a lamp by driving it on some sweeps and not others, so this
    /// is a level and not a switch.
    pub fn lamp_level(&self, number: u8) -> f32 {
        self.board.borrow().lamp_level(number)
    }

    /// The lamp matrix as it stands, by column.
    pub fn lamp_columns(&self) -> [u8; 8] {
        match &*self.board.borrow() {
            Hardware::S11(b) => b.board.lamps.columns(),
            // Only the low eight of its sixteen. What reads this is the
            // diagnostic view, which was drawn for a System 11.
            Hardware::Whitestar(b) => {
                let m = b.board.lamps.matrix();
                [m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7]]
            }
        }
    }

    /// What PIA 1 is driving: `(rows, columns)`.
    pub fn lamp_drive(&self) -> (u8, u8) {
        match &*self.board.borrow() {
            Hardware::S11(b) => b.board.lamp_drive(),
            Hardware::Whitestar(b) => ((b.board.lamps.column() & 0xff) as u8, b.board.lamps.rows()),
        }
    }

    /// Whether a lamp is lit right now, without consuming the change record.
    pub fn lamp_lit(&self, number: u8) -> bool {
        self.board.borrow().lamp_lit(number)
    }

    /// Whether a playfield switch is closed. For a host or a test that wants
    /// to see what the script told the board.
    pub fn switch_closed(&self, number: u8) -> bool {
        self.board.borrow().switch_closed(number)
    }

    /// The whole switch matrix at once, switch *n* in bit *n-1*.
    ///
    /// One borrow for sixty-four switches rather than sixty-four borrows: the
    /// telemetry asks for this every millisecond, and the point of a rolling
    /// record is that carrying it costs almost nothing.
    pub fn switch_matrix(&self) -> u64 {
        let board = self.board.borrow();
        let mut bits = 0u64;
        for number in 1..=64u8 {
            if board.switch_closed(number) {
                bits |= 1 << (number - 1);
            }
        }
        bits
    }

    /// Whether a solenoid fired during the board's last sweep.
    ///
    /// Unlike [`Machine::changed_solenoids`] this does not consume anything, so
    /// a host or a test can watch the coils without stealing them from the
    /// script that is supposed to be reading them.
    pub fn solenoid_fired(&self, number: u8) -> bool {
        self.board.borrow().solenoid_fired(number)
    }

    /// What the solenoid drivers are being told this instant, unsmoothed.
    pub fn solenoids_active(&self) -> u32 {
        self.board.borrow().solenoids_active()
    }

    /// Whether the special solenoids are enabled, for diagnostics.
    pub fn special_enabled(&self) -> bool {
        match &*self.board.borrow() {
            Hardware::S11(b) => b.board.solenoids.special_enabled(),
            // A Whitestar drives all thirty-two of its coils directly; there is
            // no special bank and nothing to enable.
            Hardware::Whitestar(_) => false,
        }
    }

    /// Every display digit whose segments changed since the last time this was
    /// asked, as `(digit, which segments changed, what they are now)`.
    ///
    /// This is how a score gets onto the glass. A table does not draw the
    /// display itself: it carries a light for every segment of every digit —
    /// F-14 has four hundred and forty of them — and a handler that turns them
    /// on and off from what this returns. So the score appears wherever the
    /// table's author put it, drawn in the table's own artwork, and nothing in
    /// the player needs to know what a seven-segment display looks like.
    ///
    /// The digits are numbered the way the original numbers them for a table,
    /// which is not the way the hardware does. The board multiplexes sixteen
    /// slots and a System 11A only fills fourteen of them, in two groups of
    /// seven with a gap; what a table sees is those packed, top row then bottom
    /// row, nought upwards. See [`vpw_s11::display::visible_slots`].
    ///
    /// `mask` selects digits 0 to 63 and `mask2` the next sixty-four, which no
    /// System 11 reaches.
    pub fn changed_leds(&self, mask: u64, mask2: u64) -> Vec<(u8, u16, u16)> {
        let _ = mask2;
        let now = self.board.borrow().segments();
        let mut seen = self.seen_segments.borrow_mut();
        let mut rows = Vec::new();
        for digit in 0..SEGMENTS.min(now.len()) {
            if mask & (1 << digit) == 0 {
                continue;
            }
            let changed = now[digit] ^ seen[digit];
            if changed != 0 {
                seen[digit] = now[digit];
                rows.push((digit as u8, changed, now[digit]));
            }
        }
        rows
    }

    /// Every visible digit's segments, both rows, one after the other.
    ///
    /// The raw bits rather than the text: a host that is *drawing* the display
    /// wants the strokes, and turning them into characters and back would lose
    /// exactly the ones that are not characters — a digit with two strokes lit
    /// mid-change spells nothing, and on the glass it is what you see.
    pub fn segments(&self) -> Vec<u16> {
        self.board.borrow().segments()
    }

    /// The two score displays, for a host that wants to draw them.
    pub fn displays(&self) -> (String, String) {
        let b = self.board.borrow();
        match &*b {
            Hardware::S11(b) => (b.upper_display(), b.lower_display()),
            // The dot matrix is a board of its own and is not here yet.
            Hardware::Whitestar(_) => (String::new(), String::new()),
        }
    }

    /// Every lamp that changed since the last time this was asked.
    ///
    /// Reading **clears** the record, which is the contract the script is
    /// written against: a table polls this from a timer and expects each change
    /// once. Reporting a change twice makes the playfield flicker; never
    /// clearing it makes every lamp latch on and stay on.
    ///
    /// For the numbering, which is not the WPC one, see [`lamp_number`].
    /// The machine's dot matrix, one byte per dot from 0 (dark) to 3 (full),
    /// and how wide it is. Empty on a machine whose score is segments.
    pub fn dmd(&self) -> (Vec<u8>, usize, usize) {
        match self.board.borrow().dmd() {
            Some(f) => (f.to_vec(), vpw_ws::dmd::WIDTH, vpw_ws::dmd::HEIGHT),
            None => (Vec::new(), 0, 0),
        }
    }

    /// The last sound command the board sent its sound board.
    ///
    /// The sound board is not emulated for every family, but *that a command
    /// was sent* is the cheapest evidence there is that the game's rules are
    /// running: a machine sends one for every bumper, every target and every
    /// mode, and a machine that is merely powered sends none.
    pub fn sound_latch(&self) -> u8 {
        match &*self.board.borrow() {
            Hardware::S11(b) => b.board.sound_latch(),
            Hardware::Whitestar(b) => b.board.sound_latch,
        }
    }

    /// The general illumination strings that have changed since last asked.
    ///
    /// A table polls this every time it polls the lamps, and what it does with
    /// the answer is `UBound` — so the answer has to be an array. Returning
    /// anything else is not a quiet degradation: `core.vbs:2488` raises a type
    /// mismatch, that aborts the whole of `PinMameTimer`, and everything the
    /// rest of that handler would have done — including dispatching the
    /// solenoid callbacks that serve a ball — silently does not happen. The
    /// machine lights up, knocks a few times and never gives you a ball.
    pub fn changed_gis(&self) -> Vec<(u8, bool)> {
        let now = self.board.borrow().gi();
        let mut seen = self.seen_gi.borrow_mut();
        seen.resize(now.len(), false);
        let mut rows = Vec::new();
        for (i, level) in now.iter().enumerate() {
            let on = *level >= 0.5;
            if on != seen[i] {
                seen[i] = on;
                rows.push((i as u8, on));
            }
        }
        rows
    }

    pub fn changed_lamps(&self) -> Vec<(u8, bool)> {
        let board = self.board.borrow();
        let mut seen = self.seen_lamps.borrow_mut();
        let mut rows = Vec::new();
        // Every lamp the widest board here has. A System 11's sixty-four are
        // the first sixty-four of them and the rest simply never light, which
        // costs one comparison each and keeps this from having to know which
        // machine it is reporting on.
        for i in 0..LAMPS {
            let number = lamp_number(i);
            let now = board.lamp_lit(number);
            if now != seen[i] {
                seen[i] = now;
                rows.push((number, now));
            }
        }
        rows
    }

    /// Every solenoid that fired or released since the last time this was
    /// asked. Same contract as [`Machine::changed_lamps`].
    ///
    /// These are numbered straight through from 1, not by column and row: the
    /// solenoids are not a matrix.
    ///
    /// The four at the end are the flipper ones, 45 to 48, which no ROM drives
    /// and which a table's script cannot do without — see
    /// [`vpw_s11::io::FIRST_FLIPPER`]. They are reported here rather than
    /// somewhere of their own because to a script they are solenoids like any
    /// other: `SolCallback(sLLFlipper) = "SolLFlipper"`.
    pub fn changed_solenoids(&self) -> Vec<(u8, bool)> {
        let board = self.board.borrow();
        let mut seen = self.seen_solenoids.borrow_mut();
        let mut rows = Vec::new();
        for (i, number) in solenoid_numbers().enumerate() {
            let now = board.solenoid_fired(number);
            if now != seen[i] {
                seen[i] = now;
                rows.push((number, now));
            }
        }
        rows
    }
}

/// Gets a machine with no saved settings past its "FACTORY SETTING" screen.
///
/// A System 11 whose battery-backed memory does not checksum refuses to play:
/// it shows `FACTORY SETTING` and waits for the operator. Its own boot code
/// writes a valid set of defaults on the way there, so the *second* power-up
/// finds them and goes straight to attract mode. That is exactly what a
/// VPinMAME player is told to do by hand on a table's first run — press F3 —
/// and there is no reason to make anybody do it.
///
/// So: run long enough for the ROM to write its defaults, then power-cycle.
/// Nothing about the machine is faked; it is switched off and on again.
fn prime(board: &mut System11) {
    // Measured on F-14: the message is up by the second second. The loop stops
    // as soon as it sees it rather than always paying for the worst case, and
    // gives up after eight in case a set does not use the message at all.
    for _ in 0..8 {
        board.run_seconds(1.0);
        if board.upper_display().contains("ETTING") {
            break;
        }
    }
    board.reset();
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

/// `Controller`: VPinMAME, as the script sees it.
pub struct Controller {
    pub machine: Rc<Machine>,
    pub roms: Rc<dyn RomSource>,
    /// What the script set `GameName` to, before it said `Run`.
    pending: RefCell<String>,
    /// Why the ROM could not be started, if it could not.
    pub failure: RefCell<Option<String>>,
}

impl Controller {
    pub fn new(machine: Rc<Machine>, roms: Rc<dyn RomSource>) -> Self {
        Self {
            machine,
            roms,
            pending: RefCell::new(String::new()),
            failure: RefCell::new(None),
        }
    }

    /// `Controller.Run`: find the ROM and start the board.
    fn run(&self) -> VbResult<Value> {
        let set = self.pending.borrow().clone();
        if set.is_empty() {
            return Err(VbError::new(5, "the game name was never set"));
        }
        let Some(zip) = self.roms.read(&set) else {
            let why = format!("no ROM available for '{set}'");
            log::warn!("{why}");
            *self.failure.borrow_mut() = Some(why.clone());
            // The script checks `Err` right after `Run` and reports it to the
            // player, which is the right thing to happen: the table says it
            // cannot start, and that is the truth.
            return Err(VbError::new(5, why));
        };
        match self
            .machine
            .load(&set, &zip, self.roms.cmos(&set).as_deref())
        {
            Ok(()) => {
                log::info!("ROM '{set}' running");
                *self.failure.borrow_mut() = None;
                Ok(Value::Empty)
            }
            Err(why) => {
                log::warn!("ROM '{set}': {why}");
                *self.failure.borrow_mut() = Some(why.clone());
                Err(VbError::new(5, why))
            }
        }
    }

    fn number(args: &[Value]) -> VbResult<i32> {
        args.first().ok_or_else(VbError::invalid_call)?.to_int()
    }

    /// Reads a switch by the number a script uses.
    ///
    /// Negative numbers are the coin door — see [`Door`].
    fn switch(&self, number: i32) -> bool {
        let board = self.machine.board.borrow();
        match Door::of(number) {
            Some(Door::Up) => board.diagnostic_up(),
            Some(Door::Advance) => board.diagnostic_advance(),
            Some(_) => false,
            None => u8::try_from(number).is_ok_and(|n| board.switch_closed(n)),
        }
    }
}

impl Object for Controller {
    fn type_name(&self) -> &'static str {
        "Controller"
    }

    fn get(&self, name: &str, args: &[Value]) -> VbResult<Value> {
        match &*name.to_ascii_lowercase() {
            "gamename" => Ok(Value::str(self.pending.borrow().as_str())),
            "run" => self.run(),
            "stop" => Ok(Value::Empty),
            "pause" => Ok(Value::Bool(false)),
            "running" => Ok(Value::Bool(self.machine.is_running())),
            "version" => Ok(Value::str("03060000")),

            // The switch matrix, read and written by number.
            "switch" => Ok(Value::Bool(self.switch(Self::number(args)?))),
            "lamp" => {
                let n = u8::try_from(Self::number(args)?).unwrap_or(0);
                Ok(Value::Bool(self.machine.board.borrow().lamp_lit(n)))
            }
            "solenoid" => {
                let n = u8::try_from(Self::number(args)?).unwrap_or(0);
                Ok(Value::Bool(self.machine.board.borrow().solenoid_fired(n)))
            }

            // What the board has done since the script last looked.
            "changedlamps" => Ok(two_column_array(&self.machine.changed_lamps())),
            "changedsolenoids" => Ok(two_column_array(&self.machine.changed_solenoids())),
            // Both spellings. `core.vbs:2436` asks for `ChangedGIStrings`;
            // older tables ask for `ChangedGIs`. Answering only one of them
            // leaves the other falling through to the chainable stand-in,
            // which is an object rather than an array and blows up on
            // `UBound` — see [`Machine::changed_gis`] for what that costs.
            "changedgis" | "changedgistrings" => Ok(two_column_array(&self.machine.changed_gis())),
            "changedleds" => {
                // `ChangedLEDs(nHigh, nLow, [nnHigh], [nnLow])`, two 32-bit
                // halves per mask and the second mask defaulting to zero
                // (`VPinMAME.idl:214`). A table passes `&Hffffffff, &Hffffffff`
                // and means "all of them".
                let word = |hi: usize, lo: usize| -> u64 {
                    let part = |i: usize| {
                        args.get(i)
                            .and_then(|v| v.to_int().ok())
                            .map_or(0u64, |n| u64::from(n as u32))
                    };
                    (part(hi) << 32) | part(lo)
                };
                Ok(three_column_array(
                    &self.machine.changed_leds(word(0, 1), word(2, 3)),
                ))
            }
            // General illumination is not modelled; an empty array means
            // "nothing changed", which is true.
            "changedgi" | "changedlamps2" => Ok(empty_array()),

            // The two score displays, as text, which is neither what VPinMAME
            // returns nor what a table reads — it is here for tests and for a
            // host that wants the score without drawing segments.
            "upperdisplay" => Ok(Value::str(self.machine.displays().0)),
            "lowerdisplay" => Ok(Value::str(self.machine.displays().1)),

            // Everything else is the parts of VPinMAME that are configuration
            // rather than emulation: `Games(...)`, `Settings`, `SplashInfoLine`,
            // the window handling. A table walks into them several levels deep,
            // so they answer with something that can be chained.
            _ => Ok(Value::Object(Rc::new(Chainable))),
        }
    }

    fn set(&self, name: &str, args: &[Value], value: Value, _by_ref: bool) -> VbResult<()> {
        match &*name.to_ascii_lowercase() {
            "gamename" => {
                *self.pending.borrow_mut() = value.to_str()?.to_string();
                Ok(())
            }
            // `Controller.Switch(16) = 1` — the one write that matters, and the
            // one a table does thousands of times a game.
            "switch" => {
                let n = Self::number(args)?;
                let closed = value.to_bool()?;
                let mut board = self.machine.board.borrow_mut();
                match Door::of(n) {
                    // The coin door's buttons are not in the matrix; they hang
                    // off the CPU board's own inputs.
                    Some(Door::Up) => board.set_diagnostic_up(closed),
                    Some(Door::Advance) => board.set_diagnostic_advance(closed),
                    // The two diagnostic buttons on the sound and CPU boards
                    // reset them. Nothing here has a reset line to pull, so
                    // they are accepted and do nothing rather than raising —
                    // a player pressing one should not stop the table.
                    Some(_) => {}
                    None => {
                        if let Ok(n) = u8::try_from(n) {
                            board.set_switch(n, closed);
                        }
                    }
                }
                Ok(())
            }
            // Accepted and ignored: display size, window position, dip
            // switches. None of them changes what the ROM does.
            _ => Ok(()),
        }
    }
}

/// The coin door's buttons, which a script addresses with negative numbers.
///
/// They are not switches in the playfield matrix — they are wired straight to
/// the CPU board, inside the coin door, where an operator can reach them and a
/// player cannot. VPinMAME gives them negative numbers to keep them out of the
/// matrix's numbering, and every machine library uses the same four
/// (`s11.vbs:24`).
///
/// Without them a table's script raises on `Controller.Switch(swAdvance) = True`
/// and takes its whole key handler with it — which is how the diagnostic keys
/// came to be the last four that did not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Door {
    Advance,
    Up,
    CpuDiag,
    SoundDiag,
}

impl Door {
    fn of(number: i32) -> Option<Self> {
        Some(match number {
            -7 => Door::Advance,
            -6 => Door::Up,
            -5 => Door::CpuDiag,
            -4 => Door::SoundDiag,
            _ => return None,
        })
    }
}

/// The corners of VPinMAME that are configuration rather than emulation.
struct Chainable;

impl Object for Chainable {
    fn type_name(&self) -> &'static str {
        "Object"
    }
    fn get(&self, _name: &str, _args: &[Value]) -> VbResult<Value> {
        Ok(Value::Object(Rc::new(Chainable)))
    }
    fn set(&self, _n: &str, _a: &[Value], _v: Value, _r: bool) -> VbResult<()> {
        Ok(())
    }
    fn default_value(&self) -> VbResult<Value> {
        Ok(Value::Long(0))
    }
    fn enumerate(&self) -> Option<Vec<Value>> {
        Some(Vec::new())
    }
}

fn empty_array() -> Value {
    Value::Array(Rc::new(RefCell::new(Array::from_values(Vec::new()))))
}

/// The shape `ChangedLamps` returns: a **two-dimensional** array of number and
/// state, with the numbers down the first dimension.
///
/// Two dimensions and not an array of arrays. VBScript tells them apart and
/// every table's lamp loop is written for the first:
///
/// ```vbscript
/// For ii = 0 To UBound(chgLamp)
///     LampState(chgLamp(ii, 0)) = chgLamp(ii, 1)
/// ```
///
/// `chgLamp(ii, 0)` on an array of arrays is a two-subscript read of a
/// one-dimensional array, which raises "subscript out of range" — every time,
/// on every poll, swallowed by the `On Error Resume Next` around it. The table
/// keeps running and not one lamp ever lights.
fn two_column_array(rows: &[(u8, bool)]) -> Value {
    // An empty change list still has to be an array a table can take `UBound`
    // of: the loop is guarded by `If Not IsEmpty(...)`, not by the count.
    let mut array = Array::new(vec![rows.len() as i32 - 1, 1]).unwrap_or_else(|_| {
        // Only reachable with no rows at all, where the first bound is -1.
        Array::from_values(Vec::new())
    });
    for (i, (number, on)) in rows.iter().enumerate() {
        let i = i as i32;
        let _ = array.set(&[i, 0], Value::Long(i32::from(*number)));
        let _ = array.set(&[i, 1], Value::Long(i32::from(*on)));
    }
    Value::Array(Rc::new(RefCell::new(array)))
}

/// The same, for the three columns `ChangedLEDs` hands back.
fn three_column_array(rows: &[(u8, u16, u16)]) -> Value {
    let mut array = Array::new(vec![rows.len() as i32 - 1, 2])
        .unwrap_or_else(|_| Array::from_values(Vec::new()));
    for (i, (digit, changed, now)) in rows.iter().enumerate() {
        let i = i as i32;
        let _ = array.set(&[i, 0], Value::Long(i32::from(*digit)));
        let _ = array.set(&[i, 1], Value::Long(i32::from(*changed)));
        let _ = array.set(&[i, 2], Value::Long(i32::from(*now)));
    }
    Value::Array(Rc::new(RefCell::new(array)))
}

/// Reads a PinMAME ROM zip into its entries.
///
/// Written by hand rather than pulled in as a dependency: a ROM zip is a
/// handful of small files stored or deflated, and the alternative is a crate
/// that has to build for wasm as well.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    // The central directory is at the end, after the end-of-central-directory
    // record. Finding that record means scanning back for its signature, since
    // it has a variable-length comment after it.
    let eocd = (0..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("not a zip: no end-of-central-directory record")?;

    let count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
    let mut at = u32::from_le_bytes([
        bytes[eocd + 16],
        bytes[eocd + 17],
        bytes[eocd + 18],
        bytes[eocd + 19],
    ]) as usize;

    for _ in 0..count {
        if at + 46 > bytes.len() || bytes[at..at + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err("the zip's central directory is malformed".into());
        }
        let method = u16::from_le_bytes([bytes[at + 10], bytes[at + 11]]);
        let compressed = u32::from_le_bytes([
            bytes[at + 20],
            bytes[at + 21],
            bytes[at + 22],
            bytes[at + 23],
        ]) as usize;
        let name_len = u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[at + 30], bytes[at + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[at + 32], bytes[at + 33]]) as usize;
        let offset = u32::from_le_bytes([
            bytes[at + 42],
            bytes[at + 43],
            bytes[at + 44],
            bytes[at + 45],
        ]) as usize;
        let name = String::from_utf8_lossy(&bytes[at + 46..at + 46 + name_len]).into_owned();
        at += 46 + name_len + extra_len + comment_len;

        // The local header repeats the name and extra fields, and its own extra
        // length is the one that counts.
        if offset + 30 > bytes.len() {
            return Err(format!("'{name}' points outside the file"));
        }
        let l_name = u16::from_le_bytes([bytes[offset + 26], bytes[offset + 27]]) as usize;
        let l_extra = u16::from_le_bytes([bytes[offset + 28], bytes[offset + 29]]) as usize;
        let start = offset + 30 + l_name + l_extra;
        let end = start + compressed;
        if end > bytes.len() {
            return Err(format!("'{name}' runs past the end of the file"));
        }

        let data = match method {
            0 => bytes[start..end].to_vec(),
            8 => inflate(&bytes[start..end])
                .ok_or_else(|| format!("'{name}' is deflated in a way we cannot read"))?,
            other => return Err(format!("'{name}' uses compression method {other}")),
        };
        out.push((name.to_ascii_lowercase(), data));
    }
    Ok(out)
}

/// Raw DEFLATE.
///
/// Every entry in a PinMAME ROM zip is deflated, so this is not an edge case.
/// It comes from a library on purpose: a wrong bit here does not fail, it
/// produces a ROM image that is almost right, and the machine then runs
/// something that is not the game.
fn inflate(input: &[u8]) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec(input).ok()
}

fn pick<'a>(images: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    let wanted = name.to_ascii_lowercase();
    images
        .iter()
        .find(|(n, _)| *n == wanted)
        .map(|(_, d)| d.as_slice())
}

/// Finds an image whose name contains a marker, for the sound boards.
///
/// Sound images are named after the chip position — `u21`, `u22`, `u4` — but
/// with a game-specific prefix and suffix that the manifest does not record.
fn pick_like<'a>(images: &'a [(String, Vec<u8>)], marker: &str) -> Option<&'a [u8]> {
    images
        .iter()
        .find(|(n, _)| n.contains(marker))
        .map(|(_, d)| d.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lamp_has_a_number_the_board_answers_for() {
        // The number goes back to the board on the very next line
        // (`board.lamp_lit(number)`), so the two have to agree on what a number
        // means. They did not: the numbering here was WPC's `col*10 + row + 1`
        // and the board's is a System 11's straight 1 to 64, so the last two
        // columns asked about lamps that do not exist, always came back dark,
        // and were therefore never reported as having changed — sixteen bulbs
        // the table was never told about.
        //
        // Checking the round trip and not the formula: restating `i + 1` in a
        // test proves nothing, whereas this fails the moment either side moves.
        //
        // Sixty-four of them, because this is about a System 11's matrix. A
        // Whitestar has twice as many and numbers them on its own board.
        for i in 0..64usize {
            let number = lamp_number(i);
            let position = vpw_s11::matrix_position(number);
            assert_eq!(
                position,
                Some((i / 8, 1 << (i % 8))),
                "lamp {i} was reported as number {number}, which the board reads as {position:?}"
            );
        }
    }

    #[test]
    fn changed_reports_each_change_once() {
        // A table polls this from a timer and expects each change once.
        // Reporting a lamp every poll is how a port ends up with lamps that
        // never go off.
        let m = Machine::new();
        // With no ROM the board reports nothing, so the record stays empty
        // and the second read is empty too.
        assert!(m.changed_lamps().is_empty());
        assert!(m.changed_lamps().is_empty());
    }

    #[test]
    fn a_row_is_a_pair_the_script_can_index() {
        // The shape a table's lamp loop reads: `chgLamp(ii, 0)` is the number
        // and `chgLamp(ii, 1)` the state, on **one** two-dimensional array.
        let v = two_column_array(&[(16, true), (17, false)]);
        let array = v.to_array().unwrap();
        let array = array.borrow();
        assert_eq!(array.dimensions(), 2, "it has to be two-dimensional");
        assert_eq!(array.bounds, vec![1, 1], "two rows, two columns");

        assert_eq!(array.get(&[0, 0]).unwrap().to_number().unwrap(), 16.0);
        assert_eq!(array.get(&[0, 1]).unwrap().to_number().unwrap(), 1.0);
        assert_eq!(array.get(&[1, 0]).unwrap().to_number().unwrap(), 17.0);
        assert_eq!(array.get(&[1, 1]).unwrap().to_number().unwrap(), 0.0);
    }

    #[test]
    fn nothing_changed_is_still_an_array() {
        // `If Not IsEmpty(chgLamp) Then For ii = 0 To UBound(chgLamp)` — the
        // guard is on emptiness, so an empty result still has to answer
        // `UBound` with -1 rather than raising.
        let v = two_column_array(&[]);
        let array = v.to_array().unwrap();
        assert_eq!(array.borrow().bounds[0], -1);
    }

    #[test]
    fn an_unknown_set_is_refused_by_name() {
        let m = Machine::new();
        let e = m.load("no_such_game", &[], None).unwrap_err();
        assert!(e.contains("no_such_game"), "{e}");
    }

    #[test]
    fn a_machine_with_no_rom_does_not_pretend_to_run() {
        let m = Machine::new();
        assert!(!m.is_running());
        assert!(m.game_name().is_none());
        // And advancing it does nothing rather than panicking.
        m.advance(1.0);
    }
}
