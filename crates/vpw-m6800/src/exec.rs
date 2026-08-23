//! Instruction decoding and execution.
//!
//! Written from the MC6800 specification (instruction set, addressing modes,
//! effects on the CCR and cycles per instruction). See the licence note in
//! `lib.rs`.

use crate::{Bus, Ccr, Cpu, vectors};

/// Addressing modes used by the accumulator group of instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Immediate,
    Direct,
    Indexed,
    Extended,
}

impl Mode {
    /// Cycles of the 8-bit ALU operations (SUB, AND, ADD, …).
    fn alu_cycles(self) -> u32 {
        match self {
            Self::Immediate => 2,
            Self::Direct => 3,
            Self::Extended => 4,
            Self::Indexed => 5,
        }
    }

    /// Cycles of `STAA` / `STAB`.
    fn store8_cycles(self) -> u32 {
        match self {
            Self::Immediate => 0, // does not exist
            Self::Direct => 4,
            Self::Extended => 5,
            Self::Indexed => 6,
        }
    }

    /// Cycles of `LDS` / `LDX` / `CPX`.
    fn wide_load_cycles(self) -> u32 {
        match self {
            Self::Immediate => 3,
            Self::Direct => 4,
            Self::Extended => 5,
            Self::Indexed => 6,
        }
    }

    /// Cycles of `STS` / `STX`.
    fn wide_store_cycles(self) -> u32 {
        match self {
            Self::Immediate => 0, // does not exist
            Self::Direct => 5,
            Self::Extended => 6,
            Self::Indexed => 7,
        }
    }
}

impl Cpu {
    pub(crate) fn execute(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            // HCF ("halt and catch fire"). Because of incomplete decoding,
            // on the MC6800 these two opcodes leave the processor
            // incrementing the PC forever and deaf to interrupts. Only a
            // reset gets it out.
            //
            // Here we diverge on purpose from MAME's table, which assigns 6
            // cycles to 0x9D as if it were a direct `JSR`: that opcode was
            // introduced by the **6801**, replacing precisely the HCF.
            // System 11 carries a 6808, that is to say a 6800, so this is the
            // correct behaviour.
            0x9D | 0xDD => {
                self.hcf = true;
                1
            }

            // --- Inherent -----------------------------------------------------
            0x01 => 2, // NOP
            0x06 => {
                // TAP: A -> CCR
                self.cc = Ccr::from_bits(self.a);
                2
            }
            0x07 => {
                // TPA: CCR -> A
                self.a = self.cc.bits();
                2
            }
            0x08 => {
                // INX. Only affects Z.
                self.x = self.x.wrapping_add(1);
                self.cc.z = self.x == 0;
                4
            }
            0x09 => {
                // DEX. Only affects Z.
                self.x = self.x.wrapping_sub(1);
                self.cc.z = self.x == 0;
                4
            }
            0x0A => {
                self.cc.v = false;
                2
            } // CLV
            0x0B => {
                self.cc.v = true;
                2
            } // SEV
            0x0C => {
                self.cc.c = false;
                2
            } // CLC
            0x0D => {
                self.cc.c = true;
                2
            } // SEC
            0x0E => {
                self.cc.i = false;
                2
            } // CLI
            0x0F => {
                self.cc.i = true;
                2
            } // SEI

            0x10 => {
                self.a = self.sub8(self.a, self.b);
                2
            } // SBA
            0x11 => {
                self.cmp8(self.a, self.b);
                2
            } // CBA
            0x16 => {
                self.b = self.a;
                self.set_nz_v0(self.b);
                2
            } // TAB
            0x17 => {
                self.a = self.b;
                self.set_nz_v0(self.a);
                2
            } // TBA
            0x19 => {
                self.daa();
                2
            } // DAA
            0x1B => {
                self.a = self.add8(self.a, self.b, false);
                2
            } // ABA

            // --- Relative branches --------------------------------------------
            0x20..=0x2F => {
                let offset = self.fetch_u8(bus) as i8;
                if self.branch_taken(opcode) {
                    self.pc = self.pc.wrapping_add_signed(i16::from(offset));
                }
                4
            }

            // --- Stack and index ----------------------------------------------
            0x30 => {
                self.x = self.sp.wrapping_add(1);
                4
            } // TSX
            0x31 => {
                self.sp = self.sp.wrapping_add(1);
                4
            } // INS
            0x32 => {
                self.a = self.pull_u8(bus);
                4
            } // PULA
            0x33 => {
                self.b = self.pull_u8(bus);
                4
            } // PULB
            0x34 => {
                self.sp = self.sp.wrapping_sub(1);
                4
            } // DES
            0x35 => {
                self.sp = self.x.wrapping_sub(1);
                4
            } // TXS
            0x36 => {
                self.push_u8(bus, self.a);
                4
            } // PSHA
            0x37 => {
                self.push_u8(bus, self.b);
                4
            } // PSHB

            0x39 => {
                // RTS
                let hi = self.pull_u8(bus);
                let lo = self.pull_u8(bus);
                self.pc = u16::from_be_bytes([hi, lo]);
                5
            }
            0x3B => {
                self.pull_context(bus);
                10
            } // RTI
            0x3E => {
                // WAI: pushes the context and halts. The interrupt that wakes
                // it up does not push again.
                self.push_context(bus);
                self.waiting = true;
                self.context_pushed = true;
                9
            }
            0x3F => {
                // SWI
                self.push_context(bus);
                self.cc.i = true;
                self.pc = bus.read_u16(vectors::SWI);
                12
            }

            // --- Accumulator operations (inherent) ---------------------------
            0x40 => {
                self.a = self.neg(self.a);
                2
            }
            0x43 => {
                self.a = self.com(self.a);
                2
            }
            0x44 => {
                self.a = self.lsr(self.a);
                2
            }
            0x46 => {
                self.a = self.ror(self.a);
                2
            }
            0x47 => {
                self.a = self.asr(self.a);
                2
            }
            0x48 => {
                self.a = self.asl(self.a);
                2
            }
            0x49 => {
                self.a = self.rol(self.a);
                2
            }
            0x4A => {
                self.a = self.dec(self.a);
                2
            }
            0x4C => {
                self.a = self.inc(self.a);
                2
            }
            0x4D => {
                self.tst(self.a);
                2
            }
            0x4F => {
                self.a = self.clr();
                2
            }

            0x50 => {
                self.b = self.neg(self.b);
                2
            }
            0x53 => {
                self.b = self.com(self.b);
                2
            }
            0x54 => {
                self.b = self.lsr(self.b);
                2
            }
            0x56 => {
                self.b = self.ror(self.b);
                2
            }
            0x57 => {
                self.b = self.asr(self.b);
                2
            }
            0x58 => {
                self.b = self.asl(self.b);
                2
            }
            0x59 => {
                self.b = self.rol(self.b);
                2
            }
            0x5A => {
                self.b = self.dec(self.b);
                2
            }
            0x5C => {
                self.b = self.inc(self.b);
                2
            }
            0x5D => {
                self.tst(self.b);
                2
            }
            0x5F => {
                self.b = self.clr();
                2
            }

            // --- Read-modify-write on memory ---------------------------------
            0x60..=0x7F => self.memory_rmw(bus, opcode),

            // --- Accumulator group --------------------------------------------
            0x80..=0xFF => self.accumulator_op(bus, opcode),

            _ => self.illegal(opcode),
        }
    }

    /// Opcodes `0x60`–`0x7F`: the same family of operations on memory, either
    /// indexed (`0x6x`, 7 cycles) or extended (`0x7x`, 6 cycles).
    fn memory_rmw(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        let indexed = opcode < 0x70;
        let addr = if indexed {
            let offset = self.fetch_u8(bus);
            self.x.wrapping_add(u16::from(offset))
        } else {
            self.fetch_u16(bus)
        };
        let cycles = if indexed { 7 } else { 6 };

        match opcode & 0x0F {
            0x0E => {
                // JMP. The only one in the group that does not touch memory.
                self.pc = addr;
                if indexed { 4 } else { 3 }
            }
            0x0D => {
                // TST
                let value = bus.read(addr);
                self.tst(value);
                cycles
            }
            0x0F => {
                // CLR
                let value = self.clr();
                bus.write(addr, value);
                cycles
            }
            op => {
                let value = bus.read(addr);
                let result = match op {
                    0x00 => self.neg(value),
                    0x03 => self.com(value),
                    0x04 => self.lsr(value),
                    0x06 => self.ror(value),
                    0x07 => self.asr(value),
                    0x08 => self.asl(value),
                    0x09 => self.rol(value),
                    0x0A => self.dec(value),
                    0x0C => self.inc(value),
                    _ => return self.illegal(opcode),
                };
                bus.write(addr, result);
                cycles
            }
        }
    }

    /// Opcodes `0x80`–`0xFF`. The map is regular: bits 4-5 pick the addressing
    /// mode, bit 6 picks the accumulator (A or B) and the low nibble picks the
    /// operation.
    fn accumulator_op(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        let mode = match (opcode >> 4) & 0x03 {
            0 => Mode::Immediate,
            1 => Mode::Direct,
            2 => Mode::Indexed,
            _ => Mode::Extended,
        };
        let use_b = opcode & 0x40 != 0;

        match opcode & 0x0F {
            // 8-bit ALU operations, all with the same shape.
            op @ (0x00 | 0x01 | 0x02 | 0x04 | 0x05 | 0x06 | 0x08 | 0x09 | 0x0A | 0x0B) => {
                let m = self.read_operand8(bus, mode);
                let acc = if use_b { self.b } else { self.a };
                let result = match op {
                    0x00 => self.sub8(acc, m), // SUB
                    0x01 => {
                        self.cmp8(acc, m);
                        acc
                    } // CMP
                    0x02 => self.sbc8(acc, m), // SBC
                    0x04 => {
                        let r = acc & m;
                        self.set_nz_v0(r);
                        r
                    } // AND
                    0x05 => {
                        let r = acc & m;
                        self.set_nz_v0(r);
                        acc
                    } // BIT
                    0x06 => {
                        self.set_nz_v0(m);
                        m
                    } // LDA
                    0x08 => {
                        let r = acc ^ m;
                        self.set_nz_v0(r);
                        r
                    } // EOR
                    0x09 => self.add8(acc, m, self.cc.c), // ADC
                    0x0A => {
                        let r = acc | m;
                        self.set_nz_v0(r);
                        r
                    } // ORA
                    _ => self.add8(acc, m, false), // ADD
                };
                if use_b {
                    self.b = result
                } else {
                    self.a = result
                }
                mode.alu_cycles()
            }

            // STAA / STAB. Does not exist in immediate.
            0x07 => {
                if mode == Mode::Immediate {
                    return self.illegal(opcode);
                }
                let addr = self.effective_address(bus, mode);
                let value = if use_b { self.b } else { self.a };
                self.set_nz_v0(value);
                bus.write(addr, value);
                mode.store8_cycles()
            }

            // CPX, only in the accumulator A column.
            0x0C if !use_b => {
                let m = self.read_operand16(bus, mode);
                self.cpx(m);
                mode.wide_load_cycles()
            }

            // BSR (immediate) and JSR (indexed / extended).
            0x0D if !use_b => match mode {
                Mode::Immediate => {
                    let offset = self.fetch_u8(bus) as i8;
                    self.push_return_address(bus);
                    self.pc = self.pc.wrapping_add_signed(i16::from(offset));
                    8
                }
                Mode::Indexed => {
                    let addr = self.effective_address(bus, mode);
                    self.push_return_address(bus);
                    self.pc = addr;
                    8
                }
                Mode::Extended => {
                    let addr = self.effective_address(bus, mode);
                    self.push_return_address(bus);
                    self.pc = addr;
                    9
                }
                // 0x9D does not exist on the 6800: there is no direct JSR.
                Mode::Direct => self.illegal(opcode),
            },

            // LDS (column A) / LDX (column B).
            0x0E => {
                let value = self.read_operand16(bus, mode);
                self.cc.set_nz16(value);
                self.cc.v = false;
                if use_b {
                    self.x = value
                } else {
                    self.sp = value
                }
                mode.wide_load_cycles()
            }

            // STS (column A) / STX (column B). Does not exist in immediate.
            0x0F => {
                if mode == Mode::Immediate {
                    return self.illegal(opcode);
                }
                let addr = self.effective_address(bus, mode);
                let value = if use_b { self.x } else { self.sp };
                self.cc.set_nz16(value);
                self.cc.v = false;
                let [hi, lo] = value.to_be_bytes();
                bus.write(addr, hi);
                bus.write(addr.wrapping_add(1), lo);
                mode.wide_store_cycles()
            }

            _ => self.illegal(opcode),
        }
    }

    // --- Addressing -------------------------------------------------------------

    /// Effective address. Does not apply to immediate, which does not have one.
    fn effective_address(&mut self, bus: &mut impl Bus, mode: Mode) -> u16 {
        match mode {
            Mode::Direct => u16::from(self.fetch_u8(bus)),
            Mode::Indexed => {
                // The indexed offset is 8 bits **unsigned**.
                let offset = self.fetch_u8(bus);
                self.x.wrapping_add(u16::from(offset))
            }
            Mode::Extended => self.fetch_u16(bus),
            Mode::Immediate => unreachable!("immediate mode has no effective address"),
        }
    }

    fn read_operand8(&mut self, bus: &mut impl Bus, mode: Mode) -> u8 {
        match mode {
            Mode::Immediate => self.fetch_u8(bus),
            _ => {
                let addr = self.effective_address(bus, mode);
                bus.read(addr)
            }
        }
    }

    fn read_operand16(&mut self, bus: &mut impl Bus, mode: Mode) -> u16 {
        match mode {
            Mode::Immediate => self.fetch_u16(bus),
            _ => {
                let addr = self.effective_address(bus, mode);
                bus.read_u16(addr)
            }
        }
    }

    /// Pushes the current PC (the return address) in the order PCL, PCH.
    fn push_return_address(&mut self, bus: &mut impl Bus) {
        let [hi, lo] = self.pc.to_be_bytes();
        self.push_u8(bus, lo);
        self.push_u8(bus, hi);
    }

    /// Condition of the `0x20`–`0x2F` branches.
    fn branch_taken(&self, opcode: u8) -> bool {
        let cc = self.cc;
        match opcode {
            0x20 => true,                  // BRA
            0x22 => !cc.c && !cc.z,        // BHI
            0x23 => cc.c || cc.z,          // BLS
            0x24 => !cc.c,                 // BCC
            0x25 => cc.c,                  // BCS
            0x26 => !cc.z,                 // BNE
            0x27 => cc.z,                  // BEQ
            0x28 => !cc.v,                 // BVC
            0x29 => cc.v,                  // BVS
            0x2A => !cc.n,                 // BPL
            0x2B => cc.n,                  // BMI
            0x2C => cc.n == cc.v,          // BGE
            0x2D => cc.n != cc.v,          // BLT
            0x2E => !cc.z && cc.n == cc.v, // BGT
            0x2F => cc.z || cc.n != cc.v,  // BLE
            // 0x21 does not exist (it would be BRN, which the 6800 lacks).
            _ => false,
        }
    }

    /// Opcode not defined on the 6800. We record it instead of swallowing it:
    /// if a ROM gets here, it is a bug of ours or the ROM went off the rails.
    fn illegal(&mut self, opcode: u8) -> u32 {
        self.last_illegal = Some((self.pc.wrapping_sub(1), opcode));
        2
    }

    // --- ALU ---------------------------------------------------------------------

    /// N and Z from the result, V cleared. The shape of the logic operations
    /// and of the loads.
    #[inline]
    fn set_nz_v0(&mut self, value: u8) {
        self.cc.set_nz8(value);
        self.cc.v = false;
    }

    fn add8(&mut self, lhs: u8, rhs: u8, carry_in: bool) -> u8 {
        let carry = u16::from(carry_in);
        let sum = u16::from(lhs) + u16::from(rhs) + carry;
        let result = sum as u8;
        self.cc.h = (lhs & 0x0F) + (rhs & 0x0F) + carry as u8 > 0x0F;
        self.cc.set_nz8(result);
        // Signed overflow: the operands share a sign and the result does not.
        self.cc.v = (lhs ^ result) & (rhs ^ result) & 0x80 != 0;
        self.cc.c = sum > 0xFF;
        result
    }

    fn sub8(&mut self, lhs: u8, rhs: u8) -> u8 {
        let result = lhs.wrapping_sub(rhs);
        self.cc.set_nz8(result);
        // Signed overflow: the operands differ in sign and the result differs
        // from the minuend.
        self.cc.v = (lhs ^ rhs) & (lhs ^ result) & 0x80 != 0;
        self.cc.c = lhs < rhs;
        result
    }

    fn sbc8(&mut self, lhs: u8, rhs: u8) -> u8 {
        let borrow = u16::from(self.cc.c);
        let result = lhs.wrapping_sub(rhs).wrapping_sub(borrow as u8);
        self.cc.set_nz8(result);
        self.cc.v = (lhs ^ rhs) & (lhs ^ result) & 0x80 != 0;
        self.cc.c = u16::from(lhs) < u16::from(rhs) + borrow;
        result
    }

    fn cmp8(&mut self, lhs: u8, rhs: u8) {
        self.sub8(lhs, rhs);
    }

    /// `CPX`. Two oddities, and the reference carries both on purpose.
    ///
    /// The known one: it does **not** touch the carry. Only the 6801 fixed
    /// that, and there are ROMs from the period that depend on the oddity.
    ///
    /// The other one is easier to miss, because the instruction looks like a
    /// sixteen-bit compare and only half of it is. **Zero** comes from the full
    /// sixteen-bit difference, but **negative and overflow come from the high
    /// bytes alone** (`6800ops.c:1091`):
    ///
    /// ```c
    /// r.w.l = d.b.h - b.b.h;      // the high bytes, in 16 bits so bit 8 keeps the borrow
    /// CLR_NZV;
    /// SET_N8(r.b.l);              // N: bit 7 of that byte
    /// SET_V8(d.b.h, b.b.h, r.w.l);
    /// r.d = d.d - b.d;
    /// SET_Z16(r.d);               // Z: the whole thing
    /// ```
    ///
    /// The 6803 does have a proper sixteen-bit compare, and it is a **different
    /// opcode** (`cpx_im`, `6800ops.c:1102`). A System 11 runs 6808s, and
    /// `#define m6808 m6800` (`m6800.c:163`) puts them on this table.
    ///
    /// It is not a rounding error either: over four hundred thousand random
    /// pairs, N comes out different in 0.4% of them and V in 0.2%. `CPX #$8000`
    /// against `$0001` is one — the sixteen-bit answer is positive and the
    /// high-byte one is negative — and a `BMI` after it goes the other way.
    fn cpx(&mut self, operand: u16) {
        let x = (self.x >> 8) as u8;
        let m = (operand >> 8) as u8;
        // Sixteen bits wide so bit 8 holds the borrow out of the high byte,
        // which is what the overflow term below reads through `r >> 1`.
        let r = u16::from(x).wrapping_sub(u16::from(m));

        self.cc.n = r as u8 & 0x80 != 0;
        self.cc.v = (u16::from(x) ^ u16::from(m) ^ r ^ (r >> 1)) & 0x80 != 0;
        self.cc.z = self.x == operand;
    }

    fn neg(&mut self, value: u8) -> u8 {
        let result = 0u8.wrapping_sub(value);
        self.cc.set_nz8(result);
        // 0x80 is its own negative in two's complement.
        self.cc.v = result == 0x80;
        self.cc.c = result != 0x00;
        result
    }

    fn com(&mut self, value: u8) -> u8 {
        let result = !value;
        self.cc.set_nz8(result);
        self.cc.v = false;
        // COM always leaves the carry at 1.
        self.cc.c = true;
        result
    }

    fn inc(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.cc.set_nz8(result);
        self.cc.v = value == 0x7F;
        // INC does not touch the carry.
        result
    }

    fn dec(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.cc.set_nz8(result);
        self.cc.v = value == 0x80;
        // DEC does not touch the carry.
        result
    }

    fn tst(&mut self, value: u8) {
        self.cc.set_nz8(value);
        self.cc.v = false;
        self.cc.c = false;
    }

    fn clr(&mut self) -> u8 {
        self.cc.n = false;
        self.cc.z = true;
        self.cc.v = false;
        self.cc.c = false;
        0
    }

    fn asl(&mut self, value: u8) -> u8 {
        let result = value << 1;
        self.cc.c = value & 0x80 != 0;
        self.cc.set_nz8(result);
        self.cc.v = self.cc.n != self.cc.c;
        result
    }

    fn asr(&mut self, value: u8) -> u8 {
        // Arithmetic: it preserves the sign bit.
        let result = (value >> 1) | (value & 0x80);
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result);
        self.cc.v = self.cc.n != self.cc.c;
        result
    }

    fn lsr(&mut self, value: u8) -> u8 {
        let result = value >> 1;
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result); // N always ends up at 0
        self.cc.v = self.cc.n != self.cc.c;
        result
    }

    fn rol(&mut self, value: u8) -> u8 {
        let result = (value << 1) | u8::from(self.cc.c);
        self.cc.c = value & 0x80 != 0;
        self.cc.set_nz8(result);
        self.cc.v = self.cc.n != self.cc.c;
        result
    }

    fn ror(&mut self, value: u8) -> u8 {
        let result = (value >> 1) | (u8::from(self.cc.c) << 7);
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result);
        self.cc.v = self.cc.n != self.cc.c;
        result
    }

    /// `DAA`: readjusts A after a BCD addition.
    fn daa(&mut self) {
        let high = self.a >> 4;
        let low = self.a & 0x0F;
        let mut correction = 0u8;
        let mut carry = self.cc.c;

        if low > 9 || self.cc.h {
            correction |= 0x06;
        }
        // The high nibble also has to be corrected if the low one is going to
        // carry into it, hence the third condition.
        if high > 9 || self.cc.c || (high >= 9 && low > 9) {
            correction |= 0x60;
            carry = true;
        }

        self.a = self.a.wrapping_add(correction);
        self.cc.set_nz8(self.a);
        self.cc.c = carry;
    }
}
