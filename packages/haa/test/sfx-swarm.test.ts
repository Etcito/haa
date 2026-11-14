/**
 * Host test for the swarm grouping of @etcito/haa/sfx (src/sfx/swarm.ts): which impacts keep their
 * own voice and which feed the cluster voice, and the cluster's inputs.
 *
 *   node --test packages/haa/test/sfx-swarm.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import { v3 } from '../src/sfx/acoustics.ts';
import { DENSITY_FULL } from '../src/sfx/contract.ts';
import { SwarmTracker } from '../src/sfx/swarm.ts';

test('a burst keeps the first three impacts (and a louder one) as voices and clusters the rest', () => {
  const sw = new SwarmTracker();
  const routes = [];
  // 40 pebbles in 1 s within a metre, energy 0.4; impact 20 is louder.
  for (let i = 0; i < 40; i++) {
    const energy = i === 20 ? 0.8 : 0.4;
    routes.push(sw.route('particle-cluster', v3((i % 5) * 0.2, 0, 0), { energy, size: 0.15, hardness: 0.9 }, i * 0.025));
  }
  const own = routes.map((r, i) => (r.own ? i : -1)).filter((i) => i >= 0);
  assert.deepEqual(own.slice(0, 3), [0, 1, 2]);
  assert.ok(own.includes(20), `the loud impact keeps its voice: ${own}`);
  // The window (0.3 s) renews the three own voices, so about one in four impacts keeps a voice.
  assert.ok(own.length <= 14, `own ${own.length}`);
  assert.equal(sw.swarms.length, 1);
  const s = sw.swarms[0];
  assert.equal(s.clustered, 40 - own.length);
  // Rate: ~30 clustered impacts per second; the means come from the clustered impacts only.
  const inputs = sw.inputs(s, 39 * 0.025);
  const rate = inputs.density * DENSITY_FULL;
  assert.ok(rate > 15 && rate < 60, `rate ${rate}`);
  assert.ok(Math.abs(inputs.energy - 0.4) < 0.02, `energy ${inputs.energy}`);
  assert.ok(Math.abs(inputs.hardness - 0.9) < 1e-9 && Math.abs(inputs.size - 0.15) < 1e-9);
  // The cluster voice falls quiet 0.15 s after the last clustered impact; the swarm is then forgotten.
  assert.equal(sw.quiet(s, s.lastClustered + 0.1), false);
  assert.equal(sw.quiet(s, s.lastClustered + 0.2), true);
  assert.equal(sw.prune(1.6).length, 1);
  assert.equal(sw.swarms.length, 0);
});

test('regions and cluster presets are separate swarms', () => {
  const sw = new SwarmTracker();
  for (let i = 0; i < 6; i++) {
    sw.route('particle-cluster', v3(0, 0, 0), { energy: 0.4, size: 0.1, hardness: 0.9 }, i * 0.01);
    sw.route('particle-cluster', v3(10, 0, 0), { energy: 0.4, size: 0.1, hardness: 0.9 }, i * 0.01);
    sw.route('glass-cluster', v3(0, 0, 0), { energy: 0.4, size: 0.1, hardness: 0.9 }, i * 0.01);
  }
  assert.equal(sw.swarms.length, 3);
  for (const s of sw.swarms) assert.equal(s.clustered, 3);
});
