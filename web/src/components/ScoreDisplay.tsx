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
// On a narrow screen —a phone held upright, which is the case the overhead view
// exists for— it is not drawn either. There is no room beside the table and
// none above it, and a panel that covers the flippers is worse than no panel.

import { useEffect, useRef, useState } from 'react';
import { backboxRect, displayImage } from '../lib/player';
import type { CameraView } from '../lib/settings';

/** Below this the overhead view is the whole screen and nothing else fits. */
const NARROW_PX = 720;

/** How big the canvas is. Wide and short, which is the shape of two rows of
 *  sixteen digits, and small enough that handing the pixels over costs little. */
const SIZE = { width: 512, height: 128 };

interface Props {
  view: CameraView;
}

export function ScoreDisplay({ view }: Props) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const [narrow, setNarrow] = useState(() => window.innerWidth < NARROW_PX);
  const [inShot, setInShot] = useState(false);
  const [lit, setLit] = useState(false);

  useEffect(() => {
    const onResize = () => setNarrow(window.innerWidth < NARROW_PX);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  // Whether the head is in the picture. Asked again on a change of view and on
  // a slow beat besides, because the framing also moves when the window changes
  // shape and there is no event for "the renderer reframed".
  useEffect(() => {
    let alive = true;
    const ask = () =>
      void backboxRect().then((r) => {
        if (alive) setInShot(r !== null);
      });
    ask();
    const timer = window.setInterval(ask, 500);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [view]);

  // The pixels. Ten times a second is well past what an eye resolves on a
  // segment display, and the call comes back empty unless the machine has
  // actually said something new — so a still display costs nothing at all.
  useEffect(() => {
    let alive = true;
    const timer = window.setInterval(() => {
      void displayImage(SIZE.width, SIZE.height).then((rgba) => {
        if (!alive || !rgba) return;
        const ctx = canvas.current?.getContext('2d');
        if (!ctx) return;
        ctx.putImageData(
          new ImageData(new Uint8ClampedArray(rgba), SIZE.width, SIZE.height),
          0,
          0,
        );
        setLit(true);
      });
    }, 100);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  // Nothing to show: a table with no ROM rolls a ball perfectly well and has no
  // score, and an empty panel floating over it is furniture.
  if (!lit || inShot || (narrow && view === 'overhead')) return null;

  return (
    <canvas
      className="score"
      ref={canvas}
      width={SIZE.width}
      height={SIZE.height}
      aria-hidden="true"
    />
  );
}
