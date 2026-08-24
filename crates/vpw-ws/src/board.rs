//! The Whitestar CPU board: memory, and everything hanging off it.
//!
//! The map is the whole board and it is short (`se.c:1089`). There are no PIAs
//! — Sega dropped them when it took the design over from Data East — so every
//! port is a byte at an address and nothing has a control register:
//!
//! ```text
//! 0000-1fff  RAM, battery backed          3200  ROM bank select
//! 2000-2003  solenoids                    3300  switch column strobe
//! 2006-2007  auxiliary board              3400  switch rows
//! 2008       lamp column, low             3406  general illumination
//! 2009       lamp column, high            3500  DMD interrupt enable
//! 200a       lamp rows                    3600  DMD command latch
//! 200b       general illumination         3601  DMD reset
//! 3000       dedicated switches           3700  DMD status
//! 3100       dip switches                 3800  sound command
//! 4000-7fff  banked ROM, 32 x 16 KB
//! 8000-ffff  fixed ROM, 32 KB
//! ```

use vpw_bus::Bus;

use crate::io::{Lamps, Solenoids, Switches};

/// How much ROM the board addresses through the bank at `$4000`.
///
/// Half a megabyte, which is more than any game carries: a 128 KB image is
/// loaded four times over to fill it (`se.h:82`). The mirroring is not a
/// convenience, it is what makes the top bank bits harmless on a small game.
pub const ROM_REGION: usize = 0x8_0000;

/// The banked window, and how many banks fit in the region.
const BANK_SIZE: usize = 0x4000;
const BANK_MASK: u8 = 0x1f;

/// Where the fixed half of the map is taken from within the region.
///
/// `ROM_COPY(SE_ROMREGION, 0x18000, 0x8000, 0x8000)` — the last 32 KB of the
/// first copy of the image. Get this wrong and the CPU starts on the wrong
/// reset vector, which does not fail, it just runs somebody else's code.
const FIXED_FROM: usize = 0x1_8000;

pub struct Board {
    ram: Box<[u8; 0x2000]>,
    /// The whole addressable ROM region, already mirrored.
    rom: Box<[u8; ROM_REGION]>,
    bank: u8,

    pub lamps: Lamps,
    pub switches: Switches,
    pub solenoids: Solenoids,

    /// The eight dip switches on the board. Read at `$3100`.
    pub dips: u8,
    /// General illumination, `$200b` and `$3406`.
    pub gi: [u8; 3],

    /// What the CPU last sent the display board, and whether that is new.
    ///
    /// The display is a board of its own with its own processor, reached
    /// through a one-byte latch and two control lines. Writing the latch is a
    /// *command*, so it is flagged rather than merely stored: the machine hands
    /// it over on the next step and strobes it in.
    pub dmd_latch: u8,
    pub dmd_data_pending: bool,
    pub dmd_ctrl: u8,
    pub dmd_ctrl_pending: Option<u8>,
    pub dmd_enabled: bool,
    /// What the display board answers at `$3700`: busy in bit 7 and its four
    /// status bits from bit 3 (`se.c:753`).
    pub dmd_status: u8,
    /// The last sound command, `$3800`, and whether the sound board has been
    /// told about it yet.
    pub sound_latch: u8,
    pub sound_pending: bool,
    /// Set by bit 7 of the bank register, which is also the diagnostic LED.
    pub diagnostic_led: bool,

    /// An address the map does not cover, kept for whoever is debugging. The
    /// original logs these; a silently absent port is how a board runs for
    /// twenty minutes and then does something inexplicable.
    pub last_unmapped: Option<(u16, bool)>,
}

impl Board {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0; 0x2000]),
            rom: Box::new([0xff; ROM_REGION]),
            bank: 0,
            lamps: Lamps::default(),
            switches: Switches::default(),
            solenoids: Solenoids::default(),
            // All open, which is what an untouched board reads.
            dips: 0xff,
            gi: [0; 3],
            dmd_latch: 0,
            dmd_data_pending: false,
            dmd_ctrl: 0,
            dmd_ctrl_pending: None,
            dmd_enabled: false,
            dmd_status: 0,
            sound_latch: 0,
            sound_pending: false,
            diagnostic_led: false,
            last_unmapped: None,
        }
    }

    /// Loads the game's CPU image, mirroring it to fill the region.
    ///
    /// `se.h:82` writes the mirroring out by hand — one `ROM_LOAD` and three
    /// `ROM_RELOAD` — because the region is fixed at half a megabyte whatever
    /// size the game is.
    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), String> {
        if image.is_empty() || image.len() > ROM_REGION {
            return Err(format!(
                "a CPU image of {} bytes does not fit the {ROM_REGION} byte region",
                image.len()
            ));
        }
        let mut at = 0;
        while at < ROM_REGION {
            let n = image.len().min(ROM_REGION - at);
            self.rom[at..at + n].copy_from_slice(&image[..n]);
            at += n;
        }
        Ok(())
    }

    /// Which 16 KB bank is showing at `$4000`.
    pub fn bank(&self) -> u8 {
        self.bank
    }

    pub fn ram(&self) -> &[u8] {
        &self.ram[..]
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram[..]
    }

    fn banked(&self, addr: u16) -> u8 {
        let base = usize::from(self.bank & BANK_MASK) * BANK_SIZE;
        self.rom[(base + usize::from(addr - 0x4000)) % ROM_REGION]
    }

    fn fixed(&self, addr: u16) -> u8 {
        self.rom[FIXED_FROM + usize::from(addr - 0x8000)]
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for Board {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)],
            0x2000..=0x2003 => self.solenoids.read((addr & 3) as u8),
            0x2007 => self.solenoids.aux(),
            0x2008 => (self.lamps.column() & 0xff) as u8,
            0x2009 => (self.lamps.column() >> 8) as u8,
            // A write location that GoldenEye reads anyway, so the original
            // wires it up and answers what was last written (`se.c:615`).
            0x200a => self.lamps.rows().reverse_bits(),
            0x3000 => self.switches.dedicated(),
            0x3100 => self.dips,
            0x3400 => self.switches.read(),
            0x3406 => self.gi[0],
            0x3407 => self.gi[1],
            // The display board's two lines. Nothing drives it yet.
            0x3500 => 0,
            0x3700 => self.dmd_status,
            0x4000..=0x7fff => self.banked(addr),
            0x8000..=0xffff => self.fixed(addr),
            _ => {
                self.last_unmapped = Some((addr, false));
                0
            }
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)] = value,
            0x2000..=0x2003 => self.solenoids.write((addr & 3) as u8, value),
            0x2006..=0x2007 => self.solenoids.set_aux(value),
            0x2008 => self.lamps.set_column_low(value),
            0x2009 => self.lamps.set_column_high(value),
            0x200a => self.lamps.set_rows(value),
            0x200b => self.gi[2] = value,
            0x3200 => {
                self.bank = value;
                // Bit 7 is the diagnostic LED, inverted (`se.c:568`).
                self.diagnostic_led = value & 0x80 == 0;
            }
            0x3300 => self.switches.strobe(value),
            0x3406 => self.gi[0] = value,
            0x3407 => self.gi[1] = value,
            0x3500 => self.dmd_enabled = value != 0,
            0x3600 => {
                self.dmd_latch = value;
                self.dmd_data_pending = true;
            }
            // Reset, on bit 1 (`dmdreset_w`, `se.c:729`).
            0x3601 => {
                self.dmd_ctrl_pending = Some(if value != 0 { 0x02 } else { 0x00 });
            }
            0x3800 => {
                self.sound_latch = value;
                self.sound_pending = true;
            }
            // Writing to ROM is not an error: there is code that walks memory
            // without discriminating.
            0x3801 | 0x4000..=0xffff => {}
            _ => self.last_unmapped = Some((addr, true)),
        }
    }
}
