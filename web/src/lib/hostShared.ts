// What crosses the boundary between the page and wherever the player runs.
//
// The wasm module answers a few calls with `wasm-bindgen` class instances, and
// a class instance does not survive `postMessage`: it is a handle into one
// module's memory, meaningless anywhere else. These mappers turn them into
// plain data **at the source**, so the page sees the same shapes whether the
// player answered from this thread or from a worker — and the worker protocol
// never has to know which calls are special.

import type * as Wasm from '../wasm/vpw_player';

/** What the player says about a scene it just loaded. */
export interface SceneStats {
  meshes: number;
  vertices: number;
  triangles: number;
  textures: number;
  drawCalls: number;
  drawCallsNaive: number;
  parseMs: number;
  extractMs: number;
  uploadMs: number;
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
  /** Whether this machine has a sound board of its own that loaded. */
  soundBoard: boolean;
  /** Samples a second it is making. A working one is a shade over 24000. */
  soundRate: number;
  /** The governor's rung: 0 is full quality, higher is softer. */
  qualityTier: number;
  /** What the machine last said about itself, mostly why a ROM would not load. */
  notice: string;
}

/** Flattens the load's answer and frees the wasm-side struct. */
export function sceneStats(s: Wasm.SceneStats): SceneStats {
  const out = {
    meshes: s.meshes,
    vertices: s.vertices,
    triangles: s.triangles,
    textures: s.textures,
    drawCalls: s.drawCalls,
    drawCallsNaive: s.drawCallsNaive,
    parseMs: s.parseMs,
    extractMs: s.extractMs,
    uploadMs: s.uploadMs,
  };
  s.free();
  return out;
}

/** Flattens the loop's answer and frees the wasm-side struct. */
export function loopStats(l: Wasm.LoopStats): Loop {
  const out = {
    fps: l.fps,
    tps: l.physicsTicksPerSecond,
    balls: l.balls,
    tilt: l.tilt,
    warnings: l.warnings,
    tiltRisk: l.tiltRisk,
    handlerCalls: l.handlerCalls,
    romRunning: l.romRunning,
    romName: l.romName,
    soundBoard: l.soundBoard,
    soundRate: l.soundRate,
    qualityTier: l.qualityTier,
    notice: l.notice,
  };
  l.free();
  return out;
}

// -- the photograph for the library's card ------------------------------------

/** How wide a card's picture is kept. */
const CARD_WIDTH = 640;
/** And its shape, which is the shape of the card it goes on. */
const CARD_ASPECT = 4 / 3;
/**
 * How bright a pixel has to be to count as part of the machine.
 *
 * Low, because a cabinet's sides are dark wood in a dark room and cropping
 * them off would cut the machine in half — but not zero, because the room
 * behind it is never quite zero either.
 */
const INK = 14;

/**
 * Turns the last drawn frame into a card for the library.
 *
 * The frame is the whole window, and a pinball machine standing in the middle
 * of a widescreen window is a narrow column with a great deal of nothing
 * either side of it. So the picture is cropped to what was actually drawn —
 * found by looking, which needs no help from the renderer and works for any
 * view — and then widened back out to the card's own shape, so the shelf can
 * crop it to fit without cutting the head off the machine.
 *
 * **The copy is taken before anything is awaited.** `drawImage` snapshots its
 * source synchronously, so the animation frame that would otherwise draw over
 * the photograph cannot get in between; an `await` before this line is what
 * would make this return whatever the player happened to be showing.
 */
export async function cardImage(
  surface: OffscreenCanvas | HTMLCanvasElement,
): Promise<Uint8Array | null> {
  const scale = Math.min(1, CARD_WIDTH / surface.width);
  const w = Math.max(1, Math.round(surface.width * scale));
  const h = Math.max(1, Math.round(surface.height * scale));
  const full = new OffscreenCanvas(w, h);
  const ctx = full.getContext('2d', { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(surface, 0, 0, w, h);

  const box = inkBounds(ctx.getImageData(0, 0, w, h).data, w, h);
  if (!box) return null;
  const crop = widen(box, w, h);

  const out = new OffscreenCanvas(CARD_WIDTH, Math.round(CARD_WIDTH / CARD_ASPECT));
  const paint = out.getContext('2d');
  if (!paint) return null;
  // Back to the original rather than to the copy: the copy was only ever a
  // cheap thing to measure, and going through it would cost a generation of
  // resampling for nothing.
  const back = surface.width / w;
  paint.drawImage(
    surface,
    crop.x * back,
    crop.y * back,
    crop.w * back,
    crop.h * back,
    0,
    0,
    out.width,
    out.height,
  );
  const blob = await out.convertToBlob({ type: 'image/jpeg', quality: 0.82 });
  return new Uint8Array(await blob.arrayBuffer());
}

/** The smallest box holding everything that is not the empty room. */
function inkBounds(
  px: Uint8ClampedArray,
  w: number,
  h: number,
): { x: number; y: number; w: number; h: number } | null {
  let x0 = w;
  let y0 = h;
  let x1 = -1;
  let y1 = -1;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      if (px[i] > INK || px[i + 1] > INK || px[i + 2] > INK) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  // A frame with nothing on it: the load has not drawn yet, or the table is
  // black. Either way there is no picture worth keeping.
  if (x1 < x0 || y1 < y0) return null;
  return { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
}

/** Grows a box to the card's shape, around its own centre, without leaving
 *  the frame. */
function widen(
  box: { x: number; y: number; w: number; h: number },
  w: number,
  h: number,
): { x: number; y: number; w: number; h: number } {
  // A little air, so the machine is not jammed against the edge.
  const margin = Math.round(Math.max(box.w, box.h) * 0.04);
  let cw = Math.min(w, box.w + margin * 2);
  let ch = Math.min(h, box.h + margin * 2);
  if (cw / ch < CARD_ASPECT) {
    cw = Math.min(w, Math.round(ch * CARD_ASPECT));
    // Wider than the frame is: take the height back instead, so the shape is
    // right even on a window that is nearly square.
    ch = Math.min(h, Math.round(cw / CARD_ASPECT));
  } else {
    ch = Math.min(h, Math.round(cw / CARD_ASPECT));
  }
  const cx = box.x + box.w / 2;
  const cy = box.y + box.h / 2;
  return {
    x: Math.max(0, Math.min(w - cw, Math.round(cx - cw / 2))),
    y: Math.max(0, Math.min(h - ch, Math.round(cy - ch / 2))),
    w: cw,
    h: ch,
  };
}
