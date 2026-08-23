//! The 6809 condition code register.
//!
//! ```text
//!   7   6   5   4   3   2   1   0
//! +---+---+---+---+---+---+---+---+
//! | E | F | H | I | N | Z | V | C |
//! +---+---+---+---+---+---+---+---+
//! ```
//!
//! Unlike the 6800, here all eight bits are real: none of them is wired to
//! one. The top two are new: `E` says whether the last interrupt stacked the
//! **entire** state (`RTI` needs it to know how many bytes to pull) and `F`
//! masks the FIRQ, the fast interrupt the 6800 did not have.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ccr {
    /// Entire: the interrupt stacked the twelve bytes of the complete state,
    /// not the three of a FIRQ.
    pub e: bool,
    /// FIRQ mask.
    pub f: bool,
    /// Half carry: carry from bit 3 to bit 4. Only 8-bit additions touch it.
    pub h: bool,
    /// IRQ mask.
    pub i: bool,
    /// Negative.
    pub n: bool,
    /// Zero.
    pub z: bool,
    /// Signed overflow.
    pub v: bool,
    /// Carry / borrow.
    pub c: bool,
}

impl Ccr {
    pub fn bits(self) -> u8 {
        (u8::from(self.e) << 7)
            | (u8::from(self.f) << 6)
            | (u8::from(self.h) << 5)
            | (u8::from(self.i) << 4)
            | (u8::from(self.n) << 3)
            | (u8::from(self.z) << 2)
            | (u8::from(self.v) << 1)
            | u8::from(self.c)
    }

    pub fn from_bits(bits: u8) -> Self {
        Self {
            e: bits & 0x80 != 0,
            f: bits & 0x40 != 0,
            h: bits & 0x20 != 0,
            i: bits & 0x10 != 0,
            n: bits & 0x08 != 0,
            z: bits & 0x04 != 0,
            v: bits & 0x02 != 0,
            c: bits & 0x01 != 0,
        }
    }

    #[inline]
    pub fn set_nz8(&mut self, value: u8) {
        self.n = value & 0x80 != 0;
        self.z = value == 0;
    }

    #[inline]
    pub fn set_nz16(&mut self, value: u16) {
        self.n = value & 0x8000 != 0;
        self.z = value == 0;
    }

    /// `N`, `Z` and `V` cleared: the shape of the logic operations and the
    /// loads.
    #[inline]
    pub fn set_nz8_v0(&mut self, value: u8) {
        self.set_nz8(value);
        self.v = false;
    }

    #[inline]
    pub fn set_nz16_v0(&mut self, value: u16) {
        self.set_nz16(value);
        self.v = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_bits_are_real() {
        // The 6800 forced the top two to one; the 6809 does not.
        assert_eq!(Ccr::default().bits(), 0x00);
        for raw in 0u8..=0xFF {
            assert_eq!(Ccr::from_bits(raw).bits(), raw, "raw = {raw:#04x}");
        }
    }
}
