//! Stern's third-generation sound board, as an address space.
//!
//! The map is what the original declares (`desound.c:854`), and every line of
//! it is a fact about the hardware rather than a convenience:
//!
//! ```text
//! read
//!   00000000-000fffff  page 0: the BIOS at reset, the program after the remap
//!   00300000-003fffff  the RAM the program is built in before it is moved
//!   00400000-005fffff  the BIOS, where it stays
//!   10000000-101fffff  U8, two megabytes, on an eight-bit bus
//!   20000000-20000003  the command from the CPU board
//!   20000004-207fffff  U17, U21, U36 and U37, in two interleaved pairs
//!   40000000-4000ffff  U7, sixty-four kilobytes
//! write
//!   20400000-20400003  the samples, two channels in the two halves of a word
//! ```
//!
//! The sample parts are the part that is not obvious, and guessing at them is
//! how a board produces music with noise all over it: near enough to the right
//! bytes that a tune is recognisable, and wrong enough that nothing else is.
//! They are not four parts one after another and they are not four sockets in
//! a row. They are two pairs, wired the way byte-wide parts are wired to make
//! a sixteen-bit-wide store: **A0 chooses within a pair and A21 chooses the
//! pair**, so consecutive addresses alternate between two chips, and the
//! address a chip actually sees is the processor's with those two lines taken
//! out — which means shifted right by one. A29 is the chip select's own line
//! and A22 is not connected to anything.
//!
//! The order the four sit in is not the order they are named in either:
//! `desound.c:453` gives it as U37, U21, U36, U17 for selector zero to three.
//!
//! The eight-bit buses matter. U8 and the sample ROMs are byte-wide devices on
//! the external bus, so a word read of one is four accesses, and a core that
//! answers a whole word from a byte-wide device gives the firmware three bytes
//! of something else.

use vpw_arm7::Bus;

use crate::soc::{Aic, Pio, Timer, irq};

/// The processor's clock. `MDRV_CPU_ADD(AT91, 40000000)`.
pub const CLOCK_HZ: u32 = 40_000_000;

/// How big each part of the map is.
const PAGE0: usize = 0x10_0000;
const RESET_RAM: usize = 0x10_0000;
const BIOS: usize = 0x20_0000;
/// Where the sample parts answer, from `desound.c:860`.
const SAMPLES_TO: u32 = 0x207f_ffff;
const U7: usize = 0x1_0000;

/// How many sound commands the board holds. `desound.c:433`.
const COMMAND_QUEUE: usize = 4;

/// The sound board's memory and peripherals.
pub struct Sound {
    /// Page zero: the BIOS is here at reset and the program is here after.
    page0: Box<[u8; PAGE0]>,
    /// Where the program is assembled before the remap moves it down.
    reset_ram: Box<[u8; RESET_RAM]>,
    bios: Box<[u8; BIOS]>,
    /// The four sample parts, each answering across its own socket.
    samples: [Vec<u8>; 4],
    u7: Box<[u8; U7]>,

    pub aic: Aic,
    pub timers: [Timer; 3],
    pub pio: Pio,
    /// Which peripheral clocks are running. The timers only count while theirs
    /// is, which is how firmware stops them without losing their setup.
    pub power: u32,

    /// The commands the CPU board has posted and the firmware has not read.
    ///
    /// A queue and not a latch, and the reason is written on the reference's
    /// read handler: "Buffer the sound commands (to account for timing offsets
    /// between the 6809 Main CPU & the AT91 CPU)" (`desound.c:713`). The game's
    /// processor writes a burst of bytes microseconds apart while the firmware
    /// looks at the port on its own schedule, so a single byte loses every one
    /// but the last. Single-byte commands survive that — which is why the right
    /// samples play — and a command with a parameter after it does not: the
    /// firmware sees the parameter twice and never sees what it was for.
    ///
    /// Four deep, which is the reference's `ARMSNDBUFSIZE` and its comment that
    /// two "should actually be good enough, but lets play safe"
    /// (`desound.c:433`).
    pub command: u8,
    pub command_pending: bool,
    queue: [u8; COMMAND_QUEUE],
    write_at: usize,
    read_at: usize,
    last_written: u8,
    /// Commands actually taken off the queue, idle repeats not counted.
    pub commands_taken: u64,

    /// The two channels the firmware writes, kept apart because it writes them
    /// apart: one halfword each, to the two halves of the same word.
    left: Vec<i16>,
    right: Vec<i16>,

    /// Whether the remap has happened. Before it, page zero is the BIOS.
    pub remapped: bool,
    /// An address nothing answered, kept for whoever is debugging.
    pub last_unmapped: Option<(u32, bool)>,
    /// How many times each region has been read. Diagnostics: a firmware that
    /// never looks at the sample ROMs is not playing anything, whatever else
    /// it is doing.
    pub reads: [u64; 4],
    /// Cycles run but not yet given to the timers, how many more may pass
    /// before one of them could do something, and the interrupt line as it was
    /// when they were last brought up to date. See [`Sound::tick`].
    owed: u32,
    quiet_for: u32,
    irq_line: bool,
    /// The last address read that was not memory, for a host that is trying to
    /// work out what the firmware is waiting for. Costs nothing on the paths
    /// that matter because memory does not set it.
    pub last_read: Option<u32>,
}

impl Sound {
    pub fn new() -> Self {
        Self {
            page0: Box::new([0; PAGE0]),
            reset_ram: Box::new([0; RESET_RAM]),
            bios: Box::new([0; BIOS]),
            samples: [const { Vec::new() }; 4],
            u7: Box::new([0; U7]),
            aic: Aic::default(),
            timers: Default::default(),
            pio: Pio::default(),
            // What the part comes up with (`at91.c:1683`).
            power: 0x17c,
            command: 0,
            command_pending: false,
            queue: [0; COMMAND_QUEUE],
            write_at: 0,
            read_at: 0,
            last_written: 0,
            commands_taken: 0,
            left: Vec::new(),
            right: Vec::new(),
            remapped: false,
            last_unmapped: None,
            reads: [0; 4],
            owed: 0,
            quiet_for: 1,
            irq_line: false,
            last_read: None,
        }
    }

    /// Loads the two-megabyte BIOS, which is also what page zero starts as.
    ///
    /// The original loads it twice — once at the top of the region and once at
    /// zero — and the processor boots out of the copy at zero, builds its real
    /// program in the RAM at `$300000`, and asks the bus interface to swap the
    /// two over.
    pub fn load_bios(&mut self, image: &[u8]) {
        let n = image.len().min(BIOS);
        self.bios[..n].copy_from_slice(&image[..n]);
        let n = image.len().min(PAGE0);
        self.page0[..n].copy_from_slice(&image[..n]);
        // The window at `$300000` is the second megabyte of the same image.
        if image.len() > PAGE0 {
            let n = (image.len() - PAGE0).min(RESET_RAM);
            self.reset_ram[..n].copy_from_slice(&image[PAGE0..PAGE0 + n]);
        }
    }

    /// Loads the sixty-four kilobyte ROM at `$40000000`.
    pub fn load_u7(&mut self, image: &[u8]) {
        let n = image.len().min(U7);
        self.u7[..n].copy_from_slice(&image[..n]);
    }

    /// Loads one of the four sample ROMs, in order.
    pub fn load_samples(&mut self, index: usize, image: &[u8]) {
        if let Some(slot) = self.samples.get_mut(index) {
            *slot = image.to_vec();
        }
    }

    /// One byte from the sample sockets. See the note at the top of the file
    /// about why the address is folded twice: once to pick the socket, and
    /// again inside it because the part is smaller than its window.
    fn sample_byte(&self, addr: u32) -> u8 {
        // A29 is the chip select and A22 goes nowhere (`desound.c:556`).
        let a = addr & 0xdfbf_ffff;
        // Which of the four, from A21 and A0. The numbering is the board's,
        // not the parts' names: `rommap` in the original is `{4,2,3,1}`.
        const ORDER: [usize; 4] = [3, 1, 2, 0];
        let part = &self.samples[ORDER[(((a & 0x0020_0000) >> 20) | (a & 1)) as usize]];
        // And what is left of the address once those two lines are taken out.
        let at = ((a & !0x0020_0000) >> 1) as usize;
        match part.is_empty() {
            // An empty socket floats high, which is what a ROM test reads
            // when the part is not there.
            true => 0xff,
            false => part[at % part.len()],
        }
    }

    /// The CPU board has sent a sound command.
    ///
    /// Queued, and nothing else: the reference's write handler (`scmd_w`,
    /// `desound.c:694`) raises no interrupt — the firmware polls the port on
    /// its own schedule, and the only interrupt this board lives on is the
    /// sample timer's. The one thing the reference does beyond the queue is
    /// wake a host-suspended processor, which is MAME's scheduler and not the
    /// hardware.
    pub fn send(&mut self, command: u8) {
        // Only a change is queued (`desound.c:696`): the game holds the latch
        // between commands and re-posting the same byte is not a new one.
        if command == self.last_written {
            return;
        }
        self.last_written = command;
        self.queue[self.write_at] = command;
        self.write_at = (self.write_at + 1) % COMMAND_QUEUE;
        // A full queue drops the oldest rather than the newest, so a burst
        // longer than the queue loses its head and keeps its tail.
        if self.write_at == self.read_at {
            self.read_at = (self.read_at + 1) % COMMAND_QUEUE;
        }
        self.command = command;
        self.command_pending = true;
    }

    /// Takes the next command, or repeats the last one when there is none.
    ///
    /// Repeating rather than answering zero is the reference's behaviour
    /// (`desound.c:725`) and it is what makes a polling firmware work: it reads
    /// the port continuously and acts on a **change**, so an idle port has to
    /// keep saying the same thing.
    fn take_command(&mut self) -> u8 {
        let data = self.queue[self.read_at];
        if self.read_at != self.write_at {
            self.read_at = (self.read_at + 1) % COMMAND_QUEUE;
            // A dequeue, as opposed to the idle repeat below. The count is
            // for whoever is asking why a machine is quiet: a firmware that
            // is polling but never taking anything is a different fault from
            // one that is not polling at all.
            self.commands_taken += 1;
        }
        if self.read_at == self.write_at {
            self.queue[self.read_at] = data;
        }
        self.command = data;
        self.command_pending = false;
        data
    }

    /// Whether the fast interrupt line is down. A command does not pull it —
    /// the firmware polls for those, see [`Sound::send`] — so this only ever
    /// answers what the firmware has raised on itself through the controller.
    pub fn fiq(&self) -> bool {
        self.aic.fast_asserted()
    }

    /// The samples the firmware has produced since this was last asked.
    ///
    /// Paired up here rather than at the port, because the two channels are
    /// written as two separate halfword stores and can arrive in either order
    /// and in unequal numbers. Whichever is shorter decides how many pairs
    /// there are; the rest waits for its partner.
    pub fn take_audio(&mut self) -> Vec<(i16, i16)> {
        let n = self.left.len().min(self.right.len());
        let out: Vec<(i16, i16)> = self.left.drain(..n).zip(self.right.drain(..n)).collect();
        out
    }

    /// How many samples a second the firmware has set the board to make.
    ///
    /// Not a constant: the rate is whatever the firmware programmed into the
    /// timer whose interrupt fills the buffer, so it is asked for rather than
    /// assumed. Zero until it has done that.
    pub fn sample_rate(&self) -> f64 {
        self.timers[0].rate(CLOCK_HZ)
    }

    /// How many complete samples are waiting. The one number that says whether
    /// the firmware is doing its job.
    pub fn queued(&self) -> usize {
        self.left.len().min(self.right.len())
    }

    /// One channel's sample, as the firmware writes it.
    ///
    /// The board takes them a halfword at a time at `$20400000`: the upper
    /// half of the word is one channel and the lower half the other
    /// (`desound.c:571`). A core whose halfword store falls through to two
    /// byte stores loses every sample, silently, having run the firmware
    /// perfectly — which is exactly what happened here.
    fn sample(&mut self, upper: bool, value: u16) {
        // Sixteen bits, signed.
        let v = value as i16;
        if upper {
            self.left.push(v)
        } else {
            self.right.push(v)
        }
        // A channel that is never read by the host must not grow without
        // bound. Eight thousand samples is a third of a second at the rate
        // these boards run at, which is far more than a host drawing frames
        // will ever be behind by.
        //
        // Trimmed in blocks rather than one at a time, because dropping the
        // front sample of a full buffer moves every other sample down and this
        // is called twenty-four thousand times a second. Letting it run to
        // twice the cap and then trimming makes that cost amortise away.
        const CAP: usize = 8192;
        if self.left.len() > CAP * 2 {
            self.left.drain(..self.left.len() - CAP);
        }
        if self.right.len() > CAP * 2 {
            self.right.drain(..self.right.len() - CAP);
        }
    }

    /// Advances the timers and works out whether the processor should see an
    /// interrupt.
    pub fn tick(&mut self, cycles: u32) -> bool {
        // Almost every call has nothing to do. A sample interrupt at
        // twenty-four kilohertz on a forty-megahertz part is one event every
        // sixteen hundred cycles, and this is asked after every instruction,
        // so the common case has to be an add and a compare rather than three
        // timers' worth of division. The cycles are not lost, only owed.
        self.owed += cycles;
        if self.owed < self.quiet_for {
            return self.irq_line;
        }
        self.settle();
        self.irq_line
    }

    /// Brings the timers up to date and works out how long they can be left
    /// alone again. Called when [`Sound::tick`] runs out of quiet, and before
    /// anything reads or writes a peripheral, because firmware that asks a
    /// timer for its count has to be told the truth.
    fn settle(&mut self) {
        let cycles = std::mem::take(&mut self.owed);
        for (i, timer) in self.timers.iter_mut().enumerate() {
            // Bit 4 upwards of the power register is a timer's clock.
            if self.power & (0x10 << i) != 0 {
                timer.advance(cycles);
            }
            // A peripheral's interrupt is a **line**, not an event, and the
            // controller's pending bit is that line rather than a latch of it.
            // Latching it instead is a board that interrupts once and then
            // never again: the handler reads the timer's status, which is what
            // drops the line, and a latched pending bit does not notice.
            let asserted = timer.status & timer.irq_mask != 0;
            if asserted {
                self.aic.raise(irq::TC0 + i as u32);
            } else {
                self.aic.clear(irq::TC0 + i as u32);
            }
        }
        self.irq_line = self.aic.asserted();
        self.quiet_for = self
            .timers
            .iter()
            .enumerate()
            .map(|(i, t)| match self.power & (0x10 << i) != 0 {
                true => t.until_event(),
                false => u32::MAX,
            })
            .min()
            .unwrap_or(u32::MAX)
            .max(1);
    }

    fn read_peripheral(&mut self, addr: u32) -> u32 {
        self.last_read = Some(addr);
        self.settle();
        match addr & 0xffff_f000 {
            // The bus interface. Only the remap register does anything.
            0xffe0_0000 => 0,
            0xfffe_0000 => {
                let channel = ((addr >> 6) & 3) as usize;
                if channel > 2 {
                    return 0;
                }
                match addr & 0x3f {
                    0x04 => self.timers[channel].mode,
                    0x10 => self.timers[channel].counter,
                    0x14 => self.timers[channel].ra,
                    0x18 => self.timers[channel].rb,
                    0x1c => self.timers[channel].rc,
                    0x20 => self.timers[channel].take_status(),
                    // The mask is read at 0x2c (`at91.c:1386`). 0x28 is the
                    // disable register, which is write-only.
                    0x2c => self.timers[channel].irq_mask,
                    _ => 0,
                }
            }
            0xffff_0000 => match addr & 0xfff {
                // The status registers, not their enable/disable pairs: which
                // lines the port controls at 0x08 and which are outputs at
                // 0x18 (`at91.c:1416,1419`). 0x00 and 0x10 are the write-only
                // enables, and the reference does not answer them.
                0x08 => self.pio.enabled,
                0x18 => self.pio.output,
                0x38 => self.pio.data,
                0x3c => self.pio.pins(),
                0x48 => self.pio.irq_mask,
                0x4c => std::mem::take(&mut self.pio.irq_status),
                _ => 0,
            },
            // Power saving: only the clock status register reads back, at
            // 0x0c (`at91.c:1465`). The other three are write-only.
            0xffff_4000 => match addr & 0xfff {
                0x0c => self.power,
                _ => 0,
            },
            // The interrupt controller. The register map is Atmel's and the
            // order of the first two blocks is the one thing in it that is
            // easy to get backwards: the **modes** are at zero and the
            // **vectors** are at 0x80, not the other way round. Swapped, the
            // controller answers a priority where the handler expects an
            // address and the processor branches into nowhere — having taken
            // the interrupt perfectly correctly.
            0xffff_f000 => match addr & 0xfff {
                0x000..=0x07f => self.aic.modes[((addr & 0x7f) / 4) as usize],
                0x080..=0x0ff => self.aic.vectors[((addr & 0x7f) / 4) as usize],
                // Reading the interrupt vector register is the acknowledge.
                0x100 => self.aic.acknowledge(),
                0x104 => self.aic.fast_vector(),
                // Which source is being serviced.
                0x108 => self.aic.in_service().unwrap_or(0),
                0x10c => self.aic.pending,
                0x110 => self.aic.enabled,
                // Whether the core's lines are asserted: bit 0 the fast one,
                // bit 1 the normal one.
                0x114 => u32::from(self.aic.asserted()) << 1,
                _ => 0,
            },
            _ => {
                self.last_unmapped = Some((addr, false));
                0
            }
        }
    }

    fn write_peripheral(&mut self, addr: u32, value: u32) {
        // A write can start a timer, change its divider or move its compare,
        // and all three change how long the timers may be left alone. Settling
        // first also means the cycles owed are charged against the old setup
        // rather than the new one.
        self.settle();
        match addr & 0xffff_f000 {
            0xffe0_0000 => {
                // The remap. Writing bit 0 moves the RAM the program was built
                // in down to page zero, which is the moment the board stops
                // running the BIOS and starts running the game's sound.
                if addr & 0xfff == 0x20 && value & 1 != 0 && !self.remapped {
                    self.page0.copy_from_slice(&self.reset_ram[..]);
                    self.remapped = true;
                }
            }
            0xfffe_0000 => {
                let channel = ((addr >> 6) & 3) as usize;
                if channel > 2 {
                    return;
                }
                match addr & 0x3f {
                    0x00 => {
                        // The control register: enable, disable, and the
                        // software trigger that resets the count.
                        if value & 1 != 0 {
                            self.timers[channel].running = true;
                        }
                        if value & 2 != 0 {
                            self.timers[channel].running = false;
                        }
                        if value & 4 != 0 {
                            self.timers[channel].counter = 0;
                            self.timers[channel].running = true;
                        }
                    }
                    0x04 => self.timers[channel].mode = value,
                    0x14 => self.timers[channel].ra = value,
                    0x18 => self.timers[channel].rb = value,
                    0x1c => self.timers[channel].rc = value,
                    0x24 => self.timers[channel].irq_mask |= value,
                    0x28 => self.timers[channel].irq_mask &= !value,
                    _ => {}
                }
            }
            0xffff_0000 => match addr & 0xfff {
                0x00 => self.pio.enabled |= value,
                0x04 => self.pio.enabled &= !value,
                0x10 => self.pio.output |= value,
                0x14 => self.pio.output &= !value,
                0x30 => self.pio.data |= value,
                0x34 => self.pio.data &= !value,
                0x40 => self.pio.irq_mask |= value,
                0x44 => self.pio.irq_mask &= !value,
                _ => {}
            },
            // The clock enable is at 0x04 and the disable at 0x08
            // (`at91.c:1148,1157`); 0x00 is the control register, whose only
            // job is entering idle mode, and it does not touch the clocks.
            // Decoded one register down, an enable is a disable — latent only
            // because reset already has the timer clocks on.
            0xffff_4000 => match addr & 0xfff {
                0x04 => self.power |= value,
                0x08 => self.power &= !value,
                _ => {}
            },
            0xffff_f000 => {
                let off = addr & 0xfff;
                match off {
                    0x000..=0x07f => {
                        let src = (off / 4) as usize;
                        self.aic.modes[src] = value;
                        // Bit 5 of the mode says how the source is triggered.
                        // A level-sensitive one stays pending until whatever
                        // raised it is dealt with; an edge-triggered one is
                        // cleared by being taken.
                        if value & 0x20 == 0 {
                            self.aic.level_sensitive |= 1 << src;
                        } else {
                            self.aic.level_sensitive &= !(1 << src);
                        }
                    }
                    0x080..=0x0ff => {
                        self.aic.vectors[((off - 0x80) / 4) as usize] = value;
                    }
                    0x120 => self.aic.enabled |= value,
                    0x124 => self.aic.enabled &= !value,
                    0x128 => self.aic.clear_many(value),
                    0x12c => self.aic.pending |= value,
                    0x130 => self.aic.end_of_interrupt(),
                    0x134 => self.aic.spurious_vector = value,
                    _ => {}
                }
            }
            _ => self.last_unmapped = Some((addr, true)),
        }
        // The settle above computed how long the timers may be left alone —
        // under the setup this write has just replaced. A write that starts a
        // timer, or turns its clock back on through the power register, would
        // otherwise leave a stale bound of "forever", and the sample interrupt
        // would sit unfired until the firmware happened to touch some other
        // peripheral. Settling again costs nothing — the cycles owed were
        // already taken — and recomputes the bound against what the register
        // says now.
        self.settle();
    }
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for Sound {
    fn read8(&mut self, addr: u32) -> u8 {
        match addr {
            0x0000_0000..=0x000f_ffff => self.page0[addr as usize],
            0x0030_0000..=0x003f_ffff => self.reset_ram[(addr - 0x0030_0000) as usize],
            0x0040_0000..=0x005f_ffff => self.bios[(addr - 0x0040_0000) as usize],
            0x1000_0000..=0x101f_ffff => self.bios[(addr - 0x1000_0000) as usize],
            // The command from the CPU board, which reading takes.
            0x2000_0000..=0x2000_0003 => {
                self.aic.clear(irq::FIQ);
                self.reads[0] += 1;
                self.last_read = Some(addr);
                self.take_command()
            }
            0x2000_0004..=SAMPLES_TO => {
                self.reads[1] += 1;
                self.sample_byte(addr)
            }
            0x4000_0000..=0x4000_ffff => self.u7[(addr - 0x4000_0000) as usize],
            _ if addr >= 0xffe0_0000 => (self.read_peripheral(addr & !3) >> ((addr & 3) * 8)) as u8,
            _ => {
                self.last_unmapped = Some((addr, false));
                0
            }
        }
    }

    fn write16(&mut self, addr: u32, value: u16) {
        if Self::is_sample_port(addr) {
            // Which half of the word decides which channel it is.
            self.sample(addr & 2 != 0, value);
            return;
        }
        self.write8(addr, value as u8);
        self.write8(addr.wrapping_add(1), (value >> 8) as u8);
    }

    fn write8(&mut self, addr: u32, value: u8) {
        match addr {
            0x0000_0000..=0x000f_ffff => self.page0[addr as usize] = value,
            0x0030_0000..=0x003f_ffff => self.reset_ram[(addr - 0x0030_0000) as usize] = value,
            0x0040_0000..=0x005f_ffff => self.bios[(addr - 0x0040_0000) as usize] = value,
            // The ROMs do not take writes, and firmware writes to them anyway
            // while it is working out how big they are.
            0x1000_0000..=0x207f_ffff | 0x4000_0000..=0x4000_ffff => {}
            _ if addr >= 0xffe0_0000 => {
                let shift = (addr & 3) * 8;
                let now = self.read_peripheral(addr & !3);
                let next = (now & !(0xff << shift)) | (u32::from(value) << shift);
                self.write_peripheral(addr & !3, next);
            }
            _ => self.last_unmapped = Some((addr, true)),
        }
    }

    fn read32(&mut self, addr: u32) -> u32 {
        let addr = addr & !3;
        match addr {
            0x0000_0000..=0x000f_fffc => word(&self.page0[..], addr as usize),
            0x0030_0000..=0x003f_fffc => word(&self.reset_ram[..], (addr - 0x0030_0000) as usize),
            0x0040_0000..=0x005f_fffc => word(&self.bios[..], (addr - 0x0040_0000) as usize),
            // Byte-wide devices: a word is four accesses on the bus, not one.
            0x1000_0000..=0x101f_fffc => word(&self.bios[..], (addr - 0x1000_0000) as usize),
            0x2000_0000..=0x2000_0003 => {
                self.aic.clear(irq::FIQ);
                self.reads[2] += 1;
                self.last_read = Some(addr);
                u32::from(self.take_command())
            }
            // A word from a byte-wide part is four accesses on the bus, and
            // the part answers each of them for itself.
            0x2000_0004..=SAMPLES_TO => {
                self.reads[3] += 1;
                u32::from_le_bytes([
                    self.sample_byte(addr),
                    self.sample_byte(addr + 1),
                    self.sample_byte(addr + 2),
                    self.sample_byte(addr + 3),
                ])
            }
            0x4000_0000..=0x4000_fffc => word(&self.u7[..], (addr - 0x4000_0000) as usize),
            _ if addr >= 0xffe0_0000 => self.read_peripheral(addr),
            _ => {
                self.last_unmapped = Some((addr, false));
                0
            }
        }
    }

    fn write32(&mut self, addr: u32, value: u32) {
        let addr = addr & !3;
        match addr {
            0x0000_0000..=0x000f_fffc => set_word(&mut self.page0[..], addr as usize, value),
            0x0030_0000..=0x003f_fffc => {
                set_word(
                    &mut self.reset_ram[..],
                    (addr - 0x0030_0000) as usize,
                    value,
                );
            }
            0x0040_0000..=0x005f_fffc => {
                set_word(&mut self.bios[..], (addr - 0x0040_0000) as usize, value);
            }
            // The samples. The firmware writes them a halfword per channel —
            // that path is in [`Bus::write16`] — and a full-word store makes
            // **one** sample, not two: the reference's handler
            // (`desound.c:573-586`) tests the upper half of the mask first,
            // and a word store has both halves, so the upper wins and the
            // lower is never looked at.
            0x2040_0000..=0x2040_0003 => {
                self.sample(true, (value >> 16) as u16);
            }
            0x1000_0000..=0x207f_fffc | 0x4000_0000..=0x4000_fffc => {}
            _ if addr >= 0xffe0_0000 => self.write_peripheral(addr, value),
            _ => self.last_unmapped = Some((addr, true)),
        }
    }
}

/// The sample port, and the halfword store that reaches it.
impl Sound {
    fn is_sample_port(addr: u32) -> bool {
        (0x2040_0000..=0x2040_0003).contains(&addr)
    }
}

fn word(mem: &[u8], at: usize) -> u32 {
    if at + 4 > mem.len() {
        return 0;
    }
    u32::from_le_bytes([mem[at], mem[at + 1], mem[at + 2], mem[at + 3]])
}

fn set_word(mem: &mut [u8], at: usize, value: u32) {
    if at + 4 > mem.len() {
        return;
    }
    mem[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
