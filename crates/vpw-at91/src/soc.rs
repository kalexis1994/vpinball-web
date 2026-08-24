//! The peripherals around the processor.
//!
//! An AT91M40800 is an ARM7TDMI with a bus interface, an interrupt controller,
//! three timers, a parallel port, two serial ports and a watchdog. They all
//! live in the top megabyte of the address space, sixteen kilobytes apart:
//!
//! ```text
//! ffe00000  EBI   external bus interface, and the remap
//! fffcc000  US0   serial
//! fffd0000  US1   serial
//! fffe0000  TC    three timer channels
//! ffff0000  PIO   thirty-two parallel lines
//! ffff4000  PS    power saving, which is also the timers' clock enables
//! ffff8000  WD    watchdog
//! fffff000  AIC   interrupt controller
//! ```
//!
//! What a sound board actually needs is the remap, the interrupt controller
//! and the timers: the timers pace the samples and the interrupt controller is
//! how the processor hears about them. The serial ports and the watchdog are
//! answered so that firmware writing to them carries on rather than hanging on
//! a status bit, and are not modelled beyond that.

/// The interrupt sources, by their bit in the controller.
pub mod irq {
    pub const FIQ: u32 = 0;
    pub const SOFTWARE: u32 = 1;
    pub const TC0: u32 = 4;
    pub const TC1: u32 = 5;
    pub const TC2: u32 = 6;
    pub const WATCHDOG: u32 = 7;
    pub const PIO: u32 = 8;
}

/// The advanced interrupt controller.
///
/// Eight priority levels, a vector per source, and a stack of what is being
/// serviced. The processor sees one line; this decides which source owns it.
#[derive(Debug, Default, Clone)]
pub struct Aic {
    /// Which sources are asserted, whether or not they are enabled.
    pub pending: u32,
    pub enabled: u32,
    /// Sources whose interrupt is held until written to the clear register.
    pub level_sensitive: u32,
    pub vectors: [u32; 32],
    pub modes: [u32; 32],
    /// The source currently being serviced, if any, and the ones under it.
    stack: Vec<u32>,
    /// What the last vector read answered, which is what the processor
    /// branches through.
    pub current_vector: u32,
    pub spurious_vector: u32,
}

impl Aic {
    /// Whether the processor's IRQ line should be asserted.
    ///
    /// Anything pending and enabled, with a higher priority than whatever is
    /// already being serviced.
    pub fn asserted(&self) -> bool {
        self.next().is_some()
    }

    /// The highest-priority source waiting, if it outranks what is in service.
    fn next(&self) -> Option<u32> {
        let ready = self.pending & self.enabled;
        if ready == 0 {
            return None;
        }
        let priority = |src: u32| self.modes[src as usize] & 7;
        let in_service = self.stack.last().map(|&s| priority(s));
        let mut best: Option<u32> = None;
        // Source 0 is the fast interrupt and does not come through here.
        // Walking the set bits rather than all thirty-two matters: a processor
        // asks whether an interrupt is waiting between every pair of
        // instructions, and on this board the answer is almost always "one
        // source, or none".
        let mut ready = ready & !(1 << irq::FIQ);
        while ready != 0 {
            let src = ready.trailing_zeros();
            ready &= ready - 1;
            if best.is_none_or(|b| priority(src) > priority(b)) {
                best = Some(src);
            }
        }
        match (best, in_service) {
            (Some(b), Some(s)) if priority(b) <= s => None,
            (b, _) => b,
        }
    }

    /// The processor has taken the interrupt and is reading the vector.
    pub fn acknowledge(&mut self) -> u32 {
        match self.next() {
            Some(src) => {
                self.stack.push(src);
                // An edge-triggered source is cleared by being taken; a
                // level-sensitive one stays until its peripheral is dealt with.
                if self.level_sensitive & (1 << src) == 0 {
                    self.pending &= !(1 << src);
                }
                self.current_vector = self.vectors[src as usize];
                self.current_vector
            }
            None => {
                self.current_vector = self.spurious_vector;
                self.current_vector
            }
        }
    }

    /// The handler has finished with the source it was servicing.
    pub fn end_of_interrupt(&mut self) {
        self.stack.pop();
    }

    /// The source being serviced, if any.
    pub fn in_service(&self) -> Option<u32> {
        self.stack.last().copied()
    }

    /// Whether the fast interrupt is asking, which is a line of its own and
    /// not part of the priority scheme the rest go through.
    pub fn fast_asserted(&self) -> bool {
        self.pending & self.enabled & (1 << irq::FIQ) != 0
    }

    /// What the fast interrupt's vector register answers.
    pub fn fast_vector(&self) -> u32 {
        self.vectors[0]
    }

    pub fn raise(&mut self, source: u32) {
        self.pending |= 1 << source;
    }

    pub fn clear(&mut self, source: u32) {
        self.pending &= !(1 << source);
    }

    /// The clear register takes a mask, not a number.
    pub fn clear_many(&mut self, mask: u32) {
        self.pending &= !mask;
    }
}

/// One of the three timer channels.
///
/// Only what a sound board uses: count up at the clock the mode register
/// selects, compare against C, raise an interrupt and — if asked — start again.
/// That is what turns into a sample rate.
#[derive(Debug, Default, Clone)]
pub struct Timer {
    pub counter: u32,
    pub mode: u32,
    pub rc: u32,
    pub ra: u32,
    pub rb: u32,
    /// Which of the events have happened since the status was last read.
    pub status: u32,
    pub irq_mask: u32,
    pub running: bool,
    /// Cycles left over from the last advance, so a divider does not lose time.
    pub remainder: u32,
}

/// Bits of a channel's status register.
pub const TC_COVFS: u32 = 1 << 0;
pub const TC_CPCS: u32 = 1 << 4;
/// Bit 16 of the status is "the clock is enabled", which is not an event.
pub const TC_CLKSTA: u32 = 1 << 16;

impl Timer {
    /// How many processor cycles make one tick of this channel.
    ///
    /// The bottom three bits of the mode register choose the divider. The
    /// external clocks are not wired on this board.
    fn divider(&self) -> u32 {
        match self.mode & 7 {
            0 => 2,
            1 => 8,
            2 => 32,
            3 => 128,
            4 => 1024,
            _ => 1024,
        }
    }

    /// How often the compare fires, in hertz, given the processor's clock.
    ///
    /// Zero while the channel is stopped or has no compare set, which is what
    /// it is before firmware has configured it.
    pub fn rate(&self, clock: u32) -> f64 {
        if !self.running || self.rc == 0 {
            return 0.0;
        }
        f64::from(clock) / f64::from(self.divider()) / f64::from(self.rc)
    }

    /// Advances by `cycles` of the processor's clock, and says whether the
    /// compare fired.
    pub fn advance(&mut self, cycles: u32) -> bool {
        if !self.running {
            return false;
        }
        let step = self.divider();
        let total = self.remainder + cycles;
        let mut ticks = total / step;
        self.remainder = total % step;

        let mut fired = false;
        // Counting one tick at a time is the obvious way to write this and the
        // wrong one: at a sample rate of twenty-four kilohertz the compare is
        // hundreds of ticks away, and the only ticks that are different from
        // the rest are the one that reaches it and the one that runs off the
        // top of the counter's sixteen bits. So jump to whichever comes first.
        while ticks > 0 {
            let to_overflow = 0x1_0000 - self.counter;
            let to_compare = match self.rc {
                rc if rc != 0 && rc > self.counter => rc - self.counter,
                _ => to_overflow,
            };
            let jump = ticks.min(to_compare).min(to_overflow);
            self.counter += jump;
            ticks -= jump;
            // Wave mode with the RC compare trigger restarts the count, which
            // is how a fixed rate is made.
            if self.counter == self.rc && self.rc != 0 {
                self.status |= TC_CPCS;
                fired = true;
                if self.mode & 0x4000 != 0 {
                    self.counter = 0;
                }
            }
            if self.counter > 0xffff {
                self.counter = 0;
                self.status |= TC_COVFS;
            }
        }
        fired
    }

    /// How many of the processor's cycles may pass before this channel could
    /// possibly change its status.
    ///
    /// What lets a host skip the timers entirely between events instead of
    /// asking three of them for permission after every instruction.
    pub fn until_event(&self) -> u32 {
        if !self.running {
            return u32::MAX;
        }
        let to_overflow = 0x1_0000 - self.counter;
        let to_compare = match self.rc {
            rc if rc != 0 && rc > self.counter => rc - self.counter,
            _ => to_overflow,
        };
        to_compare
            .saturating_mul(self.divider())
            .saturating_sub(self.remainder)
    }

    /// Reading the status register clears the events in it.
    pub fn take_status(&mut self) -> u32 {
        let was = self.status | if self.running { TC_CLKSTA } else { 0 };
        self.status = 0;
        was
    }
}

/// The parallel port: thirty-two lines that can be output, input or neither.
#[derive(Debug, Default, Clone)]
pub struct Pio {
    /// Which lines the port controls rather than their alternate function.
    pub enabled: u32,
    /// Which of those are outputs.
    pub output: u32,
    /// What the outputs are driving.
    pub data: u32,
    /// What the inputs are seeing, which the board sets.
    pub input: u32,
    pub irq_mask: u32,
    pub irq_status: u32,
}

impl Pio {
    /// What a read of the pin data register answers: the outputs it is driving
    /// and the inputs as they stand.
    pub fn pins(&self) -> u32 {
        (self.data & self.output) | (self.input & !self.output)
    }
}
