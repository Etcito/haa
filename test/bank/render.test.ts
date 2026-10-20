// Unit tests for bank variant rendering and family loudness normalization (synthetic families only).
import { describe, expect, it } from 'vitest';
import { NORMALIZE_PEAK_CAP_DB } from '../../src/contract.ts';
import type { FamilyDef, MixPreset } from '../../src/contract.ts';
import {
  familyGain,
  oneShotLufs,
  renderFamilyVariants,
  renderFamilyVariantsAsync,
  renderVariantRaw,
} from '../../src/bank/render.ts';
import { samplePeakDb } from '../../src/dsp/meter.ts';

const SR = 48000;
const TARGET = -20;
// renderFamilyVariants only reads preset.loudness.
const PRESET = {
  loudness: { impact: -21, fx: TARGET, ui: -24, ambience: -26, sting: -18 },
} as unknown as MixPreset;

/** 8 noise bursts of 0.3 s with a short decay tail; moderate crest factor. */
const NOISE: FamilyDef = {
  id: 'test.noise',
  bus: 'effects',
  category: 'fx',
  variants: 8,
  maxSeconds: 0.5,
  seed: 7,
  render(out, sr, rng) {
    const amp = rng.range(0.05, 0.2);
    const n = Math.min(out.length, Math.round(0.3 * sr));
    for (let i = 0; i < n; i++) out[i] += amp * Math.exp(-i / (0.2 * sr)) * (2 * rng.next() - 1);
    return n;
  },
};

/** Single-sample spikes over a -60 dBFS noise floor: loud peak, very low loudness. */
const SPIKES: FamilyDef = {
  id: 'test.spikes',
  bus: 'effects',
  category: 'fx',
  variants: 8,
  maxSeconds: 0.3,
  seed: 11,
  render(out, sr, rng) {
    const n = Math.min(out.length, Math.round(0.2 * sr));
    for (let i = 0; i < n; i++) out[i] += 1e-3 * (2 * rng.next() - 1);
    out[100] += rng.range(0.2, 0.5);
    return n;
  },
};

function lowerMedian(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor((s.length - 1) / 2)];
}

describe('bank/render', () => {
  it('renders bit-identical families twice', () => {
    const a = renderFamilyVariants(NOISE, SR, PRESET);
    const b = renderFamilyVariants(NOISE, SR, PRESET);
    expect(a.length).toBe(8);
    for (let v = 0; v < a.length; v++) {
      expect(Buffer.from(a[v].buffer).equals(Buffer.from(b[v].buffer))).toBe(true);
    }
  });

  it('normalizes a noise-burst family median to the category target', () => {
    const out = renderFamilyVariants(NOISE, SR, PRESET);
    expect(out.length).toBe(NOISE.variants);
    const median = lowerMedian(out.map((x) => oneShotLufs(x, x.length, SR)));
    expect(Math.abs(median - TARGET)).toBeLessThanOrEqual(0.05);
    expect(Math.max(...out.map((x) => samplePeakDb(x, null)))).toBeLessThan(NORMALIZE_PEAK_CAP_DB);
  });

  it('caps a peaky family at NORMALIZE_PEAK_CAP_DB', () => {
    const out = renderFamilyVariants(SPIKES, SR, PRESET);
    const peak = Math.max(...out.map((x) => samplePeakDb(x, null)));
    expect(Math.abs(peak - NORMALIZE_PEAK_CAP_DB)).toBeLessThanOrEqual(0.01);
    const median = lowerMedian(out.map((x) => oneShotLufs(x, x.length, SR)));
    expect(median).toBeLessThan(TARGET - 1);
  });

  it('clamps a render count beyond out.length', () => {
    const def: FamilyDef = {
      ...NOISE,
      id: 'test.over',
      maxSeconds: 0.1,
      render(out) {
        for (let i = 0; i < out.length; i++) out[i] = i % 2 === 0 ? 0.25 : -0.25;
        return out.length + 1000;
      },
    };
    expect(renderVariantRaw(def, 0, SR).length).toBe(Math.ceil(0.1 * SR));
  });

  it('returns gain 1 when no variant has a finite loudness', () => {
    expect(familyGain([new Float32Array(10), new Float32Array(0)], TARGET, SR)).toEqual({
      gain: 1,
      medianLufs: -Infinity,
    });
  });

  it('gives the same output when sliced asynchronously', async () => {
    let t = 0;
    const out = await renderFamilyVariantsAsync(NOISE, SR, PRESET, 1, () => (t += 1), async () => {});
    const ref = renderFamilyVariants(NOISE, SR, PRESET);
    for (let v = 0; v < ref.length; v++) {
      expect(Buffer.from(out[v].buffer).equals(Buffer.from(ref[v].buffer))).toBe(true);
    }
  });
});
