//! The instruction set: one `match` on the opcode.
//!
//! Written from the MOS specification rather than ported — see the crate's
//! note on why. The shape is the same as the 6800's next door: fetch, decode,
//! do it, and return the cycles the chip would have taken.
//!
//! # The cycle counts
//!
//! The base figure is in each arm. Three things add to it and all three are
//! real:
//!
//! * an indexed **read** that crosses a page costs one more, because the chip
//!   fetches from the wrong page first and then fixes the high byte;
//! * an indexed **write** or read-modify-write costs the extra *always*, since
//!   the fix-up happens whether or not it was needed — which is why `STA
//!   $1234,X` is five cycles and `LDA $1234,X` is four;
//! * a taken branch costs one more, and one more again if it lands on another
//!   page.

use crate::{Bus, Cpu, flags, read_u16};

/// Runs one instruction and returns the cycles it took.
pub(crate) fn step(cpu: &mut Cpu, bus: &mut impl Bus) -> u32 {
    let pc = cpu.pc;
    let op = fetch(cpu, bus);
    match op {
        // -- load and store ------------------------------------------------
        0xA9 => {
            let v = imm(cpu, bus);
            cpu.a = v;
            cpu.set_zn(v);
            2
        }
        0xA5 => {
            let a = zp(cpu, bus);
            load_a(cpu, bus, a);
            3
        }
        0xB5 => {
            let a = zpx(cpu, bus);
            load_a(cpu, bus, a);
            4
        }
        0xAD => {
            let a = abs(cpu, bus);
            load_a(cpu, bus, a);
            4
        }
        0xBD => {
            let (a, x) = absx(cpu, bus);
            load_a(cpu, bus, a);
            4 + x
        }
        0xB9 => {
            let (a, x) = absy(cpu, bus);
            load_a(cpu, bus, a);
            4 + x
        }
        0xA1 => {
            let a = indx(cpu, bus);
            load_a(cpu, bus, a);
            6
        }
        0xB1 => {
            let (a, x) = indy(cpu, bus);
            load_a(cpu, bus, a);
            5 + x
        }

        0xA2 => {
            let v = imm(cpu, bus);
            cpu.x = v;
            cpu.set_zn(v);
            2
        }
        0xA6 => {
            let a = zp(cpu, bus);
            load_x(cpu, bus, a);
            3
        }
        0xB6 => {
            let a = zpy(cpu, bus);
            load_x(cpu, bus, a);
            4
        }
        0xAE => {
            let a = abs(cpu, bus);
            load_x(cpu, bus, a);
            4
        }
        0xBE => {
            let (a, x) = absy(cpu, bus);
            load_x(cpu, bus, a);
            4 + x
        }

        0xA0 => {
            let v = imm(cpu, bus);
            cpu.y = v;
            cpu.set_zn(v);
            2
        }
        0xA4 => {
            let a = zp(cpu, bus);
            load_y(cpu, bus, a);
            3
        }
        0xB4 => {
            let a = zpx(cpu, bus);
            load_y(cpu, bus, a);
            4
        }
        0xAC => {
            let a = abs(cpu, bus);
            load_y(cpu, bus, a);
            4
        }
        0xBC => {
            let (a, x) = absx(cpu, bus);
            load_y(cpu, bus, a);
            4 + x
        }

        0x85 => {
            let a = zp(cpu, bus);
            bus.write(a, cpu.a);
            3
        }
        0x95 => {
            let a = zpx(cpu, bus);
            bus.write(a, cpu.a);
            4
        }
        0x8D => {
            let a = abs(cpu, bus);
            bus.write(a, cpu.a);
            4
        }
        0x9D => {
            let (a, _) = absx(cpu, bus);
            bus.write(a, cpu.a);
            5
        }
        0x99 => {
            let (a, _) = absy(cpu, bus);
            bus.write(a, cpu.a);
            5
        }
        0x81 => {
            let a = indx(cpu, bus);
            bus.write(a, cpu.a);
            6
        }
        0x91 => {
            let (a, _) = indy(cpu, bus);
            bus.write(a, cpu.a);
            6
        }

        0x86 => {
            let a = zp(cpu, bus);
            bus.write(a, cpu.x);
            3
        }
        0x96 => {
            let a = zpy(cpu, bus);
            bus.write(a, cpu.x);
            4
        }
        0x8E => {
            let a = abs(cpu, bus);
            bus.write(a, cpu.x);
            4
        }

        0x84 => {
            let a = zp(cpu, bus);
            bus.write(a, cpu.y);
            3
        }
        0x94 => {
            let a = zpx(cpu, bus);
            bus.write(a, cpu.y);
            4
        }
        0x8C => {
            let a = abs(cpu, bus);
            bus.write(a, cpu.y);
            4
        }

        // -- moving between registers --------------------------------------
        0xAA => {
            cpu.x = cpu.a;
            cpu.set_zn(cpu.x);
            2
        }
        0xA8 => {
            cpu.y = cpu.a;
            cpu.set_zn(cpu.y);
            2
        }
        0x8A => {
            cpu.a = cpu.x;
            cpu.set_zn(cpu.a);
            2
        }
        0x98 => {
            cpu.a = cpu.y;
            cpu.set_zn(cpu.a);
            2
        }
        0xBA => {
            cpu.x = cpu.s;
            cpu.set_zn(cpu.x);
            2
        }
        // `TXS` is the one transfer that touches no flags: it is loading the
        // stack pointer, not moving a value about.
        0x9A => {
            cpu.s = cpu.x;
            2
        }

        // -- the stack -----------------------------------------------------
        0x48 => {
            let v = cpu.a;
            cpu.push(bus, v);
            3
        }
        0x08 => {
            let v = cpu.p | flags::BREAK | flags::UNUSED;
            cpu.push(bus, v);
            3
        }
        0x68 => {
            cpu.a = cpu.pull(bus);
            cpu.set_zn(cpu.a);
            4
        }
        0x28 => {
            // Bit 5 always reads as one and bit 4 is not a register bit at
            // all, so neither survives being pulled.
            let v = cpu.pull(bus);
            cpu.p = (v | flags::UNUSED) & !flags::BREAK;
            4
        }

        // -- arithmetic ----------------------------------------------------
        0x69 => {
            let v = imm(cpu, bus);
            adc(cpu, v);
            2
        }
        0x65 => {
            let a = zp(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            3
        }
        0x75 => {
            let a = zpx(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            4
        }
        0x6D => {
            let a = abs(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            4
        }
        0x7D => {
            let (a, x) = absx(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            4 + x
        }
        0x79 => {
            let (a, x) = absy(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            4 + x
        }
        0x61 => {
            let a = indx(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            6
        }
        0x71 => {
            let (a, x) = indy(cpu, bus);
            let v = bus.read(a);
            adc(cpu, v);
            5 + x
        }

        0xE9 => {
            let v = imm(cpu, bus);
            sbc(cpu, v);
            2
        }
        0xE5 => {
            let a = zp(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            3
        }
        0xF5 => {
            let a = zpx(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            4
        }
        0xED => {
            let a = abs(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            4
        }
        0xFD => {
            let (a, x) = absx(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            4 + x
        }
        0xF9 => {
            let (a, x) = absy(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            4 + x
        }
        0xE1 => {
            let a = indx(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            6
        }
        0xF1 => {
            let (a, x) = indy(cpu, bus);
            let v = bus.read(a);
            sbc(cpu, v);
            5 + x
        }

        // -- the logic ops -------------------------------------------------
        0x29 => {
            let v = imm(cpu, bus);
            cpu.a &= v;
            cpu.set_zn(cpu.a);
            2
        }
        0x25 => {
            let a = zp(cpu, bus);
            and(cpu, bus, a);
            3
        }
        0x35 => {
            let a = zpx(cpu, bus);
            and(cpu, bus, a);
            4
        }
        0x2D => {
            let a = abs(cpu, bus);
            and(cpu, bus, a);
            4
        }
        0x3D => {
            let (a, x) = absx(cpu, bus);
            and(cpu, bus, a);
            4 + x
        }
        0x39 => {
            let (a, x) = absy(cpu, bus);
            and(cpu, bus, a);
            4 + x
        }
        0x21 => {
            let a = indx(cpu, bus);
            and(cpu, bus, a);
            6
        }
        0x31 => {
            let (a, x) = indy(cpu, bus);
            and(cpu, bus, a);
            5 + x
        }

        0x09 => {
            let v = imm(cpu, bus);
            cpu.a |= v;
            cpu.set_zn(cpu.a);
            2
        }
        0x05 => {
            let a = zp(cpu, bus);
            ora(cpu, bus, a);
            3
        }
        0x15 => {
            let a = zpx(cpu, bus);
            ora(cpu, bus, a);
            4
        }
        0x0D => {
            let a = abs(cpu, bus);
            ora(cpu, bus, a);
            4
        }
        0x1D => {
            let (a, x) = absx(cpu, bus);
            ora(cpu, bus, a);
            4 + x
        }
        0x19 => {
            let (a, x) = absy(cpu, bus);
            ora(cpu, bus, a);
            4 + x
        }
        0x01 => {
            let a = indx(cpu, bus);
            ora(cpu, bus, a);
            6
        }
        0x11 => {
            let (a, x) = indy(cpu, bus);
            ora(cpu, bus, a);
            5 + x
        }

        0x49 => {
            let v = imm(cpu, bus);
            cpu.a ^= v;
            cpu.set_zn(cpu.a);
            2
        }
        0x45 => {
            let a = zp(cpu, bus);
            eor(cpu, bus, a);
            3
        }
        0x55 => {
            let a = zpx(cpu, bus);
            eor(cpu, bus, a);
            4
        }
        0x4D => {
            let a = abs(cpu, bus);
            eor(cpu, bus, a);
            4
        }
        0x5D => {
            let (a, x) = absx(cpu, bus);
            eor(cpu, bus, a);
            4 + x
        }
        0x59 => {
            let (a, x) = absy(cpu, bus);
            eor(cpu, bus, a);
            4 + x
        }
        0x41 => {
            let a = indx(cpu, bus);
            eor(cpu, bus, a);
            6
        }
        0x51 => {
            let (a, x) = indy(cpu, bus);
            eor(cpu, bus, a);
            5 + x
        }

        // `BIT` is the odd one: the accumulator decides only the zero flag,
        // and N and V come straight off the memory byte's top two bits
        // without the accumulator being involved at all.
        0x24 => {
            let a = zp(cpu, bus);
            bit(cpu, bus, a);
            3
        }
        0x2C => {
            let a = abs(cpu, bus);
            bit(cpu, bus, a);
            4
        }

        // -- comparisons ---------------------------------------------------
        0xC9 => {
            let v = imm(cpu, bus);
            compare(cpu, cpu.a, v);
            2
        }
        0xC5 => {
            let a = zp(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            3
        }
        0xD5 => {
            let a = zpx(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            4
        }
        0xCD => {
            let a = abs(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            4
        }
        0xDD => {
            let (a, x) = absx(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            4 + x
        }
        0xD9 => {
            let (a, x) = absy(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            4 + x
        }
        0xC1 => {
            let a = indx(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            6
        }
        0xD1 => {
            let (a, x) = indy(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.a, v);
            5 + x
        }

        0xE0 => {
            let v = imm(cpu, bus);
            compare(cpu, cpu.x, v);
            2
        }
        0xE4 => {
            let a = zp(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.x, v);
            3
        }
        0xEC => {
            let a = abs(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.x, v);
            4
        }

        0xC0 => {
            let v = imm(cpu, bus);
            compare(cpu, cpu.y, v);
            2
        }
        0xC4 => {
            let a = zp(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.y, v);
            3
        }
        0xCC => {
            let a = abs(cpu, bus);
            let v = bus.read(a);
            compare(cpu, cpu.y, v);
            4
        }

        // -- up and down ---------------------------------------------------
        0xE6 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, inc_value);
            5
        }
        0xF6 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, inc_value);
            6
        }
        0xEE => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, inc_value);
            6
        }
        0xFE => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, inc_value);
            7
        }

        0xC6 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, dec_value);
            5
        }
        0xD6 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, dec_value);
            6
        }
        0xCE => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, dec_value);
            6
        }
        0xDE => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, dec_value);
            7
        }

        0xE8 => {
            cpu.x = cpu.x.wrapping_add(1);
            cpu.set_zn(cpu.x);
            2
        }
        0xC8 => {
            cpu.y = cpu.y.wrapping_add(1);
            cpu.set_zn(cpu.y);
            2
        }
        0xCA => {
            cpu.x = cpu.x.wrapping_sub(1);
            cpu.set_zn(cpu.x);
            2
        }
        0x88 => {
            cpu.y = cpu.y.wrapping_sub(1);
            cpu.set_zn(cpu.y);
            2
        }

        // -- shifts and rotates --------------------------------------------
        0x0A => {
            cpu.a = asl_value(cpu, cpu.a);
            2
        }
        0x06 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, asl_value);
            5
        }
        0x16 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, asl_value);
            6
        }
        0x0E => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, asl_value);
            6
        }
        0x1E => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, asl_value);
            7
        }

        0x4A => {
            cpu.a = lsr_value(cpu, cpu.a);
            2
        }
        0x46 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, lsr_value);
            5
        }
        0x56 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, lsr_value);
            6
        }
        0x4E => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, lsr_value);
            6
        }
        0x5E => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, lsr_value);
            7
        }

        0x2A => {
            cpu.a = rol_value(cpu, cpu.a);
            2
        }
        0x26 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, rol_value);
            5
        }
        0x36 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, rol_value);
            6
        }
        0x2E => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, rol_value);
            6
        }
        0x3E => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, rol_value);
            7
        }

        0x6A => {
            cpu.a = ror_value(cpu, cpu.a);
            2
        }
        0x66 => {
            let a = zp(cpu, bus);
            rmw(cpu, bus, a, ror_value);
            5
        }
        0x76 => {
            let a = zpx(cpu, bus);
            rmw(cpu, bus, a, ror_value);
            6
        }
        0x6E => {
            let a = abs(cpu, bus);
            rmw(cpu, bus, a, ror_value);
            6
        }
        0x7E => {
            let (a, _) = absx(cpu, bus);
            rmw(cpu, bus, a, ror_value);
            7
        }

        // -- jumps and calls -----------------------------------------------
        0x4C => {
            cpu.pc = abs(cpu, bus);
            3
        }
        0x6C => {
            let ptr = abs(cpu, bus);
            // The hardware bug, kept on purpose: the high byte is fetched
            // from the same page, so `JMP ($10FF)` reads `$10FF` and `$1000`.
            // ROMs of this age were tested against a chip that did this.
            let lo = bus.read(ptr);
            let hi = bus.read((ptr & 0xFF00) | u16::from((ptr as u8).wrapping_add(1)));
            cpu.pc = u16::from_le_bytes([lo, hi]);
            5
        }
        0x20 => {
            // The pushed address is the *last byte of the instruction*, not
            // the next one — `RTS` adds the one back. Off by one either way
            // and every call returns into the middle of itself.
            let target = abs(cpu, bus);
            let ret = cpu.pc.wrapping_sub(1);
            cpu.push(bus, (ret >> 8) as u8);
            cpu.push(bus, ret as u8);
            cpu.pc = target;
            6
        }
        0x60 => {
            let lo = cpu.pull(bus);
            let hi = cpu.pull(bus);
            cpu.pc = u16::from_le_bytes([lo, hi]).wrapping_add(1);
            6
        }
        0x40 => {
            let p = cpu.pull(bus);
            cpu.p = (p | flags::UNUSED) & !flags::BREAK;
            let lo = cpu.pull(bus);
            let hi = cpu.pull(bus);
            // No `+1` here: an interrupt pushed the address to come back to,
            // not the one before it.
            cpu.pc = u16::from_le_bytes([lo, hi]);
            6
        }
        0x00 => {
            // `BRK` is two bytes: the operand is skipped and never read,
            // which is what lets a handler pick up a code after it.
            cpu.pc = cpu.pc.wrapping_add(1);
            let pc = cpu.pc;
            cpu.push(bus, (pc >> 8) as u8);
            cpu.push(bus, pc as u8);
            cpu.push(bus, cpu.p | flags::BREAK | flags::UNUSED);
            cpu.p |= flags::IRQ_DISABLE;
            cpu.pc = read_u16(bus, crate::vectors::IRQ);
            7
        }

        // -- branches ------------------------------------------------------
        0x10 => branch(cpu, bus, !cpu.flag(flags::NEGATIVE)),
        0x30 => branch(cpu, bus, cpu.flag(flags::NEGATIVE)),
        0x50 => branch(cpu, bus, !cpu.flag(flags::OVERFLOW)),
        0x70 => branch(cpu, bus, cpu.flag(flags::OVERFLOW)),
        0x90 => branch(cpu, bus, !cpu.flag(flags::CARRY)),
        0xB0 => branch(cpu, bus, cpu.flag(flags::CARRY)),
        0xD0 => branch(cpu, bus, !cpu.flag(flags::ZERO)),
        0xF0 => branch(cpu, bus, cpu.flag(flags::ZERO)),

        // -- the flags -----------------------------------------------------
        0x18 => {
            cpu.set_flag(flags::CARRY, false);
            2
        }
        0x38 => {
            cpu.set_flag(flags::CARRY, true);
            2
        }
        0x58 => {
            cpu.set_flag(flags::IRQ_DISABLE, false);
            2
        }
        0x78 => {
            cpu.set_flag(flags::IRQ_DISABLE, true);
            2
        }
        0xB8 => {
            cpu.set_flag(flags::OVERFLOW, false);
            2
        }
        0xD8 => {
            cpu.set_flag(flags::DECIMAL, false);
            2
        }
        0xF8 => {
            cpu.set_flag(flags::DECIMAL, true);
            2
        }

        0xEA => 2,

        // Anything else is undocumented. Recorded rather than guessed at: a
        // ROM assembled against Rockwell's own tools has no reason to contain
        // one, so if this fires the interesting question is what put it there.
        _ => {
            cpu.last_illegal = Some((pc, op));
            2
        }
    }
}

// -- fetching and addressing ------------------------------------------------

fn fetch(cpu: &mut Cpu, bus: &mut impl Bus) -> u8 {
    let v = bus.read(cpu.pc);
    cpu.pc = cpu.pc.wrapping_add(1);
    v
}

fn imm(cpu: &mut Cpu, bus: &mut impl Bus) -> u8 {
    fetch(cpu, bus)
}

fn zp(cpu: &mut Cpu, bus: &mut impl Bus) -> u16 {
    u16::from(fetch(cpu, bus))
}

/// Zero page indexed, and the index **wraps inside the page**: `$FF + 2` is
/// `$01`, not `$0101`.
fn zpx(cpu: &mut Cpu, bus: &mut impl Bus) -> u16 {
    let base = fetch(cpu, bus);
    u16::from(base.wrapping_add(cpu.x))
}

fn zpy(cpu: &mut Cpu, bus: &mut impl Bus) -> u16 {
    let base = fetch(cpu, bus);
    u16::from(base.wrapping_add(cpu.y))
}

fn abs(cpu: &mut Cpu, bus: &mut impl Bus) -> u16 {
    let lo = fetch(cpu, bus);
    let hi = fetch(cpu, bus);
    u16::from_le_bytes([lo, hi])
}

/// Absolute indexed by X, with whether the index crossed a page.
fn absx(cpu: &mut Cpu, bus: &mut impl Bus) -> (u16, u32) {
    let base = abs(cpu, bus);
    let addr = base.wrapping_add(u16::from(cpu.x));
    (addr, u32::from(base & 0xFF00 != addr & 0xFF00))
}

fn absy(cpu: &mut Cpu, bus: &mut impl Bus) -> (u16, u32) {
    let base = abs(cpu, bus);
    let addr = base.wrapping_add(u16::from(cpu.y));
    (addr, u32::from(base & 0xFF00 != addr & 0xFF00))
}

/// `(zp,X)`: the index is added *before* the fetch, and both bytes of the
/// pointer come from the zero page.
fn indx(cpu: &mut Cpu, bus: &mut impl Bus) -> u16 {
    let base = fetch(cpu, bus).wrapping_add(cpu.x);
    let lo = bus.read(u16::from(base));
    let hi = bus.read(u16::from(base.wrapping_add(1)));
    u16::from_le_bytes([lo, hi])
}

/// `(zp),Y`: the pointer is fetched first — from the zero page, wrapping —
/// and the index is added to what it points at.
fn indy(cpu: &mut Cpu, bus: &mut impl Bus) -> (u16, u32) {
    let ptr = fetch(cpu, bus);
    let lo = bus.read(u16::from(ptr));
    let hi = bus.read(u16::from(ptr.wrapping_add(1)));
    let base = u16::from_le_bytes([lo, hi]);
    let addr = base.wrapping_add(u16::from(cpu.y));
    (addr, u32::from(base & 0xFF00 != addr & 0xFF00))
}

// -- the operations ---------------------------------------------------------

fn load_a(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.a = bus.read(addr);
    cpu.set_zn(cpu.a);
}

fn load_x(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.x = bus.read(addr);
    cpu.set_zn(cpu.x);
}

fn load_y(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.y = bus.read(addr);
    cpu.set_zn(cpu.y);
}

fn and(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.a &= bus.read(addr);
    cpu.set_zn(cpu.a);
}

fn ora(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.a |= bus.read(addr);
    cpu.set_zn(cpu.a);
}

fn eor(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    cpu.a ^= bus.read(addr);
    cpu.set_zn(cpu.a);
}

fn bit(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16) {
    let v = bus.read(addr);
    cpu.set_flag(flags::ZERO, v & cpu.a == 0);
    cpu.set_flag(flags::NEGATIVE, v & 0x80 != 0);
    cpu.set_flag(flags::OVERFLOW, v & 0x40 != 0);
}

/// A comparison is a subtraction that keeps only the flags, and the carry is
/// "no borrow": set when the register is the larger.
fn compare(cpu: &mut Cpu, reg: u8, value: u8) {
    let diff = reg.wrapping_sub(value);
    cpu.set_flag(flags::CARRY, reg >= value);
    cpu.set_zn(diff);
}

/// Read, change, write back. The chip writes the *old* value first and then
/// the new one — a detail that matters only where a write has a side effect,
/// which on this machine is nowhere, so only the new value is written.
fn rmw(cpu: &mut Cpu, bus: &mut impl Bus, addr: u16, op: fn(&mut Cpu, u8) -> u8) {
    let v = bus.read(addr);
    let out = op(cpu, v);
    bus.write(addr, out);
}

fn inc_value(cpu: &mut Cpu, v: u8) -> u8 {
    let out = v.wrapping_add(1);
    cpu.set_zn(out);
    out
}

fn dec_value(cpu: &mut Cpu, v: u8) -> u8 {
    let out = v.wrapping_sub(1);
    cpu.set_zn(out);
    out
}

fn asl_value(cpu: &mut Cpu, v: u8) -> u8 {
    cpu.set_flag(flags::CARRY, v & 0x80 != 0);
    let out = v << 1;
    cpu.set_zn(out);
    out
}

fn lsr_value(cpu: &mut Cpu, v: u8) -> u8 {
    cpu.set_flag(flags::CARRY, v & 0x01 != 0);
    let out = v >> 1;
    cpu.set_zn(out);
    out
}

fn rol_value(cpu: &mut Cpu, v: u8) -> u8 {
    let carry = u8::from(cpu.flag(flags::CARRY));
    cpu.set_flag(flags::CARRY, v & 0x80 != 0);
    let out = (v << 1) | carry;
    cpu.set_zn(out);
    out
}

fn ror_value(cpu: &mut Cpu, v: u8) -> u8 {
    let carry = u8::from(cpu.flag(flags::CARRY)) << 7;
    cpu.set_flag(flags::CARRY, v & 0x01 != 0);
    let out = (v >> 1) | carry;
    cpu.set_zn(out);
    out
}

/// Add with carry, in binary or in BCD.
///
/// Decimal mode is not a curiosity here: a pinball score is a string of BCD
/// digits and every machine of this era adds to it with `SED`/`ADC`/`CLD`.
/// The 6502's decimal adjust is per nibble — add six to a digit that came out
/// above nine, and carry into the next — and the *flags* it leaves are the
/// binary ones, which is a documented oddity and what ROMs of the period were
/// written against.
fn adc(cpu: &mut Cpu, value: u8) {
    let carry = u16::from(cpu.flag(flags::CARRY));
    let a = u16::from(cpu.a);
    let v = u16::from(value);
    let binary = a + v + carry;

    // Overflow is about signs: it is set when two operands of the same sign
    // produce a result of the other one.
    let overflow = (!(a ^ v) & (a ^ binary) & 0x80) != 0;
    cpu.set_flag(flags::OVERFLOW, overflow);
    // Zero comes from the binary sum even in decimal mode.
    cpu.set_flag(flags::ZERO, binary as u8 == 0);

    if cpu.flag(flags::DECIMAL) {
        let mut lo = (a & 0x0F) + (v & 0x0F) + carry;
        if lo > 9 {
            lo += 6;
        }
        let mut hi = (a >> 4) + (v >> 4) + u16::from(lo > 0x0F);
        cpu.set_flag(flags::NEGATIVE, (hi << 4) as u8 & 0x80 != 0);
        if hi > 9 {
            hi += 6;
        }
        cpu.set_flag(flags::CARRY, hi > 0x0F);
        cpu.a = ((hi << 4) | (lo & 0x0F)) as u8;
    } else {
        cpu.set_flag(flags::CARRY, binary > 0xFF);
        cpu.a = binary as u8;
        cpu.set_flag(flags::NEGATIVE, cpu.a & 0x80 != 0);
    }
}

/// Subtract with borrow. The carry is the *inverse* of a borrow, so a
/// subtraction is preceded by `SEC` and not by `CLC`.
fn sbc(cpu: &mut Cpu, value: u8) {
    let carry = u16::from(cpu.flag(flags::CARRY));
    let a = u16::from(cpu.a);
    let v = u16::from(value);
    let binary = a.wrapping_sub(v).wrapping_sub(1 - carry);

    let overflow = ((a ^ v) & (a ^ binary) & 0x80) != 0;
    cpu.set_flag(flags::OVERFLOW, overflow);
    cpu.set_flag(flags::CARRY, binary < 0x100);
    cpu.set_flag(flags::ZERO, binary as u8 == 0);
    cpu.set_flag(flags::NEGATIVE, binary as u8 & 0x80 != 0);

    if cpu.flag(flags::DECIMAL) {
        let mut lo = (a & 0x0F).wrapping_sub(v & 0x0F).wrapping_sub(1 - carry);
        let mut hi = (a >> 4).wrapping_sub(v >> 4);
        if lo & 0x10 != 0 {
            lo = lo.wrapping_sub(6);
            hi = hi.wrapping_sub(1);
        }
        if hi & 0x10 != 0 {
            hi = hi.wrapping_sub(6);
        }
        cpu.a = (((hi & 0x0F) << 4) | (lo & 0x0F)) as u8;
    } else {
        cpu.a = binary as u8;
    }
}

/// A branch: two cycles, three if taken, four if it also lands on another
/// page. The offset is signed and measured from the instruction after it.
fn branch(cpu: &mut Cpu, bus: &mut impl Bus, take: bool) -> u32 {
    let offset = fetch(cpu, bus) as i8;
    if !take {
        return 2;
    }
    let from = cpu.pc;
    let to = from.wrapping_add_signed(i16::from(offset));
    cpu.pc = to;
    3 + u32::from(from & 0xFF00 != to & 0xFF00)
}
