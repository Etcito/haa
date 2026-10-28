import type { AudioRng } from './rng.js';
/** Uniform white noise in [-1, 1). */
export declare class WhiteNoise {
    private readonly rng;
    constructor(rng: AudioRng);
    next(): number;
    reset(): void;
}
/**
 * Pink noise, Paul Kellet's refined (economy) filter. The output is scaled to about
 * unit peak and clamped for safety.
 */
export declare class PinkNoise {
    private b0;
    private b1;
    private b2;
    private b3;
    private b4;
    private b5;
    private b6;
    private readonly rng;
    constructor(rng: AudioRng);
    next(): number;
    reset(): void;
}
/** Brown noise: a leaky integrator (leak 0.02), normalized and clamped. */
export declare class BrownNoise {
    private last;
    private readonly rng;
    constructor(rng: AudioRng);
    next(): number;
    reset(): void;
}
/**
 * Sparse Poisson crackle: each sample is an impulse with probability rate/sr, with a
 * random amplitude and a random sign; otherwise 0.
 */
export declare class Crackle {
    private readonly rng;
    constructor(rng: AudioRng);
    next(ratePerSec: number, sr: number): number;
    reset(): void;
}
/**
 * Velvet noise: exactly one +/-1 impulse per period of sr/density samples, placed at a
 * random position inside that period.
 */
export declare class VelvetNoise {
    private remaining;
    private at;
    private sign;
    private readonly rng;
    constructor(rng: AudioRng);
    next(density: number, sr: number): number;
    reset(): void;
}
