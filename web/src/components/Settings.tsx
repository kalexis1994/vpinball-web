// How the player wants it to behave.
//
// Four tabs, because this is a settings screen and not a list: somebody
// looking for the flipper keys should not scroll past the room's lighting to
// find them. Controls, Graphics, Score panel, Audio — and each tab is short
// enough to read without scrolling on a phone, which is the test that says
// whether a group has grown too big.
//
// Every control applies as it changes rather than on save. That is the
// difference between finding a level and guessing at one — and it works while
// a table is playing, because the player subscribes to the settings instead
// of reading them once.

import { useEffect, useState } from 'react';
import {
  CAMERA_VIEWS,
  ENVIRONMENTS,
  MAX_MIX_GAIN,
  SCORE_DOCKS,
  SCORE_SIDES,
  settings,
  updateSettings,
  type CameraView,
  type Environment,
  type ScoreDock,
  type ScoreSide,
} from '../lib/settings';
import {
  ACTIONS,
  DEFAULT_KEYS,
  DEFAULT_PAD,
  type ActionId,
  type KeyMap,
  type PadMap,
} from '../lib/controls';
import { Bindings } from './Bindings';
import { ScreenHead } from './ScreenHead';

interface Props {
  onBack: () => void;
}

type Tab = 'controls' | 'graphics' | 'score' | 'audio';

const TABS: readonly { id: Tab; label: string }[] = [
  { id: 'controls', label: 'Controls' },
  { id: 'graphics', label: 'Graphics' },
  { id: 'score', label: 'Score panel' },
  { id: 'audio', label: 'Audio' },
];

export function Settings({ onBack }: Props) {
  const [volume, setVolume] = useState(() => settings().volume);
  const [machineVol, setMachineVol] = useState(() => settings().volumeMachine);
  const [tableVol, setTableVol] = useState(() => settings().volumeTable);
  const [camera, setCamera] = useState<CameraView>(() => settings().camera);
  const [room, setRoom] = useState<Environment>(() => settings().environment);
  const [flat, setFlat] = useState(() => settings().flat);
  const [adaptive, setAdaptive] = useState(() => settings().adaptive);
  const [scoreSide, setScoreSide] = useState<ScoreSide>(() => settings().scoreSide);
  const [scoreDock, setScoreDock] = useState<ScoreDock>(() => settings().scoreDock);
  const [keys, setKeys] = useState<KeyMap>(() => settings().keys);
  const [pad, setPad] = useState<PadMap>(() => settings().pad);
  const [tab, setTab] = useState<Tab>('controls');

  // Keep the screen honest if something else changed them — the camera has a
  // key of its own, and it can have been pressed while this screen was open
  // over a table that is still playing.
  useEffect(() => {
    const s = settings();
    setVolume(s.volume);
    setMachineVol(s.volumeMachine);
    setTableVol(s.volumeTable);
    setCamera(s.camera);
    setRoom(s.environment);
    setFlat(s.flat);
    setAdaptive(s.adaptive);
    setScoreSide(s.scoreSide);
    setScoreDock(s.scoreDock);
    setKeys(s.keys);
    setPad(s.pad);
  }, []);

  /**
   * Gives an action a key, and takes it from whoever had it.
   *
   * Two actions on one key is a table that does two things at once, so the
   * older claim is dropped rather than the new one refused: somebody pressing
   * a key that is already in use has said which action they want it for.
   */
  const bindKey = (id: ActionId, value: string | number) => {
    const code = String(value);
    const next: KeyMap = { ...keys };
    for (const a of ACTIONS) {
      if (a.id !== id && next[a.id] === code) next[a.id] = '';
    }
    next[id] = code;
    setKeys(next);
    updateSettings({ keys: next });
  };

  const bindPad = (id: ActionId, value: string | number) => {
    const button = Number(value);
    const next: PadMap = { ...pad };
    for (const a of ACTIONS) {
      if (a.id !== id && next[a.id] === button) next[a.id] = null;
    }
    next[id] = button;
    setPad(next);
    updateSettings({ pad: next });
  };

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

      {tab === 'controls' && (
        <div className="tab-pane">
          <section className="section">
            <h2 className="section-head">Keyboard</h2>
            <Bindings
              kind="keyboard"
              keys={keys}
              onChange={bindKey}
              onReset={() => {
                setKeys({ ...DEFAULT_KEYS });
                updateSettings({ keys: { ...DEFAULT_KEYS } });
              }}
            />
          </section>

          <section className="section">
            <h2 className="section-head">Gamepad</h2>
            <Bindings
              kind="gamepad"
              pad={pad}
              onChange={bindPad}
              onReset={() => {
                setPad({ ...DEFAULT_PAD });
                updateSettings({ pad: { ...DEFAULT_PAD } });
              }}
            />
          </section>

          <p className="footnote">
            The two are kept apart on purpose: they are different instruments,
            and moving the left flipper on the pad says nothing about where it
            is on the keyboard. On a touch screen the controls are drawn on the
            table itself and need no binding.
          </p>
        </div>
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

      {tab === 'score' && (
        <div className="tab-pane">
          <section className="section">
            <h2 className="section-head">On a large screen</h2>
            <div className="setting">
              <span className="setting-label">Which side</span>
              <span className="setting-control setting-choice">
                {SCORE_SIDES.map((v) => (
                  <button
                    key={v}
                    type="button"
                    className={`choice${v === scoreSide ? ' choice-on' : ''}`}
                    aria-pressed={v === scoreSide}
                    onClick={() => {
                      setScoreSide(v);
                      updateSettings({ scoreSide: v });
                    }}
                  >
                    {SIDE_LABELS[v]}
                  </button>
                ))}
              </span>
              <span className="setting-hint">
                The overhead view leaves a gutter either side of the table.
                The panel floats in the one chosen here, as large as that
                gutter allows.
              </span>
            </div>
          </section>

          <section className="section">
            <h2 className="section-head">On a small screen</h2>
            <div className="setting">
              <span className="setting-label">Where it goes</span>
              <span className="setting-control setting-choice">
                {SCORE_DOCKS.map((v) => (
                  <button
                    key={v}
                    type="button"
                    className={`choice${v === scoreDock ? ' choice-on' : ''}`}
                    aria-pressed={v === scoreDock}
                    onClick={() => {
                      setScoreDock(v);
                      updateSettings({ scoreDock: v });
                    }}
                  >
                    {DOCK_LABELS[v]}
                  </button>
                ))}
              </span>
              <span className="setting-hint">{DOCK_HINTS[scoreDock]}</span>
            </div>
          </section>

          <p className="footnote">
            There is no gutter on a phone, so the panel docks instead and the
            table gives up that strip — it shrinks to fit what is left, keeping
            its shape. In the front view none of this applies: the machine's
            head is in the picture with the score already on it.
          </p>
        </div>
      )}

      {tab === 'audio' && (
        <div className="tab-pane">
          <section className="section">
            <Fader
              id="volume"
              label="Master volume"
              value={volume}
              hint="Everything at once. Applies straight away, and is remembered for next time."
              onChange={(v) => {
                setVolume(v);
                updateSettings({ volume: v });
              }}
            />
          </section>

          <section className="section">
            <h2 className="section-head">The balance</h2>
            <Fader
              id="volume-machine"
              label="Machine"
              value={machineVol}
              max={MAX_MIX_GAIN}
              hint="The ROM's own sound board: the music, the speech and the game's effects — everything the machine says. Plenty of ROMs were mastered quietly, so this one goes past 100%."
              onChange={(v) => {
                setMachineVol(v);
                updateSettings({ volumeMachine: v });
              }}
            />
            <Fader
              id="volume-table"
              label="Table"
              value={tableVol}
              max={MAX_MIX_GAIN}
              hint="The mechanics: bumpers, flippers, slingshots and the ball on the wood — everything the table does when it is hit."
              onChange={(v) => {
                setTableVol(v);
                updateSettings({ volumeTable: v });
              }}
            />
            <p className="footnote">
              Both sit under the master, so this is a balance rather than a
              second volume: turning one down leaves the other exactly where it
              was. Past 100% is a boost — useful against a quietly mastered
              ROM, and loud enough at the top that the loudest moments may
              flatten rather than get louder.
            </p>
          </section>
        </div>
      )}
    </main>
  );
}

const SIDE_LABELS: Record<ScoreSide, string> = {
  left: 'Left',
  right: 'Right',
};

const DOCK_LABELS: Record<ScoreDock, string> = {
  top: 'Above',
  bottom: 'Below',
  hidden: 'Hidden',
};

const DOCK_HINTS: Record<ScoreDock, string> = {
  top: 'A strip above the playfield, where a machine keeps its head. The table shrinks to fit what is left, keeping its shape.',
  bottom: 'A strip below the playfield, under the flippers. The table shrinks to fit what is left, keeping its shape.',
  hidden: 'No panel: the whole screen is the glass over the playfield, and the score is only on the machine.',
};

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

/** One fader: its name, what it is at, and the two ends of its travel. */
function Fader({
  id,
  label,
  value,
  hint,
  max = 1,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  hint: string;
  /** Above one the fader boosts; see `MAX_MIX_GAIN`. */
  max?: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="setting" htmlFor={id}>
      <span className="setting-label">
        {label}
        <span className={`setting-value mono${value > 1 ? ' setting-boosted' : ''}`}>
          {Math.round(value * 100)}%
        </span>
      </span>
      <span className="setting-control">
        <MuteIcon quiet />
        <input
          id={id}
          className="slider"
          type="range"
          min={0}
          max={max}
          step={0.01}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        <MuteIcon />
      </span>
      <span className="setting-hint">{hint}</span>
    </label>
  );
}

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
