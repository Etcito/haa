import { GraphDSP } from './dsp.js';
import { ModulationRecorder } from './modulation-trace.js';

class SonaGraphProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.dsp = new GraphDSP(options.processorOptions.graph, sampleRate);
    this.dsp.setPosition(options.processorOptions.offset || 0);
    this.modulation = new ModulationRecorder(this.dsp, { sequence: options.processorOptions.sequence || 0 });
    this.reportFrames = 0;
    this.stopping = -1;
    this.finished = false;
    this.port.onmessage = ({ data }) => {
      try {
        if (data.type === 'graph') {
          this.dsp.setGraph(data.graph, true);
          this.modulation.sequence = data.sequence;
        }
        else if (data.type === 'stop') this.stopping = Math.round(sampleRate * .012);
      } catch (error) {
        this.port.postMessage({ type: 'error', message: error.message });
      }
    };
  }
  process(_inputs, outputs) {
    if (this.finished) return false;
    const channels = outputs[0];
    if (!channels?.length) return true;
    const left = channels[0], right = channels[1] || left;
    this.dsp.process(left, right, left.length);
    if (this.stopping >= 0) {
      for (let i = 0; i < left.length; i++) {
        const gain = Math.max(0, this.stopping-- / (sampleRate * .012));
        left[i] *= gain;
        if (right !== left) right[i] *= gain;
      }
      if (this.stopping <= 0) { this.finished = true; return false; }
    }
    this.reportFrames += left.length;
    if (this.reportFrames >= sampleRate / 25 || this.dsp.ended) {
      this.reportFrames = 0;
      this.port.postMessage({ type: 'time', seconds: this.dsp.position });
      this.port.postMessage({ type: 'modulation', ...this.modulation.drain({ flush: this.dsp.ended }) });
    }
    if (this.dsp.ended) {
      this.finished = true;
      this.port.postMessage({ type: 'ended' });
      return false;
    }
    return true;
  }
}
registerProcessor('sonalab-graph', SonaGraphProcessor);
