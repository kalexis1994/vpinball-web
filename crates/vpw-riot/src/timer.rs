//! The interval timer both RIOTs carry, which is the same circuit on each.
//!
//! A byte and a prescaler. Writing loads the byte and picks how many clocks
//! each count lasts; it counts down; when it passes zero the flag sets **and
//! the prescaler goes to one**, so from then on it runs down from `$FF` every
//! clock. That last part is not a detail: it is what lets a program read the
//! timer *after* the timeout and know how long ago the timeout was, and a
//! model that stops at zero looks right until a ROM uses it that way.

/// One interval timer.
#[derive(Debug, Clone)]
pub struct Timer {
    /// The count, as the CPU would read it.
    count: u8,
    /// Clocks per decrement: 1, 8, 64 or 1024 until it times out, one after.
    divider: u32,
    /// Clocks left before the next decrement.
    remaining: u32,
    flag: bool,
    enabled: bool,
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Timer {
    pub fn new() -> Self {
        Self {
            count: 0,
            divider: 1,
            remaining: 1,
            flag: false,
            enabled: false,
        }
    }

    /// Loads the timer. `code` is the two address lines that pick the rate.
    pub fn start(&mut self, value: u8, code: u8, interrupt: bool) {
        self.count = value;
        self.divider = match code & 0x03 {
            0 => 1,
            1 => 8,
            2 => 64,
            _ => 1024,
        };
        self.remaining = self.divider;
        self.enabled = interrupt;
        self.flag = false;
    }

    /// Reads the count, which **clears the flag**, and says whether the timer
    /// may interrupt from now on — on both chips that is chosen by which of
    /// two addresses the read came from rather than by the data.
    pub fn read(&mut self, interrupt: bool) -> u8 {
        self.enabled = interrupt;
        self.flag = false;
        self.count
    }

    /// The flag on its own, for a chip whose flag register reports it without
    /// clearing it.
    pub fn flag(&self) -> bool {
        self.flag
    }

    /// Whether the timer is asking for an interrupt.
    pub fn interrupting(&self) -> bool {
        self.flag && self.enabled
    }

    /// Runs for a number of CPU clocks.
    pub fn tick(&mut self, clocks: u32) {
        for _ in 0..clocks {
            self.remaining -= 1;
            if self.remaining > 0 {
                continue;
            }
            if self.count == 0 {
                self.count = 0xFF;
                self.flag = true;
                self.divider = 1;
            } else {
                self.count -= 1;
            }
            self.remaining = self.divider;
        }
    }
}
