/** Shared, bounded observation of the values already calculated by GraphDSP. */
export const CONTROL_TYPES = new Set(['curve', 'response', 'motion', 'lfo', 'envelope', 'random', 'math']);

export class ModulationRecorder {
  constructor(dsp, { bucketFrames = Math.round(dsp.sampleRate / 120), sequence = 0, maxBuckets = 256 } = {}) {
    console.info('[Sona] Creating modulation output recorder');
    this.bucketFrames = Math.max(1, Math.round(Number(bucketFrames) || 1));
    this.capacity = Math.max(2, Math.min(4096, Math.round(Number(maxBuckets) || 256)));
    this.sequence = sequence;
    this.bind(dsp, sequence);
  }

  /** Reindex after a graph edit. Only observation buffers are reset. */
  bind(dsp, sequence = this.sequence) {
    console.info('[Sona] Binding modulation output recorder');
    this.sampleRate = dsp.sampleRate;
    this.sequence = sequence;
    const nodes = dsp.compiled.map((node, index) => ({ node, index })).filter(({ node }) => CONTROL_TYPES.has(node.type));
    this.ids = nodes.map(({ node }) => node.id);
    this.indices = new Uint16Array(nodes.map(({ index }) => index));
    this.minimum = new Float64Array(nodes.length);
    this.maximum = new Float64Array(nodes.length);
    this.last = new Float64Array(nodes.length);
    this.times = new Float64Array(this.capacity);
    this.minima = new Float64Array(this.capacity * nodes.length);
    this.maxima = new Float64Array(this.capacity * nodes.length);
    this.values = new Float64Array(this.capacity * nodes.length);
    this.read = 0; this.pending = 0; this.samples = 0; this.first = true;
    this.lastFrame = dsp.totalFrames - 1;
    dsp.modulationRecorder = this;
  }

  // Called for every real DSP sample, after all control dependencies resolve.
  // Preallocated buffers keep this path free of allocations and logging.
  capture(values, frame) {
    if (!this.indices.length) return;
    for (let i = 0; i < this.indices.length; i++) {
      const value = values[this.indices[i]];
      this.last[i] = value;
      if (!this.samples) this.minimum[i] = this.maximum[i] = value;
      else {
        if (value < this.minimum[i]) this.minimum[i] = value;
        if (value > this.maximum[i]) this.maximum[i] = value;
      }
    }
    this.lastFrame = frame;
    // An exact first point anchors the left edge without inventing a zero.
    if (this.first && this.bucketFrames > 1) this.store(frame);
    this.first = false;
    this.samples++;
    if (this.samples >= this.bucketFrames) {
      this.store(frame);
      this.samples = 0;
    }
  }

  // Called from capture; preserves each bucket's extrema, including spikes.
  store(frame) {
    if (this.pending === this.capacity) {
      this.read = (this.read + 1) % this.capacity;
      this.pending--;
    }
    const slot = (this.read + this.pending) % this.capacity;
    this.times[slot] = frame / this.sampleRate;
    const offset = slot * this.indices.length;
    for (let i = 0; i < this.indices.length; i++) {
      this.minima[offset + i] = this.minimum[i];
      this.maxima[offset + i] = this.maximum[i];
      this.values[offset + i] = this.last[i];
    }
    this.pending++;
  }

  // Called at the UI reporting cadence, never from the per-sample loop.
  // Point timestamps are the last actual sample in the bucket, not a centre.
  drain({ flush = false } = {}) {
    if (flush && this.samples) { this.store(this.lastFrame); this.samples = 0; }
    const traces = this.ids.map((id, i) => {
      const points = [];
      for (let bucket = 0; bucket < this.pending; bucket++) {
        const slot = (this.read + bucket) % this.capacity;
        const offset = slot * this.indices.length + i;
        points.push([this.times[slot], this.minima[offset], this.maxima[offset], this.values[offset]]);
      }
      return { id, points };
    });
    this.read = (this.read + this.pending) % this.capacity;
    this.pending = 0;
    return { sampleRate: this.sampleRate, time: (this.lastFrame + 1) / this.sampleRate, sequence: this.sequence, traces };
  }
}
