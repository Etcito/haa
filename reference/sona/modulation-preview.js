import { validateGraph } from './dsp.js';
import { NODE_TYPES } from './catalog.js';

const isControl = node => NODE_TYPES[node.type]?.category === 'control';

// Normalize both authored and audio-engine graphs before comparing them. Labels,
// layout and audio-only edits do not change a modulation signal.
export function modulationGraphKey(graph) {
  const checked = validateGraph(graph);
  const controls = checked.nodes.filter(isControl);
  const ids = new Set(controls.map(node => node.id));
  return JSON.stringify({
    duration: checked.duration, loop: checked.loop, mode: checked.performance?.mode === 'live' ? 'live' : 'curve',
    nodes: controls.map(node => ({id: node.id, type: node.type, params: node.params, muted: !!node.muted,
      ...(node.type === 'curve' ? {curve: node.curve} : {})})),
    connections: checked.connections.filter(edge => ids.has(edge.to)).map(edge => ({from: edge.from, to: edge.to, target: edge.target, amount: edge.amount})),
  });
}

// Flat ancestry lists avoid repeatedly embedding a parent's complete key.
// Each local signature appears at most once in an output's dependency key.
function modulationState(graph, sampleRate) {
  const checked = validateGraph(graph);
  const controls = checked.nodes.filter(isControl), ids = new Set(controls.map(node => node.id));
  const incoming = new Map(controls.map(node => [node.id, []]));
  for (const edge of checked.connections) if (ids.has(edge.to)) incoming.get(edge.to).push(edge);
  const local = new Map(controls.map(node => [node.id, JSON.stringify({
    type: node.type, params: node.params, muted: !!node.muted,
    ...(node.type === 'curve' ? {curve: node.curve} : {}),
    inputs: incoming.get(node.id).map(edge => [edge.from, edge.target, edge.amount]),
  })]));
  const ancestors = new Map();
  for (const id of checked.order) {
    if (!ids.has(id)) continue;
    const upstream = new Set([id]);
    for (const edge of incoming.get(id)) for (const parent of ancestors.get(edge.from)) upstream.add(parent);
    ancestors.set(id, upstream);
  }
  const shared = JSON.stringify([checked.duration, checked.loop, checked.performance?.mode === 'live', sampleRate]);
  const keys = new Map(controls.map(node => [node.id, shared + JSON.stringify(
    [...ancestors.get(node.id)].sort().map(id => [id, local.get(id)]),
  )]));
  return {graph: checked, key: modulationGraphKey(checked), keys};
}

/** Cached first-pass previews and a short history of measured playback values. */
export class ModulationPreviews {
  constructor({onChange = () => {}, makeWorker = () => new Worker(new URL('./modulation-worker.js', import.meta.url), {type: 'module'}),
    schedule = (callback, delay) => setTimeout(callback, delay), cancel = timer => clearTimeout(timer)} = {}) {
    console.info('[Sona] Initializing measured modulation previews');
    this.onChange = onChange; this.makeWorker = makeWorker;
    this.schedule = schedule; this.cancel = cancel;
    this.graph = null; this.key = ''; this.sampleRate = 48000;
    this.previews = new Map(); this.previewKeys = new Map(); this.nodeKeys = new Map();
    this.statuses = new Map(); this.dirty = new Set();
    this.worker = null; this.job = null; this.timer = null; this.timeout = null; this.requestId = 0;
    this.running = false; this.liveKey = ''; this.sequence = -1;
    this.live = new Map(); this.liveKeys = new Map(); this.liveTimes = new Map(); this.liveTime = 0;
  }

  setGraph(graph) {
    const state = modulationState(graph, this.sampleRate);
    const changed = [...new Set([...this.nodeKeys.keys(), ...state.keys.keys()])]
      .filter(id => this.nodeKeys.get(id) !== state.keys.get(id));
    this.graph = state.graph; this.key = state.key; this.nodeKeys = state.keys;
    if (!changed.length) return;
    console.info('[Sona] Updating affected modulation outputs', changed);
    for (const id of changed) {
      if (!this.nodeKeys.has(id)) {
        this.previews.delete(id); this.previewKeys.delete(id); this.statuses.delete(id); this.dirty.delete(id);
        this.live.delete(id); this.liveTimes.delete(id);
      } else if (this.previewKeys.get(id) === this.nodeKeys.get(id)) {
        this.statuses.set(id, 'ready'); this.dirty.delete(id);
      } else {
        this.statuses.set(id, 'pending'); this.dirty.add(id);
      }
    }
    // Preserve an unrelated pending branch. Cancel only a job whose inputs
    // changed; its unresolved IDs remain dirty and join the next request.
    if (this.timer !== null || this.job?.nodeIds.some(id => this.job.keys.get(id) !== this.nodeKeys.get(id))) this.cancelRender();
    this.onChange(changed);
    this.queueRender();
  }

  cancelRender() {
    this.requestId++;
    if (this.timer !== null) this.cancel(this.timer);
    if (this.timeout !== null) this.cancel(this.timeout);
    this.timer = null; this.timeout = null;
    if (this.worker) this.worker.terminate();
    this.worker = null; this.job = null;
  }

  // Scheduling and rendering helpers can be called during an edit gesture.
  liveMatches(id) {
    return this.running && this.nodeKeys.has(id) && this.liveKeys.get(id) === this.nodeKeys.get(id);
  }

  renderIds() {
    return [...this.dirty].filter(id => this.nodeKeys.has(id) && !this.liveMatches(id) && this.statuses.get(id) !== 'unavailable');
  }

  queueRender() {
    if (!this.graph || this.timer !== null || this.worker || !this.renderIds().length) return;
    this.timer = this.schedule(() => { this.timer = null; this.render(); }, 100);
  }

  render() {
    const nodeIds = this.renderIds();
    if (!nodeIds.length) return;
    console.info('[Sona] Calculating affected modulation outputs', nodeIds);
    const requestId = ++this.requestId, sampleRate = this.sampleRate;
    const job = {requestId, nodeIds, keys: new Map(nodeIds.map(id => [id, this.nodeKeys.get(id)]))};
    this.job = job;
    let worker;
    const finish = () => {
      if (this.job !== job) return;
      if (this.timeout !== null) this.cancel(this.timeout);
      this.timeout = null;
      worker?.terminate();
      this.worker = null; this.job = null;
    };
    const failed = error => {
      if (this.job !== job || requestId !== this.requestId) return;
      console.warn('[Sona] Modulation preview unavailable', error?.message || error);
      const affected = nodeIds.filter(id => this.nodeKeys.get(id) === job.keys.get(id));
      for (const id of affected) { this.statuses.set(id, 'unavailable'); this.dirty.delete(id); }
      finish();
      if (affected.length) this.onChange(affected);
      this.queueRender();
    };
    try {
      worker = this.makeWorker(); this.worker = worker;
      worker.onmessage = ({data}) => {
        if (this.job !== job || requestId !== this.requestId || data.requestId !== requestId) return;
        if (data.type === 'error') { failed(data.message); return; }
        if (data.type !== 'done') return;
        const received = new Map(data.traces.map(trace => [trace.id, trace.points]));
        const affected = nodeIds.filter(id => this.nodeKeys.get(id) === job.keys.get(id));
        for (const id of affected) {
          const points = received.get(id);
          if (points?.length) {
            this.previews.set(id, points); this.previewKeys.set(id, job.keys.get(id)); this.statuses.set(id, 'ready');
          } else this.statuses.set(id, 'unavailable');
          this.dirty.delete(id);
        }
        finish();
        if (affected.length) this.onChange(affected);
        this.queueRender();
      };
      worker.onerror = failed;
      this.timeout = this.schedule(() => failed('Preview rendering timed out.'), 30000);
      worker.postMessage({type: 'render', requestId, graph: this.graph, sampleRate, bins: 192, nodeIds});
    } catch (error) { failed(error); }
  }

  resetAudio({reason, sequence, graph, sampleRate}) {
    console.info('[Sona] Modulation playback state', {reason, sequence});
    this.sequence = sequence;
    const rateChanged = Number.isFinite(sampleRate) && sampleRate !== this.sampleRate;
    if (rateChanged) {
      this.sampleRate = sampleRate;
      if (this.graph) this.setGraph(this.graph);
    }
    if (reason === 'stop') { this.endPlayback(); return; }
    const previous = this.liveKeys, wasRunning = this.running;
    const resetAll = reason === 'start' || !wasRunning || rateChanged;
    const state = modulationState(graph, this.sampleRate);
    if (resetAll) { this.live.clear(); this.liveTimes.clear(); this.liveTime = 0; }
    this.running = true; this.liveKey = state.key; this.liveKeys = state.keys;
    for (const id of this.live.keys()) if (!state.keys.has(id)) { this.live.delete(id); this.liveTimes.delete(id); }
    const affected = [...this.nodeKeys.keys()].filter(id => resetAll ||
      (previous.get(id) !== state.keys.get(id) &&
        ((wasRunning && previous.get(id) === this.nodeKeys.get(id)) || this.liveMatches(id))));
    if (this.job?.nodeIds.some(id => this.liveMatches(id)) || (this.timer !== null && !this.renderIds().length)) this.cancelRender();
    this.queueRender(); // Start & drive may play another graph.
    if (affected.length) this.onChange(affected);
  }

  endPlayback() {
    console.info('[Sona] Returning to the full-duration modulation preview');
    this.running = false; this.liveKey = ''; this.liveKeys.clear(); this.live.clear(); this.liveTimes.clear(); this.liveTime = 0;
    this.queueRender();
    const affected = [...this.nodeKeys.keys()];
    if (affected.length) this.onChange(affected);
  }

  // Called by the playback report loop. Never log or rerun DSP here.
  accept(packet) {
    if (!this.running || packet.sequence !== this.sequence) return;
    if (!Number.isFinite(packet.time)) return;
    this.liveTime = packet.time;
    const start = Math.max(0, this.liveTime - 4);
    const affected = [];
    for (const trace of packet.traces) {
      if (!this.liveMatches(trace.id) || !trace.points.length) continue;
      const points = packet.time < (this.liveTimes.get(trace.id) || 0) ? [] : this.live.get(trace.id) || [];
      for (const point of trace.points) points.push(point);
      let first = 0;
      while (first < points.length - 1 && points[first + 1][0] < start) first++;
      if (first) points.splice(0, first);
      // A bounded history even if a future capture format has finer buckets.
      if (points.length > 2048) points.splice(0, points.length - 2048);
      this.live.set(trace.id, points);
      this.liveTimes.set(trace.id, packet.time);
      affected.push(trace.id);
    }
    if (affected.length) this.onChange(affected);
  }

  // Read from the card/inspector render loop; does not mutate authored data.
  frame(id) {
    if (this.liveMatches(id)) {
      const points = this.live.get(id) || [];
      const time = this.liveTimes.get(id) || 0;
      return {mode: points.length ? 'live' : 'waiting', points,
        start: Math.max(0, time - 4), end: Math.max(4, time),
        current: points.at(-1)?.[3], sampleRate: this.sampleRate};
    }
    const points = this.previewKeys.get(id) === this.nodeKeys.get(id) ? this.previews.get(id) || [] : [];
    return {mode: points.length ? 'preview' : this.statuses.get(id) === 'unavailable' ? 'unavailable' : 'pending',
      points, start: 0, end: this.graph?.duration || 8, sampleRate: this.sampleRate};
  }
}
