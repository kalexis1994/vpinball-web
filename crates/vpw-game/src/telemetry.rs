//! A rolling record of what the machine did, for looking backwards.
//!
//! Some bugs only show up while somebody is playing, and by the time they are
//! noticed the interesting part has already happened. A breakpoint is no use —
//! stopping the game destroys the thing being looked at — and neither is a log
//! that prints everything, because a table at a thousand ticks a second buries
//! the one moment that matters under a million that do not.
//!
//! So this keeps the last half hour and throws the rest away. When something
//! goes wrong the player presses a key, and what comes out is the minutes
//! before it, at full detail.
//!
//! # Two tracks, because there are two kinds of fact
//!
//! **Motion is continuous** and only interesting as a shape over time: where
//! the ball was, how fast, where the flippers were pointing. Sampling that at
//! [`SAMPLE_MS`] is enough to see a ball slow down, wedge itself, or leave a
//! saucer, and cheap enough to keep thirty minutes of it.
//!
//! **Everything else is an edge**: a coil fires, a switch closes, the script
//! asks for a sound. Those are meaningless when averaged and easy to miss when
//! sampled — a coil pulse is thirty milliseconds — so they are recorded as they
//! happen, with the millisecond they happened on.
//!
//! Both are rings: once full, the oldest goes to make room. Memory is bounded
//! by construction rather than by remembering to clear it.

use std::collections::VecDeque;

/// How often the continuous track takes a sample, in table milliseconds.
///
/// Twenty milliseconds. Fast enough that a ball travelling at the speed of a
/// hard shot moves about one ball radius between samples, so a trajectory is a
/// line and not a series of teleports; slow enough that half an hour of it is
/// megabytes and not hundreds. Anything faster than this is an edge, and edges
/// go in the other track at full resolution.
pub const SAMPLE_MS: u32 = 20;

/// How much history to keep, in table milliseconds. Thirty minutes.
pub const WINDOW_MS: u32 = 30 * 60 * 1000;

/// How many balls a sample records.
///
/// Four. A System 11 multiball is three, and the fourth is there so that a bug
/// which produces a ball too many is visible rather than silently cropped.
pub const MAX_BALLS: usize = 4;

/// The most edges to keep. Past this the oldest go, the same as the samples.
///
/// Two hundred thousand: about eight bytes of struct each, and far more than a
/// half hour of a table that is behaving. A table that is *not* behaving —
/// a coil retrying forever — is exactly the case that would otherwise grow
/// without limit, and it is the case this bound exists for.
const MAX_EVENTS: usize = 200_000;

/// One ball, at one instant.
#[derive(Clone, Copy, Default, Debug)]
pub struct BallSample {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
}

/// Everything that moves, at one instant.
///
/// A fixed array rather than a `Vec` on purpose: at fifty of these a second for
/// half an hour, a per-sample allocation is ninety thousand of them.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Table time, in milliseconds since the table was loaded.
    pub t_ms: u32,
    pub balls: [BallSample; MAX_BALLS],
    pub ball_count: u8,
    /// Which balls are held by a kicker, ball *n* in bit *n*.
    ///
    /// Worth its own bit because a held ball and a ball merely resting in the
    /// bowl look almost identical from position alone, and the difference is
    /// the whole question when a saucer will not give a ball back.
    pub locked: u8,
    /// Flipper angles in degrees, left and right, or `NaN` where there is no
    /// such flipper.
    pub flipper_left: f32,
    pub flipper_right: f32,
    /// The solenoid drivers as the board is holding them this instant.
    pub solenoids: u32,
    /// The switch matrix, switch *n* in bit *n-1*.
    pub switches: u64,
}

/// An edge: something that happened at a moment rather than over one.
#[derive(Clone, Debug)]
pub enum Event {
    /// The script asked for a sound.
    Sound(Box<str>),
    /// A coil went live, or stopped being.
    Solenoid { number: u8, on: bool },
    /// A switch closed or opened.
    Switch { number: u8, closed: bool },
    /// The script said something — a `MsgBox`, or a diagnostic.
    Message(Box<str>),
    /// A ball appeared or disappeared. Carries the new count.
    Balls(u8),
    /// A kicker took a ball or let one go. Carries the part's name, so a
    /// reader does not have to know shape indices.
    Kicker { name: Box<str>, holding: bool },
    /// The player pressed the mark key. Carries whatever the host wants to
    /// say about the moment — usually the wall-clock time.
    Mark(Box<str>),
    /// A frame that took longer than it should have.
    ///
    /// Only the slow ones: a record of every frame would be a hundred thousand
    /// entries in a half-hour window and would crowd out everything else, and
    /// the frames that arrive on time are not the ones anybody is asking about.
    ///
    /// The breakdown is what makes it worth having. Every part of this that
    /// runs in Rust has been measured and none of it is close to a frame — the
    /// physics never reached a millisecond over ninety seconds of play, the
    /// audio stays under fifty microseconds — so a hitch is either the work of
    /// pushing state to the GPU, the browser deciding to do something else, or
    /// a frame arriving late and the loop paying for the time it owes. Those
    /// three look completely different here, and cannot be told apart from
    /// outside.
    Frame {
        /// Wall clock since the previous frame: what the player actually feels.
        total_ms: f32,
        /// Physics ticks run in it, and how long they took.
        steps: u32,
        step_ms: f32,
        /// Copying the state into the renderer's buffers.
        sync_ms: f32,
        /// Recording and submitting the draw.
        render_ms: f32,
        /// How late the callback ran after the frame it belongs to was due.
        ///
        /// Large means the main thread was busy with something else when the
        /// frame came round. Small means it was not: the browser simply did
        /// not offer us a frame, and `idle_ms` says for how long.
        late_ms: f32,
        /// Wall clock between finishing the previous frame and starting this
        /// one — the time we were not running at all.
        idle_ms: f32,
    },
    /// Something outside the frame held the main thread.
    ///
    /// The page does more than step and draw, and the rest of it does not go
    /// through the frame at all: the audio is pumped from an animation frame of
    /// its own. A hitch that shows up here is ours; one that shows up nowhere,
    /// with the frame idle the whole time, is the browser's.
    Pause { source: Box<str>, ms: f32 },
}

/// An event and when it happened.
#[derive(Clone, Debug)]
pub struct Entry {
    pub t_ms: u32,
    pub event: Event,
}

/// The rolling record.
pub struct Telemetry {
    samples: VecDeque<Sample>,
    events: VecDeque<Entry>,
    /// Table time of the last sample taken, so the rate is kept in table time
    /// and not in frames.
    last_sample_ms: u32,
    /// Previous solenoid word and switch matrix, to find the edges.
    prev_solenoids: u32,
    prev_switches: u64,
    /// Previous ball count, so a ball appearing or draining is an edge.
    prev_balls: u8,
    /// Off until a host turns it on: a table nobody is debugging should not pay
    /// for eleven megabytes of history.
    enabled: bool,
    /// Whether the first sample has been taken. Before it there is no previous
    /// state, and every switch that starts closed would read as an edge.
    primed: bool,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            events: VecDeque::new(),
            last_sample_ms: 0,
            prev_solenoids: 0,
            prev_switches: 0,
            prev_balls: 0,
            enabled: false,
            primed: false,
        }
    }

    /// How many samples fit in the window.
    fn capacity(&self) -> usize {
        (WINDOW_MS / SAMPLE_MS) as usize
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turns recording on or off. Turning it off drops what was recorded:
    /// keeping it would be keeping a window that no longer ends at "now".
    pub fn set_enabled(&mut self, on: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if on {
            self.samples.reserve(self.capacity());
        } else {
            self.samples = VecDeque::new();
            self.events = VecDeque::new();
        }
        self.primed = false;
        self.last_sample_ms = 0;
    }

    /// Records an edge. Cheap enough to call from anywhere on the hot path.
    pub fn event(&mut self, t_ms: f64, event: Event) {
        if !self.enabled {
            return;
        }
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(Entry {
            t_ms: t_ms.max(0.0) as u32,
            event,
        });
    }

    /// Whether the continuous track wants a sample at this table time.
    pub fn wants_sample(&self, t_ms: f64) -> bool {
        if !self.enabled {
            return false;
        }
        let now = t_ms.max(0.0) as u32;
        !self.primed || now.saturating_sub(self.last_sample_ms) >= SAMPLE_MS
    }

    /// Takes one sample of the continuous track.
    pub fn sample(&mut self, sample: Sample) {
        if !self.enabled {
            return;
        }
        while self.samples.len() >= self.capacity() {
            self.samples.pop_front();
        }
        self.last_sample_ms = sample.t_ms;
        self.primed = true;
        self.samples.push_back(sample);
    }

    /// Compares the board's state with the last one seen and records what
    /// changed. Called every tick, which is what gives coil pulses their
    /// millisecond.
    pub fn edges(&mut self, t_ms: f64, solenoids: u32, switches: u64) {
        if !self.enabled {
            return;
        }
        // Before the first sample there is nothing to compare against, and
        // reporting the machine's whole starting state as a burst of edges
        // would say something happened when nothing did.
        if !self.primed {
            self.prev_solenoids = solenoids;
            self.prev_switches = switches;
            return;
        }
        let sol_changed = solenoids ^ self.prev_solenoids;
        if sol_changed != 0 {
            for bit in 0..32u8 {
                if sol_changed & (1 << bit) != 0 {
                    self.event(
                        t_ms,
                        Event::Solenoid {
                            number: bit + 1,
                            on: solenoids & (1 << bit) != 0,
                        },
                    );
                }
            }
            self.prev_solenoids = solenoids;
        }
        let sw_changed = switches ^ self.prev_switches;
        if sw_changed != 0 {
            for bit in 0..64u8 {
                if sw_changed & (1 << bit) != 0 {
                    self.event(
                        t_ms,
                        Event::Switch {
                            number: bit + 1,
                            closed: switches & (1 << bit) != 0,
                        },
                    );
                }
            }
            self.prev_switches = switches;
        }
    }

    /// Notes how many balls are on the table, recording an edge if it changed.
    pub fn balls(&mut self, t_ms: f64, count: u8) {
        if !self.enabled || count == self.prev_balls {
            return;
        }
        self.prev_balls = count;
        // Same reason as the switches: before the first sample there is no
        // previous state, and the first ball would read as an event.
        if self.primed {
            self.event(t_ms, Event::Balls(count));
        }
    }

    /// Table time of the newest thing recorded, or zero if nothing is.
    pub fn latest_ms(&self) -> u32 {
        let s = self.samples.back().map(|s| s.t_ms).unwrap_or(0);
        let e = self.events.back().map(|e| e.t_ms).unwrap_or(0);
        s.max(e)
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// The last `seconds` of history, as JSON.
    ///
    /// Written by hand rather than with a serialiser: it is one shape, it is
    /// emitted from one place, and a serialiser in the player is weight in
    /// every table's download for the benefit of a debug key.
    ///
    /// `note` is whatever the host wants recorded about the moment the dump was
    /// asked for — the wall-clock time, normally, since table time means
    /// nothing to somebody reading it later.
    pub fn dump(&self, seconds: f64, note: &str) -> String {
        let latest = self.latest_ms();
        let span = (seconds.max(0.0) * 1000.0) as u32;
        let from = latest.saturating_sub(span);

        let mut out = String::with_capacity(1 << 16);
        out.push_str("{\n  \"note\": ");
        push_json_str(&mut out, note);
        out.push_str(",\n  \"tableTimeMs\": ");
        push_u32(&mut out, latest);
        out.push_str(",\n  \"windowMs\": ");
        push_u32(&mut out, latest - from);
        out.push_str(",\n  \"sampleIntervalMs\": ");
        push_u32(&mut out, SAMPLE_MS);
        out.push_str(",\n  \"heldMs\": ");
        push_u32(&mut out, WINDOW_MS);
        out.push_str(",\n  \"samplesHeld\": ");
        push_u32(&mut out, self.samples.len() as u32);
        out.push_str(",\n  \"eventsHeld\": ");
        push_u32(&mut out, self.events.len() as u32);

        // The continuous track. One array per field rather than one object per
        // sample: the same numbers in a third of the bytes, and a reader that
        // wants the ball's path gets it as a column instead of a walk.
        out.push_str(",\n  \"samples\": {\n    \"t\": [");
        let mut first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            push_u32(&mut out, s.t_ms);
        }
        out.push_str("],\n    \"balls\": [");
        first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push('[');
            for (i, b) in s.balls.iter().take(s.ball_count as usize).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                for (j, v) in [b.x, b.y, b.z, b.vx, b.vy, b.vz].iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    push_f32(&mut out, *v);
                }
                out.push(']');
            }
            out.push(']');
        }
        out.push_str("],\n    \"flippers\": [");
        first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push('[');
            push_f32(&mut out, s.flipper_left);
            out.push(',');
            push_f32(&mut out, s.flipper_right);
            out.push(']');
        }
        // Which balls a kicker is holding. A held ball and a ball resting in
        // the same bowl are a millimetre apart in position and worlds apart in
        // what happens next, and this is the bit that tells them apart.
        out.push_str("],\n    \"locked\": [");
        first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            push_u32(&mut out, u32::from(s.locked));
        }
        out.push_str("],\n    \"solenoids\": [");
        first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            push_u32(&mut out, s.solenoids);
        }
        // The switch matrix as a string, because sixty-four bits does not
        // survive a trip through a JSON number: anything past fifty-three is a
        // double, and the top switches would come back wrong.
        out.push_str("],\n    \"switches\": [");
        first = true;
        for s in self.samples.iter().filter(|s| s.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push('"');
            push_hex64(&mut out, s.switches);
            out.push('"');
        }
        out.push_str("]\n  }");

        // The edges.
        out.push_str(",\n  \"events\": [");
        first = true;
        for e in self.events.iter().filter(|e| e.t_ms >= from) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("\n    {\"t\":");
            push_u32(&mut out, e.t_ms);
            match &e.event {
                Event::Sound(name) => {
                    out.push_str(",\"kind\":\"sound\",\"name\":");
                    push_json_str(&mut out, name);
                }
                Event::Solenoid { number, on } => {
                    out.push_str(",\"kind\":\"solenoid\",\"n\":");
                    push_u32(&mut out, u32::from(*number));
                    out.push_str(",\"on\":");
                    out.push_str(if *on { "true" } else { "false" });
                }
                Event::Switch { number, closed } => {
                    out.push_str(",\"kind\":\"switch\",\"n\":");
                    push_u32(&mut out, u32::from(*number));
                    out.push_str(",\"closed\":");
                    out.push_str(if *closed { "true" } else { "false" });
                }
                Event::Message(text) => {
                    out.push_str(",\"kind\":\"message\",\"text\":");
                    push_json_str(&mut out, text);
                }
                Event::Balls(n) => {
                    out.push_str(",\"kind\":\"balls\",\"count\":");
                    push_u32(&mut out, u32::from(*n));
                }
                Event::Kicker { name, holding } => {
                    out.push_str(",\"kind\":\"kicker\",\"name\":");
                    push_json_str(&mut out, name);
                    out.push_str(",\"holding\":");
                    out.push_str(if *holding { "true" } else { "false" });
                }
                Event::Mark(note) => {
                    out.push_str(",\"kind\":\"mark\",\"note\":");
                    push_json_str(&mut out, note);
                }
                Event::Frame {
                    total_ms,
                    steps,
                    step_ms,
                    sync_ms,
                    render_ms,
                    late_ms,
                    idle_ms,
                } => {
                    out.push_str(",\"kind\":\"frame\",\"totalMs\":");
                    push_f32(&mut out, *total_ms);
                    out.push_str(",\"steps\":");
                    push_u32(&mut out, *steps);
                    out.push_str(",\"stepMs\":");
                    push_f32(&mut out, *step_ms);
                    out.push_str(",\"syncMs\":");
                    push_f32(&mut out, *sync_ms);
                    out.push_str(",\"renderMs\":");
                    push_f32(&mut out, *render_ms);
                    out.push_str(",\"lateMs\":");
                    push_f32(&mut out, *late_ms);
                    out.push_str(",\"idleMs\":");
                    push_f32(&mut out, *idle_ms);
                }
                Event::Pause { source, ms } => {
                    out.push_str(",\"kind\":\"pause\",\"source\":");
                    push_json_str(&mut out, source);
                    out.push_str(",\"ms\":");
                    push_f32(&mut out, *ms);
                }
            }
            out.push('}');
        }
        out.push_str("\n  ]\n}\n");
        out
    }
}

fn push_u32(out: &mut String, v: u32) {
    use std::fmt::Write;
    let _ = write!(out, "{v}");
}

/// A float with three decimals, which is finer than anything a table measures.
///
/// `NaN` and the infinities become `null`: JSON has no way to write them, and a
/// reader that gets `NaN` back is a reader that crashes.
fn push_f32(out: &mut String, v: f32) {
    if !v.is_finite() {
        out.push_str("null");
        return;
    }
    use std::fmt::Write;
    let rounded = (f64::from(v) * 1000.0).round() / 1000.0;
    let mut text = String::new();
    let _ = write!(text, "{rounded}");
    if let Some(head) = text.strip_suffix(".0") {
        out.push_str(head);
    } else {
        out.push_str(&text);
    }
}

fn push_hex64(out: &mut String, v: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for shift in (0..16).rev() {
        out.push(HEX[((v >> (shift * 4)) & 0xf) as usize] as char);
    }
}

/// A JSON string, with the escapes the format actually requires.
fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u00");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push(HEX[((c as usize) >> 4) & 0xf] as char);
                out.push(HEX[(c as usize) & 0xf] as char);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
