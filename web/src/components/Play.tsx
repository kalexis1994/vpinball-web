// The tables you can play.
//
// Nothing but the list. Everything about *getting* a table — the drop zone, the
// ROM matching, what is taking up space — lives in Content, because a player
// opening this screen has already decided to play and is looking for a name.

import { useEffect, useState } from 'react';
import { listRoms, loadLibrary } from '../lib/library';
import type { TableEntry } from '../lib/types';
import { ScreenHead } from './ScreenHead';
import { TableCard } from './TableCard';

interface Props {
  onPlay: (table: TableEntry) => void;
  onBack: () => void;
  /** Sends the player to Content, which is where an empty list is fixed. */
  onAdd: () => void;
}

export function Play({ onPlay, onBack, onAdd }: Props) {
  const [tables, setTables] = useState<TableEntry[] | null>(null);
  const [roms, setRoms] = useState<string[]>([]);
  const [search, setSearch] = useState('');

  useEffect(() => {
    void loadLibrary().then(setTables);
    void listRoms().then(setRoms);
  }, []);

  const term = search.trim().toLowerCase();
  const shown =
    tables && term
      ? tables.filter((t) =>
          `${t.tableName ?? ''} ${t.fileName} ${t.author ?? ''}`.toLowerCase().includes(term),
        )
      : tables;

  return (
    <main className="shell">
      <ScreenHead title="Play" onBack={onBack}>
        {tables && tables.length > 6 && (
          <input
            className="search"
            type="search"
            placeholder="Search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label="Search tables"
          />
        )}
      </ScreenHead>

      {tables === null ? (
        <p className="notice notice-busy">Reading the library…</p>
      ) : tables.length === 0 ? (
        <div className="empty">
          <p>No tables yet.</p>
          <p className="empty-hint">
            Tables live in this browser, not on a server. Add a <code>.vpx</code> in
            Content and it will be here.
          </p>
          <button className="btn btn-primary" onClick={onAdd}>
            Go to Content
          </button>
        </div>
      ) : shown && shown.length === 0 ? (
        <div className="empty">
          <p>Nothing matches “{search}”.</p>
        </div>
      ) : (
        <div className="grid">
          {shown?.map((t) => (
            <TableCard
              key={t.id}
              table={t}
              onPlay={onPlay}
              hasRom={t.rom.name !== null && roms.includes(t.rom.name)}
            />
          ))}
        </div>
      )}
    </main>
  );
}
