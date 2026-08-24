//! Thumb state: the sixteen-bit instruction set.
//!
//! Not yet. The board's firmware is compiled for ARM state, and this exists so
//! that a `BX` into Thumb fails loudly rather than executing ARM words as if
//! nothing had happened — which is the failure that looks like a corrupt ROM.

use super::{Bus, Cpu};

pub(super) fn step(cpu: &mut Cpu, _bus: &mut impl Bus) -> u32 {
    panic!(
        "the processor entered Thumb state at {:08x}, which this core does not implement yet",
        cpu.r[15]
    );
}
