//! The CPU board: a 6502, three RIOTs and the wiring between them.

use vpw_bus::Bus;
use vpw_m6502::Cpu;
use vpw_riot::Riot;

use crate::display::Displays;
use crate::io::Io;
use crate::sound::SoundBoard;

/// How many bytes of battery-backed RAM the board has.
///
/// Two hundred and fifty-six, mirrored eight times across `$1800`–`$1FFF`
/// because only the low eight address lines reach the chip. The audits, the
/// settings and the high scores all live here, and it is what a host has to
/// keep between sessions for a machine to be the same machine next time.
pub const NVRAM: usize = 256;

/// The board's memory, and everything hanging off it.
///
/// Separate from [`Board`] because the CPU needs the bus mutably while it
/// runs, and everything the outside world wants to look at is here.
pub struct System80 {
    /// U4, U5 and U6.
    pub riot: [Riot; 3],
    /// The game ROM, 2 KB at `$1000`.
    pub game: Vec<u8>,
    /// The two system ROMs, 4 KB each at `$2000` and `$3000`.
    pub u2: Vec<u8>,
    pub u3: Vec<u8>,
    /// The battery-backed RAM.
    pub nvram: [u8; NVRAM],

    pub io: Io,
    pub displays: Displays,

    /// Clocks left before the next vertical blank, and how many have gone by.
    ///
    /// A System 80 has no video and no vertical blank; PinMAME hangs its
    /// housekeeping off the emulated one anyway, and the periods it chose are
    /// what tables are written against. Sixty a second, and the solenoid sweep
    /// every fourth (`gts80.c:34`).
    to_vblank: i32,
    vblanks: u32,
}

impl System80 {
    /// Builds a board around a set of ROMs.
    pub fn new(game: Vec<u8>, u2: Vec<u8>, u3: Vec<u8>) -> Self {
        Self {
            riot: [Riot::new(), Riot::new(), Riot::new()],
            game,
            u2,
            u3,
            nvram: [0; NVRAM],
            io: Io::new(),
            displays: Displays::new(),
            to_vblank: VBLANK_CLOCKS,
            vblanks: 0,
        }
    }

    /// Whether any of the three RIOTs is asking for an interrupt.
    ///
    /// Their interrupt pins are open drain and all three pull the same line,
    /// so the CPU sees the OR of them.
    pub fn irq(&self) -> bool {
        self.riot.iter().any(Riot::irq)
    }

    /// Runs the RIOTs' timers for a number of CPU clocks.
    pub fn tick(&mut self, clocks: u32) {
        // The slam switch reaches RIOT 1's port A, so it is put there before
        // the chips look at their inputs. All ones is "not slammed", which is
        // the state of a machine nobody is kicking.
        self.riot[1].in_a = if self.io.slammed() { 0x00 } else { 0xFF };
        for riot in &mut self.riot {
            riot.tick(clocks);
        }

        self.to_vblank -= i32::try_from(clocks).unwrap_or(i32::MAX);
        while self.to_vblank <= 0 {
            self.to_vblank += VBLANK_CLOCKS;
            self.vblanks = self.vblanks.wrapping_add(1);
            if self.vblanks.is_multiple_of(SOLENOID_SWEEP) {
                self.io.sweep();
            }
            if self.vblanks.is_multiple_of(DISPLAY_SWEEP) {
                self.displays.sweep();
            }
        }
    }

    /// Puts the switch returns — or the dips — on RIOT 0's port A.
    ///
    /// Which of the two depends on a bit of RIOT 1's port B, and the bank on
    /// two bits of RIOT 2's, because a System 80 has more things to read than
    /// it has pins to read them with.
    fn refresh_switch_input(&mut self) {
        let reading_dips = self.riot[1].port_b_output() & 0x80 != 0;
        self.riot[0].in_a = if reading_dips {
            let bank = usize::from((self.riot[2].port_b_output() >> 4) & 0x03);
            // Reversed, because the bank is wired to the port the other way
            // up. Off by a mirror and every setting reads as its opposite.
            self.io.dips[bank].reverse_bits()
        } else {
            let strobe = self.riot[0].port_b_output();
            self.io.switch_returns(strobe)
        };
    }

    /// RIOT 2's port A: nine solenoids and the sound board's command.
    ///
    /// The port drives the coils through inverting buffers, so the byte here
    /// is read upside down before anything is decided from it. The first eight
    /// coils are two banks of four, each with its own enable — which is why a
    /// System 80 can fire only one coil from each bank at a time — and the
    /// ninth has a pin to itself.
    fn solenoids_written(&mut self, written: u8) {
        let data = !written;

        let mut solenoids = 0u16;
        if data & 0x20 != 0 {
            solenoids |= 1 << (data & 0x03);
        }
        if data & 0x40 != 0 {
            solenoids |= 0x10 << ((data >> 2) & 0x03);
        }
        if data & 0x80 != 0 {
            solenoids |= 0x100;
        }
        self.io.drive_solenoids(solenoids);

        // The sound command shares the port. Four bits of it, with a fifth
        // taken from a lamp latch, which is how a board with sixteen sounds
        // came to have thirty-two.
        let command = if data & 0x10 != 0 { data & 0x0F } else { 0 };
        let high = u8::from(self.io.lamps[2] & 0x02 != 0) << 4;
        let full = high | command;
        if full != self.io.sound_command {
            self.io.sound_command = full;
            self.io.sound_pending = true;
        }
    }

    /// RIOT 2's port B: twelve lamp latches, four lamps each.
    ///
    /// The top nibble picks the latch and the bottom one is what goes in it.
    /// Latch zero is not a lamp at all: its bits are the game-on and tilt
    /// relays.
    fn lamps_written(&mut self, written: u8) {
        // The latches are selected from 1 to 12 and counted here from 0, which
        // is the numbering everything downstream uses: column 0 is the relays.
        let selected = usize::from(written >> 4);
        if selected == 0 || selected > crate::io::LAMP_COLUMNS {
            return;
        }
        self.io.lamps[selected - 1] = written & 0x0F;
    }
}

impl Bus for System80 {
    fn read(&mut self, addr: u16) -> u8 {
        // Address lines 14 and 15 are not connected, so the map repeats four
        // times. This is what puts the CPU's vectors, fetched from `$FFFA`, in
        // the top of the U3 ROM.
        let a = addr & 0x3FFF;
        match a {
            0x0000..=0x007F => self.riot[0].ram[usize::from(a)],
            0x0080..=0x00FF => self.riot[1].ram[usize::from(a - 0x0080)],
            0x0100..=0x017F => self.riot[2].ram[usize::from(a - 0x0100)],
            // Two addresses in the hole above RIOT 2's RAM answer with the
            // slam switch. They are not a chip; they are a gate on the board.
            0x01CB | 0x01E4 => {
                if self.io.slammed() {
                    0x00
                } else {
                    0xFF
                }
            }
            0x0200..=0x027F => {
                self.refresh_switch_input();
                self.riot[0].read(a)
            }
            0x0280..=0x02FF => self.riot[1].read(a),
            0x0300..=0x037F => self.riot[2].read(a),
            0x1000..=0x17FF => byte(&self.game, usize::from(a - 0x1000)),
            0x1800..=0x1FFF => self.nvram[usize::from(a & 0x00FF)],
            0x2000..=0x2FFF => byte(&self.u2, usize::from(a - 0x2000)),
            0x3000..=0x3FFF => byte(&self.u3, usize::from(a - 0x3000)),
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        let a = addr & 0x3FFF;
        match a {
            0x0000..=0x007F => self.riot[0].ram[usize::from(a)] = value,
            0x0080..=0x00FF => self.riot[1].ram[usize::from(a - 0x0080)] = value,
            0x0100..=0x017F => self.riot[2].ram[usize::from(a - 0x0100)] = value,
            0x0200..=0x027F => self.riot[0].write(a, value),
            0x0280..=0x02FF => {
                self.riot[1].write(a, value);
                // Port A carries the digit select and the latch strobes, and
                // the displays are driven from the edges of those, so they are
                // looked at after every write rather than polled.
                let (pa, pb) = (self.riot[1].port_a_output(), self.riot[1].port_b_output());
                self.displays.port_a(pa, pb);
            }
            0x0300..=0x037F => {
                self.riot[2].write(a, value);
                match a & 0x03 {
                    0 => {
                        let out = self.riot[2].port_a_output();
                        self.solenoids_written(out);
                    }
                    2 => {
                        let out = self.riot[2].port_b_output();
                        self.lamps_written(out);
                    }
                    _ => {}
                }
            }
            0x1800..=0x1FFF => self.nvram[usize::from(a & 0x00FF)] = value,
            // A write to ROM is a bug in the ROM or in us, and either way it
            // is not a crash.
            _ => {}
        }
    }
}

fn byte(rom: &[u8], at: usize) -> u8 {
    rom.get(at).copied().unwrap_or(0xFF)
}

/// A whole System 80: the board, the processor on it, and the card it talks to.
pub struct Board {
    pub cpu: Cpu,
    pub mem: System80,
    /// The sound card, when the set carried one this port knows. Its absence
    /// costs the sound and cannot stall the game: the CPU board writes a
    /// command to a latch and never waits for an answer.
    pub sound: Option<SoundBoard>,
}

impl Board {
    /// Builds a machine and pulls its reset.
    pub fn new(game: Vec<u8>, u2: Vec<u8>, u3: Vec<u8>) -> Self {
        let mut mem = System80::new(game, u2, u3);
        let mut cpu = Cpu::new();
        cpu.reset(&mut mem);
        Self {
            cpu,
            mem,
            sound: None,
        }
    }

    /// Plugs a sound card in, built from whatever the set carried.
    pub fn load_sound(&mut self, sound: crate::games::Sound<'_>, rate: u32) {
        self.sound = Some(match sound {
            crate::games::Sound::Piggyback(rom) => SoundBoard::piggyback(rom.to_vec(), rate),
            crate::games::Sound::Card { rom, system } => {
                SoundBoard::sound(rom.to_vec(), system.to_vec(), rate)
            }
        });
    }

    /// Runs one instruction and gives the chips the clocks it took.
    pub fn step(&mut self) -> u32 {
        self.cpu.irq = self.mem.irq();
        let clocks = self.cpu.step(&mut self.mem);
        self.mem.tick(clocks);

        if let Some(sound) = &mut self.sound {
            // A command reaches the card the instant the latch is written; on
            // a piggyback it interrupts the card's processor, and on the older
            // one it sits on port B until the firmware next looks.
            if self.mem.io.sound_pending {
                self.mem.io.sound_pending = false;
                sound.command(self.mem.io.sound_command);
            }
            // The two processors are clocked by different things — one a
            // crystal, the other a resistor and a capacitor — so the card runs
            // for its own share of the same wall-clock time, remainder
            // carried.
            sound.run(f64::from(clocks) * f64::from(sound.card().clock()) / f64::from(CLOCK));
        }
        clocks
    }

    /// Runs for a number of CPU clocks, give or take the last instruction.
    pub fn run(&mut self, clocks: u32) {
        let mut spent = 0;
        while spent < clocks {
            spent += self.step();
        }
    }

    /// Runs for a stretch of wall-clock time.
    pub fn run_seconds(&mut self, seconds: f64) {
        self.run((f64::from(CLOCK) * seconds.max(0.0)) as u32);
    }

    /// The battery-backed RAM, which is what a host keeps between sessions.
    pub fn cmos(&self) -> &[u8] {
        &self.mem.nvram
    }

    /// Puts a previous session's memory back.
    pub fn restore_cmos(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(NVRAM);
        self.mem.nvram[..n].copy_from_slice(&bytes[..n]);
    }

    /// Takes the sound card's audio away, resampled to the rate asked for.
    pub fn take_audio_at(&mut self, rate: u32) -> Vec<f32> {
        match &mut self.sound {
            Some(s) => s.take_audio_at(rate),
            None => Vec::new(),
        }
    }

    /// Whether there is a card at all, and how many samples it has made.
    pub fn sound_stats(&self) -> (bool, u64) {
        match &self.sound {
            Some(s) => (true, s.produced()),
            None => (false, 0),
        }
    }

    /// The score displays, as segment words a host can draw.
    ///
    /// The two player banks, sixteen positions each — which is exactly the two
    /// rows of sixteen a score panel already has. The ball-and-credit strip is
    /// a third bank with nowhere to go on that panel and is left out rather
    /// than crammed in; it is `mem.displays.digits[2]` for whoever wants it.
    pub fn segments(&self) -> Vec<u16> {
        self.mem.displays.digits[0]
            .iter()
            .chain(self.mem.displays.digits[1].iter())
            .map(|s| u16::from(*s))
            .collect()
    }
}

/// The CPU clock: 3.579545 MHz divided by four (`gts80.c:708`).
pub const CLOCK: u32 = 894_886;

/// Clocks between two of PinMAME's vertical blanks, at sixty a second.
const VBLANK_CLOCKS: i32 = (CLOCK / 60) as i32;

/// How many of those the solenoid sweep spans (`GTS80_SOLSMOOTH`).
const SOLENOID_SWEEP: u32 = 4;

/// And the display's, which is quicker because a score that lags is worse than
/// one that flickers (`GTS80_DISPLAYSMOOTH`).
const DISPLAY_SWEEP: u32 = 2;
