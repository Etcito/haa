/** Cyclic table with linear interpolation. */
export declare class LoopTable {
    readonly length: number;
    private readonly data;
    constructor(data: Float32Array);
    /** Read at a fractional position; pos wraps cyclically. */
    read(pos: number): number;
}
/**
 * Build a seamless table of length buf.length - crossfade.
 * The tail (the last `crossfade` samples) is faded into the head with equal-power gains:
 * the head uses cos(t * pi/2), the tail sin(t * pi/2), t going 0..1 across the region.
 */
export declare function makeSeamless(buf: Float32Array, crossfade: number): Float32Array;
/**
 * Seam quality of a loop table: the wrap jump |t[0] - t[N-1]| and the 99th percentile of
 * the per-sample step |t[i+1] - t[i]|. Allocates (it sorts a copy): proof time only.
 */
export declare function seamScore(t: Float32Array): {
    jump: number;
    p99: number;
};
