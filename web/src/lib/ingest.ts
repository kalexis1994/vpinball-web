// Working out what a pile of dropped files actually contains.
//
// The naive version of this asks the user to sort their own downloads: tables
// here, ROMs there, and unpack the archive first. That is the computer refusing
// to do arithmetic it is perfectly capable of. What arrives from a table site is
// one zip with a table, its ROM, and a readme in it, or a table one week and its
// ROM the next.
//
// So the order of operations is: find every table, ask each one which machine it
// is — which means reading its script, because the VPX format has no field for
// it — and only then go looking for that firmware. Looking for ROMs first would
// mean not knowing which of them matter.
//
// The second half is the one that makes this worth writing. The ROMs found are
// matched against the tables **already in the library**, not only the ones
// arriving. A table stored months ago without its firmware is not a mistake to
// be corrected; it is a table waiting, and the moment its ROM turns up in
// somebody's zip it should start working. Nothing about that requires the user
// to remember which table it was for.

import { listZip, NotAZip, readZipEntry, isVpx, isZip, type ZipEntry } from './archive';
import { loadLibrary, listRoms, parseTable } from './library';
import type { ParsedTable, RomStatus, TableEntry } from './types';

/** A table found somewhere in what was dropped. */
export interface FoundTable {
  /** Stable key for React, and for the accept step. */
  id: string;
  /** Where it came from, written for a person: `pack.zip → tables/f14.vpx`. */
  origin: string;
  fileName: string;
  size: number;
  file: File;
  /** What the parse found, or null if it could not be read. */
  parsed: ParsedTable | null;
  error: string | null;
  /** The PinMAME set it needs, lowercased, or null if it needs none. */
  romSet: string | null;
  romStatus: RomStatus;
  /** Other sets its script mentions. */
  alternates: string[];
  /** Whether the library already holds this exact file, so this replaces it. */
  replaces: TableEntry | null;
}

/** A ROM set found somewhere in what was dropped. */
export interface FoundRom {
  id: string;
  /** Set name, lowercased: `f14_l1`. */
  set: string;
  origin: string;
  size: number;
  file: File;
  /** Whether the library already has a ROM under this name. */
  replaces: boolean;
}

/** A table already in the library that one of the incoming ROMs completes. */
export interface Adoption {
  table: TableEntry;
  set: string;
}

/** An incoming ROM that some table mentions, but not as the one it asks for. */
export interface AlternateMatch {
  rom: FoundRom;
  /** The table that mentions it. */
  tableName: string;
  /** What that table actually asks for. */
  asks: string;
}

export interface Plan {
  tables: FoundTable[];
  roms: FoundRom[];
  /** Library tables that stop being orphans if this is accepted. */
  adopted: Adoption[];
  /** Incoming tables that will still be waiting for firmware afterwards. */
  waiting: FoundTable[];
  /** ROMs nothing here and nothing stored asks for. */
  unclaimed: FoundRom[];
  /** ROMs that match an alternate rather than what the table asks for. */
  alternates: AlternateMatch[];
  /** Files that could not be read, and why. */
  errors: string[];
  /** True when there is nothing at all to do. */
  empty: boolean;
}

/** What the caller is told while a big archive is being read. */
export interface Progress {
  stage: 'opening' | 'reading' | 'matching';
  /** The file being worked on. */
  name: string;
  done: number;
  total: number;
}

/** A candidate pulled out of the input, before anything is known about it. */
interface Candidate {
  origin: string;
  name: string;
  bytes: () => Promise<Uint8Array>;
  size: number;
}

/**
 * Looks at what was dropped and says what would happen, without doing any of it.
 *
 * Nothing is written here. The result is what the review modal shows, and
 * `applyPlan` is what carries it out.
 */
export async function planImport(
  files: File[],
  onProgress?: (p: Progress) => void,
): Promise<Plan> {
  const errors: string[] = [];
  const tableFiles: Candidate[] = [];
  const romFiles: Candidate[] = [];

  // --- Unpack ---------------------------------------------------------------

  for (const file of files) {
    onProgress?.({ stage: 'opening', name: file.name, done: 0, total: files.length });

    if (isVpx(file.name)) {
      tableFiles.push(fromFile(file));
      continue;
    }
    if (!isZip(file.name)) {
      errors.push(`${file.name}: not a .vpx table or a .zip`);
      continue;
    }

    let entries: ZipEntry[];
    try {
      entries = await listZip(file);
    } catch (e) {
      errors.push(`${file.name}: ${message(e)}`);
      continue;
    }

    // Is this a ROM set, or a bag with tables and ROM sets in it?
    //
    // A PinMAME ROM zip holds the machine's raw memory images — `u26`, `.rom`,
    // `.bin` — and never a table or another zip. So anything holding either of
    // those is an archive to be opened, and anything else is firmware named
    // after the file it arrived in.
    const inner = entries.filter((e) => isVpx(e.name) || isZip(e.name));
    if (inner.length === 0) {
      romFiles.push(fromFile(file));
      continue;
    }

    for (const entry of inner) {
      const origin = `${file.name} → ${entry.path}`;
      const candidate: Candidate = {
        origin,
        name: entry.name,
        size: entry.size,
        bytes: () => readZipEntry(file, entry),
      };
      (isVpx(entry.name) ? tableFiles : romFiles).push(candidate);
    }
  }

  // --- Read the tables ------------------------------------------------------
  //
  // Before the ROMs, and this is the whole reason the flow is shaped this way:
  // until a table has been read there is no telling which firmware matters.

  const stored = await loadLibrary().catch(() => [] as TableEntry[]);
  const tables: FoundTable[] = [];

  for (const [i, candidate] of tableFiles.entries()) {
    onProgress?.({
      stage: 'reading',
      name: candidate.name,
      done: i,
      total: tableFiles.length,
    });

    let file: File;
    try {
      file = new File([await candidate.bytes() as BlobPart], candidate.name);
    } catch (e) {
      errors.push(`${candidate.origin}: ${message(e)}`);
      continue;
    }

    let parsed: ParsedTable | null = null;
    let error: string | null = null;
    try {
      parsed = await parseTable(await file.arrayBuffer());
    } catch (e) {
      error = message(e);
      errors.push(`${candidate.origin}: ${error}`);
    }

    const rom = parsed?.meta.rom;
    tables.push({
      id: `t${i}`,
      origin: candidate.origin,
      fileName: candidate.name,
      size: file.size,
      file,
      parsed,
      error,
      romSet: rom?.name ? rom.name.toLowerCase() : null,
      romStatus: rom?.status ?? 'unknown',
      alternates: (rom?.alternates ?? []).map((a) => a.toLowerCase()),
      replaces:
        stored.find((e) => e.fileName === candidate.name && e.fileSize === file.size) ?? null,
    });
  }

  // --- And now the ROMs -----------------------------------------------------

  onProgress?.({ stage: 'matching', name: '', done: 0, total: 0 });

  const held = new Set(await listRoms());
  const roms: FoundRom[] = [];
  const seen = new Set<string>();

  for (const [i, candidate] of romFiles.entries()) {
    const set = candidate.name.replace(/\.zip$/i, '').toLowerCase();
    // The same set twice in one drop — a zip and a loose copy, say. The first
    // wins; taking both would just write one over the other.
    if (seen.has(set)) continue;
    seen.add(set);

    let file: File;
    try {
      file = new File([await candidate.bytes() as BlobPart], candidate.name);
    } catch (e) {
      errors.push(`${candidate.origin}: ${message(e)}`);
      continue;
    }

    roms.push({
      id: `r${i}`,
      set,
      origin: candidate.origin,
      size: file.size,
      file,
      replaces: held.has(set),
    });
  }

  // --- Match ----------------------------------------------------------------

  const arriving = new Set(roms.map((r) => r.set));
  const willHave = new Set([...held, ...arriving]);

  // Tables already stored, waiting for firmware, that one of these completes.
  const adopted: Adoption[] = [];
  for (const table of stored) {
    const set = table.rom.name?.toLowerCase() ?? null;
    if (table.rom.status !== 'required' || set === null) continue;
    if (held.has(set) || !arriving.has(set)) continue;
    // A table being replaced in this same import is not an adoption: it is
    // reported once, as the table it is.
    if (tables.some((t) => t.replaces?.id === table.id)) continue;
    adopted.push({ table, set });
  }

  const waiting = tables.filter(
    (t) => t.romStatus === 'required' && t.romSet !== null && !willHave.has(t.romSet),
  );

  // Which sets anything at all is asking for, incoming or stored.
  const wanted = new Set<string>();
  for (const t of tables) if (t.romSet) wanted.add(t.romSet);
  for (const t of stored) if (t.rom.name) wanted.add(t.rom.name.toLowerCase());

  const unclaimed = roms.filter((r) => !wanted.has(r.set));

  // A ROM nothing asks for by name, but that some table names as another
  // version of itself. Worth saying: it is almost certainly the right machine
  // and the wrong revision, and the table will not start on it.
  const alternates: AlternateMatch[] = [];
  for (const rom of unclaimed) {
    const incoming = tables.find((t) => t.alternates.includes(rom.set) && t.romSet);
    if (incoming) {
      alternates.push({
        rom,
        tableName: incoming.parsed?.meta.tableName ?? incoming.fileName,
        asks: incoming.romSet!,
      });
      continue;
    }
    const existing = stored.find(
      (t) => t.rom.alternates.some((a) => a.toLowerCase() === rom.set) && t.rom.name,
    );
    if (existing) {
      alternates.push({
        rom,
        tableName: existing.tableName ?? existing.fileName,
        asks: existing.rom.name!.toLowerCase(),
      });
    }
  }

  return {
    tables,
    roms,
    adopted,
    waiting,
    unclaimed,
    alternates,
    errors,
    empty: tables.length === 0 && roms.length === 0,
  };
}

/** Carries out a plan the user has accepted. */
export async function applyPlan(
  plan: Plan,
  add: {
    table: (file: File, parsed: ParsedTable | null) => Promise<unknown>;
    rom: (file: File) => Promise<unknown>;
  },
  onProgress?: (p: Progress) => void,
): Promise<string[]> {
  const failures: string[] = [];
  const total = plan.tables.length + plan.roms.length;
  let done = 0;

  // ROMs first. They are small and quick, and doing them before the tables
  // means a table stored halfway through a long import is already playable.
  for (const rom of plan.roms) {
    onProgress?.({ stage: 'reading', name: rom.file.name, done: done++, total });
    try {
      await add.rom(rom.file);
    } catch (e) {
      failures.push(`${rom.origin}: ${message(e)}`);
    }
  }

  for (const table of plan.tables) {
    onProgress?.({ stage: 'reading', name: table.fileName, done: done++, total });
    if (!table.parsed) {
      // It could not be read when the plan was built, and nothing has changed.
      failures.push(`${table.origin}: ${table.error ?? 'could not be read'}`);
      continue;
    }
    try {
      await add.table(table.file, table.parsed);
    } catch (e) {
      failures.push(`${table.origin}: ${message(e)}`);
    }
  }

  return failures;
}

function fromFile(file: File): Candidate {
  return {
    origin: file.name,
    name: file.name,
    size: file.size,
    bytes: async () => new Uint8Array(await file.arrayBuffer()),
  };
}

function message(e: unknown): string {
  if (e instanceof NotAZip) return e.message;
  return e instanceof Error ? e.message : String(e);
}
