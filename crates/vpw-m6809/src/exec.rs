//! Decoding and execution.
//!
//! The 6809 has three opcode pages: the main one and two prefixed by `$10`
//! and `$11`. Written from the Motorola specification; see the licence note in
//! `lib.rs`.

use crate::{Bus, Ccr, Cpu, vectors};

/// Addressing modes of the accumulator group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Immediate,
    Direct,
    Indexed,
    Extended,
}

impl Cpu {
    pub(crate) fn execute(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            0x10 => {
                let second = self.fetch_u8(bus);
                self.execute_page1(bus, second)
            }
            0x11 => {
                let second = self.fetch_u8(bus);
                self.execute_page2(bus, second)
            }

            // --- Read-modify-write on the direct page ----------------------------
            0x00..=0x0F => self.memory_rmw(bus, opcode, Mode::Direct, 6, 3),

            // --- Inherent and control -------------------------------------------
            0x12 => 2, // NOP
            0x13 => {
                // SYNC: halts until an interrupt line goes down.
                // The datasheet gives 2 cycles as a **minimum**; 4 are used,
                // which is what the reference implementation counts.
                self.syncing = true;
                4
            }
            0x16 => {
                // LBRA
                let offset = self.fetch_u16(bus) as i16;
                self.pc = self.pc.wrapping_add_signed(offset);
                5
            }
            0x17 => {
                // LBSR
                let offset = self.fetch_u16(bus) as i16;
                self.push_return_address(bus);
                self.pc = self.pc.wrapping_add_signed(offset);
                9
            }
            0x19 => {
                self.daa();
                2
            }
            0x1A => {
                // ORCC
                let mask = self.fetch_u8(bus);
                self.cc = Ccr::from_bits(self.cc.bits() | mask);
                3
            }
            0x1C => {
                // ANDCC
                let mask = self.fetch_u8(bus);
                self.cc = Ccr::from_bits(self.cc.bits() & mask);
                3
            }
            0x1D => {
                // SEX: extends the sign of B over A.
                self.a = if self.b & 0x80 != 0 { 0xFF } else { 0x00 };
                let d = self.d();
                self.cc.set_nz16(d);
                2
            }
            0x1E => {
                self.exchange(bus);
                8
            }
            0x1F => {
                self.transfer(bus);
                6
            }

            // --- Short branches --------------------------------------------------
            0x20..=0x2F => {
                let offset = self.fetch_u8(bus) as i8;
                if self.branch_taken(opcode) {
                    self.pc = self.pc.wrapping_add_signed(i16::from(offset));
                }
                3
            }

            // --- LEA ---------------------------------------------------------------
            0x30..=0x33 => {
                let (address, extra) = self.decode_indexed(bus);
                match opcode {
                    // LEAX and LEAY affect Z; LEAS and LEAU touch nothing. The
                    // asymmetry is real: it lets you use LEAS/LEAU to adjust
                    // the stack without losing the flags.
                    0x30 => {
                        self.x = address;
                        self.cc.z = address == 0;
                    }
                    0x31 => {
                        self.y = address;
                        self.cc.z = address == 0;
                    }
                    0x32 => self.s = address,
                    _ => self.u = address,
                }
                4 + extra
            }

            // --- Stack --------------------------------------------------------------
            0x34 => self.push_multiple(bus, true),
            0x35 => self.pull_multiple(bus, true),
            0x36 => self.push_multiple(bus, false),
            0x37 => self.pull_multiple(bus, false),

            0x39 => {
                // RTS
                let hi = self.pull_s(bus);
                let lo = self.pull_s(bus);
                self.pc = u16::from_be_bytes([hi, lo]);
                5
            }
            0x3A => {
                // ABX: adds B to X, unsigned and without touching flags.
                self.x = self.x.wrapping_add(u16::from(self.b));
                3
            }
            0x3B => self.rti(bus),
            0x3C => {
                // CWAI: clears the masks it is asked to, stacks the entire
                // state and halts.
                let mask = self.fetch_u8(bus);
                self.cc = Ccr::from_bits(self.cc.bits() & mask);
                self.cc.e = true;
                self.push_entire_state(bus);
                self.waiting = true;
                20
            }
            0x3D => {
                // MUL: A times B, unsigned, result in D.
                let result = u16::from(self.a) * u16::from(self.b);
                self.set_d(result);
                self.cc.z = result == 0;
                // The carry comes out of bit 7 of the result, so you can round.
                self.cc.c = result & 0x80 != 0;
                11
            }
            0x3F => {
                // SWI: stacks everything and masks both interrupts.
                self.cc.e = true;
                self.push_entire_state(bus);
                self.cc.i = true;
                self.cc.f = true;
                self.pc = bus.read_u16(vectors::SWI);
                19
            }

            // --- Accumulator operations -------------------------------------
            0x40..=0x4F => self.accumulator_rmw(opcode, false),
            0x50..=0x5F => self.accumulator_rmw(opcode, true),

            // --- Indexed and extended read-modify-write ---------------------------
            0x60..=0x6F => self.memory_rmw(bus, opcode, Mode::Indexed, 6, 3),
            0x70..=0x7F => self.memory_rmw(bus, opcode, Mode::Extended, 7, 4),

            // --- Accumulator group -------------------------------------------
            0x80..=0xFF => self.accumulator_op(bus, opcode),

            _ => self.illegal(opcode),
        }
    }

    // --- Prefixed pages ------------------------------------------------------------

    /// Page `$10`: long branches, `Y`, `S` and the 16-bit comparisons that did
    /// not fit in the main page.
    fn execute_page1(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            0x21..=0x2F => {
                // Long branches: same predicate, 16-bit offset.
                let offset = self.fetch_u16(bus) as i16;
                if self.branch_taken(opcode) {
                    self.pc = self.pc.wrapping_add_signed(offset);
                    6
                } else {
                    5
                }
            }
            0x3F => {
                // SWI2 masks nothing: it is a system call, not an error
                // condition.
                self.cc.e = true;
                self.push_entire_state(bus);
                self.pc = bus.read_u16(vectors::SWI2);
                20
            }
            // CMPD, CMPY, LDY, STY and LDS/STS. They cost one cycle more than
            // their equivalents on the main page, because of the prefix.
            0x83 | 0x93 | 0xA3 | 0xB3 => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                let d = self.d();
                self.compare16(d, value);
                wide_cycles(mode_of(opcode)) + 1 + extra
            }
            0x8C | 0x9C | 0xAC | 0xBC => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                self.compare16(self.y, value);
                wide_cycles(mode_of(opcode)) + 1 + extra
            }
            0x8E | 0x9E | 0xAE | 0xBE => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                self.y = value;
                self.cc.set_nz16_v0(value);
                load16_cycles(mode_of(opcode)) + 1 + extra
            }
            0x9F | 0xAF | 0xBF => {
                let (address, extra) = self.address_of(bus, mode_of(opcode));
                self.store16(bus, address, self.y);
                load16_cycles(mode_of(opcode)) + 1 + extra
            }
            0xCE | 0xDE | 0xEE | 0xFE => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                self.s = value;
                self.cc.set_nz16_v0(value);
                load16_cycles(mode_of(opcode)) + 1 + extra
            }
            0xDF | 0xEF | 0xFF => {
                let (address, extra) = self.address_of(bus, mode_of(opcode));
                self.store16(bus, address, self.s);
                load16_cycles(mode_of(opcode)) + 1 + extra
            }
            _ => self.illegal(opcode),
        }
    }

    /// Page `$11`: only `CMPU`, `CMPS` and `SWI3`.
    fn execute_page2(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            0x3F => {
                self.cc.e = true;
                self.push_entire_state(bus);
                self.pc = bus.read_u16(vectors::SWI3);
                20
            }
            0x83 | 0x93 | 0xA3 | 0xB3 => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                self.compare16(self.u, value);
                wide_cycles(mode_of(opcode)) + 1 + extra
            }
            0x8C | 0x9C | 0xAC | 0xBC => {
                let (value, extra) = self.operand16(bus, mode_of(opcode));
                self.compare16(self.s, value);
                wide_cycles(mode_of(opcode)) + 1 + extra
            }
            _ => self.illegal(opcode),
        }
    }

    // --- Instruction groups ----------------------------------------------------

    /// `$40`-`$5F`: the same family of operations on `A` or on `B`.
    fn accumulator_rmw(&mut self, opcode: u8, use_b: bool) -> u32 {
        let value = if use_b { self.b } else { self.a };
        let result = match opcode & 0x0F {
            0x00 => self.neg(value),
            0x03 => self.com(value),
            0x04 => self.lsr(value),
            0x06 => self.ror(value),
            0x07 => self.asr(value),
            0x08 => self.asl(value),
            0x09 => self.rol(value),
            0x0A => self.dec(value),
            0x0C => self.inc(value),
            0x0D => {
                self.tst(value);
                return 2;
            }
            0x0F => self.clr(),
            _ => return self.illegal(opcode),
        };
        if use_b {
            self.b = result;
        } else {
            self.a = result;
        }
        2
    }

    /// `$00`-`$0F`, `$60`-`$6F` and `$70`-`$7F`: the same family on memory.
    fn memory_rmw(
        &mut self,
        bus: &mut impl Bus,
        opcode: u8,
        mode: Mode,
        cycles: u32,
        jmp_cycles: u32,
    ) -> u32 {
        // The undefined opcodes have to be discarded **before** reading the
        // address: otherwise an operand byte that does not really exist gets
        // consumed and the PC ends up shifted.
        if matches!(opcode & 0x0F, 0x01 | 0x02 | 0x05 | 0x0B) {
            return self.illegal(opcode);
        }

        let (address, extra) = self.address_of(bus, mode);

        match opcode & 0x0F {
            0x0E => {
                // JMP
                self.pc = address;
                jmp_cycles + extra
            }
            0x0D => {
                let value = bus.read(address);
                self.tst(value);
                cycles + extra
            }
            0x0F => {
                // CLR writes without reading.
                let value = self.clr();
                bus.write(address, value);
                cycles + extra
            }
            op => {
                let value = bus.read(address);
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
                bus.write(address, result);
                cycles + extra
            }
        }
    }

    /// `$80`-`$FF`. Bits 4-5 pick the mode, bit 6 the accumulator and the low
    /// nibble the operation.
    fn accumulator_op(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        let mode = mode_of(opcode);
        let use_b = opcode & 0x40 != 0;

        match opcode & 0x0F {
            // 8-bit operations, all with the same shape.
            op @ (0x00 | 0x01 | 0x02 | 0x04 | 0x05 | 0x06 | 0x08 | 0x09 | 0x0A | 0x0B) => {
                let (m, extra) = self.operand8(bus, mode);
                let acc = if use_b { self.b } else { self.a };
                let result = match op {
                    0x00 => self.sub8(acc, m),
                    0x01 => {
                        self.sub8(acc, m);
                        acc
                    }
                    0x02 => self.sbc8(acc, m),
                    0x04 => {
                        let r = acc & m;
                        self.cc.set_nz8_v0(r);
                        r
                    }
                    0x05 => {
                        let r = acc & m;
                        self.cc.set_nz8_v0(r);
                        acc
                    }
                    0x06 => {
                        self.cc.set_nz8_v0(m);
                        m
                    }
                    0x08 => {
                        let r = acc ^ m;
                        self.cc.set_nz8_v0(r);
                        r
                    }
                    0x09 => self.add8(acc, m, self.cc.c),
                    0x0A => {
                        let r = acc | m;
                        self.cc.set_nz8_v0(r);
                        r
                    }
                    _ => self.add8(acc, m, false),
                };
                if use_b {
                    self.b = result;
                } else {
                    self.a = result;
                }
                alu_cycles(mode) + extra
            }

            // STA / STB. Does not exist in immediate mode.
            0x07 => {
                if mode == Mode::Immediate {
                    return self.illegal(opcode);
                }
                let (address, extra) = self.address_of(bus, mode);
                let value = if use_b { self.b } else { self.a };
                self.cc.set_nz8_v0(value);
                bus.write(address, value);
                alu_cycles(mode) + extra
            }

            // SUBD (column A) / ADDD (column B).
            0x03 => {
                let (value, extra) = self.operand16(bus, mode);
                let d = self.d();
                let result = if use_b {
                    self.add16(d, value)
                } else {
                    self.sub16(d, value)
                };
                self.set_d(result);
                wide_cycles(mode) + extra
            }

            // CMPX (column A) / LDD (column B).
            0x0C => {
                let (value, extra) = self.operand16(bus, mode);
                if use_b {
                    self.set_d(value);
                    self.cc.set_nz16_v0(value);
                    load16_cycles(mode) + extra
                } else {
                    self.compare16(self.x, value);
                    wide_cycles(mode) + extra
                }
            }

            // BSR/JSR (column A) / STD (column B).
            0x0D => {
                if use_b {
                    if mode == Mode::Immediate {
                        return self.illegal(opcode);
                    }
                    let (address, extra) = self.address_of(bus, mode);
                    let d = self.d();
                    self.store16(bus, address, d);
                    load16_cycles(mode) + extra
                } else if mode == Mode::Immediate {
                    // BSR: the "immediate" of this column is a relative offset.
                    let offset = self.fetch_u8(bus) as i8;
                    self.push_return_address(bus);
                    self.pc = self.pc.wrapping_add_signed(i16::from(offset));
                    7
                } else {
                    let (address, extra) = self.address_of(bus, mode);
                    self.push_return_address(bus);
                    self.pc = address;
                    jsr_cycles(mode) + extra
                }
            }

            // LDX (column A) / LDU (column B).
            0x0E => {
                let (value, extra) = self.operand16(bus, mode);
                if use_b {
                    self.u = value;
                } else {
                    self.x = value;
                }
                self.cc.set_nz16_v0(value);
                load16_cycles(mode) + extra
            }

            // STX (column A) / STU (column B).
            0x0F => {
                if mode == Mode::Immediate {
                    return self.illegal(opcode);
                }
                let (address, extra) = self.address_of(bus, mode);
                let value = if use_b { self.u } else { self.x };
                self.store16(bus, address, value);
                load16_cycles(mode) + extra
            }

            _ => self.illegal(opcode),
        }
    }

    // --- Transfers and stack ---------------------------------------------------

    /// `TFR`: copies one register into another. The postbyte carries source
    /// and destination.
    fn transfer(&mut self, bus: &mut impl Bus) {
        let postbyte = self.fetch_u8(bus);
        let value = self.read_tfr_register(postbyte >> 4);
        self.write_tfr_register(postbyte & 0x0F, value);
    }

    /// `EXG`: swaps two registers.
    fn exchange(&mut self, bus: &mut impl Bus) {
        let postbyte = self.fetch_u8(bus);
        let (src, dst) = (postbyte >> 4, postbyte & 0x0F);
        let a = self.read_tfr_register(src);
        let b = self.read_tfr_register(dst);
        self.write_tfr_register(dst, a);
        self.write_tfr_register(src, b);
    }

    /// Register code of `TFR`/`EXG`. The 8-bit ones are read replicated into
    /// both bytes, which is what the chip does.
    fn read_tfr_register(&self, code: u8) -> u16 {
        match code {
            0x0 => self.d(),
            0x1 => self.x,
            0x2 => self.y,
            0x3 => self.u,
            0x4 => self.s,
            0x5 => self.pc,
            0x8 => u16::from(self.a) | 0xFF00,
            0x9 => u16::from(self.b) | 0xFF00,
            0xA => u16::from(self.cc.bits()) | 0xFF00,
            0xB => u16::from(self.dp) | 0xFF00,
            _ => 0xFFFF,
        }
    }

    fn write_tfr_register(&mut self, code: u8, value: u16) {
        match code {
            0x0 => self.set_d(value),
            0x1 => self.x = value,
            0x2 => self.y = value,
            0x3 => self.u = value,
            0x4 => self.s = value,
            0x5 => self.pc = value,
            0x8 => self.a = value as u8,
            0x9 => self.b = value as u8,
            0xA => self.cc = Ccr::from_bits(value as u8),
            0xB => self.dp = value as u8,
            _ => {}
        }
    }

    /// `PSHS` / `PSHU`. The postbyte is a bitmap of what to stack.
    ///
    /// The order goes from highest to lowest —PC, the other stack, Y, X, DP,
    /// B, A, CC— so that pulling in the reverse order puts everything back in
    /// its place.
    fn push_multiple(&mut self, bus: &mut impl Bus, to_system_stack: bool) -> u32 {
        let postbyte = self.fetch_u8(bus);
        let mut bytes = 0;

        macro_rules! push {
            ($value:expr) => {
                if to_system_stack {
                    self.push_s(bus, $value)
                } else {
                    self.push_u(bus, $value)
                }
            };
        }

        if postbyte & 0x80 != 0 {
            let [hi, lo] = self.pc.to_be_bytes();
            push!(lo);
            push!(hi);
            bytes += 2;
        }
        if postbyte & 0x40 != 0 {
            // Bit 6 means "the other stack": U if we are using S, and S if we
            // are using U.
            let other = if to_system_stack { self.u } else { self.s };
            let [hi, lo] = other.to_be_bytes();
            push!(lo);
            push!(hi);
            bytes += 2;
        }
        if postbyte & 0x20 != 0 {
            let [hi, lo] = self.y.to_be_bytes();
            push!(lo);
            push!(hi);
            bytes += 2;
        }
        if postbyte & 0x10 != 0 {
            let [hi, lo] = self.x.to_be_bytes();
            push!(lo);
            push!(hi);
            bytes += 2;
        }
        if postbyte & 0x08 != 0 {
            push!(self.dp);
            bytes += 1;
        }
        if postbyte & 0x04 != 0 {
            push!(self.b);
            bytes += 1;
        }
        if postbyte & 0x02 != 0 {
            push!(self.a);
            bytes += 1;
        }
        if postbyte & 0x01 != 0 {
            push!(self.cc.bits());
            bytes += 1;
        }

        5 + bytes
    }

    /// `PULS` / `PULU`: the reverse of the stacking order.
    fn pull_multiple(&mut self, bus: &mut impl Bus, from_system_stack: bool) -> u32 {
        let postbyte = self.fetch_u8(bus);
        let mut bytes = 0;

        macro_rules! pull {
            () => {
                if from_system_stack {
                    self.pull_s(bus)
                } else {
                    self.pull_u(bus)
                }
            };
        }

        if postbyte & 0x01 != 0 {
            self.cc = Ccr::from_bits(pull!());
            bytes += 1;
        }
        if postbyte & 0x02 != 0 {
            self.a = pull!();
            bytes += 1;
        }
        if postbyte & 0x04 != 0 {
            self.b = pull!();
            bytes += 1;
        }
        if postbyte & 0x08 != 0 {
            self.dp = pull!();
            bytes += 1;
        }
        if postbyte & 0x10 != 0 {
            let hi = pull!();
            let lo = pull!();
            self.x = u16::from_be_bytes([hi, lo]);
            bytes += 2;
        }
        if postbyte & 0x20 != 0 {
            let hi = pull!();
            let lo = pull!();
            self.y = u16::from_be_bytes([hi, lo]);
            bytes += 2;
        }
        if postbyte & 0x40 != 0 {
            let hi = pull!();
            let lo = pull!();
            let value = u16::from_be_bytes([hi, lo]);
            if from_system_stack {
                self.u = value;
            } else {
                self.s = value;
            }
            bytes += 2;
        }
        if postbyte & 0x80 != 0 {
            let hi = pull!();
            let lo = pull!();
            self.pc = u16::from_be_bytes([hi, lo]);
            bytes += 2;
        }

        5 + bytes
    }

    /// `RTI`. How much it pulls depends on the `E` flag the interrupt left
    /// behind: a FIRQ only saved `CC` and `PC`.
    fn rti(&mut self, bus: &mut impl Bus) -> u32 {
        self.cc = Ccr::from_bits(self.pull_s(bus));

        if self.cc.e {
            self.a = self.pull_s(bus);
            self.b = self.pull_s(bus);
            self.dp = self.pull_s(bus);
            let xh = self.pull_s(bus);
            let xl = self.pull_s(bus);
            self.x = u16::from_be_bytes([xh, xl]);
            let yh = self.pull_s(bus);
            let yl = self.pull_s(bus);
            self.y = u16::from_be_bytes([yh, yl]);
            let uh = self.pull_s(bus);
            let ul = self.pull_s(bus);
            self.u = u16::from_be_bytes([uh, ul]);
            let pch = self.pull_s(bus);
            let pcl = self.pull_s(bus);
            self.pc = u16::from_be_bytes([pch, pcl]);
            15
        } else {
            let pch = self.pull_s(bus);
            let pcl = self.pull_s(bus);
            self.pc = u16::from_be_bytes([pch, pcl]);
            6
        }
    }

    fn push_return_address(&mut self, bus: &mut impl Bus) {
        let [hi, lo] = self.pc.to_be_bytes();
        self.push_s(bus, lo);
        self.push_s(bus, hi);
    }

    // --- Addressing ---------------------------------------------------------

    /// Effective address and extra cycles. Does not apply to immediate.
    fn address_of(&mut self, bus: &mut impl Bus, mode: Mode) -> (u16, u32) {
        match mode {
            // Direct mode uses DP as the high byte, so "page zero" can be moved
            // anywhere in the map.
            Mode::Direct => {
                let low = self.fetch_u8(bus);
                (u16::from_be_bytes([self.dp, low]), 0)
            }
            Mode::Indexed => self.decode_indexed(bus),
            Mode::Extended => (self.fetch_u16(bus), 0),
            Mode::Immediate => unreachable!("immediate mode has no effective address"),
        }
    }

    fn operand8(&mut self, bus: &mut impl Bus, mode: Mode) -> (u8, u32) {
        match mode {
            Mode::Immediate => (self.fetch_u8(bus), 0),
            _ => {
                let (address, extra) = self.address_of(bus, mode);
                (bus.read(address), extra)
            }
        }
    }

    fn operand16(&mut self, bus: &mut impl Bus, mode: Mode) -> (u16, u32) {
        match mode {
            Mode::Immediate => (self.fetch_u16(bus), 0),
            _ => {
                let (address, extra) = self.address_of(bus, mode);
                (bus.read_u16(address), extra)
            }
        }
    }

    fn store16(&mut self, bus: &mut impl Bus, address: u16, value: u16) {
        self.cc.set_nz16_v0(value);
        let [hi, lo] = value.to_be_bytes();
        bus.write(address, hi);
        bus.write(address.wrapping_add(1), lo);
    }

    /// Branch condition. The low nibble of the opcode picks it, so it works
    /// the same for the short ones and for the long ones of page `$10`.
    fn branch_taken(&self, opcode: u8) -> bool {
        let cc = self.cc;
        match opcode & 0x0F {
            0x0 => true,                  // BRA
            0x1 => false,                 // BRN: never. It exists as a long NOP.
            0x2 => !cc.c && !cc.z,        // BHI
            0x3 => cc.c || cc.z,          // BLS
            0x4 => !cc.c,                 // BCC / BHS
            0x5 => cc.c,                  // BCS / BLO
            0x6 => !cc.z,                 // BNE
            0x7 => cc.z,                  // BEQ
            0x8 => !cc.v,                 // BVC
            0x9 => cc.v,                  // BVS
            0xA => !cc.n,                 // BPL
            0xB => cc.n,                  // BMI
            0xC => cc.n == cc.v,          // BGE
            0xD => cc.n != cc.v,          // BLT
            0xE => !cc.z && cc.n == cc.v, // BGT
            _ => cc.z || cc.n != cc.v,    // BLE
        }
    }

    fn illegal(&mut self, opcode: u8) -> u32 {
        self.last_illegal = Some((self.pc.wrapping_sub(1), opcode));
        2
    }

    // --- ALU -----------------------------------------------------------------------------

    fn add8(&mut self, lhs: u8, rhs: u8, carry_in: bool) -> u8 {
        let carry = u16::from(carry_in);
        let sum = u16::from(lhs) + u16::from(rhs) + carry;
        let result = sum as u8;
        self.cc.h = (lhs & 0x0F) + (rhs & 0x0F) + carry as u8 > 0x0F;
        self.cc.set_nz8(result);
        self.cc.v = (lhs ^ result) & (rhs ^ result) & 0x80 != 0;
        self.cc.c = sum > 0xFF;
        result
    }

    fn sub8(&mut self, lhs: u8, rhs: u8) -> u8 {
        let result = lhs.wrapping_sub(rhs);
        self.cc.set_nz8(result);
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

    fn add16(&mut self, lhs: u16, rhs: u16) -> u16 {
        let sum = u32::from(lhs) + u32::from(rhs);
        let result = sum as u16;
        self.cc.set_nz16(result);
        self.cc.v = (lhs ^ result) & (rhs ^ result) & 0x8000 != 0;
        self.cc.c = sum > 0xFFFF;
        result
    }

    fn sub16(&mut self, lhs: u16, rhs: u16) -> u16 {
        let result = lhs.wrapping_sub(rhs);
        self.cc.set_nz16(result);
        self.cc.v = (lhs ^ rhs) & (lhs ^ result) & 0x8000 != 0;
        self.cc.c = lhs < rhs;
        result
    }

    /// Unlike the 6800, here a 16-bit `CMP` **does** affect the carry: the
    /// 6809 fixed that oddity.
    fn compare16(&mut self, lhs: u16, rhs: u16) {
        self.sub16(lhs, rhs);
    }

    fn neg(&mut self, value: u8) -> u8 {
        let result = 0u8.wrapping_sub(value);
        self.cc.set_nz8(result);
        self.cc.v = result == 0x80;
        self.cc.c = result != 0x00;
        result
    }

    fn com(&mut self, value: u8) -> u8 {
        let result = !value;
        self.cc.set_nz8(result);
        self.cc.v = false;
        self.cc.c = true;
        result
    }

    fn inc(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.cc.set_nz8(result);
        self.cc.v = value == 0x7F;
        result
    }

    fn dec(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.cc.set_nz8(result);
        self.cc.v = value == 0x80;
        result
    }

    /// `TST` does not touch the carry. On the 6800 it cleared it.
    fn tst(&mut self, value: u8) {
        self.cc.set_nz8_v0(value);
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
        // The overflow comes out of the top two bits of the **original**, not
        // of `N xor C` as on the 6800.
        self.cc.v = (value ^ (value << 1)) & 0x80 != 0;
        self.cc.c = value & 0x80 != 0;
        self.cc.set_nz8(result);
        result
    }

    fn asr(&mut self, value: u8) -> u8 {
        let result = (value >> 1) | (value & 0x80);
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result);
        result
    }

    fn lsr(&mut self, value: u8) -> u8 {
        let result = value >> 1;
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result);
        result
    }

    fn rol(&mut self, value: u8) -> u8 {
        let result = (value << 1) | u8::from(self.cc.c);
        self.cc.v = (value ^ (value << 1)) & 0x80 != 0;
        self.cc.c = value & 0x80 != 0;
        self.cc.set_nz8(result);
        result
    }

    fn ror(&mut self, value: u8) -> u8 {
        let result = (value >> 1) | (u8::from(self.cc.c) << 7);
        self.cc.c = value & 0x01 != 0;
        self.cc.set_nz8(result);
        result
    }

    fn daa(&mut self) {
        let high = self.a >> 4;
        let low = self.a & 0x0F;
        let mut correction = 0u8;
        let mut carry = self.cc.c;

        if low > 9 || self.cc.h {
            correction |= 0x06;
        }
        if high > 9 || self.cc.c || (high >= 9 && low > 9) {
            correction |= 0x60;
            carry = true;
        }

        self.a = self.a.wrapping_add(correction);
        self.cc.set_nz8(self.a);
        self.cc.c = carry;
    }
}

/// The mode comes out of bits 4-5 of the opcode, the same on all three pages.
fn mode_of(opcode: u8) -> Mode {
    match (opcode >> 4) & 0x03 {
        0 => Mode::Immediate,
        1 => Mode::Direct,
        2 => Mode::Indexed,
        _ => Mode::Extended,
    }
}

/// Cycles of the 8-bit ALU operations.
fn alu_cycles(mode: Mode) -> u32 {
    match mode {
        Mode::Immediate => 2,
        Mode::Direct | Mode::Indexed => 4,
        Mode::Extended => 5,
    }
}

/// Cycles of the 16-bit loads and stores.
fn load16_cycles(mode: Mode) -> u32 {
    match mode {
        Mode::Immediate => 3,
        Mode::Direct | Mode::Indexed => 5,
        Mode::Extended => 6,
    }
}

/// Cycles of the 16-bit arithmetic and comparisons.
fn wide_cycles(mode: Mode) -> u32 {
    match mode {
        Mode::Immediate => 4,
        Mode::Direct | Mode::Indexed => 6,
        Mode::Extended => 7,
    }
}

fn jsr_cycles(mode: Mode) -> u32 {
    match mode {
        Mode::Extended => 8,
        _ => 7,
    }
}
