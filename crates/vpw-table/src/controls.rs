//! From the keys to the table.
//!
//! In Visual Pinball this is done by the **script**: each table's VBScript gets
//! a `KeyDown` with a code and decides what to move. Without that engine, the
//! bridge has to decide it, and this is the layer that does.
//!
//! It is deliberately thin. When the script engine exists, what changes is where
//! the decisions come from, not their shape: the game still says "they pressed
//! the left flipper" and somebody still translates that into which solenoid gets
//! energized.
//!
//! # The mapping
//!
//! | key | what it does |
//! |-----|--------------|
//! | `Z` | left flipper |
//! | `M` | right flipper |
//! | space | **held**, pulls the plunger; on release, it fires |
//! | arrows ← ↑ → | nudge the cabinet |
//!
//! The plunger is the only one that tells pressing apart from releasing: the
//! strength of the shot comes from how long it was held down, so the key is not
//! a trigger but a state. Nudges are the other way around: a single impulse,
//! and how long it lasts is decided by the cabinet's spring, not by the finger.

use crate::physics::{Button, assign_flippers, player_plunger};
use vpin::vpx::VPX;
use vpw_physics::engine::{Engine, Shape};

/// Which way each key pushes, in degrees and with the original's convention
/// (`InputManager.cpp:735-739`): zero is pushing the table forwards.
///
/// They are not 90 and 270 but 75 and 285: a nudge from the side also sends the
/// table a little forwards, because it is impossible to hit a hundred and
/// thirteen kilo cabinet exactly side-on.
const NUDGE_ANGLE_LEFT: f32 = 75.0;
const NUDGE_ANGLE_RIGHT: f32 = 285.0;
const NUDGE_ANGLE_CENTER: f32 = 0.0;

/// What the player can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    LeftFlipper,
    RightFlipper,
    Plunger,
    NudgeLeft,
    NudgeRight,
    NudgeCenter,
}

impl Action {
    /// A key's action, if it has one.
    ///
    /// The codes are the browser's `KeyboardEvent.code`, which identify the
    /// **physical position** of the key and not the letter printed on it. That
    /// is what you have to use for a game: on a French keyboard the key that
    /// says `Z` is somewhere else, and whoever is playing expects the one under
    /// their finger to respond.
    pub fn from_key_code(code: &str) -> Option<Self> {
        match code {
            "KeyZ" => Some(Action::LeftFlipper),
            "KeyM" => Some(Action::RightFlipper),
            "Space" => Some(Action::Plunger),
            "ArrowLeft" => Some(Action::NudgeLeft),
            "ArrowRight" => Some(Action::NudgeRight),
            "ArrowUp" => Some(Action::NudgeCenter),
            _ => None,
        }
    }

    /// Whether the action is a shove at the cabinet.
    pub fn is_nudge(self) -> bool {
        matches!(
            self,
            Action::NudgeLeft | Action::NudgeRight | Action::NudgeCenter
        )
    }
}

/// Which piece of the table each action moves.
///
/// It is built once at load time and after that it only gets handed keys.
#[derive(Debug, Clone, Default)]
pub struct Controls {
    left: Vec<usize>,
    right: Vec<usize>,
    plunger: Option<usize>,
    /// Whether the plunger is being pulled right now. Without this the browser
    /// repeats the `keydown` while the key is held and every repetition would
    /// restart the pull.
    pulling: bool,
    /// Which nudges have their key held down. For the same reason as the
    /// plunger: the browser repeats the `keydown` about fifteen times a second,
    /// and one nudge per repetition tilts the table in a second flat.
    nudging: [bool; 3],
    /// Whether the table was already tilted the last time we looked, so
    /// everything is released once and not on every frame.
    tilt_seen: bool,
    /// Which flipper buttons are physically held, whatever the flippers are
    /// doing about it. Kept so the relay coming back finds the buttons where
    /// the player left them.
    holding: [bool; 2],
    /// Whether the machine is giving the flippers power. See
    /// [`Controls::set_flippers_powered`].
    powered: bool,
}

impl Controls {
    /// Builds the controls for a table already loaded into the engine.
    pub fn new(vpx: &VPX, engine: &Engine) -> Self {
        let assignment = assign_flippers(vpx, engine.shapes());
        Self {
            left: assignment
                .iter()
                .filter(|(_, b)| *b == Button::Left)
                .map(|(i, _)| *i)
                .collect(),
            right: assignment
                .iter()
                .filter(|(_, b)| *b == Button::Right)
                .map(|(i, _)| *i)
                .collect(),
            plunger: player_plunger(vpx, engine.shapes()),
            pulling: false,
            nudging: [false; 3],
            tilt_seen: false,
            holding: [false; 2],
            powered: true,
        }
    }

    /// Follows the machine's flipper relay.
    ///
    /// A real machine cuts flipper power between balls, in the operator's
    /// menu and on a tilt — the same relay that silences the flipper sound —
    /// and a button held across the cut behaves like the real thing: the
    /// flipper falls when the power goes, and rises again the moment it
    /// comes back, which is also what `vpmFlips` does (`core.vbs:2132`).
    pub fn set_flippers_powered(&mut self, engine: &mut Engine, powered: bool) {
        if self.powered == powered {
            return;
        }
        self.powered = powered;
        let tilt = engine.is_tilted();
        for (side, &held) in [&self.left, &self.right].into_iter().zip(&self.holding) {
            for &i in side {
                engine.set_flipper_solenoid(i, held && powered && !tilt);
            }
        }
    }

    /// The flippers on each side, for drawing them.
    pub fn flippers(&self, button: Button) -> &[usize] {
        match button {
            Button::Left => &self.left,
            Button::Right => &self.right,
        }
    }

    pub fn plunger(&self) -> Option<usize> {
        self.plunger
    }

    /// Applies an action to the engine.
    ///
    /// `pressed` tells the `keydown` from the `keyup`. Keyboard repeats are
    /// harmless: pressing a flipper that is already pressed does nothing, and
    /// both the plunger and the nudges guard themselves with their own flag.
    ///
    /// With the table tilted nothing responds. That is the whole punishment: the
    /// flippers go dead and the cabinet does not move any more.
    pub fn apply(&mut self, engine: &mut Engine, action: Action, pressed: bool) {
        let tilt = engine.is_tilted();
        match action {
            Action::LeftFlipper => {
                self.holding[0] = pressed;
                for &i in &self.left {
                    engine.set_flipper_solenoid(i, pressed && !tilt && self.powered);
                }
            }
            Action::RightFlipper => {
                self.holding[1] = pressed;
                for &i in &self.right {
                    engine.set_flipper_solenoid(i, pressed && !tilt && self.powered);
                }
            }
            Action::Plunger => {
                let Some(i) = self.plunger else { return };
                if pressed && !tilt {
                    if !self.pulling {
                        self.pulling = true;
                        engine.pull_plunger(i);
                    }
                } else if self.pulling {
                    self.pulling = false;
                    engine.release_plunger(i);
                }
            }
            _ => self.nudge(engine, action, pressed),
        }
    }

    /// A shove at the cabinet, on the `keydown` and only once per press.
    fn nudge(&mut self, engine: &mut Engine, action: Action, pressed: bool) {
        let (slot, angle) = match action {
            Action::NudgeLeft => (0, NUDGE_ANGLE_LEFT),
            Action::NudgeRight => (1, NUDGE_ANGLE_RIGHT),
            Action::NudgeCenter => (2, NUDGE_ANGLE_CENTER),
            _ => return,
        };
        if !pressed {
            self.nudging[slot] = false;
            return;
        }
        if self.nudging[slot] {
            return;
        }
        self.nudging[slot] = true;
        engine.nudge_at(angle);
    }

    /// Checks whether the table has just tilted and, if it has, releases
    /// everything.
    ///
    /// It goes once per frame. Without this, a flipper that was up when the tilt
    /// arrived **stays up**: the `keyup` that would bring it down arrives later,
    /// and by then `apply` no longer touches it.
    ///
    /// Returns `true` the first time it sees the tilt, so the game can announce
    /// it.
    pub fn check_tilt(&mut self, engine: &mut Engine) -> bool {
        let tilt = engine.is_tilted();
        if tilt == self.tilt_seen {
            return false;
        }
        self.tilt_seen = tilt;
        if tilt {
            self.release_all(engine);
            return true;
        }
        false
    }

    /// Translates a browser key and applies it. Returns whether it recognized
    /// it.
    pub fn key(&mut self, engine: &mut Engine, code: &str, pressed: bool) -> bool {
        match Action::from_key_code(code) {
            Some(a) => {
                self.apply(engine, a, pressed);
                true
            }
            None => false,
        }
    }

    /// Releases everything. Useful when the window loses focus: otherwise a
    /// flipper can stay up because another window got the `keyup`.
    pub fn release_all(&mut self, engine: &mut Engine) {
        self.apply(engine, Action::LeftFlipper, false);
        self.apply(engine, Action::RightFlipper, false);
        self.apply(engine, Action::Plunger, false);
        self.nudging = [false; 3];
    }
}

/// Where a new ball goes: right in front of the plunger.
pub fn ball_start_point(engine: &Engine, plunger: usize, radius: f32) -> Option<vpw_math::Vec3> {
    let Some(Shape::Plunger(p)) = engine.shapes().get(plunger) else {
        return None;
    };
    let c = p.bbox();
    Some(vpw_math::Vec3::new(
        (c.min.x + c.max.x) * 0.5,
        p.pos - radius,
        c.min.z + radius,
    ))
}
