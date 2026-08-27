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

const KEY = 'vpw.settings';

/** Where the player looks at the machine from. */
export type CameraView = 'front' | 'overhead';

export const CAMERA_VIEWS: readonly CameraView[] = ['front', 'overhead'];

/** What room the machine stands in: the table's own light, or a real one. */
export type Environment = 'table' | 'bar';

export const ENVIRONMENTS: readonly Environment[] = ['table', 'bar'];

export interface Settings {
  /** Master volume, 0 to 1. */
  volume: number;
  /** Which of the named views the camera starts in. */
  camera: CameraView;
  /**
   * Day/night, 0 to 1, or `null` for the table's own. Plenty of tables are
   * authored dark on purpose — F-14 asks for 8% — and Visual Pinball's own
   * settings carry exactly this override for exactly that reason.
   */
  brightness: number | null;
  /**
   * The room the machine stands in. `table` is the environment map the table
   * itself asked for; `bar` is a real room's HDR — dim walls, a few bright
   * lamps — which is mostly seen reflected in the steel and the plastics.
   */
  environment: Environment;
}

const DEFAULTS: Settings = {
  // Not 1: a table's own mix already runs close to full scale, and a player who
  // wants it louder can say so, where one deafened on the first table may not
  // stay to find the slider.
  volume: 0.7,
  // Standing in front of the machine, because that is what it looks like and
  // the first thing anybody wants is to see what they have loaded. The overhead
  // view is the one you switch to once you want to *play* it.
  camera: 'front',
  // The table's own, until the player says otherwise: the author lit the room.
  brightness: null,
  // The table's own: the author lit the room here too.
  environment: 'table',
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
  if (s.brightness === null) {
    out.brightness = null;
  } else if (typeof s.brightness === 'number' && Number.isFinite(s.brightness)) {
    out.brightness = Math.min(1, Math.max(0, s.brightness));
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
