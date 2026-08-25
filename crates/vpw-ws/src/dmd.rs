//! The dot matrix board: 128 by 32, and a processor of its own to drive it.
//!
//! Sega and Stern used the same display board as Data East before them —
//! 520-5055-00 for Data East, -01 to -03 for Whitestar — and it is a whole
//! second computer. A 6809 at 2 MHz with half a megabyte of its own ROM, its
//! own RAM, and a CRTC6845 to scan the panel. The CPU board does not draw
//! anything: it hands this one a command byte and gets on with the game.
//!
//! # Three shades out of a one-bit panel
//!
//! The panel can only light a dot or not, and the display has four levels. The
//! board gets them by rasterising **every row twice** — once with the CRTC
//! clocked at 500 kHz and once at 1 MHz — from two frames stored a page apart
//! in RAM. A dot lit in both is on for the whole cycle; lit only in the slow
//! frame it is on for two thirds; only in the fast one, a third; in neither,
//! dark. So a pixel here is the pair of bits weighted two and one, which is
//! why [`Dmd::frame`] answers 0 to 3 and not a boolean.
//!
//! The original's own note on this is worth keeping: *"large parts of the
//! following have been deduced from digital recordings and schematics
//! analysis, so may be entirely wrong"* (`dedmd.c:135`).

use vpw_bus::Bus;
use vpw_m6809::Cpu;

/// The panel, in dots.
pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 32;

/// One frame's worth of RAM: one bit per dot.
const PLANE: usize = WIDTH * HEIGHT / 8;

/// How long a frame takes, in cycles of the board's own 2 MHz clock.
///
/// Not calculated — measured. The theory of operation gives 80.1 Hz, and the
/// original notes that real hardware spends a few more cycles switching the
/// CRTC's clock divider, differently on the two revisions: 25620 for Data
/// East's board and 25720 for this one, which is 77.77 Hz (`dedmd.c:216`).
const FRAME_CYCLES: u64 = 25_720;

/// How much ROM the board addresses. Thirty-two banks of sixteen kilobytes.
const ROM_REGION: usize = 0x8_0000;

/// The display board.
pub struct Dmd {
    cpu: Cpu,
    board: Board,
    cycle: u64,
    next_frame: u64,
    /// The last frame that finished, one byte per dot, 0 to 3.
    frame: Box<[u8; WIDTH * HEIGHT]>,
    /// How many frames have been produced. A host that wants to know whether
    /// the picture is moving can watch this instead of comparing pixels.
    pub frames: u64,
    /// How many times the CPU board has pulsed this one's reset line. A
    /// display that never resets and a display that resets constantly look the
    /// same from outside — blank — and this is what tells them apart.
    pub resets: u64,
}

struct Board {
    ram: Box<[u8; 0x2000]>,
    /// Eight pages of four kilobytes, one of which shows at `$2000`. The frame
    /// being displayed lives in here.
    vram: Box<[u8; 8 * 0x1000]>,
    vram_bank: usize,
    rom: Box<[u8; ROM_REGION]>,
    rom_bank: usize,
    /// The CRTC's address latch and its registers. Only the start address is
    /// read back — see [`Board::start_address`] — but they are all stored,
    /// because the ROM reads them back and a register that answers zero is a
    /// register the ROM will write again for ever.
    crtc_address: u8,
    crtc: [u8; 32],
    /// The command the CPU board has sent, and the one it is sending.
    cmd: u8,
    next_cmd: u8,
    /// What the CPU board sees: busy while a command is unread, and the four
    /// bits this board answers with.
    pub busy: bool,
    pub status: u8,
    /// The control lines from the CPU board, kept so the edges can be found.
    ctrl: u8,
    /// The interrupt this board takes when a command arrives. Held until the
    /// command is read, which is what `dmd32_io_r` does.
    irq: bool,
}

impl Board {
    /// Where in the display RAM the frame starts.
    ///
    /// The CRTC's start address, registers 12 and 13. The board wires the
    /// CRTC's address lines to the RAM shifted by two, and the low eight bits
    /// are not wired at all — the original leaves that unimplemented on the
    /// grounds that no game uses it, and asserts as much (`dedmd.c:164`).
    fn start_address(&self) -> usize {
        let base = (usize::from(self.crtc[12]) << 8) | usize::from(self.crtc[13]);
        (base << 2) & 0x7c00
    }
}

impl Bus for Board {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)],
            0x2000..=0x2fff => self.vram[self.vram_bank * 0x1000 + usize::from(addr - 0x2000)],
            0x3000..=0x3fff => match addr & 3 {
                // Reading the address register is not a thing the chip does,
                // and the original notes that every write sequence is followed
                // by one anyway.
                0 => 0,
                1 => self.crtc[usize::from(self.crtc_address & 31)],
                // Taking the command clears the interrupt and tells the CPU
                // board this one is free again.
                _ => {
                    self.busy = false;
                    self.irq = false;
                    self.cmd
                }
            },
            0x4000..=0x7fff => {
                self.rom[(self.rom_bank * 0x4000 + usize::from(addr - 0x4000)) % ROM_REGION]
            }
            // The last thirty-two kilobytes of the region, always.
            0x8000..=0xffff => self.rom[ROM_REGION - 0x8000 + usize::from(addr - 0x8000)],
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1fff => self.ram[usize::from(addr)] = value,
            0x2000..=0x2fff => {
                let at = self.vram_bank * 0x1000 + usize::from(addr - 0x2000);
                self.vram[at] = value;
            }
            0x3000..=0x3fff => match addr & 3 {
                0 => self.crtc_address = value,
                1 => self.crtc[usize::from(self.crtc_address & 31)] = value,
                // One byte selects both banks: the top three bits pick the
                // page of display RAM, the bottom five the page of ROM.
                _ => {
                    self.vram_bank = usize::from(value >> 5);
                    self.rom_bank = usize::from(value & 0x1f);
                }
            },
            // Writing anywhere in the ROM window is how this board answers the
            // other one. Four bits of it.
            0x4000..=0x7fff => self.status = value & 0x0f,
            0x8000..=0xffff => {}
        }
    }
}

impl Dmd {
    pub fn new() -> Self {
        let mut board = Board {
            ram: Box::new([0; 0x2000]),
            vram: Box::new([0; 8 * 0x1000]),
            vram_bank: 0,
            rom: Box::new([0xff; ROM_REGION]),
            rom_bank: 0,
            crtc_address: 0,
            crtc: [0; 32],
            cmd: 0,
            next_cmd: 0,
            busy: false,
            status: 0,
            ctrl: 0,
            irq: false,
        };
        let mut cpu = Cpu::new();
        cpu.reset(&mut board);
        Self {
            cpu,
            board,
            cycle: 0,
            next_frame: FRAME_CYCLES,
            frame: Box::new([0; WIDTH * HEIGHT]),
            frames: 0,
            resets: 0,
        }
    }

    /// Loads the display ROM, mirroring a short image to fill the region.
    pub fn load_rom(&mut self, image: &[u8]) -> Result<(), String> {
        if image.is_empty() || image.len() > ROM_REGION {
            return Err(format!(
                "a display image of {} bytes does not fit the {ROM_REGION} byte region",
                image.len()
            ));
        }
        let mut at = 0;
        while at < ROM_REGION {
            let n = image.len().min(ROM_REGION - at);
            self.board.rom[at..at + n].copy_from_slice(&image[..n]);
            at += n;
        }
        self.reset();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.board.vram_bank = 0;
        self.board.rom_bank = 0;
        self.board.irq = false;
        self.cpu.reset(&mut self.board);
    }

    /// The CPU board writing the command latch at `$3600`.
    ///
    /// `dmdlatch_w` (`se.c:725`) is three lines: the data, then `ctrl = 0`,
    /// then `ctrl = 1`. Those are **literal** zeroes and ones, not the reset
    /// bit preserved and the command bit toggled, and the difference is this
    /// board's whole boot sequence. `dmdreset_w` leaves bit 1 set
    /// (`se.c:729`); this board resets on bit 1 going *down*
    /// (`dmd32_ctrl_w`, `dedmd.c:129`); and this is the only place it ever
    /// does. Preserve bit 1 here and the display processor never gets its
    /// reset pulse — it comes up on whatever banks it happened to have and
    /// draws nothing, with no way to tell that from a board that is simply
    /// idle.
    pub fn latch(&mut self, data: u8) {
        self.set_data(data);
        self.set_ctrl(0);
        self.set_ctrl(1);
    }

    /// The command byte the CPU board is putting on the bus. `$3600`.
    pub fn set_data(&mut self, data: u8) {
        self.board.next_cmd = data;
    }

    /// The two control lines. Bit 0 strobes a command in, bit 1 is reset.
    ///
    /// Both are edges and not levels, which is why the previous value is kept:
    /// a command arrives on bit 0 going **up** and the board resets on bit 1
    /// going **down** (`dedmd.c:123`).
    pub fn set_ctrl(&mut self, data: u8) {
        let was = self.board.ctrl;
        if was & 1 == 0 && data & 1 != 0 {
            self.board.cmd = self.board.next_cmd;
            self.board.busy = true;
            self.board.irq = true;
        }
        if was & 2 != 0 && data & 2 == 0 {
            self.resets += 1;
            self.reset();
        }
        self.board.ctrl = data;
    }

    /// Whether a command is waiting to be read.
    pub fn busy(&self) -> bool {
        self.board.busy
    }

    /// The four bits this board answers the CPU board with.
    pub fn status(&self) -> u8 {
        self.board.status
    }

    /// The last frame, one byte per dot from 0 (dark) to 3 (full).
    pub fn frame(&self) -> &[u8; WIDTH * HEIGHT] {
        &self.frame
    }

    /// Runs the board up to `target` of its own cycles.
    pub fn run_until(&mut self, target: u64) {
        while self.cycle < target {
            self.cpu.set_irq(self.board.irq);
            let cycles = self.cpu.step(&mut self.board);
            self.cycle += u64::from(cycles);

            if self.cycle >= self.next_frame {
                self.capture();
                self.next_frame += FRAME_CYCLES;
                // The original guesses that the CRTC's cursor signal is what
                // raises this, and says so. It is what paces the board's own
                // animations.
                if self.board.crtc[14] != 0 || self.board.crtc[15] != 0 {
                    self.cpu.set_firq(true);
                    let _ = self.cpu.step(&mut self.board);
                    self.cpu.set_firq(false);
                }
            }
        }
    }

    /// Reads the two frames out of display RAM and folds them into shades.
    fn capture(&mut self) {
        let src = self.board.start_address();
        let page = self.board.vram_bank * 0x1000;
        // The two are a page apart: the first is the one shown for two thirds
        // of the cycle, the second for one third.
        let slow = page + src;
        let fast = page + (src | 0x200);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let byte = y * (WIDTH / 8) + x / 8;
                let bit = 7 - (x % 8);
                let at = |base: usize| -> u8 {
                    let i = (base + byte) % self.board.vram.len();
                    (self.board.vram[i] >> bit) & 1
                };
                self.frame[y * WIDTH + x] = at(slow) * 2 + at(fast);
            }
        }
        self.frames += 1;
    }
}

impl Default for Dmd {
    fn default() -> Self {
        Self::new()
    }
}

/// How many bytes one frame occupies in display RAM.
pub const FRAME_BYTES: usize = PLANE;
