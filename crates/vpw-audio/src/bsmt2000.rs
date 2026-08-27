//! The BSMT2000, Data East and Sega's sample chip.
//!
//! Physically a TMS320C15 DSP with a mask ROM — the name stands for "Brian
//! Schmidt's Mouse Trap" — but nobody emulates the DSP: the firmware inside
//! never changed, so what the world sees is a register file, and that is what
//! is modelled, as PinMAME does (`src/sound/bsmt2000.c`, whose structure this
//! port follows).
//!
//! Eleven or twelve PCM voices play signed 8-bit samples straight out of up
//! to four megabytes of ROM, each with its own position, rate, loop and
//! stereo volumes; one extra voice decodes ADPCM nibbles for the long music
//! beds. The host CPU picks a register with one write and supplies sixteen
//! bits of data with another; everything else is mixing.
//!
//! The later Stern sound board dropped the chip and had an ARM emulate it in
//! firmware — that path is [`vpw_at91`] and runs the real firmware. This
//! module is for the boards that still carried the silicon: Sega and early
//! Stern Whitestar, and the Data East generations before them.

/// One bank is 64 KB of sample ROM; the bank register selects which.
const BANK: usize = 0x10000;

/// The register file of one voice, in the order the chip's own manual has
/// them (`bsmt2000.c:75-81`).
const CURRPOS: usize = 0;
const RATE: usize = 1;
const LOOPEND: usize = 2;
const LOOPSTART: usize = 3;
const BANK_REG: usize = 4;
const RIGHTVOL: usize = 5;
const LEFTVOL: usize = 6;
const REGS: usize = 7;

/// Where each mode's register groups start (`regmap`, `bsmt2000.c:41`). The
/// row is indexed by mode, the column by register kind; a write's offset
/// minus the group base is the voice number.
const REGMAP: [[u8; 7]; 8] = [
    [0x00, 0x18, 0x24, 0x30, 0x3c, 0x48, 0xff],
    [0x00, 0x16, 0x21, 0x2c, 0x37, 0x42, 0x4d],
    [0x00; 7],
    [0x00; 7],
    [0x00; 7],
    [0x00, 0x18, 0x24, 0x30, 0x3c, 0x54, 0x60],
    [0x00, 0x10, 0x18, 0x20, 0x28, 0x38, 0x40],
    [0x00, 0x12, 0x1b, 0x24, 0x2d, 0x3f, 0x48],
];

/// The voice that plays compressed data, one past the PCM ones.
const ADPCM_VOICE: usize = 12;

#[derive(Clone, Copy)]
struct Voice {
    reg: [u16; REGS],
    fraction: u16,
}

impl Voice {
    fn fresh() -> Self {
        let mut reg = [0u16; REGS];
        // Full volume both sides, as `init_voice` leaves them
        // (`bsmt2000.c:459`): a game that never touches a volume still hears
        // that voice.
        reg[LEFTVOL] = 0x7fff;
        reg[RIGHTVOL] = 0x7fff;
        Self { reg, fraction: 0 }
    }
}

pub struct Bsmt2000 {
    /// The sample ROM: every bank end to end, in the order the board wires
    /// them.
    rom: Vec<i8>,
    total_banks: u16,
    voices: [Voice; ADPCM_VOICE + 1],
    /// How many PCM voices the current mode plays.
    active_voices: usize,
    stereo: bool,
    adpcm: bool,
    mode: usize,
    /// The last register offset written, which is what the reset pin reads
    /// the new mode from (`set_mode`, `bsmt2000.c:159`).
    last_register: u8,
    /// Output rate in Hz, decided by the mode from the 24 MHz clock.
    sample_rate: f64,
    adpcm_current: i32,
    adpcm_delta_n: i32,
    /// The mystery register at 0x77: PinMAME treats it as a fallback volume
    /// for voices whose own volumes were never set (`bsmt2000.c:383`).
    adpcm_77: u16,
    /// Data East and Sega boards load the sample ROMs in a permuted bank
    /// order, and the chip's bank register is remapped on the fly to match
    /// (`de_rom_bank_remap`, `bsmt2000.c:151`).
    de_banking: bool,
    /// Monopoly and RollerCoaster Tycoon never set a right volume on their
    /// stereo hardware; until one is seen, the left one serves both sides
    /// (`bsmt2000.c:262`).
    right_volume_set: bool,
}

impl Bsmt2000 {
    /// Builds the chip over its sample ROM.
    ///
    /// `voices` is 11 for the Sega/early-Stern boards and 12 for the later
    /// ones — the count picks the initial mode exactly as PinMAME guesses it
    /// before the firmware's first reset (`bsmt2000.c:509`).
    pub fn new(rom: Vec<u8>, voices: u8) -> Self {
        let total_banks = (rom.len() / BANK) as u16;
        let mut chip = Self {
            rom: rom.into_iter().map(|b| b as i8).collect(),
            total_banks,
            voices: [Voice::fresh(); ADPCM_VOICE + 1],
            active_voices: usize::from(voices),
            stereo: true,
            adpcm: true,
            mode: 1,
            last_register: if voices == 11 { 1 } else { 5 },
            sample_rate: 24000.0,
            adpcm_current: 0,
            adpcm_delta_n: 10,
            adpcm_77: 0,
            de_banking: true,
            right_volume_set: false,
        };
        chip.reset();
        chip
    }

    /// The rate the chip makes samples at under the current mode.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// The reset pin: every voice back to fresh and the mode re-read from
    /// the last register offset the CPU touched (`BSMT2000_sh_reset`).
    pub fn reset(&mut self) {
        for v in &mut self.voices {
            *v = Voice::fresh();
        }
        // The compressed voice halts rather than starting clean: rate zero,
        // bank parked out of range (`reset_compression_flags`).
        self.voices[ADPCM_VOICE].reg[RATE] = 0;
        self.voices[ADPCM_VOICE].reg[BANK_REG] = 0xfe;

        // `set_mode`, from the 24 MHz clock. Only the modes pinball boards
        // use; an unknown last register keeps the mode it had.
        const CLOCK: f64 = 24_000_000.0;
        match self.last_register {
            1 => {
                self.sample_rate = CLOCK / 1000.0;
                self.stereo = true;
                self.active_voices = 11;
                self.adpcm = true;
                self.mode = 1;
            }
            5 => {
                self.sample_rate = CLOCK / 1000.0;
                self.stereo = true;
                self.active_voices = 12;
                self.adpcm = false;
                self.mode = 5;
            }
            6 => {
                self.sample_rate = CLOCK / 706.0;
                self.stereo = true;
                self.active_voices = 8;
                self.adpcm = false;
                self.mode = 6;
            }
            7 => {
                self.sample_rate = CLOCK / 750.0;
                self.stereo = true;
                self.active_voices = 9;
                self.adpcm = false;
                self.mode = 7;
            }
            _ => {}
        }
    }

    /// `(b&7) | ((b&0x18)<<1) | ((b&0x20)>>2)`: how a bank number on the bus
    /// finds its image in the permuted order the board's ROMs are wired in.
    fn remap_bank(&self, bank: u16) -> u16 {
        if self.de_banking {
            (bank & 0x07) | ((bank & 0x18) << 1) | ((bank & 0x20) >> 2)
        } else {
            bank
        }
    }

    /// A register write: `offset` picks the register, `data` is its new
    /// sixteen bits (`bsmt2000_reg_write`).
    pub fn write(&mut self, offset: u8, data: u16) {
        self.last_register = offset;
        if offset >= 0x80 {
            return;
        }

        if offset < 0x6d {
            // A PCM voice register: the group base decides which register,
            // the remainder decides which voice.
            let map = &REGMAP[self.mode];
            let mut regindex = 6;
            while offset < map[regindex] {
                regindex -= 1;
            }
            let voice_index = usize::from(offset - map[regindex]);
            if voice_index >= self.active_voices {
                return;
            }
            let banked = self.remap_bank(data);
            let voice = &mut self.voices[voice_index];
            voice.reg[regindex] = data;
            if regindex == BANK_REG {
                voice.reg[BANK_REG] = banked;
            }
            if regindex == RIGHTVOL {
                self.right_volume_set = true;
            }
            if regindex == CURRPOS {
                voice.fraction = 0;
            }
        } else if self.adpcm {
            let banked = self.remap_bank(data);
            let voice = &mut self.voices[ADPCM_VOICE];
            match offset {
                0x6d => voice.reg[LOOPEND] = data,
                0x6f => voice.reg[BANK_REG] = banked,
                0x73 => {
                    voice.reg[RATE] = data;
                    if data != 0 {
                        self.adpcm_current = 0;
                        self.adpcm_delta_n = 10;
                    }
                }
                0x6e | 0x74 => {
                    voice.reg[RIGHTVOL] = data;
                    self.right_volume_set = true;
                }
                0x75 => {
                    voice.reg[CURRPOS] = data;
                    voice.fraction = 0;
                }
                0x77 => self.adpcm_77 = data,
                0x70 | 0x78 if self.stereo => voice.reg[LEFTVOL] = data,
                _ => {}
            }
        }
    }

    /// Renders the next stretch of output at [`Self::sample_rate`].
    ///
    /// The port of `bsmt2000_update`: an accumulator wide enough for every
    /// voice at full volume, the compressed voice first, then the PCM ones,
    /// then the TMS's final `SACH` — take the top sixteen bits and saturate.
    pub fn render(&mut self, out: &mut [(i16, i16)]) {
        let mut left = vec![0i64; out.len()];
        let mut right = vec![0i64; out.len()];

        self.render_adpcm(&mut left, &mut right);

        for voicenum in 0..self.active_voices {
            let voice = &mut self.voices[voicenum];
            if voice.reg[BANK_REG] >= self.total_banks {
                continue;
            }
            let base = usize::from(voice.reg[BANK_REG]) * BANK;
            let rate = u32::from(voice.reg[RATE]);
            let mut rvol = i64::from(voice.reg[RIGHTVOL]);
            let mut lvol = if self.stereo {
                i64::from(voice.reg[LEFTVOL])
            } else {
                rvol
            };
            if self.stereo && !self.right_volume_set {
                rvol = lvol;
            }
            if self.adpcm_77 > 0 && rvol == 0 && lvol == 0 {
                rvol = i64::from(self.adpcm_77);
                lvol = rvol;
            }
            let mut pos = u32::from(voice.reg[CURRPOS]);
            let mut frac = u32::from(voice.fraction);

            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let sample = i64::from(self.rom[base + pos as usize]) << 8;
                *l += sample * lvol;
                *r += sample * rvol;

                frac += rate;
                pos += frac >> 11;
                frac &= 0x7ff;

                if pos >= u32::from(voice.reg[LOOPEND]) {
                    // "Looks whacky, but it seems to be correct like this"
                    // (`bsmt2000.c:410`): the overshoot carries into the loop.
                    pos = pos
                        .wrapping_add(u32::from(voice.reg[LOOPSTART]))
                        .wrapping_sub(u32::from(voice.reg[LOOPEND]))
                        & 0xffff;
                    frac = 0;
                }
            }

            voice.reg[CURRPOS] = pos as u16;
            voice.fraction = frac as u16;
        }

        for ((l, r), o) in left.iter().zip(right.iter()).zip(out.iter_mut()) {
            *o = (
                (l >> 16).clamp(-32768, 32767) as i16,
                (r >> 16).clamp(-32768, 32767) as i16,
            );
        }
    }

    /// The compressed voice: two nibbles per byte, each held for three
    /// output samples, run through the chip's fixed delta table.
    fn render_adpcm(&mut self, left: &mut [i64], right: &mut [i64]) {
        let voice = &mut self.voices[ADPCM_VOICE];
        if !self.adpcm || voice.reg[BANK_REG] >= self.total_banks || voice.reg[RATE] == 0 {
            return;
        }
        const DELTA_TAB: [i32; 16] = [
            154, 154, 128, 102, 77, 58, 58, 58, 58, 58, 58, 58, 77, 102, 128, 154,
        ];
        let base = usize::from(voice.reg[BANK_REG]) * BANK;
        let rvol_reg = i64::from(voice.reg[RIGHTVOL]);
        let lvol = if self.stereo {
            i64::from(voice.reg[LEFTVOL])
        } else {
            rvol_reg
        };
        let rvol = if self.stereo && !self.right_volume_set {
            lvol
        } else {
            rvol_reg
        };
        let mut pos = u32::from(voice.reg[CURRPOS]);
        let mut frac = voice.fraction;

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // The compressed voice is added twice to the accumulator on the
            // real chip ("2x APAC"), hence the doubled volumes.
            *l = i64::from(self.adpcm_current) * (lvol * 2);
            *r = i64::from(self.adpcm_current) * (rvol * 2);

            frac += 1;
            if frac == 6 {
                pos += 1;
                frac = 0;
            }

            if frac == 1 || frac == 4 {
                if pos >= u32::from(voice.reg[LOOPEND]) {
                    break;
                }
                let byte = self.rom[base + pos as usize];
                let mut value = i32::from(if frac == 1 { byte >> 4 } else { byte }) & 0xf;
                if value & 0x8 != 0 {
                    value |= !0xf;
                }

                let mut delta = self.adpcm_delta_n * value;
                if value > 0 {
                    delta += self.adpcm_delta_n >> 1;
                } else {
                    delta -= self.adpcm_delta_n >> 1;
                }
                self.adpcm_current = (self.adpcm_current + delta).clamp(-32768, 32767);
                self.adpcm_delta_n =
                    ((self.adpcm_delta_n * DELTA_TAB[(value + 8) as usize]) >> 6).clamp(1, 2000);
            }
        }

        voice.reg[CURRPOS] = pos as u16;
        voice.fraction = frac;
        // Rate is a control register here: reaching the end clears it, which
        // is how the firmware learns the bed finished.
        if pos >= u32::from(voice.reg[LOOPEND]) {
            voice.reg[RATE] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chip with one bank whose samples ramp, so a voice's output is easy
    /// to recognise.
    fn chip() -> Bsmt2000 {
        let mut rom = vec![0u8; BANK];
        for (i, b) in rom.iter_mut().enumerate() {
            *b = (i % 100) as u8;
        }
        let mut c = Bsmt2000::new(rom, 11);
        c.de_banking = false;
        c
    }

    #[test]
    fn a_silent_chip_renders_silence() {
        let mut c = chip();
        let mut out = [(1i16, 1i16); 32];
        c.render(&mut out);
        assert!(out.iter().all(|&(l, r)| l == 0 && r == 0));
    }

    #[test]
    fn a_voice_with_a_bank_and_a_rate_makes_sound() {
        let mut c = chip();
        // Mode 1 register map: CURRPOS group at 0x00, RATE at 0x16, LOOPEND
        // at 0x21, BANK at 0x37, volumes at 0x42/0x4d — all for voice 0.
        c.write(0x37, 0); // bank 0
        c.write(0x00, 0); // position 0
        c.write(0x21, 0x4000); // loop end well away
        c.write(0x2c, 0); // loop start
        c.write(0x42, 0x7fff); // right volume
        c.write(0x4d, 0x7fff); // left volume
        c.write(0x16, 0x800); // rate: one sample per output sample
        let mut out = [(0i16, 0i16); 64];
        c.render(&mut out);
        assert!(out.iter().any(|&(l, _)| l != 0), "no sound came out");
    }

    #[test]
    fn the_reset_mode_follows_the_last_register() {
        let mut c = chip();
        assert_eq!(c.mode, 1);
        assert!((c.sample_rate() - 24000.0).abs() < 1.0);
        // Touching register 6 then resetting moves to the 34 kHz mode.
        c.write(0x06, 0);
        c.last_register = 6;
        c.reset();
        assert_eq!(c.mode, 6);
        assert!(c.sample_rate() > 30000.0);
    }

    #[test]
    fn the_de_bank_permutation_matches_the_board() {
        let mut c = chip();
        c.de_banking = true;
        // `(b&7) | ((b&0x18)<<1) | ((b&0x20)>>2)`, spot-checked.
        assert_eq!(c.remap_bank(0x00), 0x00);
        assert_eq!(c.remap_bank(0x07), 0x07);
        assert_eq!(c.remap_bank(0x08), 0x10);
        assert_eq!(c.remap_bank(0x20), 0x08);
        assert_eq!(c.remap_bank(0x3f), 0x3f);
        assert_eq!(c.remap_bank(0x18), 0x30);
    }
}
