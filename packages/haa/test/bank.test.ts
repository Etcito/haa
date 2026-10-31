/**
 * Bank tests against the compiled wasm engine: variant seeds, deterministic variants
 * and the aggregate analysis of a render. Run with:
 *   node --test packages/haa/test/bank.test.ts
 */
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { readFile } from 'node:fs/promises';

import type { HaaPatch } from '../src/types.ts';
import { analyse, renderBank, variantSeed } from '../src/bank.ts';

const module = await WebAssembly.compile(await readFile(new URL('../pkg/haa.wasm', import.meta.url)));

/** A minimal HAA 0.2.0 patch: one seeded white-noise source into the output. */
const PATCH: HaaPatch = {
  haa: 1,
  id: 'bank-test-noise',
  title: 'Bank test noise',
  inputs: [],
  master: 1,
  graph: {
    duration: 1,
    loop: true,
    mode: 'timeline',
    nodes: [
      {
        id: 'noise',
        type: 'noise',
        x: 40,
        y: 40,
        label: 'Noise',
        params: { color: 'white', level: 0.4, width: 0.5 },
      },
      {
        id: 'out',
        type: 'output',
        x: 340,
        y: 40,
        label: 'Output',
        params: { gain: 0.9 },
      },
    ],
    connections: [{ id: 'noise-out', from: 'noise', to: 'out', target: 'audio' }],
  },
};

/** The FNV-1a 32 reference over the UTF-8 bytes of `${id}#${variant}`. */
function referenceSeed(id: string, variant: number): number {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(`${id}#${variant}`)) {
    hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

/** True when two renders hold the same samples on both channels. */
function sameSamples(a: Float32Array, b: Float32Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

test('variantSeed matches the FNV-1a 32 reference over UTF-8 bytes', () => {
  for (const [id, variant] of [['bank-test-noise', 0], ['a', 1], ['sound.é', 7]] as const) {
    assert.equal(variantSeed(id, variant), referenceSeed(id, variant), `${id}#${variant}`);
  }
  assert.equal(variantSeed('bank-test-noise', 0), variantSeed('bank-test-noise', 0));
});

test('variants 0 and 1 render different samples', () => {
  const [zero, one] = renderBank(module, PATCH, [{ variant: 0, seconds: 0.25 }, { variant: 1, seconds: 0.25 }]);
  assert.equal(zero.id, PATCH.id);
  assert.equal(zero.variant, 0);
  assert.equal(one.seed, referenceSeed(PATCH.id, 1));
  assert.notEqual(zero.seed, one.seed);
  assert.equal(sameSamples(zero.render.left, one.render.left), false);
});

test('variant 0 rendered twice is identical, from an object or a JSON string', () => {
  const entries = [{ variant: 0, seconds: 0.25 }];
  const first = renderBank(module, PATCH, entries)[0];
  const second = renderBank(module, PATCH, entries)[0];
  const fromString = renderBank(module, JSON.stringify(PATCH), entries)[0];
  assert.equal(sameSamples(first.render.left, second.render.left), true);
  assert.equal(sameSamples(first.render.right, second.render.right), true);
  assert.equal(sameSamples(first.render.left, fromString.render.left), true);
  assert.equal(first.render.sampleRate, 48_000);
  assert.equal(first.render.left.length, 12_000);
});

test('analyse of a rendered variant is audible with no non-finite samples', () => {
  const [entry] = renderBank(module, PATCH, [{ variant: 0, seconds: 0.5 }]);
  const stats = analyse(entry.render);
  assert.equal(stats.frames, 24_000);
  assert.equal(stats.sampleRate, 48_000);
  assert.equal(stats.nonFinite, 0);
  assert.equal(stats.silent, false);
  assert.ok(Number.isFinite(stats.peakDbfs));
  assert.ok(stats.peakDbfs < 0);
});
