//! A table being played: physics, script and the wire between them.
//!
//! Everything up to here could be described on its own. The physics simulates a
//! table without knowing what a game is; the renderer draws one without knowing
//! what it is for; `vpw-vbscript` runs a language without knowing what a
//! flipper is. This crate is where those become a game.
//!
//! # What the wire actually carries
//!
//! Three things, in both directions:
//!
//! - **Names to objects.** The script says `LeftFlipper`; [`items`] answers
//!   with something that reads and writes the real flipper in the engine.
//! - **Events to handlers.** The physics queues "ball 0 hit shape 1174"; this
//!   crate turns that into a call to `Bumper1_Hit`, with `ActiveBall` bound to
//!   the ball that did it.
//! - **Time to timers.** `X.TimerInterval = 100` means `X_Timer` runs ten
//!   times a second, counted in table time and not in frames.
//!
//! # Why it is its own crate and not part of the player
//!
//! Because none of it is about being in a browser, and putting it in
//! `vpw-player` — which is `wasm32`-only — would mean none of it could be
//! tested. The tests here load a published table and play it.
//!
//! # Where it departs from the original
//!
//! Visual Pinball's parts fire their events by calling into the script engine
//! directly from the collision code (`FireGroupEvent`), which is what ties its
//! physics to VBScript. Here the physics queues events and does not know who
//! listens, and this crate is the listener. The practical difference is that
//! the physics can be tested, and run, with no script at all — which is how
//! this port got a playable table before it had a script engine.

pub mod controller;
pub mod grouping;
pub mod items;
pub mod sound;
pub mod telemetry;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use vpw_math::Vec3;
use vpw_physics::ball::Ball;
use vpw_physics::constants::{DEFAULT_BALL_SIZE, DEFAULT_TABLE_GRAVITY_VPU, PHYSICS_STEPTIME_S};
use vpw_physics::engine::{Engine, Event, Shape};
use vpw_physics::trigger::Crossing;
use vpw_table::animation::AnimatedPart;
use vpw_table::controls::Controls;
use vpw_table::geometry::Scene;
use vpw_table::physics::Button;
use vpw_vbscript::error::Result as VbResult;
use vpw_vbscript::object::{Host, Object};
use vpw_vbscript::value::Value;
use vpw_vbscript::{Error as VbError, Interpreter};

use controller::{Controller, Machine, NoRoms, RomSource};
use items::{Item, Items};
use sound::{Request, Sounds};

/// How many milliseconds of table time one physics step is.
const STEP_MS: f64 = PHYSICS_STEPTIME_S * 1000.0;

/// The rate everything audible is mixed at.
///
/// It is the sound board's own output rate, so the one stream that cannot be
/// resampled cheaply — the machine's — needs no resampling at all. A host whose
/// output device wants a different rate should ask for this one and let the
/// platform convert; a browser does that for free.
pub const AUDIO_RATE: u32 = vpw_s11::DEFAULT_AUDIO_RATE;

/// How many recent `PlaySound` names to remember. A few seconds of a rolling
/// ball, which is enough to see what a table is doing and not enough to matter.
const SOUND_LOG: usize = 512;

/// Where a table's libraries come from.
///
/// A table does not carry its dependencies: its script opens with
/// `LoadVPM "…", "sam.VBS", …`, which asks the host for the *text* of another
/// script by name, at run time. So the set of libraries a table needs is not
/// knowable until it asks for them, and this is how it asks.
pub trait ScriptLibrary {
    /// The text of a library script, or `None` if there is no such file.
    fn read(&self, name: &str) -> Option<String>;
}

/// A library source that has nothing.
pub struct NoLibraries;

impl ScriptLibrary for NoLibraries {
    fn read(&self, _name: &str) -> Option<String> {
        None
    }
}

/// A library source backed by a directory on disk. Not available on wasm.
#[cfg(not(target_arch = "wasm32"))]
pub struct LibraryDir(pub std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl ScriptLibrary for LibraryDir {
    fn read(&self, name: &str) -> Option<String> {
        // Tables spell the name however they like — `core.vbs`, `Core.VBS` —
        // and the file on disk has one spelling.
        let wanted = name.to_ascii_lowercase();
        let entries = std::fs::read_dir(&self.0).ok()?;
        for e in entries.flatten() {
            let f = e.file_name();
            if f.to_string_lossy().to_ascii_lowercase() == wanted {
                let bytes = std::fs::read(e.path()).ok()?;
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        None
    }
}

/// Everything a table needs from outside the `.vpx` file.
///
/// A `.vpx` is not self-contained: its script asks the host for library scripts
/// by name, and if it is a ROM table the rules themselves live in a firmware
/// image that ships separately. Both are answered by asking somebody else for
/// bytes, and both differ between a browser and a test, so they travel together
/// rather than as two more arguments to [`Game::load`] that are easy to swap.
pub struct Resources {
    pub libraries: Rc<dyn ScriptLibrary>,
    pub roms: Rc<dyn RomSource>,
    /// The time of day, for the one thing that wants it. See [`Clock`].
    pub clock: Rc<dyn Clock>,
}

/// Where the time of day comes from.
///
/// Some tables draw a clock on the backglass and set its hands from the real
/// time. Nothing else in a pinball table asks: the clock a script measures
/// intervals against is the table's own, it starts at zero when the player
/// walks up, and it is a different thing entirely.
///
/// It is a seam rather than a call to the system because the machine this port
/// runs on is a browser tab, which has no system clock to call.
pub trait Clock {
    /// Milliseconds since the Unix epoch.
    fn now_millis(&self) -> f64;
}

/// A host with no clock at all. Every date reads as the day VBScript counts
/// from, which is a Saturday in 1899, and a backglass clock drawn from it does
/// not move.
pub struct NoClock;

impl Clock for NoClock {
    fn now_millis(&self) -> f64 {
        0.0
    }
}

/// The machine's own clock, where there is a machine to ask.
///
/// Not on wasm, where there is no `SystemTime` and the page hands one in
/// through [`Resources::with_clock`] instead.
pub struct SystemClock;

impl Clock for SystemClock {
    #[cfg(not(target_arch = "wasm32"))]
    fn now_millis(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_millis() as f64)
    }

    #[cfg(target_arch = "wasm32")]
    fn now_millis(&self) -> f64 {
        0.0
    }
}

impl Default for Resources {
    /// Nothing available: the table runs on its own script and without a ROM.
    fn default() -> Self {
        Self {
            libraries: Rc::new(NoLibraries),
            roms: Rc::new(NoRoms),
            clock: Rc::new(SystemClock),
        }
    }
}

impl Resources {
    /// Libraries, and no ROMs.
    pub fn new(libraries: Rc<dyn ScriptLibrary>) -> Self {
        Self {
            libraries,
            ..Self::default()
        }
    }

    pub fn with_roms(mut self, roms: Rc<dyn RomSource>) -> Self {
        self.roms = roms;
        self
    }

    pub fn with_clock(mut self, clock: Rc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
}

/// The events a table's script can be told about.
///
/// The names are the suffixes VBScript expects: a handler for a bumper called
/// `Bumper1` is `Sub Bumper1_Hit`. Measured over four published tables and
/// `core.vbs`, `_Hit` accounts for two thirds of every handler written.
const HIT: &str = "Hit";
const UNHIT: &str = "Unhit";
const TIMER: &str = "Timer";
/// Fired once a frame at everything that moves, so a table can drag whatever it
/// has drawn on top of a part into line with it. See [`Game::animate`].
const ANIMATE: &str = "Animate";
const SPIN: &str = "Spin";
const SLINGSHOT: &str = "Slingshot";
/// A drop target reaching the bottom of its travel, and the top again.
/// `DISPID_TargetEvents_Dropped` / `_Raised`, `hittarget.cpp:614` and `:626`.
const DROPPED: &str = "Dropped";
const RAISED: &str = "Raised";

/// Where a millisecond of table time actually goes.
///
/// Off the wasm build entirely: `Instant` is not implemented there, and a
/// profiler that panics in the browser is worse than none. On a desktop it is
/// four clock reads per tick, which is far below the noise it is measuring.
///
/// It exists because the honest answer to "why is it slow" is almost never the
/// one you would guess. The table here swings between three and thirteen times
/// real time with the same number of script handlers running in each window,
/// and no amount of reading the code says which of the four phases is the one
/// that changed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default, Clone, Copy, Debug)]
pub struct Profile {
    /// Nanoseconds in the physics engine.
    pub physics_ns: u64,
    /// Nanoseconds running the ROM and taking its audio.
    pub board_ns: u64,
    /// Nanoseconds dispatching collision events into the script.
    pub events_ns: u64,
    /// Nanoseconds in the script's timers.
    pub timers_ns: u64,
    /// How many ticks these totals cover.
    pub ticks: u64,
    /// How many timers the scan looked at, summed over every tick. This is the
    /// cost of *looking*, and it is paid whether or not anything is due.
    pub timers_scanned: u64,
    /// How many `_Timer` handlers actually ran. The cost of *doing*.
    pub timers_fired: u64,
}

/// A table, loaded and playable.
pub struct Game {
    /// The simulation. Shared because the script's objects write into it.
    pub engine: Rc<RefCell<Engine>>,
    pub controls: Controls,
    parts: Vec<AnimatedPart>,
    script: Interpreter,
    state: Rc<GameState>,
    /// What the script calls the table object, from the file. See
    /// [`Self::handler`].
    table_object: String,
    ball_radius: f32,
    /// Table time since the table was loaded, in milliseconds.
    clock_ms: f64,
    /// How many script handlers have been called. A cell because the events
    /// are dispatched through `&self`.
    fired: Cell<u64>,
    /// Which parts have an `_Animate` handler, worked out once. See
    /// [`Game::animate`].
    animated: RefCell<Option<Vec<Rc<str>>>>,
    /// Set once the script has been told the table is starting.
    started: bool,
    /// What each kicker was holding last time the record looked, so a take or
    /// a release is an edge rather than a state repeated every sample.
    held_by_kicker: RefCell<HashMap<usize, bool>>,
    /// Where the time goes, for a host that asks. See [`Profile`].
    #[cfg(not(target_arch = "wasm32"))]
    profile: Profile,
}

/// The part of the game the script's objects reach.
///
/// It is behind an `Rc` because the host, every game item and the `Table1`
/// object all need it, and they outlive any single call.
pub struct GameState {
    pub engine: Rc<RefCell<Engine>>,
    pub items: Items,
    /// The ball that caused the event being handled, which is what the script
    /// calls `ActiveBall`.
    active_ball: Cell<Option<usize>>,
    /// The part whose handler is running, which is what the script calls `Me`.
    ///
    /// Inside `Bumper1_Hit`, `Me` is `Bumper1` — not the table. `core.vbs`
    /// depends on it in a place that is easy to miss:
    /// `Sub PulseTimer_Init : vpmTimer.InitTimer Me, False : End Sub` hands its
    /// own timer part to the library so the library can drive itself from it.
    /// Answer the table instead and the library's clock never starts, which
    /// means no switch a script pulses ever reaches the ROM — no coin, no
    /// start button, no game.
    current_item: RefCell<Option<Rc<Item>>>,
    /// The names of sounds the script asked for, most recent last. It answers
    /// a different question from the mixer's: what the table *tried* to play,
    /// which is what a test asserts on.
    ///
    /// Bounded, because a rolling sound is re-asked for on every tick of a
    /// 10 ms timer — a hundred a second, for as long as the ball is moving.
    /// A host that never drains it would otherwise pay for the whole session.
    pub sounds: RefCell<Vec<String>>,
    /// The table's sound bank and the mixer playing it.
    pub audio: RefCell<Sounds>,
    pub messages: RefCell<Vec<String>>,
    /// Table time, for `Timer`.
    clock_ms: Cell<f64>,
    /// Cached objects, so `LeftFlipper` costs a hash lookup once.
    resolved: RefCell<HashMap<Box<str>, Value>>,
    /// The playfield's width and height in table units, which a script reads
    /// off `Table1` and divides by.
    size: (f32, f32),
    /// The table's collections, by lowercased name.
    collections: HashMap<Box<str>, Vec<Rc<Item>>>,
    /// Where `GetTextFile` gets its text.
    libraries: Rc<dyn ScriptLibrary>,
    /// The emulated ROM board. Idle until the script says `Controller.Run`.
    pub machine: Rc<Machine>,
    /// Where the ROM images come from.
    roms: Rc<dyn RomSource>,
    /// Where the time of day comes from.
    clock: Rc<dyn Clock>,
    /// The rolling record of what the machine did. Off until a host turns it
    /// on; see [`telemetry`].
    pub telemetry: RefCell<telemetry::Telemetry>,
    /// The one `Controller` the script gets.
    ///
    /// `CreateObject("VPinMAME.Controller")` appears in more than one place —
    /// the table's own script and `vpmcore` both do it — and they have to reach
    /// the same board. More importantly they have to share the *record of what
    /// the script has already seen*, or two pollers would each get half of the
    /// lamp changes and half the playfield would stay dark.
    controller: RefCell<Option<Rc<Controller>>>,
    /// What the table has saved with `SaveValue`: its high scores, its
    /// options, whatever it wants to find again next time. Keyed by the
    /// table's name and the value's, both folded. See [`LoadValue`].
    values: RefCell<HashMap<String, String>>,
    /// Materials the script merely repainted with `MaterialColor`, by folded
    /// name. Kept apart from the wholesale rewrites because a repaint has to
    /// leave the gloss and the thickness where the table's author put them.
    pub tints: RefCell<HashMap<String, [f32; 3]>>,
    /// Materials the script rewrote with `UpdateMaterial`, by folded name.
    materials: RefCell<HashMap<String, MaterialChange>>,
    /// Every material as the table's author left it, by folded name. What
    /// `GetMaterial` answers with, once the script's own changes are laid over
    /// it. Taken at load, before a line of script has run, because that is the
    /// only moment it is still true.
    authored: HashMap<String, Authored>,
}

impl GameState {
    /// The `Controller`, built on first use.
    fn controller(&self) -> Rc<Controller> {
        self.controller
            .borrow_mut()
            .get_or_insert_with(|| {
                Rc::new(Controller::new(self.machine.clone(), self.roms.clone()))
            })
            .clone()
    }
}

impl Game {
    /// Loads a table: physics, moving parts, script and all.
    ///
    /// The scene is modified — the moving parts are taken out of it, because
    /// they are drawn separately — so it has to be the same scene that is then
    /// uploaded to the renderer.
    pub fn load(
        vpx: &vpin::vpx::VPX,
        scene: &mut Scene,
        resources: Resources,
    ) -> Result<Self, VbError> {
        let g = &vpx.gamedata;
        // Before anything: what every material *was*. `apply_material_changes`
        // writes into this same scene later, so read now or not at all.
        let authored: HashMap<String, Authored> = scene
            .materials
            .iter()
            .map(|m| {
                let (elasticity, elasticity_falloff, friction, scatter_angle) =
                    vpw_table::physics::material_physics(vpx, &m.name).unwrap_or_default();
                (
                    m.name.to_lowercase(),
                    Authored {
                        look: MaterialChange {
                            wrap_lighting: m.wrap_lighting,
                            roughness: m.roughness,
                            glossy_image_lerp: m.glossy_image_lerp,
                            thickness: m.thickness,
                            edge: m.edge,
                            edge_alpha: m.edge_alpha,
                            opacity: m.opacity,
                            base_color: m.base_color,
                            glossy_color: m.glossy_color,
                            clearcoat_color: m.clearcoat_color,
                            is_metal: m.is_metal,
                            opacity_active: m.opacity_active,
                        },
                        elasticity,
                        elasticity_falloff,
                        friction,
                        scatter_angle,
                    },
                )
            })
            .collect();
        let collision = vpw_table::physics::build_with_owners(vpx);

        // The table's own slope and its own gravity, not the maximum and a
        // constant. See `vpw_table::geometry::TablePhysics`: a table gives a
        // range of slopes and its difficulty picks a point in it
        // (`player.cpp:498`), and the gravity in the file is already an
        // acceleration — the 0.97 that used to be passed here is a *multiple*
        // of Earth's, so passing it bare made every table 1.82 times too
        // floaty.
        let table = scene.physics;
        // The file's own `GRAV`, which is already an acceleration:
        // `m_Gravity = 0.97f * GRAVITYCONST` (`pintable.cpp:3833`) is what gets
        // stored, so it is the product and not the 0.97. Passing the bare
        // multiple, which is what this used to do, ran every table at 1/1.82 of
        // its weight — a ball dropped one diameter took 101 ms instead of 75,
        // and nothing about it looked broken, it just played floaty.
        //
        // The fallback is the same product, for a file that stores zero.
        //
        // This line waited on `DoChangeBallVelocity` (`kicker.cpp:1042`), and
        // that is worth remembering rather than deleting: weak gravity was
        // holding a saucer together. The port used to bounce an uncaptured ball
        // off the kicker's circle as if it were a wall, and at the right
        // acceleration the ball arrives with enough energy to be spat back out
        // of one. It only worked because the ball was too slow to escape.
        let strength = if table.gravity > 0.0 {
            table.gravity
        } else {
            DEFAULT_TABLE_GRAVITY_VPU
        };
        let mut engine = Engine::new(
            collision.shapes.clone(),
            Engine::gravity_from_slope(table.slope_deg, strength),
        );
        engine.slope_rad = table.slope_deg.to_radians();
        for trigger in vpw_table::physics::triggers(vpx) {
            engine.add_trigger(trigger);
        }

        let controls = Controls::new(vpx, &engine);
        let parts = vpw_table::animation::animated_parts(vpx, &engine);
        scene.remove(&vpw_table::animation::names(&parts));

        let engine = Rc::new(RefCell::new(engine));
        let items = Items::new(vpx, &collision, engine.clone());
        let state = Rc::new(GameState {
            tints: RefCell::new(HashMap::new()),
            engine: engine.clone(),
            items,
            active_ball: Cell::new(None),
            current_item: RefCell::new(None),
            sounds: RefCell::new(Vec::new()),
            telemetry: RefCell::new(telemetry::Telemetry::new()),
            audio: RefCell::new(Sounds::new(vpw_table::sound::extract(vpx), AUDIO_RATE)),
            messages: RefCell::new(Vec::new()),
            clock_ms: Cell::new(0.0),
            resolved: RefCell::new(HashMap::new()),
            size: ((g.right - g.left).max(1.0), (g.bottom - g.top).max(1.0)),
            collections: HashMap::new(),
            libraries: resources.libraries,
            machine: Rc::new(Machine::new()),
            roms: resources.roms,
            clock: resources.clock,
            controller: RefCell::new(None),
            values: RefCell::new(HashMap::new()),
            materials: RefCell::new(HashMap::new()),
            authored,
        });

        // Filled after the state exists, because a collection holds the same
        // `Item`s the state does rather than copies of them.
        let collections = state.items.collections(vpx);
        let mut state = state;
        Rc::get_mut(&mut state)
            .expect("nothing else holds the state yet")
            .collections = collections;

        let script = Interpreter::new(Rc::new(TableHost {
            state: state.clone(),
        }));

        let mut game = Self {
            animated: RefCell::new(None),
            engine,
            controls,
            parts,
            script,
            state,
            table_object: table_object_name(&vpx.gamedata.name),
            ball_radius: DEFAULT_BALL_SIZE,
            clock_ms: 0.0,
            fired: Cell::new(0),
            started: false,
            held_by_kicker: RefCell::new(HashMap::new()),
            #[cfg(not(target_arch = "wasm32"))]
            profile: Profile::default(),
        };
        game.load_script(&vpx.gamedata.code.string)?;
        game.arm_hit_reporting();
        game.apply_visibility_changes(scene);
        Ok(game)
    }

    /// Puts what the script has done to `Visible` into the static scene.
    ///
    /// A table's first lines are very often a fork on `Table.ShowDT` — one set
    /// of parts for a desktop and another for a cabinet — and the set that
    /// does not belong is switched off by name. The Sopranos does exactly
    /// that, and without this its cabinet walls stand in the middle of the
    /// playfield as a black slab: the file stores them visible, because the
    /// file is what the script edits.
    ///
    /// It goes the other way too. Circus stores its desktop rails as two
    /// ramps the file *hides*, and its `Init` shows them — so the whole
    /// bottom of the table was missing. That is why the builders keep a
    /// hidden part's mesh rather than dropping it: the file's flag is where
    /// the part starts, not where it ends up. And it is why this runs twice,
    /// which is the original's order: once as the script loads, and again
    /// after `Init` (`player.cpp:739`), which is when the original takes its
    /// static picture and reads each part's live flag (`ramp.cpp:862`).
    ///
    /// The moving parts are asked every frame ([`Self::part_visible`])
    /// because the script changes them *while playing* — hiding the built-in
    /// flipper bat under a primitive one is the usual reason. These are the
    /// parts that never move, whose meshes are baked into one buffer when the
    /// scene is uploaded. A table that hides one of those mid-game keeps it;
    /// the original rebuilds its static picture then, and this port does not
    /// yet.
    pub fn apply_visibility_changes(&self, scene: &mut Scene) {
        let mut changed = 0;
        for mesh in &mut scene.meshes {
            let Some(item) = self.state.items.get(owner_of(&mesh.name)) else {
                continue;
            };
            // A wall's side follows its own flag. Everything else — a wall's
            // top included — follows `Visible`.
            let shown = if mesh.name.ends_with(" (side)") {
                item.side_visible()
            } else {
                item.visible()
            };
            if mesh.visible != shown {
                mesh.visible = shown;
                changed += 1;
            }
        }
        if changed > 0 {
            log::debug!(
                "the script changed what {changed} static meshes show before the table was drawn"
            );
        }
    }

    /// Loads the table's script, pulling in whatever libraries it asks for.
    ///
    /// A table's first lines are `LoadVPM` or `LoadScript`, which in Visual
    /// Pinball read a file off disk. We cannot let the script do that, so the
    /// libraries it names are resolved here and loaded before it runs.
    fn load_script(&mut self, code: &str) -> Result<(), VbError> {
        for name in BOOTSTRAP {
            let Some(text) = self.state.libraries.read(name) else {
                log::warn!("'{name}' is not available; the table may not start");
                continue;
            };
            if let Err(e) = self.script.load(&text) {
                // A library that fails is worth reporting but not worth
                // refusing to start over: several are optional, and a table
                // that cannot find its DMD library still plays.
                log::warn!("library '{name}': {}", e.blame(name, &text));
            }
        }
        let _ = code;
        // And the table's own, named and quoted the same way. An error that
        // came out of a library was already blamed on its way through
        // `ExecuteGlobal` and has no line left, so this cannot take the credit
        // for somebody else's line 1972.
        self.script
            .load(code)
            .map_err(|e| e.blame("in the table's own script", code))
    }

    /// Marks the shapes whose hits the script actually wants.
    ///
    /// A table has three thousand shapes and a ball touches something every few
    /// milliseconds. Reporting all of it would queue hundreds of events a
    /// second for a listener that throws nearly all of them away, so only the
    /// parts with a handler are armed — which is a property of the script, and
    /// therefore something only this crate can work out.
    fn arm_hit_reporting(&mut self) {
        let mut engine = self.engine.borrow_mut();
        for item in self.state.items.iter() {
            let has = |e: &str| self.script.has_proc(&format!("{}_{e}", item.name));
            // Two conditions, not one, and the second is the file's. A part
            // reports only if it is *marked* to (`m_fe = m_d.m_hitEvent`,
            // `ramp.cpp:818`, `surface.cpp:380`); deciding from the script
            // alone made every decorative post with a handler-shaped name into
            // a live switch. Parts with no such flag — gate, spinner, kicker —
            // answer `true` and are unaffected.
            let reports_hits = item.has_hit_event() && [HIT, UNHIT, SPIN].iter().any(|e| has(e));
            // A slingshot is the exception the original spells out: "slingshots
            // always have hit events" (`surface.cpp:AddLine`), where `m_fe` is
            // forced on for that one segment whatever the wall's own flag says.
            let reports_slingshot = has(SLINGSHOT);
            if !reports_hits && !reports_slingshot {
                continue;
            }
            for &s in &item.shapes {
                engine.report_hits(s, true);
            }
        }
    }

    /// The moving parts, to hand to the renderer.
    pub fn parts(&self) -> &[AnimatedPart] {
        &self.parts
    }

    /// Where the `i`th moving part is now.
    ///
    /// A host should use this rather than [`AnimatedPart::transform`]: most
    /// pieces are placed by the physics and that is all `transform` knows, but
    /// a primitive is placed by the **script**, from the nine numbers it writes
    /// on the part. On F-14 the flipper you see is one of those — the script
    /// hides the flipper part and turns a primitive bat to match it — so a host
    /// that skips this draws flippers that never move.
    pub fn part_transform(&self, i: usize) -> vpw_math::Mat4 {
        let Some(part) = self.parts.get(i) else {
            return vpw_math::Mat4::IDENTITY;
        };
        match part.anim {
            vpw_table::animation::Animation::Primitive { index } => self
                .state
                .items
                .by_index(index)
                .map(|item| item.placement_matrix())
                .unwrap_or(part.mesh.transform),
            _ => part.transform(&self.engine.borrow()),
        }
    }

    /// Whether the `i`th moving part should be drawn at all.
    ///
    /// The script hides and shows parts as it likes, and on a table with
    /// primitive flippers the hiding is the point: `LeftFlipper.visible = 0`
    /// is what stops the built-in bat from poking out from under the one you
    /// are meant to see.
    pub fn part_visible(&self, i: usize) -> bool {
        let Some(part) = self.parts.get(i) else {
            return false;
        };
        match self.state.items.get(&part.mesh.name) {
            Some(item) => item.visible(),
            None => true,
        }
    }

    pub fn items(&self) -> &Items {
        &self.state.items
    }

    /// Everything the table has saved with `SaveValue`, as a flat list of
    /// key and value.
    ///
    /// For a host that wants to keep it between sessions, which is the whole
    /// point of the call: a table saves its high scores and its options this
    /// way and looks for them again next time. See [`LoadValue`].
    pub fn table_values(&self) -> Vec<String> {
        let mut out = Vec::new();
        let values = self.state.values.borrow();
        // Sorted, so two saves of the same state are the same bytes and the
        // page can tell whether anything changed.
        let mut keys: Vec<&String> = values.keys().collect();
        keys.sort();
        for k in keys {
            out.push(k.clone());
            out.push(values[k].clone());
        }
        out
    }

    /// Puts a previous session's values back. Before `start`, because a
    /// table's `Init` is where it reads them.
    pub fn restore_table_values(&mut self, pairs: &[String]) {
        let mut values = self.state.values.borrow_mut();
        for pair in pairs.as_chunks::<2>().0 {
            values.insert(pair[0].clone(), pair[1].clone());
        }
    }

    /// Puts the script's `UpdateMaterial` calls into the scene.
    ///
    /// Called between the script running and the scene being uploaded, which
    /// is the window where a material can still be changed: after the upload
    /// each one is a bind group on the GPU and a change would have to go back
    /// up. Almost every call is in that window — a table tunes its materials
    /// while setting itself up — and one made later is recorded and takes
    /// effect nowhere, which is the honest half-measure and better than the
    /// script stopping at the call.
    pub fn apply_material_changes(&self, scene: &mut Scene) {
        let changes = self.state.materials.borrow();
        let tints = self.state.tints.borrow();
        if changes.is_empty() && tints.is_empty() {
            return;
        }
        // The repaints go on first, so a wholesale rewrite of the same
        // material still wins: that is the order the script made the calls in
        // for every table that does both.
        for m in &mut scene.materials {
            if let Some(colour) = tints.get(&m.name.to_lowercase()) {
                m.base_color = *colour;
            }
        }
        for m in &mut scene.materials {
            let Some(c) = changes.get(&m.name.to_lowercase()) else {
                continue;
            };
            m.wrap_lighting = c.wrap_lighting;
            m.roughness = c.roughness;
            m.glossy_image_lerp = c.glossy_image_lerp;
            m.thickness = c.thickness;
            m.edge = c.edge;
            m.edge_alpha = c.edge_alpha;
            m.opacity = c.opacity;
            m.base_color = c.base_color;
            m.glossy_color = c.glossy_color;
            m.clearcoat_color = c.clearcoat_color;
            m.is_metal = c.is_metal;
            m.opacity_active = c.opacity_active;
        }
    }

    /// The emulated ROM board, for a host that wants to draw the score.
    pub fn machine(&self) -> &Rc<Machine> {
        &self.state.machine
    }

    /// Tells the script the table is starting. Runs the table's own `_Init`.
    pub fn start(&mut self) -> Result<(), VbError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        if let Some(name) = self.handler("Init") {
            self.script.call(&name, &[])?;
        }

        // And then every part's own `_Init`. Visual Pinball calls these on each
        // object as the table starts (`pinball.cpp`, `Player::Init`), and
        // `core.vbs` depends on it: `PinMAMETimer_Init` is what turns on the
        // timer that polls the ROM, so without this pass a ROM table loads,
        // starts its machine, and then never looks at it again.
        //
        // A failing one is logged rather than fatal, the same way a failing
        // library is: one part that cannot initialise should not cost the
        // player the table.
        let names: Vec<String> = self
            .state
            .items
            .iter()
            .map(|i| format!("{}_Init", i.name))
            .collect();
        for name in names {
            if !self.script.has_proc(&name) {
                continue;
            }
            let part = name.trim_end_matches("_Init").to_string();
            if let Err(e) = self.call_as(&name, self.state.items.get(&part)) {
                log::warn!("{name}: {e}");
            }
        }
        Ok(())
    }

    /// Advances one millisecond of table time.
    pub fn step(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        let mut mark = std::time::Instant::now();
        self.engine.borrow_mut().step();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.physics_ns += mark.elapsed().as_nanos() as u64;
            mark = std::time::Instant::now();
        }
        self.clock_ms += STEP_MS;
        self.state.clock_ms.set(self.clock_ms);

        // The ROM runs on the table's clock, not on its own thread. It is a
        // no-op until the script says `Controller.Run`, and on a table with no
        // ROM it stays one forever.
        self.state.machine.advance(STEP_MS / 1000.0);
        // What the board made while it ran goes straight into the mix. Draining
        // it here and not in `render_audio` keeps the two clocks separate: the
        // board's output belongs to table time, and how often a host asks for
        // audio is its own business.
        let board = self.state.machine.take_audio();
        if !board.is_empty() {
            self.state.audio.borrow_mut().mixer.push_board(&board);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.board_ns += mark.elapsed().as_nanos() as u64;
            mark = std::time::Instant::now();
        }

        self.dispatch_events();
        // The targets' slide, before the announcements: a target that reaches
        // the bottom of its travel on this step leaves its `Dropped` in the
        // queue and it goes out in the same step rather than the next.
        self.animate_targets();
        self.announce_targets();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.events_ns += mark.elapsed().as_nanos() as u64;
            mark = std::time::Instant::now();
        }
        self.run_timers();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.timers_ns += mark.elapsed().as_nanos() as u64;
            self.profile.ticks += 1;
        }
        self.record();
    }

    /// How far the shooter rod is drawn back, from 0 to 1, if there is one.
    ///
    /// For a host that draws its own plunger. What decides a shot is how long
    /// the button is held and not how far a finger travelled — the physics
    /// draws the rod back on its own for as long as the key is down — so a
    /// control that animates from the finger is showing the player something
    /// that is not what the table is about to do with it.
    ///
    /// Zero is where the rod parks, not where the frame ends: see
    /// [`vpw_physics::plunger::Plunger::travel`].
    pub fn plunger_pull(&self) -> Option<f32> {
        let i = self.controls.plunger()?;
        let engine = self.engine.borrow();
        match engine.shapes().get(i)? {
            Shape::Plunger(p) => Some(p.travel()),
            _ => None,
        }
    }

    /// Holds the shooter rod where the player is holding it, 0 to 1.
    ///
    /// The counterpart of the plunger *key*, and the honest one for a finger on
    /// a screen: a key has no position to offer so it draws the rod back for as
    /// long as it is held, while a finger says exactly how far back the player
    /// means to pull. See [`vpw_physics::plunger::Plunger::hold_at`].
    pub fn hold_plunger(&mut self, travel: f32) {
        if let Some(i) = self.controls.plunger() {
            self.engine.borrow_mut().hold_plunger(i, travel);
        }
    }

    /// Lets go of a rod that was being held. The shot comes from where it is.
    pub fn let_go_of_plunger(&mut self) {
        if let Some(i) = self.controls.plunger() {
            self.engine.borrow_mut().let_go_of_plunger(i);
        }
    }

    /// Every timer the script has armed, with its interval in milliseconds.
    ///
    /// A table that has quietly left a hundred parts polling every tick looks
    /// exactly like a slow port from the outside, and this is the difference.
    pub fn armed_timers(&self) -> Vec<(Rc<str>, f64)> {
        self.state
            .items
            .iter()
            .filter(|i| i.timer.enabled.get())
            .map(|i| (i.name.clone(), i.timer.interval.get()))
            .collect()
    }

    /// Where the time has gone since this was last called, and resets it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_profile(&mut self) -> Profile {
        std::mem::take(&mut self.profile)
    }

    /// Adds this millisecond to the rolling record, if one is being kept.
    ///
    /// The edges go in every tick and the continuous track only every
    /// [`telemetry::SAMPLE_MS`]; see that module for why the two are separate.
    fn record(&self) {
        let mut tel = self.state.telemetry.borrow_mut();
        if !tel.is_enabled() {
            return;
        }
        let now = self.clock_ms;
        tel.edges(
            now,
            self.state.machine.solenoids_active(),
            self.state.machine.switch_matrix(),
        );
        let ball_count = self.engine.borrow().balls.len();
        tel.balls(now, ball_count.min(255) as u8);
        if !tel.wants_sample(now) {
            return;
        }

        // Which kicker holds what, by name. A saucer that will not give a ball
        // back is the case this exists for, and "captured" is the fact that
        // separates a held ball from one merely sitting in the bowl.
        let changes: Vec<(Rc<str>, bool)> = {
            let engine = self.engine.borrow();
            engine
                .shapes()
                .iter()
                .enumerate()
                .filter_map(|(i, shape)| {
                    let Shape::Kicker(k) = shape else { return None };
                    let item = self.state.items.by_shape(i)?;
                    Some((i, item.name.clone(), k.captured.is_some()))
                })
                .filter(|(i, _, holding)| self.held_by_kicker.borrow().get(i) != Some(holding))
                .map(|(i, name, holding)| {
                    self.held_by_kicker.borrow_mut().insert(i, holding);
                    (name, holding)
                })
                .collect()
        };
        for (name, holding) in changes {
            tel.event(
                now,
                telemetry::Event::Kicker {
                    name: name.as_ref().into(),
                    holding,
                },
            );
        }

        let engine = self.engine.borrow();
        let mut balls = [telemetry::BallSample::default(); telemetry::MAX_BALLS];
        let count = engine.balls.len().min(telemetry::MAX_BALLS);
        for (slot, ball) in balls.iter_mut().zip(engine.balls.iter()) {
            *slot = telemetry::BallSample {
                x: ball.pos.x,
                y: ball.pos.y,
                z: ball.pos.z,
                vx: ball.vel.x,
                vy: ball.vel.y,
                vz: ball.vel.z,
            };
        }
        let angle = |button| {
            self.controls
                .flippers(button)
                .first()
                .and_then(|&i| match engine.shapes().get(i) {
                    Some(Shape::Flipper(f)) => Some(f.angle_cur.to_degrees()),
                    _ => None,
                })
                .unwrap_or(f32::NAN)
        };
        let sample = telemetry::Sample {
            t_ms: now.max(0.0) as u32,
            balls,
            ball_count: count as u8,
            locked: engine
                .balls
                .iter()
                .take(8)
                .enumerate()
                .fold(0u8, |bits, (n, b)| bits | (u8::from(b.locked) << n)),
            flipper_left: angle(Button::Left),
            flipper_right: angle(Button::Right),
            solenoids: self.state.machine.solenoids_active(),
            switches: self.state.machine.switch_matrix(),
        };
        drop(engine);
        tel.sample(sample);
    }

    /// Starts or stops keeping a rolling record. See [`telemetry`].
    pub fn set_telemetry(&self, on: bool) {
        self.state.telemetry.borrow_mut().set_enabled(on);
    }

    pub fn telemetry_enabled(&self) -> bool {
        self.state.telemetry.borrow().is_enabled()
    }

    /// Marks this instant in the record, with whatever the host wants to say
    /// about it.
    pub fn telemetry_mark(&self, note: &str) {
        self.state
            .telemetry
            .borrow_mut()
            .event(self.clock_ms, telemetry::Event::Mark(note.into()));
    }

    /// Records a frame that took too long, with where its time went.
    ///
    /// Called by the host, because the host is the only one that knows: the
    /// game is stepped from a loop it does not own, and everything it does see
    /// —the physics, the script, the sound boards— has been measured and comes
    /// nowhere near a frame. What is left is on the other side of this call.
    ///
    /// Only the slow ones are kept, and the threshold is the host's to choose:
    /// a record of every frame would be a hundred thousand entries in a
    /// half-hour window and would bury the events worth reading.
    #[allow(clippy::too_many_arguments)]
    pub fn telemetry_frame(
        &self,
        total_ms: f32,
        steps: u32,
        step_ms: f32,
        sync_ms: f32,
        render_ms: f32,
        late_ms: f32,
        idle_ms: f32,
    ) {
        self.state.telemetry.borrow_mut().event(
            self.clock_ms,
            telemetry::Event::Frame {
                total_ms,
                steps,
                step_ms,
                sync_ms,
                render_ms,
                late_ms,
                idle_ms,
            },
        );
    }

    /// Records something outside the frame that held the main thread.
    ///
    /// The page pumps its audio from an animation frame of its own, so that
    /// work never passes through [`Game::step`] and cannot be timed from in
    /// here. See [`telemetry::Event::Pause`].
    pub fn telemetry_pause(&self, source: &str, ms: f32) {
        self.state.telemetry.borrow_mut().event(
            self.clock_ms,
            telemetry::Event::Pause {
                source: source.into(),
                ms,
            },
        );
    }

    /// The last `seconds` of the record, as JSON.
    pub fn telemetry_dump(&self, seconds: f64, note: &str) -> String {
        self.state.telemetry.borrow().dump(seconds, note)
    }

    /// How much is being held: samples and edges.
    pub fn telemetry_held(&self) -> (usize, usize) {
        let tel = self.state.telemetry.borrow();
        (tel.sample_count(), tel.event_count())
    }

    /// Turns what the physics queued into calls into the script.
    fn dispatch_events(&mut self) {
        let events: Vec<Event> = self.engine.borrow_mut().take_events().collect();
        for event in events {
            match event {
                Event::Trigger {
                    trigger,
                    ball,
                    crossing,
                } => {
                    let Some(item) = self.state.items.by_trigger(trigger) else {
                        continue;
                    };
                    let name = item.name.clone();
                    let suffix = match crossing {
                        Crossing::Entered => HIT,
                        Crossing::Exited => UNHIT,
                    };
                    self.fire(&name, suffix, Some(ball));
                }
                Event::Hit { shape, ball, speed } => {
                    let Some(item) = self.state.items.by_shape(shape) else {
                        continue;
                    };
                    // How hard it has to have been hit for the part to notice.
                    // The original compares before it calls anything —
                    // `dot <= -m_threshold` (`collide.cpp:172`), `dot >=
                    // m_threshold` (`collideex.cpp:701`) — and without it the
                    // lightest graze fires the hit sound, closes the switch and
                    // scores. A ball rolling down a wall on its way to the
                    // drain is a dozen of those.
                    if speed < item.hit_threshold() {
                        continue;
                    }
                    let (name, kind) = (item.name.clone(), item.kind);
                    // A spinner reports as spinning; everything else is a plain
                    // hit. A slingshot has its own event and does not come
                    // through here at all.
                    let suffix = match kind {
                        items::Kind::Spinner => SPIN,
                        _ => HIT,
                    };
                    self.fire(&name, suffix, Some(ball));
                    // A drop target starts going down when it is hit on the
                    // face. The shape that reports the hit is the one standing
                    // in front of it — see `physics::hit_target` — so a ball
                    // brushing the back or the side bounces off without
                    // knocking it over. The `Dropped` event is **not** raised
                    // here: it belongs to the moment the target reaches the
                    // bottom of its travel, and comes out of the same
                    // announcement queue a script-driven drop uses.
                    if let Some(item) = self.state.items.get(&name) {
                        item.knock_down();
                    }
                }
                Event::Unhit { shape, ball } => {
                    // Only a kicker produces this. A saucer wired
                    // `swNN_Hit` / `swNN_UnHit` is a switch, and a ROM that
                    // never sees it open re-energises the eject coil at an
                    // empty hole for the rest of the game.
                    let Some(item) = self.state.items.by_shape(shape) else {
                        continue;
                    };
                    let name = item.name.clone();
                    self.fire(&name, UNHIT, Some(ball));
                }
                Event::Slingshot { shape, ball } => {
                    // Everything a slingshot does for the player hangs off this
                    // one call. On F-14 `LeftSlingShot_Slingshot` reports the
                    // switch, plays `fx_slingshot`, swaps the three plastics
                    // that draw the arm moving and starts the timer that puts
                    // them back — and there is no `_Hit` anywhere to fall back
                    // on. Without the event the ball is flung out in silence,
                    // off a plastic that never moves.
                    let Some(item) = self.state.items.by_shape(shape) else {
                        continue;
                    };
                    let name = item.name.clone();
                    self.fire(&name, SLINGSHOT, Some(ball));
                }
            }
        }
    }

    /// Moves every drop target that is mid-slide.
    ///
    /// On the table's clock and not on the video frame, for the reason
    /// [`Game::run_timers`] gives: a bank of targets has to take the same
    /// tenth of a second to fall on a machine running at thirty frames per
    /// second as on one running at a hundred and forty-four. Visual Pinball
    /// itself gets this wrong — `UpdateAnimation` is handed the frame's
    /// elapsed time — so its drop targets fall faster on a faster machine.
    fn animate_targets(&self) {
        for item in self.state.items.iter() {
            item.animate_drop(STEP_MS as f32);
        }
    }

    /// Tells the script about any drop target that has finished moving.
    ///
    /// Every one of them comes through here, whether a ball knocked it down or
    /// a script wrote `IsDropped`: the original raises `Dropped` and `Raised`
    /// from `UpdateAnimation` (`hittarget.cpp:615`, `:626`), when the slide
    /// reaches its end, and by then the ball that started it is long gone. So
    /// there is no `ActiveBall` bound for these, which is the original's
    /// behaviour too.
    fn announce_targets(&self) {
        let names: Vec<(Rc<str>, bool)> = self
            .state
            .items
            .iter()
            .filter(|i| i.drops())
            .filter_map(|i| i.take_announcement().map(|down| (i.name.clone(), down)))
            .collect();
        for (name, down) in names {
            self.fire(&name, if down { DROPPED } else { RAISED }, None);
        }
    }

    /// Calls `Name_Suffix`, with `ActiveBall` bound while it runs.
    fn fire(&self, name: &str, suffix: &str, ball: Option<usize>) {
        let handler = format!("{name}_{suffix}");
        if !self.script.has_proc(&handler) {
            return;
        }
        self.fired.set(self.fired.get() + 1);
        self.state.active_ball.set(ball);
        let outcome = self.call_as(&handler, self.state.items.get(name));
        self.state.active_ball.set(None);
        if let Err(e) = outcome {
            // A handler that fails must not stop the table. Visual Pinball
            // shows a dialog and carries on; a browser has nowhere to put one,
            // so it goes to the log and the ball keeps rolling.
            log::error!("{handler}: {e}");
        }
    }

    /// Calls a handler with `Me` bound to the part it belongs to.
    ///
    /// Restores whatever `Me` was rather than clearing it, because a handler
    /// can call another one — `core.vbs` does — and the outer one's `Me` has to
    /// survive the inner one returning.
    fn call_as(&self, handler: &str, item: Option<Rc<Item>>) -> Result<Option<Value>, VbError> {
        let previous = self.state.current_item.replace(item);
        let outcome = self.script.call(handler, &[]);
        *self.state.current_item.borrow_mut() = previous;
        outcome
    }

    /// Runs whatever timers are due.
    ///
    /// Counted in **table time**, not frames. A table that animates a drop
    /// target over five 30 ms ticks has to take 150 ms whatever the frame rate,
    /// and Visual Pinball itself gets this wrong — its timers run off the video
    /// frame, so an animation is faster on a faster machine.
    fn run_timers(&mut self) {
        let now = self.clock_ms;
        let armed = self.state.items.armed();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.timers_scanned += armed.len() as u64;
        }
        let due: Vec<Rc<Item>> = armed
            .iter()
            // `HitTimer::Update` opens with `if (m_interval >= 0 && ...)`: a
            // negative interval is a frame event and never runs off the clock.
            // See [`Game::new_frame`] and [`Game::game_sync`].
            .filter(|i| i.timer.enabled.get() && i.timer.interval.get() >= 0.0)
            .filter(|i| {
                let at = i.timer.due_ms.get();
                if at.is_nan() {
                    // Switched on but never scheduled. The original gives it
                    // its first due time an interval from now rather than
                    // firing it straight away; this is the only code that knows
                    // what "now" is, so it is done here.
                    i.timer
                        .due_ms
                        .set(now + i.timer.interval.get().max(STEP_MS));
                    return false;
                }
                at <= now
            })
            .cloned()
            .collect();
        // Before any handler runs: a script that arms a timer writes through
        // the very list being borrowed.
        drop(armed);

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.timers_fired += due.len() as u64;
        }
        for item in due {
            let interval = item.timer.interval.get().max(STEP_MS);
            let scheduled = item.timer.due_ms.get();
            self.fire(&item.name, TIMER, None);
            // Advance from when it was *due*, not from now. Rebasing on the
            // current tick makes a timer that runs late drift later still, and
            // a hundred-millisecond animation ends up taking longer than the
            // table meant it to. `hittimer.cpp:36`.
            //
            // Unless the handler rescheduled the timer itself, in which case
            // its choice stands: the original guards this with the same
            // comparison, for the `Enabled = False : Interval = 1000 :
            // Enabled = True` case.
            if item.timer.due_ms.get() == scheduled {
                let mut next = scheduled + interval;
                while next <= now {
                    next += interval;
                }
                item.timer.due_ms.set(next);
            }
        }
    }

    /// The two timer events that belong to a rendered frame rather than to the
    /// clock. `Player::FireTimers`, `player.cpp:1309`.
    fn fire_frame_timers(&mut self, interval: f64) {
        let armed = self.state.items.armed();
        let due: Vec<Rc<Item>> = armed
            .iter()
            .filter(|i| i.timer.enabled.get() && i.timer.interval.get() == interval)
            .cloned()
            .collect();
        drop(armed);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.timers_fired += due.len() as u64;
        }
        for item in due {
            self.fire(&item.name, TIMER, None);
        }
    }

    /// A new frame is about to be drawn: `TimerInterval = -1`.
    ///
    /// `HitTimer::OnNewFrame`, reached from `Player::PrepareFrame` after the
    /// parts have been animated and before anything is drawn
    /// (`player.cpp:2132`).
    pub fn new_frame(&mut self) {
        self.animate();
        self.fire_frame_timers(-1.0);
    }

    /// Tells every moving part that a frame is about to be drawn.
    ///
    /// `Player::UpdateAnimations`, which fires the event at each animatable
    /// object every frame (`player.cpp:2107`). A table uses it to drag
    /// something it has drawn *on top of* a part into line with the part
    /// itself:
    ///
    /// ```vbscript
    /// Sub LeftFlipper_Animate: LeftFlipper_Top.RotZ = LeftFlipper.CurrentAngle: End Sub
    /// ```
    ///
    /// Almost every table since about 2015 does that, because a flipper drawn
    /// as a primitive looks far better than the editor's own. Without this the
    /// primitive never turns: it sits at whatever angle it was authored at
    /// while the invisible flipper underneath does the work, which looks
    /// exactly like a flipper that has come off the playfield.
    ///
    /// Which parts get it is decided once, by asking the script: an object with
    /// no handler costs nothing to skip, and a table with three hundred parts
    /// should not pay for three hundred lookups sixty times a second.
    fn animate(&mut self) {
        if self.animated.borrow().is_none() {
            let names: Vec<Rc<str>> = self
                .state
                .items
                .iter()
                .filter(|i| self.script.has_proc(&format!("{}_{ANIMATE}", i.name)))
                .map(|i| i.name.clone())
                .collect();
            *self.animated.borrow_mut() = Some(names);
        }
        let names = self.animated.borrow().clone().unwrap_or_default();
        for name in names {
            self.fire(&name, ANIMATE, None);
        }
    }

    /// The physics has caught up with the wall clock: `TimerInterval = -2`.
    ///
    /// `HitTimer::OnGameSync`, reached from the main loop immediately after
    /// `UpdatePhysics` and commented there as "trigger script sync event (to
    /// sync solenoids back)" (`player.cpp:1848`).
    ///
    /// This is the one that matters for a ROM table. `core.vbs` arms
    /// `PinMameTimer` at `-2` and polls the controller from it — lamps,
    /// solenoids, switches, the score display, all of it — so how often this is
    /// called *is* how often the machine and the table talk to each other.
    /// Calling it every millisecond, as this port did until the interval was
    /// read properly, does that sixteen times more often than Visual Pinball
    /// does, for sixteen times the cost and no more fidelity: the board it is
    /// polling has not changed in between.
    pub fn game_sync(&mut self) {
        self.fire_frame_timers(-2.0);
    }

    /// The table's own handler for an event, if the script has one.
    ///
    /// Named after the **table object**, which is whatever its author called
    /// it. `Table1` is the editor's default and most tables keep it, but The
    /// Sopranos calls its table `Table` — and looking only for `Table1_KeyDown`
    /// there meant no key ever reached the script, so the coin door and the
    /// start button did nothing at all. The flippers still moved, because the
    /// physics answers the keyboard itself; nothing that had to go through the
    /// machine worked.
    ///
    /// The stored name is tried first and the two common defaults after it,
    /// because the name in the file and the name in the script are two
    /// different strings and a table where they disagree should still play.
    fn handler(&self, event: &str) -> Option<String> {
        [self.table_object.as_str(), "Table1", "Table"]
            .iter()
            .map(|t| format!("{t}_{event}"))
            .find(|name| self.script.has_proc(name))
    }

    /// A key went down or came up. Also hands it to the script, which is what
    /// gives a table its own key bindings.
    pub fn key(&mut self, code: &str, pressed: bool) -> bool {
        let handled = self
            .controls
            .key(&mut self.engine.borrow_mut(), code, pressed);
        if let Some(vp_key) = vp_key_code(code) {
            let suffix = if pressed { "KeyDown" } else { "KeyUp" };
            if let Some(handler) = self.handler(suffix)
                && let Err(e) = self.script.call(&handler, &[Value::Long(vp_key)])
            {
                log::error!("{handler}: {e}");
            }
        }
        handled
    }

    /// Puts a ball in front of the plunger and clears any tilt.
    pub fn new_ball(&mut self) {
        let mut engine = self.engine.borrow_mut();
        engine.balls.clear();
        engine.nudge.reset();
        drop(engine);
        self.controls.release_all(&mut self.engine.borrow_mut());

        let engine = self.engine.borrow();
        let at = self
            .controls
            .plunger()
            .and_then(|i| vpw_table::controls::ball_start_point(&engine, i, self.ball_radius))
            .unwrap_or_else(|| Vec3::new(0.0, 0.0, self.ball_radius));
        drop(engine);
        self.engine
            .borrow_mut()
            .add_ball(Ball::new(at, self.ball_radius));
    }

    /// Fills `out` with interleaved stereo at [`AUDIO_RATE`].
    ///
    /// The host drives this from its audio device, at whatever size and however
    /// often it likes: what comes out is the sound board mixed with whatever
    /// the script has started. Calling it is what makes sounds advance, so a
    /// host that never does hears nothing and also never finishes a sound.
    pub fn render_audio(&mut self, out: &mut [f32]) {
        self.state.audio.borrow_mut().mixer.render(out);
    }

    /// The table's sounds and the mixer playing them.
    pub fn audio(&self) -> std::cell::Ref<'_, Sounds> {
        self.state.audio.borrow()
    }

    /// The same, to start or stop a sound from outside the script. The physics
    /// does not make noise on its own yet; this is how a host or a test does.
    pub fn audio_mut(&self) -> std::cell::RefMut<'_, Sounds> {
        self.state.audio.borrow_mut()
    }

    /// The interpreter, to load or call something from outside the table.
    pub fn script_mut(&mut self) -> &mut Interpreter {
        &mut self.script
    }

    /// The ceiling on each half of the mix.
    ///
    /// Above one is a boost, and it is offered because it is needed: a table
    /// whose ROM was mastered quietly cannot be balanced against its own
    /// bumpers by turning the bumpers down — that just makes the whole
    /// machine quiet. The mixer clamps its output, so the worst a boost can
    /// do is flatten the loudest moments rather than wrap them into noise.
    pub const MAX_MIX_GAIN: f32 = 1.2;

    /// Master volume, 0 to 1.
    pub fn set_volume(&mut self, volume: f32) {
        self.state.audio.borrow_mut().mixer.master = volume.clamp(0.0, 1.0);
    }

    /// How loud the machine's own sound board is against the table's
    /// mechanics. See [`vpw_audio::mixer::Mixer::board_gain`].
    ///
    /// Up to [`MAX_MIX_GAIN`] rather than to one: plenty of ROMs were mastered
    /// quietly and a fader that stops at "as recorded" cannot rescue them.
    pub fn set_machine_volume(&mut self, volume: f32) {
        self.state.audio.borrow_mut().mixer.board_gain = volume.clamp(0.0, Self::MAX_MIX_GAIN);
    }

    /// And the other half: the bumpers, the flippers and the ball.
    pub fn set_table_volume(&mut self, volume: f32) {
        self.state.audio.borrow_mut().mixer.voice_gain = volume.clamp(0.0, Self::MAX_MIX_GAIN);
    }

    /// The sounds the script asked for since the last time this was called.
    pub fn take_sounds(&self) -> Vec<String> {
        std::mem::take(&mut self.state.sounds.borrow_mut())
    }

    /// Anything the script put in front of the player with `MsgBox`.
    pub fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut self.state.messages.borrow_mut())
    }

    /// How many sounds and messages are waiting to be taken.
    ///
    /// Both queues only empty when a host asks for them. A host that never
    /// does keeps every sound name the script ever uttered, which is a slow
    /// leak and invisible without a way to look at it.
    pub fn pending(&self) -> (usize, usize) {
        (
            self.state.sounds.borrow().len(),
            self.state.messages.borrow().len(),
        )
    }

    /// The script, for a host that wants to call a handler itself.
    pub fn script(&self) -> &Interpreter {
        &self.script
    }

    /// How many script handlers have run since the table was loaded.
    ///
    /// One number, and it answers the question that matters when a table looks
    /// dead: are its rules running at all, or is the ball just rolling around
    /// a simulation nobody is watching?
    pub fn handlers_fired(&self) -> u64 {
        self.fired.get()
    }
}

/// The two libraries that have to be there before a table's first line runs.
///
/// Everything else a table needs it asks for itself, through `LoadVPM` or
/// `LoadScript`, and gets through `GetTextFile`. These two are the exception
/// because they are what *defines* those: `controller.vbs` declares `LoadVPM`,
/// and it in turn needs `core.vbs`. A table's line 80 reading
/// `LoadVPM "01560000", "S11.VBS", 3.26` cannot bootstrap itself.
const BOOTSTRAP: [&str; 2] = ["core.vbs", "controller.vbs"];

/// The globals Visual Pinball defines before any script runs.
///
/// A table's script does not declare these and cannot work without them.
/// `s11.vbs` opens by reading `VPBuildVersion` to decide **how to load its own
/// libraries** — a negative or missing value sends it down a path that opens
/// files through `Scripting.FileSystemObject`, which does not exist in a
/// browser, and the table then reports that it cannot find `core.vbs`. One
/// missing number is the difference between a table that loads and one that
/// does not.
///
/// The key codes are DirectInput scan codes, which is what
/// `Table1_KeyDown(keycode)` is handed and compares against. They have to agree
/// with [`vp_key_code`], which is what produces them — a table comparing
/// `keycode = LeftFlipperKey` only works if both sides say 42.
/// The globals whose answer is different every time they are asked.
///
/// Kept apart from [`vp_global`] because of what happens to the rest: a name
/// that resolves is remembered, so `LeftFlipper` costs one lookup for the whole
/// game. That is right for a part and for a version number, and quietly fatal
/// for a clock. `GameTime` was going through the same door, so the first read
/// froze it — measured on F-14, it answered 0 at the start of the game and 0
/// two and a half seconds later, for good. A script that measures an interval
/// gets zero every time, and `core.vbs` measures plenty of them.
///
/// The split is here rather than a list of exceptions at the cache so that
/// adding a live global is a matter of putting it in this function, and putting
/// it in the other one gives a value that never changes again.
fn vp_live_global(name: &str, state: &GameState) -> Option<Value> {
    Some(match name {
        // Table time, which is what a script measures intervals with.
        // `m_time_msec`, in milliseconds (`ScriptGlobalTable.cpp:682`).
        "gametime" => Value::Long(state.clock_ms.get() as i32),

        // The same clock in **seconds**: it is `m_time_sec`
        // (`ScriptGlobalTable.cpp:691`), which the physics engine keeps as
        // microseconds over a million (`PhysicsEngine.cpp:486`). `core.vbs`
        // says so itself, using one as the other's replacement:
        //
        //     If HasPreciseGameTime Then
        //         Controller.TimeFence = PreciseGameTime
        //     Else
        //         Controller.TimeFence = GameTime / 1000.0
        //
        // so handing back milliseconds here is a thousandfold error in whatever
        // the script does with it.
        "precisegametime" => Value::Double(state.clock_ms.get() / 1000.0),

        // The wall clock in the original — `msec()`, milliseconds since the
        // player started (`ScriptGlobalTable.cpp:700`), which keeps running
        // while the table is paused. Table time instead, because there is no
        // clock down here to ask: `std::time::Instant` does not work on
        // `wasm32-unknown-unknown` and nothing hands one in. It is monotonic
        // and in milliseconds, which is the shape a script expects; it differs
        // only across a pause, and only for a script that measures real time
        // rather than table time.
        "systemtime" => Value::Long(state.clock_ms.get() as i32),
        _ => return None,
    })
}

fn vp_global(name: &str) -> Option<Value> {
    Some(match name {
        // Which build the table thinks it is running on. Anything not negative
        // sends a library down its modern path.
        "vpbuildversion" => Value::Long(10_800),
        "versionmajor" => Value::Long(10),
        "versionminor" => Value::Long(8),
        "versionrevision" => Value::Long(0),
        "version" => Value::Long(10_800),

        // Where it is being played. On a screen, never in a cabinet.
        // Renderer settings a table writes and this port has no equivalent
        // for. They are here so that *writing* one is allowed at all: under
        // `Option Explicit` a name the host has never heard of cannot be
        // assigned, and Circus opens its room-brightness routine by setting
        // this one. Nothing reads them back — there is no static pre-render
        // here to disable.
        "disablestaticprerendering" => Value::Bool(false),
        "showdt" | "showdesktop" => Value::Bool(true),
        "showfss" => Value::Bool(false),
        "renderingmode" => Value::Long(0),
        "platformos" => Value::str("Web"),
        "platformbits" => Value::Long(32),
        "platformcpu" => Value::str("wasm"),

        // The keys. See the note above about agreeing with `vp_key_code`.
        "leftflipperkey" | "stagedleftflipperkey" => Value::Long(42),
        "rightflipperkey" | "stagedrightflipperkey" => Value::Long(54),
        "plungerkey" => Value::Long(28),
        "startgamekey" => Value::Long(2),
        "addcreditkey" => Value::Long(6),
        "addcreditkey2" => Value::Long(7),
        "lefttiltkey" => Value::Long(44),
        "righttiltkey" => Value::Long(53),
        "centertiltkey" => Value::Long(57),
        "mechanicaltilt" => Value::Long(88),
        "leftmagnasave" => Value::Long(29),
        "rightmagnasave" => Value::Long(157),
        "lockbarkey" => Value::Long(56),
        "exitgame" => Value::Long(1),

        // A window handle a browser does not have. Zero is the honest answer
        // and tables only ever pass it on.
        "getplayerhwnd" => Value::Long(0),
        "nightday" => Value::Long(50),
        "musicvolume" => Value::Long(100),

        // Directories, for a table that wants to tell the player where things
        // are. There is no filesystem, so they are empty rather than wrong.
        "scriptsdirectory" | "tablesdirectory" | "musicdirectory" | "userdirectory" => {
            Value::str("")
        }
        _ => return None,
    })
}

/// `Setting(section, key)`: what the player has configured.
///
/// It decides more than it looks. `controller.vbs` reads
/// `Setting("Controller", "ForceDisableB2S")` and, if it comes back zero, goes
/// looking for a B2S backglass server before it will consider loading
/// VPinMAME. There is no B2S here and there is not going to be one, so saying
/// so plainly is what puts the table on the path that ends with a controller.
/// Left unanswered, the setting reads as `Empty`, `Empty = 0` is true, and the
/// table waits for a backglass that never arrives.
struct SettingLookup;

impl Object for SettingLookup {
    fn type_name(&self) -> &'static str {
        "Setting"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let key = match args.get(1) {
            Some(v) => v.to_str()?.to_ascii_lowercase(),
            None => return Ok(Value::str("0")),
        };
        Ok(match &*key {
            // No backglass server.
            "forcedisableb2s" => Value::str("1"),
            // No force feedback either. Two means "the table decides", which
            // for a DOF effect means "do nothing".
            _ if key.starts_with("dof") => Value::str("2"),
            _ => Value::str("0"),
        })
    }
}

/// `VPXActionKey(n)`: the key bound to one of the player's actions.
///
/// `VPMKeys.vbs` is written almost entirely in terms of this, and without it
/// that file fails and every library that depends on it — which is all of them
/// — reports that it cannot be opened.
///
/// The numbers are the DirectInput scan codes of Visual Pinball's own defaults,
/// read off the comments in `VPMKeys.vbs` itself. Actions this player has no
/// key for get a code it will never produce, so a table comparing against them
/// simply never matches, which is the truth: there is no coin slot.
struct ActionKey;

impl Object for ActionKey {
    fn type_name(&self) -> &'static str {
        "VPXActionKey"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let action = match args.first() {
            Some(v) => v.to_int()?,
            None => return Err(VbError::invalid_call()),
        };
        Ok(Value::Long(match action {
            9 => 6,    // insert coin 1 — the "5" key
            10 => 5,   // insert coin 2 — "4"
            11 => 4,   // insert coin 3 — "3"
            12 => 7,   // insert coin 4 — "6"
            13 => 20,  // bang back — T
            19 => 61,  // reset emulation — F3
            22 => 48,  // add ball — B
            23 => 199, // slam tilt — Home
            24 => 207, // coin door — End
            25 => 8,   // cancel — "7"
            26 => 9,   // down — "8"
            27 => 10,  // up — "9"
            28 => 11,  // enter — "0"
            29 => 7,   // service — "6"
            30 => 201, // page up
            31 => 12,  // minus
            // Well above any scan code, and distinct per action so two of them
            // never compare equal.
            other => 1000 + other,
        }))
    }
}

/// Visual Pinball's key codes, for the ones this player produces.
///
/// They are DirectInput scan codes, not ASCII and not `KeyboardEvent.code`, so
/// there has to be a table. A script's `Table1_KeyDown` compares against the
/// named constants (`LeftFlipperKey`, `PlungerKey`), which are these numbers.
/// A browser key name to the code Visual Pinball's scripts expect.
///
/// The numbers are DirectInput scan codes — `DIK_1` is 2, `DIK_5` is 6 — which
/// is what a table's `Table1_KeyDown` is written against and what every
/// machine library's `Select Case keycode` compares. They have nothing to do
/// with the character on the key, which is why the mapping has to be spelled
/// out rather than derived.
///
/// The digit row is the coin door. On a real machine those buttons are behind
/// a locked door and only an operator sees them; here they are the difference
/// between a table you can look at and a table you can put a coin in and play,
/// so all of them are wired:
///
/// | Key | What it is |
/// |---|---|
/// | 1 | Start button |
/// | 3, 4, 5 | The three coin slots |
/// | 6 | High-score reset |
/// | 7 | The coin door's up/down toggle |
/// | 8 | Advance, which walks the operator menu |
/// | 9, 0 | CPU and sound board diagnostics |
fn vp_key_code(code: &str) -> Option<i32> {
    Some(match code {
        "KeyZ" => 42,       // LeftFlipperKey
        "KeyM" => 54,       // RightFlipperKey
        "Space" => 28,      // PlungerKey
        "Enter" => 28,      // the plunger again: it is what a keyboard player expects
        "ArrowLeft" => 44,  // LeftTiltKey
        "ArrowRight" => 53, // RightTiltKey
        "ArrowUp" => 57,    // CenterTiltKey
        "KeyT" => 20,       // keyBangBack, the mechanical tilt

        // The coin door.
        "Digit1" => 2,  // StartGameKey
        "Digit3" => 4,  // keyInsertCoin2
        "Digit4" => 5,  // keyInsertCoin3
        "Digit5" => 6,  // keyInsertCoin1, the one everybody uses
        "Digit6" => 7,  // keyHiscoreReset
        "Digit7" => 8,  // keyUpDown
        "Digit8" => 9,  // keyAdvance
        "Digit9" => 10, // keyCPUDiag
        "Digit0" => 11, // keySoundDiag
        _ => return None,
    })
}

/// The host the script talks to.
struct TableHost {
    state: Rc<GameState>,
}

impl TableHost {
    /// A global name the script used and never declared.
    ///
    /// The order matters: a table's own parts win over anything else, because
    /// a table is free to name a wall `Timer` and a script that says `Timer`
    /// means the wall.
    fn resolve(&self, name: &str) -> Option<Value> {
        if let Some(item) = self.state.items.get(name) {
            return Some(Value::Object(item));
        }
        // Then the collections. After the parts, because a part and a
        // collection can share a name and the original resolves the part.
        if let Some(members) = self
            .state
            .collections
            .get(name.to_ascii_lowercase().as_str())
        {
            return Some(Value::Object(Rc::new(Collection {
                members: members.clone(),
            })));
        }
        match &*name.to_ascii_lowercase() {
            // The table itself. A script reaches it as `Table1` and, inside one
            // of its own methods, as `Me`.
            "table1" | "table" => Some(Value::Object(Rc::new(TableObject {
                state: self.state.clone(),
            }))),
            // Global subs, not objects: `PlaySound "bumper", 0, 1`.
            "playsound" | "playsoundat" | "playsoundatball" | "playmusic" => {
                Some(Value::Object(Rc::new(PlaySound {
                    state: self.state.clone(),
                })))
            }
            "stopsound" => Some(Value::Object(Rc::new(StopSound {
                state: self.state.clone(),
            }))),
            // Music is streamed from a file next to the table rather than kept
            // inside it, and there is no file here. Accepted and ignored, which
            // is better than a table stopping at its first `EndMusic`.
            "endmusic" | "fireknocker" | "playsoundatlevelstatic" => {
                Some(Value::Object(Rc::new(Stub)))
            }
            // Visual Pinball's own hook for reading a script off disk. A
            // table's `LoadVPM` is `ExecuteGlobal GetTextFile("S11.VBS")`, so
            // answering this is what lets a table pull in whatever it needs
            // without us having to guess the list.
            "gettextfile" => Some(Value::Object(Rc::new(GetTextFile {
                state: self.state.clone(),
            }))),
            // The player's own key-value store, which is where a table keeps
            // its high scores and its options between sessions
            // (`ScriptGlobalTable.cpp:505` and `:581`). Two things about it
            // matter and both are load-bearing: a value that was never saved
            // reads back as an **empty string** rather than failing, which is
            // how every table asks "has anybody played me before"; and it is
            // keyed by the table's own name, so two tables cannot tread on
            // each other's scores.
            //
            // Ice Fever calls it on the first line of its `Init` and stopped
            // there without it, which is the shape of every missing global:
            // silent until the one table that uses it, then total.
            // A table rewriting a material at run time
            // (`ScriptGlobalTable.cpp:724`). Almost every call is set-up —
            // The Getaway tunes its ball shadows this way before the first
            // ball — so the change is recorded and put into the scene before
            // it is uploaded. See [`Game::apply_material_changes`].
            "updatematerial" => Some(Value::Object(Rc::new(UpdateMaterial {
                state: self.state.clone(),
                physics_only: false,
            }))),
            "updatematerialphysics" => Some(Value::Object(Rc::new(UpdateMaterial {
                state: self.state.clone(),
                physics_only: true,
            }))),
            // The read half of the pair, which a table uses to save a
            // material's colour before it tints it
            // (`ScriptGlobalTable.cpp:700`).
            //
            // It answers, and it answers nothing. `GetMaterial` hands its
            // values back through **sixteen by-reference arguments**, and a
            // host object here is given values rather than the caller's cells
            // — script procedures alias them, objects do not. So the variables
            // come back `Empty`.
            //
            // That is the difference between a table that loads and one that
            // does not, which is worth having on its own; what it costs is the
            // room-brightness feature a few tables build on top, which will
            // save a colour of nothing and tint towards it.
            "getmaterial" => Some(Value::Object(Rc::new(GetMaterial {
                state: self.state.clone(),
                physics_only: false,
            }))),
            // Four of the sixteen, for a table that only wants to know how
            // bouncy something is (`ScriptGlobalTable.cpp:815`).
            "getmaterialphysics" => Some(Value::Object(Rc::new(GetMaterial {
                state: self.state.clone(),
                physics_only: true,
            }))),
            // The same idea with one argument instead of sixteen: repaint a
            // material and leave everything else about it alone
            // (`ScriptGlobalTable.cpp:757`). A table uses it to tint its
            // rubbers and its plastics as the general illumination comes up.
            "materialcolor" => Some(Value::Object(Rc::new(MaterialColor {
                state: self.state.clone(),
            }))),
            "loadvalue" => Some(Value::Object(Rc::new(LoadValue {
                state: self.state.clone(),
            }))),
            "savevalue" => Some(Value::Object(Rc::new(SaveValue {
                state: self.state.clone(),
            }))),
            // `GetBalls` is how a table finds the balls in play, and there is
            // no other way: the script has `ActiveBall` inside a handler and
            // nothing at all outside one. Every table's rolling-sound routine
            // is built on it, which is why a table without it plays in silence
            // even with every sound loaded.
            "getballs" => Some(Value::Object(Rc::new(GetBalls {
                state: self.state.clone(),
            }))),
            // Functions, not values.
            "vpxactionkey" => Some(Value::Object(Rc::new(ActionKey))),
            "setting" => Some(Value::Object(Rc::new(SettingLookup))),
            _ => vp_global(&name.to_ascii_lowercase()),
        }
    }
}

impl Host for TableHost {
    fn global(&self, name: &str) -> Option<Value> {
        // `ActiveBall` is the one name that must never be cached: it means a
        // different ball in every event, and a stale one would have a script
        // reading the velocity of a ball that drained two handlers ago.
        // Neither of these may be cached: they mean something different in
        // every handler, and a stale one is worse than a missing one.
        if name.eq_ignore_ascii_case("Me") {
            return Some(match self.state.current_item.borrow().clone() {
                Some(item) => Value::Object(item),
                // Outside a part's handler, `Me` is the table.
                None => Value::Object(Rc::new(TableObject {
                    state: self.state.clone(),
                })),
            });
        }
        if name.eq_ignore_ascii_case("ActiveBall") {
            let index = self.state.active_ball.get()?;
            return Some(Value::Object(Rc::new(BallObject {
                engine: self.state.engine.clone(),
                index,
            })));
        }
        let lowercase = name.to_ascii_lowercase();
        if let Some(v) = vp_live_global(&lowercase, &self.state) {
            return Some(v);
        }
        if let Some(v) = self.state.resolved.borrow().get(lowercase.as_str()) {
            return Some(v.clone());
        }
        let v = self.resolve(name)?;
        self.state
            .resolved
            .borrow_mut()
            .insert(lowercase.into_boxed_str(), v.clone());
        Some(v)
    }

    fn create_object(&self, prog_id: &str) -> VbResult<Value> {
        // The one object tables ask for is VPinMAME's controller, and behind it
        // is a real System 11 board. Every caller gets the same one — see
        // [`GameState::controller`].
        let id = prog_id.to_ascii_lowercase();
        if id.contains("vpinmame") {
            return Ok(Value::Object(self.state.controller()));
        }
        // `core.vbs` will not load without one of these: it keeps the classes
        // whose timers are due in a dictionary.
        if id.contains("dictionary") {
            return Ok(Value::Object(Rc::new(
                vpw_vbscript::scripting::Dictionary::new(),
            )));
        }
        // And a table that remembers a preference between sessions asks for a
        // file system. There is none here, and saying so is what lets the
        // table take the path it takes the first time it is ever run.
        if id.contains("filesystemobject") {
            return Ok(Value::Object(Rc::new(vpw_vbscript::scripting::FileSystem)));
        }
        Err(VbError::new(
            429,
            format!("ActiveX component can't create object: '{prog_id}'"),
        ))
    }

    fn message(&self, text: &str) {
        log::trace!("table says: {text}");
        self.state.messages.borrow_mut().push(text.to_string());
        let clock = self.state.clock_ms.get();
        self.state
            .telemetry
            .borrow_mut()
            .event(clock, telemetry::Event::Message(text.into()));
    }

    fn seconds(&self) -> f64 {
        self.state.clock_ms.get() / 1000.0
    }

    fn now_millis(&self) -> Option<f64> {
        Some(self.state.clock.now_millis())
    }
}

/// `Table1`, and `Me` inside a table method.
struct TableObject {
    state: Rc<GameState>,
}

impl Object for TableObject {
    fn type_name(&self) -> &'static str {
        "Table"
    }

    fn get(&self, name: &str, args: &[Value]) -> VbResult<Value> {
        match &*name.to_ascii_lowercase() {
            // `Table1.Nudge angle, force` — a script shaking its own cabinet,
            // which is how the tilt bob gets exercised on a ROM table.
            "nudge" => {
                let angle = args
                    .first()
                    .map(Value::to_number)
                    .transpose()?
                    .unwrap_or(0.0);
                self.state.engine.borrow_mut().nudge_at(angle as f32);
                Ok(Value::Empty)
            }
            "name" => Ok(Value::str("Table1")),

            // The playfield's size, and not an optional nicety: the `Pan`
            // function that virtually every table carries is
            //
            //     tmp = ball.x * 2 / table1.width - 1
            //
            // so a table whose `Width` reads as `Empty` divides by zero, the
            // whole `PlaySound` line raises, and the ball rolls in silence.
            // That is how this came to be written: F-14 played, and only the
            // rolling sound was missing.
            "width" => Ok(Value::Double(f64::from(self.state.size.0))),
            "height" => Ok(Value::Double(f64::from(self.state.size.1))),

            // Anything the script reads off the table and we do not model
            // answers `Empty` rather than failing. These are almost all
            // presentation — `ColorGradeImage`, `BallImage` — and a table that
            // cannot set one should still play.
            //
            // Except the handful that are also globals. Visual Pinball puts
            // `ShowDT` and its neighbours in both places, and a table is free
            // to ask either way: F-14 opens with
            //
            //     Dim DesktopMode: DesktopMode = Table1.ShowDT
            //
            // and everything it draws for a player at a desk — the whole score
            // display among it — sits behind that one variable. Read as `Empty`
            // it is neither true nor false, the branch never runs, and the
            // table plays with a blank backbox and no sign of why.
            other => Ok(vp_live_global(other, &self.state)
                .or_else(|| vp_global(other))
                .unwrap_or(Value::Empty)),
        }
    }

    fn set(&self, _name: &str, _args: &[Value], _value: Value, _by_ref: bool) -> VbResult<()> {
        Ok(())
    }
}

/// `ActiveBall`: the ball that caused the event being handled.
pub(crate) struct BallObject {
    pub(crate) engine: Rc<RefCell<Engine>>,
    pub(crate) index: usize,
}

impl Object for BallObject {
    fn type_name(&self) -> &'static str {
        "Ball"
    }

    fn get(&self, name: &str, _args: &[Value]) -> VbResult<Value> {
        let engine = self.engine.borrow();
        let Some(b) = engine.balls.get(self.index) else {
            return Err(VbError::object_variable_not_set());
        };
        Ok(match &*name.to_ascii_lowercase() {
            "x" => Value::Double(b.pos.x.into()),
            "y" => Value::Double(b.pos.y.into()),
            "z" => Value::Double(b.pos.z.into()),
            "velx" => Value::Double(b.vel.x.into()),
            "vely" => Value::Double(b.vel.y.into()),
            "velz" => Value::Double(b.vel.z.into()),
            "angmomx" => Value::Double(b.angular_momentum.x.into()),
            "angmomy" => Value::Double(b.angular_momentum.y.into()),
            "angmomz" => Value::Double(b.angular_momentum.z.into()),
            // Its own number, which the original hands out once and never
            // reuses (`ball.cpp:829`). A table's velocity tracker keeps an
            // array indexed by it, so a reused one would mix two balls up.
            "id" => Value::Long(b.id as i32),
            "radius" => Value::Double(b.radius.into()),
            "mass" => Value::Double(b.mass.into()),
            // Presentation the renderer does not read yet.
            "image" | "frontdecal" | "color" => Value::str(""),
            _ => return Err(VbError::no_such_member(name)),
        })
    }

    fn set(&self, name: &str, _args: &[Value], value: Value, _by_ref: bool) -> VbResult<()> {
        let mut engine = self.engine.borrow_mut();
        let Some(b) = engine.balls.get_mut(self.index) else {
            return Err(VbError::object_variable_not_set());
        };
        let n = || value.to_number().map(|v| v as f32);
        match &*name.to_ascii_lowercase() {
            "x" => b.pos.x = n()?,
            "y" => b.pos.y = n()?,
            "z" => b.pos.z = n()?,
            "velx" => b.vel.x = n()?,
            "vely" => b.vel.y = n()?,
            "velz" => b.vel.z = n()?,
            "angmomx" => b.angular_momentum.x = n()?,
            "angmomy" => b.angular_momentum.y = n()?,
            "angmomz" => b.angular_momentum.z = n()?,
            "mass" => b.mass = n()?,
            "image" | "frontdecal" | "color" => {}
            _ => return Err(VbError::no_such_member(name)),
        }
        Ok(())
    }
}

/// `PlaySound "name", loop, volume, …`.
///
/// A global procedure rather than an object in Visual Pinball; here it is an
/// object with a default member, which is the only way a name can be callable
/// through the host interface. The script cannot tell.
struct PlaySound {
    state: Rc<GameState>,
}

impl Object for PlaySound {
    fn type_name(&self) -> &'static str {
        "PlaySound"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let Some(Ok(name)) = args.first().map(Value::to_str) else {
            return Ok(Value::Empty);
        };
        {
            let mut log = self.state.sounds.borrow_mut();
            if log.len() >= SOUND_LOG {
                log.remove(0);
            }
            log.push(name.to_string());
        }
        {
            let clock = self.state.clock_ms.get();
            self.state
                .telemetry
                .borrow_mut()
                .event(clock, telemetry::Event::Sound(name.as_ref().into()));
        }
        self.state
            .audio
            .borrow_mut()
            .play(&name, Request::from_args(args));
        Ok(Value::Empty)
    }

    fn default_value(&self) -> VbResult<Value> {
        Ok(Value::Empty)
    }
}

/// A named group of parts, made in the table's editor.
///
/// A script never declares one and never builds one: it just says
/// `For Each x In GI`, and the name resolves because Visual Pinball puts every
/// collection in the global scope. That makes them invisible when reading a
/// script — the first sign one is missing is a sub that raises on its first
/// line under `Option Explicit`.
struct Collection {
    members: Vec<Rc<Item>>,
}

impl Object for Collection {
    fn type_name(&self) -> &'static str {
        "Collection"
    }

    fn get(&self, name: &str, args: &[Value]) -> VbResult<Value> {
        match &*name.to_ascii_lowercase() {
            "count" => Ok(Value::Long(self.members.len() as i32)),
            // `GI(3)`, and the bare `GI(3)` that is really an index.
            "" | "item" => {
                let i = args.first().ok_or_else(VbError::invalid_call)?.to_int()?;
                let item = usize::try_from(i)
                    .ok()
                    .and_then(|i| self.members.get(i))
                    .ok_or_else(VbError::subscript_out_of_range)?;
                Ok(Value::Object(item.clone()))
            }
            _ => Err(VbError::no_such_member(name)),
        }
    }

    fn set(&self, name: &str, _args: &[Value], _value: Value, _by_ref: bool) -> VbResult<()> {
        Err(VbError::no_such_member(name))
    }

    /// `For Each x In GI` — the only thing most tables ever do with one.
    fn enumerate(&self) -> Option<Vec<Value>> {
        Some(
            self.members
                .iter()
                .map(|i| Value::Object(i.clone()))
                .collect(),
        )
    }
}

/// `GetBalls`: every ball on the table, as an array.
///
/// The array is built fresh on each call — it has to be, because the point of
/// asking is to find out what is there now. A table calls this twenty times a
/// second from its rolling-sound timer, so it is a handful of allocations per
/// frame and worth leaving alone until something says otherwise.
///
/// An empty table gives an array whose `UBound` is -1, which is exactly what
/// `If UBound(BOT) = -1 Then Exit Sub` in every table's routine is testing.
struct GetBalls {
    state: Rc<GameState>,
}

impl Object for GetBalls {
    fn type_name(&self) -> &'static str {
        "GetBalls"
    }

    fn get(&self, _name: &str, _args: &[Value]) -> VbResult<Value> {
        let count = self.state.engine.borrow().balls.len();
        let balls: Vec<Value> = (0..count)
            .map(|index| {
                Value::Object(Rc::new(BallObject {
                    engine: self.state.engine.clone(),
                    index,
                }))
            })
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(
            vpw_vbscript::value::Array::from_values(balls),
        ))))
    }

    fn default_value(&self) -> VbResult<Value> {
        self.get("", &[])
    }
}

/// `StopSound name`.
struct StopSound {
    state: Rc<GameState>,
}

impl Object for StopSound {
    fn type_name(&self) -> &'static str {
        "StopSound"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        if let Some(Ok(name)) = args.first().map(Value::to_str) {
            self.state.audio.borrow_mut().stop(&name);
        }
        Ok(Value::Empty)
    }

    fn default_value(&self) -> VbResult<Value> {
        Ok(Value::Empty)
    }
}

/// The name a table's own event handlers are prefixed with.
///
/// Visual Pinball's editor calls the table object `Table1` and stores whatever
/// it ends up being called in the file's `NAME`. Empty falls back to the
/// default rather than to nothing, so a file that never wrote one still finds
/// the handlers a script written against the default would have.
fn table_object_name(stored: &str) -> String {
    if stored.trim().is_empty() {
        "Table1".into()
    } else {
        stored.trim().into()
    }
}

/// The name of the part a generated mesh belongs to.
///
/// One part often becomes several meshes, and the extra ones say what they are
/// in brackets: a wall is `Plastic1 (top)` and `Plastic1 (side)`, a ramp's
/// handrails are `Ramp1 (wire 1)` and so on. The script only ever knows the
/// part, so the bracket has to come off before anything is looked up by name.
fn owner_of(mesh: &str) -> &str {
    match mesh.rfind(" (") {
        Some(at) if mesh.ends_with(')') => &mesh[..at],
        _ => mesh,
    }
}

/// `GetTextFile("name.vbs")`.
///
/// The one piece of file access a table has, and the reason it does not need
/// any other: `LoadVPM` and `LoadScript` are both this plus `ExecuteGlobal`.
/// It reaches a [`ScriptLibrary`] rather than a filesystem, because in a
/// browser there is no filesystem to reach.
struct GetTextFile {
    state: Rc<GameState>,
}

impl Object for GetTextFile {
    fn type_name(&self) -> &'static str {
        "GetTextFile"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let wanted = match args.first() {
            Some(v) => v.to_str()?,
            None => return Err(VbError::invalid_call()),
        };
        match self.state.libraries.read(&wanted) {
            Some(text) => Ok(Value::str(text)),
            // The real one raises, and a table's `On Error Resume Next` around
            // the call is written expecting it to.
            None => Err(VbError::new(53, format!("File not found: '{wanted}'"))),
        }
    }
}

/// `UpdateMaterial(name, ...)` and its physics-only sibling.
struct UpdateMaterial {
    state: Rc<GameState>,
    physics_only: bool,
}

impl Object for UpdateMaterial {
    fn type_name(&self) -> &'static str {
        "UpdateMaterial"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let Some(first) = args.first() else {
            return Err(VbError::invalid_call());
        };
        let name = first.to_str()?.to_lowercase();
        // Only the look. The four physics numbers at the end are the same in
        // both forms and are not modelled: a material's elasticity is read
        // when the collision shapes are built, and those are built before a
        // script can say anything.
        if self.physics_only {
            return Ok(Value::Empty);
        }
        let n = |i: usize| {
            args.get(i)
                .map_or(Ok(0.0), |v| v.to_number())
                .map(|v| v as f32)
        };
        let colour = |i: usize| -> VbResult<[f32; 3]> {
            let c = args.get(i).map_or(Ok(0.0), |v| v.to_number())? as u32;
            // An OLE colour, which is BGR packed into a long.
            Ok([
                (c & 0xff) as f32 / 255.0,
                ((c >> 8) & 0xff) as f32 / 255.0,
                ((c >> 16) & 0xff) as f32 / 255.0,
            ])
        };
        let boolean = |i: usize| args.get(i).is_some_and(|v| v.to_bool().unwrap_or(false));
        self.state.materials.borrow_mut().insert(
            name,
            MaterialChange {
                wrap_lighting: n(1)?,
                roughness: n(2)?,
                glossy_image_lerp: n(3)?,
                thickness: n(4)?,
                edge: n(5)?,
                edge_alpha: n(6)?,
                opacity: n(7)?,
                base_color: colour(8)?,
                glossy_color: colour(9)?,
                clearcoat_color: colour(10)?,
                is_metal: boolean(11),
                opacity_active: boolean(12),
            },
        );
        Ok(Value::Empty)
    }
}

/// What one `UpdateMaterial` call asked for.
#[derive(Debug, Clone, Copy)]
struct MaterialChange {
    wrap_lighting: f32,
    roughness: f32,
    glossy_image_lerp: f32,
    thickness: f32,
    edge: f32,
    edge_alpha: f32,
    opacity: f32,
    base_color: [f32; 3],
    glossy_color: [f32; 3],
    clearcoat_color: [f32; 3],
    is_metal: bool,
    opacity_active: bool,
}

/// A material as the table's author left it: the twelve numbers that describe
/// how it looks and the four that describe how it bounces.
#[derive(Debug, Clone, Copy)]
struct Authored {
    look: MaterialChange,
    elasticity: f32,
    elasticity_falloff: f32,
    friction: f32,
    /// In **degrees**, which is the unit a script reads and writes.
    scatter_angle: f32,
}

/// `GetMaterial(name, …)` and `GetMaterialPhysics(name, …)`: the read half of
/// the material pair (`ScriptGlobalTable.cpp:759` and `:815`).
///
/// Both answer entirely through by-reference arguments — sixteen of them, or
/// four — and return nothing at all. See [`Object::call_out`], which exists
/// for this.
///
/// A table uses it in one shape, over and over: save a material, tint it,
/// and put it back when the light comes down.
///
/// ```vbscript
/// GetMaterial "Plastics", w, r, gil, t, e, ea, o, b, g, cc, im, oa, el, ef, f, sa
/// UpdateMaterial "Plastics", w, r, gil, t, e, ea, o, Dim(b, 0.4), g, cc, im, oa
/// ```
///
/// Without the first line the second one writes zeros, and the room goes
/// black instead of dim. That is the whole reason this is here.
struct GetMaterial {
    state: Rc<GameState>,
    physics_only: bool,
}

impl GetMaterial {
    /// Writes the answers into `args`, which the interpreter then copies back
    /// into the caller's variables.
    ///
    /// Only as many as the script actually passed: a table that wants four of
    /// the sixteen passes four, and reaching past them would be writing into
    /// nothing.
    fn fill(&self, args: &mut [Value]) -> VbResult<Value> {
        let Some(first) = args.first() else {
            return Err(VbError::invalid_call());
        };
        let name = first.to_str()?.to_lowercase();
        let Some(m) = self.state.authored.get(&name) else {
            // The original looks the name up, finds the dummy material, and
            // returns `E_FAIL` — a table asking about a material it does not
            // have is a table with a typo in it, and saying so is kinder than
            // handing back a plausible-looking nothing.
            return Err(VbError::new(
                5,
                format!("no material named '{}'", first.to_str()?),
            ));
        };

        // What the script has done to it since. The same order as
        // [`Game::apply_material_changes`]: a repaint goes on first, a
        // wholesale rewrite wins over it.
        let mut look = m.look;
        if let Some(tint) = self.state.tints.borrow().get(&name) {
            look.base_color = *tint;
        }
        if let Some(change) = self.state.materials.borrow().get(&name) {
            look = *change;
        }

        let out: Vec<Value> = if self.physics_only {
            vec![
                Value::from_number(f64::from(m.elasticity)),
                Value::from_number(f64::from(m.elasticity_falloff)),
                Value::from_number(f64::from(m.friction)),
                Value::from_number(f64::from(m.scatter_angle)),
            ]
        } else {
            vec![
                Value::from_number(f64::from(look.wrap_lighting)),
                Value::from_number(f64::from(look.roughness)),
                Value::from_number(f64::from(look.glossy_image_lerp)),
                Value::from_number(f64::from(look.thickness)),
                Value::from_number(f64::from(look.edge)),
                Value::from_number(f64::from(look.edge_alpha)),
                Value::from_number(f64::from(look.opacity)),
                Value::Long(pack(look.base_color)),
                Value::Long(pack(look.glossy_color)),
                Value::Long(pack(look.clearcoat_color)),
                Value::Bool(look.is_metal),
                Value::Bool(look.opacity_active),
                Value::from_number(f64::from(m.elasticity)),
                Value::from_number(f64::from(m.elasticity_falloff)),
                Value::from_number(f64::from(m.friction)),
                Value::from_number(f64::from(m.scatter_angle)),
            ]
        };
        for (slot, v) in args[1..].iter_mut().zip(out) {
            *slot = v;
        }
        Ok(Value::Empty)
    }
}

impl Object for GetMaterial {
    fn type_name(&self) -> &'static str {
        "GetMaterial"
    }

    fn call_out(&self, _name: &str, args: &mut [Value]) -> Option<VbResult<Value>> {
        Some(self.fill(args))
    }

    /// Reached only if something calls it without offering the arguments back
    /// — there is nowhere to put an answer, so there is no answer.
    fn get(&self, _name: &str, _args: &[Value]) -> VbResult<Value> {
        Ok(Value::Empty)
    }
}

/// Three channels back into the long a script reads a colour as: `0x00BBGGRR`,
/// which is red in the low byte however it is usually said aloud.
fn pack(c: [f32; 3]) -> i32 {
    let byte = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xff;
    (byte(c[0]) | (byte(c[1]) << 8) | (byte(c[2]) << 16)) as i32
}

/// `MaterialColor(name, colour)`: repaint one material and change nothing
/// else about it.
///
/// Which is why it does not go through [`MaterialChange`]. That record is a
/// material's *whole* appearance, and filling one in from a single colour
/// would hand back a rubber with no gloss and no thickness. A repaint is kept
/// on its own and laid over whatever the table was authored with.
struct MaterialColor {
    state: Rc<GameState>,
}

impl Object for MaterialColor {
    fn type_name(&self) -> &'static str {
        "MaterialColor"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let Some(first) = args.first() else {
            return Err(VbError::invalid_call());
        };
        let name = first.to_str()?.to_lowercase();
        let packed = args.get(1).map_or(Ok(0.0), |v| v.to_number())? as u32;
        // An OLE colour: blue, green and red packed into a long, in that
        // order, which is the other way round from how anybody says it.
        let colour = [
            (packed & 0xff) as f32 / 255.0,
            ((packed >> 8) & 0xff) as f32 / 255.0,
            ((packed >> 16) & 0xff) as f32 / 255.0,
        ];

        // If the script has already rewritten this material wholesale, the
        // repaint belongs in that record; otherwise it is a tint of its own,
        // and everything else about the material stays as authored.
        if let Some(existing) = self.state.materials.borrow_mut().get_mut(&name) {
            existing.base_color = colour;
            return Ok(Value::Empty);
        }
        self.state.tints.borrow_mut().insert(name, colour);
        Ok(Value::Empty)
    }
}

/// `LoadValue(table, name)` — what the table saved last time, or "".
struct LoadValue {
    state: Rc<GameState>,
}

impl Object for LoadValue {
    fn type_name(&self) -> &'static str {
        "LoadValue"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let key = value_key(args)?;
        Ok(Value::str(
            self.state
                .values
                .borrow()
                .get(&key)
                .map_or("", String::as_str),
        ))
    }
}

/// `SaveValue(table, name, value)`.
struct SaveValue {
    state: Rc<GameState>,
}

impl Object for SaveValue {
    fn type_name(&self) -> &'static str {
        "SaveValue"
    }

    fn get(&self, _name: &str, args: &[Value]) -> VbResult<Value> {
        let key = value_key(args)?;
        // Everything comes back out as a string, because that is all the
        // original's storage holds: a table that saves a number and compares
        // it to one later is relying on VBScript's own coercion, not on us
        // keeping the type.
        let text = args.get(2).map_or(Ok(Rc::from("")), Value::to_str)?;
        self.state.values.borrow_mut().insert(key, text.to_string());
        Ok(Value::Empty)
    }
}

/// How a value is filed: the table's name and the value's, folded.
fn value_key(args: &[Value]) -> VbResult<String> {
    let table = args.first().map_or(Ok(Rc::from("")), Value::to_str)?;
    let name = args.get(1).map_or(Ok(Rc::from("")), Value::to_str)?;
    // Folded, because a table that saves "HighScore" and loads "Highscore" is
    // a table VBScript would have answered.
    Ok(format!("{}/{}", table.to_lowercase(), name.to_lowercase()))
}

/// Something that accepts anything and does nothing.
///
/// Used for the corners of the host that are not modelled yet: the audio calls,
/// and the inside of VPinMAME's controller. It answers every read with another
/// one of itself so a chain of dots gets all the way to the end, and reads as
/// zero or an empty string when a table finally uses it as a value.
struct Stub;

impl Object for Stub {
    fn type_name(&self) -> &'static str {
        "Object"
    }

    fn get(&self, _name: &str, _args: &[Value]) -> VbResult<Value> {
        Ok(Value::Object(Rc::new(Stub)))
    }

    fn set(&self, _name: &str, _args: &[Value], _value: Value, _by_ref: bool) -> VbResult<()> {
        Ok(())
    }

    fn default_value(&self) -> VbResult<Value> {
        Ok(Value::Long(0))
    }

    fn enumerate(&self) -> Option<Vec<Value>> {
        Some(Vec::new())
    }
}
