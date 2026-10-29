import type { FamilyDef, MixPreset } from '../contract.js';
/** Render one raw (unnormalized) variant of one family at sample rate sr. Bit-identical. */
export declare function renderVariantRaw(def: FamilyDef, v: number, sr: number): Float32Array;
/**
 * One-shot loudness (shared 'Loudness'): x[0..n) × √½ (centered equal-power pan) on both
 * channels, zero-padded to at least 0.4 s, metered as BS.1770-4 integrated loudness.
 * Returns -Infinity when gated out.
 */
export declare function oneShotLufs(x: Float32Array, n: number, sr: number): number;
/**
 * The family normalization gain: the lower median of the finite variant LUFS goes to target,
 * capped so the maximum variant sample peak lands at NORMALIZE_PEAK_CAP_DB. Gain 1 (and
 * medianLufs -Infinity) when no variant has a finite loudness.
 */
export declare function familyGain(raw: readonly Float32Array[], target: number, sr: number): {
    gain: number;
    medianLufs: number;
};
/**
 * Render every variant of def in order, loudness-normalized to preset.loudness[def.category].
 * Deterministic: bit-identical for the same inputs.
 */
export declare function renderFamilyVariants(def: FamilyDef, sr: number, preset: MixPreset): Float32Array[];
/**
 * The same output as renderFamilyVariants, cooperatively sliced for the browser main thread.
 * After each variant it awaits yieldFn whenever now() has advanced by yieldEveryMs since the
 * last yield (the start counts as the first yield point).
 */
export declare function renderFamilyVariantsAsync(def: FamilyDef, sr: number, preset: MixPreset, yieldEveryMs: number, now: () => number, yieldFn: () => Promise<void>): Promise<Float32Array[]>;
