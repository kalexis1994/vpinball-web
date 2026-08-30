//! MOS 6530 RRIOT — the 6532's older, smaller sibling.
//!
//! Gottlieb's System 80 **sound** board is a 6502 and one of these, and that
//! is the whole card: 64 bytes of RAM, a timer, and two 8-bit ports of which
//! one is wired straight to a resistor ladder and *is* the loudspeaker.
//!
//! # What is different from a 6532
//!
//! Half the RAM, an extra kilobyte of mask ROM on the die, and — the part that
//! matters to whoever writes the decode — **four address lines instead of
//! five**. The 6532 uses A4 to tell "start the timer" from "set the edge
//! detector"; a 6530 has no programmable edge detector, so every write with A2
//! set starts the timer.
//!
//! | A3 | A2 | A1 A0 | Write | Read |
//! |----|----|-------|-------|------|
//! | x | 0 | 00 | port A output | port A pins |
//! | x | 0 | 01 | direction A | direction A |
//! | x | 0 | 10 | port B output | port B pins |
//! | x | 0 | 11 | direction B | direction B |
//! | e | 1 | pp | start the timer, `pp` the divider, `e` its interrupt | — |
//! | e | 1 | x0 | — | the timer, and `e` sets its interrupt |
//! | x | 1 | x1 | — | the interrupt flags |
//!
//! # About where this code comes from
//!
//! Written from the MOS specification, like the 6532 next door. PinMAME's
//! `machine/6530riot.c` is under the old MAME licence and none of it was read
//! for this; Gottlieb's own wiring in `gts80s.c` was, because a schematic is
//! not a program.

use crate::timer::Timer;

/// The RAM on the die. The board decides where it appears, and a System 80
/// sound board makes it appear in a great many places at once.
pub const RAM: usize = 64;

/// One MOS 6530.
#[derive(Debug, Clone)]
pub struct Riot6530 {
    pub ram: [u8; RAM],

    out_a: u8,
    ddr_a: u8,
    /// What the outside world is driving on port A.
    pub in_a: u8,

    out_b: u8,
    ddr_b: u8,
    /// The same for port B — on a sound board, the command from the game.
    pub in_b: u8,

    timer: Timer,
}

impl Default for Riot6530 {
    fn default() -> Self {
        Self::new()
    }
}

impl Riot6530 {
    pub fn new() -> Self {
        Self {
            ram: [0; RAM],
            out_a: 0,
            ddr_a: 0,
            in_a: 0xFF,
            out_b: 0,
            ddr_b: 0,
            in_b: 0xFF,
            timer: Timer::new(),
        }
    }

    pub fn port_a_output(&self) -> u8 {
        self.out_a & self.ddr_a
    }

    pub fn port_b_output(&self) -> u8 {
        self.out_b & self.ddr_b
    }

    pub fn ddr_a(&self) -> u8 {
        self.ddr_a
    }

    pub fn ddr_b(&self) -> u8 {
        self.ddr_b
    }

    /// Whether the chip is asking for an interrupt.
    pub fn irq(&self) -> bool {
        self.timer.interrupting()
    }

    /// Reads a register. `addr` is the four lines that reach the chip.
    pub fn read(&mut self, addr: u16) -> u8 {
        let a = (addr & 0x0F) as u8;
        if a & 0x04 == 0 {
            return match a & 0x03 {
                0 => (self.in_a & !self.ddr_a) | (self.out_a & self.ddr_a),
                1 => self.ddr_a,
                2 => (self.in_b & !self.ddr_b) | (self.out_b & self.ddr_b),
                _ => self.ddr_b,
            };
        }
        if a & 0x01 == 0 {
            self.timer.read(a & 0x08 != 0)
        } else {
            // Bit 6 is the PA7 flag on a chip that was masked for one. A
            // System 80 sound board's PA7 is the top bit of the loudspeaker,
            // so there is nothing there to report.
            u8::from(self.timer.flag()) << 7
        }
    }

    /// Writes a register.
    pub fn write(&mut self, addr: u16, value: u8) {
        let a = (addr & 0x0F) as u8;
        if a & 0x04 == 0 {
            match a & 0x03 {
                0 => self.out_a = value,
                1 => self.ddr_a = value,
                2 => self.out_b = value,
                _ => self.ddr_b = value,
            }
        } else {
            self.timer.start(value, a, a & 0x08 != 0);
        }
    }

    /// Runs the timer for a number of CPU clocks.
    pub fn tick(&mut self, clocks: u32) {
        self.timer.tick(clocks);
    }
}
