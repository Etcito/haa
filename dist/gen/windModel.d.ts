import type { BedVoice } from '../contract.js';
/** One shaping band: center Hz, SVF Q, base level dB and the speed exponent. */
export interface WindBand {
    hz: number;
    q: number;
    levelDb: number;
    exp: number;
}
/** The five wind bands. The base level falls with frequency while the exponent rises, so fast
 *  air gains high end faster than low end: the speed sensation is a timbre change, not volume. */
export declare const WIND_BANDS: readonly WindBand[];
/** The listener-level wind voice. See the file header for the full model. */
export declare class WindModel implements BedVoice {
    private readonly sr;
    private readonly controlDt;
    /** Decorrelated L/R sources: pink plus a smaller brown blend, from seeded forks. */
    private readonly rngPinkL;
    private readonly rngBrownL;
    private readonly rngPinkR;
    private readonly rngBrownR;
    private readonly pinkL;
    private readonly brownL;
    private readonly pinkR;
    private readonly brownR;
    private readonly pinkLSeed;
    private readonly brownLSeed;
    private readonly pinkRSeed;
    private readonly brownRSeed;
    /** One bandpass per band and channel. */
    private readonly bandL;
    private readonly bandR;
    /** Per-sample linear ramps of the band gains. */
    private readonly bandCur;
    private readonly bandStep;
    /** Turbulence: one seeded slow modulator per band. */
    private readonly turbRng;
    private readonly turbSeed;
    private readonly turbVal;
    private readonly turbCoef;
    /** Input scale giving each smoothed modulator unit std: sqrt(3 (2 - c) / c) for uniform white. */
    private readonly turbNorm;
    /** Gusts. */
    private readonly gustRng;
    private readonly gustSeed;
    private gustTimer;
    private gustDur;
    private gustAmp;
    private gustPhase;
    /** Buffeting quadrature oscillator (a gain only, never routed to the output). */
    private bufCos;
    private bufSin;
    /** Flutter whistle. */
    private readonly whistle0;
    private readonly whistle1;
    private readonly driftRng;
    private readonly driftSeed;
    private readonly driftCoef;
    private readonly driftNorm;
    private driftVal0;
    private driftVal1;
    /** Highpassed bright rush for boost. */
    private readonly boostHp;
    /** Drafting-pocket lowpass, one pole per channel. */
    private readonly slipLpL;
    private readonly slipLpR;
    /** Underwater fade (150 ms). */
    private readonly under;
    /** Scalar ramps. */
    private whCur;
    private whStep;
    private boostCur;
    private boostStep;
    private slipCur;
    private slipStep;
    private slipGainCur;
    private slipGainStep;
    private panCurL;
    private panCurR;
    private panStepL;
    private panStepR;
    private levelCur;
    private levelStep;
    /** The final scalar gain applied to the last sample, for the early-out test. */
    private outGain;
    /** Preallocated scratch so a non-finite sample never reaches the caller's buffers. */
    private readonly bufL;
    private readonly bufR;
    /** Per-call smoothed control values. */
    private speed;
    private speed01;
    private air;
    private boost;
    private slip;
    private yaw;
    private underwater;
    private level;
    constructor(sr: number, seed: number);
    reset(): void;
    /** ADDS the wind voice into l/r[0..n), in chunks of at most BLOCK frames. */
    process(p: Float32Array, offset: number, l: Float32Array, r: Float32Array, n: number): void;
    /** Recompute every control-rate target and turn it into a per-sample linear ramp. */
    private control;
    /** Renders m <= CONTROL frames into the scratch buffers. */
    private renderChunk;
}
