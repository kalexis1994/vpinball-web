/// <reference lib="webworker" />
//
// The player, off the main thread.
//
// The whole of it: the wasm module, the game loop, the WebGPU renderer and the
// audio production all live here, drawing into the `OffscreenCanvas` the page
// transferred. What stays on the page is what only a page can do — React,
// layout, the DOM events, the `AudioContext` a browser will only start from a
// user gesture, and IndexedDB reads the page already owns.
//
// The reason is isolation, in both directions. The simulation stops competing
// with React renders, garbage collection and whatever else the main thread is
// doing — a busy page no longer steals physics time — and a heavy load stops
// freezing the page: parsing a 111 MB table used to hold the main thread for
// seconds, and here it holds nothing the user can feel.
//
// The protocol is one request shape, `{id, op, args}`, answered by
// `{kind: 'reply', id, ...}`. Almost every op is simply the name of a wasm
// export; the handful that are not — the canvas, the audio port, the two calls
// whose answers are wasm classes — are named cases in `dispatch`.

import init, * as wasm from '../wasm/vpw_player.js';
import wasmUrl from '../wasm/vpw_player_bg.wasm?url';
import { cardImage, loopStats, sceneStats } from './hostShared';

interface Request {
  id: number;
  op: string;
  args: unknown[];
}

let ready: Promise<unknown> | null = null;
const ensure = () => (ready ??= init({ module_or_path: wasmUrl }));

// -- the pump -----------------------------------------------------------------
//
// The worker-side twin of the main thread's audio pump: once per animation
// frame, render whatever keeps the worklet's queue at its target depth and
// push it straight to the audio thread. The page is not on this path at all —
// the samples go worker → worklet through a `MessageChannel` — so a busy page
// cannot starve the sound.

/** How much audio to keep queued ahead, in seconds. See `lib/audio.ts`. */
const TARGET_SECONDS = 0.1;
/** The most to render in one go, so a stall cannot ask for a second of audio. */
const MAX_CHUNK_SECONDS = 0.25;

/** The worklet's end of the channel, once the page has wired it. */
let feed: MessagePort | null = null;
let rate = 48000;
/** Frames believed queued: the worklet's last report, plus what was sent since. */
let queued = 0;

/** Last plunger position pushed to the page, so it is only sent on change. */
let lastPlunger: number | null | undefined;
/** The canvas the player draws on, kept so a photograph can be taken of it. */
let surface: OffscreenCanvas | null = null;
let looping = false;

function tick() {
  if (feed) {
    const target = rate * TARGET_SECONDS;
    const want = Math.min(Math.ceil(target - queued), Math.ceil(rate * MAX_CHUNK_SECONDS));
    if (want > 0) {
      const t = performance.now();
      const pcm = wasm.renderAudio(want);
      if (pcm.length > 0) {
        queued += pcm.length / 2;
        feed.postMessage(pcm, [pcm.buffer]);
      }
      const took = performance.now() - t;
      if (took > 8) wasm.notePause('audio pump', took);
    }
  }

  // The plunger, pushed rather than asked for: the page draws the on-screen
  // rod every frame, and a round trip per frame to fetch one float would put
  // the drawing a message behind the finger.
  const plunger = wasm.plungerPull() ?? null;
  if (plunger !== lastPlunger) {
    lastPlunger = plunger;
    self.postMessage({ kind: 'plunger', value: plunger });
  }

  requestAnimationFrame(tick);
}

// -- the protocol -------------------------------------------------------------

async function dispatch(op: string, args: unknown[]): Promise<unknown> {
  // The probe answers before the module is fetched: a browser that cannot run
  // the player here should find out cheaply, not after eight megabytes. The
  // decision is irreversible on the page side — a transferred canvas cannot
  // be taken back — so it is made on evidence: an adapter actually granted,
  // or a real WebGL2 context, not just an API present.
  if (op === 'probe') {
    if (typeof requestAnimationFrame !== 'function') return false;
    const wantGl = args[0] === true;
    const gpu = (navigator as { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
    if (!wantGl && gpu) {
      try {
        if ((await gpu.requestAdapter()) !== null) return true;
      } catch {
        // WebGPU said no; WebGL2 below still counts.
      }
    }
    // The fallback renderer: WebGL2 works in a worker and needs no secure
    // context, which is what a phone on plain-HTTP LAN has.
    try {
      return new OffscreenCanvas(1, 1).getContext('webgl2') !== null;
    } catch {
      return false;
    }
  }

  await ensure();
  switch (op) {
    case 'start': {
      const a = args[0] as {
        canvas: OffscreenCanvas;
        width: number;
        height: number;
        forceGl?: boolean;
      };
      // The renderer reads the flag off this scope's global when the
      // instance is built.
      if (a.forceGl) (globalThis as { VPW_FORCE_WEBGL?: boolean }).VPW_FORCE_WEBGL = true;
      surface = a.canvas;
      await wasm.startOffscreen(a.canvas, a.width, a.height);
      if (!looping) {
        looping = true;
        requestAnimationFrame(tick);
      }
      return undefined;
    }
    case 'audio': {
      const a = args[0] as { port: MessagePort; rate: number };
      feed = a.port;
      rate = a.rate;
      queued = 0;
      feed.onmessage = (event: MessageEvent<{ queued?: number }>) => {
        if (typeof event.data?.queued === 'number') queued = event.data.queued;
      };
      return undefined;
    }
    // The two answers that are wasm classes, flattened here because a class
    // instance does not survive postMessage.
    case 'loadTable': {
      const s = wasm.loadTable(args[0] as Uint8Array);
      return sceneStats(s);
    }
    // A photograph of the machine, for the library's card.
    //
    // Two steps that must not be interrupted: the player draws one frame from
    // the front, and the canvas is copied *in the same task*. `drawImage`
    // takes its snapshot synchronously, so the animation frame that would
    // otherwise overwrite the picture cannot get in between — which is why
    // there is no pausing here and no promise until the copy is already made.
    case 'shoot': {
      if (!surface || !wasm.shoot()) return null;
      return cardImage(surface);
    }
    case 'loopStats': {
      const l = wasm.loopStats();
      return l ? loopStats(l) : null;
    }
    default: {
      const fn = (wasm as unknown as Record<string, unknown>)[op];
      if (typeof fn !== 'function') throw new Error(`the player has no export named '${op}'`);
      return (fn as (...a: unknown[]) => unknown)(...args);
    }
  }
}

self.onmessage = (event: MessageEvent<Request>) => {
  const { id, op, args } = event.data;
  void dispatch(op, args).then(
    (value) => {
      // Big binary answers move, they are not copied: the display image is a
      // quarter megabyte ten times a second.
      const transfer =
        value instanceof Uint8Array || value instanceof Float32Array || value instanceof Uint32Array
          ? [value.buffer as ArrayBuffer]
          : [];
      self.postMessage({ kind: 'reply', id, ok: true, value }, { transfer });
    },
    (error: unknown) => {
      self.postMessage({
        kind: 'reply',
        id,
        ok: false,
        error: error instanceof Error ? error.message : String(error),
      });
    },
  );
};
