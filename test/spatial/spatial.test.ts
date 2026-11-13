// Unit tests for the spatial math: the shared known-geometry table, distance gain and the
// right-hand rule of the listener frame; the model's 1/N, dominant-listener and null-source rules.
import { describe, expect, it } from 'vitest';
import type { Listener, SourcePos } from '../../src/contract.ts';
import {
  DEFAULT_RANGE, REF_DISTANCE, distanceGain, dopplerRatio, spatializeOne,
} from '../../src/spatial/spatial.ts';
import type { OneListener } from '../../src/spatial/spatial.ts';
import { createSpatial } from '../../src/spatial/model.ts';
import { SP, newSpatialResult } from '../../src/contract.ts';

const TOL = 1e-4;
const ENV = { absorptionScale: 1 };
const origin = (): Listener => ({ pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } });
const newOut = (): OneListener => ({ gain: 0, pan: 0, lowpass: 0, send: 0, doppler: 0, dist: 0 });
const at = (x: number, y: number, z: number, extra: Partial<SourcePos> = {}): SourcePos => ({ pos: { x, y, z }, ...extra });
const one = (src: SourcePos, l = origin(), count = 1, occ = 0) => spatializeOne(l, src, count, occ, ENV, newOut());
const near = (got: number, want: number) => expect(Math.abs(got - want)).toBeLessThanOrEqual(TOL);

describe('spatial/spatial known-geometry table', () => {
  it('(0,0,-2): gain 1, pan 0, lowpass 19900, send 0.0916667', () => {
    const r = one(at(0, 0, -2));
    near(r.gain, 1); near(r.pan, 0); near(r.lowpass, 19900); near(r.send, 0.0916667);
    expect(r.dist).toBe(2);
  });

  it('(8,0,0): gain 0.5, pan 1, lowpass 19600, send 0.1266667', () => {
    const r = one(at(8, 0, 0));
    near(r.gain, 0.5); near(r.pan, 1); near(r.lowpass, 19600); near(r.send, 0.1266667);
  });

  it('(0,0,40): gain 0.1, |pan| < 1e-6, lowpass 6300', () => {
    const r = one(at(0, 0, 40));
    near(r.gain, 0.1); expect(Math.abs(r.pan)).toBeLessThan(1e-6); near(r.lowpass, 6300);
  });

  it('(0,0,-68): gain 0.0294118', () => {
    near(one(at(0, 0, -68)).gain, 0.0294118);
  });

  it('(0,0,-2) occluded 1: gain 0.3548134, lowpass 995 (source field or passed value)', () => {
    for (const r of [one(at(0, 0, -2, { occlusion: 1 })), one(at(0, 0, -2), origin(), 1, 1)]) {
      near(r.gain, 0.3548134); near(r.lowpass, 995);
    }
  });

  it('Doppler approach 1.0540926 and recede 0.9534626', () => {
    const l = origin();
    near(dopplerRatio(l, at(0, 0, -100, { vel: { x: 0, y: 0, z: 34.3 } })), 1.0540926);
    near(dopplerRatio(l, at(0, 0, -100, { vel: { x: 0, y: 0, z: -34.3 } })), 0.9534626);
    near(one(at(0, 0, -100, { vel: { x: 0, y: 0, z: 34.3 } })).doppler, 1.0540926);
  });

  it('2 listeners [origin, (100,0,0)], src (0,0,-8): raw g0 0.5 (0.25 after 1/N), g1 0, pan0 0', () => {
    const src = at(0, 0, -8);
    const l1: Listener = { pos: { x: 100, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } };
    const r0 = one(src, origin(), 2);
    const r1 = one(src, l1, 2);
    near(r0.gain, 0.5); near(r0.pan, 0); near(r1.gain, 0);
  });

  it('co-located source: gain 1, pan 0, lowpass 20000, send base, Doppler 1', () => {
    const r = one(at(0, 0, 0, { vel: { x: 0, y: 0, z: 10 } }));
    expect([r.gain, r.pan, r.lowpass, r.doppler]).toEqual([1, 0, 20000, 1]);
    near(r.send, 0.08);
  });
});

describe('spatial/spatial distanceGain', () => {
  it('is 1 at d <= ref and 0 at d >= max', () => {
    expect(distanceGain(0)).toBe(1);
    expect(distanceGain(REF_DISTANCE)).toBe(1);
    expect(distanceGain(DEFAULT_RANGE)).toBe(0);
    expect(distanceGain(500)).toBe(0);
    expect(distanceGain(20, 4, 10)).toBe(0);
  });
});

describe('spatial/spatial right-hand rule', () => {
  /**
   * A listener yawed +90° about +Y (q = (0, sin45°, 0, cos45°)) turns counter-clockwise seen
   * from above: forward (0,0,-1) becomes -X and right (1,0,0) becomes -Z. A source at (0,0,-5),
   * formerly in front, is then on the listener's RIGHT: pan = +1. Yaw -90° puts it on the
   * left: pan = -1.
   */
  it('yaw +90° gives pan +1 and yaw -90° gives pan -1 for a source at (0,0,-5)', () => {
    const s = Math.SQRT1_2;
    const yaw = (sign: number): Listener => ({ pos: { x: 0, y: 0, z: 0 }, quat: { x: 0, y: sign * s, z: 0, w: s } });
    const left = one(at(0, 0, -5), yaw(-1));
    const right = one(at(0, 0, -5), yaw(1));
    near(Math.abs(right.pan), 1); expect(right.pan).toBeGreaterThan(0);
    near(Math.abs(left.pan), 1); expect(left.pan).toBeLessThan(0);
  });
});

describe('spatial/model createSpatial', () => {
  const origins = (n: number) => Array.from({ length: n }, origin);
  const run = (ls: Listener[], src: SourcePos | null) => createSpatial().evaluate(ls, src, null, newSpatialResult());

  it('N = 1..4 co-located listeners, null source: gains sum to 1, count N', () => {
    for (let n = 1; n <= 4; n++) {
      const r = run(origins(n), null);
      let sum = 0;
      for (let l = 0; l < n; l++) sum += r.sp[SP.GAIN + l];
      expect(Math.abs(sum - 1)).toBeLessThanOrEqual(1e-6);
      expect(r.sp[SP.COUNT]).toBe(n);
      expect(r.rate).toBe(1);
    }
  });

  it('N = 1..4 listeners at the origin, src (0,0,-8): each gain = distanceGain(8)/N', () => {
    for (let n = 1; n <= 4; n++) {
      const r = run(origins(n), at(0, 0, -8));
      for (let l = 0; l < n; l++) expect(Math.abs(r.sp[SP.GAIN + l] - distanceGain(8) / n)).toBeLessThanOrEqual(1e-6);
    }
  });

  it('2 listeners [origin, (100,0,0)], src (0,0,-8): g0 0.25, g1 0, pan0 0; send and rate from l0', () => {
    const src = at(0, 0, -8, { vel: { x: 0, y: 0, z: 10 } });
    const l1: Listener = { pos: { x: 100, y: 0, z: 0 }, quat: { x: 0, y: 0, z: 0, w: 1 } };
    const r = run([origin(), l1], src);
    near(r.sp[SP.GAIN], 0.25); near(r.sp[SP.GAIN + 1], 0); near(r.sp[SP.PAN], 0);
    const r0 = one(src, origin(), 2);
    near(r.sp[SP.SEND], r0.send); near(r.rate, r0.doppler);
  });

  it('0 listeners behaves as one listener at the origin', () => {
    const src = at(8, 0, 0);
    expect(Array.from(run([], src).sp)).toEqual(Array.from(run([origin()], src).sp));
    expect(run([], null).sp[SP.COUNT]).toBe(1);
  });

  it('evaluate twice gives identical results', () => {
    const m = createSpatial();
    const ls = [origin(), { pos: { x: 3, y: 0, z: 1 }, quat: { x: 0, y: 0, z: 0, w: 1 } }];
    const src = at(2, 1, -6, { vel: { x: 5, y: 0, z: 0 } });
    const a = m.evaluate(ls, src, () => 0.3, newSpatialResult());
    const b = m.evaluate(ls, src, () => 0.3, newSpatialResult());
    expect(Array.from(b.sp)).toEqual(Array.from(a.sp));
    expect(b.rate).toBe(a.rate);
  });
});
