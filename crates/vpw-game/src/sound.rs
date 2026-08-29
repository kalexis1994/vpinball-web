//! What the script means when it says `PlaySound`.
//!
//! The call looks trivial and is not. Its full form is
//!
//! ```vbscript
//! PlaySound "fx_bumper", 0, 1, 0.2, 0.25, 0, 0, 1, 0
//! ```
//!
//! — nine arguments, of which tables use anywhere between one and all nine, and
//! whose meanings are not the obvious ones. `volume` is an **offset added to
//! the sound's own stored level**, not a replacement for it. `pan` goes through
//! a tenth-root curve, because tables were written against a version that
//! raised it to the tenth and the original now undoes that. `LoopCount` of 1
//! means play once, not loop once. Getting any of these wrong gives a table
//! that makes noise but does not sound right, which is harder to notice and
//! harder to track down than silence.
//!
//! Ported from `pintable.cpp:4149`, `AudioPlayer.cpp:474` and
//! `SoundPlayer.cpp:376`; the defaults are the ones in `vpinball.idl:711`.

use std::rc::Rc;

use vpw_audio::mixer::{Mixer, Play};
use vpw_table::sound::Bank;
use vpw_vbscript::value::Value;

/// The arguments of `PlaySound`, as the original declares them.
///
/// `vpinball.idl:711`. A script that leaves one out gets these, and several of
/// them are not what you would guess: `restart` defaults to true, so the common
/// `PlaySound "x"` restarts the sound rather than layering another copy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    pub loop_count: i32,
    /// Added to the sound's own stored volume.
    pub volume: f32,
    /// Added to the sound's own stored pan.
    pub pan: f32,
    /// How far the pitch may wander, as a fraction. 0 is none.
    pub random_pitch: f32,
    /// A shift in Hz applied to the sample rate.
    pub pitch: i32,
    pub use_same: bool,
    pub restart: bool,
    pub front_rear_fade: f32,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            loop_count: 1,
            volume: 1.0,
            pan: 0.0,
            random_pitch: 0.0,
            pitch: 0,
            use_same: false,
            restart: true,
            front_rear_fade: 0.0,
        }
    }
}

impl Request {
    /// Reads the arguments a script actually passed, leaving the rest as the
    /// original's defaults.
    ///
    /// A bad argument is skipped rather than refused: `PlaySound` in Visual
    /// Pinball is a `Sub` that cannot fail, and a table that would stop on a
    /// malformed sound call is worse than one that plays it plainly.
    pub fn from_args(args: &[Value]) -> Self {
        let mut r = Self::default();
        let num = |i: usize| args.get(i).and_then(|v| v.to_number().ok());
        let int = |i: usize| args.get(i).and_then(|v| v.to_int().ok());
        let boolean = |i: usize| args.get(i).and_then(|v| v.to_bool().ok());

        if let Some(v) = int(1) {
            r.loop_count = v;
        }
        if let Some(v) = num(2) {
            r.volume = v as f32;
        }
        if let Some(v) = num(3) {
            r.pan = v as f32;
        }
        if let Some(v) = num(4) {
            r.random_pitch = v as f32;
        }
        if let Some(v) = int(5) {
            r.pitch = v;
        }
        if let Some(v) = boolean(6) {
            r.use_same = v;
        }
        if let Some(v) = boolean(7) {
            r.restart = v;
        }
        if let Some(v) = num(8) {
            r.front_rear_fade = v as f32;
        }
        r
    }

    /// How many further repeats the mixer should do.
    ///
    /// `SoundPlayer.cpp:412`. Negative is for ever; 0 and 1 both mean play once
    /// — which is the trap, because `PlaySound "x", 1` reads like "loop once"
    /// and is not.
    fn repeats(self) -> i32 {
        if self.loop_count < 0 {
            -1
        } else if self.loop_count <= 1 {
            0
        } else {
            self.loop_count - 1
        }
    }
}

/// A table's sounds, and the state of playing them.
pub struct Sounds {
    pub bank: Bank,
    pub mixer: Mixer,
    /// The decoded samples, shared with the voices playing them so that a
    /// twenty-megabyte bank is not copied every time a bumper is hit.
    pcm: Vec<Rc<[f32]>>,
    /// State for the random pitch. Its own, and not the interpreter's, so that
    /// audio never changes the sequence a script sees — a table that seeds its
    /// modes off `Rnd` would otherwise play differently with the sound on.
    seed: u64,
    /// Names a script asked for that are not in the table, already reported.
    /// The original keeps the same set, for the same reason: a missing sound in
    /// a hit handler would otherwise log sixty times a second.
    missing: Vec<Box<str>>,
}

impl Sounds {
    pub fn new(bank: Bank, rate: u32) -> Self {
        let pcm = bank
            .iter()
            .map(|s| Rc::from(s.pcm.as_slice()))
            .collect::<Vec<Rc<[f32]>>>();
        Self {
            bank,
            mixer: Mixer::new(rate),
            pcm,
            seed: 0x2545_F491_4F6C_DD1D,
            missing: Vec::new(),
        }
    }

    /// `PlaySound name, ...`.
    ///
    /// Returns whether anything was started, which is what a test looks at.
    pub fn play(&mut self, name: &str, request: Request) -> bool {
        let Some(index) = self.bank.find(name) else {
            self.report_missing(name);
            return false;
        };
        // Copied out rather than borrowed: `speed` needs `&mut self` for the
        // random pitch, and the sample would still be borrowed otherwise.
        let (rate, channels, stored_volume, stored_pan) = {
            let s = self.bank.get(index).expect("just found it");
            (s.rate, s.channels, s.volume, s.pan)
        };
        let key = index as u64;

        let volume = (stored_volume + request.volume).clamp(0.0, 1.0);
        // `SoundPlayer.cpp:372`: the gain is the square root of the level, not
        // the level. A table asking for 0.25 expects half as loud, not a
        // quarter — the curve is there to make the number behave the way a
        // table author assumed it did.
        let gain = volume.sqrt();
        let pan = pan_curve(stored_pan + request.pan);
        if *PAN_DEBUG {
            eprintln!(
                "PAN {name}: stored {stored_pan:+.3} request {:+.3} -> {pan:+.3}",
                request.pan
            );
        }
        let speed = self.speed(rate, request);

        // `AudioPlayer.cpp:486`. Three cases, and the middle one matters most:
        //
        // - `restart` stops what is playing and starts again.
        // - `usesame` without `restart` **updates** the running voice and
        //   leaves it running. That is how a rolling sound follows the ball:
        //   the table re-asks for it on every tick of a timer with a new level
        //   and pitch worked out from the ball's speed and position.
        // - neither gives a second, independent voice, which is what a bumper
        //   hit twice in quick succession wants.
        if request.restart {
            self.mixer.stop(key);
        } else if request.use_same && self.mixer.update(key, gain, pan, speed, rate) {
            return true;
        }

        self.mixer.play(Play {
            pcm: self.pcm[index].clone(),
            channels,
            source_rate: rate,
            gain,
            pan,
            speed,
            loops: request.repeats(),
            key,
        })
    }

    /// `StopSound name`.
    pub fn stop(&mut self, name: &str) {
        if let Some(index) = self.bank.find(name) {
            self.mixer.stop(index as u64);
        }
    }

    /// The playback rate for a request, random pitch included.
    ///
    /// `SoundPlayer.cpp:420`. `pitch` is a shift in **hertz** applied to the
    /// sample rate, so its effect depends on the recording: the same `pitch`
    /// moves a 22 kHz sound twice as far as a 44 kHz one. That is the
    /// original's behaviour and tables are tuned against it.
    fn speed(&mut self, rate: u32, request: Request) -> f32 {
        let base = rate as f32;
        let mut wanted = base + request.pitch as f32;
        if request.random_pitch > 0.0 {
            let high = self.random();
            let low = self.random();
            wanted *=
                1.0 + request.random_pitch * high * high - request.random_pitch * low * low * 0.5;
        }
        (wanted / base).clamp(0.01, 8.0)
    }

    /// A number in 0..1. xorshift64*, which is plenty for jittering a pitch.
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u32 << 24) as f32
    }

    fn report_missing(&mut self, name: &str) {
        if name.is_empty() || self.missing.iter().any(|n| &**n == name) {
            return;
        }
        log::trace!("the script asked for '{name}', which the table does not have");
        self.missing.push(name.into());
    }

    /// Names the script asked for that the table does not have.
    pub fn missing(&self) -> impl Iterator<Item = &str> {
        self.missing.iter().map(|n| &**n)
    }
}

/// `VPW_PAN=1` prints every sound's pan as it is played. Read once: this is
/// on the path of every `PlaySound`, and a table under multiball makes a lot
/// of them.
static PAN_DEBUG: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("VPW_PAN").is_ok());

/// How far off centre a sound is ever placed.
///
/// The curve below is the original's and is kept, but what it hands back is
/// brutal: `core.vbs` works its pan out linearly from where the object stands
/// across the table — `CoreAudioPan`, `(x*2/width)-1` — and the tenth root
/// turns six per cent off centre into seventy-five. Measured on F-14, a ball
/// rolling a little right of the middle came out at +0.76, which is most of
/// the way to one speaker with nothing in the other, and a ball crossing the
/// table swung from one ear to the other.
///
/// A pinball table is a metre wide and stands in front of you. Nothing on it
/// is *hard* left or right; the ball is a few degrees off centre at the
/// extremes, and what the ear wants is that much — a direction, not a
/// destination. So the placed pan is scaled, once, at the end: the curve's
/// full swing becomes half of one, which keeps both speakers in play at every
/// point on the playfield and still says which side the sound came from.
const STEREO_WIDTH: f32 = 0.5;

/// The pan curve, `SoundPlayer.cpp:387`, narrowed to [`STEREO_WIDTH`].
///
/// Tables were written against a version that raised the pan to the tenth
/// power, so they pass values chosen to survive that. The original now takes
/// the tenth root to undo it, which means a table's `0.5` really is near hard
/// over rather than half way. Skipping it leaves everything sounding like it
/// came from the middle; keeping it unnarrowed leaves everything sounding
/// like it came from one wall.
fn pan_curve(pan: f32) -> f32 {
    let p = pan.clamp(-1.0, 1.0);
    let curved = if p < 0.0 {
        -(-p).powf(0.1)
    } else {
        p.powf(0.1)
    };
    curved * STEREO_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpw_table::sound::Sample;

    fn bank_of(names: &[&str]) -> Bank {
        let mut b = Bank::default();
        for name in names {
            b.push(Sample {
                name: (*name).into(),
                rate: 1000,
                channels: 1,
                pcm: vec![1.0; 1000],
                volume: 0.0,
                pan: 0.0,
                fade: 0.0,
            });
        }
        b
    }

    fn sounds(names: &[&str]) -> Sounds {
        Sounds::new(bank_of(names), 1000)
    }

    #[test]
    fn the_defaults_are_the_ones_in_the_idl() {
        // `PlaySound "x"` with nothing else. `restart` being true by default is
        // the one that surprises: the same sound asked for twice does not
        // layer.
        let r = Request::from_args(&[Value::str("x")]);
        assert_eq!(r, Request::default());
        assert_eq!(r.loop_count, 1);
        assert_eq!(r.volume, 1.0);
        assert!(r.restart);
        assert!(!r.use_same);
    }

    #[test]
    fn a_loop_count_of_one_plays_once() {
        // The trap: it reads like "loop once" and means "play once".
        assert_eq!(
            Request {
                loop_count: 1,
                ..Default::default()
            }
            .repeats(),
            0
        );
        assert_eq!(
            Request {
                loop_count: 0,
                ..Default::default()
            }
            .repeats(),
            0
        );
        assert_eq!(
            Request {
                loop_count: 3,
                ..Default::default()
            }
            .repeats(),
            2
        );
        assert_eq!(
            Request {
                loop_count: -1,
                ..Default::default()
            }
            .repeats(),
            -1
        );
    }

    #[test]
    fn arguments_are_read_in_the_order_the_original_declares_them() {
        // `PlaySound "x", 0, 0.5, -0.3, 0.25, 100, True, False, 0`
        let args = [
            Value::str("x"),
            Value::Long(0),
            Value::Double(0.5),
            Value::Double(-0.3),
            Value::Double(0.25),
            Value::Long(100),
            Value::Bool(true),
            Value::Bool(false),
            Value::Double(0.0),
        ];
        let r = Request::from_args(&args);
        assert_eq!(r.loop_count, 0);
        assert_eq!(r.volume, 0.5);
        assert_eq!(r.pan, -0.3);
        assert_eq!(r.random_pitch, 0.25);
        assert_eq!(r.pitch, 100);
        assert!(r.use_same);
        assert!(!r.restart);
    }

    #[test]
    fn the_volume_argument_adds_to_the_sounds_own_level() {
        // The thing that is easy to get backwards. A sound stored at 0 with the
        // default argument of 1 plays at full, which is why nearly every table
        // sound has a stored volume of 0.
        let mut s = sounds(&["fx_bumper"]);
        assert!(s.play("fx_bumper", Request::default()));
        assert_eq!(s.mixer.voices(), 1);
    }

    #[test]
    fn a_missing_sound_is_reported_once_and_does_not_stop_the_table() {
        let mut s = sounds(&["a"]);
        assert!(!s.play("nope", Request::default()));
        assert!(!s.play("nope", Request::default()));
        assert_eq!(s.missing().count(), 1);
        // And the table keeps going: nothing panicked, nothing was raised.
        assert!(s.play("a", Request::default()));
    }

    #[test]
    fn the_name_is_matched_without_regard_to_case() {
        // Scripts are wildly inconsistent about this and the original compares
        // case-insensitively.
        let mut s = sounds(&["fx_Bumper"]);
        assert!(s.play("FX_BUMPER", Request::default()));
    }

    #[test]
    fn restart_replaces_rather_than_layering() {
        let mut s = sounds(&["a"]);
        s.play("a", Request::default());
        s.play("a", Request::default());
        assert_eq!(s.mixer.voices(), 1, "restart should have stopped the first");
    }

    #[test]
    fn without_restart_a_sound_layers() {
        // Which is what a table wants for a bumper hit twice in quick
        // succession: two hits, two sounds.
        let mut s = sounds(&["a"]);
        let r = Request {
            restart: false,
            ..Default::default()
        };
        s.play("a", r);
        s.play("a", r);
        assert_eq!(s.mixer.voices(), 2);
    }

    #[test]
    fn use_same_updates_the_running_sound_rather_than_starting_another() {
        // This is how a rolling sound follows the ball: F-14's script calls
        // `PlaySound("fx_ballrolling0"), -1, Vol, Pan, 0, Pitch, 1, 0` on every
        // tick of a timer, with the level and pitch worked out from the ball.
        // Starting a new voice each time would stack twenty a second; ignoring
        // the call would leave the ball sounding the same wherever it is.
        let mut s = sounds(&["a"]);
        let quiet = Request {
            restart: false,
            use_same: true,
            volume: 0.1,
            pan: -1.0,
            ..Default::default()
        };
        assert!(s.play("a", quiet));
        assert_eq!(s.mixer.voices(), 1);

        let loud = Request {
            volume: 1.0,
            pan: 1.0,
            ..quiet
        };
        assert!(s.play("a", loud));
        assert_eq!(s.mixer.voices(), 1, "it should still be the one voice");

        // And the update took: the sound moved from the left to the right.
        let mut out = vec![0.0; 2 * 100];
        s.mixer.render(&mut out);
        let side = |c: usize| {
            out.iter()
                .skip(c)
                .step_by(2)
                .fold(0.0f32, |a, x| a.max(x.abs()))
        };
        // Decisively right, and — since the width is limited on purpose, see
        // `STEREO_WIDTH` — still present on the left.
        assert!(
            side(1) > side(0) * 2.0,
            "it should have moved right: left {} right {}",
            side(0),
            side(1)
        );
        assert!(side(0) > 0.05, "and not abandoned the left: {}", side(0));
    }

    #[test]
    fn stop_silences_the_sound_by_name() {
        let mut s = sounds(&["a", "b"]);
        s.play(
            "a",
            Request {
                restart: false,
                ..Default::default()
            },
        );
        s.play(
            "b",
            Request {
                restart: false,
                ..Default::default()
            },
        );
        s.stop("a");
        assert_eq!(s.mixer.voices(), 1);
        // Stopping something the table does not have is not an error.
        s.stop("nope");
    }

    #[test]
    fn a_sound_off_centre_leans_but_never_leaves_the_other_side() {
        // Two things at once, and the second is why this test changed. The
        // curve is the original's, so a middling value really does mean "well
        // over to that side" rather than "slightly": without it everything
        // sounds like it came from the middle.
        assert!(pan_curve(0.5) > 0.4, "{}", pan_curve(0.5));
        assert!(pan_curve(-0.5) < -0.4);
        assert_eq!(pan_curve(0.0), 0.0);

        // But the swing is bounded. A table is a metre wide and stands in
        // front of you; nothing on it belongs hard against one speaker, and
        // the measured case — a ball six per cent right of the middle, which
        // used to come out at +0.76 — has to stay a lean.
        assert!(pan_curve(1.0) <= STEREO_WIDTH + 1e-6);
        assert!(pan_curve(0.063) < 0.45, "{}", pan_curve(0.063));
        for p in [-4.0, -1.0, 0.06, 0.5, 1.0, 4.0] {
            assert!(pan_curve(p).abs() <= STEREO_WIDTH + 1e-6, "pan {p}");
        }
        // And it stays inside the range whatever a table adds up to.
        assert_eq!(pan_curve(4.0), STEREO_WIDTH);
    }

    #[test]
    fn pitch_is_a_shift_in_hertz_and_not_a_ratio() {
        // The same `pitch` moves a 22 kHz sound twice as far as a 44 kHz one,
        // which is the original's behaviour and what tables are tuned against.
        let mut s = sounds(&["a"]);
        let r = Request {
            pitch: 500,
            ..Default::default()
        };
        assert_eq!(s.speed(1000, r), 1.5);
        assert_eq!(s.speed(2000, r), 1.25);
    }

    #[test]
    fn random_pitch_stays_near_one_and_varies() {
        let mut s = sounds(&["a"]);
        let r = Request {
            random_pitch: 0.25,
            ..Default::default()
        };
        let mut seen = Vec::new();
        for _ in 0..20 {
            let v = s.speed(1000, r);
            assert!((0.8..=1.3).contains(&v), "{v} is not a small jitter");
            seen.push(v);
        }
        assert!(
            seen.windows(2).any(|w| w[0] != w[1]),
            "a random pitch that never changes is not random"
        );
    }

    #[test]
    fn no_random_pitch_means_exactly_the_recorded_rate() {
        // A table that does not ask for jitter must get none, or every sound
        // wanders and the table sounds broken in a way that is hard to name.
        let mut s = sounds(&["a"]);
        assert_eq!(s.speed(1000, Request::default()), 1.0);
    }
}
