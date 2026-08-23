/// <reference lib="webworker" />
//
// Parses `.vpx` off the main thread. Real tables weigh more than 100 MB
// (Williams' F-14 is 111 MB), so doing it on the main thread freezes the UI
// for several seconds.

import init, { readTable } from '../wasm/vpw_player.js';
import wasmUrl from '../wasm/vpw_player_bg.wasm?url';
import type { ParsedTable } from './types';

let ready: Promise<unknown> | null = null;

export interface ParseRequest {
  id: string;
  bytes: ArrayBuffer;
}

export type ParseResponse =
  | { id: string; ok: true; result: ParsedTable }
  | { id: string; ok: false; error: string };

self.onmessage = async (event: MessageEvent<ParseRequest>) => {
  const { id, bytes } = event.data;

  ready ??= init({ module_or_path: wasmUrl });
  await ready;

  try {
    const info = readTable(new Uint8Array(bytes));
    const screenshot = info.screenshot ?? null;

    const result: ParsedTable = {
      meta: {
        tableName: info.tableName ?? null,
        author: info.author ?? null,
        version: info.version ?? null,
        releaseDate: info.releaseDate ?? null,
        description: info.description ?? null,
        fileVersion: info.fileVersion,
        gameitemCount: info.gameitemCount,
        imageCount: info.imageCount,
        soundCount: info.soundCount,
        scriptLen: info.scriptLen,
        rom: {
          status: info.romStatus as ParsedTable['meta']['rom']['status'],
          name: info.romName ?? null,
          zip: info.romZip ?? null,
          alternates: info.romAlternates,
        },
        thumbMime: screenshot ? sniffImageMime(screenshot) : null,
      },
      screenshot,
    };
    info.free();

    const transfer = screenshot ? [screenshot.buffer as ArrayBuffer] : [];
    (self as DedicatedWorkerGlobalScope).postMessage(
      { id, ok: true, result } satisfies ParseResponse,
      transfer,
    );
  } catch (error) {
    (self as DedicatedWorkerGlobalScope).postMessage({
      id,
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    } satisfies ParseResponse);
  }
};

/** The `.vpx` stores the screenshot without declaring a format: we sniff it. */
function sniffImageMime(bytes: Uint8Array): string | null {
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return 'image/jpeg';
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47)
    return 'image/png';
  if (bytes[0] === 0x42 && bytes[1] === 0x4d) return 'image/bmp';
  if (bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return 'image/gif';
  return null;
}
