// Getting the game's sound out of the player and into the speakers.
//
// The game mixes everything itself and hands over interleaved stereo, so there
// is no Web Audio graph to speak of: one output node, one gain, the
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
// device wants an even stream. The sink holds a queue between the two, and
// the pump keeps that queue at a target depth: too shallow and a slow frame is
// heard as a gap, too deep and the sound lags behind the picture.
//
// The queue lives in one of two places. Where it can, it is an `AudioWorklet`
// — the audio thread's own code, immune to a busy page. But a worklet needs a
// secure context, and this player deliberately runs on plain-http LAN
// addresses too (the same reason the renderer carries a WebGL2 fallback), so
// there is a second sink: a `ScriptProcessorNode`, deprecated, main-thread,
// and available everywhere. Sound over LAN beats silence over principle.
//
// Where the pump runs depends on where the player is and which sink this page
// got. With a worklet and a worker, the pump lives with the game
// (`game.worker.ts`) and pushes straight to the worklet through a
// `MessageChannel` — the page is not on the path at all. Every other
// combination pumps here, on an animation frame of this page. The gain stays
// here either way: it is the page's volume knob.

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

/** One end of the queue between the game and the device.
 *
 * Both sinks take interleaved stereo through `push` and answer how deep the
 * queue is; where the samples actually wait — the audio thread or this one —
 * is the difference between them.
 */
interface Sink {
  /** The node to wire into the graph. */
  node: AudioNode;
  /** Frames still waiting to be played. */
  queued(): number;
  /** Frames of silence invented for want of anything to play. */
  starved(): number;
  push(pcm: Float32Array): void;
  flush(): void;
  stop(): void;
}

/** The queue on the audio thread, where a busy page cannot reach it. */
class WorkletSink implements Sink {
  node: AudioWorkletNode;
  /** Frames the worklet says it still has. Updated about 45 times a second,
   * and counted up optimistically on every push in between — the report
   * arrives every 22 ms, and a burst of frames would otherwise each think
   * the queue is still empty and send the same audio several times over. */
  private q = 0;
  private s = 0;

  constructor(node: AudioWorkletNode) {
    this.node = node;
    node.port.onmessage = (event: MessageEvent<Report>) => {
      this.q = event.data.queued;
      this.s = event.data.starved;
    };
  }

  queued(): number {
    return this.q;
  }

  starved(): number {
    return this.s;
  }

  push(pcm: Float32Array): void {
    this.q += pcm.length / 2;
    this.node.port.postMessage(pcm, [pcm.buffer as ArrayBuffer]);
  }

  flush(): void {
    this.q = 0;
    this.node.port.postMessage('flush');
  }

  stop(): void {
    this.node.port.postMessage('stop');
  }
}

/** The queue on this thread, for pages a worklet is not allowed on.
 *
 * `ScriptProcessorNode` has been deprecated for a decade and still ships in
 * every browser, because it is the only way to make sound from an insecure
 * page. Its callback runs here on the main thread, so a long stall can be
 * heard — the worklet exists because of that — but the buffer below rides out
 * anything short of a real hang.
 */
class ProcessorSink implements Sink {
  node: ScriptProcessorNode;
  private chunks: Float32Array[] = [];
  /** Frames already played out of `chunks[0]`. */
  private offset = 0;
  private frames = 0;
  private starvedCount = 0;

  constructor(context: AudioContext) {
    this.node = context.createScriptProcessor(1024, 0, 2);
    this.node.onaudioprocess = (event) => {
      const left = event.outputBuffer.getChannelData(0);
      const right = event.outputBuffer.getChannelData(1);
      let i = 0;
      while (i < left.length && this.chunks.length > 0) {
        const chunk = this.chunks[0];
        left[i] = chunk[this.offset * 2];
        right[i] = chunk[this.offset * 2 + 1];
        i += 1;
        this.offset += 1;
        if (this.offset * 2 >= chunk.length) {
          this.chunks.shift();
          this.offset = 0;
        }
      }
      this.frames = Math.max(0, this.frames - left.length);
      if (i < left.length) {
        this.starvedCount += left.length - i;
        left.fill(0, i);
        right.fill(0, i);
      }
    };
  }

  queued(): number {
    return this.frames;
  }

  starved(): number {
    return this.starvedCount;
  }

  push(pcm: Float32Array): void {
    this.chunks.push(pcm);
    this.frames += pcm.length / 2;
  }

  flush(): void {
    this.chunks = [];
    this.offset = 0;
    this.frames = 0;
  }

  stop(): void {
    this.node.onaudioprocess = null;
  }
}

class Engine {
  private context: AudioContext;
  private sink: Sink;
  private gain: GainNode;
  private host: PlayerHost;
  /** True when the worker feeds the worklet itself and this page must not. */
  private workerPumps: boolean;
  /** Whether a render is already in flight, so the pump never overlaps itself. */
  private rendering = false;
  private stopped = false;

  private constructor(
    context: AudioContext,
    sink: Sink,
    gain: GainNode,
    host: PlayerHost,
    workerPumps: boolean,
  ) {
    this.context = context;
    this.sink = sink;
    this.gain = gain;
    this.host = host;
    this.workerPumps = workerPumps;
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

    let sink: Sink;
    let workerPumps = false;
    // A worklet wants a secure context, and `audioWorklet` is simply absent
    // without one — the same test WebGPU fails on the same pages.
    if (context.audioWorklet) {
      // `?url` rather than an import: an AudioWorklet module is fetched by the
      // browser and evaluated in the audio thread's own scope, so it must stay
      // a separate file rather than being bundled into the page.
      const url = (await import('../audio/worklet.js?url')).default;
      await context.audioWorklet.addModule(url);

      const node = new AudioWorkletNode(context, 'playfield', {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
      });
      sink = new WorkletSink(node);

      // When the game lives in a worker, the samples flow worker → worklet on
      // a channel of their own; each end gets a port. The node's own port
      // stays here for `stop` and `flush`.
      if (host.kind === 'worker') {
        const channel = new MessageChannel();
        node.port.postMessage({ port: channel.port1 }, [channel.port1]);
        host.attachAudio(channel.port2, rate);
        workerPumps = true;
      }
    } else {
      console.warn(
        '[audio] no AudioWorklet on this page (an insecure context?): falling back to a ScriptProcessorNode',
      );
      sink = new ProcessorSink(context);
    }

    const gain = context.createGain();
    sink.node.connect(gain).connect(context.destination);
    return new Engine(context, sink, gain, host, workerPumps);
  }

  get rate(): number {
    return this.context.sampleRate;
  }

  get state(): AudioContextState {
    return this.context.state;
  }

  /** Whether this page has to pump, or the worker is doing it. */
  get pumpsHere(): boolean {
    return !this.workerPumps;
  }

  /** Frames of silence played for want of anything to play. */
  get underruns(): number {
    return this.sink.starved();
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
   * For every path but worker-with-worklet — there the worker pumps for
   * itself. Call it once per animation frame; the samples it produces are the
   * ones belonging to the time that just passed.
   */
  pump(): void {
    if (this.stopped || this.rendering || this.context.state !== 'running') return;

    const target = this.rate * TARGET_SECONDS;
    const want = Math.min(
      Math.ceil(target - this.sink.queued()),
      Math.ceil(this.rate * MAX_CHUNK_SECONDS),
    );
    if (want <= 0) return;

    // The in-flight flag covers the await, so a burst of animation frames
    // cannot each render the same stretch of time.
    this.rendering = true;
    this.host
      .call<Float32Array>('renderAudio', [want])
      .then((pcm) => {
        this.rendering = false;
        if (pcm.length === 0) return;
        this.sink.push(pcm);
      })
      .catch(() => {
        this.rendering = false;
      });
  }

  /** Throws away what is queued. For when a table is swapped underneath. */
  flush(): void {
    this.sink.flush();
  }

  async close(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    this.sink.stop();
    this.sink.node.disconnect();
    await this.context.close();
  }
}

let engine: Promise<Engine> | null = null;
let pumping = 0;
/** Cancels the settings subscription. Set while the engine is alive. */
let unsubscribe: (() => void) | null = null;

/** The audio engine, built once.
 *
 * A failed build is not kept: caching the rejection would make one bad moment
 * — a device briefly refusing a context, say — permanent, when the next user
 * gesture could simply try again.
 */
export function audio(): Promise<Engine> {
  engine ??= Engine.create().catch((e: unknown) => {
    engine = null;
    throw e;
  });
  return engine;
}

/**
 * Lets the sound start, and keeps it fed.
 *
 * Must be called from inside a user gesture — a click or a keypress — because
 * that is the only place a browser will start an `AudioContext`. Calling it
 * again once it is running is harmless, and so is calling it where sound is
 * impossible: a page that cannot make noise logs one warning and plays on
 * silently.
 *
 * On the paths where this page pumps, the pump runs on its own animation
 * frame here; it does not need to be in step with the game — the queue
 * between the two is what absorbs the difference — only to run about as
 * often. It also stops when the tab is hidden, which is exactly right: the
 * game stops there too.
 */
export async function startAudio(): Promise<void> {
  let live: Engine;
  let host: PlayerHost;
  try {
    [live, host] = await Promise.all([audio(), getHost()]);
  } catch (e) {
    console.warn('[audio] no sound on this page:', e);
    return;
  }
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
  try {
    await (await current).close();
  } catch {
    // A build that failed has nothing to close.
  }
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
