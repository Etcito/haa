/** FNV-1a 32 over the parts; returns an unsigned 32-bit integer. */
export declare function hashSeed(...parts: (number | string)[]): number;
/**
 * mulberry32 PRNG. Audio's own stream: it never reads or writes a simulation Rng.
 */
export declare class AudioRng {
    private s;
    private gaussCache;
    constructor(seed: number);
    get state(): number;
    set state(v: number);
    /** Uniform in [0, 1). */
    next(): number;
    /** Uniform in [a, b). */
    range(a: number, b: number): number;
    /** Uniform integer in [a, b], inclusive on both ends. */
    int(a: number, b: number): number;
    /** Standard normal via Box-Muller; the second value of each pair is cached. */
    gauss(): number;
    /** A fresh independent stream derived from the current state and the salt; does not advance this one. */
    fork(salt: number): AudioRng;
}
