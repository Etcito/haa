// Unit tests for the scalar math helpers ported from the origin engine.
import { describe, expect, it } from 'vitest';
import { clamp, dbToGain, gainToDb } from '../../src/dsp/math.ts';

describe('dsp/math', () => {
  it('clamp resolves NaN to lo', () => {
    expect(clamp(NaN, -2, 3)).toBe(-2);
  });

  it('gainToDb floors silence at -200', () => {
    expect(gainToDb(0)).toBe(-200);
  });

  it('dbToGain inverts gainToDb', () => {
    expect(dbToGain(gainToDb(0.5))).toBeCloseTo(0.5, 12);
  });
});
