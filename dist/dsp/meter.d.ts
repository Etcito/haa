/**
 * ITU-R BS.1770-4 / EBU R128 loudness meter for a stereo program (channel weights 1, 1).
 * Energy accumulates per 100 ms sub-block; each new sub-block closes a 400 ms block made of
 * the last 4 sub-blocks (75% overlap) and, once 30 exist, a short-term window of 3 s.
 */
export declare class LoudnessMeter {
    readonly sampleRate: number;
    private readonly subLen;
    private readonly preL;
    private readonly rlbL;
    private readonly preR;
    private readonly rlbR;
    /** Mean-square energy of the last SHORT_TERM_SUBS sub-blocks (ring). */
    private readonly subs;
    private subCount;
    private subAcc;
    private subFill;
    /** Loudness (LUFS) of every completed 400 ms block; grown by doubling. */
    private blocks;
    private blockCount;
    private momMax;
    private stMax;
    constructor(sr: number);
    /** Feeds n stereo frames. Non-finite samples are metered as 0. */
    push(l: Float32Array, r: Float32Array, n: number): void;
    private closeSubBlock;
    private meanLastSubs;
    /** Gated integrated loudness in LUFS; -Infinity when no block passes the gates. */
    integrated(): number;
    /** Highest momentary (400 ms) loudness in LUFS; -Infinity before the first block. */
    momentaryMax(): number;
    /** Highest short-term (3 s) loudness in LUFS; -Infinity before the first 3 s. */
    shortTermMax(): number;
    /** Number of completed 400 ms blocks. */
    get blockTotal(): number;
    reset(): void;
}
/** Oversampling factor of the true-peak interpolator. */
export declare const TP_OVERSAMPLE = 4;
/** Taps per polyphase branch (48 / 4). */
export declare const TP_TAPS = 12;
/** The 4 polyphase branches of the BS.1770-4 Annex 2 filter; y_k[n] = sum_j h_k[j] x[n - j]. */
export declare const TP_PHASE_TAPS: readonly Float64Array[];
/**
 * Streaming 4x true-peak detector: push one input sample, get the max |y| over the 4
 * interpolated phases at that position (delayed by the filter's ~5.9-sample group delay).
 * No allocation after construction.
 */
export declare class TruePeakDetector {
    /** Doubled history ring: hist[idx + j] = x[n - j] for j in 0..TP_TAPS-1. */
    private readonly hist;
    private idx;
    push(x: number): number;
    reset(): void;
}
/**
 * True peak in dBTP: the max |x| of the 4x oversampled signal (BS.1770-4 Annex 2) over
 * both channels. r may be null for mono. sr is accepted for the API; the interpolator is
 * the fixed 4x Annex 2 filter.
 */
export declare function truePeakDb(l: Float32Array, r: Float32Array | null, sr: number): number;
/** Sample peak in dBFS over both channels (r may be null); -200 for silence. */
export declare function samplePeakDb(l: Float32Array, r: Float32Array | null): number;
/** RMS in dBFS over x[from, to) (clamped to the array); -200 for silence or an empty range. */
export declare function rmsDb(x: Float32Array, from: number, to: number): number;
/**
 * RMS in dBFS of consecutive non-overlapping windows of windowSec. A trailing partial window
 * is dropped, except that a signal shorter than one window yields one window over all of it.
 */
export declare function windowRmsDb(x: Float32Array, sr: number, windowSec: number): Float32Array;
/**
 * Max over lags -maxLag..maxLag of |sum_i a[i] b[i + lag]| / sqrt(sum a^2 * sum b^2),
 * summing only where both indices exist. 0 when either signal is silent.
 */
export declare function maxNormXcorr(a: Float32Array, b: Float32Array, maxLag: number): number;
/** Number of NaN or infinite samples. */
export declare function countNonFinite(x: Float32Array): number;
