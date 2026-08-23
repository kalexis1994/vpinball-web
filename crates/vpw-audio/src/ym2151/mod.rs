//! Yamaha YM2151 (OPM), the FM synthesiser of the CS sound board.
//!
//! Eight channels of four operators each: 32 oscillators. Each operator is a
//! sine wave with its own ADSR envelope, and what makes them interesting is
//! that **the output of one modulates the phase of the next**. Eight
//! "algorithms" define how they are chained together, from all four in series
//! to all four in parallel, and the first operator of each channel also feeds
//! back into itself.
//!
//! # What that sounds like
//!
//! Modulating the phase of a sine wave with another sine wave generates
//! sidebands: a rich spectrum out of two cheap oscillators. That is the whole
//! idea of FM synthesis, and why a chip from 1987 could do brass and bells
//! that cost a subtractive synthesiser a great deal more.
//!
//! # Origin
//!
//! Ported from PinMAME's `src/sound/ym2151.c`, which is under
//! **GPL-2.0-or-later**. The "or later" makes it compatible with this
//! project's GPLv3.
//!
//! # Scope
//!
//! The eight algorithms are there, the feedback, the envelopes with their key
//! scaling, the two timers, the LFO with its four waveforms and the noise
//! generator of channel 8.

mod tables;

use tables::{
    DT1_TAB, DT2_TAB, EG_INC, EG_RATE_SELECT, EG_RATE_SHIFT, LFO_NOISE_WAVEFORM, PHASE_INC_ROM,
};

// --- Chip constants --------------------------------------------------------------

/// 16.16 fixed point for the frequency calculations.
const FREQ_SH: u32 = 16;
const FREQ_MASK: u32 = (1 << FREQ_SH) - 1;

/// The envelope works with 10 bits of attenuation.
const ENV_BITS: u32 = 10;
const ENV_LEN: u32 = 1 << ENV_BITS;
const ENV_STEP: f64 = 128.0 / ENV_LEN as f64;

/// Total silence.
const MAX_ATT_INDEX: i32 = ENV_LEN as i32 - 1;
/// Maximum volume.
const MIN_ATT_INDEX: i32 = 0;

const SIN_BITS: u32 = 10;
const SIN_LEN: usize = 1 << SIN_BITS;
const SIN_MASK: u32 = SIN_LEN as u32 - 1;

const TL_RES_LEN: usize = 256;
const TL_TAB_LEN: usize = 13 * 2 * TL_RES_LEN;
/// Above this attenuation the operator contributes nothing audible, so it is
/// not even calculated. It is the optimisation the real chip makes.
const ENV_QUIET: u32 = TL_TAB_LEN as u32 / 8;

const RATE_STEPS: usize = 8;

/// 22.10 fixed point for the LFO timing.
const LFO_SH: u32 = 10;

/// Waveform of the low frequency oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoWave {
    /// Sawtooth.
    Sawtooth,
    /// Square: it jumps between the two extremes.
    Square,
    /// Triangle.
    Triangle,
    /// Noise. The chip's real algorithm is unknown; a capture from a real one
    /// is used instead.
    Noise,
}

/// Envelope state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EgState {
    Off,
    Release,
    Sustain,
    Decay,
    Attack,
}

/// Where an operator's output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connect {
    /// Modulates operator C1.
    C1,
    /// Modulates operator C2.
    C2,
    /// Goes into the one-sample delay memory.
    Mem,
    /// Modulates operator M2.
    M2,
    /// Goes straight out of the channel output.
    ChanOut,
    /// Special marker of algorithm 5: it feeds all three at once.
    All,
}

// --- Computed tables ---------------------------------------------------------------

/// Attenuation table: converts from the logarithmic domain to the linear one.
///
/// The chip works entirely in "decibels" —adding attenuations is cheaper than
/// multiplying amplitudes— and only converts at the very end. The thirteen
/// blocks are the successive 3 dB shifts.
fn build_tl_table() -> Vec<i32> {
    let mut tl = vec![0i32; TL_TAB_LEN];
    for x in 0..TL_RES_LEN {
        let m =
            ((1u32 << 16) as f64 / 2f64.powf((x as f64 + 1.0) * (ENV_STEP / 4.0) / 8.0)).floor();
        let mut n = m as i32 >> 4;
        // Round to nearest.
        n = if n & 1 != 0 { (n >> 1) + 1 } else { n >> 1 };
        n <<= 2;

        tl[x * 2] = n;
        tl[x * 2 + 1] = -n;
        for i in 1..13 {
            tl[x * 2 + i * 2 * TL_RES_LEN] = n >> i;
            tl[x * 2 + 1 + i * 2 * TL_RES_LEN] = -(n >> i);
        }
    }
    tl
}

/// Sine wave on a logarithmic scale. Each entry is an attenuation, and the low
/// bit carries the sign.
fn build_sin_table() -> Vec<u32> {
    let mut sin = vec![0u32; SIN_LEN];
    for (i, slot) in sin.iter_mut().enumerate() {
        // The sine is offset by half a step: the real chip never evaluates the
        // sine at exactly zero, which spares it an infinity when going to
        // logarithms.
        let m = (((i * 2) + 1) as f64 * (std::f64::consts::PI / SIN_LEN as f64)).sin();
        let o = 8.0 * (1.0 / m.abs()).ln() / 2f64.ln() / (ENV_STEP / 4.0);

        let mut n = (2.0 * o) as i32;
        n = if n & 1 != 0 { (n >> 1) + 1 } else { n >> 1 };
        *slot = (n * 2 + i32::from(m < 0.0)) as u32;
    }
    sin
}

/// Level at which the envelope goes from decay to sustain.
fn build_d1l_table() -> [u32; 16] {
    std::array::from_fn(|i| {
        // Each step is 3 dB, except the last one which goes all the way to
        // silence.
        let level = if i != 15 { i } else { i + 16 };
        (level as f64 * (4.0 / ENV_STEP)) as u32
    })
}

/// Phase increments per note and octave, in fixed point.
fn build_freq_table() -> Vec<u32> {
    // Eleven octaves: the -1, the eight real ones and two padding ones on top.
    let mut freq = vec![0u32; 11 * 768];

    for i in 0..768 {
        // Octave 2 is the reference one; the rest comes from shifting it.
        let base = (u32::from(PHASE_INC_ROM[i]) << (FREQ_SH - 10)) & 0xFFFF_FFC0;
        freq[768 + 2 * 768 + i] = base;
        for j in 0..2 {
            freq[768 + j * 768 + i] = (base >> (2 - j)) & 0xFFFF_FFC0;
        }
        for j in 3..8 {
            freq[768 + j * 768 + i] = base << (j - 2);
        }
    }

    // Octave -1: all of it equal to the lowest note.
    let lowest = freq[768];
    for slot in freq.iter_mut().take(768) {
        *slot = lowest;
    }
    // Octaves 8 and 9: all of them equal to the highest note.
    let highest = freq[768 + 8 * 768 - 1];
    for j in 8..10 {
        for i in 0..768 {
            freq[768 + j * 768 + i] = highest;
        }
    }
    freq
}

/// Detune 1 offsets, with their negative versions.
fn build_dt1_table() -> [i32; 8 * 32] {
    let mut dt1 = [0i32; 8 * 32];
    for j in 0..4 {
        for i in 0..32 {
            let value = (i32::from(DT1_TAB[j * 32 + i]) * SIN_LEN as i32) >> (20 - FREQ_SH);
            dt1[j * 32 + i] = value;
            dt1[(j + 4) * 32 + i] = -value;
        }
    }
    dt1
}

// --- The operator ------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Operator {
    /// Accumulated phase, in fixed point.
    phase: u32,
    /// Phase increment per sample.
    freq: u32,
    dt1: i32,
    mul: u32,
    dt1_index: u32,
    dt2: u32,

    connect: Connect,
    /// Only operator M1 uses this: where the delayed sample goes.
    mem_connect: Connect,
    mem_value: i32,

    /// Feedback, only on M1.
    fb_shift: u32,
    fb_out_curr: i32,
    fb_out_prev: i32,

    /// Key code of the channel, seven bits, copied to all four operators.
    kc: u32,
    kc_index: u32,
    /// Sensitivity to the LFO's phase and amplitude modulation.
    pms: u32,
    ams: u32,
    /// Enable mask for the amplitude modulation. It is either all ones or all
    /// zeros, so it can be applied without a branch.
    am_mask: u32,

    state: EgState,
    /// Programmed total level.
    tl: u32,
    /// Current attenuation of the envelope.
    volume: i32,
    /// Level where the decay ends.
    d1l: u32,

    eg_sh_ar: u8,
    eg_sel_ar: u16,
    eg_sh_d1r: u8,
    eg_sel_d1r: u16,
    eg_sh_d2r: u8,
    eg_sel_d2r: u16,
    eg_sh_rr: u8,
    eg_sel_rr: u16,

    key: bool,
    ks: u32,
    ar: u32,
    d1r: u32,
    d2r: u32,
    rr: u32,
}

impl Operator {
    fn new() -> Self {
        Self {
            phase: 0,
            freq: 0,
            dt1: 0,
            mul: 0,
            dt1_index: 0,
            dt2: 0,
            connect: Connect::ChanOut,
            mem_connect: Connect::Mem,
            mem_value: 0,
            fb_shift: 0,
            fb_out_curr: 0,
            fb_out_prev: 0,
            kc: 0,
            kc_index: 0,
            pms: 0,
            ams: 0,
            am_mask: 0,
            state: EgState::Off,
            tl: 0,
            volume: MAX_ATT_INDEX,
            d1l: 0,
            eg_sh_ar: 0,
            eg_sel_ar: 0,
            eg_sh_d1r: 0,
            eg_sel_d1r: 0,
            eg_sh_d2r: 0,
            eg_sel_d2r: 0,
            eg_sh_rr: 0,
            eg_sel_rr: 0,
            key: false,
            ks: 0,
            ar: 0,
            d1r: 0,
            d2r: 0,
            rr: 0,
        }
    }

    /// Recomputes the effective rates of the envelope.
    ///
    /// The rates depend on the note: on a real instrument high notes die away
    /// faster than low ones, and the chip imitates that by adding to the
    /// programmed rate a term derived from the note (the *key scaling*).
    fn refresh_eg(&mut self) {
        let v = self.kc >> self.ks;

        if self.ar + v < 32 + 62 {
            self.eg_sh_ar = EG_RATE_SHIFT[(self.ar + v) as usize];
            self.eg_sel_ar = EG_RATE_SELECT[(self.ar + v) as usize];
        } else {
            // Instantaneous attack.
            self.eg_sh_ar = 0;
            self.eg_sel_ar = (17 * RATE_STEPS) as u16;
        }
        self.eg_sh_d1r = EG_RATE_SHIFT[(self.d1r + v) as usize];
        self.eg_sel_d1r = EG_RATE_SELECT[(self.d1r + v) as usize];
        self.eg_sh_d2r = EG_RATE_SHIFT[(self.d2r + v) as usize];
        self.eg_sel_d2r = EG_RATE_SELECT[(self.d2r + v) as usize];
        self.eg_sh_rr = EG_RATE_SHIFT[(self.rr + v) as usize];
        self.eg_sel_rr = EG_RATE_SELECT[(self.rr + v) as usize];
    }

    fn key_on(&mut self, eg_cnt: u32) {
        if !self.key {
            self.phase = 0;
            self.state = EgState::Attack;
            // The attack is not linear: it approaches the maximum
            // proportionally, which is what gives it its characteristic curve.
            let inc = EG_INC[(self.eg_sel_ar + ((eg_cnt >> self.eg_sh_ar) & 7) as u16) as usize];
            self.volume += (!self.volume * i32::from(inc)) >> 4;
            if self.volume <= MIN_ATT_INDEX {
                self.volume = MIN_ATT_INDEX;
                self.state = EgState::Decay;
            }
        }
        self.key = true;
    }

    fn key_off(&mut self) {
        if self.key {
            self.key = false;
            if self.state > EgState::Release {
                self.state = EgState::Release;
            }
        }
    }

    /// Total attenuation this operator comes out with right now.
    ///
    /// The tremolo adds attenuation, it does not subtract it: the LFO makes
    /// the operator quieter, never louder than its programmed level.
    #[inline]
    fn volume_calc(&self, am: u32) -> u32 {
        self.tl + self.volume as u32 + (am & self.am_mask)
    }
}

// So that states can be compared with `>` the way the chip does.
impl PartialOrd for EgState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.rank().cmp(&other.rank()))
    }
}

impl EgState {
    fn rank(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Release => 1,
            Self::Sustain => 2,
            Self::Decay => 3,
            Self::Attack => 4,
        }
    }
}

// --- The chip ------------------------------------------------------------------------

/// A YM2151.
pub struct Ym2151 {
    ops: Vec<Operator>,

    /// Output of each channel.
    chanout: [i32; 8],
    /// Phase modulation inputs of operators 2, 3 and 4.
    m2: i32,
    c1: i32,
    c2: i32,
    /// One-sample delay memory.
    mem: i32,

    /// The 256 registers, exactly as they were written.
    regs: [u8; 256],
    /// Register pointed at by the address port.
    address: u8,

    /// Envelope counter and its timing.
    eg_cnt: u32,
    eg_timer: u32,
    eg_timer_add: u32,
    eg_timer_overflow: u32,

    /// Which channels have their output connected to left and right.
    pan: [bool; 16],

    /// Indices of the two timers, exactly as they are programmed.
    timer_a_index: u32,
    timer_b_index: u32,
    /// Countdowns, in generated samples.
    timer_a_counter: i32,
    timer_b_counter: i32,
    timer_a_running: bool,
    timer_b_running: bool,
    // --- Low frequency oscillator ---
    /// LFO phase, from 0 to 255.
    lfo_phase: u32,
    lfo_timer: u32,
    lfo_timer_add: u32,
    lfo_overflow: u32,
    lfo_counter: u32,
    lfo_counter_add: u32,
    lfo_wave: LfoWave,
    /// Depth of the amplitude and phase modulation.
    amd: u32,
    pmd: i32,
    /// LFO outputs already scaled by their depth.
    lfa: u32,
    lfp: i32,

    // --- Noise generator ---
    /// Register `$0F`: bit 7 enables it and bits 0-4 give its period.
    noise: u8,
    /// 17-bit shift register.
    noise_rng: u32,
    /// Count that decides when to shift it.
    noise_phase: u32,
    /// Period, in counts.
    noise_period: u32,

    /// Bits 0 and 1: flags of timers A and B.
    status: u8,
    /// Register `$14`: which timers generate an interrupt.
    irq_enable: u8,

    // Tables: the ones computed once and the ones that depend on the clock.
    tl_tab: Vec<i32>,
    sin_tab: Vec<u32>,
    d1l_tab: [u32; 16],
    freq: Vec<u32>,
    dt1_freq: [i32; 8 * 32],
}

impl Ym2151 {
    /// Creates a chip with its input clock and the desired sample rate.
    ///
    /// On the System 11 CS board the clock is 3.579545 MHz, the classic NTSC
    /// colour crystal.
    pub fn new(clock_hz: u32, sample_rate: u32) -> Self {
        let mut chip = Self {
            ops: (0..32).map(|_| Operator::new()).collect(),
            chanout: [0; 8],
            m2: 0,
            c1: 0,
            c2: 0,
            mem: 0,
            regs: [0; 256],
            address: 0,
            eg_cnt: 0,
            eg_timer: 0,
            eg_timer_add: 0,
            eg_timer_overflow: 0,
            pan: [true; 16],
            timer_a_index: 0,
            timer_b_index: 0,
            timer_a_counter: 0,
            timer_b_counter: 0,
            timer_a_running: false,
            timer_b_running: false,
            lfo_phase: 0,
            lfo_timer: 0,
            lfo_timer_add: 0,
            lfo_overflow: 0,
            lfo_counter: 0,
            lfo_counter_add: 0,
            lfo_wave: LfoWave::Sawtooth,
            amd: 0,
            pmd: 0,
            lfa: 0,
            lfp: 0,
            noise: 0,
            noise_rng: 0,
            noise_phase: 0,
            noise_period: 32,
            status: 0,
            irq_enable: 0,
            tl_tab: build_tl_table(),
            sin_tab: build_sin_table(),
            d1l_tab: build_d1l_table(),
            freq: build_freq_table(),
            dt1_freq: build_dt1_table(),
        };
        chip.set_rates(clock_hz, sample_rate);
        chip.reset();
        chip
    }

    /// Adjusts the timing to the clock and the sample rate.
    ///
    /// The chip divides its clock by 64 to generate a sample, so if the output
    /// rate does not match you have to resample: `eg_timer_add` is how much
    /// the internal clock advances for each sample that is asked of it.
    pub fn set_rates(&mut self, clock_hz: u32, sample_rate: u32) {
        let chip_rate = f64::from(clock_hz) / 64.0;
        let ratio = chip_rate / f64::from(sample_rate);
        self.eg_timer_add = ((1u64 << FREQ_SH) as f64 * ratio) as u32;
        self.eg_timer_overflow = 3 << FREQ_SH;
        self.lfo_timer_add = ((1u64 << LFO_SH) as f64 * ratio) as u32;
    }

    /// Reset: all the operators silent and the registers at zero.
    pub fn reset(&mut self) {
        for op in &mut self.ops {
            *op = Operator::new();
        }
        self.chanout = [0; 8];
        self.m2 = 0;
        self.c1 = 0;
        self.c2 = 0;
        self.mem = 0;
        self.eg_cnt = 0;
        self.eg_timer = 0;
        self.regs = [0; 256];
        self.pan = [true; 16];
        self.timer_a_index = 0;
        self.timer_b_index = 0;
        self.timer_a_counter = 0;
        self.timer_b_counter = 0;
        self.timer_a_running = false;
        self.timer_b_running = false;
        self.status = 0;
        self.irq_enable = 0;
        self.lfo_phase = 0;
        self.lfo_timer = 0;
        self.lfo_counter = 0;
        self.lfo_counter_add = 0;
        self.lfo_overflow = 0;
        self.lfo_wave = LfoWave::Sawtooth;
        self.amd = 0;
        self.pmd = 0;
        self.lfa = 0;
        self.lfp = 0;
        self.noise = 0;
        self.noise_rng = 0;
        self.noise_phase = 0;
        self.noise_period = 32;

        // Silence everything and leave the rates coherent.
        for i in 0..32 {
            self.ops[i].volume = MAX_ATT_INDEX;
            self.ops[i].state = EgState::Off;
            self.ops[i].refresh_eg();
        }
        for channel in 0..8 {
            self.set_connect(channel, 0);
        }
    }

    /// Address port: chooses which register is going to receive the next
    /// datum.
    pub fn write_address(&mut self, value: u8) {
        self.address = value;
    }

    /// Data port: writes the register being pointed at.
    pub fn write_data(&mut self, value: u8) {
        let reg = self.address;
        self.regs[reg as usize] = value;

        match reg & 0xE0 {
            // --- Global registers ---
            0x00 => self.write_global(reg, value),
            // --- Per channel ---
            0x20 => self.write_channel(reg, value),
            // --- Per operator ---
            _ => self.write_operator(reg, value),
        }
    }

    fn write_global(&mut self, reg: u8, value: u8) {
        match reg {
            // Timer A: eight high bits and two low ones.
            0x10 => {
                self.timer_a_index = (self.timer_a_index & 0x003) | (u32::from(value) << 2);
                return;
            }
            0x11 => {
                self.timer_a_index = (self.timer_a_index & 0x3FC) | u32::from(value & 3);
                return;
            }
            // Timer B: eight bits.
            0x12 => {
                self.timer_b_index = u32::from(value);
                return;
            }
            // Noise enable and period.
            0x0F => {
                self.noise = value;
                // Rates 30 and 31 give the same thing, and the period never
                // goes below 2.
                self.noise_period = (32 - u32::from(value & 0x1F)).max(2);
                return;
            }
            0x14 => {
                self.write_timer_control(value);
                return;
            }
            // LFO frequency. The high nibble picks the range in powers of two
            // and the low one tunes within that range.
            0x18 => {
                self.lfo_overflow = (1 << ((15 - u32::from(value >> 4)) + 3)) << LFO_SH;
                self.lfo_counter_add = 0x10 + u32::from(value & 0x0F);
                return;
            }
            // Depth: bit 7 chooses whether the value is for the phase or for
            // the amplitude.
            0x19 => {
                if value & 0x80 != 0 {
                    self.pmd = i32::from(value & 0x7F);
                } else {
                    self.amd = u32::from(value & 0x7F);
                }
                return;
            }
            // LFO waveform.
            0x1B => {
                self.lfo_wave = match value & 3 {
                    0 => LfoWave::Sawtooth,
                    1 => LfoWave::Square,
                    2 => LfoWave::Triangle,
                    _ => LfoWave::Noise,
                };
                return;
            }
            _ => {}
        }

        if reg == 0x08 {
            // Key on/off: bits 3-6 choose which operators of the channel.
            let channel = (value & 7) as usize;
            let base = channel * 4;
            for (i, bit) in [0x08u8, 0x10, 0x20, 0x40].into_iter().enumerate() {
                // The order of the operators in the register is M1, M2, C1, C2,
                // which is not the order they sit in memory.
                let op = base + [0, 2, 1, 3][i];
                if value & bit != 0 {
                    let cnt = self.eg_cnt;
                    self.ops[op].key_on(cnt);
                } else {
                    self.ops[op].key_off();
                }
            }
        }
        // The rest of the globals (test, LFO, timers, noise) stay stored in
        // `regs` but have no effect yet.
    }

    /// Register `$14`: starts, stops and clears the timers.
    fn write_timer_control(&mut self, value: u8) {
        self.irq_enable = value;

        // Bits 4 and 5 clear the flags. It is how the driver acknowledges the
        // interrupt: without this it would keep firing forever.
        if value & 0x10 != 0 {
            self.status &= !0x01;
        }
        if value & 0x20 != 0 {
            self.status &= !0x02;
        }

        // Bits 0 and 1 start them. Starting one that was already running does
        // not restart it.
        let start_a = value & 0x01 != 0;
        if start_a && !self.timer_a_running {
            self.timer_a_counter = self.timer_a_period();
        }
        self.timer_a_running = start_a;

        let start_b = value & 0x02 != 0;
        if start_b && !self.timer_b_running {
            self.timer_b_counter = self.timer_b_period();
        }
        self.timer_b_running = start_b;
    }

    /// Period of timer A, in samples.
    ///
    /// The chip counts it in 64 clocks per step, and since one sample is
    /// exactly 64 clocks, the period in samples is directly
    /// `1024 - index`.
    fn timer_a_period(&self) -> i32 {
        1024 - self.timer_a_index as i32
    }

    /// Period of timer B: 1024 clocks per step, that is, 16 samples.
    fn timer_b_period(&self) -> i32 {
        16 * (256 - self.timer_b_index as i32)
    }

    /// Advances the timers by one sample.
    fn advance_timers(&mut self) {
        if self.timer_a_running {
            self.timer_a_counter -= 1;
            if self.timer_a_counter <= 0 {
                self.timer_a_counter += self.timer_a_period();
                if self.irq_enable & 0x04 != 0 {
                    self.status |= 0x01;
                }
            }
        }
        if self.timer_b_running {
            self.timer_b_counter -= 1;
            if self.timer_b_counter <= 0 {
                self.timer_b_counter += self.timer_b_period();
                if self.irq_enable & 0x08 != 0 {
                    self.status |= 0x02;
                }
            }
        }
    }

    /// Status register: the timer flags.
    pub fn read_status(&self) -> u8 {
        self.status
    }

    /// Whether the chip is requesting an interrupt.
    ///
    /// On the CS board this line goes to the PIA's CA1, and it is what gives
    /// the music sequencer its time base: without timers the tune does not
    /// advance.
    pub fn irq(&self) -> bool {
        self.status & 0x03 != 0
    }

    fn write_channel(&mut self, reg: u8, value: u8) {
        let channel = (reg & 7) as usize;
        let base = channel * 4;

        match reg & 0x18 {
            // $20-$27: panning, feedback and algorithm.
            0x00 => {
                self.pan[channel * 2] = value & 0x40 != 0;
                self.pan[channel * 2 + 1] = value & 0x80 != 0;

                // The feedback is a shift: 0 turns it off and 7 is the
                // maximum.
                let fb = (value >> 3) & 7;
                self.ops[base].fb_shift = if fb != 0 { u32::from(fb) + 6 } else { 0 };
                self.set_connect(channel, value & 7);
            }
            // $28-$2F: note (key code).
            0x08 => {
                let kc = u32::from(value & 0x7F);
                // The note field has four codes for every group of three
                // semitones, so one in every four does not exist. Subtracting
                // `kc >> 2` discards exactly those and leaves a continuous
                // index, with no need for a table or special cases.
                let kc_index = (kc - (kc >> 2)) * 64 + 768;
                for i in 0..4 {
                    let op = &mut self.ops[base + i];
                    op.kc = kc;
                    // Whatever key fraction was already set is preserved.
                    op.kc_index = kc_index | (op.kc_index & 63);
                    op.refresh_eg();
                }
                self.refresh_channel_freq(channel);
            }
            // $30-$37: key fraction.
            0x10 => {
                let kf = u32::from(value >> 2);
                for i in 0..4 {
                    let op = &mut self.ops[base + i];
                    op.kc_index = (op.kc_index & !63) | kf;
                }
                self.refresh_channel_freq(channel);
            }
            // $38-$3F: LFO sensitivity.
            _ => {
                for i in 0..4 {
                    self.ops[base + i].pms = u32::from((value >> 4) & 7);
                    self.ops[base + i].ams = u32::from(value & 3);
                }
            }
        }
    }

    fn write_operator(&mut self, reg: u8, value: u8) {
        // The operators are addressed interleaved: the register picks the
        // channel in the three low bits and the operator in the next two.
        let channel = (reg & 7) as usize;
        let slot = ((reg >> 3) & 3) as usize;
        let index = channel * 4 + slot;

        match reg & 0xE0 {
            // $40-$5F: detune 1 and frequency multiplier.
            0x40 => {
                let op = &mut self.ops[index];
                op.dt1_index = u32::from((value >> 4) & 7) * 32;
                // Multiplier 0 means half frequency, not zero.
                op.mul = if value & 0x0F != 0 {
                    u32::from(value & 0x0F) * 2
                } else {
                    1
                };
                self.refresh_channel_freq(channel);
            }
            // $60-$7F: total level.
            0x60 => {
                self.ops[index].tl = u32::from(value & 0x7F) << (ENV_BITS - 7);
            }
            // $80-$9F: key scaling and attack rate.
            0x80 => {
                let op = &mut self.ops[index];
                op.ks = 5 - u32::from(value >> 6);
                op.ar = if value & 0x1F != 0 {
                    32 + (u32::from(value & 0x1F) << 1)
                } else {
                    0
                };
                op.refresh_eg();
            }
            // $A0-$BF: tremolo enable and first decay rate.
            0xA0 => {
                let op = &mut self.ops[index];
                // All ones or all zeros, so it can be applied without a branch.
                op.am_mask = if value & 0x80 != 0 { u32::MAX } else { 0 };
                op.d1r = if value & 0x1F != 0 {
                    32 + (u32::from(value & 0x1F) << 1)
                } else {
                    0
                };
                op.refresh_eg();
            }
            // $C0-$DF: detune 2 and second decay rate.
            0xC0 => {
                let op = &mut self.ops[index];
                op.dt2 = DT2_TAB[(value >> 6) as usize];
                op.d2r = if value & 0x1F != 0 {
                    32 + (u32::from(value & 0x1F) << 1)
                } else {
                    0
                };
                op.refresh_eg();
                self.refresh_channel_freq(channel);
            }
            // $E0-$FF: sustain level and release rate.
            _ => {
                let d1l = self.d1l_tab[(value >> 4) as usize];
                let op = &mut self.ops[index];
                op.d1l = d1l;
                // The release rate has half the resolution.
                op.rr = 34 + (u32::from(value & 0x0F) << 2);
                op.refresh_eg();
            }
        }
    }

    /// Recomputes the phase increments of the channel's four operators.
    fn refresh_channel_freq(&mut self, channel: usize) {
        let base = channel * 4;
        for i in 0..4 {
            let op = &self.ops[base + i];
            let kc_index = (op.kc_index + op.dt2) as usize;
            // The detune is indexed with the note at semitone resolution,
            // which is the six high bits of the key code.
            let dt1 = self.dt1_freq[(op.dt1_index + (op.kc >> 2)) as usize];
            let freq =
                ((self.freq[kc_index.min(self.freq.len() - 1)] as i32 + dt1) as u32 * op.mul) >> 1;
            self.ops[base + i].freq = freq;
            self.ops[base + i].dt1 = dt1;
        }
    }

    /// Wires up the channel's four operators according to the chosen
    /// algorithm.
    ///
    /// The eight algorithms range from all four operators in a chain —where
    /// each one modulates the next and only the last is heard— to all four in
    /// parallel, which is almost an additive organ.
    fn set_connect(&mut self, channel: usize, algorithm: u8) {
        let base = channel * 4;
        let (m1, m2, c1) = (base, base + 1, base + 2);

        let (m1_conn, c1_conn, m2_conn, m1_mem) = match algorithm & 7 {
            // M1 -> C1 -> MEM -> M2 -> C2 -> output
            0 => (Connect::C1, Connect::Mem, Connect::C2, Connect::M2),
            // M1 and C1 together -> MEM -> M2 -> C2
            1 => (Connect::Mem, Connect::Mem, Connect::C2, Connect::M2),
            // M1 and (C1 -> MEM -> M2) together -> C2
            2 => (Connect::C2, Connect::Mem, Connect::C2, Connect::M2),
            // M1 -> C1 -> MEM, and that plus M2 -> C2
            3 => (Connect::C1, Connect::Mem, Connect::C2, Connect::C2),
            // Two chains in parallel: M1->C1 and M2->C2
            4 => (Connect::C1, Connect::ChanOut, Connect::C2, Connect::Mem),
            // M1 modulates all three, which sound together
            5 => (
                Connect::All,
                Connect::ChanOut,
                Connect::ChanOut,
                Connect::M2,
            ),
            // M1->C1, plus M2 and C2 on their own
            6 => (
                Connect::C1,
                Connect::ChanOut,
                Connect::ChanOut,
                Connect::Mem,
            ),
            // All four in parallel
            _ => (
                Connect::ChanOut,
                Connect::ChanOut,
                Connect::ChanOut,
                Connect::Mem,
            ),
        };

        self.ops[m1].connect = m1_conn;
        self.ops[m1].mem_connect = m1_mem;
        self.ops[c1].connect = c1_conn;
        self.ops[m2].connect = m2_conn;
    }

    // --- Synthesis --------------------------------------------------------------------

    /// Output of an operator: a phase-modulated sine wave, attenuated by the
    /// envelope.
    #[inline]
    fn op_calc(&self, index: usize, env: u32, pm: i32) -> i32 {
        let op = &self.ops[index];
        let phase = ((op.phase & !FREQ_MASK) as i32).wrapping_add(pm << 15);
        let p = (env << 3) + self.sin_tab[((phase >> FREQ_SH) as u32 & SIN_MASK) as usize];
        if p as usize >= TL_TAB_LEN {
            0
        } else {
            self.tl_tab[p as usize]
        }
    }

    /// The same but for the feedback, which does not shift the modulation.
    #[inline]
    fn op_calc_feedback(&self, index: usize, env: u32, pm: i32) -> i32 {
        let op = &self.ops[index];
        let phase = ((op.phase & !FREQ_MASK) as i32).wrapping_add(pm);
        let p = (env << 3) + self.sin_tab[((phase >> FREQ_SH) as u32 & SIN_MASK) as usize];
        if p as usize >= TL_TAB_LEN {
            0
        } else {
            self.tl_tab[p as usize]
        }
    }

    fn route(&mut self, dest: Connect, channel: usize, value: i32) {
        match dest {
            Connect::C1 => self.c1 += value,
            Connect::C2 => self.c2 += value,
            Connect::Mem => self.mem += value,
            Connect::M2 => self.m2 += value,
            Connect::ChanOut => self.chanout[channel] += value,
            Connect::All => {
                self.mem += value;
                self.c1 += value;
                self.c2 += value;
            }
        }
    }

    fn set_route(&mut self, dest: Connect, channel: usize, value: i32) {
        match dest {
            Connect::C1 => self.c1 = value,
            Connect::C2 => self.c2 = value,
            Connect::Mem => self.mem = value,
            Connect::M2 => self.m2 = value,
            Connect::ChanOut => self.chanout[channel] = value,
            Connect::All => {
                self.mem = value;
                self.c1 = value;
                self.c2 = value;
            }
        }
    }

    /// Computes the output of a channel.
    fn chan_calc(&mut self, channel: usize) {
        let base = channel * 4;
        // The tremolo is scaled by the channel's sensitivity before being
        // added to the attenuation of every operator that has it enabled.
        let ams = self.ops[base].ams;
        let am = if ams != 0 { self.lfa << (ams - 1) } else { 0 };
        self.m2 = 0;
        self.c1 = 0;
        self.c2 = 0;
        self.mem = 0;

        // The delayed sample from the previous round comes in now.
        let mem_dest = self.ops[base].mem_connect;
        let mem_value = self.ops[base].mem_value;
        self.set_route(mem_dest, channel, mem_value);

        // M1, with its feedback.
        {
            let env = self.ops[base].volume_calc(am);
            let out = self.ops[base].fb_out_prev + self.ops[base].fb_out_curr;
            self.ops[base].fb_out_prev = self.ops[base].fb_out_curr;

            let dest = self.ops[base].connect;
            let prev = self.ops[base].fb_out_prev;
            self.set_route(dest, channel, prev);

            self.ops[base].fb_out_curr = 0;
            if env < ENV_QUIET {
                let shift = self.ops[base].fb_shift;
                let modulation = if shift != 0 { out << shift } else { 0 };
                self.ops[base].fb_out_curr = self.op_calc_feedback(base, env, modulation);
            }
        }

        // M2, C1 and C2, each one modulated by whatever reached it.
        for (offset, input) in [(1usize, self.m2), (2, self.c1)] {
            let env = self.ops[base + offset].volume_calc(am);
            if env < ENV_QUIET {
                let value = self.op_calc(base + offset, env, input);
                let dest = self.ops[base + offset].connect;
                self.route(dest, channel, value);
            }
        }

        // The last operator of channel 8 can put out noise instead of its
        // sine wave. It is the only place in the chip where the noise
        // generator gets in, and it is good for percussion and effects.
        let env = self.ops[base + 3].volume_calc(am);
        if channel == 7 && self.noise & 0x80 != 0 {
            let level = if env < 0x3FF {
                ((env ^ 0x3FF) * 2) as i32
            } else {
                0
            };
            // Bit 16 of the register decides the sign.
            self.chanout[7] += if self.noise_rng & 0x1_0000 != 0 {
                level
            } else {
                -level
            };
        } else if env < ENV_QUIET {
            let value = self.op_calc(base + 3, env, self.c2);
            self.chanout[channel] += value;
        }

        self.ops[base].mem_value = self.mem;
    }

    /// Advances the low frequency oscillator and computes its two outputs.
    ///
    /// The LFO is not heard by itself: it modulates the amplitude (tremolo)
    /// and the phase (vibrato) of the operators that have it enabled. It is
    /// what separates a flat tone from a living one.
    fn advance_lfo(&mut self) {
        self.lfo_timer += self.lfo_timer_add;
        if self.lfo_overflow != 0 && self.lfo_timer >= self.lfo_overflow {
            self.lfo_timer -= self.lfo_overflow;
            self.lfo_counter += self.lfo_counter_add;
            self.lfo_phase = (self.lfo_phase + (self.lfo_counter >> 4)) & 255;
            self.lfo_counter &= 15;
        }

        let i = self.lfo_phase as i32;
        // Each waveform gives a pair: `a` for the amplitude (0..255) and `p`
        // for the phase (-128..127).
        let (a, p) = match self.lfo_wave {
            LfoWave::Sawtooth => (255 - i, if i < 128 { i } else { i - 255 }),
            LfoWave::Square => {
                if i < 128 {
                    (255, 128)
                } else {
                    (0, -128)
                }
            }
            LfoWave::Triangle => {
                let a = if i < 128 { 255 - i * 2 } else { i * 2 - 256 };
                let p = if i < 64 {
                    i * 2
                } else if i < 128 {
                    255 - i * 2
                } else if i < 192 {
                    256 - i * 2
                } else {
                    i * 2 - 511
                };
                (a, p)
            }
            LfoWave::Noise => {
                let a = i32::from(LFO_NOISE_WAVEFORM[i as usize]);
                (a, a - 128)
            }
        };

        self.lfa = (a * self.amd as i32 / 128).max(0) as u32;
        self.lfp = p * self.pmd / 128;
    }

    /// Advances the noise generator.
    ///
    /// It is a 17-bit shift register with feedback: the input to bit 16 is the
    /// XNOR of bits 0 and 3. That is the whole secret of a pseudo-random noise
    /// generator of the era — a great deal of apparent randomness out of four
    /// gates.
    fn advance_noise(&mut self) {
        // The noise runs at twice the sample rate.
        self.noise_phase += 2;
        while self.noise_phase >= self.noise_period {
            self.noise_phase -= self.noise_period;
            let feedback = ((self.noise_rng ^ (self.noise_rng >> 3)) & 1) ^ 1;
            self.noise_rng = (feedback << 16) | (self.noise_rng >> 1);
        }
    }

    /// Advances the envelopes of the 32 operators.
    fn advance_eg(&mut self) {
        self.eg_timer += self.eg_timer_add;

        while self.eg_timer >= self.eg_timer_overflow {
            self.eg_timer -= self.eg_timer_overflow;
            self.eg_cnt = self.eg_cnt.wrapping_add(1);
            let cnt = self.eg_cnt;

            for op in &mut self.ops {
                match op.state {
                    EgState::Attack => {
                        if cnt & ((1 << op.eg_sh_ar) - 1) == 0 {
                            let inc =
                                EG_INC[(op.eg_sel_ar + ((cnt >> op.eg_sh_ar) & 7) as u16) as usize];
                            op.volume += (!op.volume * i32::from(inc)) >> 4;
                            if op.volume <= MIN_ATT_INDEX {
                                op.volume = MIN_ATT_INDEX;
                                op.state = EgState::Decay;
                            }
                        }
                    }
                    EgState::Decay => {
                        if cnt & ((1 << op.eg_sh_d1r) - 1) == 0 {
                            let inc = EG_INC
                                [(op.eg_sel_d1r + ((cnt >> op.eg_sh_d1r) & 7) as u16) as usize];
                            op.volume += i32::from(inc);
                            if op.volume >= op.d1l as i32 {
                                op.state = EgState::Sustain;
                            }
                        }
                    }
                    EgState::Sustain => {
                        if cnt & ((1 << op.eg_sh_d2r) - 1) == 0 {
                            let inc = EG_INC
                                [(op.eg_sel_d2r + ((cnt >> op.eg_sh_d2r) & 7) as u16) as usize];
                            op.volume += i32::from(inc);
                            if op.volume >= MAX_ATT_INDEX {
                                op.volume = MAX_ATT_INDEX;
                                op.state = EgState::Off;
                            }
                        }
                    }
                    EgState::Release => {
                        if cnt & ((1 << op.eg_sh_rr) - 1) == 0 {
                            let inc =
                                EG_INC[(op.eg_sel_rr + ((cnt >> op.eg_sh_rr) & 7) as u16) as usize];
                            op.volume += i32::from(inc);
                            if op.volume >= MAX_ATT_INDEX {
                                op.volume = MAX_ATT_INDEX;
                                op.state = EgState::Off;
                            }
                        }
                    }
                    EgState::Off => {}
                }
            }
        }
    }

    /// Advances the phase of the 32 operators, with vibrato if the channel
    /// asks for it.
    fn advance_phase(&mut self) {
        let limit = self.freq.len() - 1;

        for channel in 0..8 {
            let base = channel * 4;
            let pms = self.ops[base].pms;

            let mod_ind = if pms == 0 {
                0
            } else if pms < 6 {
                self.lfp >> (6 - pms)
            } else {
                self.lfp << (pms - 5)
            };

            // Without vibrato it is the simple path: add the usual increment.
            if mod_ind == 0 {
                for i in 0..4 {
                    let op = &mut self.ops[base + i];
                    op.phase = op.phase.wrapping_add(op.freq);
                }
                continue;
            }

            // With vibrato you have to look the increment up in the table
            // again, because the effective note has shifted.
            for i in 0..4 {
                let op = &self.ops[base + i];
                let index = ((op.kc_index as i32 + mod_ind) as u32 + op.dt2) as usize;
                let freq = ((self.freq[index.min(limit)] as i32 + op.dt1) as u32 * op.mul) >> 1;
                self.ops[base + i].phase = self.ops[base + i].phase.wrapping_add(freq);
            }
        }
    }

    /// Generates a stereo sample, normalised to `-1.0..1.0`.
    pub fn next_sample(&mut self) -> (f32, f32) {
        self.chanout = [0; 8];
        for channel in 0..8 {
            self.chan_calc(channel);
        }

        let mut left = 0i32;
        let mut right = 0i32;
        for (channel, &out) in self.chanout.iter().enumerate() {
            if self.pan[channel * 2] {
                left += out;
            }
            if self.pan[channel * 2 + 1] {
                right += out;
            }
        }

        self.advance_lfo();
        self.advance_noise();
        self.advance_eg();
        self.advance_phase();
        self.advance_timers();

        // Eight channels of signed 14 bits fit into this range with room to
        // spare.
        const SCALE: f32 = 1.0 / 32768.0;
        (
            (left as f32 * SCALE).clamp(-1.0, 1.0),
            (right as f32 * SCALE).clamp(-1.0, 1.0),
        )
    }

    /// Mono sample: the average of the two channels.
    pub fn next_sample_mono(&mut self) -> f32 {
        let (l, r) = self.next_sample();
        (l + r) / 2.0
    }

    /// Raw contents of a register.
    pub fn register(&self, index: u8) -> u8 {
        self.regs[index as usize]
    }

    /// How many operators are not silent. Useful for telling whether the chip
    /// is doing anything without having to look at the waveform.
    pub fn active_operators(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| op.state != EgState::Off)
            .count()
    }
}
