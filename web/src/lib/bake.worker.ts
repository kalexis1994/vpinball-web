/// <reference lib="webworker" />
//
// The baker, off everyone's thread.
//
// Tracing a table's GI lightmaps is tens of millions of rays — a second or
// two of CPU — and neither the page nor the game's own worker can afford to
// stand still for it. So it runs here, in a worker with a wasm instance of
// its own, the same arrangement the `.vpx` parser has and for the same
// reason. The result crosses back as one transferable buffer and goes into
// IndexedDB, so a table pays for its bake exactly once.

import init, { addRom, addScriptLibrary, bakeGi } from '../wasm/vpw_player.js';
import wasmUrl from '../wasm/vpw_player_bg.wasm?url';
import { provideLibraries } from './scripts';

let ready: Promise<unknown> | null = null;

export interface BakeRequest {
  key: string;
  bytes: ArrayBuffer;
  /** The machine, when the table has one: with it the baker boots the game
   * and asks the ROM which lamps switch together; without it the grouping
   * falls back to names and colours. */
  rom?: { name: string; zip: ArrayBuffer };
}

export type BakeResponse =
  | {
      key: string;
      ok: true;
      width: number;
      height: number;
      layers: number;
      data: ArrayBuffer;
      groups: string[][];
    }
  | { key: string; ok: false; error: string };

/** How long the machine is watched, in table seconds. Long enough for the
 * attract's light shows to make one full round. */
const OBSERVE_S = 30;

self.onmessage = (event: MessageEvent<BakeRequest>) => {
  const { key, bytes, rom } = event.data;
  void (async () => {
    try {
      await (ready ??= init({ module_or_path: wasmUrl }));
      // The same table-loading courtesies the player extends: the script
      // libraries, and the ROM when there is one.
      provideLibraries(addScriptLibrary);
      if (rom) addRom(rom.name, new Uint8Array(rom.zip));
      const result = bakeGi(new Uint8Array(bytes), rom ? OBSERVE_S : 0) as {
        width?: number;
        height?: number;
        layers: number;
        data?: Uint8Array;
        groups?: string[][];
      };
      if (!result.layers || !result.data) {
        // Nothing to bake: no GI string, or the table brought its own maps.
        self.postMessage({ key, ok: true, width: 0, height: 0, layers: 0, data: new ArrayBuffer(0), groups: [] } satisfies BakeResponse);
        return;
      }
      const response: BakeResponse = {
        key,
        ok: true,
        width: result.width ?? 0,
        height: result.height ?? 0,
        layers: result.layers,
        data: result.data.buffer as ArrayBuffer,
        groups: result.groups ?? [],
      };
      self.postMessage(response, { transfer: [response.data] });
    } catch (e) {
      self.postMessage({
        key,
        ok: false,
        error: e instanceof Error ? e.message : String(e),
      } satisfies BakeResponse);
    }
  })();
};
