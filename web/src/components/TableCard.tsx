import { useEffect, useState } from 'react';
import { thumbnailUrl } from '../lib/library';
import { displayName, formatBytes, type TableEntry } from '../lib/types';
import { RomBadge } from './RomBadge';

interface Props {
  table: TableEntry;
  onPlay: (table: TableEntry) => void;
  /** Left out where removing does not belong — the Play screen is for picking
   * a table, and a delete button beside every one of them is an accident
   * waiting to happen. */
  onRemove?: (table: TableEntry) => void;
  /** Whether the library has the ROM this table needs. */
  hasRom: boolean;
}

export function TableCard({ table, onPlay, onRemove, hasRom }: Props) {
  const thumb = useThumbnail(table);

  return (
    <article className="card">
      <div className="card-thumb">
        {thumb ? (
          <img src={thumb} alt="" />
        ) : (
          <span className="card-thumb-empty" aria-hidden="true">
            🎱
          </span>
        )}
      </div>

      <div className="card-body">
        <h3 title={table.fileName}>{displayName(table)}</h3>
        <p className="card-meta">
          {table.author ?? 'unknown author'}
          {table.version && ` · v${table.version}`}
          {` · ${formatBytes(table.fileSize)}`}
        </p>

        <div className="card-badges">
          <RomBadge rom={table.rom} available={hasRom} />
          <span className="badge badge-dim" title="VPX format version">
            VPX {(table.fileVersion / 100).toFixed(1)}
          </span>
        </div>

        <p className="card-stats">
          {table.gameitemCount} items · {table.imageCount} images ·{' '}
          {table.soundCount} sounds
        </p>
      </div>

      <div className="card-actions">
        <button className="btn btn-primary" onClick={() => onPlay(table)}>
          Play
        </button>
        {onRemove && (
          <button
            className="btn btn-ghost"
            onClick={() => onRemove(table)}
            aria-label={`Remove ${displayName(table)}`}
          >
            Remove
          </button>
        )}
      </div>
    </article>
  );
}

function useThumbnail(table: TableEntry): string | null {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    let revoked = false;
    let current: string | null = null;

    void thumbnailUrl(table).then((next) => {
      if (revoked) {
        if (next) URL.revokeObjectURL(next);
        return;
      }
      current = next;
      setUrl(next);
    });

    return () => {
      revoked = true;
      if (current) URL.revokeObjectURL(current);
    };
  }, [table]);

  return url;
}
