// Where the player runs: in a worker when the browser can, on this thread when
// it cannot.
//
// The worker is the preferred home — the simulation stops competing with React
// and the browser for the main thread, and parsing a 111 MB table stops
// freezing the page — but it needs three things not every browser grants:
// `transferControlToOffscreen`, WebGPU inside a worker, and an animation frame
// in a worker scope. So the choice is made once, on evidence: the page checks
// the transfer exists, then asks a fresh worker to actually obtain a GPU
// adapter. Only if that succeeds is the canvas transferred, because a transfer
// cannot be taken back; anything less and the player runs exactly where it
// always did.
//
// `?host=main` on the URL forces the fallback, which is how the path that only
// some browsers take gets exercised on one that does not need it.
//
// Both homes answer the same interface. `call(op, args)` names a wasm export;
// the worker forwards it and the main thread calls it directly, and the two
// answers with awkward shapes are flattened at the source (`hostShared.ts`) so
// the page cannot tell which home replied.

import GameWorker from './game.worker?worker';
import { loopStats, sceneStats } from './hostShared';

export interface PlayerHost {
  readonly kind: 'worker' | 'main';
  /** Calls one of the player's exports by name. */
  call<T = void>(op: string, args?: unknown[], transfer?: Transferable[]): Promise<T>;
  /** Starts the player on this canvas, or moves it there. */
  start(canvas: HTMLCanvasElement): Promise<void>;
  /** Whether anyone can see the player. Worker path only; the main-thread
   * player watches its own canvas leave the document. */
  setVisible(visible: boolean): void;
  /** Hands the worker its end of the channel to the audio worklet. */
  attachAudio(port: MessagePort, rate: number): void;
  /**
   * Synchronous reads and writes for the per-frame UI: the on-screen plunger
   * follows a finger, and a promise per pointer event is a promise too many.
   * On the worker path the position is pushed here every frame, so reading it
   * is reading a cached number.
   */
  plungerPull(): number | null;
  holdPlunger(travel: number): void;
  releasePlunger(): void;
}

let chosen: Promise<PlayerHost> | null = null;

/** The player's home, decided once per page. */
export function getHost(): Promise<PlayerHost> {
  chosen ??= choose();
  return chosen;
}

/**
 * `?gpu=gl` pins the renderer to WebGL2, the way `?host=main` pins the home:
 * it is the backend a phone on plain-HTTP LAN gets — an insecure origin has no
 * `navigator.gpu` at all — and forcing it is how that path gets exercised on a
 * desktop that would never take it.
 */
function forceWebGl(): boolean {
  return new URLSearchParams(window.location.search).get('gpu') === 'gl';
}

async function choose(): Promise<PlayerHost> {
  const forced = new URLSearchParams(window.location.search).get('host');
  const canTransfer = 'transferControlToOffscreen' in HTMLCanvasElement.prototype;
  if (forced !== 'main' && canTransfer) {
    const worker = new GameWorker();
    const rpc = new Rpc(worker);
    try {
      // Bounded: a worker that never answers — the module failed to load, a
      // driver hung — must not hold the whole page on a spinner.
      const capable = await withTimeout(rpc.call<boolean>('probe', [forceWebGl()]), 8_000);
      if (capable) {
        console.debug('[player] running in a worker');
        return workerHost(rpc);
      }
      console.debug('[player] no GPU in a worker here; using the main thread');
    } catch (e) {
      console.warn('[player] worker probe failed; using the main thread:', e);
    }
    worker.terminate();
  }
  return mainHost();
}

function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(new Error(`no answer in ${ms} ms`)), ms);
    p.then(
      (v) => {
        window.clearTimeout(timer);
        resolve(v);
      },
      (e: unknown) => {
        window.clearTimeout(timer);
        reject(e instanceof Error ? e : new Error(String(e)));
      },
    );
  });
}

// -- the worker home ----------------------------------------------------------

interface Reply {
  kind: 'reply';
  id: number;
  ok: boolean;
  value?: unknown;
  error?: string;
}

interface Push {
  kind: 'plunger';
  value: number | null;
}

/** The request/reply pairing over the worker's one message stream. */
class Rpc {
  private next = 1;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  /** Latest pushed plunger position. See {@link PlayerHost.plungerPull}. */
  plunger: number | null = null;

  constructor(readonly worker: Worker) {
    worker.onmessage = (event: MessageEvent<Reply | Push>) => {
      const data = event.data;
      if (data.kind === 'plunger') {
        this.plunger = data.value;
        return;
      }
      const waiter = this.pending.get(data.id);
      if (!waiter) return;
      this.pending.delete(data.id);
      if (data.ok) waiter.resolve(data.value);
      else waiter.reject(new Error(data.error ?? 'the worker failed without saying why'));
    };
    // A worker that dies mid-call must not leave its callers hanging for
    // ever on promises nothing will ever settle: everyone waiting gets the
    // failure, which surfaces as the same load error a main-thread crash
    // would have been.
    worker.onerror = (event: ErrorEvent) => this.fail(event.message || 'the worker crashed');
    worker.onmessageerror = () => this.fail('a message to or from the worker could not be read');
  }

  private fail(why: string): void {
    const waiters = [...this.pending.values()];
    this.pending.clear();
    for (const waiter of waiters) waiter.reject(new Error(why));
  }

  call<T>(op: string, args: unknown[] = [], transfer: Transferable[] = []): Promise<T> {
    const id = this.next++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ id, op, args }, transfer);
    });
  }
}

function workerHost(rpc: Rpc): PlayerHost {
  // A canvas can only be transferred once, and React in strict mode runs every
  // effect twice: the second start on the same element must reuse what the
  // worker already has instead of throwing on a second transfer.
  const transferred = new WeakSet<HTMLCanvasElement>();
  let observer: ResizeObserver | null = null;

  const deviceSize = (canvas: HTMLCanvasElement): [number, number] => {
    const dpr = window.devicePixelRatio || 1;
    return [
      Math.max(1, Math.round(canvas.clientWidth * dpr)),
      Math.max(1, Math.round(canvas.clientHeight * dpr)),
    ];
  };

  return {
    kind: 'worker',
    call: (op, args, transfer) => rpc.call(op, args, transfer),

    async start(canvas: HTMLCanvasElement): Promise<void> {
      const [width, height] = deviceSize(canvas);
      if (transferred.has(canvas)) {
        // Strict mode's second pass, or a return to a canvas that survived.
        await rpc.call('resizeSurface', [width, height]);
        await rpc.call('setVisible', [true]);
      } else {
        const offscreen = canvas.transferControlToOffscreen();
        transferred.add(canvas);
        await rpc.call(
          'start',
          [{ canvas: offscreen, width, height, forceGl: forceWebGl() }],
          [offscreen],
        );
      }
      // Layout lives here and the renderer lives there, so the element is
      // watched here and the worker is told in device pixels.
      observer?.disconnect();
      observer = new ResizeObserver(() => {
        const [w, h] = deviceSize(canvas);
        void rpc.call('resizeSurface', [w, h]);
      });
      observer.observe(canvas);
    },

    setVisible(visible: boolean): void {
      void rpc.call('setVisible', [visible]);
    },

    attachAudio(port: MessagePort, rate: number): void {
      void rpc.call('audio', [{ port, rate }], [port]);
    },

    plungerPull: () => rpc.plunger,
    holdPlunger(travel: number): void {
      void rpc.call('holdPlunger', [travel]);
    },
    releasePlunger(): void {
      void rpc.call('releasePlunger', []);
    },
  };
}

// -- the main-thread home -----------------------------------------------------

type Wasm = typeof import('../wasm/vpw_player.js');

let wasmReady: Promise<Wasm> | null = null;

/** Loads and initialises the wasm module on this thread. Idempotent. */
function initWasm(): Promise<Wasm> {
  wasmReady ??= (async () => {
    const wasm = await import('../wasm/vpw_player.js');
    const url = (await import('../wasm/vpw_player_bg.wasm?url')).default;
    await wasm.default({ module_or_path: url });
    return wasm;
  })();
  return wasmReady;
}

async function mainHost(): Promise<PlayerHost> {
  // Before the module runs: the renderer reads the flag off the global when
  // the instance is built.
  if (forceWebGl()) (globalThis as { VPW_FORCE_WEBGL?: boolean }).VPW_FORCE_WEBGL = true;
  const wasm = await initWasm();
  return {
    kind: 'main',

    async call<T>(op: string, args: unknown[] = []): Promise<T> {
      // The same two special shapes the worker flattens, flattened here, so
      // the page sees one API. Everything else is the export, called.
      if (op === 'loadTable') {
        // A breather so the browser can paint the loading notice before the
        // parse locks this thread up. `setTimeout` and not rAF: rAF does not
        // fire in a background tab, and a table opened there would hang.
        await new Promise((r) => setTimeout(r, 0));
        return sceneStats(wasm.loadTable(args[0] as Uint8Array)) as T;
      }
      if (op === 'loopStats') {
        const l = wasm.loopStats();
        return (l ? loopStats(l) : null) as T;
      }
      const fn = (wasm as unknown as Record<string, unknown>)[op];
      if (typeof fn !== 'function') throw new Error(`the player has no export named '${op}'`);
      return (fn as (...a: unknown[]) => T)(...args);
    },

    start: (canvas: HTMLCanvasElement) => wasm.start(canvas.id),

    // The main-thread player watches its own canvas: a detached element is
    // how it knows nobody is looking.
    setVisible(): void {},

    attachAudio(): void {
      throw new Error('the main-thread player pumps its own audio');
    },

    plungerPull: () => wasm.plungerPull() ?? null,
    holdPlunger: (travel: number) => wasm.holdPlunger(travel),
    releasePlunger: () => wasm.releasePlunger(),
  };
}
