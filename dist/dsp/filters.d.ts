export type BiquadType = 'lowpass' | 'highpass' | 'bandpass' | 'notch' | 'peaking' | 'lowshelf' | 'highshelf' | 'allpass';
/** RBJ Audio EQ Cookbook biquad, coefficients normalized by a0, transposed direct form II. */
export declare class Biquad {
    private b0;
    private b1;
    private b2;
    private a1;
    private a2;
    private z1;
    private z2;
    /** Recompute coefficients; the state is kept so the filter can be modulated. */
    set(type: BiquadType, hz: number, q: number, gainDb: number, sr: number): void;
    process(x: number): number;
    /** Filters buf[0..n) in place. */
    processBlock(buf: Float32Array, n: number): void;
    /** Analytic |H(e^jw)| in dB at hz for the current coefficients, w = 2 pi hz / sr. */
    magnitudeDb(hz: number, sr: number): number;
    reset(): void;
}
/**
 * Simper/Cytomic TPT state-variable filter (g = tan(pi hz / sr), k = 1 / q).
 * Trapezoidal integrators keep it stable under per-sample set().
 */
export declare class Svf {
    private a1;
    private a2;
    private a3;
    private k;
    private ic1;
    private ic2;
    private lpOut;
    private bpOut;
    private hpOut;
    constructor();
    get lp(): number;
    get bp(): number;
    get hp(): number;
    set(hz: number, q: number, sr: number): void;
    process(x: number): void;
    reset(): void;
}
/** One-pole lowpass (coefficient exp(-2 pi hz / sr)); the highpass is x - lowpass. */
export declare class OnePole {
    private a;
    private y;
    private high;
    setLowpass(hz: number, sr: number): void;
    setHighpass(hz: number, sr: number): void;
    process(x: number): number;
    reset(): void;
}
/** DC blocker: y[n] = x[n] - x[n-1] + R y[n-1], R = 0.995. */
export declare class DcBlocker {
    private x1;
    private y1;
    process(x: number): number;
    reset(): void;
}
/** One-pole parameter smoother toward target with time constant timeSec. */
export declare class Smoother {
    target: number;
    value: number;
    private coef;
    constructor(timeSec: number, sr: number);
    /** Change the time constant; timeSec <= 0 makes next() jump straight to the target. */
    setTime(timeSec: number, sr: number): void;
    next(): number;
    /** Set value and target at once (no glide). */
    jump(v: number): void;
    /** Snap the value to the current target. */
    reset(): void;
}
/**
 * 3-band Linkwitz-Riley 4th-order crossover. Each split is two Butterworth biquads in
 * series per side; the low band gets the high split's allpass so that
 * low + mid + high = AP(high) AP(low) x, an allpass response of x.
 */
export declare class Crossover3 {
    private readonly lp1a;
    private readonly lp1b;
    private readonly hp1a;
    private readonly hp1b;
    private readonly lp2a;
    private readonly lp2b;
    private readonly hp2a;
    private readonly hp2b;
    private readonly apLow;
    private lowOut;
    private midOut;
    private highOut;
    constructor(lowHz: number, highHz: number, sr: number);
    get low(): number;
    get mid(): number;
    get high(): number;
    /** Move the split points (highHz is kept at or above lowHz). */
    set(lowHz: number, highHz: number, sr: number): void;
    process(x: number): void;
    reset(): void;
}
