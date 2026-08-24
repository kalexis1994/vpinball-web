import { useEffect, useRef, useState } from 'react';
import { startAudio, stopAudio } from '../lib/audio';
import { readTableFile } from '../lib/library';
import { onSettingsChange, settings, updateSettings, type CameraView } from '../lib/settings';
import {
  TELEMETRY_WINDOW_S,
  loadTable,
  loopStats,
  mark,
  newBall,
  nextCameraView,
  saveMachineState,
  setCameraView,
  setTelemetry,
  startPlayer,
  type Loop,
} from '../lib/player';
import { ScoreDisplay } from './ScoreDisplay';
import { TouchControls } from './TouchControls';
import { displayName, type RomInfo, type TableEntry } from '../lib/types';

interface Props {
  /** The table from the library, or `null` in debug mode. */
  table: TableEntry | null;
  /** Title to show when there is no library entry. */
  title?: string;
  /** Where to get the `.vpx` from: IndexedDB by id, or a URL. */
  source: { kind: 'library'; id: string } | { kind: 'url'; url: string };
  /** Which ROM the table needs, when there is no library entry to ask. */
  rom?: RomInfo;
  onExit: () => void;
}

type Phase = 'starting' | 'fetching' | 'loading' | 'ready';

const PHASES: Record<Phase, string> = {
  starting: 'Initialising WebGPU…',
  fetching: 'Reading the file…',
  loading: 'Parsing the table and uploading it to the GPU…',
  ready: '',
};

/** The keys the player answers to. Mirrors `vpw_table::controls`. */
/**
 * Game view: mounts the canvas, starts the wasm player and hands it the table.
 *
 * The keyboard is **not** wired up here: the wasm side listens on `window`
 * itself, so a keypress reaches the physics without crossing this component and
 * without waiting for React to re-render. The one key this owns is `Escape`,
 * because leaving is a UI decision and not a table one.
 */
export function Player({ table, title, source, rom, onExit }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>('starting');
  const [loop, setLoop] = useState<Loop | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [marked, setMarked] = useState<string | null>(null);
  const [view, setView] = useState<CameraView>(() => settings().camera);

  const key = source.kind === 'library' ? `db:${source.id}` : `url:${source.url}`;

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;

    void (async () => {
      try {
        await startPlayer('playfield');
        if (!alive) return;

        setPhase('loading');
        const s = await loadTable(key, () =>
          source.kind === 'library'
            ? readTableFile(source.id)
            : fetch(source.url)
                .then((r) => r.arrayBuffer())
                .then((b) => new Uint8Array(b)),
          table?.rom ?? rom,
        );
        if (!alive) return;

        console.info('[player] loaded', s);
        // The recorder is only useful if it was already running when the thing
        // being chased happened, so it goes on with the table and not on
        // demand. See `vpw_game::telemetry` for what it costs.
        await setTelemetry(true);
        // The stored view, applied once the player exists to receive it. The
        // camera is the renderer's and does not survive a reload, so this is
        // what makes the setting mean anything.
        await setCameraView(settings().camera);
        setPhase('ready');

        timer = window.setInterval(() => {
          void loopStats().then((l) => {
            if (!alive || !l) return;
            setLoop(l);
            // Kept until it is replaced rather than for one poll: the machine
            // says why it would not start exactly once, and a notice that
            // flashes for a quarter of a second is a notice nobody read.
            if (l.notice) setNotice(l.notice);
          });
        }, 250);
      } catch (e) {
        // No `if (alive)`: swallowing the error once the effect has already
        // been cancelled leaves the screen stuck on "loading" with no clue
        // about what happened, which is exactly what was so hard to track
        // down the first time.
        console.error('[player] the load failed:', e);
        setError(e instanceof Error ? e.message : String(e));
      }
    })();

    // A machine that forgets its high scores every time is not the machine
    // the player left. `visibilitychange` and not `unload`: on mobile a tab is
    // often killed without ever unloading.
    const save = () => void saveMachineState();
    document.addEventListener('visibilitychange', save);

    return () => {
      alive = false;
      document.removeEventListener('visibilitychange', save);
      save();
      if (timer !== undefined) window.clearInterval(timer);
      void stopAudio();
    };
  }, [key]);

  // The sound cannot start on its own: a browser only lets an `AudioContext`
  // run from inside a real input event. Which event does not matter, and by the
  // time somebody has pressed a flipper they have provided one — so every path
  // into the game is also the path that turns the sound on.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      void startAudio();
      if (e.key === 'Escape') onExit();
      // `B` for the bug you just saw. The table does not use it — neither
      // `Action::from_key_code` nor `vp_key_code` maps it — so pressing it
      // marks the moment without also doing something on the playfield.
      // `C` for the camera. Like `B`, a key the table itself does not use —
      // neither `Action::from_key_code` nor `vp_key_code` maps it — so cycling
      // the view does not also do something on the playfield.
      if (e.code === 'KeyC' && !e.repeat) {
        const next = nextCameraView(settings().camera);
        // Through the setting rather than straight to the player, so the key
        // and the menu cannot disagree and the choice outlives the session.
        updateSettings({ camera: next });
      }
      if (e.code === 'KeyB' && !e.repeat) {
        void mark().then((m) => {
          if (!m) return;
          console.info('[telemetry]', m);
          setMarked(m.savedTo ?? m.name);
          window.setTimeout(() => setMarked(null), 4000);
        });
      }
    };
    const onPoint = () => void startAudio();
    window.addEventListener('keydown', onKey);
    window.addEventListener('pointerdown', onPoint);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('pointerdown', onPoint);
    };
  }, [onExit]);

  // One subscription for both ways in. The key writes the setting and the menu
  // writes the setting, so the camera only has to watch the setting — which
  // also means the two can never drift apart.
  useEffect(
    () =>
      onSettingsChange((s) => {
        setView(s.camera);
        void setCameraView(s.camera);
      }),
    [],
  );

  const name = table ? displayName(table) : (title ?? 'Table');

  return (
    <div className="player">
      <canvas id="playfield" ref={canvasRef} />

      {phase === 'ready' && <ScoreDisplay view={view} />}
      {phase === 'ready' && <TouchControls onNewBall={() => void newBall()} />}

      <div className="player-hud">
        <button className="touch-exit" onClick={onExit} aria-label={`Leave ${name}`}>
          <ExitIcon />
        </button>
        {/* The numbers only appear when they are news: the table tilted, or the
            physics is falling behind and the game is running in slow motion.
            The rest of the time a playfield is better with nothing on it. */}
        {loop && (loop.tilt || loop.tps < 900) && <LoopBadge loop={loop} />}
        {loop && !loop.romRunning && <NoMachine wanted={table?.rom.name ?? rom?.name ?? null} />}
        {notice && <p className="player-notice">{notice}</p>}
        {marked && (
          <span className="player-fps">
            saved {TELEMETRY_WINDOW_S}s · {marked}
          </span>
        )}
      </div>

      {/* Loading and errors still get words, because there is nothing to look
          at yet and "something went wrong" is not an icon. */}
      {(error || phase !== 'ready') && (
        <div className="player-status">
          {error ? (
            <p className="notice notice-error">{error}</p>
          ) : (
            <p className="notice">{PHASES[phase]}</p>
          )}
        </div>
      )}
    </div>
  );
}

/** A door with an arrow out of it. */
function ExitIcon() {
  return (
    <svg viewBox="0 0 24 24" role="presentation" focusable="false">
      <path
        d="M14 4 H6 a2 2 0 0 0 -2 2 v12 a2 2 0 0 0 2 2 h8"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M18 15 L21 12 L18 9 M21 12 H10"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * The live numbers.
 *
 * Physics ticks per second is worth as much as the frame rate here: the physics
 * runs at a fixed 1000 Hz, decoupled from the frame rate, so a number below
 * that means the simulation is falling behind and the table is running in slow
 * motion — which looks like lag but is not.
 */
/**
 * Says that the table is running without its machine.
 *
 * A table with no ROM — or with a ROM for hardware this emulator does not have
 * — loads, renders, and rolls a ball around perfectly. It also takes a coin and
 * starts nothing, because the rules of the game live on a board that is not
 * there. From the player's seat that is indistinguishable from a bug, and there
 * was nothing anywhere on the screen to tell the two apart: somebody put a coin
 * in, pressed start, and watched nothing happen with no way of knowing why.
 */
function NoMachine({ wanted }: { wanted: string | null }) {
  return (
    <div className="player-nomachine" role="status">
      <strong>No machine</strong>
      <span>
        The table is running but its ROM is not, so a coin credits nothing and
        start starts nothing: the rules of the game live on the machine's board.
      </span>
      {wanted ? (
        <span>
          It asks for <code>{wanted}.zip</code>. Import it from Content — or it
          may be a machine this emulator cannot run yet.
        </span>
      ) : (
        <span>This table did not say which ROM it needs.</span>
      )}
    </div>
  );
}

function LoopBadge({ loop }: { loop: Loop }) {
  return (
    <span className="player-fps">
      {loop.fps.toFixed(0)} fps · {loop.tps.toFixed(0)} Hz physics
      {loop.tilt && <strong className="player-tilt"> · TILT</strong>}
    </span>
  );
}

