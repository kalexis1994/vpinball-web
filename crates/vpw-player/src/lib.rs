//! The player's wasm entry point.
//!
//! Equivalent to the original's `src/core/player.cpp` + `VPApp`, but with none
//! of the editor: just the game loop.

// This crate only applies to the wasm target; on the host it stays empty so
// that `cargo test --workspace` and `cargo clippy` keep working on the rest.
#![cfg(target_arch = "wasm32")]

pub mod table_api;

use std::cell::RefCell;
use std::rc::Rc;

use std::collections::HashMap;

use vpw_game::controller::RomSource;
use vpw_game::{Game, Resources, ScriptLibrary};
use vpw_physics::FixedStep;
use vpw_render::TableRenderer;
use vpw_render::camera::View;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Self-rescheduling closure that drives `requestAnimationFrame`.
type RafClosure = Closure<dyn FnMut(f64)>;

/// Initialises logging and the panic hook. Idempotent.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
}

/// Loop statistics, so we have numbers from day one: the point of the port is
/// performance, so we measure before optimising.
#[derive(Default)]
struct FrameStats {
    frames: u32,
    physics_ticks: u64,
    window_start_ms: f64,
    /// Last average computed, so the UI can read it.
    last_fps: f64,
    last_tps: f64,
    /// Samples the sound board had made when the window opened, and the rate
    /// worked out from the window that just closed.
    sound_at: u64,
    sound_rate: f64,
}

impl FrameStats {
    /// Accumulates and, once per second, returns (fps, ticks/s) and resets.
    fn tick(&mut self, now_ms: f64, ticks: u32, sound: u64) -> Option<(f64, f64)> {
        self.frames += 1;
        self.physics_ticks += u64::from(ticks);
        if self.window_start_ms == 0.0 {
            self.window_start_ms = now_ms;
            self.sound_at = sound;
            return None;
        }
        let elapsed = (now_ms - self.window_start_ms) / 1000.0;
        if elapsed < 1.0 {
            return None;
        }
        self.last_fps = f64::from(self.frames) / elapsed;
        self.last_tps = self.physics_ticks as f64 / elapsed;
        self.sound_rate = sound.saturating_sub(self.sound_at) as f64 / elapsed;
        self.sound_at = sound;
        self.frames = 0;
        self.physics_ticks = 0;
        self.window_start_ms = now_ms;
        Some((self.last_fps, self.last_tps))
    }
}

/// State of the camera the user drives with the mouse.
#[derive(Default)]
struct CameraControls {
    dragging: bool,
    last: (f32, f32),
}

struct Player {
    renderer: TableRenderer,
    /// The canvas being drawn to, and the pixel ratio it was measured at.
    ///
    /// Owned here rather than captured by the animation-frame closure, because
    /// it can change: leaving the game and coming back builds a new element,
    /// and a closure holding the old one would keep measuring something that is
    /// no longer in the document. See [`start`].
    canvas: web_sys::HtmlCanvasElement,
    dpr: f64,
    /// The table being played, once one is loaded.
    table: Option<Game>,
    clock: FixedStep,
    stats: FrameStats,
    camera_controls: CameraControls,
    /// The segments the display was last drawn from, so it is only redrawn
    /// when the machine says something new.
    display_segments: Vec<u16>,
    /// The same, for the copy handed to the page, which asks at its own size
    /// and its own rate.
    display_sent: Option<(Vec<u16>, u32, u32)>,
    last_ms: f64,
    /// Wall clock at the end of the previous frame's work, so the next one can
    /// say how long we were not running.
    finished_ms: f64,
    elapsed_ticks: u64,
}

impl Player {
    fn frame(&mut self, now_ms: f64) {
        let canvas = self.canvas.clone();
        let (canvas, dpr) = (&canvas, self.dpr);

        // Nothing to do while the canvas is out of the document, which is what
        // it is the whole time the player is in the menu.
        //
        // Not merely a saving. The loop measures the canvas to size the
        // backbuffer, and a detached element measures zero, so every frame
        // spent in the menu was resizing the surface to one pixel by one and
        // rebuilding the offscreen targets to match. And the table went on
        // being played with nobody watching: a ball can drain while somebody
        // reads the table list, and they come back to a game they did not lose.
        //
        // The clocks are held still rather than left to run, or the first frame
        // back owes every millisecond spent in the menu and the loop tries to
        // pay it in one go.
        use wasm_bindgen::JsCast as _;
        if !canvas.unchecked_ref::<web_sys::Node>().is_connected() {
            self.last_ms = now_ms;
            self.finished_ms = clock_ms();
            return;
        }

        // Two clocks, because they answer different questions. `now_ms` is the
        // animation frame's own timestamp: when the frame was **due**. This one
        // is when we actually got to run. The gap between them is how busy the
        // main thread was with something else.
        let entered = clock_ms();
        let idle_ms = (entered - self.finished_ms) as f32;
        let late_ms = (entered - now_ms) as f32;

        // Fit the backbuffer to the current CSS size times the device pixel
        // ratio.
        let w = (f64::from(canvas.client_width()) * dpr).round().max(1.0) as u32;
        let h = (f64::from(canvas.client_height()) * dpr).round().max(1.0) as u32;
        if (canvas.width(), canvas.height()) != (w, h) {
            canvas.set_width(w);
            canvas.set_height(h);
            self.renderer.resize(w, h);
        }

        // Clamped to exactly what the clock is willing to make up for. Not a
        // number of its own: clamping to more than that throws the difference
        // away silently, which is what a physics rate of 820 Hz on a 120 Hz
        // display turned out to be.
        let dt = ((now_ms - self.last_ms) / 1000.0).clamp(0.0, self.clock.max_catch_up_seconds());
        self.last_ms = now_ms;

        let ticks = self.clock.advance(dt);
        self.elapsed_ticks += u64::from(ticks);

        let (mut step_ms, mut sync_ms) = (0.0f32, 0.0f32);
        if let Some(table) = &mut self.table {
            // The physics runs at a fixed 1000 Hz, decoupled from the frame
            // rate, exactly as in the original (`PhysicsEngine::UpdatePhysics`).
            // The worst case is a tab coming back from the background owing a
            // quarter of a second: `dt` is clamped above, so that is 250 steps
            // at once and `FixedStep` caps it at 100 anyway.
            let t0 = clock_ms();
            for _ in 0..ticks {
                table.step();
            }
            // The physics has caught up, so the script may sync with the ROM:
            // `FireTimers(-2)` sits at exactly this point of the original's
            // loop, right after `UpdatePhysics` (`player.cpp:1848`). This is
            // what drives `PinMameTimer`, and it is a per-frame event — not a
            // per-millisecond one.
            table.game_sync();
            // Losing the ball to a tilt has to release the flippers, and the
            // `keyup` that would do it may never come.
            if table.controls.check_tilt(&mut table.engine.borrow_mut()) {
                log::info!("tilt");
            }
            // The new frame event, before anything is drawn and after the
            // parts have been brought up to date — the original's order
            // (`player.cpp:2132`). It is script work like `game_sync` above,
            // so it is timed with the steps and not with the sync.
            table.new_frame();
            let t1 = clock_ms();
            // And only now, once per frame, the state reaches the GPU. The
            // lamps are told how much *table* time the frame was worth so they
            // can fade by it: the physics runs at a fixed 1000 Hz, so a tick is
            // a millisecond and the count is the answer.
            sync(table, &mut self.renderer, ticks as f32);
            draw_display(table, &mut self.renderer, &mut self.display_segments);
            let t2 = clock_ms();
            step_ms = (t1 - t0) as f32;
            sync_ms = (t2 - t1) as f32;
        }

        let t3 = clock_ms();
        if let Err(e) = self.renderer.render() {
            log::error!("the frame render failed: {e}");
        }
        let render_ms = (clock_ms() - t3) as f32;

        // A frame that took too long goes into the rolling record, with where
        // its time went. Only the slow ones: every frame would be a hundred
        // thousand entries in a half-hour window and would bury everything else.
        //
        // The threshold is generous on purpose. Two frames' worth at sixty
        // hertz is past anything a display can hide and is what a player
        // notices as a hitch, while an ordinary frame that ran a little long
        // is not worth a line.
        const SLOW_FRAME_MS: f64 = 34.0;
        if dt * 1000.0 > SLOW_FRAME_MS
            && let Some(table) = &self.table
            && table.telemetry_enabled()
        {
            table.telemetry_frame(
                (dt * 1000.0) as f32,
                ticks,
                step_ms,
                sync_ms,
                render_ms,
                late_ms,
                idle_ms,
            );
        }
        self.finished_ms = clock_ms();

        let made = self
            .table
            .as_ref()
            .map_or(0, |t| t.machine().sound_stats().1);
        if let Some((fps, tps)) = self.stats.tick(now_ms, ticks, made) {
            log::info!(
                "{fps:.1} fps | {tps:.0} physics ticks/s (target 1000) | \
                 sound board {:.0} samples/s (target 24242)",
                self.stats.sound_rate
            );
        }
    }
}

/// The machine's segments, drawn into an image of the given size.
///
/// The one place either destination goes through, so the head in the scene and
/// the canvas on the page can never be showing different things.
fn segment_raster(segments: &[u16], size: (u32, u32)) -> vpw_render::segments::Raster {
    use vpw_render::segments::{Glyph, Row, Style};
    let (rows, columns) = vpw_table::backbox::DISPLAY_GRID;
    // Two rows, laid out one after the other exactly as the machine hands them
    // over: the top one alphanumeric and the bottom one the score.
    let split = (segments.len() / rows.max(1)).min(segments.len());
    vpw_render::segments::draw(
        &[
            Row {
                segments: &segments[..split],
                glyph: Glyph::Alphanumeric,
            },
            Row {
                segments: &segments[split..],
                glyph: Glyph::Numeric,
            },
        ],
        size,
        Style {
            columns,
            ..Default::default()
        },
    )
}

/// Redraws the machine's score onto the head, if it has changed.
///
/// The drawing itself belongs to `vpw_render::segments`, which knows how a
/// digit is shaped and nothing else. This only decides **when**: the segments
/// change a few times a second at most, and redrawing a texture sixty times a
/// second to put the same pixels back is work nobody sees.
fn draw_display(table: &Game, renderer: &mut TableRenderer, last: &mut Vec<u16>) {
    // A machine says its score one of two ways and this is where the two meet:
    // a row of segments, or a panel of dots. Nothing above here knows which
    // kind of machine it is talking to.
    let (dots, width, height) = table.machine().dmd();
    if !dots.is_empty() {
        // Cheap enough to compare: four thousand bytes against a frame time.
        // Worth comparing, because uploading a texture that has not changed is
        // the same picture at the cost of a texture upload.
        let digest: Vec<u16> = dots
            .chunks(64)
            .map(|c| c.iter().map(|&d| u16::from(d)).sum())
            .collect();
        if digest == *last {
            return;
        }
        *last = digest;
        renderer.set_display(&dot_raster(
            &dots,
            width,
            height,
            vpw_table::backbox::DISPLAY_PIXELS,
        ));
        // And the same dots to any flasher the table uses as its display —
        // which is how a 10.8 table puts the DMD on the playfield. Those draw
        // the dots themselves, so they get the dots and not the picture.
        renderer.set_dmd(&dots, width, height);
        return;
    }

    let segments = table.machine().segments();
    if segments.is_empty() || segments == *last {
        return;
    }
    last.clone_from(&segments);
    renderer.set_display(&segment_raster(
        &segments,
        vpw_table::backbox::DISPLAY_PIXELS,
    ));
}

/// Draws a dot matrix into the panel on the machine's head.
///
/// A dot has four levels rather than two — see `vpw_ws::dmd` for how a one-bit
/// panel produces them — so this is not a stencil, it is four brightnesses of
/// the same amber. The dots are drawn as dots, with a gap, because that is what
/// the thing looks like: a solid block of lit pixels reads as a screen and a
/// grid of round lights reads as a machine.
fn dot_raster(
    dots: &[u8],
    width: usize,
    height: usize,
    size: (u32, u32),
) -> vpw_render::segments::Raster {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let mut rgba = vec![0u8; w * h * 4];
    if width == 0 || height == 0 {
        return vpw_render::segments::Raster {
            width: size.0,
            height: size.1,
            rgba,
        };
    }

    // The panel is drawn to fit the width, centred vertically: a 128 by 32
    // display is four times wider than it is tall and the head is not.
    let cell = (w / width).max(1);
    let top = (h.saturating_sub(height * cell)) / 2;
    let left = (w.saturating_sub(width * cell)) / 2;
    // A dot fills most of its cell, so the grid between them stays visible.
    let dot = (cell * 4 / 5).max(1);
    let inset = (cell - dot) / 2;

    for y in 0..height {
        for x in 0..width {
            let level = dots[y * width + x];
            if level == 0 {
                continue;
            }
            // 0 is dark, 3 is full. The steps are what the board itself makes
            // by showing a frame for two thirds of the cycle or one third.
            let scale = f32::from(level) / 3.0;
            let colour = [
                (255.0 * scale) as u8,
                (150.0 * scale) as u8,
                (30.0 * scale) as u8,
                255,
            ];
            for dy in 0..dot {
                let py = top + y * cell + inset + dy;
                if py >= h {
                    continue;
                }
                for dx in 0..dot {
                    let px = left + x * cell + inset + dx;
                    if px >= w {
                        continue;
                    }
                    let at = (py * w + px) * 4;
                    rgba[at..at + 4].copy_from_slice(&colour);
                }
            }
        }
    }

    vpw_render::segments::Raster {
        width: size.0,
        height: size.1,
        rgba,
    }
}

/// Copies the state of the physics into the renderer's matrices.
///
/// Once per frame and not once per physics step: at 1000 Hz against 60 frames
/// per second, writing them every step would be sixteen uploads nobody ever
/// sees.
fn sync(table: &mut Game, renderer: &mut TableRenderer, dt_ms: f32) {
    let queue = renderer.queue().clone();

    // The lamps first, and on their own borrow. This is what a table's lights
    // actually are: the file has them nearly all off, because they are the
    // *game's* lamps and the game turns them on — so a port that does not push
    // the script's state every frame draws a playfield that is permanently
    // dark, however well everything else works.
    {
        let lights = renderer.lights_mut();
        for i in 0..lights.names.len() {
            // Two numbers, because the original keeps two: `State` is the
            // switch — 0 off, 1 on, 2 blinking (`light.cpp:315`) — and
            // `IntensityScale` is the dimmer a script writes every frame while
            // it fades a lamp by hand. `light_level` folds the first into the
            // second, so the switch has to be read separately or a blinking
            // lamp arrives here as a plainly lit one and never blinks.
            //
            // A lamp with no item behind it is left on: it is not the script's
            // to turn off, and the file's own state is already in the lamp.
            let (state, scale) = match table.items().get(&lights.names[i]) {
                Some(item) => {
                    // `light_state` cannot answer two. The script layer stores
                    // a lamp's state as an `i32` it builds with
                    // `i32::from(state != 0.0)` (`vpw-game/src/items.rs:1236`),
                    // so "blinking" arrives here as plain "on" and every
                    // blinking lamp on the table sits permanently lit. Until
                    // that value carries three states, a lamp the *file*
                    // declared a blinker goes back to blinking whenever the
                    // game says it is on — which is what the original does for
                    // any such lamp no script has written to, and the best
                    // available answer for the rest.
                    let state = match item.light_state() {
                        0 => 0.0,
                        _ if lights.blinks(i) => vpw_table::light::BLINKING,
                        n => n as f32,
                    };
                    // `light_level` is zero when the switch is off, whatever
                    // the dimmer says, so the dimmer has to be recovered from
                    // it rather than read through it. With the switch off the
                    // target is zero either way and the scale does not matter.
                    let scale = if item.light_state() == 0 {
                        1.0
                    } else {
                        item.light_level()
                    };
                    (state, scale)
                }
                None => (1.0, 1.0),
            };
            lights.animate(&queue, i, state, scale, dt_ms);
        }
    }

    // The flashers, on the same terms: the file leaves them mostly off, and
    // the game fires them — `core.vbs` toggles `Visible` off the solenoids
    // (`core.vbs:2534`), a table's own script fades `IntensityScale`. The
    // renderer skips a flasher whose numbers have not moved, so handing every
    // one over every frame costs a comparison each.
    {
        let (flashers, device, queue) = renderer.flashers_mut();
        for i in 0..flashers.names.len() {
            let Some(item) = table.items().get(&flashers.names[i]) else {
                continue;
            };
            // `LMAP`: a flasher bound to a lamp fades with it, by the ratio
            // of the lamp's current level to its full one
            // (`flasher.cpp:1171-1177`). The fade itself lives in the light
            // pass, which this cannot read; the switch is here, and a lamp
            // either side of its fade is at the switch's value. A departure,
            // and a small one: a lightmap flasher snaps where its lamp ramps.
            let light_scale = match flashers.light_map(i).and_then(|n| table.items().get(n)) {
                Some(lamp) if lamp.light_state() == 0 => 0.0,
                _ => 1.0,
            };
            flashers.set_state(device, queue, i, &item.flasher_state(), light_scale);
        }
    }

    let Some(dynamic) = renderer.dynamic_mut() else {
        return;
    };
    for i in 0..table.parts().len() {
        // Through the table rather than off the part: a primitive is placed by
        // the script, and only the table knows what the script has written.
        dynamic.set_part_transform(&queue, i, table.part_transform(i));
        dynamic.set_part_visible(i, table.part_visible(i));
    }

    let engine = table.engine.borrow();
    for slot in 0..vpw_render::MAX_BALLS {
        let m = engine
            .balls
            .get(slot)
            .map(|b| vpw_table::ball::transform(b.pos, b.radius, b.orientation, b.locked));
        dynamic.set_ball_transform(&queue, slot, m);
    }
}

/// What the HUD reads out of a running table.
fn hud_of(table: &Game) -> Hud {
    let engine = table.engine.borrow();
    Hud {
        balls: engine.balls.len(),
        tilt: engine.is_tilted(),
        warnings: engine.nudge.warnings,
        tilt_risk: engine.nudge.risk(),
        handlers: table.handlers_fired(),
    }
}

/// A snapshot of the game, for the UI.
#[derive(Debug, Clone, Copy)]
struct Hud {
    balls: usize,
    tilt: bool,
    warnings: u32,
    tilt_risk: f32,
    handlers: u64,
}

thread_local! {
    /// The script libraries the page handed over.
    ///
    /// A table's script pulls in `core.vbs` and its machine's library at run
    /// time, by name. There is no filesystem here, so the page fetches them and
    /// puts them in this registry before a table is loaded.
    static LIBRARIES: RefCell<HashMap<String, Rc<str>>> =
        RefCell::new(HashMap::new());
}

/// Hands the player one of the scripts a table may ask for.
///
/// Call it for every library before `loadTable`. A table that asks for one that
/// is not here still loads, and plays without its rules.
#[wasm_bindgen(js_name = addScriptLibrary)]
pub fn add_script_library(name: String, text: String) {
    LIBRARIES.with(|l| {
        l.borrow_mut()
            .insert(name.to_ascii_lowercase(), Rc::from(text.as_str()))
    });
}

/// The rate the audio comes out at.
///
/// The page should build its `AudioContext` at this rate. A browser will happily
/// resample if it is asked for something else, but doing it here would mean
/// resampling the sound board's stream twice.
#[wasm_bindgen(js_name = audioRate)]
pub fn audio_rate() -> u32 {
    vpw_game::AUDIO_RATE
}

/// Renders `frames` frames of interleaved stereo, ready to hand to an
/// `AudioWorklet`.
///
/// Returns an empty array when no table is loaded, which is the signal to play
/// silence rather than to stop: the graph stays up between tables.
///
/// This is a **pull**: nothing is buffered until the page asks, and asking is
/// what makes a sound advance. A page that stops calling it stops the sound,
/// which is what should happen when its tab goes to the background.
#[wasm_bindgen(js_name = renderAudio)]
pub fn render_audio(frames: usize) -> Vec<f32> {
    // A cap so that a page asking for a silly number cannot allocate the tab
    // out of memory. Ten seconds is far more than any sane buffer.
    let frames = frames.min(vpw_game::AUDIO_RATE as usize * 10);
    PLAYER.with(|p| {
        let player = p.borrow();
        let Some(player) = player.as_ref() else {
            return Vec::new();
        };
        let mut player = player.borrow_mut();
        let Some(table) = player.table.as_mut() else {
            return Vec::new();
        };
        let mut out = vec![0.0; frames * 2];
        table.render_audio(&mut out);
        out
    })
}

/// Presses or releases a key, by the same `KeyboardEvent.code` the keyboard
/// handler uses.
///
/// For touch. A phone has no keyboard, and the on-screen buttons need to reach
/// the same path a real key does — the table's script gets `Table1_KeyDown`
/// either way, and the flippers guard against auto-repeat either way. The
/// alternative, synthesising `KeyboardEvent`s, is a lie the browser only half
/// believes.
#[wasm_bindgen(js_name = pressKey)]
pub fn press_key(code: &str, pressed: bool) {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow_mut().table.as_mut()
        {
            table.key(code, pressed);
        }
    });
}

/// The machine's score display, drawn at the size the caller asks for.
///
/// The **same drawing** that goes onto the head in the scene — one module knows
/// how a digit is shaped, and both destinations ask it. What differs is only
/// where the pixels land: on a texture in the 3D scene, or on a canvas the page
/// puts wherever it likes when the head is not in shot.
///
/// Returns **empty** when the machine has not said anything new since the last
/// call at this size. A megabyte of unchanged pixels ten times a second is a
/// megabyte of unchanged pixels: the caller keeps what it already drew.
#[wasm_bindgen(js_name = displayImage)]
pub fn display_image(width: u32, height: u32) -> Vec<u8> {
    PLAYER.with(|p| {
        let player = p.borrow();
        let Some(player) = player.as_ref() else {
            return Vec::new();
        };
        let mut player = player.borrow_mut();
        let Some(table) = player.table.as_ref() else {
            return Vec::new();
        };
        let segments = table.machine().segments();
        if segments.is_empty() {
            return Vec::new();
        }
        let asked = (segments.clone(), width, height);
        if player.display_sent.as_ref() == Some(&asked) {
            return Vec::new();
        }
        player.display_sent = Some(asked);
        segment_raster(&segments, (width, height)).rgba
    })
}

/// Where the machine's head lands on screen, as `[left, top, width, height]` in
/// fractions of the canvas.
///
/// So the page can draw the score **on** the head rather than at a guessed
/// offset from the top of the window. A guess is wrong the moment the window
/// changes shape, and there is no single guess that is right for two views that
/// frame completely different things.
///
/// Empty when the head is not in shot, which is the signal to put the score
/// somewhere else rather than to hide it.
#[wasm_bindgen(js_name = backboxRect)]
pub fn backbox_rect() -> Vec<f32> {
    PLAYER.with(|p| {
        p.borrow()
            .as_ref()
            .and_then(|player| player.borrow().renderer.backbox_screen_rect())
            .map(|r| r.to_vec())
            .unwrap_or_default()
    })
}

/// The two rows of the machine's score display, as text.
///
/// A System 11 has no screen. It has two rows of fourteen characters — the top
/// one sixteen-segment and alphanumeric, the bottom one seven-segment — and the
/// CPU lights one digit at a time and relies on the eye to hold the rest. What
/// comes back here is a whole sweep's worth, already assembled, which is the
/// only form in which it spells anything.
///
/// Empty strings when there is no machine, which is a table without a ROM: it
/// still rolls a ball, it just has nothing to say.
#[wasm_bindgen(js_name = displays)]
pub fn displays() -> Vec<String> {
    PLAYER.with(|p| {
        let player = p.borrow();
        let Some(player) = player.as_ref() else {
            return Vec::new();
        };
        let player = player.borrow();
        let Some(table) = player.table.as_ref() else {
            return Vec::new();
        };
        let (upper, lower) = table.machine().displays();
        vec![upper, lower]
    })
}

/// Where the player is looking at the machine from.
///
/// A name rather than a number, because it crosses into a page that stores it
/// in `localStorage` and shows it in a menu, and a number there is a number
/// nobody can read six months later.
#[wasm_bindgen(js_name = cameraView)]
pub fn camera_view() -> String {
    PLAYER.with(|p| {
        p.borrow()
            .as_ref()
            .map(|player| view_name(player.borrow().renderer.view()).to_string())
            .unwrap_or_default()
    })
}

/// Moves to one of the named views. Anything unrecognised is ignored.
///
/// Ignored rather than refused: this is fed by a stored setting, and a setting
/// written by an older version of the page — or by somebody poking at
/// `localStorage` — should leave the camera where it is rather than stopping
/// the table from starting.
#[wasm_bindgen(js_name = setCameraView)]
pub fn set_camera_view(name: &str) {
    let Some(view) = view_from_name(name) else {
        log::warn!("unknown camera view '{name}'; leaving it where it is");
        return;
    };
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref() {
            player.borrow_mut().renderer.set_view(view);
        }
    });
}

/// The views there are, in the order a key cycles through them.
///
/// Given to the page so the menu and the key agree without either of them
/// carrying its own copy of the list.
#[wasm_bindgen(js_name = cameraViews)]
pub fn camera_views() -> Vec<String> {
    VIEWS.iter().map(|v| view_name(*v).to_string()).collect()
}

const VIEWS: [View; 2] = [View::Front, View::Overhead];

fn view_name(view: View) -> &'static str {
    match view {
        View::Front => "front",
        View::Overhead => "overhead",
    }
}

fn view_from_name(name: &str) -> Option<View> {
    VIEWS.iter().copied().find(|v| view_name(*v) == name)
}

/// Lets go of everything. For when a touch is cancelled or the player leaves:
/// a flipper held by a finger that is no longer there stays up for ever.
#[wasm_bindgen(js_name = releaseAllKeys)]
pub fn release_all_keys() {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow_mut().table.as_mut()
        {
            table.controls.release_all(&mut table.engine.borrow_mut());
        }
    });
}

/// Master volume, 0 to 1.
#[wasm_bindgen(js_name = setVolume)]
pub fn set_volume(volume: f32) {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow_mut().table.as_mut()
        {
            table.set_volume(volume);
        }
    });
}

/// How many libraries the page has handed over.
#[wasm_bindgen(js_name = scriptLibraryCount)]
pub fn script_library_count() -> usize {
    LIBRARIES.with(|l| l.borrow().len())
}

/// The registry, as the game's script loader sees it.
struct PageLibraries;

impl ScriptLibrary for PageLibraries {
    fn read(&self, name: &str) -> Option<String> {
        LIBRARIES.with(|l| {
            l.borrow()
                .get(&name.to_ascii_lowercase())
                .map(|s| s.to_string())
        })
    }
}

thread_local! {
    /// The ROM zips the page handed over, by set name.
    ///
    /// Same arrangement as the libraries, for the same reason: on a ROM table
    /// the rules live in a firmware image that ships separately from the
    /// `.vpx`, and a browser has nowhere to look for it.
    static ROMS: RefCell<HashMap<String, Rc<[u8]>>> = RefCell::new(HashMap::new());
    /// Saved settings, audits and high scores, by set name.
    static SAVES: RefCell<HashMap<String, Rc<[u8]>>> = RefCell::new(HashMap::new());
}

/// Hands the player a machine's ROM, as the bytes of its `.zip`.
///
/// The name is the set — `f14_l1` — which is what the table's script asks for.
/// Call it before `loadTable`. A ROM table without its ROM still loads and the
/// ball still moves; it just never scores, because nothing is running the game.
#[wasm_bindgen(js_name = addRom)]
pub fn add_rom(set: String, zip: Vec<u8>) {
    ROMS.with(|r| {
        r.borrow_mut()
            .insert(set.to_ascii_lowercase(), Rc::from(zip))
    });
}

/// Restores a machine's battery-backed memory from a previous session.
///
/// Optional, and worth doing: without it the machine starts factory-fresh
/// every time, with no settings and no high scores. Pair it with
/// [`take_machine_state`].
#[wasm_bindgen(js_name = restoreMachineState)]
pub fn restore_machine_state(set: String, data: Vec<u8>) {
    SAVES.with(|s| {
        s.borrow_mut()
            .insert(set.to_ascii_lowercase(), Rc::from(data))
    });
}

/// The current machine's memory, for the page to put in local storage.
///
/// Returns nothing if no ROM is running.
#[wasm_bindgen(js_name = machineState)]
pub fn machine_state() -> Option<Vec<u8>> {
    PLAYER.with(|p| {
        let player = p.borrow();
        let player = player.as_ref()?.borrow();
        let machine = player.table.as_ref()?.machine();
        machine.is_running().then(|| machine.cmos())
    })
}

/// How many ROMs the page has handed over.
#[wasm_bindgen(js_name = romCount)]
pub fn rom_count() -> usize {
    ROMS.with(|r| r.borrow().len())
}

/// The ROM registry, as the controller sees it.
struct PageRoms;

impl RomSource for PageRoms {
    fn read(&self, set: &str) -> Option<Vec<u8>> {
        ROMS.with(|r| {
            r.borrow()
                .get(&set.to_ascii_lowercase())
                .map(|z| z.to_vec())
        })
    }

    fn cmos(&self, set: &str) -> Option<Vec<u8>> {
        SAVES.with(|s| {
            s.borrow()
                .get(&set.to_ascii_lowercase())
                .map(|d| d.to_vec())
        })
    }
}

thread_local! {
    /// The live player. Global because the event handlers and the functions JS
    /// calls all have to reach the same state.
    static PLAYER: RefCell<Option<Rc<RefCell<Player>>>> = const { RefCell::new(None) };
}

/// What the player tells the UI after loading a table.
#[wasm_bindgen]
pub struct SceneStats {
    #[wasm_bindgen(readonly)]
    pub meshes: usize,
    #[wasm_bindgen(readonly)]
    pub vertices: usize,
    #[wasm_bindgen(readonly)]
    pub triangles: usize,
    #[wasm_bindgen(readonly)]
    pub textures: usize,
    #[wasm_bindgen(js_name = drawCalls, readonly)]
    pub draw_calls: usize,
    /// How many draw calls there would be without merging, that is, one per
    /// mesh.
    #[wasm_bindgen(js_name = drawCallsNaive, readonly)]
    pub draw_calls_naive: usize,
    /// Milliseconds each stage of the load took.
    #[wasm_bindgen(js_name = parseMs, readonly)]
    pub parse_ms: f64,
    #[wasm_bindgen(js_name = extractMs, readonly)]
    pub extract_ms: f64,
    #[wasm_bindgen(js_name = uploadMs, readonly)]
    pub upload_ms: f64,
}

/// Frames per second, physics ticks per second, and the state of the game.
#[wasm_bindgen]
pub struct LoopStats {
    #[wasm_bindgen(readonly)]
    pub fps: f64,
    #[wasm_bindgen(js_name = physicsTicksPerSecond, readonly)]
    pub tps: f64,
    /// How many balls are in play.
    #[wasm_bindgen(readonly)]
    pub balls: usize,
    #[wasm_bindgen(readonly)]
    pub tilt: bool,
    /// Tilt warnings so far. The fourth one ends the ball.
    #[wasm_bindgen(readonly)]
    pub warnings: u32,
    /// How close the plumb is to the ring, from 0 to 1.
    #[wasm_bindgen(js_name = tiltRisk, readonly)]
    pub tilt_risk: f32,
    /// How many of the table's own script handlers have run. The one number
    /// that says whether the table's rules are running at all.
    #[wasm_bindgen(js_name = handlerCalls, readonly)]
    pub handler_calls: f64,
    /// Whether the machine's ROM is loaded and executing.
    ///
    /// The one thing that was impossible to see from the outside. A table with
    /// no ROM, or with a ROM this emulator cannot run, loads and renders and
    /// rolls a ball around perfectly — and takes a coin, and starts nothing,
    /// because the rules of the game live on a board that is not there. From
    /// the player's seat that is indistinguishable from a bug, and there was
    /// nothing anywhere on the screen to tell them apart.
    #[wasm_bindgen(js_name = romRunning, readonly)]
    pub rom_running: bool,
    /// Whether this machine's sound board is there, and how many samples a
    /// second it is making.
    ///
    /// Separate from `rom_running` because they fail apart: a game board runs
    /// perfectly out of one image in the zip while the five the sound board
    /// needs are missing, and the result is a machine that plays and says
    /// nothing. And a board that is there but making nothing is a different
    /// fault again from one making its full rate with nothing to say, which is
    /// what a machine in attract mode sounds like.
    #[wasm_bindgen(js_name = soundBoard, readonly)]
    pub sound_board: bool,
    #[wasm_bindgen(js_name = soundRate, readonly)]
    pub sound_rate: f64,
    /// The set that is running, or empty. Answers "which machine is this".
    rom_name: String,
    /// What the machine or the script last said about itself: why a ROM would
    /// not load, mostly. Empty when it has said nothing.
    notice: String,
}

#[wasm_bindgen]
impl LoopStats {
    /// A `String` cannot be a `wasm_bindgen` field, so these are getters. See
    /// the fields for what they are.
    #[wasm_bindgen(getter, js_name = romName)]
    pub fn rom_name(&self) -> String {
        self.rom_name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn notice(&self) -> String {
        self.notice.clone()
    }
}

/// Holds the shooter rod where a finger is holding it, from 0 to 1.
///
/// What a real plunger is: how far it is pulled is the shot. The plunger *key*
/// is a different control and still behaves like a key — held down, it draws
/// the rod back on its own, because a key has no position to give.
#[wasm_bindgen(js_name = holdPlunger)]
pub fn hold_plunger(travel: f32) {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow_mut().table.as_mut()
        {
            table.hold_plunger(travel);
        }
    });
}

/// Lets go of a rod that was being held. The shot comes from where it is.
#[wasm_bindgen(js_name = releasePlunger)]
pub fn release_plunger() {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow_mut().table.as_mut()
        {
            table.let_go_of_plunger();
        }
    });
}

/// How far the shooter rod is drawn back, from 0 to 1.
///
/// `None` before a table is loaded, or on a table with no plunger. It is read
/// every frame by whatever is drawing the on-screen shooter, so it does the
/// least work it can: two borrows and a subtraction.
#[wasm_bindgen(js_name = plungerPull)]
pub fn plunger_pull() -> Option<f32> {
    PLAYER.with(|p| {
        p.borrow()
            .as_ref()
            .and_then(|player| player.borrow().table.as_ref()?.plunger_pull())
    })
}

/// What the UI can read at any time for the HUD.
#[wasm_bindgen(js_name = loopStats)]
pub fn loop_stats() -> Option<LoopStats> {
    PLAYER.with(|p| {
        p.borrow().as_ref().map(|player| {
            let player = player.borrow();
            let s = &player.stats;
            let hud = player.table.as_ref().map(hud_of);
            LoopStats {
                fps: s.last_fps,
                tps: s.last_tps,
                balls: hud.map_or(0, |h| h.balls),
                tilt: hud.is_some_and(|h| h.tilt),
                warnings: hud.map_or(0, |h| h.warnings),
                tilt_risk: hud.map_or(0.0, |h| h.tilt_risk),
                // As `f64` because JS has no `u64`, and `wasm-bindgen` would
                // hand it over as a `BigInt`, which the UI then cannot format.
                handler_calls: hud.map_or(0.0, |h| h.handlers as f64),
                rom_running: player
                    .table
                    .as_ref()
                    .is_some_and(|t| t.machine().is_running()),
                sound_board: player
                    .table
                    .as_ref()
                    .is_some_and(|t| t.machine().sound_stats().0),
                sound_rate: s.sound_rate,
                rom_name: player
                    .table
                    .as_ref()
                    .and_then(|t| t.machine().game_name())
                    .unwrap_or_default()
                    .to_string(),
                // Taken, not peeked: a notice is shown once and then it is the
                // player's problem, not something to repeat sixty times a
                // second for the rest of the session.
                notice: player
                    .table
                    .as_ref()
                    .map(|t| t.take_messages().join(" · "))
                    .unwrap_or_default(),
            }
        })
    })
}

/// Puts a new ball in front of the plunger, clearing any tilt.
#[wasm_bindgen(js_name = newBall)]
pub fn new_ball() {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = &mut player.borrow_mut().table
        {
            table.new_ball();
        }
    });
}

/// Starts or stops keeping the rolling record. See `vpw_game::telemetry`.
///
/// Off by default: half an hour of history is eleven megabytes, and a player
/// who is playing rather than debugging should not be carrying it.
#[wasm_bindgen(js_name = setTelemetry)]
pub fn set_telemetry(on: bool) {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref()
            && let Some(table) = player.borrow().table.as_ref()
        {
            table.set_telemetry(on);
        }
    });
}

/// Whether the record is being kept.
#[wasm_bindgen(js_name = telemetryEnabled)]
pub fn telemetry_enabled() -> bool {
    PLAYER.with(|p| {
        p.borrow()
            .as_ref()
            .and_then(|player| {
                let player = player.borrow();
                player.table.as_ref().map(|t| t.telemetry_enabled())
            })
            .unwrap_or(false)
    })
}

/// The last `seconds` of the record, as JSON, or `None` if no table is loaded.
///
/// `note` is what the host wants said about the moment it was asked for. Table
/// time is what everything inside is stamped with, and table time means nothing
/// to somebody reading the file later, so the wall clock goes in here.
#[wasm_bindgen(js_name = telemetryDump)]
pub fn telemetry_dump(seconds: f64, note: &str) -> Option<String> {
    PLAYER.with(|p| {
        let player = p.borrow();
        let player = player.as_ref()?;
        let player = player.borrow();
        let table = player.table.as_ref()?;
        table.telemetry_mark(note);
        Some(table.telemetry_dump(seconds, note))
    })
}

/// How much history is being held: samples, then edges.
#[wasm_bindgen(js_name = telemetryHeld)]
pub fn telemetry_held() -> Vec<u32> {
    PLAYER.with(|p| {
        p.borrow()
            .as_ref()
            .and_then(|player| {
                let player = player.borrow();
                let table = player.table.as_ref()?;
                let (samples, events) = table.telemetry_held();
                Some(vec![samples as u32, events as u32])
            })
            .unwrap_or_default()
    })
}

/// Records something outside the frame that held the main thread.
///
/// The page does more than step and draw, and the rest of it does not pass
/// through the frame at all: the audio is pumped from an animation frame of its
/// own, so nothing in Rust ever sees how long that took. A hitch reported here
/// is ours; one that shows up nowhere, with the frame idle the whole time, is
/// the browser's.
///
/// Cheap to call and ignored unless the rolling record is on, so the page can
/// hand over anything it thinks was slow without having to decide first.
#[wasm_bindgen(js_name = notePause)]
pub fn note_pause(source: &str, ms: f32) {
    PLAYER.with(|p| {
        let player = p.borrow();
        let Some(player) = player.as_ref() else {
            return;
        };
        let player = player.borrow();
        let Some(table) = player.table.as_ref() else {
            return;
        };
        if table.telemetry_enabled() {
            table.telemetry_pause(source, ms);
        }
    });
}

fn now(window: &web_sys::Window) -> f64 {
    window.performance().map_or(0.0, |p| p.now())
}

/// The same clock without a window to hand.
///
/// `std::time::Instant` panics on `wasm32-unknown-unknown`, so the browser's
/// own is the only one there is. Browsers coarsen it deliberately against
/// timing attacks — a fraction of a millisecond, typically — which is far below
/// anything worth calling a hitch.
fn clock_ms() -> f64 {
    web_sys::window().map_or(0.0, |w| now(&w))
}

/// Starts the player on the `<canvas>` with the given id.
///
/// Calling it again is safe, and it is called again: the two cases are the
/// same call and they are told apart by the canvas element, not by the id.
///
/// **The same element.** React in strict mode mounts every effect twice in
/// development, and building a second surface on a canvas that already has one
/// leaves the promise hanging for ever. Nothing happens.
///
/// **A different element.** Leaving the game for the menu unmounts the canvas
/// and coming back builds a new one — same id, different object. A surface is
/// bound to the element it was made from, so without noticing this the
/// renderer carries on drawing into a canvas that is no longer in the document:
/// the sound plays, the controls are there, and the table is a black rectangle.
/// The renderer is pointed at the new element and everything already uploaded
/// stays where it is, so coming back from the menu costs nothing.
#[wasm_bindgen]
pub async fn start(canvas_id: String) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window object"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let canvas: web_sys::HtmlCanvasElement = document
        .get_element_by_id(&canvas_id)
        .ok_or_else(|| JsValue::from_str(&format!("there is no canvas #{canvas_id}")))?
        .dyn_into()
        .map_err(|_| JsValue::from_str(&format!("#{canvas_id} is not a <canvas>")))?;

    let dpr = window.device_pixel_ratio();
    let width = (f64::from(canvas.client_width()) * dpr).round().max(1.0) as u32;
    let height = (f64::from(canvas.client_height()) * dpr).round().max(1.0) as u32;
    canvas.set_width(width);
    canvas.set_height(height);

    if let Some(player) = PLAYER.with(|p| p.borrow().clone()) {
        // `Object::is` and not a comparison of ids: the id is what they have in
        // common, and the element is what differs.
        let same = {
            let running = player.borrow();
            js_sys::Object::is(canvas.as_ref(), running.canvas.as_ref())
        };
        if same {
            log::info!("the player is already on this canvas");
            return Ok(());
        }

        {
            let mut running = player.borrow_mut();
            running
                .renderer
                .attach(wgpu::SurfaceTarget::Canvas(canvas.clone()), width, height)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            running.canvas = canvas.clone();
            running.dpr = dpr;
            // The clock has been running while nobody was looking. Without
            // this the first frame back owes every millisecond spent in the
            // menu and the loop tries to pay it all at once.
            running.last_ms = now(&window);
            running.finished_ms = running.last_ms;
        }
        // Only the canvas half. The old element's listeners died with it, but
        // the keyboard's are on `window` and are still there — connecting them
        // again would deliver every keypress twice, and once more for every
        // trip through the menu after that.
        connect_pointer(&canvas, &player)?;
        log::info!("the player moved to a new canvas ({width}x{height})");
        return Ok(());
    }

    let renderer = TableRenderer::new(wgpu::SurfaceTarget::Canvas(canvas.clone()), width, height)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let player = Rc::new(RefCell::new(Player {
        renderer,
        canvas: canvas.clone(),
        dpr,
        table: None,
        clock: FixedStep::default(),
        stats: FrameStats::default(),
        camera_controls: CameraControls::default(),
        display_segments: Vec::new(),
        display_sent: None,
        last_ms: now(&window),
        finished_ms: now(&window),
        elapsed_ticks: 0,
    }));
    PLAYER.with(|p| *p.borrow_mut() = Some(player.clone()));

    log::info!("WebGPU ready ({width}x{height})");
    connect_controls(&canvas, &player)?;

    // One synchronous frame before entering the loop: it avoids a blank canvas
    // until the first rAF and leaves the backbuffer at the real size the layout
    // has already resolved.
    player.borrow_mut().frame(now(&window));

    // The rAF loop: the closure reschedules itself.
    let handle: Rc<RefCell<Option<RafClosure>>> = Rc::new(RefCell::new(None));
    let scheduler = handle.clone();
    *handle.borrow_mut() = Some(Closure::wrap(Box::new(move |now_ms: f64| {
        player.borrow_mut().frame(now_ms);

        if let (Some(win), Some(cb)) = (web_sys::window(), scheduler.borrow().as_ref()) {
            let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut(f64)>));

    window.request_animation_frame(handle.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;
    // The closure has to outlive this function; it reschedules itself.
    std::mem::forget(handle);

    Ok(())
}

/// Loads a table into the already started player.
///
/// It takes the raw `.vpx`. Parsing and uploading can take hundreds of
/// milliseconds on a big table, so the UI had better show something before
/// calling.
#[wasm_bindgen(js_name = loadTable)]
pub fn load_table(bytes: &[u8]) -> Result<SceneStats, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window object"))?;

    let t0 = now(&window);
    let vpx = vpin::vpx::from_bytes(bytes)
        .map_err(|e| JsValue::from_str(&format!("could not read the .vpx: {e}")))?;
    let t1 = now(&window);
    let mut scene = vpw_table::geometry::extract(&vpx);
    let t2 = now(&window);

    PLAYER.with(|p| {
        let borrow = p.borrow();
        let player = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("the player is not started"))?;
        let mut player = player.borrow_mut();

        // The physics comes out of the same parse as the geometry. Building the
        // table also takes the moving parts **out** of `scene`, so this has to
        // happen before the scene is uploaded.
        // The libraries a table's script pulls in — `core.vbs` and the rest —
        // come from the page, which fetched them and handed them over through
        // `addScriptLibrary` before this ran.
        let resources = Resources::new(Rc::new(PageLibraries)).with_roms(Rc::new(PageRoms));
        let mut table = Game::load(&vpx, &mut scene, resources)
            .map_err(|e| JsValue::from_str(&format!("the table's script failed: {e}")))?;
        if let Err(e) = table.start() {
            log::error!("the table's Init failed: {e}");
        }
        // No ball. On a ROM table the machine serves one when somebody puts a
        // coin in and presses start, and putting one in the lane before that
        // is a ball the game does not know about: it sits there through the
        // attract mode and the machine still serves its own when a game
        // begins. A table with no ROM has `newBall` for the same purpose.
        if !table.machine().is_running() {
            table.new_ball();
        }

        player.renderer.load_with_parts(&scene, table.parts());
        // Zero milliseconds: this is the first sync of a table that has not
        // run yet, so a lamp's fade has nothing to advance over — it should
        // start wherever the file left it, not somewhere along a ramp.
        sync(&mut table, &mut player.renderer, 0.0);
        let t3 = now(&window);

        {
            let engine = table.engine.borrow();
            log::info!(
                "physics: {} shapes, {} triggers, {} moving parts, {} script items",
                engine.shapes().len(),
                engine.triggers().len(),
                table.parts().len(),
                table.items().len(),
            );
        }
        player.table = Some(table);

        let s = player
            .renderer
            .stats()
            .ok_or_else(|| JsValue::from_str("the scene did not end up loaded"))?;

        log::info!(
            "table loaded: {} meshes, {} triangles, {} draw calls (one per mesh would be {})",
            s.meshes,
            s.triangles,
            s.draw_calls,
            s.draw_calls_naive
        );

        Ok(SceneStats {
            meshes: s.meshes,
            vertices: s.vertices,
            triangles: s.triangles,
            textures: s.textures,
            draw_calls: s.draw_calls,
            draw_calls_naive: s.draw_calls_naive,
            parse_ms: t1 - t0,
            extract_ms: t2 - t1,
            upload_ms: t3 - t2,
        })
    })
}

/// Hooks up dragging and the wheel to move the camera.
///
/// It is provisional: once the original's `ViewSetup` is in, the camera will be
/// fixed by the table and this stays only for inspection mode.
fn connect_controls(
    canvas: &web_sys::HtmlCanvasElement,
    player: &Rc<RefCell<Player>>,
) -> Result<(), JsValue> {
    connect_pointer(canvas, player)?;
    connect_keyboard(player)?;
    Ok(())
}

/// The listeners that belong to the canvas, which is the half that has to be
/// done again when the canvas changes.
///
/// Kept apart from the keyboard on purpose. These die with the element they are
/// on, so re-attaching to a new canvas has to redo them; the keyboard's are on
/// `window`, which outlives every canvas, and doing *those* again would leave
/// two sets of handlers delivering every keypress to the table twice — and
/// another set for every trip through the menu.
fn connect_pointer(
    canvas: &web_sys::HtmlCanvasElement,
    player: &Rc<RefCell<Player>>,
) -> Result<(), JsValue> {
    use web_sys::{MouseEvent, WheelEvent};

    let p = player.clone();
    let down = Closure::wrap(Box::new(move |e: MouseEvent| {
        let mut p = p.borrow_mut();
        p.camera_controls.dragging = true;
        p.camera_controls.last = (e.client_x() as f32, e.client_y() as f32);
    }) as Box<dyn FnMut(MouseEvent)>);
    canvas.add_event_listener_with_callback("mousedown", down.as_ref().unchecked_ref())?;
    down.forget();

    let p = player.clone();
    let up = Closure::wrap(Box::new(move |_: MouseEvent| {
        p.borrow_mut().camera_controls.dragging = false;
    }) as Box<dyn FnMut(MouseEvent)>);
    canvas.add_event_listener_with_callback("mouseup", up.as_ref().unchecked_ref())?;
    canvas.add_event_listener_with_callback("mouseleave", up.as_ref().unchecked_ref())?;
    up.forget();

    let p = player.clone();
    let mouse_move = Closure::wrap(Box::new(move |e: MouseEvent| {
        let mut p = p.borrow_mut();
        if !p.camera_controls.dragging {
            return;
        }
        let (x, y) = (e.client_x() as f32, e.client_y() as f32);
        let (dx, dy) = (x - p.camera_controls.last.0, y - p.camera_controls.last.1);
        p.camera_controls.last = (x, y);
        p.renderer.camera.azimuth -= dx * 0.3;
        p.renderer.camera.inclination = (p.renderer.camera.inclination + dy * 0.3).clamp(5.0, 89.0);
    }) as Box<dyn FnMut(MouseEvent)>);
    canvas.add_event_listener_with_callback("mousemove", mouse_move.as_ref().unchecked_ref())?;
    mouse_move.forget();

    let p = player.clone();
    let wheel = Closure::wrap(Box::new(move |e: WheelEvent| {
        e.prevent_default();
        let mut p = p.borrow_mut();
        let factor = if e.delta_y() > 0.0 { 1.1 } else { 1.0 / 1.1 };
        p.renderer.camera.distance = (p.renderer.camera.distance * factor).clamp(100.0, 50_000.0);
    }) as Box<dyn FnMut(WheelEvent)>);
    canvas.add_event_listener_with_callback("wheel", wheel.as_ref().unchecked_ref())?;
    wheel.forget();
    Ok(())
}

/// Wires the keyboard to the table.
///
/// The listeners go on `window` and not on the canvas: a `<canvas>` does not
/// take focus unless it is given a `tabindex`, so keys aimed at it would end up
/// nowhere. The flip side is that they also fire while the menu is open, which
/// is harmless because with no table loaded there is nothing to move.
///
/// # Why the `keyup` on `blur` matters
///
/// Alt-tabbing away with a flipper held down means the `keyup` is delivered to
/// whatever window took the focus. Without releasing everything on `blur`, you
/// come back to a table with a flipper standing up and no way to lower it.
fn connect_keyboard(player: &Rc<RefCell<Player>>) -> Result<(), JsValue> {
    use web_sys::KeyboardEvent;

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window object"))?;

    let key = |event: &str, pressed: bool| -> Result<(), JsValue> {
        let p = player.clone();
        let handler = Closure::wrap(Box::new(move |e: KeyboardEvent| {
            // Auto-repeat is not a new press. The controls guard against it
            // anyway, but leaving it out saves fifteen calls a second per key.
            if pressed && e.repeat() {
                return;
            }
            let mut player = p.borrow_mut();
            let Some(table) = &mut player.table else {
                return;
            };
            if pressed && e.code() == "Enter" {
                table.new_ball();
                e.prevent_default();
                return;
            }
            if table.key(&e.code(), pressed) {
                // Space scrolls the page and the arrows move the caret; a table
                // that jumps every time you nudge is unplayable.
                e.prevent_default();
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);
        window.add_event_listener_with_callback(event, handler.as_ref().unchecked_ref())?;
        handler.forget();
        Ok(())
    };
    key("keydown", true)?;
    key("keyup", false)?;

    let p = player.clone();
    let blur = Closure::wrap(Box::new(move |_: web_sys::Event| {
        if let Some(table) = &mut p.borrow_mut().table {
            table.controls.release_all(&mut table.engine.borrow_mut());
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    window.add_event_listener_with_callback("blur", blur.as_ref().unchecked_ref())?;
    blur.forget();

    Ok(())
}
