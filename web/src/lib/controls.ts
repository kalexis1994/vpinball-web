// What the player can be asked to do, and what asks it.
//
// The player's own vocabulary is fixed: it speaks the key codes Visual Pinball
// speaks — `KeyZ` is a left flipper, `Digit5` is a coin — because that is what
// a table's own script listens for, and rebinding a key must not change what
// the table hears. So the binding lives *here*, on the page: a physical key or
// a gamepad button is looked up, and the action it stands for is sent on in
// the player's words.
//
// Keyboard and gamepad keep separate maps on purpose. They are different
// instruments — one has a Z and the other has a shoulder button — and a
// player who moves the left flipper on the pad has said nothing at all about
// where it is on the keyboard.

/** Everything a player can bind. */
export type ActionId =
  | 'leftFlipper'
  | 'rightFlipper'
  | 'plunger'
  | 'nudgeLeft'
  | 'nudgeRight'
  | 'nudgeCenter'
  | 'coin'
  | 'start'
  | 'newBall';

export interface Action {
  id: ActionId;
  label: string;
  /** What the player is told, in its own vocabulary. See the note above. */
  code: string;
  /** Held rather than tapped: the plunger's strength is how long it was down. */
  held: boolean;
  hint?: string;
}

/**
 * The actions, in the order a person thinks of them: the two flippers first,
 * because that is the game.
 */
export const ACTIONS: readonly Action[] = [
  { id: 'leftFlipper', label: 'Left flipper', code: 'KeyZ', held: true },
  { id: 'rightFlipper', label: 'Right flipper', code: 'KeyM', held: true },
  {
    id: 'plunger',
    label: 'Plunger',
    code: 'Space',
    held: true,
    hint: 'Hold to draw it back, let go to shoot.',
  },
  { id: 'nudgeLeft', label: 'Nudge left', code: 'ArrowLeft', held: false },
  { id: 'nudgeRight', label: 'Nudge right', code: 'ArrowRight', held: false },
  { id: 'nudgeCenter', label: 'Nudge forward', code: 'ArrowUp', held: false },
  { id: 'coin', label: 'Insert coin', code: 'Digit5', held: false },
  { id: 'start', label: 'Start', code: 'Digit1', held: false },
  {
    id: 'newBall',
    label: 'New ball',
    code: 'Enter',
    held: false,
    hint: 'Puts a ball back in the lane and clears a tilt.',
  },
];

export type KeyMap = Record<ActionId, string>;
/** A gamepad button index, or `null` for "not bound to this pad". */
export type PadMap = Record<ActionId, number | null>;

/** The keyboard as the machine itself is labelled. */
export const DEFAULT_KEYS: KeyMap = {
  leftFlipper: 'KeyZ',
  rightFlipper: 'KeyM',
  plunger: 'Space',
  nudgeLeft: 'ArrowLeft',
  nudgeRight: 'ArrowRight',
  nudgeCenter: 'ArrowUp',
  coin: 'Digit5',
  start: 'Digit1',
  newBall: 'Enter',
};

/**
 * The pad, in the standard mapping every browser reports controllers in.
 *
 * The shoulders are the flippers because that is where a cabinet's flipper
 * buttons are — under the fingers, one each side — and everything else falls
 * where a console player expects it.
 */
export const DEFAULT_PAD: PadMap = {
  leftFlipper: 4,
  rightFlipper: 5,
  plunger: 0,
  nudgeLeft: 14,
  nudgeRight: 15,
  nudgeCenter: 12,
  coin: 8,
  start: 9,
  newBall: 3,
};

/** The standard mapping's buttons, as the pad in a hand is labelled. */
const PAD_LABELS: Record<number, string> = {
  0: 'A / ✕',
  1: 'B / ○',
  2: 'X / □',
  3: 'Y / △',
  4: 'L1',
  5: 'R1',
  6: 'L2',
  7: 'R2',
  8: 'Select',
  9: 'Start',
  10: 'L3',
  11: 'R3',
  12: 'D-pad up',
  13: 'D-pad down',
  14: 'D-pad left',
  15: 'D-pad right',
  16: 'Guide',
};

export function padLabel(button: number | null): string {
  if (button === null) return 'Not bound';
  return PAD_LABELS[button] ?? `Button ${button}`;
}

/** A `KeyboardEvent.code` as a person would write it on a keycap. */
export function keyLabel(code: string): string {
  if (!code) return 'Not bound';
  const named: Record<string, string> = {
    Space: 'Space',
    Enter: 'Enter',
    ArrowLeft: '←',
    ArrowRight: '→',
    ArrowUp: '↑',
    ArrowDown: '↓',
    ShiftLeft: 'Left Shift',
    ShiftRight: 'Right Shift',
    ControlLeft: 'Left Ctrl',
    ControlRight: 'Right Ctrl',
    AltLeft: 'Left Alt',
    AltRight: 'Right Alt',
    Backslash: '\\',
    Slash: '/',
    Comma: ',',
    Period: '.',
    Semicolon: ';',
    Quote: "'",
    BracketLeft: '[',
    BracketRight: ']',
    Minus: '−',
    Equal: '=',
    Backquote: '`',
    Tab: 'Tab',
  };
  if (named[code]) return named[code];
  if (code.startsWith('Key')) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (code.startsWith('Numpad')) return `Numpad ${code.slice(6)}`;
  return code;
}

/** Which action a physical key stands for, if any. */
export function actionForKey(keys: KeyMap, code: string): Action | undefined {
  return ACTIONS.find((a) => keys[a.id] === code);
}

/** Which action a pad button stands for, if any. */
export function actionForButton(pad: PadMap, button: number): Action | undefined {
  return ACTIONS.find((a) => pad[a.id] === button);
}

/**
 * A map with anything unrecognised dropped and anything missing filled in.
 *
 * Bindings come out of storage written by an older version of this page, or
 * by nothing at all. What must never happen is a table with no flippers
 * because a key this build does not know sat in the slot.
 */
export function cleanKeys(stored: unknown): KeyMap {
  const out = { ...DEFAULT_KEYS };
  if (stored && typeof stored === 'object') {
    for (const a of ACTIONS) {
      const v = (stored as Record<string, unknown>)[a.id];
      if (typeof v === 'string' && v.length > 0) out[a.id] = v;
    }
  }
  return out;
}

export function cleanPad(stored: unknown): PadMap {
  const out = { ...DEFAULT_PAD };
  if (stored && typeof stored === 'object') {
    for (const a of ACTIONS) {
      const v = (stored as Record<string, unknown>)[a.id];
      if (v === null) out[a.id] = null;
      else if (typeof v === 'number' && Number.isInteger(v) && v >= 0 && v < 32) {
        out[a.id] = v;
      }
    }
  }
  return out;
}
