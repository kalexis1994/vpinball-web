//! Dumps the cycles of every opcode of the main page.
//!
//! It is there to check the timing table against an external reference. For
//! the indexed opcodes it uses the `,R` postbyte, which is the only one that
//! adds no cycles, so what comes out is the base cost.
//!
//!     cargo run -p vpw-m6809 --example cycles

use vpw_m6809::{Cpu, FlatMemory, vectors};

/// `,R` postbyte on X: zero extra cycles.
const POSTBYTE_ZERO_OFFSET: u8 = 0b1000_0100;

fn main() {
    for opcode in 0u16..=0xFF {
        let opcode = opcode as u8;

        // The byte that follows the opcode depends on what the instruction
        // expects.
        let operand = match opcode {
            // Indexed: postbyte with no extra cost.
            0x30..=0x33 | 0x60..=0x6F | 0xA0..=0xAF | 0xE0..=0xEF => POSTBYTE_ZERO_OFFSET,
            // PSHS/PULS/PSHU/PULU: empty map, to measure only the base cost.
            0x34..=0x37 => 0x00,
            _ => 0x00,
        };

        let mut mem = FlatMemory::new();
        mem.load(0x0100, &[opcode, operand, 0x00, 0x00]);
        mem.set_vector(vectors::RESET, 0x0100);
        for vector in [
            vectors::IRQ,
            vectors::FIRQ,
            vectors::NMI,
            vectors::SWI,
            vectors::SWI2,
            vectors::SWI3,
        ] {
            mem.set_vector(vector, 0x9000);
        }

        let mut cpu = Cpu::new();
        cpu.reset(&mut mem);
        cpu.s = 0x0200;
        cpu.u = 0x0300;

        let cycles = cpu.step(&mut mem);
        let illegal = cpu.last_illegal.is_some();

        println!(
            "{opcode:02X} {}",
            if illegal {
                "0".to_string()
            } else {
                cycles.to_string()
            }
        );
    }
}
