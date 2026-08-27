// How the player wants it to behave.
//
// Two settings so far. The volume is the one that matters on a phone: a table's
// mix runs close to full scale and the first thing anyone does with a new one is
// reach for the volume. The camera is the one that matters on any screen that
// is not shaped like a pinball machine.
//
// The slider applies as it moves rather than on release. That is the difference
// between finding a level and guessing at one — and it works while a table is
// playing behind this screen, because the audio subscribes to the setting
// instead of reading it once.

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

export function Settings({ onBack }: Props) {
  const [volume, setVolume] = useState(() => settings().volume);
  const [camera, setCamera] = useState<CameraView>(() => settings().camera);
  const [room, setRoom] = useState<Environment>(() => settings().environment);

  // Keep the screen honest if something else changed them — the camera has a
  // key of its own, and it can have been pressed while this screen was open
  // over a table that is still playing.
  useEffect(() => {
    setVolume(settings().volume);
    setCamera(settings().camera);
    setRoom(settings().environment);
  }, []);

  return (
    <main className="shell">
      <ScreenHead title="Settings" onBack={onBack} />

      <section className="section">
        <h2 className="section-head">Sound</h2>

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

      <section className="section">
        <h2 className="section-head">Lighting</h2>

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
        <h2 className="section-head">View</h2>

        <div className="setting">
          <span className="setting-label">Camera</span>
          <span className="setting-control setting-choice">
            {CAMERA_VIEWS.map((v) => (
              <button
                key={v}
                type="button"
                className={`choice${v === camera ? ' choice-on' : ''}`}
                aria-pressed={v === camera}
                onClick={() => {
                  setCamera(v);
                  updateSettings({ camera: v });
                }}
              >
                {CAMERA_LABELS[v]}
              </button>
            ))}
          </span>
          <span className="setting-hint">{CAMERA_HINTS[camera]} Press C while playing to switch.</span>
        </div>
      </section>
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
