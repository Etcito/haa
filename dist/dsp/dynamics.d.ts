import type { CompPreset } from '../contract.js';
/**
 * Stereo-linked feed-forward compressor. The instantaneous peak max(|l|, |r|) feeds a
 * log-domain gain computer with a quadratic soft knee; the gain reduction (dB) is smoothed
 * with separate attack/release one-poles, then the makeup gain is applied.
 */
export declare class Compressor {
    private readonly sr;
    private threshold;
    private slope;
    private knee;
    private kneeLoLin;
    private attackCoef;
    private releaseCoef;
    private makeupDb;
    private env;
    private maxRed;
    constructor(sr: number);
    /** Last block's maximum gain reduction, in dB (>= 0). */
    get reductionDb(): number;
    set(p: CompPreset): void;
    /** Static gain computer: the desired gain reduction (dB >= 0) for a peak in dB. */
    private computeReduction;
    processStereo(l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/**
 * Lookahead true-peak limiter (stereo-linked). Per sample:
 * 1. the input is delayed by the lookahead (one delay line per channel);
 * 2. the peak is detected on the 4x oversampled input (meter.ts TruePeakDetector);
 * 3. the required gain min(1, ceiling / peak) goes through a running minimum over the
 *    lookahead window (monotonic deque in preallocated Int32/Float32 rings);
 * 4. it is smoothed by a raised-cosine (Hann) attack across the lookahead, computed with
 *    O(1) sliding sums, and an exponential release;
 * 5. the delayed sample is multiplied by the gain, then clamped at the ceiling (sample peak).
 * The Hann average of a held minimum reaches the required gain exactly when the peak leaves
 * the delay, so the output true peak stays <= ceiling + 0.2 dB.
 */
export declare class TruePeakLimiter {
    private readonly sr;
    private readonly detL;
    private readonly detR;
    private readonly dl;
    private readonly dr;
    private readonly dMask;
    private dPos;
    private readonly qIdx;
    private readonly qVal;
    private readonly qMask;
    private qHead;
    private qSize;
    private t;
    private readonly hRing;
    private readonly cosT;
    private readonly sinT;
    private readonly maxAttack;
    private hPos;
    private sumB;
    private sumCr;
    private sumCi;
    private resync;
    private ceiling;
    private attackLen;
    private delay;
    private hold;
    private relCoef;
    private gain;
    private minGain;
    constructor(sr: number);
    /** Last block's maximum gain reduction, in dB (>= 0). */
    get reductionDb(): number;
    set(ceilingDbtp: number, lookaheadMs: number, releaseMs: number): void;
    /** Recompute the sliding sums exactly from the ring (drift control). */
    private resyncSums;
    processStereo(l: Float32Array, r: Float32Array, n: number): void;
    reset(): void;
}
/**
 * Sidechain-free ducker. Each trigger runs its own envelope, linear in dB: attack from 0 dB
 * to -depth, hold, release back to 0 dB. Overlapping triggers take the deepest (the minimum
 * over the active envelopes). next() returns the linear gain for one sample.
 */
export declare class Ducker {
    private readonly sr;
    private readonly phase;
    private readonly level;
    private readonly depth;
    private readonly attackStep;
    private readonly releaseStep;
    private readonly holdLeft;
    private active;
    constructor(sr: number);
    trigger(depthDb: number, attackMs: number, holdMs: number, releaseMs: number): void;
    next(): number;
    reset(): void;
}
/**
 * Transient shaper from the difference of a fast (1 ms attack) and a slow (30 ms attack)
 * peak follower. The normalized difference d = (fast - slow) / max(fast, slow) is positive
 * on onsets (scaled by attackDb) and negative on decaying tails, where the slow follower
 * releases later (scaled by sustainDb).
 */
export declare class TransientShaper {
    private readonly fA;
    private readonly fR;
    private readonly sA;
    private readonly sR;
    private fast;
    private slow;
    private attackDb;
    private sustainDb;
    constructor(sr: number);
    set(attackDb: number, sustainDb: number): void;
    process(x: number): number;
    reset(): void;
}
