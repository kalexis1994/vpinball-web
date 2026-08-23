//! Dumps the cycles each opcode consumes, one per line, in hexadecimal.
//!
//! Useful for checking the timing table against an external reference. The
//! 6800's timings are fixed per opcode (no instruction changes duration
//! depending on the flags), so running each one once is enough.
//!
//!     cargo run -p vpw-m6800 --example cycles

use vpw_m6800::{Cpu, FlatMemory, vectors};

fn main() {
    for opcode in 0u16..=0xFF {
        let opcode = opcode as u8;

        let mut mem = FlatMemory::new();
        // Neutral operands behind the opcode and vectors pointing somewhere,
        // so that the jump instructions do not read garbage.
        mem.load(0x0100, &[opcode, 0x00, 0x00]);
        mem.set_vector(vectors::RESET, 0x0100);
        mem.set_vector(vectors::SWI, 0x9000);
        mem.set_vector(vectors::IRQ, 0x9000);
        mem.set_vector(vectors::NMI, 0x9000);

        let mut cpu = Cpu::new();
        cpu.reset(&mut mem);
        cpu.sp = 0x00FF;

        let cycles = cpu.step(&mut mem);
        let illegal = cpu.last_illegal.is_some();

        println!(
            "{opcode:02X} {}",
            if illegal {
                "XX".to_string()
            } else {
                cycles.to_string()
            }
        );
    }
}
