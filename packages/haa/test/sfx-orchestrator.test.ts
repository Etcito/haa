/**
 * The orchestrator's pure parts: cells, the byte-budgeted cache, the load governor.
 * Run with: node --test packages/haa/test/sfx-orchestrator.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';

import { BakeCache, GOVERNOR_START, governorStep, quantize } from '../src/sfx/orchestrator.ts';
import type { HaaInput } from '../src/types.ts';

const DECLS = [
  { name: 'energy', kind: 'continuous', min: 0, max: 1, default: 0.5, smooth: 0, curve: 'lin' },
  { name: 'dim', kind: 'continuous', min: 0.01, max: 10, default: 0.16, smooth: 0, curve: 'log' },
  { name: 'material', kind: 'enum', values: ['wood', 'metal', 'glass'], default: 0 },
] as unknown as HaaInput[];

test('nearby inputs share a cell, energy has 16 steps, a log input is stepped on its log axis, an enum is exact', () => {
  const a = quantize(DECLS, { energy: 0.61, dim: 0.5, material: 1 });
  const b = quantize(DECLS, { energy: 0.63, dim: 0.52, material: 1 });
  assert.equal(a.key, b.key);
  assert.equal(a.inputs.energy, 10 / 16);
  assert.notEqual(quantize(DECLS, { energy: 0.61, dim: 0.5, material: 2 }).key, a.key);
  // dim 0.5 m sits at log position 0.566 of 0.01..10 m: step 5 of 8, rendered at 0.01·1000^(5/8).
  assert.ok(Math.abs(a.inputs.dim - 0.01 * 1000 ** (5 / 8)) < 1e-9);
  assert.equal(a.energy?.cell, 10);
});

test('the cache keeps within its byte budget, evicting the least recently used', () => {
  const entry = () => ({ l: new Float32Array(250), r: new Float32Array(250), rms: 0.1 }); // 2000 bytes
  const c = new BakeCache(5000);
  c.set('a', entry());
  c.set('b', entry());
  c.get('a'); // a is now the most recent
  c.set('c', entry());
  assert.equal(c.bytes, 4000);
  assert.ok(c.has('a') && c.has('c') && !c.has('b'));
  assert.equal(c.evicted, 1);
});

test('the governor steps up on underruns, down after a calm spell, and backs off when a step down fails', () => {
  let g = governorStep(GOVERNOR_START, 1, 5, 0.4);
  assert.equal(g.level, 1);
  g = governorStep(g, 2, 9, 0.4);
  assert.equal(g.level, 2);
  g = governorStep(g, 7, 9, 0.4); // 5 s calm > 4 s
  assert.equal(g.level, 1);
  g = governorStep(g, 8, 12, 0.4); // underruns 1 s after the step down
  assert.equal(g.level, 2);
  assert.equal(g.calmNeeded, 8);
  g = governorStep(g, 14, 12, 0.4); // 6 s calm is no longer enough
  assert.equal(g.level, 2);
  g = governorStep(g, 17, 12, 0.4);
  assert.equal(g.level, 1);
});
