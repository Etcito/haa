// Unit tests for the filters ported from the origin engine.
import { describe, expect, it } from 'vitest';
import { Biquad, Smoother } from '../../src/dsp/filters.ts';

describe('dsp/filters', () => {
  it('Biquad lowpass is -3 dB at its cutoff with Q 0.7071', () => {
    const bq = new Biquad();
    bq.set('lowpass', 1000, 0.7071, 0, 48000);
    const db = bq.magnitudeDb(1000, 48000);
    expect(Math.abs(db - -3)).toBeLessThanOrEqual(0.5);
  });

  it('Smoother converges to its target', () => {
    const s = new Smoother(0.01, 48000);
    s.jump(0);
    s.target = 1;
    for (let i = 0; i < 48000; i++) s.next();
    expect(s.value).toBeCloseTo(1, 9);
  });
});
