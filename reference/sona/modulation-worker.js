import { renderModulationTraces } from './modulation-render.js';

self.onmessage = ({ data }) => {
  if (data.type !== 'render') return;
  console.info('[Sona] Rendering modulation previews in the background');
  try {
    const result = renderModulationTraces(data.graph, { sampleRate: data.sampleRate, bins: data.bins, nodeIds: data.nodeIds });
    self.postMessage({ type: 'done', requestId: data.requestId, ...result });
  } catch (error) {
    self.postMessage({ type: 'error', requestId: data.requestId, message: error.message || 'Modulation preview failed.' });
  }
};
