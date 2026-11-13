// Unit tests (Q5) for the voice allocator, the variation picker and the FLAT spatial rule.
import { describe, expect, it } from 'vitest';
import { newSpatialResult, SP, VOICE_LIMITS } from '../../src/contract.ts';
import type { Listener } from '../../src/contract.ts';
import { FLAT_SPATIAL } from '../../src/runtime/flatSpatial.ts';
import { VariationPicker, VoiceAllocator } from '../../src/runtime/voices.ts';

const listener: Listener = { pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } };

describe('runtime/voices VoiceAllocator', () => {
  it('honours the impact limit of 12', () => {
    const a = new VoiceAllocator(VOICE_LIMITS);
    const ids = new Set<number>();
    for (let i = 0; i < 12; i++) {
      const r = a.request('impact', 1 + i, 0, 1);
      expect(r).not.toBeNull();
      expect(r!.stolen).toBe(-1);
      ids.add(r!.id);
    }
    expect(ids.size).toBe(12);
    expect(a.active('impact')).toBe(12);
    expect(a.request('fx', 1, 0, 1)).not.toBeNull();
  });

  it('drops a lower-score 13th request and lets a higher score steal the lowest', () => {
    const a = new VoiceAllocator(VOICE_LIMITS);
    const ids: number[] = [];
    for (let i = 0; i < 12; i++) ids.push(a.request('impact', 1 + i, 0, 1)!.id);
    expect(a.request('impact', 0.5, 0, 1)).toBeNull();
    expect(a.dropped).toBe(1);
    const r = a.request('impact', 100, 0, 1);
    expect(r).toEqual({ id: ids[0], stolen: ids[0] });
    expect(a.stolenCount).toBe(1);
    expect(a.active('impact')).toBe(12);
    // The next lowest is now the score-2 voice.
    expect(a.request('impact', 50, 0, 1)!.stolen).toBe(ids[1]);
  });

  it('expire frees ended voices', () => {
    const a = new VoiceAllocator(VOICE_LIMITS);
    for (let i = 0; i < 12; i++) a.request('impact', 1, 0, i < 4 ? 0.5 : 2);
    a.expire(1);
    expect(a.active('impact')).toBe(8);
    expect(a.request('impact', 0.1, 1, 1)!.stolen).toBe(-1);
    a.expire(10);
    expect(a.active('impact')).toBe(0);
  });

  it('steals over all voices when the 64 slots are exhausted', () => {
    const a = new VoiceAllocator({ impact: 64, fx: 64, ui: 64, ambience: 64, sting: 64 });
    for (let i = 0; i < 64; i++) a.request('impact', 2 + i, 0, 1);
    expect(a.request('fx', 1, 0, 1)).toBeNull();
    expect(a.request('fx', 10, 0, 1)!.stolen).toBe(0);
    expect(a.active('fx')).toBe(1);
    expect(a.active('impact')).toBe(63);
  });
});

describe('runtime/voices VariationPicker', () => {
  it('never repeats back to back in 100 picks of 8 and covers all 8', () => {
    const p = new VariationPicker(1234);
    const seen = new Set<number>();
    let last = -1;
    for (let i = 0; i < 100; i++) {
      const v = p.pick('impact.metal.small', 8);
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(8);
      expect(v).not.toBe(last);
      seen.add(v);
      last = v;
    }
    expect(seen.size).toBe(8);
  });

  it('keeps 10 000 jitters within rate ±3 % and gain ±1 dB', () => {
    const p = new VariationPicker(7);
    const j = { rate: 0, gainDb: 0 };
    for (let i = 0; i < 10000; i++) {
      p.jitter(j);
      expect(j.rate).toBeGreaterThanOrEqual(0.97);
      expect(j.rate).toBeLessThanOrEqual(1.03);
      expect(j.gainDb).toBeGreaterThanOrEqual(-1);
      expect(j.gainDb).toBeLessThanOrEqual(1);
    }
  });
});

describe('runtime/flatSpatial', () => {
  it('gives each of 2 listeners 0.5, pan 0 and lowpass 20000', () => {
    const out = newSpatialResult();
    out.sp.fill(9);
    FLAT_SPATIAL.evaluate([listener, listener], { pos: { x: 5, y: 0, z: 0 } }, null, out);
    expect(out.sp[SP.COUNT]).toBe(2);
    expect(out.sp[SP.SEND]).toBeCloseTo(0.08, 6);
    expect(out.rate).toBe(1);
    for (const l of [0, 1]) {
      expect(out.sp[SP.GAIN + l]).toBe(0.5);
      expect(out.sp[SP.PAN + l]).toBe(0);
      expect(out.sp[SP.LOWPASS + l]).toBe(20000);
    }
    for (const l of [2, 3]) {
      expect(out.sp[SP.GAIN + l]).toBe(0);
      expect(out.sp[SP.LOWPASS + l]).toBe(0);
    }
  });
});
