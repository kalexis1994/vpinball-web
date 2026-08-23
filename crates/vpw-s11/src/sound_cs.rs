//! Williams System 11 "CS" sound board.
//!
//! The second sound board, the one that does the music. It is bigger than the
//! XS: a **6809 at 2 MHz** instead of a 6808, a **YM2151** for FM synthesis,
//! and on top of that its own DAC and its own CVSD.
//!
//! F-14 Tomcat carries both boards: the XS for effects and speech, this one for
//! the music.
//!
//! # Memory map (`wmssnd.c:679-694`)
//!
//! | Range | What's there |
//! |-------|---------|
//! | `$0000`-`$1FFF` | RAM |
//! | `$2000` / `$2001` | YM2151: address and data. Reading the odd one gives the status |
//! | `$4000`-`$4003` | PIA |
//! | `$6000` | CVSD: data bit, and lowers the clock |
//! | `$6800` | CVSD: raises the clock, and that is when the bit goes in |
//! | `$7800` | ROM bank select |
//! | `$8000`-`$FFFF` | Banked ROM window, 32 KB |
//!
//! This board's CVSD is **not** driven through the PIA like the XS's: it goes
//! through writes to two different addresses, one that sets the bit and one
//! that shifts it in.
//!
//! # PIA wiring (`wmssnd.c:750-763`)
//!
//! - **PA (output):** the DAC.
//! - **PB:** the channel to the main board. Reading it brings the sound number;
//!   writing it sends data back.
//! - **CA1 (input):** the YM2151's interrupt, **inverted**. It is the time base
//!   of the music sequencer.
//! - **CB1 (input):** the main board's handshake.
//! - **CA2 (output):** YM2151 reset.
//!
//! And the PIA's two interrupt lines do not go together: the A side goes to the
//! 6809's **FIRQ** and the B side to the **NMI**.

use crate::levels;
use vpw_audio::{Cvsd, Dac, Ym2151};
use vpw_m6809::{Bus, Cpu};
use vpw_pia6821::Pia;

/// Clock of the board's 6809 (`wmssnd.c:738`).
pub const CS_CPU_CLOCK_HZ: u32 = 2_000_000;
/// YM2151 clock: NTSC's colour crystal.
pub const YM_CLOCK_HZ: u32 = 3_579_545;
/// The YM divides its clock by 64 to put out a sample.
pub const YM_SAMPLE_RATE: u32 = YM_CLOCK_HZ / 64;

const RAM_SIZE: usize = 0x2000;
/// The board's ROM region: seven 64 KB banks.
const ROM_REGION_SIZE: usize = 0x7_0000;
/// The `$8000` window sees 32 KB.
const WINDOW_SIZE: usize = 0x8000;

/// The CS board's bus.
pub struct CsSoundBus {
    ram: Box<[u8; RAM_SIZE]>,
    rom: Box<[u8; ROM_REGION_SIZE]>,
    /// Offset within the region that the `$8000` window sees.
    window: usize,

    pub pia: Pia,
    pub dac: Dac,
    pub cvsd: Cvsd,
    pub ym: Ym2151,

    /// Bit the CVSD is waiting for, set by the write to `$6000`.
    cvsd_bit: bool,
    /// Level of the CVSD's clock.
    cvsd_clock: bool,
    /// Bits that went into the CVSD since reset.
    cvsd_bits: u64,
    /// DAC level changes since reset.
    dac_writes: u64,

    /// Last sample the YM2151 generated. It is held between generations, just
    /// like a DAC's level.
    ym_sample: f32,

    /// Sound number the main board left behind.
    sound_latch: u8,
    /// What this board answers back to the main one, over that same port B.
    reply: u8,

    pub last_unmapped: Option<u16>,
}

impl CsSoundBus {
    pub fn new() -> Self {
        Self {
            ram: Box::new([0; RAM_SIZE]),
            rom: Box::new([0; ROM_REGION_SIZE]),
            window: 0,
            pia: Pia::new(),
            dac: Dac::new(),
            cvsd: Cvsd::new(),
            ym: Ym2151::new(YM_CLOCK_HZ, YM_SAMPLE_RATE),
            cvsd_bit: false,
            cvsd_clock: false,
            cvsd_bits: 0,
            dac_writes: 0,
            ym_sample: 0.0,
            sound_latch: 0,
            reply: 0,
            last_unmapped: None,
        }
    }

    /// Loads the board's sound images.
    ///
    /// Each 32 KB image is mirrored four times inside the region, which is what
    /// `S11CS_ROMLOAD8` (`wmssnd.h:79`) does: the same content ends up visible
    /// from several banks.
    pub fn load_roms(&mut self, images: &[&[u8]]) -> bool {
        if images.iter().any(|img| img.len() != 0x8000) {
            return false;
        }
        for (slot, image) in images.iter().enumerate() {
            let start = slot * 0x1_0000;
            for mirror in [0x0000, 0x8000, 0x4_0000, 0x4_8000] {
                let at = start + mirror;
                if at + 0x8000 <= ROM_REGION_SIZE {
                    self.rom[at..at + 0x8000].copy_from_slice(image);
                }
            }
        }
        true
    }

    /// Write to `$7800`. Bits 0, 1 and 3 pick the 64 KB bank and bit 2 the
    /// 32 KB half.
    fn select_bank(&mut self, value: u8) {
        let chip = usize::from(value & 0x03) + (usize::from(value & 0x08) >> 1);
        let half = usize::from(value & 0x04) >> 2;
        self.window = (chip * 0x1_0000 + half * WINDOW_SIZE).min(ROM_REGION_SIZE - WINDOW_SIZE);
    }

    /// Sound number arriving from the main board.
    pub fn set_sound_latch(&mut self, value: u8) {
        self.sound_latch = value;
        self.pia.set_port_b_input(value);
    }

    /// The main board's handshake comes in on CB1.
    pub fn set_handshake(&mut self, level: bool) {
        self.pia.set_cb1(level);
    }

    fn service_audio(&mut self) {
        let level = self.pia.port_a_output();
        if level != self.dac.level() {
            self.dac_writes += 1;
        }
        self.dac.write(level);
        self.reply = self.pia.port_b_output();
    }

    /// Bits that went into the CVSD since reset.
    pub fn cvsd_bits(&self) -> u64 {
        self.cvsd_bits
    }

    /// DAC level changes since reset.
    pub fn dac_writes(&self) -> u64 {
        self.dac_writes
    }

    /// What the board is answering back to the main one.
    ///
    /// Port B is bidirectional: the main board leaves the sound number there
    /// and the board answers over the same path.
    pub fn reply(&self) -> u8 {
        self.reply
    }

    /// Notification from the board to the main one, on CB2.
    pub fn reply_strobe(&self) -> bool {
        self.pia.cb2_output()
    }

    /// Generates a YM2151 sample and makes it available.
    fn tick_ym(&mut self) {
        self.ym_sample = self.ym.next_sample_mono();
    }

    /// Audio sample from the board: the speech, the effects and the FM, each at
    /// the level the machine driver gives it. See [`crate::levels`].
    pub fn sample(&self) -> f32 {
        levels::weighted(&self.sources())
    }

    /// The board's sources with their levels, for a caller that has to mix them
    /// alongside the main board's rather than after them. See [`crate::levels`]
    /// for why that distinction is not a detail.
    pub fn sources(&self) -> [(f32, f32); 3] {
        [
            (levels::CVSD, self.cvsd.sample()),
            (levels::DAC, self.dac.sample()),
            (levels::YM2151, self.ym_sample),
        ]
    }
}

impl Default for CsSoundBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for CsSoundBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize],
            // The YM takes up the whole $2000 window by repeating itself; only
            // the parity of the address matters.
            0x2000..=0x2FFF => {
                if addr & 1 != 0 {
                    self.ym.read_status()
                } else {
                    0xFF
                }
            }
            0x4000..=0x4FFF => {
                let value = self.pia.read((addr & 3) as u8);
                self.service_audio();
                value
            }
            0x8000..=0xFFFF => self.rom[self.window + (addr as usize - 0x8000)],
            _ => {
                self.last_unmapped = Some(addr);
                0xFF
            }
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize] = value,
            0x2000..=0x2FFF => {
                if addr & 1 != 0 {
                    self.ym.write_data(value);
                } else {
                    self.ym.write_address(value);
                }
            }
            0x4000..=0x4FFF => {
                self.pia.write((addr & 3) as u8, value);
                self.service_audio();
            }
            // Sets the bit and lowers the clock.
            0x6000..=0x67FF => {
                self.cvsd_bit = value & 1 != 0;
                self.cvsd_clock = false;
            }
            // Raises the clock: that is when the bit goes in.
            0x6800..=0x6FFF => {
                if !self.cvsd_clock {
                    self.cvsd.clock(self.cvsd_bit);
                    self.cvsd_bits += 1;
                }
                self.cvsd_clock = true;
            }
            0x7800..=0x7FFF => self.select_bank(value),
            // Writing to the ROM does nothing.
            0x8000..=0xFFFF => {}
            _ => self.last_unmapped = Some(addr),
        }
    }
}

/// The complete CS board.
pub struct CsSoundBoard {
    pub cpu: Cpu,
    pub bus: CsSoundBus,
    cycle: u64,
    /// Accumulated remainder used to decide when to generate a YM sample.
    ym_accumulator: f64,
    cycles_per_ym_sample: f64,
}

impl Default for CsSoundBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl CsSoundBoard {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            bus: CsSoundBus::new(),
            cycle: 0,
            ym_accumulator: 0.0,
            cycles_per_ym_sample: f64::from(CS_CPU_CLOCK_HZ) / f64::from(YM_SAMPLE_RATE),
        }
    }

    /// Loads the sound images. For F-14 they are `f14_u4.l1` and `f14_u19.l1`.
    pub fn load_roms(&mut self, images: &[&[u8]]) -> bool {
        self.bus.load_roms(images)
    }

    pub fn reset(&mut self) {
        self.bus.pia.reset();
        self.bus.dac.reset();
        self.bus.cvsd.reset();
        self.bus.ym.reset();
        self.bus.cvsd_bits = 0;
        self.bus.dac_writes = 0;
        self.bus.window = 0;
        self.cycle = 0;
        self.ym_accumulator = 0.0;
        self.cpu.reset(&mut self.bus);
    }

    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// Executes one 6809 instruction.
    pub fn step(&mut self) -> u32 {
        // This board's interrupt wiring is not the obvious one
        // (`wmssnd.c:782-787`):
        //
        // - The YM2151's interrupt comes in **inverted** on CA1.
        // - The PIA's A side goes to the **FIRQ**, the fast interrupt.
        // - The B side goes to the **NMI**, and that is the path the sound
        //   numbers from the main board arrive on.
        //
        // Using the plain IRQ for both leaves the board mute: the commands are
        // never serviced.
        let ym_irq = self.bus.ym.irq();
        self.bus.pia.set_ca1(!ym_irq);
        self.cpu.set_firq(self.bus.pia.irq_a());
        self.cpu.set_nmi(self.bus.pia.irq_b());

        let cycles = self.cpu.step(&mut self.bus);
        self.cycle += u64::from(cycles);

        // Generate the YM samples that belong to those cycles.
        self.ym_accumulator += f64::from(cycles);
        while self.ym_accumulator >= self.cycles_per_ym_sample {
            self.ym_accumulator -= self.cycles_per_ym_sample;
            self.bus.tick_ym();
        }

        cycles
    }

    /// Runs until reaching `target` cycles since reset.
    ///
    /// Absolute target, not an increment: see the note in
    /// [`crate::SoundBoard`].
    pub fn run_until(&mut self, target: u64) -> u64 {
        while self.cycle < target {
            self.step();
        }
        self.cycle
    }

    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        self.run_until(self.cycle + cycles)
    }

    /// Sends it a sound number: leaves the latch and pulses the handshake.
    pub fn send_command(&mut self, command: u8) {
        self.bus.set_sound_latch(command);
        self.bus.set_handshake(true);
        self.bus.set_handshake(false);
    }

    pub fn sample(&self) -> f32 {
        self.bus.sample()
    }

    /// The board's sources with their levels. See [`crate::levels`].
    pub fn sources(&self) -> [(f32, f32); 3] {
        self.bus.sources()
    }
}
