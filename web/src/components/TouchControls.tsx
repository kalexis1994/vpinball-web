// Playing with your thumbs.
//
// A phone has no keyboard, so the four things a pinball asks of a player —
// left flipper, right flipper, plunger, coin — have to be on the glass. They
// are laid out the way a cabinet is: flippers at the bottom corners where the
// buttons are, the plunger top right where the rod is, the coin slot top left
// where the door is. Nothing here is a new input path; each one presses the
// same key a keyboard would.
//
// Everything is drawn as SVG rather than a font or a bitmap, for two reasons
// that matter on a phone: it stays sharp at any density, and it can be
// semi-transparent over the playfield without a rectangle of background around
// it. A button you can see through is a button that is not covering the table.
//
// # Why `setPointerCapture` and not `onPointerLeave`
//
// A finger that slides off a flipper button while still on the glass must keep
// the flipper up — that is how a real cabinet button behaves, and a player
// rolling their thumb off the edge mid-shot would otherwise drop the ball.
// Capturing the pointer sends every move and the release to the element that
// was first touched, whatever is under the finger by then.

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  holdPlunger,
  plungerPull,
  pressKey,
  releaseAllKeys,
  releasePlunger,
} from '../lib/player';

/** How far the finger drags, in pixels, to draw the rod all the way back.
 *
 * Also how far the thumb slides on screen, so the two agree: a drag of this
 * much is a full pull and a full pull looks like this much. */
const PLUNGER_TRAVEL = 96;

interface Props {
  /** Called when the player asks for a new ball, so the host can react. */
  onNewBall?: () => void;
}

export function TouchControls({ onNewBall }: Props) {
  // Let go of everything if the component goes away mid-press: a flipper held
  // by a finger that no longer exists stays up for ever.
  useEffect(() => () => void releaseAllKeys(), []);

  return (
    <div className="touch" aria-label="Touch controls">
      <CoinSlot />
      <StartButton />
      <Plunger onNewBall={onNewBall} />
      <FlipperButton side="left" code="KeyZ" />
      <FlipperButton side="right" code="KeyM" />
    </div>
  );
}

/**
 * One cabinet flipper button.
 *
 * The same component on both sides — the only differences are which corner it
 * sits in and which key it presses, so there is one of these and it is
 * instantiated twice. It is drawn as a plain arcade pushbutton, because that is
 * what it is: on a real cabinet the button says nothing about the flipper, and
 * which one is which is obvious from which hand reaches it.
 */
function FlipperButton({ side, code }: { side: 'left' | 'right'; code: string }) {
  const [held, setHeld] = useState(false);

  const down = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      e.currentTarget.setPointerCapture(e.pointerId);
      e.preventDefault();
      setHeld(true);
      void pressKey(code, true);
    },
    [code],
  );

  const up = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      e.preventDefault();
      setHeld(false);
      void pressKey(code, false);
    },
    [code],
  );

  return (
    <button
      className={`touch-flipper touch-${side}${held ? ' is-held' : ''}`}
      aria-label={`${side} flipper`}
      onPointerDown={down}
      onPointerUp={up}
      onPointerCancel={up}
      onContextMenu={(e) => e.preventDefault()}
    >
      <ArcadeButtonIcon />
    </button>
  );
}

/**
 * The plunger: pull it back as far as you mean to, and let go.
 *
 * Which is what a plunger *is*. The space bar cannot work that way — a key has
 * no position to give, so held down it draws the rod back on its own — but a
 * finger on a screen has one, and using it is both more faithful and the thing
 * anybody who has stood at a machine expects. The original has the same control
 * for people with a real plunger wired to their cabinet, and this is wired to
 * the same place: see `Plunger::hold_at`.
 *
 * The rod does not jump to the finger. It is pulled towards it by a spring, so
 * a ball resting against the tip gets pushed instead of passed through, and
 * yanking the rod back and slamming it forward is a shot.
 *
 * While a finger is on it the picture is drawn from the **finger** rather than
 * from the rod, and once it is let go, from the rod again. That is not a lie
 * about where the rod is — the spring closes the gap in a few milliseconds —
 * it is the difference between a control that stretches as you drag it and one
 * that appears not to have noticed you. A plunger you cannot see move is a
 * plunger nobody believes they are pulling, and this one is pulled the way the
 * real one is: from the top of its travel, downwards, and let go.
 */
function Plunger({ onNewBall }: { onNewBall?: () => void }) {
  // 0 at rest, 1 fully drawn back. Read from the table every frame.
  const [pulled, setPulled] = useState(0);
  /** Where the finger has it, or `null` when no finger is on it. */
  const [held, setHeld] = useState<number | null>(null);

  useEffect(() => {
    let frame = 0;
    let last = -1;
    const poll = () => {
      const p = plungerPull();
      if (p !== null) {
        // Rounded before comparing: the plunger settles with a tail of
        // movements far too small to see, and re-rendering for those is sixty
        // wasted renders a second for a picture nobody can tell apart.
        const q = Math.round(p * 200) / 200;
        if (q !== last) {
          last = q;
          setPulled(q);
        }
      }
      frame = requestAnimationFrame(poll);
    };
    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, []);

  const origin = useRef(0);
  /** The same thing as `held`, for the handlers, which see it as it is now
   *  rather than as it was when they were made. */
  const holding = useRef(false);

  const down = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    // Nice to have and not to be relied on: it keeps the drag alive when the
    // finger leaves the widget, and it throws on some pointers. Whether the
    // plunger is being held is tracked here, not asked of the browser —
    // asking is what made a drag that started slightly off the knob do
    // nothing at all.
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* the drag still works, it just ends at the edge */
    }
    e.preventDefault();
    origin.current = e.clientY;
    holding.current = true;
    setHeld(0);
    holdPlunger(0);
  }, []);

  const move = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!holding.current) return;
    // Downwards is backwards: the shooter is at the bottom right of the glass
    // and pulling it is pulling it towards you.
    const dragged = Math.max(0, Math.min(1, (e.clientY - origin.current) / PLUNGER_TRAVEL));
    setHeld(dragged);
    holdPlunger(dragged);
  }, []);

  const up = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    holding.current = false;
    setHeld(null);
    releasePlunger();
  }, []);

  return (
    <div
      className="touch-plunger"
      aria-label="Plunger: pull down and release"
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onContextMenu={(e) => e.preventDefault()}
      // A double tap serves a fresh ball, which is what you want after a drain
      // and what the keyboard spells `Enter`.
      onDoubleClick={() => onNewBall?.()}
    >
      <PlungerIcon pulled={held ?? pulled} />
      <span
        className="touch-thumb"
        style={{ transform: `translateY(${((held ?? pulled) * PLUNGER_TRAVEL).toFixed(1)}px)` }}
        aria-hidden="true"
      >
        <ThumbIcon />
      </span>
    </div>
  );
}

/**
 * The coin slot.
 *
 * A press is a coin: it closes the chute switch for a moment, exactly as
 * dropping one does. The machine takes a beat to credit it — the script waits
 * three quarters of a second before pulsing the switch, because a real coin
 * takes that long to fall — so the button flashes to say the press landed
 * rather than leaving the player wondering.
 */
function CoinSlot() {
  const [inserted, setInserted] = useState(0);

  const insert = useCallback(() => {
    void (async () => {
      await pressKey('Digit5', true);
      window.setTimeout(() => void pressKey('Digit5', false), 60);
    })();
    setInserted((n) => n + 1);
  }, []);

  // The flash is keyed on the count so that a second coin restarts it rather
  // than being swallowed by the animation already running.
  return (
    <button
      key={inserted}
      className={`touch-coin${inserted > 0 ? ' is-inserted' : ''}`}
      aria-label="Insert coin"
      onPointerDown={(e) => {
        e.preventDefault();
        insert();
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <CoinIcon />
    </button>
  );
}

/**
 * The start button.
 *
 * Under the coin, because that is the order you use them in and because on a
 * cabinet they are both on the front. Green and a play triangle, which beside a
 * coin slot is unambiguous without a word on it.
 */
function StartButton() {
  const [held, setHeld] = useState(false);

  const press = useCallback((down: boolean) => {
    setHeld(down);
    void pressKey('Digit1', down);
  }, []);

  return (
    <button
      className={`touch-start${held ? ' is-held' : ''}`}
      aria-label="Start"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        e.preventDefault();
        press(true);
      }}
      onPointerUp={(e) => {
        e.preventDefault();
        press(false);
      }}
      onPointerCancel={() => press(false)}
      onContextMenu={(e) => e.preventDefault()}
    >
      <StartIcon />
    </button>
  );
}

// --- The icons ---------------------------------------------------------------
//
// Drawn rather than imported: four shapes is less code than a sprite sheet and
// none of it has to be fetched.

/**
 * An arcade pushbutton, seen from above.
 *
 * Nothing but the button: the bezel nut that holds it in the panel, the plunger
 * housing under it, the cap, and the highlight that makes a domed cap read as
 * domed. What it does is written on the cabinet, not on the button — and a
 * flipper bat drawn inside one would be a diagram of the mechanism rather than
 * a control.
 */
function ArcadeButtonIcon() {
  return (
    <svg viewBox="0 0 100 100" role="presentation" focusable="false">
      {/* The bezel nut, and the shadow it casts into the panel. */}
      <circle cx="50" cy="50" r="46" className="touch-bezel" />
      <circle cx="50" cy="50" r="39" className="touch-housing" />
      {/* The cap. This is the part that moves. */}
      <circle cx="50" cy="50" r="32" className="touch-cap" />
      {/* A crescent of light along the top left, which is what says "domed"
          rather than "flat disc". */}
      <path
        className="touch-shine"
        d="M28 42 A25 25 0 0 1 66 29 A31 31 0 0 0 28 42 Z"
      />
    </svg>
  );
}

/** The shooter rod: a shaft, a spring, and the ball waiting in the lane. */
function PlungerIcon({ pulled }: { pulled: number }) {
  // The spring compresses as the rod comes back. Five coils is enough to read
  // as a spring at this size and few enough to stay legible when squashed.
  const coils = 5;
  const top = 18 + pulled * 34;
  const bottom = 74;
  const step = (bottom - top) / coils;
  const zigzag = Array.from({ length: coils * 2 + 1 }, (_, i) => {
    const y = top + (i * step) / 2;
    const x = i % 2 === 0 ? 22 : 38;
    return `${i === 0 ? 'M' : 'L'}${x} ${y.toFixed(1)}`;
  }).join(' ');

  return (
    <svg viewBox="0 0 60 110" role="presentation" focusable="false">
      {/* The lane the rod runs in. */}
      <rect x="16" y="6" width="28" height="98" rx="14" className="touch-lane" />
      {/* The rod, which shortens as it is pulled. */}
      <line x1="30" y1="6" x2="30" y2={top} className="touch-rod" />
      <path d={zigzag} className="touch-spring" />
      <circle cx="30" cy={bottom + 14} r="9" className="touch-ball" />
    </svg>
  );
}

/** The grip the finger sits on. A knurled knob, seen from the front. */
function ThumbIcon() {
  return (
    <svg viewBox="0 0 44 44" role="presentation" focusable="false">
      <circle cx="22" cy="22" r="20" className="touch-knob" />
      <circle cx="22" cy="22" r="13" className="touch-knob-inner" />
      {/* The knurling: short strokes around the rim. */}
      <g className="touch-knurl">
        {Array.from({ length: 12 }, (_, i) => {
          const a = (i * Math.PI) / 6;
          const [sx, sy] = [22 + Math.cos(a) * 14, 22 + Math.sin(a) * 14];
          const [ex, ey] = [22 + Math.cos(a) * 19, 22 + Math.sin(a) * 19];
          return <line key={i} x1={sx} y1={sy} x2={ex} y2={ey} />;
        })}
      </g>
      {/* An arrow down, because "pull" is not obvious from a knob. */}
      <path d="M22 15 L22 27 M17 23 L22 28 L27 23" className="touch-pull-hint" />
    </svg>
  );
}

/** A lit start button. */
function StartIcon() {
  return (
    <svg viewBox="0 0 100 100" role="presentation" focusable="false">
      <circle cx="50" cy="50" r="46" className="touch-bezel" />
      <circle cx="50" cy="50" r="39" className="touch-start-cap" />
      {/* A play triangle. Beside a coin slot it needs no word on it, and the
          word is the only text there was left on the playfield. */}
      <path className="touch-start-mark" d="M41 34 L68 50 L41 66 Z" />
    </svg>
  );
}

/** An arcade token going into a slot. */
function CoinIcon() {
  return (
    <svg viewBox="0 0 100 100" role="presentation" focusable="false">
      {/* The slot in the door, at an angle, as they always are. */}
      <rect
        x="60"
        y="18"
        width="10"
        height="34"
        rx="5"
        className="touch-slot"
        transform="rotate(20 65 35)"
      />
      {/* The token: a milled edge, and a star where the denomination goes. */}
      <g className="touch-token">
        <circle cx="42" cy="56" r="30" className="touch-token-edge" />
        <circle cx="42" cy="56" r="23" className="touch-token-face" />
        <g className="touch-milling">
          {Array.from({ length: 16 }, (_, i) => {
            const a = (i * Math.PI) / 8;
            const [sx, sy] = [42 + Math.cos(a) * 24, 56 + Math.sin(a) * 24];
            const [ex, ey] = [42 + Math.cos(a) * 29, 56 + Math.sin(a) * 29];
            return <line key={i} x1={sx} y1={sy} x2={ex} y2={ey} />;
          })}
        </g>
        <path
          className="touch-token-star"
          d="M42 42 L46.4 51.4 L56.6 52.6 L49 59.6 L51 69.6 L42 64.6 L33 69.6 L35 59.6 L27.4 52.6 L37.6 51.4 Z"
        />
      </g>
    </svg>
  );
}
