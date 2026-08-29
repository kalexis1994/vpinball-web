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

/// The player's own console logger, replacing the `console_log` crate for
/// two reasons it could not be configured around. It has no per-module
/// filter, so opening the gate to our trace chatter also let naga narrate
/// every WGSL overload resolution — pages of it per shader. And it mapped
/// `debug` to `console.log`, which Chrome *shows*; here everything below
/// `info` goes to `console.debug`, the one channel Chrome keeps behind its
/// Verbose switch. Foreign crates speak only when something is wrong.
struct ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.target().starts_with("vpw") || meta.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let msg = wasm_bindgen::JsValue::from_str(&format!("{}", record.args()));
        match record.level() {
            log::Level::Error => web_sys::console::error_1(&msg),
            log::Level::Warn => web_sys::console::warn_1(&msg),
            log::Level::Info => web_sys::console::info_1(&msg),
            log::Level::Debug | log::Level::Trace => web_sys::console::debug_1(&msg),
        }
    }

    fn flush(&self) {}
}

/// Initialises logging and the panic hook. Idempotent.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    static LOGGER: ConsoleLogger = ConsoleLogger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
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
    /// Milliseconds the render half of each frame took, summed over the
    /// window, and the average from the window that just closed. The number a
    /// rendering change has to answer to: fps saturates at the display's
    /// refresh and hides everything below it.
    render_ms_sum: f64,
    render_ms_avg: f64,
    /// Seconds reported since the last loud one. Every tenth window speaks at
    /// info: once a second is a wall, once in ten is a pulse.
    windows: u32,
}

impl FrameStats {
    /// Accumulates and, once per second, returns (fps, ticks/s) and resets.
    fn tick(&mut self, now_ms: f64, ticks: u32, sound: u64, render_ms: f32) -> Option<(f64, f64)> {
        self.frames += 1;
        self.physics_ticks += u64::from(ticks);
        self.render_ms_sum += f64::from(render_ms);
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
        self.render_ms_avg = self.render_ms_sum / f64::from(self.frames.max(1));
        self.render_ms_sum = 0.0;
        self.sound_at = sound;
        self.frames = 0;
        self.physics_ticks = 0;
        self.window_start_ms = now_ms;
        Some((self.last_fps, self.last_tps))
    }
}

/// Where the frames land.
///
/// Two shapes, because the player runs in two places. On the **main thread**
/// it draws into the page's own `<canvas>`: layout owns the size, so the
/// element is measured every frame, and "nobody can see it" is a fact the
/// element itself knows. In a **worker** it draws into the `OffscreenCanvas`
/// the page transferred: there is no layout to ask and no document to be in,
/// so the size is whatever the page last said and visibility is told rather
/// than observed.
enum Surface {
    Dom {
        /// Owned here rather than captured by the animation-frame closure,
        /// because it can change: leaving the game and coming back builds a
        /// new element, and a closure holding the old one would keep measuring
        /// something that is no longer in the document. See [`start`].
        canvas: web_sys::HtmlCanvasElement,
        dpr: f64,
    },
    Offscreen {
        canvas: web_sys::OffscreenCanvas,
        /// Set through [`set_visible`]. While false the loop holds the clocks,
        /// exactly as the main-thread path does for a detached canvas.
        visible: bool,
    },
}

struct Player {
    renderer: TableRenderer,
    surface: Surface,
    /// The table being played, once one is loaded.
    table: Option<Game>,
    clock: FixedStep,
    stats: FrameStats,
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
    /// The size the page asked for, before the governor's scale.
    full_size: (u32, u32),
    /// Which rung of [`SCALES`] the renderer is on. See [`Player::govern`].
    scale_tier: usize,
    /// Seconds spent comfortably under budget, before daring to climb.
    calm: u32,
    /// The fps a step-down was taken at, when that step was taken on the
    /// fps signal and still awaits judgement. See [`Player::govern`].
    judging: Option<f64>,
    /// Seconds the governor sits out after concluding the display, not the
    /// GPU, is what limits the rate.
    frozen: u32,
    /// Seconds since the last climb, saturating. A climb that is followed
    /// within a few seconds by a drop is a climb that should not have been
    /// tried, and trying it again every few seconds is a screen that blinks.
    climbed_ago: u32,
    /// Seconds left of the no-climbing sentence handed down when a climb
    /// bounced. The ladder has found its level; let it stand.
    climb_lockout: u32,
    /// Whether the governor runs at all. Off, the resolution is the
    /// player's to choose and the ladder stands at full; some players would
    /// rather have a slow sharp picture than a fast soft one, and that is
    /// their call to make.
    adaptive: bool,
    /// Whether the game is held still. See [`set_paused`].
    paused: bool,
}

/// The rungs of the quality ladder, as fractions of the canvas size.
///
/// The first step down costs no pixels at all: it turns the playfield's
/// reflection probe off (see [`Player::apply_scale`]). The probe is a second
/// pass over the whole table's geometry, and on a tile-based mobile GPU —
/// which walks every triangle twice, once to bin and once to draw — it is
/// the single most expensive thing on the ladder while being the least
/// visible: a machine on a Snapdragon was geometry-bound, and shrinking the
/// picture was costing sharpness without buying frames. Losing the mirror
/// under the ball is the better first trade everywhere.
///
/// From there the payoff is quadratic: the bottom rung draws sixteen percent
/// of the pixels the top one does, which is the difference between fourteen
/// frames a second on an integrated GPU and sixty. The composite stretches
/// the smaller picture over the canvas, and a moving pinball hides the
/// softness far better than it hides a slideshow.
const SCALES: [f32; 6] = [1.0, 1.0, 0.85, 0.7, 0.55, 0.4];

impl Player {
    /// Whether there is anything to draw for, and the backbuffer size to draw
    /// at if it changed.
    ///
    /// `None` means nobody could see the frame: the canvas is out of the
    /// document, or the page said the player is hidden. Not merely a saving.
    /// The main-thread loop measures the canvas to size the backbuffer, and a
    /// detached element measures zero, so every frame spent in the menu was
    /// resizing the surface to one pixel by one and rebuilding the offscreen
    /// targets to match. And the table went on being played with nobody
    /// watching: a ball can drain while somebody reads the table list, and
    /// they come back to a game they did not lose.
    fn visible_size(&self) -> Option<Option<(u32, u32)>> {
        match &self.surface {
            Surface::Dom { canvas, dpr } => {
                use wasm_bindgen::JsCast as _;
                if !canvas.unchecked_ref::<web_sys::Node>().is_connected() {
                    return None;
                }
                // Fit the backbuffer to the current CSS size times the device
                // pixel ratio. The quality ladder no longer touches this:
                // scaling lives inside the renderer, where it cannot blink.
                let w = (f64::from(canvas.client_width()) * dpr).round().max(1.0) as u32;
                let h = (f64::from(canvas.client_height()) * dpr).round().max(1.0) as u32;
                if (canvas.width(), canvas.height()) != (w, h) {
                    canvas.set_width(w);
                    canvas.set_height(h);
                    Some(Some((w, h)))
                } else {
                    Some(None)
                }
            }
            // An offscreen canvas is resized by [`resize_surface`] when the
            // page tells it to; the frame has nothing to measure.
            Surface::Offscreen { visible, .. } => visible.then_some(None),
        }
    }

    /// Trades pixels for frames, once a second, on the evidence.
    ///
    /// The render average is the number that matters: fps saturates at the
    /// display's refresh and physics runs on its own clock, so only the
    /// render half says whether the GPU is drowning. Over budget it steps
    /// down a rung of [`SCALES`] immediately — a slideshow is the worst
    /// picture quality there is — and it climbs back one rung only after
    /// three comfortable seconds, so a table that sits at the edge does not
    /// shimmer between sizes.
    fn govern(&mut self) {
        // `VPW_TIER` on the global scope pins the ladder to one rung, so the
        // governor can be exercised from any machine's console:
        // `globalThis.VPW_TIER = 4` in the worker is the slowest GPU there is.
        if let Ok(v) = js_sys::Reflect::get(&js_sys::global(), &"VPW_TIER".into())
            && let Some(t) = v.as_f64()
        {
            let t = (t as usize).min(SCALES.len() - 1);
            if t != self.scale_tier {
                self.scale_tier = t;
                log::trace!("pinned to {:.0}% resolution", SCALES[t] * 100.0);
                self.apply_scale();
            }
            return;
        }
        if !self.adaptive {
            return;
        }
        // Two-thirds of a sixty-hertz frame: the render is not the frame's
        // only tenant, and a budget that spends all sixteen milliseconds on
        // drawing leaves nothing for the physics and the browser.
        const OVER_MS: f64 = 11.0;
        const UNDER_MS: f64 = 5.0;
        // The fps thresholds are the second signal, and the one that matters
        // on a weak GPU: `render_ms` is the cost of *encoding* the frame,
        // and a queue submitted in two milliseconds can still take the GPU
        // thirty to draw — which shows up nowhere but the frame rate.
        const SLOW_FPS: f64 = 50.0;
        const COMFY_FPS: f64 = 57.0;

        if self.frozen > 0 {
            self.frozen -= 1;
            return;
        }
        self.climbed_ago = self.climbed_ago.saturating_add(1);
        self.climb_lockout = self.climb_lockout.saturating_sub(1);
        let avg = self.stats.render_ms_avg;
        let fps = self.stats.last_fps;

        // Judgement on the previous fps-driven step: if pixels came off and
        // no frames came back while the encode stayed cheap, the limit is
        // the display's refresh, not the GPU — put the pixels back and stop
        // pestering a machine that was never the problem.
        if let Some(before) = self.judging.take()
            && fps < before + 3.0
            && avg < UNDER_MS
            && self.scale_tier > 0
        {
            self.scale_tier -= 1;
            log::trace!(
                "no frames gained: back to {:.0}% and standing down for a minute",
                SCALES[self.scale_tier] * 100.0
            );
            self.apply_scale();
            self.frozen = 60;
            return;
        }

        let slow = avg > OVER_MS || (fps > 1.0 && fps < SLOW_FPS);
        let comfy = avg < UNDER_MS && fps > COMFY_FPS;
        if slow && self.scale_tier + 1 < SCALES.len() {
            self.scale_tier += 1;
            self.calm = 0;
            if avg <= OVER_MS {
                self.judging = Some(fps);
            }
            // A drop right after a climb convicts the climb: this rung was
            // tried and did not hold, and every retry is a visible blink —
            // the surface reconfigures with the canvas. No climbing for a
            // good while; the ladder has found its level.
            if self.climbed_ago <= 5 {
                self.climb_lockout = 300;
                log::trace!("that resolution did not hold; staying down for a while");
            }
            log::trace!(
                "{fps:.0} fps, render {avg:.1} ms: dropping to {:.0}% resolution",
                SCALES[self.scale_tier] * 100.0
            );
            self.apply_scale();
        } else if comfy && self.scale_tier > 0 && self.climb_lockout == 0 {
            self.calm += 1;
            // Ten comfortable seconds, not three: every transition is a
            // surface reconfigure the player sees as a blink, so the climb
            // has to be worth the flash.
            if self.calm >= 10 {
                self.scale_tier -= 1;
                self.calm = 0;
                self.climbed_ago = 0;
                log::trace!(
                    "{fps:.0} fps, render {avg:.1} ms: back up to {:.0}% resolution",
                    SCALES[self.scale_tier] * 100.0
                );
                self.apply_scale();
            }
        } else {
            self.calm = 0;
        }
    }

    /// Puts the new rung into effect, inside the renderer: the scene buffers
    /// shrink and the composite stretches them back over a surface that
    /// never changes — a scale change used to resize the canvas, and every
    /// resize reconfigured the surface, which the player saw as a blink.
    /// From the third rung down the playfield's reflection probe goes too:
    /// it is a second pass over the whole scene, and its vertex work is the
    /// cost that fewer pixels cannot shrink.
    fn apply_scale(&mut self) {
        // The probe goes first and comes back last: its vertex work is the
        // cost fewer pixels cannot shrink, and its absence is the softest
        // loss on the ladder. See [`SCALES`].
        self.renderer.set_reflection_enabled(self.scale_tier == 0);
        self.renderer.set_render_scale(SCALES[self.scale_tier]);
    }

    fn frame(&mut self, now_ms: f64) {
        // Paused: the clocks are held exactly as they are for a table nobody
        // is looking at, and for the same reason. Nothing is drawn either,
        // which is what leaves the last frame standing on the canvas — the
        // pause menu wants the table behind it, stopped, not black.
        if self.paused {
            self.last_ms = now_ms;
            self.finished_ms = clock_ms();
            return;
        }
        // The clocks are held still rather than left to run while nobody can
        // see the table, or the first frame back owes every millisecond spent
        // in the menu and the loop tries to pay it in one go.
        let Some(resized) = self.visible_size() else {
            self.last_ms = now_ms;
            self.finished_ms = clock_ms();
            return;
        };
        if let Some((w, h)) = resized {
            self.renderer.resize(w, h);
        }

        // Two clocks, because they answer different questions. `now_ms` is the
        // animation frame's own timestamp: when the frame was **due**. This one
        // is when we actually got to run. The gap between them is how busy the
        // thread was with something else.
        let entered = clock_ms();
        let idle_ms = (entered - self.finished_ms) as f32;
        let late_ms = (entered - now_ms) as f32;

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
                log::trace!("tilt");
            }
            // The machine's flipper relay has the last word: between balls
            // and in the menus the flippers go dead — which the flipper
            // *sound* already did, hanging off the same relay, and a flipper
            // that moves silently is a flipper the machine said no to.
            let powered = table.machine().flippers_enabled();
            table
                .controls
                .set_flippers_powered(&mut table.engine.borrow_mut(), powered);
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
        if let Some((fps, tps)) = self.stats.tick(now_ms, ticks, made, render_ms) {
            self.govern();
            // The HUD reads these numbers through `loopStats`; the console
            // copy lives behind Chrome's Verbose switch, a pulse every ten
            // seconds and the rest every second.
            self.stats.windows += 1;
            let level = if self.stats.windows.is_multiple_of(10) {
                log::Level::Debug
            } else {
                log::Level::Trace
            };
            log::log!(
                level,
                "{fps:.1} fps | {tps:.0} physics ticks/s (target 1000) | render {:.2} ms avg | \
                 sound board {:.0} samples/s (target 24242)",
                self.stats.render_ms_avg,
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
/// A dot has a level rather than a bit — see `vpw_ws::dmd::Pwm` for how a
/// one-bit panel produces shades and how they are smoothed — so this is not a
/// stencil, it is brightnesses of the same amber. The dots are drawn as dots, with a gap, because that is what
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
            // 0 is dark, 255 is full, and everything between is the board's
            // own flicker averaged the way an eye averages it — see
            // `vpw_ws::dmd::Pwm`. Not four steps: a game animates by moving a
            // dot between the two frames, and four steps of that strobe.
            let scale = f32::from(level) / 255.0;
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

    // The same phosphor halo the segment display gets: a dot matrix is the
    // same gas behind the same glass, and the dots are round for the same
    // reason. See `vpw_render::segments::bloom`.
    vpw_render::segments::bloom(&mut rgba, size.0, size.1, 0.5, [255, 150, 30]);

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
            lights.animate(i, state, scale, dt_ms);
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
    with_player(|player| {
        let Some(table) = player.table.as_mut() else {
            return Vec::new();
        };
        let mut out = vec![0.0; frames * 2];
        table.render_audio(&mut out);
        out
    })
    .unwrap_or_default()
}

/// One frame by hand: physics, machine, render — exactly what the rAF loop
/// runs, callable when rAF will not fire. A hidden tab freezes
/// `requestAnimationFrame` and with it the whole player, which makes every
/// "does it actually run" question unanswerable from a harness; this is the
/// answer's crank. `__vpw.call('pump')` from the console turns it.
#[wasm_bindgen]
pub fn pump() {
    with_player(|player| player.frame(clock_ms()));
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
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
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
pub fn display_image(width: u32, height: u32, refresh: bool) -> Vec<u8> {
    with_player(|player| {
        let Some(table) = player.table.as_ref() else {
            return Vec::new();
        };
        // A machine says its score one of two ways, and the floating panel
        // deserves both: a row of segments, or a panel of dots. The dots are
        // digested rather than kept — four thousand bytes against a frame
        // time — and either way an unchanged display costs nothing to ask.
        //
        // `refresh` bypasses the unchanged-answer skip for a caller that has
        // nothing on its canvas yet. The skip remembers only that *somebody*
        // was told, and in development React mounts every component twice: the
        // first mount's poll collects the picture, dies, and the mount that
        // stays would otherwise wait for the machine to change a display that
        // — on a game-over screen — holds still for minutes.
        let (dots, dw, dh) = table.machine().dmd();
        if !dots.is_empty() {
            let digest: Vec<u16> = dots
                .chunks(64)
                .map(|c| c.iter().map(|&d| u16::from(d)).sum())
                .collect();
            let asked = (digest, width, height);
            if !refresh && player.display_sent.as_ref() == Some(&asked) {
                return Vec::new();
            }
            player.display_sent = Some(asked);
            return dot_raster(&dots, dw, dh, (width, height)).rgba;
        }
        let segments = table.machine().segments();
        if segments.is_empty() {
            return Vec::new();
        }
        let asked = (segments.clone(), width, height);
        if !refresh && player.display_sent.as_ref() == Some(&asked) {
            return Vec::new();
        }
        player.display_sent = Some(asked);
        segment_raster(&segments, (width, height)).rgba
    })
    .unwrap_or_default()
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
/// Where the playfield lands on screen, as fractions, or empty when there is
/// no table. The page uses it to park the score panel in the gutter beside
/// the table instead of on top of it.
#[wasm_bindgen(js_name = playfieldRect)]
pub fn playfield_rect() -> Vec<f32> {
    with_player(|player| player.renderer.playfield_screen_rect().map(|r| r.to_vec()))
        .flatten()
        .unwrap_or_default()
}

#[wasm_bindgen(js_name = backboxRect)]
pub fn backbox_rect() -> Vec<f32> {
    with_player(|player| player.renderer.backbox_screen_rect().map(|r| r.to_vec()))
        .flatten()
        .unwrap_or_default()
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
    with_player(|player| {
        let Some(table) = player.table.as_ref() else {
            return Vec::new();
        };
        let (upper, lower) = table.machine().displays();
        vec![upper, lower]
    })
    .unwrap_or_default()
}

/// Where the player is looking at the machine from.
///
/// A name rather than a number, because it crosses into a page that stores it
/// in `localStorage` and shows it in a menu, and a number there is a number
/// nobody can read six months later.
#[wasm_bindgen(js_name = cameraView)]
pub fn camera_view() -> String {
    with_player(|player| view_name(player.renderer.view()).to_string()).unwrap_or_default()
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
    with_player(|player| player.renderer.set_view(view));
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
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
            table.controls.release_all(&mut table.engine.borrow_mut());
        }
    });
}

/// How loud the machine's sound board is: its music, its speech and the
/// game's own effects, against the table's mechanical noise. 0 to 1, and it
/// sits under the master rather than beside it.
#[wasm_bindgen(js_name = setMachineVolume)]
pub fn set_machine_volume(volume: f32) {
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
            table.set_machine_volume(volume);
        }
    });
}

/// And the table's own: the bumpers, the flippers, the ball on the wood.
#[wasm_bindgen(js_name = setTableVolume)]
pub fn set_table_volume(volume: f32) {
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
            table.set_table_volume(volume);
        }
    });
}

/// Master volume, 0 to 1.
#[wasm_bindgen(js_name = setVolume)]
pub fn set_volume(volume: f32) {
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
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
    with_player(|player| {
        let machine = player.table.as_ref()?.machine();
        machine.is_running().then(|| machine.cmos())
    })
    .flatten()
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

    /// Whether the wedged-player message has been printed. Once is a clue;
    /// sixty times a second is a wall.
    static WEDGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Says, once, that the player is beyond saving and why the console looks the
/// way it does.
fn wedged_notice() {
    if !WEDGED.with(|w| w.replace(true)) {
        log::error!(
            "the player's state is still borrowed from a frame that never \
             finished — a panic above this line is the real fault, and \
             everything after it is aftermath"
        );
    }
}

/// Runs `f` on the live player, or on nobody.
///
/// **Every** export that touches the player goes through here — that is the
/// rule, and it is about what the console looks like after a disaster. A
/// panic mid-frame aborts the wasm instance without unwinding, so the frame's
/// borrow is never returned and the cell stays borrowed forever; every export
/// that then demands the borrow panics in turn, several times a second, and
/// the one error that names the real fault scrolls away under a wall of
/// "already borrowed". Trying the borrow instead turns all of that into one
/// line and silence.
fn with_player<R>(f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    PLAYER.with(|p| {
        let outer = p.try_borrow().ok()?;
        let player = outer.as_ref()?;
        match player.try_borrow_mut() {
            Ok(mut player) => Some(f(&mut player)),
            Err(_) => {
                wedged_notice();
                None
            }
        }
    })
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
    /// Which rung of the quality ladder the governor is standing on: 0 is
    /// everything on at full resolution. On screen it is the difference
    /// between "the machine is slow" and "the machine is slow *with the
    /// ladder already at the bottom*", which are different bug reports.
    #[wasm_bindgen(js_name = qualityTier, readonly)]
    pub quality_tier: usize,
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
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
            table.hold_plunger(travel);
        }
    });
}

/// Lets go of a rod that was being held. The shot comes from where it is.
#[wasm_bindgen(js_name = releasePlunger)]
pub fn release_plunger() {
    with_player(|player| {
        if let Some(table) = player.table.as_mut() {
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
    with_player(|player| player.table.as_ref()?.plunger_pull()).flatten()
}

/// What the UI can read at any time for the HUD.
#[wasm_bindgen(js_name = loopStats)]
pub fn loop_stats() -> Option<LoopStats> {
    with_player(|player| {
        {
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
                quality_tier: player.scale_tier,
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
        }
    })
}

/// Puts a new ball in front of the plunger, clearing any tilt.
#[wasm_bindgen(js_name = newBall)]
pub fn new_ball() {
    with_player(|player| {
        if let Some(table) = &mut player.table {
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
    with_player(|player| {
        if let Some(table) = player.table.as_ref() {
            table.set_telemetry(on);
        }
    });
}

/// Whether the record is being kept.
#[wasm_bindgen(js_name = telemetryEnabled)]
pub fn telemetry_enabled() -> bool {
    with_player(|player| player.table.as_ref().map(|t| t.telemetry_enabled()))
        .flatten()
        .unwrap_or(false)
}

/// The last `seconds` of the record, as JSON, or `None` if no table is loaded.
///
/// `note` is what the host wants said about the moment it was asked for. Table
/// time is what everything inside is stamped with, and table time means nothing
/// to somebody reading the file later, so the wall clock goes in here.
#[wasm_bindgen(js_name = telemetryDump)]
pub fn telemetry_dump(seconds: f64, note: &str) -> Option<String> {
    with_player(|player| {
        let table = player.table.as_ref()?;
        table.telemetry_mark(note);
        Some(table.telemetry_dump(seconds, note))
    })
    .flatten()
}

/// How much history is being held: samples, then edges.
#[wasm_bindgen(js_name = telemetryHeld)]
pub fn telemetry_held() -> Vec<u32> {
    with_player(|player| {
        let table = player.table.as_ref()?;
        let (samples, events) = table.telemetry_held();
        Some(vec![samples as u32, events as u32])
    })
    .flatten()
    .unwrap_or_default()
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
    with_player(|player| {
        let Some(table) = player.table.as_ref() else {
            return;
        };
        if table.telemetry_enabled() {
            table.telemetry_pause(source, ms);
        }
    });
}

thread_local! {
    /// The scope's `performance`, looked up once.
    ///
    /// Through the global and not through `Window`, because there is no
    /// `Window` in a worker and the clock is the same object under either
    /// name. `std::time::Instant` panics on `wasm32-unknown-unknown`, so the
    /// browser's own is the only one there is. Browsers coarsen it
    /// deliberately against timing attacks — a fraction of a millisecond,
    /// typically — which is far below anything worth calling a hitch.
    static PERFORMANCE: Option<web_sys::Performance> =
        js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("performance"))
            .ok()
            .and_then(|p| p.dyn_into::<web_sys::Performance>().ok());
}

fn clock_ms() -> f64 {
    PERFORMANCE.with(|p| p.as_ref().map_or(0.0, |p| p.now()))
}

/// Schedules `cb` for the next animation frame of whichever thread this is:
/// the window's on the main thread, the worker's own in a worker.
///
/// Looked up by name on the global rather than through the `web_sys` types,
/// because the same loop runs under both globals and the function is the same
/// function either way. A scope without one cannot run the loop at all — which
/// is what the page's capability probe is for, so by the time this runs the
/// answer is known.
fn request_frame(cb: &js_sys::Function) {
    let global = js_sys::global();
    match js_sys::Reflect::get(&global, &JsValue::from_str("requestAnimationFrame")) {
        Ok(f) if f.is_function() => {
            let _ = f.unchecked_into::<js_sys::Function>().call1(&global, cb);
        }
        _ => log::error!("no requestAnimationFrame in this scope; the loop cannot run"),
    }
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

    // Capped at two for the same reason the worker path caps it: a phone's
    // three-and-more device pixels per CSS pixel is more picture than the
    // eye gets back, at more than double the fill cost.
    let dpr = window.device_pixel_ratio().min(2.0);
    let width = (f64::from(canvas.client_width()) * dpr).round().max(1.0) as u32;
    let height = (f64::from(canvas.client_height()) * dpr).round().max(1.0) as u32;
    canvas.set_width(width);
    canvas.set_height(height);

    if let Some(player) = PLAYER.with(|p| p.borrow().clone()) {
        // `Object::is` and not a comparison of ids: the id is what they have in
        // common, and the element is what differs.
        let same = {
            let running = player.borrow();
            match &running.surface {
                Surface::Dom { canvas: c, .. } => js_sys::Object::is(canvas.as_ref(), c.as_ref()),
                Surface::Offscreen { .. } => false,
            }
        };
        if same {
            log::trace!("the player is already on this canvas");
            return Ok(());
        }
        reattach(
            &player,
            wgpu::SurfaceTarget::Canvas(canvas.clone()),
            Surface::Dom { canvas, dpr },
            width,
            height,
        )?;
        return Ok(());
    }

    let renderer = TableRenderer::new(wgpu::SurfaceTarget::Canvas(canvas.clone()), width, height)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    log::trace!("WebGPU ready ({width}x{height})");
    install_and_run(renderer, Surface::Dom { canvas, dpr });
    Ok(())
}

/// Starts the player in a worker, on the `OffscreenCanvas` the page handed
/// over.
///
/// The worker half of [`start`]: the same player and the same loop, with none
/// of the DOM. The page keeps the `<canvas>` element, transfers control of it
/// here, and stays responsible for everything only it can see — the element's
/// size on the layout ([`resize_surface`]), whether anyone is looking at it
/// ([`set_visible`]), and the input events, which arrive through the same
/// exports the touch controls already use.
#[wasm_bindgen(js_name = startOffscreen)]
pub async fn start_offscreen(
    canvas: web_sys::OffscreenCanvas,
    width: u32,
    height: u32,
) -> Result<(), JsValue> {
    let (width, height) = (width.max(1), height.max(1));
    canvas.set_width(width);
    canvas.set_height(height);

    if let Some(player) = PLAYER.with(|p| p.borrow().clone()) {
        // No "already on this canvas" case: the page only transfers a canvas
        // once, so a second start is always a new element after a trip
        // through the menu.
        reattach(
            &player,
            wgpu::SurfaceTarget::OffscreenCanvas(canvas.clone()),
            Surface::Offscreen {
                canvas,
                visible: true,
            },
            width,
            height,
        )?;
        return Ok(());
    }

    let renderer = TableRenderer::new(
        wgpu::SurfaceTarget::OffscreenCanvas(canvas.clone()),
        width,
        height,
    )
    .await
    .map_err(|e| JsValue::from_str(&e.to_string()))?;
    log::trace!("WebGPU ready in a worker ({width}x{height})");
    install_and_run(
        renderer,
        Surface::Offscreen {
            canvas,
            visible: true,
        },
    );
    Ok(())
}

/// Points the running player at a new surface.
///
/// Leaving the game for the menu unmounts the canvas and coming back builds a
/// new one — same id, different object. A surface is bound to the element it
/// was made from, so without this the renderer carries on drawing into a
/// canvas that is no longer in the document: the sound plays, the controls are
/// there, and the table is a black rectangle. Everything already uploaded
/// stays where it is, so coming back from the menu costs nothing.
fn reattach(
    player: &Rc<RefCell<Player>>,
    target: wgpu::SurfaceTarget<'static>,
    surface: Surface,
    width: u32,
    height: u32,
) -> Result<(), JsValue> {
    let mut running = player.try_borrow_mut().map_err(|_| {
        wedged_notice();
        JsValue::from_str("the player is wedged after an earlier crash; reload the page")
    })?;
    running
        .renderer
        .attach(target, width, height)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    running.surface = surface;
    // The clock has been running while nobody was looking. Without this the
    // first frame back owes every millisecond spent in the menu and the loop
    // tries to pay it all at once.
    running.last_ms = clock_ms();
    running.finished_ms = running.last_ms;
    log::trace!("the player moved to a new canvas ({width}x{height})");
    Ok(())
}

/// Installs the player as the live one and spawns its frame loop.
fn install_and_run(renderer: TableRenderer, surface: Surface) {
    let player = Rc::new(RefCell::new(Player {
        renderer,
        surface,
        table: None,
        clock: FixedStep::default(),
        stats: FrameStats::default(),
        display_segments: Vec::new(),
        display_sent: None,
        last_ms: clock_ms(),
        finished_ms: clock_ms(),
        elapsed_ticks: 0,
        full_size: (0, 0),
        scale_tier: 0,
        calm: 0,
        judging: None,
        frozen: 0,
        climbed_ago: u32::MAX,
        climb_lockout: 0,
        adaptive: true,
        paused: false,
    }));
    PLAYER.with(|p| *p.borrow_mut() = Some(player.clone()));

    // One synchronous frame before entering the loop: it avoids a blank canvas
    // until the first rAF and leaves the backbuffer at the real size the layout
    // has already resolved.
    player.borrow_mut().frame(clock_ms());

    // The rAF loop: the closure reschedules itself.
    let handle: Rc<RefCell<Option<RafClosure>>> = Rc::new(RefCell::new(None));
    let scheduler = handle.clone();
    *handle.borrow_mut() = Some(Closure::wrap(Box::new(move |now_ms: f64| {
        // `try_borrow` for the same reason as `with_player`: after a panic
        // mid-frame the borrow is never coming back, and the loop's job then
        // is to stop, not to add a panic per frame to the pile.
        match player.try_borrow_mut() {
            Ok(mut player) => player.frame(now_ms),
            Err(_) => {
                wedged_notice();
                return;
            }
        }
        if let Some(cb) = scheduler.borrow().as_ref() {
            request_frame(cb.as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut(f64)>));

    request_frame(handle.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    // The closure has to outlive this function; it reschedules itself.
    std::mem::forget(handle);
}

/// Resizes an offscreen surface to the size the page measured.
///
/// The worker path's half of what the main-thread loop does by measuring the
/// canvas: layout lives on the page, so the page watches the element with a
/// `ResizeObserver` and reports here in device pixels. A no-op on the
/// main-thread path, which measures for itself.
#[wasm_bindgen(js_name = resizeSurface)]
pub fn resize_surface(width: u32, height: u32) {
    with_player(|player| {
        let (width, height) = (width.max(1), height.max(1));
        match &player.surface {
            Surface::Offscreen { canvas, .. } => {
                canvas.set_width(width);
                canvas.set_height(height);
            }
            Surface::Dom { .. } => return,
        }
        player.full_size = (width, height);
        player.renderer.resize(width, height);
    });
}

/// Tells an offscreen player whether anyone can see it.
///
/// The worker cannot know: the element whose visibility matters lives on the
/// page. While hidden the loop holds the clocks and draws nothing, exactly as
/// the main-thread path does when its canvas leaves the document — a ball must
/// not drain while somebody reads the table list.
#[wasm_bindgen(js_name = setVisible)]
pub fn set_visible(visible: bool) {
    with_player(|player| {
        if let Surface::Offscreen { visible: v, .. } = &mut player.surface {
            *v = visible;
        }
    });
}

/// Orbits the camera by a mouse drag, in pixels.
///
/// The page owns the listeners — there is no DOM in a worker — and hands the
/// deltas here, where the feel lives: how many degrees a pixel is worth is a
/// property of the camera, not of whichever thread caught the event.
///
/// Provisional, like the drag it replaces: once the original's `ViewSetup` is
/// in, the camera will be fixed by the table and this stays only for
/// inspection.
#[wasm_bindgen(js_name = cameraOrbit)]
pub fn camera_orbit(_dx: f32, _dy: f32) {
    // The camera is a place to stand, not a thing to drag: every view is a
    // framed composition, and orbiting out of one leaves a picture nobody
    // designed. The export stays so old pages do not crash calling it.
}

/// Traces the GI lightmaps for a table, away from any player.
///
/// The bake worker's entry point: it runs in a worker of its own with a wasm
/// instance of its own, because tens of millions of rays on the game's thread
/// would be a visible stutter. The answer is everything the cache stores and
/// [`apply_gi_bake`] takes: the layers as one buffer of half floats, and each
/// group's lamp names, which is how a bake finds its lamps again in a scene
/// it has never met.
/// Boots the game headless and asks the machine which lamps switch together.
///
/// `observe_seconds` of table time, after a warm-up long enough for attract
/// to get going. Needs the script libraries and — for a table with one — the
/// ROM to already be in this instance's registries, exactly as `loadTable`
/// does. Empty when the game will not run, which is the caller's cue to fall
/// back to guessing from names and colours.
fn observe_groups(
    vpx: &vpin::vpx::VPX,
    candidates: &[String],
    observe_seconds: f32,
) -> Vec<Vec<String>> {
    if observe_seconds <= 0.0 || candidates.is_empty() {
        return Vec::new();
    }
    // A scene of its own: `Game::load` takes the moving parts out of the one
    // it is given, and the bake wants the scene whole.
    let mut scene = vpw_table::geometry::extract(vpx);
    let resources = Resources::new(Rc::new(PageLibraries)).with_roms(Rc::new(PageRoms));
    let Ok(mut game) = Game::load(vpx, &mut scene, resources) else {
        return Vec::new();
    };
    if game.start().is_err() {
        return Vec::new();
    }
    // Attract needs a moment to begin saying anything.
    const WARMUP_S: f32 = 10.0;
    for _ in 0..(WARMUP_S * 1000.0) as u32 {
        game.step();
    }
    game.game_sync();
    vpw_game::grouping::observe_lamp_groups(&mut game, candidates, observe_seconds)
}

#[wasm_bindgen(js_name = bakeGi)]
pub fn bake_gi(bytes: &[u8], observe_seconds: f32) -> Result<JsValue, JsValue> {
    let vpx = vpin::vpx::from_bytes(bytes)
        .map_err(|e| JsValue::from_str(&format!("could not read the .vpx: {e}")))?;
    let scene = vpw_table::geometry::extract(&vpx);

    // The machine's own answer first: run it and watch what switches
    // together. Names and colours only when there is nothing to watch —
    // no ROM handed over, or a game that will not start.
    let candidates = vpw_render::bake::field_scale_candidates(&scene);
    let observed = observe_groups(&vpx, &candidates, observe_seconds);
    let groups = if observed.is_empty() {
        log::trace!("bake: grouping by names and colours (no machine to watch)");
        vpw_render::bake::gi_groups(&scene)
    } else {
        log::trace!(
            "bake: the machine grouped {} lamps into {} groups",
            observed.iter().map(Vec::len).sum::<usize>(),
            observed.len()
        );
        vpw_render::bake::gi_groups_from_names(&scene, &observed)
    };
    let out = js_sys::Object::new();
    if groups.is_empty() {
        js_sys::Reflect::set(&out, &"layers".into(), &JsValue::from_f64(0.0))?;
        return Ok(out.into());
    }
    let bake = vpw_render::bake::bake_gi_set(&scene, &groups, vpw_render::bake::INDIRECT_SAMPLES);

    let mut data: Vec<u8> = Vec::new();
    for layer in &bake.layers {
        for &texel in layer {
            data.extend_from_slice(&texel.to_le_bytes());
        }
    }
    let names = js_sys::Array::new();
    for group in &groups {
        let list = js_sys::Array::new();
        for name in &group.names {
            list.push(&JsValue::from_str(name));
        }
        names.push(&list);
    }
    js_sys::Reflect::set(&out, &"width".into(), &JsValue::from_f64(bake.width as f64))?;
    js_sys::Reflect::set(
        &out,
        &"height".into(),
        &JsValue::from_f64(bake.height as f64),
    )?;
    js_sys::Reflect::set(
        &out,
        &"layers".into(),
        &JsValue::from_f64(bake.layers.len() as f64),
    )?;
    js_sys::Reflect::set(&out, &"data".into(), &js_sys::Uint8Array::from(&data[..]))?;
    js_sys::Reflect::set(&out, &"groups".into(), &names)?;
    Ok(out.into())
}

/// Installs a traced bake into the running player — this session's, or one
/// the cache kept from an earlier visit.
#[wasm_bindgen(js_name = applyGiBake)]
pub fn apply_gi_bake(
    width: u32,
    height: u32,
    layers: u32,
    data: &[u8],
    groups: js_sys::Array,
) -> Result<(), JsValue> {
    let per_layer = (width * height * 4) as usize;
    let expect = per_layer * layers as usize * 2;
    if data.len() != expect || layers == 0 {
        return Err(JsValue::from_str(&format!(
            "bake data is {} bytes where {expect} were expected",
            data.len()
        )));
    }
    let mut split = Vec::with_capacity(layers as usize);
    for l in 0..layers as usize {
        let bytes = &data[l * per_layer * 2..(l + 1) * per_layer * 2];
        split.push(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect::<Vec<u16>>(),
        );
    }
    let bake = vpw_render::bake::GiBakeSet {
        width,
        height,
        layers: split,
    };
    let group_names: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            js_sys::Array::from(&g)
                .iter()
                .filter_map(|n| n.as_string())
                .collect()
        })
        .collect();
    with_player(|player| {
        player.renderer.apply_gi_bake(&bake, &group_names);
    });
    Ok(())
}

/// The player's day/night, 0 to 1, or any negative number for the table's own.
///
/// The renderer replaces the table's global emission scale with it, exactly as
/// the original's user mode does (`Renderer.cpp:377-398`). Plenty of tables
/// are authored dark on purpose — F-14 asks for 8% — and how dark a room the
/// player sits in is the player's call, which is why the original carries the
/// very same override in its settings.
/// Puts the table in a room, or back in its own light.
///
/// `bytes` is a Radiance `.hdr` equirectangular map the page fetched — a real
/// room, dim walls and a few bright lamps, which is what the steel and the
/// plastics then reflect. Empty bytes restore the environment the table asked
/// for. Returns whether anything changed; before a table has loaded there is
/// nothing to light and the answer is no.
#[wasm_bindgen(js_name = setEnvironment)]
pub fn set_environment(bytes: &[u8]) -> bool {
    with_player(|player| {
        player
            .renderer
            .set_environment((!bytes.is_empty()).then_some(bytes))
    })
    .unwrap_or(false)
}

/// Holds the game still, or lets it go on.
///
/// A real pause and not a hidden one: the physics stops, the machine stops,
/// nothing is drawn, and the clocks are held where they are so the first
/// frame back does not owe the loop every millisecond spent in the menu. The
/// last frame stays on the canvas, which is what the pause menu stands over.
#[wasm_bindgen(js_name = setPaused)]
pub fn set_paused(on: bool) {
    with_player(|player| {
        if player.paused == on {
            return;
        }
        player.paused = on;
        // Coming back, the clock starts from now: the time spent in the menu
        // belongs to nobody.
        if !on {
            player.last_ms = clock_ms();
            player.finished_ms = player.last_ms;
        }
    });
}

/// Draws one frame of the whole machine, seen from the front.
///
/// For the library's card: the picture a table shows before anybody has
/// played it. Plenty of `.vpx` files carry a screenshot of their own and
/// those are used as they are, but plenty carry none, and a shelf of blank
/// cards is a shelf you cannot read.
///
/// It lands on the canvas like any other frame and the next frame replaces
/// it, so the caller has to be holding the loop still and take the canvas
/// immediately — see the `shoot` op in the worker, which does both.
#[wasm_bindgen(js_name = shoot)]
pub fn shoot() -> bool {
    with_player(|player| player.renderer.shoot().is_ok()).unwrap_or(false)
}

/// Ends the game and lets go of the table.
///
/// What "quit" means, and it has to mean this: the machine, the script, the
/// ball in play and the uploaded scene are all dropped, so nothing is left
/// running behind a menu and coming back starts the table fresh rather than
/// resuming somebody else's ball. The player itself stays — one wasm
/// instance, one canvas, for the life of the page — but it stays empty and
/// paused, which costs a cleared frame and nothing else.
#[wasm_bindgen(js_name = stopTable)]
pub fn stop_table() {
    with_player(|player| {
        player.table = None;
        player.renderer.unload();
        player.display_segments.clear();
        player.display_sent = None;
        player.paused = true;
    });
}

/// Switches the resolution governor. Off, the ladder snaps back to full
/// resolution and stays there: the player has said they would rather have
/// every pixel than every frame.
#[wasm_bindgen(js_name = setAdaptive)]
pub fn set_adaptive(on: bool) {
    with_player(|player| {
        player.adaptive = on;
        if !on && player.scale_tier != 0 {
            player.scale_tier = 0;
            player.calm = 0;
            player.judging = None;
            player.apply_scale();
        }
    });
}

/// Switches the flat engine — the table photographed and played as pictures,
/// for a machine whose GPU cannot afford the real render. The bake runs a few
/// lamps per frame while the 3D renderer keeps playing, and the frame flips
/// to the photographs when the last lamp is done. See `vpw_render::flat`.
#[wasm_bindgen(js_name = setFlat)]
pub fn set_flat(on: bool) {
    with_player(|player| {
        player.renderer.set_flat(on);
        // The photographs deserve every pixel: whatever rung the governor
        // was on when the mode arrived, the flat frame is cheap enough for
        // the top one, and a bake taken at 55% resolution stays soft for
        // ever after.
        if on && player.scale_tier != 0 {
            player.scale_tier = 0;
            player.calm = 0;
            player.judging = None;
            player.apply_scale();
        }
    });
}

#[wasm_bindgen(js_name = setDayNight)]
pub fn set_day_night(scale: f32) {
    with_player(|player| {
        player
            .renderer
            .set_day_night((scale >= 0.0).then_some(scale));
    });
}

/// Zooms the camera by one wheel notch, in or out.
#[wasm_bindgen(js_name = cameraZoom)]
pub fn camera_zoom(_out: bool) {
    // Fixed, like the orbit above: the framing is the view's own.
}

/// Loads a table into the already started player.
///
/// It takes the raw `.vpx`. Parsing and uploading can take hundreds of
/// milliseconds on a big table, so the UI had better show something before
/// calling.
#[wasm_bindgen(js_name = loadTable)]
pub fn load_table(bytes: &[u8]) -> Result<SceneStats, JsValue> {
    let t0 = clock_ms();
    let vpx = vpin::vpx::from_bytes(bytes)
        .map_err(|e| JsValue::from_str(&format!("could not read the .vpx: {e}")))?;
    let t1 = clock_ms();
    let mut scene = vpw_table::geometry::extract(&vpx);
    let t2 = clock_ms();

    PLAYER.with(|p| {
        let borrow = p.borrow();
        let player = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("the player is not started"))?;
        let mut player = player.try_borrow_mut().map_err(|_| {
            wedged_notice();
            JsValue::from_str("the player is wedged after an earlier crash; reload the page")
        })?;

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
        let t3 = clock_ms();

        {
            let engine = table.engine.borrow();
            log::trace!(
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

        log::trace!(
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
