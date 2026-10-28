/**
 * Polyphase halfband oversampler: 2x is one stage, 4x two cascaded stages. up() writes
 * `factor` samples into the caller's preallocated array; down() consumes them.
 * An up/down round trip delays by `latency` whole base-rate samples (15 at 2x, 23 at 4x).
 */
export declare class Oversampler {
    readonly factor: 2 | 4;
    readonly latency: number;
    private readonly s1;
    private readonly s2;
    private readonly mid;
    /** 4x only: one 2x-rate sample of delay so the round trip lands on a whole base sample. */
    private hold;
    constructor(factor: 2 | 4);
    up(x: number, out: Float32Array): void;
    down(buf: Float32Array): number;
    reset(): void;
}
/**
 * Asymmetric normalized tanh saturator:
 * y = (fastTanh(drive (x + asym)) - fastTanh(drive asym)) / fastTanh(drive),
 * evaluated at the oversampled rate (factor 1 bypasses oversampling), then mixed with the
 * latency-aligned dry signal by `mix`.
 */
export declare class Saturator {
    readonly factor: 1 | 2 | 4;
    private readonly os;
    private readonly buf;
    private readonly dry;
    private dryIdx;
    private readonly latency;
    private drive;
    private asym;
    private offset;
    private norm;
    private mix;
    constructor(factor: 1 | 2 | 4);
    set(drive: number, asym: number, mix: number): void;
    private shape;
    process(x: number): number;
    /** Writes x into the dry ring and returns the sample `latency` steps old. */
    private pushDry;
    reset(): void;
}
