/** The measures of one rendered signal (mono or stereo). Levels in dBFS, dBTP and LUFS. */
export interface Measure {
    frames: number;
    seconds: number;
    nonFinite: number;
    samplePeakDb: number;
    truePeakDb: number;
    dc: number;
    rmsDb: number;
    minWindowRmsDb: number;
    lufs: number;
    shortTermMaxLufs: number;
    bandsDb: number[];
}
/** Octave band centers (Hz) of Measure.bandsDb. */
export declare const OCTAVE_BANDS: readonly number[];
/**
 * Measures a stereo signal. A shorter channel counts as zero-padded to the longer one. dc is
 * the max over channels of |mean|, rmsDb the RMS over both channels, and the bands use the
 * mono sum (l + r) / 2. lufs is the BS.1770-4 integrated loudness of (l, r), metered over at
 * least 0.4 s; -Infinity when gated out (shortTermMaxLufs: shorter than 3 s).
 */
export declare function measureStereo(l: Float32Array, r: Float32Array, sr: number): Measure;
/**
 * Measures a mono signal. Loudness is that of the centered equal-power pair (x√½, x√½), so
 * lufs equals oneShotLufs(x); the peaks, dc, RMS and bands use x itself.
 */
export declare function measureMono(x: Float32Array, sr: number): Measure;
/**
 * Applies SAFETY to a measure. Each failed rule is named: 'nonFinite' (any NaN/Inf sample),
 * 'samplePeak' (above -1 dBFS), 'dc' (|DC| of 1e-3 or more) and 'silence' (the minimum window
 * RMS below -60 dBFS). A NaN measure fails its rule.
 */
export declare function safetyVerdict(m: Measure): {
    ok: boolean;
    failures: string[];
};
