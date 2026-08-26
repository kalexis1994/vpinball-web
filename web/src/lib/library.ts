// Table library persisted in IndexedDB.
//
// We store the `.vpx` as a Blob directly: IndexedDB keeps them out of line, so
// a 100+ MB table never passes through the JS heap when reading or writing it.
// The metadata goes in its own store so the menu can be listed without
// touching the files.

import type { ParsedTable, TableEntry } from './types';
import ParseWorker from './parse.worker?worker';
import type { ParseRequest, ParseResponse } from './parse.worker';

const DB_NAME = 'vpinball-web';
const DB_VERSION = 3;

const STORE_TABLES = 'tables'; // TableEntry, keyPath: id
const STORE_FILES = 'files'; // Blob of the .vpx, key: id
const STORE_THUMBS = 'thumbs'; // Blob of the screenshot, key: id
const STORE_ROMS = 'roms'; // Blob of the ROM zip, key: set name, lowercased
const STORE_SAVES = 'saves'; // The machine's CMOS, key: set name, lowercased
const STORE_BAKES = 'bakes'; // A GiBake, key: the table's load key

export class StorageUnavailable extends Error {
  constructor() {
    super('This browser does not expose IndexedDB.');
    this.name = 'StorageUnavailable';
  }
}

export function storageAvailable(): boolean {
  return typeof indexedDB !== 'undefined';
}

// --- Minimal wrapper over IndexedDB ------------------------------------------

let dbPromise: Promise<IDBDatabase> | null = null;

function openDb(): Promise<IDBDatabase> {
  if (!storageAvailable()) return Promise.reject(new StorageUnavailable());

  dbPromise ??= new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_TABLES)) {
        db.createObjectStore(STORE_TABLES, { keyPath: 'id' });
      }
      for (const name of [STORE_FILES, STORE_THUMBS, STORE_ROMS, STORE_SAVES, STORE_BAKES]) {
        if (!db.objectStoreNames.contains(name)) db.createObjectStore(name);
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('could not open the DB'));
  });

  return dbPromise;
}

/** Wraps an `IDBRequest` in a promise. */
function promisify<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('the operation failed'));
  });
}

/**
 * Runs `run` inside a transaction and, if it writes, only resolves once the
 * transaction commits. Without waiting for the commit a write can report OK
 * and abort afterwards.
 */
async function transact<T>(
  stores: string[],
  mode: IDBTransactionMode,
  run: (tx: IDBTransaction) => Promise<T>,
): Promise<T> {
  const db = await openDb();
  const tx = db.transaction(stores, mode);

  const done = new Promise<void>((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error ?? new Error('transaction aborted'));
    tx.onerror = () => reject(tx.error ?? new Error('error in the transaction'));
  });

  const result = await run(tx);
  if (mode !== 'readonly') await done;
  return result;
}

// --- Metadata ----------------------------------------------------------------

export async function loadLibrary(): Promise<TableEntry[]> {
  try {
    const entries = await transact([STORE_TABLES], 'readonly', (tx) =>
      promisify(tx.objectStore(STORE_TABLES).getAll() as IDBRequest<TableEntry[]>),
    );
    return entries.sort((a, b) => a.addedAt - b.addedAt);
  } catch {
    // First visit, or the browser denied us storage.
    return [];
  }
}

// --- Adding tables -----------------------------------------------------------

let worker: Worker | null = null;
let nextRequestId = 0;

/** Parses a `.vpx` in the worker, off the main thread. */
/**
 * Reads a `.vpx` without storing anything.
 *
 * Separate from `addTable` because the import flow has to know what a file *is*
 * before it can ask whether to keep it: which table, and which ROM it wants.
 * The answer to both is in the script, and finding it means parsing.
 */
export function parseTable(bytes: ArrayBuffer): Promise<ParsedTable> {
  return parseInWorker(bytes);
}

function parseInWorker(bytes: ArrayBuffer): Promise<ParsedTable> {
  worker ??= new ParseWorker();
  const id = String(nextRequestId++);

  return new Promise((resolve, reject) => {
    const onMessage = (event: MessageEvent<ParseResponse>) => {
      if (event.data.id !== id) return;
      worker!.removeEventListener('message', onMessage);
      if (event.data.ok) resolve(event.data.result);
      else reject(new Error(event.data.error));
    };
    worker!.addEventListener('message', onMessage);
    // We transfer the buffer: no copy, but `bytes` becomes unusable here.
    worker!.postMessage({ id, bytes } satisfies ParseRequest, [bytes]);
  });
}

/**
 * Adds a table to the library. If there is already one with the same name and
 * size, it replaces it instead of duplicating it.
 */
export async function addTable(file: File, known?: ParsedTable): Promise<TableEntry> {
  // The `File` is stored as is; the ArrayBuffer is only for the parsing and is
  // transferred to the worker, so the 100+ MB are not duplicated on the heap.
  //
  // `known` is what the import flow already found out when it built its preview.
  // Parsing a hundred-megabyte table twice to show it and then keep it is the
  // sort of thing that makes an import feel broken.
  const parsed = known ?? (await parseInWorker(await file.arrayBuffer()));

  const existing = (await loadLibrary()).find(
    (e) => e.fileName === file.name && e.fileSize === file.size,
  );
  const id = existing?.id ?? crypto.randomUUID();

  const entry: TableEntry = {
    id,
    fileName: file.name,
    fileSize: file.size,
    addedAt: existing?.addedAt ?? Date.now(),
    ...parsed.meta,
  };

  const thumb =
    parsed.screenshot && parsed.meta.thumbMime
      ? new Blob([parsed.screenshot as BlobPart], { type: parsed.meta.thumbMime })
      : null;

  await transact([STORE_TABLES, STORE_FILES, STORE_THUMBS], 'readwrite', async (tx) => {
    tx.objectStore(STORE_TABLES).put(entry);
    tx.objectStore(STORE_FILES).put(file, id);
    if (thumb) tx.objectStore(STORE_THUMBS).put(thumb, id);
  });

  return entry;
}

export async function removeTable(id: string): Promise<TableEntry[]> {
  await transact([STORE_TABLES, STORE_FILES, STORE_THUMBS], 'readwrite', async (tx) => {
    tx.objectStore(STORE_TABLES).delete(id);
    tx.objectStore(STORE_FILES).delete(id);
    tx.objectStore(STORE_THUMBS).delete(id);
  });
  return loadLibrary();
}

/** Wipes the whole library. */
export async function clearLibrary(): Promise<void> {
  await transact([STORE_TABLES, STORE_FILES, STORE_THUMBS], 'readwrite', async (tx) => {
    tx.objectStore(STORE_TABLES).clear();
    tx.objectStore(STORE_FILES).clear();
    tx.objectStore(STORE_THUMBS).clear();
  });
}

/** Bytes of the `.vpx`, for when the player can load it. */
export async function readTableFile(id: string): Promise<Uint8Array> {
  const blob = await transact([STORE_FILES], 'readonly', (tx) =>
    promisify(tx.objectStore(STORE_FILES).get(id) as IDBRequest<Blob | undefined>),
  );
  if (!blob) throw new Error(`table ${id} is not in the library`);
  return new Uint8Array(await blob.arrayBuffer());
}

/** Object URL for the thumbnail, or null if the table ships no screenshot. */
export async function thumbnailUrl(entry: TableEntry): Promise<string | null> {
  if (!entry.thumbMime) return null;
  try {
    const blob = await transact([STORE_THUMBS], 'readonly', (tx) =>
      promisify(tx.objectStore(STORE_THUMBS).get(entry.id) as IDBRequest<Blob | undefined>),
    );
    return blob ? URL.createObjectURL(blob) : null;
  } catch {
    return null;
  }
}

// --- ROMs --------------------------------------------------------------------
//
// A ROM table's rules are not in the `.vpx`: they are in the machine's firmware,
// which is copyrighted and ships separately as a zip named after the set. So
// the player needs both, and they are kept apart because one ROM serves every
// table built on that machine.

/** The set name a zip is for, taken from its file name: `F14_L1.zip` -> `f14_l1`. */
export function romSetName(file: File): string {
  return file.name.replace(/\.zip$/i, '').toLowerCase();
}

/** Adds a ROM zip to the library, replacing any earlier one for the same set. */
export async function addRom(file: File): Promise<string> {
  const set = romSetName(file);
  await transact([STORE_ROMS], 'readwrite', async (tx) => {
    tx.objectStore(STORE_ROMS).put(file, set);
  });
  return set;
}

/** The set names the library has ROMs for. */
export async function listRoms(): Promise<string[]> {
  try {
    const keys = await transact([STORE_ROMS], 'readonly', (tx) =>
      promisify(tx.objectStore(STORE_ROMS).getAllKeys() as IDBRequest<IDBValidKey[]>),
    );
    return keys.map(String).sort();
  } catch {
    return [];
  }
}

export async function removeRom(set: string): Promise<void> {
  await transact([STORE_ROMS, STORE_SAVES], 'readwrite', async (tx) => {
    tx.objectStore(STORE_ROMS).delete(set);
    tx.objectStore(STORE_SAVES).delete(set);
  });
}

/** Bytes of a ROM zip, or null if the library does not have that set. */
export async function readRom(set: string): Promise<Uint8Array | null> {
  try {
    const blob = await transact([STORE_ROMS], 'readonly', (tx) =>
      promisify(tx.objectStore(STORE_ROMS).get(set.toLowerCase()) as IDBRequest<Blob | undefined>),
    );
    return blob ? new Uint8Array(await blob.arrayBuffer()) : null;
  } catch {
    return null;
  }
}

/**
 * The machine's battery-backed memory: its settings, audits and high scores.
 *
 * Saving it is what makes a machine feel like the same machine next time. A
 * fresh one is not broken, it is just factory-new — and it costs a few seconds
 * at start-up while the ROM writes its defaults.
 */
export async function readMachineState(set: string): Promise<Uint8Array | null> {
  try {
    const blob = await transact([STORE_SAVES], 'readonly', (tx) =>
      promisify(tx.objectStore(STORE_SAVES).get(set.toLowerCase()) as IDBRequest<Blob | undefined>),
    );
    return blob ? new Uint8Array(await blob.arrayBuffer()) : null;
  } catch {
    return null;
  }
}

export async function writeMachineState(set: string, data: Uint8Array): Promise<void> {
  await transact([STORE_SAVES], 'readwrite', async (tx) => {
    tx.objectStore(STORE_SAVES).put(new Blob([data as BlobPart]), set.toLowerCase());
  });
}

/** How much space the library takes up, according to the browser. */
export async function storageUsage(): Promise<{ usage: number; quota: number } | null> {
  if (!navigator.storage?.estimate) return null;
  const { usage = 0, quota = 0 } = await navigator.storage.estimate();
  return { usage, quota };
}

// -- the baked lightmaps ------------------------------------------------------

/**
 * A traced GI bake, exactly as the wasm baker hands it over and the player
 * takes it back. Tens of millions of rays went into `data`, which is the
 * whole reason it is kept: the second visit to a table pays nothing.
 */
export interface GiBake {
  /** Bump when the baker's output changes shape or meaning. */
  version: number;
  width: number;
  height: number;
  layers: number;
  /** The layers' half floats, one after another. */
  data: ArrayBuffer;
  /** Each layer's lamp names, for finding the lamps again. */
  groups: string[][];
}

/** The baker's current output shape. */
export const BAKE_VERSION = 3;

export async function readBake(key: string): Promise<GiBake | null> {
  const bake = await transact([STORE_BAKES], 'readonly', (tx) =>
    promisify(tx.objectStore(STORE_BAKES).get(key) as IDBRequest<GiBake | undefined>),
  );
  // A bake from an older baker is not half-usable: the shapes moved.
  return bake && bake.version === BAKE_VERSION ? bake : null;
}

export async function writeBake(key: string, bake: GiBake): Promise<void> {
  await transact([STORE_BAKES], 'readwrite', (tx) =>
    promisify(tx.objectStore(STORE_BAKES).put(bake, key)),
  );
}
