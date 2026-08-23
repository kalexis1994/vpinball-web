//! 128x32 dot matrix display.
//!
//! It is the most visible change from System 11: instead of fourteen segment
//! characters there is a matrix of 4096 dots, and the game draws into it as if
//! it were a framebuffer.
//!
//! The CPU does not see the display's 8 KB of memory all at once: it sees them
//! **through two 512-byte windows**, and picks which page shows up in each one.
//! With 16 pages that is enough to build one frame while another one is being
//! shown, which is how the game gets flicker-free animation.

/// Width of the display, in dots.
pub const DMD_WIDTH: usize = 128;
/// Height of the display, in dots.
pub const DMD_HEIGHT: usize = 32;
/// Each page covers one frame: 128x32 bits.
pub const DMD_PAGE_SIZE: usize = DMD_WIDTH * DMD_HEIGHT / 8;
/// How many pages the display memory has.
pub const DMD_PAGES: usize = 16;

const DMD_RAM: usize = DMD_PAGE_SIZE * DMD_PAGES;
/// Each window the CPU sees is 512 bytes wide.
const WINDOW: usize = 0x200;

pub struct Dmd {
    ram: Box<[u8; DMD_RAM]>,
    /// Page showing at `$3800`.
    low_page: usize,
    /// Page showing at `$3A00`.
    high_page: usize,
    /// Page that is being displayed.
    visible_page: usize,
    /// How many times the visible page changed. It is the most direct signal
    /// that the game is animating something.
    frames: u64,
}

impl Default for Dmd {
    fn default() -> Self {
        Self::new()
    }
}

impl Dmd {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0; DMD_RAM]),
            low_page: 0,
            high_page: 0,
            visible_page: 0,
            frames: 0,
        }
    }

    pub fn reset(&mut self) {
        self.ram.fill(0);
        self.low_page = 0;
        self.high_page = 0;
        self.visible_page = 0;
        self.frames = 0;
    }

    pub fn select_low(&mut self, page: u8) {
        self.low_page = usize::from(page) % DMD_PAGES;
    }

    pub fn select_high(&mut self, page: u8) {
        self.high_page = usize::from(page) % DMD_PAGES;
    }

    /// Picks which page gets drawn on the next strobe pass.
    pub fn show(&mut self, page: u8) {
        let page = usize::from(page) % DMD_PAGES;
        if page != self.visible_page {
            self.frames += 1;
        }
        self.visible_page = page;
    }

    pub fn read_low(&self, offset: u16) -> u8 {
        self.ram[self.low_page * WINDOW + offset as usize]
    }

    pub fn read_high(&self, offset: u16) -> u8 {
        self.ram[self.high_page * WINDOW + offset as usize]
    }

    pub fn write_low(&mut self, offset: u16, value: u8) {
        self.ram[self.low_page * WINDOW + offset as usize] = value;
    }

    pub fn write_high(&mut self, offset: u16, value: u8) {
        self.ram[self.high_page * WINDOW + offset as usize] = value;
    }

    /// How many visible page changes there were since the reset.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn visible_page(&self) -> usize {
        self.visible_page
    }

    /// Whether a dot of the visible frame is lit.
    pub fn pixel(&self, x: usize, y: usize) -> bool {
        if x >= DMD_WIDTH || y >= DMD_HEIGHT {
            return false;
        }
        let bit = y * DMD_WIDTH + x;
        let byte = self.ram[self.visible_page * WINDOW + bit / 8];
        byte & (1 << (bit % 8)) != 0
    }

    /// How many dots are lit in the visible frame.
    pub fn lit_pixels(&self) -> u32 {
        let page = &self.ram[self.visible_page * WINDOW..(self.visible_page + 1) * WINDOW];
        page.iter().map(|b| b.count_ones()).sum()
    }

    /// The visible frame drawn with characters, to look at it in a terminal.
    pub fn render_text(&self) -> String {
        let mut out = String::with_capacity((DMD_WIDTH + 1) * DMD_HEIGHT);
        for y in 0..DMD_HEIGHT {
            for x in 0..DMD_WIDTH {
                out.push(if self.pixel(x, y) { '#' } else { ' ' });
            }
            out.push('\n');
        }
        out
    }
}
