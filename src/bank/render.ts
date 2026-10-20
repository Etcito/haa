// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, render into a preallocated buffer, loudness normalization instead of peak normalization
// Deterministic family variant rendering and one-shot loudness normalization. DSP-safe (no
// DOM, no Web Audio, no node builtins): the same inputs always give bit-identical output, so
// the offline proof, the bake and the in-browser fallback all render the same bytes.
//
// renderVariantRaw runs the six steps of the bank-render spec, in order:
//   1. allocate a zeroed buffer of ceil(maxSeconds × sr) frames;
//   2. render with a fresh AudioRng seeded from the family seed, id and variant, and clamp
//      the returned frame count to the buffer;
//   3. non-finite samples become 0;
//   4. trim the trailing samples below -70 dBFS;
//   5. cosine-fade the last 5 ms;
//   6. return the trimmed copy.
// renderFamilyVariants then applies one gain to the whole family (shared 'Loudness'): the
// lower median of the variant LUFS goes to the category target, capped so that the loudest
// sample peak stays at NORMALIZE_PEAK_CAP_DB.

import { NORMALIZE_PEAK_CAP_DB } from '../contract.ts'
import type { FamilyDef, MixPreset } from '../contract.ts'
import { clamp, dbToGain } from '../dsp/math.ts'
import { LoudnessMeter } from '../dsp/meter.ts'
import { AudioRng, hashSeed } from '../dsp/rng.ts'

const TRIM_DB = -70
const FADE_SECONDS = 0.005
/** Minimum metered length: one BS.1770 400 ms gating block. */
const LUFS_MIN_SECONDS = 0.4

/** Render one raw (unnormalized) variant of one family at sample rate sr. Bit-identical. */
export function renderVariantRaw(def: FamilyDef, v: number, sr: number): Float32Array {
  // 1. Preallocated, zeroed buffer of the family's maximum length.
  const buf = new Float32Array(Math.max(0, Math.ceil(def.maxSeconds * sr)))

  // 2. Render with its own deterministic stream; clamp the frames written.
  const written = def.render(buf, sr, new AudioRng(hashSeed(def.seed, def.id, v)), v)
  const n = Math.floor(clamp(written, 0, buf.length))

  // 3. Guard the samples.
  for (let i = 0; i < n; i++) {
    if (!Number.isFinite(buf[i])) buf[i] = 0
  }

  // 4. Trim the trailing sub-threshold tail.
  const floor = dbToGain(TRIM_DB)
  let len = n
  while (len > 0 && Math.abs(buf[len - 1]) < floor) len--

  // 5. Fade the last 5 ms.
  if (len > 0) {
    const fade = Math.min(len, Math.max(1, Math.round(FADE_SECONDS * sr)))
    if (fade > 1) {
      for (let k = 0; k < fade; k++) {
        buf[len - fade + k] *= 0.5 * (1 + Math.cos((Math.PI * k) / (fade - 1)))
      }
    } else {
      buf[len - 1] = 0
    }
  }

  // 6. The trimmed copy.
  return buf.slice(0, len)
}

/**
 * One-shot loudness (shared 'Loudness'): x[0..n) × √½ (centered equal-power pan) on both
 * channels, zero-padded to at least 0.4 s, metered as BS.1770-4 integrated loudness.
 * Returns -Infinity when gated out.
 */
export function oneShotLufs(x: Float32Array, n: number, sr: number): number {
  const m = Math.floor(clamp(n, 0, x.length))
  const y = new Float32Array(Math.max(m, Math.ceil(LUFS_MIN_SECONDS * sr)))
  for (let i = 0; i < m; i++) y[i] = x[i] * Math.SQRT1_2
  const meter = new LoudnessMeter(sr)
  meter.push(y, y, y.length)
  return meter.integrated()
}

/**
 * The family normalization gain: the lower median of the finite variant LUFS goes to target,
 * capped so the maximum variant sample peak lands at NORMALIZE_PEAK_CAP_DB. Gain 1 (and
 * medianLufs -Infinity) when no variant has a finite loudness.
 */
export function familyGain(raw: readonly Float32Array[], target: number, sr: number): { gain: number; medianLufs: number } {
  const lufs: number[] = []
  let peak = 0
  for (const x of raw) {
    const l = oneShotLufs(x, x.length, sr)
    if (Number.isFinite(l)) lufs.push(l)
    for (let i = 0; i < x.length; i++) {
      const a = Math.abs(x[i])
      if (a > peak) peak = a
    }
  }
  if (lufs.length === 0) return { gain: 1, medianLufs: -Infinity }
  lufs.sort((a, b) => a - b)
  const medianLufs = lufs[Math.floor((lufs.length - 1) / 2)]
  let gainDb = target - medianLufs
  if (peak > 0) gainDb = Math.min(gainDb, NORMALIZE_PEAK_CAP_DB - 20 * Math.log10(peak))
  return { gain: dbToGain(gainDb), medianLufs }
}

/** Applies one gain to every variant in place and returns them. */
function applyGain(variants: Float32Array[], gain: number): Float32Array[] {
  if (gain !== 1) {
    for (const x of variants) {
      for (let i = 0; i < x.length; i++) x[i] *= gain
    }
  }
  return variants
}

/**
 * Render every variant of def in order, loudness-normalized to preset.loudness[def.category].
 * Deterministic: bit-identical for the same inputs.
 */
export function renderFamilyVariants(def: FamilyDef, sr: number, preset: MixPreset): Float32Array[] {
  const raw: Float32Array[] = []
  for (let v = 0; v < def.variants; v++) raw.push(renderVariantRaw(def, v, sr))
  return applyGain(raw, familyGain(raw, preset.loudness[def.category], sr).gain)
}

/**
 * The same output as renderFamilyVariants, cooperatively sliced for the browser main thread.
 * After each variant it awaits yieldFn whenever now() has advanced by yieldEveryMs since the
 * last yield (the start counts as the first yield point).
 */
export async function renderFamilyVariantsAsync(
  def: FamilyDef,
  sr: number,
  preset: MixPreset,
  yieldEveryMs: number,
  now: () => number,
  yieldFn: () => Promise<void>,
): Promise<Float32Array[]> {
  const raw: Float32Array[] = []
  let last = now()
  for (let v = 0; v < def.variants; v++) {
    raw.push(renderVariantRaw(def, v, sr))
    if (now() - last >= yieldEveryMs) {
      await yieldFn()
      last = now()
    }
  }
  return applyGain(raw, familyGain(raw, preset.loudness[def.category], sr).gain)
}
