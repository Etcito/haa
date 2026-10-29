// Exports every Sona preset (reference/sona/presets.js) as a HAA patch file under patches/sona/.
// Run with `npm run export:sona`. Output is deterministic (no timestamps).
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const ID_PATTERN = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const SONA_MASTER = 0.65; // reference/sona/app.js:48
const clamp01 = (v) => Math.min(1, Math.max(0, v));

async function main() {
  console.info = () => {}; // catalog.js runs buildPresets at load, which logs
  const { NODE_TYPES } = await import(new URL('../reference/sona/catalog.js', import.meta.url).href);
  const { buildPresets } = await import(new URL('../reference/sona/presets.js', import.meta.url).href);

  // Same factories as reference/sona/catalog.js:330-338.
  const presetNode = (id, type, x, y, params = {}, label, curve) => {
    const schema = NODE_TYPES[type];
    return {
      id, type, x, y, label: label || schema.name, compact: true,
      params: { ...Object.fromEntries(schema.params.map(parameter => [parameter.key, parameter.default])), ...params },
      ...(curve ? { curve } : {}),
    };
  };
  const connection = (id, from, to, target = 'audio', amount = 1) => ({ id, from, to, target, amount });

  const patches = [];
  for (const preset of buildPresets(presetNode, connection)) {
    const performance = preset.graph.performance;
    const inputs = performance
      ? [{
          name: 'pedal', kind: 'continuous', unit: '', min: 0, max: 1,
          default: clamp01(performance.manualValue), smooth: 0.05, curve: 'lin',
        }]
      : [];
    patches.push({ haa: 1, id: preset.id, title: preset.name, inputs, graph: preset.graph, master: SONA_MASTER });
  }

  const v12 = patches.find((p) => p.id === 'v12');
  if (!v12?.graph.performance) throw new Error('preset v12 has no performance block');
  const live = structuredClone(v12);
  live.id = 'v12-live';
  live.title = 'Italian V12 · live pedal';
  // Sona setPerformanceMode('live'), reference/sona/app.js:377-380.
  const perf = live.graph.performance;
  const value = clamp01(perf.manualValue);
  perf.mode = 'live';
  perf.manualValue = value;
  const node = live.graph.nodes.find((n) => n.id === perf.node);
  if (!node) throw new Error(`v12 performance node ${perf.node} not found`);
  node.params[perf.parameter] = value;
  const edge = live.graph.connections.find((c) => c.id === perf.edge);
  if (edge) edge.amount = 0;
  patches.push(live);

  const outDir = new URL('../patches/sona/', import.meta.url);
  mkdirSync(fileURLToPath(outDir), { recursive: true });
  for (const patch of patches) {
    if (!ID_PATTERN.test(patch.id)) throw new Error(`bad preset id "${patch.id}"`);
    writeFileSync(new URL(`${patch.id}.json`, outDir), JSON.stringify(patch, null, 2) + '\n');
  }
  console.log(`[export-sona] wrote ${patches.length} patches`);
}

main().catch((err) => {
  const reason = String(err instanceof Error ? err.message : err).split('\n')[0];
  process.stderr.write(`[export-sona] ${reason}\n`);
  process.exit(1);
});
