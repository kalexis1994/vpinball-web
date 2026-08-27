import { useEffect, useRef, useState } from 'react';
import { startAudio, stopAudio } from '../lib/audio';
import { readTableFile } from '../lib/library';
import { onSettingsChange, settings, updateSettings, type CameraView } from '../lib/settings';
import {
  TELEMETRY_WINDOW_S,
  connectInput,
  hidePlayer,
  loadTable,
  loopStats,
  mark,
  newBall,
  nextCameraView,
  playerCanvas,
  saveMachineState,
  setCameraView,
  setEnvironment,
  setAdaptive,
  setFlat,
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

/**
 * The curtain's states. `loading` shows the marquee and the chase lamps over
 * black; when the table is ready the lamps go out first (`blackout` — a fade
 * to plain black, the moment a real machine's attract mode goes dark), then
 * the black itself lifts off the table (`reveal`), then the curtain leaves
 * the DOM (`done`).
 */
type Intro = 'loading' | 'blackout' | 'reveal' | 'done';

/**
 * Game view: mounts the canvas, starts the player and hands it the table.
 *
 * The table's own keys are wired by `connectInput` (`lib/input.ts`), outside
 * React, so a keypress reaches the physics without waiting for a re-render.
 * The keys this component owns are the UI's: `Escape` because leaving is a UI
 * decision and not a table one, `C` for the camera and `B` for the telemetry
 * mark.
 */
export function Player({ table, title, source, rom, onExit }: Props) {
  const stageRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>('starting');
  const [loop, setLoop] = useState<Loop | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [marked, setMarked] = useState<string | null>(null);
  // A new build, waiting for permission to take over. See `watchForUpdates`
  // in `main.tsx` for why this is asked rather than done.
  const [update, setUpdate] = useState<(() => void) | null>(null);
  const [view, setView] = useState<CameraView>(() => settings().camera);
  const [intro, setIntro] = useState<Intro>('loading');

  // The curtain follows the phase: the moment the table is ready the lamps
  // fade, the black holds a beat, and then it lifts. A new load (a different
  // table into the same view) drops the curtain again.
  useEffect(() => {
    if (phase !== 'ready') {
      setIntro('loading');
      return;
    }
    setIntro('blackout');
    const lift = window.setTimeout(() => setIntro('reveal'), 600);
    const gone = window.setTimeout(() => setIntro('done'), 600 + 1000);
    return () => {
      window.clearTimeout(lift);
      window.clearTimeout(gone);
    };
  }, [phase]);

  useEffect(() => {
    const offered = (e: Event) => setUpdate(() => (e as CustomEvent<() => void>).detail);
    window.addEventListener('vpw-update', offered);
    return () => window.removeEventListener('vpw-update', offered);
  }, []);

  const key = source.kind === 'library' ? `db:${source.id}` : `url:${source.url}`;

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;
    let disconnectInput: (() => void) | undefined;

    // The canvas is the player's own element, appended rather than rendered:
    // a WebGL context and a transferred `OffscreenCanvas` are both married to
    // one element for life, so every visit must present the same one. See
    // `playerCanvas`.
    const canvas = playerCanvas();
    stageRef.current?.appendChild(canvas);

    void (async () => {
      try {
        await startPlayer(canvas);
        if (!alive) return;

        // The listeners live on the page for both of the player's homes — a
        // worker has no DOM to listen on — and die with this view.
        disconnectInput = connectInput(canvas);

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

        console.debug('[player] loaded', s);
        // The recorder is only useful if it was already running when the thing
        // being chased happened, so it goes on with the table and not on
        // demand. See `vpw_game::telemetry` for what it costs.
        await setTelemetry(true);
        // The stored view, applied once the player exists to receive it. The
        // camera is the renderer's and does not survive a reload, so this is
        // what makes the setting mean anything.
        await setCameraView(settings().camera);
        // The stored room: the bar's map is a fetch away, so it goes on
        // after the table is playable rather than making it wait.
        void setEnvironment(settings().environment);
        // The stored renderer choice; `?flat=1` forces it on from the
        // address bar, which is how the debug route exercises it.
        const forced = new URLSearchParams(window.location.search).get('flat') === '1';
        void setFlat(forced || settings().flat);
        void setAdaptive(settings().adaptive);
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
        const message = e instanceof Error ? e.message : String(e);
        // A refused graphics context has a cause the player can act on and
        // the raw message does not name: browsers cap live WebGL contexts
        // per process, and a session's worth of crashed or forgotten tabs
        // can spend the whole allowance.
        setError(
          message.includes('create the surface')
            ? `${message} — the browser refused a graphics context. Closing other tabs of this page, or restarting the browser, usually gives it back.`
            : message,
        );
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
      disconnectInput?.();
      // A worker cannot see this canvas unmount; it has to be told, or the
      // ball drains while somebody reads the table list.
      hidePlayer();
      void stopAudio();
      // Detached, not destroyed: the element goes back in on the next visit.
      canvas.remove();
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
        if (settings().flat) {
          // The flat renderer holds the camera, and a key that silently does
          // nothing reads as a bug. Say why, where the tilt notice lives.
          setNotice('The flat renderer holds the camera — switch to Full 3D in Settings to move it.');
          return;
        }
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
        // The room switch works while a table is playing behind the
        // settings screen, the same way the volume does.
        void setEnvironment(s.environment);
        // And so does the renderer: the flat engine bakes over a couple of
        // seconds of play, then takes the frame; off is immediate.
        void setFlat(s.flat);
        void setAdaptive(s.adaptive);
      }),
    [],
  );

  const name = table ? displayName(table) : (title ?? 'Table');

  return (
    <div className="player">
      {/* The canvas lands here, appended by the effect above; the div itself
          generates no box, so the canvas lays out as a direct child. */}
      <div className="player-stage" ref={stageRef} />

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
        {/* Worth the space it takes: without it a player stays on an old build
            for as long as they keep reloading instead of closing the tab, and
            every fix lands nowhere. */}
        {update && (
          <button
            type="button"
            className="player-update"
            onClick={() => {
              setUpdate(null);
              update();
            }}
          >
            New version ready · tap to load it
          </button>
        )}
        {marked && (
          <span className="player-fps">
            saved {TELEMETRY_WINDOW_S}s · {marked}
          </span>
        )}
      </div>

      {/* The curtain: the table's name up in lights while the machine gets
          ready, a fade to black, and the black lifting off the playfield. */}
      {intro !== 'done' && !error && (
        <div className={`intro intro-${intro}`} aria-hidden={intro !== 'loading'}>
          <div className="intro-body">
            <span className="intro-kicker">Now loading</span>
            <h2 className="intro-title">{name}</h2>
            <span className="intro-lamps" aria-hidden="true">
              <i /><i /><i /><i /><i />
            </span>
            <p className="intro-phase" aria-live="polite">
              {PHASES[phase]}
            </p>
          </div>
        </div>
      )}

      {/* Errors still get words, because there is nothing to look at and
          "something went wrong" is not an icon. */}
      {error && (
        <div className="player-status">
          <p className="notice notice-error">{error}</p>
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
      {/* Why a machine is quiet, which is otherwise invisible: no board at all
          is a missing image in the zip, a board at zero is firmware that has
          not got going, and a board at its full rate with nothing coming out
          of the speakers is a machine in attract mode with nothing to say. */}
      {loop.romRunning && !loop.soundBoard && (
        <strong className="player-quiet"> · no sound board</strong>
      )}
      {loop.romRunning && loop.soundBoard && (
        <span> · {(loop.soundRate / 1000).toFixed(1)}k sound</span>
      )}
      {loop.tilt && <strong className="player-tilt"> · TILT</strong>}
    </span>
  );
}

