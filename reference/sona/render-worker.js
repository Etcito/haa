import { GraphDSP, validateGraph } from './dsp.js';

function writeText(view, offset, text) {
  for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
}

self.onmessage = ({ data }) => {
  if (data.type !== 'render') return;
  console.info('[SonaLab] Export WAV en arrière-plan');
  try {
    const sampleRate = 48000;
    const graph = validateGraph({ ...data.graph, duration: data.duration ?? data.graph.duration, loop: false });
    const master = Math.min(1, Math.max(0, Number.isFinite(Number(data.master)) ? Number(data.master) : .65));
    const length = Math.ceil(graph.duration * sampleRate), channels = 2, bytesPerSample = 2;
    const buffer = new ArrayBuffer(44 + length * channels * bytesPerSample), view = new DataView(buffer);
    writeText(view, 0, 'RIFF'); view.setUint32(4, buffer.byteLength - 8, true); writeText(view, 8, 'WAVE');
    writeText(view, 12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, channels, true);
    view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * channels * bytesPerSample, true);
    view.setUint16(32, channels * bytesPerSample, true); view.setUint16(34, 16, true); writeText(view, 36, 'data');
    view.setUint32(40, length * channels * bytesPerSample, true);
    const dsp = new GraphDSP(graph, sampleRate), left = new Float32Array(4096), right = new Float32Array(4096);
    let offset = 44, nextReport = .1;
    for (let frame = 0; frame < length; frame += left.length) {
      const count = Math.min(left.length, length - frame);
      dsp.process(left, right, count);
      for (let i = 0; i < count; i++) {
        const l = Math.max(-1, Math.min(1, left[i] * master)), r = Math.max(-1, Math.min(1, right[i] * master));
        view.setInt16(offset, Math.round(l * (l < 0 ? 32768 : 32767)), true);
        view.setInt16(offset + 2, Math.round(r * (r < 0 ? 32768 : 32767)), true);
        offset += 4;
      }
      if (frame / length >= nextReport) { self.postMessage({ type: 'progress', progress: frame / length }); nextReport += .1; }
    }
    self.postMessage({ type: 'done', buffer, sampleRate, channels, duration: graph.duration }, [buffer]);
  } catch (error) { self.postMessage({ type: 'error', message: error.message || 'Échec du rendu audio.' }); }
};
