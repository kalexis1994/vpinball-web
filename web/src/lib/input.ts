// The table's controls, wired on the page.
//
// One wiring for both of the player's homes. It used to live in Rust, as DOM
// listeners the wasm module attached to `window` — which a worker cannot do,
// there being no DOM in one. Rather than keep that wiring for one path and
// build this for the other, the page owns the listeners for both: every event
// becomes the same call the touch controls already make, and the wasm side
// keeps only what it is for — what a key does to the table.
//
// `preventDefault` cannot wait for an answer from a worker, so the keys whose
// browser default has to be suppressed are named here: the ones that scroll
// the page or move the caret. A table that jumps every time you nudge is
// unplayable.
//
// A gamepad arrives the same way, through the same actions, from a map of its
// own — see `lib/controls` for why the two are kept apart. It is polled
// rather than listened to, because that is the only way a browser offers it.


import { getHost, type PlayerHost } from './host';
import { actionForButton, actionForKey } from './controls';
import { settings } from './settings';

/** The keys whose browser default fights the table. */
const SCROLLING_KEYS = new Set(['Space', 'ArrowLeft', 'ArrowRight', 'ArrowUp']);

/**
 * Wires the keyboard and the pointer to the player. Returns the unwiring.
 *
 * The keyboard goes on `window` and not on the canvas: a `<canvas>` does not
 * take focus unless it is given a `tabindex`, so keys aimed at it would end up
 * nowhere. The flip side — they also fire while a menu overlay is open — is
 * harmless, because the table under it is the thing being played either way.
 *
 * The `keyup` on `blur` matters: alt-tabbing away with a flipper held down
 * delivers the `keyup` to whatever window took the focus, and without
 * releasing everything you come back to a flipper standing up with no way to
 * lower it.
 */
export function connectInput(canvas: HTMLCanvasElement): () => void {
  // The host resolves once per page and this is called after the player has
  // started, so the race is theoretical — but a keypress before it resolves
  // should be dropped, not crash.
  let host: PlayerHost | null = null;
  void getHost().then((h) => {
    host = h;
  });

  // What a key does is looked up rather than assumed: the player's own
  // vocabulary never changes — `KeyZ` is a left flipper because that is what
  // the table's script listens for — and rebinding moves which physical key
  // speaks it. A key bound to nothing is still passed on, because a table's
  // script may listen for keys this page has no opinion about.
  const act = (code: string, pressed: boolean) => {
    const action = actionForKey(settings().keys, code);
    if (!action) {
      void host?.call('pressKey', [code, pressed]);
      return;
    }
    if (action.id === 'newBall') {
      // A player decision rather than a cabinet one, so it is answered here
      // instead of being handed to the table.
      if (pressed) void host?.call('newBall');
      return;
    }
    void host?.call('pressKey', [action.code, pressed]);
  };

  const keydown = (e: KeyboardEvent) => {
    // Auto-repeat is not a new press. The controls guard against it anyway,
    // but leaving it out saves fifteen calls a second per key.
    if (e.repeat) return;
    if (SCROLLING_KEYS.has(e.code) || actionForKey(settings().keys, e.code)) {
      e.preventDefault();
    }
    act(e.code, true);
  };
  const keyup = (e: KeyboardEvent) => {
    act(e.code, false);
  };
  const blur = () => {
    void host?.call('releaseAllKeys');
  };

  // The camera drag. Provisional, like the camera it moves: once the
  // original's `ViewSetup` is in, the view is the table's and this stays only
  // for inspection. The feel — degrees per pixel — lives on the wasm side;
  // this only reports what the mouse did.
  let dragging = false;
  let last: [number, number] = [0, 0];
  const mousedown = (e: MouseEvent) => {
    dragging = true;
    last = [e.clientX, e.clientY];
  };
  const mouseup = () => {
    dragging = false;
  };
  const mousemove = (e: MouseEvent) => {
    if (!dragging) return;
    const dx = e.clientX - last[0];
    const dy = e.clientY - last[1];
    last = [e.clientX, e.clientY];
    void host?.call('cameraOrbit', [dx, dy]);
  };
  const wheel = (e: WheelEvent) => {
    e.preventDefault();
    void host?.call('cameraZoom', [e.deltaY > 0]);
  };

  // The gamepad, which no browser will tell you about: it has to be polled,
  // and only the edges matter — a button that was up and is now down is a
  // press, and the frame after that is nothing at all.
  const held = new Map<number, boolean>();
  let polling = 0;
  const poll = () => {
    polling = requestAnimationFrame(poll);
    const pads = navigator.getGamepads?.() ?? [];
    // The first pad that is actually there. A browser hands back a sparse
    // array with holes in it where controllers have come and gone.
    const pad = pads.find((p) => p && p.connected);
    if (!pad) {
      if (held.size > 0) {
        // Unplugged mid-game with a flipper down: let go of everything.
        held.clear();
        void host?.call('releaseAllKeys');
      }
      return;
    }
    const map = settings().pad;
    for (const [i, button] of pad.buttons.entries()) {
      const down = button.pressed || button.value > 0.5;
      if (down === (held.get(i) ?? false)) continue;
      held.set(i, down);
      const action = actionForButton(map, i);
      if (!action) continue;
      if (action.id === 'newBall') {
        if (down) void host?.call('newBall');
        continue;
      }
      void host?.call('pressKey', [action.code, down]);
    }
  };
  polling = requestAnimationFrame(poll);

  window.addEventListener('keydown', keydown);
  window.addEventListener('keyup', keyup);
  window.addEventListener('blur', blur);
  canvas.addEventListener('mousedown', mousedown);
  canvas.addEventListener('mouseup', mouseup);
  canvas.addEventListener('mouseleave', mouseup);
  canvas.addEventListener('mousemove', mousemove);
  canvas.addEventListener('wheel', wheel, { passive: false });

  return () => {
    cancelAnimationFrame(polling);
    window.removeEventListener('keydown', keydown);
    window.removeEventListener('keyup', keyup);
    window.removeEventListener('blur', blur);
    canvas.removeEventListener('mousedown', mousedown);
    canvas.removeEventListener('mouseup', mouseup);
    canvas.removeEventListener('mouseleave', mouseup);
    canvas.removeEventListener('mousemove', mousemove);
    canvas.removeEventListener('wheel', wheel);
  };
}
