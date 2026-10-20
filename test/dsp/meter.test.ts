// Unit tests for the BS.1770 loudness meter and simple measurements ported from the origin engine.
import { describe, expect, it } from 'vitest';
import { countNonFinite, LoudnessMeter, samplePeakDb } from '../../src/dsp/meter.ts';

const SR = 48000;

function sine(peakDb: number, hz: number, seconds: number): Float32Array {
  const amp = Math.pow(10, peakDb / 20);
  const x = new Float32Array(Math.round(seconds * SR));
  for (let i = 0; i < x.length; i++) x[i] = amp * Math.sin((2 * Math.PI * hz * i) / SR);
  return x;
}

describe('dsp/meter', () => {
  const tone = sine(-20, 1000, 5);

  it('measures a stereo 1 kHz sine at -20 dBFS peak as -20 LUFS integrated', () => {
    const meter = new LoudnessMeter(SR);
    meter.push(tone, tone, tone.length);
    expect(Math.abs(meter.integrated() - -20)).toBeLessThanOrEqual(0.1);
  });

  it('measures the sample peak of the sine', () => {
    expect(Math.abs(samplePeakDb(tone, tone) - -20)).toBeLessThanOrEqual(0.01);
  });

  it('counts injected NaN and Infinity samples', () => {
    const x = new Float32Array(64);
    expect(countNonFinite(x)).toBe(0);
    x[3] = Number.NaN;
    x[10] = Number.POSITIVE_INFINITY;
    x[63] = Number.NEGATIVE_INFINITY;
    expect(countNonFinite(x)).toBe(3);
  });
});
