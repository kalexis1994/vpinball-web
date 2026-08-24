// Getting tables and ROMs into the browser.
//
// Everything here goes into IndexedDB and stays there: no server holds a copy,
// and nothing is uploaded anywhere. That is worth saying on the screen, because
// "drop your files here" usually means the opposite.
//
// Tables and ROMs arrive through the same drop zone and are told apart by
// extension. They belong together — a ROM table is unplayable without its
// firmware, and the two are almost always downloaded in the same sitting — so
// asking which is which would be a question with an obvious answer.

import { useCallback, useEffect, useRef, useState } from 'react';
import { canReadZips } from '../lib/archive';
import { applyPlan, planImport, type Plan, type Progress } from '../lib/ingest';
import {
  addRom,
  addTable,
  listRoms,
  loadLibrary,
  removeRom,
  removeTable,
  storageAvailable,
  storageUsage,
} from '../lib/library';
import { forgetLoadedTable } from '../lib/player';
import { displayName, formatBytes, type TableEntry } from '../lib/types';
import { ImportReview } from './ImportReview';
import { ScreenHead } from './ScreenHead';
import { RomBadge } from './RomBadge';

interface Props {
  onBack: () => void;
  /** Told whenever the stored contents change, so the menu's counts follow. */
  onChange: () => void;
}


export function Content({ onBack, onChange }: Props) {
  const [tables, setTables] = useState<TableEntry[] | null>(null);
  const [roms, setRoms] = useState<string[]>([]);
  /** What the import is doing, while it is doing it. */
  const [busy, setBusy] = useState<Progress | null>(null);
  /** What was found, waiting for the user to accept it. */
  const [plan, setPlan] = useState<Plan | null>(null);
  /** True while an accepted plan is being written. */
  const [applying, setApplying] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  const [usage, setUsage] = useState<{ usage: number; quota: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    const [nextTables, nextRoms, nextUsage] = await Promise.all([
      loadLibrary(),
      listRoms(),
      storageUsage(),
    ]);
    setTables(nextTables);
    setRoms(nextRoms);
    setUsage(nextUsage);
    onChange();
  }, [onChange]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /** Look at what was dropped, then ask. Nothing is written until Accept. */
  const inspect = useCallback(async (files: File[]) => {
    if (files.length === 0) return;
    setBusy({ stage: 'opening', name: files[0].name, done: 0, total: files.length });
    try {
      setPlan(await planImport(files, setBusy));
    } catch (e) {
      setErrors((prev) => [...prev, message(e)]);
    } finally {
      setBusy(null);
    }
  }, []);

  const accept = useCallback(async () => {
    if (!plan) return;
    setApplying(true);
    const failures = await applyPlan(
      plan,
      { table: (file, parsed) => addTable(file, parsed ?? undefined), rom: addRom },
      setBusy,
    );
    setApplying(false);
    setBusy(null);
    setPlan(null);
    if (failures.length > 0) setErrors((prev) => [...prev, ...failures]);
    // Whatever was last loaded was loaded against the library as it was a
    // moment ago. If what just arrived is the ROM that table was missing,
    // reusing that load would hand the player the same machine-less table for
    // the rest of the session — the script has already asked for a controller
    // and already been told there is not one, and nothing but loading it again
    // makes it ask a second time.
    forgetLoadedTable();
    await refresh();
  }, [plan, refresh]);

  if (!storageAvailable()) {
    return (
      <main className="shell">
        <ScreenHead title="Content" onBack={onBack} />
        <p className="notice notice-error">
          This browser does not expose IndexedDB, so there is nowhere to keep a table.
          A private window is the usual reason.
        </p>
      </main>
    );
  }

  const needed = missingRoms(tables, roms);

  return (
    <main className="shell">
      <ScreenHead title="Content" onBack={onBack}>
        <button
          className="btn btn-primary"
          onClick={() => fileInput.current?.click()}
          disabled={busy !== null}
        >
          Add files
        </button>
        <input
          ref={fileInput}
          type="file"
          accept=".vpx,.zip"
          multiple
          hidden
          onChange={(e) => {
            void inspect(Array.from(e.target.files ?? []));
            e.target.value = '';
          }}
        />
      </ScreenHead>

      {busy && (
        <p className="notice notice-busy">
          {busy.stage === 'matching' ? (
            'Matching ROMs against your tables…'
          ) : (
            <>
              {busy.stage === 'opening' ? 'Opening' : 'Reading'} <strong>{busy.name}</strong>
              {busy.total > 1 && ` (${busy.done + 1} of ${busy.total})`}…
            </>
          )}
        </p>
      )}

      {errors.length > 0 && (
        <div className="notice notice-error">
          <button className="notice-close" onClick={() => setErrors([])} aria-label="Dismiss">
            ×
          </button>
          {errors.map((e, i) => (
            <p key={i}>{e}</p>
          ))}
        </div>
      )}

      <div
        className={`dropzone ${dragging ? 'dropzone-active' : ''}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          void inspect(Array.from(e.dataTransfer.files));
        }}
      >
        <div className="drop-hint">
          <p>
            Drop <code>.vpx</code> tables, <code>.zip</code> ROM sets, or a zip with both
            in it.
          </p>
          {!canReadZips() && (
            <p className="tag tag-warn">
              This browser cannot open zips, so only loose <code>.vpx</code> files will
              work here. A ROM zip is still stored as it arrives.
            </p>
          )}
          <p className="empty-hint">
            Nothing leaves this browser. A zip is opened and looked through: the tables
            are read first, to find out which machine each one is, and then their ROMs are
            looked for — in the zip, and against the tables already here that are waiting
            for firmware. You get to see all of it before anything is kept.
          </p>
        </div>
      </div>

      {needed.length > 0 && (
        <section className="section">
          <h2 className="section-head">Missing ROMs</h2>
          <p className="section-note">
            These tables are here but cannot start until their firmware is too.
          </p>
          <ul className="rom-list">
            {needed.map(({ table, set }) => (
              <li key={table.id} className="rom-row">
                <span className="rom-name mono">{set}.zip</span>
                <span className="rom-for">for {displayName(table)}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="section">
        <h2 className="section-head">
          Tables{tables && tables.length > 0 && <span className="count">{tables.length}</span>}
        </h2>
        {tables === null ? (
          <p className="section-note">Reading…</p>
        ) : tables.length === 0 ? (
          <p className="section-note">Nothing yet.</p>
        ) : (
          <ul className="stored">
            {tables.map((t) => (
              <li key={t.id} className="stored-row">
                <span className="stored-name">
                  {displayName(t)}
                  <span className="stored-sub">
                    {t.fileName} · {formatBytes(t.fileSize)}
                  </span>
                </span>
                <RomBadge rom={t.rom} available={t.rom.name !== null && roms.includes(t.rom.name)} />
                <button
                  className="btn btn-ghost"
                  onClick={() => {
                    void removeTable(t.id).then(refresh);
                  }}
                  aria-label={`Remove ${displayName(t)}`}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="section">
        <h2 className="section-head">
          ROMs{roms.length > 0 && <span className="count">{roms.length}</span>}
        </h2>
        {roms.length === 0 ? (
          <p className="section-note">Nothing yet.</p>
        ) : (
          <ul className="stored">
            {roms.map((set) => (
              <li key={set} className="stored-row">
                <span className="stored-name mono">{set}</span>
                <button
                  className="btn btn-ghost"
                  onClick={() => {
                    void removeRom(set).then(refresh);
                  }}
                  aria-label={`Remove ${set}`}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {plan && (
        <ImportReview
          plan={plan}
          busy={applying}
          onAccept={() => void accept()}
          onCancel={() => setPlan(null)}
        />
      )}

      {usage && usage.usage > 0 && (
        <p className="footnote">
          {formatBytes(usage.usage)} stored
          {usage.quota > 0 && ` of about ${formatBytes(usage.quota)} this browser allows`}.
        </p>
      )}
    </main>
  );
}

/** Tables whose ROM is named but not stored. */
function missingRoms(
  tables: TableEntry[] | null,
  roms: string[],
): { table: TableEntry; set: string }[] {
  if (!tables) return [];
  const out: { table: TableEntry; set: string }[] = [];
  for (const table of tables) {
    const set = table.rom.name;
    if (table.rom.status === 'required' && set !== null && !roms.includes(set)) {
      out.push({ table, set });
    }
  }
  return out;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
