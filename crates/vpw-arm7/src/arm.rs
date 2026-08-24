//! ARM state: the thirty-two bit instruction set.
//!
//! One instruction is one word and the top four bits are the condition, so
//! every instruction is conditional and most of the work is deciding which of
//! nine classes a word belongs to. The decode below is in the order the
//! architecture manual lists them, which is also the order that gets the
//! ambiguous encodings right: the multiplies and the halfword transfers hide
//! *inside* the data-processing space and have to be picked out first.

use super::{Bus, C, Cpu, Mode, N, T, V, Z, vectors};

impl Cpu {
    pub(super) fn step_arm(&mut self, bus: &mut impl Bus) -> u32 {
        // `r[15]` holds the address of the instruction being executed, so
        // that `reg(15)` can answer the pipeline value the architecture
        // promises — that address plus eight. It moves on at the end, and only
        // if nothing wrote it.
        let pc = self.r[15];
        let insn = bus.read32(pc);
        self.branched = false;

        let cond = insn >> 28;
        if !self.passes(cond) {
            self.r[15] = pc.wrapping_add(4);
            return 1;
        }
        let cycles = self.execute(insn, bus);
        if !self.branched {
            self.r[15] = pc.wrapping_add(4);
        }
        cycles
    }

    fn execute(&mut self, insn: u32, bus: &mut impl Bus) -> u32 {
        // The order matters. `MUL` and the halfword transfers live inside the
        // data-processing encoding and are told apart by bits 7 and 4 being
        // set, so they have to be tested before the class they hide in.
        if insn & 0x0fff_fff0 == 0x012f_ff10 {
            return self.branch_exchange(insn);
        }
        if insn & 0x0fc0_00f0 == 0x0000_0090 {
            return self.multiply(insn);
        }
        if insn & 0x0f80_00f0 == 0x0080_0090 {
            return self.multiply_long(insn);
        }
        if insn & 0x0fb0_0ff0 == 0x0100_0090 {
            return self.swap(insn, bus);
        }
        if insn & 0x0e00_0090 == 0x0000_0090 {
            return self.halfword(insn, bus);
        }
        match (insn >> 25) & 7 {
            0 | 1 => {
                // `MRS`/`MSR` are the data-processing opcodes 8 to 11 with the
                // S bit clear, which is otherwise a comparison that discards
                // its result and would be pointless.
                let op = (insn >> 21) & 0xf;
                if (8..=11).contains(&op) && insn & (1 << 20) == 0 {
                    self.psr_transfer(insn)
                } else {
                    self.data_processing(insn)
                }
            }
            2 | 3 => self.single_transfer(insn, bus),
            4 => self.block_transfer(insn, bus),
            5 => self.branch(insn),
            // The coprocessor space. An ARM7TDMI in this board has none, and
            // the architecture says an absent coprocessor takes the undefined
            // instruction trap — which is what the ROM's own handler expects.
            6 | 7 if insn & 0x0f00_0000 == 0x0f00_0000 => self.software_interrupt(insn),
            _ => self.undefined(),
        }
    }

    fn undefined(&mut self) -> u32 {
        let ret = self.r[15].wrapping_add(4);
        self.enter(vectors::UNDEFINED, Mode::Undefined, ret, false);
        3
    }

    fn software_interrupt(&mut self, insn: u32) -> u32 {
        self.swi = Some(insn & 0x00ff_ffff);
        let ret = self.r[15].wrapping_add(4);
        self.enter(vectors::SWI, Mode::Supervisor, ret, false);
        3
    }

    fn branch(&mut self, insn: u32) -> u32 {
        // Twenty-four bits, signed, in words. The `+8` is the pipeline, which
        // `reg(15)` already answers with.
        let offset = ((insn & 0x00ff_ffff) << 8) as i32 >> 6;
        if insn & (1 << 24) != 0 {
            self.r[14] = self.r[15].wrapping_add(4);
        }
        let target = self.reg(15).wrapping_add(offset as u32);
        self.set_reg(15, target);
        3
    }

    fn branch_exchange(&mut self, insn: u32) -> u32 {
        let target = self.reg((insn & 0xf) as usize);
        // The low bit is not part of the address: it is the state to go to.
        self.set_flag(T, target & 1 != 0);
        self.set_reg(15, target & !1);
        3
    }

    /// The barrel shifter, which every data-processing instruction runs its
    /// second operand through.
    ///
    /// Returns the value and what the carry out was, because the shifter's
    /// carry is what `ANDS` and friends put in the C flag — a detail that is
    /// invisible until a compiler uses `MOVS r0, r1, LSR #32` to get a bit
    /// into carry, which they do.
    fn shift(&self, value: u32, kind: u32, amount: u32, by_register: bool) -> (u32, bool) {
        let carry_in = self.cpsr & C != 0;
        match (kind, amount) {
            // A register shift of zero leaves everything alone, including the
            // carry. An immediate zero means something else for three of the
            // four kinds, below.
            (_, 0) if by_register => (value, carry_in),
            (0, 0) => (value, carry_in),
            (0, s) if s < 32 => (value << s, value & (1 << (32 - s)) != 0),
            (0, 32) => (0, value & 1 != 0),
            (0, _) => (0, false),
            // An immediate LSR of zero encodes 32.
            (1, 0) => (0, value & 0x8000_0000 != 0),
            (1, s) if s < 32 => (value >> s, value & (1 << (s - 1)) != 0),
            (1, 32) => (0, value & 0x8000_0000 != 0),
            (1, _) => (0, false),
            // And an immediate ASR of zero also encodes 32.
            (2, 0) | (2, 32) => {
                let sign = value & 0x8000_0000 != 0;
                (if sign { !0 } else { 0 }, sign)
            }
            (2, s) if s < 32 => (((value as i32) >> s) as u32, value & (1 << (s - 1)) != 0),
            (2, _) => {
                let sign = value & 0x8000_0000 != 0;
                (if sign { !0 } else { 0 }, sign)
            }
            // An immediate ROR of zero is RRX: one place right through carry.
            (3, 0) => ((value >> 1) | (u32::from(carry_in) << 31), value & 1 != 0),
            (3, s) => {
                let s = s & 31;
                if s == 0 {
                    (value, value & 0x8000_0000 != 0)
                } else {
                    (value.rotate_right(s), value & (1 << (s - 1)) != 0)
                }
            }
            _ => (value, carry_in),
        }
    }

    /// The second operand, and the shifter's carry out.
    fn operand2(&self, insn: u32) -> (u32, bool) {
        if insn & (1 << 25) != 0 {
            // An immediate: eight bits rotated right by twice a four-bit field.
            let rot = ((insn >> 8) & 0xf) * 2;
            let value = (insn & 0xff).rotate_right(rot);
            let carry = if rot == 0 {
                self.cpsr & C != 0
            } else {
                value & 0x8000_0000 != 0
            };
            (value, carry)
        } else {
            let rm = (insn & 0xf) as usize;
            let kind = (insn >> 5) & 3;
            if insn & (1 << 4) != 0 {
                // Shifted by a register, which reads R15 as twelve on rather
                // than eight because of the extra cycle.
                let rs = ((insn >> 8) & 0xf) as usize;
                let amount = self.reg(rs) & 0xff;
                let value = if rm == 15 {
                    self.reg(15).wrapping_add(4)
                } else {
                    self.reg(rm)
                };
                self.shift(value, kind, amount, true)
            } else {
                let amount = (insn >> 7) & 0x1f;
                self.shift(self.reg(rm), kind, amount, false)
            }
        }
    }

    fn data_processing(&mut self, insn: u32) -> u32 {
        let op = (insn >> 21) & 0xf;
        let set = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xf) as usize;
        let rd = ((insn >> 12) & 0xf) as usize;

        let (op2, shift_carry) = self.operand2(insn);
        // R15 reads as twelve on when the shift amount came from a register.
        let a = if rn == 15 && insn & 0x0201_0000 == 0x0001_0000 && insn & (1 << 4) != 0 {
            self.reg(15).wrapping_add(4)
        } else {
            self.reg(rn)
        };

        let carry_in = u32::from(self.cpsr & C != 0);
        let (result, carry, overflow, writes) = match op {
            0x0 => (a & op2, shift_carry, None, true),  // AND
            0x1 => (a ^ op2, shift_carry, None, true),  // EOR
            0x2 => sub(a, op2, 1),                      // SUB
            0x3 => sub(op2, a, 1),                      // RSB
            0x4 => add(a, op2, 0),                      // ADD
            0x5 => add(a, op2, carry_in),               // ADC
            0x6 => sub(a, op2, carry_in),               // SBC
            0x7 => sub(op2, a, carry_in),               // RSC
            0x8 => (a & op2, shift_carry, None, false), // TST
            0x9 => (a ^ op2, shift_carry, None, false), // TEQ
            0xa => {
                let (r, c, v, _) = sub(a, op2, 1);
                (r, c, v, false)
            } // CMP
            0xb => {
                let (r, c, v, _) = add(a, op2, 0);
                (r, c, v, false)
            } // CMN
            0xc => (a | op2, shift_carry, None, true),  // ORR
            0xd => (op2, shift_carry, None, true),      // MOV
            0xe => (a & !op2, shift_carry, None, true), // BIC
            _ => (!op2, shift_carry, None, true),       // MVN
        };

        if writes {
            self.set_reg(rd, result);
        }
        if set {
            if rd == 15 && writes {
                // `MOVS pc, lr` and the rest: this is how an exception returns,
                // and the restore of the saved status is the whole point of it.
                let spsr = self.spsr();
                let mode = Mode::from_bits(spsr);
                self.set_mode(mode);
                self.cpsr = spsr;
            } else {
                self.set_nz(result);
                self.set_flag(C, carry);
                if let Some(v) = overflow {
                    self.set_flag(V, v);
                }
            }
        }
        if rd == 15 && writes { 3 } else { 1 }
    }

    fn psr_transfer(&mut self, insn: u32) -> u32 {
        let spsr = insn & (1 << 22) != 0;
        if insn & (1 << 21) == 0 {
            // MRS
            let rd = ((insn >> 12) & 0xf) as usize;
            let v = if spsr { self.spsr() } else { self.cpsr };
            self.set_reg(rd, v);
            return 1;
        }

        // MSR. The field mask says which bytes may be written, and in user
        // mode only the flags may be, whatever the mask says.
        let value = if insn & (1 << 25) != 0 {
            let rot = ((insn >> 8) & 0xf) * 2;
            (insn & 0xff).rotate_right(rot)
        } else {
            self.reg((insn & 0xf) as usize)
        };

        let mut mask = 0u32;
        if insn & (1 << 16) != 0 {
            mask |= 0x0000_00ff;
        }
        if insn & (1 << 17) != 0 {
            mask |= 0x0000_ff00;
        }
        if insn & (1 << 18) != 0 {
            mask |= 0x00ff_0000;
        }
        if insn & (1 << 19) != 0 {
            mask |= 0xff00_0000;
        }
        if self.mode() == Mode::User {
            mask &= 0xff00_0000;
        }

        if spsr {
            let now = self.spsr();
            self.set_spsr((now & !mask) | (value & mask));
        } else {
            let now = self.cpsr;
            let next = (now & !mask) | (value & mask);
            // Changing the mode field has to move the banked registers with
            // it, which is the whole reason `set_mode` exists.
            if next & 0x1f != now & 0x1f {
                self.set_mode(Mode::from_bits(next));
            }
            self.cpsr = (self.cpsr & 0x1f) | (next & !0x1f);
        }
        1
    }

    fn multiply(&mut self, insn: u32) -> u32 {
        let rd = ((insn >> 16) & 0xf) as usize;
        let rn = ((insn >> 12) & 0xf) as usize;
        let rs = ((insn >> 8) & 0xf) as usize;
        let rm = (insn & 0xf) as usize;
        let mut result = self.reg(rm).wrapping_mul(self.reg(rs));
        if insn & (1 << 21) != 0 {
            result = result.wrapping_add(self.reg(rn));
        }
        self.set_reg(rd, result);
        if insn & (1 << 20) != 0 {
            self.set_nz(result);
            // The architecture leaves C unpredictable here and says V is
            // untouched. Leaving both alone is what the part does.
        }
        4
    }

    fn multiply_long(&mut self, insn: u32) -> u32 {
        let hi = ((insn >> 16) & 0xf) as usize;
        let lo = ((insn >> 12) & 0xf) as usize;
        let rs = ((insn >> 8) & 0xf) as usize;
        let rm = (insn & 0xf) as usize;
        let signed = insn & (1 << 22) != 0;

        let mut result = if signed {
            ((self.reg(rm) as i32 as i64).wrapping_mul(self.reg(rs) as i32 as i64)) as u64
        } else {
            u64::from(self.reg(rm)).wrapping_mul(u64::from(self.reg(rs)))
        };
        if insn & (1 << 21) != 0 {
            let acc = (u64::from(self.reg(hi)) << 32) | u64::from(self.reg(lo));
            result = result.wrapping_add(acc);
        }
        self.set_reg(lo, result as u32);
        self.set_reg(hi, (result >> 32) as u32);
        if insn & (1 << 20) != 0 {
            self.set_flag(N, result & (1 << 63) != 0);
            self.set_flag(Z, result == 0);
        }
        5
    }

    fn swap(&mut self, insn: u32, bus: &mut impl Bus) -> u32 {
        let rn = ((insn >> 16) & 0xf) as usize;
        let rd = ((insn >> 12) & 0xf) as usize;
        let rm = (insn & 0xf) as usize;
        let addr = self.reg(rn);
        if insn & (1 << 22) != 0 {
            let was = bus.read8(addr);
            bus.write8(addr, self.reg(rm) as u8);
            self.set_reg(rd, u32::from(was));
        } else {
            let was = rotated_word(bus.read32(addr & !3), addr);
            bus.write32(addr & !3, self.reg(rm));
            self.set_reg(rd, was);
        }
        4
    }

    fn halfword(&mut self, insn: u32, bus: &mut impl Bus) -> u32 {
        let pre = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xf) as usize;
        let rd = ((insn >> 12) & 0xf) as usize;

        let offset = if insn & (1 << 22) != 0 {
            ((insn >> 4) & 0xf0) | (insn & 0xf)
        } else {
            self.reg((insn & 0xf) as usize)
        };

        let base = self.reg(rn);
        let moved = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if pre { moved } else { base };

        if load {
            let v = match (insn >> 5) & 3 {
                1 => u32::from(bus.read16(addr & !1)),
                2 => i32::from(bus.read8(addr) as i8) as u32,
                _ => i32::from(bus.read16(addr & !1) as i16) as u32,
            };
            // The write-back happens first so that `LDR r0, [r0], #4` loads
            // into the register it was addressing through.
            if !pre || writeback {
                self.set_reg(rn, moved);
            }
            self.set_reg(rd, v);
        } else {
            bus.write16(addr & !1, self.reg(rd) as u16);
            if !pre || writeback {
                self.set_reg(rn, moved);
            }
        }
        if load { 3 } else { 2 }
    }

    fn single_transfer(&mut self, insn: u32, bus: &mut impl Bus) -> u32 {
        let pre = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let byte = insn & (1 << 22) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xf) as usize;
        let rd = ((insn >> 12) & 0xf) as usize;

        let offset = if insn & (1 << 25) != 0 {
            let rm = (insn & 0xf) as usize;
            let kind = (insn >> 5) & 3;
            let amount = (insn >> 7) & 0x1f;
            self.shift(self.reg(rm), kind, amount, false).0
        } else {
            insn & 0xfff
        };

        let base = self.reg(rn);
        let moved = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if pre { moved } else { base };

        if load {
            let v = if byte {
                u32::from(bus.read8(addr))
            } else {
                // An unaligned word load does not fault on this part: it reads
                // the aligned word and rotates it, which compilers of the era
                // used deliberately.
                rotated_word(bus.read32(addr & !3), addr)
            };
            if !pre || writeback {
                self.set_reg(rn, moved);
            }
            self.set_reg(rd, v);
            if rd == 15 { 5 } else { 3 }
        } else {
            // Storing R15 stores the pipeline value, twelve on.
            let v = if rd == 15 {
                self.reg(15).wrapping_add(4)
            } else {
                self.reg(rd)
            };
            if byte {
                bus.write8(addr, v as u8);
            } else {
                bus.write32(addr & !3, v);
            }
            if !pre || writeback {
                self.set_reg(rn, moved);
            }
            2
        }
    }

    fn block_transfer(&mut self, insn: u32, bus: &mut impl Bus) -> u32 {
        let pre = insn & (1 << 24) != 0;
        let up = insn & (1 << 23) != 0;
        let user = insn & (1 << 22) != 0;
        let writeback = insn & (1 << 21) != 0;
        let load = insn & (1 << 20) != 0;
        let rn = ((insn >> 16) & 0xf) as usize;
        let list = insn & 0xffff;

        let count = list.count_ones();
        let base = self.reg(rn);
        // The lowest-numbered register always goes to the lowest address,
        // whatever order the bits are written in, so the block is worked out
        // once and walked upwards either way.
        let start = if up {
            base.wrapping_add(if pre { 4 } else { 0 })
        } else {
            base.wrapping_sub(count * 4)
                .wrapping_add(if pre { 0 } else { 4 })
        };
        let end = if up {
            base.wrapping_add(count * 4)
        } else {
            base.wrapping_sub(count * 4)
        };

        // `^` on a load with R15 in the list restores the saved status; on
        // anything else it means the user-mode bank.
        let restore = user && load && list & (1 << 15) != 0;
        let banked = user && !restore;

        let mut addr = start;
        for i in 0..16 {
            if list & (1 << i) == 0 {
                continue;
            }
            if load {
                let v = bus.read32(addr & !3);
                if banked {
                    self.set_user_reg(i, v);
                } else {
                    self.set_reg(i, v);
                }
            } else {
                let v = if i == 15 {
                    self.reg(15).wrapping_add(4)
                } else if banked {
                    self.user_reg(i)
                } else {
                    self.reg(i)
                };
                bus.write32(addr & !3, v);
            }
            addr = addr.wrapping_add(4);
        }

        // Write-back to a register that was loaded is the loaded value, not
        // the new base — the architecture calls it unpredictable and the part
        // keeps the load.
        if writeback && !(load && list & (1 << rn) != 0) {
            self.set_reg(rn, end);
        }
        if restore {
            let spsr = self.spsr();
            self.set_mode(Mode::from_bits(spsr));
            self.cpsr = spsr;
        }
        count + if load { 2 } else { 1 }
    }
}

/// A word read from an unaligned address comes back rotated. See
/// [`Cpu::single_transfer`].
fn rotated_word(word: u32, addr: u32) -> u32 {
    word.rotate_right((addr & 3) * 8)
}

/// `a + b + carry`, with the flags an addition sets.
fn add(a: u32, b: u32, carry: u32) -> (u32, bool, Option<bool>, bool) {
    let wide = u64::from(a) + u64::from(b) + u64::from(carry);
    let result = wide as u32;
    let overflow = (!(a ^ b) & (a ^ result)) & 0x8000_0000 != 0;
    (result, wide > u64::from(u32::MAX), Some(overflow), true)
}

/// `a - b - !carry`, with the flags a subtraction sets. The carry out of a
/// subtraction is **not borrow**: it is set when there was no borrow.
fn sub(a: u32, b: u32, carry: u32) -> (u32, bool, Option<bool>, bool) {
    let wide = u64::from(a)
        .wrapping_add(u64::from(!b))
        .wrapping_add(u64::from(carry));
    let result = wide as u32;
    let overflow = ((a ^ b) & (a ^ result)) & 0x8000_0000 != 0;
    (result, wide > u64::from(u32::MAX), Some(overflow), true)
}
