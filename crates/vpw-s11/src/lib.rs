//! Williams System 11 machine: 6808 + six PIAs + the board's timing.
//!
//! This is the F-14 Tomcat hardware (System 11A). The memory map and the
//! timings come from PinMAME's driver (`src/wpc/s11.c`), which is under
//! BSD-3-Clause.
//!
//! # Memory map
//!
//! | Range | What's there |
//! |-------|---------|
//! | `$0000`-`$1FFF` | Battery-backed RAM (CMOS) |
//! | `$2100`-`$2103` | PIA 0: sound latch, solenoids 9-16 |
//! | `$2200` | Latch for solenoids 1-8 (write only) |
//! | `$2400`-`$2403` | PIA 1: lamp matrix |
//! | `$2800`-`$2803` | PIA 2: digit select, diagnostics |
//! | `$2C00`-`$2C03` | PIA 3: display data |
//! | `$3000`-`$3003` | PIA 4: switch matrix |
//! | `$3400`-`$3403` | PIA 5: secondary display, Widget I/O |
//! | `$4000`-`$FFFF` | Game ROM (48 KB) |
//!
//! # Timing
//!
//! The CPU runs at 1 MHz. The board generates a periodic interrupt every 928
//! cycles (`S11_IRQFREQ` in `s11.c:40`) that is held for 32 cycles and then
//! released. That line is wired together with the twelve interrupt lines of the
//! six PIAs, so the IRQ the 6808 sees is the OR of all of them.
//!
//! # The sound board
//!
//! It is a separate computer, with its own 6808. The main board sends it a
//! sound number through PIA 0's latch and pulses the handshake on CA2; it takes
//! care of the rest on its own. See [`SoundBoard`].
//!
//! F-14 carries **two** sound boards: the XS ([`SoundBoard`]) for effects and
//! speech, and the CS ([`CsSoundBoard`]) for the music.
//!
//! # What's missing
//!
//! The Widget I/O: its PIA exists and responds, but nobody looks at what it
//! puts out.

mod board;
pub mod bulb;
pub mod display;
pub mod games;
pub mod io;
pub mod levels;
mod sound;
mod sound_cs;

pub use board::{
    Board, PIA_BASES, RAM_SIZE, ROM_SIZE, ROM_START, RomError, SOLENOID_LATCH, UnmappedAccess, pia,
};
pub use display::{
    DIGIT_SLOTS, DisplayConfig, Displays, VISIBLE_CHARS, seg, segments_to_char,
    segments_to_char_7seg,
};
pub use io::{LampMatrix, Solenoids, SwitchMatrix, matrix_position, switches};
pub use sound::{SOUND_CPU_CLOCK_HZ, SoundBoard, SoundBus};
pub use sound_cs::{CS_CPU_CLOCK_HZ, CsSoundBoard, CsSoundBus, YM_CLOCK_HZ, YM_SAMPLE_RATE};

pub use vpw_audio::{DcBlocker, SampleClock};
pub use vpw_m6800::{Bus, Cpu};
pub use vpw_pia6821::Pia;

/// Main CPU clock (`s11.c:1320`).
pub const CPU_CLOCK_HZ: u32 = 1_000_000;

/// Period of the board's interrupt, in CPU cycles.
///
/// `S11_IRQFREQ` is defined as `1000000.0/928.0`, that is, one interrupt every
/// 928 cycles: about 1078 per second.
pub const IRQ_PERIOD_CYCLES: u64 = 928;

/// How many cycles the interrupt is held before being released
/// (`s11.c:s11_irq`).
pub const IRQ_HOLD_CYCLES: u64 = 32;

/// Default sample rate of the output audio.
pub const DEFAULT_AUDIO_RATE: u32 = 48_000;

/// How many cycles between the end of one display sweep and the next.
///
/// The display is multiplexed: the CPU walks the 16 digit slots over and over.
/// Publishing at 60 Hz gives the sweep plenty of time to complete, which is the
/// same thing the phosphor's persistence does on the real machine.
pub const DISPLAY_REFRESH_CYCLES: u64 = CPU_CLOCK_HZ as u64 / 60;

/// The complete machine.
pub struct System11 {
    pub cpu: Cpu,
    pub board: Board,
    /// XS sound board: effects and speech. The machine boots and reaches the
    /// operator menu without either of the two.
    pub sound: Option<SoundBoard>,
    /// CS sound board: music and speech.
    pub sound_cs: Option<CsSoundBoard>,

    /// Already-mixed audio, ready for the host. It fills up as the machine
    /// runs; whoever consumes it has to empty it with
    /// [`System11::drain_audio`].
    audio: Vec<f32>,
    audio_clock: SampleClock,
    /// The coupling capacitor between the sound boards and the amplifier.
    /// Both boards idle well off zero; see [`vpw_audio::DcBlocker`].
    dc: DcBlocker,
    /// The two op-amp stages each speech channel goes through on the way out of
    /// the cabinet, one set per board. Not part of the chip: they are on the
    /// sound board, and the machine driver names which pair
    /// (`HC55516_FILTER_SYS11`, `wmssnd.c:481`). Without them what reaches the
    /// speaker is the decoder's raw staircase, hash and all — which PinMAME
    /// blames by name for the speech sounding "terrible" (`wmssnd.c:242`).
    speech: [vpw_audio::OutputFilter; 2],
    /// CPU cycles since reset.
    cycle: u64,
    /// Previous state of the periodic interrupt, to detect the edge.
    periodic_was_asserted: bool,
    /// Cycle at which the last display sweep was closed.
    last_display_refresh: u64,
}

impl Default for System11 {
    fn default() -> Self {
        Self::new()
    }
}

impl System11 {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            board: Board::new(),
            sound: None,
            sound_cs: None,
            audio: Vec::new(),
            audio_clock: SampleClock::new(CPU_CLOCK_HZ, DEFAULT_AUDIO_RATE),
            dc: DcBlocker::new(DEFAULT_AUDIO_RATE),
            speech: [vpw_audio::OutputFilter::new(
                vpw_audio::biquad::Board::System11,
                f64::from(DEFAULT_AUDIO_RATE),
            ); 2],
            cycle: 0,
            periodic_was_asserted: false,
            last_display_refresh: 0,
        }
    }

    /// Loads the two ROM images of a 48 KB System 11.
    ///
    /// This is the `S11_ROMSTART48` layout (`s11.h:133`): the first image is
    /// 16 KB at `$4000` and the second one 32 KB at `$8000`. For F-14 L-1 they
    /// are `f14_u26.l1` and `f14_u27.l1`.
    pub fn load_rom_48(&mut self, u26: &[u8], u27: &[u8]) -> Result<(), RomError> {
        self.board.load_rom(0x4000, u26)?;
        self.board.load_rom(0x8000, u27)
    }

    /// Loads the two images of the sound board: `u21` and `u22`, 32 KB each.
    /// For F-14 they are `f14_u21.l1` and `f14_u22.l1`.
    ///
    /// Returns `false` if either one is not 32 KB.
    pub fn load_sound_roms(&mut self, u21: &[u8], u22: &[u8]) -> bool {
        let mut board = SoundBoard::new();
        if !board.load_roms(u21, u22) {
            return false;
        }
        board.reset();
        self.sound = Some(board);
        true
    }

    /// Loads the two main-CPU images, deducing the layout from the size of the
    /// first one.
    ///
    /// System 11 machines come in two variants: the 16 KB + 32 KB one, which
    /// fills the whole 48 KB of the map, and the 8 KB + 32 KB one, where the
    /// small image is **mirrored** to cover the gap from `$6000` to `$7FFF`.
    /// The second image is always 32 KB.
    pub fn load_main_roms(&mut self, first: &[u8], second: &[u8]) -> Result<(), RomError> {
        if second.len() != 0x8000 {
            return Err(RomError::TooLarge {
                addr: 0x8000,
                len: second.len(),
            });
        }
        match first.len() {
            0x4000 => self.board.load_rom(0x4000, first)?,
            0x2000 => {
                self.board.load_rom(0x4000, first)?;
                self.board.load_rom(0x6000, first)?;
            }
            len => return Err(RomError::TooLarge { addr: 0x4000, len }),
        }
        self.board.load_rom(0x8000, second)
    }

    /// Loads images into the CPU's 64 KB region, each one with its region
    /// offset.
    ///
    /// This is the general form: it covers the four layouts Williams and Data
    /// East use, including the ones that load from `$0000` and leave the first
    /// part covered by the RAM.
    pub fn load_cpu_region(&mut self, images: &[(usize, &[u8])]) -> Result<(), RomError> {
        for &(offset, data) in images {
            self.board.load_cpu_region(offset, data)?;
        }
        Ok(())
    }

    /// Loads the two images of the CS board. For F-14 they are `f14_u4.l1` and
    /// `f14_u19.l1`.
    pub fn load_cs_sound_roms(&mut self, images: &[&[u8]]) -> bool {
        let mut board = CsSoundBoard::new();
        if !board.load_roms(images) {
            return false;
        }
        board.reset();
        self.sound_cs = Some(board);
        true
    }

    /// Audio sample from the XS board, or silence if it isn't there.
    pub fn sound_sample(&self) -> f32 {
        self.sound.as_ref().map_or(0.0, SoundBoard::sample)
    }

    /// Audio sample from the CS board, or silence if it isn't there.
    pub fn cs_sound_sample(&self) -> f32 {
        self.sound_cs.as_ref().map_or(0.0, CsSoundBoard::sample)
    }

    /// The two boards mixed together: what comes out of the cabinet.
    ///
    /// One sum over every source there is, not a board at a time. The original
    /// puts all five channels into a single mixer and each carries its own
    /// level (`wmssnd.c:479-501`); mixing each board down first and then
    /// averaging the two gives each board half the total regardless of what is
    /// inside it, which is a different weighting. See [`crate::levels`].
    /// It takes `&mut` because the speech goes through the board's output
    /// filter on the way, and a filter has a memory. Call it once per output
    /// sample and no more: calling it twice for the same instant advances the
    /// filter twice and is not the same signal.
    pub fn audio_sample(&mut self) -> f32 {
        let mut sources = [(0.0f32, 0.0f32); 5];
        let mut n = 0;
        if let Some(xs) = &self.sound {
            for s in xs.sources() {
                sources[n] = s;
                n += 1;
            }
        }
        if let Some(cs) = &self.sound_cs {
            for s in cs.sources() {
                sources[n] = s;
                n += 1;
            }
        }

        // Each board's speech through its own pair of stages. The effects and
        // the FM do not go through them: on the board those come off their own
        // outputs and meet the speech at the summing amplifier, past this.
        let mut speech = 0;
        for source in sources[..n].iter_mut() {
            if source.0 == levels::CVSD {
                source.1 = self.speech[speech].step(source.1);
                speech += 1;
            }
        }

        levels::weighted(&sources[..n])
    }

    /// Changes the sample rate of the output audio.
    pub fn set_audio_rate(&mut self, hz: u32) {
        self.audio_clock = SampleClock::new(CPU_CLOCK_HZ, hz);
        self.dc = DcBlocker::new(hz);
        // The cutoffs come from resistors and capacitors, so where they land in
        // a digital filter depends on the rate it runs at. A filter built for
        // another rate is not a slightly wrong filter, it is a different one.
        self.speech =
            [vpw_audio::OutputFilter::new(vpw_audio::biquad::Board::System11, f64::from(hz)); 2];
        self.audio.clear();
    }

    /// Takes the accumulated audio away and leaves the buffer empty.
    pub fn drain_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio)
    }

    /// How many samples are waiting.
    pub fn pending_audio(&self) -> usize {
        self.audio.len()
    }

    /// Reset: the CPU takes the vector at `$FFFE` and the PIAs are cleared.
    pub fn reset(&mut self) {
        for pia in &mut self.board.pias {
            pia.reset();
        }
        self.cycle = 0;
        self.board.cycle = 0;
        self.periodic_was_asserted = false;
        self.last_display_refresh = 0;
        self.cpu.reset(&mut self.board);
        if let Some(sound) = &mut self.sound {
            sound.reset();
        }
        if let Some(sound) = &mut self.sound_cs {
            sound.reset();
        }
        self.audio.clear();
        self.audio_clock.reset();
        self.dc.reset();
    }

    /// CPU cycles elapsed since reset.
    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// Simulated seconds since reset.
    pub fn elapsed_seconds(&self) -> f64 {
        self.cycle as f64 / f64::from(CPU_CLOCK_HZ)
    }

    /// Whether the board's periodic interrupt is asserted right now.
    ///
    /// No 6800 instruction lasts more than 12 cycles and the window is 32, so
    /// stepping instruction by instruction never skips over it.
    pub fn periodic_irq(&self) -> bool {
        self.cycle % IRQ_PERIOD_CYCLES < IRQ_HOLD_CYCLES
    }

    /// What the 6808's IRQ pin sees: the periodic one or any of the PIAs.
    pub fn irq_line(&self) -> bool {
        self.periodic_irq() || self.board.pia_irq()
    }

    /// Declares which solenoid throws the mux relay.
    /// See [`crate::io::Solenoids::set_mux`].
    pub fn set_mux_solenoid(&mut self, number: u8) {
        self.board.solenoids.set_mux(number);
    }

    /// Declares which playfield switches fire special solenoids directly.
    /// See [`crate::io::Solenoids::set_special_switch`].
    pub fn set_special_switches(&mut self, switches: [u8; 6]) {
        self.board.set_special_switches(switches);
    }

    /// Declares which switches the two flipper buttons close, left then right.
    /// See [`crate::io::FIRST_FLIPPER`].
    pub fn set_flipper_buttons(&mut self, buttons: (u8, u8)) {
        self.board.set_flipper_buttons(buttons);
    }

    /// Declares a solenoid output as driving a bulb rather than a coil.
    /// See [`crate::io::Solenoids::set_bulb`].
    pub fn set_solenoid_bulb(&mut self, number: u8, drive: crate::bulb::Drive) {
        self.board.solenoids.set_bulb(number, drive);
    }

    /// What the solenoid drivers are being told this instant, before any
    /// smoothing. For finding out whether the ROM is holding a coil or
    /// chopping it.
    pub fn solenoids_active(&self) -> u32 {
        self.board.solenoids.active()
    }

    /// Executes one instruction and returns the cycles it consumed.
    pub fn step(&mut self) -> u32 {
        self.board.refresh_inputs();
        self.cpu.set_irq(self.irq_line());

        // The display needs to know which cycle each write happens on, and the
        // writes happen inside the instruction.
        self.board.cycle = self.cycle;

        let cycles = self.cpu.step(&mut self.board);
        self.cycle += u64::from(cycles);

        self.update_diagnostic_lines();
        self.run_sound_boards();

        // The display, the lamps and the solenoids are all either multiplexed
        // or pulsed: the sweep for all three is closed at the same rate.
        // Take the audio samples that belong to these cycles.
        for _ in 0..self.audio_clock.advance(cycles) {
            let mixed = self.audio_sample();
            let sample = self.dc.process(mixed);
            self.audio.push(sample);
        }

        if self.cycle - self.last_display_refresh >= DISPLAY_REFRESH_CYCLES {
            self.board.displays.refresh();
            self.board.refresh_outputs();
            self.last_display_refresh = self.cycle;
        }

        cycles
    }

    /// Runs at least `cycles` cycles and returns how many it spent.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let target = self.cycle + cycles;
        while self.cycle < target {
            self.step();
        }
        self.cycle
    }

    /// Runs the equivalent of `seconds` of real time.
    pub fn run_seconds(&mut self, seconds: f64) -> u64 {
        self.run_cycles((seconds * f64::from(CPU_CLOCK_HZ)) as u64)
    }

    /// How this machine's display is wired.
    ///
    /// The default is a System 11A's. An 11B or 11C sends the segment data
    /// inverted and has both rows alphanumeric: reading it with the wrong
    /// configuration returns garbage, not running text.
    pub fn set_display_config(&mut self, config: DisplayConfig) {
        self.board.displays.set_config(config);
    }

    /// The top row of the display, alphanumeric, 14 characters.
    pub fn upper_display(&self) -> String {
        self.board.displays.upper_text()
    }

    /// Opens or closes a playfield switch by its Williams number
    /// (`column*10 + row`, both starting at 1).
    ///
    /// Returns `false` if the number does not exist in the 8x8 matrix.
    pub fn set_switch(&mut self, number: u8, closed: bool) -> bool {
        self.board.switches.set(number, closed)
    }

    /// Whether a switch is closed.
    pub fn switch_closed(&self, number: u8) -> bool {
        self.board.switches.is_closed(number)
    }

    /// Closes a switch, lets the machine run, and opens it again.
    ///
    /// That is what a ball does when it goes over a rollover, and it is the
    /// correct way to simulate it: closing and opening it without letting
    /// anything run in between does not give the matrix sweep time to see it,
    /// because the CPU only reads its column once per pass.
    pub fn pulse_switch(&mut self, number: u8, cycles: u64) -> bool {
        if !self.board.switches.set(number, true) {
            return false;
        }
        self.run_cycles(cycles);
        self.board.switches.set(number, false);
        true
    }

    /// Opens every switch.
    pub fn clear_switches(&mut self) {
        self.board.switches.clear();
    }

    /// Advance button of the diagnostic menu, inside the door.
    pub fn set_diagnostic_advance(&mut self, pressed: bool) {
        self.board.diagnostic_advance = pressed;
    }

    /// Up/down switch of the diagnostic menu.
    pub fn set_diagnostic_up(&mut self, up: bool) {
        self.board.diagnostic_up = up;
    }

    /// State of the playfield's 64 lamps.
    pub fn lamp_lit(&self, number: u8) -> bool {
        self.board.lamps.is_lit(number)
    }

    /// Whether a solenoid (1-16) fired during the last sweep.
    pub fn solenoid_fired(&self, number: u8) -> bool {
        self.board.solenoids.did_fire(number)
    }

    /// Every visible digit's segments, both rows, packed the way a table
    /// numbers them: the top row first, then the bottom row carrying straight
    /// on. See [`crate::display::visible_slots`].
    pub fn segments(&self) -> Vec<u16> {
        let d = &self.board.displays;
        let slots = crate::display::visible_slots(d.config());
        let (upper, lower) = (d.upper_segments(), d.lower_segments());
        slots
            .iter()
            .map(|&s| upper[s])
            .chain(slots.iter().map(|&s| lower[s]))
            .collect()
    }

    /// Contents of the battery-backed RAM. This is what has to be persisted
    /// between sessions so the machine doesn't lose its settings or its high
    /// scores.
    pub fn cmos(&self) -> &[u8] {
        self.board.ram()
    }

    /// Restores a saved CMOS. It has to be exactly [`RAM_SIZE`] bytes.
    pub fn load_cmos(&mut self, data: &[u8]) -> bool {
        if data.len() != RAM_SIZE {
            return false;
        }
        self.board.ram_mut().copy_from_slice(data);
        true
    }

    /// The bottom row, the seven-segment score displays.
    pub fn lower_display(&self) -> String {
        self.board.displays.lower_text()
    }

    /// Runs the sound board in parallel and passes it whatever the main board
    /// is telling it.
    ///
    /// Both CPUs run at 1 MHz, so the ratio is one to one. The latch comes out
    /// of PIA 0's port A and the handshake out of its CA2.
    /// Runs both sound boards in parallel.
    ///
    /// Each one receives its commands over a different path: the XS through
    /// PIA 0's port A with the handshake on CA2, and the CS through PIA 5's
    /// port B with the handshake on CB2 (`s11.c:661-663`, `s11.c:709`).
    fn run_sound_boards(&mut self) {
        let target = self.cycle;
        // Read what comes out of the main board before borrowing the sound
        // boards.
        let xs_latch = self.board.sound_latch();
        let xs_handshake = self.board.pias[pia::SOUND_SOLENOIDS].ca2_output();
        let cs_latch = self.board.pias[pia::WIDGET].port_b_output();
        let cs_handshake = self.board.pias[pia::WIDGET].cb2_output();

        if let Some(sound) = &mut self.sound {
            sound.bus.set_sound_latch(xs_latch);
            sound.bus.set_handshake(xs_handshake);
            // Absolute target: the XS runs at 1 MHz just like the main board.
            sound.run_until(target);
        }

        if let Some(sound) = &mut self.sound_cs {
            sound.bus.set_sound_latch(cs_latch);
            sound.bus.set_handshake(cs_handshake);
            // The CS runs at twice the clock, so its target is twice as far.
            sound.run_until(target * 2);
        }

        // Return channel: the CS answers over that same PIA 5 port B, and
        // signals it on CB1 (`s11.c:849`, `pia_5_cb1_w`).
        if let Some(sound) = &self.sound_cs {
            let (reply, strobe) = (sound.bus.reply(), sound.bus.reply_strobe());
            let pia5 = &mut self.board.pias[pia::WIDGET];
            pia5.set_port_b_input(reply);
            pia5.set_cb1(strobe);
        }
    }

    /// The diagnostic buttons on the door come in through PIA 2's CA1/CB1, and
    /// the board refreshes them when it raises the periodic interrupt
    /// (`s11.c:101-118`).
    fn update_diagnostic_lines(&mut self) {
        let asserted = self.periodic_irq();
        if asserted == self.periodic_was_asserted {
            return;
        }
        self.periodic_was_asserted = asserted;

        let pia2 = &mut self.board.pias[pia::DISPLAY_SELECT];
        if asserted {
            let (advance, up) = (self.board.diagnostic_advance, self.board.diagnostic_up);
            pia2.set_ca1(advance);
            pia2.set_cb1(up);
        } else {
            pia2.set_ca1(false);
            pia2.set_cb1(false);
        }
    }
}
