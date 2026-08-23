//! The table's own sounds: the ones that are not the machine's.
//!
//! A pinball makes two kinds of noise. One comes out of the cabinet speakers
//! and is the sound board's — on a ROM table that is emulated hardware, and it
//! is [`vpw_audio`]'s problem. The other is mechanical: the flipper's clack,
//! the ball rolling on wood, the slingshot, the drain. Those are recordings,
//! and every `.vpx` carries its own set.
//!
//! They matter more than they sound like they should. The hit is *information*:
//! it is how a player knows the ball caught the flipper rather than passing
//! over it, and how they know a shot went in without looking at it. A silent
//! table plays measurably worse.
//!
//! # What is in the file
//!
//! Uncompressed PCM, always — across the four tables tried, every one of the
//! 187 sounds is `format_tag = 1`. What varies is everything else: 8, 16 and 24
//! bits, mono and stereo, and rates from 11025 to 44100. Rather than carry that
//! variety into the mixer, each is decoded once, on load, into `f32` samples in
//! -1..1. It costs about twice the file's size in memory and makes everything
//! downstream one code path.
//!
//! # Where it departs from the original
//!
//! Visual Pinball hands the file to miniaudio, which decodes it on demand and
//! can also read Ogg and MP3. We decode PCM ourselves and skip anything else
//! with a warning, because none of the tables tried uses anything else and a
//! decoder for formats nobody has is code that cannot be tested.

use std::collections::HashMap;

use vpin::vpx::VPX;
use vpin::vpx::sound::SoundData;

/// One sound, decoded and ready to play.
pub struct Sample {
    /// What the script calls it: `PlaySound "fx_bumper"`.
    pub name: Box<str>,
    pub rate: u32,
    pub channels: u16,
    /// Interleaved samples in -1..1.
    pub pcm: Vec<f32>,
    /// The table's own default level for this sound, in -1..1.
    ///
    /// A script's `volume` argument is **added** to this rather than replacing
    /// it, which is why `PlaySound "x", 0, 1` — the common form — comes out at
    /// 1.0 for a sound whose stored volume is 0.
    pub volume: f32,
    /// The table's own default pan, -1 left to 1 right.
    pub pan: f32,
    /// Front-to-rear fade, for a cabinet with four speakers. Kept because it is
    /// in the file and costs nothing; nothing here uses it yet.
    pub fade: f32,
}

impl Sample {
    /// How long it lasts, in seconds.
    pub fn duration(&self) -> f32 {
        let frames = self.pcm.len() / self.channels.max(1) as usize;
        frames as f32 / self.rate.max(1) as f32
    }
}

/// Every sound a table carries, looked up the way a script asks for them.
#[derive(Default)]
pub struct Bank {
    samples: Vec<Sample>,
    /// Lowercased name to index. Scripts are inconsistent about case and the
    /// original compares without it.
    by_name: HashMap<Box<str>, usize>,
}

impl Bank {
    /// Adds a sample, keeping the name index in step.
    ///
    /// Public because a test wants a bank it can describe in three lines
    /// rather than a hundred-megabyte table.
    pub fn push(&mut self, sample: Sample) {
        let key: Box<str> = sample.name.to_ascii_lowercase().into_boxed_str();
        self.by_name.entry(key).or_insert(self.samples.len());
        self.samples.push(sample);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Sample> {
        self.samples.get(index)
    }

    /// The index of a sound by the name a script uses, ignoring case.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name
            .get(name.to_ascii_lowercase().as_str())
            .copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }

    /// How much memory the decoded samples take. Worth knowing: a big table's
    /// bank is tens of megabytes.
    pub fn bytes(&self) -> usize {
        self.samples.iter().map(|s| s.pcm.len() * 4).sum()
    }
}

/// Decodes every sound in a table.
pub fn extract(vpx: &VPX) -> Bank {
    let mut bank = Bank::default();
    for data in &vpx.sounds {
        let Some(sample) = decode(data) else {
            continue;
        };
        // A duplicate name is not an error in the file, but only one of them
        // can be reachable — the original's lookup returns the first match, so
        // `push` keeps the first.
        bank.push(sample);
    }
    bank
}

/// `WAVE_FORMAT_PCM`. The only one any table tried actually uses.
const FORMAT_PCM: u16 = 1;

fn decode(data: &SoundData) -> Option<Sample> {
    let w = &data.wave_form;
    if w.format_tag != FORMAT_PCM {
        log::warn!(
            "sound '{}' is format {}, which we do not decode",
            data.name,
            w.format_tag
        );
        return None;
    }
    let pcm = match w.bits_per_sample {
        8 => data.data.iter().map(|&b| unsigned8(b)).collect(),
        16 => signed_le(&data.data, 2),
        24 => signed_le(&data.data, 3),
        bits => {
            log::warn!(
                "sound '{}' is {bits} bits, which we do not decode",
                data.name
            );
            return None;
        }
    };
    Some(Sample {
        name: data.name.as_str().into(),
        rate: w.samples_per_sec.max(1),
        channels: w.channels.max(1),
        pcm,
        volume: signed_percent(data.volume),
        pan: signed_percent(data.balance),
        fade: signed_percent(data.fade),
    })
}

/// 8-bit PCM is **unsigned**, centred on 128. Reading it as signed gives a
/// sound that is inverted and clipped, which is audible and wrong rather than
/// quiet and wrong.
fn unsigned8(b: u8) -> f32 {
    (f32::from(b) - 128.0) / 128.0
}

/// Little-endian signed PCM of `width` bytes per sample.
///
/// The scale is `1 << (bits - 1)`, so a full-scale negative sample lands at
/// exactly -1.0 and a full-scale positive one just short of it. That is the
/// usual asymmetry of two's complement audio and not worth correcting.
fn signed_le(bytes: &[u8], width: usize) -> Vec<f32> {
    let shift = 32 - width * 8;
    let scale = 1.0 / (1i64 << (width * 8 - 1)) as f32;
    bytes
        .chunks_exact(width)
        .map(|c| {
            // Assemble into the top of an i32 and shift down, which sign-extends
            // whatever the width is without a special case for 24-bit.
            let mut raw: u32 = 0;
            for (i, &b) in c.iter().enumerate() {
                raw |= u32::from(b) << (shift + i * 8);
            }
            ((raw as i32) >> shift) as f32 * scale
        })
        .collect()
}

/// The file stores volume, pan and fade as a percentage in -100..100.
///
/// `math.h:95`, `dequantizeSignedPercent`.
fn signed_percent(stored: u32) -> f32 {
    // The field is unsigned in the file but signed in meaning: the original
    // reads it into an `int`, so a negative pan arrives as a large u32.
    (stored as i32 as f32 / 100.0).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_is_unsigned_and_centred_on_128() {
        // Silence in 8-bit PCM is 128, not 0. Reading it as signed turns a
        // quiet sound into a loud one at the wrong polarity.
        assert_eq!(unsigned8(128), 0.0);
        assert_eq!(unsigned8(255), 127.0 / 128.0);
        assert_eq!(unsigned8(0), -1.0);
    }

    #[test]
    fn sixteen_bit_covers_the_whole_range() {
        let bytes = [0x00, 0x00, 0xFF, 0x7F, 0x00, 0x80, 0x00, 0x40];
        let pcm = signed_le(&bytes, 2);
        assert_eq!(pcm[0], 0.0);
        assert!((pcm[1] - 0.999_97).abs() < 1e-4, "{}", pcm[1]);
        assert_eq!(pcm[2], -1.0);
        assert_eq!(pcm[3], 0.5);
    }

    #[test]
    fn twenty_four_bit_sign_extends() {
        // The one that a hand-rolled decoder gets wrong: 24-bit has no integer
        // type of its own, so the sign bit has to be carried up by hand.
        let bytes = [0x00, 0x00, 0x80, 0x00, 0x00, 0x40];
        let pcm = signed_le(&bytes, 3);
        assert_eq!(pcm[0], -1.0);
        assert_eq!(pcm[1], 0.5);
    }

    #[test]
    fn a_stored_percentage_becomes_a_fraction() {
        assert_eq!(signed_percent(0), 0.0);
        assert_eq!(signed_percent(100), 1.0);
        // Stored as a negative int, which arrives here as a large u32.
        assert_eq!(signed_percent(-50i32 as u32), -0.5);
        // And anything past the range is clamped rather than trusted.
        assert_eq!(signed_percent(500), 1.0);
    }

    #[test]
    fn duration_counts_frames_and_not_samples() {
        // A stereo sample has two values per frame; counting them as frames
        // makes every stereo sound report half its real length.
        let s = Sample {
            name: "x".into(),
            rate: 100,
            channels: 2,
            pcm: vec![0.0; 400],
            volume: 0.0,
            pan: 0.0,
            fade: 0.0,
        };
        assert_eq!(s.duration(), 2.0);
    }
}
