//! Mixing what a table sounds like into one stereo stream.
//!
//! Two things make noise while a table is played, and they arrive completely
//! differently. The sound board produces a continuous mono stream, one sample
//! at a time, as its CPU runs. The mechanical sounds are one-shots: a script
//! says `PlaySound "fx_bumper"` and a recording starts, at its own sample rate,
//! with its own gain and pan. This puts both into one interleaved stereo buffer
//! at the output rate.
//!
//! # Why mix here and not in the browser
//!
//! Web Audio would do this natively, and do the resampling better. It was still
//! worth doing here, for two reasons. The board's stream has to come out of
//! wasm as a stream whatever happens, so there is one buffer and one latency
//! budget to get right either way; and mixing in Rust means a test can play a
//! table for five seconds and measure what came out, which is how everything
//! else in this port is checked. Audio that can only be judged by listening to
//! it in a browser is audio nobody will notice breaking.
//!
//! # The resampling
//!
//! Linear interpolation. The sources are 11 to 44 kHz and the output is 48, so
//! most of the work is mild upsampling of short percussive recordings, where
//! the difference against a windowed-sinc resampler is not audible. The one
//! place it would show — a long, tonal, looping sound — is the ball roll, and
//! that is noise.

use std::rc::Rc;

/// A sound asking to be played.
pub struct Play {
    /// Interleaved samples in -1..1.
    pub pcm: Rc<[f32]>,
    pub channels: u16,
    pub source_rate: u32,
    /// Linear gain, 1.0 being the recording as it is.
    pub gain: f32,
    /// -1 hard left, 0 centre, 1 hard right.
    pub pan: f32,
    /// Playback rate multiplier: 1.0 plays at the recorded pitch.
    pub speed: f32,
    /// How many further times to repeat after the first: 0 plays once,
    /// a negative value repeats for ever.
    pub loops: i32,
    /// What [`Mixer::stop`] matches on. A table stops a sound by name, and
    /// several voices can share one.
    pub key: u64,
}

struct Voice {
    pcm: Rc<[f32]>,
    channels: usize,
    /// Position in frames, fractional because of the resampling.
    at: f64,
    /// How far the cursor moves per output frame.
    step: f64,
    left: f32,
    right: f32,
    loops: i32,
    key: u64,
}

impl Voice {
    fn frames(&self) -> usize {
        self.pcm.len() / self.channels
    }

    /// The frame at `at`, interpolated, as a stereo pair before gain.
    fn sample(&self) -> (f32, f32) {
        let frames = self.frames();
        let i = self.at as usize;
        let frac = (self.at - i as f64) as f32;
        // The last frame has nothing after it to interpolate towards. Holding
        // it is right for a one-shot and inaudible for a loop, where the next
        // frame is the start again and the seam is one sample long.
        let j = (i + 1).min(frames.saturating_sub(1));
        let at = |f: usize, c: usize| self.pcm[f * self.channels + c];
        let lerp = |a: f32, b: f32| a + (b - a) * frac;
        if self.channels >= 2 {
            (lerp(at(i, 0), at(j, 0)), lerp(at(i, 1), at(j, 1)))
        } else {
            let m = lerp(at(i, 0), at(j, 0));
            (m, m)
        }
    }
}

/// The stream from the sound board, and everything the table is playing.
pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    /// Mono samples from the emulated sound board, waiting their turn.
    stream: Vec<f32>,
    /// Where in `stream` the next output frame comes from. The buffer is
    /// compacted rather than drained one sample at a time.
    stream_at: usize,
    /// Master gain, applied to everything.
    pub master: f32,
    /// Gain for the board's stream on its own, so the machine can be balanced
    /// against the mechanical sounds without touching either.
    pub board_gain: f32,
    /// How many voices can sound at once.
    ///
    /// A table under multiball with the flippers going asks for a handful; the
    /// cap is there so that a script bug — a `PlaySound` in a 1 ms timer —
    /// costs a dropped sound rather than the frame rate.
    pub max_voices: usize,
}

/// How much board audio to keep before deciding the game has run ahead.
///
/// The table's clock and the browser's audio clock are not the same clock, and
/// over a minute they drift. Letting the queue grow without limit turns that
/// drift into latency: the sound of a bumper arriving half a second after the
/// ball hit it. Dropping the oldest is the right trade — a click nobody notices
/// against a delay everybody does.
const MAX_STREAM_SECONDS: f32 = 0.25;

impl Mixer {
    pub fn new(rate: u32) -> Self {
        Self {
            rate: rate.max(1),
            voices: Vec::new(),
            stream: Vec::new(),
            stream_at: 0,
            master: 1.0,
            board_gain: 1.0,
            max_voices: 24,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How many sounds are playing.
    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    /// How much board audio is queued, in samples.
    pub fn queued(&self) -> usize {
        self.stream.len() - self.stream_at
    }

    /// Starts a sound. Returns false if it was dropped for want of a voice.
    pub fn play(&mut self, play: Play) -> bool {
        if play.pcm.is_empty() || play.channels == 0 {
            return false;
        }
        if self.voices.len() >= self.max_voices {
            log::trace!("dropped a sound: {} already playing", self.voices.len());
            return false;
        }
        let (left, right) = pan_gains(play.pan, play.gain);
        self.voices.push(Voice {
            channels: usize::from(play.channels),
            pcm: play.pcm,
            at: 0.0,
            step: f64::from(play.source_rate) * f64::from(play.speed.max(0.01))
                / f64::from(self.rate),
            left,
            right,
            loops: play.loops,
            key: play.key,
        });
        true
    }

    /// Changes the gain, pan and speed of what is already playing.
    ///
    /// This is what a rolling sound needs. A table re-asks for it on every tick
    /// of a timer with a level and a pitch worked out from the ball's speed and
    /// a pan from where it is; the original applies those to the voice already
    /// running rather than starting another. Restarting instead gives a stutter
    /// twenty times a second, and ignoring it gives a ball that sounds the same
    /// wherever it is and however fast it goes.
    ///
    /// Returns whether anything was updated.
    pub fn update(&mut self, key: u64, gain: f32, pan: f32, speed: f32, source_rate: u32) -> bool {
        let (left, right) = pan_gains(pan, gain);
        let step = f64::from(source_rate) * f64::from(speed.max(0.01)) / f64::from(self.rate);
        let mut found = false;
        for voice in self.voices.iter_mut().filter(|v| v.key == key) {
            voice.left = left;
            voice.right = right;
            // The cursor stays where it is on purpose: changing the speed moves
            // the pitch from here on, it does not seek.
            voice.step = step;
            found = true;
        }
        found
    }

    /// Stops every voice started with this key. What `StopSound` does.
    pub fn stop(&mut self, key: u64) {
        self.voices.retain(|v| v.key != key);
    }

    /// Whether anything with this key is playing. `usesame` needs to know.
    pub fn playing(&self, key: u64) -> bool {
        self.voices.iter().any(|v| v.key == key)
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    /// Hands over mono samples from the sound board, already at the output rate.
    pub fn push_board(&mut self, samples: &[f32]) {
        self.stream.extend_from_slice(samples);
        let cap = (self.rate as f32 * MAX_STREAM_SECONDS) as usize;
        let queued = self.queued();
        if queued > cap {
            // Keep the newest: the machine is what it is doing now, not what it
            // was doing a quarter of a second ago.
            self.stream_at += queued - cap;
        }
        self.compact();
    }

    /// Renders `out.len() / 2` frames of interleaved stereo.
    ///
    /// Anything already in `out` is overwritten, so a caller does not have to
    /// clear it.
    pub fn render(&mut self, out: &mut [f32]) {
        // `as_chunks_mut` rather than `chunks_exact_mut(2)`: the two say the
        // same thing, but this one says it in the type — a frame is a pair, and
        // the compiler knows it is a pair, so indexing it needs no bounds check.
        let (frames, _) = out.as_chunks_mut::<2>();
        for frame in frames {
            let board = self.next_board() * self.board_gain;
            frame[0] = board;
            frame[1] = board;
        }

        for voice in &mut self.voices {
            let (frames, _) = out.as_chunks_mut::<2>();
            for frame in frames {
                let (l, r) = voice.sample();
                frame[0] += l * voice.left;
                frame[1] += r * voice.right;

                voice.at += voice.step;
                if voice.at >= voice.frames() as f64 {
                    if voice.loops == 0 {
                        break;
                    }
                    if voice.loops > 0 {
                        voice.loops -= 1;
                    }
                    // Carry the overshoot rather than snapping to zero, or a
                    // short looping sound drifts flat by a fraction of a frame
                    // every repeat.
                    voice.at -= voice.frames() as f64;
                }
            }
            // A voice that ran out is marked by leaving the cursor past the
            // end; it is removed below rather than mid-iteration.
        }
        self.voices.retain(|v| v.at < v.frames() as f64);

        for s in out.iter_mut() {
            *s = (*s * self.master).clamp(-1.0, 1.0);
        }
        self.compact();
    }

    fn next_board(&mut self) -> f32 {
        match self.stream.get(self.stream_at) {
            Some(&s) => {
                self.stream_at += 1;
                s
            }
            // An empty queue means the game did not run far enough to make the
            // sound yet. Silence is the only honest answer; repeating the last
            // sample would buzz.
            None => 0.0,
        }
    }

    /// Drops what has already been played, once it is worth the move.
    fn compact(&mut self) {
        if self.stream_at > 0 && self.stream_at * 2 >= self.stream.len() {
            self.stream.drain(..self.stream_at);
            self.stream_at = 0;
        }
    }
}

/// Constant-power panning.
///
/// The obvious `left = 1 - pan` is wrong in a way that is easy to hear: a sound
/// swept across the middle dips, because two half-amplitude speakers are
/// quieter than one at full. Taking the gains off a quarter-circle keeps the
/// power constant, so a sound crossing the playfield stays the same loudness.
fn pan_gains(pan: f32, gain: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (gain * angle.cos(), gain * angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(values: &[f32]) -> Rc<[f32]> {
        Rc::from(values)
    }

    fn play(pcm: Rc<[f32]>, rate: u32) -> Play {
        Play {
            pcm,
            channels: 1,
            source_rate: rate,
            gain: 1.0,
            pan: 0.0,
            speed: 1.0,
            loops: 0,
            key: 0,
        }
    }

    /// Peak of one channel, 0 for left and 1 for right.
    fn peak(out: &[f32], channel: usize) -> f32 {
        out.iter()
            .skip(channel)
            .step_by(2)
            .fold(0.0f32, |a, s| a.max(s.abs()))
    }

    #[test]
    fn a_sound_plays_once_and_stops() {
        let mut m = Mixer::new(1000);
        m.play(play(mono(&[1.0; 100]), 1000));
        assert_eq!(m.voices(), 1);

        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out);
        assert_eq!(m.voices(), 1, "half way through it is still playing");
        assert!(peak(&out, 0) > 0.5);

        m.render(&mut out);
        assert_eq!(m.voices(), 0, "it should have finished and been dropped");

        // And then there is silence, not the last sample held.
        m.render(&mut out);
        assert_eq!(peak(&out, 0), 0.0);
    }

    #[test]
    fn a_loop_repeats_the_number_of_times_it_was_asked_to() {
        let mut m = Mixer::new(1000);
        let mut p = play(mono(&[1.0; 10]), 1000);
        p.loops = 2; // the first pass plus two more
        m.play(p);

        // 30 frames is exactly the three passes; on the 31st it is gone.
        let mut out = vec![0.0; 2 * 30];
        m.render(&mut out);
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn a_negative_loop_count_never_ends() {
        let mut m = Mixer::new(1000);
        let mut p = play(mono(&[1.0; 10]), 1000);
        p.loops = -1;
        m.play(p);
        let mut out = vec![0.0; 2 * 5000];
        m.render(&mut out);
        assert_eq!(m.voices(), 1, "an endless loop should still be going");
        assert!(peak(&out, 0) > 0.5);
    }

    #[test]
    fn panning_keeps_the_power_constant() {
        // The test that catches `left = 1 - pan`: a sound in the middle has to
        // be as loud as one hard over, not 3 dB quieter.
        let power = |pan: f32| {
            let (l, r) = pan_gains(pan, 1.0);
            l * l + r * r
        };
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            assert!((power(pan) - 1.0).abs() < 1e-5, "pan {pan}: {}", power(pan));
        }
        // And it does go where it was told.
        let (l, r) = pan_gains(-1.0, 1.0);
        assert!(l > 0.99 && r < 0.01);
    }

    #[test]
    fn a_pan_puts_the_sound_on_the_right_side() {
        let mut m = Mixer::new(1000);
        let mut p = play(mono(&[1.0; 100]), 1000);
        p.pan = 1.0;
        m.play(p);
        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out);
        assert!(peak(&out, 1) > 0.9, "the right channel should have it");
        assert!(peak(&out, 0) < 0.01, "and the left should not");
    }

    #[test]
    fn resampling_stretches_a_slower_source() {
        // A 500 Hz source into a 1000 Hz mixer lasts twice as many output
        // frames. Getting this backwards makes every sound play at double speed
        // and half length, which sounds like a different sound rather than a
        // broken one.
        let mut m = Mixer::new(1000);
        m.play(play(mono(&[1.0; 10]), 500));
        let mut out = vec![0.0; 2 * 19];
        m.render(&mut out);
        assert_eq!(m.voices(), 1, "10 frames at half rate should fill 20");
        m.render(&mut out);
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn speed_changes_the_pitch() {
        let mut m = Mixer::new(1000);
        let mut p = play(mono(&[1.0; 100]), 1000);
        p.speed = 2.0;
        m.play(p);
        let mut out = vec![0.0; 2 * 49];
        m.render(&mut out);
        assert_eq!(m.voices(), 1);
        m.render(&mut out);
        assert_eq!(m.voices(), 0, "at double speed it lasts half as long");
    }

    #[test]
    fn interpolation_actually_interpolates() {
        // A ramp resampled up should come out as a ramp, not as a staircase.
        let mut m = Mixer::new(1000);
        let pcm: Vec<f32> = (0..10).map(|i| i as f32 / 10.0).collect();
        m.play(play(Rc::from(pcm.as_slice()), 100));
        let mut out = vec![0.0; 2 * 20];
        m.render(&mut out);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        for pair in left.windows(2) {
            assert!(pair[1] >= pair[0], "the ramp went backwards: {left:?}");
        }
        // Ten source frames at a tenth of the rate is a hundred output frames,
        // so twenty in we are a fifth of the way up.
        assert!(left[19] > 0.0 && left[19] < 0.3, "{}", left[19]);
    }

    #[test]
    fn the_board_stream_comes_out_on_both_channels() {
        let mut m = Mixer::new(1000);
        m.push_board(&[0.5; 10]);
        let mut out = vec![0.0; 2 * 10];
        m.render(&mut out);
        assert_eq!(peak(&out, 0), 0.5);
        assert_eq!(peak(&out, 1), 0.5);
        assert_eq!(m.queued(), 0);
    }

    #[test]
    fn an_empty_board_queue_is_silence() {
        // Not the last sample repeated, which is what a naive hold does and
        // which buzzes at the frame rate.
        let mut m = Mixer::new(1000);
        m.push_board(&[1.0; 4]);
        let mut out = vec![0.0; 2 * 10];
        m.render(&mut out);
        assert_eq!(out[2 * 8], 0.0);
        assert_eq!(out[2 * 9], 0.0);
    }

    #[test]
    fn the_board_queue_does_not_grow_without_limit() {
        // A game running faster than real time would otherwise build a queue
        // that turns into latency: the bumper heard after the ball has gone.
        let mut m = Mixer::new(1000);
        for _ in 0..100 {
            m.push_board(&[0.1; 100]);
        }
        assert!(
            m.queued() <= 250,
            "a quarter of a second at most, not {}",
            m.queued()
        );
    }

    #[test]
    fn stopping_by_key_stops_only_that_sound() {
        let mut m = Mixer::new(1000);
        let mut a = play(mono(&[1.0; 100]), 1000);
        a.key = 7;
        m.play(a);
        let mut b = play(mono(&[1.0; 100]), 1000);
        b.key = 9;
        m.play(b);

        assert!(m.playing(7) && m.playing(9));
        m.stop(7);
        assert!(!m.playing(7) && m.playing(9));
        assert_eq!(m.voices(), 1);
    }

    #[test]
    fn too_many_at_once_are_dropped_rather_than_played() {
        let mut m = Mixer::new(1000);
        m.max_voices = 3;
        for _ in 0..3 {
            assert!(m.play(play(mono(&[1.0; 100]), 1000)));
        }
        assert!(!m.play(play(mono(&[1.0; 100]), 1000)));
        assert_eq!(m.voices(), 3);
    }

    #[test]
    fn a_stereo_source_keeps_its_sides() {
        let mut m = Mixer::new(1000);
        // Left channel loud, right silent.
        let pcm: Vec<f32> = (0..100).flat_map(|_| [1.0, 0.0]).collect();
        let mut p = play(Rc::from(pcm.as_slice()), 1000);
        p.channels = 2;
        m.play(p);
        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out);
        assert!(peak(&out, 0) > 0.5);
        assert!(peak(&out, 1) < 0.01);
    }

    #[test]
    fn the_output_does_not_clip_past_full_scale() {
        // Eight loud sounds at once is a real thing during multiball, and a
        // sum past 1.0 wraps into a very audible crackle if it is not clamped.
        let mut m = Mixer::new(1000);
        for _ in 0..8 {
            m.play(play(mono(&[1.0; 100]), 1000));
        }
        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out);
        assert!(out.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn render_overwrites_rather_than_accumulating() {
        // The caller reuses one buffer every callback. If render added to it,
        // silence would slowly turn into whatever was there before.
        let mut m = Mixer::new(1000);
        let mut out = vec![0.7; 2 * 10];
        m.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0), "{out:?}");
    }
}
