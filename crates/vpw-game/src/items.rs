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
    /// How far a drop target has slid out of the playfield, in VP units. Zero
    /// is standing, `-DROP_DEPTH` is all the way down.
    ///
    /// It is a number and not a flag because the slide **takes time**:
    /// `HitTarget::UpdateAnimation` (`hittarget.cpp:576`) moves it by
    /// `dropSpeed` units per millisecond and only declares the target dropped
    /// when it reaches the bottom. Snapping it in one frame is what a bank of
    /// targets looks like when it is wrong: they blink out of the playfield
    /// instead of being knocked down.
    drop_offset: Cell<f32>,
    /// Whether the slide is running.
    dropping: Cell<bool>,
    /// Which way it is going. Down on a hit, up when something raises it.
    dropping_down: Cell<bool>,
    /// Milliseconds still to wait before a raise starts moving.
    ///
    /// The original holds a timestamp and compares it against the player's
    /// clock (`hittarget.cpp:601`); counting down is the same thing without
    /// having to hand this module a clock it otherwise has no use for.
    raise_wait_ms: Cell<f32>,
    /// A flasher's own members. Every item carries the block because the
    /// items are one type, and it is a dozen cells.
    flasher: FlasherProps,
    /// Everything else the script sets on this part. See the module note: a
    /// member we do not model is kept rather than refused, so it at least reads
    /// back as what was written.
    extra: RefCell<HashMap<Box<str>, Value>>,
}

/// What a script writes on a flasher, as the original stores it
/// (`FlasherData`, `flasher.h:15-62`), less the rotations, which sit in
/// [`Visual::rot_and_tra`] with everyone else's `RotX/Y/Z`.
///
/// The renderer reads it back through [`Item::flasher_state`] every frame; the
/// script writes it whenever it likes. `core.vbs` alone flips `Visible` on
/// every flasher wired to a solenoid (`vpmToggleObj`, `core.vbs:2534`), and a
/// table's own script fades with `IntensityScale`, dims with `Opacity` and
/// recolours with `Color` — a strobe that is not told about any of that stays
/// however the file left it, which is to say off.
#[derive(Debug)]
struct FlasherProps {
    /// `X` and `Y`, the centre of the outline: `get_X` answers the box's
    /// centre and `put_X` shifts every point by the difference
    /// (`flasher.cpp:553-586`).
    x: Cell<f64>,
    y: Cell<f64>,
    height: Cell<f64>,
    /// `Color`, as the script sees it: an `OLE_COLOR`, `0x00BBGGRR`.
    color: Cell<u32>,
    /// `Opacity`, out of a hundred and never below zero (`SetAlpha`,
    /// `flasher.h:154`).
    opacity: Cell<f64>,
    modulate_vs_add: Cell<f64>,
    add_blend: Cell<bool>,
    filter: Cell<vpw_table::flasher::Filter>,
    /// `Amount`, out of a hundred and never below zero (`SetFilterAmount`,
    /// `flasher.h:162`).
    amount: Cell<f64>,
    image_a: RefCell<Rc<str>>,
    image_b: RefCell<Rc<str>>,
}

impl Default for FlasherProps {
    fn default() -> Self {
        Self {
            x: Cell::new(0.0),
            y: Cell::new(0.0),
            height: Cell::new(0.0),
            color: Cell::new(0),
            opacity: Cell::new(100.0),
            modulate_vs_add: Cell::new(0.9),
            add_blend: Cell::new(false),
            filter: Cell::new(vpw_table::flasher::Filter::default()),
            amount: Cell::new(100.0),
            image_a: RefCell::new(Rc::from("")),
            image_b: RefCell::new(Rc::from("")),
        }
    }
}

impl FlasherProps {
    /// Where the file leaves it.
    fn seed(&self, f: &vpin::vpx::gameitem::flasher::Flasher) {
        if let Some(c) = vpw_table::flasher::center(f) {
            self.x.set(f64::from(c.x));
            self.y.set(f64::from(c.y));
        }
        self.height.set(f64::from(f.height));
        self.color
            .set(u32::from(f.color.r) | (u32::from(f.color.g) << 8) | (u32::from(f.color.b) << 16));
        self.opacity.set(f64::from(f.alpha.max(0)));
        self.modulate_vs_add.set(f64::from(f.modulate_vs_add));
        self.add_blend.set(f.add_blend);
        // Through the name rather than the number, so the two crates cannot
        // disagree about which number is which filter.
        let filter = match f.filter {
            vpin::vpx::gameitem::flasher::Filter::None => "none",
            vpin::vpx::gameitem::flasher::Filter::Additive => "additive",
            vpin::vpx::gameitem::flasher::Filter::Overlay => "overlay",
            vpin::vpx::gameitem::flasher::Filter::Multiply => "multiply",
            vpin::vpx::gameitem::flasher::Filter::Screen => "screen",
        };
        if let Some(filter) = vpw_table::flasher::Filter::from_name(filter) {
            self.filter.set(filter);
        }
        self.amount.set(f64::from(f.filter_amount));
        *self.image_a.borrow_mut() = Rc::from(f.image_a.as_str());
        *self.image_b.borrow_mut() = Rc::from(f.image_b.as_str());
    }
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            flasher: FlasherProps::default(),
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
            drop_offset: Cell::new(0.0),
            dropping: Cell::new(false),
            dropping_down: Cell::new(false),
            raise_wait_ms: Cell::new(0.0),
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
    position: Cell<Vec3>,
    size: Vec3,
    /// How hard the ball has to arrive for a hit to count, along the collision
    /// normal (`THRS`).
    ///
    /// Every part that reports hits has one and the original compares against
    /// it before it calls the script: `dot <= -m_threshold` on a wall's segment
    /// (`collide.cpp:172`), `dot >= m_threshold` on a ramp's or a target's
    /// triangle (`collideex.cpp:701`, `:913`, `:1126`). Throwing the number
    /// away means the lightest graze fires the hit sound, closes the switch and
    /// scores: a ball trickling along a wall on its way to the drain plays the
    /// wall's sound a dozen times.
    threshold: f32,
    /// Whether the part is marked to raise events at all (`HTEV` / `HAHE`).
    ///
    /// A `.vpx` decides this, not the script: `m_fe = m_d.m_hitEvent`
    /// (`ramp.cpp:818`, `surface.cpp:380-385`), and a part without it is silent
    /// however many handlers are written for it. Deciding instead from "does
    /// the script have an `X_Hit`" inverts where the authority lies. A table's
    /// author unticks the flag on decorative geometry *precisely* so that it
    /// stays quiet, and a handler written against a whole collection then makes
    /// a live switch out of every post in it.
    hit_event: bool,
    /// A drop target's slide speed in units per millisecond (`DRSP`) and the
    /// pause before it comes back up in milliseconds (`RADE`).
    drop_speed: f32,
    raise_delay: f32,
    /// A kicker's kick scatter, in radians, already weighted by the table's
    /// global difficulty (`kicker.cpp:725-726`).
    kick_scatter: f32,
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
    /// Where this piece sits on the playfield, as the file places it.
    ///
    /// For naming a fault a player describes by position: VPX's y grows toward
    /// the player, so the bottom left of a table is small x and large y.
    pub fn position(&self) -> Vec3 {
        self.position.get()
    }

    pub fn placement(&self) -> [f32; 9] {
        self.visual.rot_and_tra.get()
    }

    /// Where this part sits, built from its placement numbers.
    ///
    /// The same chain the geometry uses for a baked primitive, so a piece that
    /// nothing has moved lands exactly where it was baked.
    pub fn placement_matrix(&self) -> vpw_math::Mat4 {
        vpw_table::geometry::primitive_transform_from_fields(
            self.position.get(),
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

    /// The first shape of a given kind this item owns.
    ///
    /// [`Item::shape`] takes the first one full stop, which is right for every
    /// part that makes exactly one. A one-way gate makes several, and the leaf
    /// is not the first: `physics::gate` pushes the blocking segment, then the
    /// leaf, then a bracket post at each end. So asking for the first gave a
    /// `Shape::Line`, every `Gate.Open` and `Gate.CurrentAngle` a table wrote
    /// fell through to "no such member", and the write was quietly filed away
    /// as an unmodelled extra where nothing would ever read it.
    fn shape_like(&self, wanted: impl Fn(&Shape) -> bool) -> Option<usize> {
        let engine = self.engine.borrow();
        self.shapes
            .iter()
            .copied()
            .find(|&i| engine.shapes().get(i).is_some_and(&wanted))
    }

    /// The leaf: the pass-through line that swings.
    fn gate_leaf(&self) -> Option<usize> {
        self.shape_like(|s| matches!(s, Shape::Gate(_)))
    }

    /// The rigid segment that stops the ball coming back the wrong way through
    /// a **one-way** gate. A two-way gate has none, so this is `None` there.
    fn gate_blocker(&self) -> Option<usize> {
        self.shape_like(|s| matches!(s, Shape::Line(_)))
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
            Kind::Gate => self.gate_get(&lower, args),
            Kind::Spinner => self.spinner_get(&lower),
            Kind::Plunger => self.plunger_get(&lower),
            Kind::Bumper => self.bumper_get(&lower),
            Kind::Flasher => self.flasher_get(&lower),
            _ => Err(Error::no_such_member(name)),
        };
        match specific {
            Ok(v) => Ok(v),
            Err(e) => {
                // Where the part is, for every kind that does not keep its
                // own. A flasher tracks a position the script moves and
                // answers from that; a primitive, a bumper or a light answers
                // from where the file put it.
                //
                // Circus measures its slingshot between two primitives and
                // divides by the distance (`PSlope`, line 2194), so two parts
                // that both answer zero are a division by zero on every
                // slingshot hit.
                if let Some(v) = self.position_get(&lower) {
                    return Ok(v);
                }
                // Accepted rather than refused, for the reason in the module
                // note: a real Visual Pinball part has every member, so
                // answering "no such member" is the less faithful of the two
                // mistakes. But it is *said*, because the two cases hiding
                // behind this line are worlds apart — a table asking for a
                // presentational member this renderer does not do yet, and a
                // member that is implemented and failed. The second one was
                // invisible until a kicker quietly refused to make a ball.
                // 438 is "no such member", which is the case this is for.
                if e.number != 438 {
                    log::warn!("{}.{name}: {e}", self.name);
                }
                Ok(self.remembered(&lower))
            }
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
            Kind::Flasher => self.flasher_set(&lower, &value),
            Kind::Plunger => self.plunger_set(&lower, &value),
            _ => Err(Error::no_such_member(name)),
        };
        if specific.is_err() && !self.position_set(&lower, &value)? {
            self.remember(&lower, value);
        }
        Ok(())
    }
}

impl Item {
    /// `X`, `Y` and `Z`: where the part sits, as the file places it.
    ///
    /// `None` for any other name. Only reached for a kind that does not answer
    /// these itself — see [`Object::get`] on this type.
    fn position_get(&self, name: &str) -> Option<Value> {
        let at = self.position.get();
        match name {
            "x" => Some(Value::Double(at.x.into())),
            "y" => Some(Value::Double(at.y.into())),
            "z" => Some(Value::Double(at.z.into())),
            _ => None,
        }
    }

    /// Moves the part, as far as it can be moved.
    ///
    /// For a primitive that is the whole of it: [`Self::placement_matrix`]
    /// builds from this, so the piece is drawn where the script put it. For a
    /// part the physics owns, the collision shape stays where it was authored
    /// — the same half-measure `RotX` on a bumper already is, and better than
    /// a script writing a position that then reads back as the old one.
    ///
    /// Answers whether the name was one of the three.
    fn position_set(&self, name: &str, value: &Value) -> Result<bool> {
        // The name first. Reaching for the number before knowing whether this
        // is even a position turns every unmodelled member a script writes a
        // *string* to into a type mismatch, which is a table that no longer
        // loads.
        if !matches!(name, "x" | "y" | "z") {
            return Ok(false);
        }
        let mut at = self.position.get();
        let n = value.to_number()? as f32;
        match name {
            "x" => at.x = n,
            "y" => at.y = n,
            _ => at.z = n,
        }
        self.position.set(at);
        Ok(true)
    }

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
        let mut extra = self.visual.extra.borrow_mut();
        // Only the first write is news. South Park's script sets `colorfull`
        // on a lamp every frame, and a console with sixty lines a second of
        // the same fact in it has no room left for a different one.
        if !extra.contains_key(name) {
            log::trace!("{}: '{name}' is not modelled; keeping it as set", self.name);
        }
        extra.insert(name.into(), value);
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
        // `put_IsDropped` (`hittarget.cpp:1180`) flips the flag **now** and
        // then starts the slide: a target a script drops stops colliding on the
        // instant, and one it raises collides again on the instant, whichever
        // way the mesh happens to be travelling. Only a target the *ball*
        // knocks over waits for the bottom — see [`Item::knock_down`].
        self.visual.dropped.set(down);
        self.visual.announce.set(Some(down));
        self.set_collidable(!down && self.visual.collidable.get());

        self.visual
            .drop_offset
            .set(if down { 0.0 } else { -DROP_DEPTH });
        self.visual.dropping.set(true);
        self.visual.dropping_down.set(down);
        // The delay is on the way **up** only, and it is what makes a bank
        // reset look like a bank reset: the coil fires, the targets sit there
        // for a tenth of a second and then rise together.
        self.visual
            .raise_wait_ms
            .set(if down { 0.0 } else { self.raise_delay });
        self.show_drop_offset();
    }

    /// Advances the slide by `dt_ms` of table time
    /// (`HitTarget::UpdateAnimation`, `hittarget.cpp:576-632`).
    ///
    /// A stand-up target has nothing to animate: the original runs the whole of
    /// this only for the three drop types, and gives the others a shallower
    /// wobble that nothing in the physics or the rules can see.
    pub fn animate_drop(&self, dt_ms: f32) {
        if !self.visual.drops.get() || !self.visual.dropping.get() {
            return;
        }
        // An invisible target does not move, and the original spells out why:
        // "this is needed for backward compatibility since animation used to be
        // part of rendering and would not be performed, therefore hidden drop
        // targets would never actually drop, and old tables rely on this
        // behavior" (`hittarget.cpp:580`). A table that hides a target to take
        // it out of play is relying on it staying up.
        if !self.visual.visible.get() {
            return;
        }
        let going_down = self.visual.dropping_down.get();

        let mut step = self.drop_speed;
        if going_down {
            step = -step;
        } else if self.visual.raise_wait_ms.get() > 0.0 {
            self.visual
                .raise_wait_ms
                .set(self.visual.raise_wait_ms.get() - dt_ms);
            step = 0.0;
        }

        let mut offset = self.visual.drop_offset.get() + step * dt_ms;
        if going_down {
            if offset <= -DROP_DEPTH {
                offset = -DROP_DEPTH;
                self.visual.dropping_down.set(false);
                self.visual.dropping.set(false);
                // The ball's route: the flag has not been set yet, so this is
                // where the target stops colliding and where `_Dropped` is
                // raised. On the script's route it is already set and this does
                // nothing, which is the original's shape too.
                if !self.visual.dropped.get() {
                    self.visual.dropped.set(true);
                    self.visual.announce.set(Some(true));
                    self.set_collidable(false);
                }
            }
        } else if offset >= 0.0 {
            offset = 0.0;
            self.visual.dropping.set(false);
            if self.visual.dropped.get() {
                self.visual.dropped.set(false);
                self.visual.announce.set(Some(false));
                self.set_collidable(self.visual.collidable.get());
            }
        }
        self.visual.drop_offset.set(offset);
        self.show_drop_offset();
    }

    /// Puts the slide's current position where the renderer reads it from.
    ///
    /// The mesh does not move in the physics at all — the original only lowers
    /// the vertices it draws with (`UpdateTarget`, `hittarget.cpp:697`) — so
    /// this is the whole of the visible half.
    fn show_drop_offset(&self) {
        set_placement(
            &self.visual.rot_and_tra,
            5,
            f64::from(self.visual.drop_offset.get()),
        );
    }

    /// Whether this target has a slide still running, for tests.
    pub fn is_dropping(&self) -> bool {
        self.visual.dropping.get()
    }

    /// How far out of the playfield this target has slid, in VP units.
    pub fn drop_offset(&self) -> f32 {
        self.visual.drop_offset.get()
    }

    /// The impact speed below which a hit on this part does not count.
    pub fn hit_threshold(&self) -> f32 {
        self.threshold
    }

    /// Whether the file marks this part as raising hit events at all.
    pub fn has_hit_event(&self) -> bool {
        self.hit_event
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
    /// Returns whether the hit started it moving, which a stand-up target and
    /// one that is already down or already on its way down do not.
    ///
    /// The hit does **not** declare the target dropped. The original sets a
    /// flag on the target and the animation carries it down over some tens of
    /// milliseconds before flipping `m_d.m_isDropped` and firing `Dropped`
    /// (`hittarget.cpp:584` and `:606-618`) — so the target still collides
    /// while it is sinking, and a ball that is leaning on it keeps being pushed
    /// off rather than falling through the moment it is touched. The `Dropped`
    /// event comes out of [`Item::animate_drop`] through the same announcement
    /// queue a script-driven drop uses.
    pub fn knock_down(&self) -> bool {
        if !self.visual.drops.get() || self.visual.dropped.get() {
            return false;
        }
        if self.visual.dropping.get() && self.visual.dropping_down.get() {
            return false; // already on its way
        }
        self.visual.dropping.set(true);
        self.visual.dropping_down.set(true);
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
                // The six a table retunes, which is most of what a modern
                // table's flipper block does. Reading one that is not here
                // does not fail — it comes back `Empty`, which is zero — and
                // Circus divides by `LeftFlipper.Return` on every key release
                // (`Flipper.eostorque = EOST * EOSReturn / FReturn`).
                "return" => Some(Value::Double(f.return_ratio.into())),
                "rampup" => Some(Value::Double(f.ramp_up.into())),
                "eostorque" => Some(Value::Double(f.torque_damping.into())),
                // Degrees to the script, radians inside. The `.vpx` stores
                // degrees and the original converts on the way in; the same
                // conversion has to happen on the way back out or a table that
                // reads the angle, does arithmetic on it and writes it back
                // multiplies it by fifty-seven.
                "eostorqueangle" => Some(Value::Double(f.torque_damping_angle.to_degrees().into())),
                "scatter" => Some(Value::Double(f.material.scatter.to_degrees().into())),
                // Mass is not stored; the moment of inertia is, and it is the
                // mass of a bar turning about one end
                // (`FlipperMover::GetMass`, `hitflipper.cpp`).
                "mass" => Some(Value::Double(
                    (3.0 * f.inertia / (f.flipper_radius * f.flipper_radius)).into(),
                )),
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
            "return" => f.return_ratio = value.to_number()? as f32,
            "rampup" => f.ramp_up = value.to_number()? as f32,
            "eostorque" => f.torque_damping = value.to_number()? as f32,
            "eostorqueangle" => {
                f.torque_damping_angle = (value.to_number()? as f32).to_radians();
            }
            "scatter" => f.material.scatter = (value.to_number()? as f32).to_radians(),
            "mass" => {
                let m = value.to_number()? as f32;
                f.inertia = m * f.flipper_radius * f.flipper_radius / 3.0;
            }
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

    /// The flasher's own members (`flasher.cpp:553-830`). `Visible`,
    /// `IntensityScale` and the three rotations are common members and never
    /// get here.
    fn flasher_get(&self, name: &str) -> Result<Value> {
        let f = &self.visual.flasher;
        Ok(match name {
            "x" => Value::Double(f.x.get()),
            "y" => Value::Double(f.y.get()),
            "height" => Value::Double(f.height.get()),
            // `OLE_COLOR` is a long to a script.
            "color" => Value::Long(f.color.get() as i32),
            "imagea" => Value::Str(f.image_a.borrow().clone()),
            "imageb" => Value::Str(f.image_b.borrow().clone()),
            "opacity" => Value::Long(f.opacity.get() as i32),
            "modulatevsadd" => Value::Double(f.modulate_vs_add.get()),
            "amount" => Value::Long(f.amount.get() as i32),
            "addblend" => Value::Bool(f.add_blend.get()),
            "filter" => Value::Str(Rc::from(f.filter.get().name())),
            _ => return Err(Error::no_such_member(name)),
        })
    }

    fn flasher_set(&self, name: &str, value: &Value) -> Result<()> {
        let f = &self.visual.flasher;
        match name {
            "x" => f.x.set(value.to_number()?),
            "y" => f.y.set(value.to_number()?),
            "height" => f.height.set(value.to_number()?),
            "color" => f.color.set(value.to_int()? as u32),
            "imagea" => *f.image_a.borrow_mut() = value.to_str()?,
            "imageb" => *f.image_b.borrow_mut() = value.to_str()?,
            // A `LONG` in the interface, so `Opacity = 12.7` has already lost
            // its fraction; and clamped at zero (`SetAlpha`, `flasher.h:154`).
            "opacity" => f.opacity.set(f64::from(value.to_int()?.max(0))),
            "modulatevsadd" => f.modulate_vs_add.set(value.to_number()?),
            "amount" => f.amount.set(f64::from(value.to_int()?.max(0))),
            "addblend" => f.add_blend.set(value.to_bool()?),
            // A name it does not know changes nothing, which is exactly
            // `put_Filter` (`flasher.cpp:710-726`).
            "filter" => {
                if let Some(filter) = vpw_table::flasher::Filter::from_name(&value.to_str()?) {
                    f.filter.set(filter);
                }
            }
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }

    /// What the renderer needs of a flasher this frame: everything the script
    /// can have written, in the form `vpw_table::flasher` defines.
    ///
    /// Built fresh each time rather than kept, because the renderer compares
    /// it against what it last drew and a table has a few dozen flashers: the
    /// clone is cheaper than the bookkeeping that would avoid it.
    pub fn flasher_state(&self) -> vpw_table::flasher::State {
        let v = &self.visual;
        let f = &v.flasher;
        let c = f.color.get();
        let rot = v.rot_and_tra.get();
        vpw_table::flasher::State {
            visible: v.visible.get(),
            x: f.x.get() as f32,
            y: f.y.get() as f32,
            height: f.height.get() as f32,
            rot: [rot[0], rot[1], rot[2]],
            // `convertColor`: out of 255 and not decoded (`utils/color.h:22`).
            color: [c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff].map(|ch| ch as f32 / 255.0),
            alpha: f.opacity.get() as f32,
            intensity_scale: v.intensity_scale.get() as f32,
            modulate_vs_add: f.modulate_vs_add.get() as f32,
            add_blend: f.add_blend.get(),
            filter: f.filter.get(),
            filter_amount: f.amount.get() as f32,
            image_a: f.image_a.borrow().to_string(),
            image_b: f.image_b.borrow().to_string(),
        }
    }

    fn gate_get(&self, name: &str, args: &[Value]) -> Result<Value> {
        // `Move dir, speed, angle` is a method, and the only one a gate has.
        if name == "move" {
            return self.gate_move(args).map(|()| Value::Empty);
        }
        let Some(i) = self.gate_leaf() else {
            return Err(Error::no_such_member(name));
        };
        let engine = self.engine.borrow();
        let Some(Shape::Gate(g)) = engine.shapes().get(i) else {
            return Err(Error::no_such_member(name));
        };
        match name {
            "currentangle" => Ok(Value::Double(g.angle.to_degrees().into())),
            "open" => Ok(Value::Bool(g.open)),
            // The player's live limits, not the file's (`gate.cpp:145`,
            // `:169`): a `Move` or an earlier write may have narrowed them, and
            // a script that reads one back expects what it set.
            "openangle" => Ok(Value::Double(g.angle_max.to_degrees().into())),
            "closeangle" => Ok(Value::Double(g.angle_min.to_degrees().into())),
            _ => Err(Error::no_such_member(name)),
        }
    }

    fn gate_set(&self, name: &str, value: &Value) -> Result<()> {
        let Some(i) = self.gate_leaf() else {
            return Err(Error::no_such_member(name));
        };
        // Read before the engine is borrowed: both borrow it.
        let blocker = self.gate_blocker();
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Gate(g)) = engine.shape_mut(i) else {
            return Err(Error::no_such_member(name));
        };
        match name {
            // A script holding a gate open is how a table makes a one-way
            // passage temporarily two-way — or, far more often, how it lets a
            // ball out of a lane it has just finished counting.
            //
            // Setting the flag was not enough. `open` is read in one place,
            // `Gate::update_velocities`, where all it does is stop gravity
            // pulling the leaf shut; `Gate::put_Open` (`gate.cpp:736`) also
            // switches the leaf off, kicks it so it visibly swings, **and**
            // switches off the blocking segment beside it. Leave that segment
            // on and the script opens a gate onto an invisible wall.
            "open" => {
                let open = value.to_bool()?;
                g.set_open(open);
                // Closing puts it back to what the *file* said, not to "on":
                // `m_plineseg->m_enabled = m_d.m_collidable` (`gate.cpp:754`).
                // A table that ships a gate non-collidable and opens it for a
                // moment must not come back with a wall it never had.
                let back_on = g.collidable;
                if let Some(line) = blocker {
                    engine.set_shape_enabled(line, !open && back_on);
                }
            }
            // Both are in degrees on the script side and radians in the engine.
            "openangle" => {
                let a = (value.to_number()? as f32).to_radians();
                g.set_open_angle(a);
            }
            "closeangle" => {
                let a = (value.to_number()? as f32).to_radians();
                g.set_close_angle(a);
            }
            _ => return Err(Error::no_such_member(name)),
        }
        Ok(())
    }

    /// `Gate.Move dir, speed, angle` (`Gate::Move`, `gate.cpp:865`).
    ///
    /// A graphics-only animation: it takes the gate out of the physics for as
    /// long as it runs, which means the blocking segment goes with it exactly
    /// as it does for `Open`.
    fn gate_move(&self, args: &[Value]) -> Result<()> {
        let Some(i) = self.gate_leaf() else {
            return Err(Error::no_such_member("Move"));
        };
        let number = |n: usize| -> Result<f64> {
            args.get(n)
                .map(Value::to_number)
                .transpose()
                .map(|v| v.unwrap_or(0.0))
        };
        let dir = number(0)? as i32;
        // A speed at or below zero means "the default", so an omitted argument
        // is already the right answer.
        let speed = number(1)? as f32;
        let angle = number(2)? as f32;

        let blocker = self.gate_blocker();
        let mut engine = self.engine.borrow_mut();
        let Some(Shape::Gate(g)) = engine.shape_mut(i) else {
            return Err(Error::no_such_member("Move"));
        };
        g.move_toward(dir, speed, angle);
        if let Some(line) = blocker {
            engine.set_shape_enabled(line, false);
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
            // `Kick angle, speed [, inclination]`: the whole point of a
            // kicker. `KickZ` and `KickXYZ` are the same call with the ball
            // nudged first (`kicker.cpp:704`, `:762`, `:768`); tables reach for
            // them when a saucer is buried under a ramp and the ball has to
            // come out at the right height to clear it.
            "kick" | "kickz" | "kickxyz" => {
                let number = |n: usize| -> Result<f64> {
                    args.get(n)
                        .map(Value::to_number)
                        .transpose()
                        .map(|v| v.unwrap_or(0.0))
                };
                let offset = match name {
                    "kickz" => Vec3::new(0.0, 0.0, number(3)? as f32),
                    "kickxyz" => Vec3::new(number(3)? as f32, number(4)? as f32, number(5)? as f32),
                    _ => Vec3::ZERO,
                };
                self.kick(number(0)?, number(1)?, number(2)?, offset)?;
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

    /// `KickXYZ angle, speed, inclination, x, y, z` (`kicker.cpp:704`), which
    /// is what all three of the kicker's kicks come down to.
    ///
    /// The angle is in degrees, measured from straight up the table and turning
    /// clockwise, which is Visual Pinball's convention and not the one a maths
    /// library would pick.
    ///
    /// The **inclination** is what a flat vector loses. A saucer that ejects at
    /// forty-five degrees is aiming the ball over something — a ramp entrance,
    /// a wall, the row of targets in front of it — and the ball has to leave
    /// with the vertical speed to clear it. `speedz = sin(inclination) * speed`
    /// and the planar part scaled by `cos(inclination)`, so the total is what
    /// the table asked for rather than that plus a climb.
    fn kick(&self, angle_deg: f64, speed: f64, inclination: f64, offset: Vec3) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member("Kick"));
        };
        let mut angle = (angle_deg as f32).to_radians();
        let mut speed = speed as f32;

        // "radians or degrees? if greater PI/2 assume degrees"
        // (`kicker.cpp:721`). Tables are written both ways and the original
        // guesses rather than picking one, so this has to guess with it.
        let mut inclination = inclination as f32;
        if inclination.abs() > std::f32::consts::FRAC_PI_2 {
            inclination = inclination.to_radians();
        }

        let mut engine = self.engine.borrow_mut();
        // The scatter (`kicker.cpp:720-728`). `rand_mt_m11()` gives -1..1 and
        // the shaping `r * (1 - r²) * 2.59808` bends a flat distribution into
        // one that clusters near the aimed angle and reaches the full scatter
        // only rarely — a kick that misses by the maximum every other time
        // would not be scatter, it would be a wobble.
        if self.kick_scatter > 1.0e-5 {
            let r = engine.random_m11();
            angle += r * (1.0 - r * r) * 2.59808 * self.kick_scatter;
        }

        let speed_z = inclination.sin() * speed;
        // Only when it is aimed **upwards**: the original leaves the planar
        // speed alone for a downward kick, so a table that fires a ball down a
        // hole does not lose the length of the shot as well.
        if speed_z > 0.0 {
            speed *= inclination.cos();
        }

        let v = Vec3::new(angle.sin() * speed, -angle.cos() * speed, speed_z);
        engine.kick_from(i, v, offset);
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

    fn plunger_set(&self, name: &str, value: &Value) -> Result<()> {
        let Some(i) = self.shape() else {
            return Err(Error::no_such_member(name));
        };
        match name {
            // The ROM-controlled launcher. The file can author a plunger this
            // way, but plenty of tables flip it at run time instead — South
            // Park's AutoLaunch turns it on, fires, and turns it off — and
            // the original honours the property live (`Plunger::Fire`,
            // `plunger.cpp:988`: an auto plunger releases from full
            // retraction, a solenoid having no notion of how hard).
            "autoplunger" => {
                let on = value.to_bool()?;
                let mut engine = self.engine.borrow_mut();
                if let Some(Shape::Plunger(p)) = engine.shapes_mut().get_mut(i) {
                    p.auto_plunger = on;
                }
                Ok(())
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
            if let vpin::vpx::gameitem::GameItemEnum::Flasher(f) = item {
                visual.flasher.seed(f);
            }
            if let vpin::vpx::gameitem::GameItemEnum::HitTarget(t) = item {
                use vpin::vpx::gameitem::hittarget::TargetType as T;
                visual.drops.set(matches!(
                    t.target_type,
                    T::DropTargetBeveled | T::DropTargetSimple | T::DropTargetFlatSimple
                ));
                visual.collidable.set(t.is_collidable);
                // A table can be saved with a bank already knocked down.
                // `RenderSetup` (`hittarget.cpp:557`) puts such a target at the
                // bottom of its travel before the first frame is drawn, and
                // `DoHitTest` skips a dropped target's colliders, so it neither
                // shows nor blocks. Setting only the flag, which is what this
                // did, left it standing on screen and solid to the ball.
                visual.dropped.set(t.is_dropped);
                if t.is_dropped {
                    visual.drop_offset.set(-DROP_DEPTH);
                    set_placement(&visual.rot_and_tra, 5, -f64::from(DROP_DEPTH));
                }
            }

            let (hit_event, threshold) = hit_rules(item);
            let entry = Rc::new(Item {
                name: name.clone(),
                kind,
                index,
                shapes: collision.shapes_of(index).collect(),
                trigger,
                timer: item_timer(item),
                position: Cell::new(item_position(item)),
                size: item_size(item),
                hit_event,
                threshold,
                drop_speed: drop_speed(item),
                raise_delay: raise_delay(item),
                kick_scatter: kick_scatter(vpx, item),
                visual,
                engine: engine.clone(),
                timers_changed: dirty.clone(),
            });
            // A target the file saved knocked down does not collide.
            if entry.visual.dropped.get() {
                entry.set_collidable(false);
            }
            // A part the file saved switched off. The shape exists either way —
            // see `physics::kicker` for why — so the state is applied here,
            // through the same switch a script would use.
            if let vpin::vpx::gameitem::GameItemEnum::Kicker(k) = item
                && !k.is_enabled
            {
                entry.visual.collidable.set(false);
                entry.set_collidable(false);
            }
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

/// Whether a part raises hit events, and how hard it has to be hit.
///
/// The two are read together because the original reads them together: the
/// `hitEvent` flag is what puts the part's address on the collider (`m_fe`) and
/// the threshold is what the impact speed is measured against once it is there.
/// A part with no flag never reports, whatever the speed.
///
/// The parts that are not listed are the ones the original gives neither of:
/// a gate, a spinner, a kicker and a trigger always report, so they get `true`
/// and a threshold of zero. The defaults for the two `Option` fields are the
/// editor's, which is also what a file too old to carry the tag was saved with
/// (`Settings_properties.inl:1205`, `:1206`).
fn hit_rules(item: &vpin::vpx::gameitem::GameItemEnum) -> (bool, f32) {
    use vpin::vpx::gameitem::GameItemEnum as G;
    match item {
        G::Wall(w) => (w.hit_event, w.threshold),
        G::Ramp(r) => (r.hit_event.unwrap_or(false), r.threshold.unwrap_or(2.0)),
        // A rubber has no threshold field at all: `Rubber::SetupHitObject`
        // hard-codes it, with the original's own comment beside it saying so —
        // "hard coded threshold for now" (`rubber.cpp:609`).
        G::Rubber(r) => (r.hit_event, 2.0),
        G::Primitive(p) => (p.hit_event, p.threshold),
        G::HitTarget(t) => (t.use_hit_event, t.threshold),
        // A bumper's pair is read by the physics itself, because there the flag
        // gates the kick and not only the report (`collideex.cpp:33`). It is
        // repeated here so the event layer agrees with the coil.
        G::Bumper(b) => (b.hit_event.unwrap_or(true), b.threshold),
        _ => (true, 0.0),
    }
}

/// A drop target's slide speed, in units per millisecond (`DRSP`).
fn drop_speed(item: &vpin::vpx::gameitem::GameItemEnum) -> f32 {
    match item {
        // A deliberate departure: a file that says zero is taken as the
        // editor's default rather than at its word. The original would believe
        // it, and a target whose slide advances by zero per millisecond never
        // reaches the bottom — so it never sets `IsDropped`, never fires
        // `Dropped` and never stops colliding. On a ROM table that is a bank
        // that can never be completed, from one bad number, silently.
        vpin::vpx::gameitem::GameItemEnum::HitTarget(t) if t.drop_speed > 0.0 => t.drop_speed,
        vpin::vpx::gameitem::GameItemEnum::HitTarget(_) => 0.2,
        _ => 0.0,
    }
}

/// How long a raised target waits before it starts coming up (`RADE`), in
/// milliseconds. The editor's default is a tenth of a second.
fn raise_delay(item: &vpin::vpx::gameitem::GameItemEnum) -> f32 {
    match item {
        vpin::vpx::gameitem::GameItemEnum::HitTarget(t) => t.raise_delay.unwrap_or(100) as f32,
        _ => 0.0,
    }
}

/// A kicker's kick scatter in radians, weighted by the table's difficulty.
///
/// `kicker.cpp:725-726`:
///
/// ```cpp
/// float scatterAngle = (m_d.m_scatter < 0.0f) ? c_hardScatter : ANGTORAD(m_d.m_scatter);
/// scatterAngle *= m_ptable->m_globalDifficulty;
/// ```
///
/// A negative `KSCT` means "use the table's own default scatter", which is
/// `c_hardScatter = ANGTORAD(m_defaultScatter)` (`player.cpp:197`). The
/// difficulty weighting is the reason a table set to its easiest ejects the
/// same way every time and one set hard does not: at difficulty zero the whole
/// term vanishes, which is the default and therefore the common case.
fn kick_scatter(vpx: &VPX, item: &vpin::vpx::gameitem::GameItemEnum) -> f32 {
    let vpin::vpx::gameitem::GameItemEnum::Kicker(k) = item else {
        return 0.0;
    };
    let base = if k.scatter < 0.0 {
        vpx.gamedata.default_scatter
    } else {
        k.scatter
    };
    // The original keeps the difficulty as a fraction and only multiplies by a
    // hundred for the script (`pintable.cpp:6373`); a file that stored a
    // percentage would otherwise scale the scatter a hundredfold.
    base.to_radians() * vpx.gamedata.global_difficulty.clamp(0.0, 1.0)
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
///
/// A flasher has the first three: its `RotX/Y/Z` are the same members and
/// live in the same slots, so the common accessors serve both.
fn item_placement(item: &vpin::vpx::gameitem::GameItemEnum) -> [f32; 9] {
    match item {
        vpin::vpx::gameitem::GameItemEnum::Primitive(p) => p.rot_and_tra,
        vpin::vpx::gameitem::GameItemEnum::Flasher(f) => {
            [f.rot_x, f.rot_y, f.rot_z, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        }
        _ => [0.0; 9],
    }
}

/// Where a part sits, for every kind that has a single place to sit.
///
/// Not only primitives. A script reads `X` and `Y` off anything it can name —
/// most often to work out where something is *relative* to something else, and
/// two parts that both answer zero are in the same place. Circus measures its
/// slingshot between two primitives and divides by the distance
/// (`PSlope`, line 2194); with both ends at the origin that is a division by
/// zero on every slingshot hit.
///
/// A wall, a ramp or a rubber is a run of points with no centre, and those
/// stay at the origin because there is no better answer.
fn item_position(item: &vpin::vpx::gameitem::GameItemEnum) -> Vec3 {
    use vpin::vpx::gameitem::GameItemEnum as G;
    let flat = |c: &vpin::vpx::gameitem::vertex2d::Vertex2D| Vec3::new(c.x, c.y, 0.0);
    match item {
        G::Primitive(p) => Vec3::new(p.position.x, p.position.y, p.position.z),
        G::HitTarget(h) => Vec3::new(h.position.x, h.position.y, h.position.z),
        G::Bumper(b) => flat(&b.center),
        G::Flipper(f) => flat(&f.center),
        G::Gate(g) => flat(&g.center),
        G::Kicker(k) => flat(&k.center),
        G::Light(l) => flat(&l.center),
        G::Plunger(p) => flat(&p.center),
        G::Spinner(s) => flat(&s.center),
        G::Trigger(t) => flat(&t.center),
        G::Decal(d) => flat(&d.center),
        G::Timer(t) => flat(&t.center),
        G::LightSequencer(l) => flat(&l.center),
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
