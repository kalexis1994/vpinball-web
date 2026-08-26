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
    notice: l.notice,
  };
  l.free();
  return out;
}
