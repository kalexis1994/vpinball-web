//! The simulation loop.
//!
//! Port of `PhysicsEngine::PhysicsSimulateCycle` (`PhysicsEngine.cpp:428`).
//!
//! # How time advances
//!
//! We do not advance a fixed step and then fix up the overlaps. We look for
//! **the first hit that is going to happen**, advance exactly up to it,
//! resolve it, and repeat with the time that is left over. That is why a ball
//! going at a thousand does not go through a thin wall: the hit is found
//! before it gets moved.
//!
//! That carries a risk: if two hits land practically on the same instant, the
//! loop can end up spinning in place advancing zero. The original cuts that
//! off with a counter ([`STATICCNTS`]) that, after ten events too close
//! together, forces a minimum advance.

use crate::Aabb;
use crate::ball::{Ball, collide_balls, hit_test_balls};
use crate::collision::{CollisionEvent, Material};
use crate::constants::{PHYS_FACTOR, STATICCNTS, STATICTIME};
use crate::flipper::Flipper;
use crate::nudge::Nudge;
use crate::parts::{Bumper, Gate, Kicker, Spinner};
use crate::plunger::Plunger;
use crate::quadtree::{Entry, Quadtree};
use crate::shapes::{
    HitCircle, HitLine3D, HitLineZ, HitPlane, HitPoint, HitPoly, HitTriangle, LineSeg, Slingshot,
};
use crate::trigger::{Crossing, Trigger};
use vpw_math::Vec3;

/// The plunger resolves its own hit with a fixed hardness that does not come
/// from any material (`hitplunger.cpp:747`), so this one only exists so the
/// engine has something to return.
static PLUNGER_MATERIAL: Material = Material {
    elasticity: 0.0,
    elasticity_falloff: 0.0,
    friction: 0.0,
    scatter: 0.0,
};

/// A collision shape of the table.
#[derive(Debug, Clone)]
pub enum Shape {
    Line(LineSeg),
    /// A vertical pole, and a leaning one. They seal the seams where two
    /// surfaces meet; see [`crate::shapes::HitLineZ`].
    LineZ(HitLineZ),
    Line3D(HitLine3D),
    Circle(HitCircle),
    Plane(HitPlane),
    Point(HitPoint),
    Poly(HitPoly),
    /// One triangle of a mesh. Not the same test as [`Shape::Poly`]; see
    /// [`HitTriangle`].
    Triangle(HitTriangle),
    Slingshot(Slingshot),
    Bumper(Bumper),
    Kicker(Kicker),
    Gate(Gate),
    Spinner(Spinner),
    Flipper(Flipper),
    Plunger(Plunger),
}

impl Shape {
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        match self {
            Shape::Line(s) => s.hit_test(ball, dtime),
            Shape::LineZ(s) => s.hit_test(ball, dtime),
            Shape::Line3D(s) => s.hit_test(ball, dtime),
            Shape::Circle(s) => s.hit_test(ball, dtime),
            Shape::Plane(s) => s.hit_test(ball, dtime),
            Shape::Point(s) => s.hit_test(ball, dtime),
            Shape::Poly(s) => s.hit_test(ball, dtime),
            Shape::Triangle(s) => s.hit_test(ball, dtime),
            Shape::Slingshot(s) => s.hit_test(ball, dtime),
            Shape::Bumper(s) => s.hit_test(ball, dtime),
            Shape::Kicker(s) => s.hit_test(ball, dtime),
            Shape::Gate(s) => s.hit_test(ball, dtime),
            Shape::Spinner(s) => s.hit_test(ball, dtime),
            Shape::Flipper(s) => s.hit_test(ball, dtime),
            Shape::Plunger(s) => s.hit_test(ball, dtime),
        }
    }

    pub fn material(&self) -> &Material {
        match self {
            Shape::Line(s) => &s.material,
            Shape::LineZ(s) => &s.material,
            Shape::Line3D(s) => s.material(),
            Shape::Circle(s) => &s.material,
            Shape::Plane(s) => &s.material,
            Shape::Point(s) => &s.material,
            Shape::Poly(s) => &s.material,
            Shape::Triangle(s) => &s.material,
            Shape::Slingshot(s) => &s.seg.material,
            Shape::Bumper(s) => &s.circle.material,
            Shape::Kicker(s) => &s.circle.material,
            Shape::Gate(s) => &s.seg.material,
            Shape::Spinner(s) => &s.seg.material,
            Shape::Flipper(s) => &s.material,
            Shape::Plunger(_) => &PLUNGER_MATERIAL,
        }
    }

    /// Bounding box, or `None` if the shape is infinite.
    ///
    /// The planes —the playfield and the glass— do not go into the tree: they
    /// are always tested, as the original does
    /// (`PhysicsEngine.cpp:556-558`).
    pub fn bbox(&self) -> Option<Aabb> {
        match self {
            Shape::Line(s) => Some(s.bbox()),
            Shape::LineZ(s) => Some(s.bbox()),
            Shape::Line3D(s) => Some(s.bbox()),
            Shape::Circle(s) => Some(s.bbox()),
            Shape::Point(s) => Some(s.bbox()),
            Shape::Poly(s) => Some(s.bbox()),
            Shape::Triangle(s) => Some(s.bbox()),
            Shape::Slingshot(s) => Some(s.bbox()),
            Shape::Bumper(s) => Some(s.circle.bbox()),
            Shape::Kicker(s) => Some(s.circle.bbox()),
            Shape::Gate(s) => Some(s.seg.bbox()),
            Shape::Spinner(s) => Some(s.seg.bbox()),
            Shape::Flipper(s) => Some(s.bbox()),
            Shape::Plunger(s) => Some(s.bbox()),
            Shape::Plane(_) => None,
        }
    }
}

/// What a ball hits against.
///
/// The original puts the balls into a dynamic octree that it **rebuilds every
/// frame** (`m_hitoctree_dynamic`) so it can treat them the same as the fixed
/// geometry. Here they go separately and all of them are tested against all of
/// them: a table has one ball, or six in a multiball. Six balls are fifteen
/// pairs; building a tree for fifteen tests costs quite a bit more than just
/// doing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Shape(usize),
    Ball(usize),
}

/// Something that happened during the simulation and that the game will want
/// to know about.
///
/// In the original each piece calls the table's script directly
/// (`FireGroupEvent`), which ties the physics to the VBScript engine. Here the
/// events are **queued** and whoever wants to reads them: the rules engine,
/// the sound, the animation, a test. The physics does not know who is
/// listening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A ball entered or left a trigger.
    Trigger {
        trigger: usize,
        ball: usize,
        crossing: Crossing,
    },
    /// A ball hit a shape that was asked to report.
    ///
    /// Only shapes marked with [`Engine::report_hits`] produce these. Without
    /// that filter a table would queue a few hundred events a second — every
    /// bounce off every wall — and the listener would throw away nearly all of
    /// them. The mark costs one bit and is set once, when the game works out
    /// which parts the script actually has a handler for.
    Hit {
        shape: usize,
        ball: usize,
        /// How hard, along the collision normal. Visual Pinball's parts have a
        /// threshold below which they do not fire, and this is what it is
        /// compared against.
        speed: f32,
    },
    /// A ball left a shape that reports comings and goings.
    ///
    /// Only the kicker produces this: `kicker.cpp:1189` is the one place in the
    /// whole original where something that is not a trigger fires
    /// `DISPID_HitEvents_Unhit`. A saucer is a switch, and a switch that closes
    /// and never opens leaves a ROM firing its eject coil at an empty hole.
    Unhit { shape: usize, ball: usize },
    /// A slingshot's solenoid fired.
    ///
    /// Separate from [`Event::Hit`] because the original makes it separate:
    /// `LineSegSlingshot::Collide` replaces the plain segment's `Collide`
    /// outright and fires `DISPID_SurfaceEvents_Slingshot` and nothing else
    /// (`collideex.cpp:61`). A wall with a slingshot stretch therefore reports
    /// two different things depending on which stretch was hit, and a table's
    /// script is written against that: F-14 hangs the sound, the switch and the
    /// whole plastic animation off `LeftSlingShot_Slingshot`, and has no
    /// `_Hit` at all.
    ///
    /// Only queued when the solenoid actually **fires** — a ball that brushes
    /// the rubber below the threshold bounces off in silence, which is what a
    /// real one does.
    Slingshot { shape: usize, ball: usize },
}

/// A sustained contact still to be resolved.
#[derive(Debug, Clone, Copy)]
struct Contact {
    ball: usize,
    /// Which shape the ball is resting on. The flipper resolves its own
    /// contacts and needs to know it is the one being leant on.
    shape: Option<usize>,
    coll: CollisionEvent,
    friction: f32,
}

/// The simulable table: its shapes, its balls and the loop that moves them.
pub struct Engine {
    /// Milliseconds simulated, which is one per [`Engine::step`]. Only the
    /// balls' position history needs it, and that needs it in whole
    /// milliseconds rather than as a float.
    time_ms: u64,
    /// The shapes with a box, indexed by the tree.
    shapes: Vec<Shape>,
    tree: Quadtree,
    /// The infinite shapes, which are always tested.
    unbounded: Vec<usize>,
    /// The shapes that besides colliding also **move**: gates and spinners.
    ///
    /// They go in a separate list, like the original's `m_vmover`
    /// (`PhysicsEngine.h`), and that is not an organizational detail: they
    /// have to be moved on every collision sub-cycle, and walking the three
    /// thousand shapes of a table to touch eight costs more than the whole of
    /// the hit detection put together.
    movers: Vec<usize>,
    /// The bumpers, which do not move but do animate their ring. They go in
    /// their own list for the same reason as the movers: so as not to walk the
    /// three thousand shapes of the table once per step looking for four.
    /// The bumpers, which do not move but do animate their ring. They get a
    /// list of their own for the same reason the movers do: so a step does not
    /// have to walk a table's three thousand shapes to touch four.
    bumpers: Vec<usize>,
    /// The kickers, for the same reason again: after every step each one has to
    /// be asked whether the ball it was holding has finally left the hole, and
    /// that question is not worth walking three thousand shapes for.
    kickers: Vec<usize>,
    pub balls: Vec<Ball>,
    /// The zones that report. They do not collide with anything, so they do
    /// not go into the tree.
    triggers: Vec<Trigger>,
    /// What happened in the last step, for the game to read.
    events: Vec<Event>,
    pub gravity: Vec3,
    /// An acceleration imposed from outside, in VP units. It is **added** to
    /// the cabinet's: it is useful for forcing a constant acceleration from a
    /// test, or for the day there is a real accelerometer.
    pub external_accel: Vec3,
    /// The cabinet and its plumb: what turns a key into a shake and three
    /// shakes into a tilt.
    pub nudge: Nudge,
    pub slope_rad: f32,

    // Space reused between steps, so as not to allocate inside the loop.
    /// Which shapes report their hits. See [`Engine::report_hits`].
    reports_hits: Vec<bool>,
    /// Which shapes a script has switched off. See
    /// [`Engine::set_shape_enabled`].
    disabled: Vec<bool>,

    candidates: Vec<usize>,
    contacts: Vec<Contact>,
    /// State of the scatter generator. It is kept here so the simulation is
    /// **reproducible**: same seed, same game.
    rng: u32,
}

impl Engine {
    /// Builds the engine over a set of shapes.
    pub fn new(shapes: Vec<Shape>, gravity: Vec3) -> Self {
        let mut entries = Vec::new();
        let mut unbounded = Vec::new();
        let mut movers = Vec::new();
        let mut bumpers = Vec::new();
        let mut kickers = Vec::new();
        for (i, s) in shapes.iter().enumerate() {
            match s.bbox() {
                Some(bbox) => entries.push(Entry { bbox, index: i }),
                None => unbounded.push(i),
            }
            if matches!(
                s,
                Shape::Gate(_) | Shape::Spinner(_) | Shape::Flipper(_) | Shape::Plunger(_)
            ) {
                movers.push(i);
            }
            if matches!(s, Shape::Bumper(_)) {
                bumpers.push(i);
            }
            if matches!(s, Shape::Kicker(_)) {
                kickers.push(i);
            }
        }

        Self {
            time_ms: 0,
            shapes,
            tree: Quadtree::build(entries),
            unbounded,
            movers,
            bumpers,
            kickers,
            balls: Vec::new(),
            triggers: Vec::new(),
            events: Vec::new(),
            gravity,
            external_accel: Vec3::ZERO,
            nudge: Nudge::default(),
            slope_rad: 0.0,
            reports_hits: Vec::new(),
            disabled: Vec::new(),
            candidates: Vec::new(),
            contacts: Vec::new(),
            rng: 0x2545_F491,
        }
    }

    /// The gravity of a table given its slope (`PhysicsEngine::SetGravity`,
    /// `PhysicsEngine.cpp:104`).
    pub fn gravity_from_slope(slope_deg: f32, strength: f32) -> Vec3 {
        let r = slope_deg.to_radians();
        Vec3::new(0.0, r.sin() * strength, -r.cos() * strength)
    }

    pub fn add_ball(&mut self, ball: Ball) -> usize {
        self.balls.push(ball);
        self.balls.len() - 1
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// A shape, to change it while the table is running.
    ///
    /// The script does this constantly — a table raises a flipper's strength
    /// for a mode, opens a gate, turns a wall's collision off — so there has to
    /// be a way in. It is the only mutable door into the shape list, and it
    /// deliberately cannot add or remove: the collision tree is built once and
    /// an index has to keep meaning the same thing.
    pub fn shape_mut(&mut self, index: usize) -> Option<&mut Shape> {
        self.shapes.get_mut(index)
    }

    /// Turns a shape's collision on or off.
    ///
    /// The shape stays in the tree — rebuilding it mid-game is not something
    /// this engine does — and simply stops answering hit tests. For the kinds
    /// that carry their own `enabled` flag this sets that; for the rest the
    /// shape is moved out of the way is *not* an option, so they report as
    /// disabled through the same flag list the hit reports use.
    pub fn set_shape_enabled(&mut self, index: usize, on: bool) {
        match self.shapes.get_mut(index) {
            Some(Shape::Bumper(b)) => b.enabled = on,
            Some(Shape::Kicker(k)) => k.enabled = on,
            // A gate has more to put back than a flag: see `Gate::set_collidable`.
            Some(Shape::Gate(g)) => g.set_collidable(on),
            Some(Shape::Spinner(sp)) => sp.enabled = on,
            _ => {
                if self.disabled.len() < self.shapes.len() {
                    self.disabled.resize(self.shapes.len(), false);
                }
                if index < self.disabled.len() {
                    self.disabled[index] = !on;
                }
            }
        }
    }

    fn is_disabled(&self, shape: usize) -> bool {
        self.disabled.get(shape).copied().unwrap_or(false)
    }

    /// Puts a ball into a kicker, as `CreateBall` does.
    pub fn capture_ball(&mut self, shape: usize, ball: usize) {
        let Some(Shape::Kicker(k)) = self.shapes.get_mut(shape) else {
            return;
        };
        // `kicker.cpp:1113`, the first line of `DoCollide`: a kicker that
        // already has a ball takes no other, and that guard applies to a ball
        // the script has just created as much as to one that rolled in.
        //
        // Overwriting instead is not a smaller mistake than it looks. The ball
        // that was in there stays locked with nothing left pointing at it, and
        // a locked ball is skipped by every hit test and by the whole
        // simulation loop, so nothing can ever reach it again: it becomes a
        // permanent obstacle of infinite mass sitting in the hole. A table
        // whose script re-serves a stuck saucer once a second — F-14's bumper
        // popper does exactly that — piles up a new one on every retry.
        if k.captured.is_some() {
            return;
        }
        let (izq, der) = self.shapes.split_at_mut(shape);
        let _ = izq;
        if let Some(Shape::Kicker(k)) = der.first_mut()
            && let Some(b) = self.balls.get_mut(ball)
        {
            k.hold(b, ball);
        }
    }

    /// Lets a kicker's ball go with the given velocity, as `Kick` does.
    pub fn release_from_kicker(&mut self, shape: usize, velocity: Vec3) {
        self.kick_from(shape, velocity, Vec3::ZERO);
    }

    /// The same, nudging the ball by `offset` first.
    ///
    /// `KickXYZ` adds its three numbers to the ball's position before it lets
    /// go (`kicker.cpp:748-750`, where the original credits "brian's
    /// suggestion"). It is how a table gets a ball out of a saucer that is
    /// buried under a ramp: kick it along the ramp's floor rather than into the
    /// underside of it.
    pub fn kick_from(&mut self, shape: usize, velocity: Vec3, offset: Vec3) {
        let Some(Shape::Kicker(k)) = self.shapes.get_mut(shape) else {
            return;
        };
        let Some(ball) = k.captured else { return };
        if let Some(b) = self.balls.get_mut(ball) {
            b.pos += offset;
        }
        let (izq, der) = self.shapes.split_at_mut(shape);
        let _ = izq;
        if let Some(Shape::Kicker(k)) = der.first_mut()
            && let Some(b) = self.balls.get_mut(ball)
        {
            k.release(b, velocity);
        }
    }

    /// Removes the ball a kicker is holding, as `DestroyBall` does.
    ///
    /// Removing a ball renumbers the ones after it, so every index anything
    /// else is holding has to move with it. That is why this lives on the
    /// engine and is not something a caller can do by hand.
    pub fn destroy_captured_ball(&mut self, shape: usize) {
        let Some(Shape::Kicker(k)) = self.shapes.get_mut(shape) else {
            return;
        };
        let Some(ball) = k.captured.take() else {
            return;
        };
        self.remove_ball(ball);
    }

    /// Takes a ball off the table, keeping every stored index consistent.
    pub fn remove_ball(&mut self, ball: usize) {
        if ball >= self.balls.len() {
            return;
        }
        self.balls.remove(ball);
        for s in &mut self.shapes {
            if let Shape::Kicker(k) = s {
                k.captured = match k.captured {
                    Some(c) if c == ball => None,
                    Some(c) if c > ball => Some(c - 1),
                    other => other,
                };
                // The ball it is waiting to see leave has to move with it, or
                // the kicker would fire an `Unhit` for whichever ball inherits
                // the index — most often the next one served into that very
                // hole.
                k.renumber_after_removing(ball);
            }
        }
        for t in &mut self.triggers {
            t.renumber_after_removing(ball);
        }
    }

    /// Asks a shape to report when a ball hits it.
    ///
    /// Off by default, and deliberately so: a table has three thousand shapes
    /// and a ball touches something every few milliseconds, so reporting
    /// everything would drown the listener in events it does not want. The
    /// game turns this on only for the parts whose script has a handler.
    pub fn report_hits(&mut self, shape: usize, on: bool) {
        if shape >= self.shapes.len() {
            return;
        }
        if self.reports_hits.len() < self.shapes.len() {
            self.reports_hits.resize(self.shapes.len(), false);
        }
        self.reports_hits[shape] = on;
    }

    /// Whether a shape is armed to report its hits, for tests and diagnostics.
    pub fn reports_hits_at(&self, shape: usize) -> bool {
        self.reports_hits(shape)
    }

    fn reports_hits(&self, shape: usize) -> bool {
        self.reports_hits.get(shape).copied().unwrap_or(false)
    }

    pub fn add_trigger(&mut self, t: Trigger) -> usize {
        self.triggers.push(t);
        self.triggers.len() - 1
    }

    pub fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    /// Turns a trigger on or off. In the game the script does this.
    ///
    /// On turning it off, its balls are released and it is sent back to rest.
    /// The original only stops testing the collision, so a trigger that is
    /// turned off with the wire sunk **stays sunk**: the exit that would bring
    /// it back never fires again.
    pub fn set_trigger_enabled(&mut self, index: usize, enabled: bool) {
        let Some(t) = self.triggers.get_mut(index) else {
            return;
        };
        t.enabled = enabled;
        if !enabled {
            t.release_all();
            t.animate_crossing(Crossing::Exited);
        }
    }

    /// The events of the last step. They are emptied on being read.
    pub fn take_events(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.events.drain(..)
    }

    /// The indices of the flippers, in the order the table brought them in.
    pub fn flipper_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.movers
            .iter()
            .copied()
            .filter(|&i| matches!(self.shapes[i], Shape::Flipper(_)))
    }

    /// A flipper by index, to read its angle when drawing it.
    pub fn flipper(&self, index: usize) -> Option<&Flipper> {
        match self.shapes.get(index) {
            Some(Shape::Flipper(f)) => Some(f),
            _ => None,
        }
    }

    /// The indices of every plunger on the table.
    ///
    /// There can be more than one and not all of them are the player's: there
    /// are mechanisms —exit kicks, for example— built as plungers. Which one
    /// answers the button is decided by the table's bridge, which is the one
    /// that has the names.
    pub fn plunger_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.movers
            .iter()
            .copied()
            .filter(|&i| matches!(self.shapes[i], Shape::Plunger(_)))
    }

    /// The plunger, to read its position when drawing it.
    pub fn plunger(&self, index: usize) -> Option<&Plunger> {
        match self.shapes.get(index) {
            Some(Shape::Plunger(p)) => Some(p),
            _ => None,
        }
    }

    /// Starts pulling the plunger backwards.
    pub fn pull_plunger(&mut self, index: usize) {
        if let Some(Shape::Plunger(p)) = self.shapes.get_mut(index) {
            p.pull();
        }
    }

    /// Holds it where the player is holding it, from 0 at rest to 1 drawn all
    /// the way back. See [`crate::plunger::Plunger::hold_at`].
    pub fn hold_plunger(&mut self, index: usize, travel: f32) {
        if let Some(Shape::Plunger(p)) = self.shapes.get_mut(index) {
            p.hold_at(travel);
        }
    }

    /// Lets go of one that was being held, firing from where it is.
    pub fn let_go_of_plunger(&mut self, index: usize) {
        if let Some(Shape::Plunger(p)) = self.shapes.get_mut(index) {
            p.let_go();
        }
    }

    /// Lets it go.
    pub fn release_plunger(&mut self, index: usize) {
        if let Some(Shape::Plunger(p)) = self.shapes.get_mut(index) {
            p.release();
        }
    }

    /// Presses or releases a flipper's solenoid.
    ///
    /// This is what the script does in the game when a button is touched.
    /// Without a VBScript engine, the table's bridge decides which flipper
    /// answers which button and calls in here.
    pub fn set_flipper_solenoid(&mut self, index: usize, pressed: bool) {
        if let Some(Shape::Flipper(f)) = self.shapes.get_mut(index) {
            f.solenoid = pressed;
        }
    }

    /// The same number, for a caller outside the engine.
    ///
    /// The original's `rand_mt_m11()`. A kicker's `Kick` uses it to scatter the
    /// angle it fires at (`kicker.cpp:727`), and that lives in the game's item
    /// layer rather than in here — but it has to draw from **this** generator
    /// and not from the system's, for the reason below.
    pub fn random_m11(&mut self) -> f32 {
        self.next_random()
    }

    /// A number in -1..1 for the scatter.
    ///
    /// It is a generator of our own and not the system's on purpose: the
    /// physics has to be **reproducible**. With the same seed and the same
    /// inputs, the same game — which is what makes it possible to have
    /// regression tests over trajectories.
    fn next_random(&mut self) -> f32 {
        // xorshift32: cheap and with a period far beyond enough for this.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Hits the cabinet, with the scatter the original puts on it.
    ///
    /// `angle_deg` is measured as in Visual Pinball: 0 is pushing the table
    /// forwards, 75 the left nudge and 285 the right one. Each impulse comes
    /// out with the angle and the force shifted at random
    /// (`InputManager.cpp:731-739`), which is what keeps two nudges in a row
    /// from giving exactly the same thing.
    ///
    /// The scatter comes from **the engine's** generator and not the system's,
    /// for the same reason as the one on hits: the game has to be
    /// reproducible.
    pub fn nudge_at(&mut self, angle_deg: f32) {
        const BASE_FORCE: f32 = 2.0;
        let offset = self.next_random() * 0.5 * 15.0 * BASE_FORCE;
        let force = (1.0 + self.next_random() * 0.4) * BASE_FORCE;
        self.nudge.impulse(angle_deg + offset, force);
    }

    /// Whether the table is tilted.
    pub fn is_tilted(&self) -> bool {
        self.nudge.tilt
    }

    /// One complete physics step: one millisecond of table time.
    pub fn step(&mut self) {
        // What the ball feels is the sum of the two: the acceleration someone
        // imposes from outside and the cabinet's, shaking.
        let acceleration = self.external_accel + self.nudge.step().extend(0.0);

        for b in &mut self.balls {
            b.update_velocities(self.gravity, acceleration, self.slope_rad);
        }
        // The pieces that rotate update their velocity only once per step, not
        // once per collision sub-cycle: that is where the gravity that brings
        // them back to rest comes in (`PhysicsEngine.cpp:465`).
        for &i in &self.movers {
            match &mut self.shapes[i] {
                Shape::Gate(g) => g.update_velocities(),
                Shape::Spinner(sp) => sp.update_velocities(),
                Shape::Flipper(f) => f.update_velocities(),
                Shape::Plunger(pl) => pl.update_velocities(),
                _ => {}
            }
        }
        self.simulate_cycle(PHYS_FACTOR);

        // Where every ball ended up this millisecond
        // (`PhysicsEngine.cpp:472`). It goes after the cycle and before
        // anything else looks at the balls.
        self.time_ms += 1;
        let now = self.time_ms;
        for b in &mut self.balls {
            b.record_position(now);
        }

        self.check_triggers();
        self.check_kickers();

        // The bumper rings, after the cycle: if a bumper fired on this step,
        // the ring has to start dropping on this step.
        for &i in &self.bumpers {
            if let Shape::Bumper(b) = &mut self.shapes[i] {
                b.animate(1.0);
            }
        }
    }

    /// Looks at which balls entered or left each zone.
    ///
    /// It goes **after** the collision cycle, with the balls already in their
    /// final position: a trigger has to report where the ball ended up at the
    /// end of the step, not where it was at some intermediate instant.
    fn check_triggers(&mut self) {
        for (t, trigger) in self.triggers.iter_mut().enumerate() {
            // The animation advances even if the trigger is off: if it gets
            // turned off with the wire sunk, it still has to finish coming
            // back.
            //
            // A physics step is one millisecond of table time. The original
            // ties this to the video frame, so a trigger sinks faster on a
            // machine that runs at more frames per second; tying it to the
            // table's clock makes it independent of that.
            trigger.animate(1.0);

            if !trigger.enabled {
                continue;
            }
            for (b, ball) in self.balls.iter().enumerate() {
                if let Some(crossing) = trigger.update(b, ball) {
                    trigger.animate_crossing(crossing);
                    self.events.push(Event::Trigger {
                        trigger: t,
                        ball: b,
                        crossing,
                    });
                }
            }
        }
    }

    /// Settles up with every kicker at the end of the step.
    ///
    /// Two things, and both are the original's `m_vpVolObjs` bookkeeping seen
    /// from the other side — see [`crate::parts::Kicker::contains`] for why a
    /// kicker needs a standing test as well as a swept one.
    ///
    /// A ball that is **inside** a hole and not in its set is one the hole has
    /// not dealt with: the original reports that as a hit at time zero
    /// (`collide.cpp:277-287`) and runs the whole of `DoCollide` on it. That is
    /// what catches a ball put down on a saucer, and what keeps working on one
    /// that has rolled in and is loitering — steering it down the bowl, step
    /// after step, until it is low enough to be taken.
    ///
    /// A ball that is in the set and **no longer inside** has gone, and that is
    /// the kicker's `Unhit` (`kicker.cpp:1189`).
    ///
    /// After the cycle, for the same reason as [`Engine::check_triggers`]: what
    /// matters is where the ball ended the step.
    fn check_kickers(&mut self) {
        for k in 0..self.kickers.len() {
            let s = self.kickers[k];

            // Arrivals first. Split off so the ball and the kicker can be held
            // mutably at once.
            for b in 0..self.balls.len() {
                let Some(Shape::Kicker(kicker)) = self.shapes.get(s) else {
                    continue;
                };
                if !kicker.enabled
                    || kicker.captured.is_some()
                    || kicker.touched(b)
                    || kicker.has_ball(b)
                    || self.balls[b].locked
                    || !kicker.contains(&self.balls[b])
                {
                    continue;
                }
                // The circle's normal at the ball, which is what the swept test
                // would have handed over. Straight up for a ball dead in the
                // middle, where there is no direction to speak of.
                let centre = kicker.circle.center;
                let flat = self.balls[b].pos.truncate() - centre;
                let normal = if flat.length_squared() > 1.0e-8 {
                    let n = flat.normalize();
                    Vec3::new(n.x, n.y, 0.0)
                } else {
                    Vec3::Z
                };

                let (left, right) = self.shapes.split_at_mut(s);
                let _ = left;
                let Some(Shape::Kicker(kicker)) = right.first_mut() else {
                    continue;
                };
                let took = kicker.take_ball(&mut self.balls[b], b, normal);
                if took != crate::parts::KickerHit::Passed && self.reports_hits(s) {
                    self.events.push(Event::Hit {
                        shape: s,
                        ball: b,
                        speed: f32::INFINITY,
                    });
                }
            }

            // Then departures, and the step's slate wiped.
            let reports = self.reports_hits(s);
            let Some(Shape::Kicker(kicker)) = self.shapes.get_mut(s) else {
                continue;
            };
            kicker.end_step();
            if kicker.captured.is_some() {
                continue; // still holding it: it has not gone anywhere
            }
            for (b, ball) in self.balls.iter().enumerate() {
                if kicker.check_exit(b, ball) && reports {
                    self.events.push(Event::Unhit { shape: s, ball: b });
                }
            }
        }
    }

    /// Advances `dtime` resolving hits in order (`PhysicsEngine.cpp:428`).
    pub fn simulate_cycle(&mut self, mut dtime: f32) {
        let mut static_cnts = STATICCNTS;

        while dtime > 0.0 {
            let mut hittime = dtime;
            self.contacts.clear();

            // For each ball, the nearest hit within the time that is left. It
            // is kept aside so as not to fight with the mutable borrow.
            let mut chosen: Vec<Option<(Target, CollisionEvent)>> = vec![None; self.balls.len()];

            for (i, pick) in chosen.iter_mut().enumerate() {
                if self.balls[i].locked {
                    continue;
                }
                let mut best: Option<(Target, CollisionEvent)> = None;
                let mut limit = hittime;

                // Against the other balls. It goes first because it narrows
                // `limit` down and saves the tree some work.
                for j in 0..self.balls.len() {
                    if j == i {
                        continue;
                    }
                    // The normal points from the other ball towards this one,
                    // which is the convention of `collide_balls`.
                    let Some(coll) = hit_test_balls(&self.balls[j], &self.balls[i], limit) else {
                        continue;
                    };
                    if coll.hit_time < 0.0 || coll.hit_time > limit {
                        continue;
                    }
                    limit = coll.hit_time;
                    best = Some((Target::Ball(j), coll));
                }

                // The infinite shapes are always tested: the playfield and the
                // glass do not go into the tree.
                for &s in &self.unbounded {
                    if self.is_disabled(s) {
                        continue;
                    }
                    Self::test_shape(
                        &self.shapes[s],
                        s,
                        &self.balls[i],
                        limit,
                        &mut best,
                        &mut limit,
                        i,
                        &mut self.contacts,
                    );
                }

                // And the rest come out of the tree.
                self.candidates.clear();
                let bbox = self.balls[i].hit_bbox();
                let candidates = &mut self.candidates;
                self.tree.query(&bbox, |idx| candidates.push(idx));

                for k in 0..self.candidates.len() {
                    let s = self.candidates[k];
                    // A shape the script switched off stays in the tree and
                    // simply stops being tested; rebuilding the tree mid-game
                    // is not something this engine does.
                    if self.is_disabled(s) {
                        continue;
                    }
                    Self::test_shape(
                        &self.shapes[s],
                        s,
                        &self.balls[i],
                        limit,
                        &mut best,
                        &mut limit,
                        i,
                        &mut self.contacts,
                    );
                }

                if let Some((_, coll)) = &best
                    && coll.hit_time <= hittime
                {
                    hittime = coll.hit_time;
                    // Too many events too close together: force an advance so
                    // we do not stay in place.
                    if hittime < STATICTIME {
                        if static_cnts == 0 {
                            hittime = STATICTIME;
                        } else {
                            static_cnts -= 1;
                        }
                    }
                }
                *pick = best;
            }

            // A long step means the pile-up cleared: the original gives the
            // credits back right there (`PhysicsEngine.cpp:608`). Without
            // this, ten events together leave the engine with no headroom for
            // the rest of the game.
            if hittime > STATICTIME {
                static_cnts = STATICCNTS;
            }

            // The plunger that has just hit freezes where it is. This has to
            // come before moving it: the limit is its position now, not the
            // one after advancing.
            for pick in chosen.iter().flatten() {
                if let (Target::Shape(s), _) = pick
                    && let Shape::Plunger(pl) = &mut self.shapes[*s]
                {
                    pl.mark_travel_limit();
                }
            }

            // Move every ball up to the instant of the first hit, and with
            // them the pieces that rotate.
            for b in &mut self.balls {
                b.update_displacements(hittime);
            }
            for &i in &self.movers {
                match &mut self.shapes[i] {
                    Shape::Gate(g) => g.update_displacements(hittime),
                    Shape::Spinner(sp) => sp.update_displacements(hittime),
                    Shape::Flipper(f) => f.update_displacements(hittime),
                    Shape::Plunger(pl) => pl.update_displacements(hittime),
                    _ => {}
                }
            }

            // And resolve the hits that land right there.
            for (i, pick) in chosen.iter().copied().enumerate() {
                let Some((target, coll)) = pick else {
                    continue;
                };
                if coll.hit_time > hittime + 1e-6 {
                    continue; // it was later: it gets resolved next time round
                }

                // A pair of balls is resolved only once, from the one with the
                // lower index. If the other one did not pick this one, it still
                // has to be resolved, because from its side it never will be.
                if let Target::Ball(j) = target {
                    let the_other_too = chosen[j].is_some_and(|(o, _)| o == Target::Ball(i));
                    if the_other_too && j < i {
                        continue;
                    }
                    let (lower, upper) = if i < j { (i, j) } else { (j, i) };
                    let (left, right) = self.balls.split_at_mut(upper);
                    // `coll` has the normal from `j` towards `i`; if the order
                    // gets flipped, so does the normal.
                    let mut c = coll;
                    if lower == i {
                        c.hit_normal = -c.hit_normal;
                    }
                    collide_balls(&mut left[lower], &mut right[0], &c);
                    continue;
                }
                let Target::Shape(s) = target else {
                    unreachable!()
                };

                // How hard, measured **before** the collision changes the
                // velocity. Afterwards the ball is already moving the other
                // way and the number means nothing.
                // A slingshot is left out here on purpose: it announces
                // itself below, and only when its solenoid fires. In the
                // original its `Collide` replaces the plain segment's rather
                // than adding to it, so it never reports a plain hit.
                // A kicker is left out for the same reason: it reports its hit
                // only when it actually takes the ball (`kicker.cpp:1157`, the
                // `FireGroupEvent` inside the `hitEvent` branch). A ball that
                // rolls over the lip of a hole it does not fall into has not
                // hit anything a table wants to hear about, and announcing it
                // would pulse a saucer's switch every time a ball crossed it.
                let is_slingshot = matches!(self.shapes[s], Shape::Slingshot(_));
                let is_kicker = matches!(self.shapes[s], Shape::Kicker(_));
                if !is_slingshot
                    && !is_kicker
                    && self.reports_hits(s)
                    && self.balls[i].fires_event()
                {
                    let speed = -coll.hit_normal.dot(self.balls[i].vel);
                    self.events.push(Event::Hit {
                        shape: s,
                        ball: i,
                        speed,
                    });
                }

                let scatter = self.next_random();
                // The slingshot resolves its own hit: first it kicks and then
                // it bounces.
                let mut solenoid_fired = false;
                let mut kicker_took_it = false;
                match &mut self.shapes[s] {
                    Shape::Slingshot(sling) => {
                        solenoid_fired = sling.collide(&mut self.balls[i], &coll, scatter);
                    }
                    Shape::Bumper(bumper) => {
                        bumper.collide(&mut self.balls[i], &coll, scatter);
                    }
                    Shape::Kicker(kicker) => {
                        // A kicker resolves the whole hit itself, both ways
                        // round: it either takes the ball or steers it across
                        // the bowl (`DoChangeBallVelocity`). Neither is a wall
                        // bounce — a hole made out of `collide_3d_wall` is a
                        // post — so nothing else happens to the ball here, and
                        // the scatter this shape would have used goes unspent.
                        let took = kicker.take_ball(&mut self.balls[i], i, coll.hit_normal);
                        kicker_took_it = took != crate::parts::KickerHit::Passed;
                        // The end-of-step sweep must not act on this ball
                        // again: it has had its millisecond.
                        kicker.mark_touched(i);
                    }
                    // Gate and spinner are **pass-through**: the ball carries
                    // straight on without being deflected and the only thing
                    // that changes is the angle of the piece. On a one-way
                    // gate, what stops the ball on the wrong side is a separate
                    // rigid `Line`. The flipper is the only one that **gives
                    // back** impulse: the ball brakes it as much as it pushes
                    // the ball.
                    Shape::Flipper(flipper) => flipper.collide(&mut self.balls[i], &coll),
                    Shape::Plunger(pl) => pl.collide(&mut self.balls[i], &coll, scatter),
                    Shape::Gate(gate) => gate.collide(&self.balls[i], &coll),
                    Shape::Spinner(spinner) => {
                        spinner.collide(&self.balls[i], &coll);
                    }
                    other => {
                        let material = *other.material();
                        self.balls[i].collide_3d_wall(coll.hit_normal, &material, &coll, scatter);
                    }
                }

                // After the bounce, and only if the solenoid went off. That is
                // the original's order too: it works out whether the ball was
                // fast enough *before* touching the velocity, kicks, bounces,
                // and asks the repeat filter last (`collideex.cpp:88`).
                if solenoid_fired && self.reports_hits(s) && self.balls[i].fires_event() {
                    self.events.push(Event::Slingshot { shape: s, ball: i });
                }
                // The kicker's hit, which only exists when the hole took the
                // ball. The repeat filter is skipped on purpose: the original
                // does not run it either here — it calls `FireGroupEvent`
                // directly rather than `FireHitEvent` (`kicker.cpp:1157`) — and
                // it must not, because a ball that came to rest in a saucer has
                // not moved a quarter of a unit since the last event and the
                // filter would swallow the one report the table is waiting for.
                if kicker_took_it && self.reports_hits(s) {
                    self.events.push(Event::Hit {
                        shape: s,
                        ball: i,
                        // A kicker has no threshold: the hole either takes the
                        // ball or it does not.
                        speed: f32::INFINITY,
                    });
                }
            }

            // The sustained contacts go last: they are the ball resting on
            // something, not a hit.
            // Which balls are resting on something this cycle. The original
            // asks each ball's own pending collision whether it is within
            // `PHYS_TOUCH` (`PhysicsEngine.cpp:695`); the port does not keep a
            // collision on the ball, and having produced a contact is the same
            // question asked the other way round.
            let resting: Vec<usize> = self.contacts.iter().map(|c| c.ball).collect();

            for c in std::mem::take(&mut self.contacts) {
                let g = self.gravity;
                // `collide.h:103`: "Ball, Spinner and Gate do nothing here,
                // Flipper has a specialized handling". The generic one treats
                // whatever the ball is lying on as an immovable wall, which is
                // right for everything on a table except the one part that is
                // itself accelerating under the ball.
                let flipper = c.shape.and_then(|i| match self.shapes.get(i) {
                    Some(Shape::Flipper(_)) => Some(i),
                    _ => None,
                });
                match flipper {
                    Some(i) => {
                        let Some(Shape::Flipper(f)) = self.shapes.get_mut(i) else {
                            continue;
                        };
                        f.contact(&mut self.balls[c.ball], &c.coll, hittime, g);
                    }
                    None => {
                        self.balls[c.ball].handle_static_contact(&c.coll, c.friction, hittime, g);
                    }
                }
            }

            self.damp_resting_spin(&resting);

            dtime -= hittime;
        }
    }

    /// Takes the spin off a ball that has stopped going anywhere.
    ///
    /// Port of the `C_BALL_SPIN_HACK` block at `PhysicsEngine.cpp:690-730`.
    /// The original's own comment on the damping factor says what it is for:
    /// *do not kill spin completely, otherwise stuck balls will happen during
    /// regular gameplay*. A ball wedged against something can keep enough
    /// angular momentum to hold itself there, and this is what shakes it loose.
    ///
    /// The test is a ratio: planar angular momentum against how far the ball
    /// has actually travelled in the last eighty and ninety milliseconds. Spin
    /// with no travel is what gets damped; a ball genuinely rolling has plenty
    /// of both.
    fn damp_resting_spin(&mut self, resting: &[usize]) {
        if resting.is_empty() {
            return;
        }
        let now = self.time_ms;
        for &i in resting {
            let Some(ball) = self.balls.get(i) else {
                continue;
            };
            // Both readings have to exist: a ball younger than a tenth of a
            // second has nowhere to have travelled from.
            let (Some(old0), Some(old1)) = (
                ball.old_position(now.saturating_sub(90)),
                ball.old_position(now.saturating_sub(80)),
            ) else {
                continue;
            };
            let planar = |a: Vec3| {
                let d = a - ball.pos;
                d.x * d.x + d.y * d.y
            };
            let travelled = planar(old0).max(planar(old1));
            let spin = ball.angular_momentum.x * ball.angular_momentum.x
                + ball.angular_momentum.y * ball.angular_momentum.y;
            let ratio = spin / travelled;
            if !ratio.is_finite() || ratio <= 666.0 {
                continue;
            }
            let damp = (1.0 - (ratio - 666.0) / 10000.0).clamp(0.23, 1.0);
            self.balls[i].angular_momentum *= damp;
        }
    }

    /// Tests a shape and keeps the hit if it is the nearest one.
    ///
    /// Contacts do not compete for being "the first hit": they are gathered
    /// aside and all resolved at the end of the cycle (`collide.cpp:625`).
    #[expect(
        clippy::too_many_arguments,
        reason = "it is the inside of the hot loop"
    )]
    fn test_shape(
        shape: &Shape,
        index: usize,
        ball: &Ball,
        limit: f32,
        best: &mut Option<(Target, CollisionEvent)>,
        new_limit: &mut f32,
        ball_idx: usize,
        contacts: &mut Vec<Contact>,
    ) {
        let Some(coll) = shape.hit_test(ball, limit) else {
            return;
        };
        if coll.hit_time < 0.0 || coll.hit_time > limit {
            return;
        }

        if coll.is_contact {
            contacts.push(Contact {
                ball: ball_idx,
                shape: Some(index),
                coll,
                friction: shape.material().friction,
            });
        } else {
            *best = Some((Target::Shape(index), coll));
            *new_limit = coll.hit_time;
        }
    }
}
