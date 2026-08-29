// What the player remembers between visits.
//
// Deliberately not IndexedDB: that is where the tables live, it is asynchronous,
// and a setting has to be readable before the first frame is drawn.
// `localStorage` is synchronous and tiny, which is exactly the shape of a
// handful of preferences.
//
// Nothing here imports the audio engine, and that is on purpose. A setting is a
// number the page remembers; *applying* it belongs to whatever owns the thing
// being set. So this publishes changes and the audio subscribes, which also
// means moving the volume slider does not drag in three megabytes of wasm to
// find out nobody is listening yet.

import { cleanKeys, cleanPad, type KeyMap, type PadMap } from './controls';

const KEY = 'vpw.settings';

/**
 * How far above "as recorded" either half of the mix can be pushed.
 *
 * The same ceiling the player clamps to. It exists because plenty of ROMs
 * were mastered quietly, and a table cannot be balanced against one of those
 * by turning the table down — that only makes the whole machine quiet.
 */
export const MAX_MIX_GAIN = 1.2;

/** Where the player looks at the machine from. */
export type CameraView = 'front' | 'cabinet' | 'overhead';

export const CAMERA_VIEWS: readonly CameraView[] = ['front', 'cabinet', 'overhead'];

/** Which gutter the floating score panel stands in, seen from above. */
export type ScoreSide = 'left' | 'right';

export const SCORE_SIDES: readonly ScoreSide[] = ['left', 'right'];

/**
 * Where that panel goes when there is no gutter to stand in — a phone held
 * upright, which is the case the overhead view exists for. Docking it costs
 * the table a strip of screen, and the table shrinks to keep its shape, so
 * "hidden" stays on offer for a player who would rather have every pixel.
 */
export type ScoreDock = 'top' | 'bottom' | 'hidden';

export const SCORE_DOCKS: readonly ScoreDock[] = ['top', 'bottom', 'hidden'];

/** What room the machine stands in: the table's own light, or a real one. */
export type Environment = 'table' | 'bar';

export const ENVIRONMENTS: readonly Environment[] = ['table', 'bar'];

export interface Settings {
  /** Master volume, 0 to 1. */
  volume: number;
  /**
   * The flat engine: the table photographed once and played as pictures,
   * with only the moving pieces in real 3D — for machines whose GPU cannot
   * afford the full render. While it is on the camera is the photograph's,
   * so the view choice below does not apply.
   */
  flat: boolean;
  /**
   * Whether the resolution governor runs: it drops the render resolution to
   * hold sixty frames and climbs back when there is room. Off, the picture
   * stays at full resolution whatever the frame rate does.
   */
  adaptive: boolean;
  /** Which of the named views the camera starts in. */
  camera: CameraView;
  /**
   * The room the machine stands in. `table` is the environment map the table
   * itself asked for; `bar` is a real room's HDR — dim walls, a few bright
   * lamps — which is mostly seen reflected in the steel and the plastics.
   */
  environment: Environment;
  /**
   * The balance between the two halves of a machine's noise, each 0 to 1 and
   * both under {@link Settings.volume}: what the game *says* — its board's
   * music, speech and effects — against what the table *does* when it is hit.
   */
  volumeMachine: number;
  volumeTable: number;
  /** The ceiling on both, matching the player's own. Above one is a boost. */
  /** What each key on the keyboard does. See `lib/controls`. */
  keys: KeyMap;
  /** And each button on a gamepad, kept apart from the keyboard's. */
  pad: PadMap;
  /** Which side the overhead view's score panel floats on. */
  scoreSide: ScoreSide;
  /** Where it docks when the screen is too narrow for either gutter. */
  scoreDock: ScoreDock;
}

const DEFAULTS: Settings = {
  // Not 1: a table's own mix already runs close to full scale, and a player who
  // wants it louder can say so, where one deafened on the first table may not
  // stay to find the slider.
  volume: 0.7,
  // The full renderer: the flat engine is the low-end escape hatch, and the
  // machine that needs it is the one that turns it on.
  flat: false,
  // On: sixty frames out of the box is the better first impression, and the
  // player who wants every pixel can say so here.
  adaptive: true,
  // Standing in front of the machine, because that is what it looks like and
  // the first thing anybody wants is to see what they have loaded. The overhead
  // view is the one you switch to once you want to *play* it.
  camera: 'front',
  // The bar: a machine stands somewhere, and somewhere with lamps in it is a
  // better first impression than a void. The table's own light is one tap
  // away for whoever wants the authored look.
  environment: 'bar',
  // Both halves at full: the balance the tables and the ROMs were authored
  // against, and the place to start from when changing it.
  volumeMachine: 1,
  volumeTable: 1,
  // The machine's own labelling, and a pad laid out the way a console
  // player expects. Both live in `lib/controls`, which is also what knows
  // how to read one back that an older version of this page wrote.
  keys: cleanKeys(null),
  pad: cleanPad(null),
  // The left, because a right-handed player's hand rests over the right of a
  // phone and the flippers are what they are watching anyway.
  scoreSide: 'left',
  // Above the table: it is where a machine's head is, and the strip it takes
  // comes off the top of the playfield, which is the end nothing is decided
  // at.
  scoreDock: 'top',
};

let current: Settings = load();
const listeners = new Set<(s: Settings) => void>();

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULTS };
    const stored = JSON.parse(raw) as Partial<Settings>;
    return { ...DEFAULTS, ...clean(stored) };
  } catch {
    // A private window, cleared site data, or something else's key in the way.
    // Defaults are a perfectly good answer to all three.
    return { ...DEFAULTS };
  }
}

/** Keeps only what we recognise, in range. Anything else is somebody else's. */
function clean(s: Partial<Settings>): Partial<Settings> {
  const out: Partial<Settings> = {};
  if (typeof s.volume === 'number' && Number.isFinite(s.volume)) {
    out.volume = Math.min(1, Math.max(0, s.volume));
  }
  // A view this version does not know is left out rather than kept: it would
  // reach the player as a name it cannot resolve, and a camera stuck somewhere
  // nobody asked for is worse than one at the default.
  if (CAMERA_VIEWS.includes(s.camera as CameraView)) {
    out.camera = s.camera as CameraView;
  }
  if (ENVIRONMENTS.includes(s.environment as Environment)) {
    out.environment = s.environment as Environment;
  }
  if (typeof s.flat === 'boolean') {
    out.flat = s.flat;
  }
  if (typeof s.adaptive === 'boolean') {
    out.adaptive = s.adaptive;
  }
  for (const k of ['volumeMachine', 'volumeTable'] as const) {
    const v = s[k];
    if (typeof v === 'number' && Number.isFinite(v)) {
      out[k] = Math.min(MAX_MIX_GAIN, Math.max(0, v));
    }
  }
  // Always rebuilt rather than trusted: a binding map is the one setting
  // that can leave the player unable to flip, and a key this build does not
  // know sitting in a slot is exactly how that happens.
  if (s.keys !== undefined) {
    out.keys = cleanKeys(s.keys);
  }
  if (s.pad !== undefined) {
    out.pad = cleanPad(s.pad);
  }
  if (SCORE_SIDES.includes(s.scoreSide as ScoreSide)) {
    out.scoreSide = s.scoreSide as ScoreSide;
  }
  if (SCORE_DOCKS.includes(s.scoreDock as ScoreDock)) {
    out.scoreDock = s.scoreDock as ScoreDock;
  }
  return out;
}

export function settings(): Settings {
  return current;
}

export function updateSettings(change: Partial<Settings>): Settings {
  current = { ...current, ...clean(change) };
  try {
    localStorage.setItem(KEY, JSON.stringify(current));
  } catch {
    // Out of quota or storage disabled. The setting still applies for this
    // session, which is better than refusing to change it.
  }
  for (const listener of listeners) listener(current);
  return current;
}

/** Calls `listener` on every change, and returns a function to stop. */
export function onSettingsChange(listener: (s: Settings) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
