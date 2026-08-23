// What was found, before anything is written.
//
// The modal exists because an import from a table site is rarely just "here are
// two files". A zip turns out to hold three tables and one ROM; the ROM belongs
// to a table added last month; one of the tables is a newer copy of one already
// stored. All of that is worth seeing *before* it happens, not discovering
// afterwards from a list that has changed.
//
// It is written to be read top to bottom and answer one question: after
// Accept, what is different? So the sections are consequences, not contents —
// "these will play", "this one is still waiting for firmware", "this replaces
// what you had" — and the plain inventory comes last.

import { useEffect, useRef } from 'react';
import type { Plan } from '../lib/ingest';
import { formatBytes } from '../lib/types';

interface Props {
  plan: Plan;
  busy: boolean;
  onAccept: () => void;
  onCancel: () => void;
}

export function ImportReview({ plan, busy, onAccept, onCancel }: Props) {
  const dialog = useRef<HTMLDivElement>(null);

  // Escape closes it, and the focus starts inside so a keyboard can reach the
  // buttons without tabbing through the page behind.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) onCancel();
    };
    window.addEventListener('keydown', onKey);
    dialog.current?.focus();
    return () => window.removeEventListener('keydown', onKey);
  }, [busy, onCancel]);

  const nothing = plan.empty;

  return (
    <div className="modal-backdrop" onClick={() => !busy && onCancel()}>
      <div
        ref={dialog}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label="Review what was found"
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="modal-head">
          <h2>{nothing ? 'Nothing to add' : 'Found in what you dropped'}</h2>
          <p className="modal-sub">{summary(plan)}</p>
        </header>

        <div className="modal-body">
          {plan.adopted.length > 0 && (
            <Section
              tone="ok"
              title="Completes a table you already have"
              note="These were stored without their firmware. They will play after this."
            >
              {plan.adopted.map(({ table, set }) => (
                <Row
                  key={table.id}
                  name={table.tableName ?? table.fileName}
                  detail={
                    <>
                      gets <span className="mono">{set}</span>
                    </>
                  }
                />
              ))}
            </Section>
          )}

          {plan.tables.length > 0 && (
            <Section title="Tables" count={plan.tables.length}>
              {plan.tables.map((t) => (
                <Row
                  key={t.id}
                  name={t.parsed?.meta.tableName ?? t.fileName}
                  detail={
                    <>
                      {t.origin !== t.fileName && <span className="row-origin">{t.origin}</span>}
                      <span>{formatBytes(t.size)}</span>
                      {t.replaces && <span className="tag tag-warn">replaces</span>}
                    </>
                  }
                  status={
                    t.error ? (
                      <span className="tag tag-error">unreadable</span>
                    ) : t.romStatus !== 'required' ? (
                      <span className="tag tag-ok">no ROM needed</span>
                    ) : plan.waiting.includes(t) ? (
                      <span className="tag tag-warn">
                        needs <span className="mono">{t.romSet}</span>
                      </span>
                    ) : (
                      <span className="tag tag-ok">
                        <span className="mono">{t.romSet}</span> ✓
                      </span>
                    )
                  }
                />
              ))}
            </Section>
          )}

          {plan.roms.length > 0 && (
            <Section title="ROMs" count={plan.roms.length}>
              {plan.roms.map((r) => (
                <Row
                  key={r.id}
                  name={<span className="mono">{r.set}</span>}
                  detail={
                    <>
                      {r.origin !== r.file.name && <span className="row-origin">{r.origin}</span>}
                      <span>{formatBytes(r.size)}</span>
                      {r.replaces && <span className="tag tag-warn">replaces</span>}
                    </>
                  }
                />
              ))}
            </Section>
          )}

          {plan.waiting.length > 0 && (
            <Section
              tone="warn"
              title="Will still be waiting for firmware"
              note="They will be stored and listed. Drop the ROM in whenever you have it — this screen will match it up on its own."
            >
              {plan.waiting.map((t) => (
                <Row
                  key={t.id}
                  name={t.parsed?.meta.tableName ?? t.fileName}
                  detail={
                    <>
                      needs <span className="mono">{t.romSet}.zip</span>
                    </>
                  }
                />
              ))}
            </Section>
          )}

          {plan.alternates.length > 0 && (
            <Section
              tone="warn"
              title="Right machine, other revision"
              note="These will be kept, but the table asks for a different version and will not start on them."
            >
              {plan.alternates.map(({ rom, tableName, asks }) => (
                <Row
                  key={rom.id}
                  name={<span className="mono">{rom.set}</span>}
                  detail={
                    <>
                      {tableName} asks for <span className="mono">{asks}</span>
                    </>
                  }
                />
              ))}
            </Section>
          )}

          {plan.unclaimed.filter((r) => !plan.alternates.some((a) => a.rom.id === r.id)).length >
            0 && (
            <Section
              title="Nothing here asks for these"
              note="Kept anyway: the table that wants them may arrive later."
            >
              {plan.unclaimed
                .filter((r) => !plan.alternates.some((a) => a.rom.id === r.id))
                .map((r) => (
                  <Row key={r.id} name={<span className="mono">{r.set}</span>} />
                ))}
            </Section>
          )}

          {plan.errors.length > 0 && (
            <Section tone="error" title="Could not be read">
              {plan.errors.map((e, i) => (
                <Row key={i} name={e} />
              ))}
            </Section>
          )}
        </div>

        <footer className="modal-foot">
          <button className="btn btn-ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={onAccept} disabled={busy || nothing}>
            {busy ? 'Adding…' : accept(plan)}
          </button>
        </footer>
      </div>
    </div>
  );
}

function Section({
  title,
  count,
  note,
  tone,
  children,
}: {
  title: string;
  count?: number;
  note?: string;
  tone?: 'ok' | 'warn' | 'error';
  children: React.ReactNode;
}) {
  return (
    <section className={`review-section${tone ? ` review-${tone}` : ''}`}>
      <h3>
        {title}
        {count !== undefined && <span className="count">{count}</span>}
      </h3>
      {note && <p className="review-note">{note}</p>}
      <ul className="review-list">{children}</ul>
    </section>
  );
}

function Row({
  name,
  detail,
  status,
}: {
  name: React.ReactNode;
  detail?: React.ReactNode;
  status?: React.ReactNode;
}) {
  return (
    <li className="review-row">
      <span className="review-name">
        {name}
        {detail && <span className="review-detail">{detail}</span>}
      </span>
      {status}
    </li>
  );
}

function summary(plan: Plan): string {
  if (plan.empty) {
    return plan.errors.length > 0
      ? 'Nothing in there could be read.'
      : 'No tables and no ROMs in there.';
  }
  const parts: string[] = [];
  if (plan.tables.length > 0) {
    parts.push(`${plan.tables.length} ${plan.tables.length === 1 ? 'table' : 'tables'}`);
  }
  if (plan.roms.length > 0) {
    parts.push(`${plan.roms.length} ROM ${plan.roms.length === 1 ? 'set' : 'sets'}`);
  }
  const found = parts.join(' and ');
  return plan.adopted.length > 0
    ? `${found}, and ${plan.adopted.length} stored ${plan.adopted.length === 1 ? 'table' : 'tables'} finally gets what it was waiting for.`
    : `${found}.`;
}

function accept(plan: Plan): string {
  const n = plan.tables.length + plan.roms.length;
  return n === 1 ? 'Add it' : `Add all ${n}`;
}
