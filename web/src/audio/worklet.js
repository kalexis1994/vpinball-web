// The audio thread. Plays whatever the game hands it, and nothing else.
//
// This file is loaded by `AudioWorklet.addModule`, which means it runs in its
// own global scope with no DOM, no wasm module and no imports. It cannot ask
// the game for samples; the main thread pushes them in through the port. That
// is the whole reason it exists: `process` is called on a real-time thread every
// 128 frames, and a game frame that takes 20 ms would otherwise be a 20 ms hole
// in the sound.
//
// Its only job is to absorb the difference between the two clocks. The game
// produces audio in bursts of a frame's worth; the device consumes it in even
// 128-frame blocks. A queue in between turns the bursts into a stream.

/** How much to hold before deciding the game has fallen too far behind.
 *
 * `sampleRate` is a global in the worklet scope, so this is half a second
 * whatever rate the context ended up at. */
const MAX_QUEUED_FRAMES = sampleRate / 2;

class PlayfieldProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    /** Blocks of interleaved stereo waiting to be played. */
    this.queue = [];
    /** How many frames are in the queue, kept rather than recomputed. */
    this.queued = 0;
    /** How far into `queue[0]` we are, in frames. */
    this.offset = 0;
    /** Frames of silence played for want of anything else. */
    this.starved = 0;
    this.running = true;
    /**
     * Where the samples come from. The node's own port by default; when the
     * game runs in a worker, the page hands over one end of a channel whose
     * other end is the worker's, and the samples flow worker → here without
     * the page in between. The reports go back the way the samples came,
     * because that is who decides how much to send next.
     */
    this.feed = null;

    this.port.onmessage = (event) => this.take(event.data);
  }

  take(data) {
    if (data === 'stop') {
      this.running = false;
      return;
    }
    if (data === 'flush') {
      this.queue = [];
      this.queued = 0;
      this.offset = 0;
      return;
    }
    if (data && data.port instanceof MessagePort) {
      this.feed = data.port;
      this.feed.onmessage = (event) => this.take(event.data);
      return;
    }
    if (!(data instanceof Float32Array)) return;

    this.queue.push(data);
    this.queued += data.length / 2;
    // If the game has run far ahead — which happens when a tab comes back
    // from the background owing a second of catch-up — keep the newest and
    // drop the rest. Playing it all would put the sound permanently behind
    // the picture.
    while (this.queued > MAX_QUEUED_FRAMES && this.queue.length > 1) {
      const dropped = this.queue.shift();
      this.queued -= dropped.length / 2 - this.offset;
      this.offset = 0;
    }
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    if (!output || output.length === 0) return this.running;

    const left = output[0];
    const right = output.length > 1 ? output[1] : output[0];
    const frames = left.length;

    for (let i = 0; i < frames; i++) {
      const block = this.queue[0];
      if (block === undefined) {
        // Nothing to play. Silence, not the last sample held: a held sample
        // buzzes at the block rate and is far more noticeable than a gap.
        left[i] = 0;
        if (right !== left) right[i] = 0;
        this.starved++;
        continue;
      }
      const at = this.offset * 2;
      left[i] = block[at];
      if (right !== left) right[i] = block[at + 1];

      this.offset++;
      this.queued--;
      if (this.offset * 2 >= block.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }

    // Tell whoever feeds us how far ahead it is, so it knows how much to send
    // next time. Once per block is 375 messages a second at 48 kHz, which is
    // more than anybody needs; every sixteenth block is about 22 ms.
    this.reports = (this.reports ?? 0) + 1;
    if (this.reports % 16 === 0) {
      (this.feed ?? this.port).postMessage({ queued: this.queued, starved: this.starved });
    }

    return this.running;
  }
}

registerProcessor('playfield', PlayfieldProcessor);
