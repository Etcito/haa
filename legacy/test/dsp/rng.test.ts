// Unit tests for seed hashing and the mulberry32 audio stream ported from the origin engine.
import { describe, expect, it } from 'vitest';
import { AudioRng, hashSeed } from '../../src/dsp/rng.ts';

function draw(seed: number, n: number): number[] {
  const rng = new AudioRng(seed);
  const out: number[] = [];
  for (let i = 0; i < n; i++) out.push(rng.next());
  return out;
}

describe('dsp/rng', () => {
  it('hashSeed is pinned', () => {
    expect(hashSeed('a', 1)).toBe(247438432);
  });

  it('AudioRng sequences repeat per seed and differ across seeds', () => {
    const a = draw(1234, 100);
    expect(draw(1234, 100)).toEqual(a);
    expect(draw(1235, 100)).not.toEqual(a);
  });
});
