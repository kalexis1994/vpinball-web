//! The WPC ASIC: all of the machine's I/O in eighty addresses.
//!
//! It replaces System 11's six PIAs with a single custom chip. Besides the I/O
//! it brings two things the old hardware did not have: ROM paging, which makes
//! it possible to go from 48 KB to a megabyte of code, and a **shift
//! accelerator** for bit handling.

/// Register offsets, relative to `$3FB0`.
///
/// Taken from `wpc.h:156-227`.
pub mod registers {
    // --- Dot matrix display ---
    /// Picks the DMD page seen at `$3A00`.
    pub const DMD_PAGE3A00: usize = 0x3FBC - 0x3FB0;
    /// Status and acknowledge of the DMD interrupt.
    pub const DMD_FIRQLINE: usize = 0x3FBD - 0x3FB0;
    /// Picks the DMD page seen at `$3800`.
    pub const DMD_PAGE3800: usize = 0x3FBE - 0x3FB0;
    /// Page to be drawn on the next strobe pass.
    pub const DMD_SHOWPAGE: usize = 0x3FBF - 0x3FB0;

    // --- Sound and flippers ---
    /// Fliptronics: reading gives the flipper switches, writing drives their coils.
    pub const FLIPPERS: usize = 0x3FD4 - 0x3FB0;
    pub const SND_DATA: usize = 0x3FDC - 0x3FB0;
    pub const SND_CTRL: usize = 0x3FDD - 0x3FB0;

    // --- Power driver board ---
    /// Solenoids 25-28.
    pub const SOLENOID1: usize = 0x3FE0 - 0x3FB0;
    /// Solenoids 1-8.
    pub const SOLENOID2: usize = 0x3FE1 - 0x3FB0;
    /// Solenoids 17-24.
    pub const SOLENOID3: usize = 0x3FE2 - 0x3FB0;
    /// Solenoids 9-16.
    pub const SOLENOID4: usize = 0x3FE3 - 0x3FB0;
    /// Rows of the lamp matrix.
    pub const LAMPROW: usize = 0x3FE4 - 0x3FB0;
    /// Columns of the lamp matrix.
    pub const LAMPCOLUMN: usize = 0x3FE5 - 0x3FB0;
    /// General illumination.
    pub const GILAMPS: usize = 0x3FE6 - 0x3FB0;
    /// Configuration switches on the CPU board.
    pub const DIPSWITCH: usize = 0x3FE7 - 0x3FB0;
    /// Coin door switches.
    pub const SWCOINDOOR: usize = 0x3FE8 - 0x3FB0;
    /// Reads the switch row for the selected column.
    pub const SWROWREAD: usize = 0x3FE9 - 0x3FB0;
    /// Column select for the switch matrix.
    pub const SWCOLSELECT: usize = 0x3FEA - 0x3FB0;

    // --- Alphanumeric display, on the WPCs that predate the DMD ---
    pub const ALPHAPOS: usize = 0x3FEB - 0x3FB0;
    pub const ALPHA1LO: usize = 0x3FEC - 0x3FB0;
    pub const ALPHA1HI: usize = 0x3FED - 0x3FB0;
    pub const ALPHA2LO: usize = 0x3FEE - 0x3FB0;
    pub const ALPHA2HI: usize = 0x3FEF - 0x3FB0;
    /// On WPC-95 this address is the flipper coils.
    pub const FLIPPERCOIL95: usize = ALPHA2LO;
    /// And this one, their switches.
    pub const FLIPPERSW95: usize = ALPHA2HI;

    // --- CPU board ---
    /// Diagnostic LED, on bit 7.
    pub const LED: usize = 0x3FF2 - 0x3FB0;
    pub const IRQACK: usize = 0x3FF3 - 0x3FB0;
    /// Shift accelerator: high byte of the address.
    pub const SHIFTADRH: usize = 0x3FF4 - 0x3FB0;
    /// Low byte.
    pub const SHIFTADRL: usize = 0x3FF5 - 0x3FB0;
    /// Bit number.
    pub const SHIFTBIT: usize = 0x3FF6 - 0x3FB0;
    /// Second bit number.
    pub const SHIFTBIT2: usize = 0x3FF7 - 0x3FB0;
    /// High resolution timer.
    pub const HIGHRESTIMER: usize = 0x3FF8 - 0x3FB0;
    pub const RTCHOUR: usize = 0x3FFA - 0x3FB0;
    pub const RTCMIN: usize = 0x3FFB - 0x3FB0;
    /// ROM bank select.
    pub const ROMBANK: usize = 0x3FFC - 0x3FB0;
    pub const PROTMEM: usize = 0x3FFD - 0x3FB0;
    pub const PROTMEMCTRL: usize = 0x3FFE - 0x3FB0;
    /// Zero crossing of the AC line, and control of the periodic interrupt.
    pub const ZC_IRQ_ACK: usize = 0x3FFF - 0x3FB0;
}

use registers as r;

/// How many registers the chip has.
const REGISTERS: usize = 0x50;

/// Columns of the playfield switch matrix.
pub const SWITCH_COLUMNS: usize = 8;

pub struct Asic {
    /// The registers exactly as they were written.
    data: [u8; REGISTERS],

    /// Playfield switch matrix: `columns[i]` are the closed switches of column
    /// `i+1`.
    switches: [u8; SWITCH_COLUMNS],
    /// Coin door and cabinet switches.
    pub coin_door: u8,
    /// Flipper switches, which come in on their own line.
    pub flipper_switches: u8,
    /// Configuration switches on the board.
    pub dip_switches: u8,

    /// Lamp matrix, accumulated over the strobe pass.
    lamp_accum: [u8; 8],
    lamps: [u8; 8],

    /// The 28 solenoids as a bitmap.
    solenoids: u32,

    /// Periodic interrupt pending.
    irq_pending: bool,
    /// Fast interrupt pending (raised by the DMD and by the timer).
    firq_pending: bool,
    /// State of the zero crossing.
    zero_cross: bool,
}

impl Default for Asic {
    fn default() -> Self {
        Self::new()
    }
}

impl Asic {
    pub fn new() -> Self {
        Self {
            data: [0; REGISTERS],
            switches: [0; SWITCH_COLUMNS],
            coin_door: 0,
            flipper_switches: 0,
            // All switches open is the standard configuration.
            dip_switches: 0x00,
            lamp_accum: [0; 8],
            lamps: [0; 8],
            solenoids: 0,
            irq_pending: false,
            firq_pending: false,
            zero_cross: false,
        }
    }

    pub fn reset(&mut self) {
        let (switches, coin_door, dips) = (self.switches, self.coin_door, self.dip_switches);
        *self = Self::new();
        self.switches = switches;
        self.coin_door = coin_door;
        self.dip_switches = dips;
    }

    // --- Access from the CPU --------------------------------------------------

    pub fn read(&mut self, reg: usize) -> u8 {
        if reg >= REGISTERS {
            return 0xFF;
        }
        match reg {
            // The flipper switches arrive inverted: a zero means pressed.
            r::FLIPPERS | r::FLIPPERSW95 => !self.flipper_switches,
            r::SWCOINDOOR => self.coin_door,
            r::SWROWREAD => self.switch_rows(self.data[r::SWCOLSELECT]),
            r::DIPSWITCH => self.dip_switches,

            // The shift accelerator. The CPU leaves an address and a bit
            // number, and the chip gives back the already shifted address and
            // the bit mask. It saves the 6809 several instructions in the
            // game's hottest loop: walking bit tables.
            r::SHIFTADRH => self.data[r::SHIFTADRH].wrapping_add(
                ((u16::from(self.data[r::SHIFTADRL]) + u16::from(self.data[r::SHIFTBIT] >> 3)) >> 8)
                    as u8,
            ),
            r::SHIFTADRL => self.data[r::SHIFTADRL].wrapping_add(self.data[r::SHIFTBIT] >> 3),
            r::SHIFTBIT | r::SHIFTBIT2 => 1 << (self.data[reg] & 0x07),

            r::HIGHRESTIMER => u8::from(self.firq_pending) << 7,

            r::ZC_IRQ_ACK => {
                // Bit 7 carries the zero crossing, and reading it consumes it.
                let value = (u8::from(self.zero_cross) << 7) | (self.data[reg] & 0x7F);
                self.zero_cross = false;
                value
            }

            // Real time clock. With no real time source it returns zero, which
            // is what a machine with a dead battery sees.
            r::RTCHOUR | r::RTCMIN => 0,

            _ => self.data[reg],
        }
    }

    pub fn write(&mut self, reg: usize, value: u8) {
        if reg >= REGISTERS {
            return;
        }
        self.data[reg] = value;

        match reg {
            // The row and the column can be written in any order, so the
            // matrix has to be recomputed from both of them.
            r::LAMPROW | r::LAMPCOLUMN => {
                let (row, column) = (self.data[r::LAMPROW], self.data[r::LAMPCOLUMN]);
                for i in 0..8 {
                    if column & (1 << i) != 0 {
                        self.lamp_accum[i] |= row;
                    }
                }
            }
            r::SOLENOID1 => self.set_solenoid_group(24, value),
            r::SOLENOID2 => self.set_solenoid_group(0, value),
            r::SOLENOID3 => self.set_solenoid_group(16, value),
            r::SOLENOID4 => self.set_solenoid_group(8, value),

            r::ZC_IRQ_ACK => {
                // Bit 7 acknowledges the periodic interrupt.
                if value & 0x80 != 0 {
                    self.irq_pending = false;
                }
            }
            r::IRQACK => self.irq_pending = false,
            r::DMD_FIRQLINE | r::HIGHRESTIMER => self.firq_pending = false,
            _ => {}
        }
    }

    fn set_solenoid_group(&mut self, base: u32, value: u8) {
        let mask = 0xFFu32 << base;
        self.solenoids = (self.solenoids & !mask) | (u32::from(value) << base);
    }

    // --- State -----------------------------------------------------------------

    /// Raw contents of a register.
    pub fn register(&self, reg: usize) -> u8 {
        self.data.get(reg).copied().unwrap_or(0)
    }

    /// Whether the game enabled the periodic interrupt (bit 2 of the zero
    /// cross register).
    ///
    /// At power-on it is off: the boot sequence runs without interrupts until
    /// the ROM decides to turn them on.
    pub fn periodic_irq_enabled(&self) -> bool {
        self.data[r::ZC_IRQ_ACK] & 0x04 != 0
    }

    pub fn raise_irq(&mut self) {
        self.irq_pending = true;
    }

    pub fn raise_firq(&mut self) {
        self.firq_pending = true;
    }

    pub fn irq(&self) -> bool {
        self.irq_pending
    }

    pub fn firq(&self) -> bool {
        self.firq_pending
    }

    /// Flags a zero crossing of the AC line.
    pub fn tick_zero_cross(&mut self) {
        self.zero_cross = true;
    }

    /// The board's diagnostic LED, on bit 7.
    pub fn diagnostic_led(&self) -> bool {
        self.data[r::LED] & 0x80 != 0
    }

    /// The 28 solenoids as a bitmap: bit 0 is solenoid 1.
    pub fn solenoids(&self) -> u32 {
        self.solenoids
    }

    /// Closes the lamp strobe pass.
    pub fn refresh_lamps(&mut self) {
        self.lamps = self.lamp_accum;
        self.lamp_accum = [0; 8];
    }

    /// Whether a lamp is lit, by its Williams number.
    pub fn lamp_lit(&self, number: u8) -> bool {
        matrix_position(number).is_some_and(|(column, mask)| self.lamps[column] & mask != 0)
    }

    /// How many lamps are lit.
    pub fn lit_count(&self) -> u32 {
        self.lamps.iter().map(|c| c.count_ones()).sum()
    }

    // --- Switches ------------------------------------------------------------------

    /// Opens or closes a switch by its Williams number.
    pub fn set_switch(&mut self, number: u8, closed: bool) -> bool {
        let Some((column, mask)) = matrix_position(number) else {
            return false;
        };
        if closed {
            self.switches[column] |= mask;
        } else {
            self.switches[column] &= !mask;
        }
        true
    }

    pub fn switch_closed(&self, number: u8) -> bool {
        matrix_position(number).is_some_and(|(column, mask)| self.switches[column] & mask != 0)
    }

    /// Rows that are read with the indicated columns active.
    fn switch_rows(&self, strobe: u8) -> u8 {
        let mut rows = 0;
        for (column, closed) in self.switches.iter().enumerate() {
            if strobe & (1 << column) != 0 {
                rows |= closed;
            }
        }
        rows
    }
}

/// Translates a Williams number (`column*10 + row`) into `(column index, row
/// mask)`.
pub fn matrix_position(number: u8) -> Option<(usize, u8)> {
    let column = number / 10;
    let row = number % 10;
    if !(1..=8).contains(&column) || !(1..=8).contains(&row) {
        return None;
    }
    Some((column as usize - 1, 1 << (row - 1)))
}
