// One instrument's worth of bindings, and the listening that changes them.
//
// Keyboard and gamepad get one of these each. They differ in what they
// listen to and in nothing else, so the difference is a prop and not a
// second component: a keyboard press arrives as an event and a pad press has
// to be polled for, and both end as "this action is now that".
//
// While a row is listening it takes the very next thing pressed, whatever it
// is — including a key that is already bound elsewhere, which is not an error
// to refuse but a swap to make: two actions on one key is a table that does
// two things at once, so the one that had it gives it up.

import { useEffect, useState } from 'react';
import {
  ACTIONS,
  keyLabel,
  padLabel,
  type ActionId,
  type KeyMap,
  type PadMap,
} from '../lib/controls';

interface Props {
  kind: 'keyboard' | 'gamepad';
  keys?: KeyMap;
  pad?: PadMap;
  onChange: (id: ActionId, value: string | number) => void;
  onReset: () => void;
}

export function Bindings({ kind, keys, pad, onChange, onReset }: Props) {
  /** Which action is waiting to be told what it is, if any. */
  const [listening, setListening] = useState<ActionId | null>(null);
  /** Whether a pad is there at all, so the section can say so. */
  const [padPresent, setPadPresent] = useState(false);

  // The keyboard, while a row is armed. On `window` and capturing, so the
  // key lands here and not in whatever had focus — a space bar would
  // otherwise press the button that started the listening.
  useEffect(() => {
    if (listening === null || kind !== 'keyboard') return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code !== 'Escape') onChange(listening, e.code);
      setListening(null);
    };
    window.addEventListener('keydown', onKey, { capture: true });
    return () => window.removeEventListener('keydown', onKey, { capture: true });
  }, [listening, kind, onChange]);

  // The pad: polled, because that is the only way a browser offers it. This
  // runs whenever the section is on screen — a controller that is plugged in
  // while the page is open should say so without a reload — and takes the
  // first button pressed while a row is armed.
  useEffect(() => {
    if (kind !== 'gamepad') return;
    let raf = 0;
    let wasDown = new Set<number>();
    const tick = () => {
      raf = requestAnimationFrame(tick);
      const pads = navigator.getGamepads?.() ?? [];
      const found = pads.find((p) => p && p.connected) ?? null;
      setPadPresent(found !== null);
      if (!found) {
        wasDown = new Set();
        return;
      }
      const down = new Set<number>();
      for (const [i, b] of found.buttons.entries()) {
        if (b.pressed || b.value > 0.5) down.add(i);
      }
      if (listening !== null) {
        for (const i of down) {
          if (!wasDown.has(i)) {
            onChange(listening, i);
            setListening(null);
            break;
          }
        }
      }
      wasDown = down;
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [kind, listening, onChange]);

  const label = (id: ActionId): string =>
    kind === 'keyboard' ? keyLabel(keys?.[id] ?? '') : padLabel(pad?.[id] ?? null);

  return (
    <div className="bindings">
      {kind === 'gamepad' && !padPresent && (
        <p className="bindings-note">
          No controller seen yet. Plug one in and press a button — a browser
          only reports a pad once it has been used.
        </p>
      )}

      <ul className="binding-list">
        {ACTIONS.map((a) => (
          <li className="binding" key={a.id}>
            <span className="binding-name">
              {a.label}
              {a.hint && <span className="binding-hint">{a.hint}</span>}
            </span>
            <button
              type="button"
              className={`binding-key${listening === a.id ? ' binding-listening' : ''}`}
              aria-label={`Change ${a.label}`}
              onClick={() => setListening((now) => (now === a.id ? null : a.id))}
            >
              {listening === a.id
                ? kind === 'keyboard'
                  ? 'Press a key…'
                  : 'Press a button…'
                : label(a.id)}
            </button>
          </li>
        ))}
      </ul>

      <div className="bindings-foot">
        <span className="setting-hint">
          {listening !== null
            ? 'Escape cancels.'
            : 'Tap a binding to change it. Taking a key from another action leaves that one unbound.'}
        </span>
        <button type="button" className="btn" onClick={onReset}>
          Reset
        </button>
      </div>
    </div>
  );
}
