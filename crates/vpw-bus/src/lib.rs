//! The memory bus as seen by the 8-bit CPUs.
//!
//! It lives in its own crate because a real machine carries several CPUs with
//! different memory maps —a System 11 has a main 6808, a second 6808 for sound
//! and a 6809 on the CS board— and they all talk to memory the same way.

/// Everything the 6800 can address: a flat 64 KB.
///
/// The CPU knows nothing about ROM, RAM or the PIAs: every access goes through
/// here and it is the machine that decides what answers. `read` takes
/// `&mut self` on purpose, because reading a hardware register usually has
/// side effects (for example, reading a 6821's port clears its interrupt
/// flag).
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);

    /// 16-bit read, big-endian as everywhere in the 68xx family.
    fn read_u16(&mut self, addr: u16) -> u16 {
        let hi = self.read(addr);
        let lo = self.read(addr.wrapping_add(1));
        u16::from_be_bytes([hi, lo])
    }
}

/// 64 KB of flat RAM. For tests and for booting without a complete machine.
pub struct FlatMemory {
    pub bytes: Box<[u8; 0x1_0000]>,
}

impl Default for FlatMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl FlatMemory {
    pub fn new() -> Self {
        Self {
            bytes: Box::new([0; 0x1_0000]),
        }
    }

    /// Copies `data` starting at `addr`.
    pub fn load(&mut self, addr: u16, data: &[u8]) {
        let start = addr as usize;
        self.bytes[start..start + data.len()].copy_from_slice(data);
    }

    /// Writes an exception vector (RESET, IRQ, NMI, SWI).
    pub fn set_vector(&mut self, vector: u16, target: u16) {
        let [hi, lo] = target.to_be_bytes();
        self.bytes[vector as usize] = hi;
        self.bytes[vector as usize + 1] = lo;
    }
}

impl Bus for FlatMemory {
    fn read(&mut self, addr: u16) -> u8 {
        self.bytes[addr as usize]
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.bytes[addr as usize] = value;
    }
}
