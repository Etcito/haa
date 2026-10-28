/**
 * Linear-attack, exponential-decay/release ADSR. `active` turns false once the release
 * has run its full length.
 */
export declare class Adsr {
    private stage;
    private level;
    private aSamples;
    private dSamples;
    private rSamples;
    private aLeft;
    private dLeft;
    private rLeft;
    private aInc;
    private decayCoef;
    private releaseCoef;
    private sus;
    set(a: number, d: number, s: number, r: number, sr: number): void;
    gate(on: boolean): void;
    get active(): boolean;
    next(): number;
    reset(): void;
}
/** Exponential decay that reaches -60 dB (0.001) after t60 seconds. */
export declare class ExpDecay {
    private level;
    private coef;
    trigger(level: number, t60: number, sr: number): void;
    next(): number;
    reset(): void;
}
/** Peak detector with independent attack and release coefficients. */
export declare class EnvFollower {
    private env;
    private readonly attack;
    private readonly release;
    constructor(attackMs: number, releaseMs: number, sr: number);
    process(x: number): number;
    reset(): void;
}
