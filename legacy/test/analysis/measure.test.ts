// Unit tests for the analysis measures and the safety verdict on synthetic signals.
import { describe, expect, it } from 'vitest';
import { measureMono, measureStereo, OCTAVE_BANDS, safetyVerdict } from '../../src/analysis/measure.ts';
import { oneShotLufs } from '../../src/bank/render.ts';

const SR = 48000;

/** A sine of the given peak dBFS, frequency and length, plus a constant offset. */
function sine(peakDb: number, hz: number, seconds: number, offset = 0): Float32Array {
  const amp = Math.pow(10, peakDb / 20);
  const x = new Float32Array(Math.round(seconds * SR));
  for (let i = 0; i < x.length; i++) x[i] = offset + amp * Math.sin((2 * Math.PI * hz * i) / SR);
  return x;
}

/** A decaying 440 Hz + 2.2 kHz burst, so the loudness gating sees a real envelope. */
function burst(seconds: number): Float32Array {
  const x = new Float32Array(Math.round(seconds * SR));
  for (let i = 0; i < x.length; i++) {
    const t = i / SR;
    x[i] = Math.exp(-3 * t) * (0.4 * Math.sin(2 * Math.PI * 440 * t) + 0.1 * Math.sin(2 * Math.PI * 2200 * t));
  }
  return x;
}

function argmax(xs: readonly number[]): number {
  let best = 0;
  for (let i = 1; i < xs.length; i++) if (xs[i] > xs[best]) best = i;
  return best;
}

describe('analysis/measure', () => {
  it('lists the ten octave band centers', () => {
    expect(OCTAVE_BANDS).toEqual([31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]);
  });

  it('detects a 0.01 DC offset and fails safety on dc', () => {
    const m = measureMono(sine(-20, 1000, 1, 0.01), SR);
    expect(m.dc).toBeCloseTo(0.01, 6);
    expect(safetyVerdict(m)).toEqual({ ok: false, failures: ['dc'] });
  });

  it('fails silence on a silent signal', () => {
    const m = measureStereo(new Float32Array(SR), new Float32Array(SR), SR);
    expect(m.rmsDb).toBe(-200);
    expect(safetyVerdict(m)).toEqual({ ok: false, failures: ['silence'] });
  });

  it('passes a -20 dBFS 1 kHz sine whose loudest band is 1000 Hz', () => {
    const m = measureMono(sine(-20, 1000, 1), SR);
    expect(safetyVerdict(m)).toEqual({ ok: true, failures: [] });
    expect(m.frames).toBe(SR);
    expect(m.seconds).toBe(1);
    expect(m.samplePeakDb).toBeCloseTo(-20, 2);
    expect(m.rmsDb).toBeCloseTo(-23.01, 1);
    expect(m.bandsDb).toHaveLength(OCTAVE_BANDS.length);
    expect(OCTAVE_BANDS[argmax(m.bandsDb)]).toBe(1000);
  });

  it('meters measureMono loudness as the centered pair (x√½, x√½)', () => {
    const x = burst(1.5);
    const half = x.map((v) => v * Math.SQRT1_2);
    const mono = measureMono(x, SR);
    expect(Number.isFinite(mono.lufs)).toBe(true);
    expect(Math.abs(mono.lufs - measureStereo(half, half, SR).lufs)).toBeLessThanOrEqual(1e-6);
    expect(Math.abs(mono.lufs - oneShotLufs(x, x.length, SR))).toBeLessThanOrEqual(1e-6);
    // Peaks, dc and RMS come from x itself, as for the stereo pair (x, x).
    const pair = measureStereo(x, x, SR);
    expect(mono.samplePeakDb).toBe(pair.samplePeakDb);
    expect(mono.rmsDb).toBeCloseTo(pair.rmsDb, 9);
  });

  it('counts non-finite samples, then measures them as 0', () => {
    const x = sine(-6, 1000, 1);
    x[10] = Number.NaN;
    x[20] = Number.POSITIVE_INFINITY;
    x[30] = 1;
    const m = measureMono(x, SR);
    expect(m.nonFinite).toBe(2);
    expect(m.samplePeakDb).toBe(0);
    expect(Number.isFinite(m.rmsDb) && Number.isFinite(m.truePeakDb) && Number.isFinite(m.lufs)).toBe(true);
    expect(safetyVerdict(m)).toEqual({ ok: false, failures: ['nonFinite', 'samplePeak'] });
  });

  it('takes the minimum 1 s window RMS from 2 s on', () => {
    const x = sine(-20, 1000, 3);
    x.fill(0, 2 * SR);
    const m = measureStereo(x, x, SR);
    expect(m.rmsDb).toBeGreaterThan(-60);
    expect(m.minWindowRmsDb).toBe(-200);
    expect(safetyVerdict(m).failures).toEqual(['silence']);
  });

  it('reports -200 for a band at or above Nyquist', () => {
    const m = measureMono(sine(-20, 1000, 0.5), 32000);
    expect(m.bandsDb[OCTAVE_BANDS.indexOf(16000)]).toBe(-200);
    expect(m.bandsDb[OCTAVE_BANDS.indexOf(8000)]).toBeGreaterThan(-200);
  });
});
