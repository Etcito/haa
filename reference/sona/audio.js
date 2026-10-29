import { GraphDSP, validateGraph } from './dsp.js';
import { ModulationRecorder } from './modulation-trace.js';

export class AudioEngine {
  constructor({ onTime = () => {}, onEnded = () => {}, onError = () => {}, onModulation = () => {}, onModulationReset = () => {} } = {}) {
    console.info('[SonaLab] Initialisation du moteur audio');
    this.onTime = onTime; this.onEnded = onEnded; this.onError = onError;
    this.onModulation = onModulation; this.onModulationReset = onModulationReset; this._modulationSequence = 0;
    this.context = null; this.analyser = null; this.masterNode = null; this.worklet = null;
    this._fallback = null; this._master = .65; this._isPlaying = false;
    this._serial = 0; this._graph = null; this._moduleReady = null;
  }
  get isPlaying() { return this._isPlaying; }
  getAnalyser() { return this.analyser; }
  async start(graph, { offset = 0 } = {}) {
    console.info('[SonaLab] Lecture du graphe');
    const checked = validateGraph(graph);
    this.stop();
    const serial = this._serial;
    this._graph = checked;
    const position = Math.min(checked.duration - 1e-6, Math.max(0, Number.isFinite(Number(offset)) ? Number(offset) : 0));
    try {
      const AudioContextClass = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (!AudioContextClass) throw new Error('Ce navigateur ne prend pas en charge Web Audio.');
      if (!this.context || this.context.state === 'closed') {
        this.context = new AudioContextClass({ latencyHint: 'interactive' });
        this.analyser = null; this.masterNode = null; this._moduleReady = null;
      }
      // Each part is initialized separately so a failed attempt remains recoverable.
      if (!this.analyser) {
        this.analyser = this.context.createAnalyser();
        this.analyser.fftSize = 2048; this.analyser.smoothingTimeConstant = .82;
        this.analyser.connect(this.context.destination);
      }
      if (!this.masterNode) {
        this.masterNode = this.context.createGain(); this.masterNode.gain.value = this._master;
        this.masterNode.connect(this.analyser);
      }
      if (!this._moduleReady) {
        this._moduleReady = this.context.audioWorklet && typeof globalThis.AudioWorkletNode === 'function'
          ? this.context.audioWorklet.addModule(new URL('./audio-worklet.js', import.meta.url).href).then(() => true).catch(error => {
            console.warn('[SonaLab] AudioWorklet indisponible, activation du moteur compatible.', error.message);
            return false;
          })
          : Promise.resolve(false);
      }
      await this.context.resume();
      const hasWorklet = await this._moduleReady;
      if (serial !== this._serial) return false;
      const sequence = this._resetModulation('start', this._graph);
      let node = null;
      if (hasWorklet) {
        try {
          node = new AudioWorkletNode(this.context, 'sonalab-graph', {
            numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2],
            processorOptions: { graph: this._graph, offset: position, sequence },
          });
        } catch (error) { console.warn('[SonaLab] Activation du moteur audio compatible.', error.message); }
      }
      if (node) {
        node.port.onmessage = ({ data }) => {
          if (this.worklet !== node) return;
          if (data.type === 'time') this.onTime(data.seconds);
          else if (data.type === 'modulation') {
            if (data.sequence === this._modulationSequence) this.onModulation(data);
          }
          else if (data.type === 'ended') {
            this._isPlaying = false; node.disconnect(); this.worklet = null; this._resetModulation('stop'); this.onEnded();
          } else if (data.type === 'error') { this.stop(); this.onError(new Error(data.message)); }
        };
        node.onprocessorerror = () => {
          if (this.worklet !== node) return;
          this.stop(); this.onError(new Error('Le moteur audio a rencontré une erreur. Relancez la lecture.'));
        };
        this.worklet = node;
        node.connect(this.masterNode);
      } else this._startFallback(this._graph, position);
      this._isPlaying = true;
      this.onTime(position);
      return true;
    } catch (error) {
      this._isPlaying = false;
      if (serial === this._serial) this._resetModulation('stop');
      this.onError(error);
      throw error;
    }
  }
  _startFallback(graph, offset) {
    console.info('[SonaLab] Moteur audio compatible activé');
    if (typeof this.context.createScriptProcessor !== 'function') throw new Error('Le navigateur ne dispose pas de moteur audio compatible.');
    const node = this.context.createScriptProcessor(1024, 0, 2);
    const dsp = new GraphDSP(graph, this.context.sampleRate);
    dsp.setPosition(offset);
    const modulation = new ModulationRecorder(dsp, { sequence: this._modulationSequence });
    const state = { node, dsp, modulation, reportFrames: 0, stopping: -1, closed: false, fadeFrames: Math.round(this.context.sampleRate * .012) };
    this._fallback = state;
    node.onaudioprocess = event => {
      const left = event.outputBuffer.getChannelData(0), right = event.outputBuffer.getChannelData(1);
      if (state.closed) { left.fill(0); right.fill(0); return; }
      try {
        dsp.process(left, right, left.length);
        if (state.stopping >= 0) {
          for (let i = 0; i < left.length; i++) {
            const gain = Math.max(0, state.stopping-- / state.fadeFrames);
            left[i] *= gain; right[i] *= gain;
          }
          if (state.stopping <= 0) state.closed = true;
          return;
        }
        if (this._fallback !== state) return;
        state.reportFrames += left.length;
        if (state.reportFrames >= this.context.sampleRate / 25 || dsp.ended) {
          state.reportFrames = 0;
          this.onTime(dsp.position);
          const packet = modulation.drain({ flush: dsp.ended });
          if (packet.sequence === this._modulationSequence) this.onModulation(packet);
        }
        if (dsp.ended) {
          state.closed = true; this._fallback = null; this._isPlaying = false;
          setTimeout(() => { node.disconnect(); node.onaudioprocess = null; }, 50);
          this._resetModulation('stop');
          this.onEnded();
        }
      } catch (error) {
        left.fill(0); right.fill(0); state.closed = true;
        node.disconnect(); node.onaudioprocess = null;
        if (this._fallback === state) { this._fallback = null; this._isPlaying = false; this._resetModulation('stop'); this.onError(error); }
      }
    };
    node.connect(this.masterNode);
  }
  stop() {
    console.info('[SonaLab] Arrêt de la lecture');
    this._serial++; this._isPlaying = false;
    this._resetModulation('stop');
    const node = this.worklet; this.worklet = null;
    if (node) {
      node.port.onmessage = null; node.onprocessorerror = null;
      node.port.postMessage({ type: 'stop' });
      setTimeout(() => { node.disconnect(); node.port.close(); }, 25);
    }
    const fallback = this._fallback; this._fallback = null;
    if (fallback) {
      fallback.stopping = fallback.fadeFrames;
      setTimeout(() => {
        fallback.closed = true; fallback.node.disconnect(); fallback.node.onaudioprocess = null;
      }, 80);
    }
  }
  setGraph(graph) {
    const checked = validateGraph(graph);
    this._graph = checked;
    if (!this.worklet && !this._fallback) return;
    const sequence = this._resetModulation('graph', checked);
    if (this.worklet) this.worklet.port.postMessage({ type: 'graph', graph: checked, sequence });
    if (this._fallback) {
      this._fallback.dsp.setGraph(checked, true);
      this._fallback.modulation.sequence = sequence;
    }
  }
  _resetModulation(reason, graph = this._graph) {
    console.info('[SonaLab] Resetting modulation observation', reason);
    const sequence = ++this._modulationSequence;
    this.onModulationReset({ reason, sequence, graph, duration: graph?.duration || 0, sampleRate: this.context?.sampleRate || 48000 });
    return sequence;
  }
  setMaster(value) {
    this._master = Math.min(1, Math.max(0, Number(value) || 0));
    if (this.masterNode) this.masterNode.gain.setTargetAtTime(this._master, this.context.currentTime, .012);
  }
  async renderWav(graph, { duration = graph.duration, master = this._master, onProgress } = {}) {
    console.info('[SonaLab] Préparation de l’export WAV', { duration });
    const checked = validateGraph({ ...graph, duration, loop: false });
    return new Promise((resolve, reject) => {
      const worker = new Worker(new URL('./render-worker.js', import.meta.url), { type: 'module' });
      const timer = setTimeout(() => { worker.terminate(); reject(new Error('Le rendu audio a pris trop de temps. Réduisez la durée ou le nombre de nœuds.')); }, 120000);
      const finish = () => { clearTimeout(timer); worker.terminate(); };
      worker.onmessage = ({ data }) => {
        if (data.type === 'done') { finish(); resolve(new Blob([data.buffer], { type: 'audio/wav' })); }
        else if (data.type === 'error') { finish(); reject(new Error(data.message)); }
        else if (data.type === 'progress' && onProgress) onProgress(data.progress);
      };
      worker.onerror = event => { finish(); reject(new Error(event.message || 'Le navigateur n’a pas pu lancer le rendu audio.')); };
      worker.postMessage({ type: 'render', graph: checked, duration: checked.duration, master });
    });
  }
}
