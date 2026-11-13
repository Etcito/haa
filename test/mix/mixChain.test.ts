// Unit tests for MixChain with the base preset: limiter ceiling, music ducking, non-finite guard.
import { describe, expect, it } from 'vitest';
import { BLOCK, BUS_INDEX, MIX_INPUTS } from '../../src/contract.ts';
import { MixChain } from '../../src/mix/mixChain.ts';
import { BASE_PRESET } from '../../src/mix/presets/base.ts';
import { rmsDb, truePeakDb } from '../../src/dsp/meter.ts';

const SR = 48000;

type Pair = readonly [Float32Array, Float32Array];

/** Run the chain block by block; fill(inputs, frame0) writes one block of every input. */
function render(
  chain: MixChain,
  seconds: number,
  fill: (inputs: Pair[], frame0: number, block: number) => void,
  onBlock?: (block: number) => void,
): { l: Float32Array; r: Float32Array } {
  const total = Math.round(seconds * SR);
  const l = new Float32Array(total);
  const r = new Float32Array(total);
  const inputs: Pair[] = [];
  for (let k = 0; k < MIX_INPUTS; k++) inputs.push([new Float32Array(BLOCK), new Float32Array(BLOCK)]);
  const oL = new Float32Array(BLOCK);
  const oR = new Float32Array(BLOCK);
  for (let off = 0, b = 0; off < total; off += BLOCK, b++) {
    for (const [a, c] of inputs) {
      a.fill(0);
      c.fill(0);
    }
    fill(inputs, off, b);
    onBlock?.(b);
    const n = Math.min(BLOCK, total - off);
    chain.process(inputs, oL, oR, n);
    l.set(oL.subarray(0, n), off);
    r.set(oR.subarray(0, n), off);
  }
  return { l, r };
}

function sineInto(x: Float32Array, frame0: number, amp: number, hz: number): void {
  for (let i = 0; i < x.length; i++) x[i] = amp * Math.sin((2 * Math.PI * hz * (frame0 + i)) / SR);
}

describe('mix/MixChain with BASE_PRESET', () => {
  it('keeps a +6 dBFS sine on every input at or below -1.3 dBTP after 10 ms', () => {
    const chain = new MixChain(SR, BASE_PRESET);
    const amp = Math.pow(10, 6 / 20);
    const { l, r } = render(chain, 1, (inputs, f0) => {
      for (const [a, c] of inputs) {
        sineInto(a, f0, amp, 1000);
        c.set(a);
      }
    });
    const from = Math.round(0.01 * SR);
    const tp = truePeakDb(l.subarray(from), r.subarray(from), SR);
    expect(Number.isFinite(tp)).toBe(true);
    expect(tp).toBeLessThanOrEqual(-1.3);
  });

  it("duck('impact', 1) lowers the music bus RMS over 100..300 ms", () => {
    const music = (inputs: Pair[], f0: number): void => {
      const [a, c] = inputs[BUS_INDEX.music];
      sineInto(a, f0, 0.25, 440);
      c.set(a);
    };
    const window = (x: Float32Array): number => rmsDb(x, Math.round(0.1 * SR), Math.round(0.3 * SR));
    const dry = render(new MixChain(SR, BASE_PRESET), 0.3, music);
    const ducked = new MixChain(SR, BASE_PRESET);
    const wet = render(ducked, 0.3, music, (b) => {
      if (b === 0) ducked.duck('impact', 1);
    });
    const dryDb = window(dry.l);
    const wetDb = window(wet.l);
    expect(Number.isFinite(dryDb)).toBe(true);
    expect(wetDb).toBeLessThan(dryDb - 1);
  });

  it('zeroes the block holding a NaN input sample, then outputs finite audio', () => {
    const chain = new MixChain(SR, BASE_PRESET);
    const badBlock = 20;
    const { l, r } = render(chain, 0.2, (inputs, f0, b) => {
      for (const [a, c] of inputs) {
        sineInto(a, f0, 0.25, 440);
        c.set(a);
      }
      if (b === badBlock) inputs[BUS_INDEX.effects][0][37] = Number.NaN;
    });
    const s = badBlock * BLOCK;
    for (let i = s; i < s + BLOCK; i++) {
      expect(l[i]).toBe(0);
      expect(r[i]).toBe(0);
    }
    let nonFinite = 0;
    let energy = 0;
    for (let i = s + BLOCK; i < l.length; i++) {
      if (!Number.isFinite(l[i]) || !Number.isFinite(r[i])) nonFinite++;
      energy += l[i] * l[i] + r[i] * r[i];
    }
    expect(nonFinite).toBe(0);
    expect(energy).toBeGreaterThan(0);
  });
});
