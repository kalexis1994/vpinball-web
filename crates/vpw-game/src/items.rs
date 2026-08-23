//! The table's parts, as the script sees them.
//!
//! A table's VBScript spends nearly all its time talking to objects it did not
//! create: `LeftFlipper.RotateToEnd`, `Bumper1.TimerEnabled = True`,
//! `Light3.State = 1`. Those names are the table's game items, and this module
//! is what answers them.
//!
//! # What is here, and why exactly this
//!
//! The set of members was measured rather than guessed. Parsing the scripts of
//! four published tables and Visual Pinball's `core.vbs` and counting every
//! `x.Member` gives a bounded list with a very long tail: `Visible` is used a
//! hundred times, `TimerEnabled` a dozen, and most of the hundred distinct
//! names once each. What is implemented is the head of that distribution plus
//! everything a part needs to be *playable*.
//!
//! The tail is **accepted and remembered** rather than rejected. A table that
//! sets `.ColorGradeImage` or `.Fader` is asking for something presentational
//! that this renderer does not do yet, and refusing it would stop the table
//! dead over a colour. It is also closer to the original than refusing: a real
//! Visual Pinball part has all of these members, so answering "no such member"
//! is the less faithful of the two mistakes. The cost is that a genuine typo in
//! a table's script goes unnoticed, which is a trade this project can afford
//! and Visual Pinball, as an editor, could not.
//!
//! # Where the state lives
//!
//! Split in two, and the split is the interesting part:
//!
//! - Anything the **physics** owns — a flipper's angle, a gate being open, a
//!   kicker holding a ball — is read from and written to the engine. There is
//!   no copy, so a script and the simulation can never disagree.
//! - Anything the physics does not know about — `Visible`, a light's `State`,
//!   a timer's interval — lives here, in cells. The renderer reads it back.
//!
//! Getting that boundary wrong is how a port ends up with a flipper that looks
//! raised and collides lowered.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use vpin::vpx::VPX;
use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::engine::{Engine, Shape};
use vpw_vbscript::error::{Error, Result};
use vpw_vbscript::object::Object;
use vpw_vbscript::value::Value;

/// What kind of part this is, which decides what members it answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flipper,
    Bumper,
    Kicker,
    Gate,
    Spinner,
    Trigger,
    Plunger,
    Light,
    Wall,
    Ramp,
    Rubber,
    Primitive,
    Flasher,
    Target,
    Timer,
    Decal,
    TextBox,
    /// Something in the file we have no model for. It still answers the common
    /// members, because a script that sets `.Visible` on one should not fall
    /// over.
    Other,
}

impl Kind {
    pub fn of(item: &vpin::vpx::gameitem::GameItemEnum) -> Self {
        use vpin::vpx::gameitem::GameItemEnum as G;
        match item {
            G::Flipper(_) => Kind::Flipper,
            G::Bumper(_) => Kind::Bumper,
            G::Kicker(_) => Kind::Kicker,
            G::Gate(_) => Kind::Gate,
            G::Spinner(_) => Kind::Spinner,
            G::Trigger(_) => Kind::Trigger,
            G::Plunger(_) => Kind::Plunger,
            G::Light(_) => Kind::Light,
            G::Wall(_) => Kind::Wall,
            G::Ramp(_) => Kind::Ramp,
            G::Rubber(_) => Kind::Rubber,
            G::Primitive(_) => Kind::Primitive,
            G::Flasher(_) => Kind::Flasher,
            G::HitTarget(_) => Kind::Target,
            G::Timer(_) => Kind::Timer,
            G::Decal(_) => Kind::Decal,
            G::TextBox(_) => Kind::TextBox,
            _ => Kind::Other,
        }
    }
}

/// The state of a part that the physics does not model.
/// How far a drop target sinks when it goes down.
/// `DROP_TARGET_LIMIT`, `hittarget.h:71`.
pub const DROP_DEPTH: f32 = 52.0;

#[derive(Debug)]
struct Visual {
    visible: Cell<bool>,
    /// A light's `State`: 0 off, 1 on, 2 blinking.
    state: Cell<i32>,
    intensity_scale: Cell<f64>,
    image: RefCell<Rc<str>>,
    text: RefCell<Rc<str>>,
    /// A primitive's nine placement numbers, as the file stores them: `RotX/Y/Z`
    /// at 0..2, `TransX/Y/Z` at 3..5 and `ObjRotX/Y/Z` at 6..8.
    ///
    /// Kept as the one array rather than as separate rotations and
    /// translations because that is what the transform is built from, and
    /// because `RotZ` and `ObjRotZ` are **different numbers applied at
    /// different stages** — folding them together turns a flipper bat that
    /// swings into one that sits still at a wrong angle.
    rot_and_tra: Cell<[f32; 9]>,
    disable_lighting: Cell<f64>,
    /// Whether the part collides at all. A script turns this off to let a ball
    /// through a wall it has just opened.
    collidable: Cell<bool>,
    /// A drop target that is down. See [`Item::set_dropped`].
    dropped: Cell<bool>,
    /// Whether this target is one of the three that drop, as against a stand-up
    /// that only takes the hit. Both are `HitTarget` in the file and only the
    /// type tells them apart (`hittarget.cpp:231`).
    drops: Cell<bool>,
    /// A change of [`Visual::dropped`] that nobody has been told about yet.
    ///
    /// A target does not know the script exists, so it cannot call
    /// `Name_Raised` itself. It leaves the news here and the game picks it up
    /// on the next step, the same way the physics leaves its collisions in a
    /// queue rather than reaching into the interpreter.
    announce: Cell<Option<bool>>,
    /// Everything else the script sets on this part. See the module note: a
    /// member we do not model is kept rather than refused, so it at least reads
    /// back as what was written.
    extra: RefCell<HashMap<Box<str>, Value>>,
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            visible: Cell::new(true),
            state: Cell::new(0),
            intensity_scale: Cell::new(1.0),
            image: RefCell::new(Rc::from("")),
            text: RefCell::new(Rc::from("")),
            rot_and_tra: Cell::new([0.0; 9]),
            disable_lighting: Cell::new(0.0),
            collidable: Cell::new(true),
            dropped: Cell::new(false),
            drops: Cell::new(false),
            announce: Cell::new(None),
            extra: RefCell::new(HashMap::new()),
        }
    }
}

/// A part's timer, which is a script-visible scheduler and not a decoration.
///
/// `X.TimerInterval = 100 : X.TimerEnabled = True` means "call `X_Timer` every
/// hundred milliseconds", and tables run whole animations off it. It is here
/// rather than in the physics because it counts table time and calls a script,
/// neither of which the physics has any business knowing about.
#[derive(Debug, Default)]
pub struct Timer {
    pub enabled: Cell<bool>,
    /// In milliseconds — or, if negative, not an interval at all.
    ///
    /// `-1` and `-2` are the two *frame* events, not intervals: see
    /// [`clamp_interval`] and `Player::FireTimers`. A table sets `-2` on the
    /// timer that polls the ROM controller, and reading that as "every
    /// millisecond" runs it sixteen times too often.
    pub interval: Cell<f64>,
    /// Table time at which it is next due, or `NaN` for a timer that has just
    /// been switched on and has not been given its first due time yet.
    pub due_ms: Cell<f64>,
}

/// `TimerInterval`, as the original stores it. `hittimer.cpp:16`:
///
/// ```cpp
/// m_interval = intervalMs >= 0 ? max(intervalMs, MAX_TIMER_MSEC_INTERVAL) : max(-2, intervalMs);
/// ```
///
/// Two separate things happen here. A non-negative interval is floored at one
/// millisecond, because the update loop runs at a thousand hertz and asking for
/// less is asking for something the loop cannot give. A negative one is *not*
/// an interval: `-1` means "once per rendered frame" and `-2` means "once per
/// frame, after the physics has caught up", and everything below `-2` is
/// clamped up to `-2`. `Update` ignores negative intervals entirely.
///
/// The truncation is not a rounding choice: `TimerInterval` is a `long` in
/// `vpinball.idl`, so a script that writes `1.5` has already written `1` before
/// the timer ever sees it.
pub fn clamp_interval(ms: f64) -> f64 {
    let ms = ms.trunc();
    if ms >= 0.0 { ms.max(1.0) } else { ms.max(-2.0) }
}

/// One part of the table.
pub struct Item {
    pub name: Rc<str>,
    pub kind: Kind,
    /// Index into `vpx.gameitems`.
    pub index: usize,
    /// The shapes this item produced, if any.
    pub shapes: Vec<usize>,
    /// The trigger this item is, if it is one.
    pub trigger: Option<usize>,
    pub timer: Timer,
    /// Where the part was placed and how big it is. Only a primitive has these
    /// as separate numbers; everything else keeps the defaults and never asks.
    position: Vec3,
    size: Vec3,
    visual: Visual,
    engine: Rc<RefCell<Engine>>,
    /// Shared by every item: set whenever any timer is armed, disarmed or
    /// given a new interval. See [`Items::armed`].
    timers_changed: Rc<Cell<bool>>,
}

impl Item {
    /// Whether the script changed anything the renderer has to notice.
    pub fn visible(&self) -> bool {
        self.visual.visible.get()
    }

    /// The part's nine placement numbers as they stand, which is how a script
    /// animates a toy the physics knows nothing about.
    pub fn placement(&self) -> [f32; 9] {
        self.visual.rot_and_tra.get()
    }

    /// Where this part sits, built from its placement numbers.
    ///
    /// The same chain the geometry uses for a baked primitive, so a piece that
    /// nothing has moved lands exactly where it was baked.
    pub fn placement_matrix(&self) -> vpw_math::Mat4 {
        vpw_table::geometry::primitive_transform_from_fields(
            self.position,
            self.size,
            self.visual.rot_and_tra.get(),
        )
    }

    /// A light's state: 0 off, 1 on, 2 blinking.
    /// How bright this lamp should be drawn, 0 to 1.
    ///
    /// `State` is the switch — 0 off, 1 on, 2 blinking — and `IntensityScale`
    /// is the dimmer, which is how a modern table fades a bulb up instead of
    /// snapping it on. `core.vbs` writes the second one every frame while a
    /// lamp is fading, so the two have to be read together.
    pub fn light_level(&self) -> f32 {
        let on = match self.light_state() {
            0 => 0.0,
            // Blinking is left on. Making it blink needs a clock this does not
            // have, and a lamp that stays lit is much closer to right than one
            // that never comes on.
            _ => 1.0,
        };
        (on * self.visual.intensity_scale.get() as f32).clamp(0.0, 1.0)
    }

    pub fn light_state(&self) -> i32 {
        self.visual.state.get()
    }

    /// The first shape this item owns, which is the one that carries its
    /// physics for the single-shape kinds.
    fn shape(&self) -> Option<usize> {
        self.shapes.first().copied()
    }

    fn with_shape<T>(&self, f: impl FnOnce(&Shape) -> T) -> Option<T> {
        let i = self.shape()?;
        let engine = self.engine.borrow();
        engine.shapes().get(i).map(f)
    }
}

impl Object for Item {
    fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::Flipper => "Flipper",
            Kind::Bumper => "Bumper",
            Kind::Kicker => "Kicker",
            Kind::Gate => "Gate",
            Kind::Spinner => "Spinner",
            Kind::Trigger => "Trigger",
            Kind::Plunger => "Plunger",
            Kind::Light => "Light",
            Kind::Wall => "Wall",
            Kind::Ramp => "Ramp",
            Kind::Rubber => "Rubber",
            Kind::Primitive => "Primitive",
            Kind::Flasher => "Flasher",
            Kind::Target => "HitTarget",
            Kind::Timer => "Timer",
            Kind::Decal => "Decal",
            Kind::TextBox => "TextBox",
            Kind::Other => "Object",
        }
    }

    fn get(&self, name: &str, args: &[Value]) -> Result<Value> {
        let lower = name.to_ascii_lowercase();
        if let Some(v) = self.common_get(&lower)? {
            return Ok(v);
        }
        let specific = match self.kind {
            Kind::Flipper => self.flipper_get(&lower),
            Kind::Kicker => self.kicker_get(&lower, args),
            Kind::Gate => self.gate_get(&lower),
            Kind::Spinner => self.spinner_get(&lower),
            Kind::Plunger => self.plunger_get(&lower),
            Kind::Bumper => self.bumper_get(&lower),
            _ => Err(Error::no_such_member(name)),
        };
        match specific {
            Ok(v) => Ok(v),
            Err(_) => Ok(self.remembered(&lower)),
        }
    }

    fn set(&self, name: &str, _args: &[Value], value: Value, _by_ref: bool) -> Result<()> {
        let lower = name.to_ascii_lowercase();
        if self.common_set(&lower, &value)? {
            return Ok(());
        }
        let specific = match self.kind {
            Kind::Flipper => self.flipper_set(&lower, &value),
            Kind::Gate => self.gate_set(&lower, &value),
            Kind::Kicker => self.kicker_set(&lower, &value),
            Kind::Bumper => self.bumper_set(&lower, &value),
            _ => Err(Error::no_such_member(name)),
        };
        if specific.is_err() {
            self.remember(&lower, value);
        }
        Ok(())
    }
}

impl Item {
    /// A member we do not model, as it was last written.
    fn remembered(&self, name: &str) -> Value {
        self.visual
            .extra
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or(Value::Empty)
    }

    fn remember(&self, name: &str, value: Value) {
        log::debug!("{}: '{name}' is not modelled; keeping it as set", self.name);
        self.visual.extra.borrow_mut().insert(name.into(), value);
    }

    /// Anything the script set that this port does not model, for a host that
    /// wants to know what a table is asking for.
    pub fn unmodelled(&self) -> Vec<String> {
        self.visual
            .extra
            .borrow()
            .keys()
            .map(|k| k.to_string())
            .collect()
    }

    // -- members every part has -------------------------------------------

    fn common_get(&self, name: &str) -> Result<Option<Value>> {
        let v = &self.visual;
        Ok(Some(match name {
            "name" => Value::Str(self.name.clone()),
            "visible" => Value::Bool(v.visible.get()),
            "image" => Value::Str(v.image.borrow().clone()),
            "text" => Value::Str(v.text.borrow().clone()),
            "state" => Value::Long(v.state.get()),
            "intensityscale" => Value::Double(v.intensity_scale.get()),
            "disablelighting" | "disablelightingtop" => Value::Double(v.disable_lighting.get()),
            "collidable" | "iscollidable" => Value::Bool(v.collidable.get()),
            "isdropped" => Value::Bool(v.dropped.get()),
            "timerenabled" => Value::Bool(self.timer.enabled.get()),
            "timerinterval" | "interval" => Value::Double(self.timer.interval.get()),
            // A Timer object is the one kind whose `Enabled` is the timer:
            // everything else on a table spells that `TimerEnabled` and keeps
            // `Enabled` for whether it is in play. `core.vbs` turns the whole
            // ROM poll on with `PinMAMETimer.Enabled = True`, so reading this
            // as "collidable" is how a ROM table ends up never polling at all.
            "enabled" if self.kind == Kind::Timer => Value::Bool(self.timer.enabled.get()),
            // `Enabled` means different things per kind; the ones that have a
            // real meaning override it below.
            "enabled" => Value::Bool(v.collidable.get()),
            "rotx" => Value::Double(v.rot_and_tra.get()[0].into()),
            "roty" => Value::Double(v.rot_and_tra.get()[1].into()),
            "rotz" => Value::Double(v.rot_and_tra.get()[2].into()),
            "transx" => Value::Double(v.rot_and_tra.get()[3].into()),
            "transy" => Value::Double(v.rot_and_tra.get()[4].into()),
            "transz" => Value::Double(v.rot_and_tra.get()[5].into()),
            "objrotx" => Value::Double(v.rot_and_tra.get()[6].into()),
            "objroty" => Value::Double(v.rot_and_tra.get()[7].into()),
            "objrotz" => Value::Double(v.rot_and_tra.get()[8].into()),
            _ => return Ok(None),
        }))
    }

    /// Returns whether the member was handled.
    fn common_set(&self, name: &str, value: &Value) -> Result<bool> {
        let v = &self.visual;
        match name {
            "visible" => v.visible.set(value.to_bool()?),
            "image" => *v.image.borrow_mut() = value.to_str()?,
            "text" => *v.text.borrow_mut() = value.to_str()?,
            "state" => v.state.set(value.to_int()?),
            "intensityscale" => v.intensity_scale.set(value.to_number()?),
            "disablelighting" | "disablelightingtop" => {
                v.disable_lighting.set(value.to_number()?);
            }
            "collidable" | "iscollidable" => {
                let on = value.to_bool()?;
                v.collidable.set(on);
                self.set_collidable(on && !v.dropped.get());
            }
            "isdropped" => self.set_dropped(value.to_bool()?),
            "enabled" if self.kind == Kind::Timer => {
                self.timer.enabled.set(value.to_bool()?);
                self.timer.due_ms.set(f64::NAN);
                self.timers_changed.set(true);
            }
            "timerenabled" => {
                self.timer.enabled.set(value.to_bool()?);
                // Switching a timer on restarts its countdown: the original
                // runs the whole of `SetInterval` here, and that sets the next
                // fire to *now plus one interval*
                // (`Player::TimerStateChange`, `player.cpp:1361`). It does not
                // fire on the next tick — a table that arms a one-second timer
                // gets it in a second, not immediately. `NaN` is that "not
                // scheduled yet" state; the loop, which is the only code that
                // knows what time it is, fills it in.
                self.timer.due_ms.set(f64::NAN);
                self.timers_changed.set(true);
            }
            "timerinterval" | "interval" => {
                self.timer.interval.set(clamp_interval(value.to_number()?));
                // The interval decides *which* list this timer belongs to —
                // the clock's, the frame's, or neither — so a change to it
                // invalidates the cache just as arming it does.
                self.timers_changed.set(true);
            }
            "rotx" => set_placement(&v.rot_and_tra, 0, value.to_number()?),
            "roty" => set_placement(&v.rot_and_tra, 1, value.to_number()?),
            "rotz" => set_placement(&v.rot_and_tra, 2, value.to_number()?),
            "transx" => set_placement(&v.rot_and_tra, 3, value.to_number()?),
            "transy" => set_placement(&v.rot_and_tra, 4, value.to_number()?),
            "transz" => set_placement(&v.rot_and_tra, 5, value.to_number()?),
            "objrotx" => set_placement(&v.rot_and_tra, 6, value.to_number()?),
            "objroty" => set_placement(&v.rot_and_tra, 7, value.to_number()?),
            "objrotz" => set_placement(&v.rot_and_tra, 8, value.to_number()?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Drops a target out of the playfield, or raises it back.
    ///
    /// A dropped target stops colliding entirely. That is not an approximation
    /// of the original: it skips every collider belonging to a dropped target
    /// before it tests anything (`DoHitTest`, `collide.cpp:607`, where the
    /// comment beside it wonders why it is done there and not in the hit tests
    /// themselves). The mesh does not move in the physics at all — the original
    /// only lowers the vertices it draws with (`UpdateTarget`,
    /// `hittarget.cpp:697`) — so this is the whole of it.
    ///
    /// The visible half is [`DROP_DEPTH`] of downward translation, which the
    /// renderer picks up the same way it picks up a moving primitive.
    fn set_dropped(&self, down: bool) {
        if self.visual.dropped.get() == down {
            return;
        }
        self.visual.dropped.set(down);
        self.visual.announce.set(Some(down));
        self.set_collidable(!down && self.visual.collidable.get());
        set_placement(
            &self.visual.rot_and_tra,
            5,
            if down { -f64::from(DROP_DEPTH) } else { 0.0 },
        );
    }

    /// Whether this is a target that is currently down.
    pub fn is_dropped(&self) -> bool {
        self.visual.dropped.get()
    }

    /// Takes the news of this target having dropped or risen, if there is any.
    pub fn take_announcement(&self) -> Option<bool> {
        self.visual.announce.take()
    }

    /// Whether this part is a drop target, as against a stand-up.
    pub fn drops(&self) -> bool {
        self.visual.drops.get()
    }

    /// Knocks a drop target down because a ball hit it.
    ///
    /// Returns whether it went down, so the caller can raise the `Dropped`
    /// event only when something actually happened. A stand-up target, or one
    /// that is already down, returns false.
    ///
    /// The original reaches the same place by a longer road: the collision sets
    /// a flag on the target, and the animation that runs on the next frame
    /// carries it down over some tens of milliseconds and *then* declares it
    /// dropped (`UpdateAnimation`, `hittarget.cpp:576`). The travel is
    /// cosmetic — the colliders never move — and what the game reacts to is the
    /// switch, which closes on the hit either way.
    pub fn knock_down(&self) -> bool {
        if !self.visual.drops.get() || self.visual.dropped.get() {
            return false;
        }
        self.set_dropped(true);
        self.visual.announce.set(None);
        true
    }

    /// Turning a part's collision off has to reach the physics, or a script
    /// that opens a gate leaves an invisible wall behind.
    fn set_collidable(&self, on: bool) {
        let mut engine = self.engine.borrow_mut();
        for &s in &self.shapes {
            engine.set_shape_enabled(s, on);
        }
        if let Some(t) = self.trigger {
            engine.set_trigger_enabled(t, on);
        }
    }

    // -- flipper ----------------------------------------------------------

    fn flipper_get(&self, name: &str) -> Result<Value> {
        let value = self.with_shape(|s| match s {
            Shape::Flipper(f) => match name {
                "currentangle" => Some(Value::Double(f.angle_cur.to_degrees().into())),
                "startangle" => Some(Value::Double(f.angle_start.to_degrees().into())),
                "endangle" => Some(Value::Double(f.angle_end.to_degrees().into())),
                "strength" => Some(Value::Double(f.strength.into())),
                "elasticity" => Some(Value::Double(f.material.elasticity.into())),
                "friction" => Some(Value::Double(f.material.friction.into())),
                "length" => Some(Value::Double(f.flipper_radius.into())),
                "enabled" => Some(Value::Bool(true)),
                _ => None,
            },
            _ => None,
        });
        match value.flatten() {
            Some(v) => Ok(v),
            // `RotateToEnd` and `RotateToStart` are methods, and they are what
            // a table's key handler actually calls.
            None => match name {
                "rotatetoend" => {
                    self.set_solenoid(true);
                    Ok(Value::Empty)
                }
                "rotatetostart" => {
                    self.set_solenoid(false);
                    Ok(Value::Empty)
                }
                _ => Err(Error::no_such_member(name)),
            },
        }
    }

    fn set_solenoid(&self, on: bool) {
        let Some(i) = self.shape() else { return };
        self.engine.borrow_mut().set_flipper_solenoid(i, on);
    }

    fn flipper_set(&self, name: &str, value: &Value) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member(name));
        };
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Flipper(f)) = engine.shape_mut(i) else {
            return Err(Error::no_such_member(name));
        };
        match name {
            "strength" => f.strength = value.to_number()? as f32,
            "elasticity" => f.material.elasticity = value.to_number()? as f32,
            "friction" => f.material.friction = value.to_number()? as f32,
            "endangle" => f.angle_end = (value.to_number()? as f32).to_radians(),
            "startangle" => f.angle_start = (value.to_number()? as f32).to_radians(),
            // The two the script uses to drive it directly.
            "rotatetoend" => f.solenoid = true,
            "rotatetostart" => f.solenoid = false,
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }

    // -- bumper, gate, spinner --------------------------------------------

    fn bumper_get(&self, name: &str) -> Result<Value> {
        self.with_shape(|s| match s {
            Shape::Bumper(b) => match name {
                "force" => Some(Value::Double(b.force.into())),
                "threshold" => Some(Value::Double(b.threshold.into())),
                "hashitevent" => Some(Value::Bool(true)),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .ok_or_else(|| Error::no_such_member(name))
    }

    fn bumper_set(&self, name: &str, value: &Value) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member(name));
        };
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Bumper(b)) = engine.shape_mut(i) else {
            return Err(Error::no_such_member(name));
        };
        match name {
            "force" => b.force = value.to_number()? as f32,
            "threshold" => b.threshold = value.to_number()? as f32,
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }

    fn gate_get(&self, name: &str) -> Result<Value> {
        self.with_shape(|s| match s {
            Shape::Gate(g) => match name {
                "currentangle" => Some(Value::Double(g.angle.to_degrees().into())),
                "open" => Some(Value::Bool(g.open)),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .ok_or_else(|| Error::no_such_member(name))
    }

    fn gate_set(&self, name: &str, value: &Value) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member(name));
        };
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Gate(g)) = engine.shape_mut(i) else {
            return Err(Error::no_such_member(name));
        };
        match name {
            // A script holding a gate open is how a table makes a one-way
            // passage temporarily two-way.
            "open" => {
                g.open = value.to_bool()?;
                g.forced_move = true;
            }
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }

    fn spinner_get(&self, name: &str) -> Result<Value> {
        self.with_shape(|s| match s {
            Shape::Spinner(sp) => match name {
                "currentangle" => Some(Value::Double(sp.angle.to_degrees().into())),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .ok_or_else(|| Error::no_such_member(name))
    }

    // -- kicker -----------------------------------------------------------

    fn kicker_get(&self, name: &str, args: &[Value]) -> Result<Value> {
        match name {
            "ballcntover" => {
                let held = self
                    .with_shape(|s| matches!(s, Shape::Kicker(k) if k.captured.is_some()))
                    .unwrap_or(false);
                Ok(Value::Long(i32::from(held)))
            }
            // `CreateBall`, and the two sized forms. `core.vbs` picks one by
            // Visual Pinball version and modern tables all land on
            // `CreateSizedBallWithMass` — which is the call that actually
            // serves the ball at the start of a game, so a kicker that does not
            // answer it gives a machine that starts a game with no ball in it.
            //
            // The radius is honoured; the mass is not, because the physics
            // treats every ball as the same mass and a ball that weighed
            // differently would need the whole collision response to care.
            "createball" | "createsizedball" | "createsizedballwithmass" => {
                let radius = args
                    .first()
                    .map(Value::to_number)
                    .transpose()?
                    .map(|r| r as f32)
                    .filter(|r| *r > 0.0)
                    .unwrap_or(vpw_physics::constants::DEFAULT_BALL_SIZE);
                let index = self.create_ball(radius)?;
                // The original returns the ball it made (`kicker.cpp:663`,
                // `*pResult = pball`) and a table does far more with it than
                // set an image. F-14 animates its bumper popper by hand:
                //
                // ```vbscript
                // Set popperBall = sw24a.Createball
                // popperBall.Z = 0
                // ```
                //
                // Answer `Empty` and the second line raises, which takes the
                // rest of the sub with it — including the timer that would have
                // destroyed the ball and told the stack it was gone. The ROM
                // then re-serves the popper for ever, a ball at a time.
                Ok(Value::Object(Rc::new(crate::BallObject {
                    engine: self.engine.clone(),
                    index,
                })))
            }
            "destroyball" => {
                self.destroy_ball();
                Ok(Value::Empty)
            }
            // `Kick angle, speed [, inclination]`: the whole point of a kicker.
            "kick" => {
                let angle = args
                    .first()
                    .map(Value::to_number)
                    .transpose()?
                    .unwrap_or(0.0);
                let speed = args
                    .get(1)
                    .map(Value::to_number)
                    .transpose()?
                    .unwrap_or(0.0);
                self.kick(angle, speed)?;
                Ok(Value::Empty)
            }
            _ => Err(Error::no_such_member(name)),
        }
    }

    fn kicker_set(&self, name: &str, value: &Value) -> Result<()> {
        match name {
            "enabled" => {
                let on = value.to_bool()?;
                self.visual.collidable.set(on);
                self.set_collidable(on);
                Ok(())
            }
            _ => Err(Error::no_such_member(name)),
        }
    }

    /// Puts a ball in this kicker, which is how a table serves one.
    ///
    /// Returns the ball's index, because the script gets the ball back and
    /// tables use it — see the caller.
    fn create_ball(&self, radius: f32) -> Result<usize> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member("CreateBall"));
        };
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Kicker(k)) = engine.shapes().get(i) else {
            return Err(Error::no_such_member("CreateBall"));
        };
        let at = Vec3::new(
            k.circle.center.x,
            k.circle.center.y,
            k.circle.z_low + radius,
        );
        let mut ball = Ball::new(at, radius);
        // `kicker.cpp:660` serves it with a nudge along +x. It matters only
        // when the kicker turns out to be occupied and refuses to hold this
        // one: then the nudge is what rolls it back out instead of leaving it
        // parked on top of the ball already there. A kicker that does take it
        // zeroes the velocity on capture, exactly as before.
        ball.vel = Vec3::new(0.1, 0.0, 0.0);
        let ball = engine.add_ball(ball);
        engine.capture_ball(i, ball);
        Ok(ball)
    }

    fn destroy_ball(&self) {
        let Some(i) = self.shape() else { return };
        self.engine.borrow_mut().destroy_captured_ball(i);
    }

    /// `Kick angle, speed`: the angle is in degrees, measured from straight up
    /// the table and turning clockwise, which is Visual Pinball's convention
    /// and not the one a maths library would pick.
    fn kick(&self, angle_deg: f64, speed: f64) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member("Kick"));
        };
        let a = (angle_deg as f32).to_radians();
        let v = Vec3::new(a.sin(), -a.cos(), 0.0) * speed as f32;
        self.engine.borrow_mut().release_from_kicker(i, v);
        Ok(())
    }

    // -- plunger ----------------------------------------------------------

    fn plunger_get(&self, name: &str) -> Result<Value> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member(name));
        };
        match name {
            "fire" => {
                self.engine.borrow_mut().release_plunger(i);
                Ok(Value::Empty)
            }
            "pullback" => {
                self.engine.borrow_mut().pull_plunger(i);
                Ok(Value::Empty)
            }
            "position" => {
                let p = self
                    .with_shape(|s| match s {
                        Shape::Plunger(p) => Some(p.relative_position()),
                        _ => None,
                    })
                    .flatten()
                    .unwrap_or(0.0);
                // The original reports 0 at the front and 25 fully drawn back.
                Ok(Value::Double(f64::from(p) * 25.0))
            }
            _ => Err(Error::no_such_member(name)),
        }
    }
}

fn set_placement(cell: &Cell<[f32; 9]>, i: usize, v: f64) {
    let mut a = cell.get();
    a[i] = v as f32;
    cell.set(a);
}

/// Every part of the table, by name.
pub struct Items {
    by_name: HashMap<Box<str>, Rc<Item>>,
    /// In game-item order, so an index can go back to a name.
    all: Vec<Rc<Item>>,
    /// The items with a timer switched on, and whether that list is stale.
    ///
    /// The loop asks for this a thousand times a second and a table has a
    /// handful of timers among hundreds of parts — F-14 arms six of seven
    /// hundred and forty-one. Walking them all to find the six is most of what
    /// the timer phase costs, and it is pure searching: the answer only changes
    /// when a script arms or disarms something, which happens a few times a
    /// game. So the answer is kept and the writes say when it is stale.
    armed: RefCell<Vec<Rc<Item>>>,
    dirty: Rc<Cell<bool>>,
}

impl Items {
    /// Builds the set from a parsed table and its collision shapes.
    pub fn new(
        vpx: &vpin::vpx::VPX,
        collision: &vpw_table::physics::Collision,
        engine: Rc<RefCell<Engine>>,
    ) -> Self {
        // Shared by every item built below, so that any of them can mark the
        // armed-timer list stale without knowing anything about the others.
        let dirty = Rc::new(Cell::new(true));

        // `physics::triggers` walks the game items in order and keeps the
        // enabled ones, so the n-th trigger it produced is the n-th enabled
        // trigger in the file. That is the only way back to a name.
        let mut trigger_index = 0usize;

        let mut by_name = HashMap::new();
        let mut all = Vec::new();
        for (index, item) in vpx.gameitems.iter().enumerate() {
            let kind = Kind::of(item);
            let name: Rc<str> = Rc::from(item_name(item));
            if name.is_empty() {
                continue;
            }

            let trigger = match item {
                vpin::vpx::gameitem::GameItemEnum::Trigger(t) if t.is_enabled => {
                    let i = trigger_index;
                    trigger_index += 1;
                    Some(i)
                }
                _ => None,
            };

            let visual = Visual::default();
            visual.visible.set(item_visible(item));
            visual.rot_and_tra.set(item_placement(item));
            if let vpin::vpx::gameitem::GameItemEnum::Light(l) = item {
                // A light that the file says starts on.
                visual.state.set(i32::from(l.state.unwrap_or(0.0) != 0.0));
            }
            if let vpin::vpx::gameitem::GameItemEnum::HitTarget(t) = item {
                use vpin::vpx::gameitem::hittarget::TargetType as T;
                visual.drops.set(matches!(
                    t.target_type,
                    T::DropTargetBeveled | T::DropTargetSimple | T::DropTargetFlatSimple
                ));
                visual.collidable.set(t.is_collidable);
                // A table can be saved with a bank already knocked down.
                visual.dropped.set(t.is_dropped);
            }

            let entry = Rc::new(Item {
                name: name.clone(),
                kind,
                index,
                shapes: collision.shapes_of(index).collect(),
                trigger,
                timer: item_timer(item),
                position: item_position(item),
                size: item_size(item),
                visual,
                engine: engine.clone(),
                timers_changed: dirty.clone(),
            });
            by_name.insert(name.to_ascii_lowercase().into_boxed_str(), entry.clone());
            all.push(entry);
        }
        Self {
            by_name,
            all,
            armed: RefCell::new(Vec::new()),
            // Nothing has been collected yet, so the empty list is stale.
            dirty,
        }
    }

    /// The items with a timer switched on.
    ///
    /// Rebuilt only when something has changed it since the last call; see the
    /// field. The caller gets a borrow, so it must copy out what it needs
    /// before running any script — a handler is free to arm another timer, and
    /// that writes through this same list.
    pub fn armed(&self) -> std::cell::Ref<'_, Vec<Rc<Item>>> {
        if self.dirty.replace(false) {
            let mut armed = self.armed.borrow_mut();
            armed.clear();
            armed.extend(self.all.iter().filter(|i| i.timer.enabled.get()).cloned());
        }
        self.armed.borrow()
    }

    pub fn get(&self, name: &str) -> Option<Rc<Item>> {
        self.by_name
            .get(name.to_ascii_lowercase().as_str())
            .cloned()
    }

    /// The table's collections, by the name a script uses.
    ///
    /// A collection is a named group of parts, made in the editor and never
    /// mentioned in the script that uses it — F-14 walks `For Each xx In GI`
    /// to turn the general illumination on and off, and `GI` is declared
    /// nowhere. Under `Option Explicit` that is not a subtle failure: the whole
    /// sub raises and the table's lights never move.
    pub fn collections(&self, vpx: &VPX) -> HashMap<Box<str>, Vec<Rc<Item>>> {
        let mut out = HashMap::new();
        for collection in &vpx.collections {
            let members: Vec<Rc<Item>> = collection
                .items
                .iter()
                .filter_map(|n| self.get(n))
                .collect();
            out.insert(
                collection.name.to_ascii_lowercase().into_boxed_str(),
                members,
            );
        }
        out
    }

    /// The part at a given place in `vpx.gameitems`.
    pub fn by_index(&self, index: usize) -> Option<&Rc<Item>> {
        self.all.iter().find(|i| i.index == index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Rc<Item>> {
        self.all.iter()
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The item that owns a given collision shape.
    pub fn by_shape(&self, shape: usize) -> Option<&Rc<Item>> {
        self.all.iter().find(|i| i.shapes.contains(&shape))
    }

    /// The item that is a given trigger.
    pub fn by_trigger(&self, trigger: usize) -> Option<&Rc<Item>> {
        self.all.iter().find(|i| i.trigger == Some(trigger))
    }
}

fn item_name(item: &vpin::vpx::gameitem::GameItemEnum) -> &str {
    use vpin::vpx::gameitem::GameItemEnum as G;
    match item {
        G::Wall(w) => &w.name,
        G::Flipper(f) => &f.name,
        G::Timer(t) => &t.name,
        G::Plunger(p) => &p.name,
        G::TextBox(t) => &t.name,
        G::Bumper(b) => &b.name,
        G::Trigger(t) => &t.name,
        G::Light(l) => &l.name,
        G::Kicker(k) => &k.name,
        G::Decal(d) => &d.name,
        G::Gate(g) => &g.name,
        G::Spinner(s) => &s.name,
        G::Ramp(r) => &r.name,
        G::LightSequencer(l) => &l.name,
        G::Primitive(p) => &p.name,
        G::Flasher(f) => &f.name,
        G::Rubber(r) => &r.name,
        G::HitTarget(h) => &h.name,
        G::Reel(r) => &r.name,
        // A `Ball` in the file is a leftover from the editor and a
        // `PartGroup` is a folder; neither is something a script talks to.
        G::Ball(_) | G::PartGroup(_) | G::Generic(..) => "",
    }
}

/// The timer a part was saved with.
///
/// Not something a script sets up: it is a property of the part, ticked on in
/// the editor with an interval next to it. `core.vbs` leans on this — the
/// `RollingTimer` that follows the ball with the rolling sound is switched on
/// in the file and never mentioned in any `_Init` — so a port that starts every
/// timer off runs a table that never rolls, never blinks and never polls.
fn item_timer(item: &vpin::vpx::gameitem::GameItemEnum) -> Timer {
    use vpin::vpx::gameitem::GameItemEnum as G;
    let data = match item {
        G::Wall(w) => &w.timer,
        G::Flipper(f) => &f.timer,
        G::Timer(t) => &t.timer,
        G::Plunger(p) => &p.timer,
        G::TextBox(t) => &t.timer,
        G::Bumper(b) => &b.timer,
        G::Trigger(t) => &t.timer,
        G::Light(l) => &l.timer,
        G::Kicker(k) => &k.timer,
        G::Gate(g) => &g.timer,
        G::Spinner(s) => &s.timer,
        G::Ramp(r) => &r.timer,
        G::LightSequencer(l) => &l.timer,
        G::Flasher(f) => &f.timer,
        G::Rubber(r) => &r.timer,
        G::HitTarget(h) => &h.timer,
        G::Reel(r) => &r.timer,
        // A decal is paint, and a primitive's timer is one of the fields
        // `vpin` does not read back — a primitive that animates itself off its
        // own timer would need it, and none of the tables tried does.
        G::Decal(_) | G::Primitive(_) | G::Ball(_) | G::PartGroup(_) | G::Generic(..) => {
            return Timer::default();
        }
    };
    Timer {
        enabled: Cell::new(data.is_enabled),
        interval: Cell::new(clamp_interval(f64::from(data.interval))),
        // Unscheduled, the same as one the script has just switched on: the
        // original builds a `HitTimer` by calling `SetInterval`, so a timer
        // saved switched on is due one interval after the table starts.
        due_ms: Cell::new(f64::NAN),
    }
}

/// A primitive's nine placement numbers, as saved.
///
/// Zero is not a safe default: a table that reads `RotX` before writing it —
/// and `core.vbs` does, to animate relative to where a toy already is — would
/// see a part it has not touched claim to be square with the table.
fn item_placement(item: &vpin::vpx::gameitem::GameItemEnum) -> [f32; 9] {
    match item {
        vpin::vpx::gameitem::GameItemEnum::Primitive(p) => p.rot_and_tra,
        _ => [0.0; 9],
    }
}

fn item_position(item: &vpin::vpx::gameitem::GameItemEnum) -> Vec3 {
    match item {
        vpin::vpx::gameitem::GameItemEnum::Primitive(p) => {
            Vec3::new(p.position.x, p.position.y, p.position.z)
        }
        _ => Vec3::ZERO,
    }
}

fn item_size(item: &vpin::vpx::gameitem::GameItemEnum) -> Vec3 {
    match item {
        vpin::vpx::gameitem::GameItemEnum::Primitive(p) => Vec3::new(p.size.x, p.size.y, p.size.z),
        _ => Vec3::ONE,
    }
}

fn item_visible(item: &vpin::vpx::gameitem::GameItemEnum) -> bool {
    use vpin::vpx::gameitem::GameItemEnum as G;
    match item {
        G::Flipper(f) => f.is_visible,
        G::Trigger(t) => t.is_visible,
        G::Gate(g) => g.is_visible,
        G::Spinner(s) => s.is_visible,
        G::Primitive(p) => p.is_visible,
        G::Flasher(f) => f.is_visible,
        G::HitTarget(h) => h.is_visible,
        G::Plunger(p) => p.is_visible,
        G::Rubber(r) => r.is_visible,
        G::Ramp(r) => r.is_visible,
        // A wall has two visibility flags and `Visible` is the top one:
        // `Surface::get_Visible` returns `m_topBottomVisible`
        // (`surface.cpp:1590`), and the side has its own `SideVisible`.
        G::Wall(w) => w.is_top_bottom_visible,
        // Everything else has no single flag to read, and answering "shown"
        // for it is the honest default: a part nobody hid is on the table.
        _ => true,
    }
}
