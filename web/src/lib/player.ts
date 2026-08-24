// Startup of the wasm player, with a memory of what was already done.
//
// React in strict mode mounts every effect **twice** in development. With
// nothing to stop it, that means downloading the table twice —111 MB each
// time— and parsing it twice. The second pass blocks the main thread for
// several seconds and leaves the UI frozen, even though the canvas already
// shows the table loaded by the first one.
//
// The fix is not a React trick: the wasm player is a real singleton —it lives
// in a `thread_local` of the module— so starting it and loading a table into
// it are operations that only make sense once. This module keeps that promise
// around and reuses it.

import { readMachineState, readRom, writeMachineState } from './library';
import { CAMERA_VIEWS, type CameraView } from './settings';
import type { ParsedTable, RomInfo } from './types';

type Wasm = typeof import('../wasm/vpw_player.js');

export interface LoadStats {
  meshes: number;
  vertices: number;
  triangles: number;
  textures: number;
  drawCalls: number;
  drawCallsNaive: number;
  parseMs: number;
  extractMs: number;
  uploadMs: number;
  bytes: number;
  fetchMs: number;
}

let wasmReady: Promise<Wasm> | null = null;
/** The module once it has resolved, for the few reads that cannot afford to
 * await. See {@link plungerPull}. */
let live: Wasm | null = null;
/** The wasm module and its script libraries: fetched once, whatever happens
 * to the canvas afterwards. See {@link startPlayer}. */
let ready: Promise<Wasm> | null = null;
/** Resolves once the player has been started at least once. The calls below
 * that only make sense against a running player wait on this. */
let started: Promise<void> | null = null;
/** Key of the loaded table, so the work is not repeated. */
let loaded: { key: string; stats: Promise<LoadStats> } | null = null;

/** Loads and initialises the wasm module. Idempotent. */
export function initWasm(): Promise<Wasm> {
  wasmReady ??= (async () => {
    const wasm = await import('../wasm/vpw_player.js');
    const url = (await import('../wasm/vpw_player_bg.wasm?url')).default;
    await wasm.default({ module_or_path: url });
    live = wasm;
    return wasm;
  })();
  return wasmReady;
}

/**
 * Holds the shooter rod where a finger is holding it, from 0 at rest to 1 drawn
 * all the way back.
 *
 * Synchronous, and for the same reason as {@link plungerPull}: it is called
 * from a pointer-move handler, which can fire many times a frame.
 */
export function holdPlunger(travel: number): void {
  live?.holdPlunger(travel);
}

/** Lets go of a rod that was being held. The shot comes from where it is. */
export function releasePlunger(): void {
  live?.releasePlunger();
}

/**
 * How far the shooter rod is drawn back, from 0 to 1, or `null` if there is
 * nothing to ask.
 *
 * Synchronous, unlike everything else here, because it is read once per
 * animation frame by the on-screen plunger and a promise per frame to fetch one
 * float is a promise per frame too many. It answers `null` until the module has
 * resolved, which is exactly the period during which there is no table anyway.
 */
export function plungerPull(): number | null {
  return live?.plungerPull() ?? null;
}

/**
 * Visual Pinball's script library, bundled with the app.
 *
 * A table's script is not self-contained: it opens by pulling in `core.vbs`
 * and the library for its machine — `s11.vbs`, `sam.vbs` — **by name, at run
 * time**. In Visual Pinball those are files in a Scripts folder. There is no
 * folder here, so they are bundled and handed to the player before any table
 * is loaded. Without them a table loads and rolls a ball around, and nothing
 * scores.
 *
 * They are GPL-3.0, the same licence as this project, and come from Visual
 * Pinball's own `scripts/` directory.
 */
const LIBRARIES = import.meta.glob('../scripts/*.vbs', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Hands the bundled libraries to the player. Idempotent. */
async function provideLibraries(): Promise<void> {
  const wasm = await initWasm();
  if (wasm.scriptLibraryCount() > 0) return;
  for (const [path, text] of Object.entries(LIBRARIES)) {
    const name = path.slice(path.lastIndexOf('/') + 1);
    wasm.addScriptLibrary(name, text);
  }
}

/**
 * Hands the player the ROM this table needs, if the library has it.
 *
 * It has to happen before `loadTable`, because the table's script asks for its
 * machine while it is being loaded — `Controller.Run` runs inside `Table1_Init`.
 * A table whose ROM is missing still loads: the ball rolls, the flippers work,
 * and nothing scores, which is the honest outcome rather than a failure.
 */
async function provideRom(wasm: Wasm, rom: RomInfo | undefined): Promise<void> {
  // Both of these used to be silent, and both leave the player with a table
  // that renders, rolls a ball, takes a coin and starts nothing — because the
  // rules of the game are on a board that was never handed over.
  if (!rom?.name) {
    console.warn('[player] this table did not say which ROM it needs');
    return;
  }
  const zip = (await readRom(rom.name)) ?? (await devRom(rom.name));
  if (!zip) {
    console.warn(`[player] the ROM "${rom.name}" is not loaded; import ${rom.name}.zip`);
    return;
  }
  wasm.addRom(rom.name, zip);

  // And the machine's own memory, so it is the same machine as last time,
  // settings and high scores included.
  const saved = await readMachineState(rom.name);
  if (saved) wasm.restoreMachineState(rom.name, saved);
  runningSet = rom.name;
}

/**
 * The ROM the dev server has, for the `/debug` route.
 *
 * Only in development, and only as a fallback: it is what makes going to
 * `/debug` give you a whole machine without loading anything into IndexedDB
 * first. In a build this is dead code and the bundler drops it.
 */
async function devRom(set: string): Promise<Uint8Array | null> {
  if (!import.meta.env.DEV) return null;
  try {
    const r = await fetch(`/debug-assets/roms/${set}.zip`);
    if (!r.ok) return null;
    return new Uint8Array(await r.arrayBuffer());
  } catch {
    return null;
  }
}

/** The set the player is running, for saving its memory afterwards. */
let runningSet: string | null = null;

/**
 * Saves the machine's memory back to the library.
 *
 * Worth calling when the player stops or the page is hidden: the alternative
 * is a machine that forgets its high scores every time.
 */
export async function saveMachineState(): Promise<void> {
  if (!runningSet) return;
  const wasm = await initWasm();
  const data = wasm.machineState();
  if (data) await writeMachineState(runningSet, data);
}

/**
 * Starts the player on the canvas, or moves it there.
 *
 * Every call reaches `wasm.start`, which is what makes coming back from the
 * menu work: React unmounts the canvas on the way out and builds a **new**
 * element on the way back, and only the wasm side can see that the element
 * changed. Memoising this away — which it used to do — left the renderer
 * drawing into the canvas from the first visit, so the second one had sound and
 * controls and a black rectangle where the table should be.
 *
 * What is memoised is the expensive half: fetching the wasm and handing over
 * the script libraries. Those are the same whatever canvas is in front of us.
 */
export function startPlayer(canvasId: string): Promise<void> {
  ready ??= (async () => {
    const wasm = await initWasm();
    await provideLibraries();
    return wasm;
  })();
  const attached = ready.then((wasm) => wasm.start(canvasId));
  started ??= attached;
  return attached;
}

/**
 * Fetches a table and loads it into the player.
 *
 * `key` identifies the table: asking twice for the same one returns the
 * promise from the first call instead of redoing the work.
 */
export function loadTable(
  key: string,
  fetchBytes: () => Promise<Uint8Array>,
  rom?: RomInfo,
): Promise<LoadStats> {
  if (loaded?.key === key) return loaded.stats;

  const stats = (async () => {
    const wasm = await initWasm();
    await provideRom(wasm, rom);
    const t0 = performance.now();
    const bytes = await fetchBytes();
    const fetchMs = performance.now() - t0;

    // A breather so the browser can paint the notice before it locks up:
    // parsing a 100 MB table is synchronous and takes a while.
    //
    // With `setTimeout` and not with `requestAnimationFrame`: rAF **does not
    // fire** while the tab is not visible, so awaiting it leaves the load hung
    // forever if the user opened the table in a background tab.
    await new Promise((r) => setTimeout(r, 0));

    const s = wasm.loadTable(bytes);
    return {
      meshes: s.meshes,
      vertices: s.vertices,
      triangles: s.triangles,
      textures: s.textures,
      drawCalls: s.drawCalls,
      drawCallsNaive: s.drawCallsNaive,
      parseMs: s.parseMs,
      extractMs: s.extractMs,
      uploadMs: s.uploadMs,
      bytes: bytes.byteLength,
      fetchMs,
    };
  })();

  loaded = { key, stats };
  // If it fails, forget it so it can be retried.
  stats.catch(() => {
    if (loaded?.key === key) loaded = null;
  });
  return stats;
}

/** What the HUD shows: the loop's numbers plus the state of the game. */
export interface Loop {
  fps: number;
  /** Physics ticks per second. The target is 1000. */
  tps: number;
  balls: number;
  tilt: boolean;
  /** Tilt warnings so far. The fourth one ends the ball. */
  warnings: number;
  /** How close the plumb is to the ring, from 0 to 1. */
  tiltRisk: number;
  /** How many of the table's own script handlers have run. */
  handlerCalls: number;
  /** Whether the machine's ROM is loaded and executing. */
  romRunning: boolean;
  /** The set that is running, or empty. */
  romName: string;
  /** What the machine last said about itself, mostly why a ROM would not load. */
  notice: string;
}

/** The state of the last second, or `null` if the player never started. */
export async function loopStats(): Promise<Loop | null> {
  if (!started) return null;
  const wasm = await initWasm();
  const l = wasm.loopStats();
  if (!l) return null;
  return {
    fps: l.fps,
    tps: l.physicsTicksPerSecond,
    balls: l.balls,
    tilt: l.tilt,
    warnings: l.warnings,
    tiltRisk: l.tiltRisk,
    handlerCalls: l.handlerCalls,
    romRunning: l.romRunning,
    romName: l.romName,
    notice: l.notice,
  };
}

/** Puts a new ball in front of the plunger, clearing any tilt. */
export async function newBall(): Promise<void> {
  if (!started) return;
  const wasm = await initWasm();
  wasm.newBall();
}

/**
 * Presses or releases one of the table's keys.
 *
 * The same path a real key takes, which is the point: the on-screen buttons are
 * not a second input system, they are a second way of pressing the same keys —
 * so the table's script sees `Table1_KeyDown` either way and nothing downstream
 * has to know a finger was involved.
 */
export async function pressKey(code: string, pressed: boolean): Promise<void> {
  if (!started) return;
  (await initWasm()).pressKey(code, pressed);
}

/** Lets go of everything. For a touch that was cancelled, or leaving the game. */
export async function releaseAllKeys(): Promise<void> {
  if (!started) return;
  (await initWasm()).releaseAllKeys();
}

// -- where the player looks from ---------------------------------------------

/**
 * Moves the camera to one of the named views.
 *
 * Safe to call before a table is loaded and safe to call with a view the player
 * does not know: the setting is stored in `localStorage`, so it can outlive the
 * version of the page that wrote it, and a camera left where it is beats a
 * table that will not start.
 */
export async function setCameraView(view: CameraView): Promise<void> {
  if (!started) return;
  (await initWasm()).setCameraView(view);
}

/** Where it is looking from now, or `null` before the player has started. */
export async function cameraView(): Promise<CameraView | null> {
  if (!started) return null;
  const name = (await initWasm()).cameraView();
  return CAMERA_VIEWS.includes(name as CameraView) ? (name as CameraView) : null;
}

/** The next view round, for a key that cycles rather than choosing. */
export function nextCameraView(from: CameraView): CameraView {
  const i = CAMERA_VIEWS.indexOf(from);
  return CAMERA_VIEWS[(i + 1) % CAMERA_VIEWS.length];
}

/**
 * The machine's score display, drawn at the size asked for.
 *
 * The **same drawing** that goes onto the head in the 3D scene: one module knows
 * how a digit is shaped and both destinations ask it, so the two can never be
 * showing different things.
 *
 * Empty when the machine has not said anything new since the last call at this
 * size — the caller keeps what it already drew rather than being handed a
 * megabyte of unchanged pixels ten times a second.
 */
export async function displayImage(width: number, height: number): Promise<Uint8Array | null> {
  if (!started) return null;
  const rgba = (await initWasm()).displayImage(width, height);
  return rgba.length > 0 ? rgba : null;
}

/** Where the machine's head is on screen: `[left, top, width, height]`, in
 * fractions of the canvas, or `null` when it is not in shot. */
export async function backboxRect(): Promise<[number, number, number, number] | null> {
  if (!started) return null;
  const r = (await initWasm()).backboxRect();
  return r.length === 4 ? [r[0], r[1], r[2], r[3]] : null;
}

/**
 * The two rows of the machine's score display, as text.
 *
 * A System 11 has no screen: it has two rows of fourteen characters, the top
 * one alphanumeric and the bottom one seven-segment. What comes back is what
 * they spell.
 */
export async function displays(): Promise<[string, string]> {
  if (!started) return ['', ''];
  const rows = (await initWasm()).displays();
  return [rows[0] ?? '', rows[1] ?? ''];
}

// -- the rolling record ------------------------------------------------------

/** How many seconds a mark takes with it. */
export const TELEMETRY_WINDOW_S = 30;

/** Starts or stops keeping the rolling record. */
export async function setTelemetry(on: boolean): Promise<void> {
  if (!started) return;
  (await initWasm()).setTelemetry(on);
}

/** Samples and edges currently held, as `[samples, events]`. */
export async function telemetryHeld(): Promise<[number, number]> {
  if (!started) return [0, 0];
  const held = (await initWasm()).telemetryHeld();
  return [held[0] ?? 0, held[1] ?? 0];
}

export interface Mark {
  /** File name it was saved under. */
  name: string;
  /** Bytes of JSON. */
  size: number;
  /** Where the dev server put it, if it took it. */
  savedTo: string | null;
}

/**
 * Marks this instant and takes the seconds before it out of the record.
 *
 * Two deliveries, because the two failure modes are different. The **download**
 * always works and needs nobody's cooperation, but it lands in whatever folder
 * the browser downloads to and somebody then has to carry the file. The
 * **POST** only works against the dev server, and when it does the file is
 * already sitting in the repo, which is the difference between "send me this"
 * and "have a look".
 */
export async function mark(seconds = TELEMETRY_WINDOW_S): Promise<Mark | null> {
  if (!started) return null;
  const wasm = await initWasm();

  const at = new Date();
  const json = wasm.telemetryDump(seconds, at.toISOString());
  if (!json) return null;

  // `2026-08-22T04-19-07` — colons are not allowed in a file name on Windows,
  // and the browser silently renames the download rather than saying so.
  const stamp = at.toISOString().replace(/[:.]/g, '-').replace(/Z$/, '');
  const name = `telemetry-${stamp}.json`;

  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  // Not straight away: revoking it before the browser has read it cancels the
  // download, and whether it has is not something the page gets told.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);

  let savedTo: string | null = null;
  try {
    const r = await fetch(`/debug-telemetry/${encodeURIComponent(name)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: json,
    });
    if (r.ok) savedTo = (await r.text()).trim();
  } catch {
    // No dev server, or it is not serving this. The download already happened.
  }

  return { name, size: blob.size, savedTo };
}

export type { ParsedTable };
