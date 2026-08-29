// The machine's score display, when the machine's head is not in shot.
//
// It is the **same drawing** that goes onto the head in the 3D scene: one
// module in Rust knows how a segment digit is shaped, and both destinations ask
// it for an image. This one puts that image on a canvas; the other puts it on a
// texture. Neither knows anything about digits, so the two cannot drift apart.
//
// Which is also why this is not drawn in the front view. There the head is
// right there in the picture with the display on it, and a second copy floating
// over the machine would be the same score twice.
//
// Where it goes depends on what the screen leaves over. On a wide one the
// overhead view puts gutters either side of the table and the panel floats in
// the one the player chose. On a phone there is no gutter at all, so it
// *docks*: a strip above or below the playfield, which costs the table some
// height — the camera reframes into what is left and the table keeps its
// shape, smaller. A player who would rather have the pixels can turn it off.

import { useEffect, useRef, useState } from 'react';
import { backboxRect, displayImage, playfieldRect } from '../lib/player';
import type { CameraView, ScoreDock, ScoreSide } from '../lib/settings';

/** Below this the overhead view is the whole screen and nothing else fits. */
export const NARROW_PX = 720;

/** How much of the gutter a floating panel needs before it is worth using. */
const GUTTER_PX = 220;

/** Whether the window is too narrow for a panel beside the table. */
export function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.innerWidth < NARROW_PX);
  useEffect(() => {
    const onResize = () => setNarrow(window.innerWidth < NARROW_PX);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return narrow;
}

/**
 * Where the panel docks, or `null` for "it does not".
 *
 * The player asks this too, and has to: a docked panel is a strip of the
 * window the table no longer has, which is a layout question and not a
 * decoration one.
 */
export function dockedAt(
  view: CameraView,
  narrow: boolean,
  dock: ScoreDock,
): 'top' | 'bottom' | null {
  if (view !== 'overhead' || !narrow || dock === 'hidden') return null;
  return dock;
}

/** How big the canvas is. Wide and short, which is the shape of two rows of
 *  sixteen digits, and small enough that handing the pixels over costs little. */
const SIZE = { width: 512, height: 128 };

interface Props {
  view: CameraView;
  /** Which gutter to float in, when there is one. */
  side: ScoreSide;
  /** The strip the player has already made room for, if any. */
  docked: 'top' | 'bottom' | null;
}

export function ScoreDisplay({ view, side, docked }: Props) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  /** The newest picture, kept for as long as it is the newest.
   *
   * Kept rather than consumed, and that is the point: the canvas is
   * remounted whenever the panel moves — docked to floating, one side to the
   * other — and a fresh canvas is a blank one. The player only hands over a
   * picture that has *changed*, so a display holding still would leave the
   * new canvas empty until the machine next said something, which on a game
   * over screen is minutes. Holding the last one means a move repaints
   * immediately. */
  const latest = useRef<Uint8Array | null>(null);
  const narrow = useNarrow();
  const [inShot, setInShot] = useState(false);
  const [lit, setLit] = useState(false);
  /** The free strip beside the table: where its centre is and how wide it
   * is, both as fractions of the window. `null` keeps the panel in its
   * corner. */
  const [strip, setStrip] = useState<{ centre: number; room: number } | null>(null);

  // Whether the head is in the picture, and where the table's edges are.
  // Asked again on a change of view and on a slow beat besides, because the
  // framing also moves when the window changes shape and there is no event
  // for "the renderer reframed".
  useEffect(() => {
    let alive = true;
    const ask = () => {
      void backboxRect().then((r) => {
        if (alive) setInShot(r !== null);
      });
      // Overhead on a wide window leaves gutters either side of the table;
      // the panel moves into the one the player chose when it fits with
      // padding to spare, and stays in its corner when it would not.
      if (view === 'overhead' && !docked) {
        void playfieldRect().then((r) => {
          if (!alive) return;
          if (!r) {
            setStrip(null);
            return;
          }
          const [left, , width] = r;
          const right = left + width;
          const room = side === 'left' ? left : 1 - right;
          if (room * window.innerWidth < GUTTER_PX) {
            setStrip(null);
            return;
          }
          setStrip({ centre: side === 'left' ? left / 2 : (right + 1) / 2, room });
        });
      } else {
        setStrip(null);
      }
    };
    ask();
    const timer = window.setInterval(ask, 500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [view, side, docked]);

  // The pixels. Ten times a second is well past what an eye resolves on a
  // segment display, and the call comes back empty unless the machine has
  // actually said something new — so a still display costs nothing at all.
  useEffect(() => {
    let alive = true;
    // Until a picture has arrived it asks for a refresh: the player's
    // change-skip remembers that *somebody* was told, and that somebody can
    // be a mount of this component that no longer exists.
    let received = false;
    const timer = window.setInterval(() => {
      void displayImage(SIZE.width, SIZE.height, !received).then((rgba) => {
        if (!alive || !rgba) return;
        received = true;
        latest.current = rgba;
        // Lit first, drawn second: the canvas does not exist until `lit`
        // renders it, so the picture waits in the ref and the effect below
        // paints it the moment the canvas is real. Doing the paint here and
        // the flag after was the deadlock that kept this panel dark: the
        // first picture hit a null canvas, the change-skip then had nothing
        // new to say, and `lit` never came.
        setLit(true);
        paint();
      });
    }, 100);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  // The catch-up paint, for the picture that arrived before the canvas —
  // and again whenever the panel moves, since React remounts the canvas
  // into its new home empty.
  useEffect(() => {
    paint();
  }, [lit, docked, strip !== null]);

  function paint() {
    const ctx = canvas.current?.getContext('2d');
    if (!ctx || !latest.current) return;
    ctx.putImageData(
      new ImageData(new Uint8ClampedArray(latest.current), SIZE.width, SIZE.height),
      0,
      0,
    );
  }

  // Nothing to show: a table with no ROM rolls a ball perfectly well and has
  // no score, and an empty panel floating over it is furniture. Narrow and
  // undocked means the player asked for it off.
  if (!lit || inShot) return null;
  if (narrow && view === 'overhead' && !docked) return null;

  // Docked: the strip is already laid out by the player, and the canvas only
  // has to sit in the middle of it.
  if (docked) {
    return (
      <div className="score-dock">
        <canvas
          className="score score-docked"
          ref={canvas}
          width={SIZE.width}
          height={SIZE.height}
          aria-hidden="true"
        />
      </div>
    );
  }

  // Floating: centred on the free strip beside the table, vertically centred,
  // and as large as that strip allows. The gutter is empty screen — a panel
  // that stops at a fixed width leaves the reader squinting at a small
  // display with blank space either side of it — so it takes the strip less
  // a margin, up to a ceiling that keeps it from dwarfing the table on a
  // very wide window.
  //
  // The margin is two rem at each end and it is not decoration: the panel is
  // a second lit screen beside the first, and one standing flush against the
  // table's edge reads as part of the machine rather than as something next
  // to it. Taking it off both ends keeps the panel centred in the strip.
  const place = strip
    ? {
        left: `${strip.centre * 100}%`,
        right: 'auto' as const,
        top: '50%',
        transform: 'translate(-50%, -50%)',
        width: `min(calc(${strip.room * 100}vw - 4rem), 40rem)`,
      }
    : undefined;

  return (
    <canvas
      className="score"
      ref={canvas}
      width={SIZE.width}
      height={SIZE.height}
      style={place}
      aria-hidden="true"
    />
  );
}
