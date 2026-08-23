//! Williams Pinball Controller (WPC): the hardware of the '90s.
//!
//! It is the platform of Twilight Zone, The Addams Family, Terminator 2 and a
//! hundred more — the tables that dominate the VPX recreations.
//!
//! # How it resembles System 11, and how it doesn't
//!
//! The CPU is an **MC6809E at 2 MHz**, the same one as System 11's CS sound
//! board. But the resemblance ends there: there are not six PIAs but **a
//! single ASIC** that concentrates all the I/O into 80 addresses, the ROM is
//! paged in 16 KB chunks so it can go from 32 KB to a megabyte, and the
//! display is a 128x32 dot matrix instead of fourteen-segment characters.
//!
//! # Memory map (`wpc.c:155-180`)
//!
//! | Range | What is there |
//! |-------|---------------|
//! | `$0000`-`$2FFF` | RAM, with the upper part battery-backed |
//! | `$3000`-`$37FF` | DMD pages (WPC-95 only) |
//! | `$3800`-`$3BFF` | Two DMD pages, selectable |
//! | `$3C00`-`$3FAF` | More RAM |
//! | `$3FB0`-`$3FFF` | The ASIC registers |
//! | `$4000`-`$7FFF` | Paged ROM window |
//! | `$8000`-`$FFFF` | Fixed ROM: the last 32 KB of the image |
//!
//! # Origin
//!
//! Ported from PinMAME's `src/wpc/wpc.c`, which is under BSD-3-Clause.

mod asic;
mod dmd;

pub use asic::{Asic, registers};
pub use dmd::{DMD_HEIGHT, DMD_PAGE_SIZE, DMD_WIDTH, Dmd};
pub use vpw_m6809::{Bus, Cpu};

/// CPU clock (`wpc.c:371`).
pub const CPU_CLOCK_HZ: u32 = 2_000_000;

/// Frequency of the ASIC's periodic interrupt (`wpc.c:47`).
///
/// It is 8 MHz divided by 8192: about 976 Hz.
pub const IRQ_FREQ_HZ: f64 = 8_000_000.0 / 8192.0;

/// How many CPU cycles between interrupts.
pub const IRQ_PERIOD_CYCLES: u64 = (CPU_CLOCK_HZ as f64 / IRQ_FREQ_HZ) as u64;

/// RAM: `$0000`-`$3FAF`, with the DMD and ASIC holes in it.
const RAM_SIZE: usize = 0x4000;
/// The paged window is 16 KB wide.
const BANK_SIZE: usize = 0x4000;
/// The fixed ROM is the last 32 KB of the image.
const FIXED_SIZE: usize = 0x8000;

/// Base of the ASIC registers.
const ASIC_BASE: u16 = 0x3FB0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomError {
    /// The image is not a power of two between 128 KB and 4 MB.
    BadSize(usize),
}

impl std::fmt::Display for RomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSize(len) => write!(f, "an image of {len} bytes is not a valid WPC ROM size"),
        }
    }
}

impl std::error::Error for RomError {}

/// The bus the 6809 sees.
pub struct WpcBus {
    ram: Box<[u8; RAM_SIZE]>,
    rom: Vec<u8>,
    /// The last 32 KB of the image, always visible at `$8000`.
    fixed: Vec<u8>,
    /// Offset of the `$4000` window.
    bank: usize,
    /// How many bits of the bank register are significant. It comes from the
    /// size of the image (`wpc.c:1189`).
    page_mask: u8,

    pub asic: Asic,
    pub dmd: Dmd,

    /// Last access to an undecoded address.
    pub last_unmapped: Option<u16>,
}

impl Default for WpcBus {
    fn default() -> Self {
        Self::new()
    }
}

impl WpcBus {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0; RAM_SIZE]),
            rom: Vec::new(),
            fixed: vec![0xFF; FIXED_SIZE],
            bank: 0,
            page_mask: 0x3F,
            asic: Asic::new(),
            dmd: Dmd::new(),
            last_unmapped: None,
        }
    }

    /// Loads the game's ROM image.
    ///
    /// It is a single image, from 128 KB to 1 MB. The last 32 KB stay fixed at
    /// `$8000` and the rest is seen 16 KB at a time through the paged window
    /// (`wpc.c:1536-1540`).
    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), RomError> {
        if image.len() < 0x2_0000 || image.len() > 0x40_0000 || !image.len().is_power_of_two() {
            return Err(RomError::BadSize(image.len()));
        }
        self.rom = image.to_vec();
        self.fixed = image[image.len() - FIXED_SIZE..].to_vec();

        // The page mask comes from the size: how many 16 KB banks fit in it.
        const MASKS: [u8; 8] = [0x07, 0x0F, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x3F];
        self.page_mask = MASKS[((image.len() >> 17).wrapping_sub(1)) & 0x07];
        self.bank = 0;
        Ok(())
    }

    /// Write to the bank register: picks which 16 KB show up at `$4000`.
    fn select_bank(&mut self, value: u8) {
        self.bank = usize::from(value & self.page_mask) * BANK_SIZE;
    }

    fn read_bank(&self, addr: u16) -> u8 {
        let offset = self.bank + (addr as usize - 0x4000);
        self.rom.get(offset).copied().unwrap_or(0xFF)
    }
}

impl Bus for WpcBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // The two DMD windows, each with its own selected page.
            0x3800..=0x39FF => self.dmd.read_low(addr - 0x3800),
            0x3A00..=0x3BFF => self.dmd.read_high(addr - 0x3A00),
            0x3FB0..=0x3FFF => self.asic.read((addr - ASIC_BASE) as usize),
            0x0000..=0x3FAF => self.ram[addr as usize],
            0x4000..=0x7FFF => self.read_bank(addr),
            0x8000..=0xFFFF => self.fixed[addr as usize - 0x8000],
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x3800..=0x39FF => self.dmd.write_low(addr - 0x3800, value),
            0x3A00..=0x3BFF => self.dmd.write_high(addr - 0x3A00, value),
            0x3FB0..=0x3FFF => {
                let reg = (addr - ASIC_BASE) as usize;
                self.asic.write(reg, value);
                match reg {
                    registers::ROMBANK => self.select_bank(value),
                    registers::DMD_PAGE3800 => self.dmd.select_low(value),
                    registers::DMD_PAGE3A00 => self.dmd.select_high(value),
                    registers::DMD_SHOWPAGE => self.dmd.show(value),
                    _ => {}
                }
            }
            0x0000..=0x3FAF => self.ram[addr as usize] = value,
            // Writing to the ROM does nothing.
            0x4000..=0xFFFF => {}
        }
    }
}

/// The complete WPC machine.
pub struct Wpc {
    pub cpu: Cpu,
    pub bus: WpcBus,
    cycle: u64,
    last_irq: u64,
}

impl Default for Wpc {
    fn default() -> Self {
        Self::new()
    }
}

impl Wpc {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            bus: WpcBus::new(),
            cycle: 0,
            last_irq: 0,
        }
    }

    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), RomError> {
        self.bus.load_rom(image)
    }

    pub fn reset(&mut self) {
        self.bus.asic.reset();
        self.bus.dmd.reset();
        self.cycle = 0;
        self.last_irq = 0;
        self.cpu.reset(&mut self.bus);
    }

    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    pub fn elapsed_seconds(&self) -> f64 {
        self.cycle as f64 / f64::from(CPU_CLOCK_HZ)
    }

    /// Runs one instruction.
    pub fn step(&mut self) -> u32 {
        // The periodic interrupt only fires if the game enabled it through bit
        // 2 of the zero cross register (`wpc.c:1150`). At power-on it is off,
        // so the boot sequence runs without interrupts until the ROM decides
        // to turn them on.
        if self.cycle - self.last_irq >= IRQ_PERIOD_CYCLES {
            self.last_irq += IRQ_PERIOD_CYCLES;
            if self.bus.asic.periodic_irq_enabled() {
                self.bus.asic.raise_irq();
            }
            // The zero crossing of the 60 Hz line also goes through here.
            self.bus.asic.tick_zero_cross();
        }

        self.cpu.set_irq(self.bus.asic.irq());
        self.cpu.set_firq(self.bus.asic.firq());

        let cycles = self.cpu.step(&mut self.bus);
        self.cycle += u64::from(cycles);
        cycles
    }

    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let target = self.cycle + cycles;
        while self.cycle < target {
            self.step();
        }
        self.cycle
    }

    pub fn run_seconds(&mut self, seconds: f64) -> u64 {
        self.run_cycles((seconds * f64::from(CPU_CLOCK_HZ)) as u64)
    }

    /// Opens or closes a playfield switch, using Williams' numbering.
    pub fn set_switch(&mut self, number: u8, closed: bool) -> bool {
        self.bus.asic.set_switch(number, closed)
    }

    /// State of the lamps from the last strobe pass.
    pub fn lamp_lit(&self, number: u8) -> bool {
        self.bus.asic.lamp_lit(number)
    }
}
