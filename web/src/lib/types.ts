export type RomStatus = 'none' | 'required' | 'unknown';

export interface RomInfo {
  status: RomStatus;
  /** Name of the PinMAME set, e.g. `f14_l1`. */
  name: string | null;
  /** File to get hold of, e.g. `f14_l1.zip`. */
  zip: string | null;
  /** Other ROM versions the script mentions. */
  alternates: string[];
}

/** A table in the local library. */
export interface TableEntry {
  id: string;
  fileName: string;
  fileSize: number;
  addedAt: number;

  tableName: string | null;
  author: string | null;
  version: string | null;
  releaseDate: string | null;
  description: string | null;

  fileVersion: number;
  gameitemCount: number;
  imageCount: number;
  soundCount: number;
  scriptLen: number;

  rom: RomInfo;
  /** MIME type of the embedded screenshot, or null if the table has none. */
  thumbMime: string | null;
}

/** What the worker returns after parsing a `.vpx`. */
export interface ParsedTable {
  meta: Omit<TableEntry, 'id' | 'fileName' | 'fileSize' | 'addedAt'>;
  screenshot: Uint8Array | null;
}

/** Title to show: plenty of old tables ship no metadata. */
export function displayName(t: TableEntry): string {
  return t.tableName ?? t.fileName.replace(/\.vpx$/i, '');
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

/** Which screen the shell is showing. */
export type Screen = 'home' | 'play' | 'content' | 'settings';
