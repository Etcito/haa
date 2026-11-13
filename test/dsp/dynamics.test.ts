// Unit tests for Ducker, TransientShaper and the TruePeakLimiter ceiling.
import { describe, expect, it } from 'vitest';
import { Ducker, TransientShaper, TruePeakLimiter } from '../../src/dsp/dynamics.ts';
import { truePeakDb } from '../../src/dsp/meter.ts';

const SR = 48000;
const db = (g: number): number => 20 * Math.log10(g);

describe('dsp/dynamics Ducker', () => {
  it('is exactly 1 when idle', () => {
    const d = new Ducker(SR);
    for (let i = 0; i < 100; i++) expect(d.next()).toBe(1);
  });

  it('attacks to the depth, holds, then releases back to 1', () => {
    const d = new Ducker(SR);
    d.trigger(-12, 10, 50, 100);
    const g: number[] = [];
    const total = Math.round(0.3 * SR);
    for (let i = 0; i < total; i++) g.push(d.next());
    const atAttackEnd = g[Math.round(0.01 * SR) + 2];
    expect(db(atAttackEnd)).toBeCloseTo(-12, 1);
    expect(db(g[Math.round(0.04 * SR)])).toBeCloseTo(-12, 1);
    expect(db(g[Math.round(0.01 * SR) + 20])).toBeLessThan(-11.9);
    expect(g[Math.round(0.1 * SR)]).toBeGreaterThan(atAttackEnd);
    expect(g[g.length - 1]).toBe(1);
    expect(d.next()).toBe(1);
  });

  it('takes the deepest of overlapping triggers', () => {
    const d = new Ducker(SR);
    d.trigger(-6, 1, 200, 50);
    d.trigger(-18, 1, 200, 50);
    d.trigger(-3, 1, 200, 50);
    let g = 1;
    for (let i = 0; i < Math.round(0.05 * SR); i++) g = d.next();
    expect(db(g)).toBeCloseTo(-18, 1);
  });

  it('ignores a depth of 0', () => {
    const d = new Ducker(SR);
    d.trigger(0, 10, 50, 100);
    for (let i = 0; i < 1000; i++) expect(d.next()).toBe(1);
  });

  it('survives non-finite arguments without NaN', () => {
    const d = new Ducker(SR);
    const bad = [NaN, Infinity, -Infinity];
    for (const a of bad) {
      expect(() => d.trigger(a, a, a, a)).not.toThrow();
      expect(() => d.trigger(-6, a, a, a)).not.toThrow();
    }
    for (let i = 0; i < 20000; i++) {
      const g = d.next();
      expect(Number.isFinite(g)).toBe(true);
      expect(g).toBeLessThanOrEqual(1);
      expect(g).toBeGreaterThan(0);
    }
  });

  it('reset returns the gain to 1', () => {
    const d = new Ducker(SR);
    d.trigger(-12, 1, 500, 500);
    for (let i = 0; i < 500; i++) d.next();
    expect(d.next()).toBeLessThan(1);
    d.reset();
    expect(d.next()).toBe(1);
  });
});

/** Decaying 440 Hz burst, exp(-t/0.1) over 1 s. */
function burst(): Float32Array {
  const x = new Float32Array(SR);
  for (let i = 0; i < x.length; i++) {
    x[i] = 0.5 * Math.exp(-i / SR / 0.1) * Math.sin((2 * Math.PI * 440 * i) / SR);
  }
  return x;
}

function run(ts: TransientShaper, x: Float32Array): Float32Array {
  const y = new Float32Array(x.length);
  for (let i = 0; i < x.length; i++) y[i] = ts.process(x[i]);
  return y;
}

function peak(x: Float32Array, from: number, to: number): number {
  let p = 0;
  for (let i = from; i < to; i++) p = Math.max(p, Math.abs(x[i]));
  return p;
}

describe('dsp/dynamics TransientShaper', () => {
  const x = burst();
  const onset: [number, number] = [0, Math.round(0.02 * SR)];
  const tail: [number, number] = [Math.round(0.3 * SR), Math.round(0.5 * SR)];

  it('is the identity at set(0, 0)', () => {
    const ts = new TransientShaper(SR);
    ts.set(0, 0);
    const y = run(ts, x);
    for (let i = 0; i < x.length; i += 97) expect(y[i]).toBe(x[i]);
  });

  it('boosts the onset with a positive attackDb', () => {
    const ts = new TransientShaper(SR);
    ts.set(12, 0);
    const y = run(ts, x);
    expect(peak(y, ...onset)).toBeGreaterThan(peak(x, ...onset) * 1.5);
  });

  it('shapes the tail by sustainDb (negative cuts, positive boosts, as in the origin engine)', () => {
    const cut = new TransientShaper(SR);
    cut.set(0, -12);
    expect(peak(run(cut, x), ...tail)).toBeLessThan(peak(x, ...tail) * 0.7);
    const boost = new TransientShaper(SR);
    boost.set(0, 12);
    expect(peak(run(boost, x), ...tail)).toBeGreaterThan(peak(x, ...tail) * 1.5);
  });

  it('returns 0 for NaN and Infinity input', () => {
    const ts = new TransientShaper(SR);
    ts.set(6, 6);
    expect(ts.process(NaN)).toBe(0);
    expect(ts.process(Infinity)).toBe(0);
    expect(ts.process(-Infinity)).toBe(0);
  });

  it('clamps the gains to +/-24 dB', () => {
    const a = new TransientShaper(SR);
    const b = new TransientShaper(SR);
    a.set(1000, 0);
    b.set(24, 0);
    const ya = run(a, x);
    const yb = run(b, x);
    for (let i = 0; i < x.length; i += 31) expect(ya[i]).toBeCloseTo(yb[i], 9);
    expect(peak(ya, ...onset) / peak(x, ...onset)).toBeLessThanOrEqual(Math.pow(10, 24 / 20) * 1.0001);
    const c = new TransientShaper(SR);
    const e = new TransientShaper(SR);
    c.set(-1000, 0);
    e.set(-24, 0);
    const yc = run(c, x);
    const ye = run(e, x);
    for (let i = 0; i < x.length; i += 31) expect(yc[i]).toBeCloseTo(ye[i], 9);
  });
});

describe('dsp/dynamics TruePeakLimiter', () => {
  it('never exceeds the ceiling on a loud burst', () => {
    const lim = new TruePeakLimiter(SR);
    const ceiling = -1.5;
    lim.set(ceiling, 5, 100);
    const n = SR;
    const l = new Float32Array(n);
    const r = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      const s = 2 * Math.sin((2 * Math.PI * 1000 * i) / SR);
      l[i] = s;
      r[i] = s * 0.7;
    }
    lim.processStereo(l, r, n);
    const lim_ceiling = Math.pow(10, ceiling / 20);
    let p = 0;
    for (let i = 0; i < n; i++) p = Math.max(p, Math.abs(l[i]), Math.abs(r[i]));
    expect(p).toBeLessThanOrEqual(lim_ceiling + 1e-6);
    expect(truePeakDb(l, r, SR)).toBeLessThanOrEqual(ceiling + 0.2);
  });
});
