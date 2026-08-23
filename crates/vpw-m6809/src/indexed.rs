//! Indexed addressing: the postbyte.
//!
//! It is the most distinctive thing about the 6809. The opcode is followed by
//! a byte that describes how to compute the address, with sixteen different
//! forms and an indirect variant of almost all of them.
//!
//! ```text
//!   7   6   5   4   3   2   1   0
//! +---+---+---+---+---+---+---+---+
//! | 0 |  RR   |      5-bit offset |   short signed offset
//! +---+---+---+---+---+---+---+---+
//! | 1 |  RR   | I |    mode       |   all the other forms
//! +---+---+---+---+---+---+---+---+
//! ```
//!
//! `RR` picks the base register (`X`, `Y`, `U` or `S`) and `I` asks for an
//! **indirection**: the computed address is not the operand but where the
//! address of the operand is stored.

use crate::{Bus, Cpu};

/// The four registers that can act as a base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexRegister {
    X,
    Y,
    U,
    S,
}

impl IndexRegister {
    fn from_postbyte(postbyte: u8) -> Self {
        match (postbyte >> 5) & 0x03 {
            0 => Self::X,
            1 => Self::Y,
            2 => Self::U,
            _ => Self::S,
        }
    }
}

impl Cpu {
    fn index_register(&self, which: IndexRegister) -> u16 {
        match which {
            IndexRegister::X => self.x,
            IndexRegister::Y => self.y,
            IndexRegister::U => self.u,
            IndexRegister::S => self.s,
        }
    }

    fn set_index_register(&mut self, which: IndexRegister, value: u16) {
        match which {
            IndexRegister::X => self.x = value,
            IndexRegister::Y => self.y = value,
            IndexRegister::U => self.u = value,
            IndexRegister::S => self.s = value,
        }
    }

    /// Reads the postbyte and returns `(effective address, extra cycles)`.
    ///
    /// The extra cycles are added to the base cost of the instruction; each
    /// indexing form costs what it costs.
    pub(crate) fn decode_indexed(&mut self, bus: &mut impl Bus) -> (u16, u32) {
        let postbyte = self.fetch_u8(bus);
        let base = IndexRegister::from_postbyte(postbyte);

        // Bit 7 clear: short signed 5-bit offset, no indirection.
        if postbyte & 0x80 == 0 {
            let offset = sign_extend_5(postbyte & 0x1F);
            let address = self.index_register(base).wrapping_add_signed(offset);
            return (address, 1);
        }

        let indirect = postbyte & 0x10 != 0;
        let (address, cycles) = match postbyte & 0x0F {
            // ,R+ and ,R++ : the register is used and incremented afterwards.
            0x00 => {
                let address = self.index_register(base);
                self.set_index_register(base, address.wrapping_add(1));
                (address, 2)
            }
            0x01 => {
                let address = self.index_register(base);
                self.set_index_register(base, address.wrapping_add(2));
                (address, 3)
            }
            // ,-R and ,--R : first it is decremented and then used.
            0x02 => {
                let address = self.index_register(base).wrapping_sub(1);
                self.set_index_register(base, address);
                (address, 2)
            }
            0x03 => {
                let address = self.index_register(base).wrapping_sub(2);
                self.set_index_register(base, address);
                (address, 3)
            }
            // ,R with no offset.
            0x04 => (self.index_register(base), 0),
            // Offset taken from an accumulator, signed.
            0x05 => {
                let offset = i16::from(self.b as i8);
                (self.index_register(base).wrapping_add_signed(offset), 1)
            }
            0x06 => {
                let offset = i16::from(self.a as i8);
                (self.index_register(base).wrapping_add_signed(offset), 1)
            }
            // Constant 8- and 16-bit offsets.
            0x08 => {
                let offset = i16::from(self.fetch_u8(bus) as i8);
                (self.index_register(base).wrapping_add_signed(offset), 1)
            }
            0x09 => {
                let offset = self.fetch_u16(bus) as i16;
                (self.index_register(base).wrapping_add_signed(offset), 4)
            }
            // 16-bit offset taken from D.
            0x0B => {
                let offset = self.d() as i16;
                (self.index_register(base).wrapping_add_signed(offset), 4)
            }
            // PC relative. The offset counts from the PC **after** reading it,
            // not from the postbyte.
            0x0C => {
                let offset = i16::from(self.fetch_u8(bus) as i8);
                (self.pc.wrapping_add_signed(offset), 1)
            }
            0x0D => {
                let offset = self.fetch_u16(bus) as i16;
                (self.pc.wrapping_add_signed(offset), 5)
            }
            // Extended indirect: the address comes whole in the code. It only
            // exists with the indirection bit set.
            0x0F if indirect => (self.fetch_u16(bus), 2),
            _ => {
                self.last_illegal = Some((self.pc.wrapping_sub(1), postbyte));
                (0, 0)
            }
        };

        if indirect {
            // The indirection costs three more cycles and a 16-bit read.
            (bus.read_u16(address), cycles + 3)
        } else {
            (address, cycles)
        }
    }
}

/// Sign-extends a 5-bit offset.
fn sign_extend_5(value: u8) -> i16 {
    if value & 0x10 != 0 {
        i16::from(value) - 32
    } else {
        i16::from(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_5_bit_offset_is_signed() {
        assert_eq!(sign_extend_5(0x00), 0);
        assert_eq!(sign_extend_5(0x0F), 15, "the largest positive");
        assert_eq!(sign_extend_5(0x10), -16, "the smallest negative");
        assert_eq!(sign_extend_5(0x1F), -1);
    }

    #[test]
    fn the_postbyte_picks_the_base_register() {
        assert_eq!(IndexRegister::from_postbyte(0b0000_0000), IndexRegister::X);
        assert_eq!(IndexRegister::from_postbyte(0b0010_0000), IndexRegister::Y);
        assert_eq!(IndexRegister::from_postbyte(0b0100_0000), IndexRegister::U);
        assert_eq!(IndexRegister::from_postbyte(0b0110_0000), IndexRegister::S);
    }
}
