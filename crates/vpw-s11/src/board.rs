//! Memory map and peripherals of the Williams System 11 board.

use vpw_m6800::Bus;
use vpw_pia6821::Pia;

use crate::display::Displays;
use crate::io::{LampMatrix, Solenoids, SwitchMatrix};

/// Battery-backed RAM: `$0000`-`$1FFF`.
pub const RAM_SIZE: usize = 0x2000;
/// Game ROM: `$4000`-`$FFFF`.
pub const ROM_START: u16 = 0x4000;
pub const ROM_SIZE: usize = 0x1_0000 - ROM_START as usize;

/// Write-only latch for solenoids 1-8 (`latch2200` in `s11.c:603`).
pub const SOLENOID_LATCH: u16 = 0x2200;

/// Base address of each of the six PIAs. Each one decodes four registers from
/// there on (`s11.c:1291-1312`).
pub const PIA_BASES: [u16; 6] = [0x2100, 0x2400, 0x2800, 0x2C00, 0x3000, 0x3400];

/// What each PIA does on the board. The comments in the original driver
/// (`s11.c:702-760`) document the wiring.
pub mod pia {
    /// `$2100`. PA = sound latch, PB = solenoids 9-16,
    /// CA2 = sound handshake, CB2 = enables the special solenoids.
    pub const SOUND_SOLENOIDS: usize = 0;
    /// `$2400`. PA = lamp matrix rows, PB = strobe columns.
    pub const LAMPS: usize = 1;
    /// `$2800`. PA0-3 = digit select, PA4 = diagnostic LED,
    /// PB = digit in BCD, CA1/CB1 = diagnostic buttons.
    pub const DISPLAY_SELECT: usize = 2;
    /// `$2C00`. PA/PB = segments of the alphanumeric display.
    pub const DISPLAY_DATA: usize = 3;
    /// `$3000`. PA = switch rows (input), PB = columns (output).
    pub const SWITCHES: usize = 4;
    /// `$3400`. Secondary display and Widget I/O.
    pub const WIDGET: usize = 5;
}

/// An access to an address the board does not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnmappedAccess {
    pub address: u16,
    pub write: bool,
}

/// The board: memory, the six PIAs and the peripherals hanging off them.
pub struct Board {
    ram: Box<[u8; RAM_SIZE]>,
    rom: Box<[u8; ROM_SIZE]>,

    /// The six PIAs, indexed as in [`pia`].
    pub pias: [Pia; 6],

    /// The two displays on the backbox.
    pub displays: Displays,

    /// Current CPU cycle. [`crate::System11`] keeps it up to date; the
    /// display's anti-flicker filter needs a time reference to tell a blip
    /// apart from an actual lit segment.
    pub(crate) cycle: u64,

    /// Playfield switch matrix, numbered the Williams way.
    pub switches: SwitchMatrix,

    /// Lamp matrix, accumulated over each sweep.
    pub lamps: LampMatrix,

    /// The 32 solenoid outputs.
    pub solenoids: Solenoids,

    /// The playfield switch that fires each special solenoid directly, or 0.
    /// Per game; see [`crate::games`].
    special_switches: [u8; 6],
    /// The switches the two flipper buttons close, left then right. See
    /// [`crate::io::FIRST_FLIPPER`].
    flipper_buttons: (u8, u8),

    /// Advance button of the diagnostic menu (inside the door).
    pub diagnostic_advance: bool,
    /// Up/down switch of the diagnostic menu.
    pub diagnostic_up: bool,

    /// Last access to an address that isn't decoded. Useful during boot: if a
    /// real ROM pokes outside the map, something is missing.
    pub last_unmapped: Option<UnmappedAccess>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0; RAM_SIZE]),
            rom: Box::new([0; ROM_SIZE]),
            pias: std::array::from_fn(|_| Pia::new()),
            displays: Displays::new(),
            cycle: 0,
            switches: SwitchMatrix::new(),
            lamps: LampMatrix::new(),
            solenoids: Solenoids::new(),
            special_switches: [0; 6],
            flipper_buttons: (0, 0),
            diagnostic_advance: false,
            diagnostic_up: false,
            last_unmapped: None,
        }
    }

    /// Copies a ROM image in. `addr` has to fall inside `$4000`-`$FFFF`.
    pub fn load_rom(&mut self, addr: u16, data: &[u8]) -> Result<(), RomError> {
        if addr < ROM_START {
            return Err(RomError::BelowRomStart(addr));
        }
        let offset = (addr - ROM_START) as usize;
        if offset + data.len() > ROM_SIZE {
            return Err(RomError::TooLarge {
                addr,
                len: data.len(),
            });
        }
        self.rom[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Loads an image into the CPU's 64 KB region, by region offset.
    ///
    /// Only what falls from `$4000` on is visible: below that is the RAM.
    /// Several Data East games load 32 or 64 KB images from `$0000` and leave
    /// that first part covered on purpose, so offsets that never reach the ROM
    /// have to be accepted instead of rejected.
    pub fn load_cpu_region(&mut self, offset: usize, data: &[u8]) -> Result<(), RomError> {
        if offset + data.len() > 0x1_0000 {
            return Err(RomError::TooLarge {
                addr: offset as u16,
                len: data.len(),
            });
        }
        let rom_start = ROM_START as usize;
        // Trim off the part that lands below the ROM.
        let (skip, at) = if offset >= rom_start {
            (0, offset - rom_start)
        } else {
            (rom_start - offset, 0)
        };
        if skip >= data.len() {
            return Ok(()); // the whole image is covered by the RAM
        }
        let visible = &data[skip..];
        self.rom[at..at + visible.len()].copy_from_slice(visible);
        Ok(())
    }

    pub fn ram(&self) -> &[u8] {
        &self.ram[..]
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram[..]
    }

    /// Whether any of the six PIAs is requesting an interrupt. On the board
    /// the twelve lines are wired together to the 6808's IRQ (`s11.c:121`).
    pub fn pia_irq(&self) -> bool {
        self.pias.iter().any(Pia::irq)
    }

    /// Base address and register if `addr` falls inside some PIA's window.
    fn decode_pia(addr: u16) -> Option<(usize, u8)> {
        PIA_BASES.iter().enumerate().find_map(|(index, &base)| {
            (addr >= base && addr < base + 4).then(|| (index, (addr - base) as u8))
        })
    }

    // --- Peripherals hanging off the PIAs ---------------------------------------

    /// Passes on to the display whatever just came out of the ports.
    ///
    /// This has to hook into the **write** and not be polled: the display is
    /// multiplexed, and each write latches the segments of whichever digit is
    /// selected at that moment.
    fn feed_displays(&mut self, index: usize, select: u8) {
        let cycle = self.cycle;
        match (index, select) {
            // PIA 2, port A: digit select (and the diagnostic LED).
            (pia::DISPLAY_SELECT, 0) if self.pias[index].port_a_data_selected() => {
                let value = self.pias[index].port_a_output();
                self.displays.select_digit(value);
            }
            // PIA 2, port B: the bottom row, seven-segment.
            (pia::DISPLAY_SELECT, 2) if self.pias[index].port_b_data_selected() => {
                let value = self.pias[index].port_b_output();
                self.displays.write_lower(value);
            }
            // PIA 3, port A: high segments of the top row.
            (pia::DISPLAY_DATA, 0) if self.pias[index].port_a_data_selected() => {
                let value = self.pias[index].port_a_output();
                self.displays.write_upper_high(value, cycle);
            }
            // PIA 3, port B: low segments of the top row.
            (pia::DISPLAY_DATA, 2) if self.pias[index].port_b_data_selected() => {
                let value = self.pias[index].port_b_output();
                self.displays.write_upper_low(value, cycle);
            }

            // PIA 5, port A: the high byte of the bottom row, on the machines
            // where it is alphanumeric.
            (pia::WIDGET, 0) if self.pias[index].port_a_data_selected() => {
                let value = self.pias[index].port_a_output();
                self.displays.write_lower_high(value);
            }

            // PIA 1: the lamp matrix, applied on the **row** write.
            //
            // The rows are active low: a 0 turns the lamp on. The ROM selects
            // the column first and writes the rows second, so the row write is
            // the moment both halves are the pair that was meant — and it is
            // the only such moment.
            //
            // Applying on the column write as well is what a first attempt does
            // and it is wrong in a way that looks like a lighting effect: at
            // that instant the rows still hold the *previous* column's lamps,
            // which get smeared onto the new column. Every sweep ghosts one
            // column onto the next until nearly the whole playfield is lit,
            // flickering in time with the sweep. Applying on the column write
            // *only* is wrong the other way — the ROM blanks the rows to `$FF`
            // before moving the column, so every lamp reads as off and the
            // table never lights at all.
            //
            // Guarded on the data register being the one selected, the same way
            // the solenoid latch is. A write to address 0 goes to the *direction*
            // register when the control register says so, and while the direction
            // bits are being set up the pins read as undriven — which, on an
            // active-low line, is every lamp in the selected column coming on.
            // Both the rows and the column strobe, exactly as the original
            // hooks both `pia1a_w` and `pia1b_w` (`s11.c:363`). The ROM sets one
            // and then the other, so between the two writes the wrong lamps are
            // briefly driven — which is what the machine does, and the filaments
            // charge it the few microseconds it is worth.
            (pia::LAMPS, 0) | (pia::LAMPS, 2) => {
                let p = &self.pias[pia::LAMPS];
                let (rows, strobe) = (!p.port_a_output(), p.port_b_output());
                self.lamps.drive(self.seconds(), rows, strobe);
            }

            // PIA 0, port B: solenoids 9-16.
            (pia::SOUND_SOLENOIDS, 2) if self.pias[index].port_b_data_selected() => {
                let value = self.pias[index].port_b_output();
                self.solenoids.set_high(self.seconds(), value);
            }
            // PIA 0, control register B: CB2 enables the special solenoids and
            // "game on" with them. Inverted — the pin **low** is enabled
            // (`locals.ssEn = !data`, `s11.c:612`).
            (pia::SOUND_SOLENOIDS, 3) if self.pias[index].cb2_is_output() => {
                let enabled = !self.pias[index].cb2_output();
                let now = self.seconds();
                self.solenoids.set_special_enabled(now, enabled);
            }

            // The six control lines that drive the special solenoids, in the
            // order the original hooks them (`s11.c:622`): PIA 1's CA2 and CB2,
            // then PIA 3's, then PIA 4's. Which solenoid each one reaches is not
            // the order they are in; see `SPECIAL_ORDER`.
            //
            // Address 1 is a PIA's control register A and 3 is its control
            // register B, so a write to either is where CA2 or CB2 can have
            // changed.
            //
            // Guarded on the line being an **output**, because a solenoid
            // hanging off it only hears from the PIA while the PIA is driving
            // it. In the original that guard is free: MAME calls `pia1ca2_w`
            // and its five siblings when the output changes and not otherwise,
            // and a control register written while the line is still an input
            // never reaches them. Here the write is what we see, so the
            // question has to be asked.
            (pia::LAMPS, 1) if self.pias[pia::LAMPS].ca2_is_output() => {
                self.drive_special(0, self.pias[pia::LAMPS].ca2_output());
            }
            (pia::LAMPS, 3) if self.pias[pia::LAMPS].cb2_is_output() => {
                self.drive_special(1, self.pias[pia::LAMPS].cb2_output());
            }
            (pia::DISPLAY_DATA, 1) if self.pias[pia::DISPLAY_DATA].ca2_is_output() => {
                self.drive_special(2, self.pias[pia::DISPLAY_DATA].ca2_output());
            }
            (pia::DISPLAY_DATA, 3) if self.pias[pia::DISPLAY_DATA].cb2_is_output() => {
                self.drive_special(3, self.pias[pia::DISPLAY_DATA].cb2_output());
            }
            (pia::SWITCHES, 1) if self.pias[pia::SWITCHES].ca2_is_output() => {
                self.drive_special(4, self.pias[pia::SWITCHES].ca2_output());
            }
            (pia::SWITCHES, 3) if self.pias[pia::SWITCHES].cb2_is_output() => {
                self.drive_special(5, self.pias[pia::SWITCHES].cb2_output());
            }
            _ => {}
        }
    }

    /// One of the six special-solenoid control lines has moved.
    fn drive_special(&mut self, line: usize, level: bool) {
        let now = self.seconds();
        self.solenoids.set_special_line(now, line, level);
    }

    /// Which playfield switch, if any, fires each of the six special solenoids
    /// directly. See [`crate::io::Solenoids::set_special_switch`].
    pub fn set_special_switches(&mut self, switches: [u8; 6]) {
        self.special_switches = switches;
    }

    /// Which switches the two flipper buttons close, left then right. See
    /// [`crate::io::FIRST_FLIPPER`].
    pub fn set_flipper_buttons(&mut self, buttons: (u8, u8)) {
        self.flipper_buttons = buttons;
    }

    /// Reads the switches that are wired straight to a special solenoid.
    ///
    /// A slingshot and a pop bumper fire from their own switch through the
    /// hardware, without the CPU being involved, and the original reads them
    /// once a frame in its fake VBLANK (`s11.c:193`).
    fn poll_special_switches(&mut self) {
        let now = self.seconds();
        for (offset, &switch) in self.special_switches.iter().enumerate() {
            if switch != 0 {
                let closed = self.switches.is_closed(switch);
                self.solenoids.set_special_switch(now, offset, closed);
            }
        }

        self.poll_flipper_buttons(now);
    }

    /// Reads the flipper buttons and does the two things they do.
    ///
    /// Port of `core_updateSw` (`core.c:1731-1753`), which a System 11 calls
    /// once a frame with the flipper relay as its argument
    /// (`core_updateSw(locals.ssEn)`, `s11.c:358`).
    ///
    /// The buttons are not on the playfield matrix. A script reports them in a
    /// column of their own — `swLLFlip = 84`, `swLRFlip = 82` (`s11.vbs:37`) —
    /// and from there they go two separate ways:
    ///
    /// 1. Into the playfield matrix, at whichever switches this game's
    ///    `FLIP_SWNO` names, which is how the **ROM** learns a button is held.
    /// 2. Up onto four solenoids nobody drives, which is how the **table**
    ///    learns it: `SolCallback(sLLFlipper)` is where the flipper sound and
    ///    the flipper's own movement hang. See [`crate::io::FIRST_FLIPPER`].
    ///
    /// The first of those also **overwrites** whatever else set those matrix
    /// switches, every sweep, which is deliberate: it is the machine's own
    /// wiring having the last word over anything a table happened to write.
    fn poll_flipper_buttons(&mut self, now: f32) {
        use crate::io::switches::{FLIPPER_BUTTON_LEFT, FLIPPER_BUTTON_RIGHT};

        let left = self.switches.is_closed(FLIPPER_BUTTON_LEFT);
        let right = self.switches.is_closed(FLIPPER_BUTTON_RIGHT);

        let (matrix_left, matrix_right) = self.flipper_buttons;
        if matrix_left != 0 {
            self.switches.set(matrix_left, left);
        }
        if matrix_right != 0 {
            self.switches.set(matrix_right, right);
        }

        self.solenoids.set_flipper_buttons(now, left, right);
    }

    /// Refreshes the inputs that depend on the state of the outputs.
    ///
    /// The switch matrix is multiplexed: PIA 4 drives the columns through port
    /// B and reads the rows through port A, so what the CPU sees on PA depends
    /// on what it itself just put on PB.
    pub fn refresh_inputs(&mut self) {
        let strobe = self.pias[pia::SWITCHES].port_b_output();
        let rows = self.switches.rows_for_strobe(strobe);
        self.pias[pia::SWITCHES].set_port_a_input(rows);
    }

    /// Closes a lamp and solenoid sweep.
    pub fn refresh_outputs(&mut self) {
        self.poll_special_switches();
        let now = self.seconds();
        self.lamps.integrate(now);
        self.solenoids.integrate(now);
        self.solenoids.refresh();
    }

    /// The board's clock, in seconds, which is what the bulbs are integrated
    /// against.
    fn seconds(&self) -> f32 {
        self.cycle as f32 / crate::CPU_CLOCK_HZ as f32
    }

    /// What PIA 1 is driving right now: `(rows, columns)`, with the rows
    /// already un-inverted: a bit set to 1 is a lit lamp.
    /// To find out which ones are actually on, use [`Board::lamps`].
    pub fn lamp_drive(&self) -> (u8, u8) {
        let p = &self.pias[pia::LAMPS];
        (!p.port_a_output(), p.port_b_output())
    }

    /// Sound code sitting in the latch (PIA 0's port A).
    pub fn sound_latch(&self) -> u8 {
        self.pias[pia::SOUND_SOLENOIDS].port_a_output()
    }

    /// The diagnostic LED is bit 4 of PIA 2's port A. On a real board it
    /// blinks during boot, so it is the first sign of life.
    pub fn diagnostic_led(&self) -> bool {
        self.pias[pia::DISPLAY_SELECT].port_a_output() & 0x10 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomError {
    /// The address falls below `$4000`, where there is no ROM.
    BelowRomStart(u16),
    /// The image does not fit in the ROM space.
    TooLarge { addr: u16, len: usize },
}

impl std::fmt::Display for RomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BelowRomStart(addr) => {
                write!(
                    f,
                    "{addr:#06X} is below the start of the ROM ({ROM_START:#06X})"
                )
            }
            Self::TooLarge { addr, len } => {
                write!(
                    f,
                    "a {len}-byte image at {addr:#06X} runs past the end of the map"
                )
            }
        }
    }
}

impl std::error::Error for RomError {}

impl Bus for Board {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize],
            0x4000..=0xFFFF => self.rom[(addr - ROM_START) as usize],
            _ => match Self::decode_pia(addr) {
                Some((index, select)) => self.pias[index].read(select),
                None => {
                    self.last_unmapped = Some(UnmappedAccess {
                        address: addr,
                        write: false,
                    });
                    // Floating bus: on the real board the lines stay high.
                    0xFF
                }
            },
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize] = value,
            // Writing to the ROM does nothing, but it isn't an error either:
            // there is code that scans memory without discriminating.
            0x4000..=0xFFFF => {}
            SOLENOID_LATCH => self.solenoids.set_low(self.seconds(), value),
            _ => match Self::decode_pia(addr) {
                Some((index, select)) => {
                    self.pias[index].write(select, value);
                    self.feed_displays(index, select);
                }
                None => {
                    self.last_unmapped = Some(UnmappedAccess {
                        address: addr,
                        write: true,
                    });
                }
            },
        }
    }
}
