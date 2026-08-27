// How the player wants it to behave.
//
// Two tabs: Sound and Graphics. The volume is the one that matters on a
// phone — a table's mix runs close to full scale and the first thing anyone
// does with a new one is reach for the volume. Everything about the picture —
// the renderer, the resolution governor, the room, the camera — lives
// together under Graphics, because those are the knobs a player turns when
// the machine in front of them is slow or looks wrong.
//
// Every control applies as it changes rather than on save. That is the
// difference between finding a level and guessing at one — and it works while
// a table is playing behind this screen, because the player subscribes to the
// settings instead of reading them once.

import { useEffect, useState } from 'react';
import {
  CAMERA_VIEWS,
  ENVIRONMENTS,
  settings,
  updateSettings,
  type CameraView,
  type Environment,
} from '../lib/settings';
import { ScreenHead } from './ScreenHead';

interface Props {
  onBack: () => void;
}

type Tab = 'sound' | 'graphics';

const TABS: readonly { id: Tab; label: string }[] = [
  { id: 'sound', label: 'Sound' },
  { id: 'graphics', label: 'Graphics' },
];

export function Settings({ onBack }: Props) {
  const [volume, setVolume] = useState(() => settings().volume);
  const [camera, setCamera] = useState<CameraView>(() => settings().camera);
  const [room, setRoom] = useState<Environment>(() => settings().environment);
  const [flat, setFlat] = useState(() => settings().flat);
  const [adaptive, setAdaptive] = useState(() => settings().adaptive);
  const [tab, setTab] = useState<Tab>('sound');

  // Keep the screen honest if something else changed them — the camera has a
  // key of its own, and it can have been pressed while this screen was open
  // over a table that is still playing.
  useEffect(() => {
    setVolume(settings().volume);
    setCamera(settings().camera);
    setRoom(settings().environment);
    setFlat(settings().flat);
    setAdaptive(settings().adaptive);
  }, []);

  return (
    <main className="shell">
      <ScreenHead title="Settings" onBack={onBack} />

      <nav className="tabs" role="tablist" aria-label="Setting groups">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            className={`tab${tab === t.id ? ' tab-on' : ''}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {tab === 'sound' && (
        <section className="section tab-pane">
          <label className="setting" htmlFor="volume">
            <span className="setting-label">
              Master volume
              <span className="setting-value mono">{Math.round(volume * 100)}%</span>
            </span>
            <span className="setting-control">
              <MuteIcon quiet />
              <input
                id="volume"
                className="slider"
                type="range"
                min={0}
                max={1}
                step={0.01}
                value={volume}
                onChange={(e) => {
                  const next = Number(e.target.value);
                  setVolume(next);
                  updateSettings({ volume: next });
                }}
              />
              <MuteIcon />
            </span>
            <span className="setting-hint">
              Applies straight away, and is remembered for next time.
            </span>
          </label>
        </section>
      )}

      {tab === 'graphics' && (
        <div className="tab-pane">
          <section className="section">
            <div className="setting">
              <span className="setting-label">Renderer</span>
              <span className="setting-control setting-choice">
                <button
                  type="button"
                  className={`choice${flat ? '' : ' choice-on'}`}
                  aria-pressed={!flat}
                  onClick={() => {
                    setFlat(false);
                    updateSettings({ flat: false });
                  }}
                >
                  Full 3D
                </button>
                <button
                  type="button"
                  className={`choice${flat ? ' choice-on' : ''}`}
                  aria-pressed={flat}
                  onClick={() => {
                    setFlat(true);
                    updateSettings({ flat: true });
                  }}
                >
                  Flat (2D)
                </button>
              </span>
              <span className="setting-hint">
                {flat
                  ? 'The table is photographed once and played as pictures; only the ball and the moving pieces stay 3D. Made for weak GPUs — and the camera is fixed while it is on.'
                  : 'The full renderer: real lighting, reflections and a camera you can move.'}
              </span>
            </div>
          </section>

          <section className="section">
            <div className="setting">
              <span className="setting-label">Adaptive resolution</span>
              <span className="setting-control setting-choice">
                <button
                  type="button"
                  className={`choice${adaptive ? ' choice-on' : ''}`}
                  aria-pressed={adaptive}
                  onClick={() => {
                    setAdaptive(true);
                    updateSettings({ adaptive: true });
                  }}
                >
                  On
                </button>
                <button
                  type="button"
                  className={`choice${adaptive ? '' : ' choice-on'}`}
                  aria-pressed={!adaptive}
                  onClick={() => {
                    setAdaptive(false);
                    updateSettings({ adaptive: false });
                  }}
                >
                  Off
                </button>
              </span>
              <span className="setting-hint">
                {adaptive
                  ? 'Trades pixels for frames: the picture softens when the GPU falls behind and sharpens again when there is room.'
                  : 'The picture stays at full resolution whatever the frame rate does.'}
              </span>
            </div>
          </section>

          <section className="section">
            <div className="setting">
              <span className="setting-label">Room</span>
              <span className="setting-control setting-choice">
                {ENVIRONMENTS.map((e) => (
                  <button
                    key={e}
                    type="button"
                    className={`choice${e === room ? ' choice-on' : ''}`}
                    aria-pressed={e === room}
                    onClick={() => {
                      setRoom(e);
                      updateSettings({ environment: e });
                    }}
                  >
                    {ROOM_LABELS[e]}
                  </button>
                ))}
              </span>
              <span className="setting-hint">{ROOM_HINTS[room]}</span>
            </div>
          </section>

          <section className="section">
            <div className="setting">
              <span className="setting-label">Camera</span>
              <span className="setting-control setting-choice">
                {CAMERA_VIEWS.map((v) => (
                  <button
                    key={v}
                    type="button"
                    className={`choice${v === camera ? ' choice-on' : ''}`}
                    aria-pressed={v === camera}
                    disabled={flat}
                    onClick={() => {
                      setCamera(v);
                      updateSettings({ camera: v });
                    }}
                  >
                    {CAMERA_LABELS[v]}
                  </button>
                ))}
              </span>
              <span className="setting-hint">
                {flat
                  ? 'The flat renderer holds the camera: a photograph is taken from one place.'
                  : `${CAMERA_HINTS[camera]} Press C while playing to switch.`}
              </span>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}

const ROOM_LABELS: Record<Environment, string> = {
  table: "Table's own",
  bar: 'A bar',
};

const ROOM_HINTS: Record<Environment, string> = {
  table: 'The environment the table was authored under.',
  bar: 'A real bar, in HDR: its lamps and windows show up reflected in the ball and the plastics.',
};

const CAMERA_LABELS: Record<CameraView, string> = {
  front: 'In front',
  overhead: 'Overhead',
};

const CAMERA_HINTS: Record<CameraView, string> = {
  front: 'The whole machine, backbox and all, the way it looks on the floor.',
  overhead: 'Straight down on the playfield: nothing foreshortened, nothing hidden behind a ramp.',
};

/** A speaker, with or without the waves. The pair marks the ends of the slider. */
function MuteIcon({ quiet }: { quiet?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" width="18" height="18" aria-hidden="true">
      <path
        d="M4 9h3l4-3.5v13L7 15H4z"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      {quiet ? (
        <path d="M15 9.5l4 5M19 9.5l-4 5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      ) : (
        <>
          <path d="M14.5 9.5a3.5 3.5 0 0 1 0 5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          <path d="M17 7a7 7 0 0 1 0 10" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </>
      )}
    </svg>
  );
}
