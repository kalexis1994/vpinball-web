// Getting the game's sound out of the player and into the speakers.
//
// The game mixes everything itself and hands over interleaved stereo, so there
// is no Web Audio graph to speak of: one worklet node, one gain, the
// destination. What this module actually deals with is the two problems that
// come with browser audio and have nothing to do with sound.
//
// **A page cannot make noise until the user has touched it.** Every browser
// suspends a fresh `AudioContext` and only lets it start from inside a real
// input event. So starting is deferred: the context is built when the player is
// and resumed on the first click or keypress.
//
// **The game's clock is not the audio clock.** The game produces sound in
// bursts, once per animation frame, at whatever rate its frames come. The
// device wants an even stream. The worklet holds a queue between the two, and
// the pump keeps that queue at a target depth: too shallow and a slow frame is
// heard as a gap, too deep and the sound lags behind the picture.
//
// Where the pump runs depends on where the player does. On the main thread it
// is here, on an animation frame of this page. In a worker the pump lives with
// the game (`game.worker.ts`) and pushes straight to the worklet through a
// `MessageChannel` this module wires up — the page is not on the path at all,
// so a busy page cannot starve the sound. The gain stays here either way: it
// is the page's volume knob.

import { getHost, type PlayerHost } from './host';
import { onSettingsChange, settings } from './settings';

/** How much audio to keep queued ahead, in seconds.
 *
 * Long enough to ride out a frame that takes three times as long as it should,
 * short enough that a flipper is not heard after the ball has left it. Below
 * about 60 ms a single slow frame is audible; above about 150 ms so is the
 * delay. */
const TARGET_SECONDS = 0.1;

/** The most to render in one go, so a stall cannot ask for a second of audio. */
const MAX_CHUNK_SECONDS = 0.25;

interface Report {
  queued: number;
  starved: number;
}

class Engine {
  private context: AudioContext;
  private node: AudioWorkletNode;
  private gain: GainNode;
  private host: PlayerHost;
  /** Frames the worklet says it still has. Updated about 45 times a second. */
  private queued = 0;
  /** Frames of silence the worklet has had to invent. A running count. */
  private starved = 0;
  /** Whether a render is already in flight, so the pump never overlaps itself. */
  private rendering = false;
  private stopped = false;

  private constructor(
    context: AudioContext,
    node: AudioWorkletNode,
    gain: GainNode,
    host: PlayerHost,
  ) {
    this.context = context;
    this.node = node;
    this.gain = gain;
    this.host = host;
    this.node.port.onmessage = (event: MessageEvent<Report>) => {
      this.queued = event.data.queued;
      this.starved = event.data.starved;
    };
  }

  static async create(): Promise<Engine> {
    const host = await getHost();
    // Asked for at the game's own rate. The browser resamples if the device
    // cannot do it, which is one resampling; asking for the device's rate would
    // mean resampling the sound board's stream here as well.
    const rate = await host.call<number>('audioRate');
    const context = new AudioContext({
      sampleRate: rate,
      latencyHint: 'interactive',
    });

    // `?url` rather than an import: an AudioWorklet module is fetched by the
    // browser and evaluated in the audio thread's own scope, so it must stay a
    // separate file rather than being bundled into the page.
    const url = (await import('../audio/worklet.js?url')).default;
    await context.audioWorklet.addModule(url);

    const node = new AudioWorkletNode(context, 'playfield', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    const gain = context.createGain();
    node.connect(gain).connect(context.destination);

    // When the game lives in a worker, the samples flow worker → worklet on a
    // channel of their own; each end gets a port. The node's own port stays
    // here for `stop` and `flush`.
    if (host.kind === 'worker') {
      const channel = new MessageChannel();
      node.port.postMessage({ port: channel.port1 }, [channel.port1]);
      host.attachAudio(channel.port2, rate);
    }

    return new Engine(context, node, gain, host);
  }

  get rate(): number {
    return this.context.sampleRate;
  }

  get state(): AudioContextState {
    return this.context.state;
  }

  /** Whether this page has to pump, or the worker is doing it. */
  get pumpsHere(): boolean {
    return this.host.kind === 'main';
  }

  /** Frames of silence played for want of anything to play. */
  get underruns(): number {
    return this.starved;
  }

  /** Starts the context. Only works from inside a real user gesture. */
  async resume(): Promise<void> {
    if (this.context.state !== 'running') {
      await this.context.resume();
    }
  }

  setVolume(volume: number): void {
    // Ramped rather than set: an instant change in gain is a click.
    const now = this.context.currentTime;
    this.gain.gain.cancelScheduledValues(now);
    this.gain.gain.setTargetAtTime(Math.max(0, Math.min(1, volume)), now, 0.02);
  }

  /**
   * Renders whatever is needed to keep the queue at its target depth.
   *
   * Main-thread path only — the worker pumps for itself. Call it once per
   * animation frame; the samples it produces are the ones belonging to the
   * time that just passed.
   */
  pump(): void {
    if (this.stopped || this.rendering || this.context.state !== 'running') return;

    const target = this.rate * TARGET_SECONDS;
    const want = Math.min(
      Math.ceil(target - this.queued),
      Math.ceil(this.rate * MAX_CHUNK_SECONDS),
    );
    if (want <= 0) return;

    // Counted before the render lands rather than waiting for the worklet's
    // next report, which arrives every 22 ms and would otherwise let a burst
    // of frames each think the queue is still empty and send the same audio
    // several times over. The in-flight flag covers the await.
    this.rendering = true;
    this.host
      .call<Float32Array>('renderAudio', [want])
      .then((pcm) => {
        this.rendering = false;
        if (pcm.length === 0) return;
        this.queued += pcm.length / 2;
        this.node.port.postMessage(pcm, [pcm.buffer as ArrayBuffer]);
      })
      .catch(() => {
        this.rendering = false;
      });
  }

  /** Throws away what is queued. For when a table is swapped underneath. */
  flush(): void {
    this.queued = 0;
    this.node.port.postMessage('flush');
  }

  async close(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    this.node.port.postMessage('stop');
    this.node.disconnect();
    await this.context.close();
  }
}

let engine: Promise<Engine> | null = null;
let pumping = 0;
/** Cancels the settings subscription. Set while the engine is alive. */
let unsubscribe: (() => void) | null = null;

/** The audio engine, built once. */
export function audio(): Promise<Engine> {
  engine ??= Engine.create();
  return engine;
}

/**
 * Lets the sound start, and keeps it fed.
 *
 * Must be called from inside a user gesture — a click or a keypress — because
 * that is the only place a browser will start an `AudioContext`. Calling it
 * again once it is running is harmless.
 *
 * On the main-thread path the pump runs on its own animation frame here; it
 * does not need to be in step with the game — the queue between the two is
 * what absorbs the difference — only to run about as often. It also stops when
 * the tab is hidden, which is exactly right: the game stops there too. On the
 * worker path there is nothing to run here at all.
 */
export async function startAudio(): Promise<void> {
  const [live, host] = await Promise.all([audio(), getHost()]);
  try {
    await live.resume();
  } catch (e) {
    console.warn('[audio] could not start:', e);
    return;
  }

  // Whatever the player last chose, and whatever they choose from here on.
  // Subscribing rather than reading once is what lets the settings screen move
  // the slider while a table is playing behind it.
  live.setVolume(settings().volume);
  void host.call('setVolume', [1]);
  unsubscribe ??= onSettingsChange((s) => live.setVolume(s.volume));
  if (!live.pumpsHere || pumping) return;

  // The pump runs on its own animation frame, so nothing on the Rust side ever
  // sees how long it took — and a hitch the player feels has to come from
  // somewhere. Anything past half a frame is handed to the rolling record, so
  // a dump can say whether the stall was this or the browser.
  let lastEnd = performance.now();
  const tick = () => {
    if (!engine) {
      pumping = 0;
      return;
    }
    const idle = performance.now() - lastEnd;
    const t = performance.now();
    live.pump();
    const took = performance.now() - t;
    if (took > 8) void host.call('notePause', ['audio pump', took]);
    // And the gap before it: if nothing ran for a long time, the browser was
    // busy with something that is neither the pump nor the frame.
    if (idle > 34) void host.call('notePause', ['nothing ran', idle]);
    lastEnd = performance.now();
    pumping = requestAnimationFrame(tick);
  };
  pumping = requestAnimationFrame(tick);
}

/** Shuts the audio down and releases the device. */
export async function stopAudio(): Promise<void> {
  if (pumping) {
    cancelAnimationFrame(pumping);
    pumping = 0;
  }
  unsubscribe?.();
  unsubscribe = null;
  if (!engine) return;
  const current = engine;
  engine = null;
  await (await current).close();
}

/** Master volume, 0 to 1. Applied to the graph and to the mix inside the game.
 *
 * Both, because they do different things: the graph's gain fades smoothly and
 * belongs to the page, while the game's clamps the mix before it is summed and
 * is what keeps a loud table from clipping.
 *
 * Prefer `updateSettings({ volume })`, which remembers the choice and reaches
 * here through the subscription. This is the direct route, for a caller that
 * wants to change the volume without changing the setting.
 */
export async function setVolume(volume: number): Promise<void> {
  const [live, host] = await Promise.all([audio(), getHost()]);
  live.setVolume(volume);
  void host.call('setVolume', [1]);
}

export type { Engine };
