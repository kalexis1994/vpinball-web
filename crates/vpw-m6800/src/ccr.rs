//! Condition code register (CCR).
//!
//! MC6800 layout, from bit 7 down to bit 0:
//!
//! ```text
//!   7   6   5   4   3   2   1   0
//! +---+---+---+---+---+---+---+---+
//! | 1 | 1 | H | I | N | Z | V | C |
//! +---+---+---+---+---+---+---+---+
//! ```
//!
//! The two high bits are hardwired to 1: reading them always gives 1 and
//! writing them does nothing. It matters because `TPA` and the CCR pushed
//! during an interrupt expose them, and ROM code sometimes depends on that
//! value.

/// Bits of the CCR that are fixed at 1.
pub const CCR_ALWAYS_SET: u8 = 0b1100_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ccr {
    /// Half carry: carry from bit 3 into bit 4. Only additions touch it.
    pub h: bool,
    /// Interrupt mask: with this at 1 the IRQ stays masked.
    pub i: bool,
    /// Negative: most significant bit of the result.
    pub n: bool,
    /// Zero.
    pub z: bool,
    /// Signed overflow (two's complement).
    pub v: bool,
    /// Carry / borrow.
    pub c: bool,
}

impl Ccr {
    pub fn bits(self) -> u8 {
        CCR_ALWAYS_SET
            | (u8::from(self.h) << 5)
            | (u8::from(self.i) << 4)
            | (u8::from(self.n) << 3)
            | (u8::from(self.z) << 2)
            | (u8::from(self.v) << 1)
            | u8::from(self.c)
    }

    pub fn from_bits(bits: u8) -> Self {
        Self {
            h: bits & 0b0010_0000 != 0,
            i: bits & 0b0001_0000 != 0,
            n: bits & 0b0000_1000 != 0,
            z: bits & 0b0000_0100 != 0,
            v: bits & 0b0000_0010 != 0,
            c: bits & 0b0000_0001 != 0,
        }
    }

    /// Updates N and Z from an 8-bit result.
    #[inline]
    pub fn set_nz8(&mut self, value: u8) {
        self.n = value & 0x80 != 0;
        self.z = value == 0;
    }

    /// Updates N and Z from a 16-bit result.
    #[inline]
    pub fn set_nz16(&mut self, value: u16) {
        self.n = value & 0x8000 != 0;
        self.z = value == 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_high_bits_always_read_one() {
        assert_eq!(Ccr::default().bits(), 0b1100_0000);
    }

    #[test]
    fn round_trip_through_bits() {
        for raw in 0u8..=0xFF {
            // Bits 6 and 7 cannot be cleared, so the round trip puts them back.
            let expected = raw | CCR_ALWAYS_SET;
            assert_eq!(Ccr::from_bits(raw).bits(), expected, "raw = {raw:#04x}");
        }
    }
}
