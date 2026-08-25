//! The active pieces of the table: bumpers, gates, spinners and kickers.
//!
//! They all share one idea: stopping the ball is not enough, they have to
//! **react**. The bumper kicks it, the gate opens and lets it through, the
//! spinner spins, the kicker captures it.
//!
//! Port of `physics/collideex.cpp` and of `parts/kicker.cpp`.

use crate::ball::Ball;
use crate::collision::{CollisionEvent, Material};
use crate::constants::{PHYS_FACTOR, PHYS_SKIN};
use crate::shapes::{HitCircle, LineSeg};
use vpw_math::{Vec2, Vec3};

/// A bumper: a circle that kicks.
///
/// Port of `BumperHitCircle::Collide` (`collideex.cpp:24`).
#[derive(Debug, Clone)]
pub struct Bumper {
    pub circle: HitCircle,
    /// How much velocity it adds to the ball.
    pub force: f32,
    /// How much velocity it has to be hit with in order to fire.
    pub threshold: f32,
    /// The file's `HAHE`. It does **not** only decide whether the script hears
    /// about the hit: `BumperHitCircle::Collide` (`collideex.cpp:33`) puts it
    /// in the same condition as the threshold, so a bumper with it off does not
    /// kick either — it is a plain round wall. A table that decorates a lane
    /// with a dead bumper gets a post; ignoring the flag gets a live bumper
    /// that flings the ball back out of a lane it was meant to roll down.
    pub hit_event: bool,
    pub enabled: bool,
    /// Whether it has just fired, so the animation knows about it.
    pub hit: bool,

    /// How far the ring dropped from its place, in VP units. Always negative
    /// or zero.
    pub ring_offset: f32,
    /// How far the ring moves per table millisecond.
    pub ring_speed: f32,
    /// How far down it goes.
    pub ring_drop: f32,
    /// Whether it is going down; when it reaches the bottom, it comes up.
    ring_dropping: bool,
    ring_animating: bool,
}

impl Bumper {
    pub fn new(circle: HitCircle, force: f32, threshold: f32) -> Self {
        Self {
            circle,
            force,
            threshold,
            // The editor's default is on (`Settings_properties.inl:917`), and
            // it is what a file that predates the tag was saved with.
            hit_event: true,
            enabled: true,
            hit: false,
            ring_offset: 0.0,
            ring_speed: 0.5,
            ring_drop: 0.0,
            ring_dropping: false,
            ring_animating: false,
        }
    }

    /// Moves the ring. `dt_ms` is in table milliseconds, not video ones.
    ///
    /// The ring drops all at once when the bumper fires and comes back on its
    /// own. It is the only piece that animates **by itself**: the others
    /// follow an angle or a position the physics already computes, and this
    /// one has neither — the bumper is a fixed circle, and the ring that goes
    /// up and down is pure decoration.
    ///
    /// Like the trigger, the animation is tied to the table's clock and not to
    /// the video frame: that way the ring takes the same time on a machine
    /// running at 30 frames per second as on one running at 144.
    pub fn animate(&mut self, dt_ms: f32) {
        if self.ring_drop <= 0.0 {
            return;
        }
        if self.hit {
            self.hit = false;
            self.ring_animating = true;
            self.ring_dropping = true;
        }
        if !self.ring_animating {
            return;
        }

        let step = if self.ring_dropping {
            -self.ring_speed
        } else {
            self.ring_speed
        };
        self.ring_offset += step * dt_ms;

        if self.ring_dropping {
            if self.ring_offset <= -self.ring_drop {
                self.ring_offset = -self.ring_drop;
                self.ring_dropping = false;
            }
        } else if self.ring_offset >= 0.0 {
            self.ring_offset = 0.0;
            self.ring_animating = false;
        }
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        self.circle.hit_test(ball, dtime)
    }

    /// Resolves the hit. Returns `true` if the bumper fired.
    ///
    /// The order matters: first the incoming velocity is computed, then the
    /// bounce happens, and **only after that** the kick is added. The other
    /// way around, the threshold would be measured against a velocity that has
    /// already changed.
    pub fn collide(&mut self, ball: &mut Ball, coll: &CollisionEvent, scatter: f32) -> bool {
        let dot = coll.hit_normal.dot(ball.vel);
        ball.collide_3d_wall(coll.hit_normal, &self.circle.material, coll, scatter);

        // `collideex.cpp:33` — one condition, not two: the kick and the event
        // are both behind `m_d.m_hitEvent && dot <= -threshold`.
        let fires = self.hit_event && dot <= -self.threshold;
        if fires {
            ball.vel += coll.hit_normal * self.force;
            self.hit = true;
        }
        fires
    }
}

/// A gate: a one-way door.
///
/// The ball pushes it and goes through; from the other side, it bounces. It is
/// what keeps a ball from coming back the way it came.
///
/// The ball is **not** deflected when it crosses: the gate is a pass-through
/// line, and what stops it on the wrong side is a separate rigid `LineSeg` the
/// table adds only when the door is one-way (`gate.cpp:PhysicSetup`).
///
/// Port of `HitGate` and `GateMoverObject` (`collideex.cpp:192` and `:303`).
#[derive(Debug, Clone)]
pub struct Gate {
    /// The axis of the door, as a pass-through line. It is tested on both
    /// faces.
    pub seg: LineSeg,
    /// Height of the door. It divides the velocity, so hitting it low makes it
    /// spin more — like a real door, more leverage down low.
    pub height: f32,

    pub angle: f32,
    pub angle_speed: f32,
    pub angle_min: f32,
    pub angle_max: f32,
    /// The limits **as the file declares them**, which are not the same thing
    /// as the two above.
    ///
    /// The original keeps the pair twice — `m_d.m_angleMin/Max` on the part and
    /// `m_angleMin/Max` on the mover — and `SetOpenAngle` / `SetCloseAngle`
    /// (`gate.cpp:150` and `:171`) clamp the script's number against the file's
    /// pair before writing the mover's. Keeping only the mover's would make the
    /// limits a ratchet: every `OpenAngle` a script wrote would narrow the range
    /// the next one is allowed to ask for, and a gate that a table opens and
    /// closes on a timer would close on itself.
    pub limit_min: f32,
    pub limit_max: f32,
    /// Already raised to `PHYS_FACTOR`: it is applied once per physics step.
    pub damping: f32,
    /// How hard gravity pulls the door back towards rest.
    pub gravity_factor: f32,

    pub two_way: bool,
    /// The file's `GCOL`, and **not** the same as [`Gate::enabled`].
    ///
    /// `Open` turns the leaf off while it is held open and puts it back to this
    /// on the way down (`gate.cpp:747` and `:753`) — to the file's value, note,
    /// not to whatever a script last wrote through `Collidable`, because
    /// `put_Collidable` deliberately leaves `m_d.m_collidable` alone once the
    /// player is running (`gate.cpp:840-857`). Keeping the two apart is what
    /// makes `Open = True : Open = False` end where it started.
    pub collidable: bool,
    /// Whether the script locked it open.
    pub open: bool,
    /// Which way it opened. It defines the sign of its limits.
    pub hit_direction: bool,
    /// Whether it is the script moving it and not a ball.
    pub forced_move: bool,
    pub enabled: bool,
}

/// The data the table builds a gate or a spinner from.
///
/// They go together because they describe the same thing —an axis that
/// rotates— and because passing them loose would be nine positional
/// arguments, all `f32`, impossible to read at the call site.
#[derive(Debug, Clone, Copy)]
pub struct PivotAxis {
    pub center: Vec2,
    pub length: f32,
    pub rotation_deg: f32,
    /// Height of the piece above its base. It divides the velocity of the hit.
    pub height: f32,
    pub z_low: f32,
    pub angle_min: f32,
    pub angle_max: f32,
    /// Not raised to `PHYS_FACTOR`: that is fixed up when the piece is built.
    pub damping: f32,
}

impl Gate {
    /// Builds the door from its axis.
    pub fn new(pivot: PivotAxis, gravity_factor: f32, two_way: bool) -> Self {
        let (min, max) = sort_limits(pivot.angle_min, pivot.angle_max);
        Self {
            seg: pivot_segment(&pivot),
            height: pivot.height,
            angle: min,
            angle_speed: 0.0,
            angle_min: min,
            angle_max: max,
            limit_min: min,
            limit_max: max,
            damping: pivot.damping.powf(PHYS_FACTOR),
            gravity_factor,
            two_way,
            collidable: true,
            open: false,
            hit_direction: false,
            forced_move: false,
            enabled: true,
        }
    }

    /// Tests both faces (`HitGate::HitTest`, `collideex.cpp:246`).
    ///
    /// `hit_flag` encodes **which face** the ball came in through, and from
    /// that we later work out whether the door gives way or only bounces.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        if let Some(mut c) = self.seg.hit_test_through(ball, dtime) {
            c.hit_flag = false;
            return Some(c);
        }
        if let Some(mut c) = self.seg.flipped().hit_test_through(ball, dtime) {
            c.hit_flag = true;
            return Some(c);
        }
        None
    }

    /// The hit (`collideex.cpp:192`).
    pub fn collide(&mut self, ball: &Ball, coll: &CollisionEvent) {
        let dot = coll.hit_normal.dot(ball.vel);
        let h = self.height * 0.5;

        let mut speed = dot.abs();
        if h.abs() > 1.0 {
            speed /= h;
        }
        self.angle_speed = speed;

        // Hit from behind and it is not a two-way one: it does not go through,
        // it only bounces a little. That `1/50` comes from the original, so it
        // does not look rigid.
        if !coll.hit_flag && !self.two_way {
            self.hit_direction = dot > 0.0;
            self.angle_speed *= 1.0 / 50.0;
            return;
        }

        self.hit_direction = false;
        if coll.hit_flag && self.two_way {
            self.angle_speed = -self.angle_speed;
        }
    }

    /// Moves the door (`GateMoverObject::UpdateDisplacements`).
    ///
    /// The order is the original's and it is not arbitrary: **first** it
    /// clamps against the limits with the angle from the previous round, and
    /// **then** it integrates.
    pub fn update_displacements(&mut self, dtime: f32) {
        if self.two_way {
            if self.angle.abs() > self.angle_max {
                self.angle = self.angle_max.copysign(self.angle);
                self.hit_stop(true);
            }
            if self.angle.abs() < self.angle_min {
                self.angle = self.angle_min.copysign(self.angle);
                self.hit_stop(false);
            }
        } else {
            // On a one-way one, the door opens towards the side it was pushed
            // from.
            let direction = if self.hit_direction { -1.0 } else { 1.0 };
            if direction * self.angle > self.angle_max {
                self.angle = direction * self.angle_max;
                self.hit_stop(true);
            }
            if direction * self.angle < self.angle_min {
                self.angle = direction * self.angle_min;
                self.hit_stop(false);
            }
        }

        if self.angle_speed == 0.0 {
            self.forced_move = false;
        }
        self.angle += self.angle_speed * dtime;
    }

    /// Bounce against a stop. `opening` tells the opening one from the closing
    /// one.
    fn hit_stop(&mut self, opening: bool) {
        if !self.forced_move {
            // The extra `0.8` comes from the original: it brakes a bit faster
            // so the door does not sit there rattling against the stop.
            self.angle_speed = -self.angle_speed * self.damping * 0.8;
        } else if (opening && self.angle_speed > 0.0) || (!opening && self.angle_speed < 0.0) {
            self.angle_speed = 0.0;
        }
    }

    /// Damps the door and lets it fall back to rest
    /// (`GateMoverObject::UpdateVelocities`, `collideex.cpp:303`).
    ///
    /// The `sin(angle)` is what closes it on its own: the center of gravity
    /// pulls downwards, and it pulls harder the further open it is. Without
    /// that term the door stays open forever.
    pub fn update_velocities(&mut self) {
        if self.open {
            return;
        }
        if self.angle.abs() < self.angle_min + 0.01 && self.angle_speed.abs() < 0.01 {
            // It brakes a little early so it does not end up animating forever
            // with a slow ball.
            self.angle = self.angle_min;
            self.angle_speed = 0.0;
        }
        if self.angle_speed != 0.0 && self.angle != self.angle_min {
            self.angle_speed -= self.angle.sin() * self.gravity_factor * (PHYS_FACTOR / 100.0);
            self.angle_speed *= self.damping;
        }
    }

    /// `Gate.Open = True` / `= False` (`Gate::put_Open`, `gate.cpp:736`).
    ///
    /// Setting the flag is not what opens a gate, and that was the bug: `open`
    /// is only read by [`Gate::update_velocities`], where all it does is stop
    /// gravity pulling the leaf shut. The leaf **also has to stop colliding**,
    /// and it has to be kicked so it actually swings up — a gate whose script
    /// opens it and whose angle never leaves the stop is drawn shut.
    ///
    /// The caller has one more thing to do that a gate cannot do for itself:
    /// on a one-way gate the thing that stops the ball is not the leaf but a
    /// separate rigid `LineSeg` beside it, and `gate.cpp:749` switches that off
    /// too. Leave it on and a script that opens a gate to let the ball out gets
    /// an invisible wall instead. See the caller in the game's item layer.
    pub fn set_open(&mut self, open: bool) {
        self.hit_direction = false;
        // Back to the file's limits first: a `Move` may have narrowed them.
        self.angle_max = self.limit_max;
        self.angle_min = self.limit_min;
        self.forced_move = true;
        self.open = open;

        if open {
            self.enabled = false;
            if self.angle < self.angle_max {
                self.angle_speed = 0.2;
            }
        } else {
            self.enabled = self.collidable;
            if self.angle > self.angle_min {
                self.angle_speed = -0.2;
            }
        }
    }

    /// `Gate.Collidable = ...` (`Gate::put_Collidable`, `gate.cpp:840`).
    ///
    /// The `angle_min = 0` is the original's and it is not decoration: a
    /// collidable gate has to be able to lie flat, and a file that shipped a
    /// non-zero closing angle would otherwise leave the leaf propped up in the
    /// ball's way the moment a script switched collision back on.
    pub fn set_collidable(&mut self, on: bool) {
        self.enabled = on;
        self.angle_max = self.limit_max;
        self.angle_min = self.limit_min;
        if on {
            self.angle_min = 0.0;
        }
    }

    /// `Gate.OpenAngle = ...` in radians (`Gate::SetOpenAngle`, `gate.cpp:150`).
    ///
    /// Which of the two limits it writes depends on where the new angle falls,
    /// which looks like a mistake and is not: a gate can be built with its
    /// limits either way round, and the original picks whichever end the value
    /// belongs to rather than assuming.
    pub fn set_open_angle(&mut self, angle: f32) {
        let v = angle.clamp(self.limit_min, self.limit_max);
        if self.angle_min < v {
            self.angle_max = v;
        } else {
            self.angle_min = v;
        }
    }

    /// `Gate.CloseAngle = ...` in radians (`Gate::SetCloseAngle`, `gate.cpp:171`).
    pub fn set_close_angle(&mut self, angle: f32) {
        let v = angle.clamp(self.limit_min, self.limit_max);
        if self.angle_max > v {
            self.angle_min = v;
        } else {
            self.angle_max = v;
        }
    }

    /// `Gate.Move dir, speed, angle` (`Gate::Move`, `gate.cpp:865`).
    ///
    /// The original's own comment: "animate gate toward a given direction or
    /// angle at the given speed, **disabling physics**, for graphic effects
    /// only". So it turns the natural swing off and the collision with it —
    /// which is why, like [`Gate::set_open`], the caller has to switch the
    /// one-way gate's blocking segment off as well.
    ///
    /// `dir` is 0 to use `angle_rad`, -1 to close and +1 to open; `speed_deg`
    /// at or below zero means the default. Returns the direction it settled on,
    /// which is what the caller needs to know: a `Move` that resolves to "stay
    /// where you are" must not leave the leaf drifting.
    pub fn move_toward(&mut self, mut dir: i32, speed_deg: f32, angle_deg: f32) -> i32 {
        self.hit_direction = false;
        self.forced_move = true;
        // `Move` always turns the natural swing off, and collision with it.
        self.open = true;
        self.enabled = false;

        let speed = if speed_deg <= 0.0 {
            0.2 // the original's default gate angle speed
        } else {
            speed_deg.to_radians()
        };

        let mut angle = angle_deg;
        if dir == 0 || angle != 0.0 {
            let target = angle.to_radians().clamp(self.limit_min, self.limit_max);
            angle = target;
            let da = target - self.angle;
            dir = if da > 1.0e-5 {
                1
            } else if da < -1.0e-5 {
                -1
            } else {
                self.angle_speed = 0.0;
                0
            };
        } else {
            angle = if dir < 0 {
                self.limit_min
            } else {
                self.limit_max
            };
        }

        if dir > 0 {
            self.angle_max = angle;
            if self.angle < self.angle_max {
                self.angle_speed = speed;
            }
        } else if dir < 0 {
            self.angle_min = angle;
            if self.angle > self.angle_min {
                self.angle_speed = -speed;
            }
        }
        dir
    }
}

/// A spinner: the vane that spins when the ball goes under it.
///
/// Port of `HitSpinner` and `SpinnerMoverObject` (`collideex.cpp:390`).
#[derive(Debug, Clone)]
pub struct Spinner {
    pub seg: LineSeg,
    pub height: f32,

    pub angle: f32,
    pub angle_speed: f32,
    /// If the two are equal, the vane spins free; otherwise it is limited.
    pub angle_min: f32,
    pub angle_max: f32,
    /// Already raised to `PHYS_FACTOR`.
    pub damping: f32,
    pub elasticity: f32,
    pub enabled: bool,
}

impl Spinner {
    pub fn new(pivot: PivotAxis, elasticity: f32) -> Self {
        let (min, max) = sort_limits(pivot.angle_min, pivot.angle_max);
        Self {
            seg: pivot_segment(&pivot),
            height: pivot.height,
            angle: 0.0f32.clamp(min, max),
            angle_speed: 0.0,
            angle_min: min,
            angle_max: max,
            damping: pivot.damping.powf(PHYS_FACTOR),
            elasticity,
            enabled: true,
        }
    }

    /// Tests both faces (`HitSpinner::HitTest`, `collideex.cpp:428`).
    ///
    /// Careful: the spinner sets `hit_flag` **the other way round** from the
    /// gate — the face that is the front one for one is the back one for the
    /// other.
    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled {
            return None;
        }
        if let Some(mut c) = self.seg.hit_test_through(ball, dtime) {
            c.hit_flag = true;
            return Some(c);
        }
        if let Some(mut c) = self.seg.flipped().hit_test_through(ball, dtime) {
            c.hit_flag = false;
            return Some(c);
        }
        None
    }

    /// The hit (`collideex.cpp:390`).
    ///
    /// Unlike the gate, the spinner **only counts from the front**: hit from
    /// behind it does not spin. Returns `true` if it spun.
    pub fn collide(&mut self, ball: &Ball, coll: &CollisionEvent) -> bool {
        let dot = coll.hit_normal.dot(ball.vel);
        if dot < 0.0 {
            return false; // from behind it does not count
        }

        let h = self.height * 0.5;
        let mut speed = dot.abs();
        if h.abs() > 1.0 {
            speed /= h;
        }
        speed *= self.damping;

        self.angle_speed = if coll.hit_flag { -speed } else { speed };
        true
    }

    /// Spins the vane (`SpinnerMoverObject::UpdateDisplacements`).
    pub fn update_displacements(&mut self, dtime: f32) {
        use std::f32::consts::TAU;

        self.angle += self.angle_speed * dtime;

        if self.angle_min != self.angle_max {
            // Limited vane: it bounces against its limits.
            if self.angle > self.angle_max {
                self.angle = self.angle_max;
                if self.angle_speed > 0.0 {
                    self.angle_speed *= -0.005 - self.elasticity;
                }
            }
            if self.angle < self.angle_min {
                self.angle = self.angle_min;
                if self.angle_speed < 0.0 {
                    self.angle_speed *= -0.005 - self.elasticity;
                }
            }
        } else {
            // Free vane: it goes all the way around and the angle wraps.
            while self.angle > TAU {
                self.angle -= TAU;
            }
            while self.angle < 0.0 {
                self.angle += TAU;
            }
        }
    }

    /// Brakes the vane and leaves it hanging downwards
    /// (`SpinnerMoverObject::UpdateVelocities`).
    ///
    /// Just as on the gate, the `sin(angle)` is gravity acting on the center
    /// of mass: it is what makes the vane end up still and vertical instead of
    /// stopping in any old position.
    pub fn update_velocities(&mut self) {
        self.angle_speed -= self.angle.sin() * (0.0025 * PHYS_FACTOR);
        self.angle_speed *= self.damping;
    }
}

/// The .vpx can bring the limits the wrong way round; the original sorts them
/// when building.
fn sort_limits(a: f32, b: f32) -> (f32, f32) {
    (a.min(b), a.max(b))
}

/// The axis of a gate or of a spinner, as a pass-through line.
///
/// It overshoots by `PHYS_SKIN` on each side so the ball does not slip through
/// the ends, and it is finite in z: one ball high, because what matters is
/// whether the ball goes **underneath** the vane and not whether it flies over
/// it.
fn pivot_segment(d: &PivotAxis) -> LineSeg {
    let (sn, cs) = d.rotation_deg.to_radians().sin_cos();
    let half = d.length * 0.5 + PHYS_SKIN;
    let tip = Vec2::new(cs * half, sn * half);
    LineSeg::new(
        d.center - tip,
        d.center + tip,
        d.z_low,
        d.z_low + 2.0 * PHYS_SKIN,
        Material::default(),
    )
}

/// A kicker: the hole that captures the ball.
///
/// Port of `KickerHitCircle::DoCollide` (`kicker.cpp:1111`).
///
/// With no script engine there is nobody to release it, so capturing a ball
/// leaves it there. That is the right thing: in the real game the table
/// releases it, not the physics.
#[derive(Debug, Clone)]
pub struct Kicker {
    pub circle: HitCircle,
    /// What relative height has to be reached for it to grab.
    pub hit_accuracy: f32,
    /// Whether it behaves the way kickers did before the height test existed.
    ///
    /// A legacy kicker grabs whatever touches it, however high the ball is
    /// riding (`kicker.cpp:1128`). Most tables' saucers are legacy — F-14's
    /// launch-lane saucer is — so leaving this out means a ball rolls into the
    /// hole, comes to a stop on top of it and stays there, because nothing
    /// catches it and nothing tells the script it arrived.
    pub legacy_mode: bool,
    /// The file's `FATH`: the hole has no bottom.
    ///
    /// `kicker.cpp:1148` reads it as `lockedInKicker = !fallThrough`, so a
    /// fall-through kicker takes the ball out of play instead of holding it —
    /// `kicker.cpp:1177` drops it to `zlow - radius - 5`, below the playfield,
    /// where the table's script is expected to destroy it from the `Hit`
    /// handler. Ignoring the flag turns a drain hole into a saucer: the ball is
    /// grabbed and parked in plain sight for ever, and the ROM, which is
    /// waiting for its drain switch to open again, never serves the next ball.
    pub fall_through: bool,
    pub enabled: bool,
    /// Which ball it has inside, if it has one.
    pub captured: Option<usize>,
    /// Which balls the hole counts as being inside it, one per bit.
    ///
    /// The original's `m_vpVolObjs`, which it keeps on the **ball** as the set
    /// of volumes that ball is in; asked from this side it is the same
    /// question. A `u64` is plenty, for the reason [`crate::trigger::Trigger`]
    /// gives: no table has sixty-four balls at once.
    ///
    /// It does two jobs, and both are the original's.
    ///
    /// A ball joins the set when the hole **takes** it (`kicker.cpp:1153`) and
    /// leaves it when it clears the circle, which is one step later than the
    /// coil fired. That pair is a switch as far as a ROM is concerned: a saucer
    /// wired `swNN_Hit` / `swNN_UnHit` whose `Unhit` never comes keeps the
    /// switch closed for ever, and the ROM re-energises the eject coil on a
    /// ball that left long ago.
    ///
    /// And a ball that is inside the circle while **not** in the set is one the
    /// hole has not dealt with yet, which `HitTestBasicRadius` reports as a hit
    /// at time zero (`collide.cpp:277-287`, "ball inside and no hit set"). That
    /// is not a corner case: it is how a hole gets a ball that was put down on
    /// top of it, and how it keeps working on one that is loitering in it.
    inside: u64,
    /// Which balls the collision cycle has already dealt with this step.
    ///
    /// Cleared at the end of every step. Without it the swept test and the
    /// end-of-step sweep would both act on the same ball in the same
    /// millisecond — see [`Kicker::contains`] for why there are two.
    touched: u64,
    /// The bowl, as `(position, normal)` per vertex: position placed on the
    /// table, normal in the mesh's own frame.
    ///
    /// See `builtin::kicker_hit_mesh`. It is what makes a kicker a funnel
    /// rather than a disc, and an empty one means "leave the ball alone",
    /// which is the legacy case and the original's `idx == ~0u`.
    pub hit_mesh: Vec<(Vec3, Vec3)>,
}

/// What a kicker did with a ball that reached it
/// (`KickerHitCircle::DoCollide`, `kicker.cpp:1111`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KickerHit {
    /// It rode over the top. The hole behaves as an ordinary edge and the
    /// caller still has to bounce the ball off it.
    Passed,
    /// The hole took it and is holding it.
    Captured,
    /// It went straight through the playfield: a `FallThrough` kicker.
    FellThrough,
}

impl Kicker {
    pub fn new(circle: HitCircle, hit_accuracy: f32, legacy_mode: bool) -> Self {
        Self {
            circle,
            hit_accuracy,
            legacy_mode,
            fall_through: false,
            enabled: true,
            captured: None,
            inside: 0,
            touched: 0,
            hit_mesh: Vec::new(),
        }
    }

    /// The `FATH` flag, as the builder reads it out of the file.
    pub fn with_fall_through(mut self, fall_through: bool) -> Self {
        self.fall_through = fall_through;
        self
    }

    /// The bowl the ball rolls on. See [`Kicker::hit_mesh`].
    pub fn with_hit_mesh(mut self, mesh: Vec<(Vec3, Vec3)>) -> Self {
        self.hit_mesh = mesh;
        self
    }

    pub fn hit_test(&self, ball: &Ball, dtime: f32) -> Option<CollisionEvent> {
        if !self.enabled || self.captured.is_some() {
            return None;
        }
        // A kicker is not a wall. See `HitCircle::hit_test_as_volume`.
        self.circle.hit_test_as_volume(ball, dtime)
    }

    /// Whether the ball's centre is inside the hole's cylinder.
    ///
    /// # Why a kicker is tested twice
    ///
    /// [`Kicker::hit_test`] is a swept crossing of the circle and this is a
    /// plain "is it in there now", and the engine runs both: the swept one
    /// inside the collision cycle, this one at the end of the step.
    ///
    /// The original needs both too, and for the same two reasons. The swept
    /// test is what catches a ball moving fast enough to cross the whole hole
    /// within one step — a drain kicker takes balls at thirty units per
    /// millisecond and a saucer is thirty units across, so a test that only
    /// looked at where the ball ended up would let one straight through. And
    /// the standing test is `collide.cpp:277-287`, the branch that fires a hit
    /// at time zero when the ball is inside the circle and not yet in the
    /// hole's volume set: without it a ball that is *put down* on a kicker, or
    /// that rolls in and stalls without ever crossing the rim while
    /// approaching, is never seen at all.
    ///
    /// That second case is not exotic. It is what a table does when it serves a
    /// ball into a saucer, and it is what a ball does when it drifts a few
    /// units off the middle of a hole and rolls back: the port's swept test
    /// discards a receding ball (see `HitCircle::hit_test_as_volume`, which
    /// keeps `direction` on and says why), so the rim is never crossed inbound
    /// and nothing ever fires.
    ///
    /// A ball that has fallen through ends up **below** `z_low`, so it leaves
    /// the volume by the same test that a kicked one does.
    pub fn contains(&self, ball: &Ball) -> bool {
        let d = ball.pos.truncate() - self.circle.center;
        ball.pos.z >= self.circle.z_low
            && ball.pos.z <= self.circle.z_high
            && d.length_squared() <= self.circle.radius * self.circle.radius
    }

    /// Whether the hole counts this ball as being inside it.
    pub fn has_ball(&self, ball_index: usize) -> bool {
        ball_index < 64 && self.inside & (1 << ball_index) != 0
    }

    /// Whether the ball has just left the hole's volume, which is the kicker's
    /// `Unhit` (`kicker.cpp:1189` — the only non-trigger `Unhit` in the whole
    /// of the original).
    pub fn check_exit(&mut self, ball_index: usize, ball: &Ball) -> bool {
        if !self.has_ball(ball_index) || self.contains(ball) {
            return false;
        }
        self.inside &= !(1u64 << ball_index);
        true
    }

    /// Marks the ball as dealt with for the rest of this step. See
    /// [`Kicker::touched`].
    pub fn mark_touched(&mut self, ball_index: usize) {
        if ball_index < 64 {
            self.touched |= 1u64 << ball_index;
        }
    }

    pub fn touched(&self, ball_index: usize) -> bool {
        ball_index < 64 && self.touched & (1 << ball_index) != 0
    }

    /// Forgets this step's bookkeeping, ready for the next one.
    pub fn end_step(&mut self) {
        self.touched = 0;
    }

    /// Keeps the balls it is watching in step with one being removed.
    pub fn renumber_after_removing(&mut self, ball_index: usize) {
        if ball_index >= 64 {
            return;
        }
        let squeeze = |mask: u64| {
            let below = mask & ((1u64 << ball_index) - 1);
            // Nothing can sit above the last slot, and shifting by 64 is not a
            // shift.
            let above = if ball_index == 63 {
                0
            } else {
                mask >> (ball_index + 1)
            };
            below | (above << ball_index)
        };
        self.inside = squeeze(self.inside);
        self.touched = squeeze(self.touched);
    }

    /// What height the ball has to be at for the kicker to grab it.
    pub fn grab_height(&self, ball: &Ball) -> f32 {
        (self.circle.z_low + ball.radius) * self.hit_accuracy
    }

    /// Steers the ball along the bowl (`DoChangeBallVelocity`,
    /// `kicker.cpp:1042`).
    ///
    /// This is what a kicker does to a ball that is over it but too high to be
    /// taken, and it is **not** a wall bounce. The port used to reflect the
    /// ball off the circle's normal, which points outwards in the plane: a hole
    /// built out of that is a post, and it flings the ball back out the way it
    /// came. The original instead finds the nearest vertex of the hit mesh,
    /// takes *that* vertex's normal — which on a shallow bowl tilts inwards and
    /// downwards — and applies `dot * hitnorm` with no elasticity at all, plus
    /// friction. The ball is pushed **into** the hole, loses a little speed to
    /// the rim, and comes round again lower until the height test takes it.
    ///
    /// It only became visible with the gravity fixed. Under-strength gravity
    /// left the ball too slow to escape even a flat disc, so a saucer worked by
    /// accident; at the right acceleration it arrives with enough energy to be
    /// spat straight back out, and a machine re-serves the same shot for ever.
    ///
    /// Two oddities are the original's and are kept:
    ///
    /// - `surfP` and the projection that builds `tangent` use the **circle's**
    ///   normal, the one passed in, while the impulse and the rest of the
    ///   tangent use the **mesh's**. Mixing the two looks like a slip, but it
    ///   is what ships and what every table was tuned against.
    /// - the friction coefficient is hard-coded to `0.3` and ignores the
    ///   kicker's material.
    fn change_ball_velocity(&self, ball: &mut Ball, hit_normal: Vec3) {
        // The nearest vertex of the bowl. An empty mesh is the original's
        // `idx == ~0u`: it changes nothing about the ball at all.
        let Some((_, hitnorm)) = self
            .hit_mesh
            .iter()
            .min_by(|a, b| {
                let da = (ball.pos - a.0).length_squared();
                let db = (ball.pos - b.0).length_squared();
                da.total_cmp(&db)
            })
            .copied()
        else {
            return;
        };

        let dot = -ball.vel.dot(hitnorm);
        let reaction_impulse = ball.mass * dot.abs();

        let surf_p = hit_normal * -ball.radius;
        let surf_vel = ball.surface_velocity(surf_p);
        // `tangent = surfVel - surfVel.Dot(hitnormal) * hitnorm` — the dot is
        // against the circle's normal and the vector is the mesh's. See above.
        let mut tangent = surf_vel - hitnorm * surf_vel.dot(hit_normal);

        // Along the normal, so it makes no torque. No elasticity term: the
        // bowl does not bounce the ball, it redirects it.
        ball.vel += hitnorm * dot;

        const FRICTION: f32 = 0.3;
        let tangent_sp_sq = tangent.length_squared();
        if tangent_sp_sq > 1e-6 {
            tangent /= tangent_sp_sq.sqrt();
            let vt = surf_vel.dot(tangent);

            let cross = surf_p.cross(tangent);
            let kt = 1.0 / ball.mass + tangent.dot((cross / ball.inertia()).cross(surf_p));

            // The Coulomb cone, same as everywhere else.
            let max_fric = FRICTION * reaction_impulse;
            let jt = (-vt / kt).clamp(-max_fric, max_fric);
            if jt.is_finite() {
                ball.apply_surface_impulse(cross * jt, tangent * jt);
            }
        }
    }

    /// Offers the ball to the hole (`kicker.cpp:1125-1184`).
    ///
    /// The ball has to be sunk far enough into it: if it comes in fast and
    /// high, it runs straight past over the edge. That is what keeps every ball
    /// that grazes a kicker from being captured.
    ///
    /// Whether the hole then *holds* it is a second question, and the answer is
    /// `!fall_through` (`kicker.cpp:1148`). Both answers stop the ball dead in
    /// the middle of the hole and clear its spin; they differ in where it is
    /// left and in whether it is frozen there.
    pub fn take_ball(&mut self, ball: &mut Ball, index: usize, hit_normal: Vec3) -> KickerHit {
        if self.captured.is_some() || !self.enabled {
            return KickerHit::Passed;
        }
        // `kicker.cpp:1128`. A legacy kicker does not ask how high the ball is.
        if !self.legacy_mode && ball.pos.z >= self.grab_height(ball) {
            // Over the hole and riding too high to be taken. It is **not** a
            // bounce: the ball rolls on across the bowl, and what steers it is
            // the bowl's own surface (`kicker.cpp:1133`).
            self.change_ball_velocity(ball, hit_normal);

            // `kicker.cpp:1134`, and the original calls it an ugly hack too: a
            // ball rolling over the lip of a kicker it is not going to fall
            // into loses nearly all its speed to the bevel and stops there.
            // Give it back what it had. It belongs to this branch and to no
            // other — it is the tail of the same `if`.
            if ball.vel.length_squared() < 0.2 * 0.2 {
                ball.vel = ball.old_vel;
            }
            ball.old_vel = ball.vel;
            return KickerHit::Passed;
        }

        ball.vel = Vec3::ZERO;
        ball.angular_momentum = Vec3::ZERO;
        ball.pos.x = self.circle.center.x;
        ball.pos.y = self.circle.center.y;

        if self.fall_through {
            // `kicker.cpp:1177`. Five units below the bottom of the hole, and
            // the original explains why it is not merely cosmetic: the height
            // test in `HitTestBasicRadius` is what stops the same hit being
            // reported again and again while the ball drops out of sight.
            //
            // The ball is **not** locked, and nothing here takes it off the
            // table: the script's `Hit` handler is what destroys it. The
            // original calls that handler from inside this function, before
            // moving the ball; here the event is queued and read a step later,
            // so a fall-through ball spends one millisecond under the
            // playfield. Nothing can reach it there and nothing draws it.
            ball.pos.z = self.circle.z_low - ball.radius - 5.0;
            return KickerHit::FellThrough;
        }

        ball.pos.z = self.circle.z_low + ball.radius;
        ball.locked = true;
        self.captured = Some(index);
        self.inside |= 1u64 << (index.min(63));
        KickerHit::Captured
    }

    /// Releases the ball with a given velocity.
    ///
    /// In the game this is fired by the script; here it is exposed so the
    /// engine can do it when one exists.
    pub fn release(&mut self, ball: &mut Ball, vel: Vec3) {
        if self.captured.is_none() {
            return;
        }
        ball.locked = false;
        ball.vel = vel;
        // A ball comes out of a saucer with the spin it went in with, and that
        // spin is nobody's business now: the original clears it on the way out
        // (`m_angularmomentum.SetZero()`, `kicker.cpp:738`). Leaving it means
        // the ball lands somewhere down the table still turning the way it was
        // when it arrived, and friction steers it off the line the kick aimed
        // it along.
        ball.angular_momentum = Vec3::ZERO;
        self.captured = None;
        // The volume set is deliberately left alone: the ball is on its way out
        // but it is still in the hole, and the `Unhit` that tells the ROM its
        // saucer switch has opened belongs to the moment it clears the circle,
        // not to the moment the coil fired.
    }

    /// Puts a ball in the hole without a collision, as `CreateBall` does.
    ///
    /// Same bookkeeping as a capture, so the ball the script served leaves
    /// through the same `Unhit` as one that rolled in.
    pub fn hold(&mut self, ball: &mut Ball, index: usize) {
        self.captured = Some(index);
        self.inside |= 1u64 << (index.min(63));
        ball.pos = Vec3::new(
            self.circle.center.x,
            self.circle.center.y,
            self.circle.z_low + ball.radius,
        );
        ball.vel = Vec3::ZERO;
        ball.angular_momentum = Vec3::ZERO;
        ball.locked = true;
    }
}
