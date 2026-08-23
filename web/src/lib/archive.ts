// Reading a zip in the browser, by hand.
//
// There is no dependency here for the same reason there is none on the Rust
// side: a zip is a directory at the end of the file and a stream of entries in
// front of it, the browser already carries an inflater in
// `DecompressionStream`, and the alternative is a library to keep an eye on for
// something the format spec fits on one page.
//
// The Rust half of this project reads a PinMAME ROM zip the same way — see
// `read_zip` in `crates/vpw-game/src/controller.rs`. The two do not share code
// and do not need to: one runs where a table is stored and the other where it
// is played.
//
// Only the two methods that occur in practice are supported: stored, and
// deflate. A zip using anything else says so, rather than handing back
// nonsense.

/** One file inside a zip, as listed in its central directory. */
export interface ZipEntry {
  /** The path as stored, e.g. `roms/f14_l1.zip`. */
  path: string;
  /** The last segment of it, which is the part that names things. */
  name: string;
  /** Size once inflated. */
  size: number;
  /** 0 stored, 8 deflate. Anything else cannot be read. */
  method: number;
  compressedSize: number;
  /** Where the local header sits, which is where reading starts. */
  offset: number;
}

/** Signature of the end-of-central-directory record. */
const EOCD = 0x0605_4b50;
/** Its fixed size, before the trailing comment. */
const EOCD_SIZE = 22;
/** How far back to look for it: the comment can be up to 64 KB. */
const EOCD_SEARCH = EOCD_SIZE + 0xffff;

export class NotAZip extends Error {
  constructor(why = 'this is not a zip') {
    super(why);
    this.name = 'NotAZip';
  }
}

/**
 * Lists what a zip holds, without inflating any of it.
 *
 * Reads only the tail of the blob, so asking what is in a two-gigabyte archive
 * costs a few kilobytes.
 */
export async function listZip(blob: Blob): Promise<ZipEntry[]> {
  const tailSize = Math.min(blob.size, EOCD_SEARCH);
  const tail = new DataView(await blob.slice(blob.size - tailSize).arrayBuffer());

  // Backwards from the end: the record is last, and scanning forwards would
  // find a signature that happens to appear inside a compressed file.
  let eocd = -1;
  for (let i = tail.byteLength - EOCD_SIZE; i >= 0; i--) {
    if (tail.getUint32(i, true) === EOCD) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new NotAZip();

  const count = tail.getUint16(eocd + 10, true);
  const dirSize = tail.getUint32(eocd + 12, true);
  const dirStart = tail.getUint32(eocd + 16, true);
  if (dirStart + dirSize > blob.size) throw new NotAZip('the directory is out of bounds');

  const dir = new DataView(await blob.slice(dirStart, dirStart + dirSize).arrayBuffer());
  const decoder = new TextDecoder();
  const entries: ZipEntry[] = [];

  let at = 0;
  for (let i = 0; i < count && at + 46 <= dir.byteLength; i++) {
    if (dir.getUint32(at, true) !== 0x0201_4b50) break;
    const method = dir.getUint16(at + 10, true);
    const compressedSize = dir.getUint32(at + 20, true);
    const size = dir.getUint32(at + 24, true);
    const nameLen = dir.getUint16(at + 28, true);
    const extraLen = dir.getUint16(at + 30, true);
    const commentLen = dir.getUint16(at + 32, true);
    const offset = dir.getUint32(at + 42, true);
    const path = decoder.decode(new Uint8Array(dir.buffer, dir.byteOffset + at + 46, nameLen));

    // Directories are entries too, and they are not files.
    if (!path.endsWith('/')) {
      entries.push({
        path,
        name: path.split('/').pop() ?? path,
        size,
        method,
        compressedSize,
        offset,
      });
    }
    at += 46 + nameLen + extraLen + commentLen;
  }

  return entries;
}

/** Pulls one entry out, inflating it if it needs it. */
export async function readZipEntry(blob: Blob, entry: ZipEntry): Promise<Uint8Array> {
  // The local header repeats the name and extra field, and its lengths are the
  // ones that count: some writers put a different extra field in each copy.
  const header = new DataView(await blob.slice(entry.offset, entry.offset + 30).arrayBuffer());
  if (header.byteLength < 30 || header.getUint32(0, true) !== 0x0403_4b50) {
    throw new NotAZip(`${entry.path}: the entry's header is not where the directory says`);
  }
  const nameLen = header.getUint16(26, true);
  const extraLen = header.getUint16(28, true);
  const start = entry.offset + 30 + nameLen + extraLen;
  const data = blob.slice(start, start + entry.compressedSize);

  if (entry.method === 0) return new Uint8Array(await data.arrayBuffer());
  if (entry.method !== 8) {
    throw new NotAZip(`${entry.path}: compressed with method ${entry.method}, which we cannot read`);
  }

  // `deflate-raw` is deflate with no zlib wrapper, which is what a zip stores.
  const inflated = data.stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(inflated).arrayBuffer());
}

/** Whether the browser can inflate at all. */
export function canReadZips(): boolean {
  return typeof DecompressionStream !== 'undefined';
}

export function isVpx(name: string): boolean {
  return name.toLowerCase().endsWith('.vpx');
}

export function isZip(name: string): boolean {
  return name.toLowerCase().endsWith('.zip');
}
