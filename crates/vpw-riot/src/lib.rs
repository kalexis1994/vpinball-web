//! MOS 6532 RIOT — RAM, I/O and Timer on one chip.
//!
//! Gottlieb's System 80 has **three** of them (U4, U5 and U6), and between
//! them they are the whole machine: 128 bytes of scratch RAM each, six 8-bit
//! ports carrying the switch matrix, the lamps, the solenoids, the displays
//! and the dip switches, and a timer that provides the interrupt the game runs
//! on (`gts80.c:505-517` in PinMAME shows where each one lands in the map).
//!
//! It is the poor relation of the 6821 next door: the ports have no handshake
//! lines and no control register, just a data register and a direction
//! register each. What it has instead is the timer, and an edge detector on
//! one pin.
//!
//! # About where this code comes from
//!
//! Written from the MOS specification. `machine/6532riot.c` in PinMAME is
//! under the old MAME licence — non-commercial, and so incompatible with this
//! project's GPLv3 — and none of it was read for this. What *was* read is
//! Gottlieb's own wiring in `gts80.c`, which is a description of a circuit
//! board rather than a program.
//!
//! # The register map
//!
//! Five address lines reach the chip, and they are decoded as a small mess of
//! special cases rather than as a table. That is the chip, not a simplification:
//!
//! | A4 | A3 | A2 | A1 A0 | Write | Read |
//! |----|----|----|-------|-------|------|
//! | x | x | 0 | 00 | port A output | port A pins |
//! | x | x | 0 | 01 | direction A | direction A |
//! | x | x | 0 | 10 | port B output | port B pins |
//! | x | x | 0 | 11 | direction B | direction B |
//! | 0 | x | 1 | ab | edge detect: `a` enables the interrupt, `b` picks the edge | — |
//! | 1 | e | 1 | pp | start the timer, `pp` the divider, `e` its interrupt | — |
//! | x | e | 1 | x0 | — | the timer, and `e` sets its interrupt |
//! | x | x | 1 | x1 | — | the interrupt flags |
//!
//! # The timer, which is the part worth reading twice
//!
//! Writing starts it counting down at one of four rates: every clock, every
//! 8, 64 or 1024. When it passes zero the timer flag sets **and the divider
//! goes to one** — from then on it counts down every clock from `$FF`. That
//! second half is what lets a program work out how long ago the timeout
//! happened, and a model that stops at zero instead looks fine until a ROM
//! uses it that way.
//!
//! Reading the timer clears its flag. Reading the flags clears the *edge*
//! flag and leaves the timer's alone.

/// One MOS 6532.
#[derive(Debug, Clone)]
pub struct Riot {
    /// The 128 bytes of RAM. The machine decides where they appear.
    pub ram: [u8; 128],

    out_a: u8,
    ddr_a: u8,
    /// What the outside world is driving on port A. The machine writes this
    /// before the CPU reads, which is how the switch matrix answers.
    pub in_a: u8,

    out_b: u8,
    ddr_b: u8,
    /// The same for port B.
    pub in_b: u8,

    /// The count, as the CPU would read it.
    timer: u8,
    /// Clocks per decrement: 1, 8, 64 or 1024 until it times out, and 1 after.
    divider: u32,
    /// Clocks left before the next decrement.
    remaining: u32,
    timer_flag: bool,
    timer_irq: bool,

    /// Whether the edge detector wants a rising edge rather than a falling one.
    edge_positive: bool,
    edge_irq: bool,
    edge_flag: bool,
    last_pa7: bool,
}

impl Default for Riot {
    fn default() -> Self {
        Self::new()
    }
}

impl Riot {
    pub fn new() -> Self {
        Self {
            ram: [0; 128],
            out_a: 0,
            ddr_a: 0,
            in_a: 0xFF,
            out_b: 0,
            ddr_b: 0,
            in_b: 0xFF,
            timer: 0,
            divider: 1,
            remaining: 1,
            timer_flag: false,
            timer_irq: false,
            edge_positive: false,
            edge_irq: false,
            edge_flag: false,
            // Matching `in_a`, so a chip that powers up with the pin high does
            // not report a rising edge on its first tick.
            last_pa7: true,
        }
    }

    /// What the chip drives on port A: its output bits, and nothing on the
    /// others.
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
    ///
    /// Two sources, either of which can be switched off on its own; the pin is
    /// open drain and every RIOT on the board pulls the same line, so a
    /// machine ORs the three together.
    pub fn irq(&self) -> bool {
        (self.timer_flag && self.timer_irq) || (self.edge_flag && self.edge_irq)
    }

    /// Reads one of the I/O registers. `addr` is the five lines that reach the
    /// chip; anything above them is the machine's business.
    pub fn read(&mut self, addr: u16) -> u8 {
        let a = (addr & 0x1F) as u8;
        if a & 0x04 == 0 {
            return match a & 0x03 {
                // A read of a port returns the *pins*: what the outside world
                // is driving on the input bits, and what the chip is driving
                // on its own.
                0 => (self.in_a & !self.ddr_a) | (self.out_a & self.ddr_a),
                1 => self.ddr_a,
                2 => (self.in_b & !self.ddr_b) | (self.out_b & self.ddr_b),
                _ => self.ddr_b,
            };
        }
        if a & 0x01 == 0 {
            // Reading the timer sets whether it may interrupt — the address
            // line is the switch, which is why a ROM has two different
            // addresses for "read the timer".
            self.timer_irq = a & 0x08 != 0;
            self.timer_flag = false;
            self.timer
        } else {
            // And reading the flags clears the *edge* one only. The timer's
            // is cleared by reading the timer, and a routine that wanted both
            // has to do both.
            let flags = (u8::from(self.timer_flag) << 7) | (u8::from(self.edge_flag) << 6);
            self.edge_flag = false;
            flags
        }
    }

    /// Writes one of the I/O registers.
    pub fn write(&mut self, addr: u16, value: u8) {
        let a = (addr & 0x1F) as u8;
        if a & 0x04 == 0 {
            match a & 0x03 {
                0 => self.out_a = value,
                1 => self.ddr_a = value,
                2 => self.out_b = value,
                _ => self.ddr_b = value,
            }
            return;
        }
        if a & 0x10 != 0 {
            self.timer = value;
            self.divider = match a & 0x03 {
                0 => 1,
                1 => 8,
                2 => 64,
                _ => 1024,
            };
            self.remaining = self.divider;
            self.timer_irq = a & 0x08 != 0;
            self.timer_flag = false;
        } else {
            self.edge_positive = a & 0x01 != 0;
            self.edge_irq = a & 0x02 != 0;
        }
    }

    /// Runs the timer and the edge detector for a number of CPU clocks.
    pub fn tick(&mut self, clocks: u32) {
        for _ in 0..clocks {
            self.remaining -= 1;
            if self.remaining > 0 {
                continue;
            }
            if self.timer == 0 {
                // Past zero: the flag sets and the divider goes to one, so
                // what the CPU reads from here is how long ago that was.
                self.timer = 0xFF;
                self.timer_flag = true;
                self.divider = 1;
            } else {
                self.timer -= 1;
            }
            self.remaining = self.divider;
        }
        self.check_edge();
    }

    /// Watches PA7 for the edge it was told to look for.
    ///
    /// The pin is a real input even when the port is programmed as an output,
    /// so this reads [`Self::in_a`] and not the output register.
    fn check_edge(&mut self) {
        let pa7 = self.in_a & 0x80 != 0;
        let edge = if self.edge_positive {
            !self.last_pa7 && pa7
        } else {
            self.last_pa7 && !pa7
        };
        if edge {
            self.edge_flag = true;
        }
        self.last_pa7 = pa7;
    }
}
