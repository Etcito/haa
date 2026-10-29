// Extracted from an Etcito game's audio engine (see PROVENANCE.md). HAA changes: .ts imports, no sfx aliases, render writes into the given buffer, tuned for m1 safety/loudness (DC blocker + crest soft clip)
// The 12 baked impact one-shot families (module content-core). DSP-safe: the same
// renderImpact() runs in the node proof, the build-time bake and the in-browser fallback.
//
// One family per MATERIALS x ['small' | 'medium' | 'large'], bus 'effects', category 'impact',
// 10 seeded variants each. The variant draws its own velocity (0.85..1) and contact time
// (0.8..3 ms) from the family variant stream, then hands the SAME rng to renderImpact so the
// modal jitter, phase, debris grains and layer balances vary per variant, while a re-render
// from the same seed is bit-identical. maxSeconds is the per-material/size decay cap; the whole
// bank is 10 x 6.7 s = 67 s.
import { MATERIALS } from '../contract.ts'
import type { FamilyDef, MaterialId } from '../contract.ts'
import { DcBlocker } from '../dsp/filters.ts'
import { hashSeed } from '../dsp/rng.ts'
import { Saturator } from '../dsp/shaper.ts'
import type { AudioRng } from '../dsp/rng.ts'
import { renderImpact } from '../gen/impactModel.ts'
import type { ImpactSize } from '../gen/impactModel.ts'

/** The three impact sizes, in proof order. */
const SIZES: readonly ImpactSize[] = ['small', 'medium', 'large']

/** Variants per family (within FAMILY_VARIANTS_MIN..MAX). */
const VARIANTS = 10

/** Decay cap per material and size (seconds): metal rings longest, plastic shortest. */
const MAX_SECONDS: Record<MaterialId, Record<ImpactSize, number>> = {
  metal: { small: 0.6, medium: 0.9, large: 1.2 },
  plastic: { small: 0.25, medium: 0.3, large: 0.4 },
  wood: { small: 0.35, medium: 0.45, large: 0.6 },
  concrete: { small: 0.4, medium: 0.55, large: 0.7 },
}

/** Velocity and contact ranges the per-variant stream draws from. */
const VELOCITY_MIN = 0.85
const VELOCITY_MAX = 1
const CONTACT_MIN = 0.0008
const CONTACT_MAX = 0.003

// HAA tuning: the raw renderImpact output carries a DC offset (up to 1.2e-2 after the family
// gain) and a first-millisecond contact spike that capped familyGain at the -1.05 dBFS peak,
// so several families missed the -21 LUFS impact target. Each variant is now DC-blocked over
// the whole written length, soft-clipped relative to its own peak (2x oversampled tanh, so the
// variants keep their level differences) to lower the crest, and DC-blocked again because
// clipping an asymmetric waveform leaves a small offset.
/** Soft-clip drive per material and size (1 = transparent; higher trims the onset spike more). */
const CREST_DRIVE: Record<MaterialId, Record<ImpactSize, number>> = {
  metal: { small: 1, medium: 1, large: 1 },
  plastic: { small: 2.5, medium: 2.3, large: 1.5 },
  wood: { small: 1.6, medium: 1.3, large: 1.3 },
  concrete: { small: 2.5, medium: 3, large: 5 },
}

/** Lower the crest factor of x in place: tanh soft clip at `drive`, scaled to x's own peak,
 *  so the clip shape is independent of the variant level and the output peak stays put. */
function softenCrest(x: Float32Array, drive: number): void {
  let peak = 0
  for (let i = 0; i < x.length; i++) {
    const a = Math.abs(x[i])
    if (a > peak) peak = a
  }
  if (drive <= 1 || peak <= 0) return
  const sat = new Saturator(2)
  sat.set(drive, 0, 1)
  const inv = 1 / peak
  for (let i = 0; i < x.length; i++) x[i] = sat.process(x[i] * inv) * peak
}

/** Remove the DC offset of x in place (one-pole DC blocker over the whole length). */
function removeDc(x: Float32Array): void {
  const dc = new DcBlocker()
  for (let i = 0; i < x.length; i++) x[i] = dc.process(x[i])
}

/** One impact family: the seeded variant render draws its velocity and contact, then adds the
 *  three-layer renderImpact into the first round(maxSeconds x sr) frames of the given buffer. */
function impactFamily(material: MaterialId, size: ImpactSize): FamilyDef {
  const id = `impact.${material}.${size}`
  const maxSeconds = MAX_SECONDS[material][size]
  const drive = CREST_DRIVE[material][size]
  return {
    id,
    bus: 'effects',
    category: 'impact',
    variants: VARIANTS,
    maxSeconds,
    seed: hashSeed('impact', material, size),
    render: (out: Float32Array, sr: number, rng: AudioRng): number => {
      const n = Math.min(out.length, Math.round(maxSeconds * sr))
      const velocity = rng.range(VELOCITY_MIN, VELOCITY_MAX)
      const contact = rng.range(CONTACT_MIN, CONTACT_MAX)
      const seg = out.subarray(0, n)
      renderImpact({ material, size, velocity, contact }, sr, rng, seg)
      removeDc(seg)
      softenCrest(seg, drive)
      removeDc(seg)
      return n
    },
  }
}

/** Every impact family, MATERIALS x SIZES order. */
export const IMPACT_FAMILIES: readonly FamilyDef[] = MATERIALS.flatMap((material) =>
  SIZES.map((size) => impactFamily(material, size)),
)
