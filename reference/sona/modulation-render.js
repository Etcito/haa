import { GraphDSP, validateGraph } from './dsp.js';
import { CONTROL_TYPES, ModulationRecorder } from './modulation-trace.js';

/** The first pass through the real control graph, with the audio branches omitted. */
export function renderModulationTraces(graph, { sampleRate = 48000, bins = 192, nodeIds } = {}) {
  console.info('[Sona] Calculating modulation output previews');
  const checked = validateGraph(graph);
  const allControls = checked.nodes.filter(node => CONTROL_TYPES.has(node.type));
  const controlIds = new Set(allControls.map(node => node.id));
  const requested = nodeIds === undefined ? controlIds : new Set(nodeIds.filter(id => controlIds.has(id)));
  const ids = new Set(requested);
  // Walk backwards once through the topological order to keep just the inputs
  // required by requested outputs. Unrelated branches are never simulated.
  const incoming = new Map(allControls.map(node => [node.id, []]));
  for (const edge of checked.connections) if (controlIds.has(edge.to)) incoming.get(edge.to).push(edge.from);
  for (const id of [...checked.order].reverse()) if (ids.has(id)) for (const parent of incoming.get(id)) ids.add(parent);
  const nodes = allControls.filter(node => ids.has(node.id));
  // Validation guarantees this subgraph is closed: only controls can modulate
  // a control. Original IDs, order, bases and edges retain exact DSP semantics.
  const controls = { ...checked, nodes, connections: checked.connections.filter(edge => ids.has(edge.to) && ids.has(edge.from)), loop: false };
  const dsp = new GraphDSP(controls, sampleRate);
  if (!nodes.length) return {sampleRate: dsp.sampleRate, duration: checked.duration, traces: []};
  const count = Math.max(2, Math.min(2048, Math.round(Number(bins) || 192)));
  const recorder = new ModulationRecorder(dsp, { bucketFrames: Math.ceil(dsp.durationFrames / count), maxBuckets: count + 2 });
  const buffer = new Float32Array(4096);
  for (let frame = 0; frame < dsp.durationFrames; frame += buffer.length) dsp.process(buffer, buffer, Math.min(buffer.length, dsp.durationFrames - frame));
  const { traces } = recorder.drain({ flush: true });
  return { sampleRate: dsp.sampleRate, duration: checked.duration, traces: traces.filter(trace => requested.has(trace.id)) };
}
