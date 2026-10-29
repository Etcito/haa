import type { BankFamilyEntry } from '../contract.js';
/**
 * Concatenate the variants, each followed by GAP_SAMPLES zeros, and record where each one
 * starts and how long it is. onset0 is the first strip index above ONSET_THRESHOLD (0 when
 * the strip is silent). A zero-length variant still reserves its gap.
 */
export declare function packStrip(variants: readonly Float32Array[]): {
    strip: Float32Array;
    entries: {
        start: number;
        length: number;
    }[];
    onset0: number;
};
/**
 * FNV-1a 32 (offset 2166136261, prime 16777619) over the strip's little-endian float bytes,
 * as 8 lowercase hex digits. A Uint8Array view of the underlying buffer is used so the value
 * is independent of the platform's typed-array element handling.
 */
export declare function pcmHash(strip: Float32Array): string;
/**
 * The shared runtime alignment rule. After decodeAudioData at rate R the strip is compressed
 * or stretched by k = R / 48000; the decoded onset sits at firstDecodedOnset, so every
 * recorded offset is shifted by the difference from where the bake put onset0. Each variant
 * is decoded[round(start * k) + shift, + round(length * k)]. Slices are clamped to the
 * decoded buffer; an empty result is a zero-length Float32Array.
 */
export declare function alignVariants(decoded: Float32Array, entry: BankFamilyEntry, rate: number): Float32Array[];
