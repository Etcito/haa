// Unit tests for the offline render helpers (synthetic family and bed only).
import { describe, expect, it } from 'vitest';
import { BLOCK } from '../../src/contract.ts';
import type { BedDef, FamilyDef, MixPreset } from '../../src/contract.ts';
import { renderFamilyVariants } from '../../src/bank/render.ts';
import { renderBedDemo, renderFamilyStrip, renderOffline } from '../../src/offline/render.ts';

const SR = 48000;
// renderFamilyVariants only reads preset.loudness.
const PRESET = {
  loudness: { impact: -21, fx: -20, ui: -24, ambience: -26, sting: -18 },
} as unknown as MixPreset;

/** 8 short decaying noise bursts of different lengths. */
const FAMILY: FamilyDef = {
  id: 'test.strip',
  bus: 'effects',
  category: 'fx',
  variants: 8,
  maxSeconds: 0.3,
  seed: 3,
  render(out, sr, rng, v) {
    const n = Math.min(out.length, Math.round((0.05 + 0.02 * v) * sr));
    for (let i = 0; i < n; i++) out[i] += 0.2 * Math.exp(-i / (0.03 * sr)) * (2 * rng.next() - 1);
    return n;
  },
};

describe('renderOffline', () => {
  it('has length round(seconds × sr) and advances frame by BLOCK', () => {
    const frames: number[] = [];
    const sizes: number[] = [];
    let dirty = 0;
    const out = renderOffline((l, r, n, frame) => {
      frames.push(frame);
      sizes.push(n);
      for (let i = 0; i < n; i++) {
        if (l[i] !== 0 || r[i] !== 0) dirty++;
        l[i] = 1;
        r[i] = -1;
      }
    }, 1.001);
    const total = Math.round(1.001 * SR);
    expect(dirty).toBe(0);
    expect(out.left.length).toBe(total);
    expect(out.right.length).toBe(total);
    expect(out.sampleRate).toBe(SR);
    expect(out.name).toBe('render');
    expect(frames.length).toBe(Math.ceil(total / BLOCK));
    for (let k = 0; k < frames.length; k++) expect(frames[k]).toBe(k * BLOCK);
    expect(sizes[sizes.length - 1]).toBe(total - (frames.length - 1) * BLOCK);
    expect(out.left.every((x) => x === 1)).toBe(true);
    expect(out.right.every((x) => x === -1)).toBe(true);
  });
});

describe('renderFamilyStrip', () => {
  it('places the variants with 7 gaps, centered with L === R', () => {
    const variants = renderFamilyVariants(FAMILY, SR, PRESET);
    expect(variants.length).toBe(8);
    const gap = Math.round(0.25 * SR);
    const strip = renderFamilyStrip(FAMILY, PRESET);
    const sum = variants.reduce((a, x) => a + x.length, 0);
    expect(strip.left.length).toBe(sum + 7 * gap);
    expect(strip.right).toEqual(strip.left);
    let at = 0;
    for (const x of variants) {
      for (let i = 0; i < x.length; i += 97) expect(strip.left[at + i]).toBe(Math.fround(x[i] * Math.SQRT1_2));
      at += x.length;
      for (let i = 0; i < gap && at + i < strip.left.length; i++) expect(strip.left[at + i]).toBe(0);
      at += gap;
    }
  });
});

describe('renderBedDemo', () => {
  it('calls the script once per block before process and returns the raw output', () => {
    const times: number[] = [];
    let processed = 0;
    let lastP = -1;
    const BED: BedDef = {
      id: 'test-bed',
      bus: 'ambience',
      params: ['level'],
      defaults: [0.5],
      positional: false,
      create: () => ({
        process(p, offset, l, r, n) {
          processed++;
          lastP = p[offset];
          for (let i = 0; i < n; i++) {
            l[i] += p[offset];
            r[i] += -p[offset];
          }
        },
        reset() {},
      }),
    };
    const seconds = 0.1;
    const out = renderBedDemo(BED, {
      seconds,
      seed: 1,
      script: (t, p) => {
        expect(times.length).toBe(processed);
        if (times.length === 0) expect(p[0]).toBe(0.5);
        times.push(t);
        p[0] = 0.25;
      },
    });
    const total = Math.round(seconds * SR);
    const blocks = Math.ceil(total / BLOCK);
    expect(times.length).toBe(blocks);
    expect(processed).toBe(blocks);
    for (let k = 0; k < blocks; k++) expect(times[k]).toBe((k * BLOCK) / SR);
    expect(lastP).toBe(0.25);
    expect(out.left.length).toBe(total);
    expect(out.name).toBe('test-bed');
    expect(out.left.every((x) => x === 0.25)).toBe(true);
    expect(out.right.every((x) => x === -0.25)).toBe(true);
    expect(BED.defaults[0]).toBe(0.5);
  });
});
