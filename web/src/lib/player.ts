// The page's side of the player, wherever the player runs.
//
// Everything here goes through the host (`host.ts`): a worker with the wasm
// module in it when the browser can, this thread when it cannot. The functions
// keep the shapes the components already use, so nothing above this file knows
// there are two homes.
//
// The memoisation is the other half of the job. React in strict mode mounts
// every effect **twice** in development; with nothing to stop it, that means
// downloading the table twice —111 MB each time— and parsing it twice. The
// player is a real singleton — one wasm instance in one home — so starting it
// and loading a table into it are operations that only make sense once, and
// this module keeps those promises around and reuses them.

import { getHost, type PlayerHost } from './host';
import { connectInput } from './input';
import type { Loop, SceneStats } from './hostShared';
import {
  BAKE_VERSION,
  readBake,
  readMachineState,
  readRom,
  writeBake,
  writeMachineState,
  type GiBake,
} from './library';
import BakeWorker from './bake.worker?worker';
import { provideLibraries } from './scripts';
import type { BakeRequest, BakeResponse } from './bake.worker';
import { CAMERA_VIEWS, type CameraView, type Environment } from './settings';
import type { ParsedTable, RomInfo } from './types';

export interface LoadStats extends SceneStats {
  bytes: number;
  fetchMs: number;
}

/** The host once it has resolved, for the few reads that cannot afford to
 * await. See {@link plungerPull}. */
let live: PlayerHost | null = null;
/** The host with the script libraries already handed over. */
let ready: Promise<PlayerHost> | null = null;
/** Resolves once the player has been started at least once. The calls below
 * that only make sense against a running player wait on this. */
let started: Promise<void> | null = null;
/** Key of the loaded table, so the work is not repeated. */
let loaded: { key: string; stats: Promise<LoadStats> } | null = null;

/** The host, remembered for the synchronous reads. */
function host(): Promise<PlayerHost> {
  return getHost().then((h) => {
    live = h;
    return h;
  });
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
 * float is a promise per frame too many. On the worker path the position is
 * pushed to the page every frame, so this reads a cached number; it answers
 * `null` until the host has resolved, which is exactly the period during which
 * there is no table anyway.
 */
export function plungerPull(): number | null {
  return live?.plungerPull() ?? null;
}

/** Hands the bundled libraries to the player. Idempotent. */
async function provideHostLibraries(h: PlayerHost): Promise<void> {
  if ((await h.call<number>('scriptLibraryCount')) > 0) return;
  const pending: Promise<void>[] = [];
  provideLibraries((name, text) => {
    pending.push(h.call('addScriptLibrary', [name, text]));
  });
  await Promise.all(pending);
}

/**
 * Hands the player the ROM this table needs, if the library has it.
 *
 * It has to happen before `loadTable`, because the table's script asks for its
 * machine while it is being loaded — `Controller.Run` runs inside `Table1_Init`.
 * A table whose ROM is missing still loads: the ball rolls, the flippers work,
 * and nothing scores, which is the honest outcome rather than a failure.
 */
async function provideRom(h: PlayerHost, rom: RomInfo | undefined): Promise<void> {
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
  await h.call('addRom', [rom.name, zip]);

  // And the machine's own memory, so it is the same machine as last time,
  // settings and high scores included.
  const saved = await readMachineState(rom.name);
  if (saved) await h.call('restoreMachineState', [rom.name, saved]);
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
  if (!runningSet || !started) return;
  const h = await host();
  const data = await h.call<Uint8Array | undefined>('machineState');
  if (data) await writeMachineState(runningSet, data);
}

/** The one canvas the player ever draws into. */
let canvasEl: HTMLCanvasElement | null = null;

/**
 * The player's canvas, the same element every time.
 *
 * The element belongs to the player, not to React: the game view appends it on
 * the way in and detaches it on the way out, and a return trip gets the very
 * same node back. That is not a nicety — on the WebGL backend the graphics
 * context is married to the canvas it was created from and can never present
 * to another, and on the worker path a canvas's control can only be
 * transferred once. A fresh element per visit meant a fresh surface per visit,
 * which WebGPU tolerated and WebGL answered with a crash.
 */
export function playerCanvas(): HTMLCanvasElement {
  if (!canvasEl) {
    canvasEl = document.createElement('canvas');
    canvasEl.id = 'playfield';
  }
  return canvasEl;
}

/**
 * Starts the player on the canvas, or moves it there.
 *
 * Every call reaches the host's `start`. With {@link playerCanvas} the element
 * is the same one every visit, so this usually amounts to "you are already
 * there, wake up" — but only the player's side can tell, and the answer has to
 * stay correct if the element ever does change.
 *
 * What is memoised is the expensive half: choosing the host, fetching the wasm
 * and handing over the script libraries. Those are the same whatever canvas is
 * in front of us.
 */
export function startPlayer(canvas: HTMLCanvasElement): Promise<void> {
  ready ??= (async () => {
    const h = await host();
    await provideHostLibraries(h);
    return h;
  })();
  // One start at a time. React in strict mode runs the effect twice, and two
  // starts in flight both find no player yet and both build a renderer — the
  // "already on this canvas" answer only exists once the first has finished.
  // Chaining through the previous start, whatever became of it, is what lets
  // the second call see the first one's work.
  const attached = (starting ?? Promise.resolve()).then(
    async () => (await ready!).start(canvas),
    async () => (await ready!).start(canvas),
  );
  starting = attached;
  started ??= attached;
  return attached;
}

/** The start in flight, so a second one waits for it. */
let starting: Promise<void> | null = null;

/**
 * Wires the keyboard and the pointer to the player. Returns the unwiring.
 * Re-exported so the component that mounts the canvas has one module to talk
 * to; the wiring itself lives in `input.ts`.
 */
export { connectInput };

/**
 * Tells the player nobody is looking at it, so a worker-side loop can hold the
 * clocks the way the main-thread loop does when its canvas leaves the
 * document. Safe to call before anything started.
 */
export function hidePlayer(): void {
  live?.setVisible(false);
}

/**
 * Forgets the last table that was loaded, so the next ask reloads it.
 *
 * Called when a ROM arrives. Handing a machine over *after* the table has
 * loaded does nothing: the script has already run, already asked for a
 * controller, and already been told there is not one. Only loading the table
 * again makes that ask happen a second time.
 */
export function forgetLoadedTable(): void {
  loaded = null;
}

/**
 * Fetches a table and loads it into the player.
 *
 * `key` identifies the table **and the machine it was loaded with**: asking
 * twice for the same pair returns the promise from the first call instead of
 * redoing the work.
 *
 * The machine is part of the key because it is part of the answer. Loading a
 * table without its ROM and then loading the ROM used to leave the player with
 * the first result for ever — a table that renders and rolls a ball and starts
 * no game, with the machine sitting in storage unused until the page was
 * reloaded.
 */
export function loadTable(
  key: string,
  fetchBytes: () => Promise<Uint8Array>,
  rom?: RomInfo,
): Promise<LoadStats> {
  const cacheKey = `${key}|${rom?.name ?? ''}`;
  if (loaded?.key === cacheKey) return loaded.stats;

  const stats = (async () => {
    const h = await host();
    await provideRom(h, rom);
    const t0 = performance.now();
    const bytes = await fetchBytes();
    const fetchMs = performance.now() - t0;

    // The bytes are moved, not copied: on the worker path this is 111 MB that
    // would otherwise exist twice while the copy is in flight.
    const size = bytes.byteLength;
    const s = await h.call<SceneStats>('loadTable', [bytes], [bytes.buffer as ArrayBuffer]);
    // The lightmaps, in the background: from the cache when a visit already
    // paid for them, traced in the bake worker when not. The table is already
    // playing either way; the light arrives when it arrives.
    void ensureGiBake(key, fetchBytes, rom).catch((e) => {
      console.warn('[bake] the GI bake failed:', e);
    });
    return { ...s, bytes: size, fetchMs };
  })();

  loaded = { key: cacheKey, stats };
  // If it fails, forget it so it can be retried.
  stats.catch(() => {
    if (loaded?.key === cacheKey) loaded = null;
  });
  return stats;
}

/** The bakes already being traced or applied, so a strict-mode double mount
 * does not trace twice. */
const baking = new Set<string>();

/**
 * Sees to it that the table's GI lightmaps are installed: from IndexedDB if a
 * past visit traced them, from the bake worker if not — in which case they are
 * also put away for every visit after.
 */
async function ensureGiBake(
  key: string,
  fetchBytes: () => Promise<Uint8Array>,
  rom?: RomInfo,
): Promise<void> {
  if (baking.has(key)) return;
  baking.add(key);
  const h = await host();

  const apply = (bake: GiBake) =>
    bake.layers > 0
      ? h
          .call('applyGiBake', [
            bake.width,
            bake.height,
            bake.layers,
            new Uint8Array(bake.data),
            bake.groups,
          ])
          .then(() => console.debug(`[bake] GI lightmaps on: ${bake.layers} groups`))
      : Promise.resolve();

  const cached = await readBake(key);
  if (cached) {
    await apply(cached);
    return;
  }

  // A fresh copy of the table: the load transferred its own away. And the
  // ROM beside it, so the baker can boot the machine and watch which lamps
  // it switches together instead of guessing from names.
  const bytes = await fetchBytes();
  const zip = rom?.name ? ((await readRom(rom.name)) ?? (await devRom(rom.name))) : null;
  const result = await new Promise<BakeResponse>((resolve, reject) => {
    // A worker per bake, and gone straight after: it holds a wasm instance
    // and the whole table, which is not a thing to keep around idle.
    const worker = new BakeWorker();
    worker.onmessage = (e: MessageEvent<BakeResponse>) => {
      worker.terminate();
      resolve(e.data);
    };
    worker.onerror = (e) => {
      worker.terminate();
      reject(new Error(e.message || 'the bake worker crashed'));
    };
    const request: BakeRequest = {
      key,
      bytes: bytes.buffer as ArrayBuffer,
      rom: rom?.name && zip ? { name: rom.name, zip: zip.buffer as ArrayBuffer } : undefined,
    };
    const transfer: ArrayBuffer[] = [bytes.buffer as ArrayBuffer];
    if (request.rom) transfer.push(request.rom.zip);
    worker.postMessage(request, transfer);
  });
  if (!result.ok) {
    console.warn('[bake] the trace failed:', result.error);
    baking.delete(key);
    return;
  }

  const bake: GiBake = {
    version: BAKE_VERSION,
    width: result.width,
    height: result.height,
    layers: result.layers,
    data: result.data,
    groups: result.groups,
  };
  // Stored even when there was nothing to bake, so a table with no GI string
  // is not re-parsed on every visit to find that out again.
  await writeBake(key, bake);
  await apply(bake);
}

export type { Loop };

/** The state of the last second, or `null` if the player never started. */
export async function loopStats(): Promise<Loop | null> {
  if (!started) return null;
  return (await host()).call<Loop | null>('loopStats');
}

/** Puts a new ball in front of the plunger, clearing any tilt. */
export async function newBall(): Promise<void> {
  if (!started) return;
  await (await host()).call('newBall');
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
  await (await host()).call('pressKey', [code, pressed]);
}

/** Lets go of everything. For a touch that was cancelled, or leaving the game. */
export async function releaseAllKeys(): Promise<void> {
  if (!started) return;
  await (await host()).call('releaseAllKeys');
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
  await (await host()).call('setCameraView', [view]);
}

/** Where it is looking from now, or `null` before the player has started. */
export async function cameraView(): Promise<CameraView | null> {
  if (!started) return null;
  const name = await (await host()).call<string>('cameraView');
  return CAMERA_VIEWS.includes(name as CameraView) ? (name as CameraView) : null;
}

/**
 * The player's day/night, 0 to 1, or `null` for the table's own lighting.
 * Plenty of tables are authored dark on purpose; this is the same override
 * Visual Pinball's own settings carry.
 */
export async function setDayNight(brightness: number | null): Promise<void> {
  if (!started) return;
  await (await host()).call('setDayNight', [brightness ?? -1]);
}

/** The bar's environment map, fetched once and kept: switching rooms twice
 * should not download the room twice. */
let roomBytes: Promise<Uint8Array> | null = null;

/**
 * Puts the machine in a room, or back in the table's own light.
 *
 * The room is a Radiance `.hdr` the page ships (`public/env/`); the map is
 * fetched on first use and cached for the session. A copy crosses to the
 * player each time — transferring the cached buffer would consume it.
 */
export async function setEnvironment(env: Environment): Promise<void> {
  if (!started) return;
  const h = await host();
  if (env === 'table') {
    await h.call('setEnvironment', [new Uint8Array(0)]);
    return;
  }
  roomBytes ??= fetch(`${import.meta.env.BASE_URL}env/bar.hdr`)
    .then((r) => {
      if (!r.ok) throw new Error(`the room's map answered ${r.status}`);
      return r.arrayBuffer();
    })
    .then((b) => new Uint8Array(b));
  try {
    const bytes = await roomBytes;
    const copy = bytes.slice();
    const on = await h.call<boolean>('setEnvironment', [copy], [copy.buffer as ArrayBuffer]);
    if (on) console.debug('[env] the room is on');
    else console.warn('[env] the player declined the room (no table yet, or the map did not decode)');
  } catch (e) {
    roomBytes = null;
    console.warn('[env] the room did not load:', e);
  }
}

/**
 * Switches the flat engine: the table photographed once and played as
 * pictures, with only the ball, the flippers and the display in real 3D —
 * for machines whose GPU cannot afford the full render. The bake spreads
 * over a couple of seconds of play; the switch is seamless when it lands.
 */
export async function setFlat(on: boolean): Promise<void> {
  if (!started) return;
  await (await host()).call('setFlat', [on]);
}

/**
 * Switches the resolution governor: on, it trades pixels for frames on a
 * machine that cannot have both; off, the picture stays sharp whatever the
 * frame rate does.
 */
export async function setAdaptive(on: boolean): Promise<void> {
  if (!started) return;
  await (await host()).call('setAdaptive', [on]);
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
  const rgba = await (await host()).call<Uint8Array>('displayImage', [width, height]);
  return rgba.length > 0 ? rgba : null;
}

/** Where the playfield is on screen: `[left, top, width, height]`, in
 * fractions of the canvas, or `null` when there is no table. What the score
 * panel uses to sit in the gutter beside the table rather than on it. */
export async function playfieldRect(): Promise<[number, number, number, number] | null> {
  if (!started) return null;
  const r = await (await host()).call<Float32Array>('playfieldRect');
  return r.length === 4 ? [r[0], r[1], r[2], r[3]] : null;
}

/** Where the machine's head is on screen: `[left, top, width, height]`, in
 * fractions of the canvas, or `null` when it is not in shot. */
export async function backboxRect(): Promise<[number, number, number, number] | null> {
  if (!started) return null;
  const r = await (await host()).call<Float32Array>('backboxRect');
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
  const rows = await (await host()).call<string[]>('displays');
  return [rows[0] ?? '', rows[1] ?? ''];
}

// -- the rolling record ------------------------------------------------------

/** How many seconds a mark takes with it. */
export const TELEMETRY_WINDOW_S = 30;

/** Starts or stops keeping the rolling record. */
export async function setTelemetry(on: boolean): Promise<void> {
  if (!started) return;
  await (await host()).call('setTelemetry', [on]);
}

/** Samples and edges currently held, as `[samples, events]`. */
export async function telemetryHeld(): Promise<[number, number]> {
  if (!started) return [0, 0];
  const held = await (await host()).call<Uint32Array>('telemetryHeld');
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
  const h = await host();

  const at = new Date();
  const json = await h.call<string | undefined>('telemetryDump', [seconds, at.toISOString()]);
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
