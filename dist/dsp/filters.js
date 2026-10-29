// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports
// DSP-safe filters: RBJ biquad (TDF-II), Simper TPT state-variable filter, one-pole,
// DC blocker, parameter smoother and a Linkwitz-Riley 3-band crossover.
// Runs unchanged in node, on the main thread and in AudioWorkletGlobalScope.
// Sample-rate agnostic (sr passed in). No allocation after construction.
import { TAU, clamp, finiteOr, gainToDb } from "./math.js";
/** Clamp a cutoff to [10, 0.49 sr]; a non-finite hz resolves to 1 kHz. */
function clampHz(hz, sr) {
    return clamp(finiteOr(hz, 1000), 10, 0.49 * sr);
}
/** A usable sample rate: non-finite or tiny rates fall back to 48 kHz. */
function safeSr(sr) {
    return sr > 1 && Number.isFinite(sr) ? sr : 48000;
}
/** RBJ Audio EQ Cookbook biquad, coefficients normalized by a0, transposed direct form II. */
export class Biquad {
    b0 = 1;
    b1 = 0;
    b2 = 0;
    a1 = 0;
    a2 = 0;
    z1 = 0;
    z2 = 0;
    /** Recompute coefficients; the state is kept so the filter can be modulated. */
    set(type, hz, q, gainDb, sr) {
        sr = safeSr(sr);
        const f = clampHz(hz, sr);
        const qq = Math.max(1e-3, finiteOr(q, Math.SQRT1_2));
        const w0 = (TAU * f) / sr;
        const cw = Math.cos(w0);
        const alpha = Math.sin(w0) / (2 * qq);
        const A = Math.pow(10, finiteOr(gainDb, 0) / 40);
        let b0, b1, b2, a0, a1, a2;
        switch (type) {
            case 'lowpass':
                b0 = (1 - cw) / 2;
                b1 = 1 - cw;
                b2 = b0;
                a0 = 1 + alpha;
                a1 = -2 * cw;
                a2 = 1 - alpha;
                break;
            case 'highpass':
                b0 = (1 + cw) / 2;
                b1 = -(1 + cw);
                b2 = b0;
                a0 = 1 + alpha;
                a1 = -2 * cw;
                a2 = 1 - alpha;
                break;
            case 'bandpass':
                b0 = alpha;
                b1 = 0;
                b2 = -alpha;
                a0 = 1 + alpha;
                a1 = -2 * cw;
                a2 = 1 - alpha;
                break;
            case 'notch':
                b0 = 1;
                b1 = -2 * cw;
                b2 = 1;
                a0 = 1 + alpha;
                a1 = -2 * cw;
                a2 = 1 - alpha;
                break;
            case 'peaking':
                b0 = 1 + alpha * A;
                b1 = -2 * cw;
                b2 = 1 - alpha * A;
                a0 = 1 + alpha / A;
                a1 = -2 * cw;
                a2 = 1 - alpha / A;
                break;
            case 'lowshelf': {
                const s = 2 * Math.sqrt(A) * alpha;
                b0 = A * ((A + 1) - (A - 1) * cw + s);
                b1 = 2 * A * ((A - 1) - (A + 1) * cw);
                b2 = A * ((A + 1) - (A - 1) * cw - s);
                a0 = (A + 1) + (A - 1) * cw + s;
                a1 = -2 * ((A - 1) + (A + 1) * cw);
                a2 = (A + 1) + (A - 1) * cw - s;
                break;
            }
            case 'highshelf': {
                const s = 2 * Math.sqrt(A) * alpha;
                b0 = A * ((A + 1) + (A - 1) * cw + s);
                b1 = -2 * A * ((A - 1) + (A + 1) * cw);
                b2 = A * ((A + 1) + (A - 1) * cw - s);
                a0 = (A + 1) - (A - 1) * cw + s;
                a1 = 2 * ((A - 1) - (A + 1) * cw);
                a2 = (A + 1) - (A - 1) * cw - s;
                break;
            }
            case 'allpass':
                b0 = 1 - alpha;
                b1 = -2 * cw;
                b2 = 1 + alpha;
                a0 = 1 + alpha;
                a1 = -2 * cw;
                a2 = 1 - alpha;
                break;
            default:
                b0 = 1;
                b1 = 0;
                b2 = 0;
                a0 = 1;
                a1 = 0;
                a2 = 0;
        }
        const inv = 1 / a0;
        this.b0 = b0 * inv;
        this.b1 = b1 * inv;
        this.b2 = b2 * inv;
        this.a1 = a1 * inv;
        this.a2 = a2 * inv;
    }
    process(x) {
        const y = this.b0 * x + this.z1;
        this.z1 = this.b1 * x - this.a1 * y + this.z2;
        this.z2 = this.b2 * x - this.a2 * y;
        return y;
    }
    /** Filters buf[0..n) in place. */
    processBlock(buf, n) {
        const b0 = this.b0, b1 = this.b1, b2 = this.b2, a1 = this.a1, a2 = this.a2;
        let z1 = this.z1, z2 = this.z2;
        const m = Math.min(n, buf.length);
        for (let i = 0; i < m; i++) {
            const x = buf[i];
            const y = b0 * x + z1;
            z1 = b1 * x - a1 * y + z2;
            z2 = b2 * x - a2 * y;
            buf[i] = y;
        }
        this.z1 = z1;
        this.z2 = z2;
    }
    /** Analytic |H(e^jw)| in dB at hz for the current coefficients, w = 2 pi hz / sr. */
    magnitudeDb(hz, sr) {
        const w = (TAU * hz) / safeSr(sr);
        const c1 = Math.cos(w), s1 = Math.sin(w);
        const c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
        const nr = this.b0 + this.b1 * c1 + this.b2 * c2;
        const ni = this.b1 * s1 + this.b2 * s2;
        const dr = 1 + this.a1 * c1 + this.a2 * c2;
        const di = this.a1 * s1 + this.a2 * s2;
        const den = dr * dr + di * di;
        if (!(den > 0))
            return 200;
        return gainToDb(Math.sqrt((nr * nr + ni * ni) / den));
    }
    reset() {
        this.z1 = 0;
        this.z2 = 0;
    }
}
/**
 * Simper/Cytomic TPT state-variable filter (g = tan(pi hz / sr), k = 1 / q).
 * Trapezoidal integrators keep it stable under per-sample set().
 */
export class Svf {
    a1 = 0;
    a2 = 0;
    a3 = 0;
    k = 1;
    ic1 = 0;
    ic2 = 0;
    lpOut = 0;
    bpOut = 0;
    hpOut = 0;
    constructor() {
        this.set(1000, Math.SQRT1_2, 48000);
    }
    get lp() { return this.lpOut; }
    get bp() { return this.bpOut; }
    get hp() { return this.hpOut; }
    set(hz, q, sr) {
        sr = safeSr(sr);
        const g = Math.tan((Math.PI * clampHz(hz, sr)) / sr);
        const k = 1 / Math.max(1e-3, finiteOr(q, Math.SQRT1_2));
        this.k = k;
        this.a1 = 1 / (1 + g * (g + k));
        this.a2 = g * this.a1;
        this.a3 = g * this.a2;
    }
    process(x) {
        const v3 = x - this.ic2;
        const v1 = this.a1 * this.ic1 + this.a2 * v3;
        const v2 = this.ic2 + this.a2 * this.ic1 + this.a3 * v3;
        this.ic1 = 2 * v1 - this.ic1;
        this.ic2 = 2 * v2 - this.ic2;
        this.lpOut = v2;
        this.bpOut = v1;
        this.hpOut = x - this.k * v1 - v2;
    }
    reset() {
        this.ic1 = 0;
        this.ic2 = 0;
        this.lpOut = 0;
        this.bpOut = 0;
        this.hpOut = 0;
    }
}
/** One-pole lowpass (coefficient exp(-2 pi hz / sr)); the highpass is x - lowpass. */
export class OnePole {
    a = 0;
    y = 0;
    high = false;
    setLowpass(hz, sr) {
        sr = safeSr(sr);
        this.a = Math.exp((-TAU * clampHz(hz, sr)) / sr);
        this.high = false;
    }
    setHighpass(hz, sr) {
        sr = safeSr(sr);
        this.a = Math.exp((-TAU * clampHz(hz, sr)) / sr);
        this.high = true;
    }
    process(x) {
        const y = (1 - this.a) * x + this.a * this.y;
        this.y = y;
        return this.high ? x - y : y;
    }
    reset() {
        this.y = 0;
    }
}
/** DC blocker: y[n] = x[n] - x[n-1] + R y[n-1], R = 0.995. */
export class DcBlocker {
    x1 = 0;
    y1 = 0;
    process(x) {
        const y = x - this.x1 + 0.995 * this.y1;
        this.x1 = x;
        this.y1 = y;
        return y;
    }
    reset() {
        this.x1 = 0;
        this.y1 = 0;
    }
}
/** One-pole parameter smoother toward target with time constant timeSec. */
export class Smoother {
    target = 0;
    value = 0;
    coef = 0;
    constructor(timeSec, sr) {
        this.setTime(timeSec, sr);
    }
    /** Change the time constant; timeSec <= 0 makes next() jump straight to the target. */
    setTime(timeSec, sr) {
        const n = finiteOr(timeSec, 0) * safeSr(sr);
        this.coef = n > 0 ? Math.exp(-1 / n) : 0;
    }
    next() {
        const v = this.target + (this.value - this.target) * this.coef;
        this.value = Number.isFinite(v) ? v : finiteOr(this.target, 0);
        return this.value;
    }
    /** Set value and target at once (no glide). */
    jump(v) {
        this.value = v;
        this.target = v;
    }
    /** Snap the value to the current target. */
    reset() {
        this.value = this.target;
    }
}
/**
 * 3-band Linkwitz-Riley 4th-order crossover. Each split is two Butterworth biquads in
 * series per side; the low band gets the high split's allpass so that
 * low + mid + high = AP(high) AP(low) x, an allpass response of x.
 */
export class Crossover3 {
    lp1a = new Biquad();
    lp1b = new Biquad();
    hp1a = new Biquad();
    hp1b = new Biquad();
    lp2a = new Biquad();
    lp2b = new Biquad();
    hp2a = new Biquad();
    hp2b = new Biquad();
    apLow = new Biquad();
    lowOut = 0;
    midOut = 0;
    highOut = 0;
    constructor(lowHz, highHz, sr) {
        this.set(lowHz, highHz, sr);
    }
    get low() { return this.lowOut; }
    get mid() { return this.midOut; }
    get high() { return this.highOut; }
    /** Move the split points (highHz is kept at or above lowHz). */
    set(lowHz, highHz, sr) {
        sr = safeSr(sr);
        const f1 = clampHz(lowHz, sr);
        const f2 = Math.max(f1, clampHz(highHz, sr));
        const q = Math.SQRT1_2;
        this.lp1a.set('lowpass', f1, q, 0, sr);
        this.lp1b.set('lowpass', f1, q, 0, sr);
        this.hp1a.set('highpass', f1, q, 0, sr);
        this.hp1b.set('highpass', f1, q, 0, sr);
        this.lp2a.set('lowpass', f2, q, 0, sr);
        this.lp2b.set('lowpass', f2, q, 0, sr);
        this.hp2a.set('highpass', f2, q, 0, sr);
        this.hp2b.set('highpass', f2, q, 0, sr);
        // LR4 LP + HP at f2 equals the 2nd-order allpass at f2 with Q = 1/sqrt(2).
        this.apLow.set('allpass', f2, q, 0, sr);
    }
    process(x) {
        const low = this.lp1b.process(this.lp1a.process(x));
        const rest = this.hp1b.process(this.hp1a.process(x));
        this.lowOut = this.apLow.process(low);
        this.midOut = this.lp2b.process(this.lp2a.process(rest));
        this.highOut = this.hp2b.process(this.hp2a.process(rest));
    }
    reset() {
        this.lp1a.reset();
        this.lp1b.reset();
        this.hp1a.reset();
        this.hp1b.reset();
        this.lp2a.reset();
        this.lp2b.reset();
        this.hp2a.reset();
        this.hp2b.reset();
        this.apLow.reset();
        this.lowOut = 0;
        this.midOut = 0;
        this.highOut = 0;
    }
}
